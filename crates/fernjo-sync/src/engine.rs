//! The sync engine: set up, join, run, re-seed (spec decisions 3 and 4).
//!
//! One run reads the local data, fetches the head, merges, applies the merge locally, pushes the
//! result with a compare-and-swap and then stores the new base. The engine holds no lock and no
//! `AppState`: everything local goes through a [`LocalPort`], the hub through a [`HubClient`].
//!
//! Safety properties this module is built around:
//! - A merge is a three-way merge against the stored base, and it is idempotent: a run that was
//!   interrupted after "apply locally" and before "push" is repaired by the next run, which
//!   merges again and finds the head's changes already present (no second conflict copy).
//! - Order of the device-local writes after a successful push: `sync-base.json` first, then
//!   `sync.json` (which carries `last_seq`). A crash between the two leaves an older base but a
//!   `last_seq` that is not ahead of a result this device did not store.
//! - Everything adopted from the hub is authenticated, belongs to the group, has the sequence
//!   number of its envelope, is at most [`MAX_NODES`] nodes and `MAX_TREE_DEPTH` deep, and its
//!   vault part parses (KDF bounds) before anything is written locally.
//! - Findings come only from the final attempt of a run. An attempt that is abandoned (the local
//!   data changed under it, or the push was refused with `hub_conflict`) is started over from
//!   the beginning and its findings are dropped, so a repeated push never shows a notice twice.
//! - A slow link that hits the client's total timeout is `hub_unreachable`: the quiet `Offline`
//!   outcome, never corruption, and nothing is partially applied because the merge is applied
//!   only after the head was read completely.

use std::collections::HashMap;

use fernjo_core::persist::document;
use fernjo_core::{Node, NodeId, NodeKind, Tree};
use fernjo_vault::{EntryId, NewEntry, inspect_file, read_entries};
use secrecy::SecretString;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::client::{Expected, HubClient, HubConfig};
use crate::envelope::{GroupId, MAX_ENVELOPE_LEN, SyncKey, open, peek_header, seal};
use crate::error::SyncError;
use crate::merge::{Finding, MergeContext, merge_entries, merge_tree};
use crate::pairing::PairingCode;
use crate::ports::{LocalChange, LocalPort, LocalState, VaultChange, VaultSide};
use crate::secrets::SyncSecrets;
use crate::snapshot::{DeviceInfo, SNAPSHOT_FORMAT_VERSION, Snapshot};
use crate::state::{StateStore, SyncState};

/// The most nodes a snapshot adopted from the hub may hold.
pub const MAX_NODES: usize = 100_000;
/// How often one run starts over (local data changed, or a push was refused) before it gives up.
const MAX_ATTEMPTS: usize = 5;

/// The engine and what it works with. `ctx` labels conflict copies and stamps `writtenAt`; the
/// engine never reads the clock or the host name itself.
pub struct Engine<'a> {
    pub hub: &'a HubClient,
    pub state: &'a StateStore,
    pub secrets: &'a dyn SyncSecrets,
    pub local: &'a dyn LocalPort,
    pub ctx: MergeContext,
}

/// How a run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The local data was changed or pushed; `findings` are what to tell the user and `seq` is
    /// the sequence number this device is at now.
    Synced { findings: Vec<Finding>, seq: u64 },
    /// Local data and head already agree.
    NothingToDo,
    /// The hub could not be reached; `pending_changes` is the count waiting (see
    /// [`Engine::pending_changes`]).
    Offline { pending_changes: usize },
    /// Both sides changed the vault and it is locked: nothing was pulled or pushed.
    WaitingForUnlock,
    /// The hub shows an older state than this device has seen (`found` is 0 for an empty hub);
    /// nothing was merged or written.
    HubRolledBack { found: u64, seen: u64 },
}

/// What happens to the vault file in a run.
enum VaultPlan {
    Keep,
    Adopt(Vec<u8>),
    Merge,
}

fn invalid(reason: &str) -> SyncError {
    SyncError::SnapshotInvalid(reason.to_owned())
}

fn count_nodes(nodes: &[Node]) -> usize {
    nodes
        .iter()
        .map(|n| {
            1 + match &n.kind {
                NodeKind::Folder { children } => count_nodes(children),
                NodeKind::Connection { .. } => 0,
            }
        })
        .sum()
}

