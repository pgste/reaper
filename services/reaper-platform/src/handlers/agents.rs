//! Agent management handlers (placeholder).

use axum::{extract::Path, http::StatusCode, response::Json};
use serde_json::{json, Value};
use tracing::instrument;

#[instrument]
pub async fn list_agents() -> Result<Json<Value>, StatusCode> {
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
pub async fn get_agent(Path(id): Path<String>) -> Result<Json<Value>, StatusCode> {
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
