//! ReBAC relationship edges: named, directed, interned, index-backed.
//!
//! Zanzibar-style semantics: an entity *declares* its relations —
//! `doc1.relationships.owner = ["alice"]` means `doc1 #owner @alice`.
//! Both directions are indexed at write time:
//!
//! - forward:  `(declaring entity, relation) -> subjects` — answers
//!   "who is `owner` of doc1?" in one lookup
//! - reverse:  `(subject, relation) -> declaring entities` — answers
//!   "what is alice `owner` of?" (list-style queries)
//!
//! Performance choices (why this beats an external graph or Rego data walks):
//! - all node/edge labels are interned `u32`s — comparisons are integer
//!   compares, never string hashing on the hot path
//! - adjacency lists are **sorted `SmallVec<[EntityId; 4]>`**: the common
//!   few-edges case lives inline (zero heap indirection), membership is a
//!   branch-light binary search over a contiguous cache line
//! - traversals are bounded BFS with an `FxHashSet` visited set and an
//!   explicit node budget — cycle-safe and worst-case-capped by construction
//!
//! Checks used by policies (shared verbatim by the compiled evaluator and the
//! AST interpreter, so both paths give identical answers):
//! - [`RelationshipGraph::has_relation`] — direct: `subject ∈ object#relation`
//! - [`RelationshipGraph::has_relation_reachable`] — subject-side group
//!   expansion: subject reaches a member of `object#relation` via `via` edges
//!   (e.g. user → team → group that is a viewer)
//! - [`RelationshipGraph::has_relation_inherited`] — object-side ancestor
//!   walk: the relation holds on the object or any ancestor along `up` edges
//!   (e.g. folder hierarchies)

use crate::data::interning::{InternedString, StringInterner};
use crate::data::EntityId;
use dashmap::DashMap;
use rustc_hash::FxHashSet;
use smallvec::SmallVec;
use std::collections::VecDeque;

/// Adjacency list — inline up to 4 edges, sorted for binary-search membership.
pub type EdgeList = SmallVec<[EntityId; 4]>;

/// Hard cap on nodes visited per traversal, independent of the caller's depth
/// bound. Keeps a pathological graph from turning one policy check into a
/// full-graph walk.
const TRAVERSAL_NODE_BUDGET: usize = 4_096;

/// Concurrent, doubly-indexed relationship graph.
#[derive(Debug)]
pub struct RelationshipGraph {
    /// (declaring entity, relation) -> subjects, sorted.
    forward: DashMap<(EntityId, InternedString), EdgeList>,
    /// (subject, relation) -> declaring entities, sorted.
    reverse: DashMap<(EntityId, InternedString), EdgeList>,
    /// entity -> relations it CARRIES (declares) — makes entity-scoped edge
    /// removal O(degree) instead of a full-map scan (delta sync applies
    /// per-entity upserts; a 100k-key scan per delta would be quadratic).
    carrier_rels: DashMap<EntityId, SmallVec<[InternedString; 4]>>,
    /// entity -> relations it appears in as SUBJECT (edge target).
    subject_rels: DashMap<EntityId, SmallVec<[InternedString; 4]>>,
    /// The store's shared interner. The loader counts one reference per
    /// subject occurrence it interns; this graph releases exactly one when
    /// each live forward edge is removed, so a high-cardinality *subject*
    /// churn (and entity ids used only as subjects) is reclaimed rather than
    /// pinned for the life of the store. Relations are pinned (bounded
    /// vocabulary), so releasing them is a no-op.
    interner: StringInterner,
}

impl RelationshipGraph {
    /// Build a graph over the store's shared interner. The interner is
    /// `Clone` but Arc-backed internally, so this shares state with the store
    /// rather than copying it — releases here evict from the same table the
    /// loader interned into.
    pub fn new(interner: StringInterner) -> Self {
        Self {
            forward: DashMap::new(),
            reverse: DashMap::new(),
            carrier_rels: DashMap::new(),
            subject_rels: DashMap::new(),
            interner,
        }
    }

    /// Record `from #relation @to` (idempotent; lists stay sorted).
    ///
    /// The loader interns each subject *counted* once per occurrence in the
    /// source list before calling this. A duplicate occurrence collapses onto
    /// the same stored edge, so its redundant count is released here — the
    /// invariant is **exactly one counted subject reference per live forward
    /// edge**, balanced by the release in every removal path below.
    pub fn add_edge(&self, from: EntityId, relation: InternedString, to: EntityId) {
        let is_new = insert_sorted(
            self.forward
                .entry((from, relation))
                .or_default()
                .value_mut(),
            to,
        );
        insert_sorted(
            self.reverse.entry((to, relation)).or_default().value_mut(),
            from,
        );
        register_rel(&self.carrier_rels, from, relation);
        register_rel(&self.subject_rels, to, relation);
        if !is_new {
            // Duplicate edge — the loader's extra counted reference is redundant.
            self.interner.release(to);
        }
    }

