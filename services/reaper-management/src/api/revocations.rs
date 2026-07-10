//! Bundle revocation API (Plan 02, Phase B, step 4).
//!
//! Serves a **signed** revocation list that agents pull on their sync cadence
//! and check at bundle load, plus admin endpoints to revoke / un-revoke a
//! bundle hash or a signing key id. The served list is signed with the same
//! bundle signing key agents already pin, so a compromised CDN/proxy can
//! neither forge nor strip it.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use serde::Deserialize;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{
    api::error::{ApiError, ApiResult},
    api::orgs::authorize_org,
    auth::middleware::RequireAuth,
    auth::scopes::Scope,
    db::repositories::{RevocationEntry, RevocationKind, RevocationRepository},
    state::AppState,
};
use reaper_core::revocation::{RevocationList, SignedRevocationList};

pub fn routes() -> OpenApiRouter<Arc<AppState>> {
    OpenApiRouter::new().routes(routes!(get_revocations, add_revocation, remove_revocation))
}

/// Fetch the org's signed revocation list. Agents call this (agent:read); an
/// org admin can read it too. Errors when signing is not configured — a list
/// that can't be signed is one agents can't trust, so we refuse to serve a
/// forgeable one rather than pretend.
#[utoipa::path(
    get,
    path = "/orgs/{org}/revocations",
    tag = "revocations",
    params(
        ("org" = String, Path, description = "Organization ID or slug")
    ),
    responses((status = 200, description = "Signed revocation list")),
    security(("bearer_jwt" = []))
)]
async fn get_revocations(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
) -> ApiResult<Json<SignedRevocationList>> {
    let organization =
        authorize_org(&state, &user, &org, &[Scope::AgentRead, Scope::OrgAdmin]).await?;

    let set = RevocationRepository::new(&state.db)
        .get_set(organization.id)
        .await?;

    // next_update rides the configured signature validity window: a list is
    // "fresh" for the same horizon as a bundle signature, so an agent that
    // can reach the control plane refetches well within it.
    let now = chrono::Utc::now();
    let next_update =
        now.timestamp() + (state.config.bundles.signature_validity_days as i64).min(30) * 86_400;

    let list = RevocationList {
        issued_at: now.to_rfc3339(),
        serial: set.serial.max(0) as u64,
        next_update,
        revoked_bundle_hashes: set.hashes,
        revoked_key_ids: set.key_ids,
    };

    match state.bundle_service.sign_revocation_list(list) {
        Some(signed) => Ok(Json(signed)),
        None => Err(ApiError::Internal(
            "revocation list requires bundle signing to be configured \
             (set REAPER_BUNDLE_SIGNING_KEY)"
                .to_string(),
        )),
    }
}

#[derive(Debug, Deserialize, ToSchema)]
struct RevokeRequest {
    /// "hash" (bundle-bytes sha256, lowercase hex) or "key_id".
    kind: String,
    value: String,
    reason: Option<String>,
}

/// Revoke a bundle hash or signing key id (org admin). Bumps the list serial.
#[utoipa::path(
    post,
    path = "/orgs/{org}/revocations",
    tag = "revocations",
    params(
        ("org" = String, Path, description = "Organization ID or slug")
    ),
    request_body = RevokeRequest,
    responses((status = 201, description = "Revocation added; serial bumped")),
    security(("bearer_jwt" = []))
)]
async fn add_revocation(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Json(req): Json<RevokeRequest>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let organization = authorize_org(&state, &user, &org, &[Scope::OrgAdmin]).await?;
    let kind = RevocationKind::parse(&req.kind)
        .ok_or_else(|| ApiError::BadRequest("kind must be 'hash' or 'key_id'".to_string()))?;
    if req.value.trim().is_empty() {
        return Err(ApiError::BadRequest("value is required".to_string()));
    }

    let serial = RevocationRepository::new(&state.db)
        .add(
            organization.id,
            &RevocationEntry {
                kind,
                value: req.value.trim().to_string(),
                reason: req.reason,
            },
        )
        .await?;

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({ "revoked": true, "serial": serial })),
    ))
}

#[derive(Debug, Deserialize, ToSchema)]
struct UnrevokeRequest {
    kind: String,
    value: String,
}

/// Un-revoke (org admin). Bumps the serial so agents refetch.
#[utoipa::path(
    delete,
    path = "/orgs/{org}/revocations",
    tag = "revocations",
    params(
        ("org" = String, Path, description = "Organization ID or slug")
    ),
    request_body = UnrevokeRequest,
    responses((status = 200, description = "Revocation removed; serial bumped")),
    security(("bearer_jwt" = []))
)]
async fn remove_revocation(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Json(req): Json<UnrevokeRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let organization = authorize_org(&state, &user, &org, &[Scope::OrgAdmin]).await?;
    let kind = RevocationKind::parse(&req.kind)
        .ok_or_else(|| ApiError::BadRequest("kind must be 'hash' or 'key_id'".to_string()))?;
    let serial = RevocationRepository::new(&state.db)
        .remove(organization.id, kind, req.value.trim())
        .await?;
    Ok(Json(
        serde_json::json!({ "revoked": false, "serial": serial }),
    ))
}
