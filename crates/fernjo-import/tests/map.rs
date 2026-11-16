use std::collections::HashSet;

use fernjo_core::{Node, NodeKind, Protocol, Source, Tree};
use fernjo_import::mremoteng::{
    ImportPlan, Inherit, PlannedEntry, RawDocument, RawKind, RawNode, plan, read,
};
use fernjo_import::report::ReportLine;
use fernjo_vault::Secret;
use secrecy::ExposeSecret;
use zeroize::Zeroizing;

const TODAY: &str = "2026-11-16";
const DEFAULT_KEY_XML: &[u8] = include_bytes!("fixtures/default-key.xml");

fn raw(kind: RawKind, name: &str) -> RawNode {
    RawNode {
        kind,
        name: name.to_owned(),
        description: String::new(),
        hostname: String::new(),
        protocol: String::new(),
        port: None,
        port_invalid: None,
        username: String::new(),
        domain: String::new(),
        password: None,
        password_error: false,
        inherit: Inherit::default(),
        putty_session: String::new(),
        children: Vec::new(),
    }
}

fn conn(name: &str, protocol: &str, host: &str) -> RawNode {
    let mut n = raw(RawKind::Connection, name);
    n.protocol = protocol.to_owned();
    n.hostname = host.to_owned();
    n
}

fn ssh(name: &str) -> RawNode {
    conn(name, "SSH2", "host.example")
}

fn folder(name: &str, children: Vec<RawNode>) -> RawNode {
    let mut n = raw(RawKind::Container, name);
    n.children = children;
    n
}

fn inheriting(mut n: RawNode) -> RawNode {
    n.inherit = Inherit {
        username: true,
        password: true,
        domain: true,
        port: true,
    };
    n
}

fn pw(mut n: RawNode, user: &str, domain: &str, password: &str) -> RawNode {
    n.username = user.to_owned();
    n.domain = domain.to_owned();
    n.password = Some(Zeroizing::new(password.to_owned()));
    n
}

fn run(nodes: Vec<RawNode>) -> ImportPlan {
    plan(&RawDocument { nodes }, TODAY, &[])
}

fn children(n: &Node) -> &[Node] {
    match &n.kind {
        NodeKind::Folder { children } => children,
        NodeKind::Connection { .. } => panic!("not a folder"),
    }
}

fn all_ids(n: &Node, out: &mut Vec<uuid::Uuid>) {
    out.push(n.id);
    if let NodeKind::Folder { children } = &n.kind {
        for c in children {
            all_ids(c, out);
        }
    }
}

fn password_of(p: &PlannedEntry) -> String {
    match &p.entry.secret {
        Secret::Password { password } => password.expose_secret().to_owned(),
        Secret::PrivateKey { .. } => panic!("not a password"),
    }
}

// ---- structure ------------------------------------------------------------

#[test]
fn containers_become_folders_and_ssh2_rdp_become_connections() {
    let p = run(vec![folder(
        "Servers",
        vec![ssh("web"), conn("desk", "RDP", "10.0.0.5")],
    )]);
    assert_eq!(p.folder.name, "Imported from mRemoteNG (2026-11-16)");
    let servers = &children(&p.folder)[0];
    assert_eq!(servers.name, "Servers");
    let kids = children(servers);
    assert!(matches!(
        kids[0].kind,
        NodeKind::Connection { protocol: Protocol::Ssh, ref host } if host == "host.example"
    ));
    assert!(matches!(
        kids[1].kind,
        NodeKind::Connection { protocol: Protocol::Rdp, ref host } if host == "10.0.0.5"
    ));
    assert_eq!(p.report.counts.folders, 1);
    assert_eq!(p.report.counts.ssh, 1);
    assert_eq!(p.report.counts.rdp, 1);
}

#[test]
fn document_order_is_kept() {
    let p = run(vec![ssh("c"), folder("a", vec![]), ssh("b")]);
    let names: Vec<_> = children(&p.folder)
        .iter()
        .map(|n| n.name.as_str())
        .collect();
    assert_eq!(names, ["c", "a", "b"]);
}

