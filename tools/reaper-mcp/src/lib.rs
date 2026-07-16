//! Reaper MCP adapter — a stdio MCP server that gates tool calls through a
//! Reaper Agent.
//!
//! This is the reference implementation of "the enforcing edge that labels
//! taint" (F1 agentic authorization): the adapter — not the calling LLM —
//! decides which context keys are platform-derived and which are
//! LLM-asserted. Everything the caller supplies (tool arguments, extra
//! context) is labeled `llm`; only keys the adapter derives itself
//! (`mcp.tool`, `mcp.server`) are labeled `platform`. Taint mode is always
//! on for requests that pass through this edge, so a policy demanding
//! platform trust can never be satisfied by a prompt-injected assertion.
//!
//! Exposed MCP tools:
//! - `authorize_tool_call` — evaluate a proposed tool call against policy
//!   (with optional signed capability) and return allow/deny with the
//!   deciding rule and a `decision_id`.
//! - `explain_decision` — fetch the full decision record (including the
//!   `input_data` explain snapshot) for a previous `decision_id`.

pub mod config;
pub mod protocol;
pub mod tools;

pub use config::AdapterConfig;
pub use protocol::McpServer;
