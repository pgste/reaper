# OPA vs Reaper Comparison Suite

Comprehensive benchmark suite comparing Reaper and OPA (Open Policy Agent) across multiple policy patterns with 100k+ entity datasets.

## Overview

This suite tests 8 different policy scenarios with 100,000 entities each to provide realistic performance comparisons and identify linear scaling vs degradation patterns.

## Policy Scenarios

### 1. **Math Policy** (8 rules)
Tests numeric validation and mathematical operations.

**Rules**:
- Credit score thresholds (≥ 700)
- Budget limit checks
- Average rating validation (≥ 4.0)
- Price range validation (1-10,000)
- Tier upgrades based on score (Bronze: 50-70, Silver: 70-90, Gold: 90+)
- Temperature range checks (-50 to +50)
- Loyalty points tiers (Basic: 100-500, Premium: 500-1000, Elite: 1000+)
- Discount percentage validation (0-50%)

**Dataset**: 100k users with varying numeric attributes

**Files**:
- Reaper: `policies/reaper/math.reap`
- OPA: `policies/opa/math.rego`
- Data: `data/100k/math.json`

---

### 2. **Regex Policy** (9 rules)
Tests pattern matching with regex validation.

**Rules**:
- Email validation (RFC-compliant)
- Phone number validation (US format)
- URL validation (http/https)
- IPv4 address validation
- UUID validation
- Credit card number validation
- ZIP code validation
- HEX color validation
- ISO date validation
- 24-hour time validation
- Domain name validation
- Username pattern validation
- MAC address validation
- IPv6 address validation
- Semantic version validation

**Dataset**: 100k users with various formatted strings

**Files**:
- Reaper: `policies/reaper/regex.reap`
- OPA: `policies/opa/regex.rego`
- Data: `data/100k/regex.json`

---

### 3. **Time Policy** (12 rules)
Tests time-based access control with nanosecond precision timestamps.

**Rules**:
- Token expiration checks
- Business hours validation (9 AM - 5 PM)
- Age verification (18+, 21+)
- Lease expiration checks
- Maintenance window validation
- Session expiration checks
- Future event scheduling
- Temporary access grants (contractors)
- Subscription expiration
- Trial period validation
- Contract date validation
- Certification expiration
- Rate limiting with time windows
- Data retention policies

**Dataset**: 100k users with various timestamp attributes

**Files**:
- Reaper: `policies/reaper/time.reap`
- OPA: `policies/opa/time.rego`
- Data: `data/100k/time.json`

---

### 4. **String Policy** (8 rules)
Tests string manipulation and comparison operations.

**Rules**:
- Case-insensitive matching (lowercase, uppercase)
- String trimming and whitespace handling
- Email domain checks (`contains`)
- Username prefix validation (`startswith`)
- Email suffix validation (`endswith`, .gov, .mil, .edu)
- Full name validation (split and count)
- Complex chained string operations

**Dataset**: 100k users with various string formats

**Files**:
- Reaper: `policies/reaper/string.reap`
- OPA: `policies/opa/string.rego`
- Data: `data/100k/string.json`

---

### 5. **Collection Policy** (9 rules)
Tests array, set, and map operations.

**Rules**:
- Array contains checks (permissions: read, write, delete)
- Array length validation (skills count for junior/mid/senior positions)
- Set intersection (group overlap)
- Set subset validation (allowed tags only)
- Array "any" checks (has admin role)
- Array "all" checks (all projects active)
- Map keys validation (required metadata)
- Comprehension filters (verified emails only)
- Nested array access (department permissions)

**Dataset**: 100k users with collections (2-7 items each)

**Files**:
- Reaper: `policies/reaper/collection.reap`
- OPA: `policies/opa/collection.rego`
- Data: `data/100k/collection.json`

---

### 6. **Comprehension Policy** (6 rules)
Tests set, array, and object comprehensions with filters.

**Rules**:
- Set comprehension with filters (numbers > 5)
- Array comprehension preserving order (priority filtering)
- Object comprehension creating mappings (active records)
- Multi-filter comprehensions (score > 80 AND verified)
- Nested iteration comprehensions
- Transformation comprehensions (uppercase strings with "a")

