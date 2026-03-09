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

mod entity;

use aya_ebpf::{
    macros::{lsm, map},
    maps::{HashMap, RingBuf},
    programs::LsmContext,
};
use aya_log_ebpf::info;
use entity::{Entity, MAX_ENTITY_ID_LEN};

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
// Entity Maps (Tier 1: Direct maps for < 10K entities)
// ============================================================================

/// User entities map
/// Stores user attributes, JWT claims, and relationships
#[map]
static USERS: HashMap<[u8; MAX_ENTITY_ID_LEN], Entity> =
    HashMap::with_max_entries(10000, 0);

/// Role entities map (for RBAC)
/// Stores role definitions and associated permissions
#[map]
static ROLES: HashMap<[u8; MAX_ENTITY_ID_LEN], Entity> =
    HashMap::with_max_entries(1000, 0);

/// Group entities map (for hierarchical access)
/// Stores group memberships and hierarchies
#[map]
static GROUPS: HashMap<[u8; MAX_ENTITY_ID_LEN], Entity> =
    HashMap::with_max_entries(5000, 0);

/// Resource entities map (for ABAC)
/// Stores protected resource attributes
#[map]
static RESOURCES: HashMap<[u8; MAX_ENTITY_ID_LEN], Entity> =
    HashMap::with_max_entries(10000, 0);

/// JWT session entities map (for authentication)
/// Stores active JWT sessions with claims
#[map]
static JWT_SESSIONS: HashMap<[u8; MAX_ENTITY_ID_LEN], Entity> =
    HashMap::with_max_entries(50000, 0);

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

/// Apply a policy entry with UID/GID checks
///
/// Returns the policy decision (0 = allow, -1 = deny)
#[inline(always)]
fn apply_policy(
    ctx: &LsmContext,
    policy: &PolicyEntry,
    uid: u32,
    gid: u32,
    path_len: u32,
) -> Result<i32, i64> {
    // Check UID requirement if enabled
    if policy.flags & 0x01 != 0 {
        // UID check enabled
        if uid != policy.required_uid {
            increment_stat(STAT_FAST_PATH);
            increment_stat(STAT_DENIALS);
            return Ok(-1); // -EPERM (deny)
        }
    }

    // Check GID requirement if enabled
    if policy.flags & 0x02 != 0 {
        // GID check enabled
        if gid != policy.required_gid {
            increment_stat(STAT_FAST_PATH);
            increment_stat(STAT_DENIALS);
            return Ok(-1); // -EPERM (deny)
        }
    }

    // All checks passed, apply policy action
    increment_stat(STAT_FAST_PATH);
    match policy.action {
        0 => {
            // Deny
            increment_stat(STAT_DENIALS);
            info!(ctx, "eBPF: DENY uid={} path_len={}", uid, path_len);
            Ok(-1) // -EPERM
        }
        1 => {
            // Allow
            increment_stat(STAT_ALLOWS);
            Ok(0) // Allow
        }
        _ => {
            // Log (allow but log)
            increment_stat(STAT_ALLOWS);
            info!(ctx, "eBPF: LOG uid={} path_len={}", uid, path_len);
            Ok(0)
        }
    }
}

fn try_file_open(ctx: LsmContext) -> Result<i32, i64> {
    // Extract UID/GID
    let uid_gid = aya_ebpf::helpers::bpf_get_current_uid_gid();
    let uid = (uid_gid >> 32) as u32;
    let gid = (uid_gid & 0xFFFFFFFF) as u32;

    // Extract PID
    let pid_tgid = aya_ebpf::helpers::bpf_get_current_pid_tgid();
    let pid = (pid_tgid >> 32) as u32;

    // Extract file path from LSM context
    // LSM file_open signature: int file_open(struct file *file)
    let mut path: [u8; MAX_PATH_LEN] = [0; MAX_PATH_LEN];
    let path_len = extract_file_path(&ctx, &mut path)?;

    // Fast path: Lookup in policy map (exact match)
    if let Some(policy) = unsafe { POLICY_MAP.get(&path) } {
        return apply_policy(&ctx, policy, uid, gid, path_len);
    }

    // Check for wildcard entry in POLICY_MAP (key[0] == 0xFF)
    let mut wildcard_key: [u8; MAX_PATH_LEN] = [0; MAX_PATH_LEN];
    wildcard_key[0] = 0xFF;  // Wildcard marker
    if let Some(policy) = unsafe { POLICY_MAP.get(&wildcard_key) } {
        return apply_policy(&ctx, policy, uid, gid, path_len);
    }

    // Check global wildcard policy (separate map)
    let global_wildcard_key = 0u8;
    if let Some(policy) = unsafe { WILDCARD_POLICY.get(&global_wildcard_key) } {
        return apply_policy(&ctx, policy, uid, gid, path_len);
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

/// LSM hook: inode_permission
/// Intercepts inode permission checks (read, write, execute)
///
/// This is called for most file operations and is a critical hook
/// for comprehensive file access control.
#[lsm(hook = "inode_permission")]
pub fn reaper_inode_permission(ctx: LsmContext) -> i32 {
    match try_inode_permission(ctx) {
        Ok(ret) => ret,
        Err(_) => {
            increment_stat(STAT_ERRORS);
            0  // Allow on error to prevent breaking the system
        }
    }
}

fn try_inode_permission(_ctx: LsmContext) -> Result<i32, i64> {
    // LSM inode_permission signature: int inode_permission(struct inode *inode, int mask)
    // mask contains: MAY_READ, MAY_WRITE, MAY_EXEC, MAY_APPEND, etc.

    // Extract UID/GID for policy check
    let uid_gid = aya_ebpf::helpers::bpf_get_current_uid_gid();
    let _uid = (uid_gid >> 32) as u32;
    let _gid = (uid_gid & 0xFFFFFFFF) as u32;

    // For now, we'll allow all inode operations
    // In production, you'd:
    // 1. Extract the path from inode
    // 2. Check against POLICY_MAP
    // 3. Consider the mask (read/write/exec)

    // Allow by default to prevent system breakage
    increment_stat(STAT_FAST_PATH);
    increment_stat(STAT_ALLOWS);
    Ok(0)
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
            0  // Allow on error for network operations
        }
    }
}

