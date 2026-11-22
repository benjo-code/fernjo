//! The sync engine's view of the app's data: [`AppPort`] implements `LocalPort` over the `Store`
//! and the `Vault` in [`AppState`].
//!
//! Locking: every method takes the locks in the app's order (pending, then vault, then store),
//! copies out or writes, and releases them before it returns, so the engine never holds one while
//! it talks to the hub. `read` copies the tree, the vault file bytes and, while the vault is
//! unlocked, its entries and data key. `apply` takes the locks again and first compares the
//! current tree and vault file bytes with what the merge was made from (`expected`): any
//! difference is `local_changed` and nothing is written, so a user edit between "read" and
//! "apply" is never overwritten (the engine starts the run over). The writes then go through
//! `Vault` and `Store`, which keep their own guards (the vault header guard, the generation
//! floor, the `connections.json` fingerprint): a refusal of one of them (`changed_on_disk`,
//! `rolled_back`, ...) is reported as `SyncError::LocalFailed(code)` with that stable code and
//! ends the run. The vault is written before the tree, so the more guarded write refuses first;
//! a run that is cut off between the two is repaired by the next one (the merge is idempotent).

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, MutexGuard};

use fernjo_core::{Node, NodeKind, Store, Tree};
use fernjo_sync::SyncError;
use fernjo_sync::ports::{LocalChange, LocalPort, LocalState, VaultChange, VaultSide};
use fernjo_vault::{FILE_NAME as VAULT_FILE_NAME, Vault, VaultError, inspect_file};

use crate::import_commands::Pending;
use crate::state::{AppState, StartupFailure, VaultState};

/// The port the app gives the engine.
pub struct AppPort<'a>(pub &'a AppState);

fn local_failed(code: &str) -> SyncError {
    SyncError::LocalFailed(code.to_owned())
}

fn vault_failed(e: &VaultError) -> SyncError {
    SyncError::LocalFailed(e.code().to_owned())
}

/// A poisoned lock means a thread panicked while it held the data; refuse instead of reading it.
fn lock<T>(m: &Mutex<T>) -> Result<MutexGuard<'_, T>, SyncError> {
    m.lock().map_err(|_| local_failed("internal"))
}

/// The tree, the vault and the guards over them, in lock order.
struct Locked<'a> {
    _pending: MutexGuard<'a, Option<Pending>>,
    vault: MutexGuard<'a, VaultState>,
    store: MutexGuard<'a, Result<Store, StartupFailure>>,
}

fn take(state: &AppState) -> Result<Locked<'_>, SyncError> {
    // An import or restore that is applying holds `pending` for its whole write; waiting for it
    // is correct (and never involves the network).
    let pending = lock(&state.pending)?;
    let vault = lock(&state.vault)?;
    let store = lock(&state.store)?;
    Ok(Locked {
        _pending: pending,
        vault,
        store,
    })
}

fn tree_of(locked: &Locked<'_>) -> Result<Tree, SyncError> {
    match &*locked.store {
        Ok(store) => Ok(store.tree().clone()),
        Err(failure) => Err(local_failed(&failure.code)),
    }
}

