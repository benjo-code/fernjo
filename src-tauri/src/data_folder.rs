//! Tauri commands for the data folder: show where the data lives and move it.
//!
//! Like `commands.rs`: plain functions over `&AppState` (testable without Tauri) plus thin
//! wrappers in [`handlers`]. Changing the folder only copies the data files and writes
//! `settings.json`; the running app keeps using the old folder, every mutating command
//! reports `restart_required` until the app is restarted.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use fernjo_core::{AppSettings, DataDirSource, FILE_NAME as CONNECTIONS_FILE, settings};
use fernjo_vault::FILE_NAME as VAULT_FILE;
use serde::Serialize;

use crate::commands::CommandError;
use crate::state::AppState;

fn error(code: &str, message: impl Into<String>) -> CommandError {
    CommandError {
        code: code.to_string(),
        message: message.into(),
    }
}

/// Where the data lives and whether the user may move it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataFolderInfo {
    /// The folder in use; while a restart is pending, the folder chosen for the next start.
    pub path: String,
    /// `env`, `settings` or `default` (what picked the folder of this run).
    pub source: &'static str,
    /// `false` while `FERNJO_DATA_DIR` controls the folder, `settings.json` is unusable or
    /// there is no config folder to write it to.
    pub changeable: bool,
    pub restart_required: bool,
    /// Data files (`connections.json`, `vault.fernjo`, in that order) that were already in the
    /// chosen folder and were kept untouched, so the user sees what was adopted. Empty when
    /// files were copied, the folder was empty or no change is pending.
    pub existing_files: Vec<String>,
}

fn changeable(state: &AppState) -> bool {
    state.source != DataDirSource::Env && !state.settings_unusable && state.config_dir.is_some()
}

fn info(state: &AppState) -> Result<DataFolderInfo, CommandError> {
    let restart_required = state.restart_required.load(Ordering::SeqCst);
    let pending = if restart_required {
        state
            .pending_data_dir
            .lock()
            .map_err(|_| CommandError::internal())?
            .clone()
    } else {
        None
    };
    let (path, existing_files) = match pending {
        Some((path, existing)) => (Some(path), existing),
        None => (state.data_dir.clone(), Vec::new()),
    };
    Ok(DataFolderInfo {
        path: path.map(|p| p.display().to_string()).unwrap_or_default(),
        source: match state.source {
            DataDirSource::Env => "env",
            DataDirSource::Settings => "settings",
            DataDirSource::Default => "default",
        },
        changeable: changeable(state),
        restart_required,
        existing_files,
    })
}

pub fn get_data_folder(state: &AppState) -> Result<DataFolderInfo, CommandError> {
    info(state)
}

/// A test write: proves the folder accepts a new file, then removes it.
fn check_writable(dir: &Path) -> std::io::Result<()> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let probe = dir.join(format!(".fernjo-write-test-{}-{nanos}", std::process::id()));
    fs::write(&probe, b"fernjo")?;
    fs::remove_file(&probe)
}

fn not_usable(target: &Path, reason: impl std::fmt::Display) -> CommandError {
    error(
        "folder_not_usable",
        format!(
            "{} cannot be used as data folder: {reason}",
            target.display()
        ),
    )
}

