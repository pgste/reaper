//! OAuth connection management handlers

use axum::{
    extract::{Path, State},
    http::{header::HeaderMap, StatusCode},
    response::Json,
};
use chrono::{DateTime, Utc};
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    api::error::{ApiError, ApiResult},
    api::orgs::resolve_org,
    audit::{actions, ActorType, AuditEntry, ClientInfo, ResourceType},
    auth::{middleware::RequireAuth, users::UserOrgRepository},
    db::repositories::OrganizationRepository,
    state::AppState,
};

use super::types::{ConnectionSummary, ListConnectionsResponse};

/// List OAuth connections for an org
pub(super) async fn list_connections(
    State(state): State<Arc<AppState>>,
    RequireAuth(auth_user): RequireAuth,
    Path(org): Path<String>,
) -> ApiResult<Json<ListConnectionsResponse>> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user has access
    let user_id = Uuid::parse_str(&auth_user.id)
        .map_err(|_| ApiError::Internal("Invalid user ID".to_string()))?;

    let user_org_repo = UserOrgRepository::new(&state.db);
    user_org_repo
        .get_role(user_id, organization.id)
        .await?
        .ok_or_else(|| {
            ApiError::Forbidden("You are not a member of this organization".to_string())
        })?;

    // Get connections
    let pool = state.db.sqlite_pool().ok_or(sqlx::Error::PoolClosed)?;
    let rows: Vec<(String, Option<String>, String, String)> = sqlx::query_as(
        "SELECT provider, provider_username, scopes, created_at FROM oauth_connections WHERE org_id = ?",
    )
    .bind(organization.id.to_string())
    .fetch_all(pool)
    .await?;

    let connections = rows
        .into_iter()
        .map(|(provider, username, scopes_json, created_at)| {
            let scopes: Vec<String> = serde_json::from_str(&scopes_json).unwrap_or_default();
            ConnectionSummary {
                provider,
                provider_username: username,
                connected_at: DateTime::parse_from_rfc3339(&created_at)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                scopes,
            }
        })
        .collect();

    Ok(Json(ListConnectionsResponse { connections }))
}

/// Get a specific OAuth connection
pub(super) async fn get_connection(
    State(state): State<Arc<AppState>>,
    RequireAuth(auth_user): RequireAuth,
    Path((org, provider)): Path<(String, String)>,
) -> ApiResult<Json<ConnectionSummary>> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user has access
    let user_id = Uuid::parse_str(&auth_user.id)
        .map_err(|_| ApiError::Internal("Invalid user ID".to_string()))?;

    let user_org_repo = UserOrgRepository::new(&state.db);
    user_org_repo
        .get_role(user_id, organization.id)
        .await?
        .ok_or_else(|| {
            ApiError::Forbidden("You are not a member of this organization".to_string())
        })?;

    // Get connection
    let pool = state.db.sqlite_pool().ok_or(sqlx::Error::PoolClosed)?;
    let row: Option<(String, Option<String>, String, String)> = sqlx::query_as(
        "SELECT provider, provider_username, scopes, created_at FROM oauth_connections WHERE org_id = ? AND provider = ?",
    )
    .bind(organization.id.to_string())
    .bind(&provider)
    .fetch_optional(pool)
    .await?;

    let (provider, username, scopes_json, created_at) = row
        .ok_or_else(|| ApiError::NotFound("OAuth connection not found".to_string()))?;

    let scopes: Vec<String> = serde_json::from_str(&scopes_json).unwrap_or_default();

    Ok(Json(ConnectionSummary {
        provider,
        provider_username: username,
        connected_at: DateTime::parse_from_rfc3339(&created_at)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
        scopes,
    }))
}

/// Create OAuth connection manually (for testing/admin)
pub(super) async fn create_connection(
    State(_state): State<Arc<AppState>>,
    RequireAuth(_auth_user): RequireAuth,
    Path(_org): Path<String>,
) -> ApiResult<StatusCode> {
    // This would be used for manual token entry (admin only)
    Err(ApiError::BadRequest(
        "Use OAuth flow to create connections".to_string(),
    ))
}

/// Delete OAuth connection
pub(super) async fn delete_connection(
    State(state): State<Arc<AppState>>,
    RequireAuth(auth_user): RequireAuth,
    headers: HeaderMap,
    Path((org, provider)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    let client_info = ClientInfo::from_headers(&headers);

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user has permission
    let user_id = Uuid::parse_str(&auth_user.id)
        .map_err(|_| ApiError::Internal("Invalid user ID".to_string()))?;

    let user_org_repo = UserOrgRepository::new(&state.db);
    let role = user_org_repo
        .get_role(user_id, organization.id)
        .await?
        .ok_or_else(|| {
            ApiError::Forbidden("You are not a member of this organization".to_string())
        })?;

    if !role.can_manage_policies() {
        return Err(ApiError::Forbidden(
            "You don't have permission to disconnect OAuth providers".to_string(),
        ));
    }

    // Delete connection
    let pool = state.db.sqlite_pool().ok_or(sqlx::Error::PoolClosed)?;
    let result = sqlx::query("DELETE FROM oauth_connections WHERE org_id = ? AND provider = ?")
        .bind(organization.id.to_string())
        .bind(&provider)
        .execute(pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound("OAuth connection not found".to_string()));
    }

    // Audit log
    AuditEntry::builder(
        actions::OAUTH_DISCONNECT,
        ActorType::User,
        user_id.to_string(),
    )
    .org_id(organization.id)
    .resource(ResourceType::Org, organization.id.to_string())
    .ip_address(client_info.ip_address.unwrap_or_default())
    .user_agent(client_info.user_agent.unwrap_or_default())
    .details(serde_json::json!({
        "provider": provider
    }))
    .log(&state.db)
    .await
    .ok();

    Ok(StatusCode::NO_CONTENT)
}
