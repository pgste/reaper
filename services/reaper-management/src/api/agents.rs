//! Agent API endpoints
//!
//! Provides endpoints for agent registration and management.

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
    auth::{jwt::JwtManager, middleware::RequireAuth, scopes::Scope},
    db::repositories::{AgentRepository, OrganizationRepository},
    domain::agent::{Agent, RegisterAgent},
    state::{AppState, ServerEvent},
};

/// Build agent routes
pub fn routes() -> OpenApiRouter<Arc<AppState>> {
    OpenApiRouter::new()
        // Agent self-registration (uses API key auth)
        .routes(routes!(register_agent))
        // Agent listing and details (requires auth)
        .routes(routes!(list_agents))
        .routes(routes!(get_agent, delete_agent))
        // Heartbeat endpoint
        .routes(routes!(heartbeat))
        // Deploy-status report: agent confirms the bundle version it applied
        .routes(routes!(report_deployment))
}

/// Agent's report of the bundle version it just applied (or failed to apply).
#[derive(Debug, Deserialize, ToSchema)]
pub struct DeploymentReportRequest {
    pub bundle_id: Uuid,
    #[serde(default)]
    pub checksum: Option<String>,
    /// "deployed" or "failed".
    pub status: String,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DeploymentReportResponse {
    pub acknowledged: bool,
}

/// Request to register an agent
#[derive(Debug, Deserialize, ToSchema)]
pub struct RegisterAgentRequest {
    pub name: String,
    pub hostname: Option<String>,
    pub version: Option<String>,
    #[serde(default)]
    pub labels: serde_json::Value,
}

/// Response after successful registration
#[derive(Debug, Serialize, ToSchema)]
pub struct RegisterAgentResponse {
    pub agent: AgentSummary,
    /// JWT token for subsequent requests
    pub token: String,
    pub token_expires_at: chrono::DateTime<chrono::Utc>,
}

/// Agent summary for API responses
#[derive(Debug, Serialize, ToSchema)]
pub struct AgentSummary {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    pub hostname: Option<String>,
    pub version: Option<String>,
    pub status: String,
    pub labels: serde_json::Value,
    pub last_heartbeat_at: Option<chrono::DateTime<chrono::Utc>>,
    pub registered_at: chrono::DateTime<chrono::Utc>,
}

impl From<Agent> for AgentSummary {
    fn from(agent: Agent) -> Self {
        Self {
            id: agent.id,
            org_id: agent.org_id,
            name: agent.name,
            hostname: agent.hostname,
            version: agent.version,
            status: agent.status.to_string(),
            labels: agent.labels,
            last_heartbeat_at: agent.last_heartbeat_at,
            registered_at: agent.registered_at,
        }
    }
}

/// Heartbeat request
#[derive(Debug, Deserialize)]
pub struct HeartbeatRequest {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub metrics: Option<crate::domain::agent::AgentMetrics>,
}

/// Heartbeat response
#[derive(Debug, Serialize, ToSchema)]
pub struct HeartbeatResponse {
    pub acknowledged: bool,
    pub server_time: chrono::DateTime<chrono::Utc>,
}

/// Register a new agent
#[utoipa::path(
    post,
    path = "/orgs/{org}/agents/register",
    tag = "agents",
    params(
        ("org" = String, Path, description = "Organization ID or slug")
    ),
    request_body = RegisterAgentRequest,
    responses(
        (status = 201, description = "Agent registered", body = RegisterAgentResponse)
    ),
    security(("bearer_jwt" = []))
)]
async fn register_agent(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Json(request): Json<RegisterAgentRequest>,
) -> ApiResult<(StatusCode, Json<RegisterAgentResponse>)> {
    // Check permissions
    if !user.has_permission(Scope::AgentRegister) && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Missing agent:register scope".to_string(),
        ));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify API key belongs to this org
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot register agents for other organizations".to_string(),
        ));
    }

    // Check if agent with this name already exists
    let agent_repo = AgentRepository::new(&state.db);
    if let Some(_existing) = agent_repo
        .get_by_name(organization.id, &request.name)
        .await?
    {
        return Err(ApiError::Conflict(format!(
            "Agent with name '{}' already exists",
            request.name
        )));
    }

    // Create the agent
    let input = RegisterAgent {
        name: request.name,
        hostname: request.hostname,
        version: request.version,
        labels: request.labels,
    };

    let agent = agent_repo.create(organization.id, input).await?;

    // Generate JWT token for the agent
    let jwt_secret = state
        .config
        .auth
        .jwt_secret
        .as_ref()
        .ok_or_else(|| ApiError::Internal("JWT not configured".to_string()))?;

    let manager = JwtManager::with_secret(
        jwt_secret,
        &state.config.auth.jwt_issuer,
        &state.config.auth.jwt_audience,
        state.config.auth.jwt_expiry_hours,
    );

    let agent_scopes = vec![
        Scope::AgentRead.to_string(),
        Scope::PolicyRead.to_string(),
        Scope::BundleRead.to_string(),
    ];

    let token = manager
        .generate(&agent.id.to_string(), organization.id, agent_scopes, None)
        .map_err(|e| ApiError::Internal(format!("Failed to generate token: {}", e)))?;

    let claims = manager
        .validate(&token)
        .map_err(|e| ApiError::Internal(format!("Token validation failed: {}", e)))?;

    let expires_at =
        chrono::DateTime::from_timestamp(claims.exp, 0).unwrap_or_else(chrono::Utc::now);

    // Broadcast agent registered event
    state.broadcast_event(ServerEvent::AgentRegistered {
        agent_id: agent.id,
        agent_name: agent.name.clone(),
        org_id: organization.id,
        namespace_id: None, // Agents are org-wide, namespace subscriptions are separate
    });

    Ok((
        StatusCode::CREATED,
        Json(RegisterAgentResponse {
            agent: agent.into(),
            token,
            token_expires_at: expires_at,
        }),
    ))
}

