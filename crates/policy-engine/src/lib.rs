pub use reaper_core;

pub mod data;
mod engine;
mod evaluators;
pub mod gherkin;
pub mod optimizer;
pub mod reap;

pub use engine::{
    EnhancedPolicy, PolicyAction, PolicyDecision, PolicyEngine, PolicyEngineStats, PolicyLanguage,
    PolicyRequest, PolicyRule, SimpleAction, SimpleRule,
};

pub use evaluators::{
    CedarPolicyEvaluator, EvaluatorMetadata, PolicyEvaluator, SimplePolicyEvaluator,
};

// Re-export reaper_dsl module for examples
pub use evaluators::reaper_dsl;

// Re-export reap parser and bundle format
pub use reap::{BundleFormat, PolicyBundle, ReaperPolicy};

// Re-export optimizer types (Phase 5A: Decision Trees)
pub use optimizer::{DecisionTree, DecisionTreeBuilder, TreeStats};

pub use data::{
    AttributeValue, Attributes, DataFormat, DataLoader, DataStore, Entity, EntityId, EntityType,
    IndexStrategy, InternedString, QueryBuilder, StringInterner,
};

// Re-export entity builder for convenience
pub use data::entity::EntityBuilder;

// Re-export core types for convenience
pub use reaper_core::{Policy, PolicyId, PolicyVersion, ReaperError, Result};
