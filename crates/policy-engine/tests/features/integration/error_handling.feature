@error-handling @edge-cases
Feature: Error Handling and Edge Cases
  Test policy behavior under error conditions and edge cases

  Background:
    Given the policy file "examples/policies/error_handling_policy.reap"
    And the data file "../../test-data/error-handling-test-data.json"

  # Missing Attribute Scenarios
  @missing-attribute @negative
  Scenario: Missing user attribute results in deny
    Given a principal "user_minimal"
    When they perform action "check_department" on resource "dept_resource"
    Then the decision should be "deny"

  # Null Value Handling
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

  # Empty Collection Scenarios
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

  # Deeply Nested Data
  @nested-data @positive
  Scenario: Deep attribute access succeeds
    Given a principal "user_deep_nested"
    When they perform action "deep_access" on resource "nested_resource"
    Then the decision should be "allow"

  @nested-data @negative
  Scenario: Partial nested path returns deny
    Given a principal "user_partial_nested"
    When they perform action "deep_access" on resource "nested_resource"
    Then the decision should be "deny"

  # Large Collection Performance
  @large-collection @positive
  Scenario: Large permissions array evaluates correctly
    Given a principal "user_many_permissions"
    When they perform action "check" on resource "perf_resource"
    Then the decision should be "allow"

  # Boolean Edge Cases
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

  # Whitespace Handling
  @whitespace @positive
  Scenario: Trimmed whitespace matches
    Given a principal "user_whitespace_name"
    When they perform action "name_check" on resource "name_resource"
    Then the decision should be "allow"

  # Unicode Handling
  @unicode @positive
  Scenario: Unicode characters in attributes
    Given a principal "user_unicode_name"
    When they perform action "unicode_check" on resource "unicode_resource"
    Then the decision should be "allow"

  # Scenario Outline for Edge Cases
  @edge-case-matrix
  Scenario Outline: Edge case matrix
    Given a principal "<principal>"
    When they perform action "<action>" on resource "<resource>"
    Then the decision should be "<decision>"

    Examples:
      | principal              | action           | resource         | decision |
      | user_minimal           | check_department | dept_resource    | deny     |
      | user_null_role         | admin_action     | admin_resource   | deny     |
      | user_empty_permissions | any_action       | perm_resource    | deny     |
      | user_deep_nested       | deep_access      | nested_resource  | allow    |
      | user_active_true       | active_check     | active_resource  | allow    |
      | user_active_false      | active_check     | active_resource  | deny     |
      | user_unicode_name      | unicode_check    | unicode_resource | allow    |