/// List agents for an organization (keyset-paginated: Plan 07 Phase E).
#[utoipa::path(
    get,
    path = "/orgs/{org}/agents",
    tag = "agents",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("limit" = Option<i64>, Query, description = "Page size (default 50, max 200)"),
        ("cursor" = Option<String>, Query, description = "Opaque cursor from the previous page's next_cursor")
    ),
    responses(
        (status = 200, description = "One page of agents with a next_cursor to resume"),
        (status = 400, description = "limit out of range or cursor invalid")
    ),
    security(("bearer_jwt" = []))
)]
async fn list_agents(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Query(query): Query<PageQuery>,
) -> ApiResult<Json<Paginated<AgentSummary>>> {
    // Check permissions
    if !user.has_permission(Scope::AgentRead) && !user.has_permission(Scope::OrgAdmin) {
        return Err(ApiError::Forbidden("Missing agent:read scope".to_string()));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot access agents for other organizations".to_string(),
        ));
    }

    let page = query.validate()?;

    let agent_repo = AgentRepository::new(&state.db);
    let agents = agent_repo
        .list_page_by_org(organization.id, page.limit + 1, page.after.as_ref())
        .await?;

    let summaries: Vec<AgentSummary> = agents.into_iter().map(|a| a.into()).collect();

    Ok(Json(Paginated::from_rows(summaries, &page, |a| {
        (a.registered_at.to_rfc3339(), a.id.to_string())
    })))
}

/// Get agent by ID
#[utoipa::path(
    get,
    path = "/orgs/{org}/agents/{agent_id}",
    tag = "agents",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("agent_id" = Uuid, Path, description = "Agent ID")
    ),
    responses(
        (status = 200, description = "Agent details", body = AgentSummary)
    ),
    security(("bearer_jwt" = []))
)]
async fn get_agent(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, agent_id)): Path<(String, Uuid)>,
) -> ApiResult<Json<AgentSummary>> {
    // Check permissions
    if !user.has_permission(Scope::AgentRead) && !user.has_permission(Scope::OrgAdmin) {
        return Err(ApiError::Forbidden("Missing agent:read scope".to_string()));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot access agents for other organizations".to_string(),
        ));
    }

    let agent_repo = AgentRepository::new(&state.db);
    let agent = agent_repo
        .get_by_id(agent_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Agent not found".to_string()))?;

    // Verify agent belongs to this org
    if agent.org_id != organization.id {
        return Err(ApiError::NotFound("Agent not found".to_string()));
    }

    Ok(Json(agent.into()))
}

