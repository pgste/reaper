//! Behavioural cross-tenant isolation tests (round-3 Plan 01 §6).
//!
//! `tenant_authz.rs` proves *structurally* (static scan) that every mutating
//! org-scoped route references a tenant-authz primitive. This suite proves it
//! *behaviourally*: it drives one tenant's credential against another tenant's
//! resources through the real gateway + router + DB and asserts the request is
//! refused. These are the exact probes a bank's pen-tester runs first, and the
//! regression tests the round-3 P0 fixes (SEC P0-1..P0-4, P1-b) call for.

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use reaper_management::{
    api::build_served_router,
    auth::api_key::{ApiKeyRepository, CreateApiKey},
    auth::sso::broker::{establish_session, ExternalIdentity, LoginContext},
    auth::sso::{SsoConfig, SsoProtocol},
    auth::users::{OrgRole, User, UserOrg, UserOrgRepository, UserRepository},
    config::{AuthConfig, Config},
    db::repositories::{AgentRepository, OrganizationRepository, PolicySourceRepository},
    db::Database,
    domain::agent::RegisterAgent,
    domain::organization::{CreateOrganization, Organization},
    domain::source::{CreatePolicySource, SourceType},
    storage::FilesystemStorage,
    AppState,
};
use serde_json::{json, Value};
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;
use uuid::Uuid;

struct Env {
    #[allow(dead_code)]
    tmp: TempDir,
    app: axum::Router,
    db: Arc<Database>,
}

async fn setup() -> Env {
    let tmp = TempDir::new().unwrap();
    let storage_path = tmp.path().join("storage");
    std::fs::create_dir_all(&storage_path).unwrap();

    let db_config = reaper_management::db::ephemeral_test_config(tmp.path()).await;
    let db = Arc::new(Database::new(&db_config).await.unwrap());
    db.run_migrations().await.unwrap();

    let storage = Arc::new(FilesystemStorage::new(&storage_path).unwrap())
        as Arc<dyn reaper_management::storage::BundleStorage>;

    let config = Config {
        auth: AuthConfig {
            jwt_secret: Some("test-secret-key-for-testing-only".to_string()),
            ..AuthConfig::default()
        },
        ..Config::default()
    };
    let state = AppState::new(db.clone(), config, storage);
    let app = build_served_router().with_state(Arc::new(state));
    Env { tmp, app, db }
}

/// Every API route lives under `/api/v1` in the served router.
fn v1(uri: &str) -> String {
    format!("/api/v1{uri}")
}

fn authed(method: &str, uri: &str, body: Option<Value>, key: &str) -> Request<Body> {
    let mut b = Request::builder()
        .uri(v1(uri))
        .method(method)
        .header("X-API-Key", key);
    if body.is_some() {
        b = b.header("content-type", "application/json");
    }
    let body = body
        .map(|v| Body::from(serde_json::to_vec(&v).unwrap()))
        .unwrap_or(Body::empty());
    b.body(body).unwrap()
}

fn anon(method: &str, uri: &str, body: Option<Value>) -> Request<Body> {
    let mut b = Request::builder().uri(v1(uri)).method(method);
    if body.is_some() {
        b = b.header("content-type", "application/json");
    }
    let body = body
        .map(|v| Body::from(serde_json::to_vec(&v).unwrap()))
        .unwrap_or(Body::empty());
    b.body(body).unwrap()
}

async fn make_org(db: &Database, slug: &str, name: &str) -> Organization {
    OrganizationRepository::new(db)
        .create(CreateOrganization {
            name: name.to_string(),
            slug: slug.to_string(),
            display_name: None,
            description: None,
            settings: json!({}),
        })
        .await
        .unwrap()
}

/// A non-`admin` key for `org_id` — the platform `admin` scope would bypass the
/// tenant guard, so these tests must never use it.
async fn scoped_key(db: &Database, org_id: Uuid, scopes: &[&str]) -> String {
    ApiKeyRepository::new(db)
        .create(
            org_id,
            CreateApiKey {
                name: format!("k-{}", Uuid::new_v4()),
                scopes: scopes.iter().map(|s| s.to_string()).collect(),
                expires_at: None,
                created_by: None,
            },
        )
        .await
        .unwrap()
        .key
}

async fn status_of(app: &axum::Router, req: Request<Body>) -> StatusCode {
    app.clone().oneshot(req).await.unwrap().status()
}

async fn body_of(app: &axum::Router, req: Request<Body>) -> Value {
    let resp = app.clone().oneshot(req).await.unwrap();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or(json!({}))
}

