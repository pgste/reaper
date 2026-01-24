//! Namespace domain model
//!
//! Provides hierarchical namespace isolation within organizations.
//! Namespaces allow scoped policies, deployments, and event filtering.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Namespace entity
///
/// Namespaces provide hierarchical isolation within an organization.
/// Policies and bundles can be scoped to namespaces, and agents
/// subscribe to specific namespaces to receive targeted events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Namespace {
    pub id: Uuid,
    pub org_id: Uuid,
    /// URL-safe slug (e.g., "production", "production/us-east")
    pub slug: String,
    /// Human-readable display name
    pub display_name: Option<String>,
    /// Optional parent namespace for hierarchy
    pub parent_id: Option<Uuid>,
    /// Optional description
    pub description: Option<String>,
    /// Namespace-specific settings (JSON)
    pub settings: serde_json::Value,
    /// Whether this namespace is active
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Namespace {
    /// Check if this namespace is a root namespace (no parent)
    pub fn is_root(&self) -> bool {
        self.parent_id.is_none()
    }

    /// Get the namespace depth (0 for root, 1 for first child, etc.)
    pub fn depth(&self) -> usize {
        self.slug.matches('/').count()
    }

    /// Get the immediate parent slug (if any)
    pub fn parent_slug(&self) -> Option<&str> {
        self.slug.rsplit_once('/').map(|(parent, _)| parent)
    }

    /// Get the leaf name (last segment of slug)
    pub fn leaf_name(&self) -> &str {
        self.slug.rsplit_once('/').map(|(_, leaf)| leaf).unwrap_or(&self.slug)
    }

    /// Check if this namespace is an ancestor of another
    pub fn is_ancestor_of(&self, other: &Namespace) -> bool {
        other.slug.starts_with(&format!("{}/", self.slug))
    }

    /// Check if this namespace is a descendant of another
    pub fn is_descendant_of(&self, other: &Namespace) -> bool {
        self.slug.starts_with(&format!("{}/", other.slug))
    }
}

/// Input for creating a namespace
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateNamespace {
    /// URL-safe slug
    pub slug: String,
    /// Human-readable display name
    pub display_name: Option<String>,
    /// Optional parent namespace ID
    pub parent_id: Option<Uuid>,
    /// Optional description
    pub description: Option<String>,
    /// Namespace-specific settings
    #[serde(default)]
    pub settings: serde_json::Value,
}

/// Input for updating a namespace
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateNamespace {
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub settings: Option<serde_json::Value>,
    pub is_active: Option<bool>,
}

/// Agent subscription to a namespace
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSubscription {
    pub agent_id: Uuid,
    pub namespace_id: Uuid,
    /// Whether to include events from child namespaces
    pub include_children: bool,
    pub created_at: DateTime<Utc>,
}

/// Input for creating an agent subscription
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateAgentSubscription {
    pub namespace_id: Uuid,
    #[serde(default = "default_include_children")]
    pub include_children: bool,
}

fn default_include_children() -> bool {
    true
}

/// Namespace tree node for hierarchical display
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamespaceTree {
    pub namespace: Namespace,
    pub children: Vec<NamespaceTree>,
}

impl NamespaceTree {
    /// Create a new tree node
    pub fn new(namespace: Namespace) -> Self {
        Self {
            namespace,
            children: Vec::new(),
        }
    }

    /// Count total nodes in the tree
    pub fn count(&self) -> usize {
        1 + self.children.iter().map(|c| c.count()).sum::<usize>()
    }

    /// Find a namespace by ID in the tree
    pub fn find(&self, id: Uuid) -> Option<&Namespace> {
        if self.namespace.id == id {
            return Some(&self.namespace);
        }
        for child in &self.children {
            if let Some(ns) = child.find(id) {
                return Some(ns);
            }
        }
        None
    }
}

/// Build a tree from a flat list of namespaces
/// Resolve a namespace by ID (UUID) or slug
///
/// This helper function looks up a namespace by trying to parse the input
/// as a UUID first, then falling back to slug lookup.
pub async fn resolve_namespace(
    db: &crate::db::Database,
    org_id: Uuid,
    namespace: &str,
) -> Result<Uuid, String> {
    use crate::db::repositories::NamespaceRepository;

    let ns_repo = NamespaceRepository::new(db);

    // Try parsing as UUID first, then as slug
    let ns = if let Ok(id) = Uuid::parse_str(namespace) {
        ns_repo
            .get_by_id(id)
            .await
            .map_err(|e| e.to_string())?
    } else {
        ns_repo
            .get_by_slug(org_id, namespace)
            .await
            .map_err(|e| e.to_string())?
    };

    ns.map(|n| n.id)
        .ok_or_else(|| format!("Namespace not found: {}", namespace))
}

