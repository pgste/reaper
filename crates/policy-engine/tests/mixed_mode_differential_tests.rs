//! Mixed-mode (per-rule compiled/AST fallback) differential tests — R4-01
//! Phase A.2's merge gate.
//!
//! Contract: for any policy that does not compile whole, the
//! `MixedReapEvaluator` produced by `build_preferred` must return the same
//! decision AND the same deciding rule name as the whole-policy
//! `ReapAstEvaluator` (which is what served these policies before A.2).
//! Mixing is a speed feature; any divergence is an authorization bug.
//!
//! The uncompilable construct used throughout is a literal-value assignment
//! (`x := "admin"`), which the compiler rejects (`reap/compiler/mod.rs`
//! "literal value assignment … not yet supported") but the interpreter
//! evaluates fine — the smallest reliable trigger. If a future slice makes
//! it compile, swap in another trigger from the REGO_GAP_ANALYSIS §4
//! inventory (the `mixed_policy_builds_mixed` test will fail loudly the
//! moment the trigger stops triggering).

use policy_engine::data::{DataLoader, DataStore};
use policy_engine::reap::ReaperPolicy;
use policy_engine::{PolicyAction, PolicyEvaluator, PolicyRequest};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

fn dataset() -> serde_json::Value {
    serde_json::json!({
        "entities": [
            {"id": "alice", "type": "user", "attributes": {
                "role": "admin", "status": "active", "clearance": 7
            }},
            {"id": "bob", "type": "user", "attributes": {
                "role": "guest", "status": "active", "clearance": 1
            }},
            {"id": "mallory", "type": "user", "attributes": {
                "role": "admin", "status": "suspended", "clearance": 9
            }},
            {"id": "doc1", "type": "resource", "attributes": {
                "classification": "public", "clearance_required": 3
            }},
            {"id": "doc2", "type": "resource", "attributes": {
                "classification": "secret", "clearance_required": 9
            }}
        ]
    })
}

fn store_with_data() -> Arc<DataStore> {
    let store = Arc::new(DataStore::new());
    let loader = DataLoader::new((*store).clone());
    loader
        .load_json(&dataset().to_string())
        .expect("load test data");
    store
}

fn request(principal: &str, resource: &str) -> PolicyRequest {
    let mut context = HashMap::new();
    context.insert("principal".to_string(), principal.to_string());
    PolicyRequest {
        resource: resource.to_string(),
        action: "read".to_string(),
        context,
        ..Default::default()
    }
}

/// A policy mixing compilable rules with the uncompilable literal-assignment
/// trigger, exercising all four ordering interactions:
/// - AST deny (mallory suspended) must override the compiled allow;
/// - compiled deny (doc2 secret vs clearance) must behave as before;
/// - AST allow and compiled allow interleave in source order.
const MIXED_POLICY: &str = r#"
policy mixed_diff {
    default: deny,

    rule suspended_deny_ast {
        deny if {
            wanted := "suspended" &&
            user.status == wanted
        }
    }

    rule clearance_deny_compiled {
        deny if resource.clearance_required > user.clearance
    }

    rule admin_allow_ast {
        allow if {
            x := "admin" &&
            user.role == x
        }
    }

    rule active_allow_compiled {
        allow if user.status == "active"
    }
}
"#;

/// Build the preferred (expected: mixed) evaluator and the whole-policy AST
/// evaluator; assert decision equivalence for one request, returning the
/// agreed decision.
fn assert_mixed_equals_ast(policy_text: &str, principal: &str, resource: &str) -> PolicyAction {
    let policy = ReaperPolicy::from_str(policy_text).expect("parse policy");

    let preferred = policy
        .clone()
        .build_preferred(store_with_data())
        .expect("build preferred");
    let ast = policy.build_ast_evaluator(store_with_data());

    let req = request(principal, resource);
    let mixed_named = preferred.evaluate_named(&req).expect("mixed evaluate");
    let ast_named = ast.evaluate_named(&req).expect("ast evaluate");

    assert_eq!(
        mixed_named.decision, ast_named.decision,
        "decision diverged for ({principal}, {resource})"
    );
    assert_eq!(
        mixed_named.rule_name, ast_named.rule_name,
        "deciding rule diverged for ({principal}, {resource})"
    );
    // Mirror of the AST always-decisive contract.
    assert!(mixed_named.matched, "mixed must stay always-decisive");
    mixed_named.decision
}

#[test]
fn mixed_policy_builds_mixed() {
    // Pin BOTH halves of the setup: the trigger really fails whole-policy
    // compilation, and per-rule fallback really produces the mixed shape
    // (not a silent whole-AST fallback).
    let policy = ReaperPolicy::from_str(MIXED_POLICY).expect("parse");
    assert!(
        policy.clone().build(store_with_data()).is_err(),
        "literal assignment must still be a compile-fallback trigger; \
         if this fails, the trigger compiled — update this suite's trigger"
    );

    let preferred = policy.build_preferred(store_with_data()).expect("build");
    assert_eq!(preferred.evaluator_type(), "reaper_dsl_mixed");

    let meta = preferred.metadata().expect("mixed metadata");
    assert_eq!(meta.rule_count, 4);
    assert_eq!(meta.extra.get("compiled_rules").unwrap(), "2");
    assert_eq!(meta.extra.get("ast_rules").unwrap(), "2");
    assert_eq!(
        meta.extra.get("ast_rule_names").unwrap(),
        "suspended_deny_ast,admin_allow_ast"
    );
}

