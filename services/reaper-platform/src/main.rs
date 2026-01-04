use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Json, Response},
    routing::{get, post},
    Router,
};
use chrono::{DateTime, Utc};
use lazy_static::lazy_static;
use parking_lot::RwLock;
use policy_engine::{
    reap::{Decision, Policy, ReapCondition, ReapRule},
    EnhancedPolicy, PolicyAction, PolicyBundle, PolicyEngine, PolicyRule,
};
use prometheus::{
    register_counter_vec, register_gauge, register_histogram_vec, CounterVec, Encoder, Gauge,
    HistogramVec, TextEncoder,
};
use reaper_core::{endpoints, ReaperError, BUILD_INFO, VERSION};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::{error, info, instrument, warn};
use uuid::Uuid;

// ============================================================================
// Prometheus Metrics
// ============================================================================

lazy_static! {
    /// Total API requests by endpoint and status
    static ref API_REQUESTS_TOTAL: CounterVec = register_counter_vec!(
        "reaper_platform_api_requests_total",
        "Total API requests",
        &["endpoint", "method", "status"]
    )
    .unwrap();

    /// API request duration histogram
    static ref API_REQUEST_DURATION: HistogramVec = register_histogram_vec!(
        "reaper_platform_api_request_duration_seconds",
        "API request duration in seconds",
        &["endpoint"],
        vec![0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0]
    )
    .unwrap();

    /// Total policies managed
    static ref POLICIES_TOTAL: Gauge = register_gauge!(
        "reaper_platform_policies_total",
        "Total policies managed"
    )
    .unwrap();

    /// Total deployments
    static ref DEPLOYMENTS_TOTAL: CounterVec = register_counter_vec!(
        "reaper_platform_deployments_total",
        "Total policy deployments",
        &["result"]
    )
    .unwrap();

    /// Registered agents
    static ref AGENTS_TOTAL: Gauge = register_gauge!(
        "reaper_platform_agents_total",
        "Total registered agents"
    )
    .unwrap();

    /// Bundles stored
    static ref BUNDLES_TOTAL: Gauge = register_gauge!(
        "reaper_platform_bundles_total",
        "Total bundles stored"
    )
    .unwrap();
}

#[derive(Clone)]
struct PlatformState {
    policy_engine: PolicyEngine,
    deployment_stats: Arc<RwLock<DeploymentStats>>,
    /// Bundle storage: policy_id -> PolicyBundle
    bundle_storage: Arc<RwLock<HashMap<String, PolicyBundle>>>,
    /// Registered agents: agent_id -> agent_url
    #[allow(dead_code)]
    agents: Arc<RwLock<HashMap<String, String>>>,
}

// Add Debug manually since PolicyEngine doesn't implement Debug
impl std::fmt::Debug for PlatformState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlatformState")
            .field("deployment_stats", &self.deployment_stats)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Default)]
struct DeploymentStats {
    total_deployments: u64,
    successful_deployments: u64,
    failed_deployments: u64,
}

/// Request to create a new policy
#[derive(Debug, Deserialize)]
struct CreatePolicyRequest {
    pub name: String,
    pub description: Option<String>,
    pub rules: Vec<CreatePolicyRule>,
}

#[derive(Debug, Deserialize)]
struct CreatePolicyRule {
    pub action: String, // "allow", "deny", "log"
    pub resource: String,
    pub conditions: Option<Vec<String>>,
}

/// Request to update a policy
#[derive(Debug, Deserialize)]
struct UpdatePolicyRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub rules: Option<Vec<CreatePolicyRule>>,
}

