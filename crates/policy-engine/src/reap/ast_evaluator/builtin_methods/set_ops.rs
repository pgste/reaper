//! Set operations for AST evaluation.
//!
//! This module provides set operations:
//! - union() - Union of two collections
//! - intersection() - Intersection of two collections
//! - difference() - Difference of two collections

use super::super::types::EvalValue;
use super::collection::get_collection_items;
use reaper_core::ReaperError;
use std::collections::HashSet;

/// union() - Returns union of two collections as a Set
#[inline]
pub fn method_union(value: &EvalValue, other: &EvalValue) -> Result<EvalValue, ReaperError> {
    let items1 = get_collection_items(value)?;
    let items2 = get_collection_items(other)?;

    let set1: HashSet<_> = items1.into_iter().cloned().collect();
    let set2: HashSet<_> = items2.into_iter().cloned().collect();

    let union: Vec<EvalValue> = set1.union(&set2).cloned().collect();
    Ok(EvalValue::Set(union))
}

/// intersection() - Returns intersection of two collections as a Set
#[inline]
pub fn method_intersection(value: &EvalValue, other: &EvalValue) -> Result<EvalValue, ReaperError> {
    let items1 = get_collection_items(value)?;
    let items2 = get_collection_items(other)?;

    let set1: HashSet<_> = items1.into_iter().cloned().collect();
    let set2: HashSet<_> = items2.into_iter().cloned().collect();

    let intersection: Vec<EvalValue> = set1.intersection(&set2).cloned().collect();
    Ok(EvalValue::Set(intersection))
}

/// difference() - Returns difference of two collections (items in first but not second)
#[inline]
pub fn method_difference(value: &EvalValue, other: &EvalValue) -> Result<EvalValue, ReaperError> {
    let items1 = get_collection_items(value)?;
    let items2 = get_collection_items(other)?;

    let set1: HashSet<_> = items1.into_iter().cloned().collect();
    let set2: HashSet<_> = items2.into_iter().cloned().collect();

    let difference: Vec<EvalValue> = set1.difference(&set2).cloned().collect();
    Ok(EvalValue::Set(difference))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_union() {
        let arr1 = EvalValue::Array(vec![EvalValue::Integer(1), EvalValue::Integer(2)]);
        let arr2 = EvalValue::Array(vec![EvalValue::Integer(2), EvalValue::Integer(3)]);

        let result = method_union(&arr1, &arr2).unwrap();
        if let EvalValue::Set(items) = result {
            assert_eq!(items.len(), 3);
        } else {
            panic!("Expected Set");
        }
    }

    #[test]
    fn test_intersection() {
        let arr1 = EvalValue::Array(vec![
            EvalValue::Integer(1),
            EvalValue::Integer(2),
            EvalValue::Integer(3),
        ]);
        let arr2 = EvalValue::Array(vec![
            EvalValue::Integer(2),
            EvalValue::Integer(3),
            EvalValue::Integer(4),
        ]);

        let result = method_intersection(&arr1, &arr2).unwrap();
        if let EvalValue::Set(items) = result {
            assert_eq!(items.len(), 2);
        } else {
            panic!("Expected Set");
        }
    }

    #[test]
    fn test_difference() {
        let arr1 = EvalValue::Array(vec![
            EvalValue::Integer(1),
            EvalValue::Integer(2),
            EvalValue::Integer(3),
        ]);
        let arr2 = EvalValue::Array(vec![EvalValue::Integer(2), EvalValue::Integer(3)]);

        let result = method_difference(&arr1, &arr2).unwrap();
        if let EvalValue::Set(items) = result {
            assert_eq!(items.len(), 1);
            assert!(items.contains(&EvalValue::Integer(1)));
        } else {
            panic!("Expected Set");
        }
    }
}
