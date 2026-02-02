# Reaper Testing Strategy

**Author:** Testing QA Expert Review
**Date:** 2026-02-02
**Status:** Proposed

---

## Executive Summary

This document outlines a comprehensive testing strategy for the Reaper policy engine, building upon the existing excellent unit and BDD test suites. The strategy addresses:

1. **Extended BDD Suite** - Complete coverage of all DSL features
2. **Integration Testing** - Real engine/agent testing with policy + data
3. **OPA vs Reaper Comparison** - Semantic equivalence testing
4. **Unified Test Data Management** - Consistent policy/data organization

---

## Current Testing Landscape

### What We Have (Excellent Foundation)

| Component | Type | Count | Coverage |
|-----------|------|-------|----------|
| Unit Tests | `cargo test --lib` | 508 tests | Core functionality |
| BDD Tests | Gherkin/Cucumber | 295 scenarios | RBAC, ABAC, Multilayer |
| E2E Tests | `tests/e2e/` | 15+ tests | Management + Agent flow |
| OPA Comparison | `benchmarks/reaper-vs-opa/` | 8 scenarios | Performance comparison |
| Reaper Bench | Service | 18 policies | Live benchmarking |

### Identified Gaps

1. **BDD Coverage Gaps:**
   - Regex validation scenarios limited
   - Comprehension edge cases not fully covered
   - Error handling scenarios sparse
   - Nested data structure tests minimal

2. **Integration Testing Gaps:**
   - No unified test runner for policy+data combinations
   - Inconsistent data file locations
   - No automated regression suite for deployed policies

3. **OPA Comparison Gaps:**
   - Semantic equivalence not tested (only performance)
   - No CI integration for policy parity checks
   - Missing hybrid test cases (same input → same decision)

---

## Proposed Testing Architecture

```
tests/
├── unit/                    # Existing: cargo test --lib
├── bdd/                     # Extended BDD suite
│   ├── features/
│   │   ├── core/           # Core functionality
│   │   ├── dsl/            # DSL feature coverage
│   │   └── regression/     # Regression scenarios
│   └── steps/
├── integration/             # New: Engine + Agent integration
│   ├── fixtures/
│   │   ├── policies/       # Test policies (.reap)
│   │   ├── data/           # Test data (.json)
│   │   └── expectations/   # Expected results
│   └── suites/
│       ├── rbac_suite.rs
│       ├── abac_suite.rs
│       └── multilayer_suite.rs
├── comparison/              # OPA parity testing
│   ├── policies/
│   │   ├── reaper/         # .reap versions
│   │   └── opa/            # .rego versions
│   ├── test_cases/         # Input/output pairs
│   └── runner/             # Comparison harness
└── performance/             # Performance regression
    ├── baselines/          # Recorded baselines
    └── benchmarks/         # Criterion benchmarks
```

---

## Phase 1: Extended BDD Suite

### Goal
Achieve 100% coverage of Reaper DSL features through BDD scenarios.

### New Feature Files to Add

#### 1. `features/dsl/string_operations.feature`
```gherkin
@string @dsl
Feature: String Operations
  Complete coverage of string manipulation functions

  Background:
    Given the policy file "test-policies/string_ops.reap"
    And the data file "test-data/string_users.json"

  @contains
  Scenario: String contains check - positive
    Given a user "alice" with email "alice@company.com"
    When they perform action "access" on resource "internal-docs"
    Then the decision should be "allow"
    And the reason should include "email domain verified"

  @contains @negative
  Scenario: String contains check - external domain
    Given a user "bob" with email "bob@external.com"
    When they perform action "access" on resource "internal-docs"
    Then the decision should be "deny"

  @startswith
  Scenario Outline: Username prefix validation
    Given a user with username "<username>"
    When they perform action "access" on resource "service/<type>"
    Then the decision should be "<decision>"

    Examples:
      | username    | type    | decision |
      | svc_api     | api     | allow    |
      | svc_worker  | worker  | allow    |
      | admin_bob   | api     | deny     |
      | user_alice  | worker  | deny     |

  @lowercase @uppercase
  Scenario: Case-insensitive matching
    Given a user with role "ADMIN"
    When they perform action "manage" on resource "system"
    Then the decision should be "allow"
    And the policy should have normalized role to "admin"

  @trim
  Scenario: Whitespace handling
    Given a user with department "  engineering  "
    When they perform action "access" on resource "code"
    Then the decision should be "allow"

  @split
  Scenario: Path segment matching
    Given a user with path "/org/team/project"
    When they perform action "access" on resource "project"
    Then the decision should be "allow"
    And path segments should be ["org", "team", "project"]
```