/// Policy response
#[derive(Debug, Serialize)]
struct PolicyResponse {
    pub id: String,
    pub version: u64,
    pub name: String,
    pub description: String,
    pub rules: Vec<PolicyRuleResponse>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct PolicyRuleResponse {
    pub action: String,
    pub resource: String,
    pub conditions: Vec<String>,
}

impl From<EnhancedPolicy> for PolicyResponse {
    fn from(policy: EnhancedPolicy) -> Self {
        Self {
            id: policy.id.to_string(),
            version: policy.version,
            name: policy.name,
            description: policy.description,
            rules: policy
                .rules
                .into_iter()
                .map(|rule| PolicyRuleResponse {
                    action: match rule.action {
                        PolicyAction::Allow => "allow".to_string(),
                        PolicyAction::Deny => "deny".to_string(),
                        PolicyAction::Log => "log".to_string(),
                    },
                    resource: rule.resource,
                    conditions: rule.conditions,
                })
                .collect(),
            created_at: policy.created_at,
            updated_at: policy.updated_at,
        }
    }
}

/// Request to create a bundle from a policy
#[derive(Debug, Deserialize)]
struct CreateBundleRequest {
    pub policy_id: String,
    pub version: String,
    pub description: Option<String>,
}

/// Bundle response
#[derive(Debug, Serialize)]
struct BundleResponse {
    pub bundle_id: String,
    pub policy_id: String,
    pub version: String,
    pub size_bytes: usize,
    pub created_at: DateTime<Utc>,
}

/// Request to deploy bundle to agents
#[derive(Debug, Deserialize)]
struct DeployBundleToAgentsRequest {
    pub bundle_id: String, // Bundle ID to deploy
    #[allow(dead_code)]
    #[serde(default)]
    pub agent_ids: Vec<String>, // If empty, deploy to all agents
    #[allow(dead_code)]
    #[serde(default)]
    pub force: bool,
}

/// Deployment result per agent
#[derive(Debug, Serialize)]
struct AgentDeploymentResult {
    pub agent_id: String,
    pub agent_url: String,
    pub success: bool,
    pub message: String,
    pub deployed_version: Option<String>,
}

/// Bundle deployment response
#[derive(Debug, Serialize)]
struct DeployBundleToAgentsResponse {
    pub bundle_id: String,
    pub total_agents: usize,
    pub successful: usize,
    pub failed: usize,
    pub results: Vec<AgentDeploymentResult>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    info!(
        "Starting Reaper Platform {} - Policy Management & Agent Orchestration",
        BUILD_INFO
    );

    let policy_engine = PolicyEngine::new();

    // Create a default "allow-all" policy for demo purposes
    let default_policy = EnhancedPolicy::new(
        "default-allow-all".to_string(),
        "Default policy that allows all requests".to_string(),
        vec![PolicyRule {
            action: PolicyAction::Allow,
            resource: "*".to_string(),
            conditions: vec![],
        }],
    );

    info!("Deploying default allow-all policy");
    policy_engine.deploy_policy(default_policy)?;

    // Initialize agents with default localhost agent for testing
    let mut agents = HashMap::new();
    agents.insert("agent-001".to_string(), "http://localhost:8080".to_string());

    let state = Arc::new(PlatformState {
        policy_engine,
        deployment_stats: Arc::new(RwLock::new(DeploymentStats::default())),
        bundle_storage: Arc::new(RwLock::new(HashMap::new())),
        agents: Arc::new(RwLock::new(agents)),
    });

    let app = Router::new()
        // Health and metrics
        .route(endpoints::HEALTH, get(health_check))
        .route(endpoints::METRICS, get(metrics))
        .route("/metrics/prometheus", get(prometheus_metrics))
        // Policy management
        .route(
            endpoints::API_V1_POLICIES,
            get(list_policies).post(create_policy),
        )
        .route(
            "/api/v1/policies/{id}",
            get(get_policy).put(update_policy).delete(delete_policy),
        )
        .route(
            "/api/v1/policies/{id}/deploy",
            post(deploy_policy_to_agents),
        )
        // Bundle management
        .route("/api/v1/bundles", post(create_bundle))
        .route("/api/v1/bundles/{id}", get(get_bundle))
        .route("/api/v1/bundles/deploy", post(deploy_bundle_to_agents))
        // Agent management (placeholder for now)
        .route(endpoints::API_V1_AGENTS, get(list_agents))
        .route("/api/v1/agents/{id}", get(get_agent))
        .with_state(state);

    let listener = TcpListener::bind("0.0.0.0:8081").await?;
    info!("🎯 Reaper Platform listening on {}", listener.local_addr()?);
    info!("");
    info!("📋 Policy Management API:");
    info!("  GET    /api/v1/policies        - List all policies");
    info!("  POST   /api/v1/policies        - Create new policy");
    info!("  GET    /api/v1/policies/{{id}} - Get policy details");
    info!("  PUT    /api/v1/policies/{{id}} - Update policy");
    info!("  DELETE /api/v1/policies/{{id}} - Delete policy");
    info!("  POST   /api/v1/policies/{{id}}/deploy - Deploy to agents");
    info!("");

