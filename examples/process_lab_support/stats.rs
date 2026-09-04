//! Whole timed-stream validation for process_lab; never called inside its timer.
#[derive(Debug)]
pub(crate) struct StreamStats {
    pub(crate) finite: bool,
    pub(crate) peak: f32,
    pub(crate) sum: f64,
    pub(crate) energy: f64,
}

impl Default for StreamStats {
    fn default() -> Self {
        Self {
            finite: true,
            peak: 0.0,
            sum: 0.0,
            energy: 0.0,
        }
    }
}

impl StreamStats {
    pub(crate) fn observe(&mut self, samples: &[f32]) {
        for &sample in samples {
            self.finite &= sample.is_finite();
            self.peak = self.peak.max(sample.abs());
            self.sum += f64::from(sample);
            self.energy = f64::from(sample).mul_add(f64::from(sample), self.energy);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn early_nonfinite_is_not_hidden_by_valid_final_buffer() {
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut stats = StreamStats::default();
            stats.observe(&[0.25, bad]);
            stats.observe(&[0.5, -0.5]);
            assert!(!stats.finite);
        }
    }

    #[test]
    fn early_peak_survives_quiet_final_buffer() {
        let mut stats = StreamStats::default();
        stats.observe(&[-2.0, 1.0]);
        stats.observe(&[0.0, 0.0]);
        assert!(stats.finite);
        assert_eq!(stats.peak, 2.0);
        assert_eq!(stats.sum, -1.0);
        assert_eq!(stats.energy, 5.0);
    }
}
