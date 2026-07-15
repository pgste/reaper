//! Failure-mode matrix — engine tier (Plan 05, Step 5).
//!
//! One test per fail-open-vs-fail-closed row that the enforcement *engine*
//! owns. The agent-tier rows (audit gate, data-staleness gate, handler panic,
//! batch cap, DSL depth) are asserted by their own suites and referenced from
//! `docs/deployment/OPERATIONS_GUIDE.md`.

use std::collections::HashMap;

use policy_engine::{
    EnhancedPolicy, PolicyAction, PolicyEngine, PolicyLanguage, PolicyRequest, PolicyRule,
};
use reaper_core::PolicyId;

fn request() -> PolicyRequest {
    PolicyRequest {
        resource: "/doc".to_string(),
        action: "read".to_string(),
        context: {
            let mut c = HashMap::new();
            c.insert("principal".to_string(), "alice".to_string());
            c
        },

        ..Default::default()
    }
}

fn allow_all(name: &str) -> EnhancedPolicy {
    EnhancedPolicy::new(
        name.to_string(),
        "test".to_string(),
        vec![PolicyRule {
            action: PolicyAction::Allow,
            resource: "*".to_string(),
            conditions: vec![],
        }],
    )
}

/// Row: no policy loaded / nothing matched ⇒ default deny (fail closed).
#[test]
fn no_policy_denies() {
    let engine = PolicyEngine::new();
    let outcome = engine.evaluate_set(&[], &request());
    assert_eq!(outcome.decision, PolicyAction::Deny);
    assert!(outcome.policy_id.is_nil(), "no policy attributed");
}

/// Row: any evaluation error ⇒ deny and stop (fail closed). An unknown policy
/// id makes `evaluate` return `Err`, exercising the `Err => Deny` branch that
/// every runtime evaluation error also takes.
#[test]
fn evaluation_error_denies() {
    let engine = PolicyEngine::new();
    let unknown: PolicyId = PolicyId::new_v4();
    let outcome = engine.evaluate_set(&[unknown], &request());
    assert_eq!(outcome.decision, PolicyAction::Deny);
    assert!(
        outcome.error.is_some(),
        "the error is recorded on the fail-closed outcome"
    );
}

/// Row: a policy load/compile failure never displaces the last-good policy.
/// Validation happens at construction, so a malformed policy fails to build and
/// the live policy keeps serving unchanged (atomic hot-swap).
#[test]
fn deploy_failure_keeps_last_good() {
    let engine = PolicyEngine::new();

    let good = allow_all("good");
    let good_id = good.id;
    engine.deploy_policy(good).expect("good policy deploys");
    assert_eq!(
        engine.evaluate_set(&[good_id], &request()).decision,
        PolicyAction::Allow
    );

    // A malformed DSL policy fails to build — it can never reach deploy_policy,
    // so it cannot replace the live policy.
    let bad = EnhancedPolicy::new_with_language(
        "bad".to_string(),
        "test".to_string(),
        PolicyLanguage::ReaperDsl,
        "this is definitely not a valid reap policy {{{".to_string(),
    );
    assert!(bad.is_err(), "malformed policy is rejected at build time");

    // The last-good policy still serves its original decision.
    assert_eq!(
        engine.evaluate_set(&[good_id], &request()).decision,
        PolicyAction::Allow,
        "a failed load must not disturb the live policy"
    );
}
