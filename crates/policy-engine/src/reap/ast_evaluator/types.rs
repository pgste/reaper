//! Type definitions for the AST evaluator.
//!
//! Contains the core types used during AST evaluation:
//! - EvalContext: Holds variable bindings and entity references during evaluation
//! - EvalValue: Runtime value representation for policy expressions

use crate::data::EntityId;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// Evaluation context holding variable bindings
#[derive(Debug, Clone)]
pub(super) struct EvalContext {
    /// Variable name -> value mappings
    pub(super) variables: HashMap<String, EvalValue>,
    /// User entity from request
    pub(super) user_id: EntityId,
    /// Actor entity (F1 agentic authz): the non-human actor from the
    /// request's `actor` field. `None` when the request carries no actor —
    /// `actor.*` then reads null rather than erroring.
    pub(super) actor_id: Option<EntityId>,
    /// Resource entity from request
    pub(super) resource_id: EntityId,
    /// Request context (includes action and other attributes)
    pub(super) request_context: HashMap<String, String>,
    /// Structured request document (`input`): arbitrary nested JSON converted
    /// once per evaluation. None when the request carries no document.
    pub(super) input: Option<EvalValue>,
}

/// Runtime value during evaluation
#[derive(Debug, Clone)]
pub(crate) enum EvalValue {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Null,
    Array(Vec<EvalValue>),
    Object(HashMap<String, EvalValue>),
    Set(Vec<EvalValue>), // Using Vec for now, can optimize to HashSet later
}

impl PartialEq for EvalValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (EvalValue::String(a), EvalValue::String(b)) => a == b,
            (EvalValue::Integer(a), EvalValue::Integer(b)) => a == b,
            (EvalValue::Float(a), EvalValue::Float(b)) => a.to_bits() == b.to_bits(),
            (EvalValue::Boolean(a), EvalValue::Boolean(b)) => a == b,
            (EvalValue::Null, EvalValue::Null) => true,
            (EvalValue::Array(a), EvalValue::Array(b)) => a == b,
            (EvalValue::Object(a), EvalValue::Object(b)) => a == b,
            (EvalValue::Set(a), EvalValue::Set(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for EvalValue {}

impl Hash for EvalValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            EvalValue::String(s) => {
                0u8.hash(state);
                s.hash(state);
            }
            EvalValue::Integer(i) => {
                1u8.hash(state);
                i.hash(state);
            }
            EvalValue::Float(f) => {
                2u8.hash(state);
                f.to_bits().hash(state);
            }
            EvalValue::Boolean(b) => {
                3u8.hash(state);
                b.hash(state);
            }
            EvalValue::Null => {
                4u8.hash(state);
            }
            EvalValue::Array(arr) => {
                5u8.hash(state);
                arr.hash(state);
            }
            EvalValue::Object(obj) => {
                6u8.hash(state);
                // Hash objects by sorted keys for consistency
                let mut entries: Vec<_> = obj.iter().collect();
                entries.sort_by_key(|(k, _)| *k);
                entries.hash(state);
            }
            EvalValue::Set(set) => {
                7u8.hash(state);
                set.hash(state);
            }
        }
    }
}
