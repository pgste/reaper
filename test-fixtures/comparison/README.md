# Reaper vs OPA Semantic Equivalence Tests

This directory contains tools for testing that Reaper policies produce the same decisions as equivalent OPA (Rego) policies.

## Quick Start

### Option 1: Docker-based Comparison (Full)

Run both Reaper Agent and Enterprise OPA in Docker, then test both:

```bash
cd test-fixtures/comparison

# Start services and run tests
./run-comparison.sh

# Stop services when done
./run-comparison.sh --cleanup
```

### Option 2: Reaper-only Tests (CI/Unit Testing)

Run Reaper tests without needing OPA:

```bash
# From workspace root
cargo test -p policy-engine --test comparison_runner

# Or via script (without Docker)
./run-comparison.sh --reaper-only --skip-docker
```

### Option 3: Against Running Services

If you already have Reaper and OPA running:

```bash
export REAPER_URL=http://localhost:8080
export OPA_URL=http://localhost:8181
./run-comparison.sh --skip-docker
```

## Directory Structure

```
comparison/
├── docker-compose.yml      # Docker setup for Reaper + OPA
├── run-comparison.sh       # Test runner script
├── README.md               # This file
├── test_cases/             # YAML test case definitions
│   ├── rbac_equivalence.yaml
│   └── string_equivalence.yaml
├── opa_policies/           # Equivalent OPA policies
│   ├── rbac.rego
│   └── string_ops.rego
└── opa_data/               # Data files for OPA (auto-populated)
```

## Test Case Format

Test cases are defined in YAML:

```yaml
name: "RBAC Equivalence"
description: "Verify Reaper and OPA produce same decisions"

reaper_policy: "path/to/policy.reap"
opa_policy: "path/to/policy.rego"
data_file: "path/to/data.json"

test_cases:
  - id: "admin_read"
    description: "Admin can read any resource"
    input:
      principal: "user_0"
      action: "read"
      resource: "resource_100"
    expected: allow
```

## Adding New Tests

1. **Create Reaper Policy** in `crates/policy-engine/examples/policies/`
2. **Create Equivalent OPA Policy** in `opa_policies/`
3. **Add Test Data** to `test-data/` (or use existing)
4. **Create Test Case YAML** in `test_cases/`
5. **Run Tests** to verify equivalence

## Understanding Results

```
✓ admin_read: Reaper=allow (45µs), OPA=allow (1234µs), expected=allow
✗ viewer_write: Reaper=deny (38µs), OPA=allow (987µs), expected=deny
```

- Green checkmark (✓) = Both engines match expected result
- Red X (✗) = At least one engine differs from expected
- Time in parentheses = evaluation latency in microseconds

## Troubleshooting

### Services won't start

```bash
# Check Docker logs
docker compose logs reaper
docker compose logs opa

# Rebuild from scratch
docker compose down -v
docker compose up --build
```

### OPA policy errors

```bash
# Validate OPA policy syntax
opa check opa_policies/rbac.rego

# Test OPA policy directly
opa eval -d opa_policies/ -d opa_data/ "data.rbac.allow" \
    --input '{"principal":"user_0","action":"read","resource":"resource_100"}'
```

### Reaper policy errors

```bash
# Validate Reaper policy
cargo run -p reaper-cli -- validate path/to/policy.reap

# Test evaluation directly
cargo run -p reaper-cli -- eval \
    --policy path/to/policy.reap \
    --data path/to/data.json \
    --principal user_0 \
    --action read \
    --resource resource_100
```

## CI Integration

For CI pipelines, use the Rust-based tests which don't require Docker:

```yaml
# .github/workflows/test.yml
- name: Run comparison tests
  run: cargo test -p policy-engine --test comparison_runner
```

For full OPA comparison in CI:

```yaml
- name: Run Docker comparison
  run: |
    cd test-fixtures/comparison
    docker compose up -d
    sleep 10
    ./run-comparison.sh --skip-docker
    docker compose down
```
