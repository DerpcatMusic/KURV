//! Stateless 1x high-note crossover for unwarped canonical saw/triangle.
//!
//! From .20 to .25 cycles/sample, smoothly crossfade the shipping optimized
//! spline to the finite Fourier sum. Above .25 the fundamental is the only
//! allowed harmonic. Saw harmonic two fades during .225..25 before Nyquist.
//! This restores wanted amplitude; it is not a guarantee for FM/PM sidebands.
use super::antialias::{self, Antialiasing};
use truce_simd::simd::{f32x4, f32x8};
use wide::CmpGt;

#[inline]
fn smooth(value: f64) -> f64 {
    let value = value.clamp(0.0, 1.0);
    value * value * (3.0 - 2.0 * value)
}

pub(super) fn saw(phase: f64, step: f64) -> f64 {
    if step <= 0.20 {
        return antialias::bandlimited_saw(phase, step, Antialiasing::SplineOptimized);
    }
    let p = phase as f32;
    let sine = -antialias::aligned_sine_phase(p - 0.25) as f64;
    let gain = smooth((0.5 - step) * 20.0);
    if step >= 0.25 {
        return -std::f64::consts::FRAC_2_PI * sine * gain;
    }
    let cosine = -antialias::aligned_sine_phase(p) as f64;
    let second = smooth((0.25 - step) * 40.0);
    let harmonic = -std::f64::consts::FRAC_2_PI * sine
        - std::f64::consts::FRAC_1_PI * (2.0 * sine * cosine) * second;
    let harmonic = harmonic * smooth((0.5 - step) * 20.0);
    if step >= 0.25 {
        return harmonic;
    }
    let original = antialias::bandlimited_saw(phase, step, Antialiasing::SplineOptimized);
    (harmonic - original).mul_add(smooth((step - 0.20) * 20.0), original)
}

pub(super) fn triangle(phase: f64, step: f64) -> f64 {
    if step <= 0.20 {
        return antialias::bandlimited_triangle(phase, step, Antialiasing::SplineOptimized);
    }
    let harmonic = (8.0 / std::f64::consts::PI.powi(2))
        * antialias::aligned_sine_phase(phase as f32) as f64
        * smooth((0.5 - step) * 20.0);
    if step >= 0.25 {
        return harmonic;
    }
    let original = antialias::bandlimited_triangle(phase, step, Antialiasing::SplineOptimized);
    (harmonic - original).mul_add(smooth((step - 0.20) * 20.0), original)
}

macro_rules! vector {
    ($saw:ident, $triangle:ident, $v:ident, $basis:ident, $old_saw:ident, $old_triangle:ident, $sine:ident, $aligned:ident) => {
        pub(super) fn $saw(phase: $v, step: $v) -> $v {
            if !step.cmp_gt($v::splat(0.20)).any() {
                return antialias::$old_saw(phase, step, Antialiasing::SplineOptimized);
            }
            let smooth = |v: $v| {
                let t = v.fast_max($v::ZERO).fast_min($v::ONE);
                t * t * ($v::splat(3.0) - t * $v::splat(2.0))
            };
            if !($v::splat(0.25).cmp_gt(step)).any() {
                return -$v::splat(std::f32::consts::FRAC_2_PI)
                    * antialias::$sine(phase)
                    * smooth(($v::splat(0.5) - step) * $v::splat(20.0));
            }
            let (sin, cos) = antialias::$basis(phase);
            let second = smooth(($v::splat(0.25) - step) * $v::splat(40.0));
            let harmonic = (-$v::splat(std::f32::consts::FRAC_2_PI) * sin
                - $v::splat(std::f32::consts::FRAC_1_PI) * (sin * cos * $v::splat(2.0)) * second)
                * smooth(($v::splat(0.5) - step) * $v::splat(20.0));
            if !($v::splat(0.25).cmp_gt(step)).any() {
                return harmonic;
            }
            let old = antialias::$old_saw(phase, step, Antialiasing::SplineOptimized);
            (harmonic - old).mul_add(smooth((step - $v::splat(0.20)) * $v::splat(20.0)), old)
        }
        pub(super) fn $triangle(phase: $v, step: $v) -> $v {
            if !step.cmp_gt($v::splat(0.20)).any() {
                return antialias::$old_triangle(phase, step, Antialiasing::SplineOptimized);
            }
            let smooth = |v: $v| {
                let t = v.fast_max($v::ZERO).fast_min($v::ONE);
                t * t * ($v::splat(3.0) - t * $v::splat(2.0))
            };
            let harmonic = $v::splat(8.0 / std::f32::consts::PI.powi(2))
                * antialias::$aligned(phase)
                * smooth(($v::splat(0.5) - step) * $v::splat(20.0));
            if !($v::splat(0.25).cmp_gt(step)).any() {
                return harmonic;
            }
            let old = antialias::$old_triangle(phase, step, Antialiasing::SplineOptimized);
            (harmonic - old).mul_add(smooth((step - $v::splat(0.20)) * $v::splat(20.0)), old)
        }
    };
}
vector!(
    saw4,
    triangle4,
    f32x4,
    sine_cosine_phase4,
    bandlimited_saw4,
    bandlimited_triangle4,
    sine_phase4,
    aligned_sine_phase4
);
vector!(
    saw8,
    triangle8,
    f32x8,
    sine_cosine_phase8,
    bandlimited_saw8,
    bandlimited_triangle8,
    sine_phase8,
    aligned_sine_phase8
);
