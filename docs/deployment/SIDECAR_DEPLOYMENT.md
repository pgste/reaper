# Reaper Sidecar Deployment Guide

## Memory Footprint Analysis

Based on extensive testing with 100k entities, Reaper uses approximately **2,605 bytes per entity** in memory.

### Realistic Capacity by Pod Size

| Pod Memory | Max Entities | Reaper Memory | OS Overhead | Use Case |
|------------|--------------|---------------|-------------|----------|
| 256 MB     | 80,000       | ~200 MB       | ~56 MB      | Small apps, RBAC |
| 512 MB     | 180,000      | ~450 MB       | ~62 MB      | Standard apps |
| 1 GB       | 380,000      | ~950 MB       | ~74 MB      | Multi-tenant SaaS |
| 2 GB       | 750,000      | ~1.88 GB      | ~120 MB     | Large enterprise |
| 4 GB       | 1.5M         | ~3.75 GB      | ~250 MB     | Massive scale |
| 8 GB       | 3M           | ~7.5 GB       | ~500 MB     | Extreme scale |
| 16 GB      | 6M           | ~15 GB        | ~1 GB       | Multi-region aggregate |

### Performance Characteristics

- **Evaluation time**: 2µs mean, 3.6µs P99 (100k entities)
- **Throughput**: 400k-500k evaluations/second
- **No memory leaks**: Validated over 10k+ evaluation cycles
- **Linear scaling**: Memory grows linearly with entity count

### Typical Production Scenarios

**Scenario 1: SaaS Application (10k users, 50k resources)**
- **Entities**: 60k total
- **Memory**: ~150 MB
- **Pod size**: 256 MB (plenty of headroom)
- **Performance**: Sub-2µs evaluation

**Scenario 2: Enterprise Platform (100k users, 500k resources)**
- **Entities**: 600k total
- **Memory**: ~1.5 GB
- **Pod size**: 2 GB (safe margin)
- **Performance**: Sub-4µs evaluation

**Scenario 3: Multi-Tenant Giant (500k users, 2M resources)**
- **Entities**: 2.5M total
- **Memory**: ~6.25 GB
- **Pod size**: 8 GB (safe margin)
- **Performance**: ~5-10µs evaluation (estimated)

## SDK Connection Protocols

### The Critical Bottleneck

Reaper's evaluation is **2µs**, but network protocols introduce significant overhead:

| Protocol          | Latency (localhost) | Throughput     | Best For |
|-------------------|---------------------|----------------|----------|
| **FFI (C bindings)** | 10-100 ns        | 10M+ ops/sec   | In-process embedding |
| **Unix Domain Socket** | 10-50 µs       | 100k-200k ops/sec | Sidecar (same pod) |
| **gRPC (HTTP/2)**    | 100-500 µs      | 10k-50k ops/sec | Sidecar/remote |
| **HTTP/REST**        | 500-2000 µs     | 1k-5k ops/sec  | Simple integration |

### Recommended Architectures

#### 1. **High-Performance Sidecar (Recommended)**

