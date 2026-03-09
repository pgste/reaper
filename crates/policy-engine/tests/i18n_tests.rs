//! Internationalization (i18n) and UTF-8 Tests
//!
//! Tests for proper handling of:
//! - Unicode strings in policies and data
//! - Multi-byte characters
//! - Special Unicode categories (RTL, zero-width, combining chars)
//! - Various scripts and languages

use policy_engine::data::DataLoader;
use policy_engine::reap::ReaperPolicy;
use policy_engine::{DataStore, PolicyAction, PolicyEvaluator, PolicyRequest};
use std::collections::HashMap;
use std::sync::Arc;

// ============================================================================
// SECTION 1: Basic Unicode Support
// ============================================================================

/// Test policy with Unicode identifiers in strings
#[test]
fn test_unicode_string_values() {
    let policy_text = r#"
policy unicode_test {
    default: deny,

    rule japanese_department {
        allow if {
            user.department == "技術部"
        }
    }
}
"#;

    let policy = policy_text.parse::<ReaperPolicy>().unwrap();
    let store = Arc::new(DataStore::new());

    let json = r#"
{
    "entities": [
        {"id": "tanaka", "type": "User", "attributes": {"id": "tanaka", "department": "技術部"}},
        {"id": "sato", "type": "User", "attributes": {"id": "sato", "department": "営業部"}}
    ]
}
"#;

    let loader = DataLoader::new((*store).clone());
    loader.load_json(json).unwrap();

    let evaluator = policy.build(Arc::clone(&store)).unwrap();

    // Should allow - department matches Japanese text
    let mut context = HashMap::new();
    context.insert("principal".to_string(), "tanaka".to_string());

    let request = PolicyRequest {
        resource: "test".to_string(),
        action: "read".to_string(),
        context,
    };

    let result = evaluator.evaluate(&request).unwrap();
    assert_eq!(result, PolicyAction::Allow);

    // Should deny - different department
    let mut context2 = HashMap::new();
    context2.insert("principal".to_string(), "sato".to_string());

    let request2 = PolicyRequest {
        resource: "test".to_string(),
        action: "read".to_string(),
        context: context2,
    };

    let result2 = evaluator.evaluate(&request2).unwrap();
    assert_eq!(result2, PolicyAction::Deny);
}

/// Test Chinese characters in policy
#[test]
fn test_chinese_characters() {
    let policy_text = r#"
policy chinese_test {
    default: deny,

    rule chinese_role {
        allow if {
            user.role == "管理员"
        }
    }
}
"#;

    let policy = policy_text.parse::<ReaperPolicy>().unwrap();
    let store = Arc::new(DataStore::new());

    let json = r#"
{
    "entities": [
        {"id": "wang", "type": "User", "attributes": {"id": "wang", "role": "管理员"}},
        {"id": "li", "type": "User", "attributes": {"id": "li", "role": "用户"}}
    ]
}
"#;

    let loader = DataLoader::new((*store).clone());
    loader.load_json(json).unwrap();

    let evaluator = policy.build(Arc::clone(&store)).unwrap();

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "wang".to_string());

    let request = PolicyRequest {
        resource: "test".to_string(),
        action: "read".to_string(),
        context,
    };

    let result = evaluator.evaluate(&request).unwrap();
    assert_eq!(result, PolicyAction::Allow);
}

/// Test Korean characters
#[test]
fn test_korean_characters() {
    let policy_text = r#"
policy korean_test {
    default: deny,

    rule korean_check {
        allow if {
            user.name != null && user.department == "개발팀"
        }
    }
}
"#;

    let policy = policy_text.parse::<ReaperPolicy>().unwrap();
    let store = Arc::new(DataStore::new());

    let json = r#"
{
    "entities": [
        {"id": "kim", "type": "User", "attributes": {"id": "kim", "name": "김철수", "department": "개발팀"}}
    ]
}
"#;

    let loader = DataLoader::new((*store).clone());
    loader.load_json(json).unwrap();

    let evaluator = policy.build(Arc::clone(&store)).unwrap();

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "kim".to_string());

    let request = PolicyRequest {
        resource: "test".to_string(),
        action: "read".to_string(),
        context,
    };

    let result = evaluator.evaluate(&request).unwrap();
    assert_eq!(result, PolicyAction::Allow);
}

// ============================================================================
// SECTION 2: Special Unicode Characters
// ============================================================================

