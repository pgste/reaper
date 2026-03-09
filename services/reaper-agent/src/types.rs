//! Request and response types for the agent API.
//!
//! These types define the JSON schemas for all API endpoints.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ============================================================================
// Policy Evaluation Types
// ============================================================================

/// Policy evaluation request from external services.
///
/// Either `policy_id` or `policy_name` should be provided to identify
/// which policy to evaluate. If both are provided, `policy_id` takes precedence.
#[derive(Debug, Clone, Deserialize)]
pub struct EvaluateRequest {
    /// UUID of the policy to evaluate
    pub policy_id: Option<String>,
    /// Name of the policy to evaluate
    pub policy_name: Option<String>,
    /// Principal making the request (e.g., user ID, role)
    pub principal: String,
    /// Resource being accessed
    pub resource: String,
    /// Action being performed
    pub action: String,
    /// Additional context for evaluation (optional)
    pub context: Option<HashMap<String, String>>,
}

/// Response from policy evaluation.
#[derive(Debug, Clone, Serialize)]
pub struct EvaluateResponse {
    /// Whether the request is allowed
    pub allowed: bool,
    /// Decision outcome ("allow" or "deny")
    pub decision: String,
    /// Name of the policy that made the decision
    pub policy_name: Option<String>,
    /// Evaluation duration in nanoseconds
    pub evaluation_time_ns: u64,
    /// Whether the result was served from cache
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached: Option<bool>,
}

/// Batch evaluation request for multiple principals/actions/resources.
#[derive(Debug, Clone, Deserialize)]
pub struct BatchEvaluateRequest {
    /// Policy to evaluate
    pub policy_id: Option<String>,
    pub policy_name: Option<String>,
    /// List of evaluation requests
    pub requests: Vec<BatchRequestItem>,
}

/// Single item in a batch evaluation request.
#[derive(Debug, Clone, Deserialize)]
pub struct BatchRequestItem {
    /// Request identifier for correlation
    pub id: String,
    pub principal: String,
    pub resource: String,
    pub action: String,
    pub context: Option<HashMap<String, String>>,
}

/// Response item for batch evaluation.
#[derive(Debug, Clone, Serialize)]
pub struct BatchResponseItem {
    /// Correlates with request ID
    pub id: String,
    pub allowed: bool,
    pub decision: String,
    pub evaluation_time_ns: u64,
}

/// Batch evaluation response.
#[derive(Debug, Clone, Serialize)]
pub struct BatchEvaluateResponse {
    /// Individual results
    pub results: Vec<BatchResponseItem>,
    /// Total evaluation time
    pub total_time_ns: u64,
    /// Number of requests processed
    pub count: usize,
}

// ============================================================================
// Policy Deployment Types
// ============================================================================

/// Policy deployment request from platform.
#[derive(Debug, Clone, Deserialize)]
pub struct DeployPolicyRequest {
    /// Policy UUID
    pub policy_id: String,
    /// Human-readable policy name
    pub name: String,
    /// Policy description
    pub description: String,
    /// Policy rules
    pub rules: Vec<DeployPolicyRule>,
}

/// Rule within a policy deployment.
#[derive(Debug, Clone, Deserialize)]
pub struct DeployPolicyRule {
    /// Action to take ("allow" or "deny")
    pub action: String,
    /// Resource pattern (supports wildcards)
    pub resource: String,
    /// Optional conditions for the rule
    pub conditions: Option<Vec<String>>,
}

/// Bundle deployment request.
#[derive(Debug, Clone, Deserialize)]
pub struct DeployBundleRequest {
    /// Raw .rbb bundle bytes
    pub bundle: Vec<u8>,
    /// Expected bundle version
    pub version: String,
    /// Override version check
    #[serde(default)]
    pub force: bool,
}

/// Bundle deployment response.
#[derive(Debug, Clone, Serialize)]
pub struct DeployBundleResponse {
    /// Policy ID from the bundle
    pub policy_id: String,
    /// Bundle version
    pub version: String,
    /// Deployment timestamp
    pub deployed_at: String,
    /// SHA-256 hash of the bundle
    pub bundle_hash: String,
}

/// Compiled policy deployment request.
#[derive(Debug, Clone, Deserialize)]
pub struct DeployCompiledRequest {
    /// Policy ID
    pub policy_id: String,
    /// Policy name
    pub name: String,
    /// Policy language (e.g., "simple", "cedar", "reaper_dsl")
    pub language: String,
    /// Raw policy content
    pub content: String,
    /// Optional data for the policy
    pub data: Option<serde_json::Value>,
}

