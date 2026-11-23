//! The tabs of the last run (`open-tabs.json`) and the commands around them.
//!
//! The file lives in the device folder (`AppState::device_dir`), never in the synced data folder.
//! It holds only targets (a node id, or the host, port and user of a quick connect): no names,
//! no protocol of a node, no credential, nothing that could go stale or leak. Title, protocol and
//! whether a vault credential is involved come from the tree at restore time. The file is a
//! convenience: a missing, damaged, oversized or newer-schema file reads as empty and the next
//! save replaces it, so it can never keep the app from starting or saving.

use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;

use fernjo_core::{DEFAULT_RDP_PORT, DEFAULT_SSH_PORT, NodeId, Protocol, QuickTarget};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::commands::{CommandError, with_store};
use crate::connect_plan::Target;
use crate::session_commands::{UnsavedReason, persistent, unsaved_reason};
use crate::state::AppState;

/// Name of the file inside the device folder.
pub const FILE_NAME: &str = "open-tabs.json";
/// Most tabs that are saved and restored.
pub const MAX_TABS: usize = 50;
/// File format version this build writes and understands.
pub const CURRENT_SCHEMA: u32 = 1;
/// Largest file that is read: 50 short entries need a few KiB, so anything beyond this is not
/// something this app wrote.
const MAX_FILE_LEN: u64 = 1024 * 1024;

fn error(code: &str, message: impl Into<String>) -> CommandError {
    CommandError {
        code: code.to_string(),
        message: message.into(),
    }
}

/// One saved tab: exactly one of `node` and `quick` is set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedTab {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<NodeId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quick: Option<QuickTarget>,
}

/// The content of `open-tabs.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenTabsFile {
    pub schema: u32,
    pub tabs: Vec<SavedTab>,
    /// Index into `tabs` of the active tab.
    pub active: Option<usize>,
}

impl Default for OpenTabsFile {
    fn default() -> Self {
        Self {
            schema: CURRENT_SCHEMA,
            tabs: Vec::new(),
            active: None,
        }
    }
}

/// Reads the file; anything but a well-formed file of this schema gives an empty list.
pub fn load(device_dir: &Path) -> OpenTabsFile {
    read(device_dir).unwrap_or_default()
}