#### 2. `features/dsl/regex_validation.feature`
```gherkin
@regex @dsl
Feature: Regex Pattern Validation
  Comprehensive regex pattern matching tests

  Background:
    Given the policy file "test-policies/regex_validation.reap"

  @email
  Scenario Outline: Email format validation
    Given a user with email "<email>"
    When they request resource access
    Then the email validation should be "<result>"

    Examples:
      | email                  | result |
      | user@domain.com        | valid  |
      | user.name@domain.co.uk | valid  |
      | user+tag@domain.com    | valid  |
      | invalid@              | invalid |
      | @domain.com           | invalid |
      | no-at-sign.com        | invalid |

  @uuid
  Scenario: UUID format validation
    Given a resource with id "550e8400-e29b-41d4-a716-446655440000"
    When validating the resource id
    Then the UUID should be valid

  @ipv4
  Scenario Outline: IPv4 address validation
    Given a context with ip_address "<ip>"
    When checking IP validity
    Then the result should be "<result>"

    Examples:
      | ip              | result  |
      | 192.168.1.1     | valid   |
      | 10.0.0.1        | valid   |
      | 256.1.1.1       | invalid |
      | 192.168.1       | invalid |

  @custom_pattern
  Scenario: Custom regex pattern matching
    Given a user with employee_id "EMP-12345-US"
    And a policy rule checking pattern "EMP-\d{5}-[A-Z]{2}"
    When evaluating the pattern
    Then the match should succeed

  @regex_escape
  Scenario: Regex special character escaping
    Given a filename containing special characters "file[1].txt"
    When matching against escaped pattern
    Then the literal match should succeed
```

#### 3. `features/dsl/comprehensions.feature`
```gherkin
@comprehension @dsl
Feature: Collection Comprehensions
  Test set, array, and object comprehension operations

  Background:
    Given the policy file "test-policies/comprehensions.reap"
    And the data file "test-data/comprehension_data.json"

  @set_comprehension
  Scenario: Set comprehension with filter
    Given a user with scores [85, 92, 78, 95, 88]
    When filtering scores > 80
    Then the result set should contain [85, 92, 95, 88]

  @array_comprehension
  Scenario: Array comprehension preserving order
    Given a user with priorities ["high", "low", "critical", "medium"]
    When filtering critical and high priorities
    Then the result should be ["high", "critical"] in order

  @object_comprehension
  Scenario: Object comprehension mapping
    Given records with active status
    When creating active_by_id mapping
    Then the object should have keys for all active records

  @nested_comprehension
  Scenario: Nested comprehension
    Given departments with teams containing members
    When extracting all senior members
    Then the flattened list should contain all seniors

  @comprehension_with_transform
  Scenario: Comprehension with transformation
    Given values [1, 2, 3, 4, 5]
    When transforming with x * 2 for x > 2
    Then result should be [6, 8, 10]

  @empty_comprehension
  Scenario: Comprehension with no matches
    Given an empty array []
    When applying any comprehension
    Then result should be empty collection
```

