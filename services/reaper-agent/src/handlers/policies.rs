//! Policy management handlers.
//!
//! This module contains handlers for policy deployment and management:
//! - `deploy_policy` - Deploy a policy from JSON rules
//! - `list_policies` - List all deployed policies
//! - `get_policy_versions` - Get version history for a policy
//! - `get_policy_current_version` - Get current version of a policy
//! - `deploy_compiled_policy` - Deploy and compile a .reap policy
//! - `deploy_bundle` - Deploy a policy bundle (.rbb file)

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use policy_engine::{EnhancedPolicy, PolicyAction, PolicyBundle, PolicyRule};
use serde::Deserialize;
use serde_json::{json, Value};
use std::str::FromStr;
use std::sync::Arc;
use tracing::{error, info, instrument, warn};
use uuid::Uuid;

use crate::observability::{ACTIVE_POLICIES, ERRORS_TOTAL};
use crate::state::AgentState;
use crate::types::{DeployBundleRequest, DeployBundleResponse, DeployPolicyRequest};

/// Deploy and compile a .reap policy file with the agent's DataStore.
#[derive(Debug, Deserialize)]
pub struct DeployCompiledPolicyRequest {
    /// Raw .reap policy content
    pub policy_content: String,
    /// Policy name
    pub policy_name: String,
}

/// Deploy a policy from JSON rules.
#[instrument(skip(state, payload))]
pub async fn deploy_policy(
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
            // Update active policies gauge
            let engine_stats = state.policy_engine.get_stats();
            ACTIVE_POLICIES.set(engine_stats.total_policies as f64);

            // Invalidate cached decisions — the policy just changed, so any
            // decision computed under the old policy is now stale.
            if let Some(ref cache) = state.decision_cache {
                cache.invalidate();
            }

            // Save to policy cache if enabled
            if let Some(ref cache) = state.policy_cache {
                if let Err(e) = cache.save_policy(&policy).await {
                    warn!("Failed to cache policy {}: {}", policy_id, e);
                }
            }

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
            ERRORS_TOTAL
                .with_label_values(&["policy_deployment_failed"])
                .inc();
            error!("Failed to deploy policy: {}", e);
            Ok(Json(json!({
                "error": format!("Failed to deploy policy: {}", e)
            })))
        }
    }
}

/// List all deployed policies.
#[instrument(skip(state))]
pub async fn list_policies(
    State(state): State<Arc<AgentState>>,
) -> Result<Json<Value>, StatusCode> {
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

/// Get version history for a policy.
#[instrument(skip(state))]
pub async fn get_policy_versions(
    State(state): State<Arc<AgentState>>,
    Path(policy_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let policy_uuid = Uuid::from_str(&policy_id)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid policy ID: {}", e)))?;

    let versions = state.policy_engine.list_versions(&policy_uuid);

    let version_list: Vec<Value> = versions
        .into_iter()
        .map(|v| {
            json!({
                "version": v.version,
                "deployed_at": chrono::DateTime::<chrono::Utc>::from(v.deployed_at).to_rfc3339(),
                "bundle_hash": v.bundle_hash.iter().map(|b| format!("{:02x}", b)).collect::<String>(),
                "policy_id": v.policy_id
            })
        })
        .collect();

    Ok(Json(json!({
        "policy_id": policy_id,
        "versions": version_list,
        "total": version_list.len()
    })))
}

/// Get current version of a policy.
#[instrument(skip(state))]
pub async fn get_policy_current_version(
    State(state): State<Arc<AgentState>>,
    Path(policy_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let policy_uuid = Uuid::from_str(&policy_id)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid policy ID: {}", e)))?;

    let version = state
        .policy_engine
        .get_version(&policy_uuid)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("No version found for policy {}", policy_id),
            )
        })?;

    Ok(Json(json!({
        "policy_id": policy_id,
        "version": version.version,
        "deployed_at": chrono::DateTime::<chrono::Utc>::from(version.deployed_at).to_rfc3339(),
        "bundle_hash": version.bundle_hash.iter().map(|b| format!("{:02x}", b)).collect::<String>()
    })))
}

