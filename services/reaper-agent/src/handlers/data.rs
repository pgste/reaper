//! Data loading and synchronization handlers.
//!
//! This module contains handlers for entity data management:
//! - `load_data_handler` - Load entity data from JSON
//! - `load_data_stream_handler` - Load entity data using streaming (memory-efficient)
//! - `sync_data` - Synchronize entity data from external source

use axum::{body::Bytes, extract::State, http::StatusCode, response::Json};
use policy_engine::{AttributeValue, DataLoader, EntityBuilder, StreamingLoader, StringInterner};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{error, info, instrument, warn};
use utoipa::ToSchema;

use crate::state::AgentState;

// ============================================================================
// Data Loading Types
// ============================================================================

/// Request to load entity data from JSON.
#[derive(Debug, Deserialize, ToSchema)]
pub struct LoadDataRequest {
    /// Raw JSON string with entities
    pub data: String,
}

/// Request to synchronize entity data from a management server.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SyncDataRequest {
    /// List of entities to sync
    pub entities: Vec<SyncEntityData>,
    /// If true, clear all existing entities before inserting
    #[serde(default)]
    pub replace_all: bool,
    /// Optional source metadata for tracking
    pub source: Option<SyncSource>,
}

/// Entity data for sync endpoint.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SyncEntityData {
    /// Unique entity identifier
    pub id: String,
    /// Entity type (e.g., "User", "Resource", "Group")
    pub entity_type: String,
    /// Entity attributes as key-value pairs
    #[schema(value_type = Object)]
    pub attributes: serde_json::Map<String, serde_json::Value>,
    /// Optional parent entity ID (for hierarchies)
    pub parent: Option<String>,
}

/// Source information for tracking where the sync came from.
#[derive(Debug, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SyncSource {
    /// Source type: "sync-client", "api", "file"
    #[serde(rename = "type")]
    pub source_type: String,
    /// Server URL if from sync client
    pub server_url: Option<String>,
    /// Server version for tracking
    pub server_version: Option<String>,
    /// Team/namespace if applicable
    pub team: Option<String>,
}

/// Response from sync operation.
#[derive(Debug, serde::Serialize, ToSchema)]
pub struct SyncDataResponse {
    pub status: String,
    pub inserted: usize,
    pub failed: usize,
    pub replaced: bool,
    pub total_entities: usize,
}

// ============================================================================
// Handlers
// ============================================================================

/// Load entity data (JSON) into the agent's DataStore.
#[utoipa::path(
    post,
    path = "/api/v1/data",
    tag = "data",
    request_body = LoadDataRequest,
    responses(
        (status = 200, description = "Entities loaded into the DataStore")
    ),
    security(("bearer_jwt" = []))
)]
#[instrument(skip(state, payload))]
pub async fn load_data_handler(
    State(state): State<Arc<AgentState>>,
    Json(payload): Json<LoadDataRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    info!("Loading entity data into DataStore");

    // DataStore uses Arc internally, so cloning is cheap and shares data
    let loader = DataLoader::new((*state.data_store).clone());
    let entity_count = loader.load_json(&payload.data).map_err(|e| {
        error!("Failed to load entity data: {}", e);
        (
            StatusCode::BAD_REQUEST,
            format!("Failed to load entity data: {}", e),
        )
    })?;

    info!("✓ Loaded {} entities into DataStore", entity_count);

    // Entity attributes changed — cached decisions may now be wrong.
    if let Some(ref cache) = state.decision_cache {
        cache.invalidate();
    }

    Ok(Json(json!({
        "status": "success",
        "entities_loaded": entity_count,
        "message": format!("Loaded {} entities successfully", entity_count)
    })))
}

