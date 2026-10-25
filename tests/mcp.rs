//! The MCP tools as a client sees them, through rmcp's own client over an in-process stream and a
//! fake browser: parameter clamps, and every failure as { message, remediation } instead of a defect.

mod common;

use common::{Fake, row};
use hltv_demo::mcp::HltvServer;
use rmcp::model::{CallToolRequestParams, CallToolResult, ClientConfig};
use rmcp::service::RunningService;
use rmcp::{ClientHandler, RoleClient, ServiceExt};
use serde_json::{Value, json};

#[derive(Clone, Default)]
struct Client;

impl ClientHandler for Client {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::default()
    }
}

async fn connect(fake: Fake) -> RunningService<RoleClient, Client> {
    let (server_io, client_io) = tokio::io::duplex(1 << 16);
    tokio::spawn(async move {
        HltvServer::new(fake)
            .serve(server_io)
            .await
            .unwrap()
            .waiting()
            .await
    });
    Client.serve(client_io).await.unwrap()
}

async fn call(
    client: &RunningService<RoleClient, Client>,
    tool: &'static str,
    args: Value,
) -> CallToolResult {
    let params = CallToolRequestParams::new(tool).with_arguments(args.as_object().unwrap().clone());
    client.call_tool(params).await.unwrap()
}

/// The tool's answer, read from its text content, which is what older clients see.
fn ok(result: CallToolResult) -> Value {
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let text: Value = serde_json::from_str(&result.content[0].as_text().unwrap().text).unwrap();
    assert_eq!(
        Some(&text),
        result.structured_content.as_ref(),
        "text and structured content agree"
    );
    text
}

fn failure(result: CallToolResult) -> (String, String) {
    assert_eq!(result.is_error, Some(true), "{result:?}");
    let value = result.structured_content.unwrap();
    (
        value["message"].as_str().unwrap().into(),
        value["remediation"].as_str().unwrap().into(),
    )
}

#[tokio::test]
async fn search_returns_matches_and_clamps_the_limit_to_1_through_100() {
    let client = connect(Fake::default().rows(vec![row(1), row(2), row(3)])).await;
    let ids = |v: Value| {
        v.as_array()
            .unwrap()
            .iter()
            .map(|m| m["matchId"].as_u64().unwrap())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        ids(ok(
            call(&client, "search_matches", json!({ "limit": 0 })).await
        )),
        vec![1]
    );
    assert_eq!(
        ids(ok(
            call(&client, "search_matches", json!({ "limit": -5 })).await
        )),
        vec![1]
    );
    assert_eq!(
        ids(ok(
            call(&client, "search_matches", json!({ "limit": 500 })).await
        ))
        .len(),
        3
    );
}

#[tokio::test]
async fn search_failures_reach_the_client_as_message_plus_remediation() {
    let client = connect(Fake::default()).await;
    let (message, remediation) =
        failure(call(&client, "search_matches", json!({ "team": 1 })).await);
    assert!(message.contains("lists no matches"), "{message}");
    assert!(remediation.contains("team id"), "{remediation}");
}

#[tokio::test]
async fn a_match_with_no_demo_is_a_tool_failure_not_a_defect() {
    let client = connect(Fake::default()).await;
    let (message, remediation) = failure(
        call(
            &client,
            "resolve_demo",
            json!({ "matchUrl": "/matches/1/x" }),
        )
        .await,
    );
    assert!(message.contains("has no demo download link"), "{message}");
    assert!(
        remediation.contains("may not publish a demo"),
        "{remediation}"
    );
}

#[tokio::test]
async fn a_non_hltv_url_is_refused_with_the_accepted_shapes() {
    let fake = Fake::default();
    let client = connect(fake.clone()).await;
    let (message, remediation) = failure(
        call(
            &client,
            "resolve_demo",
            json!({ "matchUrl": "https://example.com/x" }),
        )
        .await,
    );
    assert!(message.contains("is not an hltv.org URL"), "{message}");
    assert!(remediation.contains("/download/demo/"), "{remediation}");
    assert_eq!(fake.opened(), 0, "refused before a browser opened");
}

#[tokio::test]
async fn the_resolved_demo_reaches_the_client_as_a_url() {
    let client = connect(Fake::default().demo(|_| Some(113159))).await;
    let demo = ok(call(
        &client,
        "resolve_demo",
        json!({ "matchUrl": "/matches/2399723/vitality-vs-9z" }),
    )
    .await);
    assert_eq!(demo["url"], "https://www.hltv.org/download/demo/113159");
    assert_eq!(demo["matchId"], 2399723);
}

