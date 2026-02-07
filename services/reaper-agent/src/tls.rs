//! TLS/mTLS support for Reaper Agent
//!
//! Provides HTTPS server configuration with optional mutual TLS authentication.
//!
//! # Usage
//!
//! Enable TLS via environment variables:
//! ```bash
//! REAPER_TLS_ENABLED=true
//! REAPER_TLS_CERT=/certs/server.crt
//! REAPER_TLS_KEY=/certs/server.key
//! REAPER_TLS_CA=/certs/ca.crt
//! REAPER_TLS_REQUIRE_CLIENT_CERT=true
//! ```

use axum_server::tls_rustls::RustlsConfig;
use reaper_core::config::TlsSettings;
use rustls::server::WebPkiClientVerifier;
use rustls::RootCertStore;
use rustls_pemfile::{certs, private_key};
use std::fs::File;
use std::io::BufReader;
use std::sync::Arc;
use tracing::info;

/// TLS configuration errors
#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    #[error("TLS cert file not specified")]
    CertNotSpecified,

    #[error("TLS key file not specified")]
    KeyNotSpecified,

    #[error("CA file required for mTLS but not specified")]
    CaNotSpecified,

    #[error("Failed to read cert file: {0}")]
    CertReadError(String),

    #[error("Failed to read key file: {0}")]
    KeyReadError(String),

    #[error("Failed to read CA file: {0}")]
    CaReadError(String),

    #[error("No certificates found in cert file")]
    NoCertsFound,

    #[error("No private key found in key file")]
    NoKeyFound,

    #[error("Failed to build TLS config: {0}")]
    ConfigBuildError(String),

    #[error("Rustls error: {0}")]
    RustlsError(#[from] rustls::Error),
}

/// Create rustls server config from TLS settings
pub async fn create_tls_config(settings: &TlsSettings) -> Result<RustlsConfig, TlsError> {
    // Validate required files
    let cert_path = settings
        .cert_file
        .as_ref()
        .ok_or(TlsError::CertNotSpecified)?;

    let key_path = settings
        .key_file
        .as_ref()
        .ok_or(TlsError::KeyNotSpecified)?;

    info!(
        "Loading TLS certificates: cert={:?}, key={:?}",
        cert_path, key_path
    );

    // Load certificates
    let cert_file = File::open(cert_path).map_err(|e| TlsError::CertReadError(e.to_string()))?;
    let mut cert_reader = BufReader::new(cert_file);
    let certs_vec: Vec<_> = certs(&mut cert_reader).filter_map(|r| r.ok()).collect();

    if certs_vec.is_empty() {
        return Err(TlsError::NoCertsFound);
    }

    info!("Loaded {} certificate(s)", certs_vec.len());

    // Load private key
    let key_file = File::open(key_path).map_err(|e| TlsError::KeyReadError(e.to_string()))?;
    let mut key_reader = BufReader::new(key_file);
    let key = private_key(&mut key_reader)
        .map_err(|e| TlsError::KeyReadError(e.to_string()))?
        .ok_or(TlsError::NoKeyFound)?;

    info!("Private key loaded successfully");

    // Build server config
    if settings.require_client_cert {
        // mTLS mode - require client certificates
        let ca_path = settings.ca_file.as_ref().ok_or(TlsError::CaNotSpecified)?;

        info!("mTLS enabled - loading CA from {:?}", ca_path);

        // Load CA certificates for client verification
        let ca_file = File::open(ca_path).map_err(|e| TlsError::CaReadError(e.to_string()))?;
        let mut ca_reader = BufReader::new(ca_file);
        let ca_certs: Vec<_> = certs(&mut ca_reader).filter_map(|r| r.ok()).collect();

        if ca_certs.is_empty() {
            return Err(TlsError::CaReadError(
                "No CA certificates found".to_string(),
            ));
        }

        info!("Loaded {} CA certificate(s)", ca_certs.len());

        // Build root cert store for client verification
        let mut root_store = RootCertStore::empty();
        for cert in ca_certs {
            root_store
                .add(cert)
                .map_err(|e| TlsError::ConfigBuildError(format!("Failed to add CA cert: {}", e)))?;
        }

        // Create client verifier that requires valid certificates
        let client_verifier = WebPkiClientVerifier::builder(Arc::new(root_store))
            .build()
            .map_err(|e| TlsError::ConfigBuildError(format!("Failed to build verifier: {}", e)))?;

        // Build rustls config with client verification
        let config = rustls::ServerConfig::builder()
            .with_client_cert_verifier(client_verifier)
            .with_single_cert(certs_vec, key.into())
            .map_err(|e| TlsError::RustlsError(e))?;

        info!("mTLS configuration complete - client certificates required");

        Ok(RustlsConfig::from_config(Arc::new(config)))
    } else {
        // Standard TLS mode - no client certificate required
        info!("TLS mode (no client cert required)");

        RustlsConfig::from_pem_file(cert_path, key_path)
            .await
            .map_err(|e| TlsError::ConfigBuildError(e.to_string()))
    }
}

/// Validate TLS settings before starting server
pub fn validate_tls_settings(settings: &TlsSettings) -> Result<(), TlsError> {
    if !settings.enabled {
        return Ok(());
    }

    // Check cert file exists
    if let Some(ref cert_path) = settings.cert_file {
        if !cert_path.exists() {
            return Err(TlsError::CertReadError(format!(
                "Certificate file not found: {:?}",
                cert_path
            )));
        }
    } else {
        return Err(TlsError::CertNotSpecified);
    }

    // Check key file exists
    if let Some(ref key_path) = settings.key_file {
        if !key_path.exists() {
            return Err(TlsError::KeyReadError(format!(
                "Key file not found: {:?}",
                key_path
            )));
        }
    } else {
        return Err(TlsError::KeyNotSpecified);
    }

    // Check CA file if mTLS is required
    if settings.require_client_cert {
        if let Some(ref ca_path) = settings.ca_file {
            if !ca_path.exists() {
                return Err(TlsError::CaReadError(format!(
                    "CA file not found: {:?}",
                    ca_path
                )));
            }
        } else {
            return Err(TlsError::CaNotSpecified);
        }
    }

    Ok(())
}
