//! Differential property testing for CHECK MODE — the conftest-style
//! document-validation path used for Terraform plans, K8s manifests, and any
//! `input` JSON (`POST /api/v1/check`, `reaper-cli check`).
//!
//! Check mode is AST-only (no compiled twin), so instead of compiled/AST
//! parity this suite pins the semantics against an independent ORACLE: a
//! naive re-implementation of the check contract over the generated
//! structures that never touches the parser or evaluator. Every generated
//! (policy, document) pair must agree on:
//!   - the exact SET of violated deny rules (order-insensitive)
//!   - each violation's rendered message
//!   - the final `allowed` flag (no violations AND (default allow OR any
//!     allow rule matches))
//!
//! The generator deliberately exercises the null/undefined spec on input
//! paths: absent fields, absent parent objects (`input.tags.env` with no
//! `tags`), and absent documents entirely. Absence must never satisfy a
//! comparison other than an explicit `== null` / `!= null` presence check.
//!
//! Tuning: PROPTEST_CASES=1000 cargo test -p policy-engine --test
//! check_mode_differential_tests --release

use policy_engine::reap::ReaperPolicy;
use policy_engine::{DataStore, PolicyRequest};
use proptest::prelude::*;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Generated documents: a miniature Terraform/K8s-ish resource description
// with every field optional so absence paths are exercised constantly.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Doc {
    acl: Option<&'static str>,              // "private" | "public-read"
    encrypted: Option<bool>,                // SSE on/off
    replicas: Option<i64>,                  // 0..5
    tags_env: Option<Option<&'static str>>, // None = no tags object at all;
    // Some(None) = tags {} without env;
    // Some(Some(v)) = tags.env = v
    privileged: Option<bool>, // pod securityContext-ish flag
}

fn doc_strategy() -> impl Strategy<Value = Option<Doc>> {
    let doc = (
        prop::option::of(prop::sample::select(&["private", "public-read"][..])),
        prop::option::of(any::<bool>()),
        prop::option::of(0i64..5),
        prop::option::of(prop::option::of(prop::sample::select(&["prod", "dev"][..]))),
        prop::option::of(any::<bool>()),
    )
        .prop_map(|(acl, encrypted, replicas, tags_env, privileged)| Doc {
            acl,
            encrypted,
            replicas,
            tags_env,
            privileged,
        });
    // Sometimes there is no document at all: every input.* must go Null.
    prop::option::weighted(0.9, doc)
}

fn doc_to_json(doc: &Doc) -> serde_json::Value {
    let mut m = serde_json::Map::new();
    if let Some(acl) = doc.acl {
        m.insert("acl".into(), serde_json::json!(acl));
    }
    if let Some(e) = doc.encrypted {
        m.insert("encrypted".into(), serde_json::json!(e));
    }
    if let Some(r) = doc.replicas {
        m.insert("replicas".into(), serde_json::json!(r));
    }
    if let Some(tags) = &doc.tags_env {
        let mut t = serde_json::Map::new();
        if let Some(env) = tags {
            t.insert("env".into(), serde_json::json!(env));
        }
        m.insert("tags".into(), serde_json::Value::Object(t));
    }
    if let Some(p) = doc.privileged {
        m.insert("privileged".into(), serde_json::json!(p));
    }
    serde_json::Value::Object(m)
}

// ---------------------------------------------------------------------------
// Generated conditions over input paths.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum DocAtom {
    AclEq { negate: bool },                     // input.acl ==/!= "public-read"
    EncryptedIs { val: bool },                  // input.encrypted == true/false
    ReplicasCmp { op: &'static str, val: i64 }, // input.replicas <op> N
    TagEnvEq { negate: bool },                  // input.tags.env ==/!= "prod"
    TagsNull { negate: bool },                  // input.tags ==/!= null
    PrivilegedTrue,                             // input.privileged == true
}

