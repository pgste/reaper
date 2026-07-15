//! Audit logging module for compliance and debugging
//!
//! Provides structured audit logging for all significant actions in the system.
//! Audit logs are stored in the database and can be queried for compliance reporting.

// sqlx rows decode into wide tuples by design; aliases would just move the noise.
#![allow(clippy::type_complexity)]

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::db::Database;

/// Audit logging errors
#[derive(Debug, Error)]
pub enum AuditError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// Actor type for audit logs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorType {
    User,
    ApiKey,
    Agent,
    System,
}

impl std::fmt::Display for ActorType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActorType::User => write!(f, "user"),
            ActorType::ApiKey => write!(f, "api_key"),
            ActorType::Agent => write!(f, "agent"),
            ActorType::System => write!(f, "system"),
        }
    }
}

impl std::str::FromStr for ActorType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "user" => Ok(ActorType::User),
            "api_key" => Ok(ActorType::ApiKey),
            "agent" => Ok(ActorType::Agent),
            "system" => Ok(ActorType::System),
            _ => Err(format!("Invalid actor type: {}", s)),
        }
    }
}

/// Resource type for audit logs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceType {
    Org,
    User,
    Agent,
    Policy,
    Bundle,
    Source,
    ApiKey,
    Namespace,
    Team,
    Rollout,
    Webhook,
    JwksConfig,
    Certificate,
    LegalHold,
    Environment,
    ChangeRequest,
    Connector,
}

impl std::fmt::Display for ResourceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResourceType::Org => write!(f, "org"),
            ResourceType::User => write!(f, "user"),
            ResourceType::Agent => write!(f, "agent"),
            ResourceType::Policy => write!(f, "policy"),
            ResourceType::Bundle => write!(f, "bundle"),
            ResourceType::Source => write!(f, "source"),
            ResourceType::ApiKey => write!(f, "api_key"),
            ResourceType::Namespace => write!(f, "namespace"),
            ResourceType::Team => write!(f, "team"),
            ResourceType::Rollout => write!(f, "rollout"),
            ResourceType::Webhook => write!(f, "webhook"),
            ResourceType::JwksConfig => write!(f, "jwks_config"),
            ResourceType::Certificate => write!(f, "certificate"),
            ResourceType::LegalHold => write!(f, "legal_hold"),
            ResourceType::Environment => write!(f, "environment"),
            ResourceType::ChangeRequest => write!(f, "change_request"),
            ResourceType::Connector => write!(f, "connector"),
        }
    }
}

impl std::str::FromStr for ResourceType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "org" => Ok(ResourceType::Org),
            "user" => Ok(ResourceType::User),
            "agent" => Ok(ResourceType::Agent),
            "policy" => Ok(ResourceType::Policy),
            "bundle" => Ok(ResourceType::Bundle),
            "source" => Ok(ResourceType::Source),
            "api_key" => Ok(ResourceType::ApiKey),
            "namespace" => Ok(ResourceType::Namespace),
            "team" => Ok(ResourceType::Team),
            "rollout" => Ok(ResourceType::Rollout),
            "webhook" => Ok(ResourceType::Webhook),
            "jwks_config" => Ok(ResourceType::JwksConfig),
            "certificate" => Ok(ResourceType::Certificate),
            "legal_hold" => Ok(ResourceType::LegalHold),
            "environment" => Ok(ResourceType::Environment),
            "change_request" => Ok(ResourceType::ChangeRequest),
            "connector" => Ok(ResourceType::Connector),
            _ => Err(format!("Invalid resource type: {}", s)),
        }
    }
}

/// Common audit actions
pub mod actions {
    // User actions
    pub const USER_SIGNUP: &str = "user.signup";
    pub const USER_LOGIN: &str = "user.login";
    pub const USER_LOGOUT: &str = "user.logout";
    pub const USER_PASSWORD_RESET_REQUEST: &str = "user.password_reset_request";
    pub const USER_PASSWORD_RESET: &str = "user.password_reset";
    pub const USER_EMAIL_VERIFY: &str = "user.email_verify";
    pub const USER_UPDATE: &str = "user.update";
    pub const USER_SUSPEND: &str = "user.suspend";
    pub const USER_ACTIVATE: &str = "user.activate";

    // Org actions
    pub const ORG_CREATE: &str = "org.create";
    pub const ORG_UPDATE: &str = "org.update";
    pub const ORG_DELETE: &str = "org.delete";
    pub const ORG_MEMBER_ADD: &str = "org.member_add";
    pub const ORG_MEMBER_REMOVE: &str = "org.member_remove";
    pub const ORG_MEMBER_ROLE_CHANGE: &str = "org.member_role_change";

