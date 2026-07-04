//! Policy examples library: browse and run the scenarios under
//! `policy-library/`. The same manifests drive the Rust CI suite
//! (`policy_library_tests.rs`) and — later — the control-plane UI's template
//! gallery (each scenario ships a walkthrough README.md + machine-readable
//! manifest.json, so a UI can render the story and offer one-click runs).

use policy_engine::reap::ReaperPolicy;
use policy_engine::{DataLoader, DataStore, PolicyEvaluator, PolicyRequest};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Deserialize)]
struct Manifest {
    name: String,
    #[serde(default)]
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
    #[serde(default)]
    context: Option<HashMap<String, String>>,
}

fn root(path: Option<&str>) -> anyhow::Result<PathBuf> {
    let candidates = [
        path.map(PathBuf::from),
        std::env::var("REAPER_LIBRARY_PATH").ok().map(PathBuf::from),
        Some(PathBuf::from("policy-library")),
    ];
    for candidate in candidates.into_iter().flatten() {
        if candidate.is_dir() {
            return Ok(candidate);
        }
    }
    anyhow::bail!(
        "policy library not found: pass --path, set REAPER_LIBRARY_PATH, or run from the repo root"
    )
}

fn scenarios(root: &Path) -> anyhow::Result<Vec<(String, PathBuf)>> {
    let mut found = Vec::new();
    walk(root, root, &mut found)?;
    found.sort();
    Ok(found)
}

fn walk(base: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            walk(base, &path, out)?;
        } else if path.file_name().is_some_and(|n| n == "manifest.json") {
            let dir = path.parent().unwrap();
            let id = dir
                .strip_prefix(base)
                .unwrap_or(dir)
                .to_string_lossy()
                .replace('\\', "/");
            out.push((id, dir.to_path_buf()));
        }
    }
    Ok(())
}

fn read_manifest(dir: &Path) -> anyhow::Result<Manifest> {
    let raw = std::fs::read_to_string(dir.join("manifest.json"))?;
    Ok(serde_json::from_str(&raw)?)
}

/// `reaper-cli library list`
pub fn list(path: Option<&str>) -> anyhow::Result<()> {
    let root = root(path)?;
    println!("📚 Policy library ({})\n", root.display());
    println!("{:<34} {:>5}  {}", "ID", "CASES", "NAME");
    for (id, dir) in scenarios(&root)? {
        let manifest = read_manifest(&dir)?;
        println!("{:<34} {:>5}  {}", id, manifest.cases.len(), manifest.name);
    }
    println!("\nrun one:   reaper-cli library run <id>");
    println!("read one:  reaper-cli library show <id>");
    Ok(())
}

/// `reaper-cli library show <id>`
pub fn show(id: &str, path: Option<&str>) -> anyhow::Result<()> {
    let root = root(path)?;
    let dir = root.join(id);
    let manifest = read_manifest(&dir)
        .map_err(|_| anyhow::anyhow!("scenario '{id}' not found (try `library list`)"))?;

    println!("# {}\nsource: {}\n", manifest.name, manifest.source);
    let readme = dir.join("README.md");
    if readme.is_file() {
        println!("{}", std::fs::read_to_string(readme)?);
    }
    println!("--- {} ---\n", manifest.policy);
    println!("{}", std::fs::read_to_string(dir.join(&manifest.policy))?);
    Ok(())
}

