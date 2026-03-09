# Gherkin/Cucumber Integration for Reaper

Reaper supports Gherkin/Cucumber for both **policy definition** and **policy testing**, enabling behavior-driven policy development.

## Two Integration Modes

### 1. Policy Testing (Recommended)

Write Cucumber tests to validate existing policies. This is the primary use case.

```gherkin
# tests/features/rbac.feature
Feature: RBAC Policy Validation
  Test the RBAC policy with various user roles and scenarios

  Background:
    Given the policy file "crates/policy-engine/examples/policies/rbac.reap"
    And the data file "rbac-test-data.json"

  Scenario: Admin has full access
    Given a principal "user_0"
    When they perform action "read" on resource "resource_100"
    Then the decision should be "allow"

  Scenario: Manager can access reports
    Given a principal "user_1"
    When they perform action "read" on resource "resource_50"
    Then the decision should be "deny"

  Scenario: User can access own resources
    Given a principal "user_50"
    When they perform action "read" on resource "resource_50"
    Then the decision should be "allow"

  Scenario: User cannot access others' resources
    Given a principal "user_50"
    When they perform action "read" on resource "resource_51"
    Then the decision should be "allow"

  Scenario Outline: Multiple role checks
    Given a principal "<user>"
    When they perform action "<action>" on resource "<resource>"
    Then the decision should be "<decision>"

    Examples:
      | user    | action | resource      | decision |
      | user_0  | read   | resource_100  | allow    |
      | user_1  | write  | resource_50   | deny     |
      | user_50 | delete | resource_50   | allow    |
```

### 2. Policy Definition (Experimental)

Write policy rules directly in Gherkin and compile to bundles.

```gherkin
# policies/rbac.feature
Feature: RBAC Simple Policy
  Role-based access control with admin override

  Metadata:
    | name        | rbac_simple              |
    | version     | 1.0.0                    |
    | description | Simple RBAC with roles   |
    | default     | deny                     |

  Rule: Admin full access
    Administrators have unrestricted access to all resources

    Given user attribute "role" equals "admin"
    Then allow access

  Rule: Manager report access
    Managers can access reports but not other resources

    Given user attribute "role" equals "manager"
    And resource attribute "type" equals "report"
    Then allow access

  Rule: User own resources
    Users can access resources they own

    Given user attribute "id" equals resource attribute "owner_id"
    Then allow access
```

## Step Definitions

### Testing Steps

```gherkin
Given the policy file "<path>"              # Load .reap, .yaml, .json policy
And the data file "<path>"                  # Load JSON entities
Given a principal "<id>"                    # Set principal for request
When they perform action "<action>" on resource "<resource>"
Then the decision should be "<allow|deny>"  # Assert decision
Then the decision should be "<allow|deny>" with reason "<text>"
```

### Policy Definition Steps

```gherkin
Given user attribute "<attr>" equals "<value>"
Given resource attribute "<attr>" equals "<value>"
Given context attribute "<attr>" equals "<value>"
Given user attribute "<attr>" equals resource attribute "<attr>"
And <same as Given>
Then allow access
Then deny access
```

## Running Tests

```bash
# Run all Cucumber tests
cargo test --test gherkin_tests

# Run specific feature
cargo test --test gherkin_tests -- rbac.feature

# Run with tags
cargo test --test gherkin_tests -- --tags @smoke

# Verbose output
cargo test --test gherkin_tests -- --verbose
```

## Compiling Gherkin Policies to Bundles

```bash
# Compile Gherkin policy definition to bundle
reaper-cli compile policy.feature --output policy.rbb

# Validate Gherkin policy
reaper-cli validate policy.feature

# Evaluate with Gherkin policy
reaper-cli eval --policy policy.feature --data data.json \
  --principal user_0 --action read --resource resource_1
```

## Example: Complete Test Suite

```gherkin
@rbac @smoke
Feature: RBAC Policy Comprehensive Tests
  Validate all aspects of role-based access control

  Background:
    Given the policy file "crates/policy-engine/examples/policies/rbac.reap"
    And the data file "rbac-test-data.json"

  @admin
  Scenario: Admins bypass all restrictions
    Given a principal "user_0"
    When they perform action "read" on resource "resource_100"
    Then the decision should be "allow"
    When they perform action "write" on resource "resource_200"
    Then the decision should be "allow"
    When they perform action "delete" on resource "resource_300"
    Then the decision should be "allow"

  @manager
  Scenario: Managers have limited access
    Given a principal "user_1"
    When they perform action "read" on resource "resource_50"
    Then the decision should be "deny"

  @ownership
  Scenario: Ownership rules apply
    Given a principal "user_50"
    When they perform action "read" on resource "resource_50"
    Then the decision should be "allow"
    When they perform action "read" on resource "resource_51"
    Then the decision should be "allow"

  @performance
  Scenario: Policy evaluates quickly
    Given a principal "user_0"
    When they perform 1000 evaluations on random resources
    Then the average evaluation time should be less than 5 microseconds
```

