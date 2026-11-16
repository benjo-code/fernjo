use std::collections::HashSet;

use fernjo_core::persist;
use fernjo_core::{Node, NodeId, NodeKind, NodePatch, Patch, Protocol, Tree};
use fernjo_import::connections::{ConnectionsError, export, plan};
use serde_json::{Value, json};

const TODAY: &str = "2026-11-16";
const AT: &str = "2026-11-16T10:00:00Z";
const CRED: &str = "0b0f6a52-7c1e-4a3d-9a57-3a1f2b6f9c11";

fn tree_with_credentials() -> Tree {
    let mut t = Tree::new();
    let f = t.create_folder(None, None, "Lab").unwrap();
    let c = t
        .create_connection(Some(f), None, "web", Protocol::Ssh, "10.0.0.1")
        .unwrap();
    let r = t
        .create_connection(None, None, "desk", Protocol::Rdp, "10.0.0.2")
        .unwrap();
    for id in [f, c, r] {
        t.update(
            id,
            NodePatch {
                credential: Patch::Set(CRED.parse().unwrap()),
                ..NodePatch::default()
            },
        )
        .unwrap();
    }
    t.update(
        c,
        NodePatch {
            username: Patch::Set("ops".into()),
            port: Patch::Set(2200),
            ..NodePatch::default()
        },
    )
    .unwrap();
    t
}

fn ids(nodes: &[Node], out: &mut Vec<NodeId>) {
    for n in nodes {
        out.push(n.id);
        if let NodeKind::Folder { children } = &n.kind {
            ids(children, out);
        }
    }
}

/// The shape of a node list without ids and credentials.
fn shape(nodes: &[Node]) -> Value {
    Value::Array(
        nodes
            .iter()
            .map(|n| {
                let mut v = json!({
                    "name": n.name, "description": n.description,
                    "username": n.settings.username, "port": n.settings.port,
                    "keepalive": n.settings.keepalive_secs,
                });
                match &n.kind {
                    NodeKind::Folder { children } => v["children"] = shape(children),
                    NodeKind::Connection { protocol, host } => {
                        v["protocol"] = json!(format!("{protocol:?}"));
                        v["host"] = json!(host);
                    }
                }
                v
            })
            .collect(),
    )
}

#[test]
fn export_contains_no_credential_and_counts_the_dropped() {
    let out = export(&tree_with_credentials(), AT);
    assert_eq!(out.dropped_credentials, 3);
    let text = String::from_utf8(out.bytes).unwrap();
    assert!(!text.contains(CRED));
    assert!(!text.contains("credential"));
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["format"], "fernjo-connections");
    assert_eq!(v["exportedAt"], AT);
    assert_eq!(v["schemaVersion"], persist::CURRENT_VERSION);
    // stable, readable key order, pretty-printed
    let at = |k: &str| text.find(&format!("\"{k}\"")).unwrap();
    assert!(at("format") < at("exportedAt"));
    assert!(at("exportedAt") < at("schemaVersion"));
    assert!(at("schemaVersion") < at("children"));
    assert!(text.contains("\n  \"children\""));
}

#[test]
fn export_round_trips_through_plan_with_fresh_ids_and_same_shape() {
    let tree = tree_with_credentials();
    let out = export(&tree, AT);
    let p = plan(&out.bytes, TODAY, &[]).unwrap();
    assert_eq!(p.folder.name, "Imported from fernjo export (2026-11-16)");
    let NodeKind::Folder { children } = &p.folder.kind else {
        panic!("folder expected");
    };
    assert_eq!(shape(children), shape(tree.children()));

    let (mut old, mut new) = (Vec::new(), Vec::new());
    ids(tree.children(), &mut old);
    ids(std::slice::from_ref(&p.folder), &mut new);
    let old: HashSet<_> = old.into_iter().collect();
    assert_eq!(new.iter().collect::<HashSet<_>>().len(), new.len());
    assert!(new.iter().all(|i| !old.contains(i)));
    // the folder can be grafted next to the original tree
    let mut check = Tree::new();
    check.graft(None, p.folder.clone()).unwrap();

    let c = &p.report.counts;
    assert_eq!((c.folders, c.ssh, c.rdp, c.credentials), (1, 1, 1, 0));
    assert_eq!(c.partial, 0);
}

#[test]
fn plan_refuses_a_newer_schema() {
    let doc = json!({"format": "fernjo-connections", "exportedAt": AT, "schemaVersion": 99, "children": []});
    let err = plan(doc.to_string().as_bytes(), TODAY, &[]).err().unwrap();
    assert_eq!(
        err,
        ConnectionsError::NewerSchema {
            found: 99,
            supported: persist::CURRENT_VERSION
        }
    );
    assert_eq!(err.code(), "newer_schema");
}

#[test]
fn plan_refuses_json_without_the_format_marker() {
    let plain = persist::document(&tree_with_credentials()).to_string();
    let err = plan(plain.as_bytes(), TODAY, &[]).err().unwrap();
    assert_eq!(err, ConnectionsError::NotThisFormat);
    assert_eq!(err.code(), "not_this_format");
    let other = json!({"format": "something-else", "schemaVersion": 2, "children": []});
    assert_eq!(
        plan(other.to_string().as_bytes(), TODAY, &[])
            .err()
            .unwrap(),
        ConnectionsError::NotThisFormat
    );
}