#[test]
fn folder_is_kept_when_every_child_is_skipped() {
    let p = run(vec![folder("F", vec![conn("v", "VNC", "h")])]);
    let f = &children(&p.folder)[0];
    assert!(children(f).is_empty());
    assert_eq!(p.report.counts.skipped, 1);
    assert_eq!(p.report.counts.folders, 1);
}

#[test]
fn second_import_same_day_gets_a_numbered_folder() {
    let d = RawDocument { nodes: vec![] };
    let first = "Imported from mRemoteNG (2026-11-16)".to_owned();
    let p2 = plan(&d, TODAY, std::slice::from_ref(&first));
    assert_eq!(p2.folder.name, format!("{first} (2)"));
    let second = p2.folder.name.clone();
    let p3 = plan(&d, TODAY, &[first, second]);
    assert_eq!(p3.folder.name, "Imported from mRemoteNG (2026-11-16) (3)");
}

#[test]
fn fresh_ids_are_unique() {
    let p = run(vec![
        folder(
            "F",
            vec![pw(ssh("a"), "u", "", "x"), pw(ssh("b"), "v", "", "y")],
        ),
        ssh("c"),
    ]);
    let mut ids = Vec::new();
    all_ids(&p.folder, &mut ids);
    for e in &p.entries {
        ids.push(e.id);
    }
    let set: HashSet<_> = ids.iter().collect();
    assert_eq!(set.len(), ids.len());
    let q = run(vec![ssh("c")]);
    let mut other = Vec::new();
    all_ids(&q.folder, &mut other);
    assert!(other.iter().all(|i| !set.contains(i)));
}

// ---- skipped ---------------------------------------------------------------

#[test]
fn vnc_telnet_ssh1_http_ica_are_skipped_and_listed_with_path_and_protocol() {
    let p = run(vec![folder(
        "Lab",
        vec![
            conn("a", "VNC", "h"),
            conn("b", "Telnet", "h"),
            conn("c", "SSH1", "h"),
            conn("d", "HTTPS", "h"),
            conn("e", "ICA", "h"),
            ssh("ok"),
        ],
    )]);
    assert_eq!(p.report.counts.skipped, 5);
    assert_eq!(p.report.skipped.len(), 5);
    assert_eq!(p.report.skipped[0].path, "Lab / a");
    assert!(p.report.skipped[0].text.contains("VNC"));
    assert!(p.report.skipped[2].text.contains("SSH1"));
    assert!(p.report.skipped[3].text.contains("HTTPS"));
    assert_eq!(children(&children(&p.folder)[0]).len(), 1);
}

#[test]
fn unknown_type_is_skipped_with_its_subtree() {
    let mut weird = raw(RawKind::Unknown("Gadget".to_owned()), "W");
    weird.children = vec![ssh("inner")];
    let p = run(vec![weird, ssh("after")]);
    assert_eq!(p.report.counts.skipped, 1);
    assert_eq!(p.report.counts.ssh, 1);
    assert_eq!(p.report.skipped[0].path, "W");
    assert!(p.report.skipped[0].text.contains("Gadget"));
    assert_eq!(children(&p.folder).len(), 1);
}

// ---- settings --------------------------------------------------------------

#[test]
fn inherit_flag_true_leaves_port_and_username_unset() {
    let mut n = ssh("a");
    n.port = Some(2222);
    n.username = "root".to_owned();
    let p = run(vec![inheriting(n)]);
    let s = &children(&p.folder)[0].settings;
    assert_eq!(s.port, None);
    assert_eq!(s.username, None);
}

#[test]
fn inherit_flag_false_sets_local_values() {
    let mut n = ssh("a");
    n.port = Some(2222);
    n.username = "root".to_owned();
    let p = run(vec![n]);
    let s = &children(&p.folder)[0].settings;
    assert_eq!(s.port, Some(2222));
    assert_eq!(s.username.as_deref(), Some("root"));
}

