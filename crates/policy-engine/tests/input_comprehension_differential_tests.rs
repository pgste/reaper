//! Compiled input-comprehension differential tests (R4-01 Phase B.2a).
//!
//! Contract: comprehension rules iterating `input.<path>[_]` that now
//! compile must produce identical decisions AND deciding rule names to the
//! interpreter, over document matrices covering: matching/non-matching
//! elements, dotted-path filters into nested objects (the navigation-parity
//! risk), null-comparison filters, empty arrays, missing paths, non-array
//! nodes, and no document at all. The policies are the real library shapes
//! (kubernetes admission / terraform guardrails), not synthetic ones.

use policy_engine::data::DataStore;
use policy_engine::reap::ReaperPolicy;
use policy_engine::{PolicyAction, PolicyRequest};
use serde_json::json;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

fn store() -> Arc<DataStore> {
    Arc::new(DataStore::new())
}

fn request() -> PolicyRequest {
    PolicyRequest {
        resource: "res".to_string(),
        action: "check".to_string(),
        context: HashMap::new(),
        ..Default::default()
    }
}

fn assert_equivalent(
    policy_text: &str,
    doc: Option<&serde_json::Value>,
    expect_compiled: bool,
) -> PolicyAction {
    let policy = ReaperPolicy::from_str(policy_text).expect("parse");
    let ast = policy.clone().build_ast_evaluator(store());
    let (a_dec, a_name) = ast
        .evaluate_with_input_named(&request(), doc)
        .expect("ast evaluate");

    if expect_compiled {
        let compiled = policy
            .build(store())
            .expect("policy must compile whole (B.2a)");
        let (c_dec, c_name) = compiled
            .evaluate_with_input_named(&request(), doc)
            .expect("compiled evaluate");
        assert_eq!(
            c_dec, a_dec,
            "decision diverged\ndoc: {doc:?}\n{policy_text}"
        );
        assert_eq!(
            c_name, a_name,
            "rule name diverged\ndoc: {doc:?}\n{policy_text}"
        );
    }
    a_dec
}

/// The k8s `no_privileged_containers` shape: comprehension + dotted-path
/// filter into a nested object + count guard. THE navigation-parity case.
const PRIVILEGED: &str = r#"
policy k8s_privileged {
    default: allow,
    rule no_privileged_containers {
        deny if {
            bad := [c.name | c := input.request.object.spec.containers[_]; c.securityContext.privileged == true] &&
            bad.count() > 0
        }
    }
}
"#;

#[test]
fn privileged_containers_rule_compiles_and_matches_ast() {
    let privileged = json!({"request": {"object": {"spec": {"containers": [
        {"name": "app", "securityContext": {"privileged": false}},
        {"name": "sidecar", "securityContext": {"privileged": true}}
    ]}}}});
    let clean = json!({"request": {"object": {"spec": {"containers": [
        {"name": "app", "securityContext": {"privileged": false}},
        {"name": "plain"}
    ]}}}});
    let empty = json!({"request": {"object": {"spec": {"containers": []}}}});
    let missing = json!({"request": {"object": {"spec": {}}}});
    let non_array = json!({"request": {"object": {"spec": {"containers": "oops"}}}});

    assert_eq!(
        assert_equivalent(PRIVILEGED, Some(&privileged), true),
        PolicyAction::Deny
    );
    for doc in [&clean, &empty, &missing, &non_array] {
        assert_eq!(
            assert_equivalent(PRIVILEGED, Some(doc), true),
            PolicyAction::Allow,
            "no privileged container ⇒ allow: {doc}"
        );
    }
    assert_eq!(
        assert_equivalent(PRIVILEGED, None, true),
        PolicyAction::Allow
    );
}

/// The k8s `resource_limits_required` shape: null-comparison filter on a
/// dotted element path.
const LIMITS: &str = r#"
policy k8s_limits {
    default: allow,
    rule resource_limits_required {
        deny if {
            bad := [c.name | c := input.request.object.spec.containers[_]; c.resources.limits == null] &&
            bad.count() > 0
        }
    }
}
"#;

#[test]
fn resource_limits_rule_compiles_and_matches_ast() {
    let limited = json!({"request": {"object": {"spec": {"containers": [
        {"name": "app", "resources": {"limits": {"cpu": "1"}}}
    ]}}}});
    let unlimited = json!({"request": {"object": {"spec": {"containers": [
        {"name": "app", "resources": {"limits": {"cpu": "1"}}},
        {"name": "bare", "resources": {}}
    ]}}}});
    let no_resources = json!({"request": {"object": {"spec": {"containers": [
        {"name": "naked"}
    ]}}}});

    assert_eq!(
        assert_equivalent(LIMITS, Some(&limited), true),
        PolicyAction::Allow
    );
    for doc in [&unlimited, &no_resources] {
        assert_eq!(
            assert_equivalent(LIMITS, Some(doc), true),
            PolicyAction::Deny,
            "container without limits ⇒ deny: {doc}"
        );
    }
}

