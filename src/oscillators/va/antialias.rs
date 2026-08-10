use truce_simd::simd::{f32x4, f32x8};
use wide::{CmpGt, CmpLt};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Antialiasing {
    #[cfg(test)]
    Legacy,
    #[default]
    Spline,
    SplineOptimized,
    #[cfg(test)]
    Lagrange,
    #[cfg(test)]
    Spectral,
}

impl Antialiasing {
    pub const fn for_factor(self, factor: u8) -> Self {
        let _ = self;
        if factor <= 2 {
            Self::SplineOptimized
        } else {
            Self::Spline
        }
    }
}

#[inline]
pub(super) fn wrap_phase4(phase: f32x4) -> f32x4 {
    phase.cmp_lt(f32x4::ONE).blend(phase, phase - f32x4::ONE)
}

#[inline]
pub(super) fn wrap_phase8(phase: f32x8) -> f32x8 {
    phase.cmp_lt(f32x8::ONE).blend(phase, phase - f32x8::ONE)
}

#[inline]
pub(super) fn cosine_phase4(phase: f32x4) -> f32x4 {
    sine_phase4(wrap_phase4(phase + f32x4::splat(0.25)))
}

#[inline]
pub(super) fn cosine_phase8(phase: f32x8) -> f32x8 {
    sine_phase8(wrap_phase8(phase + f32x8::splat(0.25)))
}

#[inline]
pub(super) fn aligned_sine_phase4(phase: f32x4) -> f32x4 {
    -cosine_phase4(phase)
}

#[inline]
pub(super) fn aligned_sine_phase8(phase: f32x8) -> f32x8 {
    -cosine_phase8(phase)
}

#[inline]
pub(super) fn aligned_sine_phase(phase: f32) -> f32 {
    let shifted = phase + 0.25;
    let shifted = if shifted >= 1.0 {
        shifted - 1.0
    } else {
        shifted
    };
    let folded = 0.25 - ((shifted - 0.5).abs() - 0.25).abs();
    let sine = sine_polynomial(folded);
    if shifted > 0.5 { sine } else { -sine }
}

#[inline]
pub(super) fn sine_cosine_phase4(phase: f32x4) -> (f32x4, f32x4) {
    let half = f32x4::splat(0.5);
    let quarter = f32x4::splat(0.25);
    let folded = quarter - ((phase - half).abs() - quarter).abs();
    let sine = sine_polynomial4(folded);
    let cosine = sine_polynomial4(quarter - folded);
    (
        phase.cmp_gt(half).blend(-sine, sine),
        (phase.cmp_gt(quarter) & phase.cmp_lt(f32x4::splat(0.75))).blend(-cosine, cosine),
    )
}

#[inline]
pub(super) fn sine_cosine_phase8(phase: f32x8) -> (f32x8, f32x8) {
    let half = f32x8::splat(0.5);
    let quarter = f32x8::splat(0.25);
    let folded = quarter - ((phase - half).abs() - quarter).abs();
    let sine = sine_polynomial8(folded);
    let cosine = sine_polynomial8(quarter - folded);
    (
        phase.cmp_gt(half).blend(-sine, sine),
        (phase.cmp_gt(quarter) & phase.cmp_lt(f32x8::splat(0.75))).blend(-cosine, cosine),
    )
}

#[inline]
fn sine_polynomial4(folded: f32x4) -> f32x4 {
    let folded2 = folded * folded;
    let folded4 = folded2 * folded2;
    let low = f32x4::splat(-41.341_7).mul_add(folded2, f32x4::splat(std::f32::consts::TAU));
    let middle = f32x4::splat(-76.705_86).mul_add(folded2, f32x4::splat(81.605_25));
    let high = f32x4::splat(-15.094_643).mul_add(folded2, f32x4::splat(42.058_693));
    folded * high.mul_add(folded4, middle).mul_add(folded4, low)
}

#[inline]
fn sine_polynomial8(folded: f32x8) -> f32x8 {
    let folded2 = folded * folded;
    let folded4 = folded2 * folded2;
    let low = f32x8::splat(-41.341_7).mul_add(folded2, f32x8::splat(std::f32::consts::TAU));
    let middle = f32x8::splat(-76.705_86).mul_add(folded2, f32x8::splat(81.605_25));
    let high = f32x8::splat(-15.094_643).mul_add(folded2, f32x8::splat(42.058_693));
    folded * high.mul_add(folded4, middle).mul_add(folded4, low)
}

#[inline]
fn sine_polynomial(folded: f32) -> f32 {
    let folded2 = folded * folded;
    let folded4 = folded2 * folded2;
    let low = (-41.341_7_f32).mul_add(folded2, std::f32::consts::TAU);
    let middle = (-76.705_86_f32).mul_add(folded2, 81.605_25);
    let high = (-15.094_643_f32).mul_add(folded2, 42.058_693);
    folded * high.mul_add(folded4, middle).mul_add(folded4, low)
}

