//! DSL v2 Phase 1+2: structured `input` documents + check-mode violations.
//!
//! Proves the OPA-style story end to end: a Terraform-plan-like JSON document
//! evaluated by a .reap policy, with all matching deny rules collected as
//! human-readable violations — while decision-mode evaluation stays untouched.

use policy_engine::reap::ReaperPolicy;
use policy_engine::{DataStore, PolicyRequest};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

fn tf_plan() -> serde_json::Value {
    json!({
        "format_version": "1.2",
        "resource_changes": [
            {
                "address": "aws_s3_bucket.logs",
                "type": "aws_s3_bucket",
                "name": "logs",
                "change": { "actions": ["create"], "after": { "acl": "public-read", "bucket": "logs" } }
            },
            {
                "address": "aws_s3_bucket.data",
                "type": "aws_s3_bucket",
                "name": "data",
                "change": { "actions": ["create"], "after": { "acl": "private", "bucket": "data" } }
            },
            {
                "address": "aws_instance.web",
                "type": "aws_instance",
                "name": "web",
                "change": { "actions": ["create"], "after": { "instance_type": "t3.2xlarge" } }
            }
        ]
    })
}

const TF_POLICY: &str = r#"
policy terraform_guard {
    default: allow,

    rule no_public_buckets {
        deny with message concat("public S3 bucket: ", first) if {
            bad := [rc.name | rc := input.resource_changes[_]; rc.type == "aws_s3_bucket"; rc.change.after.acl == "public-read"] &&
            first := bad[0] &&
            bad.count() > 0
        }
    }

    rule no_big_instances {
        deny with message "instance type too large" if {
            big := [rc | rc := input.resource_changes[_]; rc.type == "aws_instance"; rc.change.after.instance_type == "t3.2xlarge"] &&
            big.count() > 0
        }
    }

    rule never_fires {
        deny with message "unused" if {
            gone := [rc | rc := input.resource_changes[_]; rc.type == "aws_eip"] &&
            gone.count() > 0
        }
    }
}
"#;

fn evaluator(policy: &str) -> policy_engine::reap::ReapAstEvaluator {
    let parsed: ReaperPolicy = policy.parse().expect("parse policy");
    parsed.build_ast_evaluator(Arc::new(DataStore::new()))
}

fn doc_request() -> PolicyRequest {
    PolicyRequest {
        resource: "plan".to_string(),
        action: "apply".to_string(),
        context: HashMap::new(),

        ..Default::default()
    }
}

#[test]
fn check_collects_all_violations_with_messages() {
    let eval = evaluator(TF_POLICY);
    let result = eval
        .check_with_input(&doc_request(), Some(&tf_plan()))
        .expect("check");

    assert!(!result.allowed);
    assert_eq!(
        result.violations.len(),
        2,
        "both real violations, not just first"
    );

    let rules: Vec<&str> = result.violations.iter().map(|v| v.rule.as_str()).collect();
    assert!(rules.contains(&"no_public_buckets"));
    assert!(rules.contains(&"no_big_instances"));
    assert!(!rules.contains(&"never_fires"));

    let msg = result
        .violations
        .iter()
        .find(|v| v.rule == "no_public_buckets")
        .and_then(|v| v.message.as_deref())
        .expect("message rendered");
    assert_eq!(
        msg, "public S3 bucket: logs",
        "message renders rule-bound variables"
    );
}

#[test]
fn clean_plan_is_allowed() {
    let eval = evaluator(TF_POLICY);
    let clean = json!({ "resource_changes": [
        { "address": "aws_s3_bucket.data", "type": "aws_s3_bucket", "name": "data",
          "change": { "actions": ["create"], "after": { "acl": "private" } } }
    ]});
    let result = eval.check_with_input(&doc_request(), Some(&clean)).unwrap();
    assert!(result.allowed);
    assert!(result.violations.is_empty());
}

#[test]
fn missing_input_document_yields_null_not_error() {
    // No document at all: rules over `input` just don't match.
    let eval = evaluator(TF_POLICY);
    let result = eval.check_with_input(&doc_request(), None).unwrap();
    assert!(result.allowed);
    assert!(result.violations.is_empty());
}

#[test]
fn decision_mode_first_match_still_works_with_input() {
    // evaluate_with_input keeps decision semantics (deny wins, first match).
    let eval = evaluator(TF_POLICY);
    let action = eval
        .evaluate_with_input(&doc_request(), Some(&tf_plan()))
        .unwrap();
    assert_eq!(format!("{action:?}"), "Deny");

    let clean = json!({ "resource_changes": [] });
    let action = eval
        .evaluate_with_input(&doc_request(), Some(&clean))
        .unwrap();
    assert_eq!(format!("{action:?}"), "Allow");
}

#[test]
fn deep_paths_and_partial_documents_are_total() {
    // Malformed/partial documents must fail rules, never crash evaluation.
    let policy = r#"
policy deep {
    default: allow,
    rule deep_check {
        deny if {
            hits := [x | x := input.a[_]; x.b.c.d == "boom"] &&
            hits.count() > 0
        }
    }
}
"#;
    let eval = evaluator(policy);

    // matching nested doc
    let doc = json!({ "a": [ { "b": { "c": { "d": "boom" } } } ] });
    let r = eval.check_with_input(&doc_request(), Some(&doc)).unwrap();
    assert!(!r.allowed);

    // partial: missing intermediate keys — total evaluation, no error
    let doc = json!({ "a": [ { "b": {} }, { "unrelated": 1 } ] });
    let r = eval.check_with_input(&doc_request(), Some(&doc)).unwrap();
    assert!(r.allowed, "missing path = Null = rule fails, not error");
}

#[test]
fn default_deny_policy_requires_an_allow_rule_in_check_mode() {
    let policy = r#"
policy strict {
    default: deny,
    rule allow_reads {
        allow if context.action == "read"
    }
}
"#;
    let eval = evaluator(policy);

    let mut req = doc_request();
    req.action = "read".to_string();
    assert!(eval.check_with_input(&req, None).unwrap().allowed);

    let mut req = doc_request();
    req.action = "write".to_string();
    let r = eval.check_with_input(&req, None).unwrap();
    assert!(!r.allowed, "default deny with no matching allow");
    assert!(
        r.violations.is_empty(),
        "not a rule violation, just not allowed"
    );
}

#[test]
fn input_policy_falls_back_to_ast_via_engine_deploy() {
    // Deploying through the engine must not fail: the compiled path rejects
    // `input` with a hint and the engine transparently uses the AST evaluator.
    use policy_engine::{EnhancedPolicy, PolicyEngine, PolicyLanguage};
    let engine = PolicyEngine::new();
    let mut policy = EnhancedPolicy::new("tfguard".to_string(), String::new(), vec![]);
    policy.language = PolicyLanguage::ReaperDsl;
    policy.content = TF_POLICY.to_string();
    policy
        .build_evaluator_with_data(Some(Arc::new(DataStore::new())))
        .expect("deploy with AST fallback");
    engine.deploy_policy(policy).expect("deploy");
}

#[test]
fn message_is_optional_and_decision_mode_ignores_it() {
    let policy = r#"
policy plain {
    default: allow,
    rule block {
        deny if input.flag == true
    }
}
"#;
    let eval = evaluator(policy);
    let r = eval
        .check_with_input(&doc_request(), Some(&json!({"flag": true})))
        .unwrap();
    assert!(!r.allowed);
    assert_eq!(r.violations[0].rule, "block");
    assert!(r.violations[0].message.is_none());
}
