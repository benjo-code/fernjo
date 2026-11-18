//! Tauri commands for the connection tree.
//!
//! Each command is a plain function over `&AppState` (testable without Tauri) plus a thin
//! wrapper in [`handlers`]. Every mutation returns the new full snapshot.

use fernjo_core::{
    Node, NodeId, NodePatch, Notice, Protocol, ResolvedNode, Store, StoreError, TreeError,
};
use serde::Serialize;
use std::sync::atomic::Ordering;

use crate::state::AppState;

/// The whole tree plus, once after startup, the notices (restored from backup, sync
/// conflicts, unreadable settings).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub children: Vec<Node>,
    pub notices: Vec<Notice>,
}

/// Result of a mutation: the affected id (if any) and the new tree.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Mutation {
    pub id: Option<NodeId>,
    pub snapshot: Snapshot,
}

/// Error shown to the UI: a stable `code` and a message fit for display.
#[derive(Debug, Serialize)]
pub struct CommandError {
    pub code: String,
    pub message: String,
}

impl CommandError {
    pub(crate) fn internal() -> CommandError {
        CommandError {
            code: "internal".to_string(),
            message: "the connection data is unavailable after an internal error".to_string(),
        }
    }
}

impl From<StoreError> for CommandError {
    fn from(e: StoreError) -> Self {
        CommandError {
            code: e.code().to_string(),
            message: e.to_string(),
        }
    }
}

impl From<TreeError> for CommandError {
    fn from(e: TreeError) -> Self {
        CommandError {
            code: e.code().to_string(),
            message: e.to_string(),
        }
    }
}

pub(crate) fn snapshot(store: &Store, notices: Vec<Notice>) -> Snapshot {
    Snapshot {
        children: store.tree().children().to_vec(),
        notices,
    }
}

/// Refuses mutations while a change (such as a moved data folder) waits for a restart.
pub fn ensure_writable(state: &AppState) -> Result<(), CommandError> {
    if state.restart_required.load(Ordering::SeqCst) {
        return Err(CommandError {
            code: "restart_required".to_string(),
            message: "restart fernjo to apply the changed data folder before editing".to_string(),
        });
    }
    Ok(())
}

/// Runs `f` on the store, or reports why there is none.
pub(crate) fn with_store<T>(
    state: &AppState,
    f: impl FnOnce(&mut Store) -> Result<T, CommandError>,
) -> Result<T, CommandError> {
    let mut guard = state.store.lock().map_err(|_| CommandError::internal())?;
    match &mut *guard {
        Ok(store) => f(store),
        Err(failure) => Err(CommandError {
            code: failure.code.clone(),
            message: failure.message.clone(),
        }),
    }
}

fn mutate(
    state: &AppState,
    f: impl FnOnce(&mut Store) -> Result<Option<NodeId>, StoreError>,
) -> Result<Mutation, CommandError> {
    ensure_writable(state)?;
    with_store(state, |store| {
        let id = f(store)?;
        Ok(Mutation {
            id,
            snapshot: snapshot(store, Vec::new()),
        })
    })
}

pub fn list_tree(state: &AppState) -> Result<Snapshot, CommandError> {
    with_store(state, |store| {
        let notices =
            std::mem::take(&mut *state.notices.lock().map_err(|_| CommandError::internal())?);
        Ok(snapshot(store, notices))
    })
}

pub fn get_node(state: &AppState, id: NodeId) -> Result<ResolvedNode, CommandError> {
    with_store(state, |store| Ok(store.tree().resolve_node(id)?))
}

pub fn create_folder(
    state: &AppState,
    parent_id: Option<NodeId>,
    index: Option<usize>,
    name: String,
) -> Result<Mutation, CommandError> {
    mutate(state, |s| {
        s.create_folder(parent_id, index, &name).map(Some)
    })
}

pub fn create_connection(
    state: &AppState,
    parent_id: Option<NodeId>,
    index: Option<usize>,
    name: String,
    protocol: Protocol,
    host: String,
) -> Result<Mutation, CommandError> {
    mutate(state, |s| {
        s.create_connection(parent_id, index, &name, protocol, &host)
            .map(Some)
    })
}

