# Phase 8.1: Production Deployment & Future Vision

**Status**: In Progress
**Date**: 2025-12-14

---

## 🎯 Objectives

1. ✅ **Bundle Compilation** - Pre-compile .reap → .rbb for instant loading
2. 🔄 **Container Deployment** - Docker images for Agent and Platform
3. 🔄 **Health & Readiness** - Production health checks
4. 🔄 **Configuration Management** - Environment-based config
5. 📋 **Future WASM Vision** - Browser & service mesh execution

---

## Part 1: Bundle Compilation Workflow

### Current Status: ✅ IMPLEMENTED

Bundle format already exists (`src/reap/bundle.rs`):
- **Format**: Binary bundle (.rbb)
- **Magic bytes**: `REAP` (4 bytes)
- **Serialization**: bincode (fast, compact)
- **Metadata**: Version, timestamp, checksum
- **API**: `compile_to_bundle()` and `load_from_bundle()`

### Usage

```rust
// Compile .reap to .rbb
let policy: ReaperPolicy = policy_text.parse()?;
let bundle_bytes = policy.compile_to_bundle()?;
std::fs::write("policy.rbb", bundle_bytes)?;

// Load .rbb (instant, no parsing)
let bundle_bytes = std::fs::read("policy.rbb")?;
let policy = ReaperPolicy::from_bundle(&bundle_bytes, store)?;
```

### CLI Integration Needed

Add to `reaper-cli`:

```bash
# Compile single policy
reaper-cli compile policy.reap -o policy.rbb

# Compile multiple policies
reaper-cli compile policies/*.reap -o bundle.rbb

# Validate bundle
reaper-cli validate bundle.rbb

# Inspect bundle metadata
reaper-cli inspect bundle.rbb
```

### Benefits

- **Instant Loading**: No parsing overhead (~10-100x faster)
- **Size**: Compact binary format (~30-50% smaller than YAML)
- **Integrity**: Built-in checksum validation
- **Versioning**: Format version for compatibility

---

## Part 2: Container Deployment

### Dockerfile Strategy

**Multi-stage build**:
1. **Builder stage**: Compile Rust binaries (release mode)
2. **Runtime stage**: Minimal container (distroless or alpine)

**Key Features**:
- Static linking for portability
- Non-root user for security
- Health check support
- Signal handling for graceful shutdown

### Agent Dockerfile

```dockerfile
# syntax=docker/dockerfile:1
FROM rust:1.75-alpine AS builder

WORKDIR /build
COPY . .

RUN apk add --no-cache musl-dev && \
    cargo build --release --bin reaper-agent

FROM alpine:3.19
RUN apk add --no-cache ca-certificates

COPY --from=builder /build/target/release/reaper-agent /usr/local/bin/
USER 1000:1000

EXPOSE 8080
HEALTHCHECK --interval=10s --timeout=3s --start-period=5s --retries=3 \
  CMD wget --quiet --tries=1 --spider http://localhost:8080/health || exit 1

ENTRYPOINT ["/usr/local/bin/reaper-agent"]
```

### Platform Dockerfile

```dockerfile
# syntax=docker/dockerfile:1
FROM rust:1.75-alpine AS builder

WORKDIR /build
COPY . .

RUN apk add --no-cache musl-dev && \
    cargo build --release --bin reaper-platform

FROM alpine:3.19
RUN apk add --no-cache ca-certificates

COPY --from=builder /build/target/release/reaper-platform /usr/local/bin/
USER 1000:1000

EXPOSE 8081
HEALTHCHECK --interval=10s --timeout=3s --start-period=5s --retries=3 \
  CMD wget --quiet --tries=1 --spider http://localhost:8081/health || exit 1

ENTRYPOINT ["/usr/local/bin/reaper-platform"]
```

### docker-compose.yml

```yaml
version: '3.9'

services:
  reaper-platform:
    build:
      context: .
      dockerfile: services/reaper-platform/Dockerfile
    ports:
      - "8081:8081"
    environment:
      - RUST_LOG=info
      - REAPER_BIND_ADDR=0.0.0.0:8081
    volumes:
      - platform-data:/data
    networks:
      - reaper-net
    healthcheck:
      test: ["CMD", "wget", "--quiet", "--tries=1", "--spider", "http://localhost:8081/health"]
      interval: 10s
      timeout: 3s
      retries: 3
      start_period: 5s

  reaper-agent:
    build:
      context: .
      dockerfile: services/reaper-agent/Dockerfile
    ports:
      - "8080:8080"
    environment:
      - RUST_LOG=info
      - REAPER_BIND_ADDR=0.0.0.0:8080
      - PLATFORM_URL=http://reaper-platform:8081
    volumes:
      - agent-data:/data
      - ./policies:/policies:ro  # Mount policy bundles
    networks:
      - reaper-net
    depends_on:
      reaper-platform:
        condition: service_healthy
    healthcheck:
      test: ["CMD", "wget", "--quiet", "--tries=1", "--spider", "http://localhost:8080/health"]
      interval: 10s
      timeout: 3s
      retries: 3
      start_period: 5s

networks:
  reaper-net:
    driver: bridge

volumes:
  platform-data:
  agent-data:
```

