use fernjo_import::error::ImportError;
use fernjo_import::mremoteng::{RawDocument, RawKind, RawNode, read};
use secrecy::SecretString;
use serde_json::{Value, json};

const DEFAULT_KEY_XML: &[u8] = include_bytes!("fixtures/default-key.xml");
const CUSTOM_KEY_XML: &[u8] = include_bytes!("fixtures/custom-key.xml");
const FULL_FILE_XML: &[u8] = include_bytes!("fixtures/full-file.xml");

const CUSTOM_KEY: &str = "fixture-key-ünï";
const FULL_FILE_KEY: &str = "fixture-key";

fn key(s: &str) -> SecretString {
    SecretString::from(s.to_owned())
}

fn node_json(n: &RawNode) -> Value {
    json!({
        "kind": match &n.kind { RawKind::Container => "container", RawKind::Connection => "connection", RawKind::Unknown(_) => "unknown" },
        "name": n.name,
        "description": n.description,
        "hostname": n.hostname,
        "protocol": n.protocol,
        "port": n.port,
        "username": n.username,
        "domain": n.domain,
        "password": n.password.as_ref().map(|p| p.as_str()),
        "passwordError": n.password_error,
        "inherit": {
            "username": n.inherit.username,
            "password": n.inherit.password,
            "domain": n.inherit.domain,
            "port": n.inherit.port,
        },
        "puttySession": n.putty_session,
        "children": n.children.iter().map(node_json).collect::<Vec<_>>(),
    })
}

fn doc_json(d: &RawDocument) -> Value {
    Value::Array(d.nodes.iter().map(node_json).collect())
}

fn expected(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap()
}

/// The `Protected` attribute of the default-key fixture (iterations 1000, key mR3m).
fn default_protected() -> String {
    let text = std::str::from_utf8(DEFAULT_KEY_XML).unwrap();
    let doc = roxmltree::Document::parse(text).unwrap();
    doc.root_element()
        .attribute("Protected")
        .unwrap()
        .to_owned()
}

/// A small file that passes the key check with the default key.
fn small(iterations_attr: &str, body: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?><Connections Name="Connections" Export="false" EncryptionEngine="AES" BlockCipherMode="GCM" {iterations_attr} FullFileEncryption="false" Protected="{}" ConfVersion="2.6">{body}</Connections>"#,
        default_protected()
    )
}

