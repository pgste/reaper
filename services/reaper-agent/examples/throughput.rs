//! End-to-end throughput & latency harness: HTTP (TCP) vs UDS, compiled vs AST.
//!
//! This spins up the *real* agent router (the same `evaluate_policy` /
//! `fast_evaluate_policy` handlers used in production) in-process, serves it
//! simultaneously over a loopback TCP socket and a Unix domain socket, deploys
//! the same policy twice — once as the compiled Reaper-DSL evaluator and once as
//! the AST interpreter — and drives closed-loop load over both transports using
//! one shared hyper HTTP/1.1 client stack (keep-alive). Because only the
//! transport differs, the TCP-vs-UDS numbers isolate the round-trip cost of the
//! network stack, and the compiled-vs-AST numbers isolate the evaluator.
//!
//! # Run
//! ```bash
//! # Build with the release profile (LTO + mimalloc) for representative numbers:
//! cargo run --release --example throughput -p reaper-agent
//!
//! # Tune the load:
//! cargo run --release --example throughput -p reaper-agent -- \
//!     --connections 16 --duration-secs 5 --warmup-secs 1
//! ```
//!
//! The matrix run is: {TCP, UDS} x {fast endpoint, standard endpoint} x
//! {compiled, AST}. `fast` is the sonic-rs SIMD path; `standard` is the
//! serde_json path — the difference shows JSON-parsing overhead on the request
//! path, which usually dwarfs the sub-microsecond evaluation itself.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{routing::post, Router};
use bytes::Bytes;
use clap::Parser;
use http_body_util::{BodyExt, Full};
use hyper::{client::conn::http1, Method, Request};
use hyper_util::rt::TokioIo;
use tokio::net::{TcpListener, TcpStream, UnixListener, UnixStream};

use policy_engine::{
    cache_config::CacheConfig, DataLoader, DataStore, EnhancedPolicy, PolicyEngine, PolicyLanguage,
    ReaperPolicy,
};
use reaper_agent::handlers::{evaluate_policy, fast_evaluate_policy};
use reaper_agent::state::{AgentState, AgentStats};
use reaper_core::config::ReaperAgentConfig;

#[derive(Parser, Debug)]
#[command(about = "Reaper agent throughput: HTTP vs UDS, compiled vs AST")]
struct Args {
    /// Concurrent keep-alive connections (closed-loop workers).
    #[arg(long, default_value_t = 8)]
    connections: usize,
    /// Measurement duration per configuration, in seconds.
    #[arg(long, default_value_t = 3)]
    duration_secs: u64,
    /// Warmup duration per configuration, in seconds (results discarded).
    #[arg(long, default_value_t = 1)]
    warmup_secs: u64,
}

/// A moderately complex ABAC policy so the compiled-vs-AST gap is meaningful.
const POLICY: &str = r#"
policy bench {
    default: deny,

    rule deny_suspended {
        deny if user.suspended == true
    }
    rule admin_full {
        allow if user.role == "admin"
    }
    rule dept_clearance {
        allow if {
            user.department == resource.department &&
            user.clearance_level >= resource.clearance_level &&
            user.status == "active"
        }
    }
    rule public_read {
        allow if {
            resource.classification == "public" &&
            user.status == "active" &&
            action == "read"
        }
    }
}
"#;

/// Entities: an engineer and a same-department resource (dept_clearance allows).
const DATA: &str = r#"{
  "entities": [
    {"id": "alice", "type": "User", "attributes": {
        "role": "engineer", "department": "engineering",
        "clearance_level": 4, "status": "active"}},
    {"id": "doc1", "type": "Resource", "attributes": {
        "department": "engineering", "clearance_level": 3,
        "classification": "internal"}}
  ]
}"#;

const COMPILED_POLICY_NAME: &str = "bench_compiled";
const AST_POLICY_NAME: &str = "bench_ast";

/// A single benchmarked configuration.
struct Config {
    label: &'static str,
    transport: Transport,
    path: &'static str,
    policy_name: &'static str,
}

#[derive(Clone)]
enum Transport {
    Tcp(std::net::SocketAddr),
    Uds(PathBuf),
}

struct Stats {
    count: u64,
    wall: Duration,
    /// Sorted per-request latencies, in nanoseconds.
    latencies_ns: Vec<u64>,
}

