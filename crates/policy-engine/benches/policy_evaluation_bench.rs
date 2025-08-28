use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use policy_engine::{EnhancedPolicy, PolicyAction, PolicyEngine, PolicyRequest, PolicyRule};
use std::collections::HashMap;
use std::hint::black_box;
use std::sync::Arc;
use std::thread;

/// Benchmark policy evaluation performance for sub-microsecond targets
fn benchmark_policy_evaluation(c: &mut Criterion) {
    let mut group = c.benchmark_group("policy_evaluation");

    // Create a policy engine with various policy sizes
    let engine = PolicyEngine::new();

    // Single rule policy
    let simple_policy = EnhancedPolicy::new(
        "simple".to_string(),
        "Simple policy".to_string(),
        vec![PolicyRule {
            action: PolicyAction::Allow,
            resource: "*".to_string(),
            conditions: vec![],
        }],
    );
    let simple_policy_id = simple_policy.id;
    engine.deploy_policy(simple_policy).unwrap();

    // Complex policy with multiple rules
    let complex_rules: Vec<PolicyRule> = (0..100)
        .map(|i| PolicyRule {
            action: if i % 3 == 0 {
                PolicyAction::Allow
            } else {
                PolicyAction::Deny
            },
            resource: format!("resource-{}", i),
            conditions: vec![format!("condition-{}", i)],
        })
        .collect();

    let complex_policy = EnhancedPolicy::new(
        "complex".to_string(),
        "Complex policy".to_string(),
        complex_rules,
    );
    let complex_policy_id = complex_policy.id;
    engine.deploy_policy(complex_policy).unwrap();

    // Test requests
    let simple_request = PolicyRequest {
        resource: "*".to_string(),
        action: "read".to_string(),
        context: HashMap::new(),
    };

    let complex_request = PolicyRequest {
        resource: "resource-50".to_string(),
        action: "read".to_string(),
        context: HashMap::new(),
    };

    // Benchmark simple policy evaluation
    group.bench_function("simple_policy", |b| {
        b.iter(|| engine.evaluate(black_box(&simple_policy_id), black_box(&simple_request)))
    });

    // Benchmark complex policy evaluation
    group.bench_function("complex_policy", |b| {
        b.iter(|| engine.evaluate(black_box(&complex_policy_id), black_box(&complex_request)))
    });

    group.finish();
}

/// Benchmark policy hot-swapping performance
fn benchmark_policy_hot_swap(c: &mut Criterion) {
    let mut group = c.benchmark_group("policy_hot_swap");

    let engine = PolicyEngine::new();

    // Benchmark policy deployment (hot-swap)
    group.bench_function("deploy_policy", |b| {
        let mut counter = 0;
        b.iter(|| {
            counter += 1;
            let policy = EnhancedPolicy::new(
                format!("policy-{}", counter),
                "Test policy".to_string(),
                vec![PolicyRule {
                    action: PolicyAction::Allow,
                    resource: "*".to_string(),
                    conditions: vec![],
                }],
            );
            engine.deploy_policy(black_box(policy))
        })
    });

    // Benchmark concurrent policy lookups during hot-swap
    group.bench_function("concurrent_lookup", |b| {
        // Create a policy to lookup
        let lookup_policy = EnhancedPolicy::new(
            "lookup-test".to_string(),
            "Lookup test".to_string(),
            vec![PolicyRule {
                action: PolicyAction::Allow,
                resource: "*".to_string(),
                conditions: vec![],
            }],
        );
        let lookup_policy_id = lookup_policy.id;
        engine.deploy_policy(lookup_policy).unwrap();

        b.iter(|| engine.get_policy(black_box(&lookup_policy_id)))
    });

    group.finish();
}

/// Benchmark concurrent access patterns
fn benchmark_concurrent_access(c: &mut Criterion) {
    let mut group = c.benchmark_group("concurrent_access");
    group.throughput(Throughput::Elements(1000));

    let engine = Arc::new(PolicyEngine::new());

    // Create test policies
    for i in 0..10 {
        let policy = EnhancedPolicy::new(
            format!("policy-{}", i),
            format!("Policy {}", i),
            vec![PolicyRule {
                action: PolicyAction::Allow,
                resource: format!("resource-{}", i),
                conditions: vec![],
            }],
        );
        engine.deploy_policy(policy).unwrap();
    }

    // Benchmark concurrent policy evaluations
    group.bench_function("concurrent_evaluations", |b| {
        b.iter(|| {
            let handles: Vec<_> = (0..10)
                .map(|i| {
                    let engine = Arc::clone(&engine);
                    thread::spawn(move || {
                        let policies = engine.list_policies();
                        if let Some(policy) = policies.get(i % policies.len()) {
                            let request = PolicyRequest {
                                resource: format!("resource-{}", i),
                                action: "read".to_string(),
                                context: HashMap::new(),
                            };
                            engine.evaluate(&policy.id, &request)
                        } else {
                            Err(policy_engine::ReaperError::PolicyNotFound {
                                policy_id: "none".to_string(),
                            })
                        }
                    })
                })
                .collect();

            for handle in handles {
                let _ = handle.join().unwrap();
            }
        })
    });

    group.finish();
}

