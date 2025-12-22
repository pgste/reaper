//! eBPF validation and analysis commands

use anyhow::{Context, Result};
use policy_engine::ReaperPolicy;
use reaper_ebpf::{
    ConditionAnalyzer, CustomDataRegistry, CustomDataSource, EntityDataset, EntityValidator,
};
use std::fs;
use tabled::{Table, Tabled};

// ============================================================================
// validate-data command
// ============================================================================

#[derive(Tabled)]
struct ValidationSummary {
    #[tabled(rename = "Status")]
    status: String,
    #[tabled(rename = "Entities")]
    entity_count: usize,
    #[tabled(rename = "Tier")]
    tier: String,
    #[tabled(rename = "Memory (MB)")]
    memory_mb: String,
    #[tabled(rename = "Errors")]
    errors: usize,
    #[tabled(rename = "Warnings")]
    warnings: usize,
}

#[derive(Tabled)]
struct EntityTypeSummary {
    #[tabled(rename = "Entity Type")]
    entity_type: String,
    #[tabled(rename = "Count")]
    count: usize,
}

pub fn handle_validate_data(
    file: &str,
    check_ebpf: bool,
    format: &str,
    custom_schemas_file: Option<&str>,
) -> Result<()> {
    println!("🔍 Validating Entity Dataset\n");

    // Load entity dataset
    println!("1️⃣  Loading data file: {}", file);
    let data_content =
        fs::read_to_string(file).with_context(|| format!("Failed to read data file: {}", file))?;

    let dataset: EntityDataset = serde_json::from_str(&data_content)
        .with_context(|| format!("Failed to parse JSON: {}", file))?;

    println!(
        "   ✓ Loaded dataset: {} (version: {})\n",
        dataset.dataset, dataset.version
    );

    // Load custom schemas if provided
    let mut validator = EntityValidator::new();
    if let Some(schemas_file) = custom_schemas_file {
        println!("2️⃣  Loading custom schemas: {}", schemas_file);
        let schemas_content = fs::read_to_string(schemas_file)
            .with_context(|| format!("Failed to read schemas file: {}", schemas_file))?;

        let schemas: Vec<CustomDataSource> = serde_json::from_str(&schemas_content)
            .with_context(|| format!("Failed to parse schemas JSON: {}", schemas_file))?;

        let mut registry = CustomDataRegistry::new();
        for schema in schemas {
            registry
                .register(schema)
                .map_err(|e| anyhow::anyhow!("Failed to register custom schema: {}", e))?;
        }

        println!(
            "   ✓ Registered {} custom schema(s)\n",
            registry.list().len()
        );
        validator = validator.with_custom_registry(registry);
    }

    // Validate dataset
    println!(
        "{}  Validating dataset...",
        if custom_schemas_file.is_some() {
            "3️⃣"
        } else {
            "2️⃣"
        }
    );
    let result = validator.validate(&dataset);

    // Output format
    match format {
        "json" => {
            // JSON output
            let json = serde_json::to_string_pretty(&result)?;
            println!("{}", json);
        }
        _ => {
            // Table output (default)
            println!();

            // Summary table
            let summary = ValidationSummary {
                status: if result.valid {
                    "✓ VALID".to_string()
                } else {
                    "✗ INVALID".to_string()
                },
                entity_count: result.entity_count,
                tier: format!("{:?}", result.tier),
                memory_mb: format!("{:.2}", result.estimated_memory as f64 / 1_000_000.0),
                errors: result.errors.len(),
                warnings: result.warnings.len(),
            };

            let table = Table::new(vec![summary]).to_string();
            println!("{}", table);
            println!();

            // Entity type breakdown
            if !result.entity_types.is_empty() {
                println!("📊 Entity Types:");
                let mut type_summaries: Vec<EntityTypeSummary> = result
                    .entity_types
                    .iter()
                    .map(|(t, c)| EntityTypeSummary {
                        entity_type: t.clone(),
                        count: *c,
                    })
                    .collect();
                type_summaries.sort_by(|a, b| b.count.cmp(&a.count));

                let table = Table::new(type_summaries).to_string();
                println!("{}\n", table);
            }

            // Errors
            if !result.errors.is_empty() {
                println!("❌ Validation Errors:");
                for error in &result.errors {
                    println!("   • {}", error);
                }
                println!();
            }

            // Warnings
            if !result.warnings.is_empty() {
                println!("⚠️  Warnings:");
                for warning in &result.warnings {
                    println!("   • {}", warning);
                }
                println!();
            }

            // eBPF check
            if check_ebpf {
                println!("🔧 eBPF Compatibility:");
                if result.valid {
                    println!("   ✓ Dataset is eBPF-ready");
                    println!("   ✓ Tier: {:?}", result.tier);
                    println!("   ✓ Estimated latency: {}ns", result.tier.latency_ns());
                    println!(
                        "   ✓ Memory estimate: {:.2} MB",
                        result.estimated_memory as f64 / 1_000_000.0
                    );
                } else {
                    println!("   ✗ Dataset cannot be loaded into eBPF");
                    println!("   → Fix validation errors first");
                }
                println!();
            }

            // Final status
            if result.valid {
                println!("✅ Validation passed!");
            } else {
                println!(
                    "❌ Validation failed - {} error(s) found",
                    result.errors.len()
                );
                std::process::exit(1);
            }
        }
    }

    Ok(())
}

