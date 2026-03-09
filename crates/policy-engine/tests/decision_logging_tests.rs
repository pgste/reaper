//! Decision Logging Tests
//!
//! Tests for decision logging functionality including:
//! - SIEM format compliance (NDJSON)
//! - Audit trail completeness
//! - Buffer operations and filtering
//! - Log configuration

use policy_engine::decision_buffer::{DecisionBuffer, DecisionFilter};
use policy_engine::decision_log::{DecisionLogConfig, DecisionLogEntry};
use std::collections::HashMap;
use std::time::Duration;

// ============================================================================
// SECTION 1: Decision Log Entry Tests
// ============================================================================

/// Test creating a basic decision log entry
#[test]
fn test_decision_log_entry_creation() {
    let entry = DecisionLogEntry::new(
        "alice".to_string(),
        "read".to_string(),
        "document-123".to_string(),
        "allow".to_string(),
        "policy-001".to_string(),
        "data-access-policy".to_string(),
    );

    assert_eq!(entry.principal, "alice");
    assert_eq!(entry.action, "read");
    assert_eq!(entry.resource, "document-123");
    assert_eq!(entry.decision, "allow");
    assert_eq!(entry.policy_id, "policy-001");
    assert_eq!(entry.policy_name, "data-access-policy");
    assert!(!entry.decision_id.is_empty());
    assert!(!entry.timestamp.is_empty());
    assert!(!entry.cache_hit);
    assert_eq!(entry.evaluation_time_ns, 0);
}

/// Test decision log entry builder methods
#[test]
fn test_decision_log_entry_builder() {
    let mut context = HashMap::new();
    context.insert("ip".to_string(), serde_json::json!("192.168.1.1"));
    context.insert("user_agent".to_string(), serde_json::json!("Mozilla/5.0"));

    let entry = DecisionLogEntry::new(
        "bob".to_string(),
        "write".to_string(),
        "file-456".to_string(),
        "deny".to_string(),
        "policy-002".to_string(),
        "write-policy".to_string(),
    )
    .with_trace_id("trace-abc-123".to_string())
    .with_context(context)
    .with_evaluation_time_ns(1500)
    .with_agent_id("agent-1".to_string())
    .with_matched_rule("rule-deny-write".to_string())
    .with_cache_hit(true);

    assert_eq!(entry.trace_id, Some("trace-abc-123".to_string()));
    assert_eq!(entry.context.len(), 2);
    assert_eq!(entry.evaluation_time_ns, 1500);
    assert_eq!(entry.agent_id, Some("agent-1".to_string()));
    assert_eq!(entry.matched_rule, Some("rule-deny-write".to_string()));
    assert!(entry.cache_hit);
}

// ============================================================================
// SECTION 2: SIEM Format Compliance (NDJSON)
// ============================================================================

/// Test that decision log entries serialize to valid NDJSON
#[test]
fn test_ndjson_format_compliance() {
    let entry = DecisionLogEntry::new(
        "alice".to_string(),
        "read".to_string(),
        "document".to_string(),
        "allow".to_string(),
        "policy-001".to_string(),
        "access-policy".to_string(),
    );

    // Should serialize to JSON without errors
    let json = serde_json::to_string(&entry);
    assert!(json.is_ok(), "Should serialize to JSON");

    let json_str = json.unwrap();

    // NDJSON is single-line JSON (no newlines within the record)
    assert!(
        !json_str.contains('\n'),
        "NDJSON should not contain newlines within record"
    );

    // Should be valid JSON that can be parsed back
    let parsed: Result<DecisionLogEntry, _> = serde_json::from_str(&json_str);
    assert!(parsed.is_ok(), "Should parse back from JSON");

    let parsed_entry = parsed.unwrap();
    assert_eq!(parsed_entry.principal, "alice");
    assert_eq!(parsed_entry.decision, "allow");
}

