//! Landscape API endpoints
//!
//! Provides REST endpoints for fleet visibility, including landscape views,
//! bundle distribution, and aggregated metrics.

use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    api::error::ApiError, api::orgs::resolve_org, auth::middleware::RequireAuth,
    db::repositories::OrganizationRepository, landscape::service::LandscapeService,
    state::AppState,
};

/// Build landscape routes
pub fn routes() -> OpenApiRouter<Arc<AppState>> {
    OpenApiRouter::new()
        // Organization landscape
        .routes(routes!(get_landscape))
        // Namespace landscape
        .routes(routes!(get_namespace_landscape))
        // Organization metrics
        .routes(routes!(get_org_metrics))
        // Agent metrics
        .routes(routes!(get_agent_metrics))
        // Dashboard (combined view)
        .routes(routes!(get_dashboard))
}

// ==================== Request/Response Types ====================

#[derive(Debug, Deserialize)]
pub struct LandscapeQuery {
    /// Include inactive agents
    #[serde(default)]
    pub include_inactive: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LandscapeResponse {
    pub org_id: Uuid,
    pub namespace_id: Option<Uuid>,
    pub summary: SummaryResponse,
    pub agents: Vec<AgentEntryResponse>,
    pub bundle_distribution: Vec<BundleDistributionResponse>,
    pub generated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SummaryResponse {
    pub total_agents: usize,
    pub healthy: usize,
    pub unhealthy: usize,
    pub pending_update: usize,
    pub pinned: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AgentEntryResponse {
    pub id: Uuid,
    pub name: String,
    pub hostname: Option<String>,
    pub status: String,
    pub labels: serde_json::Value,
    pub last_heartbeat_at: Option<chrono::DateTime<chrono::Utc>>,
    pub is_healthy: bool,
    pub current_bundle_id: Option<Uuid>,
    pub current_bundle_version: Option<String>,
    pub metrics: Option<AgentMetricsResponse>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AgentMetricsResponse {
    pub requests_total: u64,
    pub requests_per_second: f64,
    pub p99_latency_us: f64,
    pub memory_mb: f64,
    pub allow_rate: f64,
    pub uptime_seconds: u64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BundleDistributionResponse {
    pub bundle_id: Uuid,
    pub bundle_name: String,
    pub version: Option<String>,
    pub agent_count: usize,
    pub percentage: f64,
    pub is_promoted: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OrgMetricsResponse {
    pub org_id: Uuid,
    pub total_agents: usize,
    pub healthy_agents: usize,
    pub total_requests: u64,
    pub avg_requests_per_second: f64,
    pub avg_latency_p99_us: f64,
    pub total_allow_decisions: u64,
    pub total_deny_decisions: u64,
    pub allow_rate_percent: f64,
    pub total_memory_mb: f64,
    pub period_start: chrono::DateTime<chrono::Utc>,
    pub period_end: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SingleAgentMetricsResponse {
    pub agent_id: Uuid,
    pub metrics: Option<DetailedAgentMetrics>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DetailedAgentMetrics {
    pub requests_total: u64,
    pub requests_per_second: f64,
    pub avg_latency_us: f64,
    pub p50_latency_us: f64,
    pub p99_latency_us: f64,
    pub memory_bytes: u64,
    pub decisions_allow: u64,
    pub decisions_deny: u64,
    pub uptime_seconds: u64,
    pub current_bundle_id: Option<Uuid>,
    pub current_bundle_version: Option<String>,
    /// Data-plane replica state (two-way visibility): which datastore
    /// version this reaper serves, its change-stream position, staleness.
    pub data_version: Option<i64>,
    pub data_applied_seq: Option<i64>,
    pub data_stale: Option<bool>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DashboardResponse {
    pub summary: SummaryResponse,
    pub metrics: OrgMetricsResponse,
    pub recent_rollouts: Vec<RecentRolloutResponse>,
    pub alerts: Vec<AlertResponse>,
    pub generated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RecentRolloutResponse {
    pub id: Uuid,
    pub bundle_name: String,
    pub status: String,
    pub progress_percent: f64,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AlertResponse {
    pub severity: String,
    pub message: String,
    pub agent_id: Option<Uuid>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

// ==================== Handlers ====================

/// Get landscape view for an organization
#[utoipa::path(
    get,
    path = "/orgs/{org}/landscape",
    tag = "landscape",
    params(
        ("org" = String, Path, description = "Organization ID or slug")
    ),
    responses((status = 200, description = "Organization landscape view", body = LandscapeResponse)),
    security(("bearer_jwt" = []))
)]
async fn get_landscape(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Query(_query): Query<LandscapeQuery>,
) -> Result<Json<LandscapeResponse>, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot access landscape for other organizations".to_string(),
        ));
    }

    let service = LandscapeService::new(state.db.clone());
    let landscape = service
        .get_landscape(organization.id, None)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(convert_landscape(landscape)))
}

/// Get landscape view for a namespace
#[utoipa::path(
    get,
    path = "/orgs/{org}/namespaces/{namespace}/landscape",
    tag = "landscape",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("namespace" = String, Path, description = "Namespace slug")
    ),
    responses((status = 200, description = "Namespace landscape view", body = LandscapeResponse)),
    security(("bearer_jwt" = []))
)]
async fn get_namespace_landscape(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, namespace)): Path<(String, String)>,
    Query(_query): Query<LandscapeQuery>,
) -> Result<Json<LandscapeResponse>, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot access landscape for other organizations".to_string(),
        ));
    }

    // Resolve namespace
    let ns_repo = crate::db::repositories::NamespaceRepository::new(&state.db);
    let ns = ns_repo
        .get_by_slug(organization.id, &namespace)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound("Namespace not found".to_string()))?;

    let service = LandscapeService::new(state.db.clone());
    let landscape = service
        .get_landscape(organization.id, Some(ns.id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(convert_landscape(landscape)))
}

