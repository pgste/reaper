//! Served-path SLO harness (Plan 08 §3; closes PERF R2-P1-1).
//!
//! Drives a REAL reaper-agent over HTTP through the four SLO-table rows and
//! reports request-total latency percentiles from an HDR histogram recorded in
//! nanoseconds (client-observed: send → full response read, an upper bound on
//! the server-side request-total the SLA is defined on):
//!
//! | scenario                 | served path                     | SLO row                          |
//! |--------------------------|---------------------------------|----------------------------------|
//! | `slo-targeted`           | POST /api/v1/messages, policy_id| Targeted, 10k DSL policies       |
//! | `slo-evaluate-all`       | POST /api/v1/messages, no policy| Evaluate-all, resource-ID tier   |
//! | `slo-evaluate-all-abac`  | POST /api/v1/messages, no policy| Evaluate-all, resource-TYPE tier |
//! | `slo-rebac`              | POST /api/v1/messages, policy_id| ABAC/ReBAC bounded traversal     |
//! | `slo-batch`              | POST /api/v1/batch-messages     | Batch, 100 requests/call         |
//!
//! Policy sets come from `generate-data policy-set` (see README "SLO
//! harness"). `--assert-slo slo.yaml` turns the run into a gate: every
//! measured percentile is compared against the checked-in §3 table scaled by
//! `--slo-multiplier` (env `SLO_MULTIPLIER`); violations are listed and the
//! process exits non-zero. Multiplier 1.0 is the REAL SLA (dedicated
//! hardware); shared CI runners use a documented larger multiplier.
//!
//! Agent prerequisites per scenario (see README):
//! - `slo-evaluate-all` needs `REAPER_ALLOW_EVALUATE_ALL=true` (and the
//!   default `REAPER_USE_PRUNING_INDEX=true`), and the loaded set must be
//!   PRUNABLE. Both the `simple` and the `dsl` policy sets qualify: round-2 D2
//!   made DSL literal-resource policies prunable (compiled resource-literal
//!   extraction), so a DSL set buckets in the pruning index just like Simple.
//!   When running `--scenario all`, evaluate-all runs FIRST against a fresh
//!   agent so its set is the only one deployed.
//! - `slo-evaluate-all-abac` (round-3 R3-P2-1) has the same agent
//!   prerequisites but takes an `abac` policy set: `resource.type`-gated DSL
//!   policies plus the typed resource entities the requests address, prunable
//!   via the resource-TYPE index tier. Run it against a FRESH agent (it is
//!   deliberately not part of `--scenario all`).

use anyhow::{bail, Context, Result};
use clap::Parser;
use colored::Colorize;
use hdrhistogram::Histogram;
use indicatif::{ProgressBar, ProgressStyle};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::task::JoinSet;

#[derive(Parser)]
#[command(name = "slo-harness")]
#[command(about = "Measure the Plan 08 §3 served-path SLO against a real reaper-agent")]
struct Args {
    /// Reaper agent base URL
    #[arg(long, default_value = "http://localhost:8080")]
    reaper_url: String,

    /// Scenario: slo-targeted | slo-evaluate-all | slo-evaluate-all-abac |
    /// slo-rebac | slo-batch | all (all excludes the abac leg — it needs its
    /// own fresh agent)
    #[arg(short, long, default_value = "all")]
    scenario: String,

    /// DSL policy-set file (from `generate-data policy-set --language dsl`);
    /// required for slo-targeted and slo-batch.
    #[arg(long)]
    policy_set: Option<String>,

    /// Policy-set file for slo-evaluate-all (`--language simple|dsl`, both
    /// prunable since round-2 D2) or slo-evaluate-all-abac (`--language abac`,
    /// prunable via the R3-P2-1 resource-type tier; carries the typed
    /// resource entities). From `generate-data policy-set`.
    #[arg(long)]
    evaluate_all_policy_set: Option<String>,

    /// ReBAC .reap policy for slo-rebac.
    #[arg(long, default_value = "policies/reaper/rebac.reap")]
    rebac_policy: String,

    /// ReBAC entity data for slo-rebac (loaded via /api/v1/data/stream).
    #[arg(long, default_value = "data/10k/rebac.json")]
    rebac_data: String,

    /// Measured requests per scenario (for slo-batch: number of CALLS).
    #[arg(short, long, default_value = "10000")]
    requests: usize,

    /// Concurrent in-flight requests.
    #[arg(short, long, default_value = "16")]
    concurrency: usize,

    /// Requests per batch call (the §3 batch row is 100).
    #[arg(long, default_value = "100")]
    batch_size: usize,

    /// Unrecorded warmup requests before measuring.
    #[arg(long, default_value = "1000")]
    warmup: usize,

