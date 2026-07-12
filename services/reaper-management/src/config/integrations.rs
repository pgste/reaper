//! External-integration configuration (ITSM change management).

use serde::{Deserialize, Serialize};

/// External integrations. All optional — nothing here is required for a
/// default deployment.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct IntegrationsConfig {
    /// ServiceNow instance used to validate external change records when an
    /// environment's approval policy sets `external_change_record: validated`.
    #[serde(default)]
    pub servicenow: Option<ServiceNowConfig>,
}

/// Connection to a ServiceNow instance (Table API, basic auth).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServiceNowConfig {
    /// Instance base URL, e.g. `https://acme.service-now.com`.
    pub base_url: String,
    /// Basic-auth username.
    #[serde(default)]
    pub username: String,
    /// Basic-auth password / API token. Prefer the
    /// `REAPER_SERVICENOW_TOKEN` env var over putting this in a config file.
    #[serde(default, skip_serializing)]
    pub api_token: Option<String>,
    /// `approval` field values accepted as a deployable change record.
    /// Defaults to `["approved"]`.
    #[serde(default = "default_accepted_approvals")]
    pub accepted_approvals: Vec<String>,
}

fn default_accepted_approvals() -> Vec<String> {
    vec!["approved".to_string()]
}

impl ServiceNowConfig {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            username: String::new(),
            api_token: None,
            accepted_approvals: default_accepted_approvals(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_no_integrations() {
        let cfg = IntegrationsConfig::default();
        assert!(cfg.servicenow.is_none());
    }

    #[test]
    fn servicenow_yaml_parses_with_defaults() {
        let cfg: IntegrationsConfig = serde_yaml::from_str(
            "servicenow:\n  base_url: https://acme.service-now.com\n  username: reaper\n",
        )
        .unwrap();
        let snow = cfg.servicenow.unwrap();
        assert_eq!(snow.base_url, "https://acme.service-now.com");
        assert_eq!(snow.accepted_approvals, vec!["approved"]);
        assert!(snow.api_token.is_none());
    }

    #[test]
    fn api_token_is_never_serialized() {
        let mut snow = ServiceNowConfig::new("https://acme.service-now.com");
        snow.api_token = Some("s3cret".to_string());
        let yaml = serde_yaml::to_string(&snow).unwrap();
        assert!(!yaml.contains("s3cret"));
    }
}
