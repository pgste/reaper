//! Integration tests: the MCP server driven end-to-end against an
//! in-process mock Reaper Agent, plus a stdio smoke test that drives the
//! real binary over piped newline-delimited JSON-RPC.

// Test-only code: the ergonomic unwrap/expect idiom is fine here (the
// workspace gate targets reachable production code).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};

use reaper_mcp::{AdapterConfig, McpServer};

/// Requests the mock agent received, so tests can assert on the wire shape
/// the adapter actually produced.
#[derive(Clone, Default)]
struct Captured {
    eval_requests: Arc<Mutex<Vec<Value>>>,
}

async fn mock_eval(State(cap): State<Captured>, Json(body): Json<Value>) -> Json<Value> {
    // Deny anything whose capability is absent; allow otherwise — enough to
    // exercise both decision paths through the adapter.
    let has_capability = body.get("capability").is_some();
    cap.eval_requests.lock().unwrap().push(body);
    Json(json!({
        "decision_id": "550e8400-e29b-41d4-a716-446655440000",
        "decision": if has_capability { "allow" } else { "deny" },
        "policy_id": "p-1",
        "policy_version": 1,
        "evaluation_time_microseconds": 0.42,
        "total_time_microseconds": 1.0,
        "matched_rule": if has_capability { "tools_allow" } else { "capability_missing" },
        "agent_id": "mock-agent",
        "cache_hit": false,
    }))
}

async fn mock_decision(Path(id): Path<String>) -> Result<Json<Value>, axum::http::StatusCode> {
    if id == "550e8400-e29b-41d4-a716-446655440000" {
        Ok(Json(json!({
            "enabled": true,
            "decision": {
                "decision_id": id,
                "decision": "allow",
                "matched_rule": "tools_allow",
                "input_data": { "actor": { "type": "agent" } },
            }
        })))
    } else {
        Err(axum::http::StatusCode::NOT_FOUND)
    }
}

/// Start the mock agent on an ephemeral port; returns its base URL and the
/// capture handle.
async fn start_mock_agent() -> (String, Captured) {
    let captured = Captured::default();
    let app = Router::new()
        .route("/api/v1/messages", post(mock_eval))
        .route("/api/v1/decisions/{decision_id}", get(mock_decision))
        .with_state(captured.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), captured)
}

fn adapter_config(agent_url: &str) -> AdapterConfig {
    AdapterConfig {
        transport: reaper_sdk::Transport::http(agent_url),
        default_policy: Some("mcp-gate".to_string()),
        default_principal: Some("user_alice".to_string()),
        default_actor: Some("agent_claude".to_string()),
        default_capability: None,
        server_label: Some("files-server".to_string()),
    }
}

fn parse(line: &str) -> Value {
    serde_json::from_str(line).unwrap()
}

