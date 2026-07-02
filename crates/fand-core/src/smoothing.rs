//! Rolling-average smoothing over a per-channel window.
//!
//! Radiator channel uses a longer window (10–15 s, coolant thermal mass);
//! case channel ~5 s. The window is sized in ticks, not seconds — use
//! `window_ticks` to convert.

use std::collections::VecDeque;

#[derive(Debug, Clone)]
pub struct RollingAverage {
    buf: VecDeque<f64>,
    capacity: usize,
}

impl RollingAverage {
    /// Capacity is clamped to at least 1 (a 1-sample window is a no-op).
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            buf: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Push a sample and return the mean of the current window.
    pub fn push(&mut self, value: f64) -> f64 {
        if self.buf.len() == self.capacity {
            self.buf.pop_front();
        }
        self.buf.push_back(value);
        self.mean().expect("just pushed")
    }

    pub fn mean(&self) -> Option<f64> {
        if self.buf.is_empty() {
            None
        } else {
            Some(self.buf.iter().sum::<f64>() / self.buf.len() as f64)
        }
    }
}

/// Window size in ticks for a smoothing duration (rounds up, minimum 1).
pub fn window_ticks(smoothing_seconds: u64, tick_seconds: u64) -> usize {
    let tick = tick_seconds.max(1);
    (smoothing_seconds.div_ceil(tick)).max(1) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mean_over_partial_window() {
        let mut avg = RollingAverage::new(4);
        assert_eq!(avg.push(10.0), 10.0);
        assert_eq!(avg.push(20.0), 15.0);
        assert_eq!(avg.mean(), Some(15.0));
    }

    #[test]
    fn old_samples_fall_out_of_full_window() {
        let mut avg = RollingAverage::new(3);
        avg.push(10.0);
        avg.push(20.0);
        avg.push(30.0);
        // Window full: pushing 40 evicts the 10.
        assert_eq!(avg.push(40.0), 30.0);
    }

    #[test]
    fn spike_is_damped_not_followed() {
        let mut avg = RollingAverage::new(5);
        for _ in 0..5 {
            avg.push(50.0);
        }
        // One 90 °C spike moves a 5-sample window by only 8 degrees.
        assert_eq!(avg.push(90.0), 58.0);
    }

    #[test]
    fn zero_capacity_clamped_to_one() {
        let mut avg = RollingAverage::new(0);
        avg.push(1.0);
        assert_eq!(avg.push(9.0), 9.0);
    }

    #[test]
    fn empty_window_has_no_mean() {
        assert_eq!(RollingAverage::new(3).mean(), None);
    }

    #[test]
    fn window_ticks_rounds_up() {
        assert_eq!(window_ticks(12, 2), 6);
        assert_eq!(window_ticks(5, 2), 3);
        assert_eq!(window_ticks(1, 2), 1);
        assert_eq!(window_ticks(0, 2), 1);
    }
}