pub(super) fn sine_phase4(phase: f32x4) -> f32x4 {
    let half = f32x4::splat(0.5);
    let quarter = f32x4::splat(0.25);
    let folded = quarter - ((phase - half).abs() - quarter).abs();
    let sine = sine_polynomial4(folded);
    phase.cmp_gt(half).blend(-sine, sine)
}

pub(super) fn sine_phase8(phase: f32x8) -> f32x8 {
    let half = f32x8::splat(0.5);
    let quarter = f32x8::splat(0.25);
    let folded = quarter - ((phase - half).abs() - quarter).abs();
    let sine = sine_polynomial8(folded);
    phase.cmp_gt(half).blend(-sine, sine)
}

pub(super) fn bandlimited_triangle(phase: f64, phase_step: f64, antialiasing: Antialiasing) -> f64 {
    let sample = (-4.0_f64).mul_add((phase - 0.5).abs(), 1.0);
    let peak_phase = wrap01(phase + 0.5);
    let optimized = antialiasing == Antialiasing::SplineOptimized;
    let correction = spline_blamp(phase, phase_step, optimized)
        - spline_blamp(peak_phase, phase_step, optimized);
    (8.0 * phase_step).mul_add(correction, sample)
}

pub(super) fn bandlimited_triangle4(
    phase: f32x4,
    phase_step: f32x4,
    antialiasing: Antialiasing,
) -> f32x4 {
    let half = f32x4::splat(0.5);
    let sample = (phase - half).abs() * f32x4::splat(-4.0) + f32x4::ONE;
    let shifted = phase + half;
    let peak_phase = shifted
        .cmp_lt(f32x4::ONE)
        .blend(shifted, shifted - f32x4::ONE);
    let optimized = antialiasing == Antialiasing::SplineOptimized;
    let correction = spline_blamp4(phase, phase_step, optimized)
        - spline_blamp4(peak_phase, phase_step, optimized);
    (phase_step * f32x4::splat(8.0)).mul_add(correction, sample)
}

pub(super) fn bandlimited_triangle8(
    phase: f32x8,
    phase_step: f32x8,
    antialiasing: Antialiasing,
) -> f32x8 {
    let half = f32x8::splat(0.5);
    let sample = (phase - half).abs() * f32x8::splat(-4.0) + f32x8::ONE;
    let shifted = phase + half;
    let peak_phase = shifted
        .cmp_lt(f32x8::ONE)
        .blend(shifted, shifted - f32x8::ONE);
    let optimized = antialiasing == Antialiasing::SplineOptimized;
    let correction = spline_blamp8(phase, phase_step, optimized)
        - spline_blamp8(peak_phase, phase_step, optimized);
    (phase_step * f32x8::splat(8.0)).mul_add(correction, sample)
}

pub(super) fn bandlimited_saw(phase: f64, phase_step: f64, antialiasing: Antialiasing) -> f64 {
    2.0_f64.mul_add(phase, -1.0) - edge_blep(phase, phase_step, antialiasing)
}

pub(super) fn edge_blep(phase: f64, phase_step: f64, antialiasing: Antialiasing) -> f64 {
    spline_blep(
        phase,
        phase_step,
        antialiasing == Antialiasing::SplineOptimized,
    )
}

pub(super) fn bandlimited_saw4(
    phase: f32x4,
    phase_step: f32x4,
    antialiasing: Antialiasing,
) -> f32x4 {
    phase * f32x4::splat(2.0) - f32x4::ONE - edge_blep4(phase, phase_step, antialiasing)
}

pub(super) fn edge_blep4(phase: f32x4, phase_step: f32x4, antialiasing: Antialiasing) -> f32x4 {
    spline_blep4(
        phase,
        phase_step,
        antialiasing == Antialiasing::SplineOptimized,
    )
}

pub(super) fn bandlimited_saw8(
    phase: f32x8,
    phase_step: f32x8,
    antialiasing: Antialiasing,
) -> f32x8 {
    phase * f32x8::splat(2.0) - f32x8::ONE - edge_blep8(phase, phase_step, antialiasing)
}

pub(super) fn edge_blep8(phase: f32x8, phase_step: f32x8, antialiasing: Antialiasing) -> f32x8 {
    spline_blep8(
        phase,
        phase_step,
        antialiasing == Antialiasing::SplineOptimized,
    )
}

pub(super) fn bandlimited_saw_pulse_morph4(
    phase: f32x4,
    phase_step: f32x4,
    pulse_width: f32,
    blend: f32,
    antialiasing: Antialiasing,
) -> f32x4 {
    let one = f32x4::ONE;
    let blend = f32x4::splat(blend);
    let width = phase_step
        .fast_max(f32x4::splat(pulse_width.clamp(0.03, 0.97)))
        .fast_min(one - phase_step);
    let saw = phase * f32x4::splat(2.0) - one;
    let pulse = phase.cmp_lt(width).blend(one, f32x4::splat(-1.0));
    let shifted = phase + one - width;
    let shifted = shifted.cmp_lt(one).blend(shifted, shifted - one);
    let wrap_correction = edge_blep4(phase, phase_step, antialiasing);
    let width_correction = edge_blep4(shifted, phase_step, antialiasing);
    let raw = (pulse - saw).mul_add(blend, saw);
    let correction =
        (blend * f32x4::splat(2.0) - one).mul_add(wrap_correction, -(blend * width_correction));
    raw + correction
}

