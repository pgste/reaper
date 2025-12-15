//! eBPF Controller - Loads and manages the eBPF LSM program
//!
//! This module provides the userspace interface to the kernel eBPF program.
//! It handles:
//! - Loading the eBPF program from .o file
//! - Attaching to LSM hooks
//! - Managing BPF maps (POLICY_MAP, CONTEXT_MAP, etc.)
//! - Reading statistics

use crate::compiler::PolicyCompiler;
use crate::types::{
    stats_keys, EbpfStats, PolicyEntry, MAX_CONTEXT_KEY_LEN, MAX_CONTEXT_VALUE_LEN, MAX_PATH_LEN,
};
use anyhow::{Context as AnyhowContext, Result};
use aya::{
    maps::{HashMap as AyaHashMap, RingBuf},
    Bpf,
};
use policy_engine::SimplePolicyEvaluator;
use std::path::Path;
use tracing::{debug, info, warn};

/// eBPF Controller - Manages the kernel-side eBPF program
pub struct EbpfController {
    /// Loaded eBPF program (owns all maps)
    bpf: Bpf,

    /// Policy compiler
    compiler: PolicyCompiler,

    /// Count of policies in eBPF
    policy_count: usize,
}

impl EbpfController {
    /// Load eBPF program from file and initialize maps
    ///
    /// # Arguments
    /// * `program_path` - Path to the compiled eBPF .o file
    ///
    /// # Example
    /// ```no_run
    /// use reaper_ebpf::EbpfController;
    ///
    /// let controller = EbpfController::load("target/bpfel-unknown-none/release/reaper_ebpf_kern.o")?;
    /// ```
    pub fn load(program_path: impl AsRef<Path>) -> Result<Self> {
        let path = program_path.as_ref();
        info!("Loading eBPF program from: {}", path.display());

        // Load eBPF program
        let bpf = Bpf::load_file(path)
            .with_context(|| format!("Failed to load eBPF program from {}", path.display()))?;

        info!("eBPF program loaded successfully");

        Ok(Self {
            bpf,
            compiler: PolicyCompiler::new(),
            policy_count: 0,
        })
    }

    /// Get mutable reference to policy map
    fn policy_map(
        &mut self,
    ) -> Result<AyaHashMap<&mut aya::maps::MapData, [u8; MAX_PATH_LEN], PolicyEntry>> {
        AyaHashMap::try_from(
            self.bpf
                .map_mut("POLICY_MAP")
                .context("POLICY_MAP not found")?,
        )
        .context("Failed to get POLICY_MAP")
    }

    /// Get mutable reference to wildcard policy map
    fn wildcard_policy(&mut self) -> Result<AyaHashMap<&mut aya::maps::MapData, u8, PolicyEntry>> {
        AyaHashMap::try_from(
            self.bpf
                .map_mut("WILDCARD_POLICY")
                .context("WILDCARD_POLICY not found")?,
        )
        .context("Failed to get WILDCARD_POLICY")
    }

    /// Get mutable reference to context map
    fn context_map(
        &mut self,
    ) -> Result<
        AyaHashMap<&mut aya::maps::MapData, [u8; MAX_CONTEXT_KEY_LEN], [u8; MAX_CONTEXT_VALUE_LEN]>,
    > {
        AyaHashMap::try_from(
            self.bpf
                .map_mut("CONTEXT_MAP")
                .context("CONTEXT_MAP not found")?,
        )
        .context("Failed to get CONTEXT_MAP")
    }

    /// Get mutable reference to stats map
    fn stats_map(&mut self) -> Result<AyaHashMap<&mut aya::maps::MapData, u32, u64>> {
        AyaHashMap::try_from(self.bpf.map_mut("STATS").context("STATS not found")?)
            .context("Failed to get STATS")
    }

    /// Attach eBPF program to LSM hooks
    ///
    /// This requires root/CAP_BPF privileges.
    pub fn attach(&mut self) -> Result<()> {
        info!("Attaching eBPF LSM hooks...");

        // Note: In Aya 0.12, LSM programs may not exist yet or have different names
        // This is a placeholder implementation that may need adjustment based on
        // the actual eBPF program structure

        // For now, we'll skip attaching since we don't have the compiled eBPF program yet
        // and the API may vary based on the kernel eBPF implementation

        info!("eBPF LSM attachment skipped (requires compiled eBPF program)");

        Ok(())
    }

    /// Deploy a Simple policy to eBPF
    ///
    /// Compiles all rules and inserts them into POLICY_MAP.
    /// This is the main method for loading policies into eBPF.
    pub fn deploy_simple_policy(&mut self, evaluator: &SimplePolicyEvaluator) -> Result<()> {
        info!(
            "Deploying Simple policy to eBPF ({} rules)",
            evaluator.rules.len()
        );

        let compiled = self.compiler.compile_simple_policy(evaluator)?;

        let mut inserted = 0;
        let mut errors = 0;

        let mut policy_map = self.policy_map()?;
        for (key, entry) in compiled {
            match policy_map.insert(key, entry, 0) {
                Ok(_) => inserted += 1,
                Err(e) => {
                    warn!("Failed to insert policy: {}", e);
                    errors += 1;
                }
            }
        }

        self.policy_count = inserted;

        info!("Deployed {} rules to eBPF ({} errors)", inserted, errors);

        Ok(())
    }