---

## Part 3: Health & Readiness Checks

### Endpoints to Add

**Health Check** (`/health`):
- Returns 200 OK if service is alive
- No dependencies checked
- Fast (<10ms)

**Readiness Check** (`/ready`):
- Returns 200 OK if service can handle requests
- Checks critical dependencies (DB, policy store)
- Used by Kubernetes for traffic routing

**Liveness Check** (`/live`):
- Returns 200 OK if service should not be restarted
- Used by Kubernetes for container restart decisions

### Implementation

```rust
// Add to both Agent and Platform

async fn health_check() -> impl IntoResponse {
    Json(json!({
        "status": "healthy",
        "timestamp": chrono::Utc::now().to_rfc3339()
    }))
}

async fn readiness_check(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Check if policy engine is initialized
    let policy_count = state.policy_engine.get_all_policies().len();

    if policy_count == 0 {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "status": "not_ready",
                "reason": "No policies loaded"
            }))
        );
    }

    (
        StatusCode::OK,
        Json(json!({
            "status": "ready",
            "policies_loaded": policy_count
        }))
    )
}

async fn liveness_check() -> impl IntoResponse {
    // Simple check - if we can respond, we're alive
    StatusCode::OK
}
```

---

## Part 4: Configuration Management

### Environment Variables

**Agent Configuration**:
```bash
# Network
REAPER_BIND_ADDR=0.0.0.0:8080
PLATFORM_URL=http://platform:8081

# Policies
POLICY_DIR=/policies
POLICY_REFRESH_INTERVAL=60s

# Performance
MAX_CONCURRENT_REQUESTS=10000
POLICY_CACHE_SIZE=1000

# Logging
RUST_LOG=info,reaper_agent=debug
LOG_FORMAT=json
```

**Platform Configuration**:
```bash
# Network
REAPER_BIND_ADDR=0.0.0.0:8081

# Storage
DATA_DIR=/data
ENABLE_PERSISTENCE=true

# Agent Management
AGENT_TIMEOUT=30s

# Logging
RUST_LOG=info,reaper_platform=debug
LOG_FORMAT=json
```

---

## 🚀 Future Vision: WASM Compilation

### Phase 9.x: WebAssembly Target

**Objectives**:
1. Compile policy engine to WASM
2. Run policies in browser (client-side authorization)
3. Run in service mesh (Envoy WASM filter)
4. Zero-overhead policy enforcement

### Use Cases

#### 1. Browser-Based Policy Evaluation

```javascript
// Load policy engine WASM module
import init, { PolicyEngine } from './reaper_wasm.js';

await init();
const engine = new PolicyEngine();

// Load precompiled bundle
const bundle = await fetch('/policies/rbac.rbb');
engine.loadBundle(await bundle.arrayBuffer());

// Evaluate (sub-microsecond, no network call)
const decision = engine.evaluate({
    principal: userId,
    action: 'read',
    resource: documentId
});

if (decision.allow) {
    // Render document
}
```

**Benefits**:
- **Zero latency**: No API call to policy service
- **Offline capable**: Works without network
- **Privacy**: Sensitive data never leaves browser
- **Scale**: No backend bottleneck

#### 2. Envoy WASM Filter (Service Mesh)

```rust
// Envoy WASM filter - runs on every request
#[no_mangle]
pub fn proxy_on_request_headers(...) -> Action {
    let policy_engine = get_cached_engine();

    // Extract request attributes
    let principal = get_header("x-user-id");
    let resource = get_header(":path");

    // Evaluate policy (sub-microsecond)
    let decision = policy_engine.evaluate(principal, "GET", resource);

    if decision.allow {
        Action::Continue
    } else {
        send_http_response(403, ...)
        Action::Pause
    }
}
```

**Benefits**:
- **Zero hops**: Policy evaluated in-proxy
- **Sub-microsecond**: Negligible latency overhead
- **No sidecar**: Runs directly in Envoy
- **HA**: No policy service SPOF

#### 3. eBPF Integration (Future)

**Vision**: Policy enforcement in kernel space

```c
// eBPF program - runs in kernel
SEC("lsm/socket_connect")
int reaper_socket_connect(struct socket *sock) {
    // Extract connection attributes
    u32 uid = bpf_get_current_uid_gid();
    u16 port = get_dest_port(sock);

    // Evaluate policy (eBPF map lookup)
    struct policy_decision *decision =
        bpf_map_lookup_elem(&policy_cache, &uid);

    if (decision && decision->allow) {
        return 0; // Allow
    }

    return -EPERM; // Deny
}
```

**Benefits**:
- **Kernel-level enforcement**: Cannot be bypassed
- **Nanosecond latency**: eBPF map lookup
- **Zero overhead**: No userspace context switch
- **Universal**: Works for all processes

---

## 🧠 Zero-Overhead Policy Patterns

### Pattern 1: Policy as Code (Current)

```
Request → Agent (HTTP) → Policy Engine → Decision
Latency: 50-200µs (network + eval)
```

