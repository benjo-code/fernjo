//! The three-way merge of the vault's entries (rules V2 to V4 of the spec, with C1 to C7 applied
//! to entries): the same field rule as the tree merge, entry by entry.
//!
//! Fields are merged independently: name, username, domain, notes and the secret as one unit
//! (its kind and values together). Both sides changing a field to different values keeps the
//! head's version on the original id and adds the local version as a new entry named
//! `"<name> (conflict <device> <date>)"` with a fresh id. A delete of an unchanged entry deletes
//! it, an edit beats a delete, and additions on either side are kept.
//!
//! Like the tree merge, an entry kept against a delete yields its finding every time the same
//! inputs are merged: the merge is a pure function of its three inputs and remembers nothing, so
//! the finding disappears once the merged result has become the new base.
//!
//! Secrets never leave this module except inside the merged entries: findings carry ids and
//! names only, and the secret comparison exposes nothing beyond a bool.

use std::collections::{HashMap, HashSet};

use fernjo_vault::{EntryId, NewEntry, Secret};
use secrecy::ExposeSecret;

use super::{FieldOutcome, Finding, MergeContext, merge_field};

/// The merged entries (head order first, then local additions in local order, each conflict
/// copy right after its original) and what the user should be told about.
#[derive(Debug, Clone)]
pub struct EntriesMerge {
    pub entries: Vec<(EntryId, NewEntry)>,
    pub findings: Vec<Finding>,
}

/// A secret that can be compared by value for the field rule. Only `eq` looks at the contents
/// and it returns a bool; `Debug` is redacted.
#[derive(Clone)]
struct SecretValue(Secret);

impl PartialEq for SecretValue {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Secret::Password { password: a }, Secret::Password { password: b }) => {
                a.expose_secret() == b.expose_secret()
            }
            (
                Secret::PrivateKey {
                    key: ka,
                    passphrase: pa,
                },
                Secret::PrivateKey {
                    key: kb,
                    passphrase: pb,
                },
            ) => {
                ka.expose_secret() == kb.expose_secret()
                    && pa.as_ref().map(ExposeSecret::expose_secret)
                        == pb.as_ref().map(ExposeSecret::expose_secret)
            }
            _ => false,
        }
    }
}

impl std::fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretValue(<redacted>)")
    }
}

/// Every field of a `NewEntry`. `Fields::of` and `into_entry` destructure without `..`, so a new
/// field in the vault model stops compiling here instead of being dropped by the merge.
#[derive(Debug, Clone, PartialEq)]
struct Fields {
    name: String,
    username: Option<String>,
    domain: Option<String>,
    notes: String,
    secret: SecretValue,
}

impl Fields {
    fn of(entry: &NewEntry) -> Self {
        let NewEntry {
            name,
            username,
            domain,
            secret,
            notes,
        } = entry;
        Self {
            name: name.clone(),
            username: username.clone(),
            domain: domain.clone(),
            notes: notes.clone(),
            secret: SecretValue(secret.clone()),
        }
    }

    fn into_entry(self) -> NewEntry {
        let Self {
            name,
            username,
            domain,
            notes,
            secret,
        } = self;
        NewEntry {
            name,
            username,
            domain,
            secret: secret.0,
            notes,
        }
    }
}

/// The three-way rule for one field; without a base, equal values agree and different ones
/// conflict. A conflict keeps the head's value.
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

fn merge_fields(base: Option<&Fields>, local: &Fields, head: &Fields) -> (Fields, bool) {
    let mut conflict = false;
    macro_rules! fields {
        ($($field:ident),*) => {
            Fields {
                $($field: pick(base.map(|b| &b.$field), &local.$field, &head.$field, &mut conflict),)*
            }
        };
    }
    let merged = fields!(name, username, domain, notes, secret);
    (merged, conflict)
}

/// The first occurrence of every id, in order (a duplicate id in a list is ignored).
fn index(list: &[(EntryId, NewEntry)]) -> (Vec<EntryId>, HashMap<EntryId, Fields>) {
    let mut order = Vec::new();
    let mut map = HashMap::new();
    for (id, entry) in list {
        if !map.contains_key(id) {
            order.push(*id);
            map.insert(*id, Fields::of(entry));
        }
    }
    (order, map)
}

