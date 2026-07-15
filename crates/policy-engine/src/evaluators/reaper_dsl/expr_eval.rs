//! Expression evaluation helpers.
//!
//! This module handles expression type evaluation:
//! - String operations (lower, upper, trim, split)
//! - Collection operations (count, sum, min, max, average)
//! - Indexed access (collection[0], map["key"])
//! - Variable references
//!
//! ## Performance Characteristics
//! - String operations use interned strings for memory efficiency
//! - Collection operations are O(n) where n is collection size
//! - Map lookups are O(1) using interned keys

// Allow unused functions - some are used in tests only or reserved for future use
#![allow(dead_code)]

use crate::data::{AttributeValue, InternedString, StringInterner};
use std::collections::HashMap;

use super::entity_helpers::get_entity_for_type;
use super::types::EntityType;

/// Evaluate string lowercase: entity.attr.lower()
#[inline]
pub fn eval_string_lower(
    entity_type: &EntityType,
    attribute: InternedString,
    bindings: EntityBindings<'_>,
    interner: &StringInterner,
) -> Option<AttributeValue> {
    let entity = get_entity_for_type(entity_type, bindings)?;
    if let Some(AttributeValue::String(s)) = entity.get_attribute(attribute) {
        if let Some(resolved) = interner.resolve(*s) {
            let lower = resolved.to_lowercase();
            let interned = super::intern_transient(interner, &lower);
            return Some(AttributeValue::String(interned));
        }
    }
    None
}

/// Evaluate string uppercase: entity.attr.upper()
#[inline]
pub fn eval_string_upper(
    entity_type: &EntityType,
    attribute: InternedString,
    bindings: EntityBindings<'_>,
    interner: &StringInterner,
) -> Option<AttributeValue> {
    let entity = get_entity_for_type(entity_type, bindings)?;
    if let Some(AttributeValue::String(s)) = entity.get_attribute(attribute) {
        if let Some(resolved) = interner.resolve(*s) {
            let upper = resolved.to_uppercase();
            let interned = super::intern_transient(interner, &upper);
            return Some(AttributeValue::String(interned));
        }
    }
    None
}

/// Evaluate string trim: entity.attr.trim()
#[inline]
pub fn eval_string_trim(
    entity_type: &EntityType,
    attribute: InternedString,
    bindings: EntityBindings<'_>,
    interner: &StringInterner,
) -> Option<AttributeValue> {
    let entity = get_entity_for_type(entity_type, bindings)?;
    if let Some(AttributeValue::String(s)) = entity.get_attribute(attribute) {
        if let Some(resolved) = interner.resolve(*s) {
            let trimmed = resolved.trim().to_string();
            let interned = super::intern_transient(interner, &trimmed);
            return Some(AttributeValue::String(interned));
        }
    }
    None
}

/// Evaluate string split: entity.attr.split(delimiter)
#[inline]
pub fn eval_string_split(
    entity_type: &EntityType,
    attribute: InternedString,
    delimiter: &str,
    bindings: EntityBindings<'_>,
    interner: &StringInterner,
) -> Option<AttributeValue> {
    let entity = get_entity_for_type(entity_type, bindings)?;
    if let Some(AttributeValue::String(s)) = entity.get_attribute(attribute) {
        if let Some(resolved) = interner.resolve(*s) {
            let parts: Vec<AttributeValue> = resolved
                .split(delimiter)
                .map(|part| {
                    let interned = super::intern_transient(interner, part);
                    AttributeValue::String(interned)
                })
                .collect();
            return Some(AttributeValue::List(parts));
        }
    }
    None
}

/// Evaluate string replace: entity.attr.replace(pattern, replacement)
#[inline]
pub fn eval_string_replace(
    entity_type: &EntityType,
    attribute: InternedString,
    pattern: &str,
    replacement: &str,
    bindings: EntityBindings<'_>,
    interner: &StringInterner,
) -> Option<AttributeValue> {
    let entity = get_entity_for_type(entity_type, bindings)?;
    if let Some(AttributeValue::String(s)) = entity.get_attribute(attribute) {
        if let Some(resolved) = interner.resolve(*s) {
            let replaced = resolved.replace(pattern, replacement);
            let interned = super::intern_transient(interner, &replaced);
            return Some(AttributeValue::String(interned));
        }
    }
    None
}

/// Evaluate string substring: entity.attr.substring(start, end)
#[inline]
pub fn eval_string_substring(
    entity_type: &EntityType,
    attribute: InternedString,
    start: usize,
    end: Option<usize>,
    bindings: EntityBindings<'_>,
    interner: &StringInterner,
) -> Option<AttributeValue> {
    let entity = get_entity_for_type(entity_type, bindings)?;
    if let Some(AttributeValue::String(s)) = entity.get_attribute(attribute) {
        if let Some(resolved) = interner.resolve(*s) {
            let end_idx = end.unwrap_or(resolved.len()).min(resolved.len());
            let start_idx = start.min(end_idx);
            // Handle UTF-8 properly by using chars
            let substring: String = resolved
                .chars()
                .skip(start_idx)
                .take(end_idx - start_idx)
                .collect();
            let interned = super::intern_transient(interner, &substring);
            return Some(AttributeValue::String(interned));
        }
    }
    None
}

