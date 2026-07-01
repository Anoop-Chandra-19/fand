//! Hysteresis + ramp state machine.
//!
//! - Hysteresis: only move if |target − current| ≥ deadband (default 3 PWM
//!   units) or the temp crossed a curve point.
//! - Ramp: step current toward target by max_step per tick, no instant jumps.
//! - Clamp to [channel.min_pwm, 255].
//! - Zero-RPM (opt-in): when leaving 0, write kick_pwm (~100) for kick_seconds
//!   before settling to the curve value.
