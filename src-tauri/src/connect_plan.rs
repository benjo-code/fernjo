//! Resolves what a session needs before it can connect: effective host, port, username and
//! credential of one connection node, as an SSH plan or an RDP plan.
//!
//! The effective values come from `Tree::resolve_node` (inheritance is not re-derived here).
//! Secrets are read from the vault only when it is unlocked and `skip_vault` is false; they
//! leave this module as `fernjo_ssh::Credential` or a `SecretString` (secrecy types, redacted
//! `Debug`).

use fernjo_core::{
    CredentialId, DEFAULT_KEEPALIVE_SECS, DEFAULT_RDP_CLIPBOARD, DEFAULT_RDP_COLOR_DEPTH,
    DEFAULT_RDP_PORT, DEFAULT_SSH_PORT, NodeId, NodeType, Protocol, QuickTarget, RdpResolution,
    ResolvedNode, is_valid_domain,
};
use fernjo_ssh::Credential;
use fernjo_vault::{Secret, SecretKind, VaultError};
use secrecy::SecretString;

use crate::commands::{CommandError, with_store};
use crate::state::{AppState, VaultState};

/// Why a referenced credential is not available. `text` is the short caption for the tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingReason {
    /// The connection points at a credential that no longer exists.
    Dangling,
    /// There is no vault yet.
    VaultNone,
    /// The vault file cannot be used.
    VaultUnusable,
    /// The user chose to connect without the vault.
    Skipped,
    /// The credential is not of a kind this protocol can use (a private key for RDP).
    WrongKind,
}

impl MissingReason {
    pub fn text(self) -> &'static str {
        match self {
            Self::Dangling => "The stored credential no longer exists",
            Self::VaultNone => "There is no vault yet",
            Self::VaultUnusable => "The vault cannot be used",
            Self::Skipped => "Vault skipped",
            Self::WrongKind => "The credential is not a password",
        }
    }
}

/// What to authenticate with.
#[derive(Debug)]
pub enum CredentialPlan {
    /// The connection references no credential.
    None,
    /// A credential is referenced but not available.
    Missing(MissingReason),
    /// The vault is locked; unlock it and resolve again.
    NeedsVault,
    Ready(Credential),
}

/// Everything the session manager needs to open a connection. `Debug` never shows a secret.
#[derive(Debug)]
pub struct ConnectPlan {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub keepalive_secs: u32,
    pub credential: CredentialPlan,
}

/// What to authenticate an RDP session with (a password only).
#[derive(Debug)]
pub enum RdpCredentialPlan {
    /// The connection references no credential.
    None,
    /// A credential is referenced but not available.
    Missing(MissingReason),
    /// The vault is locked; unlock it and resolve again.
    NeedsVault,
    Ready(SecretString),
}

/// Everything the session manager needs to open an RDP connection. `Debug` never shows the
/// password.
#[derive(Debug)]
pub struct RdpPlan {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    /// The domain of the credential when it has one, else the connection's effective domain.
    pub domain: Option<String>,
    pub resolution: RdpResolution,
    pub color_depth: u8,
    pub clipboard: bool,
    pub credential: RdpCredentialPlan,
}

/// The plan for either protocol.
#[derive(Debug)]
pub enum Plan {
    Ssh(ConnectPlan),
    Rdp(RdpPlan),
}

fn non_blank(name: &str) -> Option<String> {
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_string())
}

fn error(code: &str, message: impl Into<String>) -> CommandError {
    CommandError {
        code: code.to_string(),
        message: message.into(),
    }
}

/// What the vault says about the credential a connection references.
enum Lookup {
    /// The connection references no credential.
    None,
    Missing(MissingReason),
    NeedsVault,
    Found {
        username: Option<String>,
        domain: Option<String>,
        secret: Secret,
    },
}

