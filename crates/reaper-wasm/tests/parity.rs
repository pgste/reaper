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
//! Document-mode cases (`input`/`violations`) are skipped: check mode is not
//! part of the slice-2 wasm surface (tracked for slice 3 in
//! `plans/round-2/F2-wasm-target.md`).

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
    context: Option<HashMap<String, String>>,
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
    let mut skipped_document_cases = 0;
    let mut ast_fallback_scenarios: Vec<String> = Vec::new();

    for manifest_path in manifests {
        let dir = manifest_path.parent().expect("manifest has parent dir");
        let manifest: Manifest =
            serde_json::from_str(&std::fs::read_to_string(&manifest_path).expect("read manifest"))
                .unwrap_or_else(|e| panic!("bad manifest {manifest_path:?}: {e}"));

        // Document-mode scenario (all cases carry `input`): outside the
        // slice-2 wasm surface.
        if manifest.cases.iter().all(|c| c.input.is_some()) {
            skipped_document_cases += manifest.cases.len();
            continue;
        }

        // Fresh engine per scenario — same isolation the library runner uses.
        let engine = ReaperEngine::new();
        let policy_src = std::fs::read_to_string(dir.join(&manifest.policy))
            .unwrap_or_else(|e| panic!("[{}] read policy: {e}", manifest.name));

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
                skipped_document_cases += 1;
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
        "wasm-wrapper parity: {scenarios} scenarios, {cases_run} authz cases verified \
         ({skipped_document_cases} document-mode cases out of slice-2 scope); \
         compiled tier on {} scenarios, AST fallback on {:?}",
        scenarios - ast_fallback_scenarios.len(),
        ast_fallback_scenarios
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
