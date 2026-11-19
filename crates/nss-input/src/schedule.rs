//! Timer cadence and sampling statistics for 120 Hz cursor tracking.
//!
//! Port of upstream `NSSInput/SamplingSchedule.swift`.

use std::time::Duration;

/// Timer cadence for cursor sampling, clamped to `[1, 1000]` Hz.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SamplingSchedule {
    pub rate: f64,
}

impl Default for SamplingSchedule {
    fn default() -> Self {
        Self::new(120.0)
    }
}

impl SamplingSchedule {
    #[must_use]
    pub fn new(rate: f64) -> Self {
        let r = if rate.is_finite() {
            rate.clamp(1.0, 1000.0)
        } else {
            120.0
        };
        Self { rate: r }
    }

    #[must_use]
    pub fn interval(&self) -> f64 {
        1.0 / self.rate
    }

    #[must_use]
    pub fn interval_nanos(&self) -> u64 {
        (1_000_000_000.0 / self.rate).round() as u64
    }

    #[must_use]
    pub fn interval_duration(&self) -> Duration {
        Duration::from_nanos(self.interval_nanos())
    }
}

/// Measures how faithfully a recorded series of sample timestamps tracks its nominal rate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SamplingStats {
    pub count: usize,
    /// Samples per second over the span.
    pub mean_rate: f64,
    pub mean_interval: f64,
    pub max_gap: f64,
    /// Standard deviation of inter-sample intervals, in seconds.
    pub jitter: f64,
}

impl SamplingStats {
    #[must_use]
    pub fn from_times(times: &[f64]) -> Self {
        let count = times.len();
        if count < 2 {
            return Self {
                count,
                mean_rate: 0.0,
                mean_interval: 0.0,
                max_gap: 0.0,
                jitter: 0.0,
            };
        }
        let mut gaps = Vec::with_capacity(count - 1);
        for w in times.windows(2) {
            gaps.push(w[1] - w[0]);
        }
        let n = gaps.len() as f64;
        let mean = gaps.iter().sum::<f64>() / n;
        let variance = gaps.iter().map(|&g| (g - mean) * (g - mean)).sum::<f64>() / n;
        let max_gap = gaps
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max)
            .max(0.0);
        Self {
            count,
            mean_rate: if mean > 0.0 { 1.0 / mean } else { 0.0 },
            mean_interval: mean,
            max_gap,
            jitter: variance.sqrt(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_intervals_and_clamping() {
        assert_eq!(SamplingSchedule::default().interval_nanos(), 8_333_333);
        assert!((SamplingSchedule::new(120.0).interval() - 1.0 / 120.0).abs() < 1e-12);
        assert_eq!(SamplingSchedule::new(0.0).rate, 1.0);
        assert_eq!(SamplingSchedule::new(1e6).rate, 1000.0);
        assert_eq!(SamplingSchedule::new(f64::NAN).rate, 120.0);
    }

    #[test]
    fn stats_detect_rate_jitter_and_gaps() {
        let steady: Vec<f64> = (0..121).map(|i| f64::from(i) / 120.0).collect();
        let s = SamplingStats::from_times(&steady);
        assert!((s.mean_rate - 120.0).abs() < 1e-6);
        assert!(s.jitter < 1e-9);

        let mut gappy = steady;
        gappy[60] += 0.003;
        let g = SamplingStats::from_times(&gappy);
        assert!(g.max_gap > 1.0 / 120.0);
        assert!(g.jitter > 0.0);
        assert_eq!(SamplingStats::from_times(&[1.0]).mean_rate, 0.0);
    }
}
