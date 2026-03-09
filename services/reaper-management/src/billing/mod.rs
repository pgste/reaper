//! Billing module
//!
//! Stripe integration for subscription billing.

mod service;

pub use service::{BillingConfig, BillingError, BillingService};