/// Load entity data using streaming for memory efficiency.
///
/// Accepts file content as raw bytes in request body.
#[utoipa::path(
    post,
    path = "/api/v1/data/stream",
    tag = "data",
    responses(
        (status = 200, description = "Entities streamed into the DataStore")
    ),
    security(("bearer_jwt" = []))
)]
#[instrument(skip(state, body))]
pub async fn load_data_stream_handler(
    State(state): State<Arc<AgentState>>,
    body: Bytes,
) -> Result<Json<Value>, (StatusCode, String)> {
    info!("Loading entity data using streaming (memory-efficient)");

    use std::io::Write;
    use tempfile::NamedTempFile;

    // Write incoming data to temp file
    let mut temp_file = NamedTempFile::new().map_err(|e| {
        error!("Failed to create temp file: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to create temp file: {}", e),
        )
    })?;

    temp_file.write_all(&body).map_err(|e| {
        error!("Failed to write to temp file: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to write to temp file: {}", e),
        )
    })?;

    temp_file.flush().map_err(|e| {
        error!("Failed to flush temp file: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to flush temp file: {}", e),
        )
    })?;

    let temp_path = temp_file.path();

    // Use streaming loader with 10K chunk size
    let loader = DataLoader::new((*state.data_store).clone());
    let streaming_loader = StreamingLoader::new(loader, 10_000);

    let stats = streaming_loader.stream_and_load(temp_path).map_err(|e| {
        error!("Failed to stream entity data: {}", e);
        (
            StatusCode::BAD_REQUEST,
            format!("Failed to stream entity data: {}", e),
        )
    })?;

    info!(
        "✓ Streamed {} entities in {} chunks ({:.2}s)",
        stats.total,
        stats.chunks_processed,
        stats.duration.as_secs_f64()
    );

    // Entity attributes changed — cached decisions may now be wrong.
    if let Some(ref cache) = state.decision_cache {
        cache.invalidate();
    }

    Ok(Json(json!({
        "status": "success",
        "entities_loaded": stats.total,
        "chunks_processed": stats.chunks_processed,
        "duration_ms": stats.duration.as_millis(),
        "message": format!("Streamed {} entities in {} chunks", stats.total, stats.chunks_processed)
    })))
}

