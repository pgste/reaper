//! Persistence for per-org SSO configurations (`sso_configs`).

use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::{SsoConfig, SsoConfigInput, SsoProtocol};
use crate::auth::users::UserError;
use crate::db::Database;

/// Row tuple as stored (all TEXT except `enabled`), decoded uniformly across the
/// sqlite and postgres `AnyPool` backends.
type Row = (
    String,         // id
    String,         // org_id
    String,         // protocol
    i32,            // enabled
    String,         // issuer
    String,         // client_id
    Option<String>, // client_secret_encrypted
    Option<String>, // discovery_url
    Option<String>, // jwks_url
    Option<String>, // attr_map_json
    Option<String>, // allowed_domains_json
    String,         // default_role
    String,         // created_at
    String,         // updated_at
);

const COLS: &str = "id, org_id, protocol, enabled, issuer, client_id, \
     client_secret_encrypted, discovery_url, jwks_url, attr_map_json, \
     allowed_domains_json, default_role, created_at, updated_at";

pub struct SsoConfigStore<'a> {
    db: &'a Database,
}

impl<'a> SsoConfigStore<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    fn row_to_config(r: Row) -> Result<SsoConfig, UserError> {
        let parse_uuid = |s: &str| -> Result<Uuid, UserError> {
            Uuid::parse_str(s).map_err(|_| UserError::NotFound)
        };
        let parse_ts = |s: &str| -> Result<DateTime<Utc>, UserError> {
            DateTime::parse_from_rfc3339(s)
                .map(|d| d.with_timezone(&Utc))
                .map_err(|_| UserError::NotFound)
        };
        Ok(SsoConfig {
            id: parse_uuid(&r.0)?,
            org_id: parse_uuid(&r.1)?,
            protocol: SsoProtocol::parse(&r.2).ok_or(UserError::NotFound)?,
            enabled: r.3 != 0,
            issuer: r.4,
            client_id: r.5,
            client_secret_encrypted: r.6,
            discovery_url: r.7,
            jwks_url: r.8,
            attr_map_json: r.9,
            allowed_domains_json: r.10,
            default_role: r.11,
            created_at: parse_ts(&r.12)?,
            updated_at: parse_ts(&r.13)?,
        })
    }

    /// Create or replace the config for an org+protocol (unique per pair).
    pub async fn upsert(
        &self,
        org_id: Uuid,
        input: &SsoConfigInput,
    ) -> Result<SsoConfig, UserError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;
        let now = Utc::now().to_rfc3339();
        // ON CONFLICT(org_id, protocol) is valid on both sqlite (3.24+) and pg.
        // created_at is preserved on update via excluded/existing semantics: we
        // only overwrite it on insert by binding it, and keep the row id stable.
        sqlx::query(
            "INSERT INTO sso_configs \
             (id, org_id, protocol, enabled, issuer, client_id, client_secret_encrypted, \
              discovery_url, jwks_url, attr_map_json, allowed_domains_json, default_role, \
              created_at, updated_at) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$13) \
             ON CONFLICT(org_id, protocol) DO UPDATE SET \
               enabled = excluded.enabled, \
               issuer = excluded.issuer, \
               client_id = excluded.client_id, \
               client_secret_encrypted = excluded.client_secret_encrypted, \
               discovery_url = excluded.discovery_url, \
               jwks_url = excluded.jwks_url, \
               attr_map_json = excluded.attr_map_json, \
               allowed_domains_json = excluded.allowed_domains_json, \
               default_role = excluded.default_role, \
               updated_at = excluded.updated_at",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(org_id.to_string())
        .bind(input.protocol.as_str())
        .bind(input.enabled as i32)
        .bind(&input.issuer)
        .bind(&input.client_id)
        .bind(&input.client_secret_encrypted)
        .bind(&input.discovery_url)
        .bind(&input.jwks_url)
        .bind(&input.attr_map_json)
        .bind(&input.allowed_domains_json)
        .bind(&input.default_role)
        .bind(&now)
        .execute(pool)
        .await?;

        self.get(org_id, input.protocol)
            .await?
            .ok_or(UserError::NotFound)
    }

    /// Fetch the config for an org+protocol (tenant-safe: keyed by org_id).
    pub async fn get(
        &self,
        org_id: Uuid,
        protocol: SsoProtocol,
    ) -> Result<Option<SsoConfig>, UserError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;
        let sql = format!("SELECT {COLS} FROM sso_configs WHERE org_id = $1 AND protocol = $2");
        let row: Option<Row> = sqlx::query_as(&sql)
            .bind(org_id.to_string())
            .bind(protocol.as_str())
            .fetch_optional(pool)
            .await?;
        row.map(Self::row_to_config).transpose()
    }

    /// Fetch the org's OIDC config only if it is enabled — the lookup the login
    /// flow uses. `None` means "SSO not available for this org".
    pub async fn get_enabled_oidc(&self, org_id: Uuid) -> Result<Option<SsoConfig>, UserError> {
        Ok(self
            .get(org_id, SsoProtocol::Oidc)
            .await?
            .filter(|c| c.enabled))
    }
}