### Pattern 2: Sidecar (Traditional)

```
Request → Sidecar → Policy Engine → Decision
Latency: 10-50µs (local IPC)
```

### Pattern 3: In-Process WASM

```
Request → App (with WASM) → Policy Engine → Decision
Latency: 1-5µs (function call)
```

### Pattern 4: Service Mesh WASM Filter

```
Request → Envoy (WASM filter) → Policy Engine → Decision
Latency: <1µs (in-proxy)
```

### Pattern 5: eBPF (Future)

```
Request → Kernel (eBPF) → Policy Map Lookup → Decision
Latency: <100ns (map lookup)
```

---

## 🎯 WASM Compilation Roadmap

### Phase 9.1: WASM Target Support (1 week)

**Tasks**:
1. Add `wasm32-unknown-unknown` target to Cargo.toml
2. Remove incompatible dependencies (tokio, etc.)
3. Create WASM-compatible API bindings
4. wasm-bindgen integration
5. Browser JavaScript wrapper

**Deliverables**:
- `reaper_wasm.wasm` - WebAssembly module
- `reaper_wasm.js` - JavaScript bindings
- `reaper_wasm.d.ts` - TypeScript definitions

### Phase 9.2: Browser SDK (1 week)

**Tasks**:
1. NPM package for browser use
2. Policy bundle loading API
3. Browser-compatible data store
4. Performance benchmarks
5. Demo application

**Deliverables**:
- `@reaper/policy-engine` NPM package
- Browser demo (React app)
- Performance comparison (WASM vs API call)

### Phase 9.3: Envoy WASM Filter (2 weeks)

**Tasks**:
1. Envoy Proxy SDK integration
2. WASM filter implementation
3. Policy bundle preloading
4. Metrics integration (Prometheus)
5. Deployment guide

**Deliverables**:
- Envoy WASM filter binary
- Kubernetes deployment manifests
- Performance benchmarks vs sidecar

### Phase 9.4: eBPF Integration (Research - 1 month)

**Tasks**:
1. eBPF policy compiler (Reaper DSL → eBPF bytecode)
2. Kernel-space policy map
3. LSM hook integration
4. Userspace control plane
5. Proof of concept

**Deliverables**:
- eBPF policy compiler
- Kernel module
- Performance comparison

---

## 🔧 Deployment Pattern Matrix

| Pattern | Latency | Overhead | Complexity | Use Case |
|---------|---------|----------|------------|----------|
| **HTTP Agent** | 50-200µs | Network call | Low | Simple deployments |
| **Sidecar** | 10-50µs | Extra container | Medium | Kubernetes |
| **In-Process** | 1-5µs | Library | Medium | Embedded |
| **WASM Filter** | <1µs | Negligible | High | Service mesh |
| **eBPF** | <100ns | None | Very High | Kernel enforcement |

---

## 📋 Phase 8.1 Task Breakdown

### Week 1: Bundle & Container Foundation

**Day 1-2**: Bundle Compilation
- [ ] Add CLI commands (compile, validate, inspect)
- [ ] Bundle format documentation
- [ ] Performance benchmarks (bundle vs parse)
- [ ] Integration tests

**Day 3-4**: Docker Containers
- [ ] Create Dockerfiles (Agent, Platform)
- [ ] Multi-stage build optimization
- [ ] Security hardening (non-root, minimal base)
- [ ] Image size optimization

**Day 5**: Docker Compose & Testing
- [ ] docker-compose.yml with networking
- [ ] End-to-end deployment test
- [ ] Volume persistence testing
- [ ] Health check validation

### Week 2: Production Readiness

**Day 1-2**: Health & Configuration
- [ ] Implement /health, /ready, /live endpoints
- [ ] Environment-based configuration
- [ ] Configuration validation
- [ ] Graceful shutdown handling

**Day 3**: Kubernetes Manifests
- [ ] Deployment manifests (Agent, Platform)
- [ ] Service definitions
- [ ] ConfigMaps and Secrets
- [ ] Ingress configuration

**Day 4-5**: Documentation & Testing
- [ ] Deployment guide
- [ ] Troubleshooting guide
- [ ] Load testing (1000+ RPS)
- [ ] High availability validation

---

## 🎯 Success Criteria

### Phase 8.1 Complete When:

- ✅ Bundle compilation CLI commands work
- ✅ Docker images build successfully (<500MB)
- ✅ docker-compose deploys full stack
- ✅ Health checks functional
- ✅ Kubernetes manifests deploy successfully
- ✅ Load testing passes (>10K RPS)
- ✅ Documentation complete

### Future WASM Vision:

- 📋 Roadmap documented
- 📋 Technical feasibility validated
- 📋 POC timeline established

---

## 📚 References

- Bundle format: `src/reap/bundle.rs`
- Current deployment docs: `docs/deployment/`
- Service mesh patterns: `docs/deployment/SIDECAR_DEPLOYMENT.md`

---

*Phase 8.1 started: 2025-12-14*
*Target completion: 2025-12-28 (2 weeks)*
