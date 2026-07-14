//! Subject-erasure receipt persistence (E2 follow-up #3).
//!
//! A queryable history of GDPR Art. 17 erasures, complementary to the
//! append-only `audit.subject_erasure` trail entry (which remains the primary
//! durable proof). One row per completed `POST /orgs/{org}/audit/erasure`: the
//! full `ErasureReceipt` JSON is stored verbatim in `receipt`, with the columns
//! beside it decomposed only so the history is filterable ("every erasure for
//! org X / subject Y, with its outcome").
//!
//! Recorded best-effort, *after* the irreversible erasure completes, from inside
//! the idempotency-guarded op — so a write hiccup never fails an erasure that
//! already ran, and an `Idempotency-Key` replay never double-inserts.

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use crate::db::{Database, DatabaseError};

/// A recorded subject-erasure, as stored and returned by the history endpoint.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ErasureRecord {
    pub id: Uuid,
    pub org_id: Uuid,
    pub subject: String,
    pub requested_by: Option<String>,
    pub requested_at: DateTime<Utc>,
    /// `submitted` | `deferred_blanket_hold` | `store_not_configured`.
    pub decision_log_status: String,
    /// Active holds honored by the redaction (present only when `submitted`).
    pub holds_honored: Option<i64>,
    /// Whether pseudonymised (`sha256:<hmac>`) columns were also matched.
    pub matched_pseudonyms: bool,
    /// `erased` | `skipped`.
    pub datastore_status: String,
    pub datastores_scanned: i64,
    pub entities_deleted: i64,
    /// Post-erasure store verification posture (always `linkage` today).
    pub verification_posture: String,
    /// The full `ErasureReceipt` JSON as shipped to the caller and audit trail
    /// (includes the immutable-surface `exemptions[]`).
    #[schema(value_type = Object)]
    pub receipt: Value,
    pub completed_at: DateTime<Utc>,
}

/// The decomposed fields captured when recording a completed erasure. Borrowed
/// so the caller can hand over slices of its already-built receipt without
/// cloning; `id`/`requested_at`/`completed_at` are assigned by [`record`].
#[derive(Debug, Clone)]
pub struct NewErasureRecord<'a> {
    pub org_id: Uuid,
    pub subject: &'a str,
    pub requested_by: Option<&'a str>,
    pub decision_log_status: &'a str,
    pub holds_honored: Option<i64>,
    pub matched_pseudonyms: bool,
    pub datastore_status: &'a str,
    pub datastores_scanned: i64,
    pub entities_deleted: i64,
    pub verification_posture: &'a str,
    pub receipt: &'a Value,
}

/// Default page size for erasure history, and the hard cap.
const DEFAULT_HISTORY_LIMIT: i64 = 100;
const MAX_HISTORY_LIMIT: i64 = 500;

type ErasureRow = (
    String,         // id
    String,         // org_id
    String,         // subject
    Option<String>, // requested_by
    String,         // requested_at
    String,         // decision_log_status
    Option<i64>,    // holds_honored
    i64,            // matched_pseudonyms (0/1)
    String,         // datastore_status
    i64,            // datastores_scanned
    i64,            // entities_deleted
    String,         // verification_posture
    String,         // receipt (JSON)
    String,         // completed_at
);

const ERASURE_COLS: &str = "id, org_id, subject, requested_by, requested_at, decision_log_status, \
     holds_honored, matched_pseudonyms, datastore_status, datastores_scanned, entities_deleted, \
     verification_posture, receipt, completed_at";

pub struct AuditErasureRepository<'a> {
    db: &'a Database,
}

