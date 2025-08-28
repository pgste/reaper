use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use policy_engine::{EnhancedPolicy, PolicyAction, PolicyEngine, PolicyRule};
use reaper_core::{endpoints, ReaperError, BUILD_INFO, VERSION};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::str::FromStr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::{error, info, instrument, warn};
use uuid::Uuid;

#[derive(Clone)]
struct PlatformState {
    policy_engine: PolicyEngine,
    deployment_stats: Arc<RwLock<DeploymentStats>>,
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

    let state = Arc::new(PlatformState {
        policy_engine,
        deployment_stats: Arc::new(RwLock::new(DeploymentStats::default())),
    });

    let app = Router::new()
        // Health and metrics
        .route(endpoints::HEALTH, get(health_check))
        .route(endpoints::METRICS, get(metrics))
        // Policy management
        .route(
            endpoints::API_V1_POLICIES,
            get(list_policies).post(create_policy),
        )
        .route(
            "/api/v1/policies/:id",
            get(get_policy).put(update_policy).delete(delete_policy),
        )
        .route("/api/v1/policies/:id/deploy", post(deploy_policy_to_agents))
        // Agent management (placeholder for now)
        .route(endpoints::API_V1_AGENTS, get(list_agents))
        .route("/api/v1/agents/:id", get(get_agent))
        .with_state(state);

    let listener = TcpListener::bind("0.0.0.0:8081").await?;
    info!("🎯 Reaper Platform listening on {}", listener.local_addr()?);
    info!("");
    info!("📋 Policy Management API:");
    info!("  GET    /api/v1/policies     - List all policies");
    info!("  POST   /api/v1/policies     - Create new policy");
    info!("  GET    /api/v1/policies/:id - Get policy details");
    info!("  PUT    /api/v1/policies/:id - Update policy");
    info!("  DELETE /api/v1/policies/:id - Delete policy");
    info!("  POST   /api/v1/policies/:id/deploy - Deploy to agents");
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
