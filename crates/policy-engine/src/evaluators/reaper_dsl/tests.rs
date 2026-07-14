//! Tests for the Reaper DSL evaluator.

use super::*;
use crate::EntityBuilder;
use std::collections::HashMap;

#[test]
fn test_reaper_dsl_simple_rule() {
    let store = Arc::new(DataStore::new());
    let interner = store.interner();

    // Create test entities
    let alice_id = interner.intern("alice");
    let user_type = interner.intern("User");
    let role_key = interner.intern("role");
    let admin_value = interner.intern("admin");

    let alice = EntityBuilder::new(alice_id, user_type)
        .with_string(role_key, admin_value)
        .build();

    let doc_id = interner.intern("doc1");
    let doc_type = interner.intern("Document");
    let doc = EntityBuilder::new(doc_id, doc_type).build();

    store.insert(alice);
    store.insert(doc);

    // Create policy: admin can do anything (using V2 type)
    let rules = vec![Rule {
        name: "admin_access".to_string(),
        condition: Condition::AttributeCompare(AttributeComparison {
            entity_type: EntityType::User,
            attribute: "role".to_string(),
            op: NumericOp::Equal,
            target: CompareTarget::LiteralString("admin".to_string()),
        }),
        decision: PolicyAction::Allow,
    }];

    let evaluator = ReaperDSLEvaluator::new(store, rules, PolicyAction::Deny);

    // Test evaluation
    let mut context = HashMap::new();
    context.insert("principal".to_string(), "alice".to_string());

    let request = PolicyRequest {
        resource: "doc1".to_string(),
        action: "read".to_string(),
        context,
    };

    let decision = evaluator.evaluate(&request).unwrap();
    assert!(matches!(decision, PolicyAction::Allow));
}

#[test]
fn test_reaper_dsl_complex_rule() {
    let store = Arc::new(DataStore::new());
    let interner = store.interner();

    // Create user
    let bob_id = interner.intern("bob");
    let user_type = interner.intern("User");
    let dept_key = interner.intern("department");
    let eng_value = interner.intern("engineering");

    let bob = EntityBuilder::new(bob_id, user_type)
        .with_string(dept_key, eng_value)
        .build();

    // Create resource
    let doc_id = interner.intern("doc2");
    let doc_type = interner.intern("Document");
    let doc = EntityBuilder::new(doc_id, doc_type)
        .with_string(dept_key, eng_value)
        .build();

    store.insert(bob);
    store.insert(doc);

    // Create policy: same department access (using V2 type)
    let rules = vec![Rule {
        name: "department_access".to_string(),
        condition: Condition::CrossEntityCompare(CrossEntityComparison {
            left_entity: EntityType::User,
            left_attr: "department".to_string(),
            op: NumericOp::Equal,
            right_entity: EntityType::Resource,
            right_attr: "department".to_string(),
        }),
        decision: PolicyAction::Allow,
    }];

    let evaluator = ReaperDSLEvaluator::new(store, rules, PolicyAction::Deny);

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "bob".to_string());

    let request = PolicyRequest {
        resource: "doc2".to_string(),
        action: "read".to_string(),
        context,
    };

    let decision = evaluator.evaluate(&request).unwrap();
    assert!(matches!(decision, PolicyAction::Allow));
}

// ===========================================================================
// D2: resource_index_terms — compiled DSL resource-literal extraction
// ===========================================================================

/// Build a compiled DSL evaluator over a store with a single admin user, from
/// hand-built rules (deterministic — no parser dependency).
fn ridx_evaluator(rules: Vec<Rule>, default: PolicyAction) -> (Arc<DataStore>, ReaperDSLEvaluator) {
    let store = Arc::new(DataStore::new());
    let interner = store.interner();
    let alice_id = interner.intern("alice");
    let user_type = interner.intern("User");
    let role_key = interner.intern("role");
    let admin_value = interner.intern("admin");
    let alice = EntityBuilder::new(alice_id, user_type)
        .with_string(role_key, admin_value)
        .build();
    store.insert(alice);
    let evaluator = ReaperDSLEvaluator::new(store.clone(), rules, default);
    (store, evaluator)
}

fn resource_eq(value: &str) -> Condition {
    Condition::ResourceIdEquals {
        value: value.to_string(),
    }
}

fn resource_type_eq(value: &str) -> Condition {
    Condition::AttributeCompare(AttributeComparison {
        entity_type: EntityType::Resource,
        attribute: "type".to_string(),
        op: NumericOp::Equal,
        target: CompareTarget::LiteralString(value.to_string()),
    })
}

