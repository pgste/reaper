//! Tests for expression evaluation helpers.

use super::expr_eval::*;
use super::types::{EntityBindings, EntityType};
use crate::data::{AttributeValue, Entity, InternedString, StringInterner};
use std::collections::HashMap;
use std::sync::Arc;

fn create_test_interner() -> Arc<StringInterner> {
    Arc::new(StringInterner::new())
}

fn create_test_user(interner: &StringInterner) -> Entity {
    let user_id = interner.intern("user_alice");
    let user_type = interner.intern("User");

    let name_key = interner.intern("name");
    let name_val = interner.intern("  Alice Smith  ");

    let email_key = interner.intern("email");
    let email_val = interner.intern("alice@example.com");

    let scores_key = interner.intern("scores");

    let tags_key = interner.intern("tags");
    let tag1 = interner.intern("admin");
    let tag2 = interner.intern("developer");
    let tag3 = interner.intern("admin"); // duplicate

    let metadata_key = interner.intern("metadata");
    let dept_key = interner.intern("department");
    let dept_val = interner.intern("engineering");
    let level_key = interner.intern("level");

    let mut attrs: HashMap<InternedString, AttributeValue> = HashMap::new();
    attrs.insert(name_key, AttributeValue::String(name_val));
    attrs.insert(email_key, AttributeValue::String(email_val));
    attrs.insert(
        scores_key,
        AttributeValue::List(vec![
            AttributeValue::Int(85),
            AttributeValue::Int(90),
            AttributeValue::Int(78),
            AttributeValue::Int(92),
        ]),
    );
    attrs.insert(
        tags_key,
        AttributeValue::List(vec![
            AttributeValue::String(tag1),
            AttributeValue::String(tag2),
            AttributeValue::String(tag3),
        ]),
    );

    // Metadata object
    let mut metadata: HashMap<InternedString, AttributeValue> = HashMap::new();
    metadata.insert(dept_key, AttributeValue::String(dept_val));
    metadata.insert(level_key, AttributeValue::Int(3));
    attrs.insert(metadata_key, AttributeValue::Object(metadata));

    Entity::new(user_id, user_type, attrs)
}

fn create_test_resource(interner: &StringInterner) -> Entity {
    let resource_id = interner.intern("resource_doc1");
    let resource_type = interner.intern("Resource");
    Entity::new(resource_id, resource_type, HashMap::new())
}

#[test]
fn test_eval_string_lower() {
    let interner = create_test_interner();
    let user = create_test_user(&interner);
    let resource = create_test_resource(&interner);

    let name_key = interner.intern("name");

    let result = eval_string_lower(
        &EntityType::User,
        name_key,
        EntityBindings {
            user: &user,
            actor: None,
            resource: &resource,
        },
        &interner,
    );
    assert!(result.is_some());
    if let Some(AttributeValue::String(s)) = result {
        let resolved = interner.resolve(s).unwrap();
        assert_eq!(&*resolved, "  alice smith  ");
    } else {
        panic!("Expected String");
    }
}

#[test]
fn test_eval_string_upper() {
    let interner = create_test_interner();
    let user = create_test_user(&interner);
    let resource = create_test_resource(&interner);

    let name_key = interner.intern("name");

    let result = eval_string_upper(
        &EntityType::User,
        name_key,
        EntityBindings {
            user: &user,
            actor: None,
            resource: &resource,
        },
        &interner,
    );
    assert!(result.is_some());
    if let Some(AttributeValue::String(s)) = result {
        let resolved = interner.resolve(s).unwrap();
        assert_eq!(&*resolved, "  ALICE SMITH  ");
    } else {
        panic!("Expected String");
    }
}

#[test]
fn test_eval_string_trim() {
    let interner = create_test_interner();
    let user = create_test_user(&interner);
    let resource = create_test_resource(&interner);

    let name_key = interner.intern("name");

    let result = eval_string_trim(
        &EntityType::User,
        name_key,
        EntityBindings {
            user: &user,
            actor: None,
            resource: &resource,
        },
        &interner,
    );
    assert!(result.is_some());
    if let Some(AttributeValue::String(s)) = result {
        let resolved = interner.resolve(s).unwrap();
        assert_eq!(&*resolved, "Alice Smith");
    } else {
        panic!("Expected String");
    }
}

#[test]
fn test_eval_string_split() {
    let interner = create_test_interner();
    let user = create_test_user(&interner);
    let resource = create_test_resource(&interner);

    let email_key = interner.intern("email");

    let result = eval_string_split(
        &EntityType::User,
        email_key,
        "@",
        EntityBindings {
            user: &user,
            actor: None,
            resource: &resource,
        },
        &interner,
    );
    assert!(result.is_some());
    if let Some(AttributeValue::List(parts)) = result {
        assert_eq!(parts.len(), 2);
    } else {
        panic!("Expected List");
    }
}

