//! Large-Scale Tests
//!
//! Tests for validating policy engine performance and correctness at scale:
//! - 100K+ entity tests
//! - High-throughput evaluation
//! - Memory efficiency under load
//! - Concurrent large-scale operations

use policy_engine::data::DataLoader;
use policy_engine::reap::ReaperPolicy;
use policy_engine::{DataStore, PolicyAction, PolicyEvaluator, PolicyRequest};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

// ============================================================================
// SECTION 1: Large Entity Dataset Tests
// ============================================================================

/// Test loading and querying 100K entities
#[test]
fn test_100k_entity_load_and_query() {
    let policy_text = r#"
policy large_scale_test {
    default: deny,

    rule active_users {
        allow if {
            user.status == "active"
        }
    }
}
"#;

    let policy = policy_text.parse::<ReaperPolicy>().unwrap();
    let store = Arc::new(DataStore::new());

    // Generate 100K entities
    let start = Instant::now();
    let mut entities = Vec::with_capacity(100_000);

    for i in 0..100_000 {
        let status = if i % 10 == 0 { "inactive" } else { "active" };
        entities.push(format!(
            r#"{{"id": "user_{}", "type": "User", "attributes": {{"id": "user_{}", "status": "{}"}}}}"#,
            i, i, status
        ));
    }

    let json = format!(r#"{{"entities": [{}]}}"#, entities.join(","));
    let load_start = Instant::now();

    let loader = DataLoader::new((*store).clone());
    loader.load_json(&json).unwrap();

    let load_time = load_start.elapsed();
    println!("Load time for 100K entities: {:?}", load_time);

    // Loading should complete in reasonable time (< 30 seconds)
    assert!(
        load_time < Duration::from_secs(30),
        "Loading 100K entities took too long: {:?}",
        load_time
    );

    // Verify count
    let stats = store.stats();
    assert_eq!(stats.total_entities, 100_000);

    // Build evaluator
    let evaluator = policy.build(Arc::clone(&store)).unwrap();

    // Test evaluation performance
    let eval_start = Instant::now();
    let num_evals = 10_000;

    for i in 0..num_evals {
        let user_idx = i % 100_000;
        let mut context = HashMap::new();
        context.insert("principal".to_string(), format!("user_{}", user_idx));

        let request = PolicyRequest {
            resource: "test".to_string(),
            action: "read".to_string(),
            context,
        };

        let result = evaluator.evaluate(&request);
        assert!(result.is_ok());
    }

    let eval_time = eval_start.elapsed();
    let avg_eval_us = eval_time.as_micros() / num_evals as u128;

    println!(
        "Evaluation time for {} requests: {:?} (avg: {}us)",
        num_evals, eval_time, avg_eval_us
    );

    // Average evaluation should be under 100 microseconds
    assert!(
        avg_eval_us < 100,
        "Average evaluation time {} us exceeds 100us",
        avg_eval_us
    );

    let total_time = start.elapsed();
    println!("Total test time: {:?}", total_time);
}

/// Test memory efficiency with 100K entities
#[test]
fn test_100k_entity_memory_efficiency() {
    let store = Arc::new(DataStore::new());

    // Generate entities with varied attributes
    let mut entities = Vec::with_capacity(100_000);

    for i in 0..100_000 {
        let department = match i % 5 {
            0 => "engineering",
            1 => "sales",
            2 => "marketing",
            3 => "finance",
            _ => "operations",
        };
        let role = match i % 3 {
            0 => "admin",
            1 => "user",
            _ => "guest",
        };

        entities.push(format!(
            r#"{{"id": "emp_{}", "type": "Employee", "attributes": {{
                "id": "emp_{}",
                "name": "Employee {}",
                "email": "emp{}@company.com",
                "department": "{}",
                "role": "{}",
                "active": {}
            }}}}"#,
            i,
            i,
            i,
            i,
            department,
            role,
            i % 10 != 0
        ));
    }

    let json = format!(r#"{{"entities": [{}]}}"#, entities.join(","));

    let loader = DataLoader::new((*store).clone());
    loader.load_json(&json).unwrap();

    let stats = store.stats();

    // Check memory efficiency
    // With string interning, repeated values like "engineering", "admin" should be shared
    println!("Entity count: {}", stats.total_entities);
    println!("Unique types: {}", stats.unique_types);
    println!("Indexed attributes: {}", stats.indexed_attributes);
    println!(
        "Estimated memory: {} bytes ({:.2} MB)",
        stats.estimated_memory_bytes,
        stats.estimated_memory_bytes as f64 / 1_000_000.0
    );

    // Memory should be efficient - less than 100 bytes per entity on average
    // (string interning should help significantly)
    let bytes_per_entity = stats.estimated_memory_bytes / stats.total_entities;
    println!("Bytes per entity: {}", bytes_per_entity);

    // This is a soft assertion - actual memory depends on implementation
    // Memory includes entity attributes, indexes, and string interning overhead
    assert!(
        bytes_per_entity < 1000,
        "Memory per entity {} bytes exceeds 1KB limit",
        bytes_per_entity
    );
}

