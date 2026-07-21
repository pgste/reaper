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
                "relationships": {"acts_for": ["agent-x", "ghost-agent"]},
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
            // Actor fixtures (F1-s2c): agents bound via the request's `actor`
            // field. alice declares acts_for → agent-x AND → ghost-agent (an
            // id that is deliberately NOT a loaded entity — rebac must still
            // match it while attribute reads stay null).
            {"id": "agent-x", "type": "agent", "attributes": {
                "kind": "agent", "trusted": true, "level": 9, "name": "AgentX",
                "scopes": ["deploy", "read"], "team": "ci", "squad": "ci",
                "profile": {"env": "ci"}
            }},
            {"id": "agent-y", "type": "agent", "attributes": {
                "kind": "agent", "trusted": false, "level": 2, "name": "AgentY",
                "nickname": "spot", "scopes": [], "team": "ci", "squad": "qa"
            }},
            {"id": "res_actor", "type": "resource", "attributes": {}},
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

        ..Default::default()
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

// ===========================================================================
// Actor binding (F1-s2c): the compiled evaluator serves `actor.*` policies.
//
// Matrix: every operator class × actor PRESENT (loaded entity) / actor
// ABSENT / actor present but NOT a loaded entity. The trap pinned hardest:
// an absent actor reads every attribute as Null, which satisfies NO
// comparison — with the single exception of an explicit `== null` literal,
// which MATCHES. Compiled and AST must agree on all of it.
// ===========================================================================

fn actor_request(principal: &str, actor: Option<&str>, resource: &str) -> PolicyRequest {
    let mut req = request(principal, resource);
    req.actor = actor.map(str::to_string);
    req
}

/// Both evaluators on the same actor-carrying request; the compiled build is
/// FORCED (`.build()`), so an actor construct the compiler quietly stopped
/// supporting fails here rather than falling back.
fn assert_actor_equivalent_is(
    policy_text: &str,
    principal: &str,
    actor: Option<&str>,
    resource: &str,
    expected: PolicyAction,
) {
    let policy = ReaperPolicy::from_str(policy_text)
        .unwrap_or_else(|e| panic!("parse failed: {e}\npolicy:\n{policy_text}"));

    let compiled = policy
        .clone()
        .build(store_with_data())
        .unwrap_or_else(|e| panic!("actor policy must compile: {e}\npolicy:\n{policy_text}"));
    let ast = policy.build_ast_evaluator(store_with_data());

    let req = actor_request(principal, actor, resource);
    let compiled_decision = compiled.evaluate(&req).expect("compiled evaluate");
    let ast_decision = ast.evaluate(&req).expect("ast evaluate");

    assert_eq!(
        compiled_decision, ast_decision,
        "compiled and AST evaluators diverged for principal={principal} actor={actor:?} \
         resource={resource}\ncompiled={compiled_decision:?} ast={ast_decision:?}\n\
         policy:\n{policy_text}"
    );
    assert_eq!(
        compiled_decision, expected,
        "decision mismatch (both agreed on {compiled_decision:?}, expected {expected:?}) \
         for principal={principal} actor={actor:?} resource={resource}"
    );
}

#[test]
fn actor_string_equality() {
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_actor" && actor.kind == "agent" } } }"#;
    assert_actor_equivalent_is(
        p,
        "alice",
        Some("agent-x"),
        "res_actor",
        PolicyAction::Allow,
    );
    assert_actor_equivalent_is(p, "alice", None, "res_actor", PolicyAction::Deny);
}

#[test]
fn actor_string_not_equal_fails_closed_when_absent() {
    // `!=` on a PRESENT differing value is true; on an ABSENT actor the read
    // is Null and Null satisfies no comparison — deny, not "trivially not
    // equal". This is the fail-closed rule that keeps actor gates safe.
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_actor" && actor.kind != "human" } } }"#;
    assert_actor_equivalent_is(
        p,
        "alice",
        Some("agent-x"),
        "res_actor",
        PolicyAction::Allow,
    );
    assert_actor_equivalent_is(p, "alice", None, "res_actor", PolicyAction::Deny);
}

#[test]
fn actor_numeric_ordering() {
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_actor" && actor.level >= 5 } } }"#;
    assert_actor_equivalent_is(
        p,
        "alice",
        Some("agent-x"),
        "res_actor",
        PolicyAction::Allow,
    );
    assert_actor_equivalent_is(p, "alice", Some("agent-y"), "res_actor", PolicyAction::Deny);
    assert_actor_equivalent_is(p, "alice", None, "res_actor", PolicyAction::Deny);
}

