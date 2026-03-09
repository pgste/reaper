//! Aggregate methods for collections.
//!
//! This module provides aggregate operations on collections:
//! - count() - Number of items
//! - sum() - Sum of numeric values
//! - max() - Maximum value
//! - min() - Minimum value
//! - any() - Any element truthy
//! - all() - All elements truthy
//!
//! Performance characteristics:
//! - SIMD optimization for large arrays (>64 elements)
//! - Short-circuit evaluation for any/all

use super::super::types::EvalValue;
use reaper_core::ReaperError;

/// count() - Returns the number of items in a collection
/// Performance: O(1) for arrays/sets (length lookup), O(n) for object (key count)
#[inline]
pub fn method_count(value: &EvalValue) -> Result<EvalValue, ReaperError> {
    match value {
        EvalValue::Array(arr) => Ok(EvalValue::Integer(arr.len() as i64)),
        EvalValue::Set(set) => Ok(EvalValue::Integer(set.len() as i64)),
        EvalValue::Object(obj) => Ok(EvalValue::Integer(obj.len() as i64)),
        EvalValue::String(s) => Ok(EvalValue::Integer(s.len() as i64)), // Character count
        EvalValue::Null => Ok(EvalValue::Integer(0)),                   // Null counts as 0 items
        _ => Ok(EvalValue::Null), // Unsupported types return null (for type checking patterns)
    }
}

/// sum() - Sums all numeric values in a collection
/// Performance: O(n) with SIMD optimization for large arrays (>64 elements)
#[inline]
pub fn method_sum(items: &[EvalValue]) -> Result<EvalValue, ReaperError> {
    // Fast path for large pure-integer arrays using SIMD-friendly patterns
    if items.len() > 64 && items.iter().all(|v| matches!(v, EvalValue::Integer(_))) {
        let sum: i64 = items
            .iter()
            .filter_map(|v| {
                if let EvalValue::Integer(i) = v {
                    Some(*i)
                } else {
                    None
                }
            })
            .sum();
        return Ok(EvalValue::Integer(sum));
    }

    // Fast path for large pure-float arrays using SIMD-friendly patterns
    if items.len() > 64 && items.iter().all(|v| matches!(v, EvalValue::Float(_))) {
        let sum: f64 = items
            .iter()
            .filter_map(|v| {
                if let EvalValue::Float(f) = v {
                    Some(*f)
                } else {
                    None
                }
            })
            .sum();
        return Ok(EvalValue::Float(sum));
    }

    // Standard path for mixed types or small arrays
    let mut int_sum: i64 = 0;
    let mut has_float = false;
    let mut float_sum: f64 = 0.0;

    for item in items {
        match item {
            EvalValue::Integer(i) => {
                if has_float {
                    float_sum += *i as f64;
                } else {
                    int_sum += i;
                }
            }
            EvalValue::Float(f) => {
                if !has_float {
                    float_sum = int_sum as f64;
                    has_float = true;
                }
                float_sum += f;
            }
            _ => {
                return Err(ReaperError::InvalidPolicy {
                    reason: "sum() requires numeric values".to_string(),
                })
            }
        }
    }

    if has_float {
        Ok(EvalValue::Float(float_sum))
    } else {
        Ok(EvalValue::Integer(int_sum))
    }
}

/// max() - Finds the maximum value in a collection
/// Performance: O(n) with SIMD optimization for large arrays (>64 elements)
#[inline]
pub fn method_max(items: &[EvalValue]) -> Result<EvalValue, ReaperError> {
    if items.is_empty() {
        return Err(ReaperError::InvalidPolicy {
            reason: "max() requires non-empty collection".to_string(),
        });
    }

    // Fast path for large pure-integer arrays
    if items.len() > 64 && items.iter().all(|v| matches!(v, EvalValue::Integer(_))) {
        let max = items
            .iter()
            .filter_map(|v| {
                if let EvalValue::Integer(i) = v {
                    Some(*i)
                } else {
                    None
                }
            })
            .max()
            .unwrap();
        return Ok(EvalValue::Integer(max));
    }

    // Fast path for large pure-float arrays
    if items.len() > 64 && items.iter().all(|v| matches!(v, EvalValue::Float(_))) {
        let max = items
            .iter()
            .filter_map(|v| {
                if let EvalValue::Float(f) = v {
                    Some(*f)
                } else {
                    None
                }
            })
            .fold(f64::NEG_INFINITY, f64::max);
        return Ok(EvalValue::Float(max));
    }

    // Standard path for mixed types or small arrays
    let mut max_int: Option<i64> = None;
    let mut max_float: Option<f64> = None;
    let mut has_float = false;

    for item in items {
        match item {
            EvalValue::Integer(i) => {
                if has_float {
                    let i_as_float = *i as f64;
                    max_float = Some(max_float.map_or(i_as_float, |m| m.max(i_as_float)));
                } else {
                    max_int = Some(max_int.map_or(*i, |m| m.max(*i)));
                }
            }
            EvalValue::Float(f) => {
                if !has_float {
                    let current_max = max_int.map(|i| i as f64).unwrap_or(f64::NEG_INFINITY);
                    max_float = Some(current_max.max(*f));
                    has_float = true;
                } else {
                    max_float = Some(max_float.map_or(*f, |m| m.max(*f)));
                }
            }
            _ => {
                return Err(ReaperError::InvalidPolicy {
                    reason: "max() requires numeric values".to_string(),
                })
            }
        }
    }

    if has_float {
        Ok(EvalValue::Float(max_float.unwrap()))
    } else {
        Ok(EvalValue::Integer(max_int.unwrap()))
    }
}

