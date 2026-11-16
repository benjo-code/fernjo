//! Import, restore and export as plain functions over `AppState` (the `#[tauri::command]`
//! wrappers come with the UI work).
//!
//! One file at a time waits in `AppState.pending`: `import_begin` remembers the picked path,
//! `import_inspect` reads it and replaces the path by a plan (or a decrypted backup), and
//! `import_apply` or `import_cancel` ends it. A wrong key or password leaves the picked file
//! in place so the UI can ask again. Secrets typed by the user become `SecretString` on
//! arrival and no response type carries one: previews and results hold counts, names, the
//! report, the vault status and the tree snapshot.
//!
//! Lock order is the one of `delete_credential`: pending, then vault, then store.

// Task 9 registers the commands that call these functions; until then only the tests do.
#![allow(dead_code)]

use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::PoisonError;
use std::time::{SystemTime, UNIX_EPOCH};

use fernjo_core::{Node, NodeKind, Tree, write_exact};
use fernjo_import::backup::{self, BackupContents, BackupError};
use fernjo_import::connections::{self, ConnectionsError, ConnectionsPlan};
use fernjo_import::error::ImportError;
use fernjo_import::mremoteng::{self, ImportPlan, PlannedEntry};
use fernjo_import::report::{Report, ReportLine};
use fernjo_vault::{EntryId, Vault};
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

/// `(YYYY-MM-DD, YYYY-MM-DDTHH:MM:SSZ)` for Unix seconds, in UTC. The workspace has no date
/// crate of its own, so the folder date is the UTC date (it can differ from the local date
/// around midnight; a repeat on the same day relies on the numbered folder name).
fn format_utc(secs: u64) -> (String, String) {
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let rest = secs % 86_400;
    // civil-from-days (proleptic Gregorian), after H. Hinnant
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let date = format!("{year:04}-{month:02}-{day:02}");
    let time = format!(
        "{date}T{:02}:{:02}:{:02}Z",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    );
    (date, time)
}

fn now_utc() -> (String, String) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    format_utc(secs)
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

/// Remembers the picked file and drops whatever was pending.
pub fn import_begin(state: &AppState, kind: ImportKind, path: PathBuf) -> PickedFile {
    let file_name = file_name_of(&path);
    *state.pending.lock().unwrap_or_else(PoisonError::into_inner) =
        Some(Pending::Picked { kind, path });
    PickedFile { file_name }
}

/// Drops the pending value (also what every page load does).
pub fn import_cancel(state: &AppState) {
    *state.pending.lock().unwrap_or_else(PoisonError::into_inner) = None;
}

/// Reads the picked file with the optional typed key (mRemoteNG) or password (backup) and
/// returns what applying it would do. `key_needed`, `wrong_key`, `wrong_password` and
/// `password_needed` leave the picked file in place for a retry; success replaces it by the
/// plan; any other error leaves it as it was.
pub fn import_inspect(state: &AppState, secret: Option<String>) -> Result<Preview, CommandError> {
    inspect_on(state, secret, &now_utc().0)
}

