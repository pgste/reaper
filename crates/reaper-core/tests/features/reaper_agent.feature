Feature: Reaper Agent Policy Enforcement
    As a system administrator
    I want the Reaper Agent to enforce policies on requests
    So that I can control access and behavior

    Background:
        Given a running Reaper Agent

    Scenario: Basic policy enforcement
        Given a policy that allows all requests
        When I evaluate a request against the policy
        Then the decision should be "allow"
        And the response time should be under 1ms

    Scenario: Policy not found handling
        When I evaluate a request against a non-existent policy
        Then I should get a "policy_not_found" error
        And the error should include the policy ID