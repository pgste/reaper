//! Time operation evaluation.
//!
//! This module handles time-based conditions:
//! - time comparisons (is_after, is_before)
//! - current time retrieval (now, now_ms, now_ns)
//! - time arithmetic (add/subtract durations)
//!
//! ## Performance Characteristics
//! - Time comparisons are O(1)
//! - System time calls have ~25ns overhead

// Allow unused functions - some are used in tests only or reserved for future use
#![allow(dead_code)]

use std::time::{SystemTime, UNIX_EPOCH};

use crate::data::{AttributeValue, Entity, InternedString};

use super::entity_helpers::get_entity_for_type;
use super::types::{CompiledTimeCondition, EntityType, NumericOp};

// ============================================================================
// V2 Dispatch Functions
// ============================================================================

/// Evaluate a V2 time operation
#[inline]
pub fn eval_time_operation(
    cond: &CompiledTimeCondition,
    user: &Entity,
    resource: &Entity,
) -> bool {
    let entity = match get_entity_for_type(&cond.entity_type, user, resource) {
        Some(e) => e,
        None => return false,
    };

    match entity.get_attribute(cond.attribute) {
        Some(AttributeValue::Int(ts)) => {
            match cond.op {
                NumericOp::Greater => *ts > cond.threshold,
                NumericOp::GreaterEqual => *ts >= cond.threshold,
                NumericOp::Less => *ts < cond.threshold,
                NumericOp::LessEqual => *ts <= cond.threshold,
                NumericOp::Equal => *ts == cond.threshold,
                NumericOp::NotEqual => *ts != cond.threshold,
            }
        }
        Some(AttributeValue::Float(ts)) => {
            let ts_int = *ts as i64;
            match cond.op {
                NumericOp::Greater => ts_int > cond.threshold,
                NumericOp::GreaterEqual => ts_int >= cond.threshold,
                NumericOp::Less => ts_int < cond.threshold,
                NumericOp::LessEqual => ts_int <= cond.threshold,
                NumericOp::Equal => ts_int == cond.threshold,
                NumericOp::NotEqual => ts_int != cond.threshold,
            }
        }
        _ => false,
    }
}

// ============================================================================
// Legacy Functions (still used by some code paths)
// ============================================================================

/// Evaluate time is after: entity.attr > threshold (as unix timestamp)
#[inline]
pub fn eval_time_is_after(
    entity_type: &EntityType,
    attribute: InternedString,
    threshold: i64,
    user: &Entity,
    resource: &Entity,
) -> bool {
    let entity = match get_entity_for_type(entity_type, user, resource) {
        Some(e) => e,
        None => return false,
    };

    match entity.get_attribute(attribute) {
        Some(AttributeValue::Int(ts)) => *ts > threshold,
        Some(AttributeValue::Float(ts)) => (*ts as i64) > threshold,
        _ => false,
    }
}

/// Evaluate time is before: entity.attr < threshold (as unix timestamp)
#[inline]
pub fn eval_time_is_before(
    entity_type: &EntityType,
    attribute: InternedString,
    threshold: i64,
    user: &Entity,
    resource: &Entity,
) -> bool {
    let entity = match get_entity_for_type(entity_type, user, resource) {
        Some(e) => e,
        None => return false,
    };

    match entity.get_attribute(attribute) {
        Some(AttributeValue::Int(ts)) => *ts < threshold,
        Some(AttributeValue::Float(ts)) => (*ts as i64) < threshold,
        _ => false,
    }
}

/// Evaluate time is between: start <= entity.attr <= end
#[inline]
pub fn eval_time_is_between(
    entity_type: &EntityType,
    attribute: InternedString,
    start: i64,
    end: i64,
    user: &Entity,
    resource: &Entity,
) -> bool {
    let entity = match get_entity_for_type(entity_type, user, resource) {
        Some(e) => e,
        None => return false,
    };

    match entity.get_attribute(attribute) {
        Some(AttributeValue::Int(ts)) => *ts >= start && *ts <= end,
        Some(AttributeValue::Float(ts)) => {
            let ts_int = *ts as i64;
            ts_int >= start && ts_int <= end
        }
        _ => false,
    }
}

