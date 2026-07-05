//! Version pin repository operations

use chrono::Utc;
use uuid::Uuid;

use crate::db::{Database, DatabaseError};
use crate::domain::deployment::{CreateVersionPin, VersionPin};

use super::row_conversions::row_to_pin;

/// Version pin repository operations
pub struct PinOps<'a> {
    pub(super) db: &'a Database,
}

impl<'a> PinOps<'a> {
    /// Create or update a version pin
    pub async fn create(
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
            VALUES ($1, $2, $3, $4, $5, $6)
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

        self.get(agent_id)
            .await?
            .ok_or_else(|| DatabaseError::NotFound("Pin not found after creation".to_string()))
    }

    /// Get a version pin for an agent
    pub async fn get(&self, agent_id: Uuid) -> Result<Option<VersionPin>, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let sql = r#"
            SELECT agent_id, bundle_id, pinned_by, reason, expires_at, created_at
            FROM version_pins
            WHERE agent_id = $1
        "#;

        let row = sqlx::query(sql)
            .bind(agent_id.to_string())
            .fetch_optional(pool)
            .await?;

        row.map(|r| row_to_pin(&r)).transpose()
    }

    /// Get active (non-expired) pin for an agent
    pub async fn get_active(&self, agent_id: Uuid) -> Result<Option<VersionPin>, DatabaseError> {
        let pin = self.get(agent_id).await?;
        Ok(pin.filter(|p| !p.is_expired()))
    }

    /// List all pins for agents in an org
    pub async fn list(&self, org_id: Uuid) -> Result<Vec<VersionPin>, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let sql = r#"
            SELECT vp.agent_id, vp.bundle_id, vp.pinned_by, vp.reason, vp.expires_at, vp.created_at
            FROM version_pins vp
            INNER JOIN agents a ON vp.agent_id = a.id
            WHERE a.org_id = $1
        "#;

        let rows = sqlx::query(sql)
            .bind(org_id.to_string())
            .fetch_all(pool)
            .await?;

        rows.iter().map(|r| row_to_pin(r)).collect()
    }

    /// Delete a version pin
    pub async fn delete(&self, agent_id: Uuid) -> Result<(), DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let sql = "DELETE FROM version_pins WHERE agent_id = $1";
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
    pub async fn delete_expired(&self) -> Result<u64, DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let now = Utc::now();

        let sql = "DELETE FROM version_pins WHERE expires_at IS NOT NULL AND expires_at < $1";
        let result = sqlx::query(sql)
            .bind(now.to_rfc3339())
            .execute(pool)
            .await?;

        Ok(result.rows_affected())
    }
}
