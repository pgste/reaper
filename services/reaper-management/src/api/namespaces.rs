//! Namespace API endpoints
//!
//! Provides endpoints for managing namespaces and agent subscriptions.

use axum::{
    extract::{Path, State},
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
    auth::{middleware::RequireAuth, scopes::Scope},
    db::repositories::{AgentRepository, NamespaceRepository, OrganizationRepository},
    domain::namespace::{
        build_namespace_tree, CreateAgentSubscription, CreateNamespace, Namespace, NamespaceTree,
        UpdateNamespace,
    },
    state::AppState,
};

/// Build namespace routes
pub fn routes() -> OpenApiRouter<Arc<AppState>> {
    OpenApiRouter::new()
        // Namespace CRUD
        .routes(routes!(list_namespaces, create_namespace))
        .routes(routes!(get_namespace, update_namespace, delete_namespace))
        // Namespace tree view
        .routes(routes!(get_namespace_tree))
        // Agent subscriptions
        .routes(routes!(list_agent_subscriptions, create_agent_subscription))
        .routes(routes!(delete_agent_subscription))
}

/// Namespace summary for API responses
#[derive(Debug, Serialize, ToSchema)]
pub struct NamespaceSummary {
    pub id: Uuid,
    pub org_id: Uuid,
    pub slug: String,
    pub display_name: Option<String>,
    pub parent_id: Option<Uuid>,
    pub description: Option<String>,
    pub settings: serde_json::Value,
    pub is_active: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<Namespace> for NamespaceSummary {
    fn from(ns: Namespace) -> Self {
        Self {
            id: ns.id,
            org_id: ns.org_id,
            slug: ns.slug,
            display_name: ns.display_name,
            parent_id: ns.parent_id,
            description: ns.description,
            settings: ns.settings,
            is_active: ns.is_active,
            created_at: ns.created_at,
            updated_at: ns.updated_at,
        }
    }
}

/// Response for listing namespaces
#[derive(Debug, Serialize, ToSchema)]
pub struct ListNamespacesResponse {
    pub namespaces: Vec<NamespaceSummary>,
    pub total: usize,
}

/// Request to create a namespace
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateNamespaceRequest {
    pub slug: String,
    pub display_name: Option<String>,
    pub parent_id: Option<Uuid>,
    pub description: Option<String>,
    #[serde(default)]
    pub settings: serde_json::Value,
}

/// Request to update a namespace
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateNamespaceRequest {
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub settings: Option<serde_json::Value>,
    pub is_active: Option<bool>,
}

/// Response for namespace tree
#[derive(Debug, Serialize, ToSchema)]
pub struct NamespaceTreeResponse {
    pub roots: Vec<NamespaceTreeNode>,
    pub total: usize,
}

/// Tree node for response
#[derive(Debug, Serialize, ToSchema)]
pub struct NamespaceTreeNode {
    pub namespace: NamespaceSummary,
    // Self-referential: stop utoipa's schema generator from expanding the
    // subtree inline (which recurses forever) — it emits a `$ref` instead.
    #[schema(no_recursion)]
    pub children: Vec<NamespaceTreeNode>,
}

impl From<NamespaceTree> for NamespaceTreeNode {
    fn from(tree: NamespaceTree) -> Self {
        Self {
            namespace: tree.namespace.into(),
            children: tree.children.into_iter().map(|c| c.into()).collect(),
        }
    }
}

/// Subscription summary for API responses
#[derive(Debug, Serialize, ToSchema)]
pub struct SubscriptionSummary {
    pub agent_id: Uuid,
    pub namespace_id: Uuid,
    pub namespace_slug: Option<String>,
    pub include_children: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Response for listing subscriptions
#[derive(Debug, Serialize, ToSchema)]
pub struct ListSubscriptionsResponse {
    pub subscriptions: Vec<SubscriptionSummary>,
    pub total: usize,
}

/// Request to create a subscription
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateSubscriptionRequest {
    pub namespace_id: Uuid,
    #[serde(default = "default_include_children")]
    pub include_children: bool,
}

fn default_include_children() -> bool {
    true
}

// ===== Namespace Handlers =====

/// List namespaces for an organization
#[utoipa::path(
    get,
    path = "/orgs/{org}/namespaces",
    tag = "namespaces",
    params(
        ("org" = String, Path, description = "Organization ID or slug")
    ),
    responses(
        (status = 200, description = "List of namespaces", body = ListNamespacesResponse)
    ),
    security(("bearer_jwt" = []))
)]
async fn list_namespaces(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
) -> ApiResult<Json<ListNamespacesResponse>> {
    if !user.has_permission(Scope::PolicyRead) && !user.has_permission(Scope::OrgAdmin) {
        return Err(ApiError::Forbidden("Missing policy:read scope".to_string()));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot access namespaces for other organizations".to_string(),
        ));
    }

    let ns_repo = NamespaceRepository::new(&state.db);
    let namespaces = ns_repo.list_by_org(organization.id).await?;

    let total = namespaces.len();
    let summaries: Vec<NamespaceSummary> = namespaces.into_iter().map(|n| n.into()).collect();

    Ok(Json(ListNamespacesResponse {
        namespaces: summaries,
        total,
    }))
}

