//! # reaper-wasm — the Reaper eval core as a WebAssembly embedding
//!
//! Workstream F2 slice 2 (`plans/round-2/F2-wasm-target.md`): a thin
//! wasm-bindgen wrapper over the I/O-free `policy-engine` core so the same
//! sub-microsecond DSL evaluation runs in a browser, an edge worker, or a
//! Node process — without the agent.
//!
//! Design constraints:
//! - **Parity with the agent's serving semantics.** The wrapper mirrors what
//!   `services/reaper-agent` does around the engine: the principal is
//!   injected as `context["principal"]` (and nothing else — `context.action`
//!   is resolved by the evaluator from the typed request field), and scalar
//!   context values are coerced to strings exactly like the agent's fast
//!   path (nested objects/arrays are dropped, not errors). A decision
//!   produced here must match the decision the agent would have produced for
//!   the same inputs — enforced by the parity suite: `tests/parity.rs` runs
//!   the policy-library manifest cases natively through this very wrapper,
//!   and `tests/node/smoke.mjs` runs the same manifest cases through the
//!   actual wasm artifact.
//! - **JSON strings at the boundary.** Requests/decisions cross the JS
//!   boundary as JSON matching the engine's serialized shapes
//!   (`PolicyDecision`, `AllPoliciesEvaluationResult`), so an embedding can
//!   swap between the HTTP agent and the in-process wasm gate without
//!   remapping fields.
//! - **Deterministic time.** On wasm the DSL `time::*` builtins read the
//!   host-injectable clock (`setNowUnixNs`); unset, they fall back to the JS
//!   clock via `policy_engine::clock`. Pinning the clock makes decisions
//!   replayable.
//!
//! The inherent `*_impl` methods carry the logic and return
//! `Result<T, String>` so native tests exercise the identical code path; the
//! `#[wasm_bindgen]` exports only map errors into `JsError` at the boundary
//! (constructing a `JsError` outside a JS runtime is not supported).

use policy_engine::{
    DataLoader, DataStore, EnhancedPolicy, PolicyEngine, PolicyId, PolicyLanguage, PolicyRequest,
};
use std::collections::HashMap;
use std::sync::Arc;
use wasm_bindgen::prelude::*;

/// A self-contained policy evaluation engine: policy store + entity data
/// store + evaluators, behind the same combination semantics the agent
/// serves.
#[wasm_bindgen]
pub struct ReaperEngine {
    engine: PolicyEngine,
    store: Arc<DataStore>,
    loader: DataLoader,
}

impl Default for ReaperEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Core implementation — target-independent, exercised directly by the native
// parity tests. Everything returns Result<T, String>.
// ---------------------------------------------------------------------------
impl ReaperEngine {
    /// Deploy a Reaper DSL (`.reap`) policy; returns its UUID string.
    ///
    /// Fully dynamic, like the agent: redeploying an existing policy NAME is
    /// an atomic zero-downtime hot-swap — the policy keeps its id, its
    /// version bumps, and concurrent evaluations see either the old or the
    /// new version, never a gap (the engine's `deploy_policy` DashMap swap).
    pub fn deploy_policy_impl(&self, name: &str, reap_source: &str) -> Result<String, String> {
        let mut policy = EnhancedPolicy::new_with_language(
            name.to_string(),
            String::new(),
            PolicyLanguage::ReaperDsl,
            reap_source.to_string(),
        )
        .map_err(|e| format!("policy parse/build failed: {e}"))?;

        // Hot-swap semantics: same name ⇒ same policy id, next version.
        if let Some(existing) = self.engine.get_policy_by_name(name) {
            policy.id = existing.id;
            policy.version = existing.version + 1;
        }

        policy
            .build_evaluator_with_data(Some(self.store.clone()))
            .map_err(|e| format!("evaluator build failed: {e}"))?;

        let id = policy.id;
        self.engine
            .deploy_policy(policy)
            .map_err(|e| format!("deploy failed: {e}"))?;
        Ok(id.to_string())
    }

    /// Remove a deployed policy (atomic; in-flight evaluations holding the
    /// old Arc complete against it). Returns the removed policy's version.
    pub fn remove_policy_impl(&self, policy_id: &str) -> Result<u64, String> {
        let id = policy_id
            .parse::<PolicyId>()
            .map_err(|e| format!("invalid policy id '{policy_id}': {e}"))?;
        let removed = self
            .engine
            .remove_policy(&id)
            .map_err(|e| format!("remove failed: {e}"))?;
        Ok(removed.version)
    }

