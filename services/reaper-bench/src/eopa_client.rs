//! Enterprise OPA HTTP client for benchmark comparison
//!
//! Standalone HTTP client for the OPA REST API using `reqwest` directly.
//! This is separate from the Reaper SDK because OPA has a different API contract.

use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

/// Enterprise OPA HTTP client
pub struct EopaClient {
    client: reqwest::Client,
    base_url: String,
}

/// OPA input envelope sent to the data API
#[derive(Debug, Clone, Serialize)]
pub struct OpaInput {
    pub principal: OpaInputPrincipal,
    pub action: String,
    pub resource: String,
}

/// Principal information for OPA input
#[derive(Debug, Clone, Serialize)]
pub struct OpaInputPrincipal {
    pub id: String,
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub department: Option<String>,
}

/// OPA evaluation result
#[derive(Debug, Clone)]
pub struct OpaEvalResult {
    pub allowed: bool,
    pub duration: Duration,
}

/// OPA data API response wrapper
#[derive(Debug, Deserialize)]
struct OpaDataResponse {
    result: Option<bool>,
}

/// OPA health response
#[derive(Debug, Deserialize)]
pub struct OpaHealthResponse {
    #[serde(default)]
    pub status: Option<String>,
}

impl EopaClient {
    /// Create a new eOPA client pointing at the given base URL (e.g. "http://localhost:8181").
    pub fn new(base_url: &str) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .pool_max_idle_per_host(128)
            .build()
            .expect("failed to build reqwest client");

        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    /// Health check — `GET /health`
    pub async fn health(&self) -> anyhow::Result<bool> {
        let url = format!("{}/health", self.base_url);
        let resp = self.client.get(&url).send().await?;
        Ok(resp.status().is_success())
    }

    /// Load data into OPA — `PUT /v1/data` with JSON body.
    ///
    /// Accepts a serde_json::Value representing the full data document.
    pub async fn load_data(&self, data: &serde_json::Value) -> anyhow::Result<()> {
        let url = format!("{}/v1/data", self.base_url);
        let resp = self.client.put(&url).json(data).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("OPA load_data failed ({}): {}", status, body);
        }
        debug!("OPA data loaded successfully");
        Ok(())
    }

    /// Load a Rego policy into OPA — `PUT /v1/policies/{id}` with text/plain body.
    pub async fn load_policy(&self, id: &str, rego: &str) -> anyhow::Result<()> {
        let url = format!("{}/v1/policies/{}", self.base_url, id);
        let resp = self
            .client
            .put(&url)
            .header("Content-Type", "text/plain")
            .body(rego.to_string())
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("OPA load_policy '{}' failed ({}): {}", id, status, body);
        }
        debug!("OPA policy '{}' loaded successfully", id);
        Ok(())
    }

    /// Evaluate a policy rule — `POST /v1/data/{package_path}/{rule}` with `{"input": {...}}`.
    ///
    /// Returns the boolean result and the round-trip duration.
    pub async fn evaluate(
        &self,
        package_path: &str,
        rule: &str,
        input: &OpaInput,
    ) -> anyhow::Result<OpaEvalResult> {
        let url = format!("{}/v1/data/{}/{}", self.base_url, package_path, rule);
        let body = serde_json::json!({ "input": input });

        let start = Instant::now();
        let resp = self.client.post(&url).json(&body).send().await?;
        let duration = start.elapsed();

        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            anyhow::bail!("OPA evaluate failed ({}): {}", status, body_text);
        }

        let opa_resp: OpaDataResponse = resp.json().await?;
        Ok(OpaEvalResult {
            allowed: opa_resp.result.unwrap_or(false),
            duration,
        })
    }

    /// Get the base URL for this client.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }
}

/// Transform Reaper entity data to OPA-compatible format.
///
/// Reaper stores entities as arrays: `[{id: "foo", ...}, ...]`
/// OPA policies expect maps keyed by entity ID: `{"foo": {...}, "bar": {...}}`
///
/// This matches how `.rego` policies reference `data.entities[input.principal]`.
pub fn transform_entities_for_opa(reaper_data: &serde_json::Value) -> serde_json::Value {
    let mut opa_data = serde_json::Map::new();

    // Convert entity arrays to maps keyed by ID
    if let Some(entities) = reaper_data.get("entities") {
        if let Some(arr) = entities.as_array() {
            let mut entity_map = serde_json::Map::new();
            for entity in arr {
                if let Some(id) = entity.get("id").and_then(|v| v.as_str()) {
                    entity_map.insert(id.to_string(), entity.clone());
                }
            }
            opa_data.insert(
                "entities".to_string(),
                serde_json::Value::Object(entity_map),
            );
        } else if entities.is_object() {
            // Already a map — use as-is
            opa_data.insert("entities".to_string(), entities.clone());
        }
    }

    // Copy any other top-level keys as-is
    if let Some(obj) = reaper_data.as_object() {
        for (key, value) in obj {
            if key != "entities" {
                opa_data.insert(key.clone(), value.clone());
            }
        }
    }

    serde_json::Value::Object(opa_data)
}

/// Load all .rego policy files from a directory into the eOPA instance.
pub async fn load_rego_policies_from_dir(
    client: &EopaClient,
    dir: &str,
) -> anyhow::Result<Vec<String>> {
    let path = std::path::Path::new(dir);
    if !path.exists() {
        anyhow::bail!("OPA policies directory does not exist: {}", dir);
    }

    let mut loaded = Vec::new();

    let entries = std::fs::read_dir(path)?;
    for entry in entries.flatten() {
        let file_path = entry.path();
        if file_path.extension().map(|e| e == "rego").unwrap_or(false) {
            let filename = file_path.file_stem().unwrap().to_string_lossy().to_string();
            let rego_content = std::fs::read_to_string(&file_path)?;

            match client.load_policy(&filename, &rego_content).await {
                Ok(_) => {
                    info!("  eOPA policy loaded: {}", filename);
                    loaded.push(filename);
                }
                Err(e) => {
                    warn!("  eOPA policy '{}' failed: {}", filename, e);
                }
            }
        }
    }

    Ok(loaded)
}
