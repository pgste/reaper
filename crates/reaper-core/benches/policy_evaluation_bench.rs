//! Policy Engine benchmarks

use criterion::{criterion_group, criterion_main, Criterion};

fn benchmark_policy_engine(c: &mut Criterion) {
    c.bench_function("policy_engine_evaluation", |b| b.iter(|| 1 + 1));
}

criterion_group!(benches, benchmark_policy_engine);
criterion_main!(benches);