pub(super) fn bandlimited_saw_pulse_morph8(
    phase: f32x8,
    phase_step: f32x8,
    pulse_width: f32,
    blend: f32,
    antialiasing: Antialiasing,
) -> f32x8 {
    let one = f32x8::ONE;
    let blend = f32x8::splat(blend);
    let width = phase_step
        .fast_max(f32x8::splat(pulse_width.clamp(0.03, 0.97)))
        .fast_min(one - phase_step);
    let saw = phase * f32x8::splat(2.0) - one;
    let pulse = phase.cmp_lt(width).blend(one, f32x8::splat(-1.0));
    let shifted = phase + one - width;
    let shifted = shifted.cmp_lt(one).blend(shifted, shifted - one);
    let wrap_correction = edge_blep8(phase, phase_step, antialiasing);
    let width_correction = edge_blep8(shifted, phase_step, antialiasing);
    let raw = (pulse - saw).mul_add(blend, saw);
    let correction =
        (blend * f32x8::splat(2.0) - one).mul_add(wrap_correction, -(blend * width_correction));
    raw + correction
}

pub(super) fn bandlimited_pulse(
    phase: f64,
    phase_step: f64,
    pulse_width: f64,
    antialiasing: Antialiasing,
) -> f64 {
    let minimum_width = phase_step.max(0.03);
    let width = pulse_width.clamp(minimum_width, 1.0 - minimum_width);
    let mut sample = if phase < width { 1.0 } else { -1.0 };
    let shifted_phase = phase + 1.0 - width;
    let shifted_phase = if shifted_phase >= 1.0 {
        shifted_phase - 1.0
    } else {
        shifted_phase
    };
    sample += edge_blep(phase, phase_step, antialiasing);
    sample -= edge_blep(shifted_phase, phase_step, antialiasing);
    sample
}

pub(super) fn bandlimited_pulse4(
    phase: f32x4,
    phase_step: f32x4,
    pulse_width: f32,
    antialiasing: Antialiasing,
) -> f32x4 {
    let one = f32x4::ONE;
    let width = phase_step
        .fast_max(f32x4::splat(pulse_width.clamp(0.03, 0.97)))
        .fast_min(one - phase_step);
    let sample = phase.cmp_lt(width).blend(one, f32x4::splat(-1.0));
    let shifted = phase + one - width;
    let shifted = shifted.cmp_lt(one).blend(shifted, shifted - one);
    sample + edge_blep4(phase, phase_step, antialiasing)
        - edge_blep4(shifted, phase_step, antialiasing)
}

pub(super) fn bandlimited_pulse8(
    phase: f32x8,
    phase_step: f32x8,
    pulse_width: f32,
    antialiasing: Antialiasing,
) -> f32x8 {
    let one = f32x8::ONE;
    let width = phase_step
        .fast_max(f32x8::splat(pulse_width.clamp(0.03, 0.97)))
        .fast_min(one - phase_step);
    let sample = phase.cmp_lt(width).blend(one, f32x8::splat(-1.0));
    let shifted = phase + one - width;
    let shifted = shifted.cmp_lt(one).blend(shifted, shifted - one);
    sample + edge_blep8(phase, phase_step, antialiasing)
        - edge_blep8(shifted, phase_step, antialiasing)
}

fn spline_blep(phase: f64, phase_step: f64, optimized: bool) -> f64 {
    if phase_step <= f64::EPSILON {
        return 0.0;
    }
    let support = 2.0 * phase_step;
    if support < 0.5 && phase >= support && phase <= 1.0 - support {
        return 0.0;
    }
    let inverse_step = phase_step.recip();
    let residual = if optimized {
        optimized_cubic_blep_residual
    } else {
        cubic_blep_residual
    };
    if support < 0.5 {
        let nearest_edge = if phase < 0.5 { phase } else { phase - 1.0 };
        2.0 * residual(nearest_edge * inverse_step)
    } else {
        2.0 * (residual(phase * inverse_step) + residual((phase - 1.0) * inverse_step))
    }
}

fn cubic_blep_residual(position: f64) -> f64 {
    let distance = position.abs();
    if distance >= 2.0 {
        return 0.0;
    }
    let residual = if distance < 1.0 {
        let inner = 0.125_f64.mul_add(distance, -1.0 / 3.0) * distance;
        inner.mul_add(distance, 2.0 / 3.0).mul_add(distance, -0.5)
    } else {
        let tail = 2.0 - distance;
        -(tail * tail * tail * tail) / 24.0
    };
    if position < 0.0 { -residual } else { residual }
}

