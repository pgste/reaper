//! Compiled `input`-comparison differential tests (R4-01 Phase B.1's merge
//! gate — design COMPILED_INPUT_DESIGN.md §4 gates 1 and 3).
//!
//! Contract: for every policy shape B.1 compiles, the compiled evaluator's
//! `evaluate_with_input_named` must return the same decision AND deciding
//! rule name as the interpreter's, over a document matrix that exercises
//! the whole truth table: present/missing paths, wrong-typed nodes, JSON
//! null vs absent, arrays (existential equality), int/float strictness,
//! and no document at all. Plus fallback pins: the shapes B.1 deliberately
//! does NOT compile must still fail whole-policy compilation.

use policy_engine::data::{DataLoader, DataStore};
use policy_engine::reap::ReaperPolicy;
use policy_engine::{PolicyAction, PolicyRequest};
use serde_json::json;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

fn store() -> Arc<DataStore> {
    let store = Arc::new(DataStore::new());
    let loader = DataLoader::new((*store).clone());
    loader
        .load_json(
            &json!({"entities": [
                {"id": "alice", "type": "user", "attributes": {"role": "admin"}}
            ]})
            .to_string(),
        )
        .expect("load");
    store
}

fn request(principal: Option<&str>) -> PolicyRequest {
    let mut context = HashMap::new();
    if let Some(p) = principal {
        context.insert("principal".to_string(), p.to_string());
    }
    PolicyRequest {
        resource: "res".to_string(),
        action: "check".to_string(),
        context,
        ..Default::default()
    }
}

/// Build compiled + AST evaluators; assert the policy COMPILES whole and
/// both paths agree (decision + rule name) for (principal, doc).
fn assert_input_equivalent(
    policy_text: &str,
    principal: Option<&str>,
    doc: Option<&serde_json::Value>,
) -> PolicyAction {
    let policy = ReaperPolicy::from_str(policy_text).expect("parse");
    let compiled = policy
        .clone()
        .build(store())
        .expect("policy must compile whole — that is the point of B.1");
    let ast = policy.build_ast_evaluator(store());

    let req = request(principal);
    let (c_dec, c_name) = compiled
        .evaluate_with_input_named(&req, doc)
        .expect("compiled evaluate");
    let (a_dec, a_name) = ast
        .evaluate_with_input_named(&req, doc)
        .expect("ast evaluate");

    assert_eq!(
        c_dec, a_dec,
        "decision diverged (principal={principal:?})\ndoc: {doc:?}\npolicy:\n{policy_text}"
    );
    assert_eq!(
        c_name, a_name,
        "rule name diverged (principal={principal:?})\ndoc: {doc:?}\npolicy:\n{policy_text}"
    );
    c_dec
}

/// The k8s library shape: presence check on a deep path.
const OWNER_LABEL: &str = r#"
policy owner_label {
    default: allow,
    rule owner_label_required {
        deny if input.request.object.metadata.labels.owner == null
    }
}
"#;

#[test]
fn owner_label_rule_compiles_and_matches_ast_over_document_matrix() {
    let labelled = json!({"request": {"object": {"metadata": {"labels": {"owner": "team-a"}}}}});
    let unlabelled = json!({"request": {"object": {"metadata": {"labels": {"env": "prod"}}}}});
    let no_labels = json!({"request": {"object": {"metadata": {}}}});
    let null_label = json!({"request": {"object": {"metadata": {"labels": {"owner": null}}}}});
    let wrong_shape = json!({"request": "not-an-object"});

    assert_eq!(
        assert_input_equivalent(OWNER_LABEL, None, Some(&labelled)),
        PolicyAction::Allow
    );
    for doc in [&unlabelled, &no_labels, &null_label, &wrong_shape] {
        assert_eq!(
            assert_input_equivalent(OWNER_LABEL, None, Some(doc)),
            PolicyAction::Deny,
            "absent/null/malformed owner label must deny: {doc}"
        );
    }
    // No document at all: the whole access is Null ⇒ == null matches ⇒ deny.
    assert_eq!(
        assert_input_equivalent(OWNER_LABEL, None, None),
        PolicyAction::Deny
    );
}

