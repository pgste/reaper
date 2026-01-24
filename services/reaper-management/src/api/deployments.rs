//! Deployment API endpoints
//!
//! Provides REST endpoints for managing deployment strategies, rollouts,
//! rollbacks, and version pins.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    api::error::ApiError,
    api::orgs::resolve_org,
    auth::middleware::RequireAuth,
    db::repositories::{AgentDeploymentRepository, OrganizationRepository, RollbackConfigRepository},
    deployment::{
        AgentInfo as ServiceAgentInfo, DeploymentService, DryRunResult,
        SkippedAgent as ServiceSkippedAgent, StrategyInfo as ServiceStrategyInfo,
    },
    domain::agent_deployment::{AgentDeployment, DeploymentSummary, RollbackConfig, UpdateRollbackConfig},
    domain::deployment::{
        CreateDeploymentStrategy, CreateVersionPin, DeploymentStrategy, Rollout, RolloutWave,
        StartRollout, StrategyConfig, StrategyType, VersionPin,
    },
    domain::namespace::resolve_namespace,
    state::AppState,
};

/// Build deployment routes
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        // Deployment strategies
        .route("/orgs/{org}/deployment-strategies", get(list_strategies))
        .route("/orgs/{org}/deployment-strategies", post(create_strategy))
        .route(
            "/orgs/{org}/deployment-strategies/{strategy_id}",
            get(get_strategy),
        )
        .route(
            "/orgs/{org}/deployment-strategies/{strategy_id}",
            delete(delete_strategy),
        )
        // Rollouts
        .route("/orgs/{org}/bundles/{bundle_id}/rollout", post(start_rollout))
        .route("/orgs/{org}/rollouts", get(list_rollouts))
        .route("/orgs/{org}/rollouts/{rollout_id}", get(get_rollout))
        .route(
            "/orgs/{org}/rollouts/{rollout_id}/approve",
            post(approve_wave),
        )
        .route(
            "/orgs/{org}/rollouts/{rollout_id}/cancel",
            post(cancel_rollout),
        )
        // Rollback
        .route(
            "/orgs/{org}/namespaces/{namespace}/rollback",
            post(rollback_namespace),
        )
        .route("/orgs/{org}/rollback", post(rollback_org))
        // Version pins
        .route("/orgs/{org}/agents/{agent_id}/pin", post(create_pin))
        .route("/orgs/{org}/agents/{agent_id}/pin", get(get_pin))
        .route("/orgs/{org}/agents/{agent_id}/pin", delete(delete_pin))
        .route("/orgs/{org}/pins", get(list_pins))
        // Deployment status tracking
        .route(
            "/orgs/{org}/rollouts/{rollout_id}/deployments",
            get(get_rollout_deployments),
        )
        .route(
            "/orgs/{org}/rollouts/{rollout_id}/summary",
            get(get_deployment_summary),
        )
        .route(
            "/orgs/{org}/agents/{agent_id}/deployment/acknowledge",
            post(acknowledge_deployment),
        )
        .route(
            "/orgs/{org}/agents/{agent_id}/deployment",
            get(get_agent_deployment),
        )
        // Auto-rollback configuration
        .route("/orgs/{org}/auto-rollback", get(get_rollback_config))
        .route("/orgs/{org}/auto-rollback", post(update_rollback_config))
        .route(
            "/orgs/{org}/namespaces/{namespace}/auto-rollback",
            get(get_namespace_rollback_config),
        )
        .route(
            "/orgs/{org}/namespaces/{namespace}/auto-rollback",
            post(update_namespace_rollback_config),
        )
        .route(
            "/orgs/{org}/rollouts/{rollout_id}/check-rollback",
            post(check_rollback_trigger),
        )
}

// ==================== Request/Response Types ====================

#[derive(Debug, Deserialize)]
pub struct CreateStrategyRequest {
    pub name: String,
    pub namespace_id: Option<Uuid>,
    pub strategy_type: StrategyType,
    pub config: StrategyConfig,
    #[serde(default)]
    pub is_default: bool,
}