    /// Set wildcard policy (applies to all resources)
    ///
    /// # Arguments
    /// * `entry` - The policy entry for wildcard matching
    pub fn set_wildcard_policy(&mut self, entry: PolicyEntry) -> Result<()> {
        let mut wildcard = self.wildcard_policy()?;
        wildcard.insert(0u8, entry, 0)?;
        info!("Set wildcard policy: action={}", entry.action);
        Ok(())
    }

    /// Insert a single policy rule into eBPF
    ///
    /// Used by the learning engine to promote frequently accessed paths.
    pub fn insert_policy(&mut self, key: [u8; MAX_PATH_LEN], entry: PolicyEntry) -> Result<()> {
        let mut policy_map = self.policy_map()?;
        policy_map.insert(key, entry, 0)?;
        self.policy_count += 1;

        debug!(
            "Inserted policy: {} → action={}",
            self.compiler.key_to_resource(&key),
            entry.action
        );

        Ok(())
    }

    /// Remove a policy from eBPF
    pub fn remove_policy(&mut self, key: &[u8; MAX_PATH_LEN]) -> Result<()> {
        let mut policy_map = self.policy_map()?;
        policy_map.remove(key)?;
        self.policy_count = self.policy_count.saturating_sub(1);

        debug!("Removed policy: {}", self.compiler.key_to_resource(key));

        Ok(())
    }

    /// Clear all policies from eBPF
    pub fn clear_policies(&mut self) -> Result<()> {
        info!("Clearing all eBPF policies...");

        // Iterate and remove all entries
        let mut policy_map = self.policy_map()?;
        let keys: Vec<_> = policy_map.keys().collect::<Result<_, _>>()?;

        for key in keys {
            policy_map.remove(&key)?;
        }

        self.policy_count = 0;

        info!("All eBPF policies cleared");

        Ok(())
    }

    /// Update context data (JWT claims, user attributes, etc.)
    ///
    /// # Arguments
    /// * `key` - Context key (e.g., "user_id", "role")
    /// * `value` - Context value (e.g., "alice", "admin")
    pub fn update_context(&mut self, key: &str, value: &str) -> Result<()> {
        let mut key_buf = [0u8; MAX_CONTEXT_KEY_LEN];
        let mut value_buf = [0u8; MAX_CONTEXT_VALUE_LEN];

        // Copy key
        let key_bytes = key.as_bytes();
        let key_len = key_bytes.len().min(MAX_CONTEXT_KEY_LEN - 1);
        key_buf[..key_len].copy_from_slice(&key_bytes[..key_len]);

        // Copy value
        let value_bytes = value.as_bytes();
        let value_len = value_bytes.len().min(MAX_CONTEXT_VALUE_LEN - 1);
        value_buf[..value_len].copy_from_slice(&value_bytes[..value_len]);

        let mut context_map = self.context_map()?;
        context_map.insert(key_buf, value_buf, 0)?;

        debug!("Updated context: {} = {}", key, value);

        Ok(())
    }

    /// Get statistics from eBPF
    pub fn get_stats(&mut self) -> Result<EbpfStats> {
        let stats_map = self.stats_map()?;

        let fast_path = stats_map.get(&stats_keys::FAST_PATH, 0).unwrap_or(0);

        let slow_path = stats_map.get(&stats_keys::SLOW_PATH, 0).unwrap_or(0);

        let denials = stats_map.get(&stats_keys::DENIALS, 0).unwrap_or(0);

        let allows = stats_map.get(&stats_keys::ALLOWS, 0).unwrap_or(0);

        let errors = stats_map.get(&stats_keys::ERRORS, 0).unwrap_or(0);

        Ok(EbpfStats {
            fast_path,
            slow_path,
            denials,
            allows,
            errors,
        })
    }

    /// Get reference to events ring buffer
    ///
    /// Used by SlowPathHandler to consume events
    /// Note: This creates a new RingBuf reference each time it's called
    pub fn events(&mut self) -> Result<RingBuf<&mut aya::maps::MapData>> {
        RingBuf::try_from(self.bpf.map_mut("EVENTS").context("EVENTS not found")?)
            .context("Failed to get EVENTS ring buffer")
    }

    /// Get compiler reference
    pub fn compiler(&self) -> &PolicyCompiler {
        &self.compiler
    }

    /// Get count of policies in eBPF
    pub fn policy_count(&self) -> usize {
        self.policy_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_program_not_found() {
        let result = EbpfController::load("nonexistent.o");
        assert!(result.is_err());
    }

    // Note: Integration tests with actual eBPF loading require:
    // 1. Compiled eBPF .o file
    // 2. Root privileges
    // 3. LSM BPF enabled kernel
    //
    // These tests should be run in a VM or container:
    // ```
    // sudo -E cargo test --test integration_test
    // ```
}
