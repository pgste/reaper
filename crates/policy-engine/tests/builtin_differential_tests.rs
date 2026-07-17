//! Builtin differential/property oracle (round-3 Plan 05 §4.2, Testing T2).
//!
//! The compiled-vs-AST equivalence suite pins builtin behaviour on *curated*
//! inputs. This suite brings the authz-relevant builtins under a **property**
//! oracle: over generated/boundary inputs, a policy using the builtin is
//! evaluated on BOTH the compiled `ReaperDSLEvaluator` and the AST
//! `ReapAstEvaluator`, and both are checked against an **independent naive
//! oracle** (the `regex` crate, a hand comparison, `base64`+`serde_json`) —
//! never against the evaluator's own code. Testing T2 flags the builtins as
//! "the single most likely place for a correctness bug to reach production
//! undetected"; a miscompiled dispatch arm that the curated set happens to miss
//! is caught here.
//!
//! Harness ownership per ADR-2: this file owns the differential *engine*; the
//! canonical builtin *semantics* are plan 04's. Time-dependent cases pin the
//! process clock via `policy_engine::clock` (now native-available, T6) under a
//! shared lock so pinned tests never race the wall clock or each other.

use policy_engine::clock;
use policy_engine::data::{DataLoader, DataStore};
use policy_engine::reap::ReaperPolicy;
use policy_engine::{PolicyAction, PolicyEvaluator, PolicyRequest};
use proptest::prelude::*;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

/// Serializes every test that pins the global clock, so a pinned instant in one
/// test never leaks into another running in parallel.
static CLOCK_LOCK: Mutex<()> = Mutex::new(());

/// Evaluate `policy_src` for principal `alice` (with attribute `attrs`) against
/// `resource`, on BOTH evaluators. Returns `(ast, compiled)`; either is `None`
/// if that evaluator errored or (for compiled) the policy did not compile.
fn eval_both(
    policy_src: &str,
    attrs: serde_json::Value,
    action: &str,
    resource: &str,
) -> (Option<PolicyAction>, Option<PolicyAction>) {
    let data = serde_json::json!({
        "entities": [
            {"id": "alice", "type": "user", "attributes": attrs},
            {"id": resource, "type": "resource", "attributes": {}}
        ]
    });

    let store = Arc::new(DataStore::new());
    DataLoader::new((*store).clone())
        .load_json(&data.to_string())
        .expect("load data");

    let policy = ReaperPolicy::from_str(policy_src).expect("policy must parse");

    let mut context = std::collections::HashMap::new();
    // The evaluator resolves `user.*` from the principal named in the context.
    context.insert("principal".to_string(), "alice".to_string());
    let request = PolicyRequest {
        resource: resource.to_string(),
        action: action.to_string(),
        context,
        ..Default::default()
    };

    let ast = policy
        .clone()
        .build_ast_evaluator(store.clone())
        .evaluate(&request)
        .ok();
    let compiled = policy
        .build(store)
        .ok()
        .and_then(|c| c.evaluate(&request).ok());

    (ast, compiled)
}

/// Assert both evaluators produced `expected`. At least the AST path must exist
/// (it supports every feature); when the compiled path exists it must agree.
fn assert_both_are(
    ast: Option<PolicyAction>,
    compiled: Option<PolicyAction>,
    expected: PolicyAction,
    ctx: &str,
) {
    assert_eq!(
        ast,
        Some(expected.clone()),
        "AST evaluator disagreed: {ctx}"
    );
    if let Some(c) = compiled {
        assert_eq!(
            c, expected,
            "compiled evaluator disagreed with oracle: {ctx}"
        );
    }
}

// ===========================================================================
// regex::matches — oracle: the `regex` crate directly.
// ===========================================================================

/// Patterns drawn from a safe alphabet: all valid regexes, none needing quote
/// or backslash escaping inside a `.reap` string literal.
fn safe_pattern() -> impl Strategy<Value = String> {
    prop::sample::select(vec![
        "^[a-z]+$",
        "[0-9]{2,4}",
        "^admin",
        "corp$",
        "a.c",
        "(foo|bar)",
        "[A-Z][a-z]*",
        "x+y*",
        "^$",
        "[aeiou]",
    ])
    .prop_map(String::from)
}

fn safe_text() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9]{0,12}".prop_map(|s| s)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    /// `regex::matches(user.text, PAT)` on both evaluators must equal
    /// `Regex::new(PAT).is_match(text)` — the independent crate oracle.
    #[test]
    fn regex_matches_agrees_with_the_regex_crate(text in safe_text(), pat in safe_pattern()) {
        let oracle = regex::Regex::new(&pat).unwrap().is_match(&text);
        let expected = if oracle { PolicyAction::Allow } else { PolicyAction::Deny };

        let policy_src = format!(
            r#"policy p {{ default: deny, rule r {{ allow if regex::matches(user.text, "{pat}") }} }}"#
        );
        let (ast, compiled) = eval_both(
            &policy_src,
            serde_json::json!({ "text": text }),
            "read",
            "res",
        );
        assert_both_are(ast, compiled, expected, &format!("text={text:?} pat={pat:?}"));
    }
}

// ===========================================================================
// jwt::decode — oracle: base64url + serde_json. SECURITY contract cases.
// ===========================================================================

fn b64url(s: &str) -> String {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    URL_SAFE_NO_PAD.encode(s)
}

fn jwt_from(header: &str, payload: &str, signature: &str) -> String {
    format!(
        "{}.{}.{}",
        b64url(header),
        b64url(payload),
        b64url(signature)
    )
}

