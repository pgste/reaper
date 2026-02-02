//! Collection operation evaluation.
//!
//! This module handles collection-based conditions:
//! - count comparisons (count() >= N, count() > N, count() == N)
//! - membership tests (value in collection)
//! - set operations (intersection, union, difference)
//! - indexed access (collection[0], collection[key])
//!
//! ## Performance Characteristics
//! - HashSet membership is O(1)
//! - List iteration is O(n) but optimized with iterators
//! - Set operations leverage Rust's FxHashSet

// Allow unused functions - some are used in tests only or reserved for future use
#![allow(dead_code)]

use crate::data::{AttributeValue, Entity, InternedString, StringInterner};
use rustc_hash::FxHashSet;

use super::entity_helpers::get_entity_for_type;
use super::types::{CompiledCountCondition, CompiledLiteralValue, CountOp, EntityType, IndexExpr};

// ============================================================================
// V2 Dispatch Functions
// ============================================================================

/// Evaluate a V2 count operation
#[inline]
pub fn eval_count_operation(
    cond: &CompiledCountCondition,
    user: &Entity,
    resource: &Entity,
) -> bool {
    let entity = match get_entity_for_type(&cond.entity_type, user, resource) {
        Some(e) => e,
        None => return false,
    };

    let count = match entity.get_attribute(cond.attribute) {
        Some(AttributeValue::List(items)) => items.len(),
        Some(AttributeValue::Set(items)) => items.len(),
        _ => return false,
    };

    match cond.op {
        CountOp::GreaterEqual => count >= cond.threshold,
        CountOp::Greater => count > cond.threshold,
        CountOp::Equal => count == cond.threshold,
        CountOp::LessEqual => count <= cond.threshold,
        CountOp::Less => count < cond.threshold,
    }
}

// ============================================================================
// Legacy Functions (still used by some code paths)
// ============================================================================

/// Evaluate count >= threshold: entity.collection.count() >= N
#[inline]
pub fn eval_count_gte(
    entity_type: &EntityType,
    attribute: InternedString,
    threshold: usize,
    user: &Entity,
    resource: &Entity,
) -> bool {
    let entity = match get_entity_for_type(entity_type, user, resource) {
        Some(e) => e,
        None => return false,
    };

    match entity.get_attribute(attribute) {
        Some(AttributeValue::List(items)) => items.len() >= threshold,
        Some(AttributeValue::Set(items)) => items.len() >= threshold,
        _ => false,
    }
}

/// Evaluate count > threshold: entity.collection.count() > N
#[inline]
pub fn eval_count_gt(
    entity_type: &EntityType,
    attribute: InternedString,
    threshold: usize,
    user: &Entity,
    resource: &Entity,
) -> bool {
    let entity = match get_entity_for_type(entity_type, user, resource) {
        Some(e) => e,
        None => return false,
    };

    match entity.get_attribute(attribute) {
        Some(AttributeValue::List(items)) => items.len() > threshold,
        Some(AttributeValue::Set(items)) => items.len() > threshold,
        _ => false,
    }
}

/// Evaluate count == threshold: entity.collection.count() == N
#[inline]
pub fn eval_count_eq(
    entity_type: &EntityType,
    attribute: InternedString,
    threshold: usize,
    user: &Entity,
    resource: &Entity,
) -> bool {
    let entity = match get_entity_for_type(entity_type, user, resource) {
        Some(e) => e,
        None => return false,
    };

    match entity.get_attribute(attribute) {
        Some(AttributeValue::List(items)) => items.len() == threshold,
        Some(AttributeValue::Set(items)) => items.len() == threshold,
        _ => false,
    }
}

/// Check if a compiled literal value is in a list.
#[inline]
pub fn compiled_value_in_list(value: &CompiledLiteralValue, items: &[AttributeValue]) -> bool {
    match value {
        CompiledLiteralValue::String(s) => items
            .iter()
            .any(|item| matches!(item, AttributeValue::String(actual) if *actual == *s)),
        CompiledLiteralValue::Int(n) => items
            .iter()
            .any(|item| matches!(item, AttributeValue::Int(actual) if *actual == *n)),
        CompiledLiteralValue::Bool(b) => items
            .iter()
            .any(|item| matches!(item, AttributeValue::Bool(actual) if *actual == *b)),
    }
}

