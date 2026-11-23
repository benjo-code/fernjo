//! Vault auto-lock after idle time (M10).
//!
//! Rust owns the clock: the UI only sends a throttled heartbeat (`note_activity`), so a
//! reloaded or hung page cannot stop the lock from happening. [`IdleTracker`] is a pure
//! clock over injected time; [`spawn`] is the thin thread that looks at it every few seconds
//! and locks through `vault_commands::vault_lock` (the same lock order as every other lock).
//! Connected sessions keep running; only new credential lookups need the unlock again.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};

use crate::state::{AppState, VaultState};
use crate::vault_commands::{self, VaultStatus};

/// How often the thread looks at the clock.
const TICK: Duration = Duration::from_secs(5);

/// The time of the last user activity.
#[derive(Debug)]
pub struct IdleTracker {
    last: Mutex<Instant>,
}

impl IdleTracker {
    pub fn new(now: Instant) -> Self {
        Self {
            last: Mutex::new(now),
        }
    }

    /// Records activity at `now`.
    pub fn note(&self, now: Instant) {
        if let Ok(mut last) = self.last.lock() {
            // never move the clock backwards (a late heartbeat must not shorten the idle time)
            if now > *last {
                *last = now;
            }
        }
    }

    /// Whether the idle time has reached `limit_minutes`; never when the limit is off. A
    /// lowered limit applies at once because the limit is read at every check.
    pub fn is_due(&self, now: Instant, limit_minutes: Option<u32>) -> bool {
        let Some(minutes) = limit_minutes else {
            return false;
        };
        let Ok(last) = self.last.lock() else {
            return false;
        };
        now.saturating_duration_since(*last) >= Duration::from_secs(u64::from(minutes) * 60)
    }
}

fn vault_is_unlocked(state: &AppState) -> bool {
    state
        .vault
        .lock()
        .is_ok_and(|v| matches!(&*v, VaultState::Open(v) if v.is_unlocked()))
}

/// One look at the clock: locks the vault when it is unlocked and the idle time is over.
/// Returns the new status when it locked. A lock error is ignored; the next tick retries.
pub fn tick(state: &AppState, now: Instant) -> Option<VaultStatus> {
    let limit = state.vault_auto_lock_minutes.lock().ok().map(|l| *l)?;
    if !state.idle.is_due(now, limit) || !vault_is_unlocked(state) {
        return None;
    }
    vault_commands::vault_lock(state).ok()
}

/// Starts the idle thread. It needs no secret and runs for the life of the app.
pub fn spawn(app: AppHandle) {
    let _ = std::thread::Builder::new()
        .name("idle-lock".into())
        .spawn(move || {
            loop {
                std::thread::sleep(TICK);
                let state = app.state::<AppState>();
                if let Some(status) = tick(&state, Instant::now()) {
                    let _ = app.emit("vault-status", status);
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: Duration = Duration::from_secs(60);

    #[test]
    fn not_due_while_activity_is_within_the_limit() {
        let t0 = Instant::now();
        let tracker = IdleTracker::new(t0);
        assert!(!tracker.is_due(t0 + 14 * MIN, Some(15)));
    }

    #[test]
    fn due_after_the_limit_without_input() {
        let t0 = Instant::now();
        let tracker = IdleTracker::new(t0);
        assert!(tracker.is_due(t0 + 15 * MIN, Some(15)));
        assert!(tracker.is_due(t0 + 16 * MIN, Some(15)));
    }

    #[test]
    fn never_due_when_the_limit_is_off() {
        let t0 = Instant::now();
        let tracker = IdleTracker::new(t0);
        assert!(!tracker.is_due(t0 + 100_000 * MIN, None));
    }

    #[test]
    fn a_lowered_limit_applies_at_once() {
        let t0 = Instant::now();
        let tracker = IdleTracker::new(t0);
        let now = t0 + 30 * MIN;
        assert!(!tracker.is_due(now, Some(60)));
        assert!(tracker.is_due(now, Some(10)));
    }

    #[test]
    fn activity_resets_the_clock() {
        let t0 = Instant::now();
        let tracker = IdleTracker::new(t0);
        tracker.note(t0 + 14 * MIN);
        assert!(!tracker.is_due(t0 + 20 * MIN, Some(15)));
        assert!(tracker.is_due(t0 + 29 * MIN, Some(15)));
    }

    #[test]
    fn tick_locks_an_unlocked_vault_only_when_due() {
        let dir = tempfile::tempdir().unwrap();
        let st = AppState::open(Some(dir.path().to_path_buf()));
        vault_commands::vault_create(&st, secrecy::SecretString::from("a master password"), false)
            .unwrap();
        assert!(vault_is_unlocked(&st));
        let t0 = Instant::now();
        st.idle.note(t0);
        // off: nothing happens
        assert!(tick(&st, t0 + 500 * MIN).is_none());
        *st.vault_auto_lock_minutes.lock().unwrap() = Some(5);
        assert!(tick(&st, t0 + 4 * MIN).is_none());
        assert!(vault_is_unlocked(&st));
        let status = tick(&st, t0 + 5 * MIN).expect("locks");
        assert_eq!(status.state, "locked");
        assert!(!vault_is_unlocked(&st));
        // already locked: nothing more to do
        assert!(tick(&st, t0 + 6 * MIN).is_none());
    }
}
