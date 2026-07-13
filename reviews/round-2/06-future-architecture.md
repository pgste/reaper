# Reaper — Architectural Fitness for the Future

*A board-level strategy assessment. Distinct from the four tactical reviewers
(performance, security, code/API, product). Lens: is this architecture fit to be
a long-lived platform, and does it sit where authorization engineering is
heading over the next 3–7 years? Grounded in the current tree (post-Plans 01–12,
PRs #10–#47).*

---

## Thesis up front

Reaper has quietly done the hard, unglamorous thing that most authorization
startups get wrong: it built an **I/O-free evaluation core with a clean trait
seam**, and it kept that core honest under pressure (delta≡rebuild determinism,
model_version provenance, a bounded ReBAC traversal). That is the load-bearing
architectural bet, and it is the right one. The risk is not the core — it is that
the **control plane is accreting enterprise surface faster than it is being
decomposed** (97 route sites, one deployable), and that Reaper is **racing a
commoditizing eval core** (OPA, Cedar, OpenFGA, SpiceDB all converge on the same
"policy + relationships + sub-ms eval" shape) without yet having planted its flag
on the axis that will actually differentiate the next five years: **authorization
for non-human, AI/agent actors**. The engine is wasm-ready in source but
wasm-shipped nowhere; the crypto is genuinely agile but not post-quantum; the
audit trail is a bespoke pipeline sitting one adapter away from being an
OpenTelemetry-native one. None of these are emergencies. All of them are
decisions being made *by default* right now that will be expensive to reverse.

---

## Fitness scorecard

| Dimension | Call | One-line rationale |
|---|---|---|
| Evolvability (evaluator seam, dual-DB, pluggable languages) | **Aligned** | `PolicyEvaluator` trait is clean and total; sqlx `Any` is a real seam; but two evaluators (`ReaperDSL`, Cedar-full) are half-wired, a smell of premature pluralism. |
| Modularity / coupling (engine stays I/O-free) | **Leading** | `policy-engine` has zero tokio/sqlx/reqwest in prod deps; I/O lives only in dev-deps. This is the discipline most competitors lack. |
| Architectural quantum (smallest deployable unit) | **Aligned → exposed** | Agent is a clean independent quantum; the **control plane is a single 63k-LOC monolith** — fine now, a scaling seam later. |
| Conceptual integrity | **Aligned** | The distribution story (sign → sync → hot-swap → confirm) hangs together; the API surface is sprawling but contract-gated. |
| Policy-as-code / authz-as-a-service positioning | **Aligned (core commoditizing)** | ReBAC + Zanzibar semantics present, but no consistency tokens/zookies — behind SpiceDB on the one thing Zanzibar is *famous* for. |
| AI / agent-era authorization | **Lagging (biggest opportunity)** | Right primitives (fast eval, provenance, ReBAC) but no short-lived/attenuated capabilities, no per-request LLM context model, no MCP-shaped integration, explainability is opt-in denies-only. |
| WebAssembly distribution | **Leading in source, not shipped** | Systematic `cfg(target_arch="wasm32")` across engine; **no cdylib, no wasm artifact, no CI target** — ambition coded, product absent. |
| Edge / distributed authz | **Aligned → leading** | Delta-sync + staleness-budget + Enforce/Monitor + fail-closed is a genuinely good edge posture; missing CRDT/offline-write and consistency tokens. |
| Platform-engineering / IDP fit | **Aligned** | GitOps + OpenAPI 3.1 + generated contract make it a usable IDP primitive; no Backstage plugin/Terraform provider/CRD yet. |
| Observability / OTel convergence | **Aligned** | trace_id on decisions, OTLP exporter in agent; audit is still a **bespoke NDJSON→Vector→ClickHouse** pipeline, not OTel-log-native. |
| Supply-chain / provenance / SLSA | **Aligned** | Signed bundles (v2 anti-replay), SBOM, cargo-deny/audit, Trivy; no in-toto attestation / reproducible policy builds / transparency log yet. |
| Post-quantum / crypto-agility | **Aligned** | `algorithm` field + Ed25519 **and** ECDSA-P256 already selectable — genuinely agile; no PQC/hybrid suite, but the seam to add one exists. |

---

## 1. Architectural fitness assessment

### The load-bearing abstraction is right: an I/O-free engine core

The single most important structural fact about this codebase is visible in
`crates/policy-engine/Cargo.toml`: the engine's production dependency set is
`serde`, `dashmap`, `pest`, `cedar-policy`, `regex`, `bumpalo`, `rayon`,
crypto primitives — and **no `tokio`, no `sqlx`, no `reqwest`, no `axum`**. Those
appear only under `[dev-dependencies]`. This is not an accident; it is the
discipline that determines whether a platform can evolve. Because the core is
I/O-free and deterministic, it can be: embedded in the agent, driven from the
CLI, replayed counterfactually in the control plane (`replay/`), fuzzed, and —
critically — **compiled to a different target** without dragging a runtime with
it. Most authorization products entangle evaluation with their HTTP/gRPC server
and can never cleanly get to edge/embedded/wasm. Reaper can. **Protect this
boundary with your life** — the first PR that adds a `tokio::spawn` or a DB call
inside `policy-engine` is the PR that ends the platform's optionality.

The `PolicyEvaluator` trait (`evaluators/mod.rs:29`) is the other load-bearing
seam, and it is well-shaped: total (`evaluate` + `validate` + `evaluator_type`),
`Send + Sync + Debug`, and — a nice touch — it evolved *additively* to support
set-level combination via `evaluate_matched` with a conservative default
(`:61-66`) so existing evaluators didn't break. That is textbook evolutionary
architecture: extend the interface with a defaulted method rather than a breaking
change. The trait will absorb a new policy language without a core rewrite.

### The abstraction that will crack first: pluggable languages you don't actually run

Here is the tension. `evaluators/mod.rs:18` carries a candid comment:
`datastore_to_cedar_entities and ReaperDSLEvaluator are not yet used ... kept as
internal implementations for future features`. Reaper advertises three policy
languages (Simple, Cedar, Reaper DSL). In practice the **DSL is the
perf-critical, compiled, fuzzed, optimized one** and the other two are
partly-inhabited rooms. Cedar drags a heavyweight external crate
(`cedar-policy 4.2`) into the core purely to keep a promise the product may not
want to keep. **Pluralism has a carrying cost**: every optimizer pass, every
compilation tier, every fuzz target either has to cover N languages or silently
covers one and lies about the rest. My call: the three-language story is a
**conceptual-integrity liability**, not an asset. Decide whether Reaper is a
*DSL company that can import Cedar* or a *multi-language runtime*. Right now it's
paying for the latter while shipping the former. If Cedar is a checkbox for
buyers, isolate it behind a feature flag / separate crate so it stops taxing the
core's compile time and dependency surface.

### The dual-DB seam is real but is a slow tax

`sqlx Any` (SQLite dev / Postgres prod, per repo-map and `db/connection.rs`) is a
legitimate evolvability seam — it lets the CLI and tests run without Postgres.
But `Any` is the lowest-common-denominator SQL dialect; it forecloses Postgres
superpowers (partial indexes, `LISTEN/NOTIFY` — which they've had to bolt on
separately via `events_pg.rs`, `jsonb`, advisory locks that they *do* use but
have to special-case). This is a classic Fowler "database as integration point"
smell in slow motion: the abstraction that buys dev convenience today will be the
one the team curses when they need a Postgres-native feature and discover half
the query layer is dialect-neutral. It won't crack in 18 months. It will ache for
five years. Keep it, but **budget for a Postgres-native repository layer** as the
control plane matures, with SQLite relegated to an explicit test double rather
than a co-equal production dialect.

### The architectural quantum: two quanta, unequal maturity

The **agent is a clean architectural quantum** — independently deployable,
independently versioned (it carries its own `model_version`/`data_version`
state), fail-closed by default, and it can run disconnected off its last synced
snapshot. This is exactly what you want the enforcement plane to be. The
**control plane is a single quantum of ~63k LOC** with 97 route sites spanning
orgs, auth, OIDC, SCIM, billing, ServiceNow, replay, migrations, datastore,
deployments, environments. It is a monolith. That is the *correct* choice today
(one team, coherent transactions, one deploy) and I would not decompose it now.
But conceptual load is rising fast, and the natural fracture lines are already
visible in the module tree: **identity/SSO/SCIM**, **policy-authoring/GitOps**,
**distribution/rollout**, **audit/replay/decisions**, and **billing** are five
bounded contexts wearing one binary. The decomposition trigger is organizational,
not technical: when a second team can't ship without stepping on a first team's
migrations, carve **billing and audit-query out first** (they're the least
coupled to the policy lifecycle). Do not carve early — a distributed monolith
would be strictly worse than what exists.

### Conceptual integrity: the distribution spine holds; the surface sprawls

The thing that genuinely hangs together is the distribution spine:
sign (`bundle_signing`, v2 envelopes with anti-replay) → broadcast (SSE +
pg_notify) → pull (delta or full, contiguity-enforced, self-healing) → atomic
hot-swap → agent-confirmed convergence. That is a *coherent idea*, executed
consistently, and it is the architectural achievement of the round-1→round-2
work. Against that, the API surface (97 routes, `api/mod.rs`) is broad enough
that conceptual integrity is now maintained by *tooling* (the `api_contract.rs`
parity gate, utoipa annotations) rather than by small size. That's an acceptable
trade — you buy back integrity with a contract gate — but it means the API is now
in the regime where **only the gate stands between the platform and drift**.
Guard that gate as a first-class asset.

---

## 2. Fit with future engineering trends

### Policy-as-code & the authorization-as-a-service wave — *Aligned, but the core is commoditizing*

The space is converging hard: OPA/Rego, AWS Cedar, OpenFGA and SpiceDB
(Zanzibar-style ReBAC) are all racing to the same shape — a policy language, a
relationship graph, sub-millisecond eval, decision logs. Reaper has *all four*
and a genuinely faster core (compiled DSL, arena alloc, SIMD parse, interning).
But "faster eval" is a commoditizing axis: everyone is at "fast enough," and the
buyer stops caring below a latency floor. The `relationships.rs` module is
explicitly Zanzibar-modelled ("`doc1 #owner @alice`", forward+reverse indexed,
bounded BFS) — this is good and correct.

**The exposure:** Reaper has Zanzibar *semantics* but not Zanzibar's *signature
feature* — **consistency tokens / zookies** (the repo-map lists them as
explicitly deferred, and a grep confirms zero occurrences). SpiceDB's entire
reason to exist is solving the "new enemy" problem — bounded staleness with a
snapshot token so you never authorize against a relationship you *just* revoked.
Reaper's answer is a *staleness budget* (time-bounded) rather than a *causal
token* (correctness-bounded). For many buyers that's fine. For the security-
sophisticated buyer who chose SpiceDB *specifically* for new-enemy protection,
Reaper currently cannot compete on the merits. My call: **you are racing a
commoditizing core, and the one place you're behind is the one place the
category leader is famous for.** Either build consistency tokens or make an
explicit, documented case for why the staleness-budget model is sufficient
(there is a real argument — most authz doesn't need linearizability — but it must
be *made*, not defaulted into).

### AI / LLM-era authorization — *Lagging, and this is the single biggest strategic opening*

This is the axis I would bet the company's differentiation on, and it is the one
Reaper has not yet leaned into. The thesis: agentic systems — MCP tool-callers,
autonomous agents, LLM copilots acting on a user's behalf — are exploding the
number of **non-human principals** and making **runtime authorization the last
line of defense against prompt injection**. When an LLM can be talked into
calling any tool, the only thing standing between it and your data is a fast,
fine-grained, *per-action* authorization check with an auditable "who was this
agent allowed to act as, and why did it say yes." That is *exactly* an
authorization engine's job, and Reaper's primitives are unusually well-suited:

- **Sub-microsecond eval** matters far more for agents than for humans — an
  agent loop makes thousands of tool calls where a human makes one click. The
  latency headline that is commoditized for human authz is *re-valued* for
  agentic authz.
- **ReBAC** is the natural model for "agent X may act on behalf of user Y for
  resource-set Z" — delegation is a relationship.
- **Decision provenance** (`decision_log.rs`: `trace_id`, `replay_input`,
  `policy_version`, `data_version`, `model_version`) is already richer than most
  competitors and is *precisely* what an AI-actions audit needs: "prove why the
  agent was allowed to do that."

**What's missing, concretely:**
1. **Short-lived / attenuated capabilities.** Agent authz wants macaroon/biscuit-
   style attenuation: "this token can do a *subset* of what the granting user can,
   for the next 5 minutes." Reaper's principal model is durable identity + durable
   relationships. There is no first-class notion of a *derived, attenuated,
   expiring* principal. The `bundle_signing` crypto-agility and the revocation
   machinery (`revocation.rs`) are building blocks, but the *capability* concept
   isn't modelled.
2. **Per-request LLM context as a typed input.** The DSL takes a `context`
   HashMap, which *can* carry LLM-supplied attributes — but there is no notion of
   **trust level of the context** (attributes asserted by a possibly-injected LLM
   must be treated differently from attributes the platform derived). An
   agent-era engine needs a taint/provenance model on request context.
3. **Explainability for AI actions.** The `input_data` "explain" tier exists but
   is opt-in and "typically denies-only" (`decision_log.rs:63-67`). For AI
   actions the *allows* are the dangerous ones — you want to explain why the
   agent was permitted, cheaply, at scale. Denies-only explainability is a
   human-debugging posture, not an AI-governance posture.
4. **MCP-shaped integration.** There is no MCP server / tool-authorization
   adapter. The most natural distribution channel for agent authz in 2026–2028 is
   "drop an MCP-aware authorization gate in front of your tool server." Reaper's
   agent is a clean quantum that could *be* that gate.

My call: **Reaper is one deliberate product bet away from being the obvious
authorization layer for agentic systems**, and it is currently spending its
energy on enterprise-console parity (SCIM, ServiceNow, billing) instead. The
enterprise work was necessary to be *buyable*; the agent work is what would make
it *chosen*. This is the highest-leverage direction in the whole assessment.

### WebAssembly everywhere — *Leading in the source tree, absent from the product*

This surprised me, in a good way. The engine is being **systematically kept
wasm-portable**: `Cargo.toml:42-44` splits JSON parsing (`sonic-rs` SIMD for
native, `serde_json` for wasm), and there are deliberate `cfg(target_arch =
"wasm32")` arms across `fast_parse.rs`, `data/loader.rs`, `decision_log.rs`, and
the DSL JSON builtins. Someone is thinking about this. That is the hard part —
keeping an I/O-free, `no-std`-adjacent core that *can* target wasm.

**But it is ambition, not artifact.** There is no `crate-type = ["cdylib"]` on
the engine, no `wasm-bindgen`/`wasm-pack`, no wasm build in CI, no published
component. The eBPF crate (`crate-type = ["cdylib"]`, kernel target) is the
*only* alternative-target thing that actually builds. So the wasm story today is:
"we could, and we've paid the source-level tax to keep the option open, but we
haven't shipped it." My call: **this is a cheap, high-value flag to plant.** OPA
and Cedar both went to wasm precisely to enable browser/edge/multi-language
embedding — a policy-eval `.wasm` component (WASI/Component-Model) would let
Reaper's DSL run in a browser, in a Cloudflare/Fastly worker, or embedded in a
Go/Python/Node app *without the agent*. Given the source is 80% ready, shipping a
wasm build target is a small investment with a large surface-expansion payoff,
and it doubles as the distribution mechanism for the *agent-era edge gate* above.
Do this before someone asks "does it run in the browser?" and you have to say
"almost."

### Edge & distributed authz — *Aligned, trending leading*

Reaper's data-plane model is genuinely well-suited to edge: the agent holds a
local snapshot, syncs deltas (`apply-deltas`, contiguity-enforced, self-healing
via `snapshot_required`), runs on a **staleness budget** with explicit
Enforce/Monitor modes and `deny_reason()` fail-closed behavior, and can serve
disconnected. That is a mature offline-first posture — better than most control-
plane-tethered competitors. The delta≡rebuild determinism (proptest-verified per
repo-map) is the property that makes eventually-consistent edge distribution
*safe*: the edge always converges to exactly the same store the control plane
would have built. **This is a strength worth protecting and marketing.**

The gap is the same one as the ReBAC section: **no CRDT / offline-write model and
no consistency tokens.** Reaper's edge is *read-replicated* (agents receive and
enforce; they don't originate relationship writes at the edge). True distributed
authz — where an edge node can *write* a relationship (grant/revoke) and
reconcile — needs CRDTs or a consistency-token protocol. For the enforcement use
case, read-replication is the right and sufficient model. If the product ever
moves toward "authorize *and mutate* at the edge," that's a genuinely hard
distributed-systems project the current architecture doesn't pretend to solve
(correctly — it stays honest about being read-path at the edge).

### Platform engineering & IDP integration — *Aligned*

The GitOps source model (`sync/`: git/api/s3/bundle-url + GitHub App + signed
commits + drift + commit-back) plus the generated **OpenAPI 3.1 contract** make
Reaper a *usable* internal-developer-platform primitive: a platform team can wire
"policies live in this repo, promote through these environments" as a golden
path. The environment/promotion/change-request machinery (Plan 10) is exactly the
shape an IDP wants. What's missing to be a *first-class* IDP citizen rather than a
well-behaved API: a **Backstage plugin**, a **Terraform provider**, and a
**Kubernetes CRD / operator** so policies are declarable as k8s resources
(`kind: Policy`) reconciled by an operator. These are integration surfaces, not
architecture changes — the OpenAPI contract makes them straightforward — but
their absence means Reaper today is a platform *you integrate* rather than a
platform *that's already in your golden path*. Medium-term, ship the Terraform
provider first (lowest effort, highest platform-team resonance).

### Observability & the OpenTelemetry convergence — *Aligned, one adapter from leading*

Good news: the agent already runs an **OTLP exporter** (`observability.rs`:
`opentelemetry_otlp`, semantic conventions), Prometheus metrics are well-modelled
(request-total vs engine-slice latency split — a subtle and correct distinction),
and `DecisionLogEntry` carries a `trace_id` for correlation. So decisions can be
*correlated* to traces today.

The strategic question is whether **audit is a first-class OTel citizen or a
bespoke pipeline**, and the answer is currently "bespoke": decisions ship as
**NDJSON → Vector → ClickHouse** (`deploy/decision-logs/`). That's a fine,
performant pipeline — but the industry is converging on OTLP *logs* and the
semantic-conventions-for-audit work. The risk: a buyer standardized on an OTel
collector + their SIEM will ask "why do I need a *second* pipeline (Vector +
ClickHouse) just for Reaper's decisions?" My call: keep the high-throughput
NDJSON path for volume, but **add an OTLP-logs emitter for decisions** as a
first-class option so audit rides the same collector as everything else. The
`trace_id` field means the correlation model is already right; this is an export
adapter, not a redesign. Doing it moves this from aligned to leading and removes a
"yet another pipeline" objection.