/// Get current time as unix timestamp (seconds since epoch)
#[inline]
pub fn get_time_now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Get current time as milliseconds since epoch
#[inline]
pub fn get_time_now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Get current time as nanoseconds since epoch
#[inline]
pub fn get_time_now_nanos() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

/// Evaluate time attribute plus duration: entity.attr + duration_secs
#[inline]
pub fn eval_time_add_duration(
    entity_type: &EntityType,
    attribute: InternedString,
    duration_secs: i64,
    user: &Entity,
    resource: &Entity,
) -> Option<AttributeValue> {
    let entity = get_entity_for_type(entity_type, user, resource)?;

    match entity.get_attribute(attribute) {
        Some(AttributeValue::Int(ts)) => Some(AttributeValue::Int(*ts + duration_secs)),
        Some(AttributeValue::Float(ts)) => Some(AttributeValue::Int((*ts as i64) + duration_secs)),
        _ => None,
    }
}

/// Evaluate time attribute minus duration: entity.attr - duration_secs
#[inline]
pub fn eval_time_sub_duration(
    entity_type: &EntityType,
    attribute: InternedString,
    duration_secs: i64,
    user: &Entity,
    resource: &Entity,
) -> Option<AttributeValue> {
    let entity = get_entity_for_type(entity_type, user, resource)?;

    match entity.get_attribute(attribute) {
        Some(AttributeValue::Int(ts)) => Some(AttributeValue::Int(*ts - duration_secs)),
        Some(AttributeValue::Float(ts)) => Some(AttributeValue::Int((*ts as i64) - duration_secs)),
        _ => None,
    }
}

/// Check if time attribute is in the future (greater than now)
#[inline]
pub fn eval_time_is_future(
    entity_type: &EntityType,
    attribute: InternedString,
    user: &Entity,
    resource: &Entity,
) -> bool {
    let now = get_time_now_secs();
    eval_time_is_after(entity_type, attribute, now, user, resource)
}

/// Check if time attribute is in the past (less than now)
#[inline]
pub fn eval_time_is_past(
    entity_type: &EntityType,
    attribute: InternedString,
    user: &Entity,
    resource: &Entity,
) -> bool {
    let now = get_time_now_secs();
    eval_time_is_before(entity_type, attribute, now, user, resource)
}