/// Test SIEM-required fields are present
#[test]
fn test_siem_required_fields() {
    let entry = DecisionLogEntry::new(
        "user-123".to_string(),
        "delete".to_string(),
        "resource-456".to_string(),
        "deny".to_string(),
        "pol-789".to_string(),
        "deletion-policy".to_string(),
    );

    let json = serde_json::to_value(&entry).unwrap();

    // Required SIEM fields
    assert!(json.get("timestamp").is_some(), "Must have timestamp");
    assert!(json.get("decision_id").is_some(), "Must have decision_id");
    assert!(json.get("principal").is_some(), "Must have principal");
    assert!(json.get("action").is_some(), "Must have action");
    assert!(json.get("resource").is_some(), "Must have resource");
    assert!(json.get("decision").is_some(), "Must have decision");
    assert!(json.get("policy_id").is_some(), "Must have policy_id");
    assert!(json.get("policy_name").is_some(), "Must have policy_name");

    // Timestamp should be RFC3339 format
    let timestamp = json.get("timestamp").unwrap().as_str().unwrap();
    assert!(
        chrono::DateTime::parse_from_rfc3339(timestamp).is_ok(),
        "Timestamp should be RFC3339 format"
    );

    // Decision ID should be UUID format
    let decision_id = json.get("decision_id").unwrap().as_str().unwrap();
    assert!(
        uuid::Uuid::parse_str(decision_id).is_ok(),
        "Decision ID should be UUID format"
    );
}

/// Test optional fields are excluded when not set (clean NDJSON)
#[test]
fn test_optional_fields_excluded() {
    let entry = DecisionLogEntry::new(
        "alice".to_string(),
        "read".to_string(),
        "doc".to_string(),
        "allow".to_string(),
        "pol-1".to_string(),
        "policy".to_string(),
    );

    let json = serde_json::to_value(&entry).unwrap();

    // Optional fields should not be present when None
    assert!(
        json.get("trace_id").is_none(),
        "trace_id should be excluded when None"
    );
    assert!(
        json.get("policy_version").is_none(),
        "policy_version should be excluded when None"
    );
    assert!(
        json.get("agent_id").is_none(),
        "agent_id should be excluded when None"
    );
    assert!(
        json.get("matched_rule").is_none(),
        "matched_rule should be excluded when None"
    );
}

// ============================================================================
// SECTION 3: Decision Buffer Operations
// ============================================================================

/// Test basic buffer logging
#[test]
fn test_buffer_basic_logging() {
    let config = DecisionLogConfig {
        enabled: true,
        buffer_capacity: 100,
        ..Default::default()
    };

    let buffer = DecisionBuffer::new(config).unwrap();

    let entry = DecisionLogEntry::new(
        "alice".to_string(),
        "read".to_string(),
        "doc".to_string(),
        "allow".to_string(),
        "pol".to_string(),
        "policy".to_string(),
    );

    buffer.log(entry);

    let stats = buffer.stats();
    assert_eq!(stats.total_entries, 1);
    assert_eq!(stats.allow_count, 1);
    assert_eq!(stats.deny_count, 0);
}

/// Test buffer capacity limits
#[test]
fn test_buffer_capacity_limits() {
    let config = DecisionLogConfig {
        enabled: true,
        buffer_capacity: 10, // Small capacity
        ..Default::default()
    };

    let buffer = DecisionBuffer::new(config).unwrap();

    // Log more entries than capacity
    for i in 0..20 {
        let entry = DecisionLogEntry::new(
            format!("user-{}", i),
            "read".to_string(),
            "doc".to_string(),
            "allow".to_string(),
            "pol".to_string(),
            "policy".to_string(),
        );
        buffer.log(entry);
    }

    let stats = buffer.stats();
    assert_eq!(stats.total_entries, 20, "Should count all entries");
    assert!(
        stats.buffer_size <= 10,
        "Buffer size should not exceed capacity"
    );
    assert!(stats.dropped_entries > 0, "Should have dropped entries");
}

