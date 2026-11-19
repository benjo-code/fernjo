//! A `Tree` bound to its file: every successful mutation is saved before it is visible.

use std::path::{Path, PathBuf};

use crate::error::{LoadError, SaveError, StoreError, TreeError};
use crate::model::{CredentialId, Node, NodeId, Protocol};
use crate::persist::{self, FILE_NAME, Fingerprint, Notice};
use crate::tree::{MAX_TREE_DEPTH, NodePatch, Tree};

/// A tree plus the file it is saved to.
#[derive(Debug)]
pub struct Store {
    path: PathBuf,
    tree: Tree,
    /// SHA-256 of the file bytes this store last read or wrote (`None`: there was no file).
    fingerprint: Option<Fingerprint>,
}

/// A freshly opened store and anything the user should be told about.
#[derive(Debug)]
pub struct Opened {
    pub store: Store,
    pub notice: Option<Notice>,
}

/// The fingerprint of the file a load just adopted. After a restore from `.bak` the main file
/// is written again (best effort) and the fingerprint is that of the new file.
fn adopted_fingerprint(path: &Path, loaded: &persist::Loaded) -> Option<Fingerprint> {
    if loaded.notice.is_some()
        && let Ok(fingerprint) = persist::save(path, &loaded.tree)
    {
        return Some(fingerprint);
    }
    persist::fingerprint(path).ok().flatten()
}

