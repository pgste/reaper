use clap::{Parser, Subcommand};
use reqwest::Client;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

// Reap policy imports
use policy_engine::{
    DataLoader, DataStore, PolicyAction as EngineAction, PolicyBundle, PolicyEvaluator,
    PolicyRequest, ReaperPolicy,
};

// eBPF command handlers (Linux only)
#[cfg(target_os = "linux")]
mod ebpf_commands;
#[cfg(target_os = "linux")]
use ebpf_commands::{handle_analyze_policy, handle_validate_data};

// Stub implementations for non-Linux platforms
#[cfg(not(target_os = "linux"))]
fn handle_validate_data(
    _file: &str,
    _check_ebpf: bool,
    _format: &str,
    _custom_schemas: Option<&str>,
) -> anyhow::Result<()> {
    anyhow::bail!("eBPF commands are only available on Linux")
}

#[cfg(not(target_os = "linux"))]
fn handle_analyze_policy(
    _file: &str,
    _check_ebpf: bool,
    _show_recommendations: bool,
    _format: &str,
) -> anyhow::Result<()> {
    anyhow::bail!("eBPF commands are only available on Linux")
}

mod library;

#[derive(Parser)]
#[command(name = "reaper")]
#[command(about = "Reaper CLI - High-Performance Policy Management")]
#[command(version = reaper_core::VERSION)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Platform URL
    #[arg(long, default_value = "http://localhost:8081")]
    platform_url: String,

    /// Agent URL
    #[arg(long, default_value = "http://localhost:8080")]
    agent_url: String,
}

#[derive(Subcommand)]
enum Commands {
    /// Evaluate a policy file locally (.reap, .yaml, .yml, .json)
    Eval {
        /// Path to policy file (.reap, .yaml, .yml, or .json)
        #[arg(short, long)]
        policy: String,

        /// Path to JSON data file
        #[arg(short, long)]
        data: String,

        /// Principal (user) ID
        #[arg(long)]
        principal: String,

        /// Action to evaluate
        #[arg(short, long)]
        action: String,

        /// Resource ID
        #[arg(short, long)]
        resource: String,

        /// Show detailed timing information
        #[arg(long)]
        timing: bool,
    },

    /// Compile policy to binary bundle (.rbb)
    Compile {
        /// Input policy file(s) (.reap, .yaml, .yml, or .json)
        #[arg(required = true)]
        input: Vec<String>,

        /// Output bundle file (.rbb)
        #[arg(short, long)]
        output: String,

        /// Enable optimizations
        #[arg(long)]
        optimize: bool,

        /// Show bundle metadata
        #[arg(long)]
        info: bool,
    },

    /// Validate policy syntax
    Validate {
        /// Path to policy file (.reap, .yaml, .yml, or .json)
        policy: String,

        /// Path to JSON data file for validation
        #[arg(short, long)]
        data: Option<String>,

        /// Show detailed parse tree
        #[arg(long)]
        verbose: bool,
    },

    /// Check a JSON document against a policy (CI mode): evaluates EVERY deny
    /// rule and reports all violations with messages; exit code 1 on failure.
    /// The conftest workflow: `reaper-cli check -p tf.reap -i plan.json`
    Check {
        /// Path to policy file (.reap, .yaml, .yml, or .json)
        #[arg(short, long)]
        policy: String,

        /// Path to the JSON input document ('-' for stdin)
        #[arg(short, long)]
        input: String,

        /// Optional entity data file (for policies that also use user/resource)
        #[arg(short, long)]
        data: Option<String>,

        /// Principal id (optional; document checks usually have none)
        #[arg(long)]
        principal: Option<String>,

        /// Action (default: "check")
        #[arg(long, default_value = "check")]
        action: String,

        /// Resource (default: the input file name)
        #[arg(long)]
        resource: Option<String>,

        /// Output format: text (default) or json
        #[arg(long, default_value = "text")]
        format: String,
    },

    /// Generate a bundle signing keypair (Ed25519 or ECDSA P-256)
    Keygen {
        /// Signature algorithm: ed25519-sha256 or ecdsa-p256-sha256
        #[arg(long, default_value = "ed25519-sha256")]
        algorithm: String,

        /// Key id advertised in signatures (for rotation)
        #[arg(long, default_value = "default")]
        key_id: String,
    },

    /// Decision-log data protection utilities (keys, decryption)
    Decisions {
        #[command(subcommand)]
        action: DecisionsAction,
    },

    /// Browse and run the bundled policy examples library
    Library {
        #[command(subcommand)]
        action: LibraryAction,
    },

    /// Policy management commands (platform/agent)
    Policy {
        #[command(subcommand)]
        action: PolicyAction,
    },
    /// Agent management commands
    Agent {
        #[command(subcommand)]
        action: AgentAction,
    },
    /// Platform status and monitoring
    Status,
    /// Demo the complete workflow
    Demo,
    /// Run performance tests
    Benchmark {
        /// Number of requests to send
        #[arg(short, long, default_value = "1000")]
        requests: usize,
    },

    /// Validate entity data for eBPF loading
    ValidateData {
        /// Path to entity JSON file
        #[arg(short, long)]
        file: String,

        /// Check eBPF compatibility
        #[arg(long)]
        check_ebpf: bool,

        /// Output format (table, json)
        #[arg(long, default_value = "table")]
        format: String,

        /// Path to custom schema definitions (JSON)
        #[arg(long)]
        custom_schemas: Option<String>,
    },

    /// Analyze policy for eBPF promotability
    AnalyzePolicy {
        /// Path to policy file (.reap, .yaml, .yml, .json)
        #[arg(short, long)]
        file: String,

        /// Check eBPF compatibility
        #[arg(long)]
        check_ebpf: bool,

        /// Show detailed recommendations
        #[arg(long)]
        show_recommendations: bool,

        /// Output format (table, json)
        #[arg(long, default_value = "table")]
        format: String,
    },

    /// Bundle management commands
    Bundle {
        #[command(subcommand)]
        action: BundleAction,
    },

    /// Management plane commands (for centralized management)
    Management {
        #[command(subcommand)]
        action: ManagementAction,

        /// Management server URL
        #[arg(long, default_value = "http://localhost:3000")]
        management_url: String,

        /// API key for authentication
        #[arg(long, env = "REAPER_MANAGEMENT_API_KEY")]
        api_key: Option<String>,
    },

    /// Test a single policy assertion (returns exit code 0 for pass, 1 for fail)
    Test {
        /// Path to policy file (.reap, .yaml, .yml, or .json)
        #[arg(short, long)]
        policy: String,

        /// Path to JSON data file
        #[arg(short, long)]
        data: String,

        /// Principal (user) ID
        #[arg(long)]
        principal: String,

        /// Action to evaluate
        #[arg(short, long)]
        action: String,

        /// Resource ID
        #[arg(short, long)]
        resource: String,

        /// Expected decision (allow or deny)
        #[arg(short, long)]
        expect: String,

        /// Show detailed output
        #[arg(short, long)]
        verbose: bool,
    },

    /// Run a test suite from a YAML file
    TestSuite {
        /// Path to test suite YAML file
        #[arg(short, long)]
        file: String,

        /// Show detailed output for each test
        #[arg(short, long)]
        verbose: bool,

        /// Stop on first failure
        #[arg(long)]
        fail_fast: bool,
    },
}

#[derive(Subcommand)]
enum BundleAction {
    /// Show bundle information
    Info {
        /// Path to .rbb bundle file
        file: String,
    },
    /// Deploy bundle to agent
    Deploy {
        /// Path to .rbb bundle file
        file: String,

        /// Optional path to JSON data file to load before deploying
        #[arg(short, long)]
        data: Option<String>,

        /// Force deployment even if version already exists
        #[arg(long)]
        force: bool,
    },
    /// Rollback policy to previous version
    Rollback {
        /// Policy ID to rollback
        policy_id: String,

        /// Target version to rollback to
        version: String,
    },
    /// List versions of a deployed policy
    Versions {
        /// Policy ID to list versions for
        policy_id: String,
    },
    /// Create a policy package (.rpp) from multiple policies
    Package {
        /// Input policy files (.reap, .yaml, .yml, or .json)
        #[arg(required = true)]
        input: Vec<String>,

        /// Output package file (.rpp)
        #[arg(short, long)]
        output: String,

        /// Package name
        #[arg(short, long)]
        name: Option<String>,

        /// Package version
        #[arg(short, long, default_value = "1.0.0")]
        version: String,
    },
}

#[derive(Subcommand)]
enum DecisionsAction {
    /// Generate secrets for decision-log data protection (HMAC salt + AES key)
    Keygen,
    /// Decrypt an encrypted input_data envelope from a decision log entry
    Decrypt {
        /// 64-hex-char AES-256-GCM key (REAPER_DECISION_LOG_ENCRYPTION_KEY)
        #[arg(long)]
        key: String,
        /// The envelope JSON ({"enc":"aes256gcm",...}) or a full decision
        /// entry / NDJSON line containing an "input_data" field. Use '-' to
        /// read from stdin (e.g. pipe a line from decisions.ndjson).
        input: String,
    },
}

#[derive(Subcommand)]
enum LibraryAction {
    /// List all scenarios (id, name, models, cases)
    List {
        /// Path to the policy library (default: ./policy-library or $REAPER_LIBRARY_PATH)
        #[arg(long)]
        path: Option<String>,
    },
    /// Show a scenario: its walkthrough (README) and the policy source
    Show {
        /// Scenario id as printed by `library list` (e.g. combined/saas-tenancy)
        id: String,
        #[arg(long)]
        path: Option<String>,
    },
    /// Run a scenario's manifest cases and report PASS/FAIL (exit 1 on failure)
    Run {
        /// Scenario id, or omit to run the entire library
        id: Option<String>,
        #[arg(long)]
        path: Option<String>,
    },
}

