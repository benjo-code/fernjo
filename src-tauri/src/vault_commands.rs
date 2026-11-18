//! Tauri commands for the vault and its credential entries.
//!
//! Like `commands.rs`: plain functions over `&AppState` (testable without Tauri) plus thin
//! wrappers in [`handlers`]. Secrets stay in Rust: incoming secrets become `SecretString`
//! at the command boundary, no response type has a secret field except
//! [`RevealedSecret`], and no error message contains one.

use std::fmt;

use fernjo_core::{CredentialId, NodeKind};
use fernjo_vault::{
    EntrySummary, EntryUpdate, FILE_NAME as VAULT_FILE_NAME, NewEntry, Secret, SecretKind, Vault,
    VaultError,
};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Deserializer, Serialize};

use crate::commands::{CommandError, Mutation, ensure_writable, snapshot, with_store};
use crate::state::{AppState, AutoUnlockIssue, VaultState};

impl From<VaultError> for CommandError {
    fn from(e: VaultError) -> Self {
        CommandError {
            code: e.code().to_string(),
            message: e.to_string(),
        }
    }
}

fn error(code: &str, message: impl Into<String>) -> CommandError {
    CommandError {
        code: code.to_string(),
        message: message.into(),
    }
}

fn invalid(message: &str) -> CommandError {
    error("invalid_credential", message)
}

/// What commands report about the vault.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultStatus {
    /// `none`, `locked`, `unlocked` or `unusable`.
    pub state: &'static str,
    /// Why the vault file cannot be used (`unusable` only).
    pub reason: Option<String>,
    pub path: Option<String>,
    /// The key store holds this vault's key.
    pub auto_unlock: bool,
    /// Why the automatic unlock at startup did not work (`locked` only): `key_not_accepted`,
    /// `keychain_unavailable`, `unavailable` or `rolled_back` (the file is older than the last
    /// version this device opened). A stored key is never deleted.
    pub auto_unlock_issue: Option<&'static str>,
}

fn view(state: &AppState, vault: &VaultState) -> VaultStatus {
    match vault {
        VaultState::None => VaultStatus {
            state: "none",
            reason: None,
            path: None,
            auto_unlock: false,
            auto_unlock_issue: None,
        },
        VaultState::Open(v) => VaultStatus {
            state: if v.is_unlocked() {
                "unlocked"
            } else {
                "locked"
            },
            reason: None,
            path: Some(v.path().display().to_string()),
            // A keychain error reads as "not stored".
            auto_unlock: matches!(state.keystore.get(v.id()), Ok(Some(_))),
            auto_unlock_issue: if v.is_unlocked() {
                None
            } else {
                state
                    .auto_unlock_issue
                    .lock()
                    .ok()
                    .and_then(|g| g.map(AutoUnlockIssue::code))
            },
        },
        VaultState::Unusable { reason, path } => VaultStatus {
            state: "unusable",
            reason: Some(reason.clone()),
            path: Some(path.display().to_string()),
            auto_unlock: false,
            auto_unlock_issue: None,
        },
    }
}

fn clear_auto_unlock_issue(state: &AppState) {
    if let Ok(mut issue) = state.auto_unlock_issue.lock() {
        *issue = None;
    }
}

/// Runs `f` on the vault, or reports why there is none.
pub(crate) fn with_vault<T>(
    state: &AppState,
    f: impl FnOnce(&mut Vault) -> Result<T, CommandError>,
) -> Result<T, CommandError> {
    let mut guard = state.vault.lock().map_err(|_| CommandError::internal())?;
    match &mut *guard {
        VaultState::Open(v) => f(v),
        VaultState::None => Err(error("no_vault", "there is no vault yet")),
        VaultState::Unusable { reason, path } => Err(error(
            "vault_unusable",
            format!("the vault file {} cannot be used: {reason}", path.display()),
        )),
    }
}

/// Stores the data key in the key store. A failure is deliberately ignored: the vault
/// operation succeeded, and the status then reports `autoUnlock: false`.
fn remember_key(state: &AppState, vault: &Vault) {
    if let Ok(key) = vault.data_key() {
        let _ = state.keystore.set(vault.id(), &key);
    }
}

pub fn vault_status(state: &AppState) -> Result<VaultStatus, CommandError> {
    let guard = state.vault.lock().map_err(|_| CommandError::internal())?;
    Ok(view(state, &guard))
}

/// Creates the vault (unlocked). Refused whenever a vault file exists, usable or not.
pub fn vault_create(
    state: &AppState,
    password: SecretString,
    remember: bool,
) -> Result<VaultStatus, CommandError> {
    ensure_writable(state)?;
    let dir = state.data_dir.as_ref().ok_or_else(|| {
        error(
            "no_data_dir",
            "no data folder is available on this system; set FERNJO_DATA_DIR",
        )
    })?;
    let path = dir.join(VAULT_FILE_NAME);
    let mut guard = state.vault.lock().map_err(|_| CommandError::internal())?;
    // Unreadable metadata counts as "exists": never risk overwriting.
    if path.try_exists().unwrap_or(true) {
        return Err(VaultError::AlreadyExists.into());
    }
    let mut vault = Vault::create_with(&path, &password, state.kdf)?;
    vault.attach_generation_store(std::sync::Arc::clone(&state.generations));
    if remember {
        remember_key(state, &vault);
    }
    *guard = VaultState::Open(vault);
    clear_auto_unlock_issue(state);
    Ok(view(state, &guard))
}

pub fn vault_unlock(
    state: &AppState,
    password: SecretString,
    remember: bool,
) -> Result<VaultStatus, CommandError> {
    with_vault(state, |v| {
        v.unlock(&password)?;
        if remember {
            remember_key(state, v);
        }
        Ok(())
    })?;
    clear_auto_unlock_issue(state);
    vault_status(state)
}

/// Like [`vault_unlock`], but takes a vault file that is older than the version this device last
/// opened (the user saw `rolled_back` and decided to open it). The file is re-stamped at once.
pub fn vault_unlock_older(
    state: &AppState,
    password: SecretString,
    remember: bool,
) -> Result<VaultStatus, CommandError> {
    // Re-stamping the file is a write, so a pending data-folder restart refuses it like the other
    // writers.
    ensure_writable(state)?;
    with_vault(state, |v| {
        v.unlock_accepting_older(&password)?;
        if remember {
            remember_key(state, v);
        }
        Ok(())
    })?;
    clear_auto_unlock_issue(state);
    vault_status(state)
}

/// Forgets the key in memory. Never touches the keychain item.
pub fn vault_lock(state: &AppState) -> Result<VaultStatus, CommandError> {
    with_vault(state, |v| {
        v.lock();
        Ok(())
    })?;
    vault_status(state)
}

pub fn vault_change_password(
    state: &AppState,
    old: SecretString,
    new: SecretString,
) -> Result<VaultStatus, CommandError> {
    ensure_writable(state)?;
    with_vault(state, |v| Ok(v.change_password(&old, &new)?))?;
    vault_status(state)
}

/// Stores or removes this vault's key in the key store. Needs an unlocked vault; a key
/// store failure is reported (the user asked for it explicitly).
pub fn vault_set_auto_unlock(state: &AppState, enabled: bool) -> Result<VaultStatus, CommandError> {
    ensure_writable(state)?;
    with_vault(state, |v| {
        let key = v.data_key()?;
        let outcome = if enabled {
            state.keystore.set(v.id(), &key)
        } else {
            state.keystore.delete(v.id())
        };
        outcome.map_err(|e| error("keychain_unavailable", e.message()))
    })?;
    vault_status(state)
}

/// A credential as the UI sees it: no secret field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialInfo {
    pub id: CredentialId,
    pub name: String,
    /// `password` or `privateKey`.
    pub kind: &'static str,
    pub username: Option<String>,
    pub domain: Option<String>,
    pub notes: String,
}

impl From<EntrySummary> for CredentialInfo {
    fn from(e: EntrySummary) -> Self {
        CredentialInfo {
            id: e.id,
            name: e.name,
            kind: match e.kind {
                SecretKind::Password => "password",
                SecretKind::PrivateKey => "privateKey",
            },
            username: e.username,
            domain: e.domain,
            notes: e.notes,
        }
    }
}

fn secret_opt<'de, D: Deserializer<'de>>(d: D) -> Result<Option<SecretString>, D::Error> {
    Ok(Option::<String>::deserialize(d)?.map(SecretString::from))
}

/// Input for a new credential. Secret fields are `SecretString`s (redacted in `Debug`) and
/// the type is never serialised.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialDraft {
    pub name: String,
    pub kind: SecretKind,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub notes: String,
    #[serde(default, deserialize_with = "secret_opt")]
    pub password: Option<SecretString>,
    #[serde(default, deserialize_with = "secret_opt")]
    pub private_key: Option<SecretString>,
    #[serde(default, deserialize_with = "secret_opt")]
    pub passphrase: Option<SecretString>,
}

/// Partial update. A secret field that is present replaces that part of the secret; the
/// kind cannot change. A new `private_key` keeps the stored passphrase unless `passphrase`
/// replaces it or `clear_passphrase` removes it. Empty `username`/`domain` clear the
/// field. Never serialised.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CredentialUpdate {
    pub name: Option<String>,
    pub username: Option<String>,
    pub domain: Option<String>,
    pub notes: Option<String>,
    #[serde(deserialize_with = "secret_opt")]
    pub password: Option<SecretString>,
    #[serde(deserialize_with = "secret_opt")]
    pub private_key: Option<SecretString>,
    #[serde(deserialize_with = "secret_opt")]
    pub passphrase: Option<SecretString>,
    /// Remove the stored passphrase of a private key credential.
    pub clear_passphrase: bool,
}

