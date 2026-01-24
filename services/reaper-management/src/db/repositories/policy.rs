//! Policy repository
//!
//! Provides CRUD operations for policies and policy versions.

use crate::db::{Database, DatabaseError};
use crate::domain::policy::{CreatePolicy, Policy, PolicyLanguage, PolicyVersion, UpdatePolicy};
use chrono::Utc;
use sqlx::Row;
use uuid::Uuid;

/// Repository for policy operations
pub struct PolicyRepository<'a> {
    db: &'a Database,
}

impl<'a> PolicyRepository<'a> {
    /// Create a new policy repository
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// Create a new policy with initial version
    pub async fn create(&self, org_id: Uuid, input: CreatePolicy) -> Result<Policy, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let id = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();

        // Create policy
        sqlx::query(
            r#"
            INSERT INTO policies (id, org_id, team_id, source_id, name, description, language, source_path, current_version, is_active, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, 1, 1, ?, ?)
            "#,
        )
        .bind(id.to_string())
        .bind(org_id.to_string())
        .bind(input.team_id.map(|t| t.to_string()))
        .bind(input.source_id.map(|s| s.to_string()))
        .bind(&input.name)
        .bind(&input.description)
        .bind(input.language.to_string())
        .bind(&input.source_path)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await?;

        // Create initial version
        let version_id = Uuid::new_v4();
        let content_hash = PolicyVersion::compute_hash(&input.content);

        sqlx::query(
            r#"
            INSERT INTO policy_versions (id, policy_id, version, content, content_hash, created_at)
            VALUES (?, ?, 1, ?, ?, ?)
            "#,
        )
        .bind(version_id.to_string())
        .bind(id.to_string())
        .bind(&input.content)
        .bind(&content_hash)
        .bind(&now)
        .execute(pool)
        .await?;