#[derive(Debug, Deserialize)]
pub struct StrategiesQuery {
    pub namespace_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct RolloutRequest {
    pub strategy_id: Option<Uuid>,
    pub namespace_id: Option<Uuid>,
    #[serde(default)]
    pub dry_run: bool,
}

/// Dry-run response showing what would happen without executing
#[derive(Debug, Serialize)]
pub struct DryRunResponse {
    /// Agents that would receive the deployment
    pub would_deploy_to: Vec<AgentInfo>,
    /// Agents that would be skipped with reasons
    pub agents_skipped: Vec<SkippedAgent>,
    /// Total count of target agents
    pub target_count: u32,
    /// Validation errors (if any)
    pub validation_errors: Vec<String>,
    /// Strategy that would be used
    pub strategy: Option<StrategyInfo>,
}

impl From<DryRunResult> for DryRunResponse {
    fn from(r: DryRunResult) -> Self {
        Self {
            would_deploy_to: r.would_deploy_to.into_iter().map(Into::into).collect(),
            agents_skipped: r.agents_skipped.into_iter().map(Into::into).collect(),
            target_count: r.target_count,
            validation_errors: r.validation_errors,
            strategy: r.strategy.map(Into::into),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct AgentInfo {
    pub id: Uuid,
    pub name: String,
    pub hostname: Option<String>,
}

impl From<ServiceAgentInfo> for AgentInfo {
    fn from(a: ServiceAgentInfo) -> Self {
        Self {
            id: a.id,
            name: a.name,
            hostname: a.hostname,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct SkippedAgent {
    pub id: Uuid,
    pub name: String,
    pub reason: String,
}

impl From<ServiceSkippedAgent> for SkippedAgent {
    fn from(s: ServiceSkippedAgent) -> Self {
        Self {
            id: s.id,
            name: s.name,
            reason: s.reason,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct StrategyInfo {
    pub id: Uuid,
    pub name: String,
    pub strategy_type: String,
}

impl From<ServiceStrategyInfo> for StrategyInfo {
    fn from(s: ServiceStrategyInfo) -> Self {
        Self {
            id: s.id,
            name: s.name,
            strategy_type: s.strategy_type,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct RolloutsQuery {
    pub namespace_id: Option<Uuid>,
    #[serde(default = "default_limit")]
    pub limit: i32,
}

fn default_limit() -> i32 {
    50
}

#[derive(Debug, Deserialize)]
pub struct CancelRequest {
    pub reason: String,
}

#[derive(Debug, Deserialize)]
pub struct RollbackRequest {
    pub reason: String,
    pub target_bundle_id: Option<Uuid>,
}

/// Response for auto-rollback configuration
#[derive(Debug, Serialize)]
pub struct RollbackConfigResponse {
    pub id: Uuid,
    pub org_id: Uuid,
    pub namespace_id: Option<Uuid>,
    pub is_enabled: bool,
    pub error_rate_threshold: f64,
    pub window_seconds: u32,
    pub min_requests: u32,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<RollbackConfig> for RollbackConfigResponse {
    fn from(c: RollbackConfig) -> Self {
        Self {
            id: c.id,
            org_id: c.org_id,
            namespace_id: c.namespace_id,
            is_enabled: c.is_enabled,
            error_rate_threshold: c.error_rate_threshold,
            window_seconds: c.window_seconds,
            min_requests: c.min_requests,
            created_at: c.created_at,
            updated_at: c.updated_at,
        }
    }
}

/// Result of checking auto-rollback trigger
#[derive(Debug, Serialize)]
pub struct CheckRollbackResponse {
    /// Whether rollback should be triggered
    pub should_rollback: bool,
    /// Current error rate percentage
    pub current_error_rate: f64,
    /// Configured threshold
    pub threshold: f64,
    /// Number of completed deployments in window
    pub completed_count: u32,
    /// Required minimum requests
    pub min_requests: u32,
    /// Reason for the decision
    pub reason: String,
}

#[derive(Debug, Deserialize)]
pub struct CreatePinRequest {
    pub bundle_id: Uuid,
    pub reason: Option<String>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Serialize)]
pub struct StrategyResponse {
    pub id: Uuid,
    pub org_id: Uuid,
    pub namespace_id: Option<Uuid>,
    pub name: String,
    pub strategy_type: StrategyType,
    pub config: StrategyConfig,
    pub is_default: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<DeploymentStrategy> for StrategyResponse {
    fn from(s: DeploymentStrategy) -> Self {
        Self {
            id: s.id,
            org_id: s.org_id,
            namespace_id: s.namespace_id,
            name: s.name,
            strategy_type: s.strategy_type,
            config: s.config,
            is_default: s.is_default,
            created_at: s.created_at,
            updated_at: s.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RolloutResponse {
    pub id: Uuid,
    pub bundle_id: Uuid,
    pub strategy_id: Option<Uuid>,
    pub namespace_id: Option<Uuid>,
    pub status: String,
    pub current_wave: u32,
    pub target_agent_count: u32,
    pub deployed_agent_count: u32,
    pub progress_percent: f64,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub error: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<Rollout> for RolloutResponse {
    fn from(r: Rollout) -> Self {
        let progress = r.progress_percent();
        Self {
            id: r.id,
            bundle_id: r.bundle_id,
            strategy_id: r.strategy_id,
            namespace_id: r.namespace_id,
            status: r.status.to_string(),
            current_wave: r.current_wave,
            target_agent_count: r.target_agent_count,
            deployed_agent_count: r.deployed_agent_count,
            progress_percent: progress,
            started_at: r.started_at,
            completed_at: r.completed_at,
            error: r.error,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RolloutDetailResponse {
    #[serde(flatten)]
    pub rollout: RolloutResponse,
    pub waves: Vec<WaveResponse>,
}

#[derive(Debug, Serialize)]
pub struct WaveResponse {
    pub id: Uuid,
    pub wave_number: u32,
    pub target_agent_count: usize,
    pub deployed_count: u32,
    pub status: String,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl From<RolloutWave> for WaveResponse {
    fn from(w: RolloutWave) -> Self {
        Self {
            id: w.id,
            wave_number: w.wave_number,
            target_agent_count: w.target_agents.len(),
            deployed_count: w.deployed_count,
            status: w.status.to_string(),
            started_at: w.started_at,
            completed_at: w.completed_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct PinResponse {
    pub agent_id: Uuid,
    pub bundle_id: Uuid,
    pub pinned_by: Option<String>,
    pub reason: Option<String>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub is_expired: bool,
}

impl From<VersionPin> for PinResponse {
    fn from(p: VersionPin) -> Self {
        let is_expired = p.is_expired();
        Self {
            agent_id: p.agent_id,
            bundle_id: p.bundle_id,
            pinned_by: p.pinned_by,
            reason: p.reason,
            expires_at: p.expires_at,
            created_at: p.created_at,
            is_expired,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RolloutStartResponse {
    pub rollout: RolloutResponse,
    pub waves: Vec<WaveResponse>,
    pub target_agent_count: usize,
}

// ==================== Strategy Handlers ====================

/// List deployment strategies
async fn list_strategies(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Query(query): Query<StrategiesQuery>,
) -> Result<Json<Vec<StrategyResponse>>, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    // Verify user belongs to this org
    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot access strategies for other organizations".to_string(),
        ));
    }

    let service = DeploymentService::new(state.db.clone());
    let strategies = service
        .list_strategies(organization.id, query.namespace_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(strategies.into_iter().map(Into::into).collect()))
}

/// Create a deployment strategy
async fn create_strategy(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Json(request): Json<CreateStrategyRequest>,
) -> Result<(StatusCode, Json<StrategyResponse>), ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot create strategies for other organizations".to_string(),
        ));
    }

    let input = CreateDeploymentStrategy {
        name: request.name,
        namespace_id: request.namespace_id,
        strategy_type: request.strategy_type,
        config: request.config,
        is_default: request.is_default,
    };

    let service = DeploymentService::new(state.db.clone());
    let strategy = service
        .create_strategy(organization.id, &input)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok((StatusCode::CREATED, Json(strategy.into())))
}

/// Get a deployment strategy
async fn get_strategy(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, strategy_id)): Path<(String, Uuid)>,
) -> Result<Json<StrategyResponse>, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot access strategies for other organizations".to_string(),
        ));
    }

    let service = DeploymentService::new(state.db.clone());
    let strategy = service
        .get_strategy(strategy_id)
        .await
        .map_err(|e| match e {
            crate::deployment::DeploymentError::StrategyNotFound(_) => {
                ApiError::NotFound("Strategy not found".to_string())
            }
            e => ApiError::Internal(e.to_string()),
        })?;

    // Verify strategy belongs to this org
    if strategy.org_id != organization.id {
        return Err(ApiError::NotFound("Strategy not found".to_string()));
    }

    Ok(Json(strategy.into()))
}

/// Delete a deployment strategy
async fn delete_strategy(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, strategy_id)): Path<(String, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot delete strategies for other organizations".to_string(),
        ));
    }

    let service = DeploymentService::new(state.db.clone());

    // Verify strategy belongs to this org
    let strategy = service.get_strategy(strategy_id).await.map_err(|e| {
        match e {
            crate::deployment::DeploymentError::StrategyNotFound(_) => {
                ApiError::NotFound("Strategy not found".to_string())
            }
            e => ApiError::Internal(e.to_string()),
        }
    })?;

    if strategy.org_id != organization.id {
        return Err(ApiError::NotFound("Strategy not found".to_string()));
    }

    service
        .delete_strategy(strategy_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}

// ==================== Rollout Handlers ====================

/// Unified response for rollout - either dry-run or actual start
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum RolloutOrDryRun {
    DryRun(DryRunResponse),
    Rollout(RolloutStartResponse),
}

/// Start a new rollout (or dry-run)
async fn start_rollout(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, bundle_id)): Path<(String, Uuid)>,
    Json(request): Json<RolloutRequest>,
) -> Result<(StatusCode, Json<RolloutOrDryRun>), ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot start rollouts for other organizations".to_string(),
        ));
    }

    let service = DeploymentService::new(state.db.clone());

    // Handle dry-run mode
    if request.dry_run {
        let result = service
            .dry_run_rollout(organization.id, bundle_id, request.strategy_id, request.namespace_id)
            .await
            .map_err(|e| match e {
                crate::deployment::DeploymentError::BundleNotFound(_) => {
                    ApiError::NotFound("Bundle not found".to_string())
                }
                crate::deployment::DeploymentError::BundleNotReady(msg) => {
                    ApiError::BadRequest(msg)
                }
                e => ApiError::Internal(e.to_string()),
            })?;

        return Ok((
            StatusCode::OK,
            Json(RolloutOrDryRun::DryRun(result.into())),
        ));
    }

    // Actual rollout
    let input = StartRollout {
        bundle_id,
        strategy_id: request.strategy_id,
        namespace_id: request.namespace_id,
    };

    let result = service
        .start_rollout(organization.id, &input, &state)
        .await
        .map_err(|e| match e {
            crate::deployment::DeploymentError::BundleNotFound(_) => {
                ApiError::NotFound("Bundle not found".to_string())
            }
            crate::deployment::DeploymentError::BundleNotReady(msg) => {
                ApiError::BadRequest(msg)
            }
            crate::deployment::DeploymentError::ActiveRolloutExists(_) => {
                ApiError::Conflict("Active rollout already exists for this bundle".to_string())
            }
            crate::deployment::DeploymentError::NoAgentsAvailable => {
                ApiError::BadRequest("No active agents available for deployment".to_string())
            }
            e => ApiError::Internal(e.to_string()),
        })?;

    Ok((
        StatusCode::CREATED,
        Json(RolloutOrDryRun::Rollout(RolloutStartResponse {
            rollout: result.rollout.into(),
            waves: result.waves.into_iter().map(Into::into).collect(),
            target_agent_count: result.target_agents.len(),
        })),
    ))
}

/// List rollouts
async fn list_rollouts(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Query(query): Query<RolloutsQuery>,
) -> Result<Json<Vec<RolloutResponse>>, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot access rollouts for other organizations".to_string(),
        ));
    }

