//! Executable policy library: every scenario under `policy-library/` declares
//! its expected decisions in `manifest.json`; this runner enforces them.
//!
//! - Authorization cases run through the AST evaluator AND, when the policy
//!   compiles, the compiled evaluator — asserting both agree with the
//!   expectation (parity is a contract, not a hope).
//! - Document cases run through check mode, asserting `allowed` and the exact
//!   set of violated rules.

use policy_engine::reap::ReaperPolicy;
use policy_engine::{DataLoader, DataStore, PolicyEvaluator, PolicyRequest};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Deserialize)]
struct Manifest {
    name: String,
    #[allow(dead_code)]
    source: String,
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
    /// Extra request context entries (e.g. a support ticket id).
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

#[test]
fn every_library_scenario_meets_its_manifest() {
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
    for manifest_path in manifests {
        let dir = manifest_path.parent().unwrap();
        let manifest: Manifest =
            serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap())
                .unwrap_or_else(|e| panic!("bad manifest {manifest_path:?}: {e}"));

        // Load policy + data. File-based load so `import "..." as ns`
        // declarations resolve against the scenario directory (language v3).
        let parsed: ReaperPolicy = ReaperPolicy::from_file(dir.join(&manifest.policy))
            .unwrap_or_else(|e| panic!("[{}] parse policy: {e:?}", manifest.name));

        let store = Arc::new(DataStore::new());
        if let Some(ref data) = manifest.data {
            let json = std::fs::read_to_string(dir.join(data))
                .unwrap_or_else(|e| panic!("[{}] read data: {e}", manifest.name));
            DataLoader::new((*store).clone())
                .load_json(&json)
                .unwrap_or_else(|e| panic!("[{}] load data: {e:?}", manifest.name));
        }

        let ast = parsed.clone().build_ast_evaluator(store.clone());
        let compiled = parsed.clone().build(store.clone()).ok();

        for case in &manifest.cases {
            cases_run += 1;
            let label = format!("[{}] {}", manifest.name, case.name);

            if let Some(ref input_file) = case.input {
                // Document case: check mode, exact violation set.
                let doc: serde_json::Value =
                    serde_json::from_str(&std::fs::read_to_string(dir.join(input_file)).unwrap())
                        .unwrap_or_else(|e| panic!("{label}: bad input json: {e}"));
                let request = PolicyRequest {
                    resource: input_file.clone(),
                    action: case.action.clone().unwrap_or_else(|| "check".to_string()),
                    context: HashMap::new(),

                    ..Default::default()
                };
                let result = ast
                    .check_with_input(&request, Some(&doc))
                    .unwrap_or_else(|e| panic!("{label}: check failed: {e:?}"));

                let expect_allowed = case.expect == "allow";
                assert_eq!(result.allowed, expect_allowed, "{label}: allowed mismatch");
                if let Some(ref expected) = case.violations {
                    let mut got: Vec<&str> =
                        result.violations.iter().map(|v| v.rule.as_str()).collect();
                    let mut want: Vec<&str> = expected.iter().map(String::as_str).collect();
                    got.sort_unstable();
                    want.sort_unstable();
                    assert_eq!(got, want, "{label}: violation set mismatch");
                }
                // Every violation with a message must have rendered non-empty.
                for v in &result.violations {
                    if let Some(msg) = &v.message {
                        assert!(!msg.is_empty(), "{label}: empty message for {}", v.rule);
                    }
                }
            } else {
                // Authorization case: AST decision must match, and the
                // compiled evaluator (when the policy compiles) must agree.
                let mut context = case.context.clone().unwrap_or_default();
                context.insert(
                    "principal".to_string(),
                    case.principal.clone().expect("authz case needs principal"),
                );
                let request = PolicyRequest {
                    resource: case.resource.clone().expect("authz case needs resource"),
                    action: case.action.clone().expect("authz case needs action"),
                    context,

                    ..Default::default()
                };

                let decision = format!(
                    "{:?}",
                    ast.evaluate(&request)
                        .unwrap_or_else(|e| { panic!("{label}: AST evaluation failed: {e:?}") })
                )
                .to_lowercase();
                assert_eq!(decision, case.expect, "{label}: AST decision mismatch");

                if let Some(ref compiled) = compiled {
                    let cdecision = format!(
                        "{:?}",
                        compiled.evaluate(&request).unwrap_or_else(|e| {
                            panic!("{label}: compiled evaluation failed: {e:?}")
                        })
                    )
                    .to_lowercase();
                    assert_eq!(
                        cdecision, case.expect,
                        "{label}: compiled decision mismatch (parity break)"
                    );
                }
            }
        }
        scenarios += 1;
    }
    println!("policy library: {scenarios} scenarios, {cases_run} cases verified");
}