fn optimized_cubic_blep_residual(position: f64) -> f64 {
    let distance = position.abs();
    if distance >= 2.0 {
        return 0.0;
    }
    let residual = if distance < 1.0 {
        let inner = 0.116_560_557_324_044_6_f64.mul_add(distance, -0.316_694_721_754_637_4);
        let inner = inner.mul_add(distance, 0.024_084_598_590_023_5);
        inner
            .mul_add(distance, 0.623_499_608_339_861)
            .mul_add(distance, -0.5)
    } else {
        let tail = 2.0 - distance;
        let outer = (-0.038_711_854_802_419_96_f64).mul_add(tail, -0.006_173_230_446_159_231);
        let outer = outer.mul_add(tail, -0.007_354_877_418_709_688);
        outer.mul_add(tail, -0.000_309_994_833_419_443) * tail
    };
    if position < 0.0 { -residual } else { residual }
}

fn spline_blamp(phase: f64, phase_step: f64, optimized: bool) -> f64 {
    if phase_step <= f64::EPSILON {
        return 0.0;
    }
    let support = 2.0 * phase_step;
    if support < 0.5 && phase >= support && phase <= 1.0 - support {
        return 0.0;
    }
    let inverse_step = phase_step.recip();
    let residual = if optimized {
        optimized_cubic_blamp_residual
    } else {
        cubic_blamp_residual
    };
    residual(phase * inverse_step) + residual((phase - 1.0) * inverse_step)
}

fn cubic_blamp_residual(position: f64) -> f64 {
    let distance = position.abs();
    if distance >= 2.0 {
        return 0.0;
    }
    if distance < 1.0 {
        let squared = distance * distance;
        let fourth = squared * squared;
        let fifth = fourth * distance;
        fifth / 40.0 - fourth / 12.0 + squared / 3.0 - distance / 2.0 + 7.0 / 30.0
    } else {
        let tail = 2.0 - distance;
        let squared = tail * tail;
        squared * squared * tail / 120.0
    }
}

fn optimized_cubic_blamp_residual(position: f64) -> f64 {
    let distance = position.abs();
    if distance >= 2.0 {
        return 0.0;
    }
    if distance < 1.0 {
        let inner = 0.023_312_111_464_808_92_f64.mul_add(distance, -0.079_173_680_438_659_36);
        let inner = inner.mul_add(distance, 0.008_028_199_530_007_833);
        let inner = inner.mul_add(distance, 0.311_749_804_169_930_5);
        inner
            .mul_add(distance, -0.5)
            .mul_add(distance, 0.247_975_867_068_882_2)
    } else {
        let tail = 2.0 - distance;
        let outer = 0.007_742_370_960_483_992_f64.mul_add(tail, 0.001_543_307_611_539_807_7);
        let outer = outer.mul_add(tail, 0.002_451_625_806_236_563);
        outer.mul_add(tail, 0.000_154_997_416_709_721_5) * tail * tail
    }
}

fn spline_blep4(phase: f32x4, phase_step: f32x4, optimized: bool) -> f32x4 {
    let zero = f32x4::ZERO;
    let one = f32x4::ONE;
    let active = phase_step.cmp_gt(f32x4::splat(f32::EPSILON));
    let support = phase_step * f32x4::splat(2.0);
    let event = active & (phase.cmp_lt(support) | phase.cmp_gt(one - support));
    if !event.any() {
        return zero;
    }
    let safe_step = event.blend(phase_step, one);
    let inverse_step = one / safe_step;
    let narrow = support.cmp_lt(f32x4::splat(0.5)).all();
    let correction = if narrow {
        let position = phase.cmp_lt(f32x4::splat(0.5)).blend(phase, phase - one) * inverse_step;
        if optimized {
            optimized_cubic_blep_residual4(position)
        } else {
            cubic_blep_residual4(position)
        }
    } else if optimized {
        optimized_cubic_blep_residual4(phase * inverse_step)
            + optimized_cubic_blep_residual4((phase - one) * inverse_step)
    } else {
        cubic_blep_residual4(phase * inverse_step)
            + cubic_blep_residual4((phase - one) * inverse_step)
    } * f32x4::splat(2.0);
    event.blend(correction, zero)
}

pub(super) fn spline_blep4_precomputed(
    phase: f32x4,
    active: f32x4,
    support: f32x4,
    inverse_step: f32x4,
    optimized: bool,
) -> f32x4 {
    let zero = f32x4::ZERO;
    let one = f32x4::ONE;
    let event = active & (phase.cmp_lt(support) | phase.cmp_gt(one - support));
    if !event.any() {
        return zero;
    }
    let narrow = support.cmp_lt(f32x4::splat(0.5)).all();
    let correction = if narrow {
        let position = phase.cmp_lt(f32x4::splat(0.5)).blend(phase, phase - one) * inverse_step;
        if optimized {
            optimized_cubic_blep_residual4(position)
        } else {
            cubic_blep_residual4(position)
        }
    } else if optimized {
        optimized_cubic_blep_residual4(phase * inverse_step)
            + optimized_cubic_blep_residual4((phase - one) * inverse_step)
    } else {
        cubic_blep_residual4(phase * inverse_step)
            + cubic_blep_residual4((phase - one) * inverse_step)
    } * f32x4::splat(2.0);
    event.blend(correction, zero)
}

