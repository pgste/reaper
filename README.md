# Reaper - High-Performance Policy Enforcement Platform

**Reaper Agent** provides sub-microsecond policy enforcement for enterprise sidecars, while **Reaper Platform** manages distributed agents with zero-downtime deployments.

## 🎯 Core Value Proposition

- **60-80% Memory Reduction** vs traditional JVM-based policy engines
- **Sub-microsecond Decision Latency** for cost-effective sidecar deployment
- **Zero-downtime Policy Updates** using atomic swapping
- **Enterprise-grade Reliability** with comprehensive BDD testing

## 🚀 Quick Start

```bash
# Setup development environment
make setup

# Start development mode
make dev

# Run Reaper services
make dev-services

# Run tests
make test

# Build CLI tool
make cli
```

## 🏗️ Architecture

### Core Products

- **Reaper Agent** - High-performance policy enforcement service
- **Reaper Platform** - Distributed agent management and monitoring
- **Reaper CLI** - Command-line interface for policy and agent management

### Components

- **Policy Engine** - Sub-microsecond decision evaluation with hot-swapping
- **Message Queue** - Reliable async communication between components
- **Metrics** - Performance monitoring and compliance reporting

## 📊 API Endpoints

### Reaper Agent (Port 8080)
- `GET /health` - Health check
- `GET /metrics` - Performance metrics
- `POST /api/v1/messages` - Policy evaluation

### Reaper Platform (Port 8081)
- `GET /health` - Health check
- `GET /metrics` - Platform metrics
- `GET /api/v1/policies` - List policies
- `POST /api/v1/policies` - Create policy
- `PUT /api/v1/policies/:id` - Update policy
- `GET /api/v1/agents` - List agents

## 🧪 Testing

### Unit Tests
```bash
cargo test --workspace --lib
```

### BDD Scenarios
```bash
make bdd
```

### Performance Benchmarks
```bash
make bench
```

## 🚢 Release Process

```bash
# Patch release
make release

# Minor release  
make release VERSION=minor

# Major release
make release VERSION=major
```

## 🔧 Development

### Project Structure

```
reaper/
├── crates/
│   ├── reaper-core/     # Core types and traits
│   ├── policy-engine/   # Policy evaluation engine
│   ├── message-queue/   # Async messaging
│   └── metrics/         # Monitoring and metrics
├── services/
│   ├── reaper-agent/    # Policy enforcement service
│   └── reaper-platform/ # Agent management service
├── tools/
│   └── reaper-cli/      # Command-line interface
└── tests/
    ├── integration/     # Integration tests
    └── performance/     # Performance tests
```

### Key Commands

- `make dev` - Development mode with auto-reload
- `make agent` - Run Reaper Agent locally
- `make platform` - Run Reaper Platform locally
- `make cli` - Build CLI tool
- `make check` - Code quality checks
- `make coverage` - Test coverage report

## 📈 Performance Goals

- **Policy Evaluation**: < 1 microsecond p99 latency
- **Memory Usage**: < 50MB per agent instance
- **Throughput**: > 100K decisions/second per agent
- **Startup Time**: < 100ms cold start

## 🎖️ Enterprise Features

- Zero-downtime policy deployments
- Centralized agent management
- Real-time compliance monitoring
- Automated rollback capabilities
- Audit logging and reporting

---

Built with ❤️ using Rust for maximum performance and reliability.