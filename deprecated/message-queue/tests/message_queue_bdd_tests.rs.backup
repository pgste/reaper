use cucumber::{given, then, when, World};

#[derive(Debug, World)]
#[world(init = Self::new)]
struct MessageQueueWorld {
    // Test state will be stored here
}

impl MessageQueueWorld {
    fn new() -> Self {
        Self {}
    }
}

#[given("a message queue")]
async fn given_message_queue(_world: &mut MessageQueueWorld) {
    // Setup will go here
}

#[when("I send a message")]
async fn when_send_message(_world: &mut MessageQueueWorld) {
    // Action will go here
}

#[then("the message should be delivered")]
async fn then_message_delivered(_world: &mut MessageQueueWorld) {
    // Verification will go here
}

#[tokio::main]
async fn main() {
    MessageQueueWorld::run("tests/features").await;
}
