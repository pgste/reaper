//! SIEM export connector persistence (round-2 E1, slice 3).
//!
//! Per-org outbound push targets for decision-log export (Splunk HEC / generic
//! HTTP), shaped as NDJSON / OCSF / CEF. A connector is a standing exfiltration
//! path, so it is `audit:export`-gated and audited; only the *config* lives here,
//! delivery reads history from the ClickHouse decision store.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::{Database, DatabaseError};
use policy_engine::ExportFormat;

/// Transport for a SIEM connector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorType {
    /// Splunk HTTP Event Collector — `Authorization: Splunk <secret>` token auth,
    /// each record wrapped as `{"event": …}` at the HEC event endpoint.
    SplunkHec,
    /// Generic HTTP endpoint — NDJSON body, optional HMAC-SHA-256 request
    /// signature (`X-Reaper-Signature`) when a secret is set.
    Http,
}

impl ConnectorType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SplunkHec => "splunk_hec",
            Self::Http => "http",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "splunk_hec" => Some(Self::SplunkHec),
            "http" => Some(Self::Http),
            _ => None,
        }
    }
}

/// A configured SIEM export connector.
#[derive(Debug, Clone, Serialize)]
pub struct SiemConnector {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    pub connector_type: ConnectorType,
    pub endpoint: String,
    /// HMAC secret (http) or HEC token (splunk_hec). Never serialized out to
    /// clients — the API returns a summary without it.
    #[serde(skip_serializing)]
    pub secret: Option<String>,
    pub format: ExportFormat,
    pub enabled: bool,
    pub failure_count: i32,
    pub last_export_at: Option<DateTime<Utc>>,
    pub created_by: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Fields for creating a connector.
#[derive(Debug, Clone)]
pub struct NewConnector<'a> {
    pub name: &'a str,
    pub connector_type: ConnectorType,
    pub endpoint: &'a str,
    pub secret: Option<&'a str>,
    pub format: ExportFormat,
    pub created_by: Option<&'a str>,
}

/// Fields for updating a connector (all optional). `secret: Some(None)` is not
/// expressible here; a secret is set at create and replaced via `Some(Some(_))`.
#[derive(Debug, Clone, Default)]
pub struct ConnectorPatch<'a> {
    pub name: Option<&'a str>,
    pub endpoint: Option<&'a str>,
    pub secret: Option<&'a str>,
    pub format: Option<ExportFormat>,
    pub enabled: Option<bool>,
}

type ConnectorRow = (
    String,         // id
    String,         // org_id
    String,         // name
    String,         // connector_type
    String,         // endpoint
    Option<String>, // secret
    String,         // format
    i64,            // enabled
    i64,            // failure_count
    Option<String>, // last_export_at
    Option<String>, // created_by
    String,         // created_at
    String,         // updated_at
);

const COLS: &str = "id, org_id, name, connector_type, endpoint, secret, format, enabled, \
     failure_count, last_export_at, created_by, created_at, updated_at";

pub struct AuditConnectorRepository<'a> {
    db: &'a Database,
}