    /// Load a `{"entities": [...]}` JSON document; returns entities loaded.
    pub fn load_entities_json_impl(&self, json: &str) -> Result<u32, String> {
        let n = self
            .loader
            .load_json(json)
            .map_err(|e| format!("entity load failed: {e}"))?;
        Ok(n as u32)
    }

    /// Evaluate one request against one policy → `PolicyDecision` JSON.
    pub fn evaluate_impl(
        &self,
        policy_id: &str,
        principal: &str,
        action: &str,
        resource: &str,
        context_json: Option<&str>,
    ) -> Result<String, String> {
        let id = policy_id
            .parse::<PolicyId>()
            .map_err(|e| format!("invalid policy id '{policy_id}': {e}"))?;
        let request = build_request(principal, action, resource, context_json)?;
        let decision = self
            .engine
            .evaluate(&id, &request)
            .map_err(|e| format!("evaluation failed: {e}"))?;
        serde_json::to_string(&decision).map_err(|e| format!("decision serialization failed: {e}"))
    }

    /// The evaluator tier serving a deployed policy: `"reaper_dsl"` is the
    /// compiled DSL v2 evaluator (the PRIMARY path, same as the agent);
    /// `"ReapAstEvaluator"` is the AST-interpreter fallback for constructs
    /// the compiler does not yet support. The two are pinned to identical
    /// decisions by the compiled-vs-AST equivalence differential — the tier
    /// affects speed, never outcomes. The parity suite asserts wasm and
    /// native land on the SAME tier for every library scenario.
    pub fn evaluator_type_impl(&self, policy_id: &str) -> Result<String, String> {
        let id = policy_id
            .parse::<PolicyId>()
            .map_err(|e| format!("invalid policy id '{policy_id}': {e}"))?;
        let policy = self
            .engine
            .get_policy(&id)
            .ok_or_else(|| format!("no policy with id '{policy_id}'"))?;
        let evaluator = policy
            .get_evaluator()
            .map_err(|e| format!("evaluator not built: {e}"))?;
        Ok(evaluator.evaluator_type().to_string())
    }

    /// Evaluate against ALL deployed policies (any deny wins) →
    /// `AllPoliciesEvaluationResult` JSON.
    pub fn evaluate_all_impl(
        &self,
        principal: &str,
        action: &str,
        resource: &str,
        context_json: Option<&str>,
    ) -> Result<String, String> {
        let request = build_request(principal, action, resource, context_json)?;
        let result = self.engine.evaluate_all(&request);
        serde_json::to_string(&result).map_err(|e| format!("result serialization failed: {e}"))
    }
}

// ---------------------------------------------------------------------------
// wasm-bindgen boundary — thin wrappers, JsError mapping only.
// ---------------------------------------------------------------------------
#[wasm_bindgen]
impl ReaperEngine {
    /// Create an empty engine (no policies, no entities).
    #[wasm_bindgen(constructor)]
    pub fn new() -> ReaperEngine {
        let store = Arc::new(DataStore::new());
        // DataStore clones share the same underlying interned storage, so the
        // loader writes into the exact store the evaluators read.
        let loader = DataLoader::new(store.as_ref().clone());
        ReaperEngine {
            engine: PolicyEngine::new(),
            store,
            loader,
        }
    }

    /// Deploy a Reaper DSL (`.reap`) policy from source text; returns the
    /// policy id (UUID string) to pass to [`ReaperEngine::evaluate`].
    #[wasm_bindgen(js_name = deployPolicy)]
    pub fn deploy_policy(&self, name: &str, reap_source: &str) -> Result<String, JsError> {
        self.deploy_policy_impl(name, reap_source)
            .map_err(|e| JsError::new(&e))
    }

    /// Load entities from a `{"entities": [...]}` JSON document (the same
    /// format the agent and CLI consume). Returns the number loaded.
    #[wasm_bindgen(js_name = loadEntitiesJson)]
    pub fn load_entities_json(&self, json: &str) -> Result<u32, JsError> {
        self.load_entities_json_impl(json)
            .map_err(|e| JsError::new(&e))
    }

