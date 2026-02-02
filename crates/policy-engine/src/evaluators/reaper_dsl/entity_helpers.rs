//! Entity access helpers for condition evaluation.
//!
//! This module provides helper functions that eliminate the 20+ duplicate
//! entity matching patterns in the evaluator:
//!
//! ```text
//! let entity = match entity_type {
//!     EntityType::User => user,
//!     EntityType::Resource => resource,
//!     EntityType::Context => return None,
//! };
//! ```
//!
//! Performance: These helpers are inlined for zero-overhead abstraction.

use crate::data::{AttributeValue, Entity, InternedString, StringInterner};

use super::types::EntityType;

/// Get the entity reference for a given entity type.
///
/// Returns `None` for Context type since it's not an Entity.
///
/// # Examples
/// ```text
/// let entity = get_entity_for_type(&entity_type, user, resource)?;
/// let attr = entity.get_attribute(attr_name);
/// ```
#[inline(always)]
pub fn get_entity_for_type<'a>(
    entity_type: &EntityType,
    user: &'a Entity,
    resource: &'a Entity,
) -> Option<&'a Entity> {
    match entity_type {
        EntityType::User => Some(user),
        EntityType::Resource => Some(resource),
        EntityType::Context => None,
    }
}

/// Get a string attribute from the appropriate entity.
///
/// Returns `None` if:
/// - Entity type is Context
/// - Attribute doesn't exist
/// - Attribute is not a string
#[inline(always)]
pub fn get_string_attr<'a>(
    entity_type: &EntityType,
    attribute: InternedString,
    user: &'a Entity,
    resource: &'a Entity,
) -> Option<InternedString> {
    let entity = get_entity_for_type(entity_type, user, resource)?;
    match entity.get_attribute(attribute) {
        Some(AttributeValue::String(s)) => Some(*s),
        _ => None,
    }
}

/// Get a numeric attribute from the appropriate entity as f64.
///
/// Returns `None` if:
/// - Entity type is Context
/// - Attribute doesn't exist
/// - Attribute is not numeric (Int or Float)
#[inline(always)]
pub fn get_numeric_attr(
    entity_type: &EntityType,
    attribute: InternedString,
    user: &Entity,
    resource: &Entity,
) -> Option<f64> {
    let entity = get_entity_for_type(entity_type, user, resource)?;
    match entity.get_attribute(attribute) {
        Some(AttributeValue::Int(n)) => Some(*n as f64),
        Some(AttributeValue::Float(f)) => Some(*f),
        _ => None,
    }
}

/// Get an integer attribute from the appropriate entity.
#[inline(always)]
pub fn get_int_attr(
    entity_type: &EntityType,
    attribute: InternedString,
    user: &Entity,
    resource: &Entity,
) -> Option<i64> {
    let entity = get_entity_for_type(entity_type, user, resource)?;
    match entity.get_attribute(attribute) {
        Some(AttributeValue::Int(n)) => Some(*n),
        _ => None,
    }
}

/// Get a boolean attribute from the appropriate entity.
#[inline(always)]
pub fn get_bool_attr(
    entity_type: &EntityType,
    attribute: InternedString,
    user: &Entity,
    resource: &Entity,
) -> Option<bool> {
    let entity = get_entity_for_type(entity_type, user, resource)?;
    match entity.get_attribute(attribute) {
        Some(AttributeValue::Bool(b)) => Some(*b),
        _ => None,
    }
}

/// Get any attribute value from the appropriate entity.
#[inline(always)]
pub fn get_attr<'a>(
    entity_type: &EntityType,
    attribute: InternedString,
    user: &'a Entity,
    resource: &'a Entity,
) -> Option<&'a AttributeValue> {
    let entity = get_entity_for_type(entity_type, user, resource)?;
    entity.get_attribute(attribute)
}

/// Get a potentially nested attribute value from the appropriate entity.
///
/// Handles dotted attribute names like "config.name" by navigating through
/// nested objects. For simple attributes, this is equivalent to get_attr.
///
/// Returns `None` if:
/// - Entity type is Context
/// - Any part of the attribute path doesn't exist
/// - Any intermediate value is not an Object
#[inline]
pub fn get_nested_attr<'a>(
    entity_type: &EntityType,
    attribute: InternedString,
    user: &'a Entity,
    resource: &'a Entity,
    interner: &StringInterner,
) -> Option<AttributeValue> {
    let entity = get_entity_for_type(entity_type, user, resource)?;

    // Resolve the attribute name
    let attr_name = interner.resolve(attribute)?;

    // Check if it's a simple or dotted attribute
    if !attr_name.contains('.') {
        // Simple attribute - just return a clone
        return entity.get_attribute(attribute).cloned();
    }

    // Split by '.' and navigate through nested objects
    let parts: Vec<&str> = attr_name.split('.').collect();

    // Get the first attribute
    let first_attr = interner.intern(parts[0]);
    let mut current_value = entity.get_attribute(first_attr)?.clone();

    // Navigate through the rest of the path
    for part in &parts[1..] {
        match current_value {
            AttributeValue::Object(ref map) => {
                // Object uses InternedString keys
                let key = interner.intern(part);
                current_value = map.get(&key)?.clone();
            }
            _ => return None, // Can't navigate into non-object
        }
    }

    Some(current_value)
}