    // Agent actions
    pub const AGENT_REGISTER: &str = "agent.register";
    pub const AGENT_HEARTBEAT: &str = "agent.heartbeat";
    pub const AGENT_DELETE: &str = "agent.delete";
    pub const AGENT_UPDATE: &str = "agent.update";

    // Source actions
    pub const SOURCE_CREATE: &str = "source.create";
    pub const SOURCE_UPDATE: &str = "source.update";
    pub const SOURCE_DELETE: &str = "source.delete";
    pub const SOURCE_SYNC: &str = "source.sync";
    pub const SOURCE_SYNC_ERROR: &str = "source.sync_error";

    // Bundle actions
    pub const BUNDLE_COMPILE: &str = "bundle.compile";
    pub const BUNDLE_STAGE: &str = "bundle.stage";
    pub const BUNDLE_PROMOTE: &str = "bundle.promote";
    pub const BUNDLE_DELETE: &str = "bundle.delete";

    // Rollout actions
    pub const ROLLOUT_START: &str = "rollout.start";
    pub const ROLLOUT_APPROVE_WAVE: &str = "rollout.approve_wave";
    pub const ROLLOUT_CANCEL: &str = "rollout.cancel";
    pub const ROLLOUT_COMPLETE: &str = "rollout.complete";
    pub const ROLLOUT_ROLLBACK: &str = "rollout.rollback";
    pub const ROLLOUT_BREAK_GLASS: &str = "rollout.break_glass";

    // Rollout supervisor (B2 / PROD R2-1): autonomous auto-rollback.
    // `..._TRIGGERED` = the trigger fired in monitor mode (no action taken);
    // `DEPLOYMENT_AUTO_ROLLBACK` = enforce mode acted (cancel + rollback).
    pub const DEPLOYMENT_AUTO_ROLLBACK_TRIGGERED: &str = "deployment.auto_rollback_triggered";
    pub const DEPLOYMENT_AUTO_ROLLBACK: &str = "deployment.auto_rollback";

    // API Key actions
    pub const APIKEY_CREATE: &str = "apikey.create";
    pub const APIKEY_REVOKE: &str = "apikey.revoke";
    pub const APIKEY_DELETE: &str = "apikey.delete";

    // Namespace actions
    pub const NAMESPACE_CREATE: &str = "namespace.create";
    pub const NAMESPACE_UPDATE: &str = "namespace.update";
    pub const NAMESPACE_DELETE: &str = "namespace.delete";

    // Environments & promotion (Plan 10)
    pub const ENV_PROMOTE: &str = "env.promote";
    pub const CHANGE_REQUEST_CREATE: &str = "change_request.create";
    pub const CHANGE_REQUEST_APPROVE: &str = "change_request.approve";
    pub const CHANGE_REQUEST_REJECT: &str = "change_request.reject";

    // Team actions
    pub const TEAM_CREATE: &str = "team.create";
    pub const TEAM_UPDATE: &str = "team.update";
    pub const TEAM_DELETE: &str = "team.delete";
    pub const TEAM_MEMBER_ADD: &str = "team.member_add";
    pub const TEAM_MEMBER_REMOVE: &str = "team.member_remove";

    // Webhook actions
    pub const WEBHOOK_CREATE: &str = "webhook.create";
    pub const WEBHOOK_UPDATE: &str = "webhook.update";
    pub const WEBHOOK_DELETE: &str = "webhook.delete";
    pub const WEBHOOK_TRIGGER: &str = "webhook.trigger";

    // Security actions
    pub const JWKS_CONFIG_CREATE: &str = "jwks_config.create";
    pub const JWKS_CONFIG_UPDATE: &str = "jwks_config.update";
    pub const JWKS_CONFIG_DELETE: &str = "jwks_config.delete";
    pub const CERTIFICATE_CREATE: &str = "certificate.create";
    pub const CERTIFICATE_REVOKE: &str = "certificate.revoke";

    // OAuth actions
    pub const OAUTH_CONNECT: &str = "oauth.connect";
    pub const OAUTH_DISCONNECT: &str = "oauth.disconnect";
    pub const OAUTH_REFRESH: &str = "oauth.refresh";

    // SSO / enterprise identity actions
    pub const SSO_LOGIN: &str = "sso.login";
    pub const SSO_CONFIG_UPDATE: &str = "sso.config_update";

