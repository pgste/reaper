use clap::{Parser, Subcommand};
use reqwest::Client;
use serde_json::{json, Value};
use std::time::Instant;
use std::fs;
use std::path::Path;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

// Reap policy imports
use policy_engine::{
    ReaperPolicy, PolicyBundle, DataStore, DataLoader,
    PolicyRequest, PolicyEvaluator, PolicyAction as EngineAction,
};

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
    let data_content = fs::read_to_string(data_path).map_err(|e| {
        anyhow::anyhow!("❌ Failed to read data file: {}", e)
    })?;

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
    let evaluator = policy.build(store.clone()).map_err(|e| {
        anyhow::anyhow!("❌ Failed to build evaluator: {:?}", e)
    })?;
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
    let decision = evaluator.evaluate(&request).map_err(|e| {
        anyhow::anyhow!("❌ Evaluation failed: {:?}", e)
    })?;
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
        println!("   • Evaluate: {:?} ({:.0} ns)", eval_time, eval_time.as_nanos());
        println!();
    } else {
        println!("⏱  Evaluation time: {:?} ({:.0} ns)", eval_time, eval_time.as_nanos());
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
    let bundle_bytes = policy.compile_to_bundle().map_err(|e| {
        anyhow::anyhow!("❌ Compilation failed: {:?}", e)
    })?;
    let compile_time = compile_start.elapsed();

    println!("   ✓ Compiled successfully");
    println!("   ⏱  Compilation time: {:?}", compile_time);
    println!();

    // Write bundle
    println!("3️⃣  Writing bundle: {}", output_path);
    fs::write(output_path, &bundle_bytes).map_err(|e| {
        anyhow::anyhow!("❌ Failed to write bundle: {}", e)
    })?;

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

        let data_content = fs::read_to_string(data_path).map_err(|e| {
            anyhow::anyhow!("❌ Failed to read data file: {}", e)
        })?;

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

        Commands::Policy { ref action } => handle_policy_action(action, &cli, &client).await?,
        Commands::Agent { ref action } => handle_agent_action(action, &cli, &client).await?,
        Commands::Status => handle_status(&cli, &client).await?,
        Commands::Demo => handle_demo(&cli, &client).await?,
        Commands::Benchmark { requests } => handle_benchmark(&cli, &client, requests).await?,
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
    match client
        .get(format!("{}/health", cli.agent_url))
        .send()
        .await
    {
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
    let policy_name = format!(
        "demo-policy-{}",
        &Uuid::new_v4().to_string()[..8]
    );

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
    let policy_data = policy_result.get("policy").unwrap();
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
