//! Policy source API endpoints
//!
//! Provides endpoints for managing policy sources (Git and API).

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    api::error::{ApiError, ApiResult},
    api::orgs::resolve_org,
    api::pagination::{PageQuery, Paginated},
    auth::{middleware::RequireAuth, scopes::Scope},
    db::repositories::{OrganizationRepository, PolicySourceRepository},
    domain::source::{
        CreatePolicySource, PolicySource, SourceType, SyncStatus, UpdatePolicySource,
    },
    state::AppState,
};

/// Build source routes
pub fn routes() -> OpenApiRouter<Arc<AppState>> {
    OpenApiRouter::new()
        .routes(routes!(list_sources, create_source))
        .routes(routes!(get_source, update_source, delete_source))
        .routes(routes!(trigger_sync))
}

/// Policy source summary for API responses
#[derive(Debug, Serialize, ToSchema)]
pub struct SourceSummary {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub source_type: String,
    pub config: serde_json::Value,
    pub sync_interval_secs: u32,
    pub sync_status: String,
    pub last_sync_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_sync_error: Option<String>,
    pub last_sync_commit: Option<String>,
    pub is_enabled: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<PolicySource> for SourceSummary {
    fn from(source: PolicySource) -> Self {
        Self {
            id: source.id,
            org_id: source.org_id,
            name: source.name,
            description: source.description,
            source_type: source.source_type.to_string(),
            config: source.config,
            sync_interval_secs: source.sync_interval_secs,
            sync_status: source.sync_status.to_string(),
            last_sync_at: source.last_sync_at,
            last_sync_error: source.last_sync_error,
            last_sync_commit: source.last_sync_commit,
            is_enabled: source.is_enabled,
            created_at: source.created_at,
            updated_at: source.updated_at,
        }
    }
}

/// Request to create a source
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateSourceRequest {
    pub name: String,
    pub description: Option<String>,
    pub source_type: String,
    pub config: serde_json::Value,
    #[serde(default = "default_sync_interval")]
    pub sync_interval_secs: u32,
}

fn default_sync_interval() -> u32 {
    300
}

/// Request to update a source
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateSourceRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub config: Option<serde_json::Value>,
    pub sync_interval_secs: Option<u32>,
    pub is_enabled: Option<bool>,
}

/// Response for sync trigger
#[derive(Debug, Serialize, ToSchema)]
pub struct SyncResponse {
    pub success: bool,
    pub message: String,
    pub policies_found: Option<usize>,
    pub commit: Option<String>,
}

/// List sources for an organization (keyset-paginated: Plan 07 Phase E).
#[utoipa::path(
    get,
    path = "/orgs/{org}/sources",
    tag = "sources",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("limit" = Option<i64>, Query, description = "Page size (default 50, max 200)"),
        ("cursor" = Option<String>, Query, description = "Opaque cursor from the previous page's next_cursor")
    ),
    responses(
        (status = 200, description = "One page of policy sources with a next_cursor to resume"),
        (status = 400, description = "limit out of range or cursor invalid")
    ),
    security(("bearer_jwt" = []))
)]
async fn list_sources(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Query(query): Query<PageQuery>,
) -> ApiResult<Json<Paginated<SourceSummary>>> {
    if !user.has_permission(Scope::PolicyRead) && !user.has_permission(Scope::OrgAdmin) {
        return Err(ApiError::Forbidden("Missing policy:read scope".to_string()));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot access sources for other organizations".to_string(),
        ));
    }

    let page = query.validate()?;

    let source_repo = PolicySourceRepository::new(&state.db);
    let sources = source_repo
        .list_page_by_org(organization.id, page.limit + 1, page.after.as_ref())
        .await?;

    let summaries: Vec<SourceSummary> = sources.into_iter().map(|s| s.into()).collect();

    Ok(Json(Paginated::from_rows(summaries, &page, |s| {
        (s.created_at.to_rfc3339(), s.id.to_string())
    })))
}

