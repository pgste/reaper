# Rego ↔ Reaper DSL Gap Analysis

**Status:** ANALYSIS — feature-planning input, nothing scheduled.
**Sources:** OPA policy-language documentation + builtin metadata
(open-policy-agent/opa, fetched 2026-07-20); Reaper inventory from the shipped
grammar (`crates/policy-engine/src/reap.pest`), AST (`reap/ast.rs`), builtin
dispatch (`reap/ast_evaluator/function_dispatch.rs`, `method_dispatch.rs`),
compiled condition set (`evaluators/reaper_dsl/types/compiled_condition.rs`),
and every compile-fallback trigger in `reap/compiler/`.
**Authoritative Reaper spec:** `docs/reference/reap-language.md` + the grammar.
(`docs/reference/policy-syntax.md` is an early vision doc; features like
`priority:` in it are NOT implemented.)

---

## 1. The two contract philosophies (read this first)

The feature gaps below are mostly *consequences* of one root divergence:

**OPA/Rego is a query language over unshaped documents.** `input` is any
JSON, `data` is any JSON, a policy is a set of rules that derive *any*
document as output. Nothing is declared; nothing is indexed; `allow` is a
convention, not a contract.

**Reaper is a decision engine over a shaped request.** The request is
`(principal, action, resource, context, actor)`; entities are typed records
in an indexed store; a policy produces exactly `allow`/`deny` (+ violation
messages in check mode); a missing attribute is `null` and fails closed;
cross-type comparison is `false`, never a coercion, and never "undefined".

Almost every Reaper performance and safety property is purchased *with* the
strict contract:

| Property | Why the strict contract enables it |
|---|---|
| Sub-µs compiled evaluation | Leaves anchor to a closed entity set → pre-interned `CompiledCondition`s, no document traversal |
| Pruning index (evaluate-all at 10k policies) | `resource`-anchored leaves are statically extractable |
| Filter compilation design (`FILTER_COMPILATION.md`) | Dependence analysis needs classifiable leaves |
| Fail-closed semantics | No "undefined" tri-state; absent → `null` → non-match |
| Frozen decision corpus / compat guarantee | A closed language surface is checksummable |
| Totality (no divergence, depth-capped, bounded ReBAC BFS) | No recursion, no user-defined evaluation, no I/O |

And OPA's loose contract buys the opposite set: universal domain coverage
(authz, admission control, CI checks, config validation with one engine),
zero-modeling onboarding (send whatever JSON you have), and policy evolution
without engine releases. Its costs are the famous footguns: `undefined` vs
`false` confusion, typo'd paths silently never matching, per-integration
response shapes, unpredictable performance (hence their need for partial
evaluation), and weak static analyzability.

**Reaper already contains the hybrid answer:** the `input` document
(`ast.rs` `Entity::Input`) is a *loose island inside the strict contract* —
arbitrary per-request JSON with OPA-style comprehensions and check-mode
`deny with message`. That is the right shape; its problem today is that it
is AST-only (§4). The strategic direction this analysis supports: **keep the
request/entity contract strict, and invest in the loose island** rather than
loosening the core.

---

## 2. Language-construct matrix

Legend: ✅ equivalent · 🟡 partial · ❌ gap (candidate feature) ·
🚫 missing by design (documented refusal, not a roadmap item)

| Rego construct | Reaper status | Notes |
|---|---|---|
| Complete rules (rule := value) | 🚫 | Rules produce decisions, not values. Data derivation belongs to the DataStore/materialized views, not policies. |
| Partial set/object rules (`p contains x if …`) | 🚫 | Same rationale. The in-rule analog is a comprehension bound with `:=`, scoped to the rule. |
| Incremental definitions (same-name rules OR together) | ✅ | Multiple `allow`/`deny` rules; deny-overrides then allow-any. Semantically the union Rego gives, with a *stronger* fixed precedence. |
| `default` rules | ✅ (stronger) | `default: allow\|deny` is **mandatory**; no undefined outcome exists at all. |
| `:=` assignment | ✅ | Rule-scoped; RHS: comprehensions, comparisons, function/method calls, attrs, literals. |
| `=` unification | 🚫 | Datalog-specific; adds solver semantics for no authz benefit. |
| `==`/ordering/`!=`/`in` | ✅ | Type-strict, no coercion (frozen contract). Rego is also non-coercing but yields undefined; we yield `false`. |
| Dot/bracket refs, `["key"]`, `[0]` | ✅ | On the 5 entities + `input`. |
| `[_]` wildcard iteration | ✅ | Existential over collections. |
| Variable index binding (`sites[i]`; `i` reused across terms) | 🟡 | Only anonymous `[_]`; no named index binding. Rarely limiting given comprehensions; low priority. |
| Comprehensions (array/set/object, filters) | ✅ | All three kinds, multi-filter. 🟡 iterator source must be an attribute/variable, not another comprehension (no nested-source). |
| `some` (explicit existential) | 🟡 | `[_]` covers the semantics; no named-binding sugar. |
| `every` (universal) | 🟡 | `.all()`, comprehension-count equality (`[x | filter].count() == coll.count()`), no block syntax. |
| `not` (negation-as-absence) | 🟡 | `!` on boolean exprs + `== null` absence checks. No rule-reference negation — but there are no named value-rules to negate (see functions gap). |
| `else` / ordered alternatives | ❌ (low) | Fixed deny-overrides + first-allow-wins covers the authz cases; per-rule `else` chains would complicate the frozen semantics for little gain. |
| **User-defined functions / helper predicates** | ❌ **(top authoring gap)** | No `func`, no named reusable conditions. Rego teams live on helper rules; `.reap` policies repeat condition blocks verbatim. |
| **Imports / packages / multi-file composition** | ❌ **(top authoring gap)** | One file = one policy; `package` is a metadata string. Declared "Future Enhancement" in the language doc. |
| `with` (input/data mocking) | 🚫 | Testing is externalized: `reaper-cli test`/`test-suite` inject policy+data+request fixtures. Equivalent power, no in-language override machinery. (`with message` is unrelated — violation text.) |
| Metadata annotations | 🟡 | Free-form metadata fields ship; JSON-Schema annotations (typed entities) are a declared future item. |
| String interpolation / raw strings | ❌ (small) | Only `concat(...)`; regex patterns pay double-escaping. |
| Infix arithmetic (`+ - * /`) | ❌ | `math::` builtins only. Ergonomics gap for quota/limit policies (`used + requested <= quota` is unwritable infix). |
| Array/object/set literals | ✅ | In grammar, including set literals — ahead of common belief. |
| Loops / recursion | 🚫 | Totality by construction (depth cap 64, bounded BFS). This is a *feature*; Rego is also loop-free but allows unbounded `graph.reachable`. |

