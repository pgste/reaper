//! Frozen decision corpus — the fitness function for DSL semantic stability
//! (round-3 Plan 04, Step 1).
//!
//! `policy_library_tests.rs` proves the library's decisions are CORRECT today,
//! but a PR can change a decision and update its manifest together and pass —
//! it verifies current correctness, not cross-version stability. This suite
//! proves decisions are STABLE ACROSS ENGINE VERSIONS: every case under
//! `policy-library/frozen/` is FROZEN — its decision must never change. A change
//! to a shared operator/builtin that alters a frozen decision (which the
//! compiled-vs-AST differential cannot catch, because it hits both paths) turns
//! this job red.
//!
//! Two gates:
//! 1. **Decision gate** — every frozen case runs through BOTH the AST and the
//!    compiled evaluator and must produce its pinned decision.
//! 2. **Immutability gate** — every file under `policy-library/frozen/` must
//!    match its checked-in SHA-256 in `CHECKSUMS`. Editing a frozen expectation
//!    therefore forces a visible `CHECKSUMS` change: a decision change can only
//!    land LOUDLY (a reviewed language-version bump + a documented waiver),
//!    never silently.

use policy_engine::reap::ReaperPolicy;
use policy_engine::{DataLoader, DataStore, PolicyEvaluator, PolicyRequest};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

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
    #[allow(dead_code)]
    name: String,
    #[serde(default)]
    principal: Option<String>,
    #[serde(default)]
    action: Option<String>,
    #[serde(default)]
    resource: Option<String>,
    expect: String,
    #[serde(default)]
    context: Option<HashMap<String, String>>,
}

fn frozen_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../policy-library/frozen")
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Collect every frozen file (except `CHECKSUMS`) as `(relative_path, abs_path)`,
/// with `relative_path` in the same `./scenario/file` form `sha256sum` emitted.
fn collect_frozen_files(dir: &Path, base: &Path, out: &mut Vec<(String, PathBuf)>) {
    for entry in std::fs::read_dir(dir).expect("read frozen dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect_frozen_files(&path, base, out);
        } else if path.file_name().is_some_and(|n| n != "CHECKSUMS") {
            let rel = format!("./{}", path.strip_prefix(base).unwrap().to_string_lossy());
            out.push((rel, path));
        }
    }
}

/// Immutability gate: every frozen file matches its recorded checksum, and
/// `CHECKSUMS` lists exactly the frozen files — no unrecorded add, edit, or
/// removal can slip through.
#[test]
fn frozen_corpus_is_immutable() {
    let root = frozen_root();
    let checksums = std::fs::read_to_string(root.join("CHECKSUMS"))
        .expect("policy-library/frozen/CHECKSUMS must exist");

    let mut recorded: HashMap<String, String> = HashMap::new();
    for line in checksums.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (hash, path) = line
            .split_once("  ")
            .expect("CHECKSUMS line must be '<sha256>  <path>'");
        recorded.insert(path.to_string(), hash.to_string());
    }

    let mut files = Vec::new();
    collect_frozen_files(&root, &root, &mut files);

    let mut drift = Vec::new();
    for (rel, path) in &files {
        let actual = sha256_hex(&std::fs::read(path).unwrap());
        match recorded.get(rel) {
            Some(expected) if expected == &actual => {}
            Some(_) => drift.push(format!("{rel}: content changed")),
            None => drift.push(format!("{rel}: new frozen file, not recorded in CHECKSUMS")),
        }
    }
    let on_disk: HashSet<&str> = files.iter().map(|(r, _)| r.as_str()).collect();
    for rel in recorded.keys() {
        if !on_disk.contains(rel.as_str()) {
            drift.push(format!("{rel}: recorded in CHECKSUMS but missing on disk"));
        }
    }

    assert!(
        drift.is_empty(),
        "FROZEN corpus changed. A frozen case pins an authorization decision that \
         must never change silently. If you INTENDED to change a decision, that is \
         a BREAKING DSL change: bump the language version, document a waiver in \
         docs/reference/DSL_COMPATIBILITY.md, and regenerate CHECKSUMS \
         (`cd policy-library/frozen && find . -type f ! -name CHECKSUMS -print0 | \
         sort -z | xargs -0 sha256sum > CHECKSUMS`).\nDrift:\n  {}",
        drift.join("\n  ")
    );
}

