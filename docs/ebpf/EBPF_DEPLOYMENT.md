# eBPF Deployment Guide

This guide covers deploying the Reaper eBPF policy engine in production environments.

## Prerequisites

### System Requirements

- **Linux Kernel**: 5.7+ with LSM BPF support (`CONFIG_BPF_LSM=y`)
- **Architecture**: x86_64 (eBPF program compiled for `bpfel-unknown-none`)
- **Privileges**: Root or `CAP_BPF` + `CAP_NET_ADMIN` + `CAP_PERFMON`
- **Memory**: Minimum 512MB available (< 50MB typical usage)

### Verify Kernel Support

```bash
# Check kernel version
uname -r  # Should be 5.7 or higher

# Verify LSM BPF is enabled
cat /sys/kernel/security/lsm | grep -o bpf

# Check available LSM hooks
ls /sys/kernel/security/bpf/
```

### Install Dependencies

```bash
# Ubuntu/Debian
sudo apt-get update
sudo apt-get install -y \
    linux-headers-$(uname -r) \
    clang \
    llvm \
    libelf-dev

# RHEL/CentOS
sudo yum install -y \
    kernel-devel-$(uname -r) \
    clang \
    llvm \
    elfutils-libelf-devel
```

## Deployment Options

### Option 1: Standalone Agent (Recommended)

Deploy Reaper Agent as a standalone service with eBPF enforcement.

```bash
# 1. Build the eBPF program (on x86_64 with bpf-linker)
cd crates/reaper-ebpf/reaper-ebpf-kern
cargo +nightly build --target=bpfel-unknown-none \
    -Z build-std=core --release

# 2. Find the compiled eBPF object
EBPF_OBJ=$(find target/bpfel-unknown-none/release/deps \
    -name "*.rcgu.o" -name "*reaper_ebpf_kern*" | head -1)

echo "eBPF object: $EBPF_OBJ"

# 3. Copy to deployment location
sudo mkdir -p /opt/reaper/ebpf
sudo cp $EBPF_OBJ /opt/reaper/ebpf/reaper_ebpf_kern.o
sudo chmod 644 /opt/reaper/ebpf/reaper_ebpf_kern.o

# 4. Build and deploy the agent
cd ../../../services/reaper-agent
cargo build --release
sudo cp target/release/reaper-agent /usr/local/bin/
sudo chmod 755 /usr/local/bin/reaper-agent

# 5. Create configuration
sudo mkdir -p /etc/reaper
sudo tee /etc/reaper/agent.toml <<EOF
[ebpf]
enabled = true
program_path = "/opt/reaper/ebpf/reaper_ebpf_kern.o"
auto_promote = true
promotion_threshold = 100
stability_window = 100

[server]
host = "0.0.0.0"
port = 8080

[logging]
level = "info"
format = "json"
EOF

# 6. Create systemd service
sudo tee /etc/systemd/system/reaper-agent.service <<EOF
[Unit]
Description=Reaper Policy Agent with eBPF
After=network.target
Documentation=https://github.com/pgste/reaper

[Service]
Type=simple
ExecStart=/usr/local/bin/reaper-agent
Restart=always
RestartSec=10
User=root
Group=root

# Security hardening
ProtectSystem=strict
ProtectHome=yes
ReadWritePaths=/var/log/reaper
NoNewPrivileges=false  # Required for BPF operations
AmbientCapabilities=CAP_BPF CAP_NET_ADMIN CAP_PERFMON

# Resource limits
MemoryLimit=512M
CPUQuota=50%

[Install]
WantedBy=multi-user.target
EOF

# 7. Start the service
sudo systemctl daemon-reload
sudo systemctl enable reaper-agent
sudo systemctl start reaper-agent
sudo systemctl status reaper-agent
```

### Option 2: Platform + Agent

Deploy both Platform (management) and Agent (enforcement).

