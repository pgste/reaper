# Reaper Zero-Overhead Deployment Vision

**From Network to Kernel: The Ultimate Performance Evolution**

---

## 🎯 The Vision

Move policy enforcement from **network latency** to **kernel-level performance**, enabling:

1. **eBPF LSM Deployment** - <100ns latency (kernel-level)
2. **WASM Service Mesh** - <1µs latency (Envoy/Istio filters)
3. **Browser SDK** - Offline-first (client-side evaluation)

**Impact**: 10-100x performance improvement + eliminate network hops + zero sidecar overhead

---

## 📊 Performance Comparison

| Deployment Pattern | Latency | Network Hops | Resource Usage | Status |
|-------------------|---------|--------------|----------------|--------|
| **HTTP Agent** (current) | 50-200µs | 1-2 | ~50MB RAM, 1 pod | ✅ Complete |
| **Sidecar** | 10-50µs | 0 (localhost) | ~30MB RAM per pod | 📋 Planned |
| **WASM Filter** | **<1µs** | 0 (in-proxy) | ~2MB per proxy | 🚀 This phase |
| **eBPF LSM** | **<100ns** | 0 (kernel) | ~500KB kernel | 🚀 This phase |
| **Browser SDK** | **0µs** | 0 (offline) | ~500KB bundle | 🚀 This phase |

---

## 1️⃣ eBPF LSM Deployment (Game-Changer)

### Overview

Use **Linux Security Module (LSM) BPF** to enforce policies directly in the kernel, intercepting system calls before they reach userspace.

