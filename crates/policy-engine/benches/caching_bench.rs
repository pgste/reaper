//! Regex Caching Performance Benchmarks
//!
//! Phase 7.2: Performance Profiling & Benchmarking
//!
//! Measures the performance impact of regex caching:
//! - Cache hit vs cache miss performance
//! - Multiple regex patterns
//! - Cache effectiveness with different workloads

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

/// Benchmark regex cache hits - same pattern used repeatedly
fn regex_cache_hit(c: &mut Criterion) {
    let data = r#"{"entities":[{"id":"u","type":"U","attributes":{"email":"user@example.com"}},{"id":"res","type":"R","attributes":{}}]}"#;

    let mut group = c.benchmark_group("regex_cache_hit");

    // Email validation pattern - will be cached after first use
    let ev = create_evaluator(
        r#"policy p { default: deny, rule r { allow if user.email.matches("^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\\.[a-zA-Z]{2,}$") } }"#,
        data,
    );

    // First call will compile and cache the regex
    // Subsequent calls will hit the cache
    group.bench_function("email_pattern", |b| b.iter(|| eval(black_box(&ev), "u")));

    group.finish();
}

/// Benchmark regex cache miss - different patterns
fn regex_cache_miss(c: &mut Criterion) {
    let data = r#"{"entities":[{"id":"u1","type":"U","attributes":{"text":"abc123"}},{"id":"u2","type":"U","attributes":{"text":"xyz789"}},{"id":"u3","type":"U","attributes":{"text":"def456"}},{"id":"res","type":"R","attributes":{}}]}"#;

    let mut group = c.benchmark_group("regex_cache_miss");

    // Three different patterns - each will have its own cache entry
    let ev1 = create_evaluator(
        r#"policy p { default: deny, rule r { allow if user.text.matches("^abc") } }"#,
        data,
    );
    let ev2 = create_evaluator(
        r#"policy p { default: deny, rule r { allow if user.text.matches("^xyz") } }"#,
        data,
    );
    let ev3 = create_evaluator(
        r#"policy p { default: deny, rule r { allow if user.text.matches("^def") } }"#,
        data,
    );

    group.bench_function("pattern_1", |b| b.iter(|| eval(black_box(&ev1), "u1")));
    group.bench_function("pattern_2", |b| b.iter(|| eval(black_box(&ev2), "u2")));
    group.bench_function("pattern_3", |b| b.iter(|| eval(black_box(&ev3), "u3")));

    group.finish();
}

/// Benchmark complex regex patterns
fn regex_complexity(c: &mut Criterion) {
    let data = r#"{"entities":[{"id":"u","type":"U","attributes":{"text":"The quick brown fox jumps over the lazy dog"}},{"id":"res","type":"R","attributes":{}}]}"#;

    let mut group = c.benchmark_group("regex_complexity");

    // Simple pattern
    let ev_simple = create_evaluator(
        r#"policy p { default: deny, rule r { allow if user.text.matches("fox") } }"#,
        data,
    );
    group.bench_function("simple", |b| b.iter(|| eval(black_box(&ev_simple), "u")));

    // Medium complexity
    let ev_medium = create_evaluator(
        r#"policy p { default: deny, rule r { allow if user.text.matches("\\b\\w{5}\\b") } }"#,
        data,
    );
    group.bench_function("medium", |b| b.iter(|| eval(black_box(&ev_medium), "u")));

    // Complex pattern with multiple groups
    let ev_complex = create_evaluator(
        r#"policy p { default: deny, rule r { allow if user.text.matches("^(\\w+)\\s+(\\w+)\\s+(\\w+)") } }"#,
        data,
    );
    group.bench_function("complex", |b| b.iter(|| eval(black_box(&ev_complex), "u")));

    group.finish();
}

/// Benchmark find vs matches operations
fn regex_operations(c: &mut Criterion) {
    let data = r#"{"entities":[{"id":"u","type":"U","attributes":{"log":"[2023-01-01] INFO: User logged in"}},{"id":"res","type":"R","attributes":{}}]}"#;

    let mut group = c.benchmark_group("regex_operations");

    // matches() - returns boolean
    let ev_matches = create_evaluator(
        r#"policy p { default: deny, rule r { allow if user.log.matches("\\[\\d{4}-\\d{2}-\\d{2}\\]") } }"#,
        data,
    );
    group.bench_function("matches", |b| b.iter(|| eval(black_box(&ev_matches), "u")));

    // find() - extracts matched text
    let ev_find = create_evaluator(
        r#"policy p { default: deny, rule r { allow if { date := user.log.find("\\d{4}-\\d{2}-\\d{2}") && date == "2023-01-01" } } }"#,
        data,
    );
    group.bench_function("find", |b| b.iter(|| eval(black_box(&ev_find), "u")));

    group.finish();
}

/// Benchmark repeated regex usage (cache effectiveness)
fn regex_repeated_usage(c: &mut Criterion) {
    let data = r#"{"entities":[{"id":"u","type":"U","attributes":{"url":"https://example.com/path?query=value"}},{"id":"res","type":"R","attributes":{}}]}"#;

    let mut group = c.benchmark_group("regex_repeated");

    // URL validation pattern - commonly reused
    let ev = create_evaluator(
        r#"policy p { default: deny, rule r { allow if user.url.matches("^https?://[a-zA-Z0-9.-]+\\.[a-zA-Z]{2,}") } }"#,
        data,
    );

    // Measure performance over many iterations (cache should stay hot)
    for iterations in [10, 100, 1000].iter() {
        group.bench_with_input(
            BenchmarkId::new("iterations", iterations),
            iterations,
            |b, &iters| {
                b.iter(|| {
                    for _ in 0..iters {
                        black_box(eval(&ev, "u"));
                    }
                })
            },
        );
    }

    group.finish();
}

criterion_group!(
    caching,
    regex_cache_hit,
    regex_cache_miss,
    regex_complexity,
    regex_operations,
    regex_repeated_usage
);

criterion_main!(caching);
