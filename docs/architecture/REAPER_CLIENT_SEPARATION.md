# Reaper & Web Client Separation Architecture

## Overview

This document outlines the architecture for separating **Reaper** (the independent policy engine) from the **Web Client** (policy synchronization component) and integration with a future **Management Server**.

## Core Principles

1. **Reaper is fully independent** - Works standalone without any external services
2. **Web Client is optional** - Enables integration with centralized management
3. **Clear separation of concerns** - Each component has a single, well-defined responsibility
4. **Backwards compatible** - Existing deployments continue to work

---

## Architecture Diagram

```
┌──────────────────────────────────────────────────────────────┐
│                    Management Server (Future)                 │
│                                                               │
│  Responsibilities:                                            │
│  • Central policy repository                                 │
│  • Version management & rollback                             │
│  • Team-based policy scoping                                 │
│  • Deployment orchestration                                  │
│  • Audit logging & compliance                                │
│  • Policy testing & validation                               │
│  • Analytics & reporting                                     │
│                                                               │
│  API:                                                         │
│  • GET  /api/v1/policies?scope=team&version=latest          │
│  • GET  /api/v1/policies/:id/versions                       │
│  • POST /api/v1/policies/:id/deploy                         │
│  • GET  /api/v1/policies/:id/data                           │
└─────────────────────────┬────────────────────────────────────┘
                          │
                          │ HTTPS/gRPC (Authenticated)
                          │ • TLS mutual auth
                          │ • API tokens
                          │ • Policy pull
                          │ • Data sync
                          │
                          ▼
┌──────────────────────────────────────────────────────────────┐
│              Reaper Sync Client (New Component)               │
│                                                               │
│  Responsibilities:                                            │
│  • Poll management server for policy updates                 │
│  • Handle authentication & authorization                     │
│  • Download policies & associated data                       │
│  • Push updates to local Reaper Agent                        │
│  • Cache policies for offline operation                      │
│  • Health monitoring & metrics reporting                     │
│  • Graceful degradation (works without server)               │
│                                                               │
│  Configuration:                                               │
│  • server_url: "https://reaper-mgmt.example.com"            │
│  • poll_interval: 30s                                        │
│  • team_scope: ["engineering", "platform"]                   │
│  • agent_url: "http://localhost:8080"                        │
│  • cache_dir: "/var/cache/reaper"                            │
│  • offline_mode: true/false                                  │
│                                                               │
│  Modes:                                                       │
│  1. Active Sync - Continuously poll & update                 │
│  2. On-Demand - Update on trigger/webhook                    │
│  3. Offline - Use cached policies only                       │
└─────────────────────────┬────────────────────────────────────┘
                          │
                          │ Local HTTP (Unauthenticated)
                          │ • POST /api/v1/policies/deploy
                          │ • POST /api/v1/data/sync
                          │
                          ▼
┌──────────────────────────────────────────────────────────────┐
│                Reaper Agent (Independent Service)             │
│                                                               │
│  Responsibilities:                                            │
│  • Policy evaluation (sub-microsecond)                       │
│  • Request processing                                        │
│  • Metrics collection                                        │
│  • Works completely independently                            │
│                                                               │
│  Policy Sources (Priority Order):                            │
│  1. Sync Client updates (hot-swap)                           │
│  2. Direct API calls (manual deployment)                     │
│  3. Local file loading (bootstrap)                           │
│  4. Default policy (fallback)                                │
│                                                               │
│  API:                                                         │
│  • POST /api/v1/messages           - Policy evaluation       │
│  • POST /api/v1/policies/deploy    - Deploy policy           │
│  • POST /api/v1/data/sync          - Sync entity data        │
│  • GET  /api/v1/policies           - List active policies    │
│  • GET  /health                     - Health check           │
│  • GET  /metrics                    - Performance metrics    │
└─────────────────────────┬────────────────────────────────────┘
                          │
                          │ In-Process (No Network)
                          │
                          ▼
┌──────────────────────────────────────────────────────────────┐
│                Policy Engine (Core Library)                   │
│                                                               │
│  Characteristics:                                             │
│  • Pure Rust library (crate)                                 │
│  • Zero external service dependencies                        │
│  • Can be embedded in any Rust application                   │
│  • Lock-free concurrent data structures                      │
│  • Sub-microsecond evaluation                                │
│  • Multiple policy language support                          │
│                                                               │
│  Can be used:                                                 │
│  • Standalone in applications                                │
│  • Via Reaper Agent service                                  │
│  • Embedded in edge devices                                  │
│  • In serverless functions                                   │
└──────────────────────────────────────────────────────────────┘
```