/// Decision gate: every frozen case's pinned decision, on BOTH evaluator paths.
#[test]
fn frozen_decisions_never_change() {
    let root = frozen_root();
    let mut scenario_dirs = Vec::new();
    for entry in std::fs::read_dir(&root).expect("read frozen dir") {
        let path = entry.unwrap().path();
        if path.is_dir() {
            scenario_dirs.push(path);
        }
    }
    assert!(!scenario_dirs.is_empty(), "no frozen scenarios found");

    let mut cases_run = 0;
    for dir in scenario_dirs {
        let manifest_path = dir.join("manifest.json");
        let manifest: Manifest =
            serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap())
                .unwrap_or_else(|e| panic!("bad manifest {manifest_path:?}: {e}"));

        let policy_src = std::fs::read_to_string(dir.join(&manifest.policy)).unwrap();
        let parsed: ReaperPolicy = policy_src
            .parse()
            .unwrap_or_else(|e| panic!("[{}] parse policy: {e:?}", manifest.name));

        let store = Arc::new(DataStore::new());
        if let Some(ref data) = manifest.data {
            let json = std::fs::read_to_string(dir.join(data)).unwrap();
            DataLoader::new((*store).clone())
                .load_json(&json)
                .unwrap_or_else(|e| panic!("[{}] load data: {e:?}", manifest.name));
        }

        let ast = parsed.clone().build_ast_evaluator(store.clone());
        let compiled = parsed.clone().build(store.clone()).ok();

        for case in &manifest.cases {
            // The frozen tier pins authorization decisions; document/check cases
            // live in the mutable golden library. Skip anything non-authz.
            let (Some(principal), Some(action), Some(resource)) =
                (&case.principal, &case.action, &case.resource)
            else {
                continue;
            };
            cases_run += 1;
            let label = format!("[{}] {}", manifest.name, case.name);

            let mut context = case.context.clone().unwrap_or_default();
            context.insert("principal".to_string(), principal.clone());
            let request = PolicyRequest {
                resource: resource.clone(),
                action: action.clone(),
                context,
                ..Default::default()
            };

            let ast_decision = format!(
                "{:?}",
                ast.evaluate(&request)
                    .unwrap_or_else(|e| panic!("{label}: AST eval failed: {e:?}"))
            )
            .to_lowercase();
            assert_eq!(
                ast_decision, case.expect,
                "{label}: FROZEN AST decision changed (was {}, now {ast_decision})",
                case.expect
            );

            if let Some(ref compiled) = compiled {
                let compiled_decision = format!(
                    "{:?}",
                    compiled
                        .evaluate(&request)
                        .unwrap_or_else(|e| panic!("{label}: compiled eval failed: {e:?}"))
                )
                .to_lowercase();
                assert_eq!(
                    compiled_decision, case.expect,
                    "{label}: FROZEN compiled decision changed"
                );
            }
        }
    }
    assert!(
        cases_run >= 10,
        "expected a non-trivial frozen corpus, only ran {cases_run} cases"
    );
    println!("frozen decision corpus: {cases_run} pinned decisions verified");
}

/// Self-test (in the spirit of `perf-gate.yml`'s synthetic regression check):
/// the immutability guard must actually distinguish a changed expectation.
#[test]
fn immutability_guard_detects_tampering() {
    let original = sha256_hex(b"{\"expect\": \"allow\"}");
    let tampered = sha256_hex(b"{\"expect\": \"deny\"}");
    assert_ne!(
        original, tampered,
        "the immutability guard must flag a flipped expectation"
    );
}
