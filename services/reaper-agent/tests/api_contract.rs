//! Contract-parity gate for the agent (Plan 07, Phase A).
//!
//! The agent's served router (`src/main.rs`) is kept separate from the OpenAPI
//! assembly (`src/api.rs`) on purpose: the enforcement hot path must not be
//! refactored. That means two route lists, so this test guarantees they cannot
//! drift — every handler routed in `main.rs` must have a documented operation
//! (utoipa derives the `operationId` from the handler fn name), and every
//! documented operation must correspond to a routed handler.

use std::collections::BTreeSet;
use std::fs;

/// Routed identifiers that are intentionally NOT documented operations.
const NOT_OPERATIONS: &[&str] = &[
    "serve_openapi", // the contract endpoint itself
];

/// Extract the handler identifier passed to each axum routing constructor
/// (`get(h)`, `post(h)`, `put(h)`, `delete(h)`) in `main.rs`, returning the
/// final path segment (e.g. `axum::routing::delete(delete_entity_handler)` ->
/// `delete_entity_handler`).
fn routed_handlers(src: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for ctor in ["get(", "post(", "put(", "delete("] {
        let mut idx = 0;
        while let Some(rel) = src[idx..].find(ctor) {
            let start = idx + rel + ctor.len();
            idx = start;
            // Read the identifier (letters, digits, _, ::) right after `(`.
            let rest = &src[start..];
            let ident: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == ':')
                .collect();
            let ident = ident.trim();
            if ident.is_empty() {
                continue;
            }
            // Skip closures / non-handler calls (e.g. `get(|| async {..})`).
            let last = ident.rsplit("::").next().unwrap_or(ident);
            if last.is_empty() || last.chars().next().is_some_and(|c| c.is_numeric()) {
                continue;
            }
            if !NOT_OPERATIONS.contains(&last) {
                out.insert(last.to_string());
            }
        }
    }
    out
}

#[test]
fn every_routed_handler_is_documented_and_vice_versa() {
    let main_src = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/main.rs"))
        .expect("read main.rs");
    let routed = routed_handlers(&main_src);
    assert!(
        routed.len() > 20,
        "route scan found only {} handlers — the scanner or router changed shape",
        routed.len()
    );

    let spec = reaper_agent::api::build_openapi();
    let json = spec.to_json().expect("serialize openapi");
    let doc: serde_json::Value = serde_json::from_str(&json).expect("valid json");
    let paths = doc["paths"].as_object().expect("paths object");

    let mut documented = BTreeSet::new();
    for (_p, item) in paths {
        for (method, op) in item.as_object().expect("path item") {
            if !["get", "post", "put", "delete", "patch"].contains(&method.as_str()) {
                continue;
            }
            let op_id = op["operationId"]
                .as_str()
                .expect("every operation has an operationId");
            documented.insert(op_id.to_string());
        }
    }

    let routed_not_documented: Vec<_> = routed.difference(&documented).collect();
    let documented_not_routed: Vec<_> = documented.difference(&routed).collect();

    assert!(
        routed_not_documented.is_empty(),
        "handlers routed in main.rs but missing a #[utoipa::path] operation: {routed_not_documented:?}"
    );
    assert!(
        documented_not_routed.is_empty(),
        "operations documented in api.rs but not routed in main.rs (stale/incorrect annotation): {documented_not_routed:?}"
    );
}

#[test]
fn openapi_is_valid_3_1() {
    let spec = reaper_agent::api::build_openapi();
    let json = spec.to_json().expect("serialize openapi");
    let doc: serde_json::Value = serde_json::from_str(&json).expect("valid json");

    assert!(
        doc["openapi"]
            .as_str()
            .unwrap_or_default()
            .starts_with("3.1"),
        "expected OpenAPI 3.1.x"
    );

    // Unique operationIds + non-empty responses (spec-validator errors otherwise).
    let paths = doc["paths"].as_object().expect("paths object");
    let mut seen = BTreeSet::new();
    for (path, item) in paths {
        for (method, op) in item.as_object().expect("path item") {
            if !["get", "post", "put", "delete", "patch"].contains(&method.as_str()) {
                continue;
            }
            if let Some(id) = op["operationId"].as_str() {
                assert!(
                    seen.insert(id.to_string()),
                    "duplicate operationId {id:?} — add an explicit operation_id"
                );
            }
            assert!(
                op["responses"].as_object().is_some_and(|r| !r.is_empty()),
                "{method} {path} has no documented responses"
            );
        }
    }
}
