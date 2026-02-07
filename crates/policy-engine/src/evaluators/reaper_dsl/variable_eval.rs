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

    // Get the attribute from the variable (if it's an object)
    let attr_val = match var_val {
        AttributeValue::Object(obj) => obj.get(&attribute).cloned(),
        _ => None,
    };

    match (attr_val.as_ref(), value) {
        (Some(AttributeValue::String(s)), CompiledLiteralValue::String(expected)) => {
            *s == *expected
        }
        (Some(AttributeValue::Int(i)), CompiledLiteralValue::Int(expected)) => *i == *expected,
        (Some(AttributeValue::Bool(b)), CompiledLiteralValue::Bool(expected)) => *b == *expected,
        _ => false,
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

    // Get the attribute from the variable (if it's an object)
    let attr_val = match var_val {
        AttributeValue::Object(obj) => obj.get(&attribute).cloned(),
        _ => None,
    };

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

    match var_val {
        AttributeValue::Object(obj) => {
            matches!(obj.get(&attribute), None | Some(AttributeValue::Null))
        }
        _ => true, // Not an object, attribute doesn't exist
    }
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

    match var_val {
        AttributeValue::Object(obj) => {
            !matches!(obj.get(&attribute), None | Some(AttributeValue::Null))
        }
        _ => false,
    }
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

    // Get the attribute from the variable
    let attr_val = match var_val {
        AttributeValue::Object(obj) => obj.get(&attribute),
        _ => None,
    };

    // Check if it's a collection containing the substring
    match attr_val {
        Some(AttributeValue::List(items)) => items
            .iter()
            .any(|item| matches!(item, AttributeValue::String(s) if *s == substring)),
        Some(AttributeValue::Set(items)) => items.contains(&AttributeValue::String(substring)),
        Some(AttributeValue::String(s)) => {
            // String contains check
            let str_val = match interner.resolve(*s) {
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
}
