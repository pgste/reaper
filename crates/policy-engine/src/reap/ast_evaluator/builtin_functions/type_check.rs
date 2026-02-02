//! Type checking functions for policy evaluation.
//!
//! This module provides type inspection operations:
//! - is_string() - Check if value is a string
//! - is_number() - Check if value is numeric (integer or float)
//! - is_bool() - Check if value is a boolean
//! - is_array() - Check if value is an array
//! - is_set() - Check if value is a set
//! - is_object() - Check if value is an object
//! - is_null() - Check if value is null

use super::super::types::EvalValue;

/// is_string(value) - Check if value is a string
#[inline]
pub fn is_string(value: &EvalValue) -> EvalValue {
    EvalValue::Boolean(matches!(value, EvalValue::String(_)))
}

/// is_number(value) - Check if value is numeric (integer or float)
#[inline]
pub fn is_number(value: &EvalValue) -> EvalValue {
    EvalValue::Boolean(matches!(value, EvalValue::Integer(_) | EvalValue::Float(_)))
}

/// is_bool(value) - Check if value is a boolean
#[inline]
pub fn is_bool(value: &EvalValue) -> EvalValue {
    EvalValue::Boolean(matches!(value, EvalValue::Boolean(_)))
}

/// is_array(value) - Check if value is an array
#[inline]
pub fn is_array(value: &EvalValue) -> EvalValue {
    EvalValue::Boolean(matches!(value, EvalValue::Array(_)))
}

/// is_set(value) - Check if value is a set
#[inline]
pub fn is_set(value: &EvalValue) -> EvalValue {
    EvalValue::Boolean(matches!(value, EvalValue::Set(_)))
}

/// is_object(value) - Check if value is an object
#[inline]
pub fn is_object(value: &EvalValue) -> EvalValue {
    EvalValue::Boolean(matches!(value, EvalValue::Object(_)))
}

/// is_null(value) - Check if value is null
#[inline]
pub fn is_null(value: &EvalValue) -> EvalValue {
    EvalValue::Boolean(matches!(value, EvalValue::Null))
}

/// concat(strings) - Concatenate multiple strings
#[inline]
pub fn concat(values: &[EvalValue]) -> Result<EvalValue, reaper_core::ReaperError> {
    let mut result = String::new();
    for value in values {
        match value {
            EvalValue::String(s) => result.push_str(s),
            _ => {
                return Err(reaper_core::ReaperError::InvalidPolicy {
                    reason: "concat() requires string arguments".to_string(),
                })
            }
        }
    }
    Ok(EvalValue::String(result))
}
