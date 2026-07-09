//! SCIM 2.0 Groups (read-only, Plan 03 Phase 2).
//!
//! Reaper's authorization model is role-based, so we expose the org's four
//! roles as SCIM Groups with their current members. This lets an IdP *discover*
//! group structure; pushing membership changes via SCIM Group PATCH (role
//! assignment through directory sync) is a later phase — for now roles are set
//! at OIDC login (group→role mapping) or via the API.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::HeaderMap,
    response::{IntoResponse, Response},
    Json,
};

use super::{authenticate_scim, list_response, ScimContext, ScimError, SCHEMA_GROUP};
use crate::auth::users::{OrgRole, UserOrgRepository, UserRepository};
use crate::state::AppState;

const ROLES: [OrgRole; 4] = [
    OrgRole::Owner,
    OrgRole::Admin,
    OrgRole::Developer,
    OrgRole::Viewer,
];

pub async fn list_groups(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Response, ScimError> {
    let ctx = authenticate_scim(&state, &headers).await?;
    let mut resources = Vec::new();
    for role in ROLES {
        resources.push(group_resource(&state, &ctx, role).await?);
    }
    Ok(Json(list_response(resources)).into_response())
}

pub async fn get_group(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ScimError> {
    let ctx = authenticate_scim(&state, &headers).await?;
    let role = id.parse::<OrgRole>().map_err(|_| ScimError::not_found())?;
    Ok(Json(group_resource(&state, &ctx, role).await?).into_response())
}

async fn group_resource(
    state: &AppState,
    ctx: &ScimContext,
    role: OrgRole,
) -> Result<serde_json::Value, ScimError> {
    let members = UserOrgRepository::new(&state.db)
        .get_org_members(ctx.org_id)
        .await
        .map_err(|_| ScimError::internal())?;
    let users = UserRepository::new(&state.db);

    let mut member_json = Vec::new();
    for m in members.into_iter().filter(|m| m.role == role) {
        if let Ok(Some(u)) = users.find_by_id(m.user_id).await {
            member_json.push(serde_json::json!({
                "value": u.id,
                "display": u.email,
            }));
        }
    }

    Ok(serde_json::json!({
        "schemas": [SCHEMA_GROUP],
        "id": role.to_string(),
        "displayName": role.to_string(),
        "members": member_json,
        "meta": { "resourceType": "Group", "location": format!("/scim/v2/Groups/{role}") },
    }))
}