#[derive(Subcommand)]
enum PolicyAction {
    /// List all policies
    List,
    /// Create a new policy
    Create {
        name: String,
        #[arg(short, long, default_value = "allow")]
        action: String,
        #[arg(short, long, default_value = "*")]
        resource: String,
        #[arg(short, long)]
        description: Option<String>,
    },
    /// Update an existing policy
    Update {
        id: String,
        #[arg(short, long)]
        name: Option<String>,
        #[arg(short, long)]
        action: Option<String>,
        #[arg(short, long)]
        resource: Option<String>,
        #[arg(short, long)]
        description: Option<String>,
    },
    /// Delete a policy
    Delete { id: String },
    /// Deploy policy to agents
    Deploy {
        id: String,
        #[arg(long)]
        verify: bool,
    },
    /// Evaluate a policy
    Evaluate {
        #[arg(short, long)]
        policy_id: Option<String>,
        #[arg(short, long)]
        policy_name: Option<String>,
        resource: String,
        action: String,
    },
}

#[derive(Subcommand)]
enum AgentAction {
    /// List all agents
    List,
    /// Show agent details
    Show { id: String },
    /// Check agent health
    Health,
    /// Show agent metrics
    Metrics,
}

#[derive(Subcommand)]
enum ManagementAction {
    /// Check management server health
    Health,

    /// List organizations
    Orgs,

    /// List policy sources in an organization
    Sources {
        /// Organization ID or name
        #[arg(short, long)]
        org: String,
    },

    /// List bundles
    Bundles {
        /// Organization ID or name
        #[arg(short, long)]
        org: String,

        /// Show only promoted bundles
        #[arg(long)]
        promoted: bool,
    },

    /// Show bundle details
    BundleInfo {
        /// Bundle ID
        id: String,
    },

    /// List agents registered with management
    Agents {
        /// Organization ID or name
        #[arg(short, long)]
        org: String,
    },

    /// Push a policy to the management server
    Push {
        /// Organization ID or name
        #[arg(short, long)]
        org: String,

        /// Policy source name
        #[arg(short, long)]
        source: String,

        /// Path to policy file (.reap, .yaml, .yml, or .json)
        file: String,

        /// Policy name (defaults to filename)
        #[arg(short, long)]
        name: Option<String>,

        /// Policy description
        #[arg(short, long)]
        description: Option<String>,
    },

    /// Create and promote a bundle
    Promote {
        /// Organization ID or name
        #[arg(short, long)]
        org: String,

        /// Bundle name
        #[arg(short, long)]
        name: String,

        /// Policy IDs to include (comma-separated)
        #[arg(short, long)]
        policies: String,

        /// Bundle description
        #[arg(short, long)]
        description: Option<String>,
    },
}

// ============================================================================
// .reap File Command Handlers
// ============================================================================

/// Handle: reaper eval
fn handle_eval(
    policy_path: &str,
    data_path: &str,
    principal: &str,
    action: &str,
    resource: &str,
    show_timing: bool,
) -> anyhow::Result<()> {
    println!("🔍 Evaluating Reaper Policy\n");

    // Validate inputs
    if !Path::new(policy_path).exists() {
        anyhow::bail!("❌ Error: Policy file not found: {}", policy_path);
    }
    if !Path::new(data_path).exists() {
        anyhow::bail!("❌ Error: Data file not found: {}", data_path);
    }

    // Load and parse policy (auto-detect format)
    println!("1️⃣  Loading policy: {}", policy_path);
    let load_start = Instant::now();
    let policy = ReaperPolicy::from_file_auto(policy_path).map_err(|e| {
        anyhow::anyhow!("❌ Failed to parse policy: {:?}\n\nCheck your policy syntax (.reap, .yaml, .yml, or .json)", e)
    })?;
    let load_time = load_start.elapsed();

    println!("   ✓ Parsed policy: {}", policy.name());
    if let Some(version) = policy.version() {
        println!("   ✓ Version: {}", version);
    }
    if show_timing {
        println!("   ⏱  Parse time: {:?}", load_time);
    }
    println!();

    // Load data
    println!("2️⃣  Loading data: {}", data_path);
    let data_content = fs::read_to_string(data_path)
        .map_err(|e| anyhow::anyhow!("❌ Failed to read data file: {}", e))?;

    let store = DataStore::new();
    let loader = DataLoader::new(store.clone());

    let entity_count = loader.load_json(&data_content).map_err(|e| {
        anyhow::anyhow!("❌ Failed to load data: {:?}\n\nCheck your JSON format", e)
    })?;

    println!("   ✓ Loaded {} entities", entity_count);
    println!();

    // Build evaluator
    println!("3️⃣  Building evaluator...");
    let build_start = Instant::now();
    let store = Arc::new(store);
    let evaluator = policy
        .build(store.clone())
        .map_err(|e| anyhow::anyhow!("❌ Failed to build evaluator: {:?}", e))?;
    let build_time = build_start.elapsed();

    println!("   ✓ Evaluator ready");
    if show_timing {
        println!("   ⏱  Build time: {:?}", build_time);
    }
    println!();

    // Validate entities exist
    println!("4️⃣  Validating request...");
    let interner = store.interner();
    let principal_id = interner.intern(principal);
    let resource_id = interner.intern(resource);

    if store.get(principal_id).is_none() {
        anyhow::bail!(
            "❌ Error: Principal '{}' not found in data\n   Available entities: Use --verbose to list",
            principal
        );
    }

    if store.get(resource_id).is_none() {
        anyhow::bail!(
            "❌ Error: Resource '{}' not found in data\n   Available entities: Use --verbose to list",
            resource
        );
    }

    println!("   ✓ Principal: {}", principal);
    println!("   ✓ Action: {}", action);
    println!("   ✓ Resource: {}", resource);
    println!();

    // Evaluate policy
    println!("5️⃣  Evaluating policy...");
    let mut context = HashMap::new();
    context.insert("principal".to_string(), principal.to_string());

    let request = PolicyRequest {
        resource: resource.to_string(),
        action: action.to_string(),
        context,
    };

    let eval_start = Instant::now();
    let decision = evaluator
        .evaluate(&request)
        .map_err(|e| anyhow::anyhow!("❌ Evaluation failed: {:?}", e))?;
    let eval_time = eval_start.elapsed();

    // Display result
    println!();
    println!("═══════════════════════════════════════════════════════");
    println!("                    📊 RESULT                          ");
    println!("═══════════════════════════════════════════════════════");

    let (symbol, decision_text) = match decision {
        EngineAction::Allow => ("✅", "ALLOW"),
        EngineAction::Deny => ("❌", "DENY"),
        EngineAction::Log => ("📝", "LOG"),
    };

    println!(" {} Decision: {}", symbol, decision_text);
    println!();

    if show_timing {
        println!("⏱  Performance:");
        println!("   • Parse policy: {:?}", load_time);
        println!("   • Build evaluator: {:?}", build_time);
        println!(
            "   • Evaluate: {:?} ({:.0} ns)",
            eval_time,
            eval_time.as_nanos()
        );
        println!();
    } else {
        println!(
            "⏱  Evaluation time: {:?} ({:.0} ns)",
            eval_time,
            eval_time.as_nanos()
        );
        println!();
    }

    println!("═══════════════════════════════════════════════════════");

    Ok(())
}

/// Handle: reaper compile
fn handle_compile(
    input_files: &[String],
    output_path: &str,
    _optimize: bool,
    show_info: bool,
) -> anyhow::Result<()> {
    println!("🔨 Compiling Reaper Policy Bundle\n");

    if input_files.is_empty() {
        anyhow::bail!("❌ Error: No input files specified");
    }

    // For now, support single file compilation
    // TODO: Support multiple files and merging
    if input_files.len() > 1 {
        println!("⚠️  Warning: Multiple files specified, only first will be compiled");
        println!("   Multi-file bundling coming soon!");
        println!();
    }

    let input_path = &input_files[0];

    // Validate input
    if !Path::new(input_path).exists() {
        anyhow::bail!("❌ Error: Input file not found: {}", input_path);
    }

    // Load and parse policy (auto-detect format)
    println!("1️⃣  Parsing policy: {}", input_path);
    let policy = ReaperPolicy::from_file_auto(input_path).map_err(|e| {
        anyhow::anyhow!("❌ Failed to parse policy: {:?}\n\nCheck your policy syntax (.reap, .yaml, .yml, or .json)", e)
    })?;

    println!("   ✓ Parsed: {}", policy.name());
    if let Some(version) = policy.version() {
        println!("   ✓ Version: {}", version);
    }
    println!();

    // Compile to bundle
    println!("2️⃣  Compiling to binary bundle...");
    let compile_start = Instant::now();
    let bundle_bytes = policy
        .compile_to_bundle()
        .map_err(|e| anyhow::anyhow!("❌ Compilation failed: {:?}", e))?;
    let compile_time = compile_start.elapsed();

    println!("   ✓ Compiled successfully");
    println!("   ⏱  Compilation time: {:?}", compile_time);
    println!();

    // Write bundle
    println!("3️⃣  Writing bundle: {}", output_path);
    fs::write(output_path, &bundle_bytes)
        .map_err(|e| anyhow::anyhow!("❌ Failed to write bundle: {}", e))?;

    println!("   ✓ Bundle written");
    println!("   📦 Size: {} bytes", bundle_bytes.len());
    println!();

    // Show bundle info
    if show_info {
        println!("4️⃣  Bundle Information:");
        match PolicyBundle::from_bytes(&bundle_bytes) {
            Ok(bundle) => {
                println!("   • Policy: {}", bundle.metadata.policy_name);
                if let Some(v) = &bundle.metadata.policy_version {
                    println!("   • Version: {}", v);
                }
                println!("   • Format version: {}", bundle.metadata.version);
                println!("   • Compiled at: {}", bundle.metadata.compiled_at);
                println!("   • Checksum: {:x}", bundle.metadata.source_checksum);
                println!("   • Rules: {}", bundle.policy.rules.len());
            }
            Err(e) => {
                println!("   ⚠️  Could not read bundle metadata: {:?}", e);
            }
        }
        println!();
    }

    println!("✅ Success! Bundle ready for production deployment");
    println!();
    println!("Load with:");
    println!("   let bundle = fs::read(\"{}\").unwrap();", output_path);
    println!("   let evaluator = ReaperPolicy::from_bundle(&bundle, store)?;");

    Ok(())
}

