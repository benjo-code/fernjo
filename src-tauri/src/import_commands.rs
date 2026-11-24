//! Import, restore and export as plain functions over `AppState` plus the thin
//! `#[tauri::command]` wrappers in [`handlers`] (which also own the native file dialogs).
//!
//! One file at a time waits in `AppState.pending`: `import_begin` remembers the picked path,
//! `import_inspect` reads it and replaces the path by a plan (or a decrypted backup), and
//! `import_apply` or `import_cancel` ends it. A wrong key or password leaves the picked file
//! in place so the UI can ask again. Secrets typed by the user become `SecretString` on
//! arrival and no response type carries one: previews and results hold counts, names, the
//! report, the vault status and the tree snapshot.
//!
//! Lock order is the one of `delete_credential`: pending, then vault, then store.

use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{MutexGuard, PoisonError};

use fernjo_core::{Node, NodeKind, write_exact};
use fernjo_import::backup::{self, BackupContents, BackupError};
use fernjo_import::connections::{self, ConnectionsError, ConnectionsPlan};
use fernjo_import::error::ImportError;
use fernjo_import::mremoteng::{self, ImportPlan, PlannedEntry};
use fernjo_import::report::Report;
use fernjo_vault::{EntryId, FILE_NAME as VAULT_FILE_NAME, NewEntry, Vault};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::commands::{CommandError, Snapshot, ensure_writable, snapshot, with_store};
use crate::state::{AppState, VaultStatus as State};
use crate::vault_commands::{VaultStatus, vault_status, with_vault};

/// Largest `confCons.xml` that is read.
const MAX_MREMOTENG_BYTES: u64 = 32 * 1024 * 1024;
/// Largest connections export or backup that is read.
const MAX_OTHER_BYTES: u64 = 64 * 1024 * 1024;

/// Which kind of file the user picked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImportKind {
    Mremoteng,
    Connections,
    Backup,
}

/// What waits for `import_apply`. `Debug` is written by hand and shows no secret and no path.
pub enum Pending {
    /// A file was picked but not (successfully) inspected yet.
    Picked {
        kind: ImportKind,
        path: PathBuf,
    },
    MRemoteNg(ImportPlan),
    Connections(ConnectionsPlan),
    Backup(BackupContents),
}

