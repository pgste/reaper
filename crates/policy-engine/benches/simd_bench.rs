//! SIMD Aggregate Performance Benchmarks
//!
//! Phase 7.2: Performance Profiling & Benchmarking
//!
//! Measures SIMD performance for aggregate operations:
//! - sum(), max(), min() with various array sizes
//! - SIMD threshold effectiveness (64+ elements)
//! - Integer vs Float performance
//! - Mixed type arrays

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

/// Benchmark sum() across different array sizes
/// SIMD optimization should kick in at 64+ elements
fn simd_sum(c: &mut Criterion) {
    let mut group = c.benchmark_group("simd_sum");

    // Test array sizes: below SIMD threshold, at threshold, and well above
    for size in [16, 32, 64, 128, 256, 512, 1024].iter() {
        let nums: Vec<i64> = (1..=*size).collect();
        let data = format!(
            r#"{{"entities":[{{"id":"u","type":"U","attributes":{{"nums":{}}}}},{{"id":"res","type":"R","attributes":{{}}}}]}}"#,
            serde_json::to_string(&nums).unwrap()
        );

        let ev = create_evaluator(
            r#"policy p { default: deny, rule r { allow if user.nums.sum() > 0 } }"#,
            &data,
        );

        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, _| {
            b.iter(|| eval(black_box(&ev), "u"))
        });
    }

    group.finish();
}

/// Benchmark max() across different array sizes
fn simd_max(c: &mut Criterion) {
    let mut group = c.benchmark_group("simd_max");

    for size in [16, 32, 64, 128, 256, 512, 1024].iter() {
        let nums: Vec<i64> = (1..=*size).collect();
        let data = format!(
            r#"{{"entities":[{{"id":"u","type":"U","attributes":{{"nums":{}}}}},{{"id":"res","type":"R","attributes":{{}}}}]}}"#,
            serde_json::to_string(&nums).unwrap()
        );

        let ev = create_evaluator(
            r#"policy p { default: deny, rule r { allow if user.nums.max() > 0 } }"#,
            &data,
        );

        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, _| {
            b.iter(|| eval(black_box(&ev), "u"))
        });
    }

    group.finish();
}

/// Benchmark min() across different array sizes
fn simd_min(c: &mut Criterion) {
    let mut group = c.benchmark_group("simd_min");

    for size in [16, 32, 64, 128, 256, 512, 1024].iter() {
        let nums: Vec<i64> = (1..=*size).collect();
        let data = format!(
            r#"{{"entities":[{{"id":"u","type":"U","attributes":{{"nums":{}}}}},{{"id":"res","type":"R","attributes":{{}}}}]}}"#,
            serde_json::to_string(&nums).unwrap()
        );

        let ev = create_evaluator(
            r#"policy p { default: deny, rule r { allow if user.nums.min() > 0 } }"#,
            &data,
        );

        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, _| {
            b.iter(|| eval(black_box(&ev), "u"))
        });
    }

    group.finish();
}

/// Compare integer vs float SIMD performance
fn simd_int_vs_float(c: &mut Criterion) {
    let mut group = c.benchmark_group("simd_int_vs_float");

    let size = 256; // Large enough for SIMD

    // Integer array
    let ints: Vec<i64> = (1..=size).collect();
    let int_data = format!(
        r#"{{"entities":[{{"id":"u","type":"U","attributes":{{"nums":{}}}}},{{"id":"res","type":"R","attributes":{{}}}}]}}"#,
        serde_json::to_string(&ints).unwrap()
    );

    // Float array
    let floats: Vec<f64> = (1..=size).map(|x| x as f64 * 1.5).collect();
    let float_data = format!(
        r#"{{"entities":[{{"id":"u","type":"U","attributes":{{"nums":{}}}}},{{"id":"res","type":"R","attributes":{{}}}}]}}"#,
        serde_json::to_string(&floats).unwrap()
    );

    // Integer sum
    let ev_int = create_evaluator(
        r#"policy p { default: deny, rule r { allow if user.nums.sum() > 0 } }"#,
        &int_data,
    );
    group.bench_function("int_sum", |b| b.iter(|| eval(black_box(&ev_int), "u")));

    // Float sum
    let ev_float = create_evaluator(
        r#"policy p { default: deny, rule r { allow if user.nums.sum() > 0 } }"#,
        &float_data,
    );
    group.bench_function("float_sum", |b| b.iter(|| eval(black_box(&ev_float), "u")));

    group.finish();
}

/// Benchmark count() - should be constant time
fn simd_count(c: &mut Criterion) {
    let mut group = c.benchmark_group("simd_count");

    // count() should be O(1) regardless of array size
    for size in [16, 64, 256, 1024].iter() {
        let nums: Vec<i64> = (1..=*size).collect();
        let data = format!(
            r#"{{"entities":[{{"id":"u","type":"U","attributes":{{"nums":{}}}}},{{"id":"res","type":"R","attributes":{{}}}}]}}"#,
            serde_json::to_string(&nums).unwrap()
        );

        let ev = create_evaluator(
            r#"policy p { default: deny, rule r { allow if user.nums.count() > 0 } }"#,
            &data,
        );

        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, _| {
            b.iter(|| eval(black_box(&ev), "u"))
        });
    }

    group.finish();
}

/// Benchmark SIMD threshold - measure speedup at 64 elements
fn simd_threshold(c: &mut Criterion) {
    let mut group = c.benchmark_group("simd_threshold");

    // Test sizes around the SIMD threshold (typically 64)
    for size in [48, 56, 64, 72, 80].iter() {
        let nums: Vec<i64> = (1..=*size).collect();
        let data = format!(
            r#"{{"entities":[{{"id":"u","type":"U","attributes":{{"nums":{}}}}},{{"id":"res","type":"R","attributes":{{}}}}]}}"#,
            serde_json::to_string(&nums).unwrap()
        );

        let ev = create_evaluator(
            r#"policy p { default: deny, rule r { allow if user.nums.sum() > 0 } }"#,
            &data,
        );

        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, _| {
            b.iter(|| eval(black_box(&ev), "u"))
        });
    }

    group.finish();
}

/// Benchmark multiple aggregates in one policy
fn simd_multiple_aggregates(c: &mut Criterion) {
    let mut group = c.benchmark_group("simd_multi_agg");

    let size = 256;
    let nums: Vec<i64> = (1..=size).collect();
    let data = format!(
        r#"{{"entities":[{{"id":"u","type":"U","attributes":{{"nums":{}}}}},{{"id":"res","type":"R","attributes":{{}}}}]}}"#,
        serde_json::to_string(&nums).unwrap()
    );

    // Single aggregate
    let ev_single = create_evaluator(
        r#"policy p { default: deny, rule r { allow if user.nums.sum() > 0 } }"#,
        &data,
    );
    group.bench_function("single_sum", |b| {
        b.iter(|| eval(black_box(&ev_single), "u"))
    });

    // Multiple aggregates
    let ev_multi = create_evaluator(
        r#"policy p { default: deny, rule r { allow if { total := user.nums.sum() && maximum := user.nums.max() && minimum := user.nums.min() && total > 0 } } }"#,
        &data,
    );
    group.bench_function("sum_max_min", |b| {
        b.iter(|| eval(black_box(&ev_multi), "u"))
    });

    group.finish();
}

criterion_group!(
    simd,
    simd_sum,
    simd_max,
    simd_min,
    simd_int_vs_float,
    simd_count,
    simd_threshold,
    simd_multiple_aggregates
);

criterion_main!(simd);
