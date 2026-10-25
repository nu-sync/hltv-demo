//! Live end-to-end check: both entry points, against whatever is newest on https://www.hltv.org/results.
//!
//! The target is discovered, not hardcoded: read the newest results page, walk the candidates until one
//! publishes a demo, then drive the CLI and the MCP server against that match and one output folder.
//! A hardcoded match URL rots the day its demo is removed; a discovered one keeps the check honest.
//!
//! Exit status: 0 every check passed, 1 a check failed, 2 a prerequisite is missing (a coverage gap,
//! not a pass: Cloudflare's challenge, a held profile, no Chrome, no network, or HLTV simply
//! publishing no demo yet).
//!
//! Environment:
//!   LIVE_SCAN          newest matches to scan for a demo (default 10)
//!   LIVE_MAX_AGE_DAYS  how old the discovered target may be (default 30)
//!   LIVE_KEEP=1        keep the downloaded archive (440-890 MB) instead of deleting it
//!   EVIDENCE           evidence folder root (default <crate>/.live-checks)
//!   CHROME, HLTV_DEMO_PROFILE  as the tool itself reads them

use std::collections::HashSet;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hltv_demo::browser::{Chrome, chrome_path, profile_dir};
use hltv_demo::core::manifest;
use hltv_demo::core::types::{Archive, DemoRef, Match, SearchFilter};
use hltv_demo::core::url::BASE;
use hltv_demo::{HltvError, Source, resolve, search, with_source};
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::TokioChildProcess;
use rmcp::{RoleClient, ServiceExt};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};
use tokio::io::AsyncReadExt;

use crate::{paths, text};

const MIN_ARCHIVE_BYTES: u64 = 100_000_000;

// ---------------------------------------------------------------- check harness

/// Why a check did not pass. A gap is the environment, not the code: the run ends at exit 2.
enum Problem {
    Fail(String),
    Gap(String),
}

impl From<HltvError> for Problem {
    /// Cloudflare, a held profile, no Chrome, and no network are gaps. Anything else is a real failure.
    fn from(error: HltvError) -> Self {
        match error {
            HltvError::Challenge { .. }
            | HltvError::ProfileBusy { .. }
            | HltvError::BrowserMissing { .. }
            | HltvError::Unreachable { .. } => Problem::Gap(error.explain()),
            other => Problem::Fail(other.explain()),
        }
    }
}

type Outcome<T = String> = Result<T, Problem>;

fn ensure(condition: bool, message: impl FnOnce() -> String) -> Outcome<()> {
    if condition {
        Ok(())
    } else {
        Err(Problem::Fail(message()))
    }
}

/// An environment that is not there: not a code failure.
fn require(condition: bool, message: impl FnOnce() -> String) -> Outcome<()> {
    if condition {
        Ok(())
    } else {
        Err(Problem::Gap(message()))
    }
}

#[derive(serde::Serialize)]
struct Check {
    name: String,
    status: &'static str,
    detail: Option<String>,
    ms: u128,
}

#[derive(Default)]
struct Harness {
    checks: Vec<Check>,
    lines: Vec<String>,
    facts: Map<String, Value>,
}

impl Harness {
    fn emit(&mut self, line: impl Into<String>) {
        let line = line.into();
        println!("{line}");
        self.lines.push(line);
    }

    fn fact(&mut self, key: &str, value: Value) {
        self.facts.insert(key.into(), value);
    }

    /// Run one check. A check that does not pass stops the run: everything after depends on it.
    async fn check(&mut self, name: &str, work: impl Future<Output = Outcome>) -> Outcome<()> {
        let started = Instant::now();
        let outcome = work.await;
        let ms = started.elapsed().as_millis();
        let (status, detail) = match &outcome {
            Ok(detail) => ("PASS", detail.clone()),
            Err(Problem::Fail(detail)) => ("FAIL", detail.clone()),
            Err(Problem::Gap(detail)) => ("GAP", detail.clone()),
        };
        match status {
            "PASS" if detail.is_empty() => self.emit(format!("PASS  {name}")),
            "PASS" => self.emit(format!("PASS  {name} — {detail}")),
            _ => self.emit(format!(
                "{status:<4}  {name}\n      {}",
                detail.replace('\n', "\n      ")
            )),
        }
        self.checks.push(Check {
            name: name.into(),
            status,
            detail: Some(detail).filter(|d| !d.is_empty()),
            ms,
        });
        outcome.map(|_| ())
    }
}

