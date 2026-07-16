//! Tenant-authorization fitness function (round-3 Plan 01, Phase A2).
//!
//! The round-2 auth gateway *authenticates* every non-public route but delegates
//! per-tenant *authorization* to the handlers. That delegation has no backstop:
//! a mutating handler that forgets to bind the caller to the resource's org
//! silently becomes a cross-tenant hole (round-3 SEC P0-2 was exactly this — six
//! webhook-subscription handlers that resolved the org but never authorized the
//! caller against it). The existing `api_contract.rs` gate proves routes are
//! *documented*, and the gateway proves they are *authenticated* — neither
//! proves they are *authorized to the right tenant*.
//!
//! This gate encodes that missing property as a compile-time-adjacent CI check:
//!
//!   > every mutating (`post`/`put`/`patch`/`delete`) handler whose path is
//!   > org-scoped (`…/orgs/{org}/…`) must reference an approved tenant-authz
//!   > primitive in its body.
//!
//! It is intentionally a *coarse, sound* check: it does not try to prove the
//! authorization is semantically correct (static analysis can't), only that a
//! recognised tenant-authz primitive is present. The finer "by-id resource-org
//! recheck" property is covered behaviourally by the cross-tenant integration
//! suite. A new org-scoped mutation therefore cannot merge with *no* tenant
//! check at all — the build goes red until it authorizes or is explicitly,
//! justifiably exempted.
//!
//! `fitness_function_catches_unguarded_route` is the canary meta-test: it proves
//! the checker actually fires on a deliberately-unguarded handler, so a future
//! refactor that neuters the checker is itself caught.

use std::fs;
use std::path::{Path, PathBuf};

/// Substrings whose presence in a handler body counts as a tenant-authz
/// primitive. Each resolves the org *and* binds the caller to it (or, for the
/// `ensure_*_in_org` / `get_scoped` helpers, rechecks a by-id resource's owning
/// tenant). Keep this list to primitives that genuinely enforce tenant scope.
/// Non-`authorize*` tenant-authz primitives, matched as plain substrings.
/// Calls to any `authorize*(..)` function are matched separately by
/// [`calls_authorize_fn`] (covers `authorize_org`, `authorize_deploy`,
/// `authorize_admin`, `authorize_export`, the module-local `authorize(..)`
/// helpers, etc. — every one binds the caller to the org before use).
const AUTHZ_MARKERS: &[&str] = &[
    // The manual membership check `user.org_id != organization.id` (namespaces,
    // deployment status, rollback_config).
    "user.org_id",
    // The DB membership check `get_role(user_id, organization.id)` used by the
    // member-management and oauth handlers (returns 403 if the caller is not a
    // member of the resolved org).
    "get_role(",
    // By-id resource-org rechecks that scope a repo read to the caller's org.
    "get_scoped",
    "load_scoped",
    "ensure_rollout_in_org",
    "ensure_agent_in_org",
];

/// True if `body` calls some `authorize*(..)` function — i.e. the substring
/// `authorize`, optionally followed by `[a-z0-9_]*`, then `(`. This recognises
/// every org-binding helper in the codebase (bare `authorize(`, `authorize_org(`,
/// `authorize_deploy(`, `authorize_admin(`, `authorize_export(`, …) without
/// hard-coding each name, while NOT matching a bare `authorize` mentioned in a
/// comment or a scope string.
fn calls_authorize_fn(body: &str) -> bool {
    let bytes = body.as_bytes();
    let needle = "authorize";
    let mut idx = 0;
    while let Some(rel) = body[idx..].find(needle) {
        let mut j = idx + rel + needle.len();
        while j < body.len() && (bytes[j] == b'_' || bytes[j].is_ascii_alphanumeric()) {
            j += 1;
        }
        if bytes.get(j) == Some(&b'(') {
            return true;
        }
        idx = idx + rel + needle.len();
    }
    false
}