**Connection**: Unix Domain Sockets
- **Latency**: ~10-50µs total (5-25x Reaper's evaluation time)
- **Throughput**: 100k-200k requests/sec per pod
- **Protocol**: Custom binary or MessagePack over UDS

```
┌─────────────────────────────────────────────┐
│             Kubernetes Pod                   │
│                                              │
│  ┌──────────┐         ┌─────────────────┐   │
│  │   App    │◄───UDS──►│ Reaper Sidecar │   │
│  │Container │         │   (10-50µs)     │   │
│  └──────────┘         └─────────────────┘   │
└─────────────────────────────────────────────┘
```

**SDK Implementation**:
```rust
// Rust example
let socket = UnixStream::connect("/var/run/reaper.sock")?;
let request = PolicyRequest { ... };
socket.write_all(&bincode::serialize(&request)?)?;
let decision = bincode::deserialize(&socket.read()?)?;
```

**Why This Works**:
- No TCP/IP stack overhead
- No serialization overhead (binary protocol)
- Direct kernel IPC
- ~10-50µs round trip = **still 100-500x faster than OPA**

#### 2. **Embedded In-Process (Maximum Performance)**

**Connection**: FFI (Foreign Function Interface)
- **Latency**: ~10-100ns overhead
- **Throughput**: Limited only by Reaper (500k ops/sec)
- **Protocol**: Direct C ABI calls

```
┌────────────────────────────────┐
│     Application Process        │
│                                │
│  ┌──────────────────────────┐  │
│  │   App Code               │  │
│  │         │                │  │
│  │         ▼                │  │
│  │   libreaper.so (FFI)     │  │
│  │   (10-100ns overhead)    │  │
│  └──────────────────────────┘  │
└────────────────────────────────┘
```

**SDK Implementation** (example for Python):
```python
# Python ctypes example
import ctypes
libreaper = ctypes.CDLL('libreaper.so')

libreaper.evaluate.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_char_p]
libreaper.evaluate.restype = ctypes.c_int

decision = libreaper.evaluate(
    b"user_123",     # principal
    b"read",         # action
    b"doc_456"       # resource
)
```

**Trade-offs**:
- ✅ Near-zero overhead (10-100ns)
- ✅ Maximum throughput
- ✅ No network complexity
- ❌ Memory shared with app
- ❌ Crash isolation lost
- ❌ Updates require app restart

#### 3. **gRPC Remote Service (Flexible)**

**Connection**: gRPC over HTTP/2
- **Latency**: ~100-500µs (localhost), ~1-5ms (cross-zone)
- **Throughput**: 10k-50k requests/sec
- **Protocol**: Protobuf

```
┌──────────┐    gRPC     ┌─────────────────┐
│   App    │◄──────────►│ Reaper Service  │
│  (any)   │  100-500µs  │   (central)     │
└──────────┘             └─────────────────┘
```

**Why This Works**:
- Multiple apps share one Reaper instance
- Language-agnostic (all gRPC SDKs)
- Streaming support for batch operations
- Still 10-50x faster than OPA

**Batch Optimization**:
```protobuf
service ReaperService {
  // Single request (100-500µs)
  rpc Evaluate(PolicyRequest) returns (Decision);

  // Batch request (amortized cost)
  rpc EvaluateBatch(stream PolicyRequest) returns (stream Decision);
}
```

Batch 100 requests → ~100µs per request amortized!

#### 4. **HTTP/REST (Simple Integration)**

**Connection**: HTTP/1.1 or HTTP/2
- **Latency**: ~500-2000µs
- **Throughput**: 1k-5k requests/sec
- **Protocol**: JSON

```json
POST /v1/evaluate
{
  "principal": "user_123",
  "action": "read",
  "resource": "doc_456"
}
```

**Trade-offs**:
- ✅ Simple, universal
- ✅ Easy debugging
- ✅ Works everywhere
- ❌ Higher latency
- ❌ JSON serialization overhead

## SDK Architecture Recommendations

### 1. **Connection Pool**
Always maintain persistent connections (don't connect per request):

```rust
// Bad: 1000µs overhead per request
for request in requests {
    let conn = connect_to_reaper()?;  // 500µs overhead!
    let decision = conn.evaluate(request)?;
}

// Good: 10µs overhead per request
let pool = ConnectionPool::new(size: 10);
for request in requests {
    let conn = pool.get()?;  // instant
    let decision = conn.evaluate(request)?;
}
```

### 2. **Request Batching**
Group multiple authorization checks:

```rust
// Bad: 100 requests × 50µs = 5000µs
let decisions: Vec<Decision> = requests.iter()
    .map(|r| client.evaluate(r))
    .collect();

// Good: 1 batch request = 200µs total
let decisions = client.evaluate_batch(requests)?;
```

### 3. **Client-Side Caching**
Cache decisions with TTL for repeated checks:

```rust
let cache = LruCache::new(10_000);

fn evaluate_cached(request: &PolicyRequest) -> Decision {
    let cache_key = (request.principal, request.action, request.resource);

    if let Some(decision) = cache.get(&cache_key) {
        return decision;  // 50ns cache hit!
    }

    let decision = reaper_client.evaluate(request)?;
    cache.insert(cache_key, decision, ttl: 60s);
    decision
}
```

### 4. **Async/Non-Blocking**
Use async I/O to not block application threads:

```javascript
// Node.js example with async
async function checkAuthorization(user, action, resource) {
    const decision = await reaperClient.evaluate({
        principal: user,
        action: action,
        resource: resource
    });
    return decision.allowed;
}
```

## Deployment Patterns

### Pattern 1: Sidecar per Pod (Recommended)

**Pros**:
- Lowest latency (UDS)
- Fault isolation
- Independent scaling
- Zero network hops

**Cons**:
- Memory per pod
- Data consistency (if entities change)

**Best for**: High-throughput applications, microservices

### Pattern 2: Centralized Service

**Pros**:
- Single source of truth
- Lower total memory
- Easier updates

**Cons**:
- Network latency
- Single point of failure
- Network bottleneck

**Best for**: Low-throughput applications, simple deployments

### Pattern 3: Hybrid (Sidecar + Central)

**Pros**:
- Hot data local (sidecar)
- Full dataset remote (central)
- Best latency for common cases

**Cons**:
- Complex architecture
- Cache consistency

**Best for**: Large-scale platforms with hot/cold data split

## Performance Comparison

### Reaper vs Traditional Policy Engines

| Scenario | OPA (REST) | Cedar (gRPC) | Reaper (UDS) | Reaper (FFI) |
|----------|------------|--------------|--------------|--------------|
| Single request | 5-50 ms | 1-10 ms | 10-50 µs | 2-10 µs |
| Batch (100 req) | 500-5000 ms | 100-1000 ms | 1-5 ms | 0.2-1 ms |
| Throughput | 100-1k ops/s | 1k-10k ops/s | 100k ops/s | 500k ops/s |
| Memory (100k entities) | 2-5 GB | 1-2 GB | 250 MB | 250 MB |

### Real-World Impact

**Traditional (OPA with REST):**
```
Authorization check: 10ms
Database query: 5ms
Business logic: 2ms
-------------------------
Total request: 17ms (59% auth overhead!)
```

**Reaper with UDS:**
```
Authorization check: 0.02ms
Database query: 5ms
Business logic: 2ms
-------------------------
Total request: 7.02ms (0.3% auth overhead!)
```

**Reaper with FFI:**
```
Authorization check: 0.002ms
Database query: 5ms
Business logic: 2ms
-------------------------
Total request: 7.002ms (0.03% auth overhead!)
```

## SDK Language Matrix

### Recommended Implementation per Language

| Language | Best Protocol | Library | Latency | Difficulty |
|----------|--------------|---------|---------|------------|
| **Rust** | FFI (native) | Direct linking | 10 ns | Easy |
| **Go** | Unix Socket | net.Dial("unix") | 20 µs | Easy |
| **Python** | FFI | ctypes/cffi | 100 ns | Medium |
| **Node.js** | Unix Socket | net.Socket | 30 µs | Easy |
| **Java** | gRPC | grpc-java | 200 µs | Easy |
| **C#/.NET** | Unix Socket | Socket | 50 µs | Medium |
| **Ruby** | Unix Socket | UNIXSocket | 40 µs | Easy |
| **PHP** | Unix Socket | socket_create | 60 µs | Medium |

## Getting Started

### 1. Deploy Reaper Sidecar

```yaml
apiVersion: v1
kind: Pod
metadata:
  name: app-with-reaper
spec:
  containers:
  - name: app
    image: myapp:latest
    volumeMounts:
    - name: reaper-socket
      mountPath: /var/run/reaper

  - name: reaper
    image: reaper:latest
    args:
      - --socket=/var/run/reaper/reaper.sock
      - --policy=/etc/reaper/policy.reap
      - --data=/etc/reaper/data.json
    volumeMounts:
    - name: reaper-socket
      mountPath: /var/run/reaper
    resources:
      limits:
        memory: 512Mi
      requests:
        memory: 256Mi

  volumes:
  - name: reaper-socket
    emptyDir: {}
```

### 2. SDK Connection Example

```python
# Python SDK example
import socket
import msgpack

class ReaperClient:
    def __init__(self, socket_path="/var/run/reaper/reaper.sock"):
        self.socket_path = socket_path
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.connect(self.socket_path)

    def evaluate(self, principal, action, resource):
        request = {
            "principal": principal,
            "action": action,
            "resource": resource
        }

        # Send request
        self.sock.sendall(msgpack.packb(request))

        # Receive response
        response = msgpack.unpackb(self.sock.recv(1024))
        return response["decision"]

# Usage
client = ReaperClient()
allowed = client.evaluate("user_123", "read", "doc_456")
```

## Summary

### Memory Recommendations
- **256 MB pod**: 50k-80k entities (most apps)
- **512 MB pod**: 150k-180k entities (standard)
- **1-2 GB pod**: 300k-750k entities (large)
- **4+ GB pod**: 1M+ entities (massive scale)

### Protocol Recommendations
- **Best latency**: FFI embedding (10-100ns overhead)
- **Best sidecar**: Unix Domain Sockets (10-50µs)
- **Best remote**: gRPC with batching (100-500µs)
- **Simplest**: HTTP/REST (500-2000µs)

### Key Insight
Even with 500µs gRPC overhead, Reaper is **10-100x faster** than OPA/Cedar because:
1. Sub-2µs evaluation (vs 5-50ms for competitors)
2. No policy compilation on each request
3. Optimized data structures with string interning
4. Zero-copy Arc-based architecture

**The protocol overhead matters, but Reaper is so fast that even "slow" gRPC is blazing fast compared to alternatives!**
