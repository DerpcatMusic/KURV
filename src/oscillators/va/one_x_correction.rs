//! Experimental, zero-state compact BLEP, intentionally not wired to runtime.
//! Valid only for constant positive phase increments in (0, 0.5].
//! Includes ALL overlapping periodic edges; correcting only phase and phase-1
//! is incorrect when kernel support exceeds one oscillator period.
//! Continuous phase/shape modulation sidebands are not corrected by this model.
#[path = "one_x_correction_coefficients.rs"]
mod coefficients;
pub use coefficients::{KERNEL_3, KERNEL_4, KERNEL_6};

#[derive(Clone, Copy)]
pub struct Correction<const R: usize> {
    inverse_step: f64,
    support: f64,
}
impl<const R: usize> Correction<R> {
    pub fn new(step: f64) -> Self {
        assert!(step.is_finite() && step > 0.0 && step <= 0.5);
        Self {
            inverse_step: step.recip(),
            support: R as f64 * step,
        }
    }
    #[inline]
    pub fn residual(position: f64, coefficients: &[[f64; 8]; R]) -> f64 {
        let distance = position.abs();
        if distance >= R as f64 {
            return 0.0;
        }
        let index = distance as usize;
        let t = distance - index as f64;
        let c = &coefficients[index];
        let degree = if R == 3 { 5 } else { 7 };
        let mut y = c[degree];
        for j in (0..degree).rev() {
            y = y.mul_add(t, c[j]);
        }
        if position < 0.0 { -y } else { y }
    }
    #[inline]
    pub fn edge(&self, phase: f64, coefficients: &[[f64; 8]; R]) -> f64 {
        if self.support < 0.5 {
            if phase >= self.support && phase <= 1.0 - self.support {
                return 0.0;
            }
            let position = if phase < 0.5 { phase } else { phase - 1.0 };
            return 2.0 * Self::residual(position * self.inverse_step, coefficients);
        }
        let first = (phase - self.support).ceil() as i32;
        let last = (phase + self.support).floor() as i32;
        let mut sum = 0.0;
        for edge in first..=last {
            sum += Self::residual((phase - edge as f64) * self.inverse_step, coefficients);
        }
        2.0 * sum
    }
    #[inline]
    pub fn saw(&self, phase: f64, coefficients: &[[f64; 8]; R]) -> f64 {
        2.0 * phase - 1.0 - self.edge(phase, coefficients)
    }
    #[inline]
    pub fn pulse(&self, phase: f64, width: f64, coefficients: &[[f64; 8]; R]) -> f64 {
        let shifted = phase + 1.0 - width;
        let shifted = if shifted >= 1.0 {
            shifted - 1.0
        } else {
            shifted
        };
        (if phase < width { 1.0 } else { -1.0 }) + self.edge(phase, coefficients)
            - self.edge(shifted, coefficients)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn check<const R: usize>(coefficients: &[[f64; 8]; R]) {
        // BLEP must cancel the exact naive waveform step; half-step at zero.
        assert_eq!(Correction::<R>::residual(0.0, coefficients), -0.5);
        assert_eq!(Correction::<R>::residual(R as f64, coefficients), 0.0);
        for k in 1..R {
            let a = Correction::<R>::residual(k as f64 - 1e-9, coefficients);
            let b = Correction::<R>::residual(k as f64 + 1e-9, coefficients);
            assert!((a - b).abs() < 1e-7, "piece discontinuity {k}: {a} {b}");
        }
        for step in [0.0001, 0.01, 0.08, 0.17, 0.31, 0.499999, 0.5] {
            let c = Correction::<R>::new(step);
            for j in 0..1024 {
                let p = j as f64 / 1024.0;
                let reference = 2.0
                    * (-8..=8)
                        .map(|e| Correction::<R>::residual((p - e as f64) / step, coefficients))
                        .sum::<f64>();
                assert!((c.edge(p, coefficients) - reference).abs() < 1e-12);
                assert!(c.saw(p, coefficients).is_finite());
            }
            // Discontinuity cancels from both sides even when several periods overlap.
            assert!((c.saw(1e-10, coefficients) - c.saw(1.0 - 1e-10, coefficients)).abs() < 1e-5);
        }
    }
    #[test]
    fn support_six_contract() {
        check(&KERNEL_3);
    }
    #[test]
    fn support_eight_contract() {
        check(&KERNEL_4);
    }
    #[test]
    fn support_twelve_contract() {
        check(&KERNEL_6);
    }
}

// SIMD prototype: evaluate only polynomial pieces touched by active lanes.
// No scalar gathers. Caller must guarantee 0 < step < 1/6 in every lane.
macro_rules! narrow_saw {
    ($name:ident,$vector:ty) => {
        #[inline]
        pub fn $name(phase: $vector, inverse_step: $vector, support: $vector) -> $vector {
            use wide::{CmpGt, CmpLt};
            let one = <$vector>::ONE;
            let half = <$vector>::splat(0.5);
            let event = phase.cmp_lt(support) | phase.cmp_gt(one - support);
            let raw = phase * <$vector>::splat(2.0) - one;
            if !event.any() {
                return raw;
            }
            let negative = phase.cmp_gt(half);
            let distance = negative.blend(one - phase, phase) * inverse_step;
            let mut residual = <$vector>::ZERO;
            for interval in 0..3 {
                let lower = <$vector>::splat(interval as f32);
                let mask = event & !distance.cmp_lt(lower) & distance.cmp_lt(lower + one);
                if mask.any() {
                    let t = distance - lower;
                    let c = &KERNEL_3[interval];
                    let mut y = <$vector>::splat(c[5] as f32);
                    for j in (0..5).rev() {
                        y = y.mul_add(t, <$vector>::splat(c[j] as f32));
                    }
                    residual = mask.blend(y, residual);
                }
            }
            raw - negative.blend(-residual, residual) * <$vector>::splat(2.0)
        }
    };
}
narrow_saw!(saw6_narrow4, wide::f32x4);
narrow_saw!(saw6_narrow8, wide::f32x8);
