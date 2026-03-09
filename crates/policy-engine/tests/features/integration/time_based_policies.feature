@integration @time-functions @phase7
Feature: Time-Based Policy Validation
  Validate all time/date functions work correctly in realistic policy scenarios

  Background:
    Given the policy file "examples/policies/time_policy.reap"
    And the data file "../../test-data/time-test-data.json"

  @token-expiration @positive
  Scenario: Valid token allows access
    Given a principal "user_valid_token"
    When they perform action "api_call" on resource "api_endpoint"
    Then the decision should be "allow"

  @token-expiration @negative
  Scenario: Expired token denies access
    Given a principal "user_expired_token"
    When they perform action "api_call" on resource "api_endpoint"
    Then the decision should be "deny"

  @business-hours @positive
  Scenario: Access allowed during business hours
    Given a principal "employee"
    When they perform action "access" on resource "office_system"
    Then the decision should be "allow"

  @business-hours @negative
  Scenario: Access denied outside business hours
    Given a principal "employee_after_hours"
    When they perform action "access" on resource "office_system"
    Then the decision should be "deny"

  @age-verification @positive
  Scenario: User meets age requirement
    Given a principal "user_adult"
    When they perform action "purchase" on resource "alcohol"
    Then the decision should be "allow"

  @age-verification @negative
  Scenario: User below age requirement
    Given a principal "user_minor"
    When they perform action "purchase" on resource "alcohol"
    Then the decision should be "deny"

  @lease-expiration @positive
  Scenario: Active lease allows resource access
    Given a principal "tenant_active"
    When they perform action "enter" on resource "apartment_101"
    Then the decision should be "allow"

  @lease-expiration @negative
  Scenario: Expired lease denies resource access
    Given a principal "tenant_expired"
    When they perform action "enter" on resource "apartment_101"
    Then the decision should be "deny"

  @time-window @positive
  Scenario: Access within maintenance window
    Given a principal "operator"
    When they perform action "deploy" on resource "production_system"
    Then the decision should be "allow"

  @time-window @negative
  Scenario: Deployment blocked outside maintenance window
    Given a principal "operator_wrong_time"
    When they perform action "deploy" on resource "production_system_outside_window"
    Then the decision should be "deny"

  @time-arithmetic @positive
  Scenario: Session extended with time addition
    Given a principal "user_extended_session"
    When they perform action "continue" on resource "web_session"
    Then the decision should be "allow"

  @time-comparison @positive
  Scenario: Event scheduled for future time
    Given a principal "event_planner"
    When they perform action "schedule" on resource "conference_room"
    Then the decision should be "allow"

  @time-comparison @negative
  Scenario: Event scheduled in past rejected
    Given a principal "event_planner_past"
    When they perform action "schedule" on resource "conference_room"
    Then the decision should be "deny"

  @temporal-access @positive
  Scenario: Temporary access grant is active
    Given a principal "contractor_active"
    When they perform action "read" on resource "project_files"
    Then the decision should be "allow"

  @temporal-access @negative
  Scenario: Temporary access grant has expired
    Given a principal "contractor_expired"
    When they perform action "read" on resource "project_files"
    Then the decision should be "deny"

  @time-parsing @positive
  Scenario: RFC3339 timestamp parsed correctly
    Given a principal "system_with_timestamp"
    When they perform action "validate" on resource "timestamp_data"
    Then the decision should be "allow"

  @time-formatting @positive
  Scenario: Time formatted to RFC3339 for logging
    Given a principal "audit_logger"
    When they perform action "log" on resource "audit_trail"
    Then the decision should be "allow"

  @rate-limiting @positive
  Scenario: Request within rate limit window
    Given a principal "api_client_normal"
    When they perform action "request" on resource "rate_limited_endpoint"
    Then the decision should be "allow"

  @rate-limiting @negative
  Scenario: Request exceeds rate limit window
    Given a principal "api_client_exceeded"
    When they perform action "request" on resource "rate_limited_endpoint"
    Then the decision should be "deny"

  @data-retention @positive
  Scenario: Data within retention period
    Given a principal "archiver"
    When they perform action "archive" on resource "old_data"
    Then the decision should be "allow"

  @data-retention @negative
  Scenario: Data exceeds retention period and should be deleted
    Given a principal "retention_policy"
    When they perform action "keep" on resource "expired_data"
    Then the decision should be "deny"

  @performance @time-functions
  Scenario: Time-based policies evaluate efficiently
    Given a principal "user_valid_token"
    When they perform 1000 evaluations on random resources
    Then the average evaluation time should be less than 100 microseconds

  Scenario Outline: Multiple time-based scenarios
    Given a principal "<user>"
    When they perform action "<action>" on resource "<resource>"
    Then the decision should be "<decision>"

    Examples:
      | user                  | action    | resource              | decision |
      | user_valid_token      | api_call  | api_endpoint          | allow    |
      | user_expired_token    | api_call  | api_endpoint          | deny     |
      | employee              | access    | office_system         | allow    |
      | user_adult            | purchase  | alcohol               | allow    |
      | user_minor            | purchase  | alcohol               | deny     |
      | tenant_active         | enter     | apartment_101         | allow    |
      | tenant_expired        | enter     | apartment_101         | deny     |
      | operator              | deploy    | production_system     | allow    |
      | contractor_active     | read      | project_files         | allow    |
      | contractor_expired    | read      | project_files         | deny     |
