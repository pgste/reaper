//! Rate limiting configuration

use serde::{Deserialize, Serialize};

use super::error::ConfigError;

/// Rate limiting configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RateLimitConfig {
    /// Enable rate limiting
    #[serde(default = "default_rate_limit_enabled")]
    pub enabled: bool,
    /// Requests per second (global)
    #[serde(default = "default_requests_per_second")]
    pub requests_per_second: u32,
    /// Burst size (bucket capacity)
    #[serde(default = "default_burst_size")]
    pub burst_size: u32,
    /// Signup requests per hour per IP
    #[serde(default = "default_signup_per_hour")]
    pub signup_per_hour: u32,
    /// Login attempts per minute per IP
    #[serde(default = "default_login_per_minute")]
    pub login_per_minute: u32,
    /// API requests per org per minute
    #[serde(default = "default_api_per_org_per_minute")]
    pub api_per_org_per_minute: u32,
    /// Whether to trust client-supplied forwarding headers (X-Forwarded-For,
    /// X-Real-IP, Forwarded) for the client IP used in per-IP rate limits.
    ///
    /// Default false: those headers are trivially spoofable, so trusting them
    /// lets an attacker bypass per-IP login/signup limits by rotating the
    /// header value. Only enable this when the server sits behind a trusted
    /// reverse proxy that sets these headers.
    #[serde(default = "default_trust_proxy_headers")]
    pub trust_proxy_headers: bool,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: default_rate_limit_enabled(),
            requests_per_second: default_requests_per_second(),
            burst_size: default_burst_size(),
            signup_per_hour: default_signup_per_hour(),
            login_per_minute: default_login_per_minute(),
            api_per_org_per_minute: default_api_per_org_per_minute(),
            trust_proxy_headers: default_trust_proxy_headers(),
        }
    }
}

impl RateLimitConfig {
    /// Validate rate limit configuration
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.enabled {
            if self.requests_per_second == 0 {
                return Err(ConfigError::InvalidRateLimit(
                    "requests_per_second must be positive".to_string(),
                ));
            }

            if self.burst_size == 0 {
                return Err(ConfigError::InvalidRateLimit(
                    "burst_size must be positive".to_string(),
                ));
            }

            // Burst size should be >= requests_per_second for proper token bucket
            if self.burst_size < self.requests_per_second {
                tracing::warn!(
                    "burst_size ({}) is less than requests_per_second ({}), this may cause issues",
                    self.burst_size,
                    self.requests_per_second
                );
            }
        }

        Ok(())
    }
}

fn default_rate_limit_enabled() -> bool {
    true
}

fn default_requests_per_second() -> u32 {
    100
}

fn default_burst_size() -> u32 {
    200
}

fn default_signup_per_hour() -> u32 {
    5
}

fn default_login_per_minute() -> u32 {
    10
}

fn default_api_per_org_per_minute() -> u32 {
    1000
}

fn default_trust_proxy_headers() -> bool {
    false
}
