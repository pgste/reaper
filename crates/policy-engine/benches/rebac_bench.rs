//! ReBAC micro-benchmarks: the compiled relationship checks on a realistic
//! graph — 10k users, 1k groups (nested 3 deep), 5k documents in a folder
//! tree. Measures the primitives (graph ops) and the full compiled policy
//! evaluation path (parse request → rules → rebac check).
//!
//! Run: cargo bench -p policy-engine --bench rebac_bench

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use policy_engine::reap::ReaperPolicy;
use policy_engine::{DataStore, PolicyEvaluator, PolicyRequest};
use std::collections::HashMap;
use std::sync::Arc;

const USERS: usize = 10_000;
const GROUPS: usize = 1_000;
const DOCS: usize = 5_000;

/// Build the graph directly against the store (no JSON parse in the setup
/// timing): users → groups (chained 3 deep), docs → folders, docs shared with
/// groups, folders owned by users.
fn build_store() -> Arc<DataStore> {
    let store = Arc::new(DataStore::new());
    let interner = store.interner();
    let member_of = interner.intern("member_of");
    let viewer = interner.intern("viewer");
    let owner = interner.intern("owner");
    let parent = interner.intern("parent");

    // Entities must exist for the compiled evaluator's user/resource lookup.
    let mut entities = Vec::with_capacity(USERS + GROUPS + DOCS + 100);
    let user_type = interner.intern("User");
    let group_type = interner.intern("Group");
    let doc_type = interner.intern("Document");
    let folder_type = interner.intern("Folder");

    for u in 0..USERS {
        let id = interner.intern(&format!("user-{u}"));
        entities.push(policy_engine::Entity::new(
            id,
            user_type,
            Default::default(),
        ));
        // each user in one leaf group
        let g = interner.intern(&format!("group-{}", u % GROUPS));
        store.add_relationship(id, member_of, g);
    }
    for g in 0..GROUPS {
        let id = interner.intern(&format!("group-{g}"));
        entities.push(policy_engine::Entity::new(
            id,
            group_type,
            Default::default(),
        ));
        // 3-deep nesting: leaf groups -> mid groups -> org
        if g >= 10 {
            let mid = interner.intern(&format!("group-{}", g % 10));
            store.add_relationship(id, member_of, mid);
        } else if g != 0 {
            let org = interner.intern("group-0");
            store.add_relationship(id, member_of, org);
        }
    }
    for f in 0..100 {
        let id = interner.intern(&format!("folder-{f}"));
        entities.push(policy_engine::Entity::new(
            id,
            folder_type,
            Default::default(),
        ));
        let owner_id = interner.intern(&format!("user-{}", f * 97 % USERS));
        store.add_relationship(id, owner, owner_id);
    }
    for d in 0..DOCS {
        let id = interner.intern(&format!("doc-{d}"));
        entities.push(policy_engine::Entity::new(id, doc_type, Default::default()));
        let folder = interner.intern(&format!("folder-{}", d % 100));
        store.add_relationship(id, parent, folder);
        // shared with one leaf group
        let g = interner.intern(&format!("group-{}", d % GROUPS));
        store.add_relationship(id, viewer, g);
    }
    store.insert_batch(entities);
    store
}

const POLICY: &str = r#"
policy rebac_bench {
    default: deny,
    rule owner { allow if rebac::related(user, "owner", resource) }
    rule group_viewer { allow if rebac::reachable(user, "viewer", resource, "member_of", 3) }
    rule folder_inherit { allow if rebac::inherited(user, "owner", resource, "parent", 3) }
}
"#;

fn request(principal: &str, resource: &str) -> PolicyRequest {
    let mut context = HashMap::new();
    context.insert("principal".to_string(), principal.to_string());
    PolicyRequest {
        resource: resource.to_string(),
        action: "read".to_string(),
        context,
    }
}

fn bench_graph_primitives(c: &mut Criterion) {
    let store = build_store();
    let interner = store.interner();
    let graph = store.relationships();
    let viewer = interner.intern("viewer");
    let member_of = interner.intern("member_of");
    let owner = interner.intern("owner");
    let parent = interner.intern("parent");

    // doc-42 is shared with group-42; user-42 is in group-42 (1 hop).
    let doc = interner.intern("doc-42");
    let user_direct = interner.intern("group-42"); // direct holder
    let user_1hop = interner.intern("user-42");
    let user_miss = interner.intern("user-43"); // in group-43, not a viewer

    c.bench_function("rebac/has_relation_direct_hit", |b| {
        b.iter(|| black_box(graph.has_relation(doc, viewer, user_direct)))
    });
    c.bench_function("rebac/has_relation_direct_miss", |b| {
        b.iter(|| black_box(graph.has_relation(doc, viewer, user_miss)))
    });
    c.bench_function("rebac/reachable_1hop_hit", |b| {
        b.iter(|| black_box(graph.has_relation_reachable(doc, viewer, user_1hop, member_of, 3)))
    });
    c.bench_function("rebac/reachable_3hop_miss_bounded", |b| {
        // full bounded expansion without a hit: worst case for one check
        b.iter(|| black_box(graph.has_relation_reachable(doc, viewer, user_miss, member_of, 3)))
    });

    // inherited: doc-0 -> folder-0, owner user-0
    let doc0 = interner.intern("doc-0");
    let owner_user = interner.intern("user-0");
    c.bench_function("rebac/inherited_1hop_hit", |b| {
        b.iter(|| black_box(graph.has_relation_inherited(doc0, owner, owner_user, parent, 3)))
    });
}

fn bench_compiled_policy(c: &mut Criterion) {
    let store = build_store();
    let parsed: ReaperPolicy = POLICY.parse().expect("parse");
    let compiled = parsed
        .clone()
        .build(store.clone())
        .expect("rebac policy must compile");
    let ast = parsed.build_ast_evaluator(store);

    // user-42 reads doc-42: allowed via group viewer (1-hop reachable).
    let req_hit = request("user-42", "doc-42");
    // user-43 reads doc-42: full rule sweep, all miss -> default deny.
    let req_miss = request("user-43", "doc-42");

    c.bench_function("rebac/compiled_policy_allow_group_viewer", |b| {
        b.iter(|| black_box(compiled.evaluate(&req_hit).unwrap()))
    });
    c.bench_function("rebac/compiled_policy_deny_full_sweep", |b| {
        b.iter(|| black_box(compiled.evaluate(&req_miss).unwrap()))
    });
    c.bench_function("rebac/ast_policy_allow_group_viewer", |b| {
        b.iter(|| black_box(ast.evaluate(&req_hit).unwrap()))
    });
}

criterion_group!(benches, bench_graph_primitives, bench_compiled_policy);
criterion_main!(benches);