/// Whether two vault files are the same version: the 150 header bytes carry the body nonce,
/// which is fresh on every save. A file that cannot be inspected is compared by its bytes.
fn same_version(a: &[u8], b: &[u8]) -> bool {
    match (inspect_file(a), inspect_file(b)) {
        (Ok(x), Ok(y)) => x.version_header == y.version_header,
        _ => a == b,
    }
}

/// Whether `side` differs from `base` (a missing file against a present one is a change).
fn vault_changed(side: Option<&[u8]>, base: Option<&[u8]>) -> bool {
    match (side, base) {
        (None, None) => false,
        (Some(a), Some(b)) => !same_version(a, b),
        _ => true,
    }
}

fn shallow_eq(a: &Node, b: &Node) -> bool {
    a.id == b.id
        && a.name == b.name
        && a.description == b.description
        && a.settings == b.settings
        && match (&a.kind, &b.kind) {
            (NodeKind::Folder { .. }, NodeKind::Folder { .. }) => true,
            (
                NodeKind::Connection {
                    protocol: pa,
                    host: ha,
                },
                NodeKind::Connection {
                    protocol: pb,
                    host: hb,
                },
            ) => pa == pb && ha == hb,
            _ => false,
        }
}

fn flatten<'t>(
    nodes: &'t [Node],
    parent: Option<NodeId>,
    out: &mut HashMap<NodeId, (Option<NodeId>, &'t Node)>,
) {
    for node in nodes {
        out.insert(node.id, (parent, node));
        if let NodeKind::Folder { children } = &node.kind {
            flatten(children, Some(node.id), out);
        }
    }
}

/// Nodes that differ from the base (added, removed, changed or moved, by id) plus one for a
/// changed vault file. Without a base everything is waiting.
fn count_pending(base: Option<&Snapshot>, local: &LocalState) -> usize {
    let local_vault = local.vault.as_ref().map(|v| v.file.as_slice());
    let Some(base) = base else {
        return count_nodes(local.tree.children()) + usize::from(local_vault.is_some());
    };
    let vault = usize::from(vault_changed(local_vault, base.vault.as_deref()));
    let (mut now, mut then) = (HashMap::new(), HashMap::new());
    flatten(local.tree.children(), None, &mut now);
    flatten(base.connections.children(), None, &mut then);
    let changed = now
        .iter()
        .filter(|(id, (parent, node))| match then.get(id) {
            Some((base_parent, base_node)) => parent != base_parent || !shallow_eq(node, base_node),
            None => true,
        })
        .count();
    let removed = then.keys().filter(|id| !now.contains_key(id)).count();
    changed + removed + vault
}

fn tree_doc_len(tree: &Tree) -> usize {
    serde_json::to_vec(&document(tree)).map_or(usize::MAX, |v| v.len())
}

