# Test Data

This directory contains test data files for the Reaper policy engine. Most files are **generated** by example programs and are **not tracked in git**.

## Quick Start

```bash
# Generate required test data for CI
cargo run -p policy-engine --example generate_rbac_data --release
cargo run -p policy-engine --example generate_abac_data --release

# Generate all data (including large datasets)
./scripts/run_scale_tests.sh
```

## File Categories

### Small Test Data (<10KB) - Unit Tests & BDD
These small files are used by unit tests and BDD scenarios:

| File | Purpose | Used By |
|------|---------|---------|
| `string-test-data.json` | String operations (lower, upper, trim, contains) | String BDD tests |
| `math-test-data.json` | Math operations (add, subtract, comparisons) | Math BDD tests |
| `time-test-data.json` | Time/date operations | Time BDD tests |
| `regex-test-data.json` | Regex matching patterns | Regex BDD tests |
| `collection-test-data.json` | Collection operations (count, sum, filter) | Collection BDD tests |
| `comprehension-test-data.json` | Comprehension expressions | Comprehension tests |
| `json-test-data.json` | JSON path operations | JSON BDD tests |
| `conditional-test-data.json` | Conditional logic (if/else) | Conditional tests |
| `error-handling-test-data.json` | Edge cases (null, empty, unicode) | Error handling tests |
| `test-data.json` | General small dataset | Quick examples |

### Medium Datasets (~1-2MB) - Integration Tests
Used by integration and volume tests:

| File | Size | Entities | Purpose |
|------|------|----------|---------|
| `rbac-test-data.json` | 772K | 3,000 | Role-based access control |
| `abac-test-data.json` | 1.1M | 4,000 | Attribute-based access control |
| `rebac-test-data.json` | 1.6M | 5,000 | Relationship-based access control |
| `multilayer-test-data.json` | 2.1M | 7,000 | Combined RBAC+ABAC+ReBAC |

### Large Datasets (10MB+) - Stress Tests
Used by memory and volume tests:

| File | Size | Entities | Purpose |
|------|------|----------|---------|
| `huge-test-data.json` | 39M | 100,000 | Memory/volume tests |
| `dualsource-attributes-large.json` | 26M | 50,000 | Dual-source joins |
| `dualsource-resources-large.json` | 45M | 50,000 | Dual-source queries |
| `dualsource-roles-large.json` | 11M | 50,000 | Dual-source roles |

### Small Dual-Source Files
For dual-source policy testing:

| File | Size |
|------|------|
| `dualsource-roles-small.json` | 20K |
| `dualsource-attributes-small.json` | 40K |
| `dualsource-resources-small.json` | 67K |

## Data Format

All files follow the standard entity format:

```json
{
  "entities": [
    {
      "id": "user_123",
      "type": "User",
      "attributes": {
        "name": "Alice",
        "role": "admin",
        "department": "engineering"
      }
    }
  ]
}
```

## Related Directories

| Directory | Purpose | Relationship |
|-----------|---------|--------------|
| `test-fixtures/data/` | Organized symlinks | Points to files here (small/medium/large) |
| `services/reaper-bench/policies/data/` | Benchmark data | Separate minimal datasets for benchmarks |
| `benchmarks/reaper-vs-opa/data/` | OPA comparison | Git LFS tracked, 100K entity datasets |

## Generating Test Data

```bash
# Generate standard test data
cargo run -p policy-engine --example generate_rbac_data --release
cargo run -p policy-engine --example generate_abac_data --release
cargo run -p policy-engine --example generate_multilayer_data --release
cargo run -p policy-engine --example generate_rebac_data --release

# Generate large datasets (for stress testing)
cargo run -p policy-engine --example generate_large_data --release
cargo run -p policy-engine --example generate_huge_data --release

# Generate dual-source data
cargo run -p policy-engine --example generate_dualsource_data --release
```

Or use the convenience script:

```bash
./scripts/run_scale_tests.sh
```

## Git Ignore

This directory is listed in `.gitignore` because test data is:
- Generated from code (reproducible)
- Large (100+ MB total when fully generated)
- Not needed in version control

## Size Reference

| Category | Files | Total Size |
|----------|-------|------------|
| Small (<10KB) | 10 | ~50KB |
| Medium (1-2MB) | 4 | ~6MB |
| Large (10MB+) | 4 | ~120MB |
| **Total** | 18+ | **~127MB** |

## Usage in Code

```rust
// Load test data
let data = fs::read_to_string("test-data/rbac-test-data.json")?;
let store = DataStore::new();
let loader = DataLoader::new(store.clone());
loader.load_json(&data)?;
```
