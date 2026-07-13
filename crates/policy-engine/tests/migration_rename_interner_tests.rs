//! Plan 12 step 6 — interner hygiene across a mass rename.
//!
//! A model migration that renames a role/relation/attribute reaches agents
//! as ordinary upsert deltas (each entity's document re-emitted with the new
//! vocabulary). The refcounted interner must RELEASE the old strings as the
//! last carrier is upserted away from them — a rename of N entities must
//! not leave N orphaned strings resident, or repeated migrations grow agent
//! memory without bound.

use policy_engine::{DataLoader, DataStore};
use serde_json::json;

const N: usize = 500;

fn doc(i: usize, role: &str, relation: &str, group: &str) -> serde_json::Value {
    json!({
        "id": format!("user-{i}"),
        "type": "user",
        "attributes": {
            "roles": [role],
            "permissions": ["resource:read"],
        },
        "relationships": { relation: [group] },
    })
}

#[test]
fn mass_rename_releases_old_interned_strings() {
    let store = DataStore::new();
    let loader = DataLoader::new(store.clone());

    // Load N users carrying the OLD vocabulary (distinctive strings so no
    // other machinery pins them).
    for i in 0..N {
        loader
            .upsert_entity_doc(&doc(i, "mig_old_role", "mig_old_rel", "mig-group"))
            .expect("initial upsert");
    }
    assert!(
        store.interner().lookup("mig_old_role").is_some(),
        "old role value interned while carried"
    );
    assert!(
        store.interner().lookup("mig_old_rel").is_some(),
        "old relation name interned while carried"
    );
    let resident_before = store.interner().stats().unique_strings;

    // The migration's delta wave: every carrier re-upserted with the NEW
    // vocabulary (exactly what the changes API emits after an apply).
    for i in 0..N {
        loader
            .upsert_entity_doc(&doc(i, "mig_new_role", "mig_new_rel", "mig-group"))
            .expect("rename upsert");
    }

    // ENTITY-OWNED strings (attribute values, carried across N entities)
    // are refcounted: the old role value must be fully released when the
    // last carrier is upserted away from it — this is the O(N) side of a
    // rename, and it must not accumulate.
    assert!(
        store.interner().lookup("mig_old_role").is_none(),
        "renamed-away role value must be released from the interner"
    );
    // SCHEMA vocabulary (relation names, attribute keys, entity types) is
    // deliberately pinned uncounted — it is bounded by model size, not by
    // record count. A rename therefore orphans exactly ONE schema string,
    // O(#migrations), never O(N). Assert the design rather than pretending
    // it releases.
    assert!(
        store.interner().lookup("mig_old_rel").is_some(),
        "schema vocabulary is pinned by design (bounded, uncounted)"
    );
    assert!(
        store.interner().lookup("mig_new_role").is_some(),
        "new vocabulary is resident"
    );

    // Net growth after renaming N=500 carriers is bounded by the SCHEMA
    // delta (one new relation name + swapped role value), never by N.
    let resident_after = store.interner().stats().unique_strings;
    let growth = resident_after.saturating_sub(resident_before);
    assert!(
        growth <= 2,
        "rename-of-{N} must grow the interner by O(schema), not O(N) \
         (before={resident_before}, after={resident_after}, growth={growth})"
    );

    // And the data is intact: every user carries the new edge.
    let interner = store.interner();
    let rel = interner
        .lookup("mig_new_rel")
        .expect("new relation interned");
    let user0 = interner.lookup("user-0").expect("user-0 resident");
    let group = interner.lookup("mig-group").expect("group resident");
    assert!(store.relationships().related(user0, rel).contains(&group));
}
