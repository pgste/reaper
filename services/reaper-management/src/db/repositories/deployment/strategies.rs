//! Deployment strategy repository operations

use chrono::Utc;
use uuid::Uuid;

use crate::db::{Database, DatabaseError};
use crate::domain::deployment::{CreateDeploymentStrategy, DeploymentStrategy};

use super::row_conversions::row_to_strategy;

/// Deployment strategy repository operations
pub struct StrategyOps<'a> {
    pub(super) db: &'a Database,
}

impl<'a> StrategyOps<'a> {
    /// Create a new deployment strategy
    pub async fn create(
        &self,
        org_id: Uuid,
        input: &CreateDeploymentStrategy,
    ) -> Result<DeploymentStrategy, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let id = Uuid::new_v4();
        let now = Utc::now();
        let config_json = serde_json::to_string(&input.config)
            .map_err(|e| DatabaseError::Config(format!("Failed to serialize config: {}", e)))?;

        // If this is set as default, unset other defaults first
        if input.is_default {
            self.unset_defaults(org_id, input.namespace_id).await?;
        }

        let sql = r#"
            INSERT INTO deployment_strategies (id, org_id, namespace_id, name, strategy_type, config, is_default, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        "#;

        sqlx::query(sql)
            .bind(id.to_string())
            .bind(org_id.to_string())
            .bind(input.namespace_id.map(|id| id.to_string()))
            .bind(&input.name)
            .bind(input.strategy_type.to_string())
            .bind(&config_json)
            .bind(input.is_default as i32)
            .bind(now.to_rfc3339())
            .bind(now.to_rfc3339())
            .execute(pool)
            .await?;

        self.get_by_id(id)
            .await?
            .ok_or_else(|| DatabaseError::NotFound("Strategy not found after creation".to_string()))
    }

    /// Get a deployment strategy by ID
    pub async fn get_by_id(&self, id: Uuid) -> Result<Option<DeploymentStrategy>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let sql = r#"
            SELECT id, org_id, namespace_id, name, strategy_type, config, is_default, created_at, updated_at
            FROM deployment_strategies
            WHERE id = $1
        "#;

        let row = sqlx::query(sql)
            .bind(id.to_string())
            .fetch_optional(pool)
            .await?;

        row.map(|r| row_to_strategy(&r)).transpose()
    }

    /// List deployment strategies for an organization
    pub async fn list(
        &self,
        org_id: Uuid,
        namespace_id: Option<Uuid>,
        limit: i64,
    ) -> Result<Vec<DeploymentStrategy>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        // Bounded cap so the list is never unbounded (round-3 Plan 06 §4.2).
        let rows = if let Some(ns_id) = namespace_id {
            let sql = r#"
                SELECT id, org_id, namespace_id, name, strategy_type, config, is_default, created_at, updated_at
                FROM deployment_strategies
                WHERE org_id = $1 AND (namespace_id = $2 OR namespace_id IS NULL)
                ORDER BY is_default DESC, name ASC
                LIMIT $3
            "#;
            sqlx::query(sql)
                .bind(org_id.to_string())
                .bind(ns_id.to_string())
                .bind(limit)
                .fetch_all(pool)
                .await?
        } else {
            let sql = r#"
                SELECT id, org_id, namespace_id, name, strategy_type, config, is_default, created_at, updated_at
                FROM deployment_strategies
                WHERE org_id = $1
                ORDER BY is_default DESC, name ASC
                LIMIT $2
            "#;
            sqlx::query(sql)
                .bind(org_id.to_string())
                .bind(limit)
                .fetch_all(pool)
                .await?
        };

        rows.iter().map(row_to_strategy).collect()
    }

    /// Get the default strategy for a namespace (or org-wide)
    pub async fn get_default(
        &self,
        org_id: Uuid,
        namespace_id: Option<Uuid>,
    ) -> Result<Option<DeploymentStrategy>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        // First try namespace-specific default, then org-wide default
        let row = if let Some(ns_id) = namespace_id {
            let sql = r#"
                SELECT id, org_id, namespace_id, name, strategy_type, config, is_default, created_at, updated_at
                FROM deployment_strategies
                WHERE org_id = $1 AND is_default = 1 AND (namespace_id = $2 OR namespace_id IS NULL)
                ORDER BY namespace_id DESC NULLS LAST
                LIMIT 1
            "#;
            sqlx::query(sql)
                .bind(org_id.to_string())
                .bind(ns_id.to_string())
                .fetch_optional(pool)
                .await?
        } else {
            let sql = r#"
                SELECT id, org_id, namespace_id, name, strategy_type, config, is_default, created_at, updated_at
                FROM deployment_strategies
                WHERE org_id = $1 AND is_default = 1 AND namespace_id IS NULL
                LIMIT 1
            "#;
            sqlx::query(sql)
                .bind(org_id.to_string())
                .fetch_optional(pool)
                .await?
        };

        row.map(|r| row_to_strategy(&r)).transpose()
    }

    /// Delete a deployment strategy
    pub async fn delete(&self, id: Uuid) -> Result<(), DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let sql = "DELETE FROM deployment_strategies WHERE id = $1";
        let result = sqlx::query(sql).bind(id.to_string()).execute(pool).await?;

        if result.rows_affected() == 0 {
            return Err(DatabaseError::NotFound(format!(
                "Strategy {} not found",
                id
            )));
        }

        Ok(())
    }

    /// Unset default flag for strategies in scope
    async fn unset_defaults(
        &self,
        org_id: Uuid,
        namespace_id: Option<Uuid>,
    ) -> Result<(), DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        if let Some(ns_id) = namespace_id {
            let sql = "UPDATE deployment_strategies SET is_default = 0 WHERE org_id = $1 AND namespace_id = $2";
            sqlx::query(sql)
                .bind(org_id.to_string())
                .bind(ns_id.to_string())
                .execute(pool)
                .await?;
        } else {
            let sql = "UPDATE deployment_strategies SET is_default = 0 WHERE org_id = $1 AND namespace_id IS NULL";
            sqlx::query(sql)
                .bind(org_id.to_string())
                .execute(pool)
                .await?;
        }

        Ok(())
    }
}
