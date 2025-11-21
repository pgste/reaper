# HTTP Optimization Guide for Reaper

## The Challenge: Network Overhead vs. Evaluation Speed

**The Problem:**
- Reaper evaluates policies in **2µs** ⚡
- Network protocols add **10-2000µs** overhead 🐌
- Traditional REST can make authorization **250-1000x slower** than it needs to be!

**The Solution:**
Multiple connection strategies optimized for different use cases, all minimizing the bottleneck.

## Quick Reference: Protocol Comparison

| Protocol | Latency | Throughput | Use Case | Implementation |
|----------|---------|------------|----------|----------------|
| **Unix Socket** | 10-50µs | 100k-200k ops/s | Sidecar (same pod) | Binary protocol over UDS |
| **HTTP/2 Pooled** | 200-500µs | 5k-20k ops/s | Standard REST API | Keep-Alive + connection pool |
| **HTTP/2 Batch** | 10-50µs/req | 50k-100k ops/s | Bulk operations | POST /v1/evaluate/batch |
| **WebSocket** | 100-300µs | 10k-30k ops/s | Real-time streaming | WS /v1/stream |
| **HTTP/1.1** | 500-2000µs | 1k-5k ops/s | Legacy/simple | Basic REST |

## HTTP/2 with Persistent Connections (Recommended for REST)

### What It Solves

**Without Optimization (HTTP/1.1, new connection per request):**
```
TCP handshake:         500µs
TLS handshake:         500µs
HTTP request:          100µs
Reaper evaluation:     2µs    ← The actual work!
HTTP response:         100µs
TCP teardown:          100µs
─────────────────────────────
Total:                 1,302µs (99.8% is overhead!)
```

**With HTTP/2 + Keep-Alive:**
```
[First request]
TCP handshake:         500µs
TLS handshake:         500µs
HTTP/2 setup:          50µs
Reaper evaluation:     2µs
Response:              50µs
─────────────────────────────
First request:         1,102µs

[Subsequent requests - reuse connection]
HTTP/2 request:        50µs
Reaper evaluation:     2µs
Response:              50µs
─────────────────────────────
Per request:           102µs  (98% improvement!)
```

### Implementation

#### Server Side (Axum)

```rust
// HTTP/2 is enabled by default in Axum with hyper
// Just start the server normally:

let app = Router::new()
    .route("/v1/evaluate", post(handle_evaluate))
    .with_state(state);

let listener = TcpListener::bind("0.0.0.0:3000").await?;
axum::serve(listener, app).await?;

// Axum automatically:
// - Enables HTTP/2
// - Handles Keep-Alive
// - Manages connection lifecycle
```

#### Client Side (Rust)

```rust
use reqwest::{Client, ClientBuilder};
use std::time::Duration;

// Create a client with connection pooling
let client = ClientBuilder::new()
    .pool_max_idle_per_host(10)              // Keep 10 connections warm
    .pool_idle_timeout(Duration::from_secs(90))  // Keep-Alive timeout
    .http2_prior_knowledge()                 // Force HTTP/2
    .tcp_keepalive(Duration::from_secs(60))  // TCP keep-alive
    .build()?;

// Reuse this client for ALL requests!
// Don't create a new client per request

for request in requests {
    let response = client.post("http://localhost:3000/v1/evaluate")
        .json(&request)
        .send()
        .await?;

    let decision: EvaluateResponse = response.json().await?;
}
```

#### Client Side (Python)

```python
import httpx
import asyncio

# Use httpx with connection pooling
async with httpx.AsyncClient(
    http2=True,                          # Enable HTTP/2
    limits=httpx.Limits(
        max_keepalive_connections=10,    # Pool size
        keepalive_expiry=90.0,           # Keep-Alive timeout
    ),
) as client:
    # All requests reuse connections
    for request in requests:
        response = await client.post(
            "http://localhost:3000/v1/evaluate",
            json=request
        )
        decision = response.json()
```

#### Client Side (Node.js)