---

## Component Details

### 1. Policy Engine (Core Library)

**Status:** ✅ Already implemented
**Location:** `crates/policy-engine/`

**Characteristics:**
- Fully independent Rust crate
- No network dependencies
- No database dependencies
- Can be embedded anywhere

**Key APIs:**
```rust
// Standalone usage
let engine = PolicyEngine::new();
engine.deploy_policy(policy)?;
let decision = engine.evaluate(&policy_id, &request)?;

// Load from files
let policy = ReaperPolicy::from_file("policy.yaml")?;
engine.deploy_policy(policy.into())?;
```

**Dependencies:**
- `dashmap` - Lock-free HashMap
- `parking_lot` - Synchronization
- `serde` - Serialization
- `uuid` - IDs

---

### 2. Reaper Agent (Independent Service)

**Status:** ✅ Already implemented
**Location:** `services/reaper-agent/`

**Current State:**
- Runs on port 8080
- Has policy evaluation endpoint
- Has deployment endpoint
- Works independently

**Enhancements Needed:**
1. Add data synchronization endpoint
2. Add policy source tracking
3. Add offline mode support
4. Add policy caching to disk

**New Endpoints:**
```rust
// New endpoint for data sync
POST /api/v1/data/sync
{
  "entities": [
    {
      "id": "user:alice",
      "type": "User",
      "attributes": {
        "role": "admin",
        "department": "engineering"
      }
    }
  ]
}

// Enhanced deployment with metadata
POST /api/v1/policies/deploy
{
  "policy_id": "uuid",
  "name": "admin-access",
  "description": "...",
  "rules": [...],
  "metadata": {
    "source": "sync-client",  // or "api", "file", "default"
    "server_version": "1.2.3",
    "deployed_by": "team:engineering",
    "deployed_at": "2025-01-15T10:30:00Z"
  }
}
```

**Configuration File:** `/etc/reaper/agent.yaml`
```yaml
agent:
  port: 8080
  bind_address: "0.0.0.0"

policies:
  # Load policies from these sources on startup
  bootstrap_dir: "/etc/reaper/policies"
  cache_dir: "/var/cache/reaper/policies"

data:
  # Load entity data on startup
  bootstrap_file: "/etc/reaper/data/entities.json"
  cache_dir: "/var/cache/reaper/data"

performance:
  target_latency_microseconds: 1.0
  enable_metrics: true
```

---

### 3. Reaper Sync Client (New Component)

**Status:** 🔨 To be built
**Location:** `services/reaper-sync/` (new)

**Purpose:**
Enable integration with centralized management server while keeping Reaper Agent independent.

**Key Features:**

1. **Policy Synchronization**
   - Poll management server for updates
   - Download new/updated policies
   - Push to local agent via `/api/v1/policies/deploy`

2. **Data Synchronization**
   - Download entity data (users, roles, resources)
   - Push to local agent via `/api/v1/data/sync`

3. **Version Management**
   - Track deployed versions
   - Support rollback via server
   - Handle version conflicts

4. **Scope Filtering**
   - Only sync policies for configured teams/scopes
   - Support multi-tenant deployments

5. **Offline Operation**
   - Cache policies locally
   - Continue operating if server unavailable
   - Sync when connection restored

6. **Health Monitoring**
   - Report agent health to server
   - Send metrics and analytics
   - Alert on failures

**Configuration:** `/etc/reaper/sync.yaml`
```yaml
sync:
  # Management server configuration
  server:
    url: "https://reaper-mgmt.example.com"
    api_version: "v1"
    timeout_seconds: 30

  # Authentication
  auth:
    type: "api_token"  # or "mtls", "oauth2"
    token_file: "/etc/reaper/secrets/token"
    # For mTLS:
    # cert_file: "/etc/reaper/certs/client.crt"
    # key_file: "/etc/reaper/certs/client.key"
    # ca_file: "/etc/reaper/certs/ca.crt"

  # Scope - what policies to sync
  scope:
    teams: ["engineering", "platform"]
    environments: ["production"]
    regions: ["us-west-2"]

  # Sync behavior
  behavior:
    mode: "active"  # or "on-demand", "offline"
    poll_interval_seconds: 30
    batch_size: 100
    retry_max_attempts: 3
    retry_backoff_seconds: 5

  # Local agent connection
  agent:
    url: "http://localhost:8080"
    health_check_interval_seconds: 10

  # Caching for offline operation
  cache:
    directory: "/var/cache/reaper/sync"
    enable_offline_mode: true
    max_age_hours: 24

  # Metrics reporting
  metrics:
    enable: true
    report_interval_seconds: 60
```

