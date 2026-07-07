# Reaper DSL v2 — One Language for RBAC + ABAC + ReBAC + Document Policies

**Goal:** a single policy language where RBAC, ABAC, and ReBAC compose freely in
one rule set, plus the OPA-style capabilities people actually use OPA for —
validating arbitrary JSON documents (Terraform plans, Kubernetes admission
requests, CI config checks) with human-readable violation messages — while
keeping the sub-microsecond compiled hot path for authorization decisions.

## Where the DSL stands today (assessment)

**Strong already:**

| Capability | Status |
|---|---|
| RBAC | ✅ `user.role == "admin"`, `"editor" in user.roles`, RBAC materialized views in DataStore |
| ABAC | ✅ arbitrary entity attributes, comparisons, `&&`/`\|\|`/`!`, first-match rules, default decision |
| Expressions | ✅ variables (`:=`), set/array/object comprehensions, method chains (string/collection/set/aggregate), builtins: `json::parse/stringify`, `math::*`, `regex::matches`, `time::*`, type checks |
| Performance | ✅ compiled evaluator (~sub-µs) with AST-interpreter fallback; interned attributes; multi-index store |
| Formats | ✅ `.reap` / YAML / JSON compile to one representation; `.rbb` bundles; signing |

**Gaps against the goal:**