**Dataset**: 100k users with arrays, sets, and nested structures

**Files**:
- Reaper: `policies/reaper/comprehension.reap`
- OPA: `policies/opa/comprehension.rego`
- Data: `data/100k/comprehension.json`

---

### 7. **JSON Policy** (10 rules)
Tests JSON parsing, validation, and manipulation.

**Rules**:
- JSON payload validation
- JSON path access (nested fields)
- Nested JSON structure checks (payment.card.number)
- JSON array validation (order items present)
- JSON type checking (string, number, boolean)
- JSON object field validation
- JSON merge checks (primary + secondary data)

**Dataset**: 100k users with complex JSON structures

**Files**:
- Reaper: `policies/reaper/json.reap`
- OPA: `policies/opa/json.rego`
- Data: `data/100k/json.json`

---

### 8. **Mega Policy** (105 rules)
Combines ALL patterns from above policies to test linear scaling vs degradation.

**Purpose**: Tests whether policy engines degrade gracefully or linearly as rule count increases.

**Rule Distribution**:
- Math operations: 15 rules
- String operations: 15 rules
- Regex validation: 15 rules
- Time operations: 15 rules
- Collection operations: 15 rules
- Comprehension operations: 15 rules
- JSON operations: 15 rules

**Total**: 105 rules exercising all policy patterns

**Dataset**: 100k users with ALL attributes from all scenarios (33+ attributes per user)

**Files**:
- Reaper: `policies/reaper/mega.reap`
- OPA: `policies/opa/mega.rego`
- Data: `data/100k/mega.json`

---

## Generating Datasets

### Quick Start

Generate all 100k datasets:

```bash
cd benchmarks/reaper-vs-opa
./data/generate_all_data.sh
```

This generates 8 datasets totaling **800,000 entities**.

### Custom Generation

Generate specific scenarios:

```bash
cd benchmarks/reaper-vs-opa
cargo build --release --bin generate-data

# Math dataset (100k)
./target/release/generate-data --count 100000 --output data/100k/math.json math

# Mega dataset (100k)
./target/release/generate-data --count 100000 --output data/100k/mega.json mega

# Smaller test dataset
./target/release/generate-data --count 1000 --output data/1k/math.json math
```

### Available Generators

- `math` - Numeric validation data
- `regex` - Pattern matching data
- `time` - Timestamp-based data
- `string` - String manipulation data
- `collection` - Array/set/map data
- `comprehension` - Comprehension expression data
- `json` - JSON structure data
- `mega` - Combined data (all patterns)

---

## Running Benchmarks

### Individual Scenarios

```bash
# Math policy benchmark
./bin/benchmark.sh --scenario math --scale 100k --requests 50000

# Regex policy benchmark
./bin/benchmark.sh --scenario regex --scale 100k --requests 50000

# Mega policy benchmark (100+ rules)
./bin/benchmark.sh --scenario mega --scale 100k --requests 50000
```

### All Scenarios

```bash
for scenario in math regex time string collection comprehension json mega; do
  ./bin/benchmark.sh --scenario $scenario --scale 100k --requests 50000
done
```

---

## Expected Results

### Performance Targets

| Scenario | Reaper (µs) | OPA Enterprise (µs) | Speedup |
|----------|-------------|---------------------|---------|
| Math | 1-3 | 10-30 | 3-10x |
| Regex | 2-5 | 15-40 | 5-10x |
| Time | 1-3 | 10-25 | 5-10x |
| String | 1-2 | 8-20 | 5-10x |
| Collection | 3-8 | 20-50 | 5-10x |
| Comprehension | 5-10 | 30-80 | 5-10x |
| JSON | 2-5 | 15-35 | 5-10x |
| **Mega (105 rules)** | **10-50** | **100-500** | **10-20x** |

### Linear Scaling Test (Mega Policy)

The **Mega Policy** with 105 rules is designed to answer:

1. **Does performance degrade linearly?**
   - Reaper: Expected linear O(n) with number of rules
   - OPA: May show quadratic degradation O(n²) due to Rego interpretation overhead

2. **What's the degradation factor?**
   - Measure latency per rule: `total_latency / rule_count`
   - Compare 10-rule vs 100-rule policies

3. **Where's the breaking point?**
   - At what rule count does each engine become unacceptable (> 100ms)?

---

## File Structure

```
benchmarks/reaper-vs-opa/
├── policies/
│   ├── reaper/
│   │   ├── math.reap
│   │   ├── regex.reap
│   │   ├── time.reap
│   │   ├── string.reap
│   │   ├── collection.reap
│   │   ├── comprehension.reap
│   │   ├── json.reap
│   │   └── mega.reap           # 105 rules
│   └── opa/
│       ├── math.rego
│       ├── regex.rego
│       ├── time.rego
│       ├── string.rego
│       ├── collection.rego
│       ├── comprehension.rego
│       ├── json.rego
│       └── mega.rego           # 105 rules
├── data/
│   ├── 100k/
│   │   ├── math.json           # 100,000 entities
│   │   ├── regex.json          # 100,000 entities
│   │   ├── time.json           # 100,000 entities
│   │   ├── string.json         # 100,000 entities
│   │   ├── collection.json     # 100,000 entities
│   │   ├── comprehension.json  # 100,000 entities
│   │   ├── json.json           # 100,000 entities
│   │   └── mega.json           # 100,000 entities (33+ attrs each)
│   └── generate_all_data.sh
├── src/
│   ├── main.rs                 # Benchmark harness
│   └── bin/
│       └── generate_data.rs    # Data generator
├── bin/
│   ├── benchmark.sh
│   ├── deploy-reaper.sh
│   ├── deploy-opa.sh
│   └── cleanup.sh
└── OPA_COMPARISON_SUITE.md     # This file
```

---

## Analysis Questions

### 1. Linear Scaling

**Question**: Does performance scale linearly with rule count?

**Test**:
- Run math policy (8 rules) → measure P50 latency
- Run mega policy (105 rules) → measure P50 latency
- Calculate: `latency_per_rule = total_latency / rule_count`

**Expected**:
- Reaper: Linear scaling (O(n))
- OPA: May show super-linear degradation

### 2. Rule Complexity Impact

**Question**: Which types of rules cause the most degradation?

**Test**: Compare scenarios:
- Math (simple numeric) vs Comprehension (complex iteration)
- String (simple contains) vs Regex (complex patterns)
- Time (simple comparison) vs Collection (nested iteration)

### 3. Dataset Size Impact

**Question**: Does dataset size affect evaluation speed?

**Test**:
- Run with 1k, 10k, 100k datasets
- Measure latency for same policy
- Check if latency increases with dataset size

### 4. Throughput Under Load

**Question**: How many requests/second can each handle?

**Test**:
- Run with 50 concurrent connections
- Measure RPS (requests per second)
- Compare Reaper vs OPA Enterprise

---

## CI Integration

The GitHub Actions workflow (`.github/workflows/benchmark.yml`) already runs RBAC, ABAC, ReBAC, and Multilayer benchmarks.

To add these new scenarios:

```yaml
strategy:
  matrix:
    scenario: [rbac, abac, rebac, multilayer, math, regex, time, string, collection, comprehension, json, mega]
    scale: [100k]
```

---

## Summary

This comparison suite provides:

- ✅ **8 policy scenarios** covering all major policy patterns
- ✅ **100k datasets** for realistic performance testing
- ✅ **105-rule mega policy** to test linear scaling vs degradation
- ✅ **Equivalent Rego policies** for fair comparison
- ✅ **Automated data generation** for reproducibility
- ✅ **Comprehensive coverage** of Reaper DSL features

**Total Test Coverage**:
- 62 individual rules across 7 scenarios
- 105 rules in mega policy
- 800,000 total test entities
- All major policy patterns (RBAC, ABAC, ReBAC, math, regex, time, strings, collections, comprehensions, JSON)
