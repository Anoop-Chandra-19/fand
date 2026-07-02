//! Mix mode: evaluate each curve at its own sensor's temperature and take
//! the **max of the resulting PWMs**.
//!
//! Deliberately max-of-outputs, not one curve fed max-temp — 70 °C means
//! different things per component.

use crate::curve::Curve;

/// Returns the highest PWM demanded by any (temperature, curve) pair, or
/// None for an empty input (config validation rejects empty mixes, so the
/// daemon never sees None for a configured channel).
pub fn eval_max(inputs: &[(f64, &Curve)]) -> Option<u8> {
    inputs.iter().map(|&(temp, curve)| curve.eval(temp)).max()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn curve<const N: usize>(points: [(f64, u8); N]) -> Curve {
        Curve::new(points.to_vec()).unwrap()
    }

    fn eval<const N: usize>(inputs: [(f64, &Curve); N]) -> Option<u8> {
        eval_max(inputs.as_slice())
    }

    #[test]
    fn takes_max_of_outputs() {
        let cpu_case = curve([(45.0, 90), (70.0, 160), (85.0, 255)]);
        let gpu_case = curve([(45.0, 90), (60.0, 140), (75.0, 255)]);
        // Gaming: CPU 62 °C → ~138, GPU 71 °C → ~224. GPU demand wins.
        let pwm = eval([(62.0, &cpu_case), (71.0, &gpu_case)]).unwrap();
        assert_eq!(pwm, gpu_case.eval(71.0));
        assert!(pwm > cpu_case.eval(62.0));
    }

    #[test]
    fn hotter_component_does_not_automatically_win() {
        // CPU is hotter (80 vs 60) but its curve is relaxed; GPU's curve is
        // steep. Max-of-outputs picks the GPU's demand — feeding the max
        // *temperature* into one curve would get this wrong.
        let relaxed = curve([(40.0, 60), (90.0, 100)]);
        let steep = curve([(40.0, 60), (65.0, 255)]);
        let pwm = eval([(80.0, &relaxed), (60.0, &steep)]).unwrap();
        assert_eq!(pwm, steep.eval(60.0));
        assert!(steep.eval(60.0) > relaxed.eval(80.0));
    }

    #[test]
    fn single_input_mix_equals_single_policy() {
        let c = curve([(40.0, 80), (60.0, 130)]);
        assert_eq!(eval([(50.0, &c)]), Some(c.eval(50.0)));
    }

    #[test]
    fn empty_mix_is_none() {
        assert_eq!(eval_max(&[]), None);
    }
}
