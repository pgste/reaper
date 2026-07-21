//! Variable evaluation module.
//!
//! This module handles evaluation of variable-related conditions:
//! - Variable comparisons with literals
//! - Variable null checks
//! - Variable type checks
//! - Variable method operations
//! - Variable attribute operations
//!
//! ## Performance Characteristics
//! - String interning ensures O(1) variable name lookup
//! - HashMap-based variable storage with FxHashMap

use crate::data::{AttributeValue, InternedString, StringInterner};
use std::collections::HashMap;

use super::types::{AttrCompareOp, CompiledLiteralValue, VariableCollectionMethod};

/// Resolve `var.attr` where `attr` may be a DOTTED path ("change.after.acl")
/// — the variable-domain mirror of `entity_helpers::get_nested_attr`, with
/// the same two-step strategy: try the whole attribute as a single key
/// first (zero resolve on the common flat case), then split on '.' and
/// navigate nested objects. Added in R4-01 B.2a: comprehension filters over
/// input-document elements are dotted-path-heavy, and the single-key lookup
/// silently never matched them (caught by the input-comprehension
/// differential on its first run).
pub(super) fn get_var_attr_value(
    var_val: &AttributeValue,
    attribute: InternedString,
    interner: &StringInterner,
) -> Option<AttributeValue> {
    let AttributeValue::Object(obj) = var_val else {
        return None;
    };
    if let Some(v) = obj.get(&attribute) {
        return Some(v.clone());
    }
    let attr_name = interner.resolve(attribute)?;
    if !attr_name.contains('.') {
        return None;
    }
    let parts: Vec<&str> = attr_name.split('.').collect();
    // Segment keys are bounded schema vocabulary — pinned interning, the
    // same discipline as get_nested_attr.
    let first = interner.intern(parts[0]);
    let mut current = obj.get(&first)?.clone();
    for part in &parts[1..] {
        match current {
            AttributeValue::Object(ref map) => {
                let key = interner.intern(part);
                current = map.get(&key)?.clone();
            }
            _ => return None,
        }
    }
    Some(current)
}

/// Evaluate variable equals literal: var == "value"
#[inline]
pub fn eval_variable_equals_literal(
    variable: InternedString,
    value: &CompiledLiteralValue,
    variables: &HashMap<String, AttributeValue>,
    interner: &StringInterner,
) -> bool {
    let var_name = match interner.resolve(variable) {
        Some(name) => name,
        None => return false,
    };

    let var_val = match variables.get(&*var_name) {
        Some(val) => val,
        None => return false,
    };

    match (var_val, value) {
        (AttributeValue::String(s), CompiledLiteralValue::String(expected)) => *s == *expected,
        (AttributeValue::Int(i), CompiledLiteralValue::Int(expected)) => *i == *expected,
        (AttributeValue::Bool(b), CompiledLiteralValue::Bool(expected)) => *b == *expected,
        _ => false,
    }
}

/// Evaluate variable not-equals literal: var != "value".
///
/// NULL SEMANTICS: an UNBOUND variable fails the guard — `!=` is only
/// satisfied by a bound value that differs. (Not(VariableEqualsLiteral)
/// would let unbound variables pass every `!=` filter: fail-open.)
/// A bound value of a different type differs by definition (matches the
/// AST's !values_equal semantics for present values).
#[inline]
pub fn eval_variable_not_equals_literal(
    variable: InternedString,
    value: &CompiledLiteralValue,
    variables: &HashMap<String, AttributeValue>,
    interner: &StringInterner,
) -> bool {
    let var_name = match interner.resolve(variable) {
        Some(name) => name,
        None => return false,
    };

    let var_val = match variables.get(&*var_name) {
        Some(val) => val,
        None => return false,
    };

    match (var_val, value) {
        (AttributeValue::String(s), CompiledLiteralValue::String(expected)) => *s != *expected,
        (AttributeValue::Int(i), CompiledLiteralValue::Int(expected)) => *i != *expected,
        (AttributeValue::Bool(b), CompiledLiteralValue::Bool(expected)) => *b != *expected,
        // Bound but different type: the values necessarily differ.
        _ => true,
    }
}

