//! Delta-sync differential property: applying any sequence of entity-level
//! deltas (upserts + tombstones) INCREMENTALLY to a live DataStore must be
//! indistinguishable from rebuilding a fresh store from the final state.
//!
//! This is the correctness gate docs/development/DATA_PLANE_PLAN.md (D2)
//! requires before incremental sync ships: at-least-once delivery means
//! deltas can be re-applied, runs re-pulled after gaps, and an entity's
//! attributes/edges replaced many times over — none of it may leave residue
//! (stale attribute-index entries, dangling relationship edges) that a
//! fresh replica wouldn't have.
//!
//! REFERENTIAL-CASCADE CONTRACT (found by this test's first run): deleting
//! an entity removes it AND every edge referencing it at delete time — a
//! deleted group must not keep granting group-hop access as a dangling
//! reference (fail-open through a tombstone). A LATER upsert may
//! legitimately re-reference the dead id; only edges existing at delete
//! time cascade. The control plane mirrors this by cascading tuple deletes.
//!
//! Tuning: PROPTEST_CASES=1000 cargo test -p policy-engine --test
//! delta_sync_differential_tests --release

use policy_engine::reap::ReaperPolicy;
use policy_engine::{DataLoader, DataStore, PolicyRequest};
use proptest::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;

const USERS: &[&str] = &["alice", "bob", "carol"];
const DOCS: &[&str] = &["doc-a", "doc-b"];
const GROUPS: &[&str] = &["g-one", "g-two"];
const ROLES: &[&str] = &["admin", "editor", "viewer"];

/// One delta: an entity-level upsert (full replacement doc, the shape the
/// changes API emits) or a tombstone.
#[derive(Debug, Clone)]
enum Delta {
    UpsertUser {
        user: usize,
        role: usize,
        level: i64,
        mfa: bool,
        member_of: Vec<usize>, // group indexes (traversal edges: user carries)
    },
    UpsertDoc {
        doc: usize,
        tier: i64,
        owners: Vec<usize>,  // user indexes (doc carries owner edges)
        viewers: Vec<usize>, // group indexes
    },
    UpsertGroup {
        group: usize,
        nested_in: Option<usize>,
    },
    DeleteUser(usize),
    DeleteDoc(usize),
    DeleteGroup(usize),
}

impl Delta {
    /// The entity id this delta targets (for replaying final state).
    fn key(&self) -> String {
        match self {
            Delta::UpsertUser { user, .. } | Delta::DeleteUser(user) => USERS[*user].to_string(),
            Delta::UpsertDoc { doc, .. } | Delta::DeleteDoc(doc) => DOCS[*doc].to_string(),
            Delta::UpsertGroup { group, .. } | Delta::DeleteGroup(group) => {
                GROUPS[*group].to_string()
            }
        }
    }

    fn is_delete(&self) -> bool {
        matches!(
            self,
            Delta::DeleteUser(_) | Delta::DeleteDoc(_) | Delta::DeleteGroup(_)
        )
    }

    /// Render the full entity document this upsert carries.
    fn doc(&self) -> serde_json::Value {
        match self {
            Delta::UpsertUser {
                user,
                role,
                level,
                mfa,
                member_of,
            } => {
                let groups: Vec<&str> = member_of.iter().map(|g| GROUPS[*g]).collect();
                let mut rels = serde_json::Map::new();
                if !groups.is_empty() {
                    rels.insert("member_of".into(), serde_json::json!(groups));
                }
                serde_json::json!({
                    "id": USERS[*user], "type": "user",
                    "attributes": {"role": ROLES[*role], "level": level, "mfa": mfa},
                    "relationships": rels,
                })
            }
            Delta::UpsertDoc {
                doc,
                tier,
                owners,
                viewers,
            } => {
                let mut rels = serde_json::Map::new();
                let owner_ids: Vec<&str> = owners.iter().map(|u| USERS[*u]).collect();
                let viewer_ids: Vec<&str> = viewers.iter().map(|g| GROUPS[*g]).collect();
                if !owner_ids.is_empty() {
                    rels.insert("owner".into(), serde_json::json!(owner_ids));
                }
                if !viewer_ids.is_empty() {
                    rels.insert("viewer".into(), serde_json::json!(viewer_ids));
                }
                serde_json::json!({
                    "id": DOCS[*doc], "type": "resource",
                    "attributes": {"tier": tier},
                    "relationships": rels,
                })
            }
            Delta::UpsertGroup { group, nested_in } => {
                let mut rels = serde_json::Map::new();
                if let Some(parent) = nested_in {
                    if parent != group {
                        rels.insert("member_of".into(), serde_json::json!([GROUPS[*parent]]));
                    }
                }
                serde_json::json!({
                    "id": GROUPS[*group], "type": "group",
                    "attributes": {},
                    "relationships": rels,
                })
            }
            _ => unreachable!("deletes carry no document"),
        }
    }
}

