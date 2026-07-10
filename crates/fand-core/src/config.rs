//! Config types (serde/TOML) + full validation before applying.
//!
//! See config/fand.example.toml for the shape. FanControl-style model: a
//! curve owns its temperature source (graph) or combines other curves
//! (mix); a channel always binds exactly one curve by name.
//!
//! Validation rejects: unsorted curve points, pwm out of 0–255, unknown
//! sensor/curve references anywhere in a curve tree, mix cycles, and
//! min_pwm below the channel's floor (60 everywhere, 80 on pwm1 — the AIO
//! pump rides that header inline). Fans never stop: there is no zero-RPM
//! mode.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Fans may stall below this duty; no channel may set a lower min_pwm.
pub const MIN_PWM_FLOOR: u8 = 60;

/// The channel carrying the AIO pump inline (pump + VRM fan + rad fans on
/// one header, pump has no tach). Its duty must never drop below
/// [`PUMP_MIN_PWM_FLOOR`].
pub const PUMP_CHANNEL: &str = "pwm1";

/// At/above the firmware-auto idle (77/255) this system has proven safe.
pub const PUMP_MIN_PWM_FLOOR: u8 = 80;

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
#[serde(deny_unknown_fields)]
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

/// Newtype variants (like `CurveConfig`) so each payload struct can carry
/// `deny_unknown_fields` — serde does not support that attribute directly
/// on internally-tagged enums.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SensorConfig {
    Hwmon(HwmonSensor),
    Nvml(NvmlSensor),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HwmonSensor {
    pub hwmon_name: String,
    pub input: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NvmlSensor {
    pub device_index: u32,
}

/// A curve is graph (points evaluated at its own sensor), mix (combines
/// other curves' *outputs* — never their temperatures), flat (constant), or
/// trigger (latches between two duties across a temperature deadband).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum CurveConfig {
    Graph(GraphCurve),
    Mix(MixCurve),
    Flat(FlatCurve),
    Trigger(TriggerCurve),
}

/// Points as written in TOML: whole-degree temps, pwm parsed wide (u16) so
/// out-of-range values reach validation instead of a serde error.
///
/// The hysteresis fields feed `hysteresis::InputFilter`, which gates the
/// smoothed temp before curve interpolation.
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