#[test]
fn actor_bool_gate() {
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_actor" && actor.trusted == true } } }"#;
    assert_actor_equivalent_is(
        p,
        "alice",
        Some("agent-x"),
        "res_actor",
        PolicyAction::Allow,
    );
    assert_actor_equivalent_is(p, "alice", Some("agent-y"), "res_actor", PolicyAction::Deny);
    assert_actor_equivalent_is(p, "alice", None, "res_actor", PolicyAction::Deny);
}

#[test]
fn actor_null_literal_matches_absent_actor() {
    // THE exception: an explicit `== null` literal matches a Null read, so a
    // policy can deliberately gate on "no actor attribute". Absent actor and
    // attribute-less actor both allow; an actor WITH the attribute denies.
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_actor" && actor.nickname == null } } }"#;
    assert_actor_equivalent_is(p, "alice", None, "res_actor", PolicyAction::Allow);
    assert_actor_equivalent_is(
        p,
        "alice",
        Some("agent-x"),
        "res_actor",
        PolicyAction::Allow,
    );
    assert_actor_equivalent_is(p, "alice", Some("agent-y"), "res_actor", PolicyAction::Deny);
}

#[test]
fn actor_not_null_literal_requires_present_value() {
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_actor" && actor.nickname != null } } }"#;
    assert_actor_equivalent_is(
        p,
        "alice",
        Some("agent-y"),
        "res_actor",
        PolicyAction::Allow,
    );
    assert_actor_equivalent_is(p, "alice", Some("agent-x"), "res_actor", PolicyAction::Deny);
    assert_actor_equivalent_is(p, "alice", None, "res_actor", PolicyAction::Deny);
}

#[test]
fn actor_membership() {
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_actor" && "deploy" in actor.scopes } } }"#;
    assert_actor_equivalent_is(
        p,
        "alice",
        Some("agent-x"),
        "res_actor",
        PolicyAction::Allow,
    );
    assert_actor_equivalent_is(p, "alice", Some("agent-y"), "res_actor", PolicyAction::Deny);
    assert_actor_equivalent_is(p, "alice", None, "res_actor", PolicyAction::Deny);
}

#[test]
fn actor_count() {
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_actor" && actor.scopes.count() >= 2 } } }"#;
    assert_actor_equivalent_is(
        p,
        "alice",
        Some("agent-x"),
        "res_actor",
        PolicyAction::Allow,
    );
    assert_actor_equivalent_is(p, "alice", Some("agent-y"), "res_actor", PolicyAction::Deny);
    assert_actor_equivalent_is(p, "alice", None, "res_actor", PolicyAction::Deny);
}

#[test]
fn actor_string_method() {
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_actor" && actor.name.lower() == "agentx" } } }"#;
    assert_actor_equivalent_is(
        p,
        "alice",
        Some("agent-x"),
        "res_actor",
        PolicyAction::Allow,
    );
    assert_actor_equivalent_is(p, "alice", None, "res_actor", PolicyAction::Deny);
}

#[test]
fn actor_nested_attribute() {
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_actor" && actor.profile.env == "ci" } } }"#;
    assert_actor_equivalent_is(
        p,
        "alice",
        Some("agent-x"),
        "res_actor",
        PolicyAction::Allow,
    );
    assert_actor_equivalent_is(p, "alice", Some("agent-y"), "res_actor", PolicyAction::Deny);
    assert_actor_equivalent_is(p, "alice", None, "res_actor", PolicyAction::Deny);
}

#[test]
fn actor_cross_entity_comparison_with_user() {
    // agent-x.level(9) > alice.level(7); agent-y.level(2) is not.
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_actor" && actor.level > user.level } } }"#;
    assert_actor_equivalent_is(
        p,
        "alice",
        Some("agent-x"),
        "res_actor",
        PolicyAction::Allow,
    );
    assert_actor_equivalent_is(p, "alice", Some("agent-y"), "res_actor", PolicyAction::Deny);
    assert_actor_equivalent_is(p, "alice", None, "res_actor", PolicyAction::Deny);
}

#[test]
fn actor_same_entity_comparison() {
    // agent-x: team == squad ("ci" == "ci"); agent-y: "ci" != "qa".
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_actor" && actor.team == actor.squad } } }"#;
    assert_actor_equivalent_is(
        p,
        "alice",
        Some("agent-x"),
        "res_actor",
        PolicyAction::Allow,
    );
    assert_actor_equivalent_is(p, "alice", Some("agent-y"), "res_actor", PolicyAction::Deny);
    assert_actor_equivalent_is(p, "alice", None, "res_actor", PolicyAction::Deny);
}

