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
//!
//! `contract_is_publishable` (round-2 C4, finding R2-06) is the schema-QUALITY
//! gate: presence alone does not make a contract usable for client codegen.
//! It hard-fails on an undocumented error model (the RFC 9457
//! `ProblemDetails` schema and its members, including `instance` — R2-08) and
//! on untyped error bodies in the typed endpoint groups, and RATCHETS the
//! checks the rest of the surface does not yet pass (see the baseline consts
//! — lower them, never raise them).

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

// ---------------------------------------------------------------------------
// Schema-quality gate (round-2 C4 / R2-06): the contract must be PUBLISHABLE
// — typed bodies, documented errors, described operations — not merely
// structurally valid.
// ---------------------------------------------------------------------------

/// HTTP methods that carry operations in a path item.
const METHODS: &[&str] = &["get", "put", "post", "delete", "patch", "options", "head"];

/// Documented paths that are public probes / self-authenticating ingest and
/// are exempt from the "must document a 4xx" rule. Mirrors the auth
/// gateway's `is_public_path` allowlist (spec paths are unprefixed).
fn is_public_spec_path(path: &str) -> bool {
    path == "/health"
        || path.starts_with("/health/")
        || path == "/live"
        || path == "/ready"
        || path == "/metrics"
        || path.starts_with("/metrics/")
        || path == "/openapi.json"
        || path.starts_with("/auth/")
        || path.starts_with("/webhooks/bundle-update")
        || path == "/webhooks/stripe"
}

/// The endpoint groups typed in round-2 C4: every documented client-error
/// response here MUST reference the `ProblemDetails` schema (hard fail).
fn is_typed_group_path(path: &str) -> bool {
    path.contains("/datastore")
        || path.starts_with("/orgs/{org}/decisions")
        || path.starts_with("/orgs/{org}/replay")
        || path.starts_with("/orgs/{org}/audit/")
}

/// Does this response object reference the ProblemDetails schema?
fn references_problem_details(response: &serde_json::Value) -> bool {
    let Some(content) = response["content"].as_object() else {
        return false;
    };
    content.values().any(|media| {
        media["schema"]["$ref"]
            .as_str()
            .is_some_and(|r| r.ends_with("/ProblemDetails"))
    })
}

// RATCHETS: violation counts on the parts of the surface round-2 C4 did not
// touch. These may only go DOWN — fix an operation, lower the number. A new
// undescribed / error-undocumented / body-less operation pushes the count
// over its baseline and fails this gate. Operations in the TYPED groups
// (`is_typed_group_path`) are exempt from the ratchet and hard-fail instead.
//
// Baselines recorded 2026-07-13 (C4 landing):
// - every operation now carries a summary/description → 0.
// - 129 pre-existing operations document no 4xx (auth/users/orgs/policies/
//   bundles/agents/sources/deployments/scim/...).
// - 94 pre-existing 200/201 responses document no body schema.
const DESCRIPTION_BASELINE: usize = 0;
const MISSING_4XX_BASELINE: usize = 129;
const UNTYPED_SUCCESS_BASELINE: usize = 94;

fn ratchet(name: &str, baseline: usize, violations: &[String]) {
    assert!(
        violations.len() <= baseline,
        "{name}: {} violations exceed the ratchet baseline of {baseline} — new \
         operations must satisfy this check (fix them); if you fixed existing \
         ones, LOWER the baseline.\n  {}",
        violations.len(),
        violations.join("\n  ")
    );
}

#[test]
fn contract_is_publishable() {
    let spec = reaper_management::api::build_openapi();
    let json = spec.to_json().expect("serialize openapi to json");
    let doc: serde_json::Value = serde_json::from_str(&json).expect("valid json");

    // --- The error model itself: ProblemDetails is a published component
    // carrying the RFC 9457 members, including `instance` (R2-08). Hard fail.
    let problem = &doc["components"]["schemas"]["ProblemDetails"];
    assert!(
        problem.is_object(),
        "components.schemas.ProblemDetails missing — the error model is not published"
    );
    for member in ["type", "title", "status", "detail", "instance", "code"] {
        assert!(
            problem["properties"][member].is_object(),
            "ProblemDetails schema lacks the `{member}` member"
        );
    }

    let paths = doc["paths"].as_object().expect("spec has a paths object");

    let mut no_description = Vec::new();
    let mut no_4xx = Vec::new();
    let mut untyped_success = Vec::new();
    let mut untyped_errors = Vec::new();
    let mut typed_group_regressions = Vec::new();

    for (path, item) in paths {
        for (method, op) in item.as_object().expect("path item object") {
            if !METHODS.contains(&method.as_str()) {
                continue;
            }
            let opref = format!("{method} {path}");

            // Every operation carries a summary or description (codegen
            // surfaces these as docstrings).
            // Typed groups must stay fully clean (hard fail); the rest of
            // the surface is held by the ratchet.
            let typed_group = is_typed_group_path(path);

            let has_summary = op["summary"].as_str().is_some_and(|s| !s.is_empty());
            let has_description = op["description"].as_str().is_some_and(|s| !s.is_empty());
            if !has_summary && !has_description {
                if typed_group {
                    typed_group_regressions.push(format!("{opref}: no summary/description"));
                } else {
                    no_description.push(opref.clone());
                }
            }

            let responses = op["responses"].as_object().expect("responses object");

            // Authenticated endpoints must document at least one 4xx (they
            // all can at minimum 401/403).
            if !is_public_spec_path(path) {
                let any_4xx = responses.keys().any(|s| s.starts_with('4'));
                if !any_4xx {
                    if typed_group {
                        typed_group_regressions.push(format!("{opref}: no documented 4xx"));
                    } else {
                        no_4xx.push(opref.clone());
                    }
                }
            }

            for (status, resp) in responses {
                // Success responses (200/201) must document a body schema —
                // an empty-content 200 generates a client that returns
                // nothing. 204 is the legitimate no-body success.
                if (status == "200" || status == "201")
                    && resp["content"]
                        .as_object()
                        .is_none_or(|content| content.is_empty())
                {
                    if typed_group {
                        typed_group_regressions
                            .push(format!("{opref} -> {status}: no body schema"));
                    } else {
                        untyped_success.push(format!("{opref} -> {status}"));
                    }
                }

                // Typed groups (C4): documented client errors reference the
                // published error model. HARD FAIL — this is the contract
                // the round-2 work establishes.
                if is_typed_group_path(path)
                    && ["400", "403", "404", "409", "412", "422", "428"].contains(&status.as_str())
                    && !references_problem_details(resp)
                {
                    untyped_errors.push(format!("{opref} -> {status}"));
                }
            }
        }
    }

    assert!(
        untyped_errors.is_empty(),
        "typed endpoint groups must document client errors as ProblemDetails \
         (`body = ProblemDetails` in the utoipa annotation):\n  {}",
        untyped_errors.join("\n  ")
    );
    assert!(
        typed_group_regressions.is_empty(),
        "typed endpoint groups (datastore/decisions/replay/audit) must stay \
         fully documented — summary, a 4xx response, and typed success bodies:\n  {}",
        typed_group_regressions.join("\n  ")
    );

    ratchet(
        "operations without summary/description",
        DESCRIPTION_BASELINE,
        &no_description,
    );
    ratchet(
        "authenticated operations without a documented 4xx",
        MISSING_4XX_BASELINE,
        &no_4xx,
    );
    ratchet(
        "200/201 responses without a documented body",
        UNTYPED_SUCCESS_BASELINE,
        &untyped_success,
    );
}
