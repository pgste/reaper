use futures_util::{SinkExt, StreamExt};
/// HTTP client demonstrating connection pooling and persistent connections
///
/// This client shows best practices for minimizing HTTP overhead:
/// - Connection pooling (reuse TCP connections)
/// - HTTP/2 multiplexing (multiple requests on one connection)
/// - Keep-Alive headers
/// - Batch requests where possible
///
/// Usage:
///   # Start server first
///   cargo run --example http_server --release
///
///   # Run client
///   cargo run --example http_client --release -- --mode single
///   cargo run --example http_client --release -- --mode batch
///   cargo run --example http_client --release -- --mode websocket
///   cargo run --example http_client --release -- --mode benchmark
use reqwest::ClientBuilder;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EvaluateRequest {
    principal: String,
    action: String,
    resource: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BatchEvaluateRequest {
    requests: Vec<EvaluateRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EvaluateResponse {
    decision: String,
    allowed: bool,
    evaluation_time_ns: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BatchEvaluateResponse {
    results: Vec<EvaluateResponse>,
    total_time_ns: u128,
    count: usize,
}

/// Demonstrates single request with connection pooling
async fn test_single_requests(iterations: usize) -> anyhow::Result<()> {
    println!("🔄 Testing single requests with connection pooling...\n");

    // Create client with connection pooling
    // This client will maintain a pool of persistent connections
    let client = ClientBuilder::new()
        .pool_max_idle_per_host(10) // Keep 10 idle connections
        .pool_idle_timeout(Duration::from_secs(90)) // Keep alive for 90s
        .http2_prior_knowledge() // Use HTTP/2
        .tcp_keepalive(Duration::from_secs(60))
        .build()?;

    let url = "http://localhost:3000/v1/evaluate";

    let mut total_time = Duration::ZERO;
    let mut latencies = Vec::new();

    println!("Running {} requests...", iterations);

    for i in 0..iterations {
        let req = EvaluateRequest {
            principal: format!("user_{}", i % 1000),
            action: "read".to_string(),
            resource: format!("doc_{}", i % 1000),
        };

        let start = Instant::now();

        let response = client.post(url).json(&req).send().await?;

        let _result: EvaluateResponse = response.json().await?;

        let elapsed = start.elapsed();
        total_time += elapsed;
        latencies.push(elapsed.as_micros());

        if (i + 1) % 100 == 0 {
            print!("\r   Progress: {}/{}", i + 1, iterations);
        }
    }

    println!();

    // Calculate statistics
    latencies.sort();
    let mean = total_time.as_micros() / iterations as u128;
    let median = latencies[latencies.len() / 2];
    let p95 = latencies[(latencies.len() as f64 * 0.95) as usize];
    let p99 = latencies[(latencies.len() as f64 * 0.99) as usize];
    let min = latencies[0];
    let max = latencies[latencies.len() - 1];

    println!("\n📊 Results (HTTP/2 with connection pooling):");
    println!("   Total time:     {:?}", total_time);
    println!("   Mean latency:   {} µs", mean);
    println!("   Median latency: {} µs", median);
    println!("   P95 latency:    {} µs", p95);
    println!("   P99 latency:    {} µs", p99);
    println!("   Min latency:    {} µs", min);
    println!("   Max latency:    {} µs", max);
    println!(
        "   Throughput:     {:.2} req/sec",
        iterations as f64 / total_time.as_secs_f64()
    );

    Ok(())
}

/// Demonstrates batch requests (amortize connection overhead)
async fn test_batch_requests(total_requests: usize, batch_size: usize) -> anyhow::Result<()> {
    println!(
        "📦 Testing batch requests (batch size: {})...\n",
        batch_size
    );

    let client = ClientBuilder::new()
        .pool_max_idle_per_host(10)
        .http2_prior_knowledge()
        .build()?;

    let url = "http://localhost:3000/v1/evaluate/batch";

    let num_batches = total_requests / batch_size;
    let mut total_time = Duration::ZERO;
    let mut batch_latencies = Vec::new();

    println!(
        "Running {} batches of {} requests...",
        num_batches, batch_size
    );

    for i in 0..num_batches {
        let requests: Vec<EvaluateRequest> = (0..batch_size)
            .map(|j| EvaluateRequest {
                principal: format!("user_{}", (i * batch_size + j) % 1000),
                action: "read".to_string(),
                resource: format!("doc_{}", (i * batch_size + j) % 1000),
            })
            .collect();

        let batch_req = BatchEvaluateRequest { requests };

        let start = Instant::now();

        let response = client.post(url).json(&batch_req).send().await?;

        let _result: BatchEvaluateResponse = response.json().await?;

        let elapsed = start.elapsed();
        total_time += elapsed;
        batch_latencies.push(elapsed.as_micros());

        if (i + 1) % 10 == 0 {
            print!("\r   Progress: {}/{} batches", i + 1, num_batches);
        }
    }

    println!();

    let amortized_per_request = total_time.as_micros() / total_requests as u128;
    let mean_batch = total_time.as_micros() / num_batches as u128;

    println!("\n📊 Results (Batch processing):");
    println!("   Total requests:        {}", total_requests);
    println!("   Batch size:            {}", batch_size);
    println!("   Total time:            {:?}", total_time);
    println!("   Mean batch latency:    {} µs", mean_batch);
    println!("   Amortized per request: {} µs", amortized_per_request);
    println!(
        "   Throughput:            {:.2} req/sec",
        total_requests as f64 / total_time.as_secs_f64()
    );

    Ok(())
}

/// Demonstrates WebSocket streaming (persistent bidirectional connection)
async fn test_websocket(iterations: usize) -> anyhow::Result<()> {
    println!("🔌 Testing WebSocket streaming...\n");

    let url = "ws://localhost:3000/v1/stream";

    println!("Connecting to {}...", url);
    let (ws_stream, _) = connect_async(url).await?;
    println!("✓ Connected\n");

    let (mut write, mut read) = ws_stream.split();

    let mut total_time = Duration::ZERO;
    let mut latencies = Vec::new();

    println!("Streaming {} requests...", iterations);

    for i in 0..iterations {
        let req = EvaluateRequest {
            principal: format!("user_{}", i % 1000),
            action: "read".to_string(),
            resource: format!("doc_{}", i % 1000),
        };

        let start = Instant::now();

        // Send request
        let json = serde_json::to_string(&req)?;
        write.send(Message::Text(json)).await?;

        // Receive response
        if let Some(msg) = read.next().await {
            let msg = msg?;
            if let Message::Text(text) = msg {
                let _response: EvaluateResponse = serde_json::from_str(&text)?;

                let elapsed = start.elapsed();
                total_time += elapsed;
                latencies.push(elapsed.as_micros());
            }
        }

        if (i + 1) % 100 == 0 {
            print!("\r   Progress: {}/{}", i + 1, iterations);
        }
    }

    println!();

    // Calculate statistics
    latencies.sort();
    let mean = total_time.as_micros() / iterations as u128;
    let median = latencies[latencies.len() / 2];
    let p95 = latencies[(latencies.len() as f64 * 0.95) as usize];
    let p99 = latencies[(latencies.len() as f64 * 0.99) as usize];

    println!("\n📊 Results (WebSocket streaming):");
    println!("   Total time:     {:?}", total_time);
    println!("   Mean latency:   {} µs", mean);
    println!("   Median latency: {} µs", median);
    println!("   P95 latency:    {} µs", p95);
    println!("   P99 latency:    {} µs", p99);
    println!(
        "   Throughput:     {:.2} req/sec",
        iterations as f64 / total_time.as_secs_f64()
    );

    Ok(())
}

/// Comparison benchmark of all methods
async fn run_benchmark() -> anyhow::Result<()> {
    println!("\n🏁 Running comprehensive benchmark...\n");
    println!("{}", "=".repeat(70));

    let iterations = 1000;

    // Test 1: Single requests with pooling
    test_single_requests(iterations).await?;
    println!("\n{}", "=".repeat(70));

    // Wait a bit between tests
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Test 2: Batch requests (batch size 10)
    test_batch_requests(iterations, 10).await?;
    println!("\n{}", "=".repeat(70));

    tokio::time::sleep(Duration::from_secs(1)).await;

    // Test 3: Batch requests (batch size 100)
    test_batch_requests(iterations, 100).await?;
    println!("\n{}", "=".repeat(70));

    tokio::time::sleep(Duration::from_secs(1)).await;

    // Test 4: WebSocket streaming
    test_websocket(iterations).await?;
    println!("\n{}", "=".repeat(70));

    println!("\n📝 Summary:");
    println!("   HTTP/2 pooled:         Good for standard REST APIs (200-500 µs)");
    println!("   Batch (size 10):       Better for high throughput (~50 µs per request)");
    println!("   Batch (size 100):      Best for bulk operations (~10 µs per request)");
    println!("   WebSocket streaming:   Best for real-time applications (100-300 µs)");

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    let mode = if args.len() > 2 && args[1] == "--mode" {
        &args[2]
    } else {
        "single"
    };

    match mode {
        "single" => test_single_requests(100).await?,
        "batch" => test_batch_requests(1000, 100).await?,
        "websocket" => test_websocket(100).await?,
        "benchmark" => run_benchmark().await?,
        _ => {
            println!("Usage: cargo run --example http_client -- --mode <single|batch|websocket|benchmark>");
            std::process::exit(1);
        }
    }

    Ok(())
}
