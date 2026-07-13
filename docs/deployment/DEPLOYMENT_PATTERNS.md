# Reaper Deployment Patterns

## Quick Reference

This document provides a quick visual reference for the three main deployment patterns for Reaper.

---

## Pattern 1: Standalone (Simple)

**Best for:** Development, testing, simple deployments, edge devices

```
┌─────────────────────────────────┐
│   Your Application              │
│                                 │
│   - Load policies from files    │
│   - HTTP API to agent           │
└────────────┬────────────────────┘
             │ HTTP
             ▼
┌─────────────────────────────────┐
│   Reaper Agent :8080            │
│                                 │
│   Policy Sources:               │
│   1. Local files (/etc/reaper)  │
│   2. Direct API calls           │
│   3. Default policy             │
└────────────┬────────────────────┘
             │ In-process
             ▼
┌─────────────────────────────────┐
│   Policy Engine (Library)       │
│                                 │
│   - Sub-microsecond eval        │
│   - Lock-free cache             │
└─────────────────────────────────┘
```

**Configuration:**

```yaml
# /etc/reaper/agent.yaml
agent:
  port: 8080

policies:
  bootstrap_dir: "/etc/reaper/policies"

data:
  bootstrap_file: "/etc/reaper/data/entities.json"
```

**Startup:**

```bash
# Put policies in directory
mkdir -p /etc/reaper/policies
cp my-policy.yaml /etc/reaper/policies/

# Start agent
reaper-agent --config /etc/reaper/agent.yaml
```

**Pros:**
- ✅ Simple setup
- ✅ No external dependencies
- ✅ Works offline
- ✅ Full control
- ✅ Low latency

**Cons:**
- ❌ Manual policy updates
- ❌ No centralized management
- ❌ No version control
- ❌ No team scoping

---

## Pattern 2: Integrated with Sync Client

**Best for:** Production deployments, multi-team organizations, managed environments

```
┌──────────────────────────────────────────┐
│   Management Server (Cloud)              │
│                                          │
│   - Policy repository                    │
│   - Version management                   │
│   - Team-based scoping                   │
│   - Deployment orchestration             │
│   - Analytics & audit logs               │
└────────────┬─────────────────────────────┘
             │ HTTPS (Authenticated)
             │ • Poll for updates
             │ • Pull policies
             │ • Report metrics
             ▼
┌──────────────────────────────────────────┐
│   Reaper Sync Client                     │
│                                          │
│   - Polls server every 30s               │
│   - Downloads policy updates             │
│   - Pushes to local agent                │
│   - Caches for offline mode              │
└────────────┬─────────────────────────────┘
             │ HTTP (Local)
             │ • Deploy policies
             │ • Sync data
             ▼
┌──────────────────────────────────────────┐
│   Reaper Agent :8080                     │
│                                          │
│   Policy Sources:                        │
│   1. Sync client (preferred)             │
│   2. Cached policies                     │
│   3. Direct API (manual override)        │
└────────────┬─────────────────────────────┘
             │ In-process
             ▼
┌──────────────────────────────────────────┐
│   Policy Engine (Library)                │
│                                          │
│   - Sub-microsecond eval                 │
│   - Lock-free cache                      │
└──────────────────────────────────────────┘
```

**Configuration:**

```yaml
# /etc/reaper/agent.yaml
agent:
  port: 8080

policies:
  cache_dir: "/var/cache/reaper/policies"

data:
  cache_dir: "/var/cache/reaper/data"
```

```yaml
# /etc/reaper/sync.yaml
sync:
  server:
    url: "https://reaper-mgmt.example.com"

  auth:
    type: "api_token"
    token_file: "/etc/reaper/secrets/token"

  scope:
    teams: ["engineering", "platform"]
    environments: ["production"]

  behavior:
    mode: "active"
    poll_interval_seconds: 30

  agent:
    url: "http://localhost:8080"

  cache:
    directory: "/var/cache/reaper/sync"
    enable_offline_mode: true
```

**Startup:**

```bash
# Start agent
systemctl start reaper-agent

# Start sync client
systemctl start reaper-sync

# Sync client will automatically:
# 1. Connect to management server
# 2. Download policies for your teams
# 3. Push them to local agent
# 4. Keep them updated
```

**Pros:**
- ✅ Centralized policy management
- ✅ Automatic updates
- ✅ Version control & rollback
- ✅ Team-based scoping
- ✅ Audit logging
- ✅ Works offline (cached)
- ✅ Analytics & reporting