fn cubic_blep_residual4(position: f32x4) -> f32x4 {
    let zero = f32x4::ZERO;
    let distance = position.abs();
    let inner = (distance * f32x4::splat(0.125) - f32x4::splat(1.0 / 3.0)) * distance;
    let inner = (inner * distance + f32x4::splat(2.0 / 3.0)) * distance - f32x4::splat(0.5);
    let tail = f32x4::splat(2.0) - distance;
    let tail_squared = tail * tail;
    let outer = -(tail_squared * tail_squared) * f32x4::splat(1.0 / 24.0);
    let residual = distance.cmp_lt(f32x4::ONE).blend(inner, outer);
    let residual = distance.cmp_lt(f32x4::splat(2.0)).blend(residual, zero);
    position.cmp_lt(zero).blend(-residual, residual)
}

fn optimized_cubic_blep_residual4(position: f32x4) -> f32x4 {
    let zero = f32x4::ZERO;
    let distance = position.abs();
    let inner = f32x4::splat(0.116_560_56).mul_add(distance, f32x4::splat(-0.316_694_7));
    let inner = inner.mul_add(distance, f32x4::splat(0.024_084_598));
    let inner = inner
        .mul_add(distance, f32x4::splat(0.623_499_63))
        .mul_add(distance, f32x4::splat(-0.5));
    let tail = f32x4::splat(2.0) - distance;
    let outer = f32x4::splat(-0.038_711_853).mul_add(tail, f32x4::splat(-0.006_173_230_2));
    let outer = outer.mul_add(tail, f32x4::splat(-0.007_354_877_4));
    let outer = outer.mul_add(tail, f32x4::splat(-0.000_309_994_82)) * tail;
    let residual = distance.cmp_lt(f32x4::ONE).blend(inner, outer);
    let residual = distance.cmp_lt(f32x4::splat(2.0)).blend(residual, zero);
    position.cmp_lt(zero).blend(-residual, residual)
}

fn spline_blamp4(phase: f32x4, phase_step: f32x4, optimized: bool) -> f32x4 {
    let zero = f32x4::ZERO;
    let one = f32x4::ONE;
    let active = phase_step.cmp_gt(f32x4::splat(f32::EPSILON));
    let support = phase_step * f32x4::splat(2.0);
    let event = active & (phase.cmp_lt(support) | phase.cmp_gt(one - support));
    if !event.any() {
        return zero;
    }
    let safe_step = event.blend(phase_step, one);
    let inverse_step = one / safe_step;
    let correction = if optimized {
        optimized_cubic_blamp_residual4(phase * inverse_step)
            + optimized_cubic_blamp_residual4((phase - one) * inverse_step)
    } else {
        cubic_blamp_residual4(phase * inverse_step)
            + cubic_blamp_residual4((phase - one) * inverse_step)
    };
    event.blend(correction, zero)
}

fn spline_blamp4_precomputed(
    phase: f32x4,
    active: f32x4,
    support: f32x4,
    inverse_step: f32x4,
    optimized: bool,
) -> f32x4 {
    let zero = f32x4::ZERO;
    let one = f32x4::ONE;
    let event = active & (phase.cmp_lt(support) | phase.cmp_gt(one - support));
    if !event.any() {
        return zero;
    }
    let correction = if optimized {
        optimized_cubic_blamp_residual4(phase * inverse_step)
            + optimized_cubic_blamp_residual4((phase - one) * inverse_step)
    } else {
        cubic_blamp_residual4(phase * inverse_step)
            + cubic_blamp_residual4((phase - one) * inverse_step)
    };
    event.blend(correction, zero)
}

pub(super) fn spline_triangle4_precomputed(
    phase: f32x4,
    phase_step: f32x4,
    active: f32x4,
    support: f32x4,
    inverse_step: f32x4,
    optimized: bool,
) -> f32x4 {
    let half = f32x4::splat(0.5);
    let sample = (phase - half).abs() * f32x4::splat(-4.0) + f32x4::ONE;
    if support.cmp_lt(f32x4::splat(0.25)).all() {
        let one = f32x4::ONE;
        let wrap_event = phase.cmp_lt(support) | phase.cmp_gt(one - support);
        let peak_distance = (phase - half).abs();
        let peak_event = peak_distance.cmp_lt(support);
        let event = active & (wrap_event | peak_event);
        if !event.any() {
            return sample;
        }
        let wrap_position = phase.cmp_lt(half).blend(phase, phase - one);
        let position = wrap_event.blend(wrap_position, phase - half) * inverse_step;
        let correction = if optimized {
            optimized_cubic_blamp_residual4(position)
        } else {
            cubic_blamp_residual4(position)
        };
        let correction = peak_event.blend(-correction, correction);
        return (phase_step * f32x4::splat(8.0))
            .mul_add(event.blend(correction, f32x4::ZERO), sample);
    }
    let peak_phase = wrap_phase4(phase + half);
    let correction = spline_blamp4_precomputed(phase, active, support, inverse_step, optimized)
        - spline_blamp4_precomputed(peak_phase, active, support, inverse_step, optimized);
    (phase_step * f32x4::splat(8.0)).mul_add(correction, sample)
}

