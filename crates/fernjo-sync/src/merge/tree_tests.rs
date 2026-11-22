use std::collections::HashSet;

use fernjo_core::{
    MAX_TREE_DEPTH, Node, NodeId, NodeKind, Protocol, RdpResolution, Settings, Tree,
};
use uuid::Uuid;

use super::tree::{TreeMerge, merge_tree_with};
use super::{Finding, MergeContext};

const DEVICE: &str = "laptop";
const DATE: &str = "2026-11-22";

fn id(n: u128) -> NodeId {
    Uuid::from_u128(n)
}

fn conn(n: u128, name: &str, host: &str) -> Node {
    Node {
        id: id(n),
        name: name.into(),
        description: String::new(),
        settings: Settings::default(),
        kind: NodeKind::Connection {
            protocol: Protocol::Ssh,
            host: host.into(),
        },
    }
}

fn folder(n: u128, name: &str, children: Vec<Node>) -> Node {
    Node {
        id: id(n),
        name: name.into(),
        description: String::new(),
        settings: Settings::default(),
        kind: NodeKind::Folder { children },
    }
}

fn tree(children: Vec<Node>) -> Tree {
    Tree::from_children(children)
}

fn ctx() -> MergeContext {
    MergeContext {
        device_name: DEVICE.into(),
        date: DATE.into(),
    }
}

/// Merges with copy ids 9001, 9002, ...
fn merge(base: &Tree, local: &Tree, head: &Tree) -> TreeMerge {
    let mut next = 9000;
    merge_tree_with(base, local, head, &ctx(), &mut || {
        next += 1;
        id(next)
    })
    .expect("the merge succeeds")
}

fn copy_name(name: &str) -> String {
    format!("{name} (conflict {DEVICE} {DATE})")
}

fn edit(tree: &Tree, n: u128, change: impl Fn(&mut Node)) -> Tree {
    fn walk(nodes: &mut [Node], target: NodeId, change: &dyn Fn(&mut Node)) -> bool {
        for node in nodes {
            if node.id == target {
                change(node);
                return true;
            }
            if let NodeKind::Folder { children } = &mut node.kind
                && walk(children, target, change)
            {
                return true;
            }
        }
        false
    }
    let mut children = tree.children().to_vec();
    assert!(walk(&mut children, id(n), &change), "node {n} exists");
    Tree::from_children(children)
}

fn rename(tree: &Tree, n: u128, name: &str) -> Tree {
    edit(tree, n, |node| node.name = name.into())
}

fn set_host(tree: &Tree, n: u128, host: &str) -> Tree {
    edit(tree, n, |node| {
        if let NodeKind::Connection { host: h, .. } = &mut node.kind {
            *h = host.into();
        }
    })
}

fn ids(nodes: &[Node]) -> Vec<NodeId> {
    nodes.iter().map(|n| n.id).collect()
}

fn children(tree: &Tree, n: u128) -> &[Node] {
    match &tree.get(id(n)).expect("node exists").kind {
        NodeKind::Folder { children } => children,
        NodeKind::Connection { .. } => panic!("not a folder"),
    }
}