/// Test buffer filtering by decision type
#[test]
fn test_buffer_filtering_by_decision() {
    // Config that only logs denies
    let config = DecisionLogConfig {
        enabled: true,
        buffer_capacity: 100,
        log_allows: false,
        log_denies: true,
        ..Default::default()
    };

    let buffer = DecisionBuffer::new(config).unwrap();

    // Log some allows and denies
    for _ in 0..5 {
        let allow = DecisionLogEntry::new(
            "user".to_string(),
            "read".to_string(),
            "doc".to_string(),
            "allow".to_string(),
            "pol".to_string(),
            "policy".to_string(),
        );
        buffer.log(allow);

        let deny = DecisionLogEntry::new(
            "user".to_string(),
            "write".to_string(),
            "doc".to_string(),
            "deny".to_string(),
            "pol".to_string(),
            "policy".to_string(),
        );
        buffer.log(deny);
    }

    let stats = buffer.stats();
    // Only denies should be logged
    assert_eq!(stats.deny_count, 5, "Should count 5 denies");
    assert_eq!(stats.allow_count, 0, "Should not count allows");
    assert_eq!(stats.buffer_size, 5, "Buffer should only have denies");
}

/// Test buffer disabled mode
#[test]
fn test_buffer_disabled() {
    let config = DecisionLogConfig {
        enabled: false, // Disabled
        buffer_capacity: 100,
        ..Default::default()
    };

    let buffer = DecisionBuffer::new(config).unwrap();

    for _ in 0..10 {
        let entry = DecisionLogEntry::new(
            "user".to_string(),
            "read".to_string(),
            "doc".to_string(),
            "allow".to_string(),
            "pol".to_string(),
            "policy".to_string(),
        );
        buffer.log(entry);
    }

    let stats = buffer.stats();
    assert_eq!(stats.total_entries, 0, "Disabled buffer should not log");
    assert_eq!(stats.buffer_size, 0);
}

// ============================================================================
// SECTION 4: Buffer Query Operations
// ============================================================================

/// Test retrieving recent entries
#[test]
fn test_buffer_get_recent() {
    let config = DecisionLogConfig {
        enabled: true,
        buffer_capacity: 100,
        ..Default::default()
    };

    let buffer = DecisionBuffer::new(config).unwrap();

    for i in 0..10 {
        let entry = DecisionLogEntry::new(
            format!("user-{}", i),
            "read".to_string(),
            format!("doc-{}", i),
            "allow".to_string(),
            "pol".to_string(),
            "policy".to_string(),
        );
        buffer.log(entry);
    }

    let recent = buffer.get_recent(5);
    assert_eq!(recent.len(), 5, "Should return 5 recent entries");

    // Most recent should be last logged
    assert_eq!(recent[0].principal, "user-9");
    assert_eq!(recent[4].principal, "user-5");
}

/// Test filtering entries by principal
#[test]
fn test_buffer_filter_by_principal() {
    let config = DecisionLogConfig {
        enabled: true,
        buffer_capacity: 100,
        ..Default::default()
    };

    let buffer = DecisionBuffer::new(config).unwrap();

    // Log entries for different principals
    for i in 0..10 {
        let principal = if i % 2 == 0 { "alice" } else { "bob" };
        let entry = DecisionLogEntry::new(
            principal.to_string(),
            "read".to_string(),
            format!("doc-{}", i),
            "allow".to_string(),
            "pol".to_string(),
            "policy".to_string(),
        );
        buffer.log(entry);
    }

    let filter = DecisionFilter::new().with_principal("alice".to_string());
    let alice_entries = buffer.query(filter, 100);
    assert_eq!(alice_entries.len(), 5, "Should have 5 entries for alice");

    for entry in alice_entries {
        assert_eq!(entry.principal, "alice");
    }
}

/// Test filtering entries by decision
#[test]
fn test_buffer_filter_by_decision() {
    let config = DecisionLogConfig {
        enabled: true,
        buffer_capacity: 100,
        ..Default::default()
    };

    let buffer = DecisionBuffer::new(config).unwrap();

    // Log mix of allows and denies
    for i in 0..10 {
        let decision = if i % 3 == 0 { "deny" } else { "allow" };
        let entry = DecisionLogEntry::new(
            "user".to_string(),
            "read".to_string(),
            format!("doc-{}", i),
            decision.to_string(),
            "pol".to_string(),
            "policy".to_string(),
        );
        buffer.log(entry);
    }

    let filter = DecisionFilter::new().with_decision("deny".to_string());
    let denies = buffer.query(filter, 100);
    assert_eq!(denies.len(), 4, "Should have 4 deny entries"); // 0, 3, 6, 9

    for entry in denies {
        assert_eq!(entry.decision, "deny");
    }
}