fn cubic_blamp_residual4(position: f32x4) -> f32x4 {
    let zero = f32x4::ZERO;
    let distance = position.abs();
    let squared = distance * distance;
    let fourth = squared * squared;
    let fifth = fourth * distance;
    let inner = fifth * f32x4::splat(1.0 / 40.0) - fourth * f32x4::splat(1.0 / 12.0)
        + squared * f32x4::splat(1.0 / 3.0)
        - distance * f32x4::splat(0.5)
        + f32x4::splat(7.0 / 30.0);
    let tail = f32x4::splat(2.0) - distance;
    let tail_squared = tail * tail;
    let outer = tail_squared * tail_squared * tail * f32x4::splat(1.0 / 120.0);
    let residual = distance.cmp_lt(f32x4::ONE).blend(inner, outer);
    distance.cmp_lt(f32x4::splat(2.0)).blend(residual, zero)
}

fn optimized_cubic_blamp_residual4(position: f32x4) -> f32x4 {
    let zero = f32x4::ZERO;
    let distance = position.abs();
    let inner = f32x4::splat(0.023_312_11).mul_add(distance, f32x4::splat(-0.079_173_68));
    let inner = inner.mul_add(distance, f32x4::splat(0.008_028_2));
    let inner = inner.mul_add(distance, f32x4::splat(0.311_749_82));
    let inner = inner
        .mul_add(distance, f32x4::splat(-0.5))
        .mul_add(distance, f32x4::splat(0.247_975_87));
    let tail = f32x4::splat(2.0) - distance;
    let outer = f32x4::splat(0.007_742_371).mul_add(tail, f32x4::splat(0.001_543_307_7));
    let outer = outer.mul_add(tail, f32x4::splat(0.002_451_625_8));
    let outer = outer.mul_add(tail, f32x4::splat(0.000_154_997_42)) * tail * tail;
    let residual = distance.cmp_lt(f32x4::ONE).blend(inner, outer);
    distance.cmp_lt(f32x4::splat(2.0)).blend(residual, zero)
}

fn spline_blep8(phase: f32x8, phase_step: f32x8, optimized: bool) -> f32x8 {
    let zero = f32x8::ZERO;
    let one = f32x8::ONE;
    let active = phase_step.cmp_gt(f32x8::splat(f32::EPSILON));
    let support = phase_step * f32x8::splat(2.0);
    let event = active & (phase.cmp_lt(support) | phase.cmp_gt(one - support));
    if !event.any() {
        return zero;
    }
    let safe_step = event.blend(phase_step, one);
    let inverse_step = one / safe_step;
    let narrow = support.cmp_lt(f32x8::splat(0.5)).all();
    let correction = if narrow {
        let position = phase.cmp_lt(f32x8::splat(0.5)).blend(phase, phase - one) * inverse_step;
        if optimized {
            optimized_cubic_blep_residual8(position, event)
        } else {
            cubic_blep_residual8(position)
        }
    } else if optimized {
        optimized_cubic_blep_residual8(phase * inverse_step, event)
            + optimized_cubic_blep_residual8((phase - one) * inverse_step, event)
    } else {
        cubic_blep_residual8(phase * inverse_step)
            + cubic_blep_residual8((phase - one) * inverse_step)
    } * f32x8::splat(2.0);
    event.blend(correction, zero)
}

#[inline]
pub(super) fn spline_saw8_narrow(phase: f32x8, phase_step: f32x8, optimized: bool) -> f32x8 {
    let zero = f32x8::ZERO;
    let one = f32x8::ONE;
    let support = phase_step * f32x8::splat(2.0);
    let event = phase.cmp_lt(support) | phase.cmp_gt(one - support);
    let correction = if event.any() {
        let inverse_step = one / event.blend(phase_step, one);
        let position = phase.cmp_lt(f32x8::splat(0.5)).blend(phase, phase - one) * inverse_step;
        let residual = if optimized {
            optimized_cubic_blep_residual8(position, event)
        } else {
            cubic_blep_residual8(position)
        };
        event.blend(residual, zero) * f32x8::splat(2.0)
    } else {
        zero
    };
    phase * f32x8::splat(2.0) - one - correction
}

#[inline]
pub(super) fn spline_blep8_precomputed(
    phase: f32x8,
    active: f32x8,
    support: f32x8,
    inverse_step: f32x8,
    optimized: bool,
) -> f32x8 {
    if optimized {
        spline_blep8_precomputed_static::<true>(phase, active, support, inverse_step)
    } else {
        spline_blep8_precomputed_static::<false>(phase, active, support, inverse_step)
    }
}

#[inline(always)]
fn spline_blep8_precomputed_static<const OPTIMIZED: bool>(
    phase: f32x8,
    active: f32x8,
    support: f32x8,
    inverse_step: f32x8,
) -> f32x8 {
    let narrow = support.cmp_lt(f32x8::splat(0.5)).all();
    spline_blep8_precomputed_static_with_bounds::<OPTIMIZED>(
        phase,
        active,
        support,
        f32x8::ONE - support,
        inverse_step,
        narrow,
    )
}

