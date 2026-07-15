//! Control-plane decision-log queries (ClickHouse-backed).
//!
//! The agents' in-memory ring only covers the last N decisions on one agent.
//! This module gives the control plane the *full* picture: tenant-scoped
//! queries over the central ClickHouse store that Vector ships decision NDJSON
//! into (see `deploy/decision-logs/` and DECISION_LOG_PIPELINE.md).
//!
//! Design choices:
//! - **ClickHouse HTTP interface via `reqwest`** — no native-protocol crate to
//!   carry; works against self-hosted and ClickHouse Cloud alike.
//! - **Server-side query parameters** (`{name:String}` placeholders bound via
//!   `param_*`), so user-supplied filters are never spliced into SQL —
//!   injection-safe by construction.
//! - **Tenant scoping is mandatory by default**: every query is pinned to the
//!   caller's organization id (the `tenant_id` Vector injects). Self-hosted
//!   single-tenant stores (empty `tenant_id`) can disable the filter with
//!   `REAPER_CLICKHOUSE_TENANT_FILTER=false`.
//! - **`FINAL` reads** collapse at-least-once ingest duplicates at query time
//!   (ReplacingMergeTree by `decision_id`).

pub mod purge;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Configuration for the ClickHouse decision store, from environment.
#[derive(Debug, Clone)]
pub struct DecisionStoreConfig {
    /// Base URL of the ClickHouse HTTP interface, e.g. `http://clickhouse:8123`.
    pub url: String,
    pub database: String,
    pub user: Option<String>,
    pub password: Option<String>,
    /// When false (single-tenant self-host), queries are not filtered by
    /// tenant_id. Defaults to true — never disable in the managed stack.
    pub tenant_filter: bool,
}

impl DecisionStoreConfig {
    /// Read from environment. Returns None when REAPER_CLICKHOUSE_URL is unset
    /// (decision queries disabled — endpoints answer 503 with setup guidance).
    pub fn from_env() -> Option<Self> {
        let url = std::env::var("REAPER_CLICKHOUSE_URL").ok()?;
        if url.trim().is_empty() {
            return None;
        }
        Some(Self {
            url: url.trim_end_matches('/').to_string(),
            database: std::env::var("REAPER_CLICKHOUSE_DATABASE")
                .unwrap_or_else(|_| "reaper_audit".to_string()),
            user: std::env::var("REAPER_CLICKHOUSE_USER").ok(),
            password: std::env::var("REAPER_CLICKHOUSE_PASSWORD").ok(),
            tenant_filter: std::env::var("REAPER_CLICKHOUSE_TENANT_FILTER")
                .map(|v| v.to_lowercase() != "false")
                .unwrap_or(true),
        })
    }
}

/// Filters for listing decisions. All optional; combined with AND.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DecisionQuery {
    pub principal: Option<String>,
    pub action: Option<String>,
    pub resource: Option<String>,
    /// "allow" / "deny" / "log"
    pub decision: Option<String>,
    pub policy_name: Option<String>,
    pub agent_id: Option<String>,
    /// Inclusive lower bound, RFC3339 or `YYYY-MM-DD HH:MM:SS`.
    pub from: Option<String>,
    /// Exclusive upper bound.
    pub to: Option<String>,
    pub limit: Option<u64>,
    /// Deprecated in favor of `cursor` (offset drifts under concurrent inserts
    /// and degrades on deep pages — Plan 07 Phase E); still honored when no
    /// cursor is given.
    pub offset: Option<u64>,
    /// Opaque keyset cursor; decoded by the API layer into [`Self::after`].
    /// Takes precedence over `offset`.
    pub cursor: Option<String>,
    /// Decoded exclusive resume position `(timestamp, decision_id)` — set by
    /// the API layer from `cursor`, never from the wire.
    #[serde(skip)]
    pub after: Option<(String, String)>,
}

/// A legal hold's row selector (Plan 04, step 6): the same dimensions as
/// [`DecisionQuery`] filters, all optional and ANDed together. An **empty**
/// filter is a *blanket hold* — it protects every decision the org has, and
/// suspends the org's retention purge entirely while active.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct HoldFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub principal: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Inclusive lower time bound (RFC3339 / `YYYY-MM-DD HH:MM:SS`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// Exclusive upper time bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
}

impl HoldFilter {
    /// True when no dimension is constrained — the hold covers everything.
    pub fn is_blanket(&self) -> bool {
        self.principal.is_none()
            && self.action.is_none()
            && self.resource.is_none()
            && self.decision.is_none()
            && self.policy_name.is_none()
            && self.agent_id.is_none()
            && self.from.is_none()
            && self.to.is_none()
    }
}

/// Outcome of a retention purge request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum PurgeOutcome {
    /// The DELETE mutation was submitted to ClickHouse (mutations apply
    /// asynchronously in the background).
    Submitted {
        cutoff: String,
        holds_honored: usize,
    },
    /// An active blanket hold covers the whole tenant — nothing was purged.
    SkippedBlanketHold,
}

/// Outcome of a subject-erasure request over the decision store (E2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum EraseOutcome {
    /// The redact-in-place UPDATE was submitted to ClickHouse (mutations apply
    /// asynchronously). `holds_honored` non-held rows carrying the subject were
    /// scheduled for redaction.
    Submitted { holds_honored: usize },
    /// An active blanket hold preserves the whole tenant for litigation — a
    /// lawful basis to retain, so no decision rows were redacted. The caller
    /// records the erasure request as deferred, not completed.
    DeferredBlanketHold,
}

/// Pseudonymised forms of an erasure subject, for tenants running
/// `PrivacyProfile::Pseudonymize` — where the decision-log `principal`/`resource`
/// columns hold `sha256:<hmac>` tokens, not the plaintext identifier. Computed
/// control-plane-side from the tenant's decision-log salt (which lives agent-side
/// and is never persisted here); see `api::audit`. Matching these alongside the
/// plaintext subject lets one erasure cover both a plaintext tenant and a
/// pseudonymised one without the caller having to know which profile is in force:
/// an HMAC token can never collide with a plaintext principal/resource, so the
/// extra match terms only ever hit the subject's own hashed rows.
#[derive(Debug, Clone)]
pub struct SubjectPseudonyms {
    /// `pseudonymize(salt, subject)` — the token stored in the `principal`
    /// column when `hash_principal` (or the `pseudonymize` profile) is on.
    pub principal: String,
    /// `pseudonymize_domain(salt, "resource", subject)` — the domain-separated
    /// token stored in the `resource` column when `hash_resource` is on.
    pub resource: String,
}

/// One decision row as returned to API clients (matches the agent's
/// DecisionLogEntry fields plus ingest metadata).
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct DecisionRow {
    pub timestamp: String,
    pub decision_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub trace_id: String,
    pub principal: String,
    pub action: String,
    pub resource: String,
    pub decision: String,
    pub policy_id: String,
    pub policy_name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub policy_version: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub matched_rule: String,
    pub evaluation_time_ns: u64,
    #[serde(default)]
    pub cache_hit: u8,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub agent_id: String,
    /// Raw JSON string in ClickHouse; surfaced as parsed JSON when possible.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    #[schema(value_type = Object)]
    pub context: Value,
    /// Explain snapshot (possibly an encryption envelope). Parsed JSON when
    /// present; the control plane can decrypt per tenant with
    /// `policy_engine::decrypt_input_data`.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    #[schema(value_type = Object)]
    pub input_data: Value,
    /// Replayable-capture snapshot (Plan 04 step 7): the full resolved request
    /// (`{"principal","action","resource","context"}`), possibly an encryption
    /// envelope. Null when the tier was off at capture — such rows are NOT
    /// replayable.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    #[schema(value_type = Object)]
    pub replay_input: Value,
}

/// Aggregate stats for a tenant + time range.
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct DecisionStats {
    pub total: u64,
    pub allows: u64,
    pub denies: u64,
    pub agents: u64,
    pub avg_evaluation_time_ns: f64,
    /// Top denied (policy_name, count) pairs.
    #[schema(value_type = Vec<Object>)]
    pub top_denied_policies: Vec<(String, u64)>,
}

