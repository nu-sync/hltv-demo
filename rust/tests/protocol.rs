//! The MCP server over its real transport: the `hltv-demo-mcp` binary as a subprocess speaking
//! newline-delimited JSON-RPC, driven by rmcp's client. Nothing is stubbed except the folder, so this
//! catches protocol regressions (schema, tool names, error mapping) that an in-process call cannot.

use std::process::Stdio;

use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::TokioChildProcess;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};
use tokio::process::Command;

async fn server() -> RunningService<RoleClient, ()> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_hltv-demo-mcp"));
    // No call here may need a browser; a launch would fail on this path instead of opening Chrome.
    command.env("CHROME", "/nonexistent/chrome");
    let (transport, _) = TokioChildProcess::builder(command)
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    ().serve(transport).await.unwrap()
}

async fn call(
    client: &RunningService<RoleClient, ()>,
    tool: &'static str,
    args: Value,
) -> CallToolResult {
    let params = CallToolRequestParams::new(tool).with_arguments(args.as_object().unwrap().clone());
    client.call_tool(params).await.unwrap()
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

#[tokio::test]
async fn advertises_the_four_tools_with_descriptions_and_input_schemas() {
    let client = server().await;
    let info = client.peer_info().unwrap();
    assert_eq!(
        info.server_info.as_ref().map(|s| s.name.as_str()),
        Some("hltv-demo")
    );
    assert!(
        info.instructions
            .as_deref()
            .unwrap_or("")
            .contains("search_matches → resolve_demo → download_demo")
    );

    let tools = client.list_all_tools().await.unwrap();
    let mut names: Vec<_> = tools.iter().map(|t| t.name.to_string()).collect();
    names.sort();
    assert_eq!(
        names,
        [
            "download_demo",
            "list_downloads",
            "resolve_demo",
            "search_matches"
        ]
    );
    for tool in &tools {
        assert!(
            tool.description.as_deref().unwrap_or("").len() > 20,
            "{}",
            tool.name
        );
        assert_eq!(
            tool.input_schema.get("type"),
            Some(&json!("object")),
            "{}",
            tool.name
        );
    }
    let search = tools.iter().find(|t| t.name == "search_matches").unwrap();
    let properties = search.input_schema["properties"].as_object().unwrap();
    assert!(
        ["team", "map", "limit"]
            .iter()
            .all(|p| properties.contains_key(*p)),
        "{properties:?}"
    );
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn list_downloads_answers_with_the_manifest_and_its_entries() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(".hltv-demo.tsv"),
        "https://www.hltv.org/download/demo/5\tevent-a-vs-b-bo3-x.rar\n",
    )
    .unwrap();
    let client = server().await;
    let result = call(
        &client,
        "list_downloads",
        json!({ "out": dir.path().to_str().unwrap() }),
    )
    .await;
    let body: Value = serde_json::from_str(&text_of(&result)).unwrap();
    assert_eq!(
        body["manifest"],
        dir.path().join(".hltv-demo.tsv").to_str().unwrap()
    );
    assert_eq!(
        body["entries"],
        json!([{ "demoUrl": "https://www.hltv.org/download/demo/5", "fileName": "event-a-vs-b-bo3-x.rar" }])
    );
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn a_bad_url_comes_back_as_an_error_result_with_a_remediation_not_a_crash() {
    let client = server().await;
    let result = call(
        &client,
        "resolve_demo",
        json!({ "matchUrl": "https://example.com/matches/1/x" }),
    )
    .await;
    assert_eq!(result.is_error, Some(true));
    let text = text_of(&result);
    assert!(text.contains("is not an hltv.org URL"), "{text}");
    assert!(text.contains("/download/demo/"), "{text}");

    // ...and the server is still healthy after it.
    assert_eq!(client.list_all_tools().await.unwrap().len(), 4);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn a_browser_failure_is_an_error_result_too() {
    let client = server().await;
    let result = call(&client, "search_matches", json!({ "team": 9565 })).await;
    assert_eq!(result.is_error, Some(true));
    let text = text_of(&result);
    assert!(
        text.contains("no Chrome found at /nonexistent/chrome"),
        "{text}"
    );
    assert!(text.contains("CHROME=/path/to/chrome"), "{text}");
    client.cancel().await.unwrap();
}
