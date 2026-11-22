//! When sync runs (M9): the triggers and the close push.
//!
//! The decisions live in [`Scheduler`], a small struct over injected time that is unit-tested;
//! the Tauri side is a thin thread. Triggers: once after the app started (the startup
//! auto-unlock already ran when `AppState` was built), five minutes after the last run, when the
//! main window gets focus (at most every ten seconds), five seconds after the last local edit
//! (`AppState.edit_seq`, bumped by the commands that change the synced data; sync's own writes
//! do not count), "Sync now" (a command, not here), and a best-effort push of at most three
//! seconds when the window closes. Nothing runs while sync is off, while a data folder change
//! waits for a restart, or while another run is in progress (`sync_commands::try_begin`: a
//! trigger that finds the slot taken is dropped, so runs never overlap). An unreachable hub is
//! the quiet `offline` state and the next trigger tries again.

use std::sync::atomic::Ordering;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};

use crate::state::AppState;
use crate::sync_commands::{build_status, run_background};

/// How long after the last local edit the "after a save" run starts.
pub const DEBOUNCE: Duration = Duration::from_secs(5);
/// How long after the last run the next one starts without any other trigger.
pub const INTERVAL: Duration = Duration::from_secs(5 * 60);
/// A window focus does not start a run when the last one began less than this ago.
pub const FOCUS_MIN_GAP: Duration = Duration::from_secs(10);
/// The most the window close waits for the final push.
pub const CLOSE_BUDGET: Duration = Duration::from_secs(3);
/// How often the runner looks at its triggers.
const TICK: Duration = Duration::from_secs(1);

/// Why a run starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    Startup,
    Edit,
    Focus,
    Interval,
}

/// The pure trigger logic. Times are offsets from one fixed start, injected by the caller.
#[derive(Debug)]
pub struct Scheduler {
    startup_due: bool,
    focus_due: bool,
    edit_seen: u64,
    edit_at: Option<Duration>,
    last_run: Option<Duration>,
}

impl Scheduler {
    /// `edit_seq` is the counter's value now: edits before the start do not trigger anything.
    #[must_use]
    pub fn new(edit_seq: u64) -> Scheduler {
        Scheduler {
            startup_due: true,
            focus_due: false,
            edit_seen: edit_seq,
            edit_at: None,
            last_run: None,
        }
    }

    /// The main window got focus.
    pub fn focus(&mut self) {
        self.focus_due = true;
    }

    /// Looks at the edit counter: a new value restarts the debounce.
    pub fn observe(&mut self, now: Duration, edit_seq: u64) {
        if edit_seq != self.edit_seen {
            self.edit_seen = edit_seq;
            self.edit_at = Some(now);
        }
    }

    /// The reason to run now, if any.
    #[must_use]
    pub fn due(&self, now: Duration) -> Option<Why> {
        let since = |t: Duration| now.saturating_sub(t);
        if self.startup_due {
            return Some(Why::Startup);
        }
        if self.edit_at.is_some_and(|at| since(at) >= DEBOUNCE) {
            return Some(Why::Edit);
        }
        if self.focus_due && self.last_run.is_none_or(|l| since(l) >= FOCUS_MIN_GAP) {
            return Some(Why::Focus);
        }
        if self.last_run.is_none_or(|l| since(l) >= INTERVAL) {
            return Some(Why::Interval);
        }
        None
    }

    /// A run starts (or the triggers are swallowed because nothing can run): the pending
    /// triggers are used up. An edit that arrives while the run is going sets a new debounce.
    pub fn begin(&mut self, now: Duration) {
        self.startup_due = false;
        self.focus_due = false;
        self.edit_at = None;
        self.last_run = Some(now);
    }

    /// The run ended: the interval counts from here.
    pub fn finished(&mut self, now: Duration) {
        self.last_run = Some(now);
    }
}

/// Wakes the runner thread early (window focus).
#[derive(Default)]
pub struct SyncWake {
    focus: Mutex<bool>,
    cv: Condvar,
}

impl SyncWake {
    /// The main window got focus.
    pub fn focus(&self) {
        *self
            .focus
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = true;
        self.cv.notify_all();
    }

    /// Waits up to `max` for a focus; returns whether one happened (and clears it).
    fn wait(&self, max: Duration) -> bool {
        let guard = self
            .focus
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut guard, _) = self
            .cv
            .wait_timeout_while(guard, max, |focused| !*focused)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::mem::take(&mut *guard)
    }
}

/// Whether a run may start: this device is set up and no data folder change waits for a restart.
fn ready(state: &AppState) -> bool {
    let paired = state
        .sync
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .paired;
    let waiting_for_restart = state.restart_required.load(Ordering::SeqCst)
        || state
            .pending_data_dir
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some();
    paired && !waiting_for_restart
}

/// Starts the runner thread. Call once, from the app's `setup`.
pub fn spawn_triggers(app: &AppHandle) {
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("fernjo-sync".to_owned())
        .spawn(move || run_loop(&app));
    // Without the thread there is no background sync; the commands still work.
    drop(spawned);
}

fn run_loop(app: &AppHandle) {
    let state = app.state::<AppState>();
    crate::sync_commands::refresh_paired(&state);
    let epoch = Instant::now();
    let mut scheduler = Scheduler::new(state.edit_seq.load(Ordering::SeqCst));
    loop {
        if state.sync_wake.wait(TICK) {
            scheduler.focus();
        }
        let now = epoch.elapsed();
        scheduler.observe(now, state.edit_seq.load(Ordering::SeqCst));
        if scheduler.due(now).is_none() {
            continue;
        }
        scheduler.begin(now);
        if !ready(&state) {
            continue;
        }
        if run_background(&state) {
            let _ = app.emit("sync-status", build_status(&state));
        }
        scheduler.finished(epoch.elapsed());
    }
}