/// `reaper-cli library run [<id>]` — exit code 1 if any case fails.
pub fn run(id: Option<&str>, path: Option<&str>) -> anyhow::Result<()> {
    let root = root(path)?;
    let targets: Vec<(String, PathBuf)> = match id {
        Some(id) => {
            let dir = root.join(id);
            if !dir.join("manifest.json").is_file() {
                anyhow::bail!("scenario '{id}' not found (try `library list`)");
            }
            vec![(id.to_string(), dir)]
        }
        None => scenarios(&root)?,
    };

    let mut passed = 0usize;
    let mut failed = 0usize;
    for (id, dir) in targets {
        let manifest = read_manifest(&dir)?;
        println!("▶ {id} — {}", manifest.name);
        match run_scenario(&dir, &manifest) {
            Ok(results) => {
                for (case, ok, detail) in results {
                    if ok {
                        passed += 1;
                        println!("   ✅ {case}");
                    } else {
                        failed += 1;
                        println!("   ❌ {case} — {detail}");
                    }
                }
            }
            Err(e) => {
                failed += 1;
                println!("   ❌ scenario failed to load: {e}");
            }
        }
    }
    println!("\n{passed} passed, {failed} failed");
    if failed > 0 {
        std::process::exit(1);
    }
    Ok(())
}

fn run_scenario(dir: &Path, manifest: &Manifest) -> anyhow::Result<Vec<(String, bool, String)>> {
    let policy_src = std::fs::read_to_string(dir.join(&manifest.policy))?;
    let parsed: ReaperPolicy = policy_src
        .parse()
        .map_err(|e| anyhow::anyhow!("parse policy: {e:?}"))?;

    let store = Arc::new(DataStore::new());
    if let Some(ref data) = manifest.data {
        let json = std::fs::read_to_string(dir.join(data))?;
        DataLoader::new((*store).clone())
            .load_json(&json)
            .map_err(|e| anyhow::anyhow!("load data: {e:?}"))?;
    }
    let ast = parsed.clone().build_ast_evaluator(store.clone());
    let compiled = parsed.build(store).ok();

    let mut results = Vec::new();
    for case in &manifest.cases {
        let (ok, detail) = if let Some(ref input_file) = case.input {
            let doc: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(dir.join(input_file))?)?;
            let request = PolicyRequest {
                resource: input_file.clone(),
                action: case.action.clone().unwrap_or_else(|| "check".to_string()),
                context: case.context.clone().unwrap_or_default(),
            };
            match ast.check_with_input(&request, Some(&doc)) {
                Ok(result) => {
                    let expect_allowed = case.expect == "allow";
                    let mut ok = result.allowed == expect_allowed;
                    let mut detail =
                        format!("allowed={} (expected {})", result.allowed, expect_allowed);
                    if let Some(ref expected) = case.violations {
                        let mut got: Vec<&str> =
                            result.violations.iter().map(|v| v.rule.as_str()).collect();
                        let mut want: Vec<&str> = expected.iter().map(String::as_str).collect();
                        got.sort_unstable();
                        want.sort_unstable();
                        if got != want {
                            ok = false;
                            detail = format!("violations {got:?} != expected {want:?}");
                        }
                    }
                    (ok, detail)
                }
                Err(e) => (false, format!("check error: {e:?}")),
            }
        } else {
            let mut context = case.context.clone().unwrap_or_default();
            context.insert(
                "principal".to_string(),
                case.principal.clone().unwrap_or_default(),
            );
            let request = PolicyRequest {
                resource: case.resource.clone().unwrap_or_default(),
                action: case.action.clone().unwrap_or_default(),
                context,
            };
            match ast.evaluate(&request) {
                Ok(decision) => {
                    let got = format!("{decision:?}").to_lowercase();
                    let mut ok = got == case.expect;
                    let mut detail = format!("{got} (expected {})", case.expect);
                    // Compiled parity when the policy compiles.
                    if ok {
                        if let Some(ref compiled) = compiled {
                            match compiled.evaluate(&request) {
                                Ok(cd) => {
                                    let cgot = format!("{cd:?}").to_lowercase();
                                    if cgot != got {
                                        ok = false;
                                        detail = format!("PARITY: compiled={cgot}, ast={got}");
                                    }
                                }
                                Err(e) => {
                                    ok = false;
                                    detail = format!("compiled eval error: {e:?}");
                                }
                            }
                        }
                    }
                    (ok, detail)
                }
                Err(e) => (false, format!("eval error: {e:?}")),
            }
        };
        results.push((case.name.clone(), ok, detail));
    }
    Ok(results)
}
