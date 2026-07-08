# Reaper Security Review (Subagent 2 — Offensive Security Engineer)

**Verdict: NOT READY** — multiple independent P0s. Reaper is authorization infrastructure; the flaws below mean an anonymous network attacker can (a) rewrite the active policy on any enforcement node, (b) rewrite/promote policies for any tenant on the control plane, and (c) read every tenant's decision audit. Any one of these fails a bank review outright.

## Executive summary (≤10 lines)
1. **The agent (enforcement node, port 8080) has no authentication on any route** and binds `0.0.0.0` with TLS off by default. Anyone who can reach it can deploy policy bundles, replace entity/relationship data, and dump decision logs. (P0-1)
2. **The direct HTTP bundle-deploy path does no signature verification** — the entire bundle-signing chain of trust (which is otherwise correctly implemented and enforced on the *pull* path) is bypassed by a single unauthenticated POST. (P0-2)
3. **Whole route groups on the control plane — bundles, policies, orgs, teams, billing — take no `RequireAuth` and no tenant check.** Unauthenticated cross-tenant policy CRUD and bundle promotion. Sibling routes (datastore, agents, deployments) *do* enforce auth, proving these are omissions, not a gateway design. (P0-3)
4. Bundle distribution has **no anti-rollback / revocation** — a compromised store/CDN/proxy can replay an old signed bundle. (P1)
5. **Decision logs are not tamper-evident** (no hash chain / write-once), are lossy under load, and can be sampled/disabled — audit completeness and integrity are not defensible to a regulator. (P1)
6. Git policy-source path has **no commit-signature verification and no SSRF guard** (contrast the JWKS path, which has both). (P1)
7. Positives: the signing primitive itself, the JWKS validator, Argon2 password hashing, and the datastore route's tenant guard are all done correctly — which is why the gaps read as missing wiring rather than absent knowledge.

## Threat model (assets → actors → boundaries)
**Assets:** policy bundles (`.rbb`), entity/relationship data (ReBAC graph = crown jewels), decision/audit logs, bundle signing keys, JWT/HMAC secrets, git & DB credentials, tenant boundaries.
**Actors:** anonymous network attacker, malicious/curious tenant, hostile insider (org member), compromised reaper node, compromised CI, compromised bundle store/CDN, on-path (post-TLS) attacker.
**Trust boundaries:** client→control plane, control plane→data plane (agents), bundle store/CDN→agent, git→control plane, agent→audit sink.
Findings below are mapped to the boundary they break.

