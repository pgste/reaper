//! Eval-path interner bounding: the compiled evaluator must not grow the shared
//! interner per request. Two leak classes are covered:
//!
//! 1. Per-request entity lookups (principal / resource) — resolved via `lookup`,
//!    never interned. High-cardinality non-entity resources therefore don't
//!    accumulate, and a loaded entity used as a principal is NOT pinned (so the
//!    data-plane's refcounted reclamation still evicts it when it's removed).
//! 2. Result-producing string methods (lower/upper/trim/split/replace/find/…) —
//!    interned *counted* per evaluation and released when the evaluation ends,
//!    so a policy producing high-cardinality results stays bounded.

use policy_engine::data::{DataLoader, DataStore};
use policy_engine::reap::ReaperPolicy;
use policy_engine::{PolicyAction, PolicyEvaluator, PolicyRequest};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

fn request(principal: &str, resource: &str) -> PolicyRequest {
    let mut context = HashMap::new();
    context.insert("principal".to_string(), principal.to_string());
    PolicyRequest {
        resource: resource.to_string(),
        action: "read".to_string(),
        context,
    }
}

fn load(store: &DataStore, data: serde_json::Value) {
    DataLoader::new(store.clone())
        .load_json(&data.to_string())
        .expect("load data");
}

// --- 1a. High-cardinality non-entity resources don't grow the interner -------

#[test]
fn high_cardinality_resources_do_not_grow_the_interner() {
    let store = Arc::new(DataStore::new());
    load(
        &store,
        serde_json::json!({"entities": [
            {"id": "alice", "type": "user", "attributes": {"role": "admin"}}
        ]}),
    );
    let compiled = ReaperPolicy::from_str(
        r#"policy p { default: deny, rule r { allow if resource == "known_resource" } }"#,
    )
    .unwrap()
    .build(store.clone())
    .unwrap();
    assert_eq!(compiled.evaluator_type(), "reaper_dsl");

    let baseline = store.interner().stats().unique_strings;
    for i in 0..10_000 {
        // A distinct, never-registered resource id on every request.
        let d = compiled
            .evaluate(&request("alice", &format!("resource_{i}")))
            .expect("eval");
        assert_eq!(d, PolicyAction::Deny); // none equal "known_resource"
    }
    let after = store.interner().stats().unique_strings;
    assert_eq!(
        after, baseline,
        "per-request resource ids leaked the interner: grew {baseline} -> {after}"
    );
}

// --- 1b. A principal that is a loaded entity is not pinned by eval ------------

#[test]
fn principal_entity_stays_evictable_after_eval() {
    let store = Arc::new(DataStore::new());
    load(
        &store,
        serde_json::json!({"entities": [
            {"id": "ephemeral_user", "type": "user", "attributes": {"role": "guest"}}
        ]}),
    );
    let compiled = ReaperPolicy::from_str(
        r#"policy p { default: deny, rule r { allow if user.role == "admin" } }"#,
    )
    .unwrap()
    .build(store.clone())
    .unwrap();

    // Use the entity as a principal — pre-fix this interned (pinned) its id.
    assert_eq!(
        compiled
            .evaluate(&request("ephemeral_user", "r"))
            .expect("eval"),
        PolicyAction::Deny
    );
    assert!(store.interner().lookup("ephemeral_user").is_some());

    // Removing the entity must evict its id (it was counted, not pinned).
    DataLoader::new((*store).clone()).delete_entity("ephemeral_user");
    assert!(
        store.interner().lookup("ephemeral_user").is_none(),
        "principal entity id survived removal — eval pinned it instead of using lookup"
    );
}

// --- 2. High-cardinality string-method RESULTS don't grow the interner --------

#[test]
fn high_cardinality_lower_results_do_not_grow_the_interner() {
    let store = Arc::new(DataStore::new());
    // 500 users, each with a distinct UPPERCASE token.
    let mut entities = Vec::new();
    for i in 0..500 {
        entities.push(serde_json::json!({
            "id": format!("user_{i}"),
            "type": "user",
            "attributes": {"token": format!("TOK_{i}")}
        }));
    }
    load(&store, serde_json::json!({ "entities": entities }));

    let compiled = ReaperPolicy::from_str(
        r#"policy p { default: deny, rule r { allow if { t := user.token.lower() && t == "never" } } }"#,
    )
    .unwrap()
    .build(store.clone())
    .unwrap();
    assert_eq!(compiled.evaluator_type(), "reaper_dsl");

    let baseline = store.interner().stats().unique_strings;
    for i in 0..500 {
        // Each eval lowercases a distinct token -> a distinct NEW string
        // ("tok_i") that must be reclaimed when the evaluation ends.
        let d = compiled
            .evaluate(&request(&format!("user_{i}"), "r"))
            .expect("eval");
        assert_eq!(d, PolicyAction::Deny); // "tok_i" != "never"
    }
    let after = store.interner().stats().unique_strings;
    assert_eq!(
        after, baseline,
        "lower() results leaked the interner: grew {baseline} -> {after} over 500 distinct results"
    );
}

#[test]
fn high_cardinality_replace_and_split_results_do_not_grow_the_interner() {
    let store = Arc::new(DataStore::new());
    let mut entities = Vec::new();
    for i in 0..300 {
        entities.push(serde_json::json!({
            "id": format!("user_{i}"),
            "type": "user",
            // csv is distinct per user, so split/replace produce distinct results.
            "attributes": {"csv": format!("a{i},b{i},c{i}"), "name": format!("temp{i}")}
        }));
    }
    load(&store, serde_json::json!({ "entities": entities }));

    // split produces a fresh list of parts; replace produces a fresh string.
    let compiled = ReaperPolicy::from_str(
        r#"policy p { default: deny, rule r { allow if {
            parts := user.csv.split(",") && pc := parts.count() && pc == 99 &&
            rp := user.name.replace("temp", "perm") && rp == "never"
        } } }"#,
    )
    .unwrap()
    .build(store.clone())
    .unwrap();
    assert_eq!(compiled.evaluator_type(), "reaper_dsl");

    let baseline = store.interner().stats().unique_strings;
    for i in 0..300 {
        let d = compiled
            .evaluate(&request(&format!("user_{i}"), "r"))
            .expect("eval");
        assert_eq!(d, PolicyAction::Deny);
    }
    let after = store.interner().stats().unique_strings;
    assert_eq!(
        after, baseline,
        "split/replace results leaked the interner: grew {baseline} -> {after}"
    );
}