#[test]
fn user_indexed_access_regression() {
    // Regression (found by the actor matrix, but user-affecting): the reap
    // compiler used to DROP the bracket index in value comparisons, so
    // `user.skills[0] == "rust"` compared the whole list to the literal — a
    // compiled-only wrong Deny. Now it compiles to IndexedEquals.
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_actor" && user.skills[0] == "rust" } } }"#;
    assert_equivalent_is(p, "alice", "res_actor", PolicyAction::Allow);
    assert_equivalent_is(p, "bob", "res_actor", PolicyAction::Allow); // bob skills=[rust]

    let p_wild = r#"policy p { default: deny, rule r { allow if {
        resource == "res_actor" && user.tags[_] == "gamma" } } }"#;
    assert_equivalent_is(p_wild, "alice", "res_actor", PolicyAction::Allow);
    assert_equivalent_is(p_wild, "bob", "res_actor", PolicyAction::Deny);
}

#[test]
fn actor_indexed_access() {
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_actor" && actor.scopes[0] == "deploy" } } }"#;
    assert_actor_equivalent_is(
        p,
        "alice",
        Some("agent-x"),
        "res_actor",
        PolicyAction::Allow,
    );
    assert_actor_equivalent_is(p, "alice", Some("agent-y"), "res_actor", PolicyAction::Deny);
    assert_actor_equivalent_is(p, "alice", None, "res_actor", PolicyAction::Deny);
}

#[test]
fn actor_has_key() {
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_actor" && actor.profile.has_key("env") } } }"#;
    assert_actor_equivalent_is(
        p,
        "alice",
        Some("agent-x"),
        "res_actor",
        PolicyAction::Allow,
    );
    assert_actor_equivalent_is(p, "alice", Some("agent-y"), "res_actor", PolicyAction::Deny);
    assert_actor_equivalent_is(p, "alice", None, "res_actor", PolicyAction::Deny);
}

#[test]
fn actor_rebac_delegation() {
    // alice declares acts_for → agent-x; agent-y is not delegated; an absent
    // actor must fail the check rather than error.
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_actor" && rebac::related(actor, "acts_for", user) } } }"#;
    assert_actor_equivalent_is(
        p,
        "alice",
        Some("agent-x"),
        "res_actor",
        PolicyAction::Allow,
    );
    assert_actor_equivalent_is(p, "alice", Some("agent-y"), "res_actor", PolicyAction::Deny);
    assert_actor_equivalent_is(p, "alice", None, "res_actor", PolicyAction::Deny);
}

#[test]
fn actor_rebac_matches_unloaded_actor_id() {
    // ghost-agent is NOT a loaded entity, but alice's acts_for relation names
    // it — the relation must still hold (a relation may reference ids that
    // are not themselves entities), on both evaluators.
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_actor" && rebac::related(actor, "acts_for", user) } } }"#;
    assert_actor_equivalent_is(
        p,
        "alice",
        Some("ghost-agent"),
        "res_actor",
        PolicyAction::Allow,
    );
    // A never-seen id matches nothing.
    assert_actor_equivalent_is(
        p,
        "alice",
        Some("never-seen"),
        "res_actor",
        PolicyAction::Deny,
    );
}

#[test]
fn actor_unloaded_entity_reads_null() {
    // ghost-agent binds as an actor but has no entity: every attribute read
    // is null ⇒ fail-closed on comparisons, `== null` matches.
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_actor" && actor.kind == "agent" } } }"#;
    assert_actor_equivalent_is(
        p,
        "alice",
        Some("ghost-agent"),
        "res_actor",
        PolicyAction::Deny,
    );

    let p_null = r#"policy p { default: deny, rule r { allow if {
        resource == "res_actor" && actor.kind == null } } }"#;
    assert_actor_equivalent_is(
        p_null,
        "alice",
        Some("ghost-agent"),
        "res_actor",
        PolicyAction::Allow,
    );
}

#[test]
fn actor_and_user_bindings_stay_distinct() {
    // The same request must read user.* from the principal and actor.* from
    // the actor — passing the human id as actor lacks agent attributes.
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_actor" && user.role == "admin" && actor.kind == "agent" } } }"#;
    assert_actor_equivalent_is(
        p,
        "alice",
        Some("agent-x"),
        "res_actor",
        PolicyAction::Allow,
    );
    assert_actor_equivalent_is(p, "alice", Some("alice"), "res_actor", PolicyAction::Deny);
}

