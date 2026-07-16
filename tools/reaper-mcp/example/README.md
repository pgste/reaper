# End-to-end example: gating MCP tool calls with Reaper

This walkthrough runs the full F1 agentic-authorization stack on real
binaries: a Reaper Agent enforcing the `mcp_tool_gate` policy, and the
`reaper-mcp` adapter authorizing tool calls over stdio MCP. Every output
below is from an actual run.

The policy (`policy.reap`) has three tiers:

1. **Read-only tools** (`read_file`, `search`): any bound agent actor may
   call them.
2. **`delete_file`**: requires `context.approved == "yes"` **and**
   `taint::trusted("approved")` — the approval must be platform- or
   verified-trust. The adapter labels everything the caller supplies as
   `llm`, so a model that *claims* approval can never satisfy this rule.
3. Everything else: default deny.

## 1. Build and start the agent

```bash
cargo build -p reaper-agent -p reaper-mcp -p reaper-cli

REAPER_DECISION_LOG_ENABLED=true \
REAPER_DECISION_LOG_PRIVACY=raw \
REAPER_AGENT_BIND=127.0.0.1 REAPER_AGENT_PORT=18080 \
./target/debug/reaper-agent
```

(`REAPER_DECISION_LOG_PRIVACY=raw` is the explicit dev-mode opt-out; pick
`pseudonymize` in production. Decision logging is what powers
`explain_decision`.)

## 2. Deploy the policy and entities

```bash
./target/debug/reaper-cli compile tools/reaper-mcp/example/policy.reap \
    --output /tmp/gate.rbb
./target/debug/reaper-cli --agent-url http://127.0.0.1:18080 \
    bundle deploy /tmp/gate.rbb --data tools/reaper-mcp/example/data.json
```

`data.json` loads two entities: `user_alice` (the human principal) and
`agent_claude` (an `Agent` entity with `kind: "agent"`, bound as the actor).

## 3. Run the adapter

The adapter is a stdio MCP server; an MCP client (e.g. an agent runtime)
would launch it like this:

```json
{
  "mcpServers": {
    "reaper-gate": {
      "command": "reaper-mcp",
      "env": {
        "REAPER_MCP_AGENT_URL": "http://127.0.0.1:18080",
        "REAPER_MCP_POLICY": "mcp_tool_gate",
        "REAPER_MCP_PRINCIPAL": "user_alice",
        "REAPER_MCP_ACTOR": "agent_claude"
      }
    }
  }
}
```

To drive it by hand, pipe newline-delimited JSON-RPC:

```bash
REAPER_MCP_AGENT_URL=http://127.0.0.1:18080 REAPER_MCP_POLICY=mcp_tool_gate \
REAPER_MCP_PRINCIPAL=user_alice REAPER_MCP_ACTOR=agent_claude \
./target/debug/reaper-mcp << 'EOF'
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}
{"jsonrpc":"2.0","method":"notifications/initialized"}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"authorize_tool_call","arguments":{"tool":"read_file","args":{"path":"/srv/docs/spec.md"}}}}
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"authorize_tool_call","arguments":{"tool":"delete_file","args":{"path":"/srv/docs/spec.md"},"context":{"approved":"yes"}}}}
EOF
```

Results (structuredContent, abridged):

```json
{"allowed": true,  "matched_rule": "agent_readonly_read_file", "decision_id": "bd433348-…"}
{"allowed": false, "matched_rule": "default_deny",             "decision_id": "324ae0ed-…"}
```

The second call is the point of the whole exercise: the model passed
`context: {"approved": "yes"}` — the right value! — but the adapter labeled
it `llm` trust, `taint::trusted("approved")` failed, and the call fell
through to default deny. **A prompt-injected approval cannot authorize a
mutating tool.**

## 4. Explain the allow

```json
{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"explain_decision","arguments":{"decision_id":"bd433348-…"}}}
```

returns the full decision record. Because the request carried an actor,
the explain snapshot was captured **by default** (F1-s4):

```json
{
  "decision": "allow",
  "matched_rule": "agent_readonly_read_file",
  "principal": "user_alice",
  "input_data": {
    "principal": { "role": "engineer", "department": "platform" },
    "actor":     { "kind": "agent", "model": "claude", "trusted": true }
  }
}
```

## 5. The counterfactual: a real approval

A platform-side approval flow (an operator UI, a ticketing hook — any
channel that is *not* the model's mouth) calls the agent directly and
labels its context itself:

```bash
curl -s -X POST http://127.0.0.1:18080/api/v1/messages \
  -H "Content-Type: application/json" -d '{
    "policy_name": "mcp_tool_gate",
    "principal": "user_alice", "actor": "agent_claude",
    "action": "call", "resource": "tool:delete_file",
    "context": {"mcp.tool": "delete_file", "approved": "yes"},
    "context_provenance": {"mcp.tool": "platform", "approved": "platform"}
  }'
```

```json
{"decision": "allow", "matched_rule": "agent_delete_with_trusted_approval"}
```

Same value, different provenance, opposite decision — trust comes from the
channel, never the payload.

## Adding a capability requirement

To also require a signed capability (F1-s3), issue one from the management
service (`POST /orgs/{org}/capabilities` with the `capability:issue` scope)
and hand it to the adapter via `REAPER_MCP_CAPABILITY_FILE=/path/to/cap.json`
(or per-call as the `capability` argument). The agent then verifies
signature, expiry, revocation, subject/actor binding, and grant coverage of
`(action, resource)` **before** evaluation — see
`docs/deployment/MCP_ADAPTER.md`.
