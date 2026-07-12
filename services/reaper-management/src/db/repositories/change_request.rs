//! Change request repository (Plan 10 Phase B).

use chrono::Utc;
use sqlx::Row;
use uuid::Uuid;

use crate::db::{Database, DatabaseError};
use crate::domain::change_request::{
    ApprovalDecision, ChangeApproval, ChangeRequest, ChangeRequestStatus, CreateChangeRequest,
};

pub struct ChangeRequestRepository<'a> {
    db: &'a Database,
}

impl<'a> ChangeRequestRepository<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    pub async fn create(
        &self,
        org_id: Uuid,
        input: CreateChangeRequest,
    ) -> Result<ChangeRequest, DatabaseError> {
        let pool = self.pool()?;
        let id = Uuid::new_v4();
        let now = Utc::now();

        sqlx::query(
            r#"
            INSERT INTO change_requests
                (id, org_id, from_env_id, to_env_id, bundle_id, data_version,
                 strategy_id, status, requested_by, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, 'pending', $8, $9)
            "#,
        )
        .bind(id.to_string())
        .bind(org_id.to_string())
        .bind(input.from_env_id.to_string())
        .bind(input.to_env_id.to_string())
        .bind(input.bundle_id.to_string())
        .bind(input.data_version)
        .bind(input.strategy_id.map(|s| s.to_string()))
        .bind(&input.requested_by)
        .bind(now.to_rfc3339())
        .execute(pool)
        .await?;

        Ok(ChangeRequest {
            id,
            org_id,
            from_env_id: input.from_env_id,
            to_env_id: input.to_env_id,
            bundle_id: input.bundle_id,
            data_version: input.data_version,
            strategy_id: input.strategy_id,
            status: ChangeRequestStatus::Pending,
            requested_by: input.requested_by,
            rollout_id: None,
            reason: None,
            created_at: now,
            decided_at: None,
        })
    }

    pub async fn get(&self, id: Uuid) -> Result<Option<ChangeRequest>, DatabaseError> {
        let pool = self.pool()?;
        let row = sqlx::query(&format!("{CR_COLUMNS} WHERE id = $1"))
            .bind(id.to_string())
            .fetch_optional(pool)
            .await?;
        row.map(|r| Self::row_to_cr(&r)).transpose()
    }

    pub async fn list_by_org(
        &self,
        org_id: Uuid,
        status: Option<ChangeRequestStatus>,
    ) -> Result<Vec<ChangeRequest>, DatabaseError> {
        let pool = self.pool()?;
        let rows = match status {
            Some(s) => {
                sqlx::query(&format!(
                    "{CR_COLUMNS} WHERE org_id = $1 AND status = $2 ORDER BY created_at DESC"
                ))
                .bind(org_id.to_string())
                .bind(s.as_str())
                .fetch_all(pool)
                .await?
            }
            None => {
                sqlx::query(&format!(
                    "{CR_COLUMNS} WHERE org_id = $1 ORDER BY created_at DESC"
                ))
                .bind(org_id.to_string())
                .fetch_all(pool)
                .await?
            }
        };
        rows.iter().map(Self::row_to_cr).collect()
    }

    /// Keyset-paginated listing (Plan 07 pattern; Plan 10 Step 8): rows
    /// strictly after the `(created_at, id)` position in
    /// `ORDER BY created_at DESC, id DESC` order, optionally filtered by
    /// status. `fetch` is page limit + 1 — the caller uses the sentinel row
    /// to detect whether another page exists.
    pub async fn list_page_by_org(
        &self,
        org_id: Uuid,
        status: Option<ChangeRequestStatus>,
        fetch: i64,
        after: Option<&(String, String)>,
    ) -> Result<Vec<ChangeRequest>, DatabaseError> {
        let pool = self.pool()?;

        // Build the query with positional binds in a fixed order:
        // org_id [, status] [, created_at, id], fetch.
        let mut sql = format!("{CR_COLUMNS} WHERE org_id = $1");
        let mut next = 2;
        if status.is_some() {
            sql.push_str(&format!(" AND status = ${next}"));
            next += 1;
        }
        if after.is_some() {
            sql.push_str(&format!(" AND (created_at, id) < (${next}, ${})", next + 1));
            next += 2;
        }
        sql.push_str(&format!(" ORDER BY created_at DESC, id DESC LIMIT ${next}"));

        let mut query = sqlx::query(&sql).bind(org_id.to_string());
        if let Some(s) = status {
            query = query.bind(s.as_str());
        }
        if let Some((created_at, id)) = after {
            query = query.bind(created_at).bind(id);
        }
        let rows = query.bind(fetch).fetch_all(pool).await?;
        rows.iter().map(Self::row_to_cr).collect()
    }

    /// Record (or update) an approver's decision. One row per approver per
    /// request; a re-vote replaces the prior decision.
    pub async fn record_decision(
        &self,
        change_request_id: Uuid,
        approver_id: &str,
        decision: ApprovalDecision,
        reason: Option<&str>,
    ) -> Result<(), DatabaseError> {
        let pool = self.pool()?;
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r#"
            INSERT INTO change_request_approvals
                (id, change_request_id, approver_id, decision, reason, created_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT(change_request_id, approver_id) DO UPDATE SET
                decision = excluded.decision,
                reason = excluded.reason,
                created_at = excluded.created_at
            "#,
        )
        .bind(Uuid::new_v4().to_string())
        .bind(change_request_id.to_string())
        .bind(approver_id)
        .bind(decision.as_str())
        .bind(reason)
        .bind(&now)
        .execute(pool)
        .await?;
        Ok(())
    }

    pub async fn list_approvals(
        &self,
        change_request_id: Uuid,
    ) -> Result<Vec<ChangeApproval>, DatabaseError> {
        let pool = self.pool()?;
        let rows = sqlx::query(
            r#"
            SELECT id, change_request_id, approver_id, decision, reason, created_at
            FROM change_request_approvals
            WHERE change_request_id = $1
            ORDER BY created_at ASC
            "#,
        )
        .bind(change_request_id.to_string())
        .fetch_all(pool)
        .await?;

        rows.iter()
            .map(|row| {
                let id: String = row.get("id");
                let cr_id: String = row.get("change_request_id");
                let decision: String = row.get("decision");
                let created_at: String = row.get("created_at");
                Ok(ChangeApproval {
                    id: parse_uuid(&id)?,
                    change_request_id: parse_uuid(&cr_id)?,
                    approver_id: row.get("approver_id"),
                    decision: ApprovalDecision::parse(&decision),
                    reason: row.get("reason"),
                    created_at: parse_ts(&created_at)?,
                })
            })
            .collect()
    }

    /// Move a change request to a terminal/next status, stamping `decided_at`
    /// and (optionally) the resulting rollout id or a reason.
    pub async fn set_status(
        &self,
        id: Uuid,
        status: ChangeRequestStatus,
        rollout_id: Option<Uuid>,
        reason: Option<&str>,
    ) -> Result<(), DatabaseError> {
        let pool = self.pool()?;
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r#"
            UPDATE change_requests
            SET status = $1, rollout_id = COALESCE($2, rollout_id),
                reason = COALESCE($3, reason), decided_at = $4
            WHERE id = $5
            "#,
        )
        .bind(status.as_str())
        .bind(rollout_id.map(|r| r.to_string()))
        .bind(reason)
        .bind(&now)
        .bind(id.to_string())
        .execute(pool)
        .await?;
        Ok(())
    }

    fn pool(&self) -> Result<&sqlx::AnyPool, DatabaseError> {
        self.db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))
    }

    fn row_to_cr(row: &sqlx::any::AnyRow) -> Result<ChangeRequest, DatabaseError> {
        let id: String = row.get("id");
        let org_id: String = row.get("org_id");
        let from_env_id: String = row.get("from_env_id");
        let to_env_id: String = row.get("to_env_id");
        let bundle_id: String = row.get("bundle_id");
        let strategy_id: Option<String> = row.get("strategy_id");
        let rollout_id: Option<String> = row.get("rollout_id");
        let status: String = row.get("status");
        let created_at: String = row.get("created_at");
        let decided_at: Option<String> = row.get("decided_at");

        Ok(ChangeRequest {
            id: parse_uuid(&id)?,
            org_id: parse_uuid(&org_id)?,
            from_env_id: parse_uuid(&from_env_id)?,
            to_env_id: parse_uuid(&to_env_id)?,
            bundle_id: parse_uuid(&bundle_id)?,
            data_version: row.get("data_version"),
            strategy_id: strategy_id.as_deref().map(parse_uuid).transpose()?,
            status: ChangeRequestStatus::parse(&status),
            requested_by: row.get("requested_by"),
            rollout_id: rollout_id.as_deref().map(parse_uuid).transpose()?,
            reason: row.get("reason"),
            created_at: parse_ts(&created_at)?,
            decided_at: decided_at.as_deref().map(parse_ts).transpose()?,
        })
    }
}

const CR_COLUMNS: &str = r#"
    SELECT id, org_id, from_env_id, to_env_id, bundle_id, data_version,
           strategy_id, status, requested_by, rollout_id, reason, created_at, decided_at
    FROM change_requests
"#;

fn parse_uuid(s: &str) -> Result<Uuid, DatabaseError> {
    Uuid::parse_str(s).map_err(|e| DatabaseError::Config(format!("Invalid UUID: {e}")))
}

fn parse_ts(s: &str) -> Result<chrono::DateTime<chrono::Utc>, DatabaseError> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map_err(|e| DatabaseError::Config(format!("Invalid timestamp: {e}")))
        .map(|dt| dt.with_timezone(&chrono::Utc))
}
