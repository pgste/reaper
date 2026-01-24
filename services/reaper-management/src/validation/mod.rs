//! Policy validation module
//!
//! Provides syntax validation and test evaluation for policies before promotion.

mod service;

pub use service::{
    PolicyValidationResult, TestCase, TestResult, ValidationError, ValidationService,
};
