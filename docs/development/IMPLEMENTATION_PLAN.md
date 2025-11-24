# Reaper Client Separation - Implementation Plan

## Overview

This document provides the step-by-step implementation plan for separating Reaper and the Web Client.

---

## Phase 1: Agent Enhancements (Week 1-2)

### Goal
Make Reaper Agent fully independent with support for multiple policy sources and data synchronization.

### Tasks

#### 1.1 Add Configuration File Support

**Create:** `crates/reaper-core/src/config.rs`

```rust
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentConfig {
    pub agent: AgentSettings,
    pub policies: PolicySettings,
    pub data: DataSettings,
    pub performance: PerformanceSettings,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentSettings {
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_bind_address")]
    pub bind_address: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PolicySettings {
    pub bootstrap_dir: Option<PathBuf>,
    pub cache_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DataSettings {
    pub bootstrap_file: Option<PathBuf>,
    pub cache_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PerformanceSettings {
    #[serde(default = "default_target_latency")]
    pub target_latency_microseconds: f64,
    #[serde(default = "default_enable_metrics")]
    pub enable_metrics: bool,
}

fn default_port() -> u16 { 8080 }
fn default_bind_address() -> String { "0.0.0.0".to_string() }
fn default_target_latency() -> f64 { 1.0 }
fn default_enable_metrics() -> bool { true }

impl AgentConfig {
    pub fn from_file(path: &PathBuf) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: AgentConfig = serde_yaml::from_str(&content)?;
        Ok(config)
    }

    pub fn default() -> Self {
        Self {
            agent: AgentSettings {
                port: 8080,
                bind_address: "0.0.0.0".to_string(),
            },
            policies: PolicySettings {
                bootstrap_dir: None,
                cache_dir: None,
            },
            data: DataSettings {
                bootstrap_file: None,
                cache_dir: None,
            },
            performance: PerformanceSettings {
                target_latency_microseconds: 1.0,
                enable_metrics: true,
            },
        }
    }
}
```

**Update:** `services/reaper-agent/Cargo.toml`
```toml
[dependencies]
# ... existing dependencies ...
clap = { version = "4.0", features = ["derive"] }
serde_yaml = "0.9"
```

**Update:** `services/reaper-agent/src/main.rs`
```rust
use clap::Parser;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Path to configuration file
    #[arg(short, long, default_value = "/etc/reaper/agent.yaml")]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let args = Args::parse();

    // Load configuration
    let config = if args.config.exists() {
        info!("Loading configuration from {:?}", args.config);
        AgentConfig::from_file(&args.config)?
    } else {
        warn!("Config file not found, using defaults");
        AgentConfig::default()
    };

    // ... rest of initialization ...
}
```

---

#### 1.2 Add Policy Source Tracking

**Update:** `crates/policy-engine/src/policy.rs`

```rust
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PolicySource {
    /// Loaded from local file on startup
    File { path: String },
    /// Deployed via direct API call
    Api { client_id: Option<String> },
    /// Synchronized from management server
    SyncClient {
        server_url: String,
        version: String,
        team: Option<String>,
    },
    /// Default policy created by system
    Default,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyMetadata {
    pub source: PolicySource,
    pub deployed_at: DateTime<Utc>,
    pub deployed_by: Option<String>,
    pub server_version: Option<String>,
    pub checksum: Option<String>,
}

// Add to EnhancedPolicy
pub struct EnhancedPolicy {
    // ... existing fields ...
    pub metadata: Option<PolicyMetadata>,
}
```

**Update:** `services/reaper-agent/src/main.rs` deployment endpoint

```rust
#[derive(Debug, Deserialize)]
struct DeployPolicyRequest {
    pub policy_id: String,
    pub name: String,
    pub description: String,
    pub rules: Vec<DeployPolicyRule>,
    pub metadata: Option<DeployMetadata>,  // NEW
}

#[derive(Debug, Deserialize)]
struct DeployMetadata {
    pub source: String,  // "sync-client", "api", "file"
    pub server_version: Option<String>,
    pub deployed_by: Option<String>,
}
```

---

#### 1.3 Add Data Synchronization Endpoint

**Create:** `services/reaper-agent/src/handlers/data.rs`