    let service = DeploymentService::new(state.db.clone());
    let rollouts = service
        .list_rollouts(organization.id, query.namespace_id, query.limit)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(rollouts.into_iter().map(Into::into).collect()))
}

/// Get rollout details
async fn get_rollout(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, rollout_id)): Path<(String, Uuid)>,
) -> Result<Json<RolloutDetailResponse>, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot access rollouts for other organizations".to_string(),
        ));
    }

    let service = DeploymentService::new(state.db.clone());
    let (rollout, waves) = service
        .get_rollout_with_waves(rollout_id)
        .await
        .map_err(|e| match e {
            crate::deployment::DeploymentError::RolloutNotFound(_) => {
                ApiError::NotFound("Rollout not found".to_string())
            }
            e => ApiError::Internal(e.to_string()),
        })?;

    Ok(Json(RolloutDetailResponse {
        rollout: rollout.into(),
        waves: waves.into_iter().map(Into::into).collect(),
    }))
}

/// Approve next wave
async fn approve_wave(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, rollout_id)): Path<(String, Uuid)>,
) -> Result<Json<RolloutResponse>, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot approve rollouts for other organizations".to_string(),
        ));
    }

    let service = DeploymentService::new(state.db.clone());
    let rollout = service
        .approve_wave(rollout_id, &state)
        .await
        .map_err(|e| match e {
            crate::deployment::DeploymentError::RolloutNotFound(_) => {
                ApiError::NotFound("Rollout not found".to_string())
            }
            crate::deployment::DeploymentError::InvalidState(msg) => {
                ApiError::BadRequest(msg)
            }
            e => ApiError::Internal(e.to_string()),
        })?;

    Ok(Json(rollout.into()))
}

