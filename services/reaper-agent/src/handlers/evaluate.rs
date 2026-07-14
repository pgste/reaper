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
    CACHE_HITS, CACHE_MISSES, CONCURRENT_EVALUATIONS, DENIALS_TOTAL, ERRORS_TOTAL,
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

/// Build the "explain" input-data snapshot: the resolved principal/resource
/// entity attributes the decision branched on, as
/// `{"principal": {...}, "resource": {...}}`. Read-only DataStore lookups on the
/// LOG path (never the eval loop); returns `None` if neither is a known entity.
fn capture_input_data(
    data_store: &policy_engine::DataStore,
    principal: &str,
    resource: &str,
) -> Option<Value> {
    let mut input = serde_json::Map::new();
    if let Some(p) = data_store.entity_attributes_json(principal) {
        input.insert("principal".to_string(), p);
    }
    if let Some(r) = data_store.entity_attributes_json(resource) {
        input.insert("resource".to_string(), r);
    }
    (!input.is_empty()).then_some(Value::Object(input))
}

/// Outcome of evaluating a request against a set of policies — the engine's
/// [`policy_engine::SetEvalOutcome`], re-exported under the handler-local name
/// the endpoints already use.
type EvalOutcome = policy_engine::SetEvalOutcome;

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
    // The combination semantics live in the ENGINE (PolicyEngine::evaluate_set)
    // as the single source of truth, shared with the control plane's
    // counterfactual replay engine — the two can never diverge.
    engine.evaluate_set(policy_ids, request)
}

