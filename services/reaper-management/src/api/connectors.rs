//! SIEM export connectors API (round-2 E1, slice 3).
//!
//! Per-org CRUD for outbound decision-log push targets (Splunk HEC / generic
//! HTTP), plus a **push-export** endpoint that reads a range of decisions from
//! the central store, shapes them (NDJSON / OCSF / CEF), and delivers them to the
//! connector. All routes require the dedicated `audit:export` scope — a connector
//! is a standing exfiltration path, so it is separated from general org admin —
//! and every mutation and push is written to the audit trail.
//!
//! Shaping lives in `policy-engine` (`DecisionLogEntry::export`), transport in
//! `crate::siem::ConnectorDeliveryService`; this module orchestrates.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use policy_engine::{DecisionLogEntry, ExportFormat};

use crate::{
    api::error::{ApiError, ApiResult, ProblemDetails},
    api::orgs::resolve_org,
    audit::{actions, ActorType, AuditEntry, ResourceType},
    auth::middleware::{AuthenticatedUser, RequireAuth},
    auth::scopes::Scope,
    db::repositories::{
        AuditConnectorRepository, ConnectorPatch, ConnectorType, NewConnector,
        OrganizationRepository, SiemConnector,
    },
    decisions::{DecisionQuery, DecisionRow, DecisionStoreError},
    siem::{ConnectorDeliveryResult, ConnectorDeliveryService},
    state::AppState,
};

/// Hard cap on how many decisions one push-export delivers.
const MAX_EXPORT_LIMIT: u64 = 5000;
const DEFAULT_EXPORT_LIMIT: u64 = 1000;

/// Build connector routes.
pub fn routes() -> OpenApiRouter<Arc<AppState>> {
    OpenApiRouter::new()
        .routes(routes!(list_connectors, create_connector))
        .routes(routes!(get_connector, update_connector, delete_connector))
        .routes(routes!(test_connector))
        .routes(routes!(export_now))
}

/// Authorize a connector operation on `org` and return the org id. Requires the
/// dedicated `audit:export` scope (separation of duties — a connector is a
/// standing exfiltration path); the global `admin` scope still covers it and the
/// platform-operator cross-org escape.
async fn authorize_export(
    state: &AppState,
    user: &AuthenticatedUser,
    org_ref: &str,
) -> ApiResult<Uuid> {
    if !user.has_permission(Scope::AuditExport) && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Managing SIEM connectors requires the audit:export scope".to_string(),
        ));
    }
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, org_ref).await?;
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot manage connectors for other organizations".to_string(),
        ));
    }
    Ok(organization.id)
}

fn actor_type_of(user: &AuthenticatedUser) -> ActorType {
    match user.auth_method {
        crate::auth::middleware::AuthMethod::ApiKey { .. } => ActorType::ApiKey,
        crate::auth::middleware::AuthMethod::Mtls { .. } => ActorType::Agent,
        crate::auth::middleware::AuthMethod::Jwt { .. } => ActorType::User,
    }
}

/// Write a connector audit record; failure is logged, never blocks the API.
async fn write_audit(
    state: &AppState,
    user: &AuthenticatedUser,
    org_id: Uuid,
    action: &str,
    connector_id: Uuid,
    details: Value,
) {
    let entry = AuditEntry::builder(action, actor_type_of(user), user.id.clone())
        .org_id(org_id)
        .resource(ResourceType::Connector, connector_id.to_string())
        .details(details);
    if let Err(e) = entry.log(&state.db).await {
        tracing::error!(error = %e, action, "failed to write connector audit record");
    }
}

// ---- DTOs ----