#[test]
fn port_zero_or_above_65535_is_unset_and_reported_as_a_problem() {
    let mut a = ssh("a");
    a.port = Some(0);
    let mut b = ssh("b");
    b.port = Some(65536);
    let mut c = ssh("c");
    c.port_invalid = Some("abc".to_owned());
    let mut d = ssh("d");
    d.port = Some(65535);
    let p = run(vec![a, b, c, d]);
    let kids = children(&p.folder);
    assert_eq!(kids[0].settings.port, None);
    assert_eq!(kids[1].settings.port, None);
    assert_eq!(kids[2].settings.port, None);
    assert_eq!(kids[3].settings.port, Some(65535));
    assert_eq!(p.report.counts.problems, 3);
    assert_eq!(p.report.problems[0].path, "a");
    assert!(p.report.problems[1].text.contains("65536"));
    assert!(p.report.problems[2].text.contains("abc"));
}

#[test]
fn empty_name_becomes_unnamed_and_is_reported() {
    let p = run(vec![ssh("  ")]);
    assert_eq!(children(&p.folder)[0].name, "Unnamed");
    assert_eq!(p.report.counts.problems, 1);
    assert_eq!(p.report.problems[0].path, "Unnamed");
}

#[test]
fn connection_without_host_is_kept_and_reported() {
    let p = run(vec![conn("nohost", "SSH2", "")]);
    assert_eq!(children(&p.folder).len(), 1);
    assert_eq!(p.report.counts.problems, 1);
    assert!(p.report.problems[0].text.contains("host"));
}

// ---- credentials -----------------------------------------------------------

#[test]
fn own_password_creates_an_entry_referenced_by_the_node() {
    let p = run(vec![pw(ssh("a"), "root", "", "s3cret")]);
    assert_eq!(p.entries.len(), 1);
    let e = &p.entries[0];
    assert_eq!(e.entry.username.as_deref(), Some("root"));
    assert_eq!(password_of(e), "s3cret");
    assert_eq!(children(&p.folder)[0].settings.credential, Some(e.id));
    assert_eq!(p.report.counts.credentials, 1);
}

#[test]
fn identical_username_domain_password_share_one_entry() {
    let p = run(vec![
        pw(ssh("a"), "root", "D", "same"),
        pw(ssh("b"), "root", "D", "same"),
        pw(ssh("c"), "root", "D", "other"),
        pw(ssh("d"), "root", "E", "same"),
    ]);
    assert_eq!(p.entries.len(), 3);
    let kids = children(&p.folder);
    assert_eq!(kids[0].settings.credential, kids[1].settings.credential);
    assert_ne!(kids[0].settings.credential, kids[2].settings.credential);
    assert_ne!(kids[0].settings.credential, kids[3].settings.credential);
    assert_eq!(p.report.counts.credentials, 3);
    assert_eq!(p.report.counts.shared_credentials, 1);
}

#[test]
fn entry_takes_username_and_domain_only_when_their_flags_are_false() {
    let mut a = pw(ssh("a"), "root", "CORP", "x");
    a.inherit.domain = true;
    let mut b = pw(ssh("b"), "root", "CORP", "y");
    b.inherit.username = true;
    let c = pw(ssh("c"), "", "", "z");
    let p = run(vec![a, b, c]);
    assert_eq!(p.entries[0].entry.username.as_deref(), Some("root"));
    assert_eq!(p.entries[0].entry.domain, None);
    assert_eq!(p.entries[1].entry.username, None);
    assert_eq!(p.entries[1].entry.domain.as_deref(), Some("CORP"));
    assert_eq!(p.entries[2].entry.username, None);
    assert_eq!(p.entries[2].entry.domain, None);
}

#[test]
fn inherited_password_creates_nothing() {
    let mut n = pw(ssh("a"), "root", "", "ignored");
    n.inherit.password = true;
    let p = run(vec![n]);
    assert!(p.entries.is_empty());
    assert_eq!(children(&p.folder)[0].settings.credential, None);
}