/// Check if a compiled literal value is in a set.
#[inline]
pub fn compiled_value_in_set(
    value: &CompiledLiteralValue,
    items: &FxHashSet<AttributeValue>,
) -> bool {
    match value {
        CompiledLiteralValue::String(s) => items.contains(&AttributeValue::String(*s)),
        CompiledLiteralValue::Int(n) => items.contains(&AttributeValue::Int(*n)),
        CompiledLiteralValue::Bool(b) => items.contains(&AttributeValue::Bool(*b)),
    }
}

/// Evaluate membership test: value in entity.collection
#[inline]
pub fn eval_membership_test(
    value: &CompiledLiteralValue,
    entity_type: &EntityType,
    attribute: InternedString,
    index: Option<&IndexExpr>,
    user: &Entity,
    resource: &Entity,
    interner: &StringInterner,
) -> bool {
    let entity = match get_entity_for_type(entity_type, user, resource) {
        Some(e) => e,
        None => return false,
    };

    let collection = if let Some(idx) = index {
        get_indexed_value_compiled(entity, attribute, idx, interner)
    } else {
        entity.get_attribute(attribute).cloned()
    };

    if let Some(coll) = collection {
        match &coll {
            AttributeValue::List(items) => compiled_value_in_list(value, items),
            AttributeValue::Set(items) => compiled_value_in_set(value, items),
            _ => false,
        }
    } else {
        false
    }
}

/// Get an indexed value from an entity's collection attribute.
#[inline]
pub fn get_indexed_value_compiled(
    entity: &Entity,
    attribute: InternedString,
    index: &IndexExpr,
    interner: &StringInterner,
) -> Option<AttributeValue> {
    let collection = entity.get_attribute(attribute)?;

    match (collection, index) {
        // List[n] or Set iteration
        (AttributeValue::List(items), IndexExpr::Number(n)) => {
            let idx = if *n < 0 {
                // Negative indexing from end
                items.len().checked_sub((-*n) as usize)?
            } else {
                *n as usize
            };
            items.get(idx).cloned()
        }

        // Object["key"]
        (AttributeValue::Object(map), IndexExpr::String(key)) => {
            let key_interned = interner.intern(key);
            map.get(&key_interned).cloned()
        }

        // Wildcard - return first element
        (AttributeValue::List(items), IndexExpr::Wildcard) => items.first().cloned(),
        (AttributeValue::Set(items), IndexExpr::Wildcard) => items.iter().next().cloned(),

        _ => None,
    }
}

/// Evaluate indexed equals: entity.collection[index] == value
#[inline]
pub fn eval_indexed_equals(
    entity_type: &EntityType,
    attribute: InternedString,
    index: &IndexExpr,
    value: InternedString,
    user: &Entity,
    resource: &Entity,
    interner: &StringInterner,
) -> bool {
    let entity = match get_entity_for_type(entity_type, user, resource) {
        Some(e) => e,
        None => return false,
    };

    if matches!(index, IndexExpr::Wildcard) {
        // Wildcard: check if ANY element equals value
        if let Some(collection) = entity.get_attribute(attribute) {
            match collection {
                AttributeValue::List(items) => {
                    return items
                        .iter()
                        .any(|item| matches!(item, AttributeValue::String(s) if *s == value));
                }
                AttributeValue::Set(items) => {
                    return items.contains(&AttributeValue::String(value));
                }
                _ => return false,
            }
        }
        false
    } else {
        let indexed_val = get_indexed_value_compiled(entity, attribute, index, interner);
        matches!(indexed_val, Some(AttributeValue::String(actual)) if actual == value)
    }
}