// ---------------------------------------------------------------- subprocesses

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
    ms: u128,
}

impl Run {
    fn evidence(&self) -> Value {
        json!({ "code": self.code, "ms": self.ms, "stdout": self.stdout, "stderr": self.stderr })
    }

    fn tail(&self) -> String {
        let at = self.stderr.len().saturating_sub(1500);
        self.stderr[text::ceil(&self.stderr, at)..].to_string()
    }

    /// Exit 0, or the reason not: an exit 2 or a challenge is the environment, anything else the code.
    fn succeeded(&self) -> Outcome<()> {
        if self.code == 0 {
            Ok(())
        } else if self.code == 2 || self.stderr.contains("behind the Cloudflare challenge") {
            Err(Problem::Gap(format!("exit {}: {}", self.code, self.tail())))
        } else {
            Err(Problem::Fail(format!(
                "exit {}: {}",
                self.code,
                self.tail()
            )))
        }
    }
}

async fn spawn(program: &Path, args: &[&str], cwd: &Path) -> Outcome<Run> {
    let started = Instant::now();
    let output = tokio::process::Command::new(program)
        .args(args)
        .current_dir(cwd)
        .output()
        .await
        .map_err(|e| Problem::Gap(format!("cannot run {}: {e}", program.display())))?;
    Ok(Run {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        ms: started.elapsed().as_millis(),
    })
}

/// One JSON value, keeping the raw text in the failure message.
fn parse<T: DeserializeOwned>(text: &str, what: &str) -> Outcome<T> {
    serde_json::from_str(text.trim()).map_err(|_| {
        let shown: String = text.trim().chars().take(400).collect();
        Problem::Fail(format!(
            "{what} did not print JSON: {}",
            if shown.is_empty() {
                "(empty)".into()
            } else {
                shown
            }
        ))
    })
}

/// The CLI prints JSON one object per line.
fn json_lines<T: DeserializeOwned>(text: &str, what: &str) -> Outcome<Vec<T>> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| parse(line, what))
        .collect()
}

/// The archives in a folder, by HLTV's own extension.
fn rars(dir: &Path) -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".rar"))
        .collect();
    found.sort();
    found
}

