//! IdP-agnostic session broker.
//!
//! Every SSO protocol (OIDC now, SAML later) funnels through
//! [`establish_session`], the single place that touches identity tables. It
//! find-or-creates the user by `(issuer, subject)`, reconciles the org role
//! from the IdP's groups, mints an `rst_` session that `RequireAuth` already
//! understands, and audits the login.

use uuid::Uuid;

use super::SsoConfig;
use crate::audit::{actions, ActorType, AuditEntry};
use crate::auth::users::{
    OrgRole, Session, SessionRepository, User, UserError, UserOrg, UserOrgRepository,
    UserRepository,
};
use crate::db::Database;

/// An identity asserted by an external IdP after a successful authentication.
#[derive(Debug, Clone)]
pub struct ExternalIdentity {
    pub issuer: String,
    pub subject: String,
    pub email: String,
    /// Whether the IdP asserts the email is verified.
    pub email_verified: bool,
    /// Raw IdP group names (mapped to a role via the config's attribute map).
    pub groups: Vec<String>,
    pub display_name: Option<String>,
}

/// Request-scoped context for the login (session + audit metadata).
#[derive(Debug, Clone, Default)]
pub struct LoginContext {
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    /// Session lifetime; 0 falls back to the 24h default.
    pub session_ttl_hours: u64,
}

/// Result of establishing a session: the plaintext `rst_` token to hand back to
/// the browser, plus the resolved user and role.
pub struct EstablishedSession {
    pub token: String,
    pub user_id: Uuid,
    pub role: OrgRole,
}

/// Turn an external identity into a Reaper session for `org_id`.
pub async fn establish_session(
    db: &Database,
    org_id: Uuid,
    identity: &ExternalIdentity,
    config: &SsoConfig,
    ctx: &LoginContext,
) -> Result<EstablishedSession, UserError> {
    let role = map_groups_to_role(config, &identity.groups);
    let users = UserRepository::new(db);

    // Resolve the user: by IdP subject first (stable), else adopt a pre-existing
    // verified-email account, else provision a fresh SSO user.
    let user = match users
        .find_by_idp_identity(&identity.issuer, &identity.subject)
        .await?
    {
        Some(u) => u,
        None => match users.find_by_email(&identity.email).await? {
            Some(u) => {
                users
                    .link_idp_identity(u.id, &identity.issuer, &identity.subject)
                    .await?;
                u
            }
            None => {
                let u = User::external(identity.email.clone(), identity.email_verified);
                users
                    .create_external(&u, &identity.issuer, &identity.subject)
                    .await?;
                u
            }
        },
    };

    // Reconcile org membership + role drift from the IdP on every login.
    let memberships = UserOrgRepository::new(db);
    match memberships.get_role(user.id, org_id).await? {
        Some(existing) if existing == role => {}
        Some(_) => memberships.update_role(user.id, org_id, role).await?,
        None => {
            memberships
                .add_membership(&UserOrg {
                    id: Uuid::new_v4(),
                    user_id: user.id,
                    org_id,
                    role,
                    invited_by: None,
                    joined_at: chrono_now(),
                })
                .await?
        }
    }

    let _ = users.update_last_login(user.id).await;

    // Mint the session token RequireAuth consumes.
    let ttl = if ctx.session_ttl_hours == 0 {
        24
    } else {
        ctx.session_ttl_hours
    };
    let (session, token) = Session::new(
        user.id,
        ctx.ip_address.clone(),
        ctx.user_agent.clone(),
        ttl,
    );
    SessionRepository::new(db).create(&session).await?;

    // Audit the login — best-effort: a logging hiccup must not deny access the
    // IdP already granted.
    let mut entry = AuditEntry::builder(actions::SSO_LOGIN, ActorType::User, user.id.to_string())
        .org_id(org_id)
        .details(serde_json::json!({
            "issuer": identity.issuer,
            "subject": identity.subject,
            "email": identity.email,
            "role": role.to_string(),
            "protocol": config.protocol.as_str(),
        }));
    if let Some(ip) = &ctx.ip_address {
        entry = entry.ip_address(ip.clone());
    }
    if let Some(ua) = &ctx.user_agent {
        entry = entry.user_agent(ua.clone());
    }
    if let Err(e) = entry.log(db).await {
        tracing::error!(error = %e, user_id = %user.id, "failed to write sso.login audit");
    }

    Ok(EstablishedSession {
        token,
        user_id: user.id,
        role,
    })
}

