# Reaper — Security & Performance Review

**Goal of this review:** find what blocks Reaper from being a *compelling*, credibly-faster-than-OPA
authorization platform, with particular attention to whether **compiled mode** actually works.

**Method:** direct source reading of the hot path plus four focused deep-dives (compiled-mode
correctness — including ~8,800 empirical compiled-vs-AST comparisons; control-plane security;
request-path performance; benchmark validity). All findings below are backed by `file:line`
evidence.

---

## Executive summary

| Area | Verdict |
|------|---------|
| **Compiled DSL mode** | ✅ **Works and is correct.** Compiles once at deploy, cached on the hot path, byte-for-byte parity with the AST interpreter across all 17 shipped policies. The suspicion that it silently no-ops is **unfounded**. Real gaps are *robustness*, not correctness (see A4/A5). |
| **"10x faster than OPA" / "<1µs p99"** | ❌ **Not currently defensible.** The marketed numbers come from a benchmark where **OPA is silently broken** (returns wrong answers), a **UDS-vs-TCP** transport advantage, and an **in-process micro-benchmark** on trivial policies. Fixable, but today the claim would not survive scrutiny. |
| **`SimplePolicyEvaluator` (the "<1µs" path)** | ❌ **Functionally broken for real authz** — ignores `action`, `principal`/context, and rule `conditions`. It is a resource-string compare, not RBAC/ABAC. |
| **Decision cache** | ❌ **Correctness + security + throughput hazard** — never invalidated on hot-swap/data change, key omits policy identity, and it takes a global write lock + O(capacity) scan on every *hit*. |
| **Control plane (management service)** | ❌ **Critical holes** — cross-tenant privilege escalation, reversible XOR "encryption" of OAuth tokens, OAuth CSRF, no real auth rate-limiting, reset tokens logged. |

**Headline:** the engine's *compiled path is genuinely good*. What undermines the product today is
(1) a broken "simple" evaluator and inconsistent endpoint semantics, (2) a decision cache that trades
correctness for a speed it doesn't actually deliver, (3) a benchmark that can't back the marketing, and
(4) a control plane with a total tenant-isolation break. Fix these and the 10x story becomes real and
defensible.

---

## Part A — Data-plane correctness & security (the engine)

### A1. `SimplePolicyEvaluator` ignores action, principal, and conditions — **CRITICAL**
`crates/policy-engine/src/evaluators/simple.rs:106-111`

```rust
fn matches_rule(&self, rule: &PolicyRule, request: &PolicyRequest) -> bool {
    rule.resource == "*" || rule.resource == request.resource
}
```

The match considers **only the resource string**. It never checks `request.action`, never checks the
principal/context, and never evaluates `rule.conditions` (the field exists on `PolicyRule` but is read
nowhere in this evaluator). The decision-tree "optimization" is the same: it partitions on
`rule.action` as the *effect* (Allow/Deny), and hardcodes `"principal" => "*" // Simplified for now`
(`crates/policy-engine/src/optimizer/decision_tree.rs:376`). The tree path also runs against an **empty
thread-local `DataStore`** (`simple.rs:124-127`), so ABAC cannot work there either.

**Impact:** any request whose resource matches a rule pattern gets that rule's action regardless of who
is asking or what they're doing. A rule intended to allow `alice` to `read` `/doc` will allow *anyone*
to *any-action* `/doc`. Rules with conditions are enforced as if unconditional (fail-open). This is not
RBAC or ABAC — it's a resource ACL with the subject and verb removed.

**Fix:** either (a) make `SimplePolicyEvaluator` evaluate `action`, principal, and `conditions`, or
(b) formally deprecate it and route all real policies through the Reaper DSL. Do **not** benchmark or
document it as an RBAC/ABAC engine in its current form.

### A2. The two evaluation endpoints have opposite default decisions — **CRITICAL**
`services/reaper-agent/src/handlers/evaluate.rs:219` vs `:541`

- `POST /api/v1/messages` (`evaluate_policy`) initializes `final_decision = PolicyAction::Allow`
  (`matched_rule = "default_allow"`). If no policy explicitly denies, the request is **allowed**.
- `POST /api/v1/fast-messages` (`fast_evaluate_policy`) initializes `final_decision = PolicyAction::Deny`.