/// Handle: reaper validate
fn handle_keygen(algorithm: &str, key_id: &str) -> anyhow::Result<()> {
    use reaper_core::bundle_signing::{SigAlgorithm, SigningKey};

    let alg = SigAlgorithm::parse(algorithm)
        .map_err(|e| anyhow::anyhow!("{e} (use ed25519-sha256 or ecdsa-p256-sha256)"))?;
    let key = SigningKey::generate(alg);

    println!("🔑 Reaper bundle signing keypair ({algorithm}), key_id={key_id}\n");
    println!("── Control plane (reaper-management) — KEEP THE PRIVATE KEY SECRET ──");
    println!("REAPER_BUNDLE_SIGNING_KEY={}", key.private_key_hex());
    println!("REAPER_BUNDLE_SIGNING_KEY_ID={key_id}");
    println!("REAPER_BUNDLE_SIGNING_ALGORITHM={algorithm}");
    println!("\n── Agents (reaper-agent) — distribute the PUBLIC key ──");
    println!(
        "REAPER_MANAGEMENT_BUNDLE_PUBLIC_KEY={}",
        key.public_key_hex()
    );
    println!("REAPER_MANAGEMENT_BUNDLE_SIGNATURE_ALGORITHM={algorithm}");
    println!("REAPER_MANAGEMENT_BUNDLE_KEY_ID={key_id}");
    println!("REAPER_MANAGEMENT_REQUIRE_SIGNED_BUNDLES=true");
    Ok(())
}

/// Handle: reaper decisions keygen
fn handle_decisions_keygen() -> anyhow::Result<()> {
    // A random 32-byte salt is plenty for HMAC-SHA-256 pseudonymization; reuse
    // the AES key generator since it produces exactly that.
    let salt = policy_engine::generate_encryption_key_hex();
    let key = policy_engine::generate_encryption_key_hex();

    println!("🔑 Decision-log data protection secrets — KEEP THESE SECRET\n");
    println!("── Agent (reaper-agent) ──");
    println!("REAPER_DECISION_LOG_HASH_PRINCIPAL=true");
    println!("REAPER_DECISION_LOG_HASH_SALT={salt}");
    println!("REAPER_DECISION_LOG_ENCRYPT_INPUT_DATA=true");
    println!("REAPER_DECISION_LOG_ENCRYPTION_KEY={key}");
    println!("\n── Optional masking (comma-separated, case-insensitive) ──");
    println!("# REAPER_DECISION_LOG_MASK_KEYS=ssn,password,token");
    println!("# REAPER_DECISION_LOG_CONTEXT_ALLOWLIST=request_id,ip");
    println!("\nStore the encryption key with whoever must read explain data");
    println!("(e.g. the control plane, per tenant). Decrypt with:");
    println!("  reaper-cli decisions decrypt --key <hex> '<input_data JSON>'");
    Ok(())
}

/// Handle: reaper decisions decrypt
fn handle_decisions_decrypt(key: &str, input: &str) -> anyhow::Result<()> {
    let raw = if input == "-" {
        let mut buf = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)?;
        buf
    } else {
        input.to_string()
    };

    let value: serde_json::Value = serde_json::from_str(raw.trim())
        .map_err(|e| anyhow::anyhow!("input is not valid JSON: {e}"))?;
    // Accept either the bare envelope or a full entry containing input_data.
    let envelope = if value.get("ciphertext").is_some() {
        &value
    } else {
        value
            .get("input_data")
            .ok_or_else(|| anyhow::anyhow!("no input_data field in the given entry"))?
    };

    let opened = policy_engine::decrypt_input_data(envelope, key)
        .map_err(|e| anyhow::anyhow!("decrypt failed: {e}"))?;
    println!("{}", serde_json::to_string_pretty(&opened)?);
    Ok(())
}

/// Handle: reaper check — evaluate a JSON document against a policy and
/// report every violation (CI gate: exit 1 when not allowed).
fn handle_check(
    policy_path: &str,
    input_path: &str,
    data_path: Option<&str>,
    principal: Option<&str>,
    action: &str,
    resource: Option<&str>,
    format: &str,
) -> anyhow::Result<()> {
    use policy_engine::PolicyRequest;

    let policy = ReaperPolicy::from_file_auto(policy_path)
        .map_err(|e| anyhow::anyhow!("failed to load policy {policy_path}: {e}"))?;

    let store = std::sync::Arc::new(DataStore::new());
    if let Some(data) = data_path {
        let json = std::fs::read_to_string(data)
            .map_err(|e| anyhow::anyhow!("failed to read data file {data}: {e}"))?;
        DataLoader::new((*store).clone())
            .load_json(&json)
            .map_err(|e| anyhow::anyhow!("failed to load data: {e}"))?;
    }

    let raw = if input_path == "-" {
        use std::io::Read;
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        buf
    } else {
        std::fs::read_to_string(input_path)
            .map_err(|e| anyhow::anyhow!("failed to read input {input_path}: {e}"))?
    };
    let input: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| anyhow::anyhow!("input is not valid JSON: {e}"))?;

    let mut context = std::collections::HashMap::new();
    if let Some(pr) = principal {
        context.insert("principal".to_string(), pr.to_string());
    }
    let request = PolicyRequest {
        resource: resource.unwrap_or(input_path).to_string(),
        action: action.to_string(),
        context,
    };

    let evaluator = policy.build_ast_evaluator(store);
    let result = evaluator
        .check_with_input(&request, Some(&input))
        .map_err(|e| anyhow::anyhow!("check failed: {e}"))?;

    match format {
        "json" => println!("{}", serde_json::to_string_pretty(&result)?),
        _ => {
            if result.violations.is_empty() {
                println!("PASS: no violations");
            } else {
                for v in &result.violations {
                    match &v.message {
                        Some(msg) => println!("FAIL [{}]: {}", v.rule, msg),
                        None => println!("FAIL [{}]", v.rule),
                    }
                }
                println!("\n{} violation(s)", result.violations.len());
            }
            if !result.allowed && result.violations.is_empty() {
                println!("DENIED: no allow rule matched (default deny)");
            }
        }
    }

    if !result.allowed {
        std::process::exit(1);
    }
    Ok(())
}

fn handle_validate(
    policy_path: &str,
    data_path: Option<&str>,
    verbose: bool,
) -> anyhow::Result<()> {
    println!("✅ Validating Reaper Policy\n");

    // Validate file exists
    if !Path::new(policy_path).exists() {
        anyhow::bail!("❌ Error: Policy file not found: {}", policy_path);
    }

    // Parse policy (auto-detect format)
    println!("1️⃣  Parsing policy: {}", policy_path);
    let parse_start = Instant::now();

    let policy = match ReaperPolicy::from_file_auto(policy_path) {
        Ok(p) => p,
        Err(e) => {
            println!();
            println!("❌ SYNTAX ERROR");
            println!("══════════════════════════════════════════════════════");
            println!("{:?}", e);
            println!("══════════════════════════════════════════════════════");
            println!();
            println!("💡 Common issues:");
            println!("   • Missing 'default_decision' field (YAML/JSON) or 'default' (Reap)");
            println!("   • Unmatched curly braces or incorrect YAML/JSON syntax");
            println!("   • Missing quotes around strings");
            println!("   • Invalid operators (use equal, not_equal, gt, lt, gte, lte)");
            println!("   • Unsupported file extension (use .reap, .yaml, .yml, or .json)");
            println!();
            anyhow::bail!("Validation failed");
        }
    };

    let parse_time = parse_start.elapsed();

    println!("   ✅ Syntax valid");
    println!("   ⏱  Parse time: {:?}", parse_time);
    println!();

    // Show policy info
    println!("2️⃣  Policy Information:");
    println!("   • Name: {}", policy.name());
    if let Some(version) = policy.version() {
        println!("   • Version: {}", version);
    }
    println!();

    // Validate with data if provided
    if let Some(data_path) = data_path {
        println!("3️⃣  Validating with data: {}", data_path);

        if !Path::new(data_path).exists() {
            anyhow::bail!("❌ Error: Data file not found: {}", data_path);
        }

        let data_content = fs::read_to_string(data_path)
            .map_err(|e| anyhow::anyhow!("❌ Failed to read data file: {}", e))?;

        let store = DataStore::new();
        let loader = DataLoader::new(store.clone());

        let entity_count = match loader.load_json(&data_content) {
            Ok(count) => count,
            Err(e) => {
                println!();
                println!("❌ DATA ERROR");
                println!("══════════════════════════════════════════════════════");
                println!("{:?}", e);
                println!("══════════════════════════════════════════════════════");
                println!();
                anyhow::bail!("Invalid data format");
            }
        };

        println!("   ✅ Data valid");
        println!("   • Entities loaded: {}", entity_count);
        println!();

        // Try to build evaluator
        println!("4️⃣  Building evaluator...");
        let store = Arc::new(store);
        if let Err(e) = policy.build(store) {
            println!();
            println!("❌ BUILD ERROR");
            println!("══════════════════════════════════════════════════════");
            println!("{:?}", e);
            println!("══════════════════════════════════════════════════════");
            anyhow::bail!("Failed to build evaluator");
        }

        println!("   ✅ Evaluator builds successfully");
        println!();
    }

    println!("══════════════════════════════════════════════════════");
    println!("✅ VALIDATION PASSED");
    println!("══════════════════════════════════════════════════════");

    if verbose {
        println!();
        println!("Policy file: {}", policy_path);
        if let Some(dp) = data_path {
            println!("Data file: {}", dp);
        }
    }

    Ok(())
}

// ============================================================================
// Test Command Handlers
// ============================================================================

/// Test case definition for test suite YAML files
#[derive(serde::Deserialize, Debug)]
struct TestCase {
    name: String,
    policy: String,
    data: String,
    principal: String,
    action: String,
    resource: String,
    expect: String,
}

/// Test suite definition
#[derive(serde::Deserialize, Debug)]
struct TestSuiteDefinition {
    tests: Vec<TestCase>,
}