/// Standard policy evaluation.
///
/// Supports:
/// - Policy lookup by UUID, name, or evaluate all policies
/// - Decision caching
/// - OpenTelemetry attributes when the trace is sampled
/// - Decision logging (OPA-style audit)
///
/// Note: no `#[instrument]` — building a span with `%`-formatted fields on every
/// request costs ~200-800ns, which is a large fraction of the sub-µs budget (the
/// engine dropped it for the same reason). Tracing attributes are attached to
/// the ambient span only when the trace is actually sampled.
/// Mandatory-audit fail-closed gate (Plan 04 step 4). Once the durable audit
/// trail is compromised (a record was lost from the durable sink in mandatory
/// mode), the agent must not serve further decisions — they would be
/// un-audited. Returns `503` so callers fail closed; readiness also flips
/// not-ready so load balancers drain the instance.
#[inline]
fn audit_gate(state: &AgentState) -> Result<(), StatusCode> {
    if let Some(ref buffer) = state.decision_buffer {
        if buffer.audit_required() && !buffer.is_audit_healthy() {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
    }
    Ok(())
}

/// Observe request-total latency for an early-return response (deny or 503)
/// into the SLA histogram (`reaper_decision_duration_seconds`).
///
/// Early-return denies — `data_stale`, `policy_not_found`,
/// `evaluate_all_disabled`, `no_policies_loaded`, `candidate_cap_exceeded`,
/// fast-path `parse_error`, and the audit-gate 503 — are SERVED requests. If
/// they skip the histogram, a denial storm (stale-data gate tripped,
/// misconfig, attack) leaves the request-total latency series silent while the
/// agent answers at line rate, hiding the incident from the SLA dashboard
/// (PERF R2-P2-3). The reason is already counted by `ERRORS_TOTAL`; here a
/// single constant policy label keeps the histogram's cardinality bounded (the
/// same pattern as the "cached" label on the cache-hit path).
#[inline]
fn observe_early_return(state: &AgentState, start_time: std::time::Instant) {
    state
        .decision_metrics
        .for_policy("early_deny")
        .duration
        .observe(start_time.elapsed().as_secs_f64());
}

#[utoipa::path(
    post,
    path = "/api/v1/messages",
    tag = "evaluation",
    responses(
        (status = 200, description = "Policy decision")
    ),
    security(("bearer_jwt" = []))
)]
pub async fn evaluate_policy(
    State(state): State<Arc<AgentState>>,
    Json(mut payload): Json<EvaluateRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    // Start the request-total clock before the FIRST possible return so every
    // served response — success, deny, or fail-closed 503 — is observed.
    let start_time = std::time::Instant::now();

    if let Err(status) = audit_gate(&state) {
        observe_early_return(&state, start_time);
        return Err(status);
    }
    // Track concurrent evaluations
    CONCURRENT_EVALUATIONS.inc();
    let _guard = scopeguard::guard((), |_| {
        CONCURRENT_EVALUATIONS.dec();
    });

    // Generate decision ID with zero-alloc stack buffer (saves ~300-700ns vs .to_string())
    let uuid = Uuid::new_v4();
    let mut decision_id_buf = [0u8; Hyphenated::LENGTH];
    let decision_id: &str = uuid.as_hyphenated().encode_lower(&mut decision_id_buf);

    // DATA-PLANE GUARD: fail CLOSED when the operator armed a gate and it
    // is tripped — either the staleness budget is exceeded in enforce mode
    // (stale data must not mint allows) or REAPER_DATA_REQUIRE_SYNC is set
    // and the first verified snapshot hasn't landed yet (an empty replica
    // must not answer as if it had data). matched_rule names which gate.
    // Two relaxed atomic loads on the hot path; zero cost when unarmed.
    if let Some(reason) = state.data_sync.deny_reason() {
        ERRORS_TOTAL.with_label_values(&["data_stale"]).inc();
        let body = sonic_rs::to_vec(&EvalResponse {
            decision_id,
            decision: "deny",
            policy_id: "",
            policy_version: 0,
            evaluation_time_microseconds: 0.0,
            total_time_microseconds: 0.0,
            matched_rule: reason,
            agent_id: &state.agent_id,
            cache_hit: false,
        })
        .unwrap_or_default();
        observe_early_return(&state, start_time);
        return Ok(([(header::CONTENT_TYPE, "application/json")], body));
    }

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
                            decision_id,
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
                        observe_early_return(&state, start_time);
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
                    decision_id,
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
                observe_early_return(&state, start_time);
                return Ok(([(header::CONTENT_TYPE, "application/json")], body));
            }
        }
    } else {
        // No policy specified - evaluate-all path (Plan 08 Phase A).
        let perf = &state.agent_config.performance;

        // ADR-2: evaluate-all is opt-in. A policy-less request fanning out to
        // every policy is a DoS amplifier, so it fails closed unless armed.
        if !perf.allow_evaluate_all {
            ERRORS_TOTAL
                .with_label_values(&["evaluate_all_disabled"])
                .inc();
            let body = sonic_rs::to_vec(&EvalResponse {
                decision_id,
                decision: "deny",
                policy_id: "",
                policy_version: 0,
                evaluation_time_microseconds: 0.0,
                total_time_microseconds: 0.0,
                matched_rule: "evaluate_all_disabled",
                agent_id: &state.agent_id,
                cache_hit: false,
            })
            .unwrap_or_default();
            observe_early_return(&state, start_time);
            return Ok(([(header::CONTENT_TYPE, "application/json")], body));
        }

        // Prune to candidate policies for this resource instead of cloning the
        // whole set (ADR-1). The linear scan remains available as the fallback.
        let candidate_ids: Vec<Uuid> = if perf.use_pruning_index {
            state.policy_engine.candidate_policy_ids(&payload.resource)
        } else {
            state
                .policy_engine
                .list_policies()
                .into_iter()
                .map(|p| p.id)
                .collect()
        };

        if candidate_ids.is_empty() {
            ERRORS_TOTAL.with_label_values(&["no_policies"]).inc();
            let body = sonic_rs::to_vec(&EvalResponse {
                decision_id,
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
            observe_early_return(&state, start_time);
            return Ok(([(header::CONTENT_TYPE, "application/json")], body));
        }

        // Hard cap post-pruning: reject rather than fan out to an N-eval.
        if candidate_ids.len() > perf.max_candidate_policies {
            ERRORS_TOTAL
                .with_label_values(&["candidate_cap_exceeded"])
                .inc();
            let body = sonic_rs::to_vec(&EvalResponse {
                decision_id,
                decision: "deny",
                policy_id: "",
                policy_version: 0,
                evaluation_time_microseconds: 0.0,
                total_time_microseconds: 0.0,
                matched_rule: "candidate_cap_exceeded",
                agent_id: &state.agent_id,
                cache_hit: false,
            })
            .unwrap_or_default();
            observe_early_return(&state, start_time);
            return Ok(([(header::CONTENT_TYPE, "application/json")], body));
        }

        candidate_ids
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
                decision_id,
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

            // Cache hits are served requests too — feed the request-total SLA
            // series so a cache-heavy workload's p99 isn't invisible. Constant
            // "cached" label: the hit is scoped to a policy set, not one policy.
            state
                .decision_metrics
                .for_policy("cached")
                .duration
                .observe(start_time.elapsed().as_secs_f64());

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

    // Record Prometheus metrics via cached per-policy handles — no UUID in
    // labels (high-cardinality anti-pattern), and no per-request label hashing.
    // The engine slice goes to reaper_engine_eval_seconds; the request-total
    // observation happens after response serialization below (Phase D).
    let metrics = state.decision_metrics.for_policy(&matched_policy_name);
    metrics.counter(&final_decision).inc();
    metrics
        .engine_duration
        .observe(total_eval_time_ns as f64 / 1_000_000_000.0);

    // Attach OpenTelemetry attributes only when the trace is sampled. The
    // context/span-context lookup and all the KeyValue allocations are skipped
    // entirely on the common unsampled request.
    let span = tracing::Span::current();
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
        // resource is intentionally not a label (unbounded cardinality).
        DENIALS_TOTAL
            .with_label_values(&[&matched_policy_name, &payload.action])
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
        // Deny-priority sampling gate BEFORE building the entry, so sampled-out
        // or disabled decisions cost nothing (no alloc, no formatting).
        if buffer.should_log(decision_str == "allow") {
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
            .with_policy_version(matched_policy_version.to_string())
            .with_matched_rule(matched_rule.clone());
            // Data-plane provenance: which datastore version/checksum this
            // decision saw, and whether it ran past the staleness budget.
            let (data_version, data_checksum) = state.data_sync.provenance();
            entry = entry
                .with_data_sync(data_version, data_checksum, state.data_sync.flag_stale())
                .with_model_version(state.data_sync.model_provenance());

            // "Explain" tier (opt-in, typically denies-only): snapshot the
            // resolved principal/resource attributes the decision branched on.
            // Gated + off the eval path.
            if buffer.should_capture_input(decision_str == "allow") {
                entry.input_data =
                    capture_input_data(&state.data_store, &payload.principal, &payload.resource);
            }

            // Replayable-capture tier (opt-in): the full resolved request as a
            // self-contained blob, so the counterfactual replay engine can
            // re-evaluate this decision under a different policy/data version.
            // Protection (mask/hash/encrypt) applies in buffer.log().
            if buffer.should_capture_replay(decision_str == "allow") {
                entry.replay_input = Some(serde_json::json!({
                    "principal": payload.principal,
                    "action": payload.action,
                    "resource": payload.resource,
                    "context": payload.context.clone().unwrap_or_default(),
                }));
            }

            // Use the same decision_id across response, logs, and audit trail
            entry.decision_id = decision_id.to_string();

            // Mandatory-audit mode: the decision must be DURABLE (written +
            // fsynced) before it is served — otherwise a crash could lose the
            // record of a decision we already answered "allow" on. log_durable
            // awaits the writer's fsync ack without blocking the reactor; if
            // durability cannot be guaranteed we fail closed exactly like the
            // audit_gate (503), never serving a non-durable decision. Best-effort
            // mode (the default) stays fire-and-forget with zero added latency.
            if buffer.mandatory_durable() {
                if !buffer.log_durable(entry).await {
                    ERRORS_TOTAL
                        .with_label_values(&["audit_persist_unavailable"])
                        .inc();
                    // Fail-closed 503s are served responses too — keep the
                    // request-total SLA series honest under audit-sink pressure.
                    metrics.duration.observe(start_time.elapsed().as_secs_f64());
                    return Err(StatusCode::SERVICE_UNAVAILABLE);
                }
            } else {
                buffer.log(entry);
            }
        }
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
        decision_id,
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

    // Request-total latency (handler entry → serialized response), so the SLA
    // series reports what a client experiences, not just the engine slice.
    metrics.duration.observe(start_time.elapsed().as_secs_f64());

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
///
/// No `#[instrument]`: this is the latency-critical path, so we do not pay for
/// per-request span construction (see `evaluate_policy`).
#[utoipa::path(
    post,
    path = "/api/v1/fast-messages",
    tag = "evaluation",
    responses(
        (status = 200, description = "Policy decision (SIMD-accelerated path)")
    ),
    security(("bearer_jwt" = []))
)]
pub async fn fast_evaluate_policy(
    State(state): State<Arc<AgentState>>,
    body: Bytes,
) -> Result<impl IntoResponse, StatusCode> {
    use sonic_rs::{JsonContainerTrait, JsonValueTrait};

    // Start the request-total clock before the FIRST possible return (see the
    // standard endpoint): every served response feeds the SLA histogram.
    let start_time = std::time::Instant::now();

    if let Err(status) = audit_gate(&state) {
        observe_early_return(&state, start_time);
        return Err(status);
    }
    // Track concurrent evaluations
    CONCURRENT_EVALUATIONS.inc();
    let _guard = scopeguard::guard((), |_| {
        CONCURRENT_EVALUATIONS.dec();
    });

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
            observe_early_return(&state, start_time);
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
                        observe_early_return(&state, start_time);
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
                observe_early_return(&state, start_time);
                return Ok(([(header::CONTENT_TYPE, "application/json")], body));
            }
        }
    } else {
        // Evaluate-all path (Plan 08 Phase A) — same fan-out controls as the
        // standard endpoint so the two paths can't diverge on policy.
        let perf = &state.agent_config.performance;

        if !perf.allow_evaluate_all {
            ERRORS_TOTAL
                .with_label_values(&["evaluate_all_disabled"])
                .inc();
            let body = sonic_rs::to_vec(&json!({
                "decision": "deny",
                "error": "evaluate_all_disabled"
            }))
            .unwrap_or_default();
            observe_early_return(&state, start_time);
            return Ok(([(header::CONTENT_TYPE, "application/json")], body));
        }

        let candidate_ids: Vec<Uuid> = if perf.use_pruning_index {
            state.policy_engine.candidate_policy_ids(resource)
        } else {
            state
                .policy_engine
                .list_policies()
                .iter()
                .map(|p| p.id)
                .collect()
        };

        if candidate_ids.is_empty() {
            ERRORS_TOTAL.with_label_values(&["no_policies"]).inc();
            let body = sonic_rs::to_vec(&json!({
                "decision": "deny",
                "error": "no_policies_loaded"
            }))
            .unwrap_or_default();
            observe_early_return(&state, start_time);
            return Ok(([(header::CONTENT_TYPE, "application/json")], body));
        }

        if candidate_ids.len() > perf.max_candidate_policies {
            ERRORS_TOTAL
                .with_label_values(&["candidate_cap_exceeded"])
                .inc();
            let body = sonic_rs::to_vec(&json!({
                "decision": "deny",
                "error": "candidate_cap_exceeded"
            }))
            .unwrap_or_default();
            observe_early_return(&state, start_time);
            return Ok(([(header::CONTENT_TYPE, "application/json")], body));
        }

        candidate_ids
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

    // Record metrics via cached per-policy handles — avoids re-hashing the
    // label values and re-locking the metric vecs on every request. Engine
    // slice here; the request-total observation happens after response
    // serialization below (Phase D), same as the standard endpoint.
    let metrics = state.decision_metrics.for_policy(&policy_name_resolved);
    metrics.counter(&final_decision).inc();
    metrics
        .engine_duration
        .observe(total_eval_time_ns as f64 / 1_000_000_000.0);

    // Audit: capture the decision (deny-priority sampled). The fast path was
    // previously not logged at all, so fast-endpoint decisions went unaudited —
    // a real gap for an audit system. Gated by should_log so sampled-out/disabled
    // decisions cost nothing.
    if let Some(ref buffer) = state.decision_buffer {
        if buffer.should_log(decision_str == "allow") {
            let principal = request
                .context
                .get("principal")
                .cloned()
                .unwrap_or_default();
            let mut entry = DecisionLogEntry::new(
                principal,
                request.action.clone(),
                request.resource.clone(),
                decision_str.to_string(),
                matched_policy_id.to_string(),
                policy_name_resolved.clone(),
            )
            .with_evaluation_time_ns(total_eval_time_ns)
            .with_cache_hit(false)
            .with_agent_id(state.agent_id.clone())
            .with_policy_version(matched_policy_version.to_string())
            .with_matched_rule(
                matched_rule
                    .map(|r| format!("rule_{}", r))
                    .unwrap_or_default(),
            );
            let (data_version, data_checksum) = state.data_sync.provenance();
            entry = entry
                .with_data_sync(data_version, data_checksum, state.data_sync.flag_stale())
                .with_model_version(state.data_sync.model_provenance());
            if buffer.should_capture_input(decision_str == "allow") {
                entry.input_data = capture_input_data(
                    &state.data_store,
                    &request
                        .context
                        .get("principal")
                        .cloned()
                        .unwrap_or_default(),
                    &request.resource,
                );
            }
            // Replayable-capture tier: the full resolved request (see the
            // canonical handler above). Protection applies in buffer.log().
            if buffer.should_capture_replay(decision_str == "allow") {
                entry.replay_input = Some(serde_json::json!({
                    "principal": request
                        .context
                        .get("principal")
                        .cloned()
                        .unwrap_or_default(),
                    "action": request.action,
                    "resource": request.resource,
                    "context": request.context,
                }));
            }
            entry.decision_id = decision_id.to_string();
            // Mandatory-audit mode: durable-before-serve (see the standard
            // handler above). Fail closed on a non-durable result; best-effort
            // stays fire-and-forget.
            if buffer.mandatory_durable() {
                if !buffer.log_durable(entry).await {
                    ERRORS_TOTAL
                        .with_label_values(&["audit_persist_unavailable"])
                        .inc();
                    // Fail-closed 503s are served responses too — keep the
                    // request-total SLA series honest under audit-sink pressure.
                    metrics.duration.observe(start_time.elapsed().as_secs_f64());
                    return Err(StatusCode::SERVICE_UNAVAILABLE);
                }
            } else {
                buffer.log(entry);
            }
        }
    }

    // Encode the policy UUID into a stack buffer — no heap allocation, same
    // trick as decision_id.
    let mut policy_id_buf = [0u8; Hyphenated::LENGTH];
    let policy_id_str: &str = matched_policy_id
        .as_hyphenated()
        .encode_lower(&mut policy_id_buf);
    let matched_rule_str = matched_rule
        .map(|r| format!("rule_{}", r))
        .unwrap_or_default();

    let resp_body = sonic_rs::to_vec(&EvalResponse {
        decision_id,
        decision: decision_str,
        policy_id: policy_id_str,
        policy_version: matched_policy_version,
        evaluation_time_microseconds: total_eval_time_ns as f64 / 1000.0,
        total_time_microseconds: total_time.as_nanos() as f64 / 1000.0,
        matched_rule: &matched_rule_str,
        agent_id: &state.agent_id,
        cache_hit: false,
    })
    .unwrap_or_default();

    // Request-total latency (handler entry → serialized response) into the
    // same SLA series as the standard endpoint, so the two are comparable.
    metrics.duration.observe(start_time.elapsed().as_secs_f64());

    Ok(([(header::CONTENT_TYPE, "application/json")], resp_body))
}