pub fn update_node(
    state: &AppState,
    id: NodeId,
    patch: NodePatch,
) -> Result<Mutation, CommandError> {
    mutate(state, |s| s.update(id, patch).map(|()| Some(id)))
}

pub fn move_node(
    state: &AppState,
    id: NodeId,
    parent_id: Option<NodeId>,
    index: usize,
) -> Result<Mutation, CommandError> {
    mutate(state, |s| {
        s.move_node(id, parent_id, index).map(|()| Some(id))
    })
}

pub fn duplicate_node(state: &AppState, id: NodeId) -> Result<Mutation, CommandError> {
    mutate(state, |s| s.duplicate(id).map(Some))
}

pub fn delete_node(state: &AppState, id: NodeId) -> Result<Mutation, CommandError> {
    mutate(state, |s| s.delete(id).map(|_| None))
}

/// The `#[tauri::command]` wrappers registered in `run()`.
pub mod handlers {
    use super::*;
    use tauri::State;

    #[tauri::command]
    pub fn list_tree(state: State<'_, AppState>) -> Result<Snapshot, CommandError> {
        super::list_tree(&state)
    }

    #[tauri::command]
    pub fn get_node(state: State<'_, AppState>, id: NodeId) -> Result<ResolvedNode, CommandError> {
        super::get_node(&state, id)
    }

    #[tauri::command]
    pub fn create_folder(
        state: State<'_, AppState>,
        parent_id: Option<NodeId>,
        index: Option<usize>,
        name: String,
    ) -> Result<Mutation, CommandError> {
        super::create_folder(&state, parent_id, index, name)
    }

    #[tauri::command]
    pub fn create_connection(
        state: State<'_, AppState>,
        parent_id: Option<NodeId>,
        index: Option<usize>,
        name: String,
        protocol: Protocol,
        host: String,
    ) -> Result<Mutation, CommandError> {
        super::create_connection(&state, parent_id, index, name, protocol, host)
    }

    #[tauri::command]
    pub fn update_node(
        state: State<'_, AppState>,
        id: NodeId,
        patch: NodePatch,
    ) -> Result<Mutation, CommandError> {
        super::update_node(&state, id, patch)
    }

    #[tauri::command]
    pub fn move_node(
        state: State<'_, AppState>,
        id: NodeId,
        parent_id: Option<NodeId>,
        index: usize,
    ) -> Result<Mutation, CommandError> {
        super::move_node(&state, id, parent_id, index)
    }

    #[tauri::command]
    pub fn duplicate_node(
        state: State<'_, AppState>,
        id: NodeId,
    ) -> Result<Mutation, CommandError> {
        super::duplicate_node(&state, id)
    }