/// Deploy and compile a .reap policy file with the agent's DataStore.
#[instrument(skip(state, payload))]
pub async fn deploy_compiled_policy(
    State(state): State<Arc<AgentState>>,
    Json(payload): Json<DeployCompiledPolicyRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    info!(
        "Deploying and compiling .reap policy: {}",
        payload.policy_name
    );

    // Build the Reaper-DSL policy as a first-class EnhancedPolicy. Parsing
    // errors are a client error (400). Compilation of a valid-but-unsupported
    // construct is NOT an error — build_evaluator_with_data falls back to the
    // AST interpreter, matching bootstrap loading and restart restore so the
    // same policy behaves identically however it is deployed.
    let mut enhanced_policy = EnhancedPolicy {
        // Stable id from the name so redeploying this policy overwrites in place.
        id: policy_engine::stable_policy_id(&payload.policy_name),
        version: 1,
        name: payload.policy_name.clone(),
        description: "Compiled .reap policy".to_string(),
        language: policy_engine::PolicyLanguage::ReaperDsl,
        content: payload.policy_content.clone(),
        rules: vec![],
        metadata: std::collections::HashMap::new(),
        priority: 100,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        evaluator: None,
        source_metadata: None,
    };

    enhanced_policy
        .build_evaluator_with_data(Some(state.data_store.clone()))
        .map_err(|e| {
            error!("Failed to parse .reap policy: {}", e);
            (
                StatusCode::BAD_REQUEST,
                format!("Failed to parse .reap policy: {}", e),
            )
        })?;

    info!("✓ Policy compiled successfully");

    // Set API source metadata
    enhanced_policy.set_api_source(None, Some("platform".to_string()));

    let policy_id = enhanced_policy.id;

    // Deploy to PolicyEngine
    state
        .policy_engine
        .deploy_policy(enhanced_policy.clone())
        .map_err(|e| {
            error!("Failed to deploy policy: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to deploy policy: {}", e),
            )
        })?;

    // Update metrics
    let engine_stats = state.policy_engine.get_stats();
    ACTIVE_POLICIES.set(engine_stats.total_policies as f64);

    // Invalidate cached decisions — a new compiled policy is now active.
    if let Some(ref cache) = state.decision_cache {
        cache.invalidate();
    }

    // Save to policy cache if enabled
    if let Some(ref cache) = state.policy_cache {
        if let Err(e) = cache.save_policy(&enhanced_policy).await {
            warn!("Failed to cache compiled policy {}: {}", policy_id, e);
        }
    }

    info!("✓ Policy deployed successfully: {}", policy_id);

    Ok(Json(json!({
        "status": "deployed",
        "policy_id": policy_id.to_string(),
        "policy_name": payload.policy_name,
        "version": 1,
        "deployment_time": chrono::Utc::now(),
        "message": "Policy compiled and deployed successfully"
    })))
}

/// Deploy a policy bundle (.rbb file) with version tracking.
///
/// This endpoint deploys bundles using the full ReaperDSL compiler,
/// preserving all complex conditions, functions, and rule logic.
#[instrument(skip(state, payload))]
pub async fn deploy_bundle(
    State(state): State<Arc<AgentState>>,
    Json(payload): Json<DeployBundleRequest>,
) -> Result<Json<DeployBundleResponse>, (StatusCode, String)> {
    info!(
        "Received bundle deployment request (version: {}, force: {})",
        payload.version, payload.force
    );

    // 1. Parse .rbb bundle
    let bundle = PolicyBundle::from_bytes(&payload.bundle).map_err(|e| {
        ERRORS_TOTAL.with_label_values(&["invalid_bundle"]).inc();
        error!("Failed to parse bundle: {}", e);
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid bundle format: {}", e),
        )
    })?;

    info!(
        "Bundle parsed successfully: {} (version: {}, rules: {})",
        bundle.metadata.policy_name,
        bundle
            .metadata
            .policy_version
            .as_deref()
            .unwrap_or("unknown"),
        bundle.policy.rules.len()
    );

    // 2. Deploy to PolicyEngine with compiled evaluator using the agent's DataStore
    let policy_version = state
        .policy_engine
        .deploy_bundle_with_store(bundle, state.data_store.clone(), payload.force)
        .map_err(|e| {
            ERRORS_TOTAL
                .with_label_values(&["bundle_deployment_failed"])
                .inc();
            error!("Failed to deploy bundle: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Bundle deployment failed: {}", e),
            )
        })?;

    // 3. Update metrics
    let engine_stats = state.policy_engine.get_stats();
    ACTIVE_POLICIES.set(engine_stats.total_policies as f64);

    // Invalidate cached decisions — a new bundle is now active.
    if let Some(ref cache) = state.decision_cache {
        cache.invalidate();
    }

    info!(
        "Bundle deployed successfully: policy_id={}, version={}",
        policy_version.policy_id, policy_version.version
    );

    // 4. Convert bundle_hash to hex string
    let bundle_hash_hex = policy_version
        .bundle_hash
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>();

    // 5. Return response
    let response = DeployBundleResponse {
        policy_id: policy_version.policy_id,
        version: policy_version.version,
        deployed_at: chrono::DateTime::<chrono::Utc>::from(policy_version.deployed_at).to_rfc3339(),
        bundle_hash: bundle_hash_hex,
    };

    Ok(Json(response))
}