pub fn build_namespace_tree(namespaces: Vec<Namespace>) -> Vec<NamespaceTree> {
    use std::collections::HashMap;

    let mut nodes: HashMap<Uuid, NamespaceTree> = namespaces
        .into_iter()
        .map(|ns| (ns.id, NamespaceTree::new(ns)))
        .collect();

    let mut roots = Vec::new();

    // Build parent-child relationships
    let ids: Vec<Uuid> = nodes.keys().cloned().collect();
    for id in ids {
        let parent_id = nodes.get(&id).and_then(|n| n.namespace.parent_id);

        if let Some(parent_id) = parent_id {
            if let Some(node) = nodes.remove(&id) {
                if let Some(parent) = nodes.get_mut(&parent_id) {
                    parent.children.push(node);
                } else {
                    // Parent not found, treat as root
                    nodes.insert(id, node);
                }
            }
        }
    }

    // Remaining nodes are roots
    roots.extend(nodes.into_values());

    // Sort roots and children by slug
    roots.sort_by(|a, b| a.namespace.slug.cmp(&b.namespace.slug));
    for root in &mut roots {
        sort_tree_children(root);
    }

    roots
}

fn sort_tree_children(node: &mut NamespaceTree) {
    node.children.sort_by(|a, b| a.namespace.slug.cmp(&b.namespace.slug));
    for child in &mut node.children {
        sort_tree_children(child);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_namespace(slug: &str, parent_id: Option<Uuid>) -> Namespace {
        Namespace {
            id: Uuid::new_v4(),
            org_id: Uuid::new_v4(),
            slug: slug.to_string(),
            display_name: Some(slug.to_string()),
            parent_id,
            description: None,
            settings: serde_json::json!({}),
            is_active: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn test_namespace_depth() {
        let root = make_namespace("production", None);
        assert_eq!(root.depth(), 0);

        let child = make_namespace("production/us-east", None);
        assert_eq!(child.depth(), 1);

        let grandchild = make_namespace("production/us-east/zone-1", None);
        assert_eq!(grandchild.depth(), 2);
    }

    #[test]
    fn test_namespace_leaf_name() {
        let root = make_namespace("production", None);
        assert_eq!(root.leaf_name(), "production");

        let child = make_namespace("production/us-east", None);
        assert_eq!(child.leaf_name(), "us-east");
    }

    #[test]
    fn test_namespace_parent_slug() {
        let root = make_namespace("production", None);
        assert_eq!(root.parent_slug(), None);

        let child = make_namespace("production/us-east", None);
        assert_eq!(child.parent_slug(), Some("production"));
    }

    #[test]
    fn test_namespace_ancestor_descendant() {
        let parent = make_namespace("production", None);
        let child = make_namespace("production/us-east", Some(parent.id));

        assert!(parent.is_ancestor_of(&child));
        assert!(!child.is_ancestor_of(&parent));
        assert!(child.is_descendant_of(&parent));
        assert!(!parent.is_descendant_of(&child));
    }

    #[test]
    fn test_build_namespace_tree() {
        let org_id = Uuid::new_v4();
        let root_id = Uuid::new_v4();
        let child_id = Uuid::new_v4();

        let namespaces = vec![
            Namespace {
                id: root_id,
                org_id,
                slug: "production".to_string(),
                display_name: Some("Production".to_string()),
                parent_id: None,
                description: None,
                settings: serde_json::json!({}),
                is_active: true,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            Namespace {
                id: child_id,
                org_id,
                slug: "production/us-east".to_string(),
                display_name: Some("US East".to_string()),
                parent_id: Some(root_id),
                description: None,
                settings: serde_json::json!({}),
                is_active: true,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
        ];

        let tree = build_namespace_tree(namespaces);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].namespace.slug, "production");
        assert_eq!(tree[0].children.len(), 1);
        assert_eq!(tree[0].children[0].namespace.slug, "production/us-east");
    }
}
