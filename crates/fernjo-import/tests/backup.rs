use fernjo_core::{NodeKind, NodePatch, Patch, Protocol, Tree};
use fernjo_import::backup::{self, BackupContents, BackupError};
use fernjo_vault::{EntryId, KdfParams, NewEntry, Secret, seal_with_password};
use secrecy::{ExposeSecret, SecretString};

const PW: &str = "correct horse";

fn pw(s: &str) -> SecretString {
    SecretString::from(s.to_owned())
}

fn entry_id(n: u128) -> EntryId {
    EntryId::from_u128(n)
}

fn contents() -> BackupContents {
    let mut tree = Tree::new();
    let f = tree.create_folder(None, None, "Lab").unwrap();
    let c = tree
        .create_connection(Some(f), None, "web", Protocol::Ssh, "10.0.0.1")
        .unwrap();
    tree.update(
        c,
        NodePatch {
            credential: Patch::Set(entry_id(1)),
            username: Patch::Set("ops".into()),
            ..NodePatch::default()
        },
    )
    .unwrap();
    tree.create_connection(None, None, "desk", Protocol::Rdp, "10.0.0.2")
        .unwrap();
    BackupContents {
        created_at: "2026-11-16T10:00:00Z".into(),
        app: "fernjo 0.1.0".into(),
        tree,
        entries: vec![
            (
                entry_id(1),
                NewEntry {
                    name: "ops password".into(),
                    username: Some("ops".into()),
                    domain: Some("LAB".into()),
                    secret: Secret::Password {
                        password: pw("S3cret-pw-sentinel"),
                    },
                    notes: "line1\nline2".into(),
                },
            ),
            (
                entry_id(2),
                NewEntry {
                    name: "deploy key".into(),
                    username: None,
                    domain: None,
                    secret: Secret::PrivateKey {
                        key: pw("-----BEGIN KEY-----\nabc\n-----END KEY-----\n"),
                        passphrase: Some(pw("key-passphrase-sentinel")),
                    },
                    notes: String::new(),
                },
            ),
        ],
    }
}

fn make(c: &BackupContents) -> Vec<u8> {
    backup::create(c, &pw(PW), KdfParams::TEST).unwrap()
}

fn secret_text(s: &Secret) -> (String, Option<String>) {
    match s {
        Secret::Password { password } => (password.expose_secret().to_owned(), None),
        Secret::PrivateKey { key, passphrase } => (
            key.expose_secret().to_owned(),
            passphrase.as_ref().map(|p| p.expose_secret().to_owned()),
        ),
    }
}

fn seal(body: &[u8]) -> Vec<u8> {
    seal_with_password(backup::MAGIC, 1, &pw(PW), KdfParams::TEST, body).unwrap()
}

#[test]
fn round_trip_gives_identical_tree_and_entries() {
    let c = contents();
    let back = backup::open(&make(&c), &pw(PW)).unwrap();
    assert_eq!(back.tree, c.tree);
    assert_eq!(back.created_at, c.created_at);
    assert_eq!(back.app, c.app);
    assert_eq!(back.entries.len(), 2);
    for ((ia, a), (ib, b)) in c.entries.iter().zip(&back.entries) {
        assert_eq!(ia, ib);
        assert_eq!(a.name, b.name);
        assert_eq!(a.username, b.username);
        assert_eq!(a.domain, b.domain);
        assert_eq!(a.notes, b.notes);
        assert_eq!(secret_text(&a.secret), secret_text(&b.secret));
    }
}

#[test]
fn password_shorter_than_8_chars_is_refused_before_deriving() {
    // Parameters that would fail validation if reached prove no derivation happens.
    let bad = KdfParams {
        memory_kib: 1,
        ..KdfParams::TEST
    };
    let err = backup::create(&contents(), &pw("short77"), bad).unwrap_err();
    assert_eq!(err, BackupError::PasswordTooShort);
    assert_eq!(err.code(), "password_too_short");
    // Eight scalar values count, even when they are multi-byte.
    let eight = pw("\u{e4}\u{e4}\u{e4}\u{e4}\u{e4}\u{e4}\u{e4}\u{e4}");
    assert!(backup::create(&contents(), &eight, KdfParams::TEST).is_ok());
}

