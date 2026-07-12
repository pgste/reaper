//! Plan 09 Phase A — the GitOps sync engine runs and materializes.
//!
//! Product F2 said the sync engine was compiled-but-dead: nothing spawned it,
//! the manual trigger was a status-flip placeholder, and even when invoked it
//! counted policy files without persisting them. These tests drive the real
//! `SyncService` against a local git fixture repo and assert the full loop:
//! trigger → clone/checkout → policies upserted → bundle created + compiled,
//! keyed by the commit SHA (idempotent), with the SSRF guard on the remote.

use std::path::Path;
use std::sync::Arc;

use reaper_management::{
    bundle::BundleService,
    db::repositories::{
        BundleRepository, OrganizationRepository, PolicyRepository, PolicySourceRepository,
    },
    db::Database,
    domain::bundle::BundleStatus,
    domain::organization::CreateOrganization,
    domain::source::{CreatePolicySource, SourceType},
    storage::{BundleStorage, FilesystemStorage},
    sync::{SyncConfig, SyncError, SyncService},
};
use serde_json::json;
use tempfile::TempDir;
use uuid::Uuid;

/// Create a git repo with two policy files; return the HEAD commit SHA.
fn init_fixture_repo(dir: &Path) -> String {
    let mut opts = git2::RepositoryInitOptions::new();
    opts.initial_head("main");
    let repo = git2::Repository::init_opts(dir, &opts).unwrap();

    std::fs::create_dir_all(dir.join("policies")).unwrap();
    std::fs::write(
        dir.join("policies/allow-docs.reap"),
        "policy allow_docs {\n  allow read on \"/docs/*\"\n}\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("policies/deny-admin.reap"),
        "policy deny_admin {\n  deny write on \"/admin/*\"\n}\n",
    )
    .unwrap();

    let mut index = repo.index().unwrap();
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let sig = git2::Signature::now("fixture", "fixture@test").unwrap();
    let commit = repo
        .commit(Some("HEAD"), &sig, &sig, "add policies", &tree, &[])
        .unwrap();
    commit.to_string()
}

/// Create a git repo whose HEAD commit is SSH-signed (as `git config
/// gpg.format ssh` produces). Returns (commit SHA, trusted authorized_keys
/// public line).
fn init_signed_fixture_repo(dir: &Path) -> (String, String) {
    use ssh_key::{HashAlg, LineEnding, PrivateKey};

    let mut opts = git2::RepositoryInitOptions::new();
    opts.initial_head("main");
    let repo = git2::Repository::init_opts(dir, &opts).unwrap();

    std::fs::create_dir_all(dir.join("policies")).unwrap();
    std::fs::write(
        dir.join("policies/allow-docs.reap"),
        "policy allow_docs {\n  allow read on \"/docs/*\"\n}\n",
    )
    .unwrap();

    let mut index = repo.index().unwrap();
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    let sig = git2::Signature::now("signer", "signer@test").unwrap();

    // Build the commit content, sign it under the git SSHSIG namespace, and
    // attach the armored signature — exactly what git does for SSH signing.
    let buf = repo
        .commit_create_buffer(&sig, &sig, "signed policies", &tree, &[])
        .unwrap();
    let content = std::str::from_utf8(&buf).unwrap();
    let key = PrivateKey::from(ssh_key::private::Ed25519Keypair::from_seed(&[11u8; 32]));
    let armored = key
        .sign("git", HashAlg::Sha512, content.as_bytes())
        .unwrap()
        .to_pem(LineEnding::LF)
        .unwrap();
    let oid = repo
        .commit_signed(content, &armored, Some("gpgsig"))
        .unwrap();
    // Point HEAD/main at the signed commit.
    repo.reference("refs/heads/main", oid, true, "signed")
        .unwrap();
    repo.set_head("refs/heads/main").unwrap();

    let pubkey = key.public_key().to_openssh().unwrap();
    (oid.to_string(), pubkey)
}

struct Env {
    _temp: TempDir,
    db: Arc<Database>,
    org_id: Uuid,
    sync: SyncService,
}