fn small_ok(body: &str) -> String {
    small(r#"KdfIterations="1000""#, body)
}

fn nested(depth: usize) -> String {
    let mut body = String::new();
    for i in 0..depth {
        body.push_str(&format!(r#"<Node Name="c{i}" Type="Container">"#));
    }
    for _ in 0..depth {
        body.push_str("</Node>");
    }
    small_ok(&body)
}

#[test]
fn default_key_file_reads_names_hosts_and_passwords() {
    let doc = read(DEFAULT_KEY_XML, None).unwrap();
    assert_eq!(
        doc_json(&doc),
        expected(include_str!("fixtures/default-key.expected.json"))
    );
}

#[test]
fn custom_key_file_needs_the_key() {
    assert_eq!(
        read(CUSTOM_KEY_XML, None).unwrap_err(),
        ImportError::KeyNeeded
    );
    assert_eq!(
        read(CUSTOM_KEY_XML, Some(&key("wrong"))).unwrap_err(),
        ImportError::WrongKey
    );
    let doc = read(CUSTOM_KEY_XML, Some(&key(CUSTOM_KEY))).unwrap();
    assert_eq!(
        doc_json(&doc),
        expected(include_str!("fixtures/custom-key.expected.json"))
    );
}

#[test]
fn full_file_encryption_reads_the_same_tree() {
    assert_eq!(
        read(FULL_FILE_XML, None).unwrap_err(),
        ImportError::KeyNeeded
    );
    assert_eq!(
        read(FULL_FILE_XML, Some(&key("wrong"))).unwrap_err(),
        ImportError::WrongKey
    );
    let doc = read(FULL_FILE_XML, Some(&key(FULL_FILE_KEY))).unwrap();
    assert_eq!(
        doc_json(&doc),
        expected(include_str!("fixtures/full-file.expected.json"))
    );
    assert_eq!(
        doc_json(&doc),
        doc_json(&read(DEFAULT_KEY_XML, None).unwrap())
    );
}

#[test]
fn undecryptable_password_sets_password_error_and_keeps_going() {
    // 64 zero bytes: valid base64 and long enough, but the tag cannot verify.
    let bogus = "A".repeat(86) + "==";
    let body = format!(
        r#"<Node Name="a" Type="Connection" Hostname="h" Protocol="SSH2" Password="{bogus}"/>
           <Node Name="b" Type="Connection" Hostname="h" Protocol="SSH2" Password="not base64 !!"/>
           <Node Name="c" Type="Connection" Hostname="h" Protocol="SSH2" Password=""/>"#
    );
    let doc = read(small_ok(&body).as_bytes(), None).unwrap();
    assert_eq!(doc.nodes.len(), 3);
    assert!(doc.nodes[0].password_error && doc.nodes[0].password.is_none());
    assert!(doc.nodes[1].password_error && doc.nodes[1].password.is_none());
    assert!(!doc.nodes[2].password_error && doc.nodes[2].password.is_none());
}

#[test]
fn legacy_and_prerelease_versions_are_unsupported_with_their_version_in_the_message() {
    let legacy = include_bytes!("fixtures/legacy-2.5.xml");
    match read(legacy, None).unwrap_err() {
        ImportError::UnsupportedFormat(m) => assert!(m.contains("2.5"), "{m}"),
        e => panic!("{e:?}"),
    }
    let v27 = include_bytes!("fixtures/v2_7.xml");
    match read(v27, None).unwrap_err() {
        ImportError::UnsupportedFormat(m) => assert!(m.contains("2.7"), "{m}"),
        e => panic!("{e:?}"),
    }
}

#[test]
fn non_aes_gcm_engine_is_unsupported() {
    let text = std::str::from_utf8(DEFAULT_KEY_XML).unwrap();
    for (from, to, needle) in [
        (
            r#"EncryptionEngine="AES""#,
            r#"EncryptionEngine="Rijndael""#,
            "Rijndael",
        ),
        (
            r#"BlockCipherMode="GCM""#,
            r#"BlockCipherMode="CCM""#,
            "CCM",
        ),
    ] {
        let changed = text.replacen(from, to, 1);
        assert_ne!(changed, text);
        match read(changed.as_bytes(), None).unwrap_err() {
            ImportError::UnsupportedFormat(m) => assert!(m.contains(needle), "{m}"),
            e => panic!("{e:?}"),
        }
    }
}

#[test]
fn xml_that_is_not_connections_is_unsupported() {
    let err = read(include_bytes!("fixtures/not-connections.xml"), None).unwrap_err();
    assert!(matches!(err, ImportError::UnsupportedFormat(_)), "{err:?}");
}

#[test]
fn a_json_or_binary_file_is_unsupported_not_a_panic() {
    let json = br#"{"format":"fernjo-connections","version":1,"children":[]}"#;
    assert!(matches!(
        read(json, None).unwrap_err(),
        ImportError::UnsupportedFormat(_)
    ));
    let mut binary = b"FERNJOBK\x00\x01".to_vec();
    binary.extend((0u8..=255).cycle().take(5000));
    assert!(matches!(
        read(&binary, None).unwrap_err(),
        ImportError::UnsupportedFormat(_)
    ));
    assert!(matches!(
        read(b"", None).unwrap_err(),
        ImportError::UnsupportedFormat(_)
    ));
    // DOCTYPE is refused instead of expanded.
    let valid = small_ok("");
    let dtd = valid.replacen(
        "<Connections",
        r#"<!DOCTYPE Connections [<!ENTITY a "aaaa"><!ENTITY b "&a;&a;&a;&a;">]><Connections"#,
        1,
    );
    assert!(read(valid.as_bytes(), None).is_ok());
    assert!(matches!(
        read(dtd.as_bytes(), None).unwrap_err(),
        ImportError::UnsupportedFormat(_)
    ));
}

#[test]
fn file_over_32_mib_is_too_large() {
    let big = vec![b' '; 32 * 1024 * 1024 + 1];
    assert_eq!(read(&big, None).unwrap_err(), ImportError::TooLarge);
}

#[test]
fn depth_65_is_too_complex_and_depth_64_is_fine() {
    let ok = read(nested(64).as_bytes(), None).unwrap();
    let mut n = &ok.nodes[0];
    let mut depth = 1;
    while let Some(c) = n.children.first() {
        n = c;
        depth += 1;
    }
    assert_eq!(depth, 64);
    assert_eq!(
        read(nested(65).as_bytes(), None).unwrap_err(),
        ImportError::TooComplex
    );
    // Far deeper than the parser itself allows: still a typed error.
    let err = read(nested(5000).as_bytes(), None).unwrap_err();
    assert!(
        matches!(
            err,
            ImportError::TooComplex | ImportError::UnsupportedFormat(_)
        ),
        "{err:?}"
    );
}

#[test]
fn ten_thousand_siblings_are_fine_and_more_than_100_000_nodes_is_too_complex() {
    let node = r#"<Node Name="n" Type="Connection" Protocol="SSH2"/>"#;
    let ok = read(small_ok(&node.repeat(10_000)).as_bytes(), None).unwrap();
    assert_eq!(ok.nodes.len(), 10_000);
    let at_cap = read(small_ok(&node.repeat(100_000)).as_bytes(), None).unwrap();
    assert_eq!(at_cap.nodes.len(), 100_000);
    assert_eq!(
        read(small_ok(&node.repeat(100_001)).as_bytes(), None).unwrap_err(),
        ImportError::TooComplex
    );
}

#[test]
fn missing_attributes_read_as_defaults() {
    let body = r#"<Node Name="bare" Type="Connection"/><Node/>"#;
    let doc = read(small_ok(body).as_bytes(), None).unwrap();
    let n = &doc.nodes[0];
    assert_eq!(n.name, "bare");
    assert_eq!(n.kind, RawKind::Connection);
    assert!(n.description.is_empty() && n.hostname.is_empty() && n.protocol.is_empty());
    assert!(n.username.is_empty() && n.domain.is_empty() && n.putty_session.is_empty());
    assert_eq!(n.port, None);
    assert!(n.password.is_none() && !n.password_error);
    assert!(!n.inherit.username && !n.inherit.password && !n.inherit.domain && !n.inherit.port);
    assert_eq!(doc.nodes[1].kind, RawKind::Connection);
    assert!(doc.nodes[1].name.is_empty());
}

#[test]
fn odd_ports_are_reported_per_node_and_the_rest_still_reads() {
    let body = r#"<Node Name="a" Type="Connection" Port="twenty" Hostname="a.example"/>
        <Node Name="b" Type="Connection" Port="99999999999" Hostname="b.example"/>
        <Node Name="c" Type="Connection" Port="-1" Hostname="c.example"/>
        <Node Name="d" Type="Connection" Port="22" Hostname="d.example"/>
        <Node Name="e" Type="Connection" Port="" Hostname="e.example"/>"#;
    let doc = read(small_ok(body).as_bytes(), None).unwrap();
    assert_eq!(doc.nodes.len(), 5);
    let bad = ["twenty", "99999999999", "-1"];
    for (n, text) in doc.nodes.iter().zip(bad) {
        assert_eq!(n.port, None);
        assert_eq!(n.port_invalid.as_deref(), Some(text));
    }
    assert_eq!(doc.nodes[3].port, Some(22));
    assert_eq!(doc.nodes[3].port_invalid, None);
    assert_eq!(doc.nodes[4].port, None);
    assert_eq!(doc.nodes[4].port_invalid, None);
    assert_eq!(doc.nodes[3].hostname, "d.example");
}

#[test]
fn unknown_node_type_is_kept_with_its_children() {
    let body = r#"<Node Name="odd" Type="Folder"><Node Name="inner" Type="Connection" Hostname="h"/></Node>
        <Node Name="after" Type="Connection" Hostname="h2"/>"#;
    let doc = read(small_ok(body).as_bytes(), None).unwrap();
    assert_eq!(doc.nodes.len(), 2);
    assert_eq!(doc.nodes[0].kind, RawKind::Unknown("Folder".to_owned()));
    assert_eq!(doc.nodes[0].children.len(), 1);
    assert_eq!(doc.nodes[0].children[0].name, "inner");
    assert_eq!(doc.nodes[1].name, "after");
}

#[test]
fn hostile_kdf_iterations_are_a_typed_error_never_wrong_key() {
    for it in [
        "1",
        "999",
        "4000000000",
        "1000001",
        "abc",
        "",
        "-5",
        "99999999999",
    ] {
        let attr = format!(r#"KdfIterations="{it}""#);
        match read(small(&attr, "").as_bytes(), None).unwrap_err() {
            ImportError::Damaged(m) => {
                assert!(m.contains("KdfIterations"), "{m}");
            }
            e => panic!("iterations {it:?}: {e:?}"),
        }
    }
}

#[test]
fn missing_kdf_iterations_defaults_to_1000() {
    let doc = read(
        small("", r#"<Node Name="a" Type="Connection"/>"#).as_bytes(),
        None,
    )
    .unwrap();
    assert_eq!(doc.nodes.len(), 1);
}

#[test]
fn missing_protected_is_damaged() {
    let text = std::str::from_utf8(DEFAULT_KEY_XML).unwrap();
    let start = text.find(" Protected=\"").unwrap();
    let end = start + 12 + text[start + 12..].find('"').unwrap() + 1;
    let changed = format!("{}{}", &text[..start], &text[end..]);
    assert!(matches!(
        read(changed.as_bytes(), None).unwrap_err(),
        ImportError::Damaged(_)
    ));
}

#[test]
fn utf16_files_are_unsupported_with_a_hint() {
    for bom in [[0xFFu8, 0xFE], [0xFE, 0xFF]] {
        let mut bytes = bom.to_vec();
        for u in "<Connections ConfVersion=\"2.6\"/>".encode_utf16() {
            bytes.extend(if bom[0] == 0xFF {
                u.to_le_bytes()
            } else {
                u.to_be_bytes()
            });
        }
        match read(&bytes, None).unwrap_err() {
            ImportError::UnsupportedFormat(m) => assert!(m.contains("UTF-16"), "{m}"),
            e => panic!("{e:?}"),
        }
    }
}

#[test]
fn utf8_bom_is_accepted() {
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(DEFAULT_KEY_XML);
    assert_eq!(read(&bytes, None).unwrap().nodes.len(), 2);
}

#[test]
fn raw_node_debug_hides_the_password() {
    let doc = read(DEFAULT_KEY_XML, None).unwrap();
    let dump = format!("{doc:?}");
    for secret in ["folder-pw-1", "vnc-pw", "p@ss", "Grüße"] {
        assert!(!dump.contains(secret), "{secret} leaked: {dump}");
    }
    assert!(dump.contains("web1"));
}
