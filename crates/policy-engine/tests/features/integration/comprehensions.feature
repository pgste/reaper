@day4 @comprehensions
Feature: Comprehension Expressions
  Test set, array, and object comprehensions with filters

  Background:
    Given the policy file "examples/policies/comprehension_policy.reap"
    And the data file "../../test-data/comprehension-test-data.json"

  @set-comprehension @positive
  Scenario: Set comprehension filters correctly
    Given a principal "user_filtered_set"
    When they perform action "compute" on resource "set_result"
    Then the decision should be "allow"

  @set-comprehension @negative
  Scenario: Set comprehension doesn't produce required values
    Given a principal "user_no_matches"
    When they perform action "compute" on resource "set_result"
    Then the decision should be "deny"

  @array-comprehension @positive
  Scenario: Array comprehension preserves order
    Given a principal "user_ordered_data"
    When they perform action "process" on resource "array_result"
    Then the decision should be "allow"

  @array-comprehension @negative
  Scenario: Array comprehension insufficient results
    Given a principal "user_sparse_data"
    When they perform action "process" on resource "array_result"
    Then the decision should be "deny"

  @object-comprehension @positive
  Scenario: Object comprehension creates valid mapping
    Given a principal "user_map_data"
    When they perform action "transform" on resource "object_result"
    Then the decision should be "allow"

  @object-comprehension @negative
  Scenario: Object comprehension produces empty result
    Given a principal "user_no_mapping"
    When they perform action "transform" on resource "object_result"
    Then the decision should be "deny"

  @comprehension-filter @positive
  Scenario: Multiple filters in comprehension
    Given a principal "user_multi_filter"
    When they perform action "query" on resource "complex_filter"
    Then the decision should be "allow"

  @comprehension-filter @negative
  Scenario: Filters exclude all elements
    Given a principal "user_excluded_all"
    When they perform action "query" on resource "complex_filter"
    Then the decision should be "deny"

  @comprehension-nested @positive
  Scenario: Nested comprehensions work correctly
    Given a principal "user_nested_data"
    When they perform action "flatten" on resource "nested_result"
    Then the decision should be "allow"

  @comprehension-nested @negative
  Scenario: Nested comprehension insufficient depth
    Given a principal "user_shallow_data"
    When they perform action "flatten" on resource "nested_result"
    Then the decision should be "deny"

  @comprehension-transform @positive
  Scenario: Comprehension with transformation functions
    Given a principal "user_transformable"
    When they perform action "convert" on resource "transformed_data"
    Then the decision should be "allow"

  @comprehension-performance
  Scenario: Comprehensions evaluate efficiently
    Given a principal "user_filtered_set"
    When they perform 1000 evaluations on random resources
    Then the average evaluation time should be less than 30 microseconds

  @comprehension-scenarios
  Scenario Outline: Various comprehension scenarios
    Given a principal "<principal>"
    When they perform action "<action>" on resource "<resource>"
    Then the decision should be "<decision>"

    Examples:
      | principal          | action    | resource          | decision |
      | user_filtered_set  | compute   | set_result        | allow    |
      | user_no_matches    | compute   | set_result        | deny     |
      | user_ordered_data  | process   | array_result      | allow    |
      | user_sparse_data   | process   | array_result      | deny     |
      | user_map_data      | transform | object_result     | allow    |
      | user_multi_filter  | query     | complex_filter    | allow    |
      | user_nested_data   | flatten   | nested_result     | allow    |
      | user_transformable | convert   | transformed_data  | allow    |
