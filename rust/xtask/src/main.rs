//! Development automation for hltv-demo, run from the crate root as `cargo xtask <task>`.
//! None of it ships: the binaries and the library never depend on this crate.

mod capture;
mod live_check;
mod mcp_call;
mod paths;

use std::process::ExitCode;

const USAGE: &str = "usage: cargo xtask <task>

tasks:
  live-check                    both entry points against the newest match on hltv.org/results
                                (network + Chrome; exit 2 is a coverage gap, not a pass)
  capture-fixtures              re-capture and trim tests/fixtures/*.html (headed Chrome)
  capture-fixtures --trim       re-trim the saved fixtures (headless, no network)
  mcp-call <tool> ['{json}']    call one MCP tool on hltv-demo-mcp over stdio";

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.split_first() {
        Some((task, rest)) if task == "live-check" => live_check::main(rest).await,
        Some((task, rest)) if task == "capture-fixtures" => capture::main(rest).await,
        Some((task, rest)) if task == "mcp-call" => mcp_call::main(rest).await,
        _ => {
            eprintln!("{USAGE}");
            2
        }
    };
    ExitCode::from(code)
}
