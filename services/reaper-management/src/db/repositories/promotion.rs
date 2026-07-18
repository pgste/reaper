//! Promotion change-request repository (Plan 02, Phase B, step 5).

use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

use crate::db::{Database, DatabaseError};
use crate::domain::promotion::{ChangeKind, ChangeStatus, PromotionChangeRequest};

pub struct PromotionChangeRepository<'a> {
    db: &'a Database,
}

impl<'a> PromotionChangeRepository<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    fn pool(&self) -> Result<&sqlx::AnyPool, DatabaseError> {
        self.db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))
    }

    fn row_to_cr(row: &sqlx::any::AnyRow) -> Result<PromotionChangeRequest, DatabaseError> {
        let parse_uuid =
            |s: String| Uuid::parse_str(&s).map_err(|e| DatabaseError::Config(e.to_string()));
        let parse_ts = |s: String| {
            DateTime::parse_from_rfc3339(&s)
                .map(|d| d.with_timezone(&Utc))
                .map_err(|e| DatabaseError::Config(e.to_string()))
        };
        Ok(PromotionChangeRequest {
            id: parse_uuid(row.get("id"))?,
            org_id: parse_uuid(row.get("org_id"))?,
            bundle_id: parse_uuid(row.get("bundle_id"))?,
            bundle_version: row.get("bundle_version"),
            kind: ChangeKind::parse(&row.get::<String, _>("kind"))
                .ok_or_else(|| DatabaseError::Config("bad change kind".into()))?,
            status: ChangeStatus::parse(&row.get::<String, _>("status"))
                .ok_or_else(|| DatabaseError::Config("bad change status".into()))?,
            requester_id: row.get("requester_id"),
            approver_id: row.get("approver_id"),
            notes: row.get("notes"),
            created_at: parse_ts(row.get("created_at"))?,
            decided_at: row
                .get::<Option<String>, _>("decided_at")
                .map(parse_ts)
                .transpose()?,
        })
    }

    const COLS: &'static str = "id, org_id, bundle_id, bundle_version, kind, status, \
         requester_id, approver_id, notes, created_at, decided_at";

    /// Open a new pending change request.
    #[allow(clippy::too_many_arguments)]
    pub async fn create(
        &self,
        org_id: Uuid,
        bundle_id: Uuid,
        bundle_version: Option<&str>,
        kind: ChangeKind,
        requester_id: &str,
        notes: Option<&str>,
    ) -> Result<PromotionChangeRequest, DatabaseError> {
        let pool = self.pool()?;
        let id = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO promotion_change_requests \
             (id, org_id, bundle_id, bundle_version, kind, status, requester_id, notes, created_at) \
             VALUES ($1, $2, $3, $4, $5, 'pending', $6, $7, $8)",
        )
        .bind(id.to_string())
        .bind(org_id.to_string())
        .bind(bundle_id.to_string())
        .bind(bundle_version)
        .bind(kind.as_str())
        .bind(requester_id)
        .bind(notes)
        .bind(&now)
        .execute(pool)
        .await?;
        self.get_scoped(org_id, id)
            .await?
            .ok_or_else(|| DatabaseError::NotFound("change request not found after create".into()))
    }

    /// Fetch a change request scoped to an org (tenant-safe: another org's id
    /// resolves to `None`).
    pub async fn get_scoped(
        &self,
        org_id: Uuid,
        id: Uuid,
    ) -> Result<Option<PromotionChangeRequest>, DatabaseError> {
        let pool = self.pool()?;
        let sql = format!(
            "SELECT {} FROM promotion_change_requests WHERE id = $1 AND org_id = $2",
            Self::COLS
        );
        let row = sqlx::query(&sql)
            .bind(id.to_string())
            .bind(org_id.to_string())
            .fetch_optional(pool)
            .await?;
        row.as_ref().map(Self::row_to_cr).transpose()
    }

    /// List an org's change requests, newest first.
    pub async fn list(
        &self,
        org_id: Uuid,
        limit: i64,
    ) -> Result<Vec<PromotionChangeRequest>, DatabaseError> {
        let pool = self.pool()?;
        // Bounded cap so the change-request list is never unbounded (round-3
        // Plan 06 §4.2, R3-02).
        let sql = format!(
            "SELECT {} FROM promotion_change_requests WHERE org_id = $1 ORDER BY created_at DESC LIMIT $2",
            Self::COLS
        );
        let rows = sqlx::query(&sql)
            .bind(org_id.to_string())
            .bind(limit)
            .fetch_all(pool)
            .await?;
        rows.iter().map(Self::row_to_cr).collect()
    }

    /// Atomically move a request from pending → executed, stamping the
    /// approver. Returns the number of rows updated: 0 means it was not
    /// pending (already decided / raced), so the caller must treat that as a
    /// conflict rather than proceeding with the promotion.
    pub async fn mark_executed(
        &self,
        org_id: Uuid,
        id: Uuid,
        approver_id: &str,
    ) -> Result<u64, DatabaseError> {
        let pool = self.pool()?;
        let now = Utc::now().to_rfc3339();
        let res = sqlx::query(
            "UPDATE promotion_change_requests \
             SET status = 'executed', approver_id = $1, decided_at = $2 \
             WHERE id = $3 AND org_id = $4 AND status = 'pending'",
        )
        .bind(approver_id)
        .bind(&now)
        .bind(id.to_string())
        .bind(org_id.to_string())
        .execute(pool)
        .await?;
        Ok(res.rows_affected())
    }

    /// Reject a pending request. Returns rows updated (0 = not pending).
    pub async fn mark_rejected(
        &self,
        org_id: Uuid,
        id: Uuid,
        actor_id: &str,
    ) -> Result<u64, DatabaseError> {
        let pool = self.pool()?;
        let now = Utc::now().to_rfc3339();
        let res = sqlx::query(
            "UPDATE promotion_change_requests \
             SET status = 'rejected', approver_id = $1, decided_at = $2 \
             WHERE id = $3 AND org_id = $4 AND status = 'pending'",
        )
        .bind(actor_id)
        .bind(&now)
        .bind(id.to_string())
        .bind(org_id.to_string())
        .execute(pool)
        .await?;
        Ok(res.rows_affected())
    }
}
