//! Concurrent decision-capture throughput measurement.
//!
//! Exercises the multi-producer path the sharded capture design targets: N
//! threads hammering `DecisionBuffer::log()` simultaneously, as happens on a
//! multi-core agent at full load. Prints aggregate throughput and the drop/
//! overflow counters (which must be 0 for a healthy run).
//!
//! Run: `cargo run --release -p policy-engine --example decision_capture_bench [threads] [per_thread]`

use policy_engine::decision_log::{DecisionLogConfig, DecisionLogEntry};
use policy_engine::DecisionBuffer;
use std::sync::Arc;
use std::time::Instant;

fn entry(i: usize, t: usize) -> DecisionLogEntry {
    DecisionLogEntry::new(
        format!("user_{t}"),
        "read".to_string(),
        format!("/api/data/{i}"),
        if i % 10 == 0 { "deny" } else { "allow" }.to_string(),
        "policy-1".to_string(),
        "bench-policy".to_string(),
    )
}

fn run(shards: usize, threads: usize, per_thread: usize) {
    let config = DecisionLogConfig {
        enabled: true,
        buffer_capacity: 100_000,
        capture_shards: shards,
        ..Default::default()
    };
    let buffer = Arc::new(DecisionBuffer::new(config).expect("buffer"));

    let start = Instant::now();
    let handles: Vec<_> = (0..threads)
        .map(|t| {
            let b = Arc::clone(&buffer);
            std::thread::spawn(move || {
                for i in 0..per_thread {
                    if b.should_log(i % 10 != 0) {
                        b.log(entry(i, t));
                    }
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    let elapsed = start.elapsed();

    let total = (threads * per_thread) as f64;
    let stats = buffer.stats();
    println!(
        "shards={shards:<3} threads={threads:<3} total={:>10.0}  {:>8.2} M ops/s  {:>7.1} ns/op  dropped(ring evict)={}",
        total,
        total / elapsed.as_secs_f64() / 1e6,
        elapsed.as_nanos() as f64 / total,
        stats.dropped_entries,
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let threads: usize = args.get(1).and_then(|v| v.parse().ok()).unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|p| p.get().min(16))
            .unwrap_or(8)
    });
    let per_thread: usize = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(200_000);

    println!("Concurrent decision-capture throughput ({threads} producer threads, {per_thread} decisions each)");
    println!("---------------------------------------------------------------------------");
    // Single shard = the old single-ring point-of-contention design.
    run(1, threads, per_thread);
    // Auto shards = the new default (one per detected core).
    run(0, threads, per_thread);
}
