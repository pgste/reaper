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

use crate::data::interning::InternedString;
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
#[derive(Debug, Default)]
pub struct RelationshipGraph {
    /// (declaring entity, relation) -> subjects, sorted.
    forward: DashMap<(EntityId, InternedString), EdgeList>,
    /// (subject, relation) -> declaring entities, sorted.
    reverse: DashMap<(EntityId, InternedString), EdgeList>,
}

impl RelationshipGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record `from #relation @to` (idempotent; lists stay sorted).
    pub fn add_edge(&self, from: EntityId, relation: InternedString, to: EntityId) {
        insert_sorted(
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

    /// Total number of forward edge lists (diagnostics).
    pub fn len(&self) -> usize {
        self.forward.len()
    }

    pub fn is_empty(&self) -> bool {
        self.forward.is_empty()
    }
}

fn insert_sorted(list: &mut EdgeList, value: EntityId) {
    if let Err(pos) = list.binary_search(&value) {
        list.insert(pos, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::interning::StringInterner;

    fn graph() -> (RelationshipGraph, StringInterner) {
        (RelationshipGraph::new(), StringInterner::new())
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
}