The same request against the same policies returns a **different decision** depending on which endpoint
is called. The default endpoint is **fail-open** — the opposite of what an authz gateway must do. (The
"policy not found" cases correctly deny, but the multi-policy accumulation default does not.)

**Fix:** default-deny everywhere. Make both handlers share one evaluation core so semantics can't drift.
Add a test asserting identical decisions across endpoints for a matrix of inputs.

### A3. Decision cache: stale after hot-swap, and key omits policy identity — **CRITICAL**
`crates/policy-engine/src/decision_cache.rs`, `services/reaper-agent/src/handlers/evaluate.rs:179,384`,
`services/reaper-agent/src/handlers/data.rs:210`

Two distinct defects:

1. **No invalidation on policy deploy or data change.** The cache is only ever `get`/`insert`-ed in the
   evaluate handlers. `deploy_policy` does not touch it, and the data-load handler clears the data store
   (`data.rs:210`) but **not** the decision cache. Default config is `enabled=true`, `TTL=300s`
   (`cache_config.rs:58-66`), or *forever* if TTL is set to 0. So after you hot-swap a policy (the
   flagship "zero-downtime" feature) or revoke a role, the agent keeps serving the **old decision** for
   up to 5 minutes — or indefinitely. For an authz product this is a security defect: a deny that
   doesn't take effect.
2. **Cache key does not include the policy.** `CacheKey::from_request` (`decision_cache.rs:52`) hashes
   only `action`, `resource`, and `context`. A request evaluated against policy A and a *different*
   request against policy B with the same `(action, resource, principal)` collide — B receives A's
   cached decision. "Evaluate-all" requests and single-policy requests also share keys.

**Fix:** version the cache — include `policy_id` + `policy_version` (or a global engine epoch counter
bumped on every deploy/data change) in the key, and `clear()` (or bump the epoch) inside
`deploy_policy` and the data mutation handlers. Consider defaulting the decision cache **off** until
invalidation is correct; a fast wrong answer is worse than a correct one here.

### A4. Compiled / DSL policies do not survive an agent restart — **HIGH**
`services/reaper-agent/src/handlers/policies.rs:246`, `services/reaper-agent/src/cache.rs:168-174`,
`services/reaper-agent/src/main.rs:248-256`, `crates/policy-engine/src/engine/policy.rs:180-184`

A `.reap` policy deployed via `POST /api/v1/policies/compile` is stored as `language: Custom` with a
pre-built evaluator (`policies.rs:246,253`). It's also written to the disk policy cache — but
`CachedPolicy::to_enhanced_policy` drops the evaluator (`cache.rs:173`, "Will be rebuilt"). On restart,
`main.rs:250` rebuilds via `policy.build_evaluator()`, and for `Custom` that returns
`Err("Custom policy language not yet implemented")` (`policy.rs:180-184`). The `continue` at
`main.rs:255` then **silently drops the policy**. Combined with A2's fail-open default endpoint, a
restart can turn a previously-denied request into an allow with no error surfaced to the operator.

**Fix:** make the Reaper DSL a first-class `PolicyLanguage::ReaperDsl` variant that `build_evaluator`
can reconstruct from `content` (it has the source — recompile it). Persist compiled bundles (`.rbb`) so
restart restores the compiled form. Fail *closed* and loudly if a cached policy can't be rebuilt.

### A5. Reaper DSL is not a first-class language; entry points disagree on fallback — **MEDIUM**
`crates/policy-engine/src/engine/policy.rs:151-185`, `services/reaper-agent/src/bootstrap.rs:133-150`,
`services/reaper-agent/src/handlers/policies.rs:230`, `tools/reaper-cli/src/main.rs:499,767`

The DSL is shoehorned into `PolicyLanguage::Custom`, and the canonical `build_evaluator()` can't build
it (A4). As a result the DSL only reaches evaluation through side paths, and those paths handle a
compiler rejection **inconsistently**:

- **Bootstrap file load** → on compile error, silently falls back to the AST interpreter
  (`bootstrap.rs:137-149`) — correct result, no speedup.
- **API deploy** (`deploy_compiled_policy`) → **400 Bad Request**, no fallback.
- **CLI `eval`/`validate`** → hard error, no fallback.