impl std::fmt::Debug for Pending {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Picked { kind, .. } => write!(f, "Pending::Picked({kind:?})"),
            Self::MRemoteNg(p) => write!(f, "Pending::MRemoteNg({} entries)", p.entries.len()),
            Self::Connections(_) => f.write_str("Pending::Connections"),
            Self::Backup(c) => write!(f, "Pending::Backup({c:?})"),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PickedFile {
    pub file_name: String,
}

/// What a backup file holds, for the confirmation of a restore.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupSummary {
    pub created_at: String,
    pub app: String,
    pub folders: usize,
    pub connections: usize,
    pub credentials: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub kind: ImportKind,
    pub file_name: String,
    /// mRemoteNG and connections imports.
    pub report: Option<Report>,
    /// Restores.
    pub backup: Option<BackupSummary>,
    /// Applying needs an unlocked vault (planned entries / backup entries).
    pub needs_vault: bool,
    pub vault: VaultStatus,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ApplyOptions {
    pub without_passwords: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreSummary {
    pub created_at: String,
    pub folders: usize,
    pub connections: usize,
    pub credentials: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResult {
    pub snapshot: Snapshot,
    pub report: Option<Report>,
    pub restored: Option<RestoreSummary>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportInfo {
    pub file_name: String,
    /// Credential references left out of a connections export (always 0 for a backup).
    pub dropped_credentials: usize,
}

fn error(code: &str, message: impl Into<String>) -> CommandError {
    CommandError {
        code: code.to_string(),
        message: message.into(),
    }
}

impl From<ImportError> for CommandError {
    fn from(e: ImportError) -> Self {
        error(e.code(), e.to_string())
    }
}

impl From<ConnectionsError> for CommandError {
    fn from(e: ConnectionsError) -> Self {
        error(e.code(), e.to_string())
    }
}

impl From<BackupError> for CommandError {
    fn from(e: BackupError) -> Self {
        error(e.code(), e.to_string())
    }
}

// ---- date and time ------------------------------------------------------------------------

/// Today's local date as `YYYY-MM-DD`: the date in the import folder names and in the export
/// file names.
pub(crate) fn today_local() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// The current time as RFC 3339 in UTC with a `Z` (`exportedAt`, `createdAt`).
fn now_utc_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

// ---- file dialogs -------------------------------------------------------------------------

/// The path a debug build takes from `FERNJO_DIALOG_FILE` instead of showing a dialog (empty
/// means unset). Kept separate from the environment read so both cases are testable.
#[cfg(any(debug_assertions, test))]
fn dialog_override_from(value: Option<std::ffi::OsString>) -> Option<PathBuf> {
    value.filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// Debug builds only: lets a driven session (DevTools port) pick or save a file without OS
/// input. Release builds compile this to `None`; the environment is not read there.
#[cfg(debug_assertions)]
fn dialog_override() -> Option<PathBuf> {
    dialog_override_from(std::env::var_os("FERNJO_DIALOG_FILE"))
}

#[cfg(not(debug_assertions))]
fn dialog_override() -> Option<PathBuf> {
    None
}

/// Dialog filter name and file extensions per kind of file.
fn filter_of(kind: ImportKind) -> (&'static str, &'static [&'static str]) {
    match kind {
        ImportKind::Mremoteng => ("mRemoteNG connections (confCons.xml)", &["xml"]),
        ImportKind::Connections => ("fernjo connections", &["json"]),
        ImportKind::Backup => ("fernjo backup", &["fjbk"]),
    }
}

/// Default file name of an export.
fn export_file_name(kind: ImportKind, today: &str) -> String {
    match kind {
        ImportKind::Backup => format!("fernjo-backup-{today}.fjbk"),
        _ => format!("fernjo-connections-{today}.json"),
    }
}

// ---- helpers ------------------------------------------------------------------------------

fn file_name_of(path: &Path) -> String {
    path.file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

/// Reads `path` when its size is within `limit`: the size is checked from the metadata first,
/// so a huge file is never loaded, and the read itself is capped in case it grows meanwhile.
fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, CommandError> {
    let unreadable = || error("file_unreadable", "the file could not be read");
    let too_large = || {
        error(
            "too_large",
            format!("the file is larger than {} MiB", limit / (1024 * 1024)),
        )
    };
    let meta = fs::metadata(path).map_err(|_| unreadable())?;
    if !meta.is_file() {
        return Err(unreadable());
    }
    if meta.len() > limit {
        return Err(too_large());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .and_then(|f| f.take(limit + 1).read_to_end(&mut bytes))
        .map_err(|_| unreadable())?;
    if bytes.len() as u64 > limit {
        return Err(too_large());
    }
    Ok(bytes)
}

/// (folders, connections) below and including `nodes`.
fn count_nodes(nodes: &[Node]) -> (usize, usize) {
    let mut folders = 0;
    let mut connections = 0;
    for node in nodes {
        match &node.kind {
            NodeKind::Folder { children } => {
                let (f, c) = count_nodes(children);
                folders += 1 + f;
                connections += c;
            }
            NodeKind::Connection { .. } => connections += 1,
        }
    }
    (folders, connections)
}

/// Removes every credential reference below `node` (itself included) and counts them.
fn strip_references(node: &mut Node) -> usize {
    let mut n = usize::from(node.settings.credential.take().is_some());
    if let NodeKind::Folder { children } = &mut node.kind {
        n += children.iter_mut().map(strip_references).sum::<usize>();
    }
    n
}

/// A backup must be sound before anything is touched: repeated entry ids or an entry without a
/// name would make `replace_entries` fail after sessions were closed. Credential references in
/// the tree that point to no entry are allowed (they restore as unresolvable credentials).
fn validate_backup(contents: &BackupContents) -> Result<(), CommandError> {
    let mut seen = HashSet::new();
    for (id, entry) in &contents.entries {
        if !seen.insert(*id) {
            return Err(error(
                "damaged",
                "the backup is damaged: a credential appears twice",
            ));
        }
        if entry.name.trim().is_empty() {
            return Err(error(
                "damaged",
                "the backup is damaged: a credential has no name",
            ));
        }
    }
    Ok(())
}

fn summary_of(contents: &BackupContents) -> BackupSummary {
    let (folders, connections) = count_nodes(contents.tree.children());
    BackupSummary {
        created_at: contents.created_at.clone(),
        app: contents.app.clone(),
        folders,
        connections,
        credentials: contents.entries.len(),
    }
}

/// The vault must be usable and unlocked for `what`.
fn require_unlocked_vault(state: &AppState, what: &str) -> Result<(), CommandError> {
    match state.vault_status()? {
        State::Unlocked => Ok(()),
        State::None => Err(error(
            "vault_needed",
            format!("{what} needs the vault, and there is no vault yet: create one first"),
        )),
        State::Locked => Err(error(
            "vault_needed",
            format!("{what} needs the vault, and the vault is locked: unlock it first"),
        )),
        State::Unusable { reason, path } => Err(error(
            "vault_unusable",
            format!("the vault file {} cannot be used: {reason}", path.display()),
        )),
    }
}

// ---- pick, inspect, cancel ----------------------------------------------------------------

/// The pending slot. A poisoned lock is recovered: the slot only holds plain values and a
/// panic elsewhere must not make import and cancel unusable.
fn pending_slot(state: &AppState) -> MutexGuard<'_, Option<Pending>> {
    state.pending.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Remembers the picked file and drops whatever was pending.
pub fn import_begin(state: &AppState, kind: ImportKind, path: PathBuf) -> PickedFile {
    let file_name = file_name_of(&path);
    *pending_slot(state) = Some(Pending::Picked { kind, path });
    PickedFile { file_name }
}

/// Drops the pending value (also what every page load does).
pub fn import_cancel(state: &AppState) {
    *pending_slot(state) = None;
}

/// Reads the picked file with the optional typed key (mRemoteNG) or password (backup) and
/// returns what applying it would do. `key_needed`, `wrong_key`, `wrong_password` and
/// `password_needed` leave the picked file in place for a retry; success replaces it by the
/// plan; any other error leaves it as it was.
pub fn import_inspect(state: &AppState, secret: Option<String>) -> Result<Preview, CommandError> {
    inspect_on(state, secret, &today_local())
}

/// [`import_inspect`] with the date passed in, so tests do not depend on the clock.
pub(crate) fn inspect_on(
    state: &AppState,
    secret: Option<String>,
    today: &str,
) -> Result<Preview, CommandError> {
    // wrapped at once; `String` -> `SecretString` moves the buffer
    let secret = secret.map(SecretString::from);
    let (kind, path) = match &*pending_slot(state) {
        Some(Pending::Picked { kind, path }) => (*kind, path.clone()),
        Some(_) => {
            return Err(error(
                "already_inspected",
                "the picked file was already inspected",
            ));
        }
        None => return Err(error("no_pending_import", "no file has been picked")),
    };
    let limit = match kind {
        ImportKind::Mremoteng => MAX_MREMOTENG_BYTES,
        _ => MAX_OTHER_BYTES,
    };
    let bytes = read_bounded(&path, limit)?;
    let taken: Vec<String> = with_store(state, |s| {
        Ok(s.tree().children().iter().map(|n| n.name.clone()).collect())
    })?;

    let (next, report, backup) = match kind {
        ImportKind::Mremoteng => {
            let key = secret.filter(|k| !secret_is_empty(k));
            let doc = mremoteng::read(&bytes, key.as_ref())?;
            let plan = mremoteng::plan(&doc, today, &taken);
            let report = plan.report.clone();
            (Pending::MRemoteNg(plan), Some(report), None)
        }
        ImportKind::Connections => {
            let plan = connections::plan(&bytes, today, &taken)?;
            let report = plan.report.clone();
            (Pending::Connections(plan), Some(report), None)
        }
        ImportKind::Backup => {
            let Some(password) = secret else {
                return Err(error("password_needed", "enter the backup password"));
            };
            let contents = backup::open(&bytes, &password)?;
            validate_backup(&contents)?;
            let summary = summary_of(&contents);
            (Pending::Backup(contents), None, Some(summary))
        }
    };
    let vault = vault_status(state)?;
    // Must agree with `import_apply`: a restore needs the vault whenever one exists (it is
    // replaced as a whole) or the backup holds entries.
    let needs_vault = match &next {
        Pending::MRemoteNg(p) => !p.entries.is_empty(),
        Pending::Backup(c) => !c.entries.is_empty() || vault.state != "none",
        _ => false,
    };
    install(state, kind, &path, next)?;
    Ok(Preview {
        kind,
        file_name: file_name_of(&path),
        report,
        backup,
        needs_vault,
        vault,
    })
}

/// Replaces the picked file by its inspected form, but only when the user has not picked
/// something else or cancelled meanwhile (same kind and path).
fn install(
    state: &AppState,
    kind: ImportKind,
    path: &Path,
    next: Pending,
) -> Result<(), CommandError> {
    let mut slot = pending_slot(state);
    match &*slot {
        Some(Pending::Picked {
            kind: now_kind,
            path: now_path,
        }) if *now_kind == kind && now_path == path => {
            *slot = Some(next);
            Ok(())
        }
        _ => Err(error("no_pending_import", "the picked file was replaced")),
    }
}

fn secret_is_empty(s: &SecretString) -> bool {
    use secrecy::ExposeSecret;
    s.expose_secret().is_empty()
}

// ---- apply --------------------------------------------------------------------------------

/// Applies the pending import or restore. The pending value is cleared only on success (and
/// when a backup turns out to be damaged, since it can never be applied).
pub fn import_apply(state: &AppState, options: ApplyOptions) -> Result<ApplyResult, CommandError> {
    ensure_writable(state)?;
    let mut slot = pending_slot(state);
    let result = match &*slot {
        None => Err(error("no_pending_import", "there is nothing to apply")),
        Some(Pending::Picked { .. }) => Err(error(
            "not_inspected",
            "the picked file has not been inspected yet",
        )),
        Some(Pending::MRemoteNg(plan)) => apply_mremoteng(state, plan, options.without_passwords),
        Some(Pending::Connections(plan)) => apply_connections(state, plan),
        Some(Pending::Backup(contents)) => match validate_backup(contents) {
            Ok(()) => apply_backup(state, contents),
            Err(e) => {
                // never applicable: drop it
                *slot = None;
                return Err(e);
            }
        },
    };
    if result.is_ok() {
        *slot = None;
    }
    // also a failed apply may have written part of it
    state.note_local_edit();
    result
}

fn apply_connections(
    state: &AppState,
    plan: &ConnectionsPlan,
) -> Result<ApplyResult, CommandError> {
    let snap = with_store(state, |store| {
        store.graft(None, plan.folder.clone())?;
        Ok(snapshot(store, Vec::new()))
    })?;
    Ok(ApplyResult {
        snapshot: snap,
        report: Some(plan.report.clone()),
        restored: None,
    })
}

fn apply_mremoteng(
    state: &AppState,
    plan: &ImportPlan,
    without_passwords: bool,
) -> Result<ApplyResult, CommandError> {
    let mut folder = plan.folder.clone();
    // The report of the chosen variant was worked out with the plan (before the line caps), so
    // it has no line that is untrue without the credentials.
    let (report, entries): (Report, &[PlannedEntry]) = if without_passwords {
        strip_references(&mut folder);
        (plan.report_without_passwords.clone(), &[])
    } else {
        (plan.report.clone(), &plan.entries)
    };

    let snap = if entries.is_empty() {
        with_store(state, |store| {
            store.graft(None, folder)?;
            Ok(snapshot(store, Vec::new()))
        })?
    } else {
        require_unlocked_vault(state, "this import")?;
        with_vault(state, |v| {
            add_then_graft(v, entries, |_| {
                with_store(state, |store| {
                    store.graft(None, folder)?;
                    Ok(snapshot(store, Vec::new()))
                })
            })
        })?
    };
    Ok(ApplyResult {
        snapshot: snap,
        report: Some(report),
        restored: None,
    })
}

/// Adds all planned entries in one vault save (so `vault.fernjo.bak` afterwards holds the vault
/// as it was before the import), then runs `graft`. A refused batch changes nothing. When the
/// graft fails, the entries are removed again in one more save; if that fails too, the error is
/// `import_incomplete`, naming how many entries could not be removed.
fn add_then_graft(
    vault: &mut Vault,
    entries: &[PlannedEntry],
    graft: impl FnOnce(&mut Vault) -> Result<Snapshot, CommandError>,
) -> Result<Snapshot, CommandError> {
    let ids: Vec<EntryId> = entries.iter().map(|p| p.id).collect();
    vault.add_entries_with_ids(
        entries
            .iter()
            .map(|p| (p.id, p.entry.clone()))
            .collect::<Vec<_>>(),
    )?;
    let failure = match graft(vault) {
        Ok(snapshot) => return Ok(snapshot),
        Err(e) => e,
    };
    if vault.remove_entries(&ids).is_ok() {
        return Err(failure);
    }
    let stuck = ids.len();
    let s = if stuck == 1 { "y" } else { "ies" };
    Err(error(
        "import_incomplete",
        format!(
            "the import failed ({}) and {stuck} imported credential entr{s} could not be removed again; remove them in the vault",
            failure.message
        ),
    ))
}

/// Puts the backup's entries in the vault, then runs `tree_step`. When that fails, the
/// previous entries are put back; if that fails too the error is `restore_incomplete`: the
/// vault then holds the backup's entries while the tree is the old one. The previous content
/// is in `vault.fernjo.bak` unless the failed rollback save got as far as replacing it.
fn replace_entries_then(
    vault: &mut Vault,
    new: Vec<(EntryId, NewEntry)>,
    previous: Vec<(EntryId, NewEntry)>,
    tree_step: impl FnOnce() -> Result<Snapshot, CommandError>,
) -> Result<Snapshot, CommandError> {
    vault.replace_entries(new)?;
    match tree_step() {
        Ok(snapshot) => Ok(snapshot),
        Err(failure) => match vault.replace_entries(previous) {
            Ok(()) => Err(failure),
            Err(_) => Err(error(
                "restore_incomplete",
                format!(
                    "the restore failed ({}) and the vault could not be put back to its previous state: it may now hold the backup's credentials while the connections are the old ones. Its previous content may still be in {VAULT_FILE_NAME}.bak next to the vault file (look there before changing the vault again)",
                    failure.message
                ),
            )),
        },
    }
}

fn apply_backup(state: &AppState, contents: &BackupContents) -> Result<ApplyResult, CommandError> {
    // A vault that exists is part of "replace everything", so it must be usable and unlocked
    // even when the backup holds no entries; without a vault there is nothing to replace.
    let has_vault = !matches!(state.vault_status()?, State::None);
    if has_vault || !contents.entries.is_empty() {
        require_unlocked_vault(state, "this restore")?;
    }
    // The store must have loaded: otherwise sessions would be closed for nothing.
    with_store(state, |_| Ok(()))?;

    state.sessions.close_all();
    let replace_tree = || {
        with_store(state, |store| {
            store.replace_tree(contents.tree.clone())?;
            Ok(snapshot(store, Vec::new()))
        })
    };
    let snap = if has_vault {
        with_vault(state, |v| {
            let previous = v.export_entries()?;
            replace_entries_then(v, contents.entries.clone(), previous, replace_tree)
        })?
    } else {
        replace_tree()?
    };
    let (folders, connections) = count_nodes(contents.tree.children());
    Ok(ApplyResult {
        snapshot: snap,
        report: None,
        restored: Some(RestoreSummary {
            created_at: contents.created_at.clone(),
            folders,
            connections,
            credentials: contents.entries.len(),
        }),
    })
}

// ---- export -------------------------------------------------------------------------------

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), CommandError> {
    write_exact(path, bytes).map_err(|e| error("save_failed", e.to_string()))
}

/// `path` made absolute with symbolic links and `..` resolved as far as it exists; the part
/// that does not exist yet (an export creates missing folders) is appended lexically.
fn resolved(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_or_else(|_| path.to_path_buf(), |d| d.join(path))
    };
    let mut existing = absolute.clone();
    let mut rest: Vec<OsString> = Vec::new();
    loop {
        if let Ok(mut out) = fs::canonicalize(&existing) {
            for part in rest.into_iter().rev() {
                if part == ".." {
                    out.pop();
                } else if part != "." {
                    out.push(part);
                }
            }
            return out;
        }
        let Some(last) = existing.components().next_back() else {
            return absolute;
        };
        rest.push(last.as_os_str().to_owned());
        if !existing.pop() {
            return absolute;
        }
    }
}

/// An export must never be written into the data folder, the folder of `settings.json`, the
/// device-local folder or a data folder chosen for the next start: it could replace the vault,
/// `connections.json`, `settings.json` or the device marks with something that is not them.
fn ensure_export_target_allowed(state: &AppState, path: &Path) -> Result<(), CommandError> {
    let pending = state
        .pending_data_dir
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .map(|(p, _)| p.clone());
    let guarded = [
        (state.data_dir.as_ref(), "fernjo's data folder"),
        (state.config_dir.as_ref(), "fernjo's settings folder"),
        (state.device_dir.as_ref(), "fernjo's device folder"),
        (
            pending.as_ref(),
            "the data folder chosen for the next start",
        ),
    ];
    let target = resolved(path);
    for (folder, what) in guarded {
        let Some(folder) = folder else { continue };
        if target.starts_with(resolved(folder)) {
            return Err(error(
                "export_target_not_allowed",
                format!(
                    "the file would be inside {what} ({}), where it could overwrite the vault, the connections or the settings; choose another folder",
                    folder.display()
                ),
            ));
        }
    }
    Ok(())
}

/// Writes the connection tree to `path` as a fernjo connections file (no credential
/// references).
pub fn export_connections_to(state: &AppState, path: &Path) -> Result<ExportInfo, CommandError> {
    ensure_export_target_allowed(state, path)?;
    let tree = with_store(state, |s| Ok(s.tree().clone()))?;
    let out = connections::export(&tree, &now_utc_rfc3339());
    write_file(path, &out.bytes)?;
    Ok(ExportInfo {
        file_name: file_name_of(path),
        dropped_credentials: out.dropped_credentials,
    })
}

/// The checks that need neither the target file nor any time: the password rule, a usable and
/// unlocked vault (when one exists) and a loaded store. The command runs them before it opens
/// the save dialog, so nobody picks a path and is then told the vault was locked.
pub fn export_backup_precheck(
    state: &AppState,
    password: &SecretString,
) -> Result<(), CommandError> {
    backup::check_password(password)?;
    match state.vault_status()? {
        State::None | State::Unlocked => {}
        State::Locked => return Err(fernjo_vault::VaultError::VaultLocked.into()),
        State::Unusable { reason, path } => {
            return Err(error(
                "vault_unusable",
                format!("the vault file {} cannot be used: {reason}", path.display()),
            ));
        }
    }
    with_store(state, |_| Ok(()))
}

/// Writes an encrypted backup of the tree and the vault entries to `path`. A vault that
/// exists must be unlocked; without a vault the backup holds the tree only.
pub fn export_backup_to(
    state: &AppState,
    path: &Path,
    password: &SecretString,
) -> Result<ExportInfo, CommandError> {
    ensure_export_target_allowed(state, path)?;
    let vault_guard = state.vault.lock().map_err(|_| CommandError::internal())?;
    let entries = match &*vault_guard {
        crate::state::VaultState::None => Vec::new(),
        crate::state::VaultState::Open(v) => v.export_entries()?,
        crate::state::VaultState::Unusable { reason, path } => {
            return Err(error(
                "vault_unusable",
                format!("the vault file {} cannot be used: {reason}", path.display()),
            ));
        }
    };
    let tree = with_store(state, |s| Ok(s.tree().clone()))?;
    drop(vault_guard);
    let created_at = now_utc_rfc3339();
    let contents = BackupContents {
        created_at,
        app: fernjo_core::version().to_string(),
        tree,
        entries,
    };
    let bytes = backup::create(&contents, password, state.kdf)?;
    write_file(path, &bytes)?;
    Ok(ExportInfo {
        file_name: file_name_of(path),
        dropped_credentials: 0,
    })
}

/// The `#[tauri::command]` wrappers registered in `run()`. The native dialogs run on a worker
/// thread (`spawn_blocking`), never on the main thread or the async runtime. Only the picked
/// file's name goes back to the webview; the path stays here. The commands that derive an
/// Argon2 key (`import_inspect`, `export_backup`) also stay off the main thread.
pub mod handlers {
    use super::*;
    use tauri::{AppHandle, Manager, State};
    use tauri_plugin_dialog::{DialogExt, FilePath};

    async fn blocking<T: Send + 'static>(
        job: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, CommandError> {
        tauri::async_runtime::spawn_blocking(job)
            .await
            .map_err(|_| CommandError::internal())
    }

    fn local_path(picked: FilePath) -> Result<PathBuf, CommandError> {
        picked
            .into_path()
            .map_err(|_| error("file_unreadable", "the chosen file is not a local file"))
    }

    fn pick_blocking(app: &AppHandle, kind: ImportKind) -> Result<Option<PathBuf>, CommandError> {
        if let Some(path) = dialog_override() {
            return Ok(Some(path));
        }
        let (name, extensions) = filter_of(kind);
        app.dialog()
            .file()
            .add_filter(name, extensions)
            .blocking_pick_file()
            .map(local_path)
            .transpose()
    }

    fn save_blocking(app: &AppHandle, kind: ImportKind) -> Result<Option<PathBuf>, CommandError> {
        if let Some(path) = dialog_override() {
            return Ok(Some(path));
        }
        let (name, extensions) = filter_of(kind);
        app.dialog()
            .file()
            .add_filter(name, extensions)
            .set_file_name(export_file_name(kind, &today_local()))
            .blocking_save_file()
            .map(local_path)
            .transpose()
    }

    /// Opens the native file picker and remembers the choice as the pending import. `None`
    /// when the picker was cancelled.
    #[tauri::command]
    pub async fn import_pick(
        app: AppHandle,
        state: State<'_, AppState>,
        kind: ImportKind,
    ) -> Result<Option<PickedFile>, CommandError> {
        let picker = app.clone();
        let picked = blocking(move || pick_blocking(&picker, kind)).await??;
        Ok(picked.map(|path| import_begin(&state, kind, path)))
    }

    #[tauri::command(async)]
    pub fn import_inspect(
        state: State<'_, AppState>,
        secret: Option<String>,
    ) -> Result<Preview, CommandError> {
        super::import_inspect(&state, secret)
    }

    /// Applies the pending import or restore on a worker thread: a big import is real disk work
    /// (a vault save and a tree save) and must not freeze the window. The state is taken from
    /// the app handle inside the worker, since a `State` borrow cannot move into it.
    #[tauri::command]
    pub async fn import_apply(
        app: AppHandle,
        options: ApplyOptions,
    ) -> Result<ApplyResult, CommandError> {
        blocking(move || {
            let state = app.state::<AppState>();
            super::import_apply(&state, options)
        })
        .await?
    }

    #[tauri::command]
    pub fn import_cancel(state: State<'_, AppState>) {
        super::import_cancel(&state);
    }

    /// Asks where to save the connections file and writes it there. `None` when the dialog was
    /// cancelled (nothing is written).
    #[tauri::command]
    pub async fn export_connections(
        app: AppHandle,
        state: State<'_, AppState>,
    ) -> Result<Option<ExportInfo>, CommandError> {
        let picker = app.clone();
        let Some(path) =
            blocking(move || save_blocking(&picker, ImportKind::Connections)).await??
        else {
            return Ok(None);
        };
        super::export_connections_to(&state, &path).map(Some)
    }

    /// Checks the password and the vault, asks where to save the backup, then derives the key
    /// and writes it (all on a worker thread). `None` when the dialog was cancelled (nothing is
    /// written). The password is wrapped at once, before anything else. The state is taken
    /// from the app handle inside the worker, since a `State` borrow cannot move into it.
    #[tauri::command]
    pub async fn export_backup(
        app: AppHandle,
        password: String,
    ) -> Result<Option<ExportInfo>, CommandError> {
        let password = SecretString::from(password);
        blocking(move || {
            let state = app.state::<AppState>();
            super::export_backup_precheck(&state, &password)?;
            let Some(path) = save_blocking(&app, ImportKind::Backup)? else {
                return Ok(None);
            };
            super::export_backup_to(&state, &path, &password).map(Some)
        })
        .await?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands;
    use crate::state::AppState;
    use crate::vault_commands;
    use fernjo_core::{Node, NodeKind};
    use fernjo_import::backup::{self, BackupContents};
    use fernjo_vault::{EntryId, KdfParams, NewEntry, Secret};
    use secrecy::SecretString;
    use serde_json::{Value, json};
    use std::fs;
    use std::path::{Path, PathBuf};

    const TODAY: &str = "2026-01-02";
    const DEFAULT_KEY_XML: &[u8] =
        include_bytes!("../../crates/fernjo-import/tests/fixtures/default-key.xml");
    const CUSTOM_KEY_XML: &[u8] =
        include_bytes!("../../crates/fernjo-import/tests/fixtures/custom-key.xml");
    const CUSTOM_KEY: &str = "fixture-key-ünï";
    const MASTER: &str = "master-password-1";
    const BACKUP_PW: &str = "backup-password-1";
    /// Passwords inside the default-key fixture; none may ever show up in a response.
    const FIXTURE_SECRETS: [&str; 2] = ["folder-pw-1", "vnc-pw"];

    fn st(dir: &Path) -> AppState {
        AppState::open(Some(dir.to_path_buf()))
    }

    fn file(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }

    fn sec(s: &str) -> SecretString {
        SecretString::from(s.to_string())
    }

    fn unlocked(st: &AppState) {
        vault_commands::vault_create(st, sec(MASTER), false).unwrap();
    }

    fn inspect(
        st: &AppState,
        kind: ImportKind,
        path: &Path,
        secret: Option<&str>,
    ) -> Result<Preview, CommandError> {
        import_begin(st, kind, path.to_path_buf());
        inspect_on(st, secret.map(str::to_string), TODAY)
    }

    fn apply(st: &AppState) -> Result<ApplyResult, CommandError> {
        import_apply(
            st,
            ApplyOptions {
                without_passwords: false,
            },
        )
    }

    fn top_names(st: &AppState) -> Vec<String> {
        commands::list_tree(st)
            .unwrap()
            .children
            .into_iter()
            .map(|n| n.name)
            .collect()
    }

    fn credential_count(st: &AppState) -> usize {
        vault_commands::list_credentials(st).unwrap().len()
    }

    fn pending_kind(st: &AppState) -> &'static str {
        match &*st.pending.lock().unwrap() {
            None => "none",
            Some(Pending::Picked { .. }) => "picked",
            Some(Pending::MRemoteNg(_)) => "mremoteng",
            Some(Pending::Connections(_)) => "connections",
            Some(Pending::Backup(_)) => "backup",
        }
    }

    fn references(nodes: &[Node], out: &mut Vec<uuid::Uuid>) {
        for n in nodes {
            out.extend(n.settings.credential);
            if let NodeKind::Folder { children } = &n.kind {
                references(children, out);
            }
        }
    }

    /// Entries as (id, JSON of everything including the secret), sorted by id.
    fn entries_json(st: &AppState) -> Vec<(EntryId, Value)> {
        let mut all: Vec<_> = vault_commands::with_vault(st, |v| Ok(v.export_entries()?))
            .unwrap()
            .into_iter()
            .map(|(id, e)| {
                (
                    id,
                    json!({
                        "name": e.name, "username": e.username, "domain": e.domain,
                        "secret": serde_json::to_value(&e.secret).unwrap(), "notes": e.notes,
                    }),
                )
            })
            .collect();
        all.sort_by_key(|(id, _)| *id);
        all
    }

    fn new_entry(name: &str, pw: &str) -> NewEntry {
        NewEntry {
            name: name.to_string(),
            username: Some("u".to_string()),
            domain: None,
            secret: Secret::Password { password: sec(pw) },
            notes: String::new(),
        }
    }

    /// A source state with the default-key fixture imported (tree + entries), and a backup of
    /// it at `<dir>/b.fjbk`.
    fn source_with_backup(dir: &Path) -> (AppState, PathBuf) {
        let src = st(&dir.join("src"));
        unlocked(&src);
        let xml = file(dir, "c.xml", DEFAULT_KEY_XML);
        inspect(&src, ImportKind::Mremoteng, &xml, None).unwrap();
        apply(&src).unwrap();
        let out = dir.join("b.fjbk");
        export_backup_to(&src, &out, &sec(BACKUP_PW)).unwrap();
        (src, out)
    }

    /// Entries as another run of the app finds them on disk.
    fn entries_on_disk(data: &Path) -> Vec<(EntryId, Value)> {
        let fresh = st(data);
        vault_commands::vault_unlock(&fresh, sec(MASTER), false).unwrap();
        entries_json(&fresh)
    }

    /// Makes the next save of the vault fail with `changed_on_disk`.
    fn spoil_vault_file(data: &Path) {
        fs::write(
            data.join(fernjo_vault::FILE_NAME),
            b"changed by someone else",
        )
        .unwrap();
    }

    fn block_store_save(dir: &Path) {
        // the store keeps the previous file as `connections.json.bak`; a folder of that name
        // fails the save (the file must exist, so callers save once before)
        assert!(dir.join("connections.json").is_file());
        let _ = fs::remove_file(dir.join("connections.json.bak"));
        fs::create_dir(dir.join("connections.json.bak")).unwrap();
    }

    #[test]
    fn dialog_override_reads_the_path_when_set_and_is_none_otherwise() {
        use std::ffi::OsString;
        assert_eq!(dialog_override_from(None), None);
        assert_eq!(dialog_override_from(Some(OsString::new())), None);
        assert_eq!(
            dialog_override_from(Some(OsString::from("/tmp/x.xml"))),
            Some(PathBuf::from("/tmp/x.xml"))
        );
    }

    #[test]
    fn dialog_filters_and_default_names_follow_the_kind() {
        assert_eq!(filter_of(ImportKind::Mremoteng).1, ["xml"]);
        assert_eq!(filter_of(ImportKind::Connections).1, ["json"]);
        assert_eq!(filter_of(ImportKind::Backup).1, ["fjbk"]);
        assert_eq!(
            export_file_name(ImportKind::Connections, "2026-01-02"),
            "fernjo-connections-2026-01-02.json"
        );
        assert_eq!(
            export_file_name(ImportKind::Backup, "2026-01-02"),
            "fernjo-backup-2026-01-02.fjbk"
        );
    }

    #[test]
    fn page_load_clears_pending() {
        let dir = tempfile::tempdir().unwrap();
        let s = st(&dir.path().join("d"));
        let path = file(dir.path(), "c.xml", DEFAULT_KEY_XML);
        inspect(&s, ImportKind::Mremoteng, &path, None).unwrap();
        assert_eq!(pending_kind(&s), "mremoteng");
        s.clear_pending();
        assert_eq!(pending_kind(&s), "none");
        assert_eq!(apply(&s).unwrap_err().code, "no_pending_import");
        // a poisoned lock is recovered as well
        import_begin(&s, ImportKind::Backup, path);
        std::thread::scope(|scope| {
            let _ = scope
                .spawn(|| {
                    let _g = s.pending.lock().unwrap();
                    panic!("poison");
                })
                .join();
        });
        assert!(s.pending.is_poisoned());
        s.clear_pending();
        assert_eq!(pending_kind_poisoned(&s), "none");
    }

    #[test]
    fn today_is_a_local_date_and_timestamps_are_utc_with_z() {
        let d = today_local();
        assert_eq!(d.len(), 10);
        assert!(d.chars().enumerate().all(|(i, c)| if i == 4 || i == 7 {
            c == '-'
        } else {
            c.is_ascii_digit()
        }));
        let t = now_utc_rfc3339();
        assert!(t.ends_with('Z') && t.len() == 20, "{t}");
    }

    #[test]
    fn inspect_default_key_file_returns_report_and_no_secret() {
        let dir = tempfile::tempdir().unwrap();
        let s = st(&dir.path().join("d"));
        let path = file(dir.path(), "confCons.xml", DEFAULT_KEY_XML);
        let begun = import_begin(&s, ImportKind::Mremoteng, path.clone());
        assert_eq!(begun.file_name, "confCons.xml");
        let p = inspect_on(&s, None, TODAY).unwrap();
        assert_eq!(p.kind, ImportKind::Mremoteng);
        assert!(p.needs_vault);
        assert!(p.backup.is_none());
        let report = p.report.as_ref().unwrap();
        assert!(report.counts.credentials > 0);
        assert_eq!(pending_kind(&s), "mremoteng");
        let text = format!("{}{:?}", serde_json::to_string(&p).unwrap(), p);
        for secret in FIXTURE_SECRETS {
            assert!(!text.contains(secret), "{secret} leaked");
        }
        assert!(!format!("{:?}", s.pending.lock().unwrap()).contains("folder-pw-1"));
    }

    #[test]
    fn inspect_custom_key_file_asks_for_the_key_then_accepts_it() {
        let dir = tempfile::tempdir().unwrap();
        let s = st(&dir.path().join("d"));
        let path = file(dir.path(), "c.xml", CUSTOM_KEY_XML);
        import_begin(&s, ImportKind::Mremoteng, path);
        assert_eq!(inspect_on(&s, None, TODAY).unwrap_err().code, "key_needed");
        assert_eq!(pending_kind(&s), "picked");
        let wrong = inspect_on(&s, Some("nope".into()), TODAY).unwrap_err();
        assert_eq!(wrong.code, "wrong_key");
        assert!(!wrong.message.contains("nope"));
        assert_eq!(pending_kind(&s), "picked");
        let p = inspect_on(&s, Some(CUSTOM_KEY.into()), TODAY).unwrap();
        assert!(p.report.is_some());
        assert!(!serde_json::to_string(&p).unwrap().contains(CUSTOM_KEY));
        assert!(!format!("{p:?}{:?}", s.pending.lock().unwrap()).contains(CUSTOM_KEY));
        assert_eq!(pending_kind(&s), "mremoteng");
    }

    #[test]
    fn wrong_kind_of_file_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let (_src, backup_path) = source_with_backup(dir.path());
        let s = st(&dir.path().join("d"));
        let e = inspect(&s, ImportKind::Mremoteng, &backup_path, None).unwrap_err();
        assert_eq!(e.code, "unsupported_format");
        let xml = file(dir.path(), "c.xml", DEFAULT_KEY_XML);
        let e = inspect(&s, ImportKind::Backup, &xml, Some(BACKUP_PW)).unwrap_err();
        assert_eq!(e.code, "not_a_backup");
        let e = inspect(&s, ImportKind::Connections, &xml, None).unwrap_err();
        assert_eq!(e.code, "not_this_format");
    }

    #[test]
    fn unreadable_and_oversized_files_are_typed_errors() {
        let dir = tempfile::tempdir().unwrap();
        let s = st(&dir.path().join("d"));
        let missing = dir.path().join("nothing.xml");
        let e = inspect(&s, ImportKind::Mremoteng, &missing, None).unwrap_err();
        assert_eq!(e.code, "file_unreadable");
        assert_eq!(pending_kind(&s), "picked");
        // a directory is not a file either
        let e = inspect(&s, ImportKind::Mremoteng, dir.path(), None).unwrap_err();
        assert_eq!(e.code, "file_unreadable");
        // sparse file just over the bound is refused from the metadata alone
        let big = dir.path().join("big.xml");
        let f = fs::File::create(&big).unwrap();
        f.set_len(32 * 1024 * 1024 + 1).unwrap();
        let e = inspect(&s, ImportKind::Mremoteng, &big, None).unwrap_err();
        assert_eq!(e.code, "too_large");
        let f = fs::File::create(&big).unwrap();
        f.set_len(64 * 1024 * 1024 + 1).unwrap();
        let e = inspect(&s, ImportKind::Backup, &big, Some(BACKUP_PW)).unwrap_err();
        assert_eq!(e.code, "too_large");
    }

    #[test]
    fn inspect_without_a_picked_file_is_no_pending_import() {
        let dir = tempfile::tempdir().unwrap();
        let s = st(dir.path());
        assert_eq!(
            inspect_on(&s, None, TODAY).unwrap_err().code,
            "no_pending_import"
        );
    }

    #[test]
    fn apply_import_adds_folder_entries_and_returns_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let s = st(&dir.path().join("d"));
        unlocked(&s);
        let path = file(dir.path(), "c.xml", DEFAULT_KEY_XML);
        let p = inspect(&s, ImportKind::Mremoteng, &path, None).unwrap();
        let planned = p.report.unwrap().counts.credentials;
        let r = apply(&s).unwrap();
        assert_eq!(r.snapshot.children.len(), 1);
        assert_eq!(
            r.snapshot.children[0].name,
            "Imported from mRemoteNG (2026-01-02)"
        );
        assert!(r.restored.is_none());
        assert_eq!(r.report.unwrap().counts.credentials, planned);
        assert_eq!(credential_count(&s), planned);
        // every reference in the tree resolves to an entry
        let ids: Vec<_> = entries_json(&s).into_iter().map(|(id, _)| id).collect();
        let mut refs = Vec::new();
        references(&r.snapshot.children, &mut refs);
        assert!(!refs.is_empty() && refs.iter().all(|r| ids.contains(r)));
        assert_eq!(pending_kind(&s), "none");
        // and it is on disk
        let reopened = st(&dir.path().join("d"));
        assert_eq!(top_names(&reopened).len(), 1);
    }

    #[test]
    fn apply_import_without_unlocked_vault_is_vault_needed_and_plan_survives() {
        let dir = tempfile::tempdir().unwrap();
        let s = st(&dir.path().join("d"));
        let path = file(dir.path(), "c.xml", DEFAULT_KEY_XML);
        inspect(&s, ImportKind::Mremoteng, &path, None).unwrap();
        let e = apply(&s).unwrap_err();
        assert_eq!(e.code, "vault_needed");
        assert!(e.message.contains("no vault"), "{}", e.message);
        assert_eq!(pending_kind(&s), "mremoteng");
        assert!(top_names(&s).is_empty());
        unlocked(&s);
        vault_commands::vault_lock(&s).unwrap();
        let e = apply(&s).unwrap_err();
        assert_eq!(e.code, "vault_needed");
        assert!(e.message.contains("locked"), "{}", e.message);
        vault_commands::vault_unlock(&s, sec(MASTER), false).unwrap();
        apply(&s).unwrap();
        assert_eq!(pending_kind(&s), "none");
    }

    #[test]
    fn apply_with_an_unusable_vault_is_vault_unusable() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("d");
        fs::create_dir_all(&data).unwrap();
        fs::write(data.join(fernjo_vault::FILE_NAME), b"garbage").unwrap();
        let s = st(&data);
        let path = file(dir.path(), "c.xml", DEFAULT_KEY_XML);
        inspect(&s, ImportKind::Mremoteng, &path, None).unwrap();
        assert_eq!(apply(&s).unwrap_err().code, "vault_unusable");
        assert_eq!(pending_kind(&s), "mremoteng");
    }

    #[test]
    fn apply_without_passwords_drops_entries_and_references() {
        let dir = tempfile::tempdir().unwrap();
        let s = st(&dir.path().join("d"));
        let path = file(dir.path(), "c.xml", DEFAULT_KEY_XML);
        let p = inspect(&s, ImportKind::Mremoteng, &path, None).unwrap();
        let planned = p.report.unwrap().counts.credentials;
        assert!(planned > 0);
        let mut before = Vec::new();
        match &*s.pending.lock().unwrap() {
            Some(Pending::MRemoteNg(plan)) => {
                references(std::slice::from_ref(&plan.folder), &mut before)
            }
            _ => panic!("plan expected"),
        }
        let r = import_apply(
            &s,
            ApplyOptions {
                without_passwords: true,
            },
        )
        .unwrap();
        let mut after = Vec::new();
        references(&r.snapshot.children, &mut after);
        assert!(after.is_empty());
        let report = r.report.unwrap();
        assert_eq!(report.counts.credentials, 0);
        assert_eq!(report.counts.shared_credentials, 0);
        let left_out = report
            .partial
            .iter()
            .find(|l| l.text.contains("left out"))
            .unwrap();
        assert!(
            left_out.text.contains(&before.len().to_string()),
            "{}",
            left_out.text
        );
        assert!(matches!(
            s.vault_status().unwrap(),
            crate::state::VaultStatus::None
        ));
        assert_eq!(pending_kind(&s), "none");
    }

    #[test]
    fn second_import_same_day_gets_a_numbered_folder() {
        let dir = tempfile::tempdir().unwrap();
        let s = st(&dir.path().join("d"));
        unlocked(&s);
        let path = file(dir.path(), "c.xml", DEFAULT_KEY_XML);
        for _ in 0..2 {
            inspect(&s, ImportKind::Mremoteng, &path, None).unwrap();
            apply(&s).unwrap();
        }
        assert_eq!(
            top_names(&s),
            [
                "Imported from mRemoteNG (2026-01-02)",
                "Imported from mRemoteNG (2026-01-02) (2)"
            ]
        );
    }

    #[test]
    fn import_rolls_entries_back_when_graft_fails() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("d");
        let s = st(&data);
        unlocked(&s);
        let path = file(dir.path(), "c.xml", DEFAULT_KEY_XML);
        // one save so connections.json exists (the block needs a file to back up)
        let tmp = commands::create_folder(&s, None, None, "tmp".into()).unwrap();
        commands::delete_node(&s, tmp.id.unwrap()).unwrap();
        inspect(&s, ImportKind::Mremoteng, &path, None).unwrap();
        block_store_save(&data);
        let e = apply(&s).unwrap_err();
        assert_eq!(e.code, "save_failed");
        assert_eq!(credential_count(&s), 0);
        assert!(top_names(&s).is_empty());
        assert_eq!(pending_kind(&s), "mremoteng");
        // the vault on disk agrees with memory: nothing is left behind
        assert!(entries_on_disk(&data).is_empty());
    }

    fn planned(n: usize) -> Vec<PlannedEntry> {
        (0..n)
            .map(|i| PlannedEntry {
                id: uuid::Uuid::new_v4(),
                entry: new_entry(&format!("e{i}"), "pw"),
            })
            .collect()
    }

    #[test]
    fn entries_exist_at_the_graft_step_and_are_removed_when_it_fails() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("d");
        let s = st(&data);
        unlocked(&s);
        let entries = planned(3);
        let e = vault_commands::with_vault(&s, |v| {
            Ok(add_then_graft(v, &entries, |v| {
                assert_eq!(v.list().unwrap().len(), 3, "added before the graft");
                assert_eq!(entries_on_disk_of(v), 3);
                Err(error("save_failed", "x"))
            })
            .unwrap_err())
        })
        .unwrap();
        assert_eq!(e.code, "save_failed");
        assert_eq!(credential_count(&s), 0);
        assert!(entries_on_disk(&data).is_empty());
    }

    /// Entry count of the vault file as written (a second handle on the same file).
    fn entries_on_disk_of(v: &fernjo_vault::Vault) -> usize {
        let mut other = fernjo_vault::Vault::open(v.path()).unwrap().unwrap();
        other.unlock(&sec(MASTER)).unwrap();
        other.list().unwrap().len()
    }

    #[test]
    fn entries_that_cannot_be_removed_again_are_reported_as_import_incomplete() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("d");
        let s = st(&data);
        unlocked(&s);
        let entries = planned(2);
        let e = vault_commands::with_vault(&s, |v| {
            Ok(add_then_graft(v, &entries, |_| {
                // the graft fails and the vault file changes under us: deletes cannot save
                spoil_vault_file(&data);
                Err(error("save_failed", "tree"))
            })
            .unwrap_err())
        })
        .unwrap();
        assert_eq!(e.code, "import_incomplete");
        assert!(
            e.message.contains("2 imported credential entries"),
            "{}",
            e.message
        );
        assert!(e.message.contains("tree"), "{}", e.message);
    }