#[test]
fn password_own_username_inherited_gives_entry_without_username_and_no_node_username() {
    let mut n = pw(ssh("a"), "root", "", "x");
    n.inherit.username = true;
    let p = run(vec![n]);
    assert_eq!(p.entries[0].entry.username, None);
    assert_eq!(children(&p.folder)[0].settings.username, None);
    assert!(children(&p.folder)[0].settings.credential.is_some());
}

#[test]
fn entry_names_are_username_or_user_at_domain_with_numbering() {
    let mut nameless = pw(ssh("named after me"), "", "", "4");
    nameless.inherit.username = true;
    let p = run(vec![
        pw(ssh("a"), "root", "", "1"),
        pw(ssh("b"), "root", "", "2"),
        pw(ssh("c"), "admin", "CORP", "3"),
        nameless,
    ]);
    let names: Vec<_> = p.entries.iter().map(|e| e.entry.name.as_str()).collect();
    assert_eq!(names, ["root", "root (2)", "admin@CORP", "named after me"]);
}

#[test]
fn undecryptable_password_is_reported_and_the_node_imports_without_one() {
    let mut n = ssh("a");
    n.password_error = true;
    let p = run(vec![n]);
    assert!(p.entries.is_empty());
    assert_eq!(children(&p.folder)[0].settings.credential, None);
    assert_eq!(p.report.counts.problems, 1);
    assert!(p.report.problems[0].text.contains("password"));
}

// ---- report ----------------------------------------------------------------

#[test]
fn domain_without_password_is_partly_mapped() {
    let mut n = ssh("a");
    n.domain = "CORP".to_owned();
    let p = run(vec![n]);
    assert_eq!(p.report.counts.partial, 1);
    assert!(p.report.partial[0].text.contains("domain"));
    // A domain that travels with a password is fully mapped.
    let q = run(vec![pw(ssh("b"), "u", "CORP", "x")]);
    assert_eq!(q.report.counts.partial, 0);
}

#[test]
fn putty_session_other_than_default_is_partly_mapped() {
    let mut a = ssh("a");
    a.putty_session = "My Session".to_owned();
    let mut b = ssh("b");
    b.putty_session = "Default Settings".to_owned();
    let p = run(vec![a, b, ssh("c")]);
    assert_eq!(p.report.counts.partial, 1);
    assert_eq!(p.report.partial[0].path, "a");
    assert!(p.report.partial[0].text.contains("My Session"));
}

#[test]
fn rdp_display_settings_are_one_aggregate_line() {
    let p = run(vec![
        conn("r1", "RDP", "h1"),
        conn("r2", "RDP", "h2"),
        conn("r3", "RDP", "h3"),
        ssh("s"),
    ]);
    assert_eq!(p.report.counts.partial, 1);
    assert!(p.report.partial[0].text.contains("3 RDP connections"));
    let none = run(vec![ssh("s")]);
    assert_eq!(none.report.counts.partial, 0);
}

fn find<'a>(lines: &'a [ReportLine], path: &str, needle: &str) -> Option<&'a str> {
    lines
        .iter()
        .find(|l| l.path == path && l.text.contains(needle))
        .map(|l| l.text.as_str())
}

const NO_PASSWORD: &str = "has no password of its own";
const NO_USERNAME: &str = "has no username of its own";

#[test]
fn empty_own_password_names_the_nearest_folder_with_a_credential() {
    // Outer sets credential and username; Inner sets nothing; the node sits below both.
    let inner = inheriting(folder("Inner", vec![ssh("db")]));
    let outer = pw(folder("Outer", vec![inner]), "admin", "", "folder-pw");
    let p = run(vec![outer]);
    let lines = &p.report.partial;
    let pw_line = find(lines, "Outer / Inner / db", NO_PASSWORD).expect("password line");
    assert!(pw_line.contains("folder \"Outer\""), "{pw_line}");
    let user_line = find(lines, "Outer / Inner / db", NO_USERNAME).expect("username line");
    assert!(user_line.contains("folder \"Outer\""), "{user_line}");
    // Inner inherits everything, so it has nothing to report itself.
    assert!(find(lines, "Outer / Inner", "has no").is_none());
}