// ---------------------------------------------------------------------------
// SEC P0-2 — webhook-subscription management is tenant-isolated.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn webhook_subscriptions_reject_cross_tenant_access() {
    let env = setup().await;
    let _attacker = make_org(&env.db, "attacker", "Attacker Inc").await;
    let victim = make_org(&env.db, "victim", "Victim Corp").await;
    let key_a = scoped_key(&env.db, _attacker.id, &["org:admin"]).await;
    let key_b = scoped_key(&env.db, victim.id, &["org:admin"]).await;

    // Victim legitimately creates a webhook in its own org (positive control).
    let create = json!({
        "name": "b-hook",
        "url": "https://victim.example.com/hook",
        "events": ["bundle_promoted"]
    });
    let resp = env
        .app
        .clone()
        .oneshot(authed(
            "POST",
            "/orgs/victim/webhooks",
            Some(create),
            &key_b,
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::CREATED,
        "victim can create its own webhook"
    );
    let hook_id = body_of(
        &env.app,
        authed("GET", "/orgs/victim/webhooks", None, &key_b),
    )
    .await["webhooks"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // The attacker holds org:admin in ITS OWN org, but every verb against the
    // victim's webhooks is refused (403) — the caller is bound to org A, not B.
    let hook = format!("/orgs/victim/webhooks/{hook_id}");
    let cases: Vec<(&str, String, Option<Value>)> = vec![
        ("GET", "/orgs/victim/webhooks".into(), None),
        (
            "POST",
            "/orgs/victim/webhooks".into(),
            Some(json!({"name":"x","url":"https://x.example/","events":["bundle_promoted"]})),
        ),
        ("GET", hook.clone(), None),
        ("PUT", hook.clone(), Some(json!({"name":"z"}))),
        ("DELETE", hook.clone(), None),
        ("POST", format!("{hook}/test"), None),
    ];
    for (method, uri, body) in cases {
        assert_eq!(
            status_of(&env.app, authed(method, &uri, body, &key_a)).await,
            StatusCode::FORBIDDEN,
            "attacker {method} {uri} must be 403"
        );
    }

    // Anonymous is rejected at the gateway.
    assert_eq!(
        status_of(&env.app, anon("GET", "/orgs/victim/webhooks", None)).await,
        StatusCode::UNAUTHORIZED,
    );

    // The victim's webhook is untouched — it can still read it.
    assert_eq!(
        status_of(&env.app, authed("GET", &hook, None, &key_b)).await,
        StatusCode::OK,
    );
}

// ---------------------------------------------------------------------------
// SEC P0-3 — a Git source cannot carry client-supplied installation identity.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn git_source_rejects_client_supplied_installation_identity() {
    let env = setup().await;
    let org = make_org(&env.db, "acme", "Acme").await;
    let key = scoped_key(&env.db, org.id, &["policy:write"]).await;

    // installation_id in the config blob → 422 (confused-deputy vector).
    let bad_install = json!({
        "name": "s1", "source_type": "git",
        "config": {"url": "https://github.com/acme/policies.git", "installation_id": "999"}
    });
    assert_eq!(
        status_of(
            &env.app,
            authed("POST", "/orgs/acme/sources", Some(bad_install), &key)
        )
        .await,
        StatusCode::UNPROCESSABLE_ENTITY,
        "installation_id must be rejected from the config blob"
    );

    // repo_full_name likewise.
    let bad_repo = json!({
        "name": "s2", "source_type": "git",
        "config": {"url": "https://github.com/acme/policies.git", "repo_full_name": "victim/private"}
    });
    assert_eq!(
        status_of(
            &env.app,
            authed("POST", "/orgs/acme/sources", Some(bad_repo), &key)
        )
        .await,
        StatusCode::UNPROCESSABLE_ENTITY,
        "repo_full_name must be rejected from the config blob"
    );

    // A clean git source is still accepted (negative control — the guard is
    // specific, not a blanket rejection of git sources).
    let ok = json!({
        "name": "s3", "source_type": "git",
        "config": {"url": "https://github.com/acme/policies.git"}
    });
    assert_eq!(
        status_of(
            &env.app,
            authed("POST", "/orgs/acme/sources", Some(ok), &key)
        )
        .await,
        StatusCode::CREATED,
        "a clean git source must still be accepted"
    );
}