/// Delete an agent
#[utoipa::path(
    delete,
    path = "/orgs/{org}/agents/{agent_id}",
    tag = "agents",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("agent_id" = Uuid, Path, description = "Agent ID")
    ),
    responses(
        (status = 204, description = "Agent deleted")
    ),
    security(("bearer_jwt" = []))
)]
async fn delete_agent(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, agent_id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    // Check permissions
    if !user.has_permission(Scope::AgentWrite) && !user.has_permission(Scope::OrgAdmin) {
        return Err(ApiError::Forbidden("Missing agent:write scope".to_string()));
    }

    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot delete agents for other organizations".to_string(),
        ));
    }

    let agent_repo = AgentRepository::new(&state.db);

    // Verify agent exists and belongs to this org
    let agent = agent_repo
        .get_by_id(agent_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Agent not found".to_string()))?;

    if agent.org_id != organization.id {
        return Err(ApiError::NotFound("Agent not found".to_string()));
    }

    agent_repo.delete(agent_id).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// Agent heartbeat
#[utoipa::path(
    post,
    path = "/orgs/{org}/agents/{agent_id}/heartbeat",
    tag = "agents",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("agent_id" = Uuid, Path, description = "Agent ID")
    ),
    responses(
        (status = 200, description = "Heartbeat acknowledged", body = HeartbeatResponse)
    ),
    security(("bearer_jwt" = []))
)]
async fn heartbeat(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, agent_id)): Path<(String, Uuid)>,
    Json(request): Json<HeartbeatRequest>,
) -> ApiResult<Json<HeartbeatResponse>> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org (agent's JWT token)
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot send heartbeat for other organizations".to_string(),
        ));
    }

    let agent_repo = AgentRepository::new(&state.db);

    // Verify agent exists and belongs to this org
    let agent = agent_repo
        .get_by_id(agent_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Agent not found".to_string()))?;

    if agent.org_id != organization.id {
        return Err(ApiError::NotFound("Agent not found".to_string()));
    }

    // Update heartbeat timestamp
    agent_repo.update_heartbeat(agent_id).await?;

    // Store metrics if provided
    if let Some(ref metrics) = request.metrics {
        // Read the previous stale flag BEFORE overwriting the row so
        // stale/fresh transitions can be detected (alert on the edge,
        // never on every heartbeat).
        let previous_stale = agent_repo
            .get_metrics(agent_id)
            .await
            .ok()
            .flatten()
            .and_then(|m| m.data_stale);

        if let Err(e) = agent_repo.update_metrics(agent_id, metrics).await {
            tracing::warn!(
                agent_id = %agent_id,
                error = %e,
                "Failed to store agent metrics"
            );
        }

        match data_stale_transition(previous_stale, metrics.data_stale) {
            Some(true) => {
                tracing::warn!(
                    agent_id = %agent_id,
                    agent_name = %agent.name,
                    data_version = metrics.data_version.unwrap_or(0),
                    data_applied_seq = metrics.data_applied_seq.unwrap_or(0),
                    "agent data replica exceeded its staleness budget"
                );
                let _ = state.event_tx.send(ServerEvent::AgentDataStale {
                    agent_id,
                    agent_name: agent.name.clone(),
                    org_id: organization.id,
                    namespace_id: None,
                    data_version: metrics.data_version.unwrap_or(0),
                    data_applied_seq: metrics.data_applied_seq.unwrap_or(0),
                });
                // Webhook delivery off the heartbeat path — an alert must
                // never slow (or fail) the heartbeat that carries it.
                let db = state.db.clone();
                let org_id = organization.id;
                let org_slug = organization.slug.clone();
                let payload = serde_json::json!({
                    "agent_id": agent_id,
                    "agent_name": agent.name,
                    "data_version": metrics.data_version,
                    "data_applied_seq": metrics.data_applied_seq,
                });
                tokio::spawn(async move {
                    crate::webhook::WebhookDeliveryService::new(db)
                        .deliver_event(
                            org_id,
                            &org_slug,
                            crate::domain::webhook::WebhookEventType::AgentDataStale,
                            payload,
                        )
                        .await;
                });
            }
            Some(false) => {
                tracing::info!(
                    agent_id = %agent_id,
                    agent_name = %agent.name,
                    "agent data replica caught back up"
                );
                let _ = state.event_tx.send(ServerEvent::AgentDataFresh {
                    agent_id,
                    agent_name: agent.name.clone(),
                    org_id: organization.id,
                    namespace_id: None,
                    data_version: metrics.data_version.unwrap_or(0),
                });
            }
            None => {}
        }
    }

    Ok(Json(HeartbeatResponse {
        acknowledged: true,
        server_time: chrono::Utc::now(),
    }))
}

