//! Ack-based flow control for terminal output.
//!
//! The forwarder counts the bytes it sends to the webview (`on_sent`); the UI reports the bytes
//! xterm.js has finished writing (`on_ack`). Sending stops once more than `high` bytes are
//! unacknowledged and resumes when acknowledgements bring the count to `low` or below
//! (hysteresis, so a slow consumer does not make the forwarder flap per byte).

use std::sync::atomic::{AtomicUsize, Ordering};

/// Unacknowledged bytes above which the forwarder stops reading from the SSH channel.
pub const HIGH_WATER: usize = 128 * 1024;
/// Unacknowledged bytes at or below which it resumes.
pub const LOW_WATER: usize = 32 * 1024;

/// Most bytes of input (keystrokes, pasted text, RDP events, clipboard text) that may wait in one
/// session's queue towards the shell. Above it the call is refused and the session stays alive.
pub const MAX_QUEUED_INPUT: usize = 8 * 1024 * 1024;

/// Counts the input bytes queued for a session's writer task. The caller reserves before it
/// queues, the writer releases after it handled the item (or dropped it), so the count falls back
/// to zero on every path. Shared by the SSH and the RDP input paths.
#[derive(Debug, Default)]
pub struct InputBudget {
    queued: AtomicUsize,
}

impl InputBudget {
    /// Takes `bytes` from the budget; `false` (and nothing taken) when that would exceed
    /// [`MAX_QUEUED_INPUT`].
    pub fn try_reserve(&self, bytes: usize) -> bool {
        let mut current = self.queued.load(Ordering::Acquire);
        loop {
            let Some(next) = current
                .checked_add(bytes)
                .filter(|total| *total <= MAX_QUEUED_INPUT)
            else {
                return false;
            };
            match self.queued.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return true,
                Err(actual) => current = actual,
            }
        }
    }

    /// Gives `bytes` back; saturates at zero.
    pub fn release(&self, bytes: usize) {
        let mut current = self.queued.load(Ordering::Acquire);
        while let Err(actual) = self.queued.compare_exchange_weak(
            current,
            current.saturating_sub(bytes),
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            current = actual;
        }
    }

    #[cfg(test)]
    pub fn queued(&self) -> usize {
        self.queued.load(Ordering::Acquire)
    }
}

#[derive(Debug, Clone)]
pub struct FlowControl {
    high: usize,
    low: usize,
    unacked: usize,
    paused: bool,
}

impl FlowControl {
    pub fn new(high: usize, low: usize) -> Self {
        Self {
            high,
            low: low.min(high),
            unacked: 0,
            paused: false,
        }
    }

    pub fn on_sent(&mut self, n: usize) {
        self.unacked = self.unacked.saturating_add(n);
        if self.unacked > self.high {
            self.paused = true;
        }
    }

    /// An acknowledgement larger than what is outstanding (for example one that arrives after a
    /// reconnect reset the counter) saturates at zero.
    pub fn on_ack(&mut self, n: usize) {
        self.unacked = self.unacked.saturating_sub(n);
        if self.unacked <= self.low {
            self.paused = false;
        }
    }

    pub fn should_send(&self) -> bool {
        !self.paused
    }

    #[cfg(test)]
    pub fn unacked(&self) -> usize {
        self.unacked
    }
}

impl Default for FlowControl {
    fn default() -> Self {
        Self::new(HIGH_WATER, LOW_WATER)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stops_above_high_and_resumes_at_low() {
        let mut f = FlowControl::new(100, 20);
        assert!(f.should_send());
        f.on_sent(100);
        assert!(f.should_send(), "exactly at the high mark still sends");
        f.on_sent(1);
        assert!(!f.should_send());
        f.on_ack(50);
        assert!(!f.should_send(), "between the marks stays stopped");
        f.on_ack(30);
        assert_eq!(f.unacked(), 21);
        assert!(!f.should_send());
        f.on_ack(1);
        assert_eq!(f.unacked(), 20);
        assert!(f.should_send(), "at the low mark it resumes");
        f.on_sent(50);
        assert!(
            f.should_send(),
            "between the marks while sending keeps sending"
        );
    }

    #[test]
    fn input_budget_refuses_above_the_cap_and_recovers_after_release() {
        let b = InputBudget::default();
        assert!(b.try_reserve(MAX_QUEUED_INPUT - 10));
        assert!(!b.try_reserve(11), "over the cap is refused");
        assert_eq!(b.queued(), MAX_QUEUED_INPUT - 10, "a refusal takes nothing");
        assert!(b.try_reserve(10), "exactly the cap is allowed");
        assert!(!b.try_reserve(1));
        assert!(!b.try_reserve(usize::MAX), "overflow is refused");
        b.release(MAX_QUEUED_INPUT);
        assert_eq!(b.queued(), 0);
        assert!(b.try_reserve(MAX_QUEUED_INPUT));
        b.release(usize::MAX);
        assert_eq!(b.queued(), 0, "release saturates at zero");
    }

    #[test]
    fn ack_larger_than_unacked_saturates_at_zero() {
        let mut f = FlowControl::new(100, 20);
        f.on_sent(10);
        f.on_ack(1_000);
        assert_eq!(f.unacked(), 0);
        f.on_sent(101);
        assert!(!f.should_send());
    }
}