### Confidential computing / supply-chain provenance / SLSA — *Aligned*

Plan 06 delivered the table stakes: cargo-deny/audit blocking CI, Trivy image
scan, CycloneDX SBOM on release, parser fuzzing. Plan 02 delivered **signed
bundles with v2 anti-replay** (bundle_id + monotonic version + validity window
folded into the signed message — not advisory JSON; `bundle_signing.rs:85-100`).
That is a genuinely good provenance foundation: the *policy artifact* is
tamper-evident and rollback-resistant independently of transport.

Where this goes next, in order of value: (1) **in-toto / SLSA attestations** for
the *policy build* — right now the bundle is signed, but the *provenance of how
the bundle was produced from source* (which git commit, which builder, which
inputs) isn't attested; a compromised control plane could sign a bundle that
doesn't match the git source. The GitOps signed-commit verification is the input
side; an in-toto link from "verified commit" → "signed bundle" would close the
loop into a full SLSA chain. (2) **Reproducible policy builds** — the same
`.reap` source should compile to a bit-identical bundle, so a third party can
verify the bundle matches the source (`stable_policy_id` suggests determinism is
already valued; extend it to whole-bundle reproducibility). (3) **A transparency
log** (Rekor-style) for signed bundles, so bundle issuance is publicly auditable.
None are urgent; all are the natural continuation of a bet already placed.

