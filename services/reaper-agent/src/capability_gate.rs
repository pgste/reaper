//! Pre-eval capability enforcement (F1-s3 agentic authz).
//!
//! An agentic caller may attach a signed capability — "actor X may exercise
//! these grants on behalf of subject Y until T" — to an evaluation request.
//! This gate runs BEFORE policy evaluation and fails closed:
//!
//! - the capability's signature/window/revocation are verified against the
//!   same trust anchor as bundles (`BundleVerifier::verify_capability_with`, verdict-cached — Plan 06 D);
//! - the capability must BIND to the request: its `subject` must equal the
//!   request principal and its `actor` the request actor (if the request
//!   names none, the capability's actor is injected — the capability IS the
//!   actor credential);
//! - the request's `(action, resource)` must be covered by the grants.
//!
//! A request without a capability is untouched unless the operator set
//! `auth.require_actor_capability` (`REAPER_REQUIRE_ACTOR_CAPABILITY`), in
//! which case any actor-carrying request without a valid capability is
//! denied — the posture for fleets where every agentic caller must present
//! a derived, expiring credential.
//!
//! Denials surface exactly like the other pre-eval guards (`data_stale`,
//! `policy_not_found`): a served `decision: "deny"` with the reason in
//! `matched_rule`, so SDKs and decision consumers need no new error shape.

use reaper_core::capability::Capability;

use crate::state::AgentState;

/// Stable `matched_rule` reason prefixes (grep-able, dashboard-friendly).
pub const REASON_REQUIRED: &str = "capability_required";
pub const REASON_SUBJECT_MISMATCH: &str = "capability_subject_mismatch";
pub const REASON_ACTOR_MISMATCH: &str = "capability_actor_mismatch";
pub const REASON_OUT_OF_GRANT: &str = "capability_out_of_grant";
/// Plan 06 Phase D: the principal exhausted its per-minute budget of FULL
/// (cache-missing) capability verifications.
pub const REASON_RATE_LIMITED: &str = "capability_verify_rate_limited";

/// Current unix seconds for capability windows. The agent is a native
/// service; the engine-side pure clock stays untouched.
pub fn now_unix() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Enforce capability policy for one evaluation request. On success the
/// request's `actor` may be filled in from the capability. `Err(reason)`
/// means DENY with that reason as `matched_rule`.
///
/// ## Verification strategy (Plan 06 Phase D, R3-P2-2 / ADR-4)
/// - **Verdict-cache HIT** (content digest + revocation generation already
///   verified, see [`crate::capability_cache`]): skip the ed25519 verify;
///   still enforce the validity window and the LIVE revocation set via
///   [`Capability::check_validity_at`] — pure integer/set checks, on-reactor.
/// - **MISS**: charge the principal's verify budget (a garbage-signature
///   flood never hits, so this is exactly the cost the limiter bounds), run
///   the full verification in `spawn_blocking` (~30-50µs of ed25519 CPU off
///   the reactor), and cache only a POSITIVE verdict.
/// - **Cache disabled** (`auth.capability_cache_enabled=false`): the exact
///   pre-Phase-D inline verify — the documented rollback path.
pub async fn enforce(
    state: &AgentState,
    principal: &str,
    action: &str,
    resource: &str,
    actor: &mut Option<String>,
    capability: Option<&Capability>,
    now: i64,
) -> Result<(), String> {
    let Some(cap) = capability else {
        // No capability presented. Fine — unless the operator requires one
        // for actor-carrying requests.
        if actor.is_some() && state.agent_config.auth.require_actor_capability {
            return Err(format!(
                "{REASON_REQUIRED}: actor-carrying request without a capability \
                 (auth.require_actor_capability is set)"
            ));
        }
        return Ok(());
    };

    // Revocation snapshot: generation (the applied list's serial) + set,
    // with the trust-anchor and staleness fail-closed checks applied first.
    let (generation, revoked) = state.bundle_verifier.capability_revocation_snapshot(now)?;

    let gate = &state.capability_gate;
    if !gate.cache_enabled {
        // Rollback path: verbatim pre-Phase-D behavior (inline verify on the
        // reactor, nothing cached). The rate limiter still applies — it is
        // independently disabled via capability_verify_limit_per_min=0.
        if !gate.limiter.admit(principal, now) {
            return Err(rate_limited_reason(principal));
        }
        state
            .bundle_verifier
            .verify_capability_with(cap, now, &revoked)?;
    } else {
        let key = (cap.cache_digest(), generation);
        if gate.cache.check(&key, now) {
            // Content-bound checks (signature/key-pin/algorithm/version) are
            // proven for these exact bytes under this generation; the clock
            // and the live revocation set are re-checked every time.
            cap.check_validity_at(now, &revoked)
                .map_err(|e| format!("capability rejected: {e}"))?;
        } else {
            if !gate.limiter.admit(principal, now) {
                return Err(rate_limited_reason(principal));
            }
            // Cold verify off the reactor. Owned clones: spawn_blocking needs
            // 'static, and a capability is a small struct (id/subject/actor/
            // grants strings) — cloned only on the rare miss path.
            let verifier = std::sync::Arc::clone(&state.bundle_verifier);
            let cap_owned = cap.clone();
            let revoked_owned = std::sync::Arc::clone(&revoked);
            tokio::task::spawn_blocking(move || {
                verifier.verify_capability_with(&cap_owned, now, &revoked_owned)
            })
            .await
            .map_err(|e| format!("capability verification task failed: {e}"))??;
            // POSITIVE verdicts only: a failed verify caches nothing.
            gate.cache.insert(key, now);
        }
    }

    // Subject binding: the capability derives from a durable principal; the
    // request must be made on that principal's behalf.
    if cap.subject != principal {
        return Err(format!(
            "{REASON_SUBJECT_MISMATCH}: capability subject '{}' != request principal '{}'",
            cap.subject, principal
        ));
    }

    // Actor binding: a request naming a different actor than the capability
    // was minted for is a confused-deputy attempt. A request naming none
    // inherits the capability's actor — the token IS the actor credential.
    match actor {
        Some(a) if *a != cap.actor => {
            return Err(format!(
                "{REASON_ACTOR_MISMATCH}: capability actor '{}' != request actor '{a}'",
                cap.actor
            ));
        }
        Some(_) => {}
        None => *actor = Some(cap.actor.clone()),
    }

    // Grant coverage: the concrete (action, resource) must be inside the
    // capability's (attenuated) authority.
    if !cap.authorizes(action, resource) {
        return Err(format!(
            "{REASON_OUT_OF_GRANT}: capability does not grant ({action}, {resource})"
        ));
    }

    Ok(())
}

fn rate_limited_reason(principal: &str) -> String {
    format!(
        "{REASON_RATE_LIMITED}: principal '{principal}' exceeded its per-minute \
         capability-verification budget (auth.capability_verify_limit_per_min)"
    )
}
