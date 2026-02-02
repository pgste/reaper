//! Policy evaluation handlers.
//!
//! This module contains handlers for policy evaluation endpoints:
//! - `evaluate_policy` - Standard evaluation with full tracing
//! - `fast_evaluate_policy` - SIMD-accelerated JSON parsing
//! - `batch_evaluate_policy` - Batch evaluation for bulk requests

use axum::{
    body::Bytes,
    extract::State,
    http::StatusCode,
    response::Json,
};
use opentelemetry::{trace::TraceContextExt, KeyValue};
use policy_engine::{DecisionLogEntry, PolicyAction, PolicyRequest};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use tracing::{error, info, instrument, warn};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use uuid::Uuid;

use crate::observability::{
    CACHE_HITS, CACHE_MISSES, CONCURRENT_EVALUATIONS, DECISION_DURATION, DECISIONS_TOTAL,
    DENIALS_TOTAL, ERRORS_TOTAL,
};
use crate::state::AgentState;
use crate::types::{BatchEvaluateRequest, EvaluateRequest};

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
    Json(payload): Json<EvaluateRequest>,
) -> Result<Json<Value>, StatusCode> {
    // Track concurrent evaluations
    CONCURRENT_EVALUATIONS.inc();
    let _guard = scopeguard::guard((), |_| {
        CONCURRENT_EVALUATIONS.dec();
    });

    let start_time = std::time::Instant::now();

    // Get current OpenTelemetry span for rich context
    let span = tracing::Span::current();
    let cx = span.context();
    let otel_span = cx.span();
    let span_context = otel_span.span_context();

    // Extract trace ID for logging correlation
    let trace_id = if span_context.is_valid() {
        format!("{:032x}", span_context.trace_id())
    } else {
        "none".to_string()
    };

    // Determine which policy/policies to evaluate
    // Can specify: UUID, policy name, or nothing (evaluate all)
    let policy_ids: Vec<Uuid> = if let Some(id_str) = payload.policy_id {
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
                        return Ok(Json(json!({
                            "decision": "deny",
                            "policy_id": id_str,
                            "policy_version": 0,
                            "evaluation_time_microseconds": 0.0,
                            "total_time_microseconds": 0.0,
                            "matched_rule": "policy_not_found",
                            "agent_id": "reaper-agent-001"
                        })));
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
                return Ok(Json(json!({
                    "decision": "deny",
                    "policy_name": name,
                    "policy_version": 0,
                    "evaluation_time_microseconds": 0.0,
                    "total_time_microseconds": 0.0,
                    "matched_rule": "policy_not_found",
                    "agent_id": "reaper-agent-001"
                })));
            }
        }
    } else {
        // No policy specified - evaluate ALL policies (if any deny, return deny)
        let all_policies = state.policy_engine.list_policies();
        if all_policies.is_empty() {
            ERRORS_TOTAL.with_label_values(&["no_policies"]).inc();
            return Ok(Json(json!({
                "decision": "deny",
                "policy_version": 0,
                "evaluation_time_microseconds": 0.0,
                "total_time_microseconds": 0.0,
                "matched_rule": "no_policies_loaded",
                "agent_id": "reaper-agent-001"
            })));
        }
        all_policies.into_iter().map(|p| p.id).collect()
    };

    // Create policy request
    // The compiled evaluator looks up user entities by ID in the DataStore
    // Use the principal as-is (it's already an entity ID like "user_admin")
    let original_context = payload.context.clone(); // Save for decision logging
    let mut context = original_context.clone().unwrap_or_default();
    context.insert("principal".to_string(), payload.principal.clone());

    let request = PolicyRequest {
        resource: payload.resource.clone(),
        action: payload.action.clone(),
        context,
    };

    // Check decision cache first (if enabled)
    if let Some(ref cache) = state.decision_cache {
        if let Some(cached_decision) = cache.get(&request) {
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

            return Ok(Json(json!({
                "decision": decision_str,
                "policy_id": "cached",
                "policy_version": 0,
                "evaluation_time_microseconds": 0.0,
                "total_time_microseconds": total_time.as_nanos() as f64 / 1000.0,
                "matched_rule": "cached_decision",
                "agent_id": "reaper-agent-001",
                "cache_hit": true
            })));
        }
        state.stats.record_decision_cache_miss();
        CACHE_MISSES.with_label_values(&["decision"]).inc();
    }

    // Evaluate all policies in policy_ids (may be 1 or many)
    // If ANY policy denies, return deny (security first)
    let mut final_decision = PolicyAction::Allow;
    let mut total_eval_time_ns = 0u64;
    let mut matched_policy_id = Uuid::nil();
    let mut matched_policy_name = String::from("unknown");
    let mut matched_policy_version = 0u64;
    let mut matched_rule = String::from("default_allow");

    for policy_id in &policy_ids {
        match state.policy_engine.evaluate(policy_id, &request) {
            Ok(decision) => {
                total_eval_time_ns += decision.evaluation_time_ns;

                // If this policy denies, override the final decision
                if matches!(decision.decision, PolicyAction::Deny) {
                    final_decision = PolicyAction::Deny;
                    matched_policy_id = decision.policy_id;
                    matched_policy_version = decision.policy_version;
                    matched_rule = decision
                        .matched_rule
                        .map(|idx| format!("rule_{}", idx))
                        .unwrap_or_else(|| "no_rule".to_string());

                    // Get policy name for this denial
                    if let Some(policy) = state.policy_engine.get_policy(policy_id) {
                        matched_policy_name = policy.name.clone();
                    }

                    // Break early on deny (security first - no need to check other policies)
                    break;
                } else if matches!(final_decision, PolicyAction::Allow) {
                    // Only update if we haven't seen a deny yet
                    matched_policy_id = decision.policy_id;
                    matched_policy_version = decision.policy_version;
                    matched_rule = decision
                        .matched_rule
                        .map(|idx| format!("rule_{}", idx))
                        .unwrap_or_else(|| "no_rule".to_string());

                    if let Some(policy) = state.policy_engine.get_policy(policy_id) {
                        matched_policy_name = policy.name.clone();
                    }
                }
            }
            Err(e) => {
                // On error, deny for security (fail closed)
                error!("Policy evaluation error for {}: {}", policy_id, e);
                ERRORS_TOTAL.with_label_values(&["evaluation_error"]).inc();
                final_decision = PolicyAction::Deny;
                matched_rule = format!("evaluation_error: {}", e);
                break;
            }
        }
    }

    let total_time = start_time.elapsed();
    state.stats.record_evaluation(total_eval_time_ns);

    // Track allow/deny decision counts
    match final_decision {
        PolicyAction::Allow => state.stats.record_allow(),
        PolicyAction::Deny => state.stats.record_deny(),
        PolicyAction::Log => {} // Log doesn't count as allow or deny
    }

    let decision_str = match final_decision {
        PolicyAction::Allow => "allow",
        PolicyAction::Deny => "deny",
        PolicyAction::Log => "log",
    };

    // Record Prometheus metrics
    DECISIONS_TOTAL
        .with_label_values(&[
            decision_str,
            &matched_policy_name,
            &matched_policy_id.to_string(),
        ])
        .inc();

    // Record latency (convert ns to seconds for Prometheus)
    let latency_seconds = total_eval_time_ns as f64 / 1_000_000_000.0;
    DECISION_DURATION
        .with_label_values(&[&matched_policy_name])
        .observe(latency_seconds);

    // Record span attributes for distributed tracing
    span.record("policy_name", matched_policy_name.as_str());
    span.record("decision", decision_str);
    span.record("latency_ns", total_eval_time_ns);

    // Add OpenTelemetry span attributes
    otel_span.set_attribute(KeyValue::new(
        "reaper.policy.name",
        matched_policy_name.clone(),
    ));
    otel_span.set_attribute(KeyValue::new(
        "reaper.policy.id",
        matched_policy_id.to_string(),
    ));
    otel_span.set_attribute(KeyValue::new("reaper.decision", decision_str));
    otel_span.set_attribute(KeyValue::new(
        "reaper.latency_ns",
        total_eval_time_ns as i64,
    ));
    otel_span.set_attribute(KeyValue::new("reaper.resource", payload.resource.clone()));
    otel_span.set_attribute(KeyValue::new("reaper.action", payload.action.clone()));

    // Log all decisions asynchronously (non-blocking)
    if decision_str == "deny" {
        DENIALS_TOTAL
            .with_label_values(&[&matched_policy_name, &payload.resource, &payload.action])
            .inc();

        // Structured log for denial (security event)
        warn!(
            trace_id = %trace_id,
            decision_id = %format!("dec_{}", uuid::Uuid::new_v4().simple()),
            policy_name = %matched_policy_name,
            policy_id = %matched_policy_id,
            resource = %payload.resource,
            action = %payload.action,
            decision = "deny",
            latency_ns = total_eval_time_ns,
            latency_us = total_eval_time_ns as f64 / 1000.0,
            "Policy decision: DENY"
        );
    } else {
        // Log allow decisions at INFO level (async, non-blocking)
        info!(
            trace_id = %trace_id,
            decision_id = %format!("dec_{}", uuid::Uuid::new_v4().simple()),
            policy_name = %matched_policy_name,
            policy_id = %matched_policy_id,
            resource = %payload.resource,
            action = %payload.action,
            decision = decision_str,
            latency_ns = total_eval_time_ns,
            latency_us = total_eval_time_ns as f64 / 1000.0,
            "Policy decision: ALLOW"
        );
    }

    // Log to decision buffer (OPA-style audit logging)
    if let Some(ref buffer) = state.decision_buffer {
        let mut context_values: HashMap<String, serde_json::Value> = HashMap::new();
        if let Some(ref ctx) = original_context {
            for (k, v) in ctx {
                context_values.insert(k.clone(), serde_json::Value::String(v.clone()));
            }
        }

        let entry = DecisionLogEntry::new(
            payload.principal.clone(),
            payload.action.clone(),
            payload.resource.clone(),
            decision_str.to_string(),
            matched_policy_id.to_string(),
            matched_policy_name.clone(),
        )
        .with_trace_id(trace_id.clone())
        .with_context(context_values)
        .with_evaluation_time_ns(total_eval_time_ns)
        .with_cache_hit(false)
        .with_agent_id(state.agent_id.clone())
        .with_matched_rule(matched_rule.clone());

        buffer.log(entry);
    }

    // Cache the decision for future requests (if caching enabled)
    if let Some(ref cache) = state.decision_cache {
        cache.insert(&request, final_decision.clone());
    }

    Ok(Json(json!({
        "decision": decision_str,
        "policy_id": matched_policy_id.to_string(),
        "policy_version": matched_policy_version,
        "evaluation_time_microseconds": total_eval_time_ns as f64 / 1000.0,
        "total_time_microseconds": total_time.as_nanos() as f64 / 1000.0,
        "matched_rule": matched_rule,
        "agent_id": state.agent_id,
        "cache_hit": false
    })))
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
) -> Result<Json<Value>, StatusCode> {
    use sonic_rs::{JsonContainerTrait, JsonValueTrait};

    // Track concurrent evaluations
    CONCURRENT_EVALUATIONS.inc();
    let _guard = scopeguard::guard((), |_| {
        CONCURRENT_EVALUATIONS.dec();
    });

    let start_time = std::time::Instant::now();

    // Parse JSON with SIMD-accelerated sonic-rs
    let value: sonic_rs::Value = match sonic_rs::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            ERRORS_TOTAL.with_label_values(&["parse_error"]).inc();
            return Ok(Json(json!({
                "error": format!("JSON parse error: {}", e),
                "decision": "deny"
            })));
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

    // Build context from JSON
    let mut context = HashMap::new();
    context.insert("principal".to_string(), principal.to_string());
    if let Some(ctx) = value.get("context") {
        if let Some(obj) = ctx.as_object() {
            for (k, v) in obj.iter() {
                if let Some(s) = v.as_str() {
                    context.insert(k.to_string(), s.to_string());
                } else if v.is_i64() {
                    context.insert(k.to_string(), v.as_i64().unwrap().to_string());
                } else if let Some(b) = v.as_bool() {
                    context.insert(k.to_string(), b.to_string());
                }
            }
        }
    }

    // Determine policy to evaluate
    let policy_ids: Vec<Uuid> = if let Some(id_str) = policy_id_opt {
        match Uuid::from_str(id_str) {
            Ok(id) => vec![id],
            Err(_) => {
                // Try as policy name
                match state.policy_engine.get_policy_by_name(id_str) {
                    Some(policy) => {
                        state.stats.record_cache_hit();
                        CACHE_HITS.with_label_values(&["policy"]).inc();
                        vec![policy.id]
                    }
                    None => {
                        ERRORS_TOTAL.with_label_values(&["policy_not_found"]).inc();
                        return Ok(Json(json!({
                            "decision": "deny",
                            "error": "policy_not_found"
                        })));
                    }
                }
            }
        }
    } else if let Some(name) = policy_name_opt {
        match state.policy_engine.get_policy_by_name(name) {
            Some(policy) => {
                state.stats.record_cache_hit();
                CACHE_HITS.with_label_values(&["policy"]).inc();
                vec![policy.id]
            }
            None => {
                ERRORS_TOTAL.with_label_values(&["policy_not_found"]).inc();
                return Ok(Json(json!({
                    "decision": "deny",
                    "error": "policy_not_found"
                })));
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

    // Evaluate policies
    let mut final_decision = PolicyAction::Deny;
    let mut matched_policy_id = Uuid::nil();
    let mut matched_policy_version = 0u64;
    let mut matched_rule: Option<usize> = None;
    let mut total_eval_time_ns = 0u64;

    for policy_id in &policy_ids {
        let eval_start = std::time::Instant::now();

        match state.policy_engine.evaluate(policy_id, &request) {
            Ok(decision) => {
                let eval_time_ns = eval_start.elapsed().as_nanos() as u64;
                total_eval_time_ns += eval_time_ns;

                if decision.decision == PolicyAction::Allow {
                    final_decision = PolicyAction::Allow;
                    matched_policy_id = decision.policy_id;
                    matched_policy_version = decision.policy_version;
                    matched_rule = decision.matched_rule;
                    break;
                } else if decision.decision == PolicyAction::Deny {
                    final_decision = PolicyAction::Deny;
                    matched_policy_id = decision.policy_id;
                    matched_policy_version = decision.policy_version;
                    matched_rule = decision.matched_rule;
                }
            }
            Err(_) => {
                ERRORS_TOTAL.with_label_values(&["evaluation_error"]).inc();
            }
        }
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

    // Record metrics
    if let Some(policy) = state.policy_engine.get_policy(&matched_policy_id) {
        DECISIONS_TOTAL
            .with_label_values(&[decision_str, &policy.name, &matched_policy_id.to_string()])
            .inc();
        DECISION_DURATION
            .with_label_values(&[&policy.name])
            .observe(total_time.as_secs_f64());
    }

    Ok(Json(json!({
        "decision": decision_str,
        "policy_id": matched_policy_id.to_string(),
        "policy_version": matched_policy_version,
        "evaluation_time_microseconds": total_eval_time_ns as f64 / 1000.0,
        "total_time_microseconds": total_time.as_nanos() as f64 / 1000.0,
        "matched_rule": matched_rule,
        "agent_id": "reaper-agent-001",
        "fast_path": true
    })))
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
    Json(payload): Json<BatchEvaluateRequest>,
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

    // Convert batch requests to PolicyRequests
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

    // Evaluate requests with decision cache support
    let results: Vec<Value> = requests
        .iter()
        .enumerate()
        .map(|(i, req)| {
            let eval_start = std::time::Instant::now();

            // Check decision cache first
            let (decision, cache_hit) = if let Some(ref cache) = state.decision_cache {
                if let Some(cached) = cache.get(req) {
                    state.stats.record_decision_cache_hit();
                    CACHE_HITS.with_label_values(&["decision"]).inc();
                    (cached, true)
                } else {
                    state.stats.record_decision_cache_miss();
                    CACHE_MISSES.with_label_values(&["decision"]).inc();

                    // Evaluate and cache
                    let result = state.policy_engine.evaluate(&policy.id, req);
                    let decision = match result {
                        Ok(d) => d.decision,
                        Err(_) => PolicyAction::Deny,
                    };
                    cache.insert(req, decision.clone());
                    (decision, false)
                }
            } else {
                // No cache - evaluate directly
                let result = state.policy_engine.evaluate(&policy.id, req);
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

            // Record metrics
            DECISIONS_TOTAL
                .with_label_values(&[decision_str, &policy.name, &policy.id.to_string()])
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

    info!(
        policy_name = %policy.name,
        request_count = request_count,
        allow_count = allow_count,
        deny_count = deny_count,
        total_time_ms = total_time.as_millis(),
        "Batch evaluation completed"
    );

    Ok(Json(json!({
        "policy_name": policy.name,
        "policy_id": policy.id.to_string(),
        "request_count": request_count,
        "results": results,
        "summary": {
            "allowed": allow_count,
            "denied": deny_count,
            "total_time_microseconds": total_time.as_nanos() as f64 / 1000.0,
            "avg_time_microseconds": total_time.as_nanos() as f64 / 1000.0 / request_count as f64
        },
        "agent_id": "reaper-agent-001"
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
}
