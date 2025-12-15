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
