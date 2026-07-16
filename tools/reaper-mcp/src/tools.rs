//! The two MCP tools and their mapping onto the Reaper Agent API.
//!
//! `authorize_tool_call` is where the taint contract lives: every
//! caller-supplied value (tool arguments, extra context) is labeled `llm`;
//! only adapter-derived keys (`mcp.tool`, `mcp.server`) are labeled
//! `platform`. The provenance map always accompanies the request, so taint
//! mode is unconditionally on for traffic through this edge — a caller
//! cannot opt out, and cannot raise its own trust.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::AdapterConfig;

/// Context keys the adapter derives itself — the only `platform`-trusted ones.
pub const CTX_TOOL: &str = "mcp.tool";
/// See [`CTX_TOOL`].
pub const CTX_SERVER: &str = "mcp.server";

/// Arguments accepted by `authorize_tool_call` (all beyond `tool` optional;
/// unspecified fields fall back to the adapter's environment defaults).
#[derive(Debug, Deserialize)]
pub struct AuthorizeArgs {
    /// Name of the tool the caller wants to invoke.
    pub tool: String,
    /// LLM-proposed tool arguments. Flattened into context as `arg.<key>`
    /// and labeled `llm`.
    #[serde(default)]
    pub args: Option<serde_json::Map<String, Value>>,
    /// Principal on whose behalf the call is made.
    #[serde(default)]
    pub principal: Option<String>,
    /// Non-human actor performing the call (the agent identity).
    #[serde(default)]
    pub actor: Option<String>,
    /// Action verb; defaults to `call`.
    #[serde(default)]
    pub action: Option<String>,
    /// Resource identifier; defaults to `tool:<tool>`.
    #[serde(default)]
    pub resource: Option<String>,
    /// Policy name; defaults to `REAPER_MCP_POLICY`.
    #[serde(default)]
    pub policy: Option<String>,
    /// Signed capability (opaque JSON, forwarded verbatim); defaults to the
    /// capability loaded from `REAPER_MCP_CAPABILITY_FILE`.
    #[serde(default)]
    pub capability: Option<Value>,
    /// Extra caller-supplied context. Labeled `llm` unconditionally — the
    /// adapter never trusts trust claims from the caller's body.
    #[serde(default)]
    pub context: Option<BTreeMap<String, String>>,
}

/// Wire request for `POST /api/v1/messages` (subset the adapter uses).
#[derive(Debug, Serialize, PartialEq)]
pub struct AgentEvalRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_name: Option<String>,
    pub principal: String,
    pub resource: String,
    pub action: String,
    pub context: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    /// Always present: taint mode is unconditionally on through this edge.
    pub context_provenance: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capability: Option<Value>,
}

/// Wire response from `POST /api/v1/messages` (fields the adapter surfaces).
#[derive(Debug, Deserialize)]
pub struct AgentEvalResponse {
    pub decision_id: String,
    pub decision: String,
    #[serde(default)]
    pub policy_id: String,
    #[serde(default)]
    pub matched_rule: String,
    #[serde(default)]
    pub agent_id: String,
    #[serde(default)]
    pub evaluation_time_microseconds: f64,
}

/// Map `authorize_tool_call` arguments (+ adapter defaults) to the agent
/// request. Pure — unit-testable without a network.
pub fn build_eval_request(
    cfg: &AdapterConfig,
    args: &AuthorizeArgs,
) -> Result<AgentEvalRequest, String> {
    if args.tool.is_empty() {
        return Err("'tool' must be a non-empty string".to_string());
    }
    let principal = args
        .principal
        .clone()
        .or_else(|| cfg.default_principal.clone())
        .ok_or_else(|| "no principal: pass 'principal' or set REAPER_MCP_PRINCIPAL".to_string())?;

    let mut context: BTreeMap<String, String> = BTreeMap::new();
    let mut provenance: BTreeMap<String, String> = BTreeMap::new();

    // Caller-supplied context: llm trust, no exceptions.
    if let Some(extra) = &args.context {
        for (k, v) in extra {
            context.insert(k.clone(), v.clone());
            provenance.insert(k.clone(), "llm".to_string());
        }
    }
    // LLM-proposed tool arguments: flattened as `arg.<key>`, llm trust.
    if let Some(tool_args) = &args.args {
        for (k, v) in tool_args {
            let rendered = match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            let key = format!("arg.{k}");
            context.insert(key.clone(), rendered);
            provenance.insert(key, "llm".to_string());
        }
    }
    // Adapter-derived keys last: they overwrite any caller attempt to spoof
    // them, and they are the only platform-trusted entries.
    context.insert(CTX_TOOL.to_string(), args.tool.clone());
    provenance.insert(CTX_TOOL.to_string(), "platform".to_string());
    if let Some(label) = &cfg.server_label {
        context.insert(CTX_SERVER.to_string(), label.clone());
        provenance.insert(CTX_SERVER.to_string(), "platform".to_string());
    }

    Ok(AgentEvalRequest {
        policy_name: args.policy.clone().or_else(|| cfg.default_policy.clone()),
        principal,
        resource: args
            .resource
            .clone()
            .unwrap_or_else(|| format!("tool:{}", args.tool)),
        action: args.action.clone().unwrap_or_else(|| "call".to_string()),
        context,
        actor: args.actor.clone().or_else(|| cfg.default_actor.clone()),
        context_provenance: provenance,
        capability: args
            .capability
            .clone()
            .or_else(|| cfg.default_capability.clone()),
    })
}

