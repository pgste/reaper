//! Deployment repository
//!
//! Data access layer for deployment strategies, rollouts, and version pins.

use chrono::Utc;
use sqlx::Row;
use uuid::Uuid;

use crate::db::{Database, DatabaseError};
use crate::domain::deployment::{
    CreateDeploymentStrategy, CreateVersionPin, DeploymentStrategy, Rollout, RolloutStatus,
    RolloutWave, StartRollout, StrategyConfig, StrategyType, VersionPin, WaveStatus,
};

/// Repository for deployment operations
pub struct DeploymentRepository<'a> {
    db: &'a Database,
}

impl<'a> DeploymentRepository<'a> {
    /// Create a new repository instance
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    // ==================== Deployment Strategies ====================

    /// Create a new deployment strategy
    pub async fn create_strategy(
        &self,
        org_id: Uuid,
        input: &CreateDeploymentStrategy,
    ) -> Result<DeploymentStrategy, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let id = Uuid::new_v4();
        let now = Utc::now();
        let config_json = serde_json::to_string(&input.config)
            .map_err(|e| DatabaseError::Config(format!("Failed to serialize config: {}", e)))?;

        // If this is set as default, unset other defaults first
        if input.is_default {
            self.unset_default_strategies(org_id, input.namespace_id)
                .await?;
        }

        let sql = r#"
            INSERT INTO deployment_strategies (id, org_id, namespace_id, name, strategy_type, config, is_default, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
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

        self.get_strategy_by_id(id)
            .await?
            .ok_or_else(|| DatabaseError::NotFound("Strategy not found after creation".to_string()))
    }

    /// Get a deployment strategy by ID
    pub async fn get_strategy_by_id(
        &self,
        id: Uuid,
    ) -> Result<Option<DeploymentStrategy>, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let sql = r#"
            SELECT id, org_id, namespace_id, name, strategy_type, config, is_default, created_at, updated_at
            FROM deployment_strategies
            WHERE id = ?
        "#;

        let row = sqlx::query(sql)
            .bind(id.to_string())
            .fetch_optional(pool)
            .await?;