// ---------------------------------------------------------------------------
// SEC P0-4 — the public bundle-update webhook fails CLOSED without a secret.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn bundle_update_webhook_fails_closed_without_secret() {
    let env = setup().await;
    let org = make_org(&env.db, "acme", "Acme").await;

    // A BundleUrl source configured WITHOUT a webhook_secret.
    let source = PolicySourceRepository::new(&env.db)
        .create(
            org.id,
            CreatePolicySource {
                name: "no-secret".into(),
                description: None,
                source_type: SourceType::BundleUrl,
                config: json!({}),
                sync_interval_secs: 300,
            },
        )
        .await
        .unwrap();

    // Unauthenticated webhook call reaches the (public) handler, which must
    // reject it because no secret is configured — never fall through and fetch.
    let req = anon(
        "POST",
        "/webhooks/bundle-update",
        Some(json!({
            "source_id": source.id,
            "bundle_url": "http://169.254.169.254/latest/meta-data/"
        })),
    );
    let resp = env.app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "a secretless bundle-update webhook must fail closed"
    );
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let body = String::from_utf8_lossy(&bytes).to_lowercase();
    assert!(
        body.contains("secret"),
        "the 401 must come from the handler's fail-closed check (mentions the \
         missing secret), not merely the gateway; got: {body}"
    );
}

// ---------------------------------------------------------------------------
// SEC P1-b — a version pin cannot target another tenant's agent by id.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn version_pin_rejects_cross_tenant_agent() {
    let env = setup().await;
    let attacker = make_org(&env.db, "a-org", "A").await;
    let victim = make_org(&env.db, "b-org", "B").await;
    let key_a = scoped_key(&env.db, attacker.id, &["deployment:write"]).await;

    // Seed an agent that belongs to the VICTIM org.
    let agent = AgentRepository::new(&env.db)
        .create(
            victim.id,
            RegisterAgent {
                name: "b-agent".into(),
                hostname: None,
                version: None,
                labels: json!({}),
            },
        )
        .await
        .unwrap();

    // The attacker authorizes against ITS OWN org path but targets the victim's
    // agent id. authorize_deploy passes (deployment:write in a-org); the
    // resource-org recheck must then 404 (foreign id is not an existence oracle).
    let pin = format!("/orgs/a-org/agents/{}/pin", agent.id);
    assert_eq!(
        status_of(
            &env.app,
            authed(
                "POST",
                &pin,
                Some(json!({"bundle_id": Uuid::new_v4()})),
                &key_a
            )
        )
        .await,
        StatusCode::NOT_FOUND,
        "cross-tenant agent pin must 404"
    );
    assert_eq!(
        status_of(&env.app, authed("DELETE", &pin, None, &key_a)).await,
        StatusCode::NOT_FOUND,
        "cross-tenant agent unpin must 404"
    );
    assert_eq!(
        status_of(&env.app, authed("GET", &pin, None, &key_a)).await,
        StatusCode::NOT_FOUND,
        "cross-tenant agent pin read must 404"
    );
}

// ---------------------------------------------------------------------------
// SEC P0-1 — a tenant-self-served IdP cannot adopt another tenant's account by
// asserting its (even verified) email.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn oidc_login_does_not_adopt_account_across_tenant() {
    let env = setup().await;
    let victim_org = make_org(&env.db, "victim-org", "Victim").await;
    let attacker_org = make_org(&env.db, "attacker-org", "Attacker").await;

    // A pre-existing victim account (external user of the victim's real IdP),
    // a member of the victim org.
    let victim = User::external("victim@corp.example".into(), true);
    UserRepository::new(&env.db)
        .create_external(&victim, "https://victim-idp.example", "victim-sub")
        .await
        .unwrap();
    UserOrgRepository::new(&env.db)
        .add_membership(&UserOrg {
            id: Uuid::new_v4(),
            user_id: victim.id,
            org_id: victim_org.id,
            role: OrgRole::Owner,
            invited_by: None,
            joined_at: chrono::Utc::now(),
        })
        .await
        .unwrap();

    // The attacker owns their org and points its SSO at an IdP they control,
    // asserting the victim's verified email.
    let attacker_issuer = "https://attacker-idp.example";
    let cfg = sso_config(attacker_org.id, attacker_issuer);
    let identity = ExternalIdentity {
        issuer: attacker_issuer.into(),
        subject: "attacker-sub".into(),
        email: "victim@corp.example".into(),
        email_verified: true, // attacker controls the IdP, so this is not trustworthy
        groups: vec![],
        display_name: None,
    };

    let result = establish_session(
        &env.db,
        attacker_org.id,
        &identity,
        &cfg,
        &LoginContext::default(),
    )
    .await;

    // The core invariant, robust to email-uniqueness: the attacker's
    // (issuer, subject) must NEVER resolve to the victim's account.
    let linked = UserRepository::new(&env.db)
        .find_by_idp_identity(attacker_issuer, "attacker-sub")
        .await
        .unwrap();
    assert!(
        linked.as_ref().map(|u| u.id) != Some(victim.id),
        "a self-served IdP must not link the attacker's identity to the victim's account"
    );

    // If a session was minted at all, it is for a DISTINCT provisioned user, not
    // the victim. (If provisioning instead failed closed, that is also safe.)
    if let Ok(session) = result {
        assert_ne!(
            session.user_id, victim.id,
            "the minted session must not be the victim's account"
        );
    }
}

