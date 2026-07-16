//! F1-s4: allow-path explainability — the deciding rule's NAME is surfaced.
//!
//! For AI-actor traffic the ALLOWS are the dangerous decisions; "which rule
//! allowed this" must be answerable from the decision record without
//! replaying the request. These tests pin: both evaluators name the deciding
//! rule identically (allow AND deny), the per-policy default reports no
//! name, and `evaluate_set` carries the name on the served outcome.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::sync::Arc;

use policy_engine::data::{DataLoader, DataStore};
use policy_engine::reap::ReaperPolicy;
use policy_engine::{
    EnhancedPolicy, PolicyAction, PolicyEngine, PolicyEvaluator, PolicyLanguage, PolicyRequest,
};

/// Deploy the explain-test policy into an engine, evaluator bound to `store`.
fn deployed(engine: &PolicyEngine, s: Arc<DataStore>) -> policy_engine::PolicyId {
    let mut p = EnhancedPolicy::new_with_language(
        "explain_test".to_string(),
        String::new(),
        PolicyLanguage::ReaperDsl,
        POLICY.to_string(),
    )
    .unwrap();
    p.build_evaluator_with_data(Some(s)).unwrap();
    let id = p.id;
    engine.deploy_policy(p).unwrap();
    id
}

fn store() -> Arc<DataStore> {
    let s = Arc::new(DataStore::new());
    let data = serde_json::json!({
        "entities": [
            {"id": "alice", "type": "user", "attributes": {"role": "admin", "level": 9}},
            {"id": "bob", "type": "user", "attributes": {"role": "guest", "level": 1}},
            {"id": "res-1", "type": "resource", "attributes": {}}
        ]
    });
    DataLoader::new((*s).clone())
        .load_json(&data.to_string())
        .unwrap();
    s
}

const POLICY: &str = r#"
policy explain_test {
    default: deny,
    rule block_guests {
        deny if user.role == "guest"
    }
    rule admins_allowed {
        allow if user.role == "admin"
    }
}
"#;

fn req(principal: &str) -> PolicyRequest {
    let mut context = HashMap::new();
    context.insert("principal".to_string(), principal.to_string());
    PolicyRequest {
        resource: "res-1".to_string(),
        action: "read".to_string(),
        context,
        ..Default::default()
    }
}

#[test]
fn compiled_evaluator_names_the_deciding_rule() {
    let policy: ReaperPolicy = POLICY.parse().unwrap();
    let eval = policy.build_preferred(store()).unwrap();
    assert_eq!(eval.evaluator_type(), "reaper_dsl");

    // Allow: the name of the rule that allowed.
    let named = eval.evaluate_named(&req("alice")).unwrap();
    assert_eq!(named.decision, PolicyAction::Allow);
    assert!(named.matched);
    assert_eq!(named.rule_name, Some("admins_allowed"));

    // Deny: the name of the rule that denied.
    let named = eval.evaluate_named(&req("bob")).unwrap();
    assert_eq!(named.decision, PolicyAction::Deny);
    assert_eq!(named.rule_name, Some("block_guests"));
}

#[test]
fn default_decision_reports_no_rule_name() {
    let policy: ReaperPolicy = POLICY.parse().unwrap();
    let s = store();
    // carol is loaded (unknown principals error on the compiled path) but her
    // role matches neither rule, so the per-policy default decides.
    let data = serde_json::json!({
        "entities": [{"id": "carol", "type": "user", "attributes": {"role": "viewer"}}]
    });
    DataLoader::new((*s).clone())
        .load_json(&data.to_string())
        .unwrap();
    let eval = policy.build_preferred(s).unwrap();

    let named = eval.evaluate_named(&req("carol")).unwrap();
    assert_eq!(named.decision, PolicyAction::Deny, "default deny");
    assert!(!named.matched, "default decision is not a match");
    assert_eq!(named.rule_name, None, "no rule decided — no name");
}

#[test]
fn ast_evaluator_names_agree_with_compiled() {
    let policy: ReaperPolicy = POLICY.parse().unwrap();
    let compiled = policy.clone().build(store()).unwrap();
    let ast = policy.build_ast_evaluator(store());

    for principal in ["alice", "bob"] {
        let c = compiled.evaluate_named(&req(principal)).unwrap();
        let a = ast.evaluate_named(&req(principal)).unwrap();
        assert_eq!(c.decision, a.decision, "decision parity for {principal}");
        assert_eq!(c.rule_name, a.rule_name, "rule-name parity for {principal}");
        assert!(c.rule_name.is_some());
    }
}

#[test]
fn evaluate_set_carries_the_rule_name() {
    let engine = PolicyEngine::new();
    let policy_id = deployed(&engine, store());

    let outcome = engine.evaluate_set(&[policy_id], &req("alice"));
    assert_eq!(outcome.decision, PolicyAction::Allow);
    assert_eq!(outcome.matched_rule_name.as_deref(), Some("admins_allowed"));

    let outcome = engine.evaluate_set(&[policy_id], &req("bob"));
    assert_eq!(outcome.decision, PolicyAction::Deny);
    assert_eq!(outcome.matched_rule_name.as_deref(), Some("block_guests"));
}

#[test]
fn single_policy_evaluate_carries_the_rule_name() {
    let engine = PolicyEngine::new();
    let policy_id = deployed(&engine, store());

    let d = engine.evaluate(&policy_id, &req("alice")).unwrap();
    assert_eq!(d.decision, PolicyAction::Allow);
    assert_eq!(d.matched_rule_name.as_deref(), Some("admins_allowed"));
}
