//! Curve evaluation: sorted (temp_c, pwm) points, linear interpolation
//! between points, clamped at both ends.

use thiserror::Error;

use crate::config::GraphCurve;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CurveError {
    #[error("curve has no points")]
    Empty,
    #[error("curve temps must be strictly increasing (point {0})")]
    NotStrictlyIncreasing(usize),
    #[error("curve pwm {pwm} out of range 0-255 (point {index})")]
    PwmOutOfRange { index: usize, pwm: u16 },
}

/// A validated fan curve. `points` is private, so a Curve can only be built
/// through `new`, which enforces the invariants (non-empty, strictly
/// increasing temps) — meaning `eval` can never fail.
#[derive(Debug, Clone, PartialEq)]
pub struct Curve {
    points: Vec<(f64, u8)>,
}

impl Curve {
    pub fn new(points: Vec<(f64, u8)>) -> Result<Self, CurveError> {
        if points.is_empty() {
            return Err(CurveError::Empty);
        }
        for (i, w) in points.windows(2).enumerate() {
            if w[1].0 <= w[0].0 {
                return Err(CurveError::NotStrictlyIncreasing(i + 1));
            }
        }
        Ok(Self { points })
    }

    pub fn points(&self) -> &[(f64, u8)] {
        &self.points
    }

    /// Linear interpolation, clamped to the first/last point outside the
    /// defined range.
    pub fn eval(&self, temp: f64) -> u8 {
        let first = self.points[0];
        let last = self.points[self.points.len() - 1];
        if temp <= first.0 {
            return first.1;
        }
        if temp >= last.0 {
            return last.1;
        }
        for w in self.points.windows(2) {
            let (t0, p0) = w[0];
            let (t1, p1) = w[1];
            if temp <= t1 {
                let frac = (temp - t0) / (t1 - t0);
                return (f64::from(p0) + (f64::from(p1) - f64::from(p0)) * frac).round() as u8;
            }
        }
        // Unreachable: the clamps above cover temps outside the point range.
        last.1
    }
}

impl TryFrom<&GraphCurve> for Curve {
    type Error = CurveError;

    fn try_from(cfg: &GraphCurve) -> Result<Self, CurveError> {
        let points = cfg
            .points
            .iter()
            .enumerate()
            .map(|(i, &(temp, pwm))| {
                let pwm =
                    u8::try_from(pwm).map_err(|_| CurveError::PwmOutOfRange { index: i, pwm })?;
                Ok((f64::from(temp), pwm))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(points)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cpu_rad() -> Curve {
        // The example config's cpu_rad curve.
        Curve::new(vec![(40.0, 80), (60.0, 130), (75.0, 200), (85.0, 255)]).unwrap()
    }

    #[test]
    fn clamps_below_first_point() {
        assert_eq!(cpu_rad().eval(20.0), 80);
        assert_eq!(cpu_rad().eval(40.0), 80);
    }

    #[test]
    fn clamps_above_last_point() {
        assert_eq!(cpu_rad().eval(85.0), 255);
        assert_eq!(cpu_rad().eval(110.0), 255);
    }

    #[test]
    fn exact_points_return_their_pwm() {
        assert_eq!(cpu_rad().eval(60.0), 130);
        assert_eq!(cpu_rad().eval(75.0), 200);
    }

    #[test]
    fn interpolates_between_points() {
        // Midpoint of (40,80)-(60,130) is (50,105).
        assert_eq!(cpu_rad().eval(50.0), 105);
        // 70 is 2/3 of the way from (60,130) to (75,200): 130 + 46.67 → 177.
        assert_eq!(cpu_rad().eval(70.0), 177);
    }

    #[test]
    fn interpolation_rounds_to_nearest() {
        let c = Curve::new(vec![(0.0, 0), (10.0, 1)]).unwrap();
        assert_eq!(c.eval(4.9), 0);
        assert_eq!(c.eval(5.1), 1);
    }

    #[test]
    fn single_point_curve_is_constant() {
        let c = Curve::new(vec![(50.0, 128)]).unwrap();
        assert_eq!(c.eval(0.0), 128);
        assert_eq!(c.eval(50.0), 128);
        assert_eq!(c.eval(100.0), 128);
    }

    #[test]
    fn empty_curve_rejected() {
        assert_eq!(Curve::new(vec![]), Err(CurveError::Empty));
    }

    #[test]
    fn unsorted_curve_rejected() {
        assert_eq!(
            Curve::new(vec![(60.0, 80), (40.0, 130)]),
            Err(CurveError::NotStrictlyIncreasing(1))
        );
        assert_eq!(
            Curve::new(vec![(40.0, 80), (40.0, 130)]),
            Err(CurveError::NotStrictlyIncreasing(1))
        );
    }

    fn graph(points: Vec<(i32, u16)>) -> GraphCurve {
        GraphCurve {
            sensor: "cpu".into(),
            points,
            hysteresis_up: 0.0,
            hysteresis_down: 0.0,
            response_seconds: 0,
            ignore_hysteresis_at_extremes: true,
        }
    }

    #[test]
    fn builds_from_config() {
        let c = Curve::try_from(&graph(vec![(40, 80), (60, 130)])).unwrap();
        assert_eq!(c.eval(50.0), 105);
    }

    #[test]
    fn config_pwm_out_of_range_rejected() {
        assert_eq!(
            Curve::try_from(&graph(vec![(40, 300)])),
            Err(CurveError::PwmOutOfRange { index: 0, pwm: 300 })
        );
    }
}
