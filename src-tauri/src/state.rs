//! Application state shared by all commands.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

use fernjo_core::{
    DataDirSource, Location, Notice, Store, TerminalPrefs, Theme, find_conflicts, settings,
};
use fernjo_vault::{
    AutoUnlock, FILE_NAME as VAULT_FILE_NAME, KdfParams, KeyStore, MemoryKeyStore, OsKeyStore,
    Vault, try_auto_unlock,
};

use crate::commands::CommandError;
use crate::import_commands::Pending;
use crate::sessions::SessionManager;

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
        }
    }

    /// The wire code the UI reads (`autoUnlockIssue` in the vault status).
    pub fn code(self) -> &'static str {
        match self {
            Self::KeyNotAccepted => "key_not_accepted",
            Self::KeychainUnavailable => "keychain_unavailable",
            Self::Unavailable => "unavailable",
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
    /// The folder chosen by `apply_data_folder` while a restart is pending (shown by
    /// `get_data_folder`) with the data files that already existed there; `None` otherwise.
    pub pending_data_dir: Mutex<Option<(PathBuf, Vec<String>)>>,
    /// Terminal preferences: loaded from `settings.json` at startup, in memory only while
    /// `FERNJO_DATA_DIR` is set.
    pub terminal: Mutex<TerminalPrefs>,
    /// Colour theme: loaded from `settings.json` at startup (same rule as `terminal`).
    pub theme: Mutex<Theme>,
    /// The open terminal sessions.
    pub sessions: SessionManager,
    /// The import or restore that waits for inspect/apply (one at a time).
    pub pending: Mutex<Option<Pending>>,
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
                let theme = loaded.and_then(|s| s.theme);
                let state = Self::build(
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
                );
                if let (Some(prefs), Ok(mut slot)) = (terminal, state.terminal.lock()) {
                    *slot = prefs;
                }
                if let (Some(theme), Ok(mut slot)) = (theme, state.theme.lock()) {
                    *slot = theme;
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
        )
    }

    fn build(
        dir: Option<PathBuf>,
        folders: Folders,
        settings_unusable: bool,
        settings_notice: Option<Notice>,
        keystore: Box<dyn KeyStore>,
        kdf: KdfParams,
    ) -> AppState {
        let Folders {
            config_dir,
            device_dir,
            source,
        } = folders;
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
        AppState {
            store: Mutex::new(store),
            notices: Mutex::new(notices),
            vault: Mutex::new(vault),
            auto_unlock_issue: Mutex::new(auto_unlock_issue),
            keystore,
            kdf,
            data_dir: dir,
            config_dir,
            device_dir,
            source,
            settings_unusable,
            restart_required: AtomicBool::new(false),
            pending_data_dir: Mutex::new(None),
            terminal: Mutex::new(TerminalPrefs::default()),
            theme: Mutex::new(Theme::default()),
            sessions: SessionManager::default(),
            pending: Mutex::new(None),
        }
    }

    #[cfg(test)]
    pub(crate) fn set_restart_required(&self, value: bool) {
        self.restart_required
            .store(value, std::sync::atomic::Ordering::SeqCst);
    }

    /// Drops the pending import or restore (every page load does this, next to closing the
    /// sessions); a poisoned lock is recovered.
    pub fn clear_pending(&self) {
        *self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
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
