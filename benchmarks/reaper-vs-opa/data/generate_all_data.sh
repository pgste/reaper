#!/bin/bash
# Generate 100k datasets for all policy scenarios

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DATA_DIR="$SCRIPT_DIR/100k"

# Create directories
mkdir -p "$DATA_DIR"

# Generate datasets
echo "Generating 100k datasets for all scenarios..."

cd "$(dirname "$0")/.."

# Build data generator
echo "Building data generator..."
cargo build --release --bin generate-data

# Math policy data
echo "Generating math policy data..."
./target/release/generate-data --count 100000 --output "$DATA_DIR/math.json" math

# Regex policy data
echo "Generating regex policy data..."
./target/release/generate-data --count 100000 --output "$DATA_DIR/regex.json" regex

# Time policy data
echo "Generating time policy data..."
./target/release/generate-data --count 100000 --output "$DATA_DIR/time.json" time

# String policy data
echo "Generating string policy data..."
./target/release/generate-data --count 100000 --output "$DATA_DIR/string.json" string

# Collection policy data
echo "Generating collection policy data..."
./target/release/generate-data --count 100000 --output "$DATA_DIR/collection.json" collection

# Comprehension policy data
echo "Generating comprehension policy data..."
./target/release/generate-data --count 100000 --output "$DATA_DIR/comprehension.json" comprehension

# JSON policy data
echo "Generating json policy data..."
./target/release/generate-data --count 100000 --output "$DATA_DIR/json.json" json

# Mega policy data (combines all patterns)
echo "Generating mega policy data..."
./target/release/generate-data --count 100000 --output "$DATA_DIR/mega.json" mega

echo "✓ All datasets generated successfully!"
echo "  Output directory: $DATA_DIR"
echo "  Total datasets: 8 scenarios × 100,000 entities each"