```javascript
// Use node-fetch or axios with http2
const http2 = require('http2');
const { promisify } = require('util');

// Create persistent HTTP/2 session
const session = http2.connect('http://localhost:3000');

async function evaluate(request) {
    const req = session.request({
        ':method': 'POST',
        ':path': '/v1/evaluate',
        'content-type': 'application/json',
    });

    req.write(JSON.stringify(request));
    req.end();

    const chunks = [];
    for await (const chunk of req) {
        chunks.push(chunk);
    }

    return JSON.parse(Buffer.concat(chunks).toString());
}

// Reuse session for all requests!
```

#### Client Side (Go)

```go
import (
    "net/http"
    "time"
)

// Create client with connection pooling
client := &http.Client{
    Transport: &http.Transport{
        MaxIdleConns:        10,
        MaxIdleConnsPerHost: 10,
        IdleConnTimeout:     90 * time.Second,
        ForceAttemptHTTP2:   true,
    },
}

// Reuse this client for all requests
for _, request := range requests {
    resp, err := client.Post(
        "http://localhost:3000/v1/evaluate",
        "application/json",
        bytes.NewBuffer(requestJSON),
    )
    // ... handle response
}
```

### Key Principles

1. **Create ONE client, reuse everywhere**
   ```rust
   // ❌ BAD: Creates new connection every time
   for req in requests {
       let client = Client::new();  // DON'T DO THIS!
       let resp = client.post(url).json(&req).send().await?;
   }

   // ✅ GOOD: Reuses connections
   let client = Client::new();  // Create ONCE
   for req in requests {
       let resp = client.post(url).json(&req).send().await?;
   }
   ```

2. **Configure appropriate pool size**
   - Low traffic: 2-5 connections
   - Medium traffic: 5-10 connections
   - High traffic: 10-20 connections
   - Don't go crazy: More isn't always better!

3. **Set reasonable timeouts**
   ```rust
   .pool_idle_timeout(Duration::from_secs(90))  // Keep-Alive
   .timeout(Duration::from_secs(10))            // Request timeout
   .connect_timeout(Duration::from_secs(5))     // Connection timeout
   ```

## Batch Requests: Amortize Connection Overhead

### When to Use

- Bulk authorization checks
- Background processing
- Data migrations
- Batch exports

### How It Works

**Single requests (100 checks):**
```
100 requests × 200µs = 20,000µs (20ms)
```

**Batched (100 checks in 1 request):**
```
1 request × 500µs = 500µs (0.5ms)
Per-request amortized: 5µs ← 40x faster!
```

### Implementation

#### Client Side

```rust
// Collect requests
let requests: Vec<EvaluateRequest> = users.iter()
    .map(|user| EvaluateRequest {
        principal: user.id.clone(),
        action: "read".to_string(),
        resource: "dashboard".to_string(),
    })
    .collect();

// Send as single batch
let batch = BatchEvaluateRequest { requests };
let response = client.post("http://localhost:3000/v1/evaluate/batch")
    .json(&batch)
    .send()
    .await?;

let results: BatchEvaluateResponse = response.json().await?;

// Process all results
for (user, result) in users.iter().zip(results.results.iter()) {
    if result.allowed {
        // Grant access
    }
}
```

#### Server Side

```rust
#[derive(Deserialize)]
struct BatchEvaluateRequest {
    requests: Vec<EvaluateRequest>,
}

async fn handle_batch_evaluate(
    State(state): State<Arc<AppState>>,
    Json(batch): Json<BatchEvaluateRequest>,
) -> Json<BatchEvaluateResponse> {
    let results: Vec<EvaluateResponse> = batch.requests
        .iter()
        .map(|req| {
            let decision = state.evaluator.evaluate(&PolicyRequest {
                principal: state.store.intern(&req.principal),
                action: state.store.intern(&req.action),
                resource: state.store.intern(&req.resource),
            })?;

            Ok(EvaluateResponse { /* ... */ })
        })
        .collect::<Result<Vec<_>, _>>()?;

    Json(BatchEvaluateResponse {
        results,
        total_time_ns: start.elapsed().as_nanos(),
        count: results.len(),
    })
}
```

### Optimal Batch Sizes

| Batch Size | Amortized Latency | Throughput | Best For |
|------------|-------------------|------------|----------|
| 10         | ~50µs/req        | 20k ops/s  | Small batches |
| 50         | ~15µs/req        | 65k ops/s  | Medium batches |
| 100        | ~10µs/req        | 100k ops/s | Large batches |
| 1000       | ~5µs/req         | 200k ops/s | Bulk processing |

