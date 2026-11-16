//! `connections.json`: format, atomic write, backup and load fallback.

use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{DocumentError, LoadError, SaveError};
use crate::model::{Node, NodeId, NodeKind};
use crate::tree::Tree;

/// File format version this build writes and understands.
pub const CURRENT_VERSION: u32 = 2;
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

#[derive(Deserialize)]
struct FileIn {
    children: Vec<Node>,
}

/// `<path>.<suffix>` (for example `connections.json.bak`).
pub(crate) fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name: OsString = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// The JSON document `save` writes for `tree`: `schemaVersion` and `children`.
pub fn document(tree: &Tree) -> Value {
    serde_json::to_value(FileOut {
        schema_version: CURRENT_VERSION,
        children: tree.children(),
    })
    .expect("a tree always serialises")
}

/// Writes `tree` atomically: temp file in the same folder, flushed to disk, the current
/// file copied to `.bak`, then the temp file renamed over the target.
pub fn save(path: &Path, tree: &Tree) -> Result<(), SaveError> {
    let json = serde_json::to_string_pretty(&document(tree)).map_err(|e| SaveError {
        path: path.to_path_buf(),
        source: io::Error::other(e),
    })?;
    write_atomic(path, json.as_bytes())
}

/// Shared by every file in the data and config folders: creates the parent folder, writes a
/// temp file next to `path`, flushes it to disk, copies the current file to `.bak`, then
/// renames the temp file over `path`. `contents` gets a trailing newline.
pub(crate) fn write_atomic(path: &Path, contents: &[u8]) -> Result<(), SaveError> {
    let fail = |source: io::Error| SaveError {
        path: path.to_path_buf(),
        source,
    };
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(dir) = dir {
        fs::create_dir_all(dir).map_err(fail)?;
    }
    let tmp = with_suffix(path, ".tmp");
    let result = (|| -> io::Result<()> {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(contents)?;
        file.write_all(b"\n")?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        if path.exists() {
            fs::copy(path, with_suffix(path, ".bak"))?;
        }
        fs::rename(&tmp, path)
    })();
    if let Err(source) = result {
        let _ = fs::remove_file(&tmp);
        return Err(fail(source));
    }
    Ok(())
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
pub fn migrate_with(value: Value, from: u32, steps: &[fn(Value) -> Value]) -> Value {
    let skip = from.saturating_sub(1) as usize;
    steps.iter().skip(skip).fold(value, |v, step| step(v))
}

/// Upgrade steps; step `i` upgrades version `i + 1` to `i + 2`.
const MIGRATIONS: [fn(Value) -> Value; 1] = [one_to_two];

/// 1 -> 2: schema 2 adds the optional `credential` setting, so nothing is rewritten. The
/// version still has to change: an older build would silently drop `credential` on its next
/// save, so it must refuse the file as a newer schema instead.
fn one_to_two(value: Value) -> Value {
    value
}

fn migrate(value: Value, from: u32) -> Value {
    migrate_with(value, from, &MIGRATIONS)
}

enum Read {
    Missing,
    Parsed(Tree),
    Bad(String),
}

fn read_file(path: &Path) -> Result<Read, LoadError> {
    let bytes = match fs::read(path) {
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
    let value: Value = match serde_json::from_slice(bytes) {
        Ok(v) => v,
        Err(e) => return Ok(Read::Bad(e.to_string())),
    };
    match parse_document(value) {
        Ok(tree) => Ok(Read::Parsed(tree)),
        Err(DocumentError::NewerSchema { found, supported }) => Err(LoadError::NewerSchema {
            found,
            supported,
            path: path.to_path_buf(),
        }),
        Err(DocumentError::Invalid(reason)) => Ok(Read::Bad(reason)),
    }
}

/// Turns a parsed `connections.json` document into a tree: checks `schemaVersion`, migrates
/// older versions and runs `sanitize`. Shared by loading a file and by importing a document.
pub fn parse_document(value: Value) -> Result<Tree, DocumentError> {
    let Some(version) = value.get("schemaVersion").and_then(Value::as_u64) else {
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
    let mut file: FileIn = serde_json::from_value(migrate(value, version))
        .map_err(|e| DocumentError::Invalid(e.to_string()))?;
    sanitize(&mut file.children, &mut HashSet::new()).map_err(DocumentError::Invalid)?;
    Ok(Tree::from_children(file.children))
}

/// Writes `bytes` to `path` exactly as given (no trailing newline, no `.bak`): temp file in
/// the same folder, flushed to disk, renamed over the target. The temp file is removed on
/// failure.
pub fn write_exact(path: &Path, bytes: &[u8]) -> Result<(), SaveError> {
    let fail = |source: io::Error| SaveError {
        path: path.to_path_buf(),
        source,
    };
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(dir) = dir {
        fs::create_dir_all(dir).map_err(fail)?;
    }
    let tmp = with_suffix(path, ".tmp");
    let result = (|| -> io::Result<()> {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)
    })();
    if let Err(source) = result {
        let _ = fs::remove_file(&tmp);
        return Err(fail(source));
    }
    Ok(())
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
        if let NodeKind::Folder { children } = &mut node.kind {
            sanitize(children, seen)?;
        }
    }
    Ok(())
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

    fn main_path(dir: &Path) -> PathBuf {
        dir.join(FILE_NAME)
    }

    fn bak_path(dir: &Path) -> PathBuf {
        dir.join(format!("{FILE_NAME}.bak"))
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
        assert_eq!(v["schemaVersion"], 2);
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
                assert_eq!(supported, 2);
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
    fn version_one_file_loads_and_is_written_as_version_two() {
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
        assert_eq!(v["schemaVersion"], 2);
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
    fn version_three_file_is_refused_as_newer_schema() {
        let dir = tempfile::tempdir().unwrap();
        let text = json!({ "schemaVersion": 3, "children": [] }).to_string();
        fs::write(main_path(dir.path()), &text).unwrap();
        let before = files(dir.path());
        let err = load(&main_path(dir.path())).unwrap_err();
        assert!(
            matches!(
                err,
                LoadError::NewerSchema {
                    found: 3,
                    supported: 2,
                    ..
                }
            ),
            "{err:?}"
        );
        assert_eq!(files(dir.path()), before);
    }

    #[test]
    fn migration_chain_has_a_one_to_two_step() {
        assert_eq!(MIGRATIONS.len(), CURRENT_VERSION as usize - 1);
        let v1 = json!({ "schemaVersion": 1, "children": [{ "name": "a" }] });
        // the 1 -> 2 step only exists to make older builds refuse the file: nothing is rewritten
        assert_eq!(migrate_with(v1.clone(), 1, &MIGRATIONS), v1);
        let v2 = json!({ "schemaVersion": 2, "children": [] });
        assert_eq!(migrate_with(v2.clone(), 2, &MIGRATIONS), v2);
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
}