/// Get namespace tree for an organization
#[utoipa::path(
    get,
    path = "/orgs/{org}/namespaces/tree",
    tag = "namespaces",
    params(
        ("org" = String, Path, description = "Organization ID or slug")
    ),
    responses(
        (status = 200, description = "Namespace tree", body = NamespaceTreeResponse)
    ),
    security(("bearer_jwt" = []))
)]
async fn get_namespace_tree(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
) -> ApiResult<Json<NamespaceTreeResponse>> {
    if !user.has_permission(Scope::PolicyRead) && !user.has_permission(Scope::OrgAdmin) {
        return Err(ApiError::Forbidden("Missing policy:read scope".to_string()));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot access namespaces for other organizations".to_string(),
        ));
    }

    let ns_repo = NamespaceRepository::new(&state.db);
    let namespaces = ns_repo.list_by_org(organization.id).await?;
    let total = namespaces.len();

    let tree = build_namespace_tree(namespaces);
    let roots: Vec<NamespaceTreeNode> = tree.into_iter().map(|t| t.into()).collect();

    Ok(Json(NamespaceTreeResponse { roots, total }))
}

/// Get a specific namespace
#[utoipa::path(
    get,
    path = "/orgs/{org}/namespaces/{namespace}",
    tag = "namespaces",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("namespace" = String, Path, description = "Namespace ID or slug")
    ),
    responses(
        (status = 200, description = "Namespace details", body = NamespaceSummary)
    ),
    security(("bearer_jwt" = []))
)]
async fn get_namespace(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, namespace)): Path<(String, String)>,
) -> ApiResult<Json<NamespaceSummary>> {
    if !user.has_permission(Scope::PolicyRead) && !user.has_permission(Scope::OrgAdmin) {
        return Err(ApiError::Forbidden("Missing policy:read scope".to_string()));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot access namespaces for other organizations".to_string(),
        ));
    }

    let ns_repo = NamespaceRepository::new(&state.db);

    // Try parsing as UUID first, then as slug
    let ns = if let Ok(id) = Uuid::parse_str(&namespace) {
        ns_repo.get_by_id(id).await?
    } else {
        ns_repo.get_by_slug(organization.id, &namespace).await?
    };

    let ns = ns.ok_or_else(|| ApiError::NotFound("Namespace not found".to_string()))?;

    if ns.org_id != organization.id {
        return Err(ApiError::NotFound("Namespace not found".to_string()));
    }

    Ok(Json(ns.into()))
}

/// Create a new namespace
#[utoipa::path(
    post,
    path = "/orgs/{org}/namespaces",
    tag = "namespaces",
    params(
        ("org" = String, Path, description = "Organization ID or slug")
    ),
    request_body = CreateNamespaceRequest,
    responses(
        (status = 201, description = "Namespace created", body = NamespaceSummary)
    ),
    security(("bearer_jwt" = []))
)]
async fn create_namespace(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Json(request): Json<CreateNamespaceRequest>,
) -> ApiResult<(StatusCode, Json<NamespaceSummary>)> {
    if !user.has_permission(Scope::PolicyWrite) && !user.has_permission(Scope::OrgAdmin) {
        return Err(ApiError::Forbidden(
            "Missing policy:write scope".to_string(),
        ));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot create namespaces for other organizations".to_string(),
        ));
    }

    // Validate slug format
    if !is_valid_slug(&request.slug) {
        return Err(ApiError::Validation(
            "Invalid slug format. Use lowercase letters, numbers, hyphens, and forward slashes only.".to_string(),
        ));
    }

    let ns_repo = NamespaceRepository::new(&state.db);

    // Check for duplicate slug
    if let Some(_existing) = ns_repo.get_by_slug(organization.id, &request.slug).await? {
        return Err(ApiError::Conflict(format!(
            "Namespace with slug '{}' already exists",
            request.slug
        )));
    }

    // Validate parent exists if specified
    if let Some(parent_id) = request.parent_id {
        let parent = ns_repo.get_by_id(parent_id).await?;
        if parent.is_none() || parent.map(|p| p.org_id) != Some(organization.id) {
            return Err(ApiError::Validation(
                "Parent namespace not found".to_string(),
            ));
        }
    }

    let input = CreateNamespace {
        slug: request.slug,
        display_name: request.display_name,
        parent_id: request.parent_id,
        description: request.description,
        settings: request.settings,
    };

    let ns = ns_repo.create(organization.id, input).await?;

    Ok((StatusCode::CREATED, Json(ns.into())))
}

