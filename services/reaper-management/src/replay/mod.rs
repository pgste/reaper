//! Counterfactual replay engine (Plan 04, step 8).
//!
//! Answers "what would policy bundle X have decided on real historical
//! traffic?": streams decisions captured with the replayable tier
//! (`replay_input`, step 7) out of ClickHouse, re-evaluates each request on a
//! **headless `PolicyEngine`** loaded with the target bundle and a pinned
//! datastore snapshot, and reports the diff — allow→deny / deny→allow flip
//! counts plus sample flipped records.
//!
//! Fidelity guarantees:
//! - **Decision semantics** come from [`PolicyEngine::evaluate_set`] — the
//!   same function the agent's serving path calls, so replay and production
//!   can never combine policies differently.
//! - **Policy load** mirrors the agent's bundle apply (same artifact schema,
//!   same `EnhancedPolicy` construction, evaluator built with the datastore).
//! - **Data** is pinned by `data_version` (the exact document agents loaded —
//!   the same provenance every decision row records), defaulting to the
//!   namespace's current published version.
//!
//! Jobs run async on a tokio task with live progress; results are held in an
//! in-memory registry (replay is an ephemeral analysis, not durable state —
//! re-run it if the control plane restarts).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use policy_engine::{
    DataLoader, DataStore, EnhancedPolicy, PolicyAction, PolicyEngine, PolicyRequest,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::RwLock;
use uuid::Uuid;

use crate::bundle::compiler::BundleContent;
use crate::db::repositories::{DatastoreRepository, NamespaceRepository};
use crate::decisions::{DecisionQuery, DecisionRow};
use crate::state::AppState;

/// Hard ceiling on rows scanned per job (requests can lower it).
pub const MAX_SCAN_ROWS: u64 = 100_000;
/// Default scan cap when the request doesn't set one.
pub const DEFAULT_SCAN_ROWS: u64 = 10_000;
/// ClickHouse page size while streaming rows.
const PAGE_SIZE: u64 = 500;
/// Max sample flipped records carried in the result.
const MAX_SAMPLES: usize = 20;

/// A replay request (API body).
#[derive(Debug, Clone, Deserialize)]
pub struct ReplayRequest {
    /// Target bundle to evaluate under (must be compiled; tenant-scoped).
    pub bundle_id: Uuid,
    /// Time range over the historical decisions (RFC3339 / `YYYY-MM-DD HH:MM:SS`).
    pub from: Option<String>,
    pub to: Option<String>,
    /// Optional row filters (same dimensions as the decisions API).
    #[serde(default)]
    pub filter: ReplayFilter,
    /// Namespace whose datastore the policies evaluate against. Omitted =
    /// evaluate with an empty datastore (request-only policies).
    pub namespace: Option<String>,
    /// Pin the exact datastore snapshot (the `data_version` decision rows
    /// record). Omitted = the namespace's current published version.
    pub data_version: Option<i64>,
    /// Tenant encryption key (hex) to open encrypted `replay_input` blobs.
    /// Never persisted; lives only for the job's duration.
    pub decryption_key: Option<String>,
    /// Scan cap (default 10k, max 100k).
    pub max_rows: Option<u64>,
}

/// Row filters for the replayed range.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ReplayFilter {
    pub principal: Option<String>,
    pub action: Option<String>,
    pub resource: Option<String>,
    pub decision: Option<String>,
    pub policy_name: Option<String>,
    pub agent_id: Option<String>,
}

/// One sampled flipped decision: what it was, what it would be.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct FlipSample {
    pub decision_id: String,
    pub timestamp: String,
    pub principal: String,
    pub action: String,
    pub resource: String,
    pub old_decision: String,
    pub new_decision: String,
    /// The rule/policy attribution then and now.
    pub old_matched_rule: String,
    pub new_policy_name: String,
    pub new_matched_rule: Option<usize>,
}

