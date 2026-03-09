//! Organization member management handlers.

use axum::{
    extract::{Path, State},
    http::{header::HeaderMap, StatusCode},
    response::Json,
};
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    api::error::{ApiError, ApiResult},
    audit::{actions, ActorType, AuditEntry, ClientInfo, ResourceType},
    auth::{
        middleware::RequireAuth,
        users::{OrgRole, UserOrg, UserOrgRepository, UserRepository},
    },
    db::repositories::OrganizationRepository,
    state::AppState,
};

use super::types::{
    InviteMemberRequest, ListMembersResponse, MemberInfo, UpdateRoleRequest, UserInfo,
};

/// List members of an organization
pub async fn list_org_members(
    State(state): State<Arc<AppState>>,
    RequireAuth(auth_user): RequireAuth,
    Path(org): Path<String>,
) -> ApiResult<Json<ListMembersResponse>> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = crate::api::orgs::resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org
    let user_org_repo = UserOrgRepository::new(&state.db);
    let user_id = Uuid::parse_str(&auth_user.id)
        .map_err(|_| ApiError::Internal("Invalid user ID".to_string()))?;

    let _role = user_org_repo
        .get_role(user_id, organization.id)
        .await?
        .ok_or_else(|| {
            ApiError::Forbidden("You are not a member of this organization".to_string())
        })?;

    // Get all members
    let memberships = user_org_repo.get_org_members(organization.id).await?;

    let user_repo = UserRepository::new(&state.db);
    let mut members = Vec::new();
    for membership in memberships {
        if let Some(user) = user_repo.find_by_id(membership.user_id).await? {
            members.push(MemberInfo {
                user: UserInfo::from(&user),
                role: membership.role,
                joined_at: membership.joined_at,
                invited_by: membership.invited_by,
            });
        }
    }

    Ok(Json(ListMembersResponse { members }))
}

/// Get a specific member
pub async fn get_member(
    State(state): State<Arc<AppState>>,
    RequireAuth(auth_user): RequireAuth,
    Path((org, member_id)): Path<(String, Uuid)>,
) -> ApiResult<Json<MemberInfo>> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = crate::api::orgs::resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org
    let user_org_repo = UserOrgRepository::new(&state.db);
    let user_id = Uuid::parse_str(&auth_user.id)
        .map_err(|_| ApiError::Internal("Invalid user ID".to_string()))?;

    user_org_repo
        .get_role(user_id, organization.id)
        .await?
        .ok_or_else(|| {
            ApiError::Forbidden("You are not a member of this organization".to_string())
        })?;

    // Get the member's role
    let role = user_org_repo
        .get_role(member_id, organization.id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Member not found".to_string()))?;

    // Get the membership details
    let memberships = user_org_repo.get_org_members(organization.id).await?;
    let membership = memberships
        .iter()
        .find(|m| m.user_id == member_id)
        .ok_or_else(|| ApiError::NotFound("Member not found".to_string()))?;

    let user_repo = UserRepository::new(&state.db);
    let user = user_repo
        .find_by_id(member_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("User not found".to_string()))?;

    Ok(Json(MemberInfo {
        user: UserInfo::from(&user),
        role,
        joined_at: membership.joined_at,
        invited_by: membership.invited_by,
    }))
}

/// Invite a member to an organization
pub async fn invite_member(
    State(state): State<Arc<AppState>>,
    RequireAuth(auth_user): RequireAuth,
    headers: HeaderMap,
    Path(org): Path<String>,
    Json(request): Json<InviteMemberRequest>,
) -> ApiResult<(StatusCode, Json<MemberInfo>)> {
    let client_info = ClientInfo::from_headers(&headers);

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = crate::api::orgs::resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org and has permission
    let user_org_repo = UserOrgRepository::new(&state.db);
    let user_id = Uuid::parse_str(&auth_user.id)
        .map_err(|_| ApiError::Internal("Invalid user ID".to_string()))?;

    let role = user_org_repo
        .get_role(user_id, organization.id)
        .await?
        .ok_or_else(|| {
            ApiError::Forbidden("You are not a member of this organization".to_string())
        })?;

    if !role.can_manage_users() {
        return Err(ApiError::Forbidden(
            "You don't have permission to invite members".to_string(),
        ));
    }

    // Can't invite someone as owner (only one owner allowed)
    if request.role == OrgRole::Owner {
        return Err(ApiError::BadRequest(
            "Cannot invite someone as owner. Use transfer ownership instead.".to_string(),
        ));
    }

    // Find the user to invite
    let user_repo = UserRepository::new(&state.db);
    let invite_user = user_repo
        .find_by_email(&request.email)
        .await?
        .ok_or_else(|| ApiError::NotFound("User not found with that email".to_string()))?;

    // Check if already a member
    if user_org_repo
        .get_role(invite_user.id, organization.id)
        .await?
        .is_some()
    {
        return Err(ApiError::Conflict(
            "User is already a member of this organization".to_string(),
        ));
    }

    // Add membership
    let membership = UserOrg {
        id: Uuid::new_v4(),
        user_id: invite_user.id,
        org_id: organization.id,
        role: request.role,
        invited_by: Some(user_id),
        joined_at: chrono::Utc::now(),
    };
    user_org_repo.add_membership(&membership).await?;

    // Audit log
    AuditEntry::builder(
        actions::ORG_MEMBER_ADD,
        ActorType::User,
        user_id.to_string(),
    )
    .org_id(organization.id)
    .resource(ResourceType::User, invite_user.id.to_string())
    .ip_address(client_info.ip_address.unwrap_or_default())
    .user_agent(client_info.user_agent.unwrap_or_default())
    .details(serde_json::json!({
        "email": invite_user.email,
        "role": request.role.to_string()
    }))
    .log(&state.db)
    .await
    .ok();

    Ok((
        StatusCode::CREATED,
        Json(MemberInfo {
            user: UserInfo::from(&invite_user),
            role: request.role,
            joined_at: membership.joined_at,
            invited_by: Some(user_id),
        }),
    ))
}