fn read(device_dir: &Path) -> Option<OpenTabsFile> {
    let path = device_dir.join(FILE_NAME);
    let file = fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > MAX_FILE_LEN {
        return None;
    }
    let mut bytes = Vec::new();
    // bounded in case the file grows while it is read
    file.take(MAX_FILE_LEN + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_FILE_LEN {
        return None;
    }
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    let schema = value.get("schema")?.as_u64()?;
    if schema != u64::from(CURRENT_SCHEMA) {
        return None;
    }
    serde_json::from_value(value).ok()
}

/// Writes the file atomically: a random-named temp file (`create_new`, so a planted link is never
/// followed), flushed, renamed over the old one. No `.bak`: the file is disposable.
///
/// # Errors
/// Any I/O error; the temp file is removed.
pub fn save(device_dir: &Path, file: &OpenTabsFile) -> io::Result<()> {
    let json = serde_json::to_vec_pretty(file).map_err(io::Error::other)?;
    fs::create_dir_all(device_dir)?;
    let path = device_dir.join(FILE_NAME);
    let tmp = device_dir.join(format!("{FILE_NAME}.{}.tmp", uuid::Uuid::new_v4().simple()));
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut out = options.open(&tmp)?;
        out.write_all(&json)?;
        out.flush()?;
        out.sync_all()?;
        drop(out);
        fs::rename(&tmp, &path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Deletes the file; a missing file or a failed delete is not an error (it is only a convenience).
pub fn remove(device_dir: &Path) {
    let _ = fs::remove_file(device_dir.join(FILE_NAME));
}

/// A session target on the wire: exactly one of `node` (an id) and `quick`. Used for a restorable
/// tab and as the argument of `restore_session`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetView {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<NodeId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quick: Option<QuickTarget>,
}

impl TargetView {
    /// # Errors
    /// `invalid_target` unless exactly one of `node` and `quick` is set.
    pub fn into_target(self) -> Result<Target, CommandError> {
        match (self.node, self.quick) {
            (Some(id), None) => Ok(Target::Node(id)),
            (None, Some(quick)) => Ok(Target::Quick(quick)),
            _ => Err(error(
                "invalid_target",
                "a target is either a connection or a quick connect target",
            )),
        }
    }
}

/// A saved tab that still resolves, with what the UI needs to show it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestorableTab {
    pub target: TargetView,
    pub protocol: Protocol,
    pub title: String,
    /// The connection's effective credential is set (so connecting needs the vault).
    pub uses_vault: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenTabsInfo {
    /// The "Reopen tabs on start" setting.
    pub reopen: bool,
    pub persistent: bool,
    pub unsaved_reason: Option<UnsavedReason>,
    /// Index into `tabs` of the tab that was active.
    pub active: Option<usize>,
    pub tabs: Vec<RestorableTab>,
}

fn quick_title(quick: &QuickTarget) -> String {
    let default_port = match quick.protocol {
        Protocol::Ssh => DEFAULT_SSH_PORT,
        Protocol::Rdp => DEFAULT_RDP_PORT,
    };
    let mut title = match &quick.user {
        Some(user) => format!("{user}@{}", quick.host),
        None => quick.host.clone(),
    };
    if quick.port != default_port {
        title = format!("{title}:{}", quick.port);
    }
    title
}

/// Resolves a saved tab against the tree as it is now; `None` when the node is gone, a folder or
/// the quick target is not valid.
fn resolve_tab(state: &AppState, tab: &SavedTab) -> Option<RestorableTab> {
    match (tab.node, &tab.quick) {
        (Some(id), None) => {
            let node = with_store(state, |s| Ok(s.tree().resolve_node(id)?)).ok()?;
            Some(RestorableTab {
                target: TargetView {
                    node: Some(id),
                    quick: None,
                },
                protocol: node.protocol?,
                title: node.name,
                uses_vault: node.credential.effective.is_some(),
            })
        }
        (None, Some(quick)) => {
            fernjo_core::validate_quick(quick).ok()?;
            Some(RestorableTab {
                target: TargetView {
                    node: None,
                    quick: Some(quick.clone()),
                },
                protocol: quick.protocol,
                title: quick_title(quick),
                uses_vault: false,
            })
        }
        _ => None,
    }
}

fn lock_reopen(state: &AppState) -> Result<std::sync::MutexGuard<'_, bool>, CommandError> {
    state
        .reopen_tabs
        .lock()
        .map_err(|_| CommandError::internal())
}

fn info(state: &AppState, reopen: bool) -> OpenTabsInfo {
    let mut info = OpenTabsInfo {
        reopen,
        persistent: persistent(state),
        unsaved_reason: unsaved_reason(state),
        active: None,
        tabs: Vec::new(),
    };
    let Some(dir) = state.device_dir.as_deref().filter(|_| reopen) else {
        return info;
    };
    let file = load(dir);
    for (index, tab) in file.tabs.iter().take(MAX_TABS).enumerate() {
        if let Some(resolved) = resolve_tab(state, tab) {
            if file.active == Some(index) {
                info.active = Some(info.tabs.len());
            }
            info.tabs.push(resolved);
        }
    }
    info
}

/// The tabs of the last run that still resolve, in order, and the setting. Nothing is connected.
pub fn get_open_tabs(state: &AppState) -> Result<OpenTabsInfo, CommandError> {
    let reopen = *lock_reopen(state)?;
    Ok(info(state, reopen))
}

/// Turns "Reopen tabs on start" on or off and saves it to `settings.json` (keeping the other
/// fields) unless that file must not be written. Off also deletes `open-tabs.json`.
///
/// # Errors
/// `settings_unusable`, `settings_write_failed`; nothing changes on an error.
pub fn set_reopen_tabs(state: &AppState, enabled: bool) -> Result<OpenTabsInfo, CommandError> {
    let mut slot = lock_reopen(state)?;
    crate::app_settings::update_settings(state, |s| s.reopen_tabs = Some(enabled))?;
    *slot = enabled;
    if !enabled && let Some(dir) = &state.device_dir {
        remove(dir);
    }
    Ok(info(state, enabled))
}

/// Stores the UI's ordered tab list. Does nothing (and writes no file) while "Reopen tabs" is off
/// or the device has no folder. Entries that are not exactly one of node and quick target, or whose
/// quick target is invalid, are dropped; at most [`MAX_TABS`] are kept; `active` follows its tab.
///
/// # Errors
/// `open_tabs_write_failed`; callers treat it as a note, never as a reason to stop.
pub fn save_open_tabs(
    state: &AppState,
    tabs: Vec<SavedTab>,
    active: Option<usize>,
) -> Result<(), CommandError> {
    let reopen = lock_reopen(state)?;
    let Some(dir) = state.device_dir.as_deref().filter(|_| *reopen) else {
        return Ok(());
    };
    let mut kept = Vec::new();
    let mut kept_active = None;
    for (index, tab) in tabs.into_iter().enumerate() {
        let valid = match (&tab.node, &tab.quick) {
            (Some(_), None) => true,
            (None, Some(quick)) => fernjo_core::validate_quick(quick).is_ok(),
            _ => false,
        };
        if !valid || kept.len() >= MAX_TABS {
            continue;
        }
        if active == Some(index) {
            kept_active = Some(kept.len());
        }
        kept.push(tab);
    }
    save(
        dir,
        &OpenTabsFile {
            schema: CURRENT_SCHEMA,
            tabs: kept,
            active: kept_active,
        },
    )
    .map_err(|e| {
        error(
            "open_tabs_write_failed",
            format!("could not save the open tabs: {e}"),
        )
    })
}

pub mod handlers {
    use tauri::State;

    use super::{CommandError, OpenTabsInfo, SavedTab};
    use crate::state::AppState;

    #[tauri::command]
    pub fn get_open_tabs(state: State<'_, AppState>) -> Result<OpenTabsInfo, CommandError> {
        super::get_open_tabs(&state)
    }

    #[tauri::command]
    pub fn set_reopen_tabs(
        state: State<'_, AppState>,
        enabled: bool,
    ) -> Result<OpenTabsInfo, CommandError> {
        super::set_reopen_tabs(&state, enabled)
    }

    #[tauri::command]
    pub fn save_open_tabs(
        state: State<'_, AppState>,
        tabs: Vec<SavedTab>,
        active: Option<usize>,
    ) -> Result<(), CommandError> {
        super::save_open_tabs(&state, tabs, active)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands;
    use fernjo_core::{AppSettings, DataDirSource, NodePatch, settings};
    use serde_json::json;
    use std::path::PathBuf;

    fn quick(host: &str) -> QuickTarget {
        QuickTarget {
            protocol: Protocol::Ssh,
            host: host.into(),
            port: 22,
            user: Some("root".into()),
        }
    }

    fn node_tab(id: NodeId) -> SavedTab {
        SavedTab {
            node: Some(id),
            quick: None,
        }
    }

    fn quick_tab(host: &str) -> SavedTab {
        SavedTab {
            node: None,
            quick: Some(quick(host)),
        }
    }

    /// A state with a data folder and a device folder (no `settings.json` folder).
    fn rig() -> (tempfile::TempDir, tempfile::TempDir, AppState) {
        let data = tempfile::tempdir().unwrap();
        let device = tempfile::tempdir().unwrap();
        let st =
            AppState::open_with_device_dir(data.path().to_path_buf(), device.path().to_path_buf());
        (data, device, st)
    }

    fn connection(st: &AppState, name: &str, protocol: Protocol) -> NodeId {
        commands::create_connection(st, None, None, name.into(), protocol, "h.example".into())
            .unwrap()
            .id
            .unwrap()
    }

    fn patch(v: serde_json::Value) -> NodePatch {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn a_missing_file_loads_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()), OpenTabsFile::default());
    }

    #[test]
    fn a_damaged_file_loads_empty_and_the_next_save_rewrites_it() {
        let dir = tempfile::tempdir().unwrap();
        for junk in [&b"{ not json"[..], b"", b"[]", b"{\"schema\":1,\"tabs\":5}"] {
            fs::write(dir.path().join(FILE_NAME), junk).unwrap();
            assert_eq!(load(dir.path()), OpenTabsFile::default());
        }
        let file = OpenTabsFile {
            schema: CURRENT_SCHEMA,
            tabs: vec![quick_tab("a.example")],
            active: Some(0),
        };
        save(dir.path(), &file).unwrap();
        assert_eq!(load(dir.path()), file);
    }

    #[test]
    fn a_newer_schema_file_loads_empty() {
        let dir = tempfile::tempdir().unwrap();
        let text = json!({"schema": 2, "tabs": [{"quick": quick("a.example")}], "active": 0});
        fs::write(dir.path().join(FILE_NAME), text.to_string()).unwrap();
        assert_eq!(load(dir.path()), OpenTabsFile::default());
    }

    #[test]
    fn a_file_over_the_cap_loads_empty() {
        let dir = tempfile::tempdir().unwrap();
        let file = fs::File::create(dir.path().join(FILE_NAME)).unwrap();
        file.set_len(MAX_FILE_LEN + 1).unwrap();
        drop(file);
        assert_eq!(load(dir.path()), OpenTabsFile::default());
    }

    #[test]
    fn save_then_load_round_trips_nodes_and_quick_targets() {
        let dir = tempfile::tempdir().unwrap();
        let file = OpenTabsFile {
            schema: CURRENT_SCHEMA,
            tabs: vec![
                node_tab(NodeId::new_v4()),
                quick_tab("a.example"),
                SavedTab {
                    node: None,
                    quick: Some(QuickTarget {
                        protocol: Protocol::Rdp,
                        host: "10.0.0.1".into(),
                        port: 3390,
                        user: None,
                    }),
                },
            ],
            active: Some(1),
        };
        save(dir.path(), &file).unwrap();
        assert_eq!(load(dir.path()), file);
        // no temp file is left behind and no `.bak` is written
        let names: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec![FILE_NAME.to_string()]);
    }

    fn keys(value: &Value, out: &mut std::collections::BTreeSet<String>) {
        match value {
            Value::Object(map) => {
                for (k, v) in map {
                    out.insert(k.clone());
                    keys(v, out);
                }
            }
            Value::Array(items) => items.iter().for_each(|v| keys(v, out)),
            _ => {}
        }
    }

    #[test]
    fn the_file_contains_no_secret_field_names() {
        let file = OpenTabsFile {
            schema: CURRENT_SCHEMA,
            tabs: vec![node_tab(NodeId::new_v4()), quick_tab("a.example")],
            active: Some(0),
        };
        let mut found = std::collections::BTreeSet::new();
        keys(&serde_json::to_value(&file).unwrap(), &mut found);
        let expected: std::collections::BTreeSet<String> = [
            "schema", "tabs", "active", "node", "quick", "protocol", "host", "port", "user",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        assert_eq!(found, expected);
    }

    #[test]
    fn the_file_lives_in_the_device_folder_never_the_data_folder() {
        let (data, device, st) = rig();
        save_open_tabs(&st, vec![quick_tab("a.example")], Some(0)).unwrap();
        assert!(device.path().join(FILE_NAME).is_file());
        assert!(!data.path().join(FILE_NAME).exists());
        let in_data: Vec<_> = fs::read_dir(data.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains("open-tabs"))
            .collect();
        assert!(in_data.is_empty(), "{in_data:?}");
    }

    #[test]
    fn get_open_tabs_drops_deleted_nodes_and_folders() {
        let (_d, _v, st) = rig();
        let kept = connection(&st, "kept", Protocol::Ssh);
        let gone = connection(&st, "gone", Protocol::Ssh);
        let folder = commands::create_folder(&st, None, None, "f".into())
            .unwrap()
            .id
            .unwrap();
        save_open_tabs(
            &st,
            vec![
                node_tab(gone),
                node_tab(folder),
                node_tab(NodeId::new_v4()),
                node_tab(kept),
                quick_tab("a.example"),
            ],
            Some(3),
        )
        .unwrap();
        commands::delete_node(&st, gone).unwrap();
        let info = get_open_tabs(&st).unwrap();
        let titles: Vec<&str> = info.tabs.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, ["kept", "root@a.example"]);
        assert_eq!(info.active, Some(0), "the active index follows its tab");
        assert!(info.reopen);
    }

    #[test]
    fn get_open_tabs_uses_the_current_protocol_of_a_changed_node() {
        let (_d, _v, st) = rig();
        let id = connection(&st, "box", Protocol::Ssh);
        save_open_tabs(&st, vec![node_tab(id)], Some(0)).unwrap();
        assert_eq!(get_open_tabs(&st).unwrap().tabs[0].protocol, Protocol::Ssh);
        commands::update_node(
            &st,
            id,
            patch(json!({"protocol": "rdp", "name": "renamed"})),
        )
        .unwrap();
        let tab = &get_open_tabs(&st).unwrap().tabs[0];
        assert_eq!(tab.protocol, Protocol::Rdp);
        assert_eq!(tab.title, "renamed");
    }

    #[test]
    fn get_open_tabs_marks_a_tab_that_uses_a_vault_credential() {
        let (_d, _v, st) = rig();
        let plain = connection(&st, "plain", Protocol::Ssh);
        let with = connection(&st, "with", Protocol::Ssh);
        let cred = uuid::Uuid::new_v4();
        commands::update_node(&st, with, patch(json!({"credential": cred}))).unwrap();
        save_open_tabs(
            &st,
            vec![node_tab(plain), node_tab(with), quick_tab("a.example")],
            None,
        )
        .unwrap();
        let uses: Vec<bool> = get_open_tabs(&st)
            .unwrap()
            .tabs
            .iter()
            .map(|t| t.uses_vault)
            .collect();
        assert_eq!(uses, [false, true, false]);
    }

    #[test]
    fn save_open_tabs_is_a_no_op_while_reopen_is_off() {
        let (_d, device, st) = rig();
        *st.reopen_tabs.lock().unwrap() = false;
        save_open_tabs(&st, vec![quick_tab("a.example")], Some(0)).unwrap();
        assert!(!device.path().join(FILE_NAME).exists());
        assert!(get_open_tabs(&st).unwrap().tabs.is_empty());
        // and without a device folder nothing happens either
        let data = tempfile::tempdir().unwrap();
        let bare = AppState::open(Some(data.path().to_path_buf()));
        save_open_tabs(&bare, vec![quick_tab("a.example")], None).unwrap();
        assert!(get_open_tabs(&bare).unwrap().tabs.is_empty());
    }

    #[test]
    fn turning_reopen_off_removes_the_file() {
        let (_d, device, st) = rig();
        save_open_tabs(&st, vec![quick_tab("a.example")], Some(0)).unwrap();
        assert!(device.path().join(FILE_NAME).is_file());
        let info = set_reopen_tabs(&st, false).unwrap();
        assert!(!info.reopen && info.tabs.is_empty());
        assert!(!device.path().join(FILE_NAME).exists());
        set_reopen_tabs(&st, true).unwrap();
        save_open_tabs(&st, vec![quick_tab("b.example")], Some(0)).unwrap();
        assert_eq!(get_open_tabs(&st).unwrap().tabs.len(), 1);
    }

    #[test]
    fn save_open_tabs_caps_the_count_and_drops_invalid_entries() {
        let (_d, device, st) = rig();
        let mut tabs = vec![
            SavedTab {
                node: None,
                quick: None,
            },
            SavedTab {
                node: Some(NodeId::new_v4()),
                quick: Some(quick("a.example")),
            },
            SavedTab {
                node: None,
                quick: Some(QuickTarget {
                    port: 0,
                    ..quick("b.example")
                }),
            },
        ];
        tabs.extend((0..MAX_TABS + 10).map(|i| quick_tab(&format!("h{i}.example"))));
        // the active tab is the 60th valid one: beyond the cap, so it falls away
        save_open_tabs(&st, tabs, Some(3 + MAX_TABS + 5)).unwrap();
        let file = load(device.path());
        assert_eq!(file.tabs.len(), MAX_TABS);
        assert_eq!(file.tabs[0], quick_tab("h0.example"));
        assert_eq!(file.active, None);
    }

    #[test]
    fn set_reopen_tabs_persists_and_keeps_other_settings() {
        let data = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        let mut st = AppState::open(Some(data.path().to_path_buf()));
        st.config_dir = Some(config.path().to_path_buf());
        st.source = DataDirSource::Default;
        let path: PathBuf = config.path().join(settings::FILE_NAME);
        settings::save(
            &path,
            &AppSettings {
                vault_auto_lock_minutes: Some(9),
                ..AppSettings::default()
            },
        )
        .unwrap();
        let info = set_reopen_tabs(&st, false).unwrap();
        assert!(!info.reopen && info.persistent && info.unsaved_reason.is_none());
        let saved = settings::load(&path).unwrap();
        assert_eq!(saved.reopen_tabs, Some(false));
        assert_eq!(saved.vault_auto_lock_minutes, Some(9));
        // other settings keep it, and it keeps theirs
        crate::app_settings::set_vault_auto_lock(&st, Some(11)).unwrap();
        assert_eq!(settings::load(&path).unwrap().reopen_tabs, Some(false));
        set_reopen_tabs(&st, true).unwrap();
        let saved = settings::load(&path).unwrap();
        assert_eq!(saved.reopen_tabs, Some(true));
        assert_eq!(saved.vault_auto_lock_minutes, Some(11));
        // under FERNJO_DATA_DIR the choice is memory only
        st.source = DataDirSource::Env;
        let info = set_reopen_tabs(&st, false).unwrap();
        assert_eq!(info.unsaved_reason, Some(UnsavedReason::Env));
        assert_eq!(settings::load(&path).unwrap().reopen_tabs, Some(true));
        assert!(!get_open_tabs(&st).unwrap().reopen);
    }

    #[test]
    fn a_target_view_needs_exactly_one_target() {
        let id = NodeId::new_v4();
        let view = |node, quick| TargetView { node, quick };
        assert!(matches!(
            view(Some(id), None).into_target(),
            Ok(Target::Node(n)) if n == id
        ));
        assert!(matches!(
            view(None, Some(quick("a.example"))).into_target(),
            Ok(Target::Quick(_))
        ));
        assert_eq!(
            view(None, None).into_target().unwrap_err().code,
            "invalid_target"
        );
        assert_eq!(
            view(Some(id), Some(quick("a.example")))
                .into_target()
                .unwrap_err()
                .code,
            "invalid_target"
        );
    }
}
