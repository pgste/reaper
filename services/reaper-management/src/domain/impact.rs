//! Headless access-impact analysis (Plan 12 step 3).
//!
//! "Who gains or loses access if this migration applies?" — answered by
//! loading the BEFORE and AFTER materialized documents into a real
//! policy-engine `DataStore` (the same loader agents use, so what we measure
//! is what production would evaluate) and diffing an access profile:
//!
//! - **grants**: `(principal, permission)` pairs from the RBAC-as-ABAC
//!   materialization (`permissions` attribute),
//! - **edges**: direct relation edges as the engine's graph holds them,
//! - **reachable**: the transitive closure over traversal relations (what
//!   `rebac::reachable` walks — group membership chains).
//!
//! Renames are compared modulo the rename map: BEFORE access is re-expressed
//! in AFTER vocabulary first, so a pure rename diffs to ZERO. A rename that
//! still changes access is a bug the report must surface, not normalize
//! away — only the *names* are mapped, never the shape.

use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use utoipa::ToSchema;

use super::migration::RenameMaps;

/// The engine-visible access surface of one materialized document.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AccessProfile {
    /// (principal, permission)
    pub grants: BTreeSet<(String, String)>,
    /// (carrier, relation, target) — direct edges, engine direction.
    pub edges: BTreeSet<(String, String, String)>,
    /// (origin, relation, reachable-target) — closure over traversal
    /// relations, the set `rebac::reachable` answers from.
    pub reachable: BTreeSet<(String, String, String)>,
}

/// Relations to profile: name + whether the engine treats it as traversal.
#[derive(Debug, Clone)]
pub struct RelationSpec {
    pub name: String,
    pub traversal: bool,
}

/// Build the access profile by loading `document` into a throwaway
/// policy-engine `DataStore` — the exact loader path agents run, so the
/// profile reflects real evaluation state, not a re-implementation of it.
pub fn access_profile(
    document: &serde_json::Value,
    relations: &[RelationSpec],
) -> Result<AccessProfile, String> {
    let json = serde_json::to_string(document).map_err(|e| e.to_string())?;
    let store = policy_engine::data::loader::from_json(&json)
        .map_err(|e| format!("engine load failed: {e}"))?;

    let mut profile = AccessProfile::default();
    let interner = store.interner();
    let graph = store.relationships();

    for entity in store.all() {
        let id = interner
            .resolve(entity.id)
            .map(|s| s.to_string())
            .unwrap_or_default();

        // Grants: the materialized `permissions` attribute is the engine's
        // RBAC truth (one interned lookup at eval time).
        if let Some(attrs) = store.entity_attributes_json(&id) {
            if let Some(perms) = attrs.get("permissions").and_then(|p| p.as_array()) {
                for p in perms.iter().filter_map(|p| p.as_str()) {
                    profile.grants.insert((id.clone(), p.to_string()));
                }
            }
        }

        // Direct edges, straight out of the engine's relationship graph.
        for rel in relations {
            let Some(rel_sym) = interner.lookup(&rel.name) else {
                continue; // relation never materialized in this store
            };
            for target in graph.related(entity.id, rel_sym).iter() {
                let target_s = interner
                    .resolve(*target)
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                profile
                    .edges
                    .insert((id.clone(), rel.name.clone(), target_s));
            }
        }
    }

    // Transitive closure per traversal relation (BFS over the edges the
    // engine holds — same walk rebac::reachable performs, made exhaustive).
    for rel in relations.iter().filter(|r| r.traversal) {
        let mut adjacency: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for (from, r, to) in &profile.edges {
            if r == &rel.name {
                adjacency
                    .entry(from.as_str())
                    .or_default()
                    .push(to.as_str());
            }
        }
        for origin in adjacency.keys().copied().collect::<Vec<_>>() {
            let mut seen: BTreeSet<&str> = BTreeSet::new();
            let mut queue: VecDeque<&str> = VecDeque::from([origin]);
            while let Some(node) = queue.pop_front() {
                for next in adjacency.get(node).into_iter().flatten() {
                    if *next != origin && seen.insert(next) {
                        queue.push_back(next);
                    }
                }
            }
            for target in seen {
                profile.reachable.insert((
                    origin.to_string(),
                    rel.name.clone(),
                    target.to_string(),
                ));
            }
        }
    }

    Ok(profile)
}

/// Re-express a profile in post-migration vocabulary (relation renames).
/// Grants are keyed by permission strings, which renames never touch; only
/// relation names need mapping.
pub fn normalize(profile: &AccessProfile, maps: &RenameMaps) -> AccessProfile {
    if maps.relations.is_empty() {
        return profile.clone();
    }
    let map_rel =
        |r: &String| -> String { maps.relations.get(r).cloned().unwrap_or_else(|| r.clone()) };
    AccessProfile {
        grants: profile.grants.clone(),
        edges: profile
            .edges
            .iter()
            .map(|(f, r, t)| (f.clone(), map_rel(r), t.clone()))
            .collect(),
        reachable: profile
            .reachable
            .iter()
            .map(|(f, r, t)| (f.clone(), map_rel(r), t.clone()))
            .collect(),
    }
}

/// How many principal ids the report lists before count-only. Full sets can
/// be recomputed; the report must stay readable at 100k principals.
const SAMPLE_CAP: usize = 50;

/// The dry-run's headline: who gains, who loses, what moved.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ImpactReport {
    pub principals_gaining: usize,
    pub principals_losing: usize,
    /// Sorted samples (capped) — the counts above are authoritative.
    pub gaining_sample: Vec<String>,
    pub losing_sample: Vec<String>,
    pub grants_added: usize,
    pub grants_removed: usize,
    pub edges_added: usize,
    pub edges_removed: usize,
    pub reachability_added: usize,
    pub reachability_removed: usize,
    /// True iff no principal gains or loses anything — the required outcome
    /// for a pure rename.
    pub decision_neutral: bool,
}

