//! String operation evaluation.
//!
//! This module handles string-based conditions:
//! - contains, starts_with, ends_with
//! - regex matching
//! - lower/upper case comparisons
//!
//! ## Performance Characteristics
//! - String operations use memchr for fast substring search
//! - Regex patterns are cached globally for O(1) lookup
//! - Case transformations are done inline

// Allow unused functions - some are used in tests only or reserved for future use
#![allow(dead_code)]

use crate::data::{AttributeValue, Entity, InternedString, StringInterner};
use memchr::memmem;

use super::entity_helpers::get_entity_for_type;
use super::types::{
    CompiledRegexMatch, CompiledStringOperation, CompiledVariableStringOp, EntityType, StringOp,
};

// ============================================================================
// V2 Dispatch Functions
// ============================================================================

/// Evaluate a V2 string operation
#[inline]
pub fn eval_string_operation(
    op: &CompiledStringOperation,
    user: &Entity,
    resource: &Entity,
    interner: &StringInterner,
) -> bool {
    let entity = match get_entity_for_type(&op.entity_type, user, resource) {
        Some(e) => e,
        None => {
            tracing::debug!(
                entity_type = ?op.entity_type,
                "StringOp: entity not found"
            );
            return false;
        }
    };

    // NOTE: do not resolve the attribute name here — it was previously computed
    // (a DashMap lookup + String allocation) on every string op purely for the
    // debug logs below, which are compiled out at runtime unless debug logging
    // is enabled. The interned id (`?op.attribute`) is logged instead at zero cost.
    match entity.get_attribute(op.attribute) {
        Some(AttributeValue::String(s)) => {
            if let Some(resolved) = interner.resolve(*s) {
                let result = match op.op {
                    StringOp::Contains => {
                        memmem::find(resolved.as_bytes(), op.value.as_bytes()).is_some()
                    }
                    StringOp::StartsWith => resolved.starts_with(&op.value),
                    StringOp::EndsWith => resolved.ends_with(&op.value),
                    StringOp::LowerEquals => resolved.to_lowercase() == op.value,
                    StringOp::UpperEquals => resolved.to_uppercase() == op.value,
                };
                tracing::debug!(
                    entity_type = ?op.entity_type,
                    attribute = ?op.attribute,
                    attr_value = %resolved,
                    op = ?op.op,
                    op_value = %op.value,
                    result = result,
                    "StringOp evaluation"
                );
                result
            } else {
                false
            }
        }
        _ => false,
    }
}

/// Evaluate a V2 variable string operation
#[inline]
pub fn eval_variable_string_operation(
    op: &CompiledVariableStringOp,
    variables: &std::collections::HashMap<String, AttributeValue>,
    interner: &StringInterner,
) -> bool {
    if let Some(var_name) = interner.resolve(op.variable) {
        if let Some(AttributeValue::String(s)) = variables.get(&*var_name) {
            if let Some(resolved) = interner.resolve(*s) {
                return match op.op {
                    StringOp::Contains => {
                        memmem::find(resolved.as_bytes(), op.value.as_bytes()).is_some()
                    }
                    StringOp::StartsWith => resolved.starts_with(&op.value),
                    StringOp::EndsWith => resolved.ends_with(&op.value),
                    StringOp::LowerEquals => resolved.to_lowercase() == op.value,
                    StringOp::UpperEquals => resolved.to_uppercase() == op.value,
                };
            }
        }
    }
    false
}

/// Evaluate a V2 regex match
#[inline]
pub fn eval_regex_match(
    m: &CompiledRegexMatch,
    user: &Entity,
    resource: &Entity,
    interner: &StringInterner,
) -> bool {
    let entity = match get_entity_for_type(&m.entity_type, user, resource) {
        Some(e) => e,
        None => return false,
    };

    match entity.get_attribute(m.attribute) {
        Some(AttributeValue::String(s)) => {
            if let Some(resolved) = interner.resolve(*s) {
                crate::regex_cache::matches(&m.pattern, &resolved)
            } else {
                false
            }
        }
        _ => false,
    }
}

// ============================================================================
// Legacy Functions (still used by some code paths)
// ============================================================================

