//! Application state shared by all commands.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use fernjo_core::{
    DataDirSource, Location, Notice, Store, TerminalPrefs, Theme, find_conflicts, settings,
};
use fernjo_vault::{
    AutoUnlock, FILE_NAME as VAULT_FILE_NAME, FileGenerationStore, GenerationStore, KdfParams,
    KeyStore, MemoryGenerationStore, MemoryKeyStore, OsKeyStore, Vault, try_auto_unlock,
};

use crate::commands::CommandError;
use crate::idle_lock::IdleTracker;
use crate::import_commands::Pending;
use crate::sessions::SessionManager;
use crate::sync_commands::SyncRuntime;
use crate::sync_runner::SyncWake;
use fernjo_sync::secrets::{MemorySyncSecrets, OsSyncSecrets, SyncSecrets};

/// Why the data could not be opened. Every command reports this until the app restarts.
#[derive(Debug, Clone)]
pub struct StartupFailure {
    pub code: String,
    pub message: String,
}

/// The vault as found at startup. "Locked" is `Open(vault)` with `!vault.is_unlocked()`.
pub enum VaultState {
    /// No vault file exists yet.
    None,
    /// A usable vault file; locked or unlocked.
    Open(Vault),
    /// The vault file exists but cannot be used. It is never overwritten.
    Unusable { reason: String, path: PathBuf },
}

/// What commands report about the vault (single reader: [`AppState::vault_status`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultStatus {
    None,
    Locked,
    Unlocked,
    Unusable { reason: String, path: PathBuf },
}

/// Why the startup auto-unlock did not unlock although a key may be stored. Only the
/// message to the user depends on it; the keychain item is never touched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoUnlockIssue {
    /// The stored key did not open the file (the file may be damaged, half-synced or replaced).
    KeyNotAccepted,
    /// The keychain could not be read; nothing is known about a stored key.
    KeychainUnavailable,
    /// The vault file could not be used.
    Unavailable,
    /// The file is older than the version this device last opened.
    RolledBack,
}

impl AutoUnlockIssue {
    /// The issue behind an outcome; `None` when nothing is worth telling the user
    /// (unlocked, or no key stored).
    pub fn from_outcome(outcome: &AutoUnlock) -> Option<Self> {
        match outcome {
            AutoUnlock::Unlocked | AutoUnlock::NoKey => None,
            AutoUnlock::KeyNotAccepted => Some(Self::KeyNotAccepted),
            AutoUnlock::KeychainUnavailable(_) => Some(Self::KeychainUnavailable),
            AutoUnlock::Unavailable(_) => Some(Self::Unavailable),
            AutoUnlock::RolledBack { .. } => Some(Self::RolledBack),
        }
    }

    /// The wire code the UI reads (`autoUnlockIssue` in the vault status).
    pub fn code(self) -> &'static str {
        match self {
            Self::KeyNotAccepted => "key_not_accepted",
            Self::KeychainUnavailable => "keychain_unavailable",
            Self::Unavailable => "unavailable",
            Self::RolledBack => "rolled_back",
        }
    }
}

/// The folders besides the data folder that `build` records.
struct Folders {
    config_dir: Option<PathBuf>,
    device_dir: Option<PathBuf>,
    source: DataDirSource,
}

impl Folders {
    /// No config or device folder, source `Default` (tests, no location).
    fn none() -> Self {
        Folders {
            config_dir: None,
            device_dir: None,
            source: DataDirSource::Default,
        }
    }
}

