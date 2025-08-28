use axum::{
    extract::State,
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use policy_engine::{EnhancedPolicy, PolicyAction, PolicyEngine, PolicyRequest, PolicyRule};
use reaper_core::{endpoints, ReaperError, BUILD_INFO, VERSION};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::{error, info, instrument, warn};
use uuid::Uuid;

#[derive(Clone)]
struct AgentState {
    policy_engine: PolicyEngine,
    stats: Arc<AgentStats>,
}

// Add Debug manually since PolicyEngine has its own Debug implementation now
impl std::fmt::Debug for AgentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentState")
            .field("policy_engine", &self.policy_engine)
            .field("stats", &"AgentStats")
            .finish()
    }
}

#[derive(Debug, Default)]
struct AgentStats {
    requests_processed: AtomicU64,
    total_evaluation_time_ns: AtomicU64,
    policy_cache_hits: AtomicU64,
    policy_cache_misses: AtomicU64,
}

/// Policy evaluation request from external services
#[derive(Debug, Deserialize)]
struct EvaluateRequest {
    pub policy_id: Option<String>,
    pub policy_name: Option<String>,
    pub resource: String,
    pub action: String,
    pub context: Option<HashMap<String, String>>,
}

/// Policy deployment request from platform
#[derive(Debug, Deserialize)]
struct DeployPolicyRequest {
    pub policy_id: String,
    pub name: String,
    pub description: String,
    pub rules: Vec<DeployPolicyRule>,
}

#[derive(Debug, Deserialize)]
struct DeployPolicyRule {
    pub action: String,
    pub resource: String,
    pub conditions: Option<Vec<String>>,
}

impl AgentStats {
    fn record_evaluation(&self, evaluation_time_ns: u64) {
        self.requests_processed.fetch_add(1, Ordering::Relaxed);
        self.total_evaluation_time_ns
            .fetch_add(evaluation_time_ns, Ordering::Relaxed);
    }