/// Cancel a rollout
async fn cancel_rollout(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, rollout_id)): Path<(String, Uuid)>,
    Json(request): Json<CancelRequest>,
) -> Result<Json<RolloutResponse>, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot cancel rollouts for other organizations".to_string(),
        ));
    }

    let service = DeploymentService::new(state.db.clone());
    let rollout = service
        .cancel_rollout(rollout_id, &request.reason, &state)
        .await
        .map_err(|e| match e {
            crate::deployment::DeploymentError::RolloutNotFound(_) => {
                ApiError::NotFound("Rollout not found".to_string())
            }
            crate::deployment::DeploymentError::InvalidState(msg) => {
                ApiError::BadRequest(msg)
            }
            e => ApiError::Internal(e.to_string()),
        })?;

    Ok(Json(rollout.into()))
}

// ==================== Rollback Handlers ====================

/// Rollback a namespace to previous bundle
async fn rollback_namespace(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, namespace)): Path<(String, String)>,
    Json(request): Json<RollbackRequest>,
) -> Result<(StatusCode, Json<RolloutStartResponse>), ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot rollback for other organizations".to_string(),
        ));
    }

    // Resolve namespace
    let ns_repo = crate::db::repositories::NamespaceRepository::new(&state.db);
    let ns = ns_repo
        .get_by_slug(organization.id, &namespace)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound("Namespace not found".to_string()))?;

    let service = DeploymentService::new(state.db.clone());
    let result = service
        .rollback(
            organization.id,
            Some(ns.id),
            request.target_bundle_id,
            &request.reason,
            &state,
        )
        .await
        .map_err(|e| match e {
            crate::deployment::DeploymentError::BundleNotFound(msg) => {
                ApiError::NotFound(msg)
            }
            e => ApiError::Internal(e.to_string()),
        })?;

    Ok((
        StatusCode::CREATED,
        Json(RolloutStartResponse {
            rollout: result.rollout.into(),
            waves: result.waves.into_iter().map(Into::into).collect(),
            target_agent_count: result.target_agents.len(),
        }),
    ))
}