#[test]
fn test_eval_string_replace() {
    let interner = create_test_interner();
    let user = create_test_user(&interner);
    let resource = create_test_resource(&interner);

    let email_key = interner.intern("email");

    let result = eval_string_replace(
        &EntityType::User,
        email_key,
        "example",
        "test",
        EntityBindings {
            user: &user,
            actor: None,
            resource: &resource,
        },
        &interner,
    );
    assert!(result.is_some());
    if let Some(AttributeValue::String(s)) = result {
        let resolved = interner.resolve(s).unwrap();
        assert_eq!(&*resolved, "alice@test.com");
    } else {
        panic!("Expected String");
    }
}

#[test]
fn test_eval_collection_count() {
    let interner = create_test_interner();
    let user = create_test_user(&interner);
    let resource = create_test_resource(&interner);

    let scores_key = interner.intern("scores");

    let result = eval_collection_count(
        &EntityType::User,
        scores_key,
        EntityBindings {
            user: &user,
            actor: None,
            resource: &resource,
        },
        &interner,
    );
    assert_eq!(result, Some(AttributeValue::Int(4)));
}

#[test]
fn test_eval_string_count() {
    let interner = create_test_interner();
    let user = create_test_user(&interner);
    let resource = create_test_resource(&interner);

    let email_key = interner.intern("email");

    // "alice@example.com" has 17 characters
    let result = eval_collection_count(
        &EntityType::User,
        email_key,
        EntityBindings {
            user: &user,
            actor: None,
            resource: &resource,
        },
        &interner,
    );
    assert_eq!(result, Some(AttributeValue::Int(17)));
}

#[test]
fn test_eval_collection_sum() {
    let interner = create_test_interner();
    let user = create_test_user(&interner);
    let resource = create_test_resource(&interner);

    let scores_key = interner.intern("scores");

    let result = eval_collection_sum(
        &EntityType::User,
        scores_key,
        EntityBindings {
            user: &user,
            actor: None,
            resource: &resource,
        },
    );
    // 85 + 90 + 78 + 92 = 345
    assert_eq!(result, Some(AttributeValue::Int(345)));
}

#[test]
fn test_eval_collection_min() {
    let interner = create_test_interner();
    let user = create_test_user(&interner);
    let resource = create_test_resource(&interner);

    let scores_key = interner.intern("scores");

    let result = eval_collection_min(
        &EntityType::User,
        scores_key,
        EntityBindings {
            user: &user,
            actor: None,
            resource: &resource,
        },
    );
    assert_eq!(result, Some(AttributeValue::Int(78)));
}

#[test]
fn test_eval_collection_max() {
    let interner = create_test_interner();
    let user = create_test_user(&interner);
    let resource = create_test_resource(&interner);

    let scores_key = interner.intern("scores");

    let result = eval_collection_max(
        &EntityType::User,
        scores_key,
        EntityBindings {
            user: &user,
            actor: None,
            resource: &resource,
        },
    );
    assert_eq!(result, Some(AttributeValue::Int(92)));
}

#[test]
fn test_eval_collection_average() {
    let interner = create_test_interner();
    let user = create_test_user(&interner);
    let resource = create_test_resource(&interner);

    let scores_key = interner.intern("scores");

    let result = eval_collection_average(
        &EntityType::User,
        scores_key,
        EntityBindings {
            user: &user,
            actor: None,
            resource: &resource,
        },
    );
    // 345 / 4 = 86.25
    assert_eq!(result, Some(AttributeValue::Float(86.25)));
}

#[test]
fn test_eval_collection_first() {
    let interner = create_test_interner();
    let user = create_test_user(&interner);
    let resource = create_test_resource(&interner);

    let scores_key = interner.intern("scores");

    let result = eval_collection_first(
        &EntityType::User,
        scores_key,
        EntityBindings {
            user: &user,
            actor: None,
            resource: &resource,
        },
    );
    assert_eq!(result, Some(AttributeValue::Int(85)));
}

#[test]
fn test_eval_collection_last() {
    let interner = create_test_interner();
    let user = create_test_user(&interner);
    let resource = create_test_resource(&interner);

    let scores_key = interner.intern("scores");

    let result = eval_collection_last(
        &EntityType::User,
        scores_key,
        EntityBindings {
            user: &user,
            actor: None,
            resource: &resource,
        },
    );
    assert_eq!(result, Some(AttributeValue::Int(92)));
}

#[test]
fn test_eval_map_keys() {
    let interner = create_test_interner();
    let user = create_test_user(&interner);
    let resource = create_test_resource(&interner);

    let metadata_key = interner.intern("metadata");

    let result = eval_map_keys(
        &EntityType::User,
        metadata_key,
        EntityBindings {
            user: &user,
            actor: None,
            resource: &resource,
        },
    );
    assert!(result.is_some());
    if let Some(AttributeValue::List(keys)) = result {
        assert_eq!(keys.len(), 2);
    } else {
        panic!("Expected List");
    }
}

#[test]
fn test_eval_map_values() {
    let interner = create_test_interner();
    let user = create_test_user(&interner);
    let resource = create_test_resource(&interner);

    let metadata_key = interner.intern("metadata");

    let result = eval_map_values(
        &EntityType::User,
        metadata_key,
        EntityBindings {
            user: &user,
            actor: None,
            resource: &resource,
        },
    );
    assert!(result.is_some());
    if let Some(AttributeValue::List(values)) = result {
        assert_eq!(values.len(), 2);
    } else {
        panic!("Expected List");
    }
}