/// The replay diff summary.
#[derive(Debug, Clone, Default, Serialize, utoipa::ToSchema)]
pub struct ReplayResult {
    /// Rows the range/filters matched (scan-capped).
    pub scanned: u64,
    /// Rows actually re-evaluated.
    pub replayed: u64,
    /// Rows captured without the replayable tier — not re-evaluable.
    pub skipped_not_replayable: u64,
    /// Encrypted blobs with no (or a wrong) decryption key supplied.
    pub skipped_encrypted: u64,
    /// Same decision under the target bundle.
    pub unchanged: u64,
    pub allow_to_deny: u64,
    pub deny_to_allow: u64,
    /// Transitions involving "log" or unknown historical values.
    pub other_changes: u64,
    /// True when the scan cap ended the run before the range was exhausted.
    pub truncated: bool,
    /// Datastore version the headless engine evaluated against (None = empty).
    pub data_version: Option<i64>,
    /// Sample flipped records (both directions, capped).
    pub samples: Vec<FlipSample>,
}

/// Live job entry in the in-memory registry.
pub struct ReplayJob {
    pub org_id: Uuid,
    pub bundle_id: Uuid,
    pub created_at: DateTime<Utc>,
    /// Live progress (rows scanned so far).
    pub scanned: AtomicU64,
    /// Terminal state: Ok(result) | Err(message). None while running.
    pub outcome: RwLock<Option<Result<ReplayResult, String>>>,
}

/// Wire status of a replay job — the `GET …/replay/{job_id}` response body.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ReplayStatus {
    pub job_id: Uuid,
    pub bundle_id: Uuid,
    pub created_at: DateTime<Utc>,
    /// `running` | `completed` | `failed`.
    pub state: String,
    /// Rows scanned so far (present while `running`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scanned: Option<u64>,
    /// The replay diff summary (present when `completed`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<ReplayResult>,
    /// Failure message (present when `failed`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ReplayJob {
    /// Render the job for the API.
    pub fn status(&self, job_id: Uuid) -> ReplayStatus {
        let mut status = ReplayStatus {
            job_id,
            bundle_id: self.bundle_id,
            created_at: self.created_at,
            state: "running".to_string(),
            scanned: None,
            result: None,
            error: None,
        };
        let outcome = self.outcome.read().expect("replay job lock poisoned");
        match &*outcome {
            None => {
                status.scanned = Some(self.scanned.load(Ordering::Relaxed));
            }
            Some(Ok(result)) => {
                status.state = "completed".to_string();
                status.result = Some(result.clone());
            }
            Some(Err(e)) => {
                status.state = "failed".to_string();
                status.error = Some(e.clone());
            }
        }
        status
    }
}

/// In-memory job registry (ephemeral analyses; not durable across restarts).
pub type ReplayJobs = Arc<DashMap<Uuid, Arc<ReplayJob>>>;

/// Start a replay job: validates + builds the headless engine up front (so
/// obvious errors — unknown bundle, uncompiled bundle, bad namespace — fail
/// the POST synchronously), then streams + re-evaluates on a background task.
/// Returns the job id.
pub async fn start_job(
    state: Arc<AppState>,
    org_id: Uuid,
    request: ReplayRequest,
) -> Result<Uuid, String> {
    let store = state
        .decision_store
        .clone()
        .ok_or_else(|| "decision store not configured: set REAPER_CLICKHOUSE_URL".to_string())?;

    // Build the headless engine NOW: cheap relative to the scan, and it front-
    // loads every configuration error into the synchronous path.
    let (engine, policy_ids, data_version) =
        build_headless_engine(&state, org_id, &request).await?;

    let job = Arc::new(ReplayJob {
        org_id,
        bundle_id: request.bundle_id,
        created_at: Utc::now(),
        scanned: AtomicU64::new(0),
        outcome: RwLock::new(None),
    });
    let job_id = Uuid::new_v4();
    state.replay_jobs.insert(job_id, job.clone());

    tokio::spawn(async move {
        let outcome = run_replay(
            &store,
            &engine,
            &policy_ids,
            data_version,
            org_id,
            &request,
            &job.scanned,
        )
        .await;
        *job.outcome.write().expect("replay job lock poisoned") = Some(outcome);
    });

    Ok(job_id)
}

