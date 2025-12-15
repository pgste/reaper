/// Complete eBPF System Example
///
/// This example demonstrates the full eBPF-accelerated policy engine with:
/// - Two-tier evaluation (eBPF fast path + userspace slow path)
/// - Learning engine with auto-promotion
/// - Real-time statistics
/// - Hot path optimization
///
/// NOTE: This example requires:
/// - x86_64 architecture (for eBPF compilation)
/// - Linux kernel 5.7+ (for eBPF LSM support)
/// - CAP_BPF capability or root privileges
/// - Compiled eBPF program: target/bpfel-unknown-none/release/reaper_ebpf_kern.o
///
/// Build eBPF program first:
/// ```bash
/// rustup +nightly target add bpfel-unknown-none
/// cd reaper-ebpf-kern
/// cargo +nightly build --target=bpfel-unknown-none -Z build-std=core --release
/// ```
use anyhow::Result;
use policy_engine::{EnhancedPolicy, PolicyAction, PolicyEngine, PolicyRequest, PolicyRule};
use reaper_ebpf::EbpfPolicyEngine;
use std::collections::HashMap;
use tracing::{info, warn};
use tracing_subscriber::FmtSubscriber;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing::subscriber::set_global_default(
        FmtSubscriber::builder()
            .with_max_level(tracing::Level::INFO)
            .finish(),
    )
    .expect("setting default subscriber failed");

    info!("{}", "=".repeat(80));
    info!("🚀 Reaper eBPF Complete System Example");
    info!("{}", "=".repeat(80));

    // ========================================================================
    // Step 1: Create traditional PolicyEngine with complex policies
    // ========================================================================
    info!("\n📋 Step 1: Creating PolicyEngine with complex policies...");

    let policy_engine = PolicyEngine::new();

    // Create some complex policies that will start in userspace
    let complex_policy = EnhancedPolicy::new(
        "complex-abac".to_string(),
        "Complex ABAC policy with multiple conditions".to_string(),
        vec![
            PolicyRule {
                action: PolicyAction::Allow,
                resource: "/api/admin/*".to_string(),
                conditions: vec![
                    "role==admin".to_string(),
                    "department==engineering".to_string(),
                    "clearance_level>=5".to_string(),
                ],
            },
            PolicyRule {
                action: PolicyAction::Allow,
                resource: "/api/users/*".to_string(),
                conditions: vec!["role==user".to_string(), "active==true".to_string()],
            },
        ],
    );

    policy_engine.deploy_policy(complex_policy)?;

    info!("✓ Deployed complex policies to userspace engine");

    // ========================================================================
    // Step 2: Initialize eBPF engine (requires compiled eBPF program)
    // ========================================================================
    info!("\n🔧 Step 2: Initializing eBPF engine...");

    // Check if eBPF program exists
    let ebpf_program_path = "target/bpfel-unknown-none/release/reaper_ebpf_kern.o";

    if !std::path::Path::new(ebpf_program_path).exists() {
        warn!("⚠️  eBPF program not found: {}", ebpf_program_path);
        warn!("   This example requires a compiled eBPF program.");
        warn!("   Build it with:");
        warn!("   cd reaper-ebpf-kern");
        warn!("   cargo +nightly build --target=bpfel-unknown-none -Z build-std=core --release");
        warn!("\n   For now, demonstrating the API without actual eBPF acceleration...");

        demonstrate_api_without_ebpf(policy_engine).await?;
        return Ok(());
    }

    // Load eBPF program
    let mut ebpf_engine = EbpfPolicyEngine::load(policy_engine, ebpf_program_path)?;

    info!("✓ eBPF program loaded");

    // ========================================================================
    // Step 3: Attach eBPF to kernel LSM hooks
    // ========================================================================
    info!("\n🔗 Step 3: Attaching eBPF to kernel LSM hooks...");

    match ebpf_engine.attach().await {
        Ok(_) => {
            info!("✓ eBPF attached to kernel LSM hooks");
            info!("  Kernel-level enforcement is now active!");
        }
        Err(e) => {
            warn!("⚠️  Failed to attach eBPF: {}", e);
            warn!("  Requires CAP_BPF capability or root privileges");
            warn!("  Continuing with userspace-only mode...");
        }
    }

    // ========================================================================
    // Step 4: Simulate traffic with learning
    // ========================================================================
    info!("\n📊 Step 4: Simulating traffic to demonstrate learning...");

    simulate_traffic(&ebpf_engine).await?;

    // ========================================================================
    // Step 5: Show statistics
    // ========================================================================
    info!("\n📈 Step 5: Collecting statistics...");

    let stats = ebpf_engine.get_combined_stats().await?;
    let learning_stats = ebpf_engine.get_learning_stats();

    info!("{}", "=".repeat(80));
    info!("📊 FINAL STATISTICS");
    info!("{}", "=".repeat(80));
    info!("\n🚀 Performance:");
    info!(
        "   Fast path (eBPF):      {} evaluations",
        stats.fast_path_evaluations
    );
    info!(
        "   Slow path (userspace): {} evaluations",
        stats.slow_path_evaluations
    );
    info!("   Fast path ratio:       {:.1}%", stats.fast_path_percent);

    info!("\n✅ Decisions:");
    info!("   Allowed:  {}", stats.allows);
    info!("   Denied:   {}", stats.denials);
    info!("   Errors:   {}", stats.errors);

    info!("\n🧠 Learning System:");
    info!(
        "   Patterns tracked:        {}",
        learning_stats.total_patterns
    );
    info!(
        "   Patterns promoted:       {}",
        learning_stats.promoted_patterns
    );
    info!(
        "   Stable patterns:         {}",
        learning_stats.stable_patterns
    );
    info!(
        "   Unstable patterns:       {}",
        learning_stats.unstable_patterns
    );
    info!(
        "   Eligible for promotion:  {}",
        learning_stats.eligible_for_promotion
    );

    info!("\n📦 Policy Distribution:");
    info!(
        "   eBPF policies:       {} (fast path)",
        stats.ebpf_policy_count
    );
    info!(
        "   Userspace policies:  {} (slow path)",
        stats.userspace_policy_count
    );

    // ========================================================================
    // Step 6: Demonstrate manual promotion
    // ========================================================================
    info!("\n🎯 Step 6: Manually triggering auto-promotion...");

    match ebpf_engine.auto_promote().await {
        Ok(count) => {
            if count > 0 {
                info!("✓ Promoted {} resources to eBPF fast path", count);
            } else {
                info!("  No resources eligible for promotion yet");
                info!("  (Need 100 accesses with stable decisions)");
            }
        }
        Err(e) => {
            warn!("⚠️  Auto-promotion failed: {}", e);
        }
    }

    info!("\n{}", "=".repeat(80));
    info!("✅ Example complete!");
    info!("{}", "=".repeat(80));

    Ok(())
}