    #[tauri::command]
    pub fn delete_node(state: State<'_, AppState>, id: NodeId) -> Result<Mutation, CommandError> {
        super::delete_node(&state, id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::VaultStatus;
    use fernjo_core::{FILE_NAME, Notice, Protocol, Store};
    use serde_json::json;
    use std::fs;
    use std::path::Path;

    fn state(dir: &Path) -> AppState {
        AppState::open(Some(dir.to_path_buf()))
    }

    #[test]
    fn list_tree_on_fresh_dir_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let snapshot = list_tree(&state(dir.path())).unwrap();
        assert!(snapshot.children.is_empty());
        assert!(snapshot.notices.is_empty());
    }

    #[test]
    fn create_folder_returns_id_and_snapshot_containing_it() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path());
        let m = create_folder(&st, None, None, "Homelab".into()).unwrap();
        let id = m.id.expect("created id");
        assert_eq!(m.snapshot.children.len(), 1);
        assert_eq!(m.snapshot.children[0].id, id);
        assert_eq!(m.snapshot.children[0].name, "Homelab");
        assert!(dir.path().join(FILE_NAME).exists());
    }

    #[test]
    fn empty_name_maps_to_code_empty_name() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path());
        let err = create_folder(&st, None, None, "   ".into()).unwrap_err();
        assert_eq!(err.code, "empty_name");
        assert!(!err.message.is_empty());
    }

    #[test]
    fn save_failure_maps_to_save_failed_and_keeps_the_old_tree() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path());
        create_folder(&st, None, None, "kept".into()).unwrap();
        // replace the data folder by a regular file so the next save must fail
        fs::remove_dir_all(dir.path()).unwrap();
        fs::write(dir.path(), "x").unwrap();
        let err = create_folder(&st, None, None, "lost".into()).unwrap_err();
        assert_eq!(err.code, "save_failed");
        let snapshot = list_tree(&st).unwrap();
        assert_eq!(snapshot.children.len(), 1);
        assert_eq!(snapshot.children[0].name, "kept");
        let _ = fs::remove_file(dir.path());
    }

    #[test]
    fn failed_startup_makes_every_command_return_load_failed_naming_the_file() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join(FILE_NAME),
            r#"{"schemaVersion":99,"children":[]}"#,
        )
        .unwrap();
        let st = state(dir.path());
        let id = fernjo_core::NodeId::new_v4();
        let errors = [
            list_tree(&st).map(|_| ()),
            get_node(&st, id).map(|_| ()),
            create_folder(&st, None, None, "x".into()).map(|_| ()),
            create_connection(&st, None, None, "x".into(), Protocol::Ssh, "h".into()).map(|_| ()),
            update_node(&st, id, Default::default()).map(|_| ()),
            move_node(&st, id, None, 0).map(|_| ()),
            duplicate_node(&st, id).map(|_| ()),
            delete_node(&st, id).map(|_| ()),
        ];
        for result in errors {
            let err = result.unwrap_err();
            assert_eq!(err.code, "load_failed");
            assert!(err.message.contains(FILE_NAME), "{}", err.message);
        }
        // the refused file was not touched
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn missing_data_dir_is_a_load_failed_startup_error() {
        let st = AppState::open(None);
        assert_eq!(list_tree(&st).unwrap_err().code, "load_failed");
    }

    #[test]
    fn startup_notice_is_returned_once() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut store = Store::open(dir.path()).unwrap().store;
            store.create_folder(None, None, "one").unwrap();
            store.create_folder(None, None, "two").unwrap();
        }
        fs::write(dir.path().join(FILE_NAME), "garbage").unwrap();
        let st = state(dir.path());
        let first = list_tree(&st).unwrap();
        assert!(matches!(
            first.notices.as_slice(),
            [Notice::RestoredFromBackup { bad_file: Some(_) }]
        ));
        assert_eq!(first.children.len(), 1);
        assert!(list_tree(&st).unwrap().notices.is_empty());
    }

    #[test]
    fn mutations_never_carry_the_startup_notice() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut store = Store::open(dir.path()).unwrap().store;
            store.create_folder(None, None, "one").unwrap();
            store.create_folder(None, None, "two").unwrap();
        }
        fs::write(dir.path().join(FILE_NAME), "garbage").unwrap();
        let st = state(dir.path());
        let m = create_folder(&st, None, None, "x".into()).unwrap();
        assert!(m.snapshot.notices.is_empty());
        assert_eq!(list_tree(&st).unwrap().notices.len(), 1);
    }

    #[test]
    fn startup_with_conflict_files_delivers_a_sync_conflicts_notice_once() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path()
                .join("connections (conflicted copy 2026-11-15).json"),
            "{}",
        )
        .unwrap();
        let st = state(dir.path());
        let first = list_tree(&st).unwrap();
        match first.notices.as_slice() {
            [Notice::SyncConflicts { files }] => {
                assert_eq!(files, &["connections (conflicted copy 2026-11-15).json"]);
            }
            other => panic!("unexpected notices: {other:?}"),
        }
        assert!(list_tree(&st).unwrap().notices.is_empty());
    }

    #[test]
    fn restored_from_backup_notice_still_arrives_in_the_notices_list() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut store = Store::open(dir.path()).unwrap().store;
            store.create_folder(None, None, "one").unwrap();
            store.create_folder(None, None, "two").unwrap();
        }
        fs::write(dir.path().join(FILE_NAME), "garbage").unwrap();
        fs::write(
            dir.path()
                .join("connections (conflicted copy 2026-11-15).json"),
            "{}",
        )
        .unwrap();
        let st = state(dir.path());
        let notices = list_tree(&st).unwrap().notices;
        assert!(
            notices
                .iter()
                .any(|n| matches!(n, Notice::RestoredFromBackup { .. }))
        );
        assert!(
            notices
                .iter()
                .any(|n| matches!(n, Notice::SyncConflicts { .. }))
        );
        let json = serde_json::to_value(&notices).unwrap();
        assert_eq!(json[0]["kind"], "restoredFromBackup");
    }

    #[test]
    fn vault_missing_is_none_and_zero_length_file_is_unusable_with_path() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(state(dir.path()).vault_status().unwrap(), VaultStatus::None);
        let path = dir.path().join(fernjo_vault::FILE_NAME);
        fs::write(&path, b"").unwrap();
        match state(dir.path()).vault_status().unwrap() {
            VaultStatus::Unusable { reason, path: p } => {
                assert_eq!(p, path);
                assert!(!reason.is_empty());
            }
            other => panic!("unexpected: {other:?}"),
        }
        fs::write(&path, b"FJVT-truncated").unwrap();
        assert!(matches!(
            state(dir.path()).vault_status().unwrap(),
            VaultStatus::Unusable { .. }
        ));
        assert_eq!(fs::read(&path).unwrap(), b"FJVT-truncated");
    }

    #[test]
    fn unusable_vault_does_not_break_list_tree() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(fernjo_vault::FILE_NAME), b"").unwrap();
        let st = state(dir.path());
        create_folder(&st, None, None, "kept".into()).unwrap();
        assert_eq!(list_tree(&st).unwrap().children.len(), 1);
    }

    #[test]
    fn startup_auto_unlocks_when_the_key_store_has_the_key() {
        use fernjo_vault::{KeyStore, MemoryKeyStore, Vault};
        use secrecy::SecretString;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(fernjo_vault::FILE_NAME);
        let vault = Vault::create_with(
            &path,
            &SecretString::from("correct horse battery"),
            fernjo_vault::KdfParams::TEST,
        )
        .unwrap();
        let keys = MemoryKeyStore::default();
        keys.set(vault.id(), &vault.data_key().unwrap()).unwrap();
        let locked = AppState::open_with_keystore(
            dir.path().to_path_buf(),
            Box::new(MemoryKeyStore::default()),
        );
        assert_eq!(locked.vault_status().unwrap(), VaultStatus::Locked);
        let st = AppState::open_with_keystore(dir.path().to_path_buf(), Box::new(keys));
        assert_eq!(st.vault_status().unwrap(), VaultStatus::Unlocked);
    }

    #[test]
    fn settings_error_becomes_a_notice() {
        let dir = tempfile::tempdir().unwrap();
        let st = AppState::open_for_app(Some(fernjo_core::Location {
            data_dir: dir.path().to_path_buf(),
            config_dir: dir.path().to_path_buf(),
            source: fernjo_core::DataDirSource::Default,
            device_dir: None,
            settings_error: Some(fernjo_core::LoadError::Unreadable {
                path: dir.path().join("settings.json"),
                reason: "bad".into(),
            }),
        }));
        assert!(st.settings_unusable);
        let notices = list_tree(&st).unwrap().notices;
        assert!(matches!(
            notices.as_slice(),
            [Notice::SettingsUnreadable { .. }]
        ));
    }

    #[test]
    fn mutating_commands_return_restart_required_when_flagged_and_reads_still_work() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path());
        let id = create_folder(&st, None, None, "a".into())
            .unwrap()
            .id
            .unwrap();
        st.set_restart_required(true);
        let errors = [
            create_folder(&st, None, None, "x".into()).map(|_| ()),
            create_connection(&st, None, None, "x".into(), Protocol::Ssh, "h".into()).map(|_| ()),
            update_node(&st, id, Default::default()).map(|_| ()),
            move_node(&st, id, None, 0).map(|_| ()),
            duplicate_node(&st, id).map(|_| ()),
            delete_node(&st, id).map(|_| ()),
        ];
        for result in errors {
            assert_eq!(result.unwrap_err().code, "restart_required");
        }
        assert_eq!(list_tree(&st).unwrap().children.len(), 1);
        assert!(get_node(&st, id).is_ok());
    }

    #[test]
    fn move_duplicate_update_and_delete_return_fresh_snapshots() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path());
        let a = create_folder(&st, None, None, "a".into())
            .unwrap()
            .id
            .unwrap();
        let c = create_connection(&st, Some(a), None, "c".into(), Protocol::Ssh, "h".into())
            .unwrap()
            .id
            .unwrap();
        let moved = move_node(&st, c, None, 0).unwrap();
        assert_eq!(moved.id, Some(c));
        assert_eq!(moved.snapshot.children[0].id, c);

        let patch: fernjo_core::NodePatch =
            serde_json::from_value(json!({ "name": "c2" })).unwrap();
        let updated = update_node(&st, c, patch).unwrap();
        assert_eq!(updated.snapshot.children[0].name, "c2");

        let copy = duplicate_node(&st, c).unwrap();
        assert_ne!(copy.id, Some(c));
        assert_eq!(copy.snapshot.children.len(), 3);

        let deleted = delete_node(&st, a).unwrap();
        assert_eq!(deleted.id, None);
        assert_eq!(deleted.snapshot.children.len(), 2);

        let other = copy.id.unwrap();
        let err = move_node(&st, c, Some(other), 0).unwrap_err();
        assert_eq!(err.code, "not_a_folder");
        let err = move_node(&st, c, Some(c), 0).unwrap_err();
        assert_eq!(err.code, "invalid_move");
    }

    #[test]
    fn snapshot_json_uses_camel_case_and_node_tags() {
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path());
        create_folder(&st, None, None, "Homelab".into()).unwrap();
        let c = create_connection(&st, None, None, "web".into(), Protocol::Ssh, "h".into())
            .unwrap()
            .id
            .unwrap();

        let snapshot = serde_json::to_value(list_tree(&st).unwrap()).unwrap();
        assert_eq!(snapshot["children"][0]["type"], "folder");
        assert_eq!(snapshot["children"][1]["type"], "connection");
        assert_eq!(snapshot["children"][1]["protocol"], "ssh");

        let mutation = serde_json::to_value(delete_node(&st, c).unwrap()).unwrap();
        assert!(mutation["id"].is_null());
        assert!(mutation["snapshot"]["children"].is_array());

        let c = create_connection(&st, None, None, "web".into(), Protocol::Ssh, "h".into())
            .unwrap()
            .id
            .unwrap();
        let node = serde_json::to_value(get_node(&st, c).unwrap()).unwrap();
        assert_eq!(node["keepaliveSecs"]["effective"], 30);
        assert_eq!(node["username"]["source"]["kind"], "default");
        assert_eq!(node["port"]["effective"], 22);
        assert_eq!(node["kind"], "connection");

        let err =
            serde_json::to_value(create_folder(&st, None, None, " ".into()).unwrap_err()).unwrap();
        assert_eq!(err["code"], "empty_name");
    }

    /// Security review M1: creating, moving and duplicating past `MAX_TREE_DEPTH` is an ordinary
    /// command error (the UI shows its message) and changes neither the tree nor the file.
    #[test]
    fn creating_or_moving_below_the_depth_limit_is_a_too_deep_command_error() {
        use fernjo_core::MAX_TREE_DEPTH;
        let dir = tempfile::tempdir().unwrap();
        let st = state(dir.path());
        let mut parent = None;
        for i in 0..MAX_TREE_DEPTH {
            parent = create_folder(&st, parent, None, format!("level {i}"))
                .unwrap()
                .id;
        }
        let deepest = parent.unwrap();
        let spare = create_folder(&st, None, None, "spare".into())
            .unwrap()
            .id
            .unwrap();
        let on_disk = fs::read(dir.path().join(FILE_NAME)).unwrap();

        let errors = [
            create_folder(&st, Some(deepest), None, "one too many".into()),
            create_connection(
                &st,
                Some(deepest),
                None,
                "c".into(),
                Protocol::Ssh,
                "h".into(),
            ),
            move_node(&st, spare, Some(deepest), 0),
        ];
        for result in errors {
            let err = result.unwrap_err();
            assert_eq!(err.code, "too_deep");
            assert!(err.message.contains("nested"), "{}", err.message);
        }
        // the file on disk is unchanged and still loads
        assert_eq!(fs::read(dir.path().join(FILE_NAME)).unwrap(), on_disk);
        assert!(
            Store::open(dir.path()).unwrap().notice.is_none(),
            "the file must load"
        );
    }
}
