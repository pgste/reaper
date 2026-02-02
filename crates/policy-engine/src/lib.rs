pub use reaper_core;

pub mod arena;
pub mod batch;
pub mod cache_config;
pub mod compiled_evaluator;
pub mod data;
pub mod decision_cache;
pub mod decision_matrix;
mod engine;
mod evaluators;
pub mod fast_parse;
pub mod gherkin;
pub mod indexed_engine;
pub mod optimized_engine;
pub mod optimizer;
pub mod partial_evaluation;
pub mod policy_compilation;
pub mod reap;
pub mod regex_cache;

pub use engine::{
    AllPoliciesEvaluationResult, DenyInfo, EnhancedPolicy, PackageEvaluationResult, PackageInfo,
    PolicyAction, PolicyDecision, PolicyEngine, PolicyEngineStats, PolicyLanguage, PolicyRequest,
    PolicyRule, PolicySource, PolicySourceMetadata, SimpleAction, SimpleRule, StagedPackage,
};
pub use engine::PolicyVersion as EngineVersion;

pub use evaluators::{
    CedarPolicyEvaluator, EvaluatorMetadata, PolicyEvaluator, SimplePolicyEvaluator,
};

// Re-export reaper_dsl module for examples
pub use evaluators::reaper_dsl;

// Re-export reap parser and bundle format
pub use reap::{BundleFormat, PolicyBundle, PolicyPackage, PrecompilationHints, ReaperPolicy};

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
    AttributeValue, Attributes, DataBundle, DataBundleMetadata, DataFormat, DataLoader, DataStore,
    Entity, EntityId, EntityType, IndexStrategy, InternedString, QueryBuilder, StreamingLoader,
    StreamingStats, StringInterner, StringTable,
};

// Re-export entity builder for convenience
pub use data::entity::EntityBuilder;

// Re-export core types for convenience
pub use reaper_core::{Policy, PolicyId, PolicyVersion, ReaperError, Result};

// Re-export fast parsing functions (SIMD-accelerated JSON parsing)
pub use fast_parse::{parse_batch_requests, parse_evaluate_request, parse_policy_request};

// Re-export arena allocator for zero-allocation evaluation
pub use arena::{
    arena_stats, prewarm_arena, reset_arena, with_arena, with_arena_reset, ArenaStats, ArenaString,
    ArenaValue, ArenaVec, EvaluationContext,
};

// Re-export decision cache for caching policy decisions
pub use decision_cache::{CachedEvaluator, DecisionCache, DecisionCacheStats};

// Re-export batch evaluation for parallel request processing
pub use batch::{BatchEvaluator, BatchResult, BatchStats};

// Decision logging (OPA-style structured decision logs)
pub mod decision_buffer;
pub mod decision_log;

pub use decision_buffer::{
    create_shared_buffer, DecisionBuffer, DecisionBufferStats, DecisionFilter, SharedDecisionBuffer,
};
pub use decision_log::{DecisionLogConfig, DecisionLogEntry};

// Re-export cache configuration for environment-based cache setup
pub use cache_config::{CacheConfig, CacheConfigBuilder};
