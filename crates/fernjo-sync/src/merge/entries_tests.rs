use fernjo_vault::{EntryId, NewEntry, Secret};
use secrecy::{ExposeSecret, SecretString};
use uuid::Uuid;

use super::entries::{EntriesMerge, merge_entries_with};
use super::report::describe;
use super::{Finding, MergeContext};

type Entries = Vec<(EntryId, NewEntry)>;

fn id(n: u128) -> EntryId {
    Uuid::from_u128(n)
}

fn ctx() -> MergeContext {
    MergeContext {
        device_name: "laptop".into(),
        date: "2026-11-22".into(),
    }
}

fn password(p: &str) -> Secret {
    Secret::Password {
        password: SecretString::from(p),
    }
}

fn entry(n: u128, name: &str, pw: &str) -> (EntryId, NewEntry) {
    (
        id(n),
        NewEntry {
            name: name.into(),
            username: Some("root".into()),
            domain: None,
            secret: password(pw),
            notes: String::new(),
        },
    )
}

fn merge(base: &Entries, local: &Entries, head: &Entries) -> EntriesMerge {
    let mut next = 1000_u128;
    merge_entries_with(base, local, head, &ctx(), &mut || {
        next += 1;
        id(next)
    })
}

/// A comparable view of an entry (tests only; this does expose the secret).
type View = (
    EntryId,
    String,
    Option<String>,
    Option<String>,
    String,
    String,
);

fn view(entries: &Entries) -> Vec<View> {
    entries
        .iter()
        .map(|(i, e)| {
            let secret = match &e.secret {
                Secret::Password { password } => format!("pw:{}", password.expose_secret()),
                Secret::PrivateKey { key, passphrase } => format!(
                    "key:{}:{:?}",
                    key.expose_secret(),
                    passphrase.as_ref().map(|p| p.expose_secret().to_string())
                ),
            };
            (
                *i,
                e.name.clone(),
                e.username.clone(),
                e.domain.clone(),
                e.notes.clone(),
                secret,
            )
        })
        .collect()
}

#[test]
fn v2_different_entries_and_fields_merge() {
    let base = vec![entry(1, "a", "p"), entry(2, "b", "q")];
    let mut local = base.clone();
    local[0].1.name = "a-local".into();
    let mut head = base.clone();
    head[1].1.username = Some("admin".into());
    head[0].1.domain = Some("corp".into());
    let out = merge(&base, &local, &head);
    assert!(out.findings.is_empty());
    let v = view(&out.entries);
    assert_eq!(v.len(), 2);
    assert_eq!(v[0].1, "a-local");
    assert_eq!(v[0].3.as_deref(), Some("corp"));
    assert_eq!(v[1].2.as_deref(), Some("admin"));
}

#[test]
fn v2_the_secret_changed_on_both_sides_copies_the_local_entry() {
    let base = vec![entry(1, "web", "old")];
    let local = vec![entry(1, "web", "mine")];
    let head = vec![entry(1, "web", "theirs")];
    let out = merge(&base, &local, &head);
    let v = view(&out.entries);
    assert_eq!(v.len(), 2);
    assert_eq!((v[0].0, v[0].5.as_str()), (id(1), "pw:theirs"));
    assert_eq!(v[1].0, id(1001));
    assert_eq!(v[1].1, "web (conflict laptop 2026-11-22)");
    assert_eq!(v[1].5, "pw:mine");
    assert_eq!(
        out.findings,
        vec![Finding::EntryConflictCopy {
            original: id(1),
            copy: id(1001),
            name: "web".into()
        }]
    );
}

#[test]
fn v2_notes_and_password_changed_on_different_sides_both_kept() {
    let base = vec![entry(1, "web", "old")];
    let mut local = base.clone();
    local[0].1.notes = "note".into();
    let head = vec![entry(1, "web", "new")];
    let out = merge(&base, &local, &head);
    assert!(out.findings.is_empty());
    let v = view(&out.entries);
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].4, "note");
    assert_eq!(v[0].5, "pw:new");
}

#[test]
fn v2_deleted_versus_edited_keeps_the_entry() {
    let base = vec![entry(1, "web", "p")];
    let mut edited = base.clone();
    edited[0].1.notes = "changed".into();
    // head deleted, local edited
    let out = merge(&base, &edited, &Vec::new());
    assert_eq!(view(&out.entries), view(&edited));
    assert_eq!(
        out.findings,
        vec![Finding::EntryKeptEdit {
            entry: id(1),
            name: "web".into()
        }]
    );
    // local deleted, head edited
    let out = merge(&base, &Vec::new(), &edited);
    assert_eq!(view(&out.entries), view(&edited));
    assert_eq!(out.findings.len(), 1);
}

#[test]
fn v2_deleted_and_unchanged_is_deleted() {
    let base = vec![entry(1, "a", "p"), entry(2, "b", "q")];
    let local = vec![entry(2, "b", "q")];
    let head = vec![entry(1, "a", "p")];
    let out = merge(&base, &local, &head);
    assert!(out.entries.is_empty());
    assert!(out.findings.is_empty());
}

#[test]
fn v2_added_on_either_side_is_kept() {
    let base = vec![entry(1, "a", "p")];
    let local = vec![entry(1, "a", "p"), entry(3, "mine", "x")];
    let head = vec![entry(1, "a", "p"), entry(2, "theirs", "y")];
    let out = merge(&base, &local, &head);
    let ids: Vec<_> = view(&out.entries).iter().map(|v| v.0).collect();
    assert_eq!(ids, vec![id(1), id(2), id(3)]);
    assert!(out.findings.is_empty());
}

#[test]
fn merging_twice_is_idempotent() {
    let base = vec![entry(1, "web", "old"), entry(2, "gone", "g")];
    let mut local = vec![entry(1, "web", "mine"), entry(3, "new", "n")];
    local[0].1.notes = "n".into();
    let head = vec![entry(1, "web", "theirs"), entry(2, "gone", "g")];
    let first = merge(&base, &local, &head);
    assert!(!first.findings.is_empty());
    // the merged result becomes head, base and local on the next run: nothing changes
    let again = merge(&first.entries, &first.entries, &first.entries);
    assert_eq!(view(&again.entries), view(&first.entries));
    assert!(again.findings.is_empty());
    // the same inputs always give the same result
    let repeat = merge(&base, &local, &head);
    assert_eq!(view(&repeat.entries), view(&first.entries));
    assert_eq!(repeat.findings, first.findings);
}

#[test]
fn findings_never_contain_a_secret() {
    let sentinels = [
        "SENTINEL-PW-L",
        "SENTINEL-PW-H",
        "SENTINEL-USER",
        "SENTINEL-NOTE",
    ];
    let mut base = vec![entry(1, "web", "base"), entry(2, "kept", "base")];
    base[0].1.username = Some("SENTINEL-USER".into());
    let mut local = vec![entry(1, "web", "SENTINEL-PW-L"), entry(2, "kept", "base")];
    local[0].1.username = Some("SENTINEL-USER".into());
    local[0].1.notes = "SENTINEL-NOTE".into();
    local[1].1.notes = "SENTINEL-NOTE".into();
    let mut head = vec![entry(1, "web", "SENTINEL-PW-H")];
    head[0].1.username = Some("SENTINEL-USER".into());
    let out = merge(&base, &local, &head);
    assert_eq!(out.findings.len(), 2);
    let mut text = format!("{:?}", out.findings);
    for f in &out.findings {
        text.push_str(&describe(f));
    }
    for s in sentinels {
        assert!(!text.contains(s), "{s} leaked");
    }
}
