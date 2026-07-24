//! Native leg of the wasm parity contract.
//!
//! `crates/policy-engine/tests/policy_library_tests.rs` already enforces that
//! every policy-library manifest case holds on the AST and compiled
//! evaluators. This suite runs the SAME manifest cases through the
//! `reaper-wasm` wrapper (`ReaperEngine`) — the exact code the wasm artifact
//! exports — natively. The Node smoke test (`tests/node/smoke.mjs`) then runs
//! the same manifests through the actual `.wasm` build. Together: manifest
//! expectations ⇒ native wrapper ⇒ wasm artifact, one shared oracle.
//!
//! Document-mode cases (`input`/`violations`) run through the wrapper's
//! `checkDocument` surface (slice 3): allowed flag AND exact violated-rule
//! set asserted per case — the full 82-case library corpus, nothing skipped.

// Test-harness file: the workspace panic gate targets reachable production
// code; helper fns here fall outside the `allow-*-in-tests` heuristic (same
// pattern as reaper-core/tests/reaper_bdd_tests.rs).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use reaper_wasm::ReaperEngine;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
struct Manifest {
    name: String,
    policy: String,
    #[serde(default)]
    data: Option<String>,
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
struct Case {
    name: String,
    #[serde(default)]
    principal: Option<String>,
    #[serde(default)]
    action: Option<String>,
    #[serde(default)]
    resource: Option<String>,
    #[serde(default)]
    input: Option<String>,
    expect: String,
    #[serde(default)]
    violations: Option<Vec<String>>,
    #[serde(default)]
    context: Option<HashMap<String, String>>,
}

/// Run one document-mode case through the wrapper's check surface and assert
/// the manifest's expectations (allowed flag + exact violated-rule set) —
/// the same contract `policy_library_tests.rs` enforces on the AST evaluator.
fn assert_check_case(
    engine: &ReaperEngine,
    dir: &Path,
    policy_src: &str,
    label: &str,
    case: &Case,
) {
    let input_file = case.input.as_deref().expect("document case input");
    let input_json = std::fs::read_to_string(dir.join(input_file)).expect("read input document");
    let action = case.action.as_deref().unwrap_or("check");

    let result_json = engine
        .check_document_impl(policy_src, &input_json, action, input_file)
        .unwrap_or_else(|e| panic!("{label}: checkDocument failed: {e}"));
    let result: serde_json::Value = serde_json::from_str(&result_json).expect("check json");

    let expect_allowed = case.expect == "allow";
    assert_eq!(
        result["allowed"].as_bool(),
        Some(expect_allowed),
        "{label}: allowed mismatch"
    );
    if let Some(ref expected) = case.violations {
        let mut got: Vec<String> = result["violations"]
            .as_array()
            .expect("violations array")
            .iter()
            .map(|v| v["rule"].as_str().expect("rule name").to_string())
            .collect();
        let mut want = expected.clone();
        got.sort_unstable();
        want.sort_unstable();
        assert_eq!(got, want, "{label}: violation set mismatch");
    }
}

fn library_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../policy-library")
}

fn find_manifests(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read library dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            find_manifests(&path, out);
        } else if path.file_name().is_some_and(|n| n == "manifest.json") {
            out.push(path);
        }
    }
}

fn decision_of(decision_json: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(decision_json).expect("decision is JSON");
    v.get("decision")
        .and_then(|d| d.as_str())
        .expect("decision field")
        .to_lowercase()
}

