//! End-to-End Policy Evaluation Benchmarks
//!
//! Phase 7.2: Performance Profiling & Benchmarking
//!
//! Measures complete policy evaluation performance:
//! - Simple policies (< 1µs target)
//! - Complex ABAC policies
//! - Multi-rule policies
//! - Comprehensions
//! - Real-world scenarios

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use policy_engine::data::DataStore;
use policy_engine::reap::{ReapAstEvaluator, ReaperPolicy};
use policy_engine::{DataLoader, PolicyAction, PolicyRequest};
use std::collections::HashMap;
use std::hint::black_box;
use std::sync::Arc;

fn create_evaluator(policy: &str, data: &str) -> ReapAstEvaluator {
    let store = DataStore::new();
    let loader = DataLoader::new(store.clone());
    loader.load_json(data).expect("Failed to load data");
    let policy: ReaperPolicy = policy.parse().expect("Failed to parse policy");
    policy.build_ast_evaluator(Arc::new(store))
}

fn eval(evaluator: &ReapAstEvaluator, principal: &str) -> PolicyAction {
    let mut ctx = HashMap::new();
    ctx.insert("principal".to_string(), principal.to_string());
    evaluator
        .evaluate(&PolicyRequest {
            resource: "res".to_string(),
            action: "act".to_string(),
            context: ctx,
        })
        .expect("Eval failed")
}

