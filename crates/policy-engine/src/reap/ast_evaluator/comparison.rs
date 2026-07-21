//! Comparison evaluation for AST evaluator.
//!
//! This module handles comparison operations:
//! - Equality/inequality comparisons
//! - Numeric comparisons (greater, less, etc.)
//! - Membership tests (in operator)
//! - Existential quantification for array comparisons

use super::types::{EvalContext, EvalValue};
use super::ReapAstEvaluator;
use crate::reap::ast::{ComparisonLeft, ComparisonRight, Operator, Value};
use reaper_core::ReaperError;

impl ReapAstEvaluator {
    /// Evaluate a comparison
    pub(super) fn evaluate_comparison(
        &self,
        left: &ComparisonLeft,
        op: Operator,
        right: &ComparisonRight,
        context: &EvalContext,
    ) -> Result<bool, ReaperError> {
        // Get left value
        let left_value = match left {
            ComparisonLeft::EntityAttr(attr) => self.get_entity_attribute(attr, context)?,
            ComparisonLeft::VarAttr(var_attr) => self.get_var_attribute(var_attr, context)?,
            ComparisonLeft::Expr(expr) => self.evaluate_expr(expr, context)?,
        };

        // Get right value
        let right_value = match right {
            ComparisonRight::Value(val) => self.value_to_eval_value(val),
            ComparisonRight::EntityAttr(attr) => self.get_entity_attribute(attr, context)?,
            ComparisonRight::VarAttr(var_attr) => self.get_var_attribute(var_attr, context)?,
            ComparisonRight::Variable(var_name) => {
                context.variables.get(var_name).cloned().ok_or_else(|| {
                    ReaperError::InvalidPolicy {
                        reason: format!("Undefined variable: {}", var_name),
                    }
                })?
            }
            ComparisonRight::Expr(expr) => self.evaluate_expr(expr, context)?,
        };

        // NULL SEMANTICS (specified; enforced by the differential oracle):
        // a missing attribute/path evaluates to Null, and Null satisfies NO
        // comparison except an EXPLICIT presence check against a null literal
        // (`x == null` / `x != null`). In particular `missing != "admin"` is
        // FALSE — absence must never satisfy an inequality guard (that would
        // be default-open: entities lacking the attribute would pass every
        // `!=` filter). Matches OPA's undefined-propagation behavior.
        let right_is_null_literal = matches!(right, ComparisonRight::Value(Value::Null));
        if right_is_null_literal {
            let is_null = matches!(left_value, EvalValue::Null);
            return match op {
                Operator::Equal => Ok(is_null),
                Operator::NotEqual => Ok(!is_null),
                _ => Err(ReaperError::InvalidPolicy {
                    reason: "null comparisons support only == and !=".to_string(),
                }),
            };
        }
        if matches!(left_value, EvalValue::Null) || matches!(right_value, EvalValue::Null) {
            // Any other comparison involving Null fails the rule (never errors).
            return Ok(false);
        }

        // Perform comparison based on operator
        match op {
            Operator::Equal => Ok(Self::values_equal(&left_value, &right_value)),
            Operator::NotEqual => Ok(!Self::values_equal(&left_value, &right_value)),
            Operator::GreaterThan => self.compare_numeric(&left_value, &right_value, |a, b| a > b),
            Operator::LessThan => self.compare_numeric(&left_value, &right_value, |a, b| a < b),
            Operator::GreaterEqual => {
                self.compare_numeric(&left_value, &right_value, |a, b| a >= b)
            }
            Operator::LessEqual => self.compare_numeric(&left_value, &right_value, |a, b| a <= b),
            // For "in" operator: "value in collection" is parsed as left=collection, right=value
            // So we need to check if right_value is in left_value (collection)
            Operator::In => self.check_membership(&left_value, &right_value),
        }
    }

    /// Check if a value is in a collection
    pub(super) fn check_membership(
        &self,
        collection: &EvalValue,
        value: &EvalValue,
    ) -> Result<bool, ReaperError> {
        match collection {
            EvalValue::Array(arr) | EvalValue::Set(arr) => {
                Ok(arr.iter().any(|item| Self::values_equal(item, value)))
            }
            EvalValue::Object(map) => {
                // For objects, check if key exists
                if let EvalValue::String(key) = value {
                    Ok(map.contains_key(key))
                } else {
                    Ok(false)
                }
            }
            // Non-collection membership target: fail-closed non-match, not
            // an error (total evaluation; matches the compiled path — R4-01
            // B.2b, caught by the input-comprehension differential).
            _ => Ok(false),
        }
    }

    /// Compare two values for equality
    /// Supports existential quantification: when comparing an array to a scalar,
    /// returns true if ANY element in the array equals the scalar.
    /// This enables wildcard iteration syntax like `user.desk_ids[_] == resource.desk_id`
    pub(super) fn values_equal(a: &EvalValue, b: &EvalValue) -> bool {
        match (a, b) {
            (EvalValue::String(a), EvalValue::String(b)) => a == b,
            (EvalValue::Integer(a), EvalValue::Integer(b)) => a == b,
            (EvalValue::Float(a), EvalValue::Float(b)) => (a - b).abs() < f64::EPSILON,
            (EvalValue::Boolean(a), EvalValue::Boolean(b)) => a == b,
            (EvalValue::Null, EvalValue::Null) => true,
            // Existential quantification: array[_] == scalar
            // Returns true if ANY element in the array equals the scalar
            (EvalValue::Array(arr), scalar) | (EvalValue::Set(arr), scalar) => {
                arr.iter().any(|item| Self::values_equal(item, scalar))
            }
            // Existential quantification: scalar == array[_]
            // Returns true if ANY element in the array equals the scalar
            (scalar, EvalValue::Array(arr)) | (scalar, EvalValue::Set(arr)) => {
                arr.iter().any(|item| Self::values_equal(scalar, item))
            }
            _ => false,
        }
    }

    /// Compare numeric values
    pub(super) fn compare_numeric<F>(
        &self,
        a: &EvalValue,
        b: &EvalValue,
        cmp: F,
    ) -> Result<bool, ReaperError>
    where
        F: Fn(f64, f64) -> bool,
    {
        // TOTAL-COMPARISON CONTRACT: ordering over a non-numeric value is
        // FALSE, never an error. A live policy must degrade toward deny,
        // not fail the request — and the compiled evaluator and the
        // differential oracle already answer false here.
        let a_num = match a {
            EvalValue::Integer(i) => *i as f64,
            EvalValue::Float(f) => *f,
            _ => return Ok(false),
        };

        let b_num = match b {
            EvalValue::Integer(i) => *i as f64,
            EvalValue::Float(f) => *f,
            _ => return Ok(false),
        };

        Ok(cmp(a_num, b_num))
    }
}