/// Handle: reaper test
fn handle_test(
    policy_path: &str,
    data_path: &str,
    principal: &str,
    action: &str,
    resource: &str,
    expect: &str,
    verbose: bool,
) -> anyhow::Result<bool> {
    // Validate expected value
    let expected_decision = match expect.to_lowercase().as_str() {
        "allow" => EngineAction::Allow,
        "deny" => EngineAction::Deny,
        _ => anyhow::bail!(
            "Invalid expected decision: '{}'. Must be 'allow' or 'deny'",
            expect
        ),
    };

    // Validate inputs
    if !Path::new(policy_path).exists() {
        anyhow::bail!("Policy file not found: {}", policy_path);
    }
    if !Path::new(data_path).exists() {
        anyhow::bail!("Data file not found: {}", data_path);
    }

    // Load and parse policy
    let policy = ReaperPolicy::from_file_auto(policy_path)
        .map_err(|e| anyhow::anyhow!("Failed to parse policy: {:?}", e))?;

    // Load data
    let data_content = fs::read_to_string(data_path)
        .map_err(|e| anyhow::anyhow!("Failed to read data file: {}", e))?;

    let store = DataStore::new();
    let loader = DataLoader::new(store.clone());
    let _entity_count = loader
        .load_json(&data_content)
        .map_err(|e| anyhow::anyhow!("Failed to load data: {:?}", e))?;

    // Build evaluator
    let store = Arc::new(store);
    let evaluator = policy
        .build(store.clone())
        .map_err(|e| anyhow::anyhow!("Failed to build evaluator: {:?}", e))?;

    // Evaluate
    let mut context = HashMap::new();
    context.insert("principal".to_string(), principal.to_string());

    let request = PolicyRequest {
        resource: resource.to_string(),
        action: action.to_string(),
        context,
    };

    let eval_start = Instant::now();
    let actual_decision = evaluator
        .evaluate(&request)
        .map_err(|e| anyhow::anyhow!("Evaluation failed: {:?}", e))?;
    let eval_time = eval_start.elapsed();

    // Compare result
    let passed = actual_decision == expected_decision;

    if verbose {
        println!("Test: {} {} {} -> {}", principal, action, resource, expect);
        println!("  Policy: {}", policy_path);
        println!("  Expected: {:?}", expected_decision);
        println!("  Actual:   {:?}", actual_decision);
        println!("  Time:     {:?}", eval_time);
        if passed {
            println!("  Result:   PASS");
        } else {
            println!("  Result:   FAIL");
        }
    } else if passed {
        println!(
            "PASS: {} {} {} -> {:?}",
            principal, action, resource, actual_decision
        );
    } else {
        println!(
            "FAIL: {} {} {} -> {:?} (expected {:?})",
            principal, action, resource, actual_decision, expected_decision
        );
    }

    Ok(passed)
}

/// Handle: reaper test-suite
fn handle_test_suite(suite_path: &str, verbose: bool, fail_fast: bool) -> anyhow::Result<bool> {
    // Load test suite
    if !Path::new(suite_path).exists() {
        anyhow::bail!("Test suite file not found: {}", suite_path);
    }

    let content = fs::read_to_string(suite_path)
        .map_err(|e| anyhow::anyhow!("Failed to read test suite: {}", e))?;

    let suite: TestSuiteDefinition = serde_yaml::from_str(&content)
        .map_err(|e| anyhow::anyhow!("Failed to parse test suite YAML: {}", e))?;

    println!(
        "Running {} test(s) from {}\n",
        suite.tests.len(),
        suite_path
    );
    println!("═══════════════════════════════════════════════════════════════");

    let mut passed = 0;
    let mut failed = 0;
    let mut failures: Vec<String> = Vec::new();
    let start_time = Instant::now();

    for test in &suite.tests {
        if verbose {
            println!("\nTest: {}", test.name);
        }

        match handle_test(
            &test.policy,
            &test.data,
            &test.principal,
            &test.action,
            &test.resource,
            &test.expect,
            verbose,
        ) {
            Ok(true) => {
                passed += 1;
                if !verbose {
                    println!("  PASS: {}", test.name);
                }
            }
            Ok(false) => {
                failed += 1;
                failures.push(test.name.clone());
                if !verbose {
                    println!("  FAIL: {}", test.name);
                }
                if fail_fast {
                    println!("\nStopping on first failure (--fail-fast)");
                    break;
                }
            }
            Err(e) => {
                failed += 1;
                failures.push(format!("{}: {}", test.name, e));
                if !verbose {
                    println!("  ERROR: {} - {}", test.name, e);
                } else {
                    println!("  ERROR: {}", e);
                }
                if fail_fast {
                    println!("\nStopping on first failure (--fail-fast)");
                    break;
                }
            }
        }
    }

    let total_time = start_time.elapsed();
    println!("\n═══════════════════════════════════════════════════════════════");
    println!(
        "Results: {} passed, {} failed ({:?})",
        passed, failed, total_time
    );

    if !failures.is_empty() {
        println!("\nFailed tests:");
        for f in &failures {
            println!("  - {}", f);
        }
    }

    if failed == 0 {
        println!("\nAll tests passed!");
    }

    Ok(failed == 0)
}

// ============================================================================
// Platform/Agent Command Handlers
// ============================================================================

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let client = Client::new();

    match cli.command {
        Commands::Eval {
            ref policy,
            ref data,
            ref principal,
            ref action,
            ref resource,
            timing,
        } => handle_eval(policy, data, principal, action, resource, timing)?,

        Commands::Compile {
            ref input,
            ref output,
            optimize,
            info,
        } => handle_compile(input, output, optimize, info)?,

        Commands::Validate {
            ref policy,
            ref data,
            verbose,
        } => handle_validate(policy, data.as_deref(), verbose)?,

        Commands::Check {
            ref policy,
            ref input,
            ref data,
            ref principal,
            ref action,
            ref resource,
            ref format,
        } => handle_check(
            policy,
            input,
            data.as_deref(),
            principal.as_deref(),
            action,
            resource.as_deref(),
            format,
        )?,

        Commands::Keygen {
            ref algorithm,
            ref key_id,
        } => handle_keygen(algorithm, key_id)?,

        Commands::Decisions { ref action } => match action {
            DecisionsAction::Keygen => handle_decisions_keygen()?,
            DecisionsAction::Decrypt { key, input } => handle_decisions_decrypt(key, input)?,
        },

        Commands::Library { ref action } => match action {
            LibraryAction::List { path } => library::list(path.as_deref())?,
            LibraryAction::Show { id, path } => library::show(id, path.as_deref())?,
            LibraryAction::Run { id, path } => library::run(id.as_deref(), path.as_deref())?,
        },

        Commands::Policy { ref action } => handle_policy_action(action, &cli, &client).await?,
        Commands::Agent { ref action } => handle_agent_action(action, &cli, &client).await?,
        Commands::Status => handle_status(&cli, &client).await?,
        Commands::Demo => handle_demo(&cli, &client).await?,
        Commands::Benchmark { requests } => handle_benchmark(&cli, &client, requests).await?,

        Commands::ValidateData {
            ref file,
            check_ebpf,
            ref format,
            ref custom_schemas,
        } => handle_validate_data(file, check_ebpf, format, custom_schemas.as_deref())?,

        Commands::AnalyzePolicy {
            ref file,
            check_ebpf,
            show_recommendations,
            ref format,
        } => handle_analyze_policy(file, check_ebpf, show_recommendations, format)?,

        Commands::Bundle { ref action } => handle_bundle_action(action, &cli, &client).await?,

        Commands::Management {
            ref action,
            ref management_url,
            ref api_key,
        } => handle_management_action(action, management_url, api_key.as_deref(), &client).await?,

        Commands::Test {
            ref policy,
            ref data,
            ref principal,
            ref action,
            ref resource,
            ref expect,
            verbose,
        } => {
            let passed = handle_test(policy, data, principal, action, resource, expect, verbose)?;
            if !passed {
                std::process::exit(1);
            }
        }

        Commands::TestSuite {
            ref file,
            verbose,
            fail_fast,
        } => {
            let all_passed = handle_test_suite(file, verbose, fail_fast)?;
            if !all_passed {
                std::process::exit(1);
            }
        }
    }

    Ok(())
}

