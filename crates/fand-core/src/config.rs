//! Config types (serde/TOML) + full validation before applying.
//!
//! See config/fand.example.toml for the shape. Validation rejects: unsorted
//! curve points, pwm out of 0–255, unknown sensor/curve references, zero_rpm
//! without kick parameters, and min_pwm below the stall floor unless zero_rpm
//! is explicitly enabled.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Fans may stall below this duty; configs must not set a lower min_pwm
/// unless the channel explicitly opts into zero_rpm (with kick parameters).
pub const MIN_PWM_FLOOR: u8 = 60;

fn default_tick() -> u64 {
    2
}
fn default_min_pwm() -> u8 {
    MIN_PWM_FLOOR
}
fn default_step_up() -> u8 {
    10
}
fn default_step_down() -> u8 {
    3
}
fn default_deadband() -> u8 {
    3
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub daemon: DaemonConfig,
    #[serde(default)]
    pub sensors: BTreeMap<String, SensorConfig>,
    #[serde(default)]
    pub curves: BTreeMap<String, CurveConfig>,
    #[serde(default)]
    pub channels: BTreeMap<String, ChannelConfig>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DaemonConfig {
    #[serde(default = "default_tick")]
    pub tick_seconds: u64,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            tick_seconds: default_tick(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SensorConfig {
    Hwmon { hwmon_name: String, input: String },
    Nvml { device_index: u32 },
}

/// Curve points as written in TOML: whole-degree temps, pwm parsed wide
/// (u16) so out-of-range values reach validation instead of a serde error.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurveConfig {
    pub points: Vec<(i32, u16)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChannelConfig {
    pub hwmon_name: String,
    #[serde(flatten)]
    pub policy: Policy,
    #[serde(default = "default_min_pwm")]
    pub min_pwm: u8,
    pub smoothing_seconds: u64,
    #[serde(default = "default_step_up")]
    pub max_step_up: u8,
    #[serde(default = "default_step_down")]
    pub max_step_down: u8,
    #[serde(default = "default_deadband")]
    pub deadband: u8,
    #[serde(default)]
    pub zero_rpm: bool,
    #[serde(default)]
    pub kick_pwm: Option<u8>,
    #[serde(default)]
    pub kick_seconds: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "policy", rename_all = "lowercase")]
pub enum Policy {
    Single { sensor: String, curve: String },
    Mix { inputs: Vec<MixInput> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MixInput {
    pub sensor: String,
    pub curve: String,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ValidationError {
    #[error("daemon.tick_seconds must be >= 1")]
    TickZero,
    #[error("curve `{0}`: needs at least 2 points")]
    CurveTooFewPoints(String),
    #[error("curve `{curve}`: temps must be strictly increasing (point {index})")]
    CurveUnsorted { curve: String, index: usize },
    #[error("curve `{curve}`: pwm {pwm} out of range 0-255 (point {index})")]
    PwmOutOfRange {
        curve: String,
        index: usize,
        pwm: u16,
    },
    #[error("channel `{channel}`: unknown sensor `{sensor}`")]
    UnknownSensor { channel: String, sensor: String },
    #[error("channel `{channel}`: unknown curve `{curve}`")]
    UnknownCurve { channel: String, curve: String },
    #[error("channel `{channel}`: mix policy needs at least one input")]
    EmptyMix { channel: String },
    #[error("channel `{channel}`: zero_rpm requires kick_pwm and kick_seconds")]
    ZeroRpmWithoutKick { channel: String },
    #[error(
        "channel `{channel}`: min_pwm {min_pwm} is below the stall floor {floor} \
         (enable zero_rpm explicitly if you want fans to stop)"
    )]
    MinPwmBelowFloor {
        channel: String,
        min_pwm: u8,
        floor: u8,
    },
    #[error("channel `{channel}`: max_step_up and max_step_down must be >= 1")]
    ZeroStep { channel: String },
    #[error("channel `{channel}`: smoothing_seconds must be >= 1")]
    ZeroSmoothing { channel: String },
    #[error("channel `{channel}`: name must be pwmN (matching the hwmon pwm file)")]
    BadChannelName { channel: String },
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("TOML parse error: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("invalid config: {}", .0.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "))]
    Invalid(Vec<ValidationError>),
}

impl Config {
    /// Parse and fully validate; the daemon must never apply a config that
    /// did not come through here (or `validate`).
    pub fn from_toml_str(s: &str) -> Result<Self, ConfigError> {
        let cfg: Config = toml::from_str(s)?;
        cfg.validate().map_err(ConfigError::Invalid)?;
        Ok(cfg)
    }

    /// Collects *all* problems rather than stopping at the first, so a
    /// client can show the user everything wrong with an edit at once.
    pub fn validate(&self) -> Result<(), Vec<ValidationError>> {
        let mut errs = Vec::new();

        if self.daemon.tick_seconds == 0 {
            errs.push(ValidationError::TickZero);
        }

        for (name, curve) in &self.curves {
            if curve.points.len() < 2 {
                errs.push(ValidationError::CurveTooFewPoints(name.clone()));
            }
            for (i, w) in curve.points.windows(2).enumerate() {
                if w[1].0 <= w[0].0 {
                    errs.push(ValidationError::CurveUnsorted {
                        curve: name.clone(),
                        index: i + 1,
                    });
                }
            }
            for (i, &(_, pwm)) in curve.points.iter().enumerate() {
                if pwm > 255 {
                    errs.push(ValidationError::PwmOutOfRange {
                        curve: name.clone(),
                        index: i,
                        pwm,
                    });
                }
            }
        }

        for (name, ch) in &self.channels {
            if !is_pwm_name(name) {
                errs.push(ValidationError::BadChannelName {
                    channel: name.clone(),
                });
            }

            let mut refs: Vec<(&str, &str)> = Vec::new();
            match &ch.policy {
                Policy::Single { sensor, curve } => refs.push((sensor, curve)),
                Policy::Mix { inputs } => {
                    if inputs.is_empty() {
                        errs.push(ValidationError::EmptyMix {
                            channel: name.clone(),
                        });
                    }
                    for input in inputs {
                        refs.push((&input.sensor, &input.curve));
                    }
                }
            }
            for (sensor, curve) in refs {
                if !self.sensors.contains_key(sensor) {
                    errs.push(ValidationError::UnknownSensor {
                        channel: name.clone(),
                        sensor: sensor.to_string(),
                    });
                }
                if !self.curves.contains_key(curve) {
                    errs.push(ValidationError::UnknownCurve {
                        channel: name.clone(),
                        curve: curve.to_string(),
                    });
                }
            }

            if ch.zero_rpm && (ch.kick_pwm.is_none() || ch.kick_seconds.is_none()) {
                errs.push(ValidationError::ZeroRpmWithoutKick {
                    channel: name.clone(),
                });
            }
            if !ch.zero_rpm && ch.min_pwm < MIN_PWM_FLOOR {
                errs.push(ValidationError::MinPwmBelowFloor {
                    channel: name.clone(),
                    min_pwm: ch.min_pwm,
                    floor: MIN_PWM_FLOOR,
                });
            }
            if ch.max_step_up == 0 || ch.max_step_down == 0 {
                errs.push(ValidationError::ZeroStep {
                    channel: name.clone(),
                });
            }
            if ch.smoothing_seconds == 0 {
                errs.push(ValidationError::ZeroSmoothing {
                    channel: name.clone(),
                });
            }
        }

        if errs.is_empty() {
            Ok(())
        } else {
            Err(errs)
        }
    }
}

fn is_pwm_name(name: &str) -> bool {
    name.strip_prefix("pwm")
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = include_str!("../../../config/fand.example.toml");

    /// A minimal valid config the rejection tests below mutate.
    fn base_toml() -> String {
        r#"
            [sensors.cpu]
            kind = "hwmon"
            hwmon_name = "k10temp"
            input = "temp1_input"

            [curves.c]
            points = [[40, 80], [70, 200]]

            [channels.pwm1]
            hwmon_name = "nct6799"
            policy = "single"
            sensor = "cpu"
            curve = "c"
            min_pwm = 70
            smoothing_seconds = 5
        "#
        .to_string()
    }

    fn errors_of(toml_str: &str) -> Vec<ValidationError> {
        match Config::from_toml_str(toml_str) {
            Err(ConfigError::Invalid(errs)) => errs,
            other => panic!("expected validation errors, got {other:?}"),
        }
    }

    #[test]
    fn example_config_is_valid() {
        let cfg = Config::from_toml_str(EXAMPLE).expect("shipped example config must validate");
        assert_eq!(cfg.daemon.tick_seconds, 2);
        assert_eq!(cfg.channels.len(), 2);
        assert!(matches!(
            cfg.channels["pwm2"].policy,
            Policy::Mix { ref inputs } if inputs.len() == 2
        ));
        assert_eq!(cfg.channels["pwm1"].min_pwm, 80);
        assert_eq!(cfg.channels["pwm1"].max_step_up, 10);
        assert_eq!(cfg.channels["pwm1"].max_step_down, 3);
    }

    #[test]
    fn defaults_applied_when_omitted() {
        let cfg = Config::from_toml_str(&base_toml()).unwrap();
        assert_eq!(cfg.daemon.tick_seconds, 2, "missing [daemon] uses default");
        let ch = &cfg.channels["pwm1"];
        assert_eq!(ch.max_step_up, 10);
        assert_eq!(ch.max_step_down, 3);
        assert_eq!(ch.deadband, 3);
        assert!(!ch.zero_rpm);
    }

    #[test]
    fn unsorted_curve_rejected() {
        let toml_str = base_toml().replace("[[40, 80], [70, 200]]", "[[70, 80], [40, 200]]");
        assert!(errors_of(&toml_str)
            .iter()
            .any(|e| matches!(e, ValidationError::CurveUnsorted { .. })));
    }

    #[test]
    fn duplicate_temp_rejected() {
        let toml_str = base_toml().replace("[[40, 80], [70, 200]]", "[[40, 80], [40, 200]]");
        assert!(errors_of(&toml_str)
            .iter()
            .any(|e| matches!(e, ValidationError::CurveUnsorted { .. })));
    }

    #[test]
    fn pwm_out_of_range_rejected() {
        let toml_str = base_toml().replace("[[40, 80], [70, 200]]", "[[40, 80], [70, 300]]");
        assert!(errors_of(&toml_str)
            .iter()
            .any(|e| matches!(e, ValidationError::PwmOutOfRange { pwm: 300, .. })));
    }

    #[test]
    fn single_point_curve_rejected() {
        let toml_str = base_toml().replace("[[40, 80], [70, 200]]", "[[40, 80]]");
        assert!(errors_of(&toml_str)
            .iter()
            .any(|e| matches!(e, ValidationError::CurveTooFewPoints(_))));
    }

    #[test]
    fn unknown_sensor_rejected() {
        let toml_str = base_toml().replace("sensor = \"cpu\"", "sensor = \"gpu\"");
        assert!(errors_of(&toml_str).iter().any(|e| matches!(
            e,
            ValidationError::UnknownSensor { sensor, .. } if sensor == "gpu"
        )));
    }

    #[test]
    fn unknown_curve_rejected() {
        let toml_str = base_toml().replace("curve = \"c\"", "curve = \"nope\"");
        assert!(errors_of(&toml_str).iter().any(|e| matches!(
            e,
            ValidationError::UnknownCurve { curve, .. } if curve == "nope"
        )));
    }

    #[test]
    fn zero_rpm_without_kick_rejected() {
        let toml_str = base_toml().replace("min_pwm = 70", "min_pwm = 70\nzero_rpm = true");
        assert!(errors_of(&toml_str)
            .iter()
            .any(|e| matches!(e, ValidationError::ZeroRpmWithoutKick { .. })));
    }

    #[test]
    fn min_pwm_below_floor_rejected_without_zero_rpm() {
        let toml_str = base_toml().replace("min_pwm = 70", "min_pwm = 30");
        assert!(errors_of(&toml_str)
            .iter()
            .any(|e| matches!(e, ValidationError::MinPwmBelowFloor { min_pwm: 30, .. })));
    }

    #[test]
    fn min_pwm_below_floor_allowed_with_zero_rpm_and_kick() {
        let toml_str = base_toml().replace(
            "min_pwm = 70",
            "min_pwm = 30\nzero_rpm = true\nkick_pwm = 100\nkick_seconds = 3",
        );
        Config::from_toml_str(&toml_str).expect("opt-in zero_rpm with kick must be accepted");
    }

    #[test]
    fn bad_channel_name_rejected() {
        let toml_str = base_toml().replace("[channels.pwm1]", "[channels.pmw1]");
        assert!(errors_of(&toml_str)
            .iter()
            .any(|e| matches!(e, ValidationError::BadChannelName { .. })));
    }

    #[test]
    fn empty_mix_rejected() {
        let toml_str = r#"
            [curves.c]
            points = [[40, 80], [70, 200]]

            [channels.pwm2]
            hwmon_name = "nct6799"
            policy = "mix"
            inputs = []
            min_pwm = 70
            smoothing_seconds = 5
        "#;
        assert!(errors_of(toml_str)
            .iter()
            .any(|e| matches!(e, ValidationError::EmptyMix { .. })));
    }

    #[test]
    fn validate_collects_multiple_errors() {
        let toml_str = base_toml()
            .replace("[[40, 80], [70, 200]]", "[[70, 80], [40, 300]]")
            .replace("sensor = \"cpu\"", "sensor = \"gpu\"");
        assert!(errors_of(&toml_str).len() >= 3);
    }
}