/// Evaluate set intersection count > threshold
#[inline]
pub fn eval_set_intersection_count_gt(
    entity_type: &EntityType,
    attribute: InternedString,
    values: &[String],
    threshold: usize,
    user: &Entity,
    resource: &Entity,
    interner: &StringInterner,
) -> bool {
    let entity = match get_entity_for_type(entity_type, user, resource) {
        Some(e) => e,
        None => return false,
    };

    let collection = match entity.get_attribute(attribute) {
        Some(AttributeValue::List(items)) => items,
        Some(AttributeValue::Set(items)) => {
            // Convert set to vec for counting
            let vec: Vec<_> = items.iter().cloned().collect();
            return count_string_intersection_vec(&vec, values, interner) > threshold;
        }
        _ => return false,
    };

    count_string_intersection_vec(collection, values, interner) > threshold
}

/// Count intersection of a list with string values.
fn count_string_intersection_vec(
    items: &[AttributeValue],
    values: &[String],
    interner: &StringInterner,
) -> usize {
    items
        .iter()
        .filter(|item| {
            if let AttributeValue::String(s) = item {
                if let Some(resolved) = interner.resolve(*s) {
                    return values.iter().any(|v| v == &*resolved);
                }
            }
            false
        })
        .count()
}

/// Evaluate map key exists: "key" in entity.map.keys()
#[inline]
pub fn eval_map_key_exists(
    entity_type: &EntityType,
    attribute: InternedString,
    key: &str,
    user: &Entity,
    resource: &Entity,
    interner: &StringInterner,
) -> bool {
    let entity = match get_entity_for_type(entity_type, user, resource) {
        Some(e) => e,
        None => return false,
    };

    match entity.get_attribute(attribute) {
        Some(AttributeValue::Object(map)) => {
            let key_interned = interner.intern(key);
            map.contains_key(&key_interned)
        }
        _ => false,
    }
}

// ============================================================================
// Type Check Functions
// ============================================================================

/// Check if an entity attribute is a string
#[inline]
pub fn eval_is_string(
    entity_type: &EntityType,
    attribute: InternedString,
    user: &Entity,
    resource: &Entity,
) -> bool {
    let entity = match get_entity_for_type(entity_type, user, resource) {
        Some(e) => e,
        None => return false,
    };
    matches!(entity.get_attribute(attribute), Some(AttributeValue::String(_)))
}

/// Check if an entity attribute is a number (int or float)
#[inline]
pub fn eval_is_number(
    entity_type: &EntityType,
    attribute: InternedString,
    user: &Entity,
    resource: &Entity,
) -> bool {
    let entity = match get_entity_for_type(entity_type, user, resource) {
        Some(e) => e,
        None => return false,
    };
    matches!(
        entity.get_attribute(attribute),
        Some(AttributeValue::Int(_)) | Some(AttributeValue::Float(_))
    )
}

/// Check if an entity attribute is a boolean
#[inline]
pub fn eval_is_bool(
    entity_type: &EntityType,
    attribute: InternedString,
    user: &Entity,
    resource: &Entity,
) -> bool {
    let entity = match get_entity_for_type(entity_type, user, resource) {
        Some(e) => e,
        None => return false,
    };
    matches!(entity.get_attribute(attribute), Some(AttributeValue::Bool(_)))
}

/// Evaluate set intersection count > threshold with InternedString values
#[inline]
pub fn eval_set_intersection_count_greater(
    entity_type: &EntityType,
    attribute: InternedString,
    values: &[InternedString],
    threshold: usize,
    user: &Entity,
    resource: &Entity,
) -> bool {
    let entity = match get_entity_for_type(entity_type, user, resource) {
        Some(e) => e,
        None => return false,
    };

    match entity.get_attribute(attribute) {
        Some(AttributeValue::Set(set)) => {
            let count = values
                .iter()
                .filter(|v| set.contains(&AttributeValue::String(**v)))
                .count();
            count > threshold
        }
        Some(AttributeValue::List(list)) => {
            let count = values
                .iter()
                .filter(|v| {
                    list.iter()
                        .any(|item| matches!(item, AttributeValue::String(s) if *s == **v))
                })
                .count();
            count > threshold
        }
        _ => false,
    }
}