impl Engine<'_> {
    fn snapshot(
        group: GroupId,
        seq: u64,
        writer: &DeviceInfo,
        written_at: &str,
        local: LocalState,
    ) -> Snapshot {
        Snapshot {
            format_version: SNAPSHOT_FORMAT_VERSION,
            group_id: group,
            seq,
            writer: writer.clone(),
            written_at: written_at.to_owned(),
            connections: local.tree,
            vault: local.vault.map(|v| v.file),
        }
    }

    /// Authenticates `bytes`, checks them against the group and builds the snapshot; nothing is
    /// trusted before every check passed.
    fn open_head(
        key: &SyncKey,
        group: GroupId,
        head_seq: u64,
        bytes: &[u8],
    ) -> Result<Snapshot, SyncError> {
        let opened = open(key, bytes)?;
        if opened.group != group {
            return Err(SyncError::WrongGroup);
        }
        if opened.seq != head_seq {
            return Err(invalid("the envelope's sequence number is not the head's"));
        }
        let snapshot = Snapshot::from_plaintext(&opened.plaintext)?;
        if snapshot.group_id != opened.group {
            return Err(invalid(
                "the snapshot belongs to another group than its envelope",
            ));
        }
        if snapshot.seq != opened.seq {
            return Err(invalid(
                "the snapshot's sequence number differs from its envelope",
            ));
        }
        if count_nodes(snapshot.connections.children()) > MAX_NODES {
            return Err(invalid("the snapshot has too many nodes"));
        }
        if let Some(vault) = &snapshot.vault {
            inspect_file(vault).map_err(|_| invalid("the vault in the snapshot is not usable"))?;
        }
        Ok(snapshot)
    }

    fn plan_vault(
        base: Option<&[u8]>,
        local: Option<&VaultSide>,
        head: Option<&[u8]>,
    ) -> Result<VaultPlan, SyncError> {
        let local_file = local.map(|v| v.file.as_slice());
        let Some(head) = head else {
            // the head has no vault: nothing to take (a local one is pushed)
            return Ok(VaultPlan::Keep);
        };
        let Some(local_file) = local_file else {
            return Ok(VaultPlan::Adopt(head.to_vec()));
        };
        if same_version(local_file, head) {
            return Ok(VaultPlan::Keep);
        }
        let local_changed = vault_changed(Some(local_file), base);
        let head_changed = vault_changed(Some(head), base);
        match (local_changed, head_changed) {
            (false, _) => Ok(VaultPlan::Adopt(head.to_vec())),
            (true, false) => Ok(VaultPlan::Keep),
            (true, true) => {
                let local_id = inspect_file(local_file)
                    .map_err(|_| SyncError::LocalFailed("vault_unreadable".to_owned()))?
                    .id;
                let head_id = inspect_file(head)
                    .map_err(|_| invalid("the vault in the snapshot is not usable"))?
                    .id;
                if local_id == head_id {
                    Ok(VaultPlan::Merge)
                } else {
                    Err(SyncError::VaultConflictDifferentVaults)
                }
            }
        }
    }

    /// The merged entries and findings, or `None` when the vault is locked.
    #[allow(clippy::type_complexity)]
    fn merge_vault(
        &self,
        base: Option<&[u8]>,
        side: &VaultSide,
        head: &[u8],
    ) -> Result<Option<(Vec<(EntryId, NewEntry)>, Vec<Finding>)>, SyncError> {
        let (Some(key), Some(local_entries)) = (&side.key, &side.entries) else {
            return Ok(None);
        };
        if !self.local.vault_unlocked() {
            return Ok(None);
        }
        let base_entries = match base {
            Some(bytes) => {
                read_entries(bytes, key)
                    .map_err(|_| {
                        SyncError::StateUnusable(
                            "the base vault does not open with this device's key".to_owned(),
                        )
                    })?
                    .entries
            }
            None => Vec::new(),
        };
        let head_entries = read_entries(head, key)
            .map_err(|_| invalid("the vault in the snapshot does not open with this key"))?
            .entries;
        let merged = merge_entries(&base_entries, local_entries, &head_entries, &self.ctx);
        Ok(Some((merged.entries, merged.findings)))
    }

    /// Writes the new base, then the state with `last_seq` (see the module comment for why).
    fn store(&self, state: &SyncState, base: &Snapshot) -> Result<(), SyncError> {
        self.state.save_base(base)?;
        self.state.save(&SyncState {
            last_seq: base.seq,
            ..state.clone()
        })
    }

    fn keys(&self, group: GroupId) -> Result<(SecretString, SyncKey), SyncError> {
        self.secrets.get(group)?.ok_or_else(|| {
            SyncError::KeychainUnavailable("the sync key is not in the keychain".to_owned())
        })
    }

    /// Changes waiting to be synced: tree nodes that differ from the base (added, removed,
    /// changed or moved) plus one when the vault file differs from the base's. Without a base
    /// every node and the vault count.
    ///
    /// # Errors
    /// `not_paired`, `state_unusable`, `local_failed`.
    pub fn pending_changes(&self) -> Result<usize, SyncError> {
        self.state.load()?.ok_or(SyncError::NotPaired)?;
        let base = self.state.load_base()?;
        Ok(count_pending(base.as_ref(), &self.local.read()?))
    }

    /// One sync run (spec decision 3).
    ///
    /// # Errors
    /// Hub errors other than "unreachable" (`hub_unauthorized`, `hub_tls_untrusted`,
    /// `hub_protocol`, ...), `wrong_group`, `wrong_key_or_damaged`, `snapshot_invalid`,
    /// `newer_format`, `snapshot_too_large`, `vault_conflict_different_vaults`, `not_paired`,
    /// `state_unusable`, `local_failed`, and `local_changed` or `hub_conflict` when a run had to
    /// start over more than five times.
    pub fn run_once(&self) -> Result<Outcome, SyncError> {
        let mut last = SyncError::LocalChanged;
        for _ in 0..MAX_ATTEMPTS {
            match self.attempt() {
                Err(e @ (SyncError::LocalChanged | SyncError::HubConflict)) => last = e,
                other => return other,
            }
        }
        Err(last)
    }

    #[allow(clippy::too_many_lines)]
    fn attempt(&self) -> Result<Outcome, SyncError> {
        // 1. Read the local data and the stored state (the port takes and releases its locks).
        let st = self.state.load()?.ok_or(SyncError::NotPaired)?;
        let base = self.state.load_base()?.ok_or_else(|| {
            SyncError::StateUnusable("the base snapshot is missing; re-seed or leave sync".into())
        })?;
        if base.group_id != st.group {
            return Err(SyncError::StateUnusable(
                "the base snapshot belongs to another group".to_owned(),
            ));
        }
        let local = self.local.read()?;

        // 2. Fetch the head, outside any lock. Unreachable (also a timeout on a large head) is
        //    the quiet offline state; nothing has been touched.
        let head = match self.hub.get_head() {
            Ok(head) => head,
            Err(SyncError::HubUnreachable) => {
                return Ok(Outcome::Offline {
                    pending_changes: count_pending(Some(&base), &local),
                });
            }
            Err(e) => return Err(e),
        };

        // 3. Rollback protection, before anything is decrypted or merged.
        let Some((head_seq, head_bytes)) = head else {
            return Ok(Outcome::HubRolledBack {
                found: 0,
                seen: st.last_seq,
            });
        };
        let (head_group, _) = peek_header(&head_bytes)?;
        if head_group != st.group {
            return Err(SyncError::WrongGroup);
        }
        if head_seq < st.last_seq {
            return Ok(Outcome::HubRolledBack {
                found: head_seq,
                seen: st.last_seq,
            });
        }
        let (_, key) = self.keys(st.group)?;
        let head_snapshot = Self::open_head(&key, st.group, head_seq, &head_bytes)?;
        drop(head_bytes);

        // 4. Merge: the tree always, the vault by V1 to V5.
        let tree = merge_tree(
            &base.connections,
            &local.tree,
            &head_snapshot.connections,
            &self.ctx,
        )?;
        let mut findings = tree.findings;
        let plan = Self::plan_vault(
            base.vault.as_deref(),
            local.vault.as_ref(),
            head_snapshot.vault.as_deref(),
        )?;
        let vault_change = match plan {
            VaultPlan::Keep => VaultChange::Keep,
            VaultPlan::Adopt(bytes) => VaultChange::Adopt(bytes),
            VaultPlan::Merge => {
                let head_file = head_snapshot.vault.as_deref().unwrap_or_default();
                let side = local.vault.as_ref().expect("a merge needs a local vault");
                match self.merge_vault(base.vault.as_deref(), side, head_file)? {
                    // V2: locked, so the whole run waits: nothing applied, nothing pushed
                    None => return Ok(Outcome::WaitingForUnlock),
                    Some((entries, entry_findings)) => {
                        findings.extend(entry_findings);
                        VaultChange::Merged {
                            head: head_file.to_vec(),
                            entries,
                        }
                    }
                }
            }
        };

        // What has to happen: apply locally (the merge differs from local), push (the result
        // differs from the head).
        let merged_tree = tree.tree;
        let tree_change = (merged_tree != local.tree).then(|| merged_tree.clone());
        let local_file = local.vault.as_ref().map(|v| v.file.as_slice());
        let push_vault = match &vault_change {
            VaultChange::Merged { .. } => true,
            VaultChange::Adopt(_) => false,
            VaultChange::Keep => local_file.is_some_and(|l| {
                head_snapshot
                    .vault
                    .as_deref()
                    .is_none_or(|h| !same_version(l, h))
            }),
        };
        let push = merged_tree != head_snapshot.connections || push_vault;
        let apply = tree_change.is_some() || !matches!(vault_change, VaultChange::Keep);

        // The size rule, checked before anything is written (the merged vault file is not
        // known yet, so the larger side stands in for it).
        if push {
            let vault_len = local_file
                .map_or(0, <[u8]>::len)
                .max(head_snapshot.vault.as_ref().map_or(0, Vec::len));
            let estimate = tree_doc_len(&merged_tree)
                .saturating_add(vault_len.saturating_add(2) / 3 * 4)
                .saturating_add(1024);
            if estimate > MAX_ENVELOPE_LEN {
                return Err(SyncError::SnapshotTooLarge);
            }
        }

        // 5. Apply locally; the port refuses with `local_changed` when the data moved on, and
        //    the run starts over.
        if apply {
            self.local.apply(
                &local,
                LocalChange {
                    tree: tree_change,
                    vault: vault_change,
                },
            )?;
        }

        if !push {
            // Local data equals the head now: it becomes the base.
            if apply || base.seq != head_seq || st.last_seq != head_seq {
                self.store(&st, &head_snapshot)?;
            }
            return Ok(if apply {
                Outcome::Synced {
                    findings,
                    seq: head_seq,
                }
            } else {
                Outcome::NothingToDo
            });
        }

        // 6. Push exactly what is on disk now (the merged vault file exists only after apply).
        let current = if apply { self.local.read()? } else { local };
        let new_seq = head_seq + 1;
        let snapshot = Self::snapshot(st.group, new_seq, &st.device, &self.ctx.date, current);
        let plaintext = Zeroizing::new(snapshot.to_plaintext()?);
        let envelope = seal(&key, st.group, new_seq, &plaintext)?;
        match self
            .hub
            .put_head(Expected::Seq(head_seq), new_seq, &envelope)
        {
            Ok(()) => {}
            Err(SyncError::HubUnreachable) => {
                return Ok(Outcome::Offline {
                    pending_changes: count_pending(
                        Some(&base),
                        &LocalState {
                            tree: snapshot.connections.clone(),
                            vault: snapshot.vault.clone().map(|file| VaultSide {
                                file,
                                entries: None,
                                key: None,
                            }),
                        },
                    ),
                });
            }
            Err(e) => return Err(e),
        }

        // 7. Base first, `sync.json` second.
        self.store(&st, &snapshot)?;
        Ok(Outcome::Synced {
            findings,
            seq: new_seq,
        })
    }

    /// Sets this device up as the first of a new group: generates the sync key and the group,
    /// checks that the hub is reachable and empty, uploads the local data as `seq` 1, stores
    /// the secrets and the state, and returns the pairing code.
    ///
    /// # Errors
    /// `already_paired`, the hub errors, `hub_not_empty`, `snapshot_too_large`,
    /// `keychain_unavailable` (nothing is uploaded then), `state_write_failed`.
    pub fn setup(&self, cfg: &HubConfig, device_name: &str) -> Result<PairingCode, SyncError> {
        if self.state.load()?.is_some() {
            return Err(SyncError::AlreadyPaired);
        }
        let client = HubClient::new(cfg)?;
        client.info()?;
        if client.get_head()?.is_some() {
            return Err(SyncError::HubNotEmpty);
        }
        let key = SyncKey::generate()?;
        let group: GroupId = Uuid::new_v4();
        let device = DeviceInfo {
            id: Uuid::new_v4(),
            name: device_name.to_owned(),
        };
        let local = self.local.read()?;
        let snapshot = Self::snapshot(group, 1, &device, &self.ctx.date, local);
        let plaintext = Zeroizing::new(snapshot.to_plaintext()?);
        let envelope = seal(&key, group, 1, &plaintext)?;

        // The keychain first: when it cannot store, nothing has been uploaded.
        self.secrets.set(group, &cfg.secret, &key)?;
        if let Err(e) = client.put_head(Expected::Empty, 1, &envelope) {
            let _ = self.secrets.delete(group);
            return Err(e);
        }
        let state = SyncState {
            url: cfg.url.clone(),
            ca_pem: cfg.ca_pem.clone(),
            device,
            group,
            last_seq: 1,
        };
        if let Err(e) = self.store(&state, &snapshot) {
            let _ = self.state.clear();
            let _ = self.secrets.delete(group);
            return Err(e);
        }
        Ok(PairingCode {
            url: cfg.url.clone(),
            secret: cfg.secret.clone(),
            key: SyncKey::from_bytes(*key.as_bytes()),
            group,
            ca_pem: cfg.ca_pem.clone(),
        })
    }

    /// Joins the group of `code`: the head replaces the local connections (and the vault, when
    /// the head has one; otherwise the local vault stays and the next run shares it). The base
    /// becomes the head. Confirming the replacement and offering a backup are the caller's job.
    ///
    /// # Errors
    /// `already_paired`, the hub errors, `hub_empty`, `wrong_group`, `wrong_key_or_damaged`,
    /// `snapshot_invalid`, `local_changed`, `local_failed`, `keychain_unavailable`,
    /// `state_write_failed`.
    pub fn join(&self, code: &PairingCode, device_name: &str) -> Result<(), SyncError> {
        if self.state.load()?.is_some() {
            return Err(SyncError::AlreadyPaired);
        }
        let client = HubClient::new(&HubConfig {
            url: code.url.clone(),
            secret: code.secret.clone(),
            ca_pem: code.ca_pem.clone(),
        })?;
        let (head_seq, bytes) = client.get_head()?.ok_or(SyncError::HubEmpty)?;
        let (head_group, _) = peek_header(&bytes)?;
        if head_group != code.group {
            return Err(SyncError::WrongGroup);
        }
        let head = Self::open_head(&code.key, code.group, head_seq, &bytes)?;
        drop(bytes);

        let local = self.local.read()?;
        let vault = match (&head.vault, &local.vault) {
            (Some(h), Some(l)) if *h == l.file => VaultChange::Keep,
            (Some(h), _) => VaultChange::Adopt(h.clone()),
            (None, _) => VaultChange::Keep,
        };
        let state = SyncState {
            url: code.url.clone(),
            ca_pem: code.ca_pem.clone(),
            device: DeviceInfo {
                id: Uuid::new_v4(),
                name: device_name.to_owned(),
            },
            group: code.group,
            last_seq: head_seq,
        };
        // The keychain first: when it cannot store, local data stays as it is.
        self.secrets.set(code.group, &code.secret, &code.key)?;
        let applied = self.local.apply(
            &local,
            LocalChange {
                tree: Some(head.connections.clone()),
                vault,
            },
        );
        if let Err(e) = applied {
            let _ = self.secrets.delete(code.group);
            return Err(e);
        }
        if let Err(e) = self.store(&state, &head) {
            let _ = self.state.clear();
            let _ = self.secrets.delete(code.group);
            return Err(e);
        }
        Ok(())
    }

    /// Pushes the local state as a new head with a `seq` above everything seen: this device's
    /// `last_seq` and the hub's head. Valid for an empty hub or a head of this group. The base
    /// becomes what was pushed.
    ///
    /// # Errors
    /// `not_paired`, `keychain_unavailable`, the hub errors (`hub_conflict` when another device
    /// pushed meanwhile), `wrong_group`, `snapshot_too_large`, `state_write_failed`.
    pub fn reseed(&self) -> Result<(), SyncError> {
        let st = self.state.load()?.ok_or(SyncError::NotPaired)?;
        let (_, key) = self.keys(st.group)?;
        let (expected, new_seq) = match self.hub.get_head()? {
            Some((head_seq, bytes)) => {
                let (group, _) = peek_header(&bytes)?;
                if group != st.group {
                    return Err(SyncError::WrongGroup);
                }
                (Expected::Seq(head_seq), st.last_seq.max(head_seq) + 1)
            }
            None => (Expected::Empty, st.last_seq + 1),
        };
        let local = self.local.read()?;
        let snapshot = Self::snapshot(st.group, new_seq, &st.device, &self.ctx.date, local);
        let plaintext = Zeroizing::new(snapshot.to_plaintext()?);
        let envelope = seal(&key, st.group, new_seq, &plaintext)?;
        self.hub.put_head(expected, new_seq, &envelope)?;
        self.store(&st, &snapshot)
    }
}
