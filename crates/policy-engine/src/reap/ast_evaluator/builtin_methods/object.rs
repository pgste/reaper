//! Object methods for AST evaluation.
//!
//! This module provides object operations:
//! - keys() - Get all keys from an object
//! - values() - Get all values from an object
//! - has_key() - Check if object contains a key

use super::super::types::EvalValue;
use reaper_core::ReaperError;

/// keys() - Returns an array of all keys in an object
/// Preserves insertion order (HashMap maintains order in Rust)
#[inline]
pub fn method_keys(value: &EvalValue) -> Result<EvalValue, ReaperError> {
    match value {
        EvalValue::Object(obj) => {
            let keys: Vec<EvalValue> = obj.keys().map(|k| EvalValue::String(k.clone())).collect();
            Ok(EvalValue::Array(keys))
        }
        _ => Err(ReaperError::InvalidPolicy {
            reason: "keys() requires an object".to_string(),
        }),
    }
}

/// values() - Returns an array of all values in an object
/// Preserves insertion order
#[inline]
pub fn method_values(value: &EvalValue) -> Result<EvalValue, ReaperError> {
    match value {
        EvalValue::Object(obj) => {
            let values: Vec<EvalValue> = obj.values().cloned().collect();
            Ok(EvalValue::Array(values))
        }
        _ => Err(ReaperError::InvalidPolicy {
            reason: "values() requires an object".to_string(),
        }),
    }
}

/// has_key(key) - Checks if an object contains a specific key
/// O(1) average case lookup using HashMap
#[inline]
pub fn method_has_key(value: &EvalValue, key: &EvalValue) -> Result<EvalValue, ReaperError> {
    match value {
        EvalValue::Object(obj) => {
            let key_str = match key {
                EvalValue::String(s) => s,
                _ => {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "has_key() requires a string key".to_string(),
                    })
                }
            };
            Ok(EvalValue::Boolean(obj.contains_key(key_str)))
        }
        _ => Err(ReaperError::InvalidPolicy {
            reason: "has_key() requires an object".to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_test_object() -> EvalValue {
        let mut obj = HashMap::new();
        obj.insert("name".to_string(), EvalValue::String("Alice".to_string()));
        obj.insert("age".to_string(), EvalValue::Integer(30));
        obj.insert("active".to_string(), EvalValue::Boolean(true));
        EvalValue::Object(obj)
    }

    #[test]
    fn test_keys() {
        let obj = make_test_object();
        let result = method_keys(&obj).unwrap();

        if let EvalValue::Array(keys) = result {
            assert_eq!(keys.len(), 3);
            // Check that all keys are present (order may vary)
            let key_strs: Vec<_> = keys
                .iter()
                .filter_map(|v| {
                    if let EvalValue::String(s) = v {
                        Some(s.as_str())
                    } else {
                        None
                    }
                })
                .collect();
            assert!(key_strs.contains(&"name"));
            assert!(key_strs.contains(&"age"));
            assert!(key_strs.contains(&"active"));
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_values() {
        let obj = make_test_object();
        let result = method_values(&obj).unwrap();

        if let EvalValue::Array(values) = result {
            assert_eq!(values.len(), 3);
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_has_key_exists() {
        let obj = make_test_object();
        let key = EvalValue::String("name".to_string());
        let result = method_has_key(&obj, &key).unwrap();
        assert_eq!(result, EvalValue::Boolean(true));
    }

    #[test]
    fn test_has_key_missing() {
        let obj = make_test_object();
        let key = EvalValue::String("missing".to_string());
        let result = method_has_key(&obj, &key).unwrap();
        assert_eq!(result, EvalValue::Boolean(false));
    }

    #[test]
    fn test_keys_requires_object() {
        let arr = EvalValue::Array(vec![EvalValue::Integer(1)]);
        let result = method_keys(&arr);
        assert!(result.is_err());
    }
}