#[inline(always)]
pub(super) fn spline_blep8_precomputed_static_with_bounds<const OPTIMIZED: bool>(
    phase: f32x8,
    active: f32x8,
    support: f32x8,
    one_minus_support: f32x8,
    inverse_step: f32x8,
    narrow: bool,
) -> f32x8 {
    let zero = f32x8::ZERO;
    let one = f32x8::ONE;
    let event = active & (phase.cmp_lt(support) | phase.cmp_gt(one_minus_support));
    if !event.any() {
        return zero;
    }
    let correction = if narrow {
        let position = phase.cmp_lt(f32x8::splat(0.5)).blend(phase, phase - one) * inverse_step;
        if OPTIMIZED {
            optimized_cubic_blep_residual8(position, event)
        } else {
            cubic_blep_residual8(position)
        }
    } else if OPTIMIZED {
        optimized_cubic_blep_residual8(phase * inverse_step, event)
            + optimized_cubic_blep_residual8((phase - one) * inverse_step, event)
    } else {
        cubic_blep_residual8(phase * inverse_step)
            + cubic_blep_residual8((phase - one) * inverse_step)
    } * f32x8::splat(2.0);
    event.blend(correction, zero)
}

fn cubic_blep_residual8(position: f32x8) -> f32x8 {
    let zero = f32x8::ZERO;
    let distance = position.abs();
    let inner = (distance * f32x8::splat(0.125) - f32x8::splat(1.0 / 3.0)) * distance;
    let inner = (inner * distance + f32x8::splat(2.0 / 3.0)) * distance - f32x8::splat(0.5);
    let tail = f32x8::splat(2.0) - distance;
    let tail_squared = tail * tail;
    let outer = -(tail_squared * tail_squared) * f32x8::splat(1.0 / 24.0);
    let residual = distance.cmp_lt(f32x8::ONE).blend(inner, outer);
    let residual = distance.cmp_lt(f32x8::splat(2.0)).blend(residual, zero);
    position.cmp_lt(zero).blend(-residual, residual)
}

fn optimized_cubic_blep_residual8(position: f32x8, event: f32x8) -> f32x8 {
    let zero = f32x8::ZERO;
    let distance = position.abs();
    let inside = event & distance.cmp_lt(f32x8::splat(2.0));
    let inner_lanes = inside & distance.cmp_lt(f32x8::ONE);
    let outer_lanes = inside & !inner_lanes;
    let mut residual = zero;
    if inner_lanes.any() {
        let inner = f32x8::splat(0.116_560_56)
            .mul_add(distance, f32x8::splat(-0.316_694_7))
            .mul_add(distance, f32x8::splat(0.024_084_598))
            .mul_add(distance, f32x8::splat(0.623_499_63))
            .mul_add(distance, f32x8::splat(-0.5));
        residual = inner_lanes.blend(inner, residual);
    }
    if outer_lanes.any() {
        let tail = f32x8::splat(2.0) - distance;
        let outer = f32x8::splat(-0.038_711_853)
            .mul_add(tail, f32x8::splat(-0.006_173_230_2))
            .mul_add(tail, f32x8::splat(-0.007_354_877_4))
            .mul_add(tail, f32x8::splat(-0.000_309_994_82))
            * tail;
        residual = outer_lanes.blend(outer, residual);
    }
    position.cmp_lt(zero).blend(-residual, residual)
}

fn spline_blamp8(phase: f32x8, phase_step: f32x8, optimized: bool) -> f32x8 {
    let zero = f32x8::ZERO;
    let one = f32x8::ONE;
    let active = phase_step.cmp_gt(f32x8::splat(f32::EPSILON));
    let support = phase_step * f32x8::splat(2.0);
    let event = active & (phase.cmp_lt(support) | phase.cmp_gt(one - support));
    if !event.any() {
        return zero;
    }
    let safe_step = event.blend(phase_step, one);
    let inverse_step = one / safe_step;
    let correction = if optimized {
        optimized_cubic_blamp_residual8(phase * inverse_step, event)
            + optimized_cubic_blamp_residual8((phase - one) * inverse_step, event)
    } else {
        cubic_blamp_residual8(phase * inverse_step)
            + cubic_blamp_residual8((phase - one) * inverse_step)
    };
    event.blend(correction, zero)
}

#[inline]
fn spline_blamp8_precomputed(
    phase: f32x8,
    active: f32x8,
    support: f32x8,
    inverse_step: f32x8,
    optimized: bool,
) -> f32x8 {
    let zero = f32x8::ZERO;
    let one = f32x8::ONE;
    let event = active & (phase.cmp_lt(support) | phase.cmp_gt(one - support));
    if !event.any() {
        return zero;
    }
    let correction = if optimized {
        optimized_cubic_blamp_residual8(phase * inverse_step, event)
            + optimized_cubic_blamp_residual8((phase - one) * inverse_step, event)
    } else {
        cubic_blamp_residual8(phase * inverse_step)
            + cubic_blamp_residual8((phase - one) * inverse_step)
    };
    event.blend(correction, zero)
}