/// Simple allow-all policy - baseline performance
fn simple_policy(c: &mut Criterion) {
    let data = r#"{"entities":[{"id":"u","type":"U","attributes":{}},{"id":"res","type":"R","attributes":{}}]}"#;

    let mut group = c.benchmark_group("simple_policy");

    // Allow all
    let ev = create_evaluator(r#"policy p { default: allow }"#, data);
    group.bench_function("allow_all", |b| b.iter(|| eval(black_box(&ev), "u")));

    // Deny all
    let ev = create_evaluator(r#"policy p { default: deny }"#, data);
    group.bench_function("deny_all", |b| b.iter(|| eval(black_box(&ev), "u")));

    // Single attribute check
    let data_with_attr = r#"{"entities":[{"id":"u","type":"U","attributes":{"role":"admin"}},{"id":"res","type":"R","attributes":{}}]}"#;
    let ev = create_evaluator(
        r#"policy p { default: deny, rule r { allow if user.role == "admin" } }"#,
        data_with_attr,
    );
    group.bench_function("single_attr", |b| b.iter(|| eval(black_box(&ev), "u")));

    group.finish();
}

/// ABAC policies with multiple conditions
fn abac_policy(c: &mut Criterion) {
    let data = r#"{"entities":[{"id":"u","type":"U","attributes":{"role":"manager","dept":"eng","clearance":5,"active":true}},{"id":"res","type":"R","attributes":{"dept":"eng","level":3}}]}"#;

    let mut group = c.benchmark_group("abac_policy");

    // Single condition
    let ev = create_evaluator(
        r#"policy p { default: deny, rule r { allow if user.role == "manager" } }"#,
        data,
    );
    group.bench_function("one_condition", |b| b.iter(|| eval(black_box(&ev), "u")));

    // Multiple AND conditions
    let ev = create_evaluator(
        r#"policy p { default: deny, rule r { allow if user.role == "manager" && user.dept == resource.dept } }"#,
        data,
    );
    group.bench_function("two_conditions", |b| b.iter(|| eval(black_box(&ev), "u")));

    // Complex ABAC
    let ev = create_evaluator(
        r#"policy p { default: deny, rule r { allow if user.role == "manager" && user.dept == resource.dept && user.clearance >= resource.level && user.active == true } }"#,
        data,
    );
    group.bench_function("complex_abac", |b| b.iter(|| eval(black_box(&ev), "u")));

    group.finish();
}

/// Multi-rule policies
fn multi_rule_policy(c: &mut Criterion) {
    let data = r#"{"entities":[{"id":"u","type":"U","attributes":{"role":"user","dept":"eng"}},{"id":"res","type":"R","attributes":{"dept":"eng","owner":"u"}}]}"#;

    let mut group = c.benchmark_group("multi_rule");

    for num_rules in [1, 5, 10, 20].iter() {
        let mut rules = Vec::new();
        for i in 0..*num_rules {
            rules.push(format!(
                "rule r{} {{ allow if user.dept == \"dept{}\" }}",
                i, i
            ));
        }
        // Add matching rule at the end
        rules.push(
            "rule final { allow if user.dept == resource.dept }".to_string()
        );

        let policy = format!("policy p {{ default: deny, {} }}", rules.join(", "));

        let ev = create_evaluator(&policy, data);
        group.bench_with_input(BenchmarkId::new("rules", num_rules), num_rules, |b, _| {
            b.iter(|| eval(black_box(&ev), "u"))
        });
    }

    group.finish();
}

/// Comprehension performance
fn comprehension_policy(c: &mut Criterion) {
    let mut group = c.benchmark_group("comprehension");

    // Small array comprehension
    let small_data = r#"{"entities":[{"id":"u","type":"U","attributes":{"perms":["read","write","delete"]}},{"id":"res","type":"R","attributes":{}}]}"#;
    let ev = create_evaluator(
        r#"policy p { default: deny, rule r { allow if { has_write := [p | p := user.perms[_] && p == "write"] && has_write.count() > 0 } } }"#,
        small_data,
    );
    group.bench_function("small_array", |b| b.iter(|| eval(black_box(&ev), "u")));

    // Medium array
    let medium_data = format!(
        r#"{{"entities":[{{"id":"u","type":"U","attributes":{{"nums":{}}}}},{{"id":"res","type":"R","attributes":{{}}}}]}}"#,
        serde_json::to_string(&(1..=50).collect::<Vec<i64>>()).unwrap()
    );
    let ev = create_evaluator(
        r#"policy p { default: deny, rule r { allow if { evens := [n | n := user.nums[_] && n % 2 == 0] && evens.count() > 10 } } }"#,
        &medium_data,
    );
    group.bench_function("medium_filter", |b| b.iter(|| eval(black_box(&ev), "u")));

    // Nested comprehension
    let nested_data = r#"{"entities":[{"id":"u","type":"U","attributes":{"matrix":[[1,2,3],[4,5,6],[7,8,9]]}},{"id":"res","type":"R","attributes":{}}]}"#;
    let ev = create_evaluator(
        r#"policy p { default: deny, rule r { allow if { flat := [val | row := user.matrix[_]; val := row[_]] && flat.count() == 9 } } }"#,
        nested_data,
    );
    group.bench_function("nested", |b| b.iter(|| eval(black_box(&ev), "u")));

    group.finish();
}

/// Time-based policies
fn time_policy(c: &mut Criterion) {
    let data = r#"{"entities":[{"id":"u","type":"U","attributes":{"login_time":1234567890000000000}},{"id":"res","type":"R","attributes":{}}]}"#;

    let mut group = c.benchmark_group("time_policy");

    // Simple time check
    let ev = create_evaluator(
        r#"policy p { default: deny, rule r { allow if time::is_before(user.login_time, time::now_ns()) } }"#,
        data,
    );
    group.bench_function("is_before", |b| b.iter(|| eval(black_box(&ev), "u")));

    // Time range check
    let ev = create_evaluator(
        r#"policy p { default: deny, rule r { allow if { now := time::now_ns() && time::is_between(user.login_time, 0, now) } } }"#,
        data,
    );
    group.bench_function("time_range", |b| b.iter(|| eval(black_box(&ev), "u")));

    group.finish();
}

/// String operations
fn string_policy(c: &mut Criterion) {
    let data = r#"{"entities":[{"id":"u","type":"U","attributes":{"email":"admin@example.com","name":"  John Doe  "}},{"id":"res","type":"R","attributes":{}}]}"#;

    let mut group = c.benchmark_group("string_policy");

    // Simple string check
    let ev = create_evaluator(
        r#"policy p { default: deny, rule r { allow if user.email.contains("@example.com") } }"#,
        data,
    );
    group.bench_function("contains", |b| b.iter(|| eval(black_box(&ev), "u")));

    // String transformation + check
    let ev = create_evaluator(
        r#"policy p { default: deny, rule r { allow if { trimmed := user.name.trim() && lower := trimmed.lower() && lower.startswith("john") } } }"#,
        data,
    );
    group.bench_function("transform_chain", |b| b.iter(|| eval(black_box(&ev), "u")));

    // Regex validation
    let ev = create_evaluator(
        r#"policy p { default: deny, rule r { allow if user.email.matches("^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\\.[a-zA-Z]{2,}$") } }"#,
        data,
    );
    group.bench_function("regex", |b| b.iter(|| eval(black_box(&ev), "u")));

    group.finish();
}

/// JSON operations
fn json_policy(c: &mut Criterion) {
    let data = r#"{"entities":[{"id":"u","type":"U","attributes":{"config":"{\"theme\":\"dark\",\"notifications\":true}"}},{"id":"res","type":"R","attributes":{}}]}"#;

    let mut group = c.benchmark_group("json_policy");

    // JSON validation
    let ev = create_evaluator(
        r#"policy p { default: deny, rule r { allow if json::is_valid(user.config) } }"#,
        data,
    );
    group.bench_function("is_valid", |b| b.iter(|| eval(black_box(&ev), "u")));

    // JSON parse
    let ev = create_evaluator(
        r#"policy p { default: deny, rule r { allow if { cfg := json::parse(user.config) && is_object(cfg) } } }"#,
        data,
    );
    group.bench_function("parse", |b| b.iter(|| eval(black_box(&ev), "u")));

    group.finish();
}

/// Real-world scenario: document access control
fn document_access_scenario(c: &mut Criterion) {
    let data = r#"{"entities":[{"id":"u","type":"U","attributes":{"id":"u","role":"manager","dept":"eng","clearance":7}},{"id":"doc","type":"Doc","attributes":{"dept":"eng","classification":"confidential","owner":"u","created":1234567890000000000}}]}"#;

    let mut group = c.benchmark_group("document_access");

    let policy = r#"
        policy doc_access {
            default: deny,

            rule owner_access {
                allow if user.id == resource.owner
            },

            rule same_dept_clearance {
                allow if {
                    user.dept == resource.dept &&
                    user.clearance >= 5 &&
                    resource.classification != "secret"
                }
            },

            rule manager_access {
                allow if {
                    user.role == "manager" &&
                    user.dept == resource.dept
                }
            }
        }
    "#;

    let ev = create_evaluator(policy, data);
    group.bench_function("full_scenario", |b| b.iter(|| eval(black_box(&ev), "u")));

    group.finish();
}

criterion_group!(
    e2e,
    simple_policy,
    abac_policy,
    multi_rule_policy,
    comprehension_policy,
    time_policy,
    string_policy,
    json_policy,
    document_access_scenario
);

criterion_main!(e2e);