// ---------------------------------------------------------------------------
// Plan 05 §4.3 — cross-tenant behavioural corpus.
//
// The `tenant_authz.rs` fitness function proves *structurally* that every
// org-scoped route references a tenant guard; this sweep proves it
// *behaviourally* across every resource family. A fully authenticated attacker
// (org:admin in ITS OWN org) drives every read+mutate verb against the victim
// org's slug and gets refused: `authorize_org` checks org membership before any
// resource lookup, so a slug-addressed route refuses (403) whether or not the
// sub-resource exists. Deleting a `WHERE org_id = $` clause or dropping an
// `authorize_org` call turns a row here red.
//
// (Webhook subscriptions have their own dedicated test above; this corpus
// covers the remaining families the plan names: policies, namespaces/datastore,
// decisions, change-requests, sources, environments, bundles, agents, audit.)
// ---------------------------------------------------------------------------

/// One probe: an HTTP verb + a path under the victim org, and the status an
/// attacker (bound to a different org) must receive.
struct Probe {
    method: &'static str,
    path: &'static str,
    body: Option<Value>,
}

/// Every resource scope EXCEPT platform `admin`. The point of the corpus is to
/// isolate the *tenant* dimension: with all resource scopes granted, the scope
/// check in every handler passes, so the ONLY thing that can refuse the caller
/// is the org-membership (tenant) guard. `admin` is deliberately excluded — it
/// bypasses the tenant guard by design (`user.org_id != org && !has(Admin)`).
const ALL_RESOURCE_SCOPES: &[&str] = &[
    "policy:read",
    "policy:write",
    "bundle:read",
    "bundle:write",
    "bundle:promote",
    "bundle:approve",
    "agent:register",
    "agent:read",
    "agent:write",
    "deployment:write",
    "org:read",
    "org:write",
    "org:admin",
    "apikey:read",
    "apikey:write",
    "audit:erase",
    "audit:export",
    "capability:issue",
    "capability:revoke",
];

fn probe(method: &'static str, path: &'static str) -> Probe {
    Probe {
        method,
        path,
        body: None,
    }
}

fn probe_body(method: &'static str, path: &'static str, body: Value) -> Probe {
    Probe {
        method,
        path,
        body: Some(body),
    }
}