#[test]
fn wrong_password_is_wrong_password() {
    let err = backup::open(&make(&contents()), &pw("another pass")).unwrap_err();
    assert_eq!(err, BackupError::WrongPassword);
    assert_eq!(err.to_string(), "wrong password or the file is damaged");
}

#[test]
fn damaged_body_json_is_damaged() {
    for body in [&b"not json at all"[..], b"{\"schemaVersion\":1}", b"[]"] {
        let err = backup::open(&seal(body), &pw(PW)).unwrap_err();
        assert!(matches!(err, BackupError::Damaged(_)), "{err:?}");
        assert_eq!(err.code(), "damaged");
    }
    // A well-formed body whose tree is not valid.
    let body = serde_json::json!({
        "schemaVersion": 1, "createdAt": "x", "app": "y",
        "connections": {"schemaVersion": 2, "children": "not a list"}, "entries": []
    });
    let err = backup::open(&seal(body.to_string().as_bytes()), &pw(PW)).unwrap_err();
    assert!(matches!(err, BackupError::Damaged(_)), "{err:?}");
}

#[test]
fn a_newer_connections_schema_is_newer_version_not_damaged() {
    let body = serde_json::json!({
        "schemaVersion": 1, "createdAt": "x", "app": "y",
        "connections": {"schemaVersion": 4, "children": []}, "entries": []
    });
    let err = backup::open(&seal(body.to_string().as_bytes()), &pw(PW)).unwrap_err();
    assert_eq!(err, BackupError::NewerVersion { found: 4 });
    assert_eq!(err.code(), "newer_version");
    assert!(err.to_string().contains('4'), "{err}");
}

/// `levels` nested folders around one connection, as a `connections` document.
fn nested_document(levels: usize) -> serde_json::Value {
    let mut node = serde_json::json!({
        "id": uuid::Uuid::new_v4().to_string(), "name": "leaf", "type": "connection",
        "protocol": "ssh", "host": "h"
    });
    for i in 0..levels {
        node = serde_json::json!({
            "id": uuid::Uuid::new_v4().to_string(), "name": format!("f{i}"),
            "type": "folder", "children": [node]
        });
    }
    serde_json::json!({"schemaVersion": 2, "children": [node]})
}

fn body_with(connections: serde_json::Value) -> Vec<u8> {
    serde_json::json!({
        "schemaVersion": 1, "createdAt": "x", "app": "y",
        "connections": connections, "entries": []
    })
    .to_string()
    .into_bytes()
}

#[test]
fn a_tree_deeper_than_the_core_limit_is_damaged_and_the_limit_itself_opens() {
    let limit = fernjo_core::MAX_TREE_DEPTH;
    // the connection at level `limit`
    let ok = backup::open(&seal(&body_with(nested_document(limit - 1))), &pw(PW)).unwrap();
    assert_eq!(ok.tree.depth(), limit);
    // one level over, still within what the JSON parser accepts
    let err = backup::open(&seal(&body_with(nested_document(limit))), &pw(PW)).unwrap_err();
    assert!(matches!(err, BackupError::Damaged(_)), "{err:?}");
    assert!(err.to_string().contains("nested"), "{err}");
}

#[test]
fn a_body_nested_beyond_the_json_parsers_limit_is_damaged_without_overflowing_the_stack() {
    // built as text: a `Value` this deep would overflow the stack of the test itself
    let open =
        r#"{"id":"6f0c1c2a-0000-4000-8000-000000000001","name":"f","type":"folder","children":["#;
    let document = format!(
        r#"{{"schemaVersion":2,"children":[{}{}]}}"#,
        open.repeat(5000),
        "]}".repeat(5000)
    );
    let body = format!(
        r#"{{"schemaVersion":1,"createdAt":"x","app":"y","connections":{document},"entries":[]}}"#
    );
    let err = backup::open(&seal(body.as_bytes()), &pw(PW)).unwrap_err();
    assert!(matches!(err, BackupError::Damaged(_)), "{err:?}");
}

#[test]
fn truncated_file_is_damaged() {
    let bytes = make(&contents());
    for len in [10, 30, 62, 70] {
        let err = backup::open(&bytes[..len], &pw(PW)).unwrap_err();
        assert!(matches!(err, BackupError::Damaged(_)), "len {len}: {err:?}");
    }
}