#[test]
fn wasm_wrapper_meets_every_library_manifest() {
    let root = library_root();
    let mut manifests = Vec::new();
    find_manifests(&root, &mut manifests);
    assert!(
        manifests.len() >= 8,
        "expected the full library, found {} manifests",
        manifests.len()
    );

    let mut scenarios = 0;
    let mut cases_run = 0;
    let mut document_cases_run = 0;
    let mut ast_fallback_scenarios: Vec<String> = Vec::new();

    for manifest_path in manifests {
        let dir = manifest_path.parent().expect("manifest has parent dir");
        let manifest: Manifest =
            serde_json::from_str(&std::fs::read_to_string(&manifest_path).expect("read manifest"))
                .unwrap_or_else(|e| panic!("bad manifest {manifest_path:?}: {e}"));

        // Fresh engine per scenario — same isolation the library runner uses.
        let engine = ReaperEngine::new();
        let policy_src = std::fs::read_to_string(dir.join(&manifest.policy))
            .unwrap_or_else(|e| panic!("[{}] read policy: {e}", manifest.name));

        // Import-using scenarios (language v3) are out of the wasm wrapper's
        // reach BY CONTRACT: the wrapper deploys policy SOURCE strings and
        // has no filesystem, and imports resolve at load time (file load /
        // bundle build). Such policies reach wasm-class consumers as
        // compiled bundles with the imports already embedded. The scenario
        // stays covered by the native library + frozen corpus runners.
        if policy_src
            .lines()
            .any(|l| l.trim_start().starts_with("import "))
        {
            continue;
        }

        // Document-mode scenario (all cases carry `input`): slice 3 — run
        // through the wrapper's checkDocument surface, asserting allowed +
        // the exact violation set per case. No deploy needed (check parses
        // the source per call, matching the CLI/agent check semantics).
        if manifest.cases.iter().all(|c| c.input.is_some()) {
            if let Some(ref data) = manifest.data {
                let json = std::fs::read_to_string(dir.join(data))
                    .unwrap_or_else(|e| panic!("[{}] read data: {e}", manifest.name));
                engine
                    .load_entities_json_impl(&json)
                    .unwrap_or_else(|e| panic!("[{}] load data: {e}", manifest.name));
            }
            for case in &manifest.cases {
                let label = format!("[{}] {}", manifest.name, case.name);
                assert_check_case(&engine, dir, &policy_src, &label, case);
                document_cases_run += 1;
            }
            scenarios += 1;
            continue;
        }

        if let Some(ref data) = manifest.data {
            let json = std::fs::read_to_string(dir.join(data))
                .unwrap_or_else(|e| panic!("[{}] read data: {e}", manifest.name));
            engine
                .load_entities_json_impl(&json)
                .unwrap_or_else(|e| panic!("[{}] load data: {e}", manifest.name));
        }

        let policy_id = engine
            .deploy_policy_impl(&manifest.name, &policy_src)
            .unwrap_or_else(|e| panic!("[{}] deploy: {e}", manifest.name));

        // Compiled-PRIMARY contract: the wrapper must serve this policy from
        // the same evaluator tier the engine would pick natively. Ground
        // truth is computed independently: if `ReaperPolicy::build` (the
        // compiler) succeeds, the wrapper MUST report the compiled tier —
        // an AST fallback there would mean the wasm surface silently
        // downgraded the primary path.
        let compiles = {
            use policy_engine::{DataLoader as PeLoader, DataStore as PeStore, ReaperPolicy};
            let probe_store = std::sync::Arc::new(PeStore::new());
            if let Some(ref data) = manifest.data {
                let json = std::fs::read_to_string(dir.join(data)).expect("re-read data");
                PeLoader::new((*probe_store).clone())
                    .load_json(&json)
                    .unwrap_or_else(|e| panic!("[{}] probe load: {e}", manifest.name));
            }
            policy_src
                .parse::<ReaperPolicy>()
                .expect("policy parsed once already")
                .build(probe_store)
                .is_ok()
        };
        let tier = engine
            .evaluator_type_impl(&policy_id)
            .unwrap_or_else(|e| panic!("[{}] evaluator_type: {e}", manifest.name));
        let expected_tier = if compiles {
            "reaper_dsl"
        } else {
            "ReapAstEvaluator"
        };
        assert_eq!(
            tier, expected_tier,
            "[{}] evaluator tier mismatch (compiled-primary contract)",
            manifest.name
        );
        if !compiles {
            ast_fallback_scenarios.push(manifest.name.clone());
        }

        for case in &manifest.cases {
            if case.input.is_some() {
                // Mixed manifest: document case inside an authz scenario.
                let label = format!("[{}] {}", manifest.name, case.name);
                assert_check_case(&engine, dir, &policy_src, &label, case);
                document_cases_run += 1;
                continue;
            }
            cases_run += 1;
            let label = format!("[{}] {}", manifest.name, case.name);

            let principal = case.principal.as_deref().expect("authz case principal");
            let action = case.action.as_deref().expect("authz case action");
            let resource = case.resource.as_deref().expect("authz case resource");
            let context_json = case
                .context
                .as_ref()
                .map(|c| serde_json::to_string(c).expect("context serializes"));

            // Single-policy path — the wasm `evaluate` export.
            let decision_json = engine
                .evaluate_impl(
                    &policy_id,
                    principal,
                    action,
                    resource,
                    context_json.as_deref(),
                )
                .unwrap_or_else(|e| panic!("{label}: evaluate failed: {e}"));
            assert_eq!(
                decision_of(&decision_json),
                case.expect,
                "{label}: single-policy decision mismatch"
            );

            // Evaluate-all path — the wasm `evaluateAll` export. With exactly
            // one deployed policy its decision must agree with the
            // single-policy path on this corpus (the per-policy default
            // decides unmatched requests in both).
            let all_json = engine
                .evaluate_all_impl(principal, action, resource, context_json.as_deref())
                .unwrap_or_else(|e| panic!("{label}: evaluate_all failed: {e}"));
            assert_eq!(
                decision_of(&all_json),
                case.expect,
                "{label}: evaluate_all decision mismatch"
            );
        }
        scenarios += 1;
    }

    assert!(cases_run >= 40, "suspiciously few cases ran: {cases_run}");
    assert!(
        document_cases_run >= 15,
        "suspiciously few document-mode cases ran: {document_cases_run}"
    );

    // The checked-in fallback list is the cross-target tier contract: the
    // Node leg asserts the wasm artifact serves the compiled tier for every
    // scenario NOT on this list. This assertion keeps the list honest — if a
    // library policy starts (or stops) compiling, the fixture must move with
    // it, and the Node leg follows automatically.
    let fixture_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ast-fallback-scenarios.json");
    let mut expected_fallbacks: Vec<String> = serde_json::from_str(
        &std::fs::read_to_string(&fixture_path).expect("read ast-fallback-scenarios.json"),
    )
    .expect("fixture is a JSON string array");
    expected_fallbacks.sort();
    ast_fallback_scenarios.sort();
    assert_eq!(
        ast_fallback_scenarios, expected_fallbacks,
        "AST-fallback scenario set drifted — update \
         tests/fixtures/ast-fallback-scenarios.json to match reality"
    );

    println!(
        "wasm-wrapper parity: {scenarios} scenarios, {cases_run} authz cases + \
         {document_cases_run} document-mode cases verified; \
         compiled tier on all non-document scenarios, AST fallback on {:?}",
        ast_fallback_scenarios
    );
}