/// Synchronize entity data from external source (sync client or management server).
///
/// POST /api/v1/data/sync
///
/// This endpoint supports bulk entity synchronization with optional replace-all semantics.
#[utoipa::path(
    post,
    path = "/api/v1/data/sync",
    tag = "data",
    request_body = SyncDataRequest,
    responses(
        (status = 200, description = "Sync result", body = SyncDataResponse)
    ),
    security(("bearer_jwt" = []))
)]
#[instrument(skip(state, payload))]
pub async fn sync_data(
    State(state): State<Arc<AgentState>>,
    Json(payload): Json<SyncDataRequest>,
) -> Result<Json<SyncDataResponse>, (StatusCode, String)> {
    let source_info = payload
        .source
        .as_ref()
        .map(|s| {
            format!(
                "{} ({})",
                s.source_type,
                s.server_url.as_deref().unwrap_or("local")
            )
        })
        .unwrap_or_else(|| "api".to_string());

    info!(
        "Syncing entity data: {} entities, replace_all={}, source={}",
        payload.entities.len(),
        payload.replace_all,
        source_info
    );

    // Clear existing data if replace_all is true
    if payload.replace_all {
        info!("Clearing existing entity data (replace_all=true)");
        state.data_store.clear();
    }

    // Get the string interner from the data store
    let interner = state.data_store.interner();

    // Convert and insert entities
    let mut inserted = 0;
    let mut failed = 0;

    for entity_data in &payload.entities {
        match convert_sync_entity(entity_data, interner) {
            Ok(entity) => {
                state.data_store.insert(entity);
                inserted += 1;
            }
            Err(e) => {
                warn!(
                    "Failed to convert entity {}/{}: {}",
                    entity_data.entity_type, entity_data.id, e
                );
                failed += 1;
            }
        }
    }

    let total = state.data_store.all().len();

    info!(
        "✓ Sync complete: inserted={}, failed={}, total_entities={}",
        inserted, failed, total
    );

    // Entity data changed — cached decisions may now be wrong.
    if inserted > 0 || payload.replace_all {
        if let Some(ref cache) = state.decision_cache {
            cache.invalidate();
        }
    }

    Ok(Json(SyncDataResponse {
        status: if failed == 0 {
            "success".to_string()
        } else {
            "partial".to_string()
        },
        inserted,
        failed,
        replaced: payload.replace_all,
        total_entities: total,
    }))
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Convert JSON entity data to the internal Entity format.
fn convert_sync_entity(
    data: &SyncEntityData,
    interner: &StringInterner,
) -> Result<policy_engine::Entity, String> {
    // Intern the entity ID and type
    let entity_id = interner.intern(&data.id);
    let entity_type = interner.intern(&data.entity_type);

    // Build entity using EntityBuilder
    let mut builder = EntityBuilder::new(entity_id, entity_type);

    // Convert attributes from JSON to AttributeValue
    for (key, value) in &data.attributes {
        let attr_key = interner.intern(key);
        let attr_value = json_to_attribute_value(value, interner)?;
        builder = builder.with_attribute(attr_key, attr_value);
    }

    // Add parent if specified
    if let Some(parent_id) = &data.parent {
        let parent_interned = interner.intern(parent_id);
        builder = builder.with_parent(parent_interned);
    }

    Ok(builder.build())
}

/// Convert a JSON value to an AttributeValue.
fn json_to_attribute_value(
    value: &serde_json::Value,
    interner: &StringInterner,
) -> Result<AttributeValue, String> {
    match value {
        serde_json::Value::Null => Ok(AttributeValue::Null),
        serde_json::Value::Bool(b) => Ok(AttributeValue::Bool(*b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(AttributeValue::Int(i))
            } else if let Some(f) = n.as_f64() {
                Ok(AttributeValue::Float(f))
            } else {
                Err(format!("Unsupported number format: {}", n))
            }
        }
        serde_json::Value::String(s) => Ok(AttributeValue::String(interner.intern(s))),
        serde_json::Value::Array(arr) => {
            let items: Result<Vec<_>, _> = arr
                .iter()
                .map(|v| json_to_attribute_value(v, interner))
                .collect();
            Ok(AttributeValue::List(items?))
        }
        serde_json::Value::Object(obj) => {
            let mut map = HashMap::new();
            for (k, v) in obj {
                let key = interner.intern(k);
                let val = json_to_attribute_value(v, interner)?;
                map.insert(key, val);
            }
            Ok(AttributeValue::Object(map))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_to_attribute_value_primitives() {
        let interner = StringInterner::new();

        assert!(matches!(
            json_to_attribute_value(&serde_json::json!(null), &interner),
            Ok(AttributeValue::Null)
        ));

        assert!(matches!(
            json_to_attribute_value(&serde_json::json!(true), &interner),
            Ok(AttributeValue::Bool(true))
        ));

        assert!(matches!(
            json_to_attribute_value(&serde_json::json!(42), &interner),
            Ok(AttributeValue::Int(42))
        ));
    }
}

// ============================================================================
// Versioned data-plane deployment (read-replica style)
// ============================================================================

/// A published datastore version from the control plane
/// (`GET /orgs/{o}/namespaces/{n}/datastore/versions/{v}`).
#[derive(Debug, Deserialize, ToSchema)]
pub struct DeployDataVersionRequest {
    /// Monotonic version number from the control plane.
    pub version: i64,
    /// The snapshot's position in the change stream (adm_versions
    /// .change_seq) — delta pulls resume from here. 0 for legacy callers.
    #[serde(default)]
    pub change_seq: i64,
    /// Published checksum ("sha256:…") — verified before anything loads.
    pub checksum: String,
    /// Model-shape version the document was materialized under (decision
    /// provenance, Plan 12); 0 for legacy control planes.
    #[serde(default)]
    pub model_version: i64,
    /// The materialized document: `{"entities": [...]}`.
    pub document: Value,
    /// Replace the whole store (default). `false` merges (advanced use).
    #[serde(default = "default_replace")]
    pub replace: bool,
}

fn default_replace() -> bool {
    true
}

/// Deploy a VERIFIED, versioned data bundle — the data-plane sync path.
///
/// Integrity contract (read-replica discipline): the sha256 checksum
/// published by the control plane is recomputed here over the CANONICAL
/// serialization and must match before the store is touched. Canonical form
/// is serde_json's sorted-key output — deliberately NOT sonic_rs, which
/// preserves insertion order and would make the hash depend on transport
/// ordering. A corrupt or tampered payload is rejected like a bad WAL
/// segment; version regressions are rejected to keep sync monotonic.
#[utoipa::path(
    post,
    path = "/api/v1/data/deploy-version",
    tag = "data",
    request_body = DeployDataVersionRequest,
    responses(
        (status = 200, description = "Verified data version deployed, already current, or a conflict")
    ),
    security(("bearer_jwt" = []))
)]
pub async fn deploy_data_version(
    State(state): State<Arc<AgentState>>,
    Json(payload): Json<DeployDataVersionRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    use sha2::{Digest, Sha256};

    let current = state
        .data_sync
        .version
        .load(std::sync::atomic::Ordering::Acquire);
    if payload.version < current {
        return Err((
            StatusCode::CONFLICT,
            format!(
                "version {} is older than current {} (sync must be monotonic)",
                payload.version, current
            ),
        ));
    }

    // Canonical serialization -> checksum verification.
    let canonical = serde_json::to_string(&payload.document).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("unserializable document: {e}"),
        )
    })?;
    let computed = format!("{:x}", Sha256::digest(canonical.as_bytes()));
    let expected = payload
        .checksum
        .strip_prefix("sha256:")
        .unwrap_or(&payload.checksum);
    if !computed.eq_ignore_ascii_case(expected) {
        error!(
            version = payload.version,
            expected, computed, "data version REJECTED: checksum mismatch"
        );
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            format!(
                "checksum mismatch for version {}: expected {expected}, computed {computed} \
                 — payload rejected, store untouched",
                payload.version
            ),
        ));
    }

    // Idempotent redelivery of the current version. The payload checksum
    // already verified above; it must ALSO match what we deployed — same
    // version with different content is a split brain, never a no-op.
    if payload.version == current && current != 0 {
        let stored = state.data_sync.checksum.read().clone();
        if !stored.eq_ignore_ascii_case(&payload.checksum) {
            error!(
                version = current,
                stored,
                payload = payload.checksum,
                "data version DIVERGENCE: same version, different checksum"
            );
            return Err((
                StatusCode::CONFLICT,
                format!(
                    "divergence at version {current}: agent has {stored}, control plane \
                     sent {} — refusing to guess; republish a new version",
                    payload.checksum
                ),
            ));
        }
        // Verified current = replica heartbeat: refresh the staleness clock.
        state.data_sync.record_heartbeat();
        return Ok(Json(json!({
            "version": current,
            "status": "already_current",
        })));
    }

    if payload.replace {
        state.data_store.clear();
    }
    let loader = DataLoader::new((*state.data_store).clone());
    let entity_count = loader.load_json(&canonical).map_err(|e| {
        error!("failed to load verified data version: {e}");
        (
            StatusCode::BAD_REQUEST,
            format!("failed to load data version: {e}"),
        )
    })?;

    state.data_sync.record_sync(
        payload.version,
        payload.checksum.clone(),
        payload.model_version,
    );
    state
        .data_sync
        .applied_seq
        .store(payload.change_seq, std::sync::atomic::Ordering::Release);

    if let Some(ref cache) = state.decision_cache {
        cache.invalidate();
    }

    info!(
        version = payload.version,
        entities = entity_count,
        "✓ data version deployed (checksum verified)"
    );

    Ok(Json(json!({
        "version": payload.version,
        "checksum": payload.checksum,
        "entities_loaded": entity_count,
        "status": "deployed",
    })))
}

