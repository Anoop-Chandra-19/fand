//! Config types (serde/TOML) + full validation before applying.
//!
//! See config/fand.example.toml for the shape. FanControl-style model: a
//! curve owns its temperature source (graph) or combines other curves
//! (mix); a channel always binds exactly one curve by name.
//!
//! Validation rejects: unsorted curve points, pwm out of 0–255, unknown
//! sensor/curve references anywhere in a curve tree, mix cycles, zero_rpm
//! without kick parameters, and min_pwm below the stall floor unless
//! zero_rpm is explicitly enabled.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Fans may stall below this duty; configs must not set a lower min_pwm
/// unless the channel explicitly opts into zero_rpm (with kick parameters).
pub const MIN_PWM_FLOOR: u8 = 60;

/// Hysteresis wider than this is a config mistake, not a preference — the
/// whole useful curve range is only ~40 °C.
pub const MAX_HYSTERESIS_C: f64 = 20.0;

/// Response times longer than this would make the daemon feel broken.
pub const MAX_RESPONSE_SECONDS: u64 = 600;

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
fn default_true() -> bool {
    true
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

/// A curve is graph (points evaluated at its own sensor), mix (combines
/// other curves' *outputs* — never their temperatures), or flat (constant).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum CurveConfig {
    Graph(GraphCurve),
    Mix(MixCurve),
    Flat(FlatCurve),
}

