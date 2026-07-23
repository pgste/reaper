//! Kubernetes admission webhook endpoint (R4-01 step 6b).
//!
//! `POST /api/v1/admission/{policy}` accepts a native Kubernetes
//! `AdmissionReview` (admission.k8s.io/v1), binds the WHOLE review body to
//! `input` (the OPA convention — policies read
//! `input.request.object.spec...`), maps the review's coordinates onto the
//! policy request (operation → action, resource → resource, userInfo.username
//! → principal), runs the check driver against the named deployed policy, and
//! answers with a well-formed `AdmissionReview` response: uid echoed,
//! `allowed`, and every violation message joined into `status.message`. A
//! `ValidatingWebhookConfiguration` can point at this route directly — no
//! adapter shim.
//!
//! Failure posture (see docs/deployment/ADMISSION_WEBHOOK.md):
//! - A parseable review that cannot be evaluated (policy missing, non-DSL
//!   policy, evaluation error) is answered `allowed: false` with the reason in
//!   `status.message` — the webhook itself fails CLOSED, independent of the
//!   webhook configuration's `failurePolicy`.
//! - Only a request that is not an AdmissionReview at all (no `request.uid`)
//!   gets a non-200, because without a uid no well-formed response exists;
//!   `failurePolicy` governs that case.

use axum::extract::Path;
use axum::{extract::State, http::StatusCode, response::Json};
use policy_engine::PolicyRequest;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::instrument;

use crate::state::AgentState;

/// The only AdmissionReview version served. v1beta1 was removed in
/// Kubernetes 1.22; answering an unknown version with a v1 body would be
/// rejected by the API server anyway, so mismatches fail closed at 400.
const ADMISSION_API_VERSION: &str = "admission.k8s.io/v1";

/// Build the AdmissionReview response envelope.
fn review_response(uid: &str, allowed: bool, code: u16, message: Option<String>) -> Value {
    let mut response = json!({
        "uid": uid,
        "allowed": allowed,
    });
    if let Some(message) = message {
        response["status"] = json!({ "code": code, "message": message });
    }
    json!({
        "apiVersion": ADMISSION_API_VERSION,
        "kind": "AdmissionReview",
        "response": response,
    })
}

/// POST /api/v1/admission/{policy}
#[utoipa::path(
    post,
    path = "/api/v1/admission/{policy}",
    tag = "evaluation",
    params(("policy" = String, Path, description = "Name of the deployed policy to validate against")),
    request_body(description = "Kubernetes AdmissionReview (admission.k8s.io/v1)",
                  content = serde_json::Value),
    responses(
        (status = 200, description = "AdmissionReview response (uid echoed, allowed, \
                                      violation messages in status.message; evaluation \
                                      problems answer allowed=false, never 5xx)"),
        (status = 400, description = "Body is not an AdmissionReview with request.uid")
    ),
    security(("bearer_jwt" = []))
)]
#[instrument(skip(state, review), fields(policy = %policy_name))]
pub async fn admission_review(
    State(state): State<Arc<AgentState>>,
    Path(policy_name): Path<String>,
    Json(review): Json<Value>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let request = &review["request"];
    let uid = request["uid"]
        .as_str()
        .filter(|u| !u.is_empty())
        .ok_or((
            StatusCode::BAD_REQUEST,
            "body is not an AdmissionReview: request.uid missing".to_string(),
        ))?
        .to_string();

    if let Some(version) = review["apiVersion"].as_str() {
        if version != ADMISSION_API_VERSION {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("unsupported AdmissionReview apiVersion '{version}' (expected {ADMISSION_API_VERSION})"),
            ));
        }
    }

    // Coordinates: operation → action, resource (plural, e.g. "pods") →
    // resource, requesting user → principal. All optional — the k8s library
    // policies decide on the object alone — but populated so policies CAN
    // discriminate on them (e.g. `action == "delete"`).
    let action = request["operation"]
        .as_str()
        .map(str::to_lowercase)
        .unwrap_or_else(|| "unknown".to_string());
    let resource = request["resource"]["resource"]
        .as_str()
        .or_else(|| request["kind"]["kind"].as_str())
        .map(str::to_lowercase)
        .unwrap_or_else(|| "admission".to_string());

    let mut context = HashMap::new();
    if let Some(username) = request["userInfo"]["username"].as_str() {
        context.insert("principal".to_string(), username.to_string());
    }
    if let Some(namespace) = request["namespace"].as_str() {
        context.insert("namespace".to_string(), namespace.to_string());
    }

    let policy_request = PolicyRequest {
        resource,
        action,
        context,

        ..Default::default()
    };

    // Fail-closed evaluation: any problem past this point denies admission
    // with the reason in status.message instead of surfacing a 5xx.
    let check = state
        .policy_engine
        .get_policy_by_name(&policy_name)
        .ok_or_else(|| format!("policy '{policy_name}' is not deployed on this agent"))
        .and_then(|policy| {
            policy
                .get_evaluator()
                .map_err(|e| format!("policy '{policy_name}' has no evaluator: {e}"))
                .and_then(|evaluator| {
                    evaluator
                        .check_with_input(&policy_request, Some(&review))
                        .map_err(|e| format!("admission check failed: {e}"))
                })
        });

    let body = match check {
        Ok(result) if result.allowed => review_response(&uid, true, 200, None),
        Ok(result) => {
            let message = if result.violations.is_empty() {
                format!("denied by policy '{policy_name}'")
            } else {
                result
                    .violations
                    .iter()
                    .map(|v| v.message.clone().unwrap_or_else(|| v.rule.clone()))
                    .collect::<Vec<_>>()
                    .join("; ")
            };
            review_response(&uid, false, 403, Some(message))
        }
        Err(reason) => {
            tracing::warn!(%reason, "admission review failed closed");
            review_response(&uid, false, 500, Some(format!("reaper: {reason}")))
        }
    };

    Ok(Json(body))
}