        row.map(|r| self.row_to_strategy(&r)).transpose()
    }

    /// List deployment strategies for an organization
    pub async fn list_strategies(
        &self,
        org_id: Uuid,
        namespace_id: Option<Uuid>,
    ) -> Result<Vec<DeploymentStrategy>, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let (sql, rows) = if let Some(ns_id) = namespace_id {
            let sql = r#"
                SELECT id, org_id, namespace_id, name, strategy_type, config, is_default, created_at, updated_at
                FROM deployment_strategies
                WHERE org_id = ? AND (namespace_id = ? OR namespace_id IS NULL)
                ORDER BY is_default DESC, name ASC
            "#;
            let rows = sqlx::query(sql)
                .bind(org_id.to_string())
                .bind(ns_id.to_string())
                .fetch_all(pool)
                .await?;
            (sql, rows)
        } else {
            let sql = r#"
                SELECT id, org_id, namespace_id, name, strategy_type, config, is_default, created_at, updated_at
                FROM deployment_strategies
                WHERE org_id = ?
                ORDER BY is_default DESC, name ASC
            "#;
            let rows = sqlx::query(sql)
                .bind(org_id.to_string())
                .fetch_all(pool)
                .await?;
            (sql, rows)
        };

        let _ = sql; // Silence unused warning
        rows.iter().map(|r| self.row_to_strategy(r)).collect()
    }

    /// Get the default strategy for a namespace (or org-wide)
    pub async fn get_default_strategy(
        &self,
        org_id: Uuid,
        namespace_id: Option<Uuid>,
    ) -> Result<Option<DeploymentStrategy>, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        // First try namespace-specific default, then org-wide default
        let sql = if namespace_id.is_some() {
            r#"
                SELECT id, org_id, namespace_id, name, strategy_type, config, is_default, created_at, updated_at
                FROM deployment_strategies
                WHERE org_id = ? AND is_default = 1 AND (namespace_id = ? OR namespace_id IS NULL)
                ORDER BY namespace_id DESC NULLS LAST
                LIMIT 1
            "#
        } else {
            r#"
                SELECT id, org_id, namespace_id, name, strategy_type, config, is_default, created_at, updated_at
                FROM deployment_strategies
                WHERE org_id = ? AND is_default = 1 AND namespace_id IS NULL
                LIMIT 1
            "#
        };

        let row = if let Some(ns_id) = namespace_id {
            sqlx::query(sql)
                .bind(org_id.to_string())
                .bind(ns_id.to_string())
                .fetch_optional(pool)
                .await?
        } else {
            sqlx::query(sql)
                .bind(org_id.to_string())
                .fetch_optional(pool)
                .await?
        };

        row.map(|r| self.row_to_strategy(&r)).transpose()
    }

    /// Delete a deployment strategy
    pub async fn delete_strategy(&self, id: Uuid) -> Result<(), DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let sql = "DELETE FROM deployment_strategies WHERE id = ?";
        let result = sqlx::query(sql).bind(id.to_string()).execute(pool).await?;

        if result.rows_affected() == 0 {
            return Err(DatabaseError::NotFound(format!("Strategy {} not found", id)));
        }

        Ok(())
    }

    /// Unset default flag for strategies in scope
    async fn unset_default_strategies(
        &self,
        org_id: Uuid,
        namespace_id: Option<Uuid>,
    ) -> Result<(), DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let sql = if namespace_id.is_some() {
            "UPDATE deployment_strategies SET is_default = 0 WHERE org_id = ? AND namespace_id = ?"
        } else {
            "UPDATE deployment_strategies SET is_default = 0 WHERE org_id = ? AND namespace_id IS NULL"
        };

        if let Some(ns_id) = namespace_id {
            sqlx::query(sql)
                .bind(org_id.to_string())
                .bind(ns_id.to_string())
                .execute(pool)
                .await?;
        } else {
            sqlx::query(sql)
                .bind(org_id.to_string())
                .execute(pool)
                .await?;
        }

        Ok(())
    }

    // ==================== Rollouts ====================

    /// Create a new rollout
    pub async fn create_rollout(
        &self,
        input: &StartRollout,
        target_agent_count: u32,
    ) -> Result<Rollout, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let id = Uuid::new_v4();
        let now = Utc::now();

        let sql = r#"
            INSERT INTO rollouts (id, bundle_id, strategy_id, namespace_id, status, current_wave,
                                  target_agent_count, deployed_agent_count, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, 0, ?, 0, ?, ?)
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

        self.get_rollout_by_id(id)
            .await?
            .ok_or_else(|| DatabaseError::NotFound("Rollout not found after creation".to_string()))
    }

    /// Get a rollout by ID
    pub async fn get_rollout_by_id(&self, id: Uuid) -> Result<Option<Rollout>, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let sql = r#"
            SELECT id, bundle_id, strategy_id, namespace_id, status, current_wave,
                   target_agent_count, deployed_agent_count, started_at, completed_at,
                   error, created_at, updated_at
            FROM rollouts
            WHERE id = ?
        "#;

        let row = sqlx::query(sql)
            .bind(id.to_string())
            .fetch_optional(pool)
            .await?;

        row.map(|r| self.row_to_rollout(&r)).transpose()
    }

    /// Get active rollouts for a bundle
    pub async fn get_active_rollouts_for_bundle(
        &self,
        bundle_id: Uuid,
    ) -> Result<Vec<Rollout>, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let sql = r#"
            SELECT id, bundle_id, strategy_id, namespace_id, status, current_wave,
                   target_agent_count, deployed_agent_count, started_at, completed_at,
                   error, created_at, updated_at
            FROM rollouts
            WHERE bundle_id = ? AND status NOT IN ('completed', 'failed', 'rolled_back', 'cancelled')
            ORDER BY created_at DESC
        "#;

        let rows = sqlx::query(sql)
            .bind(bundle_id.to_string())
            .fetch_all(pool)
            .await?;

        rows.iter().map(|r| self.row_to_rollout(r)).collect()
    }

    /// List rollouts for a namespace
    pub async fn list_rollouts(
        &self,
        org_id: Uuid,
        namespace_id: Option<Uuid>,
        limit: i32,
    ) -> Result<Vec<Rollout>, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let sql = if namespace_id.is_some() {
            r#"
                SELECT r.id, r.bundle_id, r.strategy_id, r.namespace_id, r.status, r.current_wave,
                       r.target_agent_count, r.deployed_agent_count, r.started_at, r.completed_at,
                       r.error, r.created_at, r.updated_at
                FROM rollouts r
                INNER JOIN bundles b ON r.bundle_id = b.id
                WHERE b.org_id = ? AND r.namespace_id = ?
                ORDER BY r.created_at DESC
                LIMIT ?
            "#
        } else {
            r#"
                SELECT r.id, r.bundle_id, r.strategy_id, r.namespace_id, r.status, r.current_wave,
                       r.target_agent_count, r.deployed_agent_count, r.started_at, r.completed_at,
                       r.error, r.created_at, r.updated_at
                FROM rollouts r
                INNER JOIN bundles b ON r.bundle_id = b.id
                WHERE b.org_id = ?
                ORDER BY r.created_at DESC
                LIMIT ?
            "#
        };

        let rows = if let Some(ns_id) = namespace_id {
            sqlx::query(sql)
                .bind(org_id.to_string())
                .bind(ns_id.to_string())
                .bind(limit)
                .fetch_all(pool)
                .await?
        } else {
            sqlx::query(sql)
                .bind(org_id.to_string())
                .bind(limit)
                .fetch_all(pool)
                .await?
        };

        rows.iter().map(|r| self.row_to_rollout(r)).collect()
    }

    /// Update rollout status
    pub async fn update_rollout_status(
        &self,
        id: Uuid,
        status: RolloutStatus,
        error: Option<&str>,
    ) -> Result<Rollout, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let now = Utc::now();

        let sql = match status {
            RolloutStatus::InProgress => {
                r#"
                    UPDATE rollouts
                    SET status = ?, started_at = COALESCE(started_at, ?), updated_at = ?
                    WHERE id = ?
                "#
            }
            RolloutStatus::Completed
            | RolloutStatus::Failed
            | RolloutStatus::RolledBack
            | RolloutStatus::Cancelled => {
                r#"
                    UPDATE rollouts
                    SET status = ?, completed_at = ?, error = ?, updated_at = ?
                    WHERE id = ?
                "#
            }
            _ => {
                r#"
                    UPDATE rollouts
                    SET status = ?, error = ?, updated_at = ?
                    WHERE id = ?
                "#
            }
        };

        match status {
            RolloutStatus::InProgress => {
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
                sqlx::query(sql)
                    .bind(status.to_string())
                    .bind(error)
                    .bind(now.to_rfc3339())
                    .bind(id.to_string())
                    .execute(pool)
                    .await?;
            }
        }

        self.get_rollout_by_id(id)
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
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let now = Utc::now();

        let sql = r#"
            UPDATE rollouts
            SET deployed_agent_count = deployed_agent_count + ?, updated_at = ?
            WHERE id = ?
        "#;

        sqlx::query(sql)
            .bind(count as i32)
            .bind(now.to_rfc3339())
            .bind(id.to_string())
            .execute(pool)
            .await?;

        self.get_rollout_by_id(id)
            .await?
            .ok_or_else(|| DatabaseError::NotFound("Rollout not found after update".to_string()))
    }

    /// Advance to next wave
    pub async fn advance_wave(&self, id: Uuid) -> Result<Rollout, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let now = Utc::now();

        let sql = r#"
            UPDATE rollouts
            SET current_wave = current_wave + 1, updated_at = ?
            WHERE id = ?
        "#;

        sqlx::query(sql)
            .bind(now.to_rfc3339())
            .bind(id.to_string())
            .execute(pool)
            .await?;

        self.get_rollout_by_id(id)
            .await?
            .ok_or_else(|| DatabaseError::NotFound("Rollout not found after update".to_string()))
    }

    // ==================== Rollout Waves ====================

    /// Create a rollout wave
    pub async fn create_wave(
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
            VALUES (?, ?, ?, ?, ?, 0, ?)
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

        self.get_wave_by_id(id)
            .await?
            .ok_or_else(|| DatabaseError::NotFound("Wave not found after creation".to_string()))
    }

    /// Get a wave by ID
    pub async fn get_wave_by_id(&self, id: Uuid) -> Result<Option<RolloutWave>, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let sql = r#"
            SELECT id, rollout_id, wave_number, target_agents, status, deployed_count,
                   started_at, completed_at, created_at
            FROM rollout_waves
            WHERE id = ?
        "#;

        let row = sqlx::query(sql)
            .bind(id.to_string())
            .fetch_optional(pool)
            .await?;

        row.map(|r| self.row_to_wave(&r)).transpose()
    }

    /// Get waves for a rollout
    pub async fn get_waves_for_rollout(
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
            WHERE rollout_id = ?
            ORDER BY wave_number ASC
        "#;

        let rows = sqlx::query(sql)
            .bind(rollout_id.to_string())
            .fetch_all(pool)
            .await?;

        rows.iter().map(|r| self.row_to_wave(r)).collect()
    }

    /// Update wave status
    pub async fn update_wave_status(
        &self,
        id: Uuid,
        status: WaveStatus,
    ) -> Result<RolloutWave, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let now = Utc::now();

        let sql = match status {
            WaveStatus::Deploying => {
                r#"
                    UPDATE rollout_waves
                    SET status = ?, started_at = ?
                    WHERE id = ?
                "#
            }
            WaveStatus::Completed | WaveStatus::Failed => {
                r#"
                    UPDATE rollout_waves
                    SET status = ?, completed_at = ?
                    WHERE id = ?
                "#
            }
            _ => {
                r#"
                    UPDATE rollout_waves
                    SET status = ?
                    WHERE id = ?
                "#
            }
        };

        match status {
            WaveStatus::Deploying | WaveStatus::Completed | WaveStatus::Failed => {
                sqlx::query(sql)
                    .bind(status.to_string())
                    .bind(now.to_rfc3339())
                    .bind(id.to_string())
                    .execute(pool)
                    .await?;
            }
            _ => {
                sqlx::query(sql)
                    .bind(status.to_string())
                    .bind(id.to_string())
                    .execute(pool)
                    .await?;
            }
        }

        self.get_wave_by_id(id)
            .await?
            .ok_or_else(|| DatabaseError::NotFound("Wave not found after update".to_string()))
    }

    /// Increment deployed count for a wave
    pub async fn increment_wave_deployed(
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
            SET deployed_count = deployed_count + ?
            WHERE id = ?
        "#;

        sqlx::query(sql)
            .bind(count as i32)
            .bind(id.to_string())
            .execute(pool)
            .await?;

        self.get_wave_by_id(id)
            .await?
            .ok_or_else(|| DatabaseError::NotFound("Wave not found after update".to_string()))
    }

    // ==================== Version Pins ====================

    /// Create or update a version pin
    pub async fn create_pin(
        &self,
        agent_id: Uuid,
        input: &CreateVersionPin,
        pinned_by: Option<&str>,
    ) -> Result<VersionPin, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let now = Utc::now();

        let sql = r#"
            INSERT INTO version_pins (agent_id, bundle_id, pinned_by, reason, expires_at, created_at)
            VALUES (?, ?, ?, ?, ?, ?)
            ON CONFLICT(agent_id) DO UPDATE SET
                bundle_id = excluded.bundle_id,
                pinned_by = excluded.pinned_by,
                reason = excluded.reason,
                expires_at = excluded.expires_at,
                created_at = excluded.created_at
        "#;

        sqlx::query(sql)
            .bind(agent_id.to_string())
            .bind(input.bundle_id.to_string())
            .bind(pinned_by)
            .bind(&input.reason)
            .bind(input.expires_at.map(|dt| dt.to_rfc3339()))
            .bind(now.to_rfc3339())
            .execute(pool)
            .await?;

        self.get_pin(agent_id)
            .await?
            .ok_or_else(|| DatabaseError::NotFound("Pin not found after creation".to_string()))
    }

    /// Get a version pin for an agent
    pub async fn get_pin(&self, agent_id: Uuid) -> Result<Option<VersionPin>, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let sql = r#"
            SELECT agent_id, bundle_id, pinned_by, reason, expires_at, created_at
            FROM version_pins
            WHERE agent_id = ?
        "#;

        let row = sqlx::query(sql)
            .bind(agent_id.to_string())
            .fetch_optional(pool)
            .await?;

        row.map(|r| self.row_to_pin(&r)).transpose()
    }

    /// Get active (non-expired) pin for an agent
    pub async fn get_active_pin(
        &self,
        agent_id: Uuid,
    ) -> Result<Option<VersionPin>, DatabaseError> {
        let pin = self.get_pin(agent_id).await?;
        Ok(pin.filter(|p| !p.is_expired()))
    }

    /// List all pins for agents in an org
    pub async fn list_pins(&self, org_id: Uuid) -> Result<Vec<VersionPin>, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let sql = r#"
            SELECT vp.agent_id, vp.bundle_id, vp.pinned_by, vp.reason, vp.expires_at, vp.created_at
            FROM version_pins vp
            INNER JOIN agents a ON vp.agent_id = a.id
            WHERE a.org_id = ?
        "#;

        let rows = sqlx::query(sql)
            .bind(org_id.to_string())
            .fetch_all(pool)
            .await?;

        rows.iter().map(|r| self.row_to_pin(r)).collect()
    }

    /// Delete a version pin
    pub async fn delete_pin(&self, agent_id: Uuid) -> Result<(), DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let sql = "DELETE FROM version_pins WHERE agent_id = ?";
        let result = sqlx::query(sql)
            .bind(agent_id.to_string())
            .execute(pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(DatabaseError::NotFound(format!(
                "Pin for agent {} not found",
                agent_id
            )));
        }

        Ok(())
    }

    /// Delete expired pins
    pub async fn delete_expired_pins(&self) -> Result<u64, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let now = Utc::now();

        let sql = "DELETE FROM version_pins WHERE expires_at IS NOT NULL AND expires_at < ?";
        let result = sqlx::query(sql)
            .bind(now.to_rfc3339())
            .execute(pool)
            .await?;

        Ok(result.rows_affected())
    }

    // ==================== Row Conversions ====================

    fn row_to_strategy(
        &self,
        row: &sqlx::sqlite::SqliteRow,
    ) -> Result<DeploymentStrategy, DatabaseError> {
        let id: String = row.get("id");
        let org_id: String = row.get("org_id");
        let namespace_id: Option<String> = row.get("namespace_id");
        let strategy_type: String = row.get("strategy_type");
        let config_json: String = row.get("config");
        let is_default: i32 = row.get("is_default");
        let created_at: String = row.get("created_at");
        let updated_at: String = row.get("updated_at");

        let config: StrategyConfig = serde_json::from_str(&config_json)
            .map_err(|e| DatabaseError::Config(format!("Failed to parse config: {}", e)))?;

        Ok(DeploymentStrategy {
            id: id
                .parse()
                .map_err(|e| DatabaseError::Config(format!("Invalid UUID: {}", e)))?,
            org_id: org_id
                .parse()
                .map_err(|e| DatabaseError::Config(format!("Invalid UUID: {}", e)))?,
            namespace_id: namespace_id
                .map(|s| {
                    s.parse()
                        .map_err(|e| DatabaseError::Config(format!("Invalid UUID: {}", e)))
                })
                .transpose()?,
            name: row.get("name"),
            strategy_type: strategy_type.parse().unwrap_or(StrategyType::Immediate),
            config,
            is_default: is_default != 0,
            created_at: chrono::DateTime::parse_from_rfc3339(&created_at)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            updated_at: chrono::DateTime::parse_from_rfc3339(&updated_at)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
        })
    }

    fn row_to_rollout(&self, row: &sqlx::sqlite::SqliteRow) -> Result<Rollout, DatabaseError> {
        let id: String = row.get("id");
        let bundle_id: String = row.get("bundle_id");
        let strategy_id: Option<String> = row.get("strategy_id");
        let namespace_id: Option<String> = row.get("namespace_id");
        let status: String = row.get("status");
        let started_at: Option<String> = row.get("started_at");
        let completed_at: Option<String> = row.get("completed_at");
        let created_at: String = row.get("created_at");
        let updated_at: String = row.get("updated_at");

        Ok(Rollout {
            id: id
                .parse()
                .map_err(|e| DatabaseError::Config(format!("Invalid UUID: {}", e)))?,
            bundle_id: bundle_id
                .parse()
                .map_err(|e| DatabaseError::Config(format!("Invalid UUID: {}", e)))?,
            strategy_id: strategy_id
                .map(|s| {
                    s.parse()
                        .map_err(|e| DatabaseError::Config(format!("Invalid UUID: {}", e)))
                })
                .transpose()?,
            namespace_id: namespace_id
                .map(|s| {
                    s.parse()
                        .map_err(|e| DatabaseError::Config(format!("Invalid UUID: {}", e)))
                })
                .transpose()?,
            status: status.parse().unwrap_or(RolloutStatus::Pending),
            current_wave: row.get::<i32, _>("current_wave") as u32,
            target_agent_count: row.get::<i32, _>("target_agent_count") as u32,
            deployed_agent_count: row.get::<i32, _>("deployed_agent_count") as u32,
            started_at: started_at.and_then(|s| {
                chrono::DateTime::parse_from_rfc3339(&s)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            }),
            completed_at: completed_at.and_then(|s| {
                chrono::DateTime::parse_from_rfc3339(&s)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            }),
            error: row.get("error"),
            created_at: chrono::DateTime::parse_from_rfc3339(&created_at)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            updated_at: chrono::DateTime::parse_from_rfc3339(&updated_at)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
        })
    }

    fn row_to_wave(&self, row: &sqlx::sqlite::SqliteRow) -> Result<RolloutWave, DatabaseError> {
        let id: String = row.get("id");
        let rollout_id: String = row.get("rollout_id");
        let target_agents_json: String = row.get("target_agents");
        let status: String = row.get("status");
        let started_at: Option<String> = row.get("started_at");
        let completed_at: Option<String> = row.get("completed_at");
        let created_at: String = row.get("created_at");

        let target_agents: Vec<Uuid> = serde_json::from_str(&target_agents_json)
            .map_err(|e| DatabaseError::Config(format!("Failed to parse target_agents: {}", e)))?;

        Ok(RolloutWave {
            id: id
                .parse()
                .map_err(|e| DatabaseError::Config(format!("Invalid UUID: {}", e)))?,
            rollout_id: rollout_id
                .parse()
                .map_err(|e| DatabaseError::Config(format!("Invalid UUID: {}", e)))?,
            wave_number: row.get::<i32, _>("wave_number") as u32,
            target_agents,
            status: status.parse().unwrap_or(WaveStatus::Pending),
            deployed_count: row.get::<i32, _>("deployed_count") as u32,
            started_at: started_at.and_then(|s| {
                chrono::DateTime::parse_from_rfc3339(&s)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            }),
            completed_at: completed_at.and_then(|s| {
                chrono::DateTime::parse_from_rfc3339(&s)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            }),
            created_at: chrono::DateTime::parse_from_rfc3339(&created_at)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
        })
    }

    fn row_to_pin(&self, row: &sqlx::sqlite::SqliteRow) -> Result<VersionPin, DatabaseError> {
        let agent_id: String = row.get("agent_id");
        let bundle_id: String = row.get("bundle_id");
        let expires_at: Option<String> = row.get("expires_at");
        let created_at: String = row.get("created_at");

        Ok(VersionPin {
            agent_id: agent_id
                .parse()
                .map_err(|e| DatabaseError::Config(format!("Invalid UUID: {}", e)))?,
            bundle_id: bundle_id
                .parse()
                .map_err(|e| DatabaseError::Config(format!("Invalid UUID: {}", e)))?,
            pinned_by: row.get("pinned_by"),
            reason: row.get("reason"),
            expires_at: expires_at.and_then(|s| {
                chrono::DateTime::parse_from_rfc3339(&s)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            }),
            created_at: chrono::DateTime::parse_from_rfc3339(&created_at)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DatabaseConfig;
    use std::collections::HashMap;
    use tempfile::TempDir;

    async fn setup_db() -> (TempDir, std::sync::Arc<Database>) {
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
        (temp_dir, std::sync::Arc::new(db))
    }

    async fn create_test_org(db: &Database) -> Uuid {
        let pool = db.sqlite_pool().unwrap();
        let org_id = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO organizations (id, name, slug, created_at, updated_at) VALUES (?, ?, ?, ?, ?)",
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

    async fn create_test_bundle(db: &Database, org_id: Uuid) -> Uuid {
        let pool = db.sqlite_pool().unwrap();
        let bundle_id = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO bundles (id, org_id, name, version, status, policy_count, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(bundle_id.to_string())
        .bind(org_id.to_string())
        .bind("test-bundle")
        .bind("1.0.0")
        .bind("compiled")
        .bind(0)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
        bundle_id
    }

    async fn create_test_agent(db: &Database, org_id: Uuid) -> Uuid {
        let pool = db.sqlite_pool().unwrap();
        let agent_id = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO agents (id, org_id, name, status, registered_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(agent_id.to_string())
        .bind(org_id.to_string())
        .bind("test-agent")
        .bind("online")
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
        agent_id
    }

    #[tokio::test]
    async fn test_create_and_get_strategy() {
        let (_temp_dir, db) = setup_db().await;
        let org_id = create_test_org(&db).await;
        let repo = DeploymentRepository::new(&db);

        let input = CreateDeploymentStrategy {
            name: "canary-prod".to_string(),
            namespace_id: None,
            strategy_type: StrategyType::Canary,
            config: StrategyConfig::Canary {
                canary_labels: HashMap::from([("env".to_string(), "canary".to_string())]),
                wait_seconds: 300,
                require_approval: true,
            },
            is_default: true,
        };

        let strategy = repo.create_strategy(org_id, &input).await.unwrap();
        assert_eq!(strategy.name, "canary-prod");
        assert_eq!(strategy.strategy_type, StrategyType::Canary);
        assert!(strategy.is_default);

        let retrieved = repo.get_strategy_by_id(strategy.id).await.unwrap().unwrap();
        assert_eq!(retrieved.name, "canary-prod");
    }

    #[tokio::test]
    async fn test_list_strategies() {
        let (_temp_dir, db) = setup_db().await;
        let org_id = create_test_org(&db).await;
        let repo = DeploymentRepository::new(&db);

        // Create two strategies
        repo.create_strategy(
            org_id,
            &CreateDeploymentStrategy {
                name: "immediate".to_string(),
                namespace_id: None,
                strategy_type: StrategyType::Immediate,
                config: StrategyConfig::Immediate {},
                is_default: true,
            },
        )
        .await
        .unwrap();

        repo.create_strategy(
            org_id,
            &CreateDeploymentStrategy {
                name: "percentage".to_string(),
                namespace_id: None,
                strategy_type: StrategyType::Percentage,
                config: StrategyConfig::Percentage {
                    waves: vec![10, 25, 50, 100],
                    wave_delay_seconds: 60,
                    require_approval: false,
                },
                is_default: false,
            },
        )
        .await
        .unwrap();

        let strategies = repo.list_strategies(org_id, None).await.unwrap();
        assert_eq!(strategies.len(), 2);
    }

    #[tokio::test]
    async fn test_create_and_update_rollout() {
        let (_temp_dir, db) = setup_db().await;
        let org_id = create_test_org(&db).await;
        let bundle_id = create_test_bundle(&db, org_id).await;
        let repo = DeploymentRepository::new(&db);

        let input = StartRollout {
            bundle_id,
            strategy_id: None,
            namespace_id: None,
        };

        let rollout = repo.create_rollout(&input, 10).await.unwrap();
        assert_eq!(rollout.status, RolloutStatus::Pending);
        assert_eq!(rollout.target_agent_count, 10);

        // Start the rollout
        let rollout = repo
            .update_rollout_status(rollout.id, RolloutStatus::InProgress, None)
            .await
            .unwrap();
        assert_eq!(rollout.status, RolloutStatus::InProgress);
        assert!(rollout.started_at.is_some());

        // Increment deployed count
        let rollout = repo.increment_deployed_count(rollout.id, 5).await.unwrap();
        assert_eq!(rollout.deployed_agent_count, 5);
    }

    #[tokio::test]
    async fn test_version_pins() {
        let (_temp_dir, db) = setup_db().await;
        let org_id = create_test_org(&db).await;
        let bundle_id = create_test_bundle(&db, org_id).await;
        let agent_id = create_test_agent(&db, org_id).await;
        let repo = DeploymentRepository::new(&db);

        let input = CreateVersionPin {
            bundle_id,
            reason: Some("Testing".to_string()),
            expires_at: None,
        };

        let pin = repo
            .create_pin(agent_id, &input, Some("admin"))
            .await
            .unwrap();
        assert_eq!(pin.bundle_id, bundle_id);
        assert_eq!(pin.pinned_by, Some("admin".to_string()));
        assert!(!pin.is_expired());

        // Get active pin
        let active_pin = repo.get_active_pin(agent_id).await.unwrap();
        assert!(active_pin.is_some());

        // Delete pin
        repo.delete_pin(agent_id).await.unwrap();
        let pin = repo.get_pin(agent_id).await.unwrap();
        assert!(pin.is_none());
    }

    #[tokio::test]
    async fn test_rollout_waves() {
        let (_temp_dir, db) = setup_db().await;
        let org_id = create_test_org(&db).await;
        let bundle_id = create_test_bundle(&db, org_id).await;
        let repo = DeploymentRepository::new(&db);

        let rollout = repo
            .create_rollout(
                &StartRollout {
                    bundle_id,
                    strategy_id: None,
                    namespace_id: None,
                },
                10,
            )
            .await
            .unwrap();

        let agent_ids = vec![Uuid::new_v4(), Uuid::new_v4()];
        let wave = repo
            .create_wave(rollout.id, 1, &agent_ids)
            .await
            .unwrap();
        assert_eq!(wave.wave_number, 1);
        assert_eq!(wave.target_agents.len(), 2);
        assert_eq!(wave.status, WaveStatus::Pending);

        // Start deploying
        let wave = repo
            .update_wave_status(wave.id, WaveStatus::Deploying)
            .await
            .unwrap();
        assert_eq!(wave.status, WaveStatus::Deploying);
        assert!(wave.started_at.is_some());

        // Complete
        let wave = repo
            .update_wave_status(wave.id, WaveStatus::Completed)
            .await
            .unwrap();
        assert_eq!(wave.status, WaveStatus::Completed);
        assert!(wave.completed_at.is_some());
    }
}