/// Test emoji handling
#[test]
fn test_emoji_handling() {
    let policy_text = r#"
policy emoji_test {
    default: deny,

    rule emoji_status {
        allow if {
            user.status == "✅"
        }
    }
}
"#;

    let policy = policy_text.parse::<ReaperPolicy>().unwrap();
    let store = Arc::new(DataStore::new());

    let json = r#"
{
    "entities": [
        {"id": "user1", "type": "User", "attributes": {"id": "user1", "status": "✅"}},
        {"id": "user2", "type": "User", "attributes": {"id": "user2", "status": "❌"}}
    ]
}
"#;

    let loader = DataLoader::new((*store).clone());
    loader.load_json(json).unwrap();

    let evaluator = policy.build(Arc::clone(&store)).unwrap();

    // Should allow - status matches
    let mut context = HashMap::new();
    context.insert("principal".to_string(), "user1".to_string());

    let request = PolicyRequest {
        resource: "test".to_string(),
        action: "read".to_string(),
        context,
    };

    let result = evaluator.evaluate(&request).unwrap();
    assert_eq!(result, PolicyAction::Allow);

    // Should deny - status doesn't match
    let mut context2 = HashMap::new();
    context2.insert("principal".to_string(), "user2".to_string());

    let request2 = PolicyRequest {
        resource: "test".to_string(),
        action: "read".to_string(),
        context: context2,
    };

    let result2 = evaluator.evaluate(&request2).unwrap();
    assert_eq!(result2, PolicyAction::Deny);
}

/// Test multi-codepoint emoji (family emoji, flag emoji)
#[test]
fn test_complex_emoji() {
    let policy_text = r#"
policy complex_emoji_test {
    default: deny,

    rule flag_match {
        allow if {
            user.country == "🇯🇵"
        }
    }
}
"#;

    let policy = policy_text.parse::<ReaperPolicy>().unwrap();
    let store = Arc::new(DataStore::new());

    // Flag emoji are actually two regional indicator symbols combined
    let json = r#"
{
    "entities": [
        {"id": "jp_user", "type": "User", "attributes": {"id": "jp_user", "country": "🇯🇵"}},
        {"id": "us_user", "type": "User", "attributes": {"id": "us_user", "country": "🇺🇸"}}
    ]
}
"#;

    let loader = DataLoader::new((*store).clone());
    loader.load_json(json).unwrap();

    let evaluator = policy.build(Arc::clone(&store)).unwrap();

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "jp_user".to_string());

    let request = PolicyRequest {
        resource: "test".to_string(),
        action: "read".to_string(),
        context,
    };

    let result = evaluator.evaluate(&request).unwrap();
    assert_eq!(result, PolicyAction::Allow);
}

/// Test zero-width characters (should be handled without issues)
#[test]
fn test_zero_width_characters() {
    let policy_text = r#"
policy zero_width_test {
    default: deny,

    rule check_name {
        allow if {
            user.name != null
        }
    }
}
"#;

    let policy = policy_text.parse::<ReaperPolicy>().unwrap();
    let store = Arc::new(DataStore::new());

    // Name contains zero-width joiner and zero-width non-joiner
    let json = r#"
{
    "entities": [
        {"id": "zw_user", "type": "User", "attributes": {"id": "zw_user", "name": "test\u200Bname\u200Cvalue"}}
    ]
}
"#;

    let loader = DataLoader::new((*store).clone());
    loader.load_json(json).unwrap();

    let evaluator = policy.build(Arc::clone(&store)).unwrap();

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "zw_user".to_string());

    let request = PolicyRequest {
        resource: "test".to_string(),
        action: "read".to_string(),
        context,
    };

    let result = evaluator.evaluate(&request).unwrap();
    assert_eq!(result, PolicyAction::Allow);
}

/// Test combining characters (diacritics)
#[test]
fn test_combining_characters() {
    let policy_text = r#"
policy combining_test {
    default: deny,

    rule accented_name {
        allow if {
            user.name == "José"
        }
    }
}
"#;

    let policy = policy_text.parse::<ReaperPolicy>().unwrap();
    let store = Arc::new(DataStore::new());

    // Using precomposed form (single codepoint for é)
    let json = r#"
{
    "entities": [
        {"id": "jose", "type": "User", "attributes": {"id": "jose", "name": "José"}}
    ]
}
"#;

    let loader = DataLoader::new((*store).clone());
    loader.load_json(json).unwrap();

    let evaluator = policy.build(Arc::clone(&store)).unwrap();

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "jose".to_string());

    let request = PolicyRequest {
        resource: "test".to_string(),
        action: "read".to_string(),
        context,
    };

    let result = evaluator.evaluate(&request).unwrap();
    assert_eq!(result, PolicyAction::Allow);
}

// ============================================================================
// SECTION 3: RTL (Right-to-Left) Text
// ============================================================================

