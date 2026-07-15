pub use reaper_core;

#[cfg(feature = "batch")]
pub mod batch;
pub mod cache_config;
pub mod clock;
pub mod compiled_evaluator;
pub mod data;
pub mod decision_cache;
pub mod decision_matrix;
mod engine;
mod evaluators;
pub mod fast_parse;
pub mod gherkin;
pub mod optimizer;
pub mod partial_evaluation;
pub mod policy_compilation;
pub mod reap;
pub mod regex_cache;

pub use engine::PolicyVersion as EngineVersion;
pub use engine::{
    AllPoliciesEvaluationResult, DenyInfo, EnhancedPolicy, PackageEvaluationResult, PackageInfo,
    PolicyAction, PolicyDecision, PolicyEngine, PolicyEngineStats, PolicyLanguage, PolicyRequest,
    PolicyRule, PolicySource, PolicySourceMetadata, PruningIndexStats, SetEvalOutcome,
    SimpleAction, SimpleRule, StagedPackage,
};

#[cfg(feature = "cedar")]
pub use evaluators::CedarPolicyEvaluator;
pub use evaluators::{EvaluatorMetadata, PolicyEvaluator, SimplePolicyEvaluator};

// Re-export reaper_dsl module for examples
pub use evaluators::reaper_dsl;

// Re-export reap parser and bundle format
pub use reap::{
    stable_policy_id, BundleFormat, PolicyBundle, PolicyPackage, PrecompilationHints, ReaperPolicy,
};

// Re-export optimizer types (Phase 5A: Decision Trees)
pub use optimizer::{DecisionTree, DecisionTreeBuilder, TreeStats};

// Re-export decision matrix (Phase 2: Decision Matrix Precomputation)
pub use decision_matrix::{DecisionKey, DecisionMatrix, DecisionMatrixStats, PrecomputedDecision};

// Re-export partial evaluation (Phase 3: Partial Evaluation)
pub use partial_evaluation::{Condition, OptimizationStats, PartialEvaluator};

// Re-export policy compilation (Phase 4: Policy Compilation)
pub use policy_compilation::{
    CodeGenerator, CompilationStats, CompiledPolicy, OptimizationLevel, PolicyCompiler,
};

// Re-export compiled evaluator (Executable optimized evaluation)
pub use compiled_evaluator::CompiledPolicyEvaluator;

pub use data::{
    AttributeValue, Attributes, DataBundle, DataBundleMetadata, DataFormat, DataLoader, DataStore,
    DataStoreConfig, Entity, EntityId, EntityType, IndexStrategy, InternedString, QueryBuilder,
    StreamingLoader, StreamingStats, StringInterner, StringTable,
};

// Re-export entity builder for convenience
pub use data::entity::EntityBuilder;

// Re-export core types for convenience
pub use reaper_core::{Policy, PolicyId, PolicyVersion, ReaperError, Result};

// Re-export fast parsing functions (SIMD-accelerated JSON parsing)
pub use fast_parse::{parse_batch_requests, parse_evaluate_request, parse_policy_request};

// Re-export decision cache for caching policy decisions
pub use decision_cache::{scope_hash, DecisionCache, DecisionCacheStats};

// Re-export batch evaluation for parallel request processing
#[cfg(feature = "batch")]
pub use batch::{BatchEvaluator, BatchResult, BatchStats};

// Decision logging (OPA-style structured decision logs). The entry/config
// types are always available; the async buffer, file export, and privacy
// crypto are host-runtime concerns behind features (off in wasm builds).
#[cfg(feature = "audit-buffer")]
pub mod decision_buffer;
#[cfg(feature = "audit-buffer")]
pub mod decision_export;
pub mod decision_log;
#[cfg(feature = "decision-privacy")]
pub mod decision_privacy;

#[cfg(feature = "audit-buffer")]
pub use decision_buffer::{
    create_shared_buffer, create_shared_buffer_with_stream, decision_stream_channel,
    DecisionBuffer, DecisionBufferStats, DecisionFilter, DecisionStreamReceiver,
    DecisionStreamSender, SharedDecisionBuffer,
};
#[cfg(feature = "audit-buffer")]
pub use decision_export::ExportFormat;
pub use decision_log::{DecisionLogConfig, DecisionLogEntry, PrivacyProfile};
#[cfg(feature = "decision-privacy")]
pub use decision_privacy::{
    decrypt_input_data, generate_encryption_key_hex, pseudonymize, pseudonymize_domain,
    DataProtection,
};
pub use reap::{CheckResult, Violation};

// Re-export cache configuration for environment-based cache setup
pub use cache_config::{CacheConfig, CacheConfigBuilder};