#### 4. `features/dsl/time_operations.feature`
```gherkin
@time @dsl
Feature: Time-Based Policy Evaluation
  Comprehensive time operation testing

  Background:
    Given the policy file "test-policies/time_based.reap"
    And the current time is "2026-02-02T10:30:00Z"

  @business_hours
  Scenario Outline: Business hours access control
    Given the current time is "<time>"
    When a user requests access during business hours check
    Then the decision should be "<decision>"

    Examples:
      | time                    | decision |
      | 2026-02-02T09:00:00Z    | allow    |
      | 2026-02-02T17:00:00Z    | allow    |
      | 2026-02-02T08:59:00Z    | deny     |
      | 2026-02-02T17:01:00Z    | deny     |
      | 2026-02-02T02:00:00Z    | deny     |

  @token_expiry
  Scenario: Token expiration check
    Given a user with token_expires_at "2026-02-02T12:00:00Z"
    And current time is "2026-02-02T10:00:00Z"
    When they request access
    Then the decision should be "allow"

  @token_expired
  Scenario: Expired token rejection
    Given a user with token_expires_at "2026-02-01T12:00:00Z"
    And current time is "2026-02-02T10:00:00Z"
    When they request access
    Then the decision should be "deny"
    And reason should be "token expired"

  @age_verification
  Scenario: Age verification from birthdate
    Given a user born on "2000-05-15"
    And current time is "2026-02-02T10:00:00Z"
    When checking if user is over 21
    Then the age check should pass

  @maintenance_window
  Scenario: Maintenance window blocking
    Given a maintenance window from "02:00" to "04:00" UTC
    And current time is "2026-02-02T03:00:00Z"
    When any user requests access
    Then the decision should be "deny"
    And reason should be "maintenance window"

  @rate_limiting
  Scenario: Time-window rate limiting
    Given a user with last_request_at "2026-02-02T10:29:30Z"
    And rate limit of 1 request per minute
    And current time is "2026-02-02T10:30:00Z"
    When they request access
    Then the decision should be "deny"
    And reason should be "rate limit exceeded"
```

#### 5. `features/dsl/error_handling.feature`
```gherkin
@error @dsl
Feature: Error Handling and Edge Cases
  Test policy behavior under error conditions

  @missing_attribute
  Scenario: Missing user attribute
    Given a user without "department" attribute
    When policy checks user.department == "engineering"
    Then the decision should be "deny"
    And no error should be thrown

  @null_value
  Scenario: Null value handling
    Given a user with role set to null
    When policy checks user.role == "admin"
    Then the decision should be "deny"

  @type_mismatch
  Scenario: Type mismatch in comparison
    Given a user with age as string "25"
    When policy checks user.age > 21
    Then the comparison should handle type coercion

  @invalid_regex
  Scenario: Policy with invalid regex pattern
    Given a policy with regex pattern "[invalid"
    When the policy is loaded
    Then a compilation error should be raised

  @circular_reference
  Scenario: Circular reference detection
    Given a policy with variable referencing itself
    When the policy is compiled
    Then a circular reference error should be raised

  @deeply_nested
  Scenario: Deeply nested data access
    Given a user with 10 levels of nested attributes
    When policy accesses user.a.b.c.d.e.f.g.h.i.j
    Then the access should succeed without stack overflow

  @large_collection
  Scenario: Large collection performance
    Given a user with 10000 permissions
    When policy checks if "specific_perm" in user.permissions
    Then evaluation should complete under 100 microseconds
```

### Implementation Plan

1. **Week 1:** Create test policy files for new BDD scenarios
2. **Week 2:** Implement step definitions for new features
3. **Week 3:** Add data fixtures and run full suite
4. **Week 4:** Fix any failures and document gaps

---

## Phase 2: Integration Testing Framework

### Goal
Create a unified integration test framework that loads real policies and data into engine/agent.

### Architecture

