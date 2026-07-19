//! Plan 08 Phase A — resource pruning index correctness & scale.
//!
//! These tests pin the two Definition-of-Done guarantees for the served
//! evaluate-all path:
//!
//! 1. **Scale.** At N=10k policies with only a handful matching a resource, the
//!    candidate set (and therefore the number of evaluator invocations) is ≈ the
//!    number of matching policies, not N. Asserted via `candidate_policy_ids`
//!    and `get_index_stats`.
//! 2. **Correctness (differential).** For a corpus of requests, combining the
//!    pruned candidate set yields the *identical* allow/deny decision as
//!    combining the full linear scan — the pruning index never changes a
//!    decision. This is the merge-gate guard against a fail-open bug.
//!
//! Plus hot-swap safety: the index stays consistent across deploy / redeploy /
//! remove / full replace.

use policy_engine::data::entity::EntityBuilder;
use policy_engine::{
    DataStore, EnhancedPolicy, PolicyAction, PolicyEngine, PolicyLanguage, PolicyRequest,
    PolicyRule,
};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

fn simple_policy(name: &str, rules: Vec<(PolicyAction, &str)>) -> EnhancedPolicy {
    EnhancedPolicy::new(
        name.to_string(),
        String::new(),
        rules
            .into_iter()
            .map(|(action, resource)| PolicyRule {
                action,
                resource: resource.to_string(),
                conditions: vec![],
            })
            .collect(),
    )
}

fn request(resource: &str) -> PolicyRequest {
    PolicyRequest {
        resource: resource.to_string(),
        action: "read".to_string(),
        context: HashMap::new(),

        ..Default::default()
    }
}

/// DoD: "Benchmark at N=10k with 3 matching policies shows evaluator
/// invocations ≈ 3, not 10,000." We assert the candidate set the served path
/// evaluates is ≈ 3, which bounds evaluator invocations directly.
#[test]
fn candidate_set_is_bounded_by_matches_not_total() {
    let engine = PolicyEngine::new();

    const N: usize = 10_000;
    // 10k policies over distinct resources /res/{i} — none wildcard, so each is
    // prunable and lands only in its own resource bucket.
    for i in 0..N {
        engine
            .deploy_policy(simple_policy(
                &format!("p{i}"),
                vec![(PolicyAction::Allow, &format!("/res/{i}"))],
            ))
            .unwrap();
    }

    // Three policies that all reference the SAME target resource.
    let target = "/hot/resource";
    for i in 0..3 {
        engine
            .deploy_policy(simple_policy(
                &format!("match{i}"),
                vec![(PolicyAction::Allow, target)],
            ))
            .unwrap();
    }

    let stats = engine.get_index_stats();
    assert_eq!(stats.total_policies, N + 3);
    assert_eq!(
        stats.unprunable_policies, 0,
        "no wildcard/DSL policies deployed, nothing should be unprunable"
    );

    // The served path evaluates exactly the candidates for the request resource.
    let candidates = engine.candidate_policy_ids(target, None);
    assert_eq!(
        candidates.len(),
        3,
        "candidate set must be the 3 matching policies, not all {}",
        N + 3
    );

    // A resource nobody references yields zero candidates (default deny, no evals).
    assert_eq!(engine.candidate_policy_ids("/nobody/here", None).len(), 0);
}

/// Build a compiled Reaper-DSL policy with a single rule body.
fn dsl_policy(name: &str, rule_body: &str) -> EnhancedPolicy {
    let content = format!(
        "policy {name} {{\n    default: deny,\n    rule r {{\n        {rule_body}\n    }}\n}}"
    );
    EnhancedPolicy::new_with_language(
        name.to_string(),
        String::new(),
        PolicyLanguage::ReaperDsl,
        content,
    )
    .expect("DSL policy should compile")
}