impl Store {
    /// Opens (or starts) the store in `dir`. A missing file means an empty tree and is
    /// only created by the first mutation. After a restore from `.bak` the main file is
    /// written again (best effort: a read-only folder still lets the tree be viewed).
    pub fn open(dir: &Path) -> Result<Opened, LoadError> {
        let path = dir.join(FILE_NAME);
        let loaded = persist::load(&path)?;
        let fingerprint = adopted_fingerprint(&path, &loaded);
        Ok(Opened {
            store: Store {
                path,
                tree: loaded.tree,
                fingerprint,
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

    /// Re-reads the file and makes it current, adopting what another program or device wrote.
    /// On an error nothing changes (a later save is still refused while the file differs).
    pub fn reload(&mut self) -> Result<Option<Notice>, LoadError> {
        let loaded = persist::load(&self.path)?;
        self.fingerprint = adopted_fingerprint(&self.path, &loaded);
        self.tree = loaded.tree;
        Ok(loaded.notice)
    }

    /// Refuses with `ChangedOnDisk` when the file no longer holds the bytes this store last
    /// read or wrote. A missing file is recreated. A check, not a lock: a sync tool can still
    /// win the race between this check and the rename.
    fn check_unchanged(&self) -> Result<(), StoreError> {
        let current = persist::fingerprint(&self.path).map_err(|source| SaveError {
            path: self.path.clone(),
            source,
        })?;
        match current {
            None => Ok(()),
            Some(_) if current == self.fingerprint => Ok(()),
            Some(_) => Err(StoreError::ChangedOnDisk),
        }
    }

    /// Checks the file, saves `tree` and remembers what was written. Writes nothing when the
    /// check refuses.
    fn save_checked(&mut self, tree: &Tree) -> Result<(), StoreError> {
        self.check_unchanged()?;
        self.fingerprint = Some(persist::save(&self.path, tree)?);
        Ok(())
    }

    /// Applies `f` to a copy, saves it and only then makes it current, so memory never
    /// diverges from disk.
    fn mutate<T>(
        &mut self,
        f: impl FnOnce(&mut Tree) -> Result<T, TreeError>,
    ) -> Result<T, StoreError> {
        let mut next = self.tree.clone();
        let out = f(&mut next)?;
        // Defence in depth: `Tree` refuses what is too deep, but a file that cannot be read back
        // would lock the owner out of the whole tree and its `.bak`, so a mutation that made the
        // tree deeper than the limit is never saved (a deeper tree an older version wrote can
        // still be edited, just not deepened).
        let depth = next.depth();
        if depth > MAX_TREE_DEPTH && depth > self.tree.depth() {
            return Err(TreeError::TooDeep.into());
        }
        self.save_checked(&next)?;
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
        self.save_checked(&tree)?;
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
            self.save_checked(&next)?;
            self.tree = next;
        }
        Ok(cleared)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CredentialId, NodeKind, Protocol};
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
            fingerprint: None,
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
            fingerprint: None,
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

    /// The review's sequence (M1): the deepest tree an import accepts, then folders by hand,
    /// saving after each. Past level 62 the file no longer loaded and neither did its `.bak`.
    #[test]
    fn hand_made_nesting_cannot_make_the_file_unloadable() {
        use crate::tree::MAX_TREE_DEPTH;
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        store.graft(None, chain(MAX_TREE_DEPTH - 2)).unwrap(); // folders 1..=58, leaf at 59
        let mut parent = store.tree().children()[0].id;
        loop {
            let next = match &store.tree().get(parent).unwrap().kind {
                NodeKind::Folder { children } => children
                    .iter()
                    .find(|c| matches!(c.kind, NodeKind::Folder { .. }))
                    .map(|c| c.id),
                NodeKind::Connection { .. } => None,
            };
            match next {
                Some(id) => parent = id,
                None => break,
            }
        }
        let mut refused = 0;
        for i in 0..8 {
            match store.create_folder(Some(parent), None, &format!("hand-{i}")) {
                Ok(id) => parent = id,
                Err(e) => {
                    assert_eq!(e.code(), "too_deep");
                    refused += 1;
                }
            }
            // every state on disk loads, the main file and the backup
            let again = Store::open(dir.path()).unwrap();
            assert!(again.notice.is_none(), "{i}: {:?}", again.notice);
            assert_eq!(again.store.tree(), store.tree());
            assert!(persist::load(&dir.path().join(format!("{FILE_NAME}.bak"))).is_ok());
        }
        assert!(refused > 0);
        assert_eq!(store.tree().depth(), MAX_TREE_DEPTH);
    }

    #[test]
    fn a_refused_move_or_duplicate_writes_nothing() {
        use crate::tree::MAX_TREE_DEPTH;
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        store.graft(None, chain(MAX_TREE_DEPTH - 1)).unwrap();
        let sibling = store.create_folder(None, None, "sibling").unwrap();
        store.create_folder(Some(sibling), None, "inner").unwrap();
        let deep_folder = {
            let mut id = store.tree().children()[0].id;
            while let NodeKind::Folder { children } = &store.tree().get(id).unwrap().kind {
                match children
                    .iter()
                    .find(|c| matches!(c.kind, NodeKind::Folder { .. }))
                {
                    Some(c) => id = c.id,
                    None => break,
                }
            }
            id
        };
        let on_disk = fs::read(dir.path().join(FILE_NAME)).unwrap();
        let before = store.tree().clone();
        // `sibling` has two levels; below the folder at level 59 it would end at 61
        let err = store.move_node(sibling, Some(deep_folder), 0).unwrap_err();
        assert_eq!(err.code(), "too_deep");
        assert_eq!(store.tree(), &before);
        assert_eq!(fs::read(dir.path().join(FILE_NAME)).unwrap(), on_disk);
    }

    #[test]
    fn the_save_guard_refuses_a_mutation_that_deepens_the_tree_past_the_limit() {
        use crate::tree::MAX_TREE_DEPTH;
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        // `Tree` itself refuses this, so reach the guard with a closure that bypasses it
        let err = store
            .mutate(|t| {
                *t = Tree::from_children(vec![chain(MAX_TREE_DEPTH)]);
                Ok(())
            })
            .unwrap_err();
        assert_eq!(err.code(), "too_deep");
        assert!(!dir.path().join(FILE_NAME).exists());
        assert!(store.tree().children().is_empty());
    }

    // ---- changed on disk ------------------------------------------------

    fn external_document(name: &str) -> Vec<u8> {
        let mut t = Tree::new();
        t.create_folder(None, None, name).unwrap();
        let mut bytes = serde_json::to_vec_pretty(&persist::document(&t)).unwrap();
        bytes.push(b'\n');
        bytes
    }

    #[test]
    fn save_is_refused_when_the_file_changed_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        store.create_folder(None, None, "one").unwrap();
        store.create_folder(None, None, "two").unwrap();
        let bak = dir.path().join(format!("{FILE_NAME}.bak"));
        let bak_before = fs::read(&bak).unwrap();
        let tree_before = store.tree().clone();
        let external = external_document("external");
        fs::write(store.path(), &external).unwrap();

        let err = store.create_folder(None, None, "three").unwrap_err();
        assert!(matches!(err, StoreError::ChangedOnDisk), "{err:?}");
        assert_eq!(fs::read(store.path()).unwrap(), external);
        assert_eq!(store.tree(), &tree_before);
        assert_eq!(fs::read(&bak).unwrap(), bak_before);
    }

    #[test]
    fn a_refused_first_save_creates_no_bak() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        fs::write(store.path(), external_document("external")).unwrap();
        assert!(store.create_folder(None, None, "x").is_err());
        assert!(!dir.path().join(format!("{FILE_NAME}.bak")).exists());
    }

    #[test]
    fn an_identical_rewrite_is_not_a_change() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        store.create_folder(None, None, "one").unwrap();
        let bytes = fs::read(store.path()).unwrap();
        fs::write(store.path(), bytes).unwrap();
        store.create_folder(None, None, "two").unwrap();
    }

