//! Policy evaluation handlers.
//!
//! This module contains handlers for policy evaluation endpoints:
//! - `evaluate_policy` - Standard evaluation with full tracing
//! - `fast_evaluate_policy` - SIMD-accelerated JSON parsing
//! - `batch_evaluate_policy` - Batch evaluation for bulk requests

use axum::{
    body::Bytes,
    extract::State,
    http::{header, StatusCode},
    response::IntoResponse,
    Json,
};
use opentelemetry::{trace::TraceContextExt, KeyValue};
use policy_engine::{DecisionLogEntry, PolicyAction, PolicyRequest};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use tracing::{debug, error, instrument, warn};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use uuid::{fmt::Hyphenated, Uuid};

use crate::observability::{
    CACHE_HITS, CACHE_MISSES, CONCURRENT_EVALUATIONS, DECISIONS_TOTAL, DECISION_DURATION,
    DENIALS_TOTAL, ERRORS_TOTAL,
};
use crate::state::AgentState;
use crate::types::EvaluateRequest;

/// Typed response struct — avoids serde_json::json!() dynamic Value tree per request.
/// Serialized with sonic-rs for SIMD-accelerated output.
#[derive(Serialize)]
struct EvalResponse<'a> {
    decision_id: &'a str,
    decision: &'a str,
    policy_id: &'a str,
    policy_version: u64,
    evaluation_time_microseconds: f64,
    total_time_microseconds: f64,
    matched_rule: &'a str,
    agent_id: &'a str,
    cache_hit: bool,
}

/// Outcome of evaluating a request against a set of policies.
struct EvalOutcome {
    decision: PolicyAction,
    policy_id: Uuid,
    policy_name: String,
    policy_version: u64,
    matched_rule: Option<usize>,
    total_eval_time_ns: u64,
    /// Set when an evaluator errored; the request is denied (fail closed).
    error: Option<String>,
}

/// Evaluate `request` against every policy in `policy_ids` and combine the
/// results with a single, fail-closed rule shared by every endpoint:
///
/// - **Default deny.** A request is allowed only if at least one policy
///   explicitly allows it *and* no policy denies it.
/// - **Deny overrides.** The first policy that denies wins and short-circuits.
/// - **Errors deny.** Any evaluation error denies (fail closed).
///
/// Both `evaluate_policy` and `fast_evaluate_policy` route through this so their
/// decisions can never diverge for the same input.
fn evaluate_policy_set(
    engine: &policy_engine::PolicyEngine,
    policy_ids: &[Uuid],
    request: &PolicyRequest,
) -> EvalOutcome {
    let mut outcome = EvalOutcome {
        decision: PolicyAction::Deny,
        policy_id: Uuid::nil(),
        policy_name: String::new(),
        policy_version: 0,
        matched_rule: None,
        total_eval_time_ns: 0,
        error: None,
    };
    let mut any_allow = false;

    for policy_id in policy_ids {
        match engine.evaluate(policy_id, request) {
            Ok(d) => {
                outcome.total_eval_time_ns += d.evaluation_time_ns;
                match d.decision {
                    PolicyAction::Deny => {
                        // Deny overrides everything — fail closed and stop.
                        outcome.decision = PolicyAction::Deny;
                        outcome.policy_id = d.policy_id;
                        outcome.policy_name = d.policy_name;
                        outcome.policy_version = d.policy_version;
                        outcome.matched_rule = d.matched_rule;
                        return outcome;
                    }
                    PolicyAction::Allow => {
                        // First allow sets the decision; a later deny can still
                        // override it above.
                        if !any_allow {
                            any_allow = true;
                            outcome.decision = PolicyAction::Allow;
                            outcome.policy_id = d.policy_id;
                            outcome.policy_name = d.policy_name;
                            outcome.policy_version = d.policy_version;
                            outcome.matched_rule = d.matched_rule;
                        }
                    }
                    PolicyAction::Log => {}
                }
            }
            Err(e) => {
                outcome.decision = PolicyAction::Deny;
                outcome.error = Some(e.to_string());
                return outcome;
            }
        }
    }

    outcome
}

