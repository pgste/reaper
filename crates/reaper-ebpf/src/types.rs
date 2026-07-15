//! Shared types between eBPF kernel program and userspace
//!
//! These types mirror the kernel-side structures defined in reaper-ebpf-kern.
//! They must be kept in sync with the eBPF program.

use serde::{Deserialize, Serialize};

/// Maximum path length in eBPF maps
pub const MAX_PATH_LEN: usize = 256;

/// Maximum context key length
pub const MAX_CONTEXT_KEY_LEN: usize = 64;

/// Maximum context value length
pub const MAX_CONTEXT_VALUE_LEN: usize = 256;

/// Policy action enum (matches kernel-side)
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyAction {
    Deny = 0,
    Allow = 1,
    Log = 2,
}

impl From<policy_engine::PolicyAction> for PolicyAction {
    fn from(action: policy_engine::PolicyAction) -> Self {
        match action {
            policy_engine::PolicyAction::Allow => PolicyAction::Allow,
            policy_engine::PolicyAction::Deny => PolicyAction::Deny,
            policy_engine::PolicyAction::Log => PolicyAction::Log,
        }
    }
}

impl From<PolicyAction> for policy_engine::PolicyAction {
    fn from(action: PolicyAction) -> Self {
        match action {
            PolicyAction::Allow => policy_engine::PolicyAction::Allow,
            PolicyAction::Deny => policy_engine::PolicyAction::Deny,
            PolicyAction::Log => policy_engine::PolicyAction::Log,
        }
    }
}

/// Policy entry stored in BPF maps (matches kernel-side exactly)
/// Total size: 32 bytes
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct PolicyEntry {
    /// Action to take (0=deny, 1=allow, 2=log)
    pub action: u8,

    /// Rule priority (lower number = higher priority)
    pub priority: u32,

    /// Flags for additional checks (bit field)
    /// Bit 0: Check UID
    /// Bit 1: Check GID
    /// Bit 2: Check context
    pub flags: u8,

    /// Required UID (if flags & 0x01)
    pub required_uid: u32,

    /// Required GID (if flags & 0x02)
    pub required_gid: u32,

    /// Reserved for future use (12 bytes to make total size 32 bytes with alignment)
    pub reserved: [u8; 12],
}

// SAFETY: PolicyEntry is #[repr(C)] with only Pod-safe types
unsafe impl aya::Pod for PolicyEntry {}

impl PolicyEntry {
    /// Create a new policy entry with minimal configuration
    pub fn new(action: PolicyAction) -> Self {
        Self {
            action: action as u8,
            priority: 0,
            flags: 0,
            required_uid: 0,
            required_gid: 0,
            reserved: [0; 12],
        }
    }

    /// Set UID requirement
    pub fn with_uid(mut self, uid: u32) -> Self {
        self.flags |= 0x01;
        self.required_uid = uid;
        self
    }

    /// Set GID requirement
    pub fn with_gid(mut self, gid: u32) -> Self {
        self.flags |= 0x02;
        self.required_gid = gid;
        self
    }

    /// Set priority
    pub fn with_priority(mut self, priority: u32) -> Self {
        self.priority = priority;
        self
    }
}

/// Event from eBPF kernel program (via ring buffer)
/// Total size: 280 bytes
#[repr(C)]
#[derive(Debug, Clone)]
pub struct PolicyEvent {
    /// Process ID that triggered the event
    pub pid: u32,

    /// User ID
    pub uid: u32,

    /// Group ID
    pub gid: u32,

    /// Resource path being accessed
    pub path: [u8; MAX_PATH_LEN],

    /// Path length (actual bytes used)
    pub path_len: u32,

    /// Action requested (open, read, write, execute)
    pub action: u32,

    /// Timestamp (nanoseconds)
    pub timestamp_ns: u64,
}

impl PolicyEvent {
    /// Extract path as string
    pub fn path_str(&self) -> String {
        let len = self.path_len.min(MAX_PATH_LEN as u32) as usize;
        String::from_utf8_lossy(&self.path[..len]).to_string()
    }