**Trade-off:** Larger batches = better throughput but higher latency for first result.

## WebSocket Streaming: Real-Time Persistent Connection

### When to Use

- Real-time applications
- Long-lived connections
- Bidirectional communication
- Push notifications

### Benefits

- **Single connection**: No repeated handshakes
- **Low overhead**: Binary framing, minimal headers
- **Bidirectional**: Server can push to client
- **Stateful**: Maintain context across requests

### Implementation

#### Server Side (Axum)

```rust
async fn handle_websocket(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> Response {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: Arc<AppState>) {
    while let Some(msg) = socket.recv().await {
        let msg = msg.unwrap();

        if let Message::Text(text) = msg {
            // Parse request
            let req: EvaluateRequest = serde_json::from_str(&text)?;

            // Evaluate
            let decision = state.evaluator.evaluate(&PolicyRequest {
                principal: state.store.intern(&req.principal),
                action: state.store.intern(&req.action),
                resource: state.store.intern(&req.resource),
            })?;

            // Send response
            let response = EvaluateResponse { /* ... */ };
            let json = serde_json::to_string(&response)?;
            socket.send(Message::Text(json)).await?;
        }
    }
}
```

#### Client Side (Rust)

```rust
use tokio_tungstenite::{connect_async, tungstenite::Message};
use futures_util::{StreamExt, SinkExt};

// Connect once
let (ws_stream, _) = connect_async("ws://localhost:3000/v1/stream").await?;
let (mut write, mut read) = ws_stream.split();

// Send many requests over the same connection
for request in requests {
    let json = serde_json::to_string(&request)?;
    write.send(Message::Text(json)).await?;

    // Receive response
    if let Some(msg) = read.next().await {
        let response: EvaluateResponse = serde_json::from_str(msg?.to_text()?)?;
        println!("Decision: {:?}", response);
    }
}
```

#### Client Side (JavaScript/Browser)

```javascript
// Connect once
const ws = new WebSocket('ws://localhost:3000/v1/stream');

ws.onopen = () => {
    console.log('Connected!');

    // Send requests as needed
    ws.send(JSON.stringify({
        principal: 'user_123',
        action: 'read',
        resource: 'doc_456'
    }));
};

ws.onmessage = (event) => {
    const response = JSON.parse(event.data);
    console.log('Decision:', response.decision);
    console.log('Allowed:', response.allowed);
};
```

#### Client Side (Python)

```python
import asyncio
import websockets
import json

async def stream_evaluations():
    async with websockets.connect('ws://localhost:3000/v1/stream') as ws:
        # Send requests
        for request in requests:
            await ws.send(json.dumps(request))

            # Receive response
            response = json.loads(await ws.recv())
            print(f"Decision: {response['decision']}")

asyncio.run(stream_evaluations())
```

### WebSocket vs HTTP/2

| Feature | WebSocket | HTTP/2 |
|---------|-----------|--------|
| Connection | Single persistent | Pool of persistent |
| Overhead | 2-6 bytes per frame | ~50 bytes per request |
| Bidirectional | Yes | Request-response only |
| Server push | Native | Stream/SSE workaround |
| Best latency | 100-300µs | 200-500µs |
| Browser support | Excellent | Excellent |
| Load balancing | Tricky (sticky sessions) | Easy |

## Advanced Optimizations

### 1. Client-Side Caching

**Cache repeated authorization checks:**

```rust
use std::collections::HashMap;
use std::time::{Instant, Duration};

struct CachedClient {
    client: Client,
    cache: HashMap<(String, String, String), (bool, Instant)>,
    ttl: Duration,
}

impl CachedClient {
    async fn evaluate(&mut self, req: &EvaluateRequest) -> Result<bool> {
        let key = (req.principal.clone(), req.action.clone(), req.resource.clone());

        // Check cache
        if let Some((allowed, cached_at)) = self.cache.get(&key) {
            if cached_at.elapsed() < self.ttl {
                return Ok(*allowed);  // 50ns cache hit!
            }
        }

        // Cache miss - fetch from server
        let response = self.client.post(url)
            .json(req)
            .send()
            .await?;

        let result: EvaluateResponse = response.json().await?;

        // Cache result
        self.cache.insert(key, (result.allowed, Instant::now()));

        Ok(result.allowed)
    }
}
```