#[test]
fn scalar_truth_table_matches_ast() {
    // One policy per (op, literal) shape; documents exercise type
    // strictness, absence, arrays, and numeric edges. Every case asserts
    // only EQUIVALENCE (the AST is the oracle); decision values are pinned
    // by the owner-label test above and spot pins below.
    let cases: Vec<(&str, serde_json::Value)> = vec![
        // String equality + type mismatch + array existential.
        (r#"input.env == "prod""#, json!({"env": "prod"})),
        (r#"input.env == "prod""#, json!({"env": "staging"})),
        (r#"input.env == "prod""#, json!({"env": 7})),
        (r#"input.env == "prod""#, json!({"tags": ["x"]})),
        (
            r#"input.tags == "beta""#,
            json!({"tags": ["alpha", "beta"]}),
        ),
        (r#"input.tags == "beta""#, json!({"tags": ["alpha"]})),
        (
            r#"input.tags == "beta""#,
            json!({"tags": [["nested", "beta"]]}),
        ),
        // != with absence: missing must NOT satisfy the inequality.
        (r#"input.env != "prod""#, json!({"env": "staging"})),
        (r#"input.env != "prod""#, json!({"other": 1})),
        // Int/float strictness: JSON 7 is an Integer, never equal to 7.0.
        (r#"input.count == 7"#, json!({"count": 7})),
        (r#"input.count == 7"#, json!({"count": 7.0})),
        (r#"input.count == 7.0"#, json!({"count": 7})),
        (r#"input.ratio == 0.5"#, json!({"ratio": 0.5})),
        // Ordered: numeric only; arrays/strings/bools never order.
        (r#"input.count > 3"#, json!({"count": 7})),
        (r#"input.count > 3"#, json!({"count": 2})),
        (r#"input.count > 3"#, json!({"count": "7"})),
        (r#"input.count > 3"#, json!({"count": [7]})),
        (r#"input.count >= 7.5"#, json!({"count": 7.5})),
        (r#"input.count <= 3"#, json!({"count": 3})),
        (r#"input.count < 3"#, json!({})),
        // Bool.
        (r#"input.flag == true"#, json!({"flag": true})),
        (r#"input.flag == true"#, json!({"flag": false})),
        (r#"input.flag == true"#, json!({"flag": "true"})),
        (r#"input.flag != false"#, json!({"flag": true})),
        (r#"input.flag != false"#, json!({})),
        // Deep path through wrong-typed intermediate.
        (r#"input.a.b.c == 1"#, json!({"a": {"b": {"c": 1}}})),
        (r#"input.a.b.c == 1"#, json!({"a": {"b": 5}})),
        (r#"input.a.b.c == 1"#, json!({"a": null})),
        // Null presence checks.
        (r#"input.a.b == null"#, json!({"a": {}})),
        (r#"input.a.b == null"#, json!({"a": {"b": null}})),
        (r#"input.a.b == null"#, json!({"a": {"b": 0}})),
        (r#"input.a.b != null"#, json!({"a": {"b": 0}})),
        (r#"input.a.b != null"#, json!({"a": {}})),
        // Numbers beyond i64 (JSON u64 ⇒ Float on both paths).
        (
            r#"input.big == 9223372036854775807"#,
            json!({"big": 9223372036854775807i64}),
        ),
        (r#"input.big > 0"#, json!({"big": 18446744073709551615u64})),
    ];

    for (cond, doc) in &cases {
        let policy =
            format!("policy m {{\n    default: deny,\n    rule r {{ allow if {cond} }}\n}}");
        assert_input_equivalent(&policy, None, Some(doc));
        // And with no document, for every shape.
        assert_input_equivalent(&policy, None, None);
    }
}

#[test]
fn hybrid_entity_and_input_rule_compiles_and_agrees() {
    // Entity condition + input condition in ONE rule: the whole policy
    // compiles, and principal-loaded vs principal-absent both agree with
    // the interpreter (the with-input entry's no-principal relaxation).
    let p = r#"
policy hybrid {
    default: deny,
    rule admin_in_prod {
        allow if {
            user.role == "admin" &&
            input.env == "prod"
        }
    }
}
"#;
    let prod = json!({"env": "prod"});
    let staging = json!({"env": "staging"});

    assert_eq!(
        assert_input_equivalent(p, Some("alice"), Some(&prod)),
        PolicyAction::Allow
    );
    assert_eq!(
        assert_input_equivalent(p, Some("alice"), Some(&staging)),
        PolicyAction::Deny
    );
    // Unknown / absent principal: user.role reads Null on both paths.
    assert_eq!(
        assert_input_equivalent(p, Some("nobody"), Some(&prod)),
        PolicyAction::Deny
    );
    assert_eq!(
        assert_input_equivalent(p, None, Some(&prod)),
        PolicyAction::Deny
    );
}

#[test]
fn mixed_policy_input_rule_runs_compiled_side() {
    // An input rule + a still-uncompilable rule: per-rule fallback serves
    // the policy mixed, and the with-input entry agrees with the whole-AST
    // interpreter.
    let p = r#"
policy mixed_input {
    default: allow,
    rule owner_required {
        deny if input.metadata.owner == null
    }
    rule float_trigger_ast {
        deny if { cap := 8.5 && user.clearance > cap }
    }
}
"#;
    let policy = ReaperPolicy::from_str(p).expect("parse");
    let preferred = policy.clone().build_preferred(store()).expect("build");
    assert_eq!(preferred.evaluator_type(), "reaper_dsl_mixed");

    // Reach the mixed evaluator's with-input entry directly (the parser's
    // public `parse` yields the AST `MixedReapEvaluator::build` takes).
    let ast_policy = policy_engine::reap::ReapParser::parse(p).expect("parse ast");
    let mixed = match policy_engine::reap::MixedReapEvaluator::build(ast_policy, store())
        .expect("mixed build")
    {
        policy_engine::reap::PerRuleBuild::Mixed(m) => m,
        other => panic!("expected mixed, got {other:?}"),
    };
    let ast = ReaperPolicy::from_str(p)
        .expect("parse")
        .build_ast_evaluator(store());

    let with_owner = json!({"metadata": {"owner": "t"}});
    let without_owner = json!({"metadata": {}});
    for doc in [Some(&with_owner), Some(&without_owner), None] {
        let req = request(None);
        let (m_dec, m_name) = mixed
            .evaluate_with_input_named(&req, doc)
            .expect("mixed evaluate");
        let (a_dec, a_name) = ast
            .evaluate_with_input_named(&req, doc)
            .expect("ast evaluate");
        assert_eq!(m_dec, a_dec, "mixed/AST decision diverged for {doc:?}");
        assert_eq!(m_name, a_name, "mixed/AST rule name diverged for {doc:?}");
    }
}

#[test]
fn uncompiled_input_shapes_still_fall_back() {
    // The B.1 fence: these shapes must NOT compile whole (per-rule fallback
    // serves them) — each is either B.2 scope or an interpreter-error
    // contract the compiled path must not absorb.
    for (label, cond) in [
        ("indexed input access", r#"input.items[0] == "x""#),
        ("wildcard input access", r#"input.items[_] == "x""#),
        ("ordered vs null", r#"input.count > null"#),
        (
            "input comprehension",
            r#"{ bad := [c | c := input.items[_]; c.x == 1] && bad.count() > 0 }"#,
        ),
    ] {
        let policy =
            format!("policy f {{\n    default: deny,\n    rule r {{ allow if {cond} }}\n}}");
        let parsed = ReaperPolicy::from_str(&policy).expect("parse");
        assert!(
            parsed.build(store()).is_err(),
            "{label} unexpectedly compiled — widen the B.1 differential before \
             removing this pin"
        );
    }
}