#[test]
fn redeploy_is_atomic_hot_swap_not_static() {
    // The wasm embedding is dynamic like the agent: redeploying the same
    // policy NAME keeps the id, bumps the version, and flips decisions
    // in place; removePolicy retires it.
    let engine = ReaperEngine::new();
    engine
        .load_entities_json_impl(r#"{"entities":[{"id":"svc","type":"User","attributes":{}}]}"#)
        .expect("load entity");

    let v1 = r#"policy swap { default: deny, rule r { allow if context.env == "prod" } }"#;
    let v2 = r#"policy swap { default: deny, rule r { deny if context.env == "prod" } }"#;

    let id1 = engine.deploy_policy_impl("swap", v1).expect("deploy v1");
    let d1 = engine
        .evaluate_impl(&id1, "svc", "read", "x", Some(r#"{"env":"prod"}"#))
        .expect("eval v1");
    assert_eq!(decision_of(&d1), "allow");

    let id2 = engine.deploy_policy_impl("swap", v2).expect("redeploy v2");
    assert_eq!(id1, id2, "hot-swap must keep the policy id");
    let d2 = engine
        .evaluate_impl(&id2, "svc", "read", "x", Some(r#"{"env":"prod"}"#))
        .expect("eval v2");
    assert_eq!(decision_of(&d2), "deny", "redeploy must flip the decision");
    let v2_json: serde_json::Value = serde_json::from_str(&d2).expect("decision json");
    assert_eq!(
        v2_json["policy_version"].as_u64(),
        Some(2),
        "hot-swap must bump the version"
    );

    let removed_version = engine.remove_policy_impl(&id2).expect("remove");
    assert_eq!(removed_version, 2);
    assert!(
        engine
            .evaluate_impl(&id2, "svc", "read", "x", None)
            .is_err(),
        "evaluating a removed policy must fail"
    );
}

#[test]
fn context_coercion_matches_agent_fast_path() {
    // Scalars coerce to strings; nested values drop; caller-supplied
    // context.principal is overridden by the typed principal — the same
    // rules the agent applies before the engine sees the request.
    let engine = ReaperEngine::new();
    let policy_id = engine
        .deploy_policy_impl(
            "ctx-coercion",
            r#"
policy ctx_coercion {
    default: deny,

    rule tier_gate {
        allow if {
            context.tier == "3" &&
            context.beta == "true"
        }
    }
}
"#,
        )
        .expect("deploy");

    engine
        .load_entities_json_impl(r#"{"entities":[{"id":"svc-1","type":"User","attributes":{}}]}"#)
        .expect("load principal entity");

    let decision = engine
        .evaluate_impl(
            &policy_id,
            "svc-1",
            "read",
            "thing",
            Some(r#"{"tier": 3, "beta": true, "nested": {"dropped": 1}}"#),
        )
        .expect("evaluate");
    assert_eq!(decision_of(&decision), "allow", "number/bool must coerce");

    let denied = engine
        .evaluate_impl(&policy_id, "svc-1", "read", "thing", Some(r#"{"tier": 2}"#))
        .expect("evaluate");
    assert_eq!(decision_of(&denied), "deny");

    let err = engine
        .evaluate_impl(&policy_id, "svc-1", "read", "thing", Some("[1,2]"))
        .expect_err("non-object context must error");
    assert!(err.contains("JSON object"), "unexpected error: {err}");
}
