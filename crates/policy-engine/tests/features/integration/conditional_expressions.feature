@day5 @conditional-expressions
Feature: Conditional Expressions and Advanced Logic
  Test if/else expressions, ternary operations, and complex conditionals

  Background:
    Given the policy file "examples/policies/conditional_policy.reap"
    And the data file "../../test-data/conditional-test-data.json"

  @if-else @positive
  Scenario: Simple if-else expression evaluates to true branch
    Given a principal "user_adult"
    When they perform action "access" on resource "age_restricted"
    Then the decision should be "allow"

  @if-else @negative
  Scenario: Simple if-else expression evaluates to false branch
    Given a principal "user_minor"
    When they perform action "access" on resource "age_restricted"
    Then the decision should be "deny"

  @nested-if @positive
  Scenario: Nested if-else expressions
    Given a principal "user_premium_adult"
    When they perform action "access" on resource "premium_content"
    Then the decision should be "allow"

  @nested-if @negative
  Scenario: Nested if-else fails inner condition
    Given a principal "user_premium_minor"
    When they perform action "access" on resource "premium_content"
    Then the decision should be "deny"

  @ternary @positive
  Scenario: Ternary-style conditional assignment succeeds
    Given a principal "user_high_score"
    When they perform action "qualify" on resource "leaderboard"
    Then the decision should be "allow"

  @ternary @negative
  Scenario: Ternary-style conditional assignment fails
    Given a principal "user_low_score"
    When they perform action "qualify" on resource "leaderboard"
    Then the decision should be "deny"

  @chained-conditional @positive
  Scenario: Chained conditional expressions
    Given a principal "user_tier_gold"
    When they perform action "upgrade" on resource "subscription"
    Then the decision should be "allow"

  @chained-conditional @negative
  Scenario: Chained conditionals fail at second check
    Given a principal "user_tier_bronze"
    When they perform action "upgrade" on resource "subscription"
    Then the decision should be "deny"

  @complex-boolean @positive
  Scenario: Complex boolean logic with conditionals
    Given a principal "user_verified_active"
    When they perform action "transact" on resource "payment"
    Then the decision should be "allow"

  @complex-boolean @negative
  Scenario: Complex boolean fails one condition
    Given a principal "user_unverified_active"
    When they perform action "transact" on resource "payment"
    Then the decision should be "deny"

  @short-circuit @positive
  Scenario: Short-circuit AND evaluation
    Given a principal "user_early_exit"
    When they perform action "evaluate" on resource "logic_test"
    Then the decision should be "allow"

  @short-circuit @negative
  Scenario: Short-circuit OR evaluation
    Given a principal "user_or_condition"
    When they perform action "evaluate" on resource "logic_test"
    Then the decision should be "deny"

  @null-coalesce @positive @expected_failure
  Scenario: Null coalescing with default value
    Given a principal "user_missing_field"
    When they perform action "process" on resource "nullable_data"
    Then the decision should be "allow"

  @null-coalesce @negative
  Scenario: Null coalescing rejects null
    Given a principal "user_null_value"
    When they perform action "process" on resource "nullable_data"
    Then the decision should be "deny"

  @conditional-method @positive
  Scenario: Conditional with method calls
    Given a principal "user_long_name"
    When they perform action "validate" on resource "name_check"
    Then the decision should be "allow"

  @conditional-method @negative
  Scenario: Conditional method call fails
    Given a principal "user_short_name"
    When they perform action "validate" on resource "name_check"
    Then the decision should be "deny"

  @multi-branch @positive
  Scenario: Multiple conditional branches
    Given a principal "user_category_a"
    When they perform action "classify" on resource "categorizer"
    Then the decision should be "allow"

  @multi-branch @negative
  Scenario: Multiple branches with no match
    Given a principal "user_category_unknown"
    When they perform action "classify" on resource "categorizer"
    Then the decision should be "deny"

  @conditional-aggregate @positive
  Scenario: Conditional within aggregation
    Given a principal "user_threshold_data"
    When they perform action "compute" on resource "conditional_sum"
    Then the decision should be "allow"

  @conditional-aggregate @negative
  Scenario: Conditional aggregation below threshold
    Given a principal "user_below_threshold"
    When they perform action "compute" on resource "conditional_sum"
    Then the decision should be "deny"

  @conditional-performance
  Scenario: Conditional expressions evaluate efficiently
    Given a principal "user_adult"
    When they perform 1000 evaluations on random resources
    Then the average evaluation time should be less than 30 microseconds

  @conditional-scenarios
  Scenario Outline: Various conditional expression scenarios
    Given a principal "<principal>"
    When they perform action "<action>" on resource "<resource>"
    Then the decision should be "<decision>"

    Examples:
      | principal              | action    | resource         | decision |
      | user_adult            | access    | age_restricted   | allow    |
      | user_minor            | access    | age_restricted   | deny     |
      | user_high_score       | qualify   | leaderboard      | allow    |
      | user_low_score        | qualify   | leaderboard      | deny     |
      | user_verified_active  | transact  | payment          | allow    |
      | user_unverified_active| transact  | payment          | deny     |
