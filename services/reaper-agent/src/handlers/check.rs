//! Check-mode evaluation (OPA/conftest-style document validation).
//!
//! `POST /api/v1/check` evaluates a JSON document against a deployed policy's
//! deny rules and returns EVERY violation with its rendered message — the
//! gatekeeper/CI driver, distinct from the first-match sub-microsecond
//! decision endpoints. Check calls run on the AST evaluator (parse-per-call is
//! fine at CI frequency; the authorization hot path is untouched).

use axum::{extract::State, http::StatusCode, response::Json};
use policy_engine::reap::ReaperPolicy;
use policy_engine::PolicyRequest;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::instrument;
use utoipa::ToSchema;

use crate::state::AgentState;

#[derive(Debug, Deserialize, ToSchema)]
pub struct CheckRequest {
    /// Name of a deployed policy to check against.
    pub policy_name: String,
    /// The document to validate (arbitrary JSON — Terraform plan, K8s
    /// admission request, config…). Bound to `input` in the policy.
    pub input: Value,
    #[serde(default)]
    pub principal: Option<String>,
    #[serde(default = "default_action")]
    pub action: String,
    #[serde(default)]
    pub resource: Option<String>,
    #[serde(default)]
    pub context: HashMap<String, String>,
}

fn default_action() -> String {
    "check".to_string()
}

/// POST /api/v1/check
#[utoipa::path(
    post,
    path = "/api/v1/check",
    tag = "evaluation",
    request_body = CheckRequest,
    responses(
        (status = 200, description = "Document check result with all violations")
    ),
    security(("bearer_jwt" = []))
)]
#[instrument(skip(state, payload))]
pub async fn check_document(
    State(state): State<Arc<AgentState>>,
    Json(payload): Json<CheckRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let policy = state
        .policy_engine
        .get_policy_by_name(&payload.policy_name)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("policy '{}' not found", payload.policy_name),
            )
        })?;

    // Check mode always uses the AST evaluator (it supports `input` and
    // collects all violations). Parse from the stored policy content.
    let reaper_policy: ReaperPolicy = policy.content.parse().map_err(|e| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            format!(
                "policy '{}' is not a Reaper DSL policy: {e}",
                payload.policy_name
            ),
        )
    })?;
    let evaluator = reaper_policy.build_ast_evaluator(state.data_store.clone());

    let mut context = payload.context.clone();
    if let Some(ref principal) = payload.principal {
        context.insert("principal".to_string(), principal.clone());
    }
    let request = PolicyRequest {
        resource: payload
            .resource
            .clone()
            .unwrap_or_else(|| "document".to_string()),
        action: payload.action.clone(),
        context,

        ..Default::default()
    };

    let start = std::time::Instant::now();
    let result = evaluator
        .check_with_input(&request, Some(&payload.input))
        .map_err(|e| {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("check failed: {e}"),
            )
        })?;

    Ok(Json(json!({
        "policy_id": policy.id,
        "policy_name": policy.name,
        "allowed": result.allowed,
        "violations": result.violations,
        "check_time_us": start.elapsed().as_micros() as u64,
    })))
}