#[tokio::test]
async fn download_demo_downloads_into_the_given_folder_and_returns_the_archives() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().to_str().unwrap();
    let client = connect(Fake::default().demo(|_| Some(42))).await;
    let archives = ok(call(
        &client,
        "download_demo",
        json!({ "url": "/matches/7/x", "out": out }),
    )
    .await);
    assert_eq!(archives.as_array().unwrap().len(), 1);
    assert_eq!(
        archives[0]["demoUrl"],
        "https://www.hltv.org/download/demo/42"
    );
    assert!(archives[0]["path"].as_str().unwrap().starts_with(out));
}

#[tokio::test]
async fn downloading_nothing_is_a_failure_the_client_can_act_on() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().to_str().unwrap();
    let client = connect(Fake::default()).await;
    let (message, remediation) = failure(
        call(
            &client,
            "download_demo",
            json!({ "url": "/results", "out": out }),
        )
        .await,
    );
    assert!(!message.is_empty() && !remediation.is_empty());

    let client = connect(Fake::default().rows(vec![row(1)])).await;
    let (message, _) = failure(
        call(
            &client,
            "download_demo",
            json!({ "url": "/results", "out": out }),
        )
        .await,
    );
    assert_eq!(message, "nothing downloaded");
}

#[tokio::test]
async fn list_downloads_reads_the_manifest_of_a_folder() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().to_str().unwrap();
    let client = connect(Fake::default()).await;
    ok(call(
        &client,
        "download_demo",
        json!({ "url": "/download/demo/9", "out": out }),
    )
    .await);
    let listing = ok(call(&client, "list_downloads", json!({ "out": out })).await);
    assert_eq!(
        listing["manifest"],
        dir.path().join(".hltv-demo.tsv").to_str().unwrap()
    );
    assert_eq!(listing["entries"].as_array().unwrap().len(), 1);
    assert_eq!(
        listing["entries"][0]["demoUrl"],
        "https://www.hltv.org/download/demo/9"
    );
    assert!(
        listing["entries"][0]["fileName"]
            .as_str()
            .unwrap()
            .contains("9-event-a-vs-b-bo3-x.rar")
    );
}

#[tokio::test]
async fn a_corrupt_manifest_is_a_failure_the_client_can_act_on_not_an_internal_error() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".hltv-demo.tsv"), "no-tab-here\n").unwrap();
    let client = connect(Fake::default()).await;
    let (message, remediation) = failure(
        call(
            &client,
            "list_downloads",
            json!({ "out": dir.path().to_str().unwrap() }),
        )
        .await,
    );
    assert!(
        message.contains("cannot read the download manifest"),
        "{message}"
    );
    assert!(remediation.contains("delete the manifest"), "{remediation}");
}

#[tokio::test]
async fn a_folder_with_no_manifest_lists_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let client = connect(Fake::default()).await;
    let listing = ok(call(
        &client,
        "list_downloads",
        json!({ "out": dir.path().to_str().unwrap() }),
    )
    .await);
    assert_eq!(listing["entries"], json!([]));
}

#[tokio::test]
async fn a_bad_map_is_refused_before_a_browser_opens() {
    let fake = Fake::default();
    let client = connect(fake.clone()).await;
    let (message, remediation) =
        failure(call(&client, "search_matches", json!({ "map": "cache&team=1" })).await);
    assert!(message.contains("is not a map name"), "{message}");
    assert!(remediation.contains("de_cache"), "{remediation}");
    assert_eq!(fake.opened(), 0);
}

#[tokio::test]
async fn every_tool_publishes_input_and_output_schemas() {
    let client = connect(Fake::default()).await;
    let tools = client.list_all_tools().await.unwrap();
    assert_eq!(tools.len(), 4);
    for tool in &tools {
        assert_eq!(
            tool.input_schema.get("type"),
            Some(&json!("object")),
            "{}",
            tool.name
        );
        assert!(
            tool.output_schema.is_some(),
            "{} has no output schema",
            tool.name
        );
    }
    let download = tools.iter().find(|t| t.name == "download_demo").unwrap();
    let properties = download.input_schema["properties"].as_object().unwrap();
    assert!(
        ["url", "out", "limit", "delayMs"]
            .iter()
            .all(|p| properties.contains_key(*p)),
        "{properties:?}"
    );
}