So the *same* policy using a compiler-unsupported construct works as a dropped-in file but is rejected
via API/CLI. (Compiler-rejected constructs are narrow: literal-value assignments like `x := "admin"`,
non-whitelisted namespaced functions such as `json::*` used as standalone conditions, bare non-string
entity methods, and string ordering comparisons — see `reap/compiler/mod.rs:145-148,342-355`,
`reap/compiler/methods.rs:140-146`. When rejected, the compiler errors; it never returns a *wrong*
answer.)

**Fix:** one routing policy everywhere — either always fall back to AST (log a warning + metric), or
always reject. Recommend: fall back to AST + emit a `reaper_policy_uncompiled_total` metric so you can
see which policies are missing the fast path. Expand the compiler to close the known gaps.

### A6. `matched_rule` reporting is decorative and can be wrong — **LOW**
`crates/policy-engine/src/engine/mod.rs:230-241`

For Simple policies the engine computes `matched_rule` in a **separate** loop that returns the first
rule whose *resource* matches — independent of what the evaluator actually decided, and ignoring action.
The reported "matched rule" in decision logs/responses can therefore point at the wrong rule. It's also
a second O(n) scan (see C4).

**Fix:** have the evaluator return `(action, matched_index)` in one pass
(`SimplePolicyEvaluator::evaluate_with_details` already exists — use it) and delete the duplicate loop.

---

## Part B — Control-plane security (management service)

Ranked; full exploit narratives available in the working notes. `file:line` anchors given.

### B1 (CRITICAL). Cross-tenant privilege escalation — org "Owner" == global super-admin
`auth/middleware.rs:416` maps `OrgRole::Owner → ["admin"]`; `auth/scopes.rs:165-167` makes
`Permission::has()` return true for **any** scope when `Scope::Admin` is present. Every self-service
signup makes the caller an Owner of their new org (`api/users/auth.rs:96-104`). Cross-org guards are
written as `if user.org_id != target.id && !user.has_permission(Scope::Admin)`
(`api/agents.rs:130`, `api/auth/certificates.rs:43,67-75`) — the `!has(Admin)` clause is therefore
**always false for every Owner**, disabling the "is this my org?" check platform-wide. Any tenant can
act on any other tenant's agents and mTLS certs by passing another org's slug.
**Fix:** separate a global platform-admin scope from org-owner; re-check membership/role against the
*target* org (as the `oauth/*` handlers already do via `get_role`).

### B2 (CRITICAL). GitHub OAuth tokens stored with reversible XOR (or plaintext)
`api/oauth/helpers.rs:81-113` "encrypts" tokens with repeating-key XOR over `SHA256(key)` (no AEAD, no
nonce, keystream reused). The key is `jwt_secret.unwrap_or_default()` (`github.rs:144-149`) — an empty
string when `jwt_secret` is unset (`config/auth.rs:17,29`), making storage effectively plaintext. Tokens
are *also* embedded verbatim into stored clone URLs (`github.rs:301-335`). A DB read yields `repo`-scoped
tokens to victims' private repos. **Fix:** AES-GCM/XChaCha20-Poly1305 with a dedicated random key from a
secret manager; never store tokens in URLs; require a strong key at boot.

### B3 (HIGH). OAuth `state` is unsigned and unbound → CSRF / connection hijack
`api/oauth/types.rs:46-60` — `state` is plain base64 JSON with no HMAC; the callback
(`github.rs:78-223`) is unauthenticated and trusts `state.user_id`/`state.org_slug`, upserting the
connection `ON CONFLICT (org_id, provider) DO UPDATE` (`:179-187`). An attacker forges a state naming
the victim org, completes GitHub auth with their own account, and overwrites the victim org's connection
to point at the **attacker's** token — a supply-chain injection into policy sources. **Fix:** sign the
state (HMAC) or store it server-side bound to the session; re-verify membership on callback.

### B4 (HIGH). Auth endpoints effectively unthrottled
`config/rate_limit.rs` — `check_login`/`check_signup` exist (`rate_limit.rs:91-107`) but are **never
called**; the only active limiter is a single global, un-keyed token bucket (client IP is computed then
discarded, `rate_limit.rs:63`). No per-account/per-IP limit, no lockout → practical password brute-force
and credential stuffing on `POST /auth/login`. `X-Forwarded-For` is trusted unconditionally
(`rate_limit.rs:156-163`). **Fix:** per-IP + per-account limits + backoff/lockout on
login/signup/reset; only trust XFF from a configured proxy.

