//! Entity attribute comparison evaluation.
//!
//! This module handles comparisons between entity attributes and literal values,
//! as well as cross-entity attribute comparisons.
//!
//! ## Performance Characteristics
//! - All comparisons use pre-interned strings (5ns vs 100ns with HashMap lookup)
//! - Numeric comparisons are direct f64/i64 operations
//! - String comparisons use InternedString equality (pointer comparison)

// Allow unused functions - some are used in tests only or reserved for future V1 compatibility
#![allow(dead_code)]

use crate::data::{AttributeValue, InternedString, StringInterner};

use super::entity_helpers::{get_nested_attr, get_numeric_attr};
use super::types::{
    AttrCompareOp, CompiledAttributeComparison, CompiledCompareTarget,
    CompiledCrossEntityComparison, CompiledWildcardComparison, EntityBindings, EntityType,
    NumericOp,
};

// ============================================================================
// V2 Dispatch Functions
// ============================================================================

/// Evaluate a V2 attribute comparison
#[inline]
pub fn eval_attribute_comparison(
    comp: &CompiledAttributeComparison,
    bindings: EntityBindings<'_>,
    interner: &StringInterner,
) -> bool {
    match &comp.target {
        // TYPE-STRICT TOTAL COMPARISONS (see docs/development/CORRECTNESS.md):
        // a literal only matches a value of its own type (Int/Float cross-
        // numeric excepted). `==` on a type mismatch is false; `!=` on a
        // PRESENT scalar of a different type is true (the values differ);
        // missing/Null satisfies neither. No coercion — `user.level == "5"`
        // does NOT match Int(5); it did before, which made the compiled
        // evaluator disagree with the AST and the oracle.
        CompiledCompareTarget::LiteralNum(threshold) => {
            match get_nested_attr(&comp.entity_type, comp.attribute, bindings, interner) {
                Some(AttributeValue::Int(n)) => compare_f64(n as f64, *threshold, &comp.op),
                Some(AttributeValue::Float(f)) => compare_f64(f, *threshold, &comp.op),
                Some(AttributeValue::Null) | None => false,
                // Present non-numeric scalar: differs from any number.
                Some(AttributeValue::String(_)) | Some(AttributeValue::Bool(_)) => {
                    matches!(comp.op, NumericOp::NotEqual)
                }
                Some(_) => false, // collections/objects: not comparable
            }
        }
        CompiledCompareTarget::LiteralString(expected) => {
            match get_nested_attr(&comp.entity_type, comp.attribute, bindings, interner) {
                Some(AttributeValue::String(actual)) => match comp.op {
                    NumericOp::Equal => actual == *expected,
                    NumericOp::NotEqual => actual != *expected,
                    _ => false, // ordering doesn't apply to strings
                },
                Some(AttributeValue::Null) | None => false,
                Some(AttributeValue::Bool(_))
                | Some(AttributeValue::Int(_))
                | Some(AttributeValue::Float(_)) => matches!(comp.op, NumericOp::NotEqual),
                Some(_) => false,
            }
        }
        CompiledCompareTarget::LiteralBool(expected) => {
            match get_nested_attr(&comp.entity_type, comp.attribute, bindings, interner) {
                Some(AttributeValue::Bool(actual)) => match comp.op {
                    NumericOp::Equal => actual == *expected,
                    NumericOp::NotEqual => actual != *expected,
                    _ => false,
                },
                Some(AttributeValue::Null) | None => false,
                Some(AttributeValue::String(_))
                | Some(AttributeValue::Int(_))
                | Some(AttributeValue::Float(_)) => matches!(comp.op, NumericOp::NotEqual),
                Some(_) => false,
            }
        }
        CompiledCompareTarget::LiteralNull => {
            // Handle null comparisons: attr == null or attr != null
            let attr_val = get_nested_attr(&comp.entity_type, comp.attribute, bindings, interner);
            let is_null = match &attr_val {
                None => true,                       // Missing attribute is null
                Some(AttributeValue::Null) => true, // Explicit null
                _ => false,                         // Has a value, not null
            };
            match comp.op {
                NumericOp::Equal => is_null,     // attr == null -> true if null
                NumericOp::NotEqual => !is_null, // attr != null -> true if not null
                _ => false,                      // Other ops don't make sense for null
            }
        }
        CompiledCompareTarget::EntityAttr {
            entity_type: other_entity_type,
            attribute: other_attr,
        } => {
            let left_val = get_nested_attr(&comp.entity_type, comp.attribute, bindings, interner);
            let right_val = get_nested_attr(other_entity_type, *other_attr, bindings, interner);
            compare_attr_values(left_val.as_ref(), right_val.as_ref(), &comp.op.into())
        }
        CompiledCompareTarget::Variable(_var) => {
            // Variable comparison would need access to variables map
            // For now, return false - caller should handle this case
            false
        }
    }
}