### Post-quantum / crypto-agility — *Aligned (better than expected)*

The crypto layer is **already agile**, which most reviewers would assume it isn't:
`bundle_signing.rs` selects on an `algorithm` field and *already supports two
schemes* — Ed25519 and ECDSA-P256 (the latter explicitly for FIPS-validated-
module shops). The `SigAlgorithm` enum + `parse()` + envelope-versioning
(`ENVELOPE_V2`) means adding a third scheme is a bounded change, not a rewrite.
That is the correct architecture: **the algorithm is a value, not a hardcode.**

The gap is only that neither supported scheme is post-quantum. When NIST-selected
PQC signatures (ML-DSA / SLH-DSA) become a procurement checkbox — plausibly
within the 3–7 year window for a security-infrastructure vendor selling to
governments and banks — Reaper adds a `SigAlgorithm::MlDsa` variant and a
verifier arm, not a new subsystem. The *hybrid* (classical + PQC) suite is the
likely first ask; the envelope-versioning already anticipates carrying extra
signed metadata. My call: **the seam is right; do nothing now except keep it.**
Do not let anyone "simplify" the `algorithm` field away as YAGNI — it is the
cheapest insurance in the codebase.

---

## 3. The 3–7 year view

**The one abstraction most likely to become a bottleneck:** the **control-plane
monolith's shared Postgres via `sqlx Any`.** Not the engine — the engine will
still be fast in 2031. The control plane is where every enterprise feature lands
(billing, SCIM, ServiceNow, audit query, replay, migrations), and it all funnels
through one dialect-neutral DB layer and one deployable. As tenant count and
audit volume grow, the `Any` abstraction blocks the Postgres-native scaling moves
(partitioning, `jsonb` indexing, native pub/sub) that you'll need, and the
single-quantum control plane blocks per-context scaling. This is the abstraction
whose *virtue today* (dev-DB portability) becomes its *constraint tomorrow*.

