//! Collection methods for AST evaluation.
//!
//! This module provides collection operations:
//! - first() / last() - Element access
//! - slice() - Subarray extraction
//! - reverse() - Reverse array
//! - sort() - Sort array
//! - unique() - Deduplicate to set

use super::super::types::EvalValue;
use reaper_core::ReaperError;
use std::collections::HashSet;

/// Helper: Extract items from a collection
#[inline]
pub fn get_collection_items(value: &EvalValue) -> Result<Vec<&EvalValue>, ReaperError> {
    match value {
        EvalValue::Array(arr) => Ok(arr.iter().collect()),
        EvalValue::Set(set) => Ok(set.iter().collect()),
        EvalValue::Object(obj) => Ok(obj.values().collect()),
        _ => Err(ReaperError::InvalidPolicy {
            reason: "Expected collection (array, set, or object)".to_string(),
        }),
    }
}

/// first() - Returns the first element of an array/set, or Null if empty
/// Note: For sets, returns an arbitrary element since sets don't have ordering
#[inline]
pub fn method_first(value: &EvalValue) -> Result<EvalValue, ReaperError> {
    match value {
        EvalValue::Array(arr) => match arr.first() {
            Some(elem) => Ok(elem.clone()),
            None => Ok(EvalValue::Null),
        },
        EvalValue::Set(set) => match set.first() {
            Some(elem) => Ok(elem.clone()),
            None => Ok(EvalValue::Null),
        },
        _ => Err(ReaperError::InvalidPolicy {
            reason: "first() requires an array or set".to_string(),
        }),
    }
}

/// last() - Returns the last element of an array/set, or Null if empty
/// Note: For sets, returns an arbitrary element since sets don't have ordering
#[inline]
pub fn method_last(value: &EvalValue) -> Result<EvalValue, ReaperError> {
    match value {
        EvalValue::Array(arr) => match arr.last() {
            Some(elem) => Ok(elem.clone()),
            None => Ok(EvalValue::Null),
        },
        EvalValue::Set(set) => match set.last() {
            Some(elem) => Ok(elem.clone()),
            None => Ok(EvalValue::Null),
        },
        _ => Err(ReaperError::InvalidPolicy {
            reason: "last() requires an array or set".to_string(),
        }),
    }
}

/// slice(start, end) - Extracts a subarray from start (inclusive) to end (exclusive)
#[inline]
pub fn method_slice(
    value: &EvalValue,
    start: &EvalValue,
    end: &EvalValue,
) -> Result<EvalValue, ReaperError> {
    match value {
        EvalValue::Array(arr) => {
            let start_idx = match start {
                EvalValue::Integer(i) => *i,
                _ => {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "slice() start index must be an integer".to_string(),
                    })
                }
            };

            let end_idx = match end {
                EvalValue::Integer(i) => *i,
                _ => {
                    return Err(ReaperError::InvalidPolicy {
                        reason: "slice() end index must be an integer".to_string(),
                    })
                }
            };

            let len = arr.len();
            let start_usize = if start_idx < 0 {
                0
            } else if start_idx as usize > len {
                len
            } else {
                start_idx as usize
            };

            let end_usize = if end_idx < 0 {
                0
            } else if end_idx as usize > len {
                len
            } else {
                end_idx as usize
            };

            if start_usize > end_usize {
                return Ok(EvalValue::Array(Vec::new()));
            }

            let sliced = arr[start_usize..end_usize].to_vec();
            Ok(EvalValue::Array(sliced))
        }
        _ => Err(ReaperError::InvalidPolicy {
            reason: "slice() requires an array".to_string(),
        }),
    }
}

/// reverse() - Returns a new array with elements in reverse order
#[inline]
pub fn method_reverse(value: &EvalValue) -> Result<EvalValue, ReaperError> {
    match value {
        EvalValue::Array(arr) => {
            let reversed: Vec<EvalValue> = arr.iter().rev().cloned().collect();
            Ok(EvalValue::Array(reversed))
        }
        _ => Err(ReaperError::InvalidPolicy {
            reason: "reverse() requires an array".to_string(),
        }),
    }
}

