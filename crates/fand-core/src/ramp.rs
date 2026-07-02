//! Hysteresis + ramp state machine: turns a raw curve output into the PWM
//! actually written each tick.
//!
//! - Hysteresis: ignore target changes smaller than the deadband.
//! - Ramp: asymmetric — fast up (heat is urgent), slow down (quiet decay).
//! - Floor: a running fan never goes below min_pwm.
//! - Zero-RPM (opt-in, `kick: Some(..)`): a raw target below min_pwm means
//!   *off* rather than *floor*. Leaving 0 writes the kick duty for a few
//!   ticks so the fan reliably spins up before settling to the curve value.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kick {
    pub pwm: u8,
    pub ticks: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RampConfig {
    pub min_pwm: u8,
    pub max_step_up: u8,
    pub max_step_down: u8,
    pub deadband: u8,
    /// Some(..) enables zero-RPM mode for this channel.
    pub kick: Option<Kick>,
}

#[derive(Debug, Clone)]
pub struct Ramp {
    cfg: RampConfig,
    current: u8,
    kick_ticks_left: u32,
}

impl Ramp {
    pub fn new(cfg: RampConfig, initial_pwm: u8) -> Self {
        Self {
            cfg,
            current: initial_pwm,
            kick_ticks_left: 0,
        }
    }

    pub fn current(&self) -> u8 {
        self.current
    }

    /// Advance one tick toward `raw_target` (the curve/mix output) and
    /// return the PWM to write.
    pub fn step(&mut self, raw_target: u8) -> u8 {
        let cfg = self.cfg;

        // Below the floor means "off" with zero-RPM, "floor" without.
        let desired = if raw_target < cfg.min_pwm {
            if cfg.kick.is_some() {
                0
            } else {
                cfg.min_pwm
            }
        } else {
            raw_target
        };

        if desired == 0 {
            // Ramp down to the floor, then cut to 0 — never crawl through
            // the stall region below min_pwm.
            self.kick_ticks_left = 0;
            self.current = if self.current <= cfg.min_pwm {
                0
            } else {
                self.current
                    .saturating_sub(cfg.max_step_down)
                    .max(cfg.min_pwm)
            };
            return self.current;
        }

        // Leaving 0: burst at kick duty so the fan actually starts.
        if self.current == 0 {
            let kick = cfg.kick.expect("current can only be 0 in zero-RPM mode");
            self.current = kick.pwm.max(cfg.min_pwm);
            self.kick_ticks_left = kick.ticks.saturating_sub(1);
            return self.current;
        }
        if self.kick_ticks_left > 0 {
            self.kick_ticks_left -= 1;
            return self.current;
        }

        // Hysteresis: don't chase changes smaller than the deadband.
        if self.current.abs_diff(desired) < cfg.deadband {
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
            kick: None,
        }
    }

    fn zero_rpm_cfg() -> RampConfig {
        RampConfig {
            kick: Some(Kick { pwm: 100, ticks: 3 }),
            ..cfg()
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
    fn without_zero_rpm_low_target_clamps_to_floor() {
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
    fn zero_rpm_descends_to_floor_then_cuts_to_zero() {
        let mut r = Ramp::new(zero_rpm_cfg(), 66);
        assert_eq!(r.step(0), 63);
        assert_eq!(r.step(0), 60);
        // At the floor: cut straight to 0, no crawling through stall range.
        assert_eq!(r.step(0), 0);
        assert_eq!(r.step(0), 0);
    }

    #[test]
    fn leaving_zero_kicks_then_settles() {
        let mut r = Ramp::new(zero_rpm_cfg(), 0);
        // Kick holds 100 for 3 ticks regardless of the (lower) target...
        assert_eq!(r.step(70), 100);
        assert_eq!(r.step(70), 100);
        assert_eq!(r.step(70), 100);
        // ...then normal ramping takes over toward the curve value.
        assert_eq!(r.step(70), 97);
    }

    #[test]
    fn kick_duty_respects_floor() {
        let mut config = zero_rpm_cfg();
        config.kick = Some(Kick { pwm: 40, ticks: 2 });
        let mut r = Ramp::new(config, 0);
        // A kick configured below the floor is raised to it.
        assert_eq!(r.step(70), 60);
    }

    #[test]
    fn target_dropping_to_zero_cancels_kick() {
        let mut r = Ramp::new(zero_rpm_cfg(), 0);
        assert_eq!(r.step(70), 100);
        // Temp fell again mid-kick: head back down instead of finishing it.
        assert_eq!(r.step(0), 97);
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
