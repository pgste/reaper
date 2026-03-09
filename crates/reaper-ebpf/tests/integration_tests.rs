//! Integration Tests for eBPF Policy Engine
//!
//! These tests verify the complete eBPF integration without requiring
//! actual kernel deployment. They test:
//! - Policy compilation
//! - Bundle deployment logic
//! - Event handling
//! - Learning engine integration
//!
//! Run with: cargo test -p reaper-ebpf --test integration_tests

use policy_engine::reap::{
    BundleFormat, Decision, Policy, PolicyBundle, ReapCondition as Condition, ReapRule as Rule,
};
use policy_engine::PolicyAction;
use reaper_ebpf::{PolicyCompiler, PolicyEntry};
use std::collections::HashMap;

#[test]
fn test_policy_compiler_wildcard() {
    let compiler = PolicyCompiler::new();

    // Test wildcard resource
    let (key, entry) = compiler
        .compile_decision("*", PolicyAction::Allow, None, None, 0)
        .expect("Should compile wildcard");

    // Wildcard should have 0xFF marker
    assert_eq!(key[0], 0xFF);
    assert_eq!(entry.action, 1); // Allow = 1
    assert_eq!(entry.priority, 0);
    assert_eq!(entry.flags, 0); // No UID/GID checks
}

#[test]
fn test_policy_compiler_exact_path() {
    let compiler = PolicyCompiler::new();

    // Test exact path
    let resource = "/api/users";
    let (key, entry) = compiler
        .compile_decision(resource, PolicyAction::Deny, None, None, 5)
        .expect("Should compile exact path");

    // Key should contain the path
    let recovered = compiler.key_to_resource(&key);
    assert_eq!(recovered, resource);

    assert_eq!(entry.action, 0); // Deny = 0
    assert_eq!(entry.priority, 5);
}

#[test]
fn test_policy_compiler_with_uid_gid() {
    let compiler = PolicyCompiler::new()
        .with_default_uid(1000)
        .with_default_gid(1000);

    let (key, entry) = compiler
        .compile_decision("/etc/passwd", PolicyAction::Deny, None, None, 0)
        .expect("Should compile with UID/GID");

    // Should have UID and GID checks enabled
    assert_eq!(entry.flags, 0x03); // 0x01 (UID) | 0x02 (GID)
    assert_eq!(entry.required_uid, 1000);
    assert_eq!(entry.required_gid, 1000);

    let recovered = compiler.key_to_resource(&key);
    assert_eq!(recovered, "/etc/passwd");
}

#[test]
fn test_policy_compiler_override_uid_gid() {
    let compiler = PolicyCompiler::new()
        .with_default_uid(1000)
        .with_default_gid(1000);

    // Override with specific UID/GID
    let (_, entry) = compiler
        .compile_decision("/tmp/file", PolicyAction::Allow, Some(2000), Some(2000), 10)
        .expect("Should compile with override");

    assert_eq!(entry.flags, 0x03);
    assert_eq!(entry.required_uid, 2000); // Override, not default
    assert_eq!(entry.required_gid, 2000);
}

#[test]
fn test_policy_entry_construction() {
    let entry = PolicyEntry::new(reaper_ebpf::PolicyAction::Allow)
        .with_priority(5)
        .with_uid(1000)
        .with_gid(1000);

    assert_eq!(entry.action, 1); // Allow
    assert_eq!(entry.priority, 5);
    assert_eq!(entry.flags, 0x03); // UID and GID checks
    assert_eq!(entry.required_uid, 1000);
    assert_eq!(entry.required_gid, 1000);
}

#[tokio::test]
async fn test_bundle_deployment_routing() {
    // Create a policy bundle with mixed rule types
    let policy = Policy {
        name: "test_policy".to_string(),
        metadata: HashMap::new(),
        default_decision: Decision::Deny,
        rules: vec![
            // Simple rule - should go to eBPF
            Rule {
                name: "allow_all".to_string(),
                decision: Decision::Allow,
                condition: Condition::True,
            },
            // Complex rule - should stay in userspace
            // (using False as a stand-in for more complex conditions that would require
            // private ast types)
            Rule {
                name: "deny_all".to_string(),
                decision: Decision::Deny,
                condition: Condition::False,
            },
        ],
    };

    let bundle = PolicyBundle {
        metadata: BundleFormat {
            version: 1,
            compiled_at: 0,
            policy_name: "test_policy".to_string(),
            policy_version: Some("1.0.0".to_string()),
            source_checksum: 0,
        },
        policy,
    };

    // Note: We can't test actual deployment without a compiled eBPF program
    // This test verifies the bundle structure is correct
    assert_eq!(bundle.policy.rules.len(), 2);
    assert!(matches!(bundle.policy.rules[0].condition, Condition::True));
    assert!(matches!(bundle.policy.rules[1].condition, Condition::False));
}

#[test]
fn test_learning_engine_basics() {
    use reaper_ebpf::LearningEngine;

    let engine = LearningEngine::with_defaults();
    let stats = engine.get_stats();

    assert_eq!(stats.total_patterns, 0);
    assert_eq!(stats.promoted_patterns, 0);
}

#[test]
fn test_learning_engine_recording() {
    use reaper_ebpf::LearningEngine;

    // Use low thresholds for testing (5 promotion threshold, 5 stability window)
    let engine = LearningEngine::new(5, 5);

    // Record 100 accesses to the same resource (need 100 for stability check)
    for _ in 0..100 {
        engine.record_access("/api/users", PolicyAction::Allow, Some(1000), Some(1000));
    }

    let stats = engine.get_stats();
    assert_eq!(stats.total_patterns, 1);

    // Check if eligible for promotion (requires count >= threshold AND stable)
    let should_promote = engine.should_promote("/api/users");
    assert!(
        should_promote,
        "Should be eligible after 100 consistent accesses"
    );
}