/// Shape the agent's eval response into the tool's structured result.
pub fn authorize_result(resp: &AgentEvalResponse) -> Value {
    let allowed = resp.decision == "allow";
    json!({
        "allowed": allowed,
        "decision": resp.decision,
        "matched_rule": resp.matched_rule,
        "decision_id": resp.decision_id,
        "policy_id": resp.policy_id,
        "agent_id": resp.agent_id,
        "evaluation_time_microseconds": resp.evaluation_time_microseconds,
        "reason": format!(
            "{} — matched_rule '{}' (explain: explain_decision with decision_id '{}')",
            if allowed { "ALLOW" } else { "DENY" },
            resp.matched_rule,
            resp.decision_id,
        ),
    })
}

/// Validate a decision id for path use: agent decision ids are hyphenated
/// UUIDs; reject anything else rather than interpolating it into a URL.
pub fn valid_decision_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// MCP tool descriptors for `tools/list`.
pub fn tool_descriptors() -> Value {
    json!([
        {
            "name": "authorize_tool_call",
            "description": "Authorize a proposed tool call against Reaper policy before executing it. Returns allow/deny, the deciding rule, and a decision_id for explain_decision. Tool arguments and caller context are labeled llm-trust; only adapter-derived keys are platform-trusted.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "tool": { "type": "string", "description": "Name of the tool to be invoked" },
                    "args": { "type": "object", "description": "Proposed tool arguments (flattened to context keys arg.<name>, llm trust)" },
                    "principal": { "type": "string", "description": "Principal on whose behalf the call is made (default: REAPER_MCP_PRINCIPAL)" },
                    "actor": { "type": "string", "description": "Non-human actor identity (default: REAPER_MCP_ACTOR)" },
                    "action": { "type": "string", "description": "Action verb (default: call)" },
                    "resource": { "type": "string", "description": "Resource identifier (default: tool:<tool>)" },
                    "policy": { "type": "string", "description": "Policy name (default: REAPER_MCP_POLICY)" },
                    "capability": { "type": "object", "description": "Signed capability JSON (default: REAPER_MCP_CAPABILITY_FILE contents)" },
                    "context": { "type": "object", "additionalProperties": { "type": "string" }, "description": "Extra context key/values (always llm trust)" }
                },
                "required": ["tool"]
            }
        },
        {
            "name": "explain_decision",
            "description": "Fetch the full decision record for a decision_id returned by authorize_tool_call — matched rule, input_data explain snapshot, replay blob. Requires decision logging on the agent (REAPER_DECISION_LOG_ENABLED=true).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "decision_id": { "type": "string", "description": "Decision id from a previous authorize_tool_call" }
                },
                "required": ["decision_id"]
            }
        }
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use reaper_sdk::Transport;

    fn cfg() -> AdapterConfig {
        AdapterConfig {
            transport: Transport::http("http://127.0.0.1:1"),
            default_policy: Some("mcp-gate".to_string()),
            default_principal: Some("user_alice".to_string()),
            default_actor: Some("agent_claude".to_string()),
            default_capability: None,
            server_label: Some("files-server".to_string()),
        }
    }

    fn parse_args(v: Value) -> AuthorizeArgs {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn caller_supplied_values_are_llm_trust_and_args_flattened() {
        let args = parse_args(json!({
            "tool": "delete_file",
            "args": { "path": "/etc/passwd", "recursive": true },
            "context": { "approval_level": "admin" }
        }));
        let req = build_eval_request(&cfg(), &args).unwrap();

        assert_eq!(req.context.get("arg.path").unwrap(), "/etc/passwd");
        assert_eq!(req.context.get("arg.recursive").unwrap(), "true");
        assert_eq!(req.context.get("approval_level").unwrap(), "admin");
        // Everything the caller supplied floors to llm…
        assert_eq!(req.context_provenance.get("arg.path").unwrap(), "llm");
        assert_eq!(req.context_provenance.get("arg.recursive").unwrap(), "llm");
        assert_eq!(req.context_provenance.get("approval_level").unwrap(), "llm");
        // …and only adapter-derived keys are platform.
        assert_eq!(req.context.get(CTX_TOOL).unwrap(), "delete_file");
        assert_eq!(req.context_provenance.get(CTX_TOOL).unwrap(), "platform");
        assert_eq!(req.context.get(CTX_SERVER).unwrap(), "files-server");
        assert_eq!(req.context_provenance.get(CTX_SERVER).unwrap(), "platform");
        // Every context key is labeled — no unlabeled keys leave this edge.
        assert_eq!(req.context.len(), req.context_provenance.len());
    }

    #[test]
    fn caller_cannot_spoof_adapter_keys() {
        let args = parse_args(json!({
            "tool": "read_file",
            "context": { "mcp.tool": "innocuous_tool", "mcp.server": "trusted" }
        }));
        let req = build_eval_request(&cfg(), &args).unwrap();
        // Adapter values win and stay platform-labeled.
        assert_eq!(req.context.get(CTX_TOOL).unwrap(), "read_file");
        assert_eq!(req.context.get(CTX_SERVER).unwrap(), "files-server");
        assert_eq!(req.context_provenance.get(CTX_TOOL).unwrap(), "platform");
    }

    #[test]
    fn defaults_and_overrides() {
        let req = build_eval_request(&cfg(), &parse_args(json!({ "tool": "search" }))).unwrap();
        assert_eq!(req.principal, "user_alice");
        assert_eq!(req.actor.as_deref(), Some("agent_claude"));
        assert_eq!(req.policy_name.as_deref(), Some("mcp-gate"));
        assert_eq!(req.action, "call");
        assert_eq!(req.resource, "tool:search");

        let req = build_eval_request(
            &cfg(),
            &parse_args(json!({
                "tool": "search",
                "principal": "user_bob",
                "actor": "agent_other",
                "action": "invoke",
                "resource": "custom:res",
                "policy": "other-policy"
            })),
        )
        .unwrap();
        assert_eq!(req.principal, "user_bob");
        assert_eq!(req.actor.as_deref(), Some("agent_other"));
        assert_eq!(req.policy_name.as_deref(), Some("other-policy"));
        assert_eq!(req.action, "invoke");
        assert_eq!(req.resource, "custom:res");
    }

    #[test]
    fn missing_principal_is_an_error() {
        let mut c = cfg();
        c.default_principal = None;
        let err = build_eval_request(&c, &parse_args(json!({ "tool": "x" }))).unwrap_err();
        assert!(err.contains("principal"));
    }

    #[test]
    fn provenance_always_present_even_with_no_caller_context() {
        let mut c = cfg();
        c.server_label = None;
        let req = build_eval_request(&c, &parse_args(json!({ "tool": "t" }))).unwrap();
        // Taint mode is on: the provenance map exists and labels mcp.tool.
        assert_eq!(req.context_provenance.len(), 1);
        assert_eq!(req.context_provenance.get(CTX_TOOL).unwrap(), "platform");
    }

    #[test]
    fn decision_id_validation() {
        assert!(valid_decision_id("550e8400-e29b-41d4-a716-446655440000"));
        assert!(!valid_decision_id(""));
        assert!(!valid_decision_id("../../../etc/passwd"));
        assert!(!valid_decision_id("id with spaces"));
    }

    #[test]
    fn authorize_result_shape() {
        let resp = AgentEvalResponse {
            decision_id: "d-1".to_string(),
            decision: "deny".to_string(),
            policy_id: "p-1".to_string(),
            matched_rule: "capability_missing".to_string(),
            agent_id: "agent-1".to_string(),
            evaluation_time_microseconds: 0.5,
        };
        let v = authorize_result(&resp);
        assert_eq!(v["allowed"], false);
        assert_eq!(v["matched_rule"], "capability_missing");
        assert!(v["reason"].as_str().unwrap().starts_with("DENY"));
    }
}