#[test]
fn fully_compilable_policy_stays_compiled() {
    let policy = ReaperPolicy::from_str(
        r#"
policy all_compiled {
    default: deny,
    rule admin { allow if user.role == "admin" }
}
"#,
    )
    .expect("parse");
    let preferred = policy.build_preferred(store_with_data()).expect("build");
    assert_eq!(preferred.evaluator_type(), "reaper_dsl");
}

#[test]
fn fully_uncompilable_policy_stays_whole_ast() {
    let policy = ReaperPolicy::from_str(
        r#"
policy all_ast {
    default: deny,
    rule a { allow if { x := "admin" && user.role == x } }
    rule b { deny if { y := "suspended" && user.status == y } }
}
"#,
    )
    .expect("parse");
    let preferred = policy.build_preferred(store_with_data()).expect("build");
    assert_eq!(preferred.evaluator_type(), "ReapAstEvaluator");
}

#[test]
fn differential_over_request_matrix() {
    // Every (principal, resource) pair — covers: AST deny overriding
    // compiled allow (mallory), compiled deny (bob on doc2: clearance),
    // AST allow winning by source order (alice: admin_allow_ast precedes
    // active_allow_compiled), compiled allow (bob on doc1: guest but
    // active), and the default (nobody unknown).
    for principal in ["alice", "bob", "mallory", "unknown"] {
        for resource in ["doc1", "doc2", "missing_doc"] {
            assert_mixed_equals_ast(MIXED_POLICY, principal, resource);
        }
    }
}

#[test]
fn ast_deny_overrides_compiled_allow() {
    // mallory is an active-format admin by role but suspended: the AST deny
    // rule must win over BOTH compiled/AST allow rules, across the mode split.
    let decision = assert_mixed_equals_ast(MIXED_POLICY, "mallory", "doc1");
    assert_eq!(decision, PolicyAction::Deny);

    let policy = ReaperPolicy::from_str(MIXED_POLICY).expect("parse");
    let preferred = policy.build_preferred(store_with_data()).expect("build");
    let named = preferred
        .evaluate_named(&request("mallory", "doc1"))
        .expect("evaluate");
    assert_eq!(named.rule_name, Some("suspended_deny_ast"));
}

#[test]
fn first_allow_wins_in_source_order_across_modes() {
    // alice matches BOTH allow rules; the AST one comes first in source
    // order and must name the decision — proving the wrapper preserves
    // source order across the compiled/AST split rather than grouping by
    // mode.
    let policy = ReaperPolicy::from_str(MIXED_POLICY).expect("parse");
    let preferred = policy.build_preferred(store_with_data()).expect("build");
    let named = preferred
        .evaluate_named(&request("alice", "doc1"))
        .expect("evaluate");
    assert_eq!(named.decision, PolicyAction::Allow);
    assert_eq!(named.rule_name, Some("admin_allow_ast"));

    // And the converse: a policy where the compiled allow precedes the AST
    // allow must name the compiled one.
    let swapped = r#"
policy swapped {
    default: deny,
    rule active_allow_compiled { allow if user.status == "active" }
    rule admin_allow_ast { allow if { x := "admin" && user.role == x } }
}
"#;
    let policy = ReaperPolicy::from_str(swapped).expect("parse");
    let preferred = policy.build_preferred(store_with_data()).expect("build");
    assert_eq!(preferred.evaluator_type(), "reaper_dsl_mixed");
    let named = preferred
        .evaluate_named(&request("alice", "doc1"))
        .expect("evaluate");
    assert_eq!(named.rule_name, Some("active_allow_compiled"));
}

#[test]
fn compiled_deny_still_wins_inside_mixed() {
    // bob (clearance 1) on doc2 (requires 9): the COMPILED deny decides.
    let policy = ReaperPolicy::from_str(MIXED_POLICY).expect("parse");
    let preferred = policy.build_preferred(store_with_data()).expect("build");
    let named = preferred
        .evaluate_named(&request("bob", "doc2"))
        .expect("evaluate");
    assert_eq!(named.decision, PolicyAction::Deny);
    assert_eq!(named.rule_name, Some("clearance_deny_compiled"));
}

#[test]
fn default_applies_once_when_nothing_matches() {
    // unknown principal on a missing resource: no rule matches anywhere;
    // the wrapper (not any single-rule sub-policy) must apply the default
    // exactly once, with no rule name.
    let policy = ReaperPolicy::from_str(MIXED_POLICY).expect("parse");
    let preferred = policy.build_preferred(store_with_data()).expect("build");
    let named = preferred
        .evaluate_named(&request("unknown", "missing_doc"))
        .expect("evaluate");
    assert_eq!(named.decision, PolicyAction::Deny);
    assert_eq!(named.rule_name, None);
    assert!(named.matched, "mirrors the AST always-decisive contract");

    // Same with default: allow — proving the surfaced default is the
    // parent policy's, not a sub-policy artifact.
    let default_allow = r#"
policy mixed_default_allow {
    default: allow,
    rule deny_secret_compiled { deny if resource.classification == "secret" }
    rule admin_allow_ast { allow if { x := "admin" && user.role == x } }
}
"#;
    let policy = ReaperPolicy::from_str(default_allow).expect("parse");
    let preferred = policy.build_preferred(store_with_data()).expect("build");
    assert_eq!(preferred.evaluator_type(), "reaper_dsl_mixed");
    let named = preferred
        .evaluate_named(&request("bob", "doc1"))
        .expect("evaluate");
    assert_eq!(named.decision, PolicyAction::Allow);
    assert_eq!(named.rule_name, None);
}