/// Handle bundle management commands
async fn handle_bundle_action(
    action: &BundleAction,
    cli: &Cli,
    client: &Client,
) -> anyhow::Result<()> {
    match action {
        BundleAction::Info { file } => {
            println!("📦 Bundle Information\n");

            // Read bundle file
            let bundle_bytes =
                fs::read(file).map_err(|e| anyhow::anyhow!("❌ Failed to read bundle: {}", e))?;

            // Parse bundle
            let bundle = PolicyBundle::from_bytes(&bundle_bytes)
                .map_err(|e| anyhow::anyhow!("❌ Invalid bundle format: {:?}", e))?;

            println!("═══════════════════════════════════════════════════════");
            println!("📋 Metadata:");
            println!("   • Policy Name: {}", bundle.metadata.policy_name);
            println!(
                "   • Version: {}",
                bundle
                    .metadata
                    .policy_version
                    .as_deref()
                    .unwrap_or("unknown")
            );
            println!("   • Format Version: {}", bundle.metadata.version);
            println!("   • Compiled At: {}", bundle.metadata.compiled_at);
            println!("   • Checksum: {:x}", bundle.metadata.source_checksum);
            println!();
            println!("📊 Policy:");
            println!("   • Rules: {}", bundle.policy.rules.len());
            println!(
                "   • Default Decision: {:?}",
                bundle.policy.default_decision
            );
            println!();
            println!("📝 Rules:");
            for (i, rule) in bundle.policy.rules.iter().enumerate() {
                println!("   {}. {} → {:?}", i + 1, rule.name, rule.decision);
            }
            println!("═══════════════════════════════════════════════════════");
        }

        BundleAction::Deploy { file, data, force } => {
            println!("🚀 Deploying Bundle to Agent\n");

            // Optionally load data first
            if let Some(data_path) = data {
                println!("1️⃣  Loading entity data: {}", data_path);
                let data_content = fs::read_to_string(data_path)
                    .map_err(|e| anyhow::anyhow!("❌ Failed to read data file: {}", e))?;

                let response = client
                    .post(format!("{}/api/v1/data", cli.agent_url))
                    .json(&serde_json::json!({ "data": data_content }))
                    .send()
                    .await?;

                if response.status().is_success() {
                    let result: Value = response.json().await?;
                    println!(
                        "   ✅ Loaded {} entities",
                        result.get("entities_loaded").unwrap_or(&json!(0))
                    );
                } else {
                    anyhow::bail!("❌ Failed to load data: {}", response.status());
                }
                println!();
            }

            // Read and deploy bundle
            println!(
                "{}  Deploying bundle: {}",
                if data.is_some() { "2️⃣" } else { "1️⃣" },
                file
            );
            let bundle_bytes =
                fs::read(file).map_err(|e| anyhow::anyhow!("❌ Failed to read bundle: {}", e))?;

            // Parse bundle for info display
            let bundle = PolicyBundle::from_bytes(&bundle_bytes)
                .map_err(|e| anyhow::anyhow!("❌ Invalid bundle format: {:?}", e))?;

            println!("   • Policy: {}", bundle.metadata.policy_name);
            println!(
                "   • Version: {}",
                bundle
                    .metadata
                    .policy_version
                    .as_deref()
                    .unwrap_or("unknown")
            );
            println!("   • Rules: {}", bundle.policy.rules.len());

            // Send to agent
            let response = client
                .post(format!("{}/api/v1/bundles/deploy", cli.agent_url))
                .json(&serde_json::json!({
                    "bundle": bundle_bytes,
                    "version": bundle.metadata.policy_version.as_deref().unwrap_or("1.0.0"),
                    "force": force
                }))
                .send()
                .await?;

            if response.status().is_success() {
                let result: Value = response.json().await?;
                println!();
                println!("✅ Bundle deployed successfully!");
                println!(
                    "   • Policy ID: {}",
                    result.get("policy_id").unwrap().as_str().unwrap()
                );
                println!(
                    "   • Version: {}",
                    result.get("version").unwrap().as_str().unwrap()
                );
                println!(
                    "   • Hash: {}",
                    result.get("bundle_hash").unwrap().as_str().unwrap()
                );
            } else {
                let error_text = response.text().await?;
                anyhow::bail!("❌ Deployment failed: {}", error_text);
            }
        }

        BundleAction::Rollback { policy_id, version } => {
            println!(
                "⏪ Rolling back policy {} to version {}\n",
                policy_id, version
            );
            println!("⚠️  Rollback API not yet implemented on agent");
            // TODO: Implement rollback endpoint on agent
        }

        BundleAction::Versions { policy_id } => {
            println!("📜 Version history for policy {}\n", policy_id);

            let response = client
                .get(format!(
                    "{}/api/v1/policies/{}/versions",
                    cli.agent_url, policy_id
                ))
                .send()
                .await?;

            if response.status().is_success() {
                let result: Value = response.json().await?;
                let versions = result.get("versions").and_then(|v| v.as_array());

                if let Some(versions) = versions {
                    if versions.is_empty() {
                        println!("No version history found for this policy.");
                    } else {
                        println!("┌──────────┬─────────────────────────────┬──────────────────────────────────┐");
                        println!("│ Version  │ Deployed At                 │ Bundle Hash                      │");
                        println!("├──────────┼─────────────────────────────┼──────────────────────────────────┤");
                        for v in versions {
                            println!(
                                "│ {:8} │ {:27} │ {:32}…│",
                                v.get("version").and_then(|v| v.as_str()).unwrap_or("?"),
                                v.get("deployed_at").and_then(|v| v.as_str()).unwrap_or("?"),
                                &v.get("bundle_hash")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .chars()
                                    .take(32)
                                    .collect::<String>()
                            );
                        }
                        println!("└──────────┴─────────────────────────────┴──────────────────────────────────┘");
                        println!(
                            "\nTotal: {} version(s)",
                            result.get("total").unwrap_or(&json!(0))
                        );
                    }
                }
            } else {
                let error_text = response.text().await?;
                println!("❌ Failed to get versions: {}", error_text);
            }
        }

        BundleAction::Package {
            input,
            output,
            name,
            version,
        } => {
            use policy_engine::reap::{PolicyPackage, ReaperPolicy};

            println!("📦 Creating Policy Package\n");

            // Parse all input policies
            let mut policies = Vec::new();
            for path in input {
                println!("   • Parsing: {}", path);
                let policy = ReaperPolicy::from_file_auto(path)
                    .map_err(|e| anyhow::anyhow!("Failed to parse {}: {:?}", path, e))?;
                println!("     ✓ {}", policy.name());
                policies.push(policy);
            }

            // Create package
            let package_name = name.clone().unwrap_or_else(|| {
                std::path::Path::new(output)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "policy-package".to_string())
            });

            // Convert ReaperPolicy to Policy AST
            // Note: We need to compile_to_bundle and load to get the AST, but PolicyPackage takes Policy directly
            // For now, let's parse the files directly to get the AST
            let mut policy_asts = Vec::new();
            for path in input {
                let content = fs::read_to_string(path)?;
                let ext = std::path::Path::new(path)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("");

                let ast = match ext {
                    "reap" => policy_engine::reap::ReapParser::parse(&content)
                        .map_err(|e| anyhow::anyhow!("Parse error: {:?}", e))?,
                    "yaml" | "yml" => {
                        let yaml = policy_engine::reap::YamlPolicy::from_yaml(&content)
                            .map_err(|e| anyhow::anyhow!("Parse error: {:?}", e))?;
                        yaml.to_ast()
                            .map_err(|e| anyhow::anyhow!("Conversion error: {:?}", e))?
                    }
                    "json" => {
                        let yaml = policy_engine::reap::YamlPolicy::from_json(&content)
                            .map_err(|e| anyhow::anyhow!("Parse error: {:?}", e))?;
                        yaml.to_ast()
                            .map_err(|e| anyhow::anyhow!("Conversion error: {:?}", e))?
                    }
                    _ => anyhow::bail!("Unsupported file extension: {}", ext),
                };
                policy_asts.push(ast);
            }

            let package = PolicyPackage::new(package_name.clone(), version.clone(), policy_asts);

            // Write package
            let bytes = package
                .to_bytes()
                .map_err(|e| anyhow::anyhow!("Failed to serialize: {:?}", e))?;
            fs::write(output, &bytes)?;

            println!();
            println!("═══════════════════════════════════════════════════════");
            println!("✅ Package Created Successfully!");
            println!();
            println!("📋 Package Metadata:");
            println!("   • Name: {}", package_name);
            println!("   • Version: {}", version);
            println!("   • Policies: {}", package.metadata.policy_count);
            println!("   • Total Rules: {}", package.hints.total_rules);
            println!();
            println!("🔧 Optimization Hints:");
            println!(
                "   • Strings to pre-intern: {}",
                package.hints.strings_to_intern.len()
            );
            println!(
                "   • Regex patterns to cache: {}",
                package.hints.regex_patterns.len()
            );
            println!();
            println!("📦 Output: {} ({} bytes)", output, bytes.len());
            println!("═══════════════════════════════════════════════════════");
        }
    }
    Ok(())
}

