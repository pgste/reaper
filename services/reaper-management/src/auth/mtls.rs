//! mTLS client certificate validation
//!
//! Provides validation and management of client certificates for mutual TLS
//! authentication between agents and the management server.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use thiserror::Error;
use uuid::Uuid;

/// Client certificate record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientCertificate {
    pub id: Uuid,
    pub org_id: Uuid,
    /// Agent this certificate is bound to (optional)
    pub agent_id: Option<Uuid>,
    /// SHA-256 fingerprint of the certificate
    pub fingerprint: String,
    /// Subject DN (Distinguished Name)
    pub subject: Option<String>,
    /// Issuer DN
    pub issuer: Option<String>,
    /// Certificate validity start
    pub not_before: Option<DateTime<Utc>>,
    /// Certificate validity end
    pub not_after: Option<DateTime<Utc>>,
    /// Whether the certificate has been revoked
    pub is_revoked: bool,
    /// When the certificate was revoked
    pub revoked_at: Option<DateTime<Utc>>,
    /// Reason for revocation
    pub revocation_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ClientCertificate {
    /// Check if the certificate is currently valid
    pub fn is_valid(&self) -> bool {
        if self.is_revoked {
            return false;
        }

        let now = Utc::now();

        // Check not_before
        if let Some(not_before) = self.not_before {
            if now < not_before {
                return false;
            }
        }

        // Check not_after
        if let Some(not_after) = self.not_after {
            if now > not_after {
                return false;
            }
        }

        true
    }

    /// Check if the certificate is expired
    pub fn is_expired(&self) -> bool {
        if let Some(not_after) = self.not_after {
            Utc::now() > not_after
        } else {
            false
        }
    }

    /// Check if the certificate is not yet valid
    pub fn is_not_yet_valid(&self) -> bool {
        if let Some(not_before) = self.not_before {
            Utc::now() < not_before
        } else {
            false
        }
    }
}

/// Input for registering a new client certificate
#[derive(Debug, Deserialize)]
pub struct RegisterCertificate {
    /// SHA-256 fingerprint of the certificate
    pub fingerprint: String,
    /// Subject DN
    pub subject: Option<String>,
    /// Issuer DN
    pub issuer: Option<String>,
    /// Certificate validity start
    pub not_before: Option<DateTime<Utc>>,
    /// Certificate validity end
    pub not_after: Option<DateTime<Utc>>,
    /// Agent to bind this certificate to
    pub agent_id: Option<Uuid>,
}

/// mTLS validation errors
#[derive(Debug, Error)]
pub enum MtlsError {
    #[error("Certificate not found")]
    NotFound,

    #[error("Certificate is revoked")]
    Revoked,

    #[error("Certificate is expired")]
    Expired,

    #[error("Certificate is not yet valid")]
    NotYetValid,

    #[error("Certificate fingerprint mismatch")]
    FingerprintMismatch,

    #[error("Agent binding mismatch")]
    AgentMismatch,

    #[error("Database error: {0}")]
    Database(String),
}

/// Repository for client certificate operations
pub struct ClientCertificateRepository<'a> {
    db: &'a crate::db::Database,
}

impl<'a> ClientCertificateRepository<'a> {
    pub fn new(db: &'a crate::db::Database) -> Self {
        Self { db }
    }

