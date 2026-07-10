//! Contract-parity gate (Plan 07, Phase A, step 2).
//!
//! The control-plane router and its OpenAPI 3.1 contract are generated from a
//! single `OpenApiRouter` tree (`api::build_openapi_router`): every route
//! registered through `.routes(routes!(..))` is, by construction, present in
//! both the served router and the published spec. The only way to serve a
//! route that the spec does NOT describe is to add a raw `.route(..)` that
//! bypasses `routes!`.
//!
//! `no_undocumented_raw_routes` closes exactly that gap: raw `.route(..)` calls
//! under `src/api/` are forbidden except for a small, explicit allowlist (the
//! contract endpoint itself and the short-form probe aliases). A new handler
//! therefore cannot be served without a `#[utoipa::path]` annotation — the
//! build goes red until it is documented.
//!
//! `openapi_spec_is_populated` asserts the assembled document is a non-empty
//! OpenAPI 3.1 spec with the expected anchor paths, catching a wholesale
//! regression in generation.

use std::fs;
use std::path::{Path, PathBuf};

/// Paths permitted to use a raw `.route(..)` (not part of the documented
/// surface, or intentionally undocumented probe aliases). Keep this list
/// short and justified — everything else must go through `routes!`.
const ALLOWED_RAW_ROUTES: &[&str] = &[
    "/openapi.json", // the contract endpoint (cannot document itself)
    "/live",         // short-form liveness alias of /health/live
    "/ready",        // short-form readiness alias of /health/ready
];

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Extract the path literal following each raw `.route(` occurrence in `src`.
/// Matches `.route(` exactly (never `.routes(`) and reads the next `"..."`
/// string literal, which may sit on a following line.
fn raw_route_paths(src: &str) -> Vec<String> {
    let bytes = src.as_bytes();
    let mut found = Vec::new();
    let needle = ".route(";
    let mut idx = 0;
    while let Some(rel) = src[idx..].find(needle) {
        let at = idx + rel;
        idx = at + needle.len();
        // Read the next double-quoted string literal (the route path).
        if let Some(qrel) = src[idx..].find('"') {
            let start = idx + qrel + 1;
            if let Some(erel) = src[start..].find('"') {
                let end = start + erel;
                found.push(src[start..end].to_string());
            }
        }
        let _ = bytes; // silence unused in case of empty file
    }
    found
}

#[test]
fn no_undocumented_raw_routes() {
    let api_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/api");
    let mut files = Vec::new();
    collect_rs_files(&api_dir, &mut files);
    assert!(
        !files.is_empty(),
        "no api source files found under {api_dir:?}"
    );

    let mut offenders = Vec::new();
    for file in &files {
        let content = fs::read_to_string(file).expect("read api source");
        for path in raw_route_paths(&content) {
            if !ALLOWED_RAW_ROUTES.contains(&path.as_str()) {
                offenders.push(format!(
                    "{}: raw .route(\"{}\")",
                    file.strip_prefix(env!("CARGO_MANIFEST_DIR"))
                        .unwrap_or(file)
                        .display(),
                    path
                ));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "raw `.route(..)` bypasses the OpenAPI contract — route these through \
         `.routes(routes!(handler))` with a `#[utoipa::path]` annotation, or add \
         the path to ALLOWED_RAW_ROUTES with justification.\nOffenders:\n  {}",
        offenders.join("\n  ")
    );
}

#[test]
fn openapi_spec_is_populated() {
    let spec = reaper_management::api::build_openapi();
    let json = spec.to_json().expect("serialize openapi to json");
    let doc: serde_json::Value = serde_json::from_str(&json).expect("valid json");

    let version = doc["openapi"].as_str().unwrap_or_default();
    assert!(
        version.starts_with("3.1"),
        "expected OpenAPI 3.1.x, got {version:?}"
    );

    let paths = doc["paths"].as_object().expect("spec has a paths object");
    assert!(
        paths.len() >= 30,
        "expected the full control-plane surface to be documented, only {} paths present",
        paths.len()
    );

    for anchor in ["/health", "/orgs", "/orgs/{org}/policies"] {
        assert!(
            paths.contains_key(anchor),
            "documented surface is missing anchor path {anchor}"
        );
    }

    // Every documented operation must carry a description, and operationIds must
    // be unique (spec-validator treats a missing response description or a
    // duplicate operationId — utoipa derives these from handler fn names, so
    // two same-named handlers in different modules collide — as an error).
    let mut seen_ids: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for (path, item) in paths {
        for (method, op) in item.as_object().expect("path item object") {
            if !["get", "put", "post", "delete", "patch", "options", "head"]
                .contains(&method.as_str())
            {
                continue;
            }
            let responses = op["responses"].as_object();
            assert!(
                responses.is_some_and(|r| !r.is_empty()),
                "{method} {path} has no documented responses"
            );
            if let Some(op_id) = op["operationId"].as_str() {
                if let Some(prev) = seen_ids.insert(op_id.to_string(), format!("{method} {path}")) {
                    panic!(
                        "duplicate operationId {op_id:?} on `{method} {path}` and `{prev}` — \
                         add an explicit `operation_id = \"...\"` to one of the colliding handlers"
                    );
                }
            }
        }
    }
}