/// Looks the credential up. With `only_passwords` an entry of another kind is reported as
/// `WrongKind` without its secret being read out of the vault.
fn lookup(
    state: &AppState,
    credential: Option<CredentialId>,
    skip_vault: bool,
    only_passwords: bool,
) -> Result<Lookup, CommandError> {
    let Some(credential_id) = credential else {
        return Ok(Lookup::None);
    };
    if skip_vault {
        return Ok(Lookup::Missing(MissingReason::Skipped));
    }
    let guard = state.vault.lock().map_err(|_| CommandError::internal())?;
    let vault = match &*guard {
        VaultState::None => return Ok(Lookup::Missing(MissingReason::VaultNone)),
        VaultState::Unusable { .. } => return Ok(Lookup::Missing(MissingReason::VaultUnusable)),
        VaultState::Open(v) if !v.is_unlocked() => return Ok(Lookup::NeedsVault),
        VaultState::Open(v) => v,
    };
    let entry = match vault.get(credential_id) {
        Ok(entry) => entry,
        Err(VaultError::NotFound) => return Ok(Lookup::Missing(MissingReason::Dangling)),
        Err(e) => return Err(e.into()),
    };
    if only_passwords && entry.kind != SecretKind::Password {
        return Ok(Lookup::Missing(MissingReason::WrongKind));
    }
    Ok(Lookup::Found {
        username: entry.username.as_deref().and_then(non_blank),
        domain: entry.domain.as_deref().and_then(non_blank),
        secret: vault.reveal(credential_id)?,
    })
}

/// The effective values of a connection node (not a folder).
fn connection(state: &AppState, node_id: NodeId) -> Result<ResolvedNode, CommandError> {
    let node = with_store(state, |store| Ok(store.tree().resolve_node(node_id)?))?;
    if node.kind != NodeType::Connection {
        return Err(error("not_a_connection", "only connections can be opened"));
    }
    Ok(node)
}

/// The trimmed host of a connection, refused before any session, prompt or network traffic when
/// it is empty or could not be recorded safely in `known_hosts` or a certificate pin (see
/// `fernjo_ssh::validate_host`).
fn checked_host(node: &ResolvedNode) -> Result<String, CommandError> {
    let host = node.host.as_deref().unwrap_or_default().trim().to_string();
    if host.is_empty() {
        return Err(error("invalid_host", "the connection has no host"));
    }
    if fernjo_ssh::validate_host(&host).is_err() {
        return Err(error(
            "invalid_host",
            "the host name contains characters that are not allowed (use a DNS name or an IP address)",
        ));
    }
    Ok(host)
}

fn ssh_plan(
    state: &AppState,
    node: &ResolvedNode,
    skip_vault: bool,
) -> Result<ConnectPlan, CommandError> {
    let host = checked_host(node)?;
    let mut plan = ConnectPlan {
        host,
        port: node.port.effective.unwrap_or(DEFAULT_SSH_PORT),
        // `Tree` keeps whatever was stored (also "" or blanks): blank means unset here.
        username: node.username.effective.as_deref().and_then(non_blank),
        keepalive_secs: node
            .keepalive_secs
            .effective
            .unwrap_or(DEFAULT_KEEPALIVE_SECS),
        credential: CredentialPlan::None,
    };
    match lookup(state, node.credential.effective, skip_vault, false)? {
        Lookup::None => {}
        Lookup::Missing(reason) => plan.credential = CredentialPlan::Missing(reason),
        Lookup::NeedsVault => plan.credential = CredentialPlan::NeedsVault,
        Lookup::Found {
            username, secret, ..
        } => {
            // The connection's own or inherited username wins over the entry's.
            if plan.username.is_none() {
                plan.username = username;
            }
            plan.credential = CredentialPlan::Ready(match secret {
                Secret::Password { password } => Credential::Password(password),
                Secret::PrivateKey { key, passphrase } => {
                    Credential::PrivateKey { key, passphrase }
                }
            });
        }
    }
    Ok(plan)
}

fn rdp_plan(
    state: &AppState,
    node: &ResolvedNode,
    skip_vault: bool,
) -> Result<RdpPlan, CommandError> {
    let host = checked_host(node)?;
    let mut plan = RdpPlan {
        host,
        port: node.port.effective.unwrap_or(DEFAULT_RDP_PORT),
        username: node.username.effective.as_deref().and_then(non_blank),
        domain: node.domain.effective.as_deref().and_then(non_blank),
        resolution: node.rdp_resolution.effective.unwrap_or(RdpResolution::Fit),
        color_depth: node
            .rdp_color_depth
            .effective
            .unwrap_or(DEFAULT_RDP_COLOR_DEPTH),
        clipboard: node
            .rdp_clipboard
            .effective
            .unwrap_or(DEFAULT_RDP_CLIPBOARD),
        credential: RdpCredentialPlan::None,
    };
    match lookup(state, node.credential.effective, skip_vault, true)? {
        Lookup::None => {}
        Lookup::Missing(reason) => plan.credential = RdpCredentialPlan::Missing(reason),
        Lookup::NeedsVault => plan.credential = RdpCredentialPlan::NeedsVault,
        Lookup::Found {
            username,
            domain,
            secret,
        } => {
            if plan.username.is_none() {
                plan.username = username;
            }
            // the credential's domain wins over the connection's setting
            if let Some(domain) = domain.filter(|d| is_valid_domain(d)) {
                plan.domain = Some(domain);
            }
            plan.credential = match secret {
                Secret::Password { password } => RdpCredentialPlan::Ready(password),
                // `lookup` only lets passwords through here
                Secret::PrivateKey { .. } => RdpCredentialPlan::Missing(MissingReason::WrongKind),
            };
        }
    }
    Ok(plan)
}