// ============================================================================
// analyze-policy command
// ============================================================================

#[derive(Tabled)]
struct PolicyAnalysisSummary {
    #[tabled(rename = "Total Rules")]
    total_rules: usize,
    #[tabled(rename = "eBPF-Ready")]
    promotable: usize,
    #[tabled(rename = "Userspace")]
    not_promotable: usize,
    #[tabled(rename = "Fast Path %")]
    fast_path_percent: String,
}

#[derive(Tabled, Clone)]
struct RuleAnalysis {
    #[tabled(rename = "Rule")]
    rule_name: String,
    #[tabled(rename = "Status")]
    status: String,
    #[tabled(rename = "Complexity")]
    complexity: u8,
    #[tabled(rename = "Latency (ns)")]
    latency_ns: u32,
    #[tabled(rename = "Patterns")]
    patterns: String,
}

pub fn handle_analyze_policy(
    file: &str,
    check_ebpf: bool,
    show_recommendations: bool,
    format: &str,
) -> Result<()> {
    println!("🔍 Analyzing Policy for eBPF Compatibility\n");

    // Load policy
    println!("1️⃣  Loading policy: {}", file);
    let policy = ReaperPolicy::from_file_auto(file)
        .with_context(|| format!("Failed to load policy: {}", file))?;

    println!("   ✓ Loaded policy: {}\n", policy.name());

    // Analyze each rule
    println!("2️⃣  Analyzing policy rules...\n");
    let analyzer = ConditionAnalyzer::new();

    // We need to parse the policy AST to analyze conditions
    let _reaper_policy = ReaperPolicy::from_file_auto(file)?;
    // Access the internal AST via parsing again (we need access to the AST)
    // For now, use the policy-engine reap parser directly
    let policy_str = fs::read_to_string(file)?;
    use policy_engine::reap::ReapParser;
    let parsed_policy = ReapParser::parse(&policy_str)?;

    let mut promotable_count = 0;
    let mut not_promotable_count = 0;
    let mut rule_analyses = Vec::new();

    for rule in &parsed_policy.rules {
        let result = analyzer.analyze(&rule.condition);

        if result.promotable {
            promotable_count += 1;
        } else {
            not_promotable_count += 1;
        }

        let patterns = result
            .patterns
            .iter()
            .map(|p| format!("{:?}", p))
            .collect::<Vec<_>>()
            .join(", ");

        rule_analyses.push(RuleAnalysis {
            rule_name: rule.name.clone(),
            status: if result.promotable {
                "✓ eBPF".to_string()
            } else {
                "✗ Userspace".to_string()
            },
            complexity: result.complexity,
            latency_ns: result.estimated_latency_ns,
            patterns: if patterns.is_empty() {
                "-".to_string()
            } else {
                patterns
            },
        });
    }

    let total_rules = parsed_policy.rules.len();
    let fast_path_percent = if total_rules > 0 {
        format!(
            "{:.1}%",
            (promotable_count as f64 / total_rules as f64) * 100.0
        )
    } else {
        "0%".to_string()
    };

    // Output format
    match format {
        "json" => {
            // JSON output
            let analysis = serde_json::json!({
                "policy_name": policy.name(),
                "total_rules": total_rules,
                "promotable": promotable_count,
                "not_promotable": not_promotable_count,
                "fast_path_percent": fast_path_percent,
                "rules": rule_analyses.iter().map(|r| {
                    serde_json::json!({
                        "name": r.rule_name,
                        "promotable": r.status.contains("eBPF"),
                        "complexity": r.complexity,
                        "latency_ns": r.latency_ns,
                        "patterns": r.patterns,
                    })
                }).collect::<Vec<_>>(),
            });
            println!("{}", serde_json::to_string_pretty(&analysis)?);
        }
        _ => {
            // Table output (default)
            // Summary table
            let summary = PolicyAnalysisSummary {
                total_rules,
                promotable: promotable_count,
                not_promotable: not_promotable_count,
                fast_path_percent: fast_path_percent.clone(),
            };

            let table = Table::new(vec![summary]).to_string();
            println!("{}", table);
            println!();

            // Rule analysis table
            if !rule_analyses.is_empty() {
                println!("📋 Rule Analysis:");
                let table = Table::new(rule_analyses.clone()).to_string();
                println!("{}\n", table);
            }

            // eBPF check details
            if check_ebpf {
                println!("🔧 eBPF Compatibility Details:");
                for rule in &parsed_policy.rules {
                    let result = analyzer.analyze(&rule.condition);
                    println!("\n   Rule: {}", rule.name);
                    println!(
                        "   Status: {}",
                        if result.promotable {
                            "✓ eBPF-ready"
                        } else {
                            "✗ Userspace only"
                        }
                    );

                    if !result.promotable && !result.blocking_reasons.is_empty() {
                        println!("   Blocking reasons:");
                        for reason in &result.blocking_reasons {
                            println!("      • {}", reason);
                        }
                    }

                    if result.promotable {
                        println!("   Complexity: {}/10", result.complexity);
                        println!("   Estimated latency: {}ns", result.estimated_latency_ns);
                        if !result.entity_lookups.is_empty() {
                            println!("   Entity lookups: {}", result.entity_lookups.join(", "));
                        }
                    }
                }
                println!();
            }

            // Recommendations
            if show_recommendations {
                println!("💡 Recommendations:");
                let mut has_recommendations = false;

                for rule in &parsed_policy.rules {
                    let result = analyzer.analyze(&rule.condition);
                    if !result.recommendations.is_empty() || !result.warnings.is_empty() {
                        has_recommendations = true;
                        println!("\n   Rule: {}", rule.name);
                        for rec in &result.recommendations {
                            println!("      • {}", rec);
                        }
                        for warn in &result.warnings {
                            println!("      ⚠ {}", warn);
                        }
                    }
                }

                if !has_recommendations {
                    println!("   No specific recommendations - policy looks good!");
                }
                println!();
            }

            // Final summary
            println!("📊 Summary:");
            println!("   • Expected fast path coverage: {}", fast_path_percent);
            println!(
                "   • {} rule(s) will run in eBPF (< 150ns latency)",
                promotable_count
            );
            println!(
                "   • {} rule(s) will run in userspace (10-50µs latency)",
                not_promotable_count
            );

            if promotable_count > 0 {
                println!("\n✅ Policy has eBPF-promotable rules!");
            } else {
                println!("\n⚠️  No rules can be promoted to eBPF");
            }
        }
    }

    Ok(())
}
