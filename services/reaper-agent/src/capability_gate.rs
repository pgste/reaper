//! Pre-eval capability enforcement (F1-s3 agentic authz).
//!
//! An agentic caller may attach a signed capability — "actor X may exercise
//! these grants on behalf of subject Y until T" — to an evaluation request.
//! This gate runs BEFORE policy evaluation and fails closed:
//!
//! - the capability's signature/window/revocation are verified against the
//!   same trust anchor as bundles ([`BundleVerifier::verify_capability`]);
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
pub fn enforce(
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

    // Cryptographic + freshness + revocation checks (fail closed, including
    // "no trust anchor configured").
    state.bundle_verifier.verify_capability(cap, now)?;

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