    /// Register a new client certificate
    pub async fn create(
        &self,
        org_id: Uuid,
        input: RegisterCertificate,
    ) -> Result<ClientCertificate, crate::db::DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| crate::db::DatabaseError::Config("No database pool".to_string()))?;

        let id = Uuid::new_v4();
        let now = Utc::now();

        let sql = r#"
            INSERT INTO client_certificates (
                id, org_id, agent_id, fingerprint, subject, issuer,
                not_before, not_after, is_revoked, created_at, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 0, $9, $10)
        "#;

        sqlx::query(sql)
            .bind(id.to_string())
            .bind(org_id.to_string())
            .bind(input.agent_id.map(|id| id.to_string()))
            .bind(&input.fingerprint)
            .bind(&input.subject)
            .bind(&input.issuer)
            .bind(input.not_before.map(|dt| dt.to_rfc3339()))
            .bind(input.not_after.map(|dt| dt.to_rfc3339()))
            .bind(now.to_rfc3339())
            .bind(now.to_rfc3339())
            .execute(pool)
            .await?;

        self.get_by_id(id)
            .await?
            .ok_or_else(|| crate::db::DatabaseError::NotFound("Certificate not found".to_string()))
    }

    /// Get a certificate by ID
    pub async fn get_by_id(
        &self,
        id: Uuid,
    ) -> Result<Option<ClientCertificate>, crate::db::DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| crate::db::DatabaseError::Config("No database pool".to_string()))?;

        let sql = r#"
            SELECT id, org_id, agent_id, fingerprint, subject, issuer,
                   not_before, not_after, is_revoked, revoked_at, revocation_reason,
                   created_at, updated_at
            FROM client_certificates
            WHERE id = $1
        "#;

        let row = sqlx::query(sql)
            .bind(id.to_string())
            .fetch_optional(pool)
            .await?;

        row.map(|r| self.row_to_certificate(&r)).transpose()
    }

    /// Get a certificate by fingerprint
    pub async fn get_by_fingerprint(
        &self,
        fingerprint: &str,
    ) -> Result<Option<ClientCertificate>, crate::db::DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| crate::db::DatabaseError::Config("No database pool".to_string()))?;

        let sql = r#"
            SELECT id, org_id, agent_id, fingerprint, subject, issuer,
                   not_before, not_after, is_revoked, revoked_at, revocation_reason,
                   created_at, updated_at
            FROM client_certificates
            WHERE fingerprint = $1
        "#;

        let row = sqlx::query(sql)
            .bind(fingerprint)
            .fetch_optional(pool)
            .await?;

        row.map(|r| self.row_to_certificate(&r)).transpose()
    }

    /// List all certificates for an organization
    pub async fn list_by_org(
        &self,
        org_id: Uuid,
    ) -> Result<Vec<ClientCertificate>, crate::db::DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| crate::db::DatabaseError::Config("No database pool".to_string()))?;

        let sql = r#"
            SELECT id, org_id, agent_id, fingerprint, subject, issuer,
                   not_before, not_after, is_revoked, revoked_at, revocation_reason,
                   created_at, updated_at
            FROM client_certificates
            WHERE org_id = $1
            ORDER BY created_at DESC
        "#;

        let rows = sqlx::query(sql)
            .bind(org_id.to_string())
            .fetch_all(pool)
            .await?;

        rows.iter().map(|r| self.row_to_certificate(r)).collect()
    }

    /// List certificates for an agent
    pub async fn list_by_agent(
        &self,
        agent_id: Uuid,
    ) -> Result<Vec<ClientCertificate>, crate::db::DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| crate::db::DatabaseError::Config("No database pool".to_string()))?;

        let sql = r#"
            SELECT id, org_id, agent_id, fingerprint, subject, issuer,
                   not_before, not_after, is_revoked, revoked_at, revocation_reason,
                   created_at, updated_at
            FROM client_certificates
            WHERE agent_id = $1
            ORDER BY created_at DESC
        "#;

        let rows = sqlx::query(sql)
            .bind(agent_id.to_string())
            .fetch_all(pool)
            .await?;

        rows.iter().map(|r| self.row_to_certificate(r)).collect()
    }

    /// Revoke a certificate
    pub async fn revoke(
        &self,
        id: Uuid,
        reason: Option<&str>,
    ) -> Result<bool, crate::db::DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| crate::db::DatabaseError::Config("No database pool".to_string()))?;

        let now = Utc::now();

        let sql = r#"
            UPDATE client_certificates
            SET is_revoked = 1,
                revoked_at = $1,
                revocation_reason = $2,
                updated_at = $3
            WHERE id = $4 AND is_revoked = 0
        "#;

        let result = sqlx::query(sql)
            .bind(now.to_rfc3339())
            .bind(reason)
            .bind(now.to_rfc3339())
            .bind(id.to_string())
            .execute(pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Bind a certificate to an agent
    pub async fn bind_to_agent(
        &self,
        cert_id: Uuid,
        agent_id: Uuid,
    ) -> Result<bool, crate::db::DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| crate::db::DatabaseError::Config("No database pool".to_string()))?;

        let now = Utc::now();

        let sql = r#"
            UPDATE client_certificates
            SET agent_id = $1,
                updated_at = $2
            WHERE id = $3
        "#;

        let result = sqlx::query(sql)
            .bind(agent_id.to_string())
            .bind(now.to_rfc3339())
            .bind(cert_id.to_string())
            .execute(pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Unbind a certificate from its agent
    pub async fn unbind_from_agent(&self, cert_id: Uuid) -> Result<bool, crate::db::DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| crate::db::DatabaseError::Config("No database pool".to_string()))?;

        let now = Utc::now();

        let sql = r#"
            UPDATE client_certificates
            SET agent_id = NULL,
                updated_at = $1
            WHERE id = $2
        "#;

        let result = sqlx::query(sql)
            .bind(now.to_rfc3339())
            .bind(cert_id.to_string())
            .execute(pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Delete a certificate
    pub async fn delete(&self, id: Uuid) -> Result<bool, crate::db::DatabaseError> {
        let pool = self
            .db
            .sqlite_pool()
            .ok_or_else(|| crate::db::DatabaseError::Config("No database pool".to_string()))?;

        let result = sqlx::query("DELETE FROM client_certificates WHERE id = $1")
            .bind(id.to_string())
            .execute(pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    fn row_to_certificate(
        &self,
        row: &sqlx::sqlite::SqliteRow,
    ) -> Result<ClientCertificate, crate::db::DatabaseError> {
        let id: String = row.get("id");
        let org_id: String = row.get("org_id");
        let agent_id: Option<String> = row.get("agent_id");
        let is_revoked: i32 = row.get("is_revoked");
        let created_at: String = row.get("created_at");
        let updated_at: String = row.get("updated_at");
        let not_before: Option<String> = row.get("not_before");
        let not_after: Option<String> = row.get("not_after");
        let revoked_at: Option<String> = row.get("revoked_at");

        Ok(ClientCertificate {
            id: id
                .parse()
                .map_err(|e| crate::db::DatabaseError::Config(format!("Invalid UUID: {}", e)))?,
            org_id: org_id
                .parse()
                .map_err(|e| crate::db::DatabaseError::Config(format!("Invalid UUID: {}", e)))?,
            agent_id: agent_id
                .map(|s| {
                    s.parse().map_err(|e| {
                        crate::db::DatabaseError::Config(format!("Invalid UUID: {}", e))
                    })
                })
                .transpose()?,
            fingerprint: row.get("fingerprint"),
            subject: row.get("subject"),
            issuer: row.get("issuer"),
            not_before: not_before.and_then(|s| {
                chrono::DateTime::parse_from_rfc3339(&s)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            }),
            not_after: not_after.and_then(|s| {
                chrono::DateTime::parse_from_rfc3339(&s)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            }),
            is_revoked: is_revoked != 0,
            revoked_at: revoked_at.and_then(|s| {
                chrono::DateTime::parse_from_rfc3339(&s)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            }),
            revocation_reason: row.get("revocation_reason"),
            created_at: chrono::DateTime::parse_from_rfc3339(&created_at)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            updated_at: chrono::DateTime::parse_from_rfc3339(&updated_at)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
        })
    }
}

/// Validate a client certificate by fingerprint
pub async fn validate_certificate(
    db: &crate::db::Database,
    fingerprint: &str,
    expected_agent_id: Option<Uuid>,
) -> Result<ClientCertificate, MtlsError> {
    let repo = ClientCertificateRepository::new(db);

    let cert = repo
        .get_by_fingerprint(fingerprint)
        .await
        .map_err(|e| MtlsError::Database(e.to_string()))?
        .ok_or(MtlsError::NotFound)?;

    if cert.is_revoked {
        return Err(MtlsError::Revoked);
    }

    if cert.is_expired() {
        return Err(MtlsError::Expired);
    }

    if cert.is_not_yet_valid() {
        return Err(MtlsError::NotYetValid);
    }

    // If an expected agent ID is provided, verify it matches
    if let Some(expected) = expected_agent_id {
        if let Some(bound_agent) = cert.agent_id {
            if bound_agent != expected {
                return Err(MtlsError::AgentMismatch);
            }
        }
    }

    Ok(cert)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_certificate_validity() {
        let cert = ClientCertificate {
            id: Uuid::new_v4(),
            org_id: Uuid::new_v4(),
            agent_id: None,
            fingerprint: "abc123".to_string(),
            subject: Some("CN=test".to_string()),
            issuer: Some("CN=ca".to_string()),
            not_before: Some(Utc::now() - chrono::Duration::hours(1)),
            not_after: Some(Utc::now() + chrono::Duration::hours(1)),
            is_revoked: false,
            revoked_at: None,
            revocation_reason: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        assert!(cert.is_valid());
        assert!(!cert.is_expired());
        assert!(!cert.is_not_yet_valid());
    }

    #[test]
    fn test_revoked_certificate() {
        let cert = ClientCertificate {
            id: Uuid::new_v4(),
            org_id: Uuid::new_v4(),
            agent_id: None,
            fingerprint: "abc123".to_string(),
            subject: None,
            issuer: None,
            not_before: None,
            not_after: None,
            is_revoked: true,
            revoked_at: Some(Utc::now()),
            revocation_reason: Some("Compromised".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        assert!(!cert.is_valid());
    }

    #[test]
    fn test_expired_certificate() {
        let cert = ClientCertificate {
            id: Uuid::new_v4(),
            org_id: Uuid::new_v4(),
            agent_id: None,
            fingerprint: "abc123".to_string(),
            subject: None,
            issuer: None,
            not_before: None,
            not_after: Some(Utc::now() - chrono::Duration::hours(1)),
            is_revoked: false,
            revoked_at: None,
            revocation_reason: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        assert!(!cert.is_valid());
        assert!(cert.is_expired());
    }
}
