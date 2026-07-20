//! Basic usage example for the Reaper SDK
//!
//! This example demonstrates:
//! 1. Creating an SDK client
//! 2. Checking agent health
//! 3. Evaluating a policy request
//! 4. Deploying a policy bundle (commented out - requires .rbb file)
//!
//! To run this example:
//! 1. Start the Reaper Agent: `cargo run --bin reaper-agent`
//! 2. Start the Reaper Platform: `cargo run --bin reaper-platform`
//! 3. Create a policy via the Platform API
//! 4. Run this example: `cargo run --example basic_usage`

use reaper_sdk::{Decision, PolicyRequest, ReaperClient, Result};
use std::collections::HashMap;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    println!("🚀 Reaper SDK - Basic Usage Example\n");

    // Create HTTP client pointing to local agent
    let client = ReaperClient::http("http://localhost:8080")?;
    println!("✅ Created SDK client for http://localhost:8080\n");

    // Check agent health
    println!("🏥 Checking agent health...");
    match client.health_check().await {
        Ok(()) => println!("✅ Agent is healthy\n"),
        Err(e) => {
            eprintln!("❌ Agent health check failed: {}", e);
            eprintln!("\n💡 Make sure the agent is running:");
            eprintln!("   cargo run --bin reaper-agent\n");
            return Err(e);
        }
    }

    // Evaluate a policy
    println!("📋 Evaluating policy...");
    let request = PolicyRequest {
        policy_id: "test-policy".to_string(),
        principal: "user:alice".to_string(),
        action: "read".to_string(),
        resource: "/api/data".to_string(),
        context: {
            let mut ctx = HashMap::new();
            ctx.insert("department".to_string(), "engineering".to_string());
            ctx.insert("clearance".to_string(), "level-3".to_string());
            ctx
        },
    };

    println!("  Policy ID: {}", request.policy_id);
    println!("  Principal: {}", request.principal);
    println!("  Action:    {}", request.action);
    println!("  Resource:  {}", request.resource);
    println!("  Context:   {:?}", request.context);

    match client.evaluate(request).await {
        Ok(response) => {
            println!("\n✅ Policy Evaluation Result:");
            println!("  Decision:  {:?}", response.decision);
            println!(
                "  Latency:   {} ns ({} µs)",
                response.latency_ns,
                response.latency_ns / 1000
            );
            println!("  Source:    {:?}", response.source);

            match response.decision {
                Decision::Allow => println!("\n🟢 ACCESS GRANTED"),
                Decision::Deny => println!("\n🔴 ACCESS DENIED"),
                // `Decision` is non_exhaustive: treat anything unknown as not
                // granted (fail closed).
                other => println!("\n🔴 ACCESS NOT GRANTED (unrecognized decision {other:?})"),
            }
        }
        Err(e) => {
            eprintln!("\n❌ Policy evaluation failed: {}", e);
            eprintln!("\n💡 Make sure you have created a policy:");
            eprintln!("   1. POST http://localhost:8081/api/v1/policies");
            eprintln!("   2. Deploy it to the agent");
            return Err(e);
        }
    }

    println!("\n📦 Bundle Deployment Example");
    println!("To deploy a policy bundle:");
    println!("  1. Create a .rbb bundle file");
    println!("  2. Load it: let bundle = std::fs::read(\"policy.rbb\").unwrap();");
    println!("  3. Deploy: client.deploy_bundle(&bundle, \"1.0.0\", false).await?;");

    /*
    // Example: Deploy a bundle (uncomment when you have a .rbb file)
    println!("\n📦 Deploying policy bundle...");
    let bundle_bytes = std::fs::read("policy.rbb")?;
    let response = client.deploy_bundle(&bundle_bytes, "1.0.0", false).await?;
    println!("✅ Bundle deployed:");
    println!("  Policy ID: {}", response.policy_id);
    println!("  Version:   {}", response.version);
    println!("  Hash:      {}", response.bundle_hash);
    */

    println!("\n🎉 Example complete!");
    Ok(())
}