    /// Evaluate one request against one policy. Returns the engine's
    /// `PolicyDecision` as a JSON string.
    ///
    /// `context_json` is an optional JSON object; scalar values are coerced
    /// to strings (agent fast-path semantics), nested values are dropped.
    pub fn evaluate(
        &self,
        policy_id: &str,
        principal: &str,
        action: &str,
        resource: &str,
        context_json: Option<String>,
    ) -> Result<String, JsError> {
        self.evaluate_impl(
            policy_id,
            principal,
            action,
            resource,
            context_json.as_deref(),
        )
        .map_err(|e| JsError::new(&e))
    }

    /// Evaluate one request against ALL deployed policies (security-first:
    /// any deny wins). Returns `AllPoliciesEvaluationResult` as JSON.
    #[wasm_bindgen(js_name = evaluateAll)]
    pub fn evaluate_all(
        &self,
        principal: &str,
        action: &str,
        resource: &str,
        context_json: Option<String>,
    ) -> Result<String, JsError> {
        self.evaluate_all_impl(principal, action, resource, context_json.as_deref())
            .map_err(|e| JsError::new(&e))
    }

    /// Number of currently deployed policies.
    #[wasm_bindgen(js_name = policyCount)]
    pub fn policy_count(&self) -> u32 {
        self.engine.get_stats().total_policies as u32
    }

    /// The evaluator tier serving a deployed policy (`"reaper_dsl"` =
    /// compiled PRIMARY path, `"ReapAstEvaluator"` = AST fallback).
    #[wasm_bindgen(js_name = evaluatorType)]
    pub fn evaluator_type(&self, policy_id: &str) -> Result<String, JsError> {
        self.evaluator_type_impl(policy_id)
            .map_err(|e| JsError::new(&e))
    }

    /// Remove a deployed policy (atomic). Returns the removed version.
    #[wasm_bindgen(js_name = removePolicy)]
    pub fn remove_policy(&self, policy_id: &str) -> Result<u64, JsError> {
        self.remove_policy_impl(policy_id)
            .map_err(|e| JsError::new(&e))
    }

    /// Pin the evaluation clock to a fixed unix-epoch nanosecond timestamp so
    /// DSL `time::*` builtins are deterministic/replayable (wasm builds only —
    /// native embeddings use the real system clock).
    #[cfg(target_arch = "wasm32")]
    #[wasm_bindgen(js_name = setNowUnixNs)]
    pub fn set_now_unix_ns(&self, unix_ns: i64) {
        policy_engine::clock::set_injected_now_unix_ns(unix_ns);
    }

    /// Unpin the evaluation clock; `time::*` falls back to the JS host clock.
    #[cfg(target_arch = "wasm32")]
    #[wasm_bindgen(js_name = clearInjectedNow)]
    pub fn clear_injected_now(&self) {
        policy_engine::clock::clear_injected_now();
    }
}

/// Build a `PolicyRequest` the way the agent does: principal injected as
/// `context["principal"]`, scalar context values coerced to strings, nested
/// values dropped (mirrors `services/reaper-agent` fast-path coercion).
fn build_request(
    principal: &str,
    action: &str,
    resource: &str,
    context_json: Option<&str>,
) -> Result<PolicyRequest, String> {
    let mut context: HashMap<String, String> = HashMap::new();

    if let Some(raw) = context_json {
        if !raw.trim().is_empty() {
            let value: serde_json::Value =
                serde_json::from_str(raw).map_err(|e| format!("context is not valid JSON: {e}"))?;
            let obj = value
                .as_object()
                .ok_or_else(|| "context must be a JSON object".to_string())?;
            for (k, v) in obj {
                match v {
                    serde_json::Value::String(s) => {
                        context.insert(k.clone(), s.clone());
                    }
                    serde_json::Value::Number(n) => {
                        context.insert(k.clone(), n.to_string());
                    }
                    serde_json::Value::Bool(b) => {
                        context.insert(k.clone(), b.to_string());
                    }
                    // Agent fast-path semantics: nested objects/arrays and
                    // nulls are not representable in the flat context and are
                    // dropped, not errors.
                    _ => {}
                }
            }
        }
    }

    // Exactly what the agent injects — principal only. `context.action` is
    // resolved by the evaluator from the typed request field; inserting it
    // here would shadow a caller-supplied value and diverge from the agent.
    context.insert("principal".to_string(), principal.to_string());

    Ok(PolicyRequest {
        resource: resource.to_string(),
        action: action.to_string(),
        context,
    })
}