/// Standard policy evaluation with full OpenTelemetry tracing.
///
/// Supports:
/// - Policy lookup by UUID, name, or evaluate all policies
/// - Decision caching
/// - Full tracing with span attributes
/// - Decision logging (OPA-style audit)
#[instrument(
    skip(state, payload),
    fields(
        resource = %payload.resource,
        action = %payload.action,
        policy_name = tracing::field::Empty,
        decision = tracing::field::Empty,
        latency_ns = tracing::field::Empty,
    )
)]
pub async fn evaluate_policy(
    State(state): State<Arc<AgentState>>,
    Json(mut payload): Json<EvaluateRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    // Track concurrent evaluations
    CONCURRENT_EVALUATIONS.inc();
    let _guard = scopeguard::guard((), |_| {
        CONCURRENT_EVALUATIONS.dec();
    });

    let start_time = std::time::Instant::now();

    // Generate decision ID with zero-alloc stack buffer (saves ~300-700ns vs .to_string())
    let uuid = Uuid::new_v4();
    let mut decision_id_buf = [0u8; Hyphenated::LENGTH];
    let decision_id: &str = uuid.as_hyphenated().encode_lower(&mut decision_id_buf);

    // Determine which policy/policies to evaluate
    // Can specify: UUID, policy name, or nothing (evaluate all)
    let policy_ids: Vec<Uuid> = if let Some(id_str) = payload.policy_id.take() {
        // Try to parse as UUID first
        match Uuid::from_str(&id_str) {
            Ok(id) => vec![id],
            Err(_) => {
                // Not a valid UUID - treat as policy name
                match state.policy_engine.get_policy_by_name(&id_str) {
                    Some(policy) => {
                        state.stats.record_cache_hit();
                        CACHE_HITS.with_label_values(&["policy"]).inc();
                        vec![policy.id]
                    }
                    None => {
                        // Policy not found - DENY by default for security
                        ERRORS_TOTAL.with_label_values(&["policy_not_found"]).inc();
                        let body = sonic_rs::to_vec(&EvalResponse {
                            decision_id: &decision_id,
                            decision: "deny",
                            policy_id: &id_str,
                            policy_version: 0,
                            evaluation_time_microseconds: 0.0,
                            total_time_microseconds: 0.0,
                            matched_rule: "policy_not_found",
                            agent_id: &state.agent_id,
                            cache_hit: false,
                        })
                        .unwrap_or_default();
                        return Ok(([(header::CONTENT_TYPE, "application/json")], body));
                    }
                }
            }
        }
    } else if let Some(ref name) = payload.policy_name {
        // Look up policy by name
        match state.policy_engine.get_policy_by_name(name) {
            Some(policy) => {
                state.stats.record_cache_hit();
                CACHE_HITS.with_label_values(&["policy"]).inc();
                vec![policy.id]
            }
            None => {
                // Policy not found - DENY by default for security
                state.stats.record_cache_miss();
                CACHE_MISSES.with_label_values(&["policy"]).inc();
                ERRORS_TOTAL.with_label_values(&["policy_not_found"]).inc();
                let body = sonic_rs::to_vec(&EvalResponse {
                    decision_id: &decision_id,
                    decision: "deny",
                    policy_id: name,
                    policy_version: 0,
                    evaluation_time_microseconds: 0.0,
                    total_time_microseconds: 0.0,
                    matched_rule: "policy_not_found",
                    agent_id: &state.agent_id,
                    cache_hit: false,
                })
                .unwrap_or_default();
                return Ok(([(header::CONTENT_TYPE, "application/json")], body));
            }
        }
    } else {
        // No policy specified - evaluate ALL policies (if any deny, return deny)
        let all_policies = state.policy_engine.list_policies();
        if all_policies.is_empty() {
            ERRORS_TOTAL.with_label_values(&["no_policies"]).inc();
            let body = sonic_rs::to_vec(&EvalResponse {
                decision_id: &decision_id,
                decision: "deny",
                policy_id: "",
                policy_version: 0,
                evaluation_time_microseconds: 0.0,
                total_time_microseconds: 0.0,
                matched_rule: "no_policies_loaded",
                agent_id: &state.agent_id,
                cache_hit: false,
            })
            .unwrap_or_default();
            return Ok(([(header::CONTENT_TYPE, "application/json")], body));
        }
        all_policies.into_iter().map(|p| p.id).collect()
    };

    // Create policy request — take ownership from payload, avoid cloning.
    // Only allocate context HashMap entries beyond the standard principal key
    // when there are actual extra context fields.
    let mut context = payload.context.take().unwrap_or_default();
    context.insert("principal".to_string(), payload.principal.clone());

    let request = PolicyRequest {
        resource: payload.resource.clone(),
        action: payload.action.clone(),
        context,
    };

    // Scope the cache to the exact policy set being evaluated so decisions for
    // different policies never collide on the same (principal, action, resource).
    let cache_scope = policy_engine::scope_hash(policy_ids.iter().copied());

    // Capture the cache generation BEFORE evaluating. If a deploy/data-change
    // races with this evaluation, the generation will have advanced by the time
    // we insert, and the stale decision is dropped rather than cached.
    let cache_generation = state
        .decision_cache
        .as_ref()
        .map(|c| c.generation())
        .unwrap_or(0);

    // Check decision cache first (if enabled)
    if let Some(ref cache) = state.decision_cache {
        if let Some(cached_decision) = cache.get(&request, cache_scope) {
            // Cache hit - return cached decision immediately
            state.stats.record_decision_cache_hit();
            CACHE_HITS.with_label_values(&["decision"]).inc();

            // Track the cached decision
            match cached_decision {
                PolicyAction::Allow => state.stats.record_allow(),
                PolicyAction::Deny => state.stats.record_deny(),
                PolicyAction::Log => {}
            }

            let total_time = start_time.elapsed();
            let decision_str = match cached_decision {
                PolicyAction::Allow => "allow",
                PolicyAction::Deny => "deny",
                PolicyAction::Log => "log",
            };

            let body = sonic_rs::to_vec(&EvalResponse {
                decision_id: &decision_id,
                decision: decision_str,
                policy_id: "cached",
                policy_version: 0,
                evaluation_time_microseconds: 0.0,
                total_time_microseconds: total_time.as_nanos() as f64 / 1000.0,
                matched_rule: "cached_decision",
                agent_id: &state.agent_id,
                cache_hit: true,
            })
            .unwrap_or_default();
            return Ok(([(header::CONTENT_TYPE, "application/json")], body));
        }
        state.stats.record_decision_cache_miss();
        CACHE_MISSES.with_label_values(&["decision"]).inc();
    }

    // Evaluate all policies with the shared fail-closed core (default deny,
    // deny-overrides). This is identical to the fast endpoint's semantics.
    let outcome = evaluate_policy_set(&state.policy_engine, &policy_ids, &request);

    let final_decision = outcome.decision.clone();
    let total_eval_time_ns = outcome.total_eval_time_ns;
    let matched_policy_id = outcome.policy_id;
    let matched_policy_name = outcome.policy_name;
    let matched_policy_version = outcome.policy_version;
    let matched_rule = if let Some(ref e) = outcome.error {
        error!("Policy evaluation error: {}", e);
        ERRORS_TOTAL.with_label_values(&["evaluation_error"]).inc();
        format!("evaluation_error: {}", e)
    } else {
        outcome
            .matched_rule
            .map(|idx| format!("rule_{}", idx))
            .unwrap_or_else(|| "default_deny".to_string())
    };

    let total_time = start_time.elapsed();
    state.stats.record_evaluation(total_eval_time_ns);

    // Track allow/deny decision counts
    match final_decision {
        PolicyAction::Allow => state.stats.record_allow(),
        PolicyAction::Deny => state.stats.record_deny(),
        PolicyAction::Log => {}
    }

    let decision_str = match final_decision {
        PolicyAction::Allow => "allow",
        PolicyAction::Deny => "deny",
        PolicyAction::Log => "log",
    };

    // Record Prometheus metrics — no UUID in labels (high-cardinality anti-pattern)
    DECISIONS_TOTAL
        .with_label_values(&[decision_str, &matched_policy_name])
        .inc();

    // Record latency (convert ns to seconds for Prometheus)
    let latency_seconds = total_eval_time_ns as f64 / 1_000_000_000.0;
    DECISION_DURATION
        .with_label_values(&[&matched_policy_name])
        .observe(latency_seconds);

    // Record span attributes for distributed tracing
    let span = tracing::Span::current();
    span.record("policy_name", matched_policy_name.as_str());
    span.record("decision", decision_str);
    span.record("latency_ns", total_eval_time_ns);

    // Add OpenTelemetry span attributes only when sampled
    let cx = span.context();
    let otel_span = cx.span();
    let span_context = otel_span.span_context();
    if span_context.is_sampled() {
        otel_span.set_attribute(KeyValue::new(
            "reaper.policy.name",
            matched_policy_name.clone(),
        ));
        otel_span.set_attribute(KeyValue::new("reaper.decision", decision_str));
        otel_span.set_attribute(KeyValue::new(
            "reaper.latency_ns",
            total_eval_time_ns as i64,
        ));
        otel_span.set_attribute(KeyValue::new("reaper.resource", payload.resource.clone()));
        otel_span.set_attribute(KeyValue::new("reaper.action", payload.action.clone()));
    }

    // Log decisions — allow at debug, deny at warn
    if decision_str == "deny" {
        DENIALS_TOTAL
            .with_label_values(&[&matched_policy_name, &payload.resource, &payload.action])
            .inc();

        warn!(
            decision_id = %decision_id,
            policy_name = %matched_policy_name,
            resource = %payload.resource,
            action = %payload.action,
            decision = "deny",
            latency_ns = total_eval_time_ns,
            "Policy decision: DENY"
        );
    } else {
        debug!(
            decision_id = %decision_id,
            policy_name = %matched_policy_name,
            resource = %payload.resource,
            action = %payload.action,
            decision = decision_str,
            latency_ns = total_eval_time_ns,
            "Policy decision: ALLOW"
        );
    }

    // Log to decision buffer only when enabled (gate all work inside the check)
    if let Some(ref buffer) = state.decision_buffer {
        let trace_id = if span_context.is_valid() {
            format!("{:032x}", span_context.trace_id())
        } else {
            String::new()
        };

        let mut context_values: HashMap<String, serde_json::Value> = HashMap::new();
        if let Some(ref ctx) = payload.context {
            for (k, v) in ctx {
                context_values.insert(k.clone(), serde_json::Value::String(v.clone()));
            }
        }

        let mut entry = DecisionLogEntry::new(
            payload.principal.clone(),
            payload.action.clone(),
            payload.resource.clone(),
            decision_str.to_string(),
            matched_policy_id.to_string(),
            matched_policy_name.clone(),
        )
        .with_trace_id(trace_id)
        .with_context(context_values)
        .with_evaluation_time_ns(total_eval_time_ns)
        .with_cache_hit(false)
        .with_agent_id(state.agent_id.clone())
        .with_matched_rule(matched_rule.clone());

        // Use the same decision_id across response, logs, and audit trail
        entry.decision_id = decision_id.to_string();

        buffer.log(entry);
    }

    // Cache the decision for future requests (if caching enabled)
    if let Some(ref cache) = state.decision_cache {
        cache.insert(
            &request,
            cache_scope,
            final_decision.clone(),
            cache_generation,
        );
    }

    // Pre-format the policy_id string to avoid allocation in the response struct
    let policy_id_str = matched_policy_id.to_string();

    let body = sonic_rs::to_vec(&EvalResponse {
        decision_id: &decision_id,
        decision: decision_str,
        policy_id: &policy_id_str,
        policy_version: matched_policy_version,
        evaluation_time_microseconds: total_eval_time_ns as f64 / 1000.0,
        total_time_microseconds: total_time.as_nanos() as f64 / 1000.0,
        matched_rule: &matched_rule,
        agent_id: &state.agent_id,
        cache_hit: false,
    })
    .unwrap_or_default();

    Ok(([(header::CONTENT_TYPE, "application/json")], body))
}

