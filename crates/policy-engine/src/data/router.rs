//! Query Router for Intelligent Query Execution
//!
//! Phase 6A-2: Query Router
//!
//! The QueryRouter provides intelligent routing of queries to the optimal execution
//! strategy based on available materialized views and indexes. It implements a
//! performance tier system that automatically selects the fastest available method.
//!
//! Performance Tiers:
//! - Tier 1: Pre-computed views (100-500ns)
//! - Tier 2: Indexed joins (1-3µs)
//! - Tier 3: Partial scan (3-5µs)
//! - Tier 4: Full scan (5-10µs)

use super::entity::{AttributeValue, Entity};
use super::store::DataStore;
use reaper_core::ReaperError;
use std::sync::Arc;

/// Query patterns supported by the router
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryPattern {
    /// RBAC: Check if user has permission on resource
    ///
    /// Performance:
    /// - With view: 100-500ns
    /// - With indexes: 1-3µs
    /// - Full scan: 5-10µs
    PermissionCheck {
        user: String,
        resource: String,
        action: String,
    },

    /// RBAC: Get all roles for a user
    ///
    /// Performance:
    /// - With view: 100-500ns
    /// - With indexes: 1-2µs
    /// - Full scan: 3-5µs
    UserRoles { user: String },

    /// RBAC: Get all users with a role
    ///
    /// Performance:
    /// - With view: 100-500ns
    /// - With indexes: 1-2µs
    /// - Full scan: 3-5µs
    RoleMembers { role: String },

    /// RBAC: Get all permissions for a resource
    ///
    /// Performance:
    /// - With view: 100-500ns
    /// - With indexes: 2-3µs
    /// - Full scan: 5-10µs
    ResourcePermissions { resource: String },

    /// General: Query by entity type and attributes
    ///
    /// Performance:
    /// - With indexes: 100-300ns per attribute
    /// - Full scan: 1-10µs
    TypeAndAttributes {
        entity_type: String,
        attributes: Vec<(String, String)>,
    },
}

/// Performance tier for query execution
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PerformanceTier {
    /// 100-500ns - Pre-computed materialized view
    Tier1PreComputed,

    /// 1-3µs - Indexed join on multiple tables
    Tier2IndexedJoin,

    /// 3-5µs - Partial scan with some indexed attributes
    Tier3PartialScan,

    /// 5-10µs - Full scan with filtering
    Tier4FullScan,
}

impl PerformanceTier {
    /// Get the typical time range for this tier
    pub fn time_range_ns(&self) -> (u64, u64) {
        match self {
            PerformanceTier::Tier1PreComputed => (100, 500),
            PerformanceTier::Tier2IndexedJoin => (1_000, 3_000),
            PerformanceTier::Tier3PartialScan => (3_000, 5_000),
            PerformanceTier::Tier4FullScan => (5_000, 10_000),
        }
    }

    /// Get a human-readable description
    pub fn description(&self) -> &'static str {
        match self {
            PerformanceTier::Tier1PreComputed => "Pre-computed view (100-500ns)",
            PerformanceTier::Tier2IndexedJoin => "Indexed join (1-3µs)",
            PerformanceTier::Tier3PartialScan => "Partial scan (3-5µs)",
            PerformanceTier::Tier4FullScan => "Full scan (5-10µs)",
        }
    }
}

/// Query execution result with performance metadata
#[derive(Debug, Clone)]
pub struct QueryResult {
    /// The entities that matched the query
    pub entities: Vec<Arc<Entity>>,

    /// The performance tier used for this query
    pub tier: PerformanceTier,

    /// View name if a view was used
    pub view_used: Option<String>,

    /// Whether the result came from a stale view
    pub is_stale: bool,
}

impl QueryResult {
    /// Create a new query result
    pub fn new(entities: Vec<Arc<Entity>>, tier: PerformanceTier) -> Self {
        Self {
            entities,
            tier,
            view_used: None,
            is_stale: false,
        }
    }

    /// Create a result from a view
    pub fn from_view(entities: Vec<Arc<Entity>>, view_name: String, is_stale: bool) -> Self {
        Self {
            entities,
            tier: PerformanceTier::Tier1PreComputed,
            view_used: Some(view_name),
            is_stale,
        }
    }
}

