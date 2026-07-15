//! Bundle repository
//!
//! Data access layer for policy bundles and their relationships.

use chrono::Utc;
use sqlx::Row;
use uuid::Uuid;

use crate::db::{Database, DatabaseError};
use crate::domain::bundle::{Bundle, BundlePolicy, BundlePromotion, BundleStatus, CreateBundle};

/// Repository for bundle operations
pub struct BundleRepository<'a> {
    db: &'a Database,
}

impl<'a> BundleRepository<'a> {
    /// Create a new repository instance
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// Create a new bundle
    pub async fn create(
        &self,
        org_id: Uuid,
        input: &CreateBundle,
    ) -> Result<Bundle, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let id = Uuid::new_v4();
        let now = Utc::now();
        let status = BundleStatus::Draft;

        let sql = r#"
            INSERT INTO bundles (id, org_id, name, description, version, status, policy_count, created_at, updated_at)
            VALUES ($1, $2, $3, $4, '1.0.0', $5, 0, $6, $7)
        "#;

        sqlx::query(sql)
            .bind(id.to_string())
            .bind(org_id.to_string())
            .bind(&input.name)
            .bind(&input.description)
            .bind(status.to_string())
            .bind(now.to_rfc3339())
            .bind(now.to_rfc3339())
            .execute(pool)
            .await?;

        // Add initial policies if provided
        if !input.policy_ids.is_empty() {
            for (idx, policy_id) in input.policy_ids.iter().enumerate() {
                self.add_policy(id, *policy_id, idx as i32).await?;
            }
        }