#[test]
fn jwt_decode_navigates_claims_regardless_of_signature() {
    // A token with an ENTIRELY FORGED signature must still let a policy read
    // its claims (jwt::decode does not verify — OPA parity). The policy allows
    // on the claim; both evaluators must allow. This pins that "decode ==
    // interpret an already-authenticated artifact", the documented contract.
    let token = jwt_from(
        r#"{"alg":"HS256"}"#,
        r#"{"role":"admin"}"#,
        "totally-forged-not-a-real-mac",
    );
    let policy_src = r#"policy p { default: deny,
        rule r { allow if { claims := jwt::decode(user.token) && claims.role == "admin" } } }"#;
    let (ast, compiled) = eval_both(
        policy_src,
        serde_json::json!({ "token": token }),
        "read",
        "res",
    );
    assert_both_are(ast, compiled, PolicyAction::Allow, "forged-sig admin claim");
}

#[test]
fn jwt_decode_of_malformed_token_denies_not_errors() {
    // Malformed tokens decode to null (fail-soft), so a rule navigating claims
    // naturally DENIES. Oracle: any string that is not a well-formed 3-segment
    // base64url JWT must not yield a usable claim.
    let policy_src = r#"policy p { default: deny,
        rule r { allow if { claims := jwt::decode(user.token) && claims.role == "admin" } } }"#;
    for bad in ["not-a-jwt", "only.two", "", "a.b.c.d"] {
        let (ast, compiled) = eval_both(
            policy_src,
            serde_json::json!({ "token": bad }),
            "read",
            "res",
        );
        assert_both_are(
            ast,
            compiled,
            PolicyAction::Deny,
            &format!("malformed token {bad:?}"),
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// For a well-formed token, a policy gating on `claims.role == "admin"`
    /// allows iff the payload's role IS "admin" — oracle is the payload we
    /// encoded, independent of the decoder.
    #[test]
    fn jwt_role_claim_matches_encoded_payload(role in "[a-z]{3,8}") {
        let token = jwt_from(r#"{"alg":"none"}"#, &format!(r#"{{"role":"{role}"}}"#), "sig");
        let expected = if role == "admin" { PolicyAction::Allow } else { PolicyAction::Deny };
        let policy_src = r#"policy p { default: deny,
            rule r { allow if { claims := jwt::decode(user.token) && claims.role == "admin" } } }"#;
        let (ast, compiled) = eval_both(
            policy_src,
            serde_json::json!({ "token": token }),
            "read",
            "res",
        );
        assert_both_are(ast, compiled, expected, &format!("role={role:?}"));
    }
}

// ===========================================================================
// time — oracle: an explicit integer comparison against a PINNED clock.
// ===========================================================================

#[test]
fn time_is_before_now_respects_the_pinned_clock() {
    let _guard = CLOCK_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Pin "now" to 2_000_000_000 s (2033) in nanoseconds.
    let now_ns: i64 = 2_000_000_000 * 1_000_000_000;
    clock::set_injected_now_unix_ns(now_ns);

    // A token timestamp strictly before now → is_before(ts, now) is true → allow.
    // One at/after now → deny. Oracle: the integer comparison ts < now_ns.
    for (ts, expect) in [
        (now_ns - 1, PolicyAction::Allow),
        (now_ns, PolicyAction::Deny),
        (now_ns + 1, PolicyAction::Deny),
    ] {
        let policy_src = r#"policy p { default: deny,
            rule r { allow if time::is_before(user.ts, time::now_ns()) } }"#;
        let (ast, compiled) = eval_both(policy_src, serde_json::json!({ "ts": ts }), "read", "res");
        assert_both_are(ast, compiled, expect, &format!("ts={ts} now={now_ns}"));
    }

    clock::clear_injected_now();
}

#[test]
fn jwt_expiry_boundary_denies_expired_tokens_under_pinned_clock() {
    let _guard = CLOCK_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Pin now; a policy that allows only while the token's `exp` is strictly in
    // the future must DENY at and past the boundary. This is the expiry-boundary
    // deny case Testing T2 calls out (the curated jwt test only used a far-future
    // exp and never exercised the boundary).
    let now_secs: i64 = 1_900_000_000;
    clock::set_injected_now_unix_ns(now_secs * 1_000_000_000);

    // The grammar does not allow a function call on the right of a comparison,
    // so bind `now` to a variable first, then compare the claim against it.
    let policy_src = r#"policy p { default: deny,
        rule r { allow if { claims := jwt::decode(user.token) && now := time::now_secs() && claims.exp > now } } }"#;

    for (exp, expect) in [
        (now_secs + 100, PolicyAction::Allow), // still valid
        (now_secs, PolicyAction::Deny),        // exactly at expiry
        (now_secs - 1, PolicyAction::Deny),    // expired
    ] {
        let token = jwt_from(r#"{"alg":"none"}"#, &format!(r#"{{"exp":{exp}}}"#), "sig");
        let (ast, compiled) = eval_both(
            policy_src,
            serde_json::json!({ "token": token }),
            "read",
            "res",
        );
        assert_both_are(ast, compiled, expect, &format!("exp={exp} now={now_secs}"));
    }

    clock::clear_injected_now();
}

// ===========================================================================
// Self-test: the oracle actually drives the parser/evaluators (teeth).
// ===========================================================================

#[test]
fn harness_self_test_detects_disagreement_shape() {
    // A trivially true policy must allow on both paths — proves eval_both wires
    // through real evaluators, so a regression that neutered it (e.g. always
    // returning None) would surface as a failing assert elsewhere.
    let (ast, compiled) = eval_both(
        r#"policy p { default: deny, rule r { allow if true } }"#,
        serde_json::json!({}),
        "read",
        "res",
    );
    assert_eq!(ast, Some(PolicyAction::Allow));
    assert_eq!(compiled, Some(PolicyAction::Allow));
}
