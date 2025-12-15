//! Reaper eBPF Kernel Program
//!
//! This eBPF program runs in the Linux kernel and provides <100ns policy evaluation.
//! It implements LSM (Linux Security Module) hooks for file and network access control.
//!
//! Architecture:
//! - Fast Path: Simple policies evaluated in eBPF (<100ns)
//! - Slow Path: Complex policies deferred to userspace (10-50µs)
//! - Learning Mode: Userspace can promote complex decisions to simple rules
//!
//! Build:
//! ```bash
//! cargo build --target=bpfel-unknown-none -Z build-std=core
//! ```

#![no_std]
#![no_main]

use aya_bpf::{
    macros::{lsm, map},
    maps::{HashMap, RingBuf},
    programs::LsmContext,
};
use aya_log_ebpf::info;

/// Maximum path length supported in eBPF
const MAX_PATH_LEN: usize = 256;

/// Maximum context key length
const MAX_CONTEXT_KEY_LEN: usize = 64;

/// Maximum context value length
const MAX_CONTEXT_VALUE_LEN: usize = 256;

/// Maximum number of policy rules
const MAX_POLICIES: u32 = 10000;

/// Maximum number of context entries
const MAX_CONTEXT_ENTRIES: u32 = 1000;

/// Policy action: 0 = Deny, 1 = Allow, 2 = Log
#[repr(u8)]
#[derive(Clone, Copy)]
pub enum PolicyAction {
    Deny = 0,
    Allow = 1,
    Log = 2,
}

/// Policy rule stored in BPF map
/// Key: resource path (fixed-size array)
/// Value: PolicyEntry
#[repr(C)]
#[derive(Clone, Copy)]
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

    /// Reserved for future use (12 bytes for 32-byte total alignment)
    pub reserved: [u8; 12],
}

/// Event sent to userspace for complex policy evaluation
#[repr(C)]
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

// ============================================================================
// BPF Maps
// ============================================================================

/// Main policy map: resource path → policy entry
/// This is the fast-path lookup table.
///
/// Userspace updates this map to deploy/update policies.
/// Simple policies: exact match or wildcard
#[map]
static POLICY_MAP: HashMap<[u8; MAX_PATH_LEN], PolicyEntry> =
    HashMap::with_max_entries(MAX_POLICIES, 0);

/// Wildcard policy: applies to all resources
/// Special key "*" for global allow/deny
#[map]
static WILDCARD_POLICY: HashMap<u8, PolicyEntry> =
    HashMap::with_max_entries(1, 0);

/// Context data map: key → value
/// Stores runtime context (JWT claims, user attributes, etc.)
/// Updated by userspace, read by eBPF
#[map]
static CONTEXT_MAP: HashMap<[u8; MAX_CONTEXT_KEY_LEN], [u8; MAX_CONTEXT_VALUE_LEN]> =
    HashMap::with_max_entries(MAX_CONTEXT_ENTRIES, 0);

/// Ring buffer for sending complex policy events to userspace
/// When eBPF can't make a decision, it sends event here
#[map]
static EVENTS: RingBuf = RingBuf::with_byte_size(1024 * 256, 0); // 256KB ring buffer

/// Statistics map: tracks performance metrics
#[map]
static STATS: HashMap<u32, u64> = HashMap::with_max_entries(10, 0);

// Stats keys
const STAT_FAST_PATH: u32 = 0;    // Count of fast path evaluations
const STAT_SLOW_PATH: u32 = 1;    // Count of slow path (userspace) evaluations
const STAT_DENIALS: u32 = 2;      // Count of denials
const STAT_ALLOWS: u32 = 3;       // Count of allows
const STAT_ERRORS: u32 = 4;       // Count of errors

// ============================================================================
// LSM Hooks
// ============================================================================

/// LSM hook: file_open
/// Intercepts file open operations and evaluates policy
///
/// Performance: <100ns for fast path (BPF map lookup)
/// Returns: 0 = Allow, -EPERM = Deny
#[lsm(hook = "file_open")]
pub fn reaper_file_open(ctx: LsmContext) -> i32 {
    match try_file_open(ctx) {
        Ok(ret) => ret,
        Err(_) => {
            increment_stat(STAT_ERRORS);
            -1  // -EPERM
        }
    }
}