## Findings table
| ID | Sev | Location | Finding | Impact | Recommendation |
|----|-----|----------|---------|--------|----------------|
| P0-1 | P0 | `reaper-agent/src/main.rs:490-539`; handlers/* (no `RequireAuth`); `config/settings.rs:51-52,568` | Agent exposes deploy/data/decision routes with **zero authentication**, bound `0.0.0.0`, TLS+mTLS off by default | Anyone routable to the agent controls the live authorization decision, rewrites relationship data, and reads all audit logs | Require authN (mTLS client cert or bearer) on all non-health agent routes; default-deny; bind loopback/UDS unless explicitly configured |
| P0-2 | P0 | `reaper-agent/src/handlers/policies.rs:308-383` (`deploy_bundle`), `:395` (`load_bundles_atomic`) | Direct HTTP bundle/policy deploy **never calls `verify_bundle`** — parses bytes and hot-swaps | Signing chain fully bypassed; attacker-supplied unsigned policy becomes active | Verify signature (reuse `verify_bundle_download`) on *every* load entry point, not only the sync-pull path; fail closed |
| P0-3 | P0 | `api/bundles.rs:40-77` (no `RequireAuth`), `api/policies.rs:26-49,132`, `api/orgs.rs`, `api/teams.rs`, `api/billing.rs`; no global auth layer `main.rs:219-242` | Control-plane bundle/policy/org/team/billing handlers have **no authN, no scope check, no tenant binding** | Unauthenticated cross-tenant policy CRUD + bundle **promotion** (which broadcasts to agents); tenant isolation absent | Add `RequireAuth` + `RequireScope` to every mutation handler; enforce `user.org_id == org.id` (the pattern already in `datastore.rs:118`) |
| P0-3b | P0 | `api/bundles.rs:108-148` etc. | Bundle-id operations discard org (`_org_id`) and act on a **global UUID** | IDOR: read/update/delete/promote any bundle in any org by guessing/knowing its UUID | Scope every `bundle_id` query by `org_id`; return 404 on mismatch |
| P1-1 | P1 | `management/sync.rs:488-537`, `client.rs:405-432`; `bundle_signing.rs:75-86` | **No anti-rollback / revocation**: agent accepts any validly-signed bundle; dedupe is by id/checksum only; envelope has no version/expiry/revocation | Compromised store/CDN/on-path attacker replays an old signed-but-revoked policy (explicitly in the module's own threat model) | Add monotonic version + not-before/expiry to the signed envelope; agent refuses non-increasing versions; support key/bundle revocation lists |
| P1-2 | P1 | `decision_log.rs:11-96`, `decision_buffer.rs`, config `sample_allow_rate`/`log_allows`/`log_denies` | Decision logs **not tamper-evident** (no hash chain/HMAC/write-once), lossy ring buffer, sampling/disable can drop records, wall-clock timestamp only | Insider or node compromise can alter/delete audit; dropped decisions are undetectable — fails "prove decision X at time T" | Hash-chain entries (prev-hash + periodic signed checkpoint); make lossy drops counted+alarmed; document that sampling is incompatible with audit mode |
| P1-3 | P1 | `sync/git.rs:105-134` | Git source: **no commit-signature (GPG) verification, no SSRF URL guard, plaintext userpass creds** | Compromised git push or internal-URL SSRF yields attacker-controlled policy source; confused deputy across tenant repos | Verify signed commits/tags; apply the `jwks.rs` SSRF guard to git URLs; store creds encrypted/per-tenant |
| P2-1 | P2 | `reap/parser/*`, `reap/ast_evaluator/expr_eval.rs:18` | DSL parser/evaluator recursion is **not depth-bounded**; deep nesting can stack-overflow (abort) | Pathological policy crashes eval process (host-availability incident in a sidecar); amplified by P0-2 | Add a compile-time nesting-depth limit; the language should be total/terminating by construction |
| P2-2 | P2 | `.github/workflows/` (searched) | **No `cargo audit`/`cargo deny`/SBOM/fuzz** in CI; Trivy scan is `continue-on-error` | Vulnerable/backdoored deps ship undetected | Add `cargo-deny` (advisories+licenses+bans) as a blocking gate; generate SBOM; make image scan blocking |
| P3-1 | P3 | `auth/api_key.rs:82-86` | API keys stored as **unsalted SHA-256** | Acceptable for 256-bit random keys, but no pepper/HSM; DB read = offline correlation only | Add a server-side pepper (HMAC) so a DB leak alone can't confirm guessed keys |
| P3-2 | P3 | `config/settings.rs:521,568` | mTLS / client-cert **off by default** | Weak default posture for the control-plane↔agent boundary | Ship a hardened default profile with mTLS on |

## Detailed findings

### P0-1 — Agent enforcement plane is entirely unauthenticated (boundary: control plane→data plane, network→agent)
`reaper-agent/src/main.rs:490-539` builds the full router and applies exactly one `.layer()` — `DefaultBodyLimit` — before `.with_state`. There is **no auth middleware and no `RequireAuth` extractor** anywhere in the agent (grep over `handlers/` for `RequireAuth|api_key|Authorization|Bearer` returns only comments). The exposed mutating routes include:
- `POST /api/v1/bundles/deploy`, `/api/v1/bundles/load` — replace the active policy set;
- `POST /api/v1/policies/deploy`, `/policies/compile`;
- `POST /api/v1/entities`, `DELETE /api/v1/entities/{type}/{id}`, `POST /api/v1/entities/batch`, `POST /api/v1/data`, `/data/sync`, `/data/apply-deltas` — rewrite the ReBAC/entity graph the policies read;
- `GET /api/v1/decisions`, `POST /api/v1/decisions/export` — dump the full audit buffer.

Defaults make this internet-facing out of the box: `config/settings.rs:51-52` sets `bind_address = "0.0.0.0"`, and TLS/`require_client_cert` default to disabled (`settings.rs:568` asserts `!require_client_cert`). Because the same router is served on both TCP and UDS (`main.rs:542-568`), the TCP listener carries the full unauthenticated surface.

**Why P0:** the agent *is* the enforcement point. Unauthenticated write to its policy/data plane means the attacker directly decides `allow`/`deny` for every application trusting this agent, and directly edits the relationship graph — incorrect authorization by design.

**Remediation:** default-deny all non-health routes; require mTLS client cert (the management side already models `AuthMethod::Mtls`) or a bearer token minted at registration; bind to loopback/UDS unless an operator opts into a network bind with auth configured.

### P0-2 — Direct bundle-deploy path performs no signature verification (boundary: bundle store/CDN→agent; network→agent)
The pull path is correct and fail-closed: `management/sync.rs:125-132,318-319,517-518` calls `verify_download` → `verify_bundle_download` (`sync.rs:548-588`) → `reaper_core::bundle_signing::verify_bundle` before hot-swap, with a sensible policy matrix and `require_signed_bundles` defaulting to **`true`** (`config/settings.rs:350`). That is good work.

But `deploy_bundle` (`handlers/policies.rs:308-383`) is a **separate entry point** that takes `Json<DeployBundleRequest>`, calls `PolicyBundle::from_bytes(&payload.bundle)` and immediately `deploy_bundle_with_store(...)`. There is no `BundleSignature`, no `verify_bundle`, no `require_signed_bundles` check. `load_bundles_atomic` (`:395`) is the same. So the signing infrastructure protects the transport the agent *pulls* over, while an attacker simply *pushes* an unsigned bundle to the same process and it is trusted. Combined with P0-1 (no auth), this is a one-request full policy takeover.

**Remediation:** route all in-process loads through one verification function; require a valid signature envelope on the deploy request (header or body) and honor `require_signed_bundles`.

### P0-3 — Control-plane mutation routes lack authN/authZ/tenant isolation (boundary: client→control plane; tenant→tenant)
`main.rs:219-242` shows the only middleware layers are `security_headers`, `correlation_id`, `request_metrics`, `body_size_limit`, `access_log`, `TraceLayer`, and optional rate-limiting — **no authentication layer**. Auth is therefore per-handler via the `RequireAuth` extractor. Grep of `RequireAuth` usage:
- **Present:** `agents.rs`, `datastore.rs`, `decisions.rs`, `events.rs`, `landscape.rs`, `namespaces.rs`, `sources.rs`, `deployments/*`.
- **Absent:** `bundles.rs`, `policies.rs`, `orgs.rs`, `teams.rs`, `billing.rs`.

Confirmed by reading handlers: `bundles.rs:98-106` `create_bundle`, `:119-148` update/delete, `:60-63` `promote_bundle`, and `policies.rs:132` `create_policy` take only `State`, `Path`, `Json` — no `RequireAuth`, no `RequireScope`, no identity read at all. `parse_org_id`/`resolve_org` merely resolve a slug to a UUID; they do not authenticate the caller or bind them to the org. So an anonymous client can:
- create/modify/delete/**promote** bundles (promotion broadcasts `BundlePromoted` over SSE and drives agents to pull) for any org;
- create/modify/delete policies in any org;
- enumerate every tenant's policies/bundles (cross-tenant disclosure);
- create orgs, manage teams, initiate billing.

That the sibling `datastore.rs:100-119` does it correctly (scope check *and* `if user.org_id != organization.id && !Admin → Forbidden`) proves the pattern is understood and simply not applied here.

**P0-3b (IDOR):** even the org resolution is cosmetic for id-addressed operations — `bundles.rs:113,124,145` bind `_org_id` and then call `bundle_service.get/update/delete(bundle_id)` on the **global** UUID, so any bundle is reachable by id regardless of the `{org}` in the path.

**Remediation:** add `RequireAuth` + `RequireScope` (`BundleWrite`/`BundlePromote`/`PolicyWrite`/…) to every mutation handler in these files; enforce `user.org_id == resolved.org_id` (or platform `Admin`); scope every id-addressed repository call by `org_id`.

### P1-1 — No anti-rollback or revocation in bundle distribution (boundary: bundle store/CDN→agent)
`bundle_signing.rs` binds authenticity + integrity + optional `key_id`, but the `BundleSignature` envelope (`:75-86`) carries **no version, no not-before/expiry, no revocation**. The agent's freshness logic (`client.rs:405-432` `check_for_update`) only *dedupes* by `current_bundle_id`/checksum; `sync.rs:508-515` only re-checks the checksum it was told to expect. Nothing rejects an *older* validly-signed bundle. The module's own doc comment claims the design defends against "a compromised bundle store, CDN, or a proxy past TLS termination" — but such an actor can serve a previously-valid (now-revoked) bundle and the agent accepts it. There is also no key-revocation path: rotating `key_id` does not invalidate bundles signed by a leaked old key still trusted by a pinned config.

**Remediation:** include a monotonic version and validity window in the signed bytes; agent persists the highest applied version and refuses regressions; ship a revocation list (by bundle hash and by key_id) checked at load.

### P1-2 — Audit logs are not tamper-evident, complete, or trustworthy under scrutiny (boundary: agent→audit sink; insider)
`DecisionLogEntry` (`decision_log.rs:11-82`) has good *attribution* (policy_id, `policy_version`, `data_checksum`, principal, decision_id, `evaluation_time_ns`) — enough to answer "which policy version decided this" *if the record exists and is honest*. But:
- **Not tamper-evident:** no hash chain, no per-entry HMAC/signature, no write-once. `decision_buffer.rs` is an in-memory ring; nothing detects deletion or mutation of NDJSON on disk/in ClickHouse.
- **Lossy:** a bounded ring buffer drops oldest entries under load; a burst can silently erase decisions.
- **Incompletable by config:** `sample_allow_rate`, `log_allows`, `log_denies`, `enabled` mean a decision can legitimately produce **no** record. This is explicit, but there is no "audit mode" that makes completeness mandatory.
- **Timestamp source:** `chrono::Utc::now().to_rfc3339()` (`decision_log.rs:96`) — wall clock, unattested, no monotonic pairing; clock skew/rollback is not detectable in-record.

**Remediation:** hash-chain entries (`prev_hash`) with periodic signed checkpoints exported to the sink; count and alarm on buffer drops; provide a mandatory-audit mode incompatible with sampling; record a monotonic counter alongside wall-clock time.

### P1-3 — Git policy source: no commit verification, no SSRF guard (boundary: git→control plane; tenant→tenant)
`sync/git.rs:105-134` authenticates with `git2::Cred::userpass_plaintext(username, password)` from config and clones/fetches. Unlike `jwks.rs` (which has a thorough `is_disallowed_ip`/`validate_jwks_url` SSRF guard, `jwks.rs:17-80`), there is **no URL validation** on the git remote — an attacker-configured `http://169.254.169.254/...` or internal host is fetched. There is **no commit or tag signature verification**, so whatever a branch tip contains becomes policy source. In a multi-tenant control plane pulling many tenants' repos with stored credentials, this is a confused-deputy and SSRF surface.

**Remediation:** require signed commits/tags with a per-source trusted-key set; apply the JWKS SSRF guard to git URLs; store credentials encrypted and scoped per tenant.

## Absence checks performed (falsifiable)
- **Global auth middleware on control plane:** read `main.rs:206-243` in full — the layer stack is enumerated above; no auth layer exists. Searched `middleware.rs` and `auth/middleware.rs` for `RequireAuth|authenticate|api_key` — auth lives only in the per-handler `RequireAuth` extractor.
- **Agent authentication:** grepped `reaper-agent/src/handlers/` and `main.rs` for `RequireAuth|api_key|Authorization|Bearer|verify_bundle` — only comments; no auth and no signature check on `deploy_bundle`/`load_bundles_atomic` (read `policies.rs:308-395`).
- **JWT alg confusion:** `jwt.rs:101-110` uses `Validation::default()` (HS256-locked, exp enforced) with issuer+audience set; `jwks.rs:234-251` builds `Validation::new(key.algorithm())` from the JWK's key type (RSA/EC decoding key) and **requires** an audience (`:241-246`) — no HMAC/`alg:none` confusion path found.
- **DSL depth limit:** searched `reap/parser/*` and `reap/ast_evaluator/*` for `depth|MAX_DEPTH|recursion|stacker` — only ReBAC `max_depth` (a traversal arg), no parser/expression nesting bound. ReBAC graph traversal *is* bounded (`TRAVERSAL_NODE_BUDGET=4096`, per repo map).
- **Network-reachable panics in eval hot path:** `evaluate.rs` unwraps at `:581` (guarded by `is_i64()`), `:803/:963/:964` (tests). No unguarded unwrap on request-derived data found in the hot path; the residual DoS risk is stack overflow via unbounded recursion (P2-1), not an unwrap.
- **`unsafe`:** 12 blocks, all in `reaper-ebpf` (experimental, Linux-only, off the eval hot path); the lone hit in `policy-engine/src/data/interning.rs:27` is a doc comment, not an `unsafe` block. eBPF blocks were **not** individually audited for soundness — flag for review before that feature ships.
- **Secrets in repo:** grep for hardcoded `password|secret|api_key|private_key = "..."` in non-test src found only `auth/users/password.rs:79` (a test fixture). Passwords hashed with Argon2 + `OsRng` salt (`password.rs:1-16`) — correct.
- **Dependency scanning:** confirmed no `cargo audit`/`deny`/fuzz in `.github/workflows/` (matches repo map).

## Coverage note
Prioritized per the ground rules: eval hot path → API surface → distribution/promotion → audit → data plane. I did **not** deep-audit: individual eBPF `unsafe` soundness; the OAuth/GitHub-callback CSRF/state handling; rate-limiter bypass; the full 140-handler control-plane surface line-by-line (I confirmed the per-file `RequireAuth` presence/absence and read representative handlers); SQL parameterization was spot-checked (sqlx bind params used in `api_key.rs`/`jwks.rs`) but not exhaustively across all repositories.

## What's done well (≤5)
1. The signing primitive (`bundle_signing.rs`) is solid: two independent fail-closed checks, constant-time digest compare, algorithm+`key_id` pinning, no `alg:none`, good tests.
2. The bundle **pull** path enforces verification fail-closed with a sensible matrix and a secure `require_signed_bundles = true` default.
3. `datastore.rs` demonstrates the correct authZ+tenant pattern (scope check plus `user.org_id != org.id → Forbidden`) — the template the unprotected routes should follow.
4. JWKS validation is careful: mandatory audience, SSRF guard with cloud-metadata/CGNAT/IPv6 coverage, RSA/EC-only decoding keys.
5. Passwords use Argon2 with per-hash random salt; API keys are high-entropy and stored hashed.
