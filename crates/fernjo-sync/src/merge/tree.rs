//! The three-way merge of the connection tree, rules C1 to C10 of the spec.
//!
//! The trees are flattened (id to parent and own fields, plus ordered child lists), the node
//! set is decided first (C5 to C7), then every surviving node gets its fields (C1 to C4) and its
//! parent (C8), cycles and depth are repaired by undoing local moves (C9), siblings are ordered
//! (C8) and the result is built. No clock, no I/O; the only randomness is the id of a conflict
//! copy, which comes from an injectable generator.

use std::collections::{HashMap, HashSet};

use fernjo_core::{
    CredentialId, MAX_TREE_DEPTH, Node, NodeId, NodeKind, Protocol, RdpResolution, Settings, Tree,
};

use super::{FieldOutcome, Finding, MergeContext, merge_field};
use crate::SyncError;

/// The merged tree and what the user should be told about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeMerge {
    pub tree: Tree,
    pub findings: Vec<Finding>,
}

fn invalid(reason: &str) -> SyncError {
    SyncError::SnapshotInvalid(reason.to_string())
}

/// What a node is, without its children.
#[derive(Debug, Clone, PartialEq)]
enum Kind {
    Folder,
    Connection { protocol: Protocol, host: String },
}

/// Every field of a node except its id, its place and its children. `flatten` and the merge
/// macro list all of them explicitly, so a new field in the model stops compiling here instead
/// of being dropped by the merge.
#[derive(Debug, Clone, PartialEq)]
struct Own {
    name: String,
    description: String,
    username: Option<String>,
    port: Option<u16>,
    keepalive_secs: Option<u32>,
    credential: Option<CredentialId>,
    domain: Option<String>,
    rdp_resolution: Option<RdpResolution>,
    rdp_color_depth: Option<u8>,
    rdp_clipboard: Option<bool>,
    kind: Kind,
}

struct Flat {
    parent: Option<NodeId>,
    own: Own,
}

/// One tree flattened: nodes by id, child ids per parent in order, ids in pre-order.
struct View {
    nodes: HashMap<NodeId, Flat>,
    children: HashMap<Option<NodeId>, Vec<NodeId>>,
    preorder: Vec<NodeId>,
}

impl View {
    fn kids(&self, parent: Option<NodeId>) -> &[NodeId] {
        self.children.get(&parent).map_or(&[], Vec::as_slice)
    }
}

fn flatten(tree: &Tree) -> Result<View, SyncError> {
    let mut view = View {
        nodes: HashMap::new(),
        children: HashMap::new(),
        preorder: Vec::new(),
    };
    let mut stack = vec![(None, 1_usize, tree.children().iter())];
    while let Some(top) = stack.last_mut() {
        let (parent, level) = (top.0, top.1);
        let Some(node) = top.2.next() else {
            stack.pop();
            continue;
        };
        if level > MAX_TREE_DEPTH {
            return Err(invalid("the tree is nested too deeply"));
        }
        let Node {
            id,
            name,
            description,
            settings,
            kind,
        } = node;
        let Settings {
            username,
            port,
            keepalive_secs,
            credential,
            domain,
            rdp_resolution,
            rdp_color_depth,
            rdp_clipboard,
        } = settings;
        let (kind, below) = match kind {
            NodeKind::Folder { children } => (Kind::Folder, Some(children)),
            NodeKind::Connection { protocol, host } => (
                Kind::Connection {
                    protocol: *protocol,
                    host: host.clone(),
                },
                None,
            ),
        };
        let own = Own {
            name: name.clone(),
            description: description.clone(),
            username: username.clone(),
            port: *port,
            keepalive_secs: *keepalive_secs,
            credential: *credential,
            domain: domain.clone(),
            rdp_resolution: *rdp_resolution,
            rdp_color_depth: *rdp_color_depth,
            rdp_clipboard: *rdp_clipboard,
            kind,
        };
        if view.nodes.insert(*id, Flat { parent, own }).is_some() {
            return Err(invalid("a node id appears twice"));
        }
        view.children.entry(parent).or_default().push(*id);
        view.preorder.push(*id);
        if let Some(children) = below {
            stack.push((Some(*id), level + 1, children.iter()));
        }
    }
    Ok(view)
}