async fn setup(auto_compile: bool) -> Env {
    // Local fixture repos are file:// remotes, which the SSRF guard blocks by
    // default — opt in for the test process (documented test-only flag).
    std::env::set_var("REAPER_SYNC_ALLOW_LOCAL_GIT", "1");

    let temp = TempDir::new().unwrap();
    let db_config = reaper_management::db::ephemeral_test_config(temp.path()).await;
    let db = Database::new(&db_config).await.unwrap();
    db.run_migrations().await.unwrap();
    let db = Arc::new(db);

    let org = OrganizationRepository::new(&db)
        .create(CreateOrganization {
            name: "gitops-test".to_string(),
            slug: format!("gitops-{}", Uuid::new_v4().simple()),
            display_name: None,
            description: None,
            settings: json!({}),
        })
        .await
        .unwrap();

    let storage_path = temp.path().join("storage");
    std::fs::create_dir_all(&storage_path).unwrap();
    let storage =
        Arc::new(FilesystemStorage::new(&storage_path).unwrap()) as Arc<dyn BundleStorage>;
    let bundle_service = Arc::new(BundleService::new(db.clone(), storage));

    let sync = SyncService::new(
        db.clone(),
        SyncConfig {
            git_base_path: temp.path().join("sync/git"),
            s3_cache_path: temp.path().join("sync/s3"),
            bundle_storage_path: temp.path().join("sync/bundles"),
            check_interval_secs: 60,
            max_concurrent: 5,
            auto_compile,
        },
    )
    .with_materializer(bundle_service);

    Env {
        _temp: temp,
        db,
        org_id: org.id,
        sync,
    }
}

async fn create_git_source(env: &Env, name: &str, url: &str) -> Uuid {
    create_git_source_cfg(
        env,
        name,
        json!({ "url": url, "branch": "main", "patterns": ["**/*.reap"] }),
    )
    .await
}

async fn create_git_source_cfg(env: &Env, name: &str, config: serde_json::Value) -> Uuid {
    PolicySourceRepository::new(&env.db)
        .create(
            env.org_id,
            CreatePolicySource {
                name: name.to_string(),
                description: None,
                source_type: SourceType::Git,
                config,
                sync_interval_secs: 300,
            },
        )
        .await
        .unwrap()
        .id
}

#[tokio::test]
async fn git_sync_materializes_policies_and_compiled_bundle_idempotently() {
    let env = setup(true).await;

    let fixture = env._temp.path().join("fixture-repo");
    std::fs::create_dir_all(&fixture).unwrap();
    let head_sha = init_fixture_repo(&fixture);

    let source_id = create_git_source(
        &env,
        "prod-policies",
        &format!("file://{}", fixture.display()),
    )
    .await;

    // First sync: real clone, policies upserted, bundle created + compiled.
    let result = env.sync.trigger_sync(source_id).await.unwrap();
    assert!(result.success);
    assert_eq!(result.policies_found, 2);
    assert_eq!(result.policies_created, 2);
    assert_eq!(result.commit.as_deref(), Some(head_sha.as_str()));

    let policy_repo = PolicyRepository::new(&env.db);
    let allow = policy_repo
        .get_by_name(env.org_id, "prod-policies/policies-allow-docs")
        .await
        .unwrap()
        .expect("synced policy must be materialized as a policy row");
    assert_eq!(allow.source_id, Some(source_id));
    assert_eq!(
        allow.source_path.as_deref(),
        Some("policies/allow-docs.reap")
    );

    let bundle_repo = BundleRepository::new(&env.db);
    let bundle_id = bundle_repo
        .find_by_source_commit(source_id, &head_sha)
        .await
        .unwrap()
        .expect("bundle must be linked to the source + commit SHA");
    let bundle = bundle_repo.get_by_id(bundle_id).await.unwrap().unwrap();
    assert_eq!(bundle.status, BundleStatus::Compiled);
    assert_eq!(bundle.policy_count, 2);

    // Second sync at the same SHA: idempotent — no new policies, no version
    // churn, and the SAME bundle (a webhook and a poll must not double-apply).
    let second = env.sync.trigger_sync(source_id).await.unwrap();
    assert_eq!(second.policies_created, 0);
    assert_eq!(second.policies_updated, 0);
    let bundle_again = bundle_repo
        .find_by_source_commit(source_id, &head_sha)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(bundle_again, bundle_id);
    let versions = policy_repo.get_versions(allow.id).await.unwrap();
    assert_eq!(
        versions.len(),
        1,
        "unchanged content must not bump versions"
    );
}

