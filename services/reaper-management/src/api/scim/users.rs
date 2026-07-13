//! SCIM 2.0 Users (RFC 7643/7644 subset).
//!
//! Every handler authenticates via the SCIM token (org from the token) and is
//! tenant-scoped: a token can only see/act on users who are members of its org.
//! Deprovision (DELETE or `active=false`) removes org membership **and** revokes
//! all of the user's sessions, so a terminated user is denied within one request.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

use super::{authenticate_scim, list_response, ScimContext, ScimError, SCHEMA_USER};
use crate::audit::{actions, ActorType, AuditEntry};
use crate::auth::users::{
    OrgRole, SessionRepository, User, UserOrg, UserOrgRepository, UserRepository, UserStatus,
};
use crate::state::AppState;

// ---------- SCIM resource shapes ----------

#[derive(Debug, Deserialize, ToSchema)]
pub struct ScimUserInput {
    #[serde(rename = "userName", default)]
    user_name: Option<String>,
    #[serde(default)]
    emails: Vec<ScimEmail>,
    #[serde(default = "default_true")]
    active: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
struct ScimEmail {
    #[serde(default)]
    value: Option<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    filter: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ScimPatch {
    #[serde(rename = "Operations", default)]
    operations: Vec<ScimPatchOp>,
}

#[derive(Debug, Deserialize, ToSchema)]
struct ScimPatchOp {
    #[serde(default)]
    op: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    value: serde_json::Value,
}

fn user_to_scim(user: &User) -> serde_json::Value {
    serde_json::json!({
        "schemas": [SCHEMA_USER],
        "id": user.id,
        "userName": user.email,
        "emails": [{ "value": user.email, "primary": true }],
        "active": user.status == UserStatus::Active,
        "meta": {
            "resourceType": "User",
            "created": user.created_at,
            "lastModified": user.updated_at,
            "location": format!("/scim/v2/Users/{}", user.id),
        }
    })
}

// ---------- Handlers ----------

/// List users in the caller's org. Supports `filter=userName eq "x"`.
#[utoipa::path(
    get,
    path = "/scim/v2/Users",
    tag = "scim",
    responses(
        (status = 200, description = "SCIM ListResponse of users")
    ),
    security(("bearer_jwt" = []))
)]
pub async fn list_users(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<ListQuery>,
) -> Result<Response, ScimError> {
    let ctx = authenticate_scim(&state, &headers).await?;

    // If a userName filter is present, resolve just that user (and only if a
    // member of this org); otherwise list the org's members.
    let resources = if let Some(email) = parse_username_eq(q.filter.as_deref()) {
        match member_user_by_email(&state, &ctx, &email).await? {
            Some(u) => vec![user_to_scim(&u)],
            None => vec![],
        }
    } else {
        let members = UserOrgRepository::new(&state.db)
            .get_org_members(ctx.org_id)
            .await
            .map_err(|_| ScimError::internal())?;
        let users = UserRepository::new(&state.db);
        let mut out = Vec::new();
        for m in members {
            if let Ok(Some(u)) = users.find_by_id(m.user_id).await {
                out.push(user_to_scim(&u));
            }
        }
        out
    };

    Ok(Json(list_response(resources)).into_response())
}

/// Provision (or adopt) a user and add them to the caller's org.
#[utoipa::path(
    post,
    path = "/scim/v2/Users",
    tag = "scim",
    request_body = ScimUserInput,
    responses(
        (status = 201, description = "User provisioned")
    ),
    security(("bearer_jwt" = []))
)]
pub async fn create_user(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(input): Json<ScimUserInput>,
) -> Result<Response, ScimError> {
    let ctx = authenticate_scim(&state, &headers).await?;
    let email = pick_email(&input)
        .ok_or_else(|| ScimError::bad_request("userName or an email value is required"))?;

    let users = UserRepository::new(&state.db);
    let user = match users
        .find_by_email(&email)
        .await
        .map_err(|_| ScimError::internal())?
    {
        Some(u) => u,
        None => {
            let u = User::external(email.clone(), true);
            users
                .create_external(&u, "scim", &u.id.to_string())
                .await
                .map_err(|_| ScimError::internal())?;
            u
        }
    };

    // Idempotent membership: add at the default role if not already a member.
    let memberships = UserOrgRepository::new(&state.db);
    let existing = memberships
        .get_role(user.id, ctx.org_id)
        .await
        .map_err(|_| ScimError::internal())?;
    if existing.is_none() {
        memberships
            .add_membership(&UserOrg {
                id: Uuid::new_v4(),
                user_id: user.id,
                org_id: ctx.org_id,
                role: OrgRole::Viewer,
                invited_by: None,
                joined_at: chrono::Utc::now(),
            })
            .await
            .map_err(|_| ScimError::internal())?;
    }

    // A create for a currently-suspended user reactivates them.
    if user.status != UserStatus::Active {
        let _ = users.update_status(user.id, UserStatus::Active).await;
    }

    audit(
        &state,
        ctx.org_id,
        actions::SCIM_USER_PROVISION,
        user.id,
        &email,
    )
    .await;

    let fresh = users
        .find_by_id(user.id)
        .await
        .map_err(|_| ScimError::internal())?
        .ok_or_else(ScimError::internal)?;
    Ok((StatusCode::CREATED, Json(user_to_scim(&fresh))).into_response())
}