**Cons:**
- ❌ More complex setup
- ❌ Requires management server
- ❌ Network dependency (mitigated by cache)

---

## Pattern 3: Embedded (Direct Library Usage)

**Best for:** Embedded systems, edge devices, serverless functions, maximum performance

```
┌──────────────────────────────────────────┐
│   Your Application                       │
│                                          │
│   use policy_engine::PolicyEngine;       │
│                                          │
│   fn main() {                            │
│       let engine = PolicyEngine::new();  │
│       engine.deploy_policy(policy)?;     │
│                                          │
│       let decision = engine.evaluate(    │
│           &policy_id,                    │
│           &request                       │
│       )?;                                │
│   }                                      │
└────────────┬─────────────────────────────┘
             │ In-process (no network)
             ▼
┌──────────────────────────────────────────┐
│   Policy Engine (Library)                │
│                                          │
│   - Sub-microsecond eval                 │
│   - Lock-free cache                      │
│   - Zero network overhead                │
└──────────────────────────────────────────┘
```

**Code Example:**

```rust
// Cargo.toml
[dependencies]
policy-engine = { path = "../reaper/crates/policy-engine" }

// main.rs
use policy_engine::{
    PolicyEngine, PolicyRequest, PolicyRule,
    PolicyAction, EnhancedPolicy
};

fn main() -> anyhow::Result<()> {
    // Create engine
    let engine = PolicyEngine::new();

    // Create policy
    let policy = EnhancedPolicy::new(
        "api-access".to_string(),
        "Allow API access".to_string(),
        vec![PolicyRule {
            action: PolicyAction::Allow,
            resource: "/api/*".to_string(),
            conditions: vec![],
        }],
    );

    // Deploy policy
    engine.deploy_policy(policy.clone())?;

    // Evaluate request
    let request = PolicyRequest {
        resource: "/api/users".to_string(),
        action: "read".to_string(),
        context: Default::default(),
    };

    let decision = engine.evaluate(&policy.id, &request)?;

    println!("Decision: {:?}", decision.decision);

    Ok(())
}
```

**Pros:**
- ✅ Maximum performance (no network)
- ✅ Zero external dependencies
- ✅ Smallest footprint
- ✅ Complete control
- ✅ Perfect for embedded systems

**Cons:**
- ❌ No service layer
- ❌ No HTTP API
- ❌ Manual policy management
- ❌ Must handle policies in code

---

## Comparison Matrix

| Feature | Standalone | Integrated | Embedded |
|---------|-----------|-----------|----------|
| **Setup Complexity** | Low | Medium | Low |
| **External Dependencies** | None | Management Server | None |
| **Automatic Updates** | ❌ | ✅ | ❌ |
| **Version Control** | ❌ | ✅ | ❌ |
| **Team Scoping** | ❌ | ✅ | ❌ |
| **Offline Operation** | ✅ | ✅ (cached) | ✅ |
| **Performance** | Sub-µs | Sub-µs | Sub-µs |
| **Network Overhead** | None (local) | Minimal (polling) | None |
| **Deployment** | systemd/docker | systemd/docker/k8s | In-app |
| **HTTP API** | ✅ | ✅ | ❌ |
| **Policy Sources** | Files, API | Server, API, Cache | Code |
| **Best For** | Dev, Simple | Production | Embedded |

---

## Migration Paths

### From Standalone → Integrated

1. Install sync client
2. Configure server connection
3. Set team scope
4. Start sync client
5. Policies automatically sync

**No changes needed to:**
- Reaper Agent
- Your application code
- Evaluation API calls

---

### From Integrated → Standalone

1. Stop sync client
2. Cached policies remain active
3. Agent continues working
4. Update policies manually if needed

**No data loss:**
- All policies are cached locally
- Agent works independently

---

### From Embedded → Standalone

1. Deploy Reaper Agent
2. Convert in-code policies to files
3. Update app to call HTTP API
4. Remove policy-engine dependency

**Benefits:**
- Centralized policy management
- Hot-reload policies without redeploying app

---

## Kubernetes Deployment Examples

### Standalone Pattern

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: my-app
spec:
  template:
    spec:
      containers:
      - name: app
        image: my-app:latest

      - name: reaper-agent
        image: reaper-agent:latest
        ports:
        - containerPort: 8080
        volumeMounts:
        - name: policies
          mountPath: /etc/reaper/policies

      volumes:
      - name: policies
        configMap:
          name: reaper-policies