fn user_role_eq(value: &str) -> Condition {
    Condition::AttributeCompare(AttributeComparison {
        entity_type: EntityType::User,
        attribute: "role".to_string(),
        op: NumericOp::Equal,
        target: CompareTarget::LiteralString(value.to_string()),
    })
}

fn allow_rule(name: &str, condition: Condition) -> Rule {
    Rule {
        name: name.to_string(),
        condition,
        decision: PolicyAction::Allow,
    }
}

#[test]
fn test_ridx_single_literal() {
    let (_s, ev) = ridx_evaluator(
        vec![allow_rule("r", resource_eq("doc1"))],
        PolicyAction::Deny,
    );
    assert_eq!(ev.resource_index_terms(), Some(vec!["doc1".to_string()]));
}

#[test]
fn test_ridx_multiple_rules_union() {
    let (_s, ev) = ridx_evaluator(
        vec![
            allow_rule("a", resource_eq("a")),
            allow_rule("b", resource_eq("b")),
            allow_rule("c", resource_eq("c")),
        ],
        PolicyAction::Deny,
    );
    assert_eq!(
        ev.resource_index_terms(),
        Some(vec!["a".to_string(), "b".to_string(), "c".to_string()])
    );
}

#[test]
fn test_ridx_attribute_predicate_is_unprunable() {
    // resource.type == "invoice" is an ATTRIBUTE, not the request resource id.
    let (_s, ev) = ridx_evaluator(
        vec![allow_rule("r", resource_type_eq("invoice"))],
        PolicyAction::Deny,
    );
    assert_eq!(ev.resource_index_terms(), None);
}

#[test]
fn test_ridx_or_of_resource_literals() {
    // resource == "x" || resource == "y" -> both branches bound the resource id.
    let (_s, ev) = ridx_evaluator(
        vec![allow_rule(
            "r",
            Condition::Or(vec![resource_eq("x"), resource_eq("y")]),
        )],
        PolicyAction::Deny,
    );
    assert_eq!(
        ev.resource_index_terms(),
        Some(vec!["x".to_string(), "y".to_string()])
    );
}

#[test]
fn test_ridx_or_mixed_is_unprunable() {
    // resource == "x" || user.role == "admin" — the second branch can match any
    // resource, so the Or is unbounded.
    let (_s, ev) = ridx_evaluator(
        vec![allow_rule(
            "r",
            Condition::Or(vec![resource_eq("x"), user_role_eq("admin")]),
        )],
        PolicyAction::Deny,
    );
    assert_eq!(ev.resource_index_terms(), None);
}

#[test]
fn test_ridx_and_with_attribute_stays_bounded() {
    // resource == "doc1" && user.role == "admin" — the And can only match doc1.
    let (_s, ev) = ridx_evaluator(
        vec![allow_rule(
            "r",
            Condition::And(vec![resource_eq("doc1"), user_role_eq("admin")]),
        )],
        PolicyAction::Deny,
    );
    assert_eq!(ev.resource_index_terms(), Some(vec!["doc1".to_string()]));
}

#[test]
fn test_ridx_no_resource_constraint_is_unprunable() {
    // A rule that only constrains the user matches ANY resource.
    let (_s, ev) = ridx_evaluator(
        vec![allow_rule("r", user_role_eq("admin"))],
        PolicyAction::Deny,
    );
    assert_eq!(ev.resource_index_terms(), None);
}

#[test]
fn test_ridx_always_is_unprunable() {
    let (_s, ev) = ridx_evaluator(vec![allow_rule("r", Condition::Always)], PolicyAction::Deny);
    assert_eq!(ev.resource_index_terms(), None);
}

#[test]
fn test_ridx_negation_is_unprunable() {
    // Not(resource == "x") matches every resource except x -> unbounded.
    let (_s, ev) = ridx_evaluator(
        vec![allow_rule("r", Condition::Not(Box::new(resource_eq("x"))))],
        PolicyAction::Deny,
    );
    assert_eq!(ev.resource_index_terms(), None);
}

#[test]
fn test_ridx_false_rule_contributes_no_terms() {
    // Not(Always) == `false`: the rule never fires, so it adds no terms and
    // does NOT poison the policy.
    let (_s, ev) = ridx_evaluator(
        vec![
            allow_rule("dead", Condition::Not(Box::new(Condition::Always))),
            allow_rule("live", resource_eq("doc1")),
        ],
        PolicyAction::Deny,
    );
    assert_eq!(ev.resource_index_terms(), Some(vec!["doc1".to_string()]));
}