/// Errors from the decision store.
#[derive(Debug, thiserror::Error)]
pub enum DecisionStoreError {
    #[error(
        "decision store not configured: set REAPER_CLICKHOUSE_URL (see deploy/decision-logs/)"
    )]
    NotConfigured,
    #[error("ClickHouse request failed: {0}")]
    Http(String),
    #[error("ClickHouse returned an error: {0}")]
    Query(String),
    #[error("failed to parse ClickHouse response: {0}")]
    Parse(String),
}

/// ClickHouse-backed decision store client.
pub struct DecisionStore {
    config: DecisionStoreConfig,
    client: reqwest::Client,
}

impl DecisionStore {
    pub fn new(config: DecisionStoreConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }

    /// Build from env; None when not configured.
    pub fn from_env() -> Option<Self> {
        DecisionStoreConfig::from_env().map(Self::new)
    }

    /// Whether queries/purges are tenant-scoped (false = single-tenant store).
    pub fn tenant_filter(&self) -> bool {
        self.config.tenant_filter
    }

    /// Run a SQL statement with bound parameters, returning JSONEachRow lines.
    async fn run(
        &self,
        sql: &str,
        params: &[(String, String)],
    ) -> Result<Vec<Value>, DecisionStoreError> {
        let mut req = self
            .client
            .post(&self.config.url)
            .query(&[
                ("database", self.config.database.as_str()),
                ("default_format", "JSONEachRow"),
            ])
            .body(sql.to_string());

        // Server-side parameter binding: values never touch the SQL text.
        for (name, value) in params {
            req = req.query(&[(format!("param_{name}"), value)]);
        }
        if let Some(ref user) = self.config.user {
            req = req.header("X-ClickHouse-User", user);
        }
        if let Some(ref password) = self.config.password {
            req = req.header("X-ClickHouse-Key", password);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| DecisionStoreError::Http(e.to_string()))?;
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| DecisionStoreError::Http(e.to_string()))?;
        if !status.is_success() {
            // ClickHouse puts the error text in the body; don't echo the SQL.
            return Err(DecisionStoreError::Query(
                body.lines().next().unwrap_or("unknown error").to_string(),
            ));
        }

        body.lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).map_err(|e| DecisionStoreError::Parse(e.to_string())))
            .collect()
    }

    /// List decisions for a tenant, newest first.
    pub async fn list(
        &self,
        tenant_id: &str,
        query: &DecisionQuery,
    ) -> Result<Vec<DecisionRow>, DecisionStoreError> {
        let (sql, params) = build_list_sql(&self.config, tenant_id, query);
        let rows = self.run(&sql, &params).await?;
        rows.into_iter()
            .map(|v| parse_row(v).map_err(DecisionStoreError::Parse))
            .collect()
    }

    /// Fetch one decision by id (tenant-scoped).
    pub async fn get_by_id(
        &self,
        tenant_id: &str,
        decision_id: &str,
    ) -> Result<Option<DecisionRow>, DecisionStoreError> {
        let (sql, params) = build_get_sql(&self.config, tenant_id, decision_id);
        let mut rows = self.run(&sql, &params).await?;
        match rows.pop() {
            Some(v) => Ok(Some(parse_row(v).map_err(DecisionStoreError::Parse)?)),
            None => Ok(None),
        }
    }

    /// Aggregate stats for a tenant + optional time range.
    pub async fn stats(
        &self,
        tenant_id: &str,
        from: Option<&str>,
        to: Option<&str>,
    ) -> Result<DecisionStats, DecisionStoreError> {
        let (sql, params) = build_stats_sql(&self.config, tenant_id, from, to);
        let rows = self.run(&sql, &params).await?;
        let row = rows
            .into_iter()
            .next()
            .ok_or_else(|| DecisionStoreError::Parse("empty stats result".to_string()))?;

        let (top_sql, top_params) = build_top_denied_sql(&self.config, tenant_id, from, to);
        let top_rows = self.run(&top_sql, &top_params).await?;
        let top_denied_policies = top_rows
            .into_iter()
            .filter_map(|v| {
                Some((
                    v.get("policy_name")?.as_str()?.to_string(),
                    v.get("count")?
                        .as_str()
                        .and_then(|s| s.parse().ok())
                        .or_else(|| v.get("count")?.as_u64())?,
                ))
            })
            .collect();

        let get_u64 = |k: &str| -> u64 {
            row.get(k)
                .map(|v| {
                    v.as_u64()
                        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                        .unwrap_or(0)
                })
                .unwrap_or(0)
        };
        Ok(DecisionStats {
            total: get_u64("total"),
            allows: get_u64("allows"),
            denies: get_u64("denies"),
            agents: get_u64("agents"),
            avg_evaluation_time_ns: row
                .get("avg_evaluation_time_ns")
                .and_then(|v| {
                    v.as_f64()
                        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                })
                .unwrap_or(0.0),
            top_denied_policies,
        })
    }

    /// Bucketed time series (for UI charts): total/allow/deny counts and avg
    /// eval time per `bucket_secs` interval within the range.
    pub async fn timeseries(
        &self,
        tenant_id: &str,
        from: Option<&str>,
        to: Option<&str>,
        bucket_secs: u32,
    ) -> Result<Vec<TimeseriesPoint>, DecisionStoreError> {
        let (sql, params) = build_timeseries_sql(&self.config, tenant_id, from, to, bucket_secs);
        let rows = self.run(&sql, &params).await?;
        Ok(rows
            .into_iter()
            .filter_map(|v| {
                let n = |k: &str| -> u64 {
                    v.get(k)
                        .and_then(|x| {
                            x.as_u64()
                                .or_else(|| x.as_str().and_then(|s| s.parse().ok()))
                        })
                        .unwrap_or(0)
                };
                Some(TimeseriesPoint {
                    bucket: v.get("bucket")?.as_str()?.to_string(),
                    total: n("total"),
                    allows: n("allows"),
                    denies: n("denies"),
                    avg_evaluation_time_ns: v
                        .get("avg_evaluation_time_ns")
                        .and_then(|x| {
                            x.as_f64()
                                .or_else(|| x.as_str().and_then(|s| s.parse().ok()))
                        })
                        .unwrap_or(0.0),
                })
            })
            .collect())
    }

    /// Distinct filter values with counts (for UI filter dropdowns): actions,
    /// decisions, policy names, and agent ids seen in the range.
    pub async fn facets(
        &self,
        tenant_id: &str,
        from: Option<&str>,
        to: Option<&str>,
    ) -> Result<Value, DecisionStoreError> {
        let (sql, params) = build_facets_sql(&self.config, tenant_id, from, to);
        let rows = self.run(&sql, &params).await?;
        let mut facets = serde_json::Map::new();
        for row in rows {
            let (Some(facet), Some(value)) = (
                row.get("facet").and_then(Value::as_str).map(str::to_string),
                row.get("value").and_then(Value::as_str).map(str::to_string),
            ) else {
                continue;
            };
            let count = row
                .get("count")
                .and_then(|c| {
                    c.as_u64()
                        .or_else(|| c.as_str().and_then(|s| s.parse().ok()))
                })
                .unwrap_or(0);
            facets
                .entry(facet)
                .or_insert_with(|| Value::Array(Vec::new()))
                .as_array_mut()
                .expect("facet entries are arrays")
                .push(serde_json::json!({ "value": value, "count": count }));
        }
        Ok(Value::Object(facets))
    }

    /// Retention purge (Plan 04, step 6): submit a DELETE mutation for the
    /// tenant's decisions older than `cutoff`, **excluding** rows matched by
    /// any active legal hold. Replaces the static ClickHouse `TTL ... DELETE`
    /// (which would delete held rows regardless — the exact failure legal
    /// holds exist to prevent).
    ///
    /// An active *blanket* hold (empty filter) suspends the purge entirely.
    /// ClickHouse mutations apply asynchronously; this returns once the
    /// mutation is accepted.
    pub async fn purge_expired(
        &self,
        tenant_id: &str,
        cutoff: &str,
        holds: &[HoldFilter],
    ) -> Result<PurgeOutcome, DecisionStoreError> {
        if holds.iter().any(HoldFilter::is_blanket) {
            return Ok(PurgeOutcome::SkippedBlanketHold);
        }
        let (sql, params) = build_purge_sql(&self.config, tenant_id, cutoff, holds);
        self.run(&sql, &params).await?;
        Ok(PurgeOutcome::Submitted {
            cutoff: cutoff.to_string(),
            holds_honored: holds.len(),
        })
    }

    /// Redact a data subject's PII from the decision store in place (E2, GDPR
    /// Art. 17). Overwrites `principal`/`resource`/`context`/`input_data`/
    /// `replay_input` with tombstones on every non-held row carrying the subject,
    /// leaving the hash-chain fields intact so the store still verifies under
    /// `VerifyMode::Linkage`. A blanket hold is a lawful basis to retain, so it
    /// defers the erasure rather than breaching the hold.
    ///
    /// `pseudonyms`, when supplied, extends the match to a tenant running the
    /// `pseudonymize` privacy profile: its `principal`/`resource` columns hold
    /// `sha256:<hmac>` tokens, not plaintext, so plaintext matching alone would
    /// silently miss every one of the subject's rows.
    pub async fn erase_subject(
        &self,
        tenant_id: &str,
        subject: &str,
        pseudonyms: Option<&SubjectPseudonyms>,
        holds: &[HoldFilter],
    ) -> Result<EraseOutcome, DecisionStoreError> {
        if holds.iter().any(HoldFilter::is_blanket) {
            return Ok(EraseOutcome::DeferredBlanketHold);
        }
        let (sql, params) =
            build_erase_subject_sql(&self.config, tenant_id, subject, pseudonyms, holds);
        self.run(&sql, &params).await?;
        Ok(EraseOutcome::Submitted {
            holds_honored: holds.len(),
        })
    }

    /// Purge checkpoints older than `cutoff` for a tenant. Only called when the
    /// tenant has **no** active holds: checkpoints attest decision ranges, so
    /// while anything is held the whole attestation chain is kept.
    pub async fn purge_checkpoints(
        &self,
        tenant_id: &str,
        cutoff: &str,
    ) -> Result<(), DecisionStoreError> {
        let mut params = Vec::new();
        let tenant_cond = if self.config.tenant_filter {
            params.push(("tenant".to_string(), tenant_id.to_string()));
            "tenant_id = {tenant:String} AND "
        } else {
            ""
        };
        params.push(("cutoff".to_string(), cutoff.to_string()));
        let sql = format!(
            "ALTER TABLE checkpoints DELETE WHERE {tenant_cond}\
             wallclock < parseDateTime64BestEffort({{cutoff:String}})"
        );
        self.run(&sql, &params).await?;
        Ok(())
    }

    /// Count a tenant's rows older than `cutoff` (purge preview / test proof
    /// that a purge or a hold behaved).
    pub async fn count_older_than(
        &self,
        tenant_id: &str,
        cutoff: &str,
    ) -> Result<u64, DecisionStoreError> {
        let mut params = Vec::new();
        let tenant_cond = if self.config.tenant_filter {
            params.push(("tenant".to_string(), tenant_id.to_string()));
            "tenant_id = {tenant:String} AND "
        } else {
            ""
        };
        params.push(("cutoff".to_string(), cutoff.to_string()));
        let sql = format!(
            "SELECT count() AS n FROM decisions FINAL \
             WHERE {tenant_cond}timestamp < parseDateTime64BestEffort({{cutoff:String}})"
        );
        let rows = self.run(&sql, &params).await?;
        Ok(rows
            .first()
            .and_then(|v| v.get("n"))
            .and_then(|n| {
                n.as_u64()
                    .or_else(|| n.as_str().and_then(|s| s.parse().ok()))
            })
            .unwrap_or(0))
    }

    /// Fetch the raw decision rows and checkpoint rows needed to verify the
    /// tamper-evident hash chain (round-2 A1). Decisions come back ordered by
    /// `(chain_id, seq)` — the exact write order per boot, never the table's
    /// physical order — filtered by tenant and, optionally, one `chain_id` and a
    /// `[seq_from, seq_to]` window. Checkpoints are scoped to the same
    /// tenant/chain. Rows are returned raw; the API layer reconstructs
    /// `DecisionLogEntry`/`Checkpoint` and runs `decision_log::verify_records`.
    pub async fn verify_range(
        &self,
        tenant_id: Option<&str>,
        chain_id: Option<&str>,
        seq_from: Option<u64>,
        seq_to: Option<u64>,
    ) -> Result<(Vec<Value>, Vec<Value>), DecisionStoreError> {
        let (dec_sql, dec_params) =
            build_verify_decisions_sql(&self.config, tenant_id, chain_id, seq_from, seq_to);
        let decisions = self.run(&dec_sql, &dec_params).await?;

        let (cp_sql, cp_params) = build_verify_checkpoints_sql(&self.config, tenant_id, chain_id);
        let checkpoints = self.run(&cp_sql, &cp_params).await?;

        Ok((decisions, checkpoints))
    }
}