/// The terraform shape: single-segment type filter + dotted after-state
/// filter, element-path output, first-element bind.
const TF_ACL: &str = r#"
policy tf_acl {
    default: allow,
    rule no_public_buckets {
        deny if {
            bad := [rc.name | rc := input.resource_changes[_]; rc.type == "aws_s3_bucket"; rc.change.after.acl == "public-read"] &&
            bad.count() > 0
        }
    }
}
"#;

#[test]
fn terraform_acl_rule_compiles_and_matches_ast() {
    let public = json!({"resource_changes": [
        {"name": "logs", "type": "aws_s3_bucket", "change": {"after": {"acl": "public-read"}}},
        {"name": "web", "type": "aws_instance", "change": {"after": {"acl": "public-read"}}}
    ]});
    let private = json!({"resource_changes": [
        {"name": "data", "type": "aws_s3_bucket", "change": {"after": {"acl": "private"}}}
    ]});
    // Same acl on a NON-bucket type: the type filter must gate it out.
    let non_bucket = json!({"resource_changes": [
        {"name": "web", "type": "aws_instance", "change": {"after": {"acl": "public-read"}}}
    ]});

    assert_eq!(
        assert_equivalent(TF_ACL, Some(&public), true),
        PolicyAction::Deny
    );
    for doc in [&private, &non_bucket] {
        assert_eq!(
            assert_equivalent(TF_ACL, Some(doc), true),
            PolicyAction::Allow
        );
    }
    assert_eq!(assert_equivalent(TF_ACL, None, true), PolicyAction::Allow);
}

#[test]
fn library_policies_reach_expected_mixed_split() {
    // Pin the per-rule compile coverage of the REAL library policies so a
    // regression in any lowering shows up as a changed split, loudly.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../policy-library");
    for (rel, want_compiled, want_ast) in [
        ("kubernetes/admission-control/policy.reap", 5, 0),
        ("terraform/s3-guardrails/policy.reap", 4, 0),
    ] {
        let path = root.join(rel);
        let src = std::fs::read_to_string(&path).expect("read library policy");
        let ast = policy_engine::reap::ReapParser::parse(&src).expect("parse");
        match policy_engine::reap::MixedReapEvaluator::build(ast, store()).expect("build") {
            policy_engine::reap::PerRuleBuild::Mixed(m) => {
                let (c, a) = m.rule_modes();
                assert_eq!(
                    (c, a),
                    (want_compiled, want_ast),
                    "{rel}: per-rule split changed (ast rules: {:?}) — if MORE \
                     rules compile now, update this pin upward; fewer is a \
                     regression",
                    m.ast_rule_names()
                );
            }
            other => panic!("{rel}: expected mixed, got {other:?}"),
        }
    }
}

#[test]
fn indexed_input_comprehension_source_still_falls_back() {
    let p = r#"
policy f {
    default: deny,
    rule r {
        allow if {
            xs := [c | c := input.items[0]; c.x == 1] &&
            xs.count() > 0
        }
    }
}
"#;
    let policy = ReaperPolicy::from_str(p).expect("parse");
    assert!(
        policy.build(store()).is_err(),
        "indexed comprehension source unexpectedly compiled — extend the \
         differential before removing this pin"
    );
}

/// B.2b: string-op filters (endswith / negated startswith) — the remaining
/// k8s rule shapes — plus var-attr membership (the remaining terraform
/// shape). Full-policy differentials over realistic documents.
const LATEST_TAG: &str = r#"
policy k8s_latest {
    default: allow,
    rule disallow_latest_tag {
        deny if {
            bad := [c.image | c := input.request.object.spec.containers[_]; c.image.endswith(":latest")] &&
            bad.count() > 0
        }
    }
    rule approved_registries_only {
        deny if {
            bad := [c.image | c := input.request.object.spec.containers[_]; !c.image.startswith("registry.corp.internal/")] &&
            bad.count() > 0
        }
    }
}
"#;