/// Batch policy evaluation.
///
/// Evaluates multiple policy requests against a single policy. Intended for bulk
/// authorization checks where throughput matters more than per-request latency.
///
/// Behavior:
/// - Batch size is capped at `performance.max_batch_requests` (default 1000);
///   an over-cap batch is rejected with 413 before any evaluation.
/// - The evaluation loop runs on a `spawn_blocking` thread so it cannot starve
///   the async reactor or block unrelated request latency (Plan 05, Step 3),
///   and is parallelized across the rayon pool (Plan 08 Phase B) so a large
///   batch finishes in ~batch/cores time instead of running sequentially.
/// - Optional decision-cache integration; results preserve input order via an
///   explicit `index` field.
#[utoipa::path(
    post,
    path = "/api/v1/batch-messages",
    tag = "evaluation",
    responses(
        (status = 200, description = "Batch policy decisions"),
        (status = 413, description = "Batch request count exceeds configured maximum")
    ),
    security(("bearer_jwt" = []))
)]
#[instrument(skip(state, payload))]
pub async fn batch_evaluate_policy(
    State(state): State<Arc<AgentState>>,
    Json(payload): Json<crate::types::BatchEvaluateRequest>,
) -> Result<Json<Value>, StatusCode> {
    audit_gate(&state)?;
    let start_time = std::time::Instant::now();

    // Bound the batch BEFORE any work: the 256 MB body limit exists for bulk
    // entity loads, not as a batch bound, so a body full of tiny requests could
    // otherwise drive millions of synchronous evals on one worker. Reject
    // over-cap batches with 413 up front (Plan 05, Step 3 / ADR-4).
    let max_batch = state.agent_config.performance.max_batch_requests;
    if payload.requests.len() > max_batch {
        ERRORS_TOTAL.with_label_values(&["batch_too_large"]).inc();
        warn!(
            requested = payload.requests.len(),
            max = max_batch,
            "batch evaluation rejected: request count exceeds max_batch_requests"
        );
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }

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

    // Offload the synchronous evaluation loop to a blocking thread so a large
    // batch (up to `max_batch_requests`) cannot starve the async reactor and
    // block unrelated single-eval / health traffic on a low-core sidecar
    // (Plan 05, Step 3), and fan the loop out across the rayon pool (Plan 08
    // Phase B): the engine store is lock-free and the decision cache sharded,
    // so per-request evaluations are independent. `with_min_len` keeps small
    // batches from paying rayon's split overhead; an indexed collect preserves
    // input order alongside the explicit `index` field.
    let eval_state = state.clone();
    let eval_policy_name = policy_name.clone();
    let results: Vec<Value> = match tokio::task::spawn_blocking(move || {
        use rayon::prelude::*;
        let state = eval_state;
        // Resolve the per-policy metric handle once for the whole batch.
        let metrics = state.decision_metrics.for_policy(&eval_policy_name);
        requests
            .par_iter()
            .with_min_len(32)
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

                // Record metrics via the cached per-policy handle.
                metrics.counter(&decision).inc();

                json!({
                    "index": i,
                    "decision": decision_str,
                    "evaluation_time_microseconds": duration.as_nanos() as f64 / 1000.0,
                    "cache_hit": cache_hit
                })
            })
            .collect()
    })
    .await
    {
        Ok(results) => results,
        Err(join_err) => {
            // The blocking task panicked or was cancelled — fail closed.
            ERRORS_TOTAL.with_label_values(&["batch_eval_join"]).inc();
            error!(error = %join_err, "batch evaluation task failed");
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

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

    /// The eval hot path branches on `buffer.mandatory_durable()` and, when
    /// `log_durable` cannot guarantee persistence, returns 503 instead of
    /// serving the computed decision. This exercises that exact predicate: a
    /// mandatory-audit buffer whose only sink is stdout (which can't be fsynced)
    /// reports `mandatory_durable() == true` yet `log_durable(...) == false`,
    /// so the handler fails closed rather than serving a non-durable allow.
    #[tokio::test]
    async fn test_mandatory_durable_without_durable_sink_forces_fail_closed() {
        use policy_engine::{DecisionBuffer, DecisionLogConfig, DecisionLogEntry, PrivacyProfile};

        // A mandatory config with no fsync-able file sink (stdout only) cannot
        // support durable-before-serve. Rather than degrade to a per-request 503,
        // it must fail fast at construction so the operator sees a clear config error.
        let stdout_only = DecisionLogConfig {
            enabled: true,
            privacy_profile: Some(PrivacyProfile::Raw),
            audit_required: true,
            emit_stdout: true,
            checkpoint_every: 100,
            checkpoint_signing_key: Some("07".repeat(32)),
            checkpoint_key_id: Some("k1".to_string()),
            ..Default::default()
        };
        assert!(
            DecisionBuffer::new(stdout_only).is_err(),
            "stdout-only mandatory config must be rejected at startup, not per request"
        );

        // With a real file sink the buffer starts, and durable-before-serve acks
        // only after the entry is fsynced to disk.
        let path = std::env::temp_dir().join(format!(
            "reaper_agent_durable_{}.ndjson",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let config = DecisionLogConfig {
            enabled: true,
            privacy_profile: Some(PrivacyProfile::Raw),
            audit_required: true,
            file_path: Some(path.to_string_lossy().into_owned()),
            checkpoint_every: 100,
            checkpoint_signing_key: Some("07".repeat(32)),
            checkpoint_key_id: Some("k1".to_string()),
            ..Default::default()
        };
        let buffer = DecisionBuffer::new(config).unwrap();
        assert!(buffer.mandatory_durable());

        let entry = DecisionLogEntry::new(
            "alice".to_string(),
            "read".to_string(),
            "/x".to_string(),
            "allow".to_string(),
            "pid".to_string(),
            "pname".to_string(),
        );
        // A durable file sink → the entry is persisted and the handler serves the allow.
        assert!(buffer.log_durable(entry).await);
        assert!(buffer.is_audit_healthy());
        let _ = std::fs::remove_file(&path);
    }
}