/// Evaluate collection count: entity.collection.count()
/// Also works on strings to return character count.
#[inline]
pub fn eval_collection_count(
    entity_type: &EntityType,
    attribute: InternedString,
    bindings: EntityBindings<'_>,
    interner: &StringInterner,
) -> Option<AttributeValue> {
    let entity = get_entity_for_type(entity_type, bindings)?;
    match entity.get_attribute(attribute) {
        Some(AttributeValue::List(items)) => Some(AttributeValue::Int(items.len() as i64)),
        Some(AttributeValue::Set(items)) => Some(AttributeValue::Int(items.len() as i64)),
        Some(AttributeValue::Object(map)) => Some(AttributeValue::Int(map.len() as i64)),
        Some(AttributeValue::String(s)) => interner
            .resolve(*s)
            .map(|resolved| AttributeValue::Int(resolved.len() as i64)),
        Some(AttributeValue::Null) => Some(AttributeValue::Int(0)),
        _ => None,
    }
}

/// Evaluate collection sum: entity.collection.sum()
#[inline]
pub fn eval_collection_sum(
    entity_type: &EntityType,
    attribute: InternedString,
    bindings: EntityBindings<'_>,
) -> Option<AttributeValue> {
    let entity = get_entity_for_type(entity_type, bindings)?;
    match entity.get_attribute(attribute) {
        Some(AttributeValue::List(items)) => {
            let sum: f64 = items
                .iter()
                .filter_map(|item| match item {
                    AttributeValue::Int(n) => Some(*n as f64),
                    AttributeValue::Float(f) => Some(*f),
                    _ => None,
                })
                .sum();
            // Return as int if it's a whole number, otherwise float
            if sum.fract() == 0.0 {
                Some(AttributeValue::Int(sum as i64))
            } else {
                Some(AttributeValue::Float(sum))
            }
        }
        Some(AttributeValue::Set(items)) => {
            let sum: f64 = items
                .iter()
                .filter_map(|item| match item {
                    AttributeValue::Int(n) => Some(*n as f64),
                    AttributeValue::Float(f) => Some(*f),
                    _ => None,
                })
                .sum();
            if sum.fract() == 0.0 {
                Some(AttributeValue::Int(sum as i64))
            } else {
                Some(AttributeValue::Float(sum))
            }
        }
        _ => None,
    }
}

/// Evaluate collection min: entity.collection.min()
#[inline]
pub fn eval_collection_min(
    entity_type: &EntityType,
    attribute: InternedString,
    bindings: EntityBindings<'_>,
) -> Option<AttributeValue> {
    let entity = get_entity_for_type(entity_type, bindings)?;
    match entity.get_attribute(attribute) {
        Some(AttributeValue::List(items)) => {
            let min = items
                .iter()
                .filter_map(|item| match item {
                    AttributeValue::Int(n) => Some(*n as f64),
                    AttributeValue::Float(f) => Some(*f),
                    _ => None,
                })
                .fold(f64::INFINITY, f64::min);

            if min.is_infinite() {
                None
            } else if min.fract() == 0.0 {
                Some(AttributeValue::Int(min as i64))
            } else {
                Some(AttributeValue::Float(min))
            }
        }
        Some(AttributeValue::Set(items)) => {
            let min = items
                .iter()
                .filter_map(|item| match item {
                    AttributeValue::Int(n) => Some(*n as f64),
                    AttributeValue::Float(f) => Some(*f),
                    _ => None,
                })
                .fold(f64::INFINITY, f64::min);

            if min.is_infinite() {
                None
            } else if min.fract() == 0.0 {
                Some(AttributeValue::Int(min as i64))
            } else {
                Some(AttributeValue::Float(min))
            }
        }
        _ => None,
    }
}

