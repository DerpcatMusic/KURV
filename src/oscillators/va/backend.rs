use truce_simd::simd::f32x8;
use wide::{CmpGt, CmpLt};

use super::{
    Antialiasing, PhaseWarpMode, VaOscillator, bandlimited_saw8,
    render::{prepare_shape8, sample_shape8_warped_at_auto_edge_prepared},
    spline_blep8_precomputed_static_with_bounds,
};

#[allow(clippy::too_many_arguments)]
pub(super) fn accumulate_shape8_block_steps_avx2<const SAMPLES: usize>(
    oscillators: &mut [VaOscillator],
    phase_steps: &[f32x8; SAMPLES],
    left_gain: f32x8,
    right_gain: f32x8,
    left: &mut [f32x8; SAMPLES],
    right: &mut [f32x8; SAMPLES],
    shape: f32,
    pulse_width: f32,
    antialiasing: Antialiasing,
    warp_mode: PhaseWarpMode,
    warp_amount: f32,
) -> bool {
    #[cfg(target_arch = "x86_64")]
    if warp_mode != PhaseWarpMode::None
        && warp_amount > f32::EPSILON
        && crate::performance::spline_backend() == crate::performance::SplineBackend::Avx2Fma
    {
        // SAFETY: the selected backend is only published after AVX2 and FMA
        // have both been detected on this machine.
        unsafe {
            accumulate_shape8_block_steps_warp_avx2(
                oscillators,
                phase_steps,
                left_gain,
                right_gain,
                left,
                right,
                shape,
                pulse_width,
                antialiasing,
                warp_mode,
                warp_amount,
            );
        }
        return true;
    }
    false
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
#[allow(
    unsafe_op_in_unsafe_fn,
    clippy::too_many_arguments,
    clippy::wildcard_imports,
    reason = "the runtime-guarded kernel uses the x86 intrinsic family as one implementation unit"
)]
unsafe fn accumulate_shape8_block_steps_warp_avx2<const SAMPLES: usize>(
    oscillators: &mut [VaOscillator],
    phase_steps: &[f32x8; SAMPLES],
    left_gain: f32x8,
    right_gain: f32x8,
    left: &mut [f32x8; SAMPLES],
    right: &mut [f32x8; SAMPLES],
    shape: f32,
    pulse_width: f32,
    antialiasing: Antialiasing,
    warp_mode: PhaseWarpMode,
    warp_amount: f32,
) {
    use core::arch::x86_64::*;

    debug_assert!(oscillators.len() >= 8);
    let phase_values: [f32; 8] = std::array::from_fn(|index| oscillators[index].phase);
    let left_gain_values: [f32; 8] = left_gain.into();
    let right_gain_values: [f32; 8] = right_gain.into();
    let mut phase = _mm256_loadu_ps(phase_values.as_ptr());
    let left_gain = _mm256_loadu_ps(left_gain_values.as_ptr());
    let right_gain = _mm256_loadu_ps(right_gain_values.as_ptr());
    let one = _mm256_set1_ps(1.0);
    let prepared_shape = prepare_shape8(shape);
    for frame in 0..SAMPLES {
        let step_values: [f32; 8] = phase_steps[frame].into();
        let step = _mm256_loadu_ps(step_values.as_ptr());
        let current = phase;
        let next = _mm256_add_ps(current, step);
        phase = _mm256_blendv_ps(
            _mm256_sub_ps(next, one),
            next,
            _mm256_cmp_ps(next, one, _CMP_LT_OQ),
        );
        let (warped_phase, warped_step) = warp_phase_avx2(current, step, warp_mode, warp_amount);
        let mut raw_phase_values = [0.0; 8];
        let mut warped_phase_values = [0.0; 8];
        let mut warped_step_values = [0.0; 8];
        _mm256_storeu_ps(raw_phase_values.as_mut_ptr(), current);
        _mm256_storeu_ps(warped_phase_values.as_mut_ptr(), warped_phase);
        _mm256_storeu_ps(warped_step_values.as_mut_ptr(), warped_step);
        let sample = sample_shape8_warped_at_auto_edge_prepared(
            f32x8::from(raw_phase_values),
            phase_steps[frame],
            f32x8::from(warped_phase_values),
            f32x8::from(warped_step_values),
            prepared_shape,
            pulse_width,
            antialiasing,
            warp_mode,
            warp_amount,
        );
        let sample_values: [f32; 8] = sample.into();
        let left_values: [f32; 8] = left[frame].into();
        let right_values: [f32; 8] = right[frame].into();
        let sample = _mm256_loadu_ps(sample_values.as_ptr());
        let mut output = [0.0; 8];
        _mm256_storeu_ps(
            output.as_mut_ptr(),
            _mm256_fmadd_ps(sample, left_gain, _mm256_loadu_ps(left_values.as_ptr())),
        );
        left[frame] = f32x8::from(output);
        _mm256_storeu_ps(
            output.as_mut_ptr(),
            _mm256_fmadd_ps(sample, right_gain, _mm256_loadu_ps(right_values.as_ptr())),
        );
        right[frame] = f32x8::from(output);
    }
    let mut phases = [0.0; 8];
    _mm256_storeu_ps(phases.as_mut_ptr(), phase);
    for (oscillator, phase) in oscillators.iter_mut().zip(phases) {
        oscillator.phase = phase;
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn warp_phase_avx2(
    phase: core::arch::x86_64::__m256,
    phase_step: core::arch::x86_64::__m256,
    mode: PhaseWarpMode,
    amount: f32,
) -> (core::arch::x86_64::__m256, core::arch::x86_64::__m256) {
    use core::arch::x86_64::*;

    let zero = _mm256_setzero_ps();
    let one = _mm256_set1_ps(1.0);
    let depth = _mm256_min_ps(
        _mm256_set1_ps(amount.clamp(0.0, 1.0) * 0.95),
        _mm256_max_ps(
            _mm256_sub_ps(
                _mm256_div_ps(
                    _mm256_set1_ps(0.45),
                    _mm256_max_ps(phase_step, _mm256_set1_ps(f32::EPSILON)),
                ),
                one,
            ),
            zero,
        ),
    );
    match mode {
        PhaseWarpMode::None => (phase, phase_step),
        PhaseWarpMode::Pwm => {
            let (sine, cosine) = sine_cosine_phase_avx2(phase);
            let second_sine =
                _mm256_add_ps(_mm256_mul_ps(sine, cosine), _mm256_mul_ps(sine, cosine));
            let second_cosine =
                _mm256_sub_ps(_mm256_mul_ps(cosine, cosine), _mm256_mul_ps(sine, sine));
            let normalization = _mm256_set1_ps(0.058_174_6);
            let displacement = _mm256_mul_ps(_mm256_sub_ps(cosine, second_cosine), normalization);
            let derivative = _mm256_mul_ps(
                _mm256_mul_ps(
                    _mm256_sub_ps(_mm256_add_ps(second_sine, second_sine), sine),
                    _mm256_set1_ps(std::f32::consts::TAU),
                ),
                normalization,
            );
            (
                _mm256_sub_ps(phase, _mm256_mul_ps(depth, displacement)),
                _mm256_mul_ps(
                    phase_step,
                    _mm256_sub_ps(one, _mm256_mul_ps(depth, derivative)),
                ),
            )
        }
        PhaseWarpMode::PhaseBend => {
            let doubled = _mm256_add_ps(phase, phase);
            let second_phase = _mm256_blendv_ps(
                _mm256_sub_ps(doubled, one),
                doubled,
                _mm256_cmp_ps(doubled, one, _CMP_LT_OQ),
            );
            let (sine, cosine) = sine_cosine_phase_avx2(second_phase);
            (
                _mm256_sub_ps(
                    phase,
                    _mm256_mul_ps(
                        _mm256_mul_ps(depth, sine),
                        _mm256_set1_ps((2.0 * std::f32::consts::TAU).recip()),
                    ),
                ),
                _mm256_mul_ps(phase_step, _mm256_sub_ps(one, _mm256_mul_ps(depth, cosine))),
            )
        }
        PhaseWarpMode::Harmonic => {
            let (sine, cosine) = sine_cosine_phase_avx2(phase);
            (
                _mm256_sub_ps(
                    phase,
                    _mm256_mul_ps(
                        _mm256_mul_ps(depth, sine),
                        _mm256_set1_ps(std::f32::consts::TAU.recip()),
                    ),
                ),
                _mm256_mul_ps(phase_step, _mm256_sub_ps(one, _mm256_mul_ps(depth, cosine))),
            )
        }
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn sine_cosine_phase_avx2(
    phase: core::arch::x86_64::__m256,
) -> (core::arch::x86_64::__m256, core::arch::x86_64::__m256) {
    use core::arch::x86_64::*;

    let sign = _mm256_set1_ps(-0.0);
    let half = _mm256_set1_ps(0.5);
    let quarter = _mm256_set1_ps(0.25);
    let folded = _mm256_sub_ps(
        quarter,
        _mm256_andnot_ps(
            sign,
            _mm256_sub_ps(_mm256_andnot_ps(sign, _mm256_sub_ps(phase, half)), quarter),
        ),
    );
    let sine = sine_polynomial_avx2(folded);
    let cosine = sine_polynomial_avx2(_mm256_sub_ps(quarter, folded));
    (
        _mm256_blendv_ps(
            sine,
            _mm256_xor_ps(sine, sign),
            _mm256_cmp_ps(phase, half, _CMP_GT_OQ),
        ),
        _mm256_blendv_ps(
            cosine,
            _mm256_xor_ps(cosine, sign),
            _mm256_and_ps(
                _mm256_cmp_ps(phase, quarter, _CMP_GT_OQ),
                _mm256_cmp_ps(phase, _mm256_set1_ps(0.75), _CMP_LT_OQ),
            ),
        ),
    )
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn sine_polynomial_avx2(folded: core::arch::x86_64::__m256) -> core::arch::x86_64::__m256 {
    use core::arch::x86_64::*;

    let folded2 = _mm256_mul_ps(folded, folded);
    let folded4 = _mm256_mul_ps(folded2, folded2);
    let low = _mm256_fmadd_ps(
        _mm256_set1_ps(-41.341_743),
        folded2,
        _mm256_set1_ps(std::f32::consts::TAU),
    );
    let middle = _mm256_fmadd_ps(
        _mm256_set1_ps(-76.811_29),
        folded2,
        _mm256_set1_ps(81.608_414),
    );
    let high = _mm256_fmadd_ps(
        _mm256_set1_ps(-23.605_793),
        folded2,
        _mm256_set1_ps(43.638_1),
    );
    _mm256_mul_ps(
        folded,
        _mm256_fmadd_ps(_mm256_fmadd_ps(high, folded4, middle), folded4, low),
    )
}

pub fn accumulate_saw8_block_constant<const SAMPLES: usize>(
    oscillators: &mut [VaOscillator],
    phase_step: f32x8,
    left_gain: f32x8,
    right_gain: f32x8,
    left: &mut [f32x8; SAMPLES],
    right: &mut [f32x8; SAMPLES],
    antialiasing: Antialiasing,
) {
    #[cfg(target_arch = "x86_64")]
    if crate::performance::spline_backend() == crate::performance::SplineBackend::Avx2Fma {
        // SAFETY: the selected backend is only published after AVX2 and FMA
        // have both been detected on this machine.
        unsafe {
            accumulate_saw8_block_constant_avx2(
                oscillators,
                phase_step,
                left_gain,
                right_gain,
                left,
                right,
                antialiasing,
            );
        }
        return;
    }
    accumulate_saw8_block_constant_impl(
        oscillators,
        phase_step,
        left_gain,
        right_gain,
        left,
        right,
        antialiasing,
    );
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
#[allow(
    unsafe_op_in_unsafe_fn,
    clippy::wildcard_imports,
    reason = "the runtime-guarded kernel uses the x86 intrinsic family as one implementation unit"
)]
unsafe fn accumulate_saw8_block_constant_avx2<const SAMPLES: usize>(
    oscillators: &mut [VaOscillator],
    phase_step: f32x8,
    left_gain: f32x8,
    right_gain: f32x8,
    left: &mut [f32x8; SAMPLES],
    right: &mut [f32x8; SAMPLES],
    antialiasing: Antialiasing,
) {
    use core::arch::x86_64::*;

    debug_assert!(oscillators.len() >= 8);
    let phase_values: [f32; 8] = std::array::from_fn(|index| oscillators[index].phase);
    let step_values: [f32; 8] = phase_step.into();
    let left_gain_values: [f32; 8] = left_gain.into();
    let right_gain_values: [f32; 8] = right_gain.into();
    let mut phase = _mm256_loadu_ps(phase_values.as_ptr());
    let step = _mm256_loadu_ps(step_values.as_ptr());
    let left_gain = _mm256_loadu_ps(left_gain_values.as_ptr());
    let right_gain = _mm256_loadu_ps(right_gain_values.as_ptr());
    let zero = _mm256_setzero_ps();
    let one = _mm256_set1_ps(1.0);
    let two = _mm256_set1_ps(2.0);
    let half = _mm256_set1_ps(0.5);
    let support = _mm256_add_ps(step, step);
    let active = _mm256_cmp_ps(step, _mm256_set1_ps(f32::EPSILON), _CMP_GT_OQ);
    let inverse_step = _mm256_div_ps(one, _mm256_blendv_ps(one, step, active));
    let optimized = antialiasing == Antialiasing::SplineOptimized;
    let narrow = _mm256_movemask_ps(_mm256_cmp_ps(support, half, _CMP_LT_OQ)) == 0xff;
    for frame in 0..SAMPLES {
        let current = phase;
        let next = _mm256_add_ps(phase, step);
        phase = _mm256_blendv_ps(
            _mm256_sub_ps(next, one),
            next,
            _mm256_cmp_ps(next, one, _CMP_LT_OQ),
        );
        let before_wrap = _mm256_cmp_ps(current, support, _CMP_LT_OQ);
        let event = _mm256_and_ps(
            active,
            _mm256_or_ps(
                before_wrap,
                _mm256_cmp_ps(current, _mm256_sub_ps(one, support), _CMP_GT_OQ),
            ),
        );
        let correction = if _mm256_movemask_ps(event) == 0 {
            zero
        } else if narrow {
            let nearest = _mm256_blendv_ps(_mm256_sub_ps(current, one), current, before_wrap);
            let position = _mm256_mul_ps(nearest, inverse_step);
            let residual = spline_blep_residual_narrow_avx2(position, event, optimized);
            _mm256_add_ps(residual, residual)
        } else {
            let start =
                spline_blep_residual_avx2(_mm256_mul_ps(current, inverse_step), event, optimized);
            let end = spline_blep_residual_avx2(
                _mm256_mul_ps(_mm256_sub_ps(current, one), inverse_step),
                event,
                optimized,
            );
            _mm256_mul_ps(_mm256_add_ps(start, end), two)
        };
        let sample = _mm256_sub_ps(_mm256_fmsub_ps(current, two, one), correction);
        let left_values: [f32; 8] = left[frame].into();
        let right_values: [f32; 8] = right[frame].into();
        let left_sample = _mm256_fmadd_ps(sample, left_gain, _mm256_loadu_ps(left_values.as_ptr()));
        let right_sample =
            _mm256_fmadd_ps(sample, right_gain, _mm256_loadu_ps(right_values.as_ptr()));
        let mut output = [0.0; 8];
        _mm256_storeu_ps(output.as_mut_ptr(), left_sample);
        left[frame] = f32x8::from(output);
        _mm256_storeu_ps(output.as_mut_ptr(), right_sample);
        right[frame] = f32x8::from(output);
    }
    let mut phases = [0.0; 8];
    _mm256_storeu_ps(phases.as_mut_ptr(), phase);
    for (oscillator, phase) in oscillators.iter_mut().zip(phases) {
        oscillator.phase = phase;
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
#[allow(
    unsafe_op_in_unsafe_fn,
    clippy::wildcard_imports,
    reason = "called only from the runtime-guarded x86 intrinsic kernel"
)]
unsafe fn spline_blep_residual_avx2(
    position: core::arch::x86_64::__m256,
    event: core::arch::x86_64::__m256,
    optimized: bool,
) -> core::arch::x86_64::__m256 {
    use core::arch::x86_64::*;

    let zero = _mm256_setzero_ps();
    let sign = _mm256_set1_ps(-0.0);
    let distance = _mm256_andnot_ps(sign, position);
    let inside = _mm256_and_ps(
        event,
        _mm256_cmp_ps(distance, _mm256_set1_ps(2.0), _CMP_LT_OQ),
    );
    let inner_lanes = _mm256_and_ps(
        inside,
        _mm256_cmp_ps(distance, _mm256_set1_ps(1.0), _CMP_LT_OQ),
    );
    let (inner, outer) = if optimized {
        let inner = _mm256_fmadd_ps(
            _mm256_fmadd_ps(
                _mm256_fmadd_ps(
                    _mm256_fmadd_ps(
                        _mm256_set1_ps(0.094_483_666),
                        distance,
                        _mm256_set1_ps(-0.273_396_5),
                    ),
                    distance,
                    _mm256_set1_ps(-0.000_368_514_85),
                ),
                distance,
                _mm256_set1_ps(0.626_745_1),
            ),
            distance,
            _mm256_set1_ps(-0.5),
        );
        let tail = _mm256_sub_ps(_mm256_set1_ps(2.0), distance);
        let outer = _mm256_mul_ps(
            _mm256_fmadd_ps(
                _mm256_fmadd_ps(
                    _mm256_fmadd_ps(
                        _mm256_set1_ps(-0.029_106_615),
                        tail,
                        _mm256_set1_ps(-0.026_743_31),
                    ),
                    tail,
                    _mm256_set1_ps(0.005_957_221_6),
                ),
                tail,
                _mm256_set1_ps(-0.002_643_542_6),
            ),
            tail,
        );
        (inner, outer)
    } else {
        let inner = _mm256_mul_ps(
            _mm256_fmadd_ps(
                _mm256_mul_ps(
                    _mm256_fmadd_ps(distance, _mm256_set1_ps(0.125), _mm256_set1_ps(-1.0 / 3.0)),
                    distance,
                ),
                distance,
                _mm256_set1_ps(2.0 / 3.0),
            ),
            distance,
        );
        let inner = _mm256_sub_ps(inner, _mm256_set1_ps(0.5));
        let tail = _mm256_sub_ps(_mm256_set1_ps(2.0), distance);
        let tail_squared = _mm256_mul_ps(tail, tail);
        let outer = _mm256_mul_ps(
            _mm256_sub_ps(zero, _mm256_mul_ps(tail_squared, tail_squared)),
            _mm256_set1_ps(1.0 / 24.0),
        );
        (inner, outer)
    };
    let residual = _mm256_and_ps(inside, _mm256_blendv_ps(outer, inner, inner_lanes));
    _mm256_blendv_ps(
        residual,
        _mm256_sub_ps(zero, residual),
        _mm256_cmp_ps(position, zero, _CMP_LT_OQ),
    )
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
#[allow(
    unsafe_op_in_unsafe_fn,
    clippy::wildcard_imports,
    reason = "called only from the runtime-guarded x86 intrinsic kernel"
)]
unsafe fn spline_blep_residual_narrow_avx2(
    position: core::arch::x86_64::__m256,
    event: core::arch::x86_64::__m256,
    optimized: bool,
) -> core::arch::x86_64::__m256 {
    use core::arch::x86_64::*;

    let zero = _mm256_setzero_ps();
    let sign = _mm256_set1_ps(-0.0);
    let distance = _mm256_andnot_ps(sign, position);
    let inner_lanes = _mm256_and_ps(
        event,
        _mm256_cmp_ps(distance, _mm256_set1_ps(1.0), _CMP_LT_OQ),
    );
    let (inner, outer) = if optimized {
        let inner = _mm256_fmadd_ps(
            _mm256_fmadd_ps(
                _mm256_fmadd_ps(
                    _mm256_fmadd_ps(
                        _mm256_set1_ps(0.094_483_666),
                        distance,
                        _mm256_set1_ps(-0.273_396_5),
                    ),
                    distance,
                    _mm256_set1_ps(-0.000_368_514_85),
                ),
                distance,
                _mm256_set1_ps(0.626_745_1),
            ),
            distance,
            _mm256_set1_ps(-0.5),
        );
        let tail = _mm256_sub_ps(_mm256_set1_ps(2.0), distance);
        let outer = _mm256_mul_ps(
            _mm256_fmadd_ps(
                _mm256_fmadd_ps(
                    _mm256_fmadd_ps(
                        _mm256_set1_ps(-0.029_106_615),
                        tail,
                        _mm256_set1_ps(-0.026_743_31),
                    ),
                    tail,
                    _mm256_set1_ps(0.005_957_221_6),
                ),
                tail,
                _mm256_set1_ps(-0.002_643_542_6),
            ),
            tail,
        );
        (inner, outer)
    } else {
        let inner = _mm256_mul_ps(
            _mm256_fmadd_ps(
                _mm256_mul_ps(
                    _mm256_fmadd_ps(distance, _mm256_set1_ps(0.125), _mm256_set1_ps(-1.0 / 3.0)),
                    distance,
                ),
                distance,
                _mm256_set1_ps(2.0 / 3.0),
            ),
            distance,
        );
        let inner = _mm256_sub_ps(inner, _mm256_set1_ps(0.5));
        let tail = _mm256_sub_ps(_mm256_set1_ps(2.0), distance);
        let tail_squared = _mm256_mul_ps(tail, tail);
        let outer = _mm256_mul_ps(
            _mm256_sub_ps(zero, _mm256_mul_ps(tail_squared, tail_squared)),
            _mm256_set1_ps(1.0 / 24.0),
        );
        (inner, outer)
    };
    let residual = _mm256_and_ps(event, _mm256_blendv_ps(outer, inner, inner_lanes));
    _mm256_blendv_ps(
        residual,
        _mm256_sub_ps(zero, residual),
        _mm256_cmp_ps(position, zero, _CMP_LT_OQ),
    )
}

#[inline(always)]
fn accumulate_saw8_block_constant_impl<const SAMPLES: usize>(
    oscillators: &mut [VaOscillator],
    phase_step: f32x8,
    left_gain: f32x8,
    right_gain: f32x8,
    left: &mut [f32x8; SAMPLES],
    right: &mut [f32x8; SAMPLES],
    antialiasing: Antialiasing,
) {
    debug_assert!(oscillators.len() >= 8);
    if antialiasing == Antialiasing::SplineOptimized {
        accumulate_saw8_block_constant_spline_impl::<SAMPLES, true>(
            oscillators,
            phase_step,
            left_gain,
            right_gain,
            left,
            right,
        );
        return;
    }
    if antialiasing == Antialiasing::Spline {
        accumulate_saw8_block_constant_spline_impl::<SAMPLES, false>(
            oscillators,
            phase_step,
            left_gain,
            right_gain,
            left,
            right,
        );
        return;
    }
    let mut phase = f32x8::from(std::array::from_fn(|index| oscillators[index].phase));
    for frame in 0..SAMPLES {
        let current = phase;
        let next = phase + phase_step;
        phase = next.cmp_lt(f32x8::ONE).blend(next, next - f32x8::ONE);
        let sample = bandlimited_saw8(current, phase_step, antialiasing);
        left[frame] = sample.mul_add(left_gain, left[frame]);
        right[frame] = sample.mul_add(right_gain, right[frame]);
    }
    let wrapped: [f32; 8] = phase.into();
    for (oscillator, phase) in oscillators.iter_mut().zip(wrapped) {
        oscillator.phase = phase;
    }
}

#[inline(always)]
fn accumulate_saw8_block_constant_spline_impl<const SAMPLES: usize, const OPTIMIZED: bool>(
    oscillators: &mut [VaOscillator],
    phase_step: f32x8,
    left_gain: f32x8,
    right_gain: f32x8,
    left: &mut [f32x8; SAMPLES],
    right: &mut [f32x8; SAMPLES],
) {
    debug_assert!(oscillators.len() >= 8);
    let mut phase = f32x8::from(std::array::from_fn(|index| oscillators[index].phase));
    let one = f32x8::ONE;
    let active = phase_step.cmp_gt(f32x8::splat(f32::EPSILON));
    let support = phase_step * f32x8::splat(2.0);
    let one_minus_support = one - support;
    let narrow = support.cmp_lt(f32x8::splat(0.5)).all();
    let inverse_step = one / active.blend(phase_step, one);
    for frame in 0..SAMPLES {
        let current = phase;
        let next = phase + phase_step;
        phase = next.cmp_lt(one).blend(next, next - one);
        let sample = current * f32x8::splat(2.0)
            - one
            - spline_blep8_precomputed_static_with_bounds::<OPTIMIZED>(
                current,
                active,
                support,
                one_minus_support,
                inverse_step,
                narrow,
            );
        left[frame] = sample.mul_add(left_gain, left[frame]);
        right[frame] = sample.mul_add(right_gain, right[frame]);
    }
    let wrapped: [f32; 8] = phase.into();
    for (oscillator, phase) in oscillators.iter_mut().zip(wrapped) {
        oscillator.phase = phase;
    }
}

pub(crate) fn calibrate_spline_backends()
-> Result<(u64, Option<u64>, crate::performance::SplineBackend), ()> {
    use crate::performance::SplineBackend;

    let baseline_output = calibration_output(SplineBackend::Baseline);
    let baseline_ns = measure_calibration_backend(SplineBackend::Baseline);
    if !crate::performance::backend_supported(SplineBackend::Avx2Fma) {
        return Ok((baseline_ns, None, SplineBackend::Baseline));
    }
    let avx2_output = calibration_output(SplineBackend::Avx2Fma);
    if !baseline_output.matches(&avx2_output) {
        return Err(());
    }
    let avx2_ns = measure_calibration_backend(SplineBackend::Avx2Fma);
    let selected = if avx2_ns.saturating_mul(100) < baseline_ns.saturating_mul(98) {
        SplineBackend::Avx2Fma
    } else {
        SplineBackend::Baseline
    };
    Ok((baseline_ns, Some(avx2_ns), selected))
}

const CALIBRATION_SAMPLES: usize = 32;
const CALIBRATION_PHASES: [f32; 8] = [
    0.0, 0.103_125, 0.206_25, 0.309_375, 0.412_5, 0.515_625, 0.618_75, 0.721_875,
];
const CALIBRATION_STEPS: [f32; 8] = [
    0.002_5, 0.002_81, 0.003_12, 0.003_43, 0.003_74, 0.004_05, 0.004_36, 0.004_67,
];

struct CalibrationOutput {
    left: [[f32; 8]; CALIBRATION_SAMPLES],
    right: [[f32; 8]; CALIBRATION_SAMPLES],
    phases: [f32; 8],
}

impl CalibrationOutput {
    fn matches(&self, other: &Self) -> bool {
        self.left
            .iter()
            .flatten()
            .chain(self.right.iter().flatten())
            .chain(self.phases.iter())
            .zip(
                other
                    .left
                    .iter()
                    .flatten()
                    .chain(other.right.iter().flatten())
                    .chain(other.phases.iter()),
            )
            .all(|(baseline, candidate)| (*baseline - *candidate).abs() <= 1.0e-6)
    }
}

fn calibration_output(backend: crate::performance::SplineBackend) -> CalibrationOutput {
    let mut oscillators = CALIBRATION_PHASES.map(|phase| VaOscillator {
        phase,
        resynth_zone: 0,
        resynth_zone_from: 0,
        resynth_zone_fade_remaining: 0,
        ..VaOscillator::default()
    });
    let phase_step = f32x8::from(CALIBRATION_STEPS);
    let gain = f32x8::splat(0.125);
    let mut left = [f32x8::ZERO; CALIBRATION_SAMPLES];
    let mut right = [f32x8::ZERO; CALIBRATION_SAMPLES];
    run_calibration_kernel(
        backend,
        &mut oscillators,
        phase_step,
        gain,
        &mut left,
        &mut right,
    );
    CalibrationOutput {
        left: left.map(Into::into),
        right: right.map(Into::into),
        phases: std::array::from_fn(|index| oscillators[index].phase),
    }
}

fn measure_calibration_backend(backend: crate::performance::SplineBackend) -> u64 {
    const ITERATIONS: usize = 4_096;
    const ITERATIONS_U128: u128 = 4_096;
    const REPEATS: usize = 5;
    let mut measurements = [0_u64; REPEATS];
    for measurement in &mut measurements {
        let mut oscillators = CALIBRATION_PHASES.map(|phase| VaOscillator {
            phase,
            resynth_zone: 0,
            resynth_zone_from: 0,
            resynth_zone_fade_remaining: 0,
            ..VaOscillator::default()
        });
        let phase_step = f32x8::from(CALIBRATION_STEPS);
        let gain = f32x8::splat(0.125);
        let mut left = [f32x8::ZERO; CALIBRATION_SAMPLES];
        let mut right = [f32x8::ZERO; CALIBRATION_SAMPLES];
        let start = std::time::Instant::now();
        for _ in 0..ITERATIONS {
            run_calibration_kernel(
                backend,
                &mut oscillators,
                phase_step,
                gain,
                &mut left,
                &mut right,
            );
            std::hint::black_box(left[CALIBRATION_SAMPLES - 1]);
        }
        *measurement =
            u64::try_from(start.elapsed().as_nanos() / ITERATIONS_U128).unwrap_or(u64::MAX);
    }
    measurements.sort_unstable();
    measurements[REPEATS / 2]
}

fn run_calibration_kernel<const SAMPLES: usize>(
    backend: crate::performance::SplineBackend,
    oscillators: &mut [VaOscillator; 8],
    phase_step: f32x8,
    gain: f32x8,
    left: &mut [f32x8; SAMPLES],
    right: &mut [f32x8; SAMPLES],
) {
    #[cfg(target_arch = "x86_64")]
    if backend == crate::performance::SplineBackend::Avx2Fma {
        // SAFETY: callers only request this backend after the runtime feature check.
        unsafe {
            accumulate_saw8_block_constant_avx2(
                oscillators,
                phase_step,
                gain,
                gain,
                left,
                right,
                Antialiasing::SplineOptimized,
            );
        }
        return;
    }
    let _ = backend;
    accumulate_saw8_block_constant_impl(
        oscillators,
        phase_step,
        gain,
        gain,
        left,
        right,
        Antialiasing::SplineOptimized,
    );
}