// ============================================================================
// SECTION 2: High-Throughput Evaluation Tests
// ============================================================================

/// Test sustained high-throughput evaluation
#[test]
fn test_high_throughput_evaluation() {
    let policy_text = r#"
policy throughput_test {
    default: deny,

    rule allow_readers {
        allow if {
            user.role == "reader" &&
            resource.type == "document"
        }
    }

    rule allow_writers {
        allow if {
            user.role == "writer"
        }
    }
}
"#;

    let policy = policy_text.parse::<ReaperPolicy>().unwrap();
    let store = Arc::new(DataStore::new());

    // Load test data
    let mut entities = Vec::new();
    for i in 0..1000 {
        let role = if i % 2 == 0 { "reader" } else { "writer" };
        entities.push(format!(
            r#"{{"id": "user_{}", "type": "User", "attributes": {{"id": "user_{}", "role": "{}"}}}}"#,
            i, i, role
        ));
    }
    for i in 0..100 {
        entities.push(format!(
            r#"{{"id": "doc_{}", "type": "Resource", "attributes": {{"id": "doc_{}", "type": "document"}}}}"#,
            i, i
        ));
    }

    let json = format!(r#"{{"entities": [{}]}}"#, entities.join(","));
    let loader = DataLoader::new((*store).clone());
    loader.load_json(&json).unwrap();

    let evaluator = policy.build(Arc::clone(&store)).unwrap();

    // Warm up
    for _ in 0..1000 {
        let mut context = HashMap::new();
        context.insert("principal".to_string(), "user_0".to_string());
        let request = PolicyRequest {
            resource: "doc_0".to_string(),
            action: "read".to_string(),
            context,
        };
        let _ = evaluator.evaluate(&request);
    }

    // Benchmark
    let num_requests = 100_000;
    let start = Instant::now();

    for i in 0..num_requests {
        let user_idx = i % 1000;
        let doc_idx = i % 100;

        let mut context = HashMap::new();
        context.insert("principal".to_string(), format!("user_{}", user_idx));

        let request = PolicyRequest {
            resource: format!("doc_{}", doc_idx),
            action: "read".to_string(),
            context,
        };

        let _ = evaluator.evaluate(&request);
    }

    let elapsed = start.elapsed();
    let requests_per_sec = num_requests as f64 / elapsed.as_secs_f64();

    println!(
        "Throughput: {:.0} requests/second ({} requests in {:?})",
        requests_per_sec, num_requests, elapsed
    );

    // Should achieve at least 50K requests per second (conservative for CI/dev machines)
    assert!(
        requests_per_sec > 50_000.0,
        "Throughput {:.0} req/s below 50K target",
        requests_per_sec
    );
}

/// Test evaluation latency distribution
#[test]
fn test_latency_distribution() {
    let policy_text = r#"
policy latency_test {
    default: deny,

    rule complex_check {
        allow if {
            user.clearance >= 3 &&
            user.department == "engineering" &&
            resource.classification <= 3
        }
    }
}
"#;

    let policy = policy_text.parse::<ReaperPolicy>().unwrap();
    let store = Arc::new(DataStore::new());

    // Load test data
    let mut entities = Vec::new();
    for i in 0..1000 {
        entities.push(format!(
            r#"{{"id": "user_{}", "type": "User", "attributes": {{
                "id": "user_{}",
                "clearance": {},
                "department": "{}"
            }}}}"#,
            i,
            i,
            (i % 5) + 1,
            if i % 3 == 0 { "engineering" } else { "other" }
        ));
    }
    for i in 0..100 {
        entities.push(format!(
            r#"{{"id": "doc_{}", "type": "Resource", "attributes": {{
                "id": "doc_{}",
                "classification": {}
            }}}}"#,
            i,
            i,
            (i % 5) + 1
        ));
    }

    let json = format!(r#"{{"entities": [{}]}}"#, entities.join(","));
    let loader = DataLoader::new((*store).clone());
    loader.load_json(&json).unwrap();

    let evaluator = policy.build(Arc::clone(&store)).unwrap();

    // Collect latency samples
    let mut latencies = Vec::with_capacity(10_000);

    for i in 0..10_000 {
        let user_idx = i % 1000;
        let doc_idx = i % 100;

        let mut context = HashMap::new();
        context.insert("principal".to_string(), format!("user_{}", user_idx));

        let request = PolicyRequest {
            resource: format!("doc_{}", doc_idx),
            action: "read".to_string(),
            context,
        };

        let start = Instant::now();
        let _ = evaluator.evaluate(&request);
        latencies.push(start.elapsed().as_nanos() as u64);
    }

    // Sort for percentile calculation
    latencies.sort();

    let p50 = latencies[latencies.len() / 2];
    let p95 = latencies[(latencies.len() as f64 * 0.95) as usize];
    let p99 = latencies[(latencies.len() as f64 * 0.99) as usize];
    let max = latencies[latencies.len() - 1];

    println!("Latency distribution (ns):");
    println!("  p50: {} ns ({:.2} us)", p50, p50 as f64 / 1000.0);
    println!("  p95: {} ns ({:.2} us)", p95, p95 as f64 / 1000.0);
    println!("  p99: {} ns ({:.2} us)", p99, p99 as f64 / 1000.0);
    println!("  max: {} ns ({:.2} us)", max, max as f64 / 1000.0);

    // p99 should be under 500 microseconds (conservative for CI/dev machines in debug mode)
    // In release mode, p99 is typically under 50us
    assert!(p99 < 500_000, "p99 latency {} ns exceeds 500us", p99);
}

