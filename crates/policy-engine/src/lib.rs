pub use reaper_core;

pub mod compiled_evaluator;
pub mod data;
pub mod decision_matrix;
mod engine;
mod evaluators;
pub mod gherkin;
pub mod indexed_engine;
pub mod optimized_engine;
pub mod optimizer;
pub mod partial_evaluation;
pub mod policy_compilation;
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

// Re-export indexed engine (Phase 1: Multi-Index Optimization)
pub use indexed_engine::{IndexStats, IndexedPolicyEngine};

// Re-export decision matrix (Phase 2: Decision Matrix Precomputation)
pub use decision_matrix::{DecisionKey, DecisionMatrix, DecisionMatrixStats, PrecomputedDecision};

// Re-export partial evaluation (Phase 3: Partial Evaluation)
pub use partial_evaluation::{Condition, OptimizationStats, PartialEvaluator};

// Re-export policy compilation (Phase 4: Policy Compilation)
pub use policy_compilation::{
    CodeGenerator, CompilationStats, CompiledPolicy, OptimizationLevel, PolicyCompiler,
};

// Re-export optimized engine (All Phases Integrated + Learning)
pub use optimized_engine::{OptimizationSummary, OptimizedEngineStats, OptimizedPolicyEngine};

// Re-export compiled evaluator (Executable optimized evaluation)
pub use compiled_evaluator::CompiledPolicyEvaluator;

pub use data::{
    AttributeValue, Attributes, DataFormat, DataLoader, DataStore, Entity, EntityId, EntityType,
    IndexStrategy, InternedString, QueryBuilder, StreamingLoader, StreamingStats, StringInterner,
};

// Re-export entity builder for convenience
pub use data::entity::EntityBuilder;

// Re-export core types for convenience
pub use reaper_core::{Policy, PolicyId, PolicyVersion, ReaperError, Result};