/// Get a specific source
#[utoipa::path(
    get,
    path = "/orgs/{org}/sources/{source_id}",
    tag = "sources",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("source_id" = Uuid, Path, description = "Policy source ID")
    ),
    responses(
        (status = 200, description = "Policy source details", body = SourceSummary)
    ),
    security(("bearer_jwt" = []))
)]
async fn get_source(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, source_id)): Path<(String, Uuid)>,
) -> ApiResult<Json<SourceSummary>> {
    if !user.has_permission(Scope::PolicyRead) && !user.has_permission(Scope::OrgAdmin) {
        return Err(ApiError::Forbidden("Missing policy:read scope".to_string()));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot access sources for other organizations".to_string(),
        ));
    }

    let source_repo = PolicySourceRepository::new(&state.db);
    let source = source_repo
        .get_by_id(source_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Source not found".to_string()))?;

    if source.org_id != organization.id {
        return Err(ApiError::NotFound("Source not found".to_string()));
    }

    Ok(Json(source.into()))
}

/// Create a new source
#[utoipa::path(
    post,
    path = "/orgs/{org}/sources",
    tag = "sources",
    params(
        ("org" = String, Path, description = "Organization ID or slug")
    ),
    request_body = CreateSourceRequest,
    responses(
        (status = 201, description = "Policy source created", body = SourceSummary)
    ),
    security(("bearer_jwt" = []))
)]
async fn create_source(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Json(request): Json<CreateSourceRequest>,
) -> ApiResult<(StatusCode, Json<SourceSummary>)> {
    if !user.has_permission(Scope::PolicyWrite) && !user.has_permission(Scope::OrgAdmin) {
        return Err(ApiError::Forbidden(
            "Missing policy:write scope".to_string(),
        ));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot create sources for other organizations".to_string(),
        ));
    }

    // Parse source type
    let source_type: SourceType = request
        .source_type
        .parse()
        .map_err(|e: String| ApiError::Validation(e))?;

    // Check for duplicate name
    let source_repo = PolicySourceRepository::new(&state.db);
    if let Some(_existing) = source_repo
        .get_by_name(organization.id, &request.name)
        .await?
    {
        return Err(ApiError::Conflict(format!(
            "Source with name '{}' already exists",
            request.name
        )));
    }

    // Validate config based on source type
    validate_source_config(source_type, &request.config)?;

    let input = CreatePolicySource {
        name: request.name,
        description: request.description,
        source_type,
        config: request.config,
        sync_interval_secs: request.sync_interval_secs,
    };

    let source = source_repo.create(organization.id, input).await?;

    Ok((StatusCode::CREATED, Json(source.into())))
}

