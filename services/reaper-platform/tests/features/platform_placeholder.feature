Feature: Platform Test Placeholder
  Placeholder for platform integration tests

  Scenario: Integration tests info
    Given integration tests require running services
    When the services are not available
    Then tests are skipped gracefully