/// Query router for intelligent query execution
///
/// The router analyzes query patterns and automatically selects the optimal
/// execution strategy based on available views, indexes, and data characteristics.
pub struct QueryRouter {
    /// Reference to the data store
    store: Arc<DataStore>,
}

impl QueryRouter {
    /// Create a new query router
    pub fn new(store: Arc<DataStore>) -> Self {
        Self { store }
    }

    /// Execute a query with automatic routing
    ///
    /// The router will:
    /// 1. Detect the query pattern
    /// 2. Check for applicable materialized views
    /// 3. Fall back to indexed queries if needed
    /// 4. Use full scan as last resort
    ///
    /// # Example
    /// ```text
    /// let result = router.execute(QueryPattern::PermissionCheck {
    ///     user: "alice".to_string(),
    ///     resource: "foo123".to_string(),
    ///     action: "write".to_string(),
    /// })?;
    ///
    /// println!("Found {} results using {}", result.entities.len(), result.tier.description());
    /// ```
    pub fn execute(&self, pattern: QueryPattern) -> Result<QueryResult, ReaperError> {
        match pattern {
            QueryPattern::PermissionCheck {
                user,
                resource,
                action,
            } => self.execute_permission_check(&user, &resource, &action),

            QueryPattern::UserRoles { user } => self.execute_user_roles(&user),

            QueryPattern::RoleMembers { role } => self.execute_role_members(&role),

            QueryPattern::ResourcePermissions { resource } => {
                self.execute_resource_permissions(&resource)
            }

            QueryPattern::TypeAndAttributes {
                entity_type,
                attributes,
            } => self.execute_type_and_attributes(&entity_type, &attributes),
        }
    }

    /// Execute a permission check query
    fn execute_permission_check(
        &self,
        user: &str,
        resource: &str,
        action: &str,
    ) -> Result<QueryResult, ReaperError> {
        // Tier 1: Try pre-computed view with composite index (Phase 6C)
        if let Some(view) = self.store.get_view("user_permission") {
            let interner = self.store.interner();

            // Phase 6C: Use composite index for O(1) direct lookup
            // This reduces cold query latency from ~24µs to ~3-5µs
            if view.has_composite_index("user_resource_action") {
                let user_value = AttributeValue::String(interner.intern(user));
                let resource_value = AttributeValue::String(interner.intern(resource));
                let action_value = AttributeValue::String(interner.intern(action));

                let entities = view.get_by_composite(
                    "user_resource_action",
                    &[user_value, resource_value, action_value],
                );

                // Return result if view is fresh or has results
                if !entities.is_empty() || !view.is_stale {
                    return Ok(QueryResult::from_view(
                        entities,
                        "user_permission".to_string(),
                        view.is_stale,
                    ));
                }
            } else {
                // Phase 6A-4: Fallback to sequential filtering if no composite index
                let user_key = interner.intern("user");
                let resource_key = interner.intern("resource");
                let action_key = interner.intern("action");

                let user_value = AttributeValue::String(interner.intern(user));
                let resource_value = AttributeValue::String(interner.intern(resource));
                let action_value = AttributeValue::String(interner.intern(action));

                let entities = view.get_by_attributes(vec![
                    (user_key, &user_value),
                    (resource_key, &resource_value),
                    (action_key, &action_value),
                ]);

                // Return result if view is fresh or has results
                if !entities.is_empty() || !view.is_stale {
                    return Ok(QueryResult::from_view(
                        entities,
                        "user_permission".to_string(),
                        view.is_stale,
                    ));
                }
            }
        }

        // Tier 2: Try indexed join (user_role_binding + role_permission)
        if let Ok(entities) = self.execute_rbac_join(user, resource, action) {
            if !entities.is_empty() {
                return Ok(QueryResult::new(
                    entities,
                    PerformanceTier::Tier2IndexedJoin,
                ));
            }
        }

        // Tier 4: Full scan as fallback
        let entities = self.execute_permission_full_scan(user, resource, action)?;
        Ok(QueryResult::new(entities, PerformanceTier::Tier4FullScan))
    }