```

---

### Integrated Pattern (Sidecar)

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: my-app
spec:
  template:
    spec:
      containers:
      - name: app
        image: my-app:latest

      - name: reaper-agent
        image: reaper-agent:latest
        ports:
        - containerPort: 8080

      - name: reaper-sync
        image: reaper-sync:latest
        env:
        - name: SERVER_URL
          value: "https://reaper-mgmt.example.com"
        - name: TEAM_SCOPE
          value: "engineering"
        - name: API_TOKEN
          valueFrom:
            secretKeyRef:
              name: reaper-token
              key: token
```

---

## Decision Guide

### Choose **Standalone** if:
- 🎯 You need simple, quick setup
- 🎯 You have a single application
- 🎯 You manage policies manually
- 🎯 You don't need centralized management
- 🎯 You're in development/testing

### Choose **Integrated** if:
- 🎯 You have multiple teams
- 🎯 You need centralized policy management
- 🎯 You want automatic policy updates
- 🎯 You need version control and rollback
- 🎯 You want audit logging and analytics
- 🎯 You're in production

### Choose **Embedded** if:
- 🎯 You need maximum performance
- 🎯 You're building an embedded system
- 🎯 You want zero external dependencies
- 🎯 You manage policies in code
- 🎯 You need the smallest footprint

---

## Autonomous Auto-Rollback (Rollout Supervisor)

The management server runs a background **rollout supervisor** that turns the
auto-rollback configuration into an autonomous control loop: every tick it
evaluates each active rollout's agent-deployment failure rate against the
org/namespace auto-rollback config, and — when armed — cancels a breaching
rollout and rolls the fleet back to the previous bundle without a human in
the loop. Under multiple management replicas, a per-tick advisory lock elects
a single supervisor.

### Arming: monitor → enforce

Auto-rollback configs carry a `mode` that gates what the supervisor does when
the error-rate trigger fires:

| Mode | Behavior |
|------|----------|
| `monitor` (default) | Evaluate + write an audit entry (`deployment.auto_rollback_triggered`) + emit the `auto_rollback_triggered` SSE event + increment `reaper_management_auto_rollbacks_total{mode="monitor"}`. **No action taken.** |
| `enforce` | Cancel the breaching rollout, start an immediate rollback to the previous bundle, audit as `deployment.auto_rollback` (system actor), emit the event, increment the counter with `mode="enforce"`. |

Recommended rollout of the feature itself: enable with the default `monitor`
mode first, watch the audit log / SSE events for false positives while tuning
`error_rate_threshold` and `min_requests`, then arm `enforce` per namespace:

```bash
# 1. Dry-run: enable in monitor mode (org-wide)
curl -X POST /api/v1/orgs/{org}/auto-rollback \
  -d '{"is_enabled": true, "error_rate_threshold": 5.0, "min_requests": 100, "mode": "monitor"}'

# 2. After confidence: arm enforcement for one namespace
curl -X POST /api/v1/orgs/{org}/namespaces/{ns}/auto-rollback \
  -d '{"is_enabled": true, "mode": "enforce"}'

# Inspect what the supervisor sees for a rollout (read-only)
curl /api/v1/orgs/{org}/rollouts/{rollout_id}/rollback-status
```

Rollback rollouts started by the supervisor are stamped
`triggered_by=auto_rollback` and are excluded from supervision, so the
remediation itself can never be auto-rolled-back (no rollback loops). In
monitor mode each breaching rollout is alerted once, not every tick (a
management-server restart re-alerts once).

### Environment variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `REAPER_ROLLOUT_SUPERVISOR_ENABLED` | `true` | Set `false` to disable the supervisor loop entirely |
| `REAPER_ROLLOUT_SUPERVISOR_INTERVAL_SECS` | `30` | Tick interval (minimum 5s) |

In the Helm chart these are set via `management.config.rolloutSupervisor.*`
(`enabled`, `intervalSeconds`).

---

## Summary

All three patterns use the **same policy engine core** with **identical performance characteristics**. The difference is in **how policies are managed and deployed**.

Start with **Standalone** for simplicity, migrate to **Integrated** when you need centralized management, or use **Embedded** when you need maximum control and performance.

The architecture ensures you can **switch between patterns** without losing data or performance.