async fn handle_policy_action(
    action: &PolicyAction,
    cli: &Cli,
    client: &Client,
) -> anyhow::Result<()> {
    match action {
        PolicyAction::List => {
            println!("📋 Listing policies from Platform...");
            let response = client
                .get(format!("{}/api/v1/policies", cli.platform_url))
                .send()
                .await?;

            let policies: Value = response.json().await?;
            if let Some(policies_array) = policies.get("policies").and_then(|p| p.as_array()) {
                println!("┌─────────────────────────────────────────┬──────────────────┬─────────┬──────────────────────┐");
                println!("│ Policy ID                               │ Name             │ Version │ Rules Count          │");
                println!("├─────────────────────────────────────────┼──────────────────┼─────────┼──────────────────────┤");

                for policy in policies_array {
                    let id = policy.get("id").and_then(|v| v.as_str()).unwrap_or("N/A");
                    let name = policy.get("name").and_then(|v| v.as_str()).unwrap_or("N/A");
                    let version = policy.get("version").and_then(|v| v.as_u64()).unwrap_or(0);
                    let rules_count = policy
                        .get("rules")
                        .and_then(|r| r.as_array())
                        .map(|a| a.len())
                        .unwrap_or(0);

                    println!(
                        "│ {:<39} │ {:<16} │ {:<7} │ {:<20} │",
                        &id[..std::cmp::min(39, id.len())],
                        &name[..std::cmp::min(16, name.len())],
                        version,
                        rules_count
                    );
                }

                println!("└─────────────────────────────────────────┴──────────────────┴─────────┴──────────────────────┘");
                println!("Total policies: {}", policies_array.len());
            } else {
                println!("No policies found");
            }
        }
        PolicyAction::Create {
            name,
            action,
            resource,
            description,
        } => {
            println!("➕ Creating policy: {}", name);

            let request_body = json!({
                "name": name,
                "description": description.clone().unwrap_or_else(|| "Policy created via CLI".to_string()),
                "rules": [{
                    "action": action,
                    "resource": resource,
                    "conditions": []
                }]
            });

            let response = client
                .post(format!("{}/api/v1/policies", cli.platform_url))
                .json(&request_body)
                .send()
                .await?;

            let result: Value = response.json().await?;
            if let Some(policy) = result.get("policy") {
                println!("✅ Policy created successfully!");
                println!("   ID: {}", policy.get("id").unwrap().as_str().unwrap());
                println!("   Name: {}", policy.get("name").unwrap().as_str().unwrap());
                println!(
                    "   Version: {}",
                    policy.get("version").unwrap().as_u64().unwrap()
                );
            } else if let Some(error) = result.get("error") {
                println!("❌ Failed to create policy: {}", error.as_str().unwrap());
            }
        }
        PolicyAction::Update {
            id,
            name,
            action,
            resource,
            description,
        } => {
            println!("✏️  Updating policy: {}", id);

            let mut update_body = json!({});
            if let Some(n) = name {
                update_body["name"] = json!(n);
            }
            if let Some(d) = description {
                update_body["description"] = json!(d);
            }
            if action.is_some() || resource.is_some() {
                update_body["rules"] = json!([{
                    "action": action.clone().unwrap_or("allow".to_string()),
                    "resource": resource.clone().unwrap_or("*".to_string()),
                    "conditions": []
                }]);
            }

            let response = client
                .put(format!("{}/api/v1/policies/{}", cli.platform_url, id))
                .json(&update_body)
                .send()
                .await?;

            let result: Value = response.json().await?;
            if let Some(policy) = result.get("policy") {
                println!("✅ Policy updated successfully!");
                println!(
                    "   Version: {}",
                    policy.get("version").unwrap().as_u64().unwrap()
                );
                println!("   🔥 Hot-swapped with zero downtime");
            } else if let Some(error) = result.get("error") {
                println!("❌ Failed to update policy: {}", error.as_str().unwrap());
            }
        }
        PolicyAction::Delete { id } => {
            println!("🗑️  Deleting policy: {}", id);

            let response = client
                .delete(format!("{}/api/v1/policies/{}", cli.platform_url, id))
                .send()
                .await?;

            let result: Value = response.json().await?;
            if result.get("status").and_then(|s| s.as_str()) == Some("deleted") {
                println!("✅ Policy deleted successfully!");
            } else if let Some(error) = result.get("error") {
                println!("❌ Failed to delete policy: {}", error.as_str().unwrap());
            }
        }
        PolicyAction::Deploy { id, verify } => {
            println!("🚀 Deploying policy {} to agents...", id);

            // First, get the policy from platform
            let policy_response = client
                .get(format!("{}/api/v1/policies/{}", cli.platform_url, id))
                .send()
                .await?;

            let policy_data: Value = policy_response.json().await?;
            if let Some(policy) = policy_data.get("policy") {
                let deploy_request = json!({
                    "policy_id": id,
                    "name": policy.get("name").unwrap(),
                    "description": policy.get("description").unwrap(),
                    "rules": policy.get("rules").unwrap()
                });

                let deploy_response = client
                    .post(format!("{}/api/v1/policies/deploy", cli.agent_url))
                    .json(&deploy_request)
                    .send()
                    .await?;

                let result: Value = deploy_response.json().await?;
                if result.get("status").and_then(|s| s.as_str()) == Some("deployed") {
                    println!("✅ Policy deployed successfully to agent!");
                    println!("   🔥 Zero-downtime deployment completed");

                    if *verify {
                        println!("🔍 Verifying deployment...");
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

                        // Check agent has the policy
                        let agent_policies = client
                            .get(format!("{}/api/v1/policies", cli.agent_url))
                            .send()
                            .await?;

                        let agent_data: Value = agent_policies.json().await?;
                        if let Some(policies) =
                            agent_data.get("policies").and_then(|p| p.as_array())
                        {
                            let found = policies.iter().any(|p| {
                                p.get("id").and_then(|id_val| id_val.as_str()) == Some(id)
                            });

                            if found {
                                println!("✅ Verification successful - policy is active on agent");
                            } else {
                                println!("⚠️  Verification failed - policy not found on agent");
                            }
                        }
                    }
                } else if let Some(error) = result.get("error") {
                    println!("❌ Failed to deploy policy: {}", error.as_str().unwrap());
                }
            } else {
                println!("❌ Policy not found: {}", id);
            }
        }
        PolicyAction::Evaluate {
            policy_id,
            policy_name,
            resource,
            action,
        } => {
            println!("⚡ Evaluating policy...");

            let mut eval_request = json!({
                "resource": resource,
                "action": action,
                "context": {}
            });

            if let Some(id) = policy_id {
                eval_request["policy_id"] = json!(id);
            } else if let Some(name) = policy_name {
                eval_request["policy_name"] = json!(name);
            }

            let start_time = Instant::now();
            let response = client
                .post(format!("{}/api/v1/messages", cli.agent_url))
                .json(&eval_request)
                .send()
                .await?;
            let total_time = start_time.elapsed();

            let result: Value = response.json().await?;
            if let Some(decision) = result.get("decision") {
                let eval_time_micros = result
                    .get("evaluation_time_microseconds")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);

                println!("✅ Policy Evaluation Result:");
                println!("   Decision: {}", decision.as_str().unwrap());
                println!("   Evaluation Time: {:.3} μs", eval_time_micros);
                println!(
                    "   Total Time: {:.3} μs",
                    total_time.as_nanos() as f64 / 1000.0
                );
                println!(
                    "   Policy ID: {}",
                    result.get("policy_id").unwrap().as_str().unwrap()
                );

                if eval_time_micros < 1.0 {
                    println!("   🚀 Sub-microsecond performance achieved!");
                }
            } else if let Some(error) = result.get("error") {
                println!("❌ Evaluation failed: {}", error.as_str().unwrap());
            }
        }
    }
    Ok(())
}