### B5 (HIGH). Password reset token written to logs
`api/users/auth.rs:407-411` logs the **raw** reset token at INFO. Since email delivery is a TODO, logs
are the only delivery channel — anyone with log access takes over any account. **Fix:** never log
secrets; deliver by email; log a non-sensitive event id.

### B6 (HIGH). mTLS revocation/expiry/binding is dead code
`auth/mtls.rs:445-480` `validate_certificate` has **no caller**. The `client_certificates` table
(revocation, validity window, agent binding) is never consulted at auth time; revoking a cert has no
effect. (The agent's TLS does proper chain verification via `WebPkiClientVerifier`, `tls.rs:127-135`,
but there is no CRL/OCSP or app-layer revocation anywhere.) **Fix:** wire `validate_certificate` into
the authenticated request path using the TLS peer-cert fingerprint (not a header).

### B7 (MEDIUM). Assorted
- **Username enumeration** on login — missing user skips Argon2 (timing oracle) and distinct
  suspended/unverified errors precede password check (`api/users/auth.rs:170-187`).
- **`jwt_secret` reused** for JWT signing *and* the OAuth cipher, only length-checked, optional at boot
  (`config/auth.rs:40-44`, `middleware.rs:207`).
- **SSRF via org-configured JWKS URL** fetched server-side with no allowlist (`auth/jwks.rs:206-213`) —
  can hit `169.254.169.254` etc.
- **JWKS audience optional** (`jwks.rs:168-170`) — tokens for a different RP at the same issuer are
  accepted.

**Verified-good (not vulnerabilities):** JWT alg is pinned (HS256 via `Validation::default()`,
`jwt.rs:102`; per-key alg on the JWKS path) — no `alg=none`/confusion; Argon2id with random salt and
constant-time verify (`users/password.rs:11-29`); API keys from a CSPRNG, stored as SHA-256 with indexed
equality lookup (`api_key.rs:70-86`); all reviewed queries use bound parameters (no SQLi found).

---

## Part C — Performance (getting to a real 10× over OPA)

The default enforcement route `POST /api/v1/messages` is wired to the **instrumented** handler
(`main.rs:458`), not the fast one. Ranked by impact.

### C1 (P0). Decision cache: global write lock + O(capacity) scan + 3 allocs on every *hit*
`crates/policy-engine/src/decision_cache.rs`
`get()` calls `touch_lru()` (`:229`) on every hit → `lru_order.write()` (exclusive) then
`retain(|k| k != key)`, an **O(n) scan of the whole deque under a write lock** (default n=10,000). Every
concurrent hit serializes here, so throughput collapses toward single-threaded and p99 explodes. On top
of that, `CacheKey::from_request` allocates 3 `Arc<str>` + sorts a `Vec` of context keys **per get and
per insert** (`:52-83`). The advertised "~50-100ns hit" is fiction.
**Fix:** replace with a sharded concurrent cache (`moka`/`quick_cache`); key on the pre-computed `u64`
hash; drop the `Arc<str>` allocations. This is the single biggest throughput unlock. (Do C1 together
with A3 — same file.)

### C2 (P0). Decision logging does JSON serialization + file I/O + two write locks on the request thread
`crates/policy-engine/src/decision_buffer.rs:83-119`
When file logging is on, `log()` runs inside the request: `to_ndjson()` serialization, a `RwLock` write
into a `BufWriter` (periodic real `write()` syscalls land on whatever request triggers a flush), then a
*second* global write lock on the ring buffer. This defeats the "lock-free ring buffer" claim.
**Fix:** push entries onto an MPSC / lock-free ring and drain on a dedicated writer task; never
serialize or touch the file on the request thread. Also move the ~10 clones of decision-log fields
(`evaluate.rs:348-381`) off-thread.

