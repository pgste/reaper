//! Git repository synchronization
//!
//! Clones/pulls Git repositories and extracts policy files.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;
use tracing::{debug, info, warn};

use super::commit_verify::{verify_commit_signature, CommitVerifyError};
use super::github_app::{GitHubAppClient, GitHubAppError};
use crate::domain::source::{GitConfig, PolicySource, SyncResult};

/// Git sync errors
#[derive(Debug, Error)]
pub enum GitSyncError {
    #[error("Git error: {0}")]
    Git(#[from] git2::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Configuration error: {0}")]
    Config(String),
    #[error("Pattern error: {0}")]
    Pattern(String),
    #[error("Remote URL not allowed: {0}")]
    UrlNotAllowed(String),
    #[error("GitHub App error: {0}")]
    GitHubApp(#[from] GitHubAppError),
    #[error("Commit signature verification failed: {0}")]
    Signature(#[from] CommitVerifyError),
}

/// Env flag that permits `file://` / local-path remotes. Test fixtures and
/// air-gapped mirror setups only — a local remote bypasses the SSRF guard by
/// definition, so this must never be set on an internet-facing control plane.
const ALLOW_LOCAL_GIT_ENV: &str = "REAPER_SYNC_ALLOW_LOCAL_GIT";

/// SSRF guard on the git remote (Plan 09 Step 4, Security P1-3): a source URL
/// is org-admin-controlled input, so before ANY clone/fetch it must pass the
/// same shared guard the JWKS/OIDC paths use — https only, host resolves, and
/// no address in a disallowed range (loopback/private/link-local/metadata/
/// CGNAT). `http://`, `git://`, and `ssh://` are rejected outright (the syncer
/// only implements userpass auth, which must not travel in cleartext), and
/// local paths are rejected unless explicitly opted in for test fixtures.
async fn guard_remote_url(url: &str) -> Result<(), GitSyncError> {
    let is_local = url.starts_with("file://") || std::path::Path::new(url).is_absolute();
    if is_local {
        let allowed = std::env::var(ALLOW_LOCAL_GIT_ENV)
            .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes" | "on"))
            .unwrap_or(false);
        if allowed {
            warn!(
                url = %url,
                "Allowing LOCAL git remote because {ALLOW_LOCAL_GIT_ENV} is set — \
                 test/air-gapped use only"
            );
            return Ok(());
        }
        return Err(GitSyncError::UrlNotAllowed(
            "local git remotes are disabled (set {REAPER_SYNC_ALLOW_LOCAL_GIT} for test fixtures)"
                .replace("{REAPER_SYNC_ALLOW_LOCAL_GIT}", ALLOW_LOCAL_GIT_ENV),
        ));
    }

    crate::url_guard::validate_public_https_url(url)
        .await
        .map_err(|crate::url_guard::UrlGuardError::NotAllowed(reason)| {
            GitSyncError::UrlNotAllowed(reason)
        })
}

/// Resolved clone credential for one sync (never stored). `x-access-token` +
/// a short-lived GitHub App installation token, or the source's configured
/// userpass, or none.
type GitCred = Option<(String, String)>;

/// Git repository syncer
pub struct GitSyncer {
    /// Base directory for cloned repositories
    base_path: PathBuf,
    /// GitHub App client for minting installation tokens (Plan 09 Step 6).
    /// `None` when no App is configured — the syncer then falls back to the
    /// source's configured userpass credentials.
    app_client: Option<Arc<GitHubAppClient>>,
}

impl GitSyncer {
    /// Create a new Git syncer
    pub fn new(base_path: impl AsRef<Path>) -> Self {
        Self {
            base_path: base_path.as_ref().to_path_buf(),
            app_client: None,
        }
    }

    /// Attach a GitHub App client so sources carrying an `installation_id`
    /// clone with a freshly minted, short-lived installation token instead of
    /// a stored credential (Plan 09 Step 6).
    pub fn with_app_client(mut self, app_client: Option<Arc<GitHubAppClient>>) -> Self {
        self.app_client = app_client;
        self
    }

    /// Resolve the clone URL and credential for this sync. For a GitHub App
    /// installation, mint a short-lived token now and build the standard https
    /// URL from `repo_full_name` — no long-lived credential is ever persisted.
    async fn resolve_auth(&self, config: &GitConfig) -> Result<(String, GitCred), GitSyncError> {
        if let (Some(installation_id), Some(repo_full_name)) =
            (&config.installation_id, &config.repo_full_name)
        {
            let app = self.app_client.as_ref().ok_or_else(|| {
                GitSyncError::Config(
                    "source uses a GitHub App installation but no App is configured".to_string(),
                )
            })?;
            let minted = app.installation_token(installation_id).await?;
            let url = format!("https://github.com/{repo_full_name}.git");
            return Ok((url, Some(("x-access-token".to_string(), minted.token))));
        }

        // Legacy / non-App path: use the stored userpass if present.
        let cred = config
            .username
            .as_ref()
            .map(|u| (u.clone(), config.password.clone().unwrap_or_default()));
        Ok((config.url.clone(), cred))
    }

    /// Sync a policy source
    pub async fn sync(&self, source: &PolicySource) -> Result<SyncResult, GitSyncError> {
        let start = std::time::Instant::now();

        let config = source
            .git_config()
            .ok_or_else(|| GitSyncError::Config("Invalid Git configuration".to_string()))?;

        // Resolve the effective clone URL + credential (mints an App
        // installation token when configured; never persists it).
        let (clone_url, cred) = self.resolve_auth(&config).await?;

        // SSRF guard BEFORE any network activity — clone and fetch both talk
        // to this URL. Checked on every sync (not just the first clone), so a
        // source whose config was edited to an internal address is re-blocked.
        guard_remote_url(&clone_url).await?;

        // Determine repo path
        let repo_path = self.repo_path(source.id);

        // Clone or update repository
        let (repo, is_new) = self.clone_or_open(&repo_path, &clone_url, &cred)?;

        // Fetch and checkout the branch
        let commit = self.update_repo(&repo, &config.branch, &cred)?;

        // Verify the HEAD commit signature when the source requires it — fail
        // the sync CLOSED before any policy file is read (Plan 09 Step 5).
        if config.require_signed_commits {
            self.verify_head_signature(&repo, &commit, &config.trusted_signing_keys)?;
        }

        // Find policy files matching patterns
        let policy_files = self.find_policy_files(&repo_path, &config)?;

        let duration_ms = start.elapsed().as_millis() as u64;

        info!(
            source_id = %source.id,
            commit = %commit,
            files_found = policy_files.len(),
            duration_ms = duration_ms,
            "Git sync completed"
        );

        Ok(SyncResult {
            source_id: source.id,
            success: true,
            policies_found: policy_files.len(),
            policies_updated: if is_new { 0 } else { policy_files.len() },
            policies_created: if is_new { policy_files.len() } else { 0 },
            commit: Some(commit),
            error: None,
            duration_ms,
        })
    }

    /// Get the path for a repository
    fn repo_path(&self, source_id: uuid::Uuid) -> PathBuf {
        self.base_path.join(source_id.to_string())
    }

    /// Build remote callbacks that authenticate with `cred` when present.
    fn callbacks_for(cred: &GitCred) -> git2::RemoteCallbacks<'static> {
        let mut callbacks = git2::RemoteCallbacks::new();
        if let Some((username, password)) = cred.clone() {
            callbacks.credentials(move |_url, _username_from_url, _allowed_types| {
                git2::Cred::userpass_plaintext(&username, &password)
            });
        }
        callbacks
    }

    /// Clone a new repository or open an existing one
    fn clone_or_open(
        &self,
        path: &Path,
        url: &str,
        cred: &GitCred,
    ) -> Result<(git2::Repository, bool), GitSyncError> {
        if path.exists() {
            debug!("Opening existing repository at {:?}", path);
            let repo = git2::Repository::open(path)?;
            Ok((repo, false))
        } else {
            info!("Cloning repository to {:?}", path);
            std::fs::create_dir_all(path)?;

            let mut builder = git2::build::RepoBuilder::new();
            let mut fetch_opts = git2::FetchOptions::new();
            fetch_opts.remote_callbacks(Self::callbacks_for(cred));
            builder.fetch_options(fetch_opts);

            // Clone the repository
            let repo = builder.clone(url, path)?;
            Ok((repo, true))
        }
    }

    /// Update the repository (fetch and checkout)
    fn update_repo(
        &self,
        repo: &git2::Repository,
        branch: &str,
        cred: &GitCred,
    ) -> Result<String, GitSyncError> {
        let mut remote = repo.find_remote("origin")?;

        let mut fetch_opts = git2::FetchOptions::new();
        fetch_opts.remote_callbacks(Self::callbacks_for(cred));

        // Fetch
        remote.fetch(&[branch], Some(&mut fetch_opts), None)?;

        // Get the remote branch
        let branch_ref = format!("refs/remotes/origin/{}", branch);
        let reference = repo.find_reference(&branch_ref)?;
        let commit = reference.peel_to_commit()?;
        let commit_id = commit.id().to_string();

        // Checkout the commit
        repo.checkout_tree(
            commit.as_object(),
            Some(&mut git2::build::CheckoutBuilder::new().force()),
        )?;
        repo.set_head(&branch_ref)?;

        debug!("Checked out commit {}", commit_id);

        Ok(commit_id)
    }

    /// Verify the signature on `commit_id` against the source's trusted keys
    /// (Plan 09 Step 5). `extract_signature` returns the armored signature and
    /// the exact signed content (the commit object minus its `gpgsig` header),
    /// which is what the SSHSIG verifier checks. Any failure (unsigned,
    /// unsupported, untrusted) propagates as an error so the sync fails closed.
    fn verify_head_signature(
        &self,
        repo: &git2::Repository,
        commit_id: &str,
        trusted_keys: &[String],
    ) -> Result<(), GitSyncError> {
        let oid = git2::Oid::from_str(commit_id)?;
        // `None` signature field → the default `gpgsig`. A commit with no
        // signature returns GIT_ENOTFOUND, which we map to a fail-closed
        // "unsigned" rather than a generic git error.
        let (signature, signed_content) = match repo.extract_signature(&oid, None) {
            Ok(pair) => pair,
            Err(e) if e.code() == git2::ErrorCode::NotFound => {
                return Err(GitSyncError::Signature(CommitVerifyError::Unsigned));
            }
            Err(e) => return Err(GitSyncError::Git(e)),
        };

        let fingerprint =
            verify_commit_signature(signature.as_ref(), signed_content.as_ref(), trusted_keys)?;
        info!(
            commit = %commit_id,
            key = %fingerprint,
            "Verified signed commit against a trusted key"
        );
        Ok(())
    }

    /// Find policy files matching the patterns
    fn find_policy_files(
        &self,
        repo_path: &Path,
        config: &GitConfig,
    ) -> Result<Vec<PolicyFile>, GitSyncError> {
        let base_path = if let Some(path) = &config.path {
            repo_path.join(path)
        } else {
            repo_path.to_path_buf()
        };

        let mut files = Vec::new();

        for pattern in &config.patterns {
            let glob_pattern = format!("{}/{}", base_path.display(), pattern);
            debug!("Searching for files matching: {}", glob_pattern);

            for entry in
                glob::glob(&glob_pattern).map_err(|e| GitSyncError::Pattern(e.to_string()))?
            {
                match entry {
                    Ok(path) => {
                        if path.is_file() {
                            // Read file content
                            match std::fs::read_to_string(&path) {
                                Ok(content) => {
                                    let relative_path = path
                                        .strip_prefix(repo_path)
                                        .unwrap_or(&path)
                                        .to_string_lossy()
                                        .to_string();

                                    files.push(PolicyFile {
                                        path: relative_path,
                                        content,
                                        language: detect_language(&path),
                                    });
                                }
                                Err(e) => {
                                    warn!("Failed to read file {:?}: {}", path, e);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Glob error: {}", e);
                    }
                }
            }
        }

        Ok(files)
    }

    /// Get all policy files from the last sync
    pub fn get_policy_files(&self, source: &PolicySource) -> Result<Vec<PolicyFile>, GitSyncError> {
        let config = source
            .git_config()
            .ok_or_else(|| GitSyncError::Config("Invalid Git configuration".to_string()))?;

        let repo_path = self.repo_path(source.id);
        self.find_policy_files(&repo_path, &config)
    }

    /// Clean up a repository
    pub fn cleanup(&self, source_id: uuid::Uuid) -> Result<(), GitSyncError> {
        let path = self.repo_path(source_id);
        if path.exists() {
            std::fs::remove_dir_all(path)?;
        }
        Ok(())
    }
}

/// A policy file extracted from Git
#[derive(Debug, Clone)]
pub struct PolicyFile {
    /// Relative path within the repository
    pub path: String,
    /// File content
    pub content: String,
    /// Detected policy language
    pub language: String,
}

/// Detect policy language from file extension
fn detect_language(path: &Path) -> String {
    match path.extension().and_then(|e| e.to_str()) {
        Some("reap") => "reaper".to_string(),
        Some("yaml") | Some("yml") => "reaper".to_string(), // YAML format for Reaper
        Some("json") => "reaper".to_string(),               // JSON format for Reaper
        Some("cedar") => "cedar".to_string(),
        _ => "simple".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_detect_language() {
        assert_eq!(detect_language(Path::new("policy.reap")), "reaper");
        assert_eq!(detect_language(Path::new("rules.yaml")), "reaper");
        assert_eq!(detect_language(Path::new("auth.cedar")), "cedar");
        assert_eq!(detect_language(Path::new("unknown.txt")), "simple");
    }

    #[tokio::test]
    async fn guard_rejects_http_internal_and_metadata_urls() {
        for url in [
            "http://github.com/org/repo.git",            // cleartext
            "https://127.0.0.1/repo.git",                // loopback
            "https://10.1.2.3/repo.git",                 // private
            "https://169.254.169.254/latest/meta-data/", // cloud metadata
            "git://github.com/org/repo.git",             // non-https scheme
            "ssh://git@github.com/org/repo.git",         // ssh (userpass-only syncer)
        ] {
            assert!(
                matches!(
                    guard_remote_url(url).await,
                    Err(GitSyncError::UrlNotAllowed(_))
                ),
                "{url} must be rejected"
            );
        }
    }

    #[tokio::test]
    async fn guard_gates_local_remotes_behind_env_flag() {
        // Single test covers both states of the flag — separate tests would
        // race each other on the shared process environment.
        std::env::remove_var(ALLOW_LOCAL_GIT_ENV);
        assert!(matches!(
            guard_remote_url("file:///tmp/fixture-repo").await,
            Err(GitSyncError::UrlNotAllowed(_))
        ));
        assert!(matches!(
            guard_remote_url("/tmp/fixture-repo").await,
            Err(GitSyncError::UrlNotAllowed(_))
        ));

        std::env::set_var(ALLOW_LOCAL_GIT_ENV, "1");
        assert!(guard_remote_url("file:///tmp/fixture-repo").await.is_ok());
        std::env::remove_var(ALLOW_LOCAL_GIT_ENV);
    }

    #[test]
    fn test_repo_path() {
        let temp_dir = TempDir::new().unwrap();
        let syncer = GitSyncer::new(temp_dir.path());

        let source_id = uuid::Uuid::new_v4();
        let path = syncer.repo_path(source_id);

        assert!(path.to_string_lossy().contains(&source_id.to_string()));
    }
}
