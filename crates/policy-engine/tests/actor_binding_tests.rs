//! F1-s2 part 2: the DSL `actor` entity binding (agentic delegation).
//!
//! A request may carry a non-human `actor` acting on behalf of the human
//! principal. Policies bind it as `actor.*`, distinct from `user.*`. Since
//! F1-s2c the COMPILED evaluator supports `actor` directly, so
//! `build_preferred` serves these policies on the sub-microsecond path (the
//! AST interpreter keeps identical semantics — pinned by the
//! compiled-vs-AST equivalence differential). These tests pin: the binding
//! resolves, actor-less requests read null (non-matching, not error), the
//! delegation pattern works, and the served `build_preferred` path produces
//! the right decisions.

use policy_engine::reap::ReaperPolicy;
use policy_engine::{DataLoader, DataStore, PolicyAction, PolicyEvaluator, PolicyRequest};
use std::collections::HashMap;
use std::sync::Arc;

fn store() -> Arc<DataStore> {
    let s = Arc::new(DataStore::new());
    let data = serde_json::json!({
        "entities": [
            {"id": "alice", "type": "user", "attributes": {"role": "engineer"}},
            {"id": "agent-ci", "type": "agent",
             "attributes": {"kind": "agent", "trusted": true, "scopes": ["deploy", "read"]}},
            {"id": "agent-rogue", "type": "agent",
             "attributes": {"kind": "agent", "trusted": false}},
            {"id": "svc-1", "type": "resource", "attributes": {"env": "prod"}}
        ]
    });
    DataLoader::new((*s).clone())
        .load_json(&data.to_string())
        .expect("load");
    s
}

fn req(principal: &str, actor: Option<&str>, action: &str, resource: &str) -> PolicyRequest {
    let mut context = HashMap::new();
    context.insert("principal".to_string(), principal.to_string());
    PolicyRequest {
        resource: resource.to_string(),
        action: action.to_string(),
        context,
        actor: actor.map(str::to_string),
        ..Default::default()
    }
}

/// Only a trusted agent, acting for a human, may deploy.
const POLICY: &str = r#"
policy agent_deploy {
    default: deny,

    rule trusted_agent_deploys {
        allow if {
            context.action == "deploy" &&
            actor.kind == "agent" &&
            actor.trusted == true &&
            user.role == "engineer"
        }
    }
}
"#;

fn preferred(store: Arc<DataStore>) -> Box<dyn PolicyEvaluator> {
    let policy: ReaperPolicy = POLICY.parse().expect("parse");
    // build_preferred is the served path: compiled when possible, AST
    // fallback otherwise. `actor` compiles since F1-s2c.
    policy.build_preferred(store).expect("build")
}

#[test]
fn served_path_compiles_actor_policies() {
    let eval = preferred(store());
    // Actor policies must run on the compiled sub-microsecond path — the
    // founding feature is fast AND secure decisions; the AST interpreter is
    // only a fallback for constructs the compiler can't handle yet.
    assert_eq!(
        eval.evaluator_type(),
        "reaper_dsl",
        "actor policy must be served by the compiled evaluator"
    );
}

#[test]
fn trusted_agent_acting_for_engineer_is_allowed() {
    let eval = preferred(store());
    let d = eval
        .evaluate(&req("alice", Some("agent-ci"), "deploy", "svc-1"))
        .unwrap();
    assert_eq!(d, PolicyAction::Allow);
}

#[test]
fn untrusted_agent_is_denied() {
    let eval = preferred(store());
    let d = eval
        .evaluate(&req("alice", Some("agent-rogue"), "deploy", "svc-1"))
        .unwrap();
    assert_eq!(d, PolicyAction::Deny, "untrusted actor must not deploy");
}

#[test]
fn actorless_request_reads_null_and_denies_not_errors() {
    let eval = preferred(store());
    // No actor on the request: `actor.kind` is null, the rule can't match,
    // and evaluation must DENY cleanly (not error) — a human request against
    // an agent-gated policy is simply unauthorized.
    let d = eval
        .evaluate(&req("alice", None, "deploy", "svc-1"))
        .expect("actor-less request must not error");
    assert_eq!(d, PolicyAction::Deny);
}

#[test]
fn actor_and_user_are_distinct_bindings() {
    // A policy that requires actor != user-style separation: here the actor
    // must be an agent while the principal is a human. Passing the human id
    // as the actor (no agent attributes) must fail the `actor.kind` gate.
    let eval = preferred(store());
    let d = eval
        .evaluate(&req("alice", Some("alice"), "deploy", "svc-1"))
        .unwrap();
    assert_eq!(
        d,
        PolicyAction::Deny,
        "principal-as-actor lacks agent attributes ⇒ deny"
    );
}

#[test]
fn actor_delegation_via_rebac_relationship() {
    // The natural delegation model: an `acts_for` relationship between the
    // agent and the user. Policy allows when the actor acts for the
    // principal AND is trusted.
    let s = Arc::new(DataStore::new());
    let data = serde_json::json!({
        "entities": [
            // ReBAC direction: related(subject, rel, object) holds when the
            // OBJECT declares the relation. "agent-b acts for bob" ⇒ the user
            // (object) bob declares acts_for → agent-b (subject).
            {"id": "alice", "type": "user", "attributes": {"role": "engineer"}},
            {"id": "bob", "type": "user", "attributes": {"role": "engineer"},
             "relationships": {"acts_for": ["agent-b"]}},
            {"id": "agent-b", "type": "agent", "attributes": {"trusted": true}},
            {"id": "doc-9", "type": "resource", "attributes": {}}
        ]
    });
    DataLoader::new((*s).clone())
        .load_json(&data.to_string())
        .unwrap();

    let policy_src = r#"
policy delegated {
    default: deny,
    rule agent_acts_for_user {
        allow if {
            actor.trusted == true &&
            rebac::related(actor, "acts_for", user)
        }
    }
}
"#;
    let policy: ReaperPolicy = policy_src.parse().unwrap();
    let eval = policy.build_preferred(s).unwrap();
    // rebac::related(actor, ...) compiles too — no fallback.
    assert_eq!(eval.evaluator_type(), "reaper_dsl");

    // agent-b acts_for bob → allow.
    let allow = eval
        .evaluate(&req("bob", Some("agent-b"), "read", "doc-9"))
        .unwrap();
    assert_eq!(
        allow,
        PolicyAction::Allow,
        "agent acting for bob is allowed"
    );

    // agent-b does NOT act_for alice → deny.
    let deny = eval
        .evaluate(&req("alice", Some("agent-b"), "read", "doc-9"))
        .unwrap();
    assert_eq!(deny, PolicyAction::Deny, "agent does not act for alice");
}
