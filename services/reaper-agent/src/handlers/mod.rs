//! HTTP request handlers for the Reaper Agent.
//!
//! This module organizes handlers by domain:
//! - `health`: Health checks, readiness, liveness, metrics
//! - `evaluate`: Policy evaluation endpoints
//! - `policies`: Policy deployment and management
//! - `entities`: Entity CRUD operations
//! - `data`: Data loading and synchronization
//! - `decisions`: Decision logging and analytics

pub mod check;
pub mod data;
pub mod decisions;
pub mod entities;
pub mod evaluate;
pub mod health;
pub mod policies;

// Re-export health handlers
pub use health::{health_check, liveness_check, metrics, readiness_check};

// Re-export evaluation handlers
pub use check::check_document;
pub use evaluate::{batch_evaluate_policy, evaluate_policy, fast_evaluate_policy};

// Re-export policy management handlers
pub use policies::{
    deploy_bundle, deploy_compiled_policy, deploy_policy, get_policy_current_version,
    get_policy_versions, list_policies, load_bundles_atomic, DeployCompiledPolicyRequest,
};

// Re-export data handlers
pub use data::{
    deploy_data_version, load_data_handler, load_data_stream_handler, sync_data, LoadDataRequest,
    SyncDataRequest, SyncDataResponse,
};

// Re-export entity handlers
pub use entities::{
    batch_upsert_handler, debug_datastore, delete_entity_handler, get_entity_handler,
    list_entities_handler, upsert_entity_handler, BatchUpsertRequest, BatchUpsertResponse,
    EntityResponse, ListEntitiesResponse, ListParams, UpsertEntityRequest,
};

// Re-export decision handlers
pub use decisions::{
    export_decisions, get_decision_by_id, get_decision_stats, get_decisions, DecisionQueryParams,
};