/// Rollback entire org to previous bundle
async fn rollback_org(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Json(request): Json<RollbackRequest>,
) -> Result<(StatusCode, Json<RolloutStartResponse>), ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot rollback for other organizations".to_string(),
        ));
    }

    let service = DeploymentService::new(state.db.clone());
    let result = service
        .rollback(
            organization.id,
            None,
            request.target_bundle_id,
            &request.reason,
            &state,
        )
        .await
        .map_err(|e| match e {
            crate::deployment::DeploymentError::BundleNotFound(msg) => {
                ApiError::NotFound(msg)
            }
            e => ApiError::Internal(e.to_string()),
        })?;

    Ok((
        StatusCode::CREATED,
        Json(RolloutStartResponse {
            rollout: result.rollout.into(),
            waves: result.waves.into_iter().map(Into::into).collect(),
            target_agent_count: result.target_agents.len(),
        }),
    ))
}

// ==================== Version Pin Handlers ====================

/// Create a version pin
async fn create_pin(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, agent_id)): Path<(String, Uuid)>,
    Json(request): Json<CreatePinRequest>,
) -> Result<(StatusCode, Json<PinResponse>), ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot create pins for other organizations".to_string(),
        ));
    }

    let input = CreateVersionPin {
        bundle_id: request.bundle_id,
        reason: request.reason,
        expires_at: request.expires_at,
    };

    let pinned_by = Some(user.id.as_str());

    let service = DeploymentService::new(state.db.clone());
    let pin = service
        .create_pin(agent_id, &input, pinned_by)
        .await
        .map_err(|e| match e {
            crate::deployment::DeploymentError::AgentNotFound(_) => {
                ApiError::NotFound("Agent not found".to_string())
            }
            crate::deployment::DeploymentError::BundleNotFound(_) => {
                ApiError::NotFound("Bundle not found".to_string())
            }
            e => ApiError::Internal(e.to_string()),
        })?;

    Ok((StatusCode::CREATED, Json(pin.into())))
}