    /// Convert to PolicyRequest for evaluation
    pub fn to_policy_request(&self) -> policy_engine::PolicyRequest {
        use std::collections::HashMap;

        let mut context = HashMap::new();
        context.insert("uid".to_string(), self.uid.to_string());
        context.insert("gid".to_string(), self.gid.to_string());
        context.insert("pid".to_string(), self.pid.to_string());

        policy_engine::PolicyRequest {
            resource: self.path_str(),
            action: match self.action {
                0 => "open".to_string(),
                1 => "read".to_string(),
                2 => "write".to_string(),
                3 => "execute".to_string(),
                _ => "unknown".to_string(),
            },
            context,

            ..Default::default()
        }
    }
}

/// Statistics from eBPF program
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct EbpfStats {
    /// Count of fast path evaluations (eBPF)
    pub fast_path: u64,

    /// Count of slow path evaluations (userspace)
    pub slow_path: u64,

    /// Count of denials
    pub denials: u64,

    /// Count of allows
    pub allows: u64,

    /// Count of errors
    pub errors: u64,
}

impl EbpfStats {
    /// Total evaluations
    pub fn total(&self) -> u64 {
        self.fast_path + self.slow_path
    }

    /// Fast path percentage
    pub fn fast_path_percent(&self) -> f64 {
        if self.total() == 0 {
            0.0
        } else {
            (self.fast_path as f64 / self.total() as f64) * 100.0
        }
    }
}

/// Combined statistics (eBPF + userspace learning)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CombinedStats {
    /// eBPF fast path evaluations
    pub fast_path_evaluations: u64,

    /// Userspace slow path evaluations
    pub slow_path_evaluations: u64,

    /// Fast path percentage
    pub fast_path_percent: f64,

    /// Total denials
    pub denials: u64,

    /// Total allows
    pub allows: u64,

    /// Total errors
    pub errors: u64,

    /// Number of policies promoted from complex to eBPF
    pub promoted_policies: usize,

    /// Total policies in eBPF map
    pub ebpf_policy_count: usize,

    /// Total policies in userspace
    pub userspace_policy_count: usize,
}

/// Stats keys (must match kernel-side constants)
pub mod stats_keys {
    pub const FAST_PATH: u32 = 0;
    pub const SLOW_PATH: u32 = 1;
    pub const DENIALS: u32 = 2;
    pub const ALLOWS: u32 = 3;
    pub const ERRORS: u32 = 4;
}

// Ensure PolicyEntry is the right size for BPF maps
const _: () = assert!(std::mem::size_of::<PolicyEntry>() == 32);

// ===== Entity Change Event Tracking =====

/// Entity change events for auditing and debugging
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EntityEvent {
    /// Entity was created
    Created {
        entity_id: String,
        entity_type: String,
        timestamp: u64,
    },
    /// Entity was updated
    Updated {
        entity_id: String,
        entity_type: String,
        version: u32,
        timestamp: u64,
    },
    /// Entity was deleted
    Deleted {
        entity_id: String,
        entity_type: String,
        timestamp: u64,
    },
    /// Batch operation completed
    BatchCompleted {
        succeeded: usize,
        failed: usize,
        timestamp: u64,
    },
}

impl EntityEvent {
    /// Create a Created event
    pub fn created(entity_id: String, entity_type: String) -> Self {
        Self::Created {
            entity_id,
            entity_type,
            timestamp: current_timestamp_ns(),
        }
    }

    /// Create an Updated event
    pub fn updated(entity_id: String, entity_type: String, version: u32) -> Self {
        Self::Updated {
            entity_id,
            entity_type,
            version,
            timestamp: current_timestamp_ns(),
        }
    }

    /// Create a Deleted event
    pub fn deleted(entity_id: String, entity_type: String) -> Self {
        Self::Deleted {
            entity_id,
            entity_type,
            timestamp: current_timestamp_ns(),
        }
    }

    /// Create a BatchCompleted event
    pub fn batch_completed(succeeded: usize, failed: usize) -> Self {
        Self::BatchCompleted {
            succeeded,
            failed,
            timestamp: current_timestamp_ns(),
        }
    }

    /// Get the timestamp of this event
    pub fn timestamp(&self) -> u64 {
        match self {
            EntityEvent::Created { timestamp, .. } => *timestamp,
            EntityEvent::Updated { timestamp, .. } => *timestamp,
            EntityEvent::Deleted { timestamp, .. } => *timestamp,
            EntityEvent::BatchCompleted { timestamp, .. } => *timestamp,
        }
    }

