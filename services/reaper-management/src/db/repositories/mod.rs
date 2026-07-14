//! Database repositories
//!
//! Provides data access layers for all domain entities.

pub mod agent;
pub mod agent_deployment;
pub mod audit_erasure;
pub mod audit_governance;
pub mod bundle;
pub mod change_request;
pub mod datastore;
pub mod deployment;
pub mod environment;
pub mod idempotency;
pub mod namespace;
pub mod organization;
pub mod policy;
pub mod promotion;
pub mod revocation;
pub mod source;
pub mod team;
pub mod webhook;

pub use agent::AgentRepository;
pub use agent_deployment::{AgentDeploymentRepository, RollbackConfigRepository};
pub use audit_erasure::{AuditErasureRepository, ErasureRecord, NewErasureRecord};
pub use audit_governance::{AuditGovernanceRepository, AuditRetention, LegalHold};
pub use bundle::BundleRepository;
pub use change_request::ChangeRequestRepository;
pub use datastore::DatastoreRepository;
pub use deployment::DeploymentRepository;
pub use environment::EnvironmentRepository;
pub use idempotency::IdempotencyRepository;
pub use namespace::NamespaceRepository;
pub use organization::OrganizationRepository;
pub use policy::PolicyRepository;
pub use promotion::PromotionChangeRepository;
pub use revocation::{RevocationEntry, RevocationKind, RevocationRepository, RevocationSet};
pub use source::PolicySourceRepository;
pub use team::TeamRepository;
pub use webhook::WebhookRepository;