```rust
use axum::{extract::State, http::StatusCode, response::Json};
use policy_engine::data::{DataLoader, Entity};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::{info, instrument};

use crate::AgentState;

#[derive(Debug, Deserialize)]
pub struct SyncDataRequest {
    pub entities: Vec<EntityData>,
    pub replace_all: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct EntityData {
    pub id: String,
    pub entity_type: String,
    pub attributes: serde_json::Map<String, serde_json::Value>,
    pub parent: Option<String>,
}

#[instrument(skip(state, payload))]
pub async fn sync_data(
    State(state): State<Arc<AgentState>>,
    Json(payload): Json<SyncDataRequest>,
) -> Result<Json<Value>, StatusCode> {
    let replace_all = payload.replace_all.unwrap_or(false);

    if replace_all {
        // Clear existing data
        // Note: This requires adding a clear() method to DataStore
        info!("Clearing existing entity data");
        // state.data_store.clear();
    }

    // Convert and insert entities
    let mut inserted = 0;
    let mut failed = 0;

    for entity_data in payload.entities {
        // Convert JSON attributes to AttributeValue
        // This requires implementing conversion logic
        match convert_entity(entity_data) {
            Ok(entity) => {
                // state.data_store.insert(entity)?;
                inserted += 1;
            }
            Err(e) => {
                failed += 1;
                tracing::warn!("Failed to insert entity: {}", e);
            }
        }
    }

    Ok(Json(json!({
        "status": "success",
        "inserted": inserted,
        "failed": failed,
        "replace_all": replace_all
    })))
}

fn convert_entity(data: EntityData) -> anyhow::Result<Entity> {
    // TODO: Implement conversion from JSON to Entity
    // This will need to handle the AttributeValue enum
    unimplemented!("Entity conversion")
}
```

**Update:** `services/reaper-agent/src/main.rs`

```rust
mod handlers;

use handlers::data::sync_data;

// In the router setup:
let app = Router::new()
    // ... existing routes ...
    .route("/api/v1/data/sync", post(sync_data))
    .with_state(state);
```

---

#### 1.4 Add Policy Caching

**Create:** `services/reaper-agent/src/cache.rs`

```rust
use policy_engine::EnhancedPolicy;
use std::path::PathBuf;
use anyhow::Result;

pub struct PolicyCache {
    cache_dir: PathBuf,
}

impl PolicyCache {
    pub fn new(cache_dir: PathBuf) -> Self {
        // Ensure directory exists
        std::fs::create_dir_all(&cache_dir).ok();
        Self { cache_dir }
    }

    pub async fn save_policy(&self, policy: &EnhancedPolicy) -> Result<()> {
        let filename = format!("{}.json", policy.id);
        let path = self.cache_dir.join(filename);

        let json = serde_json::to_string_pretty(policy)?;
        tokio::fs::write(path, json).await?;

        Ok(())
    }

    pub async fn load_policies(&self) -> Result<Vec<EnhancedPolicy>> {
        let mut policies = Vec::new();

        let mut entries = tokio::fs::read_dir(&self.cache_dir).await?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                let content = tokio::fs::read_to_string(&path).await?;
                match serde_json::from_str::<EnhancedPolicy>(&content) {
                    Ok(policy) => policies.push(policy),
                    Err(e) => {
                        tracing::warn!("Failed to load cached policy from {:?}: {}", path, e);
                    }
                }
            }
        }

        Ok(policies)
    }

    pub async fn delete_policy(&self, policy_id: &uuid::Uuid) -> Result<()> {
        let filename = format!("{}.json", policy_id);
        let path = self.cache_dir.join(filename);

        if path.exists() {
            tokio::fs::remove_file(path).await?;
        }

        Ok(())
    }
}
```

---

#### 1.5 Add Bootstrap Loading

**Create:** `services/reaper-agent/src/bootstrap.rs`

```rust
use policy_engine::{EnhancedPolicy, PolicyEngine, ReaperPolicy};
use std::path::PathBuf;
use anyhow::Result;
use tracing::{info, warn};

pub async fn load_bootstrap_policies(
    engine: &PolicyEngine,
    bootstrap_dir: Option<PathBuf>,
) -> Result<usize> {
    let Some(dir) = bootstrap_dir else {
        return Ok(0);
    };

    if !dir.exists() {
        warn!("Bootstrap directory does not exist: {:?}", dir);
        return Ok(0);
    }

    info!("Loading bootstrap policies from {:?}", dir);

    let mut count = 0;
    let mut entries = tokio::fs::read_dir(dir).await?;

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();

        // Check file extension
        let ext = path.extension().and_then(|s| s.to_str());
        match ext {
            Some("yaml") | Some("yml") | Some("json") | Some("reap") => {
                match ReaperPolicy::from_file_auto(&path) {
                    Ok(reaper_policy) => {
                        let enhanced: EnhancedPolicy = reaper_policy.into();
                        engine.deploy_policy(enhanced)?;
                        count += 1;
                        info!("Loaded bootstrap policy from {:?}", path);
                    }
                    Err(e) => {
                        warn!("Failed to load policy from {:?}: {}", path, e);
                    }
                }
            }
            _ => {
                // Skip non-policy files
            }
        }
    }

    Ok(count)
}
```

