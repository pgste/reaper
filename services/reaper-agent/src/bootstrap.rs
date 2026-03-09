//! Bootstrap Loading - Load Policies and Data on Startup
//!
//! Loads policies from a bootstrap directory and entity data from files
//! when the agent starts up. Supports .reap, .yaml, .yml, and .json formats.

use policy_engine::{DataLoader, DataStore, EnhancedPolicy, PolicyEngine, ReaperPolicy};
use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;
use tracing::{debug, error, info, warn};

/// Bootstrap loading errors
#[derive(Debug, Error)]
pub enum BootstrapError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Policy parsing error: {0}")]
    PolicyParse(String),
    #[error("Data loading error: {0}")]
    DataLoad(String),
    #[error("Directory not found: {0}")]
    #[allow(dead_code)]
    DirectoryNotFound(PathBuf),
}

/// Result of bootstrap loading
#[derive(Debug, Default)]
pub struct BootstrapResult {
    pub policies_loaded: usize,
    pub policies_failed: usize,
    pub entities_loaded: usize,
    pub data_files_loaded: usize,
    pub data_files_failed: usize,
}

/// Load bootstrap policies from a directory
///
/// Supports file formats:
/// - `.reap` - Reaper DSL format
/// - `.yaml` / `.yml` - YAML policy format
/// - `.json` - JSON policy format
///
/// Each policy file is loaded, compiled, and deployed to the engine.
pub async fn load_bootstrap_policies(
    engine: &PolicyEngine,
    data_store: Arc<DataStore>,
    bootstrap_dir: Option<PathBuf>,
) -> Result<BootstrapResult, BootstrapError> {
    let Some(dir) = bootstrap_dir else {
        debug!("No bootstrap policy directory configured");
        return Ok(BootstrapResult::default());
    };

    if !dir.exists() {
        warn!("Bootstrap policy directory does not exist: {:?}", dir);
        return Ok(BootstrapResult::default());
    }

    info!("Loading bootstrap policies from {:?}", dir);

    let mut result = BootstrapResult::default();
    let mut entries = tokio::fs::read_dir(&dir).await?;

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();

        // Skip directories
        if path.is_dir() {
            continue;
        }

        // Check file extension
        let ext = path.extension().and_then(|s| s.to_str());

        match ext {
            Some("reap") => match load_reap_policy(&path, engine, data_store.clone()).await {
                Ok(policy_name) => {
                    info!(
                        "Loaded bootstrap .reap policy: {} from {:?}",
                        policy_name, path
                    );
                    result.policies_loaded += 1;
                }
                Err(e) => {
                    warn!("Failed to load .reap policy from {:?}: {}", path, e);
                    result.policies_failed += 1;
                }
            },
            Some("yaml") | Some("yml") | Some("json") => {
                match load_declarative_policy(&path, engine).await {
                    Ok(policy_name) => {
                        info!("Loaded bootstrap policy: {} from {:?}", policy_name, path);
                        result.policies_loaded += 1;
                    }
                    Err(e) => {
                        warn!("Failed to load policy from {:?}: {}", path, e);
                        result.policies_failed += 1;
                    }
                }
            }
            _ => {
                debug!("Skipping non-policy file: {:?}", path);
            }
        }
    }

    info!(
        "Bootstrap policies: {} loaded, {} failed",
        result.policies_loaded, result.policies_failed
    );

    Ok(result)
}

/// Load a .reap policy file and compile it
async fn load_reap_policy(
    path: &PathBuf,
    engine: &PolicyEngine,
    data_store: Arc<DataStore>,
) -> Result<String, BootstrapError> {
    use std::str::FromStr;

    let content = tokio::fs::read_to_string(path).await?;

    // Parse the .reap policy
    let reaper_policy = ReaperPolicy::from_str(&content)
        .map_err(|e| BootstrapError::PolicyParse(format!("Failed to parse .reap: {}", e)))?;

    let policy_name = reaper_policy.name().to_string();

    // Try compiled evaluator first for sub-microsecond performance
    // Fall back to AST evaluator for policies with advanced features (variables, comprehensions, etc.)
    let evaluator: Arc<dyn policy_engine::PolicyEvaluator> =
        match reaper_policy.clone().build(data_store.clone()) {
            Ok(compiled) => {
                debug!(
                    "Using compiled evaluator for policy: {} (sub-microsecond performance)",
                    policy_name
                );
                Arc::new(compiled)
            }
            Err(compile_err) => {
                // Compilation failed - use AST evaluator which supports all features
                info!(
                    "Using AST evaluator for policy: {} (full feature support). Compile hint: {}",
                    policy_name, compile_err
                );
                Arc::new(reaper_policy.build_ast_evaluator(data_store))
            }
        };

    // Create EnhancedPolicy with the evaluator
    let mut enhanced_policy = EnhancedPolicy {
        id: uuid::Uuid::new_v4(),
        version: 1,
        name: policy_name.clone(),
        description: format!("Bootstrap policy from {:?}", path),
        language: policy_engine::PolicyLanguage::Custom,
        content: content.clone(),
        rules: vec![],
        metadata: std::collections::HashMap::new(),
        priority: 100,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        evaluator: Some(evaluator),
        source_metadata: None,
    };

    // Set file source metadata
    enhanced_policy.set_file_source(&path.to_string_lossy(), Some("bootstrap".to_string()));

    // Deploy to engine
    engine
        .deploy_policy(enhanced_policy)
        .map_err(|e| BootstrapError::PolicyParse(format!("Failed to deploy policy: {}", e)))?;

    Ok(policy_name)
}