impl Stats {
    fn rps(&self) -> f64 {
        self.count as f64 / self.wall.as_secs_f64()
    }
    fn pct(&self, p: f64) -> f64 {
        if self.latencies_ns.is_empty() {
            return 0.0;
        }
        let idx = ((self.latencies_ns.len() as f64 * p) as usize).min(self.latencies_ns.len() - 1);
        self.latencies_ns[idx] as f64 / 1000.0 // µs
    }
    fn max_us(&self) -> f64 {
        self.latencies_ns.last().copied().unwrap_or(0) as f64 / 1000.0
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // ---- Build the real agent state and deploy both evaluators ----
    let data_store = Arc::new(DataStore::new());
    DataLoader::new((*data_store).clone()).load_json(DATA)?;

    let engine = PolicyEngine::new();
    deploy(&engine, &data_store, COMPILED_POLICY_NAME, EvaluatorKind::Compiled)?;
    deploy(&engine, &data_store, AST_POLICY_NAME, EvaluatorKind::Ast)?;

    let state = Arc::new(AgentState {
        policy_engine: engine,
        data_store,
        stats: Arc::new(AgentStats::new(false)),
        // Caching OFF so we measure evaluation, not cache hits.
        decision_cache: None,
        cache_config: CacheConfig::disabled(),
        agent_config: ReaperAgentConfig::default(),
        policy_cache: None,
        decision_buffer: None,
        agent_id: "throughput-bench".to_string(),
    });

    let app = Router::new()
        .route("/api/v1/messages", post(evaluate_policy))
        .route("/api/v1/fast-messages", post(fast_evaluate_policy))
        .with_state(state);

    // ---- Serve the same router over TCP and UDS ----
    let tcp = TcpListener::bind("127.0.0.1:0").await?;
    let tcp_addr = tcp.local_addr()?;
    {
        let app = app.clone();
        tokio::spawn(async move {
            let _ = axum::serve(tcp, app).await;
        });
    }

    let uds_path = std::env::temp_dir().join(format!("reaper-bench-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&uds_path);
    let uds = UnixListener::bind(&uds_path)?;
    {
        let app = app.clone();
        tokio::spawn(async move {
            let _ = axum::serve(uds, app).await;
        });
    }
    // Let the servers come up.
    tokio::time::sleep(Duration::from_millis(200)).await;

    println!(
        "Reaper throughput — {} connections, {}s warmup + {}s measure per config\n\
         TCP: {}   UDS: {}\n",
        args.connections,
        args.warmup_secs,
        args.duration_secs,
        tcp_addr,
        uds_path.display()
    );

    let tcp_t = Transport::Tcp(tcp_addr);
    let uds_t = Transport::Uds(uds_path.clone());

    let configs = [
        Config { label: "TCP  fast  compiled", transport: tcp_t.clone(), path: "/api/v1/fast-messages", policy_name: COMPILED_POLICY_NAME },
        Config { label: "UDS  fast  compiled", transport: uds_t.clone(), path: "/api/v1/fast-messages", policy_name: COMPILED_POLICY_NAME },
        Config { label: "TCP  fast  ast",      transport: tcp_t.clone(), path: "/api/v1/fast-messages", policy_name: AST_POLICY_NAME },
        Config { label: "UDS  fast  ast",      transport: uds_t.clone(), path: "/api/v1/fast-messages", policy_name: AST_POLICY_NAME },
        Config { label: "TCP  std   compiled", transport: tcp_t.clone(), path: "/api/v1/messages",      policy_name: COMPILED_POLICY_NAME },
        Config { label: "UDS  std   compiled", transport: uds_t.clone(), path: "/api/v1/messages",      policy_name: COMPILED_POLICY_NAME },
    ];

    println!(
        "{:<22} {:>12} {:>10} {:>10} {:>10} {:>10}",
        "config", "req/s", "p50(µs)", "p95(µs)", "p99(µs)", "max(µs)"
    );
    println!("{}", "-".repeat(78));

    let mut results = Vec::new();
    for cfg in &configs {
        let body = Bytes::from(format!(
            r#"{{"principal":"alice","action":"read","resource":"doc1","policy_name":"{}"}}"#,
            cfg.policy_name
        ));
        // Warmup (discarded), then measure.
        run(&cfg.transport, cfg.path, &body, args.connections, Duration::from_secs(args.warmup_secs)).await?;
        let stats = run(&cfg.transport, cfg.path, &body, args.connections, Duration::from_secs(args.duration_secs)).await?;

        println!(
            "{:<22} {:>12.0} {:>10.2} {:>10.2} {:>10.2} {:>10.2}",
            cfg.label,
            stats.rps(),
            stats.pct(0.50),
            stats.pct(0.95),
            stats.pct(0.99),
            stats.max_us()
        );
        results.push((cfg.label, stats));
    }

    print_comparisons(&results);

    let _ = std::fs::remove_file(&uds_path);
    Ok(())
}

fn print_comparisons(results: &[(&str, Stats)]) {
    let rps = |label: &str| results.iter().find(|(l, _)| *l == label).map(|(_, s)| s.rps());
    println!("\nComparisons (throughput ratio):");
    if let (Some(u), Some(t)) = (rps("UDS  fast  compiled"), rps("TCP  fast  compiled")) {
        println!("  UDS vs TCP  (fast, compiled):  {:.2}x", u / t);
    }
    if let (Some(c), Some(a)) = (rps("TCP  fast  compiled"), rps("TCP  fast  ast")) {
        println!("  compiled vs AST (TCP, fast):   {:.2}x", c / a);
    }
    if let (Some(c), Some(a)) = (rps("UDS  fast  compiled"), rps("UDS  fast  ast")) {
        println!("  compiled vs AST (UDS, fast):   {:.2}x", c / a);
    }
    if let (Some(f), Some(s)) = (rps("TCP  fast  compiled"), rps("TCP  std   compiled")) {
        println!("  fast(sonic) vs std(serde) TCP: {:.2}x", f / s);
    }
}

enum EvaluatorKind {
    Compiled,
    Ast,
}

/// Deploy the policy under `name`, building either the compiled or AST evaluator
/// directly (bypassing the compile-or-fallback path so we benchmark each one).
fn deploy(
    engine: &PolicyEngine,
    store: &Arc<DataStore>,
    name: &str,
    kind: EvaluatorKind,
) -> anyhow::Result<()> {
    let reaper_policy: ReaperPolicy = POLICY.parse().map_err(|e| anyhow::anyhow!("{e}"))?;
    let evaluator: Arc<dyn policy_engine::PolicyEvaluator> = match kind {
        EvaluatorKind::Compiled => Arc::new(
            reaper_policy
                .build(store.clone())
                .map_err(|e| anyhow::anyhow!("compile failed: {e}"))?,
        ),
        EvaluatorKind::Ast => Arc::new(reaper_policy.build_ast_evaluator(store.clone())),
    };

    let mut policy = EnhancedPolicy::new(name.to_string(), "throughput bench".to_string(), vec![]);
    policy.language = PolicyLanguage::ReaperDsl;
    policy.content = POLICY.to_string();
    policy.evaluator = Some(evaluator);
    engine
        .deploy_policy(policy)
        .map_err(|e| anyhow::anyhow!("deploy failed: {e}"))?;
    Ok(())
}

/// Run `connections` closed-loop workers against `transport` for `duration`.
async fn run(
    transport: &Transport,
    path: &str,
    body: &Bytes,
    connections: usize,
    duration: Duration,
) -> anyhow::Result<Stats> {
    let deadline = Instant::now() + duration;
    let mut handles = Vec::with_capacity(connections);

    for _ in 0..connections {
        let path = path.to_string();
        let body = body.clone();
        match transport.clone() {
            Transport::Tcp(addr) => {
                let stream = TcpStream::connect(addr).await?;
                stream.set_nodelay(true)?;
                handles.push(tokio::spawn(worker(stream, path, body, deadline)));
            }
            Transport::Uds(sock) => {
                let stream = UnixStream::connect(&sock).await?;
                handles.push(tokio::spawn(worker(stream, path, body, deadline)));
            }
        }
    }

    let start = Instant::now();
    let mut latencies_ns = Vec::new();
    for h in handles {
        latencies_ns.extend(h.await?);
    }
    let wall = start.elapsed();
    latencies_ns.sort_unstable();

    Ok(Stats {
        count: latencies_ns.len() as u64,
        wall,
        latencies_ns,
    })
}

/// One keep-alive connection: HTTP/1.1 handshake, then send requests in a loop
/// until the deadline, recording per-request round-trip latency.
async fn worker<S>(stream: S, path: String, body: Bytes, deadline: Instant) -> Vec<u64>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let io = TokioIo::new(stream);
    let (mut sender, conn) = match http1::handshake(io).await {
        Ok(pair) => pair,
        Err(_) => return Vec::new(),
    };
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let mut latencies = Vec::new();
    while Instant::now() < deadline {
        let req = Request::builder()
            .method(Method::POST)
            .uri(&path)
            .header("host", "localhost")
            .header("content-type", "application/json")
            .body(Full::new(body.clone()))
            .expect("request build");

        let start = Instant::now();
        match sender.send_request(req).await {
            Ok(resp) => {
                // Drain the body so the connection is reusable (keep-alive).
                if resp.into_body().collect().await.is_err() {
                    break;
                }
                latencies.push(start.elapsed().as_nanos() as u64);
            }
            Err(_) => break,
        }
    }
    latencies
}
