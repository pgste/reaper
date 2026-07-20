//! Data-epoch bump-completeness tests (R3-06 Phase F.2).
//!
//! `DataStore::data_epoch()` is the staleness signal for epoch-stamped
//! consumers (deploy-time policy specialization, `/health`). Its contract:
//! **every completed mutation of the store or its relationship graph is
//! followed by a bump**, so two equal readings bracket a window in which no
//! mutation completed. A mutation path that forgets to bump is a fail-open
//! hole — a specialization overlay built against old data would keep serving.
//!
//! This file is the exhaustiveness pin (the mirror of
//! `test_binds_variables_covers_every_assignment_variant` for tier-1
//! folding): it enumerates EVERY public mutation entry point and asserts the
//! epoch strictly increases across each. If you add a mutating method to
//! `DataStore`, `RelationshipGraph`, `DataLoader`, the bundle load paths, or
//! the streaming loader, add it to the checklist below AND a case here.
//!
//! Checklist of covered entry points:
//! - `DataStore`: `insert`, `insert_batch`, `upsert`, `remove`,
//!   `remove_entity`, `clear`, `add_relationship`
//! - `RelationshipGraph` (reached via `store.relationships()`): `add_edge`,
//!   `remove_edge`, `detach_carried`, `detach`, `clear`
//! - `DataLoader`: `load_json` (streaming), `load_json_batch`,
//!   `load_json_values`, `upsert_entity_doc`, `delete_entity`
//! - Bundle loads: `DataBundle::load_into_existing_store`,
//!   `DataBundle::replace_store`, `DataStore::replace_with_bundle`
//! - `StreamingLoader::stream_and_load`
//!
//! Reads are covered by the inverse assertion: they must NOT bump, otherwise
//! epoch equality is useless and every overlay lookup would miss.

use policy_engine::data::entity::EntityBuilder;
use policy_engine::data::{AttributeValue, DataLoader, DataStore, StreamingLoader};
use serde_json::json;
use std::io::Write;

/// Assert `mutation` strictly increases the store's epoch. Returns the new
/// epoch so calls can chain.
fn assert_bumps(store: &DataStore, label: &str, mutation: impl FnOnce()) -> u64 {
    let before = store.data_epoch();
    mutation();
    let after = store.data_epoch();
    assert!(
        after > before,
        "{label}: epoch must strictly increase across the mutation \
         (before={before}, after={after})"
    );
    after
}

/// Insert a `User` entity with the given id and one attribute.
fn insert_user(store: &DataStore, id: &str) {
    let interner = store.interner();
    let uid = interner.intern(id);
    let utype = interner.intern("User");
    let role_key = interner.intern("role");
    store.insert(
        EntityBuilder::new(uid, utype)
            .with_attribute(role_key, AttributeValue::from_string("admin", interner))
            .build(),
    );
}

#[test]
fn new_store_starts_at_epoch_zero() {
    assert_eq!(DataStore::new().data_epoch(), 0);
}

#[test]
fn every_store_mutation_entry_point_bumps_the_epoch() {
    let store = DataStore::new();
    let interner = store.interner();

    // insert
    assert_bumps(&store, "insert", || insert_user(&store, "alice"));

    // insert_batch — one bump for the whole batch is sufficient (strict
    // increase is what's asserted, not bump count).
    let utype = interner.intern("User");
    let batch: Vec<_> = ["bob", "carol"]
        .iter()
        .map(|id| EntityBuilder::new(interner.intern(id), utype).build())
        .collect();
    assert_bumps(&store, "insert_batch", || store.insert_batch(batch));

    // upsert (replaces alice)
    let alice = interner.intern("alice");
    assert_bumps(&store, "upsert", || {
        store.upsert(EntityBuilder::new(alice, utype).build())
    });

    // remove (present entity)
    let bob = interner.intern("bob");
    assert_bumps(&store, "remove", || {
        assert!(store.remove(bob).is_some(), "bob must be present");
    });

    // remove_entity (cascade variant)
    let carol = interner.intern("carol");
    assert_bumps(&store, "remove_entity", || {
        assert!(
            store.remove_entity(carol).is_some(),
            "carol must be present"
        );
    });

    // add_relationship (store-level wrapper)
    insert_user(&store, "dave");
    insert_user(&store, "erin");
    let dave = interner.intern("dave");
    let erin = interner.intern("erin");
    let owns = interner.intern("owns");
    assert_bumps(&store, "add_relationship", || {
        store.add_relationship(dave, owns, erin)
    });

    // clear
    assert_bumps(&store, "clear", || store.clear());
}

#[test]
fn every_relationship_graph_mutation_bumps_the_store_epoch() {
    // The graph shares the store's counter: mutating THROUGH the graph
    // handle (bypassing DataStore methods) must still invalidate consumers.
    let store = DataStore::new();
    let interner = store.interner();
    insert_user(&store, "alice");
    insert_user(&store, "bob");
    let alice = interner.intern("alice");
    let bob = interner.intern("bob");
    let owns = interner.intern("owns");

    assert_bumps(&store, "graph add_edge", || {
        store.relationships().add_edge(alice, owns, bob)
    });
    assert_bumps(&store, "graph remove_edge", || {
        store.relationships().remove_edge(alice, owns, bob)
    });

    store.relationships().add_edge(alice, owns, bob);
    assert_bumps(&store, "graph detach_carried", || {
        store.relationships().detach_carried(alice)
    });

    store.relationships().add_edge(alice, owns, bob);
    assert_bumps(&store, "graph detach", || store.relationships().detach(bob));

    assert_bumps(&store, "graph clear", || store.relationships().clear());
}