    /// Remove one edge (both directions). Idempotent: removing a missing
    /// edge is a no-op — delta deletes are at-least-once delivered.
    pub fn remove_edge(&self, from: EntityId, relation: InternedString, to: EntityId) {
        let removed = remove_from_list(&self.forward, (from, relation), to);
        remove_from_list(&self.reverse, (to, relation), from);
        if removed {
            // Balances the one counted reference this live edge held on `to`.
            self.interner.release(to);
        }
    }

    /// Drop every edge this entity CARRIES (declares). The upsert primitive:
    /// re-materializing an entity doc replaces its relationship block, so
    /// its old carried edges must vanish — but edges OTHER entities declare
    /// pointing at it are their documents' property and stay.
    pub fn detach_carried(&self, entity: EntityId) {
        if let Some((_, rels)) = self.carrier_rels.remove(&entity) {
            for rel in rels {
                if let Some((_, targets)) = self.forward.remove(&(entity, rel)) {
                    for target in targets {
                        remove_from_list(&self.reverse, (target, rel), entity);
                        // Each removed forward edge held one count on its subject.
                        self.interner.release(target);
                    }
                }
            }
        }
    }

    /// Fully detach an entity — carried edges AND edges pointing at it.
    /// The tombstone primitive: a deleted entity must not linger as anyone's
    /// owner/viewer/member (fail closed: absent entity grants nothing).
    pub fn detach(&self, entity: EntityId) {
        self.detach_carried(entity);
        if let Some((_, rels)) = self.subject_rels.remove(&entity) {
            for rel in rels {
                if let Some((_, carriers)) = self.reverse.remove(&(entity, rel)) {
                    for carrier in carriers {
                        // `entity` is this edge's subject; release its count
                        // only if the forward edge was actually present.
                        if remove_from_list(&self.forward, (carrier, rel), entity) {
                            self.interner.release(entity);
                        }
                    }
                }
            }
        }
    }

    /// Subjects of `object #relation` (direct, one lookup).
    pub fn related(&self, object: EntityId, relation: InternedString) -> EdgeList {
        self.forward
            .get(&(object, relation))
            .map(|e| e.clone())
            .unwrap_or_default()
    }

    /// Objects that declare `#relation @subject` (reverse lookup).
    pub fn related_to(&self, subject: EntityId, relation: InternedString) -> EdgeList {
        self.reverse
            .get(&(subject, relation))
            .map(|e| e.clone())
            .unwrap_or_default()
    }

    /// Direct check: `subject ∈ object#relation`. Two integer-keyed lookups +
    /// a binary search — this is the sub-microsecond building block.
    #[inline]
    pub fn has_relation(
        &self,
        object: EntityId,
        relation: InternedString,
        subject: EntityId,
    ) -> bool {
        self.forward
            .get(&(object, relation))
            .map(|edges| edges.binary_search(&subject).is_ok())
            .unwrap_or(false)
    }

    /// Subject-side expansion: does `subject` hold `relation` on `object`
    /// directly, OR through anything it can reach along its own `via` edges
    /// (memberships), up to `max_depth` hops?
    ///
    /// `user --member_of--> team --member_of--> org`, `doc#viewer@org` ⇒
    /// `has_relation_reachable(doc, viewer, user, member_of, 2..)` is true.
    pub fn has_relation_reachable(
        &self,
        object: EntityId,
        relation: InternedString,
        subject: EntityId,
        via: InternedString,
        max_depth: usize,
    ) -> bool {
        // Fast path: direct relation, no traversal state allocated.
        if self.has_relation(object, relation, subject) {
            return true;
        }

        // Copy out in a single statement so the shard read-guard drops before
        // the BFS re-enters the map (never hold a lock across traversal —
        // a queued writer on the same shard must not be able to wedge us).
        let Some(holders): Option<EdgeList> =
            self.forward.get(&(object, relation)).map(|e| e.clone())
        else {
            return false;
        };

        self.bfs_reaches(subject, via, max_depth, |node| {
            holders.binary_search(&node).is_ok()
        })
    }