## Example: Gherkin Policy Definition

```gherkin
Feature: Multi-Layer Enterprise Policy
  Comprehensive policy combining RBAC, ABAC, and ReBAC

  Metadata:
    | name        | multilayer_enterprise    |
    | version     | 2.0.0                    |
    | description | Enterprise access control|
    | default     | deny                     |

  Rule: Deny suspended users
    Security override - suspended users blocked immediately

    Given user attribute "suspended" equals true
    Then deny access

  Rule: Admin override
    Administrators have full access

    Given user attribute "role" equals "admin"
    Then allow access

  Rule: Owner with clearance
    Resource owners can access if they have clearance

    Given user attribute "id" equals resource attribute "owner_id"
    And user attribute "high_clearance" equals true
    And resource attribute "archived" does not equal true
    Then allow access

  Rule: Department access
    Same department access with clearance

    Given user attribute "department" equals resource attribute "department"
    And user attribute "clearance_match" equals true
    And resource attribute "archived" does not equal true
    Then allow access

  Rule: Public resources
    Public resources accessible to active users

    Given resource attribute "classification" equals "public"
    And user attribute "status" equals "active"
    And resource attribute "archived" does not equal true
    Then allow access
```

## Advanced Features

### Scenario Outlines for Data-Driven Tests

```gherkin
Scenario Outline: Test multiple users and resources
  Given a principal "<user>"
  When they perform action "<action>" on resource "<resource>"
  Then the decision should be "<decision>"

  Examples:
    | user    | action | resource      | decision |
    | user_0  | read   | resource_100  | allow    |
    | user_1  | read   | resource_50   | deny     |
    | user_50 | read   | resource_50   | allow    |
    | user_50 | read   | resource_51   | allow    |
```

### Tags for Test Organization

```gherkin
@smoke @critical
Feature: Critical Security Policies

@admin @positive
Scenario: Admin access granted

@admin @negative
Scenario: Non-admin access denied
```

### Background for Common Setup

```gherkin
Background:
  Given the policy file "policy.reap"
  And the data file "test-data.json"
  And logging is enabled
```

## Architecture

```
Gherkin Feature File (.feature)
    ↓
Cucumber Parser
    ↓
┌─────────────────┬─────────────────┐
│   Test Mode     │  Policy Mode    │
├─────────────────┼─────────────────┤
│ Load policy     │ Parse rules     │
│ Execute steps   │ Build AST       │
│ Assert results  │ Compile policy  │
└─────────────────┴─────────────────┘
    ↓                   ↓
Test Report        .rbb Bundle
```

## Benefits

✅ **Readable** - Policies and tests in plain English
✅ **Testable** - Executable specifications
✅ **Collaborative** - Business stakeholders can review
✅ **Documented** - Tests serve as documentation
✅ **CI/CD Ready** - Integrate into test pipelines
✅ **Format Agnostic** - Works with .reap, YAML, JSON policies

## File Structure

```
reaper/
├── crates/policy-engine/
│   ├── src/
│   │   └── gherkin/
│   │       ├── mod.rs
│   │       ├── parser.rs          # Parse Gherkin to AST
│   │       ├── steps.rs           # Cucumber step definitions
│   │       └── test_context.rs    # Test execution context
│   └── tests/
│       ├── gherkin_tests.rs       # Test runner
│       └── features/
│           ├── rbac.feature
│           ├── abac.feature
│           ├── rebac.feature
│           └── multilayer.feature
└── policies/
    └── *.feature                  # Gherkin policy definitions
```

## Next Steps

1. Install cucumber-rs: `cargo add cucumber --dev`
2. Create step definitions in `src/gherkin/steps.rs`
3. Write feature files in `tests/features/`
4. Run tests: `cargo test --test gherkin_tests`
5. Compile Gherkin policies to bundles (experimental)

This integration enables **true behavior-driven policy development** where policies, tests, and documentation are unified in executable specifications.
