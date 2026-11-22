//! Three-way merge of the synced data (spec decision 3). `merge_field` is the one field rule
//! the tree merge and the vault merge share; `Finding` is what a merge tells the user.

mod entries;
#[cfg(test)]
mod entries_tests;
pub mod report;
mod tree;
#[cfg(test)]
mod tree_tests;

use fernjo_core::NodeId;
use fernjo_vault::{EntryId, VaultId};

pub use entries::{EntriesMerge, merge_entries};
pub use tree::{TreeMerge, merge_tree};

/// Labels for the conflict copies a merge creates. Both come from the caller (the merge never
/// reads the clock or the host name), and neither decides anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeContext {
    pub device_name: String,
    pub date: String,
}

/// Something the user should hear about after a merge. Findings carry ids and names only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finding {
    /// A field changed to different values on both sides (C4): the head's value stayed on
    /// `original`, the local version is the new item `copy`.
    ConflictCopy {
        original: NodeId,
        copy: NodeId,
        name: String,
    },
    /// An edit (or something added or edited below) beat a delete on the other side (C6).
    KeptEdit { node: NodeId, name: String },
    /// Both sides placed the node differently (C8); the head's placement was kept.
    PlacementKept { node: NodeId, name: String },
    /// A local move would have made a cycle or exceeded the depth limit (C9) and was undone.
    MoveUndone { node: NodeId, name: String },
    /// A vault entry changed to different values on both sides (V2): the head's version stayed
    /// on `original`, the local version is the new entry `copy`.
    EntryConflictCopy {
        original: EntryId,
        copy: EntryId,
        name: String,
    },
    /// An edited vault entry beat a delete on the other side (V2).
    EntryKeptEdit { entry: EntryId, name: String },
    /// Both sides changed the vault, so the other device's header (and with it its master
    /// password) was kept (V3); the password this device had set is gone. The vault id is the
    /// only thing carried: no password, wrap or key.
    PasswordChangeDropped { vault: VaultId },
}

/// What the three-way rule says about one field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldOutcome<T> {
    /// Both sides agree, or neither changed it: the head's value is the result.
    Same,
    /// Exactly one side changed it: this is the changed value.
    Take(T),
    /// Both changed it to different values.
    Conflict { local: T, head: T },
}

/// The field rule of C1 to C4: compare `local` and `head` with the `base` they both came from.
#[must_use]
pub fn merge_field<T: PartialEq + Clone>(base: &T, local: &T, head: &T) -> FieldOutcome<T> {
    if local == head || (local == base && head == base) {
        FieldOutcome::Same
    } else if local == base {
        FieldOutcome::Take(head.clone())
    } else if head == base {
        FieldOutcome::Take(local.clone())
    } else {
        FieldOutcome::Conflict {
            local: local.clone(),
            head: head.clone(),
        }
    }
}
