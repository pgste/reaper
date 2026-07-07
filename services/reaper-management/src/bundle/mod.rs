//! Bundle compilation and management service
//!
//! Handles bundle compilation, storage, and promotion workflow.

pub mod compiler;
pub mod service;

pub use compiler::BundleCompiler;
pub use service::{BundleDownloadResult, BundleError, BundleService, BundleSigner};
