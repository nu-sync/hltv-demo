//! Save real hltv.org HTML so the readers can be tested without the network — trimmed to the
//! subtrees they read, never a page dump. Trimming is part of capture, not a cleanup step somebody
//! remembers: what lands in tests/fixtures/ is already pruned, and nothing is written that still
//! carries a script, a tracker, or a per-visitor attribute, or that lost the anchor it exists to test.
//!
//! usage: cargo xtask capture-fixtures          re-capture (headed Chrome on the shared profile)
//!        cargo xtask capture-fixtures --trim   re-trim the saved fixtures (headless, no network)

use std::path::PathBuf;

use hltv_demo::HltvError;
use hltv_demo::browser::{Offline, Session};
use hltv_demo::hltv::extract;
use regex::Regex;
use serde::Deserialize;

use crate::{paths, text};

struct Fixture {
    name: &'static str,
    url: &'static str,
    /// The selector the live read waits for.
    ready: &'static str,
    /// What the trimmed page must still contain, so a challenge page can never overwrite a fixture.
    anchor: &'static str,
}

const FIXTURES: [Fixture; 2] = [
    Fixture {
        name: "results-cache",
        url: "https://www.hltv.org/results?team=9565&map=de_cache",
        ready: extract::RESULTS_READY,
        anchor: r#"class="result-con"#,
    },
    Fixture {
        name: "match",
        url: "https://www.hltv.org/matches/2399723/vitality-vs-9z-pgl-masters-bucharest-2026",
        ready: extract::MATCH_READY,
        anchor: r"/download/demo/\d+",
    },
];

/// A trimmed fixture must not carry what it was trimmed to remove.
const FORBIDDEN: &str = r"<script|<!--|facebook\.com/tr|googletagmanager|doubleclick|cloudflareinsights|data-cf-beacon|data-client-country-iso|Country: [A-Z]{2}-[A-Z]{2}";

const TRIM: &str = include_str!("trim.js");

#[derive(Deserialize)]
struct Trimmed {
    before: usize,
    html: String,
}

pub async fn main(args: &[String]) -> u8 {
    let outcome = match args {
        [] => capture().await,
        [flag] if flag == "--trim" => retrim().await,
        _ => Err("usage: cargo xtask capture-fixtures [--trim]".to_string()),
    };
    match outcome {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("capture-fixtures: {error}");
            1
        }
    }
}

fn dir() -> PathBuf {
    paths::root().join("tests").join("fixtures")
}

/// Refuse a page that kept what it should have lost or lost what it exists to test.
fn checked(fixture: &Fixture, trimmed: Trimmed) -> Result<Trimmed, String> {
    // A comment inside a kept subtree the DOM walk cannot reach (a <template>) carries no structure.
    let html = Regex::new(r"(?s)<!--.*?-->")
        .unwrap()
        .replace_all(&trimmed.html, "")
        .into_owned();
    if let Some(leak) = Regex::new(FORBIDDEN).unwrap().find(&html) {
        let from = text::floor(&html, leak.start().saturating_sub(120));
        let to = text::ceil(&html, leak.end() + 160);
        return Err(format!(
            "{}: trim left {} behind: …{}…",
            fixture.name,
            leak.as_str(),
            &html[from..to]
        ));
    }
    if !Regex::new(fixture.anchor).unwrap().is_match(&html) {
        return Err(format!(
            "{}: the trimmed page lost its anchor {}; not writing",
            fixture.name, fixture.anchor
        ));
    }
    Ok(Trimmed {
        before: trimmed.before,
        html,
    })
}

fn write(fixture: &Fixture, trimmed: &Trimmed) -> Result<(), String> {
    std::fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
    let path = dir().join(format!("{}.html", fixture.name));
    std::fs::write(&path, &trimmed.html).map_err(|e| format!("{}: {e}", path.display()))?;
    let after = trimmed.html.len();
    eprintln!(
        "{}: {} → {after} bytes ({}%), no scripts",
        fixture.name,
        trimmed.before,
        (100 * after).checked_div(trimmed.before).unwrap_or(100)
    );
    Ok(())
}

fn explain(error: HltvError) -> String {
    format!("{}\nNothing was written.", error.explain())
}

/// Through the production read path: the shared profile and its lock, arrival, and the challenge.
async fn capture() -> Result<(), String> {
    let session = Session::open(false).await.map_err(explain)?;
    let mut outcome = Ok(());
    for fixture in &FIXTURES {
        let read = session
            .read::<Trimmed>(fixture.url, fixture.ready, TRIM)
            .await;
        outcome = read
            .map_err(explain)
            .and_then(|trimmed| checked(fixture, trimmed))
            .and_then(|t| write(fixture, &t));
        if outcome.is_err() {
            break;
        }
    }
    session.close().await;
    outcome
}

async fn retrim() -> Result<(), String> {
    let browser = Offline::launch().await.map_err(explain)?;
    let mut outcome = Ok(());
    for fixture in &FIXTURES {
        let path = dir().join(format!("{}.html", fixture.name));
        outcome = match std::fs::read_to_string(&path) {
            Err(e) => Err(format!("{}: {e}", path.display())),
            Ok(original) => browser
                .read_html::<Trimmed>(&original, TRIM)
                .await
                .map_err(explain)
                .and_then(|trimmed| {
                    checked(
                        fixture,
                        Trimmed {
                            before: original.len(),
                            html: trimmed.html,
                        },
                    )
                })
                .and_then(|t| write(fixture, &t)),
        };
        if outcome.is_err() {
            break;
        }
    }
    browser.close().await;
    outcome
}
