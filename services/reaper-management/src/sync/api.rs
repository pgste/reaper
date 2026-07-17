//! External API synchronization
//!
//! Fetches policies from external HTTP APIs.

use serde_json::Value as JsonValue;
use thiserror::Error;
use tracing::{debug, info};

use crate::domain::source::{ApiConfig, PolicySource, SyncResult};

/// API sync errors
#[derive(Debug, Error)]
pub enum ApiSyncError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Configuration error: {0}")]
    Config(String),
    #[error("JSONPath error: {0}")]
    JsonPath(String),
    #[error("Parse error: {0}")]
    Parse(String),
}

/// API syncer for fetching policies from external APIs
pub struct ApiSyncer {
    client: reqwest::Client,
}

/// SSRF pre-flight for a user-configured API source URL: require https to a
/// public address (round-3 SEC R3-5). Rejection is a configuration error.
async fn guard_api_url(url: &str) -> Result<(), ApiSyncError> {
    crate::url_guard::validate_public_https_url(url)
        .await
        .map_err(|crate::url_guard::UrlGuardError::NotAllowed(reason)| {
            ApiSyncError::Config(format!("API source URL blocked: {reason}"))
        })
}

impl ApiSyncer {
    /// Create a new API syncer
    pub fn new() -> Self {
        Self {
            client: crate::http::build_or_default(
                crate::http::http_client_builder(std::time::Duration::from_secs(30))
                    // Never follow redirects: a public host that passes the SSRF
                    // pre-flight guard could otherwise 302 to an internal address /
                    // cloud metadata (round-3 SEC R3-5).
                    .redirect(reqwest::redirect::Policy::none()),
            ),
        }
    }

    /// Sync a policy source
    pub async fn sync(&self, source: &PolicySource) -> Result<SyncResult, ApiSyncError> {
        let start = std::time::Instant::now();

        let config = source
            .api_config()
            .ok_or_else(|| ApiSyncError::Config("Invalid API configuration".to_string()))?;

        // SSRF guard: the API URL and its auth header are user-configured, so an
        // unguarded fetch probes the internal network / cloud metadata and leaks
        // the api_key to it (round-3 SEC R3-5). Require https to a public address
        // before any bytes — combined with the no-redirect client above.
        guard_api_url(&config.url).await?;

        // Build the request
        let mut request = match config.method.to_uppercase().as_str() {
            "GET" => self.client.get(&config.url),
            "POST" => {
                let mut req = self.client.post(&config.url);
                if let Some(body) = &config.body {
                    req = req.json(body);
                }
                req
            }
            method => {
                return Err(ApiSyncError::Config(format!(
                    "Unsupported HTTP method: {}",
                    method
                )))
            }
        };

        // Add headers
        for (key, value) in &config.headers {
            request = request.header(key, value);
        }

        // Add API key header if configured
        if let (Some(header), Some(key)) = (&config.api_key_header, &config.api_key) {
            request = request.header(header, key);
        }

        // Execute request
        debug!("Fetching policies from API: {}", config.url);
        let response = request.send().await?;

        if !response.status().is_success() {
            return Err(ApiSyncError::Config(format!(
                "API returned error status: {}",
                response.status()
            )));
        }

        // Parse response
        let body = response.text().await?;
        let policies = self.parse_response(&body, &config)?;

        let duration_ms = start.elapsed().as_millis() as u64;

        info!(
            source_id = %source.id,
            policies_found = policies.len(),
            duration_ms = duration_ms,
            "API sync completed"
        );

        Ok(SyncResult {
            source_id: source.id,
            success: true,
            policies_found: policies.len(),
            policies_updated: policies.len(),
            policies_created: 0,
            commit: None,
            error: None,
            duration_ms,
        })
    }

    /// Parse the API response and extract policies
    fn parse_response(
        &self,
        body: &str,
        config: &ApiConfig,
    ) -> Result<Vec<ApiPolicy>, ApiSyncError> {
        // Parse based on format
        let data: JsonValue = match config.format.as_str() {
            "json" => serde_json::from_str(body)?,
            "yaml" => serde_yaml::from_str(body).map_err(|e| ApiSyncError::Parse(e.to_string()))?,
            format => {
                return Err(ApiSyncError::Config(format!(
                    "Unsupported response format: {}",
                    format
                )))
            }
        };

        // Extract policies using JSONPath if configured
        let policies_data = if let Some(jsonpath) = &config.jsonpath {
            self.apply_jsonpath(&data, jsonpath)?
        } else {
            // Assume the response is an array of policies
            match &data {
                JsonValue::Array(arr) => arr.clone(),
                _ => vec![data],
            }
        };

        // Convert to ApiPolicy objects
        let mut policies = Vec::new();
        for (index, policy_data) in policies_data.iter().enumerate() {
            let policy = self.parse_policy(policy_data, index)?;
            policies.push(policy);
        }

        Ok(policies)
    }

