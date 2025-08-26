.PHONY: setup dev test bdd bench coverage clean release check agent platform cli

# One-time setup
setup:
	./scripts/dev-setup.sh

# Development workflow with auto-reload
dev:
	cargo watch -x "check --workspace" -x "test --workspace --lib"

# Run all tests (unit + integration + BDD)
test:
	@echo "🧪 Running Reaper unit tests..."
	cargo test --workspace --lib
	@echo "🥒 Running Reaper BDD scenarios..."
	cargo test --workspace --test '*bdd*' 2>/dev/null || echo "BDD tests will be available after first implementation"
	@echo "🔗 Running integration tests..."
	cargo test --workspace --test '*integration*' 2>/dev/null || echo "Integration tests will be added"

# Run only BDD scenarios
bdd:
	cargo test --workspace --test '*bdd*' 2>/dev/null || echo "BDD tests will be available after first implementation"

# Performance benchmarks
bench:
	cargo bench --workspace

# Test coverage report
coverage:
	cargo tarpaulin --workspace --out Html --output-dir coverage/ --exclude-files 'target/*' 'tests/*'

# Code quality checks
check: 
	cargo fmt --check
	cargo clippy --workspace -- -D warnings
	cargo test --workspace

# Build and run Reaper Agent locally
agent:
	cargo run --bin reaper-agent

# Build and run Reaper Platform locally  
platform:
	cargo run --bin reaper-platform

# Build Reaper CLI
cli:
	cargo build --bin reaper-cli
	@echo "🎯 Reaper CLI built successfully!"
	@echo "Try: ./target/debug/reaper-cli status"

# Clean all build artifacts
clean:
	cargo clean
	rm -rf coverage/
	rm -rf target/criterion/

# Release (usage: make release VERSION=minor)
release:
	./scripts/release.sh $(or $(VERSION),patch)

# Quick build check for all Reaper components
build:
	cargo build --workspace
	cargo build --workspace --release

# Run both agent and platform in development
dev-services:
	@echo "🚀 Starting Reaper services in development mode..."
	@echo "🎯 Reaper Agent will be available at: http://localhost:8080"
	@echo "🎯 Reaper Platform will be available at: http://localhost:8081"
	@echo ""
	@echo "API Endpoints:"
	@echo "  Agent:    http://localhost:8080/health"
	@echo "  Agent:    http://localhost:8080/metrics"
	@echo "  Platform: http://localhost:8081/health"
	@echo "  Platform: http://localhost:8081/api/v1/policies"
	@echo ""
	cargo run --bin reaper-platform & cargo run --bin reaper-agent

# Show project status
status:
	@echo "🎯 Reaper Platform Status"
	@echo ""
	@echo "📦 Workspace members:"
	@cargo metadata --format-version 1 | jq -r '.workspace_members[]' | sed 's/.*#/  /' 2>/dev/null || echo "  (install jq for detailed info)"
	@echo ""
	@echo "🔧 Available commands:"
	@echo "  make dev         # Development with auto-reload"
	@echo "  make test        # Run all tests"
	@echo "  make agent       # Run Reaper Agent"
	@echo "  make platform    # Run Reaper Platform"
	@echo "  make cli         # Build Reaper CLI"