/// Check if a potentially nested attribute is null.
///
/// Returns `true` if:
/// - The attribute doesn't exist at any level
/// - The attribute value is AttributeValue::Null
/// - Any intermediate path component doesn't exist or isn't an object
#[inline]
pub fn is_nested_attr_null(
    entity_type: &EntityType,
    attribute: InternedString,
    user: &Entity,
    resource: &Entity,
    interner: &StringInterner,
) -> bool {
    match get_nested_attr(entity_type, attribute, user, resource, interner) {
        None => true,
        Some(AttributeValue::Null) => true,
        _ => false,
    }
}

/// Compare a string attribute to a literal value.
///
/// Returns true if the attribute equals the given value.
#[inline(always)]
pub fn string_attr_equals(
    entity_type: &EntityType,
    attribute: InternedString,
    value: InternedString,
    user: &Entity,
    resource: &Entity,
) -> bool {
    get_string_attr(entity_type, attribute, user, resource)
        .map(|actual| actual == value)
        .unwrap_or(false)
}

/// Compare a numeric attribute to a threshold with given comparison operator.
#[inline(always)]
pub fn numeric_comparison(
    entity_type: &EntityType,
    attribute: InternedString,
    threshold: f64,
    user: &Entity,
    resource: &Entity,
    cmp: impl Fn(f64, f64) -> bool,
) -> bool {
    get_numeric_attr(entity_type, attribute, user, resource)
        .map(|actual| cmp(actual, threshold))
        .unwrap_or(false)
}

/// Check if attribute value is greater than or equal to threshold.
#[inline(always)]
pub fn attr_gte(
    entity_type: &EntityType,
    attribute: InternedString,
    threshold: f64,
    user: &Entity,
    resource: &Entity,
) -> bool {
    numeric_comparison(entity_type, attribute, threshold, user, resource, |a, t| {
        a >= t
    })
}

/// Check if attribute value is greater than threshold.
#[inline(always)]
pub fn attr_gt(
    entity_type: &EntityType,
    attribute: InternedString,
    threshold: f64,
    user: &Entity,
    resource: &Entity,
) -> bool {
    numeric_comparison(entity_type, attribute, threshold, user, resource, |a, t| a > t)
}

/// Check if attribute value is less than or equal to threshold.
#[inline(always)]
pub fn attr_lte(
    entity_type: &EntityType,
    attribute: InternedString,
    threshold: f64,
    user: &Entity,
    resource: &Entity,
) -> bool {
    numeric_comparison(entity_type, attribute, threshold, user, resource, |a, t| {
        a <= t
    })
}

/// Check if attribute value is less than threshold.
#[inline(always)]
pub fn attr_lt(
    entity_type: &EntityType,
    attribute: InternedString,
    threshold: f64,
    user: &Entity,
    resource: &Entity,
) -> bool {
    numeric_comparison(entity_type, attribute, threshold, user, resource, |a, t| a < t)
}

