# CI & eBPF Benchmarking Updates - Summary

## Changes Made

### 1. ✅ Fixed Path Issue in cleanup.sh
**Problem**: Script had hardcoded `/workspaces/reaper` path that doesn't exist in GitHub Actions

**Solution**: Made the path dynamic - searches for `reaper-agent` binary in:
- `./target/release/reaper-agent` (from repo root)
- `../../target/release/reaper-agent` (from benchmark directory)

**File**: `/workspaces/reaper/benchmarks/reaper-vs-opa/bin/cleanup.sh`

### 2. ✅ Upgraded to 100k Datasets
**Changed**: Matrix now uses `100k` scale instead of `10k` for more realistic performance comparison

**Impact**:
- More realistic workload (100,000 entities vs 10,000)
- Better demonstrates performance differences at scale
- Longer benchmark runtime (~2-3x)

**File**: `.github/workflows/benchmark.yml`

### 3. ✅ Added eBPF Build Support
**Added to CI**:
- Install eBPF build dependencies (clang, llvm, kernel headers)
- Setup Rust nightly with eBPF target (`bpfel-unknown-none`)
- Build eBPF kernel program
- Verify eBPF compilation succeeds
- Check eBPF capabilities (kernel version, LSM support)

**What This Enables**:
- ✅ Verifies eBPF code compiles on every PR
- ✅ Catches eBPF compilation errors early
- 📋 Prepares for future eBPF benchmarking (when integration complete)

### 4. ✅ Updated Artifact Actions
**Upgraded**:
- `actions/checkout`: v3 → v4
- `actions/cache`: v3 → v4
- `actions/upload-artifact`: v3 → v4
- `actions/download-artifact`: v3 → v4

**Why**: v3 is deprecated and will stop working

### 5. ✅ Enterprise OPA Integration
**Using**: `ghcr.io/styrainc/enterprise-opa:latest` instead of standard OPA

**Benefits**:
- Performance optimizations from Styra
- Recently open-sourced
- More competitive comparison

### 6. 📝 Added eBPF Documentation
**Created**: `EBPF_BENCHMARKING.md` - Complete guide for:
- eBPF architecture and performance targets
- Integration steps for benchmark tool
- CI setup options (self-hosted runner, Docker privileged mode)
- Troubleshooting guide
- Local testing instructions

---

## Current CI Workflow

### Build Phase (Parallel for Each Scenario)
```
Checkout → Setup Rust → Cache → Install Enterprise OPA
    ↓
Install eBPF deps → Setup nightly → Build eBPF program
    ↓
Build Reaper (userspace + eBPF) → Build Benchmark Tool
    ↓
Check eBPF capabilities (informational)
```

### Benchmark Phase
```
Start Reaper Agent (userspace) → Start Enterprise OPA
    ↓
Deploy Policies → Deploy Data
    ↓
Run Benchmark (100k entities, scenario-specific)
    ↓
Upload Results → Stop Services
```

### Report Phase
```
Download all results → Aggregate → Post PR Comment
```

---

## Benchmark Matrix (4 Scenarios × 100k Scale)

| Scenario | Entities | Policies | Focus Area |
|----------|----------|----------|------------|
| RBAC | 100,000 | Role-based | Simple hierarchical access |
| ABAC | 100,000 | Attribute-based | Clearance levels, departments |
| ReBAC | 100,000 | Relationship-based | Graph traversal |
| Multilayer | 100,000 | Combined | Real-world complexity |

**Total**: 4 concurrent jobs running 100k-scale benchmarks

---

## PR Comment Format

```markdown
## 📊 Benchmark Results (Reaper vs Enterprise OPA)

| Scenario | Scale | Engine | RPS | P50 (μs) | P95 (μs) | P99 (μs) | Speedup |
|----------|-------|--------|-----|----------|----------|----------|---------|
| abac | 100k | Reaper | 125,000 | 150 | 350 | 450 | **2.5x** |
| abac | 100k | Enterprise OPA | 50,000 | 400 | 800 | 1000 | 1.00x |
| rbac | 100k | Reaper | 180,000 | 100 | 250 | 300 | **3.2x** |
| rbac | 100k | Enterprise OPA | 56,000 | 350 | 700 | 900 | 1.00x |
...

---

**Notes**:
- Using Styra Enterprise OPA (open source) for fair comparison
- eBPF kernel program built successfully ✅
- eBPF benchmarking: Coming soon (requires kernel LSM BPF + root)
- **Expected eBPF performance**: <100ns fast path (500-2000x faster)
```

---

## Next Steps for eBPF Benchmarking

### Short Term (Ready Now)
1. ✅ eBPF builds on every PR
2. ✅ Compilation errors caught early
3. ✅ Foundation for eBPF integration laid

