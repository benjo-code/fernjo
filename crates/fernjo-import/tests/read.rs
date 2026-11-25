use fernjo_import::error::ImportError;
use fernjo_import::mremoteng::file::{MAX_DEPTH, MAX_PBKDF2_ITERATIONS};
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
fn key_errors_say_key_because_the_field_is_labelled_custom_key() {
    for e in [ImportError::KeyNeeded, ImportError::WrongKey] {
        let text = e.to_string();
        assert!(text.contains("key"), "{text}");
        assert!(!text.contains("password"), "{text}");
    }
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
fn the_depth_limit_leaves_room_for_the_import_folder() {
    // the plan puts everything below one new top-level folder, so the file may be one level
    // shallower than the deepest tree the core accepts
    assert_eq!(MAX_DEPTH + 1, fernjo_core::MAX_TREE_DEPTH);
}

#[test]
fn one_level_over_the_depth_limit_is_too_complex_and_the_limit_itself_is_fine() {
    let ok = read(nested(MAX_DEPTH).as_bytes(), None).unwrap();
    let mut n = &ok.nodes[0];
    let mut depth = 1;
    while let Some(c) = n.children.first() {
        n = c;
        depth += 1;
    }
    assert_eq!(depth, MAX_DEPTH);
    assert!(matches!(
        read(nested(MAX_DEPTH + 1).as_bytes(), None).unwrap_err(),
        ImportError::TooComplex(_)
    ));
    // Far deeper than the parser itself allows: still a typed error.
    let err = read(nested(5000).as_bytes(), None).unwrap_err();
    assert!(
        matches!(
            err,
            ImportError::TooComplex(_) | ImportError::UnsupportedFormat(_)
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
    assert!(matches!(
        read(small_ok(&node.repeat(100_001)).as_bytes(), None).unwrap_err(),
        ImportError::TooComplex(_)
    ));
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

// ---- work budget -----------------------------------------------------------------------------

/// The `Password` attribute of the default-key fixture's `web1` (decrypts at 1000 iterations).
fn web1_password() -> String {
    let text = std::str::from_utf8(DEFAULT_KEY_XML).unwrap();
    let doc = roxmltree::Document::parse(text).unwrap();
    doc.descendants()
        .find(|n| n.attribute("Name") == Some("web1"))
        .and_then(|n| n.attribute("Password"))
        .unwrap()
        .to_owned()
}

/// `count` connections that each carry a (garbage) `Password`.
fn with_passwords(iterations: u32, count: usize) -> String {
    let node = r#"<Node Name="n" Type="Connection" Protocol="SSH2" Password="AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"/>"#;
    small(
        &format!(r#"KdfIterations="{iterations}""#),
        &node.repeat(count),
    )
}

#[test]
fn a_hostile_password_count_is_too_complex_before_any_derivation() {
    // 1 key check + 19 passwords = 20 derivations of 1 000 000 iterations = the budget;
    // one more password is over it. The root's `Protected` was made for 1000 iterations, so
    // a derivation that reached it would end as `key_needed`, never as `too_complex`.
    let start = std::time::Instant::now();
    let err = read(with_passwords(1_000_000, 25).as_bytes(), None).unwrap_err();
    let ImportError::TooComplex(message) = &err else {
        panic!("{err:?}");
    };
    assert!(message.contains("25"), "{message}");
    assert!(message.contains("1000000"), "{message}");
    assert!(
        start.elapsed() < std::time::Duration::from_secs(1),
        "the check must not derive anything ({:?})",
        start.elapsed()
    );
    assert_eq!(err.code(), "too_complex");
}

#[test]
fn the_budget_edge_is_inclusive() {
    let per = u64::from(1_000_000u32);
    let allowed = (MAX_PBKDF2_ITERATIONS / per) as usize - 1; // minus the key check
    // within the budget: the file gets as far as the key check (which fails for 1000-iteration
    // `Protected`)
    assert_eq!(
        read(with_passwords(1_000_000, allowed).as_bytes(), None).unwrap_err(),
        ImportError::KeyNeeded
    );
    assert!(matches!(
        read(with_passwords(1_000_000, allowed + 1).as_bytes(), None).unwrap_err(),
        ImportError::TooComplex(_)
    ));
}

#[test]
fn a_realistic_file_with_500_passwords_is_unaffected_by_the_budget() {
    let node = format!(
        r#"<Node Name="n" Type="Connection" Protocol="SSH2" Password="{}"/>"#,
        web1_password()
    );
    let doc = read(small_ok(&node.repeat(500)).as_bytes(), None).unwrap();
    assert_eq!(doc.nodes.len(), 500);
    assert!(doc.nodes.iter().all(|n| n.password.is_some()));
}

/// Encrypts like mRemoteNG does (test helper; fixed salt and nonce are fine here).
fn encrypt(plain: &str, password: &str, iterations: u32) -> String {
    use aes_gcm::aead::consts::U16;
    use aes_gcm::aead::{Aead, KeyInit, Payload};
    use aes_gcm::aes::Aes256;
    use base64::Engine;
    let salt = [7u8; 16];
    let nonce = [9u8; 16];
    let mut derived = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(password.as_bytes(), &salt, iterations, &mut derived);
    let cipher = aes_gcm::AesGcm::<Aes256, U16>::new_from_slice(&derived).unwrap();
    let sealed = cipher
        .encrypt(
            (&nonce).into(),
            Payload {
                msg: plain.as_bytes(),
                aad: &salt,
            },
        )
        .unwrap();
    let mut raw = salt.to_vec();
    raw.extend_from_slice(&nonce);
    raw.extend_from_slice(&sealed);
    base64::engine::general_purpose::STANDARD.encode(raw)
}

fn full_file(inner: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?><Connections Name="Connections" Export="false" EncryptionEngine="AES" BlockCipherMode="GCM" KdfIterations="1000" FullFileEncryption="true" Protected="{}" ConfVersion="2.6">{}</Connections>"#,
        encrypt("ThisIsNotProtected", "mR3m", 1000),
        encrypt(inner, "mR3m", 1000)
    )
}

#[test]
fn full_file_encryption_is_counted_after_the_body_is_decrypted() {
    let node = r#"<Node Name="n" Type="Connection" Protocol="SSH2" Password="AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"/>"#;
    // 1000 iterations: key check + body + n passwords must stay within 20 000 000 / 1000
    let limit = (MAX_PBKDF2_ITERATIONS / 1000) as usize;
    let ok = read(full_file(&node.repeat(limit - 2)).as_bytes(), None).unwrap();
    assert_eq!(ok.nodes.len(), limit - 2);
    let err = read(full_file(&node.repeat(limit - 1)).as_bytes(), None).unwrap_err();
    assert!(matches!(err, ImportError::TooComplex(_)), "{err:?}");
}

// ---- hostile XML shapes (security review H1): refused before roxmltree parses them ----

use fernjo_import::mremoteng::file::{MAX_ATTRIBUTES_PER_TAG, MAX_XMLNS_DECLARATIONS};
use std::time::{Duration, Instant};

/// Time bound for refusing a hostile file (debug build; the real cost is a few milliseconds).
const REFUSE_WITHIN: Duration = Duration::from_secs(1);

fn refused_quickly(text: &str) -> ImportError {
    let start = Instant::now();
    let err = read(text.as_bytes(), None).unwrap_err();
    let took = start.elapsed();
    assert!(took < REFUSE_WITHIN, "took {took:?}");
    err
}

fn attributes(n: usize) -> String {
    (0..n).map(|i| format!(r#" a{i}="""#)).collect()
}

#[test]
fn one_tag_with_tens_of_thousands_of_attributes_is_refused_quickly() {
    // review H1: N = 100 000 took 21 s; 30 000 keeps the red run short
    let body = format!(
        r#"<Node Name="n" Type="Connection" Protocol="SSH2"{}/>"#,
        attributes(30_000)
    );
    let err = refused_quickly(&small_ok(&body));
    assert!(matches!(err, ImportError::TooComplex(_)), "{err:?}");
    assert_eq!(err.code(), "too_complex");
}

#[test]
fn the_attribute_cap_is_an_exact_boundary() {
    // Name, Type and Protocol are three of them
    let at = MAX_ATTRIBUTES_PER_TAG - 3;
    let ok = format!(
        r#"<Node Name="n" Type="Connection" Protocol="SSH2"{}/>"#,
        attributes(at)
    );
    assert_eq!(read(small_ok(&ok).as_bytes(), None).unwrap().nodes.len(), 1);
    let over = format!(
        r#"<Node Name="n" Type="Connection" Protocol="SSH2"{}/>"#,
        attributes(at + 1)
    );
    assert!(matches!(
        read(small_ok(&over).as_bytes(), None).unwrap_err(),
        ImportError::TooComplex(_)
    ));
}

#[test]
fn nested_elements_with_their_own_namespace_declarations_are_refused_quickly() {
    // review H1: 2 500 nested `<a xmlns:pN="uN" pN:x="1">` took 6.3 s, 5 000 took 55 s
    let n = 2_500;
    let mut body = String::new();
    for i in 0..n {
        body.push_str(&format!(r#"<a xmlns:p{i}="u{i}" p{i}:x="1">"#));
    }
    for _ in 0..n {
        body.push_str("</a>");
    }
    let err = refused_quickly(&small_ok(&body));
    assert!(matches!(err, ImportError::TooComplex(_)), "{err:?}");
}

#[test]
fn the_namespace_cap_is_an_exact_boundary_and_counts_the_whole_file() {
    // the root of `small_ok` has none; spread the declarations over many elements
    let decl = |n: usize| -> String {
        (0..n)
            .map(|i| format!(r#"<x xmlns:q{i}="u{i}"/>"#))
            .collect()
    };
    let ok = small_ok(&decl(MAX_XMLNS_DECLARATIONS));
    assert!(read(ok.as_bytes(), None).is_ok());
    let over = small_ok(&decl(MAX_XMLNS_DECLARATIONS + 1));
    assert!(matches!(
        read(over.as_bytes(), None).unwrap_err(),
        ImportError::TooComplex(_)
    ));
    // a default namespace declaration counts too
    let defaults: String = (0..=MAX_XMLNS_DECLARATIONS)
        .map(|i| format!(r#"<x xmlns="u{i}"/>"#))
        .collect();
    assert!(matches!(
        read(small_ok(&defaults).as_bytes(), None).unwrap_err(),
        ImportError::TooComplex(_)
    ));
}

#[test]
fn a_large_but_legitimate_file_is_still_read() {
    // 5 000 nodes with 60 attributes each and one namespace declaration on the root, like the
    // owner's real file
    let node = format!(
        r#"<Node Name="n" Type="Connection" Protocol="SSH2" Hostname="h"{}/>"#,
        attributes(55)
    );
    let text = small_ok(&node.repeat(5_000)).replacen(
        "<Connections ",
        r#"<Connections xmlns:mrng="http://mremoteng.org" "#,
        1,
    );
    let doc = read(text.as_bytes(), None).unwrap();
    assert_eq!(doc.nodes.len(), 5_000);
}

#[test]
fn xmlns_and_markup_inside_values_comments_and_cdata_do_not_count() {
    let noise = r#"title="xmlns:a xmlns:b xmlns:c xmlns:d xmlns:e xmlns:f xmlns:g xmlns:h xmlns:i > '" other='" xmlns:j'"#;
    let body = format!(
        r#"<!-- xmlns:a="1" xmlns:b="2" xmlns:c="3" xmlns:d="4" xmlns:e="5" xmlns:f="6" xmlns:g="7" xmlns:h="8" xmlns:i="9" --><Node Name="n" Type="Connection" Protocol="SSH2" {noise}/><![CDATA[ <x xmlns:a="1"/> xmlns xmlns xmlns xmlns xmlns xmlns xmlns xmlns xmlns xmlns ]]>"#
    );
    assert_eq!(
        read(small_ok(&body).as_bytes(), None).unwrap().nodes.len(),
        1
    );
}

#[test]
fn hostile_shapes_hidden_behind_decoys_are_still_refused() {
    // a comment, CDATA and quoted `>` / `<!--` / `]]>` in front must not hide the real tag
    let decoys = r#"<!-- <a q=" --><![CDATA[ <a q=" ]]><Node Name="<!-- ]]> -->" Type='"' Protocol="SSH2"/>"#;
    let body = format!(
        r#"{decoys}<Node Name="n" Type="Connection" Protocol="SSH2"{}/>"#,
        attributes(MAX_ATTRIBUTES_PER_TAG + 1)
    );
    assert!(matches!(
        read(small_ok(&body).as_bytes(), None).unwrap_err(),
        ImportError::TooComplex(_)
    ));
    // an attribute value with the other kind of quote and a `>` before the many attributes
    let body = format!(
        r#"<Node Name='a"b>c' Type="Connection" Protocol="SSH2"{}/>"#,
        attributes(MAX_ATTRIBUTES_PER_TAG + 1)
    );
    assert!(matches!(
        read(small_ok(&body).as_bytes(), None).unwrap_err(),
        ImportError::TooComplex(_)
    ));
}

#[test]
fn a_hostile_full_file_body_is_refused_after_decryption() {
    // the default key is tried without a typed key, so anybody can build such a file
    let inner = format!(
        r#"<Node Name="n" Type="Connection" Protocol="SSH2"{}/>"#,
        attributes(5_000)
    );
    let start = Instant::now();
    let err = read(full_file(&inner).as_bytes(), None).unwrap_err();
    assert!(start.elapsed() < REFUSE_WITHIN);
    assert!(matches!(err, ImportError::TooComplex(_)), "{err:?}");
}

// ---- string caps (security review H2) ----

use fernjo_import::mremoteng::file::{
    MAX_DESCRIPTION_CHARS, MAX_HOST_CHARS, MAX_IDENTITY_CHARS, MAX_NAME_CHARS, MAX_PASSWORD_CHARS,
    MAX_PROTOCOL_CHARS,
};
use fernjo_import::mremoteng::plan;

fn one_node(attrs: &str) -> RawNode {
    let body = format!(r#"<Node Type="Connection" Protocol="SSH2" {attrs}/>"#);
    read(small_ok(&body).as_bytes(), None)
        .unwrap()
        .nodes
        .remove(0)
}

#[test]
fn values_exactly_at_their_caps_are_read_unchanged() {
    let name = "ñ".repeat(MAX_NAME_CHARS);
    let host = "h".repeat(MAX_HOST_CHARS);
    let user = "u".repeat(MAX_IDENTITY_CHARS);
    let domain = "d".repeat(MAX_IDENTITY_CHARS);
    let descr = "x".repeat(MAX_DESCRIPTION_CHARS);
    let n = one_node(&format!(
        r#"Name="{name}" Hostname="{host}" Username="{user}" Domain="{domain}" Descr="{descr}""#
    ));
    assert_eq!(n.name, name);
    assert_eq!(n.hostname, host);
    assert_eq!(n.username, user);
    assert_eq!(n.domain, domain);
    assert_eq!(n.description, descr);
    assert_eq!(n.clipped, Default::default());
    // and through the mapper into the imported node
    let p = plan(&RawDocument { nodes: vec![n] }, "2026-11-18", &[]);
    let fernjo_core::NodeKind::Folder { children } = &p.folder.kind else {
        panic!("folder")
    };
    assert_eq!(children[0].name, name);
    assert_eq!(children[0].description, descr);
    // nothing was cut, so no line says so (the domain line is the unrelated "no password" one)
    let lines = p.report.partial.iter().chain(&p.report.problems);
    assert!(lines.clone().all(|l| !l.text.contains("longer than")));
    assert_eq!(lines.count(), 1);
}

#[test]
fn values_over_their_caps_are_cut_and_flagged() {
    let n = one_node(&format!(
        r#"Name="{}" Hostname="{}" Username="{}" Domain="{}" Descr="{}""#,
        "n".repeat(MAX_NAME_CHARS + 1),
        "h".repeat(MAX_HOST_CHARS + 1),
        "u".repeat(MAX_IDENTITY_CHARS + 1),
        "d".repeat(MAX_IDENTITY_CHARS + 1),
        "x".repeat(MAX_DESCRIPTION_CHARS + 1),
    ));
    assert_eq!(n.name.chars().count(), MAX_NAME_CHARS);
    assert_eq!(n.username.chars().count(), MAX_IDENTITY_CHARS);
    assert_eq!(n.domain.chars().count(), MAX_IDENTITY_CHARS);
    assert_eq!(n.description.chars().count(), MAX_DESCRIPTION_CHARS);
    // a host that is too long to be one is dropped, never cut into another host
    assert_eq!(n.hostname, "");
    assert!(n.clipped.name && n.clipped.hostname && n.clipped.username);
    assert!(n.clipped.domain && n.clipped.description && !n.clipped.protocol);
}

#[test]
fn control_characters_are_removed_from_a_username_and_an_empty_one_is_unset() {
    let a = one_node(r#"Name="a" Hostname="h" Username="a&#10;b&#9;c""#);
    assert_eq!(a.username, "abc");
    let b = one_node(r#"Name="b" Hostname="h" Username="&#9;""#);
    assert_eq!(b.username, "");
    // through the mapper into the imported settings
    let p = plan(&RawDocument { nodes: vec![a, b] }, "2026-11-18", &[]);
    let fernjo_core::NodeKind::Folder { children } = &p.folder.kind else {
        panic!("folder")
    };
    assert_eq!(children[0].settings.username.as_deref(), Some("abc"));
    assert_eq!(children[1].settings.username, None);
}

#[test]
fn control_and_direction_characters_are_removed_from_names_hosts_descriptions_and_domains() {
    // character references can carry tab, line feed, DEL, C1 controls and bidi overrides
    let n = one_node(
        r#"Name="a&#10;b&#x85;c&#x9B;d&#x7F;e&#x202E;f" Hostname="h&#10;o&#x202E;st" Descr="l1&#10;l2&#x9B;&#x202E;x" Domain="EX&#x7F;AM&#x202E;PLE""#,
    );
    assert_eq!(n.name, "abcdef");
    assert_eq!(n.hostname, "host");
    // a description keeps its line breaks
    assert_eq!(n.description, "l1\nl2x");
    assert_eq!(n.domain, "EXAMPLE");
    // through the mapper: none of it reaches the imported tree
    let p = plan(&RawDocument { nodes: vec![n] }, "2026-11-18", &[]);
    let fernjo_core::NodeKind::Folder { children } = &p.folder.kind else {
        panic!("folder")
    };
    assert_eq!(children[0].name, "abcdef");
    assert!(
        matches!(&children[0].kind, fernjo_core::NodeKind::Connection { host, .. } if host == "host")
    );
    // a name that is nothing but controls reads as unnamed
    let m = one_node(r#"Name="&#x202E;&#9;" Hostname="h""#);
    let p = plan(&RawDocument { nodes: vec![m] }, "2026-11-18", &[]);
    let fernjo_core::NodeKind::Folder { children } = &p.folder.kind else {
        panic!("folder")
    };
    assert_eq!(children[0].name, "Unnamed");
}

#[test]
fn an_overlong_protocol_is_flagged_and_an_overlong_password_is_not_decrypted() {
    let n = one_node(&format!(
        r#"Name="n" Password="{}""#,
        "A".repeat(MAX_PASSWORD_CHARS + 1)
    ));
    assert!(n.password.is_none() && n.password_error);
    let body = format!(
        r#"<Node Name="n" Type="Connection" Protocol="SSH2{}x"/>"#,
        " ".repeat(MAX_PROTOCOL_CHARS)
    );
    let n = read(small_ok(&body).as_bytes(), None)
        .unwrap()
        .nodes
        .remove(0);
    assert!(n.clipped.protocol);
    assert!(n.protocol.chars().count() <= MAX_PROTOCOL_CHARS);
}

#[test]
fn a_file_of_huge_names_reads_and_plans_quickly() {
    // review input A end to end, scaled down: 2 MB name, 10 000 unsupported children
    let kids = r#"<Node Name="x" Type="Connection" Protocol="X"/>"#.repeat(10_000);
    let body = format!(
        r#"<Node Name="{}" Type="Container">{kids}</Node>"#,
        "N".repeat(2_000_000)
    );
    let start = Instant::now();
    let doc = read(small_ok(&body).as_bytes(), None).unwrap();
    let p = plan(&doc, "2026-11-18", &[]);
    let took = start.elapsed();
    assert!(took < Duration::from_secs(2), "took {took:?}");
    assert_eq!(p.report.counts.skipped, 10_000);
    // the cut name is reported once, not once per child
    assert!(
        p.report
            .partial
            .iter()
            .any(|l| l.text.contains("shortened to"))
    );
}

#[test]
fn resolution_colors_and_their_inherit_flags_are_read() {
    let n = one_node(r#"Resolution="Res1920x1080" Colors="Colors32Bit" InheritResolution="true""#);
    assert_eq!(n.resolution, "Res1920x1080");
    assert_eq!(n.colors, "Colors32Bit");
    assert!(n.inherit.resolution);
    assert!(!n.inherit.colors);
    let none = one_node(r#"Name="n""#);
    assert!(none.resolution.is_empty() && none.colors.is_empty());
}

#[test]
fn an_overlong_resolution_or_colors_attribute_is_cut_to_the_cap() {
    let n = one_node(&format!(
        r#"Resolution="{}" Colors="{}""#,
        "R".repeat(1_000_000),
        "C".repeat(MAX_PROTOCOL_CHARS + 1)
    ));
    assert_eq!(n.resolution.chars().count(), MAX_PROTOCOL_CHARS);
    assert_eq!(n.colors.chars().count(), MAX_PROTOCOL_CHARS);
}