fn delta_strategy() -> impl Strategy<Value = Delta> {
    prop_oneof![
        4 => (
            0..USERS.len(),
            0..ROLES.len(),
            0i64..6,
            any::<bool>(),
            prop::collection::vec(0..GROUPS.len(), 0..3),
        )
            .prop_map(|(user, role, level, mfa, member_of)| Delta::UpsertUser {
                user,
                role,
                level,
                mfa,
                member_of,
            }),
        4 => (
            0..DOCS.len(),
            0i64..6,
            prop::collection::vec(0..USERS.len(), 0..3),
            prop::collection::vec(0..GROUPS.len(), 0..3),
        )
            .prop_map(|(doc, tier, owners, viewers)| Delta::UpsertDoc {
                doc,
                tier,
                owners,
                viewers,
            }),
        2 => (0..GROUPS.len(), prop::option::of(0..GROUPS.len()))
            .prop_map(|(group, nested_in)| Delta::UpsertGroup { group, nested_in }),
        1 => (0..USERS.len()).prop_map(Delta::DeleteUser),
        1 => (0..DOCS.len()).prop_map(Delta::DeleteDoc),
        1 => (0..GROUPS.len()).prop_map(Delta::DeleteGroup),
    ]
}

/// Apply a delta the way the agent's apply-deltas handler does.
fn apply(loader: &DataLoader, delta: &Delta) {
    if delta.is_delete() {
        loader.delete_entity(&delta.key());
    } else {
        loader.upsert_entity_doc(&delta.doc()).expect("upsert");
    }
}

/// The probe policy: RBAC (role attr), ABAC (level + mfa), ReBAC (direct
/// owner + group-hop viewer). Exercises attributes, indexes, and both edge
/// directions — anything stale shows up as a decision difference.
const PROBE_POLICY: &str = r#"
policy delta_probe {
    default: deny,
    rule admins { allow if user.role == "admin" }
    rule mfa_editors {
        allow if { user.role == "editor" && user.mfa == true && resource.tier <= 3 }
    }
    rule owners { allow if rebac::related(user, "owner", resource) }
    rule team_read {
        allow if {
            context.action == "read" &&
            rebac::reachable(user, "viewer", resource, "member_of", 3)
        }
    }
}
"#;

fn decisions(store: Arc<DataStore>) -> Vec<String> {
    let policy: ReaperPolicy = PROBE_POLICY.parse().expect("probe parses");
    let evaluator = policy.build_ast_evaluator(store);
    let mut out = Vec::new();
    for user in USERS {
        for doc in DOCS {
            for action in ["read", "write"] {
                let request = PolicyRequest {
                    resource: doc.to_string(),
                    action: action.to_string(),
                    context: HashMap::from([("principal".to_string(), user.to_string())]),
                };
                out.push(match evaluator.evaluate(&request) {
                    Ok(d) => format!("{user}/{doc}/{action}={d:?}"),
                    // Error text embeds interner-local InternedString(N)
                    // ids that legitimately differ between stores —
                    // normalize them so only the error KIND compares.
                    Err(e) => {
                        let msg = format!("{e:?}");
                        let normalized: String =
                            msg.chars().filter(|c| !c.is_ascii_digit()).collect();
                        format!("{user}/{doc}/{action}=ERR:{normalized}")
                    }
                });
            }
        }
    }
    out
}