/// A connector as returned to clients — the secret is NEVER included, only
/// whether one is set.
#[derive(Debug, Serialize, ToSchema)]
struct ConnectorSummary {
    id: Uuid,
    name: String,
    /// `splunk_hec` | `http`.
    connector_type: String,
    endpoint: String,
    /// `ndjson` | `ocsf` | `cef`.
    format: String,
    enabled: bool,
    failure_count: i32,
    /// Whether an HMAC secret / HEC token is configured (the value is never
    /// returned).
    has_secret: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_export_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

impl From<SiemConnector> for ConnectorSummary {
    fn from(c: SiemConnector) -> Self {
        Self {
            id: c.id,
            name: c.name,
            connector_type: c.connector_type.as_str().to_string(),
            endpoint: c.endpoint,
            format: c.format.as_str().to_string(),
            enabled: c.enabled,
            failure_count: c.failure_count,
            has_secret: c.secret.is_some(),
            last_export_at: c.last_export_at,
            created_at: c.created_at,
        }
    }
}

/// The tenant's connectors.
#[derive(Debug, Serialize, ToSchema)]
struct ConnectorListResponse {
    count: usize,
    connectors: Vec<ConnectorSummary>,
}

#[derive(Debug, Deserialize, ToSchema)]
struct CreateConnectorRequest {
    name: String,
    /// `splunk_hec` | `http`.
    connector_type: String,
    endpoint: String,
    /// HMAC secret (http) or HEC token (splunk_hec).
    #[serde(default)]
    secret: Option<String>,
    /// Record shape: `ndjson` | `ocsf` | `cef`. Defaults to `ocsf`.
    #[serde(default)]
    format: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
struct UpdateConnectorRequest {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    endpoint: Option<String>,
    #[serde(default)]
    secret: Option<String>,
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ExportQuery {
    /// Inclusive lower time bound (RFC3339 / `YYYY-MM-DD HH:MM:SS`).
    from: Option<String>,
    /// Exclusive upper time bound.
    to: Option<String>,
    /// Max records to export (default 1000, hard cap 5000).
    limit: Option<u64>,
}

/// Result of a push-export.
#[derive(Debug, Serialize, ToSchema)]
struct ExportResponse {
    /// How many decisions were read from the store and shaped.
    records_read: usize,
    delivery: ConnectorDeliveryResult,
}

// ---- validation helpers ----

fn parse_connector_type(s: &str) -> ApiResult<ConnectorType> {
    ConnectorType::parse(s).ok_or_else(|| {
        ApiError::BadRequest("connector_type must be splunk_hec or http".to_string())
    })
}

fn parse_format(s: &str) -> ApiResult<ExportFormat> {
    ExportFormat::parse(s)
        .ok_or_else(|| ApiError::BadRequest("format must be ndjson, ocsf, or cef".to_string()))
}

fn validate_endpoint(url: &str) -> ApiResult<()> {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err(ApiError::BadRequest(
            "endpoint must be an http(s) URL".to_string(),
        ));
    }
    Ok(())
}

// ---- CRUD ----

/// List SIEM export connectors for an organization.
#[utoipa::path(
    get,
    path = "/orgs/{org}/audit/connectors",
    tag = "audit",
    params(("org" = String, Path, description = "Organization ID")),
    responses(
        (status = 200, description = "Connectors for the org", body = ConnectorListResponse),
        (status = 403, description = "Caller lacks audit:export on this org", body = ProblemDetails),
        (status = 404, description = "Organization not found", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn list_connectors(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
) -> ApiResult<Json<ConnectorListResponse>> {
    let org_id = authorize_export(&state, &user, &org).await?;
    let connectors = AuditConnectorRepository::new(&state.db)
        .list_for_org(org_id)
        .await?;
    Ok(Json(ConnectorListResponse {
        count: connectors.len(),
        connectors: connectors.into_iter().map(Into::into).collect(),
    }))
}

/// Create a SIEM export connector. Audited.
#[utoipa::path(
    post,
    path = "/orgs/{org}/audit/connectors",
    tag = "audit",
    params(("org" = String, Path, description = "Organization ID")),
    request_body = CreateConnectorRequest,
    responses(
        (status = 201, description = "Connector created", body = ConnectorSummary),
        (status = 400, description = "Invalid connector fields", body = ProblemDetails),
        (status = 403, description = "Caller lacks audit:export on this org", body = ProblemDetails),
        (status = 404, description = "Organization not found", body = ProblemDetails),
        (status = 409, description = "A connector with this name already exists", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn create_connector(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Json(req): Json<CreateConnectorRequest>,
) -> ApiResult<(StatusCode, Json<ConnectorSummary>)> {
    let org_id = authorize_export(&state, &user, &org).await?;
    let name = req.name.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest(
            "connector name is required".to_string(),
        ));
    }
    let connector_type = parse_connector_type(&req.connector_type)?;
    let format = match req.format.as_deref() {
        Some(f) => parse_format(f)?,
        None => ExportFormat::Ocsf,
    };
    validate_endpoint(&req.endpoint)?;

    let repo = AuditConnectorRepository::new(&state.db);
    if repo.get_by_name(org_id, name).await?.is_some() {
        return Err(ApiError::Conflict(format!(
            "a connector named '{name}' already exists"
        )));
    }

    let connector = repo
        .create(
            org_id,
            NewConnector {
                name,
                connector_type,
                endpoint: req.endpoint.trim(),
                secret: req.secret.as_deref().filter(|s| !s.is_empty()),
                format,
                created_by: Some(user.id.as_str()),
            },
        )
        .await?;

    write_audit(
        &state,
        &user,
        org_id,
        actions::AUDIT_CONNECTOR_CREATE,
        connector.id,
        json!({
            "name": connector.name,
            "connector_type": connector.connector_type.as_str(),
            "endpoint": connector.endpoint,
            "format": connector.format.as_str(),
        }),
    )
    .await;

    Ok((StatusCode::CREATED, Json(connector.into())))
}

/// Get a connector by ID.
#[utoipa::path(
    get,
    path = "/orgs/{org}/audit/connectors/{id}",
    tag = "audit",
    params(
        ("org" = String, Path, description = "Organization ID"),
        ("id" = Uuid, Path, description = "Connector ID")
    ),
    responses(
        (status = 200, description = "Connector detail", body = ConnectorSummary),
        (status = 403, description = "Caller lacks audit:export on this org", body = ProblemDetails),
        (status = 404, description = "Connector not found", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn get_connector(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, id)): Path<(String, Uuid)>,
) -> ApiResult<Json<ConnectorSummary>> {
    let org_id = authorize_export(&state, &user, &org).await?;
    let connector = AuditConnectorRepository::new(&state.db)
        .get(org_id, id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("connector '{id}' not found")))?;
    Ok(Json(connector.into()))
}

/// Update a connector. Audited.
#[utoipa::path(
    put,
    path = "/orgs/{org}/audit/connectors/{id}",
    tag = "audit",
    params(
        ("org" = String, Path, description = "Organization ID"),
        ("id" = Uuid, Path, description = "Connector ID")
    ),
    request_body = UpdateConnectorRequest,
    responses(
        (status = 200, description = "Connector updated", body = ConnectorSummary),
        (status = 400, description = "Invalid connector fields", body = ProblemDetails),
        (status = 403, description = "Caller lacks audit:export on this org", body = ProblemDetails),
        (status = 404, description = "Connector not found", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn update_connector(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, id)): Path<(String, Uuid)>,
    Json(req): Json<UpdateConnectorRequest>,
) -> ApiResult<Json<ConnectorSummary>> {
    let org_id = authorize_export(&state, &user, &org).await?;
    if let Some(ref ep) = req.endpoint {
        validate_endpoint(ep)?;
    }
    let format = match req.format.as_deref() {
        Some(f) => Some(parse_format(f)?),
        None => None,
    };

    let repo = AuditConnectorRepository::new(&state.db);
    let updated = repo
        .update(
            org_id,
            id,
            ConnectorPatch {
                name: req.name.as_deref().map(str::trim).filter(|s| !s.is_empty()),
                endpoint: req.endpoint.as_deref().map(str::trim),
                secret: req.secret.as_deref().filter(|s| !s.is_empty()),
                format,
                enabled: req.enabled,
            },
        )
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("connector '{id}' not found")))?;

