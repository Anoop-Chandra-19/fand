//! Input-side hysteresis + response time for graph curves (FanControl's
//! "hysteresis" / "response time" pair).
//!
//! The filter sits between the smoothed sensor temp and curve
//! interpolation. It holds an *accepted* temperature and only moves it when
//! the incoming temp has departed by at least `hysteresis_up` (rising) or
//! `hysteresis_down` (falling) AND stayed departed for `response_seconds`.
//! Small wiggles inside the band — or excursions that retreat before the
//! dwell elapses — never reach the curve, so the fan speed stays put.
//!
//! Safety: with `ignore_hysteresis_at_extremes` (the default), a temp at or
//! beyond the curve's endpoint temps is accepted immediately — a spike past
//! the last point must reach full duty without waiting out a dwell timer.

use std::time::{Duration, Instant};

use crate::config::GraphCurve;
use crate::curve::Curve;

#[derive(Debug, Clone)]
pub struct InputFilter {
    up: f64,
    down: f64,
    response: Duration,
    ignore_at_extremes: bool,
    /// The curve's first/last point temps — the bypass region.
    min_temp: f64,
    max_temp: f64,
    state: Option<State>,
}

#[derive(Debug, Clone)]
struct State {
    accepted: f64,
    pending: Option<Pending>,
}

/// An excursion beyond the band that has not yet survived the dwell.
#[derive(Debug, Clone)]
struct Pending {
    rising: bool,
    since: Instant,
}

impl InputFilter {
    /// None when every knob is at its default — the common case costs
    /// nothing and provably behaves like pre-hysteresis builds.
    pub fn new(cfg: &GraphCurve, curve: &Curve) -> Option<Self> {
        if cfg.hysteresis_up == 0.0 && cfg.hysteresis_down == 0.0 && cfg.response_seconds == 0 {
            return None;
        }
        let points = curve.points();
        Some(Self {
            up: cfg.hysteresis_up,
            down: cfg.hysteresis_down,
            response: Duration::from_secs(cfg.response_seconds),
            ignore_at_extremes: cfg.ignore_hysteresis_at_extremes,
            min_temp: points[0].0,
            max_temp: points[points.len() - 1].0,
            state: None,
        })
    }