/// Fetch one SCIM user resource by id.
#[utoipa::path(
    get,
    path = "/scim/v2/Users/{id}",
    tag = "scim",
    params(
        ("id" = Uuid, Path, description = "User ID")
    ),
    responses(
        (status = 200, description = "SCIM User resource")
    ),
    security(("bearer_jwt" = []))
)]
pub async fn get_user(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Response, ScimError> {
    let ctx = authenticate_scim(&state, &headers).await?;
    let user = member_user_by_id(&state, &ctx, id).await?;
    Ok(Json(user_to_scim(&user)).into_response())
}

/// Replace: the only mutable attribute we honor is `active` (deprovision when
/// false).
#[utoipa::path(
    put,
    path = "/scim/v2/Users/{id}",
    tag = "scim",
    params(
        ("id" = Uuid, Path, description = "User ID")
    ),
    request_body = ScimUserInput,
    responses(
        (status = 200, description = "Updated SCIM User resource")
    ),
    security(("bearer_jwt" = []))
)]
pub async fn put_user(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(input): Json<ScimUserInput>,
) -> Result<Response, ScimError> {
    let ctx = authenticate_scim(&state, &headers).await?;
    let user = member_user_by_id(&state, &ctx, id).await?;
    apply_active(&state, &ctx, user.id, input.active).await?;
    let fresh = reload(&state, user.id).await?;
    Ok(Json(user_to_scim(&fresh)).into_response())
}

/// PATCH: we honor `active` toggles (the deprovision signal). Other ops are
/// accepted as no-ops so a conformant client isn't rejected.
#[utoipa::path(
    patch,
    path = "/scim/v2/Users/{id}",
    tag = "scim",
    params(
        ("id" = Uuid, Path, description = "User ID")
    ),
    request_body = ScimPatch,
    responses(
        (status = 200, description = "Patched SCIM User resource")
    ),
    security(("bearer_jwt" = []))
)]
pub async fn patch_user(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(patch): Json<ScimPatch>,
) -> Result<Response, ScimError> {
    let ctx = authenticate_scim(&state, &headers).await?;
    let user = member_user_by_id(&state, &ctx, id).await?;

    if let Some(active) = patch_active(&patch) {
        apply_active(&state, &ctx, user.id, active).await?;
    }
    let fresh = reload(&state, user.id).await?;
    Ok(Json(user_to_scim(&fresh)).into_response())
}

