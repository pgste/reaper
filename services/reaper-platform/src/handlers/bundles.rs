//! Bundle management handlers.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use chrono::Utc;
use policy_engine::{
    reap::{Decision, Policy, ReapCondition, ReapRule},
    PolicyAction, PolicyBundle,
};
use std::str::FromStr;
use std::sync::Arc;
use tracing::{info, instrument, warn};
use uuid::Uuid;

use crate::state::PlatformState;
use crate::types::{
    BundleResponse, CreateBundleRequest, DeployBundleToAgentsRequest, DeployBundleToAgentsResponse,
};

/// Create a .rbb bundle from a policy
#[instrument(skip(state))]
pub async fn create_bundle(
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
            message: None,
        })
        .collect();

    let reap_policy = Policy {
        name: policy.name.clone(),
        metadata,
        default_decision: Decision::Deny,
        rules: reap_rules,
        // Platform-converted Simple policies never carry helper predicates
        // or imports (language-v3 .reap source constructs).
        functions: vec![],
        imports: vec![],
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
pub async fn get_bundle(
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
pub async fn deploy_bundle_to_agents(
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