/// Evaluate collection max: entity.collection.max()
#[inline]
pub fn eval_collection_max(
    entity_type: &EntityType,
    attribute: InternedString,
    bindings: EntityBindings<'_>,
) -> Option<AttributeValue> {
    let entity = get_entity_for_type(entity_type, bindings)?;
    match entity.get_attribute(attribute) {
        Some(AttributeValue::List(items)) => {
            let max = items
                .iter()
                .filter_map(|item| match item {
                    AttributeValue::Int(n) => Some(*n as f64),
                    AttributeValue::Float(f) => Some(*f),
                    _ => None,
                })
                .fold(f64::NEG_INFINITY, f64::max);

            if max.is_infinite() {
                None
            } else if max.fract() == 0.0 {
                Some(AttributeValue::Int(max as i64))
            } else {
                Some(AttributeValue::Float(max))
            }
        }
        Some(AttributeValue::Set(items)) => {
            let max = items
                .iter()
                .filter_map(|item| match item {
                    AttributeValue::Int(n) => Some(*n as f64),
                    AttributeValue::Float(f) => Some(*f),
                    _ => None,
                })
                .fold(f64::NEG_INFINITY, f64::max);

            if max.is_infinite() {
                None
            } else if max.fract() == 0.0 {
                Some(AttributeValue::Int(max as i64))
            } else {
                Some(AttributeValue::Float(max))
            }
        }
        _ => None,
    }
}

/// Evaluate collection average: entity.collection.average()
#[inline]
pub fn eval_collection_average(
    entity_type: &EntityType,
    attribute: InternedString,
    bindings: EntityBindings<'_>,
) -> Option<AttributeValue> {
    let entity = get_entity_for_type(entity_type, bindings)?;
    match entity.get_attribute(attribute) {
        Some(AttributeValue::List(items)) => {
            let nums: Vec<f64> = items
                .iter()
                .filter_map(|item| match item {
                    AttributeValue::Int(n) => Some(*n as f64),
                    AttributeValue::Float(f) => Some(*f),
                    _ => None,
                })
                .collect();
            if nums.is_empty() {
                None
            } else {
                let sum: f64 = nums.iter().sum();
                Some(AttributeValue::Float(sum / nums.len() as f64))
            }
        }
        Some(AttributeValue::Set(items)) => {
            let nums: Vec<f64> = items
                .iter()
                .filter_map(|item| match item {
                    AttributeValue::Int(n) => Some(*n as f64),
                    AttributeValue::Float(f) => Some(*f),
                    _ => None,
                })
                .collect();
            if nums.is_empty() {
                None
            } else {
                let sum: f64 = nums.iter().sum();
                Some(AttributeValue::Float(sum / nums.len() as f64))
            }
        }
        _ => None,
    }
}

/// Evaluate collection first: entity.collection.first()
#[inline]
pub fn eval_collection_first(
    entity_type: &EntityType,
    attribute: InternedString,
    bindings: EntityBindings<'_>,
) -> Option<AttributeValue> {
    let entity = get_entity_for_type(entity_type, bindings)?;
    match entity.get_attribute(attribute) {
        Some(AttributeValue::List(items)) => items.first().cloned(),
        Some(AttributeValue::Set(items)) => items.iter().next().cloned(),
        _ => None,
    }
}

/// Evaluate collection last: entity.collection.last()
#[inline]
pub fn eval_collection_last(
    entity_type: &EntityType,
    attribute: InternedString,
    bindings: EntityBindings<'_>,
) -> Option<AttributeValue> {
    let entity = get_entity_for_type(entity_type, bindings)?;
    match entity.get_attribute(attribute) {
        Some(AttributeValue::List(items)) => items.last().cloned(),
        Some(AttributeValue::Set(items)) => items.iter().last().cloned(),
        _ => None,
    }
}

/// Evaluate collection reverse: entity.collection.reverse()
#[inline]
pub fn eval_collection_reverse(
    entity_type: &EntityType,
    attribute: InternedString,
    bindings: EntityBindings<'_>,
) -> Option<AttributeValue> {
    let entity = get_entity_for_type(entity_type, bindings)?;
    match entity.get_attribute(attribute) {
        Some(AttributeValue::List(items)) => {
            let mut reversed = items.clone();
            reversed.reverse();
            Some(AttributeValue::List(reversed))
        }
        _ => None,
    }
}

/// Evaluate map keys: entity.map.keys()
#[inline]
pub fn eval_map_keys(
    entity_type: &EntityType,
    attribute: InternedString,
    bindings: EntityBindings<'_>,
) -> Option<AttributeValue> {
    let entity = get_entity_for_type(entity_type, bindings)?;
    match entity.get_attribute(attribute) {
        Some(AttributeValue::Object(map)) => {
            let keys: Vec<AttributeValue> =
                map.keys().copied().map(AttributeValue::String).collect();
            Some(AttributeValue::List(keys))
        }
        _ => None,
    }
}

/// Evaluate map values: entity.map.values()
#[inline]
pub fn eval_map_values(
    entity_type: &EntityType,
    attribute: InternedString,
    bindings: EntityBindings<'_>,
) -> Option<AttributeValue> {
    let entity = get_entity_for_type(entity_type, bindings)?;
    match entity.get_attribute(attribute) {
        Some(AttributeValue::Object(map)) => {
            let values: Vec<AttributeValue> = map.values().cloned().collect();
            Some(AttributeValue::List(values))
        }
        _ => None,
    }
}

