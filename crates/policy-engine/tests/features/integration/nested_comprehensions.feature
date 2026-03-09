@day5 @nested-comprehensions
Feature: Nested and Complex Comprehensions
  Test multi-level comprehensions and complex transformation patterns

  Background:
    Given the policy file "examples/policies/nested_comprehension_policy.reap"
    And the data file "../../test-data/nested-comprehension-test-data.json"

  @nested-array @positive @expected_failure
  Scenario: Nested array comprehension flattens matrix
    Given a principal "user_matrix_data"
    When they perform action "flatten" on resource "matrix_result"
    Then the decision should be "allow"

  @nested-array @negative @expected_failure
  Scenario: Nested array comprehension with insufficient data
    Given a principal "user_sparse_matrix"
    When they perform action "flatten" on resource "matrix_result"
    Then the decision should be "deny"

  @nested-set @positive
  Scenario: Nested set comprehension extracts unique values
    Given a principal "user_grouped_data"
    When they perform action "extract" on resource "unique_values"
    Then the decision should be "allow"

  @nested-set @negative
  Scenario: Nested set with no qualifying values
    Given a principal "user_empty_groups"
    When they perform action "extract" on resource "unique_values"
    Then the decision should be "deny"

  @nested-object @positive
  Scenario: Nested object comprehension builds hierarchy
    Given a principal "user_hierarchical"
    When they perform action "build" on resource "hierarchy_map"
    Then the decision should be "allow"

  @nested-object @negative
  Scenario: Nested object with missing structure
    Given a principal "user_flat_data"
    When they perform action "build" on resource "hierarchy_map"
    Then the decision should be "deny"

  @filter-chain @positive
  Scenario: Comprehension with multiple chained filters
    Given a principal "user_complex_filter"
    When they perform action "query" on resource "filtered_results"
    Then the decision should be "allow"

  @filter-chain @negative
  Scenario: Chained filters exclude all items
    Given a principal "user_no_match"
    When they perform action "query" on resource "filtered_results"
    Then the decision should be "deny"

  @method-chain @positive
  Scenario: Comprehension with method chains
    Given a principal "user_text_data"
    When they perform action "transform" on resource "processed_text"
    Then the decision should be "allow"

  @method-chain @negative
  Scenario: Method chain produces empty result
    Given a principal "user_invalid_text"
    When they perform action "transform" on resource "processed_text"
    Then the decision should be "deny"

  @deep-nesting @positive
  Scenario: Three-level nested comprehension
    Given a principal "user_deep_structure"
    When they perform action "traverse" on resource "deep_result"
    Then the decision should be "allow"

  @deep-nesting @negative
  Scenario: Deep nesting with insufficient depth
    Given a principal "user_shallow_structure"
    When they perform action "traverse" on resource "deep_result"
    Then the decision should be "deny"

  @conditional-filter @positive
  Scenario: Comprehension with conditional filters
    Given a principal "user_conditional_data"
    When they perform action "filter" on resource "conditional_result"
    Then the decision should be "allow"

  @conditional-filter @negative
  Scenario: Conditional filters fail condition
    Given a principal "user_wrong_condition"
    When they perform action "filter" on resource "conditional_result"
    Then the decision should be "deny"

  @aggregation @positive
  Scenario: Nested comprehension for aggregation
    Given a principal "user_aggregate_data"
    When they perform action "aggregate" on resource "summary"
    Then the decision should be "allow"

  @aggregation @negative
  Scenario: Aggregation with insufficient data points
    Given a principal "user_limited_data"
    When they perform action "aggregate" on resource "summary"
    Then the decision should be "deny"

  @transformation @positive
  Scenario: Complex object transformation via comprehension
    Given a principal "user_transformable_objects"
    When they perform action "reshape" on resource "transformed"
    Then the decision should be "allow"

  @transformation @negative
  Scenario: Transformation fails on malformed data
    Given a principal "user_malformed_objects"
    When they perform action "reshape" on resource "transformed"
    Then the decision should be "deny"

  @mixed-types @positive
  Scenario: Comprehension handling mixed types
    Given a principal "user_mixed_collection"
    When they perform action "process" on resource "type_filtered"
    Then the decision should be "allow"

  @mixed-types @negative
  Scenario: Mixed types with no valid items
    Given a principal "user_incompatible_types"
    When they perform action "process" on resource "type_filtered"
    Then the decision should be "deny"

  @nested-performance
  Scenario: Nested comprehensions evaluate efficiently
    Given a principal "user_matrix_data"
    When they perform 1000 evaluations on random resources
    Then the average evaluation time should be less than 50 microseconds

  @nested-scenarios @expected_failure
  Scenario Outline: Various nested comprehension scenarios
    Given a principal "<principal>"
    When they perform action "<action>" on resource "<resource>"
    Then the decision should be "<decision>"

    Examples:
      | principal                  | action    | resource           | decision |
      | user_matrix_data          | flatten   | matrix_result      | allow    |
      | user_sparse_matrix        | flatten   | matrix_result      | deny     |
      | user_grouped_data         | extract   | unique_values      | allow    |
      | user_empty_groups         | extract   | unique_values      | deny     |
      | user_text_data            | transform | processed_text     | allow    |
      | user_invalid_text         | transform | processed_text     | deny     |
