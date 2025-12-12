@day4 @advanced-collections
Feature: Advanced Collection Operations
  Test advanced collection manipulation methods

  Background:
    Given the policy file "examples/policies/advanced_collection_policy.reap"
    And the data file "../../test-data/advanced-collection-test-data.json"

  @collection-first @positive
  Scenario: First element meets requirement
    Given a principal "user_priority_tasks"
    When they perform action "check" on resource "task_queue"
    Then the decision should be "allow"

  @collection-first @negative
  Scenario: First element doesn't meet requirement
    Given a principal "user_low_priority_first"
    When they perform action "check" on resource "task_queue"
    Then the decision should be "deny"

  @collection-last @positive
  Scenario: Last element meets requirement
    Given a principal "user_recent_login"
    When they perform action "verify" on resource "session"
    Then the decision should be "allow"

  @collection-last @negative
  Scenario: Last element doesn't meet requirement
    Given a principal "user_old_login"
    When they perform action "verify" on resource "session"
    Then the decision should be "deny"

  @collection-slice @positive
  Scenario: Sliced collection has required properties
    Given a principal "user_top_scores"
    When they perform action "view" on resource "leaderboard"
    Then the decision should be "allow"

  @collection-slice @negative
  Scenario: Sliced collection doesn't meet requirements
    Given a principal "user_low_scores"
    When they perform action "view" on resource "leaderboard"
    Then the decision should be "deny"

  @collection-reverse @positive
  Scenario: Reversed collection meets requirement
    Given a principal "user_desc_order"
    When they perform action "sort" on resource "items"
    Then the decision should be "allow"

  @collection-sort @positive
  Scenario: Sorted collection is in correct order
    Given a principal "user_sortable_data"
    When they perform action "organize" on resource "records"
    Then the decision should be "allow"

  @collection-sort @negative
  Scenario: Unsortable collection fails
    Given a principal "user_mixed_types"
    When they perform action "organize" on resource "records"
    Then the decision should be "deny"

  @collection-unique @positive
  Scenario: Unique elements meet minimum count
    Given a principal "user_unique_skills"
    When they perform action "apply" on resource "specialized_role"
    Then the decision should be "allow"

  @collection-unique @negative
  Scenario: Not enough unique elements
    Given a principal "user_duplicate_skills"
    When they perform action "apply" on resource "specialized_role"
    Then the decision should be "deny"

  @set-union @positive
  Scenario: Union of sets has required elements
    Given a principal "user_combined_perms"
    When they perform action "execute" on resource "multi_function"
    Then the decision should be "allow"

  @set-union @negative
  Scenario: Union of sets missing required elements
    Given a principal "user_limited_perms"
    When they perform action "execute" on resource "multi_function"
    Then the decision should be "deny"

  @set-difference @positive
  Scenario: Set difference removes forbidden items
    Given a principal "user_filtered_access"
    When they perform action "list" on resource "filtered_content"
    Then the decision should be "allow"

  @set-difference @negative
  Scenario: Set difference still contains forbidden items
    Given a principal "user_unfiltered_access"
    When they perform action "list" on resource "filtered_content"
    Then the decision should be "deny"

  @aggregate-max @positive
  Scenario: Maximum value meets threshold
    Given a principal "user_high_max_score"
    When they perform action "qualify" on resource "competition"
    Then the decision should be "allow"

  @aggregate-max @negative
  Scenario: Maximum value below threshold
    Given a principal "user_low_max_score"
    When they perform action "qualify" on resource "competition"
    Then the decision should be "deny"

  @aggregate-min @positive
  Scenario: Minimum value above threshold
    Given a principal "user_consistent_performance"
    When they perform action "certify" on resource "quality_check"
    Then the decision should be "allow"

  @aggregate-min @negative
  Scenario: Minimum value below threshold
    Given a principal "user_inconsistent_performance"
    When they perform action "certify" on resource "quality_check"
    Then the decision should be "deny"

  @collection-performance
  Scenario: Advanced collection operations evaluate efficiently
    Given a principal "user_priority_tasks"
    When they perform 1000 evaluations on random resources
    Then the average evaluation time should be less than 35 microseconds

  @collection-scenarios
  Scenario Outline: Various advanced collection scenarios
    Given a principal "<principal>"
    When they perform action "<action>" on resource "<resource>"
    Then the decision should be "<decision>"

    Examples:
      | principal                  | action   | resource           | decision |
      | user_priority_tasks        | check    | task_queue         | allow    |
      | user_low_priority_first    | check    | task_queue         | deny     |
      | user_recent_login          | verify   | session            | allow    |
      | user_old_login             | verify   | session            | deny     |
      | user_top_scores            | view     | leaderboard        | allow    |
      | user_unique_skills         | apply    | specialized_role   | allow    |
      | user_duplicate_skills      | apply    | specialized_role   | deny     |
      | user_combined_perms        | execute  | multi_function     | allow    |
      | user_high_max_score        | qualify  | competition        | allow    |
      | user_consistent_performance| certify  | quality_check      | allow    |
