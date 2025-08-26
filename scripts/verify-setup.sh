#!/bin/bash
set -euo pipefail

echo "🔍 Verifying Reaper setup..."

# Check workspace can build
echo "  ✓ Checking workspace build..."
cargo check --workspace

# Check individual components
echo "  ✓ Checking Reaper Agent..."
cargo check --bin reaper-agent

echo "  ✓ Checking Reaper Platform..."
cargo check --bin reaper-platform

echo "  ✓ Checking Reaper CLI..."
cargo check --bin reaper-cli

# Run basic tests
echo "  ✓ Running unit tests..."
cargo test --workspace --lib

echo ""
echo "✅ Reaper setup verification complete!"
echo ""
echo "🎯 Next steps:"
echo "  1. Run 'make setup' to install development tools"
echo "  2. Run 'make dev' to start development mode"
echo "  3. Run 'make dev-services' to test the services"
echo "  4. Implement your first vertical feature!"
