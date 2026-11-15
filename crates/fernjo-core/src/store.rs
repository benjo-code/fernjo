//! A `Tree` bound to its file: every successful mutation is saved before it is visible.

use std::path::{Path, PathBuf};

use crate::error::{LoadError, StoreError, TreeError};
use crate::model::{NodeId, Protocol};
use crate::persist::{self, FILE_NAME, Notice};
use crate::tree::{NodePatch, Tree};

/// A tree plus the file it is saved to.
#[derive(Debug)]
pub struct Store {
    path: PathBuf,
    tree: Tree,
}

/// A freshly opened store and anything the user should be told about.
#[derive(Debug)]
pub struct Opened {
    pub store: Store,
    pub notice: Option<Notice>,
}

impl Store {
    /// Opens (or starts) the store in `dir`. A missing file means an empty tree and is
    /// only created by the first mutation. After a restore from `.bak` the main file is
    /// written again (best effort: a read-only folder still lets the tree be viewed).
    pub fn open(dir: &Path) -> Result<Opened, LoadError> {
        let path = dir.join(FILE_NAME);
        let loaded = persist::load(&path)?;
        if loaded.notice.is_some() {
            let _ = persist::save(&path, &loaded.tree);
        }
        Ok(Opened {
            store: Store {
                path,
                tree: loaded.tree,
            },
            notice: loaded.notice,
        })
    }

    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Applies `f` to a copy, saves it and only then makes it current, so memory never
    /// diverges from disk.
    fn mutate<T>(
        &mut self,
        f: impl FnOnce(&mut Tree) -> Result<T, TreeError>,
    ) -> Result<T, StoreError> {
        let mut next = self.tree.clone();
        let out = f(&mut next)?;
        persist::save(&self.path, &next)?;
        self.tree = next;
        Ok(out)
    }

    pub fn create_folder(
        &mut self,
        parent: Option<NodeId>,
        index: Option<usize>,
        name: &str,
    ) -> Result<NodeId, StoreError> {
        self.mutate(|t| t.create_folder(parent, index, name))
    }

    pub fn create_connection(
        &mut self,
        parent: Option<NodeId>,
        index: Option<usize>,
        name: &str,
        protocol: Protocol,
        host: &str,
    ) -> Result<NodeId, StoreError> {
        self.mutate(|t| t.create_connection(parent, index, name, protocol, host))
    }

    pub fn update(&mut self, id: NodeId, patch: NodePatch) -> Result<(), StoreError> {
        self.mutate(|t| t.update(id, patch))
    }

    pub fn move_node(
        &mut self,
        id: NodeId,
        new_parent: Option<NodeId>,
        index: usize,
    ) -> Result<(), StoreError> {
        self.mutate(|t| t.move_node(id, new_parent, index))
    }

    pub fn duplicate(&mut self, id: NodeId) -> Result<NodeId, StoreError> {
        self.mutate(|t| t.duplicate(id))
    }

    pub fn delete(&mut self, id: NodeId) -> Result<usize, StoreError> {
        self.mutate(|t| t.delete(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Protocol;
    use crate::persist::{self, FILE_NAME};
    use crate::tree::{NodePatch, Patch};
    use std::fs;

    fn open(dir: &Path) -> Store {
        Store::open(dir).unwrap().store
    }

    #[test]
    fn open_empty_dir_creates_nothing_until_first_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let opened = Store::open(dir.path()).unwrap();
        assert!(opened.notice.is_none());
        assert!(opened.store.tree().children().is_empty());
        assert!(fs::read_dir(dir.path()).unwrap().next().is_none());
        assert_eq!(opened.store.path(), dir.path().join(FILE_NAME));
    }

    #[test]
    fn mutation_is_persisted_and_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        let f = store.create_folder(None, None, "Homelab").unwrap();
        store
            .create_connection(Some(f), None, "pve-01", Protocol::Ssh, "pve-01.lab")
            .unwrap();
        store
            .create_connection(None, None, "dc01", Protocol::Rdp, "dc01.lab")
            .unwrap();

        let again = open(dir.path());
        let names: Vec<_> = again
            .tree()
            .children()
            .iter()
            .map(|n| n.name.as_str())
            .collect();
        assert_eq!(names, ["Homelab", "dc01"]);
        assert_eq!(again.tree(), store.tree());
        assert_eq!(again.tree().descendant_count(f).unwrap(), 1);
    }

    #[test]
    fn every_mutation_kind_is_saved() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        let a = store.create_folder(None, None, "a").unwrap();
        let b = store.create_folder(None, None, "b").unwrap();
        store
            .update(
                a,
                NodePatch {
                    name: Some("renamed".into()),
                    username: Patch::Set("ops".into()),
                    ..NodePatch::default()
                },
            )
            .unwrap();
        store.move_node(b, Some(a), 0).unwrap();
        let copy = store.duplicate(a).unwrap();
        assert_eq!(store.delete(copy).unwrap(), 2);
        assert_eq!(open(dir.path()).tree(), store.tree());
    }

    #[test]
    fn failed_save_rolls_back_memory() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("plain");
        fs::write(&blocker, "x").unwrap();
        let mut tree = Tree::new();
        tree.create_folder(None, None, "kept").unwrap();
        let mut store = Store {
            path: blocker.join(FILE_NAME),
            tree: tree.clone(),
        };

        let err = store.create_folder(None, None, "lost").unwrap_err();
        assert!(matches!(err, StoreError::Save(_)), "{err:?}");
        assert_eq!(err.code(), "save_failed");
        assert_eq!(store.tree(), &tree);
        assert_eq!(store.tree().children().len(), 1);
    }

    #[test]
    fn tree_error_does_not_touch_disk() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        let err = store.create_folder(None, None, "  ").unwrap_err();
        assert_eq!(err.code(), "empty_name");
        assert!(!dir.path().join(FILE_NAME).exists());
    }

    #[test]
    fn rejected_move_leaves_file_bytes_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        let a = store.create_folder(None, None, "a").unwrap();
        let before = fs::read(store.path()).unwrap();
        let err = store.move_node(a, Some(a), 0).unwrap_err();
        assert_eq!(err.code(), "invalid_move");
        assert_eq!(fs::read(store.path()).unwrap(), before);
    }

    #[test]
    fn open_after_restore_recreates_main_file() {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join(FILE_NAME);
        let mut store = open(dir.path());
        store.create_folder(None, None, "one").unwrap();
        store.create_folder(None, None, "two").unwrap();
        // .bak now holds the tree with only "one"
        fs::write(&main, "garbage").unwrap();

        let opened = Store::open(dir.path()).unwrap();
        assert!(opened.notice.is_some());
        assert_eq!(opened.store.tree().children().len(), 1);
        let reloaded = persist::load(&main).unwrap();
        assert_eq!(&reloaded.tree, opened.store.tree());
        assert!(reloaded.notice.is_none());
    }

    #[test]
    fn open_propagates_newer_schema_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join(FILE_NAME),
            r#"{"schemaVersion":99,"children":[]}"#,
        )
        .unwrap();
        let err = Store::open(dir.path()).err().unwrap();
        assert!(matches!(err, LoadError::NewerSchema { found: 99, .. }));
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
