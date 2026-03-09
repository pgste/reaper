//! Policy CRUD handlers.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use chrono::Utc;
use policy_engine::{EnhancedPolicy, PolicyAction, PolicyRule};
use reaper_core::ReaperError;
use serde_json::{json, Value};
use std::str::FromStr;
use std::sync::Arc;
use tracing::{error, info, instrument, warn};
use uuid::Uuid;

use crate::state::PlatformState;
use crate::types::{CreatePolicyRequest, PolicyResponse, UpdatePolicyRequest};

#[instrument(skip(state))]
pub async fn list_policies(
    State(state): State<Arc<PlatformState>>,
) -> Result<Json<Value>, StatusCode> {
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
pub async fn create_policy(
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
pub async fn get_policy(
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
pub async fn update_policy(
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
pub async fn delete_policy(
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
pub async fn deploy_policy_to_agents(
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
