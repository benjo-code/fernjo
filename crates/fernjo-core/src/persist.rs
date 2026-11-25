//! `connections.json`: format, atomic write, backup and load fallback.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use fernjo_fs::{FileMode, Parent, WriteOptions};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::{DocumentError, LoadError, SaveError};
use crate::json;
use crate::model::{Node, NodeId, NodeKind, RDP_COLOR_DEPTHS, Settings, is_valid_domain};
use crate::tree::Tree;

/// File format version this build writes and understands.
pub const CURRENT_VERSION: u32 = 3;
/// Name of the connections file inside the data folder.
pub const FILE_NAME: &str = "connections.json";

/// Something the user should be told about after loading.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Notice {
    /// The main file was missing or unreadable and the backup was used instead.
    RestoredFromBackup { bad_file: Option<PathBuf> },
    /// The data folder holds copies made by a sync tool after a conflict (file names only).
    SyncConflicts { files: Vec<String> },
    /// `settings.json` could not be used; the default data folder is in effect and the file
    /// is left untouched.
    SettingsUnreadable { message: String },
    /// `settings.json` names a data folder that contains the device-local folder (a
    /// hand-edited file; the data-folder dialog refuses that choice). The device folder is
    /// then synced with the data, which defeats its purpose.
    DeviceFolderInsideDataDir { path: PathBuf },
}

/// Result of a successful load.
#[derive(Debug)]
pub struct Loaded {
    pub tree: Tree,
    pub notice: Option<Notice>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FileOut<'a> {
    schema_version: u32,
    children: &'a [Node],
}

/// What a `connections.json` document is read into. Only `children` is read; `schemaVersion` is
/// read before by `json::head`, unknown keys are skipped without being kept.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct FileIn {
    children: Vec<Node>,
}

pub(crate) use fernjo_fs::with_suffix;

/// The JSON document `save` writes for `tree`: `schemaVersion` and `children`.
fn file_out(tree: &Tree) -> FileOut<'_> {
    FileOut {
        schema_version: CURRENT_VERSION,
        children: tree.children(),
    }
}

/// For in-memory consumers only: `Value` objects are key-sorted, so `save` serialises
/// `FileOut` directly to keep the on-disk key order.
pub fn document(tree: &Tree) -> Value {
    serde_json::to_value(file_out(tree)).expect("a tree always serialises")
}

/// Writes `tree` atomically: temp file in the same folder, flushed to disk, the current
/// file copied to `.bak`, then the temp file renamed over the target.
/// Returns the fingerprint of exactly the bytes written (the trailing newline included).
pub fn save(path: &Path, tree: &Tree) -> Result<Fingerprint, SaveError> {
    let json = serde_json::to_string_pretty(&file_out(tree)).map_err(|e| SaveError {
        path: path.to_path_buf(),
        source: io::Error::other(e),
    })?;
    write_atomic(path, json.as_bytes())?;
    let mut hasher = Sha256::new();
    hasher.update(json.as_bytes());
    hasher.update(b"\n");
    Ok(hasher.finalize().into())
}

/// SHA-256 of a file's bytes, used to notice that another program changed the file.
pub type Fingerprint = [u8; 32];