#[test]
fn test_ridx_deny_rule_literal_bounded() {
    // A deny rule bounded to a literal, with default allow: the policy is still
    // only decisive for that literal (default is non-decisive at set level).
    let (_s, ev) = ridx_evaluator(
        vec![Rule {
            name: "block".to_string(),
            condition: resource_eq("secret"),
            decision: PolicyAction::Deny,
        }],
        PolicyAction::Allow,
    );
    assert_eq!(ev.resource_index_terms(), Some(vec!["secret".to_string()]));
}

/// SOUNDNESS DIFFERENTIAL (required): for every policy whose
/// `resource_index_terms()` is `Some(v)`, any resource NOT in `v` must make the
/// evaluator NON-DECISIVE (`evaluate_matched(..).1 == false`). This is the
/// promise the pruning index relies on; a violation would be fail-open pruning.
#[test]
fn test_ridx_soundness_differential() {
    let policies: Vec<Vec<Rule>> = vec![
        vec![allow_rule("a", resource_eq("doc1"))],
        vec![
            allow_rule("a", resource_eq("doc1")),
            allow_rule("b", resource_eq("x")),
        ],
        vec![allow_rule(
            "a",
            Condition::Or(vec![resource_eq("x"), resource_eq("y")]),
        )],
        vec![allow_rule(
            "a",
            Condition::And(vec![resource_eq("doc1"), user_role_eq("admin")]),
        )],
        vec![Rule {
            name: "block".to_string(),
            condition: resource_eq("secret"),
            decision: PolicyAction::Deny,
        }],
        vec![
            allow_rule("dead", Condition::Not(Box::new(Condition::Always))),
            allow_rule("live", resource_eq("doc1")),
        ],
    ];

    let probes = [
        "doc1", "doc2", "x", "y", "secret", "random", "", "DOC1", "z",
    ];

    for rules in policies {
        let default = if rules
            .iter()
            .any(|r| matches!(r.decision, PolicyAction::Deny))
        {
            PolicyAction::Allow
        } else {
            PolicyAction::Deny
        };
        let (_s, ev) = ridx_evaluator(rules, default);

        let Some(terms) = ev.resource_index_terms() else {
            continue; // unprunable: no promise to check
        };

        for probe in probes {
            if terms.iter().any(|t| t == probe) {
                continue;
            }
            let mut context = HashMap::new();
            context.insert("principal".to_string(), "alice".to_string());
            let request = PolicyRequest {
                resource: probe.to_string(),
                action: "read".to_string(),
                context,
            };
            let (_action, matched) = ev.evaluate_matched(&request).unwrap();
            assert!(
                !matched,
                "FAIL-OPEN PRUNING: resource {:?} is outside index terms {:?} but the \
                 evaluator matched it (decisive).",
                probe, terms
            );
        }
    }
}

/// D2 fallback safety: a DSL policy the COMPILER REJECTS runs on the
/// `ReapAstEvaluator` fallback (via `ReaperPolicy::build_preferred`), and that
/// fallback reports `resource_index_terms() == None` — i.e. it is UNPRUNABLE.
/// An unprunable policy is always a candidate for every resource, so the
/// pruning index can never drop it. This is the safe default for constructs the
/// compiler does not yet handle.
///
/// The chosen fallback construct is `input.*` document access: it parses and
/// evaluates on the AST interpreter, but the compiler rejects it (see
/// `reap/compiler/comparison/entity.rs`: "`input` document access is not
/// compiled yet; policy runs on the AST evaluator"). So `build_preferred`
/// returns a LIVE fallback evaluator here — asserted via `evaluator_type()` —
/// not the compiled path, making this a real compile-fallback policy rather
/// than a direct default-None assertion.
#[test]
fn test_ast_fallback_policy_is_unprunable() {
    let store = Arc::new(DataStore::new());
    let content = "policy p {\n    default: deny,\n    rule r {\n        allow if input.tenant == \"acme\"\n    }\n}";
    let policy: crate::reap::ReaperPolicy = content
        .parse()
        .expect("policy with `input` access should parse");
    let evaluator = policy
        .build_preferred(store)
        .expect("build_preferred falls back to the AST evaluator for `input` access");

    // Confirm we exercised the real fallback, not a compiled evaluator.
    assert_eq!(
        evaluator.evaluator_type(),
        "ReapAstEvaluator",
        "compiler must reject `input` access and fall back to the AST evaluator"
    );

    // The fallback is UNPRUNABLE: no resource-index terms -> always a candidate.
    assert_eq!(
        evaluator.resource_index_terms(),
        None,
        "an AST-fallback DSL policy must be unprunable so pruning can never drop it"
    );
}
