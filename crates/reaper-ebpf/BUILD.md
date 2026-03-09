# Building Reaper eBPF

This guide covers how to build the Reaper eBPF components.

## Prerequisites

### 1. Rust Nightly Toolchain

eBPF programs must be compiled with Rust nightly and the `bpfel-unknown-none` target:

```bash
# Install nightly toolchain
rustup toolchain install nightly

# Add rust-src component (required for building eBPF)
rustup component add rust-src --toolchain nightly

# Add eBPF target
rustup +nightly target add bpfel-unknown-none
```

### 2. System Requirements

- **Linux kernel 5.7+** with LSM BPF support
- **Root/CAP_BPF privileges** for loading and attaching eBPF programs
- **Development tools**: clang, llvm (for eBPF compilation)

Check kernel version:
```bash
uname -r  # Should be >= 5.7
```

Check LSM BPF support:
```bash
cat /sys/kernel/security/lsm | grep bpf
# If "bpf" is not listed, add it to kernel boot parameters: lsm=...,bpf
```

## Building

### 1. Build Userspace Components

Build the userspace Rust library that interfaces with eBPF:

```bash
cd /workspaces/reaper
cargo build -p reaper-ebpf
```

This builds:
- `PolicyCompiler` - Compiles Reaper policies to eBPF format
- `EbpfController` - Loads and manages eBPF programs
- `LearningEngine` - Auto-promotion intelligence
- `SlowPathHandler` - Complex policy evaluator
- `EbpfPolicyEngine` - Public API

### 2. Build Kernel eBPF Program

**Note**: The kernel eBPF program is not yet implemented. This section documents the planned build process.

```bash
cd /workspaces/reaper/crates/reaper-ebpf/reaper-ebpf-kern

# Build with nightly and eBPF target
cargo +nightly build \
    --target=bpfel-unknown-none \
    -Z build-std=core \
    --release

# Output: target/bpfel-unknown-none/release/reaper_ebpf_kern.o
```

Build options:
- `--target=bpfel-unknown-none`: eBPF target (little-endian)
- `-Z build-std=core`: Build Rust core library from source
- `--release`: Optimized build (important for eBPF verification)

### 3. Verify eBPF Program (Optional)

Once built, you can inspect the eBPF program:

```bash
# View eBPF object file sections
llvm-objdump -h target/bpfel-unknown-none/release/reaper_ebpf_kern.o

# View eBPF program bytecode
llvm-objdump -d target/bpfel-unknown-none/release/reaper_ebpf_kern.o

# Check BPF program with bpftool (requires root)
sudo bpftool prog load target/bpfel-unknown-none/release/reaper_ebpf_kern.o /sys/fs/bpf/reaper_test
```

## Development Build

For iterative development:

```bash
# Watch for changes and rebuild
cargo watch -x 'build -p reaper-ebpf'

# Run tests (no eBPF program required)
cargo test -p reaper-ebpf
```

## Integration with Reaper Agent

Once eBPF program is built, integrate with Reaper Agent:

```rust
use reaper_ebpf::EbpfPolicyEngine;
use policy_engine::PolicyEngine;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Create traditional policy engine
    let policy_engine = PolicyEngine::new();

    // Wrap with eBPF acceleration
    let mut ebpf_engine = EbpfPolicyEngine::load(
        policy_engine,
        "target/bpfel-unknown-none/release/reaper_ebpf_kern.o"
    )?;

    // Attach to LSM hooks (requires root)
    ebpf_engine.attach().await?;

    // Now policies can be deployed...
    Ok(())
}
```

## Troubleshooting

### Build Errors

#### `error: couldn't find the bpfel-unknown-none target`

Solution:
```bash
rustup +nightly target add bpfel-unknown-none
```

#### `error: can't find crate for 'std'`

Solution: Use `-Z build-std=core` flag

#### `error: failed to load BTF from /sys/kernel/btf/vmlinux`

This is normal if BTF (BPF Type Format) is not available on your kernel. The program will still compile.

### Runtime Errors

#### `Permission denied` when loading eBPF program

Solution: Run with root or add CAP_BPF capability:
```bash
sudo ./reaper-agent
# or
sudo setcap cap_bpf,cap_sys_admin+ep ./target/release/reaper-agent
```

#### `Operation not supported` when attaching LSM hook

Solution: Ensure LSM BPF is enabled in kernel boot parameters:
```bash
# Edit /etc/default/grub
GRUB_CMDLINE_LINUX="... lsm=lockdown,yama,apparmor,bpf"

# Update grub and reboot
sudo update-grub
sudo reboot
```

#### `Invalid argument` when loading program

Possible causes:
1. eBPF program exceeds complexity limits (1M instructions, 8K branches)
2. eBPF verifier rejected the program
3. Kernel version < 5.7

Solution: Check dmesg for verifier logs:
```bash
sudo dmesg | grep -i bpf | tail -50
```

## CI/CD Integration

For automated builds:

```yaml
# .github/workflows/ebpf.yml
name: eBPF Build

on: [push, pull_request]

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v2

      - name: Install Rust nightly
        run: |
          rustup toolchain install nightly
          rustup component add rust-src --toolchain nightly
          rustup +nightly target add bpfel-unknown-none

      - name: Build userspace
        run: cargo build -p reaper-ebpf

      - name: Build eBPF kernel program
        run: |
          cd crates/reaper-ebpf/reaper-ebpf-kern
          cargo +nightly build --target=bpfel-unknown-none -Z build-std=core --release

      - name: Run tests
        run: cargo test -p reaper-ebpf
```

## Docker Build

Build in a container with all dependencies:

```dockerfile
FROM rust:1.70-slim

# Install dependencies
RUN apt-get update && apt-get install -y \
    clang \
    llvm \
    linux-headers-generic \
    && rm -rf /var/lib/apt/lists/*

# Install Rust nightly + eBPF target
RUN rustup toolchain install nightly && \
    rustup component add rust-src --toolchain nightly && \
    rustup +nightly target add bpfel-unknown-none

WORKDIR /app
COPY . .

# Build
RUN cargo build -p reaper-ebpf --release
RUN cd crates/reaper-ebpf/reaper-ebpf-kern && \
    cargo +nightly build --target=bpfel-unknown-none -Z build-std=core --release

CMD ["./target/release/reaper-agent"]
```

## Next Steps

1. **Implement kernel eBPF program** in `reaper-ebpf-kern/src/lib.rs`
2. **Integration tests** with loaded eBPF program
3. **Performance benchmarks** to measure <100ns fast path
4. **Production deployment** via Kubernetes DaemonSet

## References

- [Aya Documentation](https://aya-rs.dev/)
- [eBPF LSM Docs](https://docs.kernel.org/bpf/prog_lsm.html)
- [Linux BPF Documentation](https://www.kernel.org/doc/html/latest/bpf/index.html)
- [bpftool Manual](https://man.archlinux.org/man/bpftool.8.en)
