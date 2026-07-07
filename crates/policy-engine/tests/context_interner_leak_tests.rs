//! Eval-path interner-leak regression: `context.*` cross-entity comparisons
//! must NOT intern the per-request value.
//!
//! A policy comparing a high-cardinality request-context attribute to an entity
//! attribute — e.g. `context.token == resource.secret` — used to call
//! `interner.intern(raw)` on the request token, pinning it in the shared
//! interner forever. Under high request cardinality (a distinct token per
//! request) that grew the interner without bound on the hot eval path. The fix
//! compares the request value by content (already-interned → by id; novel → by
//! raw text) without ever interning it. These tests pin both the decision
//! correctness (compiled ≡ AST) and the boundedness (interner does not grow per
//! request).

use policy_engine::data::{DataLoader, DataStore};
use policy_engine::reap::ReaperPolicy;
use policy_engine::{PolicyAction, PolicyEvaluator, PolicyRequest};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

const POLICY: &str = r#"
policy p {
    default: deny,
    rule r { allow if { context.token == resource.secret } }
}
"#;

fn store() -> Arc<DataStore> {
    let data = serde_json::json!({
        "entities": [
            {"id": "alice", "type": "user", "attributes": {"role": "admin"}},
            {"id": "doc", "type": "resource", "attributes": {"secret": "s3cr3t"}}
        ]
    });
    let store = Arc::new(DataStore::new());
    DataLoader::new((*store).clone())
        .load_json(&data.to_string())
        .expect("load data");
    store
}

/// Request as principal `alice` (a loaded entity) carrying a `token` context
/// attribute — the high-cardinality value the policy compares.
fn request(token: &str) -> PolicyRequest {
    let mut context = HashMap::new();
    context.insert("principal".to_string(), "alice".to_string());
    context.insert("token".to_string(), token.to_string());
    PolicyRequest {
        resource: "doc".to_string(),
        action: "read".to_string(),
        context,
    }
}

#[test]
fn context_token_match_decisions_and_parity() {
    let policy = ReaperPolicy::from_str(POLICY).expect("parse");
    let compiled = policy.clone().build(store()).expect("compiled");
    let ast = policy.build_ast_evaluator(store());

    // Matching token (equals resource.secret) -> allow; else deny. Compiled and
    // AST must agree on both.
    for (token, expected) in [
        ("s3cr3t", PolicyAction::Allow),
        ("nope", PolicyAction::Deny),
    ] {
        let req = request(token);
        let c = compiled.evaluate(&req).expect("compiled eval");
        let a = ast.evaluate(&req).expect("ast eval");
        assert_eq!(c, a, "compiled/AST diverged for token={token}");
        assert_eq!(c, expected, "wrong decision for token={token}");
    }
}

#[test]
fn high_cardinality_context_values_do_not_grow_the_interner() {
    let store = store();
    let compiled = ReaperPolicy::from_str(POLICY)
        .expect("parse")
        .build(store.clone())
        .expect("compiled");

    // Baseline after the policy is compiled and data loaded — evaluating novel
    // context tokens must not push this number up.
    let baseline = store.interner().stats().unique_strings;

    for i in 0..10_000 {
        // A distinct, never-before-seen token on every request.
        let decision = compiled
            .evaluate(&request(&format!("ephemeral_token_{i}")))
            .expect("eval");
        // None of these equal resource.secret -> deny.
        assert_eq!(decision, PolicyAction::Deny);
    }

    let after = store.interner().stats().unique_strings;
    assert_eq!(
        after, baseline,
        "eval interned per-request context values: interner grew {} -> {} over 10k unique tokens \
         (should be 0 — the context value must not be interned)",
        baseline, after
    );
}

#[test]
fn matching_context_value_still_allows_after_churn() {
    // Even after many novel tokens, a token that genuinely equals the secret is
    // still allowed (the by-content comparison is correct, not just "never
    // equal").
    let store = store();
    let compiled = ReaperPolicy::from_str(POLICY)
        .expect("parse")
        .build(store.clone())
        .expect("compiled");

    for i in 0..1_000 {
        let _ = compiled.evaluate(&request(&format!("noise_{i}")));
    }
    assert_eq!(
        compiled.evaluate(&request("s3cr3t")).expect("eval"),
        PolicyAction::Allow,
        "matching context value must still allow after churn"
    );
}