/// Evaluate a V2 cross-entity comparison
#[inline]
pub fn eval_cross_entity_comparison(
    comp: &CompiledCrossEntityComparison,
    bindings: EntityBindings<'_>,
    interner: &StringInterner,
) -> bool {
    let left_val = get_nested_attr(&comp.left_entity, comp.left_attr, bindings, interner);
    let right_val = get_nested_attr(&comp.right_entity, comp.right_attr, bindings, interner);
    compare_attr_values(left_val.as_ref(), right_val.as_ref(), &comp.op.into())
}

/// Evaluate a V2 wildcard comparison
#[inline]
pub fn eval_wildcard_comparison(
    comp: &CompiledWildcardComparison,
    bindings: EntityBindings<'_>,
    interner: &StringInterner,
) -> bool {
    let collection = get_nested_attr(
        &comp.collection_entity,
        comp.collection_attr,
        bindings,
        interner,
    );
    let scalar_val = get_nested_attr(&comp.scalar_entity, comp.scalar_attr, bindings, interner);

    // NULL SEMANTICS: `matched` is None when either side is MISSING (or the
    // left side is not a collection / right side not a scalar). In that case
    // BOTH `==` and `!=` fail — absence must never satisfy a guard (fail
    // closed). When both sides are present, existential equality is typed
    // and total: a scalar of a non-matching element type simply matches no
    // element (== false, != true), same as the AST's values_equal.
    let matched = match (collection, scalar_val) {
        (Some(AttributeValue::List(items)), Some(scalar)) if is_scalar_attr(&scalar) => {
            Some(items.iter().any(|item| item == &scalar))
        }
        (Some(AttributeValue::Set(items)), Some(scalar)) if is_scalar_attr(&scalar) => {
            Some(items.contains(&scalar))
        }
        _ => None,
    };
    match matched {
        Some(m) => m != comp.negated,
        None => false,
    }
}

/// Helper for f64 comparisons with a given operator.
#[inline]
fn compare_f64(left: f64, right: f64, op: &NumericOp) -> bool {
    match op {
        NumericOp::Equal => (left - right).abs() < f64::EPSILON,
        NumericOp::NotEqual => (left - right).abs() >= f64::EPSILON,
        NumericOp::Less => left < right,
        NumericOp::LessEqual => left <= right,
        NumericOp::Greater => left > right,
        NumericOp::GreaterEqual => left >= right,
    }
}

// ============================================================================
// Legacy Functions (still used by some code paths)
// ============================================================================

/// Evaluate User/Resource attribute equals literal string.
///
/// Handles type coercion for Bool and Int -> String comparisons.
/// Supports nested attributes like "config.name".
#[inline]
pub fn eval_attr_equals_literal(
    entity_type: &EntityType,
    attribute: InternedString,
    value: InternedString,
    bindings: EntityBindings<'_>,
    interner: &StringInterner,
) -> bool {
    match get_nested_attr(entity_type, attribute, bindings, interner) {
        Some(AttributeValue::String(actual)) => actual == value,
        Some(AttributeValue::Bool(actual)) => interner
            .resolve(value)
            .map(|v| &*v == actual.to_string().as_str())
            .unwrap_or(false),
        Some(AttributeValue::Int(actual)) => interner
            .resolve(value)
            .map(|v| &*v == actual.to_string().as_str())
            .unwrap_or(false),
        _ => false,
    }
}

/// Evaluate User/Resource attribute >= literal numeric value.
#[inline]
pub fn eval_attr_gte_literal(
    entity_type: &EntityType,
    attribute: InternedString,
    threshold: f64,
    bindings: EntityBindings<'_>,
) -> bool {
    get_numeric_attr(entity_type, attribute, bindings)
        .map(|actual| actual >= threshold)
        .unwrap_or(false)
}