/// Get time difference between two attributes in seconds
#[inline]
pub fn eval_time_diff_secs(
    entity1_type: &EntityType,
    attribute1: InternedString,
    entity2_type: &EntityType,
    attribute2: InternedString,
    user: &Entity,
    resource: &Entity,
) -> Option<i64> {
    let entity1 = get_entity_for_type(entity1_type, user, resource)?;
    let entity2 = get_entity_for_type(entity2_type, user, resource)?;

    let ts1 = match entity1.get_attribute(attribute1) {
        Some(AttributeValue::Int(ts)) => *ts,
        Some(AttributeValue::Float(ts)) => *ts as i64,
        _ => return None,
    };

    let ts2 = match entity2.get_attribute(attribute2) {
        Some(AttributeValue::Int(ts)) => *ts,
        Some(AttributeValue::Float(ts)) => *ts as i64,
        _ => return None,
    };

    Some(ts1 - ts2)
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

    fn create_test_user_with_timestamp(interner: &StringInterner, timestamp: i64) -> Entity {
        let user_id = interner.intern("user_alice");
        let user_type = interner.intern("User");

        let created_key = interner.intern("created_at");
        let expires_key = interner.intern("expires_at");

        let mut attrs: HashMap<InternedString, AttributeValue> = HashMap::new();
        attrs.insert(created_key, AttributeValue::Int(timestamp));
        attrs.insert(expires_key, AttributeValue::Int(timestamp + 3600)); // +1 hour

        Entity::new(user_id, user_type, attrs)
    }

    fn create_test_resource(interner: &StringInterner) -> Entity {
        let resource_id = interner.intern("resource_doc1");
        let resource_type = interner.intern("Resource");

        let modified_key = interner.intern("modified_at");

        let mut attrs: HashMap<InternedString, AttributeValue> = HashMap::new();
        attrs.insert(modified_key, AttributeValue::Int(1700000000));

        Entity::new(resource_id, resource_type, attrs)
    }

    #[test]
    fn test_eval_time_is_after() {
        let interner = create_test_interner();
        let user = create_test_user_with_timestamp(&interner, 1700000000);
        let resource = create_test_resource(&interner);

        let created_key = interner.intern("created_at");

        // 1700000000 > 1699999999
        assert!(eval_time_is_after(
            &EntityType::User,
            created_key,
            1699999999,
            &user,
            &resource
        ));

        // 1700000000 > 1700000000 is false
        assert!(!eval_time_is_after(
            &EntityType::User,
            created_key,
            1700000000,
            &user,
            &resource
        ));

        // 1700000000 > 1700000001 is false
        assert!(!eval_time_is_after(
            &EntityType::User,
            created_key,
            1700000001,
            &user,
            &resource
        ));
    }

    #[test]
    fn test_eval_time_is_before() {
        let interner = create_test_interner();
        let user = create_test_user_with_timestamp(&interner, 1700000000);
        let resource = create_test_resource(&interner);

        let created_key = interner.intern("created_at");

        // 1700000000 < 1700000001
        assert!(eval_time_is_before(
            &EntityType::User,
            created_key,
            1700000001,
            &user,
            &resource
        ));

        // 1700000000 < 1700000000 is false
        assert!(!eval_time_is_before(
            &EntityType::User,
            created_key,
            1700000000,
            &user,
            &resource
        ));
    }

    #[test]
    fn test_eval_time_is_between() {
        let interner = create_test_interner();
        let user = create_test_user_with_timestamp(&interner, 1700000000);
        let resource = create_test_resource(&interner);

        let created_key = interner.intern("created_at");

        // 1700000000 is between 1699999999 and 1700000001
        assert!(eval_time_is_between(
            &EntityType::User,
            created_key,
            1699999999,
            1700000001,
            &user,
            &resource
        ));

        // Edge case: equal to boundaries
        assert!(eval_time_is_between(
            &EntityType::User,
            created_key,
            1700000000,
            1700000000,
            &user,
            &resource
        ));

        // Outside range
        assert!(!eval_time_is_between(
            &EntityType::User,
            created_key,
            1700000001,
            1700000002,
            &user,
            &resource
        ));
    }

    #[test]
    fn test_get_time_now() {
        let now = get_time_now_secs();
        // Should be a reasonable unix timestamp (after year 2020)
        assert!(now > 1577836800); // Jan 1, 2020

        let now_ms = get_time_now_millis();
        assert!(now_ms > 1577836800000); // Jan 1, 2020 in ms

        let now_ns = get_time_now_nanos();
        assert!(now_ns > 1577836800000000000); // Jan 1, 2020 in ns
    }

    #[test]
    fn test_eval_time_add_duration() {
        let interner = create_test_interner();
        let user = create_test_user_with_timestamp(&interner, 1700000000);
        let resource = create_test_resource(&interner);

        let created_key = interner.intern("created_at");

        let result = eval_time_add_duration(
            &EntityType::User,
            created_key,
            3600, // +1 hour
            &user,
            &resource,
        );
        assert_eq!(result, Some(AttributeValue::Int(1700003600)));
    }

    #[test]
    fn test_eval_time_sub_duration() {
        let interner = create_test_interner();
        let user = create_test_user_with_timestamp(&interner, 1700000000);
        let resource = create_test_resource(&interner);

        let created_key = interner.intern("created_at");

        let result = eval_time_sub_duration(
            &EntityType::User,
            created_key,
            3600, // -1 hour
            &user,
            &resource,
        );
        assert_eq!(result, Some(AttributeValue::Int(1699996400)));
    }

    #[test]
    fn test_eval_time_is_future() {
        let interner = create_test_interner();
        let now = get_time_now_secs();
        let user = create_test_user_with_timestamp(&interner, now + 3600); // 1 hour in future
        let resource = create_test_resource(&interner);

        let created_key = interner.intern("created_at");

        assert!(eval_time_is_future(
            &EntityType::User,
            created_key,
            &user,
            &resource
        ));
    }

    #[test]
    fn test_eval_time_is_past() {
        let interner = create_test_interner();
        let now = get_time_now_secs();
        let user = create_test_user_with_timestamp(&interner, now - 3600); // 1 hour in past
        let resource = create_test_resource(&interner);

        let created_key = interner.intern("created_at");

        assert!(eval_time_is_past(
            &EntityType::User,
            created_key,
            &user,
            &resource
        ));
    }

    #[test]
    fn test_eval_time_diff_secs() {
        let interner = create_test_interner();
        let user = create_test_user_with_timestamp(&interner, 1700000000);
        let resource = create_test_resource(&interner);

        let expires_key = interner.intern("expires_at");
        let created_key = interner.intern("created_at");

        // expires_at (1700003600) - created_at (1700000000) = 3600
        let diff = eval_time_diff_secs(
            &EntityType::User,
            expires_key,
            &EntityType::User,
            created_key,
            &user,
            &resource,
        );
        assert_eq!(diff, Some(3600));
    }

    #[test]
    fn test_context_entity_type_returns_false() {
        let interner = create_test_interner();
        let user = create_test_user_with_timestamp(&interner, 1700000000);
        let resource = create_test_resource(&interner);

        let created_key = interner.intern("created_at");

        assert!(!eval_time_is_after(
            &EntityType::Context,
            created_key,
            1699999999,
            &user,
            &resource
        ));
    }

    #[test]
    fn test_missing_attribute_returns_false() {
        let interner = create_test_interner();
        let user = create_test_user_with_timestamp(&interner, 1700000000);
        let resource = create_test_resource(&interner);

        let unknown_key = interner.intern("unknown");

        assert!(!eval_time_is_after(
            &EntityType::User,
            unknown_key,
            1699999999,
            &user,
            &resource
        ));
    }

    #[test]
    fn test_non_time_attribute_returns_false() {
        let interner = create_test_interner();

        let user_id = interner.intern("user_test");
        let user_type = interner.intern("User");
        let name_key = interner.intern("name");
        let name_val = interner.intern("Alice");

        let mut attrs: HashMap<InternedString, AttributeValue> = HashMap::new();
        attrs.insert(name_key, AttributeValue::String(name_val));

        let user = Entity::new(user_id, user_type, attrs);

        let resource_id = interner.intern("resource_test");
        let resource_type = interner.intern("Resource");
        let resource = Entity::new(resource_id, resource_type, HashMap::new());

        // String attribute used for time comparison
        assert!(!eval_time_is_after(
            &EntityType::User,
            name_key,
            1699999999,
            &user,
            &resource
        ));
    }

    #[test]
    fn test_float_timestamp_support() {
        let interner = create_test_interner();

        let user_id = interner.intern("user_test");
        let user_type = interner.intern("User");
        let ts_key = interner.intern("timestamp");

        let mut attrs: HashMap<InternedString, AttributeValue> = HashMap::new();
        attrs.insert(ts_key, AttributeValue::Float(1700000000.5));

        let user = Entity::new(user_id, user_type, attrs);

        let resource_id = interner.intern("resource_test");
        let resource_type = interner.intern("Resource");
        let resource = Entity::new(resource_id, resource_type, HashMap::new());

        // Float timestamp converted to i64 for comparison
        assert!(eval_time_is_after(
            &EntityType::User,
            ts_key,
            1699999999,
            &user,
            &resource
        ));
    }
}
