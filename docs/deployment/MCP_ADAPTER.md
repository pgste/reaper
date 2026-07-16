# MCP Adapter (`reaper-mcp`)

`reaper-mcp` is a stdio MCP server that puts a Reaper authorization gate in
front of tool-calling agents. An agent runtime lists it like any other MCP
server; before executing a tool call, the runtime (or the model itself,
per the server's instructions) calls `authorize_tool_call` and only
proceeds on `allowed: true`.

It is also the **reference enforcing edge for taint labeling**: the adapter
— not the calling model — assigns trust to every context key it forwards,
so policies can distinguish platform-derived facts from LLM-asserted ones.

```
┌──────────────┐  stdio MCP   ┌────────────┐  HTTP / UDS   ┌──────────────┐
│ agent runtime │ ───────────▶ │ reaper-mcp │ ─────────────▶ │ reaper-agent │
│  (MCP client) │ ◀─────────── │  (adapter) │ ◀───────────── │  (evaluator) │
└──────────────┘ allow/deny   └────────────┘  sub-µs eval   └──────────────┘
```

## Tools

### `authorize_tool_call`

| Argument | Required | Default | Notes |
|---|---|---|---|
| `tool` | yes | — | Tool the caller wants to invoke |
| `args` | no | — | Proposed tool arguments; flattened to context `arg.<key>`, `llm` trust |
| `principal` | no | `REAPER_MCP_PRINCIPAL` | Human principal (required overall) |
| `actor` | no | `REAPER_MCP_ACTOR` | Non-human actor identity |
| `action` | no | `call` | Action verb evaluated |
| `resource` | no | `tool:<tool>` | Resource evaluated |
| `policy` | no | `REAPER_MCP_POLICY` | Policy name |
| `capability` | no | `REAPER_MCP_CAPABILITY_FILE` | Signed capability JSON, forwarded verbatim |
| `context` | no | — | Extra key/values; always `llm` trust |

Returns `{allowed, decision, matched_rule, decision_id, policy_id,
agent_id, evaluation_time_microseconds, reason}`. A deny is a normal
result (`isError: false`); only transport/validation failures set
`isError: true`.

### `explain_decision`

`{decision_id}` → the full decision record from
`GET /api/v1/decisions/{id}`: matched rule, the `input_data` explain
snapshot (principal/resource/actor attributes the decision branched on —
captured by default for actor-carrying requests, F1-s4), and the replay
blob. Requires `REAPER_DECISION_LOG_ENABLED=true` on the agent.

## The taint contract

Requests through the adapter always carry a `context_provenance` map, so
taint mode is unconditionally on (unlabeled keys floor to `llm` in the
engine):

- **`llm`**: everything the caller supplied — tool `args` (as `arg.<key>`)
  and extra `context`. The adapter never honors trust claims from the
  caller's body.
- **`platform`**: only keys the adapter derives itself — `mcp.tool` (the
  tool being authorized) and `mcp.server` (from
  `REAPER_MCP_SERVER_LABEL`). These overwrite any caller attempt to spoof
  them.

Policies address these with chained attributes and the taint predicate:

```reap
rule delete_with_trusted_approval {
    allow if {
        actor.kind == "agent" &&
        context.mcp.tool == "delete_file" &&
        context.approved == "yes" &&
        taint::trusted("approved")   // llm-asserted approval can NEVER pass
    }
}
```

Platform-trusted values enter through channels the platform controls — a
direct agent call with `context_provenance` set by the operator's service,
or facts injected by capability verification — never through the adapter's
caller-facing arguments.

## Configuration

| Variable | Meaning | Default |
|---|---|---|
| `REAPER_MCP_AGENT_URL` | Agent HTTP endpoint | `http://127.0.0.1:8080` |
| `REAPER_MCP_AGENT_SOCKET` | Agent Unix socket (takes precedence) | unset |
| `REAPER_MCP_POLICY` | Default policy name | unset |
| `REAPER_MCP_PRINCIPAL` | Default principal | unset |
| `REAPER_MCP_ACTOR` | Default actor | unset |
| `REAPER_MCP_CAPABILITY_FILE` | Signed-capability JSON attached to every call by default | unset |
| `REAPER_MCP_SERVER_LABEL` | Value for the platform-trusted `mcp.server` key | unset |

The adapter starts fail-fast on a malformed capability file. Diagnostics
go to stderr (`RUST_LOG` respected); stdout carries only the protocol.

## Capabilities (F1-s3)

When a capability accompanies the request (per-call argument or the
default file), the agent verifies it **before** any policy evaluation:
signature (org bundle-signing key), validity window, revocation,
subject/actor binding, and grant coverage of `(action, resource)`. Any
failure is a deny with a stable `matched_rule` reason
(`capability_expired`, `capability_out_of_grant`, …).

Issue capabilities from the management service:
`POST /orgs/{org}/capabilities` (scope `capability:issue`) with
`{subject, actor, grants: [{action, resource}], ttl_secs}` — short-lived
by design (TTL ≤ 24h). An orchestrator can narrow a capability for a
sub-agent via the attenuation endpoint (grants must be a subset; issuer
re-signs).

## Protocol notes

- MCP over stdio, newline-delimited JSON-RPC 2.0; protocol revisions
  `2025-06-18`, `2025-03-26`, `2024-11-05`.
- `initialize`, `ping`, `tools/list`, `tools/call`; other requests get
  `-32601`, notifications are ignored.
- Zero new dependencies: the protocol loop is ~180 lines over
  `serde_json` + `tokio` (supply-chain gates stay quiet).

## End-to-end example

`tools/reaper-mcp/example/` contains a runnable walkthrough — policy,
entities, real transcripts — covering the allow path, the taint-blocked
deny, `explain_decision`, and the platform-approval counterfactual.
