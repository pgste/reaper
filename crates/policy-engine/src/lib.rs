pub use reaper_core;

mod engine;
mod evaluators;
pub mod data;
pub mod reap;

pub use engine::{
    EnhancedPolicy, PolicyAction, PolicyDecision, PolicyEngine, PolicyEngineStats,
    PolicyLanguage, PolicyRequest, PolicyRule, SimpleAction, SimpleRule,
};

pub use evaluators::{
    CedarPolicyEvaluator, PolicyEvaluator, SimplePolicyEvaluator,
    EvaluatorMetadata,
};

// Re-export reaper_dsl module for examples
pub use evaluators::reaper_dsl;

// Re-export reap parser and bundle format
pub use reap::{ReaperPolicy, PolicyBundle, BundleFormat};

pub use data::{
    DataStore, DataLoader, DataFormat,
    Entity, EntityId, EntityType, AttributeValue, Attributes,
    InternedString, StringInterner,
    IndexStrategy, QueryBuilder,
};

// Re-export entity builder for convenience
pub use data::entity::EntityBuilder;

// Re-export core types for convenience
pub use reaper_core::{Policy, PolicyId, PolicyVersion, ReaperError, Result};