    #[test]
    fn a_deleted_file_is_recreated() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        store.create_folder(None, None, "one").unwrap();
        fs::remove_file(store.path()).unwrap();
        store.create_folder(None, None, "two").unwrap();
        assert!(store.path().exists());
    }

    #[test]
    fn a_file_that_appears_after_an_empty_start_counts_as_changed() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        fs::write(store.path(), external_document("external")).unwrap();
        let err = store.create_folder(None, None, "mine").unwrap_err();
        assert!(matches!(err, StoreError::ChangedOnDisk), "{err:?}");
    }

    #[test]
    fn replace_tree_is_checked_too() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        store.create_folder(None, None, "one").unwrap();
        fs::write(store.path(), external_document("external")).unwrap();
        let err = store.replace_tree(Tree::new()).unwrap_err();
        assert!(matches!(err, StoreError::ChangedOnDisk), "{err:?}");
    }

    #[test]
    fn clear_credential_is_checked_too() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        let cred = CredentialId::new_v4();
        let a = store.create_folder(None, None, "a").unwrap();
        store
            .update(
                a,
                NodePatch {
                    credential: Patch::Set(cred),
                    ..NodePatch::default()
                },
            )
            .unwrap();
        fs::write(store.path(), external_document("external")).unwrap();
        let err = store.clear_credential(cred).unwrap_err();
        assert!(matches!(err, StoreError::ChangedOnDisk), "{err:?}");
    }

    #[test]
    fn reload_adopts_the_file_and_the_next_save_works() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        store.create_folder(None, None, "one").unwrap();
        fs::write(store.path(), external_document("external")).unwrap();
        assert!(store.create_folder(None, None, "x").is_err());

        assert_eq!(store.reload().unwrap(), None);
        let names: Vec<_> = store
            .tree()
            .children()
            .iter()
            .map(|n| n.name.clone())
            .collect();
        assert_eq!(names, ["external"]);
        store.create_folder(None, None, "after").unwrap();
        assert_eq!(open(dir.path()).tree(), store.tree());
    }

    #[test]
    fn a_failed_reload_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        store.create_folder(None, None, "one").unwrap();
        let before = store.tree().clone();
        fs::write(store.path(), r#"{"schemaVersion":99,"children":[]}"#).unwrap();
        assert!(store.reload().is_err());
        assert_eq!(store.tree(), &before);
        assert!(matches!(
            store.create_folder(None, None, "x").unwrap_err(),
            StoreError::ChangedOnDisk
        ));
    }

    #[test]
    fn changed_on_disk_has_its_stable_code() {
        assert_eq!(StoreError::ChangedOnDisk.code(), "changed_on_disk");
    }
}
