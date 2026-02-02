# Test Fixtures

Unified test fixtures for Reaper policy engine testing.

## Directory Structure

```
test-fixtures/
├── policies/           # Symlinks to test policy files
│   ├── core/          # Core policy patterns (RBAC, ABAC, ReBAC, Multilayer)
│   ├── dsl/           # DSL feature coverage policies
│   └── regression/    # Regression test policies
├── data/              # Symlinks to test data (organized by size)
│   ├── small/         # Quick tests (<10KB, ~100 entities)
│   ├── medium/        # Standard tests (1-2MB, ~1000 entities)
│   └── large/         # Scale tests (10MB+, ~100K entities)
├── suites/            # Integration test suite definitions (YAML)
├── expectations/      # Expected test results
└── comparison/        # OPA semantic comparison tests
    ├── test_cases/    # YAML comparison test definitions
    ├── opa_policies/  # Equivalent OPA/Rego policies
    └── docker-compose.yml  # Docker setup for Reaper+OPA
```

## Data Organization

All data files in `data/` are **symlinks** to `test-data/`:

| Directory | Size | Entity Count | Purpose |
|-----------|------|--------------|---------|
| `data/small/` | <10KB | ~100 | Unit tests, BDD scenarios |
| `data/medium/` | 1-2MB | ~1000 | Integration tests |
| `data/large/` | 10MB+ | ~100K | Volume/stress tests |

### Small Data Files (14 files)
- `rbac.json`, `string.json`, `math.json`, `time.json`
- `regex.json`, `collection.json`, `comprehension.json`, `json.json`
- `conditional.json`, `error-handling.json`, `type-checking.json`
- `advanced-collection.json`, `nested-comprehension.json`, `general.json`

### Medium Data Files
- `abac.json`, `rebac.json`, `multilayer.json`

### Large Data Files
- `huge.json` (100K entities)

## Usage

### Running Integration Tests

```bash
# Run YAML-based integration test suites
cargo test -p policy-engine --test integration_runner

# Run semantic comparison tests (Reaper-only)
cargo test -p policy-engine --test comparison_runner
```

### Running Docker-based OPA Comparison

```bash
cd test-fixtures/comparison

# Start Reaper + OPA
docker compose up -d

# Run comparison tests
./run-comparison.sh

# Cleanup
docker compose down
```

## Test Suite YAML Format

```yaml
name: "Suite Name"
description: "What this suite tests"

policies:
  - path: "crates/policy-engine/examples/policies/rbac.reap"

data:
  - path: "test-data/rbac-test-data.json"

test_cases:
  - name: "Admin access"
    principal: "user_0"
    action: "read"
    resource: "resource_100"
    expected: allow
    tags: [admin, positive]

performance:
  p99_threshold_us: 50
```

## Comparison Test YAML Format

```yaml
name: "RBAC Equivalence"
description: "Verify Reaper matches OPA"

reaper_policy: "crates/policy-engine/examples/policies/rbac.reap"
opa_policy: "comparison/opa_policies/rbac.rego"
data_file: "test-data/rbac-test-data.json"

test_cases:
  - id: "admin_read"
    description: "Admin can read any resource"
    input:
      principal: "user_0"
      action: "read"
      resource: "resource_100"
    expected: allow
```

## Related Directories

| Directory | Purpose | Tracked in Git |
|-----------|---------|----------------|
| `test-data/` | Canonical test data files | No (generated) |
| `benchmarks/reaper-vs-opa/data/` | OPA benchmark data | Yes (Git LFS) |
| `services/reaper-bench/policies/data/` | Benchmark service data | Yes |

## Adding New Tests

1. **Integration Test**: Create YAML in `suites/`, reference existing data
2. **Comparison Test**: Create YAML in `comparison/test_cases/`, add OPA policy
3. **New Data**: Add to `test-data/`, create symlink in appropriate `data/` subdir