// ============================================================================
// SECTION 5: Audit Trail Completeness
// ============================================================================

/// Test that all required audit fields are captured
#[test]
fn test_audit_trail_completeness() {
    let mut context = HashMap::new();
    context.insert("ip_address".to_string(), serde_json::json!("10.0.0.1"));
    context.insert("session_id".to_string(), serde_json::json!("sess-123"));
    context.insert("request_id".to_string(), serde_json::json!("req-456"));

    let entry = DecisionLogEntry::new(
        "alice".to_string(),
        "modify".to_string(),
        "sensitive-data".to_string(),
        "allow".to_string(),
        "policy-secure".to_string(),
        "data-modification-policy".to_string(),
    )
    .with_trace_id("trace-789".to_string())
    .with_context(context)
    .with_evaluation_time_ns(500)
    .with_agent_id("agent-prod-1".to_string())
    .with_matched_rule("allow-authorized-users".to_string())
    .with_policy_version("2.1.0".to_string());

    // Verify all audit-critical fields
    assert!(!entry.timestamp.is_empty(), "Timestamp required for audit");
    assert!(
        !entry.decision_id.is_empty(),
        "Decision ID required for audit"
    );
    assert!(!entry.principal.is_empty(), "Principal required for audit");
    assert!(!entry.action.is_empty(), "Action required for audit");
    assert!(!entry.resource.is_empty(), "Resource required for audit");
    assert!(!entry.decision.is_empty(), "Decision required for audit");
    assert!(!entry.policy_id.is_empty(), "Policy ID required for audit");
    assert!(
        !entry.policy_name.is_empty(),
        "Policy name required for audit"
    );

    // Context for forensics
    assert!(
        entry.context.contains_key("ip_address"),
        "IP address helpful for audit"
    );
    assert!(
        entry.context.contains_key("session_id"),
        "Session ID helpful for audit"
    );

    // Traceability
    assert!(entry.trace_id.is_some(), "Trace ID helps correlate logs");
    assert!(entry.agent_id.is_some(), "Agent ID helps identify source");
    assert!(
        entry.matched_rule.is_some(),
        "Matched rule helps understand decision"
    );
}

/// Test unique decision IDs across entries
#[test]
fn test_unique_decision_ids() {
    let mut decision_ids = std::collections::HashSet::new();

    for _ in 0..1000 {
        let entry = DecisionLogEntry::new(
            "user".to_string(),
            "read".to_string(),
            "doc".to_string(),
            "allow".to_string(),
            "pol".to_string(),
            "policy".to_string(),
        );

        assert!(
            decision_ids.insert(entry.decision_id.clone()),
            "Decision IDs must be unique"
        );
    }

    assert_eq!(decision_ids.len(), 1000);
}

/// Test timestamp ordering
#[test]
fn test_timestamp_ordering() {
    let config = DecisionLogConfig {
        enabled: true,
        buffer_capacity: 100,
        ..Default::default()
    };

    let buffer = DecisionBuffer::new(config).unwrap();

    // Log entries with small delays
    for i in 0..5 {
        let entry = DecisionLogEntry::new(
            format!("user-{}", i),
            "read".to_string(),
            "doc".to_string(),
            "allow".to_string(),
            "pol".to_string(),
            "policy".to_string(),
        );
        buffer.log(entry);
        std::thread::sleep(Duration::from_millis(10));
    }

    let entries = buffer.get_recent(5);

    // Verify timestamps are in descending order (most recent first)
    for i in 0..entries.len() - 1 {
        let t1 = chrono::DateTime::parse_from_rfc3339(&entries[i].timestamp).unwrap();
        let t2 = chrono::DateTime::parse_from_rfc3339(&entries[i + 1].timestamp).unwrap();
        assert!(
            t1 >= t2,
            "Entries should be ordered by timestamp descending"
        );
    }
}

