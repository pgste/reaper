# Reaper CI/CD Pipeline

## Pipeline Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     STAGE 1: LINT & ANALYZE                 │
│                         (Sequential)                         │
├─────────────────────────────────────────────────────────────┤
│  • cargo fmt --check                                        │
│  • cargo clippy --workspace -- -D warnings                  │
│  • Generate Clippy report artifact                          │
└──────────────────────┬──────────────────────────────────────┘
                       │
                       ▼
┌─────────────────────────────────────────────────────────────┐
│                   STAGE 2: UNIT TESTS                       │
│                  (Sequential - BLOCKING)                    │
├─────────────────────────────────────────────────────────────┤
│  • cargo test --workspace --lib                             │
│  • Generate test summary                                    │
│  • ⚠️  PIPELINE FAILS IF UNIT TESTS FAIL                    │
└──────────────────────┬──────────────────────────────────────┘
                       │
                       ▼
┌─────────────────────────────────────────────────────────────┐
│            STAGE 3: PARALLEL PERFORMANCE & BDD              │
│                  (All run concurrently)                     │
│                  (continue-on-error: true)                  │
└─────────────────────────────────────────────────────────────┘
                       │
        ┌──────────────┼──────────────┬──────────────┐
        │              │               │              │
        ▼              ▼               ▼              ▼
┌──────────────┐ ┌──────────┐  ┌──────────┐  ┌──────────────┐
│ Volume Tests │ │ Memory   │  │   BDD    │  │   Volume     │
│  (Matrix)    │ │ & Scale  │  │  Tests   │  │   Tests      │
├──────────────┤ │  Test    │  │(Cucumber)│  │  (Matrix)    │
│ • multilayer │ │          │  └──────────┘  │ • rbac       │
│ • abac       │ │ 100k     │                │ • rebac      │
└──────────────┘ │ entities │                └──────────────┘
                 └──────────┘
        │              │               │              │
        └──────────────┴───────────────┴──────────────┘
                       │
                       ▼
┌─────────────────────────────────────────────────────────────┐
│              STAGE 4: GENERATE COMBINED REPORT              │
│                      (if: always())                         │
├─────────────────────────────────────────────────────────────┤
│  • Download all artifacts                                   │
│  • Generate COMBINED_REPORT.md                              │
│  • Comment on PR (if pull request)                          │
│  • Upload combined report artifact                          │
└─────────────────────────────────────────────────────────────┘
```

## Workflow File

Location: `.github/workflows/ci.yml`

## Key Features

### 🔒 Build Failure Policy
- **Unit tests MUST pass** - Pipeline fails immediately if unit tests fail
- **Performance tests DON'T fail build** - All volume/memory/BDD tests use `continue-on-error: true`
- This allows monitoring performance regressions without blocking releases

### ⚡ Parallel Execution
All Stage 3 jobs run concurrently using GitHub Actions matrix strategy:
- 4x Volume tests (multilayer, rbac, abac, rebac)
- 1x Memory & Scale test (100k entities)
- 1x BDD test suite

Total Stage 3 time ≈ slowest individual job time (not sum of all)

### 📦 Artifacts
All test outputs are preserved as downloadable artifacts:

| Artifact Name | Description | Retention |
|--------------|-------------|-----------|
| `clippy-report` | Linting results | 90 days |
| `unit-test-results` | Unit test output + summary | 90 days |
| `volume-test-multilayer` | Multilayer policy 10k test | 90 days |
| `volume-test-rbac` | RBAC policy 10k test | 90 days |
| `volume-test-abac` | ABAC policy 10k test | 90 days |
| `volume-test-rebac` | ReBAC policy 10k test | 90 days |
| `memory-volume-test` | 100k entity test + comparison | 90 days |
| `bdd-test-results` | BDD/Cucumber test output | 90 days |
| `cucumber-report` | Cucumber JSON report | 90 days |
| `combined-test-report` | Comprehensive markdown summary | 90 days |

### 📊 Reports on Pull Requests

The pipeline automatically comments on PRs with a comprehensive test summary including:
- Unit test results
- Performance metrics for all policy types
- Memory efficiency analysis
- BDD test coverage
- Links to all artifacts

## Local Testing

Run the same checks locally before pushing:

```bash
# Stage 1: Lint & analyze
cargo fmt --check
cargo clippy --workspace -- -D warnings

# Stage 2: Unit tests
cargo test --workspace --lib

# Stage 3: Volume tests (example)
cargo run -p policy-engine --example generate_multilayer_data --release
cargo run -p policy-engine --example test_multilayer_10k --release

# Stage 3: BDD tests
cargo test --workspace --test '*bdd*'
```

## Performance Metrics Tracked

### Volume Tests (10k iterations each)
- Mean latency (target: < 1µs)
- P95 latency
- P99 latency (target: < 2µs)
- Max latency
- Throughput (ops/second)
- Decision distribution (allow/deny)
- Latency buckets (< 500ns, < 1µs, < 2µs, etc.)

### Memory & Scale Test (100k entities)
- Data file size
- Load time
- Memory usage (RSS)
- Memory per entity (target: < 3KB)
- Compression ratio
- Evaluation latency comparison (1k vs 100k)
- Memory leak detection

### BDD Tests
- Feature coverage
- Scenario execution
- Step definitions
- Pass/fail status

## Triggering the Pipeline

The pipeline runs automatically on:
- Push to `main` branch
- Push to `develop` branch
- Pull requests targeting `main` or `develop`

Manual trigger:
```bash
# Push to trigger
git push origin your-branch

# Create PR
gh pr create --base main --head your-branch
```

## Troubleshooting

### Pipeline Fails at Lint Stage
```bash
# Fix formatting
cargo fmt

# Fix Clippy warnings
cargo clippy --workspace --fix
```

### Pipeline Fails at Unit Tests
```bash
# Run tests locally
cargo test --workspace --lib -- --nocapture

# Run specific test
cargo test -p policy-engine --lib test_name
```

### Performance Tests Show Degradation
Performance tests don't fail the build, but degradation should be investigated:

```bash
# Run benchmarks locally
make bench-summary

# Run specific volume test
cargo run -p policy-engine --example test_multilayer_10k --release
```

## Customization

### Adjusting Test Iterations
Edit the example files to change iteration counts:
- `crates/policy-engine/examples/test_*_10k.rs`
- Change `iterations = 10_000` to desired value

### Adding New Volume Tests
1. Create new example in `crates/policy-engine/examples/`
2. Add to matrix in `.github/workflows/ci.yml`:
   ```yaml
   matrix:
     policy: [multilayer, rbac, abac, rebac, your-new-test]
   ```

### Modifying Artifact Retention
Edit `.github/workflows/ci.yml`:
```yaml
- name: Upload artifact
  uses: actions/upload-artifact@v3
  with:
    retention-days: 30  # Change from default 90
```