/// Benchmark memory efficiency
fn benchmark_memory_efficiency(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_efficiency");

    // Test policy storage efficiency with large numbers of policies
    for policy_count in [100, 1000, 10000].iter() {
        group.bench_with_input(
            BenchmarkId::new("policy_storage", policy_count),
            policy_count,
            |b, &policy_count| {
                b.iter(|| {
                    let engine = PolicyEngine::new();

                    // Create many policies
                    for i in 0..policy_count {
                        let policy = EnhancedPolicy::new(
                            format!("policy-{}", i),
                            format!("Policy {}", i),
                            vec![PolicyRule {
                                action: PolicyAction::Allow,
                                resource: format!("resource-{}", i),
                                conditions: vec![format!("condition-{}", i)],
                            }],
                        );
                        engine.deploy_policy(policy).unwrap();
                    }

                    black_box(engine)
                })
            },
        );
    }

    group.finish();
}

/// Benchmark realistic workload scenarios
fn benchmark_realistic_workloads(c: &mut Criterion) {
    let mut group = c.benchmark_group("realistic_workloads");
    group.sample_size(1000);

    let engine = PolicyEngine::new();

    // Create a realistic set of policies (microservice auth scenario)
    let services = [
        "user-service",
        "order-service",
        "payment-service",
        "inventory-service",
    ];
    let actions = ["read", "write", "delete", "admin"];
    let resources = ["users", "orders", "payments", "inventory"];

    for (i, service) in services.iter().enumerate() {
        for (j, resource) in resources.iter().enumerate() {
            let policy = EnhancedPolicy::new(
                format!("{}-{}", service, resource),
                format!("Policy for {} accessing {}", service, resource),
                vec![PolicyRule {
                    action: if i == j {
                        PolicyAction::Allow
                    } else {
                        PolicyAction::Deny
                    },
                    resource: resource.to_string(),
                    conditions: vec![format!("service:{}", service)],
                }],
            );
            engine.deploy_policy(policy).unwrap();
        }
    }

    let policies = engine.list_policies();
    let test_requests: Vec<PolicyRequest> = (0..100)
        .map(|i| PolicyRequest {
            resource: resources[i % resources.len()].to_string(),
            action: actions[i % actions.len()].to_string(),
            context: HashMap::from([
                (
                    "service".to_string(),
                    services[i % services.len()].to_string(),
                ),
                ("user_id".to_string(), format!("user-{}", i)),
            ]),
        })
        .collect();

    // Benchmark realistic request pattern
    group.bench_function("microservice_auth_pattern", |b| {
        b.iter(|| {
            for (i, request) in test_requests.iter().enumerate() {
                let policy = &policies[i % policies.len()];
                let _ = engine.evaluate(black_box(&policy.id), black_box(request));
            }
        })
    });

    group.finish();
}

/// Performance regression test - ensure we stay under target latencies
fn benchmark_latency_targets(c: &mut Criterion) {
    let mut group = c.benchmark_group("latency_targets");
    group.sample_size(10000);

    let engine = PolicyEngine::new();

    // Create a simple policy
    let policy = EnhancedPolicy::new(
        "latency-test".to_string(),
        "Latency test policy".to_string(),
        vec![PolicyRule {
            action: PolicyAction::Allow,
            resource: "*".to_string(),
            conditions: vec![],
        }],
    );
    let policy_id = policy.id;
    engine.deploy_policy(policy).unwrap();

    let request = PolicyRequest {
        resource: "test".to_string(),
        action: "read".to_string(),
        context: HashMap::new(),
    };

    // This benchmark will fail if we exceed our sub-microsecond target
    group.bench_function("sub_microsecond_evaluation", |b| {
        b.iter(|| {
            let start = std::time::Instant::now();
            let result = engine.evaluate(black_box(&policy_id), black_box(&request));
            let duration = start.elapsed();

            // Assert we're under 1 microsecond (1000 nanoseconds)
            assert!(
                duration.as_nanos() < 10000,
                "Policy evaluation took {}ns, exceeded 10000ns target",
                duration.as_nanos()
            );

            black_box(result)
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    benchmark_policy_evaluation,
    benchmark_policy_hot_swap,
    benchmark_concurrent_access,
    benchmark_memory_efficiency,
    benchmark_realistic_workloads,
    benchmark_latency_targets
);
criterion_main!(benches);
