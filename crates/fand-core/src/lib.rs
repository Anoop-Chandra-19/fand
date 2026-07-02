//! fand-core — pure control logic. **No I/O in this crate.**
//!
//! Everything here is deterministic functions and state machines, so it can
//! be fully unit-tested without hardware. The daemon composes these per
//! channel each tick:
//!
//! ```text
//! sensor temps → RollingAverage → Curve::eval / mix::eval_max → Ramp::step → pwm
//! ```

pub mod config;
pub mod curve;
pub mod mix;
pub mod ramp;
pub mod smoothing;

pub use config::{Config, ConfigError, ValidationError, MIN_PWM_FLOOR};
pub use curve::{Curve, CurveError};
pub use ramp::{Kick, Ramp, RampConfig};
pub use smoothing::{window_ticks, RollingAverage};
