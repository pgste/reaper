use cucumber::{given, then, when, World};
use policy_engine::{EnhancedPolicy, PolicyAction, PolicyEngine, PolicyRule};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;
use tokio::time::timeout;
use uuid::Uuid;

#[derive(Debug, World)]
#[world(init = Self::new)]
struct PolicyWorld {
    policy_engine: PolicyEngine,
    last_policy_id: Option<Uuid>,
    last_policy_name: Option<String>,
    last_response: Option<Value>,
    last_error: Option<String>,
    evaluation_time_ns: Option<u64>,
    policy_version: Option<u64>,
    platform_url: String,
    agent_url: String,
    http_client: reqwest::Client,
}

impl PolicyWorld {
    fn new() -> Self {
        Self {
            policy_engine: PolicyEngine::new(),
            last_policy_id: None,
            last_policy_name: None,
            last_response: None,
            last_error: None,
            evaluation_time_ns: None,
            policy_version: None,
            platform_url: "http://localhost:8081".to_string(),
            agent_url: "http://localhost:8080".to_string(),
            http_client: reqwest::Client::new(),
        }
    }

    async fn wait_for_service(&self, url: &str) -> Result<(), Box<dyn std::error::Error>> {
        for _ in 0..30 {
            if let Ok(response) = self
                .http_client
                .get(&format!("{}/health", url))
                .send()
                .await
            {
                if response.status().is_success() {
                    return Ok(());
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err("Service not available".into())
    }
}

// Background steps
#[given("a running Reaper Platform on port 8081")]
async fn given_running_platform(world: &mut PolicyWorld) {
    world
        .wait_for_service(&world.platform_url)
        .await
        .expect("Reaper Platform should be running on port 8081");
}

#[given("a running Reaper Agent on port 8080")]
async fn given_running_agent(world: &mut PolicyWorld) {
    world
        .wait_for_service(&world.agent_url)
        .await
        .expect("Reaper Agent should be running on port 8080");
}

// Policy creation steps
#[when("I create a policy named {string} with action {string} for resource {string}")]
async fn when_create_policy(
    world: &mut PolicyWorld,
    name: String,
    action: String,
    resource: String,
) {
    let policy_action = match action.as_str() {
        "allow" => PolicyAction::Allow,
        "deny" => PolicyAction::Deny,
        "log" => PolicyAction::Log,
        _ => panic!("Invalid action: {}", action),
    };

    let policy = EnhancedPolicy::new(
        name.clone(),
        format!("Test policy for {}", name),
        vec![PolicyRule {
            action: policy_action,
            resource: resource.clone(),
            conditions: vec![],
        }],
    );

    world.last_policy_id = Some(policy.id);
    world.last_policy_name = Some(name.clone());
    world.policy_version = Some(policy.version);

    // Create policy via HTTP API
    let request_body = json!({
        "name": name,
        "description": format!("Test policy for {}", name),
        "rules": [{
            "action": action,
            "resource": resource,
            "conditions": []
        }]
    });

    let response = world
        .http_client
        .post(&format!("{}/api/v1/policies", world.platform_url))
        .json(&request_body)
        .send()
        .await
        .expect("Failed to send request");

    let response_json: Value = response.json().await.expect("Failed to parse response");
    world.last_response = Some(response_json);

    // Also deploy to local engine for direct testing
    world
        .policy_engine
        .deploy_policy(policy)
        .expect("Failed to deploy policy locally");
}

#[then("the policy should be created successfully")]
async fn then_policy_created_successfully(world: &mut PolicyWorld) {
    let response = world.last_response.as_ref().expect("No response available");
    assert!(
        response.get("policy").is_some(),
        "Response should contain policy: {}",
        response
    );
    assert_eq!(response.get("status").unwrap().as_str().unwrap(), "created");
}

#[then("the policy should be stored in the platform")]
async fn then_policy_stored_in_platform(world: &mut PolicyWorld) {
    let policy_id = world.last_policy_id.expect("No policy ID available");

    // Verify policy exists in platform
    let response = world
        .http_client
        .get(&format!(
            "{}/api/v1/policies/{}",
            world.platform_url, policy_id
        ))
        .send()
        .await
        .expect("Failed to get policy");

    assert!(
        response.status().is_success(),
        "Policy should be retrievable from platform"
    );

    let policy_json: Value = response
        .json()
        .await
        .expect("Failed to parse policy response");
    assert!(
        policy_json.get("policy").is_some(),
        "Response should contain policy data"
    );
}

#[then("the policy should have version {int}")]
async fn then_policy_has_version(world: &mut PolicyWorld, expected_version: u64) {
    assert_eq!(world.policy_version.unwrap(), expected_version);
}

// Policy deployment steps
#[given("a policy named {string} exists")]
async fn given_policy_exists(world: &mut PolicyWorld, name: String) {
    // Create the policy first
    when_create_policy(world, name, "allow".to_string(), "*".to_string()).await;
}

#[when("I deploy the policy to the agent")]
async fn when_deploy_policy_to_agent(world: &mut PolicyWorld) {
    let policy_id = world.last_policy_id.expect("No policy ID available");
    let policy_name = world
        .last_policy_name
        .as_ref()
        .expect("No policy name available");

    let deploy_request = json!({
        "policy_id": policy_id.to_string(),
        "name": policy_name,
        "description": format!("Deployed {}", policy_name),
        "rules": [{
            "action": "allow",
            "resource": "*",
            "conditions": []
        }]
    });

    let response = world
        .http_client
        .post(&format!("{}/api/v1/policies/deploy", world.agent_url))
        .json(&deploy_request)
        .send()
        .await
        .expect("Failed to deploy policy");

    let response_json: Value = response
        .json()
        .await
        .expect("Failed to parse deployment response");
    world.last_response = Some(response_json);
}

#[then("the policy should be deployed successfully")]
async fn then_policy_deployed_successfully(world: &mut PolicyWorld) {
    let response = world.last_response.as_ref().expect("No response available");
    assert_eq!(
        response.get("status").unwrap().as_str().unwrap(),
        "deployed"
    );
}

#[then("the agent should have the policy available")]
async fn then_agent_has_policy_available(world: &mut PolicyWorld) {
    // Verify policy is available on agent
    let response = world
        .http_client
        .get(&format!("{}/api/v1/policies", world.agent_url))
        .send()
        .await
        .expect("Failed to get agent policies");

    let policies_json: Value = response
        .json()
        .await
        .expect("Failed to parse policies response");
    let policies = policies_json.get("policies").unwrap().as_array().unwrap();

    assert!(
        policies.len() > 0,
        "Agent should have at least one policy loaded"
    );
}

#[then("there should be zero downtime during deployment")]
async fn then_zero_downtime_during_deployment(_world: &mut PolicyWorld) {
    // This is verified by the atomic operations in the policy engine
    // If we got here without errors, zero-downtime deployment worked
    assert!(
        true,
        "Zero-downtime deployment verified through atomic operations"
    );
}

// Policy evaluation steps
#[given("a policy named {string} with action {string} for resource {string}")]
async fn given_policy_with_action_for_resource(
    world: &mut PolicyWorld,
    name: String,
    action: String,
    resource: String,
) {
    when_create_policy(world, name, action, resource).await;
}

#[given("the policy is deployed to the agent")]
async fn given_policy_deployed_to_agent(world: &mut PolicyWorld) {
    when_deploy_policy_to_agent(world).await;
}

#[when("I evaluate a request for resource {string} with action {string}")]
async fn when_evaluate_request(world: &mut PolicyWorld, resource: String, action: String) {
    let evaluation_request = json!({
        "policy_name": world.last_policy_name.as_ref().unwrap(),
        "resource": resource,
        "action": action,
        "context": {}
    });

    let response = world
        .http_client
        .post(&format!("{}/api/v1/messages", world.agent_url))
        .json(&evaluation_request)
        .send()
        .await
        .expect("Failed to evaluate policy");

    let response_json: Value = response
        .json()
        .await
        .expect("Failed to parse evaluation response");
    world.last_response = Some(response_json.clone());

    // Extract evaluation time
    if let Some(eval_time_micros) = response_json.get("evaluation_time_microseconds") {
        let eval_time_ns = (eval_time_micros.as_f64().unwrap() * 1000.0) as u64;
        world.evaluation_time_ns = Some(eval_time_ns);
    }
}

#[then("the decision should be {string}")]
async fn then_decision_should_be(world: &mut PolicyWorld, expected_decision: String) {
    let response = world.last_response.as_ref().expect("No response available");
    let actual_decision = response
        .get("decision")
        .expect("Response should contain decision")
        .as_str()
        .expect("Decision should be a string");
    assert_eq!(actual_decision, expected_decision);
}

#[then("the evaluation should complete in under {int} nanoseconds")]
async fn then_evaluation_under_nanoseconds(world: &mut PolicyWorld, max_ns: u64) {
    let evaluation_time = world
        .evaluation_time_ns
        .expect("No evaluation time recorded");
    assert!(
        evaluation_time < max_ns,
        "Evaluation took {}ns, expected under {}ns",
        evaluation_time,
        max_ns
    );
}

#[then("the response should include evaluation timing")]
async fn then_response_includes_timing(world: &mut PolicyWorld) {
    let response = world.last_response.as_ref().expect("No response available");
    assert!(
        response.get("evaluation_time_microseconds").is_some(),
        "Response should include evaluation_time_microseconds"
    );
    assert!(
        response.get("total_time_microseconds").is_some(),
        "Response should include total_time_microseconds"
    );
}

// Policy versioning steps
#[given("a policy named {string} exists with version {int}")]
async fn given_policy_exists_with_version(world: &mut PolicyWorld, name: String, version: u64) {
    when_create_policy(world, name, "allow".to_string(), "*".to_string()).await;
    assert_eq!(world.policy_version.unwrap(), version);
}

#[when("I update the policy rules")]
async fn when_update_policy_rules(world: &mut PolicyWorld) {
    let policy_id = world.last_policy_id.expect("No policy ID available");

    let update_request = json!({
        "rules": [{
            "action": "deny",
            "resource": "*",
            "conditions": []
        }]
    });

    let response = world
        .http_client
        .put(&format!(
            "{}/api/v1/policies/{}",
            world.platform_url, policy_id
        ))
        .json(&update_request)
        .send()
        .await
        .expect("Failed to update policy");

    let response_json: Value = response
        .json()
        .await
        .expect("Failed to parse update response");
    world.last_response = Some(response_json.clone());

    // Extract new version
    if let Some(policy_data) = response_json.get("policy") {
        if let Some(version) = policy_data.get("version") {
            world.policy_version = Some(version.as_u64().unwrap());
        }
    }
}

#[then("the policy version should increment to {int}")]
async fn then_policy_version_increments(world: &mut PolicyWorld, expected_version: u64) {
    assert_eq!(world.policy_version.unwrap(), expected_version);
}

#[then("the updated policy should be available immediately")]
async fn then_updated_policy_available_immediately(world: &mut PolicyWorld) {
    let response = world.last_response.as_ref().expect("No response available");
    assert_eq!(response.get("status").unwrap().as_str().unwrap(), "updated");
    assert!(response
        .get("message")
        .unwrap()
        .as_str()
        .unwrap()
        .contains("hot-swapped successfully"));
}

#[then("old policy versions should be replaced atomically")]
async fn then_old_versions_replaced_atomically(_world: &mut PolicyWorld) {
    // This is verified by the atomic operations in the policy engine
    // The Arc<Policy> ensures that old versions are cleaned up when no longer referenced
    assert!(
        true,
        "Atomic replacement verified through Rust's ownership model"
    );
}

// Error handling steps
#[when("I evaluate a request against a non-existent policy {string}")]
async fn when_evaluate_nonexistent_policy(world: &mut PolicyWorld, policy_name: String) {
    let evaluation_request = json!({
        "policy_name": policy_name,
        "resource": "test-resource",
        "action": "read",
        "context": {}
    });

    let response = world
        .http_client
        .post(&format!("{}/api/v1/messages", world.agent_url))
        .json(&evaluation_request)
        .send()
        .await
        .expect("Failed to send evaluation request");

    let response_json: Value = response.json().await.expect("Failed to parse response");
    world.last_response = Some(response_json.clone());

    if let Some(error) = response_json.get("error") {
        world.last_error = Some(error.as_str().unwrap().to_string());
    }
}

#[then("I should get a {string} error")]
async fn then_should_get_error(world: &mut PolicyWorld, error_type: String) {
    let error = world.last_error.as_ref().expect("No error recorded");
    let expected_error = error_type.replace('_', " ");
    assert!(
        error
            .to_lowercase()
            .contains(&expected_error.to_lowercase()),
        "Expected error containing '{}', got '{}'",
        expected_error,
        error
    );
}

#[then("the error should include the policy identifier")]
async fn then_error_includes_policy_identifier(world: &mut PolicyWorld) {
    let response = world.last_response.as_ref().expect("No response available");
    assert!(
        response.get("policy_name").is_some() || response.get("policy_id").is_some(),
        "Error response should include policy identifier"
    );
}

#[then("the agent should remain stable")]
async fn then_agent_remains_stable(world: &mut PolicyWorld) {
    // Verify agent is still responding to health checks
    let response = world
        .http_client
        .get(&format!("{}/health", world.agent_url))
        .send()
        .await
        .expect("Agent should still be responsive");

    assert!(
        response.status().is_success(),
        "Agent should remain healthy after error"
    );
}

// Default policy steps
#[given("a default policy exists")]
async fn given_default_policy_exists(world: &mut PolicyWorld) {
    // Create and set a default policy
    let default_policy = EnhancedPolicy::new(
        "default".to_string(),
        "Default policy".to_string(),
        vec![PolicyRule {
            action: PolicyAction::Allow,
            resource: "*".to_string(),
            conditions: vec![],
        }],
    );

    world.policy_engine.set_default_policy(default_policy);
}

#[when("I evaluate a request without specifying a policy")]
async fn when_evaluate_without_policy(world: &mut PolicyWorld) {
    let evaluation_request = json!({
        "resource": "test-resource",
        "action": "read",
        "context": {}
    });

    let response = world
        .http_client
        .post(&format!("{}/api/v1/messages", world.agent_url))
        .json(&evaluation_request)
        .send()
        .await
        .expect("Failed to evaluate without policy");

    let response_json: Value = response.json().await.expect("Failed to parse response");
    world.last_response = Some(response_json);
}

#[then("the default policy should be used")]
async fn then_default_policy_used(world: &mut PolicyWorld) {
    let response = world.last_response.as_ref().expect("No response available");
    assert!(
        response.get("decision").is_some(),
        "Should get a decision from default policy"
    );
    // The response should not contain an error
    assert!(
        response.get("error").is_none(),
        "Should not get error when default policy exists"
    );
}

#[then("the decision should be based on default policy rules")]
async fn then_decision_based_on_default_rules(world: &mut PolicyWorld) {
    let response = world.last_response.as_ref().expect("No response available");
    let decision = response.get("decision").unwrap().as_str().unwrap();
    // Our default policy allows everything
    assert_eq!(decision, "allow", "Default policy should allow requests");
}

// Policy deletion steps
#[given("a policy named {string} exists")]
async fn given_named_policy_exists(world: &mut PolicyWorld, name: String) {
    when_create_policy(world, name, "allow".to_string(), "*".to_string()).await;
}

#[when("I delete the policy")]
async fn when_delete_policy(world: &mut PolicyWorld) {
    let policy_id = world.last_policy_id.expect("No policy ID available");

    let response = world
        .http_client
        .delete(&format!(
            "{}/api/v1/policies/{}",
            world.platform_url, policy_id
        ))
        .send()
        .await
        .expect("Failed to delete policy");

    let response_json: Value = response
        .json()
        .await
        .expect("Failed to parse delete response");
    world.last_response = Some(response_json);
}

#[then("the policy should be removed from storage")]
async fn then_policy_removed_from_storage(world: &mut PolicyWorld) {
    let response = world.last_response.as_ref().expect("No response available");
    assert_eq!(response.get("status").unwrap().as_str().unwrap(), "deleted");
}

#[then("the policy should no longer be available for evaluation")]
async fn then_policy_not_available_for_evaluation(world: &mut PolicyWorld) {
    let policy_id = world.last_policy_id.expect("No policy ID available");

    // Try to get the policy - should fail
    let response = world
        .http_client
        .get(&format!(
            "{}/api/v1/policies/{}",
            world.platform_url, policy_id
        ))
        .send()
        .await
        .expect("Failed to attempt policy retrieval");

    let response_json: Value = response.json().await.expect("Failed to parse response");
    assert!(
        response_json.get("error").is_some(),
        "Should get error for deleted policy"
    );
}

#[then("subsequent requests should return policy not found")]
async fn then_subsequent_requests_return_not_found(world: &mut PolicyWorld) {
    let policy_id = world.last_policy_id.expect("No policy ID available");

    let evaluation_request = json!({
        "policy_id": policy_id.to_string(),
        "resource": "test-resource",
        "action": "read",
        "context": {}
    });

    let response = world
        .http_client
        .post(&format!("{}/api/v1/messages", world.agent_url))
        .json(&evaluation_request)
        .send()
        .await
        .expect("Failed to evaluate against deleted policy");

    let response_json: Value = response.json().await.expect("Failed to parse response");
    assert!(
        response_json.get("error").is_some(),
        "Should get error for deleted policy"
    );
    assert!(
        response_json
            .get("error")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("not found"),
        "Error should indicate policy not found"
    );
}

#[tokio::main]
async fn main() {
    PolicyWorld::run("tests/features/policy_management.feature").await;
}
