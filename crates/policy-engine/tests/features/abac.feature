@abac
Feature: ABAC Policy Validation
  Test attribute-based access control with clearances, departments, and
  document classification levels. Covers complex attribute combinations
  and edge cases.

  Background:
    Given the policy file "examples/policies/abac.reap"
    And the data file "../../test-data/abac-test-data.json"

  # ==========================================
  # Suspended User Scenarios (Deny First)
  # ==========================================

  @suspended @negative @deny_first
  Scenario: Suspended users are blocked regardless of clearance
    Given a principal "user_20"
    When they perform action "read" on resource "resource_100"
    Then the decision should be "deny"

  @suspended @negative
  Scenario: Suspended executive is blocked
    Given a principal "user_20"
    When they perform action "read" on resource "resource_1"
    Then the decision should be "deny"

  @suspended @negative
  Scenario: Suspended user cannot access own document
    Given a principal "user_20"
    When they perform action "read" on resource "resource_20"
    Then the decision should be "deny"

  # ==========================================
  # Clearance Level Scenarios
  # ==========================================

  @clearance @positive
  Scenario: High clearance users can access confidential docs
    Given a principal "user_0"
    When they perform action "read" on resource "resource_100"
    Then the decision should be "allow"

  @clearance @positive
  Scenario: User with matching clearance can access internal docs
    Given a principal "user_1"
    When they perform action "read" on resource "resource_1"
    Then the decision should be "allow"

  @clearance @negative
  Scenario: Low clearance user cannot access confidential docs
    Given a principal "user_3"
    When they perform action "read" on resource "resource_100"
    Then the decision should be "deny"

  @clearance @negative
  Scenario: User cannot access secret docs even with high clearance
    Given a principal "user_1"
    When they perform action "read" on resource "resource_500"
    Then the decision should be "deny"

  # ==========================================
  # Department-Based Access
  # ==========================================

  @department @positive
  Scenario: Same department access with clearance match
    Given a principal "user_1"
    When they perform action "read" on resource "resource_201"
    Then the decision should be "allow"

  @department @negative
  Scenario: Cross-department access denied
    Given a principal "user_1"
    When they perform action "read" on resource "resource_2"
    Then the decision should be "deny"

  @department @negative
  Scenario: Engineering user cannot access HR docs without clearance match
    Given a principal "user_3"
    When they perform action "read" on resource "resource_2"
    Then the decision should be "deny"

  @department @positive
  Scenario: HR user can access HR confidential docs
    Given a principal "user_2"
    When they perform action "read" on resource "resource_2"
    Then the decision should be "allow"

  # ==========================================
  # Executive Access
  # ==========================================

  @executive @positive
  Scenario: Executives have broad access to non-archived docs
    Given a principal "user_0"
    When they perform action "read" on resource "resource_501"
    Then the decision should be "allow"

  @executive @positive
  Scenario: Executive can access non-archived docs
    Given a principal "user_4"
    When they perform action "read" on resource "resource_1"
    Then the decision should be "allow"

  @executive @positive
  Scenario: Executive owner can access their own documents
    Given a principal "user_0"
    When they perform action "read" on resource "resource_0"
    Then the decision should be "allow"

  # ==========================================
  # Document Ownership
  # ==========================================

  @ownership @positive
  Scenario: Document owners can access their documents
    Given a principal "user_50"
    When they perform action "read" on resource "resource_50"
    Then the decision should be "allow"

  @ownership @positive
  Scenario: Owner can access own doc regardless of clearance
    Given a principal "user_3"
    When they perform action "read" on resource "resource_3"
    Then the decision should be "allow"

  @ownership @negative
  Scenario: Non-owner cannot access others' private docs
    Given a principal "user_50"
    When they perform action "read" on resource "resource_51"
    Then the decision should be "deny"

  # ==========================================
  # Classification Levels
  # ==========================================

  @classification
  Scenario Outline: Classification-based access control
    Given a principal "<user>"
    When they perform action "read" on resource "<resource>"
    Then the decision should be "<decision>"

    Examples: Department and clearance based access
      | user    | resource     | decision |
      | user_1  | resource_1   | allow    |
      | user_2  | resource_2   | allow    |
      | user_3  | resource_100 | deny     |
      | user_0  | resource_100 | allow    |

  # ==========================================
  # Owner Access
  # ==========================================

  @ownership @positive
  Scenario: Owner can access their own document
    Given a principal "user_1"
    When they perform action "read" on resource "resource_1"
    Then the decision should be "allow"

  # ==========================================
  # Attribute Combinations
  # ==========================================

  @combinations
  Scenario Outline: Complex attribute combinations
    Given a principal "<user>"
    When they perform action "<action>" on resource "<resource>"
    Then the decision should be "<decision>"

    Examples: Clearance + Department + Classification
      | user    | action | resource     | decision |
      | user_1  | read   | resource_1   | allow    |
      | user_2  | read   | resource_2   | allow    |
      | user_0  | read   | resource_100 | allow    |
      | user_3  | read   | resource_100 | deny     |

    Examples: Executive overrides
      | user    | action | resource     | decision |
      | user_0  | read   | resource_501 | allow    |
      | user_4  | read   | resource_201 | allow    |

    Examples: Suspended blocks all
      | user     | action | resource     | decision |
      | user_20  | read   | resource_20  | deny     |
      | user_20  | read   | resource_1   | deny     |
      | user_20  | write  | resource_1   | deny     |

  # ==========================================
  # Edge Cases (using existing test data)
  # ==========================================

  @edge_case @negative
  Scenario: Low clearance user denied confidential access
    Given a principal "user_3"
    When they perform action "read" on resource "resource_100"
    Then the decision should be "deny"

  # ==========================================
  # Action Types
  # ==========================================

  @actions
  Scenario Outline: Different actions on documents
    Given a principal "<user>"
    When they perform action "<action>" on resource "<resource>"
    Then the decision should be "<decision>"

    Examples:
      | user    | action  | resource     | decision |
      | user_0  | read    | resource_501 | allow    |
      | user_0  | write   | resource_501 | allow    |
      | user_0  | delete  | resource_501 | allow    |
      | user_50 | read    | resource_50  | allow    |
      | user_50 | edit    | resource_50  | allow    |
      | user_50 | share   | resource_50  | allow    |

  # ==========================================
  # Multiple Attribute Match Tests
  # ==========================================

  @multi_attribute
  Scenario Outline: Multi-attribute access decisions
    Given a principal "<user>"
    When they perform action "read" on resource "<resource>"
    Then the decision should be "<decision>"

    Examples: Same department, varying clearance
      | user    | resource     | decision |
      | user_0  | resource_100 | allow    |
      | user_3  | resource_100 | deny     |
      | user_2  | resource_100 | deny     |

    Examples: Same clearance, varying department
      | user    | resource     | decision |
      | user_1  | resource_1   | allow    |
      | user_1  | resource_2   | deny     |
      | user_1  | resource_3   | deny     |

    Examples: Ownership overrides
      | user    | resource     | decision |
      | user_50 | resource_50  | allow    |
      | user_50 | resource_51  | deny     |

  # ==========================================
  # Performance Tests
  # ==========================================

  @performance
  Scenario: ABAC policy maintains sub-microsecond performance
    Given a principal "user_0"
    When they perform 1000 evaluations on random resources
    Then the average evaluation time should be less than 25 microseconds

  @performance @stress
  Scenario: ABAC handles high volume with complex attributes
    Given a principal "user_1"
    When they perform 5000 evaluations on random resources
    Then the average evaluation time should be less than 50 microseconds

  @performance @mixed
  Scenario: Mixed user types maintain performance
    Given principals with mixed clearance levels
    When they perform 1000 random evaluations
    Then the average evaluation time should be less than 30 microseconds
