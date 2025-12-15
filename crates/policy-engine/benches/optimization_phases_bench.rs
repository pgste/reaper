use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use policy_engine::{
    DecisionMatrix, EnhancedPolicy, IndexedPolicyEngine, PartialEvaluator, PolicyAction,
    PolicyCompiler, PolicyEngine, PolicyRequest, PolicyRule,
};
use std::collections::HashMap;
use std::hint::black_box;

/// Benchmark Phase 1: Policy Indexing
fn benchmark_phase1_indexing(c: &mut Criterion) {
    let mut group = c.benchmark_group("phase1_indexing");

    // Create policies of varying sizes
    for num_policies in [10, 100, 1000].iter() {
        // Baseline: Standard PolicyEngine
        let baseline_engine = PolicyEngine::new();
        for i in 0..*num_policies {
            let policy = EnhancedPolicy::new(
                format!("policy-{}", i),
                format!("Policy {}", i),
                vec![PolicyRule {
                    action: PolicyAction::Allow,
                    resource: format!("/api/resource{}", i),
                    conditions: vec![],
                }],
            );
            baseline_engine.deploy_policy(policy).unwrap();
        }

        // Phase 1: IndexedPolicyEngine
        let indexed_engine = IndexedPolicyEngine::new();
        for i in 0..*num_policies {
            let policy = EnhancedPolicy::new(
                format!("policy-{}", i),
                format!("Policy {}", i),
                vec![PolicyRule {
                    action: PolicyAction::Allow,
                    resource: format!("/api/resource{}", i),
                    conditions: vec![],
                }],
            );
            indexed_engine.deploy_policy(policy).unwrap();
        }

        let request = PolicyRequest {
            resource: "/api/resource50".to_string(),
            action: "read".to_string(),
            context: HashMap::new(),
        };

        // Benchmark baseline
        group.bench_with_input(
            BenchmarkId::new("baseline", num_policies),
            num_policies,
            |b, _| {
                b.iter(|| {
                    // Try all policies
                    for i in 0..*num_policies {
                        let _ = baseline_engine.get_policy_by_name(&format!("policy-{}", i));
                    }
                })
            },
        );

        // Benchmark indexed
        group.bench_with_input(
            BenchmarkId::new("indexed", num_policies),
            num_policies,
            |b, _| b.iter(|| indexed_engine.evaluate(black_box(&request))),
        );
    }

    group.finish();
}

/// Benchmark Phase 2: Decision Matrix Precomputation
fn benchmark_phase2_matrix(c: &mut Criterion) {
    let mut group = c.benchmark_group("phase2_decision_matrix");

    let policy = EnhancedPolicy::new(
        "rbac-policy".to_string(),
        "Simple RBAC".to_string(),
        vec![PolicyRule {
            action: PolicyAction::Allow,
            resource: "/api/users".to_string(),
            conditions: vec!["role==admin".to_string()],
        }],
    );

    // Different matrix sizes
    for (principals, resources, actions) in [
        (10, 10, 5),   // 500 combinations
        (100, 10, 5),  // 5,000 combinations
        (100, 100, 5), // 50,000 combinations
    ]
    .iter()
    {
        let matrix = DecisionMatrix::new();

        let principals_vec: Vec<String> = (0..*principals).map(|i| format!("user{}", i)).collect();
        let resources_vec: Vec<String> = (0..*resources)
            .map(|i| format!("/api/resource{}", i))
            .collect();
        let actions_vec: Vec<String> = (0..*actions).map(|i| format!("action{}", i)).collect();
        let contexts = vec![HashMap::new()];

        // Benchmark precomputation time
        group.bench_with_input(
            BenchmarkId::new("precompute", principals * resources * actions),
            &(principals, resources, actions),
            |b, _| {
                b.iter(|| {
                    let m = DecisionMatrix::new();
                    m.precompute(
                        &policy,
                        principals_vec.clone(),
                        resources_vec.clone(),
                        actions_vec.clone(),
                        contexts.clone(),
                    )
                })
            },
        );

        // Precompute for lookup benchmark
        matrix
            .precompute(
                &policy,
                principals_vec,
                resources_vec,
                actions_vec,
                contexts,
            )
            .unwrap();

        let request = PolicyRequest {
            resource: "/api/resource5".to_string(),
            action: "action2".to_string(),
            context: HashMap::new(),
        };

        // Benchmark lookup time
        group.bench_with_input(
            BenchmarkId::new("lookup", principals * resources * actions),
            &(principals, resources, actions),
            |b, _| b.iter(|| matrix.lookup(black_box(&request), black_box("user5"))),
        );
    }

    group.finish();
}