### C3 (P1). `#[instrument]` + per-request OpenTelemetry work on the hot handler
`services/reaper-agent/src/handlers/evaluate.rs:55-64,296-318`
The engine already dropped `#[instrument]` for this reason (`engine/mod.rs:210` comment: "was 200-800ns
per call"), but the handler still carries it and builds span fields with `%`-Display formatting every
request, always calls `span.context()`, and clones `policy_name`/`resource`/`action` into `KeyValue`s
even inside the sampled branch. ~300ns-1µs of a sub-µs budget spent on tracing plumbing.
**Fix:** remove `#[instrument]` from the hot handler; gate all span work behind a cheap
sampled/enabled check.

### C4 (P1). Redundant work per evaluation
`crates/policy-engine/src/engine/mod.rs:230-264`
Two O(n) rule scans per decision (the `matched_rule` loop then `evaluator.evaluate` re-scans the same
rules — see A6), plus `policy_name: policy.name.clone()` — a `String` heap alloc **every** evaluation.
**Fix:** single-pass `evaluate_with_details`; return `policy_name` as `Arc<str>`.

### C5 (P1). Per-request allocations in the handler
`services/reaper-agent/src/handlers/evaluate.rs`
`payload` is owned (`mut`) yet `principal`/`resource`/`action` are `.clone()`d (`:170-173`) instead of
`mem::take`; `policy_ids` is a heap `Vec` even for the single-policy case; `matched_rule` uses
`format!("rule_{}")`; the "evaluate-all" path clones **every** `Arc<EnhancedPolicy>` into a fresh `Vec`
via `list_policies()` (`:146,163`; `engine/mod.rs:191`) then re-looks-up each by id.
**Fix:** `mem::take` owned fields; `SmallVec<[Uuid;1]>` or a scalar fast path; iterate ids without
materializing the Arc vec (cache an id snapshot rebuilt on deploy).

### C6 (P2). Metrics: multiple locked label lookups + unbounded cardinality
`services/reaper-agent/src/observability.rs:48-52`, `evaluate.rs:286-324`
4-6 `with_label_values` calls per request (each a locked map lookup), and `DENIALS_TOTAL` is labeled by
`resource` (and `action`) — effectively unbounded time-series → memory growth + ever-slower label maps.
**Fix:** cache concrete counter/histogram child handles per policy at deploy; drop `resource`/`action`
from `DENIALS_TOTAL`.

### C7 (P2). Default path uses `serde_json` (fully owned), not the SIMD/borrowed path
`evaluate.rs:67` extracts `Json<EvaluateRequest>` → `serde_json` into owned `String`/`HashMap`
(the fast handler's own comment estimates serde_json parse at ~8-10µs, which dwarfs sub-µs eval). Even
`fast_evaluate_policy` allocates an owned `HashMap<String,String>` per request.
**Fix:** make a SIMD/borrowed path the default; introduce a borrowed `PolicyRequest<'a>` (`&str`/`Cow`)
so evaluation needs no request-scoped map/string allocation. Note the fast path currently **drops**
non-string/int/bool context values with `continue` (`evaluate.rs:465-476`) — silent ABAC data loss;
fix as part of this.

### C8 (P2). No release-profile tuning, no fast allocator
Root `Cargo.toml` has **no `[profile.release]`** (LTO off, `codegen-units` default, `panic=unwind`) and
no `mimalloc`/`jemalloc` global allocator (only the eBPF kernel crate sets LTO). For an allocation-heavy
sub-µs target this is free performance.
**Fix:** add `[profile.release] lto = "fat"`, `codegen-units = 1`, `panic = "abort"`; set a
`#[global_allocator]` (mimalloc or jemalloc). Typically 5-20% on this kind of workload.

**Dead weight noted:** `optimized_engine.rs` is never constructed by the agent — it's re-exported but
unused. Either adopt it or stop shipping it as the perf story.

---

## Part D — Benchmark credibility (you cannot claim 10× until this is fixed)

### D1. The marketed harness (`benchmarks/reaper-vs-opa/`) is broken *against OPA*
`bin/deploy-opa.sh:23` loads entity data **flattened** (`.attributes` merged to top level), but every
`.rego` reads `data.entities[...].attributes` (e.g. `rbac.rego:14`, `mega.rego:12`). After flattening
there is no `.attributes` key → every `allow` rule is undefined → OPA falls through to
`default allow := false` and denies everything. The project's own sample output shows **OPA Allow: 0 /
Deny: 2000** vs Reaper Allow: 533 — the engines don't agree on a single decision, yet the harness prints
"Reaper is 2.8× faster." Only 4 hard-coded cases are validated; most scenarios have **no** decision
check. Also: no warm-up, latency recorded as integer **microseconds** (`as_micros()`), and the sample
p99s are **milliseconds** — nothing sub-µs here.
**Fix:** load OPA data in the shape the rego expects; **hard-fail the run unless decisions match on
every scenario**; add warm-up; record nanoseconds.

### D2. The better harness (`services/reaper-bench/`) tilts the headline
It loads OPA data correctly, warms up, and uses HDR histograms properly — good. But the advertised
`avg_speedup` = `reaper_uds.rps / eopa.rps` (`benchmark.rs:818-820`) compares **Reaper over a Unix
domain socket** against **OPA over TCP** (OPA has no UDS path). That's a transport advantage, not an
engine one. Reaper is also hit on the **SIMD `/api/v1/fast-messages`** path while OPA uses its stock
REST API. The fair `reaper_tcp` number is computed but not the one advertised.
**Fix:** headline **Reaper-TCP vs OPA-TCP**; present UDS and the SIMD fast path as separately-labeled
optimizations; measure the identical thing on both sides (both full round-trip *or* both inner-eval).

### D3. The "<1µs" number is an in-process micro-benchmark
`crates/policy-engine/benches/e2e_bench.rs` measures `ReapAstEvaluator::evaluate` in-process, evaluator
reused across iterations (warm), on trivial policies (`allow_all`, `deny_all`, `single_attr`), with **no
OPA baseline**. The two service-level criterion benches (`agent_bench.rs`, `platform_bench.rs`) are
stubs — `b.iter(|| 1 + 1)`.
**Fix:** label the in-process figure honestly as an *evaluator micro-benchmark*, distinct from request
latency; replace the `1+1` stubs with real end-to-end agent benchmarks; keep the compiled path in the
loop (it's the differentiator).

### D4. Compiled tests don't assert compiled-vs-AST parity
`crates/policy-engine/tests/compiled_evaluator_tests.rs` assert against hardcoded expectations and skip
the *hard* policies (comprehensions, nested comprehensions, mega, advanced collection, json,
type-checking are absent). The one place both paths run just `eprintln!`s and never asserts equality.
The comparison/integration harnesses that *do* touch advanced policies use the `Err(_) =>
build_ast_evaluator` fallback, so a compiler regression there would be masked as an AST run and still
pass CI.
**Fix:** add a parity test that runs every shipped policy through **both** compiled and AST over a
matrix of requests and asserts identical decisions; include the advanced policies. This is your
regression guard for the compiled path.

---

## Prioritized roadmap

**Phase 0 — Trust & correctness (must-do before any performance marketing).**
1. A2: default-deny on both endpoints; unify to one eval core. *(small)*
2. A1: fix or formally deprecate `SimplePolicyEvaluator`; stop presenting it as RBAC/ABAC. *(medium)*
3. A3: version/epoch the decision cache and invalidate on deploy + data change (or default it off). *(small-medium)*
4. A4/A5: make `PolicyLanguage::ReaperDsl` first-class so policies rebuild on restart; one consistent
   compile-or-fallback policy across bootstrap/API/CLI. *(medium)*

**Phase 1 — Control-plane security.**
5. B1 tenant isolation → B2/B7 secret handling → B3 OAuth state → B5 stop logging tokens → B4 auth
   rate-limiting → B6 mTLS revocation. *(B1 and B2 are the two that most undermine an authz product.)*

**Phase 2 — Performance to actually reach 10×.**
6. C1 cache rewrite (moka/sharded) — biggest throughput win, pairs with A3.
7. C2 async decision logging off the request thread.
8. C3 drop `#[instrument]`/OTel per-request work.
9. C4/C5 single-pass eval, `Arc<str>` name, `mem::take`, id-snapshot for evaluate-all.
10. C8 release profile + fast allocator (cheap, do early).
11. C7 borrowed/SIMD default path + borrowed `PolicyRequest`.
12. C6 cached metric handles + drop unbounded labels.

**Phase 3 — A benchmark you can publish.**
13. D1 fix OPA data load + hard-gate on decision parity.
14. D2 same-transport headline (TCP vs TCP), UDS/SIMD labeled separately.
15. D3/D4 honest micro-vs-e2e labeling, real service benches, compiled-vs-AST parity test.

**One-line takeaway:** the compiled engine is real and correct — the work is to make the *system around
it* (endpoint semantics, cache, control-plane auth, and the benchmark) trustworthy, then the "10× faster
than OPA" claim becomes something you can prove instead of assert.
