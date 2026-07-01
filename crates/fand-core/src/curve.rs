//! Curve evaluation: sorted (temp_c, pwm) points, linear interpolation
//! between points, clamped at both ends.
//!
//! Tests to write: interpolation endpoints, mid-segment interpolation,
//! unsorted-point rejection (validation lives in config, eval assumes sorted).