/// Load a YAML or JSON declarative policy file
async fn load_declarative_policy(
    path: &PathBuf,
    engine: &PolicyEngine,
) -> Result<String, BootstrapError> {
    let content = tokio::fs::read_to_string(path).await?;

    // Parse using ReaperPolicy which supports YAML/JSON
    let reaper_policy = if path.extension().and_then(|s| s.to_str()) == Some("json") {
        ReaperPolicy::from_json_str(&content)
            .map_err(|e| BootstrapError::PolicyParse(format!("Failed to parse JSON: {}", e)))?
    } else {
        ReaperPolicy::from_yaml_str(&content)
            .map_err(|e| BootstrapError::PolicyParse(format!("Failed to parse YAML: {}", e)))?
    };

    let policy_name = reaper_policy.name().to_string();

    // Create EnhancedPolicy using the new_with_language constructor
    // which will build the appropriate evaluator from content
    let mut enhanced_policy = EnhancedPolicy::new_with_language(
        policy_name.clone(),
        format!("Bootstrap policy from {:?}", path),
        policy_engine::PolicyLanguage::Simple,
        content.clone(),
    )
    .map_err(|e| BootstrapError::PolicyParse(format!("Failed to create policy: {}", e)))?;

    // Set file source metadata
    enhanced_policy.set_file_source(&path.to_string_lossy(), Some("bootstrap".to_string()));

    // Deploy to engine
    engine
        .deploy_policy(enhanced_policy)
        .map_err(|e| BootstrapError::PolicyParse(format!("Failed to deploy policy: {}", e)))?;

    Ok(policy_name)
}

/// Load bootstrap entity data from a file or directory
///
/// Supports:
/// - Single JSON file with entity array
/// - Directory containing multiple JSON files
pub async fn load_bootstrap_data(
    data_store: Arc<DataStore>,
    bootstrap_file: Option<PathBuf>,
    bootstrap_dir: Option<PathBuf>,
) -> Result<BootstrapResult, BootstrapError> {
    let mut result = BootstrapResult::default();

    // Load from single file if specified
    if let Some(ref file_path) = bootstrap_file {
        if file_path.exists() {
            match load_data_file(data_store.clone(), file_path).await {
                Ok(count) => {
                    info!("Loaded {} entities from {:?}", count, file_path);
                    result.entities_loaded += count;
                    result.data_files_loaded += 1;
                }
                Err(e) => {
                    warn!("Failed to load data from {:?}: {}", file_path, e);
                    result.data_files_failed += 1;
                }
            }
        } else {
            warn!("Bootstrap data file does not exist: {:?}", file_path);
        }
    }

    // Load from directory if specified
    if let Some(ref dir) = bootstrap_dir {
        if dir.exists() {
            let mut entries = tokio::fs::read_dir(dir).await?;

            while let Some(entry) = entries.next_entry().await? {
                let path = entry.path();

                // Only process .json files
                if path.extension().and_then(|s| s.to_str()) != Some("json") {
                    continue;
                }

                match load_data_file(data_store.clone(), &path).await {
                    Ok(count) => {
                        debug!("Loaded {} entities from {:?}", count, path);
                        result.entities_loaded += count;
                        result.data_files_loaded += 1;
                    }
                    Err(e) => {
                        warn!("Failed to load data from {:?}: {}", path, e);
                        result.data_files_failed += 1;
                    }
                }
            }
        } else {
            warn!("Bootstrap data directory does not exist: {:?}", dir);
        }
    }

    if result.entities_loaded > 0 || result.data_files_loaded > 0 {
        info!(
            "Bootstrap data: {} entities from {} files ({} failed)",
            result.entities_loaded, result.data_files_loaded, result.data_files_failed
        );
    }

    Ok(result)
}

/// Load a single JSON data file
async fn load_data_file(
    data_store: Arc<DataStore>,
    path: &PathBuf,
) -> Result<usize, BootstrapError> {
    let content = tokio::fs::read_to_string(path).await?;

    // DataStore uses Arc internally, so cloning shares data
    let loader = DataLoader::new((*data_store).clone());
    let count = loader
        .load_json(&content)
        .map_err(|e| BootstrapError::DataLoad(format!("Failed to parse JSON data: {}", e)))?;

    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_load_bootstrap_policies_empty_dir() {
        let temp_dir = TempDir::new().unwrap();
        let engine = PolicyEngine::new();
        let data_store = Arc::new(DataStore::new());

        let result =
            load_bootstrap_policies(&engine, data_store, Some(temp_dir.path().to_path_buf()))
                .await
                .unwrap();

        assert_eq!(result.policies_loaded, 0);
        assert_eq!(result.policies_failed, 0);
    }

    #[tokio::test]
    async fn test_load_bootstrap_policies_no_dir() {
        let engine = PolicyEngine::new();
        let data_store = Arc::new(DataStore::new());

        let result = load_bootstrap_policies(&engine, data_store, None)
            .await
            .unwrap();

        assert_eq!(result.policies_loaded, 0);
    }

    #[tokio::test]
    async fn test_load_bootstrap_data_no_file() {
        let data_store = Arc::new(DataStore::new());

        let result = load_bootstrap_data(data_store, None, None).await.unwrap();

        assert_eq!(result.entities_loaded, 0);
    }
}
