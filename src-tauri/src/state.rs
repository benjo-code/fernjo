//! Application state shared by all commands.

use std::path::PathBuf;
use std::sync::Mutex;

use fernjo_core::{Notice, Store};

/// Why the data could not be opened. Every command reports this until the app restarts.
#[derive(Debug, Clone)]
pub struct StartupFailure {
    pub code: String,
    pub message: String,
}

/// One store behind a lock, plus the startup notice that is delivered exactly once.
pub struct AppState {
    pub store: Mutex<Result<Store, StartupFailure>>,
    pub notice: Mutex<Option<Notice>>,
}

impl AppState {
    /// Opens the store in `dir`. `None` (no config directory on this system) and every
    /// load error become a [`StartupFailure`]; nothing is written in that case.
    pub fn open(dir: Option<PathBuf>) -> AppState {
        let (store, notice) = match dir {
            None => (
                Err(StartupFailure {
                    code: "load_failed".to_string(),
                    message: "no data folder is available on this system; set FERNJO_DATA_DIR"
                        .to_string(),
                }),
                None,
            ),
            Some(dir) => match Store::open(&dir) {
                Ok(opened) => (Ok(opened.store), opened.notice),
                Err(e) => (
                    Err(StartupFailure {
                        code: "load_failed".to_string(),
                        message: e.to_string(),
                    }),
                    None,
                ),
            },
        };
        AppState {
            store: Mutex::new(store),
            notice: Mutex::new(notice),
        }
    }
}
