//! CLI. Argument parsing and formatting live here; every rule lives in `core` or `hltv`.
//!
//! Exit codes: 0 success, 1 a domain failure, 2 usage error or missing prerequisite; 128 + n when a
//! signal stopped the run (Chrome is closed and the profile released first).

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Args, CommandFactory, Parser, Subcommand};
use serde::Serialize;

use crate::browser::Chrome;
use crate::core::errors::HltvError;
use crate::core::types::{Match, SearchFilter};
use crate::core::url::{classify, map_filter};
use crate::hltv::get::{Options, get};
use crate::hltv::resolve::resolve;
use crate::hltv::search::{MAX_LIMIT, search};
use crate::process;
use crate::source::{Opener, with_source};

const USAGE: &str = "hltv-demo search [--team ID] [--map NAME] [--limit N] [--offset N] [--json]
       hltv-demo resolve <match-url> [--json]
       hltv-demo download <url> [--out DIR] [--limit N] [--delay MS] [--json]";

#[derive(Debug, Parser)]
#[command(
    name = "hltv-demo",
    version,
    about = "hltv-demo — find and download HLTV demo archives",
    override_usage = USAGE,
    after_help = "urls: a match page, a results page, or /download/demo/<id>\n\
                  env:  CHROME=/path/to/chrome  HLTV_DEMO_PROFILE=/path/to/profile  HLTV_DEMO_OUT=/path"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// List recent matches, newest first
    Search {
        /// HLTV team id, e.g. 9565 as in hltv.org/team/9565/vitality
        #[arg(long, value_name = "ID")]
        team: Option<u64>,
        /// Map name, e.g. cache or de_cache
        #[arg(long, value_name = "NAME", value_parser = parse_map)]
        map: Option<String>,
        #[command(flatten)]
        limit: Limit,
        /// Results to skip, for paging
        #[arg(long, value_name = "N")]
        offset: Option<u64>,
        #[command(flatten)]
        output: Output,
    },
    /// Find the demo archive URL of a match page
    Resolve {
        #[arg(value_name = "MATCH_URL")]
        url: String,
        #[command(flatten)]
        output: Output,
    },
    /// Download the archive for a match, results, or demo URL
    Download {
        url: String,
        /// Destination folder (default: ~/.cache/hltv-demo/archives)
        #[arg(long, value_name = "DIR")]
        out: Option<PathBuf>,
        #[command(flatten)]
        limit: Limit,
        /// Pause between matches in ms (default 2000)
        #[arg(long, value_name = "MS")]
        delay: Option<u64>,
        #[command(flatten)]
        output: Output,
    },
}

impl Command {
    fn output(&self) -> &Output {
        match self {
            Self::Search { output, .. }
            | Self::Resolve { output, .. }
            | Self::Download { output, .. } => output,
        }
    }
}

#[derive(Debug, Args)]
pub struct Limit {
    /// Max matches, 1..=100; page with --offset for more
    #[arg(long, value_name = "N", value_parser = parse_limit)]
    limit: Option<usize>,
}

#[derive(Debug, Args)]
pub struct Output {
    /// One JSON object per line on stdout, and nothing else there
    #[arg(long)]
    json: bool,
    /// Not a supported mode: headless Chrome is challenged, and the flag makes that legible
    #[arg(long, hide = true)]
    headless: bool,
}

fn parse_limit(raw: &str) -> Result<usize, String> {
    match raw.parse::<usize>() {
        Ok(n) if (1..=MAX_LIMIT).contains(&n) => Ok(n),
        _ => Err(format!(
            "must be between 1 and {MAX_LIMIT}; page with --offset instead"
        )),
    }
}

/// A map name that cannot be one is a usage error, before Chrome opens.
fn parse_map(raw: &str) -> Result<String, String> {
    map_filter(raw)
        .map(|_| raw.to_string())
        .map_err(|e| e.to_string())
}

fn format_match(m: &Match) -> String {
    format!(
        "{:<9} {:<11} {:<42} {:<5} {} {}",
        m.match_id,
        m.date.as_deref().unwrap_or("?"),
        format!("{} vs {}", m.teams.0, m.teams.1),
        m.scores
            .map_or("-".to_string(), |(a, b)| format!("{a}-{b}")),
        m.event.as_deref().unwrap_or(""),
        m.url,
    )
}

fn line(json: bool, value: &impl Serialize, human: String) -> String {
    if json {
        serde_json::to_string(value).expect("plain data serializes")
    } else {
        human
    }
}

/// Run one command; what it prints on stdout, one line per result (JSON with --json).
pub async fn run(command: Command, opener: &impl Opener) -> Result<Vec<String>, HltvError> {
    match command {
        Command::Search {
            team,
            map,
            limit,
            offset,
            output,
        } => {
            let filter = SearchFilter {
                team,
                map,
                limit: limit.limit,
                offset,
            };
            let matches = with_source(opener, async |source| search(source, &filter).await).await?;
            Ok(matches
                .iter()
                .map(|m| line(output.json, m, format_match(m)))
                .collect())
        }
        Command::Resolve { url, output } => {
            // A URL that is not on hltv.org fails here, before anything launches a browser.
            classify(&url)?;
            let demo = with_source(opener, async |source| resolve(source, &url).await).await?;
            Ok(vec![line(output.json, &demo, demo.url.clone())])
        }
        Command::Download {
            url,
            out,
            limit,
            delay,
            output,
        } => {
            classify(&url)?;
            let quiet = output.json;
            let on_note = move |text: &str| {
                if !quiet {
                    eprintln!("{text}")
                }
            };
            let options = Options {
                out,
                limit: limit.limit,
                delay: delay.map(Duration::from_millis),
                on_note: Some(&on_note),
            };
            let archives =
                with_source(opener, async |source| get(source, &url, &options).await).await?;
            Ok(archives
                .iter()
                .map(|a| line(output.json, a, a.path.clone()))
                .collect())
        }
    }
}

pub async fn main() -> ExitCode {
    process::init_logging();
    let Some(command) = Cli::parse().command else {
        let _ = Cli::command().print_help();
        return ExitCode::SUCCESS;
    };
    if command.output().headless {
        // SAFETY: set before the run starts, while this is the only thread reading the environment.
        unsafe { std::env::set_var("HLTV_DEMO_HEADLESS", "1") };
    }
    // The run is its own task so that a signal can drop it, which closes Chrome and releases the
    // profile lock, and so that a panic is an exit 1 like any other failure rather than Rust's 101.
    let mut work = tokio::spawn(async move { run(command, &Chrome).await });
    let finished = tokio::select! {
        finished = &mut work => finished,
        signal = process::shutdown() => {
            work.abort();
            let _ = work.await;
            eprintln!("hltv-demo: stopped by {signal}");
            return ExitCode::from(signal.exit_code());
        }
    };
    match finished {
        Err(panic) => {
            eprintln!("hltv-demo: internal error: {panic}");
            ExitCode::from(1)
        }
        // Only a results download comes back empty: every match lacked a demo or failed.
        Ok(Ok(lines)) if lines.is_empty() => {
            eprintln!("hltv-demo: nothing downloaded");
            ExitCode::from(1)
        }
        Ok(Ok(lines)) => {
            for line in lines {
                println!("{line}");
            }
            ExitCode::SUCCESS
        }
        Ok(Err(error)) => {
            eprintln!("{}", error.explain());
            ExitCode::from(error.exit_code())
        }
    }
}