---

## Phase 2: Sync Client Implementation (Week 3-4)

### Goal
Build the Reaper Sync Client that polls the management server and pushes updates to the agent.

### Tasks

#### 2.1 Create Sync Client Crate

**Create:** `services/reaper-sync/Cargo.toml`

```toml
[package]
name = "reaper-sync"
version = "0.1.0"
edition = "2021"

[dependencies]
reaper-core = { path = "../../crates/reaper-core" }
policy-engine = { path = "../../crates/policy-engine" }

tokio = { version = "1.0", features = ["full"] }
anyhow = "1.0"
tracing = "0.1"
tracing-subscriber = "0.3"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
serde_yaml = "0.9"
reqwest = { version = "0.11", features = ["json"] }
clap = { version = "4.0", features = ["derive"] }
```

---

#### 2.2 Sync Client Configuration

**Create:** `services/reaper-sync/src/config.rs`

```rust
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SyncConfig {
    pub sync: SyncSettings,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SyncSettings {
    pub server: ServerConfig,
    pub auth: AuthConfig,
    pub scope: ScopeConfig,
    pub behavior: BehaviorConfig,
    pub agent: AgentConfig,
    pub cache: CacheConfig,
    pub metrics: MetricsConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServerConfig {
    pub url: String,
    #[serde(default = "default_api_version")]
    pub api_version: String,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AuthConfig {
    #[serde(rename = "type")]
    pub auth_type: String,  // "api_token", "mtls", "oauth2"
    pub token_file: Option<PathBuf>,
    pub cert_file: Option<PathBuf>,
    pub key_file: Option<PathBuf>,
    pub ca_file: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ScopeConfig {
    pub teams: Vec<String>,
    #[serde(default)]
    pub environments: Vec<String>,
    #[serde(default)]
    pub regions: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BehaviorConfig {
    #[serde(default = "default_mode")]
    pub mode: String,  // "active", "on-demand", "offline"
    #[serde(default = "default_poll_interval")]
    pub poll_interval_seconds: u64,
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
    #[serde(default = "default_retry_attempts")]
    pub retry_max_attempts: u32,
    #[serde(default = "default_retry_backoff")]
    pub retry_backoff_seconds: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentConfig {
    pub url: String,
    #[serde(default = "default_health_check_interval")]
    pub health_check_interval_seconds: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CacheConfig {
    pub directory: PathBuf,
    #[serde(default = "default_offline_mode")]
    pub enable_offline_mode: bool,
    #[serde(default = "default_max_age")]
    pub max_age_hours: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MetricsConfig {
    #[serde(default = "default_enable")]
    pub enable: bool,
    #[serde(default = "default_report_interval")]
    pub report_interval_seconds: u64,
}

// Defaults
fn default_api_version() -> String { "v1".to_string() }
fn default_timeout() -> u64 { 30 }
fn default_mode() -> String { "active".to_string() }
fn default_poll_interval() -> u64 { 30 }
fn default_batch_size() -> usize { 100 }
fn default_retry_attempts() -> u32 { 3 }
fn default_retry_backoff() -> u64 { 5 }
fn default_health_check_interval() -> u64 { 10 }
fn default_offline_mode() -> bool { true }
fn default_max_age() -> u64 { 24 }
fn default_enable() -> bool { true }
fn default_report_interval() -> u64 { 60 }

impl SyncConfig {
    pub fn from_file(path: &PathBuf) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: SyncConfig = serde_yaml::from_str(&content)?;
        Ok(config)
    }
}
```

---

#### 2.3 Management Server Client

**Create:** `services/reaper-sync/src/server_client.rs`