        self.get_by_id(id)
            .await?
            .ok_or_else(|| DatabaseError::NotFound("Bundle not found after creation".to_string()))
    }

    /// Get a bundle by ID
    pub async fn get_by_id(&self, id: Uuid) -> Result<Option<Bundle>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let sql = r#"
            SELECT id, org_id, name, description, version, status, storage_key, size_bytes, checksum,
                   policy_count, created_at, updated_at, compiled_at, promoted_at
            FROM bundles
            WHERE id = $1
        "#;

        let row = sqlx::query(sql)
            .bind(id.to_string())
            .fetch_optional(pool)
            .await?;

        row.map(|r| self.row_to_bundle(&r)).transpose()
    }

    /// Get a bundle by ID only if it belongs to `org_id`.
    ///
    /// Tenant-scoped lookup: a bundle in another org resolves to `None`,
    /// indistinguishable from "does not exist" — callers surface `404` either
    /// way, so cross-tenant probing can't confirm a bundle id exists.
    pub async fn get_by_id_scoped(
        &self,
        org_id: Uuid,
        id: Uuid,
    ) -> Result<Option<Bundle>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let sql = r#"
            SELECT id, org_id, name, description, version, status, storage_key, size_bytes, checksum,
                   policy_count, created_at, updated_at, compiled_at, promoted_at
            FROM bundles
            WHERE id = $1 AND org_id = $2
        "#;

        let row = sqlx::query(sql)
            .bind(id.to_string())
            .bind(org_id.to_string())
            .fetch_optional(pool)
            .await?;

        row.map(|r| self.row_to_bundle(&r)).transpose()
    }

    /// Count bundles for an organization (quota enforcement, round-2 E4).
    pub async fn count_by_org(&self, org_id: Uuid) -> Result<i64, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM bundles WHERE org_id = $1")
            .bind(org_id.to_string())
            .fetch_one(pool)
            .await?;
        Ok(row.0)
    }

    /// List bundles for an organization
    pub async fn list_by_org(
        &self,
        org_id: Uuid,
        status_filter: Option<BundleStatus>,
    ) -> Result<Vec<Bundle>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let sql = if status_filter.is_some() {
            r#"
                SELECT id, org_id, name, description, version, status, storage_key, size_bytes, checksum,
                       policy_count, created_at, updated_at, compiled_at, promoted_at
                FROM bundles
                WHERE org_id = $1 AND status = $2
                ORDER BY created_at DESC
            "#
        } else {
            r#"
                SELECT id, org_id, name, description, version, status, storage_key, size_bytes, checksum,
                       policy_count, created_at, updated_at, compiled_at, promoted_at
                FROM bundles
                WHERE org_id = $1
                ORDER BY created_at DESC
            "#
        };

        let rows = if let Some(status) = status_filter {
            sqlx::query(sql)
                .bind(org_id.to_string())
                .bind(status.to_string())
                .fetch_all(pool)
                .await
        } else {
            sqlx::query(sql)
                .bind(org_id.to_string())
                .fetch_all(pool)
                .await
        }?;

        rows.iter().map(|r| self.row_to_bundle(r)).collect()
    }

    /// Keyset-paginated listing (Plan 07 Phase E): rows strictly after the
    /// `(created_at, id)` position in `ORDER BY created_at DESC, id DESC`
    /// order, with the optional status filter preserved. `fetch` is
    /// `page limit + 1` — the caller's has-more sentinel.
    pub async fn list_page_by_org(
        &self,
        org_id: Uuid,
        status_filter: Option<BundleStatus>,
        fetch: i64,
        after: Option<&(String, String)>,
    ) -> Result<Vec<Bundle>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        // Assemble the predicate set with sequential placeholders; bind order
        // below must mirror this order exactly.
        let mut sql = String::from(
            "SELECT id, org_id, name, description, version, status, storage_key, size_bytes, checksum, \
                    policy_count, created_at, updated_at, compiled_at, promoted_at \
             FROM bundles WHERE org_id = $1",
        );
        let mut n = 1;
        if status_filter.is_some() {
            n += 1;
            sql.push_str(&format!(" AND status = ${n}"));
        }
        if after.is_some() {
            sql.push_str(&format!(" AND (created_at, id) < (${}, ${})", n + 1, n + 2));
            n += 2;
        }
        sql.push_str(&format!(
            " ORDER BY created_at DESC, id DESC LIMIT ${}",
            n + 1
        ));

        let mut q = sqlx::query(&sql).bind(org_id.to_string());
        if let Some(status) = status_filter {
            q = q.bind(status.to_string());
        }
        if let Some((created_at, id)) = after {
            q = q.bind(created_at).bind(id);
        }
        let rows = q.bind(fetch).fetch_all(pool).await?;

        rows.iter().map(|r| self.row_to_bundle(r)).collect()
    }

    /// Get the currently promoted bundle for an organization
    pub async fn get_promoted(&self, org_id: Uuid) -> Result<Option<Bundle>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let sql = r#"
            SELECT id, org_id, name, description, version, status, storage_key, size_bytes, checksum,
                   policy_count, created_at, updated_at, compiled_at, promoted_at
            FROM bundles
            WHERE org_id = $1 AND status = 'promoted'
            ORDER BY promoted_at DESC
            LIMIT 1
        "#;

        let row = sqlx::query(sql)
            .bind(org_id.to_string())
            .fetch_optional(pool)
            .await?;

        row.map(|r| self.row_to_bundle(&r)).transpose()
    }

    /// Update bundle status
    pub async fn update_status(
        &self,
        id: Uuid,
        status: BundleStatus,
        _promoted_by: Option<&str>,
        notes: Option<&str>,
    ) -> Result<Bundle, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        // Get current bundle for audit log
        let bundle = self
            .get_by_id(id)
            .await?
            .ok_or_else(|| DatabaseError::NotFound(format!("Bundle {} not found", id)))?;

        let now = Utc::now();

        // Update status with appropriate timestamp
        let sql = match status {
            BundleStatus::Compiled => {
                r#"
                    UPDATE bundles
                    SET status = $1, updated_at = $2, compiled_at = $3
                    WHERE id = $4
                "#
            }
            BundleStatus::Promoted => {
                r#"
                    UPDATE bundles
                    SET status = $1, updated_at = $2, promoted_at = $3
                    WHERE id = $4
                "#
            }
            _ => {
                r#"
                    UPDATE bundles
                    SET status = $1, updated_at = $2
                    WHERE id = $3
                "#
            }
        };

        match status {
            BundleStatus::Compiled | BundleStatus::Promoted => {
                sqlx::query(sql)
                    .bind(status.to_string())
                    .bind(now.to_rfc3339())
                    .bind(now.to_rfc3339())
                    .bind(id.to_string())
                    .execute(pool)
                    .await
            }
            _ => {
                sqlx::query(sql)
                    .bind(status.to_string())
                    .bind(now.to_rfc3339())
                    .bind(id.to_string())
                    .execute(pool)
                    .await
            }
        }?;

        // Record promotion history
        self.record_promotion(id, bundle.status, status, None, notes)
            .await?;

        self.get_by_id(id)
            .await?
            .ok_or_else(|| DatabaseError::NotFound("Bundle not found after update".to_string()))
    }

    /// Update bundle after compilation
    pub async fn update_compilation(
        &self,
        id: Uuid,
        storage_key: &str,
        size_bytes: i64,
        checksum: &str,
        policy_count: i32,
    ) -> Result<Bundle, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let now = Utc::now();

        let sql = r#"
            UPDATE bundles
            SET status = 'compiled', storage_key = $1, size_bytes = $2, checksum = $3,
                policy_count = $4, compiled_at = $5, updated_at = $6
            WHERE id = $7
        "#;

        sqlx::query(sql)
            .bind(storage_key)
            .bind(size_bytes)
            .bind(checksum)
            .bind(policy_count)
            .bind(now.to_rfc3339())
            .bind(now.to_rfc3339())
            .bind(id.to_string())
            .execute(pool)
            .await?;

        // Record status change
        self.record_promotion(id, BundleStatus::Draft, BundleStatus::Compiled, None, None)
            .await?;

        self.get_by_id(id)
            .await?
            .ok_or_else(|| DatabaseError::NotFound("Bundle not found after update".to_string()))
    }

    /// Update bundle metadata.
    ///
    /// `expected_updated_at` is the optimistic-concurrency guard (Plan 07
    /// Phase C): when `Some(rfc3339)`, the UPDATE carries
    /// `AND updated_at = $expected` — every bundle write path bumps
    /// `updated_at`, so a concurrent writer makes this write match zero rows
    /// and the call returns [`DatabaseError::VersionConflict`] instead of
    /// silently clobbering. `None` skips the guard (transitional warn-only
    /// mode for clients that did not send `If-Match`).
    pub async fn update(
        &self,
        id: Uuid,
        name: Option<&str>,
        description: Option<&str>,
        version: Option<&str>,
        expected_updated_at: Option<&str>,
    ) -> Result<Bundle, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let bundle = self
            .get_by_id(id)
            .await?
            .ok_or_else(|| DatabaseError::NotFound(format!("Bundle {} not found", id)))?;

        let now = Utc::now();
        let new_name = name.unwrap_or(&bundle.name);
        let new_description = description.or(bundle.description.as_deref());
        let new_version = version.unwrap_or("1.0.0");

        let result = if let Some(expected) = expected_updated_at {
            let sql = r#"
                UPDATE bundles
                SET name = $1, description = $2, version = $3, updated_at = $4
                WHERE id = $5 AND updated_at = $6
            "#;
            sqlx::query(sql)
                .bind(new_name)
                .bind(new_description)
                .bind(new_version)
                .bind(now.to_rfc3339())
                .bind(id.to_string())
                .bind(expected)
                .execute(pool)
                .await?
        } else {
            let sql = r#"
                UPDATE bundles
                SET name = $1, description = $2, version = $3, updated_at = $4
                WHERE id = $5
            "#;
            sqlx::query(sql)
                .bind(new_name)
                .bind(new_description)
                .bind(new_version)
                .bind(now.to_rfc3339())
                .bind(id.to_string())
                .execute(pool)
                .await?
        };

        if result.rows_affected() == 0 && expected_updated_at.is_some() {
            return Err(DatabaseError::VersionConflict(format!(
                "bundle {id} was modified concurrently"
            )));
        }

        self.get_by_id(id)
            .await?
            .ok_or_else(|| DatabaseError::NotFound("Bundle not found after update".to_string()))
    }

    /// Delete a bundle
    pub async fn delete(&self, id: Uuid) -> Result<(), DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let sql = "DELETE FROM bundles WHERE id = $1";

        let result = sqlx::query(sql).bind(id.to_string()).execute(pool).await?;

        if result.rows_affected() == 0 {
            return Err(DatabaseError::NotFound(format!("Bundle {} not found", id)));
        }

        Ok(())
    }

    /// Find the bundle already materialized for `(source, commit)`, if any —
    /// the idempotency check for git-sync materialization (Plan 09 Step 2):
    /// a webhook and a poll landing on the same SHA must not double-apply.
    pub async fn find_by_source_commit(
        &self,
        source_id: Uuid,
        commit: &str,
    ) -> Result<Option<Uuid>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let row = sqlx::query(
            "SELECT id FROM bundles WHERE source_id = $1 AND source_commit = $2 LIMIT 1",
        )
        .bind(source_id.to_string())
        .bind(commit)
        .fetch_optional(pool)
        .await?;

        row.map(|r| {
            r.get::<String, _>("id")
                .parse()
                .map_err(|e| DatabaseError::Config(format!("Invalid UUID: {}", e)))
        })
        .transpose()
    }

    /// Record which source + commit produced this bundle (Plan 09 Step 2).
    pub async fn link_source(
        &self,
        bundle_id: Uuid,
        source_id: Uuid,
        commit: &str,
    ) -> Result<(), DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let result =
            sqlx::query("UPDATE bundles SET source_id = $1, source_commit = $2 WHERE id = $3")
                .bind(source_id.to_string())
                .bind(commit)
                .bind(bundle_id.to_string())
                .execute(pool)
                .await?;

        if result.rows_affected() == 0 {
            return Err(DatabaseError::NotFound(format!(
                "Bundle {} not found",
                bundle_id
            )));
        }
        Ok(())
    }

    /// Add a policy to a bundle
    pub async fn add_policy(
        &self,
        bundle_id: Uuid,
        policy_id: Uuid,
        priority: i32,
    ) -> Result<(), DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let id = Uuid::new_v4();
        let now = Utc::now();

        // Get current policy version
        let version_sql = "SELECT current_version FROM policies WHERE id = $1";
        let version: i32 = sqlx::query(version_sql)
            .bind(policy_id.to_string())
            .fetch_optional(pool)
            .await?
            .map(|r| r.get::<i32, _>("current_version"))
            .unwrap_or(1);

        let sql = r#"
            INSERT INTO bundle_policies (id, bundle_id, policy_id, policy_version, priority, created_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT(bundle_id, policy_id) DO UPDATE SET
                policy_version = excluded.policy_version,
                priority = excluded.priority
        "#;

        sqlx::query(sql)
            .bind(id.to_string())
            .bind(bundle_id.to_string())
            .bind(policy_id.to_string())
            .bind(version)
            .bind(priority)
            .bind(now.to_rfc3339())
            .execute(pool)
            .await?;

        // Update policy count
        self.update_policy_count(bundle_id).await?;

        Ok(())
    }

    /// Remove a policy from a bundle
    pub async fn remove_policy(
        &self,
        bundle_id: Uuid,
        policy_id: Uuid,
    ) -> Result<(), DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let sql = "DELETE FROM bundle_policies WHERE bundle_id = $1 AND policy_id = $2";

        sqlx::query(sql)
            .bind(bundle_id.to_string())
            .bind(policy_id.to_string())
            .execute(pool)
            .await?;

        // Update policy count
        self.update_policy_count(bundle_id).await?;

        Ok(())
    }

    /// Get policies in a bundle
    pub async fn get_policies(&self, bundle_id: Uuid) -> Result<Vec<BundlePolicy>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let sql = r#"
            SELECT bundle_id, policy_id, policy_version, priority
            FROM bundle_policies
            WHERE bundle_id = $1
            ORDER BY priority ASC
        "#;

        let rows = sqlx::query(sql)
            .bind(bundle_id.to_string())
            .fetch_all(pool)
            .await?;

        rows.iter()
            .map(|r| {
                Ok(BundlePolicy {
                    bundle_id: r
                        .get::<String, _>("bundle_id")
                        .parse()
                        .map_err(|e| DatabaseError::Config(format!("Invalid UUID: {}", e)))?,
                    policy_id: r
                        .get::<String, _>("policy_id")
                        .parse()
                        .map_err(|e| DatabaseError::Config(format!("Invalid UUID: {}", e)))?,
                    policy_version: r.get("policy_version"),
                    priority: r.get("priority"),
                })
            })
            .collect()
    }

    /// Get promotion history for a bundle
    pub async fn get_promotion_history(
        &self,
        bundle_id: Uuid,
    ) -> Result<Vec<BundlePromotion>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let sql = r#"
            SELECT id, bundle_id, from_status, to_status, promoted_by, notes, created_at
            FROM bundle_promotions
            WHERE bundle_id = $1
            ORDER BY created_at DESC
        "#;

        let rows = sqlx::query(sql)
            .bind(bundle_id.to_string())
            .fetch_all(pool)
            .await?;

        rows.iter()
            .map(|r| {
                let from_status: String = r.get("from_status");
                let to_status: String = r.get("to_status");
                let created_at: String = r.get("created_at");

                Ok(BundlePromotion {
                    id: r
                        .get::<String, _>("id")
                        .parse()
                        .map_err(|e| DatabaseError::Config(format!("Invalid UUID: {}", e)))?,
                    bundle_id: r
                        .get::<String, _>("bundle_id")
                        .parse()
                        .map_err(|e| DatabaseError::Config(format!("Invalid UUID: {}", e)))?,
                    from_status: from_status.parse().unwrap_or(BundleStatus::Draft),
                    to_status: to_status.parse().unwrap_or(BundleStatus::Draft),
                    promoted_by: r.get("promoted_by"),
                    promoted_at: chrono::DateTime::parse_from_rfc3339(&created_at)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    notes: r.get("notes"),
                })
            })
            .collect()
    }

    /// Record a promotion event
    async fn record_promotion(
        &self,
        bundle_id: Uuid,
        from_status: BundleStatus,
        to_status: BundleStatus,
        promoted_by: Option<&str>,
        notes: Option<&str>,
    ) -> Result<(), DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let id = Uuid::new_v4();
        let now = Utc::now();

        let sql = r#"
            INSERT INTO bundle_promotions (id, bundle_id, from_status, to_status, promoted_by, notes, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
        "#;

        sqlx::query(sql)
            .bind(id.to_string())
            .bind(bundle_id.to_string())
            .bind(from_status.to_string())
            .bind(to_status.to_string())
            .bind(promoted_by)
            .bind(notes)
            .bind(now.to_rfc3339())
            .execute(pool)
            .await?;

        Ok(())
    }

    /// Update the policy count for a bundle
    async fn update_policy_count(&self, bundle_id: Uuid) -> Result<(), DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let count_sql = "SELECT COUNT(*) as cnt FROM bundle_policies WHERE bundle_id = $1";
        let count: i32 = sqlx::query(count_sql)
            .bind(bundle_id.to_string())
            .fetch_one(pool)
            .await
            .map(|r| r.get::<i32, _>("cnt"))?;

        let update_sql = "UPDATE bundles SET policy_count = $1, updated_at = $2 WHERE id = $3";
        sqlx::query(update_sql)
            .bind(count)
            .bind(Utc::now().to_rfc3339())
            .bind(bundle_id.to_string())
            .execute(pool)
            .await?;

        Ok(())
    }

    /// Convert database row to Bundle
    fn row_to_bundle(&self, row: &sqlx::any::AnyRow) -> Result<Bundle, DatabaseError> {
        let id: String = row.get("id");
        let org_id: String = row.get("org_id");
        let status: String = row.get("status");
        let created_at: String = row.get("created_at");
        let updated_at: String = row.get("updated_at");

        Ok(Bundle {
            id: id
                .parse()
                .map_err(|e| DatabaseError::Config(format!("Invalid UUID: {}", e)))?,
            org_id: org_id
                .parse()
                .map_err(|e| DatabaseError::Config(format!("Invalid UUID: {}", e)))?,
            name: row.get("name"),
            description: row.get("description"),
            status: status.parse().unwrap_or(BundleStatus::Draft),
            storage_key: row.get("storage_key"),
            size_bytes: row.get("size_bytes"),
            checksum: row.get("checksum"),
            policy_count: row.get("policy_count"),
            created_at: chrono::DateTime::parse_from_rfc3339(&created_at)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            updated_at: chrono::DateTime::parse_from_rfc3339(&updated_at)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn setup_db() -> (TempDir, std::sync::Arc<Database>) {
        let temp_dir = TempDir::new().unwrap();

        let config = crate::db::ephemeral_test_config(temp_dir.path()).await;

        let db = Database::new(&config).await.unwrap();
        db.run_migrations().await.unwrap();
        (temp_dir, std::sync::Arc::new(db))
    }

    async fn create_test_org(db: &Database) -> Uuid {
        let pool = db.any_pool().unwrap();
        let org_id = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO organizations (id, name, slug, created_at, updated_at) VALUES ($1, $2, $3, $4, $5)"
        )
        .bind(org_id.to_string())
        .bind("Test Org")
        .bind("test-org")
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
        org_id
    }

    #[tokio::test]
    async fn test_create_and_get_bundle() {
        let (_temp_dir, db) = setup_db().await;
        let org_id = create_test_org(&db).await;
        let repo = BundleRepository::new(&db);

        let input = CreateBundle {
            name: "test-bundle".to_string(),
            description: Some("Test bundle".to_string()),
            policy_ids: vec![],
        };

        let bundle = repo.create(org_id, &input).await.unwrap();
        assert_eq!(bundle.name, "test-bundle");
        assert_eq!(bundle.status, BundleStatus::Draft);
        assert_eq!(bundle.policy_count, 0);

        let retrieved = repo.get_by_id(bundle.id).await.unwrap().unwrap();
        assert_eq!(retrieved.name, "test-bundle");
    }

    #[tokio::test]
    async fn test_list_bundles() {
        let (_temp_dir, db) = setup_db().await;
        let org_id = create_test_org(&db).await;
        let repo = BundleRepository::new(&db);

        // Create two bundles
        repo.create(
            org_id,
            &CreateBundle {
                name: "bundle-1".to_string(),
                description: None,
                policy_ids: vec![],
            },
        )
        .await
        .unwrap();

        repo.create(
            org_id,
            &CreateBundle {
                name: "bundle-2".to_string(),
                description: None,
                policy_ids: vec![],
            },
        )
        .await
        .unwrap();

        let bundles = repo.list_by_org(org_id, None).await.unwrap();
        assert_eq!(bundles.len(), 2);
    }

    #[tokio::test]
    async fn test_update_bundle_status() {
        let (_temp_dir, db) = setup_db().await;
        let org_id = create_test_org(&db).await;
        let repo = BundleRepository::new(&db);

        let bundle = repo
            .create(
                org_id,
                &CreateBundle {
                    name: "status-test".to_string(),
                    description: None,
                    policy_ids: vec![],
                },
            )
            .await
            .unwrap();

        // Compile the bundle
        let updated = repo
            .update_compilation(bundle.id, "bundles/test.rbb", 1024, "abc123", 5)
            .await
            .unwrap();

        assert_eq!(updated.status, BundleStatus::Compiled);
        assert_eq!(updated.storage_key, Some("bundles/test.rbb".to_string()));
        assert_eq!(updated.policy_count, 5);

        // Check promotion history
        let history = repo.get_promotion_history(bundle.id).await.unwrap();
        assert!(!history.is_empty());
    }

    #[tokio::test]
    async fn test_delete_bundle() {
        let (_temp_dir, db) = setup_db().await;
        let org_id = create_test_org(&db).await;
        let repo = BundleRepository::new(&db);

        let bundle = repo
            .create(
                org_id,
                &CreateBundle {
                    name: "delete-test".to_string(),
                    description: None,
                    policy_ids: vec![],
                },
            )
            .await
            .unwrap();

        repo.delete(bundle.id).await.unwrap();

        let result = repo.get_by_id(bundle.id).await.unwrap();
        assert!(result.is_none());
    }
}