/// Evaluate string contains: entity.attr.contains(substring)
#[inline]
pub fn eval_string_contains(
    entity_type: &EntityType,
    attribute: InternedString,
    substring: &str,
    user: &Entity,
    resource: &Entity,
    interner: &StringInterner,
) -> bool {
    let entity = match get_entity_for_type(entity_type, user, resource) {
        Some(e) => e,
        None => return false,
    };

    match entity.get_attribute(attribute) {
        Some(AttributeValue::String(s)) => {
            if let Some(resolved) = interner.resolve(*s) {
                memmem::find(resolved.as_bytes(), substring.as_bytes()).is_some()
            } else {
                false
            }
        }
        _ => false,
    }
}

/// Evaluate string starts with: entity.attr.startswith(prefix)
#[inline]
pub fn eval_string_starts_with(
    entity_type: &EntityType,
    attribute: InternedString,
    prefix: &str,
    user: &Entity,
    resource: &Entity,
    interner: &StringInterner,
) -> bool {
    let entity = match get_entity_for_type(entity_type, user, resource) {
        Some(e) => e,
        None => return false,
    };

    match entity.get_attribute(attribute) {
        Some(AttributeValue::String(s)) => {
            if let Some(resolved) = interner.resolve(*s) {
                resolved.starts_with(prefix)
            } else {
                false
            }
        }
        _ => false,
    }
}

/// Evaluate string ends with: entity.attr.endswith(suffix)
#[inline]
pub fn eval_string_ends_with(
    entity_type: &EntityType,
    attribute: InternedString,
    suffix: &str,
    user: &Entity,
    resource: &Entity,
    interner: &StringInterner,
) -> bool {
    let entity = match get_entity_for_type(entity_type, user, resource) {
        Some(e) => e,
        None => return false,
    };

    match entity.get_attribute(attribute) {
        Some(AttributeValue::String(s)) => {
            if let Some(resolved) = interner.resolve(*s) {
                resolved.ends_with(suffix)
            } else {
                false
            }
        }
        _ => false,
    }
}

/// Evaluate regex match: regex::matches(entity.attr, pattern)
#[inline]
pub fn eval_regex_matches(
    entity_type: &EntityType,
    attribute: InternedString,
    pattern: &str,
    user: &Entity,
    resource: &Entity,
    interner: &StringInterner,
) -> bool {
    let entity = match get_entity_for_type(entity_type, user, resource) {
        Some(e) => e,
        None => return false,
    };

    match entity.get_attribute(attribute) {
        Some(AttributeValue::String(s)) => {
            if let Some(resolved) = interner.resolve(*s) {
                crate::regex_cache::matches(pattern, &resolved)
            } else {
                false
            }
        }
        _ => false,
    }
}

/// Evaluate variable string contains: var.contains(substring)
#[inline]
pub fn eval_variable_string_contains(
    variable: InternedString,
    substring: &str,
    variables: &std::collections::HashMap<String, AttributeValue>,
    interner: &StringInterner,
) -> bool {
    if let Some(var_name) = interner.resolve(variable) {
        if let Some(AttributeValue::String(s)) = variables.get(&*var_name) {
            if let Some(resolved) = interner.resolve(*s) {
                return memmem::find(resolved.as_bytes(), substring.as_bytes()).is_some();
            }
        }
    }
    false
}

/// Evaluate variable string starts with: var.startswith(prefix)
#[inline]
pub fn eval_variable_string_starts_with(
    variable: InternedString,
    prefix: &str,
    variables: &std::collections::HashMap<String, AttributeValue>,
    interner: &StringInterner,
) -> bool {
    if let Some(var_name) = interner.resolve(variable) {
        if let Some(AttributeValue::String(s)) = variables.get(&*var_name) {
            if let Some(resolved) = interner.resolve(*s) {
                return resolved.starts_with(prefix);
            }
        }
    }
    false
}

/// Evaluate variable string ends with: var.endswith(suffix)
#[inline]
pub fn eval_variable_string_ends_with(
    variable: InternedString,
    suffix: &str,
    variables: &std::collections::HashMap<String, AttributeValue>,
    interner: &StringInterner,
) -> bool {
    if let Some(var_name) = interner.resolve(variable) {
        if let Some(AttributeValue::String(s)) = variables.get(&*var_name) {
            if let Some(resolved) = interner.resolve(*s) {
                return resolved.ends_with(suffix);
            }
        }
    }
    false
}

/// Evaluate string lowercase equals: entity.attr.lower() == value
#[inline]
pub fn eval_string_lower_equals(
    entity_type: &EntityType,
    attribute: InternedString,
    value: &str,
    user: &Entity,
    resource: &Entity,
    interner: &StringInterner,
) -> bool {
    let entity = match get_entity_for_type(entity_type, user, resource) {
        Some(e) => e,
        None => return false,
    };

    match entity.get_attribute(attribute) {
        Some(AttributeValue::String(s)) => {
            if let Some(resolved) = interner.resolve(*s) {
                resolved.to_lowercase() == value
            } else {
                false
            }
        }
        _ => false,
    }
}

