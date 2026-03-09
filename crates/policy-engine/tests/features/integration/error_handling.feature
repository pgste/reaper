@error-handling @edge-cases
Feature: Error Handling and Edge Cases
  Test policy behavior under error conditions and edge cases including
  null values, missing attributes, type mismatches, and boundary conditions.

  Background:
    Given the policy file "examples/policies/error_handling_policy.reap"
    And the data file "../../test-data/error-handling-test-data.json"

  # ==========================================
  # Missing Attribute Scenarios
  # ==========================================

  @missing-attribute @negative
  Scenario: Missing user attribute results in deny
    Given a principal "user_minimal"
    When they perform action "check_department" on resource "dept_resource"
    Then the decision should be "deny"

  @missing-attribute @negative
  Scenario: User with only name cannot access any protected resource
    Given a principal "user_minimal"
    When they perform action "admin_action" on resource "admin_resource"
    Then the decision should be "deny"

  @missing-attribute @negative
  Scenario: User with level but no department is denied
    Given a principal "user_with_level"
    When they perform action "check_department" on resource "dept_resource"
    Then the decision should be "deny"

  # ==========================================
  # Null Value Handling
  # ==========================================

  @null-value @negative
  Scenario: Null role value treated as missing
    Given a principal "user_null_role"
    When they perform action "admin_action" on resource "admin_resource"
    Then the decision should be "deny"

  @null-value @negative
  Scenario: Null permissions array treated as missing
    Given a principal "user_null_permissions"
    When they perform action "check_perms" on resource "perm_resource"
    Then the decision should be "deny"

  @null-value @negative
  Scenario: Null value does not match any string
    Given a principal "user_null_role"
    When they perform action "role_check" on resource "role_resource"
    Then the decision should be "deny"

  @null-value
  Scenario Outline: Null values are consistently treated as missing
    Given a principal "<user>"
    When they perform action "<action>" on resource "<resource>"
    Then the decision should be "deny"

    Examples:
      | user                  | action           | resource        |
      | user_null_role        | admin_action     | admin_resource  |
      | user_null_permissions | check_perms      | perm_resource   |
      | user_null_role        | role_check       | role_resource   |

  # ==========================================
  # Empty Collection Scenarios
  # ==========================================

  @empty-collection @negative
  Scenario: Empty permissions array denies access
    Given a principal "user_empty_permissions"
    When they perform action "any_action" on resource "perm_resource"
    Then the decision should be "deny"

  @empty-collection @negative
  Scenario: Empty roles array denies role check
    Given a principal "user_empty_roles"
    When they perform action "role_check" on resource "role_resource"
    Then the decision should be "deny"

  @empty-collection
  Scenario Outline: Empty collections handled correctly
    Given a principal "<user>"
    When they perform action "<action>" on resource "<resource>"
    Then the decision should be "deny"

    Examples:
      | user                   | action      | resource       |
      | user_empty_permissions | any_action  | perm_resource  |
      | user_empty_roles       | role_check  | role_resource  |

  # ==========================================
  # Deeply Nested Data Access
  # ==========================================

  @nested-data @positive
  Scenario: Deep attribute access succeeds with complete path
    Given a principal "user_deep_nested"
    When they perform action "deep_access" on resource "nested_resource"
    Then the decision should be "allow"

  @nested-data @negative
  Scenario: Partial nested path returns deny
    Given a principal "user_partial_nested"
    When they perform action "deep_access" on resource "nested_resource"
    Then the decision should be "deny"

  @nested-data @negative
  Scenario: Missing intermediate nested object denies
    Given a principal "user_minimal"
    When they perform action "deep_access" on resource "nested_resource"
    Then the decision should be "deny"

  # ==========================================
  # Large Collection Performance
  # ==========================================

  @large-collection @positive
  Scenario: Large permissions array evaluates correctly
    Given a principal "user_many_permissions"
    When they perform action "check" on resource "perf_resource"
    Then the decision should be "allow"

  @large-collection @performance
  Scenario: Large collection lookup maintains performance
    Given a principal "user_many_permissions"
    When they perform 100 evaluations on resource "perf_resource"
    Then the average evaluation time should be less than 100 microseconds

  # ==========================================
  # Boolean Edge Cases
  # ==========================================

  @boolean @positive
  Scenario: Boolean true allows access
    Given a principal "user_active_true"
    When they perform action "active_check" on resource "active_resource"
    Then the decision should be "allow"

  @boolean @negative
  Scenario: Boolean false denies access
    Given a principal "user_active_false"
    When they perform action "active_check" on resource "active_resource"
    Then the decision should be "deny"

  @boolean @negative
  Scenario: String "true" is not boolean true
    Given a principal "user_active_string"
    When they perform action "strict_bool_check" on resource "bool_resource"
    Then the decision should be "deny"

  @boolean
  Scenario Outline: Boolean values are strictly typed
    Given a principal "<user>"
    When they perform action "<action>" on resource "<resource>"
    Then the decision should be "<decision>"

    Examples:
      | user               | action            | resource         | decision |
      | user_active_true   | active_check      | active_resource  | allow    |
      | user_active_false  | active_check      | active_resource  | deny     |
      | user_active_string | strict_bool_check | bool_resource    | deny     |
      | user_active_string | active_check      | active_resource  | deny     |

  # ==========================================
  # Whitespace Handling
  # ==========================================

  @whitespace @positive
  Scenario: Trimmed comparison succeeds for whitespace-padded value
    Given a principal "user_whitespace_name"
    When they perform action "name_check" on resource "name_resource"
    Then the decision should be "allow"

  @whitespace @positive
  Scenario: Exact match with whitespace succeeds
    Given a principal "user_whitespace_name"
    When they perform action "exact_match" on resource "exact_resource"
    Then the decision should be "allow"

  # ==========================================
  # Unicode Character Handling
  # ==========================================

  @unicode @positive
  Scenario: Unicode characters in name are supported
    Given a principal "user_unicode_name"
    When they perform action "unicode_check" on resource "unicode_resource"
    Then the decision should be "allow"

  @unicode @positive
  Scenario: Mixed Unicode and ASCII characters work
    Given a principal "user_unicode_name"
    When they perform action "unicode_check" on resource "unicode_resource"
    Then the decision should be "allow"

  # ==========================================
  # Numeric Comparison Scenarios
  # ==========================================

  @numeric @positive
  Scenario: Numeric comparison works correctly
    Given a principal "user_active_true"
    When they perform action "active_check" on resource "active_resource"
    Then the decision should be "allow"

  # ==========================================
  # Entity Validation
  # ==========================================

  @entity-validation @negative
  Scenario: User with minimal attributes denied protected access
    Given a principal "user_minimal"
    When they perform action "admin_action" on resource "admin_resource"
    Then the decision should be "deny"

  # ==========================================
  # Comprehensive Edge Case Matrix
  # ==========================================

  @comprehensive
  Scenario Outline: Comprehensive edge case coverage
    Given a principal "<user>"
    When they perform action "<action>" on resource "<resource>"
    Then the decision should be "<decision>"

    Examples: Missing attributes
      | user             | action           | resource        | decision |
      | user_minimal     | check_department | dept_resource   | deny     |
      | user_minimal     | admin_action     | admin_resource  | deny     |

    Examples: Null values
      | user                  | action       | resource       | decision |
      | user_null_role        | admin_action | admin_resource | deny     |
      | user_null_permissions | check_perms  | perm_resource  | deny     |

    Examples: Empty collections
      | user                   | action     | resource      | decision |
      | user_empty_permissions | any_action | perm_resource | deny     |
      | user_empty_roles       | role_check | role_resource | deny     |

    Examples: Nested data
      | user                | action      | resource        | decision |
      | user_deep_nested    | deep_access | nested_resource | allow    |
      | user_partial_nested | deep_access | nested_resource | deny     |

    Examples: Boolean types
      | user               | action       | resource        | decision |
      | user_active_true   | active_check | active_resource | allow    |
      | user_active_false  | active_check | active_resource | deny     |

  # ==========================================
  # Performance Tests
  # ==========================================

  @performance
  Scenario: Error handling maintains performance
    Given a principal "user_active_true"
    When they perform 1000 evaluations on resource "active_resource"
    Then the average evaluation time should be less than 100 microseconds