// ============================================================================
// SECTION 6: Configuration Tests
// ============================================================================

/// Test default configuration
#[test]
fn test_default_config() {
    let config = DecisionLogConfig::default();

    // Note: enabled is false by default (opt-in for logging)
    assert!(!config.enabled, "Should be disabled by default (opt-in)");
    assert!(config.log_allows, "Should log allows by default");
    assert!(config.log_denies, "Should log denies by default");
    assert!(config.include_context, "Should include context by default");
    assert!(
        config.buffer_capacity > 0,
        "Should have positive buffer_capacity"
    );
}

/// Test configuration with context stripping
#[test]
fn test_config_strip_context() {
    let config = DecisionLogConfig {
        enabled: true,
        buffer_capacity: 100,
        include_context: false, // Strip context
        ..Default::default()
    };

    let buffer = DecisionBuffer::new(config).unwrap();

    let mut context = HashMap::new();
    context.insert("sensitive".to_string(), serde_json::json!("secret-data"));

    let entry = DecisionLogEntry::new(
        "user".to_string(),
        "read".to_string(),
        "doc".to_string(),
        "allow".to_string(),
        "pol".to_string(),
        "policy".to_string(),
    )
    .with_context(context);

    buffer.log(entry);

    let logged = buffer.get_recent(1);
    assert_eq!(logged.len(), 1);
    assert!(
        logged[0].context.is_empty(),
        "Context should be stripped when configured"
    );
}

// ============================================================================
// SECTION 7: Performance Tests
// ============================================================================

/// Test high-throughput logging
#[test]
fn test_high_throughput_logging() {
    let config = DecisionLogConfig {
        enabled: true,
        buffer_capacity: 10000,
        ..Default::default()
    };

    let buffer = DecisionBuffer::new(config).unwrap();

    let start = std::time::Instant::now();

    for i in 0..10000 {
        let entry = DecisionLogEntry::new(
            format!("user-{}", i % 100),
            "read".to_string(),
            format!("doc-{}", i),
            if i % 2 == 0 { "allow" } else { "deny" }.to_string(),
            "pol".to_string(),
            "policy".to_string(),
        );
        buffer.log(entry);
    }

    let elapsed = start.elapsed();

    // Should complete 10K logs in under 1 second
    assert!(
        elapsed.as_secs() < 1,
        "10K logs took too long: {:?}",
        elapsed
    );

    let stats = buffer.stats();
    assert_eq!(stats.total_entries, 10000);
    assert_eq!(stats.allow_count, 5000);
    assert_eq!(stats.deny_count, 5000);

    // Average logging time should be under 100 microseconds
    let avg_micros = elapsed.as_micros() / 10000;
    assert!(
        avg_micros < 100,
        "Average log time {} microseconds exceeds 100us",
        avg_micros
    );
}

/// Test concurrent logging
#[test]
fn test_concurrent_logging() {
    use std::sync::Arc;
    use std::thread;

    let config = DecisionLogConfig {
        enabled: true,
        buffer_capacity: 10000,
        ..Default::default()
    };

    let buffer = Arc::new(DecisionBuffer::new(config).unwrap());

    let mut handles = vec![];

    // Spawn 10 threads, each logging 1000 entries
    for thread_id in 0..10 {
        let buffer = Arc::clone(&buffer);
        let handle = thread::spawn(move || {
            for i in 0..1000 {
                let entry = DecisionLogEntry::new(
                    format!("user-{}-{}", thread_id, i),
                    "read".to_string(),
                    "doc".to_string(),
                    "allow".to_string(),
                    "pol".to_string(),
                    "policy".to_string(),
                );
                buffer.log(entry);
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().unwrap();
    }

    let stats = buffer.stats();
    assert_eq!(
        stats.total_entries, 10000,
        "Should log all entries from all threads"
    );
}
