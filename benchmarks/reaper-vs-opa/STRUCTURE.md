# Benchmark Directory Structure

Clean, organized structure for Reaper vs OPA benchmarks.

## Directory Tree

```
benchmarks/reaper-vs-opa/
├── bin/                      # Scripts
│   ├── benchmark.sh          # Main entry point ⭐
│   ├── deploy-reaper.sh      # Deploy helper
│   ├── deploy-opa.sh         # Deploy helper
│   └── cleanup.sh            # Cleanup helper
│
├── data/                     # Test data
│   ├── 10k/                  # 12K entities (realistic)
│   │   ├── rbac.json
│   │   ├── abac.json
│   │   ├── rebac.json
│   │   └── multilayer.json
│   └── 100k/                 # 102K entities (stress test)
│       └── (same files)
│
├── policies/                 # Policy definitions
│   ├── reaper/               # Reaper DSL (.reap)
│   │   ├── rbac.reap
│   │   ├── abac.reap
│   │   ├── rebac.reap
│   │   └── multilayer.reap
│   └── opa/                  # Rego policies
│       ├── rbac.rego
│       ├── abac.rego
│       ├── rebac.rego
│       └── multilayer.rego
│
├── results/                  # Generated results
│   ├── 10k/
│   │   └── {scenario}/
│   │       ├── results.json
│   │       └── report.txt
│   └── 100k/
│       └── (same structure)
│
├── src/                      # Benchmark tool source
│   └── main.rs
│
├── Cargo.toml                # Rust dependencies
└── README.md                 # Usage guide
```

## Quick Reference

### Run Benchmarks
```bash
# Single scenario
./bin/benchmark.sh --scenario multilayer --scale 10k

# All scenarios
./bin/benchmark.sh --scenario all --scale 10k

# Full test suite (all scenarios, both scales)
./bin/benchmark.sh --scenario all --scale both
```

### Files by Purpose

| Purpose | Files | Location |
|---------|-------|----------|
| **Running benchmarks** | `benchmark.sh` | `bin/` |
| **Test data** | `{scenario}.json` | `data/10k/` or `data/100k/` |
| **Policies** | `{scenario}.reap`, `{scenario}.rego` | `policies/reaper/`, `policies/opa/` |
| **Results** | `results.json`, `report.txt` | `results/{scale}/{scenario}/` |
| **Documentation** | `README.md`, `STRUCTURE.md` | Root |

### Archived Files

Old scripts and docs are in `.archive/` directory (26 files archived).

## Navigation

- **Start here**: `README.md` - Full usage guide
- **Run benchmarks**: `./bin/benchmark.sh --help`
- **View results**: `results/{scale}/{scenario}/report.txt`
- **Modify policies**: `policies/reaper/` or `policies/opa/`
- **Add test data**: `data/10k/` or `data/100k/`

## Maintenance

```bash
# Clean up between runs
./bin/cleanup.sh

# View archived files
ls -la .archive/

# Restore from archive if needed
mv .archive/{filename} .
```

Clean and simple! 🎯