```rust
// tests/integration/framework.rs

/// Integration test configuration
pub struct IntegrationTestConfig {
    /// Path to policy file(s)
    pub policies: Vec<PathBuf>,
    /// Path to data file(s)
    pub data: Vec<PathBuf>,
    /// Test cases to run
    pub cases: Vec<TestCase>,
}

/// Single test case
pub struct TestCase {
    pub name: String,
    pub principal: String,
    pub action: String,
    pub resource: String,
    pub context: HashMap<String, Value>,
    pub expected_decision: Decision,
    pub expected_reason: Option<String>,
}

/// Test result
pub struct TestResult {
    pub case: TestCase,
    pub actual_decision: Decision,
    pub actual_reason: Option<String>,
    pub evaluation_time_ns: u64,
    pub passed: bool,
}

/// Run integration tests against real engine
pub async fn run_integration_suite(
    config: IntegrationTestConfig,
) -> Vec<TestResult> {
    // 1. Create PolicyEngine
    // 2. Load policies
    // 3. Load data into DataStore
    // 4. Run each test case
    // 5. Collect results
}
```

### Test Suite YAML Format

```yaml
# tests/integration/suites/rbac_comprehensive.yaml
name: "RBAC Comprehensive Suite"
description: "Full RBAC policy coverage with real engine"

policies:
  - path: "policies/rbac/admin_access.reap"
  - path: "policies/rbac/user_ownership.reap"
  - path: "policies/rbac/manager_reports.reap"

data:
  - path: "data/users_1000.json"
  - path: "data/resources_5000.json"
  - path: "data/roles.json"

test_cases:
  # Admin scenarios
  - name: "Admin full access to any resource"
    principal: "user_admin_001"
    action: "delete"
    resource: "resource_critical_001"
    expected: allow
    tags: [admin, positive]

  - name: "Admin access audit logs"
    principal: "user_admin_001"
    action: "read"
    resource: "audit_log_2024"
    expected: allow
    tags: [admin, audit]

  # Manager scenarios
  - name: "Manager view team reports"
    principal: "user_manager_005"
    action: "read"
    resource: "report_team_005"
    expected: allow
    tags: [manager, positive]

  - name: "Manager cannot delete reports"
    principal: "user_manager_005"
    action: "delete"
    resource: "report_team_005"
    expected: deny
    tags: [manager, negative]

  # User ownership
  - name: "User access own resource"
    principal: "user_regular_100"
    action: "read"
    resource: "resource_user_100_001"
    expected: allow
    tags: [ownership, positive]

  - name: "User cannot access other's resource"
    principal: "user_regular_100"
    action: "read"
    resource: "resource_user_200_001"
    expected: deny
    tags: [ownership, negative]

performance_thresholds:
  p50_us: 5
  p99_us: 50
  max_us: 100
```

### Running Integration Tests

```bash
# Run all integration suites
cargo test -p reaper-integration --test '*'

# Run specific suite
cargo test -p reaper-integration --test rbac_comprehensive

# Run with real agent (HTTP mode)
REAPER_AGENT_URL=http://localhost:8080 cargo test -p reaper-integration --features agent-mode

# Generate coverage report
cargo test -p reaper-integration -- --report html
```

---

## Phase 3: OPA vs Reaper Comparison

### Goal
Verify semantic equivalence between Reaper and OPA policies, not just performance.

### Approach

The existing `benchmarks/reaper-vs-opa/` focuses on performance. We add a **semantic equivalence** layer.

### Policy Parity Test Structure

```
comparison/
├── test_cases/
│   ├── rbac/
│   │   ├── inputs.json          # Test inputs
│   │   └── expected.json        # Expected outputs
│   ├── abac/
│   ├── math/
│   ├── string/
│   ├── regex/
│   ├── time/
│   ├── collection/
│   └── comprehension/
├── policies/
│   ├── reaper/                  # .reap versions
│   └── opa/                     # .rego versions
└── runner/
    └── comparison_test.rs
```

### Test Case Format

```json
// comparison/test_cases/rbac/inputs.json
{
  "test_cases": [
    {
      "id": "admin_full_access",
      "input": {
        "principal": "admin_user",
        "action": "delete",
        "resource": "/admin/sensitive"
      },
      "expected_decision": "allow"
    },
    {
      "id": "user_denied_admin",
      "input": {
        "principal": "regular_user",
        "action": "delete",
        "resource": "/admin/sensitive"
      },
      "expected_decision": "deny"
    }
  ]
}
```

