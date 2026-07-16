//! Minimal MCP server over newline-delimited JSON-RPC 2.0 (stdio transport).
//!
//! Implements exactly what the adapter needs — `initialize`, `ping`,
//! `tools/list`, `tools/call` — with no external MCP dependency, keeping the
//! supply-chain surface at zero new crates. Protocol errors are JSON-RPC
//! errors; tool execution failures (agent unreachable, unknown decision id)
//! are MCP tool results with `isError: true`, so a deny or a transport
//! failure never kills the session.

use serde::Deserialize;
use serde_json::{json, Value};
use tracing::{debug, warn};

use crate::config::AdapterConfig;
use crate::tools::{
    authorize_result, build_eval_request, tool_descriptors, valid_decision_id, AgentEvalResponse,
    AuthorizeArgs,
};

/// MCP protocol revisions this server accepts (latest first).
const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

#[derive(Debug, Deserialize)]
struct JsonRpcMessage {
    #[allow(dead_code)]
    jsonrpc: Option<String>,
    id: Option<Value>,
    method: Option<String>,
    params: Option<Value>,
}

/// The MCP server: adapter config + a client to the Reaper Agent.
pub struct McpServer {
    config: AdapterConfig,
    client: reaper_sdk::ReaperClient,
}

impl McpServer {
    /// Build the server, connecting the SDK client per the configured
    /// transport (HTTP or Unix socket).
    pub fn new(config: AdapterConfig) -> anyhow::Result<Self> {
        let client = reaper_sdk::ReaperClient::from_transport(config.transport.clone())?;
        Ok(Self { config, client })
    }

    /// Handle one newline-delimited JSON-RPC message. Returns the response
    /// line to write, or `None` for notifications (which get no response).
    pub async fn handle_message(&self, line: &str) -> Option<String> {
        let msg: JsonRpcMessage = match serde_json::from_str(line) {
            Ok(m) => m,
            Err(e) => {
                warn!("unparseable JSON-RPC message: {e}");
                return Some(error_response(Value::Null, -32700, "Parse error"));
            }
        };
        let Some(method) = msg.method.clone() else {
            // A message with no method (e.g. a stray response) — ignore.
            return None;
        };
        debug!(method = %method, "handling request");

        match (method.as_str(), msg.id) {
            ("initialize", Some(id)) => Some(self.initialize(id, msg.params)),
            ("ping", Some(id)) => Some(result_response(id, json!({}))),
            ("tools/list", Some(id)) => {
                Some(result_response(id, json!({ "tools": tool_descriptors() })))
            }
            ("tools/call", Some(id)) => Some(self.tools_call(id, msg.params).await),
            // Notifications (no id): acknowledged silently.
            (_, None) => None,
            (_, Some(id)) => Some(error_response(id, -32601, "Method not found")),
        }
    }

    fn initialize(&self, id: Value, params: Option<Value>) -> String {
        let requested = params
            .as_ref()
            .and_then(|p| p.get("protocolVersion"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let version = if SUPPORTED_PROTOCOL_VERSIONS.contains(&requested) {
            requested
        } else {
            SUPPORTED_PROTOCOL_VERSIONS[0]
        };
        result_response(
            id,
            json!({
                "protocolVersion": version,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": {
                    "name": "reaper-mcp",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "instructions": "Authorization gate backed by a Reaper Agent. Call authorize_tool_call BEFORE executing any tool call; only proceed when allowed=true. Use explain_decision with the returned decision_id to see why a decision was made. Tool arguments and caller context are treated as LLM-asserted (llm trust); policies requiring platform trust cannot be satisfied by caller-supplied values.",
            }),
        )
    }

    async fn tools_call(&self, id: Value, params: Option<Value>) -> String {
        let Some(params) = params else {
            return error_response(id, -32602, "Missing params");
        };
        let name = params.get("name").and_then(Value::as_str).unwrap_or("");
        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));

        let outcome = match name {
            "authorize_tool_call" => self.run_authorize(arguments).await,
            "explain_decision" => self.run_explain(arguments).await,
            other => Err(format!("Unknown tool: {other}")),
        };

        match outcome {
            Ok(structured) => {
                let text = serde_json::to_string_pretty(&structured)
                    .unwrap_or_else(|_| structured.to_string());
                result_response(
                    id,
                    json!({
                        "content": [{ "type": "text", "text": text }],
                        "structuredContent": structured,
                        "isError": false,
                    }),
                )
            }
            Err(message) => result_response(
                id,
                json!({
                    "content": [{ "type": "text", "text": message }],
                    "isError": true,
                }),
            ),
        }
    }

    async fn run_authorize(&self, arguments: Value) -> Result<Value, String> {
        let args: AuthorizeArgs = serde_json::from_value(arguments)
            .map_err(|e| format!("Invalid authorize_tool_call arguments: {e}"))?;
        let request = build_eval_request(&self.config, &args)?;
        let response: AgentEvalResponse = self
            .client
            .post_json("/api/v1/messages", &request)
            .await
            .map_err(|e| format!("Reaper Agent evaluation failed: {e}"))?;
        Ok(authorize_result(&response))
    }

    async fn run_explain(&self, arguments: Value) -> Result<Value, String> {
        let decision_id = arguments
            .get("decision_id")
            .and_then(Value::as_str)
            .unwrap_or("");
        if !valid_decision_id(decision_id) {
            return Err(
                "Invalid decision_id: expected the id returned by authorize_tool_call".to_string(),
            );
        }
        let record: Value = self
            .client
            .get_json(&format!("/api/v1/decisions/{decision_id}"))
            .await
            .map_err(|e| {
                let msg = e.to_string();
                if msg.contains("HTTP 404") {
                    format!(
                        "Decision '{decision_id}' not found — the ring buffer may have wrapped, \
                         or decision logging is disabled on the agent \
                         (REAPER_DECISION_LOG_ENABLED=true)"
                    )
                } else {
                    format!("Reaper Agent decision lookup failed: {msg}")
                }
            })?;
        Ok(record)
    }
}

fn result_response(id: Value, result: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}

fn error_response(id: Value, code: i64, message: &str) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }).to_string()
}
