use criterion::{criterion_group, criterion_main, Criterion};

fn benchmark_agent_request_processing(c: &mut Criterion) {
    c.bench_function("agent_request_processing", |b| b.iter(|| 1 + 1));
}

criterion_group!(benches, benchmark_agent_request_processing);
criterion_main!(benches);