/// One store behind a lock, the startup notices (delivered once), the vault and the data
/// folder location.
pub struct AppState {
    pub store: Mutex<Result<Store, StartupFailure>>,
    pub notices: Mutex<Vec<Notice>>,
    pub vault: Mutex<VaultState>,
    /// Why the startup auto-unlock failed (set once at startup, cleared by a successful
    /// unlock or a new vault). Shown while the vault is locked.
    pub auto_unlock_issue: Mutex<Option<AutoUnlockIssue>>,
    pub keystore: Box<dyn KeyStore>,
    /// The highest vault generation this device has seen, per vault (rollback protection):
    /// a file in `device_dir` when there is one, else memory only.
    pub generations: Arc<dyn GenerationStore>,
    /// Argon2id cost for vaults created by this run (tests use `KdfParams::TEST`).
    pub kdf: KdfParams,
    pub data_dir: Option<PathBuf>,
    /// Folder of `settings.json` (`None` for the test constructor).
    pub config_dir: Option<PathBuf>,
    /// Device-local folder (never synced; `None` for the test constructor or without an OS
    /// local data dir). Not created here: its consumers create it and fall back to memory.
    pub device_dir: Option<PathBuf>,
    /// Where `data_dir` came from. `settings.json` is never saved when this is `Env` (the
    /// variable wins) nor when `settings_unusable` is set (`set_terminal_settings` enforces it).
    pub source: DataDirSource,
    /// `settings.json` has a newer schema (or was unusable): it must not be saved over.
    pub settings_unusable: bool,
    pub restart_required: AtomicBool,
    /// A "close while connected" dialog is open; a second close request must not stack another.
    pub close_prompt_open: AtomicBool,
    /// The folder chosen by `apply_data_folder` while a restart is pending (shown by
    /// `get_data_folder`) with the data files that already existed there; `None` otherwise.
    pub pending_data_dir: Mutex<Option<(PathBuf, Vec<String>)>>,
    /// Terminal preferences: loaded from `settings.json` at startup, in memory only while
    /// `FERNJO_DATA_DIR` is set.
    pub terminal: Mutex<TerminalPrefs>,
    /// Colour theme: loaded from `settings.json` at startup (same rule as `terminal`).
    pub theme: Mutex<Theme>,
    /// Idle minutes after which the vault locks itself (`None` = off): loaded from
    /// `settings.json` at startup, in memory only while `FERNJO_DATA_DIR` is set.
    pub vault_auto_lock_minutes: Mutex<Option<u32>>,
    /// Whether the open tabs are saved and reopened (default on): loaded from `settings.json`
    /// at startup (same rule as `terminal`). The lock also serialises writes of `open-tabs.json`.
    pub reopen_tabs: Mutex<bool>,
    /// When the user last did something (fed by `note_activity`); see `idle_lock`.
    pub idle: IdleTracker,
    /// The open terminal sessions.
    pub sessions: SessionManager,
    /// The import or restore that waits for inspect/apply (one at a time).
    pub pending: Mutex<Option<Pending>>,
    /// Set by `clear_pending` when it had to skip because an apply held the `pending` lock;
    /// `import_apply` then drops the pending value if it fails, and resets the flag.
    pub page_reloaded: AtomicBool,
    /// Multi-device sync (M9): what the last run reported, the findings still to show and the
    /// running flag. Holds no secret. A leaf lock: never held while taking another one.
    pub sync: Mutex<SyncRuntime>,
    /// The hub secret and the sync key per group (the OS keychain in the app, memory in tests).
    pub sync_secrets: Box<dyn SyncSecrets>,
    /// Counts every local edit of the synced data (tree command, credential command, import);
    /// the sync runner debounces its "after a local save" trigger on it. Sync's own writes do
    /// not count.
    pub edit_seq: AtomicU64,
    /// Counts the sync runs that wrote remote changes into the local data; the UI reloads the
    /// tree and the vault when it changes.
    pub sync_applied: AtomicU64,
    /// Wakes the sync runner (window focus).
    pub sync_wake: SyncWake,
    /// The window is closing and the best-effort push is under way (set once).
    pub sync_closing: AtomicBool,
}

/// Debug output never includes the vault, the key store or the connection data.
impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("data_dir", &self.data_dir)
            .field("source", &self.source)
            .field("settings_unusable", &self.settings_unusable)
            .finish_non_exhaustive()
    }
}

/// The notice for a `settings.json` data folder that contains the device-local folder. Only
/// for `DataDirSource::Settings`: with `FERNJO_DATA_DIR` the device folder is `<data>/.device`
/// by design, and the default data folder is the config folder.
fn device_folder_notice(
    source: DataDirSource,
    data_dir: Option<&Path>,
    device_dir: Option<&Path>,
) -> Option<Notice> {
    if source != DataDirSource::Settings {
        return None;
    }
    let (data, device) = (data_dir?, device_dir?);
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    (device.starts_with(data) || canon(device).starts_with(canon(data))).then(|| {
        Notice::DeviceFolderInsideDataDir {
            path: device.to_path_buf(),
        }
    })
}

impl AppState {
    /// Test constructor: in-memory key store, test KDF, no settings file
    /// (`source` is `Default`).
    pub fn open(dir: Option<PathBuf>) -> AppState {
        Self::build(
            dir,
            Folders::none(),
            false,
            None,
            Box::new(MemoryKeyStore::default()),
            KdfParams::TEST,
            None,
        )
    }

