Feature: Reaper Agent
  As a system
  I want to process policy requests
  So that I can enforce authorization decisions

  Scenario: Process a request
    Given a running agent
    When I send a request
    Then I should get a response
