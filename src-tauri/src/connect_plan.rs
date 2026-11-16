//! Resolves what a session needs before it can connect: effective host, port, username and
//! credential of one connection node.
//!
//! The effective values come from `Tree::resolve_node` (inheritance is not re-derived here).
//! Secrets are read from the vault only when it is unlocked and `skip_vault` is false; they
//! leave this module as `fernjo_ssh::Credential` (secrecy types, redacted `Debug`).

use fernjo_core::{DEFAULT_KEEPALIVE_SECS, DEFAULT_SSH_PORT, NodeId, NodeType, Protocol};
use fernjo_ssh::Credential;
use fernjo_vault::{Secret, VaultError};

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
}

impl MissingReason {
    pub fn text(self) -> &'static str {
        match self {
            Self::Dangling => "The stored credential no longer exists",
            Self::VaultNone => "There is no vault yet",
            Self::VaultUnusable => "The vault cannot be used",
            Self::Skipped => "Vault skipped",
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

/// Builds the plan for `node_id`.
///
/// # Errors
/// `not_found`, `not_a_connection`, `unsupported_protocol`, `invalid_host`, or a vault error
/// other than a missing entry (for example `internal`).
pub fn resolve(
    state: &AppState,
    node_id: NodeId,
    skip_vault: bool,
) -> Result<ConnectPlan, CommandError> {
    let node = with_store(state, |store| Ok(store.tree().resolve_node(node_id)?))?;
    if node.kind != NodeType::Connection {
        return Err(error("not_a_connection", "only connections can be opened"));
    }
    if node.protocol != Some(Protocol::Ssh) {
        return Err(error(
            "unsupported_protocol",
            "RDP sessions are not available yet",
        ));
    }
    let host = node.host.as_deref().unwrap_or_default().trim().to_string();
    if host.is_empty() {
        return Err(error("invalid_host", "the connection has no host"));
    }
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
    let Some(credential_id) = node.credential.effective else {
        return Ok(plan);
    };
    if skip_vault {
        plan.credential = CredentialPlan::Missing(MissingReason::Skipped);
        return Ok(plan);
    }
    let guard = state.vault.lock().map_err(|_| CommandError::internal())?;
    let vault = match &*guard {
        VaultState::None => {
            plan.credential = CredentialPlan::Missing(MissingReason::VaultNone);
            return Ok(plan);
        }
        VaultState::Unusable { .. } => {
            plan.credential = CredentialPlan::Missing(MissingReason::VaultUnusable);
            return Ok(plan);
        }
        VaultState::Open(v) if !v.is_unlocked() => {
            plan.credential = CredentialPlan::NeedsVault;
            return Ok(plan);
        }
        VaultState::Open(v) => v,
    };
    let entry = match vault.get(credential_id) {
        Ok(entry) => entry,
        Err(VaultError::NotFound) => {
            plan.credential = CredentialPlan::Missing(MissingReason::Dangling);
            return Ok(plan);
        }
        Err(e) => return Err(e.into()),
    };
    // The connection's own or inherited username wins over the entry's.
    if plan.username.is_none() {
        plan.username = entry.username.as_deref().and_then(non_blank);
    }
    plan.credential = CredentialPlan::Ready(match vault.reveal(credential_id)? {
        Secret::Password { password } => Credential::Password(password),
        Secret::PrivateKey { key, passphrase } => Credential::PrivateKey { key, passphrase },
    });
    Ok(plan)
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
            resolve(&st, c, false).unwrap().username.as_deref(),
            Some("folderuser")
        );
        set(&st, c, json!({"username": "own"}));
        assert_eq!(
            resolve(&st, c, false).unwrap().username.as_deref(),
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
            resolve(&st, c, false).unwrap().username.as_deref(),
            Some("entryuser")
        );
        set(&st, c, json!({"username": "  padded "}));
        assert_eq!(
            resolve(&st, c, false).unwrap().username.as_deref(),
            Some("padded")
        );
        set(&st, c, json!({"credential": null, "username": ""}));
        assert_eq!(resolve(&st, c, false).unwrap().username, None);
    }

    #[test]
    fn entry_username_is_the_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let st = with_vault_state(dir.path());
        let entry = pw_entry(&st, Some("entryuser"));
        let c = conn(&st, None, "h");
        set(&st, c, json!({"credential": entry}));
        assert_eq!(
            resolve(&st, c, false).unwrap().username.as_deref(),
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
        assert_eq!(resolve(&st, c, false).unwrap().username, None);
    }

    #[test]
    fn port_and_keepalive_are_inherited_or_default() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path());
        let f = folder(&st);
        let c = conn(&st, Some(f), "h");
        let plain = resolve(&st, c, false).unwrap();
        assert_eq!((plain.port, plain.keepalive_secs), (22, 30));
        assert_eq!(plain.host.as_str(), "h");
        set(&st, f, json!({"port": 2222, "keepaliveSecs": 60}));
        let inherited = resolve(&st, c, false).unwrap();
        assert_eq!((inherited.port, inherited.keepalive_secs), (2222, 60));
        set(&st, c, json!({"port": 2200}));
        assert_eq!(resolve(&st, c, false).unwrap().port, 2200);
    }

    #[test]
    fn locked_vault_with_reference_gives_needs_vault_and_no_secret_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let st = with_vault_state(dir.path());
        let entry = pw_entry(&st, Some("entryuser"));
        let c = conn(&st, None, "h");
        set(&st, c, json!({"credential": entry}));
        vault_lock(&st).unwrap();
        let plan = resolve(&st, c, false).unwrap();
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
        let plan = resolve(&st, c, true).unwrap();
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
            resolve(&st, c, false).unwrap().credential,
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
            resolve(&st, c, false).unwrap().credential,
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
            resolve(&st, c, false).unwrap().credential,
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
                resolve(&st, c, skip).unwrap().credential,
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
            resolve(&st, c, false).unwrap().credential
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
            resolve(&st, c, false).unwrap().credential
        else {
            panic!("expected a key credential");
        };
        assert_eq!(key.expose_secret(), KEY);
        assert_eq!(passphrase.unwrap().expose_secret(), PASS);
    }

    #[test]
    fn rdp_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path());
        let id =
            commands::create_connection(&st, None, None, "r".into(), Protocol::Rdp, "h".into())
                .unwrap()
                .id
                .unwrap();
        assert_eq!(code(resolve(&st, id, false)), "unsupported_protocol");
    }

    #[test]
    fn folder_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path());
        let f = folder(&st);
        assert_eq!(code(resolve(&st, f, false)), "not_a_connection");
        assert_eq!(code(resolve(&st, NodeId::new_v4(), false)), "not_found");
    }

    #[test]
    fn empty_host_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path());
        let c = conn(&st, None, "   ");
        assert_eq!(code(resolve(&st, c, false)), "invalid_host");
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
            let plan = resolve(&st, c, false).unwrap();
            assert!(matches!(plan.credential, CredentialPlan::Ready(_)));
            let text = format!("{plan:?}");
            for s in [MASTER, PW, KEY, PASS] {
                assert!(!text.contains(s), "{text}");
            }
        }
    }
}
