//! Policy source repository for database operations
//!
//! Handles persistence of policy source records.

use chrono::Utc;
use sqlx::Row;
use uuid::Uuid;

use crate::db::{Database, DatabaseError};
use crate::domain::source::{
    CreatePolicySource, PolicySource, SourceType, SyncStatus, UpdatePolicySource,
};

/// Repository for policy source operations
pub struct PolicySourceRepository<'a> {
    db: &'a Database,
}

impl<'a> PolicySourceRepository<'a> {
    /// Create a new policy source repository
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// Create a new policy source
    pub async fn create(
        &self,
        org_id: Uuid,
        input: CreatePolicySource,
    ) -> Result<PolicySource, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let id = Uuid::new_v4();
        let now = Utc::now();
        let config_json = serde_json::to_string(&input.config).unwrap_or_else(|_| "{}".to_string());

        sqlx::query(
            r#"
            INSERT INTO policy_sources (id, org_id, name, description, source_type, config, sync_interval_secs, sync_status, is_enabled, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            "#,
        )
        .bind(id.to_string())
        .bind(org_id.to_string())
        .bind(&input.name)
        .bind(&input.description)
        .bind(input.source_type.to_string())
        .bind(&config_json)
        .bind(input.sync_interval_secs as i64)
        .bind(SyncStatus::Pending.to_string())
        .bind(1i64)
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .execute(pool)
        .await?;

        Ok(PolicySource {
            id,
            org_id,
            name: input.name,
            description: input.description,
            source_type: input.source_type,
            config: input.config,
            sync_interval_secs: input.sync_interval_secs,
            sync_status: SyncStatus::Pending,
            last_sync_at: None,
            last_sync_error: None,
            last_sync_commit: None,
            is_enabled: true,
            created_at: now,
            updated_at: now,
        })
    }

    /// Get policy source by ID
    pub async fn get_by_id(&self, id: Uuid) -> Result<Option<PolicySource>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let row = sqlx::query(
            r#"
            SELECT id, org_id, name, description, source_type, config, sync_interval_secs,
                   sync_status, last_sync_at, last_sync_error, last_sync_commit, is_enabled,
                   created_at, updated_at
            FROM policy_sources
            WHERE id = $1
            "#,
        )
        .bind(id.to_string())
        .fetch_optional(pool)
        .await?;