/// Copies `from` to `to` through a temp file and a rename, so `to` never holds a partial file.
fn copy_atomic(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut tmp = to.as_os_str().to_owned();
    tmp.push(".fernjo-copy.tmp");
    let tmp = PathBuf::from(tmp);
    let result = fs::copy(from, &tmp).and_then(|_| fs::rename(&tmp, to));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Copies the data files of `from` into `target` when `target` has neither of them. A failure
/// removes whatever this call already placed. Returns the data files that already existed in
/// `target` (kept untouched; empty when files were copied).
fn copy_data_files(from: &Path, target: &Path) -> Result<Vec<String>, CommandError> {
    let present = |dir: &Path, name: &str| dir.join(name).try_exists().unwrap_or(true);
    let names = [CONNECTIONS_FILE, VAULT_FILE];
    let existing: Vec<String> = names
        .iter()
        .filter(|n| present(target, n))
        .map(|n| (*n).to_string())
        .collect();
    if !existing.is_empty() {
        return Ok(existing);
    }
    let mut placed: Vec<PathBuf> = Vec::new();
    for name in names {
        let source = from.join(name);
        if !source.try_exists().unwrap_or(false) {
            continue;
        }
        let dest = target.join(name);
        if let Err(e) = copy_atomic(&source, &dest) {
            for p in placed {
                let _ = fs::remove_file(p);
            }
            return Err(error(
                "copy_failed",
                format!("could not copy {name} to {}: {e}", target.display()),
            ));
        }
        placed.push(dest);
    }
    Ok(Vec::new())
}

/// Canonicalises `p` even when its tail does not exist yet: the longest existing ancestor is
/// canonicalised (so Windows verbatim prefixes and letter case match a canonicalised path) and
/// the remaining components are appended, with `.` dropped and `..` applied lexically.
fn canonical_lenient(p: &Path) -> PathBuf {
    use std::path::Component;
    for ancestor in p.ancestors() {
        let Ok(mut resolved) = fs::canonicalize(ancestor) else {
            continue;
        };
        let tail = p.strip_prefix(ancestor).unwrap_or(Path::new(""));
        for component in tail.components() {
            match component {
                Component::Normal(name) => resolved.push(name),
                Component::ParentDir => {
                    resolved.pop();
                }
                _ => {}
            }
        }
        return resolved;
    }
    p.to_path_buf()
}

/// Moves the data folder: copies the data files (only into a folder that has neither of them),
/// writes `settings.json` and flags `restart_required`. The running app keeps using the old
/// folder. Choosing again while a restart is pending is allowed (the last choice wins);
/// choosing the running folder again then cancels the pending change.
pub fn apply_data_folder(state: &AppState, target: &Path) -> Result<DataFolderInfo, CommandError> {
    if state.source == DataDirSource::Env {
        return Err(error(
            "data_dir_locked_by_env",
            "the data folder is set by the FERNJO_DATA_DIR environment variable; unset it to choose a folder here",
        ));
    }
    if state.settings_unusable {
        return Err(error(
            "settings_unusable",
            "settings.json cannot be used (it was written by a newer version of fernjo or is unreadable), so the data folder cannot be changed here",
        ));
    }
    let config_dir = state.config_dir.as_ref().ok_or_else(|| {
        error(
            "no_config_dir",
            "no settings folder is available on this system; set FERNJO_DATA_DIR instead",
        )
    })?;
    if !target.is_absolute() || !target.is_dir() {
        return Err(not_usable(target, "it is not an existing folder"));
    }
    check_writable(target).map_err(|e| not_usable(target, e))?;
    let canonical_target = fs::canonicalize(target).map_err(|e| not_usable(target, e))?;
    // The device folder holds state that must never sync, so no data folder may be, hold or sit
    // inside it. A device folder that does not exist yet is compared as spelled.
    if let Some(device) = &state.device_dir {
        let device = canonical_lenient(device);
        if canonical_target.starts_with(&device) || device.starts_with(&canonical_target) {
            return Err(error(
                "folder_device_dir",
                "this folder cannot be used: it is, contains or lies inside the folder where fernjo keeps state that must stay on this device",
            ));
        }
    }
    let current = state
        .data_dir
        .as_ref()
        .and_then(|d| fs::canonicalize(d).ok());
    let pending = state.restart_required.load(Ordering::SeqCst);
    let mut cancel_pending = false;
    let mut existing_files = Vec::new();
    if let Some(current) = &current {
        if *current == canonical_target {
            if !pending {
                return info(state);
            }
            cancel_pending = true;
        } else if canonical_target.starts_with(current) || current.starts_with(&canonical_target) {
            return Err(error(
                "folder_nested",
                "the new data folder must not be inside the current one, nor the current one inside it",
            ));
        }
        if !cancel_pending {
            existing_files = copy_data_files(current, &canonical_target)?;
        }
    }

    // Settings last. Re-read them so no other field is lost; never replace a newer schema.
    let path = config_dir.join(settings::FILE_NAME);
    let mut current_settings: AppSettings = settings::load(&path).map_err(|e| {
        error(
            "settings_unusable",
            format!("settings.json cannot be used: {e}"),
        )
    })?;
    current_settings.data_dir = Some(target.to_path_buf());
    settings::save(&path, &current_settings).map_err(|e| {
        error(
            "settings_write_failed",
            format!("could not save the data folder choice: {e}"),
        )
    })?;

    *state
        .pending_data_dir
        .lock()
        .map_err(|_| CommandError::internal())? =
        (!cancel_pending).then(|| (target.to_path_buf(), existing_files));
    state
        .restart_required
        .store(!cancel_pending, Ordering::SeqCst);
    info(state)
}

/// The `#[tauri::command]` wrappers registered in `run()`.
pub mod handlers {
    use super::*;
    use tauri::{AppHandle, State};
    use tauri_plugin_dialog::DialogExt;

    #[tauri::command]
    pub fn get_data_folder(state: State<'_, AppState>) -> Result<DataFolderInfo, CommandError> {
        super::get_data_folder(&state)
    }

    /// Opens the native folder picker (on a worker thread, never on the UI thread or the async
    /// runtime) and applies the choice. `None` when the picker was cancelled.
    #[tauri::command]
    pub async fn choose_data_folder(
        app: AppHandle,
        state: State<'_, AppState>,
    ) -> Result<Option<DataFolderInfo>, CommandError> {
        let picker = app.clone();
        let picked = tauri::async_runtime::spawn_blocking(move || {
            picker.dialog().file().blocking_pick_folder()
        })
        .await
        .map_err(|_| CommandError::internal())?;
        let Some(picked) = picked else {
            return Ok(None);
        };
        let path = picked
            .into_path()
            .map_err(|e| error("folder_not_usable", format!("not a local folder: {e}")))?;
        super::apply_data_folder(&state, &path).map(Some)
    }

    /// Restarts the application. Never returns: the process is replaced.
    #[tauri::command]
    pub fn restart_app(app: AppHandle) {
        app.restart()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands;
    use fernjo_vault::Vault;
    use secrecy::SecretString;
    use std::fs;

    const PW: &str = "a long master pw";

    /// A state whose data folder is `data` and whose settings live in `config`.
    fn state(data: &Path, config: &Path, source: DataDirSource) -> AppState {
        let mut st = AppState::open(Some(data.to_path_buf()));
        st.config_dir = Some(config.to_path_buf());
        st.source = source;
        st
    }

    fn code<T>(r: Result<T, CommandError>) -> String {
        match r {
            Ok(_) => "ok".to_string(),
            Err(e) => e.code,
        }
    }

    fn restart_required(st: &AppState) -> bool {
        st.restart_required.load(Ordering::SeqCst)
    }

    fn settings_path(config: &Path) -> PathBuf {
        config.join(settings::FILE_NAME)
    }

    fn dirs<const N: usize>() -> [tempfile::TempDir; N] {
        std::array::from_fn(|_| tempfile::tempdir().unwrap())
    }

    /// Creates a folder in the tree and a vault, so both data files exist.
    fn fill(st: &AppState) {
        commands::create_folder(st, None, None, "f".into()).unwrap();
        crate::vault_commands::vault_create(st, SecretString::from(PW), false).unwrap();
    }

    #[test]
    fn choosing_an_empty_folder_copies_files_writes_settings_and_sets_restart_required() {
        let [data, config, target] = dirs();
        let st = state(data.path(), config.path(), DataDirSource::Default);
        fill(&st);
        // a backup and a foreign file are not copied
        fs::write(data.path().join("connections.json.bak"), b"bak").unwrap();
        fs::write(data.path().join("settings.json"), b"not mine").unwrap();
        let got = apply_data_folder(&st, target.path()).unwrap();
        assert!(got.restart_required);
        assert_eq!(got.path, target.path().display().to_string());
        assert_eq!(got.source, "default");
        for name in [CONNECTIONS_FILE, VAULT_FILE] {
            assert_eq!(
                fs::read(data.path().join(name)).unwrap(),
                fs::read(target.path().join(name)).unwrap(),
                "{name}"
            );
        }
        // only the two data files, no temp leftovers
        assert_eq!(fs::read_dir(target.path()).unwrap().count(), 2);
        let saved = settings::load(&settings_path(config.path())).unwrap();
        assert_eq!(saved.data_dir.as_deref(), Some(target.path()));
        assert!(restart_required(&st));
        // the vault copy opens with the same password
        let mut copy = Vault::open(&target.path().join(VAULT_FILE))
            .unwrap()
            .unwrap();
        copy.unlock(&SecretString::from(PW)).unwrap();
        // the running state still points at the old folder
        assert_eq!(st.data_dir.as_deref(), Some(data.path()));
    }

    #[test]
    fn existing_files_in_the_target_are_kept_and_not_overwritten() {
        let [data, config, target, target2] = dirs();
        let st = state(data.path(), config.path(), DataDirSource::Default);
        fill(&st);
        // only a connections file exists in the target: nothing is copied, nothing replaced
        fs::write(target.path().join(CONNECTIONS_FILE), b"theirs").unwrap();
        apply_data_folder(&st, target.path()).unwrap();
        assert_eq!(
            fs::read(target.path().join(CONNECTIONS_FILE)).unwrap(),
            b"theirs"
        );
        assert!(!target.path().join(VAULT_FILE).exists());
        assert!(restart_required(&st));
        let saved = settings::load(&settings_path(config.path())).unwrap();
        assert_eq!(saved.data_dir.as_deref(), Some(target.path()));
        // both exist: both kept
        fs::write(target2.path().join(CONNECTIONS_FILE), b"c").unwrap();
        fs::write(target2.path().join(VAULT_FILE), b"v").unwrap();
        apply_data_folder(&st, target2.path()).unwrap();
        assert_eq!(
            fs::read(target2.path().join(CONNECTIONS_FILE)).unwrap(),
            b"c"
        );
        assert_eq!(fs::read(target2.path().join(VAULT_FILE)).unwrap(), b"v");
    }

    #[test]
    fn same_folder_is_a_no_op_without_restart_required() {
        let [data, config] = dirs();
        let st = state(data.path(), config.path(), DataDirSource::Default);
        // also through a differently spelled path
        let spelled = data.path().join(".");
        for target in [data.path().to_path_buf(), spelled] {
            let got = apply_data_folder(&st, &target).unwrap();
            assert!(!got.restart_required);
            assert_eq!(got.path, data.path().display().to_string());
        }
        assert!(!restart_required(&st));
        assert!(!settings_path(config.path()).exists());
    }

    #[test]
    fn choosing_the_running_folder_again_cancels_a_pending_change() {
        let [data, config, target] = dirs();
        let st = state(data.path(), config.path(), DataDirSource::Default);
        apply_data_folder(&st, target.path()).unwrap();
        assert!(restart_required(&st));
        let got = apply_data_folder(&st, data.path()).unwrap();
        assert!(!got.restart_required);
        assert_eq!(got.path, data.path().display().to_string());
        assert!(!restart_required(&st));
        let saved = settings::load(&settings_path(config.path())).unwrap();
        assert_eq!(saved.data_dir.as_deref(), Some(data.path()));
        assert!(commands::create_folder(&st, None, None, "ok".into()).is_ok());
    }

    #[test]
    fn choosing_again_while_a_restart_is_pending_the_last_choice_wins() {
        let [data, config, a, b] = dirs();
        let st = state(data.path(), config.path(), DataDirSource::Default);
        apply_data_folder(&st, a.path()).unwrap();
        let got = apply_data_folder(&st, b.path()).unwrap();
        assert_eq!(got.path, b.path().display().to_string());
        assert_eq!(
            get_data_folder(&st).unwrap().path,
            b.path().display().to_string()
        );
        let saved = settings::load(&settings_path(config.path())).unwrap();
        assert_eq!(saved.data_dir.as_deref(), Some(b.path()));
    }

    #[test]
    fn file_instead_of_folder_is_folder_not_usable() {
        let [data, config] = dirs();
        let st = state(data.path(), config.path(), DataDirSource::Default);
        let file = data.path().join("a-file");
        fs::write(&file, b"x").unwrap();
        assert_eq!(code(apply_data_folder(&st, &file)), "folder_not_usable");
        // a missing folder and a relative path are refused as well
        let missing = config.path().join("missing");
        assert_eq!(code(apply_data_folder(&st, &missing)), "folder_not_usable");
        assert_eq!(
            code(apply_data_folder(&st, Path::new("relative"))),
            "folder_not_usable"
        );
        assert!(!restart_required(&st));
        assert!(!settings_path(config.path()).exists());
    }

    /// A folder that refuses a write. The Windows read-only attribute of a directory does not
    /// stop file creation, so this uses a folder *inside a file* (unusable on every OS) and,
    /// on Unix, a directory without write permission.
    #[test]
    fn read_only_folder_is_folder_not_usable() {
        let [data, config] = dirs();
        let st = state(data.path(), config.path(), DataDirSource::Default);
        let file = data.path().join("a-file");
        fs::write(&file, b"x").unwrap();
        assert_eq!(
            code(apply_data_folder(&st, &file.join("inside"))),
            "folder_not_usable"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let ro = tempfile::tempdir().unwrap();
            fs::set_permissions(ro.path(), fs::Permissions::from_mode(0o555)).unwrap();
            // root ignores the permission bits; only assert where the OS honours them
            if fs::File::create(ro.path().join("probe")).is_err() {
                assert_eq!(code(apply_data_folder(&st, ro.path())), "folder_not_usable");
            }
            fs::set_permissions(ro.path(), fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert!(!restart_required(&st));
        assert!(!settings_path(config.path()).exists());
    }

    #[test]
    fn nested_folders_are_refused() {
        let [data, config] = dirs();
        let st = state(data.path(), config.path(), DataDirSource::Default);
        let inside = data.path().join("sub");
        fs::create_dir(&inside).unwrap();
        assert_eq!(code(apply_data_folder(&st, &inside)), "folder_nested");
        // the current folder inside the target
        let parent = data.path().parent().unwrap();
        assert_eq!(code(apply_data_folder(&st, parent)), "folder_nested");
        assert!(!restart_required(&st));
        assert!(!settings_path(config.path()).exists());
        assert_eq!(fs::read_dir(&inside).unwrap().count(), 0);
    }

    #[test]
    fn the_device_folder_is_refused_when_equal_parent_or_child() {
        let [data, config, local] = dirs();
        let device = local.path().join("fernjo-device");
        fs::create_dir(&device).unwrap();
        let child = device.join("sub");
        fs::create_dir(&child).unwrap();
        let mut st = state(data.path(), config.path(), DataDirSource::Default);
        st.device_dir = Some(device.clone());
        for target in [device.as_path(), local.path(), child.as_path()] {
            assert_eq!(
                code(apply_data_folder(&st, target)),
                "folder_device_dir",
                "{}",
                target.display()
            );
        }
        // a spelling through `..` does not get around it
        let sneaky = child.join("..");
        assert_eq!(code(apply_data_folder(&st, &sneaky)), "folder_device_dir");
        assert!(!restart_required(&st));
        assert!(!settings_path(config.path()).exists());
        assert_eq!(fs::read_dir(&child).unwrap().count(), 0);
        // an unrelated folder is still fine; a device folder that does not exist yet is too
        let other = tempfile::tempdir().unwrap();
        assert!(apply_data_folder(&st, other.path()).is_ok());
        st.device_dir = Some(local.path().join("not-created-yet"));
        assert!(apply_data_folder(&st, data.path()).is_ok());
    }

    #[test]
    fn a_device_folder_that_does_not_exist_yet_still_blocks_its_parent() {
        let [data, config, local] = dirs();
        let device = local.path().join("fernjo-device");
        assert!(!device.exists());
        let mut st = state(data.path(), config.path(), DataDirSource::Default);
        st.device_dir = Some(device.clone());
        // the parent (canonicalised to a verbatim path on Windows), also with `..` and a
        // trailing separator
        let dotted = local.path().join("x").join("..");
        fs::create_dir(local.path().join("x")).unwrap();
        for target in [local.path().to_path_buf(), dotted] {
            assert_eq!(
                code(apply_data_folder(&st, &target)),
                "folder_device_dir",
                "{}",
                target.display()
            );
        }
        let mut trailing = local.path().as_os_str().to_owned();
        trailing.push(std::path::MAIN_SEPARATOR_STR);
        assert_eq!(
            code(apply_data_folder(&st, Path::new(&trailing))),
            "folder_device_dir"
        );
        // a device folder spelled with `..` that does not exist is resolved too
        st.device_dir = Some(local.path().join("gone").join("..").join("fernjo-device"));
        assert_eq!(
            code(apply_data_folder(&st, local.path())),
            "folder_device_dir"
        );
        // an unrelated sibling is allowed
        let sibling = local.path().join("sibling");
        fs::create_dir(&sibling).unwrap();
        st.device_dir = Some(device);
        assert!(apply_data_folder(&st, &sibling).is_ok());
        assert!(restart_required(&st));
    }

    #[test]
    fn canonical_lenient_resolves_the_existing_part_and_keeps_the_rest() {
        let base = tempfile::tempdir().unwrap();
        let real = fs::canonicalize(base.path()).unwrap();
        let missing = base.path().join("a").join("..").join("b").join("c");
        assert_eq!(canonical_lenient(&missing), real.join("b").join("c"));
        assert_eq!(canonical_lenient(base.path()), real);
    }

    #[test]
    fn env_override_refuses_with_data_dir_locked_by_env() {
        let [data, config, target] = dirs();
        let st = state(data.path(), config.path(), DataDirSource::Env);
        assert_eq!(
            code(apply_data_folder(&st, target.path())),
            "data_dir_locked_by_env"
        );
        assert!(!restart_required(&st));
        assert!(!settings_path(config.path()).exists());
        assert_eq!(fs::read_dir(target.path()).unwrap().count(), 0);
    }

    #[test]
    fn unusable_settings_or_no_config_dir_refuse() {
        let [data, config, target] = dirs();
        let mut st = state(data.path(), config.path(), DataDirSource::Default);
        st.settings_unusable = true;
        assert_eq!(
            code(apply_data_folder(&st, target.path())),
            "settings_unusable"
        );
        assert!(!get_data_folder(&st).unwrap().changeable);
        let st = AppState::open(Some(data.path().to_path_buf()));
        assert_eq!(code(apply_data_folder(&st, target.path())), "no_config_dir");
        assert!(!get_data_folder(&st).unwrap().changeable);
        assert_eq!(fs::read_dir(target.path()).unwrap().count(), 0);
    }

    #[test]
    fn failed_settings_write_leaves_restart_required_unset() {
        let [data, config, target] = dirs();
        // settings.json is absent (so it still reads as "defaults"), but a directory sits where
        // the atomic write wants its temp file, so the write fails on every OS
        fs::create_dir(config.path().join("settings.json.tmp")).unwrap();
        let st = state(data.path(), config.path(), DataDirSource::Default);
        fill(&st);
        let err = apply_data_folder(&st, target.path()).unwrap_err();
        assert_eq!(err.code, "settings_write_failed");
        assert!(!restart_required(&st));
        assert!(!get_data_folder(&st).unwrap().restart_required);
        assert!(commands::create_folder(&st, None, None, "ok".into()).is_ok());
    }

    #[test]
    fn after_success_mutating_commands_return_restart_required() {
        let [data, config, target] = dirs();
        let st = state(data.path(), config.path(), DataDirSource::Default);
        apply_data_folder(&st, target.path()).unwrap();
        assert_eq!(
            code(commands::create_folder(&st, None, None, "x".into())),
            "restart_required"
        );
        assert_eq!(
            code(crate::vault_commands::vault_create(
                &st,
                SecretString::from(PW),
                false
            )),
            "restart_required"
        );
        assert!(commands::list_tree(&st).is_ok());
    }

    #[test]
    fn get_data_folder_reports_source_and_changeable() {
        let [data, config] = dirs();
        for (source, name, can) in [
            (DataDirSource::Env, "env", false),
            (DataDirSource::Settings, "settings", true),
            (DataDirSource::Default, "default", true),
        ] {
            let st = state(data.path(), config.path(), source);
            let got = get_data_folder(&st).unwrap();
            assert_eq!(got.source, name);
            assert_eq!(got.changeable, can);
            assert!(!got.restart_required);
            assert_eq!(got.path, data.path().display().to_string());
            let json = serde_json::to_value(&got).unwrap();
            assert_eq!(json["restartRequired"], false);
            assert_eq!(json["changeable"], can);
        }
    }

    #[test]
    fn a_settings_file_that_became_newer_is_never_overwritten() {
        let [data, config, target] = dirs();
        let st = state(data.path(), config.path(), DataDirSource::Default);
        let newer = br#"{"schemaVersion": 99, "dataDir": "x"}"#;
        fs::write(settings_path(config.path()), newer).unwrap();
        assert_eq!(
            code(apply_data_folder(&st, target.path())),
            "settings_unusable"
        );
        assert_eq!(fs::read(settings_path(config.path())).unwrap(), newer);
        assert!(!restart_required(&st));
        assert_eq!(fs::read_dir(target.path()).unwrap().count(), 0);
    }

    #[test]
    fn existing_files_in_the_target_are_reported() {
        let [data, config] = dirs();
        let st = state(data.path(), config.path(), DataDirSource::Default);
        fill(&st);
        let cases: [(&[&str], &[&str]); 3] = [
            (&[VAULT_FILE], &[VAULT_FILE]),
            (&[CONNECTIONS_FILE], &[CONNECTIONS_FILE]),
            (
                &[VAULT_FILE, CONNECTIONS_FILE],
                &[CONNECTIONS_FILE, VAULT_FILE],
            ),
        ];
        for (present, expected) in cases {
            let target = tempfile::tempdir().unwrap();
            for name in present {
                fs::write(target.path().join(name), b"theirs").unwrap();
            }
            let got = apply_data_folder(&st, target.path()).unwrap();
            assert_eq!(got.existing_files, expected, "{present:?}");
            // a pending change keeps reporting it
            assert_eq!(get_data_folder(&st).unwrap().existing_files, expected);
            let json = serde_json::to_value(&got).unwrap();
            assert_eq!(json["existingFiles"], serde_json::json!(expected));
        }
        // an empty target: files are copied, nothing is adopted
        let empty = tempfile::tempdir().unwrap();
        let got = apply_data_folder(&st, empty.path()).unwrap();
        assert!(got.existing_files.is_empty());
        assert!(get_data_folder(&st).unwrap().existing_files.is_empty());
        // the same folder is a no-op and reports nothing; so does a cancelled change
        let got = apply_data_folder(&st, data.path()).unwrap();
        assert!(got.existing_files.is_empty());
        assert!(get_data_folder(&st).unwrap().existing_files.is_empty());
        let [fresh_data, fresh_config] = dirs();
        let fresh = state(
            fresh_data.path(),
            fresh_config.path(),
            DataDirSource::Default,
        );
        let got = apply_data_folder(&fresh, fresh_data.path()).unwrap();
        assert!(got.existing_files.is_empty());
    }

    /// The second copy fails after the first succeeded: the first is rolled back, nothing else
    /// in the target is touched, no settings are written and the restart flag stays unset.
    #[test]
    fn a_failing_copy_rolls_back_the_files_placed_earlier() {
        let [data, config, target] = dirs();
        let st = state(data.path(), config.path(), DataDirSource::Default);
        fill(&st);
        let before: Vec<Vec<u8>> = [CONNECTIONS_FILE, VAULT_FILE]
            .iter()
            .map(|n| fs::read(data.path().join(n)).unwrap())
            .collect();
        fs::write(target.path().join("notes.txt"), b"keep me").unwrap();
        // a directory where the temp file of the vault copy goes makes that copy fail
        let blocker = target.path().join(format!("{VAULT_FILE}.fernjo-copy.tmp"));
        fs::create_dir(&blocker).unwrap();
        let err = apply_data_folder(&st, target.path()).unwrap_err();
        assert_eq!(err.code, "copy_failed");
        assert!(!target.path().join(CONNECTIONS_FILE).exists());
        assert!(!target.path().join(VAULT_FILE).exists());
        assert_eq!(
            fs::read(target.path().join("notes.txt")).unwrap(),
            b"keep me"
        );
        assert!(blocker.is_dir());
        assert_eq!(fs::read_dir(target.path()).unwrap().count(), 2);
        assert!(!settings_path(config.path()).exists());
        assert!(!restart_required(&st));
        assert!(!get_data_folder(&st).unwrap().restart_required);
        let after: Vec<Vec<u8>> = [CONNECTIONS_FILE, VAULT_FILE]
            .iter()
            .map(|n| fs::read(data.path().join(n)).unwrap())
            .collect();
        assert_eq!(before, after);
    }
}
