# GitOps / Policy-as-Code

> **STATUS: ✅ SHIPPED** — landed via PRs #35–#37 (2026-07-12) across phases A–C.
> A: the `SyncService` is spawned at boot and its manual trigger is wired to
> the real engine; a successful git sync now materializes policy rows + a
> commit-SHA-keyed bundle (idempotent per SHA), and the JWKS SSRF guard is
> promoted to a shared module the git clone/fetch path calls before any
> network I/O. B: the git link is reshaped from personal-OAuth-token-in-URL to
> a GitHub App install minting short-lived installation tokens per sync,
> HEAD-commit SSH-signature verification (fail-closed on unsigned/untrusted),
> and HMAC-verified `POST /webhooks/git/{github,gitlab}` push. C: a drift
> endpoint reports git HEAD vs deployed policies, and a per-source
> `conflict_mode` (commit_back default / read_only / last_writer_wins) makes
> "git is the source of truth" enforceable — UI edits become commits.
> Closes Product F2/F3 and Security P1-3 in full.

**Readiness gate:** NOT READY → CONDITIONAL (restores a named product pillar; closes a P1 security finding on the git ingest path)
**Priority:** P1 (F2 wiring is S and unblocks the "as code" claim this week; F3 reshape + P1-3 hardening is M)
**Findings closed:** Product **F2** (sync engine never wired), Product **F3** (git link wrong shape: user-token / broad `repo` scope / poll-only / no App / no webhook / no GitLab), Security **P1-3** (no commit-signature verification, no SSRF guard, plaintext creds on the git source path)

---

## 1. Goal

Make "manage policy as code" real and enterprise-safe:

1. **Run the sync engine in production.** The existing `SyncService` (`services/reaper-management/src/sync/service.rs`) must be spawned by `main.rs` and reachable from the manual-trigger API, and its output must actually **materialize into policies + a compiled bundle** (today `sync_source` only flips status and counts files — it never persists policy content).
2. **Reshape the git link** from personal-OAuth-token/poll-only into a **GitHub App install + webhook push** model with fine-grained, org-approved, per-repo permissions; add a **GitLab** handler behind the config that already exists.
3. **Harden the ingest path** (Security P1-3): apply the JWKS-style SSRF guard to git remote URLs, require signed commits/tags, and stop embedding long-lived tokens in clone URLs.
4. **Define the git↔UI conflict model.** Recommend **"UI writes become commits; git is the single source of truth"** so drift is structurally impossible and regulated buyers get PR-based approval for free.
5. **Detect and surface drift** (git HEAD vs deployed bundle) as a first-class, queryable state.

This reuses mini-design **D-B** in `reviews/04-product-architecture.md:143-153`.

---

## 2. Current state (evidence) — file:line