fn text_of(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|c| c.as_text())
        .map(|t| t.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// A tool's answer: its JSON, or the reason not, triaged like a CLI exit.
fn answer<T: DeserializeOwned>(result: &CallToolResult, what: &str) -> Outcome<T> {
    let text = text_of(result);
    if result.is_error != Some(true) {
        return parse(&text, what);
    }
    let environment = [
        "behind the Cloudflare challenge",
        "is in use",
        "no Chrome found",
        "could not be reached",
    ];
    if environment.iter().any(|reason| text.contains(reason)) {
        Err(Problem::Gap(format!("{what}: {text}")))
    } else {
        Err(Problem::Fail(format!("{what}: {text}")))
    }
}

async fn call(
    client: &RunningService<RoleClient, ()>,
    tool: &'static str,
    args: Value,
) -> Outcome<CallToolResult> {
    let params = CallToolRequestParams::new(tool)
        .with_arguments(args.as_object().cloned().unwrap_or_default());
    client
        .call_tool(params)
        .await
        .map_err(|e| Problem::Fail(format!("{tool}: {e}")))
}

// ---------------------------------------------------------------- dates

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// The run's name: UTC time to the second, then the pid, so runs sort by when they started.
fn run_name() -> String {
    let secs = now_secs();
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let s = secs.rem_euclid(86_400);
    format!(
        "{y:04}{m:02}{d:02}{:02}{:02}{:02}-{}",
        s / 3600,
        s / 60 % 60,
        s % 60,
        std::process::id()
    )
}

fn age_days(iso_day: &str) -> Option<f64> {
    let mut parts = iso_day.splitn(3, '-').map(|p| p.parse::<i64>().ok());
    let (y, m, d) = (parts.next()??, parts.next()??, parts.next()??);
    Some((now_secs() - days_from_civil(y, m, d) * 86_400) as f64 / 86_400.0)
}

// ---------------------------------------------------------------- discovery

struct Trial {
    found: Match,
    demo_href: Option<String>,
    event: Option<String>,
    date: Option<String>,
}

struct Discovery {
    matches: Vec<Match>,
    trials: Vec<Trial>,
    hit: Option<Match>,
    demo: Option<DemoRef>,
}

/// One browser, one results page, then match pages until one has a demo. Deliberately sequential: the
/// same politeness the tool promises, and a burst of navigations is how a live check earns a challenge.
async fn discover(scan: usize) -> Result<Discovery, HltvError> {
    with_source(&Chrome, async |source| {
        let matches = search(
            source,
            &SearchFilter {
                limit: Some(scan),
                ..Default::default()
            },
        )
        .await?;
        let mut trials = Vec::new();
        let mut hit = None;
        for (index, found) in matches.iter().enumerate() {
            if index > 0 {
                tokio::time::sleep(Duration::from_millis(750)).await;
            }
            let page = source.match_page(&found.url).await?;
            let has_demo = page.demo_href.is_some();
            trials.push(Trial {
                found: found.clone(),
                demo_href: page.demo_href,
                event: page.event,
                date: page.date,
            });
            if has_demo {
                hit = Some(found.clone());
                break;
            }
        }
        let demo = match &hit {
            Some(found) => Some(resolve(source, &found.url).await?),
            None => None,
        };
        Ok(Discovery {
            matches,
            trials,
            hit,
            demo,
        })
    })
    .await
}

// ---------------------------------------------------------------- main

struct Settings {
    scan: usize,
    max_age_days: f64,
    keep: bool,
    evidence: PathBuf,
    run: String,
}

pub async fn main(_args: &[String]) -> u8 {
    let env = |key: &str| std::env::var(key).ok();
    let run = run_name();
    let settings = Settings {
        scan: env("LIVE_SCAN")
            .and_then(|v| v.parse().ok())
            .unwrap_or(10usize)
            .max(1),
        max_age_days: env("LIVE_MAX_AGE_DAYS")
            .and_then(|v| v.parse().ok())
            .unwrap_or(30f64)
            .max(0.0),
        keep: env("LIVE_KEEP").as_deref() == Some("1"),
        evidence: env("EVIDENCE")
            .map_or_else(|| paths::root().join(".live-checks"), PathBuf::from)
            .join(&run),
        run,
    };
    let started = Instant::now();
    let mut harness = Harness::default();
    let outcome = checks(&mut harness, &settings).await;
    report(&mut harness, &settings, outcome, started)
}

async fn checks(h: &mut Harness, s: &Settings) -> Outcome<()> {
    h.emit(format!("hltv-demo live check {}", s.run));
    h.emit(format!("target: whatever is newest on {BASE}/results"));
    h.emit(format!("evidence: {}", s.evidence.display()));
    let _ = std::fs::create_dir_all(&s.evidence);
    let root = paths::root();

    h.emit("");
    h.emit("== prerequisites");

    let mut bins = PathBuf::new();
    let mut facts = Map::new();
    h.check(
        "Chrome and bsdtar are present, and the binaries build",
        async {
            let chrome = chrome_path();
            require(chrome.exists(), || {
                format!("no Chrome at {} (set CHROME=)", chrome.display())
            })?;
            require(paths::which("bsdtar").is_some(), || {
                "bsdtar is not on PATH".into()
            })?;
            bins = paths::binaries().map_err(Problem::Gap)?;
            facts.insert("chrome".into(), json!(chrome));
            facts.insert("profile".into(), json!(profile_dir()));
            facts.insert("binaries".into(), json!(bins));
            Ok(chrome.display().to_string())
        },
    )
    .await?;
    h.facts.extend(facts);
    let cli = bins.join("hltv-demo");
    let mcp = bins.join("hltv-demo-mcp");

    let mut evidence = Value::Null;
    h.check("the offline suites pass (cargo test)", async {
        let cargo = std::env::var_os("CARGO").map_or_else(|| PathBuf::from("cargo"), PathBuf::from);
        let run = spawn(&cargo, &["test", "--package", "hltv-demo"], &root).await?;
        let (mut passed, mut failed) = (0u64, 0u64);
        for line in run.stdout.lines().filter(|line| line.starts_with("test result:")) {
            let count = |label: &str| {
                line.split(';').find_map(|part| part.trim().strip_suffix(label)?.rsplit(' ').next()?.parse::<u64>().ok())
            };
            passed += count(" passed").unwrap_or(0);
            failed += count(" failed").unwrap_or(0);
        }
        evidence = json!({ "code": run.code, "ms": run.ms, "passed": passed, "failed": failed, "tail": run.tail() });
        ensure(run.code == 0, || format!("cargo test exited {}\n{}", run.code, run.tail()))?;
        Ok(format!("{passed} pass, {failed} fail"))
    })
    .await?;
    h.fact("cargoTest", evidence);

    h.emit("");
    h.emit("== discovery: anything recent on /results");

    let mut discovery = None;
    h.check(&format!("a recent match publishes a demo (scanned up to {})", s.scan), async {
        let found = discover(s.scan).await?;
        ensure(!found.matches.is_empty(), || format!("{BASE}/results listed no matches"))?;
        let Some(hit) = &found.hit else {
            let blind = found.trials.iter().all(|t| t.event.is_none() && t.date.is_none());
            let scanned = found
                .trials
                .iter()
                .map(|t| format!("  {} event={:?}", t.found.url, t.event))
                .collect::<Vec<_>>()
                .join("\n");
            return Err(if blind {
                Problem::Fail(format!("no candidate page yielded an event or a date: the match-page reader is broken, not HLTV\n{scanned}"))
            } else {
                Problem::Gap(format!("none of the newest {} matches publishes a demo yet (HLTV publishes them hours after a match); retry later or raise LIVE_SCAN\n{scanned}", found.trials.len()))
            });
        };
        let detail = format!("{} {} vs {} ({} scanned)", hit.match_id, hit.teams.0, hit.teams.1, found.trials.len());
        discovery = Some(found);
        Ok(detail)
    })
    .await?;
    let discovery = discovery.expect("the check passed");
    h.fact(
        "discovery",
        json!({
            "matches": discovery.matches,
            "trials": discovery.trials.iter().map(|t| json!({
                "match": t.found, "demoHref": t.demo_href, "event": t.event, "date": t.date,
            })).collect::<Vec<_>>(),
            "hit": discovery.hit,
            "demo": discovery.demo,
        }),
    );
    let target = discovery.hit.clone().expect("discovered");
    let demo = discovery.demo.clone().expect("resolved");
    let trial = discovery
        .trials
        .iter()
        .find(|t| t.found.match_id == target.match_id)
        .expect("tried");

    h.check(
        "the match page reader still reads an event and a date",
        async {
            ensure(trial.event.is_some(), || {
                format!("no event on {}", target.url)
            })?;
            ensure(trial.date.is_some(), || {
                format!("no date on {}", target.url)
            })?;
            Ok(format!(
                "{}, {}",
                trial.event.as_deref().unwrap_or(""),
                trial.date.as_deref().unwrap_or("")
            ))
        },
    )
    .await?;

    h.check(
        &format!("the target is recent ({} days)", s.max_age_days),
        async {
            let Some(date) = &trial.date else {
                return Ok("undated (featured block); recency taken from position".into());
            };
            let age =
                age_days(date).ok_or_else(|| Problem::Fail(format!("{date} is not a day")))?;
            let days = age.round() as i64;
            let plural = if days == 1 { "" } else { "s" };
            ensure(age <= s.max_age_days, || {
                format!(
                    "{} is {days} day{plural} old (LIVE_MAX_AGE_DAYS={})",
                    target.url, s.max_age_days
                )
            })?;
            Ok(format!("{date}, {days} day{plural} old"))
        },
    )
    .await?;

    h.check(
        "the library's resolve() agrees with the page reader",
        async {
            ensure(demo.match_id == target.match_id, || {
                format!(
                    "resolve() reported match {} for {}",
                    demo.match_id, target.match_id
                )
            })?;
            let href = trial.demo_href.as_deref().unwrap_or("");
            ensure(demo.url.ends_with(href), || {
                format!("resolve() gave {}, the page gave {href}", demo.url)
            })?;
            Ok(demo.url.clone())
        },
    )
    .await?;

    h.emit("");
    h.emit("== cli");

    let mut cli_matches: Vec<Match> = Vec::new();
    let mut evidence = Value::Null;
    h.check(
        "search --limit 5 --json exits 0 with five unique matches",
        async {
            let run = spawn(&cli, &["search", "--limit", "5", "--json"], &root).await?;
            evidence = run.evidence();
            run.succeeded()?;
            cli_matches = json_lines(&run.stdout, "search")?;
            ensure(cli_matches.len() == 5, || {
                format!("got {} matches", cli_matches.len())
            })?;
            let unique: HashSet<u64> = cli_matches.iter().map(|m| m.match_id).collect();
            ensure(unique.len() == 5, || "duplicate match ids".into())?;
            Ok(format!("{} matches in {} ms", cli_matches.len(), run.ms))
        },
    )
    .await?;
    h.fact("cliSearch", evidence);

    h.check(
        "every match search returns is an hltv.org match URL",
        async {
            for found in &cli_matches {
                ensure(found.url.starts_with(&format!("{BASE}/matches/")), || {
                    format!("{} is not a match URL", found.url)
                })?;
                ensure(
                    !found.teams.0.is_empty() && !found.teams.1.is_empty(),
                    || format!("{} has no teams", found.url),
                )?;
            }
            Ok(format!(
                "{} newest",
                cli_matches[0].date.as_deref().unwrap_or("?")
            ))
        },
    )
    .await?;

    let mut evidence = Value::Null;
    h.check(
        "resolve <match> prints the demo URL the scan found",
        async {
            let run = spawn(&cli, &["resolve", &target.url], &root).await?;
            evidence = run.evidence();
            run.succeeded()?;
            let url = run.stdout.trim();
            let shaped = url
                .strip_prefix(&format!("{BASE}/download/demo/"))
                .is_some_and(|id| !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()));
            ensure(shaped, || format!("printed {url:?}"))?;
            ensure(url == demo.url, || {
                format!("printed {url}, the page reader gave {}", demo.url)
            })?;
            Ok(url.to_string())
        },
    )
    .await?;
    h.fact("cliResolve", evidence);

    let work = tempfile::Builder::new()
        .prefix("hltv-demo-live-")
        .tempdir()
        .map_err(|e| Problem::Gap(e.to_string()))?;
    let out = work.path().join("archives");
    let outcome = downloads(h, &cli, &mcp, &root, &out, &target, &demo, &cli_matches).await;
    if s.keep {
        let kept = work.keep();
        h.emit(format!("      kept: {}", kept.join("archives").display()));
        h.fact("keptAt", json!(kept.join("archives")));
    }
    outcome
}

