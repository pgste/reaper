//! Entity CRUD operation handlers.
//!
//! This module contains handlers for entity management operations:
//! - `upsert_entity_handler` - Create or update entity
//! - `get_entity_handler` - Get entity by type and ID
//! - `delete_entity_handler` - Delete entity
//! - `list_entities_handler` - List entities of a type
//! - `batch_upsert_handler` - Batch upsert entities
//! - `debug_datastore` - Debug endpoint for DataStore stats
//!
//! NOTE: These endpoints define the API contract for entity management.
//! Full implementation requires eBPF integration with entity maps.
//! Currently returns stub responses for API compatibility.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, instrument};

use crate::state::AgentState;

// ============================================================================
// Entity Types
// ============================================================================

/// Request to create or update an entity.
#[derive(Debug, Deserialize)]
pub struct UpsertEntityRequest {
    pub entity_type: String,
    pub entity_id: String,
    pub string_attrs: HashMap<String, String>,
    pub numeric_attrs: HashMap<String, i64>,
    pub relationships: Vec<RelationshipRequest>,
    pub flags: HashMap<String, bool>,
}

/// Relationship data for entity requests.
#[derive(Debug, Deserialize)]
pub struct RelationshipRequest {
    pub rel_type: String,
    pub target: String,
}

/// Entity response for GET and upsert operations.
#[derive(Debug, serde::Serialize)]
pub struct EntityResponse {
    pub entity_id: String,
    pub entity_type: String,
    pub version: u32,
    pub created_at: String,
    pub updated_at: String,
    pub string_attrs: HashMap<String, String>,
    pub numeric_attrs: HashMap<String, i64>,
    pub relationships: Vec<RelationshipResponse>,
    pub flags: HashMap<String, bool>,
}

/// Relationship data in responses.
#[derive(Debug, serde::Serialize)]
pub struct RelationshipResponse {
    pub rel_type: String,
    pub target: String,
}

/// Query parameters for listing entities.
#[derive(Debug, Deserialize)]
pub struct ListParams {
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    100
}

/// Response for listing entities.
#[derive(Debug, serde::Serialize)]
pub struct ListEntitiesResponse {
    pub entities: Vec<EntityResponse>,
    pub total: usize,
}

/// Batch upsert request.
#[derive(Debug, Deserialize)]
pub struct BatchUpsertRequest {
    pub entities: Vec<UpsertEntityRequest>,
}

/// Batch upsert response.
#[derive(Debug, serde::Serialize)]
pub struct BatchUpsertResponse {
    pub succeeded: usize,
    pub failed: usize,
    pub errors: Vec<(String, String)>, // (entity_id, error)
}

// ============================================================================
// Handlers
// ============================================================================

/// POST /api/v1/entities - Create or update entity.
#[instrument(skip(state))]
pub async fn upsert_entity_handler(
    State(state): State<Arc<AgentState>>,
    Json(req): Json<UpsertEntityRequest>,
) -> Result<Json<EntityResponse>, (StatusCode, String)> {
    let _ = state; // Suppress unused warning
                   // TODO: Implement with eBPF entity maps when integrated
    info!(
        "Entity upsert request (stub): type={}, id={}",
        req.entity_type, req.entity_id
    );

    // Return stub response
    let response = EntityResponse {
        entity_id: req.entity_id.clone(),
        entity_type: req.entity_type.clone(),
        version: 1,
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
        string_attrs: req.string_attrs.clone(),
        numeric_attrs: req.numeric_attrs.clone(),
        relationships: req
            .relationships
            .iter()
            .map(|r| RelationshipResponse {
                rel_type: r.rel_type.clone(),
                target: r.target.clone(),
            })
            .collect(),
        flags: req.flags.clone(),
    };

    Ok(Json(response))
}

/// GET /api/v1/entities/:type/:id - Get entity.
#[instrument(skip(state))]
pub async fn get_entity_handler(
    State(state): State<Arc<AgentState>>,
    Path((entity_type, entity_id)): Path<(String, String)>,
) -> Result<Json<EntityResponse>, (StatusCode, String)> {
    let _ = state; // Suppress unused warning
                   // TODO: Implement with eBPF entity maps when integrated
    info!(
        "Entity get request (stub): type={}, id={}",
        entity_type, entity_id
    );

    // Return stub response
    let response = EntityResponse {
        entity_id: entity_id.clone(),
        entity_type: entity_type.clone(),
        version: 1,
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
        string_attrs: HashMap::new(),
        numeric_attrs: HashMap::new(),
        relationships: vec![],
        flags: HashMap::new(),
    };

    Ok(Json(response))
}

/// DELETE /api/v1/entities/:type/:id - Delete entity.
#[instrument(skip(state))]
pub async fn delete_entity_handler(
    State(state): State<Arc<AgentState>>,
    Path((entity_type, entity_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    let _ = state; // Suppress unused warning
                   // TODO: Implement with eBPF entity maps when integrated
    info!(
        "Entity delete request (stub): type={}, id={}",
        entity_type, entity_id
    );

    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/v1/entities/:type - List entities of type.
#[instrument(skip(state))]
pub async fn list_entities_handler(
    State(state): State<Arc<AgentState>>,
    Path(entity_type): Path<String>,
    Query(params): Query<ListParams>,
) -> Result<Json<ListEntitiesResponse>, (StatusCode, String)> {
    let _ = state; // Suppress unused warning
                   // TODO: Implement with eBPF entity maps when integrated
    info!(
        "Entity list request (stub): type={}, limit={}",
        entity_type, params.limit
    );

    // Return stub response
    let response = ListEntitiesResponse {
        entities: vec![],
        total: 0,
    };

    Ok(Json(response))
}

/// POST /api/v1/entities/batch - Batch upsert.
#[instrument(skip(state))]
pub async fn batch_upsert_handler(
    State(state): State<Arc<AgentState>>,
    Json(req): Json<BatchUpsertRequest>,
) -> Result<Json<BatchUpsertResponse>, (StatusCode, String)> {
    let _ = state; // Suppress unused warning
                   // TODO: Implement with eBPF entity maps when integrated
    info!(
        "Batch upsert request (stub): {} entities",
        req.entities.len()
    );

    // Return stub response
    let response = BatchUpsertResponse {
        succeeded: req.entities.len(),
        failed: 0,
        errors: vec![],
    };

    Ok(Json(response))
}

/// Debug endpoint to check DataStore stats.
#[instrument(skip(state))]
pub async fn debug_datastore(
    State(state): State<Arc<AgentState>>,
) -> Result<Json<Value>, StatusCode> {
    let stats = state.data_store.stats();
    Ok(Json(json!({
        "total_entities": stats.total_entities,
        "unique_types": stats.unique_types,
        "indexed_attributes": stats.indexed_attributes
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_limit() {
        assert_eq!(default_limit(), 100);
    }
}