#[test]
fn every_loader_entry_point_bumps_the_epoch() {
    let store = DataStore::new();
    let loader = DataLoader::new(store.clone());

    let doc = r#"{"entities": [{"id": "u1", "type": "User",
                   "attributes": {"role": "admin"}}]}"#;

    assert_bumps(&store, "load_json (streaming)", || {
        loader.load_json(doc).expect("load_json");
    });
    assert_bumps(&store, "load_json_batch", || {
        loader.load_json_batch(doc).expect("load_json_batch");
    });
    assert_bumps(&store, "load_json_values", || {
        loader
            .load_json_values(vec![
                json!({"id": "u2", "type": "User", "attributes": {"role": "viewer"}}),
            ])
            .expect("load_json_values");
    });
    assert_bumps(&store, "upsert_entity_doc", || {
        loader
            .upsert_entity_doc(&json!({"id": "u2", "type": "User",
                                       "attributes": {"role": "admin"}}))
            .expect("upsert_entity_doc");
    });
    assert_bumps(&store, "delete_entity", || loader.delete_entity("u2"));
}

#[test]
fn bundle_load_paths_bump_the_epoch() {
    // Build a source store and serialize it to a bundle.
    let source = DataStore::new();
    insert_user(&source, "alice");
    let bundle = source.to_bundle("test".into(), "1".into());

    let store = DataStore::new();
    assert_bumps(&store, "DataBundle::load_into_existing_store", || {
        bundle
            .load_into_existing_store(&store)
            .expect("load bundle");
    });
    assert_bumps(&store, "DataBundle::replace_store", || {
        bundle.replace_store(&store).expect("replace from bundle");
    });
    assert_bumps(&store, "DataStore::replace_with_bundle", || {
        store
            .replace_with_bundle(&bundle)
            .expect("replace_with_bundle");
    });
}

#[test]
fn streaming_loader_bumps_the_epoch() {
    let mut file = tempfile::NamedTempFile::new().expect("temp file");
    file.write_all(
        br#"{"id": "s1", "type": "User", "attributes": {"role": "admin"}}
{"id": "s2", "type": "User", "attributes": {"role": "viewer"}}
"#,
    )
    .expect("write temp data");

    let store = DataStore::new();
    let loader = StreamingLoader::new(DataLoader::new(store.clone()), 1);
    assert_bumps(&store, "StreamingLoader::stream_and_load", || {
        let stats = loader.stream_and_load(file.path()).expect("stream load");
        assert_eq!(stats.total, 2);
    });
}

#[test]
fn reads_do_not_bump_the_epoch() {
    // The inverse invariant: if reads bumped, epoch equality could never
    // hold and every epoch-stamped consumer would permanently fall back.
    let store = DataStore::new();
    let interner = store.interner();
    insert_user(&store, "alice");
    insert_user(&store, "bob");
    let alice = interner.intern("alice");
    let owns = interner.intern("owns");
    let bob = interner.intern("bob");
    store.add_relationship(alice, owns, bob);

    let before = store.data_epoch();
    let utype = interner.intern("User");
    let role_key = interner.intern("role");
    let admin = interner.intern("admin");

    store.get(alice);
    store.entity_attributes_json("alice");
    store.resource_type_attr("alice");
    store.get_by_type(utype);
    store.get_by_attribute(role_key, admin);
    store.get_by_type_and_attribute(utype, role_key, admin);
    store.all();
    store.get_entity_type_stats();
    store.stats();
    store.relationships().related(alice, owns);
    store.relationships().related_to(bob, owns);
    store.relationships().has_relation(alice, owns, bob);
    store.relationships().len();

    assert_eq!(
        store.data_epoch(),
        before,
        "read-only operations must not advance the epoch"
    );
}

#[test]
fn store_clones_share_one_epoch() {
    // DataStore is Clone-with-shared-Arcs; the epoch must follow the same
    // sharing so the agent's many handles agree on staleness.
    let store = DataStore::new();
    let clone = store.clone();
    insert_user(&clone, "alice");
    assert_eq!(store.data_epoch(), clone.data_epoch());
    assert!(store.data_epoch() > 0);
}

#[test]
fn removing_a_missing_entity_does_not_bump() {
    // remove() early-returns before touching anything when the id is absent:
    // nothing changed, so not bumping is sound (and avoids invalidating
    // overlays on no-op deletes from at-least-once delta streams).
    let store = DataStore::new();
    let missing = store.interner().intern("ghost");
    let before = store.data_epoch();
    assert!(store.remove(missing).is_none());
    assert_eq!(store.data_epoch(), before);
}
