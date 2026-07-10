//! Trigger curves (FanControl's "trigger"): a two-state latch that steps
//! between an idle duty and a load duty across a temperature deadband.
//!
//! The gap between `idle_temp` and `load_temp` is the hysteresis — once
//! latched, the curve holds its state until the temp crosses the *far*
//! threshold, so it never oscillates on a sensor hovering in the middle. A
//! crossing must also persist `response_seconds` before the latch flips.
//!
//! First-sample rule: latch to load only when the temp is already at/above
//! `load_temp`, otherwise idle. Starting idle in the deadband matches the
//! feature's intent (quiet until genuinely hot) and is safe — triggers are
//! forbidden on the pump, the min_pwm floor keeps the fan spinning, and the
//! ≥115 °C plausibility failsafe still escalates regardless of the latch.

use std::time::{Duration, Instant};

use crate::config::TriggerCurve;

#[derive(Debug, Clone)]
pub struct TriggerLatch {
    idle_temp: f64,
    idle_pwm: u8,
    load_temp: f64,
    load_pwm: u8,
    response: Duration,
    state: Option<State>,
}

#[derive(Debug, Clone)]
struct State {
    loaded: bool,
    /// Set while the opposite threshold is currently crossed but the dwell
    /// has not yet elapsed; cleared the moment the temp retreats.
    pending_since: Option<Instant>,
}

impl TriggerLatch {
    /// `idle_pwm`/`load_pwm` are pre-clamped to u8 by the caller (validation
    /// guarantees 0..=255).
    pub fn new(cfg: &TriggerCurve, idle_pwm: u8, load_pwm: u8) -> Self {
        Self {
            idle_temp: cfg.idle_temp,
            idle_pwm,
            load_temp: cfg.load_temp,
            load_pwm,
            response: Duration::from_secs(cfg.response_seconds),
            state: None,
        }
    }

    /// Feed one (smoothed) sample; returns the latched duty.
    pub fn apply(&mut self, temp: f64, now: Instant) -> u8 {
        let state = self.state.get_or_insert(State {
            loaded: temp >= self.load_temp,
            pending_since: None,
        });

        // The condition that would flip the latch, and the state it flips
        // to: if idle, we flip to load once hot; if loaded, to idle once cool.
        let (crossed, target_loaded) = if state.loaded {
            (temp <= self.idle_temp, false)
        } else {
            (temp >= self.load_temp, true)
        };

        if crossed {
            let since = *state.pending_since.get_or_insert(now);
            if now.duration_since(since) >= self.response {
                state.loaded = target_loaded;
                state.pending_since = None;
            }
        } else {
            state.pending_since = None;
        }

        if state.loaded {
            self.load_pwm
        } else {
            self.idle_pwm
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(response: u64) -> TriggerCurve {
        TriggerCurve {
            sensor: "cpu".into(),
            idle_temp: 40.0,
            idle_pwm: 90,
            load_temp: 60.0,
            load_pwm: 200,
            response_seconds: response,
        }
    }

    fn latch(response: u64) -> TriggerLatch {
        TriggerLatch::new(&cfg(response), 90, 200)
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn first_sample_below_load_starts_idle() {
        let mut l = latch(0);
        // 50 is in the deadband → idle (fail-low, quiet until genuinely hot).
        assert_eq!(l.apply(50.0, Instant::now()), 90);
    }

    #[test]
    fn first_sample_at_load_starts_loaded() {
        let mut l = latch(0);
        assert_eq!(l.apply(65.0, Instant::now()), 200);
    }

    #[test]
    fn latches_up_at_load_temp_and_holds_across_deadband() {
        let mut l = latch(0);
        let now = Instant::now();
        assert_eq!(l.apply(35.0, now), 90, "starts idle");
        assert_eq!(l.apply(55.0, now), 90, "deadband: still idle");
        assert_eq!(l.apply(60.0, now), 200, "reaches load_temp: flips");
        assert_eq!(l.apply(45.0, now), 200, "deadband: holds load");
        assert_eq!(l.apply(40.0, now), 90, "reaches idle_temp: flips back");
    }

    #[test]
    fn crossing_must_survive_the_response_dwell() {
        let mut l = latch(5);
        let base = Instant::now();
        l.apply(35.0, base); // idle
        assert_eq!(l.apply(65.0, base), 90, "dwell started, not flipped yet");
        assert_eq!(l.apply(65.0, base + secs(2)), 90, "2s < 5s");
        assert_eq!(l.apply(65.0, base + secs(6)), 200, "6s ≥ 5s: flips");
    }

    #[test]
    fn retreat_before_dwell_elapses_resets_the_timer() {
        let mut l = latch(5);
        let base = Instant::now();
        l.apply(35.0, base); // idle
        l.apply(65.0, base); // pending since base
        assert_eq!(
            l.apply(50.0, base + secs(2)),
            90,
            "back in deadband: cancels"
        );
        assert_eq!(l.apply(65.0, base + secs(6)), 90, "timer restarted at 6s");
        assert_eq!(l.apply(65.0, base + secs(12)), 200);
    }

    #[test]
    fn zero_response_flips_immediately() {
        let mut l = latch(0);
        let now = Instant::now();
        l.apply(35.0, now);
        assert_eq!(l.apply(60.0, now), 200, "no dwell: flips same tick");
    }
}