        match row {
            Some(row) => Ok(Some(self.row_to_source(row)?)),
            None => Ok(None),
        }
    }

    /// Get policy source by name within an organization
    pub async fn get_by_name(
        &self,
        org_id: Uuid,
        name: &str,
    ) -> Result<Option<PolicySource>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let row = sqlx::query(
            r#"
            SELECT id, org_id, name, description, source_type, config, sync_interval_secs,
                   sync_status, last_sync_at, last_sync_error, last_sync_commit, is_enabled,
                   created_at, updated_at
            FROM policy_sources
            WHERE org_id = $1 AND name = $2
            "#,
        )
        .bind(org_id.to_string())
        .bind(name)
        .fetch_optional(pool)
        .await?;

        match row {
            Some(row) => Ok(Some(self.row_to_source(row)?)),
            None => Ok(None),
        }
    }

    /// List policy sources for an organization
    pub async fn list_by_org(&self, org_id: Uuid) -> Result<Vec<PolicySource>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let rows = sqlx::query(
            r#"
            SELECT id, org_id, name, description, source_type, config, sync_interval_secs,
                   sync_status, last_sync_at, last_sync_error, last_sync_commit, is_enabled,
                   created_at, updated_at
            FROM policy_sources
            WHERE org_id = $1
            ORDER BY name ASC
            "#,
        )
        .bind(org_id.to_string())
        .fetch_all(pool)
        .await?;

        let mut sources = Vec::with_capacity(rows.len());
        for row in rows {
            sources.push(self.row_to_source(row)?);
        }

        Ok(sources)
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
    ) -> Result<Vec<PolicySource>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let rows = if let Some((created_at, id)) = after {
            sqlx::query(
                r#"
                SELECT id, org_id, name, description, source_type, config, sync_interval_secs,
                       sync_status, last_sync_at, last_sync_error, last_sync_commit, is_enabled,
                       created_at, updated_at
                FROM policy_sources
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
                SELECT id, org_id, name, description, source_type, config, sync_interval_secs,
                       sync_status, last_sync_at, last_sync_error, last_sync_commit, is_enabled,
                       created_at, updated_at
                FROM policy_sources
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

        let mut sources = Vec::with_capacity(rows.len());
        for row in rows {
            sources.push(self.row_to_source(row)?);
        }
        Ok(sources)
    }

    /// List sources that need syncing
    pub async fn list_due_for_sync(&self) -> Result<Vec<PolicySource>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        // Get all enabled sources with sync_interval > 0
        let rows = sqlx::query(
            r#"
            SELECT id, org_id, name, description, source_type, config, sync_interval_secs,
                   sync_status, last_sync_at, last_sync_error, last_sync_commit, is_enabled,
                   created_at, updated_at
            FROM policy_sources
            WHERE is_enabled = 1
              AND sync_interval_secs > 0
              AND sync_status != $1
            ORDER BY last_sync_at ASC NULLS FIRST
            "#,
        )
        .bind(SyncStatus::Syncing.to_string())
        .fetch_all(pool)
        .await?;

        let mut sources = Vec::with_capacity(rows.len());
        for row in rows {
            let source = self.row_to_source(row)?;
            if source.is_sync_due() {
                sources.push(source);
            }
        }

        Ok(sources)
    }

    /// Find enabled git sources tracking `repo_full_name` (e.g. "owner/repo"),
    /// optionally filtered by `provider` (Plan 09 Step 7). Used by the webhook
    /// handler to resolve which source(s) a push event targets. Matches on the
    /// git config's `repo_full_name`, so a source created without it (legacy
    /// PAT-in-URL) is not webhook-resolvable — that's intended, those predate
    /// the App/webhook model.
    pub async fn find_git_sources_by_repo(
        &self,
        provider: Option<&str>,
        repo_full_name: &str,
    ) -> Result<Vec<PolicySource>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let rows = sqlx::query(
            r#"
            SELECT id, org_id, name, description, source_type, config, sync_interval_secs,
                   sync_status, last_sync_at, last_sync_error, last_sync_commit, is_enabled,
                   created_at, updated_at
            FROM policy_sources
            WHERE is_enabled = 1 AND source_type = 'git'
            "#,
        )
        .fetch_all(pool)
        .await?;

        let mut matches = Vec::new();
        for row in rows {
            let source = self.row_to_source(row)?;
            let Some(cfg) = source.git_config() else {
                continue;
            };
            let repo_matches = cfg.repo_full_name.as_deref() == Some(repo_full_name);
            let provider_matches = match (provider, cfg.provider.as_deref()) {
                (Some(want), Some(have)) => want.eq_ignore_ascii_case(have),
                (Some(_), None) => false,
                (None, _) => true,
            };
            if repo_matches && provider_matches {
                matches.push(source);
            }
        }
        Ok(matches)
    }

    /// Update policy source
    pub async fn update(&self, id: Uuid, input: UpdatePolicySource) -> Result<bool, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let mut updates: Vec<String> = Vec::new();
        let mut bindings: Vec<String> = Vec::new();

        if let Some(name) = &input.name {
            updates.push("name = ?".to_string());
            bindings.push(name.clone());
        }

        if let Some(description) = &input.description {
            updates.push("description = ?".to_string());
            bindings.push(description.clone());
        }

        if let Some(config) = &input.config {
            updates.push("config = ?".to_string());
            bindings.push(serde_json::to_string(config).unwrap_or_else(|_| "{}".to_string()));
        }

        // Numeric columns are inlined: binding a text param into an INTEGER
        // column works on SQLite (type affinity) but is a type error on
        // PostgreSQL, and this builder's bindings are all strings. Both
        // values are numeric literals, so no injection surface.
        if let Some(sync_interval) = input.sync_interval_secs {
            updates.push(format!("sync_interval_secs = {}", sync_interval));
        }

        if let Some(is_enabled) = input.is_enabled {
            updates.push(format!("is_enabled = {}", if is_enabled { 1 } else { 0 }));
        }

        if updates.is_empty() {
            return Ok(false);
        }

        updates.push("updated_at = ?".to_string());
        bindings.push(Utc::now().to_rfc3339());

        let sql = crate::db::numbered_placeholders(&format!(
            "UPDATE policy_sources SET {} WHERE id = ?",
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

    /// Update sync status
    pub async fn update_sync_status(
        &self,
        id: Uuid,
        status: SyncStatus,
        error: Option<&str>,
        commit: Option<&str>,
    ) -> Result<bool, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let now = Utc::now().to_rfc3339();

        let result = sqlx::query(
            r#"
            UPDATE policy_sources
            SET sync_status = $1, last_sync_at = $2, last_sync_error = $3, last_sync_commit = $4, updated_at = $5
            WHERE id = $6
            "#,
        )
        .bind(status.to_string())
        .bind(&now)
        .bind(error)
        .bind(commit)
        .bind(&now)
        .bind(id.to_string())
        .execute(pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Delete policy source
    pub async fn delete(&self, id: Uuid) -> Result<bool, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let result = sqlx::query("DELETE FROM policy_sources WHERE id = $1")
            .bind(id.to_string())
            .execute(pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Convert database row to PolicySource
    fn row_to_source(&self, row: sqlx::any::AnyRow) -> Result<PolicySource, DatabaseError> {
        let id_str: String = row.get("id");
        let id = Uuid::parse_str(&id_str)
            .map_err(|e| DatabaseError::Config(format!("Invalid UUID: {}", e)))?;

        let org_id_str: String = row.get("org_id");
        let org_id = Uuid::parse_str(&org_id_str)
            .map_err(|e| DatabaseError::Config(format!("Invalid org UUID: {}", e)))?;

        let source_type_str: String = row.get("source_type");
        let source_type = source_type_str
            .parse::<SourceType>()
            .unwrap_or(SourceType::Git);

        let config_str: String = row.get("config");
        let config = serde_json::from_str(&config_str).unwrap_or_else(|_| serde_json::json!({}));

        let sync_status_str: String = row.get("sync_status");
        let sync_status = sync_status_str
            .parse::<SyncStatus>()
            .unwrap_or(SyncStatus::Pending);

        let sync_interval: i64 = row.get("sync_interval_secs");

        let last_sync_at: Option<String> = row.get("last_sync_at");
        let last_sync_at = last_sync_at
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
            .map(|dt| dt.with_timezone(&Utc));

        let created_at_str: String = row.get("created_at");
        let created_at = chrono::DateTime::parse_from_rfc3339(&created_at_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());

        let updated_at_str: String = row.get("updated_at");
        let updated_at = chrono::DateTime::parse_from_rfc3339(&updated_at_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());

        let is_enabled: i64 = row.get("is_enabled");
        let is_enabled = is_enabled != 0;

        Ok(PolicySource {
            id,
            org_id,
            name: row.get("name"),
            description: row.get("description"),
            source_type,
            config,
            sync_interval_secs: sync_interval as u32,
            sync_status,
            last_sync_at,
            last_sync_error: row.get("last_sync_error"),
            last_sync_commit: row.get("last_sync_commit"),
            is_enabled,
            created_at,
            updated_at,
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
    async fn test_create_git_source() {
        let (_temp_dir, db) = setup_db().await;
        let org_id = create_test_org(&db).await;
        let repo = PolicySourceRepository::new(&db);

        let input = CreatePolicySource {
            name: "main-policies".to_string(),
            description: Some("Main policy repository".to_string()),
            source_type: SourceType::Git,
            config: serde_json::json!({
                "url": "https://github.com/example/policies.git",
                "branch": "main",
                "patterns": ["**/*.reap"]
            }),
            sync_interval_secs: 300,
        };

        let source = repo.create(org_id, input).await.unwrap();
        assert_eq!(source.name, "main-policies");
        assert_eq!(source.source_type, SourceType::Git);
        assert_eq!(source.sync_status, SyncStatus::Pending);
        assert!(source.is_enabled);
    }

    #[tokio::test]
    async fn test_create_api_source() {
        let (_temp_dir, db) = setup_db().await;
        let org_id = create_test_org(&db).await;
        let repo = PolicySourceRepository::new(&db);

        let input = CreatePolicySource {
            name: "policy-api".to_string(),
            description: None,
            source_type: SourceType::Api,
            config: serde_json::json!({
                "url": "https://api.example.com/policies",
                "method": "GET",
                "api_key_header": "X-API-Key",
                "jsonpath": "$.policies[*]"
            }),
            sync_interval_secs: 60,
        };

        let source = repo.create(org_id, input).await.unwrap();
        assert_eq!(source.name, "policy-api");
        assert_eq!(source.source_type, SourceType::Api);
    }

    #[tokio::test]
    async fn test_update_sync_status() {
        let (_temp_dir, db) = setup_db().await;
        let org_id = create_test_org(&db).await;
        let repo = PolicySourceRepository::new(&db);

        let input = CreatePolicySource {
            name: "test-source".to_string(),
            description: None,
            source_type: SourceType::Git,
            config: serde_json::json!({"url": "https://example.com/repo.git"}),
            sync_interval_secs: 300,
        };

        let source = repo.create(org_id, input).await.unwrap();

        // Update to syncing
        repo.update_sync_status(source.id, SyncStatus::Syncing, None, None)
            .await
            .unwrap();

        let updated = repo.get_by_id(source.id).await.unwrap().unwrap();
        assert_eq!(updated.sync_status, SyncStatus::Syncing);

        // Update to success with commit
        repo.update_sync_status(source.id, SyncStatus::Success, None, Some("abc123"))
            .await
            .unwrap();

        let updated = repo.get_by_id(source.id).await.unwrap().unwrap();
        assert_eq!(updated.sync_status, SyncStatus::Success);
        assert_eq!(updated.last_sync_commit, Some("abc123".to_string()));
    }

    #[tokio::test]
    async fn test_list_sources() {
        let (_temp_dir, db) = setup_db().await;
        let org_id = create_test_org(&db).await;
        let repo = PolicySourceRepository::new(&db);

        for i in 0..3 {
            let input = CreatePolicySource {
                name: format!("source-{}", i),
                description: None,
                source_type: SourceType::Git,
                config: serde_json::json!({"url": format!("https://example.com/repo{}.git", i)}),
                sync_interval_secs: 300,
            };
            repo.create(org_id, input).await.unwrap();
        }

        let sources = repo.list_by_org(org_id).await.unwrap();
        assert_eq!(sources.len(), 3);
    }
}
