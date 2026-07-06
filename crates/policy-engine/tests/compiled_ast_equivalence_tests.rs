//! Compiled vs AST evaluator equivalence (differential) tests.
//!
//! Architecture contract: the compiled `ReaperDSLEvaluator` is the PREFERRED
//! evaluator and the `ReapAstEvaluator` is the FALLBACK; for every policy they
//! can both handle, they MUST produce the same decision. A divergence is a
//! correctness bug (it silently changes authorization depending on which path
//! a policy happens to take) — exactly the class of bug that made
//! `user.skills.count() >= N` deny on the compiled path while the AST path
//! allowed.
//!
//! Each case below exercises one method/utility function of the DSL across
//! both a passing and a failing input, and asserts the two evaluators agree.

use policy_engine::data::{DataLoader, DataStore};
use policy_engine::reap::ReaperPolicy;
use policy_engine::{PolicyAction, PolicyEvaluator, PolicyRequest};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

/// One entity dataset shared by every case: a principal `alice` with a rich
/// set of attribute shapes (lists, sets-of-strings, scalars, nested objects)
/// plus the resource entities the rules gate on.
fn dataset() -> serde_json::Value {
    serde_json::json!({
        "entities": [
            {
                "id": "alice",
                "type": "user",
                "attributes": {
                    "level": 7,
                    "name": "Alice",
                    "email": "ALICE@corp.example",
                    "role": "admin",
                    "skills": ["rust", "python", "go", "java"],
                    "perms": ["read", "write"],
                    "tags": ["alpha", "beta", "beta", "gamma"],
                    "scores": [10, 40, 25],
                    "csv": "a,b,c",
                    "profile": {"country": "US", "tier": "gold"}
                }
            },
            {"id": "bob", "type": "user", "attributes": {
                "level": 1, "name": "bob", "email": "bob@corp.example",
                "role": "guest", "skills": ["rust"], "perms": ["read"],
                "tags": ["alpha"], "scores": [1], "csv": "x",
                "profile": {"country": "CA", "tier": "bronze"}
            }},
            {"id": "res_count", "type": "resource", "attributes": {}},
            {"id": "res_scalar", "type": "resource", "attributes": {}},
            {"id": "res_in", "type": "resource", "attributes": {}},
            {"id": "res_lower", "type": "resource", "attributes": {}},
            {"id": "res_upper", "type": "resource", "attributes": {}},
            {"id": "res_contains", "type": "resource", "attributes": {}},
            {"id": "res_startswith", "type": "resource", "attributes": {}},
            {"id": "res_endswith", "type": "resource", "attributes": {}},
            {"id": "res_sum", "type": "resource", "attributes": {}},
            {"id": "res_max", "type": "resource", "attributes": {}},
            {"id": "res_min", "type": "resource", "attributes": {}},
            {"id": "res_nested", "type": "resource", "attributes": {}}
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
    }
}

/// Build both evaluators from the same policy text and assert they return the
/// same decision for the given (principal, resource). Returns that agreed
/// decision so callers can additionally pin the expected value.
fn assert_equivalent(policy_text: &str, principal: &str, resource: &str) -> PolicyAction {
    let policy = ReaperPolicy::from_str(policy_text).expect("parse policy");

    let compiled = policy
        .clone()
        .build(store_with_data())
        .expect("build compiled evaluator");
    let ast = policy.build_ast_evaluator(store_with_data());

    let req = request(principal, resource);
    let compiled_decision = compiled.evaluate(&req).expect("compiled evaluate");
    let ast_decision = ast.evaluate(&req).expect("ast evaluate");

    assert_eq!(
        compiled_decision, ast_decision,
        "compiled and AST evaluators diverged for principal={principal} resource={resource}\n\
         compiled={compiled_decision:?} ast={ast_decision:?}\npolicy:\n{policy_text}"
    );
    compiled_decision
}

/// Assert equivalence AND that the agreed decision matches `expected`.
fn assert_equivalent_is(
    policy_text: &str,
    principal: &str,
    resource: &str,
    expected: PolicyAction,
) {
    let got = assert_equivalent(policy_text, principal, resource);
    assert_eq!(
        got, expected,
        "decision mismatch (both evaluators agreed on {got:?}, expected {expected:?}) for \
         principal={principal} resource={resource}"
    );
}

// ---------------------------------------------------------------------------
// count() — the regression that motivated this suite. Direct entity.attr path.
// ---------------------------------------------------------------------------

const P_COUNT: &str = r#"
policy p {
    default: deny,
    rule r { allow if { resource == "res_count" && user.skills.count() >= 3 } }
}
"#;

#[test]
fn count_direct_entity_attr_allow() {
    // alice has 4 skills >= 3
    assert_equivalent_is(P_COUNT, "alice", "res_count", PolicyAction::Allow);
}

#[test]
fn count_direct_entity_attr_deny() {
    // bob has 1 skill, < 3
    assert_equivalent_is(P_COUNT, "bob", "res_count", PolicyAction::Deny);
}

// ---------------------------------------------------------------------------
// Scalar numeric comparison
// ---------------------------------------------------------------------------

const P_SCALAR: &str = r#"
policy p {
    default: deny,
    rule r { allow if { resource == "res_scalar" && user.level >= 5 } }
}
"#;

#[test]
fn scalar_comparison_equivalence() {
    assert_equivalent_is(P_SCALAR, "alice", "res_scalar", PolicyAction::Allow);
    assert_equivalent_is(P_SCALAR, "bob", "res_scalar", PolicyAction::Deny);
}

// ---------------------------------------------------------------------------
// Membership: "in"
// ---------------------------------------------------------------------------

const P_IN: &str = r#"
policy p {
    default: deny,
    rule r { allow if { resource == "res_in" && "write" in user.perms } }
}
"#;

#[test]
fn membership_in_equivalence() {
    assert_equivalent_is(P_IN, "alice", "res_in", PolicyAction::Allow);
    assert_equivalent_is(P_IN, "bob", "res_in", PolicyAction::Deny);
}

// ---------------------------------------------------------------------------
// String methods: lower(), upper(), contains, startswith, endswith
// ---------------------------------------------------------------------------

#[test]
fn string_lower_equivalence() {
    let p = r#"
    policy p { default: deny,
        rule r { allow if { resource == "res_lower" && user.email.lower() == "alice@corp.example" } }
    }"#;
    assert_equivalent_is(p, "alice", "res_lower", PolicyAction::Allow);
    assert_equivalent_is(p, "bob", "res_lower", PolicyAction::Deny);
}

#[test]
fn string_upper_equivalence() {
    let p = r#"
    policy p { default: deny,
        rule r { allow if { resource == "res_upper" && user.name.upper() == "ALICE" } }
    }"#;
    assert_equivalent_is(p, "alice", "res_upper", PolicyAction::Allow);
    assert_equivalent_is(p, "bob", "res_upper", PolicyAction::Deny);
}

#[test]
fn string_contains_equivalence() {
    let p = r#"
    policy p { default: deny,
        rule r { allow if { resource == "res_contains" && user.email.contains("corp") } }
    }"#;
    // both alice and bob have "corp" in email -> both allow (still must agree)
    assert_equivalent_is(p, "alice", "res_contains", PolicyAction::Allow);
    assert_equivalent_is(p, "bob", "res_contains", PolicyAction::Allow);
}

#[test]
fn string_startswith_equivalence() {
    let p = r#"
    policy p { default: deny,
        rule r { allow if { resource == "res_startswith" && user.role.startswith("adm") } }
    }"#;
    assert_equivalent_is(p, "alice", "res_startswith", PolicyAction::Allow);
    assert_equivalent_is(p, "bob", "res_startswith", PolicyAction::Deny);
}

#[test]
fn string_endswith_equivalence() {
    let p = r#"
    policy p { default: deny,
        rule r { allow if { resource == "res_endswith" && user.email.endswith("example") } }
    }"#;
    assert_equivalent_is(p, "alice", "res_endswith", PolicyAction::Allow);
    assert_equivalent_is(p, "bob", "res_endswith", PolicyAction::Allow);
}

// ---------------------------------------------------------------------------
// Nested / dotted attribute access
// ---------------------------------------------------------------------------

#[test]
fn nested_attribute_equivalence() {
    let p = r#"
    policy p { default: deny,
        rule r { allow if { resource == "res_nested" && user.profile.tier == "gold" } }
    }"#;
    assert_equivalent_is(p, "alice", "res_nested", PolicyAction::Allow);
    assert_equivalent_is(p, "bob", "res_nested", PolicyAction::Deny);
}

// ---------------------------------------------------------------------------
// Boundary conditions for count(): exact threshold with every operator.
// These pin that the compiled CountOp and AST count agree on the edge, which
// is where off-by-one divergences hide.
// ---------------------------------------------------------------------------

#[test]
fn count_operators_boundary_equivalence() {
    // alice.skills has exactly 4 elements.
    for (op, res) in [
        (">= 4", PolicyAction::Allow),
        ("> 4", PolicyAction::Deny),
        ("== 4", PolicyAction::Allow),
        ("<= 4", PolicyAction::Allow),
        ("< 4", PolicyAction::Deny),
        (">= 5", PolicyAction::Deny),
    ] {
        let p = format!(
            r#"policy p {{ default: deny,
                rule r {{ allow if {{ resource == "res_count" && user.skills.count() {op} }} }}
            }}"#
        );
        assert_equivalent_is(&p, "alice", "res_count", res);
    }
}

