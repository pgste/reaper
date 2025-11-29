@abac
Feature: ABAC Policy Validation
  Test attribute-based access control with clearances and departments

  Background:
    Given the policy file "examples/policies/abac.reap"
    And the data file "../../test-data/abac-test-data.json"

  @suspended @negative
  Scenario: Suspended users are blocked
    Given a principal "user_20"
    When they perform action "read" on resource "resource_100"
    Then the decision should be "deny"

  @clearance @positive
  Scenario: High clearance users can access confidential docs
    Given a principal "user_0"
    When they perform action "read" on resource "resource_100"
    Then the decision should be "allow"

  @executive @positive
  Scenario: Executives have broad access
    Given a principal "user_0"
    When they perform action "read" on resource "resource_501"
    Then the decision should be "allow"

  @ownership @positive
  Scenario: Document owners can access their documents
    Given a principal "user_50"
    When they perform action "read" on resource "resource_50"
    Then the decision should be "allow"

  @performance
  Scenario: ABAC policy maintains sub-microsecond performance
    Given a principal "user_0"
    When they perform 1000 evaluations on random resources
    Then the average evaluation time should be less than 25 microseconds