- **Engine is compiled-but-dead.** `SyncService::new` is referenced only inside `#[cfg(test)]` at `sync/service.rs:333`. The public API (`start` `sync/service.rs:121`, `sync_source` `:185`, `trigger_sync` `:267`) exists and is well-factored but nothing constructs it outside tests.
- **`main.rs` never spawns it.** The only background task spawned is the change-log retention sweeper (`services/reaper-management/src/main.rs:144-173`). There is no `SyncService::start` spawn, and `AppState` (`state.rs:220-287`) has no `sync_service` field — only `event_tx: broadcast::Sender<ServerEvent>` (`state.rs:230`).
- **Manual trigger is a no-op.** `trigger_sync` handler (`api/sources.rs:335-389`) authenticates and scopes correctly, then only calls `update_sync_status(.., Syncing, ..)` and returns a placeholder — `// TODO: Actually trigger the sync via SyncService` at `api/sources.rs:381`. The real `SyncService::trigger_sync` (`sync/service.rs:267`) is never invoked.
- **Even when run, sync does not materialize policy.** `SyncService::sync_source` (`sync/service.rs:185-264`) dispatches to `GitSyncer::sync` (`sync/git.rs:39-78`), which clones/checks-out and returns a `SyncResult` carrying only `policies_found` **count** — it reads file contents into `PolicyFile` (`sync/git.rs:187-199`) but the service **discards them**. Nothing calls `PolicyRepository::create` (`db/repositories/policy.rs:23`) or `BundleService::compile` (`bundle/service.rs:216`). GitOps ingest has no path into the deploy pipeline.
- **Git link is personal-OAuth-token shaped.** `github_authorize` requests `scope=repo` (broad read/write to all the user's repos) at `api/oauth/github.rs:70`; the callback stores the **individual user's** token (`api/oauth/github.rs:159-203`); `create_source_from_github` embeds that token directly in the clone URL — `https://x-access-token:{token}@github.com/...` at `api/oauth/github.rs:302-305` — and hardcodes `poll_interval_seconds: 300` (`api/oauth/github.rs:320`). No webhook route exists.
- **No GitHub App, no GitLab.** `config/oauth.rs` defines `GitLabOAuthConfig` (`config/oauth.rs:25-31`) and `BitbucketOAuthConfig` (`:33-38`), but `api/oauth/` contains only `github.rs` — config surface with no handler.
- **Ingest path is unguarded (Security P1-3).** `GitSyncer::clone_or_open` authenticates with `git2::Cred::userpass_plaintext` (`sync/git.rs:104-108`, and again in `update_repo` `sync/git.rs:131-135`) against `config.url` with **no URL validation** — an attacker-set `http://169.254.169.254/...` or internal host is fetched. There is **no commit/tag signature verification**: whatever the branch tip contains at `sync/git.rs:144-159` becomes policy source. Contrast the JWKS path, which has a complete SSRF guard: `is_disallowed_ip` and `validate_jwks_url` in `auth/jwks.rs:17-80` (requires HTTPS, resolves host, rejects loopback/private/link-local/CGNAT/IPv6-ULA incl. cloud metadata).
- **Credentials in the clear.** `GitConfig.password`/`username` are plaintext in the source `config` JSON (`domain/source.rs:109-114`); the token-in-URL means revoking the user's PAT orphans every source.

---

## 3. Definition of Done — testable checkboxes

- [ ] On management boot, a `SyncService` background loop is running: with a due git source configured, `sync_due_sources` fires within `check_interval_secs` and the source's `last_sync_commit` advances (assert via `GET /orgs/{org}/sources/{id}`).
- [ ] `POST /orgs/{org}/sources/{id}/sync` performs a **real** sync (no TODO), returning `policies_found` and `commit` populated from the actual repo, and 409s if a sync is already in flight.
- [ ] A successful git sync **materializes**: policy rows are upserted (`PolicyRepository`) and a bundle is compiled (`BundleService::compile`) keyed by the commit SHA; re-running the same SHA is idempotent (no duplicate policies, no second bundle).
- [ ] `POST /webhooks/git/github` and `POST /webhooks/git/gitlab` verify the provider HMAC signature and enqueue a sync for the matching source; an unsigned/invalid-signature webhook returns 401 and does not sync.
- [ ] Git remote URLs are validated with the SSRF guard before any clone/fetch: an https public URL passes; `http://`, a loopback/private/link-local/metadata IP, or a non-resolving host is rejected with a clear error and no network fetch (unit-testable against the shared guard).
- [ ] With `require_signed_commits = true` on a source, a sync of an unsigned/untrusted-key HEAD commit **fails closed** (status `Failed`, reason "unsigned commit"); a commit signed by a configured trusted key succeeds.
- [ ] GitHub App install flow stores an **installation id** (not a user PAT); clone auth uses a short-lived installation token minted at sync time, not a token embedded in the stored URL.
- [ ] A GitLab source can be connected and synced (parity with GitHub for clone + webhook).
- [ ] `GET /orgs/{org}/sources/{id}/drift` returns `{in_sync | drift, added/changed/removed[]}` computed from git HEAD vs the last-materialized bundle.
- [ ] A UI/API policy edit under a git-backed source produces a **commit** on a branch (conflict model): after the edit, git and deployed state are reconciled (assert a new commit SHA appears and drift returns to `in_sync`).
- [ ] All new sync/webhook/App actions emit `audit_log` entries (`audit/actions`) and reuse the existing `RequireAuth` + org-scope pattern already present in `api/sources.rs`.

---

## 4. Critical steps — ordered

Each step: **what / where(files) / verify / schema**.

### Step 1 — Spawn `SyncService` and hold it in `AppState` (Phase A, S)
- **What:** Construct one `SyncService::with_event_tx(db, SyncConfig, event_tx)` at startup, store an `Arc<SyncService>` on `AppState`, and `tokio::spawn` its `start()` loop next to the existing sweeper.
- **Where:** `state.rs:220-287` (add `pub sync_service: Arc<SyncService>` beside `event_tx`); `main.rs:144-173` (add a spawn block mirroring the sweeper); `sync/service.rs:103-111` (`with_event_tx` already exists and takes the broadcaster).
- **Verify:** log line "Starting sync service" (`sync/service.rs:130`) appears on boot; a source with `sync_interval_secs>0` transitions Pending→Syncing→Success without any API call.
- **Schema:** none (uses existing `sources` table / `PolicySourceRepository`).

### Step 2 — Make sync actually materialize policies + a bundle (Phase A, M)
- **What:** Extend `sync_source` so that on `Ok(SyncResult)` it (a) reads `PolicyFile`s via `GitSyncer::get_policy_files` (`sync/git.rs:218`), (b) upserts them through `PolicyRepository` (`db/repositories/policy.rs:23`), and (c) compiles a bundle via `BundleService` (`bundle/service.rs:105` create, `:216` compile), tagging the bundle with the commit SHA for idempotency. This closes the silent gap where sync counts files but never persists them.
- **Where:** `sync/service.rs:185-264` (materialization step after status update); new helper `materialize_git(source, files, commit)`.
- **Verify:** after a sync, `GET /orgs/{org}/policies` lists the repo's policies and a bundle exists in `bundles` with `source_commit = <sha>`; a second sync at the same SHA creates no new bundle (idempotent).
- **Schema:** migration **010_source_materialization.sql** — add `source_id UUID NULL` and `source_commit TEXT NULL` to `bundles` (link a bundle to the source+SHA that produced it); index `(source_id, source_commit)` for the idempotency check.

### Step 3 — Wire the manual trigger to the real engine (Phase A, S)
- **What:** Replace the placeholder in `trigger_sync` with `state.sync_service.trigger_sync(source_id).await`, mapping `SyncError::CannotSync`→409, `NotFound`→404, others→500; return the real `SyncResult`.
- **Where:** `api/sources.rs:374-388` (remove the `// TODO` and the bare `update_sync_status`), reuse `SyncService::trigger_sync` (`sync/service.rs:267-283`).
- **Verify:** `POST /orgs/{org}/sources/{id}/sync` returns `policies_found`/`commit` from the actual repo; concurrent trigger returns 409.
- **Schema:** none.

### Step 4 — SSRF guard + credential hardening on the git path (Phase B, S) — closes P1-3 (part 1)
- **What:** Promote the JWKS guard (`auth/jwks.rs:17-80`, `is_disallowed_ip` + `validate_jwks_url`) into a shared module (e.g. `auth/ssrf.rs` or `sync/url_guard.rs`) and call it before every clone/fetch in `GitSyncer`. Require the remote scheme to be `https` (or `ssh` with a key, not plaintext userpass). Stop persisting tokens in `config.url`.
- **Where:** new shared guard reused by `auth/jwks.rs`; `sync/git.rs:86-119` (guard before `builder.clone`), `sync/git.rs:122-142` (guard before `remote.fetch`); `api/oauth/github.rs:302-305` (do not embed token in stored URL — store repo full-name + installation ref instead).
- **Verify:** unit test the guard for https-public (pass) vs http / loopback / `169.254.169.254` / private / non-resolving (reject); a source pointed at an internal URL fails with "resolves to a disallowed internal address" and does no fetch.
- **Schema:** none (guard is code); credential move handled in Step 6.

### Step 5 — Require signed commits/tags (Phase B, M) — closes P1-3 (part 2)
- **What:** Add a per-source `require_signed_commits: bool` and a `trusted_signing_keys: Vec<String>` (GPG/SSH allowed-signers). After checkout (`sync/git.rs:144-159`), verify the HEAD commit (or the resolved tag) signature against the trusted key set using `git2` signature extraction; fail the sync closed on missing/untrusted signature.
- **Where:** `domain/source.rs:96-118` (`GitConfig` gains the two fields, `#[serde(default)]`); `sync/git.rs` (new `verify_commit_signature` called in `update_repo`); `sync/service.rs:185-264` (surface verification failure as `SyncStatus::Failed` with reason).
- **Verify:** BDD: unsigned HEAD with the flag on → status `Failed`, reason "unsigned commit"; commit signed by a trusted key → `Success`.
- **Schema:** none (stored inside the `sources.config` JSON blob).

### Step 6 — GitHub App install (fine-grained perms) replacing personal OAuth (Phase B, M) — closes F3 (part 1)
- **What:** Add an App install flow: `GET /orgs/{org}/git/github/install` → GitHub App install URL; callback stores the **installation id**. At sync time, mint a short-lived installation access token via the App JWT and use it for that clone only. Narrow `scope=repo` to the App's fine-grained, per-repo contents:read permission.
- **Where:** `api/oauth/github.rs:28-77` (add install-start alongside `github_authorize`; keep OAuth path for read-only repo listing but stop using its token for cloning); `api/oauth/github.rs:105-224` (callback stores installation, not just user token); `config/oauth.rs:13-19` (`GitHubOAuthConfig` gains `app_id`, `private_key`, `webhook_secret`).
- **Verify:** connecting a repo stores `provider='github'` with an `installation_id`; revoking the connecting user's PAT does **not** break sync (installation token still mints).
- **Schema:** migration **011_git_providers.sql** — add to `sources`: `provider TEXT` ('github'|'gitlab'|'bitbucket'), `installation_id TEXT NULL`, `env_mapping JSONB NULL` (branch/dir → namespace), `last_synced_sha TEXT NULL`; new `source_syncs` history table `(id, source_id, commit_sha, status, policies_found, bundle_id, started_at, finished_at, error)` for an auditable sync trail (mini-design D-B, `reviews/04:152`).

### Step 7 — Webhook push endpoints + GitLab handler (Phase B, M) — closes F3 (part 2)
- **What:** Add `POST /webhooks/git/{provider}` that verifies the provider HMAC (`X-Hub-Signature-256` for GitHub; `X-Gitlab-Token` for GitLab) against the per-source `webhook_secret`, resolves the target source by repo identity, and calls `sync_service.trigger_sync`. Add `api/oauth/gitlab.rs` mirroring `github.rs` (authorize/callback/list-repos/create-source) behind the existing `GitLabOAuthConfig` (`config/oauth.rs:25-31`). Reconciliation loop (Step 1) remains as the fallback when webhooks are missed.
- **Where:** new `api/webhooks/git.rs` (public route, signature-authenticated not `RequireAuth`); new `api/oauth/gitlab.rs`; register in `api/mod.rs` router build.
- **Verify:** a signed GitHub `push` webhook triggers a sync within ~1s; an unsigned/forged webhook → 401 with no sync; a GitLab source connects and syncs.
- **Schema:** reuses `sources.webhook_secret` (stored in config JSON) and `source_syncs` from Step 6.

### Step 8 — Drift detection endpoint (Phase C, M) — closes F3 (part 3)
- **What:** `GET /orgs/{org}/sources/{id}/drift` computes the diff of git HEAD's materialized policy set vs the currently-deployed bundle for the source's target namespace(s): `{status: in_sync|drift, added[], changed[], removed[]}`. Surface drift on the existing SSE stream (`event_tx` / `ServerEvent`) so the landscape view reflects it.
- **Where:** new handler in `api/sources.rs`; diff util comparing `PolicyFile` set at HEAD (`GitSyncer::get_policy_files`) against policies linked to the deployed bundle; emit `ServerEvent::DriftDetected` (add variant in `state.rs`).
- **Verify:** edit a policy out-of-band (deploy without git) → drift endpoint reports the delta; re-sync → returns `in_sync`.
- **Schema:** none beyond `last_synced_sha` (Step 6); drift is computed, not stored.

### Step 9 — Conflict model: UI writes become commits (Phase C, L) — the ADR mechanism
- **What:** For git-backed sources, route UI/API policy mutations through a **commit-back** path: the write opens a branch on the source repo via the GitHub/GitLab App, commits the changed policy file, and (configurable) opens a PR or fast-forwards the tracked branch. Deployed state is then reconciled by the normal sync path, so there is exactly one lineage and drift is structurally impossible. A source flag selects `commit_back` (recommended default for git-backed policies) vs `read_only` (git is authoritative; UI edits rejected) vs `last_writer_wins` (UI edits deployed directly + drift alert — discouraged).
- **Where:** policy update handlers in `api/policies.rs` gain a "is this policy git-backed?" branch that delegates to a new `sync::commit_back(source, policy, actor)`; uses the App installation token (Step 6).
- **Verify:** editing a git-backed policy in the UI produces a commit on the repo (assert new SHA) and, after reconcile, drift returns `in_sync`; with `read_only`, the same edit returns 409 "policy is managed by git source".
- **Schema:** `sources.config` gains `conflict_mode` ('commit_back'|'read_only'|'last_writer_wins', default 'commit_back'); reuse `audit_log` for the commit-back action.

---

## 5. Dependencies

- **Auth P0s must land first or in parallel** (Synthesis P0-1/P0-3): the webhook endpoints are public-by-signature, so they must not become an unauthenticated ingress; the git-source CRUD already uses `RequireAuth` (`api/sources.rs`) — keep that. The commit-back path (Step 9) depends on the App install (Step 6).
- **Shared SSRF guard** (Step 4) requires factoring `auth/jwks.rs:17-80` into a reusable module without regressing the JWKS caller.
- **Bundle compile + policy upsert pipeline** (`bundle/service.rs`, `db/repositories/policy.rs`) must accept a batch of `PolicyFile`s — Step 2 may need a small `materialize` API on `BundleService`.
- **GitHub App registration** is an external/ops prerequisite (App id + private key + webhook secret in `config.oauth.github`).
- **DB migrations 010/011** must run before the materialization and provider code paths activate.
- Signed-commit verification (Step 5) depends on `git2` signature APIs and an agreed key-distribution format (GPG armored / SSH allowed-signers).

---

## 6. Testing & verification

- **Unit:** SSRF guard truth table (reuse/extend the JWKS guard tests); commit-signature verify (signed/unsigned/untrusted-key); webhook HMAC verify (valid/invalid/replayed); idempotent materialization keyed by SHA.
- **Integration (management, real DB):** spawn `SyncService`, create a git source pointing at a local fixture repo (served over the loopback exception only in tests), assert Pending→Success, policies upserted, bundle compiled with `source_commit`.
- **BDD (`tests/features`):** "as-code sync" feature — connect repo → push commit → webhook → policies materialized → rollout; "signed-commits gate"; "drift detection"; "commit-back conflict model".
- **E2E (`tests/e2e`):** management sync → bundle compile → agent pull (`services/reaper-agent/src/management/sync.rs`) → decision reflects the git-sourced policy. Confirms the full "as code → enforcement" loop the review says is currently broken.
- **Security regression:** assert a source pointed at `http://169.254.169.254/latest/meta-data/` never issues a request; assert token is no longer present in stored `sources.config.url`.
- **Negative:** forged webhook → 401; unsigned commit with gate on → `Failed`; concurrent `trigger_sync` → 409.

---

## 7. Effort & phasing — S/M/L

- **Phase A — Run it (Priority: this week).** Step 1 (S), Step 2 (M), Step 3 (S). Restores the "policy as code" claim end-to-end with the existing engine. **~M total.**
- **Phase B — Harden + reshape (Priority: P1).** Step 4 (S), Step 5 (M), Step 6 (M), Step 7 (M). Closes Security P1-3 and Product F3 (App + webhook + GitLab). **~L total.**
- **Phase C — Bidirectional governance (Priority: P1→P2).** Step 8 (M), Step 9 (L). Drift + commit-back conflict model. **~L total.**

Fastest credible slice that changes the verdict on F2: **Phase A only** (the engine runs and materializes). P1-3 requires at least Step 4 (SSRF) + Step 5 (signed commits).

---

## 8. Key decisions (ADR-style)

### ADR-1 — BYO repo vs Reaper-managed repo
- **Context:** Enterprises want policy in *their* VCS with *their* review/CODEOWNERS; smaller teams want zero setup.
- **Decision:** **Default = BYO repo via GitHub/GitLab App** (fine-grained, org-approved, per-repo). **Option = Reaper-managed repo** (Reaper provisions and owns a repo per org for teams without one), same sync/commit-back mechanics behind the scenes.
- **Why default BYO:** it is the shape regulated buyers require (repo-access review passes an App install, not a personal PAT); it reuses their existing branch-protection/PR approvals. Managed-repo stays an onboarding convenience, not the primary path.
- **Rejected:** personal-OAuth-PAT-in-clone-URL (today's shape, `api/oauth/github.rs:70,302`) — dies when the user leaves, broad `repo` scope, orphans sources on revocation.

### ADR-2 — Reconciliation loop vs webhook push
- **Decision:** **Both, converging on one idempotent step.** Webhook push (Step 7) is the low-latency primary; the reconciliation loop (Step 1, default `check_interval_secs=60`, `sync/service.rs:63`) is the correctness fallback for missed/duplicated webhooks. Both funnel through `sync_source` → materialize-by-SHA (Step 2), which is idempotent, so a webhook and a poll at the same SHA never double-apply.
- **Why:** webhooks alone silently miss events (delivery failures, secret rotation); polling alone is high-latency and, per F3, is the current dead default. Convergence-on-SHA makes running both safe.

### ADR-3 — Git↔UI conflict model
- **Decision:** **Git is source of truth; UI writes become commits** (`conflict_mode = commit_back`, Step 9), matching the recommendation in `reviews/04:151`.
- **Mechanism:** a git-backed policy edit opens a branch + commit (optionally a PR) via the App installation token; deployed state is only ever changed by the sync path materializing a commit. There is exactly one lineage, so drift is structurally impossible and regulated buyers get PR-based approval for free.
- **Alternatives:** `read_only` (git authoritative, UI edits rejected — safe but limits the "AND via UI" pillar); `last_writer_wins` + drift alerts (simplest, but guarantees eventual divergence and is unauditable — **rejected as default**, offered only as an escape hatch).

---

## 9. Risks & rollback

- **Risk: materialization writes bad policy fleet-wide.** A malformed repo could compile a broken bundle. **Mitigation:** sync compiles a bundle but does **not** auto-rollout — it stops at "bundle ready"; promotion to agents remains the separate, gated step (see plan 10). Rollback = the existing rollback machinery (`api/deployments/rollouts.rs:227`).
- **Risk: SSRF guard breaks legitimate self-hosted git.** Enterprises with internal GitLab on private IPs will be blocked by the public-IP requirement. **Mitigation:** an explicit per-org allowlist of internal CIDRs (opt-in, audited), mirroring how the JWKS guard notes DNS-rebinding residual risk (`auth/jwks.rs:42-45`); default stays deny.
- **Risk: commit-back loop.** A UI edit → commit → webhook → materialize could re-trigger. **Mitigation:** SHA idempotency (Step 2) plus marking Reaper-authored commits so the webhook handler skips its own commits.
- **Risk: App private key compromise.** Installation tokens are powerful. **Mitigation:** short-lived minted-per-sync tokens (never stored), key in secret store, audited via `audit_log`.
- **Rollback plan:** every step is feature-flagged. Phase A is reversible by not spawning `SyncService` (revert `main.rs` spawn) — the manual trigger falls back to its current no-op. Migrations 010/011 are additive (nullable columns + new table); down-migrations drop them without touching existing rows. GitHub App and webhooks can be disabled by clearing `config.oauth.github.app_id`, reverting to poll-only OAuth read for repo listing.
