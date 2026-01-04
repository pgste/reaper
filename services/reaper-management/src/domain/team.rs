//! Team domain model
//!
//! Teams belong to organizations and can own policies.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Team entity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Team {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating a new team
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTeam {
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
}

/// Input for updating a team
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateTeam {
    pub name: Option<String>,
    pub description: Option<String>,
}

impl Team {
    /// Get the full path for this team (org_slug/team_slug)
    pub fn full_path(&self, org_slug: &str) -> String {
        format!("{}/{}", org_slug, self.slug)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_team_full_path() {
        let team = Team {
            id: Uuid::new_v4(),
            org_id: Uuid::new_v4(),
            name: "Engineering".to_string(),
            slug: "engineering".to_string(),
            description: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        assert_eq!(team.full_path("acme"), "acme/engineering");
    }
}
