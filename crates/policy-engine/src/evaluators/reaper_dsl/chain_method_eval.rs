//! Chained method evaluation.
//!
//! This module handles evaluation of chained method calls like:
//! - `user.name.lower()` - string method chaining
//! - `user.scores.sum()` - collection method chaining
//! - `user.tags.intersection(["a", "b"])` - set operations

use crate::data::{AttributeValue, InternedString, StringInterner};

use super::expr_eval::compare_attribute_values;
use super::types::CompiledChainMethod;

/// Evaluate a chained method on a base value.
pub(super) fn evaluate_chained_method(
    method: &CompiledChainMethod,
    base_value: AttributeValue,
    interner: &StringInterner,
) -> Option<AttributeValue> {
    match method {
        CompiledChainMethod::Lower => {
            if let AttributeValue::String(s) = base_value {
                if let Some(resolved) = interner.resolve(s) {
                    let lower = resolved.to_lowercase();
                    let interned = interner.intern(&lower);
                    return Some(AttributeValue::String(interned));
                }
            }
            None
        }
        CompiledChainMethod::Upper => {
            if let AttributeValue::String(s) = base_value {
                if let Some(resolved) = interner.resolve(s) {
                    let upper = resolved.to_uppercase();
                    let interned = interner.intern(&upper);
                    return Some(AttributeValue::String(interned));
                }
            }
            None
        }
        CompiledChainMethod::Trim => {
            if let AttributeValue::String(s) = base_value {
                if let Some(resolved) = interner.resolve(s) {
                    let trimmed = resolved.trim().to_string();
                    let interned = interner.intern(&trimmed);
                    return Some(AttributeValue::String(interned));
                }
            }
            None
        }
        CompiledChainMethod::Count => match base_value {
            AttributeValue::List(items) => Some(AttributeValue::Int(items.len() as i64)),
            AttributeValue::Set(items) => Some(AttributeValue::Int(items.len() as i64)),
            AttributeValue::Object(map) => Some(AttributeValue::Int(map.len() as i64)),
            AttributeValue::String(s) => interner
                .resolve(s)
                .map(|resolved| AttributeValue::Int(resolved.len() as i64)),
            _ => None,
        },
        CompiledChainMethod::Contains { substring } => {
            if let AttributeValue::String(s) = base_value {
                if let Some(resolved) = interner.resolve(s) {
                    return Some(AttributeValue::Bool(resolved.contains(substring)));
                }
            }
            None
        }
        CompiledChainMethod::Startswith { prefix } => {
            if let AttributeValue::String(s) = base_value {
                if let Some(resolved) = interner.resolve(s) {
                    return Some(AttributeValue::Bool(resolved.starts_with(prefix)));
                }
            }
            None
        }
        CompiledChainMethod::Endswith { suffix } => {
            if let AttributeValue::String(s) = base_value {
                if let Some(resolved) = interner.resolve(s) {
                    return Some(AttributeValue::Bool(resolved.ends_with(suffix)));
                }
            }
            None
        }
        CompiledChainMethod::Sum => match base_value {
            AttributeValue::List(items) => {
                let sum: i64 = items
                    .iter()
                    .filter_map(|item| match item {
                        AttributeValue::Int(i) => Some(*i),
                        AttributeValue::Float(f) => Some(*f as i64),
                        _ => None,
                    })
                    .sum();
                Some(AttributeValue::Int(sum))
            }
            _ => None,
        },
        CompiledChainMethod::Max => match base_value {
            AttributeValue::List(items) => {
                let max = items
                    .iter()
                    .filter_map(|item| match item {
                        AttributeValue::Int(i) => Some(*i),
                        AttributeValue::Float(f) => Some(*f as i64),
                        _ => None,
                    })
                    .max();
                max.map(AttributeValue::Int)
            }
            _ => None,
        },
        CompiledChainMethod::Min => match base_value {
            AttributeValue::List(items) => {
                let min = items
                    .iter()
                    .filter_map(|item| match item {
                        AttributeValue::Int(i) => Some(*i),
                        AttributeValue::Float(f) => Some(*f as i64),
                        _ => None,
                    })
                    .min();
                min.map(AttributeValue::Int)
            }
            _ => None,
        },
        CompiledChainMethod::First => match base_value {
            AttributeValue::List(items) => items.first().cloned(),
            AttributeValue::Set(items) => items.iter().next().cloned(),
            _ => None,
        },
        CompiledChainMethod::Last => match base_value {
            AttributeValue::List(items) => items.last().cloned(),
            AttributeValue::Set(items) => items.iter().last().cloned(),
            _ => None,
        },
        CompiledChainMethod::Reverse => match base_value {
            AttributeValue::List(items) => {
                let mut reversed = items;
                reversed.reverse();
                Some(AttributeValue::List(reversed))
            }
            _ => None,
        },
        CompiledChainMethod::Sort => match base_value {
            AttributeValue::List(items) => {
                let mut sorted = items;
                sorted.sort_by(|a, b| compare_attribute_values(a, b, interner));
                Some(AttributeValue::List(sorted))
            }
            _ => None,
        },
        CompiledChainMethod::Unique => match base_value {
            AttributeValue::List(items) => {
                let mut seen = std::collections::HashSet::new();
                let unique: Vec<AttributeValue> = items
                    .into_iter()
                    .filter(|item| {
                        let key = format!("{:?}", item);
                        seen.insert(key)
                    })
                    .collect();
                Some(AttributeValue::List(unique))
            }
            _ => None,
        },
        CompiledChainMethod::Keys => match base_value {
            AttributeValue::Object(map) => {
                let keys: Vec<AttributeValue> =
                    map.keys().copied().map(AttributeValue::String).collect();
                Some(AttributeValue::List(keys))
            }
            _ => None,
        },
        CompiledChainMethod::Intersection { values } => {
            let literal_set: std::collections::HashSet<InternedString> =
                values.iter().copied().collect();
            let filter_fn = |item: &AttributeValue| -> bool {
                if let AttributeValue::String(s) = item {
                    literal_set.contains(s)
                } else {
                    false
                }
            };
            match base_value {
                AttributeValue::List(items) => {
                    let result: Vec<AttributeValue> =
                        items.iter().filter(|i| filter_fn(i)).cloned().collect();
                    Some(AttributeValue::List(result))
                }
                AttributeValue::Set(items) => {
                    let result: Vec<AttributeValue> =
                        items.iter().filter(|i| filter_fn(i)).cloned().collect();
                    Some(AttributeValue::List(result))
                }
                _ => None,
            }
        }
        CompiledChainMethod::Union { values } => match base_value {
            AttributeValue::List(items) => {
                let mut result: Vec<AttributeValue> = items.clone();
                for v in values {
                    let attr = AttributeValue::String(*v);
                    if !result.contains(&attr) {
                        result.push(attr);
                    }
                }
                Some(AttributeValue::List(result))
            }
            AttributeValue::Set(items) => {
                let mut result: Vec<AttributeValue> = items.iter().cloned().collect();
                for v in values {
                    let attr = AttributeValue::String(*v);
                    if !result.contains(&attr) {
                        result.push(attr);
                    }
                }
                Some(AttributeValue::List(result))
            }
            _ => None,
        },
        CompiledChainMethod::Difference { values } => {
            let literal_set: std::collections::HashSet<InternedString> =
                values.iter().copied().collect();
            let filter_fn = |item: &AttributeValue| -> bool {
                if let AttributeValue::String(s) = item {
                    !literal_set.contains(s)
                } else {
                    true
                }
            };
            match base_value {
                AttributeValue::List(items) => {
                    let result: Vec<AttributeValue> =
                        items.iter().filter(|i| filter_fn(i)).cloned().collect();
                    Some(AttributeValue::List(result))
                }
                AttributeValue::Set(items) => {
                    let result: Vec<AttributeValue> =
                        items.iter().filter(|i| filter_fn(i)).cloned().collect();
                    Some(AttributeValue::List(result))
                }
                _ => None,
            }
        }
    }
}