    fn record_cache_hit(&self) {
        self.policy_cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    fn record_cache_miss(&self) {
        self.policy_cache_misses.fetch_add(1, Ordering::Relaxed);
    }

    fn get_average_evaluation_time_ns(&self) -> f64 {
        let total_requests = self.requests_processed.load(Ordering::Relaxed);
        let total_time = self.total_evaluation_time_ns.load(Ordering::Relaxed);

        if total_requests > 0 {
            total_time as f64 / total_requests as f64
        } else {
            0.0
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    info!(
        "Starting Reaper Agent {} - High-Performance Policy Enforcement",
        BUILD_INFO
    );

    let policy_engine = PolicyEngine::new();

    // Create a demo policy for immediate testing
    let demo_policy = EnhancedPolicy::new(
        "demo-allow-all".to_string(),
        "Demo policy that allows all requests for testing".to_string(),
        vec![PolicyRule {
            action: PolicyAction::Allow,
            resource: "*".to_string(),
            conditions: vec![],
        }],
    );

    info!("Deploying demo allow-all policy for testing");
    policy_engine.deploy_policy(demo_policy)?;

    let state = Arc::new(AgentState {
        policy_engine,
        stats: Arc::new(AgentStats::default()),
    });

    let app = Router::new()
        // Health and metrics
        .route(endpoints::HEALTH, get(health_check))
        .route(endpoints::METRICS, get(metrics))
        // Policy evaluation - the core agent functionality
        .route(endpoints::API_V1_MESSAGES, post(evaluate_policy))
        // Policy management from platform
        .route("/api/v1/policies/deploy", post(deploy_policy))
        .route("/api/v1/policies", get(list_policies))
        .with_state(state);

    let listener = TcpListener::bind("0.0.0.0:8080").await?;
    info!("🎯 Reaper Agent listening on {}", listener.local_addr()?);
    info!("");
    info!("⚡ Policy Evaluation API:");
    info!("  POST /api/v1/messages        - Evaluate policy decision");
    info!("  POST /api/v1/policies/deploy - Deploy policy from platform");
    info!("  GET  /api/v1/policies        - List active policies");
    info!("");
    info!("🚀 Ready for sub-microsecond policy enforcement!");

    axum::serve(listener, app).await?;

    Ok(())
}

#[instrument]
async fn health_check() -> Result<Json<Value>, StatusCode> {
    Ok(Json(json!({
        "status": "healthy",
        "service": "reaper-agent",
        "version": VERSION,
        "capabilities": [
            "policy-evaluation",
            "hot-swapping",
            "sub-microsecond-latency"
        ]
    })))
}

#[instrument]
async fn metrics(State(state): State<Arc<AgentState>>) -> Result<Json<Value>, StatusCode> {
    let engine_stats = state.policy_engine.get_stats();
    let agent_stats = &state.stats;

    let requests_processed = agent_stats.requests_processed.load(Ordering::Relaxed);
    let cache_hits = agent_stats.policy_cache_hits.load(Ordering::Relaxed);
    let cache_misses = agent_stats.policy_cache_misses.load(Ordering::Relaxed);
    let avg_evaluation_time_ns = agent_stats.get_average_evaluation_time_ns();

    Ok(Json(json!({
        "service": "reaper-agent",
        "performance": {
            "requests_processed": requests_processed,
            "average_evaluation_time_nanoseconds": avg_evaluation_time_ns,
            "average_evaluation_time_microseconds": avg_evaluation_time_ns / 1000.0,
            "target_evaluation_time_microseconds": 1.0
        },
        "policies": {
            "total_loaded": engine_stats.total_policies,
            "has_default": engine_stats.has_default_policy
        },
        "cache": {
            "hits": cache_hits,
            "misses": cache_misses,
            "hit_rate": if (cache_hits + cache_misses) > 0 {
                (cache_hits as f64 / (cache_hits + cache_misses) as f64) * 100.0
            } else {
                0.0
            }
        }
    })))
}

#[instrument(skip(state, payload), fields(resource = %payload.resource, action = %payload.action))]
async fn evaluate_policy(
    State(state): State<Arc<AgentState>>,
    Json(payload): Json<EvaluateRequest>,
) -> Result<Json<Value>, StatusCode> {
    let start_time = std::time::Instant::now();

    // Determine which policy to use
    let policy_id = if let Some(id_str) = payload.policy_id {
        match Uuid::from_str(&id_str) {
            Ok(id) => Some(id),
            Err(_) => {
                return Ok(Json(json!({
                    "error": "Invalid policy ID format",
                    "policy_id": id_str
                })))
            }
        }
    } else if let Some(name) = payload.policy_name {
        // Look up policy by name
        match state.policy_engine.get_policy_by_name(&name) {
            Some(policy) => {
                state.stats.record_cache_hit();
                Some(policy.id)
            }
            None => {
                state.stats.record_cache_miss();
                return Ok(Json(json!({
                    "error": "Policy not found",
                    "policy_name": name
                })));
            }
        }
    } else {
        // Use any available policy (demo mode)
        let policies = state.policy_engine.list_policies();
        if let Some(policy) = policies.first() {
            Some(policy.id)
        } else {
            return Ok(Json(json!({
                "error": "No policies available for evaluation"
            })));
        }
    };

    let policy_id = match policy_id {
        Some(id) => id,
        None => {
            return Ok(Json(json!({
                "error": "No policy specified and no default policy available"
            })))
        }
    };

    // Create policy request
    let request = PolicyRequest {
        resource: payload.resource,
        action: payload.action,
        context: payload.context.unwrap_or_default(),
    };

    // Evaluate policy
    match state.policy_engine.evaluate(&policy_id, &request) {
        Ok(decision) => {
            let total_time = start_time.elapsed();
            state.stats.record_evaluation(decision.evaluation_time_ns);

            let decision_str = match decision.decision {
                PolicyAction::Allow => "allow",
                PolicyAction::Deny => "deny",
                PolicyAction::Log => "log",
            };

            info!(
                "Policy evaluation: {} -> {} ({}ns)",
                request.resource, decision_str, decision.evaluation_time_ns
            );

            Ok(Json(json!({
                "decision": decision_str,
                "policy_id": decision.policy_id.to_string(),
                "policy_version": decision.policy_version,
                "evaluation_time_microseconds": decision.evaluation_time_ns as f64 / 1000.0,
                "total_time_microseconds": total_time.as_nanos() as f64 / 1000.0,
                "matched_rule": decision.matched_rule,
                "agent_id": "reaper-agent-001"
            })))
        }
        Err(ReaperError::PolicyNotFound { policy_id }) => {
            state.stats.record_cache_miss();
            warn!("Policy not found: {}", policy_id);
            Ok(Json(json!({
                "error": "Policy not found",
                "policy_id": policy_id
            })))
        }
        Err(e) => {
            error!("Policy evaluation failed: {}", e);
            Ok(Json(json!({
                "error": format!("Policy evaluation failed: {}", e)
            })))
        }
    }
}

#[instrument(skip(state, payload))]
async fn deploy_policy(
    State(state): State<Arc<AgentState>>,
    Json(payload): Json<DeployPolicyRequest>,
) -> Result<Json<Value>, StatusCode> {
    let policy_id = match Uuid::from_str(&payload.policy_id) {
        Ok(id) => id,
        Err(_) => {
            return Ok(Json(json!({
                "error": "Invalid policy ID format"
            })))
        }
    };

    // Convert rules
    let rules: Result<Vec<PolicyRule>, String> = payload
        .rules
        .into_iter()
        .map(|rule| {
            let action = match rule.action.as_str() {
                "allow" => Ok(PolicyAction::Allow),
                "deny" => Ok(PolicyAction::Deny),
                "log" => Ok(PolicyAction::Log),
                _ => Err(format!("Invalid action: {}", rule.action)),
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
        Err(e) => {
            return Ok(Json(json!({
                "error": e
            })))
        }
    };

    // Create policy with the specified ID
    let mut policy = EnhancedPolicy::new(payload.name, payload.description, rules);

    // Override the generated ID with the one from the request
    policy.id = policy_id;

    // Hot-swap deploy the policy
    match state.policy_engine.deploy_policy(policy.clone()) {
        Ok(()) => {
            info!("Policy {} hot-swapped successfully", policy_id);
            Ok(Json(json!({
                "status": "deployed",
                "policy_id": policy.id.to_string(),
                "policy_name": policy.name,
                "version": policy.version,
                "deployment_time": chrono::Utc::now(),
                "message": "Policy hot-swapped successfully with zero downtime"
            })))
        }
        Err(e) => {
            error!("Failed to deploy policy: {}", e);
            Ok(Json(json!({
                "error": format!("Failed to deploy policy: {}", e)
            })))
        }
    }
}

#[instrument(skip(state))]
async fn list_policies(State(state): State<Arc<AgentState>>) -> Result<Json<Value>, StatusCode> {
    let policies = state.policy_engine.list_policies();

    let policy_list: Vec<Value> = policies
        .into_iter()
        .map(|policy| {
            json!({
                "id": policy.id.to_string(),
                "name": policy.name,
                "version": policy.version,
                "rules_count": policy.rules.len(),
                "created_at": policy.created_at,
                "updated_at": policy.updated_at
            })
        })
        .collect();

    Ok(Json(json!({
        "policies": policy_list,
        "total": policy_list.len()
    })))
}
