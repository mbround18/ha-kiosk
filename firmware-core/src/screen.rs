//! Pure decision logic for the kiosk's dim/screensaver/wake state machine. `net::kiosk::run`
//! drives this against real hardware (touch polling, backlight, drawing) and can't itself be
//! unit tested (embedded target, `harness = false`, needs a real display/network) - but the
//! actual transition rules don't need any of that, so they live here where a plain host
//! `cargo test` can cover them directly. This is exactly the kind of logic that's easy to get a
//! subtle edge case wrong in when it's buried in one big imperative loop - e.g. the very rule
//! this was extracted to pin down: a tap that wakes a dimmed screen must NOT also be treated as
//! a room toggle (there's no room grid on screen to tap while dimmed, and stale button
//! rectangles from before dimming could otherwise be hit by coincidence).

use std::time::{Duration, Instant};

/// What the main loop should do this tick, decided from touch input and idle time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TickDecision {
    /// Whether the screen should be dimmed (screensaver mode) after this tick.
    pub dimmed: bool,
    /// True exactly on the tick a tap transitions the screen from dimmed to awake. The caller
    /// must consume this tap purely as a wake-up, not also run room hit-testing against it.
    pub woke_this_tick: bool,
}

/// `tap_present`: whether a touch was detected this tick. `last_activity`/`now`/
/// `idle_dim_timeout`: for the idle-timeout comparison, only consulted when not already dimmed
/// and no tap landed this tick.
pub fn decide_tick(
    dimmed: bool,
    tap_present: bool,
    now: Instant,
    last_activity: Instant,
    idle_dim_timeout: Duration,
) -> TickDecision {
    if tap_present {
        return TickDecision { dimmed: false, woke_this_tick: dimmed };
    }
    if !dimmed && now.duration_since(last_activity) >= idle_dim_timeout {
        return TickDecision { dimmed: true, woke_this_tick: false };
    }
    TickDecision { dimmed, woke_this_tick: false }
}

/// Whether `consecutive_failures` failed dashboard-data refresh cycles in a row should flag the
/// on-screen room/weather/climate data as possibly outdated - see `net::kiosk`'s
/// `STALE_AFTER_FAILURES`.
pub fn is_stale(consecutive_failures: u32, stale_after_failures: u32) -> bool {
    consecutive_failures >= stale_after_failures
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

    #[test]
    fn stays_awake_before_idle_timeout_elapses() {
        let last_activity = Instant::now();
        let now = last_activity + Duration::from_secs(59);
        let decision = decide_tick(false, false, now, last_activity, IDLE_TIMEOUT);
        assert_eq!(decision, TickDecision { dimmed: false, woke_this_tick: false });
    }

    #[test]
    fn dims_once_idle_timeout_elapses_with_no_tap() {
        let last_activity = Instant::now();
        let now = last_activity + Duration::from_secs(60);
        let decision = decide_tick(false, false, now, last_activity, IDLE_TIMEOUT);
        assert_eq!(decision, TickDecision { dimmed: true, woke_this_tick: false });
    }

    #[test]
    fn a_tap_while_awake_and_within_timeout_changes_nothing() {
        let last_activity = Instant::now();
        let now = last_activity + Duration::from_secs(5);
        let decision = decide_tick(false, true, now, last_activity, IDLE_TIMEOUT);
        assert_eq!(decision, TickDecision { dimmed: false, woke_this_tick: false });
    }

    #[test]
    fn a_tap_while_dimmed_wakes_and_flags_woke_this_tick() {
        let last_activity = Instant::now();
        let now = last_activity + Duration::from_secs(120);
        let decision = decide_tick(true, true, now, last_activity, IDLE_TIMEOUT);
        assert_eq!(decision, TickDecision { dimmed: false, woke_this_tick: true });
    }

    #[test]
    fn stays_dimmed_with_no_tap_and_does_not_re_flag_woke() {
        let last_activity = Instant::now();
        let now = last_activity + Duration::from_secs(200);
        let decision = decide_tick(true, false, now, last_activity, IDLE_TIMEOUT);
        assert_eq!(decision, TickDecision { dimmed: true, woke_this_tick: false });
    }

    #[test]
    fn idle_timeout_is_exclusive_of_the_moment_it_elapses_being_reevaluated_next_tick() {
        // One tick before the threshold: still awake.
        let last_activity = Instant::now();
        let just_before = last_activity + Duration::from_millis(59_950);
        assert_eq!(
            decide_tick(false, false, just_before, last_activity, IDLE_TIMEOUT),
            TickDecision { dimmed: false, woke_this_tick: false }
        );
    }

    #[test]
    fn not_yet_stale_below_threshold() {
        assert!(!is_stale(2, 3));
    }

    #[test]
    fn stale_once_threshold_reached() {
        assert!(is_stale(3, 3));
    }

    #[test]
    fn stale_stays_true_past_threshold() {
        assert!(is_stale(10, 3));
    }

    #[test]
    fn a_single_failure_does_not_flag_stale() {
        assert!(!is_stale(1, 3));
    }
}
