//! Core types for the Reaper DSL evaluator.

use crate::data::Entity;
use crate::PolicyAction;
use serde::{Deserialize, Serialize};

use super::compiled_condition::CompiledCondition;
use super::condition::Condition;

/// Borrowed entity bindings for one evaluation pass.
///
/// Groups the request's resolved entities so evaluation helpers take a single
/// parameter instead of a growing list of entity refs. `Copy` — passing this
/// by value is two/three pointers, identical codegen to the previous
/// `(user, resource)` pair.
///
/// `actor` (F1 agentic authz) is the optional non-human actor from the
/// request's `actor` field. `None` means the request carried no actor;
/// actor-referencing conditions then read null and must not match.
#[derive(Clone, Copy)]
pub struct EntityBindings<'a> {
    /// Principal entity (`user.*`), resolved from `context["principal"]`.
    pub user: &'a Entity,
    /// Optional actor entity (`actor.*`), resolved from `request.actor`.
    pub actor: Option<&'a Entity>,
    /// Resource entity (`resource.*`), loaded or synthesized from the id.
    pub resource: &'a Entity,
    /// Per-key context provenance from the request (F1 taint). `None` =
    /// taint mode off (every key platform-trusted). Drives `taint::level`
    /// and `taint::trusted` on the compiled path.
    pub provenance: Option<&'a std::collections::HashMap<String, crate::TrustLevel>>,
}

impl EntityBindings<'_> {
    /// Effective trust of one context key under the fail-untrusted rule —
    /// same semantics as [`crate::PolicyRequest::context_trust`]: taint mode
    /// off ⇒ platform; taint mode on ⇒ unlabeled keys floor to llm.
    pub fn context_trust(&self, key: &str) -> crate::TrustLevel {
        match self.provenance {
            None => crate::TrustLevel::Platform,
            Some(map) => map.get(key).copied().unwrap_or(crate::TrustLevel::Llm),
        }
    }
}

/// A single policy rule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    /// Rule name (for debugging/auditing)
    pub name: String,
    /// Condition to evaluate
    pub condition: Condition,
    /// Decision if condition is true
    pub decision: PolicyAction,
}

/// Compiled rule with pre-interned condition for fast evaluation
#[derive(Debug, Clone)]
pub struct CompiledRule {
    /// Rule name (for debugging/auditing)
    pub name: String,
    /// Pre-compiled condition with interned strings
    pub condition: CompiledCondition,
    /// Decision if condition is true
    pub decision: PolicyAction,
}

/// Entity type for condition evaluation
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntityType {
    User,
    Resource,
    Context,
    /// The optional non-human actor (F1 agentic authz). Appended after the
    /// original variants so any serialized conditions keep their encoding.
    Actor,
}

/// Index expression for bracket notation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IndexExpr {
    /// Numeric index: [0], [1], [42]
    Number(i64),
    /// String key: ["department"], ["role"]
    String(String),
    /// Wildcard for iteration: [_] - iterates over all elements (existential quantification)
    Wildcard,
}

/// Literal value for comparisons
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LiteralValue {
    String(String),
    Int(i64),
    Bool(bool),
}
