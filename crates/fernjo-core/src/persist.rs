//! `connections.json`: format, atomic write, backup and load fallback.

use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{LoadError, SaveError};
use crate::model::{Node, NodeId, NodeKind};
use crate::tree::Tree;

/// File format version this build writes and understands.
pub const CURRENT_VERSION: u32 = 1;
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
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name: OsString = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Writes `tree` atomically: temp file in the same folder, flushed to disk, the current
/// file copied to `.bak`, then the temp file renamed over the target.
pub fn save(path: &Path, tree: &Tree) -> Result<(), SaveError> {
    let fail = |source: io::Error| SaveError {
        path: path.to_path_buf(),
        source,
    };
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(dir) = dir {
        fs::create_dir_all(dir).map_err(fail)?;
    }
    let json = serde_json::to_string_pretty(&FileOut {
        schema_version: CURRENT_VERSION,
        children: tree.children(),
    })
    .map_err(|e| fail(io::Error::other(e)))?;

    let tmp = with_suffix(path, ".tmp");
    let result = (|| -> io::Result<()> {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(json.as_bytes())?;
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

/// Upgrades a parsed file from version `from` using `steps` (step `i` upgrades
/// version `i + 1` to `i + 2`).
pub fn migrate_with(value: Value, from: u32, steps: &[fn(Value) -> Value]) -> Value {
    let skip = from.saturating_sub(1) as usize;
    steps.iter().skip(skip).fold(value, |v, step| step(v))
}

fn migrate(value: Value, from: u32) -> Value {
    migrate_with(value, from, &[])
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
    let Some(version) = value.get("schemaVersion").and_then(Value::as_u64) else {
        return Ok(Read::Bad("missing or invalid schemaVersion".to_string()));
    };
    let version = u32::try_from(version).unwrap_or(u32::MAX);
    if version > CURRENT_VERSION {
        return Err(LoadError::NewerSchema {
            found: version,
            supported: CURRENT_VERSION,
            path: path.to_path_buf(),
        });
    }
    if version == 0 {
        return Ok(Read::Bad("invalid schemaVersion 0".to_string()));
    }
    match serde_json::from_value::<FileIn>(migrate(value, version)) {
        Ok(mut file) => match sanitize(&mut file.children, &mut HashSet::new()) {
            Ok(()) => Ok(Read::Parsed(Tree::from_children(file.children))),
            Err(reason) => Ok(Read::Bad(reason)),
        },
        Err(e) => Ok(Read::Bad(e.to_string())),
    }
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
                let secs = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let target = with_suffix(path, &format!(".corrupt-{secs}"));
                fs::rename(path, &target).map_err(|e| LoadError::Unreadable {
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
        assert_eq!(v["schemaVersion"], 1);
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
                assert_eq!(supported, 1);
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
    fn notice_json_is_tagged_camel_case() {
        let v = serde_json::to_value(Notice::RestoredFromBackup {
            bad_file: Some(PathBuf::from("x")),
        })
        .unwrap();
        assert_eq!(v, json!({ "kind": "restoredFromBackup", "badFile": "x" }));
    }
}
