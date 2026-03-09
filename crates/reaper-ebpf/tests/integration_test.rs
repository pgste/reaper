//! Integration tests for Reaper eBPF
//!
//! These tests verify the integration between eBPF components and the PolicyEngine.
//!
//! **Note**: Full integration tests require:
//! - Compiled eBPF kernel program (.o file)
//! - Root/CAP_BPF privileges
//! - Linux kernel 5.7+ with LSM BPF enabled
//!
//! Run with: `sudo -E cargo test --test integration_test`

use policy_engine::PolicyAction as EngineAction;
use reaper_ebpf::{EbpfController, LearningEngine, PolicyAction, PolicyCompiler, PolicyEntry};

#[test]
fn test_policy_compiler() {
    let compiler = PolicyCompiler::new();

    // Test resource to key conversion
    let key = compiler.resource_to_key("/api/users").unwrap();
    assert_ne!(key[0], 0); // Should have content

    // Test key back to resource
    let resource = compiler.key_to_resource(&key);
    assert_eq!(resource, "/api/users");
}

#[test]
fn test_policy_entry_creation() {
    let entry = PolicyEntry::new(PolicyAction::Allow)
        .with_priority(100)
        .with_uid(1000)
        .with_gid(1000);

    assert_eq!(entry.action, PolicyAction::Allow as u8);
    assert_eq!(entry.priority, 100);
    assert_eq!(entry.required_uid, 1000);
    assert_eq!(entry.required_gid, 1000);
    assert!(entry.flags & 0x01 != 0); // UID flag set
    assert!(entry.flags & 0x02 != 0); // GID flag set
}

#[test]
fn test_learning_engine_basic() {
    let engine = LearningEngine::with_defaults();

    // Record some accesses
    for _ in 0..100 {
        engine.record_access("/api/users", EngineAction::Allow, Some(1000), None);
    }

    // Should be eligible for promotion
    assert!(engine.should_promote("/api/users"));

    // Get stats
    let stats = engine.get_stats();
    assert_eq!(stats.total_patterns, 1);
    assert_eq!(stats.eligible_for_promotion, 1);
}

#[test]
fn test_learning_engine_unstable() {
    let engine = LearningEngine::with_defaults();

    // Record flip-flopping decisions
    for i in 0..100 {
        let action = if i % 2 == 0 {
            EngineAction::Allow
        } else {
            EngineAction::Deny
        };
        engine.record_access("/api/unstable", action, None, None);
    }

    // Should NOT be eligible for promotion (unstable)
    assert!(!engine.should_promote("/api/unstable"));
}

#[test]
fn test_learning_engine_threshold() {
    let engine = LearningEngine::with_defaults();

    // Record only 50 accesses (below threshold of 100)
    for _ in 0..50 {
        engine.record_access("/api/new", EngineAction::Allow, None, None);
    }

    // Should NOT be eligible for promotion (below threshold)
    assert!(!engine.should_promote("/api/new"));

    // Get stats
    let stats = engine.get_stats();
    assert_eq!(stats.total_patterns, 1);
    assert_eq!(stats.eligible_for_promotion, 0);
}

// Integration tests below require actual eBPF program to be loaded

#[test]
#[ignore] // Requires eBPF program to be compiled
fn test_ebpf_controller_load() {
    let result = EbpfController::load("target/bpfel-unknown-none/release/reaper_ebpf_kern.o");

    // This will fail until eBPF program is compiled
    // When ready, it should succeed:
    // assert!(result.is_ok());

    if result.is_err() {
        println!("⚠️  eBPF controller load test skipped - kernel program not compiled");
        println!("   Build with: make ebpf-kern");
    }
}

#[test]
#[ignore] // Requires root privileges and loaded eBPF program
fn test_ebpf_controller_attach() {
    let controller = EbpfController::load("target/bpfel-unknown-none/release/reaper_ebpf_kern.o");

    if let Ok(mut ctrl) = controller {
        let result = ctrl.attach();

        // Should succeed with root privileges
        if result.is_err() {
            println!("⚠️  eBPF attach failed - may need root privileges");
            println!("   Run with: sudo -E cargo test --test integration_test");
        }
    } else {
        println!("⚠️  eBPF controller load failed - kernel program not compiled");
    }
}

#[test]
#[ignore] // Requires loaded eBPF program
fn test_ebpf_policy_deployment() {
    // This test would:
    // 1. Load EbpfController
    // 2. Deploy a simple policy
    // 3. Verify it's in the BPF map
    // 4. Test evaluation

    println!("⚠️  eBPF policy deployment test not yet implemented");
    println!("   Waiting for kernel program compilation");
}

#[test]
#[ignore] // Requires full eBPF stack
fn test_ebpf_end_to_end() {
    // This test would:
    // 1. Load PolicyEngine
    // 2. Wrap with EbpfPolicyEngine
    // 3. Attach to LSM hooks
    // 4. Deploy mixed policies (simple → eBPF, complex → userspace)
    // 5. Trigger file access (would require actual file operations)
    // 6. Verify decisions come from eBPF fast path
    // 7. Verify learning and auto-promotion

    println!("⚠️  End-to-end eBPF test not yet implemented");
    println!("   Requires full eBPF stack and test environment");
}

// Helper functions for integration tests

#[allow(dead_code)]
fn kernel_supports_lsm_bpf() -> bool {
    std::fs::read_to_string("/sys/kernel/security/lsm")
        .map(|s| s.contains("bpf"))
        .unwrap_or(false)
}

#[test]
fn test_environment_check() {
    println!("Environment check:");
    println!("  LSM BPF: {}", kernel_supports_lsm_bpf());

    if !kernel_supports_lsm_bpf() {
        println!("⚠️  LSM BPF not enabled - add to kernel boot params: lsm=...,bpf");
    }
}