fn try_socket_connect(_ctx: LsmContext) -> Result<i32, i64> {
    // LSM socket_connect signature: int socket_connect(struct socket *sock, struct sockaddr *address, int addrlen)
    // Could extract IP:port from address and check policies
    // For now, allow all network connections
    increment_stat(STAT_FAST_PATH);
    increment_stat(STAT_ALLOWS);
    Ok(0)  // Allow
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Extract file path from LSM context
///
/// This uses BPF helpers to extract the path from the file struct.
/// Returns the path length on success.
///
/// # Full Implementation (requires kernel 5.9+ and vmlinux.h bindings):
///
/// ```c
/// // Get file pointer from LSM context
/// struct file *file = (struct file *)ctx->args[0];
///
/// // Extract path using bpf_d_path helper
/// struct path *path = &file->f_path;
/// long ret = bpf_d_path(path, path_buf, MAX_PATH_LEN);
/// if (ret < 0) {
///     return ret;  // Error
/// }
/// return ret;  // Path length
/// ```
///
/// # Current Implementation:
///
/// aya-ebpf 0.1 doesn't expose bpf_d_path or provide vmlinux bindings.
/// For now, we extract the process command name as a proxy identifier.
/// This allows testing the architecture end-to-end while we work on:
///
/// 1. Upgrading to newer aya version with more helpers
/// 2. Adding vmlinux.h bindings for kernel struct access
/// 3. Implementing full bpf_d_path extraction
///
fn extract_file_path(_ctx: &LsmContext, path_buf: &mut [u8; MAX_PATH_LEN]) -> Result<u32, i64> {
    // TODO: Full path extraction requires:
    // 1. Access to file pointer from ctx (ctx as *const _ as *const *const c_void)
    // 2. Reading file->f_path with bpf_probe_read_kernel()
    // 3. Calling bpf_d_path(&file->f_path, buf, len)
    //
    // This needs either:
    // - Upgrading to aya-ebpf with bpf_d_path support
    // - Generating vmlinux.h and using raw FFI
    // - Using aya-ebpf codegen for kernel struct access

    // For now: Extract process command name as identifier
    // This allows testing the full architecture (fast path, slow path, learning)
    // without kernel struct access complexity

    // Get current process command (comm)
    // SAFETY: bpf_get_current_comm is safe to call from eBPF context
    let comm = match aya_ebpf::helpers::bpf_get_current_comm() {
        Ok(comm_buf) => comm_buf,
        Err(_) => {
            return Err(-1);
        }
    };

    // Copy comm to path_buf with a prefix to indicate it's a comm, not a path
    // Format: "comm:<process_name>"
    let prefix = b"comm:";
    let prefix_len = prefix.len();

    // Copy prefix
    for i in 0..prefix_len {
        if i < MAX_PATH_LEN {
            path_buf[i] = prefix[i];
        }
    }

    // Copy comm (up to null terminator)
    let mut comm_len = 0;
    for i in 0..comm.len() {
        if comm[i] == 0 {
            break;
        }
        if prefix_len + i < MAX_PATH_LEN {
            path_buf[prefix_len + i] = comm[i];
            comm_len += 1;
        }
    }

    // Null terminate
    let total_len = prefix_len + comm_len;
    if total_len < MAX_PATH_LEN {
        path_buf[total_len] = 0;
    }

    Ok(total_len as u32)
}

/// Check if path matches a prefix pattern
///
/// Used for wildcard policies like "/api/*"
/// Returns true if path starts with prefix
#[inline(always)]
#[allow(dead_code)]  // Will be used when prefix matching is implemented
fn matches_prefix(path: &[u8; MAX_PATH_LEN], prefix: &[u8; MAX_PATH_LEN]) -> bool {
    for i in 0..MAX_PATH_LEN {
        if prefix[i] == 0 {
            // End of prefix, it's a match
            return true;
        }
        if path[i] != prefix[i] {
            // Mismatch
            return false;
        }
        if path[i] == 0 {
            // End of path before end of prefix
            return false;
        }
    }
    true
}

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
        timestamp_ns: unsafe { aya_ebpf::helpers::bpf_ktime_get_ns() },
    };

    // Submit to ring buffer
    let _ = EVENTS.output(&event, 0);

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
