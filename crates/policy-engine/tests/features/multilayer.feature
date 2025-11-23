@multilayer @comprehensive
Feature: Multilayer Policy Validation
  Test comprehensive enterprise policy combining RBAC, ABAC, and ReBAC

  Background:
    Given the policy file "crates/policy-engine/examples/policies/multilayer.reap"
    And the data file "multilayer-test-data.json"

  @rbac @admin
  Scenario: Admin override works across all layers
    Given a principal "user_0"
    When they perform action "read" on resource "resource_500"
    Then the decision should be "allow"

  @rbac @suspended @negative
  Scenario: Suspended users blocked despite other permissions
    Given a principal "user_20"
    When they perform action "read" on resource "resource_100"
    Then the decision should be "deny"

  @rebac @ownership
  Scenario: Owners with high clearance can access
    Given a principal "user_100"
    When they perform action "read" on resource "resource_100"
    Then the decision should be "allow"

  @abac @department
  Scenario: Department access with clearance match
    Given a principal "user_200"
    When they perform action "read" on resource "resource_200"
    Then the decision should be "allow"

  @abac @public
  Scenario: Active users can access public resources
    Given a principal "user_500"
    When they perform action "read" on resource "resource_500"
    Then the decision should be "allow"

  @performance
  Scenario: Multilayer policy maintains reasonable performance
    Given a principal "user_0"
    When they perform 1000 evaluations on random resources
    Then the average evaluation time should be less than 10 microseconds

  Scenario Outline: Various multilayer scenarios
    Given a principal "<user>"
    When they perform action "<action>" on resource "<resource>"
    Then the decision should be "<decision>"

    Examples:
      | user    | action | resource      | decision |
      | user_0  | read   | resource_100  | allow    |
      | user_20 | read   | resource_200  | deny     |
      | user_100| read   | resource_100  | allow    |
      | user_200| write  | resource_200  | allow    |
      | user_500| delete | resource_500  | allow    |
