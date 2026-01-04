//! Organization domain model
//!
//! Organizations are the top-level multi-tenancy unit.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Organization entity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Organization {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub settings: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating a new organization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateOrganization {
    pub name: String,
    pub slug: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    #[serde(default = "default_settings")]
    pub settings: serde_json::Value,
}

fn default_settings() -> serde_json::Value {
    serde_json::json!({})
}

/// Input for updating an organization
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateOrganization {
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub settings: Option<serde_json::Value>,
}

impl Organization {
    /// Check if organization has a specific setting
    pub fn has_setting(&self, key: &str) -> bool {
        self.settings.get(key).is_some()
    }

    /// Get a setting value
    pub fn get_setting<T: serde::de::DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.settings
            .get(key)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_organization_defaults() {
        let input = CreateOrganization {
            name: "Test Org".to_string(),
            slug: "test-org".to_string(),
            display_name: None,
            description: None,
            settings: default_settings(),
        };

        assert_eq!(input.settings, serde_json::json!({}));
    }

    #[test]
    fn test_organization_settings() {
        let org = Organization {
            id: Uuid::new_v4(),
            name: "Test".to_string(),
            slug: "test".to_string(),
            display_name: None,
            description: None,
            settings: serde_json::json!({
                "max_agents": 100,
                "features": ["sse", "webhooks"]
            }),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        assert!(org.has_setting("max_agents"));
        assert!(!org.has_setting("nonexistent"));
        assert_eq!(org.get_setting::<i32>("max_agents"), Some(100));
    }
}
