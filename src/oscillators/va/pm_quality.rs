//! Experimental local trajectory treatments; not selected by production routing.
use super::antialias::{Antialiasing, bandlimited_saw};
use truce_simd::simd::f32x8;
use wide::CmpLt;

/// Local constant-speed approximation using a signed, unwrapped phase increment.
/// This cannot recover intersample curvature or modulation sidebands.
#[inline]
pub fn saw_local_width(phase: f64, signed_step: f64) -> f64 {
    bandlimited_saw(
        phase,
        signed_step.abs().clamp(1e-12, 0.5),
        Antialiasing::SplineOptimized,
    )
}

/// Exact box average of a saw along a linear unwrapped phase segment.
/// Timestamp is half a sample before the second endpoint. This has a box-filter
/// passband response, not an ideal lowpass, and cannot recover curved trajectories.
#[inline]
pub fn saw_linear_average(previous: f64, current: f64) -> f64 {
    let offset = previous.floor();
    saw_linear_average_rebased(previous - offset, current - offset)
}

/// Fast scalar entry when the previous phase is already wrapped into [0, 1).
#[inline]
pub fn saw_linear_average_rebased(previous: f64, current: f64) -> f64 {
    let turns = current.floor();
    let b = current - turns;
    if turns == 0.0 {
        return b + previous - 1.0;
    }
    ((b - previous) / (current - previous)) * (b + previous - 1.0)
}

/// SIMD equivalent. Inputs should be locally rebased unwrapped phases to avoid
/// precision loss: previous phase in [0, 1), current = previous + signed travel.
#[inline]
pub fn saw_linear_average8(previous: f32x8, current: f32x8) -> f32x8 {
    let delta = current - previous;
    let small = delta.abs().cmp_lt(f32x8::splat(f32::MIN_POSITIVE));
    let safe_delta = small.blend(f32x8::ONE, delta);
    let a = previous - previous.floor();
    let b = current - current.floor();
    let average = ((b - a) / safe_delta) * (b + a - f32x8::ONE);
    let mid = (previous + current) * f32x8::splat(0.5);
    small.blend(
        (mid - mid.floor()) * f32x8::splat(2.0) - f32x8::ONE,
        average,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn analytic_average_matches_piecewise_saw_integral() {
        for (a, b) in [
            (0.2, 0.7),
            (0.8, 1.1),
            (0.1, -0.2),
            (-1.2, 2.8),
            (0.9, -2.4),
            (0.3, 0.3),
        ] {
            let n = 200_000;
            let expected = (0..n)
                .map(|i| {
                    let p = a + (b - a) * (i as f64 + 0.5) / n as f64;
                    2.0 * p.rem_euclid(1.0) - 1.0
                })
                .sum::<f64>()
                / n as f64;
            assert!((saw_linear_average(a, b) - expected).abs() < 2e-5);
            assert!((saw_linear_average(a, b) - saw_linear_average(b, a)).abs() < 1e-12);
            assert!(
                (saw_linear_average(a, b) - saw_linear_average(a + 8.0, b + 8.0)).abs() < 1e-12
            );
        }
    }
    #[test]
    fn simd_agrees_with_scalar_through_wraps_and_reversals() {
        let a = [0.2, 0.8, 0.1, 0.9, 0.3, 0.0, 0.99, 0.01];
        let b = [0.7, 1.1, -0.2, -2.4, 0.3, 2.0, 1.01, -0.01];
        let y = saw_linear_average8(f32x8::from(a), f32x8::from(b)).to_array();
        for i in 0..8 {
            assert!((y[i] as f64 - saw_linear_average(a[i] as f64, b[i] as f64)).abs() < 2e-5);
        }
    }

    #[test]
    fn tiny_crossings_do_not_take_a_midpoint_discontinuity_shortcut() {
        let epsilon = 2.0_f64.powi(-30);
        assert_eq!(saw_linear_average(1.0 - epsilon, 1.0 + epsilon), 0.0);
        let epsilon = 2.0_f32.powi(-20);
        let output = saw_linear_average8(f32x8::splat(1.0 - epsilon), f32x8::splat(1.0 + epsilon));
        assert_eq!(output.to_array(), [0.0; 8]);
    }
}
