//! Namespace repository for database operations
//!
//! Handles persistence of namespace records and agent subscriptions.

use chrono::Utc;
use sqlx::Row;
use uuid::Uuid;

use crate::db::{Database, DatabaseError};
use crate::domain::namespace::{
    AgentSubscription, CreateAgentSubscription, CreateNamespace, Namespace, UpdateNamespace,
};

/// Repository for namespace operations
pub struct NamespaceRepository<'a> {
    db: &'a Database,
}

impl<'a> NamespaceRepository<'a> {
    /// Create a new namespace repository
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// Create a new namespace
    pub async fn create(
        &self,
        org_id: Uuid,
        input: CreateNamespace,
    ) -> Result<Namespace, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let id = Uuid::new_v4();
        let now = Utc::now();
        let settings_json =
            serde_json::to_string(&input.settings).unwrap_or_else(|_| "{}".to_string());

        sqlx::query(
            r#"
            INSERT INTO namespaces (id, org_id, slug, display_name, parent_id, description, settings, is_active, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            "#,
        )
        .bind(id.to_string())
        .bind(org_id.to_string())
        .bind(&input.slug)
        .bind(&input.display_name)
        .bind(input.parent_id.map(|p| p.to_string()))
        .bind(&input.description)
        .bind(&settings_json)
        .bind(true)
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .execute(pool)
        .await?;

        Ok(Namespace {
            id,
            org_id,
            slug: input.slug,
            display_name: input.display_name,
            parent_id: input.parent_id,
            description: input.description,
            settings: input.settings,
            is_active: true,
            created_at: now,
            updated_at: now,
        })
    }

    /// Get namespace by ID
    pub async fn get_by_id(&self, id: Uuid) -> Result<Option<Namespace>, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let row = sqlx::query(
            r#"
            SELECT id, org_id, slug, display_name, parent_id, description, settings, is_active, created_at, updated_at
            FROM namespaces
            WHERE id = $1
            "#,
        )
        .bind(id.to_string())
        .fetch_optional(pool)
        .await?;

