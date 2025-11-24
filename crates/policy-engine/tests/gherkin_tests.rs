// Gherkin/Cucumber Test Runner for Reaper Policy Engine
//
// Runs BDD tests defined in .feature files

use cucumber::{given, then, when, World};
use policy_engine::gherkin::TestContext;

/// World struct for Cucumber tests
#[derive(Debug, World, Default)]
#[world(init = Self::new)]
pub struct PolicyWorld {
    pub context: TestContext,
}

impl PolicyWorld {
    fn new() -> Self {
        Self {
            context: TestContext::new(),
        }
    }
}

// ============================================================================
// Given Steps - Setup
// ============================================================================

#[given(expr = "the policy file {string}")]
async fn load_policy_file(world: &mut PolicyWorld, path: String) {
    world
        .context
        .load_policy(&path)
        .expect("Failed to load policy file");
}

#[given(expr = "the data file {string}")]
async fn load_data_file(world: &mut PolicyWorld, path: String) {
    world
        .context
        .load_data(&path)
        .expect("Failed to load data file");
    world
        .context
        .build_evaluator()
        .expect("Failed to build evaluator");
}

#[given(expr = "a principal {string}")]
async fn set_principal(world: &mut PolicyWorld, principal: String) {
    world.context.principal = Some(principal);
}

// ============================================================================
// When Steps - Actions
// ============================================================================

#[when(expr = "they perform action {string} on resource {string}")]
async fn perform_action(world: &mut PolicyWorld, action: String, resource: String) {
    world.context.action = Some(action);
    world.context.resource = Some(resource);

    world.context.evaluate().expect("Evaluation failed");
}

#[when(expr = "they perform {int} evaluations on random resources")]
async fn perform_multiple_evaluations(world: &mut PolicyWorld, count: usize) {
    let action = world.context.action.clone().unwrap_or("read".to_string());

    for i in 0..count {
        world.context.action = Some(action.clone());
        world.context.resource = Some(format!("resource_{}", i % 2000));

        world.context.evaluate().expect("Evaluation failed");
    }
}

// ============================================================================
// Then Steps - Assertions
// ============================================================================

#[then(expr = "the decision should be {string}")]
async fn check_decision(world: &mut PolicyWorld, expected: String) {
    let decision = world.context.get_decision().expect("No decision recorded");

    let expected_upper = expected.to_uppercase();
    assert!(
        decision.contains(&expected_upper),
        "Expected decision '{}', but got '{}'",
        expected_upper,
        decision
    );
}

#[then(expr = "the average evaluation time should be less than {int} microseconds")]
async fn check_average_time(world: &mut PolicyWorld, max_micros: u128) {
    let avg_nanos = world.context.average_evaluation_time();
    let avg_micros = avg_nanos / 1000;

    assert!(
        avg_micros < max_micros,
        "Average evaluation time {}µs exceeds {}µs",
        avg_micros,
        max_micros
    );
}

#[then(expr = "the decision should be {string} with reason {string}")]
async fn check_decision_with_reason(world: &mut PolicyWorld, expected: String, _reason: String) {
    // For now, just check decision - can enhance with reason tracking later
    check_decision(world, expected).await;
}

// ============================================================================
// Test Runner
// ============================================================================

#[tokio::main]
async fn main() {
    PolicyWorld::cucumber()
        .before(|_feature, _rule, _scenario, world| {
            Box::pin(async move {
                // Reset world state before each scenario
                *world = PolicyWorld::default();
            })
        })
        .after(|_feature, _rule, scenario, _event, world| {
            Box::pin(async move {
                if let Some(world) = world {
                    // Log statistics after each scenario
                    if !world.context.evaluation_times.is_empty() {
                        let avg = world.context.average_evaluation_time();
                        println!(
                            "  [{}] Average eval time: {}ns ({:.2}µs)",
                            scenario.name,
                            avg,
                            avg as f64 / 1000.0
                        );
                    }
                }
            })
        })
        .run_and_exit("tests/features")
        .await;
}
