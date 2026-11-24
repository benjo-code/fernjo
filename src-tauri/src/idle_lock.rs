//! Vault auto-lock after idle time (M10).
//!
//! Rust owns the clock: the UI only sends a throttled heartbeat (`note_activity`), so a
//! reloaded or hung page cannot stop the lock from happening. [`IdleTracker`] is a pure
//! clock over injected time; [`spawn`] is the thin thread that looks at it every few seconds
//! and locks through `vault_commands::vault_lock` (the same lock order as every other lock).
//! Connected sessions keep running; only new credential lookups need the unlock again.

use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use tauri::{AppHandle, Emitter, Manager};

use crate::state::{AppState, VaultState};
use crate::vault_commands::{self, VaultStatus};

/// How often the thread looks at the clock.
const TICK: Duration = Duration::from_secs(5);

/// The time of the last user activity, on two clocks. `Instant` is steady but stops while the
/// machine sleeps on macOS and Linux; the wall clock keeps running, so the larger of the two
/// elapsed values is the idle time and a laptop closed with the vault open locks on wake.
#[derive(Debug)]
pub struct IdleTracker {
    last: Mutex<(Instant, SystemTime)>,
}

impl IdleTracker {
    pub fn new(now: Instant, wall: SystemTime) -> Self {
        Self {
            last: Mutex::new((now, wall)),
        }
    }

    /// Records activity at `now` / `wall`.
    pub fn note(&self, now: Instant, wall: SystemTime) {
        if let Ok(mut last) = self.last.lock() {
            // never move the steady clock backwards (a late heartbeat must not shorten the
            // idle time); a wall clock that went back is taken over, otherwise the stamp
            // would stay in the future and the wall side could never count again
            if now > last.0 {
                last.0 = now;
            }
            if wall > last.1
                || last
                    .1
                    .duration_since(wall)
                    .is_ok_and(|back| !back.is_zero())
            {
                last.1 = wall;
            }
        }
    }

    /// Whether the idle time has reached `limit_minutes`; never when the limit is off. A
    /// lowered limit applies at once because the limit is read at every check. The idle time
    /// is the larger of the steady and the wall clock difference; a wall clock earlier than
    /// the stamp counts as zero, so it falls back to the steady value and never delays the
    /// lock.
    pub fn is_due(&self, now: Instant, wall: SystemTime, limit_minutes: Option<u32>) -> bool {
        let Some(minutes) = limit_minutes else {
            return false;
        };
        let Ok(last) = self.last.lock() else {
            return false;
        };
        let steady = now.saturating_duration_since(last.0);
        let wall = wall.duration_since(last.1).unwrap_or(Duration::ZERO);
        steady.max(wall) >= Duration::from_secs(u64::from(minutes) * 60)
    }
}

fn vault_is_unlocked(state: &AppState) -> bool {
    // a poisoned mutex is read through: a vault that may still hold its key must be locked
    let guard = state
        .vault
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    matches!(&*guard, VaultState::Open(v) if v.is_unlocked())
}

/// One look at the clock: locks the vault when it is unlocked and the idle time is over.
/// Returns the new status when it locked. A lock error is ignored; the next tick retries.
pub fn tick(state: &AppState, now: Instant, wall: SystemTime) -> Option<VaultStatus> {
    let limit = state.vault_auto_lock_minutes.lock().ok().map(|l| *l)?;
    if !state.idle.is_due(now, wall, limit) || !vault_is_unlocked(state) {
        return None;
    }
    vault_commands::vault_lock(state).ok()
}

/// Runs `f` and turns a panic into `None`, so one bad tick cannot end the idle thread (a dead
/// thread would silently stop the auto-lock for the rest of the run). It covers a panic inside
/// the tick; a vault mutex poisoned by a panic elsewhere is handled separately: the unlocked
/// check and `vault_commands::vault_lock` read through the poison.
fn guarded<T>(f: impl FnOnce() -> Option<T>) -> Option<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or(None)
}

/// [`tick`] behind a panic guard (see [`guarded`] for what it covers).
pub fn run_tick(state: &AppState, now: Instant, wall: SystemTime) -> Option<VaultStatus> {
    guarded(|| tick(state, now, wall))
}