/// Get version pin for an agent
async fn get_pin(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, agent_id)): Path<(String, Uuid)>,
) -> Result<Json<Option<PinResponse>>, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot access pins for other organizations".to_string(),
        ));
    }

    let service = DeploymentService::new(state.db.clone());
    let pin = service
        .get_pin(agent_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(pin.map(Into::into)))
}

/// Delete a version pin
async fn delete_pin(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, agent_id)): Path<(String, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot delete pins for other organizations".to_string(),
        ));
    }

    let service = DeploymentService::new(state.db.clone());
    service.delete_pin(agent_id).await.map_err(|e| match e {
        crate::deployment::DeploymentError::Database(
            crate::db::DatabaseError::NotFound(_),
        ) => ApiError::NotFound("Pin not found".to_string()),
        e => ApiError::Internal(e.to_string()),
    })?;

    Ok(StatusCode::NO_CONTENT)
}

/// List all version pins
async fn list_pins(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
) -> Result<Json<Vec<PinResponse>>, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot access pins for other organizations".to_string(),
        ));
    }

    let service = DeploymentService::new(state.db.clone());
    let pins = service
        .list_pins(organization.id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(pins.into_iter().map(Into::into).collect()))
}

// ==================== Deployment Status Endpoints ====================

/// Response for agent deployment
#[derive(Debug, Serialize)]
pub struct AgentDeploymentResponse {
    pub id: Uuid,
    pub agent_id: Uuid,
    pub bundle_id: Uuid,
    pub rollout_id: Option<Uuid>,
    pub status: String,
    pub error_message: Option<String>,
    pub deployed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub acknowledged_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<AgentDeployment> for AgentDeploymentResponse {
    fn from(d: AgentDeployment) -> Self {
        Self {
            id: d.id,
            agent_id: d.agent_id,
            bundle_id: d.bundle_id,
            rollout_id: d.rollout_id,
            status: d.status.to_string(),
            error_message: d.error_message,
            deployed_at: d.deployed_at,
            acknowledged_at: d.acknowledged_at,
            created_at: d.created_at,
        }
    }
}

/// Response for deployment summary
#[derive(Debug, Serialize)]
pub struct DeploymentSummaryResponse {
    pub total_agents: u32,
    pub pending: u32,
    pub deploying: u32,
    pub deployed: u32,
    pub failed: u32,
    pub acknowledged: u32,
    pub success_rate: f64,
    pub failure_rate: f64,
    pub is_complete: bool,
}

impl From<DeploymentSummary> for DeploymentSummaryResponse {
    fn from(s: DeploymentSummary) -> Self {
        Self {
            total_agents: s.total_agents,
            pending: s.pending,
            deploying: s.deploying,
            deployed: s.deployed,
            failed: s.failed,
            acknowledged: s.acknowledged,
            success_rate: s.success_rate(),
            failure_rate: s.failure_rate(),
            is_complete: s.is_complete(),
        }
    }
}

/// Get per-agent deployment status for a rollout
async fn get_rollout_deployments(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, rollout_id)): Path<(String, Uuid)>,
) -> Result<Json<Vec<AgentDeploymentResponse>>, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot access deployments for other organizations".to_string(),
        ));
    }

    let repo = AgentDeploymentRepository::new(&state.db);
    let deployments = repo
        .get_by_rollout(rollout_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(deployments.into_iter().map(Into::into).collect()))
}