async fn handle_agent_action(
    action: &AgentAction,
    cli: &Cli,
    client: &Client,
) -> anyhow::Result<()> {
    match action {
        AgentAction::List => {
            println!("🤖 Listing agents from Platform...");
            let response = client
                .get(format!("{}/api/v1/agents", cli.platform_url))
                .send()
                .await?;

            let result: Value = response.json().await?;
            if let Some(message) = result.get("message") {
                println!("ℹ️  {}", message.as_str().unwrap());
            }
            println!("Total agents: {}", result.get("total").unwrap_or(&json!(0)));
        }
        AgentAction::Show { id } => {
            println!("🔍 Showing agent: {}", id);
            println!("ℹ️  Agent details will be implemented in next iteration");
        }
        AgentAction::Health => {
            println!("🏥 Checking agent health...");
            let response = client
                .get(format!("{}/health", cli.agent_url))
                .send()
                .await?;

            if response.status().is_success() {
                let health: Value = response.json().await?;
                println!("✅ Agent is healthy");
                println!(
                    "   Service: {}",
                    health.get("service").unwrap().as_str().unwrap()
                );
                println!(
                    "   Version: {}",
                    health.get("version").unwrap().as_str().unwrap()
                );
                if let Some(capabilities) = health.get("capabilities").and_then(|c| c.as_array()) {
                    println!(
                        "   Capabilities: {}",
                        capabilities
                            .iter()
                            .map(|c| c.as_str().unwrap_or("unknown"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
            } else {
                println!("❌ Agent is unhealthy");
            }
        }
        AgentAction::Metrics => {
            println!("📊 Fetching agent metrics...");
            let response = client
                .get(format!("{}/metrics", cli.agent_url))
                .send()
                .await?;

            let metrics: Value = response.json().await?;
            println!("🎯 Agent Performance Metrics:");

            if let Some(perf) = metrics.get("performance") {
                println!(
                    "   Requests Processed: {}",
                    perf.get("requests_processed").unwrap_or(&json!(0))
                );
                println!(
                    "   Avg Evaluation Time: {:.3} μs",
                    perf.get("average_evaluation_time_microseconds")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0)
                );
                println!(
                    "   Target Time: {:.3} μs",
                    perf.get("target_evaluation_time_microseconds")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(1.0)
                );
            }

            if let Some(policies) = metrics.get("policies") {
                println!(
                    "   Policies Loaded: {}",
                    policies.get("total_loaded").unwrap_or(&json!(0))
                );
                println!(
                    "   Has Default: {}",
                    policies.get("has_default").unwrap_or(&json!(false))
                );
            }

            if let Some(cache) = metrics.get("cache") {
                println!(
                    "   Cache Hit Rate: {:.1}%",
                    cache
                        .get("hit_rate")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0)
                );
            }
        }
    }
    Ok(())
}

async fn handle_status(cli: &Cli, client: &Client) -> anyhow::Result<()> {
    println!("📊 Reaper Platform Status");
    println!();

    // Check Platform
    print!("🎯 Platform ({})... ", cli.platform_url);
    match client
        .get(format!("{}/health", cli.platform_url))
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => {
            println!("✅ Healthy");

            // Get platform metrics
            if let Ok(metrics_response) = client
                .get(format!("{}/metrics", cli.platform_url))
                .send()
                .await
            {
                if let Ok(metrics) = metrics_response.json::<Value>().await {
                    if let Some(policies) = metrics.get("policies") {
                        println!(
                            "   Policies: {}",
                            policies.get("total").unwrap_or(&json!(0))
                        );
                    }
                    if let Some(deployments) = metrics.get("deployments") {
                        println!(
                            "   Success Rate: {:.1}%",
                            deployments
                                .get("success_rate")
                                .and_then(|v| v.as_f64())
                                .unwrap_or(100.0)
                        );
                    }
                }
            }
        }
        _ => println!("❌ Unhealthy or unreachable"),
    }

    // Check Agent
    print!("🎯 Agent ({})... ", cli.agent_url);
    match client.get(format!("{}/health", cli.agent_url)).send().await {
        Ok(response) if response.status().is_success() => {
            println!("✅ Healthy");

            // Get agent metrics
            if let Ok(metrics_response) = client
                .get(format!("{}/metrics", cli.agent_url))
                .send()
                .await
            {
                if let Ok(metrics) = metrics_response.json::<Value>().await {
                    if let Some(perf) = metrics.get("performance") {
                        println!(
                            "   Avg Latency: {:.3} μs",
                            perf.get("average_evaluation_time_microseconds")
                                .and_then(|v| v.as_f64())
                                .unwrap_or(0.0)
                        );
                    }
                }
            }
        }
        _ => println!("❌ Unhealthy or unreachable"),
    }

    println!();
    Ok(())
}

async fn handle_demo(cli: &Cli, client: &Client) -> anyhow::Result<()> {
    println!("🎬 Reaper Platform Demo - Policy Definition & Storage");
    println!("═══════════════════════════════════════════════════════");
    println!();

    // Step 1: Check services are running
    println!("1️⃣  Checking services...");
    handle_status(cli, client).await?;
    println!();

    // Step 2: Create a demo policy
    println!("2️⃣  Creating demo policy...");
    let policy_name = format!("demo-policy-{}", &Uuid::new_v4().to_string()[..8]);

    let create_request = json!({
        "name": policy_name,
        "description": "Demo policy showcasing hot-swapping capabilities",
        "rules": [{
            "action": "allow",
            "resource": "demo-resource",
            "conditions": []
        }]
    });

    let create_response = client
        .post(format!("{}/api/v1/policies", cli.platform_url))
        .json(&create_request)
        .send()
        .await?;

    let policy_result: Value = create_response.json().await?;
    let policy_id = policy_result
        .get("policy")
        .unwrap()
        .get("id")
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();

    println!("   ✅ Created policy: {}", policy_name);
    println!("   📝 Policy ID: {}", policy_id);
    println!();

    // Step 3: Deploy to agent
    println!("3️⃣  Hot-swapping policy to agent...");
    let policy_data = policy_result.get("policy").unwrap();
    let deploy_request = json!({
        "policy_id": policy_id,
        "name": policy_data.get("name").unwrap(),
        "description": policy_data.get("description").unwrap(),
        "rules": policy_data.get("rules").unwrap()
    });

    let _deploy_response = client
        .post(format!("{}/api/v1/policies/deploy", cli.agent_url))
        .json(&deploy_request)
        .send()
        .await?;

    println!("   🔥 Hot-swap completed with zero downtime");
    println!();

    // Step 4: Demonstrate sub-microsecond evaluation
    println!("4️⃣  Testing sub-microsecond policy evaluation...");
    let eval_request = json!({
        "policy_id": policy_id,
        "resource": "demo-resource",
        "action": "read",
        "context": {}
    });

    let mut total_time = 0.0;
    let iterations = 5;

    for i in 1..=iterations {
        let start = Instant::now();
        let eval_response = client
            .post(format!("{}/api/v1/messages", cli.agent_url))
            .json(&eval_request)
            .send()
            .await?;
        let request_time = start.elapsed();

        let eval_result: Value = eval_response.json().await?;
        let eval_time = eval_result
            .get("evaluation_time_microseconds")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        total_time += eval_time;

        println!(
            "   Test {}: {:.3} μs evaluation, {:.3} μs total",
            i,
            eval_time,
            request_time.as_nanos() as f64 / 1000.0
        );
    }

    let avg_time = total_time / iterations as f64;
    println!("   📊 Average evaluation time: {:.3} μs", avg_time);

    if avg_time < 1.0 {
        println!("   🚀 Sub-microsecond target achieved!");
    }
    println!();

    // Step 5: Update policy (hot-swap)
    println!("5️⃣  Hot-swapping policy update...");
    let update_request = json!({
        "description": "Updated demo policy - hot-swapped!",
        "rules": [{
            "action": "deny",
            "resource": "demo-resource",
            "conditions": ["updated"]
        }]
    });

    let _update_response = client
        .put(format!(
            "{}/api/v1/policies/{}",
            cli.platform_url, policy_id
        ))
        .json(&update_request)
        .send()
        .await?;

    println!("   🔄 Policy updated to version 2");
    println!("   🔥 Hot-swapped with zero service interruption");
    println!();

    // Step 6: Clean up
    println!("6️⃣  Cleaning up demo policy...");
    let _delete_response = client
        .delete(format!(
            "{}/api/v1/policies/{}",
            cli.platform_url, policy_id
        ))
        .send()
        .await?;

    println!("   🗑️  Demo policy deleted");
    println!();

    println!("✨ Demo completed successfully!");
    println!("   🎯 Key Features Demonstrated:");
    println!("   • Policy creation and storage");
    println!("   • Zero-downtime hot-swapping");
    println!("   • Sub-microsecond policy evaluation");
    println!("   • Atomic policy updates");
    println!("   • Memory-efficient storage");

    Ok(())
}

async fn handle_benchmark(cli: &Cli, client: &Client, requests: usize) -> anyhow::Result<()> {
    println!("🏃 Running Reaper Performance Benchmark");
    println!("Target: {} requests", requests);
    println!();

    // Create a test policy
    println!("Setting up benchmark policy...");
    let policy_request = json!({
        "name": "benchmark-policy",
        "description": "High-performance benchmark policy",
        "rules": [{
            "action": "allow",
            "resource": "*",
            "conditions": []
        }]
    });

    let policy_response = client
        .post(format!("{}/api/v1/policies", cli.platform_url))
        .json(&policy_request)
        .send()
        .await?;

    let policy_result: Value = policy_response.json().await?;
    let policy_id = policy_result
        .get("policy")
        .unwrap()
        .get("id")
        .unwrap()
        .as_str()
        .unwrap();

    // Deploy to agent
    let deploy_request = json!({
        "policy_id": policy_id,
        "name": "benchmark-policy",
        "description": "Benchmark policy",
        "rules": [{
            "action": "allow",
            "resource": "*",
            "conditions": []
        }]
    });

    let _deploy_response = client
        .post(format!("{}/api/v1/policies/deploy", cli.agent_url))
        .json(&deploy_request)
        .send()
        .await?;

    println!("✅ Benchmark policy deployed");
    println!();

    // Run benchmark
    println!("🚀 Starting benchmark...");
    let eval_request = json!({
        "policy_id": policy_id,
        "resource": "benchmark-resource",
        "action": "read",
        "context": {}
    });

    let mut eval_times = Vec::new();
    let start_time = Instant::now();

    for i in 0..requests {
        let _eval_start = Instant::now();
        let response = client
            .post(format!("{}/api/v1/messages", cli.agent_url))
            .json(&eval_request)
            .send()
            .await?;

        let result: Value = response.json().await?;
        if let Some(eval_time) = result
            .get("evaluation_time_microseconds")
            .and_then(|v| v.as_f64())
        {
            eval_times.push(eval_time);
        }

        if i % (requests / 10) == 0 && i > 0 {
            println!("  Progress: {}/{} requests", i, requests);
        }
    }

    let total_duration = start_time.elapsed();

    // Calculate statistics
    eval_times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let min_time = eval_times.first().copied().unwrap_or(0.0);
    let max_time = eval_times.last().copied().unwrap_or(0.0);
    let avg_time = eval_times.iter().sum::<f64>() / eval_times.len() as f64;
    let p50 = eval_times[eval_times.len() / 2];
    let p95 = eval_times[(eval_times.len() as f64 * 0.95) as usize];
    let p99 = eval_times[(eval_times.len() as f64 * 0.99) as usize];

    let throughput = requests as f64 / total_duration.as_secs_f64();

    println!();
    println!("📊 Benchmark Results:");
    println!("   Requests: {}", requests);
    println!("   Duration: {:.2}s", total_duration.as_secs_f64());
    println!("   Throughput: {:.0} req/s", throughput);
    println!();
    println!("   Policy Evaluation Latency (μs):");
    println!("   • Min:  {:.3}", min_time);
    println!("   • Avg:  {:.3}", avg_time);
    println!("   • P50:  {:.3}", p50);
    println!("   • P95:  {:.3}", p95);
    println!("   • P99:  {:.3}", p99);
    println!("   • Max:  {:.3}", max_time);
    println!();

    if p99 < 1.0 {
        println!("🎯 Sub-microsecond P99 latency achieved!");
    } else {
        println!("⚠️  P99 latency above 1μs target");
    }

    if throughput > 100_000.0 {
        println!("🚀 High-throughput target exceeded!");
    }

    // Clean up
    let _cleanup = client
        .delete(format!(
            "{}/api/v1/policies/{}",
            cli.platform_url, policy_id
        ))
        .send()
        .await;

    Ok(())
}

// ============================================================================
// Management Server Command Handlers
// ============================================================================

async fn handle_management_action(
    action: &ManagementAction,
    management_url: &str,
    api_key: Option<&str>,
    client: &Client,
) -> anyhow::Result<()> {
    // Build request with optional API key
    let build_request = |client: &Client, url: &str| {
        let req = client.get(url);
        if let Some(key) = api_key {
            req.header("X-API-Key", key)
        } else {
            req
        }
    };

    let build_post = |client: &Client, url: &str| {
        let req = client.post(url);
        if let Some(key) = api_key {
            req.header("X-API-Key", key)
        } else {
            req
        }
    };

    match action {
        ManagementAction::Health => {
            println!("🏥 Checking management server health...");
            let response = client
                .get(format!("{}/health", management_url))
                .send()
                .await?;

            if response.status().is_success() {
                let health: Value = response.json().await?;
                println!("✅ Management server is healthy");
                println!(
                    "   Service: {}",
                    health
                        .get("service")
                        .and_then(|v| v.as_str())
                        .unwrap_or("reaper-management")
                );
                println!(
                    "   Version: {}",
                    health
                        .get("version")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                );
                if let Some(status) = health.get("status").and_then(|v| v.as_str()) {
                    println!("   Status: {}", status);
                }
            } else {
                println!(
                    "❌ Management server is unhealthy (status: {})",
                    response.status()
                );
            }
        }

        ManagementAction::Orgs => {
            println!("🏢 Listing organizations...");
            let response = build_request(client, &format!("{}/api/v1/orgs", management_url))
                .send()
                .await?;

            if response.status().is_success() {
                let result: Value = response.json().await?;
                if let Some(orgs) = result.get("organizations").and_then(|v| v.as_array()) {
                    if orgs.is_empty() {
                        println!("No organizations found.");
                    } else {
                        println!("┌──────────────────────────────────────┬────────────────────┬─────────┐");
                        println!("│ ID                                   │ Name               │ Status  │");
                        println!("├──────────────────────────────────────┼────────────────────┼─────────┤");
                        for org in orgs {
                            println!(
                                "│ {:<36} │ {:<18} │ {:<7} │",
                                org.get("id").and_then(|v| v.as_str()).unwrap_or("?"),
                                &org.get("name").and_then(|v| v.as_str()).unwrap_or("?")
                                    [..std::cmp::min(
                                        18,
                                        org.get("name")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("?")
                                            .len()
                                    )],
                                org.get("status").and_then(|v| v.as_str()).unwrap_or("?")
                            );
                        }
                        println!("└──────────────────────────────────────┴────────────────────┴─────────┘");
                        println!("\nTotal: {} organization(s)", orgs.len());
                    }
                }
            } else {
                let error_text = response.text().await?;
                println!("❌ Failed to list organizations: {}", error_text);
            }
        }

        ManagementAction::Sources { org } => {
            println!("📁 Listing policy sources for org {}...", org);
            let response = build_request(
                client,
                &format!("{}/api/v1/orgs/{}/sources", management_url, org),
            )
            .send()
            .await?;

            if response.status().is_success() {
                let result: Value = response.json().await?;
                if let Some(sources) = result.get("sources").and_then(|v| v.as_array()) {
                    if sources.is_empty() {
                        println!("No policy sources found.");
                    } else {
                        println!("┌──────────────────────────────────────┬────────────────────┬──────────┬─────────┐");
                        println!("│ ID                                   │ Name               │ Type     │ Status  │");
                        println!("├──────────────────────────────────────┼────────────────────┼──────────┼─────────┤");
                        for source in sources {
                            println!(
                                "│ {:<36} │ {:<18} │ {:<8} │ {:<7} │",
                                source.get("id").and_then(|v| v.as_str()).unwrap_or("?"),
                                &source.get("name").and_then(|v| v.as_str()).unwrap_or("?")
                                    [..std::cmp::min(
                                        18,
                                        source
                                            .get("name")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("?")
                                            .len()
                                    )],
                                source
                                    .get("source_type")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("?"),
                                source.get("status").and_then(|v| v.as_str()).unwrap_or("?")
                            );
                        }
                        println!("└──────────────────────────────────────┴────────────────────┴──────────┴─────────┘");
                        println!("\nTotal: {} source(s)", sources.len());
                    }
                }
            } else {
                let error_text = response.text().await?;
                println!("❌ Failed to list sources: {}", error_text);
            }
        }

        ManagementAction::Bundles { org, promoted } => {
            println!("📦 Listing bundles for org {}...", org);
            let url = if *promoted {
                format!(
                    "{}/api/v1/orgs/{}/bundles?status=promoted",
                    management_url, org
                )
            } else {
                format!("{}/api/v1/orgs/{}/bundles", management_url, org)
            };
            let response = build_request(client, &url).send().await?;

            if response.status().is_success() {
                let result: Value = response.json().await?;
                if let Some(bundles) = result.get("bundles").and_then(|v| v.as_array()) {
                    if bundles.is_empty() {
                        println!("No bundles found.");
                    } else {
                        println!("┌──────────────────────────────────────┬────────────────────┬──────────┬──────────┐");
                        println!("│ ID                                   │ Name               │ Status   │ Policies │");
                        println!("├──────────────────────────────────────┼────────────────────┼──────────┼──────────┤");
                        for bundle in bundles {
                            println!(
                                "│ {:<36} │ {:<18} │ {:<8} │ {:<8} │",
                                bundle.get("id").and_then(|v| v.as_str()).unwrap_or("?"),
                                &bundle.get("name").and_then(|v| v.as_str()).unwrap_or("?")
                                    [..std::cmp::min(
                                        18,
                                        bundle
                                            .get("name")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("?")
                                            .len()
                                    )],
                                bundle.get("status").and_then(|v| v.as_str()).unwrap_or("?"),
                                bundle
                                    .get("policy_count")
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(0)
                            );
                        }
                        println!("└──────────────────────────────────────┴────────────────────┴──────────┴──────────┘");
                        println!("\nTotal: {} bundle(s)", bundles.len());
                    }
                }
            } else {
                let error_text = response.text().await?;
                println!("❌ Failed to list bundles: {}", error_text);
            }
        }

        ManagementAction::BundleInfo { id } => {
            println!("📦 Bundle details for {}...", id);
            let response =
                build_request(client, &format!("{}/api/v1/bundles/{}", management_url, id))
                    .send()
                    .await?;

            if response.status().is_success() {
                let result: Value = response.json().await?;
                if let Some(bundle) = result.get("bundle") {
                    println!("═══════════════════════════════════════════════════════");
                    println!("📋 Bundle Metadata:");
                    println!(
                        "   • ID: {}",
                        bundle.get("id").and_then(|v| v.as_str()).unwrap_or("?")
                    );
                    println!(
                        "   • Name: {}",
                        bundle.get("name").and_then(|v| v.as_str()).unwrap_or("?")
                    );
                    println!(
                        "   • Status: {}",
                        bundle.get("status").and_then(|v| v.as_str()).unwrap_or("?")
                    );
                    println!(
                        "   • Policies: {}",
                        bundle
                            .get("policy_count")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0)
                    );
                    if let Some(checksum) = bundle.get("checksum").and_then(|v| v.as_str()) {
                        println!("   • Checksum: {}", checksum);
                    }
                    if let Some(size) = bundle.get("compiled_size_bytes").and_then(|v| v.as_i64()) {
                        println!("   • Size: {} bytes", size);
                    }
                    if let Some(promoted_at) = bundle.get("promoted_at").and_then(|v| v.as_str()) {
                        println!("   • Promoted At: {}", promoted_at);
                    }
                    println!(
                        "   • Created At: {}",
                        bundle
                            .get("created_at")
                            .and_then(|v| v.as_str())
                            .unwrap_or("?")
                    );
                    println!("═══════════════════════════════════════════════════════");
                }
            } else {
                let error_text = response.text().await?;
                println!("❌ Failed to get bundle: {}", error_text);
            }
        }

        ManagementAction::Agents { org } => {
            println!("🤖 Listing agents for org {}...", org);
            let response = build_request(
                client,
                &format!("{}/api/v1/orgs/{}/agents", management_url, org),
            )
            .send()
            .await?;

            if response.status().is_success() {
                let result: Value = response.json().await?;
                if let Some(agents) = result.get("agents").and_then(|v| v.as_array()) {
                    if agents.is_empty() {
                        println!("No agents registered.");
                    } else {
                        println!("┌──────────────────────────────────────┬────────────────────┬──────────┬─────────────────────┐");
                        println!("│ ID                                   │ Name               │ Status   │ Last Heartbeat      │");
                        println!("├──────────────────────────────────────┼────────────────────┼──────────┼─────────────────────┤");
                        for agent in agents {
                            let last_hb = agent
                                .get("last_heartbeat_at")
                                .and_then(|v| v.as_str())
                                .map(|s| &s[..std::cmp::min(19, s.len())])
                                .unwrap_or("never");
                            println!(
                                "│ {:<36} │ {:<18} │ {:<8} │ {:<19} │",
                                agent.get("id").and_then(|v| v.as_str()).unwrap_or("?"),
                                &agent.get("name").and_then(|v| v.as_str()).unwrap_or("?")
                                    [..std::cmp::min(
                                        18,
                                        agent
                                            .get("name")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("?")
                                            .len()
                                    )],
                                agent.get("status").and_then(|v| v.as_str()).unwrap_or("?"),
                                last_hb
                            );
                        }
                        println!("└──────────────────────────────────────┴────────────────────┴──────────┴─────────────────────┘");
                        println!("\nTotal: {} agent(s)", agents.len());
                    }
                }
            } else {
                let error_text = response.text().await?;
                println!("❌ Failed to list agents: {}", error_text);
            }
        }

        ManagementAction::Push {
            org,
            source,
            file,
            name,
            description,
        } => {
            println!("⬆️  Pushing policy to management server...");

            // Read and parse the policy file
            if !Path::new(file).exists() {
                anyhow::bail!("❌ Error: Policy file not found: {}", file);
            }

            let content = fs::read_to_string(file)
                .map_err(|e| anyhow::anyhow!("❌ Failed to read policy file: {}", e))?;

            // Detect language from extension
            let ext = Path::new(file)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            let language = match ext {
                "reap" => "reaper",
                "yaml" | "yml" => "reaper",
                "json" => "reaper",
                "cedar" => "cedar",
                _ => "reaper",
            };

            let policy_name = name.clone().unwrap_or_else(|| {
                Path::new(file)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "unnamed-policy".to_string())
            });

            // First, get the source ID
            let sources_response = build_request(
                client,
                &format!("{}/api/v1/orgs/{}/sources", management_url, org),
            )
            .send()
            .await?;

            let sources_result: Value = sources_response.json().await?;
            let source_id = sources_result
                .get("sources")
                .and_then(|v| v.as_array())
                .and_then(|sources| {
                    sources
                        .iter()
                        .find(|s| s.get("name").and_then(|n| n.as_str()) == Some(source))
                })
                .and_then(|s| s.get("id"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    anyhow::anyhow!("❌ Source '{}' not found in organization", source)
                })?;

            // Create the policy
            let create_body = json!({
                "name": policy_name,
                "description": description.clone().unwrap_or_else(|| format!("Pushed from CLI: {}", file)),
                "content": content,
                "language": language
            });

            let response = build_post(
                client,
                &format!("{}/api/v1/sources/{}/policies", management_url, source_id),
            )
            .json(&create_body)
            .send()
            .await?;

            if response.status().is_success() {
                let result: Value = response.json().await?;
                if let Some(policy) = result.get("policy") {
                    println!("✅ Policy pushed successfully!");
                    println!(
                        "   • ID: {}",
                        policy.get("id").and_then(|v| v.as_str()).unwrap_or("?")
                    );
                    println!(
                        "   • Name: {}",
                        policy.get("name").and_then(|v| v.as_str()).unwrap_or("?")
                    );
                    println!(
                        "   • Version: {}",
                        policy.get("version").and_then(|v| v.as_i64()).unwrap_or(0)
                    );
                }
            } else {
                let error_text = response.text().await?;
                println!("❌ Failed to push policy: {}", error_text);
            }
        }

        ManagementAction::Promote {
            org,
            name,
            policies,
            description,
        } => {
            println!("🚀 Creating and promoting bundle...");

            // Parse policy IDs
            let policy_ids: Vec<&str> = policies.split(',').map(|s| s.trim()).collect();

            // Create bundle
            let create_body = json!({
                "name": name,
                "description": description.clone().unwrap_or_else(|| format!("Bundle with {} policies", policy_ids.len())),
                "policy_ids": policy_ids
            });

            let response = build_post(
                client,
                &format!("{}/api/v1/orgs/{}/bundles", management_url, org),
            )
            .json(&create_body)
            .send()
            .await?;

            if !response.status().is_success() {
                let error_text = response.text().await?;
                anyhow::bail!("❌ Failed to create bundle: {}", error_text);
            }

            let result: Value = response.json().await?;
            let bundle_id = result
                .get("bundle")
                .and_then(|b| b.get("id"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("❌ Failed to get bundle ID from response"))?;

            println!("   ✅ Bundle created: {}", bundle_id);

            // Promote the bundle
            let promote_response = build_post(
                client,
                &format!("{}/api/v1/bundles/{}/promote", management_url, bundle_id),
            )
            .send()
            .await?;

            if promote_response.status().is_success() {
                println!("   ✅ Bundle promoted successfully!");
                println!("   📦 Agents will automatically pull this bundle");
            } else {
                let error_text = promote_response.text().await?;
                println!("   ⚠️  Bundle created but promotion failed: {}", error_text);
            }
        }
    }

    Ok(())
}