fn to_node(id: NodeId, own: &Own, children: Vec<Node>) -> Node {
    let Own {
        name,
        description,
        username,
        port,
        keepalive_secs,
        credential,
        domain,
        rdp_resolution,
        rdp_color_depth,
        rdp_clipboard,
        kind,
    } = own;
    Node {
        id,
        name: name.clone(),
        description: description.clone(),
        settings: Settings {
            username: username.clone(),
            port: *port,
            keepalive_secs: *keepalive_secs,
            credential: *credential,
            domain: domain.clone(),
            rdp_resolution: *rdp_resolution,
            rdp_color_depth: *rdp_color_depth,
            rdp_clipboard: *rdp_clipboard,
        },
        kind: match kind {
            Kind::Folder => NodeKind::Folder { children },
            Kind::Connection { protocol, host } => NodeKind::Connection {
                protocol: *protocol,
                host: host.clone(),
            },
        },
    }
}

/// The three-way rule for one field. Without a base (the id is new on both sides) equal values
/// agree and different ones conflict.
fn pick<T: PartialEq + Clone>(base: Option<&T>, local: &T, head: &T, conflict: &mut bool) -> T {
    let outcome = match base {
        Some(base) => merge_field(base, local, head),
        None if local == head => FieldOutcome::Same,
        None => FieldOutcome::Conflict {
            local: local.clone(),
            head: head.clone(),
        },
    };
    match outcome {
        FieldOutcome::Same => head.clone(),
        FieldOutcome::Take(value) => value,
        FieldOutcome::Conflict { head, .. } => {
            *conflict = true;
            head
        }
    }
}

fn pick_kind(base: Option<&Kind>, local: &Kind, head: &Kind, conflict: &mut bool) -> Kind {
    if let (
        Kind::Connection {
            protocol: local_protocol,
            host: local_host,
        },
        Kind::Connection {
            protocol: head_protocol,
            host: head_host,
        },
    ) = (local, head)
    {
        let base = match base {
            Some(Kind::Connection { protocol, host }) => Some((protocol, host)),
            _ => None,
        };
        Kind::Connection {
            protocol: pick(base.map(|b| b.0), local_protocol, head_protocol, conflict),
            host: pick(base.map(|b| b.1), local_host, head_host, conflict),
        }
    } else {
        pick(base, local, head, conflict)
    }
}

/// Merges every field of a node present on both sides; the flag says that some field conflicted.
fn merge_own(base: Option<&Own>, local: &Own, head: &Own) -> (Own, bool) {
    let mut conflict = false;
    macro_rules! fields {
        ($($field:ident),*) => {
            Own {
                $($field: pick(base.map(|b| &b.$field), &local.$field, &head.$field, &mut conflict),)*
                kind: pick_kind(base.map(|b| &b.kind), &local.kind, &head.kind, &mut conflict),
            }
        };
    }
    let merged = fields!(
        name,
        description,
        username,
        port,
        keepalive_secs,
        credential,
        domain,
        rdp_resolution,
        rdp_color_depth,
        rdp_clipboard
    );
    (merged, conflict)
}

/// Ids that differ from the base on `side`: new, other fields or another parent.
fn touched_alone(base: &View, side: &View) -> HashSet<NodeId> {
    side.preorder
        .iter()
        .copied()
        .filter(|id| match (base.nodes.get(id), side.nodes.get(id)) {
            (Some(b), Some(s)) => b.parent != s.parent || b.own != s.own,
            _ => true,
        })
        .collect()
}

/// `touched` and the ids with a touched node below them.
fn touched_subtrees(side: &View, touched: &HashSet<NodeId>) -> HashSet<NodeId> {
    let mut marked = HashSet::new();
    for id in side.preorder.iter().rev() {
        if touched.contains(id) || side.kids(Some(*id)).iter().any(|c| marked.contains(c)) {
            marked.insert(*id);
        }
    }
    marked
}

struct Choice {
    parent: Option<NodeId>,
    /// The parent is the local side's, and the head puts the node elsewhere: an undoable move.
    local_move: bool,
}

struct Merger<'a> {
    base: &'a View,
    local: &'a View,
    head: &'a View,
    /// The node set, in a deterministic order.
    order: Vec<NodeId>,
    members: HashSet<NodeId>,
    choices: HashMap<NodeId, Choice>,
    /// Nodes both sides placed differently (C8).
    placement_conflicts: Vec<NodeId>,
    /// Nodes kept although one side deleted them (C6).
    saved: Vec<NodeId>,
    undone: Vec<NodeId>,
}

