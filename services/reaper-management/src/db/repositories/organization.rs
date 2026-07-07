//! Organization repository
//!
//! Provides CRUD operations for organizations.

use crate::db::{Database, DatabaseError};
use crate::domain::organization::{CreateOrganization, Organization, UpdateOrganization};
use chrono::Utc;
use sqlx::Row;
use uuid::Uuid;

/// Repository for organization operations
pub struct OrganizationRepository<'a> {
    db: &'a Database,
}

impl<'a> OrganizationRepository<'a> {
    /// Create a new organization repository
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// Create a new organization
    pub async fn create(&self, input: CreateOrganization) -> Result<Organization, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let id = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();
        let settings = serde_json::to_string(&input.settings).unwrap_or_else(|_| "{}".to_string());

        sqlx::query(
            r#"
            INSERT INTO organizations (id, name, slug, display_name, description, settings, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            "#,
        )
        .bind(id.to_string())
        .bind(&input.name)
        .bind(&input.slug)
        .bind(&input.display_name)
        .bind(&input.description)
        .bind(&settings)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await?;

        self.get_by_id(id).await?.ok_or_else(|| {
            DatabaseError::NotFound(format!("Organization not found after creation: {}", id))
        })
    }

    /// Get an organization by ID
    pub async fn get_by_id(&self, id: Uuid) -> Result<Option<Organization>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let row = sqlx::query(
            r#"
            SELECT id, name, slug, display_name, description, settings, created_at, updated_at
            FROM organizations
            WHERE id = $1
            "#,
        )
        .bind(id.to_string())
        .fetch_optional(pool)
        .await?;

