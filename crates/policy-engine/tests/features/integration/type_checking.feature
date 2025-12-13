@day5 @type-checking
Feature: Type Checking and Data Validation
  Test type checking functions and validation patterns

  Background:
    Given the policy file "examples/policies/type_checking_policy.reap"
    And the data file "../../test-data/type-checking-test-data.json"

  @is-string @positive
  Scenario: Type check identifies string correctly
    Given a principal "user_string_field"
    When they perform action "validate" on resource "string_check"
    Then the decision should be "allow"

  @is-string @negative
  Scenario: Type check rejects non-string
    Given a principal "user_number_field"
    When they perform action "validate" on resource "string_check"
    Then the decision should be "deny"

  @is-number @positive
  Scenario: Type check identifies number correctly
    Given a principal "user_numeric_value"
    When they perform action "validate" on resource "number_check"
    Then the decision should be "allow"

  @is-number @negative @expected_failure
  Scenario: Type check rejects non-number
    Given a principal "user_text_value"
    When they perform action "validate" on resource "number_check"
    Then the decision should be "deny"

  @is-array @positive
  Scenario: Type check identifies array correctly
    Given a principal "user_array_data"
    When they perform action "validate" on resource "array_check"
    Then the decision should be "allow"

  @is-array @negative @expected_failure
  Scenario: Type check rejects non-array
    Given a principal "user_object_data"
    When they perform action "validate" on resource "array_check"
    Then the decision should be "deny"

  @is-object @positive
  Scenario: Type check identifies object correctly
    Given a principal "user_object_value"
    When they perform action "validate" on resource "object_check"
    Then the decision should be "allow"

  @is-object @negative @expected_failure
  Scenario: Type check rejects non-object
    Given a principal "user_primitive_value"
    When they perform action "validate" on resource "object_check"
    Then the decision should be "deny"

  @is-bool @positive
  Scenario: Type check identifies boolean correctly
    Given a principal "user_bool_flag"
    When they perform action "validate" on resource "bool_check"
    Then the decision should be "allow"

  @is-bool @negative
  Scenario: Type check rejects non-boolean
    Given a principal "user_string_flag"
    When they perform action "validate" on resource "bool_check"
    Then the decision should be "deny"

  @multi-type @positive
  Scenario: Multiple type checks in one rule
    Given a principal "user_valid_schema"
    When they perform action "validate" on resource "schema_check"
    Then the decision should be "allow"

  @multi-type @negative @expected_failure
  Scenario: Multiple type checks fail one
    Given a principal "user_invalid_schema"
    When they perform action "validate" on resource "schema_check"
    Then the decision should be "deny"

  @type-assertion @positive
  Scenario: Type assertion guards operation
    Given a principal "user_safe_operation"
    When they perform action "execute" on resource "guarded_op"
    Then the decision should be "allow"

  @type-assertion @negative
  Scenario: Type assertion prevents unsafe operation
    Given a principal "user_unsafe_operation"
    When they perform action "execute" on resource "guarded_op"
    Then the decision should be "deny"

  @constraint-check @positive
  Scenario: Value constraint validation passes
    Given a principal "user_in_range"
    When they perform action "submit" on resource "constrained_value"
    Then the decision should be "allow"

  @constraint-check @negative
  Scenario: Value constraint validation fails
    Given a principal "user_out_of_range"
    When they perform action "submit" on resource "constrained_value"
    Then the decision should be "deny"

  @format-validation @positive
  Scenario: Format validation succeeds
    Given a principal "user_valid_format"
    When they perform action "parse" on resource "formatted_data"
    Then the decision should be "allow"

  @format-validation @negative
  Scenario: Format validation fails
    Given a principal "user_invalid_format"
    When they perform action "parse" on resource "formatted_data"
    Then the decision should be "deny"

  @null-check @positive
  Scenario: Null check allows non-null value
    Given a principal "user_non_null"
    When they perform action "access" on resource "nullable_field"
    Then the decision should be "allow"

  @null-check @negative
  Scenario: Null check denies null value
    Given a principal "user_null_field"
    When they perform action "access" on resource "nullable_field"
    Then the decision should be "deny"

  @mixed-validation @positive
  Scenario: Combined type and value validation
    Given a principal "user_fully_valid"
    When they perform action "process" on resource "complex_validation"
    Then the decision should be "allow"

  @mixed-validation @negative @expected_failure
  Scenario: Combined validation fails type check
    Given a principal "user_wrong_type"
    When they perform action "process" on resource "complex_validation"
    Then the decision should be "deny"

  @type-performance
  Scenario: Type checking evaluates efficiently
    Given a principal "user_string_field"
    When they perform 1000 evaluations on random resources
    Then the average evaluation time should be less than 25 microseconds

  @type-scenarios @expected_failure
  Scenario Outline: Various type checking scenarios
    Given a principal "<principal>"
    When they perform action "<action>" on resource "<resource>"
    Then the decision should be "<decision>"

    Examples:
      | principal           | action   | resource        | decision |
      | user_string_field   | validate | string_check    | allow    |
      | user_number_field   | validate | string_check    | deny     |
      | user_numeric_value  | validate | number_check    | allow    |
      | user_text_value     | validate | number_check    | deny     |
      | user_array_data     | validate | array_check     | allow    |
      | user_object_data    | validate | array_check     | deny     |
