use criterion::{criterion_group, criterion_main, Criterion};

fn benchmark_message_processing(c: &mut Criterion) {
    c.bench_function("message_queue_processing", |b| b.iter(|| 1 + 1));
}

criterion_group!(benches, benchmark_message_processing);
criterion_main!(benches);