/// Build the headless engine: target bundle's policies (agent-identical
/// construction) + the pinned datastore snapshot. Public so integration tests
/// can exercise the bundle→engine load without a ClickHouse row source.
pub async fn build_headless_engine(
    state: &AppState,
    org_id: Uuid,
    request: &ReplayRequest,
) -> Result<(Arc<PolicyEngine>, Vec<Uuid>, Option<i64>), String> {
    // Tenant-scoped bundle fetch — a foreign bundle id is a 404, not a leak.
    let bundle = state
        .bundle_service
        .get_scoped(org_id, request.bundle_id)
        .await
        .map_err(|e| format!("bundle: {e}"))?;
    let download = state
        .bundle_service
        .download(request.bundle_id)
        .await
        .map_err(|e| {
            format!(
                "bundle '{}' has no compiled artifact (compile it first): {e}",
                bundle.name
            )
        })?;
    let content: BundleContent = serde_json::from_slice(&download.data)
        .map_err(|e| format!("corrupt bundle artifact: {e}"))?;

    // Datastore snapshot, pinned by version. `DataStore` clones share their
    // Arc'd internals, so loading through a clone populates the store the
    // evaluators hold — the same pattern the agent's data handlers use.
    let data_store = Arc::new(DataStore::new());
    let mut loaded_version = None;
    if let Some(ns_slug) = request.namespace.as_deref() {
        let ns = NamespaceRepository::new(&state.db)
            .get_by_slug(org_id, ns_slug)
            .await
            .map_err(|e| format!("namespace: {e}"))?
            .ok_or_else(|| format!("namespace '{ns_slug}' not found"))?;
        let ds_repo = DatastoreRepository::new(&state.db);
        let record = ds_repo
            .get(org_id, ns.id)
            .await
            .map_err(|e| format!("datastore: {e}"))?
            .ok_or_else(|| format!("namespace '{ns_slug}' has no datastore"))?;
        let version = request.data_version.unwrap_or(record.current_version);
        if version > 0 {
            let (_meta, document) = ds_repo
                .get_version_document(record.id, version)
                .await
                .map_err(|e| format!("datastore version: {e}"))?
                .ok_or_else(|| {
                    format!("datastore version {version} not found (published versions only)")
                })?;
            // The stored document is byte-for-byte what agents load — replay
            // parity holds by construction (same loader, same payload).
            DataLoader::new((*data_store).clone())
                .load_json(&document)
                .map_err(|e| format!("load datastore snapshot v{version}: {e}"))?;
            loaded_version = Some(version);
        }
    } else if request.data_version.is_some() {
        return Err("data_version requires a namespace".to_string());
    }

    // Policies: identical construction to the agent's bundle apply, but with
    // the evaluator built AGAINST THE SNAPSHOT (build_evaluator_with_data).
    let engine = Arc::new(PolicyEngine::new());
    let mut policy_ids = Vec::with_capacity(content.policies.len());
    for entry in &content.policies {
        let policy_id = Uuid::parse_str(&entry.id).unwrap_or_else(|_| Uuid::new_v4());
        let mut policy = EnhancedPolicy::new(entry.id.clone(), "replay target".to_string(), vec![]);
        policy.id = policy_id;
        policy.version = entry.version as u64;
        policy.content = entry.content.clone();
        policy.language = match entry.language.as_str() {
            "cedar" => policy_engine::PolicyLanguage::Cedar,
            "simple" => policy_engine::PolicyLanguage::Simple,
            _ => policy_engine::PolicyLanguage::ReaperDsl,
        };
        policy
            .build_evaluator_with_data(Some(data_store.clone()))
            .map_err(|e| format!("policy '{}' failed to compile: {e}", entry.id))?;
        engine
            .deploy_policy(policy)
            .map_err(|e| format!("policy '{}' failed to deploy: {e}", entry.id))?;
        policy_ids.push(policy_id);
    }
    if policy_ids.is_empty() {
        return Err("bundle contains no policies".to_string());
    }

    Ok((engine, policy_ids, loaded_version))
}