/// Evaluate User/Resource attribute > literal numeric value.
#[inline]
pub fn eval_attr_gt_literal(
    entity_type: &EntityType,
    attribute: InternedString,
    threshold: f64,
    bindings: EntityBindings<'_>,
) -> bool {
    get_numeric_attr(entity_type, attribute, bindings)
        .map(|actual| actual > threshold)
        .unwrap_or(false)
}

/// Evaluate User/Resource attribute <= literal numeric value.
#[inline]
pub fn eval_attr_lte_literal(
    entity_type: &EntityType,
    attribute: InternedString,
    threshold: f64,
    bindings: EntityBindings<'_>,
) -> bool {
    get_numeric_attr(entity_type, attribute, bindings)
        .map(|actual| actual <= threshold)
        .unwrap_or(false)
}

/// Evaluate User/Resource attribute < literal numeric value.
#[inline]
pub fn eval_attr_lt_literal(
    entity_type: &EntityType,
    attribute: InternedString,
    threshold: f64,
    bindings: EntityBindings<'_>,
) -> bool {
    get_numeric_attr(entity_type, attribute, bindings)
        .map(|actual| actual < threshold)
        .unwrap_or(false)
}

/// Evaluate user.attr == resource.attr (cross-entity comparison).
/// Supports nested attributes like "config.name".
#[inline]
pub fn eval_user_equals_resource(
    user_attr: InternedString,
    resource_attr: InternedString,
    bindings: EntityBindings<'_>,
    interner: &StringInterner,
) -> bool {
    let user_val = get_nested_attr(&EntityType::User, user_attr, bindings, interner);
    let resource_val = get_nested_attr(&EntityType::Resource, resource_attr, bindings, interner);
    match (user_val, resource_val) {
        (Some(AttributeValue::String(u)), Some(AttributeValue::String(r))) => u == r,
        (Some(AttributeValue::Int(u)), Some(AttributeValue::Int(r))) => u == r,
        (Some(AttributeValue::Bool(u)), Some(AttributeValue::Bool(r))) => u == r,
        _ => false,
    }
}

/// Evaluate user.attr > resource.attr (integer comparison).
/// Supports nested attributes like "config.level".
#[inline]
pub fn eval_user_int_greater(
    user_attr: InternedString,
    resource_attr: InternedString,
    bindings: EntityBindings<'_>,
    interner: &StringInterner,
) -> bool {
    let user_val = get_nested_attr(&EntityType::User, user_attr, bindings, interner);
    let resource_val = get_nested_attr(&EntityType::Resource, resource_attr, bindings, interner);
    match (user_val, resource_val) {
        (Some(AttributeValue::Int(u)), Some(AttributeValue::Int(r))) => u > r,
        _ => false,
    }
}

/// Evaluate resource.attr > user.attr (integer comparison).
/// Supports nested attributes like "config.level".
#[inline]
pub fn eval_resource_int_greater(
    resource_attr: InternedString,
    user_attr: InternedString,
    bindings: EntityBindings<'_>,
    interner: &StringInterner,
) -> bool {
    let resource_val = get_nested_attr(&EntityType::Resource, resource_attr, bindings, interner);
    let user_val = get_nested_attr(&EntityType::User, user_attr, bindings, interner);
    match (resource_val, user_val) {
        (Some(AttributeValue::Int(r)), Some(AttributeValue::Int(u))) => r > u,
        _ => false,
    }
}

/// Evaluate wildcard collection membership: user.collection[_] == resource.scalar
///
/// Returns true if ANY element in user's collection equals resource's scalar value.
/// Supports nested attributes like "config.roles".
#[inline]
pub fn eval_user_wildcard_equals_resource(
    user_attr: InternedString,
    resource_attr: InternedString,
    bindings: EntityBindings<'_>,
    interner: &StringInterner,
) -> bool {
    let resource_val = get_nested_attr(&EntityType::Resource, resource_attr, bindings, interner);
    let user_collection = get_nested_attr(&EntityType::User, user_attr, bindings, interner);

    match (user_collection, resource_val) {
        (Some(AttributeValue::List(items)), Some(AttributeValue::String(expected))) => items
            .iter()
            .any(|item| matches!(item, AttributeValue::String(s) if *s == expected)),
        (Some(AttributeValue::Set(items)), Some(AttributeValue::String(expected))) => {
            items.contains(&AttributeValue::String(expected))
        }
        (Some(AttributeValue::List(items)), Some(AttributeValue::Int(expected))) => items
            .iter()
            .any(|item| matches!(item, AttributeValue::Int(i) if *i == expected)),
        (Some(AttributeValue::Set(items)), Some(AttributeValue::Int(expected))) => {
            items.contains(&AttributeValue::Int(expected))
        }
        _ => false,
    }
}

