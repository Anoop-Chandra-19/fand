//! fand-core — pure control logic. **No I/O in this crate.**
//!
//! Everything here is deterministic functions and state machines, so it can
//! be fully unit-tested without hardware. The daemon composes these per
//! channel each tick:
//!
//! ```text
//! sensor temps → CurveTree::eval (smoothing + graph/mix/flat) → Ramp::step → pwm
//! ```

pub mod channel_edit;
pub mod config;
pub mod curve;
pub mod curve_edit;
pub mod eval;
pub mod ramp;
pub mod smoothing;

pub use channel_edit::{
    set_channel_curve, set_min_pwm, set_smoothing_seconds, set_zero_rpm, ChannelEditError,
};
pub use config::{Config, ConfigError, MixFunction, ValidationError, MIN_PWM_FLOOR};
pub use curve::{Curve, CurveError};
pub use curve_edit::{
    add_mix_member, create_graph_curve, remove_curve, remove_mix_member, replace_curve_points,
    set_graph_sensor, CurveEditError,
};
pub use eval::{CurveTree, EvalError, TreeError};
pub use ramp::{Kick, Ramp, RampConfig};
pub use smoothing::{window_ticks, RollingAverage};