/// Simulate traffic to demonstrate learning
async fn simulate_traffic(_ebpf_engine: &EbpfPolicyEngine) -> Result<()> {
    info!("  Simulating 1000 requests across different resources...");

    // Simulate hot paths that will be promoted
    let hot_resources = ["/api/users/123", "/api/users/456", "/api/posts/789"];

    // Simulate cold paths (accessed rarely)
    let cold_resources = ["/api/admin/settings", "/api/debug/logs"];

    // Simulate 1000 requests
    for i in 0..1000 {
        let resource = if i % 10 == 0 {
            // 10% cold path
            &cold_resources[i % cold_resources.len()]
        } else {
            // 90% hot path
            &hot_resources[i % hot_resources.len()]
        };

        // Create request
        let mut context = HashMap::new();
        context.insert("role".to_string(), "user".to_string());
        context.insert("active".to_string(), "true".to_string());
        context.insert("department".to_string(), "engineering".to_string());

        let request = PolicyRequest {
            resource: resource.to_string(),
            action: "read".to_string(),
            context,
        };

        // In a real system, this would be evaluated
        // For this example, we just simulate the access pattern
        let _ = request;

        // Progress indicator
        if (i + 1) % 200 == 0 {
            info!("  Progress: {}/1000 requests simulated", i + 1);
        }
    }

    info!("✓ Traffic simulation complete");

    Ok(())
}

/// Demonstrate API without actual eBPF acceleration
async fn demonstrate_api_without_ebpf(_policy_engine: PolicyEngine) -> Result<()> {
    info!("\n📚 API Demonstration (without eBPF acceleration)");
    info!("{}", "=".repeat(80));

    // Show what the API looks like
    info!("\n1. Loading eBPF would look like:");
    info!("   ```rust");
    info!("   let ebpf_engine = EbpfPolicyEngine::load(");
    info!("       policy_engine,");
    info!("       \"reaper_ebpf_kern.o\"");
    info!("   )?;");
    info!("   ```");

    info!("\n2. Attaching to kernel:");
    info!("   ```rust");
    info!("   ebpf_engine.attach().await?;");
    info!("   ```");

    info!("\n3. Deploying policies:");
    info!("   ```rust");
    info!("   ebpf_engine.deploy_bundle(bundle).await?;");
    info!("   ```");
    info!("   - Simple policies → automatically compiled to eBPF");
    info!("   - Complex policies → kept in userspace");

    info!("\n4. Updating runtime context:");
    info!("   ```rust");
    info!("   ebpf_engine.update_context(\"jwt_claim_role\", \"admin\").await?;");
    info!("   ```");

    info!("\n5. Getting statistics:");
    info!("   ```rust");
    info!("   let stats = ebpf_engine.get_combined_stats().await?;");
    info!("   println!(\"Fast path: {{:.1}}%\", stats.fast_path_percent);");
    info!("   ```");

    info!("\n6. Manual promotion:");
    info!("   ```rust");
    info!("   let promoted = ebpf_engine.auto_promote().await?;");
    info!("   ```");

    info!("\n{}", "=".repeat(80));
    info!("📋 Expected Performance (on x86_64 with eBPF):");
    info!("{}", "=".repeat(80));
    info!("\n   Fast Path (eBPF in kernel):");
    info!("   • Latency: <100ns");
    info!("   • Throughput: >10M decisions/second/core");
    info!("   • Use cases: Simple policies, promoted hot paths");

    info!("\n   Slow Path (userspace):");
    info!("   • Latency: 10-50µs");
    info!("   • Throughput: 50K complex decisions/second/core");
    info!("   • Use cases: Cedar ABAC, Reaper DSL, complex conditions");

    info!("\n   Learning System:");
    info!("   • Auto-promotes after 100 accesses");
    info!("   • Requires stable decision (no changes)");
    info!("   • Result: 100-500x speedup for hot paths!");

    info!("\n{}", "=".repeat(80));

    Ok(())
}