    /// Production constructor: OS key store and the default KDF cost.
    pub fn open_for_app(location: Option<Location>) -> AppState {
        let keystore: Box<dyn KeyStore> = Box::new(OsKeyStore);
        match location {
            None => Self::build(
                None,
                Folders::none(),
                false,
                None,
                keystore,
                KdfParams::default(),
                None,
            ),
            Some(l) => {
                // `settings.json` is neither read nor written while the variable is set, and
                // an unusable file yields the defaults.
                let loaded = if l.source == DataDirSource::Env || l.settings_error.is_some() {
                    None
                } else {
                    settings::load(&l.config_dir.join(settings::FILE_NAME)).ok()
                };
                let terminal = loaded.as_ref().and_then(|s| s.terminal.clone());
                let auto_lock = loaded.as_ref().and_then(|s| s.vault_auto_lock_minutes);
                let reopen_tabs = loaded.as_ref().and_then(|s| s.reopen_tabs);
                let theme = loaded.and_then(|s| s.theme);
                let mut state = Self::build(
                    Some(l.data_dir),
                    Folders {
                        config_dir: Some(l.config_dir),
                        device_dir: l.device_dir,
                        source: l.source,
                    },
                    l.settings_error.is_some(),
                    l.settings_error.map(|e| Notice::SettingsUnreadable {
                        message: e.to_string(),
                    }),
                    keystore,
                    KdfParams::default(),
                    None,
                );
                state.sync_secrets = Box::new(OsSyncSecrets);
                if let (Some(prefs), Ok(mut slot)) = (terminal, state.terminal.lock()) {
                    *slot = prefs;
                }
                if let (Some(theme), Ok(mut slot)) = (theme, state.theme.lock()) {
                    *slot = theme;
                }
                if let Ok(mut slot) = state.vault_auto_lock_minutes.lock() {
                    *slot = auto_lock.filter(|m| settings::AUTO_LOCK_MINUTES_RANGE.contains(m));
                }
                if let (Some(on), Ok(mut slot)) = (reopen_tabs, state.reopen_tabs.lock()) {
                    *slot = on;
                }
                state
            }
        }
    }

    /// Test constructor with an explicit key store.
    #[cfg(test)]
    pub(crate) fn open_with_keystore(dir: PathBuf, keystore: Box<dyn KeyStore>) -> AppState {
        Self::build(
            Some(dir),
            Folders::none(),
            false,
            None,
            keystore,
            KdfParams::TEST,
            None,
        )
    }

    /// Test constructor with an explicit key store and generation store (a "restart" that
    /// keeps the device marks).
    #[cfg(test)]
    pub(crate) fn open_with_stores(
        dir: PathBuf,
        keystore: Box<dyn KeyStore>,
        generations: Arc<dyn GenerationStore>,
    ) -> AppState {
        Self::build(
            Some(dir),
            Folders::none(),
            false,
            None,
            keystore,
            KdfParams::TEST,
            Some(generations),
        )
    }

    fn build(
        dir: Option<PathBuf>,
        folders: Folders,
        settings_unusable: bool,
        settings_notice: Option<Notice>,
        keystore: Box<dyn KeyStore>,
        kdf: KdfParams,
        generations: Option<Arc<dyn GenerationStore>>,
    ) -> AppState {
        let Folders {
            config_dir,
            device_dir,
            source,
        } = folders;
        let generations = generations.unwrap_or_else(|| match &device_dir {
            Some(dir) => Arc::new(FileGenerationStore::in_dir(dir)),
            None => Arc::new(MemoryGenerationStore::default()),
        });
        let mut notices = Vec::new();
        let mut vault = VaultState::None;
        let mut auto_unlock_issue = None;
        let store = match &dir {
            None => Err(StartupFailure {
                code: "load_failed".to_string(),
                message: "no data folder is available on this system; set FERNJO_DATA_DIR"
                    .to_string(),
            }),
            Some(dir) => match Store::open(dir) {
                Ok(opened) => {
                    notices.extend(opened.notice);
                    Ok(opened.store)
                }
                Err(e) => Err(StartupFailure {
                    code: "load_failed".to_string(),
                    message: e.to_string(),
                }),
            },
        };
        if let Some(dir) = &dir {
            let conflicts = find_conflicts(dir);
            if !conflicts.is_empty() {
                notices.push(Notice::SyncConflicts { files: conflicts });
            }
            let path = dir.join(VAULT_FILE_NAME);
            vault = match Vault::open(&path) {
                Ok(None) => VaultState::None,
                Ok(Some(mut v)) => {
                    // The marks come first: an older file must be recognised before the
                    // stored key opens it.
                    v.attach_generation_store(Arc::clone(&generations));
                    // Never deletes the keychain item; the UI learns of a failure through
                    // `vault_status` (`autoUnlockIssue`).
                    auto_unlock_issue =
                        AutoUnlockIssue::from_outcome(&try_auto_unlock(&mut v, keystore.as_ref()));
                    VaultState::Open(v)
                }
                Err(e) => VaultState::Unusable {
                    reason: e.to_string(),
                    path,
                },
            };
        }
        notices.extend(settings_notice);
        notices.extend(device_folder_notice(
            source,
            dir.as_deref(),
            device_dir.as_deref(),
        ));
        AppState {
            store: Mutex::new(store),
            notices: Mutex::new(notices),
            vault: Mutex::new(vault),
            auto_unlock_issue: Mutex::new(auto_unlock_issue),
            keystore,
            generations,
            kdf,
            data_dir: dir,
            config_dir,
            device_dir,
            source,
            settings_unusable,
            restart_required: AtomicBool::new(false),
            close_prompt_open: AtomicBool::new(false),
            pending_data_dir: Mutex::new(None),
            terminal: Mutex::new(TerminalPrefs::default()),
            theme: Mutex::new(Theme::default()),
            vault_auto_lock_minutes: Mutex::new(None),
            reopen_tabs: Mutex::new(true),
            idle: IdleTracker::new(std::time::Instant::now(), std::time::SystemTime::now()),
            sessions: SessionManager::default(),
            pending: Mutex::new(None),
            page_reloaded: AtomicBool::new(false),
            sync: Mutex::new(SyncRuntime::default()),
            sync_secrets: Box::new(MemorySyncSecrets::default()),
            edit_seq: AtomicU64::new(0),
            sync_applied: AtomicU64::new(0),
            sync_wake: SyncWake::default(),
            sync_closing: AtomicBool::new(false),
        }
    }