    /// Object-side inheritance: does the relation hold on `object` or any of
    /// its ancestors along `up` edges (e.g. `parent`), up to `max_depth`?
    /// `subject` may also match through its own direct membership at each
    /// ancestor level.
    pub fn has_relation_inherited(
        &self,
        object: EntityId,
        relation: InternedString,
        subject: EntityId,
        up: InternedString,
        max_depth: usize,
    ) -> bool {
        if self.has_relation(object, relation, subject) {
            return true;
        }
        self.bfs_reaches(object, up, max_depth, |ancestor| {
            ancestor != object && self.has_relation(ancestor, relation, subject)
        })
    }

    /// Bounded, cycle-safe BFS from `start` along `edge` (forward direction),
    /// returning true as soon as `hit` matches a visited node (start excluded
    /// from the first check only via the caller's predicate when needed).
    fn bfs_reaches<F: Fn(EntityId) -> bool>(
        &self,
        start: EntityId,
        edge: InternedString,
        max_depth: usize,
        hit: F,
    ) -> bool {
        let mut visited: FxHashSet<EntityId> = FxHashSet::default();
        let mut queue: VecDeque<(EntityId, usize)> = VecDeque::new();
        visited.insert(start);
        queue.push_back((start, 0));

        while let Some((node, depth)) = queue.pop_front() {
            if depth >= max_depth || visited.len() > TRAVERSAL_NODE_BUDGET {
                continue;
            }
            // Same single-statement copy: guard drops before we recurse into
            // `hit` (which reads this map again).
            let nexts: Option<EdgeList> = self.forward.get(&(node, edge)).map(|e| e.clone());
            if let Some(nexts) = nexts {
                for next in nexts {
                    if visited.insert(next) {
                        if hit(next) {
                            return true;
                        }
                        queue.push_back((next, depth + 1));
                    }
                }
            }
        }
        false
    }

    /// Drop every edge and registry entry. Used by `DataStore::clear`, which
    /// also `reset_counted`s the interner — so the wholesale interner drop and
    /// this wholesale edge drop stay consistent (no stale edge survives a
    /// snapshot swap to grant a deleted entity a relation, and no counted
    /// subject is orphaned).
    pub fn clear(&self) {
        self.forward.clear();
        self.reverse.clear();
        self.carrier_rels.clear();
        self.subject_rels.clear();
    }

    /// Total number of forward edge lists (diagnostics).
    pub fn len(&self) -> usize {
        self.forward.len()
    }

    pub fn is_empty(&self) -> bool {
        self.forward.is_empty()
    }
}

/// Insert into the sorted list, returning `true` if the value was new (the
/// list changed) and `false` if it was already present. Callers use the flag
/// to keep the interner's subject refcount balanced against idempotent adds.
fn insert_sorted(list: &mut EdgeList, value: EntityId) -> bool {
    match list.binary_search(&value) {
        Ok(_) => false,
        Err(pos) => {
            list.insert(pos, value);
            true
        }
    }
}

/// Register `relation` in an entity's relation registry (dedup, unsorted —
/// registries are tiny and only walked on detach).
fn register_rel(
    registry: &DashMap<EntityId, SmallVec<[InternedString; 4]>>,
    entity: EntityId,
    relation: InternedString,
) {
    let mut rels = registry.entry(entity).or_default();
    if !rels.contains(&relation) {
        rels.push(relation);
    }
}

