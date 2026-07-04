# The Reaper Data Plane — Managed Authorization Data

**Status: D1 SHIPPED** (see "Phase D1 — shipped" below); D2+ as planned.
Companion to `DSL_V2_DESIGN.md` and `CORRECTNESS.md`.

## 1. The opportunity

Every authorization engine has the same unsolved half: **where does the data
live, and who maintains it?**

| Tool | Policy story | Data story |
|------|-------------|------------|
| OPA / Rego | mature | DIY. Bundles you build yourself; OPAL is a third-party bolt-on that syncs data you still have to store somewhere else |
| OpenFGA / SpiceDB (Zanzibar) | relationship tuples only | good tuple store — but **no attributes, no roles-as-data**; ABAC needs a second system |
| Cerbos | attribute conditions | **stateless by design** — every request must carry the data; your services become the data plane |
| AWS Verified Permissions | Cedar | entities passed per-request or shallow slices; no fleet sync |

Nobody offers: *one managed data model covering RBAC + ABAC + ReBAC together,
with first-class managers (roles, attributes, relationships), an API you can
build on, and push-based sync to sub-microsecond enforcement points.* That is
the loop we close. Users maintain THEIR authorization data in OUR data plane;
their reapers consume it automatically.

We are unusually well positioned — this is mostly assembly, not invention:

- The engine's `DataLoader` format is already the perfect materialization
  target: `entities[{id, type, attributes, relationships}]` — attributes give
  ABAC, `relationships: {relation: [subjects]}` IS the Zanzibar tuple shape
  our `RelationshipGraph` ingests, and roles are just entities + attributes +
  edges.
- The management server already has orgs/namespaces/teams, Postgres
  migrations, an SSE event stream (`/orgs/{org}/events`), API keys, and a
  `data_sources` table whose `source_type` column already anticipates
  `'http', 'kafka', 's3'`.
- `reaper-sync` (agent-side sync client) and the bundle deployment pipeline
  already move versioned artifacts to the fleet with zero-downtime swap.

## 2. The Authorization Data Model (ADM)

One model, three lenses. Everything below lives per **namespace** (prod,
staging…) inside an **org**, and everything is versioned.

### 2.1 Schema layer (the "model definition")

A namespace declares its vocabulary — this is what the UI managers render and
what write-time validation enforces:

```yaml
model:
  entity_types:
    - name: user
      attributes:            # ABAC vocabulary — TYPED, enforcing the
        - {name: mfa,        type: bool}      # engine's type-strict contract
        - {name: department, type: string}
        - {name: clearance,  type: int}
    - name: document
      attributes:
        - {name: classification, type: string, values: [public, internal, secret]}
        - {name: owner_id,       type: string}
  roles:                     # RBAC vocabulary
    - name: admin
      permissions: ["*:*"]
    - name: editor
      permissions: ["document:read", "document:write"]
  relations:                 # ReBAC vocabulary (Zanzibar-style)
    - {name: owner,     object: document, subject: [user]}
    - {name: viewer,    object: document, subject: [user, group]}
    - {name: member_of, object: group,    subject: [user, group]}  # traversable
```

Combinations need nothing special: a policy rule can reference a role
binding, an attribute, AND a relationship in one condition — the model just
guarantees all three vocabularies exist and are typed.

### 2.2 Record layer (the data itself)

Four record kinds, each a small CRUD surface:

1. **Entities** — subjects/resources with typed attribute values, validated
   against the schema (a string `"5"` cannot land in an `int` attribute:
   the type-strict comparison contract is enforced at WRITE time, where the
   mistake is cheap, instead of silently failing at evaluation).
2. **Role bindings** — `subject (user|group) → role [→ scope]`. Stored
   relationally so "who has admin?" is one query.
3. **Relationship tuples** — `(object, relation, subject)`, exactly what
   `RelationshipGraph::add_edge` consumes.
4. **Groups** — entities with `member_of` sugar in the managers.

### 2.3 Materialization

The ADM compiles to the existing engine format with **zero engine changes**:

- entity + attributes → `attributes{}` (roles become
  `attributes.roles: [..]` and/or expanded permissions, per model config)
- role bindings → either attribute expansion (RBAC-as-ABAC, fastest) or
  `relationships.has_role: [...]` edges (auditable via ReBAC), model's choice
- tuples → `relationships{}` verbatim

Materialized output = a **data bundle**: versioned, checksummed, monotonic
sequence number, distributed exactly like policy bundles today.

## 3. Provisioning & APIs

### 3.1 Provisioning
`POST /orgs/{org}/namespaces/{ns}/datastore` with a **template**:
`rbac`, `abac`, `rebac`, or `combined` (each seeds a model definition the UI
managers open ready-to-edit; `combined` is the flagship). Backed by Postgres
tables in the existing management DB (no new infra for v1); the API shape
leaves room for dedicated/self-hosted stores later.

### 3.2 The managers (API first, UI second)

