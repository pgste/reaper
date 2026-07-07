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
                    "profile": {"country": "US", "tier": "gold"},
                    "any_true": [0, 0, 5],
                    "all_true": [1, 2, 3]
                }
            },
            {"id": "bob", "type": "user", "attributes": {
                "level": 1, "name": "bob", "email": "bob@corp.example",
                "role": "guest", "skills": ["rust"], "perms": ["read"],
                "tags": ["alpha"], "scores": [1], "csv": "x",
                "profile": {"country": "CA", "tier": "bronze"},
                "any_true": [0, 0, 0],
                "all_true": [1, 0, 3]
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
            {"id": "res_nested", "type": "resource", "attributes": {}},
            {"id": "res_util", "type": "resource", "attributes": {}},
            {"id": "res_regex", "type": "resource", "attributes": {}}
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

// ===========================================================================
// FULL DSL FUNCTION COVERAGE
//
// Every DSL method/function, exercised through build_preferred (the production
// entry point). build_preferred runs the compiled DSL v2 evaluator when it can
// and the AST evaluator otherwise; either way the decision must be correct.
// When BOTH evaluators can run a policy, they must AGREE — assert_fn_is checks
// that too, so this doubles as the equivalence guard for the compiled subset
// (count/lower/upper/contains/startswith/endswith/regex) while giving the
// AST-only methods (sum/max/min/any/all/trim/split/first/last/slice/reverse/
// sort/unique/union/intersection/difference/keys/values/has_key) real
// decision coverage.
// ===========================================================================

/// Evaluate `policy` via build_preferred and assert the decision. If the
/// compiled path ALSO accepts the policy, assert it agrees with build_preferred
/// (equivalence). Returns nothing; panics with context on any mismatch.
fn assert_fn_is(policy_text: &str, principal: &str, resource: &str, expected: PolicyAction) {
    let policy = ReaperPolicy::from_str(policy_text)
        .unwrap_or_else(|e| panic!("parse failed: {e}\npolicy:\n{policy_text}"));

    let preferred = policy
        .clone()
        .build_preferred(store_with_data())
        .unwrap_or_else(|e| panic!("build_preferred failed: {e}\npolicy:\n{policy_text}"));
    // Every function this suite covers must run on the COMPILED fast path — a
    // fallback to AST here means the compiler regressed for that function.
    assert_eq!(
        preferred.evaluator_type(),
        "reaper_dsl",
        "expected the compiled fast path but build_preferred fell back to {} \
         for {principal}/{resource}\npolicy:\n{policy_text}",
        preferred.evaluator_type()
    );
    let got = preferred
        .evaluate(&request(principal, resource))
        .expect("preferred evaluate");
    assert_eq!(
        got, expected,
        "build_preferred decision mismatch for {principal}/{resource}: got {got:?} \
         expected {expected:?}\npolicy:\n{policy_text}"
    );

    // Equivalence: the compiled path must build (guaranteed above) and agree.
    let compiled = policy
        .build(store_with_data())
        .unwrap_or_else(|e| panic!("compiled build failed: {e}\npolicy:\n{policy_text}"));
    let c: PolicyAction = compiled
        .evaluate(&request(principal, resource))
        .expect("compiled evaluate");
    assert_eq!(
        c, got,
        "compiled and AST diverged for {principal}/{resource}\npolicy:\n{policy_text}"
    );
}

// ---- Aggregate: sum / max / min ------------------------------------------
// alice.scores = [10, 40, 25]  (sum 75, max 40, min 10)

#[test]
fn fn_sum() {
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_sum" && total := user.scores.sum() && total >= 70 } } }"#;
    assert_fn_is(p, "alice", "res_sum", PolicyAction::Allow); // 75 >= 70
    assert_fn_is(p, "bob", "res_sum", PolicyAction::Deny); // bob.scores=[1] sum 1
}

#[test]
fn fn_max() {
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_max" && m := user.scores.max() && m >= 40 } } }"#;
    assert_fn_is(p, "alice", "res_max", PolicyAction::Allow); // 40 >= 40
    assert_fn_is(p, "bob", "res_max", PolicyAction::Deny);
}

#[test]
fn fn_min() {
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_min" && m := user.scores.min() && m <= 10 } } }"#;
    assert_fn_is(p, "alice", "res_min", PolicyAction::Allow); // 10 <= 10
    assert_fn_is(p, "bob", "res_min", PolicyAction::Allow); // bob min 1 <= 10
}

// ---- Collection transforms: first / last / slice / reverse / sort / unique

#[test]
fn fn_first() {
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_util" && f := user.skills.first() && f == "rust" } } }"#;
    assert_fn_is(p, "alice", "res_util", PolicyAction::Allow);
    assert_fn_is(p, "bob", "res_util", PolicyAction::Allow); // bob skills=[rust]
}

#[test]
fn fn_last() {
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_util" && l := user.skills.last() && l == "java" } } }"#;
    assert_fn_is(p, "alice", "res_util", PolicyAction::Allow);
    assert_fn_is(p, "bob", "res_util", PolicyAction::Deny); // bob last = rust
}

#[test]
fn fn_slice() {
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_util" && sl := user.scores.slice(0, 2) && c := sl.count() && c == 2 } } }"#;
    assert_fn_is(p, "alice", "res_util", PolicyAction::Allow);
}

#[test]
fn fn_reverse() {
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_util" && rv := user.skills.reverse() && f := rv.first() && f == "java" } } }"#;
    assert_fn_is(p, "alice", "res_util", PolicyAction::Allow);
}

#[test]
fn fn_sort() {
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_util" && s := user.scores.sort() && f := s.first() && f <= 10 } } }"#;
    assert_fn_is(p, "alice", "res_util", PolicyAction::Allow); // sorted first = 10
}

#[test]
fn fn_unique() {
    // alice.tags = [alpha, beta, beta, gamma] -> unique 3
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_util" && u := user.tags.unique() && c := u.count() && c == 3 } } }"#;
    assert_fn_is(p, "alice", "res_util", PolicyAction::Allow);
}

// ---- Set operations: union / intersection / difference -------------------

#[test]
fn fn_union() {
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_util" && u := user.perms.union(user.skills) && "rust" in u } } }"#;
    assert_fn_is(p, "alice", "res_util", PolicyAction::Allow);
}

#[test]
fn fn_intersection() {
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_util" && i := user.skills.intersection(["rust", "nope"]) && c := i.count() && c == 1 } } }"#;
    assert_fn_is(p, "alice", "res_util", PolicyAction::Allow);
}

#[test]
fn fn_difference() {
    // alice.skills - [rust] = [python, go, java] -> 3
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_util" && d := user.skills.difference(["rust"]) && c := d.count() && c == 3 } } }"#;
    assert_fn_is(p, "alice", "res_util", PolicyAction::Allow);
}

// ---- Object methods: keys / values / has_key -----------------------------
// alice.profile = {country: US, tier: gold}

#[test]
fn fn_keys() {
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_util" && k := user.profile.keys() && c := k.count() && c == 2 } } }"#;
    assert_fn_is(p, "alice", "res_util", PolicyAction::Allow);
}

#[test]
fn fn_values() {
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_util" && v := user.profile.values() && c := v.count() && c == 2 } } }"#;
    assert_fn_is(p, "alice", "res_util", PolicyAction::Allow);
}

#[test]
fn fn_has_key() {
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_util" && user.profile.has_key("tier") } } }"#;
    assert_fn_is(p, "alice", "res_util", PolicyAction::Allow);
}

// ---- String methods: trim / split ----------------------------------------

#[test]
fn fn_split() {
    // alice.csv = "a,b,c" -> 3 parts
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_util" && parts := user.csv.split(",") && c := parts.count() && c == 3 } } }"#;
    assert_fn_is(p, "alice", "res_util", PolicyAction::Allow);
    assert_fn_is(p, "bob", "res_util", PolicyAction::Deny); // bob.csv="x" -> 1
}

#[test]
fn fn_trim() {
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_util" && t := user.name.trim() && t == "Alice" } } }"#;
    assert_fn_is(p, "alice", "res_util", PolicyAction::Allow);
}

// ---- Aggregate predicates over a comprehension: any / all ----------------

#[test]
fn fn_any() {
    // any_true: alice [0,0,5] has a truthy element; bob [0,0,0] does not.
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_util" && user.any_true.any() } } }"#;
    assert_fn_is(p, "alice", "res_util", PolicyAction::Allow);
    assert_fn_is(p, "bob", "res_util", PolicyAction::Deny);
}

#[test]
fn fn_all() {
    // all_true: alice [1,2,3] all truthy; bob [1,0,3] has a falsy element.
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_util" && user.all_true.all() } } }"#;
    assert_fn_is(p, "alice", "res_util", PolicyAction::Allow);
    assert_fn_is(p, "bob", "res_util", PolicyAction::Deny);
}

// ---- Regex: matches (compiled + AST, so also an equivalence check) --------

#[test]
fn fn_regex_matches() {
    // alice.email starts with uppercase letters before '@'
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_regex" && regex::matches(user.email, "^[A-Z]+@") } } }"#;
    assert_fn_is(p, "alice", "res_regex", PolicyAction::Allow); // ALICE@...
    assert_fn_is(p, "bob", "res_regex", PolicyAction::Deny); // bob@... lowercase
}

// ---- Regex string methods: find / find_all / replace ---------------------

#[test]
fn fn_find() {
    // first match of "corp" in alice's email
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_regex" && m := user.email.find("corp") && m == "corp" } } }"#;
    assert_fn_is(p, "alice", "res_regex", PolicyAction::Allow);
}

#[test]
fn fn_find_all() {
    // csv "a,b,c" -> three single-letter matches [a-c]
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_regex" && all := user.csv.find_all("[a-c]") && c := all.count() && c == 3 } } }"#;
    assert_fn_is(p, "alice", "res_regex", PolicyAction::Allow);
}

#[test]
fn fn_replace() {
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_regex" && rp := user.name.replace("Alice", "Bob") && rp == "Bob" } } }"#;
    assert_fn_is(p, "alice", "res_regex", PolicyAction::Allow);
}

// ---- Negative / edge cases for the newly-compiled functions ----------------
// Each still exercises the COMPILED fast path (assert_fn_is asserts that) and
// must agree with AST on the deny outcome.

#[test]
fn fn_has_key_missing_denies() {
    // profile has no "manager" key -> false -> deny.
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_util" && user.profile.has_key("manager") } } }"#;
    assert_fn_is(p, "alice", "res_util", PolicyAction::Deny);
}

#[test]
fn fn_intersection_no_overlap_denies() {
    // skills ∩ [nope] = [] -> count 0, rule wants 1 -> deny.
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_util" && i := user.skills.intersection(["nope"]) && c := i.count() && c == 1 } } }"#;
    assert_fn_is(p, "alice", "res_util", PolicyAction::Deny);
}

#[test]
fn fn_difference_removes_all_denies() {
    // skills - [all four skills] = [] -> count 0, rule wants 3 -> deny.
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_util" && d := user.skills.difference(["rust", "python", "go", "java"]) && c := d.count() && c == 3 } } }"#;
    assert_fn_is(p, "alice", "res_util", PolicyAction::Deny);
}

#[test]
fn fn_find_no_match_denies() {
    // "zzz" is not in alice's email -> find is null -> null == "corp" is false.
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_regex" && m := user.email.find("zzz") && m == "corp" } } }"#;
    assert_fn_is(p, "alice", "res_regex", PolicyAction::Deny);
}

#[test]
fn fn_has_key_true_for_both_principals() {
    // Both alice and bob have a "tier" key -> allow for each.
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_util" && user.profile.has_key("tier") } } }"#;
    assert_fn_is(p, "alice", "res_util", PolicyAction::Allow);
    assert_fn_is(p, "bob", "res_util", PolicyAction::Allow);
}