    /// Execute a user roles query
    fn execute_user_roles(&self, user: &str) -> Result<QueryResult, ReaperError> {
        // Tier 1: Try role_users view (inverse lookup)
        if let Some(_view) = self.store.get_view("role_users") {
            // For now, fall through to indexed query
            // Phase 6A-3 will implement proper view lookup
        }

        // Tier 2: Indexed query on user_role_binding
        let interner = self.store.interner();
        let binding_type = interner.intern("user_role_binding");
        let user_key = interner.intern("user");
        let user_value = interner.intern(user);

        let entities = self
            .store
            .get_by_type_and_attribute(binding_type, user_key, user_value);

        Ok(QueryResult::new(
            entities,
            PerformanceTier::Tier2IndexedJoin,
        ))
    }

    /// Execute a role members query
    fn execute_role_members(&self, role: &str) -> Result<QueryResult, ReaperError> {
        // Tier 1: Try role_users view with indexed lookup (Phase 6A-4)
        if let Some(view) = self.store.get_view("role_users") {
            let interner = self.store.interner();
            let role_key = interner.intern("role");
            let role_value = AttributeValue::String(interner.intern(role));

            // Use indexed lookup for O(1) performance
            let entities = view.get_by_attributes(vec![(role_key, &role_value)]);

            if !entities.is_empty() || !view.is_stale {
                return Ok(QueryResult::from_view(
                    entities,
                    "role_users".to_string(),
                    view.is_stale,
                ));
            }
        }

        // Tier 2: Indexed query on user_role_binding
        let interner = self.store.interner();
        let binding_type = interner.intern("user_role_binding");
        let role_key = interner.intern("role");
        let role_value = interner.intern(role);

        let entities = self
            .store
            .get_by_type_and_attribute(binding_type, role_key, role_value);

        Ok(QueryResult::new(
            entities,
            PerformanceTier::Tier2IndexedJoin,
        ))
    }

    /// Execute a resource permissions query
    fn execute_resource_permissions(&self, resource: &str) -> Result<QueryResult, ReaperError> {
        // Tier 1: Try resource_permissions view with indexed lookup (Phase 6A-4)
        if let Some(view) = self.store.get_view("resource_permissions") {
            let interner = self.store.interner();
            let resource_key = interner.intern("resource");
            let resource_value = AttributeValue::String(interner.intern(resource));

            // Use indexed lookup for O(1) performance
            let entities = view.get_by_attributes(vec![(resource_key, &resource_value)]);

            if !entities.is_empty() || !view.is_stale {
                return Ok(QueryResult::from_view(
                    entities,
                    "resource_permissions".to_string(),
                    view.is_stale,
                ));
            }
        }

        // Tier 2: Indexed query on role_permission
        let interner = self.store.interner();
        let perm_type = interner.intern("role_permission");
        let resource_key = interner.intern("resource");
        let resource_value = interner.intern(resource);

        let entities =
            self.store
                .get_by_type_and_attribute(perm_type, resource_key, resource_value);

        Ok(QueryResult::new(
            entities,
            PerformanceTier::Tier2IndexedJoin,
        ))
    }

    /// Execute a type and attributes query
    fn execute_type_and_attributes(
        &self,
        entity_type: &str,
        attributes: &[(String, String)],
    ) -> Result<QueryResult, ReaperError> {
        let interner = self.store.interner();
        let type_id = interner.intern(entity_type);

        if attributes.is_empty() {
            // No attributes - just get by type
            let entities = self.store.get_by_type(type_id);
            return Ok(QueryResult::new(
                entities,
                PerformanceTier::Tier2IndexedJoin,
            ));
        }

        // Use composite index for first attribute
        let (first_key, first_value) = &attributes[0];
        let key_id = interner.intern(first_key);
        let value_id = interner.intern(first_value);

        let mut entities = self
            .store
            .get_by_type_and_attribute(type_id, key_id, value_id);

        // Filter by remaining attributes
        for (key, value) in attributes.iter().skip(1) {
            let key_id = interner.intern(key);
            let value_id = interner.intern(value);

            entities.retain(|entity| {
                entity
                    .attributes
                    .get(&key_id)
                    .and_then(|v| match v {
                        AttributeValue::String(s) => Some(*s == value_id),
                        _ => None,
                    })
                    .unwrap_or(false)
            });
        }

        Ok(QueryResult::new(
            entities,
            if attributes.len() == 1 {
                PerformanceTier::Tier2IndexedJoin
            } else {
                PerformanceTier::Tier3PartialScan
            },
        ))
    }

