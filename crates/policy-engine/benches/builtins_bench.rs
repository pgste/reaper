//! Benchmark suite for built-in functions and methods
//!
//! Phase 7.2: Performance Profiling & Benchmarking
//!
//! Establishes baseline performance metrics for key built-in functions:
//! - Type checking (is_string, is_number, is_bool, is_array, is_null)
//! - String operations (lower, upper, trim, contains, matches)
//! - Collection operations (count, sum, max, min)
//! - Time functions (now_ns, parse_rfc3339, add_ns, is_before)
//! - JSON functions (parse, is_valid)

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use policy_engine::data::DataStore;
use policy_engine::reap::{ReapAstEvaluator, ReaperPolicy};
use policy_engine::{DataLoader, PolicyAction, PolicyRequest};
use std::collections::HashMap;
use std::hint::black_box;
use std::sync::Arc;

// ============================================================================
// Helper Functions
// ============================================================================

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

// ============================================================================
// Benchmarks
// ============================================================================

fn type_checking(c: &mut Criterion) {
    let data = r#"{"entities":[{"id":"u","type":"U","attributes":{"s":"hi","n":42,"b":true,"a":[1,2,3],"null":null}},{"id":"res","type":"R","attributes":{}}]}"#;

    let mut group = c.benchmark_group("type_check");

    // is_string
    let ev = create_evaluator(
        r#"policy p { default: deny, rule r { allow if is_string(user.s) } }"#,
        data,
    );
    group.bench_function("is_string", |b| b.iter(|| eval(black_box(&ev), "u")));

    // is_number
    let ev = create_evaluator(
        r#"policy p { default: deny, rule r { allow if is_number(user.n) } }"#,
        data,
    );
    group.bench_function("is_number", |b| b.iter(|| eval(black_box(&ev), "u")));

    // is_bool
    let ev = create_evaluator(
        r#"policy p { default: deny, rule r { allow if is_bool(user.b) } }"#,
        data,
    );
    group.bench_function("is_bool", |b| b.iter(|| eval(black_box(&ev), "u")));

    // is_array
    let ev = create_evaluator(
        r#"policy p { default: deny, rule r { allow if is_array(user.a) } }"#,
        data,
    );
    group.bench_function("is_array", |b| b.iter(|| eval(black_box(&ev), "u")));

    // is_null
    let ev = create_evaluator(
        r#"policy p { default: deny, rule r { allow if is_null(user.null) } }"#,
        data,
    );
    group.bench_function("is_null", |b| b.iter(|| eval(black_box(&ev), "u")));

    group.finish();
}

fn string_methods(c: &mut Criterion) {
    let data = r#"{"entities":[{"id":"u","type":"U","attributes":{"text":"Hello World","email":"user@example.com","pad":"  trim  "}},{"id":"res","type":"R","attributes":{}}]}"#;

    let mut group = c.benchmark_group("string_methods");

    // lower
    let ev = create_evaluator(
        r#"policy p { default: deny, rule r { allow if user.text.lower() == "hello world" } }"#,
        data,
    );
    group.bench_function("lower", |b| b.iter(|| eval(black_box(&ev), "u")));

    // upper
    let ev = create_evaluator(
        r#"policy p { default: deny, rule r { allow if user.text.upper() == "HELLO WORLD" } }"#,
        data,
    );
    group.bench_function("upper", |b| b.iter(|| eval(black_box(&ev), "u")));

    // trim
    let ev = create_evaluator(
        r#"policy p { default: deny, rule r { allow if user.pad.trim() == "trim" } }"#,
        data,
    );
    group.bench_function("trim", |b| b.iter(|| eval(black_box(&ev), "u")));

    // contains
    let ev = create_evaluator(
        r#"policy p { default: deny, rule r { allow if user.email.contains("example") } }"#,
        data,
    );
    group.bench_function("contains", |b| b.iter(|| eval(black_box(&ev), "u")));

    // startswith
    let ev = create_evaluator(
        r#"policy p { default: deny, rule r { allow if user.email.startswith("user@") } }"#,
        data,
    );
    group.bench_function("startswith", |b| b.iter(|| eval(black_box(&ev), "u")));

    // endswith
    let ev = create_evaluator(
        r#"policy p { default: deny, rule r { allow if user.email.endswith(".com") } }"#,
        data,
    );
    group.bench_function("endswith", |b| b.iter(|| eval(black_box(&ev), "u")));

    // matches - simple
    let ev = create_evaluator(
        r#"policy p { default: deny, rule r { allow if user.email.matches("^\\w+@") } }"#,
        data,
    );
    group.bench_function("matches", |b| b.iter(|| eval(black_box(&ev), "u")));

    group.finish();
}