/// Builds the plan for `node_id`, SSH or RDP by the node's protocol.
///
/// # Errors
/// `not_found`, `not_a_connection`, `invalid_host`, or a vault error other than a missing entry.
pub fn resolve_session(
    state: &AppState,
    node_id: NodeId,
    skip_vault: bool,
) -> Result<Plan, CommandError> {
    let node = connection(state, node_id)?;
    match node.protocol {
        Some(Protocol::Ssh) => ssh_plan(state, &node, skip_vault).map(Plan::Ssh),
        Some(Protocol::Rdp) => rdp_plan(state, &node, skip_vault).map(Plan::Rdp),
        None => Err(error("not_a_connection", "only connections can be opened")),
    }
}

/// What a session connects to: a node of the tree, or a target typed into quick connect (never
/// stored in the tree).
#[derive(Debug, Clone)]
pub enum Target {
    Node(NodeId),
    Quick(QuickTarget),
}

/// Builds the plan for a quick target. Nothing is read from or written to the tree or the vault;
/// the password (and an RDP user name) come through the session prompts. The host is checked
/// here again because the webview is not trusted.
///
/// # Errors
/// `invalid_host`.
pub fn resolve_quick(target: &QuickTarget) -> Result<Plan, CommandError> {
    let host = target.host.trim().to_string();
    if host.is_empty() || fernjo_ssh::validate_host(&host).is_err() {
        return Err(error(
            "invalid_host",
            "the host name contains characters that are not allowed (use a DNS name or an IP address)",
        ));
    }
    // the rest of the structure (port, user) by the parser's own rules
    fernjo_core::validate_quick(target).map_err(|e| error(e.code, e.message()))?;
    let username = target.user.as_deref().and_then(non_blank);
    Ok(match target.protocol {
        Protocol::Ssh => Plan::Ssh(ConnectPlan {
            host,
            port: target.port,
            username,
            keepalive_secs: DEFAULT_KEEPALIVE_SECS,
            credential: CredentialPlan::None,
        }),
        Protocol::Rdp => Plan::Rdp(RdpPlan {
            host,
            port: target.port,
            username,
            domain: None,
            resolution: RdpResolution::Fit,
            color_depth: DEFAULT_RDP_COLOR_DEPTH,
            clipboard: false,
            credential: RdpCredentialPlan::None,
        }),
    })
}