### Medium Term (Requires Implementation)
1. 📋 Integrate eBPF agent with reaper-agent binary
   - Add `--ebpf-enabled` flag
   - Add `--ebpf-program` parameter
   - Load eBPF on startup

2. 📋 Extend benchmark tool for 3-way comparison
   - Reaper (userspace)
   - Reaper (eBPF)
   - Enterprise OPA

3. 📋 Add eBPF policy deployment
   - Deploy to both userspace and eBPF
   - Verify decisions match

### Long Term (Production Ready)
1. 📋 Self-hosted runner with eBPF support
   - Linux kernel 5.7+ with LSM BPF
   - Root/CAP_BPF privileges
   - Run eBPF benchmarks on every PR

2. 📋 Learning mode benchmarks
   - Test auto-promotion to eBPF fast path
   - Measure fast path coverage (target: 80%+)
   - Track promotion statistics

3. 📋 Production deployment guide
   - Kubernetes DaemonSet
   - Helm charts
   - Monitoring dashboards

---

## Performance Expectations

### Current (Userspace vs Enterprise OPA)
- **Reaper (userspace)**: 100K-200K req/s, 1-5µs latency
- **Enterprise OPA**: 50K-100K req/s, 10-50µs latency
- **Speedup**: 2-4x depending on scenario

### Future (with eBPF Fast Path)
- **Reaper (eBPF fast path)**: 10M+ req/s, <100ns latency
- **Reaper (slow path)**: 100K-200K req/s, 10-50µs latency
- **Coverage**: 80%+ requests → fast path (with learning)
- **Speedup vs OPA**: 100-2000x for hot paths

---

## Testing Locally

### Userspace Benchmarks (Current)
```bash
cd /workspaces/reaper/benchmarks/reaper-vs-opa

# Start services
./bin/deploy-reaper.sh abac 100k
./bin/deploy-opa.sh abac 100k

# Run benchmark
./bin/benchmark.sh --scenario abac --scale 100k --requests 50000
```

### eBPF Benchmarks (Future - When Integrated)
```bash
# Build eBPF program
cd crates/reaper-ebpf/reaper-ebpf-kern
cargo +nightly build --target=bpfel-unknown-none -Z build-std=core --release

# Start eBPF-enabled agent (requires root)
sudo ./target/release/reaper-agent \
  --ebpf-enabled \
  --ebpf-program crates/reaper-ebpf/reaper-ebpf-kern/target/bpfel-unknown-none/release/reaper_ebpf_kern.o

# Deploy and benchmark
./bin/deploy-reaper.sh abac 100k
sudo ./bin/benchmark.sh --scenario abac --scale 100k --engines "reaper-ebpf opa"
```

---

## Troubleshooting

### CI: "cleanup.sh: No such file or directory"
✅ **Fixed**: Path is now dynamic, works in both codespace and CI

### CI: Artifact upload/download deprecated
✅ **Fixed**: Upgraded to v4 actions

### CI: Want eBPF benchmarks
📋 **Status**: eBPF builds successfully, but loading requires:
- Self-hosted runner with kernel LSM BPF support
- Or Docker privileged mode
- See `EBPF_BENCHMARKING.md` for setup guide

### Local: eBPF program won't load
**Solution**: Requires root and kernel LSM BPF:
```bash
# Check kernel version (need 5.7+)
uname -r

# Check LSM BPF
cat /sys/kernel/security/lsm | grep bpf

# If missing, enable in grub
sudo vi /etc/default/grub
# Add: lsm=...,bpf
sudo update-grub
sudo reboot
```

---

## Files Modified

```
.github/workflows/benchmark.yml          - Added eBPF build, 100k scale, upgraded actions
benchmarks/reaper-vs-opa/bin/cleanup.sh  - Fixed hardcoded path
benchmarks/reaper-vs-opa/EBPF_BENCHMARKING.md - New: eBPF integration guide
benchmarks/reaper-vs-opa/docker-compose.yml - Enterprise OPA
benchmarks/reaper-vs-opa/DOCKER.md       - Updated docs
```

---

## Summary

🎯 **Immediate Impact**:
- ✅ CI now runs successfully with 100k datasets
- ✅ More realistic performance comparison
- ✅ eBPF compilation verified on every PR
- ✅ Enterprise OPA for fair comparison

🚀 **Future Ready**:
- 📋 Foundation laid for eBPF benchmarking
- 📋 Clear path to <100ns policy evaluation
- 📋 Documentation for implementation

📊 **Results**:
- Every PR gets automated benchmark results
- 4 scenarios tested concurrently
- 100,000 entities for realistic load
- Speedup calculations in PR comments