/// Test Arabic text
#[test]
fn test_arabic_text() {
    let policy_text = r#"
policy arabic_test {
    default: deny,

    rule arabic_dept {
        allow if {
            user.department == "الإدارة"
        }
    }
}
"#;

    let policy = policy_text.parse::<ReaperPolicy>().unwrap();
    let store = Arc::new(DataStore::new());

    let json = r#"
{
    "entities": [
        {"id": "ahmed", "type": "User", "attributes": {"id": "ahmed", "department": "الإدارة"}}
    ]
}
"#;

    let loader = DataLoader::new((*store).clone());
    loader.load_json(json).unwrap();

    let evaluator = policy.build(Arc::clone(&store)).unwrap();

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "ahmed".to_string());

    let request = PolicyRequest {
        resource: "test".to_string(),
        action: "read".to_string(),
        context,
    };

    let result = evaluator.evaluate(&request).unwrap();
    assert_eq!(result, PolicyAction::Allow);
}

/// Test Hebrew text
#[test]
fn test_hebrew_text() {
    let policy_text = r#"
policy hebrew_test {
    default: deny,

    rule hebrew_role {
        allow if {
            user.role == "מנהל"
        }
    }
}
"#;

    let policy = policy_text.parse::<ReaperPolicy>().unwrap();
    let store = Arc::new(DataStore::new());

    let json = r#"
{
    "entities": [
        {"id": "david", "type": "User", "attributes": {"id": "david", "role": "מנהל"}}
    ]
}
"#;

    let loader = DataLoader::new((*store).clone());
    loader.load_json(json).unwrap();

    let evaluator = policy.build(Arc::clone(&store)).unwrap();

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "david".to_string());

    let request = PolicyRequest {
        resource: "test".to_string(),
        action: "read".to_string(),
        context,
    };

    let result = evaluator.evaluate(&request).unwrap();
    assert_eq!(result, PolicyAction::Allow);
}

// ============================================================================
// SECTION 4: String Operations with Unicode
// ============================================================================

/// Test contains() with Unicode
#[test]
fn test_contains_unicode() {
    let policy_text = r#"
policy contains_unicode_test {
    default: deny,

    rule email_domain {
        allow if {
            user.email.contains("@会社.jp")
        }
    }
}
"#;

    let policy = policy_text.parse::<ReaperPolicy>().unwrap();
    let store = Arc::new(DataStore::new());

    let json = r#"
{
    "entities": [
        {"id": "user1", "type": "User", "attributes": {"id": "user1", "email": "tanaka@会社.jp"}},
        {"id": "user2", "type": "User", "attributes": {"id": "user2", "email": "sato@other.com"}}
    ]
}
"#;

    let loader = DataLoader::new((*store).clone());
    loader.load_json(json).unwrap();

    let evaluator = policy.build(Arc::clone(&store)).unwrap();

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "user1".to_string());

    let request = PolicyRequest {
        resource: "test".to_string(),
        action: "read".to_string(),
        context,
    };

    let result = evaluator.evaluate(&request).unwrap();
    assert_eq!(result, PolicyAction::Allow);
}

/// Test startsWith() with Unicode
#[test]
fn test_startswith_unicode() {
    let policy_text = r#"
policy startswith_unicode_test {
    default: deny,

    rule admin_prefix {
        allow if {
            user.username.startswith("管理員_")
        }
    }
}
"#;

    let policy = policy_text.parse::<ReaperPolicy>().unwrap();
    let store = Arc::new(DataStore::new());

    let json = r#"
{
    "entities": [
        {"id": "admin1", "type": "User", "attributes": {"id": "admin1", "username": "管理員_王"}},
        {"id": "user1", "type": "User", "attributes": {"id": "user1", "username": "用戶_李"}}
    ]
}
"#;

    let loader = DataLoader::new((*store).clone());
    loader.load_json(json).unwrap();

    let evaluator = policy.build(Arc::clone(&store)).unwrap();

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "admin1".to_string());

    let request = PolicyRequest {
        resource: "test".to_string(),
        action: "read".to_string(),
        context,
    };

    let result = evaluator.evaluate(&request).unwrap();
    assert_eq!(result, PolicyAction::Allow);
}

// ============================================================================
// SECTION 5: Mixed Scripts
// ============================================================================

/// Test mixed Latin and CJK text
#[test]
fn test_mixed_scripts() {
    let policy_text = r#"
policy mixed_script_test {
    default: deny,

    rule mixed_id {
        allow if {
            user.employee_id != null
        }
    }
}
"#;

    let policy = policy_text.parse::<ReaperPolicy>().unwrap();
    let store = Arc::new(DataStore::new());

    // Mix of Latin, Japanese, and numbers
    let json = r#"
{
    "entities": [
        {"id": "emp1", "type": "User", "attributes": {"id": "emp1", "employee_id": "EMP-技術-001"}}
    ]
}
"#;

    let loader = DataLoader::new((*store).clone());
    loader.load_json(json).unwrap();

    let evaluator = policy.build(Arc::clone(&store)).unwrap();

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "emp1".to_string());

    let request = PolicyRequest {
        resource: "test".to_string(),
        action: "read".to_string(),
        context,
    };

    let result = evaluator.evaluate(&request).unwrap();
    assert_eq!(result, PolicyAction::Allow);
}

