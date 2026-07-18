//! SCIM token administration (org admin). Standard-auth endpoints to mint,
//! list, and revoke the per-org SCIM bearer tokens.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::api::error::{ApiError, ApiResult};
use crate::api::orgs::authorize_org;
use crate::audit::{actions, ActorType, AuditEntry};
use crate::auth::middleware::{AuthMethod, AuthenticatedUser, RequireAuth};
use crate::auth::scim::store::ScimTokenStore;
use crate::auth::scopes::Scope;
use crate::state::AppState;

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateTokenRequest {
    pub name: String,
}

/// Mint a SCIM token. The plaintext is returned **once** in this response.
#[utoipa::path(
    post,
    path = "/orgs/{org}/scim/tokens",
    tag = "scim",
    params(
        ("org" = String, Path, description = "Organization ID or slug")
    ),
    request_body = CreateTokenRequest,
    responses(
        (status = 201, description = "SCIM token created (plaintext returned once)")
    ),
    security(("bearer_jwt" = []))
)]
pub async fn create_token(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Json(req): Json<CreateTokenRequest>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let organization = authorize_org(&state, &user, &org, &[Scope::OrgAdmin]).await?;
    if req.name.trim().is_empty() {
        return Err(ApiError::BadRequest("token name is required".into()));
    }
    let minted = ScimTokenStore::new(&state.db)
        .create(organization.id, req.name.trim(), Some(&user.id))
        .await?;

    audit(
        &state,
        &user,
        organization.id,
        actions::SCIM_TOKEN_CREATE,
        minted.token.id,
    )
    .await;

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "id": minted.token.id,
            "name": minted.token.name,
            "created_at": minted.token.created_at,
            // Shown once — store it now, it cannot be retrieved again.
            "token": minted.plaintext,
        })),
    ))
}

/// List an org's SCIM tokens (metadata only — never the secret).
#[utoipa::path(
    get,
    path = "/orgs/{org}/scim/tokens",
    tag = "scim",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("limit" = Option<i64>, Query, description = "Max to return (default 200, max 500)")
    ),
    responses(
        (status = 200, description = "List of SCIM token metadata")
    ),
    security(("bearer_jwt" = []))
)]
pub async fn list_tokens(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Query(page): Query<crate::api::pagination::LimitQuery>,
) -> ApiResult<Json<Vec<serde_json::Value>>> {
    let organization = authorize_org(&state, &user, &org, &[Scope::OrgAdmin]).await?;
    let tokens = ScimTokenStore::new(&state.db)
        .list(organization.id, page.cap()?)
        .await?;
    let out = tokens
        .into_iter()
        .map(|t| {
            serde_json::json!({
                "id": t.id,
                "name": t.name,
                "created_by": t.created_by,
                "created_at": t.created_at,
                "last_used_at": t.last_used_at,
                "revoked": t.revoked,
            })
        })
        .collect();
    Ok(Json(out))
}

/// Revoke a SCIM token.
#[utoipa::path(
    delete,
    path = "/orgs/{org}/scim/tokens/{token_id}",
    tag = "scim",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("token_id" = Uuid, Path, description = "SCIM token ID")
    ),
    responses(
        (status = 204, description = "SCIM token revoked")
    ),
    security(("bearer_jwt" = []))
)]
pub async fn revoke_token(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, token_id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let organization = authorize_org(&state, &user, &org, &[Scope::OrgAdmin]).await?;
    let revoked = ScimTokenStore::new(&state.db)
        .revoke(organization.id, token_id)
        .await?;
    if !revoked {
        return Err(ApiError::NotFound(format!(
            "SCIM token not found: {token_id}"
        )));
    }
    audit(
        &state,
        &user,
        organization.id,
        actions::SCIM_TOKEN_REVOKE,
        token_id,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

fn actor_type_of(user: &AuthenticatedUser) -> ActorType {
    match user.auth_method {
        AuthMethod::ApiKey { .. } => ActorType::ApiKey,
        AuthMethod::Mtls { .. } => ActorType::Agent,
        AuthMethod::Jwt { .. } => ActorType::User,
    }
}

async fn audit(
    state: &AppState,
    user: &AuthenticatedUser,
    org_id: Uuid,
    action: &str,
    token_id: Uuid,
) {
    let entry = AuditEntry::builder(action, actor_type_of(user), user.id.clone())
        .org_id(org_id)
        .details(serde_json::json!({ "scim_token_id": token_id }));
    let _ = entry.log(&state.db).await;
}
