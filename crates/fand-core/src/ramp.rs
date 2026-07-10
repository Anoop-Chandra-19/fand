//! Hysteresis + ramp state machine: turns a raw curve output into the PWM
//! actually written each tick.
//!
//! - Hysteresis: ignore target changes smaller than the deadband.
//! - Ramp: asymmetric — fast up (heat is urgent), slow down (quiet decay).
//! - Floor: a fan never goes below min_pwm; fans never stop.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RampConfig {
    pub min_pwm: u8,
    pub max_step_up: u8,
    pub max_step_down: u8,
    pub deadband: u8,
}

/// Add a signed per-channel offset to a curve output, clamped to 0..=255.
/// This runs *before* the ramp's min_pwm floor, so the floor still wins —
/// a negative offset can never push a fan below its minimum duty.
pub fn apply_offset(raw: u8, offset: i16) -> u8 {
    (i32::from(raw) + i32::from(offset)).clamp(0, 255) as u8
}

#[derive(Debug, Clone)]
pub struct Ramp {
    cfg: RampConfig,
    current: u8,
}

impl Ramp {
    pub fn new(cfg: RampConfig, initial_pwm: u8) -> Self {
        Self {
            cfg,
            current: initial_pwm,
        }
    }

    pub fn current(&self) -> u8 {
        self.current
    }

    /// Advance one tick toward `raw_target` (the curve/mix output) and
    /// return the PWM to write.
    pub fn step(&mut self, raw_target: u8) -> u8 {
        let cfg = self.cfg;
        let desired = raw_target.max(cfg.min_pwm);

        // Hysteresis: don't chase changes smaller than the deadband — but
        // never let it hold a value below the floor (the initial pwm comes
        // from firmware and may sit just under it).
        if self.current >= cfg.min_pwm && self.current.abs_diff(desired) < cfg.deadband {
            return self.current;
        }

        // Asymmetric ramp, never overshooting the target.
        self.current = if desired > self.current {
            self.current.saturating_add(cfg.max_step_up).min(desired)
        } else {
            self.current.saturating_sub(cfg.max_step_down).max(desired)
        }
        .max(cfg.min_pwm);
        self.current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> RampConfig {
        RampConfig {
            min_pwm: 60,
            max_step_up: 10,
            max_step_down: 3,
            deadband: 3,
        }
    }

    #[test]
    fn ramps_up_by_max_step_up() {
        let mut r = Ramp::new(cfg(), 100);
        assert_eq!(r.step(200), 110);
        assert_eq!(r.step(200), 120);
    }

    #[test]
    fn ramps_down_by_max_step_down() {
        let mut r = Ramp::new(cfg(), 200);
        assert_eq!(r.step(100), 197);
        assert_eq!(r.step(100), 194);
    }

    #[test]
    fn never_overshoots_target() {
        // Up: 5 away with step 10 lands exactly on target.
        let mut r = Ramp::new(cfg(), 100);
        assert_eq!(r.step(105), 105);
        // Down: 3 away with step 3 lands exactly on target.
        let mut r = Ramp::new(cfg(), 105);
        assert_eq!(r.step(102), 102);
    }

    #[test]
    fn deadband_holds_small_changes() {
        let mut r = Ramp::new(cfg(), 100);
        assert_eq!(r.step(102), 100);
        assert_eq!(r.step(98), 100);
        // At exactly the deadband it moves.
        assert_eq!(r.step(103), 103);
    }

    #[test]
    fn reaches_target_over_many_ticks() {
        let mut r = Ramp::new(cfg(), 80);
        let mut last = 0;
        for _ in 0..30 {
            last = r.step(255);
        }
        assert_eq!(last, 255);
    }

    #[test]
    fn low_target_clamps_to_floor() {
        let mut r = Ramp::new(cfg(), 70);
        // Raw target 20 is below min_pwm 60 → floor, approached by ramp.
        assert_eq!(r.step(20), 67);
        for _ in 0..10 {
            r.step(20);
        }
        // Settles within the deadband of the floor (61: the last 1-unit gap
        // is deliberately ignored by hysteresis) and never below it.
        assert!(r.current() >= 60 && r.current() <= 62);
        assert!(r.step(0) >= 60);
    }

    #[test]
    fn starting_below_the_floor_recovers_to_it() {
        // Initial pwm comes from whatever firmware left behind; even if
        // that reads low, one step lands at/above the floor.
        let mut r = Ramp::new(cfg(), 0);
        assert!(r.step(70) >= 60);
    }

    #[test]
    fn deadband_cannot_hold_a_value_below_the_floor() {
        // Firmware left the fan at 58; floor 60 is within the deadband of
        // 58, but "close enough" never applies below the floor.
        let mut r = Ramp::new(cfg(), 58);
        assert_eq!(r.step(20), 60, "recovers to the floor, not held at 58");
        // At/above the floor the deadband behaves as usual again.
        assert_eq!(r.step(61), 60);
    }

    #[test]
    fn offset_shifts_and_clamps() {
        assert_eq!(apply_offset(100, 20), 120);
        assert_eq!(apply_offset(100, -30), 70);
        assert_eq!(apply_offset(250, 20), 255, "clamps at the top");
        assert_eq!(
            apply_offset(10, -50),
            0,
            "clamps at the bottom, not the floor"
        );
    }

    #[test]
    fn saturates_at_255() {
        let mut r = Ramp::new(
            RampConfig {
                max_step_up: 200,
                ..cfg()
            },
            250,
        );
        assert_eq!(r.step(255), 255);
        assert_eq!(r.step(255), 255);
    }
}