    /// Feed one smoothed sample; returns the temp the curve should see.
    pub fn apply(&mut self, temp: f64, now: Instant) -> f64 {
        let Some(state) = &mut self.state else {
            // First sample anchors the filter.
            self.state = Some(State {
                accepted: temp,
                pending: None,
            });
            return temp;
        };

        if self.ignore_at_extremes && (temp <= self.min_temp || temp >= self.max_temp) {
            state.accepted = temp;
            state.pending = None;
            return temp;
        }

        let delta = temp - state.accepted;
        // A zero threshold means any movement in that direction qualifies.
        let rising = delta > 0.0 && delta >= self.up;
        let falling = delta < 0.0 && -delta >= self.down;
        if !rising && !falling {
            // Inside the band: any pending excursion retreated, so its
            // dwell timer starts over if it happens again.
            state.pending = None;
            return state.accepted;
        }

        if self.response.is_zero() {
            state.accepted = temp;
            return temp;
        }

        match &state.pending {
            Some(p) if p.rising == rising => {
                if now.duration_since(p.since) >= self.response {
                    state.accepted = temp;
                    state.pending = None;
                    temp
                } else {
                    state.accepted
                }
            }
            // No pending excursion, or the direction flipped mid-dwell:
            // restart the timer for this direction.
            _ => {
                state.pending = Some(Pending { rising, since: now });
                state.accepted
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(up: f64, down: f64, response: u64, ignore: bool) -> GraphCurve {
        GraphCurve {
            sensor: "cpu".into(),
            points: vec![(40, 80), (70, 200)],
            hysteresis_up: up,
            hysteresis_down: down,
            response_seconds: response,
            ignore_hysteresis_at_extremes: ignore,
        }
    }

    fn filter(up: f64, down: f64, response: u64, ignore: bool) -> InputFilter {
        let cfg = graph(up, down, response, ignore);
        let curve = Curve::try_from(&cfg).unwrap();
        InputFilter::new(&cfg, &curve).expect("filter should be active")
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn all_defaults_build_no_filter() {
        let cfg = graph(0.0, 0.0, 0, true);
        let curve = Curve::try_from(&cfg).unwrap();
        assert!(InputFilter::new(&cfg, &curve).is_none());
    }

    #[test]
    fn first_sample_passes_through() {
        let mut f = filter(2.0, 3.0, 0, true);
        assert_eq!(f.apply(54.0, Instant::now()), 54.0);
    }

    #[test]
    fn changes_inside_the_band_are_held() {
        let mut f = filter(2.0, 3.0, 0, true);
        let now = Instant::now();
        f.apply(54.0, now);
        assert_eq!(f.apply(55.9, now), 54.0, "below up threshold");
        assert_eq!(f.apply(51.1, now), 54.0, "below down threshold");
        assert_eq!(f.apply(54.0, now), 54.0);
    }

    #[test]
    fn change_beyond_threshold_accepted_without_response_time() {
        let mut f = filter(2.0, 3.0, 0, true);
        let now = Instant::now();
        f.apply(54.0, now);
        assert_eq!(f.apply(56.5, now), 56.5, "rise of 2.5 clears up=2");
        // The band re-anchors at the new accepted value.
        assert_eq!(f.apply(55.0, now), 56.5, "fall of 1.5 held by down=3");
        assert_eq!(f.apply(53.0, now), 53.0, "fall of 3.5 clears down=3");
    }

    #[test]
    fn asymmetric_band_lets_down_lag_up() {
        // up=1, down=5: chases heat quickly, coasts down reluctantly.
        let mut f = filter(1.0, 5.0, 0, true);
        let now = Instant::now();
        f.apply(60.0, now);
        assert_eq!(f.apply(61.5, now), 61.5, "small rise accepted");
        assert_eq!(f.apply(58.0, now), 61.5, "3.5 fall held");
        assert_eq!(f.apply(56.0, now), 56.0, "5.5 fall accepted");
    }

    #[test]
    fn excursion_must_survive_the_dwell() {
        let mut f = filter(2.0, 2.0, 5, true);
        let base = Instant::now();
        f.apply(54.0, base);
        assert_eq!(f.apply(58.0, base), 54.0, "dwell starts, not accepted yet");
        assert_eq!(f.apply(58.0, base + secs(2)), 54.0, "2s < 5s");
        assert_eq!(f.apply(59.0, base + secs(6)), 59.0, "6s ≥ 5s: latest temp accepted");
    }

    #[test]
    fn retreat_into_band_resets_the_dwell() {
        let mut f = filter(2.0, 2.0, 5, true);
        let base = Instant::now();
        f.apply(54.0, base);
        f.apply(58.0, base); // pending since base
        f.apply(54.5, base + secs(2)); // back inside: pending cleared
        assert_eq!(
            f.apply(58.0, base + secs(6)),
            54.0,
            "timer restarted at 6s, not elapsed"
        );
        assert_eq!(f.apply(58.0, base + secs(12)), 58.0);
    }

    #[test]
    fn direction_flip_resets_the_dwell() {
        let mut f = filter(2.0, 2.0, 5, true);
        let base = Instant::now();
        f.apply(54.0, base);
        f.apply(58.0, base); // rising pending
        f.apply(50.0, base + secs(2)); // falling: new pending
        assert_eq!(
            f.apply(50.0, base + secs(6)),
            54.0,
            "falling dwell started at 2s, only 4s elapsed"
        );
        assert_eq!(f.apply(50.0, base + secs(8)), 50.0);
    }

    #[test]
    fn extremes_bypass_hysteresis_and_dwell() {
        // Curve endpoints are 40 and 70 °C.
        let mut f = filter(5.0, 5.0, 30, true);
        let base = Instant::now();
        f.apply(60.0, base);
        assert_eq!(f.apply(63.0, base), 60.0, "inside band, held");
        assert_eq!(f.apply(71.0, base), 71.0, "beyond last point: instant");
        assert_eq!(f.apply(39.0, base), 39.0, "below first point: instant");
    }

    #[test]
    fn extremes_respected_when_bypass_disabled() {
        let mut f = filter(5.0, 5.0, 30, false);
        let base = Instant::now();
        f.apply(68.0, base);
        assert_eq!(f.apply(74.0, base), 68.0, "no bypass: dwell applies");
        assert_eq!(f.apply(74.0, base + secs(31)), 74.0);
    }

    #[test]
    fn zero_hysteresis_with_response_deglitches_any_change() {
        let mut f = filter(0.0, 0.0, 5, true);
        let base = Instant::now();
        f.apply(54.0, base);
        assert_eq!(f.apply(54.3, base), 54.0, "any rise starts a dwell");
        assert_eq!(f.apply(54.3, base + secs(6)), 54.3, "sustained: accepted");
    }
}
