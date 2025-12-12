Feature: JSON Operations
  Comprehensive integration tests for JSON parsing and manipulation

  Background:
    Given the policy file "examples/policies/json_policy.reap"
    And the data file "../../test-data/json-test-data.json"

  @json-parse @positive
  Scenario: Valid JSON payload is accepted
    Given a principal "user_valid_json"
    When they perform action "submit" on resource "api_endpoint"
    Then the decision should be "allow"

  @json-parse @negative
  Scenario: Invalid JSON payload is rejected
    Given a principal "user_invalid_json"
    When they perform action "submit" on resource "api_endpoint"
    Then the decision should be "deny"

  @json-path @positive
  Scenario: JSON path access finds required field
    Given a principal "user_complete_profile"
    When they perform action "verify" on resource "user_profile"
    Then the decision should be "allow"

  @json-path @negative
  Scenario: JSON path access missing required field
    Given a principal "user_incomplete_profile"
    When they perform action "verify" on resource "user_profile"
    Then the decision should be "deny"

  @json-nested @positive
  Scenario: Nested JSON structure has required data
    Given a principal "user_nested_data"
    When they perform action "process" on resource "payment"
    Then the decision should be "allow"

  @json-nested @negative
  Scenario: Nested JSON structure missing data
    Given a principal "user_missing_nested"
    When they perform action "process" on resource "payment"
    Then the decision should be "deny"

  @json-array @positive
  Scenario: JSON array contains required items
    Given a principal "user_array_items"
    When they perform action "validate" on resource "order"
    Then the decision should be "allow"

  @json-array @negative
  Scenario: JSON array missing required items
    Given a principal "user_empty_array"
    When they perform action "validate" on resource "order"
    Then the decision should be "deny"

  @json-type @positive
  Scenario: JSON field has correct type
    Given a principal "user_correct_types"
    When they perform action "save" on resource "form_data"
    Then the decision should be "allow"

  @json-type @negative
  Scenario: JSON field has wrong type
    Given a principal "user_wrong_types"
    When they perform action "save" on resource "form_data"
    Then the decision should be "deny"

  @json-string @positive
  Scenario: JSON string field validation passes
    Given a principal "user_valid_string"
    When they perform action "update" on resource "text_field"
    Then the decision should be "allow"

  @json-number @positive
  Scenario: JSON number field validation passes
    Given a principal "user_valid_number"
    When they perform action "update" on resource "number_field"
    Then the decision should be "allow"

  @json-boolean @positive
  Scenario: JSON boolean field validation passes
    Given a principal "user_valid_boolean"
    When they perform action "update" on resource "boolean_field"
    Then the decision should be "allow"

  @json-object @positive
  Scenario: JSON object field validation passes
    Given a principal "user_valid_object"
    When they perform action "create" on resource "structured_data"
    Then the decision should be "allow"

  @json-merge @positive
  Scenario: Merged JSON objects meet requirements
    Given a principal "user_merge_data"
    When they perform action "combine" on resource "data_merge"
    Then the decision should be "allow"

  @performance @json
  Scenario: JSON operations evaluate efficiently
    Given a principal "user_valid_json"
    When they perform 1000 evaluations on random resources
    Then the average evaluation time should be less than 100 microseconds

  Scenario Outline: Various JSON operation scenarios
    Given a principal "<user>"
    When they perform action "<action>" on resource "<resource>"
    Then the decision should be "<decision>"

    Examples:
      | user                    | action   | resource        | decision |
      | user_valid_json         | submit   | api_endpoint    | allow    |
      | user_invalid_json       | submit   | api_endpoint    | deny     |
      | user_complete_profile   | verify   | user_profile    | allow    |
      | user_incomplete_profile | verify   | user_profile    | deny     |
      | user_nested_data        | process  | payment         | allow    |
      | user_missing_nested     | process  | payment         | deny     |
      | user_array_items        | validate | order           | allow    |
      | user_empty_array        | validate | order           | deny     |
