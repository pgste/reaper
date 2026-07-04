//! JSON functions for policy evaluation.
//!
//! This module provides JSON-related operations:
//! - parse() - Parse JSON string to EvalValue
//! - stringify() - Convert EvalValue to JSON string
//! - is_valid() - Check if string is valid JSON
//!
//! Uses sonic_rs for native SIMD-accelerated parsing,
//! with serde_json fallback for WASM compatibility.

use super::super::types::EvalValue;
use reaper_core::ReaperError;
use std::collections::HashMap;

/// json::parse(string) - Parse JSON string to EvalValue
#[inline]
pub fn parse(value: &EvalValue) -> Result<EvalValue, ReaperError> {
    let json_str = match value {
        EvalValue::String(s) => s,
        _ => {
            return Err(ReaperError::InvalidPolicy {
                reason: "json::parse() requires a string argument".to_string(),
            })
        }
    };

    #[cfg(not(target_arch = "wasm32"))]
    {
        match sonic_rs::from_str::<sonic_rs::Value>(json_str) {
            Ok(json_value) => sonic_value_to_eval_value(&json_value),
            Err(e) => Err(ReaperError::InvalidPolicy {
                reason: format!("json::parse() failed: {}", e),
            }),
        }
    }

    #[cfg(target_arch = "wasm32")]
    {
        match serde_json::from_str::<serde_json::Value>(json_str) {
            Ok(json_value) => serde_value_to_eval_value(&json_value),
            Err(e) => Err(ReaperError::InvalidPolicy {
                reason: format!("json::parse() failed: {}", e),
            }),
        }
    }
}

/// json::stringify(value) - Convert EvalValue to JSON string
#[inline]
pub fn stringify(value: &EvalValue) -> Result<EvalValue, ReaperError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let json_value = eval_value_to_sonic(value)?;
        match sonic_rs::to_string(&json_value) {
            Ok(json_str) => Ok(EvalValue::String(json_str)),
            Err(e) => Err(ReaperError::InvalidPolicy {
                reason: format!("json::stringify() failed: {}", e),
            }),
        }
    }

    #[cfg(target_arch = "wasm32")]
    {
        let json_value = eval_value_to_serde(&value)?;
        match serde_json::to_string(&json_value) {
            Ok(json_str) => Ok(EvalValue::String(json_str)),
            Err(e) => Err(ReaperError::InvalidPolicy {
                reason: format!("json::stringify() failed: {}", e),
            }),
        }
    }
}

/// json::is_valid(string) - Check if string is valid JSON
#[inline]
pub fn is_valid(value: &EvalValue) -> Result<EvalValue, ReaperError> {
    let json_str = match value {
        EvalValue::String(s) => s,
        _ => {
            return Err(ReaperError::InvalidPolicy {
                reason: "json::is_valid() requires a string argument".to_string(),
            })
        }
    };

    #[cfg(not(target_arch = "wasm32"))]
    let is_valid = sonic_rs::from_str::<sonic_rs::Value>(json_str).is_ok();

    #[cfg(target_arch = "wasm32")]
    let is_valid = serde_json::from_str::<serde_json::Value>(json_str).is_ok();

    Ok(EvalValue::Boolean(is_valid))
}

// ============================================================================
// Native (sonic_rs) conversion functions
// ============================================================================

/// Convert sonic_rs::Value to EvalValue (native)
#[cfg(not(target_arch = "wasm32"))]
fn sonic_value_to_eval_value(json: &sonic_rs::Value) -> Result<EvalValue, ReaperError> {
    use sonic_rs::{JsonContainerTrait, JsonValueTrait};

    if json.is_null() {
        Ok(EvalValue::Null)
    } else if let Some(b) = json.as_bool() {
        Ok(EvalValue::Boolean(b))
    } else if let Some(i) = json.as_i64() {
        Ok(EvalValue::Integer(i))
    } else if let Some(f) = json.as_f64() {
        Ok(EvalValue::Float(f))
    } else if let Some(s) = json.as_str() {
        Ok(EvalValue::String(s.to_string()))
    } else if let Some(arr) = json.as_array() {
        let eval_arr: Result<Vec<EvalValue>, ReaperError> =
            arr.iter().map(sonic_value_to_eval_value).collect();
        Ok(EvalValue::Array(eval_arr?))
    } else if let Some(obj) = json.as_object() {
        let mut eval_obj = HashMap::new();
        for (key, value) in obj {
            eval_obj.insert(key.to_string(), sonic_value_to_eval_value(value)?);
        }
        Ok(EvalValue::Object(eval_obj))
    } else {
        Err(ReaperError::InvalidPolicy {
            reason: "Unsupported JSON value type".to_string(),
        })
    }
}