/// D2: compiled DSL policies whose rule constrains the request resource to a
/// literal are now PRUNABLE — bucketed by resource, not forced into the
/// always-candidate `unprunable` set. Attribute-predicate DSL policies remain
/// unprunable. This is the core of R2-P2-1's closure for the mandated language.
#[test]
fn dsl_policies_are_prunable_by_resource_literal() {
    let engine = PolicyEngine::new();

    const N: usize = 50;
    // N DSL policies over distinct resources — each prunable, each in its own
    // bucket.
    for i in 0..N {
        engine
            .deploy_policy(dsl_policy(
                &format!("d{i}"),
                &format!("allow if resource == \"/res/{i}\""),
            ))
            .unwrap();
    }

    // Three DSL policies all bound to the SAME target resource.
    let target = "/hot/resource";
    for i in 0..3 {
        engine
            .deploy_policy(dsl_policy(
                &format!("hot{i}"),
                &format!("allow if resource == \"{target}\""),
            ))
            .unwrap();
    }

    // A resource-TYPE-bounded policy (R3-P2-1: now prunable on the type tier,
    // no longer unprunable) and a user-predicate policy (still unprunable).
    engine
        .deploy_policy(dsl_policy("attr", "allow if resource.type == \"invoice\""))
        .unwrap();
    engine
        .deploy_policy(dsl_policy("blocked", "deny if user.role == \"blocked\""))
        .unwrap();

    let stats = engine.get_index_stats();
    assert_eq!(stats.total_policies, N + 3 + 2);
    assert_eq!(
        stats.unprunable_policies, 1,
        "only the user-predicate DSL policy is unprunable; the type-bounded \
         policy sits in the type tier (R3-P2-1)"
    );
    assert_eq!(
        stats.resource_buckets,
        N + 1,
        "N distinct resources + the shared target bucket"
    );
    assert_eq!(stats.type_buckets, 1, "one `invoice` type bucket");

    // The served path evaluates only the 3 literal matches + the 1 unprunable —
    // not all N+5. (No resolved resource type, so the type bucket is skipped —
    // sound: without an `invoice`-typed entity, `resource.type == "invoice"`
    // is false.)
    let candidates = engine.candidate_policy_ids(target, None);
    assert_eq!(
        candidates.len(),
        3 + 1,
        "candidate set must be 3 literal matches + 1 unprunable, not all {}",
        N + 5
    );

    // A resource nobody references literally: only the 1 unprunable candidate.
    assert_eq!(engine.candidate_policy_ids("/nobody/here", None).len(), 1);

    // With a resolved `invoice` resource type, the type-bounded policy joins.
    assert_eq!(
        engine
            .candidate_policy_ids("/nobody/here", Some("invoice"))
            .len(),
        2,
        "type bucket + unprunable"
    );

    // candidate_policy_ids stays sorted + deduped.
    let mut sorted = candidates.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(candidates, sorted, "candidates must be sorted and deduped");
}

/// A DSL policy whose rule binds the resource to a literal via an `Or` of
/// literals lands in BOTH buckets and no unprunable slot.
#[test]
fn dsl_or_of_literals_buckets_both() {
    let engine = PolicyEngine::new();
    let p = dsl_policy("multi", "allow if resource == \"/x\" || resource == \"/y\"");
    let id = p.id;
    engine.deploy_policy(p).unwrap();

    assert_eq!(engine.get_index_stats().unprunable_policies, 0);
    assert_eq!(engine.candidate_policy_ids("/x", None), vec![id]);
    assert_eq!(engine.candidate_policy_ids("/y", None), vec![id]);
    assert!(engine.candidate_policy_ids("/z", None).is_empty());
}

/// Wildcard (and, by extension, DSL/Cedar) policies are unprunable — always
/// candidates regardless of resource — so a `*` deny is never pruned away.
#[test]
fn wildcard_policies_are_always_candidates() {
    let engine = PolicyEngine::new();

    engine
        .deploy_policy(simple_policy("specific", vec![(PolicyAction::Allow, "/a")]))
        .unwrap();
    engine
        .deploy_policy(simple_policy(
            "global-deny",
            vec![(PolicyAction::Deny, "*")],
        ))
        .unwrap();

    let stats = engine.get_index_stats();
    assert_eq!(stats.unprunable_policies, 1, "the `*` policy is unprunable");

    // For ANY resource the wildcard policy is a candidate.
    let for_a = engine.candidate_policy_ids("/a", None);
    assert_eq!(for_a.len(), 2, "specific + wildcard both candidates for /a");
    let for_b = engine.candidate_policy_ids("/b", None);
    assert_eq!(for_b.len(), 1, "only the wildcard is a candidate for /b");

    // And the wildcard deny actually decides via the pruned set.
    let out = engine.evaluate_set(&engine.candidate_policy_ids("/b", None), &request("/b"));
    assert_eq!(out.decision, PolicyAction::Deny);
    assert_eq!(out.policy_name, "global-deny");
}

