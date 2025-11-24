use cucumber::{given, then, when, World};

#[derive(Debug, World)]
#[world(init = Self::new)]
struct CliWorld {
    // Test state will be stored here
}

impl CliWorld {
    fn new() -> Self {
        Self {}
    }
}

#[given("a CLI interface")]
async fn given_cli_interface(_world: &mut CliWorld) {
    // Setup will go here
}

#[when("I run a command")]
async fn when_run_command(_world: &mut CliWorld) {
    // Action will go here
}

#[then("I should see output")]
async fn then_see_output(_world: &mut CliWorld) {
    // Verification will go here
}

#[tokio::main]
async fn main() {
    CliWorld::run("tests/features").await;
}