/// Convert EvalValue to sonic_rs::Value (native)
#[cfg(not(target_arch = "wasm32"))]
fn eval_value_to_sonic(eval: &EvalValue) -> Result<sonic_rs::Value, ReaperError> {
    use sonic_rs::{json, Object};

    match eval {
        EvalValue::Null => Ok(json!(null)),
        EvalValue::Boolean(b) => Ok(json!(*b)),
        EvalValue::Integer(i) => Ok(json!(*i)),
        EvalValue::Float(f) => {
            if f.is_nan() || f.is_infinite() {
                Err(ReaperError::InvalidPolicy {
                    reason: format!("Cannot convert float {} to JSON (NaN or Infinity)", f),
                })
            } else {
                Ok(json!(*f))
            }
        }
        EvalValue::String(s) => Ok(json!(s)),
        EvalValue::Array(arr) => {
            let json_arr: Result<Vec<sonic_rs::Value>, ReaperError> =
                arr.iter().map(eval_value_to_sonic).collect();
            Ok(json!(json_arr?))
        }
        EvalValue::Set(set) => {
            let json_arr: Result<Vec<sonic_rs::Value>, ReaperError> =
                set.iter().map(eval_value_to_sonic).collect();
            Ok(json!(json_arr?))
        }
        EvalValue::Object(obj) => {
            let mut json_obj = Object::new();
            for (key, value) in obj {
                json_obj.insert(key, eval_value_to_sonic(value)?);
            }
            Ok(json!(json_obj))
        }
    }
}

// ============================================================================
// WASM (serde_json) conversion functions
// ============================================================================

/// Convert serde_json::Value to EvalValue (WASM)
#[cfg(target_arch = "wasm32")]
fn serde_value_to_eval_value(json: &serde_json::Value) -> Result<EvalValue, ReaperError> {
    match json {
        serde_json::Value::Null => Ok(EvalValue::Null),
        serde_json::Value::Bool(b) => Ok(EvalValue::Boolean(*b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(EvalValue::Integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(EvalValue::Float(f))
            } else {
                Err(ReaperError::InvalidPolicy {
                    reason: "Unsupported JSON number type".to_string(),
                })
            }
        }
        serde_json::Value::String(s) => Ok(EvalValue::String(s.clone())),
        serde_json::Value::Array(arr) => {
            let eval_arr: Result<Vec<EvalValue>, ReaperError> =
                arr.iter().map(|v| serde_value_to_eval_value(v)).collect();
            Ok(EvalValue::Array(eval_arr?))
        }
        serde_json::Value::Object(obj) => {
            let mut eval_obj = HashMap::new();
            for (key, value) in obj {
                eval_obj.insert(key.clone(), serde_value_to_eval_value(value)?);
            }
            Ok(EvalValue::Object(eval_obj))
        }
    }
}

/// Convert EvalValue to serde_json::Value (WASM)
#[cfg(target_arch = "wasm32")]
fn eval_value_to_serde(eval: &EvalValue) -> Result<serde_json::Value, ReaperError> {
    use serde_json::{json, Map};

    match eval {
        EvalValue::Null => Ok(json!(null)),
        EvalValue::Boolean(b) => Ok(json!(*b)),
        EvalValue::Integer(i) => Ok(json!(*i)),
        EvalValue::Float(f) => {
            if f.is_nan() || f.is_infinite() {
                Err(ReaperError::InvalidPolicy {
                    reason: format!("Cannot convert float {} to JSON (NaN or Infinity)", f),
                })
            } else {
                Ok(json!(*f))
            }
        }
        EvalValue::String(s) => Ok(json!(s)),
        EvalValue::Array(arr) => {
            let json_arr: Result<Vec<serde_json::Value>, ReaperError> =
                arr.iter().map(|v| eval_value_to_serde(v)).collect();
            Ok(json!(json_arr?))
        }
        EvalValue::Set(set) => {
            let json_arr: Result<Vec<serde_json::Value>, ReaperError> =
                set.iter().map(|v| eval_value_to_serde(v)).collect();
            Ok(json!(json_arr?))
        }
        EvalValue::Object(obj) => {
            let mut json_obj = Map::new();
            for (key, value) in obj {
                json_obj.insert(key.clone(), eval_value_to_serde(value)?);
            }
            Ok(json!(json_obj))
        }
    }
}

/// Convert a `serde_json::Value` tree into an `EvalValue` tree (all targets).
///
/// Used to bind the per-request `input` document once, before rule
/// evaluation — rules then navigate the converted tree with no re-parsing.
pub(in crate::reap::ast_evaluator) fn json_to_eval_value(
    json: &serde_json::Value,
) -> Result<EvalValue, ReaperError> {
    Ok(match json {
        serde_json::Value::Null => EvalValue::Null,
        serde_json::Value::Bool(b) => EvalValue::Boolean(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                EvalValue::Integer(i)
            } else if let Some(f) = n.as_f64() {
                EvalValue::Float(f)
            } else {
                return Err(ReaperError::InvalidPolicy {
                    reason: format!("unrepresentable JSON number: {n}"),
                });
            }
        }
        serde_json::Value::String(s) => EvalValue::String(s.clone()),
        serde_json::Value::Array(arr) => EvalValue::Array(
            arr.iter()
                .map(json_to_eval_value)
                .collect::<Result<Vec<_>, _>>()?,
        ),
        serde_json::Value::Object(obj) => EvalValue::Object(
            obj.iter()
                .map(|(k, v)| Ok((k.clone(), json_to_eval_value(v)?)))
                .collect::<Result<HashMap<_, _>, ReaperError>>()?,
        ),
    })
}