/// Evaluate variable reference: variable_name
#[inline]
pub fn eval_variable_ref(
    variable: InternedString,
    variables: &HashMap<String, AttributeValue>,
    interner: &StringInterner,
) -> Option<AttributeValue> {
    if let Some(var_name) = interner.resolve(variable) {
        variables.get(&*var_name).cloned()
    } else {
        None
    }
}

/// Evaluate indexed access: entity.collection[index]
#[inline]
pub fn eval_indexed_access(
    entity_type: &EntityType,
    attribute: InternedString,
    index: i64,
    bindings: EntityBindings<'_>,
) -> Option<AttributeValue> {
    let entity = get_entity_for_type(entity_type, bindings)?;
    match entity.get_attribute(attribute) {
        Some(AttributeValue::List(items)) => {
            let idx = if index < 0 {
                // Negative indexing from end
                items.len().checked_sub((-index) as usize)?
            } else {
                index as usize
            };
            items.get(idx).cloned()
        }
        _ => None,
    }
}

/// Evaluate map access: entity.map["key"]
#[inline]
pub fn eval_map_access(
    entity_type: &EntityType,
    attribute: InternedString,
    key: &str,
    bindings: EntityBindings<'_>,
    interner: &StringInterner,
) -> Option<AttributeValue> {
    let entity = get_entity_for_type(entity_type, bindings)?;
    match entity.get_attribute(attribute) {
        Some(AttributeValue::Object(map)) => {
            let key_interned = interner.intern(key);
            map.get(&key_interned).cloned()
        }
        _ => None,
    }
}

/// Evaluate collection unique: entity.collection.unique()
#[inline]
pub fn eval_collection_unique(
    entity_type: &EntityType,
    attribute: InternedString,
    bindings: EntityBindings<'_>,
) -> Option<AttributeValue> {
    let entity = get_entity_for_type(entity_type, bindings)?;
    match entity.get_attribute(attribute) {
        Some(AttributeValue::List(items)) => {
            let mut seen = std::collections::HashSet::new();
            let unique: Vec<AttributeValue> = items
                .iter()
                .filter(|item| {
                    // Use Debug format for deduplication
                    let key = format!("{:?}", item);
                    seen.insert(key)
                })
                .cloned()
                .collect();
            Some(AttributeValue::List(unique))
        }
        Some(AttributeValue::Set(items)) => {
            // Sets are already unique
            let list: Vec<AttributeValue> = items.iter().cloned().collect();
            Some(AttributeValue::List(list))
        }
        _ => None,
    }
}

/// Evaluate collection sort: entity.collection.sort()
#[inline]
pub fn eval_collection_sort(
    entity_type: &EntityType,
    attribute: InternedString,
    bindings: EntityBindings<'_>,
    interner: &StringInterner,
) -> Option<AttributeValue> {
    let entity = get_entity_for_type(entity_type, bindings)?;
    match entity.get_attribute(attribute) {
        Some(AttributeValue::List(items)) => {
            let mut sorted = items.clone();
            sorted.sort_by(|a, b| compare_attribute_values(a, b, interner));
            Some(AttributeValue::List(sorted))
        }
        Some(AttributeValue::Set(items)) => {
            let mut sorted: Vec<AttributeValue> = items.iter().cloned().collect();
            sorted.sort_by(|a, b| compare_attribute_values(a, b, interner));
            Some(AttributeValue::List(sorted))
        }
        _ => None,
    }
}

/// Compare two AttributeValue for sorting
#[inline]
pub(super) fn compare_attribute_values(
    a: &AttributeValue,
    b: &AttributeValue,
    interner: &StringInterner,
) -> std::cmp::Ordering {
    match (a, b) {
        (AttributeValue::Int(x), AttributeValue::Int(y)) => x.cmp(y),
        (AttributeValue::Float(x), AttributeValue::Float(y)) => {
            x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal)
        }
        (AttributeValue::Int(x), AttributeValue::Float(y)) => (*x as f64)
            .partial_cmp(y)
            .unwrap_or(std::cmp::Ordering::Equal),
        (AttributeValue::Float(x), AttributeValue::Int(y)) => x
            .partial_cmp(&(*y as f64))
            .unwrap_or(std::cmp::Ordering::Equal),
        (AttributeValue::String(x), AttributeValue::String(y)) => {
            let x_str = interner
                .resolve(*x)
                .map(|s| s.to_string())
                .unwrap_or_default();
            let y_str = interner
                .resolve(*y)
                .map(|s| s.to_string())
                .unwrap_or_default();
            x_str.cmp(&y_str)
        }
        (AttributeValue::Bool(x), AttributeValue::Bool(y)) => x.cmp(y),
        // Different types: use Debug format for deterministic ordering
        _ => {
            let a_str = format!("{:?}", a);
            let b_str = format!("{:?}", b);
            a_str.cmp(&b_str)
        }
    }
}