/// Update a source
#[utoipa::path(
    put,
    path = "/orgs/{org}/sources/{source_id}",
    tag = "sources",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("source_id" = Uuid, Path, description = "Policy source ID")
    ),
    request_body = UpdateSourceRequest,
    responses(
        (status = 200, description = "Policy source updated", body = SourceSummary)
    ),
    security(("bearer_jwt" = []))
)]
async fn update_source(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, source_id)): Path<(String, Uuid)>,
    Json(request): Json<UpdateSourceRequest>,
) -> ApiResult<Json<SourceSummary>> {
    if !user.has_permission(Scope::PolicyWrite) && !user.has_permission(Scope::OrgAdmin) {
        return Err(ApiError::Forbidden(
            "Missing policy:write scope".to_string(),
        ));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot update sources for other organizations".to_string(),
        ));
    }

    let source_repo = PolicySourceRepository::new(&state.db);

    // Verify source exists and belongs to this org
    let source = source_repo
        .get_by_id(source_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Source not found".to_string()))?;

    if source.org_id != organization.id {
        return Err(ApiError::NotFound("Source not found".to_string()));
    }

    // Validate config if provided
    if let Some(ref config) = request.config {
        validate_source_config(source.source_type, config)?;
    }

    let input = UpdatePolicySource {
        name: request.name,
        description: request.description,
        config: request.config,
        sync_interval_secs: request.sync_interval_secs,
        is_enabled: request.is_enabled,
    };

    source_repo.update(source_id, input).await?;

    // Fetch updated source
    let updated = source_repo
        .get_by_id(source_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Source not found".to_string()))?;

    Ok(Json(updated.into()))
}

/// Delete a source
#[utoipa::path(
    delete,
    path = "/orgs/{org}/sources/{source_id}",
    tag = "sources",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("source_id" = Uuid, Path, description = "Policy source ID")
    ),
    responses(
        (status = 204, description = "Policy source deleted")
    ),
    security(("bearer_jwt" = []))
)]
async fn delete_source(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, source_id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    if !user.has_permission(Scope::PolicyWrite) && !user.has_permission(Scope::OrgAdmin) {
        return Err(ApiError::Forbidden(
            "Missing policy:write scope".to_string(),
        ));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot delete sources for other organizations".to_string(),
        ));
    }

    let source_repo = PolicySourceRepository::new(&state.db);

    // Verify source exists and belongs to this org
    let source = source_repo
        .get_by_id(source_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Source not found".to_string()))?;

    if source.org_id != organization.id {
        return Err(ApiError::NotFound("Source not found".to_string()));
    }

    source_repo.delete(source_id).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// Trigger sync for a source
#[utoipa::path(
    post,
    path = "/orgs/{org}/sources/{source_id}/sync",
    tag = "sources",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("source_id" = Uuid, Path, description = "Policy source ID")
    ),
    responses(
        (status = 200, description = "Sync triggered", body = SyncResponse)
    ),
    security(("bearer_jwt" = []))
)]
async fn trigger_sync(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, source_id)): Path<(String, Uuid)>,
) -> ApiResult<Json<SyncResponse>> {
    if !user.has_permission(Scope::PolicyWrite) && !user.has_permission(Scope::OrgAdmin) {
        return Err(ApiError::Forbidden(
            "Missing policy:write scope".to_string(),
        ));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot sync sources for other organizations".to_string(),
        ));
    }

    let source_repo = PolicySourceRepository::new(&state.db);

    // Verify source exists and belongs to this org
    let source = source_repo
        .get_by_id(source_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Source not found".to_string()))?;

    if source.org_id != organization.id {
        return Err(ApiError::NotFound("Source not found".to_string()));
    }

    if !source.can_sync() {
        return Err(ApiError::Conflict(format!(
            "Source cannot be synced (status: {}, enabled: {})",
            source.sync_status, source.is_enabled
        )));
    }

    // Get sync service from state and trigger sync
    // For now, we'll just mark the source as syncing
    // In a full implementation, this would use the SyncService
    source_repo
        .update_sync_status(source_id, SyncStatus::Syncing, None, None)
        .await?;

    // TODO: Actually trigger the sync via SyncService
    // For now, just return a placeholder response
    Ok(Json(SyncResponse {
        success: true,
        message: "Sync triggered".to_string(),
        policies_found: None,
        commit: None,
    }))
}

/// Validate source configuration based on type
fn validate_source_config(
    source_type: SourceType,
    config: &serde_json::Value,
) -> Result<(), ApiError> {
    match source_type {
        SourceType::Git => {
            // Must have a URL
            if config.get("url").and_then(|v| v.as_str()).is_none() {
                return Err(ApiError::Validation(
                    "Git source requires 'url' in config".to_string(),
                ));
            }
        }
        SourceType::Api => {
            // Must have a URL
            if config.get("url").and_then(|v| v.as_str()).is_none() {
                return Err(ApiError::Validation(
                    "API source requires 'url' in config".to_string(),
                ));
            }
        }
        SourceType::S3 => {
            // Must have bucket and region
            if config.get("bucket").and_then(|v| v.as_str()).is_none() {
                return Err(ApiError::Validation(
                    "S3 source requires 'bucket' in config".to_string(),
                ));
            }
            if config.get("region").and_then(|v| v.as_str()).is_none() {
                return Err(ApiError::Validation(
                    "S3 source requires 'region' in config".to_string(),
                ));
            }
        }
        SourceType::BundleUrl => {
            // BundleUrl can work without base_url (webhook-only mode)
            // But if checksum verification is enabled, it needs the algorithm
            if config.get("verify_checksum") == Some(&serde_json::Value::Bool(true))
                && config.get("checksum_algorithm").is_none()
            {
                // Default is sha256, so this is fine
            }
        }
    }
    Ok(())
}
