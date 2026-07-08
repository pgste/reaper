//! Per-core sharded-UDS experiment: does N single-thread runtimes each owning
//! its own Unix socket beat one multi-threaded runtime on a single socket?
//!
//! Run as two processes so the load generator never steals the server's cores:
//!
//! ```bash
//! # Terminal 1 — SHARED model: one multi-thread runtime, one socket.
//! cargo run --release --example uds_shard -p reaper-agent -- \
//!     server --mode shared --dir /tmp/reaper-shard
//! # Terminal 2:
//! cargo run --release --example uds_shard -p reaper-agent -- \
//!     load --dir /tmp/reaper-shard --shards 1 --connections 32 --duration-secs 4
//!
//! # SHARDED model: N single-thread runtimes pinned to cores, N sockets.
//! cargo run --release --example uds_shard -p reaper-agent -- \
//!     server --mode sharded --shards 4 --dir /tmp/reaper-shard
//! cargo run --release --example uds_shard -p reaper-agent -- \
//!     load --dir /tmp/reaper-shard --shards 4 --connections 32 --duration-secs 4
//! ```
//!
//! Security: the socket directory is created owner-only (0700) and every socket
//! file is chmod'd 0600 — the same model the agent's `serve_uds` uses, applied
//! to each of the N mounts (UDS has no app-layer auth; filesystem perms are the
//! boundary, and more mounts = more boundaries to get right).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{routing::post, Router};
use bytes::Bytes;
use clap::{Parser, Subcommand};
use http_body_util::{BodyExt, Full};
use hyper::{client::conn::http1, Method, Request};
use hyper_util::rt::TokioIo;
use tokio::net::UnixStream;

use policy_engine::{
    cache_config::CacheConfig, DataLoader, DataStore, EnhancedPolicy, PolicyEngine, PolicyLanguage,
    ReaperPolicy,
};
use reaper_agent::handlers::fast_evaluate_policy;
use reaper_agent::state::{AgentState, AgentStats};
use reaper_core::config::ReaperAgentConfig;

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the agent server (shared or sharded).
    Server {
        #[arg(long, default_value = "shared")]
        mode: String,
        /// Number of shards (sharded mode). Defaults to the CPU count.
        #[arg(long)]
        shards: Option<usize>,
        #[arg(long, default_value = "/tmp/reaper-shard")]
        dir: PathBuf,
    },
    /// Run the load generator against the server's sockets.
    Load {
        #[arg(long, default_value = "/tmp/reaper-shard")]
        dir: PathBuf,
        /// Number of sockets to spread connections across (agent-0..N-1.sock).
        #[arg(long, default_value_t = 1)]
        shards: usize,
        #[arg(long, default_value_t = 32)]
        connections: usize,
        #[arg(long, default_value_t = 4)]
        duration_secs: u64,
        #[arg(long, default_value_t = 1)]
        warmup_secs: u64,
    },
}

const POLICY: &str = r#"
policy bench {
    default: deny,
    rule deny_suspended { deny if user.suspended == true }
    rule admin_full { allow if user.role == "admin" }
    rule dept_clearance {
        allow if {
            user.department == resource.department &&
            user.clearance_level >= resource.clearance_level &&
            user.status == "active"
        }
    }
}
"#;

const DATA: &str = r#"{"entities":[
    {"id":"alice","type":"User","attributes":{"role":"engineer","department":"engineering","clearance_level":4,"status":"active"}},
    {"id":"doc1","type":"Resource","attributes":{"department":"engineering","clearance_level":3}}
]}"#;

fn socket_path(dir: &Path, i: usize) -> PathBuf {
    dir.join(format!("agent-{i}.sock"))
}

/// Create the socket directory owner-only (0700) — the UDS security boundary.
fn create_private_dir(dir: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(dir)?;
    Ok(())
}

/// Chmod a socket to owner-only (0600).
fn lock_down_socket(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
}

fn build_state() -> anyhow::Result<Arc<AgentState>> {
    let data_store = Arc::new(DataStore::new());
    DataLoader::new((*data_store).clone()).load_json(DATA)?;

    let engine = PolicyEngine::new();
    let reaper_policy: ReaperPolicy = POLICY.parse().map_err(|e| anyhow::anyhow!("{e}"))?;
    let evaluator = reaper_policy
        .build(data_store.clone())
        .map_err(|e| anyhow::anyhow!("compile: {e}"))?;
    let mut policy = EnhancedPolicy::new("bench".to_string(), String::new(), vec![]);
    policy.language = PolicyLanguage::ReaperDsl;
    policy.content = POLICY.to_string();
    policy.evaluator = Some(Arc::new(evaluator));
    engine
        .deploy_policy(policy)
        .map_err(|e| anyhow::anyhow!("deploy: {e}"))?;

    Ok(Arc::new(AgentState {
        policy_engine: engine,
        data_store,
        stats: Arc::new(AgentStats::new(false)),
        decision_cache: None,
        cache_config: CacheConfig::disabled(),
        agent_config: ReaperAgentConfig::default(),
        policy_cache: None,
        decision_buffer: None,
        agent_id: "shard-bench".to_string(),
        decision_metrics: Arc::new(reaper_agent::metrics_cache::DecisionMetrics::new()),
        data_sync: std::sync::Arc::new(reaper_agent::state::DataSyncState::from_env()),
        bundle_verifier: std::sync::Arc::new(
            reaper_agent::management::verify::BundleVerifier::from_config(
                &reaper_core::config::ManagementSettings::default(),
            ),
        ),
    }))
}