fn collection_methods(c: &mut Criterion) {
    let mut group = c.benchmark_group("collection_methods");

    for size in [10, 100, 500].iter() {
        let nums: Vec<i64> = (1..=*size).collect();
        let data = format!(
            r#"{{"entities":[{{"id":"u","type":"U","attributes":{{"nums":{}}}}},{{"id":"res","type":"R","attributes":{{}}}}]}}"#,
            serde_json::to_string(&nums).unwrap()
        );

        // count
        let ev = create_evaluator(
            r#"policy p { default: deny, rule r { allow if user.nums.count() > 0 } }"#,
            &data,
        );
        group.bench_with_input(BenchmarkId::new("count", size), size, |b, _| {
            b.iter(|| eval(black_box(&ev), "u"))
        });

        // sum
        let ev = create_evaluator(
            r#"policy p { default: deny, rule r { allow if user.nums.sum() > 0 } }"#,
            &data,
        );
        group.bench_with_input(BenchmarkId::new("sum", size), size, |b, _| {
            b.iter(|| eval(black_box(&ev), "u"))
        });

        // max
        let ev = create_evaluator(
            r#"policy p { default: deny, rule r { allow if user.nums.max() > 0 } }"#,
            &data,
        );
        group.bench_with_input(BenchmarkId::new("max", size), size, |b, _| {
            b.iter(|| eval(black_box(&ev), "u"))
        });

        // min
        let ev = create_evaluator(
            r#"policy p { default: deny, rule r { allow if user.nums.min() > 0 } }"#,
            &data,
        );
        group.bench_with_input(BenchmarkId::new("min", size), size, |b, _| {
            b.iter(|| eval(black_box(&ev), "u"))
        });
    }

    group.finish();
}

fn time_functions(c: &mut Criterion) {
    let data = r#"{"entities":[{"id":"u","type":"U","attributes":{"ts":1234567890000000000,"rfc":"2023-01-01T12:00:00Z"}},{"id":"res","type":"R","attributes":{}}]}"#;

    let mut group = c.benchmark_group("time_functions");

    // now_ns
    let ev = create_evaluator(
        r#"policy p { default: deny, rule r { allow if { now := time::now_ns() && now > 0 } } }"#,
        data,
    );
    group.bench_function("now_ns", |b| b.iter(|| eval(black_box(&ev), "u")));

    // parse_rfc3339
    let ev = create_evaluator(
        r#"policy p { default: deny, rule r { allow if { parsed := time::parse_rfc3339(user.rfc) && parsed > 0 } } }"#,
        data,
    );
    group.bench_function("parse_rfc3339", |b| b.iter(|| eval(black_box(&ev), "u")));

    // add_ns
    let ev = create_evaluator(
        r#"policy p { default: deny, rule r { allow if { future := time::add_ns(user.ts, 1000000000) && future > user.ts } } }"#,
        data,
    );
    group.bench_function("add_ns", |b| b.iter(|| eval(black_box(&ev), "u")));

    // is_before
    let ev = create_evaluator(
        r#"policy p { default: deny, rule r { allow if time::is_before(user.ts, time::now_ns()) } }"#,
        data,
    );
    group.bench_function("is_before", |b| b.iter(|| eval(black_box(&ev), "u")));

    group.finish();
}

fn json_functions(c: &mut Criterion) {
    let data = r#"{"entities":[{"id":"u","type":"U","attributes":{"json":"{\"k\":\"v\",\"n\":42}"}},{"id":"res","type":"R","attributes":{}}]}"#;

    let mut group = c.benchmark_group("json_functions");

    // is_valid
    let ev = create_evaluator(
        r#"policy p { default: deny, rule r { allow if json::is_valid(user.json) } }"#,
        data,
    );
    group.bench_function("is_valid", |b| b.iter(|| eval(black_box(&ev), "u")));

    // parse
    let ev = create_evaluator(
        r#"policy p { default: deny, rule r { allow if { parsed := json::parse(user.json) && is_object(parsed) } } }"#,
        data,
    );
    group.bench_function("parse", |b| b.iter(|| eval(black_box(&ev), "u")));

    group.finish();
}

criterion_group!(
    builtins,
    type_checking,
    string_methods,
    collection_methods,
    time_functions,
    json_functions
);

criterion_main!(builtins);
