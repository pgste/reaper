@multilayer @comprehensive
Feature: Multilayer Policy Validation
  Test comprehensive enterprise policy combining RBAC, ABAC, and ReBAC layers.
  Validates layer priority, conflict resolution, and complex access patterns.

  Background:
    Given the policy file "examples/policies/multilayer.reap"
    And the data file "../../test-data/multilayer-test-data.json"

  # ==========================================
  # Layer 1: Deny Rules (Highest Priority)
  # ==========================================

  @layer1 @deny_first @suspended
  Scenario: Suspended users are blocked regardless of other permissions
    Given a principal "user_20"
    When they perform action "read" on resource "resource_100"
    Then the decision should be "deny"

  @layer1 @deny_first @suspended
  Scenario: Suspended executive is still blocked
    Given a principal "user_20"
    When they perform action "read" on resource "resource_501"
    Then the decision should be "deny"

  # ==========================================
  # Layer 2: Admin Override
  # ==========================================

  @layer2 @rbac @admin
  Scenario: Admin override works across all layers
    Given a principal "user_0"
    When they perform action "read" on resource "resource_501"
    Then the decision should be "allow"

  @layer2 @rbac @admin
  Scenario: Admin can access any department
    Given a principal "user_0"
    When they perform action "read" on resource "resource_100"
    Then the decision should be "allow"

  @layer2 @rbac @admin
  Scenario: Admin can modify any resource
    Given a principal "user_0"
    When they perform action "write" on resource "resource_300"
    Then the decision should be "allow"

  # ==========================================
  # Layer 3: ReBAC + ABAC - Ownership with Clearance
  # ==========================================

  @layer3 @rebac @ownership
  Scenario: Owners with high clearance can access their docs
    Given a principal "user_101"
    When they perform action "read" on resource "resource_101"
    Then the decision should be "allow"

  # ==========================================
  # Layer 5: ABAC + ReBAC - Department Clearance
  # ==========================================

  @layer5 @abac @department
  Scenario: Department access with clearance match
    Given a principal "user_1"
    When they perform action "read" on resource "resource_201"
    Then the decision should be "allow"

  # ==========================================
  # Layer 9: ABAC - Public Resources
  # ==========================================

  @layer9 @abac @public
  Scenario: Active users can access public resources
    Given a principal "user_501"
    When they perform action "read" on resource "resource_4"
    Then the decision should be "allow"

  # ==========================================
  # Layer Interaction Tests
  # ==========================================

  @interactions @deny_cascade
  Scenario: Deny layer blocks despite allow in lower layers
    Given a principal "user_20"
    When they perform action "read" on resource "resource_4"
    Then the decision should be "deny"

  @interactions @layer_priority
  Scenario Outline: Layer priority is respected
    Given a principal "<user>"
    When they perform action "read" on resource "<resource>"
    Then the decision should be "<decision>"

    Examples: Suspended blocks all (Layer 1)
      | user     | resource      | decision |
      | user_20  | resource_1    | deny     |
      | user_20  | resource_4    | deny     |
      | user_20  | resource_101  | deny     |

    Examples: Admin overrides (Layer 2)
      | user    | resource      | decision |
      | user_0  | resource_100  | allow    |
      | user_0  | resource_500  | allow    |

    Examples: Ownership (Layer 3)
      | user     | resource      | decision |
      | user_101 | resource_101  | allow    |

  # ==========================================
  # Complex Multi-Layer Scenarios
  # ==========================================

  @complex
  Scenario Outline: Complex multi-layer access decisions
    Given a principal "<user>"
    When they perform action "<action>" on resource "<resource>"
    Then the decision should be "<decision>"

    Examples: RBAC + ABAC + ReBAC combinations
      | user            | action | resource           | decision |
      | user_0          | read   | resource_101       | allow    |
      | user_20         | read   | resource_201       | deny     |
      | user_101        | read   | resource_101       | allow    |
      | user_1          | write  | resource_201       | allow    |

  # ==========================================
  # Action Type Tests
  # ==========================================

  @actions
  Scenario Outline: Different actions are evaluated correctly
    Given a principal "<user>"
    When they perform action "<action>" on resource "<resource>"
    Then the decision should be "<decision>"

    Examples:
      | user    | action  | resource     | decision |
      | user_0  | read    | resource_101 | allow    |
      | user_0  | write   | resource_101 | allow    |
      | user_0  | delete  | resource_101 | allow    |
      | user_1  | read    | resource_201 | allow    |

  # ==========================================
  # Performance Tests
  # ==========================================

  @performance
  Scenario: Multilayer policy maintains reasonable performance
    Given a principal "user_0"
    When they perform 1000 evaluations on random resources
    Then the average evaluation time should be less than 100 microseconds

  @performance @stress
  Scenario: High volume multilayer evaluations
    Given a principal "user_1"
    When they perform 5000 evaluations on random resources
    Then the average evaluation time should be less than 100 microseconds

  # ==========================================
  # Regression Tests
  # ==========================================

  @regression
  Scenario Outline: Critical multilayer scenarios work correctly
    Given a principal "<user>"
    When they perform action "<action>" on resource "<resource>"
    Then the decision should be "<decision>"

    Examples: Admin always works
      | user   | action | resource      | decision |
      | user_0 | read   | resource_1    | allow    |
      | user_0 | read   | resource_100  | allow    |
      | user_0 | read   | resource_500  | allow    |

    Examples: Suspended always blocked
      | user    | action | resource     | decision |
      | user_20 | read   | resource_1   | deny     |
      | user_20 | write  | resource_1   | deny     |
      | user_20 | read   | resource_4   | deny     |

    Examples: Ownership works
      | user     | action | resource     | decision |
      | user_101 | read   | resource_101 | allow    |