```
# Model
GET/PUT  /orgs/{o}/ns/{n}/model                      # schema, versioned

# Entities & attributes (ABAC manager)
CRUD     /orgs/{o}/ns/{n}/entities[/{id}]
PATCH    /orgs/{o}/ns/{n}/entities/{id}/attributes   # typed, validated

# Roles & bindings (Roles manager)
CRUD     /orgs/{o}/ns/{n}/roles[/{role}]
CRUD     /orgs/{o}/ns/{n}/role-bindings              # subject, role, scope?
GET      /orgs/{o}/ns/{n}/roles/{role}/subjects      # "who has admin?"

# Relationships (ReBAC manager)
POST/DELETE /orgs/{o}/ns/{n}/tuples                  # {object, relation, subject}
GET      /orgs/{o}/ns/{n}/tuples?object=…&relation=…&subject=…
GET      /orgs/{o}/ns/{n}/graph/{entity}             # neighborhood, for the UI

# Bulk & lifecycle
POST     /orgs/{o}/ns/{n}/import                     # JSON/CSV, dry-run mode
POST     /orgs/{o}/ns/{n}/datastore/publish          # cut a data-bundle version
GET      /orgs/{o}/ns/{n}/datastore/versions[/{v}/diff]
```

All under existing org auth + API keys → **"API driven"** falls out for
free: a customer's HR system can call `PATCH /entities/{id}/attributes` on
termination, or they can wrap our API in their own service. Every mutation
lands in the existing audit tables (who changed which role, when — a
compliance feature none of the tuple stores surface well).

