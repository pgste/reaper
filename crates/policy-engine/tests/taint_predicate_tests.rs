//! F1-s2 part 2: the DSL taint predicate (`taint::level` / `taint::trusted`).
//!
//! Per-request context provenance lets a policy demand that a sensitive
//! attribute was platform-derived, not asserted by a possibly-injected LLM.
//! Fail-untrusted: taint mode off ⇒ everything reads platform (pre-F1
//! behavior); taint mode on ⇒ an unlabeled key is `llm`. Since F1-s2c,
//! `taint::` policies run on the COMPILED evaluator (sub-microsecond path);
//! the AST interpreter keeps identical semantics.

use policy_engine::reap::ReaperPolicy;
use policy_engine::{
    DataLoader, DataStore, PolicyAction, PolicyEvaluator, PolicyRequest, TrustLevel,
};
use std::collections::HashMap;
use std::sync::Arc;

fn store() -> Arc<DataStore> {
    let s = Arc::new(DataStore::new());
    let data = serde_json::json!({
        "entities": [{"id": "alice", "type": "user", "attributes": {}}]
    });
    DataLoader::new((*s).clone())
        .load_json(&data.to_string())
        .unwrap();
    s
}

fn req(context: &[(&str, &str)], prov: Option<&[(&str, TrustLevel)]>) -> PolicyRequest {
    let mut ctx = HashMap::new();
    ctx.insert("principal".to_string(), "alice".to_string());
    for (k, v) in context {
        ctx.insert(k.to_string(), v.to_string());
    }
    PolicyRequest {
        resource: "r".to_string(),
        action: "act".to_string(),
        context: ctx,
        context_provenance: prov.map(|p| {
            p.iter()
                .map(|(k, t)| (k.to_string(), *t))
                .collect::<HashMap<_, _>>()
        }),
        ..Default::default()
    }
}

// A privileged action requires the approval to be platform-trusted, not an
// attribute the LLM could have smuggled into context.
const POLICY: &str = r#"
policy sensitive {
    default: deny,
    rule gated {
        allow if {
            context.approved == "yes" &&
            taint::trusted("approved")
        }
    }
}
"#;

fn eval() -> Box<dyn PolicyEvaluator> {
    let policy: ReaperPolicy = POLICY.parse().unwrap();
    policy.build_preferred(store()).unwrap()
}

#[test]
fn taint_policy_runs_compiled() {
    // Fast AND secure: the provenance gate must not cost the AST fallback.
    assert_eq!(eval().evaluator_type(), "reaper_dsl");
}

#[test]
fn platform_trusted_attribute_is_allowed() {
    let e = eval();
    let d = e
        .evaluate(&req(
            &[("approved", "yes")],
            Some(&[("approved", TrustLevel::Platform)]),
        ))
        .unwrap();
    assert_eq!(d, PolicyAction::Allow);
}

#[test]
fn llm_asserted_attribute_cannot_satisfy_trust_gate() {
    let e = eval();
    // Same value, but the approval was asserted by the LLM — the taint gate
    // rejects it even though `context.approved == "yes"`.
    let d = e
        .evaluate(&req(
            &[("approved", "yes")],
            Some(&[("approved", TrustLevel::Llm)]),
        ))
        .unwrap();
    assert_eq!(
        d,
        PolicyAction::Deny,
        "LLM-tainted attribute must not pass a trust gate"
    );
}

#[test]
fn unlabeled_key_under_taint_mode_is_llm_and_denied() {
    let e = eval();
    // Taint mode ON (provenance present) but the key is unlabeled ⇒ floor to
    // llm ⇒ denied. This is the fail-untrusted default: a smuggled key can't
    // pass by simply omitting its label.
    let d = e
        .evaluate(&req(
            &[("approved", "yes")],
            Some(&[("something_else", TrustLevel::Platform)]),
        ))
        .unwrap();
    assert_eq!(d, PolicyAction::Deny);
}

#[test]
fn taint_mode_off_treats_everything_as_platform() {
    let e = eval();
    // No provenance map at all ⇒ pre-F1 behavior ⇒ trusted ⇒ allowed.
    let d = e.evaluate(&req(&[("approved", "yes")], None)).unwrap();
    assert_eq!(d, PolicyAction::Allow);
}

#[test]
fn taint_level_exact_values() {
    // `taint::level` returns a string; used in a comparison via assignment
    // form (same as `now := time::now_ns()` — the grammar takes namespaced
    // calls as boolean predicates or assignment RHS, not a bare comparison
    // LHS).
    let policy_src = r#"
policy lvl {
    default: deny,
    rule r { allow if { level := taint::level("k") && level == "verified" } }
}
"#;
    let policy: ReaperPolicy = policy_src.parse().unwrap();
    let e = policy.build_preferred(store()).unwrap();
    // the assignment form compiles too (CompiledExprType::TaintLevel)
    assert_eq!(e.evaluator_type(), "reaper_dsl");
    // exactly verified → allow
    assert_eq!(
        e.evaluate(&req(&[], Some(&[("k", TrustLevel::Verified)])))
            .unwrap(),
        PolicyAction::Allow
    );
    // platform is a different level string → this exact-match rule denies
    assert_eq!(
        e.evaluate(&req(&[], Some(&[("k", TrustLevel::Platform)])))
            .unwrap(),
        PolicyAction::Deny
    );
    // llm → deny
    assert_eq!(
        e.evaluate(&req(&[], Some(&[("k", TrustLevel::Llm)])))
            .unwrap(),
        PolicyAction::Deny
    );
}
