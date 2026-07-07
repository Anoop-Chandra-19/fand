//! Curve-tree evaluation: resolve a channel's bound curve into an owned
//! tree, then evaluate it each tick against the latest sensor temps.
//!
//! Mix nodes combine member *outputs* (each graph evaluated at its own
//! sensor), never temperatures — 70 °C means different things per
//! component. Every member is evaluated every tick even when max/min
//! short-circuiting would allow skipping, so each graph node's smoothing
//! window stays warm.
//!
//! Each channel builds its own tree, so two channels sharing a named curve
//! get independent smoothing state — same as the old per-channel-input
//! smoothers.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use thiserror::Error;

use crate::config::{CurveConfig, MixFunction};
use crate::curve::{Curve, CurveError};
use crate::hysteresis::InputFilter;
use crate::smoothing::RollingAverage;

/// Trees deeper than this can only come from an unvalidated config (cycles
/// are rejected by `Config::validate`); the cap makes `build` total anyway.
const MAX_DEPTH: usize = 16;

#[derive(Debug, Error, PartialEq)]
pub enum TreeError {
    #[error("unknown curve `{0}`")]
    UnknownCurve(String),
    #[error("curve `{curve}`: {source}")]
    BadCurve {
        curve: String,
        source: CurveError,
    },
    #[error("curve `{curve}`: flat pwm {pwm} out of range 0-255")]
    FlatPwmOutOfRange { curve: String, pwm: u16 },
    #[error("curve nesting deeper than {MAX_DEPTH} — mix cycle?")]
    TooDeep,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EvalError {
    #[error("no temperature for sensor `{0}`")]
    MissingSensor(String),
}

#[derive(Debug, Clone)]
pub enum CurveTree {
    Graph {
        sensor: String,
        curve: Curve,
        smoother: RollingAverage,
        /// None when the curve's hysteresis knobs are all defaults.
        filter: Option<InputFilter>,
    },
    Mix {
        function: MixFunction,
        members: Vec<CurveTree>,
    },
    Flat {
        pwm: u8,
    },
}

impl CurveTree {
    /// Resolve `root` against the curve table. `smoothing_window` (in
    /// ticks) sizes the rolling average of every graph node in this tree —
    /// smoothing is a per-channel property, and one tree belongs to exactly
    /// one channel.
    pub fn build(
        curves: &BTreeMap<String, CurveConfig>,
        root: &str,
        smoothing_window: usize,
    ) -> Result<Self, TreeError> {
        Self::build_at(curves, root, smoothing_window, 0)
    }

    fn build_at(
        curves: &BTreeMap<String, CurveConfig>,
        name: &str,
        smoothing_window: usize,
        depth: usize,
    ) -> Result<Self, TreeError> {
        if depth > MAX_DEPTH {
            return Err(TreeError::TooDeep);
        }
        let cfg = curves
            .get(name)
            .ok_or_else(|| TreeError::UnknownCurve(name.to_string()))?;
        match cfg {
            CurveConfig::Graph(g) => {
                let curve = Curve::try_from(g).map_err(|source| TreeError::BadCurve {
                    curve: name.to_string(),
                    source,
                })?;
                let filter = InputFilter::new(g, &curve);
                Ok(CurveTree::Graph {
                    sensor: g.sensor.clone(),
                    curve,
                    smoother: RollingAverage::new(smoothing_window),
                    filter,
                })
            }
            CurveConfig::Mix(m) => {
                let members = m
                    .curves
                    .iter()
                    .map(|member| Self::build_at(curves, member, smoothing_window, depth + 1))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(CurveTree::Mix {
                    function: m.function,
                    members,
                })
            }
            CurveConfig::Flat(f) => Ok(CurveTree::Flat {
                pwm: u8::try_from(f.pwm).map_err(|_| TreeError::FlatPwmOutOfRange {
                    curve: name.to_string(),
                    pwm: f.pwm,
                })?,
            }),
        }
    }