// ===========================================================================
// Taint predicates (F1-s2c): compiled `taint::trusted` / `taint::level`.
//
// Matrix: each predicate × provenance level (platform / verified / llm) ×
// taint mode off (no provenance map) × unlabeled key under taint mode (the
// fail-untrusted llm floor). Forced-compile, so a taint construct the
// compiler stops supporting fails loudly instead of falling back.
// ===========================================================================

fn taint_request(
    principal: &str,
    resource: &str,
    context: &[(&str, &str)],
    provenance: Option<&[(&str, policy_engine::TrustLevel)]>,
) -> PolicyRequest {
    let mut req = request(principal, resource);
    for (k, v) in context {
        req.context.insert(k.to_string(), v.to_string());
    }
    req.context_provenance =
        provenance.map(|p| p.iter().map(|(k, t)| (k.to_string(), *t)).collect());
    req
}

fn assert_taint_equivalent_is(
    policy_text: &str,
    req: &PolicyRequest,
    expected: PolicyAction,
    label: &str,
) {
    let policy = ReaperPolicy::from_str(policy_text)
        .unwrap_or_else(|e| panic!("parse failed: {e}\npolicy:\n{policy_text}"));

    let compiled = policy
        .clone()
        .build(store_with_data())
        .unwrap_or_else(|e| panic!("taint policy must compile: {e}\npolicy:\n{policy_text}"));
    let ast = policy.build_ast_evaluator(store_with_data());

    let compiled_decision = compiled.evaluate(req).expect("compiled evaluate");
    let ast_decision = ast.evaluate(req).expect("ast evaluate");

    assert_eq!(
        compiled_decision, ast_decision,
        "compiled and AST evaluators diverged [{label}]\n\
         compiled={compiled_decision:?} ast={ast_decision:?}\npolicy:\n{policy_text}"
    );
    assert_eq!(
        compiled_decision, expected,
        "decision mismatch [{label}] (both agreed on {compiled_decision:?}, expected {expected:?})"
    );
}

#[test]
fn taint_trusted_gate_across_levels() {
    use policy_engine::TrustLevel;
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_actor" && context.approved == "yes" && taint::trusted("approved") } } }"#;
    type TaintCase<'a> = (Option<&'a [(&'a str, TrustLevel)]>, PolicyAction, &'a str);
    let cases: &[TaintCase] = &[
        (
            Some(&[("approved", TrustLevel::Platform)]),
            PolicyAction::Allow,
            "platform",
        ),
        (
            Some(&[("approved", TrustLevel::Verified)]),
            PolicyAction::Allow,
            "verified",
        ),
        (
            Some(&[("approved", TrustLevel::Llm)]),
            PolicyAction::Deny,
            "llm",
        ),
        // taint mode ON but this key unlabeled -> llm floor -> deny
        (
            Some(&[("other", TrustLevel::Platform)]),
            PolicyAction::Deny,
            "unlabeled",
        ),
        // taint mode OFF -> platform -> allow (pre-F1 behavior)
        (None, PolicyAction::Allow, "taint-off"),
    ];
    for (prov, expected, label) in cases {
        let req = taint_request("alice", "res_actor", &[("approved", "yes")], *prov);
        assert_taint_equivalent_is(p, &req, expected.clone(), label);
    }
}

#[test]
fn taint_level_assignment_across_levels() {
    use policy_engine::TrustLevel;
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_actor" && lvl := taint::level("k") && lvl == "verified" } } }"#;
    type TaintCase<'a> = (Option<&'a [(&'a str, TrustLevel)]>, PolicyAction, &'a str);
    let cases: &[TaintCase] = &[
        (
            Some(&[("k", TrustLevel::Verified)]),
            PolicyAction::Allow,
            "verified",
        ),
        (
            Some(&[("k", TrustLevel::Platform)]),
            PolicyAction::Deny,
            "platform-not-verified",
        ),
        (Some(&[("k", TrustLevel::Llm)]), PolicyAction::Deny, "llm"),
        // taint off -> "platform" -> exact-match rule denies
        (None, PolicyAction::Deny, "taint-off-platform"),
        // unlabeled under taint mode -> "llm" -> deny
        (
            Some(&[("other", TrustLevel::Platform)]),
            PolicyAction::Deny,
            "unlabeled",
        ),
    ];
    for (prov, expected, label) in cases {
        let req = taint_request("alice", "res_actor", &[], *prov);
        assert_taint_equivalent_is(p, &req, expected.clone(), label);
    }
}

#[test]
fn taint_gate_composes_with_actor() {
    // The real agentic shape: a trusted agent AND a platform-derived
    // approval. Either leg failing denies.
    use policy_engine::TrustLevel;
    let p = r#"policy p { default: deny, rule r { allow if {
        resource == "res_actor" && actor.trusted == true && taint::trusted("approved") } } }"#;

    let mut req = taint_request(
        "alice",
        "res_actor",
        &[("approved", "yes")],
        Some(&[("approved", TrustLevel::Platform)]),
    );
    req.actor = Some("agent-x".to_string());
    assert_taint_equivalent_is(p, &req, PolicyAction::Allow, "trusted-agent+platform");

    // Same actor, LLM-asserted approval -> deny.
    let mut req2 = taint_request(
        "alice",
        "res_actor",
        &[("approved", "yes")],
        Some(&[("approved", TrustLevel::Llm)]),
    );
    req2.actor = Some("agent-x".to_string());
    assert_taint_equivalent_is(p, &req2, PolicyAction::Deny, "trusted-agent+llm");

    // Platform approval but no actor -> deny.
    let req3 = taint_request(
        "alice",
        "res_actor",
        &[("approved", "yes")],
        Some(&[("approved", TrustLevel::Platform)]),
    );
    assert_taint_equivalent_is(p, &req3, PolicyAction::Deny, "no-actor+platform");
}