/// A handler may delegate its side effect (and its authorization) to a private
/// inner function or through the idempotency wrapper. Those inner fns carry no
/// `#[utoipa::path]`, so they are not scanned directly; accept the delegation at
/// the outer handler. The inner fns are few and covered by the behavioural
/// cross-tenant tests.
const DELEGATION_MARKERS: &[&str] = &["_inner(", "idempotency::run"];

/// Mutating, org-scoped routes that are legitimately NOT caller-tenant-authorized,
/// each with the reason. This list is a ratchet: it may shrink, never grow
/// without review (`exemptions_do_not_grow`).
const TENANT_AUTHZ_EXEMPT: &[(&str, &str, &str)] = &[
    // (verb, path, justification)
    (
        "post",
        "/orgs/{org}/oauth/connections",
        "dead stub: returns 400 unconditionally with no data access or side \
         effect (manual token entry was superseded by the OAuth flow). No tenant \
         data is reachable, so there is nothing to authorize.",
    ),
];

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[derive(Debug)]
struct Handler {
    verb: String,
    path: String,
    name: String,
    body: String,
}

/// Capture a balanced `{ .. }` block starting at the first `{` in `s`.
/// Brace-counting is not string/comment aware, but for substring detection an
/// approximately-captured body is sufficient.
fn balanced_block(s: &str) -> String {
    let Some(open) = s.find('{') else {
        return String::new();
    };
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut i = open;
    while i < s.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return s[open..=i].to_string();
                }
            }
            _ => {}
        }
        i += 1;
    }
    s[open..].to_string()
}

/// Parse every `#[utoipa::path(..)] … fn name(..) { .. }` in a source file into
/// its verb, path template, name, and (approximate) body.
fn parse_handlers(src: &str) -> Vec<Handler> {
    let mut handlers = Vec::new();
    let attr = "#[utoipa::path(";
    let mut idx = 0;
    while let Some(rel) = src[idx..].find(attr) {
        let after = idx + rel + attr.len();
        let Some(crel) = src[after..].find(")]") else {
            break;
        };
        let close = after + crel;
        let attr_body = &src[after..close];

        let verb = attr_body
            .split(|c: char| c == ',' || c.is_whitespace())
            .find(|t| !t.is_empty())
            .unwrap_or("")
            .to_string();

        let path = attr_body
            .find("path")
            .and_then(|p| {
                let q = attr_body[p..].find('"')? + p + 1;
                let e = attr_body[q..].find('"')? + q;
                Some(attr_body[q..e].to_string())
            })
            .unwrap_or_default();

        idx = close + 2;

        if let Some(fnrel) = src[idx..].find("fn ") {
            let fnat = idx + fnrel;
            let name_start = fnat + 3;
            let name_end = src[name_start..]
                .find(|c: char| c == '(' || c.is_whitespace())
                .map(|e| name_start + e)
                .unwrap_or(name_start);
            let name = src[name_start..name_end].trim().to_string();
            let body = balanced_block(&src[fnat..]);
            handlers.push(Handler {
                verb,
                path,
                name,
                body,
            });
        }
    }
    handlers
}

fn is_mutating(verb: &str) -> bool {
    matches!(verb, "post" | "put" | "patch" | "delete")
}

fn is_org_scoped(path: &str) -> bool {
    path.contains("{org}")
}

fn has_tenant_authz(body: &str) -> bool {
    calls_authorize_fn(body)
        || AUTHZ_MARKERS.iter().any(|m| body.contains(m))
        || DELEGATION_MARKERS.iter().any(|m| body.contains(m))
}

fn is_exempt(verb: &str, path: &str) -> bool {
    TENANT_AUTHZ_EXEMPT
        .iter()
        .any(|(v, p, _)| *v == verb && *p == path)
}