    /// Get the entity ID if applicable
    pub fn entity_id(&self) -> Option<&str> {
        match self {
            EntityEvent::Created { entity_id, .. } => Some(entity_id),
            EntityEvent::Updated { entity_id, .. } => Some(entity_id),
            EntityEvent::Deleted { entity_id, .. } => Some(entity_id),
            EntityEvent::BatchCompleted { .. } => None,
        }
    }

    /// Get the event type as a string
    pub fn event_type(&self) -> &'static str {
        match self {
            EntityEvent::Created { .. } => "created",
            EntityEvent::Updated { .. } => "updated",
            EntityEvent::Deleted { .. } => "deleted",
            EntityEvent::BatchCompleted { .. } => "batch_completed",
        }
    }
}

/// Entity event log for tracking changes
#[derive(Debug, Clone)]
pub struct EntityEventLog {
    events: std::sync::Arc<std::sync::RwLock<std::collections::VecDeque<EntityEvent>>>,
    max_events: usize,
}

impl EntityEventLog {
    /// Create a new event log with default capacity (1000 events)
    pub fn new() -> Self {
        Self::with_capacity(1000)
    }

    /// Create a new event log with specified capacity
    pub fn with_capacity(max_events: usize) -> Self {
        Self {
            events: std::sync::Arc::new(std::sync::RwLock::new(
                std::collections::VecDeque::with_capacity(max_events),
            )),
            max_events,
        }
    }

    /// Record an event
    pub fn record(&self, event: EntityEvent) {
        let mut events = self.events.write().unwrap();

        // If at capacity, remove oldest event
        if events.len() >= self.max_events {
            events.pop_front();
        }

        events.push_back(event);
    }

    /// Get recent events (up to limit)
    pub fn get_recent(&self, limit: usize) -> Vec<EntityEvent> {
        let events = self.events.read().unwrap();
        events.iter().rev().take(limit).cloned().collect()
    }

    /// Get all events
    pub fn get_all(&self) -> Vec<EntityEvent> {
        let events = self.events.read().unwrap();
        events.iter().cloned().collect()
    }

    /// Get events for a specific entity
    pub fn get_for_entity(&self, entity_id: &str) -> Vec<EntityEvent> {
        let events = self.events.read().unwrap();
        events
            .iter()
            .filter(|e| e.entity_id() == Some(entity_id))
            .cloned()
            .collect()
    }

    /// Get event count
    pub fn count(&self) -> usize {
        let events = self.events.read().unwrap();
        events.len()
    }

    /// Clear all events
    pub fn clear(&self) {
        let mut events = self.events.write().unwrap();
        events.clear();
    }

    /// Get statistics about events
    pub fn stats(&self) -> EntityEventStats {
        let events = self.events.read().unwrap();
        let mut stats = EntityEventStats::default();

        for event in events.iter() {
            match event {
                EntityEvent::Created { .. } => stats.created += 1,
                EntityEvent::Updated { .. } => stats.updated += 1,
                EntityEvent::Deleted { .. } => stats.deleted += 1,
                EntityEvent::BatchCompleted {
                    succeeded, failed, ..
                } => {
                    stats.batch_operations += 1;
                    stats.batch_succeeded += succeeded;
                    stats.batch_failed += failed;
                }
            }
        }

        stats.total = events.len();
        stats
    }
}

impl Default for EntityEventLog {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics about entity events
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EntityEventStats {
    pub total: usize,
    pub created: usize,
    pub updated: usize,
    pub deleted: usize,
    pub batch_operations: usize,
    pub batch_succeeded: usize,
    pub batch_failed: usize,
}

/// Get current timestamp in nanoseconds
fn current_timestamp_ns() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_entity_event_creation() {
        let event = EntityEvent::created("user:alice".to_string(), "user".to_string());

        assert_eq!(event.entity_id(), Some("user:alice"));
        assert_eq!(event.event_type(), "created");
        assert!(event.timestamp() > 0);
    }