impl Merger<'_> {
    fn choose(&self, id: NodeId) -> (Choice, bool) {
        let local = self.local.nodes.get(&id);
        let head = self.head.nodes.get(&id);
        match (local, head) {
            (Some(l), Some(h)) => match self.base.nodes.get(&id) {
                Some(b) => {
                    let (local_moved, head_moved) = (l.parent != b.parent, h.parent != b.parent);
                    if !local_moved {
                        (plain(h.parent), false)
                    } else if !head_moved {
                        (
                            Choice {
                                parent: l.parent,
                                local_move: true,
                            },
                            false,
                        )
                    } else if l.parent == h.parent {
                        (plain(l.parent), false)
                    } else {
                        (plain(h.parent), true)
                    }
                }
                None if l.parent == h.parent => (plain(l.parent), false),
                None => (plain(h.parent), true),
            },
            (Some(l), None) => (plain(l.parent), false),
            (None, Some(h)) => (plain(h.parent), false),
            (None, None) => (plain(None), false),
        }
    }

    fn admit(&mut self, id: NodeId) {
        if self.members.insert(id) {
            self.order.push(id);
            let (choice, conflict) = self.choose(id);
            if conflict {
                self.placement_conflicts.push(id);
            }
            self.choices.insert(id, choice);
        }
    }

    /// Every parent must be a member too; a missing one is a folder that leads to a kept node.
    fn close_over_parents(&mut self) {
        let mut next = 0;
        while next < self.order.len() {
            let id = self.order[next];
            next += 1;
            if let Some(parent) = self.choices[&id].parent
                && !self.members.contains(&parent)
            {
                if self.base.nodes.contains_key(&parent) {
                    self.saved.push(parent);
                }
                self.admit(parent);
            }
        }
    }

    fn find_cycle(&self) -> Option<Vec<NodeId>> {
        let mut state: HashMap<NodeId, bool> = HashMap::new(); // false: on the path, true: done
        for start in &self.order {
            let mut path = Vec::new();
            let mut current = Some(*start);
            while let Some(id) = current {
                match state.get(&id) {
                    Some(true) => break,
                    Some(false) => {
                        let at = path.iter().position(|p| *p == id)?;
                        return Some(path[at..].to_vec());
                    }
                    None => {
                        state.insert(id, false);
                        path.push(id);
                        current = self.choices[&id].parent;
                    }
                }
            }
            for id in path {
                state.insert(id, true);
            }
        }
        None
    }

    /// The chain from the deepest node that is below the limit up to the top level, if any.
    fn too_deep_chain(&self) -> Option<Vec<NodeId>> {
        let mut depth: HashMap<NodeId, usize> = HashMap::new();
        let mut deepest: Option<(usize, NodeId)> = None;
        for start in &self.order {
            let mut chain = Vec::new();
            let mut current = Some(*start);
            let mut base = 0;
            while let Some(id) = current {
                if let Some(d) = depth.get(&id) {
                    base = *d;
                    break;
                }
                chain.push(id);
                current = self.choices[&id].parent;
            }
            for (i, id) in chain.iter().rev().enumerate() {
                let d = base + i + 1;
                depth.insert(*id, d);
                if deepest.is_none_or(|(best, _)| d > best) {
                    deepest = Some((d, *id));
                }
            }
        }
        let (d, id) = deepest?;
        if d <= MAX_TREE_DEPTH {
            return None;
        }
        let mut chain = vec![id];
        let mut current = self.choices[&id].parent;
        while let Some(parent) = current {
            chain.push(parent);
            current = self.choices[&parent].parent;
        }
        Some(chain)
    }

    /// Puts `id` where the head has it (the local move is undone).
    fn undo_move(&mut self, id: NodeId) {
        if let Some(h) = self.head.nodes.get(&id)
            && let Some(choice) = self.choices.get_mut(&id)
        {
            choice.parent = h.parent;
            choice.local_move = false;
            self.undone.push(id);
        }
    }

    /// C9: until there is no cycle and nothing below the depth limit, undo local moves.
    fn repair(&mut self) -> Result<(), SyncError> {
        for _ in 0..self.order.len().saturating_mul(2) + 8 {
            self.close_over_parents();
            if let Some(cycle) = self.find_cycle() {
                let moves: Vec<NodeId> = cycle
                    .into_iter()
                    .filter(|id| self.choices[id].local_move)
                    .collect();
                if moves.is_empty() {
                    return Err(invalid("the merged tree would contain a cycle"));
                }
                moves.into_iter().for_each(|id| self.undo_move(id));
            } else if let Some(chain) = self.too_deep_chain() {
                // the topmost local move on the chain is the one that pushed it down
                let Some(id) = chain
                    .into_iter()
                    .rev()
                    .find(|id| self.choices[id].local_move)
                else {
                    return Err(invalid("the merged tree would be nested too deeply"));
                };
                self.undo_move(id);
            } else {
                return Ok(());
            }
        }
        Err(invalid("the merged tree could not be repaired"))
    }

    /// The merged children of `parent` in order (C8).
    fn order_siblings(
        &self,
        parent: Option<NodeId>,
        members: &HashSet<NodeId>,
        rank: &HashMap<NodeId, usize>,
    ) -> (Vec<NodeId>, bool) {
        let (lraw, hraw, braw) = (
            self.local.kids(parent),
            self.head.kids(parent),
            self.base.kids(parent),
        );
        let set = |raw: &[NodeId]| raw.iter().copied().collect::<HashSet<_>>();
        let (lset, hset, bset) = (set(lraw), set(hraw), set(braw));
        let keep = |raw: &[NodeId], a: &HashSet<NodeId>, b: &HashSet<NodeId>| {
            raw.iter()
                .copied()
                .filter(|x| a.contains(x) && b.contains(x))
                .collect::<Vec<_>>()
        };
        let (local_primary, both_changed) = if keep(lraw, &hset, &hset) == keep(hraw, &lset, &lset)
        {
            (false, false)
        } else {
            // each side's own reorder, measured among the nodes it shares with the base
            let local_changed = keep(lraw, &bset, &bset) != keep(braw, &lset, &lset);
            let head_changed = keep(hraw, &bset, &bset) != keep(braw, &hset, &hset);
            (
                local_changed && !head_changed,
                local_changed && head_changed,
            )
        };
        let (primary, secondary) = if local_primary {
            (lraw, hraw)
        } else {
            (hraw, lraw)
        };
        let mut out: Vec<NodeId> = Vec::new();
        let mut placed = HashSet::new();
        for id in primary {
            if members.contains(id) && placed.insert(*id) {
                out.push(*id);
            }
        }
        // What only the other side has goes after its predecessor there, so an addition keeps its
        // place and a conflict copy stays next to its original. Without a predecessor it is
        // placed first. When both sides reordered
        // (C8), such nodes are simply appended.
        let mut out = place_secondary(&out, secondary, members, both_changed, &mut placed);
        let mut rest: Vec<NodeId> = members
            .iter()
            .copied()
            .filter(|id| !placed.contains(id))
            .collect();
        rest.sort_by_key(|id| rank[id]);
        out.extend(rest);
        (out, both_changed)
    }
}