/// Stream the historical rows and diff them against the headless engine.
async fn run_replay(
    store: &crate::decisions::DecisionStore,
    engine: &PolicyEngine,
    policy_ids: &[Uuid],
    data_version: Option<i64>,
    org_id: Uuid,
    request: &ReplayRequest,
    progress: &AtomicU64,
) -> Result<ReplayResult, String> {
    let tenant = org_id.to_string();
    let cap = request
        .max_rows
        .unwrap_or(DEFAULT_SCAN_ROWS)
        .min(MAX_SCAN_ROWS);

    let mut result = ReplayResult {
        data_version,
        ..Default::default()
    };
    let mut offset = 0u64;

    loop {
        let page = cap.saturating_sub(result.scanned).min(PAGE_SIZE);
        if page == 0 {
            result.truncated = true;
            break;
        }
        let query = DecisionQuery {
            principal: request.filter.principal.clone(),
            action: request.filter.action.clone(),
            resource: request.filter.resource.clone(),
            decision: request.filter.decision.clone(),
            policy_name: request.filter.policy_name.clone(),
            agent_id: request.filter.agent_id.clone(),
            from: request.from.clone(),
            to: request.to.clone(),
            limit: Some(page),
            offset: Some(offset),
            cursor: None,
            after: None,
        };
        let rows = store
            .list(&tenant, &query)
            .await
            .map_err(|e| format!("decision store: {e}"))?;
        let fetched = rows.len() as u64;

        for row in rows {
            result.scanned += 1;
            progress.store(result.scanned, Ordering::Relaxed);
            replay_row(engine, policy_ids, request, row, &mut result);
        }

        if fetched < page {
            break; // range exhausted
        }
        offset += fetched;
    }

    if result.scanned > 0 && result.replayed == 0 && result.skipped_encrypted == 0 {
        return Err(format!(
            "{} decision(s) in range but none are replayable — the replayable \
             capture tier was off at capture time. Enable it on the agents \
             (REAPER_DECISION_LOG_REPLAY_INPUT=true) and replay a later range.",
            result.scanned
        ));
    }

    Ok(result)
}