**The one place complexity is accreting faster than it's being paid down:** the
**API/control-plane surface.** 97 routes, five bounded contexts in one binary,
integrity held together by a contract-parity gate. Every plan added surface; no
plan removed any. The multi-language evaluator pluralism (`ReaperDSL` and Cedar
half-wired) is the same pattern in the engine — capability breadth outrunning
capability depth. The debt isn't dangerous yet; it's that the *rate* is
unsustainable if the next 12 plans also only add.

**The single highest-leverage architectural investment for the next 18 months:**
**make Reaper the authorization layer for AI/agent actors — and ship the wasm
build target as its distribution vehicle.** Concretely: (a) model short-lived,
attenuated, expiring capabilities as a first-class principal-derivation; (b) add
a context-provenance/taint model so LLM-asserted attributes are distinguishable
from platform-derived ones; (c) make allow-path explainability cheap and default-
on for agent actors; (d) ship the engine as a wasm component so it can be the
in-process gate in an MCP tool server or an edge worker; (e) provide an MCP-shaped
adapter. This bet uses *every* existing strength (I/O-free core, sub-µs eval,
ReBAC, provenance, wasm-ready source) and lands Reaper on the one axis where the
category is *not* yet commoditized. It is strictly higher-leverage than a sixth
enterprise-console integration.