/// Columns needed to reconstruct a `DecisionLogEntry` for hash-chain
/// verification (round-2 A1): the chain fields plus every stored decision
/// field. `seq`/`chain_id` drive per-boot ordering; `prev_hash`/`entry_hash`
/// are the chain links the verifier recomputes.
const VERIFY_COLUMNS: &str = "seq, chain_id, prev_hash, entry_hash, timestamp, decision_id, \
     trace_id, principal, action, resource, decision, policy_id, policy_name, policy_version, \
     matched_rule, evaluation_time_ns, cache_hit, agent_id, context, input_data, replay_input";

/// Decisions query for verification: tenant/chain/seq scoped, ordered by the
/// exact per-boot write order `(chain_id, seq)`. Every user value is bound as a
/// server-side parameter — never spliced.
fn build_verify_decisions_sql(
    config: &DecisionStoreConfig,
    tenant_id: Option<&str>,
    chain_id: Option<&str>,
    seq_from: Option<u64>,
    seq_to: Option<u64>,
) -> (String, Vec<(String, String)>) {
    let mut params: Vec<(String, String)> = Vec::new();
    let mut conds: Vec<String> = Vec::new();
    if config.tenant_filter {
        conds.push("tenant_id = {tenant:String}".to_string());
        params.push(("tenant".to_string(), tenant_id.unwrap_or("").to_string()));
    }
    if let Some(chain) = chain_id {
        conds.push("chain_id = {chain:String}".to_string());
        params.push(("chain".to_string(), chain.to_string()));
    }
    if let Some(from) = seq_from {
        conds.push("seq >= {seq_from:UInt64}".to_string());
        params.push(("seq_from".to_string(), from.to_string()));
    }
    if let Some(to) = seq_to {
        conds.push("seq <= {seq_to:UInt64}".to_string());
        params.push(("seq_to".to_string(), to.to_string()));
    }
    let where_sql = if conds.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conds.join(" AND "))
    };
    (
        format!("SELECT {VERIFY_COLUMNS} FROM decisions FINAL {where_sql} ORDER BY chain_id, seq"),
        params,
    )
}