        match row {
            Some(row) => Ok(Some(self.row_to_organization(row)?)),
            None => Ok(None),
        }
    }

    /// Get an organization by slug
    pub async fn get_by_slug(&self, slug: &str) -> Result<Option<Organization>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let row = sqlx::query(
            r#"
            SELECT id, name, slug, display_name, description, settings, created_at, updated_at
            FROM organizations
            WHERE slug = $1
            "#,
        )
        .bind(slug)
        .fetch_optional(pool)
        .await?;

        match row {
            Some(row) => Ok(Some(self.row_to_organization(row)?)),
            None => Ok(None),
        }
    }

    /// List all organizations
    pub async fn list(
        &self,
        limit: Option<i64>,
        offset: Option<i64>,
    ) -> Result<Vec<Organization>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let limit = limit.unwrap_or(100);
        let offset = offset.unwrap_or(0);

        let rows = sqlx::query(
            r#"
            SELECT id, name, slug, display_name, description, settings, created_at, updated_at
            FROM organizations
            ORDER BY created_at DESC
            LIMIT $1 OFFSET $2
            "#,
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;

        let mut orgs = Vec::with_capacity(rows.len());
        for row in rows {
            orgs.push(self.row_to_organization(row)?);
        }

        Ok(orgs)
    }

    /// Count total organizations
    pub async fn count(&self) -> Result<i64, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM organizations")
            .fetch_one(pool)
            .await?;

        Ok(row.0)
    }

    /// Update an organization
    pub async fn update(
        &self,
        id: Uuid,
        input: UpdateOrganization,
    ) -> Result<Option<Organization>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        // Get current org to merge updates
        let current = match self.get_by_id(id).await? {
            Some(org) => org,
            None => return Ok(None),
        };

        let now = Utc::now().to_rfc3339();
        let display_name = input.display_name.or(current.display_name);
        let description = input.description.or(current.description);
        let settings = input
            .settings
            .map(|s| serde_json::to_string(&s).unwrap_or_else(|_| "{}".to_string()))
            .unwrap_or_else(|| {
                serde_json::to_string(&current.settings).unwrap_or_else(|_| "{}".to_string())
            });

        sqlx::query(
            r#"
            UPDATE organizations
            SET display_name = $1, description = $2, settings = $3, updated_at = $4
            WHERE id = $5
            "#,
        )
        .bind(&display_name)
        .bind(&description)
        .bind(&settings)
        .bind(&now)
        .bind(id.to_string())
        .execute(pool)
        .await?;

        self.get_by_id(id).await
    }

    /// Delete an organization
    pub async fn delete(&self, id: Uuid) -> Result<bool, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let result = sqlx::query("DELETE FROM organizations WHERE id = $1")
            .bind(id.to_string())
            .execute(pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Convert a database row to an Organization
    fn row_to_organization(&self, row: sqlx::any::AnyRow) -> Result<Organization, DatabaseError> {
        let id_str: String = row.get("id");
        let id = Uuid::parse_str(&id_str)
            .map_err(|e| DatabaseError::Config(format!("Invalid UUID: {}", e)))?;

        let settings_str: String = row.get("settings");
        let settings: serde_json::Value =
            serde_json::from_str(&settings_str).unwrap_or(serde_json::json!({}));

        let created_at_str: String = row.get("created_at");
        let created_at = chrono::DateTime::parse_from_rfc3339(&created_at_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());

        let updated_at_str: String = row.get("updated_at");
        let updated_at = chrono::DateTime::parse_from_rfc3339(&updated_at_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());

        Ok(Organization {
            id,
            name: row.get("name"),
            slug: row.get("slug"),
            display_name: row.get("display_name"),
            description: row.get("description"),
            settings,
            created_at,
            updated_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn setup_db() -> (TempDir, Database) {
        let temp_dir = TempDir::new().unwrap();

        let config = crate::db::ephemeral_test_config(temp_dir.path()).await;

        let db = Database::new(&config).await.unwrap();
        db.run_migrations().await.unwrap();
        (temp_dir, db)
    }

    #[tokio::test]
    async fn test_create_organization() {
        let (_temp_dir, db) = setup_db().await;
        let repo = OrganizationRepository::new(&db);

        let input = CreateOrganization {
            name: "Test Org".to_string(),
            slug: "test-org".to_string(),
            display_name: Some("Test Organization".to_string()),
            description: Some("A test organization".to_string()),
            settings: serde_json::json!({}),
        };

        let org = repo.create(input).await.unwrap();
        assert_eq!(org.name, "Test Org");
        assert_eq!(org.slug, "test-org");
    }

    #[tokio::test]
    async fn test_get_by_slug() {
        let (_temp_dir, db) = setup_db().await;
        let repo = OrganizationRepository::new(&db);

        let input = CreateOrganization {
            name: "Another Org".to_string(),
            slug: "another-org".to_string(),
            display_name: None,
            description: None,
            settings: serde_json::json!({}),
        };

        repo.create(input).await.unwrap();

        let org = repo.get_by_slug("another-org").await.unwrap().unwrap();
        assert_eq!(org.slug, "another-org");
    }

    #[tokio::test]
    async fn test_list_organizations() {
        let (_temp_dir, db) = setup_db().await;
        let repo = OrganizationRepository::new(&db);

        // Create a few orgs
        for i in 0..3 {
            let input = CreateOrganization {
                name: format!("Org {}", i),
                slug: format!("org-{}", i),
                display_name: None,
                description: None,
                settings: serde_json::json!({}),
            };
            repo.create(input).await.unwrap();
        }

        let orgs = repo.list(None, None).await.unwrap();
        assert_eq!(orgs.len(), 3);

        let count = repo.count().await.unwrap();
        assert_eq!(count, 3);
    }

    #[tokio::test]
    async fn test_delete_organization() {
        let (_temp_dir, db) = setup_db().await;
        let repo = OrganizationRepository::new(&db);

        let input = CreateOrganization {
            name: "To Delete".to_string(),
            slug: "to-delete".to_string(),
            display_name: None,
            description: None,
            settings: serde_json::json!({}),
        };

        let org = repo.create(input).await.unwrap();
        let deleted = repo.delete(org.id).await.unwrap();
        assert!(deleted);

        let found = repo.get_by_id(org.id).await.unwrap();
        assert!(found.is_none());
    }
}