#[test]
fn plan_refuses_a_confcons_xml_and_a_backup_as_not_this_format() {
    let xml = b"<?xml version=\"1.0\"?><Connections Name=\"x\" ConfVersion=\"2.6\"/>";
    assert_eq!(
        plan(xml, TODAY, &[]).err().unwrap(),
        ConnectionsError::NotThisFormat
    );
    let mut backup = b"FERNJOBK".to_vec();
    backup.extend_from_slice(&[0, 1, 0xff, 0xfe, 0x80, 7, 9]);
    assert_eq!(
        plan(&backup, TODAY, &[]).err().unwrap(),
        ConnectionsError::NotThisFormat
    );
    assert_eq!(
        plan(&[], TODAY, &[]).err().unwrap(),
        ConnectionsError::NotThisFormat
    );
    assert_eq!(
        plan(b"42", TODAY, &[]).err().unwrap(),
        ConnectionsError::NotThisFormat
    );
}

#[test]
fn plan_reports_invalid_ports_through_the_core_rules() {
    // The core refuses port 0 for the whole document and repairs nothing it could report per
    // node, so the import fails with the core's message.
    let doc = json!({"format": "fernjo-connections", "exportedAt": AT, "schemaVersion": persist::CURRENT_VERSION,
        "children": [{"id": "6f0c1c2a-0000-4000-8000-000000000001", "name": "bad", "type": "connection",
        "protocol": "ssh", "host": "h", "settings": {"port": 0}}]});
    let err = plan(doc.to_string().as_bytes(), TODAY, &[]).err().unwrap();
    assert_eq!(err.code(), "invalid");
    assert!(matches!(&err, ConnectionsError::Invalid(m) if m.contains("port 0")));
}

#[test]
fn plan_strips_credentials_of_a_hostile_file_and_counts_them() {
    let doc = json!({"format": "fernjo-connections", "exportedAt": AT, "schemaVersion": persist::CURRENT_VERSION,
        "children": [{"id": "6f0c1c2a-0000-4000-8000-000000000001", "name": "a", "type": "connection",
        "protocol": "ssh", "host": "h", "settings": {"credential": CRED}}]});
    let p = plan(doc.to_string().as_bytes(), TODAY, &[]).unwrap();
    assert_eq!(p.report.counts.partial, 1);
    assert_eq!(p.report.partial.len(), 1);
    assert!(p.report.partial[0].text.starts_with('1'));
    let dump = format!("{:?}{:?}", p.folder, p.report);
    assert!(!dump.contains(CRED));
}

#[test]
fn plan_numbers_the_folder_name() {
    let out = export(&Tree::new(), AT);
    let taken = vec!["Imported from fernjo export (2026-11-16)".to_owned()];
    let p = plan(&out.bytes, TODAY, &taken).unwrap();
    assert_eq!(
        p.folder.name,
        "Imported from fernjo export (2026-11-16) (2)"
    );
}

#[test]
fn plan_refuses_deep_nesting_and_huge_files_with_a_typed_error() {
    // beyond serde_json's own recursion limit
    let deep = format!(
        "{{\"format\":\"fernjo-connections\",\"schemaVersion\":2,\"children\":{}{}}}",
        "[".repeat(5000),
        "]".repeat(5000)
    );
    assert!(matches!(
        plan(deep.as_bytes(), TODAY, &[]).err().unwrap(),
        ConnectionsError::Invalid(_)
    ));
    // folders nested beyond depth 64 (each folder takes two JSON levels, so serde_json's limit
    // of 128 may already refuse it; either way the answer is a typed error)
    let mut node = json!({"id": "6f0c1c2a-0000-4000-8000-000000000001", "name": "leaf", "type": "folder", "children": []});
    for i in 0..70 {
        node = json!({"id": uuid::Uuid::new_v4().to_string(), "name": format!("f{i}"), "type": "folder", "children": [node]});
    }
    let doc = json!({"format": "fernjo-connections", "schemaVersion": 2, "children": [node]});
    let r = plan(doc.to_string().as_bytes(), TODAY, &[]);
    assert!(matches!(r, Err(ConnectionsError::Invalid(_))), "{r:?}");
    // too large
    let big = vec![b' '; 33 * 1024 * 1024];
    assert!(matches!(
        plan(&big, TODAY, &[]).err().unwrap(),
        ConnectionsError::Invalid(_)
    ));
}

#[test]
fn plan_refuses_more_than_the_node_limit() {
    let leaf = json!({"id": "6f0c1c2a-0000-4000-8000-000000000001", "name": "n", "type": "connection", "protocol": "ssh", "host": "h"});
    let doc = json!({"format": "fernjo-connections", "schemaVersion": 2,
        "children": vec![leaf; 100_001]});
    let r = plan(doc.to_string().as_bytes(), TODAY, &[]);
    assert!(matches!(r, Err(ConnectionsError::Invalid(_))), "{r:?}");
}

/// A connections document with `levels` nested folders (the innermost one empty).
fn folders(levels: usize) -> Value {
    let mut node = json!({"id": uuid::Uuid::new_v4().to_string(), "name": "f0", "type": "folder", "children": []});
    for i in 1..levels {
        node = json!({"id": uuid::Uuid::new_v4().to_string(), "name": format!("f{i}"), "type": "folder", "children": [node]});
    }
    json!({"format": "fernjo-connections", "schemaVersion": 2, "children": [node]})
}

#[test]
fn the_file_may_be_nested_to_the_limit_that_still_fits_below_the_import_folder() {
    use fernjo_import::mremoteng::file::MAX_DEPTH;
    // the import folder adds one level, so the plan reaches exactly the core's limit
    let ok = plan(folders(MAX_DEPTH).to_string().as_bytes(), TODAY, &[]).unwrap();
    assert_eq!(
        fernjo_core::subtree_depth(&ok.folder),
        fernjo_core::MAX_TREE_DEPTH
    );
    let err = plan(folders(MAX_DEPTH + 1).to_string().as_bytes(), TODAY, &[]).unwrap_err();
    assert!(
        matches!(&err, ConnectionsError::Invalid(m) if m.contains("nested")),
        "{err:?}"
    );
}
