# Data Protection PEP — Design Document

**Status:** PROPOSED (design approved for phasing; no implementation yet)
**Owner:** Reaper core
**Related:** `DATA_PLANE_PLAN.md` (ADM, delta sync, staleness), `POSTGRES_CLIENT_REVIEW.md`

---

## 1. Problem & Motivation

Reaper today is a PDP (Policy Decision Point): given `(principal, action,
resource)` it answers **allow/deny** in sub-microsecond time. That answers
"may Alice call `GET /patients`?" but not the question regulated
industries actually have to answer:

> *Which rows may Alice see, and which fields of those rows, given her
> role, purpose-of-use, and relationships — and can we prove it later?*

This design extends Reaper to **drive PEPs (Policy Enforcement Points)**:
the decision response carries machine-readable *advice* that client SDKs
compile into the data access itself — SQL predicates that filter rows at
the database, field-level masking/redaction on egress, and (later)
KMS-backed field encryption. The same policies clients already write
against their ADM (Authorization Data Model) become data-protection
policies, with the decision log proving exactly which policy and which
data version shaped every result set.

Compliance mandates this maps to directly: HIPAA minimum-necessary, GDPR
data minimization / purpose limitation, PCI DSS field protection, FINRA
supervision.

### Prior art (and what it teaches us)

| System | Mechanism | Lesson |
|---|---|---|
| Cerbos | `PlanResources` → condition AST → ORM/SQL adapters | Query-plan shape works; AST not strings |
| Oso | data filtering / authorized queries | SDK compiles the query, app doesn't append |
| OPA | partial evaluation / compile API | Residual policies translate to WHERE clauses |
| Zanzibar / SpiceDB / OpenFGA | `ListObjects` / `LookupResources` | ReBAC never compiles to a predicate — expand the accessible set |
| Immuta / Privacera / Ranger | classification-driven row filters + column masks in the data platform | Classification labels + purpose-of-use are the buyer's vocabulary; audit is the product |
| XACML | obligations & advice on decisions | The response vocabulary existed all along; modernize it |

Three independent competitors converged on "return a compilable plan,
not a per-row verdict." That convergent evolution fixes our core shape.

---

## 2. Design Decisions

### D1 — Advice is a compilable plan, never a post-hoc filter

**Decision:** Reaper never evaluates per fetched row. For set-returning
queries the client asks for a **query plan** (partial evaluation over a
resource *type* with the instance unknown) and receives a **residual
predicate AST** to push into the database query.

**Rationale:** per-row evaluation is O(n) policy calls per query and dies
at scale — every vendor that tried it abandoned it. Predicate pushdown
lets the database use its indexes; Reaper does one evaluation per query.

**Consequence:** plan generation is per-query, not per-row. It does not
need the sub-µs budget; 10–100µs is invisible next to the DB round trip.
The sub-µs engine still owns the plain allow/deny path and local graph
expansion (D4).

### D2 — Advice is an AST, never a SQL string

**Decision:** The wire format for predicates is a typed expression tree
(§4.2). SDKs compile AST → **parameterized** queries. Reaper never emits
SQL text; SDKs never string-concatenate policy values into queries.

**Rationale:** a PDP that emits SQL strings is an injection engine with
an audit log. AST + parameter binding makes the unsafe path impossible
rather than discouraged, and lets one advice format target Postgres,
MySQL, Mongo, and ORMs.

### D3 — Field advice is an ALLOWLIST

**Decision:** Column/field advice enumerates what the caller MAY see
(with per-field transforms). Fields not mentioned are not returned.
There is no "blocklist" mode.

**Rationale:** the PEP failure mode is failing OPEN — an SDK that forgets
a mask returns raw data with a 200. An allowlist fails CLOSED: forgetting
to handle a field means the field is absent, which is an observable bug,
not a breach. This mirrors the agent's default-deny evaluation semantics.

### D4 — ReBAC compiles to set expansion, not predicates