/// sort() - Returns a new array with elements sorted in ascending order
#[inline]
pub fn method_sort(value: &EvalValue) -> Result<EvalValue, ReaperError> {
    match value {
        EvalValue::Array(arr) => {
            if arr.is_empty() {
                return Ok(EvalValue::Array(Vec::new()));
            }

            let mut sorted = arr.clone();

            sorted.sort_by(|a, b| {
                use std::cmp::Ordering;

                match (a, b) {
                    (EvalValue::Integer(x), EvalValue::Integer(y)) => x.cmp(y),
                    (EvalValue::Float(x), EvalValue::Float(y)) => {
                        x.partial_cmp(y).unwrap_or(Ordering::Equal)
                    }
                    (EvalValue::Integer(x), EvalValue::Float(y)) => {
                        (*x as f64).partial_cmp(y).unwrap_or(Ordering::Equal)
                    }
                    (EvalValue::Float(x), EvalValue::Integer(y)) => {
                        x.partial_cmp(&(*y as f64)).unwrap_or(Ordering::Equal)
                    }
                    (EvalValue::String(x), EvalValue::String(y)) => x.cmp(y),
                    (EvalValue::Boolean(x), EvalValue::Boolean(y)) => x.cmp(y),
                    (EvalValue::Null, EvalValue::Null) => Ordering::Equal,
                    (EvalValue::Null, _) => Ordering::Less,
                    (_, EvalValue::Null) => Ordering::Greater,
                    _ => {
                        let type_order = |v: &EvalValue| match v {
                            EvalValue::Null => 0,
                            EvalValue::Boolean(_) => 1,
                            EvalValue::Integer(_) => 2,
                            EvalValue::Float(_) => 3,
                            EvalValue::String(_) => 4,
                            EvalValue::Array(_) => 5,
                            EvalValue::Set(_) => 6,
                            EvalValue::Object(_) => 7,
                        };
                        type_order(a).cmp(&type_order(b))
                    }
                }
            });

            Ok(EvalValue::Array(sorted))
        }
        _ => Err(ReaperError::InvalidPolicy {
            reason: "sort() requires an array".to_string(),
        }),
    }
}

/// unique() - Returns a Set containing only unique elements from array
#[inline]
pub fn method_unique(value: &EvalValue) -> Result<EvalValue, ReaperError> {
    match value {
        EvalValue::Array(arr) => {
            let unique_set: HashSet<EvalValue> = arr.iter().cloned().collect();
            let unique_vec: Vec<EvalValue> = unique_set.into_iter().collect();
            Ok(EvalValue::Set(unique_vec))
        }
        _ => Err(ReaperError::InvalidPolicy {
            reason: "unique() requires an array".to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_first() {
        let arr = EvalValue::Array(vec![
            EvalValue::Integer(1),
            EvalValue::Integer(2),
            EvalValue::Integer(3),
        ]);
        assert_eq!(method_first(&arr).unwrap(), EvalValue::Integer(1));

        let empty = EvalValue::Array(vec![]);
        assert_eq!(method_first(&empty).unwrap(), EvalValue::Null);
    }

    #[test]
    fn test_last() {
        let arr = EvalValue::Array(vec![
            EvalValue::Integer(1),
            EvalValue::Integer(2),
            EvalValue::Integer(3),
        ]);
        assert_eq!(method_last(&arr).unwrap(), EvalValue::Integer(3));
    }

    #[test]
    fn test_reverse() {
        let arr = EvalValue::Array(vec![
            EvalValue::Integer(1),
            EvalValue::Integer(2),
            EvalValue::Integer(3),
        ]);
        let result = method_reverse(&arr).unwrap();
        if let EvalValue::Array(reversed) = result {
            assert_eq!(reversed[0], EvalValue::Integer(3));
            assert_eq!(reversed[2], EvalValue::Integer(1));
        }
    }

    #[test]
    fn test_sort() {
        let arr = EvalValue::Array(vec![
            EvalValue::Integer(3),
            EvalValue::Integer(1),
            EvalValue::Integer(2),
        ]);
        let result = method_sort(&arr).unwrap();
        if let EvalValue::Array(sorted) = result {
            assert_eq!(sorted[0], EvalValue::Integer(1));
            assert_eq!(sorted[1], EvalValue::Integer(2));
            assert_eq!(sorted[2], EvalValue::Integer(3));
        }
    }
}