fn all_ids(tree: &Tree) -> Vec<NodeId> {
    fn walk(nodes: &[Node], out: &mut Vec<NodeId>) {
        for node in nodes {
            out.push(node.id);
            if let NodeKind::Folder { children } = &node.kind {
                walk(children, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(tree.children(), &mut out);
    out
}

#[test]
fn c1_different_nodes_and_different_fields_are_both_kept() {
    let base = tree(vec![conn(1, "a", "h1"), conn(2, "b", "h2")]);
    // different nodes
    let local = rename(&base, 1, "a2");
    let head = set_host(&base, 2, "h2b");
    let merged = merge(&base, &local, &head);
    assert_eq!(
        merged.tree,
        tree(vec![conn(1, "a2", "h1"), conn(2, "b", "h2b")])
    );
    assert!(merged.findings.is_empty());
    // different fields of the same node
    let local = rename(&base, 1, "a2");
    let head = set_host(&base, 1, "h1b");
    let merged = merge(&base, &local, &head);
    assert_eq!(
        merged.tree,
        tree(vec![conn(1, "a2", "h1b"), conn(2, "b", "h2")])
    );
    assert!(merged.findings.is_empty());
}

#[test]
fn c2_a_field_changed_on_one_side_is_kept() {
    let base = tree(vec![conn(1, "a", "h1")]);
    let changed = rename(&base, 1, "renamed");
    let local_only = merge(&base, &changed, &base);
    assert_eq!(local_only.tree, changed);
    assert!(local_only.findings.is_empty());
    let head_only = merge(&base, &base, &changed);
    assert_eq!(head_only.tree, changed);
    assert!(head_only.findings.is_empty());
}

#[test]
fn c3_same_value_on_both_sides_is_no_conflict() {
    let base = tree(vec![conn(1, "a", "h1")]);
    let both = rename(&base, 1, "same");
    let merged = merge(&base, &both, &both);
    assert_eq!(merged.tree, both);
    assert!(merged.findings.is_empty());
}

#[test]
fn c4_different_values_keep_the_head_and_copy_the_local_version() {
    let base = tree(vec![conn(1, "a", "h1"), conn(2, "z", "hz")]);
    let local = set_host(&rename(&base, 1, "mine"), 1, "h-local");
    let head = rename(&base, 1, "theirs");
    let merged = merge(&base, &local, &head);
    // the head's name stays on the original (the head's host change did not happen: h1 stays);
    // the local version follows right after it with the local values of every own field
    let mut copy = conn(9001, &copy_name("mine"), "h-local");
    copy.id = id(9001);
    assert_eq!(
        merged.tree,
        tree(vec![conn(1, "theirs", "h-local"), copy, conn(2, "z", "hz")])
    );
    assert_eq!(
        merged.findings,
        vec![Finding::ConflictCopy {
            original: id(1),
            copy: id(9001),
            name: "theirs".into()
        }]
    );
}

#[test]
fn c4_a_conflicted_folder_copy_is_empty_and_its_children_stay() {
    let base = tree(vec![folder(1, "f", vec![conn(2, "child", "h")])]);
    let local = rename(&base, 1, "fl");
    let head = rename(&base, 1, "fh");
    let merged = merge(&base, &local, &head);
    assert_eq!(
        merged.tree,
        tree(vec![
            folder(1, "fh", vec![conn(2, "child", "h")]),
            folder(9001, &copy_name("fl"), vec![]),
        ])
    );
    assert_eq!(merged.findings.len(), 1);
}

#[test]
fn c5_deleted_on_one_side_and_unchanged_on_the_other_is_deleted() {
    let base = tree(vec![
        folder(
            1,
            "f",
            vec![conn(2, "a", "h"), folder(3, "g", vec![conn(4, "b", "h")])],
        ),
        conn(5, "c", "h"),
    ]);
    let gone = tree(vec![conn(5, "c", "h")]);
    let local_deleted = merge(&base, &gone, &base);
    assert_eq!(local_deleted.tree, gone);
    assert!(local_deleted.findings.is_empty());
    let head_deleted = merge(&base, &base, &gone);
    assert_eq!(head_deleted.tree, gone);
    assert!(head_deleted.findings.is_empty());
}

#[test]
fn c6_delete_versus_edit_keeps_the_node_and_its_folders() {
    // an edit beats a delete
    let base = tree(vec![conn(1, "a", "h"), conn(2, "b", "h")]);
    let head = tree(vec![conn(2, "b", "h")]);
    let local = set_host(&base, 1, "h-edited");
    let merged = merge(&base, &local, &head);
    assert_eq!(
        merged.tree,
        tree(vec![conn(1, "a", "h-edited"), conn(2, "b", "h")])
    );
    assert_eq!(
        merged.findings,
        vec![Finding::KeptEdit {
            node: id(1),
            name: "a".into()
        }]
    );

    // an edit below a deleted folder keeps the node and the folder; untouched siblings go
    let base = tree(vec![folder(
        1,
        "f",
        vec![conn(2, "edited", "h"), conn(3, "untouched", "h")],
    )]);
    let head = tree(vec![]);
    let local = set_host(&base, 2, "h2");
    let merged = merge(&base, &local, &head);
    assert_eq!(
        merged.tree,
        tree(vec![folder(1, "f", vec![conn(2, "edited", "h2")])])
    );
    let kept: HashSet<NodeId> = merged
        .findings
        .iter()
        .map(|f| match f {
            Finding::KeptEdit { node, .. } => *node,
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(kept, HashSet::from([id(1), id(2)]));

    // a node added under a deleted folder keeps the folder
    let local = tree(vec![folder(
        1,
        "f",
        vec![
            conn(2, "edited", "h"),
            conn(3, "untouched", "h"),
            conn(4, "new", "h"),
        ],
    )]);
    let merged = merge(&base, &local, &head);
    assert_eq!(
        merged.tree,
        tree(vec![folder(1, "f", vec![conn(4, "new", "h")])])
    );

    // and the same from the other side: the head edits, the local side deleted
    let merged = merge(&base, &head, &set_host(&base, 3, "h3"));
    assert_eq!(
        merged.tree,
        tree(vec![folder(1, "f", vec![conn(3, "untouched", "h3")])])
    );
}

#[test]
fn c7_nodes_added_on_either_side_are_kept() {
    // the head's order is the frame; a local node with no neighbour of its own leads
    let base = tree(vec![folder(1, "f", vec![])]);
    let local = tree(vec![folder(1, "f", vec![conn(10, "local", "h")])]);
    let head = tree(vec![
        folder(1, "f", vec![conn(20, "head", "h")]),
        conn(21, "top", "h"),
    ]);
    let merged = merge(&base, &local, &head);
    assert_eq!(
        merged.tree,
        tree(vec![
            folder(1, "f", vec![conn(10, "local", "h"), conn(20, "head", "h")]),
            conn(21, "top", "h")
        ])
    );
    assert!(merged.findings.is_empty());
}

#[test]
fn c8_one_sided_move_is_applied() {
    let base = tree(vec![
        folder(1, "f", vec![]),
        folder(2, "g", vec![]),
        conn(3, "c", "h"),
    ]);
    let moved = tree(vec![
        folder(1, "f", vec![conn(3, "c", "h")]),
        folder(2, "g", vec![]),
    ]);
    let local_move = merge(&base, &moved, &base);
    assert_eq!(local_move.tree, moved);
    assert!(local_move.findings.is_empty());
    let head_move = merge(&base, &base, &moved);
    assert_eq!(head_move.tree, moved);
    assert!(head_move.findings.is_empty());
}

#[test]
fn c8_move_to_different_parents_keeps_the_heads_parent() {
    let base = tree(vec![
        folder(1, "f", vec![]),
        folder(2, "g", vec![]),
        conn(3, "c", "h"),
    ]);
    let local = tree(vec![
        folder(1, "f", vec![conn(3, "c", "h")]),
        folder(2, "g", vec![]),
    ]);
    let head = tree(vec![
        folder(1, "f", vec![]),
        folder(2, "g", vec![conn(3, "c", "h")]),
    ]);
    let merged = merge(&base, &local, &head);
    assert_eq!(merged.tree, head);
    assert_eq!(
        merged.findings,
        vec![Finding::PlacementKept {
            node: id(3),
            name: "c".into()
        }]
    );
}

#[test]
fn c8_sibling_order_one_side_and_both_sides() {
    let base = tree(vec![
        conn(1, "a", "h"),
        conn(2, "b", "h"),
        conn(3, "c", "h"),
    ]);
    let abc = |order: [u128; 3]| {
        tree(
            order
                .iter()
                .map(|n| conn(*n, &base.get(id(*n)).unwrap().name, "h"))
                .collect(),
        )
    };
    // one side reordered: that order is taken
    let reordered = abc([3, 1, 2]);
    assert_eq!(merge(&base, &reordered, &base).tree, reordered);
    assert_eq!(merge(&base, &base, &reordered).tree, reordered);
    // both reordered differently: the head's order wins, local-only nodes are appended
    let local = tree(vec![
        conn(2, "b", "h"),
        conn(1, "a", "h"),
        conn(3, "c", "h"),
        conn(10, "local-new", "h"),
    ]);
    let head = abc([3, 2, 1]);
    let merged = merge(&base, &local, &head);
    assert_eq!(
        ids(merged.tree.children()),
        vec![id(3), id(2), id(1), id(10)]
    );
    // the top level has no node to name, so nothing is reported
    assert!(merged.findings.is_empty());
    // inside a folder the folder is named
    let wrap = |t: &Tree| tree(vec![folder(100, "box", t.children().to_vec())]);
    let merged = merge(&wrap(&base), &wrap(&local), &wrap(&head));
    assert_eq!(
        ids(children(&merged.tree, 100)),
        vec![id(3), id(2), id(1), id(10)]
    );
    assert_eq!(
        merged.findings,
        vec![Finding::PlacementKept {
            node: id(100),
            name: "box".into()
        }]
    );
}

#[test]
fn c9_a_cycle_from_two_moves_undoes_the_local_move() {
    let base = tree(vec![folder(1, "a", vec![]), folder(2, "b", vec![])]);
    let local = tree(vec![folder(1, "a", vec![folder(2, "b", vec![])])]);
    let head = tree(vec![folder(2, "b", vec![folder(1, "a", vec![])])]);
    let merged = merge(&base, &local, &head);
    assert_eq!(merged.tree, head);
    assert_eq!(
        merged.findings,
        vec![Finding::MoveUndone {
            node: id(2),
            name: "b".into()
        }]
    );
}

fn chain(first: u128, levels: u128, leaf: Option<Node>) -> Node {
    let mut node = leaf;
    for n in (first..first + levels).rev() {
        node = Some(folder(n, &format!("d{n}"), node.into_iter().collect()));
    }
    node.expect("at least one level")
}

#[test]
fn c9_depth_above_60_undoes_the_local_move() {
    // base: a chain of 55 folders and a separate subtree x (height 5)
    let x = chain(500, 5, None);
    let base = tree(vec![chain(1, 55, None), x.clone()]);
    // local moves x below the last folder: level 56 to 60
    let local = tree(vec![chain(1, 55, Some(x.clone()))]);
    assert_eq!(local.depth(), 60);
    // head wraps the chain in a new top-level folder: the chain now ends at level 56
    let head = tree(vec![folder(900, "w", vec![chain(1, 55, None)]), x.clone()]);
    let merged = merge(&base, &local, &head);
    assert_eq!(merged.tree, head);
    assert!(merged.tree.depth() <= MAX_TREE_DEPTH);
    assert_eq!(
        merged.findings,
        vec![Finding::MoveUndone {
            node: id(500),
            name: "d500".into()
        }]
    );
}

#[test]
fn c10_a_dangling_credential_reference_is_left_alone() {
    let reference = |n: u128| move |node: &mut Node| node.settings.credential = Some(id(n));
    let base = edit(
        &tree(vec![conn(1, "a", "h"), conn(2, "b", "h")]),
        1,
        reference(7000),
    );
    // the reference points nowhere (the merge has no vault); it stays, or changes as any field
    let local = edit(&base, 2, reference(7001));
    let merged = merge(&base, &local, &base);
    assert_eq!(merged.tree, local);
    assert!(merged.findings.is_empty());
    let head = edit(&base, 1, reference(7002));
    let merged = merge(&base, &local, &head);
    assert_eq!(merged.tree, edit(&local, 1, reference(7002)));
    assert!(merged.findings.is_empty());
}

fn fully_set() -> Tree {
    let mut web = conn(3, "web", "web.example");
    web.description = "notes".into();
    web.settings = Settings {
        username: Some("root".into()),
        port: Some(2222),
        keepalive_secs: Some(10),
        credential: Some(id(7000)),
        domain: Some("CORP".into()),
        rdp_resolution: Some(RdpResolution::Fixed {
            width: 800,
            height: 600,
        }),
        rdp_color_depth: Some(16),
        rdp_clipboard: Some(false),
    };
    let mut dir = folder(1, "dir", vec![web, conn(4, "db", "db.example")]);
    dir.description = "folder notes".into();
    dir.settings.username = Some("admin".into());
    tree(vec![dir, folder(2, "empty", vec![])])
}

#[test]
fn identical_inputs_change_nothing() {
    let t = fully_set();
    let merged = merge(&t, &t, &t);
    assert_eq!(merged.tree, t);
    assert!(merged.findings.is_empty());
    let empty = Tree::new();
    let merged = merge(&empty, &empty, &empty);
    assert_eq!(merged.tree, empty);
}

#[test]
fn every_own_field_merges_on_its_own() {
    type Change = fn(&mut Node);
    let changes: Vec<(&str, Change)> = vec![
        ("name", |n| n.name = "x".into()),
        ("description", |n| n.description = "x".into()),
        ("username", |n| n.settings.username = Some("x".into())),
        ("port", |n| n.settings.port = Some(1)),
        ("keepalive", |n| n.settings.keepalive_secs = Some(1)),
        ("credential", |n| n.settings.credential = Some(id(1))),
        ("domain", |n| n.settings.domain = Some("x".into())),
        ("resolution", |n| {
            n.settings.rdp_resolution = Some(RdpResolution::Fit)
        }),
        ("depth", |n| n.settings.rdp_color_depth = Some(24)),
        ("clipboard", |n| n.settings.rdp_clipboard = Some(true)),
        ("host", |n| {
            if let NodeKind::Connection { host, .. } = &mut n.kind {
                *host = "x".into();
            }
        }),
        ("protocol", |n| {
            if let NodeKind::Connection { protocol, .. } = &mut n.kind {
                *protocol = Protocol::Rdp;
            }
        }),
    ];
    let base = tree(vec![conn(1, "a", "h")]);
    for (i, (local_field, local_change)) in changes.iter().enumerate() {
        for (j, (head_field, head_change)) in changes.iter().enumerate() {
            let local = edit(&base, 1, local_change);
            let head = edit(&base, 1, head_change);
            let merged = merge(&base, &local, &head);
            if i == j {
                // the same change on both sides is no conflict
                assert_eq!(merged.tree, local, "{local_field}");
            } else {
                let expected = edit(&local, 1, head_change);
                assert_eq!(merged.tree, expected, "{local_field} with {head_field}");
            }
            assert!(
                merged.findings.is_empty(),
                "{local_field} with {head_field}"
            );
        }
    }
    // a conflicting setting makes a copy that carries the local value
    let local = edit(&base, 1, |n| n.settings.port = Some(1));
    let head = edit(&base, 1, |n| n.settings.port = Some(2));
    let merged = merge(&base, &local, &head);
    assert_eq!(merged.tree.get(id(1)).unwrap().settings.port, Some(2));
    assert_eq!(merged.tree.get(id(9001)).unwrap().settings.port, Some(1));
}

fn no_conflict_findings(findings: &[Finding]) -> bool {
    findings
        .iter()
        .all(|f| matches!(f, Finding::KeptEdit { .. }))
}

#[test]
fn merging_twice_is_idempotent() {
    // one scenario with a conflict copy, a placement conflict, a cycle, an order change on
    // both sides and a node kept against a delete
    let base = tree(vec![
        folder(1, "a", vec![]),
        folder(2, "b", vec![]),
        folder(3, "box", vec![conn(4, "x", "h"), conn(5, "y", "h")]),
        conn(6, "doomed", "h"),
        conn(7, "mover", "h"),
        conn(8, "name", "h"),
    ]);
    let local = tree(vec![
        folder(1, "a", vec![folder(2, "b", vec![])]),
        folder(
            3,
            "box",
            vec![conn(5, "y", "h"), conn(4, "x", "h"), conn(30, "new", "h")],
        ),
        folder(10, "pa", vec![conn(7, "mover", "h")]),
        folder(11, "pb", vec![]),
        conn(6, "doomed", "edited"),
        conn(8, "mine", "h"),
    ]);
    let head = tree(vec![
        folder(2, "b", vec![folder(1, "a", vec![])]),
        folder(
            3,
            "box",
            vec![
                conn(5, "y", "h"),
                conn(40, "head-new", "h"),
                conn(4, "x", "h"),
            ],
        ),
        folder(10, "pa", vec![]),
        folder(11, "pb", vec![conn(7, "mover", "h")]),
        conn(8, "theirs", "h"),
    ]);
    let first = merge(&base, &local, &head);
    assert!(
        first
            .findings
            .iter()
            .any(|f| matches!(f, Finding::ConflictCopy { .. }))
    );
    assert!(
        first
            .findings
            .iter()
            .any(|f| matches!(f, Finding::PlacementKept { .. }))
    );
    assert!(
        first
            .findings
            .iter()
            .any(|f| matches!(f, Finding::MoveUndone { .. }))
    );
    assert!(
        first
            .findings
            .iter()
            .any(|f| matches!(f, Finding::KeptEdit { .. }))
    );
    // run it again with the result as the local side, as after a crash before the push
    let second = merge(&base, &first.tree, &head);
    assert_eq!(
        second.tree, first.tree,
        "no second conflict copy, nothing moves"
    );
    // a conflict copy is only a local addition now; nothing else is reported again except a
    // node that is still kept against the head's delete (the same inputs, the same notice)
    assert!(
        no_conflict_findings(&second.findings),
        "{:?}",
        second.findings
    );
    // and a third time
    let third = merge(&base, &second.tree, &head);
    assert_eq!(third.tree, first.tree);
}

// ---- properties over random small trees ----

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % n as u64).unwrap()
    }
}

#[derive(Clone)]
struct Item {
    id: u128,
    parent: Option<u128>,
    folder: bool,
    name: String,
    host: String,
}

fn items_to_tree(items: &[Item]) -> Tree {
    fn level(items: &[Item], parent: Option<u128>) -> Vec<Node> {
        items
            .iter()
            .filter(|i| i.parent == parent)
            .map(|i| {
                if i.folder {
                    folder(i.id, &i.name, level(items, Some(i.id)))
                } else {
                    conn(i.id, &i.name, &i.host)
                }
            })
            .collect()
    }
    tree(level(items, None))
}

fn is_inside(items: &[Item], mut node: Option<u128>, ancestor: u128) -> bool {
    while let Some(n) = node {
        if n == ancestor {
            return true;
        }
        node = items.iter().find(|i| i.id == n).and_then(|i| i.parent);
    }
    false
}

fn mutate(items: &mut Vec<Item>, rng: &mut Rng, next_id: &mut u128) {
    if items.is_empty() {
        return;
    }
    let pick = rng.below(items.len());
    match rng.below(5) {
        0 => items[pick].name = format!("n{}", rng.below(1000)),
        1 => items[pick].host = format!("h{}", rng.below(1000)),
        2 => {
            let moving = items.remove(pick);
            let folders: Vec<Option<u128>> = std::iter::once(None)
                .chain(items.iter().filter(|i| i.folder).map(|i| Some(i.id)))
                .filter(|p| !is_inside(items, *p, moving.id))
                .collect();
            let parent = folders[rng.below(folders.len())];
            let at = rng.below(items.len() + 1);
            items.insert(at, Item { parent, ..moving });
        }
        3 => {
            let doomed = items[pick].id;
            let gone: Vec<u128> = items
                .iter()
                .filter(|i| is_inside(items, Some(i.id), doomed))
                .map(|i| i.id)
                .collect();
            items.retain(|i| !gone.contains(&i.id));
        }
        _ => {
            let folders: Vec<Option<u128>> = std::iter::once(None)
                .chain(items.iter().filter(|i| i.folder).map(|i| Some(i.id)))
                .collect();
            let parent = folders[rng.below(folders.len())];
            *next_id += 1;
            let at = rng.below(items.len() + 1);
            items.insert(
                at,
                Item {
                    id: *next_id,
                    parent,
                    folder: rng.below(3) == 0,
                    name: format!("new{next_id}"),
                    host: "h".into(),
                },
            );
        }
    }
}

fn random_base(rng: &mut Rng) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();
    for n in 1..=(3 + rng.below(8)) as u128 {
        let folders: Vec<Option<u128>> = std::iter::once(None)
            .chain(items.iter().filter(|i| i.folder).map(|i| Some(i.id)))
            .collect();
        let parent = folders[rng.below(folders.len())];
        let at = rng.below(items.len() + 1);
        items.insert(
            at,
            Item {
                id: n,
                parent,
                folder: rng.below(3) == 0,
                name: format!("base{n}"),
                host: "h".into(),
            },
        );
    }
    items
}

fn changed(rng: &mut Rng, base: &[Item], first_new_id: u128) -> Vec<Item> {
    let mut items = base.to_vec();
    let mut next_id = first_new_id;
    for _ in 0..rng.below(4) {
        mutate(&mut items, rng, &mut next_id);
    }
    items
}

#[test]
fn properties_hold_over_random_small_trees() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for round in 0..600 {
        let base_items = random_base(&mut rng);
        let local_items = changed(&mut rng, &base_items, 1000);
        let head_items = changed(&mut rng, &base_items, 2000);
        let (base, local, head) = (
            items_to_tree(&base_items),
            items_to_tree(&local_items),
            items_to_tree(&head_items),
        );
        let ctx = format!("round {round}");

        // one side alone, or both the same: the changed side wins unchanged
        assert_eq!(merge(&base, &local, &local).tree, local, "{ctx}");
        assert_eq!(merge(&base, &base, &local).tree, local, "{ctx}");
        assert_eq!(merge(&base, &local, &base).tree, local, "{ctx}");

        // both sides changed
        let merged = merge(&base, &local, &head);
        let result_ids = all_ids(&merged.tree);
        let unique: HashSet<NodeId> = result_ids.iter().copied().collect();
        assert_eq!(unique.len(), result_ids.len(), "duplicate id, {ctx}");
        assert!(merged.tree.depth() <= MAX_TREE_DEPTH, "{ctx}");

        // nothing that exists on a side is lost unless the other side deleted it untouched
        let base_ids: HashSet<NodeId> = all_ids(&base).into_iter().collect();
        for (side, other) in [(&local, &head), (&head, &local)] {
            for node in all_ids(side) {
                let on_both = other.get(node).is_some();
                let added = !base_ids.contains(&node);
                if on_both || added {
                    assert!(unique.contains(&node), "node lost, {ctx}");
                }
            }
        }

        // running it again with the result as the local side changes nothing
        let again = merge(&base, &merged.tree, &head);
        assert_eq!(
            again.tree,
            merged.tree,
            "not idempotent, {ctx}
base  {}
local {}
head  {}
first {}
again {}
{:?}",
            outline(&base),
            outline(&local),
            outline(&head),
            outline(&merged.tree),
            outline(&again.tree),
            merged.findings
        );
        assert!(
            again
                .findings
                .iter()
                .all(|f| matches!(f, Finding::KeptEdit { .. })),
            "{ctx}: {:?}",
            again.findings
        );
    }
}

#[test]
fn a_tree_with_a_repeated_id_is_snapshot_invalid() {
    let good = tree(vec![conn(1, "a", "h")]);
    let bad = tree(vec![conn(1, "a", "h"), conn(1, "b", "h")]);
    let ctx = ctx();
    for (base, local, head) in [
        (&bad, &good, &good),
        (&good, &bad, &good),
        (&good, &good, &bad),
    ] {
        let err = super::merge_tree(base, local, head, &ctx).unwrap_err();
        assert_eq!(err.code(), "snapshot_invalid");
    }
}

/// A compact one-line picture of a tree for failure messages: `id:name[children]`.
fn outline(tree: &Tree) -> String {
    fn walk(nodes: &[Node]) -> String {
        nodes
            .iter()
            .map(|n| match &n.kind {
                NodeKind::Folder { children } => {
                    format!("{}:{}[{}]", n.id.as_u128(), n.name, walk(children))
                }
                NodeKind::Connection { host, .. } => {
                    format!("{}:{}@{host}", n.id.as_u128(), n.name)
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
    walk(tree.children())
}
