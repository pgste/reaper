//! Shared SSRF guard for user-configured outbound URLs (Plan 09 Step 4).
//!
//! Any URL an org admin can point the control plane at — JWKS endpoints,
//! OIDC discovery, git remotes — is an SSRF vector: `http://169.254.169.254/`
//! reads cloud metadata, `https://10.0.0.5/` probes the internal network.
//! This module is the single guard both the auth (JWKS) and sync (git) paths
//! call before any fetch: require https, resolve the host, and reject any
//! address in a disallowed range.
//!
//! Note: the check resolves the host and inspects the addresses; a determined
//! attacker could still attempt DNS rebinding between this check and the
//! actual fetch. That residual risk is much smaller than the unrestricted
//! fetch it replaces (same trade-off the JWKS guard documented).

use std::net::IpAddr;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum UrlGuardError {
    #[error("URL not allowed: {0}")]
    NotAllowed(String),
}

/// Reject IPs that must never be reachable via a user-configured URL —
/// loopback, private, link-local (incl. the 169.254.169.254 cloud metadata
/// endpoint), CGNAT, and IPv6 equivalents. This is the core SSRF guard.
pub fn is_disallowed_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || o[0] == 0
                // 100.64.0.0/10 carrier-grade NAT
                || (o[0] == 100 && (o[1] & 0xC0) == 0x40)
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                // unique local fc00::/7
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                // link-local fe80::/10
                || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

/// Validate an outbound URL before fetching: require HTTPS and ensure every
/// resolved address is a public IP. Blocks SSRF to internal services and
/// cloud metadata.
pub async fn validate_public_https_url(url: &str) -> Result<(), UrlGuardError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|_| UrlGuardError::NotAllowed("malformed URL".to_string()))?;

    if parsed.scheme() != "https" {
        return Err(UrlGuardError::NotAllowed("must use https".to_string()));
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| UrlGuardError::NotAllowed("missing host".to_string()))?;
    let port = parsed.port_or_known_default().unwrap_or(443);

    let addrs = tokio::net::lookup_host((host, port))
        .await
        .map_err(|_| UrlGuardError::NotAllowed("host does not resolve".to_string()))?;

    let mut resolved_any = false;
    for addr in addrs {
        resolved_any = true;
        if is_disallowed_ip(&addr.ip()) {
            return Err(UrlGuardError::NotAllowed(
                "resolves to a disallowed internal address".to_string(),
            ));
        }
    }
    if !resolved_any {
        return Err(UrlGuardError::NotAllowed(
            "host does not resolve".to_string(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn rejected(url: &str) -> bool {
        validate_public_https_url(url).await.is_err()
    }

    #[tokio::test]
    async fn public_https_ip_passes() {
        // IP literal, so no external DNS needed in the test environment.
        assert!(validate_public_https_url("https://1.1.1.1/repo.git")
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn http_is_rejected() {
        assert!(rejected("http://example.com/repo.git").await);
    }

    #[tokio::test]
    async fn loopback_private_metadata_and_cgnat_are_rejected() {
        for url in [
            "https://127.0.0.1/repo.git",
            "https://10.0.0.5/repo.git",
            "https://172.16.3.4/repo.git",
            "https://192.168.1.1/repo.git",
            "https://169.254.169.254/latest/meta-data/",
            "https://100.64.0.1/repo.git",
            "https://0.0.0.0/repo.git",
            "https://[::1]/repo.git",
            "https://[fd00::1]/repo.git",
            "https://[fe80::1]/repo.git",
        ] {
            assert!(rejected(url).await, "{url} must be rejected");
        }
    }

    #[tokio::test]
    async fn malformed_missing_host_and_non_resolving_are_rejected() {
        assert!(rejected("not a url").await);
        assert!(rejected("https:///nohost").await);
        assert!(rejected("https://definitely-not-a-real-host.invalid/repo.git").await);
    }

    #[tokio::test]
    async fn other_schemes_are_rejected() {
        assert!(rejected("file:///etc/passwd").await);
        assert!(rejected("ssh://git@github.com/org/repo.git").await);
        assert!(rejected("git://github.com/org/repo.git").await);
    }
}