#[test]
fn an_inner_folder_that_sets_its_own_credential_is_the_one_named() {
    let inner = pw(folder("Inner", vec![ssh("db")]), "svc", "", "inner-pw");
    let outer = pw(folder("Outer", vec![inner]), "admin", "", "outer-pw");
    let p = run(vec![outer]);
    let lines = &p.report.partial;
    let pw_line = find(lines, "Outer / Inner / db", NO_PASSWORD).expect("password line");
    assert!(pw_line.contains("folder \"Inner\""), "{pw_line}");
    let user_line = find(lines, "Outer / Inner / db", NO_USERNAME).expect("username line");
    assert!(user_line.contains("folder \"Inner\""), "{user_line}");
    // Inner itself has no password-less own value: it has both.
    assert!(find(lines, "Outer / Inner", "has no").is_none());
}

fn check_against_core(
    raw: &RawNode,
    node: &Node,
    path: &str,
    tree: &Tree,
    lines: &[ReportLine],
    seen: &mut usize,
) {
    let r = tree.resolve_node(node.id).unwrap();
    let own_empty_pw = !raw.inherit.password && raw.password.is_none() && !raw.password_error;
    let expected_pw = match (&r.credential.source, own_empty_pw) {
        (Source::Folder { name, .. }, true) => Some(name.clone()),
        _ => None,
    };
    let own_empty_user = !raw.inherit.username && raw.username.is_empty();
    let expected_user = match (&r.username.source, own_empty_user) {
        (Source::Folder { name, .. }, true) => Some(name.clone()),
        _ => None,
    };
    for (needle, expected) in [(NO_PASSWORD, expected_pw), (NO_USERNAME, expected_user)] {
        let got = find(lines, path, needle);
        match expected {
            Some(folder) => {
                *seen += 1;
                let text = got.unwrap_or_else(|| panic!("missing line for {path}: {needle}"));
                assert!(
                    text.contains(&format!("folder \"{folder}\"")),
                    "{path}: {text}"
                );
            }
            None => assert!(got.is_none(), "unexpected line for {path}: {got:?}"),
        }
    }
    for (rc, nc) in raw.children.iter().zip(children_of(node)) {
        check_against_core(rc, nc, &format!("{path} / {}", nc.name), tree, lines, seen);
    }
}

fn children_of(n: &Node) -> &[Node] {
    match &n.kind {
        NodeKind::Folder { children } => children,
        NodeKind::Connection { .. } => &[],
    }
}

#[test]
fn inherit_lines_agree_with_core_resolution() {
    let mut n5 = pw(ssh("n5"), "", "", "own");
    n5.inherit.username = false;
    let b_folder = {
        let mut b = folder("B", vec![ssh("n4"), n5]);
        b.username = "bee".to_owned();
        b
    };
    let a = inheriting(folder(
        "A",
        vec![
            ssh("n1"),
            inheriting(ssh("n2")),
            pw(ssh("n3"), "u", "", "p"),
        ],
    ));
    let d = inheriting(folder("D", vec![ssh("n7")]));
    let c = pw(folder("C", vec![ssh("n6"), d]), "cee", "", "cpw");
    let outer = pw(folder("Outer", vec![a, b_folder, c]), "admin", "", "opw");
    let nodes = vec![outer, ssh("n8"), folder("Plain", vec![ssh("n9")])];
    let p = run(nodes.clone());
    let tree = Tree::from_children(vec![p.folder.clone()]);
    let mut seen = 0;
    for (rc, nc) in nodes.iter().zip(children_of(&p.folder)) {
        check_against_core(rc, nc, &nc.name, &tree, &p.report.partial, &mut seen);
    }
    // The sample really exercises the lines (not a vacuous pass).
    assert!(seen >= 8, "only {seen} inherit lines were compared");
}

