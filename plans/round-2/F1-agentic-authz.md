# Workstream F1 — Authorization for Non-Human / Agentic AI Actors (scoping)

Strategic bet (`reviews/round-2/06-future-architecture.md` §"AI / LLM-era
authorization", backlog `plans/round-2/00-NEXT-BACKLOG.md` Workstream F). Not
remediation. **Status: F1-s1 + s2 LANDED (capability core, request shape, DSL actor + taint); s3–s5 pending.**

## STATUS (2026-07-16) — F1-s4 allow-path explainability (LANDED on branch)

- **Engine**: `PolicyEvaluator::evaluate_named` → `NamedOutcome { decision,
  matched, rule_name: Option<&str> }` — the deciding rule's name, BORROWED
  from the evaluator (zero eval-loop allocation; the engine clones once for
  the single decisive policy, same discipline as policy-name attribution).
  Compiled + AST evaluators both surface it and name the same rule for the
  same request (pinned). AST keeps its always-decisive `matched` semantics.
  `SetEvalOutcome`/`PolicyDecision` gain `matched_rule_name` (additive).
- **Agent**: `matched_rule` in responses and decision logs now carries the
  REAL rule name for allows and denies (index form kept for Simple;
  "default_deny" for no-match). `capture_input_data` includes the ACTOR's
  attributes. `should_capture_input(is_allow, has_actor)`: actor-carrying
  requests capture the explain snapshot DEFAULT-ON once decision logging is
  enabled (`input_data_actor_requests`, default true,
  REAPER_DECISION_LOG_INPUT_DATA_ACTOR_REQUESTS=false to disable); plain
  traffic keeps the opt-in denies-only posture. Replay blobs carry
  actor + context_provenance for faithful counterfactuals. All log-path
  only; the SLA histogram behavior is unchanged (sla_histogram tests green).
- Tests: rule_name_explain_tests (5, engine incl. compiled/AST name
  parity), allow_explain_tests (5, agent incl. default-on capture, opt-out,
  plain-traffic posture). Engine 960 + agent 247 green; workspace clippy
  -D; fmt; wasm32 engine + reaper-wasm; wasm parity (3).
- Remaining F1: s5 MCP adapter.

## STATUS (2026-07-16) — F1-s3 agent enforcement + issuance (LANDED on branch)

- **Slice 1 (core)**: `RevocationList.revoked_capability_ids` on the signed
  list-pull channel; canonical bytes append the segment only when non-empty
  (existing signed lists verify byte-for-byte — pinned test; a pre-F1 agent
  receiving a capability-bearing list fails signature and keeps last-good:
  fail closed both directions).
- **Slice 2 (agent)**: pre-eval gate on the served path.
  `BundleVerifier::verify_capability` = same trust anchor as bundles +
  revocation snapshot (staleness policy applies). `capability_gate::enforce`
  binds subject==principal / actor==actor (absent actor inherits the
  capability's), checks grant coverage; denials serve as decision:"deny"
  with reason in matched_rule. EvaluateRequest/BatchRequestItem gained
  actor/context_provenance/capability; the agent finally threads actor+taint
  into the engine PolicyRequest. Decision-cache fingerprint now folds actor
  + provenance (cross-actor cache poisoning fix, regression-tested). Fast
  path dispatches agentic requests to the standard lane (memmem probe,
  false positives only cost the fast lane); batch enforces per item.
  Opt-in `auth.require_actor_capability` / REAPER_REQUIRE_ACTOR_CAPABILITY.
  12-case integration suite.
- **Slice 3 (management)**: `capability:issue` / `capability:revoke` scopes
  (Owner role confers both); POST /orgs/{org}/capabilities (+ /attenuate,
  + /revoke). Issuance signs with the SAME bundle signing key agents pin —
  zero new key distribution. Attenuation verifies the parent's signature
  and revocation state first; widened grants/windows are rejected by the
  core; ttl ceiling 86400s. Revocation writes the org revocation list
  (kind="capability", no migration) — agents pick it up on the existing
  sync cadence. api_contract green; full lifecycle integration test
  (issue → verify → attenuate → widen-reject → forged-parent-reject →
  revoke root → child dies via ancestry → revoked-parent can't attenuate →
  scope enforcement).
- Remaining F1: s4 allow-path explainability, s5 MCP adapter.

## STATUS (2026-07-15) — F1-s2c compiled-path actor (slices A+B)

- **Slice A (EntityBindings)**: the compiled evaluator's 85
  `(user: &Entity, resource: &Entity)` parameter pairs became one Copy
  struct `EntityBindings<'a> { user, actor: Option<&Entity>, resource }` —
  zero behavior change, proven by the full suite + differential.
- **Slice B (compiled actor)**: `EntityType::Actor` (appended at enum end),
  `RebacRef::Actor`/`CompiledRebacRef::Actor`, all compiler rejection sites
  now compile actor, `evaluate_with_match` resolves `request.actor`
  (lookup-only — no interning; unloaded actor id binds a synthesized empty
  stack-local entity so attributes read missing while rebac still sees the
  id). Absent actor rides the missing-attribute path everywhere ⇒ identical
  fail-closed semantics to the AST's Null reads; explicit `== null` matches.
  actor_binding_tests now assert `evaluator_type() == "reaper_dsl"`.
- **Differential grew 42 → 61 cases** (actor matrix: every operator class ×
  present/absent/unloaded actor, `== null` trap, rebac incl. ghost ids). It
  caught and we fixed:
  - **compiled index-drop bug (pre-existing, user-affecting)**:
    `user.skills[0] == "rust"` compiled by DROPPING the index (compared the
    list to the literal ⇒ wrong Deny). Now compiles to `IndexedEquals`;
    other indexed shapes fall back to AST instead of miscompiling.
  - AST totality gaps: method receivers `actor.*` (pseudo-var path),
    method-on-Null now yields Null (was error), Null as a bare predicate is
    false (was error), unbound `actor` rebac arg is non-match (was error),
    unloaded actor entity reads Null (was error).
- **Slice C (compiled taint)**: `Condition::TaintTrusted { key }` /
  `CompiledCondition::TaintTrusted` (predicate) and `ExprType::TaintLevel` /
  `CompiledExprType::TaintLevel` (assignment form), all appended at enum
  ends. `EntityBindings` carries the request's provenance map by reference
  (`context_trust()` = the same fail-untrusted rule as PolicyRequest); eval
  is one HashMap get, no interner. Only literal keys compile — dynamic key
  expressions fall back to the AST. taint_predicate_tests now assert
  `evaluator_type() == "reaper_dsl"` on both the predicate and assignment
  forms. Differential 61 → 64 (trusted × 5 provenance shapes, level × 5,
  actor+taint composition).
- Gates: engine suite 953, workspace libs 1196, differential 64, clippy -D
  (workspace), fmt, wasm32 engine + reaper-wasm builds, wasm parity (3) —
  all green.

## STATUS (2026-07-15) — F1-s2 agentic request shape + DSL surface

- **Request shape** (part 1, merged in #70): `PolicyRequest` gained
  `actor: Option<String>` and `context_provenance: Option<{key -> TrustLevel}>`
  (Llm < Verified < Platform), both additive/default-off (pre-F1 payloads
  deserialize unchanged). `context_trust()` encodes the fail-untrusted rule
  (unlabeled key under taint mode = Llm floor).
- **DSL actor binding** (part 2): first-class `actor` entity beside `user`,
  resolved from `request.actor`; actor-less requests read `actor.*` as null
  (non-matching, not error). Works in attribute/indexed/chained access and as
  a rebac arg (`rebac::related(actor, "acts_for", user)` — the delegation
  model). Compiled evaluator falls back to AST for actor policies (parse
  + 5 compiler sites reject cleanly) — decision-safe by the equivalence
  contract; compiled-path actor is a perf follow-up.
- **DSL taint predicate** (part 2): `taint::trusted("key")` (bool, key is not
  LLM-tainted) and `taint::level("key")` ("platform"|"verified"|"llm"),
  reading the request provenance under the same fail-untrusted rule. An
  LLM-asserted attribute can never satisfy a platform-trust gate. AST-path
  (compiler falls back for `taint::`).
- Tests: actor_binding_tests (6), taint_predicate_tests (6), request_shape_tests
  (4, merged); full engine suite + compiled-vs-AST equivalence (42) +
  policy-library (82) green; clippy -D warnings; reaper-core + engine wasm32
  builds green.

## STATUS (2026-07-15) — F1-s1 capability core

`reaper_core::capability`: signed, expiring, attenuable capabilities on the
existing SigningKey/VerifyingKey machinery (Ed25519 + ECDSA-P256, zero new
dependencies). `Capability { id, key_id, subject, actor, grants[(action,
resource) patterns], not_before/expires_at, ancestry, signature }` over a
domain-separated, length-prefixed canonical message (no delimiter ambiguity
with attacker-chosen strings). `issue` / `attenuate` (issuer-side
re-issuance: grant-subset + window-nesting ENFORCED, subject lineage
inherited, ancestry recorded) / `verify_at(vk, key_id, now, revoked_ids)` —
pure, clock-explicit, wasm-safe (reaper-core builds for
wasm32-unknown-unknown with it) / `authorizes(action, resource)` with a
deliberately narrow pattern language (literal | `*` | trailing-`*` prefix).
Revoking any ancestor kills the whole derivation chain. 13 adversarial
tests: tamper-every-claim, wrong key, key-id pin, algorithm confusion,
expired/not-yet-valid, widened grants/windows (incl. wildcard-escape
attempts), leaf + ancestor revocation, malformed sig, empty grants, P-256
round-trip. Signed-list revocation transport + issuance endpoint are s3
scope; actor/taint request shape is s2.

## Goal (restated)

Make Reaper the authorization layer for agentic systems — MCP tool-callers,
autonomous agents, LLM copilots acting on a user's behalf — where runtime
authorization is the last line of defense against prompt injection. Four
deliverables from the strategist's case:

- **(a) Attenuated, short-lived capabilities** — "this token can do a *subset*
  of what the granting user can, for the next 5 minutes"; a derived, expiring
  principal, not a durable identity.
- **(b) Per-request LLM context taint** — attributes asserted by a
  possibly-injected LLM must be distinguishable from platform-derived ones.
- **(c) Cheap allow-path explainability** — for AI actions the *allows* are the
  dangerous ones; explaining them must be default-on and cheap, not the current
  opt-in denies-only posture.
- **(d) An MCP adapter** — "drop an MCP-aware authorization gate in front of
  your tool server" is the natural 2026–2028 distribution channel.

Every existing strength re-values upward here: sub-µs eval (agents make
thousands of tool calls), ReBAC (delegation is a relationship), decision
provenance (prove why the agent was allowed).

## Inventory — reuse vs. build

### What exists to reuse

- **Signed validity-window envelope** — `reaper_core::bundle_signing` v2
  (`ENVELOPE_V2`, `bundle_signing.rs:89-119`): `SigAlgorithm{Ed25519, EcdsaP256}`,
  monotonic anti-rollback version, `not_before`/`expires_at` **inside the signed
  message**. This is one abstraction away from a short-lived capability token.
- **Revocation pattern** — `revocation.rs`: signed, monotonic-serial revocation
  list, list-pull on sync cadence (not per-request online check). A capability
  revocation list reuses this shape wholesale.
- **ReBAC as the delegation substrate** — `data/relationships.rs`: Zanzibar
  tuples, forward+reverse indexed, bounded BFS (4096-node budget), three check
  kinds (`Direct`/`Reachable`/`Inherited`), DSL `rebac::related(...)` /
  `rebac::reachable(...)`. "Agent X acts-for user Y" is modelable as tuples
  today — what's missing is the *request-side* two-principal shape (§build 2).
- **Time conditions** — `CompiledCondition::TimeOp` + `time::now_*` builtins
  already express "deny if `expires_at < now`"; a capability expiry check on
  the eval path reuses this. There is even a JWT builtin
  (`ast_evaluator/builtin_functions/jwt.rs`).
- **Decision provenance** — `DecisionLogEntry`: `trace_id`, `input_data` explain
  tier (entity attributes captured **on the log path, never the eval loop** —
  `capture_input_data`, `evaluate.rs:52-65`), `replay_input`, `data_version` /
  `model_version` / `data_checksum`, tamper-evident hash chain. The AI-audit
  substrate already exists; only its *defaults and shape* are wrong for allows.
- **Scopes discipline** — management `Scope` enum
  (`auth/scopes.rs`: `bundle:promote` ≠ `bundle:approve`, `audit:export`,
  `audit:erase`...). A `capability:issue` scope slots in naturally, as does an
  org-scoped issuance endpoint under the existing JWT/middleware gateway.
- **SDK transport** — `reaper-sdk` HTTP/UDS client + agent
  `/api/v1/messages|fast-messages|batch-messages`: the MCP adapter's backend
  call is already built.

### What must be built (the real gaps)

1. **A capability token** — format, issuance, *attenuation semantics*
   (subset-narrowing of actions/resources), and **agent-side verification on
   the eval path**. Today the agent's JWT check parses **only `exp`**
   (`agent/auth.rs:198-202`) — claims are never enforced — and the data plane
   can be entirely open (`open_data_plane`). The `principal` in the request
   body is **self-asserted text**.
2. **A two-principal request shape.** Engine `PolicyRequest`
   (`engine/types.rs:210`) has *no principal field at all* — the agent injects
   `context["principal"]` (`evaluate.rs:346`); the DSL resolves it to a loaded
   `user` entity (`reaper_dsl/mod.rs:1554-1581`). There is no `actor` vs
   `subject` ("on behalf of") distinction anywhere in the request model or DSL
   grammar (`reap.pest:231`: entities are `user|resource|context|input`).
3. **Context taint.** `context` is untyped `HashMap<String,String>`; nothing
   marks a key as platform-derived vs LLM-asserted. Greenfield.
4. **Stable rule identity + allow-side reason.** `matched_rule` is a positional
   `Option<usize>` stringified as `"rule_{idx}"` (`evaluate.rs:430-434`) even
   though DSL rules have names in source. Explain tier defaults:
   `include_input_data=false`, `input_data_denies_only=true`.
5. **MCP adapter.** Zero MCP code in the repo (grep: docs mentions only).

## Design sketch (proposed, pending decisions below)

**Capability = a signed, expiring, narrowing grant.**
`Capability { v, algorithm, key_id, subject (human principal), actor (agent
id), grants: [(action-pattern, resource-pattern)], not_before, expires_at,
issuance_chain }` — signed with the existing `SigAlgorithm` machinery in
`reaper-core` (pure crypto, I/O-free, wasm-compatible). Attenuation =
re-issuance with strictly-subset grants and ≤ expiry; every attenuation
references its parent's hash (auditable chain). Verification is a pure
function in `reaper-core`; the **agent** verifies pre-eval and injects the
result as *trusted* context + resolved `actor`/`subject`; the **engine stays
I/O-free and crypto-optional** — policies see verified facts, they don't do
crypto. Revocation: capability-id list reusing the `revocation.rs`
signed-serial list-pull.

**Taint = per-key provenance, fail-untrusted.**
Request gains optional `context_provenance: {key → trust}` with
`trust ∈ {platform, verified, llm}`; anything unlabeled defaults to the
*lowest* trust when taint mode is on. The trusted labels are assigned by the
enforcing edge (agent handler / MCP gate) — never by the caller's body alone.
DSL gains a predicate (e.g. `taint::trusted(context.approval_level)` or
`context.trusted("key")`) so policies can require provenance for sensitive
comparisons. Fail-closed: an LLM-asserted attribute can never satisfy a
condition that demands platform trust.

**Explainability = stable rule ids + default-on for agentic requests.**
Surface the DSL rule *name* (they exist in source) through
`PolicyDecision.matched_rule`; add a compact allow-side reason (rule name +
the capability grant that admitted it) captured on the log path like
`input_data` is today (cheap, off the eval loop). Flip the default to
capture-allows **for requests carrying an actor/capability** — humans keep
today's posture, agents get audited allows.

**MCP adapter = the distribution wedge.**
A `reaper-mcp` binary (stdio MCP server) exposing tools like
`authorize_tool_call(tool, args, capability) → allow/deny + reason` and
`explain_decision(decision_id)`, backed by `reaper-sdk` → agent. It is also
the reference implementation of "the enforcing edge that labels taint."

## Proposed PR-sized slices (independently mergeable)

- **F1-s1 — Capability core (reaper-core).** `capability.rs`: token struct,
  sign/verify/attenuate (subset-check enforced), expiry/not-before, revocation
  list reuse. Pure crypto + unit tests incl. adversarial cases (expired,
  widened grants, wrong key, revoked, tampered chain). No service changes yet.
- **F1-s2 — Actor/subject + taint in the request path.** Engine
  `PolicyRequest` gains optional `actor` + `context_provenance` (additive,
  wire-compatible: absent = today's behavior). DSL: `agent`/actor entity
  binding + taint predicate. Evaluator + engine tests; BDD feature file for
  delegation & taint scenarios.
- **F1-s3 — Agent enforcement.** Agent verifies capability pre-eval
  (management issues; `capability:issue` scope + issuance endpoint), injects
  verified facts as trusted context, rejects expired/revoked/out-of-grant
  before eval. api_contract gate applies (typed DTOs + ProblemDetails).
- **F1-s4 — Allow-path explainability.** Stable rule names in
  `matched_rule`, allow-side reason capture, default-on for actor-carrying
  requests. Log-path only; SLA histogram must show no eval-loop regression.
- **F1-s5 — MCP adapter.** `tools/reaper-mcp` stdio server + docs + an
  end-to-end example (tool server + policy + capability → allow/deny with
  reasons).

Slices s1/s2 are engine/core-only and parallelizable with F2's wasm work; s3
depends on s1; s5 depends on s3.

## Open decisions (need confirmation — product-shaped)

1. **Capability model:** (i) **homegrown signed envelope reusing
   `bundle_signing` machinery** — no new deps, online (issuer-side)
   attenuation only — vs (ii) **Biscuit** (`biscuit-auth` crate) — offline
   holder-side attenuation (an orchestrator can narrow before handing to a
   sub-agent, no issuer round-trip) but a new supply-chain dependency — vs
   (iii) macaroons (HMAC chaining; shared-secret verification, weakest fit).
   The real product question: *do agent holders need to attenuate offline?*
2. **Taint representation:** (i) **per-key provenance map + DSL predicate**
   (explicit, fail-untrusted) vs (ii) namespace convention (`llm.*` prefixes —
   zero schema change, but spoofable and implicit) vs (iii) typed context
   values (breaking wire change — probably a bridge too far now).
3. **Request shape:** add first-class optional `actor` beside the existing
   principal (subject) — vs keeping a single principal and letting the
   capability carry the whole delegation chain. First-class `actor` makes
   policies readable (`actor.type == "agent" and rebac::related(actor,
   "acts_for", user)`); capability-only keeps the wire untouched.
4. **MCP surface:** standalone `tools/reaper-mcp` stdio binary (recommended) vs
   an agent-embedded HTTP MCP endpoint vs a `reaper-sdk` library feature. And
   minimal tool set for v1: `authorize_tool_call` + `explain_decision`?

## Relationship to F2

F2 (wasm target) is F1's distribution vehicle: the capability-verify + eval
core compiled to wasm is what makes "in-process gate inside an MCP tool
server / edge worker" real. F1-s1/s2 land in `reaper-core`/`policy-engine`
and must stay wasm-compatible (pure crypto, no I/O) so they ride F2 for free.
Neither strictly blocks the other; the shared constraint is: **nothing F1 adds
to the engine may violate the I/O-free boundary.**