    // ========================================================================
    // Helper Methods
    // ========================================================================

    /// Match permission attributes in an entity
    fn match_permission_attributes(
        &self,
        entity: &Entity,
        user: &str,
        resource: &str,
        action: &str,
    ) -> bool {
        let interner = self.store.interner();

        let user_match = entity
            .get_attribute_str("user", interner)
            .map(|u| u == user)
            .unwrap_or(false);

        let resource_match = entity
            .get_attribute_str("resource", interner)
            .map(|r| r == resource)
            .unwrap_or(false);

        let action_match = entity
            .get_attribute_str("action", interner)
            .map(|a| a == action)
            .unwrap_or(false);

        user_match && resource_match && action_match
    }

    /// Execute RBAC join (user_role_binding + role_permission)
    fn execute_rbac_join(
        &self,
        user: &str,
        resource: &str,
        action: &str,
    ) -> Result<Vec<Arc<Entity>>, ReaperError> {
        let interner = self.store.interner();

        // Step 1: Get user's roles
        let binding_type = interner.intern("user_role_binding");
        let user_key = interner.intern("user");
        let user_value = interner.intern(user);

        let user_bindings =
            self.store
                .get_by_type_and_attribute(binding_type, user_key, user_value);

        // Step 2: For each role, check permissions
        let perm_type = interner.intern("role_permission");
        let role_key = interner.intern("role");
        let resource_key = interner.intern("resource");
        let action_key = interner.intern("action");

        let mut results = Vec::new();

        for binding in user_bindings {
            if let Some(AttributeValue::String(role_value)) = binding.attributes.get(&role_key) {
                // Get permissions for this role
                let perms = self
                    .store
                    .get_by_type_and_attribute(perm_type, role_key, *role_value);

                // Check if any permission matches
                for perm in perms {
                    let perm_resource = perm
                        .attributes
                        .get(&resource_key)
                        .and_then(|v| v.as_string(interner));

                    let perm_action = perm
                        .attributes
                        .get(&action_key)
                        .and_then(|v| v.as_string(interner));

                    let resource_match =
                        perm_resource.as_ref().map(|s| s.as_ref()) == Some(resource);
                    let action_match = perm_action.as_ref().map(|s| s.as_ref()) == Some(action);

                    if resource_match && action_match {
                        results.push(perm);
                    }
                }
            }
        }

        Ok(results)
    }

    /// Execute permission check with full scan
    fn execute_permission_full_scan(
        &self,
        user: &str,
        resource: &str,
        action: &str,
    ) -> Result<Vec<Arc<Entity>>, ReaperError> {
        let all_entities = self.store.all();

        let results = all_entities
            .into_iter()
            .filter(|entity| self.match_permission_attributes(entity, user, resource, action))
            .collect();

        Ok(results)
    }

    /// Get statistics about available views and indexes
    pub fn stats(&self) -> RouterStats {
        let view_count = self.store.list_views().len();
        let view_stats = self.store.view_stats();

        let stale_views = view_stats.iter().filter(|v| v.is_stale).count();

        RouterStats {
            total_views: view_count,
            stale_views,
            tier1_available: view_count > 0,
            tier2_available: true, // Always have indexes
        }
    }
}

/// Statistics about the query router
#[derive(Debug, Clone)]
pub struct RouterStats {
    pub total_views: usize,
    pub stale_views: usize,
    pub tier1_available: bool,
    pub tier2_available: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::entity::EntityBuilder;
    use crate::data::views::{MaterializedView, ViewQuery, ViewStrategy};
    use crate::data::DataStore;

    #[test]
    fn test_performance_tiers() {
        assert!(PerformanceTier::Tier1PreComputed < PerformanceTier::Tier2IndexedJoin);
        assert!(PerformanceTier::Tier2IndexedJoin < PerformanceTier::Tier3PartialScan);
        assert!(PerformanceTier::Tier3PartialScan < PerformanceTier::Tier4FullScan);

        let (min, max) = PerformanceTier::Tier1PreComputed.time_range_ns();
        assert_eq!(min, 100);
        assert_eq!(max, 500);
    }