/// Remove `value` from the sorted list at `key`, dropping the key when the
/// list empties (keeps the maps from accumulating tombstone keys). Returns
/// `true` if `value` was actually present and removed — callers gate the
/// interner subject release on this so a no-op removal doesn't over-release.
fn remove_from_list(
    map: &DashMap<(EntityId, InternedString), EdgeList>,
    key: (EntityId, InternedString),
    value: EntityId,
) -> bool {
    let (removed, now_empty) = match map.get_mut(&key) {
        Some(mut list) => {
            let removed = if let Ok(pos) = list.binary_search(&value) {
                list.remove(pos);
                true
            } else {
                false
            };
            (removed, list.is_empty())
        }
        None => return false,
    };
    if now_empty {
        map.remove_if(&key, |_, list| list.is_empty());
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::interning::StringInterner;

    fn graph() -> (RelationshipGraph, StringInterner) {
        // Share one interner so the graph's subject releases evict from the
        // same table the test interns into (the store wires it identically).
        let interner = StringInterner::new();
        (RelationshipGraph::new(interner.clone()), interner)
    }

    #[test]
    fn direct_relation_and_reverse_index() {
        let (g, i) = graph();
        let (doc, alice, bob) = (i.intern("doc1"), i.intern("alice"), i.intern("bob"));
        let owner = i.intern("owner");
        g.add_edge(doc, owner, alice);

        assert!(g.has_relation(doc, owner, alice));
        assert!(!g.has_relation(doc, owner, bob));
        assert_eq!(g.related(doc, owner).as_slice(), &[alice]);
        assert_eq!(g.related_to(alice, owner).as_slice(), &[doc]);
    }

    #[test]
    fn add_edge_is_idempotent_and_sorted() {
        let (g, i) = graph();
        let doc = i.intern("doc");
        let rel = i.intern("viewer");
        let (a, b, c) = (i.intern("a"), i.intern("b"), i.intern("c"));
        for x in [c, a, b, a, c] {
            g.add_edge(doc, rel, x);
        }
        let edges = g.related(doc, rel);
        assert_eq!(edges.len(), 3);
        assert!(edges.windows(2).all(|w| w[0] < w[1]), "sorted, deduped");
    }

    #[test]
    fn len_and_is_empty_track_edge_lists() {
        let (g, i) = graph();
        assert!(g.is_empty());
        assert_eq!(g.len(), 0);
        g.add_edge(i.intern("doc"), i.intern("owner"), i.intern("alice"));
        assert!(!g.is_empty());
        assert_eq!(g.len(), 1);
        g.add_edge(i.intern("doc"), i.intern("viewer"), i.intern("bob"));
        assert_eq!(g.len(), 2, "one forward list per (node, relation) pair");
    }

    /// The 4,096-node traversal budget must be EXACT: node number
    /// BUDGET is still reachable, node BUDGET+1 never is. Written to kill
    /// the `>` -> `>=` / `==` mutants cargo-mutants found surviving on the
    /// budget comparison (an off-by-one here either shrinks the legal
    /// search space or unbounds the DoS guard).
    #[test]
    fn traversal_node_budget_is_exact() {
        let (g, i) = graph();
        let parent = i.intern("parent");

        // Chain c0 -> c1 -> ... -> c(B+1). BFS pops c(k-1) with
        // visited.len() == k, so hit(c_k) is checked iff k <= BUDGET.
        let n = TRAVERSAL_NODE_BUDGET + 2;
        let nodes: Vec<_> = (0..n).map(|k| i.intern(&format!("c{k}"))).collect();
        for w in nodes.windows(2) {
            g.add_edge(w[0], parent, w[1]);
        }

        let deep_enough = TRAVERSAL_NODE_BUDGET + 10;
        let last_inside = nodes[TRAVERSAL_NODE_BUDGET];
        let first_outside = nodes[TRAVERSAL_NODE_BUDGET + 1];

        assert!(
            g.bfs_reaches(nodes[0], parent, deep_enough, |x| x == last_inside),
            "node exactly AT the budget must still be reachable"
        );
        assert!(
            !g.bfs_reaches(nodes[0], parent, deep_enough, |x| x == first_outside),
            "node past the budget must be cut off (DoS guard)"
        );
    }

    #[test]
    fn reachable_through_group_chain_with_depth_bound() {
        let (g, i) = graph();
        let (user, team, org, doc) = (
            i.intern("alice"),
            i.intern("team-eng"),
            i.intern("org-acme"),
            i.intern("doc1"),
        );
        let member_of = i.intern("member_of");
        let viewer = i.intern("viewer");

        g.add_edge(user, member_of, team); // alice ∈ team
        g.add_edge(team, member_of, org); // team ∈ org
        g.add_edge(doc, viewer, org); // org can view doc

        assert!(g.has_relation_reachable(doc, viewer, user, member_of, 2));
        assert!(
            !g.has_relation_reachable(doc, viewer, user, member_of, 1),
            "needs 2 hops; depth bound must be respected"
        );
        assert!(!g.has_relation(doc, viewer, user), "not direct");
    }

    #[test]
    fn reachable_survives_cycles() {
        let (g, i) = graph();
        let (a, b, doc) = (i.intern("a"), i.intern("b"), i.intern("doc"));
        let m = i.intern("member_of");
        let v = i.intern("viewer");
        g.add_edge(a, m, b);
        g.add_edge(b, m, a); // cycle
        assert!(!g.has_relation_reachable(doc, v, a, m, 10));

        g.add_edge(doc, v, b);
        assert!(g.has_relation_reachable(doc, v, a, m, 10));
    }

    #[test]
    fn inherited_through_ancestor_folders() {
        let (g, i) = graph();
        let (doc, folder, root, alice) = (
            i.intern("doc1"),
            i.intern("folder-eng"),
            i.intern("folder-root"),
            i.intern("alice"),
        );
        let parent = i.intern("parent");
        let owner = i.intern("owner");

        g.add_edge(doc, parent, folder);
        g.add_edge(folder, parent, root);
        g.add_edge(root, owner, alice); // alice owns the root folder

        assert!(g.has_relation_inherited(doc, owner, alice, parent, 3));
        assert!(
            !g.has_relation_inherited(doc, owner, alice, parent, 1),
            "root is 2 levels up"
        );
    }

    // --- Subject refcount balance (white-box) --------------------------------
    //
    // These mirror the loader's contract exactly: a subject is `intern_counted`
    // once per occurrence in the source list BEFORE `add_edge`, and the graph
    // owns the balancing release in every removal path. Relations/carriers use
    // plain `intern` (pinned, bounded), so only subjects are evictable — which
    // is precisely what these assert.

    #[test]
    fn remove_edge_releases_the_subject_and_evicts_it() {
        let (g, i) = graph();
        let (doc, owner) = (i.intern("doc"), i.intern("owner"));
        let alice = i.intern_counted("alice"); // the loader's per-subject count
        g.add_edge(doc, owner, alice);
        assert!(i.lookup("alice").is_some());

        g.remove_edge(doc, owner, alice);
        assert!(
            i.lookup("alice").is_none(),
            "the edge's single counted subject reference was not released"
        );
        // Removing again must not over-release (no panic, still gone).
        g.remove_edge(doc, owner, alice);
        assert!(i.lookup("alice").is_none());
    }

    #[test]
    fn duplicate_subject_occurrence_is_released_on_add() {
        let (g, i) = graph();
        let (doc, owner) = (i.intern("doc"), i.intern("owner"));
        // Same subject counted twice (e.g. `owner: ["alice", "alice"]`): two
        // references in, but only one live edge, so one is dropped on the
        // duplicate add. A single removal must then fully reclaim it.
        let a1 = i.intern_counted("alice");
        let a2 = i.intern_counted("alice");
        assert_eq!(a1, a2);
        g.add_edge(doc, owner, a1); // new edge — keeps the count
        g.add_edge(doc, owner, a2); // duplicate — releases the redundant count
        assert!(i.lookup("alice").is_some(), "still one live edge");

        g.remove_edge(doc, owner, a1);
        assert!(
            i.lookup("alice").is_none(),
            "duplicate occurrence left a residual count (leak)"
        );
    }

    #[test]
    fn detach_carried_releases_every_subject_it_declared() {
        let (g, i) = graph();
        let doc = i.intern("doc");
        let (owner, viewer) = (i.intern("owner"), i.intern("viewer"));
        let alice = i.intern_counted("alice");
        let bob = i.intern_counted("bob");
        g.add_edge(doc, owner, alice);
        g.add_edge(doc, viewer, bob);

        g.detach_carried(doc);
        assert!(i.lookup("alice").is_none(), "carried owner subject leaked");
        assert!(i.lookup("bob").is_none(), "carried viewer subject leaked");
        // Reverse index is gone too — nothing points anywhere anymore.
        assert!(g.is_empty());
    }

    #[test]
    fn detach_releases_subject_counts_from_every_carrier_and_fails_closed() {
        let (g, i) = graph();
        let (doc1, doc2, owner) = (i.intern("doc1"), i.intern("doc2"), i.intern("owner"));
        // `alice` is the owner-subject of two documents: two live edges, two
        // counted references. Tombstoning alice must release both and drop
        // both dangling edges (fail closed: a deleted subject owns nothing).
        let a1 = i.intern_counted("alice");
        let a2 = i.intern_counted("alice");
        g.add_edge(doc1, owner, a1);
        g.add_edge(doc2, owner, a2);
        assert!(i.lookup("alice").is_some());

        g.detach(a1);
        assert!(
            i.lookup("alice").is_none(),
            "subject counted once per carrier — detach must release all of them"
        );
        assert!(
            g.related(doc1, owner).is_empty(),
            "doc1's edge must be gone"
        );
        assert!(
            g.related(doc2, owner).is_empty(),
            "doc2's edge must be gone"
        );
    }

    #[test]
    fn self_edge_subject_count_is_reclaimed_without_double_release() {
        // A self-referential edge (entity is both carrier and subject). The
        // subject is counted once; detach_carried removes the edge and releases
        // it, and the subject-side pass then finds the reverse entry already
        // pruned — so there is no second release to evict a live string out
        // from under anyone. (`release` is a no-op on an evicted id regardless,
        // but this proves the graph doesn't rely on that.)
        let (g, i) = graph();
        let rel = i.intern("linked");
        let node = i.intern_counted("node"); // counted as its own subject
        g.add_edge(node, rel, node);
        assert!(i.lookup("node").is_some());

        g.detach(node);
        assert!(
            i.lookup("node").is_none(),
            "self-edge subject count not reclaimed"
        );
        assert!(g.is_empty());
    }
}
