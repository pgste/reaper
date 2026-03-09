@day4 @string-operations
Feature: String Operations
  Test string manipulation methods in policies

  Background:
    Given the policy file "examples/policies/string_policy.reap"
    And the data file "../../test-data/string-test-data.json"

  @string-lower @positive
  Scenario: Lowercase comparison succeeds
    Given a principal "user_mixed_case"
    When they perform action "read" on resource "case_insensitive"
    Then the decision should be "allow"

  @string-lower @negative
  Scenario: Lowercase comparison fails
    Given a principal "user_wrong_case"
    When they perform action "read" on resource "case_insensitive"
    Then the decision should be "deny"

  @string-upper @positive
  Scenario: Uppercase transformation matches
    Given a principal "user_uppercase_code"
    When they perform action "submit" on resource "code_entry"
    Then the decision should be "allow"

  @string-upper @negative
  Scenario: Uppercase transformation doesn't match
    Given a principal "user_lowercase_code"
    When they perform action "submit" on resource "code_entry"
    Then the decision should be "deny"

  @string-trim @positive
  Scenario: Trimmed string matches required value
    Given a principal "user_whitespace_role"
    When they perform action "access" on resource "trimmed_check"
    Then the decision should be "allow"

  @string-trim @negative
  Scenario: Trimmed string doesn't match
    Given a principal "user_wrong_role"
    When they perform action "access" on resource "trimmed_check"
    Then the decision should be "deny"

  @string-contains @positive
  Scenario: String contains required substring
    Given a principal "user_email_contains_company"
    When they perform action "view" on resource "internal_docs"
    Then the decision should be "allow"

  @string-contains @negative
  Scenario: String doesn't contain required substring
    Given a principal "user_external_email"
    When they perform action "view" on resource "internal_docs"
    Then the decision should be "deny"

  @string-startswith @positive
  Scenario: String starts with required prefix
    Given a principal "user_admin_username"
    When they perform action "configure" on resource "system_settings"
    Then the decision should be "allow"

  @string-startswith @negative
  Scenario: String doesn't start with required prefix
    Given a principal "user_regular_username"
    When they perform action "configure" on resource "system_settings"
    Then the decision should be "deny"

  @string-endswith @positive
  Scenario: String ends with required suffix
    Given a principal "user_gov_email"
    When they perform action "access" on resource "classified_docs"
    Then the decision should be "allow"

  @string-endswith @negative
  Scenario: String doesn't end with required suffix
    Given a principal "user_commercial_email"
    When they perform action "access" on resource "classified_docs"
    Then the decision should be "deny"

  @string-split @positive
  Scenario: Split string produces required element count
    Given a principal "user_full_name"
    When they perform action "register" on resource "profile"
    Then the decision should be "allow"

  @string-split @negative
  Scenario: Split string has insufficient elements
    Given a principal "user_single_name"
    When they perform action "register" on resource "profile"
    Then the decision should be "deny"

  @string-complex @positive
  Scenario: Complex string operations combined
    Given a principal "user_complex_email"
    When they perform action "validate" on resource "email_check"
    Then the decision should be "allow"

  @string-performance
  Scenario: String operations evaluate efficiently
    Given a principal "user_mixed_case"
    When they perform 1000 evaluations on random resources
    Then the average evaluation time should be less than 20 microseconds

  @string-scenarios
  Scenario Outline: Various string operation scenarios
    Given a principal "<principal>"
    When they perform action "<action>" on resource "<resource>"
    Then the decision should be "<decision>"

    Examples:
      | principal                  | action    | resource            | decision |
      | user_mixed_case           | read      | case_insensitive    | allow    |
      | user_wrong_case           | read      | case_insensitive    | deny     |
      | user_uppercase_code       | submit    | code_entry          | allow    |
      | user_whitespace_role      | access    | trimmed_check       | allow    |
      | user_email_contains_company | view    | internal_docs       | allow    |
      | user_external_email       | view      | internal_docs       | deny     |
      | user_admin_username       | configure | system_settings     | allow    |
      | user_gov_email            | access    | classified_docs     | allow    |
      | user_full_name            | register  | profile             | allow    |