fn is_blank(s: &SecretString) -> bool {
    s.expose_secret().is_empty()
}

/// An empty passphrase means "none".
fn passphrase(p: Option<SecretString>) -> Option<SecretString> {
    p.filter(|p| !is_blank(p))
}

fn draft_secret(d: &mut CredentialDraft) -> Result<Secret, CommandError> {
    match d.kind {
        SecretKind::Password => match (d.password.take(), &d.private_key, &d.passphrase) {
            (Some(password), None, None) if !is_blank(&password) => {
                Ok(Secret::Password { password })
            }
            _ => Err(invalid(
                "a password credential needs a password and nothing else",
            )),
        },
        SecretKind::PrivateKey => match (d.private_key.take(), &d.password) {
            (Some(key), None) if !is_blank(&key) => Ok(Secret::PrivateKey {
                key,
                passphrase: passphrase(d.passphrase.take()),
            }),
            _ => Err(invalid(
                "a private key credential needs a private key (and optionally a passphrase)",
            )),
        },
    }
}

/// The replacement secret of an update, or `None` when no secret field is present.
///
/// A new `privateKey` without `passphrase` keeps the stored passphrase; `passphrase`
/// sets or replaces it (empty means none) and `clearPassphrase` removes it. A passphrase
/// alone (or `clearPassphrase` alone) keeps the stored key.
fn update_secret(
    vault: &Vault,
    id: CredentialId,
    u: &mut CredentialUpdate,
) -> Result<Option<Secret>, CommandError> {
    let kind = vault.get(id)?.kind;
    if let Some(password) = u.password.take() {
        if kind != SecretKind::Password
            || u.private_key.is_some()
            || u.passphrase.is_some()
            || u.clear_passphrase
            || is_blank(&password)
        {
            return Err(invalid(
                "a password can only replace the password of a password credential",
            ));
        }
        return Ok(Some(Secret::Password { password }));
    }
    if u.private_key.is_none() && u.passphrase.is_none() && !u.clear_passphrase {
        return Ok(None);
    }
    if kind != SecretKind::PrivateKey {
        return Err(invalid(
            "a key or passphrase can only replace the secret of a private key credential",
        ));
    }
    if u.passphrase.is_some() && u.clear_passphrase {
        return Err(invalid(
            "a passphrase cannot be set and removed in the same update",
        ));
    }
    let Secret::PrivateKey {
        key: stored_key,
        passphrase: stored_passphrase,
    } = vault.reveal(id)?
    else {
        return Err(invalid("not a private key credential"));
    };
    let key = match u.private_key.take() {
        Some(key) if is_blank(&key) => {
            return Err(invalid("the private key must not be empty"));
        }
        Some(key) => key,
        None => stored_key,
    };
    let new_passphrase = if u.clear_passphrase {
        None
    } else if u.passphrase.is_some() {
        passphrase(u.passphrase.take())
    } else {
        stored_passphrase
    };
    Ok(Some(Secret::PrivateKey {
        key,
        passphrase: new_passphrase,
    }))
}

pub fn list_credentials(state: &AppState) -> Result<Vec<CredentialInfo>, CommandError> {
    with_vault(state, |v| {
        Ok(v.list()?.into_iter().map(CredentialInfo::from).collect())
    })
}

pub fn get_credential(state: &AppState, id: CredentialId) -> Result<CredentialInfo, CommandError> {
    with_vault(state, |v| Ok(v.get(id)?.into()))
}

pub fn create_credential(
    state: &AppState,
    mut draft: CredentialDraft,
) -> Result<CredentialInfo, CommandError> {
    ensure_writable(state)?;
    with_vault(state, |v| {
        let secret = draft_secret(&mut draft)?;
        let id = v.add(NewEntry {
            name: draft.name,
            username: draft.username,
            domain: draft.domain,
            secret,
            notes: draft.notes,
        })?;
        Ok(v.get(id)?.into())
    })
}

pub fn update_credential(
    state: &AppState,
    id: CredentialId,
    mut update: CredentialUpdate,
) -> Result<CredentialInfo, CommandError> {
    ensure_writable(state)?;
    with_vault(state, |v| {
        let secret = update_secret(v, id, &mut update)?;
        v.update(
            id,
            EntryUpdate {
                name: update.name,
                username: update.username,
                domain: update.domain,
                secret,
                notes: update.notes,
            },
        )?;
        Ok(v.get(id)?.into())
    })
}

/// A node that references a credential.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UsageItem {
    pub id: fernjo_core::NodeId,
    pub name: String,
    /// `folder` or `connection`.
    pub kind: &'static str,
}

/// The nodes that reference the credential locally (inheriting nodes are not listed).
pub fn credential_usage(
    state: &AppState,
    id: CredentialId,
) -> Result<Vec<UsageItem>, CommandError> {
    with_vault(state, |v| {
        v.get(id)?;
        with_store(state, |store| {
            let tree = store.tree();
            Ok(tree
                .nodes_using_credential(id)
                .into_iter()
                .map(|(node_id, name)| UsageItem {
                    id: node_id,
                    name,
                    kind: match tree.get(node_id).map(|n| &n.kind) {
                        Some(NodeKind::Folder { .. }) => "folder",
                        _ => "connection",
                    },
                })
                .collect())
        })
    })
}

/// Deletes a credential: the vault must be unlocked, then the references in the tree are
/// cleared and saved, then the entry is deleted. If the last step fails (not expected),
/// the references are already cleared and the error is returned.
pub fn delete_credential(state: &AppState, id: CredentialId) -> Result<Mutation, CommandError> {
    ensure_writable(state)?;
    with_vault(state, |v| {
        // Locked vault or unknown entry: nothing has been changed yet.
        v.get(id)?;
        with_store(state, |store| {
            store.clear_credential(id)?;
            Ok(())
        })?;
        v.delete(id)?;
        with_store(state, |store| {
            Ok(Mutation {
                id: None,
                snapshot: snapshot(store, Vec::new()),
            })
        })
    })
}

/// The ONLY response type of any command that carries secret values (`reveal_credential`).
/// Its `Debug` output hides the values.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevealedSecret {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub private_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passphrase: Option<String>,
}

impl fmt::Debug for RevealedSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RevealedSecret(<redacted>)")
    }
}

pub fn reveal_credential(
    state: &AppState,
    id: CredentialId,
) -> Result<RevealedSecret, CommandError> {
    with_vault(state, |v| {
        Ok(match v.reveal(id)? {
            Secret::Password { password } => RevealedSecret {
                password: Some(password.expose_secret().to_owned()),
                private_key: None,
                passphrase: None,
            },
            Secret::PrivateKey { key, passphrase } => RevealedSecret {
                password: None,
                private_key: Some(key.expose_secret().to_owned()),
                passphrase: passphrase.map(|p| p.expose_secret().to_owned()),
            },
        })
    })
}

/// The `#[tauri::command]` wrappers registered in `run()`. Secrets arrive as `String` and
/// become `SecretString` immediately. The commands that run an Argon2 derivation (create,
/// unlock, change password; about half a second) use `#[tauri::command(async)]` so they run
/// on a worker thread instead of blocking the main thread.
pub mod handlers {
    use super::*;
    use tauri::State;

    #[tauri::command]
    pub fn vault_status(state: State<'_, AppState>) -> Result<VaultStatus, CommandError> {
        super::vault_status(&state)
    }

    #[tauri::command(async)]
    pub fn vault_create(
        state: State<'_, AppState>,
        password: String,
        remember: bool,
    ) -> Result<VaultStatus, CommandError> {
        super::vault_create(&state, SecretString::from(password), remember)
    }

    #[tauri::command(async)]
    pub fn vault_unlock(
        state: State<'_, AppState>,
        password: String,
        remember: bool,
    ) -> Result<VaultStatus, CommandError> {
        super::vault_unlock(&state, SecretString::from(password), remember)
    }

    #[tauri::command(async)]
    pub fn vault_unlock_older(
        state: State<'_, AppState>,
        password: String,
        remember: bool,
    ) -> Result<VaultStatus, CommandError> {
        super::vault_unlock_older(&state, SecretString::from(password), remember)
    }

    #[tauri::command]
    pub fn vault_lock(state: State<'_, AppState>) -> Result<VaultStatus, CommandError> {
        super::vault_lock(&state)
    }

    #[tauri::command(async)]
    pub fn vault_change_password(
        state: State<'_, AppState>,
        old: String,
        new: String,
    ) -> Result<VaultStatus, CommandError> {
        super::vault_change_password(&state, SecretString::from(old), SecretString::from(new))
    }

    #[tauri::command]
    pub fn vault_set_auto_unlock(
        state: State<'_, AppState>,
        enabled: bool,
    ) -> Result<VaultStatus, CommandError> {
        super::vault_set_auto_unlock(&state, enabled)
    }

    #[tauri::command]
    pub fn list_credentials(
        state: State<'_, AppState>,
    ) -> Result<Vec<CredentialInfo>, CommandError> {
        super::list_credentials(&state)
    }

    #[tauri::command]
    pub fn get_credential(
        state: State<'_, AppState>,
        id: CredentialId,
    ) -> Result<CredentialInfo, CommandError> {
        super::get_credential(&state, id)
    }

    #[tauri::command]
    pub fn create_credential(
        state: State<'_, AppState>,
        draft: CredentialDraft,
    ) -> Result<CredentialInfo, CommandError> {
        super::create_credential(&state, draft)
    }

    #[tauri::command]
    pub fn update_credential(
        state: State<'_, AppState>,
        id: CredentialId,
        update: CredentialUpdate,
    ) -> Result<CredentialInfo, CommandError> {
        super::update_credential(&state, id, update)
    }

