Feature: Metrics Collection
  As a developer
  I want to collect and store metrics
  So that I can monitor system performance

  Scenario: Record a metric
    Given a metrics collector
    When I record a metric
    Then the metric should be stored