```rust
use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::config::SyncConfig;

#[derive(Debug, Clone, Deserialize)]
pub struct PolicyListResponse {
    pub policies: Vec<PolicySummary>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PolicySummary {
    pub id: String,
    pub name: String,
    pub version: String,
    pub checksum: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PolicyDetail {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub rules: Vec<serde_json::Value>,
}

pub struct ServerClient {
    config: SyncConfig,
    http_client: Client,
}

impl ServerClient {
    pub fn new(config: SyncConfig) -> Result<Self> {
        let http_client = Client::builder()
            .timeout(std::time::Duration::from_secs(config.sync.server.timeout_seconds))
            .build()?;

        Ok(Self {
            config,
            http_client,
        })
    }

    pub async fn list_policies(&self) -> Result<PolicyListResponse> {
        let url = format!(
            "{}/api/{}/policies",
            self.config.sync.server.url,
            self.config.sync.server.api_version
        );

        let response = self.http_client
            .get(&url)
            .query(&[
                ("teams", self.config.sync.scope.teams.join(",")),
                ("environments", self.config.sync.scope.environments.join(",")),
            ])
            .send()
            .await?;

        let policies: PolicyListResponse = response.json().await?;
        Ok(policies)
    }

    pub async fn get_policy(&self, policy_id: &str) -> Result<PolicyDetail> {
        let url = format!(
            "{}/api/{}/policies/{}",
            self.config.sync.server.url,
            self.config.sync.server.api_version,
            policy_id
        );

        let response = self.http_client
            .get(&url)
            .send()
            .await?;

        let policy: PolicyDetail = response.json().await?;
        Ok(policy)
    }
}
```

---

#### 2.4 Agent Client

**Create:** `services/reaper-sync/src/agent_client.rs`

```rust
use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::config::SyncConfig;

#[derive(Debug, Serialize)]
pub struct DeployPolicyRequest {
    pub policy_id: String,
    pub name: String,
    pub description: String,
    pub rules: Vec<serde_json::Value>,
    pub metadata: DeployMetadata,
}

#[derive(Debug, Serialize)]
pub struct DeployMetadata {
    pub source: String,
    pub server_version: String,
    pub deployed_by: String,
}

pub struct AgentClient {
    agent_url: String,
    http_client: Client,
}

impl AgentClient {
    pub fn new(config: &SyncConfig) -> Result<Self> {
        let http_client = Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()?;

        Ok(Self {
            agent_url: config.sync.agent.url.clone(),
            http_client,
        })
    }

    pub async fn deploy_policy(&self, request: DeployPolicyRequest) -> Result<()> {
        let url = format!("{}/api/v1/policies/deploy", self.agent_url);

        self.http_client
            .post(&url)
            .json(&request)
            .send()
            .await?
            .error_for_status()?;

        Ok(())
    }

    pub async fn health_check(&self) -> Result<bool> {
        let url = format!("{}/health", self.agent_url);

        let response = self.http_client
            .get(&url)
            .send()
            .await?;

        Ok(response.status().is_success())
    }
}
```

---

#### 2.5 Sync Engine

**Create:** `services/reaper-sync/src/sync_engine.rs`

```rust
use anyhow::Result;
use std::collections::HashMap;
use tracing::{info, warn};

use crate::{
    agent_client::{AgentClient, DeployPolicyRequest, DeployMetadata},
    config::SyncConfig,
    server_client::{PolicySummary, ServerClient},
};

pub struct SyncEngine {
    config: SyncConfig,
    server_client: ServerClient,
    agent_client: AgentClient,
    last_synced: HashMap<String, String>, // policy_id -> checksum
}

impl SyncEngine {
    pub fn new(config: SyncConfig) -> Result<Self> {
        let server_client = ServerClient::new(config.clone())?;
        let agent_client = AgentClient::new(&config)?;

        Ok(Self {
            config,
            server_client,
            agent_client,
            last_synced: HashMap::new(),
        })
    }

    pub async fn sync_once(&mut self) -> Result<SyncResult> {
        info!("Starting policy synchronization");

        // 1. Fetch policy list from server
        let policies = self.server_client.list_policies().await?;

        let mut deployed = 0;
        let mut skipped = 0;
        let mut failed = 0;

        // 2. Check each policy
        for policy_summary in policies.policies {
            // Check if we need to update
            if let Some(last_checksum) = self.last_synced.get(&policy_summary.id) {
                if last_checksum == &policy_summary.checksum {
                    skipped += 1;
                    continue; // No changes
                }
            }

            // 3. Fetch full policy details
            match self.server_client.get_policy(&policy_summary.id).await {
                Ok(policy_detail) => {
                    // 4. Deploy to agent
                    let deploy_request = DeployPolicyRequest {
                        policy_id: policy_detail.id.clone(),
                        name: policy_detail.name.clone(),
                        description: policy_detail.description.clone(),
                        rules: policy_detail.rules,
                        metadata: DeployMetadata {
                            source: "sync-client".to_string(),
                            server_version: policy_detail.version.clone(),
                            deployed_by: format!("sync-client:{}", self.config.sync.scope.teams.join(",")),
                        },
                    };

                    match self.agent_client.deploy_policy(deploy_request).await {
                        Ok(_) => {
                            info!("Deployed policy: {} ({})", policy_summary.name, policy_summary.id);
                            self.last_synced.insert(policy_summary.id.clone(), policy_summary.checksum);
                            deployed += 1;
                        }
                        Err(e) => {
                            warn!("Failed to deploy policy {}: {}", policy_summary.id, e);
                            failed += 1;
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to fetch policy {}: {}", policy_summary.id, e);
                    failed += 1;
                }
            }
        }

        Ok(SyncResult {
            deployed,
            skipped,
            failed,
        })
    }

    pub async fn run_continuous(&mut self) -> Result<()> {
        let poll_interval = std::time::Duration::from_secs(
            self.config.sync.behavior.poll_interval_seconds
        );

        loop {
            match self.sync_once().await {
                Ok(result) => {
                    info!(
                        "Sync complete: deployed={}, skipped={}, failed={}",
                        result.deployed, result.skipped, result.failed
                    );
                }
                Err(e) => {
                    warn!("Sync failed: {}", e);
                }
            }

            tokio::time::sleep(poll_interval).await;
        }
    }
}

#[derive(Debug)]
pub struct SyncResult {
    pub deployed: usize,
    pub skipped: usize,
    pub failed: usize,
}
```

