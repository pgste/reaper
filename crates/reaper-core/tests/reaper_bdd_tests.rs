//! BDD Tests for Reaper Core

use cucumber::{given, then, when, World};
use reaper_core::*;

#[derive(Debug, World)]
#[world(init = Self::new)]
struct ReaperWorld {
    last_error: Option<ReaperError>,
    current_policy: Option<String>,
    evaluation_result: Option<String>,
}

impl ReaperWorld {
    fn new() -> Self {
        Self {
            last_error: None,
            current_policy: None,
            evaluation_result: None,
        }
    }
}

#[given("a running Reaper Agent")]
async fn given_running_agent(_world: &mut ReaperWorld) {
    // Agent setup will go here
}

#[given("a policy that {word} all requests")]
async fn given_policy_that_action_all(world: &mut ReaperWorld, action: String) {
    world.current_policy = Some(action);
}

#[when("I evaluate a request against the policy")]
async fn when_evaluate_request(world: &mut ReaperWorld) {
    world.evaluation_result = world.current_policy.clone();
}

#[when("I evaluate a request against a non-existent policy")]
async fn when_evaluate_nonexistent_policy(world: &mut ReaperWorld) {
    world.last_error = Some(ReaperError::PolicyNotFound {
        policy_id: "non-existent".to_string(),
    });
}

#[then("the decision should be {string}")]
async fn then_decision_should_be(world: &mut ReaperWorld, expected: String) {
    assert_eq!(world.evaluation_result.as_ref().unwrap(), &expected);
}

#[then("the response time should be under {int}ms")]
async fn then_response_time_under_ms(_world: &mut ReaperWorld, _max_ms: u32) {
    // Performance verification will go here
}

#[then("I should get a {string} error")]
async fn then_should_get_error(world: &mut ReaperWorld, error_type: String) {
    assert!(world.last_error.is_some());
    let error = world.last_error.as_ref().unwrap();
    assert!(error.to_string().contains(&error_type.replace('_', " ")));
}

#[then("the error should include the policy ID")]
async fn then_error_should_include_policy_id(world: &mut ReaperWorld) {
    assert!(world.last_error.is_some());
    let error = world.last_error.as_ref().unwrap();
    assert!(error.to_string().contains("non-existent"));
}

#[tokio::main]
async fn main() {
    ReaperWorld::run("tests/features").await;
}