/// Get aggregated metrics for an organization
#[utoipa::path(
    get,
    path = "/orgs/{org}/metrics",
    tag = "landscape",
    params(
        ("org" = String, Path, description = "Organization ID or slug")
    ),
    responses((status = 200, description = "Aggregated organization metrics", body = OrgMetricsResponse)),
    security(("bearer_jwt" = []))
)]
async fn get_org_metrics(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
) -> Result<Json<OrgMetricsResponse>, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot access metrics for other organizations".to_string(),
        ));
    }

    let service = LandscapeService::new(state.db.clone());
    let metrics = service
        .get_org_metrics(organization.id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(OrgMetricsResponse {
        org_id: metrics.org_id,
        total_agents: metrics.total_agents,
        healthy_agents: metrics.healthy_agents,
        total_requests: metrics.total_requests,
        avg_requests_per_second: metrics.avg_requests_per_second,
        avg_latency_p99_us: metrics.avg_latency_p99_us,
        total_allow_decisions: metrics.total_allow_decisions,
        total_deny_decisions: metrics.total_deny_decisions,
        allow_rate_percent: metrics.allow_rate_percent,
        total_memory_mb: metrics.total_memory_mb,
        period_start: metrics.period_start,
        period_end: metrics.period_end,
    }))
}

/// Get metrics for a specific agent
#[utoipa::path(
    get,
    path = "/orgs/{org}/agents/{agent_id}/metrics",
    tag = "landscape",
    params(
        ("org" = String, Path, description = "Organization ID or slug"),
        ("agent_id" = Uuid, Path, description = "Agent ID")
    ),
    responses((status = 200, description = "Metrics for a specific agent", body = SingleAgentMetricsResponse)),
    security(("bearer_jwt" = []))
)]
async fn get_agent_metrics(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, agent_id)): Path<(String, Uuid)>,
) -> Result<Json<SingleAgentMetricsResponse>, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot access metrics for other organizations".to_string(),
        ));
    }

    // Verify agent belongs to this org
    let agent_repo = crate::db::repositories::AgentRepository::new(&state.db);
    let agent = agent_repo
        .get_by_id(agent_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound("Agent not found".to_string()))?;

    if agent.org_id != organization.id {
        return Err(ApiError::NotFound("Agent not found".to_string()));
    }

    let service = LandscapeService::new(state.db.clone());
    let metrics = service
        .get_agent_metrics(agent_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(SingleAgentMetricsResponse {
        agent_id,
        metrics: metrics.map(|m| DetailedAgentMetrics {
            requests_total: m.requests_total,
            requests_per_second: m.requests_per_second,
            avg_latency_us: m.avg_latency_us,
            p50_latency_us: m.p50_latency_us,
            p99_latency_us: m.p99_latency_us,
            memory_bytes: m.memory_bytes,
            decisions_allow: m.decisions_allow,
            decisions_deny: m.decisions_deny,
            uptime_seconds: m.uptime_seconds,
            current_bundle_id: m.current_bundle_id,
            current_bundle_version: m.current_bundle_version,
            data_version: m.data_version,
            data_applied_seq: m.data_applied_seq,
            data_stale: m.data_stale,
        }),
    }))
}

