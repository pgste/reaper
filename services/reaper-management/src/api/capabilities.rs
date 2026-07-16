//! Capability issuance API (F1-s3 agentic authz).
//!
//! Mints short-lived, attenuable, signed capabilities — "actor X may exercise
//! these grants on behalf of subject Y until T" — with the SAME bundle
//! signing key agents already pin, so verification needs no new key
//! distribution. Attenuation is issuer-side re-issuance (the locked design:
//! holders cannot mint), and revocation rides the org's signed revocation
//! list, which agents already pull on their sync cadence.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use serde::{Deserialize, Serialize};
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
use reaper_core::capability::{Capability, Grant};

/// Hard ceiling on capability lifetime. Capabilities are DERIVED, EXPIRING
/// credentials — an agent that needs standing access should hold a real
/// identity, not a day-plus capability.
const MAX_TTL_SECS: i64 = 86_400;

/// Clock-skew allowance: freshly-minted capabilities are valid from
/// `now - SKEW` so an agent with a slightly-behind clock accepts them.
const SKEW_SECS: i64 = 60;

pub fn routes() -> OpenApiRouter<Arc<AppState>> {
    OpenApiRouter::new()
        .routes(routes!(issue_capability))
        .routes(routes!(attenuate_capability))
        .routes(routes!(revoke_capability))
}

/// One (action, resource) grant. Patterns: literal, `*`, or trailing-`*`
/// prefix.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct GrantDto {
    pub action: String,
    pub resource: String,
}

/// A signed capability envelope (mirrors `reaper_core::capability::Capability`).
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CapabilityDto {
    /// Envelope version.
    pub v: u32,
    /// Unique id (the revocation handle).
    pub id: String,
    /// Signature algorithm.
    pub algorithm: String,
    /// Issuing key id.
    pub key_id: String,
    /// The durable principal this capability derives from.
    pub subject: String,
    /// The non-human actor allowed to wield it.
    pub actor: String,
    /// What the actor may do.
    pub grants: Vec<GrantDto>,
    /// Validity window (unix seconds, inclusive).
    pub not_before: i64,
    pub expires_at: i64,
    /// Ancestor capability ids (revoking any ancestor revokes this one).
    #[serde(default)]
    pub ancestry: Vec<String>,
    /// Hex signature over the canonical claims.
    pub signature: String,
}

impl From<Capability> for CapabilityDto {
    fn from(c: Capability) -> Self {
        Self {
            v: c.v,
            id: c.id,
            algorithm: c.algorithm,
            key_id: c.key_id,
            subject: c.subject,
            actor: c.actor,
            grants: c
                .grants
                .into_iter()
                .map(|g| GrantDto {
                    action: g.action,
                    resource: g.resource,
                })
                .collect(),
            not_before: c.not_before,
            expires_at: c.expires_at,
            ancestry: c.ancestry,
            signature: c.signature,
        }
    }
}

