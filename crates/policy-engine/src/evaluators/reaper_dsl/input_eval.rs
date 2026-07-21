//! Compiled evaluation of `input`-document comparisons (R4-01 Phase B.1).
//!
//! Semantics are a leaf-for-leaf mirror of the interpreter's
//! (`reap/ast_evaluator/comparison.rs` + `entity_access.rs`
//! `navigate_eval_path` + `builtin_functions/json.rs` `json_to_eval_value`),
//! evaluated directly over the raw `serde_json::Value` — no document-wide
//! conversion, no per-access path parsing. The truth table (pinned by the
//! B.1 differential):
//!
//! 1. Missing document / missing path / non-object mid-path ⇒ Null.
//! 2. `== null` ⇒ is-Null; `!= null` ⇒ !is-Null. (Ordered ops against a
//!    null literal are rejected at COMPILE time — the interpreter errors on
//!    them, and a compiled leaf must never turn an error into a decision.)
//! 3. Any other comparison where the resolved value is Null ⇒ false
//!    (absence never satisfies an inequality guard — fail closed).
//! 4. `==`/`!=` use the interpreter's `values_equal`: same-type equality
//!    only (an Int never equals a Float), float equality within EPSILON,
//!    and EXISTENTIAL ARRAY FLATTENING — `array == scalar` is true iff any
//!    element equals the scalar, recursively.
//! 5. Ordered ops are numeric-only (Int/Float, compared as f64); every
//!    non-numeric operand ⇒ false, never an error.
//! 6. JSON numbers map exactly like `json_to_eval_value`: i64-representable
//!    ⇒ integer, else f64 ⇒ float.

use super::types::{InputLiteral, InputPath, NumericOp};
use crate::data::{AttributeValue, StringInterner};
use serde_json::Value;

/// Materialize one input-array element into the compiled variable domain
/// (R4-01 B.2). Mirrors the loader's `json_value_to_attribute` shape mapping
/// (i64-first numbers, order-preserving List, Null for the unrepresentable),
/// but interns every string — keys and values — via `intern_transient`, so
/// document strings are reclaimed when the evaluation ends instead of
/// pinning request data in the shared interner. Filter attribute lookups
/// still match: policy-side attribute names are pinned, and transient
/// interning of an already-pinned string resolves to the same id.
pub(super) fn json_to_attribute_transient(
    value: &Value,
    interner: &StringInterner,
) -> AttributeValue {
    match value {
        Value::Null => AttributeValue::Null,
        Value::Bool(b) => AttributeValue::Bool(*b),
        Value::Number(n) => match n.as_i64() {
            Some(i) => AttributeValue::Int(i),
            None => match n.as_f64() {
                Some(f) => AttributeValue::Float(f),
                None => AttributeValue::Null,
            },
        },
        Value::String(s) => AttributeValue::String(super::intern_transient(interner, s)),
        Value::Array(items) => AttributeValue::List(
            items
                .iter()
                .map(|v| json_to_attribute_transient(v, interner))
                .collect(),
        ),
        Value::Object(map) => AttributeValue::Object(
            map.iter()
                .map(|(k, v)| {
                    (
                        super::intern_transient(interner, k),
                        json_to_attribute_transient(v, interner),
                    )
                })
                .collect(),
        ),
    }
}

/// Evaluate `input.<path> <op> <literal>` against the request document.
pub(super) fn eval_input_compare(
    path: &InputPath,
    op: &NumericOp,
    target: &InputLiteral,
    doc: Option<&Value>,
) -> bool {
    // Rule 1: no document ⇒ the whole access is Null.
    let resolved: Option<&Value> = doc.and_then(|d| path.resolve(d));

    // Rule 2: explicit presence checks against the null literal. A JSON
    // `null` node and a missing path are both "Null" (the interpreter's
    // conversion maps JSON null to EvalValue::Null).
    if matches!(target, InputLiteral::Null) {
        let is_null = matches!(resolved, None | Some(Value::Null));
        return match op {
            NumericOp::Equal => is_null,
            NumericOp::NotEqual => !is_null,
            // Unreachable: the compiler rejects ordered-vs-null (the
            // interpreter errors there; falling back keeps that contract).
            _ => false,
        };
    }

    // Rule 3: Null operand fails every non-presence comparison.
    let value = match resolved {
        None | Some(Value::Null) => return false,
        Some(v) => v,
    };

    match op {
        NumericOp::Equal => json_equals(value, target),
        NumericOp::NotEqual => !json_equals(value, target),
        NumericOp::Greater => json_ordered(value, target, |a, b| a > b),
        NumericOp::GreaterEqual => json_ordered(value, target, |a, b| a >= b),
        NumericOp::Less => json_ordered(value, target, |a, b| a < b),
        NumericOp::LessEqual => json_ordered(value, target, |a, b| a <= b),
    }
}

/// `values_equal` mirrored over raw JSON vs a scalar literal, including the
/// existential array rule (recursive: arrays of arrays flatten the same way
/// the interpreter's recursion does).
fn json_equals(value: &Value, target: &InputLiteral) -> bool {
    match value {
        Value::Array(items) => items.iter().any(|item| json_equals(item, target)),
        Value::String(s) => matches!(target, InputLiteral::Str(t) if s == t),
        Value::Bool(b) => matches!(target, InputLiteral::Bool(t) if b == t),
        Value::Number(n) => match (n.as_i64(), target) {
            // json_to_eval_value: i64-representable ⇒ Integer — equal only
            // to an Int literal (values_equal has no Int/Float cross-equality).
            (Some(i), InputLiteral::Int(t)) => i == *t,
            (Some(_), _) => false,
            // Not i64-representable ⇒ Float.
            (None, InputLiteral::Float(t)) => match n.as_f64() {
                Some(f) => (f - t).abs() < f64::EPSILON,
                None => false,
            },
            (None, _) => false,
        },
        // Objects and null never equal a scalar literal (null was already
        // handled); type mismatch ⇒ false.
        _ => false,
    }
}

/// `compare_numeric` mirrored: Int/Float only (as f64), anything else false.
/// No array flattening for ordered comparisons — the interpreter has none.
fn json_ordered(value: &Value, target: &InputLiteral, cmp: impl Fn(f64, f64) -> bool) -> bool {
    let a = match value {
        Value::Number(n) => match n.as_i64() {
            Some(i) => i as f64,
            None => match n.as_f64() {
                Some(f) => f,
                None => return false,
            },
        },
        _ => return false,
    };
    let b = match target {
        InputLiteral::Int(i) => *i as f64,
        InputLiteral::Float(f) => *f,
        _ => return false,
    };
    cmp(a, b)
}