/// Benchmark Phase 3: Partial Evaluation
fn benchmark_phase3_partial_eval(c: &mut Criterion) {
    let mut group = c.benchmark_group("phase3_partial_evaluation");

    // Original policy with 5 conditions
    let original_policy = EnhancedPolicy::new(
        "complex-policy".to_string(),
        "Complex RBAC".to_string(),
        vec![PolicyRule {
            action: PolicyAction::Allow,
            resource: "/api/*".to_string(),
            conditions: vec![
                "role==admin".to_string(),
                "department==engineering".to_string(),
                "action==read".to_string(),
                "time.hour>=9".to_string(),
                "time.hour<17".to_string(),
            ],
        }],
    );

    let evaluator = PartialEvaluator::new();
    let mut static_context = HashMap::new();
    static_context.insert("role".to_string(), "admin".to_string());
    static_context.insert("department".to_string(), "engineering".to_string());

    // Benchmark partial evaluation time
    group.bench_function("partial_evaluate", |b| {
        b.iter(|| {
            evaluator.partial_evaluate(black_box(&original_policy), black_box(&static_context))
        })
    });

    // Create optimized policy for comparison
    let optimized_policy = evaluator
        .partial_evaluate(&original_policy, &static_context)
        .unwrap();

    // Compare stats
    let stats = evaluator.get_optimization_stats(&original_policy, &optimized_policy);
    println!(
        "\nPartial Evaluation Stats:\n  Original conditions: {}\n  Optimized conditions: {}\n  Speedup: {:.2}x",
        stats.original_conditions, stats.optimized_conditions, stats.estimated_speedup
    );

    group.finish();
}

/// Benchmark Phase 4: Policy Compilation
fn benchmark_phase4_compilation(c: &mut Criterion) {
    let mut group = c.benchmark_group("phase4_compilation");

    let policy = EnhancedPolicy::new(
        "simple-policy".to_string(),
        "Simple RBAC".to_string(),
        vec![
            PolicyRule {
                action: PolicyAction::Allow,
                resource: "/api/users".to_string(),
                conditions: vec!["action==read".to_string()],
            },
            PolicyRule {
                action: PolicyAction::Allow,
                resource: "/api/posts".to_string(),
                conditions: vec!["action==read".to_string()],
            },
        ],
    );

    let compiler = PolicyCompiler::new();

    // Benchmark compilation time
    group.bench_function("compile", |b| {
        b.iter(|| compiler.compile(black_box(&policy)))
    });

    // Show compiled code
    let compiled = compiler.compile(&policy).unwrap();
    println!(
        "\nCompiled Code Sample:\n{}\n",
        &compiled.code[..compiled.code.len().min(300)]
    );
    println!("Compilation Stats:");
    println!("  Rules compiled: {}", compiled.stats.rules_compiled);
    println!(
        "  Conditions compiled: {}",
        compiled.stats.conditions_compiled
    );
    println!("  Generated lines: {}", compiled.stats.generated_lines);
    println!(
        "  Estimated speedup: {:.2}x",
        compiled.stats.estimated_speedup
    );

    group.finish();
}

/// Benchmark all phases combined
fn benchmark_combined_optimizations(c: &mut Criterion) {
    let mut group = c.benchmark_group("combined_optimizations");

    // Create a realistic RBAC scenario
    let num_policies = 100;

    // 1. Create policies
    let policies: Vec<_> = (0..num_policies)
        .map(|i| {
            EnhancedPolicy::new(
                format!("policy-{}", i),
                format!("Policy {}", i),
                vec![PolicyRule {
                    action: PolicyAction::Allow,
                    resource: format!("/api/resource{}", i % 20), // 20 unique resources
                    conditions: vec![],
                }],
            )
        })
        .collect();

    // Baseline: Standard PolicyEngine
    let baseline_engine = PolicyEngine::new();
    for policy in &policies {
        baseline_engine.deploy_policy(policy.clone()).unwrap();
    }

    // Phase 1: Indexed
    let indexed_engine = IndexedPolicyEngine::new();
    for policy in &policies {
        indexed_engine.deploy_policy(policy.clone()).unwrap();
    }

    let request = PolicyRequest {
        resource: "/api/resource10".to_string(),
        action: "read".to_string(),
        context: HashMap::new(),
    };

    // Benchmark baseline
    group.bench_function("baseline", |b| {
        b.iter(|| {
            let policy_id = baseline_engine.get_policy_by_name("policy-10").unwrap().id;
            baseline_engine.evaluate(black_box(&policy_id), black_box(&request))
        })
    });

    // Benchmark indexed
    group.bench_function("phase1_indexed", |b| {
        b.iter(|| indexed_engine.evaluate(black_box(&request)))
    });

    group.finish();
}

criterion_group!(
    benches,
    benchmark_phase1_indexing,
    benchmark_phase2_matrix,
    benchmark_phase3_partial_eval,
    benchmark_phase4_compilation,
    benchmark_combined_optimizations,
);
criterion_main!(benches);