/// Whether a close request must be held back for the push (the caller then calls
/// `destroy_after_push`); `false` means close at once.
#[must_use]
pub fn hold_close(state: &AppState) -> bool {
    // a push that is already under way closes the window itself
    state.sync_closing.load(Ordering::SeqCst) || ready(state)
}

/// Pushes what is waiting (at most `CLOSE_BUDGET`) and then destroys the window. Never blocks
/// the caller: the work runs on its own threads. The run itself is the normal one on a third
/// thread, so a slow hub costs the budget and nothing more; the process ending cuts it off, and
/// the engine is built for that (an interrupted run is repaired by the next one). Without sync
/// (or with a restart pending) the window is destroyed at once.
pub fn destroy_after_push(window: tauri::Window) {
    let app = window.app_handle().clone();
    let state = app.state::<AppState>();
    if state.sync_closing.load(Ordering::SeqCst) {
        // a push is already under way; its thread closes the window
        return;
    }
    if !ready(&state) {
        let _ = window.destroy();
        return;
    }
    state.sync_closing.store(true, Ordering::SeqCst);
    let fallback = window.clone();
    let spawned = std::thread::Builder::new()
        .name("fernjo-sync-close".to_owned())
        .spawn(move || {
            let (tx, rx) = std::sync::mpsc::channel();
            let worker = app.clone();
            let run = std::thread::Builder::new()
                .name("fernjo-sync-push".to_owned())
                .spawn(move || {
                    let state = worker.state::<AppState>();
                    // only when something waits: a close with nothing to push stays instant
                    if crate::sync_commands::has_pending_changes(&state) {
                        run_background(&state);
                    }
                    let _ = tx.send(());
                });
            if run.is_ok() {
                let _ = rx.recv_timeout(CLOSE_BUDGET);
            }
            let _ = window.destroy();
        });
    if spawned.is_err() {
        // no thread: close at once rather than never
        let _ = fallback.destroy();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: fn(u64) -> Duration = Duration::from_secs;

    fn started() -> Scheduler {
        let mut s = Scheduler::new(0);
        // the startup run happened at second 1
        assert_eq!(s.due(S(1)), Some(Why::Startup));
        s.begin(S(1));
        s.finished(S(2));
        s
    }

    #[test]
    fn the_first_look_after_start_runs_once() {
        let mut s = Scheduler::new(7);
        assert_eq!(s.due(S(0)), Some(Why::Startup));
        s.begin(S(0));
        s.finished(S(1));
        assert_eq!(s.due(S(2)), None);
    }

    #[test]
    fn the_interval_counts_from_the_end_of_the_last_run() {
        let s = started();
        assert_eq!(s.due(S(2) + INTERVAL - S(1)), None);
        assert_eq!(s.due(S(2) + INTERVAL), Some(Why::Interval));
    }

    #[test]
    fn a_local_edit_runs_five_seconds_after_the_last_edit_not_the_first() {
        let mut s = started();
        s.observe(S(100), 1);
        assert_eq!(s.due(S(104)), None);
        // a second edit restarts the wait
        s.observe(S(104), 2);
        assert_eq!(s.due(S(108)), None);
        assert_eq!(s.due(S(109)), Some(Why::Edit));
        s.begin(S(109));
        s.finished(S(110));
        assert_eq!(s.due(S(120)), None, "the edit was used up");
    }

    #[test]
    fn an_edit_during_a_run_triggers_the_next_one() {
        let mut s = started();
        s.observe(S(100), 1);
        s.begin(S(105));
        // the run is going; the user edits again
        s.observe(S(106), 2);
        s.finished(S(107));
        assert_eq!(s.due(S(110)), None);
        assert_eq!(s.due(S(111)), Some(Why::Edit));
    }

    #[test]
    fn edits_before_the_start_do_not_trigger() {
        let mut s = Scheduler::new(5);
        s.begin(S(0));
        s.finished(S(0));
        s.observe(S(1), 5);
        assert_eq!(s.due(S(20)), None);
    }

    #[test]
    fn a_focus_runs_unless_a_run_just_began() {
        let mut s = started();
        s.focus();
        assert_eq!(s.due(S(3)), None, "the last run began a second ago");
        assert_eq!(s.due(S(2) + FOCUS_MIN_GAP), Some(Why::Focus));
        s.begin(S(20));
        s.finished(S(21));
        assert_eq!(s.due(S(40)), None, "the focus was used up");
        s.focus();
        assert_eq!(s.due(S(40)), Some(Why::Focus));
    }

    #[test]
    fn swallowed_triggers_do_not_pile_up() {
        // sync off: the runner calls `begin` without running
        let mut s = Scheduler::new(0);
        s.observe(S(1), 1);
        s.focus();
        s.begin(S(10));
        assert_eq!(s.due(S(11)), None);
    }

    #[test]
    fn the_wake_reports_a_focus_once() {
        let wake = SyncWake::default();
        assert!(!wake.wait(Duration::from_millis(1)));
        wake.focus();
        assert!(wake.wait(Duration::from_millis(1)));
        assert!(!wake.wait(Duration::from_millis(1)));
    }
}