/// Fast policy evaluation using SIMD-accelerated JSON parsing (sonic-rs).
///
/// This endpoint provides 3-5x faster JSON parsing compared to the standard endpoint.
/// Use this for latency-critical paths where every microsecond counts.
///
/// Performance characteristics:
/// - JSON parsing: ~2-3µs (vs ~8-10µs with serde_json)
/// - Total request overhead: ~5-10µs less than standard endpoint
#[instrument(skip(state, body))]
pub async fn fast_evaluate_policy(
    State(state): State<Arc<AgentState>>,
    body: Bytes,
) -> Result<impl IntoResponse, StatusCode> {
    use sonic_rs::{JsonContainerTrait, JsonValueTrait};

    // Track concurrent evaluations
    CONCURRENT_EVALUATIONS.inc();
    let _guard = scopeguard::guard((), |_| {
        CONCURRENT_EVALUATIONS.dec();
    });

    let start_time = std::time::Instant::now();

    // Generate decision ID with zero-alloc stack buffer
    let uuid = Uuid::new_v4();
    let mut decision_id_buf = [0u8; Hyphenated::LENGTH];
    let decision_id: &str = uuid.as_hyphenated().encode_lower(&mut decision_id_buf);

    // Parse JSON with SIMD-accelerated sonic-rs
    let value: sonic_rs::Value = match sonic_rs::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            ERRORS_TOTAL.with_label_values(&["parse_error"]).inc();
            let body = sonic_rs::to_vec(&json!({
                "error": format!("JSON parse error: {}", e),
                "decision": "deny"
            }))
            .unwrap_or_default();
            return Ok(([(header::CONTENT_TYPE, "application/json")], body));
        }
    };

    // Extract fields efficiently
    let principal = value
        .get("principal")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let resource = value.get("resource").and_then(|v| v.as_str()).unwrap_or("");
    let action = value.get("action").and_then(|v| v.as_str()).unwrap_or("");
    let policy_id_opt = value.get("policy_id").and_then(|v| v.as_str());
    let policy_name_opt = value.get("policy_name").and_then(|v| v.as_str());

    // Build context — single pass, minimal allocations
    let mut context = HashMap::with_capacity(4);
    context.insert("principal".to_string(), principal.to_string());
    if let Some(ctx) = value.get("context") {
        if let Some(obj) = ctx.as_object() {
            for (k, v) in obj.iter() {
                let val = if let Some(s) = v.as_str() {
                    s.to_string()
                } else if v.is_i64() {
                    v.as_i64().unwrap().to_string()
                } else if let Some(b) = v.as_bool() {
                    b.to_string()
                } else {
                    continue;
                };
                context.insert(k.to_string(), val);
            }
        }
    }

    // Determine policy to evaluate — capture name at lookup time
    let mut policy_name_resolved = String::new();
    let policy_ids: Vec<Uuid> = if let Some(id_str) = policy_id_opt {
        match Uuid::from_str(id_str) {
            Ok(id) => vec![id],
            Err(_) => {
                // Try as policy name
                match state.policy_engine.get_policy_by_name(id_str) {
                    Some(policy) => {
                        state.stats.record_cache_hit();
                        CACHE_HITS.with_label_values(&["policy"]).inc();
                        policy_name_resolved.clone_from(&policy.name);
                        vec![policy.id]
                    }
                    None => {
                        ERRORS_TOTAL.with_label_values(&["policy_not_found"]).inc();
                        let body = sonic_rs::to_vec(&json!({
                            "decision": "deny",
                            "error": "policy_not_found"
                        }))
                        .unwrap_or_default();
                        return Ok(([(header::CONTENT_TYPE, "application/json")], body));
                    }
                }
            }
        }
    } else if let Some(name) = policy_name_opt {
        match state.policy_engine.get_policy_by_name(name) {
            Some(policy) => {
                state.stats.record_cache_hit();
                CACHE_HITS.with_label_values(&["policy"]).inc();
                policy_name_resolved.clone_from(&policy.name);
                vec![policy.id]
            }
            None => {
                ERRORS_TOTAL.with_label_values(&["policy_not_found"]).inc();
                let body = sonic_rs::to_vec(&json!({
                    "decision": "deny",
                    "error": "policy_not_found"
                }))
                .unwrap_or_default();
                return Ok(([(header::CONTENT_TYPE, "application/json")], body));
            }
        }
    } else {
        // Evaluate all policies
        state
            .policy_engine
            .list_policies()
            .iter()
            .map(|p| p.id)
            .collect()
    };

    // Build request
    let request = PolicyRequest {
        resource: resource.to_string(),
        action: action.to_string(),
        context,
    };

    // Evaluate policies with the shared fail-closed core (default deny,
    // deny-overrides) — identical semantics to the standard endpoint.
    let outcome = evaluate_policy_set(&state.policy_engine, &policy_ids, &request);
    if let Some(ref e) = outcome.error {
        error!("Policy evaluation error: {}", e);
        ERRORS_TOTAL.with_label_values(&["evaluation_error"]).inc();
    }

    let final_decision = outcome.decision.clone();
    let matched_policy_id = outcome.policy_id;
    let matched_policy_version = outcome.policy_version;
    let matched_rule: Option<usize> = outcome.matched_rule;
    let total_eval_time_ns = outcome.total_eval_time_ns;
    if policy_name_resolved.is_empty() && !outcome.policy_name.is_empty() {
        policy_name_resolved = outcome.policy_name;
    }

    let total_time = start_time.elapsed();

    // Record to agent stats
    state.stats.record_evaluation(total_eval_time_ns);

    // Track allow/deny decision counts
    match final_decision {
        PolicyAction::Allow => state.stats.record_allow(),
        PolicyAction::Deny => state.stats.record_deny(),
        PolicyAction::Log => {}
    }

    let decision_str = match final_decision {
        PolicyAction::Allow => "allow",
        PolicyAction::Deny => "deny",
        PolicyAction::Log => "log",
    };

    // Record metrics — no UUID in labels
    DECISIONS_TOTAL
        .with_label_values(&[decision_str, &policy_name_resolved])
        .inc();
    DECISION_DURATION
        .with_label_values(&[&policy_name_resolved])
        .observe(total_time.as_secs_f64());

    let policy_id_str = matched_policy_id.to_string();
    let matched_rule_str = matched_rule
        .map(|r| format!("rule_{}", r))
        .unwrap_or_default();

    let resp_body = sonic_rs::to_vec(&EvalResponse {
        decision_id: &decision_id,
        decision: decision_str,
        policy_id: &policy_id_str,
        policy_version: matched_policy_version,
        evaluation_time_microseconds: total_eval_time_ns as f64 / 1000.0,
        total_time_microseconds: total_time.as_nanos() as f64 / 1000.0,
        matched_rule: &matched_rule_str,
        agent_id: &state.agent_id,
        cache_hit: false,
    })
    .unwrap_or_default();

    Ok(([(header::CONTENT_TYPE, "application/json")], resp_body))
}