**The decision being made today that will be regretted:** *defaulting* into the
three-language runtime and the `Any` dual-DB as permanent commitments rather than
scoped choices. Neither is wrong today; both are being treated as settled when
they should be revisited. Name them as decisions with review dates, not as
architecture.

---

## 4. Strengths worth protecting (≤5)

- **The I/O-free engine core** (`policy-engine` with zero prod tokio/sqlx/reqwest;
  I/O only in dev-deps). This is the source of *all* future optionality —
  embedding, edge, wasm, replay. The first I/O dependency added to this crate is
  a strategic regression; treat it as an architectural firewall.
- **Decision provenance** (`decision_log.rs`: policy_version + data_version +
  model_version + trace_id + replay_input). Richer than the competition and
  *exactly* the substrate the AI-actions-audit future needs. Do not let volume
  pressure erode it to a thin allow/deny line.
- **Delta≡rebuild determinism** (proptest-verified) + the fail-closed staleness-
  budget agent. This is what makes eventually-consistent edge distribution *safe*,
  and it's a genuine differentiator against control-plane-tethered competitors.
- **Crypto-agility as a value, not a hardcode** (`SigAlgorithm` enum, dual scheme
  today, envelope-versioned). The cheapest future-proofing in the tree; the PQC
  and FIPS asks are already anticipated. Don't let it be "simplified" away.