/// Points as written in TOML: whole-degree temps, pwm parsed wide (u16) so
/// out-of-range values reach validation instead of a serde error.
///
/// The hysteresis fields are parsed and validated but **inert until phase
/// 8a** — the engine does not gate on them yet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphCurve {
    pub sensor: String,
    pub points: Vec<(i32, u16)>,
    /// °C the input must rise from the last accepted value (0 = off).
    #[serde(default)]
    pub hysteresis_up: f64,
    /// °C the input must fall from the last accepted value (0 = off).
    #[serde(default)]
    pub hysteresis_down: f64,
    /// Seconds a change must persist before it is accepted (0 = off).
    #[serde(default)]
    pub response_seconds: u64,
    /// Bypass hysteresis at the curve's endpoints so the fan still reaches
    /// full speed promptly and settles all the way back to idle.
    #[serde(default = "default_true")]
    pub ignore_hysteresis_at_extremes: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MixCurve {
    /// How member outputs combine. Max is the safety-documented default:
    /// whichever component demands the most cooling wins.
    #[serde(default)]
    pub function: MixFunction,
    pub curves: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MixFunction {
    #[default]
    Max,
    Min,
    Average,
}

impl MixFunction {
    pub fn as_str(self) -> &'static str {
        match self {
            MixFunction::Max => "max",
            MixFunction::Min => "min",
            MixFunction::Average => "average",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlatCurve {
    /// Wide (u16) for the same reason as graph points.
    pub pwm: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChannelConfig {
    pub hwmon_name: String,
    /// The one curve driving this channel (mixing is a curve's job).
    pub curve: String,
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

#[derive(Debug, Error, PartialEq)]
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
    #[error("curve `{curve}`: unknown sensor `{sensor}`")]
    CurveUnknownSensor { curve: String, sensor: String },
    #[error("curve `{curve}`: {field} must be a finite value in 0-{max}")]
    BadHysteresis {
        curve: String,
        field: &'static str,
        max: u32,
    },
    #[error("curve `{curve}`: mix needs at least one member curve")]
    EmptyMix { curve: String },
    #[error("curve `{curve}`: unknown member curve `{member}`")]
    MixUnknownCurve { curve: String, member: String },
    #[error("curve `{curve}`: member `{member}` appears more than once")]
    DuplicateMixMember { curve: String, member: String },
    #[error("curve `{curve}`: mix curves must not reference themselves (directly or through other mixes)")]
    MixCycle { curve: String },
    #[error("curve `{curve}`: flat pwm {pwm} out of range 0-255")]
    FlatPwmOutOfRange { curve: String, pwm: u16 },
    #[error("channel `{channel}`: unknown curve `{curve}`")]
    UnknownCurve { channel: String, curve: String },
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
            match curve {
                CurveConfig::Graph(g) => self.validate_graph(name, g, &mut errs),
                CurveConfig::Mix(m) => self.validate_mix(name, m, &mut errs),
                CurveConfig::Flat(f) => {
                    if f.pwm > 255 {
                        errs.push(ValidationError::FlatPwmOutOfRange {
                            curve: name.clone(),
                            pwm: f.pwm,
                        });
                    }
                }
            }
        }
        self.detect_mix_cycles(&mut errs);

        for (name, ch) in &self.channels {
            if !is_pwm_name(name) {
                errs.push(ValidationError::BadChannelName {
                    channel: name.clone(),
                });
            }
            if !self.curves.contains_key(&ch.curve) {
                errs.push(ValidationError::UnknownCurve {
                    channel: name.clone(),
                    curve: ch.curve.clone(),
                });
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

    fn validate_graph(&self, name: &str, g: &GraphCurve, errs: &mut Vec<ValidationError>) {
        if g.points.len() < 2 {
            errs.push(ValidationError::CurveTooFewPoints(name.to_string()));
        }
        for (i, w) in g.points.windows(2).enumerate() {
            if w[1].0 <= w[0].0 {
                errs.push(ValidationError::CurveUnsorted {
                    curve: name.to_string(),
                    index: i + 1,
                });
            }
        }
        for (i, &(_, pwm)) in g.points.iter().enumerate() {
            if pwm > 255 {
                errs.push(ValidationError::PwmOutOfRange {
                    curve: name.to_string(),
                    index: i,
                    pwm,
                });
            }
        }
        if !self.sensors.contains_key(&g.sensor) {
            errs.push(ValidationError::CurveUnknownSensor {
                curve: name.to_string(),
                sensor: g.sensor.clone(),
            });
        }
        for (field, value) in [
            ("hysteresis_up", g.hysteresis_up),
            ("hysteresis_down", g.hysteresis_down),
        ] {
            // NaN and ±inf also fail the contains check.
            if !(0.0..=MAX_HYSTERESIS_C).contains(&value) {
                errs.push(ValidationError::BadHysteresis {
                    curve: name.to_string(),
                    field,
                    max: MAX_HYSTERESIS_C as u32,
                });
            }
        }
        if g.response_seconds > MAX_RESPONSE_SECONDS {
            errs.push(ValidationError::BadHysteresis {
                curve: name.to_string(),
                field: "response_seconds",
                max: MAX_RESPONSE_SECONDS as u32,
            });
        }
    }

    fn validate_mix(&self, name: &str, m: &MixCurve, errs: &mut Vec<ValidationError>) {
        if m.curves.is_empty() {
            errs.push(ValidationError::EmptyMix {
                curve: name.to_string(),
            });
        }
        for (i, member) in m.curves.iter().enumerate() {
            if !self.curves.contains_key(member) {
                errs.push(ValidationError::MixUnknownCurve {
                    curve: name.to_string(),
                    member: member.clone(),
                });
            }
            if m.curves[..i].contains(member) {
                errs.push(ValidationError::DuplicateMixMember {
                    curve: name.to_string(),
                    member: member.clone(),
                });
            }
        }
    }

    /// Depth-first walk over mix membership; any curve reachable from
    /// itself is reported once. Unknown members are skipped here — they get
    /// their own `MixUnknownCurve` error.
    fn detect_mix_cycles(&self, errs: &mut Vec<ValidationError>) {
        #[derive(Clone, Copy, PartialEq)]
        enum Mark {
            InStack,
            Done,
        }

        fn visit(
            curves: &BTreeMap<String, CurveConfig>,
            name: &str,
            marks: &mut BTreeMap<String, Mark>,
            cyclic: &mut Vec<String>,
        ) {
            match marks.get(name) {
                Some(Mark::Done) => return,
                Some(Mark::InStack) => {
                    if !cyclic.contains(&name.to_string()) {
                        cyclic.push(name.to_string());
                    }
                    return;
                }
                None => {}
            }
            marks.insert(name.to_string(), Mark::InStack);
            if let Some(CurveConfig::Mix(m)) = curves.get(name) {
                for member in &m.curves {
                    if curves.contains_key(member) {
                        visit(curves, member, marks, cyclic);
                    }
                }
            }
            marks.insert(name.to_string(), Mark::Done);
        }

        let mut marks = BTreeMap::new();
        let mut cyclic = Vec::new();
        for name in self.curves.keys() {
            visit(&self.curves, name, &mut marks, &mut cyclic);
        }
        for curve in cyclic {
            errs.push(ValidationError::MixCycle { curve });
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
            kind = "graph"
            sensor = "cpu"
            points = [[40, 80], [70, 200]]

            [channels.pwm1]
            hwmon_name = "nct6799"
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
        assert_eq!(cfg.channels["pwm2"].curve, "case_mix");
        assert!(matches!(
            &cfg.curves["case_mix"],
            CurveConfig::Mix(m) if m.curves.len() == 2 && m.function == MixFunction::Max
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
        let CurveConfig::Graph(g) = &cfg.curves["c"] else {
            panic!("expected graph curve");
        };
        assert_eq!(g.hysteresis_up, 0.0);
        assert_eq!(g.hysteresis_down, 0.0);
        assert_eq!(g.response_seconds, 0);
        assert!(g.ignore_hysteresis_at_extremes);
    }

    #[test]
    fn curve_without_kind_rejected_at_parse() {
        let toml_str = base_toml().replace("kind = \"graph\"\n", "");
        assert!(matches!(
            Config::from_toml_str(&toml_str),
            Err(ConfigError::Parse(_))
        ));
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
    fn graph_unknown_sensor_rejected() {
        let toml_str = base_toml().replace("sensor = \"cpu\"", "sensor = \"gpu\"");
        assert!(errors_of(&toml_str).iter().any(|e| matches!(
            e,
            ValidationError::CurveUnknownSensor { sensor, .. } if sensor == "gpu"
        )));
    }

    #[test]
    fn channel_unknown_curve_rejected() {
        let toml_str = base_toml().replace("curve = \"c\"\n            min_pwm", "curve = \"nope\"\n            min_pwm");
        assert!(errors_of(&toml_str).iter().any(|e| matches!(
            e,
            ValidationError::UnknownCurve { curve, .. } if curve == "nope"
        )));
    }

    #[test]
    fn bad_hysteresis_rejected() {
        for bad in [
            "hysteresis_up = -1.0",
            "hysteresis_up = 25.0",
            "hysteresis_down = nan",
            "response_seconds = 4000",
        ] {
            let toml_str = base_toml().replace("kind = \"graph\"", &format!("kind = \"graph\"\n            {bad}"));
            assert!(
                errors_of(&toml_str)
                    .iter()
                    .any(|e| matches!(e, ValidationError::BadHysteresis { .. })),
                "expected BadHysteresis for `{bad}`"
            );
        }
    }

    #[test]
    fn sane_hysteresis_accepted() {
        let toml_str = base_toml().replace(
            "kind = \"graph\"",
            "kind = \"graph\"\n            hysteresis_up = 2.0\n            hysteresis_down = 3.0\n            response_seconds = 5",
        );
        Config::from_toml_str(&toml_str).expect("sane hysteresis must be accepted");
    }

    fn with_mix(base: &str, mix: &str) -> String {
        base.replace("[channels.pwm1]", &format!("{mix}\n            [channels.pwm1]"))
    }

    #[test]
    fn mix_curve_accepted_and_channel_can_bind_it() {
        let toml_str = with_mix(
            &base_toml().replace("curve = \"c\"\n            min_pwm", "curve = \"m\"\n            min_pwm"),
            "[curves.m]\n            kind = \"mix\"\n            curves = [\"c\"]",
        );
        let cfg = Config::from_toml_str(&toml_str).unwrap();
        assert!(matches!(
            &cfg.curves["m"],
            CurveConfig::Mix(m) if m.function == MixFunction::Max
        ));
    }

    #[test]
    fn empty_mix_rejected() {
        let toml_str = with_mix(
            &base_toml(),
            "[curves.m]\n            kind = \"mix\"\n            curves = []",
        );
        assert!(errors_of(&toml_str)
            .iter()
            .any(|e| matches!(e, ValidationError::EmptyMix { .. })));
    }

    #[test]
    fn mix_unknown_member_rejected() {
        let toml_str = with_mix(
            &base_toml(),
            "[curves.m]\n            kind = \"mix\"\n            curves = [\"nope\"]",
        );
        assert!(errors_of(&toml_str).iter().any(|e| matches!(
            e,
            ValidationError::MixUnknownCurve { member, .. } if member == "nope"
        )));
    }

    #[test]
    fn duplicate_mix_member_rejected() {
        let toml_str = with_mix(
            &base_toml(),
            "[curves.m]\n            kind = \"mix\"\n            curves = [\"c\", \"c\"]",
        );
        assert!(errors_of(&toml_str)
            .iter()
            .any(|e| matches!(e, ValidationError::DuplicateMixMember { .. })));
    }

    #[test]
    fn self_referencing_mix_rejected() {
        let toml_str = with_mix(
            &base_toml(),
            "[curves.m]\n            kind = \"mix\"\n            curves = [\"m\"]",
        );
        assert!(errors_of(&toml_str).iter().any(|e| matches!(
            e,
            ValidationError::MixCycle { curve } if curve == "m"
        )));
    }

    #[test]
    fn mutual_mix_cycle_rejected() {
        let toml_str = with_mix(
            &base_toml(),
            "[curves.a]\n            kind = \"mix\"\n            curves = [\"b\"]\n            [curves.b]\n            kind = \"mix\"\n            curves = [\"a\"]",
        );
        let errs = errors_of(&toml_str);
        assert!(errs.iter().any(|e| matches!(e, ValidationError::MixCycle { .. })), "{errs:?}");
    }

    #[test]
    fn mix_of_mix_without_cycle_accepted() {
        let toml_str = with_mix(
            &base_toml(),
            "[curves.inner]\n            kind = \"mix\"\n            curves = [\"c\"]\n            [curves.outer]\n            kind = \"mix\"\n            curves = [\"inner\", \"c\"]",
        );
        Config::from_toml_str(&toml_str).expect("acyclic mix-of-mix must be accepted");
    }

    #[test]
    fn flat_curve_accepted_and_range_checked() {
        let ok = with_mix(
            &base_toml(),
            "[curves.f]\n            kind = \"flat\"\n            pwm = 128",
        );
        Config::from_toml_str(&ok).unwrap();

        let bad = with_mix(
            &base_toml(),
            "[curves.f]\n            kind = \"flat\"\n            pwm = 300",
        );
        assert!(errors_of(&bad).iter().any(|e| matches!(
            e,
            ValidationError::FlatPwmOutOfRange { pwm: 300, .. }
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
    fn validate_collects_multiple_errors() {
        let toml_str = base_toml()
            .replace("[[40, 80], [70, 200]]", "[[70, 80], [40, 300]]")
            .replace("sensor = \"cpu\"", "sensor = \"gpu\"");
        assert!(errors_of(&toml_str).len() >= 3);
    }
}