    #[tauri::command]
    pub fn credential_usage(
        state: State<'_, AppState>,
        id: CredentialId,
    ) -> Result<Vec<UsageItem>, CommandError> {
        super::credential_usage(&state, id)
    }

    #[tauri::command]
    pub fn delete_credential(
        state: State<'_, AppState>,
        id: CredentialId,
    ) -> Result<Mutation, CommandError> {
        super::delete_credential(&state, id)
    }

    #[tauri::command]
    pub fn reveal_credential(
        state: State<'_, AppState>,
        id: CredentialId,
    ) -> Result<RevealedSecret, CommandError> {
        super::reveal_credential(&state, id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{self, CommandError};
    use crate::import_commands::{
        ApplyOptions, ImportKind, export_backup_to, export_connections_to, import_apply,
        import_begin, import_cancel, import_inspect,
    };
    use fernjo_core::{NodeId, Protocol};
    use fernjo_vault::{
        AutoUnlock, DataKey, GenerationStore, KeyStore, KeyStoreError, Vault, VaultId,
    };
    use secrecy::SecretString;
    use serde::Serialize;
    use serde_json::{Value, json};
    use std::fs;
    use std::path::Path;

    const MASTER: &str = "SENTINEL-master-1a2b3c";
    const NEW_MASTER: &str = "SENTINEL-newmaster-5e6f7a";
    const PW: &str = "SENTINEL-password-7f3a9b";
    const KEY: &str = "SENTINEL-privatekey-9c1b2d";
    const PASS: &str = "SENTINEL-passphrase-4d2e8f";
    const SENTINELS: [&str; 5] = [MASTER, NEW_MASTER, PW, KEY, PASS];
    // Import and export: the passwords inside the mRemoteNG fixtures (stored encrypted in the
    // files), the custom file key and the backup password.
    const DEFAULT_KEY_XML: &[u8] =
        include_bytes!("../../crates/fernjo-import/tests/fixtures/default-key.xml");
    const CUSTOM_KEY_XML: &[u8] =
        include_bytes!("../../crates/fernjo-import/tests/fixtures/custom-key.xml");
    const CUSTOM_KEY: &str = "fixture-key-ünï";
    const EXPORT_PW: &str = "SENTINEL-exportpw-3c9d1e";
    // Wrongly typed keys and passwords are secrets too: an error that echoes one is a leak.
    const WRONG_KEY: &str = "SENTINEL-wrongkey-8a1f";
    const WRONG_PW: &str = "SENTINEL-wrongpw-2b7c";
    // The other passwords of the fixtures: the one with XML and shell metacharacters, whole and
    // as the part that no escaping changes (JSON and `Debug` escape the quote and the
    // backslash), and the Unicode one the same way. `vnc-pw` sits on a VNC node, which is
    // skipped, so it is never imported; the three others are, and the flow test proves it.
    const META_PW: &str = "p@ss<&>\"'\\";
    const META_PW_PART: &str = "p@ss<&>";
    const UNICODE_PW: &str = "Gr\u{fc}\u{df}e, \u{4e16}\u{754c} \u{2013} \u{1f510}";
    const UNICODE_PW_PART: &str = "Gr\u{fc}\u{df}e, \u{4e16}\u{754c}";
    const IMPORT_SECRETS: [&str; 10] = [
        "folder-pw-1",
        "vnc-pw",
        META_PW,
        META_PW_PART,
        UNICODE_PW,
        UNICODE_PW_PART,
        CUSTOM_KEY,
        EXPORT_PW,
        WRONG_KEY,
        WRONG_PW,
    ];

    fn sec(s: &str) -> SecretString {
        SecretString::from(s)
    }

    fn state(dir: &Path) -> AppState {
        AppState::open(Some(dir.to_path_buf()))
    }

    fn code<T>(r: Result<T, CommandError>) -> String {
        match r {
            Ok(_) => "ok".to_string(),
            Err(e) => e.code,
        }
    }

    fn draft(v: Value) -> CredentialDraft {
        serde_json::from_value(v).unwrap()
    }

    fn update(v: Value) -> CredentialUpdate {
        serde_json::from_value(v).unwrap()
    }

    fn pw_draft(name: &str) -> Value {
        json!({"name": name, "kind": "password", "username": "root", "password": PW})
    }

    fn key_draft(name: &str) -> Value {
        json!({"name": name, "kind": "privateKey", "privateKey": KEY, "passphrase": PASS})
    }

    fn patch(v: Value) -> fernjo_core::NodePatch {
        serde_json::from_value(v).unwrap()
    }

    struct World {
        _dir: tempfile::TempDir,
        st: AppState,
        pw_id: NodeId,
        key_id: NodeId,
        folder: NodeId,
        conn: NodeId,
    }

    /// A vault with one password and one private-key entry, a folder using the first and a
    /// connection using the second.
    fn world(unlocked: bool) -> World {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path());
        vault_create(&st, sec(MASTER), false).unwrap();
        let pw_id = create_credential(&st, draft(pw_draft("pw"))).unwrap().id;
        let key_id = create_credential(&st, draft(key_draft("key"))).unwrap().id;
        let folder = commands::create_folder(&st, None, None, "f".into())
            .unwrap()
            .id
            .unwrap();
        let conn = commands::create_connection(
            &st,
            Some(folder),
            None,
            "c".into(),
            Protocol::Ssh,
            "h".into(),
        )
        .unwrap()
        .id
        .unwrap();
        commands::update_node(&st, folder, patch(json!({"credential": pw_id}))).unwrap();
        commands::update_node(&st, conn, patch(json!({"credential": key_id}))).unwrap();
        if !unlocked {
            vault_lock(&st).unwrap();
        }
        World {
            _dir: dir,
            st,
            pw_id,
            key_id,
            folder,
            conn,
        }
    }

    fn effective_credential(st: &AppState, id: NodeId) -> Value {
        let node = serde_json::to_value(commands::get_node(st, id).unwrap()).unwrap();
        node["credential"]["effective"].clone()
    }

    /// Key store whose every call fails (an unavailable keychain).
    struct FailingKeyStore;

    impl KeyStore for FailingKeyStore {
        fn get(&self, _: VaultId) -> Result<Option<DataKey>, KeyStoreError> {
            Err(KeyStoreError::new("keychain is down"))
        }
        fn set(&self, _: VaultId, _: &DataKey) -> Result<(), KeyStoreError> {
            Err(KeyStoreError::new("keychain is down"))
        }
        fn delete(&self, _: VaultId) -> Result<(), KeyStoreError> {
            Err(KeyStoreError::new("keychain is down"))
        }
    }

    // ---- lifecycle ----

    #[test]
    fn create_then_status_is_unlocked() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path());
        let before = vault_status(&st).unwrap();
        assert_eq!(before.state, "none");
        let after = vault_create(&st, sec(MASTER), false).unwrap();
        assert_eq!(after.state, "unlocked");
        assert!(!after.auto_unlock);
        assert!(after.path.unwrap().ends_with(fernjo_vault::FILE_NAME));
        assert!(dir.path().join(fernjo_vault::FILE_NAME).exists());
        let json = serde_json::to_value(vault_status(&st).unwrap()).unwrap();
        assert_eq!(json["state"], "unlocked");
        assert_eq!(json["autoUnlock"], false);
    }

    #[test]
    fn lock_then_wrong_password_then_right_password() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path());
        vault_create(&st, sec(MASTER), false).unwrap();
        assert_eq!(vault_lock(&st).unwrap().state, "locked");
        assert_eq!(
            code(vault_unlock(&st, sec("not the master"), false)),
            "wrong_password"
        );
        assert_eq!(vault_status(&st).unwrap().state, "locked");
        assert_eq!(
            vault_unlock(&st, sec(MASTER), false).unwrap().state,
            "unlocked"
        );
    }

    #[test]
    fn remember_stores_the_key_and_status_reports_auto_unlock() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path());
        assert!(vault_create(&st, sec(MASTER), true).unwrap().auto_unlock);
        // locking never touches the keychain item
        let locked = vault_lock(&st).unwrap();
        assert_eq!(locked.state, "locked");
        assert!(locked.auto_unlock);
        // unlocking without `remember` does not delete it either
        let again = vault_unlock(&st, sec(MASTER), false).unwrap();
        assert!(again.auto_unlock);
    }

    #[test]
    fn unlock_with_remember_stores_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path());
        vault_create(&st, sec(MASTER), false).unwrap();
        vault_lock(&st).unwrap();
        assert!(!vault_status(&st).unwrap().auto_unlock);
        assert!(vault_unlock(&st, sec(MASTER), true).unwrap().auto_unlock);
    }

    #[test]
    fn set_auto_unlock_false_deletes_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path());
        vault_create(&st, sec(MASTER), true).unwrap();
        let id = match &*st.vault.lock().unwrap() {
            VaultState::Open(v) => v.id(),
            _ => panic!("no vault"),
        };
        assert!(st.keystore.get(id).unwrap().is_some());
        assert!(!vault_set_auto_unlock(&st, false).unwrap().auto_unlock);
        assert!(st.keystore.get(id).unwrap().is_none());
        assert!(vault_set_auto_unlock(&st, true).unwrap().auto_unlock);
        assert!(st.keystore.get(id).unwrap().is_some());
        vault_lock(&st).unwrap();
        assert_eq!(code(vault_set_auto_unlock(&st, false)), "vault_locked");
    }

    /// A vault file with a wrong key in the key store at the next start.
    fn start_with_wrong_key(dir: &Path) -> (AppState, VaultId) {
        use fernjo_vault::MemoryKeyStore;
        let path = dir.join(fernjo_vault::FILE_NAME);
        let vault = Vault::create_with(&path, &sec(MASTER), fernjo_vault::KdfParams::TEST).unwrap();
        let keys = MemoryKeyStore::default();
        keys.set(vault.id(), &DataKey::from_bytes([7u8; 32]))
            .unwrap();
        let id = vault.id();
        let st = AppState::open_with_keystore(dir.to_path_buf(), Box::new(keys));
        (st, id)
    }

    #[test]
    fn a_failed_startup_auto_unlock_reports_the_issue_and_keeps_the_item() {
        let dir = tempfile::tempdir().unwrap();
        let (st, id) = start_with_wrong_key(dir.path());
        let status = vault_status(&st).unwrap();
        assert_eq!(status.state, "locked");
        assert_eq!(status.auto_unlock_issue, Some("key_not_accepted"));
        // the item stays, so the box is still ticked
        assert!(status.auto_unlock);
        assert!(st.keystore.get(id).unwrap().is_some());
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["autoUnlockIssue"], "key_not_accepted");
    }

    #[test]
    fn a_password_unlock_clears_the_issue_and_remember_replaces_a_wrong_item() {
        let dir = tempfile::tempdir().unwrap();
        let (st, id) = start_with_wrong_key(dir.path());
        let status = vault_unlock(&st, sec(MASTER), true).unwrap();
        assert_eq!(status.state, "unlocked");
        assert_eq!(status.auto_unlock_issue, None);
        // the wrong item was overwritten with the real key
        let stored = st.keystore.get(id).unwrap().unwrap();
        let real = match &*st.vault.lock().unwrap() {
            VaultState::Open(v) => v.data_key().unwrap(),
            _ => panic!("no vault"),
        };
        assert_eq!(stored.as_bytes(), real.as_bytes());
        // locking again does not bring the old issue back
        assert_eq!(vault_lock(&st).unwrap().auto_unlock_issue, None);
    }

    #[test]
    fn a_wrong_password_keeps_the_issue_and_unticked_unlock_keeps_the_item() {
        let dir = tempfile::tempdir().unwrap();
        let (st, id) = start_with_wrong_key(dir.path());
        assert_eq!(
            code(vault_unlock(&st, sec("wrong"), true)),
            "wrong_password"
        );
        assert_eq!(
            vault_status(&st).unwrap().auto_unlock_issue,
            Some("key_not_accepted")
        );
        let status = vault_unlock(&st, sec(MASTER), false).unwrap();
        assert_eq!(status.auto_unlock_issue, None);
        assert!(status.auto_unlock);
        assert!(st.keystore.get(id).unwrap().is_some());
    }

    /// A key store several `AppState`s can share (a "restart" that keeps the keychain).
    struct SharedKeys(std::sync::Arc<fernjo_vault::MemoryKeyStore>);

    impl KeyStore for SharedKeys {
        fn get(&self, id: VaultId) -> Result<Option<DataKey>, KeyStoreError> {
            self.0.get(id)
        }
        fn set(&self, id: VaultId, key: &DataKey) -> Result<(), KeyStoreError> {
            self.0.set(id, key)
        }
        fn delete(&self, id: VaultId) -> Result<(), KeyStoreError> {
            self.0.delete(id)
        }
    }

    /// A vault at generation 2 whose file was then replaced by the generation-1 copy; the
    /// returned state is a restart that keeps the keychain and the device marks.
    fn start_with_rolled_back_vault(
        dir: &Path,
    ) -> (
        AppState,
        std::sync::Arc<fernjo_vault::MemoryKeyStore>,
        std::sync::Arc<fernjo_vault::MemoryGenerationStore>,
    ) {
        use fernjo_vault::{MemoryGenerationStore, MemoryKeyStore};
        use std::sync::Arc;
        let keys = Arc::new(MemoryKeyStore::default());
        let marks = Arc::new(MemoryGenerationStore::default());
        let first = AppState::open_with_stores(
            dir.to_path_buf(),
            Box::new(SharedKeys(keys.clone())),
            marks.clone(),
        );
        vault_create(&first, sec(MASTER), true).unwrap(); // generation 1
        vault_change_password(&first, sec(MASTER), sec(NEW_MASTER)).unwrap(); // generation 2
        let path = dir.join(fernjo_vault::FILE_NAME);
        let mut bak = path.as_os_str().to_owned();
        bak.push(".bak");
        std::fs::copy(&bak, &path).unwrap();
        drop(first);
        let st = AppState::open_with_stores(
            dir.to_path_buf(),
            Box::new(SharedKeys(keys.clone())),
            marks.clone(),
        );
        (st, keys, marks)
    }

    #[test]
    fn a_rolled_back_vault_is_not_auto_unlocked_and_the_item_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let (st, keys, _marks) = start_with_rolled_back_vault(dir.path());
        let status = vault_status(&st).unwrap();
        assert_eq!(status.state, "locked");
        assert_eq!(status.auto_unlock_issue, Some("rolled_back"));
        assert!(status.auto_unlock, "the stored key is kept");
        let id = match &*st.vault.lock().unwrap() {
            VaultState::Open(v) => v.id(),
            _ => panic!("no vault"),
        };
        assert!(keys.get(id).unwrap().is_some());
        assert_eq!(
            serde_json::to_value(&status).unwrap()["autoUnlockIssue"],
            "rolled_back"
        );
    }

    #[test]
    fn a_password_unlock_of_a_rolled_back_vault_is_refused_until_the_user_accepts() {
        let dir = tempfile::tempdir().unwrap();
        let (st, keys, marks) = start_with_rolled_back_vault(dir.path());
        let id = match &*st.vault.lock().unwrap() {
            VaultState::Open(v) => v.id(),
            _ => panic!("no vault"),
        };
        // the generation-1 copy still has the first password
        let err = vault_unlock(&st, sec(MASTER), true).unwrap_err();
        assert_eq!(err.code, "rolled_back");
        assert!(err.message.contains("generation 1"), "{}", err.message);
        assert!(err.message.contains("last seen 2"), "{}", err.message);
        assert_eq!(vault_status(&st).unwrap().state, "locked");
        // a wrong password is a wrong password, not a rollback
        assert_eq!(
            code(vault_unlock_older(&st, sec("wrong password"), false)),
            "wrong_password"
        );
        assert_eq!(vault_status(&st).unwrap().state, "locked");
        // no key stored by the refused attempts beyond the one that was already there
        keys.delete(id).unwrap();

        let status = vault_unlock_older(&st, sec(MASTER), true).unwrap();
        assert_eq!(status.state, "unlocked");
        assert_eq!(status.auto_unlock_issue, None);
        assert!(status.auto_unlock, "remember stored the key again");
        assert_eq!(
            GenerationStore::get(&*marks, id),
            Some(3),
            "re-stamped above the floor of 2"
        );
        // and the next start auto-unlocks without a prompt
        drop(st);
        let again = AppState::open_with_stores(
            dir.path().to_path_buf(),
            Box::new(SharedKeys(keys.clone())),
            marks.clone(),
        );
        assert_eq!(vault_status(&again).unwrap().state, "unlocked");
    }

    #[test]
    fn vault_unlock_older_without_a_vault_or_with_nothing_to_accept() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path());
        assert_eq!(
            code(vault_unlock_older(&st, sec(MASTER), false)),
            "no_vault"
        );
        vault_create(&st, sec(MASTER), false).unwrap();
        vault_lock(&st).unwrap();
        // a current file: a plain unlock
        let status = vault_unlock_older(&st, sec(MASTER), false).unwrap();
        assert_eq!(status.state, "unlocked");
    }

    /// A key store whose reads fail (no Secret Service, locked keyring).
    struct BrokenKeyStore;

    impl KeyStore for BrokenKeyStore {
        fn get(&self, _: VaultId) -> Result<Option<DataKey>, KeyStoreError> {
            Err(KeyStoreError::new("keychain down"))
        }
        fn set(&self, _: VaultId, _: &DataKey) -> Result<(), KeyStoreError> {
            Err(KeyStoreError::new("keychain down"))
        }
        fn delete(&self, _: VaultId) -> Result<(), KeyStoreError> {
            Err(KeyStoreError::new("keychain down"))
        }
    }

    #[test]
    fn an_unreadable_keychain_at_startup_reports_its_own_issue() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(fernjo_vault::FILE_NAME);
        Vault::create_with(&path, &sec(MASTER), fernjo_vault::KdfParams::TEST).unwrap();
        let st = AppState::open_with_keystore(dir.path().to_path_buf(), Box::new(BrokenKeyStore));
        let status = vault_status(&st).unwrap();
        assert_eq!(status.state, "locked");
        assert_eq!(status.auto_unlock_issue, Some("keychain_unavailable"));
        // an unreadable keychain reads as "no stored key"
        assert!(!status.auto_unlock);
        let unlocked = vault_unlock(&st, sec(MASTER), false).unwrap();
        assert_eq!(unlocked.auto_unlock_issue, None);
    }

    #[test]
    fn startup_with_no_key_or_a_working_key_leaves_no_issue() {
        use fernjo_vault::MemoryKeyStore;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(fernjo_vault::FILE_NAME);
        let vault = Vault::create_with(&path, &sec(MASTER), fernjo_vault::KdfParams::TEST).unwrap();
        let none = AppState::open_with_keystore(
            dir.path().to_path_buf(),
            Box::new(MemoryKeyStore::default()),
        );
        assert_eq!(vault_status(&none).unwrap().auto_unlock_issue, None);
        let keys = MemoryKeyStore::default();
        keys.set(vault.id(), &vault.data_key().unwrap()).unwrap();
        let ok = AppState::open_with_keystore(dir.path().to_path_buf(), Box::new(keys));
        let status = vault_status(&ok).unwrap();
        assert_eq!(status.state, "unlocked");
        assert_eq!(status.auto_unlock_issue, None);
    }

    #[test]
    fn no_issue_without_a_failed_auto_unlock() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path());
        assert_eq!(vault_status(&st).unwrap().auto_unlock_issue, None);
        vault_create(&st, sec(MASTER), true).unwrap();
        vault_lock(&st).unwrap();
        assert_eq!(vault_status(&st).unwrap().auto_unlock_issue, None);
    }

    #[test]
    fn change_password_keeps_auto_unlock_working() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path());
        vault_create(&st, sec(MASTER), true).unwrap();
        let status = vault_change_password(&st, sec(MASTER), sec(NEW_MASTER)).unwrap();
        assert!(status.auto_unlock);
        assert_eq!(
            code(vault_change_password(&st, sec("wrong"), sec(NEW_MASTER))),
            "wrong_password"
        );
        // the keychain key still opens the changed file
        let mut reopened = Vault::open(&dir.path().join(fernjo_vault::FILE_NAME))
            .unwrap()
            .unwrap();
        assert_eq!(
            fernjo_vault::try_auto_unlock(&mut reopened, st.keystore.as_ref()),
            AutoUnlock::Unlocked
        );
        vault_lock(&st).unwrap();
        assert_eq!(
            code(vault_unlock(&st, sec(MASTER), false)),
            "wrong_password"
        );
        assert!(vault_unlock(&st, sec(NEW_MASTER), false).is_ok());
    }

    #[test]
    fn create_is_refused_when_a_vault_file_exists_even_if_unusable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(fernjo_vault::FILE_NAME);
        fs::write(&path, b"").unwrap();
        let st = state(dir.path());
        assert_eq!(vault_status(&st).unwrap().state, "unusable");
        assert!(vault_status(&st).unwrap().reason.is_some());
        assert_eq!(
            code(vault_create(&st, sec(MASTER), false)),
            "already_exists"
        );
        assert_eq!(fs::read(&path).unwrap(), b"");
        // a usable vault is not overwritten either
        let dir2 = tempfile::tempdir().unwrap();
        let st2 = state(dir2.path());
        vault_create(&st2, sec(MASTER), false).unwrap();
        assert_eq!(
            code(vault_create(&st2, sec(MASTER), false)),
            "already_exists"
        );
        // and not when the file appeared after startup
        let dir3 = tempfile::tempdir().unwrap();
        let st3 = state(dir3.path());
        fs::write(dir3.path().join(fernjo_vault::FILE_NAME), b"x").unwrap();
        assert_eq!(
            code(vault_create(&st3, sec(MASTER), false)),
            "already_exists"
        );
    }

    #[test]
    fn an_oversized_vault_file_is_unusable_and_stays_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(fernjo_vault::FILE_NAME);
        let len = fernjo_vault::MAX_FILE_LEN + 1;
        let file = fs::File::create(&path).unwrap();
        file.set_len(len).unwrap();
        drop(file);
        // the app starts normally, the vault is unusable and says why
        let st = state(dir.path());
        let status = vault_status(&st).unwrap();
        assert_eq!(status.state, "unusable");
        assert!(status.reason.unwrap().contains("larger than"));
        assert_eq!(
            code(vault_unlock(&st, sec(MASTER), false)),
            "vault_unusable"
        );
        assert_eq!(
            code(vault_create(&st, sec(MASTER), false)),
            "already_exists"
        );
        // nothing was overwritten, truncated or deleted, and no .bak appeared
        assert_eq!(fs::metadata(&path).unwrap().len(), len);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn short_password_returns_weak_password() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path());
        assert_eq!(
            code(vault_create(&st, sec("short"), false)),
            "weak_password"
        );
        assert!(!dir.path().join(fernjo_vault::FILE_NAME).exists());
        assert_eq!(vault_status(&st).unwrap().state, "none");
    }

    #[test]
    fn create_without_a_data_dir_is_a_clear_error() {
        let st = AppState::open(None);
        assert_eq!(code(vault_create(&st, sec(MASTER), false)), "no_data_dir");
    }

    #[test]
    fn commands_on_a_missing_or_unusable_vault_name_the_problem() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path());
        assert_eq!(code(vault_unlock(&st, sec(MASTER), false)), "no_vault");
        assert_eq!(code(list_credentials(&st)), "no_vault");
        fs::write(dir.path().join(fernjo_vault::FILE_NAME), b"").unwrap();
        let st = state(dir.path());
        assert_eq!(
            code(vault_unlock(&st, sec(MASTER), false)),
            "vault_unusable"
        );
        assert_eq!(code(list_credentials(&st)), "vault_unusable");
    }

    #[test]
    fn an_unavailable_keychain_never_fails_unlock_or_create() {
        let dir = tempfile::tempdir().unwrap();
        let st = AppState::open_with_keystore(dir.path().to_path_buf(), Box::new(FailingKeyStore));
        let created = vault_create(&st, sec(MASTER), true).unwrap();
        assert_eq!(created.state, "unlocked");
        assert!(!created.auto_unlock);
        vault_lock(&st).unwrap();
        let unlocked = vault_unlock(&st, sec(MASTER), true).unwrap();
        assert_eq!(unlocked.state, "unlocked");
        assert!(!unlocked.auto_unlock);
        // the explicit request does report the keychain error
        let err = vault_set_auto_unlock(&st, true).unwrap_err();
        assert_eq!(err.code, "keychain_unavailable");
        assert!(err.message.contains("keychain is down"), "{}", err.message);
        let err = vault_set_auto_unlock(&st, false).unwrap_err();
        assert_eq!(err.code, "keychain_unavailable");
    }

    #[test]
    fn mutating_commands_return_restart_required_when_flagged() {
        let w = world(true);
        w.st.set_restart_required(true);
        let id = w.pw_id;
        let results = [
            code(vault_create(&w.st, sec(MASTER), false)),
            code(vault_change_password(&w.st, sec(MASTER), sec(NEW_MASTER))),
            // re-stamps the vault file, so it is a writer like the others
            code(vault_unlock_older(&w.st, sec(MASTER), false)),
            code(vault_set_auto_unlock(&w.st, true)),
            code(create_credential(&w.st, draft(pw_draft("x")))),
            code(update_credential(&w.st, id, update(json!({"name": "y"})))),
            code(delete_credential(&w.st, id)),
        ];
        for r in results {
            assert_eq!(r, "restart_required");
        }
        // reads still work
        assert!(list_credentials(&w.st).is_ok());
        assert!(reveal_credential(&w.st, id).is_ok());
    }

    // ---- credentials ----

    #[test]
    fn create_list_get_update_round_trip() {
        let w = world(true);
        let info = create_credential(
            &w.st,
            draft(json!({
                "name": "web", "kind": "password", "username": "admin",
                "domain": "CORP", "notes": "n", "password": PW
            })),
        )
        .unwrap();
        assert_eq!(info.name, "web");
        assert_eq!(info.kind, "password");
        assert_eq!(info.username.as_deref(), Some("admin"));
        assert_eq!(info.domain.as_deref(), Some("CORP"));
        assert_eq!(info.notes, "n");
        let listed = list_credentials(&w.st).unwrap();
        assert_eq!(listed.len(), 3);
        assert_eq!(get_credential(&w.st, info.id).unwrap().name, "web");
        let key = listed.iter().find(|c| c.id == w.key_id).unwrap();
        assert_eq!(key.kind, "privateKey");

        let changed = update_credential(
            &w.st,
            info.id,
            update(json!({"name": "web2", "username": "", "notes": "m"})),
        )
        .unwrap();
        assert_eq!(changed.name, "web2");
        assert_eq!(changed.username, None);
        assert_eq!(changed.domain.as_deref(), Some("CORP"));
        assert_eq!(changed.notes, "m");
        // the secret is untouched without a secret field
        assert_eq!(
            reveal_credential(&w.st, info.id)
                .unwrap()
                .password
                .as_deref(),
            Some(PW)
        );
        // a secret field replaces the secret
        update_credential(&w.st, info.id, update(json!({"password": "new-secret-pw"}))).unwrap();
        assert_eq!(
            reveal_credential(&w.st, info.id)
                .unwrap()
                .password
                .as_deref(),
            Some("new-secret-pw")
        );
        // the kind cannot change through an update
        assert_eq!(
            code(update_credential(
                &w.st,
                info.id,
                update(json!({"privateKey": "k"}))
            )),
            "invalid_credential"
        );
        // passphrase alone keeps the stored key
        update_credential(&w.st, w.key_id, update(json!({"passphrase": "p2"}))).unwrap();
        let revealed = reveal_credential(&w.st, w.key_id).unwrap();
        assert_eq!(revealed.private_key.as_deref(), Some(KEY));
        assert_eq!(revealed.passphrase.as_deref(), Some("p2"));
        // a new key without a passphrase keeps the stored passphrase
        update_credential(&w.st, w.key_id, update(json!({"privateKey": "k2"}))).unwrap();
        let revealed = reveal_credential(&w.st, w.key_id).unwrap();
        assert_eq!(revealed.private_key.as_deref(), Some("k2"));
        assert_eq!(revealed.passphrase.as_deref(), Some("p2"));

        assert_eq!(code(get_credential(&w.st, NodeId::new_v4())), "not_found");
        assert_eq!(
            code(update_credential(
                &w.st,
                info.id,
                update(json!({"name": " "}))
            )),
            "empty_name"
        );
    }

    #[test]
    fn private_key_updates_keep_or_clear_the_passphrase_as_asked() {
        let w = world(true);
        let id = w.key_id;
        let secret = |st: &AppState| {
            let r = reveal_credential(st, id).unwrap();
            (r.private_key, r.passphrase)
        };
        assert_eq!(secret(&w.st), (Some(KEY.to_owned()), Some(PASS.to_owned())));
        // a new key without a passphrase keeps the stored passphrase
        update_credential(&w.st, id, update(json!({"privateKey": "key-two"}))).unwrap();
        assert_eq!(
            secret(&w.st),
            (Some("key-two".to_owned()), Some(PASS.to_owned()))
        );
        // clearPassphrase alone removes the passphrase and keeps the key
        update_credential(&w.st, id, update(json!({"clearPassphrase": true}))).unwrap();
        assert_eq!(secret(&w.st), (Some("key-two".to_owned()), None));
        // a new key plus a new passphrase replaces both
        update_credential(
            &w.st,
            id,
            update(json!({"privateKey": "key-three", "passphrase": "pass-three"})),
        )
        .unwrap();
        assert_eq!(
            secret(&w.st),
            (Some("key-three".to_owned()), Some("pass-three".to_owned()))
        );
        // a new key together with clearPassphrase drops the old passphrase
        update_credential(
            &w.st,
            id,
            update(json!({"privateKey": "key-four", "clearPassphrase": true})),
        )
        .unwrap();
        assert_eq!(secret(&w.st), (Some("key-four".to_owned()), None));
        // clearPassphrase false is a no-op
        update_credential(&w.st, id, update(json!({"passphrase": "p5"}))).unwrap();
        update_credential(&w.st, id, update(json!({"clearPassphrase": false}))).unwrap();
        assert_eq!(
            secret(&w.st),
            (Some("key-four".to_owned()), Some("p5".to_owned()))
        );
        // a typed passphrase and clearPassphrase together are contradictory
        assert_eq!(
            code(update_credential(
                &w.st,
                id,
                update(json!({"passphrase": "x", "clearPassphrase": true}))
            )),
            "invalid_credential"
        );
        // clearPassphrase does not apply to a password credential
        assert_eq!(
            code(update_credential(
                &w.st,
                w.pw_id,
                update(json!({"clearPassphrase": true}))
            )),
            "invalid_credential"
        );
        assert_eq!(secret(&w.st).1.as_deref(), Some("p5"));
    }

    #[test]
    fn drafts_must_carry_a_kind_appropriate_secret() {
        let w = world(true);
        let bad = [
            json!({"name": "a", "kind": "password"}),
            json!({"name": "a", "kind": "password", "password": ""}),
            json!({"name": "a", "kind": "password", "password": "x", "privateKey": "k"}),
            json!({"name": "a", "kind": "password", "password": "x", "passphrase": "p"}),
            json!({"name": "a", "kind": "privateKey"}),
            json!({"name": "a", "kind": "privateKey", "password": "x"}),
            json!({"name": "a", "kind": "privateKey", "privateKey": "k", "password": "x"}),
        ];
        for v in bad {
            assert_eq!(
                code(create_credential(&w.st, draft(v.clone()))),
                "invalid_credential",
                "{v}"
            );
        }
        let ok = create_credential(
            &w.st,
            draft(json!({"name": "a", "kind": "privateKey", "privateKey": "k"})),
        )
        .unwrap();
        assert_eq!(reveal_credential(&w.st, ok.id).unwrap().passphrase, None);
        assert_eq!(
            code(create_credential(&w.st, draft(pw_draft("")))),
            "empty_name"
        );
    }

    #[test]
    fn reveal_returns_the_secret_only_via_reveal() {
        let w = world(true);
        let pw = reveal_credential(&w.st, w.pw_id).unwrap();
        assert_eq!(pw.password.as_deref(), Some(PW));
        assert!(pw.private_key.is_none() && pw.passphrase.is_none());
        let json = serde_json::to_value(reveal_credential(&w.st, w.key_id).unwrap()).unwrap();
        assert_eq!(json["privateKey"], KEY);
        assert_eq!(json["passphrase"], PASS);
        assert!(json.get("password").is_none());
        for info in [
            serde_json::to_string(&list_credentials(&w.st).unwrap()).unwrap(),
            serde_json::to_string(&get_credential(&w.st, w.pw_id).unwrap()).unwrap(),
        ] {
            assert!(!info.contains(PW) && !info.contains(KEY) && !info.contains(PASS));
        }
        assert_eq!(
            code(reveal_credential(&world(false).st, NodeId::new_v4())),
            "vault_locked"
        );
        assert_eq!(
            code(reveal_credential(&w.st, NodeId::new_v4())),
            "not_found"
        );
    }

    #[test]
    fn credential_usage_names_the_local_users() {
        let w = world(true);
        let users = credential_usage(&w.st, w.pw_id).unwrap();
        let json = serde_json::to_value(&users).unwrap();
        assert_eq!(
            json,
            json!([{"id": w.folder, "name": "f", "kind": "folder"}])
        );
        let json = serde_json::to_value(credential_usage(&w.st, w.key_id).unwrap()).unwrap();
        assert_eq!(
            json,
            json!([{"id": w.conn, "name": "c", "kind": "connection"}])
        );
        assert_eq!(
            code(credential_usage(&world(false).st, w.pw_id)),
            "vault_locked"
        );
    }

    #[test]
    fn delete_clears_references_then_removes_the_entry() {
        let w = world(true);
        assert_eq!(credential_usage(&w.st, w.pw_id).unwrap().len(), 1);
        assert_eq!(effective_credential(&w.st, w.folder), json!(w.pw_id));
        let m = delete_credential(&w.st, w.pw_id).unwrap();
        assert_eq!(m.id, None);
        assert!(
            !serde_json::to_string(&m.snapshot)
                .unwrap()
                .contains(&w.pw_id.to_string())
        );
        assert_eq!(effective_credential(&w.st, w.folder), Value::Null);
        assert_eq!(code(get_credential(&w.st, w.pw_id)), "not_found");
        assert_eq!(list_credentials(&w.st).unwrap().len(), 1);
        // the other reference is untouched
        assert_eq!(effective_credential(&w.st, w.conn), json!(w.key_id));
        // the saved tree has no reference either
        let text = fs::read_to_string(w._dir.path().join(fernjo_core::FILE_NAME)).unwrap();
        assert!(!text.contains(&w.pw_id.to_string()));
        assert_eq!(code(delete_credential(&w.st, w.pw_id)), "not_found");
    }

    #[test]
    fn delete_while_locked_returns_vault_locked_and_clears_nothing() {
        let w = world(false);
        assert_eq!(code(delete_credential(&w.st, w.pw_id)), "vault_locked");
        assert_eq!(effective_credential(&w.st, w.folder), json!(w.pw_id));
        let text = fs::read_to_string(w._dir.path().join(fernjo_core::FILE_NAME)).unwrap();
        assert!(text.contains(&w.pw_id.to_string()));
        vault_unlock(&w.st, sec(MASTER), false).unwrap();
        assert!(get_credential(&w.st, w.pw_id).is_ok());
    }

    #[test]
    fn update_node_with_credential_patch_sets_and_resets() {
        let w = world(true);
        let loose =
            commands::create_connection(&w.st, None, None, "l".into(), Protocol::Ssh, "h".into())
                .unwrap()
                .id
                .unwrap();
        assert_eq!(effective_credential(&w.st, loose), Value::Null);
        commands::update_node(&w.st, loose, patch(json!({"credential": w.key_id}))).unwrap();
        assert_eq!(effective_credential(&w.st, loose), json!(w.key_id));
        commands::update_node(&w.st, loose, patch(json!({"credential": null}))).unwrap();
        assert_eq!(effective_credential(&w.st, loose), Value::Null);
    }

    #[test]
    fn locked_vault_still_resolves_credential_ids() {
        let w = world(false);
        assert_eq!(effective_credential(&w.st, w.conn), json!(w.key_id));
        assert_eq!(effective_credential(&w.st, w.folder), json!(w.pw_id));
        assert_eq!(code(list_credentials(&w.st)), "vault_locked");
        assert_eq!(code(get_credential(&w.st, w.pw_id)), "vault_locked");
        assert_eq!(
            code(create_credential(&w.st, draft(pw_draft("x")))),
            "vault_locked"
        );
        assert_eq!(
            code(update_credential(
                &w.st,
                w.pw_id,
                update(json!({"name": "y"}))
            )),
            "vault_locked"
        );
    }

    // ---- no secrets over IPC ----

    fn j<T: Serialize>(r: Result<T, CommandError>) -> String {
        match r {
            Ok(v) => serde_json::to_string(&v).unwrap(),
            Err(e) => format!(
                "{}{:?}",
                serde_json::to_string(&e).unwrap(),
                e // Debug of the error as well
            ),
        }
    }

    /// Opens a session on the world's connection (pointed at a port that refuses at once, so
    /// nothing leaves the machine) and records the JSON events it emits.
    fn opened(w: &World) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use crate::session_commands::open_session;
        use tauri::ipc::{Channel, InvokeResponseBody};
        commands::update_node(
            &w.st,
            w.conn,
            patch(json!({"host": "127.0.0.1", "port": 1})),
        )
        .unwrap();
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = std::sync::Arc::clone(&events);
        let channel = Channel::new(move |body: InvokeResponseBody| {
            if let InvokeResponseBody::Json(text) = body {
                sink.lock().unwrap().push(text);
            }
            Ok(())
        });
        let id = open_session(&w.st, w.conn, channel, 80, 24).unwrap();
        (id, events)
    }

    fn dump(events: &std::sync::Mutex<Vec<String>>) -> String {
        events.lock().unwrap().join("|")
    }

    fn secret_answer() -> crate::sessions::PromptAnswer {
        serde_json::from_value(json!({"kind": "secrets", "values": [PW, KEY, PASS, MASTER]}))
            .unwrap()
    }

    type Call = Box<dyn Fn(&World) -> String>;

    /// Every command function, called with sentinel secrets in play. When a command is
    /// added to `commands.rs` or `vault_commands.rs`, add it here (the next test enforces it).
    fn table() -> Vec<(&'static str, Call)> {
        fn entry(
            name: &'static str,
            f: impl Fn(&World) -> String + 'static,
        ) -> (&'static str, Call) {
            (name, Box::new(f))
        }
        vec![
            entry("list_tree", |w| j(commands::list_tree(&w.st))),
            entry("get_node", |w| j(commands::get_node(&w.st, w.conn))),
            entry("create_folder", |w| {
                j(commands::create_folder(&w.st, None, None, "n".into()))
            }),
            entry("create_connection", |w| {
                j(commands::create_connection(
                    &w.st,
                    None,
                    None,
                    "n".into(),
                    Protocol::Ssh,
                    "h".into(),
                ))
            }),
            entry("update_node", |w| {
                j(commands::update_node(
                    &w.st,
                    w.conn,
                    patch(json!({"name": "renamed", "credential": w.pw_id})),
                ))
            }),
            entry("move_node", |w| {
                j(commands::move_node(&w.st, w.conn, None, 0))
            }),
            entry("duplicate_node", |w| {
                j(commands::duplicate_node(&w.st, w.conn))
            }),
            entry("delete_node", |w| j(commands::delete_node(&w.st, w.folder))),
            entry("vault_status", |w| j(vault_status(&w.st))),
            entry("vault_create", |w| {
                j(vault_create(&w.st, sec(MASTER), true))
            }),
            entry("vault_unlock", |w| {
                j(vault_unlock(&w.st, sec(MASTER), true))
            }),
            entry("vault_unlock_older", |w| {
                j(vault_unlock_older(&w.st, sec(MASTER), true))
            }),
            entry("vault_lock", |w| j(vault_lock(&w.st))),
            entry("vault_change_password", |w| {
                j(vault_change_password(&w.st, sec(MASTER), sec(NEW_MASTER)))
            }),
            entry("vault_set_auto_unlock", |w| {
                j(vault_set_auto_unlock(&w.st, true))
            }),
            entry("list_credentials", |w| j(list_credentials(&w.st))),
            entry("get_credential", |w| j(get_credential(&w.st, w.key_id))),
            entry("create_credential", |w| {
                j(create_credential(&w.st, draft(key_draft("again"))))
            }),
            entry("update_credential", |w| {
                j(update_credential(
                    &w.st,
                    w.key_id,
                    update(json!({"name": "k2", "privateKey": KEY, "passphrase": PASS})),
                ))
            }),
            entry("credential_usage", |w| j(credential_usage(&w.st, w.key_id))),
            entry("delete_credential", |w| {
                j(delete_credential(&w.st, w.key_id))
            }),
            entry("reveal_credential", |w| {
                j(reveal_credential(&w.st, w.key_id))
            }),
            entry("get_data_folder", |w| {
                j(crate::data_folder::get_data_folder(&w.st))
            }),
            entry("get_terminal_settings", |w| {
                j(crate::session_commands::get_terminal_settings(&w.st))
            }),
            entry("set_terminal_settings", |w| {
                j(crate::session_commands::set_terminal_settings(
                    &w.st,
                    crate::session_commands::TerminalSettingsInput {
                        font_family: Some("Fira Code".into()),
                        font_size: 14,
                        scrollback: 5000,
                    },
                ))
            }),
            entry("get_theme", |w| j(crate::app_settings::get_theme(&w.st))),
            entry("set_theme", |w| {
                j(crate::app_settings::set_theme(
                    &w.st,
                    fernjo_core::Theme::Dark,
                ))
            }),
            entry("open_session", |w| {
                let (id, events) = opened(w);
                format!("{id}{}", dump(&events))
            }),
            entry("session_input", |w| {
                let (id, _) = opened(w);
                j(crate::session_commands::session_input(
                    &w.st,
                    &id,
                    [PW, KEY].concat().into_bytes(),
                ))
            }),
            entry("session_resize", |w| {
                let (id, _) = opened(w);
                j(crate::session_commands::session_resize(&w.st, &id, 100, 30))
            }),
            entry("session_ack", |w| {
                let (id, _) = opened(w);
                j(crate::session_commands::session_ack(&w.st, &id, 10))
            }),
            entry("close_session", |w| {
                let (id, _) = opened(w);
                j(crate::session_commands::close_session(&w.st, &id))
            }),
            entry("reconnect_session", |w| {
                let (id, _) = opened(w);
                j(crate::session_commands::reconnect_session(
                    &w.st, &id, 80, 24,
                ))
            }),
            entry("answer_prompt", |w| {
                let (id, events) = opened(w);
                // an unknown prompt id on a real session, and an unknown session
                let known = j(crate::session_commands::answer_prompt(
                    &w.st,
                    &id,
                    999,
                    secret_answer(),
                ));
                let unknown = j(crate::session_commands::answer_prompt(
                    &w.st,
                    "no-such-session",
                    1,
                    secret_answer(),
                ));
                format!("{known}{unknown}{}", dump(&events))
            }),
            entry("resume_session", |w| {
                let (id, events) = opened(w);
                let r = j(crate::session_commands::resume_session(&w.st, &id, true));
                format!("{r}{}", dump(&events))
            }),
            entry("import_inspect", |w| {
                let xml = w._dir.path().join("table.xml");
                fs::write(&xml, CUSTOM_KEY_XML).unwrap();
                import_begin(&w.st, ImportKind::Mremoteng, xml);
                let none = j(import_inspect(&w.st, None));
                let wrong = j(import_inspect(&w.st, Some(WRONG_KEY.into())));
                let ok = j(import_inspect(&w.st, Some(CUSTOM_KEY.into())));
                format!("{none}{wrong}{ok}")
            }),
            entry("import_apply", |w| {
                let xml = w._dir.path().join("table.xml");
                fs::write(&xml, DEFAULT_KEY_XML).unwrap();
                import_begin(&w.st, ImportKind::Mremoteng, xml);
                let inspected = j(import_inspect(&w.st, None));
                let applied = j(import_apply(&w.st, ApplyOptions::default()));
                format!("{inspected}{applied}")
            }),
            entry("import_cancel", |w| {
                let picked = import_begin(&w.st, ImportKind::Backup, w._dir.path().join("x.fjbk"));
                assert!(w.st.pending.lock().unwrap().is_some());
                import_cancel(&w.st);
                assert!(w.st.pending.lock().unwrap().is_none());
                format!("{picked:?}")
            }),
            // not IPC commands (see `NOT_A_COMMAND`), but called as plain functions
            entry("import_begin", |w| {
                let p = import_begin(&w.st, ImportKind::Backup, w._dir.path().join("x.fjbk"));
                format!("{p:?}{}", serde_json::to_string(&p).unwrap())
            }),
            // exports go outside the data folder, which they may not write into
            entry("export_connections_to", |w| {
                let elsewhere = tempfile::tempdir().unwrap();
                let out = elsewhere.path().join("c.json");
                let r = j(export_connections_to(&w.st, &out));
                assert!(out.exists(), "{r}");
                r
            }),
            entry("export_backup_to", |w| {
                let elsewhere = tempfile::tempdir().unwrap();
                let out = elsewhere.path().join("b.fjbk");
                let r = j(export_backup_to(&w.st, &out, &sec(EXPORT_PW)));
                // locked vault: refused with a code; otherwise the file exists
                assert!(out.exists() || r.contains("vault_locked"), "{r}");
                r
            }),
            entry("apply_data_folder", |w| {
                // not an IPC command (see `NOT_A_COMMAND`), but called here as a plain
                // function: the state has no config folder, so this is refused; the response
                // and error must be free of secrets either way
                j(crate::data_folder::apply_data_folder(&w.st, w._dir.path()))
            }),
        ]
    }

    fn assert_no_sentinel(what: &str, text: &str) {
        for s in SENTINELS.iter().chain(&IMPORT_SECRETS) {
            assert!(!text.contains(s), "{what} leaked {s}: {text}");
        }
    }

    #[test]
    fn no_response_contains_a_secret() {
        for unlocked in [true, false] {
            for (name, call) in table() {
                let w = world(unlocked);
                let out = call(&w);
                let label = format!("{name} (unlocked: {unlocked})");
                if name == "reveal_credential" && unlocked {
                    // the one and only response that carries secrets
                    assert!(out.contains(KEY) && out.contains(PASS), "{out}");
                    assert!(!out.contains(PW) && !out.contains(MASTER), "{out}");
                } else {
                    assert_no_sentinel(&label, &out);
                }
                // the saved tree, the Debug output of state and status never hold a secret
                let tree = fs::read_to_string(w._dir.path().join(fernjo_core::FILE_NAME)).unwrap();
                assert_no_sentinel(&format!("connections.json after {label}"), &tree);
                assert_no_sentinel("AppState Debug", &format!("{:?}", w.st));
                assert_no_sentinel("status Debug", &format!("{:?}", vault_status(&w.st)));
                let listed = format!("{:?}", list_credentials(&w.st));
                assert_no_sentinel("list_credentials Debug", &listed);
            }
        }
    }

    /// The whole import and restore path with sentinel secrets in every input: an mRemoteNG file
    /// with a custom key, the key itself, wrongly typed keys and passwords, the export password
    /// and a backup holding the entries. No response, error, `Debug` output, exported file or
    /// `connections.json` shows one. Every step that must succeed is unwrapped and every step
    /// that must fail has its error code asserted, so the scan never runs over empty output.
    #[test]
    fn import_and_export_flow_leaks_no_secret() {
        fn shown<T: Serialize + std::fmt::Debug>(v: &T) -> String {
            format!("{}{:?}", serde_json::to_string(v).unwrap(), v)
        }
        let w = world(true);
        let dir = w._dir.path().to_path_buf();
        // exported files go outside the data folder (which the commands refuse)
        let out_dir = tempfile::tempdir().unwrap();
        let mut seen: Vec<(String, String)> = Vec::new();
        let mut note = |what: &str, text: String| seen.push((what.to_string(), text));
        let credentials = || list_credentials(&w.st).unwrap().len();
        let at_start = credentials();
        assert_eq!(at_start, 2);

        // mRemoteNG file with a custom key
        let custom = dir.join("custom.xml");
        fs::write(&custom, CUSTOM_KEY_XML).unwrap();
        note(
            "begin",
            format!("{:?}", import_begin(&w.st, ImportKind::Mremoteng, custom)),
        );
        let e = import_inspect(&w.st, None).unwrap_err();
        assert_eq!(e.code, "key_needed");
        note("inspect without key", j::<()>(Err(e)));
        let e = import_inspect(&w.st, Some(WRONG_KEY.into())).unwrap_err();
        assert_eq!(e.code, "wrong_key");
        note("inspect wrong key", j::<()>(Err(e)));
        let preview = import_inspect(&w.st, Some(CUSTOM_KEY.into())).unwrap();
        let planned_custom = preview.report.as_ref().unwrap().counts.credentials;
        assert!(planned_custom > 0);
        note("inspect", shown(&preview));
        note("pending Debug", format!("{:?}", w.st.pending));
        let applied = import_apply(&w.st, ApplyOptions::default()).unwrap();
        note("apply", shown(&applied));
        assert_eq!(credentials(), at_start + planned_custom);

        // mRemoteNG file with the default key
        let plain = dir.join("plain.xml");
        fs::write(&plain, DEFAULT_KEY_XML).unwrap();
        import_begin(&w.st, ImportKind::Mremoteng, plain);
        let preview = import_inspect(&w.st, None).unwrap();
        let planned_plain = preview.report.as_ref().unwrap().counts.credentials;
        assert!(planned_plain > 0);
        note("inspect default key", shown(&preview));
        let applied = import_apply(&w.st, ApplyOptions::default()).unwrap();
        note("apply default key", shown(&applied));
        let total = at_start + planned_custom + planned_plain;
        assert_eq!(credentials(), total);

        // exports; the backup holds the secrets (positive control) and the file hides them
        let backup = out_dir.path().join("out.fjbk");
        let info = export_backup_to(&w.st, &backup, &sec(EXPORT_PW)).unwrap();
        note("export backup", shown(&info));
        let bytes = fs::read(&backup).unwrap();
        let opened = fernjo_import::backup::open(&bytes, &sec(EXPORT_PW)).unwrap();
        assert_eq!(opened.entries.len(), total);
        let inside: String = opened
            .entries
            .iter()
            .map(|(_, e)| serde_json::to_string(&e.secret).unwrap())
            .collect();
        // positive control: the fixtures' passwords really were imported (the metacharacter
        // one is stored with JSON escapes, so its escape-free part is searched)
        for secret in [PW, KEY, PASS, "folder-pw-1", META_PW_PART, UNICODE_PW_PART] {
            assert!(inside.contains(secret), "{secret} is not in the backup");
        }
        let connections = out_dir.path().join("out.json");
        let info = export_connections_to(&w.st, &connections).unwrap();
        note("export connections", shown(&info));

        // restore
        import_begin(&w.st, ImportKind::Backup, backup.clone());
        let e = import_inspect(&w.st, None).unwrap_err();
        assert_eq!(e.code, "password_needed");
        note("backup without password", j::<()>(Err(e)));
        let e = import_inspect(&w.st, Some(WRONG_PW.into())).unwrap_err();
        assert_eq!(e.code, "wrong_password");
        note("backup wrong password", j::<()>(Err(e)));
        let preview = import_inspect(&w.st, Some(EXPORT_PW.into())).unwrap();
        assert_eq!(preview.backup.as_ref().unwrap().credentials, total);
        note("backup inspect", shown(&preview));
        note("pending backup Debug", format!("{:?}", w.st.pending));
        let restored = import_apply(&w.st, ApplyOptions::default()).unwrap();
        assert_eq!(restored.restored.as_ref().unwrap().credentials, total);
        note("restore", shown(&restored));
        assert_eq!(credentials(), total);
        note("AppState Debug", format!("{:?}", w.st));

        for (what, text) in &seen {
            assert_no_sentinel(what, text);
        }
        // the exported files and the saved tree hold none
        for file in [
            backup.as_path(),
            connections.as_path(),
            &dir.join(fernjo_core::FILE_NAME),
        ] {
            let text = String::from_utf8_lossy(&fs::read(file).unwrap()).into_owned();
            assert_no_sentinel(&file.display().to_string(), &text);
        }
    }

    #[test]
    fn drafts_and_updates_and_revealed_secrets_are_redacted_in_debug() {
        let d = draft(json!({
            "name": "a", "kind": "privateKey", "privateKey": KEY, "passphrase": PASS
        }));
        assert_no_sentinel("CredentialDraft Debug", &format!("{d:?}"));
        let d = draft(pw_draft("a"));
        assert_no_sentinel("CredentialDraft Debug", &format!("{d:?}"));
        let u = update(json!({"password": PW, "privateKey": KEY, "passphrase": PASS}));
        assert_no_sentinel("CredentialUpdate Debug", &format!("{u:?}"));
        let w = world(true);
        let r = reveal_credential(&w.st, w.key_id).unwrap();
        assert_no_sentinel("RevealedSecret Debug", &format!("{r:?}"));
    }

    /// Commands that need an `AppHandle` cannot be called without a running app, so they have
    /// no `table()` entry. Each is exempt for the stated reason: no secret type appears in
    /// its signature (`choose_data_folder` returns `DataFolderInfo`, `restart_app` nothing).
    const APP_HANDLE_EXEMPT: [(&str, &str); 6] = [
        (
            "choose_data_folder",
            "takes an AppHandle; returns only DataFolderInfo",
        ),
        ("restart_app", "takes an AppHandle; returns nothing"),
        ("open_external_url", "takes an AppHandle; returns nothing"),
        (
            "import_pick",
            "native dialog needs an AppHandle; returns only the picked file's name",
        ),
        (
            "export_connections",
            "native dialog needs an AppHandle; returns only ExportInfo",
        ),
        (
            "export_backup",
            "native dialog needs an AppHandle; returns only ExportInfo",
        ),
    ];

    /// Plain functions that `table()` calls but that are deliberately not IPC commands:
    /// `apply_data_folder` takes a path, so exposing it would let the webview point the data
    /// folder anywhere. Only the native picker (`choose_data_folder`) may reach it.
    ///
    /// `import_begin`, `export_connections_to` and `export_backup_to` take a path as well: the
    /// webview must never choose the file to read or write, only the native dialogs do.
    const NOT_A_COMMAND: [&str; 4] = [
        "apply_data_folder",
        "import_begin",
        "export_connections_to",
        "export_backup_to",
    ];

    #[test]
    fn command_table_covers_every_tauri_command() {
        // The attribute is split so this test's own source does not count as an occurrence.
        const ATTR: &str = concat!("#[tauri::", "command");
        let sources = [
            include_str!("commands.rs"),
            include_str!("vault_commands.rs"),
            include_str!("data_folder.rs"),
            include_str!("session_commands.rs"),
            include_str!("import_commands.rs"),
            include_str!("app_settings.rs"),
        ];
        // Only lines that start with the attribute count (not doc comments or strings), so
        // `#[tauri::command(async)]` and similar forms are counted too.
        let declared: usize = sources
            .iter()
            .map(|s| s.lines().filter(|l| l.trim().starts_with(ATTR)).count())
            .sum();
        let names: Vec<&str> = table()
            .iter()
            .map(|(n, _)| *n)
            .filter(|n| !NOT_A_COMMAND.contains(n))
            .collect();
        let exempt: Vec<&str> = APP_HANDLE_EXEMPT.iter().map(|(n, _)| *n).collect();
        let covered: Vec<&str> = names.iter().chain(&exempt).copied().collect();
        assert_eq!(
            declared,
            covered.len(),
            "a tauri command was added or removed without updating table() / APP_HANDLE_EXEMPT"
        );
        // every covered name is a real wrapper and is registered in run()
        let lib = include_str!("lib.rs");
        for name in NOT_A_COMMAND {
            assert!(
                !lib.contains(name),
                "{name} must not be registered as an IPC command"
            );
            assert!(
                !sources.iter().any(|s| {
                    s.lines().zip(s.lines().skip(1)).any(|(a, b)| {
                        a.trim().starts_with(ATTR) && b.contains(&format!("fn {name}("))
                    })
                }),
                "{name} must not carry the command attribute"
            );
        }
        for name in &covered {
            // exempt commands only exist as handlers; the others also as a plain function
            let wrapper = format!("fn {name}(");
            let needed = if exempt.contains(name) { 1 } else { 2 };
            assert!(
                sources
                    .iter()
                    .any(|s| s.matches(&wrapper).count() >= needed),
                "{name} needs a plain function and a handler"
            );
            assert!(
                lib.contains(&format!("handlers::{name},")),
                "{name} is not registered in run()"
            );
        }
        // and the other way round: every handler registered in run() has a table entry
        let list = lib
            .split("generate_handler![")
            .nth(1)
            .and_then(|rest| rest.split(']').next())
            .expect("generate_handler! list in lib.rs");
        let registered = list.matches("handlers::").count();
        assert_eq!(
            registered,
            covered.len(),
            "run() registers a command that table() / APP_HANDLE_EXEMPT does not cover"
        );
    }
}