/// DoD: differential correctness — pruned candidate set vs full linear scan
/// produce the identical decision for every request in a corpus. This is the
/// authorization-correctness merge gate.
#[test]
fn differential_pruned_vs_linear_over_corpus() {
    let engine = PolicyEngine::new();

    // A deliberately mixed set: overlapping resources, allow+deny on the same
    // resource (deny must override), a wildcard deny (unprunable), and many
    // unrelated policies that must be pruned away for most requests.
    engine
        .deploy_policy(simple_policy("allow-a", vec![(PolicyAction::Allow, "/a")]))
        .unwrap();
    engine
        .deploy_policy(simple_policy("deny-a", vec![(PolicyAction::Deny, "/a")]))
        .unwrap();
    engine
        .deploy_policy(simple_policy("allow-b", vec![(PolicyAction::Allow, "/b")]))
        .unwrap();
    engine
        .deploy_policy(simple_policy(
            "allow-c-and-d",
            vec![(PolicyAction::Allow, "/c"), (PolicyAction::Allow, "/d")],
        ))
        .unwrap();
    engine
        .deploy_policy(simple_policy(
            "deny-secret-wildcard",
            vec![(PolicyAction::Allow, "/e"), (PolicyAction::Deny, "*")],
        ))
        .unwrap();
    // Noise: 500 unrelated single-resource policies.
    for i in 0..500 {
        engine
            .deploy_policy(simple_policy(
                &format!("noise{i}"),
                vec![(PolicyAction::Allow, &format!("/noise/{i}"))],
            ))
            .unwrap();
    }

    let all_ids: Vec<Uuid> = engine.list_policies().into_iter().map(|p| p.id).collect();

    // Corpus of resources: matched, overlapping, wildcard-only, unrelated.
    let corpus = [
        "/a",
        "/b",
        "/c",
        "/d",
        "/e",
        "/noise/3",
        "/noise/499",
        "/unrelated",
        "/a/sub",
        "",
    ];

    for res in corpus {
        let req = request(res);
        let linear = engine.evaluate_set(&all_ids, &req);
        let pruned = engine.evaluate_set(&engine.candidate_policy_ids(res, None), &req);
        assert_eq!(
            linear.decision, pruned.decision,
            "decision diverged for resource {res:?}: linear={:?} pruned={:?}",
            linear.decision, pruned.decision
        );
    }
}

/// A shared `DataStore` holding a single `User` entity `alice`. The compiled
/// DSL evaluator resolves the request principal against its baked-in store and
/// fails closed ("User entity not found") if the principal is absent — so EVERY
/// DSL policy below must be built against this store and EVERY request must
/// carry `principal=alice`, or evaluation would error instead of deciding.
/// Pattern mirrors `examples/d2_pruning_impact.rs`.
fn dsl_store_with_alice() -> Arc<DataStore> {
    let store = Arc::new(DataStore::new());
    let interner = store.interner();
    let id = interner.intern_counted("alice");
    let etype = interner.intern("User");
    store.insert(EntityBuilder::new(id, etype).build());
    store
}

/// Build a compiled DSL policy whose evaluator is bound to `store` (so the
/// principal resolves at eval time). `deploy_policy` does NOT rebuild the
/// evaluator, so the store sticks for the life of the deployment.
fn dsl_policy_with_store(name: &str, rule_body: &str, store: &Arc<DataStore>) -> EnhancedPolicy {
    let content = format!(
        "policy {name} {{\n    default: deny,\n    rule r {{\n        {rule_body}\n    }}\n}}"
    );
    let mut p = EnhancedPolicy::new_with_language(
        name.to_string(),
        String::new(),
        PolicyLanguage::ReaperDsl,
        content,
    )
    .expect("DSL policy should compile");
    p.build_evaluator_with_data(Some(store.clone()))
        .expect("DSL evaluator should build against the populated store");
    p
}

/// A request carrying `principal=alice` (required for the compiled DSL
/// evaluator to resolve the principal) plus the given action.
fn dsl_request(resource: &str, action: &str) -> PolicyRequest {
    let mut context = HashMap::new();
    context.insert("principal".to_string(), "alice".to_string());
    PolicyRequest {
        resource: resource.to_string(),
        action: action.to_string(),
        context,

        ..Default::default()
    }
}

