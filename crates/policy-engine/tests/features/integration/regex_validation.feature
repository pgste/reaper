Feature: Regex Pattern Validation
  Comprehensive integration tests for regex functions with caching

  Background:
    Given the policy file "examples/policies/regex_policy.reap"
    And the data file "../../test-data/regex-test-data.json"

  @regex-matching @positive
  Scenario: Valid email address passes validation
    Given a principal "user_valid_email"
    When they perform action "register" on resource "email_validation"
    Then the decision should be "allow"

  @regex-matching @negative
  Scenario: Invalid email address fails validation
    Given a principal "user_invalid_email"
    When they perform action "register" on resource "email_validation"
    Then the decision should be "deny"

  @regex-matching @positive
  Scenario: Valid phone number passes validation
    Given a principal "user_valid_phone"
    When they perform action "register" on resource "phone_validation"
    Then the decision should be "allow"

  @regex-matching @negative
  Scenario: Invalid phone number fails validation
    Given a principal "user_invalid_phone"
    When they perform action "register" on resource "phone_validation"
    Then the decision should be "deny"

  @regex-matching @positive
  Scenario: Valid URL passes validation
    Given a principal "user_valid_url"
    When they perform action "submit" on resource "url_validation"
    Then the decision should be "allow"

  @regex-matching @negative
  Scenario: Invalid URL fails validation
    Given a principal "user_invalid_url"
    When they perform action "submit" on resource "url_validation"
    Then the decision should be "deny"

  @regex-matching @positive
  Scenario: Valid IP address passes validation
    Given a principal "user_valid_ip"
    When they perform action "connect" on resource "ip_validation"
    Then the decision should be "allow"

  @regex-matching @negative
  Scenario: Invalid IP address fails validation
    Given a principal "user_invalid_ip"
    When they perform action "connect" on resource "ip_validation"
    Then the decision should be "deny"

  @regex-matching @positive
  Scenario: Valid UUID passes validation
    Given a principal "user_valid_uuid"
    When they perform action "lookup" on resource "uuid_validation"
    Then the decision should be "allow"

  @regex-matching @negative
  Scenario: Invalid UUID fails validation
    Given a principal "user_invalid_uuid"
    When they perform action "lookup" on resource "uuid_validation"
    Then the decision should be "deny"

  @regex-matching @positive
  Scenario: Valid credit card number passes validation
    Given a principal "user_valid_cc"
    When they perform action "purchase" on resource "payment_validation"
    Then the decision should be "allow"

  @regex-matching @negative
  Scenario: Invalid credit card number fails validation
    Given a principal "user_invalid_cc"
    When they perform action "purchase" on resource "payment_validation"
    Then the decision should be "deny"

  @regex-replace @positive
  Scenario: Sensitive data is redacted correctly
    Given a principal "user_with_ssn"
    When they perform action "view" on resource "redacted_data"
    Then the decision should be "allow"

  @regex-split @positive
  Scenario: CSV parsing works correctly
    Given a principal "user_with_csv"
    When they perform action "parse" on resource "csv_data"
    Then the decision should be "allow"

  @regex-extract @positive
  Scenario: Data extraction from structured text
    Given a principal "user_with_log"
    When they perform action "analyze" on resource "log_entry"
    Then the decision should be "allow"

  @performance @regex-caching
  Scenario: Regex caching provides performance benefits
    Given a principal "user_valid_email"
    When they perform 1000 evaluations on random resources
    Then the average evaluation time should be less than 50 microseconds

  Scenario Outline: Various regex validation scenarios
    Given a principal "<user>"
    When they perform action "<action>" on resource "<resource>"
    Then the decision should be "<decision>"

    Examples:
      | user                | action   | resource          | decision |
      | user_valid_email    | register | email_validation  | allow    |
      | user_invalid_email  | register | email_validation  | deny     |
      | user_valid_phone    | register | phone_validation  | allow    |
      | user_invalid_phone  | register | phone_validation  | deny     |
      | user_valid_url      | submit   | url_validation    | allow    |
      | user_invalid_url    | submit   | url_validation    | deny     |
      | user_valid_ip       | connect  | ip_validation     | allow    |
      | user_invalid_ip     | connect  | ip_validation     | deny     |