    #[test]
    fn test_query_result_creation() {
        let result = QueryResult::new(vec![], PerformanceTier::Tier2IndexedJoin);
        assert_eq!(result.tier, PerformanceTier::Tier2IndexedJoin);
        assert!(result.view_used.is_none());
        assert!(!result.is_stale);

        let result2 = QueryResult::from_view(vec![], "test_view".to_string(), false);
        assert_eq!(result2.tier, PerformanceTier::Tier1PreComputed);
        assert_eq!(result2.view_used, Some("test_view".to_string()));
    }

    #[test]
    fn test_router_creation() {
        let store = DataStore::new();
        let router = QueryRouter::new(Arc::new(store));
        let stats = router.stats();

        assert_eq!(stats.total_views, 0);
        assert!(stats.tier2_available);
    }

    #[test]
    fn test_user_roles_query() {
        let store = DataStore::new();
        let interner = store.interner();

        // Create user-role binding
        let binding_type = interner.intern("user_role_binding");
        let user_key = interner.intern("user");
        let role_key = interner.intern("role");
        let alice = interner.intern("alice");
        let dev_role = interner.intern("dev");

        let binding = EntityBuilder::new(interner.intern("alice_dev"), binding_type)
            .with_string(user_key, alice)
            .with_string(role_key, dev_role)
            .build();

        store.insert(binding);

        // Query via router
        let router = QueryRouter::new(Arc::new(store));
        let result = router
            .execute(QueryPattern::UserRoles {
                user: "alice".to_string(),
            })
            .unwrap();

        assert_eq!(result.entities.len(), 1);
        assert_eq!(result.tier, PerformanceTier::Tier2IndexedJoin);
    }

    #[test]
    fn test_type_and_attributes_query() {
        let store = DataStore::new();
        let interner = store.interner();

        let user_type = interner.intern("User");
        let role_key = interner.intern("role");
        let dept_key = interner.intern("department");
        let admin = interner.intern("admin");
        let eng = interner.intern("engineering");

        // Create entities
        let alice = EntityBuilder::new(interner.intern("alice"), user_type)
            .with_string(role_key, admin)
            .with_string(dept_key, eng)
            .build();

        let bob = EntityBuilder::new(interner.intern("bob"), user_type)
            .with_string(role_key, admin)
            .with_string(dept_key, interner.intern("sales"))
            .build();

        store.insert(alice);
        store.insert(bob);

        // Query for admin users in engineering
        let router = QueryRouter::new(Arc::new(store));
        let result = router
            .execute(QueryPattern::TypeAndAttributes {
                entity_type: "User".to_string(),
                attributes: vec![
                    ("role".to_string(), "admin".to_string()),
                    ("department".to_string(), "engineering".to_string()),
                ],
            })
            .unwrap();

        assert_eq!(result.entities.len(), 1);
        assert_eq!(result.tier, PerformanceTier::Tier3PartialScan);
    }

    #[test]
    fn test_permission_check_with_view() {
        let store = DataStore::new();
        let interner = store.interner();

        // Create a pre-computed permission view
        let mut view = MaterializedView::new(
            "user_permission".to_string(),
            ViewQuery::UserPermission {
                binding_type: "user_role_binding".to_string(),
                permission_type: "role_permission".to_string(),
                join_key: "role".to_string(),
            },
            ViewStrategy::Eager,
        );

        // Add permission entity to view
        let perm_type = interner.intern("user_permission");
        let user_key = interner.intern("user");
        let resource_key = interner.intern("resource");
        let action_key = interner.intern("action");

        let perm = EntityBuilder::new(interner.intern("alice_write_foo123"), perm_type)
            .with_string(user_key, interner.intern("alice"))
            .with_string(resource_key, interner.intern("foo123"))
            .with_string(action_key, interner.intern("write"))
            .build();

        view.insert("alice_write_foo123".to_string(), Arc::new(perm));
        view.mark_fresh();

        store.add_view(view).unwrap();

        // Query via router
        let router = QueryRouter::new(Arc::new(store));
        let result = router
            .execute(QueryPattern::PermissionCheck {
                user: "alice".to_string(),
                resource: "foo123".to_string(),
                action: "write".to_string(),
            })
            .unwrap();

        assert_eq!(result.entities.len(), 1);
        assert_eq!(result.tier, PerformanceTier::Tier1PreComputed);
        assert_eq!(result.view_used, Some("user_permission".to_string()));
        assert!(!result.is_stale);
    }
}