---

#### 2.6 Main Entry Point

**Create:** `services/reaper-sync/src/main.rs`

```rust
mod agent_client;
mod config;
mod server_client;
mod sync_engine;

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use tracing::info;

use config::SyncConfig;
use sync_engine::SyncEngine;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Path to configuration file
    #[arg(short, long, default_value = "/etc/reaper/sync.yaml")]
    config: PathBuf,

    /// Run once and exit (don't run continuously)
    #[arg(long)]
    once: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let args = Args::parse();

    // Load configuration
    let config = SyncConfig::from_file(&args.config)?;

    info!("Reaper Sync Client starting");
    info!("Server: {}", config.sync.server.url);
    info!("Teams: {:?}", config.sync.scope.teams);
    info!("Agent: {}", config.sync.agent.url);

    let mut engine = SyncEngine::new(config)?;

    if args.once {
        // Run once and exit
        let result = engine.sync_once().await?;
        info!(
            "Sync complete: deployed={}, skipped={}, failed={}",
            result.deployed, result.skipped, result.failed
        );
    } else {
        // Run continuously
        engine.run_continuous().await?;
    }

    Ok(())
}
```

---

## Phase 3: Testing & Documentation (Week 5)

### Tasks

#### 3.1 Integration Tests

Create integration tests that demonstrate all three deployment patterns:

1. **Standalone Test** - Agent only, no sync
2. **Direct Integration Test** - Application → Agent
3. **Full Integration Test** - Sync Client → Agent

#### 3.2 Example Configurations

Create example config files for common scenarios:
- `examples/configs/standalone-agent.yaml`
- `examples/configs/integrated-sync.yaml`
- `examples/configs/production.yaml`

#### 3.3 Documentation

- Update README.md with deployment patterns
- Create deployment guide
- Create troubleshooting guide

---

## Phase 4: Management Server (Future)

This is a longer-term project that would include:

1. **Database Layer** - PostgreSQL for policy storage
2. **API Server** - Full REST API for policy management
3. **Web UI** - Admin interface for policy management
4. **Team Management** - RBAC for policy access
5. **Versioning** - Full version control with rollback
6. **Analytics** - Metrics and reporting

---

## Success Criteria

### Phase 1 Complete When:
- [ ] Agent can load policies from config directory
- [ ] Agent supports multiple policy sources
- [ ] Agent has data sync endpoint
- [ ] Agent caches policies to disk
- [ ] All existing tests pass

### Phase 2 Complete When:
- [ ] Sync client can connect to mock server
- [ ] Sync client can deploy policies to agent
- [ ] Sync client runs continuously
- [ ] Sync client handles failures gracefully
- [ ] Integration test passes

### Phase 3 Complete When:
- [ ] All deployment patterns tested
- [ ] Documentation complete
- [ ] Example configs provided
- [ ] Migration guide written

---

## Timeline

- **Week 1-2**: Phase 1 (Agent Enhancements)
- **Week 3-4**: Phase 2 (Sync Client)
- **Week 5**: Phase 3 (Testing & Docs)
- **Future**: Phase 4 (Management Server)

**Total estimated time:** 5 weeks for Phases 1-3