/// Evaluate wildcard collection membership: resource.collection[_] == user.scalar
///
/// Returns true if ANY element in resource's collection equals user's scalar value.
/// Supports nested attributes like "config.readers".
#[inline]
pub fn eval_resource_wildcard_equals_user(
    resource_attr: InternedString,
    user_attr: InternedString,
    bindings: EntityBindings<'_>,
    interner: &StringInterner,
) -> bool {
    let user_val = get_nested_attr(&EntityType::User, user_attr, bindings, interner);
    let resource_collection =
        get_nested_attr(&EntityType::Resource, resource_attr, bindings, interner);

    match (resource_collection, user_val) {
        (Some(AttributeValue::List(items)), Some(AttributeValue::String(expected))) => items
            .iter()
            .any(|item| matches!(item, AttributeValue::String(s) if *s == expected)),
        (Some(AttributeValue::Set(items)), Some(AttributeValue::String(expected))) => {
            items.contains(&AttributeValue::String(expected))
        }
        (Some(AttributeValue::List(items)), Some(AttributeValue::Int(expected))) => items
            .iter()
            .any(|item| matches!(item, AttributeValue::Int(i) if *i == expected)),
        (Some(AttributeValue::Set(items)), Some(AttributeValue::Int(expected))) => {
            items.contains(&AttributeValue::Int(expected))
        }
        _ => false,
    }
}

/// Evaluate same-entity attribute comparison: entity.attr1 op entity.attr2
/// Supports nested attributes like "config.level".
#[inline]
pub fn eval_same_entity_attr_compare(
    entity_type: &EntityType,
    left_attr: InternedString,
    right_attr: InternedString,
    op: &AttrCompareOp,
    bindings: EntityBindings<'_>,
    interner: &StringInterner,
) -> bool {
    let left_val = get_nested_attr(entity_type, left_attr, bindings, interner);
    let right_val = get_nested_attr(entity_type, right_attr, bindings, interner);

    compare_attr_values(left_val.as_ref(), right_val.as_ref(), op)
}