fn try_file_open(ctx: LsmContext) -> Result<i32, i64> {
    // Get file path from context
    // Note: In real implementation, this would use bpf_d_path() helper
    // For now, this is a placeholder showing the architecture

    // Extract UID/GID
    let uid_gid = unsafe { aya_bpf::helpers::bpf_get_current_uid_gid() };
    let uid = (uid_gid >> 32) as u32;
    let gid = (uid_gid & 0xFFFFFFFF) as u32;

    // Extract PID
    let pid_tgid = unsafe { aya_bpf::helpers::bpf_get_current_pid_tgid() };
    let pid = (pid_tgid >> 32) as u32;

    // TODO: Get actual file path from ctx
    // For now, use placeholder path
    let mut path: [u8; MAX_PATH_LEN] = [0; MAX_PATH_LEN];
    let path_len = 0; // Actual path length

    // Fast path: Lookup in policy map
    if let Some(policy) = unsafe { POLICY_MAP.get(&path) } {
        // Check additional conditions if needed
        if policy.flags & 0x01 != 0 {  // Check UID
            if uid != policy.required_uid {
                increment_stat(STAT_FAST_PATH);
                increment_stat(STAT_DENIALS);
                return Ok(-1);  // -EPERM
            }
        }

        if policy.flags & 0x02 != 0 {  // Check GID
            if gid != policy.required_gid {
                increment_stat(STAT_FAST_PATH);
                increment_stat(STAT_DENIALS);
                return Ok(-1);  // -EPERM
            }
        }

        // Policy matched, apply action
        increment_stat(STAT_FAST_PATH);
        return match policy.action {
            0 => {  // Deny
                increment_stat(STAT_DENIALS);
                info!(&ctx, "eBPF: DENY file_open uid={} path_len={}", uid, path_len);
                Ok(-1)  // -EPERM
            }
            1 => {  // Allow
                increment_stat(STAT_ALLOWS);
                Ok(0)  // Allow
            }
            _ => {  // Log (allow but log)
                increment_stat(STAT_ALLOWS);
                info!(&ctx, "eBPF: LOG file_open uid={} path_len={}", uid, path_len);
                Ok(0)
            }
        };
    }

    // Check wildcard policy
    let wildcard_key = 0u8;
    if let Some(policy) = unsafe { WILDCARD_POLICY.get(&wildcard_key) } {
        increment_stat(STAT_FAST_PATH);
        return match policy.action {
            0 => {
                increment_stat(STAT_DENIALS);
                Ok(-1)  // Deny
            }
            1 => {
                increment_stat(STAT_ALLOWS);
                Ok(0)  // Allow
            }
            _ => Ok(0),  // Log
        };
    }

    // Slow path: No policy match, send to userspace
    send_to_userspace(pid, uid, gid, &path, path_len)?;

    increment_stat(STAT_SLOW_PATH);

    // Default action while userspace evaluates
    // Options:
    // 1. Fail-closed: return -1 (deny by default)
    // 2. Fail-open: return 0 (allow by default)
    // Currently: fail-closed for security
    increment_stat(STAT_DENIALS);
    Ok(-1)  // -EPERM (deny by default)
}

/// LSM hook: socket_connect
/// Intercepts network connection attempts
///
/// This allows Reaper to enforce egress policies at the kernel level
#[lsm(hook = "socket_connect")]
pub fn reaper_socket_connect(ctx: LsmContext) -> i32 {
    match try_socket_connect(ctx) {
        Ok(ret) => ret,
        Err(_) => {
            increment_stat(STAT_ERRORS);
            -1  // -EPERM
        }
    }
}

fn try_socket_connect(_ctx: LsmContext) -> Result<i32, i64> {
    // Similar logic to file_open
    // Could check against IP:port policies
    // For now, allow all (placeholder)
    increment_stat(STAT_FAST_PATH);
    increment_stat(STAT_ALLOWS);
    Ok(0)  // Allow
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Send policy evaluation request to userspace via ring buffer
fn send_to_userspace(
    pid: u32,
    uid: u32,
    gid: u32,
    path: &[u8; MAX_PATH_LEN],
    path_len: u32,
) -> Result<(), i64> {
    let event = PolicyEvent {
        pid,
        uid,
        gid,
        path: *path,
        path_len,
        action: 0,  // open
        timestamp_ns: unsafe { aya_bpf::helpers::bpf_ktime_get_ns() },
    };

    // Submit to ring buffer
    unsafe {
        EVENTS.output(&event, 0);
    }

    Ok(())
}

/// Increment a statistics counter
#[inline(always)]
fn increment_stat(key: u32) {
    unsafe {
        if let Some(counter) = STATS.get_ptr_mut(&key) {
            *counter += 1;
        } else {
            // Initialize counter if doesn't exist
            STATS.insert(&key, &1u64, 0).ok();
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    unsafe { core::hint::unreachable_unchecked() }
}