/// Builds the plan for a session target.
///
/// # Errors
/// What `resolve_session` or `resolve_quick` report.
pub fn resolve_target(
    state: &AppState,
    target: &Target,
    skip_vault: bool,
) -> Result<Plan, CommandError> {
    match target {
        Target::Node(id) => resolve_session(state, *id, skip_vault),
        Target::Quick(quick) => resolve_quick(quick),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands;
    use crate::vault_commands::{CredentialDraft, create_credential, vault_create, vault_lock};
    use fernjo_core::NodePatch;
    use secrecy::{ExposeSecret, SecretString};
    use serde_json::{Value, json};
    use std::path::Path;

    /// The SSH plan of a node (the tests below that are about SSH only use SSH nodes).
    fn resolve_ssh(
        state: &AppState,
        node_id: NodeId,
        skip_vault: bool,
    ) -> Result<ConnectPlan, CommandError> {
        match resolve_session(state, node_id, skip_vault)? {
            Plan::Ssh(plan) => Ok(plan),
            Plan::Rdp(_) => panic!("expected an SSH plan"),
        }
    }

    const MASTER: &str = "SENTINEL-master-1a2b3c";
    const PW: &str = "SENTINEL-password-7f3a9b";
    const KEY: &str = "SENTINEL-privatekey-9c1b2d";
    const PASS: &str = "SENTINEL-passphrase-4d2e8f";
    const DANGLING: &str = "6f1d3c52-8a0e-4b8e-9d57-2f5b1c0a7e11";

    fn state(dir: &Path) -> AppState {
        AppState::open(Some(dir.to_path_buf()))
    }

    fn patch(v: Value) -> NodePatch {
        serde_json::from_value(v).unwrap()
    }

    fn draft(v: Value) -> CredentialDraft {
        serde_json::from_value(v).unwrap()
    }

    fn code<T>(r: Result<T, CommandError>) -> String {
        match r {
            Ok(_) => "ok".to_string(),
            Err(e) => e.code,
        }
    }

    fn with_vault_state(dir: &Path) -> AppState {
        let st = state(dir);
        vault_create(&st, SecretString::from(MASTER), false).unwrap();
        st
    }

    fn conn(st: &AppState, parent: Option<NodeId>, host: &str) -> NodeId {
        commands::create_connection(st, parent, None, "c".into(), Protocol::Ssh, host.into())
            .unwrap()
            .id
            .unwrap()
    }

    fn folder(st: &AppState) -> NodeId {
        commands::create_folder(st, None, None, "f".into())
            .unwrap()
            .id
            .unwrap()
    }

    fn pw_entry(st: &AppState, username: Option<&str>) -> NodeId {
        create_credential(
            st,
            draft(json!({"name": "pw", "kind": "password", "username": username, "password": PW})),
        )
        .unwrap()
        .id
    }

    fn key_entry(st: &AppState) -> NodeId {
        create_credential(
            st,
            draft(
                json!({"name": "k", "kind": "privateKey", "privateKey": KEY, "passphrase": PASS}),
            ),
        )
        .unwrap()
        .id
    }

    fn set(st: &AppState, id: NodeId, v: Value) {
        commands::update_node(st, id, patch(v)).unwrap();
    }

    #[test]
    fn connection_username_beats_inherited_beats_entry_username() {
        let dir = tempfile::tempdir().unwrap();
        let st = with_vault_state(dir.path());
        let entry = pw_entry(&st, Some("entryuser"));
        let f = folder(&st);
        let c = conn(&st, Some(f), "h");
        set(&st, c, json!({"credential": entry}));
        set(&st, f, json!({"username": "folderuser"}));
        assert_eq!(
            resolve_ssh(&st, c, false).unwrap().username.as_deref(),
            Some("folderuser")
        );
        set(&st, c, json!({"username": "own"}));
        assert_eq!(
            resolve_ssh(&st, c, false).unwrap().username.as_deref(),
            Some("own")
        );
    }

    #[test]
    fn a_blank_username_counts_as_unset() {
        let dir = tempfile::tempdir().unwrap();
        let st = with_vault_state(dir.path());
        let entry = pw_entry(&st, Some("entryuser"));
        let c = conn(&st, None, "h");
        set(&st, c, json!({"credential": entry, "username": "  "}));
        assert_eq!(
            resolve_ssh(&st, c, false).unwrap().username.as_deref(),
            Some("entryuser")
        );
        set(&st, c, json!({"username": "  padded "}));
        assert_eq!(
            resolve_ssh(&st, c, false).unwrap().username.as_deref(),
            Some("padded")
        );
        set(&st, c, json!({"credential": null, "username": ""}));
        assert_eq!(resolve_ssh(&st, c, false).unwrap().username, None);
    }

    #[test]
    fn entry_username_is_the_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let st = with_vault_state(dir.path());
        let entry = pw_entry(&st, Some("entryuser"));
        let c = conn(&st, None, "h");
        set(&st, c, json!({"credential": entry}));
        assert_eq!(
            resolve_ssh(&st, c, false).unwrap().username.as_deref(),
            Some("entryuser")
        );
    }

    #[test]
    fn no_username_anywhere_gives_none() {
        let dir = tempfile::tempdir().unwrap();
        let st = with_vault_state(dir.path());
        let entry = pw_entry(&st, None);
        let c = conn(&st, None, "h");
        set(&st, c, json!({"credential": entry}));
        assert_eq!(resolve_ssh(&st, c, false).unwrap().username, None);
    }

    #[test]
    fn port_and_keepalive_are_inherited_or_default() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path());
        let f = folder(&st);
        let c = conn(&st, Some(f), "h");
        let plain = resolve_ssh(&st, c, false).unwrap();
        assert_eq!((plain.port, plain.keepalive_secs), (22, 30));
        assert_eq!(plain.host.as_str(), "h");
        set(&st, f, json!({"port": 2222, "keepaliveSecs": 60}));
        let inherited = resolve_ssh(&st, c, false).unwrap();
        assert_eq!((inherited.port, inherited.keepalive_secs), (2222, 60));
        set(&st, c, json!({"port": 2200}));
        assert_eq!(resolve_ssh(&st, c, false).unwrap().port, 2200);
    }

    #[test]
    fn locked_vault_with_reference_gives_needs_vault_and_no_secret_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let st = with_vault_state(dir.path());
        let entry = pw_entry(&st, Some("entryuser"));
        let c = conn(&st, None, "h");
        set(&st, c, json!({"credential": entry}));
        vault_lock(&st).unwrap();
        let plan = resolve_ssh(&st, c, false).unwrap();
        assert!(matches!(plan.credential, CredentialPlan::NeedsVault));
        // the entry's username lives in the encrypted body too, so it is not read either
        assert_eq!(plan.username, None);
    }

    #[test]
    fn skip_vault_gives_missing_skipped_and_ignores_entry_username() {
        let dir = tempfile::tempdir().unwrap();
        let st = with_vault_state(dir.path());
        let entry = pw_entry(&st, Some("entryuser"));
        let c = conn(&st, None, "h");
        set(&st, c, json!({"credential": entry}));
        let plan = resolve_ssh(&st, c, true).unwrap();
        assert!(matches!(
            plan.credential,
            CredentialPlan::Missing(MissingReason::Skipped)
        ));
        assert_eq!(plan.username, None);
    }

    #[test]
    fn dangling_reference_gives_missing_dangling() {
        let dir = tempfile::tempdir().unwrap();
        let st = with_vault_state(dir.path());
        let c = conn(&st, None, "h");
        set(&st, c, json!({"credential": DANGLING}));
        assert!(matches!(
            resolve_ssh(&st, c, false).unwrap().credential,
            CredentialPlan::Missing(MissingReason::Dangling)
        ));
    }

    #[test]
    fn unusable_vault_gives_missing_vault_unusable() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("vault.fernjo"), b"garbage").unwrap();
        let st = state(dir.path());
        let c = conn(&st, None, "h");
        set(&st, c, json!({"credential": DANGLING}));
        assert!(matches!(
            resolve_ssh(&st, c, false).unwrap().credential,
            CredentialPlan::Missing(MissingReason::VaultUnusable)
        ));
    }

    #[test]
    fn missing_vault_gives_missing_vault_none() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path());
        let c = conn(&st, None, "h");
        set(&st, c, json!({"credential": DANGLING}));
        assert!(matches!(
            resolve_ssh(&st, c, false).unwrap().credential,
            CredentialPlan::Missing(MissingReason::VaultNone)
        ));
    }

    #[test]
    fn no_reference_gives_none() {
        let dir = tempfile::tempdir().unwrap();
        let st = with_vault_state(dir.path());
        let c = conn(&st, None, "h");
        for skip in [false, true] {
            assert!(matches!(
                resolve_ssh(&st, c, skip).unwrap().credential,
                CredentialPlan::None
            ));
        }
    }

    #[test]
    fn password_entry_becomes_password_credential() {
        let dir = tempfile::tempdir().unwrap();
        let st = with_vault_state(dir.path());
        let entry = pw_entry(&st, None);
        let c = conn(&st, None, "h");
        set(&st, c, json!({"credential": entry}));
        let CredentialPlan::Ready(Credential::Password(p)) =
            resolve_ssh(&st, c, false).unwrap().credential
        else {
            panic!("expected a password credential");
        };
        assert_eq!(p.expose_secret(), PW);
    }

    #[test]
    fn private_key_entry_keeps_passphrase() {
        let dir = tempfile::tempdir().unwrap();
        let st = with_vault_state(dir.path());
        let entry = key_entry(&st);
        let c = conn(&st, None, "h");
        set(&st, c, json!({"credential": entry}));
        let CredentialPlan::Ready(Credential::PrivateKey { key, passphrase }) =
            resolve_ssh(&st, c, false).unwrap().credential
        else {
            panic!("expected a key credential");
        };
        assert_eq!(key.expose_secret(), KEY);
        assert_eq!(passphrase.unwrap().expose_secret(), PASS);
    }

    // ---- RDP -----------------------------------------------------------------------------

    fn rdp(st: &AppState, parent: Option<NodeId>, host: &str) -> NodeId {
        commands::create_connection(st, parent, None, "r".into(), Protocol::Rdp, host.into())
            .unwrap()
            .id
            .unwrap()
    }

    fn rdp_plan(st: &AppState, id: NodeId, skip_vault: bool) -> RdpPlan {
        match resolve_session(st, id, skip_vault).unwrap() {
            Plan::Rdp(plan) => plan,
            other => panic!("expected an RDP plan, got {other:?}"),
        }
    }

    fn entry_with_domain(st: &AppState, username: Option<&str>, domain: Option<&str>) -> NodeId {
        create_credential(
            st,
            draft(json!({
                "name": "pw", "kind": "password", "username": username,
                "domain": domain, "password": PW
            })),
        )
        .unwrap()
        .id
    }

    #[test]
    fn resolve_session_returns_the_rdp_plan_with_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path());
        let id = rdp(&st, None, " dc01 ");
        let plan = rdp_plan(&st, id, false);
        assert_eq!(plan.host, "dc01");
        assert_eq!(plan.port, 3389);
        assert_eq!(plan.username, None);
        assert_eq!(plan.domain, None);
        assert_eq!(plan.resolution, RdpResolution::Fit);
        assert_eq!(plan.color_depth, 32);
        assert!(plan.clipboard);
        assert!(matches!(plan.credential, RdpCredentialPlan::None));
        // an SSH node still gets the SSH plan
        let ssh = conn(&st, None, "h");
        assert!(matches!(
            resolve_session(&st, ssh, false).unwrap(),
            Plan::Ssh(_)
        ));
    }

    #[test]
    fn rdp_settings_are_inherited_from_a_folder() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path());
        let f = folder(&st);
        set(
            &st,
            f,
            json!({
                "port": 3390, "domain": "CORP", "rdpColorDepth": 16, "rdpClipboard": false,
                "rdpResolution": {"mode": "fixed", "width": 1920, "height": 1080}
            }),
        );
        let id = rdp(&st, Some(f), "h");
        let plan = rdp_plan(&st, id, false);
        assert_eq!(plan.port, 3390);
        assert_eq!(plan.domain.as_deref(), Some("CORP"));
        assert_eq!(plan.color_depth, 16);
        assert!(!plan.clipboard);
        assert_eq!(
            plan.resolution,
            RdpResolution::Fixed {
                width: 1920,
                height: 1080
            }
        );
    }

    #[test]
    fn credential_domain_beats_node_domain() {
        let dir = tempfile::tempdir().unwrap();
        let st = with_vault_state(dir.path());
        let id = rdp(&st, None, "h");
        set(&st, id, json!({"domain": "NODE"}));
        let with = entry_with_domain(&st, Some("admin"), Some("ENTRY"));
        set(&st, id, json!({"credential": with}));
        assert_eq!(rdp_plan(&st, id, false).domain.as_deref(), Some("ENTRY"));
        // an entry without a domain leaves the node's domain in force
        let without = entry_with_domain(&st, Some("admin"), None);
        set(&st, id, json!({"credential": without}));
        assert_eq!(rdp_plan(&st, id, false).domain.as_deref(), Some("NODE"));
    }

    #[test]
    fn blank_domain_is_unset() {
        let dir = tempfile::tempdir().unwrap();
        let st = with_vault_state(dir.path());
        let id = rdp(&st, None, "h");
        set(&st, id, json!({"domain": "   "}));
        assert_eq!(rdp_plan(&st, id, false).domain, None);
        // a blank domain on the entry does not hide the node's
        set(&st, id, json!({"domain": "NODE"}));
        let blank = entry_with_domain(&st, None, Some("  "));
        set(&st, id, json!({"credential": blank}));
        assert_eq!(rdp_plan(&st, id, false).domain.as_deref(), Some("NODE"));
    }

    #[test]
    fn private_key_credential_is_missing_wrong_kind() {
        let dir = tempfile::tempdir().unwrap();
        let st = with_vault_state(dir.path());
        let id = rdp(&st, None, "h");
        let key = key_entry(&st);
        set(&st, id, json!({"credential": key}));
        let plan = rdp_plan(&st, id, false);
        assert!(matches!(
            plan.credential,
            RdpCredentialPlan::Missing(MissingReason::WrongKind)
        ));
        assert_eq!(
            MissingReason::WrongKind.text(),
            "The credential is not a password"
        );
        assert!(!format!("{plan:?}").contains(KEY));
    }

    #[test]
    fn typed_domain_backslash_user_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path());
        let id = rdp(&st, None, "h");
        set(&st, id, json!({"username": "CORP\\admin"}));
        let plan = rdp_plan(&st, id, false);
        assert_eq!(plan.username.as_deref(), Some("CORP\\admin"));
        assert_eq!(plan.domain, None);
        // with a configured domain the typed form is kept as is and the domain is passed too
        set(
            &st,
            id,
            json!({"domain": "CORP", "username": "admin@corp.example"}),
        );
        let plan = rdp_plan(&st, id, false);
        assert_eq!(plan.username.as_deref(), Some("admin@corp.example"));
        assert_eq!(plan.domain.as_deref(), Some("CORP"));
    }

    #[test]
    fn rdp_username_precedence_and_a_ready_password() {
        let dir = tempfile::tempdir().unwrap();
        let st = with_vault_state(dir.path());
        let id = rdp(&st, None, "h");
        let entry = entry_with_domain(&st, Some("entryuser"), None);
        set(&st, id, json!({"credential": entry}));
        let plan = rdp_plan(&st, id, false);
        assert_eq!(plan.username.as_deref(), Some("entryuser"));
        match plan.credential {
            RdpCredentialPlan::Ready(password) => assert_eq!(password.expose_secret(), PW),
            other => panic!("expected a ready password, got {other:?}"),
        }
        set(&st, id, json!({"username": "own"}));
        assert_eq!(rdp_plan(&st, id, false).username.as_deref(), Some("own"));
    }

    #[test]
    fn locked_vault_is_needs_vault_for_rdp() {
        let dir = tempfile::tempdir().unwrap();
        let st = with_vault_state(dir.path());
        let id = rdp(&st, None, "h");
        let entry = pw_entry(&st, None);
        set(&st, id, json!({"credential": entry}));
        vault_lock(&st).unwrap();
        assert!(matches!(
            rdp_plan(&st, id, false).credential,
            RdpCredentialPlan::NeedsVault
        ));
    }

    #[test]
    fn skip_vault_is_missing_skipped_for_rdp() {
        let dir = tempfile::tempdir().unwrap();
        let st = with_vault_state(dir.path());
        let id = rdp(&st, None, "h");
        let entry = pw_entry(&st, None);
        set(&st, id, json!({"credential": entry}));
        assert!(matches!(
            rdp_plan(&st, id, true).credential,
            RdpCredentialPlan::Missing(MissingReason::Skipped)
        ));
    }

    #[test]
    fn a_dangling_credential_is_missing_dangling_for_rdp() {
        let dir = tempfile::tempdir().unwrap();
        let st = with_vault_state(dir.path());
        let id = rdp(&st, None, "h");
        set(&st, id, json!({"credential": DANGLING}));
        assert!(matches!(
            rdp_plan(&st, id, false).credential,
            RdpCredentialPlan::Missing(MissingReason::Dangling)
        ));
    }

    #[test]
    fn invalid_host_is_refused_for_rdp() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path());
        for bad in ["", "   ", "a b", "bad,host"] {
            let id = rdp(&st, None, bad);
            assert_eq!(
                code(resolve_session(&st, id, false)),
                "invalid_host",
                "{bad:?}"
            );
        }
    }

    #[test]
    fn rdp_plan_debug_shows_no_password() {
        let dir = tempfile::tempdir().unwrap();
        let st = with_vault_state(dir.path());
        let id = rdp(&st, None, "h");
        let entry = pw_entry(&st, Some("u"));
        set(&st, id, json!({"credential": entry}));
        let plan = resolve_session(&st, id, false).unwrap();
        let text = format!("{plan:?}");
        assert!(!text.contains(PW), "{text}");
        assert!(text.contains("Ready"));
    }

    #[test]
    fn folder_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path());
        let f = folder(&st);
        assert_eq!(code(resolve_ssh(&st, f, false)), "not_a_connection");
        assert_eq!(code(resolve_ssh(&st, NodeId::new_v4(), false)), "not_found");
    }

    #[test]
    fn empty_host_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path());
        let c = conn(&st, None, "   ");
        assert_eq!(code(resolve_ssh(&st, c, false)), "invalid_host");
    }

    #[test]
    fn a_host_that_cannot_be_recorded_in_known_hosts_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path());
        for host in [
            "evil,victim.corp",
            "a b",
            "x
victim2",
            "*.corp",
            "[::1]",
            &"a".repeat(256),
        ] {
            let c = conn(&st, None, host);
            assert_eq!(code(resolve_ssh(&st, c, false)), "invalid_host", "{host:?}");
        }
        for host in ["web-01.lab", "10.0.0.1", "fe80::1%eth0", " padded.example "] {
            let c = conn(&st, None, host);
            assert_eq!(code(resolve_ssh(&st, c, false)), "ok", "{host:?}");
        }
    }

    #[test]
    fn plan_debug_output_contains_no_secret() {
        let dir = tempfile::tempdir().unwrap();
        let st = with_vault_state(dir.path());
        let pw = pw_entry(&st, None);
        let key = key_entry(&st);
        for entry in [pw, key] {
            let c = conn(&st, None, "h");
            set(&st, c, json!({"credential": entry}));
            let plan = resolve_ssh(&st, c, false).unwrap();
            assert!(matches!(plan.credential, CredentialPlan::Ready(_)));
            let text = format!("{plan:?}");
            for s in [MASTER, PW, KEY, PASS] {
                assert!(!text.contains(s), "{text}");
            }
        }
    }

    fn quick(protocol: Protocol, host: &str, port: u16, user: Option<&str>) -> QuickTarget {
        QuickTarget {
            protocol,
            host: host.to_string(),
            port,
            user: user.map(str::to_string),
        }
    }

    #[test]
    fn quick_ssh_plan_has_no_credential_and_the_typed_user() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path());
        let target = Target::Quick(quick(Protocol::Ssh, "example.com", 2222, Some("root")));
        let Plan::Ssh(plan) = resolve_target(&st, &target, false).unwrap() else {
            panic!("expected an SSH plan");
        };
        assert_eq!(plan.host, "example.com");
        assert_eq!(plan.port, 2222);
        assert_eq!(plan.username.as_deref(), Some("root"));
        assert_eq!(plan.keepalive_secs, DEFAULT_KEEPALIVE_SECS);
        assert!(matches!(plan.credential, CredentialPlan::None));
    }

    #[test]
    fn quick_rdp_plan_has_the_clipboard_off_and_no_credential() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path());
        let target = Target::Quick(quick(Protocol::Rdp, "10.0.0.5", 3389, None));
        let Plan::Rdp(plan) = resolve_target(&st, &target, false).unwrap() else {
            panic!("expected an RDP plan");
        };
        assert_eq!((plan.host.as_str(), plan.port), ("10.0.0.5", 3389));
        assert_eq!(plan.username, None);
        assert_eq!(plan.domain, None);
        assert_eq!(plan.resolution, RdpResolution::Fit);
        assert_eq!(plan.color_depth, DEFAULT_RDP_COLOR_DEPTH);
        assert!(!plan.clipboard);
        assert!(matches!(plan.credential, RdpCredentialPlan::None));
    }

    #[test]
    fn a_quick_host_outside_the_allow_list_is_invalid_host() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path());
        for host in ["bad host", "a/b", "", "a;b"] {
            for protocol in [Protocol::Ssh, Protocol::Rdp] {
                let target = Target::Quick(quick(protocol, host, 22, Some("u")));
                assert_eq!(
                    code(resolve_target(&st, &target, false)),
                    "invalid_host",
                    "{host:?}"
                );
            }
        }
    }

    #[test]
    fn a_hand_built_quick_target_is_checked_like_a_parsed_one() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path());
        let cases = [
            (quick(Protocol::Ssh, "h", 0, Some("u")), "quick_bad_port"),
            (quick(Protocol::Rdp, "h", 0, None), "quick_bad_port"),
            (
                quick(Protocol::Ssh, "h", 22, Some("u\nx")),
                "quick_bad_target",
            ),
            (
                quick(Protocol::Ssh, "h", 22, Some("u\0")),
                "quick_bad_target",
            ),
            (
                quick(Protocol::Ssh, "h", 22, Some("a@b")),
                "quick_bad_target",
            ),
            (
                quick(Protocol::Ssh, "h", 22, Some("a b")),
                "quick_bad_target",
            ),
            (
                quick(Protocol::Ssh, "h", 22, Some(&"u".repeat(300))),
                "quick_bad_target",
            ),
            (quick(Protocol::Ssh, "h", 22, None), "quick_user_required"),
        ];
        for (target, expected) in cases {
            let t = Target::Quick(target.clone());
            assert_eq!(code(resolve_target(&st, &t, false)), expected, "{target:?}");
        }
    }

    #[test]
    fn nothing_is_written_to_connections_json_by_resolving_a_quick_target() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path());
        commands::create_connection(&st, None, None, "c".into(), Protocol::Ssh, "h".into())
            .unwrap();
        let file = dir.path().join("connections.json");
        let before = std::fs::read(&file).unwrap();
        for protocol in [Protocol::Ssh, Protocol::Rdp] {
            let target = Target::Quick(quick(protocol, "example.com", 22, Some("u")));
            resolve_target(&st, &target, false).unwrap();
        }
        assert_eq!(std::fs::read(&file).unwrap(), before);
        assert!(!dir.path().join("known_hosts").exists());
    }
}