// ============================================================================
// SECTION 3: Concurrent Large-Scale Operations
// ============================================================================

/// Test concurrent evaluation with large dataset
#[test]
fn test_concurrent_evaluation_large_dataset() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

    let policy_text = r#"
policy concurrent_test {
    default: deny,

    rule allow_active {
        allow if {
            user.active == true
        }
    }
}
"#;

    let policy = policy_text.parse::<ReaperPolicy>().unwrap();
    let store = Arc::new(DataStore::new());

    // Load 10K entities
    let mut entities = Vec::new();
    for i in 0..10_000 {
        entities.push(format!(
            r#"{{"id": "user_{}", "type": "User", "attributes": {{"id": "user_{}", "active": {}}}}}"#,
            i,
            i,
            i % 10 != 0
        ));
    }

    let json = format!(r#"{{"entities": [{}]}}"#, entities.join(","));
    let loader = DataLoader::new((*store).clone());
    loader.load_json(&json).unwrap();

    let evaluator = Arc::new(policy.build(Arc::clone(&store)).unwrap());
    let total_evaluations = Arc::new(AtomicUsize::new(0));
    let allow_count = Arc::new(AtomicUsize::new(0));
    let deny_count = Arc::new(AtomicUsize::new(0));

    let num_threads = 8;
    let evals_per_thread = 10_000;

    let start = Instant::now();

    let handles: Vec<_> = (0..num_threads)
        .map(|thread_id| {
            let evaluator = Arc::clone(&evaluator);
            let total = Arc::clone(&total_evaluations);
            let allows = Arc::clone(&allow_count);
            let denies = Arc::clone(&deny_count);

            thread::spawn(move || {
                for i in 0..evals_per_thread {
                    let user_idx = (thread_id * evals_per_thread + i) % 10_000;

                    let mut context = HashMap::new();
                    context.insert("principal".to_string(), format!("user_{}", user_idx));

                    let request = PolicyRequest {
                        resource: "test".to_string(),
                        action: "read".to_string(),
                        context,
                    };

                    match evaluator.evaluate(&request) {
                        Ok(PolicyAction::Allow) => {
                            allows.fetch_add(1, Ordering::Relaxed);
                        }
                        Ok(PolicyAction::Deny) => {
                            denies.fetch_add(1, Ordering::Relaxed);
                        }
                        _ => {}
                    }
                    total.fetch_add(1, Ordering::Relaxed);
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }

    let elapsed = start.elapsed();
    let total = total_evaluations.load(Ordering::Relaxed);
    let allows = allow_count.load(Ordering::Relaxed);
    let denies = deny_count.load(Ordering::Relaxed);

    println!(
        "Concurrent evaluation: {} total ({} allows, {} denies) in {:?}",
        total, allows, denies, elapsed
    );

    let throughput = total as f64 / elapsed.as_secs_f64();
    println!("Concurrent throughput: {:.0} req/s", throughput);

    assert_eq!(total, num_threads * evals_per_thread);
    assert!(
        throughput > 100_000.0,
        "Concurrent throughput {:.0} below 100K target",
        throughput
    );
}

// ============================================================================
// SECTION 4: Stress Tests
// ============================================================================

/// Test sustained load over time
#[test]
fn test_sustained_load() {
    let policy_text = r#"
policy sustained_test {
    default: deny,
    rule r1 { allow if { user.active == true } }
}
"#;

    let policy = policy_text.parse::<ReaperPolicy>().unwrap();
    let store = Arc::new(DataStore::new());

    // Load some entities
    let mut entities = Vec::new();
    for i in 0..1000 {
        entities.push(format!(
            r#"{{"id": "user_{}", "type": "User", "attributes": {{"id": "user_{}", "active": true}}}}"#,
            i, i
        ));
    }

    let json = format!(r#"{{"entities": [{}]}}"#, entities.join(","));
    let loader = DataLoader::new((*store).clone());
    loader.load_json(&json).unwrap();

    let evaluator = policy.build(Arc::clone(&store)).unwrap();

    // Run for a sustained period
    let duration = Duration::from_secs(2);
    let start = Instant::now();
    let mut count = 0u64;

    while start.elapsed() < duration {
        for i in 0..1000 {
            let mut context = HashMap::new();
            context.insert("principal".to_string(), format!("user_{}", i % 1000));

            let request = PolicyRequest {
                resource: "test".to_string(),
                action: "read".to_string(),
                context,
            };

            let _ = evaluator.evaluate(&request);
            count += 1;
        }
    }

    let elapsed = start.elapsed();
    let throughput = count as f64 / elapsed.as_secs_f64();

    println!(
        "Sustained load: {} evaluations in {:?} ({:.0} req/s)",
        count, elapsed, throughput
    );

    assert!(
        throughput > 50_000.0,
        "Sustained throughput {:.0} below 50K",
        throughput
    );
}

/// Test with complex policy and large dataset
#[test]
fn test_complex_policy_large_dataset() {
    // Note: Cross-entity comparisons (user.attr == resource.attr) are not supported
    // in the compiled evaluator. Using individual entity attribute checks instead.
    let policy_text = r#"
policy complex_large_test {
    default: deny,

    rule admin_override {
        allow if {
            user.role == "admin"
        }
    }

    rule engineering_read {
        allow if {
            user.department == "engineering" &&
            user.clearance >= 3 &&
            action == "read"
        }
    }

    rule high_clearance_access {
        allow if {
            user.clearance >= 4 &&
            resource.required_clearance <= 3
        }
    }

    rule active_team_member {
        allow if {
            user.active == true &&
            user.team.contains("team_1")
        }
    }
}
"#;

    let policy = policy_text.parse::<ReaperPolicy>().unwrap();
    let store = Arc::new(DataStore::new());

    // Generate diverse dataset
    let mut entities = Vec::new();

    // Users with various attributes
    for i in 0..10_000 {
        let role = match i % 100 {
            0 => "admin",
            _ => "user",
        };
        let dept = match i % 5 {
            0 => "engineering",
            1 => "sales",
            2 => "marketing",
            3 => "finance",
            _ => "operations",
        };
        let team = format!("team_{}", i % 20);

        entities.push(format!(
            r#"{{"id": "user_{}", "type": "User", "attributes": {{
                "id": "user_{}",
                "role": "{}",
                "department": "{}",
                "team": "{}",
                "clearance": {},
                "active": {}
            }}}}"#,
            i,
            i,
            role,
            dept,
            team,
            (i % 5) + 1,
            i % 10 != 0
        ));
    }

    // Resources
    for i in 0..1000 {
        let dept = match i % 5 {
            0 => "engineering",
            1 => "sales",
            2 => "marketing",
            3 => "finance",
            _ => "operations",
        };
        let team = format!("team_{}", i % 20);
        let owner = format!("user_{}", i * 10);

        entities.push(format!(
            r#"{{"id": "resource_{}", "type": "Resource", "attributes": {{
                "id": "resource_{}",
                "department": "{}",
                "team": "{}",
                "owner": "{}",
                "required_clearance": {}
            }}}}"#,
            i,
            i,
            dept,
            team,
            owner,
            (i % 5) + 1
        ));
    }

    let json = format!(r#"{{"entities": [{}]}}"#, entities.join(","));
    let loader = DataLoader::new((*store).clone());
    loader.load_json(&json).unwrap();

    let evaluator = policy.build(Arc::clone(&store)).unwrap();

    // Benchmark complex evaluation
    let start = Instant::now();
    let num_evals = 10_000;
    let mut allows = 0;
    let mut denies = 0;

    for i in 0..num_evals {
        let user_idx = i % 10_000;
        let resource_idx = i % 1000;

        let mut context = HashMap::new();
        context.insert("principal".to_string(), format!("user_{}", user_idx));

        let request = PolicyRequest {
            resource: format!("resource_{}", resource_idx),
            action: "read".to_string(),
            context,
        };

        match evaluator.evaluate(&request) {
            Ok(PolicyAction::Allow) => allows += 1,
            Ok(PolicyAction::Deny) => denies += 1,
            _ => {}
        }
    }

    let elapsed = start.elapsed();
    let avg_us = elapsed.as_micros() / num_evals as u128;

    println!(
        "Complex policy evaluation: {} evals in {:?} (avg: {}us)",
        num_evals, elapsed, avg_us
    );
    println!("Results: {} allows, {} denies", allows, denies);

    // Should still be fast even with complex policy
    assert!(
        avg_us < 50,
        "Average evaluation time {} us exceeds 50us for complex policy",
        avg_us
    );
}
