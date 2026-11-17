//! A `Tree` bound to its file: every successful mutation is saved before it is visible.

use std::path::{Path, PathBuf};

use crate::error::{LoadError, StoreError, TreeError};
use crate::model::{CredentialId, Node, NodeId, Protocol};
use crate::persist::{self, FILE_NAME, Notice};
use crate::tree::{MAX_TREE_DEPTH, NodePatch, Tree};

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

    /// Appends a whole subtree at the end of `parent` and saves it.
    pub fn graft(&mut self, parent: Option<NodeId>, node: Node) -> Result<NodeId, StoreError> {
        self.mutate(|t| t.graft(parent, node))
    }

    /// Replaces the whole tree and saves it (the previous file becomes the `.bak`). Refuses
    /// with `too_deep` a tree that is nested below `MAX_TREE_DEPTH` levels (it might not load
    /// again); nothing is written then.
    pub fn replace_tree(&mut self, tree: Tree) -> Result<(), StoreError> {
        if tree.depth() > MAX_TREE_DEPTH {
            return Err(TreeError::TooDeep.into());
        }
        persist::save(&self.path, &tree)?;
        self.tree = tree;
        Ok(())
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

    /// Removes every local reference to the vault entry `id`; returns how many nodes had
    /// one. Saves only if something changed, so an unused entry costs no write and no `.bak`.
    pub fn clear_credential(&mut self, id: CredentialId) -> Result<usize, StoreError> {
        let mut next = self.tree.clone();
        let cleared = next.clear_credential(id);
        if cleared > 0 {
            persist::save(&self.path, &next)?;
            self.tree = next;
        }
        Ok(cleared)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CredentialId, Protocol};
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
    fn clear_credential_is_persisted() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        let cred = CredentialId::new_v4();
        let a = store.create_folder(None, None, "a").unwrap();
        let b = store
            .create_connection(Some(a), None, "b", Protocol::Ssh, "h")
            .unwrap();
        for id in [a, b] {
            store
                .update(
                    id,
                    NodePatch {
                        credential: Patch::Set(cred),
                        ..NodePatch::default()
                    },
                )
                .unwrap();
        }
        assert_eq!(store.clear_credential(cred).unwrap(), 2);
        assert!(store.tree().nodes_using_credential(cred).is_empty());
        let again = open(dir.path());
        assert_eq!(again.tree(), store.tree());
        assert!(again.tree().nodes_using_credential(cred).is_empty());
    }

    #[test]
    fn clear_credential_with_no_users_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        store.create_folder(None, None, "a").unwrap();
        store.create_folder(None, None, "b").unwrap();
        let bak = dir.path().join(format!("{FILE_NAME}.bak"));
        let main_before = fs::read(store.path()).unwrap();
        let bak_before = fs::read(&bak).unwrap();
        assert_ne!(main_before, bak_before);
        assert_eq!(store.clear_credential(CredentialId::new_v4()).unwrap(), 0);
        // a save would have copied the main file over the .bak
        assert_eq!(fs::read(store.path()).unwrap(), main_before);
        assert_eq!(fs::read(&bak).unwrap(), bak_before);
    }

    fn leaf(name: &str) -> crate::model::Node {
        crate::model::Node {
            id: NodeId::new_v4(),
            name: name.into(),
            description: String::new(),
            settings: crate::model::Settings::default(),
            kind: crate::model::NodeKind::Connection {
                protocol: Protocol::Ssh,
                host: "h".into(),
            },
        }
    }

    fn broken_store(tree: Tree) -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("plain");
        fs::write(&blocker, "x").unwrap();
        let store = Store {
            path: blocker.join(FILE_NAME),
            tree,
        };
        (dir, store)
    }

    #[test]
    fn graft_is_persisted() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        let f = store.create_folder(None, None, "f").unwrap();
        let node = leaf("imported");
        let id = node.id;
        assert_eq!(store.graft(Some(f), node).unwrap(), id);
        assert!(store.tree().get(id).is_some());
        assert_eq!(open(dir.path()).tree(), store.tree());
    }

    #[test]
    fn failed_save_rolls_back_graft() {
        let mut tree = Tree::new();
        tree.create_folder(None, None, "kept").unwrap();
        let (_dir, mut store) = broken_store(tree.clone());
        let err = store.graft(None, leaf("lost")).unwrap_err();
        assert_eq!(err.code(), "save_failed");
        assert_eq!(store.tree(), &tree);
    }

    #[test]
    fn replace_tree_is_persisted_and_keeps_bak() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        store.create_folder(None, None, "old").unwrap();
        let old_bytes = fs::read(store.path()).unwrap();
        let mut next = Tree::new();
        next.create_folder(None, None, "new").unwrap();
        store.replace_tree(next.clone()).unwrap();
        assert_eq!(store.tree(), &next);
        assert_eq!(open(dir.path()).tree(), &next);
        let bak = dir.path().join(format!("{FILE_NAME}.bak"));
        assert_eq!(fs::read(bak).unwrap(), old_bytes);
    }

    #[test]
    fn failed_save_rolls_back_replace_tree() {
        let mut tree = Tree::new();
        tree.create_folder(None, None, "kept").unwrap();
        let (_dir, mut store) = broken_store(tree.clone());
        let mut next = Tree::new();
        next.create_folder(None, None, "lost").unwrap();
        let err = store.replace_tree(next).unwrap_err();
        assert_eq!(err.code(), "save_failed");
        assert_eq!(store.tree(), &tree);
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

    // ---- depth limit ----------------------------------------------------

    fn chain(levels: usize) -> Node {
        use crate::model::{NodeKind, Settings};
        // the connection has a setting, so its `settings` object is written (one level more)
        let mut node = Node {
            id: NodeId::new_v4(),
            name: "deepest".into(),
            description: String::new(),
            settings: Settings {
                port: Some(22),
                ..Settings::default()
            },
            kind: NodeKind::Connection {
                protocol: Protocol::Ssh,
                host: "h".into(),
            },
        };
        for i in 0..levels {
            node = Node {
                id: NodeId::new_v4(),
                name: format!("level-{i}"),
                description: String::new(),
                settings: Settings::default(),
                kind: NodeKind::Folder {
                    children: vec![node],
                },
            };
        }
        node
    }

    #[test]
    fn a_tree_at_the_depth_limit_saves_and_reloads() {
        use crate::tree::MAX_TREE_DEPTH;
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        store
            .graft(None, chain(MAX_TREE_DEPTH - 1))
            .expect("the deepest allowed tree");
        let again = Store::open(dir.path()).unwrap();
        assert!(again.notice.is_none(), "{:?}", again.notice);
        assert_eq!(again.store.tree(), store.tree());
        assert_eq!(again.store.tree().depth(), MAX_TREE_DEPTH);
    }

    /// The limit must stay below what the JSON loader accepts (serde_json refuses more than
    /// 128 nested levels), also inside a backup body, which nests the document one object
    /// deeper (`{ "connections": <document> }`). Find the real edges by parsing deeper and
    /// deeper documents, so a change of the limit or of the format is noticed. At the time of
    /// writing a connection that has settings loads down to level 62 both from
    /// `connections.json` and from a backup body, so the limit of 60 keeps a margin of two
    /// levels.
    #[test]
    fn the_depth_limit_leaves_a_margin_below_what_the_loader_accepts() {
        use crate::tree::MAX_TREE_DEPTH;
        let parses = |levels: usize, wrap: bool| {
            let tree = Tree::from_children(vec![chain(levels - 1)]);
            let doc = persist::document(&tree);
            let doc = if wrap {
                serde_json::json!({ "schemaVersion": 1, "connections": doc })
            } else {
                doc
            };
            serde_json::from_str::<serde_json::Value>(&serde_json::to_string(&doc).unwrap()).is_ok()
        };
        let edge = |wrap: bool| (1..120).take_while(|n| parses(*n, wrap)).last().unwrap();
        assert!(
            edge(true) > MAX_TREE_DEPTH,
            "a backup body fails at level {} but the limit is {MAX_TREE_DEPTH}",
            edge(true) + 1
        );
        assert!(edge(false) >= edge(true));

        // and the real loader agrees at the limit
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        persist::save(&path, &Tree::from_children(vec![chain(MAX_TREE_DEPTH - 1)])).unwrap();
        assert!(persist::load(&path).unwrap().notice.is_none());
    }

    #[test]
    fn one_level_over_the_limit_is_refused_and_nothing_is_written() {
        use crate::tree::MAX_TREE_DEPTH;
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        let err = store.graft(None, chain(MAX_TREE_DEPTH)).unwrap_err();
        assert_eq!(err.code(), "too_deep");
        assert!(!dir.path().join(FILE_NAME).exists());
        assert!(store.tree().children().is_empty());

        let err = store
            .replace_tree(Tree::from_children(vec![chain(MAX_TREE_DEPTH)]))
            .unwrap_err();
        assert_eq!(err.code(), "too_deep");
        assert!(!dir.path().join(FILE_NAME).exists());
        assert!(store.tree().children().is_empty());
    }
}
