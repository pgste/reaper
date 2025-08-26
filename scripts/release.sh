#!/bin/bash
set -euo pipefail

VERSION=${1:-patch}

echo "🚀 Starting Reaper release process for version: $VERSION"

# Run comprehensive test suite
echo "Running unit tests..."
cargo test --workspace --lib

echo "Running BDD scenarios..."
cargo test --workspace --test '*bdd*'

echo "Running performance benchmarks..."
cargo bench --workspace

echo "Checking code quality..."
cargo fmt --check
cargo clippy --workspace -- -D warnings

echo "Creating Reaper release..."
cargo release $VERSION --execute

echo "✅ Reaper $VERSION released successfully!"