/// Get deployment summary for a rollout
async fn get_deployment_summary(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, rollout_id)): Path<(String, Uuid)>,
) -> Result<Json<DeploymentSummaryResponse>, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot access deployments for other organizations".to_string(),
        ));
    }

    let repo = AgentDeploymentRepository::new(&state.db);
    let summary = repo
        .get_summary(rollout_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(summary.into()))
}

/// Get latest deployment for an agent
async fn get_agent_deployment(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, agent_id)): Path<(String, Uuid)>,
) -> Result<Json<Option<AgentDeploymentResponse>>, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot access deployments for other organizations".to_string(),
        ));
    }

    let repo = AgentDeploymentRepository::new(&state.db);
    let deployment = repo
        .get_latest_for_agent(agent_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(deployment.map(Into::into)))
}

/// Acknowledge a deployment (agent confirms receipt)
async fn acknowledge_deployment(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, agent_id)): Path<(String, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot acknowledge deployments for other organizations".to_string(),
        ));
    }

    let repo = AgentDeploymentRepository::new(&state.db);

    // Get the latest deployment for this agent
    let deployment = repo
        .get_latest_for_agent(agent_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound("No deployment found for agent".to_string()))?;

    repo.acknowledge(deployment.id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}

// ==================== Auto-Rollback Configuration Endpoints ====================

/// Get org-level auto-rollback configuration
async fn get_rollback_config(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
) -> Result<Json<RollbackConfigResponse>, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot access rollback config for other organizations".to_string(),
        ));
    }

    let repo = RollbackConfigRepository::new(&state.db);
    let config = repo
        .get(organization.id, None)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .unwrap_or_else(|| RollbackConfig::new(organization.id, None));

    Ok(Json(config.into()))
}

/// Update org-level auto-rollback configuration
async fn update_rollback_config(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path(org): Path<String>,
    Json(request): Json<UpdateRollbackConfig>,
) -> Result<Json<RollbackConfigResponse>, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot update rollback config for other organizations".to_string(),
        ));
    }

    let repo = RollbackConfigRepository::new(&state.db);

    // Get existing or create new
    let mut config = repo
        .get(organization.id, None)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .unwrap_or_else(|| RollbackConfig::new(organization.id, None));

    // Apply updates
    if let Some(enabled) = request.is_enabled {
        config.is_enabled = enabled;
    }
    if let Some(threshold) = request.error_rate_threshold {
        if !(0.0..=100.0).contains(&threshold) {
            return Err(ApiError::BadRequest(
                "Error rate threshold must be between 0 and 100".to_string(),
            ));
        }
        config.error_rate_threshold = threshold;
    }
    if let Some(window) = request.window_seconds {
        if window == 0 {
            return Err(ApiError::BadRequest(
                "Window seconds must be greater than 0".to_string(),
            ));
        }
        config.window_seconds = window;
    }
    if let Some(min_req) = request.min_requests {
        config.min_requests = min_req;
    }
    config.updated_at = chrono::Utc::now();

    repo.upsert(&config)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(config.into()))
}