    #[test]
    fn a_failed_entry_add_refuses_the_whole_batch() {
        let dir = tempfile::tempdir().unwrap();
        let s = st(&dir.path().join("d"));
        unlocked(&s);
        let mut entries = planned(3);
        entries[2].entry.name = "  ".to_string(); // refused by the vault
        let e = vault_commands::with_vault(&s, |v| {
            Ok(add_then_graft(v, &entries, |_| panic!("not reached")).unwrap_err())
        })
        .unwrap();
        assert_eq!(e.code, "empty_name");
        assert_eq!(credential_count(&s), 0);
    }

    #[test]
    fn restore_reports_restore_incomplete_when_the_vault_cannot_be_put_back() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("d");
        let s = st(&data);
        unlocked(&s);
        vault_commands::with_vault(&s, |v| Ok(v.add(new_entry("old", "x"))?)).unwrap();
        let e = vault_commands::with_vault(&s, |v| {
            let previous = v.export_entries()?;
            Ok(replace_entries_then(v, planned_pairs(2), previous, || {
                spoil_vault_file(&data);
                Err(error("save_failed", "tree"))
            })
            .unwrap_err())
        })
        .unwrap();
        assert_eq!(e.code, "restore_incomplete");
        assert!(e.message.contains("vault.fernjo.bak"), "{}", e.message);
        // the .bak may already hold the backup's entries if the rollback save broke half-way
        assert!(e.message.contains("may still be in"), "{}", e.message);
        assert!(e.message.contains("tree"), "{}", e.message);
    }

    fn planned_pairs(n: usize) -> Vec<(EntryId, NewEntry)> {
        planned(n).into_iter().map(|p| (p.id, p.entry)).collect()
    }

    #[test]
    fn restore_keeps_the_pending_backup_when_the_store_did_not_load() {
        let dir = tempfile::tempdir().unwrap();
        let (_src, backup_path) = source_with_backup(dir.path());
        let data = dir.path().join("dst");
        let dst = st(&data);
        unlocked(&dst);
        vault_commands::with_vault(&dst, |v| Ok(v.add(new_entry("old", "x"))?)).unwrap();
        let before = entries_json(&dst);
        // the store failed to load: nothing may be touched
        fs::write(
            data.join(fernjo_core::FILE_NAME),
            r#"{"schemaVersion":99,"children":[]}"#,
        )
        .unwrap();
        let dst2 = st(&data);
        vault_commands::vault_unlock(&dst2, sec(MASTER), false).unwrap();
        // (inspect already needs the store, so the plan is put in place directly)
        let opened = backup::open(&fs::read(&backup_path).unwrap(), &sec(BACKUP_PW)).unwrap();
        *dst2.pending.lock().unwrap() = Some(Pending::Backup(opened));
        assert_eq!(apply(&dst2).unwrap_err().code, "load_failed");
        assert_eq!(entries_json(&dst2), before);
        assert_eq!(pending_kind(&dst2), "backup");
    }

    #[test]
    fn backup_preview_needs_the_vault_whenever_apply_does() {
        let dir = tempfile::tempdir().unwrap();
        let a = st(&dir.path().join("a"));
        let out = dir.path().join("b.fjbk");
        export_backup_to(&a, &out, &sec(BACKUP_PW)).unwrap(); // no entries
        let none = st(&dir.path().join("none"));
        let p = inspect(&none, ImportKind::Backup, &out, Some(BACKUP_PW)).unwrap();
        assert!(!p.needs_vault);
        assert!(apply(&none).is_ok());
        let with = st(&dir.path().join("with"));
        unlocked(&with);
        let p = inspect(&with, ImportKind::Backup, &out, Some(BACKUP_PW)).unwrap();
        assert!(p.needs_vault);
        vault_commands::vault_lock(&with).unwrap();
        assert_eq!(apply(&with).unwrap_err().code, "vault_needed");
    }

    #[test]
    fn inspecting_twice_is_already_inspected() {
        let dir = tempfile::tempdir().unwrap();
        let s = st(&dir.path().join("d"));
        let path = file(dir.path(), "c.xml", DEFAULT_KEY_XML);
        inspect(&s, ImportKind::Mremoteng, &path, None).unwrap();
        let e = inspect_on(&s, None, TODAY).unwrap_err();
        assert_eq!(e.code, "already_inspected");
        assert_eq!(pending_kind(&s), "mremoteng");
    }

    #[test]
    fn a_picked_file_replaced_meanwhile_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let s = st(&dir.path().join("d"));
        let a = dir.path().join("a.xml");
        let b = dir.path().join("b.xml");
        let plan = || {
            Pending::Connections(ConnectionsPlan {
                folder: Node {
                    id: uuid::Uuid::new_v4(),
                    name: "x".into(),
                    description: String::new(),
                    settings: Default::default(),
                    kind: NodeKind::Folder { children: vec![] },
                },
                report: Report::default(),
            })
        };
        // another path
        import_begin(&s, ImportKind::Mremoteng, b.clone());
        let e = install(&s, ImportKind::Mremoteng, &a, plan()).unwrap_err();
        assert_eq!(e.code, "no_pending_import");
        // same path, other kind
        import_begin(&s, ImportKind::Backup, a.clone());
        let e = install(&s, ImportKind::Mremoteng, &a, plan()).unwrap_err();
        assert_eq!(e.code, "no_pending_import");
        // cancelled
        import_cancel(&s);
        assert!(install(&s, ImportKind::Mremoteng, &a, plan()).is_err());
        assert_eq!(pending_kind(&s), "none");
        // the matching one goes through
        import_begin(&s, ImportKind::Mremoteng, a.clone());
        install(&s, ImportKind::Mremoteng, &a, plan()).unwrap();
        assert_eq!(pending_kind(&s), "connections");
    }

    #[test]
    fn a_poisoned_pending_lock_does_not_block_cancel_and_begin() {
        let dir = tempfile::tempdir().unwrap();
        let s = st(dir.path());
        std::thread::scope(|scope| {
            let _ = scope
                .spawn(|| {
                    let _g = s.pending.lock().unwrap();
                    panic!("poison");
                })
                .join();
        });
        assert!(s.pending.is_poisoned());
        import_begin(&s, ImportKind::Backup, dir.path().join("x"));
        assert_eq!(pending_kind_poisoned(&s), "picked");
        import_cancel(&s);
        assert_eq!(pending_kind_poisoned(&s), "none");
        assert_eq!(apply(&s).unwrap_err().code, "no_pending_import");
    }

    fn pending_kind_poisoned(s: &AppState) -> &'static str {
        match &*pending_slot(s) {
            None => "none",
            Some(Pending::Picked { .. }) => "picked",
            _ => "other",
        }
    }

    #[test]
    fn apply_clears_pending_and_a_second_apply_is_no_pending_import() {
        let dir = tempfile::tempdir().unwrap();
        let s = st(&dir.path().join("d"));
        unlocked(&s);
        let path = file(dir.path(), "c.xml", DEFAULT_KEY_XML);
        inspect(&s, ImportKind::Mremoteng, &path, None).unwrap();
        apply(&s).unwrap();
        assert_eq!(apply(&s).unwrap_err().code, "no_pending_import");
        // a picked but not inspected file cannot be applied either
        import_begin(&s, ImportKind::Mremoteng, path);
        assert_eq!(apply(&s).unwrap_err().code, "not_inspected");
    }

    #[test]
    fn cancel_clears_pending() {
        let dir = tempfile::tempdir().unwrap();
        let s = st(&dir.path().join("d"));
        let path = file(dir.path(), "c.xml", DEFAULT_KEY_XML);
        inspect(&s, ImportKind::Mremoteng, &path, None).unwrap();
        import_cancel(&s);
        assert_eq!(pending_kind(&s), "none");
        assert_eq!(apply(&s).unwrap_err().code, "no_pending_import");
    }

    #[test]
    fn import_begin_replaces_a_pending_plan() {
        let dir = tempfile::tempdir().unwrap();
        let s = st(&dir.path().join("d"));
        let path = file(dir.path(), "c.xml", DEFAULT_KEY_XML);
        inspect(&s, ImportKind::Mremoteng, &path, None).unwrap();
        import_begin(&s, ImportKind::Backup, path);
        assert_eq!(pending_kind(&s), "picked");
    }

    #[test]
    fn connections_json_import_merges_into_new_folder_without_touching_vault() {
        let dir = tempfile::tempdir().unwrap();
        let a = st(&dir.path().join("a"));
        let f = commands::create_folder(&a, None, None, "Lab".into()).unwrap();
        commands::create_connection(
            &a,
            f.id,
            None,
            "web".into(),
            fernjo_core::Protocol::Ssh,
            "h".into(),
        )
        .unwrap();
        let out = dir.path().join("fernjo-connections-2026-01-02.json");
        export_connections_to(&a, &out).unwrap();

        let b = st(&dir.path().join("b"));
        let p = inspect(&b, ImportKind::Connections, &out, None).unwrap();
        assert!(!p.needs_vault);
        assert_eq!(p.file_name, "fernjo-connections-2026-01-02.json");
        let r = apply(&b).unwrap();
        assert!(r.report.is_some());
        assert_eq!(top_names(&b), ["Imported from fernjo export (2026-01-02)"]);
        assert!(matches!(
            b.vault_status().unwrap(),
            crate::state::VaultStatus::None
        ));
    }

    #[test]
    fn export_connections_drops_credentials_and_reports_the_count() {
        let dir = tempfile::tempdir().unwrap();
        let a = st(&dir.path().join("a"));
        let m = commands::create_connection(
            &a,
            None,
            None,
            "c".into(),
            fernjo_core::Protocol::Ssh,
            "h".into(),
        )
        .unwrap();
        let cred = uuid::Uuid::new_v4();
        commands::update_node(
            &a,
            m.id.unwrap(),
            serde_json::from_value(json!({"credential": cred})).unwrap(),
        )
        .unwrap();
        let out = dir.path().join("x.json");
        let info = export_connections_to(&a, &out).unwrap();
        assert_eq!(info.file_name, "x.json");
        assert_eq!(info.dropped_credentials, 1);
        assert!(
            !fs::read_to_string(&out)
                .unwrap()
                .contains(&cred.to_string())
        );
    }

    #[test]
    fn restore_replaces_tree_and_entries_exactly() {
        let dir = tempfile::tempdir().unwrap();
        let (src, backup_path) = source_with_backup(dir.path());
        let dst = st(&dir.path().join("dst"));
        unlocked(&dst);
        commands::create_folder(&dst, None, None, "old".into()).unwrap();
        vault_commands::with_vault(&dst, |v| Ok(v.add(new_entry("old entry", "x"))?)).unwrap();

        let p = inspect(&dst, ImportKind::Backup, &backup_path, Some(BACKUP_PW)).unwrap();
        let summary = p.backup.as_ref().unwrap();
        assert_eq!(summary.credentials, entries_json(&src).len());
        assert!(summary.connections > 0 && summary.folders > 0);
        assert!(p.needs_vault && p.report.is_none());
        assert_eq!(pending_kind(&dst), "backup");

        let r = apply(&dst).unwrap();
        let restored = r.restored.unwrap();
        assert_eq!(restored.credentials, entries_json(&src).len());
        assert!(r.report.is_none());
        assert_eq!(
            r.snapshot.children,
            commands::list_tree(&src).unwrap().children
        );
        assert_eq!(entries_json(&dst), entries_json(&src));
        assert_eq!(pending_kind(&dst), "none");
        // persisted
        let reopened = st(&dir.path().join("dst"));
        assert_eq!(top_names(&reopened), top_names(&src));
    }

    #[test]
    fn restore_closes_sessions() {
        use tauri::ipc::{Channel, InvokeResponseBody};
        let dir = tempfile::tempdir().unwrap();
        let (_src, backup_path) = source_with_backup(dir.path());
        let dst = st(&dir.path().join("dst"));
        unlocked(&dst);
        let c = commands::create_connection(
            &dst,
            None,
            None,
            "c".into(),
            fernjo_core::Protocol::Ssh,
            "127.0.0.1".into(),
        )
        .unwrap();
        commands::update_node(
            &dst,
            c.id.unwrap(),
            serde_json::from_value(json!({"port": 1, "username": "u"})).unwrap(),
        )
        .unwrap();
        let channel = Channel::new(|_: InvokeResponseBody| Ok(()));
        crate::session_commands::open_session(&dst, c.id.unwrap(), channel, 80, 24).unwrap();
        assert_eq!(dst.sessions.len(), 1);
        inspect(&dst, ImportKind::Backup, &backup_path, Some(BACKUP_PW)).unwrap();
        apply(&dst).unwrap();
        assert_eq!(dst.sessions.len(), 0);
    }

    #[test]
    fn restore_with_entries_needs_unlocked_vault() {
        let dir = tempfile::tempdir().unwrap();
        let (_src, backup_path) = source_with_backup(dir.path());
        let dst = st(&dir.path().join("dst"));
        commands::create_folder(&dst, None, None, "keep".into()).unwrap();
        inspect(&dst, ImportKind::Backup, &backup_path, Some(BACKUP_PW)).unwrap();
        let e = apply(&dst).unwrap_err();
        assert_eq!(e.code, "vault_needed");
        assert_eq!(top_names(&dst), ["keep"]);
        assert_eq!(pending_kind(&dst), "backup");
        unlocked(&dst);
        vault_commands::vault_lock(&dst).unwrap();
        assert_eq!(apply(&dst).unwrap_err().code, "vault_needed");
        assert_eq!(top_names(&dst), ["keep"]);
        vault_commands::vault_unlock(&dst, sec(MASTER), false).unwrap();
        apply(&dst).unwrap();
    }

    #[test]
    fn restore_refuses_an_unusable_vault() {
        let dir = tempfile::tempdir().unwrap();
        let (_src, backup_path) = source_with_backup(dir.path());
        let data = dir.path().join("dst");
        fs::create_dir_all(&data).unwrap();
        fs::write(data.join(fernjo_vault::FILE_NAME), b"garbage").unwrap();
        let dst = st(&data);
        commands::create_folder(&dst, None, None, "keep".into()).unwrap();
        inspect(&dst, ImportKind::Backup, &backup_path, Some(BACKUP_PW)).unwrap();
        assert_eq!(apply(&dst).unwrap_err().code, "vault_unusable");
        assert_eq!(top_names(&dst), ["keep"]);
        assert_eq!(
            fs::read(data.join(fernjo_vault::FILE_NAME)).unwrap(),
            b"garbage"
        );
    }

    #[test]
    fn restore_puts_entries_back_when_tree_replace_fails() {
        let dir = tempfile::tempdir().unwrap();
        let (_src, backup_path) = source_with_backup(dir.path());
        let data = dir.path().join("dst");
        let dst = st(&data);
        unlocked(&dst);
        commands::create_folder(&dst, None, None, "old".into()).unwrap();
        vault_commands::with_vault(&dst, |v| Ok(v.add(new_entry("old entry", "x"))?)).unwrap();
        let before = entries_json(&dst);
        inspect(&dst, ImportKind::Backup, &backup_path, Some(BACKUP_PW)).unwrap();
        block_store_save(&data);
        let e = apply(&dst).unwrap_err();
        assert_eq!(e.code, "save_failed");
        assert_eq!(entries_json(&dst), before);
        assert_eq!(top_names(&dst), ["old"]);
        assert_eq!(pending_kind(&dst), "backup");
        // and the vault file on disk holds the old entries again
        assert_eq!(entries_on_disk(&data), before);
    }

    #[test]
    fn restore_of_backup_without_entries_clears_entries_and_summary_says_zero() {
        let dir = tempfile::tempdir().unwrap();
        let a = st(&dir.path().join("a"));
        commands::create_folder(&a, None, None, "Lab".into()).unwrap();
        let out = dir.path().join("b.fjbk");
        export_backup_to(&a, &out, &sec(BACKUP_PW)).unwrap();

        let dst = st(&dir.path().join("dst"));
        unlocked(&dst);
        vault_commands::with_vault(&dst, |v| Ok(v.add(new_entry("gone", "x"))?)).unwrap();
        let p = inspect(&dst, ImportKind::Backup, &out, Some(BACKUP_PW)).unwrap();
        // a vault exists and gets replaced as a whole, so it must be unlocked
        assert!(p.needs_vault);
        assert_eq!(p.backup.unwrap().credentials, 0);
        let r = apply(&dst).unwrap();
        assert_eq!(r.restored.unwrap().credentials, 0);
        assert_eq!(credential_count(&dst), 0);
        assert_eq!(top_names(&dst), ["Lab"]);
    }

    #[test]
    fn restore_without_entries_still_needs_the_existing_vault_unlocked() {
        let dir = tempfile::tempdir().unwrap();
        let a = st(&dir.path().join("a"));
        let out = dir.path().join("b.fjbk");
        export_backup_to(&a, &out, &sec(BACKUP_PW)).unwrap();
        let dst = st(&dir.path().join("dst"));
        unlocked(&dst);
        vault_commands::vault_lock(&dst).unwrap();
        inspect(&dst, ImportKind::Backup, &out, Some(BACKUP_PW)).unwrap();
        assert_eq!(apply(&dst).unwrap_err().code, "vault_needed");
    }

    #[test]
    fn export_backup_round_trips_through_inspect_and_apply() {
        let dir = tempfile::tempdir().unwrap();
        let (src, backup_path) = source_with_backup(dir.path());
        let dst = st(&dir.path().join("dst"));
        unlocked(&dst);
        let e = inspect(
            &dst,
            ImportKind::Backup,
            &backup_path,
            Some("wrong-password"),
        )
        .unwrap_err();
        assert_eq!(e.code, "wrong_password");
        assert_eq!(pending_kind(&dst), "picked");
        let e = inspect(&dst, ImportKind::Backup, &backup_path, None).unwrap_err();
        assert_eq!(e.code, "password_needed");
        inspect(&dst, ImportKind::Backup, &backup_path, Some(BACKUP_PW)).unwrap();
        apply(&dst).unwrap();
        assert_eq!(entries_json(&dst), entries_json(&src));
        assert_eq!(top_names(&dst), top_names(&src));
    }

    #[test]
    fn export_backup_needs_unlocked_vault_when_one_exists() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("b.fjbk");
        // no vault: fine, no entries
        let none = st(&dir.path().join("none"));
        let info = export_backup_to(&none, &out, &sec(BACKUP_PW)).unwrap();
        assert_eq!(info.file_name, "b.fjbk");
        // too short a password is refused before anything is written
        let short = dir.path().join("short.fjbk");
        let e = export_backup_to(&none, &short, &sec("short")).unwrap_err();
        assert_eq!(e.code, "password_too_short");
        assert!(!short.exists());
        // locked
        let locked = st(&dir.path().join("locked"));
        unlocked(&locked);
        vault_commands::vault_lock(&locked).unwrap();
        let blocked = dir.path().join("locked.fjbk");
        let e = export_backup_to(&locked, &blocked, &sec(BACKUP_PW)).unwrap_err();
        assert_eq!(e.code, "vault_locked");
        assert!(!blocked.exists());
        // unusable
        let data = dir.path().join("bad");
        fs::create_dir_all(&data).unwrap();
        fs::write(data.join(fernjo_vault::FILE_NAME), b"garbage").unwrap();
        let bad = st(&data);
        let e = export_backup_to(&bad, &blocked, &sec(BACKUP_PW)).unwrap_err();
        assert_eq!(e.code, "vault_unusable");
    }

    #[test]
    fn backup_with_duplicate_entry_ids_is_damaged_and_touches_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4();
        let contents = BackupContents {
            created_at: "2026-01-02T00:00:00Z".into(),
            app: "test".into(),
            tree: fernjo_core::Tree::new(),
            entries: vec![(id, new_entry("a", "x")), (id, new_entry("b", "y"))],
        };
        let bytes = backup::create(&contents, &sec(BACKUP_PW), KdfParams::TEST).unwrap();
        let path = file(dir.path(), "dup.fjbk", &bytes);
        let dst = st(&dir.path().join("dst"));
        unlocked(&dst);
        commands::create_folder(&dst, None, None, "keep".into()).unwrap();
        let e = inspect(&dst, ImportKind::Backup, &path, Some(BACKUP_PW)).unwrap_err();
        assert_eq!(e.code, "damaged");
        assert_eq!(pending_kind(&dst), "picked");
        // a pending value that slipped through is refused at apply and dropped
        *dst.pending.lock().unwrap() = Some(Pending::Backup(contents));
        assert_eq!(apply(&dst).unwrap_err().code, "damaged");
        assert_eq!(pending_kind(&dst), "none");
        assert_eq!(top_names(&dst), ["keep"]);
    }

    #[test]
    fn backup_with_dangling_credential_reference_restores_without_panic() {
        let dir = tempfile::tempdir().unwrap();
        let a = st(&dir.path().join("a"));
        let m = commands::create_connection(
            &a,
            None,
            None,
            "c".into(),
            fernjo_core::Protocol::Ssh,
            "h".into(),
        )
        .unwrap();
        commands::update_node(
            &a,
            m.id.unwrap(),
            serde_json::from_value(json!({"credential": uuid::Uuid::new_v4()})).unwrap(),
        )
        .unwrap();
        let out = dir.path().join("b.fjbk");
        export_backup_to(&a, &out, &sec(BACKUP_PW)).unwrap();
        let dst = st(&dir.path().join("dst"));
        inspect(&dst, ImportKind::Backup, &out, Some(BACKUP_PW)).unwrap();
        let r = apply(&dst).unwrap();
        assert_eq!(r.snapshot.children.len(), 1);
    }

    #[test]
    fn import_apply_honours_restart_required() {
        let dir = tempfile::tempdir().unwrap();
        let s = st(&dir.path().join("d"));
        unlocked(&s);
        let path = file(dir.path(), "c.xml", DEFAULT_KEY_XML);
        inspect(&s, ImportKind::Mremoteng, &path, None).unwrap();
        s.set_restart_required(true);
        assert_eq!(apply(&s).unwrap_err().code, "restart_required");
        assert_eq!(pending_kind(&s), "mremoteng");
        assert_eq!(credential_count(&s), 0);
    }

    // ---- fix wave: depth, one vault save, export targets, wording --------------------------

    /// `levels` nested folders; the innermost one is at level `levels`.
    fn deep_folder(levels: usize) -> Node {
        let mut node = Node {
            id: uuid::Uuid::new_v4(),
            name: "level-0".into(),
            description: String::new(),
            settings: Default::default(),
            kind: NodeKind::Folder { children: vec![] },
        };
        for i in 1..levels {
            node = Node {
                id: uuid::Uuid::new_v4(),
                name: format!("level-{i}"),
                description: String::new(),
                settings: Default::default(),
                kind: NodeKind::Folder {
                    children: vec![node],
                },
            };
        }
        node
    }

    /// A mRemoteNG file (default key) with `levels` nested containers.
    fn nested_xml(levels: usize) -> Vec<u8> {
        let text = std::str::from_utf8(DEFAULT_KEY_XML).unwrap();
        let start = text.find("Protected=\"").unwrap() + "Protected=\"".len();
        let protected = &text[start..start + text[start..].find('"').unwrap()];
        let mut body = String::new();
        for i in 0..levels {
            body.push_str(&format!(r#"<Node Name="c{i}" Type="Container">"#));
        }
        body.push_str(&"</Node>".repeat(levels));
        format!(
            r#"<?xml version="1.0" encoding="utf-8"?><Connections Name="Connections" Export="false" EncryptionEngine="AES" BlockCipherMode="GCM" KdfIterations="1000" FullFileEncryption="false" Protected="{protected}" ConfVersion="2.6">{body}</Connections>"#
        )
        .into_bytes()
    }

    #[test]
    fn an_import_at_the_depth_limit_reloads_and_one_level_over_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("d");
        let s = st(&data);
        // the deepest file the reader takes; the import folder makes it exactly the core limit
        let at_limit = file(
            dir.path(),
            "limit.xml",
            &nested_xml(fernjo_import::mremoteng::file::MAX_DEPTH),
        );
        inspect(&s, ImportKind::Mremoteng, &at_limit, None).unwrap();
        let applied = apply(&s).unwrap();
        // another run of the app loads the very same tree, with no fallback notice
        let again = st(&data);
        assert_eq!(
            commands::list_tree(&again).unwrap().children,
            applied.snapshot.children
        );
        assert!(again.notices.lock().unwrap().is_empty());
        let saved = fs::read(data.join(fernjo_core::FILE_NAME)).unwrap();

        let over = file(
            dir.path(),
            "over.xml",
            &nested_xml(fernjo_import::mremoteng::file::MAX_DEPTH + 1),
        );
        let e = inspect(&s, ImportKind::Mremoteng, &over, None).unwrap_err();
        assert_eq!(e.code, "too_complex");
        assert!(e.message.contains("levels"), "{}", e.message);
        assert_eq!(fs::read(data.join(fernjo_core::FILE_NAME)).unwrap(), saved);
    }

    #[test]
    fn a_plan_that_is_too_deep_is_refused_as_too_deep_and_rolls_its_entries_back() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("d");
        let s = st(&data);
        unlocked(&s);
        let mut p = fernjo_import::mremoteng::plan(
            &fernjo_import::mremoteng::read(DEFAULT_KEY_XML, None).unwrap(),
            TODAY,
            &[],
        );
        assert!(!p.entries.is_empty());
        p.folder = deep_folder(fernjo_core::MAX_TREE_DEPTH + 1);
        *s.pending.lock().unwrap() = Some(Pending::MRemoteNg(p));
        let e = apply(&s).unwrap_err();
        assert_eq!(e.code, "too_deep");
        assert!(e.message.contains("60"), "{}", e.message);
        assert_eq!(credential_count(&s), 0);
        assert!(!data.join(fernjo_core::FILE_NAME).exists());
        assert_eq!(pending_kind(&s), "mremoteng");

        // a connections plan as well
        *s.pending.lock().unwrap() = Some(Pending::Connections(ConnectionsPlan {
            folder: deep_folder(fernjo_core::MAX_TREE_DEPTH + 1),
            report: Report::default(),
        }));
        assert_eq!(apply(&s).unwrap_err().code, "too_deep");
        assert!(!data.join(fernjo_core::FILE_NAME).exists());
    }

    #[test]
    fn an_import_saves_the_vault_once_and_bak_is_the_vault_before_it() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("d");
        let s = st(&data);
        unlocked(&s);
        vault_commands::with_vault(&s, |v| Ok(v.add(new_entry("old", "x"))?)).unwrap();
        let before = fs::read(data.join(fernjo_vault::FILE_NAME)).unwrap();
        let path = file(dir.path(), "c.xml", DEFAULT_KEY_XML);
        let p = inspect(&s, ImportKind::Mremoteng, &path, None).unwrap();
        assert!(p.report.unwrap().counts.credentials >= 2);
        apply(&s).unwrap();
        let bak = fs::read(data.join(format!("{}.bak", fernjo_vault::FILE_NAME))).unwrap();
        assert_eq!(bak, before, "one save: .bak is the pre-import vault");
    }

    #[test]
    fn an_export_into_the_data_folder_is_refused_and_nothing_is_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("d");
        let s = st(&data);
        unlocked(&s);
        commands::create_folder(&s, None, None, "keep".into()).unwrap();
        let tree_before = fs::read(data.join(fernjo_core::FILE_NAME)).unwrap();
        let vault_before = fs::read(data.join(fernjo_vault::FILE_NAME)).unwrap();
        let sub = data.join("sub");
        fs::create_dir_all(&sub).unwrap();
        for target in [
            data.join(fernjo_core::FILE_NAME),
            data.join(fernjo_vault::FILE_NAME),
            data.join("new-file.json"),
            sub.join("x.json"),
            // spelled with a detour through the folder itself
            data.join("sub").join("..").join(fernjo_core::FILE_NAME),
        ] {
            let e = export_connections_to(&s, &target).unwrap_err();
            assert_eq!(e.code, "export_target_not_allowed", "{target:?}");
            let e = export_backup_to(&s, &target, &sec(BACKUP_PW)).unwrap_err();
            assert_eq!(e.code, "export_target_not_allowed", "{target:?}");
        }
        assert_eq!(
            fs::read(data.join(fernjo_core::FILE_NAME)).unwrap(),
            tree_before
        );
        assert_eq!(
            fs::read(data.join(fernjo_vault::FILE_NAME)).unwrap(),
            vault_before
        );
        assert!(!data.join("new-file.json").exists() && !sub.join("x.json").exists());
        // a sibling folder is fine, also one whose name starts like the data folder's
        let sibling = dir.path().join("d2");
        fs::create_dir_all(&sibling).unwrap();
        export_connections_to(&s, &sibling.join("c.json")).unwrap();
        export_backup_to(&s, &sibling.join("b.fjbk"), &sec(BACKUP_PW)).unwrap();
    }

    #[test]
    fn export_refuses_config_device_and_pending_folders() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = st(&dir.path().join("d"));
        let config = dir.path().join("config");
        let device = dir.path().join("device");
        let pending = dir.path().join("pending");
        for p in [&config, &device, &pending] {
            fs::create_dir_all(p).unwrap();
        }
        s.config_dir = Some(config.clone());
        s.device_dir = Some(device.clone());
        *s.pending_data_dir.lock().unwrap() = Some((pending.clone(), Vec::new()));
        for target in [
            config.join("settings.json"),
            config.join("new").join("x.json"),
            device.join("vault-marks.json"),
            // a detour through a sibling and a relative `..`
            device
                .join("..")
                .join("device")
                .join("known_hosts.confirmed"),
            pending.join("connections.json"),
        ] {
            let e = export_connections_to(&s, &target).unwrap_err();
            assert_eq!(e.code, "export_target_not_allowed", "{target:?}");
            let e = export_backup_to(&s, &target, &sec(BACKUP_PW)).unwrap_err();
            assert_eq!(e.code, "export_target_not_allowed", "{target:?}");
        }
        assert!(!config.join("settings.json").exists());
        assert!(!pending.join("connections.json").exists());
        // folders next to them with a similar name stay fine
        let ok = dir.path().join("config2");
        fs::create_dir_all(&ok).unwrap();
        export_connections_to(&s, &ok.join("c.json")).unwrap();
    }

    #[test]
    fn clear_pending_does_not_block_while_an_apply_holds_the_lock() {
        let dir = tempfile::tempdir().unwrap();
        let s = st(&dir.path().join("d"));
        let path = file(dir.path(), "c.xml", DEFAULT_KEY_XML);
        inspect(&s, ImportKind::Mremoteng, &path, None).unwrap();
        let (held_tx, held_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let holder = &s;
        std::thread::scope(|scope| {
            scope.spawn(move || {
                let _g = holder.pending.lock().unwrap();
                held_tx.send(()).unwrap();
                let _ = release_rx.recv();
            });
            held_rx.recv().unwrap();
            let started = std::time::Instant::now();
            s.clear_pending(); // would deadlock the test (blocks) with a plain lock()
            assert!(started.elapsed() < std::time::Duration::from_secs(1));
            release_tx.send(()).unwrap();
        });
        // skipped while held: the value is still there; once free, a page load clears it
        assert_eq!(pending_kind(&s), "mremoteng");
        s.clear_pending();
        assert_eq!(pending_kind(&s), "none");
    }

    #[test]
    fn the_without_passwords_report_has_no_stale_inherit_lines_and_keeps_its_references_line() {
        let dir = tempfile::tempdir().unwrap();
        let s = st(&dir.path().join("d"));
        let path = file(dir.path(), "c.xml", DEFAULT_KEY_XML);
        inspect(&s, ImportKind::Mremoteng, &path, None).unwrap();
        // worked out with the plan; the stale-line cases themselves are tested in fernjo-import
        let bare = match &*s.pending.lock().unwrap() {
            Some(Pending::MRemoteNg(p)) => p.report_without_passwords.clone(),
            _ => panic!("plan expected"),
        };
        let r = import_apply(
            &s,
            ApplyOptions {
                without_passwords: true,
            },
        )
        .unwrap();
        let report = r.report.unwrap();
        assert_eq!(report, bare);
        assert!(
            report
                .partial
                .iter()
                .all(|l| !l.text.contains("now uses the credential"))
        );
        assert!(
            report
                .partial
                .iter()
                .any(|l| l.text.contains("stored password reference"))
        );
    }

    #[test]
    fn the_export_prechecks_run_without_a_target_and_name_what_is_wrong() {
        let dir = tempfile::tempdir().unwrap();
        // no vault: fine
        let none = st(&dir.path().join("none"));
        export_backup_precheck(&none, &sec(BACKUP_PW)).unwrap();
        let e = export_backup_precheck(&none, &sec("short")).unwrap_err();
        assert_eq!(e.code, "password_too_short");
        // locked
        let locked = st(&dir.path().join("locked"));
        unlocked(&locked);
        export_backup_precheck(&locked, &sec(BACKUP_PW)).unwrap();
        vault_commands::vault_lock(&locked).unwrap();
        let e = export_backup_precheck(&locked, &sec(BACKUP_PW)).unwrap_err();
        assert_eq!(e.code, "vault_locked");
        // unusable
        let data = dir.path().join("bad");
        fs::create_dir_all(&data).unwrap();
        fs::write(data.join(fernjo_vault::FILE_NAME), b"garbage").unwrap();
        let e = export_backup_precheck(&st(&data), &sec(BACKUP_PW)).unwrap_err();
        assert_eq!(e.code, "vault_unusable");
        // nothing was written anywhere by any of them
        assert!(
            !dir.path()
                .join("locked")
                .join(fernjo_core::FILE_NAME)
                .exists()
        );
    }
}