#[test]
fn a_vault_file_or_connections_json_is_not_a_backup() {
    let mut vault = b"FJVAULT\0".to_vec();
    vault.extend_from_slice(&[0; 200]);
    for bytes in [
        vault,
        br#"{"schemaVersion":2,"children":[]}"#.to_vec(),
        vec![7u8; 300],
        Vec::new(),
    ] {
        assert_eq!(
            backup::open(&bytes, &pw(PW)).unwrap_err(),
            BackupError::NotABackup
        );
    }
}

#[test]
fn newer_version_is_reported() {
    let container = seal_with_password(backup::MAGIC, 2, &pw(PW), KdfParams::TEST, b"{}").unwrap();
    assert_eq!(
        backup::open(&container, &pw(PW)).unwrap_err(),
        BackupError::NewerVersion { found: 2 }
    );
    let body = br#"{"schemaVersion":2,"somethingNew":true}"#;
    assert_eq!(
        backup::open(&seal(body), &pw(PW)).unwrap_err(),
        BackupError::NewerVersion { found: 2 }
    );
}

#[test]
fn hostile_kdf_parameters_are_damaged() {
    let mut bytes = make(&contents());
    // Argon2id memory field at offset 10.
    bytes[10..14].copy_from_slice(&u32::MAX.to_be_bytes());
    let start = std::time::Instant::now();
    let err = backup::open(&bytes, &pw(PW)).unwrap_err();
    assert!(matches!(err, BackupError::Damaged(_)), "{err:?}");
    assert!(start.elapsed() < std::time::Duration::from_secs(2));
}

#[test]
fn entries_reference_ids_that_exist_in_the_tree_are_kept_verbatim() {
    fn walk(nodes: &[fernjo_core::Node], out: &mut Vec<String>) {
        for n in nodes {
            if let Some(id) = n.settings.credential {
                out.push(id.to_string());
            }
            if let NodeKind::Folder { children } = &n.kind {
                walk(children, out);
            }
        }
    }
    let back = backup::open(&make(&contents()), &pw(PW)).unwrap();
    let mut referenced = Vec::new();
    walk(back.tree.children(), &mut referenced);
    assert_eq!(referenced, vec![entry_id(1).to_string()]);
    assert_eq!(back.entries[0].0, entry_id(1));
    assert_eq!(back.entries[1].0, entry_id(2));
}

#[test]
fn debug_of_contents_hides_secrets() {
    let text = format!("{:?}", contents());
    assert!(!text.contains("sentinel"));
    assert!(!text.contains("BEGIN KEY"));
    let err = backup::open(&seal(b"S3cret-pw-sentinel"), &pw(PW)).unwrap_err();
    assert!(!format!("{err:?} {err}").contains("sentinel"));
}

#[test]
fn creating_twice_gives_different_bytes() {
    let c = contents();
    assert_ne!(make(&c), make(&c));
}

#[test]
fn long_backup_header_strings_are_cut_in_reports() {
    let body = serde_json::json!({
        "schemaVersion": 1,
        "createdAt": format!("2026\u{1b}[2J{}", "x".repeat(10_000)),
        "app": "é".repeat(10_000),
        "connections": {"schemaVersion": 2, "children": []}, "entries": []
    });
    let back = backup::open(&seal(body.to_string().as_bytes()), &pw(PW)).unwrap();
    assert_eq!(back.created_at.chars().count(), 256);
    assert!(!back.created_at.chars().any(char::is_control));
    assert_eq!(back.app.chars().count(), 256);
}

#[test]
fn backup_tree_text_is_capped_like_the_other_importers() {
    let long_name = "n".repeat(400);
    let body = serde_json::json!({
        "schemaVersion": 1, "createdAt": "x", "app": "y",
        "connections": {"schemaVersion": 2, "children": [
            {"id": "6f0c1c2a-0000-4000-8000-000000000001", "name": long_name, "type": "connection",
             "protocol": "ssh", "host": "h".repeat(300)}]},
        "entries": []
    });
    let back = backup::open(&seal(body.to_string().as_bytes()), &pw(PW)).unwrap();
    let n = &back.tree.children()[0];
    assert_eq!(n.name.chars().count(), 256);
    assert!(matches!(&n.kind, NodeKind::Connection { host, .. } if host.is_empty()));
}