    write_audit(
        &state,
        &user,
        org_id,
        actions::AUDIT_CONNECTOR_UPDATE,
        id,
        json!({ "enabled": updated.enabled, "format": updated.format.as_str() }),
    )
    .await;

    Ok(Json(updated.into()))
}

/// Delete a connector. Audited.
#[utoipa::path(
    delete,
    path = "/orgs/{org}/audit/connectors/{id}",
    tag = "audit",
    params(
        ("org" = String, Path, description = "Organization ID"),
        ("id" = Uuid, Path, description = "Connector ID")
    ),
    responses(
        (status = 204, description = "Connector deleted"),
        (status = 403, description = "Caller lacks audit:export on this org", body = ProblemDetails),
        (status = 404, description = "Connector not found", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn delete_connector(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let org_id = authorize_export(&state, &user, &org).await?;
    let deleted = AuditConnectorRepository::new(&state.db)
        .delete(org_id, id)
        .await?;
    if !deleted {
        return Err(ApiError::NotFound(format!("connector '{id}' not found")));
    }
    write_audit(
        &state,
        &user,
        org_id,
        actions::AUDIT_CONNECTOR_DELETE,
        id,
        json!({}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

// ---- test + export ----

/// Send a single synthetic record to the connector to verify connectivity/auth.
#[utoipa::path(
    post,
    path = "/orgs/{org}/audit/connectors/{id}/test",
    tag = "audit",
    params(
        ("org" = String, Path, description = "Organization ID"),
        ("id" = Uuid, Path, description = "Connector ID")
    ),
    responses(
        (status = 200, description = "Test delivery result", body = ConnectorDeliveryResult),
        (status = 403, description = "Caller lacks audit:export on this org", body = ProblemDetails),
        (status = 404, description = "Connector not found", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn test_connector(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, id)): Path<(String, Uuid)>,
) -> ApiResult<Json<ConnectorDeliveryResult>> {
    let org_id = authorize_export(&state, &user, &org).await?;
    let connector = AuditConnectorRepository::new(&state.db)
        .get(org_id, id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("connector '{id}' not found")))?;

    let mut entry = DecisionLogEntry::new(
        "reaper-connectivity-test".to_string(),
        "test".to_string(),
        "connector/self-test".to_string(),
        "allow".to_string(),
        "connector-test".to_string(),
        "connector-self-test".to_string(),
    );
    entry.agent_id = Some("reaper-management".to_string());
    let line = entry
        .export(connector.format)
        .map_err(|e| ApiError::Internal(format!("shape test record: {e}")))?;

    let result = ConnectorDeliveryService::new()
        .deliver(&connector, &[line])
        .await;
    Ok(Json(result))
}

/// Reconstruct a `DecisionLogEntry` from a queried store row for shaping. The
/// list projection omits chain/provenance columns; those map to `None`, which the
/// exporters already treat as absent.
fn row_to_entry(row: DecisionRow) -> DecisionLogEntry {
    let opt = |s: String| if s.is_empty() { None } else { Some(s) };
    let context = match row.context {
        Value::Object(m) => m.into_iter().collect(),
        _ => std::collections::HashMap::new(),
    };
    let mut e = DecisionLogEntry::new(
        row.principal,
        row.action,
        row.resource,
        row.decision,
        row.policy_id,
        row.policy_name,
    );
    e.timestamp = row.timestamp;
    e.decision_id = row.decision_id;
    e.trace_id = opt(row.trace_id);
    e.context = context;
    e.policy_version = opt(row.policy_version);
    e.matched_rule = opt(row.matched_rule);
    e.evaluation_time_ns = row.evaluation_time_ns;
    e.cache_hit = row.cache_hit != 0;
    e.agent_id = opt(row.agent_id);
    e.input_data = (!row.input_data.is_null()).then_some(row.input_data);
    e.replay_input = (!row.replay_input.is_null()).then_some(row.replay_input);
    e
}

/// Push a range of decisions to the connector now (shaped to its format).
/// Audited. Reads the full history from the central store.
#[utoipa::path(
    post,
    path = "/orgs/{org}/audit/connectors/{id}/export",
    tag = "audit",
    params(
        ("org" = String, Path, description = "Organization ID"),
        ("id" = Uuid, Path, description = "Connector ID"),
        ("from" = Option<String>, Query, description = "Inclusive lower time bound"),
        ("to" = Option<String>, Query, description = "Exclusive upper time bound"),
        ("limit" = Option<u64>, Query, description = "Max records (default 1000, max 5000)")
    ),
    responses(
        (status = 200, description = "Export delivery result", body = ExportResponse),
        (status = 403, description = "Caller lacks audit:export on this org", body = ProblemDetails),
        (status = 404, description = "Connector not found", body = ProblemDetails),
        (status = 503, description = "Decision store not configured/unreachable", body = ProblemDetails)
    ),
    security(("bearer_jwt" = []))
)]
async fn export_now(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, id)): Path<(String, Uuid)>,
    Query(q): Query<ExportQuery>,
) -> ApiResult<Json<ExportResponse>> {
    let org_id = authorize_export(&state, &user, &org).await?;
    let connector = AuditConnectorRepository::new(&state.db)
        .get(org_id, id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("connector '{id}' not found")))?;