/// Update a namespace
#[utoipa::path(
    put,
    path = "/orgs/{org}/namespaces/{namespace}",
    tag = "namespaces",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("namespace" = String, Path, description = "Namespace ID or slug")
    ),
    request_body = UpdateNamespaceRequest,
    responses(
        (status = 200, description = "Namespace updated", body = NamespaceSummary)
    ),
    security(("bearer_jwt" = []))
)]
async fn update_namespace(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, namespace)): Path<(String, String)>,
    Json(request): Json<UpdateNamespaceRequest>,
) -> ApiResult<Json<NamespaceSummary>> {
    if !user.has_permission(Scope::PolicyWrite) && !user.has_permission(Scope::OrgAdmin) {
        return Err(ApiError::Forbidden(
            "Missing policy:write scope".to_string(),
        ));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot update namespaces for other organizations".to_string(),
        ));
    }

    let ns_repo = NamespaceRepository::new(&state.db);

    // Find namespace by ID or slug
    let ns = if let Ok(id) = Uuid::parse_str(&namespace) {
        ns_repo.get_by_id(id).await?
    } else {
        ns_repo.get_by_slug(organization.id, &namespace).await?
    };

    let ns = ns.ok_or_else(|| ApiError::NotFound("Namespace not found".to_string()))?;

    if ns.org_id != organization.id {
        return Err(ApiError::NotFound("Namespace not found".to_string()));
    }

    let input = UpdateNamespace {
        display_name: request.display_name,
        description: request.description,
        settings: request.settings,
        is_active: request.is_active,
    };

    ns_repo.update(ns.id, input).await?;

    // Fetch updated namespace
    let updated = ns_repo
        .get_by_id(ns.id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Namespace not found".to_string()))?;

    Ok(Json(updated.into()))
}

/// Delete a namespace
#[utoipa::path(
    delete,
    path = "/orgs/{org}/namespaces/{namespace}",
    tag = "namespaces",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("namespace" = String, Path, description = "Namespace ID or slug")
    ),
    responses(
        (status = 204, description = "Namespace deleted")
    ),
    security(("bearer_jwt" = []))
)]
async fn delete_namespace(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, namespace)): Path<(String, String)>,
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
            "Cannot delete namespaces for other organizations".to_string(),
        ));
    }

    let ns_repo = NamespaceRepository::new(&state.db);

    // Find namespace by ID or slug
    let ns = if let Ok(id) = Uuid::parse_str(&namespace) {
        ns_repo.get_by_id(id).await?
    } else {
        ns_repo.get_by_slug(organization.id, &namespace).await?
    };

    let ns = ns.ok_or_else(|| ApiError::NotFound("Namespace not found".to_string()))?;

    if ns.org_id != organization.id {
        return Err(ApiError::NotFound("Namespace not found".to_string()));
    }

    ns_repo.delete(ns.id).await?;

    Ok(StatusCode::NO_CONTENT)
}

// ===== Subscription Handlers =====

/// List subscriptions for an agent
#[utoipa::path(
    get,
    path = "/orgs/{org}/agents/{agent_id}/subscriptions",
    tag = "namespaces",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("agent_id" = Uuid, Path, description = "Agent ID")
    ),
    responses(
        (status = 200, description = "List of agent subscriptions", body = ListSubscriptionsResponse)
    ),
    security(("bearer_jwt" = []))
)]
async fn list_agent_subscriptions(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, agent_id)): Path<(String, Uuid)>,
) -> ApiResult<Json<ListSubscriptionsResponse>> {
    if !user.has_permission(Scope::AgentRead) && !user.has_permission(Scope::OrgAdmin) {
        return Err(ApiError::Forbidden("Missing agent:read scope".to_string()));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot access subscriptions for other organizations".to_string(),
        ));
    }

    // Verify agent exists and belongs to org
    let agent_repo = AgentRepository::new(&state.db);
    let agent = agent_repo
        .get_by_id(agent_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Agent not found".to_string()))?;

    if agent.org_id != organization.id {
        return Err(ApiError::NotFound("Agent not found".to_string()));
    }

    let ns_repo = NamespaceRepository::new(&state.db);
    let subscriptions = ns_repo.get_agent_subscriptions(agent_id).await?;

    // Get namespace slugs for display
    let mut summaries = Vec::with_capacity(subscriptions.len());
    for sub in subscriptions {
        let ns_slug = ns_repo.get_by_id(sub.namespace_id).await?.map(|ns| ns.slug);

        summaries.push(SubscriptionSummary {
            agent_id: sub.agent_id,
            namespace_id: sub.namespace_id,
            namespace_slug: ns_slug,
            include_children: sub.include_children,
            created_at: sub.created_at,
        });
    }

    let total = summaries.len();

    Ok(Json(ListSubscriptionsResponse {
        subscriptions: summaries,
        total,
    }))
}