/// `out` (the primary side's members, each once) with the members of `secondary` that are not
/// `placed` yet: each goes right after its predecessor in `secondary` (the node before it there,
/// placed earlier or just inserted), first when it has none, and at the end when `append` is set
/// (C8). Every inserted id is added to `placed`.
///
/// Linear in the lengths: the order is kept as a list linked by id, so an insertion after a
/// predecessor is a map update instead of a search plus a shift (a peer's 100 000 additions in a
/// folder this device reordered cost about 10^10 steps that way, M11 review #54). The result is the
/// same as searching the predecessor in the vector and inserting after it.
pub(super) fn place_secondary(
    out: &[NodeId],
    secondary: &[NodeId],
    members: &HashSet<NodeId>,
    append: bool,
    placed: &mut HashSet<NodeId>,
) -> Vec<NodeId> {
    // `next[id]` is the id after `id`; every listed id has an entry
    let mut next: HashMap<NodeId, Option<NodeId>> =
        HashMap::with_capacity(out.len() + secondary.len());
    for pair in out.windows(2) {
        next.insert(pair[0], Some(pair[1]));
    }
    let (mut first, mut tail) = (out.first().copied(), out.last().copied());
    if let Some(t) = tail {
        next.insert(t, None);
    }
    let mut last: Option<NodeId> = None;
    for &id in secondary {
        if placed.contains(&id) {
            last = Some(id);
            continue;
        }
        if !members.contains(&id) {
            continue;
        }
        match last.filter(|prev| !append && next.contains_key(prev)) {
            Some(prev) => {
                let after = next.insert(prev, Some(id)).flatten();
                next.insert(id, after);
                if tail == Some(prev) {
                    tail = Some(id);
                }
            }
            None if last.is_none() && !append => {
                next.insert(id, first);
                first = Some(id);
                if tail.is_none() {
                    tail = Some(id);
                }
            }
            None => {
                match tail {
                    Some(t) => {
                        next.insert(t, Some(id));
                    }
                    None => first = Some(id),
                }
                next.insert(id, None);
                tail = Some(id);
            }
        }
        placed.insert(id);
        last = Some(id);
    }
    let mut list = Vec::with_capacity(next.len());
    let mut at = first;
    while let Some(id) = at {
        list.push(id);
        at = next[&id];
    }
    list
}