    // SCIM provisioning actions
    pub const SCIM_USER_PROVISION: &str = "scim.user_provision";
    pub const SCIM_USER_UPDATE: &str = "scim.user_update";
    pub const SCIM_USER_DEPROVISION: &str = "scim.user_deprovision";
    pub const SCIM_GROUP_SYNC: &str = "scim.group_sync";
    pub const SCIM_TOKEN_CREATE: &str = "scim.token_create";
    pub const SCIM_TOKEN_REVOKE: &str = "scim.token_revoke";

    // Audit governance (Plan 04 step 6): retention windows + legal holds
    pub const AUDIT_RETENTION_UPDATE: &str = "audit.retention_update";
    pub const AUDIT_LEGAL_HOLD_CREATE: &str = "audit.legal_hold_create";
    pub const AUDIT_LEGAL_HOLD_RELEASE: &str = "audit.legal_hold_release";
    pub const AUDIT_PURGE: &str = "audit.purge";
    /// GDPR Art. 17 subject erasure (E2): the durable proof-of-erasure record.
    pub const AUDIT_SUBJECT_ERASURE: &str = "audit.subject_erasure";
    pub const AUDIT_REPLAY: &str = "audit.replay";
    // SIEM export connectors (E1): a connector is a standing exfiltration path,
    // so its lifecycle and every push are audited.
    pub const AUDIT_CONNECTOR_CREATE: &str = "audit.connector_create";
    pub const AUDIT_CONNECTOR_UPDATE: &str = "audit.connector_update";
    pub const AUDIT_CONNECTOR_DELETE: &str = "audit.connector_delete";
    pub const AUDIT_CONNECTOR_EXPORT: &str = "audit.connector_export";
}

/// Audit log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: Uuid,
    pub org_id: Option<Uuid>,
    pub actor_type: ActorType,
    pub actor_id: String,
    pub action: String,
    pub resource_type: Option<ResourceType>,
    pub resource_id: Option<String>,
    pub details: Option<serde_json::Value>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl AuditEntry {
    /// Create a new audit entry builder
    pub fn builder(
        action: &str,
        actor_type: ActorType,
        actor_id: impl Into<String>,
    ) -> AuditEntryBuilder {
        AuditEntryBuilder {
            org_id: None,
            actor_type,
            actor_id: actor_id.into(),
            action: action.to_string(),
            resource_type: None,
            resource_id: None,
            details: None,
            ip_address: None,
            user_agent: None,
        }
    }
}

/// Builder for audit entries
pub struct AuditEntryBuilder {
    org_id: Option<Uuid>,
    actor_type: ActorType,
    actor_id: String,
    action: String,
    resource_type: Option<ResourceType>,
    resource_id: Option<String>,
    details: Option<serde_json::Value>,
    ip_address: Option<String>,
    user_agent: Option<String>,
}

impl AuditEntryBuilder {
    pub fn org_id(mut self, org_id: Uuid) -> Self {
        self.org_id = Some(org_id);
        self
    }

    pub fn resource(mut self, resource_type: ResourceType, resource_id: impl Into<String>) -> Self {
        self.resource_type = Some(resource_type);
        self.resource_id = Some(resource_id.into());
        self
    }

    pub fn details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }

    pub fn ip_address(mut self, ip: impl Into<String>) -> Self {
        self.ip_address = Some(ip.into());
        self
    }

    pub fn user_agent(mut self, ua: impl Into<String>) -> Self {
        self.user_agent = Some(ua.into());
        self
    }

    pub fn build(self) -> AuditEntry {
        AuditEntry {
            id: Uuid::new_v4(),
            org_id: self.org_id,
            actor_type: self.actor_type,
            actor_id: self.actor_id,
            action: self.action,
            resource_type: self.resource_type,
            resource_id: self.resource_id,
            details: self.details,
            ip_address: self.ip_address,
            user_agent: self.user_agent,
            created_at: Utc::now(),
        }
    }

    /// Build and log the entry to the database
    pub async fn log(self, db: &Database) -> Result<AuditEntry, AuditError> {
        let entry = self.build();
        AuditRepository::new(db).create(&entry).await?;
        Ok(entry)
    }
}

/// Query parameters for listing audit logs
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AuditQuery {
    pub org_id: Option<Uuid>,
    pub actor_type: Option<ActorType>,
    pub actor_id: Option<String>,
    pub action: Option<String>,
    pub action_prefix: Option<String>,
    pub resource_type: Option<ResourceType>,
    pub resource_id: Option<String>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

/// Audit log repository
pub struct AuditRepository<'a> {
    db: &'a Database,
}