/// Core check over a single source string: return the offending
/// `verb path (fn)` for every mutating, org-scoped handler lacking a tenant
/// authorization primitive. Pure over `src` so the canary meta-test can drive it.
fn unauthorized_org_mutations(src: &str) -> Vec<String> {
    parse_handlers(src)
        .into_iter()
        .filter(|h| is_mutating(&h.verb) && is_org_scoped(&h.path))
        .filter(|h| !has_tenant_authz(&h.body))
        .filter(|h| !is_exempt(&h.verb, &h.path))
        .map(|h| format!("{} {} ({})", h.verb, h.path, h.name))
        .collect()
}

#[test]
fn every_org_scoped_mutation_authorizes_the_tenant() {
    let api_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/api");
    let mut files = Vec::new();
    collect_rs_files(&api_dir, &mut files);
    assert!(!files.is_empty(), "no api source files under {api_dir:?}");

    let mut offenders = Vec::new();
    for file in &files {
        let content = fs::read_to_string(file).expect("read api source");
        for offense in unauthorized_org_mutations(&content) {
            offenders.push(format!(
                "{}: {}",
                file.strip_prefix(env!("CARGO_MANIFEST_DIR"))
                    .unwrap_or(file)
                    .display(),
                offense
            ));
        }
    }

    assert!(
        offenders.is_empty(),
        "org-scoped mutating route(s) do not bind the caller to the resource's \
         tenant — authorize with `authorize_org`/`authorize_resource`/\
         `authorize_deploy` (or a `get_scoped`/`ensure_*_in_org` recheck), or add \
         a justified entry to TENANT_AUTHZ_EXEMPT. This is round-3 SEC P0-2's \
         finding class.\nOffenders:\n  {}",
        offenders.join("\n  ")
    );
}

#[test]
fn fitness_function_catches_unguarded_route() {
    // A deliberately-unguarded org-scoped mutation MUST be flagged. This proves
    // the checker fires — guarding the guard.
    let unguarded = r#"
        #[utoipa::path(post, path = "/orgs/{org}/things/{thing}", tag = "x")]
        async fn danger(State(s): State<Arc<AppState>>, Path((org, thing)): Path<(String, Uuid)>) {
            let repo = ThingRepository::new(&s.db);
            repo.mutate(thing).await?;
        }
    "#;
    assert_eq!(
        unauthorized_org_mutations(unguarded).len(),
        1,
        "the fitness function must flag an org-scoped mutation with no tenant authz"
    );

    // The same handler WITH an approved primitive must NOT be flagged.
    let guarded = r#"
        #[utoipa::path(post, path = "/orgs/{org}/things/{thing}", tag = "x")]
        async fn safe(State(s): State<Arc<AppState>>, RequireAuth(user): RequireAuth, Path((org, thing)): Path<(String, Uuid)>) {
            let organization = authorize_org(&s, &user, &org, &[Scope::OrgAdmin]).await?;
            let repo = ThingRepository::new(&s.db);
            repo.mutate(thing).await?;
        }
    "#;
    assert!(
        unauthorized_org_mutations(guarded).is_empty(),
        "a handler that calls authorize_org must pass"
    );

    // A non-mutating (GET) org-scoped handler is out of scope even without authz.
    let read = r#"
        #[utoipa::path(get, path = "/orgs/{org}/things", tag = "x")]
        async fn list(State(s): State<Arc<AppState>>, Path(org): Path<String>) {
            let _ = s;
        }
    "#;
    assert!(
        unauthorized_org_mutations(read).is_empty(),
        "GET routes are not in scope for this mutation gate"
    );
}

#[test]
fn exemptions_do_not_grow() {
    // Ratchet: the exemption allowlist may shrink but never grow without review.
    const EXEMPT_BASELINE: usize = 1;
    assert!(
        TENANT_AUTHZ_EXEMPT.len() <= EXEMPT_BASELINE,
        "TENANT_AUTHZ_EXEMPT grew to {} (baseline {}). A new exemption weakens \
         tenant isolation — justify it in review and raise the baseline \
         deliberately, or remove it.",
        TENANT_AUTHZ_EXEMPT.len(),
        EXEMPT_BASELINE
    );
}
