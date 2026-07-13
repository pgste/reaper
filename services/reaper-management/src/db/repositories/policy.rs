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
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let id = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();

        // Create policy
        sqlx::query(
            r#"
            INSERT INTO policies (id, org_id, team_id, source_id, name, description, language, source_path, current_version, is_active, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 1, 1, $9, $10)
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
            VALUES ($1, $2, 1, $3, $4, $5)
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
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let row = sqlx::query(
            r#"
            SELECT id, org_id, team_id, source_id, name, description, language, source_path, is_active, created_at, updated_at
            FROM policies
            WHERE id = $1
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
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let row = sqlx::query(
            r#"
            SELECT id, org_id, team_id, source_id, name, description, language, source_path, is_active, created_at, updated_at
            FROM policies
            WHERE org_id = $1 AND name = $2
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
    /// List all policies materialized from a given source (Plan 09 Step 8).
    /// Used by drift detection to compare the deployed policy set against the
    /// source's git HEAD.
    pub async fn list_by_source(&self, source_id: Uuid) -> Result<Vec<Policy>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let rows = sqlx::query(
            r#"
            SELECT id, org_id, team_id, source_id, name, description, language, source_path, is_active, created_at, updated_at
            FROM policies
            WHERE source_id = $1
            ORDER BY name ASC
            "#,
        )
        .bind(source_id.to_string())
        .fetch_all(pool)
        .await?;

        let mut policies = Vec::with_capacity(rows.len());
        for row in rows {
            policies.push(self.row_to_policy(row)?);
        }
        Ok(policies)
    }

    pub async fn list_by_org(
        &self,
        org_id: Uuid,
        limit: Option<i64>,
        offset: Option<i64>,
    ) -> Result<Vec<Policy>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let limit = limit.unwrap_or(100);
        let offset = offset.unwrap_or(0);

        let rows = sqlx::query(
            r#"
            SELECT id, org_id, team_id, source_id, name, description, language, source_path, is_active, created_at, updated_at
            FROM policies
            WHERE org_id = $1
            ORDER BY created_at DESC
            LIMIT $2 OFFSET $3
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

    /// Keyset-paginated listing (Plan 07 Phase E): rows strictly after the
    /// `(created_at, id)` position in `ORDER BY created_at DESC, id DESC`
    /// order. Unlike OFFSET, the walk never drifts under concurrent inserts
    /// and stays O(page) on deep pages. `fetch` is `page limit + 1` — the
    /// caller uses the sentinel row to detect whether another page exists.
    pub async fn list_page_by_org(
        &self,
        org_id: Uuid,
        fetch: i64,
        after: Option<&(String, String)>,
    ) -> Result<Vec<Policy>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let rows = if let Some((created_at, id)) = after {
            sqlx::query(
                r#"
                SELECT id, org_id, team_id, source_id, name, description, language, source_path, is_active, created_at, updated_at
                FROM policies
                WHERE org_id = $1 AND (created_at, id) < ($2, $3)
                ORDER BY created_at DESC, id DESC
                LIMIT $4
                "#,
            )
            .bind(org_id.to_string())
            .bind(created_at)
            .bind(id)
            .bind(fetch)
            .fetch_all(pool)
            .await?
        } else {
            sqlx::query(
                r#"
                SELECT id, org_id, team_id, source_id, name, description, language, source_path, is_active, created_at, updated_at
                FROM policies
                WHERE org_id = $1
                ORDER BY created_at DESC, id DESC
                LIMIT $2
                "#,
            )
            .bind(org_id.to_string())
            .bind(fetch)
            .fetch_all(pool)
            .await?
        };

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
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM policies WHERE org_id = $1")
            .bind(org_id.to_string())
            .fetch_one(pool)
            .await?;

        Ok(row.0)
    }

    /// Update a policy (optionally creates a new version if content provided).
    ///
    /// Every successful UPDATE — content or metadata — bumps `row_version`,
    /// the monotonic counter the policy ETag derives from (R2-03: metadata
    /// edits must rotate the ETag too, per RFC 9110 §8.8.1).
    ///
    /// `expected_row_version` is the optimistic-concurrency guard (Plan 07
    /// Phase C): when `Some(v)`, the UPDATE carries `AND row_version = v`, so
    /// a concurrent writer that already bumped it — including a metadata-only
    /// writer — makes this write match zero rows and the call returns
    /// [`DatabaseError::VersionConflict`] instead of silently clobbering.
    /// `None` skips the guard (internal callers such as git sync, and the
    /// opt-down warn-only mode for clients that did not send `If-Match`).
    pub async fn update(
        &self,
        id: Uuid,
        input: UpdatePolicy,
        expected_row_version: Option<i64>,
    ) -> Result<Option<Policy>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
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

        // If content is provided, create a new version. The guarded UPDATE of
        // the policies row is the atomic arbiter, so it runs FIRST; the
        // immutable version row is only inserted once this writer has won,
        // and both statements commit together.
        if let Some(content) = input.content {
            let current_version = self.get_current_version(id).await?;
            let new_version = current_version + 1;
            let version_id = Uuid::new_v4();
            let content_hash = PolicyVersion::compute_hash(&content);

            let mut tx = pool.begin().await?;

            let result = if let Some(expected) = expected_row_version {
                sqlx::query(
                    r#"
                    UPDATE policies
                    SET name = $1, description = $2, is_active = $3, current_version = $4,
                        row_version = row_version + 1, updated_at = $5
                    WHERE id = $6 AND row_version = $7
                    "#,
                )
                .bind(&name)
                .bind(&description)
                .bind(is_active as i64)
                .bind(new_version)
                .bind(&now)
                .bind(id.to_string())
                .bind(expected)
                .execute(&mut *tx)
                .await?
            } else {
                sqlx::query(
                    r#"
                    UPDATE policies
                    SET name = $1, description = $2, is_active = $3, current_version = $4,
                        row_version = row_version + 1, updated_at = $5
                    WHERE id = $6
                    "#,
                )
                .bind(&name)
                .bind(&description)
                .bind(is_active as i64)
                .bind(new_version)
                .bind(&now)
                .bind(id.to_string())
                .execute(&mut *tx)
                .await?
            };

            if result.rows_affected() == 0 {
                tx.rollback().await?;
                return Err(DatabaseError::VersionConflict(format!(
                    "policy {id} was modified concurrently"
                )));
            }

            sqlx::query(
                r#"
                INSERT INTO policy_versions (id, policy_id, version, content, content_hash, created_at)
                VALUES ($1, $2, $3, $4, $5, $6)
                "#,
            )
            .bind(version_id.to_string())
            .bind(id.to_string())
            .bind(new_version)
            .bind(&content)
            .bind(&content_hash)
            .bind(&now)
            .execute(&mut *tx)
            .await?;

            tx.commit().await?;
        } else {
            // Metadata-only update. Guarded on `row_version`, which EVERY
            // write bumps — two racing metadata editors resolve to exactly
            // one winner and one 412 (R2-03 closed; the old
            // `current_version` guard let the loser clobber silently).
            let result = if let Some(expected) = expected_row_version {
                sqlx::query(
                    r#"
                    UPDATE policies
                    SET name = $1, description = $2, is_active = $3,
                        row_version = row_version + 1, updated_at = $4
                    WHERE id = $5 AND row_version = $6
                    "#,
                )
                .bind(&name)
                .bind(&description)
                .bind(is_active as i64)
                .bind(&now)
                .bind(id.to_string())
                .bind(expected)
                .execute(pool)
                .await?
            } else {
                sqlx::query(
                    r#"
                    UPDATE policies
                    SET name = $1, description = $2, is_active = $3,
                        row_version = row_version + 1, updated_at = $4
                    WHERE id = $5
                    "#,
                )
                .bind(&name)
                .bind(&description)
                .bind(is_active as i64)
                .bind(&now)
                .bind(id.to_string())
                .execute(pool)
                .await?
            };

            if result.rows_affected() == 0 && expected_row_version.is_some() {
                return Err(DatabaseError::VersionConflict(format!(
                    "policy {id} was modified concurrently"
                )));
            }
        }

        self.get_by_id(id).await
    }

    /// The policy's `row_version` (bumped by EVERY write — the ETag/guard
    /// source since R2-03), its current content-version number, and that
    /// version's content hash, read as one consistent triple. The ETag is
    /// derived from `(content_hash, row_version)` so it changes on content
    /// AND metadata edits (RFC 9110 §8.8.1), and the `row_version` is what
    /// the guarded UPDATE re-checks in SQL.
    pub async fn current_version_info(
        &self,
        id: Uuid,
    ) -> Result<(i64, i32, Option<String>), DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let row = sqlx::query(
            r#"
            SELECT p.row_version, p.current_version, v.content_hash
            FROM policies p
            LEFT JOIN policy_versions v
                   ON v.policy_id = p.id AND v.version = p.current_version
            WHERE p.id = $1
            "#,
        )
        .bind(id.to_string())
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| DatabaseError::NotFound(format!("Policy {id} not found")))?;

        let row_version: i64 = row.get("row_version");
        let version: i32 = row.get("current_version");
        let content_hash: Option<String> = row.get("content_hash");
        Ok((row_version, version, content_hash))
    }

    /// Delete a policy
    pub async fn delete(&self, id: Uuid) -> Result<bool, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let result = sqlx::query("DELETE FROM policies WHERE id = $1")
            .bind(id.to_string())
            .execute(pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Get current version number for a policy
    async fn get_current_version(&self, policy_id: Uuid) -> Result<i32, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let row: (i32,) = sqlx::query_as(
            "SELECT COALESCE(MAX(version), 0) FROM policy_versions WHERE policy_id = $1",
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
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let rows = sqlx::query(
            r#"
            SELECT id, policy_id, version, content, content_hash, source_commit, created_at
            FROM policy_versions
            WHERE policy_id = $1
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
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let row = sqlx::query(
            r#"
            SELECT id, policy_id, version, content, content_hash, source_commit, created_at
            FROM policy_versions
            WHERE policy_id = $1 AND version = $2
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
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let row = sqlx::query(
            r#"
            SELECT id, policy_id, version, content, content_hash, source_commit, created_at
            FROM policy_versions
            WHERE policy_id = $1
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
    fn row_to_policy(&self, row: sqlx::any::AnyRow) -> Result<Policy, DatabaseError> {
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
    fn row_to_version(&self, row: sqlx::any::AnyRow) -> Result<PolicyVersion, DatabaseError> {
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
    use crate::db::repositories::OrganizationRepository;
    use crate::domain::organization::CreateOrganization;
    use tempfile::TempDir;

    async fn setup_db() -> (TempDir, Database) {
        let temp_dir = TempDir::new().unwrap();

        let config = crate::db::ephemeral_test_config(temp_dir.path()).await;

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
        // Guarded with the version we hold (1) — the optimistic-concurrency
        // fast path (Plan 07 Phase C).
        repo.update(policy.id, update, Some(1)).await.unwrap();

        // A writer holding the now-stale version 1 loses with VersionConflict.
        let stale = UpdatePolicy {
            name: None,
            description: None,
            is_active: None,
            content: Some("version 3 (stale writer)".to_string()),
        };
        let err = repo.update(policy.id, stale, Some(1)).await.unwrap_err();
        assert!(matches!(err, crate::db::DatabaseError::VersionConflict(_)));

        let versions = repo.get_versions(policy.id).await.unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].version, 2);
        assert_eq!(versions[1].version, 1);
    }
}