fn plain(parent: Option<NodeId>) -> Choice {
    Choice {
        parent,
        local_move: false,
    }
}

/// Three-way merge of the connection tree (C1 to C10). `base` is the last synced tree, `local`
/// this device's tree and `head` the hub's. The head's version keeps the original id and place
/// whenever a rule makes a choice. A result that cannot be built is `snapshot_invalid`.
pub fn merge_tree(
    base: &Tree,
    local: &Tree,
    head: &Tree,
    ctx: &MergeContext,
) -> Result<TreeMerge, SyncError> {
    merge_tree_with(base, local, head, ctx, &mut NodeId::new_v4)
}

pub(crate) fn merge_tree_with(
    base: &Tree,
    local: &Tree,
    head: &Tree,
    ctx: &MergeContext,
    new_id: &mut dyn FnMut() -> NodeId,
) -> Result<TreeMerge, SyncError> {
    let (base_view, local_view, head_view) = (flatten(base)?, flatten(local)?, flatten(head)?);
    let local_alone = touched_alone(&base_view, &local_view);
    let head_alone = touched_alone(&base_view, &head_view);
    let local_touched = touched_subtrees(&local_view, &local_alone);
    let head_touched = touched_subtrees(&head_view, &head_alone);

    let mut merger = Merger {
        base: &base_view,
        local: &local_view,
        head: &head_view,
        order: Vec::new(),
        members: HashSet::new(),
        choices: HashMap::new(),
        placement_conflicts: Vec::new(),
        saved: Vec::new(),
        undone: Vec::new(),
    };

    // C5 to C7: which nodes exist
    let mut candidates = head_view.preorder.clone();
    candidates.extend(local_view.preorder.iter().copied());
    for id in candidates {
        let (in_local, in_head, in_base) = (
            local_view.nodes.contains_key(&id),
            head_view.nodes.contains_key(&id),
            base_view.nodes.contains_key(&id),
        );
        let keep = match (in_local, in_head, in_base) {
            (true, true, _) | (true, false, false) | (false, true, false) => true,
            (true, false, true) => local_touched.contains(&id),
            (false, true, true) => head_touched.contains(&id),
            (false, false, _) => false,
        };
        if keep && !merger.members.contains(&id) {
            if in_base && !(in_local && in_head) {
                merger.saved.push(id);
            }
            merger.admit(id);
        }
    }
    merger.repair()?;

    // A node kept only to hold a touched descendant that the placement rules then moved
    // elsewhere is an empty leftover: drop it (and, in turn, its own such parents).
    let alone_on_its_side = |id: &NodeId| match (
        local_view.nodes.contains_key(id),
        head_view.nodes.contains_key(id),
    ) {
        (true, false) => local_alone.contains(id),
        (false, true) => head_alone.contains(id),
        _ => true,
    };
    loop {
        let parents: HashSet<NodeId> = merger
            .order
            .iter()
            .filter_map(|id| merger.choices[id].parent)
            .collect();
        let leftovers: HashSet<NodeId> = merger
            .saved
            .iter()
            .copied()
            .filter(|id| !parents.contains(id) && !alone_on_its_side(id))
            .collect();
        if leftovers.is_empty() {
            break;
        }
        merger.order.retain(|id| !leftovers.contains(id));
        merger.members.retain(|id| !leftovers.contains(id));
        merger.saved.retain(|id| !leftovers.contains(id));
        merger.undone.retain(|id| !leftovers.contains(id));
        merger
            .placement_conflicts
            .retain(|id| !leftovers.contains(id));
        merger.choices.retain(|id, _| !leftovers.contains(id));
    }

    // C1 to C4: fields, and the conflict copies
    let mut owns: HashMap<NodeId, Own> = HashMap::new();
    let mut copies: Vec<(NodeId, NodeId)> = Vec::new(); // (original, copy)
    let mut taken: HashSet<NodeId> = merger.members.clone();
    taken.extend(base_view.nodes.keys());
    taken.extend(local_view.nodes.keys());
    taken.extend(head_view.nodes.keys());
    for id in &merger.order {
        let base_own = base_view.nodes.get(id).map(|f| &f.own);
        let own = match (local_view.nodes.get(id), head_view.nodes.get(id)) {
            (Some(l), Some(h)) => {
                let (merged, conflict) = merge_own(base_own, &l.own, &h.own);
                if conflict {
                    let mut copy = l.own.clone();
                    copy.name =
                        format!("{} (conflict {} {})", l.own.name, ctx.device_name, ctx.date);
                    // a folder copy is empty: only the original's id has children in `kids`
                    let mut copy_id = new_id();
                    let mut tries = 0;
                    while !taken.insert(copy_id) {
                        tries += 1;
                        if tries > 16 {
                            return Err(invalid("no fresh node id available"));
                        }
                        copy_id = new_id();
                    }
                    owns.insert(copy_id, copy);
                    copies.push((*id, copy_id));
                }
                merged
            }
            (Some(f), None) | (None, Some(f)) => f.own.clone(),
            (None, None) => return Err(invalid("a merged node has no source")),
        };
        owns.insert(*id, own);
    }

    // C8: sibling order
    let rank: HashMap<NodeId, usize> = merger
        .order
        .iter()
        .enumerate()
        .map(|(i, id)| (*id, i))
        .collect();
    let mut by_parent: HashMap<Option<NodeId>, HashSet<NodeId>> = HashMap::new();
    for id in &merger.order {
        by_parent
            .entry(merger.choices[id].parent)
            .or_default()
            .insert(*id);
    }
    let mut parents: Vec<Option<NodeId>> = by_parent.keys().copied().collect();
    parents.sort_by_key(|p| p.map_or(0, |id| rank[&id] + 1));
    let mut kids: HashMap<Option<NodeId>, Vec<NodeId>> = HashMap::new();
    let mut order_conflicts: Vec<NodeId> = Vec::new();
    for parent in parents {
        let (mut list, both_changed) = merger.order_siblings(parent, &by_parent[&parent], &rank);
        if both_changed && let Some(parent) = parent {
            order_conflicts.push(parent);
        }
        for (original, copy) in &copies {
            if let Some(at) = list.iter().position(|id| id == original) {
                list.insert(at + 1, *copy);
            }
        }
        kids.insert(parent, list);
    }

    // build
    for (parent, list) in &kids {
        if let Some(parent) = parent
            && !matches!(owns[parent].kind, Kind::Folder)
            && !list.is_empty()
        {
            return Err(invalid("a node would sit below a connection"));
        }
    }
    fn build(
        parent: Option<NodeId>,
        kids: &HashMap<Option<NodeId>, Vec<NodeId>>,
        owns: &HashMap<NodeId, Own>,
    ) -> Vec<Node> {
        kids.get(&parent).map_or_else(Vec::new, |list| {
            list.iter()
                .map(|id| to_node(*id, &owns[id], build(Some(*id), kids, owns)))
                .collect()
        })
    }
    let tree = Tree::from_children(build(None, &kids, &owns));
    if tree.depth() > MAX_TREE_DEPTH {
        return Err(invalid("the merged tree is nested too deeply"));
    }
    let placed: usize = kids.values().map(Vec::len).sum();
    if placed != owns.len() {
        return Err(invalid("the merged tree lost a node"));
    }

    // findings
    let name_of = |id: NodeId| owns[&id].name.clone();
    let mut findings = Vec::new();
    for (original, copy) in &copies {
        findings.push(Finding::ConflictCopy {
            original: *original,
            copy: *copy,
            name: name_of(*original),
        });
    }
    for id in &merger.saved {
        findings.push(Finding::KeptEdit {
            node: *id,
            name: name_of(*id),
        });
    }
    let undone: HashSet<NodeId> = merger.undone.iter().copied().collect();
    let mut seen = HashSet::new();
    for id in merger
        .placement_conflicts
        .iter()
        .filter(|id| !undone.contains(id))
        .chain(&order_conflicts)
    {
        if seen.insert(*id) {
            findings.push(Finding::PlacementKept {
                node: *id,
                name: name_of(*id),
            });
        }
    }
    for id in &merger.undone {
        findings.push(Finding::MoveUndone {
            node: *id,
            name: name_of(*id),
        });
    }
    Ok(TreeMerge { tree, findings })
}