        self.get_by_id(id).await?.ok_or_else(|| {
            DatabaseError::NotFound(format!("Policy not found after creation: {}", id))
        })
    }

    /// Get a policy by ID
    pub async fn get_by_id(&self, id: Uuid) -> Result<Option<Policy>, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let row = sqlx::query(
            r#"
            SELECT id, org_id, team_id, source_id, name, description, language, source_path, is_active, created_at, updated_at
            FROM policies
            WHERE id = ?
            "#,
        )
        .bind(id.to_string())
        .fetch_optional(pool)
        .await?;

        match row {
            Some(row) => Ok(Some(self.row_to_policy(row)?)),
            None => Ok(None),
        }
    }

    /// Get a policy by org_id and name
    pub async fn get_by_name(
        &self,
        org_id: Uuid,
        name: &str,
    ) -> Result<Option<Policy>, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let row = sqlx::query(
            r#"
            SELECT id, org_id, team_id, source_id, name, description, language, source_path, is_active, created_at, updated_at
            FROM policies
            WHERE org_id = ? AND name = ?
            "#,
        )
        .bind(org_id.to_string())
        .bind(name)
        .fetch_optional(pool)
        .await?;

        match row {
            Some(row) => Ok(Some(self.row_to_policy(row)?)),
            None => Ok(None),
        }
    }

    /// List policies for an organization
    pub async fn list_by_org(
        &self,
        org_id: Uuid,
        limit: Option<i64>,
        offset: Option<i64>,
    ) -> Result<Vec<Policy>, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let limit = limit.unwrap_or(100);
        let offset = offset.unwrap_or(0);

        let rows = sqlx::query(
            r#"
            SELECT id, org_id, team_id, source_id, name, description, language, source_path, is_active, created_at, updated_at
            FROM policies
            WHERE org_id = ?
            ORDER BY created_at DESC
            LIMIT ? OFFSET ?
            "#,
        )
        .bind(org_id.to_string())
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;

        let mut policies = Vec::with_capacity(rows.len());
        for row in rows {
            policies.push(self.row_to_policy(row)?);
        }

        Ok(policies)
    }

    /// Count policies for an organization
    pub async fn count_by_org(&self, org_id: Uuid) -> Result<i64, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM policies WHERE org_id = ?")
            .bind(org_id.to_string())
            .fetch_one(pool)
            .await?;

        Ok(row.0)
    }

    /// Update a policy (optionally creates new version if content provided)
    pub async fn update(
        &self,
        id: Uuid,
        input: UpdatePolicy,
    ) -> Result<Option<Policy>, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        // Get current policy
        let current = match self.get_by_id(id).await? {
            Some(policy) => policy,
            None => return Ok(None),
        };

        let now = Utc::now().to_rfc3339();
        let name = input.name.unwrap_or(current.name);
        let description = input.description.or(current.description);
        let is_active = input.is_active.unwrap_or(current.is_active);

        // If content is provided, create a new version
        if let Some(content) = input.content {
            let current_version = self.get_current_version(id).await?;
            let new_version = current_version + 1;
            let version_id = Uuid::new_v4();
            let content_hash = PolicyVersion::compute_hash(&content);

            sqlx::query(
                r#"
                INSERT INTO policy_versions (id, policy_id, version, content, content_hash, created_at)
                VALUES (?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(version_id.to_string())
            .bind(id.to_string())
            .bind(new_version)
            .bind(&content)
            .bind(&content_hash)
            .bind(&now)
            .execute(pool)
            .await?;

            // Update policy with new current_version
            sqlx::query(
                r#"
                UPDATE policies
                SET name = ?, description = ?, is_active = ?, current_version = ?, updated_at = ?
                WHERE id = ?
                "#,
            )
            .bind(&name)
            .bind(&description)
            .bind(is_active)
            .bind(new_version)
            .bind(&now)
            .bind(id.to_string())
            .execute(pool)
            .await?;
        } else {
            // Just update metadata
            sqlx::query(
                r#"
                UPDATE policies
                SET name = ?, description = ?, is_active = ?, updated_at = ?
                WHERE id = ?
                "#,
            )
            .bind(&name)
            .bind(&description)
            .bind(is_active)
            .bind(&now)
            .bind(id.to_string())
            .execute(pool)
            .await?;
        }

        self.get_by_id(id).await
    }

    /// Delete a policy
    pub async fn delete(&self, id: Uuid) -> Result<bool, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let result = sqlx::query("DELETE FROM policies WHERE id = ?")
            .bind(id.to_string())
            .execute(pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Get current version number for a policy
    async fn get_current_version(&self, policy_id: Uuid) -> Result<i32, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let row: (i32,) = sqlx::query_as(
            "SELECT COALESCE(MAX(version), 0) FROM policy_versions WHERE policy_id = ?",
        )
        .bind(policy_id.to_string())
        .fetch_one(pool)
        .await?;

        Ok(row.0)
    }

    /// Get all versions for a policy
    pub async fn get_versions(&self, policy_id: Uuid) -> Result<Vec<PolicyVersion>, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let rows = sqlx::query(
            r#"
            SELECT id, policy_id, version, content, content_hash, source_commit, created_at
            FROM policy_versions
            WHERE policy_id = ?
            ORDER BY version DESC
            "#,
        )
        .bind(policy_id.to_string())
        .fetch_all(pool)
        .await?;

        let mut versions = Vec::with_capacity(rows.len());
        for row in rows {
            versions.push(self.row_to_version(row)?);
        }

        Ok(versions)
    }

    /// Get a specific version
    pub async fn get_version(
        &self,
        policy_id: Uuid,
        version: i32,
    ) -> Result<Option<PolicyVersion>, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let row = sqlx::query(
            r#"
            SELECT id, policy_id, version, content, content_hash, source_commit, created_at
            FROM policy_versions
            WHERE policy_id = ? AND version = ?
            "#,
        )
        .bind(policy_id.to_string())
        .bind(version)
        .fetch_optional(pool)
        .await?;

        match row {
            Some(row) => Ok(Some(self.row_to_version(row)?)),
            None => Ok(None),
        }
    }

    /// Get the latest version for a policy
    pub async fn get_latest_version(
        &self,
        policy_id: Uuid,
    ) -> Result<Option<PolicyVersion>, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let row = sqlx::query(
            r#"
            SELECT id, policy_id, version, content, content_hash, source_commit, created_at
            FROM policy_versions
            WHERE policy_id = ?
            ORDER BY version DESC
            LIMIT 1
            "#,
        )
        .bind(policy_id.to_string())
        .fetch_optional(pool)
        .await?;

        match row {
            Some(row) => Ok(Some(self.row_to_version(row)?)),
            None => Ok(None),
        }
    }

    /// Convert a database row to a Policy
    fn row_to_policy(&self, row: sqlx::sqlite::SqliteRow) -> Result<Policy, DatabaseError> {
        let id_str: String = row.get("id");
        let id = Uuid::parse_str(&id_str)
            .map_err(|e| DatabaseError::Config(format!("Invalid UUID: {}", e)))?;

        let org_id_str: String = row.get("org_id");
        let org_id = Uuid::parse_str(&org_id_str)
            .map_err(|e| DatabaseError::Config(format!("Invalid org UUID: {}", e)))?;

        let team_id: Option<String> = row.get("team_id");
        let team_id = team_id
            .map(|s| Uuid::parse_str(&s))
            .transpose()
            .map_err(|e| DatabaseError::Config(format!("Invalid team UUID: {}", e)))?;

        let source_id: Option<String> = row.get("source_id");
        let source_id = source_id
            .map(|s| Uuid::parse_str(&s))
            .transpose()
            .map_err(|e| DatabaseError::Config(format!("Invalid source UUID: {}", e)))?;

        let language_str: String = row.get("language");
        let language = language_str
            .parse::<PolicyLanguage>()
            .unwrap_or(PolicyLanguage::Reaper);

        let is_active: i32 = row.get("is_active");

        let created_at_str: String = row.get("created_at");
        let created_at = chrono::DateTime::parse_from_rfc3339(&created_at_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());

        let updated_at_str: String = row.get("updated_at");
        let updated_at = chrono::DateTime::parse_from_rfc3339(&updated_at_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());

        Ok(Policy {
            id,
            org_id,
            team_id,
            source_id,
            name: row.get("name"),
            description: row.get("description"),
            language,
            source_path: row.get("source_path"),
            is_active: is_active != 0,
            created_at,
            updated_at,
        })
    }

    /// Convert a database row to a PolicyVersion
    fn row_to_version(&self, row: sqlx::sqlite::SqliteRow) -> Result<PolicyVersion, DatabaseError> {
        let id_str: String = row.get("id");
        let id = Uuid::parse_str(&id_str)
            .map_err(|e| DatabaseError::Config(format!("Invalid UUID: {}", e)))?;

        let policy_id_str: String = row.get("policy_id");
        let policy_id = Uuid::parse_str(&policy_id_str)
            .map_err(|e| DatabaseError::Config(format!("Invalid policy UUID: {}", e)))?;

        let created_at_str: String = row.get("created_at");
        let created_at = chrono::DateTime::parse_from_rfc3339(&created_at_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());

        Ok(PolicyVersion {
            id,
            policy_id,
            version: row.get("version"),
            content: row.get("content"),
            content_hash: row.get("content_hash"),
            source_commit: row.get("source_commit"),
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
    async fn test_create_policy() {
        let (_temp_dir, db) = setup_db().await;
        let org_id = create_test_org(&db).await;
        let repo = PolicyRepository::new(&db);

        let input = CreatePolicy {
            name: "admin-access".to_string(),
            description: Some("Admin access policy".to_string()),
            team_id: None,
            source_id: None,
            language: PolicyLanguage::Reaper,
            source_path: None,
            content: "permit admin".to_string(),
        };

        let policy = repo.create(org_id, input).await.unwrap();
        assert_eq!(policy.name, "admin-access");
        assert_eq!(policy.org_id, org_id);
        assert!(policy.is_active);
    }

    #[tokio::test]
    async fn test_get_versions() {
        let (_temp_dir, db) = setup_db().await;
        let org_id = create_test_org(&db).await;
        let repo = PolicyRepository::new(&db);

        let input = CreatePolicy {
            name: "versioned-policy".to_string(),
            description: None,
            team_id: None,
            source_id: None,
            language: PolicyLanguage::Reaper,
            source_path: None,
            content: "version 1".to_string(),
        };

        let policy = repo.create(org_id, input).await.unwrap();

        // Update with new content
        let update = UpdatePolicy {
            name: None,
            description: None,
            is_active: None,
            content: Some("version 2".to_string()),
        };
        repo.update(policy.id, update).await.unwrap();

        let versions = repo.get_versions(policy.id).await.unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].version, 2);
        assert_eq!(versions[1].version, 1);
    }
}