    /// Apply JSONPath to extract data
    fn apply_jsonpath(&self, data: &JsonValue, path: &str) -> Result<Vec<JsonValue>, ApiSyncError> {
        use jsonpath_rust::JsonPath;

        // jsonpath-rust 1.0: `JsonPath` is a trait on `serde_json::Value`;
        // `query` parses the path and returns the matched nodes (borrowed),
        // which we clone into owned values.
        let matched = data
            .query(path)
            .map_err(|e| ApiSyncError::JsonPath(format!("Invalid JSONPath '{}': {:?}", path, e)))?;

        Ok(matched.into_iter().cloned().collect())
    }

    /// Parse a single policy from JSON
    fn parse_policy(&self, data: &JsonValue, index: usize) -> Result<ApiPolicy, ApiSyncError> {
        // Try to extract common fields
        let name = data
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("policy_{}", index));

        let content = if let Some(content) = data.get("content").and_then(|v| v.as_str()) {
            content.to_string()
        } else if data.get("rules").is_some() {
            // If there's a "rules" field, serialize the whole policy as JSON
            serde_json::to_string_pretty(data).unwrap_or_default()
        } else {
            // Serialize the entire object as the policy content
            serde_json::to_string_pretty(data).unwrap_or_default()
        };

        let language = data
            .get("language")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "reaper".to_string());

        let description = data
            .get("description")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let version = data
            .get("version")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        Ok(ApiPolicy {
            name,
            content,
            language,
            description,
            version,
        })
    }

    /// Get policies from the last sync (re-fetches from API)
    pub async fn get_policies(
        &self,
        source: &PolicySource,
    ) -> Result<Vec<ApiPolicy>, ApiSyncError> {
        let config = source
            .api_config()
            .ok_or_else(|| ApiSyncError::Config("Invalid API configuration".to_string()))?;

        // SSRF guard (round-3 SEC R3-5) — see `sync()`.
        guard_api_url(&config.url).await?;

        // Build the request
        let mut request = match config.method.to_uppercase().as_str() {
            "GET" => self.client.get(&config.url),
            "POST" => {
                let mut req = self.client.post(&config.url);
                if let Some(body) = &config.body {
                    req = req.json(body);
                }
                req
            }
            method => {
                return Err(ApiSyncError::Config(format!(
                    "Unsupported HTTP method: {}",
                    method
                )))
            }
        };

        // Add headers
        for (key, value) in &config.headers {
            request = request.header(key, value);
        }

        // Add API key header if configured
        if let (Some(header), Some(key)) = (&config.api_key_header, &config.api_key) {
            request = request.header(header, key);
        }

        let response = request.send().await?;

        if !response.status().is_success() {
            return Err(ApiSyncError::Config(format!(
                "API returned error status: {}",
                response.status()
            )));
        }

        let body = response.text().await?;
        self.parse_response(&body, &config)
    }
}

impl Default for ApiSyncer {
    fn default() -> Self {
        Self::new()
    }
}

/// A policy fetched from an external API
#[derive(Debug, Clone)]
pub struct ApiPolicy {
    /// Policy name
    pub name: String,
    /// Policy content
    pub content: String,
    /// Policy language
    pub language: String,
    /// Optional description
    pub description: Option<String>,
    /// API-provided version
    pub version: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn api_url_guard_blocks_internal_and_non_https() {
        // http, cloud metadata, and private ranges are refused (round-3 R3-5)…
        assert!(guard_api_url("http://example.com/policies").await.is_err());
        assert!(guard_api_url("https://169.254.169.254/latest/meta-data/")
            .await
            .is_err());
        assert!(guard_api_url("https://10.0.0.5/policies").await.is_err());
        // …while a public https endpoint is allowed (IP literal: no DNS needed).
        assert!(guard_api_url("https://1.1.1.1/policies").await.is_ok());
    }

    #[test]
    fn test_parse_policy_minimal() {
        let syncer = ApiSyncer::new();
        let data = serde_json::json!({
            "name": "test-policy",
            "content": "allow admin to access /admin"
        });

        let policy = syncer.parse_policy(&data, 0).unwrap();
        assert_eq!(policy.name, "test-policy");
        assert_eq!(policy.content, "allow admin to access /admin");
        assert_eq!(policy.language, "reaper");
    }

    #[test]
    fn test_parse_policy_full() {
        let syncer = ApiSyncer::new();
        let data = serde_json::json!({
            "name": "auth-policy",
            "content": "when principal.role == 'admin' allow",
            "language": "cedar",
            "description": "Admin authentication policy",
            "version": "1.2.3"
        });

        let policy = syncer.parse_policy(&data, 0).unwrap();
        assert_eq!(policy.name, "auth-policy");
        assert_eq!(policy.language, "cedar");
        assert_eq!(
            policy.description,
            Some("Admin authentication policy".to_string())
        );
        assert_eq!(policy.version, Some("1.2.3".to_string()));
    }

    #[test]
    fn test_parse_policy_auto_name() {
        let syncer = ApiSyncer::new();
        let data = serde_json::json!({
            "content": "allow all"
        });

        let policy = syncer.parse_policy(&data, 5).unwrap();
        assert_eq!(policy.name, "policy_5");
    }
}
