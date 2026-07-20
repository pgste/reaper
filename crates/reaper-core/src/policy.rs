//! Policy types and traits

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identifier for a policy.
pub type PolicyId = Uuid;
/// Monotonically increasing version number of a policy (bumped on each update).
pub type PolicyVersion = u64;

/// Core policy record shared between the platform and agents: identity,
/// version, and descriptive metadata (the evaluable content lives in the
/// engine's enhanced policy types).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    /// Unique identifier of the policy.
    pub id: PolicyId,
    /// Version number, incremented on every update (used for hot-swap ordering).
    pub version: PolicyVersion,
    /// Human-readable policy name (unique per deployment).
    pub name: String,
    /// Free-form description of what the policy enforces.
    pub description: String,
}

/// Marker trait for policy evaluation engines.
pub trait PolicyEngine {
    // Will be implemented in first vertical feature
}
