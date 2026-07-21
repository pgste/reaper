//! Compiled check-driver differential (R4-01 Phase B.3's merge gate —
//! design COMPILED_INPUT_DESIGN.md §4 gate 2).
//!
//! Contract: `CheckResult` from the compiled (and mixed) check drivers must
//! be BYTE-IDENTICAL to the interpreter's — `allowed`, the violation set,
//! its ORDER, and every rendered message string. Messages are user-visible
//! audit artifacts; "close enough" is not a contract.

use policy_engine::data::DataStore;
use policy_engine::reap::ReaperPolicy;
use policy_engine::PolicyRequest;
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

/// Comparable projection of a check outcome: success carries `allowed` and
/// the ordered (rule, message) pairs; failure carries the error's debug
/// text. ERROR parity is part of the contract — the interpreter's strict
/// `concat()` (string args only) must error identically on the compiled
/// path, not render a "close enough" message.
type CheckOutcome = Result<(bool, Vec<(String, Option<String>)>), String>;

fn outcome(r: Result<policy_engine::reap::CheckResult, reaper_core::ReaperError>) -> CheckOutcome {
    r.map(|c| {
        (
            c.allowed,
            c.violations
                .into_iter()
                .map(|v| (v.rule, v.message))
                .collect(),
        )
    })
    .map_err(|e| format!("{e:?}"))
}

/// Compiled check ≡ interpreter check, byte-for-byte — including errors.
fn assert_check_equivalent(policy_text: &str, doc: Option<&serde_json::Value>) {
    let policy = ReaperPolicy::from_str(policy_text).expect("parse");
    let compiled = policy
        .clone()
        .build(store())
        .expect("policy must compile whole (B.3)");
    let ast = policy.build_ast_evaluator(store());

    let c = outcome(compiled.check_with_input(&request(), doc));
    let a = outcome(ast.check_with_input(&request(), doc));
    assert_eq!(c, a, "check outcome diverged\ndoc: {doc:?}\n{policy_text}");
}