#[test]
fn test_learning_engine_multiple_resources() {
    use reaper_ebpf::LearningEngine;

    let engine = LearningEngine::with_defaults();

    // Access different resources
    engine.record_access("/api/users", PolicyAction::Allow, Some(1000), Some(1000));
    engine.record_access("/api/posts", PolicyAction::Allow, Some(1000), Some(1000));
    engine.record_access("/api/comments", PolicyAction::Deny, Some(2000), Some(2000));

    let stats = engine.get_stats();
    assert_eq!(stats.total_patterns, 3);
}

#[test]
fn test_policy_event_conversion() {
    use reaper_ebpf::PolicyEvent;

    let mut event = PolicyEvent {
        pid: 1234,
        uid: 1000,
        gid: 1000,
        path: [0u8; 256],
        path_len: 0,
        action: 0,
        timestamp_ns: 1234567890000000000,
    };

    // Set path
    let path = b"comm:test_process";
    for (i, &byte) in path.iter().enumerate() {
        event.path[i] = byte;
    }
    event.path_len = path.len() as u32;

    // Test path extraction
    let path_str = event.path_str();
    assert_eq!(path_str, "comm:test_process");

    // Test conversion to PolicyRequest
    let request = event.to_policy_request();
    assert_eq!(request.resource, "comm:test_process");
    assert_eq!(request.action, "open");
    assert_eq!(request.context.get("uid"), Some(&"1000".to_string()));
    assert_eq!(request.context.get("gid"), Some(&"1000".to_string()));
    assert_eq!(request.context.get("pid"), Some(&"1234".to_string()));
}

#[test]
fn test_ebpf_stats() {
    use reaper_ebpf::EbpfStats;

    let stats = EbpfStats {
        fast_path: 800,
        slow_path: 200,
        denials: 50,
        allows: 950,
        errors: 5,
    };

    assert_eq!(stats.fast_path_percent(), 80.0);
    assert_eq!(stats.total(), 1000);
}

#[test]
fn test_combined_stats() {
    use reaper_ebpf::CombinedStats;

    let stats = CombinedStats {
        fast_path_evaluations: 800,
        slow_path_evaluations: 200,
        fast_path_percent: 80.0,
        denials: 50,
        allows: 950,
        errors: 5,
        promoted_policies: 10,
        ebpf_policy_count: 15,
        userspace_policy_count: 5,
    };

    assert_eq!(stats.fast_path_percent, 80.0);
    assert_eq!(stats.promoted_policies, 10);
    assert_eq!(stats.ebpf_policy_count, 15);
}

#[test]
fn test_max_path_len() {
    use reaper_ebpf::MAX_PATH_LEN;

    // Verify MAX_PATH_LEN matches kernel and userspace
    assert_eq!(MAX_PATH_LEN, 256);
}

#[test]
fn test_max_context_len() {
    use reaper_ebpf::{MAX_CONTEXT_KEY_LEN, MAX_CONTEXT_VALUE_LEN};

    // Verify context limits
    assert_eq!(MAX_CONTEXT_KEY_LEN, 64);
    assert_eq!(MAX_CONTEXT_VALUE_LEN, 256);
}

#[test]
fn test_policy_compiler_long_path() {
    let compiler = PolicyCompiler::new();

    // Test path longer than MAX_PATH_LEN
    let long_path = "/".to_string() + &"a".repeat(300);
    let (key, _) = compiler
        .compile_decision(&long_path, PolicyAction::Allow, None, None, 0)
        .expect("Should compile long path");

    let recovered = compiler.key_to_resource(&key);
    // Should be truncated to MAX_PATH_LEN - 1 (reserve for null terminator)
    assert!(recovered.len() < long_path.len());
    assert!(recovered.len() <= 256);
}

#[test]
fn test_policy_action_conversion() {
    use reaper_ebpf::PolicyAction as EbpfAction;

    // These would be converted in the compiler
    let ebpf_allow = EbpfAction::Allow;
    let ebpf_deny = EbpfAction::Deny;
    let ebpf_log = EbpfAction::Log;

    assert_eq!(ebpf_allow as u8, 1);
    assert_eq!(ebpf_deny as u8, 0);
    assert_eq!(ebpf_log as u8, 2);
}

// Mock test for slow path handler (without actual ring buffer)
#[tokio::test]
async fn test_slow_path_stats() {
    use reaper_ebpf::SlowPathStats;

    let stats = SlowPathStats {
        events_processed: 100,
        events_errors: 5,
        auto_promote_enabled: true,
    };

    assert_eq!(stats.events_processed, 100);
    assert_eq!(stats.events_errors, 5);
    assert!(stats.auto_promote_enabled);
}

#[test]
fn test_policy_entry_flags() {
    use reaper_ebpf::PolicyEntry;

    // No checks
    let entry = PolicyEntry::new(reaper_ebpf::PolicyAction::Allow);
    assert_eq!(entry.flags, 0);

    // UID check only
    let entry = PolicyEntry::new(reaper_ebpf::PolicyAction::Allow).with_uid(1000);
    assert_eq!(entry.flags, 0x01);

    // GID check only
    let entry = PolicyEntry::new(reaper_ebpf::PolicyAction::Allow).with_gid(1000);
    assert_eq!(entry.flags, 0x02);

    // Both checks
    let entry = PolicyEntry::new(reaper_ebpf::PolicyAction::Allow)
        .with_uid(1000)
        .with_gid(1000);
    assert_eq!(entry.flags, 0x03);
}
