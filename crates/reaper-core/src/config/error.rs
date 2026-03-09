//! Configuration error types.

use std::path::PathBuf;

/// Configuration errors.
#[derive(Debug, Clone)]
pub enum ConfigError {
    /// Failed to read configuration file
    FileRead(PathBuf, String),
    /// Failed to parse configuration
    Parse(String),
    /// Configuration validation failed
    Validation(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::FileRead(path, err) => {
                write!(f, "Failed to read config file {:?}: {}", path, err)
            }
            ConfigError::Parse(err) => write!(f, "Failed to parse config: {}", err),
            ConfigError::Validation(err) => write!(f, "Config validation failed: {}", err),
        }
    }
}

impl std::error::Error for ConfigError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_error_display() {
        let err = ConfigError::FileRead(PathBuf::from("/test"), "not found".to_string());
        assert!(err.to_string().contains("/test"));
        assert!(err.to_string().contains("not found"));

        let err = ConfigError::Parse("invalid yaml".to_string());
        assert!(err.to_string().contains("invalid yaml"));

        let err = ConfigError::Validation("port is 0".to_string());
        assert!(err.to_string().contains("port is 0"));
    }
}