        match row {
            Some(row) => Ok(Some(self.row_to_namespace(row)?)),
            None => Ok(None),
        }
    }

    /// Get namespace by slug within an organization
    pub async fn get_by_slug(
        &self,
        org_id: Uuid,
        slug: &str,
    ) -> Result<Option<Namespace>, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let row = sqlx::query(
            r#"
            SELECT id, org_id, slug, display_name, parent_id, description, settings, is_active, created_at, updated_at
            FROM namespaces
            WHERE org_id = $1 AND slug = $2
            "#,
        )
        .bind(org_id.to_string())
        .bind(slug)
        .fetch_optional(pool)
        .await?;

        match row {
            Some(row) => Ok(Some(self.row_to_namespace(row)?)),
            None => Ok(None),
        }
    }

    /// List all namespaces for an organization
    pub async fn list_by_org(&self, org_id: Uuid) -> Result<Vec<Namespace>, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let rows = sqlx::query(
            r#"
            SELECT id, org_id, slug, display_name, parent_id, description, settings, is_active, created_at, updated_at
            FROM namespaces
            WHERE org_id = $1
            ORDER BY slug ASC
            "#,
        )
        .bind(org_id.to_string())
        .fetch_all(pool)
        .await?;

        let mut namespaces = Vec::with_capacity(rows.len());
        for row in rows {
            namespaces.push(self.row_to_namespace(row)?);
        }

        Ok(namespaces)
    }

    /// List root namespaces (no parent) for an organization
    pub async fn list_roots(&self, org_id: Uuid) -> Result<Vec<Namespace>, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let rows = sqlx::query(
            r#"
            SELECT id, org_id, slug, display_name, parent_id, description, settings, is_active, created_at, updated_at
            FROM namespaces
            WHERE org_id = $1 AND parent_id IS NULL
            ORDER BY slug ASC
            "#,
        )
        .bind(org_id.to_string())
        .fetch_all(pool)
        .await?;

        let mut namespaces = Vec::with_capacity(rows.len());
        for row in rows {
            namespaces.push(self.row_to_namespace(row)?);
        }

        Ok(namespaces)
    }

    /// List children of a namespace
    pub async fn list_children(&self, parent_id: Uuid) -> Result<Vec<Namespace>, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let rows = sqlx::query(
            r#"
            SELECT id, org_id, slug, display_name, parent_id, description, settings, is_active, created_at, updated_at
            FROM namespaces
            WHERE parent_id = $1
            ORDER BY slug ASC
            "#,
        )
        .bind(parent_id.to_string())
        .fetch_all(pool)
        .await?;

        let mut namespaces = Vec::with_capacity(rows.len());
        for row in rows {
            namespaces.push(self.row_to_namespace(row)?);
        }

        Ok(namespaces)
    }

    /// Update a namespace
    pub async fn update(&self, id: Uuid, input: UpdateNamespace) -> Result<bool, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let mut updates = Vec::new();
        let mut bindings: Vec<String> = Vec::new();

        if let Some(display_name) = &input.display_name {
            updates.push("display_name = ?");
            bindings.push(display_name.clone());
        }

        if let Some(description) = &input.description {
            updates.push("description = ?");
            bindings.push(description.clone());
        }

        if let Some(settings) = &input.settings {
            updates.push("settings = ?");
            bindings.push(serde_json::to_string(settings).unwrap_or_else(|_| "{}".to_string()));
        }

        if let Some(is_active) = input.is_active {
            updates.push("is_active = ?");
            bindings.push(if is_active {
                "1".to_string()
            } else {
                "0".to_string()
            });
        }

        if updates.is_empty() {
            return Ok(false);
        }

        updates.push("updated_at = ?");
        bindings.push(Utc::now().to_rfc3339());

        let sql = crate::db::numbered_placeholders(&format!(
            "UPDATE namespaces SET {} WHERE id = ?",
            updates.join(", ")
        ));

        let mut query = sqlx::query(&sql);
        for binding in &bindings {
            query = query.bind(binding);
        }
        query = query.bind(id.to_string());

        let result = query.execute(pool).await?;
        Ok(result.rows_affected() > 0)
    }

    /// Delete a namespace (and cascade to children)
    pub async fn delete(&self, id: Uuid) -> Result<bool, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        // Delete child namespaces first (recursive via ON DELETE CASCADE or manually)
        // For safety, we'll delete subscriptions first
        sqlx::query("DELETE FROM agent_subscriptions WHERE namespace_id = $1")
            .bind(id.to_string())
            .execute(pool)
            .await?;

        let result = sqlx::query("DELETE FROM namespaces WHERE id = $1")
            .bind(id.to_string())
            .execute(pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Convert database row to Namespace
    fn row_to_namespace(&self, row: sqlx::sqlite::SqliteRow) -> Result<Namespace, DatabaseError> {
        let id_str: String = row.get("id");
        let id = Uuid::parse_str(&id_str)
            .map_err(|e| DatabaseError::Config(format!("Invalid UUID: {}", e)))?;

        let org_id_str: String = row.get("org_id");
        let org_id = Uuid::parse_str(&org_id_str)
            .map_err(|e| DatabaseError::Config(format!("Invalid org UUID: {}", e)))?;

        let parent_id_str: Option<String> = row.get("parent_id");
        let parent_id = parent_id_str.and_then(|s| Uuid::parse_str(&s).ok());

        let settings_str: String = row.get("settings");
        let settings =
            serde_json::from_str(&settings_str).unwrap_or_else(|_| serde_json::json!({}));

        let is_active: bool = row.get("is_active");

        let created_at_str: String = row.get("created_at");
        let created_at = chrono::DateTime::parse_from_rfc3339(&created_at_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());

        let updated_at_str: String = row.get("updated_at");
        let updated_at = chrono::DateTime::parse_from_rfc3339(&updated_at_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());

        Ok(Namespace {
            id,
            org_id,
            slug: row.get("slug"),
            display_name: row.get("display_name"),
            parent_id,
            description: row.get("description"),
            settings,
            is_active,
            created_at,
            updated_at,
        })
    }

    // ===== Agent Subscriptions =====

    /// Create an agent subscription
    pub async fn create_subscription(
        &self,
        agent_id: Uuid,
        input: CreateAgentSubscription,
    ) -> Result<AgentSubscription, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let now = Utc::now();

        sqlx::query(
            r#"
            INSERT INTO agent_subscriptions (agent_id, namespace_id, include_children, created_at)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT(agent_id, namespace_id) DO UPDATE SET include_children = excluded.include_children
            "#,
        )
        .bind(agent_id.to_string())
        .bind(input.namespace_id.to_string())
        .bind(input.include_children)
        .bind(now.to_rfc3339())
        .execute(pool)
        .await?;

        Ok(AgentSubscription {
            agent_id,
            namespace_id: input.namespace_id,
            include_children: input.include_children,
            created_at: now,
        })
    }

    /// Get subscriptions for an agent
    pub async fn get_agent_subscriptions(
        &self,
        agent_id: Uuid,
    ) -> Result<Vec<AgentSubscription>, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let rows = sqlx::query(
            r#"
            SELECT agent_id, namespace_id, include_children, created_at
            FROM agent_subscriptions
            WHERE agent_id = $1
            "#,
        )
        .bind(agent_id.to_string())
        .fetch_all(pool)
        .await?;

        let mut subscriptions = Vec::with_capacity(rows.len());
        for row in rows {
            subscriptions.push(self.row_to_subscription(row)?);
        }

        Ok(subscriptions)
    }

    /// List all subscriptions for a namespace
    pub async fn list_subscriptions_for_namespace(
        &self,
        namespace_id: Uuid,
    ) -> Result<Vec<AgentSubscription>, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let rows = sqlx::query(
            r#"
            SELECT agent_id, namespace_id, include_children, created_at
            FROM agent_subscriptions
            WHERE namespace_id = $1
            "#,
        )
        .bind(namespace_id.to_string())
        .fetch_all(pool)
        .await?;

        let mut subscriptions = Vec::with_capacity(rows.len());
        for row in rows {
            subscriptions.push(self.row_to_subscription(row)?);
        }

        Ok(subscriptions)
    }

    /// Get agents subscribed to a namespace
    pub async fn get_namespace_subscribers(
        &self,
        namespace_id: Uuid,
    ) -> Result<Vec<Uuid>, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let rows = sqlx::query(
            r#"
            SELECT agent_id FROM agent_subscriptions WHERE namespace_id = $1
            "#,
        )
        .bind(namespace_id.to_string())
        .fetch_all(pool)
        .await?;

        let mut agent_ids = Vec::with_capacity(rows.len());
        for row in rows {
            let agent_id_str: String = row.get("agent_id");
            if let Ok(id) = Uuid::parse_str(&agent_id_str) {
                agent_ids.push(id);
            }
        }

        Ok(agent_ids)
    }

    /// Delete an agent subscription
    pub async fn delete_subscription(
        &self,
        agent_id: Uuid,
        namespace_id: Uuid,
    ) -> Result<bool, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let result = sqlx::query(
            "DELETE FROM agent_subscriptions WHERE agent_id = $1 AND namespace_id = $2",
        )
        .bind(agent_id.to_string())
        .bind(namespace_id.to_string())
        .execute(pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Convert database row to AgentSubscription
    fn row_to_subscription(
        &self,
        row: sqlx::sqlite::SqliteRow,
    ) -> Result<AgentSubscription, DatabaseError> {
        let agent_id_str: String = row.get("agent_id");
        let agent_id = Uuid::parse_str(&agent_id_str)
            .map_err(|e| DatabaseError::Config(format!("Invalid agent UUID: {}", e)))?;

        let namespace_id_str: String = row.get("namespace_id");
        let namespace_id = Uuid::parse_str(&namespace_id_str)
            .map_err(|e| DatabaseError::Config(format!("Invalid namespace UUID: {}", e)))?;

        let include_children: bool = row.get("include_children");

        let created_at_str: String = row.get("created_at");
        let created_at = chrono::DateTime::parse_from_rfc3339(&created_at_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());

        Ok(AgentSubscription {
            agent_id,
            namespace_id,
            include_children,
            created_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DatabaseConfig;
    use crate::db::repositories::OrganizationRepository;
    use crate::domain::organization::CreateOrganization;
    use tempfile::TempDir;

    async fn setup_db() -> (TempDir, Database) {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let url = format!("sqlite:{}", db_path.display());

        let config = DatabaseConfig {
            db_type: "sqlite".to_string(),
            url,
            max_connections: 5,
        };

        let db = Database::new(&config).await.unwrap();
        db.run_migrations().await.unwrap();
        (temp_dir, db)
    }

    async fn create_test_org(db: &Database) -> Uuid {
        let repo = OrganizationRepository::new(db);
        let input = CreateOrganization {
            name: "Test Org".to_string(),
            slug: "test-org".to_string(),
            display_name: None,
            description: None,
            settings: serde_json::json!({}),
        };
        repo.create(input).await.unwrap().id
    }

    #[tokio::test]
    async fn test_create_namespace() {
        let (_temp_dir, db) = setup_db().await;
        let org_id = create_test_org(&db).await;
        let repo = NamespaceRepository::new(&db);

        let input = CreateNamespace {
            slug: "production".to_string(),
            display_name: Some("Production".to_string()),
            parent_id: None,
            description: Some("Production environment".to_string()),
            settings: serde_json::json!({"env": "prod"}),
        };

        let ns = repo.create(org_id, input).await.unwrap();
        assert_eq!(ns.slug, "production");
        assert_eq!(ns.display_name, Some("Production".to_string()));
        assert!(ns.is_active);
    }

    #[tokio::test]
    async fn test_namespace_hierarchy() {
        let (_temp_dir, db) = setup_db().await;
        let org_id = create_test_org(&db).await;
        let repo = NamespaceRepository::new(&db);

        // Create parent
        let parent = repo
            .create(
                org_id,
                CreateNamespace {
                    slug: "production".to_string(),
                    display_name: Some("Production".to_string()),
                    parent_id: None,
                    description: None,
                    settings: serde_json::json!({}),
                },
            )
            .await
            .unwrap();

        // Create child
        let child = repo
            .create(
                org_id,
                CreateNamespace {
                    slug: "production/us-east".to_string(),
                    display_name: Some("US East".to_string()),
                    parent_id: Some(parent.id),
                    description: None,
                    settings: serde_json::json!({}),
                },
            )
            .await
            .unwrap();

        assert_eq!(child.parent_id, Some(parent.id));

        // List children
        let children = repo.list_children(parent.id).await.unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].slug, "production/us-east");
    }

    #[tokio::test]
    async fn test_list_namespaces() {
        let (_temp_dir, db) = setup_db().await;
        let org_id = create_test_org(&db).await;
        let repo = NamespaceRepository::new(&db);

        for slug in &["production", "staging", "development"] {
            repo.create(
                org_id,
                CreateNamespace {
                    slug: slug.to_string(),
                    display_name: Some(slug.to_string()),
                    parent_id: None,
                    description: None,
                    settings: serde_json::json!({}),
                },
            )
            .await
            .unwrap();
        }

        let namespaces = repo.list_by_org(org_id).await.unwrap();
        assert_eq!(namespaces.len(), 3);
    }
}