/// Atomically load a set of bundles as the ENTIRE active policy set.
///
/// Unlike `deploy_bundle` (which upserts a single policy and leaves the rest
/// untouched), this replaces the whole set in one atomic swap: the supplied
/// bundles become the complete active state and any policy not present is
/// dropped. This is the "pure bundle load" — build the bundles that represent
/// your desired state, POST them here, and the agent's policy set matches
/// exactly, with no floating leftovers and no window where a reader sees a
/// partial set.
#[instrument(skip(state, payload))]
pub async fn load_bundles_atomic(
    State(state): State<Arc<AgentState>>,
    Json(payload): Json<crate::types::LoadBundlesRequest>,
) -> Result<Json<crate::types::LoadBundlesResponse>, (StatusCode, String)> {
    info!(
        "Atomic bundle load: {} bundle(s) -> full replace",
        payload.bundles.len()
    );

    // Parse + compile every bundle BEFORE swapping anything, so a bad bundle
    // fails the whole load and leaves the current set untouched.
    let mut policies = Vec::with_capacity(payload.bundles.len());
    for (i, bytes) in payload.bundles.iter().enumerate() {
        let bundle = PolicyBundle::from_bytes(bytes).map_err(|e| {
            ERRORS_TOTAL.with_label_values(&["invalid_bundle"]).inc();
            (StatusCode::BAD_REQUEST, format!("Bundle {i} invalid: {e}"))
        })?;
        let policy = bundle
            .to_enhanced_policy_with_store(state.data_store.clone())
            .map_err(|e| {
                ERRORS_TOTAL
                    .with_label_values(&["bundle_compile_failed"])
                    .inc();
                (
                    StatusCode::BAD_REQUEST,
                    format!("Bundle {i} failed to compile: {e}"),
                )
            })?;
        policies.push(policy);
    }

    let count = policies.len();

    // Single atomic swap of the whole set.
    state
        .policy_engine
        .replace_all_policies(policies)
        .map_err(|e| {
            ERRORS_TOTAL
                .with_label_values(&["bundle_deployment_failed"])
                .inc();
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Atomic load failed: {e}"),
            )
        })?;

    // The whole set changed — drop any cached decisions.
    if let Some(ref cache) = state.decision_cache {
        cache.invalidate();
    }
    ACTIVE_POLICIES.set(state.policy_engine.get_stats().total_policies as f64);

    Ok(Json(crate::types::LoadBundlesResponse {
        status: "loaded".to_string(),
        active_policies: count,
        deployed_at: chrono::Utc::now().to_rfc3339(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_action_parsing() {
        assert!(matches!(
            match "allow" {
                "allow" => Ok(PolicyAction::Allow),
                "deny" => Ok(PolicyAction::Deny),
                "log" => Ok(PolicyAction::Log),
                _ => Err("invalid"),
            },
            Ok(PolicyAction::Allow)
        ));
    }
}