**Cache hit rates:**
- 90% cache hit rate → 10x faster overall
- 95% cache hit rate → 20x faster overall
- 99% cache hit rate → 100x faster overall

**Recommended TTL:**
- User permissions: 30-60 seconds
- Static resources: 5-10 minutes
- Dynamic data: 5-15 seconds

### 2. Request Pipelining

**Send multiple requests without waiting for responses:**

```rust
// ❌ Sequential (slow)
let r1 = client.post(url).json(&req1).send().await?;
let r2 = client.post(url).json(&req2).send().await?;
let r3 = client.post(url).json(&req3).send().await?;
// Total: 3 × 200µs = 600µs

// ✅ Pipelined (fast)
let (r1, r2, r3) = tokio::join!(
    client.post(url).json(&req1).send(),
    client.post(url).json(&req2).send(),
    client.post(url).json(&req3).send(),
);
// Total: ~200µs (HTTP/2 multiplexing!)
```

### 3. Compression

**For large batch requests, enable compression:**

```rust
let client = ClientBuilder::new()
    .gzip(true)                    // Enable gzip compression
    .brotli(true)                  // Enable brotli (better compression)
    .deflate(true)                 // Enable deflate
    .build()?;
```

**Trade-offs:**
- Small requests (<1KB): Overhead not worth it
- Medium requests (1-10KB): ~30% size reduction
- Large requests (>10KB): ~50-70% size reduction

### 4. Timeout Configuration

```rust
let client = ClientBuilder::new()
    .connect_timeout(Duration::from_secs(5))   // Connection establishment
    .timeout(Duration::from_secs(10))          // Total request timeout
    .pool_idle_timeout(Duration::from_secs(90)) // Connection keep-alive
    .build()?;
```

**Recommended timeouts:**
- Connect: 5 seconds (DNS + TCP + TLS)
- Request: 10 seconds (includes queuing + evaluation + network)
- Keep-Alive: 60-90 seconds (balance resources vs handshakes)

## Real-World Performance Comparison

### Scenario: Middleware Authorization Check

**Traditional OPA (HTTP/1.1, no pooling):**
```
Request handling:
├─ Parse request:       100µs
├─ Auth check (OPA):    10,000µs  ← Bottleneck!
│  ├─ TCP handshake:    500µs
│  ├─ TLS handshake:    500µs
│  ├─ HTTP request:     100µs
│  ├─ OPA evaluation:   5,000µs
│  ├─ HTTP response:    100µs
│  └─ Parse JSON:       200µs
├─ Business logic:      2,000µs
└─ Database query:      5,000µs
────────────────────────────────
Total:                  17,100µs (58% auth overhead)
```

**Reaper (HTTP/2 + pooling):**
```
Request handling:
├─ Parse request:       100µs
├─ Auth check (Reaper): 200µs  ← Fast!
│  ├─ HTTP/2 request:   50µs
│  ├─ Reaper eval:      2µs
│  ├─ HTTP/2 response:  50µs
│  └─ Parse JSON:       100µs
├─ Business logic:      2,000µs
└─ Database query:      5,000µs
────────────────────────────────
Total:                  7,300µs (2.7% auth overhead)
```

**Reaper (WebSocket):**
```
Request handling:
├─ Parse request:       100µs
├─ Auth check (Reaper): 100µs  ← Even faster!
│  ├─ WS frame:         20µs
│  ├─ Reaper eval:      2µs
│  ├─ WS response:      20µs
│  └─ Parse JSON:       60µs
├─ Business logic:      2,000µs
└─ Database query:      5,000µs
────────────────────────────────
Total:                  7,200µs (1.4% auth overhead)
```