impl<'a> AuditRepository<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// Create a new audit entry
    pub async fn create(&self, entry: &AuditEntry) -> Result<(), AuditError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;

        let details_json = entry
            .details
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;

        sqlx::query(
            r#"
            INSERT INTO audit_log (
                id, org_id, actor_type, actor_id, action,
                resource_type, resource_id, details, ip_address, user_agent, created_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            "#,
        )
        .bind(entry.id.to_string())
        .bind(entry.org_id.map(|id| id.to_string()))
        .bind(entry.actor_type.to_string())
        .bind(&entry.actor_id)
        .bind(&entry.action)
        .bind(entry.resource_type.map(|t| t.to_string()))
        .bind(&entry.resource_id)
        .bind(details_json)
        .bind(&entry.ip_address)
        .bind(&entry.user_agent)
        .bind(entry.created_at.to_rfc3339())
        .execute(pool)
        .await?;

        tracing::debug!(
            action = %entry.action,
            actor_type = %entry.actor_type,
            actor_id = %entry.actor_id,
            org_id = ?entry.org_id,
            "Audit log entry created"
        );

        Ok(())
    }

    /// Query audit logs with filters
    pub async fn query(&self, params: &AuditQuery) -> Result<Vec<AuditEntry>, AuditError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;

        // Build dynamic query
        let mut query = String::from(
            r#"
            SELECT id, org_id, actor_type, actor_id, action,
                   resource_type, resource_id, details, ip_address, user_agent, created_at
            FROM audit_log
            WHERE 1=1
            "#,
        );

        let mut bindings: Vec<String> = Vec::new();

        if let Some(org_id) = &params.org_id {
            query.push_str(" AND org_id = ?");
            bindings.push(org_id.to_string());
        }

        if let Some(actor_type) = &params.actor_type {
            query.push_str(" AND actor_type = ?");
            bindings.push(actor_type.to_string());
        }

        if let Some(actor_id) = &params.actor_id {
            query.push_str(" AND actor_id = ?");
            bindings.push(actor_id.clone());
        }

        if let Some(action) = &params.action {
            query.push_str(" AND action = ?");
            bindings.push(action.clone());
        }

        if let Some(prefix) = &params.action_prefix {
            query.push_str(" AND action LIKE ?");
            bindings.push(format!("{}%", prefix));
        }

        if let Some(resource_type) = &params.resource_type {
            query.push_str(" AND resource_type = ?");
            bindings.push(resource_type.to_string());
        }

        if let Some(resource_id) = &params.resource_id {
            query.push_str(" AND resource_id = ?");
            bindings.push(resource_id.clone());
        }

        if let Some(from) = &params.from {
            query.push_str(" AND created_at >= ?");
            bindings.push(from.to_rfc3339());
        }

        if let Some(to) = &params.to {
            query.push_str(" AND created_at <= ?");
            bindings.push(to.to_rfc3339());
        }

        query.push_str(" ORDER BY created_at DESC");

        if let Some(limit) = params.limit {
            query.push_str(&format!(" LIMIT {}", limit));
        } else {
            query.push_str(" LIMIT 100"); // Default limit
        }

        if let Some(offset) = params.offset {
            query.push_str(&format!(" OFFSET {}", offset));
        }

        let query = crate::db::numbered_placeholders(&query);

        // Execute with bindings
        let mut q = sqlx::query_as::<
            _,
            (
                String,
                Option<String>,
                String,
                String,
                String,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                String,
            ),
        >(&query);

        for binding in &bindings {
            q = q.bind(binding);
        }

        let rows = q.fetch_all(pool).await?;

        let entries: Result<Vec<AuditEntry>, _> =
            rows.into_iter().map(|row| self.row_to_entry(row)).collect();

        entries
    }

    /// Get audit entries for a specific resource
    pub async fn for_resource(
        &self,
        resource_type: ResourceType,
        resource_id: &str,
        limit: Option<u32>,
    ) -> Result<Vec<AuditEntry>, AuditError> {
        self.query(&AuditQuery {
            resource_type: Some(resource_type),
            resource_id: Some(resource_id.to_string()),
            limit,
            ..Default::default()
        })
        .await
    }

    /// Get recent audit entries for an org
    pub async fn for_org(
        &self,
        org_id: Uuid,
        limit: Option<u32>,
    ) -> Result<Vec<AuditEntry>, AuditError> {
        self.query(&AuditQuery {
            org_id: Some(org_id),
            limit,
            ..Default::default()
        })
        .await
    }

    /// Count entries matching a query
    pub async fn count(&self, params: &AuditQuery) -> Result<u64, AuditError> {
        let pool = self.db.any_pool().ok_or(sqlx::Error::PoolClosed)?;

        let mut query = String::from("SELECT COUNT(*) FROM audit_log WHERE 1=1");
        let mut bindings: Vec<String> = Vec::new();

        if let Some(org_id) = &params.org_id {
            query.push_str(" AND org_id = ?");
            bindings.push(org_id.to_string());
        }

        if let Some(action) = &params.action {
            query.push_str(" AND action = ?");
            bindings.push(action.clone());
        }

        if let Some(prefix) = &params.action_prefix {
            query.push_str(" AND action LIKE ?");
            bindings.push(format!("{}%", prefix));
        }

        if let Some(from) = &params.from {
            query.push_str(" AND created_at >= ?");
            bindings.push(from.to_rfc3339());
        }

        if let Some(to) = &params.to {
            query.push_str(" AND created_at <= ?");
            bindings.push(to.to_rfc3339());
        }

        let query = crate::db::numbered_placeholders(&query);
        let mut q = sqlx::query_as::<_, (i64,)>(&query);
        for binding in &bindings {
            q = q.bind(binding);
        }

        let (count,) = q.fetch_one(pool).await?;
        Ok(count as u64)
    }

    fn row_to_entry(
        &self,
        row: (
            String,
            Option<String>,
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            String,
        ),
    ) -> Result<AuditEntry, AuditError> {
        Ok(AuditEntry {
            id: Uuid::parse_str(&row.0).map_err(|e| sqlx::Error::Decode(e.into()))?,
            org_id: row.1.and_then(|s| Uuid::parse_str(&s).ok()),
            actor_type: row
                .2
                .parse()
                .map_err(|e: String| sqlx::Error::Decode(e.into()))?,
            actor_id: row.3,
            action: row.4,
            resource_type: row.5.and_then(|s| s.parse().ok()),
            resource_id: row.6,
            details: row.7.and_then(|s| serde_json::from_str(&s).ok()),
            ip_address: row.8,
            user_agent: row.9,
            created_at: chrono::DateTime::parse_from_rfc3339(&row.10)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|e| sqlx::Error::Decode(e.into()))?,
        })
    }
}