**API Client Interface:**
```rust
// Pseudo-code for sync client
struct SyncClient {
    config: SyncConfig,
    http_client: HttpClient,
    agent_client: AgentClient,
    cache: PolicyCache,
}

impl SyncClient {
    // Poll server for policy updates
    async fn sync_policies(&self) -> Result<SyncResult> {
        // 1. Query server for policies matching scope
        let policies = self.fetch_policies_from_server().await?;

        // 2. Compare with cached versions
        let updates = self.identify_updates(&policies)?;

        // 3. Download new/updated policies
        for policy in updates {
            let policy_data = self.fetch_policy(&policy.id).await?;

            // 4. Push to local agent
            self.agent_client.deploy_policy(policy_data).await?;

            // 5. Update cache
            self.cache.store(&policy).await?;
        }

        Ok(SyncResult { ... })
    }

    // Sync entity data
    async fn sync_data(&self) -> Result<()> {
        let entities = self.fetch_entities_from_server().await?;
        self.agent_client.sync_data(entities).await?;
        Ok(())
    }

    // Report metrics back to server
    async fn report_metrics(&self) -> Result<()> {
        let metrics = self.agent_client.get_metrics().await?;
        self.send_metrics_to_server(metrics).await?;
        Ok(())
    }
}
```

**Deployment Modes:**

1. **Sidecar Pattern**
   ```
   Pod:
   ├── Reaper Agent (8080)
   └── Reaper Sync Client (background process)
   ```

2. **Daemon Pattern**
   ```
   Host:
   ├── Reaper Agent (systemd service)
   └── Reaper Sync (systemd service)
   ```

3. **Standalone Pattern** (no sync client)
   ```
   Host:
   └── Reaper Agent (load from local files)
   ```

---

### 4. Management Server (Future)

**Status:** 🔮 Future implementation
**Location:** `services/reaper-server/` (future)

**Core Capabilities:**

1. **Policy Repository**
   - Store policies with full version history
   - Support multiple environments (dev, staging, prod)
   - Policy templates and inheritance

2. **Team-Based Scoping**
   ```
   Policy Hierarchy:

   Organization
   ├── Team: Engineering
   │   ├── Policy: api-access (v1.2.3)
   │   └── Policy: database-access (v2.0.1)
   └── Team: Finance
       ├── Policy: pii-access (v1.0.0)
       └── Policy: audit-logging (v1.5.0)
   ```

3. **Deployment Orchestration**
   - Gradual rollout (canary, blue-green)
   - Deployment gates and approvals
   - Automatic rollback on errors

4. **Version Management**
   - Semantic versioning
   - Rollback to previous versions
   - Version comparison and diff

5. **Compliance & Audit**
   - Full audit trail of changes
   - Policy compliance reports
   - Access control (RBAC for policy management)

6. **Analytics**
   - Policy evaluation metrics
   - Decision analytics (allow/deny rates)
   - Performance monitoring

**API Endpoints:**
```
# Policy Management
GET    /api/v1/policies?team=engineering&env=prod
POST   /api/v1/policies
GET    /api/v1/policies/:id
PUT    /api/v1/policies/:id
DELETE /api/v1/policies/:id

# Version Management
GET    /api/v1/policies/:id/versions
GET    /api/v1/policies/:id/versions/:version
POST   /api/v1/policies/:id/rollback/:version

# Deployment
POST   /api/v1/policies/:id/deploy
GET    /api/v1/deployments
GET    /api/v1/deployments/:id

# Data Management
GET    /api/v1/data/entities
POST   /api/v1/data/entities
PUT    /api/v1/data/entities/:id

# Agent Management
GET    /api/v1/agents
GET    /api/v1/agents/:id
POST   /api/v1/agents/:id/health
GET    /api/v1/agents/:id/metrics

# Team & Scope Management
GET    /api/v1/teams
POST   /api/v1/teams
GET    /api/v1/teams/:id/policies
```

---

## Integration Patterns

### Pattern 1: Fully Integrated (with Management Server)