/// Evaluate variable comparison: var >= N, var > N, etc.
#[inline]
pub fn eval_variable_compare(
    variable: InternedString,
    op: &AttrCompareOp,
    value: &CompiledLiteralValue,
    variables: &HashMap<String, AttributeValue>,
    interner: &StringInterner,
) -> bool {
    let var_name = match interner.resolve(variable) {
        Some(name) => name,
        None => return false,
    };

    let var_val = match variables.get(&*var_name) {
        Some(val) => val,
        None => return false,
    };

    match (var_val, value, op) {
        // Integer comparisons
        (
            AttributeValue::Int(i),
            CompiledLiteralValue::Int(expected),
            AttrCompareOp::GreaterEqual,
        ) => *i >= *expected,
        (AttributeValue::Int(i), CompiledLiteralValue::Int(expected), AttrCompareOp::Greater) => {
            *i > *expected
        }
        (AttributeValue::Int(i), CompiledLiteralValue::Int(expected), AttrCompareOp::LessEqual) => {
            *i <= *expected
        }
        (AttributeValue::Int(i), CompiledLiteralValue::Int(expected), AttrCompareOp::Less) => {
            *i < *expected
        }
        (AttributeValue::Int(i), CompiledLiteralValue::Int(expected), AttrCompareOp::Equal) => {
            *i == *expected
        }
        (AttributeValue::Int(i), CompiledLiteralValue::Int(expected), AttrCompareOp::NotEqual) => {
            *i != *expected
        }
        // Float comparisons
        (
            AttributeValue::Float(f),
            CompiledLiteralValue::Int(expected),
            AttrCompareOp::GreaterEqual,
        ) => *f >= (*expected as f64),
        (AttributeValue::Float(f), CompiledLiteralValue::Int(expected), AttrCompareOp::Greater) => {
            *f > (*expected as f64)
        }
        (
            AttributeValue::Float(f),
            CompiledLiteralValue::Int(expected),
            AttrCompareOp::LessEqual,
        ) => *f <= (*expected as f64),
        (AttributeValue::Float(f), CompiledLiteralValue::Int(expected), AttrCompareOp::Less) => {
            *f < (*expected as f64)
        }
        _ => false,
    }
}

/// Evaluate variable is null check
#[inline]
pub fn eval_variable_is_null(
    variable: InternedString,
    variables: &HashMap<String, AttributeValue>,
    interner: &StringInterner,
) -> bool {
    match interner.resolve(variable) {
        Some(var_name) => matches!(variables.get(&*var_name), None | Some(AttributeValue::Null)),
        None => true, // Can't resolve variable name, treat as null
    }
}

/// Evaluate variable is not null check
#[inline]
pub fn eval_variable_is_not_null(
    variable: InternedString,
    variables: &HashMap<String, AttributeValue>,
    interner: &StringInterner,
) -> bool {
    match interner.resolve(variable) {
        Some(var_name) => !matches!(variables.get(&*var_name), None | Some(AttributeValue::Null)),
        None => false,
    }
}

/// Evaluate variable membership test: "value" in var
#[inline]
pub fn eval_variable_membership_test(
    value: &CompiledLiteralValue,
    variable: InternedString,
    variables: &HashMap<String, AttributeValue>,
    interner: &StringInterner,
) -> bool {
    let var_name = match interner.resolve(variable) {
        Some(name) => name,
        None => return false,
    };

    let collection = match variables.get(&*var_name) {
        Some(val) => val,
        None => return false,
    };

    match (collection, value) {
        (AttributeValue::List(items), CompiledLiteralValue::String(s)) => items
            .iter()
            .any(|item| matches!(item, AttributeValue::String(v) if *v == *s)),
        (AttributeValue::Set(items), CompiledLiteralValue::String(s)) => {
            items.contains(&AttributeValue::String(*s))
        }
        (AttributeValue::List(items), CompiledLiteralValue::Int(i)) => items
            .iter()
            .any(|item| matches!(item, AttributeValue::Int(v) if *v == *i)),
        (AttributeValue::Set(items), CompiledLiteralValue::Int(i)) => {
            items.contains(&AttributeValue::Int(*i))
        }
        _ => false,
    }
}

/// Evaluate variable is string type check
#[inline]
pub fn eval_variable_is_string(
    variable: InternedString,
    variables: &HashMap<String, AttributeValue>,
    interner: &StringInterner,
) -> bool {
    match interner.resolve(variable) {
        Some(var_name) => matches!(variables.get(&*var_name), Some(AttributeValue::String(_))),
        None => false,
    }
}

/// Evaluate variable is number type check
#[inline]
pub fn eval_variable_is_number(
    variable: InternedString,
    variables: &HashMap<String, AttributeValue>,
    interner: &StringInterner,
) -> bool {
    match interner.resolve(variable) {
        Some(var_name) => matches!(
            variables.get(&*var_name),
            Some(AttributeValue::Int(_)) | Some(AttributeValue::Float(_))
        ),
        None => false,
    }
}