**Decision:** Relationship conditions ("resources where principal has
`viewer` via any group chain") are resolved by the agent expanding the
accessible-ID set against its **local, delta-synced ADM replica**, and
returned with a cardinality-aware strategy:

- `ids_inline` — `id IN (...)` for sets ≤ `inline_limit` (default 1 000)
- `semi_join` — the SDK stages IDs (temp table / `ANY(array)` /
  join against an app-side table) for larger sets
- `hybrid` — attribute predicates pushed down AND relationship set
  intersected

**Rationale:** Zanzibar's lesson — graph reachability has no WHERE-clause
form. Our structural advantage: the agent already holds the materialized
relationship graph in memory with sub-µs traversals, so expansion is a
local operation, not a network fan-out like cloud PDPs pay.

### D5 — The trust boundary moves to the PEP; design for it structurally

**Decision:** Three structural mitigations ship WITH phase 1, not after:

1. **SDKs build the query from the advice** (advice-first API); appending
   advice to an app-authored query is the awkward path, not the default.
2. **Conformance suite**: a versioned, language-agnostic corpus of
   `(policy, data, plan request) → (expected AST, expected result set on
   a reference dataset)` cases that every SDK PEP must pass. Reaper's
   differential-testing infrastructure generates and pins these.
3. **Advice provenance**: every advice response carries
   `policy_id/policy_version/data_version/data_checksum` plus an
   `advice_hash`; SDKs stamp the hash into query comments/log context so
   audits can join "query executed" to "advice issued".

**Rationale:** an unenforced decision is worse than a deny — it looks
protected. Verification must be observable end-to-end.

### D6 — Masking first; encryption is advice + KMS integration, never keys in the PDP

**Decision:** Phase 1–2 transforms are `mask`, `redact`, `hash`,
`tokenize_ref`, `null_out`, `truncate` — deterministic, keyless, cheap.
Field *encryption* is expressed as `encrypt(field, key_ref)` advice and
executed by a KMS-backed transform in the client/sidecar; Reaper never
holds, sees, or references raw key material — only opaque `key_ref`
names.

**Rationale:** field-level encryption is secretly a key-management
product (rotation, key-per-tenant, crypto-shredding, HSM). Coupling it
into the PDP poisons both. Masking delivers ~90% of the regulated-market
value at ~10% of the risk.

### D7 — Advice freshness rides the existing staleness machinery

**Decision:** Advice responses carry the same `data_version` /
`data_checksum` / `data_stale` stamps as decisions. In `enforce` mode a
stale agent refuses to issue plans (same fail-closed rule as
evaluation); the `REAPER_DATA_REQUIRE_SYNC` cold-start gate applies
unchanged.

**Rationale:** advice *reshapes data*; issuing it from a stale replica is
strictly worse than a stale allow. No new machinery — the read-replica
discipline built in D1/D2 of the data plane already covers it.

### D8 — Purpose-of-use is a first-class request field

**Decision:** The evaluation/plan request gains an optional
`context.purpose` (and free-form `context.*` attributes). Policies can
condition on it; the decision log records it.

**Rationale:** HIPAA/GDPR enforcement is purpose-based ("treatment" vs
"marketing"). Every incumbent buyer conversation starts here; it costs a
field now and a migration later.

---

## 3. Architecture

```
┌────────────────────────────── client pod ──────────────────────────────┐
│                                                                        │
│  App code ──── SDK (PEP) ────────────► Reaper Agent (PDP, sidecar)     │
│    │             │  1. plan(principal, action, resource_type, purpose) │
│    │             │  2. ◄─ advice {predicate AST | id sets, fields,     │
│    │             │        transforms, provenance, advice_hash}         │
│    │             │  3. compile AST → parameterized query               │
│    ▼             ▼                                                     │
│  Database ◄── filtered query (WHERE …)   [rows already minimized]      │
│    │                                                                   │
│    └──► SDK egress transforms (allowlisted fields, mask/redact/…)      │
└────────────────────────────────────────────────────────────────────────┘
         ▲                                        │
         │ policies + ADM (delta sync, verified)  │ decision/advice log
   Management plane ◄─────────────────────────────┘  (with advice_hash)
```

- **Instance decision** (existing `/api/v1/messages` + evaluate): now may
  carry `advice.fields` obligations (§4.3) alongside allow/deny.
- **Plan** (new `POST /api/v1/plan`): partial evaluation for a resource
  TYPE; returns row advice (predicate/sets) + field advice.
- **Chokepoint mode** (phase 4): compile the same plans into Postgres RLS
  policies / a SQL proxy for deployments that must not depend on per-app
  SDK discipline.

---

## 4. Data Model

### 4.1 ADM extension — classification labels

`AttributeDef` (in `ModelDefinition.entity_types[*].attributes`) gains
optional protection metadata. Backward compatible: absent = today's
behavior.

```jsonc
{
  "entity_types": {
    "patient_record": {
      "attributes": {
        "ssn":       { "type": "string", "labels": ["pii", "phi"],
                       "default_transform": "mask" },
        "diagnosis": { "type": "string", "labels": ["phi"] },
        "ward":      { "type": "string" }                  // unlabeled = plain
      }
    }
  },
  // org-defined label taxonomy; referenced by policies ("mask all phi")
  "labels": {
    "pii": { "description": "personally identifiable information" },
    "phi": { "description": "protected health information" }
  }
}
```

Rules:
- Labels are declared in the model (typo-safe: policies referencing an
  undeclared label fail validation, same as unknown attributes today).
- `default_transform` is what an allow WITHOUT explicit field advice
  applies to that field — protection by default, opt-up by policy.
- Labels ride the existing publish/delta/checksum pipeline unchanged
  (they are model content, already versioned).

### 4.2 Predicate AST (the wire format for row advice)

Deliberately small; every node compiles to every target or the plan is
rejected at generation time (no "SDK figures it out").

```jsonc
// PredicateNode =
{ "op": "and" | "or",  "args": [PredicateNode, ...] }
{ "op": "not",         "arg": PredicateNode }
{ "op": "cmp",         "attr": "ward", "cmp": "eq|ne|lt|le|gt|ge",
  "value": {"t": "string|int|bool", "v": ...} }
{ "op": "in",          "attr": "region", "values": [Value, ...] }
{ "op": "ids",         "strategy": "ids_inline",
  "ids": ["rec_1", "rec_9", ...] }                     // ReBAC expansion, small
{ "op": "ids_ref",     "strategy": "semi_join",
  "set_id": "adv_7f3a…", "count": 48211 }              // ReBAC expansion, large
{ "op": "true" } / { "op": "false" }                   // allow-all / deny-all
```

- `attr` names refer to **ADM attributes of the resource type** — the
  SDK owns the mapping `adm attribute → column/field` (declared once per
  entity type in SDK config, so schema drift is caught in one place).
- `ids_ref`: the agent stores the expanded set for `advice_ttl` (default
  30 s) and the SDK streams it (`GET /api/v1/plan/sets/{set_id}`) into a
  temp table / array bind. Never inlined above `inline_limit`.
- Values are typed; SDK compilers MUST bind them as parameters (D2).

### 4.3 Field advice (allowlist + transforms)

```jsonc
"fields": {
  "mode": "allowlist",                    // the only mode (D3)
  "allow": {
    "id":        { "transform": "none" },
    "name":      { "transform": "none" },
    "ssn":       { "transform": "mask", "args": {"keep_last": 4} },
    "diagnosis": { "transform": "redact" },
    "payer_ref": { "transform": "encrypt", "args": {"key_ref": "org-kms/phi-2026"} }
  }
}
// transforms: none | mask | redact | hash | tokenize_ref | null_out
//           | truncate | encrypt (phase 4, KMS-executed, key_ref only)
```

### 4.4 Plan API

```
POST /api/v1/plan
{
  "principal": "alice",
  "action": "read",
  "resource_type": "patient_record",
  "context": { "purpose": "treatment" },        // D8
  "policy_id": null,                            // null = all applicable
  "limits": { "inline_limit": 1000 }
}

200 →
{
  "plan_id": "…",
  "decision": "conditional",       // allow | deny | conditional
  "row": PredicateNode,            // absent when decision != conditional
  "fields": { … §4.3 … },
  "provenance": {
    "policy_id": "…", "policy_version": 3,
    "data_version": 41, "data_checksum": "sha256:…",
    "data_stale": false
  },
  "advice_hash": "sha256:…",       // over canonical (row, fields, provenance)
  "ttl_secs": 30
}
```

- `decision: "allow"` → no row restriction (`op:true`), fields still apply.
- `decision: "deny"` → SDK returns empty set WITHOUT querying.
- Staleness: in `enforce` mode a stale/never-synced agent returns 409
  with the same reason strings as `/ready` (D7).

### 4.5 Decision-response extension (instance path)

`EvalResponse` gains an optional `advice` object (fields-only — row
advice is meaningless for a single known instance). Absent today =
absent tomorrow for policies that carry no protection clauses; the hot
path pays zero when unused (one branch on a precompiled flag).

### 4.6 Audit / decision-log extension

`DecisionLogEntry` gains: `purpose`, `advice_hash` (nullable). The
existing `data_version`/`data_checksum` stamps complete the chain:

> result set ← query (carries advice_hash in comment/log) ← advice
> (hash, policy P v3) ← data version 41 (checksum) ← publish (author,
> time) — every link already durable except the two new fields.

---

## 5. Policy surface (sketch, phase 2 detail to follow)

`.reap` gains protection clauses; identical semantics in YAML/JSON forms:

```
policy patient_access {
  allow read on patient_record
    when principal.role == "clinician"
     and context.purpose == "treatment"
     and resource.ward in principal.wards        // → predicate pushdown
  protect {
    fields labeled phi  -> redact unless principal.role == "attending"
    field ssn           -> mask(keep_last: 4)
  }
}
```

Compilation: `resource.*` terms with the instance unknown become the
residual AST; `principal.*`/`context.*` terms are evaluated to constants
at plan time (they're known); relationship terms route to set expansion
(D4). A term that references data absent from the replica fails the plan
CLOSED, never silently drops a conjunct.

**Invariant (pinned by differential tests):** for every row `r` in the
reference dataset, `evaluate(principal, action, r) == allow` **iff** `r`
satisfies the compiled plan. Plan and instance paths may never diverge —
this is the correctness heart of the feature and gets the same
differential treatment as delta≡rebuild.

---

## 6. Performance strategy

| Path | Budget | Mechanism |
|---|---|---|
| Instance allow/deny (unchanged) | < 1 µs | existing engine; advice flag is one branch |
| Plan generation (attribute-only) | < 50 µs | residual walk over compiled policy AST |
| Plan with ReBAC expansion | < 1 ms @ 100k-edge graph | in-memory BFS on the local replica (already sub-µs per hop) |
| Set staging (`ids_ref`) | streamed | agent-side buffer, TTL-bound, no re-expansion per page |

Plans are cacheable per `(principal, action, resource_type, purpose,
data_version)` — the `data_version` key gives free invalidation on every
publish/delta batch (version changes → cache key changes).

---

## 7. Security invariants (the review checklist)

1. **Fail closed everywhere:** unknown label, uncompilable node, missing
   attribute mapping, stale replica (enforce), expansion overflow → deny
   / empty set / 409. Never a degraded plan.
2. **No SQL text crosses the wire** (D2). Conformance suite includes
   injection corpora (values containing `'; DROP`, unicode homoglyphs).
3. **Allowlist fields only** (D3).
4. **No key material in Reaper** (D6) — `key_ref` strings only.
5. **Advice is signed by provenance:** `advice_hash` covers the canonical
   serialization; tampering breaks the audit join visibly.
6. **PEP conformance is versioned and mandatory** (D5): an SDK that
   doesn't pass the suite for advice-version N must refuse advice-version
   N (version negotiation in the plan request).

---

## 8. Phasing

| Phase | Scope | Exit criterion |
|---|---|---|
| **P1** | Labels in ADM + field advice (allowlist, keyless transforms) on instance decisions; `purpose` context; decision-log fields | E2E: labeled model → policy → masked field on the wire; audit joins |
| **P2** | `/plan` endpoint, predicate AST, Postgres + one ORM reference compiler, conformance suite v1, differential plan≡instance harness | differential green over generated corpus; reference app filters at the DB |
| **P3** | ReBAC set expansion (`ids_inline`/`semi_join`/`hybrid`), plan cache keyed by data_version, set staging API | 100k-edge expansion < 1 ms; pagination stable under concurrent deltas |
| **P4** | Chokepoint enforcers (Postgres RLS generator, SQL proxy), KMS-executed `encrypt` transform | RLS output passes the same conformance corpus |

Non-goals (explicitly): Reaper executing queries itself; storing row
data in the PDP; per-row evaluation APIs (refused by design, D1);
key management (delegated, D6).

## 9. Open questions

1. Attribute→column mapping: SDK-side config (current design) vs
   server-registered mappings (better drift detection, more coupling).
   Leaning SDK-side + a `verify-mapping` CLI check in CI.
2. `ids_ref` set staging for horizontally scaled agents: sets are
   agent-local; a load-balanced SDK must pin plan+fetch to one agent
   (header/affinity) or sets must round-trip through the SDK. Pin first.
3. Consistency tokens (zookies): P3 plans + read-your-writes across a
   publish need `min_data_version` in the plan request. Deferred with
   the existing zookie backlog item — same mechanism.
4. Does `protect` belong in the same policy or a separate
   data-protection policy type? Same policy (shown above) keeps
   review/audit unified; revisit if authoring gets noisy.