/// Get namespace-level auto-rollback configuration
async fn get_namespace_rollback_config(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, namespace)): Path<(String, String)>,
) -> Result<Json<RollbackConfigResponse>, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot access rollback config for other organizations".to_string(),
        ));
    }

    let namespace_id = resolve_namespace(&state.db, organization.id, &namespace)
        .await
        .map_err(|e| ApiError::NotFound(e.to_string()))?;

    let repo = RollbackConfigRepository::new(&state.db);
    let config = repo
        .get(organization.id, Some(namespace_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .unwrap_or_else(|| RollbackConfig::new(organization.id, Some(namespace_id)));

    Ok(Json(config.into()))
}

/// Update namespace-level auto-rollback configuration
async fn update_namespace_rollback_config(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, namespace)): Path<(String, String)>,
    Json(request): Json<UpdateRollbackConfig>,
) -> Result<Json<RollbackConfigResponse>, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot update rollback config for other organizations".to_string(),
        ));
    }

    let namespace_id = resolve_namespace(&state.db, organization.id, &namespace)
        .await
        .map_err(|e| ApiError::NotFound(e.to_string()))?;

    let repo = RollbackConfigRepository::new(&state.db);

    // Get existing or create new
    let mut config = repo
        .get(organization.id, Some(namespace_id))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .unwrap_or_else(|| RollbackConfig::new(organization.id, Some(namespace_id)));

    // Apply updates
    if let Some(enabled) = request.is_enabled {
        config.is_enabled = enabled;
    }
    if let Some(threshold) = request.error_rate_threshold {
        if !(0.0..=100.0).contains(&threshold) {
            return Err(ApiError::BadRequest(
                "Error rate threshold must be between 0 and 100".to_string(),
            ));
        }
        config.error_rate_threshold = threshold;
    }
    if let Some(window) = request.window_seconds {
        if window == 0 {
            return Err(ApiError::BadRequest(
                "Window seconds must be greater than 0".to_string(),
            ));
        }
        config.window_seconds = window;
    }
    if let Some(min_req) = request.min_requests {
        config.min_requests = min_req;
    }
    config.updated_at = chrono::Utc::now();

    repo.upsert(&config)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(config.into()))
}

/// Check if auto-rollback should be triggered for a rollout
async fn check_rollback_trigger(
    State(state): State<Arc<AppState>>,
    RequireAuth(user): RequireAuth,
    Path((org, rollout_id)): Path<(String, Uuid)>,
) -> Result<Json<CheckRollbackResponse>, ApiError> {
    let org_repo = OrganizationRepository::new(&state.db);
    let organization = resolve_org(&org_repo, &org).await?;

    if user.org_id != organization.id
        && !user.has_any_permission(&[crate::auth::scopes::Scope::Admin])
    {
        return Err(ApiError::Forbidden(
            "Cannot check rollback for other organizations".to_string(),
        ));
    }

    // Get the rollout
    let service = DeploymentService::new(state.db.clone());
    let rollout = service
        .get_rollout(rollout_id)
        .await
        .map_err(|e| match e {
            crate::deployment::DeploymentError::RolloutNotFound(_) => {
                ApiError::NotFound("Rollout not found".to_string())
            }
            e => ApiError::Internal(e.to_string()),
        })?;

    // Get rollback config (namespace-specific or org-level fallback)
    let rollback_repo = RollbackConfigRepository::new(&state.db);
    let config = rollback_repo
        .get(organization.id, rollout.namespace_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .or(rollback_repo
            .get(organization.id, None)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?)
        .unwrap_or_else(|| RollbackConfig::new(organization.id, rollout.namespace_id));

    if !config.is_enabled {
        return Ok(Json(CheckRollbackResponse {
            should_rollback: false,
            current_error_rate: 0.0,
            threshold: config.error_rate_threshold,
            completed_count: 0,
            min_requests: config.min_requests,
            reason: "Auto-rollback is disabled".to_string(),
        }));
    }

    // Get deployment summary
    let deployment_repo = AgentDeploymentRepository::new(&state.db);
    let summary = deployment_repo
        .get_summary(rollout_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let completed_count = summary.deployed + summary.failed;

    // Check minimum requests threshold
    if completed_count < config.min_requests {
        return Ok(Json(CheckRollbackResponse {
            should_rollback: false,
            current_error_rate: summary.failure_rate(),
            threshold: config.error_rate_threshold,
            completed_count,
            min_requests: config.min_requests,
            reason: format!(
                "Minimum requests not met ({} < {})",
                completed_count, config.min_requests
            ),
        }));
    }

    let error_rate = summary.failure_rate();
    let should_rollback = error_rate > config.error_rate_threshold;

    let reason = if should_rollback {
        format!(
            "Error rate {:.2}% exceeds threshold {:.2}%",
            error_rate, config.error_rate_threshold
        )
    } else {
        format!(
            "Error rate {:.2}% within threshold {:.2}%",
            error_rate, config.error_rate_threshold
        )
    };

    Ok(Json(CheckRollbackResponse {
        should_rollback,
        current_error_rate: error_rate,
        threshold: config.error_rate_threshold,
        completed_count,
        min_requests: config.min_requests,
        reason,
    }))
}