/// Three-way merge of the vault entries. `base` is the last synced list, `local` this device's
/// and `head` the hub's. Never fails and never panics; the secrets stay in the returned entries.
#[must_use]
pub fn merge_entries(
    base: &[(EntryId, NewEntry)],
    local: &[(EntryId, NewEntry)],
    head: &[(EntryId, NewEntry)],
    ctx: &MergeContext,
) -> EntriesMerge {
    merge_entries_with(base, local, head, ctx, &mut EntryId::new_v4)
}

pub(crate) fn merge_entries_with(
    base: &[(EntryId, NewEntry)],
    local: &[(EntryId, NewEntry)],
    head: &[(EntryId, NewEntry)],
    ctx: &MergeContext,
    new_id: &mut dyn FnMut() -> EntryId,
) -> EntriesMerge {
    let (_, base_map) = index(base);
    let (local_order, local_map) = index(local);
    let (head_order, head_map) = index(head);

    let mut taken: HashSet<EntryId> = HashSet::new();
    taken.extend(base_map.keys());
    taken.extend(local_map.keys());
    taken.extend(head_map.keys());
    let mut fresh_id = || {
        for _ in 0..16 {
            let candidate = new_id();
            if taken.insert(candidate) {
                return candidate;
            }
        }
        // the generator keeps colliding: count up to the first unused id
        let mut n = 1_u128;
        loop {
            let candidate = EntryId::from_u128(n);
            if taken.insert(candidate) {
                return candidate;
            }
            n += 1;
        }
    };

    let mut entries = Vec::new();
    let mut findings = Vec::new();
    let candidates = head_order
        .iter()
        .chain(local_order.iter().filter(|id| !head_map.contains_key(*id)));
    for id in candidates {
        let base = base_map.get(id);
        match (local_map.get(id), head_map.get(id)) {
            (Some(l), Some(h)) => {
                let (merged, conflict) = merge_fields(base, l, h);
                let name = merged.name.clone();
                entries.push((*id, merged.into_entry()));
                if conflict {
                    let mut copy = l.clone();
                    copy.name = format!("{} (conflict {} {})", l.name, ctx.device_name, ctx.date);
                    let copy_id = fresh_id();
                    entries.push((copy_id, copy.into_entry()));
                    findings.push(Finding::EntryConflictCopy {
                        original: *id,
                        copy: copy_id,
                        name,
                    });
                }
            }
            (Some(only), None) | (None, Some(only)) => {
                // present on one side only: new there, deleted on the other side, or edited
                // there against a delete (the edit wins)
                let deleted_unchanged = base == Some(only);
                if !deleted_unchanged {
                    if base.is_some() {
                        findings.push(Finding::EntryKeptEdit {
                            entry: *id,
                            name: only.name.clone(),
                        });
                    }
                    entries.push((*id, only.clone().into_entry()));
                }
            }
            (None, None) => {}
        }
    }
    EntriesMerge { entries, findings }
}

#[cfg(test)]
mod tests {
    use secrecy::SecretString;

    use super::*;

    fn pw(p: &str) -> SecretValue {
        SecretValue(Secret::Password {
            password: SecretString::from(p),
        })
    }

    fn key(k: &str, passphrase: Option<&str>) -> SecretValue {
        SecretValue(Secret::PrivateKey {
            key: SecretString::from(k),
            passphrase: passphrase.map(SecretString::from),
        })
    }

    #[test]
    fn secret_equality_looks_at_kind_key_and_passphrase() {
        assert!(pw("a") == pw("a"));
        assert!(pw("a") != pw("b"));
        assert!(key("K", Some("p")) == key("K", Some("p")));
        assert!(key("K", None) == key("K", None));
        assert!(key("K", Some("p")) != key("K", Some("q")), "passphrase");
        assert!(key("K", Some("p")) != key("K", None), "passphrase removed");
        assert!(key("K", None) != key("L", None), "key");
        assert!(pw("K") != key("K", None), "kind");
        assert!(key("K", None) != pw("K"), "kind, the other way");
    }

    #[test]
    fn debug_shows_no_secret() {
        let shown = format!(
            "{:?} {:?}",
            pw("SENTINEL-PW"),
            key("SENTINEL-KEY", Some("SENTINEL-PASS"))
        );
        assert_eq!(shown, "SecretValue(<redacted>) SecretValue(<redacted>)");
    }
}