/// Get the count of elements in a List or Set attribute.
#[inline(always)]
pub fn get_collection_count(
    entity_type: &EntityType,
    attribute: InternedString,
    user: &Entity,
    resource: &Entity,
) -> Option<usize> {
    let entity = get_entity_for_type(entity_type, user, resource)?;
    match entity.get_attribute(attribute) {
        Some(AttributeValue::List(list)) => Some(list.len()),
        Some(AttributeValue::Set(set)) => Some(set.len()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::StringInterner;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn create_test_interner() -> Arc<StringInterner> {
        Arc::new(StringInterner::new())
    }

    fn create_test_entities(
        interner: &StringInterner,
    ) -> (Entity, Entity, InternedString, InternedString, InternedString, InternedString) {
        // Intern keys
        let role_key = interner.intern("role");
        let level_key = interner.intern("level");
        let owner_key = interner.intern("owner");
        let admin_val = interner.intern("admin");

        // Create user entity
        let user_id = interner.intern("user_alice");
        let user_type = interner.intern("User");
        let mut user_attrs: HashMap<InternedString, AttributeValue> = HashMap::new();
        user_attrs.insert(role_key, AttributeValue::String(admin_val));
        user_attrs.insert(level_key, AttributeValue::Int(5));
        let user = Entity::new(user_id, user_type, user_attrs);

        // Create resource entity
        let resource_id = interner.intern("resource_doc1");
        let resource_type = interner.intern("Resource");
        let alice_val = interner.intern("alice");
        let mut resource_attrs: HashMap<InternedString, AttributeValue> = HashMap::new();
        resource_attrs.insert(owner_key, AttributeValue::String(alice_val));
        let resource = Entity::new(resource_id, resource_type, resource_attrs);

        (user, resource, role_key, level_key, owner_key, admin_val)
    }

    #[test]
    fn test_get_entity_for_type_user() {
        let interner = create_test_interner();
        let (user, resource, ..) = create_test_entities(&interner);
        let entity = get_entity_for_type(&EntityType::User, &user, &resource);
        assert!(entity.is_some());
        assert_eq!(entity.unwrap().id, user.id);
    }

    #[test]
    fn test_get_entity_for_type_resource() {
        let interner = create_test_interner();
        let (user, resource, ..) = create_test_entities(&interner);
        let entity = get_entity_for_type(&EntityType::Resource, &user, &resource);
        assert!(entity.is_some());
        assert_eq!(entity.unwrap().id, resource.id);
    }

    #[test]
    fn test_get_entity_for_type_context() {
        let interner = create_test_interner();
        let (user, resource, ..) = create_test_entities(&interner);
        let entity = get_entity_for_type(&EntityType::Context, &user, &resource);
        assert!(entity.is_none());
    }

    #[test]
    fn test_get_string_attr() {
        let interner = create_test_interner();
        let (user, resource, role_key, _, _, admin_val) = create_test_entities(&interner);

        let result = get_string_attr(&EntityType::User, role_key, &user, &resource);
        assert!(result.is_some());
        assert_eq!(result.unwrap(), admin_val);
    }

    #[test]
    fn test_get_string_attr_not_found() {
        let interner = create_test_interner();
        let (user, resource, ..) = create_test_entities(&interner);
        let unknown_key = interner.intern("unknown");

        let result = get_string_attr(&EntityType::User, unknown_key, &user, &resource);
        assert!(result.is_none());
    }

    #[test]
    fn test_get_numeric_attr() {
        let interner = create_test_interner();
        let (user, resource, _, level_key, ..) = create_test_entities(&interner);

        let result = get_numeric_attr(&EntityType::User, level_key, &user, &resource);
        assert!(result.is_some());
        assert!((result.unwrap() - 5.0).abs() < 0.001);
    }

    #[test]
    fn test_string_attr_equals() {
        let interner = create_test_interner();
        let (user, resource, role_key, _, _, admin_val) = create_test_entities(&interner);
        let viewer_val = interner.intern("viewer");

        assert!(string_attr_equals(
            &EntityType::User,
            role_key,
            admin_val,
            &user,
            &resource
        ));
        assert!(!string_attr_equals(
            &EntityType::User,
            role_key,
            viewer_val,
            &user,
            &resource
        ));
    }

    #[test]
    fn test_attr_gte() {
        let interner = create_test_interner();
        let (user, resource, _, level_key, ..) = create_test_entities(&interner);

        assert!(attr_gte(&EntityType::User, level_key, 5.0, &user, &resource));
        assert!(attr_gte(&EntityType::User, level_key, 4.0, &user, &resource));
        assert!(!attr_gte(&EntityType::User, level_key, 6.0, &user, &resource));
    }

    #[test]
    fn test_attr_gt() {
        let interner = create_test_interner();
        let (user, resource, _, level_key, ..) = create_test_entities(&interner);

        assert!(attr_gt(&EntityType::User, level_key, 4.0, &user, &resource));
        assert!(!attr_gt(&EntityType::User, level_key, 5.0, &user, &resource));
        assert!(!attr_gt(&EntityType::User, level_key, 6.0, &user, &resource));
    }

    #[test]
    fn test_attr_lte() {
        let interner = create_test_interner();
        let (user, resource, _, level_key, ..) = create_test_entities(&interner);

        assert!(attr_lte(&EntityType::User, level_key, 5.0, &user, &resource));
        assert!(attr_lte(&EntityType::User, level_key, 6.0, &user, &resource));
        assert!(!attr_lte(
            &EntityType::User,
            level_key,
            4.0,
            &user,
            &resource
        ));
    }

    #[test]
    fn test_attr_lt() {
        let interner = create_test_interner();
        let (user, resource, _, level_key, ..) = create_test_entities(&interner);

        assert!(attr_lt(&EntityType::User, level_key, 6.0, &user, &resource));
        assert!(!attr_lt(&EntityType::User, level_key, 5.0, &user, &resource));
        assert!(!attr_lt(&EntityType::User, level_key, 4.0, &user, &resource));
    }

    #[test]
    fn test_numeric_comparison_with_float() {
        let interner = create_test_interner();
        let score_key = interner.intern("score");

        let user_id = interner.intern("test_user");
        let user_type = interner.intern("User");
        let mut attrs: HashMap<InternedString, AttributeValue> = HashMap::new();
        attrs.insert(score_key, AttributeValue::Float(7.5));
        let user = Entity::new(user_id, user_type, attrs);

        let resource_id = interner.intern("test_resource");
        let resource_type = interner.intern("Resource");
        let resource = Entity::new(resource_id, resource_type, HashMap::new());

        assert!(attr_gte(&EntityType::User, score_key, 7.5, &user, &resource));
        assert!(attr_gte(&EntityType::User, score_key, 7.0, &user, &resource));
        assert!(attr_gt(&EntityType::User, score_key, 7.0, &user, &resource));
        assert!(!attr_gt(&EntityType::User, score_key, 7.5, &user, &resource));
    }

    #[test]
    fn test_get_collection_count_list() {
        let interner = create_test_interner();
        let roles_key = interner.intern("roles");
        let admin = interner.intern("admin");
        let viewer = interner.intern("viewer");

        let user_id = interner.intern("test_user");
        let user_type = interner.intern("User");
        let mut attrs: HashMap<InternedString, AttributeValue> = HashMap::new();
        attrs.insert(
            roles_key,
            AttributeValue::List(vec![
                AttributeValue::String(admin),
                AttributeValue::String(viewer),
            ]),
        );
        let user = Entity::new(user_id, user_type, attrs);

        let resource_id = interner.intern("test_resource");
        let resource_type = interner.intern("Resource");
        let resource = Entity::new(resource_id, resource_type, HashMap::new());

        let count = get_collection_count(&EntityType::User, roles_key, &user, &resource);
        assert_eq!(count, Some(2));
    }

    #[test]
    fn test_resource_attributes() {
        let interner = create_test_interner();
        let (user, resource, _, _, owner_key, _) = create_test_entities(&interner);
        let alice_val = interner.intern("alice");

        let result = get_string_attr(&EntityType::Resource, owner_key, &user, &resource);
        assert!(result.is_some());
        assert_eq!(result.unwrap(), alice_val);

        assert!(string_attr_equals(
            &EntityType::Resource,
            owner_key,
            alice_val,
            &user,
            &resource
        ));
    }

    #[test]
    fn test_get_bool_attr() {
        let interner = create_test_interner();
        let active_key = interner.intern("active");

        let user_id = interner.intern("test_user");
        let user_type = interner.intern("User");
        let mut attrs: HashMap<InternedString, AttributeValue> = HashMap::new();
        attrs.insert(active_key, AttributeValue::Bool(true));
        let user = Entity::new(user_id, user_type, attrs);

        let resource_id = interner.intern("test_resource");
        let resource_type = interner.intern("Resource");
        let resource = Entity::new(resource_id, resource_type, HashMap::new());

        let result = get_bool_attr(&EntityType::User, active_key, &user, &resource);
        assert_eq!(result, Some(true));
    }

    #[test]
    fn test_get_int_attr() {
        let interner = create_test_interner();
        let (user, resource, _, level_key, ..) = create_test_entities(&interner);

        let result = get_int_attr(&EntityType::User, level_key, &user, &resource);
        assert_eq!(result, Some(5));
    }

    #[test]
    fn test_get_attr_returns_reference() {
        let interner = create_test_interner();
        let (user, resource, role_key, ..) = create_test_entities(&interner);

        let result = get_attr(&EntityType::User, role_key, &user, &resource);
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), AttributeValue::String(_)));
    }

    #[test]
    fn test_context_entity_type_returns_none() {
        let interner = create_test_interner();
        let (user, resource, role_key, ..) = create_test_entities(&interner);

        assert!(get_string_attr(&EntityType::Context, role_key, &user, &resource).is_none());
        assert!(get_numeric_attr(&EntityType::Context, role_key, &user, &resource).is_none());
        assert!(get_bool_attr(&EntityType::Context, role_key, &user, &resource).is_none());
        assert!(!string_attr_equals(
            &EntityType::Context,
            role_key,
            role_key,
            &user,
            &resource
        ));
    }
}