impl<'a> AuditErasureRepository<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    fn parse_ts(s: &str) -> Result<DateTime<Utc>, DatabaseError> {
        DateTime::parse_from_rfc3339(s)
            .map(|d| d.with_timezone(&Utc))
            .map_err(|e| DatabaseError::Config(format!("bad timestamp in audit_erasure: {e}")))
    }

    fn row_to_record(r: ErasureRow) -> Result<ErasureRecord, DatabaseError> {
        let parse_uuid = |s: &str| {
            Uuid::parse_str(s)
                .map_err(|e| DatabaseError::Config(format!("bad uuid in audit_erasure: {e}")))
        };
        Ok(ErasureRecord {
            id: parse_uuid(&r.0)?,
            org_id: parse_uuid(&r.1)?,
            subject: r.2,
            requested_by: r.3,
            requested_at: Self::parse_ts(&r.4)?,
            decision_log_status: r.5,
            holds_honored: r.6,
            matched_pseudonyms: r.7 != 0,
            datastore_status: r.8,
            datastores_scanned: r.9,
            entities_deleted: r.10,
            verification_posture: r.11,
            // A receipt that no longer parses is still surfaced (Null) rather
            // than erroring the whole history read — the decomposed columns hold
            // the queryable facts regardless.
            receipt: serde_json::from_str(&r.12).unwrap_or(Value::Null),
            completed_at: Self::parse_ts(&r.13)?,
        })
    }

    /// Persist one completed erasure, assigning the id and timestamps.
    pub async fn record(
        &self,
        input: NewErasureRecord<'_>,
    ) -> Result<ErasureRecord, DatabaseError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;
        let id = Uuid::new_v4();
        let now = Utc::now();
        let receipt_json = serde_json::to_string(input.receipt)
            .map_err(|e| DatabaseError::Config(format!("serialize erasure receipt: {e}")))?;
        sqlx::query(
            "INSERT INTO audit_erasure_requests \
             (id, org_id, subject, requested_by, requested_at, decision_log_status, \
              holds_honored, matched_pseudonyms, datastore_status, datastores_scanned, \
              entities_deleted, verification_posture, receipt, completed_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
        )
        .bind(id.to_string())
        .bind(input.org_id.to_string())
        .bind(input.subject)
        .bind(input.requested_by)
        .bind(now.to_rfc3339())
        .bind(input.decision_log_status)
        .bind(input.holds_honored)
        .bind(i64::from(input.matched_pseudonyms))
        .bind(input.datastore_status)
        .bind(input.datastores_scanned)
        .bind(input.entities_deleted)
        .bind(input.verification_posture)
        .bind(&receipt_json)
        .bind(now.to_rfc3339())
        .execute(pool)
        .await?;
        Ok(ErasureRecord {
            id,
            org_id: input.org_id,
            subject: input.subject.to_string(),
            requested_by: input.requested_by.map(str::to_string),
            requested_at: now,
            decision_log_status: input.decision_log_status.to_string(),
            holds_honored: input.holds_honored,
            matched_pseudonyms: input.matched_pseudonyms,
            datastore_status: input.datastore_status.to_string(),
            datastores_scanned: input.datastores_scanned,
            entities_deleted: input.entities_deleted,
            verification_posture: input.verification_posture.to_string(),
            receipt: input.receipt.clone(),
            completed_at: now,
        })
    }

    /// List an org's erasure history, newest first. `limit` is clamped to
    /// `[1, MAX_HISTORY_LIMIT]`, defaulting to `DEFAULT_HISTORY_LIMIT` — this is
    /// a low-volume, manually-driven table, so a bounded most-recent window
    /// (never an unbounded scan) is the right shape.
    pub async fn list_for_org(
        &self,
        org_id: Uuid,
        limit: Option<i64>,
    ) -> Result<Vec<ErasureRecord>, DatabaseError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;
        let limit = limit
            .unwrap_or(DEFAULT_HISTORY_LIMIT)
            .clamp(1, MAX_HISTORY_LIMIT);
        let rows: Vec<ErasureRow> = sqlx::query_as(&format!(
            "SELECT {ERASURE_COLS} FROM audit_erasure_requests WHERE org_id = $1 \
             ORDER BY requested_at DESC, id DESC LIMIT $2"
        ))
        .bind(org_id.to_string())
        .bind(limit)
        .fetch_all(pool)
        .await?;
        rows.into_iter().map(Self::row_to_record).collect()
    }
}