/// Starts the idle thread. It needs no secret and runs for the life of the app.
pub fn spawn(app: AppHandle) {
    let _ = std::thread::Builder::new()
        .name("idle-lock".into())
        .spawn(move || {
            loop {
                std::thread::sleep(TICK);
                let state = app.state::<AppState>();
                if let Some(status) = run_tick(&state, Instant::now(), SystemTime::now()) {
                    let _ = app.emit("vault-status", status);
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: Duration = Duration::from_secs(60);

    /// Both clocks start at the same moment.
    fn start() -> (Instant, SystemTime) {
        (Instant::now(), SystemTime::now())
    }

    #[test]
    fn not_due_while_activity_is_within_the_limit() {
        let (t0, w0) = start();
        let tracker = IdleTracker::new(t0, w0);
        assert!(!tracker.is_due(t0 + 14 * MIN, w0 + 14 * MIN, Some(15)));
    }

    #[test]
    fn due_after_the_limit_without_input() {
        let (t0, w0) = start();
        let tracker = IdleTracker::new(t0, w0);
        assert!(tracker.is_due(t0 + 15 * MIN, w0 + 15 * MIN, Some(15)));
        assert!(tracker.is_due(t0 + 16 * MIN, w0 + 16 * MIN, Some(15)));
    }

    #[test]
    fn never_due_when_the_limit_is_off() {
        let (t0, w0) = start();
        let tracker = IdleTracker::new(t0, w0);
        assert!(!tracker.is_due(t0 + 100_000 * MIN, w0 + 100_000 * MIN, None));
    }

    #[test]
    fn a_lowered_limit_applies_at_once() {
        let (t0, w0) = start();
        let tracker = IdleTracker::new(t0, w0);
        let (now, wall) = (t0 + 30 * MIN, w0 + 30 * MIN);
        assert!(!tracker.is_due(now, wall, Some(60)));
        assert!(tracker.is_due(now, wall, Some(10)));
    }

    #[test]
    fn activity_resets_the_clock() {
        let (t0, w0) = start();
        let tracker = IdleTracker::new(t0, w0);
        tracker.note(t0 + 14 * MIN, w0 + 14 * MIN);
        assert!(!tracker.is_due(t0 + 20 * MIN, w0 + 20 * MIN, Some(15)));
        assert!(tracker.is_due(t0 + 29 * MIN, w0 + 29 * MIN, Some(15)));
    }

    #[test]
    fn a_sleep_gap_in_the_wall_clock_counts_as_idle_time() {
        let (t0, w0) = start();
        let tracker = IdleTracker::new(t0, w0);
        // the steady clock stood still while the machine slept: 10 s on it, 8 h on the wall
        let eight_hours = 8 * 60 * MIN;
        assert!(tracker.is_due(t0 + Duration::from_secs(10), w0 + eight_hours, Some(15)));
        // without the gap the same steady time is far from due
        assert!(!tracker.is_due(
            t0 + Duration::from_secs(10),
            w0 + Duration::from_secs(10),
            Some(15)
        ));
    }

    #[test]
    fn a_wall_clock_that_went_backwards_does_not_delay_the_lock() {
        let (t0, w0) = start();
        let tracker = IdleTracker::new(t0, w0 + 60 * MIN);
        // wall earlier than the stamp: counts as zero, the steady value decides
        assert!(tracker.is_due(t0 + 15 * MIN, w0, Some(15)));
        assert!(!tracker.is_due(t0 + 14 * MIN, w0, Some(15)));
        // activity after the jump takes the earlier wall time over
        tracker.note(t0 + 20 * MIN, w0 + 20 * MIN);
        assert!(!tracker.is_due(t0 + 21 * MIN, w0 + 21 * MIN, Some(15)));
        assert!(tracker.is_due(t0 + 21 * MIN, w0 + 36 * MIN, Some(15)));
    }

    #[test]
    fn activity_resets_both_clocks() {
        let (t0, w0) = start();
        let tracker = IdleTracker::new(t0, w0);
        // a long sleep, then the user moves the mouse
        let (now, wall) = (t0 + Duration::from_secs(10), w0 + 8 * 60 * MIN);
        assert!(tracker.is_due(now, wall, Some(15)));
        tracker.note(now, wall);
        assert!(!tracker.is_due(now, wall, Some(15)));
        assert!(!tracker.is_due(now + 14 * MIN, wall + 14 * MIN, Some(15)));
        assert!(tracker.is_due(now + 15 * MIN, wall + 15 * MIN, Some(15)));
    }

    #[test]
    fn a_panicking_tick_does_not_end_the_idle_loop() {
        // the first tick panics: the guard turns it into None instead of unwinding the thread
        let hushed = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let first = guarded::<VaultStatus>(|| panic!("tick failed"));
        std::panic::set_hook(hushed);
        assert!(first.is_none());
        // the next tick runs and locks as usual
        let dir = tempfile::tempdir().unwrap();
        let st = AppState::open(Some(dir.path().to_path_buf()));
        vault_commands::vault_create(&st, secrecy::SecretString::from("a master password"), false)
            .unwrap();
        *st.vault_auto_lock_minutes.lock().unwrap() = Some(5);
        let (t0, w0) = start();
        st.idle.note(t0, w0);
        let status = run_tick(&st, t0 + 5 * MIN, w0 + 5 * MIN).expect("locks");
        assert_eq!(status.state, "locked");
    }

    fn poison_vault(st: &AppState) {
        std::thread::scope(|s| {
            let _ = s
                .spawn(|| {
                    let _guard = st.vault.lock().unwrap();
                    panic!("poison the vault mutex");
                })
                .join();
        });
        assert!(st.vault.is_poisoned());
    }

    #[test]
    fn a_poisoned_vault_mutex_does_not_disable_the_auto_lock() {
        let dir = tempfile::tempdir().unwrap();
        let st = AppState::open(Some(dir.path().to_path_buf()));
        vault_commands::vault_create(&st, secrecy::SecretString::from("a master password"), false)
            .unwrap();
        *st.vault_auto_lock_minutes.lock().unwrap() = Some(5);
        let (t0, w0) = start();
        st.idle.note(t0, w0);
        let hushed = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        poison_vault(&st);
        std::panic::set_hook(hushed);
        assert!(vault_is_unlocked(&st));
        let status = run_tick(&st, t0 + 5 * MIN, w0 + 5 * MIN).expect("locks");
        assert_eq!(status.state, "locked");
        assert!(!vault_is_unlocked(&st));
    }

    #[test]
    fn vault_lock_on_a_poisoned_mutex_locks_instead_of_failing() {
        let dir = tempfile::tempdir().unwrap();
        let st = AppState::open(Some(dir.path().to_path_buf()));
        vault_commands::vault_create(&st, secrecy::SecretString::from("a master password"), false)
            .unwrap();
        let hushed = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        poison_vault(&st);
        std::panic::set_hook(hushed);
        let status = vault_commands::vault_lock(&st).expect("locks");
        assert_eq!(status.state, "locked");
        // the other commands keep failing with internal on poison
        assert_eq!(
            vault_commands::vault_status(&st).unwrap_err().code,
            "internal"
        );
    }

    #[test]
    fn tick_locks_an_unlocked_vault_only_when_due() {
        let dir = tempfile::tempdir().unwrap();
        let st = AppState::open(Some(dir.path().to_path_buf()));
        vault_commands::vault_create(&st, secrecy::SecretString::from("a master password"), false)
            .unwrap();
        assert!(vault_is_unlocked(&st));
        let (t0, w0) = start();
        st.idle.note(t0, w0);
        // off: nothing happens
        assert!(tick(&st, t0 + 500 * MIN, w0 + 500 * MIN).is_none());
        *st.vault_auto_lock_minutes.lock().unwrap() = Some(5);
        assert!(tick(&st, t0 + 4 * MIN, w0 + 4 * MIN).is_none());
        assert!(vault_is_unlocked(&st));
        let status = tick(&st, t0 + 5 * MIN, w0 + 5 * MIN).expect("locks");
        assert_eq!(status.state, "locked");
        assert!(!vault_is_unlocked(&st));
        // already locked: nothing more to do
        assert!(tick(&st, t0 + 6 * MIN, w0 + 6 * MIN).is_none());
    }
}
