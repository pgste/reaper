Feature: Math Operations and Numeric Validation
  Comprehensive integration tests for math functions and numeric operations

  Background:
    Given the policy file "examples/policies/math_policy.reap"
    And the data file "../../test-data/math-test-data.json"

  @math-comparison @positive
  Scenario: User credit score above threshold
    Given a principal "user_high_credit"
    When they perform action "apply" on resource "premium_loan"
    Then the decision should be "allow"

  @math-comparison @negative
  Scenario: User credit score below threshold
    Given a principal "user_low_credit"
    When they perform action "apply" on resource "premium_loan"
    Then the decision should be "deny"

  @math-arithmetic @positive
  Scenario: Order total within budget limit
    Given a principal "user_budget_ok"
    When they perform action "checkout" on resource "shopping_cart"
    Then the decision should be "allow"

  @math-arithmetic @negative
  Scenario: Order total exceeds budget limit
    Given a principal "user_budget_exceeded"
    When they perform action "checkout" on resource "shopping_cart"
    Then the decision should be "deny"

  @math-aggregates @positive
  Scenario: Average rating meets minimum requirement
    Given a principal "seller_good_rating"
    When they perform action "promote" on resource "featured_listing"
    Then the decision should be "allow"

  @math-aggregates @negative
  Scenario: Average rating below minimum requirement
    Given a principal "seller_poor_rating"
    When they perform action "promote" on resource "featured_listing"
    Then the decision should be "deny"

  @math-min-max @positive
  Scenario: Price within acceptable range
    Given a principal "seller_fair_price"
    When they perform action "list" on resource "marketplace"
    Then the decision should be "allow"

  @math-min-max @negative
  Scenario: Price exceeds maximum allowed
    Given a principal "seller_high_price"
    When they perform action "list" on resource "marketplace"
    Then the decision should be "deny"

  @math-round @positive
  Scenario: Rounded score qualifies for tier
    Given a principal "user_tier_upgrade"
    When they perform action "access" on resource "premium_tier"
    Then the decision should be "allow"

  @math-abs @positive
  Scenario: Temperature within safe range (absolute value)
    Given a principal "sensor_normal_temp"
    When they perform action "report" on resource "temperature_monitor"
    Then the decision should be "allow"

  @math-abs @negative
  Scenario: Temperature outside safe range
    Given a principal "sensor_extreme_temp"
    When they perform action "report" on resource "temperature_monitor"
    Then the decision should be "deny"

  @math-sum @positive
  Scenario: Total points sufficient for reward
    Given a principal "user_enough_points"
    When they perform action "redeem" on resource "loyalty_reward"
    Then the decision should be "allow"

  @math-sum @negative
  Scenario: Total points insufficient for reward
    Given a principal "user_few_points"
    When they perform action "redeem" on resource "loyalty_reward"
    Then the decision should be "deny"

  @math-percentage @positive
  Scenario: Discount percentage within policy limits
    Given a principal "user_valid_discount"
    When they perform action "apply_discount" on resource "sale_item"
    Then the decision should be "allow"

  @math-percentage @negative
  Scenario: Discount percentage exceeds policy limits
    Given a principal "user_excessive_discount"
    When they perform action "apply_discount" on resource "sale_item"
    Then the decision should be "deny"

  @performance @math-operations
  Scenario: Math operations evaluate efficiently
    Given a principal "user_high_credit"
    When they perform 1000 evaluations on random resources
    Then the average evaluation time should be less than 50 microseconds

  Scenario Outline: Various math validation scenarios
    Given a principal "<user>"
    When they perform action "<action>" on resource "<resource>"
    Then the decision should be "<decision>"

    Examples:
      | user                  | action          | resource             | decision |
      | user_high_credit      | apply           | premium_loan         | allow    |
      | user_low_credit       | apply           | premium_loan         | deny     |
      | user_budget_ok        | checkout        | shopping_cart        | allow    |
      | user_budget_exceeded  | checkout        | shopping_cart        | deny     |
      | seller_good_rating    | promote         | featured_listing     | allow    |
      | seller_poor_rating    | promote         | featured_listing     | deny     |
      | seller_fair_price     | list            | marketplace          | allow    |
      | seller_high_price     | list            | marketplace          | deny     |
