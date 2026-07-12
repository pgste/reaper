//! Environment repository (Plan 10).

use chrono::Utc;
use sqlx::Row;
use uuid::Uuid;

use crate::db::{Database, DatabaseError};
use crate::domain::environment::{
    ApprovalPolicy, ChangeWindows, CreateEnvironment, Environment, UpdateEnvironment,
};

pub struct EnvironmentRepository<'a> {
    db: &'a Database,
}

impl<'a> EnvironmentRepository<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    pub async fn create(
        &self,
        org_id: Uuid,
        input: CreateEnvironment,
    ) -> Result<Environment, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let id = Uuid::new_v4();
        let now = Utc::now();
        let approval_json = serde_json::to_string(&input.approval_policy)
            .map_err(|e| DatabaseError::Config(format!("approval_policy: {e}")))?;
        let windows_json = serde_json::to_string(&input.change_windows)
            .map_err(|e| DatabaseError::Config(format!("change_windows: {e}")))?;

        sqlx::query(
            r#"
            INSERT INTO environments
                (id, org_id, name, tier_order, namespace_id, data_plane_ref,
                 approval_policy, change_windows, is_active, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 1, $9, $10)
            "#,
        )
        .bind(id.to_string())
        .bind(org_id.to_string())
        .bind(&input.name)
        .bind(input.tier_order)
        .bind(input.namespace_id.to_string())
        .bind(&input.data_plane_ref)
        .bind(&approval_json)
        .bind(&windows_json)
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .execute(pool)
        .await?;

        Ok(Environment {
            id,
            org_id,
            name: input.name,
            tier_order: input.tier_order,
            namespace_id: input.namespace_id,
            data_plane_ref: input.data_plane_ref,
            approval_policy: input.approval_policy,
            change_windows: input.change_windows,
            is_active: true,
            created_at: now,
            updated_at: now,
        })
    }

    pub async fn get_by_id(&self, id: Uuid) -> Result<Option<Environment>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;
        let row = sqlx::query(SELECT_COLUMNS_WHERE_ID)
            .bind(id.to_string())
            .fetch_optional(pool)
            .await?;
        row.map(|r| self.row_to_env(r)).transpose()
    }

    /// Resolve an environment by id or name within an org.
    pub async fn get_by_ref(
        &self,
        org_id: Uuid,
        env_ref: &str,
    ) -> Result<Option<Environment>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;
        let row = sqlx::query(
            r#"
            SELECT id, org_id, name, tier_order, namespace_id, data_plane_ref,
                   approval_policy, change_windows, is_active, created_at, updated_at
            FROM environments
            WHERE org_id = $1 AND (id = $2 OR name = $2)
            "#,
        )
        .bind(org_id.to_string())
        .bind(env_ref)
        .fetch_optional(pool)
        .await?;
        row.map(|r| self.row_to_env(r)).transpose()
    }

    pub async fn list_by_org(&self, org_id: Uuid) -> Result<Vec<Environment>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;
        let rows = sqlx::query(
            r#"
            SELECT id, org_id, name, tier_order, namespace_id, data_plane_ref,
                   approval_policy, change_windows, is_active, created_at, updated_at
            FROM environments
            WHERE org_id = $1
            ORDER BY tier_order ASC, name ASC
            "#,
        )
        .bind(org_id.to_string())
        .fetch_all(pool)
        .await?;
        rows.into_iter().map(|r| self.row_to_env(r)).collect()
    }

    /// Whether a namespace is already bound to an environment (for the 409 on
    /// duplicate binding). Excludes `exclude_env` so an update to the same env
    /// isn't seen as a conflict with itself.
    pub async fn namespace_is_bound(
        &self,
        namespace_id: Uuid,
        exclude_env: Option<Uuid>,
    ) -> Result<bool, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;
        let row =
            sqlx::query("SELECT id FROM environments WHERE namespace_id = $1 AND id != $2 LIMIT 1")
                .bind(namespace_id.to_string())
                .bind(exclude_env.map(|e| e.to_string()).unwrap_or_default())
                .fetch_optional(pool)
                .await?;
        Ok(row.is_some())
    }

    pub async fn update(
        &self,
        id: Uuid,
        input: UpdateEnvironment,
    ) -> Result<Option<Environment>, DatabaseError> {
        let Some(current) = self.get_by_id(id).await? else {
            return Ok(None);
        };
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let name = input.name.unwrap_or(current.name);
        let tier_order = input.tier_order.unwrap_or(current.tier_order);
        let data_plane_ref = input.data_plane_ref.or(current.data_plane_ref);
        let approval = input.approval_policy.unwrap_or(current.approval_policy);
        let windows = input.change_windows.unwrap_or(current.change_windows);
        let is_active = input.is_active.unwrap_or(current.is_active);
        let now = Utc::now();
        let approval_json = serde_json::to_string(&approval)
            .map_err(|e| DatabaseError::Config(format!("approval_policy: {e}")))?;
        let windows_json = serde_json::to_string(&windows)
            .map_err(|e| DatabaseError::Config(format!("change_windows: {e}")))?;

        sqlx::query(
            r#"
            UPDATE environments
            SET name = $1, tier_order = $2, data_plane_ref = $3,
                approval_policy = $4, change_windows = $5, is_active = $6, updated_at = $7
            WHERE id = $8
            "#,
        )
        .bind(&name)
        .bind(tier_order)
        .bind(&data_plane_ref)
        .bind(&approval_json)
        .bind(&windows_json)
        .bind(is_active as i64)
        .bind(now.to_rfc3339())
        .bind(id.to_string())
        .execute(pool)
        .await?;

        Ok(Some(Environment {
            id,
            org_id: current.org_id,
            name,
            tier_order,
            namespace_id: current.namespace_id,
            data_plane_ref,
            approval_policy: approval,
            change_windows: windows,
            is_active,
            created_at: current.created_at,
            updated_at: now,
        }))
    }

    pub async fn delete(&self, id: Uuid) -> Result<bool, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;
        let result = sqlx::query("DELETE FROM environments WHERE id = $1")
            .bind(id.to_string())
            .execute(pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    fn row_to_env(&self, row: sqlx::any::AnyRow) -> Result<Environment, DatabaseError> {
        let id: String = row.get("id");
        let org_id: String = row.get("org_id");
        let namespace_id: String = row.get("namespace_id");
        let approval_raw: String = row.get("approval_policy");
        let windows_raw: String = row.get("change_windows");
        let is_active: i64 = row.get("is_active");
        let created_at: String = row.get("created_at");
        let updated_at: String = row.get("updated_at");

        let approval_policy: ApprovalPolicy =
            serde_json::from_str(&approval_raw).unwrap_or_default();
        let change_windows: ChangeWindows = serde_json::from_str(&windows_raw).unwrap_or_default();

        Ok(Environment {
            id: id
                .parse()
                .map_err(|e| DatabaseError::Config(format!("Invalid UUID: {e}")))?,
            org_id: org_id
                .parse()
                .map_err(|e| DatabaseError::Config(format!("Invalid UUID: {e}")))?,
            name: row.get("name"),
            tier_order: row.get("tier_order"),
            namespace_id: namespace_id
                .parse()
                .map_err(|e| DatabaseError::Config(format!("Invalid UUID: {e}")))?,
            data_plane_ref: row.get("data_plane_ref"),
            approval_policy,
            change_windows,
            is_active: is_active != 0,
            created_at: chrono::DateTime::parse_from_rfc3339(&created_at)
                .map_err(|e| DatabaseError::Config(format!("Invalid timestamp: {e}")))?
                .with_timezone(&Utc),
            updated_at: chrono::DateTime::parse_from_rfc3339(&updated_at)
                .map_err(|e| DatabaseError::Config(format!("Invalid timestamp: {e}")))?
                .with_timezone(&Utc),
        })
    }
}

const SELECT_COLUMNS_WHERE_ID: &str = r#"
    SELECT id, org_id, name, tier_order, namespace_id, data_plane_ref,
           approval_policy, change_windows, is_active, created_at, updated_at
    FROM environments
    WHERE id = $1
"#;
