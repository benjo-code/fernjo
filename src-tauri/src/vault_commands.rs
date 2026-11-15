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
use crate::state::{AppState, VaultState};

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
}

fn view(state: &AppState, vault: &VaultState) -> VaultStatus {
    match vault {
        VaultState::None => VaultStatus {
            state: "none",
            reason: None,
            path: None,
            auto_unlock: false,
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
        },
        VaultState::Unusable { reason, path } => VaultStatus {
            state: "unusable",
            reason: Some(reason.clone()),
            path: Some(path.display().to_string()),
            auto_unlock: false,
        },
    }
}

/// Runs `f` on the vault, or reports why there is none.
fn with_vault<T>(
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
    let vault = Vault::create_with(&path, &password, state.kdf)?;
    if remember {
        remember_key(state, &vault);
    }
    *guard = VaultState::Open(vault);
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
    use fernjo_core::{NodeId, Protocol};
    use fernjo_vault::{AutoUnlock, DataKey, KeyStore, KeyStoreError, Vault, VaultId};
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
            entry("apply_data_folder", |w| {
                // not an IPC command (see `NOT_A_COMMAND`), but called here as a plain
                // function: the state has no config folder, so this is refused; the response
                // and error must be free of secrets either way
                j(crate::data_folder::apply_data_folder(&w.st, w._dir.path()))
            }),
        ]
    }

    fn assert_no_sentinel(what: &str, text: &str) {
        for s in SENTINELS {
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
    const APP_HANDLE_EXEMPT: [(&str, &str); 2] = [
        (
            "choose_data_folder",
            "takes an AppHandle; returns only DataFolderInfo",
        ),
        ("restart_app", "takes an AppHandle; returns nothing"),
    ];

    /// Plain functions that `table()` calls but that are deliberately not IPC commands:
    /// `apply_data_folder` takes a path, so exposing it would let the webview point the data
    /// folder anywhere. Only the native picker (`choose_data_folder`) may reach it.
    const NOT_A_COMMAND: [&str; 1] = ["apply_data_folder"];

    #[test]
    fn command_table_covers_every_tauri_command() {
        // The attribute is split so this test's own source does not count as an occurrence.
        const ATTR: &str = concat!("#[tauri::", "command");
        let sources = [
            include_str!("commands.rs"),
            include_str!("vault_commands.rs"),
            include_str!("data_folder.rs"),
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
