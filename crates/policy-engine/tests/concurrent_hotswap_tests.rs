//! Concurrent hot-swap tests for the policy engine
//!
//! These tests verify that the policy engine handles concurrent operations safely,
//! including simultaneous policy deployments and evaluations.

use policy_engine::{EnhancedPolicy, PolicyAction, PolicyEngine, PolicyRequest, PolicyRule};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Test concurrent hot-swaps from multiple threads
#[tokio::test]
async fn test_concurrent_hot_swap_many_threads() {
    let engine = Arc::new(PolicyEngine::new());
    let swap_count = Arc::new(AtomicUsize::new(0));
    let error_count = Arc::new(AtomicUsize::new(0));

    // Create initial policy
    let policy = EnhancedPolicy::new(
        "concurrent-swap".to_string(),
        "Concurrent swap test".to_string(),
        vec![PolicyRule {
            action: PolicyAction::Deny,
            resource: "*".to_string(),
            conditions: vec![],
        }],
    );
    let policy_id = policy.id;
    engine.deploy_policy(policy).unwrap();

    // Spawn many tasks that concurrently swap the policy
    let num_tasks = 100;
    let swaps_per_task = 10;

    let mut handles = Vec::new();

    for task_id in 0..num_tasks {
        let engine = Arc::clone(&engine);
        let swap_count = Arc::clone(&swap_count);
        let error_count = Arc::clone(&error_count);

        let handle = tokio::spawn(async move {
            for swap_num in 0..swaps_per_task {
                // Alternate between Allow and Deny
                let action = if (task_id + swap_num) % 2 == 0 {
                    PolicyAction::Allow
                } else {
                    PolicyAction::Deny
                };

                let mut policy = EnhancedPolicy::new(
                    "concurrent-swap".to_string(),
                    format!("Swap from task {} iteration {}", task_id, swap_num),
                    vec![PolicyRule {
                        action,
                        resource: "*".to_string(),
                        conditions: vec![],
                    }],
                );
                // Preserve the policy ID for hot-swap
                policy.id = policy_id;

                match engine.deploy_policy(policy) {
                    Ok(_) => {
                        swap_count.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(_) => {
                        error_count.fetch_add(1, Ordering::Relaxed);
                    }
                }

                // Small yield to increase interleaving
                tokio::task::yield_now().await;
            }
        });

        handles.push(handle);
    }

    // Wait for all tasks
    for handle in handles {
        handle.await.unwrap();
    }

    let total_swaps = swap_count.load(Ordering::Relaxed);
    let total_errors = error_count.load(Ordering::Relaxed);

    // All swaps should succeed (lock-free design)
    assert_eq!(
        total_swaps,
        num_tasks * swaps_per_task,
        "Expected {} swaps, got {} (errors: {})",
        num_tasks * swaps_per_task,
        total_swaps,
        total_errors
    );
    assert_eq!(
        total_errors, 0,
        "Expected no errors during concurrent swaps"
    );

    // Final policy should exist
    let final_policy = engine.get_policy(&policy_id);
    assert!(
        final_policy.is_some(),
        "Policy should still exist after concurrent swaps"
    );
}

/// Test concurrent evaluations during hot-swap
#[tokio::test]
async fn test_evaluation_during_hot_swap() {
    let engine = Arc::new(PolicyEngine::new());
    let eval_count = Arc::new(AtomicUsize::new(0));
    let allow_count = Arc::new(AtomicUsize::new(0));
    let deny_count = Arc::new(AtomicUsize::new(0));

    // Create initial policy (Deny)
    let policy = EnhancedPolicy::new(
        "eval-during-swap".to_string(),
        "Evaluation during swap test".to_string(),
        vec![PolicyRule {
            action: PolicyAction::Deny,
            resource: "*".to_string(),
            conditions: vec![],
        }],
    );
    let policy_id = policy.id;
    engine.deploy_policy(policy).unwrap();

    let running = Arc::new(std::sync::atomic::AtomicBool::new(true));

    // Spawn evaluation tasks
    let mut eval_handles = Vec::new();
    for _ in 0..10 {
        let engine = Arc::clone(&engine);
        let eval_count = Arc::clone(&eval_count);
        let allow_count = Arc::clone(&allow_count);
        let deny_count = Arc::clone(&deny_count);
        let running = Arc::clone(&running);

        let handle = tokio::spawn(async move {
            while running.load(Ordering::Relaxed) {
                let request = PolicyRequest {
                    resource: "test".to_string(),
                    action: "read".to_string(),
                    context: HashMap::new(),

                    ..Default::default()
                };

                if let Some(policy) = engine.get_policy(&policy_id) {
                    if let Some(evaluator) = policy.evaluator.as_ref() {
                        match evaluator.evaluate(&request) {
                            Ok(PolicyAction::Allow) => {
                                allow_count.fetch_add(1, Ordering::Relaxed);
                            }
                            Ok(PolicyAction::Deny) => {
                                deny_count.fetch_add(1, Ordering::Relaxed);
                            }
                            _ => {}
                        }
                        eval_count.fetch_add(1, Ordering::Relaxed);
                    }
                }

                tokio::task::yield_now().await;
            }
        });

        eval_handles.push(handle);
    }

    // Spawn swap task that alternates policy
    let swap_engine = Arc::clone(&engine);
    let swap_handle = tokio::spawn(async move {
        for i in 0..50 {
            let action = if i % 2 == 0 {
                PolicyAction::Allow
            } else {
                PolicyAction::Deny
            };

            let mut policy = EnhancedPolicy::new(
                "eval-during-swap".to_string(),
                format!("Swap iteration {}", i),
                vec![PolicyRule {
                    action,
                    resource: "*".to_string(),
                    conditions: vec![],
                }],
            );
            policy.id = policy_id;

            swap_engine.deploy_policy(policy).unwrap();

            // Give evaluators time to run
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    });

    // Wait for swaps to complete
    swap_handle.await.unwrap();

    // Stop evaluators
    running.store(false, Ordering::Relaxed);
    for handle in eval_handles {
        handle.await.unwrap();
    }

    let total_evals = eval_count.load(Ordering::Relaxed);
    let allows = allow_count.load(Ordering::Relaxed);
    let denies = deny_count.load(Ordering::Relaxed);

    // Should have many evaluations
    assert!(
        total_evals > 100,
        "Expected many evaluations, got {}",
        total_evals
    );

    // Should see both allows and denies (policy was swapped between them)
    assert!(allows > 0, "Expected some allows, got {}", allows);
    assert!(denies > 0, "Expected some denies, got {}", denies);

    println!(
        "Concurrent evaluation test: {} total evals ({} allows, {} denies)",
        total_evals, allows, denies
    );
}

/// Test that readers always see consistent policy state (no torn reads)
#[tokio::test]
async fn test_consistency_during_hot_swap() {
    let engine = Arc::new(PolicyEngine::new());
    let inconsistency_count = Arc::new(AtomicUsize::new(0));

    // Create policy with multiple rules that should be consistent
    let policy = EnhancedPolicy::new(
        "consistency-test".to_string(),
        "Consistency test - version 1".to_string(),
        vec![
            PolicyRule {
                action: PolicyAction::Allow,
                resource: "resource-a".to_string(),
                conditions: vec![],
            },
            PolicyRule {
                action: PolicyAction::Allow,
                resource: "resource-b".to_string(),
                conditions: vec![],
            },
        ],
    );
    let policy_id = policy.id;
    engine.deploy_policy(policy).unwrap();

    let running = Arc::new(std::sync::atomic::AtomicBool::new(true));

    // Spawn readers that check for consistency
    let mut reader_handles = Vec::new();
    for _ in 0..20 {
        let engine = Arc::clone(&engine);
        let inconsistency_count = Arc::clone(&inconsistency_count);
        let running = Arc::clone(&running);

        let handle = tokio::spawn(async move {
            while running.load(Ordering::Relaxed) {
                if let Some(policy) = engine.get_policy(&policy_id) {
                    // Check that all rules have the same action (they should)
                    // This verifies we don't see a torn read with mixed old/new rules
                    if policy.rules.len() >= 2 {
                        let first_action = &policy.rules[0].action;
                        for rule in &policy.rules[1..] {
                            if &rule.action != first_action {
                                inconsistency_count.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }

                    // Also verify name matches expected pattern
                    if !policy.name.starts_with("consistency-test") {
                        inconsistency_count.fetch_add(1, Ordering::Relaxed);
                    }
                }

                tokio::task::yield_now().await;
            }
        });

        reader_handles.push(handle);
    }

    // Spawn writer that alternates between two consistent states
    let swap_engine = Arc::clone(&engine);
    let swap_handle = tokio::spawn(async move {
        for i in 0..100 {
            let action = if i % 2 == 0 {
                PolicyAction::Allow
            } else {
                PolicyAction::Deny
            };

            // Both rules have the same action - this is the consistent state
            let mut policy = EnhancedPolicy::new(
                "consistency-test".to_string(),
                format!("Consistency test - version {}", i + 2),
                vec![
                    PolicyRule {
                        action: action.clone(),
                        resource: "resource-a".to_string(),
                        conditions: vec![],
                    },
                    PolicyRule {
                        action,
                        resource: "resource-b".to_string(),
                        conditions: vec![],
                    },
                ],
            );
            policy.id = policy_id;

            swap_engine.deploy_policy(policy).unwrap();

            tokio::task::yield_now().await;
        }
    });

    // Wait for swaps
    swap_handle.await.unwrap();

    // Stop readers
    running.store(false, Ordering::Relaxed);
    for handle in reader_handles {
        handle.await.unwrap();
    }

    let inconsistencies = inconsistency_count.load(Ordering::Relaxed);
    assert_eq!(
        inconsistencies, 0,
        "Detected {} inconsistent reads (torn reads)",
        inconsistencies
    );
}

/// Test high-contention scenario with many concurrent swaps on same policy
#[tokio::test]
async fn test_high_contention_same_policy() {
    let engine = Arc::new(PolicyEngine::new());

    // Create initial policy
    let policy = EnhancedPolicy::new(
        "contention-test".to_string(),
        "High contention test".to_string(),
        vec![PolicyRule {
            action: PolicyAction::Deny,
            resource: "*".to_string(),
            conditions: vec![],
        }],
    );
    let policy_id = policy.id;
    engine.deploy_policy(policy).unwrap();

    let success_count = Arc::new(AtomicUsize::new(0));
    let start_barrier = Arc::new(tokio::sync::Barrier::new(50));

    // Spawn 50 tasks that all try to swap at exactly the same time
    let mut handles = Vec::new();
    for task_id in 0..50 {
        let engine = Arc::clone(&engine);
        let success_count = Arc::clone(&success_count);
        let barrier = Arc::clone(&start_barrier);

        let handle = tokio::spawn(async move {
            // Wait for all tasks to be ready
            barrier.wait().await;

            // Now all try to swap simultaneously
            let mut policy = EnhancedPolicy::new(
                "contention-test".to_string(),
                format!("Swap from task {}", task_id),
                vec![PolicyRule {
                    action: PolicyAction::Allow,
                    resource: format!("resource-{}", task_id),
                    conditions: vec![],
                }],
            );
            policy.id = policy_id;

            if engine.deploy_policy(policy).is_ok() {
                success_count.fetch_add(1, Ordering::Relaxed);
            }
        });

        handles.push(handle);
    }

    for handle in handles {
        handle.await.unwrap();
    }

    // All 50 swaps should succeed (lock-free allows this)
    let successes = success_count.load(Ordering::Relaxed);
    assert_eq!(successes, 50, "All swaps should succeed, got {}", successes);

    // Policy should exist and be valid
    let final_policy = engine.get_policy(&policy_id);
    assert!(final_policy.is_some(), "Policy should still exist");
}

/// Test concurrent operations on multiple different policies
#[tokio::test]
async fn test_concurrent_multi_policy_operations() {
    let engine = Arc::new(PolicyEngine::new());
    let num_policies = 20;

    // Create multiple policies
    let mut policy_ids = Vec::new();
    for i in 0..num_policies {
        let policy = EnhancedPolicy::new(
            format!("multi-policy-{}", i),
            format!("Multi policy test {}", i),
            vec![PolicyRule {
                action: PolicyAction::Deny,
                resource: "*".to_string(),
                conditions: vec![],
            }],
        );
        policy_ids.push(policy.id);
        engine.deploy_policy(policy).unwrap();
    }

    let operation_count = Arc::new(AtomicUsize::new(0));

    // Spawn tasks that operate on random policies
    let mut handles = Vec::new();
    for _ in 0..50 {
        let engine = Arc::clone(&engine);
        let policy_ids = policy_ids.clone();
        let operation_count = Arc::clone(&operation_count);

        let handle = tokio::spawn(async move {
            for _ in 0..100 {
                // Pick a random policy
                let idx = rand_index(policy_ids.len());
                let policy_id = policy_ids[idx];

                // Randomly either read or write
                if rand_index(2) == 0 {
                    // Read
                    let _ = engine.get_policy(&policy_id);
                } else {
                    // Write
                    let mut policy = EnhancedPolicy::new(
                        format!("multi-policy-{}", idx),
                        "Updated".to_string(),
                        vec![PolicyRule {
                            action: PolicyAction::Allow,
                            resource: "*".to_string(),
                            conditions: vec![],
                        }],
                    );
                    policy.id = policy_id;
                    let _ = engine.deploy_policy(policy);
                }

                operation_count.fetch_add(1, Ordering::Relaxed);
                tokio::task::yield_now().await;
            }
        });

        handles.push(handle);
    }

    for handle in handles {
        handle.await.unwrap();
    }

    let total_ops = operation_count.load(Ordering::Relaxed);
    assert_eq!(
        total_ops,
        50 * 100,
        "All operations should complete: {}",
        total_ops
    );

    // All policies should still be accessible
    for policy_id in &policy_ids {
        assert!(
            engine.get_policy(policy_id).is_some(),
            "Policy {:?} should still exist",
            policy_id
        );
    }
}

/// Simple pseudo-random index generator (not cryptographic, just for test distribution)
fn rand_index(max: usize) -> usize {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos() as usize;
    nanos % max
}
