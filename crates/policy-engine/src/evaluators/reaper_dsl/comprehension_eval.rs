//! Comprehension evaluation helpers.
//!
//! This module handles comprehension evaluation helpers:
//! - Output extraction from variables
//! - String conversion for object keys
//! - Method calls on output values
//! - Comprehension filter value comparison

use crate::data::{AttributeValue, InternedString, StringInterner};
use std::collections::HashMap;

use super::entity_helpers::get_entity_for_type;
use super::types::{
    CompiledLiteralValue, CompiledOutput, ComprehensionFilterOp, EntityBindings, EntityType,
    OutputMethod,
};

/// Compare compiled values for comprehension filters
#[inline]
pub fn compare_compiled_values(
    field_val: &AttributeValue,
    filter_value: &CompiledLiteralValue,
    filter_op: &ComprehensionFilterOp,
    interner: &StringInterner,
) -> bool {
    match (field_val, filter_value, filter_op) {
        // String equality
        (
            AttributeValue::String(s),
            CompiledLiteralValue::String(expected),
            ComprehensionFilterOp::Equal,
        ) => *s == *expected,
        (
            AttributeValue::String(s),
            CompiledLiteralValue::String(expected),
            ComprehensionFilterOp::NotEqual,
        ) => *s != *expected,

        // Int equality
        (
            AttributeValue::Int(i),
            CompiledLiteralValue::Int(expected),
            ComprehensionFilterOp::Equal,
        ) => *i == *expected,
        (
            AttributeValue::Int(i),
            CompiledLiteralValue::Int(expected),
            ComprehensionFilterOp::NotEqual,
        ) => *i != *expected,
        (
            AttributeValue::Int(i),
            CompiledLiteralValue::Int(expected),
            ComprehensionFilterOp::GreaterThan,
        ) => *i > *expected,
        (
            AttributeValue::Int(i),
            CompiledLiteralValue::Int(expected),
            ComprehensionFilterOp::LessThan,
        ) => *i < *expected,
        (
            AttributeValue::Int(i),
            CompiledLiteralValue::Int(expected),
            ComprehensionFilterOp::GreaterEqual,
        ) => *i >= *expected,
        (
            AttributeValue::Int(i),
            CompiledLiteralValue::Int(expected),
            ComprehensionFilterOp::LessEqual,
        ) => *i <= *expected,

        // Bool equality
        (
            AttributeValue::Bool(b),
            CompiledLiteralValue::Bool(expected),
            ComprehensionFilterOp::Equal,
        ) => *b == *expected,
        (
            AttributeValue::Bool(b),
            CompiledLiteralValue::Bool(expected),
            ComprehensionFilterOp::NotEqual,
        ) => *b != *expected,

        // String contains check
        (
            AttributeValue::String(s),
            CompiledLiteralValue::String(substr),
            ComprehensionFilterOp::Contains,
        ) => {
            if let (Some(resolved), Some(substr_resolved)) =
                (interner.resolve(*s), interner.resolve(*substr))
            {
                resolved.contains(&*substr_resolved)
            } else {
                false
            }
        }

        _ => false,
    }
}