#[tokio::test]
async fn mcp_handshake_and_tool_listing() {
    let (url, _cap) = start_mock_agent().await;
    let server = McpServer::new(adapter_config(&url)).unwrap();

    let init = server
        .handle_message(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}"#,
        )
        .await
        .unwrap();
    let init = parse(&init);
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(init["result"]["serverInfo"]["name"], "reaper-mcp");

    // Unknown requested version → server answers with its latest supported.
    let init2 = server
        .handle_message(
            r#"{"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"9999-01-01"}}"#,
        )
        .await
        .unwrap();
    assert_eq!(parse(&init2)["result"]["protocolVersion"], "2025-06-18");

    // The initialized notification gets no response.
    assert!(server
        .handle_message(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
        .await
        .is_none());

    let list = server
        .handle_message(r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#)
        .await
        .unwrap();
    let list = parse(&list);
    let tools = list["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["authorize_tool_call", "explain_decision"]);

    // Unknown method with an id → JSON-RPC error, not a crash.
    let err = server
        .handle_message(r#"{"jsonrpc":"2.0","id":4,"method":"resources/list"}"#)
        .await
        .unwrap();
    assert_eq!(parse(&err)["error"]["code"], -32601);
}

#[tokio::test]
async fn authorize_deny_and_allow_with_taint_labels_on_the_wire() {
    let (url, cap) = start_mock_agent().await;
    let server = McpServer::new(adapter_config(&url)).unwrap();

    // No capability → mock denies.
    let deny = server
        .handle_message(
            r#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"authorize_tool_call","arguments":{"tool":"delete_file","args":{"path":"/tmp/x"},"context":{"approval_level":"admin"}}}}"#,
        )
        .await
        .unwrap();
    let deny = parse(&deny);
    let structured = &deny["result"]["structuredContent"];
    assert_eq!(
        deny["result"]["isError"], false,
        "deny is a result, not an error"
    );
    assert_eq!(structured["allowed"], false);
    assert_eq!(structured["matched_rule"], "capability_missing");
    assert_eq!(
        structured["decision_id"],
        "550e8400-e29b-41d4-a716-446655440000"
    );

    // With a capability → mock allows.
    let allow = server
        .handle_message(
            r#"{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"authorize_tool_call","arguments":{"tool":"read_file","capability":{"grants":[]}}}}"#,
        )
        .await
        .unwrap();
    let allow = parse(&allow);
    assert_eq!(allow["result"]["structuredContent"]["allowed"], true);
    assert_eq!(
        allow["result"]["structuredContent"]["matched_rule"],
        "tools_allow"
    );

    // The wire requests carried the enforcing-edge taint contract.
    let seen = cap.eval_requests.lock().unwrap();
    assert_eq!(seen.len(), 2);
    let first = &seen[0];
    assert_eq!(first["principal"], "user_alice");
    assert_eq!(first["actor"], "agent_claude");
    assert_eq!(first["policy_name"], "mcp-gate");
    assert_eq!(first["action"], "call");
    assert_eq!(first["resource"], "tool:delete_file");
    assert_eq!(first["context"]["arg.path"], "/tmp/x");
    assert_eq!(first["context"]["mcp.tool"], "delete_file");
    assert_eq!(first["context_provenance"]["arg.path"], "llm");
    assert_eq!(first["context_provenance"]["approval_level"], "llm");
    assert_eq!(first["context_provenance"]["mcp.tool"], "platform");
    assert_eq!(first["context_provenance"]["mcp.server"], "platform");
    assert!(first.get("capability").is_none());
    assert!(seen[1].get("capability").is_some());
}

#[tokio::test]
async fn explain_roundtrip_and_not_found() {
    let (url, _cap) = start_mock_agent().await;
    let server = McpServer::new(adapter_config(&url)).unwrap();

    let ok = server
        .handle_message(
            r#"{"jsonrpc":"2.0","id":20,"method":"tools/call","params":{"name":"explain_decision","arguments":{"decision_id":"550e8400-e29b-41d4-a716-446655440000"}}}"#,
        )
        .await
        .unwrap();
    let ok = parse(&ok);
    assert_eq!(ok["result"]["isError"], false);
    assert_eq!(
        ok["result"]["structuredContent"]["decision"]["matched_rule"],
        "tools_allow"
    );

    // Unknown id → tool error with a useful hint, not a protocol failure.
    let missing = server
        .handle_message(
            r#"{"jsonrpc":"2.0","id":21,"method":"tools/call","params":{"name":"explain_decision","arguments":{"decision_id":"00000000-0000-0000-0000-000000000000"}}}"#,
        )
        .await
        .unwrap();
    let missing = parse(&missing);
    assert_eq!(missing["result"]["isError"], true);
    let text = missing["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("not found"));

    // Path-injection attempt → rejected before any request is made.
    let bad = server
        .handle_message(
            r#"{"jsonrpc":"2.0","id":22,"method":"tools/call","params":{"name":"explain_decision","arguments":{"decision_id":"../admin"}}}"#,
        )
        .await
        .unwrap();
    assert_eq!(parse(&bad)["result"]["isError"], true);
}

#[tokio::test]
async fn agent_unreachable_is_a_tool_error_not_a_crash() {
    // Point at a port nothing listens on.
    let server = McpServer::new(adapter_config("http://127.0.0.1:9")).unwrap();
    let resp = server
        .handle_message(
            r#"{"jsonrpc":"2.0","id":30,"method":"tools/call","params":{"name":"authorize_tool_call","arguments":{"tool":"x"}}}"#,
        )
        .await
        .unwrap();
    let resp = parse(&resp);
    assert_eq!(resp["result"]["isError"], true);
    assert!(resp["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("evaluation failed"));
}

/// Drive the real binary over stdio: spawn `reaper-mcp` pointed at the mock
/// agent, pipe a handshake + tool call through stdin, and read the
/// newline-delimited responses back.
#[tokio::test]
async fn stdio_binary_smoke() {
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command, Stdio};

    let (url, _cap) = start_mock_agent().await;

    let mut child = Command::new(env!("CARGO_BIN_EXE_reaper-mcp"))
        .env("REAPER_MCP_AGENT_URL", &url)
        .env("REAPER_MCP_PRINCIPAL", "user_alice")
        .env("REAPER_MCP_ACTOR", "agent_claude")
        .env("REAPER_MCP_POLICY", "mcp-gate")
        .env_remove("REAPER_MCP_AGENT_SOCKET")
        .env_remove("REAPER_MCP_CAPABILITY_FILE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();

    // Blocking pipe I/O on a worker thread so the async mock agent stays live.
    let lines = tokio::task::spawn_blocking(move || {
        writeln!(
            stdin,
            r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2025-06-18"}}}}"#
        )
        .unwrap();
        writeln!(stdin, r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#).unwrap();
        writeln!(stdin, r#"{{"jsonrpc":"2.0","id":2,"method":"tools/list"}}"#).unwrap();
        writeln!(
            stdin,
            r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"authorize_tool_call","arguments":{{"tool":"read_file","capability":{{"grants":[]}}}}}}}}"#
        )
        .unwrap();
        drop(stdin); // EOF → clean shutdown after responses flush

        let reader = BufReader::new(stdout);
        let lines: Vec<String> = reader.lines().map(|l| l.unwrap()).collect();
        lines
    })
    .await
    .unwrap();

    let status = child.wait().unwrap();
    assert!(status.success(), "binary exited nonzero: {status:?}");

    assert_eq!(
        lines.len(),
        3,
        "3 requests with ids → 3 responses: {lines:?}"
    );
    assert_eq!(
        parse(&lines[0])["result"]["serverInfo"]["name"],
        "reaper-mcp"
    );
    assert_eq!(
        parse(&lines[1])["result"]["tools"][0]["name"],
        "authorize_tool_call"
    );
    let call = parse(&lines[2]);
    assert_eq!(call["result"]["structuredContent"]["allowed"], true);
    assert_eq!(
        call["result"]["structuredContent"]["matched_rule"],
        "tools_allow"
    );
}
