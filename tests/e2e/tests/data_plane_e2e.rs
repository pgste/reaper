//! TRUE end-to-end test of the data plane: REAL `reaper-management` and
//! `reaper-agent` binaries as separate processes, replicated by the REAL
//! `reaper-sync` engine over real HTTP. No inline shortcuts — this
//! exercises the exact code paths production runs:
//!
//!   1. bootstrap org + API key, spawn both services
//!   2. manage data through the control-plane APIs, publish snapshot v1
//!   3. `SyncEngine::sync_datastore()` -> checksum-verified snapshot deploy
//!   4. mutate (attribute change + CASCADE delete), sync again ->
//!      delta pull + contiguous apply-deltas advance the agent's seq
//!   5. KILL the agent, respawn it cold, sync once ->
//!      confirm 409 -> full snapshot redeploy -> delta catch-up to head,
//!      all inside a single self-healing sync step
//!
//! Requires the two binaries to be built (`cargo build -p reaper-agent
//! -p reaper-management`); skips with a message otherwise.

use reqwest::Client;
use serde_json::{json, Value};
use std::process::{Child, Command};
use std::time::Duration;

struct Proc(Child);
impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn bin(name: &str) -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var(format!(
        "REAPER_E2E_{}_BIN",
        name.replace('-', "_").to_uppercase()
    )) {
        return Some(p.into());
    }
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for profile in ["debug", "release"] {
        let candidate = manifest.join(format!("../../target/{profile}/{name}"));
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

async fn wait_healthy(client: &Client, url: &str) -> bool {
    for _ in 0..100 {
        if let Ok(resp) = client.get(format!("{url}/health")).send().await {
            if resp.status().is_success() {
                return true;
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    false
}

fn spawn_agent(port: u16) -> Proc {
    spawn_agent_with(port, &[])
}

fn spawn_agent_with(port: u16, extra_env: &[(&str, &str)]) -> Proc {
    let path = bin("reaper-agent").expect("agent binary");
    let mut cmd = Command::new(path);
    cmd.env("REAPER_PORT", port.to_string())
        .env("REAPER_BIND_ADDRESS", "127.0.0.1")
        .env("RUST_LOG", "warn")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    Proc(cmd.spawn().expect("spawn agent"))
}

#[tokio::test]
async fn data_plane_replication_end_to_end() {
    let (Some(mgmt_bin), Some(_)) = (bin("reaper-management"), bin("reaper-agent")) else {
        eprintln!("SKIP: build reaper-management + reaper-agent first");
        return;
    };

    let tmp = tempfile::TempDir::new().unwrap();
    let storage = tmp.path().join("storage");
    std::fs::create_dir_all(&storage).unwrap();
    // SQLite by default; a fresh PostgreSQL database per run when
    // REAPER_TEST_DATABASE_URL points at an admin database.
    let db_config = reaper_management::db::ephemeral_test_config(tmp.path()).await;
    let db_url = db_config.url.clone();

    // --- Bootstrap: migrations + org + API key via the library (mirrors
    // real ops bootstrap), BEFORE the server owns the file. ---
    let api_key;
    let org_id;
    {
        use reaper_management::auth::api_key::{ApiKeyRepository, CreateApiKey};
        use reaper_management::db::Database;

        let db = Database::new(&db_config).await.unwrap();
        db.run_migrations().await.unwrap();

        let org_repo = reaper_management::db::repositories::OrganizationRepository::new(&db);
        let org = org_repo
            .create(
                reaper_management::domain::organization::CreateOrganization {
                    name: "E2E Org".into(),
                    slug: "e2e-org".into(),
                    display_name: None,
                    description: None,
                    settings: serde_json::json!({}),
                },
            )
            .await
            .unwrap();
        org_id = org.id;

        let created = ApiKeyRepository::new(&db)
            .create(
                org_id,
                CreateApiKey {
                    name: "e2e".into(),
                    scopes: vec![
                        "admin".into(),
                        "org:admin".into(),
                        "agent:read".into(),
                        "agent:write".into(),
                        "policy:write".into(),
                    ],
                    expires_at: None,
                    created_by: None,
                },
            )
            .await
            .unwrap();
        api_key = created.key;
    }

    // --- Spawn the real processes. ---
    let mgmt_port = free_port();
    let agent_port = free_port();
    let mgmt_url = format!("http://127.0.0.1:{mgmt_port}");
    let agent_url = format!("http://127.0.0.1:{agent_port}");

    let _mgmt = Proc(
        Command::new(&mgmt_bin)
            .env("REAPER_PORT", mgmt_port.to_string())
            .env("REAPER_BIND_ADDRESS", "127.0.0.1")
            .env("REAPER_DATABASE_TYPE", &db_config.db_type)
            .env("REAPER_DATABASE_URL", &db_url)
            .env("REAPER_STORAGE_TYPE", "filesystem")
            .env("REAPER_STORAGE_PATH", storage.display().to_string())
            .env(
                "REAPER_JWT_SECRET",
                "e2e-secret-not-for-prod-needs-32-chars-min",
            )
            .env("RUST_LOG", "warn")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn management"),
    );
    let mut agent = spawn_agent(agent_port);

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    assert!(wait_healthy(&client, &mgmt_url).await, "management up");
    assert!(wait_healthy(&client, &agent_url).await, "agent up");

    let post = |path: String, body: Value| {
        let client = client.clone();
        let url = format!("{mgmt_url}{path}");
        let key = api_key.clone();
        async move {
            let resp = client
                .post(url)
                .header("X-API-Key", key)
                .json(&body)
                .send()
                .await
                .unwrap();
            let status = resp.status();
            let body: Value = resp.json().await.unwrap_or(Value::Null);
            (status, body)
        }
    };

    // --- Namespace + datastore + records over real HTTP. ---
    let (status, _) = post("/orgs/e2e-org/namespaces".into(), json!({"slug": "prod"})).await;
    assert!(status.is_success(), "namespace: {status}");
    let base = "/orgs/e2e-org/namespaces/prod/datastore";

    let (status, _) = post(base.into(), json!({"template": "combined"})).await;
    assert!(status.is_success(), "provision: {status}");

    for body in [
        json!({"entity_id": "alice", "entity_type": "user",
               "attributes": {"mfa": true, "clearance": 5}}),
        json!({"entity_id": "bob", "entity_type": "user", "attributes": {}}),
    ] {
        let (status, _) = post(format!("{base}/entities"), body).await;
        assert!(status.is_success(), "entity: {status}");
    }
    let (status, _) = post(
        format!("{base}/tuples"),
        json!({"object": "doc-1", "relation": "owner", "subject": "bob"}),
    )
    .await;
    assert!(status.is_success(), "tuple: {status}");
    let (status, published) = post(format!("{base}/publish"), Value::Null).await;
    assert!(status.is_success(), "publish: {status} {published}");
    assert_eq!(published["version"], 1);

    // --- The REAL sync engine. ---
    use reaper_sync::config::*;
    let sync_config = SyncConfig {
        sync: SyncSettings {
            server: ServerConfig {
                url: mgmt_url.clone(),
                api_version: "v1".into(),
                timeout_seconds: 10,
            },
            auth: AuthConfig {
                auth_type: "api_token".into(),
                token_file: None,
                token: Some(api_key.clone()),
                cert_file: None,
                key_file: None,
                ca_file: None,
            },
            scope: ScopeConfig {
                teams: vec![],
                environments: vec![],
                regions: vec![],
                policy_ids: vec![],
            },
            datastore: DatastoreSyncConfig {
                enabled: true,
                org: "e2e-org".into(),
                namespace: "prod".into(),
            },
            behavior: BehaviorConfig {
                mode: "on-demand".into(),
                poll_interval_seconds: 1,
                batch_size: 50,
                retry_max_attempts: 1,
                retry_backoff_seconds: 1,
                sync_on_start: false,
            },
            agent: AgentConfig {
                url: agent_url.clone(),
                health_check_interval_seconds: 30,
                timeout_seconds: 10,
            },
            cache: CacheConfig::default(),
            metrics: MetricsConfig::default(),
        },
    };
    let mut engine = reaper_sync::sync_engine::SyncEngine::new(sync_config).unwrap();

    // Snapshot replication.
    engine.sync_datastore().await.expect("snapshot sync");

    let agent_stats = |client: &Client| {
        let url = format!("{agent_url}/debug/datastore");
        let client = client.clone();
        async move {
            let v: Value = client.get(url).send().await.unwrap().json().await.unwrap();
            v["total_entities"].as_i64().unwrap()
        }
    };
    assert_eq!(
        agent_stats(&client).await,
        3,
        "agent must hold alice + bob + doc-1 after snapshot"
    );

    // --- Mutations after the snapshot: attribute change + CASCADE delete. ---
    let resp = client
        .put(format!("{mgmt_url}{base}/entities/alice/attributes"))
        .header("X-API-Key", api_key.clone())
        .json(&json!({"mfa": false, "clearance": 2}))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "attr update");
    let resp = client
        .delete(format!("{mgmt_url}{base}/entities/bob"))
        .header("X-API-Key", api_key.clone())
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "cascade delete");

    // Delta replication (same engine, next poll step).
    engine.sync_datastore().await.expect("delta sync");
    assert_eq!(
        agent_stats(&client).await,
        1,
        "bob tombstoned; doc-1 (owner edge only) cascaded away; alice remains"
    );

    // Seq visible on the wire (deploy a trivial policy so /ready is 200).
    let (s, _) = {
        let resp = client
            .post(format!("{agent_url}/api/v1/policies/deploy"))
            .json(&json!({
                "policy_id": uuid::Uuid::new_v4().to_string(),
                "name": "e2e-probe", "description": "",
                "rules": [{"action": "allow", "resource": "*", "conditions": null}]
            }))
            .send()
            .await
            .unwrap();
        (resp.status(), ())
    };
    assert!(s.is_success(), "policy deploy");
    let ready: Value = client
        .get(format!("{agent_url}/ready"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(ready["data_version"], 1);
    assert!(
        ready["data_applied_seq"].as_i64().unwrap() > 0,
        "agent advanced through the change stream: {ready}"
    );

    // --- Self-healing + cold-start gate: kill the agent, respawn cold
    // with REAPER_DATA_REQUIRE_SYNC armed. Until the first verified
    // snapshot lands the pod must report NOT ready — this is what keeps a
    // fresh Kubernetes pod out of rotation while its replica is empty —
    // and ONE sync step must both recover the data (confirm 409 ->
    // snapshot v1 -> delta catch-up) and open the gate. ---
    drop(agent);
    tokio::time::sleep(Duration::from_millis(300)).await;
    agent = spawn_agent_with(agent_port, &[("REAPER_DATA_REQUIRE_SYNC", "true")]);
    assert!(wait_healthy(&client, &agent_url).await, "agent respawn");
    assert_eq!(agent_stats(&client).await, 0, "cold agent is empty");

    // Load a policy so the ONLY readiness blocker left is the data gate.
    let resp = client
        .post(format!("{agent_url}/api/v1/policies/deploy"))
        .json(&json!({
            "policy_id": uuid::Uuid::new_v4().to_string(),
            "name": "e2e-probe-2", "description": "",
            "rules": [{"action": "allow", "resource": "*", "conditions": null}]
        }))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "policy deploy on cold agent");
    // 503 for the router, and a machine-readable WHY in the body — this
    // is the contract SDK health indicators (Spring Actuator etc.) build
    // on: map the status code to UP/DOWN, copy the body into details.
    let resp = client
        .get(format!("{agent_url}/ready"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status().as_u16(),
        503,
        "gated agent must be NOT ready before its first verified sync"
    );
    let not_ready: Value = resp.json().await.unwrap();
    assert_eq!(not_ready["status"], "not_ready");
    assert_eq!(
        not_ready["reason"], "awaiting_initial_data_sync",
        "the 503 body must say it is starting up: {not_ready}"
    );

    engine.sync_datastore().await.expect("self-heal sync");
    assert_eq!(
        agent_stats(&client).await,
        1,
        "one sync step after restart: snapshot + delta catch-up to head"
    );
    let ready = client
        .get(format!("{agent_url}/ready"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        ready.status().as_u16(),
        200,
        "first verified sync opens the readiness gate"
    );
    let ready: Value = ready.json().await.unwrap();
    assert_eq!(ready["status"], "ready");
    assert_eq!(ready["reason"], Value::Null);
    assert_eq!(ready["data_version"], 1, "gate opened by the v1 snapshot");
    drop(agent);
}