#[test]
fn string_op_filter_rules_compile_and_match_ast() {
    let good = json!({"request": {"object": {"spec": {"containers": [
        {"name": "app", "image": "registry.corp.internal/app:v1.2"}
    ]}}}});
    let latest = json!({"request": {"object": {"spec": {"containers": [
        {"name": "app", "image": "registry.corp.internal/app:latest"}
    ]}}}});
    let foreign = json!({"request": {"object": {"spec": {"containers": [
        {"name": "app", "image": "docker.io/library/nginx:v1"}
    ]}}}});
    // Non-string image value: string ops fail closed on both paths.
    let non_string = json!({"request": {"object": {"spec": {"containers": [
        {"name": "app", "image": 42}
    ]}}}});
    // Missing image attribute entirely.
    let no_image = json!({"request": {"object": {"spec": {"containers": [
        {"name": "app"}
    ]}}}});

    assert_eq!(
        assert_equivalent(LATEST_TAG, Some(&good), true),
        PolicyAction::Allow
    );
    assert_eq!(
        assert_equivalent(LATEST_TAG, Some(&latest), true),
        PolicyAction::Deny
    );
    assert_eq!(
        assert_equivalent(LATEST_TAG, Some(&foreign), true),
        PolicyAction::Deny
    );
    // Negated startswith over a NON-STRING: the interpreter's !(false-ish)
    // vs the compiled Not(VariableAttrStringOp) must agree — equivalence is
    // the assertion, whatever the agreed decision is.
    assert_equivalent(LATEST_TAG, Some(&non_string), true);
    assert_equivalent(LATEST_TAG, Some(&no_image), true);
    assert_equivalent(LATEST_TAG, None, true);
}

const IAM_DELETE: &str = r#"
policy tf_iam {
    default: allow,
    rule no_iam_deletions {
        deny if {
            deletions := [rc | rc := input.resource_changes[_]; rc.type == "aws_iam_user"; "delete" in rc.change.actions] &&
            deletions.count() > 0
        }
    }
}
"#;

#[test]
fn var_attr_membership_rule_compiles_and_matches_ast() {
    let deleting = json!({"resource_changes": [
        {"type": "aws_iam_user", "change": {"actions": ["delete"]}}
    ]});
    let creating = json!({"resource_changes": [
        {"type": "aws_iam_user", "change": {"actions": ["create"]}}
    ]});
    // delete on a NON-IAM type: gated out by the type filter.
    let bucket_delete = json!({"resource_changes": [
        {"type": "aws_s3_bucket", "change": {"actions": ["delete"]}}
    ]});
    // actions missing / non-list.
    let no_actions = json!({"resource_changes": [
        {"type": "aws_iam_user", "change": {}}
    ]});
    let scalar_actions = json!({"resource_changes": [
        {"type": "aws_iam_user", "change": {"actions": "delete"}}
    ]});

    assert_eq!(
        assert_equivalent(IAM_DELETE, Some(&deleting), true),
        PolicyAction::Deny
    );
    for doc in [&creating, &bucket_delete, &no_actions] {
        assert_eq!(
            assert_equivalent(IAM_DELETE, Some(doc), true),
            PolicyAction::Allow
        );
    }
    // Scalar (non-list) actions: equivalence is the assertion.
    assert_equivalent(IAM_DELETE, Some(&scalar_actions), true);
}

#[test]
fn full_library_policies_compile_whole_and_match_ast() {
    // The whole point of Phase B: the REAL k8s + terraform library policies
    // compile WHOLE and agree with the interpreter over realistic docs.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../policy-library");
    let k8s = std::fs::read_to_string(root.join("kubernetes/admission-control/policy.reap"))
        .expect("read k8s policy");
    let tf = std::fs::read_to_string(root.join("terraform/s3-guardrails/policy.reap"))
        .expect("read terraform policy");

    let violating_pod = json!({"request": {"object": {
        "metadata": {"labels": {"env": "prod"}},
        "spec": {"containers": [
            {"name": "app", "image": "docker.io/nginx:latest",
             "securityContext": {"privileged": true}}
        ]}
    }}});
    let clean_pod = json!({"request": {"object": {
        "metadata": {"labels": {"owner": "team-a"}},
        "spec": {"containers": [
            {"name": "app", "image": "registry.corp.internal/app:v1",
             "securityContext": {"privileged": false},
             "resources": {"limits": {"cpu": "1"}}}
        ]}
    }}});
    assert_eq!(
        assert_equivalent(&k8s, Some(&violating_pod), true),
        PolicyAction::Deny
    );
    assert_eq!(
        assert_equivalent(&k8s, Some(&clean_pod), true),
        PolicyAction::Allow
    );

    let bad_plan = json!({"resource_changes": [
        {"name": "logs", "type": "aws_s3_bucket",
         "change": {"actions": ["create"], "after": {"acl": "public-read"}}}
    ]});
    let good_plan = json!({"resource_changes": [
        {"name": "data", "type": "aws_s3_bucket",
         "change": {"actions": ["create"], "after": {"acl": "private", "versioning": true}}}
    ]});
    assert_eq!(
        assert_equivalent(&tf, Some(&bad_plan), true),
        PolicyAction::Deny
    );
    // good_plan: equivalence is the assertion (other guardrail rules may
    // still fire depending on the policy's full rule set).
    assert_equivalent(&tf, Some(&good_plan), true);
}
