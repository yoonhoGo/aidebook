use aidebook_lib::core::{
    mcp, Core, CoreClient, CoreEndpoint, CoreServer, FixtureAdapter, SearchRequest,
};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use uuid::Uuid;

#[test]
fn cli_and_mcp_protocols_share_one_authenticated_core_owner() {
    let directory = PathBuf::from(format!("/tmp/aidebook-m4-{}", Uuid::new_v4()));
    fs::create_dir_all(&directory).expect("ipc directory");
    let endpoint = CoreEndpoint::in_data_dir(&directory);
    let core = Core::in_memory().expect("core");
    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/github.json");
    let fixture = FixtureAdapter::from_path(&fixture_path).expect("fixture");
    core.sources_refresh(&fixture).expect("seed owner core");
    let direct = core
        .context_search(SearchRequest {
            query: "native".to_string(),
            provider: Some("github".to_string()),
            kind: Some("issue".to_string()),
            source_updated_after: None,
            source_updated_before: None,
            max_age_seconds: None,
            limit: Some(20),
        })
        .expect("direct core search");
    let server = CoreServer::bind(endpoint.clone(), core, Some("m4-test-secret".to_string()))
        .expect("single owner");
    let stop = Arc::new(AtomicBool::new(false));
    let stop_for_thread = stop.clone();
    let thread = std::thread::spawn(move || server.serve_until(stop_for_thread));
    let client = CoreClient::with_token(endpoint.clone(), "m4-test-secret").expect("client");
    let via_ipc: aidebook_lib::core::SearchResponse = serde_json::from_value(
        client
            .call(
                "context.search",
                serde_json::to_value(SearchRequest {
                    query: "native".to_string(),
                    provider: Some("github".to_string()),
                    kind: Some("issue".to_string()),
                    source_updated_after: None,
                    source_updated_before: None,
                    max_age_seconds: None,
                    limit: Some(20),
                })
                .unwrap(),
            )
            .expect("IPC search"),
    )
    .expect("search response");
    assert_eq!(via_ipc, direct);

    let tools: Value = serde_json::from_str(
        &mcp::handle_message(
            &client,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#,
        )
        .expect("MCP tools response"),
    )
    .expect("tools JSON");
    assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 6);
    let mcp_search: Value = serde_json::from_str(
        &mcp::handle_message(
            &client,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"context.search","arguments":{"query":"native","provider":"github","kind":"issue","limit":20}}}"#,
        )
        .expect("MCP call response"),
    )
    .expect("MCP JSON");
    assert_eq!(mcp_search["result"]["isError"], false);
    let refresh = client
        .call(
            "sources.refresh",
            serde_json::json!({"fixture_path": fixture_path}),
        )
        .expect("explicit fixture refresh");
    assert_eq!(refresh["provider"], "github");

    let wrong = CoreClient::with_token(endpoint.clone(), "wrong").expect("wrong client");
    let error = wrong
        .call("context.search", serde_json::json!({"query":"native"}))
        .expect_err("wrong token");
    assert!(
        matches!(error, aidebook_lib::core::CoreError::Provider { code, .. } if code == "unauthenticated")
    );
    stop.store(true, Ordering::Relaxed);
    thread.join().expect("owner thread").expect("owner stop");
    fs::remove_dir_all(directory).expect("cleanup");
}
