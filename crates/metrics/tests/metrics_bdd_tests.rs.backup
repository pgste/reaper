use cucumber::{given, then, when, World};

#[derive(Debug, World)]
#[world(init = Self::new)]
struct MetricsWorld {
    // Test state will be stored here
}

impl MetricsWorld {
    fn new() -> Self {
        Self {}
    }
}

#[given("a metrics collector")]
async fn given_metrics_collector(_world: &mut MetricsWorld) {
    // Setup will go here
}

#[when("I record a metric")]
async fn when_record_metric(_world: &mut MetricsWorld) {
    // Action will go here
}

#[then("the metric should be stored")]
async fn then_metric_stored(_world: &mut MetricsWorld) {
    // Verification will go here
}

#[tokio::main]
async fn main() {
    MetricsWorld::run("tests/features").await;
}