#[inline]
pub(super) fn spline_triangle8_precomputed(
    phase: f32x8,
    phase_step: f32x8,
    active: f32x8,
    support: f32x8,
    inverse_step: f32x8,
    optimized: bool,
) -> f32x8 {
    let half = f32x8::splat(0.5);
    let peak_distance = (phase - half).abs();
    let sample = peak_distance * f32x8::splat(-4.0) + f32x8::ONE;
    if support.cmp_lt(f32x8::splat(0.25)).all() {
        let peak_corner = peak_distance.cmp_lt(f32x8::splat(0.25));
        let corner_distance = peak_corner.blend(peak_distance, half - peak_distance);
        let event = active & corner_distance.cmp_lt(support);
        if !event.any() {
            return sample;
        }
        let position = corner_distance * inverse_step;
        let correction = if optimized {
            optimized_cubic_blamp_residual8(position, event)
        } else {
            cubic_blamp_residual8(position)
        };
        let correction = peak_corner.blend(-correction, correction);
        return (phase_step * f32x8::splat(8.0))
            .mul_add(event.blend(correction, f32x8::ZERO), sample);
    }
    let peak_phase = wrap_phase8(phase + half);
    let correction = spline_blamp8_precomputed(phase, active, support, inverse_step, optimized)
        - spline_blamp8_precomputed(peak_phase, active, support, inverse_step, optimized);
    (phase_step * f32x8::splat(8.0)).mul_add(correction, sample)
}

fn cubic_blamp_residual8(position: f32x8) -> f32x8 {
    let zero = f32x8::ZERO;
    let distance = position.abs();
    let squared = distance * distance;
    let fourth = squared * squared;
    let fifth = fourth * distance;
    let inner = fifth * f32x8::splat(1.0 / 40.0) - fourth * f32x8::splat(1.0 / 12.0)
        + squared * f32x8::splat(1.0 / 3.0)
        - distance * f32x8::splat(0.5)
        + f32x8::splat(7.0 / 30.0);
    let tail = f32x8::splat(2.0) - distance;
    let tail_squared = tail * tail;
    let outer = tail_squared * tail_squared * tail * f32x8::splat(1.0 / 120.0);
    let residual = distance.cmp_lt(f32x8::ONE).blend(inner, outer);
    distance.cmp_lt(f32x8::splat(2.0)).blend(residual, zero)
}

fn optimized_cubic_blamp_residual8(position: f32x8, event: f32x8) -> f32x8 {
    // Sparse event branches win with AVX2 but regress the two-SSE portable path.
    #[cfg(not(target_feature = "avx2"))]
    {
        let _ = event;
        let zero = f32x8::ZERO;
        let distance = position.abs();
        let inner = f32x8::splat(0.023_312_11)
            .mul_add(distance, f32x8::splat(-0.079_173_68))
            .mul_add(distance, f32x8::splat(0.008_028_2))
            .mul_add(distance, f32x8::splat(0.311_749_82))
            .mul_add(distance, f32x8::splat(-0.5))
            .mul_add(distance, f32x8::splat(0.247_975_87));
        let tail = f32x8::splat(2.0) - distance;
        let outer = f32x8::splat(0.007_742_371)
            .mul_add(tail, f32x8::splat(0.001_543_307_7))
            .mul_add(tail, f32x8::splat(0.002_451_625_8))
            .mul_add(tail, f32x8::splat(0.000_154_997_42))
            * tail
            * tail;
        let residual = distance.cmp_lt(f32x8::ONE).blend(inner, outer);
        return distance.cmp_lt(f32x8::splat(2.0)).blend(residual, zero);
    }
    #[cfg(target_feature = "avx2")]
    {
        let zero = f32x8::ZERO;
        let distance = position.abs();
        let inside = event & distance.cmp_lt(f32x8::splat(2.0));
        let inner_lanes = inside & distance.cmp_lt(f32x8::ONE);
        let outer_lanes = inside & !inner_lanes;
        let mut residual = zero;
        if inner_lanes.any() {
            let inner = f32x8::splat(0.023_312_11)
                .mul_add(distance, f32x8::splat(-0.079_173_68))
                .mul_add(distance, f32x8::splat(0.008_028_2))
                .mul_add(distance, f32x8::splat(0.311_749_82))
                .mul_add(distance, f32x8::splat(-0.5))
                .mul_add(distance, f32x8::splat(0.247_975_87));
            residual = inner_lanes.blend(inner, residual);
        }
        if outer_lanes.any() {
            let tail = f32x8::splat(2.0) - distance;
            let outer = f32x8::splat(0.007_742_371)
                .mul_add(tail, f32x8::splat(0.001_543_307_7))
                .mul_add(tail, f32x8::splat(0.002_451_625_8))
                .mul_add(tail, f32x8::splat(0.000_154_997_42))
                * tail
                * tail;
            residual = outer_lanes.blend(outer, residual);
        }
        residual
    }
}

pub(super) fn wrap01(value: f64) -> f64 {
    value - value.floor()
}
