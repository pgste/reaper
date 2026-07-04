# Control-Plane UI — Feature & Decision Log

Running log of everything the backend already provides that the UI (built
separately) can rely on. Append-only; newest at the bottom of each section.

## Decision-log views

- `GET /api/v1/orgs/{org}/decisions` — filtered history (principal, action,
  resource, decision, policy_name, agent_id, from/to, limit≤1000/offset).
- `GET /api/v1/orgs/{org}/decisions/stats` — totals, allow/deny counts,
  active agents, avg eval time, top denied policies.
- `GET /api/v1/orgs/{org}/decisions/timeseries?interval=30s|5m|1h|1d` —
  chart-ready buckets `{bucket, total, allows, denies, avg_evaluation_time_ns}`.
- `GET /api/v1/orgs/{org}/decisions/facets` — dropdown values with counts for
  action/decision/policy_name/agent_id (top 50 each).
- `GET /api/v1/orgs/{org}/decisions/{decision_id}` — explain view; `input_data`
  may be an AES-GCM envelope (`{"enc":"aes256gcm",...}`) the tenant key opens
  via `policy_engine::decrypt_input_data` (server-side decrypt endpoint is a
  future decision — today decryption is client/CLI-side).
- Auth: `agent:read` or `org:admin`; 503 + guidance until
  `REAPER_CLICKHOUSE_URL` configured — UI can render a "connect your store"
  empty state straight from the error body.
- Capture modes surfaced in config: `full` / `sampled` / `denies`
  (`REAPER_DECISION_LOG_MODE`), plus data-protection flags (never the secrets;
  `hash_salt`/`encryption_key` are serde-skipped and cannot leak via config
  echoes).

## Policy examples library (template gallery)

- Location: `policy-library/**` — one directory per scenario with:
  - `manifest.json` — machine-readable: `name`, `source` (provenance),
    `policy`, optional `data`, `cases[]` (expected decisions, optional
    per-case `context`, expected `violations` for document cases). THIS is the
    UI's index format; walk directories for `manifest.json`.
  - `README.md` — walkthrough markdown, written to be rendered as the
    scenario's story page.
  - policy + data/input JSON files, directly loadable.
- CLI parity (same UX the UI should offer): `reaper-cli library list`,
  `library show <id>`, `library run [<id>]` (exit 1 on failure; per-case
  PASS/FAIL with reasons, including compiled-vs-AST parity failures).
- 13 scenarios / 76 cases as of this entry; CI-enforced by
  `crates/policy-engine/tests/policy_library_tests.rs`.

## Document checks (conftest workflow)

- Agent: `POST /api/v1/check {policy_name, input, principal?, action?,
  resource?, context?}` → `{allowed, violations[{rule, message}],
  check_time_us}`.
- CLI: `reaper-cli check -p policy.reap -i doc.json [--format json]`.
- UI idea (not built): paste-a-Terraform-plan panel that calls /check and
  renders violations.

## DSL capabilities the UI can advertise

- RBAC + ABAC + ReBAC composable in one rule; `rebac::related/reachable/
  inherited` (bounded, cycle-safe; ~18ns direct, ~110ns group-hop compiled).
- `input` documents (Terraform/K8s/any JSON), `with message` violations.
- JWT interpretation: `jwt::decode(token)` (claims), `jwt::header(token)`
  (alg/kid), Bearer-prefix tolerant, malformed → null (fail closed), plus
  `time::now_secs()` for exp/nbf checks. Signature verification is
  deliberately NOT in the policy layer (gateway/JWKS boundary owns it) —
  matches OPA `io.jwt.decode` semantics.

## Warm mode / service-mesh integration (THINKING ONLY — not implemented)

Question raised: can Reaper run "warm" for Istio etc.?

Assessment: yes, and the architecture is already shaped for it.
- The agent IS the warm process: policies compiled at deploy, data resident,
  decisions in ~250ns-2µs; no cold start per request.
- Istio/Envoy path: implement Envoy's `ext_authz` gRPC contract
  (`envoy.service.auth.v3.Authorization/Check`) as a new listener in
  reaper-agent alongside HTTP/UDS. Envoy's CheckRequest maps cleanly:
  principal ← peer SAN/JWT sub, action ← :method, resource ← :path,
  input ← full AttributeContext (headers etc.) via the existing `input`
  document support. Sidecar or node-local daemonset; UDS transport already
  exists for the lowest-latency same-pod deployment (the Envoy ext_authz
  filter supports uds:// targets).
- Effort estimate: tonic (gRPC) dep + proto vendoring + one adapter
  translating CheckRequest→PolicyRequest/input and Decision→CheckResponse
  (+ optional denied-body/headers). No engine changes required.
- Also viable: Envoy WASM plugin (compile policy-engine to wasm — sonic-rs is
  already cfg'd out for wasm), but ext_authz-gRPC is the standard, lower-risk
  first step.
- Decision: PARKED until requested; nothing in current work blocks it.

## Correctness/parity program (SHIPPED — see docs/development/CORRECTNESS.md)

Six-layer verification: unit tests, golden corpus (policy-library),
differential+oracle property suites for BOTH authorization (incl. ReBAC over
random graphs) and check mode (Terraform/K8s input documents), YAML suite
runners on committed fixtures, BDD. Null/undefined semantics are now a
written contract (missing data never satisfies anything but `== null` /
`!= null` — fail closed), enforced by an independent oracle in CI on every
push (PROPTEST_CASES=500). The program has caught 6 real bug classes,
including a fail-open `!=` present in BOTH evaluators. UI relevance: a
future "policy linter" panel can reuse the harness output, and the
CORRECTNESS.md table is renderable as a trust page.

## Data plane (D1 SHIPPED — docs/development/DATA_PLANE_PLAN.md)

Managed authorization data: per-namespace data stores with a typed
Authorization Data Model (entity types + attributes, roles + bindings,
relationship tuples) covering RBAC/ABAC/ReBAC and combinations. UI surfaces
to plan for: Roles manager, Attributes manager, Relationship manager (graph
view — shares the Policy Builder's ReBAC visualization), model/schema
editor, publish bar (draft→published diff), data-version badge on decision
views. All backed by CRUD APIs under /orgs/{o}/ns/{n}/… so customers can
build their own tooling on top; sync to reapers via snapshot bundles + SSE
deltas, Kafka ingestion in a later phase.

- D1 APIs live: POST/GET /orgs/{o}/namespaces/{n}/datastore (+/model,
  /entities, /entities/{id}, /entities/{id}/attributes, /role-bindings,
  /tuples, /publish, /versions, /versions/{v}). Typed validation errors are
  UI-ready strings ("attribute 'clearance' must be Int…").
- Staleness UX surfaces: agent /ready exposes data_version,
  data_staleness_secs, data_stale; decision entries carry data_version /
  data_checksum / data_stale — render a "data freshness" badge on agents
  and a stale marker on decision rows. Modes: monitor | flag | enforce
  (enforce = fail closed + not-ready).