### Comparison Runner

```rust
// comparison/runner/comparison_test.rs

use std::process::Command;

/// Run comparison test between Reaper and OPA
pub async fn run_comparison(scenario: &str) -> ComparisonResult {
    let test_cases = load_test_cases(scenario);
    let reaper_policy = load_reaper_policy(scenario);
    let opa_policy = load_opa_policy(scenario);

    let mut results = Vec::new();

    for case in test_cases {
        // Evaluate with Reaper
        let reaper_result = evaluate_reaper(&reaper_policy, &case.input).await;

        // Evaluate with OPA
        let opa_result = evaluate_opa(&opa_policy, &case.input).await;

        // Compare results
        results.push(CaseResult {
            id: case.id,
            reaper_decision: reaper_result.decision,
            opa_decision: opa_result.decision,
            expected: case.expected_decision,
            match_expected: reaper_result.decision == case.expected_decision,
            engines_agree: reaper_result.decision == opa_result.decision,
            reaper_latency_us: reaper_result.latency_us,
            opa_latency_us: opa_result.latency_us,
        });
    }

    ComparisonResult {
        scenario,
        total_cases: results.len(),
        all_match_expected: results.iter().all(|r| r.match_expected),
        all_engines_agree: results.iter().all(|r| r.engines_agree),
        cases: results,
    }
}
```

### Is OPA Comparison a Good Idea?

**Yes, but with caveats:**

**Benefits:**
1. **Validation** - Confirms Reaper produces semantically equivalent results to a well-known engine
2. **Migration** - Helps teams migrating from OPA validate their policies work identically
3. **Trust** - Builds confidence that Reaper is a drop-in replacement
4. **Regression** - Catches any semantic drift in Reaper's DSL

**Considerations:**
1. **Maintenance** - Need to maintain parallel .rego policies
2. **Feature Parity** - OPA has features Reaper doesn't (and vice versa)
3. **Performance Gap** - OPA is 10-50x slower, so tests take longer
4. **Complexity** - Running OPA requires separate process/container

**Recommendation:**
- **Do include** OPA comparison for core scenarios (RBAC, ABAC, basic operations)
- **Don't require** 100% feature parity - focus on overlapping capabilities
- **Run periodically** (nightly CI) rather than on every commit
- **Focus on semantic equivalence** for key policy patterns, not edge cases

---

## Phase 4: Unified Test Data Management

### Current State (Fragmented)

```
# Data files scattered across:
test-data/rbac-test-data.json
services/reaper-bench/policies/data/*.json
benchmarks/reaper-vs-opa/data/100k/*.json
crates/policy-engine/tests/data/*.json
policies/*.reap (with inline test data)
```

### Proposed Structure

```
test-fixtures/
├── policies/
│   ├── core/                    # Core policy templates
│   │   ├── rbac_simple.reap
│   │   ├── abac_clearance.reap
│   │   ├── multilayer.reap
│   │   └── rebac_social.reap
│   ├── dsl/                     # DSL feature policies
│   │   ├── string_operations.reap
│   │   ├── math_validation.reap
│   │   ├── regex_validation.reap
│   │   ├── time_based.reap
│   │   ├── collection_ops.reap
│   │   └── comprehensions.reap
│   └── regression/              # Regression test policies
│       └── issue_*.reap
├── data/
│   ├── small/                   # Quick test data (100 entities)
│   │   ├── users.json
│   │   ├── resources.json
│   │   └── relationships.json
│   ├── medium/                  # Standard test data (1000 entities)
│   │   └── ...
│   ├── large/                   # Scale test data (10000 entities)
│   │   └── ...
│   └── generated/               # Auto-generated (100k+)
│       └── ...
├── expectations/                # Expected results
│   ├── rbac_results.json
│   ├── abac_results.json
│   └── ...
└── generators/                  # Data generators
    ├── generate_users.rs
    ├── generate_resources.rs
    └── generate_relationships.rs
```

### Data Generator CLI