/// Evaluate variable is bool type check
#[inline]
pub fn eval_variable_is_bool(
    variable: InternedString,
    variables: &HashMap<String, AttributeValue>,
    interner: &StringInterner,
) -> bool {
    match interner.resolve(variable) {
        Some(var_name) => matches!(variables.get(&*var_name), Some(AttributeValue::Bool(_))),
        None => false,
    }
}

/// Evaluate variable is truthy (non-null, non-false, non-zero, non-empty)
#[inline]
pub fn eval_variable_is_truthy(
    variable: InternedString,
    variables: &HashMap<String, AttributeValue>,
    interner: &StringInterner,
) -> bool {
    match interner.resolve(variable) {
        Some(var_name) => match variables.get(&*var_name) {
            Some(AttributeValue::Bool(b)) => *b,
            Some(AttributeValue::Null) => false,
            Some(AttributeValue::Int(i)) => *i != 0,
            Some(AttributeValue::Float(f)) => *f != 0.0,
            Some(AttributeValue::String(s)) => {
                // Non-empty string is truthy
                interner.resolve(*s).map(|r| !r.is_empty()).unwrap_or(false)
            }
            Some(AttributeValue::List(items)) => !items.is_empty(),
            Some(AttributeValue::Set(items)) => !items.is_empty(),
            Some(AttributeValue::Object(map)) => !map.is_empty(),
            None => false, // Variable not found = falsy
        },
        None => false,
    }
}

/// Evaluate variable equals variable: var1 == var2
#[inline]
pub fn eval_variable_equals_variable(
    left: InternedString,
    right: InternedString,
    variables: &HashMap<String, AttributeValue>,
    interner: &StringInterner,
) -> bool {
    let left_name = match interner.resolve(left) {
        Some(name) => name,
        None => return false,
    };
    let right_name = match interner.resolve(right) {
        Some(name) => name,
        None => return false,
    };

    match (variables.get(&*left_name), variables.get(&*right_name)) {
        (Some(l), Some(r)) => l == r,
        _ => false,
    }
}

/// Evaluate variable not equals variable: var1 != var2
#[inline]
pub fn eval_variable_not_equals_variable(
    left: InternedString,
    right: InternedString,
    variables: &HashMap<String, AttributeValue>,
    interner: &StringInterner,
) -> bool {
    let left_name = match interner.resolve(left) {
        Some(name) => name,
        None => return false,
    };
    let right_name = match interner.resolve(right) {
        Some(name) => name,
        None => return false,
    };

    match (variables.get(&*left_name), variables.get(&*right_name)) {
        (Some(l), Some(r)) => l != r,
        _ => true, // If either is missing, they're not equal
    }
}

/// Evaluate variable method with literal array: var.intersection(["a", "b"])
#[inline]
pub fn eval_variable_method_with_literal_array(
    variable: InternedString,
    method: &VariableCollectionMethod,
    values: &[InternedString],
    variables: &mut HashMap<String, AttributeValue>,
    interner: &StringInterner,
) -> bool {
    let var_name = match interner.resolve(variable) {
        Some(name) => name.to_string(),
        None => return false,
    };

    let var_val = match variables.get(&var_name) {
        Some(val) => val.clone(),
        None => return false,
    };

    // Get the variable as a set of strings
    let var_set: std::collections::HashSet<InternedString> = match &var_val {
        AttributeValue::List(items) => items
            .iter()
            .filter_map(|item| {
                if let AttributeValue::String(s) = item {
                    Some(*s)
                } else {
                    None
                }
            })
            .collect(),
        AttributeValue::Set(items) => items
            .iter()
            .filter_map(|item| {
                if let AttributeValue::String(s) = item {
                    Some(*s)
                } else {
                    None
                }
            })
            .collect(),
        _ => return false,
    };

    // Get the literal values as a set
    let literal_set: std::collections::HashSet<InternedString> = values.iter().copied().collect();

    // Apply the method and collect result as a Vec
    let result: Vec<AttributeValue> = match method {
        VariableCollectionMethod::Intersection => var_set
            .intersection(&literal_set)
            .map(|s| AttributeValue::String(*s))
            .collect(),
        VariableCollectionMethod::Union => var_set
            .union(&literal_set)
            .map(|s| AttributeValue::String(*s))
            .collect(),
        VariableCollectionMethod::Difference => var_set
            .difference(&literal_set)
            .map(|s| AttributeValue::String(*s))
            .collect(),
    };

    // Store result back as a list
    variables.insert(var_name, AttributeValue::List(result.clone()));
    // Return true if result is non-empty (for use in conditions)
    !result.is_empty()
}