/// Deprovision: remove from the org, revoke every session, suspend if the user
/// has no orgs left.
#[utoipa::path(
    delete,
    path = "/scim/v2/Users/{id}",
    tag = "scim",
    params(
        ("id" = Uuid, Path, description = "User ID")
    ),
    responses(
        (status = 204, description = "User deprovisioned")
    ),
    security(("bearer_jwt" = []))
)]
pub async fn delete_user(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Response, ScimError> {
    let ctx = authenticate_scim(&state, &headers).await?;
    let user = member_user_by_id(&state, &ctx, id).await?;
    deprovision(&state, &ctx, user.id).await?;
    audit(
        &state,
        ctx.org_id,
        actions::SCIM_USER_DEPROVISION,
        user.id,
        &user.email,
    )
    .await;
    Ok(StatusCode::NO_CONTENT.into_response())
}

// ---------- Shared logic ----------

/// Setting active=false is a deprovision; active=true (re)provisions membership.
async fn apply_active(
    state: &AppState,
    ctx: &ScimContext,
    user_id: Uuid,
    active: bool,
) -> Result<(), ScimError> {
    if active {
        let users = UserRepository::new(&state.db);
        let _ = users.update_status(user_id, UserStatus::Active).await;
        audit(
            state,
            ctx.org_id,
            actions::SCIM_USER_UPDATE,
            user_id,
            "active=true",
        )
        .await;
    } else {
        deprovision(state, ctx, user_id).await?;
        audit(
            state,
            ctx.org_id,
            actions::SCIM_USER_DEPROVISION,
            user_id,
            "active=false",
        )
        .await;
    }
    Ok(())
}

async fn deprovision(state: &AppState, ctx: &ScimContext, user_id: Uuid) -> Result<(), ScimError> {
    let memberships = UserOrgRepository::new(&state.db);
    memberships
        .remove_membership(user_id, ctx.org_id)
        .await
        .map_err(|_| ScimError::internal())?;
    // Revoke all live sessions so a terminated user is denied immediately.
    SessionRepository::new(&state.db)
        .delete_all_for_user(user_id)
        .await
        .map_err(|_| ScimError::internal())?;
    // Fully suspend only if they no longer belong to any org.
    let remaining = memberships
        .get_user_orgs(user_id)
        .await
        .map_err(|_| ScimError::internal())?;
    if remaining.is_empty() {
        let _ = UserRepository::new(&state.db)
            .update_status(user_id, UserStatus::Suspended)
            .await;
    }
    Ok(())
}

/// Fetch a user by id only if they belong to the caller's org (tenant guard).
async fn member_user_by_id(
    state: &AppState,
    ctx: &ScimContext,
    id: Uuid,
) -> Result<User, ScimError> {
    let role = UserOrgRepository::new(&state.db)
        .get_role(id, ctx.org_id)
        .await
        .map_err(|_| ScimError::internal())?;
    if role.is_none() {
        return Err(ScimError::not_found());
    }
    UserRepository::new(&state.db)
        .find_by_id(id)
        .await
        .map_err(|_| ScimError::internal())?
        .ok_or_else(ScimError::not_found)
}

async fn member_user_by_email(
    state: &AppState,
    ctx: &ScimContext,
    email: &str,
) -> Result<Option<User>, ScimError> {
    let user = UserRepository::new(&state.db)
        .find_by_email(email)
        .await
        .map_err(|_| ScimError::internal())?;
    match user {
        Some(u) => {
            let is_member = UserOrgRepository::new(&state.db)
                .get_role(u.id, ctx.org_id)
                .await
                .map_err(|_| ScimError::internal())?
                .is_some();
            Ok(is_member.then_some(u))
        }
        None => Ok(None),
    }
}

async fn reload(state: &AppState, id: Uuid) -> Result<User, ScimError> {
    UserRepository::new(&state.db)
        .find_by_id(id)
        .await
        .map_err(|_| ScimError::internal())?
        .ok_or_else(ScimError::not_found)
}

fn pick_email(input: &ScimUserInput) -> Option<String> {
    if let Some(u) = &input.user_name {
        if u.contains('@') {
            return Some(u.trim().to_ascii_lowercase());
        }
    }
    input
        .emails
        .iter()
        .find_map(|e| e.value.as_ref())
        .map(|s| s.trim().to_ascii_lowercase())
        .or_else(|| {
            input
                .user_name
                .as_ref()
                .map(|s| s.trim().to_ascii_lowercase())
        })
}

/// Extract `userName eq "value"` from a SCIM filter (the only filter we support).
fn parse_username_eq(filter: Option<&str>) -> Option<String> {
    let f = filter?.trim();
    let lower = f.to_ascii_lowercase();
    let rest = lower.strip_prefix("username eq ")?;
    // rest is the original-cased value, quoted; recover from `f` to keep case.
    let start = f.len() - rest.len();
    let value = f[start..].trim().trim_matches('"');
    Some(value.to_ascii_lowercase())
}

/// Find an `active` boolean anywhere in a PATCH body (path=="active" with a bool
/// value, or a valueless object `{ "active": false }`, Azure-style).
fn patch_active(patch: &ScimPatch) -> Option<bool> {
    for op in &patch.operations {
        if !matches!(op.op.to_ascii_lowercase().as_str(), "replace" | "add") {
            continue;
        }
        if op.path.as_deref().map(|p| p.eq_ignore_ascii_case("active")) == Some(true) {
            if let Some(b) = op.value.as_bool() {
                return Some(b);
            }
            if let Some(s) = op.value.as_str() {
                return Some(s.eq_ignore_ascii_case("true"));
            }
        }
        if let Some(b) = op.value.get("active").and_then(|v| v.as_bool()) {
            return Some(b);
        }
    }
    None
}

async fn audit(state: &AppState, org_id: Uuid, action: &str, user_id: Uuid, detail: &str) {
    let entry = AuditEntry::builder(action, ActorType::System, "scim")
        .org_id(org_id)
        .details(serde_json::json!({ "user_id": user_id, "detail": detail }));
    let _ = entry.log(&state.db).await;
}
