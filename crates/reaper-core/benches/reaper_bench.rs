//! Performance benchmarks for Reaper

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};

fn benchmark_policy_evaluation(c: &mut Criterion) {
    let mut group = c.benchmark_group("reaper_policy_evaluation");

    for size in [1, 10, 100, 1000].iter() {
        group.bench_with_input(BenchmarkId::new("basic_policy", size), size, |b, &size| {
            b.iter(|| black_box(size * 2));
        });
    }

    group.finish();
}

criterion_group!(benches, benchmark_policy_evaluation);
criterion_main!(benches);