/// [`import_inspect`] with the date passed in, so tests do not depend on the clock.
pub(crate) fn inspect_on(
    state: &AppState,
    secret: Option<String>,
    today: &str,
) -> Result<Preview, CommandError> {
    // wrapped at once; `String` -> `SecretString` moves the buffer
    let secret = secret.map(SecretString::from);
    let (kind, path) = match &*state.pending.lock().map_err(|_| CommandError::internal())? {
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
    let needs_vault = match &next {
        Pending::MRemoteNg(p) => !p.entries.is_empty(),
        Pending::Backup(c) => !c.entries.is_empty(),
        _ => false,
    };
    // Replace only when the user has not picked or cancelled something else meanwhile.
    {
        let mut slot = state.pending.lock().map_err(|_| CommandError::internal())?;
        match &*slot {
            Some(Pending::Picked { path: now, .. }) if *now == path => *slot = Some(next),
            _ => return Err(error("no_pending_import", "the picked file was replaced")),
        }
    }
    Ok(Preview {
        kind,
        file_name: file_name_of(&path),
        report,
        backup,
        needs_vault,
        vault: vault_status(state)?,
    })
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
    let mut slot = state.pending.lock().map_err(|_| CommandError::internal())?;
    let result = match &*slot {
        None => Err(error("no_pending_import", "there is nothing to apply")),
        Some(Pending::Picked { .. }) => Err(error(
            "not_inspected",
            "the picked file has not been inspected yet",
        )),
        Some(Pending::MRemoteNg(plan)) => apply_mremoteng(state, plan, options.without_passwords),
        Some(Pending::Connections(plan)) => apply_connections(state, plan),
        Some(Pending::Backup(contents)) => apply_backup(state, contents),
    };
    match &result {
        Ok(_) => *slot = None,
        Err(e) if e.code == "damaged" => *slot = None,
        Err(_) => {}
    }
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
    let mut report = plan.report.clone();
    let entries: &[PlannedEntry] = if without_passwords {
        let dropped = strip_references(&mut folder);
        report.counts.credentials = 0;
        report.counts.shared_credentials = 0;
        if dropped > 0 {
            let s = if dropped == 1 { "" } else { "s" };
            report.partial.push(ReportLine::new(
                "",
                format!(
                    "{dropped} stored password reference{s} left out (imported without passwords)"
                ),
            ));
            report.counts.partial += 1;
        }
        &[]
    } else {
        &plan.entries
    };

    let snap = if entries.is_empty() {
        with_store(state, |store| {
            store.graft(None, folder)?;
            Ok(snapshot(store, Vec::new()))
        })?
    } else {
        require_unlocked_vault(state, "this import")?;
        with_vault(state, |v| {
            let mut added: Vec<EntryId> = Vec::new();
            for planned in entries {
                if let Err(e) = v.add_with_id(planned.id, planned.entry.clone()) {
                    remove_entries(v, &added);
                    return Err(e.into());
                }
                added.push(planned.id);
            }
            with_store(state, |store| {
                store.graft(None, folder)?;
                Ok(snapshot(store, Vec::new()))
            })
            .inspect_err(|_| remove_entries(v, &added))
        })?
    };
    Ok(ApplyResult {
        snapshot: snap,
        report: Some(report),
        restored: None,
    })
}

/// Best effort: a delete that fails leaves an unreferenced entry behind, which the owner can
/// remove in the vault.
fn remove_entries(vault: &mut Vault, ids: &[EntryId]) {
    for id in ids.iter().rev() {
        let _ = vault.delete(*id);
    }
}

fn apply_backup(state: &AppState, contents: &BackupContents) -> Result<ApplyResult, CommandError> {
    validate_backup(contents)?;
    // A vault that exists is part of "replace everything", so it must be usable and unlocked
    // even when the backup holds no entries; without a vault there is nothing to replace.
    let has_vault = !matches!(state.vault_status()?, State::None);
    if has_vault || !contents.entries.is_empty() {
        require_unlocked_vault(state, "this restore")?;
    }

    state.sessions.close_all();
    let replace_tree = |tree: Tree| {
        with_store(state, |store| {
            store.replace_tree(tree)?;
            Ok(snapshot(store, Vec::new()))
        })
    };
    let snap = if has_vault {
        with_vault(state, |v| {
            let previous = v.export_entries()?;
            v.replace_entries(contents.entries.clone())?;
            replace_tree(contents.tree.clone()).inspect_err(|_| {
                let _ = v.replace_entries(previous);
            })
        })?
    } else {
        replace_tree(contents.tree.clone())?
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

/// Writes the connection tree to `path` as a fernjo connections file (no credential
/// references).
pub fn export_connections_to(state: &AppState, path: &Path) -> Result<ExportInfo, CommandError> {
    let tree = with_store(state, |s| Ok(s.tree().clone()))?;
    let out = connections::export(&tree, &now_utc().1);
    write_file(path, &out.bytes)?;
    Ok(ExportInfo {
        file_name: file_name_of(path),
        dropped_credentials: out.dropped_credentials,
    })
}

/// Writes an encrypted backup of the tree and the vault entries to `path`. A vault that
/// exists must be unlocked; without a vault the backup holds the tree only.
pub fn export_backup_to(
    state: &AppState,
    path: &Path,
    password: String,
) -> Result<ExportInfo, CommandError> {
    let password = SecretString::from(password);
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
    let (_, created_at) = now_utc();
    let contents = BackupContents {
        created_at,
        app: fernjo_core::version().to_string(),
        tree,
        entries,
    };
    let bytes = backup::create(&contents, &password, state.kdf)?;
    write_file(path, &bytes)?;
    Ok(ExportInfo {
        file_name: file_name_of(path),
        dropped_credentials: 0,
    })
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
        export_backup_to(&src, &out, BACKUP_PW.to_string()).unwrap();
        (src, out)
    }

    fn block_store_save(dir: &Path) {
        // the store writes `connections.json.tmp` first; a folder of that name fails the save
        fs::create_dir(dir.join("connections.json.tmp")).unwrap();
    }

    #[test]
    fn utc_date_and_time_are_formatted_from_unix_seconds() {
        assert_eq!(
            format_utc(0),
            ("1970-01-01".to_string(), "1970-01-01T00:00:00Z".to_string())
        );
        assert_eq!(format_utc(1_700_000_000).1, "2023-11-14T22:13:20Z");
        assert_eq!(format_utc(1_709_164_800).0, "2024-02-29");
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
        let last = report.partial.last().unwrap();
        assert!(
            last.text.contains(&before.len().to_string()),
            "{}",
            last.text
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
        inspect(&s, ImportKind::Mremoteng, &path, None).unwrap();
        block_store_save(&data);
        let e = apply(&s).unwrap_err();
        assert_eq!(e.code, "save_failed");
        assert_eq!(credential_count(&s), 0);
        assert!(top_names(&s).is_empty());
        assert_eq!(pending_kind(&s), "mremoteng");
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
    }

    #[test]
    fn restore_of_backup_without_entries_clears_entries_and_summary_says_zero() {
        let dir = tempfile::tempdir().unwrap();
        let a = st(&dir.path().join("a"));
        commands::create_folder(&a, None, None, "Lab".into()).unwrap();
        let out = dir.path().join("b.fjbk");
        export_backup_to(&a, &out, BACKUP_PW.to_string()).unwrap();

        let dst = st(&dir.path().join("dst"));
        unlocked(&dst);
        vault_commands::with_vault(&dst, |v| Ok(v.add(new_entry("gone", "x"))?)).unwrap();
        let p = inspect(&dst, ImportKind::Backup, &out, Some(BACKUP_PW)).unwrap();
        assert!(!p.needs_vault);
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
        export_backup_to(&a, &out, BACKUP_PW.to_string()).unwrap();
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
        let info = export_backup_to(&none, &out, BACKUP_PW.to_string()).unwrap();
        assert_eq!(info.file_name, "b.fjbk");
        // too short a password is refused before anything is written
        let short = dir.path().join("short.fjbk");
        let e = export_backup_to(&none, &short, "short".to_string()).unwrap_err();
        assert_eq!(e.code, "password_too_short");
        assert!(!short.exists());
        // locked
        let locked = st(&dir.path().join("locked"));
        unlocked(&locked);
        vault_commands::vault_lock(&locked).unwrap();
        let blocked = dir.path().join("locked.fjbk");
        let e = export_backup_to(&locked, &blocked, BACKUP_PW.to_string()).unwrap_err();
        assert_eq!(e.code, "vault_locked");
        assert!(!blocked.exists());
        // unusable
        let data = dir.path().join("bad");
        fs::create_dir_all(&data).unwrap();
        fs::write(data.join(fernjo_vault::FILE_NAME), b"garbage").unwrap();
        let bad = st(&data);
        let e = export_backup_to(&bad, &blocked, BACKUP_PW.to_string()).unwrap_err();
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
        export_backup_to(&a, &out, BACKUP_PW.to_string()).unwrap();
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
}
