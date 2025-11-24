/// High-performance HTTP/2 server for Reaper policy evaluation
///
/// Optimizations implemented:
/// - HTTP/2 with persistent connections (multiplexing, header compression)
/// - WebSocket support for streaming decisions
/// - Batch evaluation endpoint (amortize overhead)
/// - Keep-Alive connection reuse
/// - Zero-copy Arc sharing of DataStore
/// - CORS support for web clients
/// - Structured error responses
///
/// Usage:
///   cargo run --example http_server --release -- --policy examples/rbac.reap --data large-test-data.json
///
/// Endpoints:
///   POST /v1/evaluate          - Single evaluation
///   POST /v1/evaluate/batch    - Batch evaluations
///   GET  /v1/health            - Health check
///   GET  /v1/metrics           - Performance metrics
///   WS   /v1/stream            - WebSocket streaming
use policy_engine::{
    DataStore, DataLoader, ReaperPolicy, PolicyEvaluator,
    PolicyRequest, PolicyAction,
};
use std::collections::HashMap;
use axum::{
    Router,
    extract::{State, WebSocketUpgrade, ws::{WebSocket, Message}},
    response::{Response, IntoResponse},
    http::{StatusCode, Method},
    Json,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;
use tokio::net::TcpListener;
use tower::ServiceBuilder;
use tower_http::cors::{CorsLayer, Any};
use parking_lot::RwLock;

// ============================================================================
// Request/Response Types
// ============================================================================

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EvaluateRequest {
    pub principal: String,
    pub action: String,
    pub resource: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BatchEvaluateRequest {
    pub requests: Vec<EvaluateRequest>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvaluateResponse {
    pub decision: String,
    pub allowed: bool,
    pub evaluation_time_ns: u128,
}

#[derive(Debug, Clone, Serialize)]
pub struct BatchEvaluateResponse {
    pub results: Vec<EvaluateResponse>,
    pub total_time_ns: u128,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub entity_count: usize,
    pub uptime_seconds: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MetricsResponse {
    pub total_evaluations: u64,
    pub mean_eval_time_ns: u64,
    pub requests_per_second: f64,
    pub entity_count: usize,
}

// ============================================================================
// Application State
// ============================================================================

pub struct AppState {
    pub evaluator: Box<dyn PolicyEvaluator>,
    pub store: Arc<DataStore>,
    pub metrics: RwLock<Metrics>,
    pub start_time: Instant,
}

#[derive(Debug, Clone)]
pub struct Metrics {
    pub total_evaluations: u64,
    pub total_eval_time_ns: u128,
    pub last_second_count: u64,
    pub last_second_time: Instant,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            total_evaluations: 0,
            total_eval_time_ns: 0,
            last_second_count: 0,
            last_second_time: Instant::now(),
        }
    }
}

// ============================================================================
// HTTP Handlers
// ============================================================================

/// Single evaluation endpoint
/// POST /v1/evaluate
async fn handle_evaluate(
    State(state): State<Arc<AppState>>,
    Json(req): Json<EvaluateRequest>,
) -> Result<Json<EvaluateResponse>, AppError> {
    let start = Instant::now();

    // Build request with principal in context
    let mut context = HashMap::new();
    context.insert("principal".to_string(), req.principal);

    let policy_req = PolicyRequest {
        resource: req.resource,
        action: req.action,
        context,
    };

    // Evaluate
    let decision = state.evaluator.evaluate(&policy_req)
        .map_err(|e| AppError::EvaluationError(e.to_string()))?;

    let eval_time = start.elapsed().as_nanos();

    // Update metrics
    {
        let mut metrics = state.metrics.write();
        metrics.total_evaluations += 1;
        metrics.total_eval_time_ns += eval_time;
    }

    // Build response
    let (decision_str, allowed) = match decision {
        PolicyAction::Allow => ("allow", true),
        PolicyAction::Deny => ("deny", false),
        PolicyAction::Log => ("log", false),
    };

    Ok(Json(EvaluateResponse {
        decision: decision_str.to_string(),
        allowed,
        evaluation_time_ns: eval_time,
    }))
}

/// Batch evaluation endpoint
/// POST /v1/evaluate/batch
async fn handle_batch_evaluate(
    State(state): State<Arc<AppState>>,
    Json(batch_req): Json<BatchEvaluateRequest>,
) -> Result<Json<BatchEvaluateResponse>, AppError> {
    let start = Instant::now();

    let mut results = Vec::with_capacity(batch_req.requests.len());

    for req in batch_req.requests {
        let eval_start = Instant::now();

        // Build request with principal in context
        let mut context = HashMap::new();
        context.insert("principal".to_string(), req.principal);

        let policy_req = PolicyRequest {
            resource: req.resource,
            action: req.action,
            context,
        };

        // Evaluate
        let decision = state.evaluator.evaluate(&policy_req)
            .map_err(|e| AppError::EvaluationError(e.to_string()))?;

        let eval_time = eval_start.elapsed().as_nanos();

        let (decision_str, allowed) = match decision {
            PolicyAction::Allow => ("allow", true),
            PolicyAction::Deny => ("deny", false),
            PolicyAction::Log => ("log", false),
        };

        results.push(EvaluateResponse {
            decision: decision_str.to_string(),
            allowed,
            evaluation_time_ns: eval_time,
        });
    }

    let total_time = start.elapsed().as_nanos();
    let count = results.len();

    // Update metrics
    {
        let mut metrics = state.metrics.write();
        metrics.total_evaluations += count as u64;
        metrics.total_eval_time_ns += total_time;
    }

    Ok(Json(BatchEvaluateResponse {
        results,
        total_time_ns: total_time,
        count,
    }))
}

/// Health check endpoint
/// GET /v1/health
async fn handle_health(
    State(state): State<Arc<AppState>>,
) -> Json<HealthResponse> {
    let uptime = state.start_time.elapsed().as_secs();
    let entity_count = state.store.stats().total_entities;

    Json(HealthResponse {
        status: "healthy".to_string(),
        entity_count,
        uptime_seconds: uptime,
    })
}

/// Metrics endpoint
/// GET /v1/metrics
async fn handle_metrics(
    State(state): State<Arc<AppState>>,
) -> Json<MetricsResponse> {
    let metrics = state.metrics.read();
    let entity_count = state.store.stats().total_entities;

    let mean_eval_time = if metrics.total_evaluations > 0 {
        (metrics.total_eval_time_ns / metrics.total_evaluations as u128) as u64
    } else {
        0
    };

    let uptime_seconds = state.start_time.elapsed().as_secs_f64();
    let rps = if uptime_seconds > 0.0 {
        metrics.total_evaluations as f64 / uptime_seconds
    } else {
        0.0
    };

    Json(MetricsResponse {
        total_evaluations: metrics.total_evaluations,
        mean_eval_time_ns: mean_eval_time,
        requests_per_second: rps,
        entity_count,
    })
}

/// WebSocket streaming endpoint
/// WS /v1/stream
async fn handle_websocket(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> Response {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: Arc<AppState>) {
    while let Some(msg) = socket.recv().await {
        let msg = match msg {
            Ok(msg) => msg,
            Err(e) => {
                eprintln!("WebSocket error: {}", e);
                break;
            }
        };

        if let Message::Text(text) = msg {
            // Parse request
            let req: EvaluateRequest = match serde_json::from_str(&text) {
                Ok(req) => req,
                Err(e) => {
                    let error = ErrorResponse {
                        error: "parse_error".to_string(),
                        message: format!("Failed to parse request: {}", e),
                    };
                    if let Ok(error_json) = serde_json::to_string(&error) {
                        let _ = socket.send(Message::Text(error_json.into())).await;
                    }
                    continue;
                }
            };

            // Evaluate
            let start = Instant::now();

            // Build request with principal in context
            let mut context = HashMap::new();
            context.insert("principal".to_string(), req.principal);

            let policy_req = PolicyRequest {
                resource: req.resource,
                action: req.action,
                context,
            };

            let decision = match state.evaluator.evaluate(&policy_req) {
                Ok(d) => d,
                Err(e) => {
                    let error = ErrorResponse {
                        error: "evaluation_error".to_string(),
                        message: format!("Evaluation failed: {}", e),
                    };
                    if let Ok(error_json) = serde_json::to_string(&error) {
                        let _ = socket.send(Message::Text(error_json.into())).await;
                    }
                    continue;
                }
            };

            let eval_time = start.elapsed().as_nanos();

            // Update metrics
            {
                let mut metrics = state.metrics.write();
                metrics.total_evaluations += 1;
                metrics.total_eval_time_ns += eval_time;
            }

            let (decision_str, allowed) = match decision {
                PolicyAction::Allow => ("allow", true),
                PolicyAction::Deny => ("deny", false),
                PolicyAction::Log => ("log", false),
            };

            let response = EvaluateResponse {
                decision: decision_str.to_string(),
                allowed,
                evaluation_time_ns: eval_time,
            };

            // Send response
            if let Ok(response_json) = serde_json::to_string(&response) {
                if let Err(e) = socket.send(Message::Text(response_json.into())).await {
                    eprintln!("Failed to send WebSocket response: {}", e);
                    break;
                }
            }
        }
    }
}

// ============================================================================
// Error Handling
// ============================================================================

#[derive(Debug)]
enum AppError {
    EvaluationError(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, error_type, message) = match self {
            AppError::EvaluationError(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "evaluation_error", msg)
            }
        };

        let body = Json(ErrorResponse {
            error: error_type.to_string(),
            message,
        });

        (status, body).into_response()
    }
}

// ============================================================================
// Server Setup
// ============================================================================

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Parse command line arguments
    let args: Vec<String> = std::env::args().collect();
    let mut policy_path = "examples/rbac.reap";
    let mut data_path = "examples/test-data.json";
    let mut port = 3000;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--policy" => {
                policy_path = &args[i + 1];
                i += 2;
            }
            "--data" => {
                data_path = &args[i + 1];
                i += 2;
            }
            "--port" => {
                port = args[i + 1].parse()?;
                i += 2;
            }
            _ => i += 1,
        }
    }

    println!("🚀 Starting Reaper HTTP Server");
    println!("   Policy: {}", policy_path);
    println!("   Data:   {}", data_path);
    println!("   Port:   {}", port);

    // Load data
    println!("📊 Loading data...");
    let store = DataStore::new();
    let loader = DataLoader::new(store.clone());
    let data_content = std::fs::read_to_string(data_path)?;
    let entity_count = loader.load_json(&data_content)?;
    let store = Arc::new(store);
    println!("   Loaded {} entities", entity_count);

    // Load and compile policy
    println!("📜 Loading policy...");
    let policy = ReaperPolicy::from_file(policy_path)?;
    let evaluator = policy.build(store.clone())?;
    println!("   Policy compiled successfully");

    // Create application state
    let state = Arc::new(AppState {
        evaluator: Box::new(evaluator),
        store,
        metrics: RwLock::new(Metrics::default()),
        start_time: Instant::now(),
    });

    // Configure CORS for web clients
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers(Any);

    // Build router
    let app = Router::new()
        .route("/v1/evaluate", post(handle_evaluate))
        .route("/v1/evaluate/batch", post(handle_batch_evaluate))
        .route("/v1/health", get(handle_health))
        .route("/v1/metrics", get(handle_metrics))
        .route("/v1/stream", get(handle_websocket))
        .layer(
            ServiceBuilder::new()
                .layer(cors)
        )
        .with_state(state);

    // Start server
    let addr = format!("0.0.0.0:{}", port);
    let listener = TcpListener::bind(&addr).await?;

    println!("\n✅ Server running on http://{}", addr);
    println!("\n📡 Endpoints:");
    println!("   POST   http://{}/v1/evaluate        - Single evaluation", addr);
    println!("   POST   http://{}/v1/evaluate/batch  - Batch evaluations", addr);
    println!("   GET    http://{}/v1/health          - Health check", addr);
    println!("   GET    http://{}/v1/metrics         - Metrics", addr);
    println!("   WS     ws://{}/v1/stream          - WebSocket stream", addr);
    println!("\n🔧 Optimizations enabled:");
    println!("   ✓ HTTP/2 with persistent connections");
    println!("   ✓ Keep-Alive connection reuse");
    println!("   ✓ WebSocket streaming support");
    println!("   ✓ Batch endpoint (amortize overhead)");
    println!("   ✓ CORS enabled for web clients");
    println!("   ✓ Zero-copy Arc<DataStore> sharing");
    println!("\nPress Ctrl+C to stop\n");

    axum::serve(listener, app).await?;

    Ok(())
}
