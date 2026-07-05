//! Agent deployment repository
//!
//! Database operations for tracking per-agent deployment status.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::db::{Database, DatabaseError};
use crate::domain::agent_deployment::{
    AgentDeployment, AgentDeploymentStatus, DeploymentSummary, RollbackConfig,
};

/// Repository for agent deployment operations
pub struct AgentDeploymentRepository<'a> {
    db: &'a Database,
}

impl<'a> AgentDeploymentRepository<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// Create a new agent deployment record
    pub async fn create(&self, deployment: &AgentDeployment) -> Result<(), DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or(DatabaseError::Config("No database pool".to_string()))?;

        sqlx::query(
            r#"
            INSERT INTO agent_deployments
                (id, agent_id, bundle_id, rollout_id, status, error_message, deployed_at, acknowledged_at, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            "#,
        )
        .bind(deployment.id.to_string())
        .bind(deployment.agent_id.to_string())
        .bind(deployment.bundle_id.to_string())
        .bind(deployment.rollout_id.map(|id| id.to_string()))
        .bind(deployment.status.to_string())
        .bind(&deployment.error_message)
        .bind(deployment.deployed_at.map(|dt| dt.to_rfc3339()))
        .bind(deployment.acknowledged_at.map(|dt| dt.to_rfc3339()))
        .bind(deployment.created_at.to_rfc3339())
        .execute(pool)
        .await?;

        Ok(())
    }

    /// Get deployment by ID
    pub async fn get_by_id(&self, id: Uuid) -> Result<Option<AgentDeployment>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or(DatabaseError::Config("No database pool".to_string()))?;

        let row: Option<(String, String, String, Option<String>, String, Option<String>, Option<String>, Option<String>, String)> =
            sqlx::query_as(
                r#"
                SELECT id, agent_id, bundle_id, rollout_id, status, error_message, deployed_at, acknowledged_at, created_at
                FROM agent_deployments WHERE id = $1
                "#,
            )
            .bind(id.to_string())
            .fetch_optional(pool)
            .await?;

        row.map(|r| self.row_to_deployment(r)).transpose()
    }

    /// Get deployments for a rollout
    pub async fn get_by_rollout(
        &self,
        rollout_id: Uuid,
    ) -> Result<Vec<AgentDeployment>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or(DatabaseError::Config("No database pool".to_string()))?;

        let rows: Vec<(String, String, String, Option<String>, String, Option<String>, Option<String>, Option<String>, String)> =
            sqlx::query_as(
                r#"
                SELECT id, agent_id, bundle_id, rollout_id, status, error_message, deployed_at, acknowledged_at, created_at
                FROM agent_deployments WHERE rollout_id = $1
                ORDER BY created_at
                "#,
            )
            .bind(rollout_id.to_string())
            .fetch_all(pool)
            .await?;

        rows.into_iter()
            .map(|r| self.row_to_deployment(r))
            .collect()
    }

    /// Get latest deployment for an agent
    pub async fn get_latest_for_agent(
        &self,
        agent_id: Uuid,
    ) -> Result<Option<AgentDeployment>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or(DatabaseError::Config("No database pool".to_string()))?;

        let row: Option<(String, String, String, Option<String>, String, Option<String>, Option<String>, Option<String>, String)> =
            sqlx::query_as(
                r#"
                SELECT id, agent_id, bundle_id, rollout_id, status, error_message, deployed_at, acknowledged_at, created_at
                FROM agent_deployments WHERE agent_id = $1
                ORDER BY created_at DESC LIMIT 1
                "#,
            )
            .bind(agent_id.to_string())
            .fetch_optional(pool)
            .await?;

        row.map(|r| self.row_to_deployment(r)).transpose()
    }

    /// Get the most recent deployment record for a specific agent + bundle.
    pub async fn get_latest_for_agent_bundle(
        &self,
        agent_id: Uuid,
        bundle_id: Uuid,
    ) -> Result<Option<AgentDeployment>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or(DatabaseError::Config("No database pool".to_string()))?;

        let row: Option<(String, String, String, Option<String>, String, Option<String>, Option<String>, Option<String>, String)> =
            sqlx::query_as(
                r#"
                SELECT id, agent_id, bundle_id, rollout_id, status, error_message, deployed_at, acknowledged_at, created_at
                FROM agent_deployments WHERE agent_id = $1 AND bundle_id = $2
                ORDER BY created_at DESC LIMIT 1
                "#,
            )
            .bind(agent_id.to_string())
            .bind(bundle_id.to_string())
            .fetch_optional(pool)
            .await?;

        row.map(|r| self.row_to_deployment(r)).transpose()
    }

    /// Update deployment status
    pub async fn update_status(
        &self,
        id: Uuid,
        status: AgentDeploymentStatus,
        error_message: Option<&str>,
    ) -> Result<(), DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or(DatabaseError::Config("No database pool".to_string()))?;

        let deployed_at = if status == AgentDeploymentStatus::Deployed {
            Some(Utc::now().to_rfc3339())
        } else {
            None
        };

        sqlx::query(
            r#"
            UPDATE agent_deployments
            SET status = $1, error_message = $2, deployed_at = COALESCE($3, deployed_at)
            WHERE id = $4
            "#,
        )
        .bind(status.to_string())
        .bind(error_message)
        .bind(deployed_at)
        .bind(id.to_string())
        .execute(pool)
        .await?;

        Ok(())
    }

    /// Mark deployment as acknowledged
    pub async fn acknowledge(&self, id: Uuid) -> Result<(), DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or(DatabaseError::Config("No database pool".to_string()))?;

        sqlx::query("UPDATE agent_deployments SET acknowledged_at = $1 WHERE id = $2")
            .bind(Utc::now().to_rfc3339())
            .bind(id.to_string())
            .execute(pool)
            .await?;

        Ok(())
    }

    /// Get deployment summary for a rollout
    pub async fn get_summary(&self, rollout_id: Uuid) -> Result<DeploymentSummary, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or(DatabaseError::Config("No database pool".to_string()))?;

        let row: (i64, i64, i64, i64, i64, i64) = sqlx::query_as(
            r#"
            SELECT
                COUNT(*) as total,
                SUM(CASE WHEN status = 'pending' THEN 1 ELSE 0 END) as pending,
                SUM(CASE WHEN status = 'deploying' THEN 1 ELSE 0 END) as deploying,
                SUM(CASE WHEN status = 'deployed' THEN 1 ELSE 0 END) as deployed,
                SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END) as failed,
                SUM(CASE WHEN acknowledged_at IS NOT NULL THEN 1 ELSE 0 END) as acknowledged
            FROM agent_deployments WHERE rollout_id = $1
            "#,
        )
        .bind(rollout_id.to_string())
        .fetch_one(pool)
        .await?;

        Ok(DeploymentSummary {
            total_agents: row.0 as u32,
            pending: row.1 as u32,
            deploying: row.2 as u32,
            deployed: row.3 as u32,
            failed: row.4 as u32,
            acknowledged: row.5 as u32,
        })
    }

    /// Get failed deployments for a rollout
    pub async fn get_failed(
        &self,
        rollout_id: Uuid,
    ) -> Result<Vec<AgentDeployment>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or(DatabaseError::Config("No database pool".to_string()))?;

        let rows: Vec<(String, String, String, Option<String>, String, Option<String>, Option<String>, Option<String>, String)> =
            sqlx::query_as(
                r#"
                SELECT id, agent_id, bundle_id, rollout_id, status, error_message, deployed_at, acknowledged_at, created_at
                FROM agent_deployments WHERE rollout_id = $1 AND status = 'failed'
                ORDER BY created_at
                "#,
            )
            .bind(rollout_id.to_string())
            .fetch_all(pool)
            .await?;

        rows.into_iter()
            .map(|r| self.row_to_deployment(r))
            .collect()
    }

    fn row_to_deployment(
        &self,
        row: (
            String,
            String,
            String,
            Option<String>,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            String,
        ),
    ) -> Result<AgentDeployment, DatabaseError> {
        Ok(AgentDeployment {
            id: Uuid::parse_str(&row.0).map_err(|e| DatabaseError::Config(e.to_string()))?,
            agent_id: Uuid::parse_str(&row.1).map_err(|e| DatabaseError::Config(e.to_string()))?,
            bundle_id: Uuid::parse_str(&row.2).map_err(|e| DatabaseError::Config(e.to_string()))?,
            rollout_id: row.3.as_ref().and_then(|s| Uuid::parse_str(s).ok()),
            status: row
                .4
                .parse()
                .map_err(|e: String| DatabaseError::Config(e))?,
            error_message: row.5,
            deployed_at: row.6.as_ref().and_then(|s| {
                DateTime::parse_from_rfc3339(s)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            }),
            acknowledged_at: row.7.as_ref().and_then(|s| {
                DateTime::parse_from_rfc3339(s)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            }),
            created_at: DateTime::parse_from_rfc3339(&row.8)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|e| DatabaseError::Config(e.to_string()))?,
        })
    }
}

