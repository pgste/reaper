//! Data loading and synchronization handlers.
//!
//! This module contains handlers for entity data management:
//! - `load_data_handler` - Load entity data from JSON
//! - `load_data_stream_handler` - Load entity data using streaming (memory-efficient)
//! - `sync_data` - Synchronize entity data from external source

use axum::{
    body::Bytes,
    extract::State,
    http::StatusCode,
    response::Json,
};
use policy_engine::{AttributeValue, DataLoader, EntityBuilder, StreamingLoader, StringInterner};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{error, info, instrument, warn};

use crate::state::AgentState;

// ============================================================================
// Data Loading Types
// ============================================================================

/// Request to load entity data from JSON.
#[derive(Debug, Deserialize)]
pub struct LoadDataRequest {
    /// Raw JSON string with entities
    pub data: String,
}

/// Request to synchronize entity data from a management server.
#[derive(Debug, Deserialize)]
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
#[derive(Debug, Deserialize)]
pub struct SyncEntityData {
    /// Unique entity identifier
    pub id: String,
    /// Entity type (e.g., "User", "Resource", "Group")
    pub entity_type: String,
    /// Entity attributes as key-value pairs
    pub attributes: serde_json::Map<String, serde_json::Value>,
    /// Optional parent entity ID (for hierarchies)
    pub parent: Option<String>,
}

/// Source information for tracking where the sync came from.
#[derive(Debug, Deserialize)]
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
#[derive(Debug, serde::Serialize)]
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

    Ok(Json(json!({
        "status": "success",
        "entities_loaded": entity_count,
        "message": format!("Loaded {} entities successfully", entity_count)
    })))
}

/// Load entity data using streaming for memory efficiency.
///
/// Accepts file content as raw bytes in request body.
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
