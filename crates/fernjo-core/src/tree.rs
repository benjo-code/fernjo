//! The tree of folders and connections with all of its rules (no I/O).

use std::collections::HashSet;

use serde::{Deserialize, Deserializer};

use crate::error::TreeError;
use crate::model::{CredentialId, Node, NodeId, NodeKind, Protocol, ResolvedNode, Settings};
use crate::resolve::resolve;

/// A change to one inheritable setting.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Patch<T> {
    /// Leave the value as it is (field absent in JSON).
    #[default]
    Keep,
    /// Set a local value.
    Set(T),
    /// Remove the local value so the node inherits (`null` in JSON).
    Inherit,
}

fn patch_from_json<'de, D, T>(deserializer: D) -> Result<Patch<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(match Option::<T>::deserialize(deserializer)? {
        Some(value) => Patch::Set(value),
        None => Patch::Inherit,
    })
}

/// A partial update of a node. Absent fields stay as they are.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NodePatch {
    pub name: Option<String>,
    pub description: Option<String>,
    pub host: Option<String>,
    pub protocol: Option<Protocol>,
    #[serde(deserialize_with = "patch_from_json")]
    pub username: Patch<String>,
    #[serde(deserialize_with = "patch_from_json")]
    pub port: Patch<u32>,
    #[serde(deserialize_with = "patch_from_json")]
    pub keepalive_secs: Patch<u32>,
    #[serde(deserialize_with = "patch_from_json")]
    pub credential: Patch<CredentialId>,
}

/// The forest of top-level nodes. The root itself is implicit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tree {
    children: Vec<Node>,
}

fn find(nodes: &[Node], id: NodeId) -> Option<&Node> {
    for node in nodes {
        if node.id == id {
            return Some(node);
        }
        if let NodeKind::Folder { children } = &node.kind
            && let Some(found) = find(children, id)
        {
            return Some(found);
        }
    }
    None
}

fn find_mut(nodes: &mut [Node], id: NodeId) -> Option<&mut Node> {
    for node in nodes {
        if node.id == id {
            return Some(node);
        }
        if let NodeKind::Folder { children } = &mut node.kind
            && let Some(found) = find_mut(children, id)
        {
            return Some(found);
        }
    }
    None
}

/// Collects the folders above `id` (top-level first). `None` if `id` is not in `nodes`.
fn ancestors<'a>(nodes: &'a [Node], id: NodeId, trail: &mut Vec<&'a Node>) -> bool {
    for node in nodes {
        if node.id == id {
            return true;
        }
        if let NodeKind::Folder { children } = &node.kind {
            trail.push(node);
            if ancestors(children, id, trail) {
                return true;
            }
            trail.pop();
        }
    }
    false
}

/// Finds the parent and position of `id`.
fn locate(nodes: &[Node], parent: Option<NodeId>, id: NodeId) -> Option<(Option<NodeId>, usize)> {
    for (index, node) in nodes.iter().enumerate() {
        if node.id == id {
            return Some((parent, index));
        }
        if let NodeKind::Folder { children } = &node.kind
            && let Some(found) = locate(children, Some(node.id), id)
        {
            return Some(found);
        }
    }
    None
}

fn count(node: &Node) -> usize {
    match &node.kind {
        NodeKind::Folder { children } => 1 + children.iter().map(count).sum::<usize>(),
        NodeKind::Connection { .. } => 1,
    }
}

fn assign_fresh_ids(node: &mut Node) {
    node.id = NodeId::new_v4();
    if let NodeKind::Folder { children } = &mut node.kind {
        children.iter_mut().for_each(assign_fresh_ids);
    }
}

fn validated_name(name: &str) -> Result<String, TreeError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        Err(TreeError::EmptyName)
    } else {
        Ok(trimmed.to_string())
    }
}

fn validated_port(port: u32) -> Result<u16, TreeError> {
    u16::try_from(port)
        .ok()
        .filter(|p| *p != 0)
        .ok_or(TreeError::InvalidPort)
}

impl Tree {
    pub fn new() -> Tree {
        Tree::default()
    }

    pub fn from_children(children: Vec<Node>) -> Tree {
        Tree { children }
    }

    pub fn children(&self) -> &[Node] {
        &self.children
    }

    pub fn get(&self, id: NodeId) -> Option<&Node> {
        find(&self.children, id)
    }

