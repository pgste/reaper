// Test context for Cucumber tests
//
// Holds policy, data, and evaluation state across test steps

use crate::data::{DataStore, DataLoader};
use crate::reap::ReaperPolicy;
use crate::evaluators::PolicyEvaluator;
use crate::engine::PolicyRequest;
use std::sync::Arc;
use std::collections::HashMap;

/// Test context holding policy and data state
pub struct TestContext {
    pub policy: Option<ReaperPolicy>,
    pub evaluator: Option<Box<dyn PolicyEvaluator>>,
    pub store: Option<Arc<DataStore>>,
    pub principal: Option<String>,
    pub action: Option<String>,
    pub resource: Option<String>,
    pub last_decision: Option<String>,
    pub evaluation_times: Vec<u128>,
}

impl TestContext {
    pub fn new() -> Self {
        Self {
            policy: None,
            evaluator: None,
            store: None,
            principal: None,
            action: None,
            resource: None,
            last_decision: None,
            evaluation_times: Vec::new(),
        }
    }

    pub fn load_policy(&mut self, path: &str) -> Result<(), String> {
        let policy = ReaperPolicy::from_file_auto(path)
            .map_err(|e| format!("Failed to load policy: {:?}", e))?;
        self.policy = Some(policy);
        Ok(())
    }

    pub fn load_data(&mut self, path: &str) -> Result<(), String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read data file: {}", e))?;

        let store = DataStore::new();
        let loader = DataLoader::new(store.clone());

        loader.load_json(&content)
            .map_err(|e| format!("Failed to load data: {:?}", e))?;

        let store_arc = Arc::new(store);
        self.store = Some(store_arc);
        Ok(())
    }

    pub fn build_evaluator(&mut self) -> Result<(), String> {
        let policy = self.policy.take()
            .ok_or("No policy loaded")?;
        let store = self.store.clone()
            .ok_or("No data loaded")?;

        let evaluator = policy.build(store)
            .map_err(|e| format!("Failed to build evaluator: {:?}", e))?;

        self.evaluator = Some(Box::new(evaluator));
        Ok(())
    }

    pub fn evaluate(&mut self) -> Result<String, String> {
        let evaluator = self.evaluator.as_ref()
            .ok_or("No evaluator built")?;

        let principal = self.principal.as_ref()
            .ok_or("No principal set")?;
        let action = self.action.as_ref()
            .ok_or("No action set")?;
        let resource = self.resource.as_ref()
            .ok_or("No resource set")?;

        let mut context = HashMap::new();
        context.insert("principal".to_string(), principal.clone());

        let request = PolicyRequest {
            resource: resource.clone(),
            action: action.clone(),
            context,
        };

        let start = std::time::Instant::now();
        let decision = evaluator.evaluate(&request)
            .map_err(|e| format!("Evaluation failed: {:?}", e))?;
        let elapsed = start.elapsed().as_nanos();

        self.evaluation_times.push(elapsed);

        let decision_str = format!("{:?}", decision);
        self.last_decision = Some(decision_str.clone());

        Ok(decision_str)
    }

    pub fn get_decision(&self) -> Result<&str, String> {
        self.last_decision.as_deref()
            .ok_or("No decision recorded".to_string())
    }

    pub fn average_evaluation_time(&self) -> u128 {
        if self.evaluation_times.is_empty() {
            0
        } else {
            self.evaluation_times.iter().sum::<u128>() / self.evaluation_times.len() as u128
        }
    }
}

impl Default for TestContext {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for TestContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestContext")
            .field("has_policy", &self.policy.is_some())
            .field("has_evaluator", &self.evaluator.is_some())
            .field("has_store", &self.store.is_some())
            .field("principal", &self.principal)
            .field("action", &self.action)
            .field("resource", &self.resource)
            .field("last_decision", &self.last_decision)
            .field("num_evaluations", &self.evaluation_times.len())
            .finish()
    }
}
