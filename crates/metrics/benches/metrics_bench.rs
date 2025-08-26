use criterion::{criterion_group, criterion_main, Criterion};

fn benchmark_metrics_collection(c: &mut Criterion) {
    c.bench_function("metrics_collection", |b| b.iter(|| 1 + 1));
}

criterion_group!(benches, benchmark_metrics_collection);
criterion_main!(benches);