    let store = state.decision_store.as_deref().ok_or_else(|| {
        ApiError::ServiceUnavailable(
            "decision store not configured: set REAPER_CLICKHOUSE_URL".to_string(),
        )
    })?;

    let limit = q
        .limit
        .unwrap_or(DEFAULT_EXPORT_LIMIT)
        .min(MAX_EXPORT_LIMIT);
    let query = DecisionQuery {
        from: q.from.clone(),
        to: q.to.clone(),
        limit: Some(limit),
        ..Default::default()
    };
    let tenant = org_id.to_string();
    let rows = store.list(&tenant, &query).await.map_err(|e| match e {
        DecisionStoreError::NotConfigured => {
            ApiError::ServiceUnavailable("decision store not configured".to_string())
        }
        DecisionStoreError::Http(m) => {
            ApiError::ServiceUnavailable(format!("decision store unreachable: {m}"))
        }
        DecisionStoreError::Query(m) | DecisionStoreError::Parse(m) => {
            ApiError::Internal(format!("decision store error: {m}"))
        }
    })?;

    let records_read = rows.len();
    let lines: Vec<String> = rows
        .into_iter()
        .filter_map(|row| row_to_entry(row).export(connector.format).ok())
        .collect();

    let delivery = ConnectorDeliveryService::new()
        .deliver(&connector, &lines)
        .await;

    AuditConnectorRepository::new(&state.db)
        .record_export(connector.id, delivery.success)
        .await
        .ok();

    write_audit(
        &state,
        &user,
        org_id,
        actions::AUDIT_CONNECTOR_EXPORT,
        connector.id,
        json!({
            "records_read": records_read,
            "delivered": delivery.records,
            "success": delivery.success,
            "status_code": delivery.status_code,
            "from": q.from,
            "to": q.to,
        }),
    )
    .await;

    Ok(Json(ExportResponse {
        records_read,
        delivery,
    }))
}