/// Compare two attribute values with the given operator.
#[inline]
pub fn compare_attr_values(
    left: Option<&AttributeValue>,
    right: Option<&AttributeValue>,
    op: &AttrCompareOp,
) -> bool {
    match (left, right, op) {
        // String comparisons
        (
            Some(AttributeValue::String(l)),
            Some(AttributeValue::String(r)),
            AttrCompareOp::Equal,
        ) => l == r,
        (
            Some(AttributeValue::String(l)),
            Some(AttributeValue::String(r)),
            AttrCompareOp::NotEqual,
        ) => l != r,

        // Int comparisons
        (Some(AttributeValue::Int(l)), Some(AttributeValue::Int(r)), AttrCompareOp::Equal) => {
            l == r
        }
        (Some(AttributeValue::Int(l)), Some(AttributeValue::Int(r)), AttrCompareOp::NotEqual) => {
            l != r
        }
        (Some(AttributeValue::Int(l)), Some(AttributeValue::Int(r)), AttrCompareOp::Less) => l < r,
        (Some(AttributeValue::Int(l)), Some(AttributeValue::Int(r)), AttrCompareOp::LessEqual) => {
            l <= r
        }
        (Some(AttributeValue::Int(l)), Some(AttributeValue::Int(r)), AttrCompareOp::Greater) => {
            l > r
        }
        (
            Some(AttributeValue::Int(l)),
            Some(AttributeValue::Int(r)),
            AttrCompareOp::GreaterEqual,
        ) => l >= r,

        // Float comparisons
        (Some(AttributeValue::Float(l)), Some(AttributeValue::Float(r)), AttrCompareOp::Equal) => {
            (l - r).abs() < f64::EPSILON
        }
        (
            Some(AttributeValue::Float(l)),
            Some(AttributeValue::Float(r)),
            AttrCompareOp::NotEqual,
        ) => (l - r).abs() >= f64::EPSILON,
        (Some(AttributeValue::Float(l)), Some(AttributeValue::Float(r)), AttrCompareOp::Less) => {
            l < r
        }
        (
            Some(AttributeValue::Float(l)),
            Some(AttributeValue::Float(r)),
            AttrCompareOp::LessEqual,
        ) => l <= r,
        (
            Some(AttributeValue::Float(l)),
            Some(AttributeValue::Float(r)),
            AttrCompareOp::Greater,
        ) => l > r,
        (
            Some(AttributeValue::Float(l)),
            Some(AttributeValue::Float(r)),
            AttrCompareOp::GreaterEqual,
        ) => l >= r,

        // Bool comparisons
        (Some(AttributeValue::Bool(l)), Some(AttributeValue::Bool(r)), AttrCompareOp::Equal) => {
            l == r
        }
        (Some(AttributeValue::Bool(l)), Some(AttributeValue::Bool(r)), AttrCompareOp::NotEqual) => {
            l != r
        }

        // Cross-type int/float comparisons
        (Some(AttributeValue::Int(l)), Some(AttributeValue::Float(r)), op) => {
            let l_f64 = *l as f64;
            compare_floats(l_f64, *r, op)
        }
        (Some(AttributeValue::Float(l)), Some(AttributeValue::Int(r)), op) => {
            let r_f64 = *r as f64;
            compare_floats(*l, r_f64, op)
        }

        // TYPE-STRICT NotEqual: two PRESENT scalars of different types differ
        // by definition (matches AST !values_equal). Restricted to scalars —
        // collections keep their own (existential) semantics elsewhere.
        (Some(l), Some(r), AttrCompareOp::NotEqual) if is_scalar_attr(l) && is_scalar_attr(r) => {
            true
        }

        // Default: not comparable — includes anything missing or Null,
        // which satisfies neither == nor != (fail closed).
        _ => false,
    }
}

/// Scalar (non-collection, non-null) attribute values, for the type-strict
/// NotEqual rule above.
fn is_scalar_attr(v: &AttributeValue) -> bool {
    matches!(
        v,
        AttributeValue::String(_)
            | AttributeValue::Int(_)
            | AttributeValue::Float(_)
            | AttributeValue::Bool(_)
    )
}