    /// Skip setup (policies/data already deployed by a previous run).
    #[arg(long)]
    skip_setup: bool,

    /// Concurrent policy deploys during setup.
    #[arg(long, default_value = "32")]
    deploy_concurrency: usize,

    /// Output format: table | json
    #[arg(short, long, default_value = "table")]
    output: String,

    /// Save results JSON to a file.
    #[arg(long)]
    save: Option<String>,

    /// Assert measured percentiles against an SLO table file (slo.yaml).
    #[arg(long)]
    assert_slo: Option<String>,

    /// Multiplier applied to EVERY slo.yaml threshold. 1.0 = the real SLA
    /// (dedicated hardware); shared CI runners need a larger, documented one.
    #[arg(long, env = "SLO_MULTIPLIER", default_value = "1.0")]
    slo_multiplier: f64,
}

// ---------------------------------------------------------------------------
// Policy-set + SLO-table file formats
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct PolicySetFile {
    language: String,
    policies: Vec<PolicySpec>,
    /// Entities the requests address (abac sets: the typed resources the
    /// type-tier prefilter resolves against). Loaded via /api/v1/data/stream
    /// during setup; absent/empty for dsl and simple sets.
    #[serde(default)]
    entities: Vec<serde_json::Value>,
}

#[derive(Deserialize, Clone)]
struct PolicySpec {
    name: String,
    policy_id: String,
    resource: String,
    #[serde(default)]
    content: Option<String>,
}

#[derive(Deserialize)]
struct SloTable {
    scenarios: HashMap<String, SloThresholds>,
}

#[derive(Deserialize)]
struct SloThresholds {
    p50_us: f64,
    p99_us: f64,
    p999_us: f64,
}

// ---------------------------------------------------------------------------
// Results
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct SloResult {
    scenario: String,
    policies_loaded: usize,
    requests: usize,
    concurrency: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    batch_size: Option<usize>,
    successful: usize,
    failed: usize,
    allowed: usize,
    denied: usize,
    duration_secs: f64,
    throughput_rps: f64,
    p50_us: f64,
    p99_us: f64,
    p999_us: f64,
    max_us: f64,
    mean_us: f64,
}

#[derive(Default)]
struct LoadCounts {
    successful: usize,
    failed: usize,
    allowed: usize,
    denied: usize,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let scenarios: Vec<&str> = match args.scenario.as_str() {
        // evaluate-all FIRST: it must run before the unprunable DSL set is
        // deployed (the index would return every DSL policy as a candidate
        // and the cap would blanket-deny — R2-P2-1).
        // slo-evaluate-all-abac is NOT in `all`: it needs its own fresh agent
        // (a second 10k evaluate-all set deployed alongside the first would
        // measure a mixed candidate population). The nightly workflow runs it
        // as a separate scenario invocation, like the DSL evaluate-all leg.
        "all" => vec!["slo-evaluate-all", "slo-targeted", "slo-batch", "slo-rebac"],
        s @ ("slo-targeted"
        | "slo-evaluate-all"
        | "slo-evaluate-all-abac"
        | "slo-rebac"
        | "slo-batch") => vec![s],
        other => bail!(
            "unknown scenario '{other}' \
             (slo-targeted|slo-evaluate-all|slo-evaluate-all-abac|slo-rebac|slo-batch|all)"
        ),
    };

    eprintln!("{}", "\n📏 Reaper served-path SLO harness".bold().cyan());
    eprintln!("  Agent:       {}", args.reaper_url.dimmed());
    eprintln!("  Scenarios:   {}", scenarios.join(", ").yellow());
    eprintln!(
        "  Requests:    {}   Concurrency: {}   Warmup: {}",
        args.requests.to_string().yellow(),
        args.concurrency.to_string().yellow(),
        args.warmup.to_string().yellow()
    );

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;

    // Connectivity gate.
    let health = client
        .get(format!("{}/health", args.reaper_url))
        .send()
        .await
        .with_context(|| format!("cannot reach reaper-agent at {}", args.reaper_url))?;
    if !health.status().is_success() {
        bail!("reaper-agent health returned HTTP {}", health.status());
    }
    eprintln!("  {} agent reachable\n", "✓".green());

    // Shared DSL policy-set state across scenarios (targeted deploys it; batch
    // reuses it within an `all` run).
    let mut dsl_ids: Option<Vec<String>> = None;
    let mut dsl_specs: Option<Vec<PolicySpec>> = None;
    let mut results: Vec<SloResult> = Vec::new();