/// Fingerprint of the bytes in `path` (read with the size cap); `None` when it is missing.
pub fn fingerprint(path: &Path) -> io::Result<Option<Fingerprint>> {
    match read_capped(path) {
        Ok(bytes) => Ok(Some(Sha256::digest(&bytes).into())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Shared by every file in the data and config folders: creates the parent folder (private on
/// Unix), writes a temp file next to `path` under a random name, flushes it to disk, keeps the
/// current file as `.bak`, then renames the temp file over `path` (all in `fernjo_fs`).
/// `contents` gets a trailing newline. Afterwards a fixed-name `<path>.tmp` left by an older
/// build is removed (a regular file only; a failure never fails the save).
pub(crate) fn write_atomic(path: &Path, contents: &[u8]) -> Result<(), SaveError> {
    let mut bytes = Vec::with_capacity(contents.len() + 1);
    bytes.extend_from_slice(contents);
    bytes.push(b'\n');
    fernjo_fs::write_atomic(path, &bytes, WriteOptions::data_file()).map_err(|source| {
        SaveError {
            path: path.to_path_buf(),
            source,
        }
    })?;
    remove_legacy_temp(path);
    Ok(())
}

/// Removes `<path>.tmp`, the fixed temp name older builds used, when it is a regular file.
/// A symlink or a folder at that name is never followed or touched; errors are ignored.
fn remove_legacy_temp(path: &Path) {
    let legacy = with_suffix(path, ".tmp");
    if fs::symlink_metadata(&legacy).is_ok_and(|m| m.file_type().is_file()) {
        let _ = fs::remove_file(&legacy);
    }
}

/// Renames an unreadable `path` to `<path>.corrupt-<unix seconds>` so a later save cannot
/// overwrite the good `.bak` with it. Returns the new name.
pub(crate) fn move_aside(path: &Path) -> io::Result<PathBuf> {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let target = with_suffix(path, &format!(".corrupt-{secs}"));
    fs::rename(path, &target)?;
    Ok(target)
}

/// Upgrades a parsed file from version `from` using `steps` (step `i` upgrades
/// version `i + 1` to `i + 2`).
pub fn migrate_with<T>(value: T, from: u32, steps: &[fn(T) -> T]) -> T {
    let skip = from.saturating_sub(1) as usize;
    steps.iter().skip(skip).fold(value, |v, step| step(v))
}

/// Upgrade steps; step `i` upgrades version `i + 1` to `i + 2`. They run on the typed document:
/// every schema so far only added optional fields, which the typed read fills with their defaults
/// for an older file. A future step that has to rewrite data reads the old shape into its own
/// typed struct; it must not go back to a `serde_json::Value` (M11 review #28, `crate::json`).
const MIGRATIONS: [fn(FileIn) -> FileIn; 2] = [one_to_two, two_to_three];

/// 1 -> 2: schema 2 adds the optional `credential` setting, so nothing is rewritten. The
/// version still has to change: an older build would silently drop `credential` on its next
/// save, so it must refuse the file as a newer schema instead.
fn one_to_two(value: FileIn) -> FileIn {
    value
}

/// 2 -> 3: schema 3 adds the optional RDP settings (`domain`, `rdpResolution`,
/// `rdpColorDepth`, `rdpClipboard`), so nothing is rewritten. The version still has to change: an
/// older build would silently drop them on its next save, so it must refuse the file as a newer
/// schema instead.
fn two_to_three(value: FileIn) -> FileIn {
    value
}

fn migrate(value: FileIn, from: u32) -> FileIn {
    migrate_with(value, from, &MIGRATIONS)
}

enum Read {
    Missing,
    Parsed(Tree),
    Bad(String),
}

/// Largest `connections.json` or `settings.json` (and `.bak`) `load` reads: 64 MiB, far above
/// any real tree. A larger file is refused as unreadable and left untouched.
pub const MAX_DATA_FILE_LEN: u64 = 64 * 1024 * 1024;

/// Reads `path` unless it is longer than [`MAX_DATA_FILE_LEN`] (`InvalidData`); see
/// `fernjo_fs::read_capped`.
pub(crate) fn read_capped(path: &Path) -> io::Result<Vec<u8>> {
    fernjo_fs::read_capped(path, MAX_DATA_FILE_LEN)
}

fn read_file(path: &Path) -> Result<Read, LoadError> {
    let bytes = match read_capped(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Read::Missing),
        Err(e) => {
            return Err(LoadError::Unreadable {
                path: path.to_path_buf(),
                reason: e.to_string(),
            });
        }
    };
    // editors on Windows may add a UTF-8 byte order mark; that is not corruption
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
    match parse_document_bytes(bytes) {
        Ok(tree) => Ok(Read::Parsed(tree)),
        Err(DocumentError::NewerSchema { found, supported }) => Err(LoadError::NewerSchema {
            found,
            supported,
            path: path.to_path_buf(),
        }),
        Err(DocumentError::Invalid(reason)) => Ok(Read::Bad(reason)),
    }
}

/// Turns the bytes of a `connections.json` document into a tree: checks that it is JSON and its
/// `schemaVersion`, migrates older versions and runs `sanitize`. Shared by loading a file and by
/// importing a document. The document is never built as a `serde_json::Value` (`crate::json`):
/// the first pass validates it and reads `schemaVersion`, the second reads it into `FileIn`.
pub fn parse_document_bytes(bytes: &[u8]) -> Result<Tree, DocumentError> {
    let [version] =
        json::head(bytes, ["schemaVersion"]).map_err(|e| DocumentError::Invalid(e.to_string()))?;
    let Some(version) = version.as_u64() else {
        return Err(DocumentError::Invalid(
            "missing or invalid schemaVersion".to_string(),
        ));
    };
    let version = u32::try_from(version).unwrap_or(u32::MAX);
    if version > CURRENT_VERSION {
        return Err(DocumentError::NewerSchema {
            found: version,
            supported: CURRENT_VERSION,
        });
    }
    if version == 0 {
        return Err(DocumentError::Invalid(
            "invalid schemaVersion 0".to_string(),
        ));
    }
    let file: FileIn =
        serde_json::from_slice(bytes).map_err(|e| DocumentError::Invalid(e.to_string()))?;
    let mut file = migrate(file, version);
    sanitize(&mut file.children, &mut HashSet::new()).map_err(DocumentError::Invalid)?;
    Ok(Tree::from_children(file.children))
}

/// [`parse_document_bytes`] for a document that is already in memory as a `serde_json::Value`
/// (tests, a document fernjo built itself). A file or anything from another device goes through
/// [`parse_document_bytes`], never through a `Value`.
pub fn parse_document(value: Value) -> Result<Tree, DocumentError> {
    let bytes = serde_json::to_vec(&value).expect("a Value always serialises");
    parse_document_bytes(&bytes)
}

/// Writes `bytes` to `path` exactly as given (no trailing newline, no `.bak`): random temp
/// file in the same folder, flushed to disk, renamed over the target. The temp file is removed
/// on failure. Missing folders and the file get the default mode (export files).
pub fn write_exact(path: &Path, bytes: &[u8]) -> Result<(), SaveError> {
    let options = WriteOptions {
        parent: Parent::CreateDefault,
        backup: false,
        file_mode: FileMode::Default,
    };
    fernjo_fs::write_atomic(path, bytes, options).map_err(|source| SaveError {
        path: path.to_path_buf(),
        source,
    })
}

/// Checks what hand-edited files can get wrong. A repeated id (a copied block) gets a fresh
/// id so nothing is lost; a port of 0, which no operation can produce, makes the file unreadable.
fn sanitize(nodes: &mut [Node], seen: &mut HashSet<NodeId>) -> Result<(), String> {
    for node in nodes {
        if !seen.insert(node.id) {
            node.id = NodeId::new_v4();
            seen.insert(node.id);
        }
        if node.settings.port == Some(0) {
            return Err(format!("\"{}\" has the invalid port 0", node.name));
        }
        drop_invalid_rdp_settings(&mut node.settings);
        if let NodeKind::Folder { children } = &mut node.kind {
            sanitize(children, seen)?;
        }
    }
    Ok(())
}

/// A hand-edited RDP value that no operation can produce is dropped (the node inherits again)
/// instead of making the whole file unreadable: the settings are only a preference.
fn drop_invalid_rdp_settings(settings: &mut Settings) {
    if settings
        .rdp_color_depth
        .is_some_and(|d| !RDP_COLOR_DEPTHS.contains(&d))
    {
        settings.rdp_color_depth = None;
    }
    if settings.rdp_resolution.is_some_and(|r| !r.is_valid()) {
        settings.rdp_resolution = None;
    }
    if settings
        .domain
        .as_deref()
        .is_some_and(|d| !is_valid_domain(d))
    {
        settings.domain = None;
    }
}

/// Loads `path`, falling back to its `.bak` when the main file is missing or unreadable.
/// Writes nothing except renaming a corrupt main file out of the way after a successful
/// fallback.
pub fn load(path: &Path) -> Result<Loaded, LoadError> {
    let bak = with_suffix(path, ".bak");
    let reason = match read_file(path)? {
        Read::Parsed(tree) => {
            return Ok(Loaded { tree, notice: None });
        }
        Read::Missing => None,
        Read::Bad(reason) => Some(reason),
    };

    match read_file(&bak)? {
        Read::Parsed(tree) => {
            let bad_file = if reason.is_some() {
                let target = move_aside(path).map_err(|e| LoadError::Unreadable {
                    path: path.to_path_buf(),
                    reason: format!("could not move the unreadable file aside: {e}"),
                })?;
                Some(target)
            } else {
                None
            };
            Ok(Loaded {
                tree,
                notice: Some(Notice::RestoredFromBackup { bad_file }),
            })
        }
        Read::Missing => match reason {
            None => Ok(Loaded {
                tree: Tree::new(),
                notice: None,
            }),
            Some(reason) => Err(LoadError::Unreadable {
                path: path.to_path_buf(),
                reason,
            }),
        },
        Read::Bad(bak_reason) => Err(LoadError::Unreadable {
            path: path.to_path_buf(),
            reason: match reason {
                Some(r) => format!("{r}; the backup is unreadable too ({bak_reason})"),
                None => format!("the file is missing and the backup is unreadable ({bak_reason})"),
            },
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Protocol, Settings};
    use crate::tree::{NodePatch, Patch};
    use serde_json::{Value, json};
    use std::fs;
    use std::path::{Path, PathBuf};

    fn sample_tree(extra: &str) -> Tree {
        let mut t = Tree::new();
        let f = t.create_folder(None, None, "Homelab").unwrap();
        t.update(
            f,
            NodePatch {
                username: Patch::Set("ops".into()),
                port: Patch::Set(2200),
                ..NodePatch::default()
            },
        )
        .unwrap();
        let sub = t.create_folder(Some(f), None, "Proxmox").unwrap();
        t.create_connection(Some(sub), None, "pve-01", Protocol::Ssh, "pve-01.lab")
            .unwrap();
        t.create_connection(None, None, extra, Protocol::Rdp, "dc01.lab")
            .unwrap();
        t
    }

    fn files(dir: &Path) -> Vec<(String, Vec<u8>)> {
        let mut v: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|e| {
                let e = e.unwrap();
                (
                    e.file_name().to_string_lossy().into_owned(),
                    fs::read(e.path()).unwrap(),
                )
            })
            .collect();
        v.sort();
        v
    }

    #[test]
    fn load_refuses_a_file_over_the_cap_as_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        let main = main_path(dir.path());
        let file = fs::File::create(&main).unwrap();
        file.set_len(MAX_DATA_FILE_LEN + 1).unwrap();
        drop(file);
        match load(&main) {
            Err(LoadError::Unreadable { reason, .. }) => assert!(reason.contains("too large")),
            other => panic!("expected Unreadable, got {other:?}"),
        }
        assert_eq!(fs::metadata(&main).unwrap().len(), MAX_DATA_FILE_LEN + 1);
    }

    #[test]
    fn read_capped_accepts_up_to_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f");
        fs::write(&p, b"abc").unwrap();
        assert_eq!(read_capped(&p).unwrap(), b"abc");
        assert_eq!(
            read_capped(&dir.path().join("none")).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }

    fn main_path(dir: &Path) -> PathBuf {
        dir.join(FILE_NAME)
    }

    fn bak_path(dir: &Path) -> PathBuf {
        dir.join(format!("{FILE_NAME}.bak"))
    }

    #[cfg(unix)]
    #[test]
    fn data_files_are_owner_only_and_a_new_folder_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let base = tempfile::tempdir().unwrap();
        let data = base.path().join("fresh").join("fernjo");
        let file = main_path(&data);
        save(&file, &sample_tree("dc01")).unwrap();
        assert_eq!(mode(&data), 0o700, "a folder fernjo creates is private");
        assert_eq!(mode(&file), 0o600);
        save(&file, &sample_tree("dc02")).unwrap();
        assert_eq!(mode(&bak_path(&data)), 0o600);
        assert_eq!(mode(&file), 0o600);

        // a file an older build wrote with the default umask is tightened by the next save,
        // and so is the backup copied from it; an existing folder keeps its mode
        let old = main_path(base.path());
        fs::write(&old, "{}").unwrap();
        fs::set_permissions(&old, fs::Permissions::from_mode(0o644)).unwrap();
        let folder_mode = mode(base.path());
        save(&old, &sample_tree("dc03")).unwrap();
        assert_eq!(mode(&old), 0o600);
        assert_eq!(mode(&bak_path(base.path())), 0o600);
        assert_eq!(mode(base.path()), folder_mode);
    }

    #[test]
    fn save_then_load_round_trips_nested_tree_with_settings() {
        let dir = tempfile::tempdir().unwrap();
        let tree = sample_tree("dc01");
        save(&main_path(dir.path()), &tree).unwrap();
        let loaded = load(&main_path(dir.path())).unwrap();
        assert_eq!(loaded.tree, tree);
        assert_eq!(loaded.notice, None);
        let _ = Settings::default();
    }

    #[test]
    fn file_is_pretty_json_with_schema_version() {
        let dir = tempfile::tempdir().unwrap();
        save(&main_path(dir.path()), &sample_tree("dc01")).unwrap();
        let text = fs::read_to_string(main_path(dir.path())).unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["schemaVersion"], 3);
        assert_eq!(v["children"][0]["type"], "folder");
        assert_eq!(v["children"][0]["settings"]["port"], 2200);
        assert!(text.contains("\n  "));
    }

    #[test]
    fn save_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        save(&main_path(dir.path()), &sample_tree("dc01")).unwrap();
        let names: Vec<_> = files(dir.path()).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, [FILE_NAME]);
    }

    #[test]
    fn first_save_creates_no_bak() {
        let dir = tempfile::tempdir().unwrap();
        save(&main_path(dir.path()), &sample_tree("dc01")).unwrap();
        assert!(!bak_path(dir.path()).exists());
    }

    #[test]
    fn second_save_keeps_previous_content_as_bak() {
        let dir = tempfile::tempdir().unwrap();
        save(&main_path(dir.path()), &sample_tree("one")).unwrap();
        let first = fs::read(main_path(dir.path())).unwrap();
        save(&main_path(dir.path()), &sample_tree("two")).unwrap();
        assert_eq!(fs::read(bak_path(dir.path())).unwrap(), first);
        assert_ne!(fs::read(main_path(dir.path())).unwrap(), first);
    }

    #[test]
    fn save_creates_missing_parent_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a").join("b").join(FILE_NAME);
        save(&path, &Tree::new()).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn load_missing_file_gives_empty_tree_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = load(&main_path(dir.path())).unwrap();
        assert!(loaded.tree.children().is_empty());
        assert_eq!(loaded.notice, None);
        assert!(files(dir.path()).is_empty());
    }

    #[test]
    fn newer_schema_is_refused_and_files_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let text = json!({ "schemaVersion": 99, "children": [], "future": true }).to_string();
        fs::write(main_path(dir.path()), &text).unwrap();
        fs::write(bak_path(dir.path()), "{}").unwrap();
        let before = files(dir.path());
        let err = load(&main_path(dir.path())).unwrap_err();
        match err {
            LoadError::NewerSchema {
                found, supported, ..
            } => {
                assert_eq!(found, 99);
                assert_eq!(supported, 3);
            }
            other => panic!("expected NewerSchema, got {other:?}"),
        }
        assert_eq!(files(dir.path()), before);
    }

    #[test]
    fn corrupt_main_falls_back_to_bak() {
        let dir = tempfile::tempdir().unwrap();
        let first = sample_tree("one");
        save(&main_path(dir.path()), &first).unwrap();
        save(&main_path(dir.path()), &sample_tree("two")).unwrap();
        let bak_bytes = fs::read(bak_path(dir.path())).unwrap();
        fs::write(main_path(dir.path()), "{ not json").unwrap();

        let loaded = load(&main_path(dir.path())).unwrap();
        assert_eq!(loaded.tree, first);
        let Some(Notice::RestoredFromBackup {
            bad_file: Some(bad),
        }) = loaded.notice
        else {
            panic!("expected a restore notice with a bad file");
        };
        let name = bad.file_name().unwrap().to_string_lossy().into_owned();
        let suffix = name
            .strip_prefix("connections.json.corrupt-")
            .expect("bad file name");
        assert!(!suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()));
        assert_eq!(fs::read_to_string(&bad).unwrap(), "{ not json");
        assert_eq!(fs::read(bak_path(dir.path())).unwrap(), bak_bytes);
    }

    #[test]
    fn missing_main_with_bak_restores_with_notice_without_bad_file() {
        let dir = tempfile::tempdir().unwrap();
        let first = sample_tree("one");
        save(&main_path(dir.path()), &first).unwrap();
        save(&main_path(dir.path()), &sample_tree("two")).unwrap();
        fs::remove_file(main_path(dir.path())).unwrap();
        let loaded = load(&main_path(dir.path())).unwrap();
        assert_eq!(loaded.tree, first);
        assert_eq!(
            loaded.notice,
            Some(Notice::RestoredFromBackup { bad_file: None })
        );
    }

    #[test]
    fn both_unreadable_is_unreadable_error_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(main_path(dir.path()), "garbage main").unwrap();
        fs::write(bak_path(dir.path()), "garbage bak").unwrap();
        let before = files(dir.path());
        let err = load(&main_path(dir.path())).unwrap_err();
        assert!(matches!(err, LoadError::Unreadable { .. }), "{err:?}");
        assert_eq!(files(dir.path()), before);
    }

    #[test]
    fn corrupt_main_without_bak_is_unreadable_and_left_in_place() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(main_path(dir.path()), "garbage main").unwrap();
        let before = files(dir.path());
        let err = load(&main_path(dir.path())).unwrap_err();
        assert!(matches!(err, LoadError::Unreadable { .. }), "{err:?}");
        assert_eq!(files(dir.path()), before);
    }

    #[test]
    fn missing_or_non_integer_schema_version_counts_as_unreadable() {
        for text in [
            r#"{"children":[]}"#,
            r#"{"schemaVersion":"1","children":[]}"#,
        ] {
            let dir = tempfile::tempdir().unwrap();
            fs::write(main_path(dir.path()), text).unwrap();
            let err = load(&main_path(dir.path())).unwrap_err();
            assert!(
                matches!(err, LoadError::Unreadable { .. }),
                "{text}: {err:?}"
            );
        }
    }

    fn step_a(mut v: Value) -> Value {
        v["trail"] = json!(format!("{}a", v["trail"].as_str().unwrap_or("")));
        v
    }

    fn step_b(mut v: Value) -> Value {
        v["trail"] = json!(format!("{}b", v["trail"].as_str().unwrap_or("")));
        v
    }

    #[test]
    fn migrate_with_applies_steps_in_order() {
        let steps: [fn(Value) -> Value; 2] = [step_a, step_b];
        let out = migrate_with(json!({}), 1, &steps);
        assert_eq!(out["trail"], "ab");
        let out = migrate_with(json!({}), 2, &steps);
        assert_eq!(out["trail"], "b");
        let out = migrate_with(json!({}), 3, &steps);
        assert!(out.get("trail").is_none());
    }

    #[test]
    fn version_one_file_loads_and_is_written_as_the_current_version() {
        let dir = tempfile::tempdir().unwrap();
        let text = json!({ "schemaVersion": 1, "children": [{
            "id": "11111111-1111-4111-8111-111111111111", "type": "connection",
            "name": "x", "protocol": "ssh", "host": "h", "settings": { "port": 2200 }
        }] })
        .to_string();
        fs::write(main_path(dir.path()), text).unwrap();
        let loaded = load(&main_path(dir.path())).unwrap();
        assert_eq!(loaded.notice, None);
        assert_eq!(loaded.tree.children()[0].settings.port, Some(2200));
        save(&main_path(dir.path()), &loaded.tree).unwrap();
        let v: Value = serde_json::from_slice(&fs::read(main_path(dir.path())).unwrap()).unwrap();
        assert_eq!(v["schemaVersion"], 3);
        assert_eq!(v["children"][0]["settings"]["port"], 2200);
        // the previous (version 1) bytes are kept as the backup
        let bak: Value = serde_json::from_slice(&fs::read(bak_path(dir.path())).unwrap()).unwrap();
        assert_eq!(bak["schemaVersion"], 1);
    }

    #[test]
    fn credential_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let cred = crate::model::CredentialId::new_v4();
        let mut tree = Tree::new();
        let c = tree
            .create_connection(None, None, "c", Protocol::Ssh, "h")
            .unwrap();
        tree.update(
            c,
            NodePatch {
                credential: Patch::Set(cred),
                ..NodePatch::default()
            },
        )
        .unwrap();
        save(&main_path(dir.path()), &tree).unwrap();
        let v: Value = serde_json::from_slice(&fs::read(main_path(dir.path())).unwrap()).unwrap();
        assert_eq!(v["children"][0]["settings"]["credential"], cred.to_string());
        let loaded = load(&main_path(dir.path())).unwrap();
        assert_eq!(loaded.tree, tree);
        assert_eq!(loaded.tree.get(c).unwrap().settings.credential, Some(cred));
        // an unset credential is not written at all
        let mut plain = Tree::new();
        plain
            .create_connection(None, None, "p", Protocol::Ssh, "h")
            .unwrap();
        save(&main_path(dir.path()), &plain).unwrap();
        let text = fs::read_to_string(main_path(dir.path())).unwrap();
        assert!(!text.contains("credential"));
    }

    #[test]
    fn version_four_file_is_refused_as_newer_schema() {
        let dir = tempfile::tempdir().unwrap();
        let text = json!({ "schemaVersion": 4, "children": [] }).to_string();
        fs::write(main_path(dir.path()), &text).unwrap();
        let before = files(dir.path());
        let err = load(&main_path(dir.path())).unwrap_err();
        assert!(
            matches!(
                err,
                LoadError::NewerSchema {
                    found: 4,
                    supported: 3,
                    ..
                }
            ),
            "{err:?}"
        );
        assert_eq!(files(dir.path()), before);
    }

    #[test]
    fn version_two_file_loads_and_is_saved_as_three() {
        let dir = tempfile::tempdir().unwrap();
        let text = json!({ "schemaVersion": 2, "children": [{
            "id": "11111111-1111-4111-8111-111111111111", "type": "connection",
            "name": "x", "protocol": "rdp", "host": "h", "settings": { "port": 3390 }
        }] })
        .to_string();
        fs::write(main_path(dir.path()), text).unwrap();
        let loaded = load(&main_path(dir.path())).unwrap();
        assert_eq!(loaded.notice, None);
        assert_eq!(loaded.tree.children()[0].settings.port, Some(3390));
        save(&main_path(dir.path()), &loaded.tree).unwrap();
        let v: Value = serde_json::from_slice(&fs::read(main_path(dir.path())).unwrap()).unwrap();
        assert_eq!(v["schemaVersion"], 3);
        let bak: Value = serde_json::from_slice(&fs::read(bak_path(dir.path())).unwrap()).unwrap();
        assert_eq!(bak["schemaVersion"], 2);
    }

    #[test]
    fn rdp_settings_round_trip_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut tree = Tree::new();
        let c = tree
            .create_connection(None, None, "c", Protocol::Rdp, "h")
            .unwrap();
        tree.update(
            c,
            NodePatch {
                domain: Patch::Set("CORP".into()),
                rdp_resolution: Patch::Set(crate::model::RdpResolution::Fixed {
                    width: 1920,
                    height: 1080,
                }),
                rdp_color_depth: Patch::Set(16),
                rdp_clipboard: Patch::Set(false),
                ..NodePatch::default()
            },
        )
        .unwrap();
        save(&main_path(dir.path()), &tree).unwrap();
        let v: Value = serde_json::from_slice(&fs::read(main_path(dir.path())).unwrap()).unwrap();
        let settings = &v["children"][0]["settings"];
        assert_eq!(settings["domain"], "CORP");
        assert_eq!(
            settings["rdpResolution"],
            json!({ "mode": "fixed", "width": 1920, "height": 1080 })
        );
        assert_eq!(settings["rdpColorDepth"], 16);
        assert_eq!(settings["rdpClipboard"], false);
        let loaded = load(&main_path(dir.path())).unwrap();
        assert_eq!(loaded.tree, tree);
        // unset values are not written at all
        let mut plain = Tree::new();
        plain
            .create_connection(None, None, "p", Protocol::Rdp, "h")
            .unwrap();
        save(&main_path(dir.path()), &plain).unwrap();
        let text = fs::read_to_string(main_path(dir.path())).unwrap();
        assert!(
            !text.contains("rdpResolution") && !text.contains("rdpColorDepth"),
            "{text}"
        );
        assert!(
            !text.contains("rdpClipboard") && !text.contains("domain"),
            "{text}"
        );
    }

    #[test]
    fn an_invalid_stored_rdp_value_is_dropped_on_load() {
        let tree = parse_document(json!({ "schemaVersion": 3, "children": [{
            "id": "11111111-1111-4111-8111-111111111111", "type": "connection",
            "name": "x", "protocol": "rdp", "host": "h", "settings": {
                "rdpColorDepth": 7,
                "rdpResolution": { "mode": "fixed", "width": 5, "height": 600 },
                "domain": "bad\u{7}domain",
                "rdpClipboard": false
            }
        }] }))
        .unwrap();
        let s = &tree.children()[0].settings;
        assert_eq!(s.rdp_color_depth, None);
        assert_eq!(s.rdp_resolution, None);
        assert_eq!(s.domain, None);
        // a valid neighbour is kept
        assert_eq!(s.rdp_clipboard, Some(false));
    }

    #[test]
    fn migration_chain_has_a_one_to_two_and_a_two_to_three_step() {
        assert_eq!(MIGRATIONS.len(), CURRENT_VERSION as usize - 1);
        // the steps run on the typed document (M11 review #28: no `Value` on the load path)
        let file = FileIn {
            children: sample_tree("a").children().to_vec(),
        };
        // the 1 -> 2 step only exists to make older builds refuse the file: nothing is rewritten
        assert_eq!(migrate_with(file.clone(), 1, &MIGRATIONS), file);
        assert_eq!(migrate_with(file.clone(), 2, &MIGRATIONS), file);
        // the 2 -> 3 step only exists so that v0.1.1 refuses the file instead of dropping the
        // RDP settings on its next save
        assert_eq!(migrate_with(file.clone(), 3, &MIGRATIONS), file);
    }

    #[test]
    fn save_into_path_below_a_regular_file_fails() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("plain");
        fs::write(&file, "x").unwrap();
        let err = save(&file.join("sub").join(FILE_NAME), &Tree::new()).unwrap_err();
        assert!(err.to_string().contains("plain"), "{err}");
    }

    #[test]
    fn duplicate_ids_in_a_hand_edited_file_are_made_unique() {
        let dir = tempfile::tempdir().unwrap();
        let id = "11111111-1111-4111-8111-111111111111";
        let node = |name: &str| json!({ "id": id, "type": "connection", "name": name, "protocol": "ssh", "host": "h" });
        let text = json!({
            "schemaVersion": 1,
            "children": [
                node("a"),
                { "id": id, "type": "folder", "name": "f", "children": [node("b")] }
            ]
        })
        .to_string();
        fs::write(main_path(dir.path()), text).unwrap();

        let loaded = load(&main_path(dir.path())).unwrap();
        assert_eq!(loaded.notice, None);
        let mut ids = Vec::new();
        fn collect(nodes: &[crate::model::Node], ids: &mut Vec<crate::model::NodeId>) {
            for n in nodes {
                ids.push(n.id);
                if let crate::model::NodeKind::Folder { children } = &n.kind {
                    collect(children, ids);
                }
            }
        }
        collect(loaded.tree.children(), &mut ids);
        assert_eq!(ids.len(), 3);
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), 3);
        // the first occurrence keeps its id, every name survives
        assert_eq!(loaded.tree.children()[0].id.to_string(), id);
        assert_eq!(loaded.tree.children()[1].name, "f");
    }

    #[test]
    fn port_0_in_the_file_is_unreadable_and_falls_back_to_bak() {
        let dir = tempfile::tempdir().unwrap();
        let first = sample_tree("one");
        save(&main_path(dir.path()), &first).unwrap();
        save(&main_path(dir.path()), &sample_tree("two")).unwrap();
        let text = json!({
            "schemaVersion": 1,
            "children": [{
                "id": "11111111-1111-4111-8111-111111111111", "type": "connection",
                "name": "x", "protocol": "ssh", "host": "h", "settings": { "port": 0 }
            }]
        })
        .to_string();
        fs::write(main_path(dir.path()), text).unwrap();
        let loaded = load(&main_path(dir.path())).unwrap();
        assert_eq!(loaded.tree, first);
        assert!(matches!(
            loaded.notice,
            Some(Notice::RestoredFromBackup { bad_file: Some(_) })
        ));
    }

    #[test]
    fn a_utf8_bom_is_not_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let tree = sample_tree("dc01");
        save(&main_path(dir.path()), &tree).unwrap();
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend(fs::read(main_path(dir.path())).unwrap());
        fs::write(main_path(dir.path()), bytes).unwrap();
        let loaded = load(&main_path(dir.path())).unwrap();
        assert_eq!(loaded.tree, tree);
        assert_eq!(loaded.notice, None);
        assert_eq!(files(dir.path()).len(), 1);
    }

    #[test]
    fn document_round_trips_through_parse_document() {
        let tree = sample_tree("dc01");
        let doc = document(&tree);
        assert_eq!(doc["schemaVersion"], CURRENT_VERSION);
        assert!(doc["children"].is_array());
        assert_eq!(parse_document(doc).unwrap(), tree);
    }

    #[test]
    fn document_is_what_save_writes() {
        let dir = tempfile::tempdir().unwrap();
        let tree = sample_tree("dc01");
        save(&main_path(dir.path()), &tree).unwrap();
        let on_disk: Value =
            serde_json::from_slice(&fs::read(main_path(dir.path())).unwrap()).unwrap();
        assert_eq!(on_disk, document(&tree));
    }

    #[test]
    fn saved_file_keeps_schema_version_first_and_declaration_key_order() {
        let dir = tempfile::tempdir().unwrap();
        let mut tree = Tree::new();
        let c = tree
            .create_connection(None, None, "c", Protocol::Ssh, "h")
            .unwrap();
        tree.update(
            c,
            NodePatch {
                username: Patch::Set("u".into()),
                ..NodePatch::default()
            },
        )
        .unwrap();
        save(&main_path(dir.path()), &tree).unwrap();
        let text = fs::read_to_string(main_path(dir.path())).unwrap();
        assert!(
            text.starts_with("{\n  \"schemaVersion\": 3,\n  \"children\": ["),
            "{text}"
        );
        let pos = |key: &str| text.find(&format!("\"{key}\"")).unwrap();
        let order = [
            "id",
            "name",
            "description",
            "settings",
            "type",
            "protocol",
            "host",
        ];
        let positions: Vec<_> = order.iter().map(|k| pos(k)).collect();
        assert!(positions.windows(2).all(|w| w[0] < w[1]), "{text}");
    }

    #[test]
    fn a_leftover_fixed_name_temp_file_is_removed_after_a_save() {
        let dir = tempfile::tempdir().unwrap();
        let main = main_path(dir.path());
        let leftover = with_suffix(&main, ".tmp");
        fs::write(&leftover, "old build leftover").unwrap();
        save(&main, &sample_tree("dc01")).unwrap();
        assert!(!leftover.exists());
        let names: Vec<_> = files(dir.path()).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, [FILE_NAME]);
    }

    #[test]
    fn a_directory_at_the_fixed_temp_name_is_left_alone_by_a_save() {
        let dir = tempfile::tempdir().unwrap();
        let main = main_path(dir.path());
        let planted = with_suffix(&main, ".tmp");
        fs::create_dir(&planted).unwrap();
        save(&main, &sample_tree("dc01")).unwrap();
        assert!(planted.is_dir());
        assert!(main.is_file());
    }

    #[test]
    fn write_exact_uses_a_random_temp_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.json");
        // a directory at the old fixed temp name would have made the write fail
        fs::create_dir(with_suffix(&path, ".tmp")).unwrap();
        write_exact(&path, b"data").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"data");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_at_the_bak_name_is_not_followed_by_a_save() {
        let dir = tempfile::tempdir().unwrap();
        let main = main_path(dir.path());
        save(&main, &sample_tree("one")).unwrap();
        let victim = dir.path().join("victim");
        fs::write(&victim, "precious").unwrap();
        std::os::unix::fs::symlink(&victim, bak_path(dir.path())).unwrap();
        save(&main, &sample_tree("two")).unwrap();
        assert_eq!(fs::read(&victim).unwrap(), b"precious");
        assert!(
            !fs::symlink_metadata(bak_path(dir.path()))
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn write_exact_removes_temp_file_on_failure() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        fs::create_dir(&target).unwrap();
        assert!(write_exact(&target, b"x").is_err());
        // `files` cannot read a directory, so list names only
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["target"]);
    }

    #[test]
    fn parse_document_refuses_newer_schema_with_found_and_supported() {
        let err = parse_document(json!({ "schemaVersion": 99, "children": [] })).unwrap_err();
        assert_eq!(
            err,
            DocumentError::NewerSchema {
                found: 99,
                supported: CURRENT_VERSION
            }
        );
        assert_eq!(err.code(), "newer_schema");
        let err = parse_document(json!({ "children": [] })).unwrap_err();
        assert_eq!(err.code(), "invalid_document");
    }

    #[test]
    fn parse_document_gives_repeated_ids_fresh_ids() {
        let id = "11111111-1111-4111-8111-111111111111";
        let node = |name: &str| json!({ "id": id, "type": "connection", "name": name, "protocol": "ssh", "host": "h" });
        let tree = parse_document(json!({
            "schemaVersion": 2,
            "children": [node("a"), node("b")]
        }))
        .unwrap();
        assert_eq!(tree.children().len(), 2);
        assert_ne!(tree.children()[0].id, tree.children()[1].id);
        assert_eq!(tree.children()[0].id.to_string(), id);
    }

    #[test]
    fn write_exact_has_no_newline_no_bak_and_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.json");
        write_exact(&path, b"one").unwrap();
        write_exact(&path, b"two").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"two");
        let names: Vec<_> = files(dir.path()).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["out.json"]);
    }

    #[test]
    fn notice_json_is_tagged_camel_case() {
        let v = serde_json::to_value(Notice::RestoredFromBackup {
            bad_file: Some(PathBuf::from("x")),
        })
        .unwrap();
        assert_eq!(v, json!({ "kind": "restoredFromBackup", "badFile": "x" }));
    }

    #[test]
    fn sync_conflicts_notice_json_is_tagged_camel_case() {
        let v = serde_json::to_value(Notice::SyncConflicts {
            files: vec!["a (conflicted copy).json".into()],
        })
        .unwrap();
        assert_eq!(
            v,
            json!({ "kind": "syncConflicts", "files": ["a (conflicted copy).json"] })
        );
    }

    #[test]
    fn settings_unreadable_notice_json_is_tagged_camel_case() {
        let v = serde_json::to_value(Notice::SettingsUnreadable {
            message: "bad".into(),
        })
        .unwrap();
        assert_eq!(v, json!({ "kind": "settingsUnreadable", "message": "bad" }));
    }

    #[test]
    fn notice_serialises_as_device_folder_inside_data_dir() {
        let v = serde_json::to_value(Notice::DeviceFolderInsideDataDir {
            path: PathBuf::from("dev"),
        })
        .unwrap();
        assert_eq!(
            v,
            json!({ "kind": "deviceFolderInsideDataDir", "path": "dev" })
        );
    }
}
