use std::process::ExitCode;
use std::time::Duration;

fn main() -> ExitCode {
    let runtime = tokio::runtime::Runtime::new().expect("a tokio runtime");
    let code = runtime.block_on(hltv_demo::mcp::main());
    // Dropping the runtime would wait forever on the blocking read of stdin, which stays open while
    // the client holds the pipe. Shutting down drops every task instead, so a tool call still running
    // closes its Chrome and releases the profile lock, and the process exits.
    runtime.shutdown_timeout(Duration::from_secs(2));
    code
}