/// Evaluate variable attribute equals literal: var.attr == "value"
#[inline]
pub fn eval_variable_attr_equals_literal(
    variable: InternedString,
    attribute: InternedString,
    value: &CompiledLiteralValue,
    variables: &HashMap<String, AttributeValue>,
    interner: &StringInterner,
) -> bool {
    let var_name = match interner.resolve(variable) {
        Some(name) => name,
        None => return false,
    };

    let var_val = match variables.get(&*var_name) {
        Some(val) => val,
        None => return false,
    };

    // Get the attribute from the variable (dotted paths navigate)
    let attr_val = get_var_attr_value(var_val, attribute, interner);

    match (attr_val.as_ref(), value) {
        (Some(AttributeValue::String(s)), CompiledLiteralValue::String(expected)) => {
            *s == *expected
        }
        (Some(AttributeValue::Int(i)), CompiledLiteralValue::Int(expected)) => *i == *expected,
        (Some(AttributeValue::Bool(b)), CompiledLiteralValue::Bool(expected)) => *b == *expected,
        _ => false,
    }
}

/// Evaluate variable attribute not-equals literal: var.attr != "value".
///
/// NULL SEMANTICS: a MISSING attribute (or unbound/non-object variable)
/// fails the guard — absence never satisfies `!=` (fail closed). A present
/// value of a different type differs by definition.
#[inline]
pub fn eval_variable_attr_not_equals_literal(
    variable: InternedString,
    attribute: InternedString,
    value: &CompiledLiteralValue,
    variables: &HashMap<String, AttributeValue>,
    interner: &StringInterner,
) -> bool {
    let var_name = match interner.resolve(variable) {
        Some(name) => name,
        None => return false,
    };

    let var_val = match variables.get(&*var_name) {
        Some(val) => val,
        None => return false,
    };

    let attr_val = get_var_attr_value(var_val, attribute, interner);

    match (attr_val.as_ref(), value) {
        (None, _) => false,
        (Some(AttributeValue::String(s)), CompiledLiteralValue::String(expected)) => {
            *s != *expected
        }
        (Some(AttributeValue::Int(i)), CompiledLiteralValue::Int(expected)) => *i != *expected,
        (Some(AttributeValue::Bool(b)), CompiledLiteralValue::Bool(expected)) => *b != *expected,
        // Present but different type: the values necessarily differ.
        _ => true,
    }
}

/// Evaluate variable attribute compare: var.attr >= N
#[inline]
pub fn eval_variable_attr_compare(
    variable: InternedString,
    attribute: InternedString,
    op: &AttrCompareOp,
    value: &CompiledLiteralValue,
    variables: &HashMap<String, AttributeValue>,
    interner: &StringInterner,
) -> bool {
    let var_name = match interner.resolve(variable) {
        Some(name) => name,
        None => return false,
    };

    let var_val = match variables.get(&*var_name) {
        Some(val) => val,
        None => return false,
    };

    // Get the attribute from the variable (dotted paths navigate)
    let attr_val = get_var_attr_value(var_val, attribute, interner);

    match (attr_val.as_ref(), value, op) {
        // Integer comparisons
        (
            Some(AttributeValue::Int(i)),
            CompiledLiteralValue::Int(expected),
            AttrCompareOp::GreaterEqual,
        ) => *i >= *expected,
        (
            Some(AttributeValue::Int(i)),
            CompiledLiteralValue::Int(expected),
            AttrCompareOp::Greater,
        ) => *i > *expected,
        (
            Some(AttributeValue::Int(i)),
            CompiledLiteralValue::Int(expected),
            AttrCompareOp::LessEqual,
        ) => *i <= *expected,
        (
            Some(AttributeValue::Int(i)),
            CompiledLiteralValue::Int(expected),
            AttrCompareOp::Less,
        ) => *i < *expected,
        (
            Some(AttributeValue::Int(i)),
            CompiledLiteralValue::Int(expected),
            AttrCompareOp::Equal,
        ) => *i == *expected,
        (
            Some(AttributeValue::Int(i)),
            CompiledLiteralValue::Int(expected),
            AttrCompareOp::NotEqual,
        ) => *i != *expected,
        // String comparisons for equality
        (
            Some(AttributeValue::String(s)),
            CompiledLiteralValue::String(expected),
            AttrCompareOp::Equal,
        ) => *s == *expected,
        (
            Some(AttributeValue::String(s)),
            CompiledLiteralValue::String(expected),
            AttrCompareOp::NotEqual,
        ) => *s != *expected,
        _ => false,
    }
}

