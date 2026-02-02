//! Bundle compilation configuration

use serde::{Deserialize, Serialize};

/// Bundle compilation configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BundlesConfig {
    #[serde(default)]
    pub auto_compile_on_source_sync: bool,
    #[serde(default = "default_require_staged")]
    pub require_staged_before_promote: bool,
}

impl Default for BundlesConfig {
    fn default() -> Self {
        Self {
            auto_compile_on_source_sync: false,
            require_staged_before_promote: true,
        }
    }
}

fn default_require_staged() -> bool {
    true
}
