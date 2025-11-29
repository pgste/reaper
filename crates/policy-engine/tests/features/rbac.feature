@rbac @smoke
Feature: RBAC Policy Validation
  Test the RBAC policy with various user roles and resource access scenarios

  Background:
    Given the policy file "examples/policies/rbac.reap"
    And the data file "../../test-data/rbac-test-data.json"

  @admin @positive
  Scenario: Admin has full access to all resources
    Given a principal "user_0"
    When they perform action "read" on resource "resource_100"
    Then the decision should be "allow"

  @manager @negative
  Scenario: Manager cannot access non-report resources
    Given a principal "user_1"
    When they perform action "read" on resource "resource_50"
    Then the decision should be "deny"

  @ownership @positive
  Scenario: User can access their own resources
    Given a principal "user_50"
    When they perform action "read" on resource "resource_50"
    Then the decision should be "allow"

  @ownership @positive
  Scenario: User can access other resources they own
    Given a principal "user_50"
    When they perform action "read" on resource "resource_51"
    Then the decision should be "allow"

  @performance
  Scenario: Policy evaluates with sub-microsecond performance
    Given a principal "user_0"
    When they perform 1000 evaluations on random resources
    Then the average evaluation time should be less than 5 microseconds

  Scenario Outline: Multiple role and resource combinations
    Given a principal "<user>"
    When they perform action "<action>" on resource "<resource>"
    Then the decision should be "<decision>"

    Examples:
      | user    | action | resource      | decision |
      | user_0  | read   | resource_100  | allow    |
      | user_0  | write  | resource_200  | allow    |
      | user_1  | read   | resource_50   | deny     |
      | user_50 | read   | resource_50   | allow    |
      | user_50 | delete | resource_51   | allow    |
      | user_700| read   | resource_900  | allow    |