// ============================================================================
// Main Expression Type Evaluation (Dispatch)
// ============================================================================

use super::chain_method_eval::evaluate_chained_method;
use super::types::{CompiledExprIndexType, CompiledExprType, EntityBindings};

/// Evaluate a compiled expression type and return the result.
///
/// This is the main dispatch function for expression evaluation. It handles:
/// - String operations (lower, upper, trim, split, contains, startswith, endswith)
/// - Collection operations (count, sum, min, max, first, last, slice, reverse, sort, unique)
/// - Set operations (intersection, union, difference, keys)
/// - Time operations (now, now_ms, now_ns)
/// - Regex operations (matches, find)
/// - Variable references and indexed access
/// - Chained method calls
pub fn evaluate_compiled_expr_type(
    expr_type: &CompiledExprType,
    bindings: EntityBindings<'_>,
    variables: &HashMap<String, AttributeValue>,
    interner: &StringInterner,
) -> Option<AttributeValue> {
    match expr_type {
        CompiledExprType::StringLower {
            entity_type,
            attribute,
        } => eval_string_lower(entity_type, *attribute, bindings, interner),

        CompiledExprType::StringUpper {
            entity_type,
            attribute,
        } => eval_string_upper(entity_type, *attribute, bindings, interner),

        CompiledExprType::StringTrim {
            entity_type,
            attribute,
        } => eval_string_trim(entity_type, *attribute, bindings, interner),

        CompiledExprType::StringSplit {
            entity_type,
            attribute,
            delimiter,
        } => eval_string_split(entity_type, *attribute, delimiter, bindings, interner),

        CompiledExprType::CollectionCount {
            entity_type,
            attribute,
        } => eval_collection_count(entity_type, *attribute, bindings, interner),

        CompiledExprType::CollectionSum {
            entity_type,
            attribute,
        } => eval_collection_sum(entity_type, *attribute, bindings),

        CompiledExprType::CollectionMax {
            entity_type,
            attribute,
        } => eval_collection_max(entity_type, *attribute, bindings),

        CompiledExprType::CollectionMin {
            entity_type,
            attribute,
        } => eval_collection_min(entity_type, *attribute, bindings),

        CompiledExprType::CollectionFirst {
            entity_type,
            attribute,
        } => eval_collection_first(entity_type, *attribute, bindings),

        CompiledExprType::CollectionLast {
            entity_type,
            attribute,
        } => eval_collection_last(entity_type, *attribute, bindings),

        CompiledExprType::CollectionSlice {
            entity_type,
            attribute,
            start,
            end,
        } => {
            let entity = get_entity_for_type(entity_type, bindings)?;
            match entity.get_attribute(*attribute) {
                Some(AttributeValue::List(items)) => {
                    let start_idx = (*start).max(0) as usize;
                    let end_idx = (*end).max(0) as usize;
                    if start_idx < items.len() {
                        let end_idx = end_idx.min(items.len());
                        Some(AttributeValue::List(items[start_idx..end_idx].to_vec()))
                    } else {
                        Some(AttributeValue::List(vec![]))
                    }
                }
                _ => None,
            }
        }

        CompiledExprType::CollectionReverse {
            entity_type,
            attribute,
        } => eval_collection_reverse(entity_type, *attribute, bindings),

        CompiledExprType::CollectionSort {
            entity_type,
            attribute,
        } => eval_collection_sort(entity_type, *attribute, bindings, interner),

        CompiledExprType::CollectionUnique {
            entity_type,
            attribute,
        } => eval_collection_unique(entity_type, *attribute, bindings),

        CompiledExprType::CollectionDifference {
            entity_type,
            attribute,
            other_entity_type,
            other_attribute,
        } => {
            let entity = get_entity_for_type(entity_type, bindings)?;
            let other_entity = get_entity_for_type(other_entity_type, bindings)?;

            let items: Vec<AttributeValue> = match entity.get_attribute(*attribute) {
                Some(AttributeValue::List(items)) => items.clone(),
                Some(AttributeValue::Set(items)) => items.iter().cloned().collect(),
                _ => return None,
            };

            let other_items: std::collections::HashSet<String> =
                match other_entity.get_attribute(*other_attribute) {
                    Some(AttributeValue::List(items)) => items
                        .iter()
                        .filter_map(|item| {
                            if let AttributeValue::String(s) = item {
                                interner.resolve(*s).map(|s| s.to_string())
                            } else {
                                None
                            }
                        })
                        .collect(),
                    Some(AttributeValue::Set(items)) => items
                        .iter()
                        .filter_map(|item| {
                            if let AttributeValue::String(s) = item {
                                interner.resolve(*s).map(|s| s.to_string())
                            } else {
                                None
                            }
                        })
                        .collect(),
                    _ => return None,
                };

            let difference: Vec<AttributeValue> = items
                .into_iter()
                .filter(|item| {
                    if let AttributeValue::String(s) = item {
                        if let Some(resolved) = interner.resolve(*s) {
                            !other_items.contains(&*resolved)
                        } else {
                            true
                        }
                    } else {
                        true
                    }
                })
                .collect();

            Some(AttributeValue::List(difference))
        }

        CompiledExprType::CollectionUnion {
            entity_type,
            attribute,
            other_entity_type,
            other_attribute,
        } => {
            let entity = get_entity_for_type(entity_type, bindings)?;
            let other_entity = get_entity_for_type(other_entity_type, bindings)?;

            let mut items: Vec<AttributeValue> = match entity.get_attribute(*attribute) {
                Some(AttributeValue::List(items)) => items.clone(),
                Some(AttributeValue::Set(items)) => items.iter().cloned().collect(),
                _ => return None,
            };

            let existing: std::collections::HashSet<String> = items
                .iter()
                .filter_map(|item| {
                    if let AttributeValue::String(s) = item {
                        interner.resolve(*s).map(|s| s.to_string())
                    } else {
                        None
                    }
                })
                .collect();

            let other_items: Vec<AttributeValue> =
                match other_entity.get_attribute(*other_attribute) {
                    Some(AttributeValue::List(items)) => items.clone(),
                    Some(AttributeValue::Set(items)) => items.iter().cloned().collect(),
                    _ => return None,
                };

            for item in other_items {
                if let AttributeValue::String(s) = &item {
                    if let Some(resolved) = interner.resolve(*s) {
                        if !existing.contains(&*resolved) {
                            items.push(item);
                        }
                    } else {
                        items.push(item);
                    }
                } else {
                    items.push(item);
                }
            }

            Some(AttributeValue::List(items))
        }

        CompiledExprType::CollectionIntersection {
            entity_type,
            attribute,
            other_entity_type,
            other_attribute,
        } => {
            let entity = get_entity_for_type(entity_type, bindings)?;
            let other_entity = get_entity_for_type(other_entity_type, bindings)?;

            let items: Vec<AttributeValue> = match entity.get_attribute(*attribute) {
                Some(AttributeValue::List(items)) => items.clone(),
                Some(AttributeValue::Set(items)) => items.iter().cloned().collect(),
                _ => return None,
            };

            let other_items: std::collections::HashSet<String> =
                match other_entity.get_attribute(*other_attribute) {
                    Some(AttributeValue::List(items)) => items
                        .iter()
                        .filter_map(|item| {
                            if let AttributeValue::String(s) = item {
                                interner.resolve(*s).map(|s| s.to_string())
                            } else {
                                None
                            }
                        })
                        .collect(),
                    Some(AttributeValue::Set(items)) => items
                        .iter()
                        .filter_map(|item| {
                            if let AttributeValue::String(s) = item {
                                interner.resolve(*s).map(|s| s.to_string())
                            } else {
                                None
                            }
                        })
                        .collect(),
                    _ => return None,
                };

            let intersection: Vec<AttributeValue> = items
                .into_iter()
                .filter(|item| {
                    if let AttributeValue::String(s) = item {
                        if let Some(resolved) = interner.resolve(*s) {
                            other_items.contains(&*resolved)
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                })
                .collect();

            Some(AttributeValue::List(intersection))
        }

        CompiledExprType::SetIntersection {
            entity_type,
            attribute,
            values,
        } => {
            let entity = get_entity_for_type(entity_type, bindings)?;
            let items_vec: Vec<AttributeValue> = match entity.get_attribute(*attribute) {
                Some(AttributeValue::List(items)) => items.clone(),
                Some(AttributeValue::Set(items)) => items.iter().cloned().collect(),
                _ => return None,
            };
            let intersection: Vec<AttributeValue> = items_vec
                .iter()
                .filter(|item| {
                    if let AttributeValue::String(s) = item {
                        values.contains(s)
                    } else {
                        false
                    }
                })
                .cloned()
                .collect();
            Some(AttributeValue::List(intersection))
        }

        CompiledExprType::SetUnion {
            entity_type,
            attribute,
            values,
        } => {
            let entity = get_entity_for_type(entity_type, bindings)?;
            match entity.get_attribute(*attribute) {
                Some(AttributeValue::List(items)) => {
                    let mut union: Vec<AttributeValue> = items.clone();
                    for v in values {
                        if !items
                            .iter()
                            .any(|item| matches!(item, AttributeValue::String(s) if *s == *v))
                        {
                            union.push(AttributeValue::String(*v));
                        }
                    }
                    Some(AttributeValue::List(union))
                }
                _ => None,
            }
        }

        CompiledExprType::SetDifference {
            entity_type,
            attribute,
            values,
        } => {
            let entity = get_entity_for_type(entity_type, bindings)?;
            let items_vec: Vec<AttributeValue> = match entity.get_attribute(*attribute) {
                Some(AttributeValue::List(items)) => items.clone(),
                Some(AttributeValue::Set(items)) => items.iter().cloned().collect(),
                _ => return None,
            };
            // Keep only items NOT present in the literal set (attr - values).
            let difference: Vec<AttributeValue> = items_vec
                .iter()
                .filter(|item| {
                    if let AttributeValue::String(s) = item {
                        !values.contains(s)
                    } else {
                        // Non-string items are never in the (string) literal set.
                        true
                    }
                })
                .cloned()
                .collect();
            Some(AttributeValue::List(difference))
        }

        CompiledExprType::SetKeys {
            entity_type,
            attribute,
        } => {
            let entity = get_entity_for_type(entity_type, bindings)?;
            match entity.get_attribute(*attribute) {
                Some(AttributeValue::Object(map)) => {
                    let keys: Vec<AttributeValue> =
                        map.keys().map(|k| AttributeValue::String(*k)).collect();
                    Some(AttributeValue::List(keys))
                }
                _ => None,
            }
        }

        CompiledExprType::SetValues {
            entity_type,
            attribute,
        } => {
            let entity = get_entity_for_type(entity_type, bindings)?;
            match entity.get_attribute(*attribute) {
                // Matches AST method_values: the object's values as a list.
                Some(AttributeValue::Object(map)) => {
                    let values: Vec<AttributeValue> = map.values().cloned().collect();
                    Some(AttributeValue::List(values))
                }
                _ => None,
            }
        }

        CompiledExprType::TimeNow | CompiledExprType::TimeNowMs | CompiledExprType::TimeNowNs => {
            let now_ns = crate::clock::now_unix_ns().unwrap_or(0);
            let value = match expr_type {
                CompiledExprType::TimeNow => now_ns / 1_000_000_000,
                CompiledExprType::TimeNowMs => now_ns / 1_000_000,
                CompiledExprType::TimeNowNs => now_ns,
                _ => unreachable!(),
            };
            Some(AttributeValue::Int(value))
        }

        // taint::level("key") — the trust level of one context key under the
        // request provenance's fail-untrusted rule, as one of three constant
        // strings. Interning is idempotent on a 3-value set, so this pins
        // nothing new after the first evaluation and compares by id against
        // pre-interned rule literals like "verified".
        CompiledExprType::TaintLevel { key } => {
            let level = match bindings.context_trust(key) {
                crate::TrustLevel::Platform => "platform",
                crate::TrustLevel::Verified => "verified",
                crate::TrustLevel::Llm => "llm",
            };
            Some(AttributeValue::String(interner.intern(level)))
        }

        CompiledExprType::StringContains {
            entity_type,
            attribute,
            substring,
        } => {
            let entity = get_entity_for_type(entity_type, bindings)?;
            if let Some(AttributeValue::String(s)) = entity.get_attribute(*attribute) {
                if let Some(resolved) = interner.resolve(*s) {
                    return Some(AttributeValue::Bool(resolved.contains(substring.as_str())));
                }
            }
            None
        }

        CompiledExprType::StringStartsWithExpr {
            entity_type,
            attribute,
            prefix,
        } => {
            let entity = get_entity_for_type(entity_type, bindings)?;
            if let Some(AttributeValue::String(s)) = entity.get_attribute(*attribute) {
                if let Some(resolved) = interner.resolve(*s) {
                    return Some(AttributeValue::Bool(resolved.starts_with(prefix.as_str())));
                }
            }
            None
        }

        CompiledExprType::StringEndsWithExpr {
            entity_type,
            attribute,
            suffix,
        } => {
            let entity = get_entity_for_type(entity_type, bindings)?;
            if let Some(AttributeValue::String(s)) = entity.get_attribute(*attribute) {
                if let Some(resolved) = interner.resolve(*s) {
                    return Some(AttributeValue::Bool(resolved.ends_with(suffix.as_str())));
                }
            }
            None
        }

        CompiledExprType::RegexMatches {
            entity_type,
            attribute,
            pattern,
        } => {
            let entity = get_entity_for_type(entity_type, bindings)?;
            if let Some(AttributeValue::String(s)) = entity.get_attribute(*attribute) {
                if let Some(resolved) = interner.resolve(*s) {
                    let matches = crate::regex_cache::matches(pattern, &resolved);
                    return Some(AttributeValue::Bool(matches));
                }
            }
            None
        }

        CompiledExprType::RegexFind {
            entity_type,
            attribute,
            pattern,
        } => {
            let entity = get_entity_for_type(entity_type, bindings)?;
            if let Some(AttributeValue::String(s)) = entity.get_attribute(*attribute) {
                if let Some(resolved) = interner.resolve(*s) {
                    if let Some(re) = crate::regex_cache::get_or_compile(pattern) {
                        if let Some(m) = re.find(&resolved) {
                            let interned = super::intern_transient(interner, m.as_str());
                            return Some(AttributeValue::String(interned));
                        }
                    }
                }
            }
            None
        }

        CompiledExprType::RegexFindAll {
            entity_type,
            attribute,
            pattern,
        } => {
            let entity = get_entity_for_type(entity_type, bindings)?;
            if let Some(AttributeValue::String(s)) = entity.get_attribute(*attribute) {
                if let Some(resolved) = interner.resolve(*s) {
                    if let Some(re) = crate::regex_cache::get_or_compile(pattern) {
                        // Every match as a list, matching AST method_find_all.
                        let matches: Vec<AttributeValue> = re
                            .find_iter(&resolved)
                            .map(|m| {
                                AttributeValue::String(super::intern_transient(
                                    interner,
                                    m.as_str(),
                                ))
                            })
                            .collect();
                        return Some(AttributeValue::List(matches));
                    }
                }
            }
            None
        }

        CompiledExprType::StringReplace {
            entity_type,
            attribute,
            pattern,
            replacement,
        } => {
            let entity = get_entity_for_type(entity_type, bindings)?;
            if let Some(AttributeValue::String(s)) = entity.get_attribute(*attribute) {
                if let Some(resolved) = interner.resolve(*s) {
                    if let Some(re) = crate::regex_cache::get_or_compile(pattern) {
                        // Regex replace-all, matching AST method_replace.
                        let result = re.replace_all(&resolved, replacement.as_str());
                        let interned = super::intern_transient(interner, &result);
                        return Some(AttributeValue::String(interned));
                    }
                }
            }
            None
        }

        CompiledExprType::ChainedMethod { base, method } => {
            // First evaluate the base expression recursively
            let base_value = evaluate_compiled_expr_type(base, bindings, variables, interner)?;

            // Then apply the chained method
            evaluate_chained_method(method, base_value, interner)
        }

        CompiledExprType::VariableRef { variable } => {
            if let Some(var_name) = interner.resolve(*variable) {
                variables.get(&*var_name).cloned()
            } else {
                None
            }
        }

        CompiledExprType::VariableIndexed { variable, index } => {
            let var_name = interner.resolve(*variable)?;
            let var_value = variables.get(&*var_name)?;

            match index {
                CompiledExprIndexType::Wildcard => Some(var_value.clone()),
                CompiledExprIndexType::Number(n) => match var_value {
                    AttributeValue::List(items) => {
                        let idx = *n as usize;
                        items.get(idx).cloned()
                    }
                    _ => None,
                },
                CompiledExprIndexType::String(key) => match var_value {
                    AttributeValue::Object(map) => map.get(key).cloned(),
                    _ => None,
                },
            }
        }

        CompiledExprType::VariableAttrAccess {
            variable,
            attribute,
        } => {
            let var_name = interner.resolve(*variable)?;
            let var_value = variables.get(&*var_name)?;
            let attr_name = interner.resolve(*attribute)?;

            match var_value {
                AttributeValue::Object(map) => map.get(attribute).cloned().or_else(|| {
                    let attr_interned = interner.intern(&attr_name);
                    map.get(&attr_interned).cloned()
                }),
                _ => None,
            }
        }

        CompiledExprType::VariableAttrIndexed {
            variable,
            attribute,
            index,
        } => {
            let var_name = interner.resolve(*variable)?;
            let var_value = variables.get(&*var_name)?;
            let attr_name = interner.resolve(*attribute)?;

            let attr_value = match var_value {
                AttributeValue::Object(map) => map.get(attribute).cloned().or_else(|| {
                    let attr_interned = interner.intern(&attr_name);
                    map.get(&attr_interned).cloned()
                })?,
                _ => return None,
            };

            match (attr_value, index) {
                (AttributeValue::List(items), CompiledExprIndexType::Number(n)) => {
                    let idx = if *n >= 0 { *n as usize } else { return None };
                    items.get(idx).cloned()
                }
                (AttributeValue::List(items), CompiledExprIndexType::Wildcard) => {
                    Some(AttributeValue::List(items))
                }
                (AttributeValue::Object(map), CompiledExprIndexType::String(key)) => {
                    map.get(key).cloned().or_else(|| {
                        let key_str = interner.resolve(*key)?;
                        let key_interned = interner.intern(&key_str);
                        map.get(&key_interned).cloned()
                    })
                }
                _ => None,
            }
        }
    }
}
