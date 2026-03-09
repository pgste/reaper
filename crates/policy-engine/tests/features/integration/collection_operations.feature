Feature: Collection Operations
  Comprehensive integration tests for array, set, and map operations

  Background:
    Given the policy file "examples/policies/collection_policy.reap"
    And the data file "../../test-data/collection-test-data.json"

  @array-contains @positive
  Scenario: User has required permission in array
    Given a principal "user_with_read"
    When they perform action "view" on resource "document"
    Then the decision should be "allow"

  @array-contains @negative
  Scenario: User missing required permission
    Given a principal "user_no_write"
    When they perform action "edit" on resource "document"
    Then the decision should be "deny"

  @array-length @positive
  Scenario: User has enough skills
    Given a principal "user_many_skills"
    When they perform action "apply" on resource "senior_position"
    Then the decision should be "allow"

  @array-length @negative
  Scenario: User has insufficient skills
    Given a principal "user_few_skills"
    When they perform action "apply" on resource "senior_position"
    Then the decision should be "deny"

  @set-intersection @positive
  Scenario: User groups overlap with required groups
    Given a principal "user_overlap_groups"
    When they perform action "access" on resource "shared_resource"
    Then the decision should be "allow"

  @set-intersection @negative
  Scenario: User groups do not overlap with required groups
    Given a principal "user_no_overlap"
    When they perform action "access" on resource "shared_resource"
    Then the decision should be "deny"

  @set-subset @positive
  Scenario: User tags are subset of allowed tags
    Given a principal "user_allowed_tags"
    When they perform action "tag" on resource "content"
    Then the decision should be "allow"

  @set-subset @negative
  Scenario: User has forbidden tags
    Given a principal "user_forbidden_tags"
    When they perform action "tag" on resource "content"
    Then the decision should be "deny"

  @array-any @positive
  Scenario: User has at least one admin role
    Given a principal "user_any_admin"
    When they perform action "manage" on resource "system"
    Then the decision should be "allow"

  @array-any @negative
  Scenario: User has no admin roles
    Given a principal "user_no_admin"
    When they perform action "manage" on resource "system"
    Then the decision should be "deny"

  @array-all @positive
  Scenario: All user projects are active
    Given a principal "user_all_active"
    When they perform action "bill" on resource "invoice"
    Then the decision should be "allow"

  @array-all @negative
  Scenario: Some user projects are inactive
    Given a principal "user_some_inactive"
    When they perform action "bill" on resource "invoice"
    Then the decision should be "deny"

  @map-keys @positive
  Scenario: User has required metadata keys
    Given a principal "user_complete_metadata"
    When they perform action "publish" on resource "profile"
    Then the decision should be "allow"

  @map-keys @negative
  Scenario: User missing required metadata keys
    Given a principal "user_incomplete_metadata"
    When they perform action "publish" on resource "profile"
    Then the decision should be "deny"

  @comprehension-filter @positive
  Scenario: User has verified email addresses
    Given a principal "user_verified_emails"
    When they perform action "send_bulk" on resource "email_campaign"
    Then the decision should be "allow"

  @comprehension-filter @negative
  Scenario: User has unverified email addresses
    Given a principal "user_unverified_emails"
    When they perform action "send_bulk" on resource "email_campaign"
    Then the decision should be "deny"

  @nested-array @positive
  Scenario: User has nested permission structure
    Given a principal "user_nested_perms"
    When they perform action "execute" on resource "workflow"
    Then the decision should be "allow"

  @nested-array @negative
  Scenario: User lacks nested permission
    Given a principal "user_no_nested_perms"
    When they perform action "execute" on resource "workflow"
    Then the decision should be "deny"

  @performance @collections
  Scenario: Collection operations evaluate efficiently
    Given a principal "user_with_read"
    When they perform 1000 evaluations on random resources
    Then the average evaluation time should be less than 100 microseconds

  Scenario Outline: Various collection operation scenarios
    Given a principal "<user>"
    When they perform action "<action>" on resource "<resource>"
    Then the decision should be "<decision>"

    Examples:
      | user                    | action      | resource        | decision |
      | user_with_read          | view        | document        | allow    |
      | user_no_write           | edit        | document        | deny     |
      | user_many_skills        | apply       | senior_position | allow    |
      | user_few_skills         | apply       | senior_position | deny     |
      | user_overlap_groups     | access      | shared_resource | allow    |
      | user_no_overlap         | access      | shared_resource | deny     |
      | user_allowed_tags       | tag         | content         | allow    |
      | user_forbidden_tags     | tag         | content         | deny     |