/// Helper for float comparisons with a given operator.
#[inline]
fn compare_floats(l: f64, r: f64, op: &AttrCompareOp) -> bool {
    match op {
        AttrCompareOp::Equal => (l - r).abs() < f64::EPSILON,
        AttrCompareOp::NotEqual => (l - r).abs() >= f64::EPSILON,
        AttrCompareOp::Less => l < r,
        AttrCompareOp::LessEqual => l <= r,
        AttrCompareOp::Greater => l > r,
        AttrCompareOp::GreaterEqual => l >= r,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::Entity;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn create_test_interner() -> Arc<StringInterner> {
        Arc::new(StringInterner::new())
    }

    fn create_test_user(interner: &StringInterner) -> Entity {
        let user_id = interner.intern("user_alice");
        let user_type = interner.intern("User");

        let role_key = interner.intern("role");
        let level_key = interner.intern("level");
        let dept_key = interner.intern("department");
        let admin_val = interner.intern("admin");
        let eng_val = interner.intern("engineering");

        let roles_key = interner.intern("roles");
        let viewer_val = interner.intern("viewer");

        let mut attrs: HashMap<InternedString, AttributeValue> = HashMap::new();
        attrs.insert(role_key, AttributeValue::String(admin_val));
        attrs.insert(level_key, AttributeValue::Int(5));
        attrs.insert(dept_key, AttributeValue::String(eng_val));
        attrs.insert(
            roles_key,
            AttributeValue::List(vec![
                AttributeValue::String(admin_val),
                AttributeValue::String(viewer_val),
            ]),
        );

        Entity::new(user_id, user_type, attrs)
    }

    fn create_test_resource(interner: &StringInterner) -> Entity {
        let resource_id = interner.intern("resource_doc1");
        let resource_type = interner.intern("Resource");

        let owner_key = interner.intern("owner");
        let min_level_key = interner.intern("min_level");
        let dept_key = interner.intern("department");
        let alice_val = interner.intern("alice");
        let eng_val = interner.intern("engineering");

        let readers_key = interner.intern("readers");
        let admin_val = interner.intern("admin");

        let mut attrs: HashMap<InternedString, AttributeValue> = HashMap::new();
        attrs.insert(owner_key, AttributeValue::String(alice_val));
        attrs.insert(min_level_key, AttributeValue::Int(3));
        attrs.insert(dept_key, AttributeValue::String(eng_val));
        attrs.insert(
            readers_key,
            AttributeValue::List(vec![AttributeValue::String(admin_val)]),
        );

        Entity::new(resource_id, resource_type, attrs)
    }

    #[test]
    fn wildcard_not_equal_fails_closed_on_missing_attributes() {
        let interner = create_test_interner();
        let user = create_test_user(&interner); // roles = [admin, viewer]
        let resource = create_test_resource(&interner); // owner = "alice"

        let wildcard =
            |collection_attr: &str, scalar_attr: &str, negated: bool| CompiledWildcardComparison {
                collection_entity: EntityType::User,
                collection_attr: interner.intern(collection_attr),
                scalar_entity: EntityType::Resource,
                scalar_attr: interner.intern(scalar_attr),
                negated,
            };

        // Missing collection attribute: both == and != must FAIL.
        assert!(!eval_wildcard_comparison(
            &wildcard("nonexistent", "owner", false),
            EntityBindings {
                user: &user,
                actor: None,
                resource: &resource
            },
            &interner
        ));
        assert!(!eval_wildcard_comparison(
            &wildcard("nonexistent", "owner", true),
            EntityBindings {
                user: &user,
                actor: None,
                resource: &resource
            },
            &interner
        ));

        // Missing scalar attribute: both == and != must FAIL.
        assert!(!eval_wildcard_comparison(
            &wildcard("roles", "nonexistent", true),
            EntityBindings {
                user: &user,
                actor: None,
                resource: &resource
            },
            &interner
        ));

        // Present, no element matches ("alice" not in roles): != is true.
        assert!(eval_wildcard_comparison(
            &wildcard("roles", "owner", true),
            EntityBindings {
                user: &user,
                actor: None,
                resource: &resource
            },
            &interner
        ));
        // Present and matching (readers contains "admin"? use owner=="alice"
        // against a collection containing it): == false here, so != true was
        // checked above; verify the complement on a matching pair.
        let matching = CompiledWildcardComparison {
            collection_entity: EntityType::Resource,
            collection_attr: interner.intern("readers"), // ["admin"]
            scalar_entity: EntityType::User,
            scalar_attr: interner.intern("role"), // "admin"
            negated: true,
        };
        assert!(!eval_wildcard_comparison(
            &matching,
            EntityBindings {
                user: &user,
                actor: None,
                resource: &resource
            },
            &interner
        ));
    }

    #[test]
    fn test_eval_attr_equals_literal_string() {
        let interner = create_test_interner();
        let user = create_test_user(&interner);
        let resource = create_test_resource(&interner);

        let role_key = interner.intern("role");
        let admin_val = interner.intern("admin");
        let viewer_val = interner.intern("viewer");

        assert!(eval_attr_equals_literal(
            &EntityType::User,
            role_key,
            admin_val,
            EntityBindings {
                user: &user,
                actor: None,
                resource: &resource
            },
            &interner
        ));

        assert!(!eval_attr_equals_literal(
            &EntityType::User,
            role_key,
            viewer_val,
            EntityBindings {
                user: &user,
                actor: None,
                resource: &resource
            },
            &interner
        ));
    }

    #[test]
    fn test_eval_attr_gte_literal() {
        let interner = create_test_interner();
        let user = create_test_user(&interner);
        let resource = create_test_resource(&interner);

        let level_key = interner.intern("level");

        assert!(eval_attr_gte_literal(
            &EntityType::User,
            level_key,
            5.0,
            EntityBindings {
                user: &user,
                actor: None,
                resource: &resource
            }
        ));
        assert!(eval_attr_gte_literal(
            &EntityType::User,
            level_key,
            4.0,
            EntityBindings {
                user: &user,
                actor: None,
                resource: &resource
            }
        ));
        assert!(!eval_attr_gte_literal(
            &EntityType::User,
            level_key,
            6.0,
            EntityBindings {
                user: &user,
                actor: None,
                resource: &resource
            }
        ));
    }

    #[test]
    fn test_eval_attr_gt_literal() {
        let interner = create_test_interner();
        let user = create_test_user(&interner);
        let resource = create_test_resource(&interner);

        let level_key = interner.intern("level");

        assert!(eval_attr_gt_literal(
            &EntityType::User,
            level_key,
            4.0,
            EntityBindings {
                user: &user,
                actor: None,
                resource: &resource
            }
        ));
        assert!(!eval_attr_gt_literal(
            &EntityType::User,
            level_key,
            5.0,
            EntityBindings {
                user: &user,
                actor: None,
                resource: &resource
            }
        ));
    }

    #[test]
    fn test_eval_attr_lte_literal() {
        let interner = create_test_interner();
        let user = create_test_user(&interner);
        let resource = create_test_resource(&interner);

        let level_key = interner.intern("level");

        assert!(eval_attr_lte_literal(
            &EntityType::User,
            level_key,
            5.0,
            EntityBindings {
                user: &user,
                actor: None,
                resource: &resource
            }
        ));
        assert!(eval_attr_lte_literal(
            &EntityType::User,
            level_key,
            6.0,
            EntityBindings {
                user: &user,
                actor: None,
                resource: &resource
            }
        ));
        assert!(!eval_attr_lte_literal(
            &EntityType::User,
            level_key,
            4.0,
            EntityBindings {
                user: &user,
                actor: None,
                resource: &resource
            }
        ));
    }

    #[test]
    fn test_eval_user_equals_resource() {
        let interner = create_test_interner();
        let user = create_test_user(&interner);
        let resource = create_test_resource(&interner);

        let dept_key = interner.intern("department");

        // Both have department = engineering
        assert!(eval_user_equals_resource(
            dept_key,
            dept_key,
            EntityBindings {
                user: &user,
                actor: None,
                resource: &resource
            },
            &interner
        ));

        // role != owner
        let role_key = interner.intern("role");
        let owner_key = interner.intern("owner");
        assert!(!eval_user_equals_resource(
            role_key,
            owner_key,
            EntityBindings {
                user: &user,
                actor: None,
                resource: &resource
            },
            &interner
        ));
    }

    #[test]
    fn test_eval_user_int_greater() {
        let interner = create_test_interner();
        let user = create_test_user(&interner);
        let resource = create_test_resource(&interner);

        let level_key = interner.intern("level");
        let min_level_key = interner.intern("min_level");

        // user.level (5) > resource.min_level (3)
        assert!(eval_user_int_greater(
            level_key,
            min_level_key,
            EntityBindings {
                user: &user,
                actor: None,
                resource: &resource
            },
            &interner
        ));
    }

    #[test]
    fn test_eval_resource_int_greater() {
        let interner = create_test_interner();
        let user = create_test_user(&interner);
        let resource = create_test_resource(&interner);

        let level_key = interner.intern("level");
        let min_level_key = interner.intern("min_level");

        // resource.min_level (3) > user.level (5) = false
        assert!(!eval_resource_int_greater(
            min_level_key,
            level_key,
            EntityBindings {
                user: &user,
                actor: None,
                resource: &resource
            },
            &interner
        ));
    }

    #[test]
    fn test_eval_user_wildcard_equals_resource() {
        let interner = create_test_interner();
        let user = create_test_user(&interner);
        let resource = create_test_resource(&interner);

        let roles_key = interner.intern("roles");
        let owner_key = interner.intern("owner");

        // user.roles contains "admin", "viewer"
        // resource.owner is "alice"
        // So roles[_] == owner should be false
        assert!(!eval_user_wildcard_equals_resource(
            roles_key,
            owner_key,
            EntityBindings {
                user: &user,
                actor: None,
                resource: &resource
            },
            &interner
        ));
    }

    #[test]
    fn test_eval_resource_wildcard_equals_user() {
        let interner = create_test_interner();
        let user = create_test_user(&interner);
        let resource = create_test_resource(&interner);

        let readers_key = interner.intern("readers");
        let role_key = interner.intern("role");

        // resource.readers contains "admin"
        // user.role is "admin"
        // So readers[_] == role should be true
        assert!(eval_resource_wildcard_equals_user(
            readers_key,
            role_key,
            EntityBindings {
                user: &user,
                actor: None,
                resource: &resource
            },
            &interner
        ));
    }

    #[test]
    fn test_eval_same_entity_attr_compare() {
        let interner = create_test_interner();

        let user_id = interner.intern("test_user");
        let user_type = interner.intern("User");

        let a_key = interner.intern("a");
        let b_key = interner.intern("b");

        let mut attrs: HashMap<InternedString, AttributeValue> = HashMap::new();
        attrs.insert(a_key, AttributeValue::Int(10));
        attrs.insert(b_key, AttributeValue::Int(5));

        let user = Entity::new(user_id, user_type, attrs);

        let resource_id = interner.intern("test_resource");
        let resource_type = interner.intern("Resource");
        let resource = Entity::new(resource_id, resource_type, HashMap::new());

        // user.a (10) > user.b (5)
        assert!(eval_same_entity_attr_compare(
            &EntityType::User,
            a_key,
            b_key,
            &AttrCompareOp::Greater,
            EntityBindings {
                user: &user,
                actor: None,
                resource: &resource
            },
            &interner
        ));

        // user.a (10) == user.b (5) is false
        assert!(!eval_same_entity_attr_compare(
            &EntityType::User,
            a_key,
            b_key,
            &AttrCompareOp::Equal,
            EntityBindings {
                user: &user,
                actor: None,
                resource: &resource
            },
            &interner
        ));
    }

    #[test]
    fn test_compare_attr_values_int() {
        assert!(compare_attr_values(
            Some(&AttributeValue::Int(10)),
            Some(&AttributeValue::Int(5)),
            &AttrCompareOp::Greater
        ));

        assert!(compare_attr_values(
            Some(&AttributeValue::Int(5)),
            Some(&AttributeValue::Int(5)),
            &AttrCompareOp::Equal
        ));

        assert!(!compare_attr_values(
            Some(&AttributeValue::Int(5)),
            Some(&AttributeValue::Int(10)),
            &AttrCompareOp::Greater
        ));
    }

    #[test]
    fn test_compare_attr_values_float() {
        assert!(compare_attr_values(
            Some(&AttributeValue::Float(10.5)),
            Some(&AttributeValue::Float(5.5)),
            &AttrCompareOp::Greater
        ));

        assert!(compare_attr_values(
            Some(&AttributeValue::Float(5.0)),
            Some(&AttributeValue::Float(5.0)),
            &AttrCompareOp::Equal
        ));
    }

    #[test]
    fn test_compare_attr_values_cross_type() {
        // Int vs Float
        assert!(compare_attr_values(
            Some(&AttributeValue::Int(10)),
            Some(&AttributeValue::Float(5.5)),
            &AttrCompareOp::Greater
        ));

        assert!(compare_attr_values(
            Some(&AttributeValue::Float(10.5)),
            Some(&AttributeValue::Int(5)),
            &AttrCompareOp::Greater
        ));
    }

    #[test]
    fn test_compare_attr_values_bool() {
        assert!(compare_attr_values(
            Some(&AttributeValue::Bool(true)),
            Some(&AttributeValue::Bool(true)),
            &AttrCompareOp::Equal
        ));

        assert!(compare_attr_values(
            Some(&AttributeValue::Bool(true)),
            Some(&AttributeValue::Bool(false)),
            &AttrCompareOp::NotEqual
        ));
    }

    #[test]
    fn test_context_entity_type_returns_false() {
        let interner = create_test_interner();
        let user = create_test_user(&interner);
        let resource = create_test_resource(&interner);

        let role_key = interner.intern("role");
        let admin_val = interner.intern("admin");

        assert!(!eval_attr_equals_literal(
            &EntityType::Context,
            role_key,
            admin_val,
            EntityBindings {
                user: &user,
                actor: None,
                resource: &resource
            },
            &interner
        ));

        assert!(!eval_attr_gte_literal(
            &EntityType::Context,
            role_key,
            5.0,
            EntityBindings {
                user: &user,
                actor: None,
                resource: &resource
            }
        ));
    }
}