**Reaper constructs with no Rego equivalent** (the flip side, worth stating
in any comparison): `taint::trusted`/`taint::level` request-provenance gates
(agentic authz), the `actor` entity (delegation/on-behalf-of), `rebac::`
bounded relationship traversals as language primitives (Rego needs
`graph.reachable` over a hand-built adjacency object, unbounded), mandatory
default decision, check-mode violations as a typed engine surface (Rego does
this by convention), the frozen-decision compatibility contract, and the
compiled sub-µs path itself.

## 3. Builtin library comparison (by OPA category)

| OPA category | Representative | Reaper status |
|---|---|---|
| Aggregates (`count/sum/max/min/sort/product`) | ✅ mostly | `.count() .sum() .min() .max() .any() .all() .sort()`; no `product` (niche). |
| Strings (`concat/contains/split/upper/lower/trim/…`) | ✅ core | Methods + `concat`; ❌ no `sprintf`/`format_int`, no glob. |
| Arrays (`concat/flatten/reverse/slice`) | ✅ mostly | `.slice() .reverse() .first() .last() .unique()`; no `flatten`. |
| Sets (`union/intersection/difference`) | ✅ | Methods on collections. |
| Objects (`object.get/keys/filter`, `json.patch`) | 🟡 | `.keys() .values() .has_key()`; no `object.get(default)`/filter/patch. |
| Regex | ✅ | `regex::` + `.matches/.find/.find_all/.replace` (thread-local cache). |
| Time | ✅ | `time::` now/parse/format/add/compare — parity for authz needs. |
| Types (`is_string`…) | ✅ | Global `is_*` family. |
| JWT (`io.jwt.decode/verify_*`) | 🟡 deliberate | `jwt::decode`/`header` only — **verification belongs at the trust boundary** (gateway/agent auth), not per-decision. A `jwt::verify` against operator-configured keys is a possible P3 if demanded. |
| Graph (`graph.reachable`, `walk`) | ✅ better | `rebac::related/reachable/inherited` — indexed, budgeted, depth-clamped (≤16). OPA's is an unbounded walk over a policy-built object. |
| **Net/CIDR** (`net.cidr_contains/merge`) | ❌ **(real gap)** | IP-range conditions are bread-and-butter authz (`source_ip in 10.0.0.0/8`). Today impossible except string hacks. |
| **Encoding** (`base64/hex/url/json/yaml`) | 🟡→❌ | `json::parse/stringify` ship; base64/url-decode matter for the api-gateway policy class. |
| Glob | ❌ (small) | Wildcard resource matching exists at the Simple-policy layer, not as a DSL builtin. |
| Conversions (`to_number`) | ❌ (small) | Type-strict comparisons make this *more* useful, not less (explicit casts beat silent false). |
| Crypto (`crypto.sha256/hmac/x509`) | 🚫 mostly | Per-decision crypto invites doing trust-boundary work in policy. Revisit only with a concrete case (e.g. cert-attr checks). |
| Bits / units / semver / GraphQL | ❌ (niche) | Demand-driven at best. |
| **`http.send`** | 🚫 **emphatically** | The totality/latency/security refusal that defines the engine. External facts enter via the DataStore/sync, never mid-decision. |
| Nondeterministic (`rand`, `uuid`, `net.lookup`) | 🚫 | Decisions must be replayable (decision log + frozen corpus). Clock (`time::now*`) is the one sanctioned nondeterminism, as in OPA. |