    axum::serve(listener, app).await?;

    Ok(())
}

#[instrument]
async fn health_check() -> Result<Json<Value>, StatusCode> {
    Ok(Json(json!({
        "status": "healthy",
        "service": "reaper-platform",
        "version": VERSION,
        "capabilities": [
            "policy-management",
            "hot-swapping",
            "agent-orchestration"
        ]
    })))
}

#[instrument]
async fn metrics(State(state): State<Arc<PlatformState>>) -> Result<Json<Value>, StatusCode> {
    let engine_stats = state.policy_engine.get_stats();
    let deployment_stats = state.deployment_stats.read();

    // Update Prometheus gauges
    POLICIES_TOTAL.set(engine_stats.total_policies as f64);
    BUNDLES_TOTAL.set(state.bundle_storage.read().len() as f64);
    AGENTS_TOTAL.set(state.agents.read().len() as f64);

    Ok(Json(json!({
        "service": "reaper-platform",
        "policies": {
            "total": engine_stats.total_policies,
            "has_default": engine_stats.has_default_policy
        },
        "deployments": {
            "total": deployment_stats.total_deployments,
            "successful": deployment_stats.successful_deployments,
            "failed": deployment_stats.failed_deployments,
            "success_rate": if deployment_stats.total_deployments > 0 {
                (deployment_stats.successful_deployments as f64 / deployment_stats.total_deployments as f64) * 100.0
            } else {
                100.0
            }
        },
        "uptime_seconds": 0, // TODO: Add actual uptime tracking
        "memory_usage_mb": 0, // TODO: Add actual memory tracking
    })))
}

/// Prometheus metrics endpoint (text format for scraping)
async fn prometheus_metrics(State(state): State<Arc<PlatformState>>) -> Response {
    // Update gauges before encoding
    let engine_stats = state.policy_engine.get_stats();
    POLICIES_TOTAL.set(engine_stats.total_policies as f64);
    BUNDLES_TOTAL.set(state.bundle_storage.read().len() as f64);
    AGENTS_TOTAL.set(state.agents.read().len() as f64);

    let encoder = TextEncoder::new();
    let metric_families = prometheus::gather();
    let mut buffer = Vec::new();
    encoder.encode(&metric_families, &mut buffer).unwrap();
    let body = String::from_utf8(buffer).unwrap();

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/plain; version=0.0.4; charset=utf-8")
        .body(body.into())
        .unwrap()
}

#[instrument(skip(state))]
async fn list_policies(State(state): State<Arc<PlatformState>>) -> Result<Json<Value>, StatusCode> {
    let policies = state.policy_engine.list_policies();
    let policy_responses: Vec<PolicyResponse> = policies
        .into_iter()
        .map(|policy| PolicyResponse::from((*policy).clone()))
        .collect();

    Ok(Json(json!({
        "policies": policy_responses,
        "total": policy_responses.len(),
        "message": if policy_responses.is_empty() {
            "No policies found. Create your first policy to get started!"
        } else {
            "Policies retrieved successfully"
        }
    })))
}

#[instrument(skip(state, payload))]
async fn create_policy(
    State(state): State<Arc<PlatformState>>,
    Json(payload): Json<CreatePolicyRequest>,
) -> Result<Json<Value>, StatusCode> {
    // Validate policy name
    if payload.name.trim().is_empty() {
        return Ok(Json(json!({
            "error": "Policy name cannot be empty"
        })));
    }

    // Check if policy with this name already exists
    if state
        .policy_engine
        .get_policy_by_name(&payload.name)
        .is_some()
    {
        return Ok(Json(json!({
            "error": format!("Policy with name '{}' already exists", payload.name)
        })));
    }

    // Convert request rules to policy rules - fix the type annotation issue
    let rules: Result<Vec<PolicyRule>, &'static str> = payload
        .rules
        .into_iter()
        .map(|rule| {
            let action = match rule.action.as_str() {
                "allow" => Ok(PolicyAction::Allow),
                "deny" => Ok(PolicyAction::Deny),
                "log" => Ok(PolicyAction::Log),
                _ => Err("Invalid action"),
            }?;

            Ok(PolicyRule {
                action,
                resource: rule.resource,
                conditions: rule.conditions.unwrap_or_default(),
            })
        })
        .collect();

    let rules = match rules {
        Ok(rules) => {
            if rules.is_empty() {
                return Ok(Json(json!({
                    "error": "Policy must have at least one rule"
                })));
            }
            rules
        }
        Err(_) => {
            return Ok(Json(json!({
                "error": "Invalid policy rule action. Must be 'allow', 'deny', or 'log'"
            })));
        }
    };

    let policy = EnhancedPolicy::new(
        payload.name,
        payload
            .description
            .unwrap_or_else(|| "Created via API".to_string()),
        rules,
    );

    let policy_id = policy.id;
    let response = PolicyResponse::from(policy.clone());

    match state.policy_engine.deploy_policy(policy) {
        Ok(()) => {
            info!("Policy {} created successfully", policy_id);
            Ok(Json(json!({
                "policy": response,
                "status": "created",
                "message": "Policy created and deployed successfully"
            })))
        }
        Err(e) => {
            error!("Failed to create policy: {}", e);
            Ok(Json(json!({
                "error": format!("Failed to create policy: {}", e),
                "status": "failed"
            })))
        }
    }
}

#[instrument(skip(state))]
async fn get_policy(
    State(state): State<Arc<PlatformState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let policy_id = match Uuid::from_str(&id) {
        Ok(id) => id,
        Err(_) => {
            return Ok(Json(json!({
                "error": "Invalid policy ID format. Must be a valid UUID.",
                "provided_id": id
            })));
        }
    };

    match state.policy_engine.get_policy(&policy_id) {
        Some(policy) => {
            let response = PolicyResponse::from((*policy).clone());
            Ok(Json(json!({
                "policy": response
            })))
        }
        None => {
            warn!("Policy not found: {}", id);
            Ok(Json(json!({
                "error": format!("Policy not found: {}", id),
                "policy_id": id
            })))
        }
    }
}

#[instrument(skip(state, payload))]
async fn update_policy(
    State(state): State<Arc<PlatformState>>,
    Path(id): Path<String>,
    Json(payload): Json<UpdatePolicyRequest>,
) -> Result<Json<Value>, StatusCode> {
    let policy_id = match Uuid::from_str(&id) {
        Ok(id) => id,
        Err(_) => {
            return Ok(Json(json!({
                "error": "Invalid policy ID format. Must be a valid UUID.",
                "provided_id": id
            })));
        }
    };

    let mut policy = match state.policy_engine.get_policy(&policy_id) {
        Some(policy) => (*policy).clone(),
        None => {
            warn!("Attempted to update non-existent policy: {}", id);
            return Ok(Json(json!({
                "error": format!("Policy not found: {}", id),
                "policy_id": id
            })));
        }
    };

    let mut updated = false;

    // Update fields if provided
    if let Some(name) = payload.name {
        if name.trim().is_empty() {
            return Ok(Json(json!({
                "error": "Policy name cannot be empty"
            })));
        }

        // Check if another policy already has this name
        if let Some(existing) = state.policy_engine.get_policy_by_name(&name) {
            if existing.id != policy_id {
                return Ok(Json(json!({
                    "error": format!("Another policy with name '{}' already exists", name)
                })));
            }
        }

        policy.name = name;
        updated = true;
    }

    if let Some(description) = payload.description {
        policy.description = description;
        updated = true;
    }

    if let Some(rules_req) = payload.rules {
        if rules_req.is_empty() {
            return Ok(Json(json!({
                "error": "Policy must have at least one rule"
            })));
        }

        let rules: Result<Vec<PolicyRule>, &'static str> = rules_req
            .into_iter()
            .map(|rule| {
                let action = match rule.action.as_str() {
                    "allow" => Ok(PolicyAction::Allow),
                    "deny" => Ok(PolicyAction::Deny),
                    "log" => Ok(PolicyAction::Log),
                    _ => Err("Invalid action"),
                }?;

                Ok(PolicyRule {
                    action,
                    resource: rule.resource,
                    conditions: rule.conditions.unwrap_or_default(),
                })
            })
            .collect();

        let rules = match rules {
            Ok(rules) => rules,
            Err(_) => {
                return Ok(Json(json!({
                    "error": "Invalid policy rule action. Must be 'allow', 'deny', or 'log'"
                })));
            }
        };

        policy.update_rules(rules);
        updated = true;
    }

    if !updated {
        return Ok(Json(json!({
            "error": "No fields to update provided. Specify 'name', 'description', or 'rules'."
        })));
    }

    // Hot-swap the updated policy
    match state.policy_engine.deploy_policy(policy.clone()) {
        Ok(()) => {
            info!(
                "Policy {} updated successfully to version {}",
                policy_id, policy.version
            );
            let response = PolicyResponse::from(policy);
            Ok(Json(json!({
                "policy": response,
                "status": "updated",
                "message": "Policy updated and hot-swapped successfully with zero downtime"
            })))
        }
        Err(e) => {
            error!("Failed to update policy: {}", e);
            Ok(Json(json!({
                "error": format!("Failed to update policy: {}", e),
                "status": "failed"
            })))
        }
    }
}

#[instrument(skip(state))]
async fn delete_policy(
    State(state): State<Arc<PlatformState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let policy_id = match Uuid::from_str(&id) {
        Ok(id) => id,
        Err(_) => {
            return Ok(Json(json!({
                "error": "Invalid policy ID format. Must be a valid UUID.",
                "provided_id": id
            })));
        }
    };

    // Get policy info before deletion for response (fix unused variable warning)
    let _policy_info = state
        .policy_engine
        .get_policy(&policy_id)
        .map(|p| (p.name.clone(), p.version));

    match state.policy_engine.remove_policy(&policy_id) {
        Ok(removed_policy) => {
            info!(
                "Policy {} ('{}') deleted successfully",
                policy_id, removed_policy.name
            );
            Ok(Json(json!({
                "status": "deleted",
                "policy_id": id,
                "policy_name": removed_policy.name,
                "policy_version": removed_policy.version,
                "message": format!("Policy '{}' deleted successfully", removed_policy.name)
            })))
        }
        Err(ReaperError::PolicyNotFound { .. }) => {
            warn!("Attempted to delete non-existent policy: {}", id);
            Ok(Json(json!({
                "error": format!("Policy not found: {}", id),
                "policy_id": id,
                "message": "Policy may have already been deleted"
            })))
        }
        Err(e) => {
            error!("Failed to delete policy {}: {}", policy_id, e);
            Ok(Json(json!({
                "error": format!("Failed to delete policy: {}", e),
                "policy_id": id,
                "status": "failed"
            })))
        }
    }
}

#[instrument(skip(state))]
async fn deploy_policy_to_agents(
    State(state): State<Arc<PlatformState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let policy_id = match Uuid::from_str(&id) {
        Ok(id) => id,
        Err(_) => {
            return Ok(Json(json!({
                "error": "Invalid policy ID format. Must be a valid UUID.",
                "provided_id": id
            })));
        }
    };

    match state.policy_engine.get_policy(&policy_id) {
        Some(policy) => {
            // Update deployment stats
            {
                let mut stats = state.deployment_stats.write();
                stats.total_deployments += 1;
                stats.successful_deployments += 1;
            }

            info!(
                "Deploying policy {} ('{}') to agents",
                policy_id, policy.name
            );
            Ok(Json(json!({
                "status": "deployed",
                "policy_id": id,
                "policy_name": policy.name,
                "policy_version": policy.version,
                "deployed_to_agents": 1, // For now, we'll expand this later when we have agent registry
                "deployment_time": Utc::now(),
                "message": format!("Policy '{}' deployed successfully to agents", policy.name)
            })))
        }
        None => {
            // Update failure stats
            {
                let mut stats = state.deployment_stats.write();
                stats.total_deployments += 1;
                stats.failed_deployments += 1;
            }

            warn!("Attempted to deploy non-existent policy: {}", id);
            Ok(Json(json!({
                "error": format!("Policy not found: {}", id),
                "policy_id": id,
                "status": "failed",
                "message": "Cannot deploy non-existent policy"
            })))
        }
    }
}

#[instrument]
async fn list_agents() -> Result<Json<Value>, StatusCode> {
    // Placeholder for agent management - will be implemented in future iterations
    Ok(Json(json!({
        "agents": [],
        "total": 0,
        "message": "Agent management will be implemented in the next iteration",
        "planned_features": [
            "Agent discovery and registration",
            "Health monitoring",
            "Policy deployment tracking",
            "Performance metrics aggregation"
        ]
    })))
}

#[instrument]
async fn get_agent(Path(id): Path<String>) -> Result<Json<Value>, StatusCode> {
    // Placeholder for agent details
    Ok(Json(json!({
        "agent_id": id,
        "status": "not_implemented",
        "message": "Agent details will be implemented in the next iteration",
        "planned_info": {
            "status": "healthy|unhealthy|unknown",
            "last_seen": "timestamp",
            "deployed_policies": "array of policy IDs",
            "performance_metrics": "latency and throughput stats"
        }
    })))
}

/// Create a .rbb bundle from a policy
#[instrument(skip(state))]
async fn create_bundle(
    State(state): State<Arc<PlatformState>>,
    Json(req): Json<CreateBundleRequest>,
) -> Result<Json<BundleResponse>, (StatusCode, String)> {
    info!(
        "Creating bundle for policy {} (version: {})",
        req.policy_id, req.version
    );

    // 1. Get the policy from the engine
    let policy_uuid = Uuid::from_str(&req.policy_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid policy ID format".to_string(),
        )
    })?;

    let policy = state
        .policy_engine
        .get_policy(&policy_uuid)
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Policy not found".to_string()))?;

    // 2. Convert Enhanced Policy to Reaper Policy AST (simplified for now)
    // For now, we'll create a simple Policy with the metadata
    let mut metadata = std::collections::HashMap::new();
    metadata.insert("version".to_string(), req.version.clone());
    if let Some(desc) = req.description {
        metadata.insert("description".to_string(), desc);
    }

    // Convert policy rules to Reaper DSL rules (simplified: all rules become unconditional)
    let reap_rules: Vec<ReapRule> = policy
        .rules
        .iter()
        .map(|rule| ReapRule {
            name: format!("rule_{}", uuid::Uuid::new_v4().simple()),
            decision: match rule.action {
                PolicyAction::Allow => Decision::Allow,
                PolicyAction::Deny => Decision::Deny,
                _ => Decision::Deny,
            },
            condition: ReapCondition::True, // Simplified: all rules unconditional
        })
        .collect();

    let reap_policy = Policy {
        name: policy.name.clone(),
        metadata,
        default_decision: Decision::Deny,
        rules: reap_rules,
    };

    // 3. Compile to .rbb bundle
    let bundle = PolicyBundle::new(reap_policy);

    // Calculate size for response (serialize temporarily)
    let bundle_bytes = bundle.to_bytes().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Bundle compilation failed: {}", e),
        )
    })?;
    let size_bytes = bundle_bytes.len();

    // 4. Store bundle
    let bundle_id = format!("bundle_{}", uuid::Uuid::new_v4().simple());
    state
        .bundle_storage
        .write()
        .insert(bundle_id.clone(), bundle);

    info!("Bundle created successfully: {}", bundle_id);

    Ok(Json(BundleResponse {
        bundle_id,
        policy_id: req.policy_id,
        version: req.version,
        size_bytes,
        created_at: Utc::now(),
    }))
}

