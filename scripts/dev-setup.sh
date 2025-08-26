#!/bin/bash
set -euo pipefail

echo "🔧 Setting up Reaper development environment..."

# Install required tools
rustup component add clippy rustfmt
cargo install cargo-release cargo-watch cargo-tarpaulin

echo "✅ Reaper development environment ready!"
echo ""
echo "🎯 Reaper Platform Components:"
echo "  • Reaper Agent    - High-performance policy enforcement"
echo "  • Reaper Platform - Distributed agent management"
echo "  • Reaper CLI      - Command-line management interface"
echo ""
echo "Next steps:"
echo "  make dev     # Start development mode with auto-reload"
echo "  make test    # Run all tests including BDD scenarios"
echo "  make bench   # Run performance benchmarks"