/// Evaluate string uppercase equals: entity.attr.upper() == value
#[inline]
pub fn eval_string_upper_equals(
    entity_type: &EntityType,
    attribute: InternedString,
    value: &str,
    user: &Entity,
    resource: &Entity,
    interner: &StringInterner,
) -> bool {
    let entity = match get_entity_for_type(entity_type, user, resource) {
        Some(e) => e,
        None => return false,
    };

    match entity.get_attribute(attribute) {
        Some(AttributeValue::String(s)) => {
            if let Some(resolved) = interner.resolve(*s) {
                resolved.to_uppercase() == value
            } else {
                false
            }
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn create_test_interner() -> Arc<StringInterner> {
        Arc::new(StringInterner::new())
    }

    fn create_test_user(interner: &StringInterner) -> Entity {
        let user_id = interner.intern("user_alice");
        let user_type = interner.intern("User");

        let email_key = interner.intern("email");
        let email_val = interner.intern("alice@company.com");

        let name_key = interner.intern("name");
        let name_val = interner.intern("Alice Smith");

        let code_key = interner.intern("code");
        let code_val = interner.intern("ABC123");

        let mut attrs: HashMap<InternedString, AttributeValue> = HashMap::new();
        attrs.insert(email_key, AttributeValue::String(email_val));
        attrs.insert(name_key, AttributeValue::String(name_val));
        attrs.insert(code_key, AttributeValue::String(code_val));

        Entity::new(user_id, user_type, attrs)
    }

    fn create_test_resource(interner: &StringInterner) -> Entity {
        let resource_id = interner.intern("resource_doc1");
        let resource_type = interner.intern("Resource");
        Entity::new(resource_id, resource_type, HashMap::new())
    }

    #[test]
    fn test_eval_string_contains() {
        let interner = create_test_interner();
        let user = create_test_user(&interner);
        let resource = create_test_resource(&interner);

        let email_key = interner.intern("email");

        assert!(eval_string_contains(
            &EntityType::User,
            email_key,
            "@company.com",
            &user,
            &resource,
            &interner
        ));

        assert!(eval_string_contains(
            &EntityType::User,
            email_key,
            "alice",
            &user,
            &resource,
            &interner
        ));

        assert!(!eval_string_contains(
            &EntityType::User,
            email_key,
            "@gmail.com",
            &user,
            &resource,
            &interner
        ));
    }

    #[test]
    fn test_eval_string_starts_with() {
        let interner = create_test_interner();
        let user = create_test_user(&interner);
        let resource = create_test_resource(&interner);

        let email_key = interner.intern("email");

        assert!(eval_string_starts_with(
            &EntityType::User,
            email_key,
            "alice",
            &user,
            &resource,
            &interner
        ));

        assert!(!eval_string_starts_with(
            &EntityType::User,
            email_key,
            "bob",
            &user,
            &resource,
            &interner
        ));
    }

    #[test]
    fn test_eval_string_ends_with() {
        let interner = create_test_interner();
        let user = create_test_user(&interner);
        let resource = create_test_resource(&interner);

        let email_key = interner.intern("email");

        assert!(eval_string_ends_with(
            &EntityType::User,
            email_key,
            ".com",
            &user,
            &resource,
            &interner
        ));

        assert!(eval_string_ends_with(
            &EntityType::User,
            email_key,
            "@company.com",
            &user,
            &resource,
            &interner
        ));

        assert!(!eval_string_ends_with(
            &EntityType::User,
            email_key,
            ".gov",
            &user,
            &resource,
            &interner
        ));
    }

    #[test]
    fn test_eval_regex_matches() {
        let interner = create_test_interner();
        let user = create_test_user(&interner);
        let resource = create_test_resource(&interner);

        let email_key = interner.intern("email");

        // Valid email pattern
        assert!(eval_regex_matches(
            &EntityType::User,
            email_key,
            r"^[a-z]+@[a-z]+\.[a-z]+$",
            &user,
            &resource,
            &interner
        ));

        // Pattern that doesn't match
        assert!(!eval_regex_matches(
            &EntityType::User,
            email_key,
            r"^[0-9]+$",
            &user,
            &resource,
            &interner
        ));
    }

    #[test]
    fn test_eval_variable_string_contains() {
        let interner = create_test_interner();
        let var_name = interner.intern("test_var");
        let var_val = interner.intern("hello world");

        let mut variables: HashMap<String, AttributeValue> = HashMap::new();
        variables.insert("test_var".to_string(), AttributeValue::String(var_val));

        assert!(eval_variable_string_contains(
            var_name, "world", &variables, &interner
        ));

        assert!(!eval_variable_string_contains(
            var_name, "foo", &variables, &interner
        ));
    }

    #[test]
    fn test_eval_variable_string_starts_with() {
        let interner = create_test_interner();
        let var_name = interner.intern("test_var");
        let var_val = interner.intern("hello world");

        let mut variables: HashMap<String, AttributeValue> = HashMap::new();
        variables.insert("test_var".to_string(), AttributeValue::String(var_val));

        assert!(eval_variable_string_starts_with(
            var_name, "hello", &variables, &interner
        ));

        assert!(!eval_variable_string_starts_with(
            var_name, "world", &variables, &interner
        ));
    }

    #[test]
    fn test_eval_variable_string_ends_with() {
        let interner = create_test_interner();
        let var_name = interner.intern("test_var");
        let var_val = interner.intern("hello world");

        let mut variables: HashMap<String, AttributeValue> = HashMap::new();
        variables.insert("test_var".to_string(), AttributeValue::String(var_val));

        assert!(eval_variable_string_ends_with(
            var_name, "world", &variables, &interner
        ));

        assert!(!eval_variable_string_ends_with(
            var_name, "hello", &variables, &interner
        ));
    }

    #[test]
    fn test_eval_string_lower_equals() {
        let interner = create_test_interner();
        let user = create_test_user(&interner);
        let resource = create_test_resource(&interner);

        let name_key = interner.intern("name");

        // "Alice Smith".lower() == "alice smith"
        assert!(eval_string_lower_equals(
            &EntityType::User,
            name_key,
            "alice smith",
            &user,
            &resource,
            &interner
        ));

        assert!(!eval_string_lower_equals(
            &EntityType::User,
            name_key,
            "Alice Smith",
            &user,
            &resource,
            &interner
        ));
    }

    #[test]
    fn test_eval_string_upper_equals() {
        let interner = create_test_interner();
        let user = create_test_user(&interner);
        let resource = create_test_resource(&interner);

        let code_key = interner.intern("code");

        // "ABC123".upper() == "ABC123"
        assert!(eval_string_upper_equals(
            &EntityType::User,
            code_key,
            "ABC123",
            &user,
            &resource,
            &interner
        ));

        // Also test that it's case-sensitive for the comparison
        assert!(!eval_string_upper_equals(
            &EntityType::User,
            code_key,
            "abc123",
            &user,
            &resource,
            &interner
        ));
    }

    #[test]
    fn test_context_entity_type_returns_false() {
        let interner = create_test_interner();
        let user = create_test_user(&interner);
        let resource = create_test_resource(&interner);

        let email_key = interner.intern("email");

        assert!(!eval_string_contains(
            &EntityType::Context,
            email_key,
            "test",
            &user,
            &resource,
            &interner
        ));

        assert!(!eval_string_starts_with(
            &EntityType::Context,
            email_key,
            "test",
            &user,
            &resource,
            &interner
        ));
    }

    #[test]
    fn test_missing_attribute_returns_false() {
        let interner = create_test_interner();
        let user = create_test_user(&interner);
        let resource = create_test_resource(&interner);

        let unknown_key = interner.intern("unknown");

        assert!(!eval_string_contains(
            &EntityType::User,
            unknown_key,
            "test",
            &user,
            &resource,
            &interner
        ));
    }

    #[test]
    fn test_non_string_attribute_returns_false() {
        let interner = create_test_interner();

        let user_id = interner.intern("user_test");
        let user_type = interner.intern("User");
        let level_key = interner.intern("level");

        let mut attrs: HashMap<InternedString, AttributeValue> = HashMap::new();
        attrs.insert(level_key, AttributeValue::Int(5));

        let user = Entity::new(user_id, user_type, attrs);

        let resource_id = interner.intern("resource_test");
        let resource_type = interner.intern("Resource");
        let resource = Entity::new(resource_id, resource_type, HashMap::new());

        // Trying string ops on an Int attribute
        assert!(!eval_string_contains(
            &EntityType::User,
            level_key,
            "5",
            &user,
            &resource,
            &interner
        ));
    }
}