```bash
# Deploy Platform first (port 8081)
cd services/reaper-platform
cargo build --release
sudo cp target/release/reaper-platform /usr/local/bin/
sudo systemctl enable reaper-platform
sudo systemctl start reaper-platform

# Deploy Agent with Platform integration
# (Follow Option 1, but add platform URL to config)
sudo tee -a /etc/reaper/agent.toml <<EOF

[platform]
url = "http://localhost:8081"
sync_interval_secs = 60
EOF

sudo systemctl restart reaper-agent
```

### Option 3: Docker Container

```dockerfile
# Dockerfile for Reaper Agent with eBPF
FROM rust:1.75-bullseye as builder

# Install bpf-linker
RUN cargo install bpf-linker

# Copy source
WORKDIR /build
COPY . .

# Build eBPF program
WORKDIR /build/crates/reaper-ebpf/reaper-ebpf-kern
RUN cargo +nightly build --target=bpfel-unknown-none \
    -Z build-std=core --release

# Build agent
WORKDIR /build/services/reaper-agent
RUN cargo build --release

# Runtime image
FROM debian:bullseye-slim
RUN apt-get update && apt-get install -y \
    libelf1 \
    && rm -rf /var/lib/apt/lists/*

# Copy binaries
COPY --from=builder \
    /build/services/reaper-agent/target/release/reaper-agent \
    /usr/local/bin/
COPY --from=builder \
    /build/crates/reaper-ebpf/reaper-ebpf-kern/target/bpfel-unknown-none/release/deps/*.rcgu.o \
    /opt/reaper/ebpf/reaper_ebpf_kern.o

# Run as root (required for eBPF)
EXPOSE 8080
CMD ["/usr/local/bin/reaper-agent"]
```

```bash
# Build and run
docker build -t reaper-agent .
docker run --privileged \
    -v /sys/kernel/security:/sys/kernel/security:ro \
    -p 8080:8080 \
    reaper-agent
```

## Policy Deployment

### Deploy a Policy Bundle

```bash
# Create a policy bundle
cat > /tmp/policy.reap <<EOF
policy example_policy {
    default: deny,
    rule allow_all {
        allow if true
    }
}
EOF

# Compile to bundle
./target/debug/reaper-cli compile /tmp/policy.reap --output /tmp/policy.rbb

# Deploy via Platform API
curl -X POST http://localhost:8081/api/v1/policies \
    -H "Content-Type: application/octet-stream" \
    --data-binary @/tmp/policy.rbb

# Or deploy directly to Agent
curl -X POST http://localhost:8080/api/v1/policies/deploy \
    -H "Content-Type: application/octet-stream" \
    --data-binary @/tmp/policy.rbb
```

### Verify Deployment

```bash
# Check agent status
curl http://localhost:8080/health

# View statistics
curl http://localhost:8080/metrics | jq

# Expected output:
# {
#   "fast_path_evaluations": 800,
#   "slow_path_evaluations": 200,
#   "fast_path_percent": 80.0,
#   "denials": 50,
#   "allows": 950,
#   "ebpf_policy_count": 5,
#   "promoted_policies": 3
# }
```

## Monitoring and Observability

### Metrics

The agent exposes Prometheus-compatible metrics:

```bash
# Scrape metrics
curl http://localhost:8080/metrics

# Key metrics:
# - fast_path_evaluations: eBPF decisions
# - slow_path_evaluations: Userspace decisions
# - fast_path_percent: % handled in eBPF
# - promoted_policies: Auto-promoted hot paths
```

### Logs

```bash
# View agent logs
sudo journalctl -u reaper-agent -f

# Filter for eBPF events
sudo journalctl -u reaper-agent | grep "eBPF:"

# View slow path evaluations
sudo journalctl -u reaper-agent | grep "Slow path"
```

### BPF Tools

```bash
# List loaded BPF programs
sudo bpftool prog list | grep reaper

# Show BPF map statistics
sudo bpftool map list

# Dump policy map
sudo bpftool map dump name POLICY_MAP

# Monitor events
sudo bpftool prog tracelog
```

## Performance Tuning

### Auto-Promotion Thresholds

Adjust learning engine settings in `/etc/reaper/agent.toml`:

```toml
[ebpf]
# Higher threshold = more confidence before promotion
promotion_threshold = 100  # Default: 100 accesses

# Wider window = more stability required
stability_window = 100     # Default: 100 accesses

# Enable/disable auto-promotion
auto_promote = true
```

### Resource Limits

eBPF map sizes can be tuned in the kernel program:

```rust
// crates/reaper-ebpf/reaper-ebpf-kern/src/lib.rs

// Increase policy map size for more policies
#[map]
static POLICY_MAP: HashMap<[u8; MAX_PATH_LEN], PolicyEntry> =
    HashMap::with_max_entries(10000, 0);  // Default: 1024

// Increase ring buffer for higher event throughput
#[map]
static EVENTS: RingBuf = RingBuf::with_byte_size(256 * 1024, 0);  // Default: 256KB
```

### CPU Pinning

For production workloads, pin the agent to specific CPUs:

```ini
[Service]
CPUAffinity=0-3  # Use CPUs 0-3
```

## Troubleshooting

### eBPF Program Fails to Load

```bash
# Check kernel support
cat /boot/config-$(uname -r) | grep CONFIG_BPF_LSM
# Should show: CONFIG_BPF_LSM=y

# Verify LSM is enabled
cat /proc/cmdline | grep lsm
# Should include "bpf" in LSM list

# If not, add to kernel boot parameters:
sudo vim /etc/default/grub
# Add: GRUB_CMDLINE_LINUX="lsm=lockdown,yama,apparmor,bpf"
sudo update-grub
sudo reboot
```

### Permission Denied

```bash
# Grant BPF capabilities
sudo setcap cap_bpf,cap_net_admin,cap_perfmon+eip /usr/local/bin/reaper-agent

# Or run as root
sudo /usr/local/bin/reaper-agent
```

### High Slow Path Percentage

If `fast_path_percent < 50%`:

1. **Lower promotion thresholds** - Allow faster promotion
2. **Check policy complexity** - Complex conditions can't be promoted
3. **Analyze access patterns** - Use `/api/v1/policies` to see what's promoted

```bash
# Get promoted policies
curl http://localhost:8080/api/v1/policies | jq '.policies[] | select(.in_ebpf == true)'
```

### Memory Issues

```bash
# Check agent memory usage
sudo systemctl status reaper-agent

# View detailed memory stats
sudo cat /proc/$(pidof reaper-agent)/status | grep VmRSS

# If >100MB, check for memory leaks:
sudo valgrind --leak-check=full /usr/local/bin/reaper-agent
```

## Security Considerations

### Least Privilege

Run with minimal capabilities:

```bash
# Drop unnecessary capabilities
sudo setcap cap_bpf,cap_net_admin,cap_perfmon+eip /usr/local/bin/reaper-agent

# Run as dedicated user (if not using systemd socket activation)
sudo useradd -r -s /bin/false reaper
sudo chown reaper:reaper /usr/local/bin/reaper-agent
```

### Policy Validation

Always validate policies before deployment:

```bash
# Validate policy syntax
./target/debug/reaper-cli validate /tmp/policy.reap

# Test in dry-run mode
./target/debug/reaper-cli eval --policy /tmp/policy.reap \
    --principal user:alice \
    --action read \
    --resource /api/users
```

### Audit Logging

Enable comprehensive audit logs:

```toml
[logging]
level = "info"
audit_file = "/var/log/reaper/audit.log"
audit_all_decisions = true  # Log all policy decisions
```

## Production Checklist

- [ ] Kernel 5.7+ with LSM BPF enabled
- [ ] eBPF program compiled and deployed
- [ ] Agent running as systemd service
- [ ] Auto-promotion configured appropriately
- [ ] Metrics and logs configured
- [ ] Resource limits set
- [ ] Security hardening applied
- [ ] Policies validated and tested
- [ ] Monitoring dashboards created
- [ ] Runbook documented

## Next Steps

- [Performance Measurement](./PERFORMANCE.md)
- [Policy Development Guide](../testing/POLICY_TESTS.md)
- [Troubleshooting Guide](./TROUBLESHOOTING.md)