#[test]
fn undecryptable_password_gets_no_inherit_line() {
    let mut c = ssh("db");
    c.password_error = true;
    let prod = pw(folder("Prod", vec![c]), "admin", "", "folder-pw");
    let p = run(vec![prod]);
    assert!(find(&p.report.partial, "Prod / db", NO_PASSWORD).is_none());
    assert_eq!(p.report.counts.problems, 1);
}

#[test]
fn entry_name_comes_from_the_node_name_even_with_a_slash_in_it() {
    let mut n = pw(ssh("a / b"), "", "", "x");
    n.inherit.username = true;
    let p = run(vec![folder("F", vec![n])]);
    assert_eq!(p.entries[0].entry.name, "a / b");
}

#[test]
fn report_lines_are_cut_to_200_characters() {
    let long = "é".repeat(10_000);
    let mut host_less = conn(&long, "SSH2", "");
    host_less.inherit.password = false;
    let outer = pw(
        folder(&long, vec![host_less, ssh("kid")]),
        "admin",
        "",
        "pw",
    );
    let p = run(vec![outer]);
    let all: Vec<&ReportLine> = p
        .report
        .problems
        .iter()
        .chain(&p.report.partial)
        .chain(&p.report.skipped)
        .collect();
    assert!(!all.is_empty());
    for l in all {
        assert!(l.path.chars().count() <= 200 && l.text.chars().count() <= 200);
    }
    let first = &p.report.problems[0];
    assert!(first.path.ends_with('…'));
    // The partial list names the long folder in its text.
    let named = p
        .report
        .partial
        .iter()
        .find(|l| l.text.contains("folder \""))
        .unwrap();
    assert_eq!(named.text.chars().count(), 200);
}

#[test]
fn many_entries_with_one_username_are_numbered_in_linear_time() {
    const N: usize = 20_000;
    let nodes: Vec<RawNode> = (0..N)
        .map(|i| pw(ssh(&format!("n{i}")), "root", "", &format!("pw-{i}")))
        .collect();
    let started = std::time::Instant::now();
    let p = run(nodes);
    let elapsed = started.elapsed();
    assert_eq!(p.entries.len(), N);
    assert_eq!(p.entries[0].entry.name, "root");
    assert_eq!(p.entries[N - 1].entry.name, format!("root ({N})"));
    // Quadratic numbering needs roughly N^2/2 = 2e8 name lookups here (tens of seconds in a debug
    // build); the linear version takes well under a second. The generous bound only catches the
    // quadratic shape, not machine speed.
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "took {elapsed:?}"
    );
}

#[test]
fn lists_are_capped_at_200_with_exact_counts() {
    let nodes: Vec<RawNode> = (0..250)
        .map(|i| conn(&format!("v{i}"), "VNC", "h"))
        .collect();
    let p = run(nodes);
    assert_eq!(p.report.counts.skipped, 250);
    assert_eq!(p.report.skipped.len(), 201);
    assert_eq!(p.report.skipped[200].text, "and 50 more");
    assert_eq!(p.report.skipped[200].path, "");
}

#[test]
fn report_serialises_without_any_password() {
    let sentinel = "SENTINEL-PASSWORD-123";
    let mut bad = pw(ssh("a"), "root", "CORP", sentinel);
    bad.putty_session = "x".to_owned();
    bad.port = Some(0);
    let mut u = ssh("b");
    u.password_error = true;
    let p = run(vec![bad, u, conn("v", "VNC", "h")]);
    assert_eq!(password_of(&p.entries[0]), sentinel);
    let json = serde_json::to_string(&p.report).unwrap();
    assert!(!json.contains(sentinel));
    assert!(json.contains("\"sharedCredentials\""));
    assert!(!format!("{:?}", p.report).contains(sentinel));
}

// ---- fixture ---------------------------------------------------------------