// ============================================================================
// Entity Types
// ============================================================================

/// Entity upsert request.
#[derive(Debug, Clone, Deserialize)]
pub struct UpsertEntityRequest {
    /// Entity type (e.g., "user", "resource")
    pub entity_type: String,
    /// Entity identifier
    pub id: String,
    /// Entity attributes
    pub attributes: HashMap<String, serde_json::Value>,
}

/// Batch entity upsert request.
#[derive(Debug, Clone, Deserialize)]
pub struct BatchUpsertRequest {
    /// List of entities to upsert
    pub entities: Vec<UpsertEntityRequest>,
}

/// Entity response.
#[derive(Debug, Clone, Serialize)]
pub struct EntityResponse {
    /// Entity type
    pub entity_type: String,
    /// Entity ID
    pub id: String,
    /// Entity attributes
    pub attributes: HashMap<String, serde_json::Value>,
}

// ============================================================================
// Data Loading Types
// ============================================================================

/// Data loading request for JSON entities.
#[derive(Debug, Clone, Deserialize)]
pub struct LoadDataRequest {
    /// Entities to load
    pub entities: Vec<serde_json::Value>,
}

/// Data sync request with versioning.
#[derive(Debug, Clone, Deserialize)]
pub struct SyncDataRequest {
    /// Full replacement of all entities
    pub entities: Vec<serde_json::Value>,
    /// Version identifier for sync
    pub version: Option<String>,
}

// ============================================================================
// Decision Logging Types
// ============================================================================

/// Query parameters for decision log retrieval.
#[derive(Debug, Clone, Deserialize)]
pub struct DecisionQuery {
    /// Filter by principal
    pub principal: Option<String>,
    /// Filter by action
    pub action: Option<String>,
    /// Filter by resource
    pub resource: Option<String>,
    /// Filter by decision outcome
    pub decision: Option<String>,
    /// Maximum results to return
    pub limit: Option<usize>,
    /// Offset for pagination
    pub offset: Option<usize>,
}

/// Decision statistics response.
#[derive(Debug, Clone, Serialize)]
pub struct DecisionStats {
    /// Total decisions logged
    pub total: usize,
    /// Total allow decisions
    pub allows: usize,
    /// Total deny decisions
    pub denies: usize,
    /// Average evaluation time in nanoseconds
    pub avg_evaluation_time_ns: f64,
    /// P50 evaluation time in nanoseconds
    pub p50_evaluation_time_ns: u64,
    /// P99 evaluation time in nanoseconds
    pub p99_evaluation_time_ns: u64,
}

/// Decision export request.
#[derive(Debug, Clone, Deserialize)]
pub struct ExportDecisionsRequest {
    /// Output format (ndjson, json, csv)
    #[serde(default = "default_export_format")]
    pub format: String,
    /// Optional output file path
    pub path: Option<String>,
}

fn default_export_format() -> String {
    "ndjson".to_string()
}

// ============================================================================
// Package Types
// ============================================================================

/// Package evaluation request.
#[derive(Debug, Clone, Deserialize)]
pub struct PackageEvaluateRequest {
    pub principal: String,
    pub resource: String,
    pub action: String,
    pub context: Option<HashMap<String, String>>,
}

// ============================================================================
// Health & Metrics Types
// ============================================================================

/// Health check response.
#[derive(Debug, Clone, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub policies_loaded: usize,
    pub entities_loaded: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uptime_seconds: Option<u64>,
}

/// Readiness check response.
#[derive(Debug, Clone, Serialize)]
pub struct ReadyResponse {
    pub ready: bool,
    pub policies_loaded: usize,
    pub cache_enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub management_connected: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_evaluate_request_deserialize() {
        let json = r#"{
            "policy_name": "test-policy",
            "principal": "alice",
            "resource": "/api/data",
            "action": "read"
        }"#;