/// Evaluate map key exists with InternedString key
#[inline]
pub fn eval_map_key_exists_interned(
    entity_type: &EntityType,
    attribute: InternedString,
    key: &InternedString,
    user: &Entity,
    resource: &Entity,
) -> bool {
    let entity = match get_entity_for_type(entity_type, user, resource) {
        Some(e) => e,
        None => return false,
    };

    match entity.get_attribute(attribute) {
        Some(AttributeValue::Object(map)) => map.contains_key(key),
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

        let roles_key = interner.intern("roles");
        let admin = interner.intern("admin");
        let viewer = interner.intern("viewer");
        let editor = interner.intern("editor");

        let scores_key = interner.intern("scores");

        let metadata_key = interner.intern("metadata");
        let dept_key = interner.intern("department");
        let eng_val = interner.intern("engineering");

        let mut attrs: HashMap<InternedString, AttributeValue> = HashMap::new();

        // Roles as list
        attrs.insert(
            roles_key,
            AttributeValue::List(vec![
                AttributeValue::String(admin),
                AttributeValue::String(viewer),
                AttributeValue::String(editor),
            ]),
        );

        // Scores as list of ints
        attrs.insert(
            scores_key,
            AttributeValue::List(vec![
                AttributeValue::Int(85),
                AttributeValue::Int(90),
                AttributeValue::Int(78),
            ]),
        );

        // Metadata as object
        let mut metadata: HashMap<InternedString, AttributeValue> = HashMap::new();
        metadata.insert(dept_key, AttributeValue::String(eng_val));
        attrs.insert(metadata_key, AttributeValue::Object(metadata));

        Entity::new(user_id, user_type, attrs)
    }

    fn create_test_resource(interner: &StringInterner) -> Entity {
        let resource_id = interner.intern("resource_doc1");
        let resource_type = interner.intern("Resource");
        Entity::new(resource_id, resource_type, HashMap::new())
    }

    #[test]
    fn test_eval_count_gte() {
        let interner = create_test_interner();
        let user = create_test_user(&interner);
        let resource = create_test_resource(&interner);

        let roles_key = interner.intern("roles");

        assert!(eval_count_gte(
            &EntityType::User,
            roles_key,
            3,
            &user,
            &resource
        ));
        assert!(eval_count_gte(
            &EntityType::User,
            roles_key,
            2,
            &user,
            &resource
        ));
        assert!(!eval_count_gte(
            &EntityType::User,
            roles_key,
            4,
            &user,
            &resource
        ));
    }

    #[test]
    fn test_eval_count_gt() {
        let interner = create_test_interner();
        let user = create_test_user(&interner);
        let resource = create_test_resource(&interner);

        let roles_key = interner.intern("roles");

        assert!(eval_count_gt(
            &EntityType::User,
            roles_key,
            2,
            &user,
            &resource
        ));
        assert!(!eval_count_gt(
            &EntityType::User,
            roles_key,
            3,
            &user,
            &resource
        ));
    }

    #[test]
    fn test_eval_count_eq() {
        let interner = create_test_interner();
        let user = create_test_user(&interner);
        let resource = create_test_resource(&interner);

        let roles_key = interner.intern("roles");

        assert!(eval_count_eq(
            &EntityType::User,
            roles_key,
            3,
            &user,
            &resource
        ));
        assert!(!eval_count_eq(
            &EntityType::User,
            roles_key,
            2,
            &user,
            &resource
        ));
    }

    #[test]
    fn test_compiled_value_in_list_string() {
        let interner = create_test_interner();
        let admin = interner.intern("admin");
        let unknown = interner.intern("unknown");

        let items = vec![
            AttributeValue::String(admin),
            AttributeValue::String(interner.intern("viewer")),
        ];

        assert!(compiled_value_in_list(
            &CompiledLiteralValue::String(admin),
            &items
        ));
        assert!(!compiled_value_in_list(
            &CompiledLiteralValue::String(unknown),
            &items
        ));
    }

    #[test]
    fn test_compiled_value_in_list_int() {
        let items = vec![
            AttributeValue::Int(1),
            AttributeValue::Int(2),
            AttributeValue::Int(3),
        ];

        assert!(compiled_value_in_list(&CompiledLiteralValue::Int(2), &items));
        assert!(!compiled_value_in_list(&CompiledLiteralValue::Int(4), &items));
    }

    #[test]
    fn test_compiled_value_in_set() {
        let interner = create_test_interner();
        let admin = interner.intern("admin");

        let mut items: FxHashSet<AttributeValue> = FxHashSet::default();
        items.insert(AttributeValue::String(admin));
        items.insert(AttributeValue::Int(42));

        assert!(compiled_value_in_set(
            &CompiledLiteralValue::String(admin),
            &items
        ));
        assert!(compiled_value_in_set(&CompiledLiteralValue::Int(42), &items));
        assert!(!compiled_value_in_set(&CompiledLiteralValue::Int(99), &items));
    }

    #[test]
    fn test_get_indexed_value_compiled_list() {
        let interner = create_test_interner();
        let user = create_test_user(&interner);

        let scores_key = interner.intern("scores");

        // Get first element
        let result = get_indexed_value_compiled(&user, scores_key, &IndexExpr::Number(0), &interner);
        assert_eq!(result, Some(AttributeValue::Int(85)));

        // Get last element with negative index
        let result =
            get_indexed_value_compiled(&user, scores_key, &IndexExpr::Number(-1), &interner);
        assert_eq!(result, Some(AttributeValue::Int(78)));
    }

    #[test]
    fn test_get_indexed_value_compiled_object() {
        let interner = create_test_interner();
        let user = create_test_user(&interner);

        let metadata_key = interner.intern("metadata");

        let result = get_indexed_value_compiled(
            &user,
            metadata_key,
            &IndexExpr::String("department".to_string()),
            &interner,
        );
        assert!(matches!(result, Some(AttributeValue::String(_))));
    }

    #[test]
    fn test_eval_indexed_equals() {
        let interner = create_test_interner();
        let user = create_test_user(&interner);
        let resource = create_test_resource(&interner);

        let roles_key = interner.intern("roles");
        let admin = interner.intern("admin");

        // roles[0] == "admin"
        assert!(eval_indexed_equals(
            &EntityType::User,
            roles_key,
            &IndexExpr::Number(0),
            admin,
            &user,
            &resource,
            &interner
        ));
    }

    #[test]
    fn test_eval_indexed_equals_wildcard() {
        let interner = create_test_interner();
        let user = create_test_user(&interner);
        let resource = create_test_resource(&interner);

        let roles_key = interner.intern("roles");
        let admin = interner.intern("admin");
        let unknown = interner.intern("unknown");

        // roles[_] == "admin" (any role is admin)
        assert!(eval_indexed_equals(
            &EntityType::User,
            roles_key,
            &IndexExpr::Wildcard,
            admin,
            &user,
            &resource,
            &interner
        ));

        // roles[_] == "unknown" (no role is unknown)
        assert!(!eval_indexed_equals(
            &EntityType::User,
            roles_key,
            &IndexExpr::Wildcard,
            unknown,
            &user,
            &resource,
            &interner
        ));
    }

    #[test]
    fn test_eval_map_key_exists() {
        let interner = create_test_interner();
        let user = create_test_user(&interner);
        let resource = create_test_resource(&interner);

        let metadata_key = interner.intern("metadata");

        assert!(eval_map_key_exists(
            &EntityType::User,
            metadata_key,
            "department",
            &user,
            &resource,
            &interner
        ));

        assert!(!eval_map_key_exists(
            &EntityType::User,
            metadata_key,
            "unknown_key",
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

        let roles_key = interner.intern("roles");

        assert!(!eval_count_gte(
            &EntityType::Context,
            roles_key,
            1,
            &user,
            &resource
        ));
    }

    #[test]
    fn test_missing_attribute_returns_false() {
        let interner = create_test_interner();
        let user = create_test_user(&interner);
        let resource = create_test_resource(&interner);

        let unknown_key = interner.intern("unknown");

        assert!(!eval_count_gte(
            &EntityType::User,
            unknown_key,
            1,
            &user,
            &resource
        ));
    }
}
