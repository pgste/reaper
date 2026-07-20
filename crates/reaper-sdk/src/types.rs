//! Core types for the Reaper SDK

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Request to evaluate a policy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRequest {
    /// Policy ID to evaluate
    pub policy_id: String,
    /// Principal (user/service) making the request
    pub principal: String,
    /// Action being performed
    pub action: String,
    /// Resource being accessed
    pub resource: String,
    /// Additional context for evaluation
    #[serde(default)]
    pub context: HashMap<String, String>,
}

/// Response from policy evaluation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyResponse {
    /// Policy decision
    pub decision: Decision,
    /// Evaluation latency in nanoseconds
    #[serde(default)]
    pub latency_ns: u64,
    /// Where the policy was evaluated
    #[serde(default)]
    pub source: Source,
}

/// Policy decision
///
/// `#[non_exhaustive]`: the decision set may grow (e.g. audit/log outcomes),
/// so downstream matches must carry a wildcard arm — treat unknown decisions
/// as deny, never silently allow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Decision {
    /// Allow the request
    Allow,
    /// Deny the request
    Deny,
}

/// Where the policy was evaluated
///
/// `#[non_exhaustive]`: new evaluation locations may be added (e.g. wasm,
/// edge), so downstream matches must carry a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Source {
    /// Evaluated in eBPF kernel
    Ebpf,
    /// Evaluated in userspace
    #[default]
    Userspace,
}

/// Request to deploy a policy bundle
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployBundleRequest {
    /// Raw .rbb bundle bytes
    pub bundle: Vec<u8>,
    /// Expected version
    pub version: String,
    /// Override version check
    #[serde(default)]
    pub force: bool,
}

/// Response from bundle deployment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployBundleResponse {
    /// Policy ID that was deployed
    pub policy_id: String,
    /// Version that was deployed
    pub version: String,
    /// When the bundle was deployed
    pub deployed_at: String,
    /// SHA-256 hash of the bundle (hex-encoded)
    pub bundle_hash: String,
}

/// Entity data for CRUD operations (future use)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityData {
    /// Entity type (e.g., "user", "document")
    pub entity_type: String,
    /// Unique identifier of the entity within its type
    pub entity_id: String,
    /// String-valued attributes used in ABAC conditions
    #[serde(default)]
    pub string_attrs: HashMap<String, String>,
    /// Numeric attributes used in ABAC conditions
    #[serde(default)]
    pub numeric_attrs: HashMap<String, i64>,
    /// Relationships to other entities (ReBAC edges)
    #[serde(default)]
    pub relationships: Vec<Relationship>,
    /// Boolean attributes (feature flags, status markers)
    #[serde(default)]
    pub flags: HashMap<String, bool>,
}

/// Relationship between entities
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relationship {
    /// Kind of relationship (e.g., "member_of", "owner_of")
    pub relation_type: String,
    /// ID of the entity this relationship points to
    pub target_id: String,
}
