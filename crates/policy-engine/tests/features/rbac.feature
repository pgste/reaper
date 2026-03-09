@rbac @smoke
Feature: RBAC Policy Validation
  Test the RBAC policy with various user roles and resource access scenarios
  including role hierarchies, edge cases, and comprehensive access patterns

  Background:
    Given the policy file "examples/policies/rbac.reap"
    And the data file "../../test-data/rbac-test-data.json"

  # ==========================================
  # Admin Role Scenarios
  # ==========================================

  @admin @positive
  Scenario: Admin has full access to all resources
    Given a principal "user_0"
    When they perform action "read" on resource "resource_100"
    Then the decision should be "allow"

  @admin @positive
  Scenario: Admin can write to any resource
    Given a principal "user_0"
    When they perform action "write" on resource "resource_200"
    Then the decision should be "allow"

  @admin @positive
  Scenario: Admin can delete any resource
    Given a principal "user_0"
    When they perform action "delete" on resource "resource_300"
    Then the decision should be "allow"

  @admin @positive
  Scenario: Admin can access resources they don't own
    Given a principal "user_0"
    When they perform action "read" on resource "resource_999"
    Then the decision should be "allow"

  @admin @positive
  Scenario: Any admin can access any resource
    Given a principal "user_50"
    When they perform action "read" on resource "resource_100"
    Then the decision should be "allow"

  @admin @positive
  Scenario: Another admin can access any resource
    Given a principal "user_700"
    When they perform action "read" on resource "resource_100"
    Then the decision should be "allow"

  # ==========================================
  # Manager Role Scenarios
  # ==========================================

  @manager @positive
  Scenario: Manager can read report resources
    Given a principal "user_1"
    When they perform action "read" on resource "resource_100"
    Then the decision should be "allow"

  @manager @negative
  Scenario: Manager cannot access non-report documents
    Given a principal "user_1"
    When they perform action "read" on resource "resource_101"
    Then the decision should be "deny"

  @manager @positive
  Scenario: Manager can access any report type resource
    Given a principal "user_2"
    When they perform action "read" on resource "resource_100"
    Then the decision should be "allow"

  # ==========================================
  # Regular User Role Scenarios
  # ==========================================

  @user @negative
  Scenario: Regular user cannot access others' resources
    Given a principal "user_3"
    When they perform action "read" on resource "resource_100"
    Then the decision should be "deny"

  @user @negative
  Scenario: Regular user cannot access others' documents
    Given a principal "user_3"
    When they perform action "read" on resource "resource_101"
    Then the decision should be "deny"

  @user @negative
  Scenario: Regular user cannot access protected resources
    Given a principal "user_4"
    When they perform action "read" on resource "resource_200"
    Then the decision should be "deny"

  # ==========================================
  # Resource Ownership Scenarios
  # ==========================================

  @ownership @positive
  Scenario: User can access their own resources
    Given a principal "user_100"
    When they perform action "read" on resource "resource_100"
    Then the decision should be "allow"

  @ownership @positive
  Scenario: Owner can write to their own resources
    Given a principal "user_100"
    When they perform action "write" on resource "resource_100"
    Then the decision should be "allow"

  @ownership @positive
  Scenario: Owner can delete their own resources
    Given a principal "user_100"
    When they perform action "delete" on resource "resource_100"
    Then the decision should be "allow"

  @ownership @negative
  Scenario: User cannot access resources owned by others
    Given a principal "user_3"
    When they perform action "read" on resource "resource_101"
    Then the decision should be "deny"

  # ==========================================
  # Edge Cases and Boundary Conditions
  # ==========================================

  @edge_case @negative
  Scenario: User with no role is denied protected access
    Given a principal "user_3"
    When they perform action "read" on resource "resource_101"
    Then the decision should be "deny"

  @edge_case @negative
  Scenario: Unknown resource evaluation handles gracefully
    Given a principal "user_0"
    When they perform action "read" on resource "unknown_resource"
    Then the decision should be "allow"

  # ==========================================
  # Role Combination Table Tests
  # ==========================================

  @combinations
  Scenario Outline: Role-based access combinations
    Given a principal "<user>"
    When they perform action "<action>" on resource "<resource>"
    Then the decision should be "<decision>"

    Examples: Admin access (full permissions)
      | user    | action | resource      | decision |
      | user_0  | read   | resource_100  | allow    |
      | user_0  | write  | resource_100  | allow    |
      | user_0  | delete | resource_100  | allow    |
      | user_0  | admin  | resource_100  | allow    |
      | user_50 | read   | resource_100  | allow    |
      | user_700| read   | resource_100  | allow    |

    Examples: Manager access (reports only)
      | user    | action | resource      | decision |
      | user_1  | read   | resource_100  | allow    |
      | user_1  | read   | resource_101  | deny     |
      | user_2  | read   | resource_100  | allow    |

    Examples: Regular user access (owned resources only)
      | user     | action | resource      | decision |
      | user_100 | read   | resource_100  | allow    |
      | user_100 | write  | resource_100  | allow    |
      | user_3   | read   | resource_100  | deny     |
      | user_3   | read   | resource_101  | deny     |

  # ==========================================
  # Action Type Variations
  # ==========================================

  @actions
  Scenario Outline: Different action types are handled correctly
    Given a principal "<user>"
    When they perform action "<action>" on resource "<resource>"
    Then the decision should be "<decision>"

    Examples:
      | user    | action  | resource     | decision |
      | user_0  | read    | resource_100 | allow    |
      | user_0  | write   | resource_100 | allow    |
      | user_0  | delete  | resource_100 | allow    |
      | user_0  | execute | resource_100 | allow    |
      | user_0  | share   | resource_100 | allow    |
      | user_100| read    | resource_100 | allow    |
      | user_100| archive | resource_100 | allow    |

  # ==========================================
  # Multiple Users Same Resource (Correct Behavior)
  # ==========================================

  @concurrent
  Scenario Outline: Multiple users accessing same report resource
    Given a principal "<user>"
    When they perform action "read" on resource "resource_100"
    Then the decision should be "<decision>"

    Examples:
      | user     | decision |
      | user_0   | allow    |
      | user_1   | allow    |
      | user_50  | allow    |
      | user_700 | allow    |
      | user_3   | deny     |
      | user_4   | deny     |

  # ==========================================
  # Performance Tests
  # ==========================================

  @performance
  Scenario: Policy evaluates with good performance
    Given a principal "user_0"
    When they perform 1000 evaluations on random resources
    Then the average evaluation time should be less than 50 microseconds

  @performance @stress
  Scenario: Policy handles high volume evaluations
    Given a principal "user_0"
    When they perform 5000 evaluations on random resources
    Then the average evaluation time should be less than 50 microseconds
