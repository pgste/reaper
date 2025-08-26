use cucumber::{given, then, when, World};
use reaper_core::*;

#[derive(Debug, World)]
#[world(init = Self::new)]
struct PolicyEngineWorld {
    // Test state will be stored here
}

impl PolicyEngineWorld {
    fn new() -> Self {
        Self {}
    }
}

#[given("a policy engine")]
async fn given_policy_engine(_world: &mut PolicyEngineWorld) {
    // Setup will go here
}

#[when("I load a policy")]
async fn when_load_policy(_world: &mut PolicyEngineWorld) {
    // Action will go here
}

#[then("the policy should be ready")]
async fn then_policy_ready(_world: &mut PolicyEngineWorld) {
    // Verification will go here
}

#[tokio::main]
async fn main() {
    PolicyEngineWorld::run("tests/features").await;
}