/// Checkpoints query for verification: same tenant/chain scope. `record_type`
/// is implied by the table (not stored), so inject it so each row deserializes
/// into a `Checkpoint` (which requires the discriminator).
fn build_verify_checkpoints_sql(
    config: &DecisionStoreConfig,
    tenant_id: Option<&str>,
    chain_id: Option<&str>,
) -> (String, Vec<(String, String)>) {
    let mut params: Vec<(String, String)> = Vec::new();
    let mut conds: Vec<String> = Vec::new();
    if config.tenant_filter {
        conds.push("tenant_id = {tenant:String}".to_string());
        params.push(("tenant".to_string(), tenant_id.unwrap_or("").to_string()));
    }
    if let Some(chain) = chain_id {
        conds.push("chain_id = {chain:String}".to_string());
        params.push(("chain".to_string(), chain.to_string()));
    }
    let where_sql = if conds.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conds.join(" AND "))
    };
    (
        format!(
            "SELECT 'checkpoint' AS record_type, chain_id, seq_start, seq_end, count, \
             prev_hash, last_entry_hash, monotonic_start_ns, monotonic_end_ns, \
             toString(wallclock) AS wallclock, key_id, algorithm, signature \
             FROM checkpoints FINAL {where_sql} ORDER BY chain_id, seq_start"
        ),
        params,
    )
}

/// One bucket of the decision time series.
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct TimeseriesPoint {
    /// Bucket start (ClickHouse DateTime string, UTC).
    pub bucket: String,
    pub total: u64,
    pub allows: u64,
    pub denies: u64,
    pub avg_evaluation_time_ns: f64,
}

/// Parse a UI-friendly interval ("30s", "5m", "1h", "1d", or raw seconds)
/// into seconds, clamped to [10s, 7d]. Unknown input falls back to 1h.
pub fn parse_interval_secs(s: Option<&str>) -> u32 {
    let parsed = s
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|s| {
            let (num, unit) = match s.chars().last() {
                Some(c @ ('s' | 'm' | 'h' | 'd')) => (&s[..s.len() - 1], c),
                _ => (s, 's'),
            };
            let n: u64 = num.parse().ok()?;
            Some(match unit {
                'm' => n * 60,
                'h' => n * 3600,
                'd' => n * 86_400,
                _ => n,
            })
        })
        .unwrap_or(3600);
    parsed.clamp(10, 7 * 86_400) as u32
}

/// Parse a JSONEachRow decision into a DecisionRow, decoding the embedded
/// context/input_data JSON strings into structured values.
fn parse_row(mut v: Value) -> Result<DecisionRow, String> {
    // ClickHouse quotes 64-bit integers as JSON strings by default
    // (output_format_json_quote_64bit_integers); normalize back to a number.
    if let Some(n) = v
        .get("evaluation_time_ns")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<u64>().ok())
    {
        if let Some(obj) = v.as_object_mut() {
            obj.insert("evaluation_time_ns".to_string(), Value::from(n));
        }
    }
    // ClickHouse returns context/input_data as JSON *strings*; decode them so
    // API clients get structured objects (leave as null when empty).
    for key in ["context", "input_data", "replay_input"] {
        let decoded = match v.get(key).and_then(Value::as_str) {
            Some(s) if !s.trim().is_empty() && s != "{}" => {
                serde_json::from_str(s).unwrap_or(Value::Null)
            }
            _ => Value::Null,
        };
        if let Some(obj) = v.as_object_mut() {
            obj.insert(key.to_string(), decoded);
        }
    }
    serde_json::from_value(v).map_err(|e| e.to_string())
}

const LIST_COLUMNS: &str = "timestamp, decision_id, trace_id, principal, action, resource, \
     decision, policy_id, policy_name, policy_version, matched_rule, \
     evaluation_time_ns, cache_hit, agent_id, context, input_data, replay_input";

/// Shared WHERE-clause builder. Every branch appends a `{name:Type}`
/// placeholder and the bound value — never string-spliced input.
fn where_clause(
    config: &DecisionStoreConfig,
    tenant_id: &str,
    query: &DecisionQuery,
    params: &mut Vec<(String, String)>,
) -> String {
    let mut conds: Vec<String> = Vec::new();
    if config.tenant_filter {
        conds.push("tenant_id = {tenant:String}".to_string());
        params.push(("tenant".to_string(), tenant_id.to_string()));
    }
    let mut bind = |cond: &str, name: &str, value: &Option<String>| {
        if let Some(v) = value {
            conds.push(cond.to_string());
            params.push((name.to_string(), v.clone()));
        }
    };
    bind(
        "principal = {principal:String}",
        "principal",
        &query.principal,
    );
    bind("action = {action:String}", "action", &query.action);
    bind("resource = {resource:String}", "resource", &query.resource);
    bind("decision = {decision:String}", "decision", &query.decision);
    bind(
        "policy_name = {policy_name:String}",
        "policy_name",
        &query.policy_name,
    );
    bind("agent_id = {agent_id:String}", "agent_id", &query.agent_id);
    bind(
        "timestamp >= parseDateTime64BestEffort({from:String})",
        "from",
        &query.from,
    );
    bind(
        "timestamp < parseDateTime64BestEffort({to:String})",
        "to",
        &query.to,
    );

    if conds.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conds.join(" AND "))
    }
}

fn build_list_sql(
    config: &DecisionStoreConfig,
    tenant_id: &str,
    query: &DecisionQuery,
) -> (String, Vec<(String, String)>) {
    let mut params = Vec::new();
    let where_sql = where_clause(config, tenant_id, query, &mut params);
    // limit/offset are validated numerics, not strings; still bound as params.
    // Cap is 1001 (not 1000): the API layer fetches page+1 as its has-more
    // sentinel, so a full 1000-row page must still fit its sentinel row.
    let limit = query.limit.unwrap_or(100).min(1001);
    params.push(("limit".to_string(), limit.to_string()));

    // Keyset resume (Plan 07 Phase E): rows strictly after the cursor position
    // in (timestamp DESC, decision_id DESC) order. Never drifts under the
    // constant insert load a decision store sees, unlike OFFSET (kept for
    // compatibility when no cursor is given; deprecated).
    if let Some((ts, id)) = &query.after {
        let joiner = if where_sql.is_empty() { "WHERE" } else { "AND" };
        params.push(("cursor_ts".to_string(), ts.clone()));
        params.push(("cursor_id".to_string(), id.clone()));
        (
            format!(
                "SELECT {LIST_COLUMNS} FROM decisions FINAL {where_sql} {joiner} \
                 (timestamp, decision_id) < (parseDateTime64BestEffort({{cursor_ts:String}}), {{cursor_id:String}}) \
                 ORDER BY timestamp DESC, decision_id DESC LIMIT {{limit:UInt64}}"
            ),
            params,
        )
    } else {
        let offset = query.offset.unwrap_or(0);
        params.push(("offset".to_string(), offset.to_string()));
        (
            format!(
                "SELECT {LIST_COLUMNS} FROM decisions FINAL {where_sql} \
                 ORDER BY timestamp DESC, decision_id DESC LIMIT {{limit:UInt64}} OFFSET {{offset:UInt64}}"
            ),
            params,
        )
    }
}

fn build_get_sql(
    config: &DecisionStoreConfig,
    tenant_id: &str,
    decision_id: &str,
) -> (String, Vec<(String, String)>) {
    let mut params = Vec::new();
    let tenant_cond = if config.tenant_filter {
        params.push(("tenant".to_string(), tenant_id.to_string()));
        "tenant_id = {tenant:String} AND "
    } else {
        ""
    };
    params.push(("decision_id".to_string(), decision_id.to_string()));
    (
        format!(
            "SELECT {LIST_COLUMNS} FROM decisions FINAL \
             WHERE {tenant_cond}decision_id = {{decision_id:String}} \
             ORDER BY timestamp DESC LIMIT 1"
        ),
        params,
    )
}

fn build_stats_sql(
    config: &DecisionStoreConfig,
    tenant_id: &str,
    from: Option<&str>,
    to: Option<&str>,
) -> (String, Vec<(String, String)>) {
    let mut params = Vec::new();
    let range = DecisionQuery {
        from: from.map(str::to_string),
        to: to.map(str::to_string),
        ..Default::default()
    };
    let where_sql = where_clause(config, tenant_id, &range, &mut params);
    (
        format!(
            "SELECT count() AS total, \
             countIf(decision = 'allow') AS allows, \
             countIf(decision = 'deny') AS denies, \
             uniqExact(agent_id) AS agents, \
             avg(evaluation_time_ns) AS avg_evaluation_time_ns \
             FROM decisions FINAL {where_sql}"
        ),
        params,
    )
}