/// DoD (DSL): differential correctness for the mandated language — the pruned
/// candidate set produces the identical decision as the full linear scan for
/// every request in a mixed corpus of DSL policies. This is the merge-gate
/// guard that D2 resource-literal pruning never changes a DSL authorization
/// outcome. Mirrors `differential_pruned_vs_linear_over_corpus` (Simple) but
/// exercises compiled DSL policies built against a populated store.
#[test]
fn dsl_differential_pruned_vs_linear_over_corpus() {
    let engine = PolicyEngine::new();
    let store = dsl_store_with_alice();

    // A deliberately mixed set:
    //  - deny-override on the SAME resource (/a): both must decide Deny.
    engine
        .deploy_policy(dsl_policy_with_store(
            "allow_a",
            "allow if resource == \"/a\"",
            &store,
        ))
        .unwrap();
    engine
        .deploy_policy(dsl_policy_with_store(
            "deny_a",
            "deny if resource == \"/a\"",
            &store,
        ))
        .unwrap();
    //  - a single-literal allow (/b).
    engine
        .deploy_policy(dsl_policy_with_store(
            "allow_b",
            "allow if resource == \"/b\"",
            &store,
        ))
        .unwrap();
    //  - an OR-of-literals: prunable, buckets both /c and /d.
    engine
        .deploy_policy(dsl_policy_with_store(
            "allow_c_or_d",
            "allow if resource == \"/c\" || resource == \"/d\"",
            &store,
        ))
        .unwrap();
    //  - an UNPRUNABLE policy: constrains the action, not the resource, so it
    //    has no resource-index terms and is always a candidate. It reads
    //    `action` from the request (no resource entity), so it evaluates
    //    cleanly across the whole corpus without erroring.
    engine
        .deploy_policy(dsl_policy_with_store(
            "allow_admin_action",
            "allow if action == \"admin\"",
            &store,
        ))
        .unwrap();
    //  - noise: 200 unrelated single-literal DSL policies that must be pruned
    //    away for every request that does not name them.
    for i in 0..200 {
        engine
            .deploy_policy(dsl_policy_with_store(
                &format!("noise{i}"),
                &format!("allow if resource == \"/noise/{i}\""),
                &store,
            ))
            .unwrap();
    }

    // Sanity: exactly one unprunable policy (the action predicate); everything
    // else is bucketed by resource literal.
    assert_eq!(
        engine.get_index_stats().unprunable_policies,
        1,
        "only the action-predicate DSL policy is unprunable"
    );

    let all_ids: Vec<Uuid> = engine.list_policies().into_iter().map(|p| p.id).collect();

    // (resource, action) corpus exercising every required case:
    //  /a (read)      deny-override -> Deny
    //  /b,/c,/d(read) single + or-of-literals -> Allow
    //  /admin (admin) resource only the unprunable action policy can act on -> Allow
    //  /noise/*(read) a named noise policy -> Allow
    //  /unrelated,"" (read) nothing matches -> set-level default Deny
    let corpus = [
        ("/a", "read"),
        ("/b", "read"),
        ("/c", "read"),
        ("/d", "read"),
        ("/admin", "admin"),
        ("/noise/7", "read"),
        ("/noise/199", "read"),
        ("/unrelated", "read"),
        ("", "read"),
    ];

    for (res, action) in corpus {
        let req = dsl_request(res, action);
        let linear = engine.evaluate_set(&all_ids, &req);
        let pruned = engine.evaluate_set(&engine.candidate_policy_ids(res, None), &req);
        // No corpus request may error during evaluation (a stray Err short-
        // circuits evaluate_set to Deny and would falsely pass/fail the
        // differential). The linear scan touches every policy, so a clean
        // `error == None` here proves NO policy errored on this request.
        assert!(
            linear.error.is_none(),
            "linear scan errored for {res:?}/{action:?}: {:?}",
            linear.error
        );
        assert!(
            pruned.error.is_none(),
            "pruned scan errored for {res:?}/{action:?}: {:?}",
            pruned.error
        );
        assert_eq!(
            linear.decision, pruned.decision,
            "decision diverged for {res:?}/{action:?}: linear={:?} pruned={:?}",
            linear.decision, pruned.decision
        );
    }

    // Pin the intended decisions so a future regression that makes both paths
    // agree on the WRONG answer is still caught.
    let expect = |res: &str, action: &str| engine.evaluate_set(&all_ids, &dsl_request(res, action));
    assert_eq!(expect("/a", "read").decision, PolicyAction::Deny); // deny override
    assert_eq!(expect("/c", "read").decision, PolicyAction::Allow); // or-of-literals
    assert_eq!(expect("/admin", "admin").decision, PolicyAction::Allow); // unprunable only
    assert_eq!(expect("/unrelated", "read").decision, PolicyAction::Deny); // default deny
}

