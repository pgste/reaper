//! ReBAC-subject interner bounding: relationship *subjects* are the last
//! high-cardinality string class that used to be pinned forever (interned via
//! `add_relationship` with a plain `intern`). Two failure modes are covered:
//!
//! 1. **Subject churn leaks** — a stable carrier (`doc`) whose `viewer` subject
//!    is a fresh, unique id on every delta upsert (sessions, request ids, …)
//!    used to grow the interner by one string per upsert forever. It must now
//!    stay flat: the old subject is released when the carrier's edge block is
//!    replaced.
//! 2. **A subject that is also a loaded entity was pinned** — using an entity's
//!    id as a relationship subject pinned that id, defeating the data-plane's
//!    refcounted reclamation so the entity's strings survived its deletion.
//!    The id must now be evicted once both the entity and every edge naming it
//!    are gone.
//!
//! The balance invariant enforced by the graph — *exactly one counted subject
//! reference per live forward edge* — is also checked directly (duplicate
//! subjects in a source list, and re-adding an identical edge, must not leak).

use policy_engine::data::{DataLoader, DataStore};
use serde_json::json;
use std::sync::Arc;

fn unique_strings(store: &DataStore) -> usize {
    store.interner().stats().unique_strings
}

// --- 1. High-cardinality subject churn stays bounded -------------------------

#[test]
fn churning_relationship_subjects_do_not_grow_the_interner() {
    let store = Arc::new(DataStore::new());
    let loader = DataLoader::new((*store).clone());

    // Materialize a stable carrier whose sole viewer is an ever-unique id.
    let doc = |subject: &str| {
        json!({
            "id": "doc",
            "type": "resource",
            "attributes": {},
            "relationships": { "viewer": [subject] }
        })
    };

    // Prime with the first subject, then measure — the stable strings ("doc",
    // "resource", "viewer", "session_0") are all present now, so any growth
    // across the churn loop is a leak.
    loader.upsert_entity_doc(&doc("session_0")).unwrap();
    let baseline = unique_strings(&store);

    for i in 1..5_000 {
        // Each upsert replaces the relationship block: the previous session id
        // is no longer referenced by any edge and must be evicted.
        loader
            .upsert_entity_doc(&doc(&format!("session_{i}")))
            .unwrap();
    }

    let after = unique_strings(&store);
    assert_eq!(
        after, baseline,
        "churning relationship subjects leaked the interner: {baseline} -> {after}"
    );
    // Only the latest subject is live; stale ones are gone.
    assert!(store.interner().lookup("session_4999").is_some());
    assert!(
        store.interner().lookup("session_0").is_none(),
        "the first churned subject was never reclaimed"
    );
}

// --- 2. A subject that is also a loaded entity stays evictable ---------------

#[test]
fn subject_entity_id_is_evicted_once_entity_and_edges_are_gone() {
    let store = Arc::new(DataStore::new());
    let loader = DataLoader::new((*store).clone());

    // `alice` is a first-class entity AND the owner-subject of `doc`.
    loader
        .load_json(
            &json!({ "entities": [
                { "id": "alice", "type": "user", "attributes": { "role": "admin" } },
                { "id": "doc", "type": "resource", "attributes": {},
                  "relationships": { "owner": ["alice"] } }
            ] })
            .to_string(),
        )
        .unwrap();

    assert!(store.interner().lookup("alice").is_some());

    // Deleting alice cascades (fail closed): `remove_entity` detaches every
    // edge pointing at her — including doc#owner@alice — so both her entity
    // count and that edge's subject count drop to zero and her id is evicted.
    // Pre-fix the subject intern PINNED her id, so it would survive forever
    // here even though nothing references it.
    loader.delete_entity("alice");
    assert!(
        store.interner().lookup("alice").is_none(),
        "alice's id survived deletion — a pinned subject leak (used as doc#owner)"
    );
    // And the cascade genuinely removed the dangling edge (fail closed).
    assert!(
        store
            .relationships()
            .related(
                store.interner().lookup("doc").unwrap(),
                store.interner().lookup("owner").unwrap(),
            )
            .is_empty(),
        "deleting the subject entity must drop the edge naming it"
    );
}

#[test]
fn deleting_the_carrier_first_also_reclaims_the_subject() {
    // Same as above but delete order reversed: carrier before subject-entity.
    let store = Arc::new(DataStore::new());
    let loader = DataLoader::new((*store).clone());

    loader
        .load_json(
            &json!({ "entities": [
                { "id": "alice", "type": "user", "attributes": {} },
                { "id": "doc", "type": "resource", "attributes": {},
                  "relationships": { "owner": ["alice"] } }
            ] })
            .to_string(),
        )
        .unwrap();

    loader.delete_entity("doc"); // releases the edge's count on alice
    assert!(
        store.interner().lookup("alice").is_some(),
        "alice is still a live entity"
    );
    loader.delete_entity("alice"); // releases the entity's own count
    assert!(
        store.interner().lookup("alice").is_none(),
        "alice's id survived once both references were dropped"
    );
}

// --- 3. Balance invariant: one counted reference per live edge ---------------

#[test]
fn duplicate_subjects_in_a_source_list_do_not_leak() {
    // The loader counts one reference per subject *occurrence*; a duplicate
    // collapses onto the same stored edge, so add_edge releases the redundant
    // count. After deleting the carrier the subject id must be fully reclaimed.
    let store = Arc::new(DataStore::new());
    let loader = DataLoader::new((*store).clone());

    loader
        .upsert_entity_doc(&json!({
            "id": "doc",
            "type": "resource",
            "attributes": {},
            // alice listed three times — one live edge, not three counts.
            "relationships": { "viewer": ["alice", "alice", "alice"] }
        }))
        .unwrap();

    assert!(store.interner().lookup("alice").is_some());
    assert!(
        store.relationships().has_relation(
            store.interner().lookup("doc").unwrap(),
            store.interner().lookup("viewer").unwrap(),
            store.interner().lookup("alice").unwrap(),
        ),
        "the deduped edge must still exist"
    );

    loader.delete_entity("doc");
    assert!(
        store.interner().lookup("alice").is_none(),
        "duplicate subject occurrences leaked extra counts (id not reclaimed)"
    );
}

#[test]
fn re_upserting_an_identical_edge_block_does_not_leak() {
    // Applying the same upsert twice (at-least-once delta delivery) must
    // converge — the second upsert re-counts then re-adds the identical edge,
    // and detach/dedup must net to a single live count.
    let store = Arc::new(DataStore::new());
    let loader = DataLoader::new((*store).clone());

    let doc = json!({
        "id": "doc",
        "type": "resource",
        "attributes": {},
        "relationships": { "viewer": ["alice"] }
    });

    loader.upsert_entity_doc(&doc).unwrap();
    let baseline = unique_strings(&store);
    for _ in 0..1_000 {
        loader.upsert_entity_doc(&doc).unwrap();
    }
    assert_eq!(
        unique_strings(&store),
        baseline,
        "idempotent re-upsert of an identical edge grew the interner"
    );

    loader.delete_entity("doc");
    assert!(
        store.interner().lookup("alice").is_none(),
        "repeated identical upserts left a residual count on the subject"
    );
}