/// Lightweight replica heartbeat: the sync client confirms the agent is
/// still on the control plane's current version WITHOUT shipping the
/// document. Match -> staleness clock refreshes. Version/checksum mismatch
/// -> 409, telling the sync client to push a full deploy-version.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ConfirmDataVersionRequest {
    pub version: i64,
    pub checksum: String,
}

#[utoipa::path(
    post,
    path = "/api/v1/data/confirm-version",
    tag = "data",
    request_body = ConfirmDataVersionRequest,
    responses(
        (status = 200, description = "Replica confirmed on the control plane's current version")
    ),
    security(("bearer_jwt" = []))
)]
pub async fn confirm_data_version(
    State(state): State<Arc<AgentState>>,
    Json(payload): Json<ConfirmDataVersionRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let current = state
        .data_sync
        .version
        .load(std::sync::atomic::Ordering::Acquire);
    if current == 0 {
        return Err((
            StatusCode::CONFLICT,
            "agent has never synced — push a full deploy-version".to_string(),
        ));
    }
    if payload.version != current {
        return Err((
            StatusCode::CONFLICT,
            format!(
                "agent at version {current}, control plane at {} — push deploy-version",
                payload.version
            ),
        ));
    }
    let stored = state.data_sync.checksum.read().clone();
    if !stored.eq_ignore_ascii_case(&payload.checksum) {
        return Err((
            StatusCode::CONFLICT,
            format!(
                "divergence at version {current}: agent has {stored}, control plane has {}",
                payload.checksum
            ),
        ));
    }
    state.data_sync.record_heartbeat();
    Ok(Json(json!({"version": current, "status": "confirmed"})))
}