/// Re-evaluate one historical row and fold the diff into the result.
fn replay_row(
    engine: &PolicyEngine,
    policy_ids: &[Uuid],
    request: &ReplayRequest,
    row: DecisionRow,
    result: &mut ReplayResult,
) {
    // Resolve the replay blob: absent → not replayable; sealed → decrypt.
    let blob = match &row.replay_input {
        Value::Null => {
            result.skipped_not_replayable += 1;
            return;
        }
        v if v.get("enc").is_some() => {
            let Some(key) = request.decryption_key.as_deref() else {
                result.skipped_encrypted += 1;
                return;
            };
            match policy_engine::decrypt_input_data(v, key) {
                Ok(opened) => opened,
                Err(_) => {
                    result.skipped_encrypted += 1;
                    return;
                }
            }
        }
        v => v.clone(),
    };

    // Rebuild the request exactly as the agent does: context + principal key.
    let principal = blob
        .get("principal")
        .and_then(Value::as_str)
        .unwrap_or(&row.principal)
        .to_string();
    let action = blob
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or(&row.action)
        .to_string();
    let resource = blob
        .get("resource")
        .and_then(Value::as_str)
        .unwrap_or(&row.resource)
        .to_string();
    let mut context: HashMap<String, String> = blob
        .get("context")
        .and_then(Value::as_object)
        .map(|o| {
            o.iter()
                .map(|(k, v)| {
                    let s = v
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| v.to_string());
                    (k.clone(), s)
                })
                .collect()
        })
        .unwrap_or_default();
    context.insert("principal".to_string(), principal.clone());

    let policy_request = PolicyRequest {
        resource: resource.clone(),
        action: action.clone(),
        context,
    };
    let outcome = engine.evaluate_set(policy_ids, &policy_request);
    result.replayed += 1;

    let new_decision = match outcome.decision {
        PolicyAction::Allow => "allow",
        PolicyAction::Deny => "deny",
        PolicyAction::Log => "log",
    };
    let old_decision = row.decision.as_str();

    if new_decision == old_decision {
        result.unchanged += 1;
        return;
    }
    match (old_decision, new_decision) {
        ("allow", "deny") => result.allow_to_deny += 1,
        ("deny", "allow") => result.deny_to_allow += 1,
        _ => result.other_changes += 1,
    }
    if result.samples.len() < MAX_SAMPLES {
        result.samples.push(FlipSample {
            decision_id: row.decision_id,
            timestamp: row.timestamp,
            principal,
            action,
            resource,
            old_decision: old_decision.to_string(),
            new_decision: new_decision.to_string(),
            old_matched_rule: row.matched_rule,
            new_policy_name: outcome.policy_name,
            new_matched_rule: outcome.matched_rule,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use policy_engine::{PolicyLanguage, PolicyRule};

    /// A headless engine with one Simple policy allowing exactly the given
    /// resources (the Simple matcher is exact-or-`*`), default deny otherwise
    /// (the engine's evaluate_set default).
    fn engine_allowing(resources: &[&str]) -> (PolicyEngine, Vec<Uuid>) {
        let policy = EnhancedPolicy::new(
            format!("allow-{}", resources.join("+")),
            "replay test".to_string(),
            resources
                .iter()
                .map(|r| PolicyRule {
                    action: PolicyAction::Allow,
                    resource: r.to_string(),
                    conditions: vec![],
                })
                .collect(),
        );
        assert_eq!(policy.language, PolicyLanguage::Simple);
        let id = policy.id;
        let engine = PolicyEngine::new();
        engine.deploy_policy(policy).unwrap();
        (engine, vec![id])
    }

    fn row(decision: &str, resource: &str, replayable: bool) -> DecisionRow {
        DecisionRow {
            timestamp: "2026-07-01 10:00:00.000".to_string(),
            decision_id: Uuid::new_v4().to_string(),
            trace_id: String::new(),
            principal: "alice".to_string(),
            action: "read".to_string(),
            resource: resource.to_string(),
            decision: decision.to_string(),
            policy_id: "p-old".to_string(),
            policy_name: "old-policy".to_string(),
            policy_version: "1".to_string(),
            matched_rule: "rule_0".to_string(),
            evaluation_time_ns: 400,
            cache_hit: 0,
            agent_id: "agent-1".to_string(),
            context: Value::Null,
            replay_input: if replayable {
                serde_json::json!({
                    "principal": "alice",
                    "action": "read",
                    "resource": resource,
                    "context": {"region": "eu"}
                })
            } else {
                Value::Null
            },
            input_data: Value::Null,
        }
    }

    fn request() -> ReplayRequest {
        ReplayRequest {
            bundle_id: Uuid::new_v4(),
            from: None,
            to: None,
            filter: ReplayFilter::default(),
            namespace: None,
            data_version: None,
            decryption_key: None,
            max_rows: None,
        }
    }

    #[test]
    fn dsl_replay_needs_the_data_snapshot_and_fails_closed_without_it() {
        // The DSL evaluator resolves the principal as an ENTITY from the
        // DataStore — exactly like a production agent running with synced
        // data. Replay is faithful when given the pinned snapshot; without
        // it, evaluation errors and the set semantics fail closed (deny) —
        // never a silently wrong allow.
        let content = "policy replaytest {\n    default: deny,\n    rule allow_user_read {\n        allow if {\n            context.action == \"read\" && context.principal == \"user\"\n        }\n    }\n}";
        let build = |store: Arc<DataStore>| {
            let mut policy = EnhancedPolicy::new("p".into(), "".into(), vec![]);
            policy.content = content.to_string();
            policy.language = PolicyLanguage::ReaperDsl;
            policy.build_evaluator_with_data(Some(store)).unwrap();
            let id = policy.id;
            let engine = PolicyEngine::new();
            engine.deploy_policy(policy).unwrap();
            (engine, vec![id])
        };
        let eval = |engine: &PolicyEngine, ids: &[Uuid]| {
            let mut context = HashMap::new();
            context.insert("principal".to_string(), "user".to_string());
            engine.evaluate_set(
                ids,
                &PolicyRequest {
                    resource: "/api".to_string(),
                    action: "read".to_string(),
                    context,
                },
            )
        };

        // Empty snapshot: unknown principal -> error -> deny (fail closed).
        let (engine, ids) = build(Arc::new(DataStore::new()));
        let out = eval(&engine, &ids);
        assert_eq!(out.decision, PolicyAction::Deny);
        assert!(out.error.is_some(), "the miss is loud, not silent");

        // With the snapshot (as a pinned data_version provides): allows.
        let store = Arc::new(DataStore::new());
        DataLoader::new((*store).clone())
            .load_json(r#"{"entities": [{"id": "user", "type": "user", "attributes": {}}]}"#)
            .unwrap();
        let (engine, ids) = build(store);
        let out = eval(&engine, &ids);
        assert_eq!(out.decision, PolicyAction::Allow, "err={:?}", out.error);
    }

    #[test]
    fn reproduction_under_equivalent_policy_yields_zero_flips() {
        // The historical decisions were made by "allow /public*, deny rest".
        // Replaying under the SAME logic must reproduce every decision — the
        // sanity check that the replay engine itself doesn't distort.
        let (engine, ids) = engine_allowing(&["/public/1", "/public/2"]);
        let mut result = ReplayResult::default();
        let req = request();

        replay_row(
            &engine,
            &ids,
            &req,
            row("allow", "/public/1", true),
            &mut result,
        );
        replay_row(
            &engine,
            &ids,
            &req,
            row("allow", "/public/2", true),
            &mut result,
        );
        replay_row(
            &engine,
            &ids,
            &req,
            row("deny", "/admin/x", true),
            &mut result,
        );

        assert_eq!(result.replayed, 3);
        assert_eq!(result.unchanged, 3, "zero flips on reproduction");
        assert_eq!(result.allow_to_deny + result.deny_to_allow, 0);
        assert!(result.samples.is_empty());
    }

    #[test]
    fn inverted_policy_counts_flips_exactly_and_samples_carry_attribution() {
        // Target bundle allows /admin* instead of /public* — every historical
        // decision flips, in the exact direction expected.
        let (engine, ids) = engine_allowing(&["/admin/x"]);
        let mut result = ReplayResult::default();
        let req = request();

        replay_row(
            &engine,
            &ids,
            &req,
            row("allow", "/public/1", true),
            &mut result,
        );
        replay_row(
            &engine,
            &ids,
            &req,
            row("allow", "/public/2", true),
            &mut result,
        );
        replay_row(
            &engine,
            &ids,
            &req,
            row("deny", "/admin/x", true),
            &mut result,
        );

        assert_eq!(result.replayed, 3);
        assert_eq!(result.allow_to_deny, 2, "/public allows now deny");
        assert_eq!(result.deny_to_allow, 1, "/admin deny now allows");
        assert_eq!(result.unchanged, 0);
        assert_eq!(result.samples.len(), 3);

        let flip = result
            .samples
            .iter()
            .find(|s| s.old_decision == "deny")
            .expect("deny→allow sample");
        assert_eq!(flip.new_decision, "allow");
        assert_eq!(flip.resource, "/admin/x");
        assert_eq!(flip.old_matched_rule, "rule_0");
        assert_eq!(flip.new_policy_name, "allow-/admin/x");
    }

    #[test]
    fn non_replayable_rows_are_skipped_and_counted() {
        let (engine, ids) = engine_allowing(&["*"]);
        let mut result = ReplayResult::default();
        let req = request();

        replay_row(&engine, &ids, &req, row("allow", "/x", false), &mut result);
        assert_eq!(result.replayed, 0);
        assert_eq!(result.skipped_not_replayable, 1);
    }

    #[test]
    fn encrypted_blob_needs_the_key_and_replays_with_it() {
        use policy_engine::{DataProtection, DecisionLogConfig, DecisionLogEntry};

        // Seal a replay blob through the REAL capture-time protection path.
        let key_hex = "33".repeat(32);
        let config = DecisionLogConfig {
            encrypt_input_data: true,
            encryption_key: Some(key_hex.clone()),
            ..Default::default()
        };
        let protection = DataProtection::from_config(&config).unwrap().unwrap();
        let mut entry = DecisionLogEntry::new(
            "alice".into(),
            "read".into(),
            "/public/1".into(),
            "allow".into(),
            "p".into(),
            "pol".into(),
        );
        entry.replay_input = Some(serde_json::json!({
            "principal": "alice", "action": "read",
            "resource": "/public/1", "context": {}
        }));
        protection.apply(&mut entry).unwrap();
        let sealed = entry.replay_input.unwrap();
        assert_eq!(sealed["enc"], "aes256gcm");

        let (engine, ids) = engine_allowing(&["/public/1"]);
        let mut sealed_row = row("allow", "/public/1", false);
        sealed_row.replay_input = sealed;

        // Without the key: counted, never guessed at.
        let mut result = ReplayResult::default();
        replay_row(&engine, &ids, &request(), sealed_row.clone(), &mut result);
        assert_eq!(result.skipped_encrypted, 1);
        assert_eq!(result.replayed, 0);

        // With the tenant key: opens and reproduces the decision.
        let mut req = request();
        req.decryption_key = Some(key_hex);
        let mut result = ReplayResult::default();
        replay_row(&engine, &ids, &req, sealed_row, &mut result);
        assert_eq!(result.replayed, 1);
        assert_eq!(result.unchanged, 1);
    }
}