/// Evaluate variable attribute equals null: var.attr == null
#[inline]
pub fn eval_variable_attr_equals_null(
    variable: InternedString,
    attribute: InternedString,
    variables: &HashMap<String, AttributeValue>,
    interner: &StringInterner,
) -> bool {
    let var_name = match interner.resolve(variable) {
        Some(name) => name,
        None => return true, // Can't resolve, treat as null
    };

    let var_val = match variables.get(&*var_name) {
        Some(val) => val,
        None => return true,
    };

    matches!(
        get_var_attr_value(var_val, attribute, interner),
        None | Some(AttributeValue::Null)
    )
}

/// Evaluate variable attribute not equals null: var.attr != null
#[inline]
pub fn eval_variable_attr_not_equals_null(
    variable: InternedString,
    attribute: InternedString,
    variables: &HashMap<String, AttributeValue>,
    interner: &StringInterner,
) -> bool {
    let var_name = match interner.resolve(variable) {
        Some(name) => name,
        None => return false,
    };

    let var_val = match variables.get(&*var_name) {
        Some(val) => val,
        None => return false,
    };

    !matches!(
        get_var_attr_value(var_val, attribute, interner),
        None | Some(AttributeValue::Null)
    )
}

/// Evaluate variable attribute contains: var.attr.contains("value")
#[inline]
pub fn eval_variable_attr_contains(
    variable: InternedString,
    attribute: InternedString,
    substring: InternedString,
    variables: &HashMap<String, AttributeValue>,
    interner: &StringInterner,
) -> bool {
    let var_name = match interner.resolve(variable) {
        Some(name) => name,
        None => return false,
    };

    let var_val = match variables.get(&*var_name) {
        Some(val) => val,
        None => return false,
    };

    // Get the attribute from the variable (dotted paths navigate)
    let attr_val = get_var_attr_value(var_val, attribute, interner);

    // Check if it's a collection containing the substring
    match attr_val {
        Some(AttributeValue::List(items)) => items
            .iter()
            .any(|item| matches!(item, AttributeValue::String(s) if *s == substring)),
        Some(AttributeValue::Set(items)) => items.contains(&AttributeValue::String(substring)),
        Some(AttributeValue::String(s)) => {
            // String contains check
            let str_val = match interner.resolve(s) {
                Some(v) => v,
                None => return false,
            };
            let substr = match interner.resolve(substring) {
                Some(v) => v,
                None => return false,
            };
            str_val.contains(&*substr)
        }
        _ => false,
    }
}

use super::types::{VariableMethod, VariableStringTransform};

/// Evaluate variable method comparison: var.count() > N, var.sum() >= M
#[inline]
pub fn eval_variable_method_compare(
    variable: InternedString,
    method: &VariableMethod,
    op: &AttrCompareOp,
    value: &CompiledLiteralValue,
    variables: &HashMap<String, AttributeValue>,
    interner: &StringInterner,
) -> bool {
    let var_name = match interner.resolve(variable) {
        Some(name) => name,
        None => return false,
    };

    let var_val = match variables.get(&*var_name) {
        Some(val) => val,
        None => return false,
    };

    // Apply the method to get the actual value
    let method_result: Option<i64> = match (var_val, method) {
        (AttributeValue::List(items), VariableMethod::Count) => Some(items.len() as i64),
        (AttributeValue::Set(items), VariableMethod::Count) => Some(items.len() as i64),
        (AttributeValue::List(items), VariableMethod::Sum) => {
            let sum: i64 = items
                .iter()
                .filter_map(|item| match item {
                    AttributeValue::Int(i) => Some(*i),
                    AttributeValue::Float(f) => Some(*f as i64),
                    _ => None,
                })
                .sum();
            Some(sum)
        }
        (AttributeValue::List(items), VariableMethod::Max) => items
            .iter()
            .filter_map(|item| match item {
                AttributeValue::Int(i) => Some(*i),
                AttributeValue::Float(f) => Some(*f as i64),
                _ => None,
            })
            .max(),
        (AttributeValue::List(items), VariableMethod::Min) => items
            .iter()
            .filter_map(|item| match item {
                AttributeValue::Int(i) => Some(*i),
                AttributeValue::Float(f) => Some(*f as i64),
                _ => None,
            })
            .min(),
        _ => None,
    };

    if let (Some(result), CompiledLiteralValue::Int(expected)) = (method_result, value) {
        match op {
            AttrCompareOp::GreaterEqual => result >= *expected,
            AttrCompareOp::Greater => result > *expected,
            AttrCompareOp::LessEqual => result <= *expected,
            AttrCompareOp::Less => result < *expected,
            AttrCompareOp::Equal => result == *expected,
            AttrCompareOp::NotEqual => result != *expected,
        }
    } else {
        false
    }
}