/// Test Cyrillic text
#[test]
fn test_cyrillic_text() {
    let policy_text = r#"
policy cyrillic_test {
    default: deny,

    rule russian_role {
        allow if {
            user.role == "администратор"
        }
    }
}
"#;

    let policy = policy_text.parse::<ReaperPolicy>().unwrap();
    let store = Arc::new(DataStore::new());

    let json = r#"
{
    "entities": [
        {"id": "ivan", "type": "User", "attributes": {"id": "ivan", "role": "администратор"}}
    ]
}
"#;

    let loader = DataLoader::new((*store).clone());
    loader.load_json(json).unwrap();

    let evaluator = policy.build(Arc::clone(&store)).unwrap();

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "ivan".to_string());

    let request = PolicyRequest {
        resource: "test".to_string(),
        action: "read".to_string(),
        context,
    };

    let result = evaluator.evaluate(&request).unwrap();
    assert_eq!(result, PolicyAction::Allow);
}

/// Test Greek text
#[test]
fn test_greek_text() {
    let policy_text = r#"
policy greek_test {
    default: deny,

    rule greek_dept {
        allow if {
            user.department == "Τμήμα Μηχανικών"
        }
    }
}
"#;

    let policy = policy_text.parse::<ReaperPolicy>().unwrap();
    let store = Arc::new(DataStore::new());

    let json = r#"
{
    "entities": [
        {"id": "nikos", "type": "User", "attributes": {"id": "nikos", "department": "Τμήμα Μηχανικών"}}
    ]
}
"#;

    let loader = DataLoader::new((*store).clone());
    loader.load_json(json).unwrap();

    let evaluator = policy.build(Arc::clone(&store)).unwrap();

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "nikos".to_string());

    let request = PolicyRequest {
        resource: "test".to_string(),
        action: "read".to_string(),
        context,
    };

    let result = evaluator.evaluate(&request).unwrap();
    assert_eq!(result, PolicyAction::Allow);
}

// ============================================================================
// SECTION 6: Edge Cases
// ============================================================================

/// Test very long Unicode strings
#[test]
fn test_long_unicode_string() {
    let policy_text = r#"
policy long_unicode_test {
    default: deny,

    rule check_bio {
        allow if {
            user.bio != null
        }
    }
}
"#;

    let policy = policy_text.parse::<ReaperPolicy>().unwrap();
    let store = Arc::new(DataStore::new());

    // Generate a long Unicode string with various characters
    let long_bio = "日本語テスト".repeat(1000);

    let json = format!(
        r#"
{{
    "entities": [
        {{"id": "user1", "type": "User", "attributes": {{"id": "user1", "bio": "{}"}}}}
    ]
}}
"#,
        long_bio
    );

    let loader = DataLoader::new((*store).clone());
    loader.load_json(&json).unwrap();

    let evaluator = policy.build(Arc::clone(&store)).unwrap();

    let mut context = HashMap::new();
    context.insert("principal".to_string(), "user1".to_string());

    let request = PolicyRequest {
        resource: "test".to_string(),
        action: "read".to_string(),
        context,
    };

    let result = evaluator.evaluate(&request).unwrap();
    assert_eq!(result, PolicyAction::Allow);
}

/// Test normalization forms (NFC vs NFD)
#[test]
fn test_unicode_normalization() {
    // This test checks if the system handles different normalization forms
    // NFC: é as single codepoint (U+00E9)
    // NFD: é as e + combining acute accent (U+0065 U+0301)

    let policy_text = r#"
policy normalization_test {
    default: deny,

    rule name_check {
        allow if {
            user.name != null
        }
    }
}
"#;

    let policy = policy_text.parse::<ReaperPolicy>().unwrap();
    let store = Arc::new(DataStore::new());

    // NFC form
    let json = r#"
{
    "entities": [
        {"id": "user_nfc", "type": "User", "attributes": {"id": "user_nfc", "name": "café"}},
        {"id": "user_nfd", "type": "User", "attributes": {"id": "user_nfd", "name": "café"}}
    ]
}
"#;

    let loader = DataLoader::new((*store).clone());
    loader.load_json(json).unwrap();

    let evaluator = policy.build(Arc::clone(&store)).unwrap();

    // Both should work
    for user in &["user_nfc", "user_nfd"] {
        let mut context = HashMap::new();
        context.insert("principal".to_string(), user.to_string());

        let request = PolicyRequest {
            resource: "test".to_string(),
            action: "read".to_string(),
            context,
        };

        let result = evaluator.evaluate(&request).unwrap();
        assert_eq!(result, PolicyAction::Allow, "Failed for {}", user);
    }
}