/// Helper to extract client info from request headers
pub struct ClientInfo {
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
}

impl ClientInfo {
    /// Extract from axum headers
    pub fn from_headers(headers: &axum::http::HeaderMap) -> Self {
        let ip_address = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.split(',').next().unwrap_or(s).trim().to_string())
            .or_else(|| {
                headers
                    .get("x-real-ip")
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_string())
            });

        let user_agent = headers
            .get("user-agent")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        Self {
            ip_address,
            user_agent,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_actor_type_parsing() {
        assert_eq!("user".parse::<ActorType>().unwrap(), ActorType::User);
        assert_eq!("api_key".parse::<ActorType>().unwrap(), ActorType::ApiKey);
        assert_eq!("agent".parse::<ActorType>().unwrap(), ActorType::Agent);
        assert_eq!("system".parse::<ActorType>().unwrap(), ActorType::System);
    }

    #[test]
    fn test_resource_type_parsing() {
        assert_eq!("org".parse::<ResourceType>().unwrap(), ResourceType::Org);
        assert_eq!(
            "bundle".parse::<ResourceType>().unwrap(),
            ResourceType::Bundle
        );
        assert_eq!(
            "agent".parse::<ResourceType>().unwrap(),
            ResourceType::Agent
        );
    }

    #[test]
    fn test_audit_entry_builder() {
        let entry = AuditEntry::builder(actions::USER_LOGIN, ActorType::User, "user-123")
            .org_id(Uuid::new_v4())
            .resource(ResourceType::User, "user-123")
            .ip_address("192.168.1.1")
            .user_agent("Mozilla/5.0")
            .details(serde_json::json!({"success": true}))
            .build();

        assert_eq!(entry.action, actions::USER_LOGIN);
        assert_eq!(entry.actor_type, ActorType::User);
        assert_eq!(entry.actor_id, "user-123");
        assert!(entry.org_id.is_some());
        assert_eq!(entry.resource_type, Some(ResourceType::User));
    }
}
