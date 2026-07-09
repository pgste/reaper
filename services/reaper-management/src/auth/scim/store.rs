//! Persistence for per-org SCIM tokens (`scim_tokens`).

use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::ScimToken;
use crate::auth::users::{hash_token, UserError};
use crate::db::Database;

/// A minted token: the metadata plus the **plaintext** secret, returned exactly
/// once at creation.
pub struct MintedScimToken {
    pub token: ScimToken,
    pub plaintext: String,
}

type Row = (
    String,         // id
    String,         // org_id
    String,         // name
    String,         // token_hash
    Option<String>, // created_by
    String,         // created_at
    Option<String>, // last_used_at
    i32,            // revoked
);

const COLS: &str = "id, org_id, name, token_hash, created_by, created_at, last_used_at, revoked";

pub struct ScimTokenStore<'a> {
    db: &'a Database,
}

impl<'a> ScimTokenStore<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    fn row_to_token(r: Row) -> Result<ScimToken, UserError> {
        let parse_uuid = |s: &str| -> Result<Uuid, UserError> {
            Uuid::parse_str(s).map_err(|_| UserError::NotFound)
        };
        let parse_ts = |s: &str| -> Result<DateTime<Utc>, UserError> {
            DateTime::parse_from_rfc3339(s)
                .map(|d| d.with_timezone(&Utc))
                .map_err(|_| UserError::NotFound)
        };
        Ok(ScimToken {
            id: parse_uuid(&r.0)?,
            org_id: parse_uuid(&r.1)?,
            name: r.2,
            token_hash: r.3,
            created_by: r.4,
            created_at: parse_ts(&r.5)?,
            last_used_at: r.6.and_then(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .map(|d| d.with_timezone(&Utc))
                    .ok()
            }),
            revoked: r.7 != 0,
        })
    }

    /// Mint a new token for an org. The plaintext (prefix `scim_`) is returned
    /// once; only its hash is persisted.
    pub async fn create(
        &self,
        org_id: Uuid,
        name: &str,
        created_by: Option<&str>,
    ) -> Result<MintedScimToken, UserError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;
        let plaintext = generate_scim_token();
        let token_hash = hash_token(&plaintext);
        let id = Uuid::new_v4();
        let now = Utc::now();

        sqlx::query(
            "INSERT INTO scim_tokens (id, org_id, name, token_hash, created_by, created_at, revoked) \
             VALUES ($1, $2, $3, $4, $5, $6, 0)",
        )
        .bind(id.to_string())
        .bind(org_id.to_string())
        .bind(name)
        .bind(&token_hash)
        .bind(created_by)
        .bind(now.to_rfc3339())
        .execute(pool)
        .await?;

        Ok(MintedScimToken {
            token: ScimToken {
                id,
                org_id,
                name: name.to_string(),
                token_hash,
                created_by: created_by.map(|s| s.to_string()),
                created_at: now,
                last_used_at: None,
                revoked: false,
            },
            plaintext,
        })
    }

    /// Resolve a presented bearer token to its (non-revoked) record. Returns
    /// `None` for an unknown or revoked token — this is the SCIM auth check, and
    /// the org it returns is the only tenant that token can act on.
    pub async fn authenticate(&self, presented: &str) -> Result<Option<ScimToken>, UserError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;
        let hash = hash_token(presented);
        let sql = format!("SELECT {COLS} FROM scim_tokens WHERE token_hash = $1");
        let row: Option<Row> = sqlx::query_as(&sql)
            .bind(&hash)
            .fetch_optional(pool)
            .await?;
        let token = row.map(Self::row_to_token).transpose()?;
        match token {
            Some(t) if !t.revoked => {
                let _ = self.touch_last_used(t.id).await;
                Ok(Some(t))
            }
            _ => Ok(None),
        }
    }

    async fn touch_last_used(&self, id: Uuid) -> Result<(), UserError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;
        sqlx::query("UPDATE scim_tokens SET last_used_at = $1 WHERE id = $2")
            .bind(Utc::now().to_rfc3339())
            .bind(id.to_string())
            .execute(pool)
            .await?;
        Ok(())
    }

    /// List an org's tokens (metadata only; hashes never leave the store).
    pub async fn list(&self, org_id: Uuid) -> Result<Vec<ScimToken>, UserError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;
        let sql =
            format!("SELECT {COLS} FROM scim_tokens WHERE org_id = $1 ORDER BY created_at DESC");
        let rows: Vec<Row> = sqlx::query_as(&sql)
            .bind(org_id.to_string())
            .fetch_all(pool)
            .await?;
        rows.into_iter().map(Self::row_to_token).collect()
    }

    /// Revoke a token (tenant-safe: scoped by org_id). Returns whether a row
    /// was affected.
    pub async fn revoke(&self, org_id: Uuid, id: Uuid) -> Result<bool, UserError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;
        let res = sqlx::query("UPDATE scim_tokens SET revoked = 1 WHERE id = $1 AND org_id = $2")
            .bind(id.to_string())
            .bind(org_id.to_string())
            .execute(pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }
}

/// Generate a SCIM bearer token (`scim_` + 32 random bytes hex).
fn generate_scim_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    format!("scim_{}", hex::encode(bytes))
}