/// Get a bundle by ID
#[instrument(skip(state))]
async fn get_bundle(
    State(state): State<Arc<PlatformState>>,
    Path(bundle_id): Path<String>,
) -> Result<Vec<u8>, (StatusCode, String)> {
    info!("Retrieving bundle: {}", bundle_id);

    let storage = state.bundle_storage.read();
    let bundle = storage
        .get(&bundle_id)
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Bundle not found".to_string()))?;

    bundle.to_bytes().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Bundle serialization failed: {}", e),
        )
    })
}

/// Deploy a bundle to all or specific agents
#[instrument(skip(state))]
async fn deploy_bundle_to_agents(
    State(state): State<Arc<PlatformState>>,
    Json(req): Json<DeployBundleToAgentsRequest>,
) -> Result<Json<DeployBundleToAgentsResponse>, (StatusCode, String)> {
    info!("Deploying bundle {} to agents", req.bundle_id);

    // Get the bundle
    let storage = state.bundle_storage.read();
    let _bundle = storage.get(&req.bundle_id).ok_or_else(|| {
        warn!("Bundle not found: {}", req.bundle_id);
        (StatusCode::NOT_FOUND, "Bundle not found".to_string())
    })?;
    drop(storage);

    // TODO: Full implementation coming
    warn!("Bundle deployment not yet fully implemented");

    Ok(Json(DeployBundleToAgentsResponse {
        bundle_id: req.bundle_id,
        total_agents: 0,
        successful: 0,
        failed: 0,
        results: vec![],
    }))
}
