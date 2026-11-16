//! Application state shared by all commands.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

use fernjo_core::{
    DataDirSource, Location, Notice, Store, TerminalPrefs, find_conflicts, settings,
};
use fernjo_vault::{
    FILE_NAME as VAULT_FILE_NAME, KdfParams, KeyStore, MemoryKeyStore, OsKeyStore, Vault,
    try_auto_unlock,
};

use crate::commands::CommandError;

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

/// One store behind a lock, the startup notices (delivered once), the vault and the data
/// folder location.
pub struct AppState {
    pub store: Mutex<Result<Store, StartupFailure>>,
    pub notices: Mutex<Vec<Notice>>,
    pub vault: Mutex<VaultState>,
    pub keystore: Box<dyn KeyStore>,
    /// Argon2id cost for vaults created by this run (tests use `KdfParams::TEST`).
    pub kdf: KdfParams,
    pub data_dir: Option<PathBuf>,
    /// Folder of `settings.json` (`None` for the test constructor).
    pub config_dir: Option<PathBuf>,
    /// Where `data_dir` came from. Task 8 must not save `settings.json` when this is `Env`
    /// (the variable wins) nor when `settings_unusable` is set.
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
            None,
            DataDirSource::Default,
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
                None,
                DataDirSource::Default,
                false,
                None,
                keystore,
                KdfParams::default(),
            ),
            Some(l) => {
                // `settings.json` is neither read nor written while the variable is set, and
                // an unusable file yields the defaults.
                let terminal = if l.source == DataDirSource::Env || l.settings_error.is_some() {
                    None
                } else {
                    settings::load(&l.config_dir.join(settings::FILE_NAME))
                        .ok()
                        .and_then(|s| s.terminal)
                };
                let state = Self::build(
                    Some(l.data_dir),
                    Some(l.config_dir),
                    l.source,
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
                state
            }
        }
    }

    /// Test constructor with an explicit key store.
    #[cfg(test)]
    pub(crate) fn open_with_keystore(dir: PathBuf, keystore: Box<dyn KeyStore>) -> AppState {
        Self::build(
            Some(dir),
            None,
            DataDirSource::Default,
            false,
            None,
            keystore,
            KdfParams::TEST,
        )
    }

    fn build(
        dir: Option<PathBuf>,
        config_dir: Option<PathBuf>,
        source: DataDirSource,
        settings_unusable: bool,
        settings_notice: Option<Notice>,
        keystore: Box<dyn KeyStore>,
        kdf: KdfParams,
    ) -> AppState {
        let mut notices = Vec::new();
        let mut vault = VaultState::None;
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
                    // The outcome only matters for the UI, which asks `vault_status`.
                    let _ = try_auto_unlock(&mut v, keystore.as_ref());
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
            keystore,
            kdf,
            data_dir: dir,
            config_dir,
            source,
            settings_unusable,
            restart_required: AtomicBool::new(false),
            pending_data_dir: Mutex::new(None),
            terminal: Mutex::new(TerminalPrefs::default()),
        }
    }

    #[cfg(test)]
    pub(crate) fn set_restart_required(&self, value: bool) {
        self.restart_required
            .store(value, std::sync::atomic::Ordering::SeqCst);
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