/// One delta from the control plane's change stream.
#[derive(Debug, Deserialize, ToSchema)]
pub struct DataDelta {
    pub op: String, // "upsert" | "delete"
    pub entity_id: String,
    #[serde(default)]
    pub document: Option<Value>,
}

/// A contiguous slice of the change stream.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ApplyDeltasRequest {
    /// The seq the replica must currently be at (exclusive start).
    pub from_seq: i64,
    /// The seq this batch advances to.
    pub head_seq: i64,
    pub deltas: Vec<DataDelta>,
}

/// Apply a contiguous delta batch — the incremental half of read-replica
/// sync. CONTIGUITY IS THE INTEGRITY RULE: the batch must start exactly at
/// this replica's applied_seq; anything else gets a 409 carrying the
/// replica's actual position, so the sync client re-pulls precisely the
/// missing range (self-retrying, gap-proof). Deltas are entity-level
/// last-state upserts/tombstones — idempotent under at-least-once
/// delivery, proven equivalent to a fresh rebuild by
/// delta_sync_differential_tests.
#[utoipa::path(
    post,
    path = "/api/v1/data/apply-deltas",
    tag = "data",
    request_body = ApplyDeltasRequest,
    responses(
        (status = 200, description = "Contiguous delta batch applied")
    ),
    security(("bearer_jwt" = []))
)]
pub async fn apply_data_deltas(
    State(state): State<Arc<AgentState>>,
    Json(payload): Json<ApplyDeltasRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    use std::sync::atomic::Ordering;

    let current = state.data_sync.applied_seq.load(Ordering::Acquire);
    if payload.from_seq != current {
        return Err((
            StatusCode::CONFLICT,
            // The sync client parses applied_seq out of this body to
            // re-pull from the right position.
            format!(
                "{{\"error\":\"seq_mismatch\",\"applied_seq\":{current},\"requested_from\":{}}}",
                payload.from_seq
            ),
        ));
    }
    if payload.head_seq < payload.from_seq {
        return Err((
            StatusCode::BAD_REQUEST,
            "head_seq must be >= from_seq".to_string(),
        ));
    }

    let loader = DataLoader::new((*state.data_store).clone());
    let mut upserts = 0usize;
    let mut deletes = 0usize;
    for delta in &payload.deltas {
        match delta.op.as_str() {
            "upsert" => {
                let doc = delta.document.as_ref().ok_or((
                    StatusCode::BAD_REQUEST,
                    format!("upsert delta for '{}' missing document", delta.entity_id),
                ))?;
                loader.upsert_entity_doc(doc).map_err(|e| {
                    (
                        StatusCode::BAD_REQUEST,
                        format!("delta for '{}' failed: {e}", delta.entity_id),
                    )
                })?;
                upserts += 1;
            }
            "delete" => {
                loader.delete_entity(&delta.entity_id);
                deletes += 1;
            }
            other => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("unknown delta op '{other}'"),
                ));
            }
        }
    }

    state
        .data_sync
        .applied_seq
        .store(payload.head_seq, Ordering::Release);
    // Advancing through the change stream IS a successful sync contact.
    state.data_sync.record_heartbeat();

    if (upserts + deletes) > 0 {
        if let Some(ref cache) = state.decision_cache {
            cache.invalidate();
        }
    }

    info!(
        from = payload.from_seq,
        to = payload.head_seq,
        upserts,
        deletes,
        "✓ delta batch applied"
    );
    Ok(Json(json!({
        "applied_seq": payload.head_seq,
        "upserts": upserts,
        "deletes": deletes,
        "status": "applied",
    })))
}

#[cfg(test)]
mod deploy_version_tests {
    use super::*;
    use crate::state::{DataSyncState, StalenessMode};