/// Map the IdP's asserted groups to a Reaper [`OrgRole`], taking the
/// highest-privilege match and falling back to the config's `default_role`.
///
/// The result is an `OrgRole`, whose ceiling is `Owner` — which `role_to_scopes`
/// deliberately keeps below the platform `admin` scope. So no IdP group, however
/// named, can ever confer platform super-admin: the invariant is structural, not
/// a check that could be forgotten.
pub fn map_groups_to_role(config: &SsoConfig, groups: &[String]) -> OrgRole {
    let attr = config.attr_map();
    let default = config
        .default_role
        .parse::<OrgRole>()
        .unwrap_or(OrgRole::Viewer);

    let mut best: Option<OrgRole> = None;
    for g in groups {
        if let Some(role_str) = attr.group_map.get(g) {
            if let Ok(role) = role_str.parse::<OrgRole>() {
                best = Some(match best {
                    Some(b) => higher_role(b, role),
                    None => role,
                });
            }
        }
    }
    best.unwrap_or(default)
}

/// Privilege rank (higher = more privileged). `Owner` is the ceiling.
fn role_rank(role: OrgRole) -> u8 {
    match role {
        OrgRole::Viewer => 0,
        OrgRole::Developer => 1,
        OrgRole::Admin => 2,
        OrgRole::Owner => 3,
    }
}

fn higher_role(a: OrgRole, b: OrgRole) -> OrgRole {
    if role_rank(a) >= role_rank(b) {
        a
    } else {
        b
    }
}

/// `Utc::now()` indirection kept local so the broker has one clock source.
fn chrono_now() -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::sso::{SsoConfig, SsoProtocol};
    use crate::auth::scopes::{Permission, Scope};
    use crate::auth::middleware::role_to_scopes;

    fn config_with_map(default_role: &str, map_json: &str) -> SsoConfig {
        SsoConfig {
            id: Uuid::new_v4(),
            org_id: Uuid::new_v4(),
            protocol: SsoProtocol::Oidc,
            enabled: true,
            issuer: "https://idp.example.com".into(),
            client_id: "cid".into(),
            client_secret_encrypted: None,
            discovery_url: None,
            jwks_url: None,
            attr_map_json: Some(map_json.to_string()),
            allowed_domains_json: None,
            default_role: default_role.to_string(),
            created_at: chrono_now(),
            updated_at: chrono_now(),
        }
    }

    #[test]
    fn maps_groups_to_highest_role_else_default() {
        let cfg = config_with_map(
            "viewer",
            r#"{"group_map":{"reaper-admins":"owner","reaper-devs":"developer"}}"#,
        );
        // Highest match wins.
        assert_eq!(
            map_groups_to_role(&cfg, &["reaper-devs".into(), "reaper-admins".into()]),
            OrgRole::Owner
        );
        assert_eq!(
            map_groups_to_role(&cfg, &["reaper-devs".into()]),
            OrgRole::Developer
        );
        // No mapped group → default.
        assert_eq!(
            map_groups_to_role(&cfg, &["unrelated".into()]),
            OrgRole::Viewer
        );
    }

    #[test]
    fn no_idp_group_can_confer_platform_admin() {
        // Even an IdP group literally named "admin" caps at OrgRole::Owner, and
        // Owner's scopes never include the platform `admin` scope.
        let cfg = config_with_map("viewer", r#"{"group_map":{"admin":"owner"}}"#);
        let role = map_groups_to_role(&cfg, &["admin".into()]);
        assert_eq!(role, OrgRole::Owner);

        for role in [OrgRole::Owner, OrgRole::Admin, OrgRole::Developer, OrgRole::Viewer] {
            let perm = Permission::from_strings(&role_to_scopes(role));
            assert!(
                !perm.has(Scope::Admin),
                "{role:?} must never carry the platform admin scope"
            );
        }
    }
}