    /// Records a local edit of the synced data (the sync runner's debounce trigger).
    pub fn note_local_edit(&self) {
        self.edit_seq.fetch_add(1, Ordering::SeqCst);
    }

    /// Test constructor with a device folder (sync keeps its state there) and memory secrets.
    #[cfg(test)]
    pub(crate) fn open_with_device_dir(dir: PathBuf, device_dir: PathBuf) -> AppState {
        Self::build(
            Some(dir),
            Folders {
                config_dir: None,
                device_dir: Some(device_dir),
                source: DataDirSource::Default,
            },
            false,
            None,
            Box::new(MemoryKeyStore::default()),
            KdfParams::TEST,
            None,
        )
    }

    #[cfg(test)]
    pub(crate) fn set_restart_required(&self, value: bool) {
        self.restart_required
            .store(value, std::sync::atomic::Ordering::SeqCst);
    }

    /// Drops the pending import or restore (every page load does this, next to closing the
    /// sessions); a poisoned lock is recovered. It never waits: while an apply holds the lock
    /// (disk work on another thread) the call is skipped, which is safe because the apply
    /// clears the value itself on success, and on failure when `page_reloaded` is set. A pick or inspect holds the lock only
    /// to swap the value, never across I/O.
    pub fn clear_pending(&self) {
        match self.pending.try_lock() {
            Ok(mut guard) => *guard = None,
            Err(std::sync::TryLockError::Poisoned(p)) => *p.into_inner() = None,
            Err(std::sync::TryLockError::WouldBlock) => {
                // the apply drops the value itself on failure when it sees this flag
                self.page_reloaded.store(true, Ordering::SeqCst);
            }
        }
    }

    /// The vault's state as commands report it.
    ///
    /// # Errors
    /// `internal` if the vault lock is poisoned.
    pub fn vault_status(&self) -> Result<VaultStatus, CommandError> {
        let guard = self.vault.lock().map_err(|_| CommandError::internal())?;
        Ok(match &*guard {
            VaultState::None => VaultStatus::None,
            VaultState::Open(v) if v.is_unlocked() => VaultStatus::Unlocked,
            VaultState::Open(_) => VaultStatus::Locked,
            VaultState::Unusable { reason, path } => VaultStatus::Unusable {
                reason: reason.clone(),
                path: path.clone(),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fernjo_core::{DataDirSource, Notice};

    #[test]
    fn startup_notice_when_data_dir_contains_the_device_folder() {
        let data = Path::new("/sync/fernjo");
        let inside = Path::new("/sync/fernjo/dev");
        let outside = Path::new("/local/fernjo-device");
        assert_eq!(
            device_folder_notice(DataDirSource::Settings, Some(data), Some(inside)),
            Some(Notice::DeviceFolderInsideDataDir {
                path: inside.to_path_buf()
            })
        );
        assert_eq!(
            device_folder_notice(DataDirSource::Settings, Some(data), Some(data)),
            Some(Notice::DeviceFolderInsideDataDir {
                path: data.to_path_buf()
            })
        );
        assert_eq!(
            device_folder_notice(DataDirSource::Settings, Some(data), Some(outside)),
            None
        );
        // FERNJO_DATA_DIR puts the device folder in `<data>/.device` by design
        assert_eq!(
            device_folder_notice(DataDirSource::Env, Some(data), Some(inside)),
            None
        );
        assert_eq!(
            device_folder_notice(DataDirSource::Default, Some(data), Some(inside)),
            None
        );
        assert_eq!(
            device_folder_notice(DataDirSource::Settings, None, Some(inside)),
            None
        );
    }
}
