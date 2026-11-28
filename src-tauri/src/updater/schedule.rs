//! When automatic update checks run (M13): once shortly after the start and then every 24 hours.
//!
//! The decisions live in [`Scheduler`], a small struct over injected time (offsets from one fixed
//! start, and the wall clock) that is unit-tested; the thread in [`super::spawn`] is a thin shell
//! around it. The monotonic clock stands still while macOS and Linux sleep, so a day by the wall
//! clock counts too. A manual check (the command) never tells the scheduler, so it does not move
//! the 24 hour clock.

use std::cell::Cell;
use std::time::{Duration, SystemTime};

use crate::state::AppState;
use fernjo_core::DataDirSource;

/// The first automatic check comes this long after the start (or after checks were switched on).
pub const FIRST_CHECK_DELAY: Duration = Duration::from_secs(60);
/// The next automatic check comes this long after the last automatic one.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// The pure timing logic.
#[derive(Debug)]
pub struct Scheduler {
    enabled: bool,
    /// When the scheduler saw itself enabled (the first `due` call after `set_enabled(true)`).
    anchor: Cell<Option<Duration>>,
    last_ran: Option<(Duration, SystemTime)>,
}

impl Scheduler {
    #[must_use]
    pub fn new(enabled: bool) -> Scheduler {
        Scheduler {
            enabled,
            anchor: Cell::new(None),
            last_ran: None,
        }
    }

    /// Switching on starts the first-check delay again, so enabling later never causes a burst.
    pub fn set_enabled(&mut self, on: bool) {
        if on != self.enabled {
            self.anchor.set(None);
        }
        self.enabled = on;
    }

    /// Whether an automatic check should start now (`now` from the fixed start, `wall` the time of
    /// day).
    #[must_use]
    pub fn due(&self, now: Duration, wall: SystemTime) -> bool {
        if !self.enabled {
            return false;
        }
        let anchor = match self.anchor.get() {
            Some(a) => a,
            None => {
                self.anchor.set(Some(now));
                now
            }
        };
        if now < anchor + FIRST_CHECK_DELAY {
            return false;
        }
        self.last_ran.is_none_or(|(ran, ran_at)| {
            // a wall clock set back leaves the monotonic clock to decide
            now >= ran + CHECK_INTERVAL
                || wall
                    .duration_since(ran_at)
                    .is_ok_and(|d| d >= CHECK_INTERVAL)
        })
    }

    /// An automatic check ran (successful or not): the interval counts from here.
    pub fn ran(&mut self, now: Duration, wall: SystemTime) {
        self.last_ran = Some((now, wall));
    }
}

/// Whether automatic checks may run at all: not under `FERNJO_DATA_DIR` (a scratch run must not
/// contact the feed by itself), not with an unusable `settings.json`, and not when switched off.
#[must_use]
pub fn automatic_checks_allowed(state: &AppState) -> bool {
    if state.source == DataDirSource::Env || state.settings_unusable {
        return false;
    }
    state
        .check_for_updates
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: fn(u64) -> Duration = Duration::from_secs;

    /// The wall clock `offset` after the start, as it moves while the machine is awake.
    fn wall(offset: Duration) -> SystemTime {
        SystemTime::UNIX_EPOCH + S(1_000_000) + offset
    }

    fn state(source: DataDirSource, unusable: bool, on: bool) -> AppState {
        let dir = tempfile::tempdir().unwrap();
        let mut state = AppState::open(Some(dir.path().to_path_buf()));
        state.source = source;
        state.settings_unusable = unusable;
        *state.check_for_updates.lock().unwrap() = on;
        state
    }

    #[test]
    fn first_check_after_60_seconds_not_before() {
        let s = Scheduler::new(true);
        assert!(!s.due(S(0), wall(S(0))));
        assert!(!s.due(S(59), wall(S(59))));
        assert!(s.due(S(60), wall(S(60))));
    }

    #[test]
    fn next_check_24_hours_after_the_last() {
        let mut s = Scheduler::new(true);
        assert!(!s.due(S(0), wall(S(0))));
        assert!(s.due(S(61), wall(S(61))));
        s.ran(S(61), wall(S(61)));
        assert!(!s.due(S(62), wall(S(62))));
        assert!(!s.due(
            S(61) + CHECK_INTERVAL - S(1),
            wall(S(61) + CHECK_INTERVAL - S(1))
        ));
        assert!(s.due(S(61) + CHECK_INTERVAL, wall(S(61) + CHECK_INTERVAL)));
    }

    #[test]
    fn a_day_asleep_counts_towards_the_next_check() {
        let mut s = Scheduler::new(true);
        assert!(!s.due(S(0), wall(S(0))));
        assert!(s.due(S(60), wall(S(60))));
        s.ran(S(60), wall(S(60)));
        // not early by either clock
        assert!(!s.due(S(120), wall(CHECK_INTERVAL)));
        // two minutes awake, a day by the wall clock: the machine slept
        assert!(s.due(S(180), wall(S(60) + CHECK_INTERVAL)));
        // a wall clock set back does not postpone the check
        assert!(s.due(S(60) + CHECK_INTERVAL, wall(S(0))));
    }

    #[test]
    fn disabled_scheduler_never_fires() {
        let s = Scheduler::new(false);
        assert!(!s.due(S(0), wall(S(0))));
        assert!(!s.due(CHECK_INTERVAL * 3, wall(CHECK_INTERVAL * 3)));
    }

    #[test]
    fn enabling_later_counts_from_now_plus_first_delay() {
        let mut s = Scheduler::new(false);
        assert!(!s.due(S(100), wall(S(100))));
        s.set_enabled(true);
        assert!(
            !s.due(S(5000), wall(S(5000))),
            "the enabling tick only starts the delay"
        );
        assert!(!s.due(S(5059), wall(S(5059))));
        assert!(s.due(S(5060), wall(S(5060))));
        // ran long ago, disabled for hours, enabled again: no burst
        s.ran(S(5060), wall(S(5060)));
        s.set_enabled(false);
        s.set_enabled(true);
        let t = S(5060) + CHECK_INTERVAL + S(3600);
        assert!(!s.due(t, wall(t)));
        assert!(!s.due(t + S(59), wall(t + S(59))));
        assert!(s.due(t + S(60), wall(t + S(60))));
    }

    #[test]
    fn no_automatic_check_under_fernjo_data_dir() {
        assert!(!automatic_checks_allowed(&state(
            DataDirSource::Env,
            false,
            true
        )));
        assert!(automatic_checks_allowed(&state(
            DataDirSource::Default,
            false,
            true
        )));
    }

    #[test]
    fn no_automatic_check_with_unusable_settings() {
        assert!(!automatic_checks_allowed(&state(
            DataDirSource::Default,
            true,
            true
        )));
    }

    #[test]
    fn check_for_updates_off_stops_checks() {
        assert!(!automatic_checks_allowed(&state(
            DataDirSource::Default,
            false,
            false
        )));
    }
}