impl<'a> AuditConnectorRepository<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    fn parse_ts(s: &str) -> Result<DateTime<Utc>, DatabaseError> {
        DateTime::parse_from_rfc3339(s)
            .map(|d| d.with_timezone(&Utc))
            .map_err(|e| DatabaseError::Config(format!("bad timestamp in siem_connectors: {e}")))
    }

    fn row_to_connector(r: ConnectorRow) -> Result<SiemConnector, DatabaseError> {
        let parse_uuid = |s: &str| {
            Uuid::parse_str(s)
                .map_err(|e| DatabaseError::Config(format!("bad uuid in siem_connectors: {e}")))
        };
        Ok(SiemConnector {
            id: parse_uuid(&r.0)?,
            org_id: parse_uuid(&r.1)?,
            name: r.2,
            connector_type: ConnectorType::parse(&r.3)
                .ok_or_else(|| DatabaseError::Config(format!("unknown connector_type: {}", r.3)))?,
            endpoint: r.4,
            secret: r.5,
            format: ExportFormat::parse(&r.6)
                .ok_or_else(|| DatabaseError::Config(format!("unknown export format: {}", r.6)))?,
            enabled: r.7 != 0,
            failure_count: r.8 as i32,
            last_export_at: r.9.as_deref().map(Self::parse_ts).transpose()?,
            created_by: r.10,
            created_at: Self::parse_ts(&r.11)?,
            updated_at: Self::parse_ts(&r.12)?,
        })
    }

    /// Create a connector for `org_id`.
    pub async fn create(
        &self,
        org_id: Uuid,
        input: NewConnector<'_>,
    ) -> Result<SiemConnector, DatabaseError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;
        let id = Uuid::new_v4();
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO siem_connectors \
             (id, org_id, name, connector_type, endpoint, secret, format, enabled, \
              failure_count, created_by, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, 1, 0, $8, $9, $10)",
        )
        .bind(id.to_string())
        .bind(org_id.to_string())
        .bind(input.name)
        .bind(input.connector_type.as_str())
        .bind(input.endpoint)
        .bind(input.secret)
        .bind(input.format.as_str())
        .bind(input.created_by)
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .execute(pool)
        .await?;
        Ok(SiemConnector {
            id,
            org_id,
            name: input.name.to_string(),
            connector_type: input.connector_type,
            endpoint: input.endpoint.to_string(),
            secret: input.secret.map(str::to_string),
            format: input.format,
            enabled: true,
            failure_count: 0,
            last_export_at: None,
            created_by: input.created_by.map(str::to_string),
            created_at: now,
            updated_at: now,
        })
    }

    /// List an org's connectors, newest first.
    pub async fn list_for_org(&self, org_id: Uuid) -> Result<Vec<SiemConnector>, DatabaseError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;
        let rows: Vec<ConnectorRow> = sqlx::query_as(&format!(
            "SELECT {COLS} FROM siem_connectors WHERE org_id = $1 ORDER BY created_at DESC, id DESC"
        ))
        .bind(org_id.to_string())
        .fetch_all(pool)
        .await?;
        rows.into_iter().map(Self::row_to_connector).collect()
    }

    /// Fetch one connector, tenant-scoped.
    pub async fn get(
        &self,
        org_id: Uuid,
        id: Uuid,
    ) -> Result<Option<SiemConnector>, DatabaseError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;
        let row: Option<ConnectorRow> = sqlx::query_as(&format!(
            "SELECT {COLS} FROM siem_connectors WHERE id = $1 AND org_id = $2"
        ))
        .bind(id.to_string())
        .bind(org_id.to_string())
        .fetch_optional(pool)
        .await?;
        row.map(Self::row_to_connector).transpose()
    }

    /// Fetch one connector by name, tenant-scoped (for duplicate-name checks).
    pub async fn get_by_name(
        &self,
        org_id: Uuid,
        name: &str,
    ) -> Result<Option<SiemConnector>, DatabaseError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;
        let row: Option<ConnectorRow> = sqlx::query_as(&format!(
            "SELECT {COLS} FROM siem_connectors WHERE org_id = $1 AND name = $2"
        ))
        .bind(org_id.to_string())
        .bind(name)
        .fetch_optional(pool)
        .await?;
        row.map(Self::row_to_connector).transpose()
    }

    /// Patch a connector (tenant-scoped). Returns the updated row, or `None` if
    /// it does not exist for this org.
    pub async fn update(
        &self,
        org_id: Uuid,
        id: Uuid,
        patch: ConnectorPatch<'_>,
    ) -> Result<Option<SiemConnector>, DatabaseError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;
        let now = Utc::now();

        // Build the SET list with `?` placeholders, renumbered for Postgres.
        let mut sets = vec!["updated_at = ?"];
        if patch.name.is_some() {
            sets.push("name = ?");
        }
        if patch.endpoint.is_some() {
            sets.push("endpoint = ?");
        }
        if patch.secret.is_some() {
            sets.push("secret = ?");
        }
        if patch.format.is_some() {
            sets.push("format = ?");
        }
        if patch.enabled.is_some() {
            sets.push("enabled = ?");
        }
        let sql = crate::db::numbered_placeholders(&format!(
            "UPDATE siem_connectors SET {} WHERE id = ? AND org_id = ?",
            sets.join(", ")
        ));

        let mut q = sqlx::query(&sql).bind(now.to_rfc3339());
        if let Some(v) = patch.name {
            q = q.bind(v.to_string());
        }
        if let Some(v) = patch.endpoint {
            q = q.bind(v.to_string());
        }
        if let Some(v) = patch.secret {
            q = q.bind(v.to_string());
        }
        if let Some(v) = patch.format {
            q = q.bind(v.as_str().to_string());
        }
        if let Some(v) = patch.enabled {
            q = q.bind(i64::from(v));
        }
        q = q.bind(id.to_string()).bind(org_id.to_string());
        let result = q.execute(pool).await?;
        if result.rows_affected() == 0 {
            return Ok(None);
        }
        self.get(org_id, id).await
    }

    /// Record the outcome of an export attempt: success resets the failure
    /// counter and stamps `last_export_at`; failure increments it.
    pub async fn record_export(&self, id: Uuid, success: bool) -> Result<(), DatabaseError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;
        let now = Utc::now().to_rfc3339();
        let sql = if success {
            "UPDATE siem_connectors SET last_export_at = $1, failure_count = 0, updated_at = $1 \
             WHERE id = $2"
        } else {
            "UPDATE siem_connectors SET failure_count = failure_count + 1, updated_at = $1 \
             WHERE id = $2"
        };
        sqlx::query(sql)
            .bind(now)
            .bind(id.to_string())
            .execute(pool)
            .await?;
        Ok(())
    }

    /// Delete a connector (tenant-scoped). Returns false if it did not exist.
    pub async fn delete(&self, org_id: Uuid, id: Uuid) -> Result<bool, DatabaseError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;
        let result = sqlx::query("DELETE FROM siem_connectors WHERE id = $1 AND org_id = $2")
            .bind(id.to_string())
            .bind(org_id.to_string())
            .execute(pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}