```bash
# Generate test data
cargo run --bin generate-test-data -- \
  --users 1000 \
  --resources 5000 \
  --output test-fixtures/data/medium/

# Generate for specific scenario
cargo run --bin generate-test-data -- \
  --scenario abac \
  --scale large \
  --output test-fixtures/data/large/
```

---

## Phase 5: CI/CD Integration

### Test Pipeline

```yaml
# .github/workflows/test.yml

name: Test Suite

on: [push, pull_request]

jobs:
  unit-tests:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Run unit tests
        run: cargo test --workspace --lib

  bdd-tests:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Run BDD tests
        run: cargo test --workspace --test '*bdd*' --test 'gherkin*'

  integration-tests:
    runs-on: ubuntu-latest
    needs: [unit-tests]
    steps:
      - uses: actions/checkout@v4
      - name: Start services
        run: docker compose --profile engine up -d
      - name: Wait for services
        run: ./scripts/wait-for-services.sh
      - name: Run integration tests
        run: cargo test -p reaper-integration

  opa-comparison:
    runs-on: ubuntu-latest
    if: github.event_name == 'schedule' || contains(github.event.head_commit.message, '[opa-compare]')
    steps:
      - uses: actions/checkout@v4
      - name: Start OPA
        run: docker run -d -p 8181:8181 openpolicyagent/opa run --server
      - name: Start Reaper
        run: docker compose --profile engine up -d
      - name: Run comparison
        run: cargo run --bin opa-comparison -- --scenario all

  performance-regression:
    runs-on: ubuntu-latest
    needs: [unit-tests]
    steps:
      - uses: actions/checkout@v4
      - name: Run benchmarks
        run: cargo bench --workspace -- --save-baseline ci
      - name: Compare to main
        run: cargo bench --workspace -- --baseline main
```

---

## Summary: Action Items

### Immediate (Week 1-2)
1. [ ] Create new BDD feature files for DSL coverage
2. [ ] Implement missing step definitions
3. [ ] Consolidate test data into `test-fixtures/`

### Short-term (Week 3-4)
4. [ ] Build integration test framework
5. [ ] Create YAML-based test suite format
6. [ ] Add integration tests for all core scenarios

### Medium-term (Month 2)
7. [ ] Implement OPA comparison runner
8. [ ] Add semantic equivalence tests for key scenarios
9. [ ] Integrate into CI/CD pipeline

### Long-term (Ongoing)
10. [ ] Maintain test coverage as features evolve
11. [ ] Add regression tests for reported issues
12. [ ] Performance baseline tracking

---

## Appendix: Existing Test File Reference

### BDD Feature Files
| File | Scenarios | Coverage |
|------|-----------|----------|
| `rbac.feature` | 6 | Role-based access |
| `abac.feature` | 12 | Attribute-based access |
| `multilayer.feature` | 8 | Layered policies |
| `string_operations.feature` | 10 | String functions |
| `math_operations.feature` | 8 | Numeric operations |
| `regex_validation.feature` | 8 | Pattern matching |
| `time_based_policies.feature` | 12 | Time operations |
| `collection_operations.feature` | 10 | Array/set ops |
| `comprehensions.feature` | 8 | Comprehensions |
| `json_operations.feature` | 6 | JSON handling |
| `type_checking.feature` | 8 | Type validation |
| `nested_comprehensions.feature` | 6 | Nested iteration |
| `advanced_collections.feature` | 8 | Complex collections |
| `conditional_expressions.feature` | 10 | Conditionals |

### Integration Test Files
| File | Purpose |
|------|---------|
| `e2e_tests.rs` | Full deployment flow |
| `integration_tests.rs` | Management service |
| `comprehension_tests.rs` | Comprehension edge cases |
| `compiled_evaluator_tests.rs` | Compiled evaluation |
| `concurrent_hotswap_tests.rs` | Concurrent operations |
| `security_tests.rs` | Security scenarios |
| `memory_resource_tests.rs` | Memory limits |
| `large_scale_tests.rs` | Scale testing |
