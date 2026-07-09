//! Revocation repository (Plan 02, Phase B, step 4).
//!
//! Per-org list of revoked bundle hashes / signing key ids, plus a monotonic
//! `serial` bumped on every change so an agent can reject a replayed old list.

use chrono::Utc;
use sqlx::Row;
use uuid::Uuid;

use crate::db::{Database, DatabaseError};

/// One revocation entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevocationEntry {
    pub kind: RevocationKind,
    pub value: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevocationKind {
    Hash,
    KeyId,
}

impl RevocationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RevocationKind::Hash => "hash",
            RevocationKind::KeyId => "key_id",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "hash" => Some(RevocationKind::Hash),
            "key_id" => Some(RevocationKind::KeyId),
            _ => None,
        }
    }
}

/// The org's current revocation set plus its serial.
#[derive(Debug, Clone, Default)]
pub struct RevocationSet {
    pub serial: i64,
    pub updated_at: Option<String>,
    pub hashes: Vec<String>,
    pub key_ids: Vec<String>,
}

pub struct RevocationRepository<'a> {
    db: &'a Database,
}

impl<'a> RevocationRepository<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    fn pool(&self) -> Result<&sqlx::AnyPool, DatabaseError> {
        self.db
            .any_pool()
            .ok_or_else(|| DatabaseError::Config("No database pool".to_string()))
    }

    /// Add a revocation and bump the org's serial. Idempotent on
    /// (org, kind, value): re-revoking the same thing still bumps the serial
    /// so agents refetch, but does not duplicate the row.
    pub async fn add(&self, org_id: Uuid, entry: &RevocationEntry) -> Result<i64, DatabaseError> {
        let pool = self.pool()?;
        let now = Utc::now().to_rfc3339();
        // Insert-or-ignore the entry.
        sqlx::query(
            "INSERT INTO bundle_revocations (id, org_id, kind, value, reason, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6) \
             ON CONFLICT(org_id, kind, value) DO NOTHING",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(org_id.to_string())
        .bind(entry.kind.as_str())
        .bind(&entry.value)
        .bind(&entry.reason)
        .bind(&now)
        .execute(pool)
        .await?;
        self.bump_serial(org_id, &now).await
    }

    /// Remove a revocation and bump the serial (un-revoke).
    pub async fn remove(
        &self,
        org_id: Uuid,
        kind: RevocationKind,
        value: &str,
    ) -> Result<i64, DatabaseError> {
        let pool = self.pool()?;
        sqlx::query(
            "DELETE FROM bundle_revocations WHERE org_id = $1 AND kind = $2 AND value = $3",
        )
        .bind(org_id.to_string())
        .bind(kind.as_str())
        .bind(value)
        .execute(pool)
        .await?;
        self.bump_serial(org_id, &Utc::now().to_rfc3339()).await
    }

    /// Increment (or seed) the per-org serial. Returns the new serial.
    async fn bump_serial(&self, org_id: Uuid, now: &str) -> Result<i64, DatabaseError> {
        let pool = self.pool()?;
        sqlx::query(
            "INSERT INTO revocation_state (org_id, serial, updated_at) \
             VALUES ($1, 1, $2) \
             ON CONFLICT(org_id) DO UPDATE SET \
                serial = revocation_state.serial + 1, updated_at = $2",
        )
        .bind(org_id.to_string())
        .bind(now)
        .execute(pool)
        .await?;
        let row = sqlx::query("SELECT serial FROM revocation_state WHERE org_id = $1")
            .bind(org_id.to_string())
            .fetch_one(pool)
            .await?;
        Ok(row.get::<i64, _>("serial"))
    }

    /// Load the org's full revocation set (serial + all entries).
    pub async fn get_set(&self, org_id: Uuid) -> Result<RevocationSet, DatabaseError> {
        let pool = self.pool()?;
        let mut set = RevocationSet::default();

        if let Some(row) =
            sqlx::query("SELECT serial, updated_at FROM revocation_state WHERE org_id = $1")
                .bind(org_id.to_string())
                .fetch_optional(pool)
                .await?
        {
            set.serial = row.get::<i64, _>("serial");
            set.updated_at = Some(row.get::<String, _>("updated_at"));
        }

        let rows = sqlx::query(
            "SELECT kind, value FROM bundle_revocations WHERE org_id = $1 ORDER BY value",
        )
        .bind(org_id.to_string())
        .fetch_all(pool)
        .await?;
        for row in rows {
            let kind: String = row.get("kind");
            let value: String = row.get("value");
            match RevocationKind::parse(&kind) {
                Some(RevocationKind::Hash) => set.hashes.push(value),
                Some(RevocationKind::KeyId) => set.key_ids.push(value),
                None => {}
            }
        }
        Ok(set)
    }
}