fn build_timeseries_sql(
    config: &DecisionStoreConfig,
    tenant_id: &str,
    from: Option<&str>,
    to: Option<&str>,
    bucket_secs: u32,
) -> (String, Vec<(String, String)>) {
    let mut params = Vec::new();
    let range = DecisionQuery {
        from: from.map(str::to_string),
        to: to.map(str::to_string),
        ..Default::default()
    };
    let where_sql = where_clause(config, tenant_id, &range, &mut params);
    params.push(("bucket_secs".to_string(), bucket_secs.to_string()));
    (
        format!(
            "SELECT toString(toStartOfInterval(timestamp, toIntervalSecond({{bucket_secs:UInt32}}))) AS bucket, \
             count() AS total, \
             countIf(decision = 'allow') AS allows, \
             countIf(decision = 'deny') AS denies, \
             avg(evaluation_time_ns) AS avg_evaluation_time_ns \
             FROM decisions FINAL {where_sql} \
             GROUP BY bucket ORDER BY bucket"
        ),
        params,
    )
}

/// One UNION ALL query returning (facet, value, count) triples for every
/// filterable dimension, top-50 each by frequency.
fn build_facets_sql(
    config: &DecisionStoreConfig,
    tenant_id: &str,
    from: Option<&str>,
    to: Option<&str>,
) -> (String, Vec<(String, String)>) {
    let mut params = Vec::new();
    let range = DecisionQuery {
        from: from.map(str::to_string),
        to: to.map(str::to_string),
        ..Default::default()
    };
    let where_sql = where_clause(config, tenant_id, &range, &mut params);
    // Column names are a fixed allowlist here, never user input.
    let facet = |col: &str| {
        format!(
            "SELECT '{col}' AS facet, {col} AS value, count() AS count \
             FROM decisions FINAL {where_sql} GROUP BY value ORDER BY count DESC LIMIT 50"
        )
    };
    (
        [
            facet("action"),
            facet("decision"),
            facet("policy_name"),
            facet("agent_id"),
        ]
        .join(" UNION ALL "),
        params,
    )
}

/// Purge DELETE builder: tenant-scoped, cutoff-bounded, with one `NOT (...)`
/// exclusion per active hold. Every user value is bound as a server-side
/// parameter (`param_hN_field`) — never spliced into the SQL text. Blanket
/// holds are handled by the caller (purge is skipped entirely).
/// Append one active hold's exclusion clause to a mutation's `WHERE`.
///
/// A hold protects the rows its filter matches, so a purge/erasure must skip
/// them: this emits `NOT (<hold conjunction>)`, binding each dimension as a
/// server-side parameter (`h{i}_*`). Shared by [`build_purge_sql`] (retention
/// delete) and [`build_erase_subject_sql`] (subject redaction) so both honor
/// holds identically. Blanket holds never reach here (callers skip the whole
/// mutation); an empty conjunction fails SAFE — `"0"` matches no rows, so a
/// malformed hold protects everything rather than nothing.
fn push_hold_exclusion(
    hold: &HoldFilter,
    i: usize,
    conds: &mut Vec<String>,
    params: &mut Vec<(String, String)>,
) {
    let mut hold_conds: Vec<String> = Vec::new();
    let mut bind = |col_expr: &str, name_suffix: &str, value: &Option<String>| {
        if let Some(v) = value {
            let name = format!("h{i}_{name_suffix}");
            hold_conds.push(col_expr.replace("{p}", &format!("{{{name}:String}}")));
            params.push((name, v.clone()));
        }
    };
    bind("principal = {p}", "principal", &hold.principal);
    bind("action = {p}", "action", &hold.action);
    bind("resource = {p}", "resource", &hold.resource);
    bind("decision = {p}", "decision", &hold.decision);
    bind("policy_name = {p}", "policy_name", &hold.policy_name);
    bind("agent_id = {p}", "agent_id", &hold.agent_id);
    bind(
        "timestamp >= parseDateTime64BestEffort({p})",
        "from",
        &hold.from,
    );
    bind("timestamp < parseDateTime64BestEffort({p})", "to", &hold.to);
    if hold_conds.is_empty() {
        conds.push("0".to_string()); // matches no rows: mutate nothing
    } else {
        conds.push(format!("NOT ({})", hold_conds.join(" AND ")));
    }
}

fn build_purge_sql(
    config: &DecisionStoreConfig,
    tenant_id: &str,
    cutoff: &str,
    holds: &[HoldFilter],
) -> (String, Vec<(String, String)>) {
    let mut params = Vec::new();
    let mut conds: Vec<String> = Vec::new();

    if config.tenant_filter {
        conds.push("tenant_id = {tenant:String}".to_string());
        params.push(("tenant".to_string(), tenant_id.to_string()));
    }
    conds.push("timestamp < parseDateTime64BestEffort({cutoff:String})".to_string());
    params.push(("cutoff".to_string(), cutoff.to_string()));

    for (i, hold) in holds.iter().enumerate() {
        push_hold_exclusion(hold, i, &mut conds, &mut params);
    }

    (
        format!("ALTER TABLE decisions DELETE WHERE {}", conds.join(" AND ")),
        params,
    )
}

/// Tombstone written into redacted PII columns by subject erasure (E2). A fixed
/// sentinel so erased rows are self-describing and re-erasure is idempotent.
const ERASURE_TOMBSTONE: &str = "<erased>";

/// Build the redact-in-place mutation for subject erasure (E2).
///
/// Overwrites the PII-bearing columns (`principal`, `resource`, `context`,
/// `input_data`, `replay_input`) with tombstones on every row where the subject
/// appears as the actor *or* as the exact resource — while leaving
/// `seq`/`chain_id`/`prev_hash`/`entry_hash` untouched, so the tamper-evident
/// chain still verifies under `VerifyMode::Linkage` and checkpoint completeness
/// (`count`, `[seq_start, seq_end]`) is preserved. Active legal holds are
/// honored exactly as in a purge: a held row is never redacted.
///
/// When `pseudonyms` is supplied the selector also matches the tenant's
/// `sha256:<hmac>` principal/resource tokens, so a `pseudonymize`-profile tenant's
/// rows are reached too. The plaintext terms stay in place: a token can never
/// equal a plaintext identifier, so the union is exact for either profile.
fn build_erase_subject_sql(
    config: &DecisionStoreConfig,
    tenant_id: &str,
    subject: &str,
    pseudonyms: Option<&SubjectPseudonyms>,
    holds: &[HoldFilter],
) -> (String, Vec<(String, String)>) {
    let mut params = Vec::new();
    let mut conds: Vec<String> = Vec::new();

    if config.tenant_filter {
        conds.push("tenant_id = {tenant:String}".to_string());
        params.push(("tenant".to_string(), tenant_id.to_string()));
    }
    // Subject selector: plaintext principal/resource, plus the pseudonymised
    // tokens when the tenant hashes those columns. Each value is bound as a
    // server-side parameter, never spliced into the SQL text.
    let mut selector = vec![
        "principal = {subject:String}".to_string(),
        "resource = {subject:String}".to_string(),
    ];
    params.push(("subject".to_string(), subject.to_string()));
    if let Some(ps) = pseudonyms {
        selector.push("principal = {subject_hp:String}".to_string());
        selector.push("resource = {subject_hr:String}".to_string());
        params.push(("subject_hp".to_string(), ps.principal.clone()));
        params.push(("subject_hr".to_string(), ps.resource.clone()));
    }
    conds.push(format!("({})", selector.join(" OR ")));

    for (i, hold) in holds.iter().enumerate() {
        push_hold_exclusion(hold, i, &mut conds, &mut params);
    }

    params.push(("tomb".to_string(), ERASURE_TOMBSTONE.to_string()));
    (
        format!(
            "ALTER TABLE decisions UPDATE \
             principal = {{tomb:String}}, resource = {{tomb:String}}, \
             context = '', input_data = '', replay_input = '' \
             WHERE {}",
            conds.join(" AND ")
        ),
        params,
    )
}

