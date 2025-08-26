use cucumber::{given, then, when, World};

#[derive(Debug, World)]
#[world(init = Self::new)]
struct AgentWorld {
    // Test state will be stored here
}

impl AgentWorld {
    fn new() -> Self {
        Self {}
    }
}

#[given("a running agent")]
async fn given_running_agent(_world: &mut AgentWorld) {
    // Setup will go here
}

#[when("I send a request")]
async fn when_send_request(_world: &mut AgentWorld) {
    // Action will go here
}

#[then("I should get a response")]
async fn then_get_response(_world: &mut AgentWorld) {
    // Verification will go here
}

#[tokio::main]
async fn main() {
    AgentWorld::run("tests/features").await;
}