/// Get dashboard view (combined summary, metrics, and alerts)
#[utoipa::path(
    get,
    path = "/orgs/{org}/dashboard",
    tag = "landscape",
    params(
        ("org" = String, Path, description = "Organization ID or slug")
    ),
    responses((status = 200, description = "Combined dashboard view", body = DashboardResponse)),
    security(("bearer_jwt" = []))
)]
async fn get_dashboard(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
) -> Result<Json<DashboardResponse>, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot access dashboard for other organizations".to_string(),
        ));
    }

    let service = LandscapeService::new(state.db.clone());

    // Get landscape for summary
    let landscape = service
        .get_landscape(organization.id, None)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    // Get metrics
    let metrics = service
        .get_org_metrics(organization.id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    // Get recent rollouts
    let deploy_repo = crate::db::repositories::DeploymentRepository::new(&state.db);
    let rollouts = deploy_repo
        .list_rollouts(organization.id, None, 5)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let bundle_repo = crate::db::repositories::BundleRepository::new(&state.db);

    let mut recent_rollouts = Vec::new();
    for rollout in rollouts {
        if let Ok(Some(bundle)) = bundle_repo.get_by_id(rollout.bundle_id).await {
            recent_rollouts.push(RecentRolloutResponse {
                id: rollout.id,
                bundle_name: bundle.name,
                status: rollout.status.to_string(),
                progress_percent: rollout.progress_percent(),
                started_at: rollout.started_at,
            });
        }
    }

    // Generate alerts based on current state
    let mut alerts = Vec::new();

    // Alert for unhealthy agents
    if landscape.summary.unhealthy > 0 {
        alerts.push(AlertResponse {
            severity: "warning".to_string(),
            message: format!(
                "{} agent(s) are unhealthy (no recent heartbeat)",
                landscape.summary.unhealthy
            ),
            agent_id: None,
            timestamp: chrono::Utc::now(),
        });
    }

    // Alert for pending updates
    if landscape.summary.pending_update > 0 {
        alerts.push(AlertResponse {
            severity: "info".to_string(),
            message: format!(
                "{} agent(s) are pending policy update",
                landscape.summary.pending_update
            ),
            agent_id: None,
            timestamp: chrono::Utc::now(),
        });
    }

    Ok(Json(DashboardResponse {
        summary: SummaryResponse {
            total_agents: landscape.summary.total_agents,
            healthy: landscape.summary.healthy,
            unhealthy: landscape.summary.unhealthy,
            pending_update: landscape.summary.pending_update,
            pinned: landscape.summary.pinned,
        },
        metrics: OrgMetricsResponse {
            org_id: metrics.org_id,
            total_agents: metrics.total_agents,
            healthy_agents: metrics.healthy_agents,
            total_requests: metrics.total_requests,
            avg_requests_per_second: metrics.avg_requests_per_second,
            avg_latency_p99_us: metrics.avg_latency_p99_us,
            total_allow_decisions: metrics.total_allow_decisions,
            total_deny_decisions: metrics.total_deny_decisions,
            allow_rate_percent: metrics.allow_rate_percent,
            total_memory_mb: metrics.total_memory_mb,
            period_start: metrics.period_start,
            period_end: metrics.period_end,
        },
        recent_rollouts,
        alerts,
        generated_at: chrono::Utc::now(),
    }))
}

// ==================== Helpers ====================

fn convert_landscape(landscape: crate::landscape::service::LandscapeView) -> LandscapeResponse {
    LandscapeResponse {
        org_id: landscape.org_id,
        namespace_id: landscape.namespace_id,
        summary: SummaryResponse {
            total_agents: landscape.summary.total_agents,
            healthy: landscape.summary.healthy,
            unhealthy: landscape.summary.unhealthy,
            pending_update: landscape.summary.pending_update,
            pinned: landscape.summary.pinned,
        },
        agents: landscape
            .agents
            .into_iter()
            .map(|a| AgentEntryResponse {
                id: a.id,
                name: a.name,
                hostname: a.hostname,
                status: a.status.to_string(),
                labels: a.labels,
                last_heartbeat_at: a.last_heartbeat_at,
                is_healthy: a.is_healthy,
                current_bundle_id: a.current_bundle_id,
                current_bundle_version: a.current_bundle_version,
                metrics: a.metrics.map(|m| AgentMetricsResponse {
                    requests_total: m.requests_total,
                    requests_per_second: m.requests_per_second,
                    p99_latency_us: m.p99_latency_us,
                    memory_mb: m.memory_mb,
                    allow_rate: m.allow_rate,
                    uptime_seconds: m.uptime_seconds,
                }),
            })
            .collect(),
        bundle_distribution: landscape
            .bundle_distribution
            .into_iter()
            .map(|b| BundleDistributionResponse {
                bundle_id: b.bundle_id,
                bundle_name: b.bundle_name,
                version: b.version,
                agent_count: b.agent_count,
                percentage: b.percentage,
                is_promoted: b.is_promoted,
            })
            .collect(),
        generated_at: landscape.generated_at,
    }
}