/// Evaluate chained variable method comparison: t.trim().count() > 0
#[inline]
pub fn eval_variable_chained_method_compare(
    variable: InternedString,
    transform_method: &VariableStringTransform,
    compare_method: &VariableMethod,
    op: &AttrCompareOp,
    value: &CompiledLiteralValue,
    variables: &HashMap<String, AttributeValue>,
    interner: &StringInterner,
) -> bool {
    let var_name = match interner.resolve(variable) {
        Some(name) => name,
        None => return false,
    };

    let var_val = match variables.get(&*var_name) {
        Some(val) => val,
        None => return false,
    };

    // Step 1: Apply transform method to get a string
    let transformed: Option<String> = match var_val {
        AttributeValue::String(s) => {
            interner
                .resolve(*s)
                .map(|string_val| match transform_method {
                    VariableStringTransform::Trim => string_val.trim().to_string(),
                    VariableStringTransform::Lower => string_val.to_lowercase(),
                    VariableStringTransform::Upper => string_val.to_uppercase(),
                })
        }
        _ => None,
    };

    // Step 2: Apply compare method to get numeric result
    let method_result: Option<i64> = transformed.and_then(|s| match compare_method {
        VariableMethod::Count => Some(s.chars().count() as i64),
        _ => None, // Sum/Max/Min don't apply to strings
    });

    // Step 3: Compare
    if let (Some(result), CompiledLiteralValue::Int(expected)) = (method_result, value) {
        match op {
            AttrCompareOp::GreaterEqual => result >= *expected,
            AttrCompareOp::Greater => result > *expected,
            AttrCompareOp::LessEqual => result <= *expected,
            AttrCompareOp::Less => result < *expected,
            AttrCompareOp::Equal => result == *expected,
            AttrCompareOp::NotEqual => result != *expected,
        }
    } else {
        false
    }
}

/// Evaluate `var.attr.startswith("p")` / `.endswith("s")` /
/// `.contains("c")` (R4-01 B.2b). The attribute may be a dotted path
/// (navigates via [`get_var_attr_value`]); a missing/unbound/non-string
/// value fails closed — mirroring the interpreter, where a method on a
/// Null receiver yields Null (non-match) and string methods only apply to
/// strings.
#[inline]
pub fn eval_variable_attr_string_op(
    variable: InternedString,
    attribute: InternedString,
    op: &super::types::StringOp,
    value: &str,
    variables: &HashMap<String, AttributeValue>,
    interner: &StringInterner,
) -> bool {
    let var_name = match interner.resolve(variable) {
        Some(name) => name,
        None => return false,
    };
    let var_val = match variables.get(&*var_name) {
        Some(val) => val,
        None => return false,
    };
    let attr_val = get_var_attr_value(var_val, attribute, interner);
    let Some(AttributeValue::String(s)) = attr_val else {
        return false;
    };
    let Some(text) = interner.resolve(s) else {
        return false;
    };
    match op {
        super::types::StringOp::StartsWith => text.starts_with(value),
        super::types::StringOp::EndsWith => text.ends_with(value),
        super::types::StringOp::Contains => text.contains(value),
        // lower()/upper() forms are separate lowerings; this shape never
        // carries them (the compiler only emits the three above).
        super::types::StringOp::LowerEquals
        | super::types::StringOp::UpperEquals
        | super::types::StringOp::LowerNotEquals
        | super::types::StringOp::UpperNotEquals => false,
    }
}