- **The clean `PolicyEvaluator` trait seam** that has already demonstrated
  additive, non-breaking evolution (`evaluate_matched` defaulted). This is the
  hinge on which any new policy language turns without a core rewrite.

---

## If I advised the CTO

You have built the hard, correct thing — a fast, deterministic, I/O-free
authorization core with real provenance and a coherent distribution spine — and
then spent a heroic remediation cycle making it *buyable* by an enterprise
(auth, SSO, SCIM, audit integrity, HA/DR, supply chain). That work was necessary
and it's done well. But "buyable" is not "chosen," and the eval core you're
proudest of is the part the whole category is commoditizing. Your next 18 months
should not be a thirteenth enterprise integration; it should be a deliberate bet
on **authorization for non-human, agentic actors** — attenuated short-lived
capabilities, context taint, cheap allow-path explainability, an MCP adapter —
distributed via the **wasm build target your source tree is already 80% ready
for**. Every strength you have (sub-µs eval, ReBAC, provenance, I/O-free core)
is re-valued upward in that world, and it's the one axis where you'd be leading
rather than racing. Guard the engine's I/O-free boundary as non-negotiable, put
review dates on the two "decided by default" choices (three-language pluralism,
`Any` dual-DB), and treat the control-plane monolith's Postgres seam as the thing
you'll have to decompose *before* it hurts, not after.

---
*Strategy assessment — no code was modified. Evidence cited inline against the
current tree (Plans 01–12 shipped, PRs #10–#47). Companion to the four tactical
round-2 reviews.*