/// Diff two profiles (BEFORE must already be normalized into AFTER
/// vocabulary). A principal "gains" if any grant or reachable target exists
/// after that did not exist before; "loses" is the reverse.
pub fn diff(before: &AccessProfile, after: &AccessProfile) -> ImpactReport {
    let grants_added: Vec<_> = after.grants.difference(&before.grants).collect();
    let grants_removed: Vec<_> = before.grants.difference(&after.grants).collect();
    let edges_added = after.edges.difference(&before.edges).count();
    let edges_removed = before.edges.difference(&after.edges).count();
    let reach_added: Vec<_> = after.reachable.difference(&before.reachable).collect();
    let reach_removed: Vec<_> = before.reachable.difference(&after.reachable).collect();

    let mut gaining: BTreeSet<&str> = BTreeSet::new();
    for (p, _) in &grants_added {
        gaining.insert(p);
    }
    for (p, _, _) in &reach_added {
        gaining.insert(p);
    }
    let mut losing: BTreeSet<&str> = BTreeSet::new();
    for (p, _) in &grants_removed {
        losing.insert(p);
    }
    for (p, _, _) in &reach_removed {
        losing.insert(p);
    }

    let sample = |set: &BTreeSet<&str>| -> Vec<String> {
        set.iter().take(SAMPLE_CAP).map(|s| s.to_string()).collect()
    };

    ImpactReport {
        principals_gaining: gaining.len(),
        principals_losing: losing.len(),
        gaining_sample: sample(&gaining),
        losing_sample: sample(&losing),
        grants_added: grants_added.len(),
        grants_removed: grants_removed.len(),
        edges_added,
        edges_removed,
        reachability_added: reach_added.len(),
        reachability_removed: reach_removed.len(),
        decision_neutral: gaining.is_empty() && losing.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn relations() -> Vec<RelationSpec> {
        vec![
            RelationSpec {
                name: "owner".into(),
                traversal: false,
            },
            RelationSpec {
                name: "member_of".into(),
                traversal: true,
            },
        ]
    }

    #[test]
    fn profile_reads_grants_edges_and_transitive_reachability() {
        // alice -member_of-> team-a -member_of-> org-eng (traversal chain,
        // edges carried on the subject); doc-1 -owner-> alice.
        let doc = json!({"entities": [
            {"id": "alice", "type": "user",
             "attributes": {"permissions": ["resource:read"]},
             "relationships": {"member_of": ["team-a"]}},
            {"id": "team-a", "type": "group", "attributes": {},
             "relationships": {"member_of": ["org-eng"]}},
            {"id": "org-eng", "type": "group", "attributes": {}},
            {"id": "doc-1", "type": "resource", "attributes": {},
             "relationships": {"owner": ["alice"]}},
        ]});
        let p = access_profile(&doc, &relations()).unwrap();

        assert!(p.grants.contains(&("alice".into(), "resource:read".into())));
        assert!(p
            .edges
            .contains(&("doc-1".into(), "owner".into(), "alice".into())));
        assert!(p
            .reachable
            .contains(&("alice".into(), "member_of".into(), "team-a".into())));
        assert!(
            p.reachable
                .contains(&("alice".into(), "member_of".into(), "org-eng".into())),
            "closure must include the transitive hop"
        );
    }

    #[test]
    fn pure_relation_rename_is_decision_neutral_after_normalization() {
        let before = json!({"entities": [
            {"id": "doc-1", "type": "resource", "attributes": {},
             "relationships": {"owner": ["alice"]}},
        ]});
        let after = json!({"entities": [
            {"id": "doc-1", "type": "resource", "attributes": {},
             "relationships": {"possessor": ["alice"]}},
        ]});
        let rels_before = vec![RelationSpec {
            name: "owner".into(),
            traversal: false,
        }];
        let rels_after = vec![RelationSpec {
            name: "possessor".into(),
            traversal: false,
        }];

        let p_before = access_profile(&before, &rels_before).unwrap();
        let p_after = access_profile(&after, &rels_after).unwrap();

        // Naive diff sees churn…
        let naive = diff(&p_before, &p_after);
        assert_eq!(naive.edges_removed, 1);
        assert_eq!(naive.edges_added, 1);

        // …normalized diff sees a no-op.
        let mut maps = RenameMaps::default();
        maps.relations.insert("owner".into(), "possessor".into());
        let report = diff(&normalize(&p_before, &maps), &p_after);
        assert!(report.decision_neutral);
        assert_eq!(report.edges_added, 0);
        assert_eq!(report.edges_removed, 0);
    }

    #[test]
    fn losing_reachability_marks_principals_losing() {
        let before = json!({"entities": [
            {"id": "alice", "type": "user", "attributes": {},
             "relationships": {"member_of": ["team-a"]}},
            {"id": "team-a", "type": "group", "attributes": {}},
        ]});
        let after = json!({"entities": [
            {"id": "alice", "type": "user", "attributes": {}},
            {"id": "team-a", "type": "group", "attributes": {}},
        ]});
        let rels = relations();
        let report = diff(
            &access_profile(&before, &rels).unwrap(),
            &access_profile(&after, &rels).unwrap(),
        );
        assert_eq!(report.principals_losing, 1);
        assert_eq!(report.losing_sample, vec!["alice".to_string()]);
        assert_eq!(report.principals_gaining, 0);
        assert!(!report.decision_neutral);
    }
}