/// Stale-flag edge detection: `Some(true)` = became stale, `Some(false)` =
/// recovered, `None` = no transition. An agent that never reported the
/// flag (or reports None) is treated as fresh — absence of the data plane
/// is not an alert condition.
fn data_stale_transition(previous: Option<bool>, current: Option<bool>) -> Option<bool> {
    match (previous.unwrap_or(false), current.unwrap_or(false)) {
        (false, true) => Some(true),
        (true, false) => Some(false),
        _ => None,
    }
}

/// Agent reports the bundle version it applied. Records the actual per-agent
/// deployment state and, when confirmation gating is on, advances the owning
/// rollout based on real confirmations instead of optimistic completion.
#[utoipa::path(
    post,
    path = "/orgs/{org}/agents/{agent_id}/deployments/report",
    tag = "agents",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("agent_id" = Uuid, Path, description = "Agent ID")
    ),
    request_body = DeploymentReportRequest,
    responses(
        (status = 200, description = "Deployment report acknowledged", body = DeploymentReportResponse)
    ),
    security(("bearer_jwt" = []))
)]
async fn report_deployment(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, agent_id)): Path<(String, Uuid)>,
    Json(request): Json<DeploymentReportRequest>,
) -> ApiResult<Json<DeploymentReportResponse>> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // The agent's JWT must belong to this org.
    if user.org_id != organization.id && !user.has_permission(Scope::Admin) {
        return Err(ApiError::Forbidden(
            "Cannot report deployment for other organizations".to_string(),
        ));
    }

    // Verify the agent exists and belongs to this org.
    let agent_repo = AgentRepository::new(&state.db);
    let agent = agent_repo
        .get_by_id(agent_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Agent not found".to_string()))?;
    if agent.org_id != organization.id {
        return Err(ApiError::NotFound("Agent not found".to_string()));
    }

    let status: crate::domain::agent_deployment::AgentDeploymentStatus =
        request.status.parse().map_err(|_| {
            ApiError::Validation(format!(
                "invalid status '{}' (expected deployed|failed)",
                request.status
            ))
        })?;

    crate::deployment::DeploymentService::new(state.db.clone())
        .record_agent_report(
            agent_id,
            request.bundle_id,
            status,
            request.error.clone(),
            &state,
        )
        .await
        .map_err(|e| ApiError::Internal(format!("record deployment report: {e}")))?;

    Ok(Json(DeploymentReportResponse { acknowledged: true }))
}

#[cfg(test)]
mod tests {
    use super::data_stale_transition;

    #[test]
    fn stale_transitions_fire_only_on_edges() {
        // became stale
        assert_eq!(data_stale_transition(None, Some(true)), Some(true));
        assert_eq!(data_stale_transition(Some(false), Some(true)), Some(true));
        // recovered
        assert_eq!(data_stale_transition(Some(true), Some(false)), Some(false));
        assert_eq!(data_stale_transition(Some(true), None), Some(false));
        // steady state — no event spam
        assert_eq!(data_stale_transition(Some(true), Some(true)), None);
        assert_eq!(data_stale_transition(Some(false), Some(false)), None);
        assert_eq!(data_stale_transition(None, None), None);
        assert_eq!(data_stale_transition(None, Some(false)), None);
    }
}