/// Evaluate `"lit" in var.attr` (R4-01 B.2b). The attribute may be a dotted
/// path (navigates via [`get_var_attr_value`]); missing/unbound/non-
/// collection values fail closed. Same literal-matching semantics as
/// [`eval_variable_membership_test`].
#[inline]
pub fn eval_variable_attr_membership_test(
    value: &CompiledLiteralValue,
    variable: InternedString,
    attribute: InternedString,
    variables: &HashMap<String, AttributeValue>,
    interner: &StringInterner,
) -> bool {
    let var_name = match interner.resolve(variable) {
        Some(name) => name,
        None => return false,
    };
    let var_val = match variables.get(&*var_name) {
        Some(val) => val,
        None => return false,
    };
    let collection = match get_var_attr_value(var_val, attribute, interner) {
        Some(v) => v,
        None => return false,
    };
    match (&collection, value) {
        (AttributeValue::List(items), CompiledLiteralValue::String(s)) => items
            .iter()
            .any(|item| matches!(item, AttributeValue::String(v) if *v == *s)),
        (AttributeValue::Set(items), CompiledLiteralValue::String(s)) => {
            items.contains(&AttributeValue::String(*s))
        }
        (AttributeValue::List(items), CompiledLiteralValue::Int(i)) => items
            .iter()
            .any(|item| matches!(item, AttributeValue::Int(v) if *v == *i)),
        (AttributeValue::Set(items), CompiledLiteralValue::Int(i)) => {
            items.contains(&AttributeValue::Int(*i))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::StringInterner;

    fn setup_interner() -> StringInterner {
        StringInterner::new()
    }

    #[test]
    fn test_eval_variable_equals_literal_string() {
        let interner = setup_interner();
        let var_key = interner.intern("myvar");
        let value_key = interner.intern("hello");

        let mut variables = HashMap::new();
        variables.insert("myvar".to_string(), AttributeValue::String(value_key));

        let value = CompiledLiteralValue::String(value_key);
        assert!(eval_variable_equals_literal(
            var_key, &value, &variables, &interner
        ));
    }

    #[test]
    fn test_eval_variable_is_null() {
        let interner = setup_interner();
        let var_key = interner.intern("missing");

        let variables = HashMap::new();
        assert!(eval_variable_is_null(var_key, &variables, &interner));
    }

    #[test]
    fn test_eval_variable_is_not_null() {
        let interner = setup_interner();
        let var_key = interner.intern("exists");
        let value_key = interner.intern("value");

        let mut variables = HashMap::new();
        variables.insert("exists".to_string(), AttributeValue::String(value_key));

        assert!(eval_variable_is_not_null(var_key, &variables, &interner));
    }

    #[test]
    fn test_eval_variable_is_truthy() {
        let interner = setup_interner();
        let var_true = interner.intern("is_true");
        let var_false = interner.intern("is_false");
        let var_null = interner.intern("is_null");

        let mut variables = HashMap::new();
        variables.insert("is_true".to_string(), AttributeValue::Bool(true));
        variables.insert("is_false".to_string(), AttributeValue::Bool(false));
        variables.insert("is_null".to_string(), AttributeValue::Null);

        assert!(eval_variable_is_truthy(var_true, &variables, &interner));
        assert!(!eval_variable_is_truthy(var_false, &variables, &interner));
        assert!(!eval_variable_is_truthy(var_null, &variables, &interner));
    }

    #[test]
    fn test_eval_variable_compare() {
        let interner = setup_interner();
        let var_key = interner.intern("count");

        let mut variables = HashMap::new();
        variables.insert("count".to_string(), AttributeValue::Int(10));

        let value = CompiledLiteralValue::Int(5);
        assert!(eval_variable_compare(
            var_key,
            &AttrCompareOp::GreaterEqual,
            &value,
            &variables,
            &interner
        ));
        assert!(eval_variable_compare(
            var_key,
            &AttrCompareOp::Greater,
            &value,
            &variables,
            &interner
        ));
        assert!(!eval_variable_compare(
            var_key,
            &AttrCompareOp::Less,
            &value,
            &variables,
            &interner
        ));
    }

    #[test]
    fn test_eval_variable_attr_equals_literal_bool() {
        // Test case: r.active == true (comprehension filter pattern)
        let interner = setup_interner();

        // Create an object with "active": true attribute
        let active_key = interner.intern("active");
        let id_key = interner.intern("id");
        let rec1 = interner.intern("rec1");

        let mut obj_map: std::collections::HashMap<crate::data::InternedString, AttributeValue> =
            std::collections::HashMap::new();
        obj_map.insert(active_key, AttributeValue::Bool(true));
        obj_map.insert(id_key, AttributeValue::String(rec1));

        let record = AttributeValue::Object(obj_map);

        // Store the record in variables with name "r"
        let var_r = interner.intern("r");
        let mut variables = HashMap::new();
        variables.insert("r".to_string(), record);

        // Test: r.active == true should return true
        let value_true = CompiledLiteralValue::Bool(true);
        let result = eval_variable_attr_equals_literal(
            var_r,
            active_key,
            &value_true,
            &variables,
            &interner,
        );
        assert!(result, "r.active == true should be true");

        // Test: r.active == false should return false
        let value_false = CompiledLiteralValue::Bool(false);
        let result = eval_variable_attr_equals_literal(
            var_r,
            active_key,
            &value_false,
            &variables,
            &interner,
        );
        assert!(!result, "r.active == false should be false");
    }

    #[test]
    fn test_eval_variable_attr_equals_literal_string() {
        // Test case: r.id == "rec1" (string comparison)
        let interner = setup_interner();

        let id_key = interner.intern("id");
        let rec1_val = interner.intern("rec1");
        let rec2_val = interner.intern("rec2");

        let mut obj_map: std::collections::HashMap<crate::data::InternedString, AttributeValue> =
            std::collections::HashMap::new();
        obj_map.insert(id_key, AttributeValue::String(rec1_val));

        let record = AttributeValue::Object(obj_map);

        let var_r = interner.intern("r");
        let mut variables = HashMap::new();
        variables.insert("r".to_string(), record);

        // Test: r.id == "rec1" should return true
        let value_rec1 = CompiledLiteralValue::String(rec1_val);
        let result =
            eval_variable_attr_equals_literal(var_r, id_key, &value_rec1, &variables, &interner);
        assert!(result, "r.id == 'rec1' should be true");

        // Test: r.id == "rec2" should return false
        let value_rec2 = CompiledLiteralValue::String(rec2_val);
        let result =
            eval_variable_attr_equals_literal(var_r, id_key, &value_rec2, &variables, &interner);
        assert!(!result, "r.id == 'rec2' should be false");
    }

    #[test]
    fn test_eval_variable_attr_equals_literal_int() {
        // Test case: r.value == 100 (int comparison)
        let interner = setup_interner();

        let value_key = interner.intern("value");

        let mut obj_map: std::collections::HashMap<crate::data::InternedString, AttributeValue> =
            std::collections::HashMap::new();
        obj_map.insert(value_key, AttributeValue::Int(100));

        let record = AttributeValue::Object(obj_map);

        let var_r = interner.intern("r");
        let mut variables = HashMap::new();
        variables.insert("r".to_string(), record);

        // Test: r.value == 100 should return true
        let value_100 = CompiledLiteralValue::Int(100);
        let result =
            eval_variable_attr_equals_literal(var_r, value_key, &value_100, &variables, &interner);
        assert!(result, "r.value == 100 should be true");

        // Test: r.value == 200 should return false
        let value_200 = CompiledLiteralValue::Int(200);
        let result =
            eval_variable_attr_equals_literal(var_r, value_key, &value_200, &variables, &interner);
        assert!(!result, "r.value == 200 should be false");
    }

    #[test]
    fn variable_not_equals_literal_fails_closed_when_unbound() {
        let interner = setup_interner();
        let var = interner.intern("role");
        let variables: HashMap<String, AttributeValue> = HashMap::new();

        // Unbound variable: `role != "guest"` must FAIL, not pass.
        let guest = CompiledLiteralValue::String(interner.intern("guest"));
        assert!(!eval_variable_not_equals_literal(
            var, &guest, &variables, &interner
        ));

        // Bound and different → true; bound and equal → false.
        let mut bound = HashMap::new();
        let admin_val = interner.intern("admin");
        bound.insert("role".to_string(), AttributeValue::String(admin_val));
        assert!(eval_variable_not_equals_literal(
            var, &guest, &bound, &interner
        ));
        let admin = CompiledLiteralValue::String(interner.intern("admin"));
        assert!(!eval_variable_not_equals_literal(
            var, &admin, &bound, &interner
        ));
    }

    #[test]
    fn variable_attr_not_equals_literal_fails_closed_when_missing() {
        let interner = setup_interner();
        let value_key = interner.intern("value");
        let missing_key = interner.intern("nonexistent");
        let var_r = interner.intern("r");

        let mut obj_map: std::collections::HashMap<crate::data::InternedString, AttributeValue> =
            std::collections::HashMap::new();
        obj_map.insert(value_key, AttributeValue::Int(100));
        let mut variables = HashMap::new();
        variables.insert("r".to_string(), AttributeValue::Object(obj_map));

        let hundred = CompiledLiteralValue::Int(100);
        let two_hundred = CompiledLiteralValue::Int(200);

        // Missing attribute: `r.nonexistent != 100` must FAIL (fail closed).
        assert!(!eval_variable_attr_not_equals_literal(
            var_r,
            missing_key,
            &hundred,
            &variables,
            &interner
        ));
        // Present and different → true; present and equal → false.
        assert!(eval_variable_attr_not_equals_literal(
            var_r,
            value_key,
            &two_hundred,
            &variables,
            &interner
        ));
        assert!(!eval_variable_attr_not_equals_literal(
            var_r, value_key, &hundred, &variables, &interner
        ));
        // Unbound variable entirely: fail closed.
        let empty: HashMap<String, AttributeValue> = HashMap::new();
        assert!(!eval_variable_attr_not_equals_literal(
            var_r, value_key, &hundred, &empty, &interner
        ));
    }
}