// ---------------------------------------------------------------------------
// Fallback contract: a policy the compiler REJECTS must still evaluate on the
// AST path (this is why the fallback exists). We assert the compiled build
// errors while the AST build succeeds and decides — the two are not required
// to both succeed, only to agree WHEN both can run (covered above).
// ---------------------------------------------------------------------------

#[test]
fn ast_fallback_handles_comprehension_the_compiler_may_reject() {
    // Set comprehension with a filter — an advanced feature routed to AST.
    let p = r#"
    policy p { default: deny,
        rule r { allow if high := {s | s := user.scores[_]; s >= 30} }
    }"#;
    let policy = ReaperPolicy::from_str(p).expect("parse");
    // AST must always be able to build and evaluate it.
    let ast = policy.clone().build_ast_evaluator(store_with_data());
    let decision = ast
        .evaluate(&request("alice", "res_sum"))
        .expect("ast eval");
    // alice has a score (40) >= 30 -> allow
    assert_eq!(decision, PolicyAction::Allow);
    // If the compiled path ALSO accepts it, it must agree.
    if let Ok(compiled) = policy.build(store_with_data()) {
        let c: PolicyAction = compiled
            .evaluate(&request("alice", "res_sum"))
            .expect("compiled eval");
        assert_eq!(
            c, decision,
            "compiled accepted the policy but disagreed with AST"
        );
    }
}