    /// Every sensor name this tree reads — what the daemon must sample each
    /// tick for `eval` to succeed. Sensors outside this set (for any of a
    /// channel's trees) need not be read at all.
    pub fn sensors(&self) -> BTreeSet<&str> {
        let mut out = BTreeSet::new();
        self.collect_sensors(&mut out);
        out
    }

    fn collect_sensors<'a>(&'a self, out: &mut BTreeSet<&'a str>) {
        match self {
            CurveTree::Graph { sensor, .. } => {
                out.insert(sensor.as_str());
            }
            CurveTree::Mix { members, .. } => {
                for m in members {
                    m.collect_sensors(out);
                }
            }
            CurveTree::Flat { .. } => {}
        }
    }

    /// One tick: push temps through each graph node's smoother, gate through
    /// its hysteresis filter, interpolate, combine mix outputs. `&mut`
    /// because smoothing and hysteresis are stateful. `now` drives the
    /// filters' response-time dwell.
    pub fn eval(&mut self, temps: &BTreeMap<String, f64>, now: Instant) -> Result<u8, EvalError> {
        match self {
            CurveTree::Graph {
                sensor,
                curve,
                smoother,
                filter,
            } => {
                let temp = *temps
                    .get(sensor.as_str())
                    .ok_or_else(|| EvalError::MissingSensor(sensor.clone()))?;
                let smoothed = smoother.push(temp);
                let effective = match filter {
                    Some(f) => f.apply(smoothed, now),
                    None => smoothed,
                };
                Ok(curve.eval(effective))
            }
            CurveTree::Mix { function, members } => {
                // Evaluate all members first (keeps every smoother warm),
                // then combine. Validation guarantees at least one member.
                let outputs = members
                    .iter_mut()
                    .map(|m| m.eval(temps, now))
                    .collect::<Result<Vec<u8>, _>>()?;
                Ok(match function {
                    MixFunction::Max => outputs.iter().copied().max().unwrap_or(0),
                    MixFunction::Min => outputs.iter().copied().min().unwrap_or(0),
                    MixFunction::Average => {
                        let sum: u32 = outputs.iter().map(|&p| u32::from(p)).sum();
                        (f64::from(sum) / outputs.len() as f64).round() as u8
                    }
                })
            }
            CurveTree::Flat { pwm } => Ok(*pwm),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{FlatCurve, GraphCurve, MixCurve};

    fn graph(sensor: &str, points: Vec<(i32, u16)>) -> CurveConfig {
        CurveConfig::Graph(GraphCurve {
            sensor: sensor.into(),
            points,
            hysteresis_up: 0.0,
            hysteresis_down: 0.0,
            response_seconds: 0,
            ignore_hysteresis_at_extremes: true,
        })
    }

    fn mix(function: MixFunction, members: &[&str]) -> CurveConfig {
        CurveConfig::Mix(MixCurve {
            function,
            curves: members.iter().map(|s| s.to_string()).collect(),
        })
    }

    fn temps(pairs: &[(&str, f64)]) -> BTreeMap<String, f64> {
        pairs.iter().map(|&(k, v)| (k.to_string(), v)).collect()
    }

    /// The example config's case mix, as curve configs.
    fn case_curves() -> BTreeMap<String, CurveConfig> {
        BTreeMap::from([
            ("cpu_case".into(), graph("cpu", vec![(45, 90), (70, 160), (85, 255)])),
            ("gpu_case".into(), graph("gpu", vec![(45, 90), (60, 140), (75, 255)])),
            ("case_mix".into(), mix(MixFunction::Max, &["cpu_case", "gpu_case"])),
        ])
    }

    #[test]
    fn graph_evaluates_at_own_sensor() {
        let curves = BTreeMap::from([("c".into(), graph("cpu", vec![(40, 80), (70, 200)]))]);
        let mut tree = CurveTree::build(&curves, "c", 1).unwrap();
        assert_eq!(tree.eval(&temps(&[("cpu", 54.0)]), Instant::now()).unwrap(), 136);
    }

    #[test]
    fn flat_is_constant() {
        let curves = BTreeMap::from([("f".into(), CurveConfig::Flat(FlatCurve { pwm: 128 }))]);
        let mut tree = CurveTree::build(&curves, "f", 1).unwrap();
        assert_eq!(tree.eval(&temps(&[]), Instant::now()).unwrap(), 128);
    }

    #[test]
    fn sensors_lists_every_graph_sensor_in_the_tree() {
        let tree = CurveTree::build(&case_curves(), "case_mix", 1).unwrap();
        assert_eq!(tree.sensors(), BTreeSet::from(["cpu", "gpu"]));

        let curves = BTreeMap::from([("f".into(), CurveConfig::Flat(FlatCurve { pwm: 128 }))]);
        let flat = CurveTree::build(&curves, "f", 1).unwrap();
        assert!(flat.sensors().is_empty());
    }

    #[test]
    fn max_mix_takes_max_of_outputs() {
        // Gaming: CPU 62 °C → ~138, GPU 71 °C → ~224. GPU demand wins.
        let mut tree = CurveTree::build(&case_curves(), "case_mix", 1).unwrap();
        let out = tree.eval(&temps(&[("cpu", 62.0), ("gpu", 71.0)]), Instant::now()).unwrap();
        assert_eq!(out, 224);
    }

    #[test]
    fn hotter_component_does_not_automatically_win() {
        // CPU is hotter (80 vs 60) but its curve is relaxed; GPU's curve is
        // steep. Max-of-outputs picks the GPU's demand — feeding the max
        // *temperature* into one curve would get this wrong.
        let curves = BTreeMap::from([
            ("relaxed".into(), graph("cpu", vec![(40, 60), (90, 100)])),
            ("steep".into(), graph("gpu", vec![(40, 60), (65, 255)])),
            ("m".into(), mix(MixFunction::Max, &["relaxed", "steep"])),
        ]);
        let mut tree = CurveTree::build(&curves, "m", 1).unwrap();
        let out = tree.eval(&temps(&[("cpu", 80.0), ("gpu", 60.0)]), Instant::now()).unwrap();
        // steep at 60 °C: 60 + (255-60) * 20/25 = 216; relaxed at 80: 92.
        assert_eq!(out, 216);
    }

    #[test]
    fn min_and_average_mixes() {
        let mut curves = case_curves();
        curves.insert("min_mix".into(), mix(MixFunction::Min, &["cpu_case", "gpu_case"]));
        curves.insert("avg_mix".into(), mix(MixFunction::Average, &["cpu_case", "gpu_case"]));
        let t = temps(&[("cpu", 62.0), ("gpu", 71.0)]); // outputs 138 and 224

        let mut min_tree = CurveTree::build(&curves, "min_mix", 1).unwrap();
        assert_eq!(min_tree.eval(&t, Instant::now()).unwrap(), 138);

        let mut avg_tree = CurveTree::build(&curves, "avg_mix", 1).unwrap();
        assert_eq!(avg_tree.eval(&t, Instant::now()).unwrap(), 181); // (138+224)/2 = 181

        let mut max_tree = CurveTree::build(&curves, "case_mix", 1).unwrap();
        assert_eq!(max_tree.eval(&t, Instant::now()).unwrap(), 224);
    }

    #[test]
    fn single_member_mix_equals_the_member() {
        let curves = BTreeMap::from([
            ("c".into(), graph("cpu", vec![(40, 80), (60, 130)])),
            ("m".into(), mix(MixFunction::Max, &["c"])),
        ]);
        let t = temps(&[("cpu", 50.0)]);
        let mut member = CurveTree::build(&curves, "c", 1).unwrap();
        let mut wrapped = CurveTree::build(&curves, "m", 1).unwrap();
        assert_eq!(wrapped.eval(&t, Instant::now()).unwrap(), member.eval(&t, Instant::now()).unwrap());
    }

    #[test]
    fn mix_of_mix_evaluates() {
        let mut curves = case_curves();
        curves.insert("outer".into(), mix(MixFunction::Min, &["case_mix", "cpu_case"]));
        let mut tree = CurveTree::build(&curves, "outer", 1).unwrap();
        // case_mix = max(138, 224) = 224; cpu_case = 138; min = 138.
        assert_eq!(tree.eval(&temps(&[("cpu", 62.0), ("gpu", 71.0)]), Instant::now()).unwrap(), 138);
    }

    #[test]
    fn mix_members_all_keep_smoothing_warm() {
        // Window of 2 ticks: the second eval must average with the first
        // sample on BOTH members, including the one that never wins.
        let mut tree = CurveTree::build(&case_curves(), "case_mix", 2).unwrap();
        tree.eval(&temps(&[("cpu", 45.0), ("gpu", 71.0)]), Instant::now()).unwrap();
        let out = tree.eval(&temps(&[("cpu", 85.0), ("gpu", 71.0)]), Instant::now()).unwrap();
        // cpu smoothed to (45+85)/2 = 65 → 90 + 70*(20/25) = 146; gpu 224
        // wins the max — but only because the cpu member stayed warm at 146.
        assert_eq!(out, 224);
    }

    #[test]
    fn graph_hysteresis_holds_output_inside_band() {
        let curves = BTreeMap::from([(
            "c".into(),
            CurveConfig::Graph(GraphCurve {
                sensor: "cpu".into(),
                points: vec![(40, 80), (70, 200)],
                hysteresis_up: 3.0,
                hysteresis_down: 3.0,
                response_seconds: 0,
                ignore_hysteresis_at_extremes: true,
            }),
        )]);
        let mut tree = CurveTree::build(&curves, "c", 1).unwrap();
        let now = Instant::now();
        assert_eq!(tree.eval(&temps(&[("cpu", 54.0)]), now).unwrap(), 136);
        // +1.5 °C is inside the band: output pinned to the accepted 54 °C.
        assert_eq!(tree.eval(&temps(&[("cpu", 55.5)]), now).unwrap(), 136);
        // +4 °C clears hysteresis_up: curve sees 58 °C → 152.
        assert_eq!(tree.eval(&temps(&[("cpu", 58.0)]), now).unwrap(), 152);
        // Beyond the last point the bypass wins immediately even mid-band.
        assert_eq!(tree.eval(&temps(&[("cpu", 71.0)]), now).unwrap(), 200);
    }

    #[test]
    fn unknown_curve_is_a_build_error() {
        let curves = case_curves();
        assert_eq!(
            CurveTree::build(&curves, "nope", 1).unwrap_err(),
            TreeError::UnknownCurve("nope".into())
        );
    }

    #[test]
    fn missing_sensor_temp_is_an_eval_error() {
        let curves = BTreeMap::from([("c".into(), graph("cpu", vec![(40, 80), (70, 200)]))]);
        let mut tree = CurveTree::build(&curves, "c", 1).unwrap();
        assert_eq!(
            tree.eval(&temps(&[("gpu", 50.0)]), Instant::now()),
            Err(EvalError::MissingSensor("cpu".into()))
        );
    }

    #[test]
    fn cyclic_config_hits_depth_cap_instead_of_recursing_forever() {
        // Config::validate rejects cycles; build must still be total when
        // handed an unvalidated map.
        let curves = BTreeMap::from([
            ("a".into(), mix(MixFunction::Max, &["b"])),
            ("b".into(), mix(MixFunction::Max, &["a"])),
        ]);
        assert_eq!(CurveTree::build(&curves, "a", 1).unwrap_err(), TreeError::TooDeep);
    }
}