    /// Number of nodes below `id` (the node itself is not counted).
    pub fn descendant_count(&self, id: NodeId) -> Result<usize, TreeError> {
        let node = self.get(id).ok_or(TreeError::NotFound(id))?;
        Ok(count(node) - 1)
    }

    /// The child list of `parent` (`None` = top level).
    fn siblings_mut(&mut self, parent: Option<NodeId>) -> Result<&mut Vec<Node>, TreeError> {
        match parent {
            None => Ok(&mut self.children),
            Some(pid) => match find_mut(&mut self.children, pid) {
                None => Err(TreeError::NotFound(pid)),
                Some(Node {
                    kind: NodeKind::Folder { children },
                    ..
                }) => Ok(children),
                Some(_) => Err(TreeError::NotAFolder),
            },
        }
    }

    fn insert(
        &mut self,
        parent: Option<NodeId>,
        index: Option<usize>,
        node: Node,
    ) -> Result<NodeId, TreeError> {
        let id = node.id;
        let siblings = self.siblings_mut(parent)?;
        let at = index.unwrap_or(siblings.len());
        if at > siblings.len() {
            return Err(TreeError::IndexOutOfRange);
        }
        siblings.insert(at, node);
        Ok(id)
    }

    /// Appends a whole subtree (with its ids) at the end of `parent` (`None` = top level).
    /// Refuses with `DuplicateId` when any id already exists in the tree or repeats inside
    /// the subtree; nothing changes on error.
    pub fn graft(&mut self, parent: Option<NodeId>, node: Node) -> Result<NodeId, TreeError> {
        fn check(node: &Node, tree: &Tree, seen: &mut HashSet<NodeId>) -> Result<(), TreeError> {
            if tree.get(node.id).is_some() || !seen.insert(node.id) {
                return Err(TreeError::DuplicateId(node.id));
            }
            if let NodeKind::Folder { children } = &node.kind {
                for child in children {
                    check(child, tree, seen)?;
                }
            }
            Ok(())
        }
        // validate the parent first so a bad parent reports as such
        self.siblings_mut(parent)?;
        check(&node, self, &mut HashSet::new())?;
        self.insert(parent, None, node)
    }

    pub fn create_folder(
        &mut self,
        parent: Option<NodeId>,
        index: Option<usize>,
        name: &str,
    ) -> Result<NodeId, TreeError> {
        let node = Node {
            id: NodeId::new_v4(),
            name: validated_name(name)?,
            description: String::new(),
            settings: Settings::default(),
            kind: NodeKind::Folder {
                children: Vec::new(),
            },
        };
        self.insert(parent, index, node)
    }

    pub fn create_connection(
        &mut self,
        parent: Option<NodeId>,
        index: Option<usize>,
        name: &str,
        protocol: Protocol,
        host: &str,
    ) -> Result<NodeId, TreeError> {
        let node = Node {
            id: NodeId::new_v4(),
            name: validated_name(name)?,
            description: String::new(),
            settings: Settings::default(),
            kind: NodeKind::Connection {
                protocol,
                host: host.trim().to_string(),
            },
        };
        self.insert(parent, index, node)
    }

    /// Applies `patch`. Everything is validated first, so a failure changes nothing.
    pub fn update(&mut self, id: NodeId, patch: NodePatch) -> Result<(), TreeError> {
        let name = patch.name.as_deref().map(validated_name).transpose()?;
        let port = match patch.port {
            Patch::Set(p) => Patch::Set(validated_port(p)?),
            Patch::Keep => Patch::Keep,
            Patch::Inherit => Patch::Inherit,
        };
        let node = find_mut(&mut self.children, id).ok_or(TreeError::NotFound(id))?;
        if (patch.host.is_some() || patch.protocol.is_some())
            && !matches!(node.kind, NodeKind::Connection { .. })
        {
            return Err(TreeError::NotAConnection);
        }

        if let Some(name) = name {
            node.name = name;
        }
        if let Some(description) = patch.description {
            node.description = description;
        }
        if let NodeKind::Connection { protocol, host } = &mut node.kind {
            if let Some(new_host) = patch.host {
                *host = new_host.trim().to_string();
            }
            if let Some(new_protocol) = patch.protocol {
                *protocol = new_protocol;
            }
        }
        apply(&mut node.settings.username, patch.username);
        apply(&mut node.settings.port, port);
        apply(&mut node.settings.keepalive_secs, patch.keepalive_secs);
        apply(&mut node.settings.credential, patch.credential);
        Ok(())
    }