/// Evaluate comprehension count >= threshold
#[inline]
#[allow(clippy::too_many_arguments)]
pub fn eval_comprehension_count_gte(
    entity_type: &EntityType,
    attribute: InternedString,
    filter_attr: &InternedString,
    filter_value: &CompiledLiteralValue,
    filter_op: &ComprehensionFilterOp,
    threshold: usize,
    bindings: EntityBindings<'_>,
    interner: &StringInterner,
) -> bool {
    let entity = match get_entity_for_type(entity_type, bindings) {
        Some(e) => e,
        None => return false,
    };

    match entity.get_attribute(attribute) {
        Some(AttributeValue::List(items)) => {
            let count = items
                .iter()
                .filter(|item| {
                    if let AttributeValue::Object(obj) = item {
                        if let Some(field_val) = obj.get(filter_attr) {
                            compare_compiled_values(field_val, filter_value, filter_op, interner)
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                })
                .count();
            count >= threshold
        }
        _ => false,
    }
}

/// Evaluate comprehension count == threshold
#[inline]
#[allow(clippy::too_many_arguments)]
pub fn eval_comprehension_count_eq(
    entity_type: &EntityType,
    attribute: InternedString,
    filter_attr: &InternedString,
    filter_value: &CompiledLiteralValue,
    filter_op: &ComprehensionFilterOp,
    threshold: usize,
    bindings: EntityBindings<'_>,
    interner: &StringInterner,
) -> bool {
    let entity = match get_entity_for_type(entity_type, bindings) {
        Some(e) => e,
        None => return false,
    };

    match entity.get_attribute(attribute) {
        Some(AttributeValue::List(items)) => {
            let count = items
                .iter()
                .filter(|item| {
                    if let AttributeValue::Object(obj) = item {
                        if let Some(field_val) = obj.get(filter_attr) {
                            compare_compiled_values(field_val, filter_value, filter_op, interner)
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                })
                .count();
            count == threshold
        }
        _ => false,
    }
}

/// Get comprehension output as an AttributeValue.
///
/// This extracts the output value from a comprehension based on the
/// CompiledOutput specification (variable, variable attribute, literal, etc.)
pub fn get_comprehension_output(
    output: &Option<CompiledOutput>,
    variables: &HashMap<String, AttributeValue>,
    interner: &StringInterner,
) -> Option<AttributeValue> {
    let output = output.as_ref()?;
    match output {
        CompiledOutput::Variable(var) => {
            let var_name = interner.resolve(*var)?;
            variables.get(&*var_name).cloned()
        }
        CompiledOutput::VarAttr {
            variable,
            attribute,
        } => {
            let var_name = interner.resolve(*variable)?;
            let var_val = variables.get(&*var_name)?;
            if let AttributeValue::Object(obj) = var_val {
                obj.get(attribute).cloned()
            } else {
                None
            }
        }
        CompiledOutput::Literal(lit) => match lit {
            CompiledLiteralValue::String(s) => Some(AttributeValue::String(*s)),
            CompiledLiteralValue::Int(i) => Some(AttributeValue::Int(*i)),
            CompiledLiteralValue::Bool(b) => Some(AttributeValue::Bool(*b)),
        },
        CompiledOutput::VarMethodCall { variable, method } => {
            let var_name = interner.resolve(*variable)?;
            let var_val = variables.get(&*var_name)?;
            // Apply the method to the variable value
            if let AttributeValue::String(s) = var_val {
                let resolved = interner.resolve(*s)?;
                let transformed = match method {
                    OutputMethod::Lower => resolved.to_lowercase(),
                    OutputMethod::Upper => resolved.to_uppercase(),
                    OutputMethod::Trim => resolved.trim().to_string(),
                };
                let interned = super::intern_transient(interner, &transformed);
                Some(AttributeValue::String(interned))
            } else {
                None
            }
        }
    }
}

/// Get comprehension output as an interned string (for object keys).
///
/// This is used when building object comprehensions where the output
/// needs to be converted to a string key.
pub fn get_comprehension_output_as_string(
    output: &CompiledOutput,
    variables: &HashMap<String, AttributeValue>,
    interner: &StringInterner,
) -> Option<InternedString> {
    match output {
        CompiledOutput::Variable(var) => {
            let var_name = interner.resolve(*var)?;
            if let Some(val) = variables.get(&*var_name) {
                match val {
                    AttributeValue::String(s) => Some(*s),
                    AttributeValue::Int(i) => {
                        // Convert int to string
                        Some(super::intern_transient(interner, &i.to_string()))
                    }
                    _ => None,
                }
            } else {
                None
            }
        }
        CompiledOutput::VarAttr {
            variable,
            attribute,
        } => {
            let var_name = interner.resolve(*variable)?;
            if let Some(AttributeValue::Object(map)) = variables.get(&*var_name) {
                let attr_val = map.get(attribute).or_else(|| {
                    let attr_name = interner.resolve(*attribute)?;
                    let attr_interned = interner.intern(&attr_name);
                    map.get(&attr_interned)
                })?;
                match attr_val {
                    AttributeValue::String(s) => Some(*s),
                    AttributeValue::Int(i) => {
                        Some(super::intern_transient(interner, &i.to_string()))
                    }
                    _ => None,
                }
            } else {
                None
            }
        }
        CompiledOutput::Literal(lit) => match lit {
            CompiledLiteralValue::String(s) => Some(*s),
            CompiledLiteralValue::Int(i) => Some(super::intern_transient(interner, &i.to_string())),
            _ => None,
        },
        CompiledOutput::VarMethodCall { variable, method } => {
            let var_name = interner.resolve(*variable)?;
            if let Some(AttributeValue::String(s)) = variables.get(&*var_name) {
                let str_val = interner.resolve(*s)?;
                let result = match method {
                    OutputMethod::Lower => str_val.to_lowercase(),
                    OutputMethod::Upper => str_val.to_uppercase(),
                    OutputMethod::Trim => str_val.trim().to_string(),
                };
                Some(super::intern_transient(interner, &result))
            } else {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn create_test_interner() -> Arc<StringInterner> {
        Arc::new(StringInterner::new())
    }

    #[test]
    fn test_get_output_variable() {
        let interner = create_test_interner();
        let var_key = interner.intern("x");
        let val = interner.intern("hello");

        let mut vars = HashMap::new();
        vars.insert("x".to_string(), AttributeValue::String(val));

        let output = Some(CompiledOutput::Variable(var_key));
        let result = get_comprehension_output(&output, &vars, &interner);

        assert!(result.is_some());
        if let Some(AttributeValue::String(s)) = result {
            assert_eq!(s, val);
        } else {
            panic!("Expected String");
        }
    }

    #[test]
    fn test_get_output_literal_string() {
        let interner = create_test_interner();
        let lit = interner.intern("constant");

        let vars = HashMap::new();

        let output = Some(CompiledOutput::Literal(CompiledLiteralValue::String(lit)));
        let result = get_comprehension_output(&output, &vars, &interner);

        assert!(result.is_some());
        if let Some(AttributeValue::String(s)) = result {
            assert_eq!(s, lit);
        } else {
            panic!("Expected String");
        }
    }

    #[test]
    fn test_get_output_literal_int() {
        let interner = create_test_interner();
        let vars = HashMap::new();

        let output = Some(CompiledOutput::Literal(CompiledLiteralValue::Int(42)));
        let result = get_comprehension_output(&output, &vars, &interner);

        assert_eq!(result, Some(AttributeValue::Int(42)));
    }

    #[test]
    fn test_get_output_none() {
        let interner = create_test_interner();
        let vars = HashMap::new();

        let result = get_comprehension_output(&None, &vars, &interner);
        assert!(result.is_none());
    }

    #[test]
    fn test_get_output_as_string_variable() {
        let interner = create_test_interner();
        let var_key = interner.intern("key");
        let val = interner.intern("my_key");

        let mut vars = HashMap::new();
        vars.insert("key".to_string(), AttributeValue::String(val));

        let output = CompiledOutput::Variable(var_key);
        let result = get_comprehension_output_as_string(&output, &vars, &interner);

        assert_eq!(result, Some(val));
    }

    #[test]
    fn test_get_output_as_string_int() {
        let interner = create_test_interner();
        let var_key = interner.intern("num");

        let mut vars = HashMap::new();
        vars.insert("num".to_string(), AttributeValue::Int(123));

        let output = CompiledOutput::Variable(var_key);
        let result = get_comprehension_output_as_string(&output, &vars, &interner);

        assert!(result.is_some());
        if let Some(interned) = result {
            let resolved = interner.resolve(interned).unwrap();
            assert_eq!(&*resolved, "123");
        }
    }

    #[test]
    fn test_get_output_method_call_lower() {
        let interner = create_test_interner();
        let var_key = interner.intern("text");
        let val = interner.intern("HELLO");

        let mut vars = HashMap::new();
        vars.insert("text".to_string(), AttributeValue::String(val));

        let output = Some(CompiledOutput::VarMethodCall {
            variable: var_key,
            method: OutputMethod::Lower,
        });
        let result = get_comprehension_output(&output, &vars, &interner);

        assert!(result.is_some());
        if let Some(AttributeValue::String(s)) = result {
            let resolved = interner.resolve(s).unwrap();
            assert_eq!(&*resolved, "hello");
        } else {
            panic!("Expected String");
        }
    }

    #[test]
    fn test_get_output_method_call_trim() {
        let interner = create_test_interner();
        let var_key = interner.intern("text");
        let val = interner.intern("  spaced  ");

        let mut vars = HashMap::new();
        vars.insert("text".to_string(), AttributeValue::String(val));

        let output = Some(CompiledOutput::VarMethodCall {
            variable: var_key,
            method: OutputMethod::Trim,
        });
        let result = get_comprehension_output(&output, &vars, &interner);

        assert!(result.is_some());
        if let Some(AttributeValue::String(s)) = result {
            let resolved = interner.resolve(s).unwrap();
            assert_eq!(&*resolved, "spaced");
        } else {
            panic!("Expected String");
        }
    }

    /// Test VarAttr output extraction: r.id from an object
    /// This is the pattern used in object comprehensions: {r.id: r.value | ...}
    #[test]
    fn test_get_output_var_attr_string() {
        let interner = create_test_interner();
        let var_key = interner.intern("r");
        let id_attr = interner.intern("id");
        let value_attr = interner.intern("value");
        let rec1 = interner.intern("rec1");

        // Build an object like {"id": "rec1", "value": 100}
        let mut obj_map: HashMap<crate::data::InternedString, AttributeValue> = HashMap::new();
        obj_map.insert(id_attr, AttributeValue::String(rec1));
        obj_map.insert(value_attr, AttributeValue::Int(100));

        let mut vars = HashMap::new();
        vars.insert("r".to_string(), AttributeValue::Object(obj_map));

        // Test: get r.id
        let key_output = CompiledOutput::VarAttr {
            variable: var_key,
            attribute: id_attr,
        };
        let key_result = get_comprehension_output_as_string(&key_output, &vars, &interner);

        assert!(key_result.is_some(), "Expected to get r.id as string");
        assert_eq!(key_result.unwrap(), rec1, "r.id should be 'rec1'");

        // Test: get r.value
        let value_output = Some(CompiledOutput::VarAttr {
            variable: var_key,
            attribute: value_attr,
        });
        let value_result = get_comprehension_output(&value_output, &vars, &interner);

        assert!(value_result.is_some(), "Expected to get r.value");
        assert_eq!(
            value_result.unwrap(),
            AttributeValue::Int(100),
            "r.value should be 100"
        );
    }

    /// Test VarAttr output with an object loaded from JSON-like data
    /// This tests the real scenario where objects come from loaded JSON data
    #[test]
    fn test_get_output_var_attr_from_loaded_object() {
        let interner = create_test_interner();

        // Simulate how the JSON loader would create the object
        // JSON: {"id": "rec1", "value": 100, "active": true}
        let id_key = interner.intern("id");
        let value_key = interner.intern("value");
        let active_key = interner.intern("active");
        let rec1 = interner.intern("rec1");

        let mut record: HashMap<crate::data::InternedString, AttributeValue> = HashMap::new();
        record.insert(id_key, AttributeValue::String(rec1));
        record.insert(value_key, AttributeValue::Int(100));
        record.insert(active_key, AttributeValue::Bool(true));

        // Variable 'r' is bound to this record during iteration
        let var_r = interner.intern("r");
        let mut vars = HashMap::new();
        vars.insert("r".to_string(), AttributeValue::Object(record.clone()));

        // Test getting r.id using the same interner
        let id_output = CompiledOutput::VarAttr {
            variable: var_r,
            attribute: id_key, // Same InternedString used for the key
        };
        let id_result = get_comprehension_output_as_string(&id_output, &vars, &interner);

        assert!(id_result.is_some(), "r.id lookup failed - got None");
        let id_interned = id_result.unwrap();
        let id_resolved = interner.resolve(id_interned).unwrap();
        assert_eq!(&*id_resolved, "rec1", "r.id should resolve to 'rec1'");

        // Test getting r.value
        let value_output = Some(CompiledOutput::VarAttr {
            variable: var_r,
            attribute: value_key,
        });
        let value_result = get_comprehension_output(&value_output, &vars, &interner);

        assert_eq!(
            value_result,
            Some(AttributeValue::Int(100)),
            "r.value should be 100"
        );
    }
}
