//! Audit governance persistence: tenant retention windows + legal holds
//! (Plan 04, step 6).
//!
//! Governance records live in the management database — the transactional,
//! audited source of truth — while the purge they govern executes against
//! ClickHouse (`crate::decisions::DecisionStore::purge_expired`). A legal hold
//! is never hard-deleted: release stamps `released_at`/`released_by`, keeping
//! the hold's own lifecycle auditable.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::{Database, DatabaseError};
use crate::decisions::HoldFilter;

/// A tenant's audit retention setting.
#[derive(Debug, Clone, Serialize)]
pub struct AuditRetention {
    pub org_id: Uuid,
    /// Retention window in days (> 0).
    pub days: i64,
    pub updated_by: Option<String>,
    pub updated_at: DateTime<Utc>,
}

/// A legal hold: decisions matching `filter` are exempt from retention purge
/// until the hold is released.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegalHold {
    pub id: Uuid,
    pub org_id: Uuid,
    pub filter: HoldFilter,
    pub reason: String,
    pub created_by: Option<String>,
    pub created_at: DateTime<Utc>,
    /// `None` = active (exempt from purge).
    pub released_at: Option<DateTime<Utc>>,
    pub released_by: Option<String>,
}

impl LegalHold {
    pub fn is_active(&self) -> bool {
        self.released_at.is_none()
    }
}

type HoldRow = (
    String,         // id
    String,         // org_id
    String,         // filter (JSON)
    String,         // reason
    Option<String>, // created_by
    String,         // created_at
    Option<String>, // released_at
    Option<String>, // released_by
);

const HOLD_COLS: &str =
    "id, org_id, filter, reason, created_by, created_at, released_at, released_by";

pub struct AuditGovernanceRepository<'a> {
    db: &'a Database,
}