impl DocAtom {
    fn render(&self) -> String {
        match self {
            DocAtom::AclEq { negate } => {
                format!(
                    "input.acl {} \"public-read\"",
                    if *negate { "!=" } else { "==" }
                )
            }
            DocAtom::EncryptedIs { val } => format!("input.encrypted == {val}"),
            DocAtom::ReplicasCmp { op, val } => format!("input.replicas {op} {val}"),
            DocAtom::TagEnvEq { negate } => {
                format!(
                    "input.tags.env {} \"prod\"",
                    if *negate { "!=" } else { "==" }
                )
            }
            DocAtom::TagsNull { negate } => {
                format!("input.tags {} null", if *negate { "!=" } else { "==" })
            }
            DocAtom::PrivilegedTrue => "input.privileged == true".to_string(),
        }
    }

    /// The ORACLE for a single atom, honoring the null/undefined spec:
    /// a missing path satisfies NOTHING except an explicit null check.
    fn oracle(&self, doc: Option<&Doc>) -> bool {
        match self {
            DocAtom::AclEq { negate } => match doc.and_then(|d| d.acl) {
                Some(acl) => (acl == "public-read") != *negate,
                None => false,
            },
            DocAtom::EncryptedIs { val } => match doc.and_then(|d| d.encrypted) {
                Some(e) => e == *val,
                None => false,
            },
            DocAtom::ReplicasCmp { op, val } => match doc.and_then(|d| d.replicas) {
                Some(r) => match *op {
                    "==" => r == *val,
                    "!=" => r != *val,
                    ">" => r > *val,
                    ">=" => r >= *val,
                    "<" => r < *val,
                    "<=" => r <= *val,
                    _ => unreachable!(),
                },
                None => false,
            },
            DocAtom::TagEnvEq { negate } => match doc.and_then(|d| d.tags_env.flatten()) {
                Some(env) => (env == "prod") != *negate,
                None => false,
            },
            DocAtom::TagsNull { negate } => {
                let present = doc.is_some_and(|d| d.tags_env.is_some());
                if *negate {
                    present
                } else {
                    !present
                }
            }
            DocAtom::PrivilegedTrue => doc.and_then(|d| d.privileged) == Some(true),
        }
    }
}

fn doc_atom_strategy() -> impl Strategy<Value = DocAtom> {
    prop_oneof![
        any::<bool>().prop_map(|negate| DocAtom::AclEq { negate }),
        any::<bool>().prop_map(|val| DocAtom::EncryptedIs { val }),
        (
            prop::sample::select(&["==", "!=", ">", ">=", "<", "<="][..]),
            0i64..5
        )
            .prop_map(|(op, val)| DocAtom::ReplicasCmp { op, val }),
        any::<bool>().prop_map(|negate| DocAtom::TagEnvEq { negate }),
        any::<bool>().prop_map(|negate| DocAtom::TagsNull { negate }),
        Just(DocAtom::PrivilegedTrue),
    ]
}

#[derive(Debug, Clone)]
struct DocCond {
    atoms: Vec<DocAtom>,
    any: bool,
}

impl DocCond {
    fn render(&self) -> String {
        let joiner = if self.any { " || " } else { " && " };
        let body = self
            .atoms
            .iter()
            .map(DocAtom::render)
            .collect::<Vec<_>>()
            .join(joiner);
        format!("{{ {body} }}")
    }
    fn oracle(&self, doc: Option<&Doc>) -> bool {
        if self.any {
            self.atoms.iter().any(|a| a.oracle(doc))
        } else {
            self.atoms.iter().all(|a| a.oracle(doc))
        }
    }
}

#[derive(Debug, Clone)]
struct DocPolicy {
    default_allow: bool,
    denies: Vec<DocCond>, // rule d{i}, message "violation-{i}"
    allows: Vec<DocCond>, // rule a{i}
}

