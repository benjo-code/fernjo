//! The owner's sanitised mRemoteNG 1.76 export (ConfVersion 2.6, default key). The expected data
//! was written by hand from the file's plain attributes and the owner's statement of the test
//! passwords, not produced by the importer. The passwords are test values only.

use fernjo_core::{Node, NodeKind, Protocol};
use fernjo_import::mremoteng::{plan, read};
use fernjo_vault::Secret;
use secrecy::ExposeSecret;
use serde_json::Value;

const TODAY: &str = "2026-11-16";
const FIXTURE: &[u8] = include_bytes!("fixtures/owner-confCons.xml");
const EXPECTED: &str = include_str!("fixtures/owner-confCons.expected.json");

fn flatten<'a>(n: &'a Node, prefix: &str, out: &mut Vec<(String, &'a Node)>) {
    let path = if prefix.is_empty() {
        n.name.clone()
    } else {
        format!("{prefix}/{}", n.name)
    };
    out.push((path.clone(), n));
    if let NodeKind::Folder { children } = &n.kind {
        for c in children {
            flatten(c, &path, out);
        }
    }
}

fn count(v: &Value, key: &str) -> usize {
    usize::try_from(v["counts"][key].as_u64().unwrap()).unwrap()
}

#[test]
fn owner_fixture_imports_with_the_expected_tree_and_passwords() {
    let expected: Value = serde_json::from_str(EXPECTED).unwrap();

    let doc = read(FIXTURE, None).expect("the fixture reads with the default key");
    fn no_password_error(nodes: &[fernjo_import::mremoteng::RawNode]) {
        for n in nodes {
            assert!(!n.password_error, "password_error on {}", n.name);
            no_password_error(&n.children);
        }
    }
    no_password_error(&doc.nodes);

    let p = plan(&doc, TODAY, &[]);

    let c = &p.report.counts;
    assert_eq!(c.folders, count(&expected, "folders"));
    assert_eq!(c.ssh, count(&expected, "ssh"));
    assert_eq!(c.rdp, count(&expected, "rdp"));
    assert_eq!(c.skipped, count(&expected, "skipped"));
    assert_eq!(c.credentials, count(&expected, "credentials"));
    assert_eq!(c.shared_credentials, count(&expected, "sharedCredentials"));
    assert_eq!(c.problems, count(&expected, "problems"));
    assert!(p.report.skipped.is_empty());
    assert!(p.report.problems.is_empty());

    // The tree, in document order, below the wrapper folder.
    let mut nodes = Vec::new();
    if let NodeKind::Folder { children } = &p.folder.kind {
        for ch in children {
            flatten(ch, "", &mut nodes);
        }
    }
    let want = expected["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), want.len());

    for ((path, node), w) in nodes.iter().zip(want) {
        assert_eq!(path, w["path"].as_str().unwrap());
        assert_eq!(node.name, w["name"].as_str().unwrap());
        assert_eq!(
            node.settings.port,
            w["port"].as_u64().map(|v| u16::try_from(v).unwrap()),
            "{path}"
        );
        assert_eq!(
            node.settings.username.as_deref(),
            w["username"].as_str(),
            "{path}"
        );
        match &node.kind {
            NodeKind::Folder { .. } => {
                assert_eq!(w["kind"], "folder", "{path}");
                assert!(node.settings.credential.is_none(), "{path}");
            }
            NodeKind::Connection { protocol, host } => {
                assert_eq!(w["kind"], "connection", "{path}");
                let proto = match protocol {
                    Protocol::Ssh => "ssh",
                    Protocol::Rdp => "rdp",
                };
                assert_eq!(proto, w["protocol"].as_str().unwrap(), "{path}");
                assert_eq!(host, w["host"].as_str().unwrap(), "{path}");
                let id = node.settings.credential.expect("own password -> entry");
                let entry = p.entries.iter().find(|e| e.id == id).expect("entry exists");
                let Secret::Password { password } = &entry.entry.secret else {
                    panic!("not a password entry");
                };
                assert_eq!(
                    password.expose_secret(),
                    w["password"].as_str().unwrap(),
                    "{path}"
                );
                assert_eq!(entry.entry.username.as_deref(), w["username"].as_str());
            }
        }
    }

    // The entry set, in creation order.
    let want_entries = expected["entries"].as_array().unwrap();
    assert_eq!(p.entries.len(), want_entries.len());
    for (e, w) in p.entries.iter().zip(want_entries) {
        assert_eq!(e.entry.name, w["name"].as_str().unwrap());
        assert_eq!(e.entry.username.as_deref(), w["username"].as_str());
        assert_eq!(e.entry.domain.as_deref(), w["domain"].as_str());
        let Secret::Password { password } = &e.entry.secret else {
            panic!("not a password entry");
        };
        assert_eq!(password.expose_secret(), w["password"].as_str().unwrap());
    }
}