    for scenario in &scenarios {
        eprintln!("{} {}", "▶ scenario:".bold(), scenario.yellow());
        let result = match *scenario {
            "slo-targeted" => {
                let (specs, ids) =
                    ensure_dsl_set(&args, &client, &mut dsl_specs, &mut dsl_ids).await?;
                run_targeted(&args, &client, &specs, &ids).await?
            }
            "slo-evaluate-all" | "slo-evaluate-all-abac" => {
                run_evaluate_all(&args, &client, scenario).await?
            }
            "slo-batch" => {
                let (specs, _ids) =
                    ensure_dsl_set(&args, &client, &mut dsl_specs, &mut dsl_ids).await?;
                run_batch(&args, &client, &specs).await?
            }
            "slo-rebac" => run_rebac(&args, &client).await?,
            _ => unreachable!(),
        };
        eprintln!(
            "  {} p50={}µs p99={}µs p999={}µs ({:.0} rps, {} ok / {} failed)\n",
            "✓".green(),
            format!("{:.1}", result.p50_us).yellow(),
            format!("{:.1}", result.p99_us).yellow(),
            format!("{:.1}", result.p999_us).yellow(),
            result.throughput_rps,
            result.successful,
            result.failed
        );
        results.push(result);
    }

    // Output.
    match args.output.as_str() {
        "json" => println!("{}", serde_json::to_string_pretty(&results)?),
        _ => print_table(&results),
    }
    if let Some(path) = &args.save {
        std::fs::write(path, serde_json::to_string_pretty(&results)?)?;
        eprintln!("{} {}", "💾 results saved to".green(), path);
    }

