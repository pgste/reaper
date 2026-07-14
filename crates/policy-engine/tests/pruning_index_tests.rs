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

use policy_engine::{
    EnhancedPolicy, PolicyAction, PolicyEngine, PolicyLanguage, PolicyRequest, PolicyRule,
};
use std::collections::HashMap;
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
    let candidates = engine.candidate_policy_ids(target);
    assert_eq!(
        candidates.len(),
        3,
        "candidate set must be the 3 matching policies, not all {}",
        N + 3
    );

    // A resource nobody references yields zero candidates (default deny, no evals).
    assert_eq!(engine.candidate_policy_ids("/nobody/here").len(), 0);
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

    // Two DSL policies whose rules constrain an attribute / the user, not the
    // request resource identity — these must be unprunable.
    engine
        .deploy_policy(dsl_policy("attr", "allow if resource.type == \"invoice\""))
        .unwrap();
    engine
        .deploy_policy(dsl_policy("blocked", "deny if user.role == \"blocked\""))
        .unwrap();

    let stats = engine.get_index_stats();
    assert_eq!(stats.total_policies, N + 3 + 2);
    assert_eq!(
        stats.unprunable_policies, 2,
        "only the attribute/user-predicate DSL policies are unprunable"
    );
    assert_eq!(
        stats.resource_buckets,
        N + 1,
        "N distinct resources + the shared target bucket"
    );

    // The served path evaluates only the 3 literal matches + the 2 unprunable —
    // not all N+5.
    let candidates = engine.candidate_policy_ids(target);
    assert_eq!(
        candidates.len(),
        3 + 2,
        "candidate set must be 3 literal matches + 2 unprunable, not all {}",
        N + 5
    );

    // A resource nobody references literally: only the 2 unprunable are candidates.
    assert_eq!(engine.candidate_policy_ids("/nobody/here").len(), 2);

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
    assert_eq!(engine.candidate_policy_ids("/x"), vec![id]);
    assert_eq!(engine.candidate_policy_ids("/y"), vec![id]);
    assert!(engine.candidate_policy_ids("/z").is_empty());
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
    let for_a = engine.candidate_policy_ids("/a");
    assert_eq!(for_a.len(), 2, "specific + wildcard both candidates for /a");
    let for_b = engine.candidate_policy_ids("/b");
    assert_eq!(for_b.len(), 1, "only the wildcard is a candidate for /b");

    // And the wildcard deny actually decides via the pruned set.
    let out = engine.evaluate_set(&engine.candidate_policy_ids("/b"), &request("/b"));
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
        let pruned = engine.evaluate_set(&engine.candidate_policy_ids(res), &req);
        assert_eq!(
            linear.decision, pruned.decision,
            "decision diverged for resource {res:?}: linear={:?} pruned={:?}",
            linear.decision, pruned.decision
        );
    }
}

/// The index survives redeploy (terms change), removal, and a full replace —
/// no stale bucket entries, no lost policies.
#[test]
fn index_consistent_across_mutations() {
    let engine = PolicyEngine::new();

    let mut p = simple_policy("mover", vec![(PolicyAction::Allow, "/old")]);
    let id = p.id;
    engine.deploy_policy(p.clone()).unwrap();
    assert_eq!(engine.candidate_policy_ids("/old"), vec![id]);

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
        engine.candidate_policy_ids("/old").is_empty(),
        "stale /old bucket not cleared on redeploy"
    );
    assert_eq!(engine.candidate_policy_ids("/new"), vec![id]);

    // Remove — bucket empties and is pruned.
    engine.remove_policy(&id).unwrap();
    assert!(engine.candidate_policy_ids("/new").is_empty());
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
    let for_x = engine.candidate_policy_ids("/x");
    assert_eq!(for_x.len(), 2);
    assert!(for_x.contains(&aid) && for_x.contains(&bid));
    // /y matches only the wildcard.
    assert_eq!(engine.candidate_policy_ids("/y"), vec![bid]);
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