impl DocPolicy {
    fn render(&self) -> String {
        let mut out = String::from("policy checkprop {\n");
        let _ = writeln!(
            out,
            "    default: {},",
            if self.default_allow { "allow" } else { "deny" }
        );
        for (i, cond) in self.denies.iter().enumerate() {
            let _ = writeln!(
                out,
                "    rule d{i} {{ deny with message \"violation-{i}\" if {} }}",
                cond.render()
            );
        }
        for (i, cond) in self.allows.iter().enumerate() {
            let _ = writeln!(out, "    rule a{i} {{ allow if {} }}", cond.render());
        }
        out.push('}');
        out
    }
}

fn doc_cond_strategy() -> impl Strategy<Value = DocCond> {
    (
        prop::collection::vec(doc_atom_strategy(), 1..3),
        any::<bool>(),
    )
        .prop_map(|(atoms, any)| DocCond { atoms, any })
}

fn doc_policy_strategy() -> impl Strategy<Value = DocPolicy> {
    (
        any::<bool>(),
        prop::collection::vec(doc_cond_strategy(), 1..4),
        prop::collection::vec(doc_cond_strategy(), 0..3),
    )
        .prop_map(|(default_allow, denies, allows)| DocPolicy {
            default_allow,
            denies,
            allows,
        })
}

/// The ORACLE for the whole check contract (mirrors the documented behavior
/// of `check_with_input`, re-derived from the spec, not the code):
/// violations = every deny rule whose condition holds; allowed = no
/// violations AND (default allow OR any allow rule matches).
fn oracle_check(policy: &DocPolicy, doc: Option<&Doc>) -> (bool, Vec<String>) {
    let violations: Vec<String> = policy
        .denies
        .iter()
        .enumerate()
        .filter(|(_, c)| c.oracle(doc))
        .map(|(i, _)| format!("d{i}"))
        .collect();
    let allowed = violations.is_empty()
        && (policy.default_allow || policy.allows.iter().any(|c| c.oracle(doc)));
    (allowed, violations)
}

/// Explicit `cases:` would silently OVERRIDE the PROPTEST_CASES env var —
/// read it ourselves so scale runs actually scale.
fn cases_from_env(default: u32) -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: cases_from_env(128),
        max_shrink_iters: 2048,
        ..ProptestConfig::default()
    })]

    #[test]
    fn check_mode_matches_the_oracle(
        policy in doc_policy_strategy(),
        doc in doc_strategy(),
    ) {
        let source = policy.render();
        let parsed: ReaperPolicy = source
            .parse()
            .unwrap_or_else(|e| panic!("generated policy must parse: {e:?}\n{source}"));
        let store = Arc::new(DataStore::new());
        let ast = parsed.build_ast_evaluator(store);

        let json = doc.as_ref().map(doc_to_json);
        let request = PolicyRequest {
            resource: "plan.json".to_string(),
            action: "check".to_string(),
            context: HashMap::new(),

            ..Default::default()
        };

        let result = ast
            .check_with_input(&request, json.as_ref())
            .unwrap_or_else(|e| panic!("check must not error: {e:?}\npolicy:\n{source}\ndoc: {json:?}"));

        let (expected_allowed, expected_violations) = oracle_check(&policy, doc.as_ref());

        let mut got: Vec<&str> = result.violations.iter().map(|v| v.rule.as_str()).collect();
        got.sort_unstable();
        let mut want: Vec<&str> = expected_violations.iter().map(String::as_str).collect();
        want.sort_unstable();
        prop_assert_eq!(
            got, want,
            "VIOLATION-SET BREAK\npolicy:\n{}\ndoc: {:?}",
            source, json
        );

        prop_assert_eq!(
            result.allowed, expected_allowed,
            "ALLOWED-FLAG BREAK\npolicy:\n{}\ndoc: {:?}\nviolations: {:?}",
            source, json, result.violations
        );

        // Every violation message must render exactly as written.
        for v in &result.violations {
            let idx: usize = v.rule[1..].parse().unwrap();
            let expected_message = format!("violation-{idx}");
            prop_assert_eq!(
                v.message.as_deref(), Some(expected_message.as_str()),
                "MESSAGE BREAK for rule {}\npolicy:\n{}",
                &v.rule, source
            );
        }
    }
}
