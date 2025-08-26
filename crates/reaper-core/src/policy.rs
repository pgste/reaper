//! Policy types and traits

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type PolicyId = Uuid;
pub type PolicyVersion = u64;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    pub id: PolicyId,
    pub version: PolicyVersion,
    pub name: String,
    pub description: String,
}

pub trait PolicyEngine {
    // Will be implemented in first vertical feature
}
