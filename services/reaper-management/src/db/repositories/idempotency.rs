//! Idempotency-key persistence (Plan 07, Phase D).
//!
//! Claim-then-complete: [`try_claim`](IdempotencyRepository::try_claim) INSERTs
//! a `pending` row under the `(scope, scope_id, idem_key)` unique constraint —
//! the database is the arbiter, so exactly one of two concurrent requests with
//! the same key owns the operation. The owner runs the side effect and
//! [`complete`](IdempotencyRepository::complete)s the row with the response;
//! the other caller observes `pending` (409, still in flight) or `completed`
//! (replay of the stored response). Rows expire after the retention window and
//! are pruned by the sweeper in `main.rs`.

use chrono::{Duration, Utc};
use sqlx::Row;
use uuid::Uuid;

use crate::db::{Database, DatabaseError};

/// A previously-claimed idempotency record.
#[derive(Debug, Clone)]
pub struct IdempotencyRecord {
    pub request_hash: String,
    pub status: String,
    pub response_status: Option<i32>,
    pub response_body: Option<String>,
    pub expires_at: String,
}

/// How the claim attempt resolved.
#[derive(Debug)]
pub enum ClaimOutcome {
    /// This caller owns the operation: run it, then `complete` (or `release`
    /// on failure so the key can be retried).
    Claimed,
    /// The key already exists within its retention window.
    Existing(IdempotencyRecord),
}

pub struct IdempotencyRepository<'a> {
    db: &'a Database,
}

impl<'a> IdempotencyRepository<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// Try to claim `(scope, scope_id, key)` for this request. Expired rows
    /// are treated as absent (deleted, then re-claimed).
    pub async fn try_claim(
        &self,
        scope: &str,
        scope_id: &str,
        key: &str,
        request_hash: &str,
        retention: Duration,
    ) -> Result<ClaimOutcome, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;

        let now = Utc::now();
        // Two attempts: if the first INSERT conflicts with an EXPIRED row, we
        // delete it and claim once more. A second conflict is returned as-is.
        for attempt in 0..2 {
            let insert = sqlx::query(
                r#"
                INSERT INTO idempotency_keys
                    (id, scope, scope_id, idem_key, request_hash, status, created_at, expires_at)
                VALUES ($1, $2, $3, $4, $5, 'pending', $6, $7)
                "#,
            )
            .bind(Uuid::new_v4().to_string())
            .bind(scope)
            .bind(scope_id)
            .bind(key)
            .bind(request_hash)
            .bind(now.to_rfc3339())
            .bind((now + retention).to_rfc3339())
            .execute(pool)
            .await;

            match insert {
                Ok(_) => return Ok(ClaimOutcome::Claimed),
                Err(e) => {
                    // Unique-constraint conflict → the key exists; anything
                    // else is a real database error.
                    let msg = e.to_string().to_lowercase();
                    if !(msg.contains("unique") || msg.contains("duplicate")) {
                        return Err(e.into());
                    }
                }
            }

            let row = sqlx::query(
                r#"
                SELECT request_hash, status, response_status, response_body, expires_at
                FROM idempotency_keys
                WHERE scope = $1 AND scope_id = $2 AND idem_key = $3
                "#,
            )
            .bind(scope)
            .bind(scope_id)
            .bind(key)
            .fetch_optional(pool)
            .await?;

            // The conflicting row may have been pruned between the INSERT and
            // this read — loop and claim again.
            let Some(row) = row else { continue };

            let record = IdempotencyRecord {
                request_hash: row.get("request_hash"),
                status: row.get("status"),
                response_status: row.get("response_status"),
                response_body: row.get("response_body"),
                expires_at: row.get("expires_at"),
            };

            let expired = chrono::DateTime::parse_from_rfc3339(&record.expires_at)
                .map(|t| t < now)
                .unwrap_or(true);
            if expired && attempt == 0 {
                self.release(scope, scope_id, key).await?;
                continue;
            }
            return Ok(ClaimOutcome::Existing(record));
        }

        // Both claim attempts conflicted with rows that vanished before we
        // could read them — treat as contention.
        Err(DatabaseError::VersionConflict(format!(
            "idempotency key {key} is contended"
        )))
    }

    /// Store the operation's outcome so replays can return it verbatim.
    pub async fn complete(
        &self,
        scope: &str,
        scope_id: &str,
        key: &str,
        response_status: u16,
        response_body: &str,
    ) -> Result<(), DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;
        sqlx::query(
            r#"
            UPDATE idempotency_keys
            SET status = 'completed', response_status = $1, response_body = $2
            WHERE scope = $3 AND scope_id = $4 AND idem_key = $5
            "#,
        )
        .bind(response_status as i32)
        .bind(response_body)
        .bind(scope)
        .bind(scope_id)
        .bind(key)
        .execute(pool)
        .await?;
        Ok(())
    }

    /// Drop the claim (the operation failed) so the client may retry the key.
    pub async fn release(
        &self,
        scope: &str,
        scope_id: &str,
        key: &str,
    ) -> Result<(), DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;
        sqlx::query(
            "DELETE FROM idempotency_keys WHERE scope = $1 AND scope_id = $2 AND idem_key = $3",
        )
        .bind(scope)
        .bind(scope_id)
        .bind(key)
        .execute(pool)
        .await?;
        Ok(())
    }

    /// Age out expired keys; returns how many were removed. Called by the
    /// retention sweeper.
    pub async fn prune_expired(&self, cutoff_rfc3339: &str) -> Result<u64, DatabaseError> {
        let pool = self
            .db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))?;
        let result = sqlx::query("DELETE FROM idempotency_keys WHERE expires_at < $1")
            .bind(cutoff_rfc3339)
            .execute(pool)
            .await?;
        Ok(result.rows_affected())
    }
}
