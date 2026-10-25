//! Call one MCP tool on this crate's server over stdio, for scripts and manual checks.
//!
//! usage: cargo xtask mcp-call <tool> ['{"json":"arguments"}']
//! Prints the tool's text content on stdout; exit 1 when the tool returns an error result.

use std::process::Stdio;

use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use rmcp::transport::TokioChildProcess;
use serde_json::{Map, Value};

use crate::paths;

pub async fn main(args: &[String]) -> u8 {
    let Some(tool) = args.first() else {
        eprintln!("usage: cargo xtask mcp-call <tool> ['{{json arguments}}']");
        return 2;
    };
    let arguments = match serde_json::from_str::<Map<String, Value>>(
        args.get(1).map_or("{}", String::as_str),
    ) {
        Ok(arguments) => arguments,
        Err(error) => {
            eprintln!("mcp-call: the arguments are not a JSON object: {error}");
            return 2;
        }
    };
    match call(tool, arguments).await {
        Ok((text, is_error)) => {
            println!("{text}");
            u8::from(is_error)
        }
        Err(error) => {
            eprintln!("mcp-call: {error}");
            2
        }
    }
}

async fn call(tool: &str, arguments: Map<String, Value>) -> Result<(String, bool), String> {
    let server = paths::binaries()?.join("hltv-demo-mcp");
    let (transport, _) = TokioChildProcess::builder(tokio::process::Command::new(&server))
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", server.display()))?;
    let client = ().serve(transport).await.map_err(|e| e.to_string())?;
    let params = CallToolRequestParams::new(tool.to_string()).with_arguments(arguments);
    let result = client.call_tool(params).await.map_err(|e| e.to_string());
    let _ = client.cancel().await;
    let result = result?;
    let text = result
        .content
        .iter()
        .filter_map(|c| c.as_text())
        .map(|t| t.text.as_str())
        .collect::<Vec<_>>();
    Ok((text.join("\n"), result.is_error == Some(true)))
}