/// Create a subscription for an agent
#[utoipa::path(
    post,
    path = "/orgs/{org}/agents/{agent_id}/subscriptions",
    tag = "namespaces",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("agent_id" = Uuid, Path, description = "Agent ID")
    ),
    request_body = CreateSubscriptionRequest,
    responses(
        (status = 201, description = "Subscription created", body = SubscriptionSummary)
    ),
    security(("bearer_jwt" = []))
)]
async fn create_agent_subscription(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, agent_id)): Path<(String, Uuid)>,
    Json(request): Json<CreateSubscriptionRequest>,
) -> ApiResult<(StatusCode, Json<SubscriptionSummary>)> {
    if !user.has_permission(Scope::AgentWrite) && !user.has_permission(Scope::OrgAdmin) {
        return Err(ApiError::Forbidden("Missing agent:write scope".to_string()));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot create subscriptions for other organizations".to_string(),
        ));
    }

    // Verify agent exists and belongs to org
    let agent_repo = AgentRepository::new(&state.db);
    let agent = agent_repo
        .get_by_id(agent_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Agent not found".to_string()))?;

    if agent.org_id != organization.id {
        return Err(ApiError::NotFound("Agent not found".to_string()));
    }

    // Verify namespace exists and belongs to org
    let ns_repo = NamespaceRepository::new(&state.db);
    let ns = ns_repo
        .get_by_id(request.namespace_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Namespace not found".to_string()))?;

    if ns.org_id != organization.id {
        return Err(ApiError::NotFound("Namespace not found".to_string()));
    }

    let input = CreateAgentSubscription {
        namespace_id: request.namespace_id,
        include_children: request.include_children,
    };

    let subscription = ns_repo.create_subscription(agent_id, input).await?;

    Ok((
        StatusCode::CREATED,
        Json(SubscriptionSummary {
            agent_id: subscription.agent_id,
            namespace_id: subscription.namespace_id,
            namespace_slug: Some(ns.slug),
            include_children: subscription.include_children,
            created_at: subscription.created_at,
        }),
    ))
}

/// Delete a subscription for an agent
#[utoipa::path(
    delete,
    path = "/orgs/{org}/agents/{agent_id}/subscriptions/{namespace_id}",
    tag = "namespaces",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("agent_id" = Uuid, Path, description = "Agent ID"),
        ("namespace_id" = Uuid, Path, description = "Namespace ID")
    ),
    responses(
        (status = 204, description = "Subscription deleted")
    ),
    security(("bearer_jwt" = []))
)]
async fn delete_agent_subscription(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, agent_id, namespace_id)): Path<(String, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    if !user.has_permission(Scope::AgentWrite) && !user.has_permission(Scope::OrgAdmin) {
        return Err(ApiError::Forbidden("Missing agent:write scope".to_string()));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot delete subscriptions for other organizations".to_string(),
        ));
    }

    // Verify agent exists and belongs to org
    let agent_repo = AgentRepository::new(&state.db);
    let agent = agent_repo
        .get_by_id(agent_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Agent not found".to_string()))?;

    if agent.org_id != organization.id {
        return Err(ApiError::NotFound("Agent not found".to_string()));
    }

    let ns_repo = NamespaceRepository::new(&state.db);
    ns_repo.delete_subscription(agent_id, namespace_id).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// Validate namespace slug format
fn is_valid_slug(slug: &str) -> bool {
    if slug.is_empty() || slug.len() > 255 {
        return false;
    }

    // Must start and end with alphanumeric
    let chars: Vec<char> = slug.chars().collect();
    if !chars
        .first()
        .map(|c| c.is_ascii_alphanumeric())
        .unwrap_or(false)
    {
        return false;
    }
    if !chars
        .last()
        .map(|c| c.is_ascii_alphanumeric())
        .unwrap_or(false)
    {
        return false;
    }

    // Only lowercase letters, numbers, hyphens, and forward slashes
    slug.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '/')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_slugs() {
        assert!(is_valid_slug("production"));
        assert!(is_valid_slug("production-us-east"));
        assert!(is_valid_slug("production/us-east"));
        assert!(is_valid_slug("prod123"));
        assert!(is_valid_slug("a"));
    }

    #[test]
    fn test_invalid_slugs() {
        assert!(!is_valid_slug(""));
        assert!(!is_valid_slug("-production"));
        assert!(!is_valid_slug("production-"));
        assert!(!is_valid_slug("Production")); // uppercase
        assert!(!is_valid_slug("prod_env")); // underscore
        assert!(!is_valid_slug("/production")); // starts with slash
    }
}