    /// Nodes (folders and connections) that set `id` locally, depth-first, with their names.
    /// Nodes that only inherit the credential are not listed.
    pub fn nodes_using_credential(&self, id: CredentialId) -> Vec<(NodeId, String)> {
        fn walk(nodes: &[Node], id: CredentialId, out: &mut Vec<(NodeId, String)>) {
            for node in nodes {
                if node.settings.credential == Some(id) {
                    out.push((node.id, node.name.clone()));
                }
                if let NodeKind::Folder { children } = &node.kind {
                    walk(children, id, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.children, id, &mut out);
        out
    }

    /// Removes every local reference to `id` (those nodes inherit again); returns how many.
    pub fn clear_credential(&mut self, id: CredentialId) -> usize {
        fn walk(nodes: &mut [Node], id: CredentialId) -> usize {
            let mut cleared = 0;
            for node in nodes {
                if node.settings.credential == Some(id) {
                    node.settings.credential = None;
                    cleared += 1;
                }
                if let NodeKind::Folder { children } = &mut node.kind {
                    cleared += walk(children, id);
                }
            }
            cleared
        }
        walk(&mut self.children, id)
    }

    /// Moves `id` below `new_parent`. `index` counts among the new parent's children
    /// after `id` was taken out; `index == len` appends. Nothing changes on error.
    pub fn move_node(
        &mut self,
        id: NodeId,
        new_parent: Option<NodeId>,
        index: usize,
    ) -> Result<(), TreeError> {
        let node = self.get(id).ok_or(TreeError::NotFound(id))?;
        let (old_parent, _) = locate(&self.children, None, id).ok_or(TreeError::NotFound(id))?;
        let target_len = match new_parent {
            None => self.children.len(),
            Some(pid) => {
                let target = self.get(pid).ok_or(TreeError::NotFound(pid))?;
                if find(std::slice::from_ref(node), pid).is_some() {
                    return Err(TreeError::InvalidMove);
                }
                match &target.kind {
                    NodeKind::Folder { children } => children.len(),
                    NodeKind::Connection { .. } => return Err(TreeError::NotAFolder),
                }
            }
        };
        let target_len = if old_parent == new_parent {
            target_len - 1
        } else {
            target_len
        };
        if index > target_len {
            return Err(TreeError::IndexOutOfRange);
        }

        let moved = self.take(id).ok_or(TreeError::NotFound(id))?;
        self.insert(new_parent, Some(index), moved)?;
        Ok(())
    }

    /// Removes `id` (with its subtree) from wherever it is and returns it.
    fn take(&mut self, id: NodeId) -> Option<Node> {
        let (parent, index) = locate(&self.children, None, id)?;
        let siblings = self.siblings_mut(parent).ok()?;
        Some(siblings.remove(index))
    }

    /// Deep-copies `id` with fresh ids and puts the copy right after the original.
    pub fn duplicate(&mut self, id: NodeId) -> Result<NodeId, TreeError> {
        let original = self.get(id).ok_or(TreeError::NotFound(id))?;
        let (parent, index) = locate(&self.children, None, id).ok_or(TreeError::NotFound(id))?;
        let mut copy = original.clone();
        assign_fresh_ids(&mut copy);
        copy.name = format!("{} copy", copy.name);
        self.insert(parent, Some(index + 1), copy)
    }

    /// Deletes `id` and everything below it; returns how many nodes were removed.
    pub fn delete(&mut self, id: NodeId) -> Result<usize, TreeError> {
        let removed = self.take(id).ok_or(TreeError::NotFound(id))?;
        Ok(count(&removed))
    }

    pub fn resolve_node(&self, id: NodeId) -> Result<ResolvedNode, TreeError> {
        let mut trail = Vec::new();
        if !ancestors(&self.children, id, &mut trail) {
            return Err(TreeError::NotFound(id));
        }
        let node = self.get(id).ok_or(TreeError::NotFound(id))?;
        Ok(resolve(&trail, node))
    }
}

fn apply<T>(slot: &mut Option<T>, patch: Patch<T>) {
    match patch {
        Patch::Keep => {}
        Patch::Set(value) => *slot = Some(value),
        Patch::Inherit => *slot = None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{NodeKind, Protocol, Source};

    fn names(nodes: &[Node]) -> Vec<&str> {
        nodes.iter().map(|n| n.name.as_str()).collect()
    }

    fn children_of(tree: &Tree, id: NodeId) -> &[Node] {
        match &tree.get(id).unwrap().kind {
            NodeKind::Folder { children } => children,
            NodeKind::Connection { .. } => panic!("not a folder"),
        }
    }

    fn folder(tree: &mut Tree, parent: Option<NodeId>, name: &str) -> NodeId {
        tree.create_folder(parent, None, name).unwrap()
    }

    fn ssh(tree: &mut Tree, parent: Option<NodeId>, name: &str) -> NodeId {
        tree.create_connection(parent, None, name, Protocol::Ssh, "h.example")
            .unwrap()
    }

    fn all_ids(nodes: &[Node], out: &mut Vec<NodeId>) {
        for n in nodes {
            out.push(n.id);
            if let NodeKind::Folder { children } = &n.kind {
                all_ids(children, out);
            }
        }
    }

    // ---- graft --------------------------------------------------------

    fn leaf(name: &str) -> Node {
        Node {
            id: NodeId::new_v4(),
            name: name.into(),
            description: String::new(),
            settings: Settings::default(),
            kind: NodeKind::Connection {
                protocol: Protocol::Ssh,
                host: "h".into(),
            },
        }
    }

    fn branch(name: &str, children: Vec<Node>) -> Node {
        Node {
            kind: NodeKind::Folder { children },
            ..leaf(name)
        }
    }

    #[test]
    fn graft_adds_subtree_at_end_of_parent() {
        let mut t = Tree::new();
        let f = folder(&mut t, None, "f");
        ssh(&mut t, Some(f), "first");
        let sub = branch("sub", vec![leaf("x"), leaf("y")]);
        let sub_id = sub.id;
        assert_eq!(t.graft(Some(f), sub).unwrap(), sub_id);
        assert_eq!(names(children_of(&t, f)), ["first", "sub"]);
        assert_eq!(names(children_of(&t, sub_id)), ["x", "y"]);
        t.graft(None, leaf("top")).unwrap();
        assert_eq!(names(t.children()), ["f", "top"]);
    }

    #[test]
    fn graft_into_a_connection_is_not_a_folder() {
        let mut t = Tree::new();
        let c = ssh(&mut t, None, "c");
        let before = t.clone();
        let err = t.graft(Some(c), leaf("x")).unwrap_err();
        assert_eq!(err, TreeError::NotAFolder);
        assert_eq!(t, before);
    }

    #[test]
    fn graft_with_an_existing_id_is_duplicate_id() {
        let mut t = Tree::new();
        let c = ssh(&mut t, None, "c");
        let before = t.clone();
        let mut inner = leaf("x");
        inner.id = c;
        let err = t.graft(None, branch("sub", vec![inner])).unwrap_err();
        assert_eq!(err, TreeError::DuplicateId(c));
        assert_eq!(err.code(), "duplicate_id");
        assert_eq!(t, before);
    }

    #[test]
    fn graft_with_a_repeated_id_inside_is_duplicate_id() {
        let mut t = Tree::new();
        let a = leaf("a");
        let mut b = leaf("b");
        b.id = a.id;
        let id = a.id;
        let err = t.graft(None, branch("sub", vec![a, b])).unwrap_err();
        assert_eq!(err, TreeError::DuplicateId(id));
        assert!(t.children().is_empty());
    }

    // ---- credential ---------------------------------------------------

    fn set_credential(t: &mut Tree, id: NodeId, cred: CredentialId) {
        t.update(
            id,
            NodePatch {
                credential: Patch::Set(cred),
                ..NodePatch::default()
            },
        )
        .unwrap();
    }

    #[test]
    fn patch_credential_set_then_inherit_clears_local_value() {
        let mut t = Tree::new();
        let c = ssh(&mut t, None, "c");
        let cred = CredentialId::new_v4();
        set_credential(&mut t, c, cred);
        assert_eq!(t.get(c).unwrap().settings.credential, Some(cred));
        t.update(
            c,
            NodePatch {
                credential: Patch::Inherit,
                ..NodePatch::default()
            },
        )
        .unwrap();
        assert_eq!(t.get(c).unwrap().settings.credential, None);
        let p: NodePatch = serde_json::from_str(&format!(r#"{{"credential":"{cred}"}}"#)).unwrap();
        assert_eq!(p.credential, Patch::Set(cred));
        let p: NodePatch = serde_json::from_str(r#"{"credential":null}"#).unwrap();
        assert_eq!(p.credential, Patch::Inherit);
        let p: NodePatch = serde_json::from_str("{}").unwrap();
        assert_eq!(p.credential, Patch::Keep);
    }

    #[test]
    fn duplicate_keeps_the_credential() {
        let mut t = Tree::new();
        let f = folder(&mut t, None, "f");
        let c = ssh(&mut t, Some(f), "c");
        let cred = CredentialId::new_v4();
        set_credential(&mut t, f, cred);
        set_credential(&mut t, c, cred);
        let copy = t.duplicate(f).unwrap();
        assert_eq!(t.get(copy).unwrap().settings.credential, Some(cred));
        let child = &children_of(&t, copy)[0];
        assert_eq!(child.settings.credential, Some(cred));
    }

    #[test]
    fn nodes_using_credential_lists_local_users_only() {
        let mut t = Tree::new();
        let cred = CredentialId::new_v4();
        let other = CredentialId::new_v4();
        let f = folder(&mut t, None, "f");
        let a = ssh(&mut t, Some(f), "a"); // inherits only
        let b = ssh(&mut t, Some(f), "b");
        let c = ssh(&mut t, None, "c");
        let d = ssh(&mut t, None, "d");
        set_credential(&mut t, f, cred);
        set_credential(&mut t, b, cred);
        set_credential(&mut t, c, cred);
        set_credential(&mut t, d, other);
        let users = t.nodes_using_credential(cred);
        assert_eq!(
            users,
            vec![
                (f, "f".to_string()),
                (b, "b".to_string()),
                (c, "c".to_string())
            ]
        );
        assert!(!users.iter().any(|(id, _)| *id == a));
        assert!(t.nodes_using_credential(CredentialId::new_v4()).is_empty());
    }

    #[test]
    fn clear_credential_removes_every_local_reference_and_counts_them() {
        let mut t = Tree::new();
        let cred = CredentialId::new_v4();
        let other = CredentialId::new_v4();
        let f = folder(&mut t, None, "f");
        let a = ssh(&mut t, Some(f), "a");
        let b = ssh(&mut t, None, "b");
        set_credential(&mut t, f, cred);
        set_credential(&mut t, a, cred);
        set_credential(&mut t, b, other);
        assert_eq!(t.clear_credential(cred), 2);
        assert_eq!(t.get(f).unwrap().settings.credential, None);
        assert_eq!(t.get(a).unwrap().settings.credential, None);
        assert_eq!(t.get(b).unwrap().settings.credential, Some(other));
        assert_eq!(t.clear_credential(cred), 0);
    }

    #[test]
    fn dangling_credential_id_resolves_without_error() {
        let mut t = Tree::new();
        let c = ssh(&mut t, None, "c");
        let cred = CredentialId::new_v4();
        set_credential(&mut t, c, cred);
        let r = t.resolve_node(c).unwrap();
        assert_eq!(r.credential.effective, Some(cred));
    }

    // ---- create -------------------------------------------------------

    #[test]
    fn create_folder_appends_by_default() {
        let mut t = Tree::new();
        folder(&mut t, None, "a");
        folder(&mut t, None, "b");
        assert_eq!(names(t.children()), ["a", "b"]);
    }

    #[test]
    fn create_folder_at_index_0_inserts_first() {
        let mut t = Tree::new();
        folder(&mut t, None, "a");
        t.create_folder(None, Some(0), "first").unwrap();
        assert_eq!(names(t.children()), ["first", "a"]);
    }

    #[test]
    fn create_index_beyond_len_is_index_out_of_range() {
        let mut t = Tree::new();
        folder(&mut t, None, "a");
        let err = t.create_folder(None, Some(2), "x").unwrap_err();
        assert_eq!(err, TreeError::IndexOutOfRange);
        assert_eq!(names(t.children()), ["a"]);
    }

    #[test]
    fn create_in_unknown_parent_is_not_found() {
        let mut t = Tree::new();
        let err = t
            .create_folder(Some(NodeId::new_v4()), None, "x")
            .unwrap_err();
        assert_eq!(err.code(), "not_found");
    }

    #[test]
    fn create_inside_connection_is_not_a_folder() {
        let mut t = Tree::new();
        let c = ssh(&mut t, None, "c");
        let err = t.create_folder(Some(c), None, "x").unwrap_err();
        assert_eq!(err, TreeError::NotAFolder);
    }

    #[test]
    fn whitespace_only_name_is_empty_name() {
        let mut t = Tree::new();
        let err = t.create_folder(None, None, "   ").unwrap_err();
        assert_eq!(err, TreeError::EmptyName);
        let err = t
            .create_connection(None, None, "\t", Protocol::Ssh, "")
            .unwrap_err();
        assert_eq!(err, TreeError::EmptyName);
        assert!(t.children().is_empty());
    }

    #[test]
    fn name_is_stored_trimmed() {
        let mut t = Tree::new();
        let id = folder(&mut t, None, "  Homelab \n");
        assert_eq!(t.get(id).unwrap().name, "Homelab");
    }

    // ---- update -------------------------------------------------------

    #[test]
    fn rename_updates_name() {
        let mut t = Tree::new();
        let id = folder(&mut t, None, "a");
        t.update(
            id,
            NodePatch {
                name: Some("b".into()),
                ..NodePatch::default()
            },
        )
        .unwrap();
        assert_eq!(t.get(id).unwrap().name, "b");
    }

    #[test]
    fn rename_to_whitespace_is_empty_name_and_changes_nothing() {
        let mut t = Tree::new();
        let id = folder(&mut t, None, "a");
        let err = t
            .update(
                id,
                NodePatch {
                    name: Some("  ".into()),
                    description: Some("changed".into()),
                    ..NodePatch::default()
                },
            )
            .unwrap_err();
        assert_eq!(err, TreeError::EmptyName);
        assert_eq!(t.get(id).unwrap().name, "a");
        assert_eq!(t.get(id).unwrap().description, "");
    }

    #[test]
    fn port_0_and_65536_are_invalid_port() {
        let mut t = Tree::new();
        let id = ssh(&mut t, None, "c");
        for bad in [0u32, 65536] {
            let err = t
                .update(
                    id,
                    NodePatch {
                        port: Patch::Set(bad),
                        ..NodePatch::default()
                    },
                )
                .unwrap_err();
            assert_eq!(err, TreeError::InvalidPort);
        }
        assert_eq!(t.get(id).unwrap().settings.port, None);
    }

    #[test]
    fn port_65535_is_accepted() {
        let mut t = Tree::new();
        let id = ssh(&mut t, None, "c");
        t.update(
            id,
            NodePatch {
                port: Patch::Set(65535),
                ..NodePatch::default()
            },
        )
        .unwrap();
        assert_eq!(t.get(id).unwrap().settings.port, Some(65535));
    }

    #[test]
    fn host_patch_on_folder_is_not_a_connection() {
        let mut t = Tree::new();
        let id = folder(&mut t, None, "f");
        let err = t
            .update(
                id,
                NodePatch {
                    host: Some("x".into()),
                    ..NodePatch::default()
                },
            )
            .unwrap_err();
        assert_eq!(err, TreeError::NotAConnection);
        let err = t
            .update(
                id,
                NodePatch {
                    protocol: Some(Protocol::Rdp),
                    ..NodePatch::default()
                },
            )
            .unwrap_err();
        assert_eq!(err, TreeError::NotAConnection);
    }

    #[test]
    fn patch_inherit_clears_local_value() {
        let mut t = Tree::new();
        let id = ssh(&mut t, None, "c");
        t.update(
            id,
            NodePatch {
                username: Patch::Set("root".into()),
                ..NodePatch::default()
            },
        )
        .unwrap();
        assert_eq!(
            t.resolve_node(id).unwrap().username.local.as_deref(),
            Some("root")
        );
        t.update(
            id,
            NodePatch {
                username: Patch::Inherit,
                ..NodePatch::default()
            },
        )
        .unwrap();
        let r = t.resolve_node(id).unwrap();
        assert_eq!(r.username.local, None);
        assert_eq!(r.username.source, Source::Default);
    }

    #[test]
    fn patch_keep_leaves_value() {
        let mut t = Tree::new();
        let id = ssh(&mut t, None, "c");
        t.update(
            id,
            NodePatch {
                keepalive_secs: Patch::Set(0),
                ..NodePatch::default()
            },
        )
        .unwrap();
        t.update(id, NodePatch::default()).unwrap();
        assert_eq!(t.get(id).unwrap().settings.keepalive_secs, Some(0));
    }

    #[test]
    fn patch_deserializes_absent_null_and_value() {
        let p: NodePatch = serde_json::from_str("{}").unwrap();
        assert_eq!(p.username, Patch::Keep);
        let p: NodePatch = serde_json::from_str(r#"{"username":null,"port":2200}"#).unwrap();
        assert_eq!(p.username, Patch::Inherit);
        assert_eq!(p.port, Patch::Set(2200));
        assert_eq!(p.keepalive_secs, Patch::Keep);
        let p: NodePatch = serde_json::from_str(r#"{"keepaliveSecs":5,"name":"x"}"#).unwrap();
        assert_eq!(p.keepalive_secs, Patch::Set(5));
        assert_eq!(p.name.as_deref(), Some("x"));
    }

    // ---- move ---------------------------------------------------------

    #[test]
    fn reorder_within_parent() {
        let mut t = Tree::new();
        let a = folder(&mut t, None, "a");
        folder(&mut t, None, "b");
        folder(&mut t, None, "c");
        t.move_node(a, None, 2).unwrap();
        assert_eq!(names(t.children()), ["b", "c", "a"]);
    }

    #[test]
    fn move_into_other_folder_keeps_subtree() {
        let mut t = Tree::new();
        let src = folder(&mut t, None, "src");
        ssh(&mut t, Some(src), "inner");
        let dst = folder(&mut t, None, "dst");
        t.move_node(src, Some(dst), 0).unwrap();
        assert_eq!(names(t.children()), ["dst"]);
        assert_eq!(names(children_of(&t, dst)), ["src"]);
        assert_eq!(names(children_of(&t, src)), ["inner"]);
    }

    #[test]
    fn move_folder_into_itself_is_invalid_move() {
        let mut t = Tree::new();
        let a = folder(&mut t, None, "a");
        assert_eq!(
            t.move_node(a, Some(a), 0).unwrap_err(),
            TreeError::InvalidMove
        );
    }

    #[test]
    fn move_folder_into_descendant_is_invalid_move() {
        let mut t = Tree::new();
        let a = folder(&mut t, None, "a");
        let b = folder(&mut t, Some(a), "b");
        let c = folder(&mut t, Some(b), "c");
        assert_eq!(
            t.move_node(a, Some(c), 0).unwrap_err(),
            TreeError::InvalidMove
        );
    }

    #[test]
    fn index_beyond_len_after_removal_is_index_out_of_range() {
        let mut t = Tree::new();
        let a = folder(&mut t, None, "a");
        folder(&mut t, None, "b");
        // after removing `a` the root has one child, so 2 is out of range
        assert_eq!(
            t.move_node(a, None, 2).unwrap_err(),
            TreeError::IndexOutOfRange
        );
        // index == len appends
        t.move_node(a, None, 1).unwrap();
        assert_eq!(names(t.children()), ["b", "a"]);
    }

    #[test]
    fn move_unknown_node_or_parent_is_not_found_and_into_connection_is_not_a_folder() {
        let mut t = Tree::new();
        let a = folder(&mut t, None, "a");
        let c = ssh(&mut t, None, "c");
        assert_eq!(
            t.move_node(NodeId::new_v4(), None, 0).unwrap_err().code(),
            "not_found"
        );
        assert_eq!(
            t.move_node(a, Some(NodeId::new_v4()), 0)
                .unwrap_err()
                .code(),
            "not_found"
        );
        assert_eq!(
            t.move_node(a, Some(c), 0).unwrap_err(),
            TreeError::NotAFolder
        );
    }

    #[test]
    fn failed_move_leaves_tree_unchanged() {
        let mut t = Tree::new();
        let a = folder(&mut t, None, "a");
        let b = folder(&mut t, Some(a), "b");
        ssh(&mut t, Some(b), "c");
        let before = t.clone();
        assert!(t.move_node(a, Some(b), 0).is_err());
        assert!(t.move_node(b, None, 9).is_err());
        assert_eq!(t, before);
    }

    // ---- duplicate ----------------------------------------------------

    #[test]
    fn duplicate_connection_lands_right_after_original_named_copy() {
        let mut t = Tree::new();
        let f = folder(&mut t, None, "f");
        let web = ssh(&mut t, Some(f), "web-01");
        ssh(&mut t, Some(f), "web-02");
        let copy = t.duplicate(web).unwrap();
        assert_ne!(copy, web);
        assert_eq!(
            names(children_of(&t, f)),
            ["web-01", "web-01 copy", "web-02"]
        );
        assert_eq!(t.get(copy).unwrap().name, "web-01 copy");
    }

    #[test]
    fn duplicate_folder_deep_copies_with_fresh_ids() {
        let mut t = Tree::new();
        let f = folder(&mut t, None, "f");
        t.update(
            f,
            NodePatch {
                port: Patch::Set(2200),
                ..NodePatch::default()
            },
        )
        .unwrap();
        let sub = folder(&mut t, Some(f), "sub");
        ssh(&mut t, Some(sub), "c1");
        ssh(&mut t, Some(f), "c2");
        let copy = t.duplicate(f).unwrap();
        assert_eq!(names(t.children()), ["f", "f copy"]);

        let mut ids = Vec::new();
        all_ids(t.children(), &mut ids);
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(ids.len(), 8);
        assert_eq!(unique.len(), ids.len());

        let orig = t.get(f).unwrap();
        let dup = t.get(copy).unwrap();
        assert_eq!(dup.settings, orig.settings);
        let (NodeKind::Folder { children: oc }, NodeKind::Folder { children: dc }) =
            (&orig.kind, &dup.kind)
        else {
            panic!("folders expected");
        };
        assert_eq!(names(oc), names(dc));
    }

    // ---- delete / count -----------------------------------------------

    #[test]
    fn delete_folder_returns_count_including_descendants() {
        let mut t = Tree::new();
        let f = folder(&mut t, None, "f");
        let sub = folder(&mut t, Some(f), "sub");
        ssh(&mut t, Some(sub), "c1");
        ssh(&mut t, Some(f), "c2");
        assert_eq!(t.delete(f).unwrap(), 4);
        assert!(t.children().is_empty());
    }

    #[test]
    fn delete_unknown_is_not_found() {
        let mut t = Tree::new();
        assert_eq!(t.delete(NodeId::new_v4()).unwrap_err().code(), "not_found");
    }

    #[test]
    fn descendant_count_excludes_self() {
        let mut t = Tree::new();
        let f = folder(&mut t, None, "f");
        let sub = folder(&mut t, Some(f), "sub");
        ssh(&mut t, Some(sub), "c1");
        ssh(&mut t, Some(f), "c2");
        assert_eq!(t.descendant_count(f).unwrap(), 3);
        assert_eq!(t.descendant_count(sub).unwrap(), 1);
        let leaf = ssh(&mut t, None, "leaf");
        assert_eq!(t.descendant_count(leaf).unwrap(), 0);
    }

    #[test]
    fn resolve_node_uses_folders_above() {
        let mut t = Tree::new();
        let f = folder(&mut t, None, "F");
        let c = ssh(&mut t, Some(f), "c");
        t.update(
            f,
            NodePatch {
                username: Patch::Set("ops".into()),
                ..NodePatch::default()
            },
        )
        .unwrap();
        let r = t.resolve_node(c).unwrap();
        assert_eq!(r.username.effective.as_deref(), Some("ops"));
        assert_eq!(
            r.username.source,
            Source::Folder {
                id: f,
                name: "F".into()
            }
        );
    }

    #[test]
    fn error_codes_are_stable() {
        let id = NodeId::new_v4();
        let all = [
            (TreeError::NotFound(id), "not_found"),
            (TreeError::NotAFolder, "not_a_folder"),
            (TreeError::NotAConnection, "not_a_connection"),
            (TreeError::EmptyName, "empty_name"),
            (TreeError::InvalidPort, "invalid_port"),
            (TreeError::InvalidMove, "invalid_move"),
            (TreeError::IndexOutOfRange, "index_out_of_range"),
        ];
        for (e, code) in all {
            assert_eq!(e.code(), code);
        }
    }
}