    #[test]
    fn test_entity_event_types() {
        let created = EntityEvent::created("test".to_string(), "user".to_string());
        let updated = EntityEvent::updated("test".to_string(), "user".to_string(), 2);
        let deleted = EntityEvent::deleted("test".to_string(), "user".to_string());
        let batch = EntityEvent::batch_completed(10, 2);

        assert_eq!(created.event_type(), "created");
        assert_eq!(updated.event_type(), "updated");
        assert_eq!(deleted.event_type(), "deleted");
        assert_eq!(batch.event_type(), "batch_completed");

        // Batch event should not have entity_id
        assert!(batch.entity_id().is_none());
    }

    #[test]
    fn test_entity_event_log_basic_operations() {
        let log = EntityEventLog::with_capacity(5);

        // Record some events
        log.record(EntityEvent::created(
            "user:alice".to_string(),
            "user".to_string(),
        ));
        log.record(EntityEvent::updated(
            "user:alice".to_string(),
            "user".to_string(),
            2,
        ));
        log.record(EntityEvent::deleted(
            "user:bob".to_string(),
            "user".to_string(),
        ));

        // Check count
        assert_eq!(log.count(), 3);

        // Get recent events
        let recent = log.get_recent(2);
        assert_eq!(recent.len(), 2);

        // Most recent should be the delete event
        assert_eq!(recent[0].event_type(), "deleted");
        assert_eq!(recent[1].event_type(), "updated");
    }

    #[test]
    fn test_entity_event_log_capacity() {
        let log = EntityEventLog::with_capacity(3);

        // Add more events than capacity
        for i in 0..5 {
            log.record(EntityEvent::created(
                format!("user:{}", i),
                "user".to_string(),
            ));
        }

        // Should only have 3 events (oldest dropped)
        assert_eq!(log.count(), 3);

        let all = log.get_all();
        // First event should be user:2 (user:0 and user:1 were dropped)
        assert_eq!(all[0].entity_id(), Some("user:2"));
    }

    #[test]
    fn test_entity_event_log_filter_by_entity() {
        let log = EntityEventLog::new();

        log.record(EntityEvent::created(
            "user:alice".to_string(),
            "user".to_string(),
        ));
        log.record(EntityEvent::updated(
            "user:alice".to_string(),
            "user".to_string(),
            2,
        ));
        log.record(EntityEvent::created(
            "user:bob".to_string(),
            "user".to_string(),
        ));
        log.record(EntityEvent::deleted(
            "user:alice".to_string(),
            "user".to_string(),
        ));

        let alice_events = log.get_for_entity("user:alice");
        assert_eq!(alice_events.len(), 3);

        let bob_events = log.get_for_entity("user:bob");
        assert_eq!(bob_events.len(), 1);
    }

    #[test]
    fn test_entity_event_log_stats() {
        let log = EntityEventLog::new();

        log.record(EntityEvent::created(
            "user:alice".to_string(),
            "user".to_string(),
        ));
        log.record(EntityEvent::created(
            "user:bob".to_string(),
            "user".to_string(),
        ));
        log.record(EntityEvent::updated(
            "user:alice".to_string(),
            "user".to_string(),
            2,
        ));
        log.record(EntityEvent::deleted(
            "user:bob".to_string(),
            "user".to_string(),
        ));
        log.record(EntityEvent::batch_completed(10, 2));

        let stats = log.stats();

        assert_eq!(stats.total, 5);
        assert_eq!(stats.created, 2);
        assert_eq!(stats.updated, 1);
        assert_eq!(stats.deleted, 1);
        assert_eq!(stats.batch_operations, 1);
        assert_eq!(stats.batch_succeeded, 10);
        assert_eq!(stats.batch_failed, 2);
    }

    #[test]
    fn test_entity_event_log_clear() {
        let log = EntityEventLog::new();

        log.record(EntityEvent::created(
            "user:alice".to_string(),
            "user".to_string(),
        ));
        log.record(EntityEvent::created(
            "user:bob".to_string(),
            "user".to_string(),
        ));

        assert_eq!(log.count(), 2);

        log.clear();

        assert_eq!(log.count(), 0);
        assert!(log.get_all().is_empty());
    }

    #[test]
    fn test_entity_event_stats_default() {
        let stats = EntityEventStats::default();

        assert_eq!(stats.total, 0);
        assert_eq!(stats.created, 0);
        assert_eq!(stats.updated, 0);
        assert_eq!(stats.deleted, 0);
        assert_eq!(stats.batch_operations, 0);
    }
}
