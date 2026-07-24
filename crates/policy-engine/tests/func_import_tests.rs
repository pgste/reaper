//! Helper predicates (`func`) + load-time imports (R4-01 Phase C).
//!
//! Gates, in the order the plan states them:
//! - `func` definitions parse, are non-recursive (call-graph DAG enforced at
//!   parse), and depth accounting includes function bodies;
//! - calls evaluate identically on the compiled path (inlining, ADR-2) and
//!   the AST path (call-by-value interpretation) — decisions AND check-mode
//!   violations;
//! - `import "path" as ns` resolves at LOAD time; imported units are embedded
//!   in the policy AST and travel through the bundle format with no runtime
//!   file I/O;
//! - the language version bumps to 3 for policies using the constructs: they
//!   are stamped `language_version: "3"`, an explicit older declaration is
//!   rejected, and older engines fail closed on both the source metadata and
//!   the bundle wire format.

use policy_engine::data::{DataLoader, DataStore};
use policy_engine::reap::{ReaperPolicy, CURRENT_LANGUAGE_VERSION};
use policy_engine::{PolicyAction, PolicyEvaluator, PolicyRequest};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

fn store_with_data() -> Arc<DataStore> {
    let store = Arc::new(DataStore::new());
    let loader = DataLoader::new((*store).clone());
    loader
        .load_json(
            &serde_json::json!({
                "entities": [
                    {"id": "alice", "type": "user", "attributes": {
                        "role": "admin", "level": 7, "team": "core",
                        "skills": ["rust", "go", "python"]
                    }},
                    {"id": "bob", "type": "user", "attributes": {
                        "role": "guest", "level": 1, "team": "web",
                        "skills": ["css"]
                    }},
                    {"id": "doc1", "type": "resource", "attributes": {"tier": "restricted"}}
                ]
            })
            .to_string(),
        )
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

/// Both evaluators, same policy, same request — decisions must agree; returns
/// the agreed decision. This is the differential gate for the func feature:
/// the compiled side INLINES calls, the AST side INTERPRETS them.
fn assert_equivalent(policy_text: &str, principal: &str, resource: &str) -> PolicyAction {
    let policy = ReaperPolicy::from_str(policy_text).expect("parse policy");
    let compiled = policy
        .clone()
        .build(store_with_data())
        .expect("build compiled evaluator (func policies must compile via inlining)");
    let ast = policy.build_ast_evaluator(store_with_data());

    let req = request(principal, resource);
    let compiled_decision = compiled.evaluate(&req).expect("compiled evaluate");
    let ast_decision = ast.evaluate(&req).expect("ast evaluate");
    assert_eq!(
        compiled_decision, ast_decision,
        "compiled(inlined) and AST(interpreted) diverged for principal={principal}\n{policy_text}"
    );
    compiled_decision
}

fn assert_equivalent_is(policy_text: &str, principal: &str, resource: &str, want: PolicyAction) {
    let got = assert_equivalent(policy_text, principal, resource);
    assert_eq!(got, want, "principal={principal} resource={resource}");
}

// ---------------------------------------------------------------------------
// Core semantics: entity-attr args, literal args, bound-variable args,
// multi-param, nested calls, negation, deny rules.
// ---------------------------------------------------------------------------

#[test]
fn func_entity_attr_argument() {
    let p = r#"
        policy p {
            default: deny,
            func is_admin(r) := r == "admin",
            rule r { allow if is_admin(user.role) }
        }
    "#;
    assert_equivalent_is(p, "alice", "doc1", PolicyAction::Allow);
    assert_equivalent_is(p, "bob", "doc1", PolicyAction::Deny);
}

#[test]
fn func_multi_param_and_literal_argument() {
    let p = r#"
        policy p {
            default: deny,
            func at_least(lvl, min) := lvl >= min,
            rule r { allow if at_least(user.level, 5) }
        }
    "#;
    assert_equivalent_is(p, "alice", "doc1", PolicyAction::Allow);
    assert_equivalent_is(p, "bob", "doc1", PolicyAction::Deny);
}

#[test]
fn func_bound_variable_argument() {
    let p = r#"
        policy p {
            default: deny,
            func is_admin(r) := r == "admin",
            rule r { allow if role := user.role && is_admin(role) }
        }
    "#;
    assert_equivalent_is(p, "alice", "doc1", PolicyAction::Allow);
    assert_equivalent_is(p, "bob", "doc1", PolicyAction::Deny);
}

#[test]
fn func_calls_func_dag() {
    let p = r#"
        policy p {
            default: deny,
            func is_admin(r) := r == "admin",
            func is_privileged(r, lvl) := is_admin(r) && lvl >= 5,
            rule r { allow if is_privileged(user.role, user.level) }
        }
    "#;
    assert_equivalent_is(p, "alice", "doc1", PolicyAction::Allow);
    assert_equivalent_is(p, "bob", "doc1", PolicyAction::Deny);
}

#[test]
fn func_zero_params_reads_entities_directly() {
    let p = r#"
        policy p {
            default: deny,
            func admin_on_core() := user.role == "admin" && user.team == "core",
            rule r { allow if admin_on_core() }
        }
    "#;
    assert_equivalent_is(p, "alice", "doc1", PolicyAction::Allow);
    assert_equivalent_is(p, "bob", "doc1", PolicyAction::Deny);
}

#[test]
fn func_under_negation_and_disjunction() {
    let p = r#"
        policy p {
            default: deny,
            func is_guest(r) := r == "guest",
            rule r { allow if !is_guest(user.role) || user.level >= 100 }
        }
    "#;
    assert_equivalent_is(p, "alice", "doc1", PolicyAction::Allow);
    assert_equivalent_is(p, "bob", "doc1", PolicyAction::Deny);
}

#[test]
fn func_in_deny_rule_deny_overrides() {
    let p = r#"
        policy p {
            default: allow,
            func is_guest(r) := r == "guest",
            rule block { deny if is_guest(user.role) }
        }
    "#;
    assert_equivalent_is(p, "bob", "doc1", PolicyAction::Deny);
    assert_equivalent_is(p, "alice", "doc1", PolicyAction::Allow);
}

#[test]
fn func_body_with_local_binding() {
    // Body binds a local; inlining alpha-renames it per call site — two calls
    // in one rule must not collide.
    let p = r#"
        policy p {
            default: deny,
            func has_skills(n) := c := user.skills.count() && c >= n,
            rule r { allow if has_skills(2) && has_skills(3) }
        }
    "#;
    assert_equivalent_is(p, "alice", "doc1", PolicyAction::Allow);
    assert_equivalent_is(p, "bob", "doc1", PolicyAction::Deny);
}

#[test]
fn func_scope_does_not_leak_caller_variables() {
    // The interpreter runs bodies in a fresh scope; inlining guarantees the
    // same via the no-free-variables rule. A body referencing a caller rule
    // variable must be rejected at parse, not silently resolved.
    let p = r#"
        policy p {
            default: deny,
            func leaky() := caller_var == "x",
            rule r { allow if caller_var := "x" && leaky() }
        }
    "#;
    let err = ReaperPolicy::from_str(p).unwrap_err();
    assert!(
        err.to_string().contains("undefined variable"),
        "free variables in func bodies must be rejected: {err}"
    );
}

// ---------------------------------------------------------------------------
// Check mode: violations and messages flow through funcs on both drivers.
// ---------------------------------------------------------------------------

#[test]
fn func_check_mode_violations_agree() {
    let p = r#"
        policy p {
            default: allow,
            func too_low(lvl, min) := lvl < min,
            rule underleveled {
                deny with message "level below threshold"
                if too_low(user.level, 5)
            }
        }
    "#;
    let policy = ReaperPolicy::from_str(p).expect("parse");
    let compiled = policy
        .clone()
        .build(store_with_data())
        .expect("compiled build");
    let ast = policy.build_ast_evaluator(store_with_data());

    let req = request("bob", "doc1");
    let c = compiled
        .check_with_input(&req, None)
        .expect("compiled check");
    let a = PolicyEvaluator::check_with_input(&ast, &req, None).expect("ast check");
    assert!(!c.allowed && !a.allowed);
    assert_eq!(c.violations.len(), 1);
    assert_eq!(a.violations.len(), 1);
    assert_eq!(c.violations[0].rule, "underleveled");
    assert_eq!(
        c.violations[0].message.as_deref(),
        Some("level below threshold")
    );
    assert_eq!(c.violations[0].message, a.violations[0].message);

    let req_ok = request("alice", "doc1");
    assert!(compiled.check_with_input(&req_ok, None).unwrap().allowed);
    assert!(
        PolicyEvaluator::check_with_input(&ast, &req_ok, None)
            .unwrap()
            .allowed
    );
}

// ---------------------------------------------------------------------------
// Totality: recursion, depth, arity, name hygiene — all parse/validate errors.
// ---------------------------------------------------------------------------

#[test]
fn recursive_func_rejected() {
    let p = r#"
        policy p {
            default: deny,
            func f(x) := f(x),
            rule r { allow if f(user.level) }
        }
    "#;
    let err = ReaperPolicy::from_str(p).unwrap_err();
    assert!(
        err.to_string().contains("recursive"),
        "direct recursion must be a parse error: {err}"
    );
}

#[test]
fn mutually_recursive_funcs_rejected() {
    let p = r#"
        policy p {
            default: deny,
            func f(x) := g(x),
            func g(x) := f(x),
            rule r { allow if f(user.level) }
        }
    "#;
    let err = ReaperPolicy::from_str(p).unwrap_err();
    assert!(
        err.to_string().contains("recursive"),
        "mutual recursion must be a parse error: {err}"
    );
}

#[test]
fn arity_mismatch_rejected_at_parse() {
    let p = r#"
        policy p {
            default: deny,
            func f(a, b) := a == b,
            rule r { allow if f(user.level) }
        }
    "#;
    let err = ReaperPolicy::from_str(p).unwrap_err();
    assert!(
        err.to_string().contains("takes 2 argument"),
        "arity mismatch must be a parse error: {err}"
    );
}

#[test]
fn builtin_global_name_collision_rejected() {
    let p = r#"
        policy p {
            default: deny,
            func concat(a) := a == "x",
            rule r { allow if concat(user.role) }
        }
    "#;
    let err = ReaperPolicy::from_str(p).unwrap_err();
    assert!(err.to_string().contains("collides with a builtin"), "{err}");
}

#[test]
fn duplicate_func_rejected() {
    let p = r#"
        policy p {
            default: deny,
            func f(a) := a == "x",
            func f(b) := b == "y",
            rule r { allow if f(user.role) }
        }
    "#;
    let err = ReaperPolicy::from_str(p).unwrap_err();
    assert!(err.to_string().contains("duplicate func"), "{err}");
}

#[test]
fn param_shadowing_rejected() {
    let p = r#"
        policy p {
            default: deny,
            func f(a) := a := "x" && a == "x",
            rule r { allow if f(user.role) }
        }
    "#;
    let err = ReaperPolicy::from_str(p).unwrap_err();
    assert!(err.to_string().contains("shadows its parameter"), "{err}");
}

#[test]
fn unknown_namespace_call_rejected() {
    // The strict namespace check applies to policies that use v3 constructs
    // (function-free v2 policies keep their historical runtime-error behavior
    // for unknown names — parse acceptance of deployable policies must not
    // tighten retroactively).
    let p = r#"
        policy p {
            default: deny,
            func f(a) := a == "x",
            rule r { allow if nope::f(user.role) }
        }
    "#;
    let err = ReaperPolicy::from_str(p).unwrap_err();
    assert!(err.to_string().contains("unknown namespace"), "{err}");
}

#[test]
fn deep_func_chain_depth_bounded() {
    // A chain of funcs each nesting a little: the inline-effective depth
    // accounting must reject it even though each body alone is shallow.
    let mut src = String::from("policy p {\n default: deny,\n");
    src.push_str("func f0(x) := x >= 0,\n");
    for i in 1..70 {
        src.push_str(&format!("func f{i}(x) := f{}(x),\n", i - 1));
    }
    src.push_str("rule r { allow if f69(user.level) }\n}");
    let err = ReaperPolicy::from_str(&src).unwrap_err();
    assert!(
        err.to_string().contains("nesting depth"),
        "long call chains must trip the depth cap: {err}"
    );
}

// ---------------------------------------------------------------------------
// Language versioning: stamp, explicit declarations, fail-closed.
// ---------------------------------------------------------------------------

#[test]
fn func_policy_stamped_language_version_3() {
    let p = r#"
        policy p {
            default: deny,
            func f(a) := a == "admin",
            rule r { allow if f(user.role) }
        }
    "#;
    let policy = ReaperPolicy::from_str(p).expect("parse");
    assert_eq!(policy.language_version(), 3);
    assert_eq!(policy.metadata().get("language_version").unwrap(), "3");
}

#[test]
fn func_policy_declaring_v2_rejected() {
    let p = r#"
        policy p {
            language_version: "2",
            default: deny,
            func f(a) := a == "admin",
            rule r { allow if f(user.role) }
        }
    "#;
    let err = ReaperPolicy::from_str(p).unwrap_err();
    assert!(
        err.to_string().contains("language_version"),
        "understated version must be rejected: {err}"
    );
}

#[test]
fn func_policy_declaring_v3_accepted() {
    let p = r#"
        policy p {
            language_version: "3",
            default: deny,
            func f(a) := a == "admin",
            rule r { allow if f(user.role) }
        }
    "#;
    let policy = ReaperPolicy::from_str(p).expect("explicit v3 must parse");
    assert_eq!(policy.language_version(), 3);
}

#[test]
fn plain_policy_not_stamped() {
    // Function-free policies keep their headerless form — no stamp, implicit
    // current version, v2-compatible bundles.
    let p = r#"policy p { default: deny, rule r { allow if user.role == "admin" } }"#;
    let policy = ReaperPolicy::from_str(p).expect("parse");
    assert!(policy.metadata().get("language_version").is_none());
    assert_eq!(policy.language_version(), CURRENT_LANGUAGE_VERSION);
}

// ---------------------------------------------------------------------------
// Bundles: functions travel in the artifact; the wire format stays
// v2-compatible for function-free policies and fail-closed for older engines.
// ---------------------------------------------------------------------------

#[test]
fn bundle_roundtrip_carries_functions() {
    let p = r#"
        policy p {
            default: deny,
            func is_admin(r) := r == "admin",
            rule r { allow if is_admin(user.role) }
        }
    "#;
    let policy = ReaperPolicy::from_str(p).expect("parse");
    let bytes = policy.compile_to_bundle().expect("bundle");

    // Loading rebuilds the compiled evaluator — inlining runs against the
    // functions embedded in the bundle, no source or filesystem needed.
    let evaluator = ReaperPolicy::from_bundle(&bytes, store_with_data()).expect("load bundle");
    assert_eq!(
        evaluator.evaluate(&request("alice", "doc1")).unwrap(),
        PolicyAction::Allow
    );
    assert_eq!(
        evaluator.evaluate(&request("bob", "doc1")).unwrap(),
        PolicyAction::Deny
    );
}

#[test]
fn function_free_bundle_stays_wire_v2() {
    use policy_engine::reap::PolicyBundle;
    let p = r#"policy p { default: deny, rule r { allow if user.role == "admin" } }"#;
    let policy = ReaperPolicy::from_str(p).expect("parse");
    let bytes = policy.compile_to_bundle().expect("bundle");
    let decoded = PolicyBundle::from_bytes(&bytes).expect("decode");
    assert_eq!(
        decoded.metadata.version, 2,
        "function-free bundles must keep the v2 wire encoding so v2 engines load them"
    );
}

#[test]
fn func_bundle_uses_wire_v3_and_carries_language_marker() {
    use policy_engine::reap::PolicyBundle;
    let p = r#"
        policy p {
            default: deny,
            func is_admin(r) := r == "admin",
            rule r { allow if is_admin(user.role) }
        }
    "#;
    let policy = ReaperPolicy::from_str(p).expect("parse");
    let bytes = policy.compile_to_bundle().expect("bundle");
    let decoded = PolicyBundle::from_bytes(&bytes).expect("decode");
    assert_eq!(decoded.metadata.version, 3);
    assert_eq!(decoded.policy.functions.len(), 1);
    // The language marker rides in the bundle: a v2 engine that somehow read
    // the bytes would still reject on language_version "3".
    assert_eq!(
        decoded.policy.metadata.get("language_version").unwrap(),
        "3"
    );
}

// ---------------------------------------------------------------------------
// Imports: load-time resolution, embedding, namespacing, hygiene.
// ---------------------------------------------------------------------------

fn write_files(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    for (name, content) in files {
        let path = dir.path().join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }
    dir
}

const LIB_SRC: &str = r#"
    library rbac {
        func is_admin(r) := r == "admin",
        func is_privileged(r, lvl) := is_admin(r) && lvl >= 5,
    }
"#;

const IMPORTING_POLICY: &str = r#"
    import "lib/rbac.reap" as rbac

    policy p {
        default: deny,
        rule r { allow if rbac::is_privileged(user.role, user.level) }
    }
"#;

#[test]
fn import_resolves_at_load_time_and_evaluates_on_both_paths() {
    let dir = write_files(&[
        ("lib/rbac.reap", LIB_SRC),
        ("policy.reap", IMPORTING_POLICY),
    ]);
    let policy = ReaperPolicy::from_file(dir.path().join("policy.reap")).expect("load");

    // Imported functions are embedded, namespaced under the alias; the
    // library-internal call (is_privileged -> is_admin) was rewritten too.
    assert_eq!(policy.language_version(), 3);

    let compiled = policy
        .clone()
        .build(store_with_data())
        .expect("compiled build");
    let ast = policy.clone().build_ast_evaluator(store_with_data());
    for (principal, want) in [("alice", PolicyAction::Allow), ("bob", PolicyAction::Deny)] {
        let req = request(principal, "doc1");
        assert_eq!(compiled.evaluate(&req).unwrap(), want, "{principal}");
        assert_eq!(ast.evaluate(&req).unwrap(), want, "{principal}");
    }

    // No runtime file I/O: the bundle built from the loaded policy carries
    // the imported units and evaluates after the library file is DELETED.
    let bytes = policy.compile_to_bundle().expect("bundle");
    drop(dir);
    let evaluator = ReaperPolicy::from_bundle(&bytes, store_with_data()).expect("load bundle");
    assert_eq!(
        evaluator.evaluate(&request("alice", "doc1")).unwrap(),
        PolicyAction::Allow
    );
}

#[test]
fn import_rejected_when_parsed_from_string() {
    let err = ReaperPolicy::from_str(IMPORTING_POLICY).unwrap_err();
    assert!(
        err.to_string().contains("imports resolve at load time"),
        "string-parsed policies cannot import: {err}"
    );
}

#[test]
fn import_alias_colliding_with_builtin_namespace_rejected() {
    let dir = write_files(&[
        ("lib/rbac.reap", LIB_SRC),
        (
            "policy.reap",
            r#"
            import "lib/rbac.reap" as time

            policy p {
                default: deny,
                rule r { allow if time::is_admin(user.role) }
            }
            "#,
        ),
    ]);
    let err = ReaperPolicy::from_file(dir.path().join("policy.reap")).unwrap_err();
    assert!(err.to_string().contains("builtin namespace"), "{err}");
}

#[test]
fn import_path_traversal_rejected() {
    let dir = write_files(&[(
        "policy.reap",
        r#"
        import "../outside.reap" as lib

        policy p {
            default: deny,
            rule r { allow if lib::f(user.role) }
        }
        "#,
    )]);
    let err = ReaperPolicy::from_file(dir.path().join("policy.reap")).unwrap_err();
    assert!(err.to_string().contains(".."), "{err}");
}

#[test]
fn import_typoed_function_name_rejected_after_merge() {
    let dir = write_files(&[
        ("lib/rbac.reap", LIB_SRC),
        (
            "policy.reap",
            r#"
            import "lib/rbac.reap" as rbac

            policy p {
                default: deny,
                rule r { allow if rbac::is_admn(user.role) }
            }
            "#,
        ),
    ]);
    let err = ReaperPolicy::from_file(dir.path().join("policy.reap")).unwrap_err();
    assert!(
        err.to_string().contains("no function named"),
        "typo'd imported function must fail at load: {err}"
    );
}

// ---------------------------------------------------------------------------
// Fallback: argument shapes the inliner doesn't compile still evaluate
// correctly via build_preferred (per-rule AST fallback), and decisions agree
// with the pure interpreter.
// ---------------------------------------------------------------------------

#[test]
fn non_inlinable_argument_falls_back_and_agrees() {
    // Method-call argument: rejected by the inliner (re-evaluation could be
    // observed), interpreted by the AST path.
    let p = r#"
        policy p {
            default: deny,
            func many(n) := n >= 3,
            rule r { allow if many(user.skills.count()) }
        }
    "#;
    let policy = ReaperPolicy::from_str(p).expect("parse");

    // The whole-policy compiled build must REFUSE (not miscompile) this.
    assert!(policy.clone().build(store_with_data()).is_err());

    // The preferred path serves it (mixed/AST), and it matches the interpreter.
    let preferred = policy
        .clone()
        .build_preferred(store_with_data())
        .expect("preferred build");
    let ast = policy.build_ast_evaluator(store_with_data());
    for (principal, want) in [("alice", PolicyAction::Allow), ("bob", PolicyAction::Deny)] {
        let req = request(principal, "doc1");
        assert_eq!(preferred.evaluate(&req).unwrap(), want, "{principal}");
        assert_eq!(ast.evaluate(&req).unwrap(), want, "{principal}");
    }
}