1. **No ReBAC.** Entities have only an optional `parent` link. There are no
   named relationship edges (`owner`, `editor`, `member`), no reverse lookup,
   no transitive traversal ("alice can view doc because she's a member of a
   group that owns the folder the doc is in").
2. **No structured input document.** `context` is a flat
   `HashMap<String, String>`; chained access (`context.a.b`) is explicitly
   unsupported. A Terraform plan or K8s AdmissionReview cannot be evaluated at
   all — this is *the* blocker for OPA-style use.
3. **Decisions are bare allow/deny.** OPA's CI/gatekeeper value comes from
   `deny[msg]` — a set of human-readable violations ("S3 bucket 'logs' has no
   encryption"). Reaper returns only the decision + matched rule name.
4. **No quantifier sugar.** Comprehensions can express "every/some" but
   awkwardly (`count()` comparisons); document validation lives on `every
   resource in the plan …`.

## Design

### Phase 1 — Structured `input` document — ✅ IMPLEMENTED

- New request field `input: serde_json::Value` (optional, alongside
  principal/action/resource which stay optional for pure document checks).
  Agent API: `POST /api/v1/messages` and a new `POST /api/v1/check` accept a
  full JSON body; CLI: `reaper-cli eval --input plan.json`.
- New entity keyword **`input`** with deep path access and iteration:

  ```reap
  policy terraform_guard {
      default: allow,
      rule no_public_buckets {
          deny if {
              rc := input.resource_changes[_];
              rc.type == "aws_s3_bucket";
              rc.change.after.acl == "public-read"
          }
      }
  }
  ```

- `context` upgraded the same way (nested JSON allowed, backward compatible
  with flat string maps).
- Implementation: `EvalValue` already models objects/arrays; bind `input` as a
  lazily-converted `EvalValue` tree; extend grammar `entity = { "user" |
  "resource" | "context" | "input" }`; deep chained access + `[_]` iteration
  in both AST and compiled evaluators (compiled path: fall back to AST for
  `input`-touching rules first, optimize later — authorization hot path is
  unaffected).

### Phase 2 — Violations with messages — ✅ IMPLEMENTED

(Surfaced as `POST /api/v1/check` on the agent and `reaper-cli check -p policy.reap -i plan.json` with exit code 1 on violations.)

- Rule form gains an optional message:

  ```reap
  rule no_public_buckets {
      deny with message concat("bucket ", [rc.name, " is public"]) if { ... }
  }
  ```

- Semantics: for *decision* calls, unchanged first-match behavior. For
  **check** calls (`/api/v1/check`, `reaper-cli check`): evaluate ALL deny
  rules, collect every violation message → `{"allowed": false, "violations":
  ["…", "…"]}`. Exit code for CI. This mirrors OPA `deny[msg]` without a new
  evaluation model — it is a second driver over the same rules.
- Decision-log entries carry `violations` when present (explain tier already
  ships context).

### Phase 3 — First-class ReBAC — ✅ IMPLEMENTED

Shipped as `rebac::related(subject, relation, object)`,
`rebac::reachable(subject, relation, object, via, max_depth)` (subject-side
group expansion), and `rebac::inherited(subject, relation, object, up,
max_depth)` (object-side ancestor walk). Edges are declared per entity
(`"relationships": {"owner": ["alice"]}`), doubly indexed at load
(interned-u32 keys, sorted SmallVec adjacency, binary-search membership),
traversals are bounded + cycle-safe with a hard node budget. Static-arg
calls compile into the sub-microsecond path (CompiledCondition::RebacCheck,
verified compiled-vs-AST parity); dynamic ids run on the AST evaluator.
Original sketch below for reference.

- **Data model:** entities gain named, directed edges:

  ```json
  {"id": "doc1", "type": "Document",
   "relationships": {"owner": ["user:alice"], "parent": ["folder:eng"]}}
  ```

  DataStore builds forward and reverse edge indexes (interned, O(1) per hop).
- **DSL:**

  ```reap
  // direct: alice is an owner of doc1
  rule owner_can_edit { allow if user in resource.rel("owner") }

  // transitive with bounded depth: membership through groups
  rule group_viewer {
      allow if user in resource.rel_any("viewer", via: "member_of", max: 4)
  }

  // upward traversal: permission inherited from ancestor folders
  rule folder_inherit {
      allow if user in resource.ancestors("parent").rel("owner")
  }
  ```

  `rel(name)` → set of entity ids (1 hop); `ancestors(edge)` → transitive
  closure along an edge (cycle-safe, bounded); `rel_any(name, via, max)` →
  Zanzibar-style usersets (relation + group expansion). All return sets, so
  they compose with existing set ops and `in` — and with RBAC/ABAC conditions
  in the same rule:

  ```reap
  // combination: role + attribute + relationship in ONE rule
  rule sensitive_docs {
      allow if {
          user.role == "analyst" &&
          user.clearance_level >= resource.clearance_level &&
          user in resource.rel_any("viewer", via: "member_of", max: 3)
      }
  }
  ```

- Performance: 1-hop rel checks are two interned-set lookups (~100ns); bounded
  BFS for transitive with per-request memoization; reverse indexes make
  "who can see X" cheap for future list-APIs.

### Phase 4 — Recipes + parity surface

- `reaper-cli check --policy tf.reap --input plan.json` (CI: nonzero exit on
  violations, NDJSON/JSON output) — the `conftest` workflow.
- Cookbook under `docs/policies/`: Terraform plan guardrails, Kubernetes
  admission (labels/limits/registries), JSON schema-ish validation patterns,
  ReBAC modeling guide (drive/org-hierarchy examples).
- Gherkin feature files for each model and combinations (extends existing
  rbac/abac/multilayer features with rebac.feature, terraform.feature).

## Non-goals (explicit)

- Not re-implementing Rego. Reaper stays rule/decision-oriented with a
  Rust-like syntax; we add the *capabilities* people use OPA for, not its
  language semantics.
- No unbounded graph traversal — every traversal takes an explicit depth
  bound; cycles are detected. Sub-µs authorization stays the design center.
- Cedar evaluator stays as-is (interop path), unaffected.

## Build order & why

1. **Phase 1 (input document)** — foundational; smallest surface that unlocks
   the entire OPA-style use family. Everything else composes with it.
2. **Phase 2 (violations)** — small delta on Phase 1, delivers the visible
   "conftest/gatekeeper" experience end-to-end (CLI + API + decision log).
3. **Phase 3 (ReBAC)** — the deepest change (data model + indexes + traversal
   + both evaluators); worth doing after the request-shape changes settle.
4. **Phase 4 (recipes)** — turns capability into adoption.