    #[test]
    fn checksum_canonical_form_matches_control_plane() {
        use sha2::{Digest, Sha256};
        // The control plane hashes serde_json::to_string of the document.
        // Round-tripping through Value must produce the same bytes (sorted
        // keys — this is why the checksum path never uses sonic_rs).
        let doc = serde_json::json!({
            "entities": [
                {"id": "alice", "type": "user",
                 "attributes": {"role": "admin", "clearance": 5}}
            ]
        });
        let a = serde_json::to_string(&doc).unwrap();
        let reparsed: Value = serde_json::from_str(&a).unwrap();
        let b = serde_json::to_string(&reparsed).unwrap();
        assert_eq!(a, b, "canonical serialization must be stable");
        assert_eq!(Sha256::digest(a.as_bytes()), Sha256::digest(b.as_bytes()));
    }

    #[test]
    fn staleness_budget_semantics() {
        std::env::remove_var("REAPER_DATA_MAX_STALENESS_SECS");
        std::env::remove_var("REAPER_DATA_STALENESS_MODE");

        // Never-synced agent (bootstrap/standalone): no staleness clock.
        let s = DataSyncState {
            version: std::sync::atomic::AtomicI64::new(0),
            model_version: std::sync::atomic::AtomicI64::new(0),
            checksum: parking_lot::RwLock::new(String::new()),
            last_synced_epoch: std::sync::atomic::AtomicU64::new(0),
            applied_seq: std::sync::atomic::AtomicI64::new(0),
            max_staleness_secs: 10,
            mode: StalenessMode::Enforce,
            require_sync: false,
        };
        assert!(!s.is_stale(), "never-synced has no staleness clock");
        assert!(!s.must_deny());
        assert!(
            !s.awaiting_initial_sync(),
            "gate must be off unless explicitly armed"
        );

        // Synced long ago with a 10s budget: stale; behavior follows mode.
        s.record_sync(1, "sha256:abc".into(), 3);
        s.last_synced_epoch
            .store(1, std::sync::atomic::Ordering::Release); // epoch 1 = 1970
        assert!(s.is_stale());
        assert!(s.must_deny(), "enforce mode fails closed");
        assert!(s.flag_stale());

        let (version, checksum) = s.provenance();
        assert_eq!(version, 1);
        assert_eq!(checksum.as_deref(), Some("sha256:abc"));

        // REPLICA-LAG SEMANTICS: a verified "already current" heartbeat
        // refreshes the clock without a new version — staleness measures
        // lag behind the primary, not time since the last publish.
        s.record_heartbeat();
        assert!(!s.is_stale(), "heartbeat must clear staleness");
        assert!(!s.must_deny());
        assert_eq!(
            s.version.load(std::sync::atomic::Ordering::Acquire),
            1,
            "heartbeat never changes the version"
        );
    }

    #[test]
    fn require_sync_cold_start_gate() {
        // Armed gate, never synced: fail closed with the cold-start
        // reason, NOT the staleness reason (fresh pod != stale replica).
        let s = DataSyncState {
            version: std::sync::atomic::AtomicI64::new(0),
            model_version: std::sync::atomic::AtomicI64::new(0),
            checksum: parking_lot::RwLock::new(String::new()),
            last_synced_epoch: std::sync::atomic::AtomicU64::new(0),
            applied_seq: std::sync::atomic::AtomicI64::new(0),
            max_staleness_secs: 0,
            mode: StalenessMode::Monitor,
            require_sync: true,
        };
        assert!(s.awaiting_initial_sync());
        assert!(s.must_deny(), "armed gate fails closed before first sync");
        assert_eq!(s.deny_reason(), Some("awaiting_initial_data_sync"));

        // A heartbeat alone must NOT open the gate — only a VERIFIED
        // snapshot (record_sync runs after checksum verification) counts.
        s.record_heartbeat();
        assert!(
            s.awaiting_initial_sync(),
            "heartbeat without a verified snapshot must not open the gate"
        );

        // First verified snapshot: gate opens permanently.
        s.record_sync(1, "sha256:abc".into(), 3);
        assert!(!s.awaiting_initial_sync());
        assert!(!s.must_deny());
        assert_eq!(s.deny_reason(), None);
    }
}
