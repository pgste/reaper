//! Rollout wave repository operations

use chrono::Utc;
use uuid::Uuid;

use crate::db::{Database, DatabaseError};
use crate::domain::deployment::{RolloutWave, WaveStatus};

use super::row_conversions::row_to_wave;

/// Rollout wave repository operations
pub struct WaveOps<'a> {
    pub(super) db: &'a Database,
}

impl<'a> WaveOps<'a> {
    /// Create a rollout wave
    pub async fn create(
        &self,
        rollout_id: Uuid,
        wave_number: u32,
        target_agents: &[Uuid],
    ) -> Result<RolloutWave, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let id = Uuid::new_v4();
        let now = Utc::now();
        let agents_json = serde_json::to_string(target_agents)
            .map_err(|e| DatabaseError::Config(format!("Failed to serialize agents: {}", e)))?;

        let sql = r#"
            INSERT INTO rollout_waves (id, rollout_id, wave_number, target_agents, status, deployed_count, created_at)
            VALUES ($1, $2, $3, $4, $5, 0, $6)
        "#;

        sqlx::query(sql)
            .bind(id.to_string())
            .bind(rollout_id.to_string())
            .bind(wave_number as i32)
            .bind(&agents_json)
            .bind(WaveStatus::Pending.to_string())
            .bind(now.to_rfc3339())
            .execute(pool)
            .await?;

        self.get_by_id(id)
            .await?
            .ok_or_else(|| DatabaseError::NotFound("Wave not found after creation".to_string()))
    }

    /// Get a wave by ID
    pub async fn get_by_id(&self, id: Uuid) -> Result<Option<RolloutWave>, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let sql = r#"
            SELECT id, rollout_id, wave_number, target_agents, status, deployed_count,
                   started_at, completed_at, created_at
            FROM rollout_waves
            WHERE id = $1
        "#;

        let row = sqlx::query(sql)
            .bind(id.to_string())
            .fetch_optional(pool)
            .await?;

        row.map(|r| row_to_wave(&r)).transpose()
    }

    /// Get waves for a rollout
    pub async fn get_for_rollout(
        &self,
        rollout_id: Uuid,
    ) -> Result<Vec<RolloutWave>, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let sql = r#"
            SELECT id, rollout_id, wave_number, target_agents, status, deployed_count,
                   started_at, completed_at, created_at
            FROM rollout_waves
            WHERE rollout_id = $1
            ORDER BY wave_number ASC
        "#;

        let rows = sqlx::query(sql)
            .bind(rollout_id.to_string())
            .fetch_all(pool)
            .await?;

        rows.iter().map(|r| row_to_wave(r)).collect()
    }

    /// Update wave status
    pub async fn update_status(
        &self,
        id: Uuid,
        status: WaveStatus,
    ) -> Result<RolloutWave, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let now = Utc::now();

        match status {
            WaveStatus::Deploying => {
                let sql = r#"
                    UPDATE rollout_waves
                    SET status = $1, started_at = $2
                    WHERE id = $3
                "#;
                sqlx::query(sql)
                    .bind(status.to_string())
                    .bind(now.to_rfc3339())
                    .bind(id.to_string())
                    .execute(pool)
                    .await?;
            }
            WaveStatus::Completed | WaveStatus::Failed => {
                let sql = r#"
                    UPDATE rollout_waves
                    SET status = $1, completed_at = $2
                    WHERE id = $3
                "#;
                sqlx::query(sql)
                    .bind(status.to_string())
                    .bind(now.to_rfc3339())
                    .bind(id.to_string())
                    .execute(pool)
                    .await?;
            }
            _ => {
                let sql = r#"
                    UPDATE rollout_waves
                    SET status = $1
                    WHERE id = $2
                "#;
                sqlx::query(sql)
                    .bind(status.to_string())
                    .bind(id.to_string())
                    .execute(pool)
                    .await?;
            }
        }

        self.get_by_id(id)
            .await?
            .ok_or_else(|| DatabaseError::NotFound("Wave not found after update".to_string()))
    }

    /// Increment deployed count for a wave
    pub async fn increment_deployed(
        &self,
        id: Uuid,
        count: u32,
    ) -> Result<RolloutWave, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let sql = r#"
            UPDATE rollout_waves
            SET deployed_count = deployed_count + $1
            WHERE id = $2
        "#;

        sqlx::query(sql)
            .bind(count as i32)
            .bind(id.to_string())
            .execute(pool)
            .await?;

        self.get_by_id(id)
            .await?
            .ok_or_else(|| DatabaseError::NotFound("Wave not found after update".to_string()))
    }
}