impl LocalPort for AppPort<'_> {
    fn read(&self) -> Result<LocalState, SyncError> {
        let locked = take(self.0)?;
        let tree = tree_of(&locked)?;
        let vault = match &*locked.vault {
            VaultState::None => None,
            // A vault file that cannot be read is never pushed, merged or replaced.
            VaultState::Unusable { .. } => return Err(local_failed("vault_unusable")),
            VaultState::Open(v) => {
                let file = v.file_bytes().map_err(|e| vault_failed(&e))?;
                let (entries, key) = if v.is_unlocked() {
                    (
                        Some(v.export_entries().map_err(|e| vault_failed(&e))?),
                        Some(v.data_key().map_err(|e| vault_failed(&e))?),
                    )
                } else {
                    (None, None)
                };
                Some(VaultSide { file, entries, key })
            }
        };
        Ok(LocalState { tree, vault })
    }

    fn apply(&self, expected: &LocalState, next: LocalChange) -> Result<(), SyncError> {
        let state = self.0;
        if state.restart_required.load(Ordering::SeqCst) {
            return Err(local_failed("restart_required"));
        }
        let mut locked = take(state)?;

        // Compare first: the tree and the vault file bytes must still be what was merged.
        let tree_now = tree_of(&locked)?;
        let file_now = match &*locked.vault {
            VaultState::None => None,
            VaultState::Unusable { .. } => return Err(local_failed("vault_unusable")),
            VaultState::Open(v) => Some(v.file_bytes().map_err(|e| vault_failed(&e))?),
        };
        let same_tree = tree_now == expected.tree;
        let same_vault = file_now.as_deref() == expected.vault.as_ref().map(|v| v.file.as_slice());
        if !(same_tree && same_vault) {
            return Err(SyncError::LocalChanged);
        }

        let changes_something = next.tree.is_some() || !matches!(next.vault, VaultChange::Keep);
        apply_vault(state, &mut locked.vault, next.vault)?;
        if let Some(tree) = next.tree {
            match &mut *locked.store {
                Ok(store) => store
                    .replace_tree(tree)
                    .map_err(|e| local_failed(e.code()))?,
                Err(failure) => return Err(local_failed(&failure.code)),
            }
        }
        if changes_something {
            state.sync_applied.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    }

    fn vault_unlocked(&self) -> bool {
        self.0
            .vault
            .lock()
            .is_ok_and(|v| matches!(&*v, VaultState::Open(v) if v.is_unlocked()))
    }
}

/// The vault part of an apply. `Adopt` of a file of the same vault keeps an unlocked vault
/// unlocked (`adopt_file`); another vault id (a joined device that had made its own vault) or no
/// vault at all installs the file and leaves the vault locked: the key of the old file does not
/// open the new one.
fn apply_vault(
    state: &AppState,
    slot: &mut VaultState,
    change: VaultChange,
) -> Result<(), SyncError> {
    match change {
        VaultChange::Keep => Ok(()),
        VaultChange::Merged { head, entries } => match slot {
            VaultState::Open(v) => v.adopt_merged(&head, entries).map_err(|e| vault_failed(&e)),
            VaultState::None => Err(local_failed("no_vault")),
            VaultState::Unusable { .. } => Err(local_failed("vault_unusable")),
        },
        VaultChange::Adopt(bytes) => match slot {
            VaultState::Open(v) => {
                let id = inspect_file(&bytes).map_err(|e| vault_failed(&e))?.id;
                if id == v.id() {
                    v.adopt_file(&bytes).map_err(|e| vault_failed(&e))
                } else {
                    v.replace_with_file(&bytes).map_err(|e| vault_failed(&e))?;
                    forget_auto_unlock_issue(state);
                    Ok(())
                }
            }
            VaultState::None => {
                let dir = state
                    .data_dir
                    .as_ref()
                    .ok_or_else(|| local_failed("no_data_dir"))?;
                let mut v = Vault::install_file(&dir.join(VAULT_FILE_NAME), &bytes)
                    .map_err(|e| vault_failed(&e))?;
                v.attach_generation_store(Arc::clone(&state.generations));
                *slot = VaultState::Open(v);
                forget_auto_unlock_issue(state);
                Ok(())
            }
            VaultState::Unusable { .. } => Err(local_failed("vault_unusable")),
        },
    }
}

/// The startup auto-unlock note belongs to the file that was replaced.
fn forget_auto_unlock_issue(state: &AppState) {
    if let Ok(mut issue) = state.auto_unlock_issue.lock() {
        *issue = None;
    }
}

/// How many connections and folders a tree holds (for the join confirmation).
pub fn count_nodes(nodes: &[Node]) -> (usize, usize) {
    let (mut connections, mut folders) = (0, 0);
    for node in nodes {
        match &node.kind {
            NodeKind::Connection { .. } => connections += 1,
            NodeKind::Folder { children } => {
                folders += 1;
                let (c, f) = count_nodes(children);
                connections += c;
                folders += f;
            }
        }
    }
    (connections, folders)
}
