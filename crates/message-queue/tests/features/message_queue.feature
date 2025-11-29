Feature: Message Queue
  As a developer
  I want to send and receive messages
  So that components can communicate asynchronously

  Scenario: Send a message
    Given a message queue
    When I send a message
    Then the message should be delivered
