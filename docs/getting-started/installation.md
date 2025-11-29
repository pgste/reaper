# Installation

Get Reaper up and running on your system.

## Prerequisites

- **Rust** 1.70+ (2021 edition)
- **Cargo** (comes with Rust)
- **Operating System**: Linux, macOS, or Windows

## Quick Install

### From Source

```bash
# Clone the repository
git clone https://github.com/your-org/reaper.git
cd reaper

# Build the workspace
cargo build --release

# Install CLI (optional)
cargo install --path tools/reaper-cli
```

### Using Cargo

```bash
# Install from crates.io (coming soon)
cargo install reaper-cli
```

### Using Docker

```bash
# Pull the image
docker pull reaper/agent:latest
docker pull reaper/platform:latest

# Run the platform
docker run -p 8081:8081 reaper/platform:latest

# Run the agent
docker run -p 8080:8080 reaper/agent:latest
```

## Development Setup

For developing with Reaper:

```bash
# Clone the repository
git clone https://github.com/your-org/reaper.git
cd reaper

# Run one-time setup
make setup

# Build all workspace members
make build

# Run tests to verify installation
make test
```

## Verify Installation

### Check CLI

```bash
reaper-cli --version
# Output: reaper-cli 0.1.0
```

### Run Health Check

```bash
# Start the agent
cargo run --bin reaper-agent

# In another terminal, check health
curl http://localhost:8080/health
# Output: {"status":"healthy"}
```

## Directory Structure

After installation, your directory should look like:

```
reaper/
├── crates/
│   ├── reaper-core/       # Core types and traits
│   ├── policy-engine/     # Policy evaluation engine
│   ├── message-queue/     # Async messaging
│   └── metrics/           # Performance monitoring
├── services/
│   ├── reaper-agent/      # Agent service
│   └── reaper-platform/   # Platform service
├── tools/
│   └── reaper-cli/        # CLI tool
└── docs/                  # Documentation
```

## Configuration

### Environment Variables

```bash
# Agent configuration
export REAPER_AGENT_PORT=8080
export REAPER_AGENT_LOG_LEVEL=info

# Platform configuration
export REAPER_PLATFORM_PORT=8081
export REAPER_PLATFORM_LOG_LEVEL=info
```

### Configuration File

Create `config.toml`:

```toml
[agent]
port = 8080
log_level = "info"
max_policies = 1000

[platform]
port = 8081
log_level = "info"
db_path = "./reaper.db"
```

## Running Services

### Development Mode

```bash
# Run both services with auto-reload
make dev-services

# Or run individually
make agent      # Agent on port 8080
make platform   # Platform on port 8081
```

### Production Mode

```bash
# Build optimized binaries
cargo build --release

# Run agent
./target/release/reaper-agent

# Run platform
./target/release/reaper-platform
```

## Platform-Specific Notes

### Linux

No additional setup required. Ensure you have build essentials:

```bash
# Ubuntu/Debian
sudo apt-get install build-essential

# Fedora/RHEL
sudo dnf install gcc
```

### macOS

Install Xcode Command Line Tools:

```bash
xcode-select --install
```

### Windows

Install Visual Studio Build Tools:

1. Download [Visual Studio Build Tools](https://visualstudio.microsoft.com/downloads/)
2. Install "Desktop development with C++"
3. Run commands in PowerShell or CMD

## Troubleshooting

### Port Already in Use

```bash
# Check what's using port 8080
lsof -i :8080

# Kill the process
kill -9 <PID>
```

### Compilation Errors

```bash
# Update Rust toolchain
rustup update stable

# Clean and rebuild
cargo clean
cargo build --release
```

### Missing Dependencies

```bash
# Update dependencies
cargo update

# Check for outdated dependencies
cargo outdated
```

## Next Steps

- **[Quick Start](./quick-start.md)** - Run your first policy evaluation
- **[First Policy](./first-policy.md)** - Write your first policy
- **[Examples](./examples.md)** - Explore example policies

## Development Tools

### Useful Make Commands

```bash
make setup      # One-time setup
make build      # Build workspace
make test       # Run all tests
make check      # Format, lint, test
make dev        # Auto-reload on changes
make bench      # Run benchmarks
make coverage   # Generate coverage report
```

### IDE Setup

#### VS Code

Install recommended extensions:

```bash
# Rust Analyzer
code --install-extension rust-lang.rust-analyzer

# CodeLLDB (debugging)
code --install-extension vadimcn.vscode-lldb
```

#### IntelliJ IDEA

1. Install Rust plugin
2. Import Cargo project
3. Enable "Use native toolchain"

## Getting Help

- **Documentation**: [docs/](../index.md)
- **GitHub Issues**: [Report a bug](https://github.com/your-org/reaper/issues)
- **Discussions**: [Ask questions](https://github.com/your-org/reaper/discussions)