#[test]
fn default_key_fixture_maps_end_to_end() {
    let d = read(DEFAULT_KEY_XML, None).unwrap();
    let p = plan(&d, TODAY, &[]);
    let tree = Tree::from_children(vec![p.folder.clone()]);
    let servers = &children(&p.folder)[0];
    assert_eq!(servers.name, "Servers");
    // Every credential reference points at a planned entry.
    let ids: HashSet<_> = p.entries.iter().map(|e| e.id).collect();
    fn refs(n: &Node, out: &mut Vec<uuid::Uuid>) {
        out.extend(n.settings.credential);
        if let NodeKind::Folder { children } = &n.kind {
            children.iter().for_each(|c| refs(c, out));
        }
    }
    let mut used = Vec::new();
    refs(&p.folder, &mut used);
    assert!(!used.is_empty());
    assert!(used.iter().all(|u| ids.contains(u)));
    assert!(tree.get(servers.id).is_some());
    assert_eq!(p.report.counts.credentials, p.entries.len());
}

// ---- password problem line, import without passwords -------------------------------------------

#[test]
fn undecryptable_password_under_a_folder_credential_does_not_claim_to_be_imported_without_one() {
    let mut c = ssh("db");
    c.password_error = true;
    let prod = pw(folder("Prod", vec![c]), "admin", "", "folder-pw");
    let p = run(vec![prod]);
    let line = &p.report.problems[0];
    assert_eq!(line.path, "Prod / db");
    assert!(
        line.text.contains("folder \"Prod\""),
        "the node inherits the folder's credential: {}",
        line.text
    );
    assert!(!line.text.contains("imported without one"), "{}", line.text);
    // and without any folder credential the plain wording stays
    let mut alone = ssh("db");
    alone.password_error = true;
    let p = run(vec![alone]);
    assert!(p.report.problems[0].text.contains("imported without one"));
}

#[test]
fn the_report_without_passwords_matches_what_the_import_then_does() {
    // Prod sets a credential (reference 1); web has its own password (reference 2); empty
    // inherits it; broken has an undecryptable password under Prod.
    let empty = ssh("empty");
    let mut broken = ssh("broken");
    broken.password_error = true;
    let web = pw(ssh("web"), "root", "", "web-pw");
    let prod = pw(
        folder("Prod", vec![empty, broken, web]),
        "admin",
        "",
        "folder-pw",
    );
    let p = run(vec![prod]);

    // with passwords: the inherit line and the "now uses" problem line
    assert!(find(&p.report.partial, "Prod / empty", "now uses the credential").is_some());
    assert_eq!(p.report.counts.credentials, 2);

    let bare = &p.report_without_passwords;
    assert_eq!(bare.counts.credentials, 0);
    assert_eq!(bare.counts.shared_credentials, 0);
    assert!(
        bare.partial
            .iter()
            .all(|l| !l.text.contains("now uses the credential")),
        "{:?}",
        bare.partial
    );
    let left_out = bare
        .partial
        .iter()
        .find(|l| l.text.contains("left out"))
        .expect("the references line");
    assert!(
        left_out.text.starts_with("2 stored password references"),
        "{}",
        left_out.text
    );
    assert_eq!(bare.counts.partial, bare.partial.len());
    // the undecryptable password now really is imported without one
    assert_eq!(bare.problems.len(), 1);
    assert!(bare.problems[0].text.contains("imported without one"));
    // lines that stay true are the same
    assert_eq!(bare.skipped, p.report.skipped);
}

#[test]
fn the_references_line_counts_before_the_cap_and_is_never_cut() {
    let mut nodes: Vec<RawNode> = (0..250)
        .map(|i| {
            let mut n = ssh(&format!("c{i}"));
            n.putty_session = "custom".to_owned();
            n
        })
        .collect();
    nodes.push(pw(ssh("with-password"), "root", "", "pw"));
    let p = run(nodes);
    let bare = &p.report_without_passwords;
    // 250 PuTTY lines + the references line, capped to 200 and "and N more"
    assert_eq!(bare.counts.partial, 251);
    assert_eq!(bare.partial.len(), 201);
    assert!(
        bare.partial[0]
            .text
            .contains("1 stored password reference ")
    );
    assert_eq!(bare.partial[200].text, "and 51 more");
}