    // Assertion gate.
    if let Some(slo_path) = &args.assert_slo {
        assert_slo(&results, slo_path, args.slo_multiplier)?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Scenario setup + drivers
// ---------------------------------------------------------------------------

/// Deploy (once) and return the DSL policy set: (specs, deployed policy ids
/// aligned by index).
async fn ensure_dsl_set(
    args: &Args,
    client: &reqwest::Client,
    specs_slot: &mut Option<Vec<PolicySpec>>,
    ids_slot: &mut Option<Vec<String>>,
) -> Result<(Vec<PolicySpec>, Vec<String>)> {
    if let (Some(specs), Some(ids)) = (specs_slot.as_ref(), ids_slot.as_ref()) {
        return Ok((specs.clone(), ids.clone()));
    }
    let path = args
        .policy_set
        .as_ref()
        .context("--policy-set <dsl policy-set file> is required for slo-targeted / slo-batch")?;
    let set = load_policy_set(path)?;
    if set.language != "dsl" {
        bail!(
            "--policy-set {} has language '{}', expected 'dsl' \
             (generate with: generate-data policy-set --language dsl)",
            path,
            set.language
        );
    }

    let ids = if args.skip_setup {
        // The compile endpoint derives a stable id from the policy name, so a
        // redeploy-free run can only target by name. Names work in the
        // policy_id field (the handler falls back to name lookup).
        set.policies.iter().map(|p| p.name.clone()).collect()
    } else {
        // The DSL evaluator resolves the principal as a loaded entity, so the
        // request principals (slo_user_0..999) must exist in the DataStore.
        load_principal_entities(client, &args.reaper_url, PRINCIPAL_POOL).await?;
        deploy_dsl_policies(
            client,
            &args.reaper_url,
            &set.policies,
            args.deploy_concurrency,
        )
        .await?
    };

    *specs_slot = Some(set.policies.clone());
    *ids_slot = Some(ids.clone());
    Ok((set.policies, ids))
}

/// Distinct principals cycled through request payloads; must exist as
/// entities for DSL evaluation (the evaluator resolves the principal).
const PRINCIPAL_POOL: usize = 1000;

/// Load `count` `slo_user_{i}` User entities via /api/v1/data/stream.
async fn load_principal_entities(client: &reqwest::Client, base: &str, count: usize) -> Result<()> {
    let entities: Vec<serde_json::Value> = (0..count)
        .map(|i| {
            json!({
                "id": format!("slo_user_{i}"),
                "type": "User",
                "attributes": {"role": "user", "index": i},
            })
        })
        .collect();
    let resp = client
        .post(format!("{base}/api/v1/data/stream"))
        .json(&json!({"entities": entities}))
        .send()
        .await?;
    if !resp.status().is_success() {
        bail!(
            "principal entity load failed (HTTP {}): {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        );
    }
    Ok(())
}

/// The agent's current policy count (from /health) — reported per result so
/// the "at N policies" context of each measurement is recorded, not assumed.
async fn agent_policies_loaded(client: &reqwest::Client, base: &str) -> usize {
    let Ok(resp) = client.get(format!("{base}/health")).send().await else {
        return 0;
    };
    resp.json::<serde_json::Value>()
        .await
        .ok()
        .and_then(|v| v.get("policies_loaded").and_then(|p| p.as_u64()))
        .unwrap_or(0) as usize
}

fn load_policy_set(path: &str) -> Result<PolicySetFile> {
    let raw = std::fs::read(path).with_context(|| format!("cannot read policy set file {path}"))?;
    serde_json::from_slice(&raw).with_context(|| format!("cannot parse policy set file {path}"))
}

/// Deploy DSL policies via POST /api/v1/policies/compile; returns the agent's
/// policy ids aligned with `specs`.
async fn deploy_dsl_policies(
    client: &reqwest::Client,
    base: &str,
    specs: &[PolicySpec],
    deploy_concurrency: usize,
) -> Result<Vec<String>> {
    let pb = deploy_progress(specs.len(), "compile+deploy DSL policies");
    let mut ids: Vec<Option<String>> = vec![None; specs.len()];
    let mut join = JoinSet::new();
    let mut next = 0usize;

    while next < specs.len() || !join.is_empty() {
        while next < specs.len() && join.len() < deploy_concurrency {
            let client = client.clone();
            let url = format!("{base}/api/v1/policies/compile");
            let body = json!({
                "policy_content": specs[next].content.as_deref()
                    .context("dsl policy spec missing 'content'")?,
                "policy_name": specs[next].name,
            });
            let idx = next;
            join.spawn(async move {
                let resp = client.post(&url).json(&body).send().await?;
                let status = resp.status();
                let body: serde_json::Value = resp.json().await?;
                if !status.is_success() {
                    bail!("compile deploy failed (HTTP {status}): {body}");
                }
                let id = body
                    .get("policy_id")
                    .and_then(|v| v.as_str())
                    .context("compile response missing policy_id")?
                    .to_string();
                Ok::<(usize, String), anyhow::Error>((idx, id))
            });
            next += 1;
        }
        if let Some(res) = join.join_next().await {
            let (idx, id) = res??;
            ids[idx] = Some(id);
            pb.inc(1);
        }
    }
    pb.finish_and_clear();
    ids.into_iter()
        .map(|o| o.context("missing deployed policy id"))
        .collect()
}

/// Deploy Simple policies via POST /api/v1/policies/deploy.
async fn deploy_simple_policies(
    client: &reqwest::Client,
    base: &str,
    specs: &[PolicySpec],
    deploy_concurrency: usize,
) -> Result<()> {
    let pb = deploy_progress(specs.len(), "deploy Simple policies");
    let mut join = JoinSet::new();
    let mut next = 0usize;

    while next < specs.len() || !join.is_empty() {
        while next < specs.len() && join.len() < deploy_concurrency {
            let client = client.clone();
            let url = format!("{base}/api/v1/policies/deploy");
            let body = json!({
                "policy_id": specs[next].policy_id,
                "name": specs[next].name,
                "description": "slo-harness evaluate-all policy",
                "rules": [{"action": "allow", "resource": specs[next].resource}],
            });
            join.spawn(async move {
                let resp = client.post(&url).json(&body).send().await?;
                let status = resp.status();
                let body: serde_json::Value = resp.json().await?;
                if !status.is_success() || body.get("error").is_some() {
                    bail!("simple deploy failed (HTTP {status}): {body}");
                }
                Ok::<(), anyhow::Error>(())
            });
            next += 1;
        }
        if let Some(res) = join.join_next().await {
            res??;
            pb.inc(1);
        }
    }
    pb.finish_and_clear();
    Ok(())
}

fn deploy_progress(len: usize, msg: &str) -> ProgressBar {
    let pb = ProgressBar::new(len as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("    {msg} [{bar:40.cyan/blue}] {pos}/{len} ({eta})")
            .unwrap()
            .progress_chars("#>-"),
    );
    pb.set_message(msg.to_string());
    pb
}

/// slo-targeted: 10k DSL policies loaded, requests carry a specific policy_id.
async fn run_targeted(
    args: &Args,
    client: &reqwest::Client,
    specs: &[PolicySpec],
    ids: &[String],
) -> Result<SloResult> {
    let n = specs.len();
    // Pre-serialize a pool of request payloads cycling over the policy set so
    // client-side JSON building never sits inside the timed window.
    let payloads: Vec<String> = (0..n.min(args.requests.max(1)))
        .map(|i| {
            json!({
                "policy_id": ids[i % n],
                "principal": format!("slo_user_{}", i % 1000),
                "resource": specs[i % n].resource,
                "action": "read",
            })
            .to_string()
        })
        .collect();

    // Probe: the targeted path must actually allow (a deploy failure would
    // otherwise silently benchmark policy_not_found denies).
    let probe: serde_json::Value = client
        .post(format!("{}/api/v1/messages", args.reaper_url))
        .body(payloads[0].clone())
        .header("content-type", "application/json")
        .send()
        .await?
        .json()
        .await?;
    if probe.get("decision").and_then(|d| d.as_str()) != Some("allow") {
        bail!("slo-targeted probe was not allowed: {probe} — policy set not deployed correctly?");
    }

    let policies_loaded = agent_policies_loaded(client, &args.reaper_url).await;
    let (hist, counts, duration) = run_load(
        client,
        format!("{}/api/v1/messages", args.reaper_url),
        Arc::new(payloads),
        args.requests,
        args.concurrency,
        args.warmup,
    )
    .await?;
    Ok(build_result(
        "slo-targeted",
        policies_loaded,
        args,
        None,
        hist,
        counts,
        duration,
    ))
}

/// slo-evaluate-all / slo-evaluate-all-abac: 10k prunable policies, no
/// policy_id in requests. The plain scenario takes a Simple or literal-DSL
/// set (resource-ID index tier); the abac scenario takes an abac set —
/// `resource.type`-gated DSL policies plus the typed resource entities —
/// exercising the resource-TYPE index tier (R3-P2-1) end-to-end.
async fn run_evaluate_all(
    args: &Args,
    client: &reqwest::Client,
    scenario: &str,
) -> Result<SloResult> {
    let path = args.evaluate_all_policy_set.as_ref().with_context(|| {
        format!("--evaluate-all-policy-set <policy-set file> is required for {scenario}")
    })?;
    let set = load_policy_set(path)?;
    // Simple and literal-DSL sets prune via the resource-id tier (round-2 D2);
    // abac sets prune via the resource-type tier (round-3 R3-P2-1). The probe
    // below is the end-to-end guard either way: if pruning did NOT engage for
    // this set, evaluate-all returns `candidate_cap_exceeded` and the run
    // fails loudly.
    match (scenario, set.language.as_str()) {
        ("slo-evaluate-all", "simple" | "dsl") => {}
        ("slo-evaluate-all-abac", "abac") => {}
        ("slo-evaluate-all", other) => bail!(
            "--evaluate-all-policy-set {path} has language '{other}', expected 'simple' or \
             'dsl' (use --scenario slo-evaluate-all-abac for abac sets)"
        ),
        (_, other) => bail!(
            "--evaluate-all-policy-set {path} has language '{other}', expected 'abac' \
             (generate with: generate-data policy-set --language abac)"
        ),
    }
    if !args.skip_setup {
        match set.language.as_str() {
            "dsl" | "abac" => {
                // The compiled DSL evaluator resolves the principal as a loaded
                // entity, so the request principals must exist in the DataStore.
                load_principal_entities(client, &args.reaper_url, PRINCIPAL_POOL).await?;
                // abac sets carry the typed resource entities the requests
                // address — without them no request resolves a resource type,
                // every type bucket is skipped, and the probe below denies.
                if !set.entities.is_empty() {
                    let resp = client
                        .post(format!("{}/api/v1/data/stream", args.reaper_url))
                        .json(&json!({"entities": set.entities}))
                        .send()
                        .await?;
                    if !resp.status().is_success() {
                        bail!(
                            "policy-set entity load failed (HTTP {}): {}",
                            resp.status(),
                            resp.text().await.unwrap_or_default()
                        );
                    }
                }
                deploy_dsl_policies(
                    client,
                    &args.reaper_url,
                    &set.policies,
                    args.deploy_concurrency,
                )
                .await?;
            }
            _ => {
                deploy_simple_policies(
                    client,
                    &args.reaper_url,
                    &set.policies,
                    args.deploy_concurrency,
                )
                .await?;
            }
        }
    }

    let n = set.policies.len();
    let payloads: Vec<String> = (0..n.min(args.requests.max(1)))
        .map(|i| {
            json!({
                "principal": format!("slo_user_{}", i % 1000),
                "resource": set.policies[i % n].resource,
                "action": "read",
            })
            .to_string()
        })
        .collect();

    // Probe: catches a disarmed agent or an unprunable set up front.
    let probe: serde_json::Value = client
        .post(format!("{}/api/v1/messages", args.reaper_url))
        .body(payloads[0].clone())
        .header("content-type", "application/json")
        .send()
        .await?
        .json()
        .await?;
    let matched = probe
        .get("matched_rule")
        .and_then(|m| m.as_str())
        .unwrap_or("");
    if matches!(
        matched,
        "evaluate_all_disabled" | "candidate_cap_exceeded" | "no_policies_loaded"
    ) {
        bail!(
            "{scenario} probe denied with '{matched}'. The agent under test needs \
             REAPER_ALLOW_EVALUATE_ALL=true (and the default REAPER_USE_PRUNING_INDEX=true); \
             'candidate_cap_exceeded' means the loaded policies did NOT prune down to the \
             matching candidates — for a literal-DSL set that indicates the resource-literal \
             extraction (D2) regressed; for an abac set it indicates the resource-type tier \
             (R3-P2-1) regressed (extraction, type_index maintenance, or the agent's \
             same-store type resolution)."
        );
    }
    if probe.get("decision").and_then(|d| d.as_str()) != Some("allow") {
        bail!("{scenario} probe was not allowed: {probe}");
    }

    let policies_loaded = agent_policies_loaded(client, &args.reaper_url).await;
    let (hist, counts, duration) = run_load(
        client,
        format!("{}/api/v1/messages", args.reaper_url),
        Arc::new(payloads),
        args.requests,
        args.concurrency,
        args.warmup,
    )
    .await?;
    Ok(build_result(
        scenario,
        policies_loaded,
        args,
        None,
        hist,
        counts,
        duration,
    ))
}

/// slo-rebac: the ReBAC/ABAC bounded-traversal shape — rebac.reap over the
/// 10k-entity rebac dataset, targeted by policy id.
async fn run_rebac(args: &Args, client: &reqwest::Client) -> Result<SloResult> {
    let policy_id = if args.skip_setup {
        "slo-rebac-policy".to_string() // name fallback in the policy_id field
    } else {
        // Load entities via the streaming endpoint (same as deploy-reaper.sh).
        let data = std::fs::read(&args.rebac_data)
            .with_context(|| format!("cannot read rebac data {}", args.rebac_data))?;
        let resp = client
            .post(format!("{}/api/v1/data/stream", args.reaper_url))
            .header("content-type", "application/json")
            .body(data)
            .send()
            .await?;
        if !resp.status().is_success() {
            bail!(
                "rebac data load failed (HTTP {}): {}",
                resp.status(),
                resp.text().await.unwrap_or_default()
            );
        }
        // Deploy the ReBAC policy.
        let content = std::fs::read_to_string(&args.rebac_policy)
            .with_context(|| format!("cannot read rebac policy {}", args.rebac_policy))?;
        let resp: serde_json::Value = client
            .post(format!("{}/api/v1/policies/compile", args.reaper_url))
            .json(&json!({"policy_content": content, "policy_name": "slo-rebac-policy"}))
            .send()
            .await?
            .json()
            .await?;
        resp.get("policy_id")
            .and_then(|v| v.as_str())
            .with_context(|| format!("rebac policy deploy failed: {resp}"))?
            .to_string()
    };

    // Same request shape as the benchmark tool's rebac scenario, so requests
    // line up with the checked-in data/10k/rebac.json entities.
    let teams = ["alpha", "beta", "gamma", "delta", "omega"];
    let actions = ["read", "write", "delete"];
    let pool = 10_000.min(args.requests.max(1));
    let payloads: Vec<String> = (0..pool)
        .map(|i| {
            let team = teams[i % teams.len()];
            json!({
                "policy_id": policy_id,
                "principal": format!("user_{}_{}", team, i % 1000),
                "resource": format!("resource_{}_{}", team, i % 200),
                "action": actions[i % actions.len()],
            })
            .to_string()
        })
        .collect();

    // Probe: only that the policy resolves (mixed allow/deny is expected).
    let probe: serde_json::Value = client
        .post(format!("{}/api/v1/messages", args.reaper_url))
        .body(payloads[0].clone())
        .header("content-type", "application/json")
        .send()
        .await?
        .json()
        .await?;
    if probe.get("matched_rule").and_then(|m| m.as_str()) == Some("policy_not_found") {
        bail!("slo-rebac probe hit policy_not_found: {probe}");
    }

    let policies_loaded = agent_policies_loaded(client, &args.reaper_url).await;
    let (hist, counts, duration) = run_load(
        client,
        format!("{}/api/v1/messages", args.reaper_url),
        Arc::new(payloads),
        args.requests,
        args.concurrency,
        args.warmup,
    )
    .await?;
    Ok(build_result(
        "slo-rebac",
        policies_loaded,
        args,
        None,
        hist,
        counts,
        duration,
    ))
}

/// slo-batch: 100-request calls against one policy of the DSL set; latency is
/// per CALL (the §3 batch row is per-call).
async fn run_batch(
    args: &Args,
    client: &reqwest::Client,
    specs: &[PolicySpec],
) -> Result<SloResult> {
    let target = &specs[0];
    // A pool of distinct batch payloads (principals vary per item and per
    // payload) so the decision cache sees a realistic mixed stream.
    let pool = 64usize;
    let payloads: Vec<String> = (0..pool)
        .map(|p| {
            let items: Vec<serde_json::Value> = (0..args.batch_size)
                .map(|j| {
                    json!({
                        "id": format!("r{j}"),
                        "principal": format!("slo_user_{}", (p * args.batch_size + j) % PRINCIPAL_POOL),
                        "resource": target.resource,
                        "action": "read",
                    })
                })
                .collect();
            json!({"policy_name": target.name, "requests": items}).to_string()
        })
        .collect();

    let probe: serde_json::Value = client
        .post(format!("{}/api/v1/batch-messages", args.reaper_url))
        .body(payloads[0].clone())
        .header("content-type", "application/json")
        .send()
        .await?
        .json()
        .await?;
    if probe.get("error").is_some() {
        bail!("slo-batch probe failed: {probe}");
    }

    let policies_loaded = agent_policies_loaded(client, &args.reaper_url).await;
    let (hist, counts, duration) = run_load(
        client,
        format!("{}/api/v1/batch-messages", args.reaper_url),
        Arc::new(payloads),
        args.requests,
        args.concurrency,
        args.warmup.min(args.requests / 10 + 1),
    )
    .await?;
    Ok(build_result(
        "slo-batch",
        policies_loaded,
        args,
        Some(args.batch_size),
        hist,
        counts,
        duration,
    ))
}

// ---------------------------------------------------------------------------
// Load driver — fixed worker tasks over a shared cursor, per-worker HDR
// histograms (nanosecond resolution) merged at the end.
// ---------------------------------------------------------------------------

async fn run_load(
    client: &reqwest::Client,
    url: String,
    payloads: Arc<Vec<String>>,
    total: usize,
    concurrency: usize,
    warmup: usize,
) -> Result<(Histogram<u64>, LoadCounts, Duration)> {
    // Warmup (unrecorded): connection pool, agent caches, allocator.
    if warmup > 0 {
        drive(client, &url, payloads.clone(), warmup, concurrency, None).await?;
    }
    let pb = ProgressBar::new(total as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("    measuring [{bar:40.cyan/blue}] {pos}/{len} ({eta})")
            .unwrap()
            .progress_chars("#>-"),
    );
    let start = Instant::now();
    let (hist, counts) =
        drive(client, &url, payloads, total, concurrency, Some(pb.clone())).await?;
    let duration = start.elapsed();
    pb.finish_and_clear();
    Ok((hist, counts, duration))
}

async fn drive(
    client: &reqwest::Client,
    url: &str,
    payloads: Arc<Vec<String>>,
    total: usize,
    concurrency: usize,
    pb: Option<ProgressBar>,
) -> Result<(Histogram<u64>, LoadCounts)> {
    let cursor = Arc::new(AtomicUsize::new(0));
    let mut join = JoinSet::new();

    for _ in 0..concurrency.max(1) {
        let client = client.clone();
        let url = url.to_string();
        let payloads = payloads.clone();
        let cursor = cursor.clone();
        let pb = pb.clone();
        join.spawn(async move {
            // 1ns..60s at 3 significant digits.
            let mut hist =
                Histogram::<u64>::new_with_bounds(1, 60_000_000_000, 3).expect("histogram bounds");
            let mut counts = LoadCounts::default();
            loop {
                let i = cursor.fetch_add(1, Ordering::Relaxed);
                if i >= total {
                    break;
                }
                let payload = &payloads[i % payloads.len()];
                let t = Instant::now();
                let resp = client
                    .post(&url)
                    .header("content-type", "application/json")
                    .body(payload.clone())
                    .send()
                    .await;
                let outcome = match resp {
                    Ok(r) if r.status().is_success() => r.bytes().await.ok(),
                    _ => None,
                };
                let elapsed_ns = t.elapsed().as_nanos() as u64;
                if let Some(pb) = &pb {
                    pb.inc(1);
                }
                match outcome {
                    Some(body) => {
                        counts.successful += 1;
                        hist.record(elapsed_ns.max(1)).ok();
                        // Cheap decision sniff without a full JSON parse.
                        if memfind(&body, b"\"decision\":\"allow\"")
                            || memfind(&body, b"\"decision\": \"allow\"")
                        {
                            counts.allowed += 1;
                        } else {
                            counts.denied += 1;
                        }
                    }
                    None => counts.failed += 1,
                }
            }
            (hist, counts)
        });
    }

    let mut hist =
        Histogram::<u64>::new_with_bounds(1, 60_000_000_000, 3).expect("histogram bounds");
    let mut counts = LoadCounts::default();
    while let Some(res) = join.join_next().await {
        let (h, c) = res?;
        hist.add(&h)?;
        counts.successful += c.successful;
        counts.failed += c.failed;
        counts.allowed += c.allowed;
        counts.denied += c.denied;
    }
    Ok((hist, counts))
}

/// Tiny substring search (avoids pulling a JSON parse into the timed loop's
/// accounting path; called after the latency was already recorded).
fn memfind(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn build_result(
    scenario: &str,
    policies_loaded: usize,
    args: &Args,
    batch_size: Option<usize>,
    hist: Histogram<u64>,
    counts: LoadCounts,
    duration: Duration,
) -> SloResult {
    let us = |v: u64| v as f64 / 1000.0;
    SloResult {
        scenario: scenario.to_string(),
        policies_loaded,
        requests: args.requests,
        concurrency: args.concurrency,
        batch_size,
        successful: counts.successful,
        failed: counts.failed,
        allowed: counts.allowed,
        denied: counts.denied,
        duration_secs: duration.as_secs_f64(),
        throughput_rps: counts.successful as f64 / duration.as_secs_f64(),
        p50_us: us(hist.value_at_quantile(0.50)),
        p99_us: us(hist.value_at_quantile(0.99)),
        p999_us: us(hist.value_at_quantile(0.999)),
        max_us: us(hist.max()),
        mean_us: hist.mean() / 1000.0,
    }
}

fn print_table(results: &[SloResult]) {
    eprintln!(
        "\n{}",
        "📈 SLO harness results (request-total, µs)".bold().cyan()
    );
    eprintln!(
        "{:<18} {:>9} {:>9} {:>10} {:>10} {:>10} {:>10} {:>10}",
        "scenario", "policies", "rps", "p50", "p99", "p999", "max", "failed"
    );
    for r in results {
        eprintln!(
            "{:<18} {:>9} {:>9.0} {:>10.1} {:>10.1} {:>10.1} {:>10.1} {:>10}",
            r.scenario,
            r.policies_loaded,
            r.throughput_rps,
            r.p50_us,
            r.p99_us,
            r.p999_us,
            r.max_us,
            r.failed
        );
    }
}

// ---------------------------------------------------------------------------
// SLO assertion
// ---------------------------------------------------------------------------

fn assert_slo(results: &[SloResult], slo_path: &str, multiplier: f64) -> Result<()> {
    if multiplier <= 0.0 {
        bail!("--slo-multiplier must be > 0 (got {multiplier})");
    }
    let raw = std::fs::read_to_string(slo_path)
        .with_context(|| format!("cannot read SLO table {slo_path}"))?;
    let table: SloTable =
        serde_yaml::from_str(&raw).with_context(|| format!("cannot parse SLO table {slo_path}"))?;

    eprintln!(
        "\n{} {} (multiplier {}x{})",
        "⚖️  asserting against".bold(),
        slo_path,
        multiplier,
        if (multiplier - 1.0).abs() < f64::EPSILON {
            " — the REAL SLA"
        } else {
            ""
        }
    );

    let mut violations: Vec<String> = Vec::new();
    let mut asserted = 0usize;
    for r in results {
        let Some(t) = table.scenarios.get(&r.scenario) else {
            eprintln!("  {} no SLO row for {}, skipping", "•".dimmed(), r.scenario);
            continue;
        };
        asserted += 1;
        if r.failed > 0 {
            violations.push(format!(
                "{}: {} failed requests (SLO percentiles only count served requests)",
                r.scenario, r.failed
            ));
        }
        for (cell, measured, limit) in [
            ("p50", r.p50_us, t.p50_us * multiplier),
            ("p99", r.p99_us, t.p99_us * multiplier),
            ("p999", r.p999_us, t.p999_us * multiplier),
        ] {
            if measured > limit {
                violations.push(format!(
                    "{}: {cell} {measured:.1}µs > {limit:.1}µs (= {:.1}µs × {multiplier})",
                    r.scenario,
                    limit / multiplier
                ));
            } else {
                eprintln!(
                    "  {} {} {cell}: {measured:.1}µs ≤ {limit:.1}µs",
                    "✓".green(),
                    r.scenario
                );
            }
        }
    }

    if asserted == 0 {
        bail!("--assert-slo matched no measured scenario — refusing to pass vacuously");
    }
    if violations.is_empty() {
        eprintln!("{}", "  all asserted SLO cells within limits".green());
        Ok(())
    } else {
        for v in &violations {
            eprintln!("  {} {}", "✗".red().bold(), v.red());
        }
        eprintln!(
            "{}",
            format!(
                "FAIL: {} SLO cell(s) violated (multiplier {multiplier}x)",
                violations.len()
            )
            .red()
            .bold()
        );
        // Clean non-zero exit for CI (no anyhow backtrace noise).
        std::process::exit(1);
    }
}