/// Update a member's role
pub async fn update_member_role(
    State(state): State<Arc<AppState>>,
    RequireAuth(auth_user): RequireAuth,
    headers: HeaderMap,
    Path((org, member_id)): Path<(String, Uuid)>,
    Json(request): Json<UpdateRoleRequest>,
) -> ApiResult<StatusCode> {
    let client_info = ClientInfo::from_headers(&headers);

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = crate::api::orgs::resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org and has permission
    let user_org_repo = UserOrgRepository::new(&state.db);
    let user_id = Uuid::parse_str(&auth_user.id)
        .map_err(|_| ApiError::Internal("Invalid user ID".to_string()))?;

    let role = user_org_repo
        .get_role(user_id, organization.id)
        .await?
        .ok_or_else(|| {
            ApiError::Forbidden("You are not a member of this organization".to_string())
        })?;

    if !role.can_manage_users() {
        return Err(ApiError::Forbidden(
            "You don't have permission to update member roles".to_string(),
        ));
    }

    // Can't change to/from owner
    let current_role = user_org_repo
        .get_role(member_id, organization.id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Member not found".to_string()))?;

    if current_role == OrgRole::Owner || request.role == OrgRole::Owner {
        return Err(ApiError::BadRequest(
            "Cannot change owner role. Use transfer ownership instead.".to_string(),
        ));
    }

    // Can't demote yourself
    if member_id == user_id {
        return Err(ApiError::BadRequest(
            "Cannot change your own role".to_string(),
        ));
    }

    // Update role
    user_org_repo
        .update_role(member_id, organization.id, request.role)
        .await?;

    // Audit log
    AuditEntry::builder(
        actions::ORG_MEMBER_ROLE_CHANGE,
        ActorType::User,
        user_id.to_string(),
    )
    .org_id(organization.id)
    .resource(ResourceType::User, member_id.to_string())
    .ip_address(client_info.ip_address.unwrap_or_default())
    .user_agent(client_info.user_agent.unwrap_or_default())
    .details(serde_json::json!({
        "old_role": current_role.to_string(),
        "new_role": request.role.to_string()
    }))
    .log(&state.db)
    .await
    .ok();

    Ok(StatusCode::NO_CONTENT)
}

/// Remove a member from an organization
pub async fn remove_member(
    State(state): State<Arc<AppState>>,
    RequireAuth(auth_user): RequireAuth,
    headers: HeaderMap,
    Path((org, member_id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let client_info = ClientInfo::from_headers(&headers);

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = crate::api::orgs::resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org and has permission
    let user_org_repo = UserOrgRepository::new(&state.db);
    let user_id = Uuid::parse_str(&auth_user.id)
        .map_err(|_| ApiError::Internal("Invalid user ID".to_string()))?;

    let role = user_org_repo
        .get_role(user_id, organization.id)
        .await?
        .ok_or_else(|| {
            ApiError::Forbidden("You are not a member of this organization".to_string())
        })?;

    if !role.can_manage_users() {
        return Err(ApiError::Forbidden(
            "You don't have permission to remove members".to_string(),
        ));
    }

    // Can't remove owner
    let target_role = user_org_repo
        .get_role(member_id, organization.id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Member not found".to_string()))?;

    if target_role == OrgRole::Owner {
        return Err(ApiError::BadRequest(
            "Cannot remove the owner. Transfer ownership first.".to_string(),
        ));
    }

    // Can't remove yourself (use leave org instead)
    if member_id == user_id {
        return Err(ApiError::BadRequest(
            "Cannot remove yourself. Use leave organization instead.".to_string(),
        ));
    }

    // Remove membership
    user_org_repo
        .remove_membership(member_id, organization.id)
        .await?;

    // Audit log
    AuditEntry::builder(
        actions::ORG_MEMBER_REMOVE,
        ActorType::User,
        user_id.to_string(),
    )
    .org_id(organization.id)
    .resource(ResourceType::User, member_id.to_string())
    .ip_address(client_info.ip_address.unwrap_or_default())
    .user_agent(client_info.user_agent.unwrap_or_default())
    .log(&state.db)
    .await
    .ok();

    Ok(StatusCode::NO_CONTENT)
}