**Reaper (Unix Socket sidecar):**
```
Request handling:
├─ Parse request:       100µs
├─ Auth check (Reaper): 20µs   ← Blazing!
│  ├─ UDS send:         5µs
│  ├─ Reaper eval:      2µs
│  ├─ UDS recv:         5µs
│  └─ Parse binary:     8µs
├─ Business logic:      2,000µs
└─ Database query:      5,000µs
────────────────────────────────
Total:                  7,120µs (0.3% auth overhead)
```

## Deployment Architecture

### Architecture 1: HTTP/2 Service

```
┌─────────────────────────────────────────────┐
│           Load Balancer (HTTP/2)            │
└───────────────┬─────────────────────────────┘
                │
    ┌───────────┼───────────┐
    ▼           ▼           ▼
┌────────┐  ┌────────┐  ┌────────┐
│ Reaper │  │ Reaper │  │ Reaper │
│Service │  │Service │  │Service │
│  Pod   │  │  Pod   │  │  Pod   │
└────────┘  └────────┘  └────────┘
```

**Kubernetes Service:**
```yaml
apiVersion: v1
kind: Service
metadata:
  name: reaper-service
spec:
  selector:
    app: reaper
  ports:
  - name: http2
    port: 3000
    protocol: TCP
  type: ClusterIP
```

**Client configuration:**
```rust
// Use Kubernetes service DNS
let client = Client::new();
let url = "http://reaper-service.default.svc.cluster.local:3000/v1/evaluate";
```

### Architecture 2: Sidecar with HTTP

```
┌────────────────────────────────────┐
│           Kubernetes Pod            │
│                                    │
│  ┌──────────┐     ┌─────────────┐ │
│  │   App    │◄───►│   Reaper    │ │
│  │Container │HTTP │  Sidecar    │ │
│  │          │     │(localhost)  │ │
│  └──────────┘     └─────────────┘ │
└────────────────────────────────────┘
```

**Connection:**
```rust
// Connect to localhost (same pod)
let client = Client::new();
let url = "http://localhost:3000/v1/evaluate";

// Ultra-low latency: ~50-100µs
```

## Testing and Benchmarking

### Run the Examples

```bash
# Terminal 1: Start server
cargo run --example http_server --release -- \
    --policy examples/rbac.reap \
    --data large-test-data.json \
    --port 3000

# Terminal 2: Test different modes
cargo run --example http_client --release -- --mode single
cargo run --example http_client --release -- --mode batch
cargo run --example http_client --release -- --mode websocket
cargo run --example http_client --release -- --mode benchmark
```

### Expected Results

```
📊 HTTP/2 Pooled (1000 requests):
   Mean latency:   250-500 µs
   Throughput:     5,000-10,000 req/sec

📦 Batch (1000 requests, batch size 100):
   Amortized:      10-20 µs per request
   Throughput:     50,000-100,000 req/sec

🔌 WebSocket (1000 requests):
   Mean latency:   100-300 µs
   Throughput:     10,000-30,000 req/sec
```

## Summary: Choosing the Right Protocol

### Decision Matrix

| Scenario | Recommended | Latency | Complexity |
|----------|-------------|---------|------------|
| Sidecar (same pod) | Unix Socket | 10-50µs | Low |
| Standard REST API | HTTP/2 Pooled | 200-500µs | Low |
| Bulk operations | Batch HTTP/2 | 10-50µs/req | Medium |
| Real-time apps | WebSocket | 100-300µs | Medium |
| Browser/Web apps | WebSocket or HTTP/2 | 100-500µs | Low-Medium |
| Legacy systems | HTTP/1.1 Keep-Alive | 500-1000µs | Very Low |

### Best Practices Checklist

✅ **Always use connection pooling**
✅ **Enable HTTP/2 when possible**
✅ **Batch requests for bulk operations**
✅ **Use WebSocket for real-time applications**
✅ **Add client-side caching with appropriate TTL**
✅ **Set reasonable timeouts**
✅ **Monitor connection pool metrics**
✅ **Consider sidecar pattern for lowest latency**

### Key Takeaway

Even with network overhead, **Reaper is 10-100x faster than OPA** because:
1. Sub-2µs evaluation (vs 5-50ms for competitors)
2. Optimized protocols minimize overhead
3. Smart caching reduces requests
4. Batching amortizes connection costs

**Authorization should be <1% of your request time, not 50-60%!** 🚀