/// A two-state step curve (FanControl's "trigger"): idle duty below the
/// deadband, load duty above it, latching in between so it never oscillates.
/// The gap between `idle_temp` and `load_temp` is the hysteresis; a crossing
/// must persist `response_seconds` before it flips. pwm parsed wide (u16)
/// like graph points so out-of-range values reach validation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TriggerCurve {
    pub sensor: String,
    /// At or below this temp the curve latches to `idle_pwm`.
    pub idle_temp: f64,
    pub idle_pwm: u16,
    /// At or above this temp the curve latches to `load_pwm`.
    pub load_temp: f64,
    pub load_pwm: u16,
    /// Seconds a crossing must persist before the latch flips (0 = instant).
    #[serde(default)]
    pub response_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    /// Signed bias added to the curve output before the min_pwm..255 clamp
    /// (lets two channels share one curve with a fixed offset). The clamp
    /// order is unchanged, so a negative offset can never push a fan below
    /// its floor.
    #[serde(default)]
    pub offset_pwm: i16,
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
    #[error("curve `{curve}`: {field} pwm {pwm} out of range 0-255")]
    TriggerPwmOutOfRange {
        curve: String,
        field: &'static str,
        pwm: u16,
    },
    #[error("curve `{curve}`: idle_temp must be a finite value strictly below load_temp")]
    TriggerThresholdsUnordered { curve: String },
    #[error("channel `{channel}`: unknown curve `{curve}`")]
    UnknownCurve { channel: String, curve: String },
    #[error(
        "channel `{channel}`: curve `{curve}` reaches a trigger curve — triggers are a \
         step function and are forbidden on the pump channel (steady control only)"
    )]
    TriggerOnPumpChannel { channel: String, curve: String },
    #[error("channel `{channel}`: offset_pwm {offset} out of range -255..=255")]
    OffsetOutOfRange { channel: String, offset: i16 },
    #[error("channel `{channel}`: min_pwm {min_pwm} is below the stall floor {floor}")]
    MinPwmBelowFloor {
        channel: String,
        min_pwm: u8,
        floor: u8,
    },
    #[error(
        "channel `{channel}`: min_pwm {min_pwm} is below the pump floor {floor} \
         (the AIO pump rides this header inline and must never slow past it)"
    )]
    MinPwmBelowPumpFloor {
        channel: String,
        min_pwm: u8,
        floor: u8,
    },
    #[error("channel `{channel}`: max_step_up and max_step_down must be >= 1")]
    ZeroStep { channel: String },
    #[error("channel `{channel}`: smoothing_seconds must be >= 1")]
    ZeroSmoothing { channel: String },
    #[error(
        "channel `{channel}`: name must be canonical pwmN with no leading zeros \
         (matching the hwmon pwm file)"
    )]
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
                CurveConfig::Trigger(t) => self.validate_trigger(name, t, &mut errs),
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
            if name == PUMP_CHANNEL && ch.min_pwm < PUMP_MIN_PWM_FLOOR {
                errs.push(ValidationError::MinPwmBelowPumpFloor {
                    channel: name.clone(),
                    min_pwm: ch.min_pwm,
                    floor: PUMP_MIN_PWM_FLOOR,
                });
            } else if ch.min_pwm < MIN_PWM_FLOOR {
                errs.push(ValidationError::MinPwmBelowFloor {
                    channel: name.clone(),
                    min_pwm: ch.min_pwm,
                    floor: MIN_PWM_FLOOR,
                });
            }
            if name == PUMP_CHANNEL && self.reaches_trigger(&ch.curve) {
                errs.push(ValidationError::TriggerOnPumpChannel {
                    channel: name.clone(),
                    curve: ch.curve.clone(),
                });
            }
            if ch.offset_pwm.unsigned_abs() > 255 {
                errs.push(ValidationError::OffsetOutOfRange {
                    channel: name.clone(),
                    offset: ch.offset_pwm,
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

    fn validate_trigger(&self, name: &str, t: &TriggerCurve, errs: &mut Vec<ValidationError>) {
        if !t.idle_temp.is_finite() || !t.load_temp.is_finite() || t.idle_temp >= t.load_temp {
            errs.push(ValidationError::TriggerThresholdsUnordered {
                curve: name.to_string(),
            });
        }
        for (field, pwm) in [("idle", t.idle_pwm), ("load", t.load_pwm)] {
            if pwm > 255 {
                errs.push(ValidationError::TriggerPwmOutOfRange {
                    curve: name.to_string(),
                    field,
                    pwm,
                });
            }
        }
        if !self.sensors.contains_key(&t.sensor) {
            errs.push(ValidationError::CurveUnknownSensor {
                curve: name.to_string(),
                sensor: t.sensor.clone(),
            });
        }
        if t.response_seconds > MAX_RESPONSE_SECONDS {
            errs.push(ValidationError::BadHysteresis {
                curve: name.to_string(),
                field: "response_seconds",
                max: MAX_RESPONSE_SECONDS as u32,
            });
        }
    }

    /// Whether any trigger curve is reachable from `root` (directly or
    /// through mixes). A visited set makes this total even on a config with
    /// mix cycles — validation collects every error and never stops early.
    fn reaches_trigger(&self, root: &str) -> bool {
        let mut stack = vec![root.to_string()];
        let mut seen = BTreeSet::new();
        while let Some(name) = stack.pop() {
            if !seen.insert(name.clone()) {
                continue;
            }
            match self.curves.get(&name) {
                Some(CurveConfig::Trigger(_)) => return true,
                Some(CurveConfig::Mix(m)) => stack.extend(m.curves.iter().cloned()),
                _ => {}
            }
        }
        false
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

/// Only canonical `pwmN` names pass — `pwm01` parses to the same physical
/// index as `pwm1` but would dodge every string comparison against
/// [`PUMP_CHANNEL`] (pump floor, trigger ban) and let two channel entries
/// alias one header. Requiring the name to round-trip through its parsed
/// index makes name equality mean physical-header equality.
fn is_pwm_name(name: &str) -> bool {
    name.strip_prefix("pwm")
        .and_then(|n| n.parse::<u32>().ok())
        .is_some_and(|n| name == format!("pwm{n}"))
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

            [channels.pwm2]
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
        let ch = &cfg.channels["pwm2"];
        assert_eq!(ch.max_step_up, 10);
        assert_eq!(ch.max_step_down, 3);
        assert_eq!(ch.deadband, 3);
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
        let toml_str = base_toml().replace(
            "curve = \"c\"\n            min_pwm",
            "curve = \"nope\"\n            min_pwm",
        );
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
            let toml_str = base_toml().replace(
                "kind = \"graph\"",
                &format!("kind = \"graph\"\n            {bad}"),
            );
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
        base.replace(
            "[channels.pwm2]",
            &format!("{mix}\n            [channels.pwm2]"),
        )
    }

    #[test]
    fn mix_curve_accepted_and_channel_can_bind_it() {
        let toml_str = with_mix(
            &base_toml().replace(
                "curve = \"c\"\n            min_pwm",
                "curve = \"m\"\n            min_pwm",
            ),
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
        assert!(
            errs.iter()
                .any(|e| matches!(e, ValidationError::MixCycle { .. })),
            "{errs:?}"
        );
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
        assert!(errors_of(&bad)
            .iter()
            .any(|e| matches!(e, ValidationError::FlatPwmOutOfRange { pwm: 300, .. })));
    }

    fn trigger_block(extra: &str) -> String {
        format!(
            "[curves.t]\n            kind = \"trigger\"\n            sensor = \"cpu\"\n            \
             idle_temp = 40\n            idle_pwm = 90\n            load_temp = 60\n            \
             load_pwm = 200\n{extra}"
        )
    }

    #[test]
    fn trigger_curve_accepted_and_bindable() {
        let toml_str = with_mix(
            &base_toml().replace(
                "curve = \"c\"\n            min_pwm",
                "curve = \"t\"\n            min_pwm",
            ),
            &trigger_block("            response_seconds = 5\n"),
        );
        let cfg = Config::from_toml_str(&toml_str).unwrap();
        assert!(matches!(
            &cfg.curves["t"],
            CurveConfig::Trigger(t) if t.idle_pwm == 90 && t.load_pwm == 200
        ));
    }

    #[test]
    fn trigger_unordered_thresholds_rejected() {
        // load_temp not above idle_temp.
        let toml_str = with_mix(
            &base_toml(),
            "[curves.t]\n            kind = \"trigger\"\n            sensor = \"cpu\"\n            \
             idle_temp = 60\n            idle_pwm = 90\n            load_temp = 40\n            load_pwm = 200",
        );
        assert!(errors_of(&toml_str)
            .iter()
            .any(|e| matches!(e, ValidationError::TriggerThresholdsUnordered { .. })));
    }

    #[test]
    fn trigger_pwm_out_of_range_rejected() {
        let toml_str = with_mix(
            &base_toml(),
            "[curves.t]\n            kind = \"trigger\"\n            sensor = \"cpu\"\n            \
             idle_temp = 40\n            idle_pwm = 90\n            load_temp = 60\n            load_pwm = 300",
        );
        assert!(errors_of(&toml_str).iter().any(|e| matches!(
            e,
            ValidationError::TriggerPwmOutOfRange {
                field: "load",
                pwm: 300,
                ..
            }
        )));
    }

    #[test]
    fn trigger_forbidden_on_pump_channel_directly_and_through_mix() {
        // pwm1 binding a trigger directly.
        let direct = format!(
            "[sensors.cpu]\n            kind = \"hwmon\"\n            hwmon_name = \"k10temp\"\n            \
             input = \"temp1_input\"\n            {}\n            \
             [channels.pwm1]\n            hwmon_name = \"nct6799\"\n            curve = \"t\"\n            \
             min_pwm = 80\n            smoothing_seconds = 5\n",
            trigger_block(""),
        );
        assert!(errors_of(&direct).iter().any(|e| matches!(
            e,
            ValidationError::TriggerOnPumpChannel { channel, .. } if channel == "pwm1"
        )));

        // pwm1 binding a mix that reaches a trigger.
        let via_mix = direct
            .replace("curve = \"t\"", "curve = \"m\"")
            .replace(
                "[channels.pwm1]",
                "[curves.m]\n            kind = \"mix\"\n            curves = [\"t\"]\n            [channels.pwm1]",
            );
        assert!(errors_of(&via_mix)
            .iter()
            .any(|e| matches!(e, ValidationError::TriggerOnPumpChannel { .. })));
    }

    #[test]
    fn trigger_allowed_on_non_pump_channel() {
        let toml_str = with_mix(
            &base_toml().replace(
                "curve = \"c\"\n            min_pwm",
                "curve = \"t\"\n            min_pwm",
            ),
            &trigger_block(""),
        );
        Config::from_toml_str(&toml_str).expect("trigger on pwm2 is fine");
    }

    #[test]
    fn offset_accepted_and_out_of_range_rejected() {
        let ok = base_toml().replace("min_pwm = 70", "min_pwm = 70\noffset_pwm = -20");
        assert_eq!(
            Config::from_toml_str(&ok).unwrap().channels["pwm2"].offset_pwm,
            -20
        );

        let bad = base_toml().replace("min_pwm = 70", "min_pwm = 70\noffset_pwm = 400");
        assert!(errors_of(&bad)
            .iter()
            .any(|e| matches!(e, ValidationError::OffsetOutOfRange { offset: 400, .. })));
    }

    #[test]
    fn zero_rpm_key_rejected_at_parse() {
        // zero_rpm mode was removed outright: fans never stop. An old
        // config carrying the key must fail loudly, not be ignored.
        let toml_str = base_toml().replace("min_pwm = 70", "min_pwm = 70\nzero_rpm = true");
        assert!(matches!(
            Config::from_toml_str(&toml_str),
            Err(ConfigError::Parse(_))
        ));
    }

    #[test]
    fn unknown_top_level_table_rejected_at_parse() {
        let toml_str = format!("{}\n[policy]\nmode = \"single\"\n", base_toml());
        assert!(matches!(
            Config::from_toml_str(&toml_str),
            Err(ConfigError::Parse(_))
        ));
    }

    #[test]
    fn unknown_sensor_field_rejected_at_parse() {
        // deny_unknown_fields lives on the newtype variants' payload
        // structs; this proves it still fires through the tagged enum.
        let toml_str = base_toml().replace(
            "input = \"temp1_input\"",
            "input = \"temp1_input\"\n            offset = 5",
        );
        assert!(matches!(
            Config::from_toml_str(&toml_str),
            Err(ConfigError::Parse(_))
        ));
    }

    #[test]
    fn unknown_curve_field_rejected_at_parse() {
        let toml_str = base_toml().replace(
            "sensor = \"cpu\"",
            "sensor = \"cpu\"\n            fill = true",
        );
        assert!(matches!(
            Config::from_toml_str(&toml_str),
            Err(ConfigError::Parse(_))
        ));
    }

    #[test]
    fn min_pwm_below_floor_rejected() {
        let toml_str = base_toml().replace("min_pwm = 70", "min_pwm = 30");
        assert!(errors_of(&toml_str)
            .iter()
            .any(|e| matches!(e, ValidationError::MinPwmBelowFloor { min_pwm: 30, .. })));
    }

    #[test]
    fn pump_channel_min_pwm_floor_is_80() {
        // pwm1 carries the AIO pump inline: 70 clears the generic floor
        // but not the pump floor.
        let toml_str = base_toml().replace("[channels.pwm2]", "[channels.pwm1]");
        assert!(errors_of(&toml_str).iter().any(|e| matches!(
            e,
            ValidationError::MinPwmBelowPumpFloor {
                min_pwm: 70,
                floor: 80,
                ..
            }
        )));

        let ok = toml_str.replace("min_pwm = 70", "min_pwm = 80");
        Config::from_toml_str(&ok).expect("pwm1 at the pump floor must be accepted");
    }

    #[test]
    fn bad_channel_name_rejected() {
        let toml_str = base_toml().replace("[channels.pwm2]", "[channels.pmw2]");
        assert!(errors_of(&toml_str)
            .iter()
            .any(|e| matches!(e, ValidationError::BadChannelName { .. })));
    }

    #[test]
    fn non_canonical_pwm_alias_rejected() {
        // `pwm01` would parse to physical index 1 — the pump header — while
        // dodging the string checks against PUMP_CHANNEL (min_pwm 80 floor,
        // trigger ban). It must die at the name check instead.
        let toml_str = base_toml().replace("[channels.pwm2]", "[channels.pwm01]");
        let errs = errors_of(&toml_str);
        assert!(errs
            .iter()
            .any(|e| matches!(e, ValidationError::BadChannelName { .. })));
        // And precisely because the name is rejected, the pump floor was
        // *not* consulted — no half-validated alias slips through.
        assert!(!errs
            .iter()
            .any(|e| matches!(e, ValidationError::MinPwmBelowPumpFloor { .. })));

        for name in ["pwm001", "pwm+1", "pwm 1", "pwm999999999999"] {
            let toml_str =
                base_toml().replace("[channels.pwm2]", &format!("[channels.\"{name}\"]"));
            assert!(
                errors_of(&toml_str)
                    .iter()
                    .any(|e| matches!(e, ValidationError::BadChannelName { .. })),
                "`{name}` must be rejected"
            );
        }
    }

    #[test]
    fn pump_alias_cannot_duplicate_the_pump_channel() {
        // pwm1 and pwm01 are distinct TOML keys but the same physical
        // header; canonical-name validation makes the alias unrepresentable.
        let toml_str = format!(
            "{}\n{}",
            base_toml()
                .replace("[channels.pwm2]", "[channels.pwm1]")
                .replace("min_pwm = 70", "min_pwm = 80"),
            r#"
            [channels.pwm01]
            hwmon_name = "nct6799"
            curve = "c"
            min_pwm = 60
            smoothing_seconds = 5
            "#
        );
        assert!(errors_of(&toml_str).iter().any(|e| matches!(
            e,
            ValidationError::BadChannelName { channel } if channel == "pwm01"
        )));
    }

    #[test]
    fn validate_collects_multiple_errors() {
        let toml_str = base_toml()
            .replace("[[40, 80], [70, 200]]", "[[70, 80], [40, 300]]")
            .replace("sensor = \"cpu\"", "sensor = \"gpu\"");
        assert!(errors_of(&toml_str).len() >= 3);
    }
}