### 3.3 UI managers (feeds the design work)
- **Roles manager** — roles, permissions, bindings; "who has access" views.
- **Attributes manager** — schema editor + entity attribute grid.
- **Relationship manager** — the graph view: draw `user → member_of → team`,
  see what a tuple unlocks (reuses the Policy Builder's ReBAC visualization).
- Every manager has a "publish" bar showing draft→published diff.

## 4. Sync to reapers

Two paths, both built on things that exist:

### 4.1 Snapshot path (Phase 1 — correctness backbone)
`publish` → materialize → data bundle vN → `datastore.published` event on the
existing SSE stream → reaper-sync fetches → agent loads into a FRESH
`DataStore` → **atomic Arc swap** (same zero-downtime pattern as policy
hot-swap; interner rebuild happens off the hot path). Recovery and cold start
are trivial: fetch latest snapshot. This alone beats OPA's story.

### 4.2 Delta path (Phase 2 — freshness)
Mutations also append to a **change log** (Postgres table, monotonic
`seq` per namespace): `entity.upserted`, `attribute.set`, `tuple.written`,
`tuple.deleted`, `binding.written`… Agents consume deltas over the existing
SSE stream and apply them in place (`DashMap` insert / `RelationshipGraph`
add_edge — **needs `remove_edge`, which doesn't exist yet**). Contract:

- deltas are idempotent upserts/deletes keyed by `seq` — at-least-once safe
- agent state = `(snapshot_version, last_seq)`; gap detected → refetch
  snapshot (self-healing, no distributed-transaction machinery)
- periodic snapshot compaction bounds the log

**Correctness gate (non-negotiable, extends the existing program):** a new
differential property — *any interleaving of deltas applied incrementally
must equal materializing the final state from scratch* (generate random
mutation sequences; compare full DataStore contents AND a battery of policy
decisions on both). Plus the mutation-testing scope grows to cover the delta
applier. This is the same discipline that caught the fail-open `!=`.

### 4.3 Kafka / event ingestion (Phase 3 — "bring your own pipeline")
The `data_sources` table already anticipates this. Two consumption points:

- **Management-side consumer (default)**: a Kafka source registered on the
  namespace consumes customer topics (CloudEvents envelope; Debezium CDC
  adapter for "mirror my users table") and writes through the SAME record
  APIs — so validation, audit, and the change log all apply. Customers keep
  ownership; we keep coherence.
- **Agent-side direct consumer (advanced, feature-flagged `kafka` in
  reaper-agent/reaper-sync via `rdkafka`)**: for air-gapped/self-hosted
  fleets that can't call home — topic partitions keyed by namespace, same
  delta envelope as 4.2, same `(snapshot, seq)` recovery.

Also: outbound **webhooks** on data changes (subscriptions table exists), so
customers can chain their own reactions.

### 4.4 Consistency posture (documented, honest)
v1 is **eventually consistent with bounded staleness** (SSE delta latency;
`X-Reaper-Data-Version` surfaced on decisions so audits can pin what data a
decision saw — the decision log already records policy identity; add data
version). Zanzibar-style consistency tokens ("zookies") for
read-your-writes enforcement are explicitly **future work** — noted so we
never accidentally claim New-Enemy protection we don't have.

## 5. Engine extensions required (small, listed exhaustively)

1. `RelationshipGraph::remove_edge` (+ reverse index maintenance) — needed
   for delta deletes; mutation-test it like `add_edge`.
2. `DataStore` entity remove / attribute unset (insert exists).
3. Atomic store swap plumbing in the agent (`ArcSwap<DataStore>` or
   equivalent) for snapshot loads — hot path today already goes through an
   `Arc`.
4. Optional `kafka` feature in reaper-sync/agent (rdkafka) — Phase 3 only.
5. NOTHING in the evaluators changes. The data plane compiles to the format
   they already consume — that's the whole trick.

## 6. Phasing

| Phase | Scope | Outcome |
|-------|-------|---------|
| **D1** | ADM schema + Postgres tables + model/entity/role/tuple CRUD APIs + validation + materializer + `publish` → data bundle → existing SSE/deploy path + atomic swap in agent | "Manage your authz data in Reaper, click publish, fleet updates." Closes the OPA gap outright |
| **D2** | Change log + SSE deltas + agent incremental apply + `remove_edge`/entity-remove + **delta≡rebuild differential suite** + data-version on decisions | Seconds-fresh data, provably correct |
| **D3** | Kafka source (management-side) + Debezium CDC adapter + outbound webhooks + agent-side Kafka feature flag + bulk import UX | "Bring your own pipeline"; HR/IdP systems drive authz data |
| **D4** | UI managers (roles/attributes/relationships + graph view) wired to D1 APIs; Policy Builder autocompletes from the LIVE model schema instead of sampled entities | The full loop, visual |
| **Later** | consistency tokens (zookies), SCIM inbound, dedicated/self-hosted stores, temporal versioning & point-in-time restore | enterprise depth |

D1 and D2 are each roughly the size of the decision-log arc. D4's design is
already covered by the frontend brief (extend it with the three managers —
the personas don't change: Priya gets the managers, Ana gets data-change
audit, Dev gets the API).

## 7. Phase D1 — SHIPPED

Implemented (management + agent, fully tested end-to-end):

- Migration `006_data_plane.sql`: datastores, adm_entities, adm_role_bindings,
  adm_tuples, adm_versions.
- `domain/datastore.rs`: ModelDefinition (typed attrs / roles / relations),
  four seed templates, write-time type-strict validation, deterministic
  `materialize()` (BTreeMap ordering → stable checksums).
- `api/datastore.rs`: the full manager surface (provision, model, entities +
  attributes, role-bindings, tuples, publish, versions) under org auth +
  API keys; `datastore_published` SSE event wakes the fleet.
- Agent: `POST /api/v1/data/deploy-version` — **read-replica discipline**:
  recomputes the sha256 over the CANONICAL serde_json serialization and
  rejects mismatches before touching the store (corrupt payload = rejected
  WAL segment); version regressions rejected (monotonic sync); idempotent
  redelivery of the current version is a verified no-op.
- **Configurable staleness budget** (the operator owns the availability vs.
  certainty tradeoff): `REAPER_DATA_MAX_STALENESS_SECS` +
  `REAPER_DATA_STALENESS_MODE`:
  * `monitor` — metrics/log only (default)
  * `flag` — keep serving; every decision log entry carries
    `data_stale: true` so audits see exactly which decisions ran on old data
  * `enforce` — FAIL CLOSED: all evaluations deny with matched_rule
    `data_staleness_exceeded`, and `/ready` returns 503 so orchestrators
    stop routing to the stale agent
  An agent that never synced (bootstrap-file/standalone) has no staleness
  clock — budgets apply only once the data plane is in use.
- **Decision provenance**: every decision log entry now carries
  `data_version` + `data_checksum` (skipped when never synced) — audits pin
  exactly which data every decision saw.
- End-to-end test (`test_data_plane_end_to_end`): provision → typed CRUD
  (mistyped attribute rejected) → publish → checksum round-trip → document
  loaded into the REAL policy engine → combined RBAC+ABAC+ReBAC policy
  decides correctly on the managed data.

Canonicalization note: checksums are computed over serde_json's sorted-key
serialization on BOTH sides — deliberately not sonic_rs (which preserves
insertion order and would make hashes transport-dependent). sonic_rs stays
on the hot paths (evaluation responses, document parsing in DataLoader).

### Persistence decision (any-cloud)

Canonical backing store: **PostgreSQL** — managed offerings on every cloud
(RDS/Aurora, Cloud SQL, Azure Database) and trivial self-host. In managed
mode the end user NEVER sees the database — "provision a datastore" is the
product surface. Self-hosted mode gets the same treatment as reaper itself:
the docker-compose `management` profile and the Helm chart already ship a
PostgreSQL instance, and the datastore rides in the same management DB (one
fewer moving part). The repository layer is plain sqlx with portable SQL;
today's embedded SQLite backend serves dev/self-contained deployments, and
completing the Postgres driver in `db/connection.rs` (currently a stub) is
the one open task — tracked for D2.

## 8. Why this wins (positioning summary)

- **vs OPA**: they made data your problem; we make it a product.
- **vs OpenFGA/SpiceDB**: tuples AND attributes AND roles in one model —
  no second system for ABAC, and the policy language already composes all
  three in a single rule.
- **vs Cerbos**: enforcement points stay stateless for the CALLER (no data
  in the request) because the data plane feeds reapers directly.
- **And it's fast**: the data plane materializes into the same interned,
  lock-free store that evaluates in nanoseconds. Managed data with no
  latency tax — nobody else has that combination.