#[test]
fn test_eval_variable_ref() {
    let interner = create_test_interner();
    let var_name = interner.intern("my_var");
    let var_val = interner.intern("hello");

    let mut variables: HashMap<String, AttributeValue> = HashMap::new();
    variables.insert("my_var".to_string(), AttributeValue::String(var_val));

    let result = eval_variable_ref(var_name, &variables, &interner);
    assert_eq!(result, Some(AttributeValue::String(var_val)));
}

#[test]
fn test_eval_variable_ref_not_found() {
    let interner = create_test_interner();
    let var_name = interner.intern("unknown_var");
    let variables: HashMap<String, AttributeValue> = HashMap::new();

    let result = eval_variable_ref(var_name, &variables, &interner);
    assert_eq!(result, None);
}

#[test]
fn test_eval_indexed_access() {
    let interner = create_test_interner();
    let user = create_test_user(&interner);
    let resource = create_test_resource(&interner);

    let scores_key = interner.intern("scores");

    // First element
    let result = eval_indexed_access(
        &EntityType::User,
        scores_key,
        0,
        EntityBindings {
            user: &user,
            actor: None,
            resource: &resource,
        },
    );
    assert_eq!(result, Some(AttributeValue::Int(85)));

    // Negative index (last element)
    let result = eval_indexed_access(
        &EntityType::User,
        scores_key,
        -1,
        EntityBindings {
            user: &user,
            actor: None,
            resource: &resource,
        },
    );
    assert_eq!(result, Some(AttributeValue::Int(92)));

    // Out of bounds
    let result = eval_indexed_access(
        &EntityType::User,
        scores_key,
        100,
        EntityBindings {
            user: &user,
            actor: None,
            resource: &resource,
        },
    );
    assert_eq!(result, None);
}

#[test]
fn test_eval_map_access() {
    let interner = create_test_interner();
    let user = create_test_user(&interner);
    let resource = create_test_resource(&interner);

    let metadata_key = interner.intern("metadata");

    let result = eval_map_access(
        &EntityType::User,
        metadata_key,
        "level",
        EntityBindings {
            user: &user,
            actor: None,
            resource: &resource,
        },
        &interner,
    );
    assert_eq!(result, Some(AttributeValue::Int(3)));

    let result = eval_map_access(
        &EntityType::User,
        metadata_key,
        "unknown",
        EntityBindings {
            user: &user,
            actor: None,
            resource: &resource,
        },
        &interner,
    );
    assert_eq!(result, None);
}

#[test]
fn test_eval_collection_unique() {
    let interner = create_test_interner();
    let user = create_test_user(&interner);
    let resource = create_test_resource(&interner);

    let tags_key = interner.intern("tags");

    let result = eval_collection_unique(
        &EntityType::User,
        tags_key,
        EntityBindings {
            user: &user,
            actor: None,
            resource: &resource,
        },
    );
    assert!(result.is_some());
    if let Some(AttributeValue::List(items)) = result {
        // Should have 2 unique tags (admin appears twice)
        assert_eq!(items.len(), 2);
    } else {
        panic!("Expected List");
    }
}

#[test]
fn test_eval_collection_sort() {
    let interner = create_test_interner();
    let user = create_test_user(&interner);
    let resource = create_test_resource(&interner);

    let scores_key = interner.intern("scores");

    let result = eval_collection_sort(
        &EntityType::User,
        scores_key,
        EntityBindings {
            user: &user,
            actor: None,
            resource: &resource,
        },
        &interner,
    );
    assert!(result.is_some());
    if let Some(AttributeValue::List(items)) = result {
        assert_eq!(items.len(), 4);
        // Sorted: 78, 85, 90, 92
        assert_eq!(items[0], AttributeValue::Int(78));
        assert_eq!(items[1], AttributeValue::Int(85));
        assert_eq!(items[2], AttributeValue::Int(90));
        assert_eq!(items[3], AttributeValue::Int(92));
    } else {
        panic!("Expected List");
    }
}

#[test]
fn test_context_entity_type_returns_none() {
    let interner = create_test_interner();
    let user = create_test_user(&interner);
    let resource = create_test_resource(&interner);

    let name_key = interner.intern("name");

    let result = eval_string_lower(
        &EntityType::Context,
        name_key,
        EntityBindings {
            user: &user,
            actor: None,
            resource: &resource,
        },
        &interner,
    );
    assert_eq!(result, None);
}

#[test]
fn test_missing_attribute_returns_none() {
    let interner = create_test_interner();
    let user = create_test_user(&interner);
    let resource = create_test_resource(&interner);

    let unknown_key = interner.intern("unknown");

    let result = eval_string_lower(
        &EntityType::User,
        unknown_key,
        EntityBindings {
            user: &user,
            actor: None,
            resource: &resource,
        },
        &interner,
    );
    assert_eq!(result, None);
}