#[tokio::test]
async fn git_sync_without_auto_compile_leaves_draft_bundle() {
    let env = setup(false).await;

    let fixture = env._temp.path().join("fixture-repo");
    std::fs::create_dir_all(&fixture).unwrap();
    let head_sha = init_fixture_repo(&fixture);

    let source_id = create_git_source(
        &env,
        "staged-policies",
        &format!("file://{}", fixture.display()),
    )
    .await;

    env.sync.trigger_sync(source_id).await.unwrap();

    let bundle_repo = BundleRepository::new(&env.db);
    let bundle_id = bundle_repo
        .find_by_source_commit(source_id, &head_sha)
        .await
        .unwrap()
        .expect("bundle is created (draft) even without auto-compile");
    let bundle = bundle_repo.get_by_id(bundle_id).await.unwrap().unwrap();
    assert_eq!(bundle.status, BundleStatus::Draft);
}

#[tokio::test]
async fn trigger_sync_on_unknown_source_is_not_found() {
    let env = setup(false).await;
    match env.sync.trigger_sync(Uuid::new_v4()).await {
        Err(SyncError::NotFound(_)) => {}
        other => panic!("expected NotFound, got {other:?}"),
    }
}

#[tokio::test]
async fn ssrf_guarded_remote_fails_the_sync_without_fetching() {
    let env = setup(false).await;

    // Cloud-metadata address: the guard must reject it before any network
    // activity (https scheme included, so only the address check can reject).
    let source_id = create_git_source(
        &env,
        "evil",
        "https://169.254.169.254/latest/meta-data/repo.git",
    )
    .await;

    let err = env.sync.trigger_sync(source_id).await.unwrap_err();
    assert!(
        err.to_string().contains("not allowed"),
        "expected the SSRF guard to reject, got: {err}"
    );
}

#[tokio::test]
async fn signed_commit_by_trusted_key_syncs_and_untrusted_fails_closed() {
    let env = setup(false).await;

    let fixture = env._temp.path().join("signed-repo");
    std::fs::create_dir_all(&fixture).unwrap();
    let (head_sha, trusted_pubkey) = init_signed_fixture_repo(&fixture);
    let url = format!("file://{}", fixture.display());

    // require_signed_commits + the actual signer key trusted → sync succeeds.
    let ok_source = create_git_source_cfg(
        &env,
        "signed-ok",
        json!({
            "url": url,
            "branch": "main",
            "patterns": ["**/*.reap"],
            "require_signed_commits": true,
            "trusted_signing_keys": [trusted_pubkey],
        }),
    )
    .await;
    let result = env.sync.trigger_sync(ok_source).await.unwrap();
    assert_eq!(result.commit.as_deref(), Some(head_sha.as_str()));

    // Same signed HEAD, but a DIFFERENT key is the only trusted one → the sync
    // must fail closed (an attacker's signature must not pass).
    let untrusted =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIExampleUntrustedKeyDoesNotMatch00000000000 other";
    let bad_source = create_git_source_cfg(
        &env,
        "signed-untrusted",
        json!({
            "url": url,
            "branch": "main",
            "patterns": ["**/*.reap"],
            "require_signed_commits": true,
            "trusted_signing_keys": [untrusted],
        }),
    )
    .await;
    let err = env.sync.trigger_sync(bad_source).await.unwrap_err();
    assert!(
        err.to_string().contains("signature"),
        "untrusted signer must fail closed, got: {err}"
    );
}

#[tokio::test]
async fn unsigned_commit_fails_closed_when_signing_required() {
    let env = setup(false).await;

    let fixture = env._temp.path().join("unsigned-repo");
    std::fs::create_dir_all(&fixture).unwrap();
    init_fixture_repo(&fixture); // ordinary unsigned commit

    let source = create_git_source_cfg(
        &env,
        "requires-signing",
        json!({
            "url": format!("file://{}", fixture.display()),
            "branch": "main",
            "patterns": ["**/*.reap"],
            "require_signed_commits": true,
            "trusted_signing_keys": ["ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIExample00000000000000000000000000000000000 k"],
        }),
    )
    .await;

    let err = env.sync.trigger_sync(source).await.unwrap_err();
    assert!(
        err.to_string().contains("signature") || err.to_string().contains("not signed"),
        "unsigned HEAD with signing required must fail closed, got: {err}"
    );
}
