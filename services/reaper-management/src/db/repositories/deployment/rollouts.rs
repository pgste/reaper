//! Rollout repository operations

use chrono::Utc;
use uuid::Uuid;

use crate::db::{Database, DatabaseError};
use crate::domain::deployment::{Rollout, RolloutStatus, StartRollout};

use super::row_conversions::row_to_rollout;

/// Rollout repository operations
pub struct RolloutOps<'a> {
    pub(super) db: &'a Database,
}

impl<'a> RolloutOps<'a> {
    /// Create a new rollout
    pub async fn create(
        &self,
        input: &StartRollout,
        target_agent_count: u32,
    ) -> Result<Rollout, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let id = Uuid::new_v4();
        let now = Utc::now();

        let sql = r#"
            INSERT INTO rollouts (id, bundle_id, strategy_id, namespace_id, status, current_wave,
                                  target_agent_count, deployed_agent_count, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, 0, $6, 0, $7, $8)
        "#;

        sqlx::query(sql)
            .bind(id.to_string())
            .bind(input.bundle_id.to_string())
            .bind(input.strategy_id.map(|id| id.to_string()))
            .bind(input.namespace_id.map(|id| id.to_string()))
            .bind(RolloutStatus::Pending.to_string())
            .bind(target_agent_count as i32)
            .bind(now.to_rfc3339())
            .bind(now.to_rfc3339())
            .execute(pool)
            .await?;

        self.get_by_id(id)
            .await?
            .ok_or_else(|| DatabaseError::NotFound("Rollout not found after creation".to_string()))
    }

    /// Get a rollout by ID
    pub async fn get_by_id(&self, id: Uuid) -> Result<Option<Rollout>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let sql = r#"
            SELECT id, bundle_id, strategy_id, namespace_id, status, current_wave,
                   target_agent_count, deployed_agent_count, started_at, completed_at,
                   error, created_at, updated_at
            FROM rollouts
            WHERE id = $1
        "#;

        let row = sqlx::query(sql)
            .bind(id.to_string())
            .fetch_optional(pool)
            .await?;

        row.map(|r| row_to_rollout(&r)).transpose()
    }

    /// Get active rollouts for a bundle
    pub async fn get_active_for_bundle(
        &self,
        bundle_id: Uuid,
    ) -> Result<Vec<Rollout>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let sql = r#"
            SELECT id, bundle_id, strategy_id, namespace_id, status, current_wave,
                   target_agent_count, deployed_agent_count, started_at, completed_at,
                   error, created_at, updated_at
            FROM rollouts
            WHERE bundle_id = $1 AND status NOT IN ('completed', 'failed', 'rolled_back', 'cancelled')
            ORDER BY created_at DESC
        "#;

        let rows = sqlx::query(sql)
            .bind(bundle_id.to_string())
            .fetch_all(pool)
            .await?;

        rows.iter().map(row_to_rollout).collect()
    }

    /// List rollouts for a namespace
    pub async fn list(
        &self,
        org_id: Uuid,
        namespace_id: Option<Uuid>,
        limit: i32,
    ) -> Result<Vec<Rollout>, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let rows = if let Some(ns_id) = namespace_id {
            let sql = r#"
                SELECT r.id, r.bundle_id, r.strategy_id, r.namespace_id, r.status, r.current_wave,
                       r.target_agent_count, r.deployed_agent_count, r.started_at, r.completed_at,
                       r.error, r.created_at, r.updated_at
                FROM rollouts r
                INNER JOIN bundles b ON r.bundle_id = b.id
                WHERE b.org_id = $1 AND r.namespace_id = $2
                ORDER BY r.created_at DESC
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
                SELECT r.id, r.bundle_id, r.strategy_id, r.namespace_id, r.status, r.current_wave,
                       r.target_agent_count, r.deployed_agent_count, r.started_at, r.completed_at,
                       r.error, r.created_at, r.updated_at
                FROM rollouts r
                INNER JOIN bundles b ON r.bundle_id = b.id
                WHERE b.org_id = $1
                ORDER BY r.created_at DESC
                LIMIT $2
            "#;
            sqlx::query(sql)
                .bind(org_id.to_string())
                .bind(limit)
                .fetch_all(pool)
                .await?
        };

        rows.iter().map(row_to_rollout).collect()
    }

    /// Update rollout status
    pub async fn update_status(
        &self,
        id: Uuid,
        status: RolloutStatus,
        error: Option<&str>,
    ) -> Result<Rollout, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let now = Utc::now();

        match status {
            RolloutStatus::InProgress => {
                let sql = r#"
                    UPDATE rollouts
                    SET status = $1, started_at = COALESCE(started_at, $2), updated_at = $3
                    WHERE id = $4
                "#;
                sqlx::query(sql)
                    .bind(status.to_string())
                    .bind(now.to_rfc3339())
                    .bind(now.to_rfc3339())
                    .bind(id.to_string())
                    .execute(pool)
                    .await?;
            }
            RolloutStatus::Completed
            | RolloutStatus::Failed
            | RolloutStatus::RolledBack
            | RolloutStatus::Cancelled => {
                let sql = r#"
                    UPDATE rollouts
                    SET status = $1, completed_at = $2, error = $3, updated_at = $4
                    WHERE id = $5
                "#;
                sqlx::query(sql)
                    .bind(status.to_string())
                    .bind(now.to_rfc3339())
                    .bind(error)
                    .bind(now.to_rfc3339())
                    .bind(id.to_string())
                    .execute(pool)
                    .await?;
            }
            _ => {
                let sql = r#"
                    UPDATE rollouts
                    SET status = $1, error = $2, updated_at = $3
                    WHERE id = $4
                "#;
                sqlx::query(sql)
                    .bind(status.to_string())
                    .bind(error)
                    .bind(now.to_rfc3339())
                    .bind(id.to_string())
                    .execute(pool)
                    .await?;
            }
        }

        self.get_by_id(id)
            .await?
            .ok_or_else(|| DatabaseError::NotFound("Rollout not found after update".to_string()))
    }

    /// Increment deployed agent count
    pub async fn increment_deployed_count(
        &self,
        id: Uuid,
        count: u32,
    ) -> Result<Rollout, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let now = Utc::now();

        let sql = r#"
            UPDATE rollouts
            SET deployed_agent_count = deployed_agent_count + $1, updated_at = $2
            WHERE id = $3
        "#;

        sqlx::query(sql)
            .bind(count as i32)
            .bind(now.to_rfc3339())
            .bind(id.to_string())
            .execute(pool)
            .await?;

        self.get_by_id(id)
            .await?
            .ok_or_else(|| DatabaseError::NotFound("Rollout not found after update".to_string()))
    }

    /// Advance to next wave
    pub async fn advance_wave(&self, id: Uuid) -> Result<Rollout, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let now = Utc::now();

        let sql = r#"
            UPDATE rollouts
            SET current_wave = current_wave + 1, updated_at = $1
            WHERE id = $2
        "#;

        sqlx::query(sql)
            .bind(now.to_rfc3339())
            .bind(id.to_string())
            .execute(pool)
            .await?;

        self.get_by_id(id)
            .await?
            .ok_or_else(|| DatabaseError::NotFound("Rollout not found after update".to_string()))
    }
}
