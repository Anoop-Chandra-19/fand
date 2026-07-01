//! fand-core — pure control logic. **No I/O in this crate.**
//!
//! Everything here is deterministic functions and state machines so it can be
//! heavily unit-tested without hardware:
//! - config types + validation (reject unsorted curve points, pwm out of 0–255,
//!   unknown sensor/curve refs)
//! - curve evaluation (linear interpolation, clamped at endpoints)
//! - mix mode (max-of-outputs: evaluate each curve at its own sensor's temp,
//!   take the max PWM — NOT one curve fed max-temp)
//! - smoothing (rolling average per channel window)
//! - hysteresis (deadband, default 3 PWM units) and ramping (max_step per tick)

pub mod config;
pub mod curve;
pub mod mix;
pub mod ramp;
pub mod smoothing;