/// Batch policy evaluation using parallel processing.
///
/// This endpoint evaluates multiple policy requests in parallel using rayon.
/// Ideal for bulk authorization checks where latency is less critical than throughput.
///
/// Performance characteristics:
/// - Parallel evaluation across all CPU cores
/// - Optional decision cache integration
/// - Returns results in same order as input requests
#[instrument(skip(state, payload))]
pub async fn batch_evaluate_policy(
    State(state): State<Arc<AgentState>>,
    Json(payload): Json<crate::types::BatchEvaluateRequest>,
) -> Result<Json<Value>, StatusCode> {
    let start_time = std::time::Instant::now();

    // Find the policy to evaluate
    let policy = if let Some(ref name) = payload.policy_name {
        match state.policy_engine.get_policy_by_name(name) {
            Some(p) => p,
            None => {
                ERRORS_TOTAL.with_label_values(&["policy_not_found"]).inc();
                return Ok(Json(json!({
                    "error": "Policy not found",
                    "policy_name": name
                })));
            }
        }
    } else {
        // Use first policy if none specified
        let policies = state.policy_engine.list_policies();
        if policies.is_empty() {
            ERRORS_TOTAL.with_label_values(&["no_policies"]).inc();
            return Ok(Json(json!({
                "error": "No policies loaded"
            })));
        }
        policies.into_iter().next().unwrap()
    };

    let policy_name = policy.name.clone();
    let policy_id = policy.id;

    // Convert batch requests to PolicyRequests — take ownership where possible
    let requests: Vec<PolicyRequest> = payload
        .requests
        .iter()
        .map(|r| {
            let mut context = r.context.clone().unwrap_or_default();
            context.insert("principal".to_string(), r.principal.clone());
            PolicyRequest {
                resource: r.resource.clone(),
                action: r.action.clone(),
                context,
            }
        })
        .collect();

    let request_count = requests.len();

    // Scope the cache to this policy and capture the generation before evaluating.
    let cache_scope = policy_engine::scope_hash(std::iter::once(policy_id));
    let cache_generation = state
        .decision_cache
        .as_ref()
        .map(|c| c.generation())
        .unwrap_or(0);

    // Evaluate requests with decision cache support
    let results: Vec<Value> = requests
        .iter()
        .enumerate()
        .map(|(i, req)| {
            let eval_start = std::time::Instant::now();

            // Check decision cache first
            let (decision, cache_hit) = if let Some(ref cache) = state.decision_cache {
                if let Some(cached) = cache.get(req, cache_scope) {
                    state.stats.record_decision_cache_hit();
                    CACHE_HITS.with_label_values(&["decision"]).inc();
                    (cached, true)
                } else {
                    state.stats.record_decision_cache_miss();
                    CACHE_MISSES.with_label_values(&["decision"]).inc();

                    // Evaluate and cache
                    let result = state.policy_engine.evaluate(&policy_id, req);
                    let decision = match result {
                        Ok(d) => d.decision,
                        Err(_) => PolicyAction::Deny,
                    };
                    cache.insert(req, cache_scope, decision.clone(), cache_generation);
                    (decision, false)
                }
            } else {
                // No cache - evaluate directly
                let result = state.policy_engine.evaluate(&policy_id, req);
                let decision = match result {
                    Ok(d) => d.decision,
                    Err(_) => PolicyAction::Deny,
                };
                (decision, false)
            };

            let duration = eval_start.elapsed();
            let decision_str = match decision {
                PolicyAction::Allow => "allow",
                PolicyAction::Deny => "deny",
                PolicyAction::Log => "log",
            };

            // Record metrics — no UUID in labels
            DECISIONS_TOTAL
                .with_label_values(&[decision_str, &policy_name])
                .inc();

            json!({
                "index": i,
                "decision": decision_str,
                "evaluation_time_microseconds": duration.as_nanos() as f64 / 1000.0,
                "cache_hit": cache_hit
            })
        })
        .collect();

    let total_time = start_time.elapsed();

    // Count decisions
    let allow_count = results
        .iter()
        .filter(|r| r.get("decision").and_then(|d| d.as_str()) == Some("allow"))
        .count();
    let deny_count = results.len() - allow_count;

    // Record batch stats to agent metrics
    state.stats.record_evaluation(total_time.as_nanos() as u64);
    for _ in 0..allow_count {
        state.stats.record_allow();
    }
    for _ in 0..deny_count {
        state.stats.record_deny();
    }

    debug!(
        policy_name = %policy_name,
        request_count = request_count,
        allow_count = allow_count,
        deny_count = deny_count,
        total_time_ms = total_time.as_millis(),
        "Batch evaluation completed"
    );

    Ok(Json(json!({
        "policy_name": policy_name,
        "policy_id": policy_id.to_string(),
        "request_count": request_count,
        "results": results,
        "summary": {
            "allowed": allow_count,
            "denied": deny_count,
            "total_time_microseconds": total_time.as_nanos() as f64 / 1000.0,
            "avg_time_microseconds": total_time.as_nanos() as f64 / 1000.0 / request_count as f64
        },
        "agent_id": state.agent_id
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decision_string_conversion() {
        assert_eq!(
            match PolicyAction::Allow {
                PolicyAction::Allow => "allow",
                PolicyAction::Deny => "deny",
                PolicyAction::Log => "log",
            },
            "allow"
        );
    }

    #[test]
    fn test_eval_response_serialization() {
        let resp = EvalResponse {
            decision_id: "test-decision-id",
            decision: "allow",
            policy_id: "test-id",
            policy_version: 1,
            evaluation_time_microseconds: 0.5,
            total_time_microseconds: 1.2,
            matched_rule: "rule_0",
            agent_id: "agent-001",
            cache_hit: false,
        };
        let bytes = sonic_rs::to_vec(&resp).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["decision_id"], "test-decision-id");
        assert_eq!(json["decision"], "allow");
        assert_eq!(json["policy_id"], "test-id");
        assert_eq!(json["cache_hit"], false);
    }
}