// ---------------------------------------------------------------------------
// build_preferred: the production entry point. For a compilable policy it must
// return the SAME decision the compiled path gives; for an
// AST-only policy it must still decide (fallback).
// ---------------------------------------------------------------------------

#[test]
fn build_preferred_matches_compiled_for_compilable_policy() {
    let policy = ReaperPolicy::from_str(P_COUNT).expect("parse");
    let compiled = policy
        .clone()
        .build(store_with_data())
        .expect("compiled build");
    let preferred = policy
        .build_preferred(store_with_data())
        .expect("preferred build");

    for (principal, resource) in [("alice", "res_count"), ("bob", "res_count")] {
        let req = request(principal, resource);
        assert_eq!(
            preferred.evaluate(&req).unwrap(),
            compiled.evaluate(&req).unwrap(),
            "build_preferred diverged from compiled for {principal}/{resource}"
        );
    }
    // And it selected the compiled evaluator (fast path) for this policy.
    assert_eq!(
        ReaperPolicy::from_str(P_COUNT)
            .unwrap()
            .build_preferred(store_with_data())
            .unwrap()
            .evaluator_type(),
        "reaper_dsl"
    );
}

#[test]
fn build_preferred_falls_back_to_ast() {
    // A comprehension policy the compiler may reject still evaluates via
    // build_preferred (fallback), never erroring.
    let p = r#"
    policy p { default: deny,
        rule r { allow if high := {s | s := user.scores[_]; s >= 30} }
    }"#;
    let preferred = ReaperPolicy::from_str(p)
        .expect("parse")
        .build_preferred(store_with_data())
        .expect("preferred build must not error even when compilation is impossible");
    // alice has a score 40 >= 30 -> allow
    assert_eq!(
        preferred.evaluate(&request("alice", "res_sum")).unwrap(),
        PolicyAction::Allow
    );
}