fn build_top_denied_sql(
    config: &DecisionStoreConfig,
    tenant_id: &str,
    from: Option<&str>,
    to: Option<&str>,
) -> (String, Vec<(String, String)>) {
    let mut params = Vec::new();
    let range = DecisionQuery {
        from: from.map(str::to_string),
        to: to.map(str::to_string),
        decision: Some("deny".to_string()),
        ..Default::default()
    };
    let where_sql = where_clause(config, tenant_id, &range, &mut params);
    (
        format!(
            "SELECT policy_name, count() AS count FROM decisions FINAL {where_sql} \
             GROUP BY policy_name ORDER BY count DESC LIMIT 10"
        ),
        params,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(tenant_filter: bool) -> DecisionStoreConfig {
        DecisionStoreConfig {
            url: "http://localhost:8123".to_string(),
            database: "reaper_audit".to_string(),
            user: None,
            password: None,
            tenant_filter,
        }
    }

    #[test]
    fn list_sql_is_tenant_scoped_and_fully_parameterized() {
        let query = DecisionQuery {
            principal: Some("alice'; DROP TABLE decisions;--".to_string()),
            decision: Some("deny".to_string()),
            from: Some("2026-07-01T00:00:00Z".to_string()),
            limit: Some(50),
            ..Default::default()
        };
        let (sql, params) = build_list_sql(&config(true), "org-123", &query);

        // Injection attempt must never appear in the SQL text.
        assert!(!sql.contains("DROP TABLE"), "{sql}");
        assert!(sql.contains("tenant_id = {tenant:String}"));
        assert!(sql.contains("principal = {principal:String}"));
        assert!(sql.contains("ORDER BY timestamp DESC"));
        assert!(sql.contains("FINAL"), "must dedup at-least-once ingest");

        let get = |k: &str| params.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("tenant"), Some("org-123"));
        assert_eq!(get("principal"), Some("alice'; DROP TABLE decisions;--"));
        assert_eq!(get("limit"), Some("50"));
    }

    #[test]
    fn list_sql_caps_limit() {
        let query = DecisionQuery {
            limit: Some(1_000_000),
            ..Default::default()
        };
        let (_, params) = build_list_sql(&config(true), "t", &query);
        let limit = params.iter().find(|(n, _)| n == "limit").unwrap();
        // Cap is 1001: the API layer's max page is 1000 plus its has-more
        // sentinel row (Plan 07 Phase E).
        assert_eq!(limit.1, "1001");
    }

    #[test]
    fn list_sql_keyset_cursor_replaces_offset() {
        let query = DecisionQuery {
            after: Some(("2026-07-11 00:00:00.000".into(), "d-1".into())),
            offset: Some(50), // ignored once a cursor position is present
            ..Default::default()
        };
        let (sql, params) = build_list_sql(&config(true), "t", &query);
        assert!(sql.contains("(timestamp, decision_id) <"));
        assert!(sql.contains("ORDER BY timestamp DESC, decision_id DESC"));
        assert!(!sql.contains("OFFSET"));
        assert!(params.iter().any(|(n, _)| n == "cursor_ts"));
        assert!(params.iter().any(|(n, _)| n == "cursor_id"));
    }

    #[test]
    fn tenant_filter_can_be_disabled_for_single_tenant() {
        let (sql, params) = build_list_sql(&config(false), "ignored", &DecisionQuery::default());
        assert!(!sql.contains("tenant_id"));
        assert!(!params.iter().any(|(n, _)| n == "tenant"));
    }

    #[test]
    fn get_sql_binds_decision_id() {
        let (sql, params) = build_get_sql(&config(true), "org-1", "abc-123");
        assert!(sql.contains("decision_id = {decision_id:String}"));
        assert!(sql.contains("tenant_id = {tenant:String}"));
        assert!(params
            .iter()
            .any(|(n, v)| n == "decision_id" && v == "abc-123"));
    }

    #[test]
    fn stats_sql_counts_by_decision() {
        let (sql, params) = build_stats_sql(&config(true), "org-1", Some("2026-07-01"), None);
        assert!(sql.contains("countIf(decision = 'allow')"));
        assert!(sql.contains("parseDateTime64BestEffort({from:String})"));
        assert!(params.iter().any(|(n, _)| n == "from"));
    }

    #[test]
    fn parse_row_decodes_embedded_json_strings() {
        let raw = serde_json::json!({
            "timestamp": "2026-07-04 10:00:00.000",
            "decision_id": "d-1",
            "trace_id": "",
            "principal": "sha256:abcd",
            "action": "read",
            "resource": "/x",
            "decision": "deny",
            "policy_id": "p-1",
            "policy_name": "pol",
            "policy_version": "3",
            "matched_rule": "",
            "evaluation_time_ns": 450u64,
            "cache_hit": 0u8,
            "agent_id": "agent-1",
            "context": "{\"ip\":\"10.0.0.1\"}",
            "input_data": "{\"enc\":\"aes256gcm\",\"nonce\":\"aa\",\"ciphertext\":\"bb\"}",
        });
        let row = parse_row(raw).unwrap();
        assert_eq!(row.context["ip"], "10.0.0.1");
        assert_eq!(row.input_data["enc"], "aes256gcm");
    }

    #[test]
    fn parse_row_decodes_replay_input() {
        let mut raw = serde_json::json!({
            "timestamp": "2026-07-04 10:00:00.000",
            "decision_id": "d-2",
            "trace_id": "",
            "principal": "alice",
            "action": "read",
            "resource": "/x",
            "decision": "allow",
            "policy_id": "p-1",
            "policy_name": "pol",
            "policy_version": "3",
            "matched_rule": "",
            "evaluation_time_ns": 450u64,
            "cache_hit": 0u8,
            "agent_id": "agent-1",
            "context": "{}",
            "input_data": "",
        });
        // Tier off at capture → null (row is NOT replayable).
        let row = parse_row(raw.clone()).unwrap();
        assert!(row.replay_input.is_null());

        // Tier on → the full request decodes structured.
        raw["replay_input"] = serde_json::json!(
            "{\"principal\":\"alice\",\"action\":\"read\",\"resource\":\"/x\",\"context\":{\"region\":\"eu\"}}"
        );
        let row = parse_row(raw).unwrap();
        assert_eq!(row.replay_input["context"]["region"], "eu");
        assert_eq!(row.replay_input["principal"], "alice");
    }

    #[test]
    fn timeseries_sql_buckets_and_binds() {
        let (sql, params) =
            build_timeseries_sql(&config(true), "org-1", Some("2026-07-01"), None, 300);
        assert!(sql.contains("toIntervalSecond({bucket_secs:UInt32})"));
        assert!(sql.contains("GROUP BY bucket ORDER BY bucket"));
        assert!(sql.contains("tenant_id = {tenant:String}"));
        assert!(params.iter().any(|(n, v)| n == "bucket_secs" && v == "300"));
    }

    #[test]
    fn facets_sql_unions_fixed_columns_only() {
        let (sql, params) = build_facets_sql(&config(true), "org-1", None, None);
        assert_eq!(sql.matches("UNION ALL").count(), 3);
        for col in ["action", "decision", "policy_name", "agent_id"] {
            assert!(sql.contains(&format!("'{col}' AS facet")));
        }
        assert!(params.iter().any(|(n, v)| n == "tenant" && v == "org-1"));
    }

    #[test]
    fn interval_parsing_units_and_clamps() {
        assert_eq!(parse_interval_secs(Some("30s")), 30);
        assert_eq!(parse_interval_secs(Some("5m")), 300);
        assert_eq!(parse_interval_secs(Some("1h")), 3600);
        assert_eq!(parse_interval_secs(Some("1d")), 86_400);
        assert_eq!(parse_interval_secs(Some("120")), 120, "raw seconds");
        assert_eq!(parse_interval_secs(None), 3600, "default 1h");
        assert_eq!(parse_interval_secs(Some("garbage")), 3600, "fallback 1h");
        assert_eq!(parse_interval_secs(Some("1s")), 10, "clamped up");
        assert_eq!(
            parse_interval_secs(Some("400d")),
            7 * 86_400,
            "clamped down"
        );
    }

    #[test]
    fn config_from_env_requires_url() {
        // No REAPER_CLICKHOUSE_URL in the test env → disabled.
        std::env::remove_var("REAPER_CLICKHOUSE_URL");
        assert!(DecisionStoreConfig::from_env().is_none());
    }

    /// End-to-end over a fake ClickHouse HTTP endpoint: verifies the request
    /// shape (POST body = SQL, `param_*` query args, auth headers) and
    /// JSONEachRow response parsing, without needing a real ClickHouse.
    #[tokio::test]
    async fn store_speaks_clickhouse_http_contract() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 16384];
            let n = sock.read(&mut buf).await.unwrap();
            let req = String::from_utf8_lossy(&buf[..n]).to_string();

            let row = r#"{"timestamp":"2026-07-04 10:00:00.000","decision_id":"d-1","trace_id":"","principal":"sha256:ab","action":"read","resource":"/x","decision":"deny","policy_id":"p","policy_name":"pol","policy_version":"1","matched_rule":"","evaluation_time_ns":"450","cache_hit":0,"agent_id":"a1","context":"{}","input_data":""}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{row}\n",
                row.len() + 1,
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
            req
        });

        let store = DecisionStore::new(DecisionStoreConfig {
            url: format!("http://{addr}"),
            database: "reaper_audit".to_string(),
            user: Some("svc".to_string()),
            password: Some("pw".to_string()),
            tenant_filter: true,
        });
        let rows = store
            .list(
                "org-1",
                &DecisionQuery {
                    decision: Some("deny".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].decision_id, "d-1");
        assert_eq!(rows[0].evaluation_time_ns, 450, "quoted UInt64 parsed");
        assert!(rows[0].input_data.is_null(), "empty input_data → null");

        let req = server.await.unwrap();
        assert!(req.starts_with("POST"), "SQL goes in a POST body");
        assert!(
            req.contains("param_tenant=org-1"),
            "tenant bound server-side"
        );
        assert!(req.contains("param_decision=deny"));
        assert!(req.contains("default_format=JSONEachRow"));
        assert!(req.contains("x-clickhouse-user: svc") || req.contains("X-ClickHouse-User: svc"));
        assert!(req.contains("SELECT"), "body carries the SQL");
    }

    // ---- Retention purge + legal holds (Plan 04 step 6) ----

    #[test]
    fn purge_sql_is_tenant_scoped_and_cutoff_bounded() {
        let (sql, params) = build_purge_sql(&config(true), "org-1", "2026-04-01T00:00:00Z", &[]);
        assert!(sql.starts_with("ALTER TABLE decisions DELETE WHERE"));
        assert!(sql.contains("tenant_id = {tenant:String}"));
        assert!(sql.contains("timestamp < parseDateTime64BestEffort({cutoff:String})"));
        let get = |k: &str| params.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("tenant"), Some("org-1"));
        assert_eq!(get("cutoff"), Some("2026-04-01T00:00:00Z"));
    }

    #[test]
    fn purge_sql_excludes_each_hold_and_binds_values() {
        let holds = vec![
            HoldFilter {
                principal: Some("alice'; DROP TABLE decisions;--".to_string()),
                decision: Some("deny".to_string()),
                ..Default::default()
            },
            HoldFilter {
                policy_name: Some("pci-policy".to_string()),
                from: Some("2026-01-01T00:00:00Z".to_string()),
                to: Some("2026-02-01T00:00:00Z".to_string()),
                ..Default::default()
            },
        ];
        let (sql, params) = build_purge_sql(&config(true), "org-1", "2026-04-01", &holds);

        // One NOT (...) exclusion per hold, values bound — never spliced.
        assert_eq!(sql.matches("NOT (").count(), 2, "{sql}");
        assert!(!sql.contains("DROP TABLE"), "{sql}");
        assert!(sql.contains(
            "NOT (principal = {h0_principal:String} AND decision = {h0_decision:String})"
        ));
        assert!(sql.contains("policy_name = {h1_policy_name:String}"));
        assert!(sql.contains("timestamp >= parseDateTime64BestEffort({h1_from:String})"));
        assert!(sql.contains("timestamp < parseDateTime64BestEffort({h1_to:String})"));

        let get = |k: &str| params.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("h0_principal"), Some("alice'; DROP TABLE decisions;--"));
        assert_eq!(get("h1_policy_name"), Some("pci-policy"));
    }

    #[test]
    fn purge_sql_empty_hold_guard_fails_safe() {
        // Blanket holds are handled by the caller (purge skipped), but if one
        // ever reached the builder the WHERE must match no rows (AND 0).
        let (sql, _) = build_purge_sql(
            &config(true),
            "org-1",
            "2026-04-01",
            &[HoldFilter::default()],
        );
        assert!(sql.ends_with("AND 0"), "{sql}");
    }

    #[test]
    fn erase_sql_redacts_pii_and_preserves_chain_columns() {
        let (sql, params) = build_erase_subject_sql(
            &config(true),
            "org-1",
            "alice'; DROP TABLE decisions;--",
            None,
            &[],
        );

        // Redact-in-place, not delete: rows (and their hash-chain fields) survive.
        assert!(sql.starts_with("ALTER TABLE decisions UPDATE"), "{sql}");
        // Every PII-bearing column is tombstoned.
        assert!(sql.contains("principal = {tomb:String}"), "{sql}");
        assert!(sql.contains("resource = {tomb:String}"), "{sql}");
        assert!(sql.contains("context = ''"), "{sql}");
        assert!(sql.contains("input_data = ''"), "{sql}");
        assert!(sql.contains("replay_input = ''"), "{sql}");
        // The tamper-evidence columns must NOT be mutated, or Linkage verification
        // and checkpoint completeness would break.
        for chain_col in ["seq =", "chain_id =", "prev_hash =", "entry_hash ="] {
            assert!(
                !sql.contains(chain_col),
                "must not mutate {chain_col}: {sql}"
            );
        }
        // Subject bound as a parameter (both actor and exact-resource match), never spliced.
        assert!(sql.contains("(principal = {subject:String} OR resource = {subject:String})"));
        assert!(!sql.contains("DROP TABLE"), "{sql}");
        assert!(sql.contains("tenant_id = {tenant:String}"));

        let get = |k: &str| params.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("tenant"), Some("org-1"));
        assert_eq!(get("subject"), Some("alice'; DROP TABLE decisions;--"));
        assert_eq!(get("tomb"), Some(ERASURE_TOMBSTONE));
    }

    #[test]
    fn erase_sql_honors_holds_like_purge() {
        let holds = vec![
            HoldFilter {
                decision: Some("deny".to_string()),
                ..Default::default()
            },
            HoldFilter {
                policy_name: Some("pci-policy".to_string()),
                ..Default::default()
            },
        ];
        let (sql, params) = build_erase_subject_sql(&config(true), "org-1", "bob", None, &holds);

        // A held row is never redacted: one NOT (...) exclusion per hold.
        assert_eq!(sql.matches("NOT (").count(), 2, "{sql}");
        assert!(
            sql.contains("NOT (decision = {h0_decision:String})"),
            "{sql}"
        );
        assert!(
            sql.contains("NOT (policy_name = {h1_policy_name:String})"),
            "{sql}"
        );
        // Subject selector still present alongside the exclusions.
        assert!(sql.contains("principal = {subject:String}"));
        let get = |k: &str| params.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("subject"), Some("bob"));
    }

    #[test]
    fn erase_sql_without_pseudonyms_matches_plaintext_only() {
        let (sql, params) = build_erase_subject_sql(&config(true), "org-1", "alice", None, &[]);
        // Only the two plaintext terms — no hashed-column match when the tenant
        // is not pseudonymised.
        assert!(sql.contains("(principal = {subject:String} OR resource = {subject:String})"));
        assert!(!sql.contains("subject_hp"), "{sql}");
        assert!(!sql.contains("subject_hr"), "{sql}");
        assert!(!params.iter().any(|(n, _)| n == "subject_hp"));
    }

    #[test]
    fn erase_sql_matches_pseudonymized_columns_when_salt_given() {
        // A `pseudonymize`-profile tenant stores sha256:<hmac> tokens in
        // principal/resource. The control plane computes the tokens from the
        // tenant salt and the erasure must match them *as well as* any plaintext
        // rows — so one call covers a tenant regardless of profile.
        let salt = b"tenant-secret";
        let pseudonyms = SubjectPseudonyms {
            principal: policy_engine::pseudonymize(salt, "alice"),
            resource: policy_engine::pseudonymize_domain(salt, "resource", "alice"),
        };
        let (sql, params) =
            build_erase_subject_sql(&config(true), "org-1", "alice", Some(&pseudonyms), &[]);

        // Redact-in-place is unchanged; the selector gains the hashed terms.
        assert!(sql.starts_with("ALTER TABLE decisions UPDATE"), "{sql}");
        assert!(sql.contains("principal = {subject:String}"), "{sql}");
        assert!(sql.contains("resource = {subject:String}"), "{sql}");
        assert!(sql.contains("principal = {subject_hp:String}"), "{sql}");
        assert!(sql.contains("resource = {subject_hr:String}"), "{sql}");

        let get = |k: &str| params.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("subject"), Some("alice"));
        // Principal token is the un-domain-separated HMAC; resource token is
        // domain-separated — the two are distinct and matched against their own
        // columns (no cross-column correlation).
        assert_eq!(get("subject_hp"), Some(pseudonyms.principal.as_str()));
        assert_eq!(get("subject_hr"), Some(pseudonyms.resource.as_str()));
        assert_ne!(pseudonyms.principal, pseudonyms.resource);
        assert!(pseudonyms.principal.starts_with("sha256:"));
    }

    #[test]
    fn erase_sql_pseudonyms_honor_holds_too() {
        let holds = vec![HoldFilter {
            decision: Some("deny".to_string()),
            ..Default::default()
        }];
        let pseudonyms = SubjectPseudonyms {
            principal: "sha256:aa".to_string(),
            resource: "sha256:bb".to_string(),
        };
        let (sql, _) =
            build_erase_subject_sql(&config(true), "org-1", "bob", Some(&pseudonyms), &holds);
        // Hold exclusion applies to the whole (plaintext OR hashed) selector.
        assert_eq!(sql.matches("NOT (").count(), 1, "{sql}");
        assert!(sql.contains("principal = {subject_hp:String}"), "{sql}");
    }

    #[test]
    fn verify_decisions_sql_is_scoped_and_ordered_by_chain_seq() {
        let (sql, params) = build_verify_decisions_sql(
            &config(true),
            Some("org-1"),
            Some("boot-abc"),
            Some(0),
            Some(99),
        );
        // Reconstruction needs the chain fields.
        assert!(sql.contains("seq, chain_id, prev_hash, entry_hash"));
        // Never trust physical table order — verification order is explicit.
        assert!(sql.contains("ORDER BY chain_id, seq"));
        assert!(sql.contains("tenant_id = {tenant:String}"));
        assert!(sql.contains("chain_id = {chain:String}"));
        assert!(sql.contains("seq >= {seq_from:UInt64}"));
        assert!(sql.contains("seq <= {seq_to:UInt64}"));
        let get = |k: &str| params.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("tenant"), Some("org-1"));
        assert_eq!(get("chain"), Some("boot-abc"));
        assert_eq!(get("seq_from"), Some("0"));
        assert_eq!(get("seq_to"), Some("99"));
    }

    #[test]
    fn verify_checkpoints_sql_injects_record_type_and_scopes() {
        let (sql, params) =
            build_verify_checkpoints_sql(&config(true), Some("org-1"), Some("boot-abc"));
        assert!(sql.contains("'checkpoint' AS record_type"));
        assert!(sql.contains("toString(wallclock) AS wallclock"));
        assert!(sql.contains("tenant_id = {tenant:String}"));
        assert!(sql.contains("chain_id = {chain:String}"));
        assert!(params.iter().any(|(n, v)| n == "tenant" && v == "org-1"));
        assert!(params.iter().any(|(n, v)| n == "chain" && v == "boot-abc"));
    }

    #[test]
    fn verify_sql_single_tenant_omits_tenant_filter() {
        let (sql, params) = build_verify_decisions_sql(&config(false), None, None, None, None);
        assert!(!sql.contains("tenant_id"));
        assert!(!params.iter().any(|(n, _)| n == "tenant"));
        // No filters at all → no WHERE clause.
        assert!(!sql.contains("WHERE"));
    }

    #[test]
    fn blanket_hold_detection() {
        assert!(HoldFilter::default().is_blanket());
        assert!(!HoldFilter {
            principal: Some("alice".to_string()),
            ..Default::default()
        }
        .is_blanket());
        // Round-trips through the stored JSON representation.
        let f = HoldFilter {
            action: Some("read".to_string()),
            ..Default::default()
        };
        let json = serde_json::to_string(&f).unwrap();
        assert_eq!(json, r#"{"action":"read"}"#, "Nones are omitted");
        let back: HoldFilter = serde_json::from_str(&json).unwrap();
        assert_eq!(back, f);
        // An unparseable stored filter must fall back to blanket (protective).
        let fallback: HoldFilter = serde_json::from_str("{}").unwrap();
        assert!(fallback.is_blanket());
    }

    #[tokio::test]
    async fn purge_skips_entirely_under_blanket_hold_without_touching_store() {
        // Store URL points nowhere: if the blanket short-circuit failed, the
        // HTTP call would error — proving no DELETE is ever attempted.
        let store = DecisionStore::new(DecisionStoreConfig {
            url: "http://127.0.0.1:1".to_string(),
            database: "reaper_audit".to_string(),
            user: None,
            password: None,
            tenant_filter: true,
        });
        let outcome = store
            .purge_expired("org-1", "2026-04-01", &[HoldFilter::default()])
            .await
            .unwrap();
        assert_eq!(outcome, PurgeOutcome::SkippedBlanketHold);
    }

    /// Purge round trip over a fake ClickHouse endpoint: the DELETE mutation
    /// text and every hold parameter arrive bound server-side.
    #[tokio::test]
    async fn purge_speaks_clickhouse_http_contract() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 16384];
            let n = sock.read(&mut buf).await.unwrap();
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let resp = "HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
            sock.write_all(resp.as_bytes()).await.unwrap();
            req
        });

        let store = DecisionStore::new(DecisionStoreConfig {
            url: format!("http://{addr}"),
            database: "reaper_audit".to_string(),
            user: None,
            password: None,
            tenant_filter: true,
        });
        let holds = vec![HoldFilter {
            principal: Some("alice".to_string()),
            ..Default::default()
        }];
        let outcome = store
            .purge_expired("org-1", "2026-04-01T00:00:00Z", &holds)
            .await
            .unwrap();
        assert_eq!(
            outcome,
            PurgeOutcome::Submitted {
                cutoff: "2026-04-01T00:00:00Z".to_string(),
                holds_honored: 1
            }
        );

        let req = server.await.unwrap();
        assert!(req.contains("ALTER TABLE decisions DELETE WHERE"));
        assert!(req.contains("param_tenant=org-1"));
        assert!(req.contains("param_h0_principal=alice"));
        // URL-encoded cutoff (colons encode as %3A).
        assert!(
            req.contains("param_cutoff=2026-04-01T00%3A00%3A00Z")
                || req.contains("param_cutoff=2026-04-01T00:00:00Z")
        );
    }
}