impl From<CapabilityDto> for Capability {
    fn from(d: CapabilityDto) -> Self {
        Self {
            v: d.v,
            id: d.id,
            algorithm: d.algorithm,
            key_id: d.key_id,
            subject: d.subject,
            actor: d.actor,
            grants: d
                .grants
                .into_iter()
                .map(|g| Grant::new(g.action, g.resource))
                .collect(),
            not_before: d.not_before,
            expires_at: d.expires_at,
            ancestry: d.ancestry,
            signature: d.signature,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct IssueCapabilityRequest {
    /// The durable principal the capability derives from.
    pub subject: String,
    /// The non-human actor that will wield it.
    pub actor: String,
    /// Granted (action, resource) patterns — must be non-empty.
    pub grants: Vec<GrantDto>,
    /// Lifetime in seconds from now (1..=86400).
    pub ttl_secs: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CapabilityResponse {
    pub capability: CapabilityDto,
}

fn validate_common(
    subject: &str,
    actor: &str,
    grants: &[GrantDto],
    ttl_secs: i64,
) -> Result<(), ApiError> {
    if subject.trim().is_empty() || actor.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "subject and actor are required".to_string(),
        ));
    }
    if grants.is_empty() {
        return Err(ApiError::BadRequest(
            "grants must be non-empty (an empty capability authorizes nothing)".to_string(),
        ));
    }
    if !(1..=MAX_TTL_SECS).contains(&ttl_secs) {
        return Err(ApiError::BadRequest(format!(
            "ttl_secs must be in 1..={MAX_TTL_SECS} (capabilities are short-lived by design)"
        )));
    }
    Ok(())
}

fn to_grants(grants: Vec<GrantDto>) -> Vec<Grant> {
    grants
        .into_iter()
        .map(|g| Grant::new(g.action, g.resource))
        .collect()
}

/// Issue a root capability signed with the org's bundle signing key.
#[utoipa::path(
    post,
    path = "/orgs/{org}/capabilities",
    tag = "capabilities",
    params(
        ("org" = String, Path, description = "Organization ID or slug")
    ),
    request_body = IssueCapabilityRequest,
    responses(
        (status = 201, description = "Signed capability", body = CapabilityResponse),
        (status = 400, description = "Invalid request"),
        (status = 403, description = "Missing capability:issue")
    ),
    security(("bearer_jwt" = []))
)]
async fn issue_capability(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Json(req): Json<IssueCapabilityRequest>,
) -> ApiResult<(StatusCode, Json<CapabilityResponse>)> {
    authorize_org(&state, &user, &org, &[Scope::CapabilityIssue]).await?;
    validate_common(&req.subject, &req.actor, &req.grants, req.ttl_secs)?;

    let now = chrono::Utc::now().timestamp();
    let cap = state
        .bundle_service
        .issue_capability(
            req.subject.trim(),
            req.actor.trim(),
            to_grants(req.grants),
            now - SKEW_SECS,
            now + req.ttl_secs,
        )
        .ok_or_else(|| {
            ApiError::Internal(
                "capability issuance requires bundle signing to be configured \
                 (set REAPER_BUNDLE_SIGNING_KEY)"
                    .to_string(),
            )
        })?
        .map_err(|e| ApiError::BadRequest(format!("capability issuance failed: {e}")))?;

    Ok((
        StatusCode::CREATED,
        Json(CapabilityResponse {
            capability: cap.into(),
        }),
    ))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AttenuateCapabilityRequest {
    /// The parent capability to narrow (full signed envelope). Its signature
    /// is verified before anything is minted.
    pub parent: CapabilityDto,
    /// The (possibly different) actor for the narrowed capability — the
    /// orchestrator-hands-to-sub-agent flow.
    pub actor: String,
    /// Narrowed grants — every one must be covered by a parent grant.
    pub grants: Vec<GrantDto>,
    /// Lifetime in seconds from now; the window must nest inside the
    /// parent's.
    pub ttl_secs: i64,
}

/// Attenuate a capability into a strictly-narrower one (issuer-side
/// re-issuance; holders cannot mint). Widened grants or windows are
/// rejected by construction, and a revoked parent cannot be attenuated.
#[utoipa::path(
    post,
    path = "/orgs/{org}/capabilities/attenuate",
    tag = "capabilities",
    params(
        ("org" = String, Path, description = "Organization ID or slug")
    ),
    request_body = AttenuateCapabilityRequest,
    responses(
        (status = 201, description = "Narrowed signed capability", body = CapabilityResponse),
        (status = 400, description = "Invalid parent, widened grant/window"),
        (status = 403, description = "Missing capability:issue")
    ),
    security(("bearer_jwt" = []))
)]
async fn attenuate_capability(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Json(req): Json<AttenuateCapabilityRequest>,
) -> ApiResult<(StatusCode, Json<CapabilityResponse>)> {
    let organization = authorize_org(&state, &user, &org, &[Scope::CapabilityIssue]).await?;
    validate_common(&req.parent.subject, &req.actor, &req.grants, req.ttl_secs)?;

    // A revoked parent (or ancestor) must not spawn fresh authority.
    let revoked: std::collections::HashSet<String> = RevocationRepository::new(&state.db)
        .get_set(organization.id)
        .await?
        .capability_ids
        .into_iter()
        .collect();

    let now = chrono::Utc::now().timestamp();
    let parent: Capability = req.parent.into();
    // The child window nests inside the parent's: start no earlier than the
    // parent (minus nothing — parent.not_before is already in force) and end
    // at the requested ttl, still subject to the parent's end via the core's
    // WidenedWindow check.
    let not_before = (now - SKEW_SECS).max(parent.not_before);

    let cap = state
        .bundle_service
        .attenuate_capability(
            &parent,
            req.actor.trim(),
            to_grants(req.grants),
            not_before,
            now + req.ttl_secs,
            &revoked,
            now,
        )
        .ok_or_else(|| {
            ApiError::Internal(
                "capability issuance requires bundle signing to be configured \
                 (set REAPER_BUNDLE_SIGNING_KEY)"
                    .to_string(),
            )
        })?
        .map_err(|e| ApiError::BadRequest(format!("attenuation rejected: {e}")))?;

    Ok((
        StatusCode::CREATED,
        Json(CapabilityResponse {
            capability: cap.into(),
        }),
    ))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RevokeCapabilityRequest {
    /// The capability id to revoke. Every capability derived from it dies
    /// with it (ancestry check at verification).
    pub capability_id: String,
    pub reason: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RevokeCapabilityResponse {
    pub revoked: bool,
    /// New revocation-list serial; agents pick it up on their sync cadence.
    pub serial: i64,
}

/// Revoke a capability id via the org's signed revocation list.
#[utoipa::path(
    post,
    path = "/orgs/{org}/capabilities/revoke",
    tag = "capabilities",
    params(
        ("org" = String, Path, description = "Organization ID or slug")
    ),
    request_body = RevokeCapabilityRequest,
    responses(
        (status = 201, description = "Capability revoked; list serial bumped",
         body = RevokeCapabilityResponse),
        (status = 400, description = "Invalid request"),
        (status = 403, description = "Missing capability:revoke")
    ),
    security(("bearer_jwt" = []))
)]
async fn revoke_capability(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Json(req): Json<RevokeCapabilityRequest>,
) -> ApiResult<(StatusCode, Json<RevokeCapabilityResponse>)> {
    let organization = authorize_org(&state, &user, &org, &[Scope::CapabilityRevoke]).await?;
    if req.capability_id.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "capability_id is required".to_string(),
        ));
    }

    let serial = RevocationRepository::new(&state.db)
        .add(
            organization.id,
            &RevocationEntry {
                kind: RevocationKind::Capability,
                value: req.capability_id.trim().to_string(),
                reason: req.reason,
            },
        )
        .await?;

    Ok((
        StatusCode::CREATED,
        Json(RevokeCapabilityResponse {
            revoked: true,
            serial,
        }),
    ))
}