impl<'a> AuditGovernanceRepository<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    fn parse_ts(s: &str) -> Result<DateTime<Utc>, DatabaseError> {
        DateTime::parse_from_rfc3339(s)
            .map(|d| d.with_timezone(&Utc))
            .map_err(|e| DatabaseError::Config(format!("bad timestamp in audit governance: {e}")))
    }

    fn row_to_hold(r: HoldRow) -> Result<LegalHold, DatabaseError> {
        let parse_uuid = |s: &str| {
            Uuid::parse_str(s)
                .map_err(|e| DatabaseError::Config(format!("bad uuid in audit governance: {e}")))
        };
        Ok(LegalHold {
            id: parse_uuid(&r.0)?,
            org_id: parse_uuid(&r.1)?,
            // A hold whose stored filter no longer parses must stay MAXIMALLY
            // protective, not silently vanish: fall back to the hold-everything
            // filter rather than erroring the purge into skipping holds.
            filter: serde_json::from_str(&r.2).unwrap_or_default(),
            reason: r.3,
            created_by: r.4,
            created_at: Self::parse_ts(&r.5)?,
            released_at: r.6.as_deref().map(Self::parse_ts).transpose()?,
            released_by: r.7,
        })
    }

    // ---- Retention ----

    /// Get the org's retention setting, if explicitly configured.
    pub async fn get_retention(
        &self,
        org_id: Uuid,
    ) -> Result<Option<AuditRetention>, DatabaseError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;
        let row: Option<(i64, Option<String>, String)> = sqlx::query_as(
            "SELECT days, updated_by, updated_at FROM audit_retention WHERE org_id = $1",
        )
        .bind(org_id.to_string())
        .fetch_optional(pool)
        .await?;
        row.map(|(days, updated_by, updated_at)| {
            Ok(AuditRetention {
                org_id,
                days,
                updated_by,
                updated_at: Self::parse_ts(&updated_at)?,
            })
        })
        .transpose()
    }

    /// Set (upsert) the org's retention window in days.
    pub async fn set_retention(
        &self,
        org_id: Uuid,
        days: i64,
        updated_by: Option<&str>,
    ) -> Result<AuditRetention, DatabaseError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO audit_retention (org_id, days, updated_by, updated_at) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT (org_id) DO UPDATE SET \
             days = excluded.days, updated_by = excluded.updated_by, \
             updated_at = excluded.updated_at",
        )
        .bind(org_id.to_string())
        .bind(days)
        .bind(updated_by)
        .bind(now.to_rfc3339())
        .execute(pool)
        .await?;
        Ok(AuditRetention {
            org_id,
            days,
            updated_by: updated_by.map(str::to_string),
            updated_at: now,
        })
    }

    /// All explicit retention settings (the purge sweeper's work list).
    pub async fn list_retention(&self) -> Result<Vec<AuditRetention>, DatabaseError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;
        let rows: Vec<(String, i64, Option<String>, String)> =
            sqlx::query_as("SELECT org_id, days, updated_by, updated_at FROM audit_retention")
                .fetch_all(pool)
                .await?;
        rows.into_iter()
            .map(|(org_id, days, updated_by, updated_at)| {
                Ok(AuditRetention {
                    org_id: Uuid::parse_str(&org_id).map_err(|e| {
                        DatabaseError::Config(format!("bad org id in audit_retention: {e}"))
                    })?,
                    days,
                    updated_by,
                    updated_at: Self::parse_ts(&updated_at)?,
                })
            })
            .collect()
    }

    // ---- Legal holds ----

    /// Place a legal hold. Decisions matching `filter` become exempt from
    /// retention purge until the hold is released.
    pub async fn create_hold(
        &self,
        org_id: Uuid,
        filter: &HoldFilter,
        reason: &str,
        created_by: Option<&str>,
    ) -> Result<LegalHold, DatabaseError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;
        let id = Uuid::new_v4();
        let now = Utc::now();
        let filter_json = serde_json::to_string(filter)
            .map_err(|e| DatabaseError::Config(format!("serialize hold filter: {e}")))?;
        sqlx::query(
            "INSERT INTO audit_legal_holds (id, org_id, filter, reason, created_by, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(id.to_string())
        .bind(org_id.to_string())
        .bind(&filter_json)
        .bind(reason)
        .bind(created_by)
        .bind(now.to_rfc3339())
        .execute(pool)
        .await?;
        Ok(LegalHold {
            id,
            org_id,
            filter: filter.clone(),
            reason: reason.to_string(),
            created_by: created_by.map(str::to_string),
            created_at: now,
            released_at: None,
            released_by: None,
        })
    }

    /// List an org's holds, newest first (active and released — the released
    /// ones are part of the compliance record).
    pub async fn list_holds(&self, org_id: Uuid) -> Result<Vec<LegalHold>, DatabaseError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;
        let rows: Vec<HoldRow> = sqlx::query_as(&format!(
            "SELECT {HOLD_COLS} FROM audit_legal_holds WHERE org_id = $1 \
             ORDER BY created_at DESC"
        ))
        .bind(org_id.to_string())
        .fetch_all(pool)
        .await?;
        rows.into_iter().map(Self::row_to_hold).collect()
    }

    /// Active (unreleased) holds for an org — what the purge must honor.
    pub async fn active_holds(&self, org_id: Uuid) -> Result<Vec<LegalHold>, DatabaseError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;
        let rows: Vec<HoldRow> = sqlx::query_as(&format!(
            "SELECT {HOLD_COLS} FROM audit_legal_holds \
             WHERE org_id = $1 AND released_at IS NULL ORDER BY created_at DESC"
        ))
        .bind(org_id.to_string())
        .fetch_all(pool)
        .await?;
        rows.into_iter().map(Self::row_to_hold).collect()
    }

    /// Fetch one hold, tenant-scoped.
    pub async fn get_hold(
        &self,
        org_id: Uuid,
        hold_id: Uuid,
    ) -> Result<Option<LegalHold>, DatabaseError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;
        let row: Option<HoldRow> = sqlx::query_as(&format!(
            "SELECT {HOLD_COLS} FROM audit_legal_holds WHERE id = $1 AND org_id = $2"
        ))
        .bind(hold_id.to_string())
        .bind(org_id.to_string())
        .fetch_optional(pool)
        .await?;
        row.map(Self::row_to_hold).transpose()
    }

    /// Release a hold (tenant-scoped). Returns false when the hold does not
    /// exist for this org or was already released — release is not idempotent
    /// on purpose, so a double-release shows up instead of masking races.
    pub async fn release_hold(
        &self,
        org_id: Uuid,
        hold_id: Uuid,
        released_by: Option<&str>,
    ) -> Result<bool, DatabaseError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;
        let result = sqlx::query(
            "UPDATE audit_legal_holds SET released_at = $1, released_by = $2 \
             WHERE id = $3 AND org_id = $4 AND released_at IS NULL",
        )
        .bind(Utc::now().to_rfc3339())
        .bind(released_by)
        .bind(hold_id.to_string())
        .bind(org_id.to_string())
        .execute(pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }
}