#[allow(clippy::too_many_arguments)]
async fn downloads(
    h: &mut Harness,
    cli: &Path,
    mcp: &Path,
    root: &Path,
    out: &Path,
    target: &Match,
    demo: &DemoRef,
    cli_matches: &[Match],
) -> Outcome<()> {
    let out_arg = out.to_str().expect("a temp path is UTF-8");
    let mut archive: Option<Archive> = None;
    let mut evidence = Value::Null;
    h.check("download <match> --out exits 0", async {
        let run = spawn(
            cli,
            &["download", &target.url, "--out", out_arg, "--json"],
            root,
        )
        .await?;
        evidence = run.evidence();
        run.succeeded()?;
        let mut archives: Vec<Archive> = json_lines(&run.stdout, "download")?;
        ensure(archives.len() == 1, || {
            format!("expected one archive, got {archives:?}")
        })?;
        let one = archives.remove(0);
        let detail = format!("{} in {} ms", one.suggested_name, run.ms);
        archive = Some(one);
        Ok(detail)
    })
    .await?;
    h.fact("cliDownload", evidence);
    let archive = archive.expect("downloaded");
    let path = PathBuf::from(&archive.path);

    h.check("the archive is on disk, and only one is", async {
        ensure(path.is_file(), || {
            format!("{} is not on disk", archive.path)
        })?;
        let listing = rars(out);
        ensure(listing.len() == 1, || {
            format!("expected one archive, found {listing:?}")
        })?;
        Ok(path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned())
    })
    .await?;

    h.check(
        &format!(
            "the archive is a real download (> {} MB)",
            MIN_ARCHIVE_BYTES / 1_000_000
        ),
        async {
            let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            ensure(bytes > MIN_ARCHIVE_BYTES, || {
                format!(
                    "{:.1} MB is too small to be a demo archive",
                    bytes as f64 / 1e6
                )
            })?;
            ensure(bytes == archive.bytes, || {
                format!("reported {} bytes, on disk {bytes}", archive.bytes)
            })?;
            Ok(format!("{:.0} MB", bytes as f64 / 1e6))
        },
    )
    .await?;
    h.fact(
        "archive",
        json!({ "archive": archive, "listing": rars(out) }),
    );

    h.check("the file name is HLTV's own, never renamed", async {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        ensure(name == archive.suggested_name, || {
            format!(
                "path says {name}, the browser suggested {}",
                archive.suggested_name
            )
        })?;
        ensure(name.to_ascii_lowercase().contains("-vs-"), || {
            format!("{name} does not name the two teams")
        })?;
        ensure(name.ends_with(".rar"), || {
            format!("{name} is not an archive")
        })?;
        let duplicate = regex::Regex::new(r"\(\d+\)\.rar$").unwrap().is_match(&name);
        ensure(!duplicate, || {
            format!("{name} looks like a browser duplicate")
        })?;
        Ok(name)
    })
    .await?;

    let mut dems = Vec::new();
    h.check("the archive really holds .dem files", async {
        let run = spawn(Path::new("bsdtar"), &["-tf", &archive.path], root).await?;
        ensure(run.code == 0, || {
            format!("bsdtar exited {}: {}", run.code, run.tail())
        })?;
        dems = run
            .stdout
            .lines()
            .filter(|line| line.ends_with(".dem"))
            .map(String::from)
            .collect();
        ensure(!dems.is_empty(), || {
            "no .dem in the archive: this is not a demo archive".into()
        })?;
        Ok(format!("{} .dem", dems.len()))
    })
    .await?;
    h.fact("demos", json!(dems));

    h.check("the download is recorded in the manifest", async {
        let entries = manifest::read(out)?;
        ensure(entries.len() == 1, || {
            format!("manifest has {} entries", entries.len())
        })?;
        ensure(entries[0].demo_url == demo.url, || {
            format!("manifest says {}", entries[0].demo_url)
        })?;
        ensure(entries[0].file_name == archive.suggested_name, || {
            format!("manifest says {}", entries[0].file_name)
        })?;
        Ok(format!("{}: {}", manifest::MANIFEST, entries[0].file_name))
    })
    .await?;

    let mut evidence = Value::Null;
    h.check("a repeat download exits 0 and skips", async {
        let run = spawn(cli, &["download", &target.url, "--out", out_arg], root).await?;
        evidence = run.evidence();
        run.succeeded()?;
        ensure(run.stderr.contains("skip "), || {
            format!("the repeat did not skip: {}", run.tail())
        })?;
        ensure(run.stdout.trim() == archive.path, || {
            format!("the repeat reported {}", run.stdout.trim())
        })?;
        Ok(format!("skip in {} ms", run.ms))
    })
    .await?;
    h.fact("cliRepeat", evidence);

    h.check("the repeat downloaded nothing and grew nothing", async {
        let listing = rars(out);
        ensure(listing.len() == 1, || {
            format!("a second archive appeared: {listing:?}")
        })?;
        let entries = manifest::read(out)?;
        ensure(entries.len() == 1, || {
            format!("the manifest grew to {} entries", entries.len())
        })?;
        Ok("one archive, one manifest line".into())
    })
    .await?;

    h.emit("");
    h.emit("== mcp over stdio");

    let (transport, stderr) = TokioChildProcess::builder(tokio::process::Command::new(mcp))
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Problem::Gap(format!("cannot start {}: {e}", mcp.display())))?;
    let logs = Arc::new(Mutex::new(String::new()));
    if let Some(mut stderr) = stderr {
        let logs = logs.clone();
        tokio::spawn(async move {
            let mut buffer = String::new();
            let _ = stderr.read_to_string(&mut buffer).await;
            logs.lock().unwrap().push_str(&buffer);
        });
    }
    let client = ().serve(transport).await.map_err(|e| Problem::Fail(format!("initialize: {e}")))?;
    let outcome = mcp_checks(h, &client, out, target, demo, &archive, cli_matches).await;
    let _ = client.cancel().await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let logs = logs.lock().unwrap().clone();
    h.fact("mcpStderr", json!(logs));
    outcome
}