#[test]
fn full_k8s_library_policy_check_is_byte_identical() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../policy-library");
    let k8s = std::fs::read_to_string(root.join("kubernetes/admission-control/policy.reap"))
        .expect("read k8s policy");

    // Fires latest-tag, unapproved-registry, privileged, missing-owner AND
    // missing-limits — five violations, each with a rendered message.
    let horror_pod = json!({"request": {"object": {
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
    let partial_pod = json!({"request": {"object": {
        "metadata": {"labels": {"owner": "team-a"}},
        "spec": {"containers": [
            {"name": "app", "image": "registry.corp.internal/app:latest",
             "resources": {"limits": {"cpu": "1"}}}
        ]}
    }}});

    for doc in [
        Some(&horror_pod),
        Some(&clean_pod),
        Some(&partial_pod),
        None,
    ] {
        assert_check_equivalent(&k8s, doc);
    }

    // Spot-pin one rendered message so the parity claim is visibly concrete.
    let policy = ReaperPolicy::from_str(&k8s).expect("parse");
    let compiled = policy.build(store()).expect("compile");
    let result = compiled
        .check_with_input(&request(), Some(&horror_pod))
        .expect("check");
    assert!(!result.allowed);
    assert!(
        result.violations.iter().any(|v| v.message.as_deref()
            == Some("image uses :latest tag: docker.io/nginx:latest")),
        "expected the concat message verbatim; got {:?}",
        result.violations
    );
}

#[test]
fn full_terraform_library_policy_check_is_byte_identical() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../policy-library");
    let tf = std::fs::read_to_string(root.join("terraform/s3-guardrails/policy.reap"))
        .expect("read terraform policy");

    let bad_plan = json!({"resource_changes": [
        {"name": "logs", "type": "aws_s3_bucket",
         "change": {"actions": ["create"], "after": {"acl": "public-read"}}},
        {"name": "users", "type": "aws_iam_user",
         "change": {"actions": ["delete"], "after": {}}}
    ]});
    let clean_plan = json!({"resource_changes": [
        {"name": "data", "type": "aws_s3_bucket",
         "change": {"actions": ["create"], "after": {"acl": "private", "versioning": true}}}
    ]});
    for doc in [Some(&bad_plan), Some(&clean_plan), None] {
        assert_check_equivalent(&tf, doc);
    }
}

#[test]
fn message_variable_rendering_matches_ast() {
    // The three message shapes: literal-only, strict concat of string
    // variables, and a bare variable (lenient render — any type
    // stringifies, mirroring the interpreter's eval_value_to_message).
    let p = r#"
policy msg_shapes {
    default: allow,
    rule string_var {
        deny with message concat("bad env: ", e) if {
            e := input.env &&
            input.env == "prod"
        }
    }
    rule bare_int_var {
        deny with message n if {
            n := input.count &&
            input.count > 3
        }
    }
    rule bare_missing_var {
        deny with message ghost if {
            ghost := input.absent &&
            input.flag == true
        }
    }
    rule literal_only {
        deny with message "flat message" if input.flag == true
    }
}
"#;
    for doc in [
        // bare_int_var renders the INT leniently ("7"); bare_missing_var
        // binds Null from the absent path and renders "".
        Some(&json!({"env": "prod", "count": 7, "flag": true})),
        Some(&json!({"env": "dev", "count": 1, "flag": false})),
        None,
    ] {
        assert_check_equivalent(p, doc);
    }
}

#[test]
fn strict_concat_type_error_matches_ast() {
    // The interpreter's concat() is STRICT: a non-string argument is an
    // evaluation error, not a stringified value. The compiled render must
    // produce the identical error — the outcome projection compares Err
    // debug text byte-for-byte.
    let p = r#"
policy strict_concat {
    default: allow,
    rule int_in_concat {
        deny with message concat("count was ", n) if {
            n := input.count &&
            input.count > 3
        }
    }
}
"#;
    // count = 7 → rule matches → both sides must ERROR identically.
    assert_check_equivalent(p, Some(&json!({"count": 7})));
    // count = 1 → rule doesn't match → no render → both sides Ok.
    assert_check_equivalent(p, Some(&json!({"count": 1})));
    assert_check_equivalent(p, None);

    // Pin that the matching case really is the concat error, not some
    // other divergence-masking failure.
    let policy = ReaperPolicy::from_str(p).expect("parse");
    let compiled = policy.build(store()).expect("compile");
    let err = compiled
        .check_with_input(&request(), Some(&json!({"count": 7})))
        .expect_err("strict concat over an int must error");
    assert!(
        format!("{err:?}").contains("concat() requires string arguments"),
        "wrong error: {err:?}"
    );
}

#[test]
fn mixed_policy_check_matches_whole_ast() {
    // One compiled input rule with a message + one AST-fallback rule (float
    // trigger) with a message: the mixed check must equal the whole-AST
    // check byte-for-byte, in rule order.
    let p = r#"
policy mixed_check {
    default: allow,
    rule owner_required {
        deny with message "missing owner" if input.metadata.owner == null
    }
    rule float_trigger_ast {
        deny with message "cap exceeded" if { cap := 8.5 && input.score > cap }
    }
}
"#;
    let policy_text = p;
    let ast_policy = policy_engine::reap::ReapParser::parse(policy_text).expect("parse ast");
    let mixed = match policy_engine::reap::MixedReapEvaluator::build(ast_policy, store())
        .expect("mixed build")
    {
        policy_engine::reap::PerRuleBuild::Mixed(m) => m,
        other => panic!("expected mixed, got {other:?}"),
    };
    let ast = ReaperPolicy::from_str(policy_text)
        .expect("parse")
        .build_ast_evaluator(store());

    for doc in [
        Some(&json!({"metadata": {}, "score": 9})),
        Some(&json!({"metadata": {"owner": "t"}, "score": 9})),
        Some(&json!({"metadata": {"owner": "t"}, "score": 1})),
        None,
    ] {
        let m = mixed
            .check_with_input(&request(), doc)
            .expect("mixed check");
        let a = ast.check_with_input(&request(), doc).expect("ast check");
        assert_eq!(m.allowed, a.allowed, "allowed diverged for {doc:?}");
        let mv: Vec<_> = m
            .violations
            .iter()
            .map(|v| (v.rule.clone(), v.message.clone()))
            .collect();
        let av: Vec<_> = a
            .violations
            .iter()
            .map(|v| (v.rule.clone(), v.message.clone()))
            .collect();
        assert_eq!(mv, av, "violations diverged for {doc:?}");
    }
}

#[test]
fn unlowerable_messages_still_fall_back() {
    // A message referencing a variable the rule never binds: the
    // interpreter would ERROR rendering it — the rule must not compile.
    let unbound = r#"
policy f1 {
    default: allow,
    rule r { deny with message concat("x: ", ghost) if input.flag == true }
}
"#;
    // A message expression shape outside the lowering (method call).
    let exotic = r#"
policy f2 {
    default: allow,
    rule r { deny with message input.env.upper() if input.flag == true }
}
"#;
    for (label, text) in [
        ("unbound message var", unbound),
        ("exotic message expr", exotic),
    ] {
        let policy = ReaperPolicy::from_str(text).expect("parse");
        assert!(
            policy.build(store()).is_err(),
            "{label} unexpectedly compiled — widen the check differential first"
        );
    }
}
