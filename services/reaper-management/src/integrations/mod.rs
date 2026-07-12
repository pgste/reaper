//! External integrations (ITSM change management).

pub mod servicenow;

pub use servicenow::{ChangeRecordCheck, ServiceNowClient, ServiceNowError};
