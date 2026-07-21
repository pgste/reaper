//! Tier-2 partial-evaluation fitness measurement (R3 Plan 06 F.3 — design
//! §6 of docs/development/PARTIAL_EVALUATION.md).
//!
//! Walks policy corpora, compiles every `.reap` policy, and reports how many
//! rules the F.4 specialization overlay would actually shorten. This is the
//! decision instrument for building (or parking) tier 2: if only a sliver of
//! real rules shorten, the overlay is not worth its complexity.
//!
//! Usage:
//! ```text
//! cargo run --example specialization_fitness -- [DIR ...]
//! # default corpus: ./policy-library (walked recursively)
//! ```
//!
//! Policies that fail to COMPILE are listed, not silently skipped: at
//! serving time those fall back to the AST evaluator, which the overlay
//! never touches, so they are genuinely out of tier 2's reach — but the
//! count must be visible for the fitness verdict to be honest.

use policy_engine::reap::ReaperPolicy;
use policy_engine::reaper_dsl::SpecializationFitness;
use policy_engine::DataStore;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn find_reap_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        eprintln!("warning: cannot read directory {}", dir.display());
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            find_reap_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "reap") {
            out.push(path);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dirs: Vec<PathBuf> = if args.is_empty() {
        vec![PathBuf::from("policy-library")]
    } else {
        args.iter().map(PathBuf::from).collect()
    };

    let mut files = Vec::new();
    for dir in &dirs {
        find_reap_files(dir, &mut files);
    }
    files.sort();

    if files.is_empty() {
        eprintln!("no .reap files found under {dirs:?}");
        std::process::exit(1);
    }

    let mut total = SpecializationFitness::default();
    let mut compiled = 0usize;
    let mut uncompilable: Vec<(PathBuf, String)> = Vec::new();
    let mut interesting: Vec<(PathBuf, SpecializationFitness)> = Vec::new();

    for path in &files {
        let policy = match ReaperPolicy::from_file(path) {
            Ok(p) => p,
            Err(e) => {
                uncompilable.push((path.clone(), format!("parse: {e}")));
                continue;
            }
        };
        // Fresh empty store per policy: fitness is a structural measurement,
        // entity data is not consulted.
        match policy.build(Arc::new(DataStore::new())) {
            Ok(evaluator) => {
                let fitness = evaluator.specialization_fitness();
                if fitness.static_leaves > 0 || fitness.static_context_leaves > 0 {
                    interesting.push((path.clone(), fitness));
                }
                total.merge(&fitness);
                compiled += 1;
            }
            Err(e) => uncompilable.push((path.clone(), format!("compile: {e}"))),
        }
    }

    println!("tier-2 specialization fitness (design §6 decision input)");
    println!("=========================================================");
    println!("corpora:            {dirs:?}");
    println!(
        "policies:           {} found, {} compiled, {} AST-fallback (out of tier-2 reach)",
        files.len(),
        compiled,
        uncompilable.len()
    );
    println!("rules:              {}", total.total_rules);
    println!("leaves:             {}", total.total_leaves);
    println!(
        "static leaves:      {} (literal-literal ReBAC — specializable today)",
        total.static_leaves
    );
    println!(
        "static-context:     {} (context-anchored — needs a static-context config)",
        total.static_context_leaves
    );
    println!(
        "rules shortenable:  {} today / {} with a static-context config",
        total.rules_shortenable, total.rules_shortenable_with_static_context
    );
    if total.total_rules > 0 {
        println!(
            "shorten rate:       {:.2}% today / {:.2}% with static context (threshold ~5%)",
            100.0 * total.rules_shortenable as f64 / total.total_rules as f64,
            100.0 * total.rules_shortenable_with_static_context as f64 / total.total_rules as f64,
        );
    }

    if !interesting.is_empty() {
        println!("\npolicies with specializable leaves:");
        for (path, f) in &interesting {
            println!(
                "  {} — static {} / static-context {} / shortenable {}(+ctx {}) of {} rules",
                path.display(),
                f.static_leaves,
                f.static_context_leaves,
                f.rules_shortenable,
                f.rules_shortenable_with_static_context,
                f.total_rules
            );
        }
    }

    if !uncompilable.is_empty() {
        println!("\nAST-fallback policies (compiled overlay can never serve these):");
        for (path, reason) in &uncompilable {
            println!("  {} — {reason}", path.display());
        }
    }
}