fn router(state: Arc<AgentState>) -> Router {
    Router::new()
        .route("/api/v1/fast-messages", post(fast_evaluate_policy))
        .with_state(state)
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Server { mode, shards, dir } => run_server(&mode, shards, &dir),
        Cmd::Load {
            dir,
            shards,
            connections,
            duration_secs,
            warmup_secs,
        } => run_load(&dir, shards, connections, duration_secs, warmup_secs),
    }
}

fn run_server(mode: &str, shards: Option<usize>, dir: &Path) -> anyhow::Result<()> {
    create_private_dir(dir)?;
    let state = build_state()?;

    match mode {
        "shared" => {
            // One socket, one multi-threaded runtime (uses all cores).
            let path = socket_path(dir, 0);
            let _ = std::fs::remove_file(&path);
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            println!(
                "SHARED server ready on {} (multi-thread runtime)",
                path.display()
            );
            rt.block_on(async move {
                let listener = tokio::net::UnixListener::bind(&path).unwrap();
                lock_down_socket(&path);
                axum::serve(listener, router(state)).await.unwrap();
            });
        }
        "sharded" => {
            let cores = core_affinity::get_core_ids().unwrap_or_default();
            let n = shards.unwrap_or_else(|| cores.len().max(1));
            println!("SHARDED server: {n} shards, one single-thread runtime + socket per shard");

            let mut handles = Vec::new();
            for i in 0..n {
                let state = state.clone();
                let path = socket_path(dir, i);
                let _ = std::fs::remove_file(&path);
                let core = cores.get(i % cores.len().max(1)).copied();
                handles.push(std::thread::spawn(move || {
                    // Pin this shard's thread to a core (share-nothing).
                    if let Some(core) = core {
                        core_affinity::set_for_current(core);
                    }
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .unwrap();
                    rt.block_on(async move {
                        let listener = tokio::net::UnixListener::bind(&path).unwrap();
                        lock_down_socket(&path);
                        println!("  shard {i} ready on {}", path.display());
                        axum::serve(listener, router(state)).await.unwrap();
                    });
                }));
            }
            for h in handles {
                let _ = h.join();
            }
        }
        other => anyhow::bail!("unknown mode '{other}' (use shared|sharded)"),
    }
    Ok(())
}

fn run_load(
    dir: &Path,
    shards: usize,
    connections: usize,
    duration_secs: u64,
    warmup_secs: u64,
) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move {
        let sockets: Vec<PathBuf> = (0..shards.max(1)).map(|i| socket_path(dir, i)).collect();
        let body = Bytes::from(
            r#"{"principal":"alice","action":"read","resource":"doc1","policy_name":"bench"}"#,
        );

        // warmup (discarded)
        let _ = drive(
            &sockets,
            &body,
            connections,
            Duration::from_secs(warmup_secs),
        )
        .await;
        let stats = drive(
            &sockets,
            &body,
            connections,
            Duration::from_secs(duration_secs),
        )
        .await;

        println!(
            "sockets={} connections={} -> {:.0} req/s   p50={:.1}µs p95={:.1}µs p99={:.1}µs",
            sockets.len(),
            connections,
            stats.rps(),
            stats.pct(0.50),
            stats.pct(0.95),
            stats.pct(0.99),
        );
    });
    Ok(())
}

struct Stats {
    wall: Duration,
    lat: Vec<u64>,
}
impl Stats {
    fn rps(&self) -> f64 {
        self.lat.len() as f64 / self.wall.as_secs_f64()
    }
    fn pct(&self, p: f64) -> f64 {
        if self.lat.is_empty() {
            return 0.0;
        }
        let i = ((self.lat.len() as f64 * p) as usize).min(self.lat.len() - 1);
        self.lat[i] as f64 / 1000.0
    }
}

/// Spread `connections` closed-loop workers across `sockets` (round-robin).
async fn drive(sockets: &[PathBuf], body: &Bytes, connections: usize, dur: Duration) -> Stats {
    let deadline = Instant::now() + dur;
    let mut handles = Vec::new();
    for c in 0..connections {
        let sock = sockets[c % sockets.len()].clone();
        let body = body.clone();
        handles.push(tokio::spawn(async move {
            match UnixStream::connect(&sock).await {
                Ok(stream) => worker(stream, body, deadline).await,
                Err(_) => Vec::new(),
            }
        }));
    }
    let start = Instant::now();
    let mut lat = Vec::new();
    for h in handles {
        if let Ok(v) = h.await {
            lat.extend(v);
        }
    }
    let wall = start.elapsed();
    lat.sort_unstable();
    Stats { wall, lat }
}

async fn worker(stream: UnixStream, body: Bytes, deadline: Instant) -> Vec<u64> {
    let io = TokioIo::new(stream);
    let (mut sender, conn) = match http1::handshake(io).await {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let mut lat = Vec::new();
    while Instant::now() < deadline {
        let req = Request::builder()
            .method(Method::POST)
            .uri("/api/v1/fast-messages")
            .header("host", "localhost")
            .header("content-type", "application/json")
            .body(Full::new(body.clone()))
            .unwrap();
        let start = Instant::now();
        match sender.send_request(req).await {
            Ok(resp) => {
                if resp.into_body().collect().await.is_err() {
                    break;
                }
                lat.push(start.elapsed().as_nanos() as u64);
            }
            Err(_) => break,
        }
    }
    lat
}