## 4. The gap Rego does not have: our compiled/AST bifurcation

OPA has one evaluator; every Rego feature runs at the same (slow) speed.
Reaper has two: the sub-µs compiled path and the AST interpreter — and
**fallback is per-policy, not per-rule** (`reap/mod.rs:133-146`): one
uncompilable construct anywhere sends the *whole policy* to the interpreter.
That is a silent 10–100× cliff and, at fleet scale, a capacity planning trap.
It is also invisible in the language docs — an author cannot tell which side
of the cliff a construct lands on.

Fallback trigger clusters (every `Err` site inventoried from
`reap/compiler/`):

1. **`input` document access** (5 sites) — the *entire* OPA-competitive
   policy class (terraform / kubernetes / api-gateway in the policy library)
   is AST-only today.
2. **Variable-shape comparisons** — var-attr↔var-attr, var↔entity-attr,
   several method-call comparison shapes.
3. **Method calls in expressions/comprehension outputs** — incl. every
   function-call expression assignment (`x := time::now()`).
4. **Dynamic-id ReBAC / dynamic-key taint** — only literal/entity refs
   compile.
5. **Odd small ones** — literal-value assignment (`x := "admin"`), indexed
   non-equality comparisons.

This bifurcation is arguably a **bigger competitive liability than any
missing language feature**: our headline vs OPA is speed, and the policies
most directly comparable to OPA workloads don't run on the fast path.

## 5. Candidate feature plan (ranked; nothing scheduled)

**P1 — close the performance bifurcation where it contradicts the pitch**
- **Compile `input` document access.** Unlocks the compiled path for the
  admission-control/CI policy class; also a precondition for those policies
  ever benefiting from pruning/filtering. Largest single win; substantial
  compiler work (paths into arbitrary JSON need a compiled access plan).
- **Compile the variable-comparison + expression-assignment clusters.**
  Mechanical, enumerable from §4's trigger list; each closure shrinks the
  cliff. Gate: the existing compiled-vs-AST differential.
- **Per-rule (not per-policy) fallback** as an interim: one exotic rule
  should not de-optimize its siblings. Needs a mixed-evaluator policy shape
  + the same differential gate.

**P1 — authoring parity (the gaps Rego users will actually hit)**
- **User-defined helper predicates** (`func active_admin(u) := …` usable in
  conditions) with the same totality rules (no recursion, depth-counted).
- **Imports** of helper libraries across policies (declared future item).
  Both change the *language contract* → language-version bump + frozen-corpus
  machinery per `DSL_COMPATIBILITY.md`.

**P2 — high-demand builtins & ergonomics**
- `net::cidr_contains` / `cidr_overlaps` (real authz gap).
- `base64::`/URL decode (api-gateway class).
- Infix arithmetic (`+ - * /` on numbers) — or at least `math::add`-style
  parity; quota policies currently unwritable.
- Named `some`/`every` sugar over the existing semantics (readability parity
  with modern Rego).
- `object.get(key, default)`, `flatten`, `to_number`.

**P3 — nice-to-have**
- Schema annotations for entities (declared future; synergizes with the
  filter design's column mapping).
- String interpolation + raw strings.
- `jwt::verify` against operator-configured JWKS (only if demanded; the
  trust-boundary argument stands).
- Glob builtin.

**Documented refusals (anti-roadmap — write these into the language doc):**
`http.send` and any mid-decision I/O; unification; value-producing rules /
arbitrary output documents; `rand`/`uuid`/nondeterministic builtins;
unbounded graph traversal; `with`-style in-language mocking. Each refusal is
a *sellable property* (replayable decisions, total evaluation, bounded
latency) — state them as such rather than leaving them as absences.

## 6. Loose-contract pros/cons, resolved for Reaper

**What adopting OPA's looseness would cost us:** interning + indexes (and
with them the sub-µs claim), the pruning index, the filter-compilation
design, fail-closed typing, and a checksummable compatibility contract. Not
worth it at any price — the strict core is the product.

**What we genuinely lose by staying strict, and the mitigation:**

| Loss | Mitigation |
|---|---|
| Zero-modeling onboarding (send any JSON) | The `input` island already provides it per-request; invest there (P1 compile) instead of loosening entities |
| One engine for admission control / CI / config validation | Same: `input` + check-mode *is* that engine; it needs the fast path and a documented story, not new semantics |
| Policy evolution without engine releases | Builtin/stdlib growth is additive and version-gated; the frozen corpus makes this *safer* than OPA's, at the cost of velocity |
| Arbitrary output documents | Deliberate: decisions + violations are the contract; data derivation belongs to views/DataStore |

**Bottom line:** OPA's loose contract is neither pure marketing nor pure
win — it is the source of both their reach and their performance/footgun
problems. Our strict core out-executes it on the decision path (and §1's
table is the evidence that the strictness is *load-bearing*). The credible
competitive risk is not a missing Rego feature; it is (a) the compiled/AST
cliff making our OPA-comparable workloads slow, and (b) authoring
ergonomics (functions/imports) making policy suites tedious at scale. Those
two clusters are where feature planning should aim first.
