Feature: Policy Definition and Storage
    As a system administrator
    I want to create, manage, and deploy policies
    So that I can control access with zero downtime

    Background:
        Given a running Reaper Platform on port 8081
        And a running Reaper Agent on port 8080

    Scenario: Create a new policy
        When I create a policy named "test-policy" with action "allow" for resource "*"
        Then the policy should be created successfully
        And the policy should be stored in the platform
        And the policy should have version 1

    Scenario: Hot-swap policy deployment
        Given a policy named "swap-test" exists
        When I deploy the policy to the agent
        Then the policy should be deployed successfully
        And the agent should have the policy available
        And there should be zero downtime during deployment

    Scenario: Policy evaluation with sub-microsecond performance
        Given a policy named "perf-test" with action "allow" for resource "test-resource"
        And the policy is deployed to the agent
        When I evaluate a request for resource "test-resource" with action "read"
        Then the decision should be "allow"
        And the evaluation should complete in under 1000 nanoseconds
        And the response should include evaluation timing

    Scenario: Policy updates with versioning
        Given a policy named "version-test" exists with version 1
        When I update the policy rules
        Then the policy version should increment to 2
        And the updated policy should be available immediately
        And old policy versions should be replaced atomically

    Scenario: Policy not found handling
        When I evaluate a request against a non-existent policy "missing-policy"
        Then I should get a "policy_not_found" error
        And the error should include the policy identifier
        And the agent should remain stable

    Scenario: Default policy fallback
        Given a default policy exists
        When I evaluate a request without specifying a policy
        Then the default policy should be used
        And the decision should be based on default policy rules

    Scenario: Policy deletion
        Given a policy named "delete-test" exists
        When I delete the policy
        Then the policy should be removed from storage
        And the policy should no longer be available for evaluation
        And subsequent requests should return policy not found