#[tokio::test]
async fn cross_tenant_corpus_refuses_every_resource_family() {
    let env = setup().await;
    // Attacker holds EVERY resource scope in its own org (so a refusal can only
    // be the tenant guard, never a missing scope) — but not platform `admin`,
    // which bypasses tenancy by design.
    let attacker = make_org(&env.db, "attacker", "Attacker Inc").await;
    let _victim = make_org(&env.db, "victim", "Victim Corp").await;
    let attacker_key = scoped_key(&env.db, attacker.id, ALL_RESOURCE_SCOPES).await;

    // GET/DELETE probes need no body and hit `authorize_org` before any resource
    // lookup, so they exercise the guard whether or not the sub-resource exists.
    // A representative UUID stands in for by-id sub-resources (the org guard
    // fires first, so the id need not resolve).
    let dead = "00000000-0000-0000-0000-000000000000";
    let mut probes = vec![
        // policies
        probe("GET", "/orgs/victim/policies"),
        probe_body(
            "POST",
            "/orgs/victim/policies",
            json!({"name":"p","content":"policy p { default: deny, }"}),
        ),
        probe("GET", "/orgs/victim/policies/p"),
        probe_body("PUT", "/orgs/victim/policies/p", json!({"name":"p2"})),
        probe("DELETE", "/orgs/victim/policies/p"),
        probe("GET", "/orgs/victim/policies/p/versions"),
        // namespaces + datastore (ABAC/ReBAC tuple store)
        probe("GET", "/orgs/victim/namespaces"),
        probe_body("POST", "/orgs/victim/namespaces", json!({"slug":"ns-x"})),
        probe("GET", "/orgs/victim/namespaces/default/datastore/entities"),
        probe_body(
            "POST",
            "/orgs/victim/namespaces/default/datastore/entities",
            json!({"entity_type":"User","entity_id":"e1","attributes":{}}),
        ),
        probe("GET", "/orgs/victim/namespaces/default/datastore/tuples"),
        // decisions (the audit read plane)
        probe("GET", "/orgs/victim/decisions"),
        probe("GET", "/orgs/victim/decisions/stats"),
        probe(
            "GET",
            "/orgs/victim/decisions/00000000-0000-0000-0000-000000000000",
        ),
        // change-requests / promotions (dual-control). The collection list is
        // GET /change-requests; the create is POST /promotions.
        probe("GET", "/orgs/victim/change-requests"),
        probe("GET", "/orgs/victim/promotions"),
        // sources
        probe("GET", "/orgs/victim/sources"),
        probe_body(
            "POST",
            "/orgs/victim/sources",
            json!({"name":"s","source_type":"git","config":{"url":"https://x.example/r.git"}}),
        ),
        // environments
        probe("GET", "/orgs/victim/environments"),
        // bundles
        probe("GET", "/orgs/victim/bundles"),
        probe_body("POST", "/orgs/victim/bundles", json!({"name":"b"})),
        // agents (enforcement fleet)
        probe("GET", "/orgs/victim/agents"),
        // audit configuration
        probe("GET", "/orgs/victim/audit/connectors"),
    ];
    // by-id sub-resources: same guard, refused regardless of the id.
    probes.push(probe(
        "GET",
        Box::leak(format!("/orgs/victim/bundles/{dead}").into_boxed_str()),
    ));

    for p in &probes {
        let status = status_of(
            &env.app,
            authed(p.method, p.path, p.body.clone(), &attacker_key),
        )
        .await;
        assert!(
            status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND,
            "attacker {} {} must be refused (403/404), got {}",
            p.method,
            p.path,
            status
        );
        // Crucially it must NOT be a success or a mere validation error — those
        // would mean the request reached past the tenant guard.
        assert!(
            !status.is_success(),
            "attacker {} {} leaked past the tenant guard with {}",
            p.method,
            p.path,
            status
        );
    }

    // Anonymous callers never even reach a handler.
    for p in &probes {
        assert_eq!(
            status_of(&env.app, anon(p.method, p.path, p.body.clone())).await,
            StatusCode::UNAUTHORIZED,
            "anonymous {} {} must be 401 at the gateway",
            p.method,
            p.path
        );
    }
}

/// The positive control for the corpus: the SAME verbs the attacker was refused
/// succeed for the resource's real owner. Without this, the corpus could pass
/// by refusing *everyone* (e.g. a route that is simply broken) — this proves the
/// refusals above are tenant-scoped, not blanket failures.
#[tokio::test]
async fn cross_tenant_corpus_owner_positive_control() {
    let env = setup().await;
    let owner = make_org(&env.db, "owner", "Owner Corp").await;
    let key = scoped_key(&env.db, owner.id, ALL_RESOURCE_SCOPES).await;

    // Reads the attacker was refused succeed for the owner. (Decisions is
    // omitted here: its store is unconfigured in the test env and returns 503
    // *after* the tenant check — the attacker corpus still exercises its guard,
    // which fires first.)
    for uri in [
        "/orgs/owner/policies",
        "/orgs/owner/bundles",
        "/orgs/owner/agents",
        "/orgs/owner/namespaces",
    ] {
        assert_eq!(
            status_of(&env.app, authed("GET", uri, None, &key)).await,
            StatusCode::OK,
            "owner GET {uri} must succeed (proves the corpus refusals are tenant-scoped)"
        );
    }

    // A create the attacker was refused succeeds for the owner.
    assert_eq!(
        status_of(
            &env.app,
            authed(
                "POST",
                "/orgs/owner/policies",
                Some(json!({"name":"ok","content":"policy ok { default: deny, }"})),
                &key,
            ),
        )
        .await,
        StatusCode::CREATED,
        "owner can create its own policy"
    );
}

fn sso_config(org_id: Uuid, issuer: &str) -> SsoConfig {
    let now = chrono::Utc::now();
    SsoConfig {
        id: Uuid::new_v4(),
        org_id,
        protocol: SsoProtocol::Oidc,
        enabled: true,
        issuer: issuer.to_string(),
        client_id: "attacker-client".into(),
        client_secret_encrypted: None,
        discovery_url: None,
        jwks_url: None,
        attr_map_json: None,
        allowed_domains_json: None,
        default_role: "viewer".into(),
        created_at: now,
        updated_at: now,
    }
}