/// Repository for rollback configuration
pub struct RollbackConfigRepository<'a> {
    db: &'a Database,
}

impl<'a> RollbackConfigRepository<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// Get rollback config for org/namespace
    pub async fn get(
        &self,
        org_id: Uuid,
        namespace_id: Option<Uuid>,
    ) -> Result<Option<RollbackConfig>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or(DatabaseError::Config("No database pool".to_string()))?;

        let row: Option<(
            String,
            String,
            Option<String>,
            i32,
            f64,
            i32,
            i32,
            String,
            String,
        )> = if let Some(ns_id) = namespace_id {
            sqlx::query_as(
                r#"
                SELECT id, org_id, namespace_id, is_enabled, error_rate_threshold, window_seconds, min_requests, created_at, updated_at
                FROM rollback_configs WHERE org_id = $1 AND namespace_id = $2
                "#,
            )
            .bind(org_id.to_string())
            .bind(ns_id.to_string())
            .fetch_optional(pool)
            .await?
        } else {
            sqlx::query_as(
                r#"
                SELECT id, org_id, namespace_id, is_enabled, error_rate_threshold, window_seconds, min_requests, created_at, updated_at
                FROM rollback_configs WHERE org_id = $1 AND namespace_id IS NULL
                "#,
            )
            .bind(org_id.to_string())
            .fetch_optional(pool)
            .await?
        };

        row.map(|r| self.row_to_config(r)).transpose()
    }

    /// Create or update rollback config
    pub async fn upsert(&self, config: &RollbackConfig) -> Result<(), DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or(DatabaseError::Config("No database pool".to_string()))?;

        sqlx::query(
            r#"
            INSERT INTO rollback_configs
                (id, org_id, namespace_id, is_enabled, error_rate_threshold, window_seconds, min_requests, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            ON CONFLICT(org_id, namespace_id) DO UPDATE SET
                is_enabled = excluded.is_enabled,
                error_rate_threshold = excluded.error_rate_threshold,
                window_seconds = excluded.window_seconds,
                min_requests = excluded.min_requests,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(config.id.to_string())
        .bind(config.org_id.to_string())
        .bind(config.namespace_id.map(|id| id.to_string()))
        .bind(config.is_enabled as i32)
        .bind(config.error_rate_threshold)
        .bind(config.window_seconds as i32)
        .bind(config.min_requests as i32)
        .bind(config.created_at.to_rfc3339())
        .bind(config.updated_at.to_rfc3339())
        .execute(pool)
        .await?;

        Ok(())
    }

    fn row_to_config(
        &self,
        row: (
            String,
            String,
            Option<String>,
            i32,
            f64,
            i32,
            i32,
            String,
            String,
        ),
    ) -> Result<RollbackConfig, DatabaseError> {
        Ok(RollbackConfig {
            id: Uuid::parse_str(&row.0).map_err(|e| DatabaseError::Config(e.to_string()))?,
            org_id: Uuid::parse_str(&row.1).map_err(|e| DatabaseError::Config(e.to_string()))?,
            namespace_id: row.2.as_ref().and_then(|s| Uuid::parse_str(s).ok()),
            is_enabled: row.3 != 0,
            error_rate_threshold: row.4,
            window_seconds: row.5 as u32,
            min_requests: row.6 as u32,
            created_at: DateTime::parse_from_rfc3339(&row.7)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|e| DatabaseError::Config(e.to_string()))?,
            updated_at: DateTime::parse_from_rfc3339(&row.8)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|e| DatabaseError::Config(e.to_string()))?,
        })
    }
}
