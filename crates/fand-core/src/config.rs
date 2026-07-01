//! Config types (serde/TOML) + full validation before applying.
//!
//! See config/fand.example.toml for the target shape: [daemon], [sensors.*]
//! (hwmon by name / nvml by device index), [curves.*] (sorted (temp_c, pwm)
//! points), [channels.pwmN] (policy = "single" | "mix", min_pwm, smoothing,
//! opt-in zero_rpm with kick_pwm + kick_seconds).