```
[Management Server]
        ↓ (poll/push)
[Sync Client]
        ↓ (local HTTP)
[Reaper Agent]
        ↓ (in-process)
[Policy Engine]
```

**Use Case:** Enterprise deployments with centralized control

**Benefits:**
- Centralized policy management
- Version control and rollback
- Team-based scoping
- Audit logging

---

### Pattern 2: Direct Integration (no Management Server)

```
[Your Application]
        ↓ (HTTP API)
[Reaper Agent]
        ↓ (in-process)
[Policy Engine]
```

**Use Case:** Standalone deployments, microservices

**Benefits:**
- Simple deployment
- No external dependencies
- Direct control
- Load from local files

---

### Pattern 3: Embedded (no services)

```
[Your Application]
        ↓ (direct library usage)
[Policy Engine]
```

**Use Case:** Edge devices, serverless, embedded systems

**Benefits:**
- Zero network overhead
- Minimal dependencies
- Maximum performance
- Smallest footprint

---

## Migration Path

### Phase 1: Current State (✅ Completed)
- Policy Engine as independent library
- Reaper Agent as standalone service
- Basic policy deployment

### Phase 2: Enhanced Agent (🔨 Next)
- Add data sync endpoint
- Add policy caching
- Add source tracking
- Configuration file support

### Phase 3: Sync Client (🔨 Future)
- Build sync client service
- Implement polling mechanism
- Add authentication support
- Offline mode and caching

### Phase 4: Management Server (🔮 Long-term)
- Build central management server
- Implement version management
- Add team-based scoping
- Deployment orchestration
- Analytics and reporting

---

## Security Considerations

### Agent Security
- **No authentication required** - Runs on localhost
- **Firewall** - Only accessible from localhost by default
- **File permissions** - Secure config and cache directories

### Sync Client Security
- **Mutual TLS** - Server and client authentication
- **API tokens** - Short-lived, rotatable
- **Secure storage** - Credentials in secure vault
- **Certificate validation** - Pin server certificates

### Management Server Security
- **Authentication** - OAuth2, API tokens, mTLS
- **Authorization** - RBAC for policy management
- **Audit logging** - All changes logged
- **Encryption** - TLS 1.3 for all communication
- **Rate limiting** - Prevent abuse

---

## Configuration Examples

### Standalone Deployment (No Sync)

`/etc/reaper/agent.yaml`:
```yaml
agent:
  port: 8080

policies:
  bootstrap_dir: "/etc/reaper/policies"

data:
  bootstrap_file: "/etc/reaper/data/entities.json"
```

Start command:
```bash
reaper-agent --config /etc/reaper/agent.yaml
```

---

### Integrated Deployment (With Sync)

`/etc/reaper/agent.yaml`:
```yaml
agent:
  port: 8080

policies:
  cache_dir: "/var/cache/reaper/policies"

data:
  cache_dir: "/var/cache/reaper/data"
```

`/etc/reaper/sync.yaml`:
```yaml
sync:
  server:
    url: "https://reaper-mgmt.example.com"
  auth:
    token_file: "/etc/reaper/secrets/token"
  scope:
    teams: ["engineering"]
  behavior:
    poll_interval_seconds: 30
  agent:
    url: "http://localhost:8080"
  cache:
    directory: "/var/cache/reaper/sync"
    enable_offline_mode: true
```

Start commands:
```bash
# Start agent
systemctl start reaper-agent

# Start sync client
systemctl start reaper-sync
```

---

## Summary

### Key Architectural Decisions

1. **Reaper Agent is fully independent**
   - Can work without sync client
   - Can work without management server
   - Can load policies from files

2. **Sync Client is optional**
   - Only needed for centralized management
   - Can run in offline mode
   - Graceful degradation

3. **Clear separation of concerns**
   - Policy Engine = evaluation
   - Agent = service layer
   - Sync Client = integration layer
   - Management Server = orchestration layer

4. **Multiple integration patterns supported**
   - Fully integrated (all components)
   - Direct integration (agent only)
   - Embedded (library only)

### Benefits

- **Flexibility**: Deploy in multiple patterns
- **Independence**: No forced dependencies
- **Scalability**: Each component scales independently
- **Simplicity**: Start simple, add complexity as needed
- **Performance**: No network overhead for core evaluation

### Next Steps

1. Enhance Reaper Agent with data sync and caching
2. Build Reaper Sync Client
3. Define Management Server API spec
4. Implement Management Server