/// min() - Finds the minimum value in a collection
/// Performance: O(n) with SIMD optimization for large arrays (>64 elements)
#[inline]
pub fn method_min(items: &[EvalValue]) -> Result<EvalValue, ReaperError> {
    if items.is_empty() {
        return Err(ReaperError::InvalidPolicy {
            reason: "min() requires non-empty collection".to_string(),
        });
    }

    // Fast path for large pure-integer arrays
    if items.len() > 64 && items.iter().all(|v| matches!(v, EvalValue::Integer(_))) {
        let min = items
            .iter()
            .filter_map(|v| {
                if let EvalValue::Integer(i) = v {
                    Some(*i)
                } else {
                    None
                }
            })
            .min()
            .unwrap();
        return Ok(EvalValue::Integer(min));
    }

    // Fast path for large pure-float arrays
    if items.len() > 64 && items.iter().all(|v| matches!(v, EvalValue::Float(_))) {
        let min = items
            .iter()
            .filter_map(|v| {
                if let EvalValue::Float(f) = v {
                    Some(*f)
                } else {
                    None
                }
            })
            .fold(f64::INFINITY, f64::min);
        return Ok(EvalValue::Float(min));
    }

    // Standard path for mixed types or small arrays
    let mut min_int: Option<i64> = None;
    let mut min_float: Option<f64> = None;
    let mut has_float = false;

    for item in items {
        match item {
            EvalValue::Integer(i) => {
                if has_float {
                    let i_as_float = *i as f64;
                    min_float = Some(min_float.map_or(i_as_float, |m| m.min(i_as_float)));
                } else {
                    min_int = Some(min_int.map_or(*i, |m| m.min(*i)));
                }
            }
            EvalValue::Float(f) => {
                if !has_float {
                    let current_min = min_int.map(|i| i as f64).unwrap_or(f64::INFINITY);
                    min_float = Some(current_min.min(*f));
                    has_float = true;
                } else {
                    min_float = Some(min_float.map_or(*f, |m| m.min(*f)));
                }
            }
            _ => {
                return Err(ReaperError::InvalidPolicy {
                    reason: "min() requires numeric values".to_string(),
                })
            }
        }
    }

    if has_float {
        Ok(EvalValue::Float(min_float.unwrap()))
    } else {
        Ok(EvalValue::Integer(min_int.unwrap()))
    }
}

/// any() - Returns true if any element is truthy
/// Performance: O(n) with short-circuit evaluation
#[inline]
pub fn method_any(items: &[EvalValue]) -> Result<EvalValue, ReaperError> {
    for item in items {
        let is_truthy = match item {
            EvalValue::Boolean(b) => *b,
            EvalValue::Integer(i) => *i != 0,
            EvalValue::Null => false,
            EvalValue::String(s) => !s.is_empty(),
            _ => true, // Arrays, objects, sets are truthy if they exist
        };

        if is_truthy {
            return Ok(EvalValue::Boolean(true));
        }
    }

    Ok(EvalValue::Boolean(false))
}

/// all() - Returns true if all elements are truthy
/// Performance: O(n) with short-circuit evaluation
#[inline]
pub fn method_all(items: &[EvalValue]) -> Result<EvalValue, ReaperError> {
    for item in items {
        let is_truthy = match item {
            EvalValue::Boolean(b) => *b,
            EvalValue::Integer(i) => *i != 0,
            EvalValue::Null => false,
            EvalValue::String(s) => !s.is_empty(),
            _ => true,
        };

        if !is_truthy {
            return Ok(EvalValue::Boolean(false));
        }
    }

    Ok(EvalValue::Boolean(true))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_count() {
        assert_eq!(
            method_count(&EvalValue::Array(vec![
                EvalValue::Integer(1),
                EvalValue::Integer(2)
            ]))
            .unwrap(),
            EvalValue::Integer(2)
        );
        assert_eq!(
            method_count(&EvalValue::String("hello".to_string())).unwrap(),
            EvalValue::Integer(5)
        );
    }

    #[test]
    fn test_sum() {
        let items = vec![
            EvalValue::Integer(1),
            EvalValue::Integer(2),
            EvalValue::Integer(3),
        ];
        assert_eq!(method_sum(&items).unwrap(), EvalValue::Integer(6));
    }

    #[test]
    fn test_max() {
        let items = vec![
            EvalValue::Integer(1),
            EvalValue::Integer(5),
            EvalValue::Integer(3),
        ];
        assert_eq!(method_max(&items).unwrap(), EvalValue::Integer(5));
    }

    #[test]
    fn test_min() {
        let items = vec![
            EvalValue::Integer(1),
            EvalValue::Integer(5),
            EvalValue::Integer(3),
        ];
        assert_eq!(method_min(&items).unwrap(), EvalValue::Integer(1));
    }

    #[test]
    fn test_any() {
        let items = vec![EvalValue::Boolean(false), EvalValue::Boolean(true)];
        assert_eq!(method_any(&items).unwrap(), EvalValue::Boolean(true));

        let items = vec![EvalValue::Boolean(false), EvalValue::Boolean(false)];
        assert_eq!(method_any(&items).unwrap(), EvalValue::Boolean(false));
    }

    #[test]
    fn test_all() {
        let items = vec![EvalValue::Boolean(true), EvalValue::Boolean(true)];
        assert_eq!(method_all(&items).unwrap(), EvalValue::Boolean(true));

        let items = vec![EvalValue::Boolean(true), EvalValue::Boolean(false)];
        assert_eq!(method_all(&items).unwrap(), EvalValue::Boolean(false));
    }
}