        let req: EvaluateRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.policy_name, Some("test-policy".to_string()));
        assert_eq!(req.principal, "alice");
        assert_eq!(req.resource, "/api/data");
        assert_eq!(req.action, "read");
        assert!(req.policy_id.is_none());
        assert!(req.context.is_none());
    }

    #[test]
    fn test_evaluate_request_with_context() {
        let json = r#"{
            "policy_id": "550e8400-e29b-41d4-a716-446655440000",
            "principal": "bob",
            "resource": "/admin",
            "action": "write",
            "context": {
                "department": "engineering",
                "level": "senior"
            }
        }"#;

        let req: EvaluateRequest = serde_json::from_str(json).unwrap();
        assert!(req.policy_id.is_some());
        let ctx = req.context.unwrap();
        assert_eq!(ctx.get("department"), Some(&"engineering".to_string()));
    }

    #[test]
    fn test_evaluate_response_serialize() {
        let resp = EvaluateResponse {
            allowed: true,
            decision: "allow".to_string(),
            policy_name: Some("admin-policy".to_string()),
            evaluation_time_ns: 500,
            cached: Some(true),
        };

        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains(r#""allowed":true"#));
        assert!(json.contains(r#""decision":"allow""#));
        assert!(json.contains(r#""cached":true"#));
    }

    #[test]
    fn test_evaluate_response_skip_cached_none() {
        let resp = EvaluateResponse {
            allowed: false,
            decision: "deny".to_string(),
            policy_name: None,
            evaluation_time_ns: 1000,
            cached: None,
        };

        let json = serde_json::to_string(&resp).unwrap();
        assert!(!json.contains("cached"));
    }

    #[test]
    fn test_batch_request_deserialize() {
        let json = r#"{
            "policy_name": "batch-policy",
            "requests": [
                {"id": "1", "principal": "alice", "resource": "/a", "action": "read"},
                {"id": "2", "principal": "bob", "resource": "/b", "action": "write"}
            ]
        }"#;

        let req: BatchEvaluateRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.requests.len(), 2);
        assert_eq!(req.requests[0].id, "1");
        assert_eq!(req.requests[1].principal, "bob");
    }

    #[test]
    fn test_deploy_policy_request_deserialize() {
        let json = r#"{
            "policy_id": "550e8400-e29b-41d4-a716-446655440000",
            "name": "test-policy",
            "description": "A test policy",
            "rules": [
                {"action": "allow", "resource": "/public/*"},
                {"action": "deny", "resource": "/admin/*", "conditions": ["role == admin"]}
            ]
        }"#;

        let req: DeployPolicyRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.name, "test-policy");
        assert_eq!(req.rules.len(), 2);
        assert!(req.rules[1].conditions.is_some());
    }

    #[test]
    fn test_deploy_bundle_request_defaults() {
        let json = r#"{
            "bundle": [1, 2, 3, 4],
            "version": "1.0.0"
        }"#;

        let req: DeployBundleRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.bundle, vec![1, 2, 3, 4]);
        assert_eq!(req.version, "1.0.0");
        assert!(!req.force);
    }

    #[test]
    fn test_upsert_entity_request() {
        let json = r#"{
            "entity_type": "user",
            "id": "alice",
            "attributes": {
                "department": "engineering",
                "level": 3
            }
        }"#;

        let req: UpsertEntityRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.entity_type, "user");
        assert_eq!(req.id, "alice");
        assert!(req.attributes.contains_key("department"));
    }

    #[test]
    fn test_decision_query_defaults() {
        let json = r#"{}"#;
        let query: DecisionQuery = serde_json::from_str(json).unwrap();
        assert!(query.principal.is_none());
        assert!(query.limit.is_none());
    }

    #[test]
    fn test_decision_stats_serialize() {
        let stats = DecisionStats {
            total: 1000,
            allows: 800,
            denies: 200,
            avg_evaluation_time_ns: 750.5,
            p50_evaluation_time_ns: 500,
            p99_evaluation_time_ns: 2500,
        };

        let json = serde_json::to_string(&stats).unwrap();
        assert!(json.contains(r#""total":1000"#));
        assert!(json.contains(r#""allows":800"#));
    }

    #[test]
    fn test_export_request_default_format() {
        let json = r#"{}"#;
        let req: ExportDecisionsRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.format, "ndjson");
    }

    #[test]
    fn test_health_response() {
        let resp = HealthResponse {
            status: "healthy".to_string(),
            version: "0.1.0".to_string(),
            policies_loaded: 5,
            entities_loaded: 100,
            uptime_seconds: Some(3600),
        };

        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains(r#""status":"healthy""#));
        assert!(json.contains(r#""policies_loaded":5"#));
    }

    #[test]
    fn test_ready_response() {
        let resp = ReadyResponse {
            ready: true,
            policies_loaded: 3,
            cache_enabled: true,
            management_connected: Some(true),
        };

        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains(r#""ready":true"#));
    }
}