/// Probes with entity ids resolved to strings so two stores with different
/// interners compare correctly.
fn resolved_probes(store: &DataStore) -> Vec<String> {
    let interner = store.interner();
    let resolve = |eid: policy_engine::data::EntityId| {
        interner
            .resolve(eid)
            .map(|s| s.to_string())
            .unwrap_or_default()
    };
    let mut out = Vec::new();
    for id in USERS.iter().chain(DOCS).chain(GROUPS) {
        let eid = interner.intern(id);
        out.push(format!("{id}.present={}", store.get(eid).is_some()));
    }
    let graph = store.relationships();
    for rel in ["owner", "viewer", "member_of"] {
        let rel_id = interner.intern(rel);
        for id in USERS.iter().chain(DOCS).chain(GROUPS) {
            let eid = interner.intern(id);
            let mut fwd: Vec<String> = graph
                .related(eid, rel_id)
                .into_iter()
                .map(resolve)
                .collect();
            let mut rev: Vec<String> = graph
                .related_to(eid, rel_id)
                .into_iter()
                .map(resolve)
                .collect();
            fwd.sort();
            rev.sort();
            out.push(format!("{id}#{rel}:fwd={fwd:?},rev={rev:?}"));
        }
    }
    out
}

fn cases_from_env(default: u32) -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: cases_from_env(128),
        max_shrink_iters: 2048,
        ..ProptestConfig::default()
    })]

    #[test]
    fn incremental_delta_apply_equals_fresh_rebuild(
        deltas in prop::collection::vec(delta_strategy(), 1..25),
        // At-least-once delivery: re-apply the final delta afterwards —
        // idempotency must hold.
        redeliver_last in any::<bool>(),
    ) {
        // Incremental replica: apply every delta in order.
        let incremental = Arc::new(DataStore::new());
        let inc_loader = DataLoader::new((*incremental).clone());
        for delta in &deltas {
            apply(&inc_loader, delta);
        }
        if redeliver_last {
            apply(&inc_loader, deltas.last().unwrap());
        }

        // Fresh rebuild from the TRUE state under the referential-cascade
        // contract: replay deltas in order, on delete removing the entity
        // AND stripping references to it from every surviving doc (exactly
        // what RelationshipGraph::detach does at the replica).
        let mut final_state: HashMap<String, serde_json::Value> = HashMap::new();
        for delta in &deltas {
            if delta.is_delete() {
                let dead = delta.key();
                final_state.remove(&dead);
                for doc in final_state.values_mut() {
                    if let Some(rels) =
                        doc.get_mut("relationships").and_then(|r| r.as_object_mut())
                    {
                        for subjects in rels.values_mut() {
                            if let Some(list) = subjects.as_array_mut() {
                                list.retain(|s| s.as_str() != Some(dead.as_str()));
                            }
                        }
                        rels.retain(|_, subjects| {
                            subjects.as_array().is_some_and(|l| !l.is_empty())
                        });
                    }
                }
            } else {
                final_state.insert(delta.key(), delta.doc());
            }
        }
        let rebuilt = Arc::new(DataStore::new());
        let rebuilt_loader = DataLoader::new((*rebuilt).clone());
        for doc in final_state.values() {
            rebuilt_loader.upsert_entity_doc(doc).expect("rebuild upsert");
        }

        // Store-level probes must match (presence + both edge directions).
        prop_assert_eq!(
            resolved_probes(&incremental),
            resolved_probes(&rebuilt),
            "STORE DIVERGENCE after {:?}", deltas
        );

        // Decision-level: the whole point — a replica that answers
        // differently from a fresh build is not a replica.
        prop_assert_eq!(
            decisions(incremental.clone()),
            decisions(rebuilt.clone()),
            "DECISION DIVERGENCE after {:?}", deltas
        );
    }
}