**Key Technology**: [BPF LSM](https://docs.kernel.org/bpf/prog_lsm.html) (kernel 5.7+)

### Architecture

```
┌─────────────────────────────────────────────────────────┐
│                    USER SPACE                            │
│                                                           │
│  Application → glibc → syscall() → [crosses into kernel] │
└─────────────────────────┬───────────────────────────────┘
                          ↓
┌─────────────────────────────────────────────────────────┐
│                  KERNEL SPACE                            │
│                                                           │
│  ┌─────────────────────────────────────────────────┐   │
│  │  LSM Hook (e.g., file_open, socket_connect)     │   │
│  │         ↓                                        │   │
│  │  [eBPF Program: reaper_authorize()]             │   │
│  │         ↓                                        │   │
│  │  • Load policy from eBPF map                    │   │
│  │  • Evaluate rules (simplified for eBPF)         │   │
│  │  • Return: 0 (allow) or -EPERM (deny)           │   │
│  │                                                   │   │
│  │  Latency: <100ns (no context switch!)           │   │
│  └─────────────────────────────────────────────────┘   │
│                          ↓                               │
│  Actual System Call (if allowed)                        │
└─────────────────────────────────────────────────────────┘
```

### LSM Hooks We Can Use

| Hook | Use Case | Example |
|------|----------|---------|
| `file_open` | File access control | Prevent reading `/etc/shadow` |
| `socket_connect` | Network policy | Block connections to external IPs |
| `bprm_check` | Process execution | Prevent running certain binaries |
| `task_kill` | Signal control | Prevent killing processes |
| `ptrace_access_check` | Debugging control | Prevent attaching debugger |

### Implementation Approach

#### Option 1: Aya (Rust eBPF Framework)

**Repository**: [aya-rs/aya](https://github.com/aya-rs/aya)

```rust
// Kernel-space eBPF program (compiled to BPF bytecode)
use aya_bpf::{
    macros::lsm,
    programs::LsmContext,
};

#[lsm(hook = "file_open")]
pub fn reaper_file_open(ctx: LsmContext) -> i32 {
    // Extract file path from LSM context
    let path = unsafe { ctx.arg::<*const u8>(0) };

    // Look up policy in BPF map
    let policy = unsafe { POLICIES.get(&resource_key) };

    // Evaluate policy (simplified for eBPF constraints)
    if policy.action == Action::Allow {
        return 0;  // Allow
    }

    -13  // -EPERM = Deny
}

// User-space control plane (Rust)
use aya::{Bpf, programs::Lsm};

fn main() {
    let mut bpf = Bpf::load_file("reaper_ebpf.o")?;
    let program: &mut Lsm = bpf.program_mut("reaper_file_open")?.try_into()?;
    program.load()?;
    program.attach()?;

    // Load policies into BPF maps
    let policies = bpf.map_mut("POLICIES")?;
    policies.insert(resource_key, policy, 0)?;
}
```

#### Option 2: libbpf-rs (C interop)

For maximum compatibility with existing eBPF tooling.

### eBPF Constraints & Solutions

| Constraint | Impact | Solution |
|------------|--------|----------|
| **No dynamic memory** | Can't allocate arbitrary memory | Use pre-allocated BPF maps |
| **Bounded loops** | Verifier rejects unbounded loops | Limit policy rules (e.g., max 16 rules) |
| **Limited stack** | 512 bytes stack limit | Use BPF maps for large data |
| **No string operations** | Can't use standard string matching | Use byte-by-byte comparison or BPF helpers |
| **Complexity limit** | ~1M instructions per program | Simplify policy evaluation logic |

**Solution**: Create **two-tier architecture**:
1. **eBPF tier** (kernel) - Fast path for simple policies (RBAC, IP allow/deny)
2. **Userspace tier** (agent) - Slow path for complex policies (ABAC, Cedar)

### Policy Compilation for eBPF

```rust
// Simplified policy representation for eBPF
#[repr(C)]
struct EbpfPolicy {
    principal_id: u64,       // Hash of principal
    resource_pattern: [u8; 64], // Resource path pattern
    action: u8,              // 0=Deny, 1=Allow
    priority: u16,           // Rule priority
}

// Compilation: Full Reaper policy → eBPF-compatible
fn compile_to_ebpf(policy: &EnhancedPolicy) -> Vec<EbpfPolicy> {
    policy.rules.iter()
        .filter(|r| r.is_simple())  // Only simple rules
        .map(|r| EbpfPolicy {
            principal_id: hash(r.principal),
            resource_pattern: r.resource.as_bytes(),
            action: if r.action == Allow { 1 } else { 0 },
            priority: r.priority,
        })
        .collect()
}
```

### Use Cases

**Perfect for**:
- **Network policies** - Allow/deny connections by IP/port
- **File access control** - Prevent access to sensitive files
- **Container security** - Restrict syscalls per container
- **Zero-trust networking** - Kernel-level service-to-service auth

**Not suitable for**:
- Complex ABAC with dynamic attributes (use userspace agent)
- Policies requiring external API calls (use async userspace)

### Real-World Example: Cilium Tetragon

**Reference**: [Cilium Tetragon](https://tetragon.io/)

Tetragon uses eBPF LSM for runtime security enforcement:
```yaml
apiVersion: cilium.io/v1alpha1
kind: TracingPolicy
spec:
  kprobes:
  - call: "security_file_permission"
    syscall: false
    args:
    - index: 0
      type: "file"
    selectors:
    - matchArgs:
      - index: 0
        operator: "Equal"
        values:
        - "/etc/shadow"
      matchActions:
      - action: Block
```

### Deployment

```yaml
# Kubernetes DaemonSet
apiVersion: apps/v1
kind: DaemonSet
metadata:
  name: reaper-ebpf
spec:
  template:
    spec:
      hostPID: true
      hostNetwork: true
      containers:
      - name: reaper-ebpf
        image: reaper/ebpf-agent:latest
        securityContext:
          privileged: true  # Required for loading eBPF
          capabilities:
            add: ["SYS_ADMIN", "SYS_RESOURCE", "NET_ADMIN"]
        volumeMounts:
        - name: sys
          mountPath: /sys
        - name: bpf
          mountPath: /sys/fs/bpf
```

### Performance Characteristics

- **Latency**: <100ns (no userspace context switch)
- **Throughput**: Limited only by kernel scheduler
- **Overhead**: <1% CPU even at millions of decisions/sec
- **Memory**: ~500KB for eBPF program + maps

---

## 2️⃣ WASM Service Mesh Integration

### Overview

Compile Reaper policy engine to **WebAssembly** and deploy as **Envoy/Istio filters** for in-proxy policy enforcement.

**Key Technology**: [proxy-wasm-rust-sdk](https://github.com/proxy-wasm/proxy-wasm-rust-sdk)

### Architecture

```
┌───────────────────────────────────────────────────────┐
│              Envoy Proxy (C++)                         │
│                                                         │
│  HTTP Request                                          │
│       ↓                                                │
│  ┌────────────────────────────────────────────────┐  │
│  │  WASM Filter (reaper.wasm)                     │  │
│  │                                                 │  │
│  │  fn on_request_headers():                      │  │
│  │    • Extract principal from JWT                │  │
│  │    • Load policy from shared memory            │  │
│  │    • Evaluate policy (full Reaper engine!)     │  │
│  │    • Return: Allow or Deny                     │  │
│  │                                                 │  │
│  │  Latency: <1µs (compiled WASM!)                │  │
│  │  Isolated: WASM sandbox (crash-safe)           │  │
│  └────────────────────────────────────────────────┘  │
│       ↓                                                │
│  Upstream Service (if allowed)                        │
└───────────────────────────────────────────────────────┘
```

### Implementation

#### Step 1: Add WASM Target

```toml
# Cargo.toml
[lib]
crate-type = ["cdylib"]  # For WASM compilation

[dependencies]
proxy-wasm = "0.2"
serde = { version = "1.0", default-features = false }
serde_json = { version = "1.0", default-features = false, features = ["alloc"] }
```

#### Step 2: Implement proxy-wasm Trait

```rust
// crates/policy-engine-wasm/src/lib.rs
use proxy_wasm::traits::*;
use proxy_wasm::types::*;

struct ReaperFilter {
    policy_engine: PolicyEngine,
}

impl Context for ReaperFilter {}

impl HttpContext for ReaperFilter {
    fn on_http_request_headers(&mut self, _: usize, _: bool) -> Action {
        // Extract JWT from Authorization header
        let jwt = self.get_http_request_header("authorization");

        // Extract resource and action from request
        let resource = self.get_http_request_header(":path").unwrap();
        let action = self.get_http_request_header(":method").unwrap();

        // Evaluate policy
        let request = PolicyRequest {
            resource,
            action,
            context: extract_claims(jwt),
        };

        match self.policy_engine.evaluate(&policy_id, &request) {
            Ok(decision) if decision.decision == PolicyAction::Allow => {
                Action::Continue  // Allow request
            }
            Ok(decision) => {
                // Deny request
                self.send_http_response(
                    403,
                    vec![("content-type", "application/json")],
                    Some(b"{\"error\":\"Forbidden\"}"),
                );
                Action::Pause
            }
            Err(_) => {
                self.send_http_response(500, vec![], Some(b"Policy error"));
                Action::Pause
            }
        }
    }
}

proxy_wasm::main! {{
    proxy_wasm::set_root_context(|_| -> Box<dyn RootContext> {
        Box::new(ReaperFilter {
            policy_engine: PolicyEngine::new(),
        })
    });
}}
```

#### Step 3: Compile to WASM

```bash
# Add WASM target
rustup target add wasm32-unknown-unknown

# Compile
cargo build --target wasm32-unknown-unknown --release

# Optimize WASM bundle
wasm-opt -Oz -o reaper_optimized.wasm \
  target/wasm32-unknown-unknown/release/reaper_wasm.wasm
```

**Result**: ~500KB WASM bundle with full policy engine!

### Envoy Configuration

```yaml
static_resources:
  listeners:
  - name: listener_0
    address:
      socket_address:
        address: 0.0.0.0
        port_value: 10000
    filter_chains:
    - filters:
      - name: envoy.filters.network.http_connection_manager
        typed_config:
          "@type": type.googleapis.com/envoy.extensions.filters.network.http_connection_manager.v3.HttpConnectionManager
          http_filters:
          - name: envoy.filters.http.wasm
            typed_config:
              "@type": type.googleapis.com/envoy.extensions.filters.http.wasm.v3.Wasm
              config:
                name: "reaper_filter"
                root_id: "reaper"
                vm_config:
                  runtime: "envoy.wasm.runtime.v8"
                  code:
                    local:
                      filename: "/etc/envoy/reaper.wasm"
                configuration:
                  "@type": "type.googleapis.com/google.protobuf.StringValue"
                  value: |
                    {
                      "policy_endpoint": "http://reaper-platform:8081/api/v1/policies"
                    }
          - name: envoy.filters.http.router
```

### Istio Integration

```yaml
apiVersion: extensions.istio.io/v1alpha1
kind: WasmPlugin
metadata:
  name: reaper-authz
  namespace: istio-system
spec:
  selector:
    matchLabels:
      istio: ingressgateway
  url: oci://ghcr.io/your-org/reaper-wasm:latest
  phase: AUTHN
  pluginConfig:
    policy_endpoint: "http://reaper-platform.reaper.svc.cluster.local:8081"
    cache_ttl: "60s"
```

### Performance Characteristics

- **Latency**: <1µs (compiled WASM, in-process)
- **Isolation**: WASM sandbox (filter crash won't crash Envoy)
- **Size**: ~500KB WASM bundle
- **Deployment**: OCI image, automatic distribution via Istio

### Benefits Over Sidecar

| Aspect | Sidecar | WASM Filter |
|--------|---------|-------------|
| **Latency** | 10-50µs | <1µs |
| **Memory** | ~30MB per pod | ~2MB per proxy |
| **Network hops** | 1 (localhost) | 0 (in-process) |
| **Deployment** | Separate container | Embedded in proxy |
| **Updates** | Restart pod | Hot reload |

---

## 3️⃣ Browser SDK (Offline-First)

### Overview

Compile policy engine to WASM for **client-side policy evaluation** in browsers, enabling offline-first applications.

### Architecture

```
┌────────────────────────────────────────────────────┐
│              Browser (JavaScript/WASM)              │
│                                                      │
│  User Action (e.g., "Delete Document")             │
│       ↓                                             │
│  ┌──────────────────────────────────────────────┐ │
│  │  Reaper Browser SDK (reaper.wasm)            │ │
│  │                                               │ │
│  │  await reaper.evaluate({                     │ │
│  │    resource: "/documents/123",               │ │
│  │    action: "delete",                         │ │
│  │    principal: currentUser                    │ │
│  │  });                                          │ │
│  │                                               │ │
│  │  → Returns: { decision: "allow" }            │ │
│  │  → Latency: 0µs (no network!)                │ │
│  └──────────────────────────────────────────────┘ │
│       ↓                                             │
│  Update UI (show/hide delete button)               │
└────────────────────────────────────────────────────┘

// Optional: Server-side verification (defense-in-depth)
User → API Call → Server re-evaluates policy
```

### Implementation

#### Step 1: Create Browser-Compatible Build

```rust
// crates/reaper-browser-sdk/src/lib.rs
use wasm_bindgen::prelude::*;
use serde::{Deserialize, Serialize};

#[wasm_bindgen]
pub struct ReaperSDK {
    engine: PolicyEngine,
}

#[wasm_bindgen]
impl ReaperSDK {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            engine: PolicyEngine::new(),
        }
    }

    #[wasm_bindgen]
    pub async fn load_policy(&mut self, policy_json: &str) -> Result<(), JsValue> {
        let policy: EnhancedPolicy = serde_json::from_str(policy_json)
            .map_err(|e| JsValue::from_str(&e.to_string()))?;

        self.engine.deploy_policy(policy)
            .map_err(|e| JsValue::from_str(&e.to_string()))?;

        Ok(())
    }

    #[wasm_bindgen]
    pub fn evaluate(&self, request_json: &str) -> Result<JsValue, JsValue> {
        let request: PolicyRequest = serde_json::from_str(request_json)
            .map_err(|e| JsValue::from_str(&e.to_string()))?;

        let decision = self.engine.evaluate(&policy_id, &request)
            .map_err(|e| JsValue::from_str(&e.to_string()))?;

        Ok(serde_wasm_bindgen::to_value(&decision)?)
    }
}
```

#### Step 2: Build with wasm-pack

```bash
# Install wasm-pack
cargo install wasm-pack

# Build for browser
wasm-pack build --target web crates/reaper-browser-sdk

# Output: pkg/reaper_browser_sdk.js + pkg/reaper_browser_sdk_bg.wasm
```

#### Step 3: JavaScript Integration

```javascript
// Load Reaper SDK
import init, { ReaperSDK } from './pkg/reaper_browser_sdk.js';

await init();  // Initialize WASM

// Create SDK instance
const reaper = new ReaperSDK();

// Load policy from server (or IndexedDB for offline)
const policyResponse = await fetch('/api/policies/my-policy');
const policy = await policyResponse.json();
await reaper.load_policy(JSON.stringify(policy));

// Evaluate policy (instant, no network!)
function canDeleteDocument(docId) {
  const decision = reaper.evaluate(JSON.stringify({
    resource: `/documents/${docId}`,
    action: "delete",
    principal: getCurrentUser(),
    context: {}
  }));

  return decision.decision === "allow";
}

// Use in UI
if (canDeleteDocument(123)) {
  showDeleteButton();
} else {
  hideDeleteButton();
}
```

### Use Cases

1. **Offline-first Apps** - PWAs that work without network
2. **Low-latency UI** - Instant permission checks (no API call)
3. **Privacy-preserving** - Evaluate policies locally (no data sent to server)
4. **Optimistic UI** - Show/hide UI elements based on permissions
5. **Mobile Apps** - Embed in React Native via WASM

### Example: React Hook

```typescript
// useReaperPolicy.ts
import { useEffect, useState } from 'react';
import { ReaperSDK } from './reaper_wasm';

export function useReaperPolicy(resource: string, action: string) {
  const [allowed, setAllowed] = useState(false);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    const sdk = new ReaperSDK();

    // Load policy from cache or server
    sdk.load_policy(getCachedPolicy()).then(() => {
      const decision = sdk.evaluate(JSON.stringify({
        resource,
        action,
        principal: getCurrentUser(),
      }));

      setAllowed(decision.decision === "allow");
      setLoading(false);
    });
  }, [resource, action]);

  return { allowed, loading };
}

// Usage in component
function DocumentView({ docId }) {
  const { allowed, loading } = useReaperPolicy(`/documents/${docId}`, "delete");

  return (
    <div>
      {!loading && allowed && (
        <button onClick={deleteDocument}>Delete</button>
      )}
    </div>
  );
}
```

### Bundle Size Optimization

```bash
# Before optimization
du -h pkg/reaper_browser_sdk_bg.wasm
# ~1.2MB

# With wasm-opt
wasm-opt -Oz pkg/reaper_browser_sdk_bg.wasm -o pkg/reaper_optimized.wasm
du -h pkg/reaper_optimized.wasm
# ~500KB

# With Brotli compression (served over HTTP)
brotli pkg/reaper_optimized.wasm
du -h pkg/reaper_optimized.wasm.br
# ~150KB (!)
```

**Result**: ~150KB over network (comparable to small JS libraries)

### Performance Characteristics

- **Initial Load**: ~150KB (Brotli-compressed WASM)
- **Evaluation Latency**: <1µs (in-memory, no network)
- **Memory**: ~2MB WASM runtime
- **Offline**: 100% functional without network

---

## 🎯 Implementation Roadmap

### Phase 9.1: WASM Foundation (2 weeks)

**Goal**: Get policy engine compiling to WASM

**Tasks**:
1. ✅ Research proxy-wasm-rust-sdk
2. Add `wasm32-unknown-unknown` target
3. Remove non-WASM-compatible dependencies
4. Create minimal WASM build
5. Test policy evaluation in WASM

**Deliverable**: `reaper.wasm` that can evaluate policies

### Phase 9.2: Envoy/Istio Integration (2 weeks)

**Goal**: Deploy as Envoy WASM filter

**Tasks**:
1. Implement proxy-wasm traits
2. Add policy loading from config
3. Create Envoy filter configuration
4. Create Istio WasmPlugin manifest
5. Performance benchmarks (<1µs target)

**Deliverable**: Reaper running in Envoy/Istio

### Phase 9.3: Browser SDK (1 week)

**Goal**: JavaScript SDK for client-side evaluation

**Tasks**:
1. Add wasm-bindgen support
2. Create JavaScript API
3. Build with wasm-pack
4. Create React/Vue/Svelte examples
5. Bundle size optimization

**Deliverable**: npm package `@reaper/browser-sdk`

### Phase 9.4: eBPF LSM (3-4 weeks)

**Goal**: Kernel-level policy enforcement

**Tasks**:
1. Research Aya framework
2. Implement simple eBPF LSM hook
3. Create policy map structures
4. Build userspace control plane
5. Test with Cilium Tetragon

**Deliverable**: DaemonSet for kernel-level enforcement

---

## 📚 Resources

### eBPF
- [LSM BPF Programs](https://docs.kernel.org/bpf/prog_lsm.html) - Kernel documentation
- [Practical Guide to LSM BPF](https://www.ebpf.top/en/post/lsm_bpf_intro/) - Tutorial
- [Cloudflare: Live-patching with eBPF LSM](https://blog.cloudflare.com/live-patch-security-vulnerabilities-with-ebpf-lsm/)
- [Cilium Tetragon](https://tetragon.io/) - Production eBPF runtime security
- [Aya](https://github.com/aya-rs/aya) - Rust eBPF framework

### WASM for Proxies
- [proxy-wasm-rust-sdk](https://github.com/proxy-wasm/proxy-wasm-rust-sdk) - Official Rust SDK
- [Extending Envoy with WASM and Rust](https://antweiss.com/blog/extending-envoy-with-wasm-and-rust/)
- [Martin Baillie: Envoy WASM Filters in Rust](https://martin.baillie.id/wrote/envoy-wasm-filters-in-rust/)
- [layer5io/wasm-filters](https://github.com/layer5io/wasm-filters) - Example filters

### Istio WASM
- [istio-ecosystem/wasm-extensions](https://github.com/istio-ecosystem/wasm-extensions) - Official extensions
- [OPA Istio Plugin](https://www.openpolicyagent.org/docs/latest/envoy-tutorial-istio/)
- [Istio WASM Plugin Reference](https://istio.io/latest/docs/reference/config/proxy_extensions/wasm-plugin/)

### Browser WASM
- [wasm-bindgen](https://rustwasm.github.io/wasm-bindgen/) - Rust ↔ JavaScript
- [wasm-pack](https://rustwasm.github.io/wasm-pack/) - Build tool

---

## 🏆 The Ultimate Stack

Once complete, users can choose their deployment pattern:

| Use Case | Deployment | Latency | Resource |
|----------|-----------|---------|----------|
| **Kubernetes ingress** | Envoy WASM filter | <1µs | ~2MB |
| **Service mesh** | Istio WASM plugin | <1µs | ~2MB |
| **Container security** | eBPF LSM | <100ns | ~500KB |
| **Serverless** | HTTP Agent | 50-200µs | ~50MB |
| **Browser/Mobile** | WASM SDK | 0µs | ~150KB |

**The vision**: One policy engine, deployed anywhere, from kernel to browser.

---

*Last updated: 2025-12-14*
*Reaper Policy Engine - Zero-Overhead Deployment*

Sources:
- [eBPF Ecosystem Progress 2024-2025](https://eunomia.dev/blog/2025/02/12/ebpf-ecosystem-progress-in-20242025-a-technical-deep-dive/)
- [Linux Security Module (LSM)](https://ebpf.hamza-megahed.com/docs/chapter5/2-lsm/)
- [LSM BPF Programs - Linux Kernel](https://docs.kernel.org/bpf/prog_lsm.html)
- [Extending Envoy With WASM and Rust](https://antweiss.com/blog/extending-envoy-with-wasm-and-rust/)
- [proxy-wasm-rust-sdk](https://github.com/proxy-wasm/proxy-wasm-rust-sdk)
- [Istio WASM Extensions](https://github.com/istio-ecosystem/wasm-extensions)
- [OPA Istio Tutorial](https://www.openpolicyagent.org/docs/latest/envoy-tutorial-istio/)