/// The index survives redeploy (terms change), removal, and a full replace —
/// no stale bucket entries, no lost policies.
#[test]
fn index_consistent_across_mutations() {
    let engine = PolicyEngine::new();

    let mut p = simple_policy("mover", vec![(PolicyAction::Allow, "/old")]);
    let id = p.id;
    engine.deploy_policy(p.clone()).unwrap();
    assert_eq!(engine.candidate_policy_ids("/old", None), vec![id]);

    // Redeploy the SAME id pointing at a new resource — old bucket must clear.
    // `update_rules` rebuilds the evaluator (the match authority the index now
    // reads via `resource_index_terms`); mutating `p.rules` alone would leave a
    // stale evaluator still matching "/old".
    p.update_rules(vec![PolicyRule {
        action: PolicyAction::Allow,
        resource: "/new".to_string(),
        conditions: vec![],
    }]);
    engine.deploy_policy(p.clone()).unwrap();
    assert!(
        engine.candidate_policy_ids("/old", None).is_empty(),
        "stale /old bucket not cleared on redeploy"
    );
    assert_eq!(engine.candidate_policy_ids("/new", None), vec![id]);

    // Remove — bucket empties and is pruned.
    engine.remove_policy(&id).unwrap();
    assert!(engine.candidate_policy_ids("/new", None).is_empty());
    assert_eq!(engine.get_index_stats().resource_buckets, 0);

    // Full replace rebuilds the index atomically.
    let a = simple_policy("a", vec![(PolicyAction::Allow, "/x")]);
    let b = simple_policy("b", vec![(PolicyAction::Deny, "*")]);
    let (aid, bid) = (a.id, b.id);
    engine.replace_all_policies(vec![a, b]).unwrap();
    let stats = engine.get_index_stats();
    assert_eq!(stats.total_policies, 2);
    assert_eq!(stats.unprunable_policies, 1);
    // /x matches the specific policy AND the unprunable wildcard.
    let for_x = engine.candidate_policy_ids("/x", None);
    assert_eq!(for_x.len(), 2);
    assert!(for_x.contains(&aid) && for_x.contains(&bid));
    // /y matches only the wildcard.
    assert_eq!(engine.candidate_policy_ids("/y", None), vec![bid]);
}

/// Non-matching policies are non-decisive (the semantic fix that makes pruning
/// sound): a set of policies that all say nothing about the request falls
/// through to the set-level default deny — not to any policy's per-policy
/// default attribution.
#[test]
fn non_matching_policies_are_non_decisive() {
    let engine = PolicyEngine::new();
    let allow_a = simple_policy("allow-a", vec![(PolicyAction::Allow, "/a")]);
    let allow_b = simple_policy("allow-b", vec![(PolicyAction::Allow, "/b")]);
    let ids = vec![allow_a.id, allow_b.id];
    engine.deploy_policy(allow_a).unwrap();
    engine.deploy_policy(allow_b).unwrap();

    // Request for /c: neither policy matches -> non-decisive -> default deny,
    // nil attribution (not attributed to allow-a or allow-b).
    let out = engine.evaluate_set(&ids, &request("/c"));
    assert_eq!(out.decision, PolicyAction::Deny);
    assert_eq!(out.policy_id, Uuid::nil());
    assert!(out.policy_name.is_empty());

    // Request for /a: only allow-a matches -> Allow, attributed to allow-a.
    let out = engine.evaluate_set(&ids, &request("/a"));
    assert_eq!(out.decision, PolicyAction::Allow);
    assert_eq!(out.policy_name, "allow-a");
}