// ===========================================================================
// R4-01 A.3 slice 1: literal-value assignments + entity-attr-vs-variable
// comparisons now compile — equivalence pins for the new lowerings.
// ===========================================================================

#[test]
fn literal_assignment_string_compiles_and_matches_ast() {
    // The canonical previously-uncompilable shape: bind a string literal,
    // compare an entity attribute against it (dominated use ⇒ compiles).
    let p = r#"
policy lit_string {
    default: deny,
    rule admin { allow if { x := "admin" && user.role == x } }
}
"#;
    assert_equivalent_is(p, "alice", "res_util", PolicyAction::Allow);
    assert_equivalent_is(p, "bob", "res_util", PolicyAction::Deny);
}

#[test]
fn literal_assignment_int_and_bool_compile_and_match_ast() {
    let p = r#"
policy lit_scalar {
    default: deny,
    rule exact_level { allow if { lvl := 7 && user.level == lvl } }
}
"#;
    assert_equivalent_is(p, "alice", "res_util", PolicyAction::Allow); // level 7
    assert_equivalent_is(p, "bob", "res_util", PolicyAction::Deny); // level 1

    let p_bool = r#"
policy lit_bool {
    default: deny,
    rule flagged { allow if { want := true && user.flagged == want } }
}
"#;
    // Neither fixture user has a `flagged` attribute: missing-attr vs bound
    // bool must be a non-match on BOTH paths.
    assert_equivalent_is(p_bool, "alice", "res_util", PolicyAction::Deny);
}

#[test]
fn entity_attr_not_equals_variable_compiles_and_matches_ast() {
    let p = r#"
policy lit_ne {
    default: deny,
    rule not_guest { allow if { g := "guest" && user.role != g } }
}
"#;
    assert_equivalent_is(p, "alice", "res_util", PolicyAction::Allow); // admin
    assert_equivalent_is(p, "bob", "res_util", PolicyAction::Deny); // guest
}

#[test]
fn float_literal_assignment_still_falls_back() {
    // CompiledLiteralValue has no float variant: this must NOT compile (it
    // would need widening — a later slice), and must still evaluate on AST.
    let p = r#"
policy lit_float {
    default: deny,
    rule f { allow if { t := 0.5 && user.level > t } }
}
"#;
    let policy = ReaperPolicy::from_str(p).expect("parse");
    assert!(
        policy.clone().build(store_with_data()).is_err(),
        "float literal assignment unexpectedly compiled — widen this test"
    );
    let ast = policy.build_ast_evaluator(store_with_data());
    assert_eq!(
        ast.evaluate(&request("alice", "res_util")).expect("ast"),
        PolicyAction::Allow
    );
}

#[test]
fn undominated_variable_use_still_falls_back() {
    // `x` is bound inside an Or branch only: the runtime may reach the use
    // with x unbound (interpreter errors, compiled EqualsVariable would read
    // false) — the dominance guard must refuse to compile the rule.
    let p = r#"
policy undominated {
    default: deny,
    rule r { allow if { ( x := "admin" && user.level > 100 ) || user.role == x } }
}
"#;
    let policy = ReaperPolicy::from_str(p).expect("parse");
    assert!(
        policy.build(store_with_data()).is_err(),
        "un-dominated variable use must not compile (unbound-read semantics \
         differ between the evaluators)"
    );
}