async fn mcp_checks(
    h: &mut Harness,
    client: &RunningService<RoleClient, ()>,
    out: &Path,
    target: &Match,
    demo: &DemoRef,
    archive: &Archive,
    cli_matches: &[Match],
) -> Outcome<()> {
    let out_arg = out.to_str().expect("a temp path is UTF-8");

    h.check("tools/list advertises the four tools", async {
        let tools = client
            .list_all_tools()
            .await
            .map_err(|e| Problem::Fail(e.to_string()))?;
        let mut names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
        names.sort();
        ensure(
            names
                == [
                    "download_demo",
                    "list_downloads",
                    "resolve_demo",
                    "search_matches",
                ],
            || format!("got {names:?}"),
        )?;
        Ok(names.join(", "))
    })
    .await?;

    let mut evidence = Value::Null;
    h.check("search_matches returns matches the CLI also saw", async {
        let result = call(client, "search_matches", json!({ "limit": 5 })).await?;
        evidence = json!(result);
        let matches: Vec<Match> = answer(&result, "search_matches")?;
        ensure(!matches.is_empty(), || "no matches".into())?;
        let seen: HashSet<u64> = cli_matches.iter().map(|m| m.match_id).collect();
        let shared = matches
            .iter()
            .filter(|m| seen.contains(&m.match_id))
            .count();
        // The results page can gain a match between the two calls; one shared id proves the same source.
        ensure(shared >= 1, || {
            let ids = |list: &[Match]| {
                list.iter()
                    .map(|m| m.match_id.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            };
            format!("MCP {} vs CLI {}", ids(&matches), ids(cli_matches))
        })?;
        ensure(
            matches
                .iter()
                .all(|m| m.url.starts_with(&format!("{BASE}/matches/"))),
            || "a match URL is off hltv.org".into(),
        )?;
        Ok(format!(
            "{} matches, {shared} shared with the CLI",
            matches.len()
        ))
    })
    .await?;
    h.fact("mcpSearch", evidence);

    let mut evidence = Value::Null;
    h.check(
        "resolve_demo returns the same demo URL and match id",
        async {
            let result = call(client, "resolve_demo", json!({ "matchUrl": target.url })).await?;
            evidence = json!(result);
            let found: DemoRef = answer(&result, "resolve_demo")?;
            ensure(found.url == demo.url, || {
                format!("MCP {}, the CLI/library {}", found.url, demo.url)
            })?;
            ensure(found.match_id == target.match_id, || {
                format!("MCP match {}, expected {}", found.match_id, target.match_id)
            })?;
            ensure(found.demo_id == demo.demo_id, || {
                format!("MCP demo {}, expected {}", found.demo_id, demo.demo_id)
            })?;
            Ok(found.url)
        },
    )
    .await?;
    h.fact("mcpResolve", evidence);

    let mut evidence = Value::Null;
    h.check(
        "download_demo reuses the archive the CLI downloaded",
        async {
            let result = call(
                client,
                "download_demo",
                json!({ "url": target.url, "out": out_arg }),
            )
            .await?;
            evidence = json!(result);
            let archives: Vec<Archive> = answer(&result, "download_demo")?;
            ensure(archives.len() == 1, || {
                format!("expected one archive, got {archives:?}")
            })?;
            let mcp = &archives[0];
            ensure(mcp.path == archive.path, || {
                format!("MCP wrote {}, the CLI wrote {}", mcp.path, archive.path)
            })?;
            ensure(mcp.demo_url == demo.url, || {
                format!("MCP demo URL {}", mcp.demo_url)
            })?;
            ensure(mcp.bytes == archive.bytes, || {
                format!("MCP {} bytes, the CLI {}", mcp.bytes, archive.bytes)
            })?;
            Ok(format!(
                "{} (shared manifest, {} bytes)",
                mcp.suggested_name, mcp.bytes
            ))
        },
    )
    .await?;
    h.fact("mcpDownload", evidence);

    h.check("download_demo added no second archive", async {
        let listing = rars(out);
        ensure(listing.len() == 1, || {
            format!("expected one archive after both transports, found {listing:?}")
        })?;
        Ok(listing[0].clone())
    })
    .await?;

    let mut evidence = Value::Null;
    h.check(
        "list_downloads reports the one entry both transports share",
        async {
            let result = call(client, "list_downloads", json!({ "out": out_arg })).await?;
            evidence = json!(result);
            let body: Value = answer(&result, "list_downloads")?;
            let expected = manifest::manifest_path(out);
            ensure(body["manifest"] == json!(expected), || {
                format!("manifest path {}", body["manifest"])
            })?;
            let entries = body["entries"].as_array().cloned().unwrap_or_default();
            ensure(entries.len() == 1, || format!("{} entries", entries.len()))?;
            ensure(entries[0]["demoUrl"] == json!(demo.url), || {
                format!("entry demo URL {}", entries[0]["demoUrl"])
            })?;
            Ok(expected.display().to_string())
        },
    )
    .await?;
    h.fact("mcpList", evidence);

    let mut evidence = Value::Null;
    h.check(
        "an off-site URL is an error result with a remediation, not a crash",
        async {
            let result = call(
                client,
                "resolve_demo",
                json!({ "matchUrl": "https://example.com/matches/1/x" }),
            )
            .await?;
            evidence = json!(result);
            let text = text_of(&result);
            ensure(result.is_error == Some(true), || {
                "the tool returned success for an off-site URL".into()
            })?;
            ensure(text.contains("is not an hltv.org URL"), || {
                format!("text was {:?}", text.chars().take(300).collect::<String>())
            })?;
            ensure(text.contains("/download/demo/"), || {
                "no remediation in the error".into()
            })?;
            Ok(text.lines().next().unwrap_or("").to_string())
        },
    )
    .await?;
    h.fact("mcpError", evidence);

    h.check("the server is still healthy after an error", async {
        let tools = client
            .list_all_tools()
            .await
            .map_err(|e| Problem::Fail(e.to_string()))?;
        ensure(tools.len() == 4, || {
            format!("{} tools after the error", tools.len())
        })?;
        Ok("four tools, still answering".into())
    })
    .await
}

fn report(h: &mut Harness, s: &Settings, outcome: Outcome<()>, started: Instant) -> u8 {
    let count = |status: &str| h.checks.iter().filter(|c| c.status == status).count();
    let (pass, fail, gaps, total) = (count("PASS"), count("FAIL"), count("GAP"), h.checks.len());
    let code = match outcome {
        _ if gaps > 0 => 2,
        Err(Problem::Gap(_)) => 2,
        _ if fail > 0 => 1,
        Err(Problem::Fail(_)) => 1,
        Ok(()) => 0,
    };
    let ms = started.elapsed().as_millis();
    h.fact("totalMs", json!(ms));
    h.emit("");
    h.emit(match code {
        0 => format!("== done: {pass}/{total} checks passed in {:.0} s", ms as f64 / 1000.0),
        2 => format!("== gap: {pass} passed, {gaps} could not run, {fail} failed — exit 2 is a coverage gap, not a pass"),
        _ => format!("== failed: {fail} of {total} checks"),
    });
    let _ = std::fs::write(
        s.evidence.join("summary.txt"),
        format!("{}\n", h.lines.join("\n")),
    );
    let evidence = json!({ "run": s.run, "code": code, "checks": h.checks, "facts": h.facts });
    let _ = std::fs::write(
        s.evidence.join("evidence.json"),
        serde_json::to_string_pretty(&evidence).unwrap_or_default(),
    );
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_days_round_trip() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2026, 10, 24), 20_750);
        assert_eq!(civil_from_days(20_750), (2026, 10, 24));
        assert_eq!(civil_from_days(days_from_civil(2000, 2, 29)), (2000, 2, 29));
    }
}