/// Reference model for `candidate_policy_ids`: every policy with a literal rule
/// on `resource`, plus every wildcard policy, sorted and deduped. The engine
/// now produces this via pre-sorted buckets merged with the pre-sorted
/// unprunable mirror (round-3 Plan 06 C) — this pins that the merge rewrite is
/// observationally identical to the old sort-per-request implementation.
fn reference_candidates(engine: &PolicyEngine, resource: &str) -> Vec<Uuid> {
    let mut ids: Vec<Uuid> = engine
        .list_policies()
        .into_iter()
        .filter(|p| {
            p.rules
                .iter()
                .any(|r| r.resource == "*" || r.resource == resource)
        })
        .map(|p| p.id)
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

/// The candidate set must be sorted, deduped, and identical to the reference
/// model — including after redeploys that move a policy between the prunable
/// (bucketed) and unprunable (wildcard) categories, which exercises the
/// `unprunable_sorted` mirror staying in lock-step with the `unprunable` map.
#[test]
fn candidates_stay_sorted_and_deduped_across_category_transitions() {
    let engine = PolicyEngine::new();

    // Random UUIDv4 ids arrive in arbitrary order — sortedness must come from
    // the index maintenance, not deployment order.
    for i in 0..50 {
        engine
            .deploy_policy(simple_policy(
                &format!("lit{i}"),
                vec![(PolicyAction::Allow, "/shared")],
            ))
            .unwrap();
    }
    for i in 0..20 {
        engine
            .deploy_policy(simple_policy(
                &format!("wild{i}"),
                vec![(PolicyAction::Deny, "*")],
            ))
            .unwrap();
    }

    let assert_matches_reference = |resource: &str| {
        let got = engine.candidate_policy_ids(resource, None);
        assert!(
            got.windows(2).all(|w| w[0] < w[1]),
            "candidates for {resource:?} not strictly sorted (sorted + deduped)"
        );
        assert_eq!(
            got,
            reference_candidates(&engine, resource),
            "candidates for {resource:?} diverge from the reference model"
        );
    };
    assert_matches_reference("/shared");
    assert_matches_reference("/absent");

    // Prunable -> unprunable: redeploy a literal policy as a wildcard. The
    // mirror must gain it (candidate for EVERY resource), the bucket must lose it.
    let mut mover = simple_policy("mover", vec![(PolicyAction::Allow, "/only-here")]);
    let mover_id = mover.id;
    engine.deploy_policy(mover.clone()).unwrap();
    mover.update_rules(vec![PolicyRule {
        action: PolicyAction::Allow,
        resource: "*".to_string(),
        conditions: vec![],
    }]);
    engine.deploy_policy(mover.clone()).unwrap();
    assert!(engine
        .candidate_policy_ids("/absent", None)
        .contains(&mover_id));
    assert_matches_reference("/shared");
    assert_matches_reference("/only-here");

    // Unprunable -> prunable: back to a literal. The mirror must drop it.
    mover.update_rules(vec![PolicyRule {
        action: PolicyAction::Allow,
        resource: "/only-here".to_string(),
        conditions: vec![],
    }]);
    engine.deploy_policy(mover).unwrap();
    assert!(
        !engine
            .candidate_policy_ids("/absent", None)
            .contains(&mover_id),
        "mover left the unprunable set but its id is still served from the sorted mirror"
    );
    assert_matches_reference("/only-here");

    // Remove an unprunable policy outright — mirror must shrink with the map.
    let wild_id = engine
        .list_policies()
        .into_iter()
        .find(|p| p.name == "wild0")
        .map(|p| p.id)
        .unwrap();
    engine.remove_policy(&wild_id).unwrap();
    assert!(!engine
        .candidate_policy_ids("/absent", None)
        .contains(&wild_id));
    assert_matches_reference("/shared");
    assert_matches_reference("/absent");

    // Full replace rebuilds mirror + buckets atomically.
    let a = simple_policy("ra", vec![(PolicyAction::Allow, "/x")]);
    let w = simple_policy("rw", vec![(PolicyAction::Deny, "*")]);
    engine.replace_all_policies(vec![a, w]).unwrap();
    assert_matches_reference("/x");
    assert_matches_reference("/y");
}

// ===========================================================================
// R3-P2-1: resource-type tier — ABAC/ReBAC-shaped DSL policies are prunable
// ===========================================================================

/// A shared `DataStore` with the principal `alice` (role=admin) and typed
/// resource entities — the store both the evaluators and the request-time type
/// resolution (`resource_type_attr`) read, as in the agent.
fn typed_store() -> Arc<DataStore> {
    let store = Arc::new(DataStore::new());
    let interner = store.interner();
    let alice = interner.intern_counted("alice");
    let user_type = interner.intern("User");
    let role_key = interner.intern("role");
    let admin = interner.intern("admin");
    store.insert(
        EntityBuilder::new(alice, user_type)
            .with_string(role_key, admin)
            .build(),
    );
    let res_type = interner.intern("Resource");
    let type_key = interner.intern("type");
    for (rid, rtype) in [
        ("inv1", "invoice"),
        ("inv2", "invoice"),
        ("rep1", "report"),
        ("doc1", "document"),
    ] {
        let rid_i = interner.intern(rid);
        let rtype_i = interner.intern(rtype);
        store.insert(
            EntityBuilder::new(rid_i, res_type)
                .with_string(type_key, rtype_i)
                .build(),
        );
    }
    store
}

/// The engine-level candidate lookup exactly as the agent serves it: type
/// resolved from the same store the evaluators read.
fn typed_candidates(engine: &PolicyEngine, store: &DataStore, resource: &str) -> Vec<Uuid> {
    engine.candidate_policy_ids(resource, store.resource_type_attr(resource).as_deref())
}

/// Type-bounded DSL policies land in the type tier, are candidates exactly for
/// resources of their type, and survive redeploy/remove/replace transitions.
#[test]
fn type_bounded_policies_use_type_tier() {
    let engine = PolicyEngine::new();
    let store = typed_store();

    let inv = dsl_policy_with_store("inv", "allow if resource.type == \"invoice\"", &store);
    let rep = dsl_policy_with_store("rep", "deny if resource.type == \"report\"", &store);
    let (inv_id, rep_id) = (inv.id, rep.id);
    engine.deploy_policy(inv).unwrap();
    engine.deploy_policy(rep).unwrap();

    let stats = engine.get_index_stats();
    assert_eq!(stats.unprunable_policies, 0);
    assert_eq!(stats.resource_buckets, 0);
    assert_eq!(stats.type_buckets, 2);

    // Candidates follow the resolved type; typeless resources get none.
    assert_eq!(typed_candidates(&engine, &store, "inv1"), vec![inv_id]);
    assert_eq!(typed_candidates(&engine, &store, "inv2"), vec![inv_id]);
    assert_eq!(typed_candidates(&engine, &store, "rep1"), vec![rep_id]);
    assert!(typed_candidates(&engine, &store, "doc1").is_empty());
    assert!(typed_candidates(&engine, &store, "/no-entity").is_empty());

    // And the pruned sets actually decide correctly.
    let out = engine.evaluate_set(
        &typed_candidates(&engine, &store, "inv1"),
        &dsl_request("inv1", "read"),
    );
    assert_eq!(out.decision, PolicyAction::Allow);
    assert_eq!(out.policy_name, "inv");
    let out = engine.evaluate_set(
        &typed_candidates(&engine, &store, "rep1"),
        &dsl_request("rep1", "read"),
    );
    assert_eq!(out.decision, PolicyAction::Deny);
    assert_eq!(out.policy_name, "rep");

    // Redeploy type-bounded -> id-bounded: the type bucket must clear.
    let moved =
        dsl_policy_with_store_and_id("inv", "allow if resource == \"doc1\"", &store, inv_id);
    engine.deploy_policy(moved).unwrap();
    assert!(
        typed_candidates(&engine, &store, "inv1").is_empty(),
        "stale invoice type bucket not cleared on redeploy"
    );
    assert_eq!(typed_candidates(&engine, &store, "doc1"), vec![inv_id]);
    assert_eq!(engine.get_index_stats().type_buckets, 1);

    // Remove the remaining type-bounded policy: its bucket is pruned.
    engine.remove_policy(&rep_id).unwrap();
    assert!(typed_candidates(&engine, &store, "rep1").is_empty());
    assert_eq!(engine.get_index_stats().type_buckets, 0);

    // Full replace rebuilds the type tier atomically.
    let a = dsl_policy_with_store("a", "allow if resource.type == \"document\"", &store);
    let aid = a.id;
    engine.replace_all_policies(vec![a]).unwrap();
    assert_eq!(typed_candidates(&engine, &store, "doc1"), vec![aid]);
    assert!(typed_candidates(&engine, &store, "inv1").is_empty());
}

/// Rebuild a policy with a FIXED id (redeploy-in-place), evaluator bound to
/// `store`. Mirrors `dsl_policy_with_store` but keeps the identity so the
/// engine treats it as a redeploy of the same policy.
fn dsl_policy_with_store_and_id(
    name: &str,
    rule_body: &str,
    store: &Arc<DataStore>,
    id: Uuid,
) -> EnhancedPolicy {
    let mut p = dsl_policy_with_store(name, rule_body, store);
    p.id = id;
    p
}

/// DoD (R3-P2-1): differential correctness for the TYPE tier — for a corpus
/// mixing id-bounded, type-bounded (ABAC shape), mixed-Or, and unprunable DSL
/// policies over typed entities, the pruned candidate set (with same-store
/// type resolution, exactly as the agent serves it) yields the identical
/// decision as the full linear scan. This is the superset-property merge gate:
/// a type-tier extraction bug that pruned a policy that could match would
/// flip a decision here.
#[test]
fn type_tier_differential_pruned_vs_linear_over_corpus() {
    let engine = PolicyEngine::new();
    let store = typed_store();

    // Deny-override on a type: invoices are readable unless action == delete.
    engine
        .deploy_policy(dsl_policy_with_store(
            "inv_read",
            "allow if resource.type == \"invoice\"",
            &store,
        ))
        .unwrap();
    engine
        .deploy_policy(dsl_policy_with_store(
            "inv_no_delete",
            "deny if resource.type == \"invoice\" && action == \"delete\"",
            &store,
        ))
        .unwrap();
    // The canonical ABAC shape: type conjunct + principal attribute.
    engine
        .deploy_policy(dsl_policy_with_store(
            "rep_admin",
            "allow if resource.type == \"report\" && user.role == \"admin\"",
            &store,
        ))
        .unwrap();
    // Mixed dimensions in one rule: id literal OR type.
    engine
        .deploy_policy(dsl_policy_with_store(
            "doc_or_rep",
            "allow if resource == \"doc1\" || resource.type == \"report\"",
            &store,
        ))
        .unwrap();
    // Id-bounded policy on a TYPED resource (both tiers in play for inv1).
    engine
        .deploy_policy(dsl_policy_with_store(
            "inv1_direct",
            "deny if resource == \"inv1\" && action == \"purge\"",
            &store,
        ))
        .unwrap();
    // Unprunable: action-only predicate, always a candidate.
    engine
        .deploy_policy(dsl_policy_with_store(
            "admin_action",
            "allow if action == \"admin\"",
            &store,
        ))
        .unwrap();
    // Noise on both tiers.
    for i in 0..100 {
        engine
            .deploy_policy(dsl_policy_with_store(
                &format!("noise_id{i}"),
                &format!("allow if resource == \"/noise/{i}\""),
                &store,
            ))
            .unwrap();
        engine
            .deploy_policy(dsl_policy_with_store(
                &format!("noise_ty{i}"),
                &format!("allow if resource.type == \"kind{i}\""),
                &store,
            ))
            .unwrap();
    }

    assert_eq!(engine.get_index_stats().unprunable_policies, 1);

    // Sorted like the pruned candidate set, so first-allow-wins attribution is
    // comparable between the two paths (both scan in ascending id order).
    let mut all_ids: Vec<Uuid> = engine.list_policies().into_iter().map(|p| p.id).collect();
    all_ids.sort();

    // (resource, action) corpus: typed entities under every policy shape,
    // deny-override actions, entity-less resources, id/type collisions.
    let corpus = [
        ("inv1", "read"),
        ("inv1", "delete"),
        ("inv1", "purge"),
        ("inv2", "read"),
        ("inv2", "delete"),
        ("rep1", "read"),
        ("doc1", "read"),
        ("doc1", "admin"),
        ("invoice", "read"),
        ("/noise/42", "read"),
        ("/no-entity", "read"),
        ("/no-entity", "admin"),
        ("", "read"),
    ];

    for (res, action) in corpus {
        let req = dsl_request(res, action);
        let linear = engine.evaluate_set(&all_ids, &req);
        let pruned = engine.evaluate_set(&typed_candidates(&engine, &store, res), &req);
        assert!(
            linear.error.is_none(),
            "linear scan errored for {res:?}/{action:?}: {:?}",
            linear.error
        );
        assert!(
            pruned.error.is_none(),
            "pruned scan errored for {res:?}/{action:?}: {:?}",
            pruned.error
        );
        assert_eq!(
            linear.decision, pruned.decision,
            "decision diverged for {res:?}/{action:?}"
        );
        assert_eq!(
            linear.policy_id, pruned.policy_id,
            "attribution diverged for {res:?}/{action:?}"
        );
    }

    // Pin intended decisions so both paths agreeing on the WRONG answer is
    // still caught.
    let expect = |res: &str, action: &str| engine.evaluate_set(&all_ids, &dsl_request(res, action));
    assert_eq!(expect("inv1", "read").decision, PolicyAction::Allow);
    assert_eq!(expect("inv1", "delete").decision, PolicyAction::Deny); // type deny-override
    assert_eq!(expect("inv1", "purge").decision, PolicyAction::Deny); // id-tier deny on typed resource
    assert_eq!(expect("rep1", "read").decision, PolicyAction::Allow); // ABAC shape
    assert_eq!(expect("doc1", "read").decision, PolicyAction::Allow); // mixed-Or id branch
    assert_eq!(expect("/no-entity", "read").decision, PolicyAction::Deny); // default deny
    assert_eq!(expect("/no-entity", "admin").decision, PolicyAction::Allow); // unprunable
}

/// DoD scale row (R3-P2-1): with 10k ABAC-shaped (attribute/type) policies and
/// evaluate-all, the candidate set for a typed resource is its type bucket +
/// unprunable — NOT all 10k. Before the type tier every one of these policies
/// was unprunable and every request fanned out to 10k evaluations (and the
/// agent's default `max_candidate_policies = 256` blanket-denied).
#[test]
fn ten_k_abac_policies_prune_to_type_bucket() {
    let engine = PolicyEngine::new();
    let store = typed_store();

    const N: usize = 10_000;
    const TYPES: usize = 100;
    for i in 0..N {
        engine
            .deploy_policy(dsl_policy_with_store(
                &format!("abac{i}"),
                &format!(
                    "allow if resource.type == \"kind{}\" && user.role == \"admin\"",
                    i % TYPES
                ),
                &store,
            ))
            .unwrap();
    }

    let stats = engine.get_index_stats();
    assert_eq!(stats.total_policies, N);
    assert_eq!(
        stats.unprunable_policies, 0,
        "ABAC-shaped policies must all be prunable on the type tier"
    );
    assert_eq!(stats.type_buckets, TYPES);

    // A typed resource evaluates only its type's bucket.
    let candidates = engine.candidate_policy_ids("some-res", Some("kind7"));
    assert_eq!(
        candidates.len(),
        N / TYPES,
        "candidate set must be the type bucket, not all {N}"
    );
    // Within the agent's default candidate cap (256) — no blanket deny.
    assert!(candidates.len() <= 256);

    // A typeless resource evaluates nothing at all.
    assert!(engine.candidate_policy_ids("some-res", None).is_empty());
}
