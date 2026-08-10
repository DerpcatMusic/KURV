//! Virtual-analog rendering, sampling, and block generation.

use truce_simd::simd::{f32x4, f32x8};
use wide::{CmpGt, CmpLt};

use crate::wave_curve::WaveCurveRt;

use super::antialias::{
    aligned_sine_phase, aligned_sine_phase4, aligned_sine_phase8, bandlimited_pulse,
    bandlimited_pulse4, bandlimited_pulse8, bandlimited_saw, bandlimited_saw_pulse_morph4,
    bandlimited_saw_pulse_morph8, bandlimited_saw4, bandlimited_saw8, bandlimited_triangle,
    bandlimited_triangle4, bandlimited_triangle8, edge_blep, edge_blep4, edge_blep8,
    spline_blep4_precomputed, spline_blep8_precomputed, spline_saw8_narrow,
    spline_triangle4_precomputed, spline_triangle8_precomputed, wrap_phase4, wrap_phase8, wrap01,
};
use super::warp::{
    PreparedWarp4, PreparedWarp8, prepare_fixed_warp4, prepare_fixed_warp8,
    warp_phase_position_scalar, warp_phase_position4, warp_phase_position8, warp_phase_scalar,
    warp_phase4, warp_phase8, warped_pulse_edge_scalar, warped_pulse_edge4, warped_pulse_edge8,
};
use super::{
    Antialiasing, MAX_PRECOMPUTED_STEP_DRIFT, MAX_UNREFINED_STEP_DRIFT, PhaseWarpMode,
    VaOscillator, Waveform,
};

macro_rules! with_fixed_warp {
    ($prepared:expr, $kind:ident, |$warp:ident| $body:block) => {
        match $prepared {
            $kind::None($warp) => $body,
            $kind::Pwm($warp) => $body,
            $kind::PhaseBend($warp) => $body,
            $kind::Harmonic($warp) => $body,
        }
    };
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "the plugin output sample type is f32"
)]
pub fn generate_sine4(oscillators: &mut [VaOscillator], phase_steps: [f32; 4]) -> f32x4 {
    aligned_sine_phase4(advance4(oscillators, phase_steps))
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "f64 phase is retained; oscillator output is intentionally f32"
)]
pub fn generate_sine8(oscillators: &mut [VaOscillator], phase_steps: [f32; 8]) -> f32x8 {
    aligned_sine_phase8(advance8(oscillators, phase_steps))
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "f64 phase is retained; oscillator output is intentionally f32"
)]
pub fn generate_shape8(
    oscillators: &mut [VaOscillator],
    shape: f32,
    phase_steps: [f32; 8],
    pulse_width: f32,
    antialiasing: Antialiasing,
) -> f32x8 {
    let phases = advance8(oscillators, phase_steps);
    sample_shape8_at(
        phases,
        f32x8::from(phase_steps),
        shape,
        pulse_width,
        antialiasing,
    )
}

pub fn generate_shape8_warped(
    oscillators: &mut [VaOscillator],
    shape: f32,
    phase_steps: [f32; 8],
    pulse_width: f32,
    antialiasing: Antialiasing,
    warp_mode: PhaseWarpMode,
    warp_amount: f32,
) -> f32x8 {
    let raw_phases = advance8(oscillators, phase_steps);
    let raw_steps = f32x8::from(phase_steps);
    let (phases, warped_steps) = warp_phase8(raw_phases, raw_steps, warp_mode, warp_amount);
    sample_shape8_warped_at_auto_edge(
        raw_phases,
        raw_steps,
        phases,
        warped_steps,
        shape,
        pulse_width,
        antialiasing,
        warp_mode,
        warp_amount,
    )
}

pub fn generate_custom8(
    oscillators: &mut [VaOscillator],
    shape: f32,
    phase_steps: [f32; 8],
    pulse_width: f32,
    antialiasing: Antialiasing,
    warp_mode: PhaseWarpMode,
    warp_amount: f32,
    curve: WaveCurveRt,
    mix: f32,
) -> f32x8 {
    let raw_phases = advance8(oscillators, phase_steps);
    let raw_steps = f32x8::from(phase_steps);
    if mix >= 1.0 {
        curve.eval8(warp_phase_position8(
            raw_phases,
            raw_steps,
            warp_mode,
            warp_amount,
        ))
    } else {
        let (phases, steps) = warp_phase8(raw_phases, raw_steps, warp_mode, warp_amount);
        let custom = curve.eval8(phases);
        let canonical = sample_shape8_warped_at_auto_edge(
            raw_phases,
            raw_steps,
            phases,
            steps,
            shape,
            pulse_width,
            antialiasing,
            warp_mode,
            warp_amount,
        );
        (custom - canonical).mul_add(f32x8::splat(mix.clamp(0.0, 1.0)), canonical)
    }
}

fn sample_shape8_at(
    phases: f32x8,
    phase_steps: f32x8,
    shape: f32,
    pulse_width: f32,
    antialiasing: Antialiasing,
) -> f32x8 {
    let shape = shape.clamp(0.0, 3.0);
    let (first, blend) = shape_segment(shape);
    if blend > f32::EPSILON && first == Waveform::Saw {
        return bandlimited_saw_pulse_morph8(phases, phase_steps, pulse_width, blend, antialiasing);
    }
    let a = sample_waveform8(first, phases, phase_steps, pulse_width, antialiasing);
    if blend <= f32::EPSILON {
        a
    } else {
        let b = sample_waveform8(
            next_waveform(first),
            phases,
            phase_steps,
            pulse_width,
            antialiasing,
        );
        (b - a).mul_add(f32x8::splat(blend), a) * f32x8::splat(morph_gain(first, blend))
    }
}

#[inline]
fn sample_shape8_warped_at_auto_edge(
    raw_phase: f32x8,
    raw_step: f32x8,
    phase: f32x8,
    phase_step: f32x8,
    shape: f32,
    pulse_width: f32,
    antialiasing: Antialiasing,
    warp_mode: PhaseWarpMode,
    warp_amount: f32,
) -> f32x8 {
    let (pulse_edge, width) = if shape > 2.0 {
        let one = f32x8::ONE;
        (
            warped_pulse_edge8(raw_step, pulse_width, warp_mode, warp_amount),
            raw_step
                .fast_max(f32x8::splat(pulse_width.clamp(0.03, 0.97)))
                .fast_min(one - raw_step),
        )
    } else {
        (None, f32x8::ZERO)
    };
    sample_shape8_warped_at_impl(
        raw_phase,
        raw_step,
        phase,
        phase_step,
        shape,
        pulse_width,
        antialiasing,
        width,
        pulse_edge,
    )
}

#[inline]
fn sample_shape8_warped_at_impl(
    raw_phase: f32x8,
    raw_step: f32x8,
    phase: f32x8,
    phase_step: f32x8,
    shape: f32,
    pulse_width: f32,
    antialiasing: Antialiasing,
    width: f32x8,
    pulse_edge: Option<f32x8>,
) -> f32x8 {
    // The warp changes sample position, not the time of the cycle reset. Keep the
    // BLEP centered on raw phase so its fractional discontinuity time stays exact.
    let shape = shape.clamp(0.0, 3.0);
    let (first, blend) = shape_segment(shape);
    if first == Waveform::Sine || first == Waveform::Triangle && blend <= f32::EPSILON {
        return sample_shape8_at(phase, phase_step, shape, pulse_width, antialiasing);
    }
    let sample = |waveform| match waveform {
        Waveform::Saw => {
            phase * f32x8::splat(2.0) - f32x8::ONE - edge_blep8(raw_phase, raw_step, antialiasing)
        }
        Waveform::Pulse => {
            let one = f32x8::ONE;
            let (shifted, edge_step) = pulse_edge.map_or_else(
                || (wrap_phase8(phase + one - width), phase_step),
                |edge| (wrap_phase8(raw_phase + one - edge), raw_step),
            );
            phase.cmp_lt(width).blend(one, -one) + edge_blep8(raw_phase, raw_step, antialiasing)
                - edge_blep8(shifted, edge_step, antialiasing)
        }
        _ => sample_waveform8(waveform, phase, phase_step, pulse_width, antialiasing),
    };
    let a = sample(first);
    if blend <= f32::EPSILON {
        a
    } else {
        let b = sample(next_waveform(first));
        (b - a).mul_add(f32x8::splat(blend), a) * f32x8::splat(morph_gain(first, blend))
    }
}

pub fn generate_shape8_pair(
    oscillators: &mut [VaOscillator],
    shape: f32,
    phase_steps: [[f32; 8]; 2],
    pulse_width: f32,
    antialiasing: Antialiasing,
) -> [f32x8; 2] {
    let [phases0, phases1] = advance8_pair(oscillators, phase_steps);
    [
        sample_shape8_at(
            phases0,
            f32x8::from(phase_steps[0]),
            shape,
            pulse_width,
            antialiasing,
        ),
        sample_shape8_at(
            phases1,
            f32x8::from(phase_steps[1]),
            shape,
            pulse_width,
            antialiasing,
        ),
    ]
}

pub fn generate_shape8_pair_warped(
    oscillators: &mut [VaOscillator],
    shape: f32,
    phase_steps: [[f32; 8]; 2],
    pulse_width: f32,
    antialiasing: Antialiasing,
    warp_mode: PhaseWarpMode,
    warp_amount: f32,
) -> [f32x8; 2] {
    let [raw_phases0, raw_phases1] = advance8_pair(oscillators, phase_steps);
    let raw_steps0 = f32x8::from(phase_steps[0]);
    let raw_steps1 = f32x8::from(phase_steps[1]);
    let (phases0, steps0) = warp_phase8(raw_phases0, raw_steps0, warp_mode, warp_amount);
    let (phases1, steps1) = warp_phase8(raw_phases1, raw_steps1, warp_mode, warp_amount);
    [
        sample_shape8_warped_at_auto_edge(
            raw_phases0,
            raw_steps0,
            phases0,
            steps0,
            shape,
            pulse_width,
            antialiasing,
            warp_mode,
            warp_amount,
        ),
        sample_shape8_warped_at_auto_edge(
            raw_phases1,
            raw_steps1,
            phases1,
            steps1,
            shape,
            pulse_width,
            antialiasing,
            warp_mode,
            warp_amount,
        ),
    ]
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "f64 phase is retained; oscillator output is intentionally f32"
)]
pub fn generate_triangle8(
    oscillators: &mut [VaOscillator],
    phase_steps: [f32; 8],
    antialiasing: Antialiasing,
) -> f32x8 {
    let phases = advance8(oscillators, phase_steps);
    bandlimited_triangle8(phases, f32x8::from(phase_steps), antialiasing)
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "f64 phase is retained; oscillator output is intentionally f32"
)]
pub fn generate_saw8(
    oscillators: &mut [VaOscillator],
    phase_steps: [f32; 8],
    antialiasing: Antialiasing,
) -> f32x8 {
    let phases = advance8(oscillators, phase_steps);
    bandlimited_saw8(phases, f32x8::from(phase_steps), antialiasing)
}

pub fn accumulate_saw8_block<const SAMPLES: usize>(
    oscillators: &mut [VaOscillator],
    phase_steps: [f32x8; SAMPLES],
    left_gain: f32x8,
    right_gain: f32x8,
    left: &mut [f32x8; SAMPLES],
    right: &mut [f32x8; SAMPLES],
    antialiasing: Antialiasing,
) {
    debug_assert!(oscillators.len() >= 8);
    let mut phase = f32x8::from(std::array::from_fn(|index| oscillators[index].phase));
    for frame in 0..SAMPLES {
        let current = phase;
        let next = phase + phase_steps[frame];
        phase = next.cmp_lt(f32x8::ONE).blend(next, next - f32x8::ONE);
        let sample = bandlimited_saw8(current, phase_steps[frame], antialiasing);
        left[frame] = sample.mul_add(left_gain, left[frame]);
        right[frame] = sample.mul_add(right_gain, right[frame]);
    }
    let wrapped: [f32; 8] = phase.into();
    for (oscillator, phase) in oscillators.iter_mut().zip(wrapped) {
        oscillator.phase = phase;
    }
}

pub fn accumulate_saw8_block_dynamic_gains<const SAMPLES: usize>(
    oscillators: &mut [VaOscillator],
    phase_steps: [f32x8; SAMPLES],
    left_gains: [f32x8; SAMPLES],
    right_gains: [f32x8; SAMPLES],
    left: &mut [f32x8; SAMPLES],
    right: &mut [f32x8; SAMPLES],
    antialiasing: Antialiasing,
) {
    debug_assert!(oscillators.len() >= 8);
    let mut phase = f32x8::from(std::array::from_fn(|index| oscillators[index].phase));
    for frame in 0..SAMPLES {
        let current = phase;
        let next = phase + phase_steps[frame];
        phase = next.cmp_lt(f32x8::ONE).blend(next, next - f32x8::ONE);
        let sample = bandlimited_saw8(current, phase_steps[frame], antialiasing);
        left[frame] = sample.mul_add(left_gains[frame], left[frame]);
        right[frame] = sample.mul_add(right_gains[frame], right[frame]);
    }
    let wrapped: [f32; 8] = phase.into();
    for (oscillator, phase) in oscillators.iter_mut().zip(wrapped) {
        oscillator.phase = phase;
    }
}

pub fn accumulate_saw8_block_static_gains<const SAMPLES: usize>(
    oscillators: &mut [VaOscillator],
    mut phase_step: f32x8,
    phase_step_delta: f32x8,
    left_gain: f32x8,
    right_gain: f32x8,
    left: &mut [f32x8; SAMPLES],
    right: &mut [f32x8; SAMPLES],
    antialiasing: Antialiasing,
) -> f32x8 {
    debug_assert!(oscillators.len() >= 8);
    let mut phase = f32x8::from(std::array::from_fn(|index| oscillators[index].phase));
    if matches!(
        antialiasing,
        Antialiasing::Spline | Antialiasing::SplineOptimized
    ) {
        let one = f32x8::ONE;
        let block_drift = phase_step_delta.abs() * f32x8::splat(SAMPLES as f32);
        let reference_step = phase_step + phase_step_delta * f32x8::splat(SAMPLES as f32 * 0.5);
        let relative_drift = block_drift / reference_step.fast_max(f32x8::splat(f32::EPSILON));
        if relative_drift
            .cmp_lt(f32x8::splat(MAX_PRECOMPUTED_STEP_DRIFT))
            .all()
        {
            let refine_step = !relative_drift
                .cmp_lt(f32x8::splat(MAX_UNREFINED_STEP_DRIFT))
                .all();
            let active = reference_step.cmp_gt(f32x8::splat(f32::EPSILON));
            let support = reference_step * f32x8::splat(2.0);
            let inverse_step = one / active.blend(reference_step, one);
            let inverse_step_squared = inverse_step * inverse_step;
            let optimized = antialiasing == Antialiasing::SplineOptimized;
            let frame_inverse_steps = if refine_step {
                std::array::from_fn(|frame| {
                    let frame_step =
                        phase_step + phase_step_delta * f32x8::splat((frame + 1) as f32);
                    (reference_step - frame_step).mul_add(inverse_step_squared, inverse_step)
                })
            } else {
                [inverse_step; SAMPLES]
            };
            for frame in 0..SAMPLES {
                phase_step += phase_step_delta;
                let current = phase;
                let next = phase + phase_step;
                phase = next.cmp_lt(one).blend(next, next - one);
                let sample = current * f32x8::splat(2.0)
                    - one
                    - spline_blep8_precomputed(
                        current,
                        active,
                        support,
                        frame_inverse_steps[frame],
                        optimized,
                    );
                #[cfg(debug_assertions)]
                {
                    let exact = bandlimited_saw8(current, phase_step, antialiasing);
                    let sample: [f32; 8] = sample.into();
                    let exact: [f32; 8] = exact.into();
                    debug_assert!(
                        sample
                            .iter()
                            .zip(exact)
                            .all(|(sample, exact)| (*sample - exact).abs() < 1.0e-5),
                        "adaptive spline step exceeded its sample-error bound"
                    );
                }
                left[frame] = sample.mul_add(left_gain, left[frame]);
                right[frame] = sample.mul_add(right_gain, right[frame]);
            }
            let wrapped: [f32; 8] = phase.into();
            for (oscillator, phase) in oscillators.iter_mut().zip(wrapped) {
                oscillator.phase = phase;
            }
            return phase_step;
        }
    }
    for frame in 0..SAMPLES {
        phase_step += phase_step_delta;
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
    phase_step
}

pub fn accumulate_saw8_block_static_gains_narrow_spline<const SAMPLES: usize>(
    oscillators: &mut [VaOscillator],
    mut phase_step: f32x8,
    phase_step_delta: f32x8,
    left_gain: f32x8,
    right_gain: f32x8,
    left: &mut [f32x8; SAMPLES],
    right: &mut [f32x8; SAMPLES],
    antialiasing: Antialiasing,
) -> f32x8 {
    debug_assert!(oscillators.len() >= 8);
    debug_assert!(matches!(
        antialiasing,
        Antialiasing::Spline | Antialiasing::SplineOptimized
    ));
    let mut phase = f32x8::from(std::array::from_fn(|index| oscillators[index].phase));
    let optimized = antialiasing == Antialiasing::SplineOptimized;
    for frame in 0..SAMPLES {
        phase_step += phase_step_delta;
        let current = phase;
        let next = phase + phase_step;
        phase = next.cmp_lt(f32x8::ONE).blend(next, next - f32x8::ONE);
        let sample = spline_saw8_narrow(current, phase_step, optimized);
        left[frame] = sample.mul_add(left_gain, left[frame]);
        right[frame] = sample.mul_add(right_gain, right[frame]);
    }
    let wrapped: [f32; 8] = phase.into();
    for (oscillator, phase) in oscillators.iter_mut().zip(wrapped) {
        oscillator.phase = phase;
    }
    phase_step
}

#[inline]
pub fn is_narrow_spline_ramp<const SAMPLES: usize>(
    phase_step: f32x8,
    phase_step_delta: f32x8,
    antialiasing: Antialiasing,
) -> bool {
    if !matches!(
        antialiasing,
        Antialiasing::Spline | Antialiasing::SplineOptimized
    ) {
        return false;
    }
    let frames = f32x8::splat(SAMPLES as f32);
    let final_step = phase_step + phase_step_delta * frames;
    let reference_step = phase_step + phase_step_delta * (frames * f32x8::splat(0.5));
    let relative_drift =
        phase_step_delta.abs() * frames / reference_step.fast_max(f32x8::splat(f32::EPSILON));
    phase_step
        .fast_max(final_step)
        .cmp_lt(f32x8::splat(0.25))
        .all()
        && !relative_drift
            .cmp_lt(f32x8::splat(MAX_PRECOMPUTED_STEP_DRIFT))
            .all()
}

pub fn accumulate_shape8_block_constant<const SAMPLES: usize>(
    oscillators: &mut [VaOscillator],
    phase_step: f32x8,
    left_gain: f32x8,
    right_gain: f32x8,
    left: &mut [f32x8; SAMPLES],
    right: &mut [f32x8; SAMPLES],
    shape: f32,
    pulse_width: f32,
    antialiasing: Antialiasing,
) {
    debug_assert!(oscillators.len() >= 8);
    let mut phase = f32x8::from(std::array::from_fn(|index| oscillators[index].phase));
    if shape == 0.0 {
        let one = f32x8::ONE;
        for frame in 0..SAMPLES {
            let current = phase;
            let next = phase + phase_step;
            phase = next.cmp_lt(one).blend(next, next - one);
            let sample = aligned_sine_phase8(current);
            left[frame] = sample.mul_add(left_gain, left[frame]);
            right[frame] = sample.mul_add(right_gain, right[frame]);
        }
        let wrapped: [f32; 8] = phase.into();
        for (oscillator, phase) in oscillators.iter_mut().zip(wrapped) {
            oscillator.phase = phase;
        }
        return;
    }
    if matches!(
        antialiasing,
        Antialiasing::Spline | Antialiasing::SplineOptimized
    ) {
        let one = f32x8::ONE;
        let active = phase_step.cmp_gt(f32x8::splat(f32::EPSILON));
        let support = phase_step * f32x8::splat(2.0);
        let inverse_step = one / active.blend(phase_step, one);
        let optimized = antialiasing == Antialiasing::SplineOptimized;
        let (first, blend_scalar) = shape_segment(shape.clamp(0.0, 3.0));
        let blend = f32x8::splat(blend_scalar);
        let inverse_blend = one - blend;
        let gain = f32x8::splat(morph_gain(first, blend_scalar));
        let width = phase_step
            .fast_max(f32x8::splat(pulse_width.clamp(0.03, 0.97)))
            .fast_min(one - phase_step);
        for frame in 0..SAMPLES {
            let current = phase;
            let next = phase + phase_step;
            phase = next.cmp_lt(one).blend(next, next - one);
            let sample = match first {
                Waveform::Sine => {
                    let sine = aligned_sine_phase8(current);
                    if blend_scalar <= f32::EPSILON {
                        sine
                    } else {
                        let triangle = spline_triangle8_precomputed(
                            current,
                            phase_step,
                            active,
                            support,
                            inverse_step,
                            optimized,
                        );
                        (triangle - sine).mul_add(blend, sine)
                    }
                }
                Waveform::Triangle => {
                    let triangle = spline_triangle8_precomputed(
                        current,
                        phase_step,
                        active,
                        support,
                        inverse_step,
                        optimized,
                    );
                    if blend_scalar == 0.0 {
                        triangle
                    } else {
                        let saw = current * f32x8::splat(2.0)
                            - one
                            - spline_blep8_precomputed(
                                current,
                                active,
                                support,
                                inverse_step,
                                optimized,
                            );
                        (saw - triangle).mul_add(blend, triangle) * gain
                    }
                }
                Waveform::Saw => {
                    let saw = current * f32x8::splat(2.0) - one;
                    let pulse = current.cmp_lt(width).blend(one, f32x8::splat(-1.0));
                    let shifted = wrap_phase8(current + one - width);
                    let wrap_correction =
                        spline_blep8_precomputed(current, active, support, inverse_step, optimized);
                    let width_correction =
                        spline_blep8_precomputed(shifted, active, support, inverse_step, optimized);
                    let raw = pulse.mul_add(blend, saw * inverse_blend);
                    let correction = (blend * f32x8::splat(2.0) - one)
                        .mul_add(wrap_correction, -(blend * width_correction));
                    raw + correction
                }
                Waveform::Pulse => {
                    let pulse = current.cmp_lt(width).blend(one, f32x8::splat(-1.0));
                    let shifted = wrap_phase8(current + one - width);
                    pulse
                        + spline_blep8_precomputed(
                            current,
                            active,
                            support,
                            inverse_step,
                            optimized,
                        )
                        - spline_blep8_precomputed(
                            shifted,
                            active,
                            support,
                            inverse_step,
                            optimized,
                        )
                }
            };
            left[frame] = sample.mul_add(left_gain, left[frame]);
            right[frame] = sample.mul_add(right_gain, right[frame]);
        }
        let wrapped: [f32; 8] = phase.into();
        for (oscillator, phase) in oscillators.iter_mut().zip(wrapped) {
            oscillator.phase = phase;
        }
        return;
    }
    for frame in 0..SAMPLES {
        let current = phase;
        let next = phase + phase_step;
        phase = next.cmp_lt(f32x8::ONE).blend(next, next - f32x8::ONE);
        let sample = sample_shape8_at(current, phase_step, shape, pulse_width, antialiasing);
        left[frame] = sample.mul_add(left_gain, left[frame]);
        right[frame] = sample.mul_add(right_gain, right[frame]);
    }
    let wrapped: [f32; 8] = phase.into();
    for (oscillator, phase) in oscillators.iter_mut().zip(wrapped) {
        oscillator.phase = phase;
    }
}

pub fn accumulate_shape8_block_constant_warped<const SAMPLES: usize>(
    oscillators: &mut [VaOscillator],
    phase_step: f32x8,
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
    debug_assert!(oscillators.len() >= 8);
    let mut raw_phase = f32x8::from(std::array::from_fn(|index| oscillators[index].phase));
    let (pulse_edge, width) = if shape > 2.0 {
        (
            warped_pulse_edge8(phase_step, pulse_width, warp_mode, warp_amount),
            phase_step
                .fast_max(f32x8::splat(pulse_width.clamp(0.03, 0.97)))
                .fast_min(f32x8::ONE - phase_step),
        )
    } else {
        (None, f32x8::ZERO)
    };
    with_fixed_warp!(
        prepare_fixed_warp8(phase_step, warp_mode, warp_amount),
        PreparedWarp8,
        |warp| {
            for frame in 0..SAMPLES {
                let current = raw_phase;
                let next = current + phase_step;
                raw_phase = next.cmp_lt(f32x8::ONE).blend(next, next - f32x8::ONE);
                let (phase, warped_step) = warp.warp_phase(current);
                let sample = sample_shape8_warped_at_impl(
                    current,
                    phase_step,
                    phase,
                    warped_step,
                    shape,
                    pulse_width,
                    antialiasing,
                    width,
                    pulse_edge,
                );
                left[frame] = sample.mul_add(left_gain, left[frame]);
                right[frame] = sample.mul_add(right_gain, right[frame]);
            }
        }
    );
    let wrapped: [f32; 8] = raw_phase.into();
    for (oscillator, phase) in oscillators.iter_mut().zip(wrapped) {
        oscillator.phase = phase;
    }
}

pub fn accumulate_custom8_block_constant<const SAMPLES: usize>(
    oscillators: &mut [VaOscillator],
    phase_step: f32x8,
    left_gain: f32x8,
    right_gain: f32x8,
    left: &mut [f32x8; SAMPLES],
    right: &mut [f32x8; SAMPLES],
    curve: WaveCurveRt,
    mix: f32,
    shape: f32,
    pulse_width: f32,
    antialiasing: Antialiasing,
    warp_mode: PhaseWarpMode,
    warp_amount: f32,
) {
    let mut phase = f32x8::from(std::array::from_fn(|index| oscillators[index].phase));
    let (pulse_edge, width) = if !(mix >= 1.0) && shape > 2.0 {
        (
            warped_pulse_edge8(phase_step, pulse_width, warp_mode, warp_amount),
            phase_step
                .fast_max(f32x8::splat(pulse_width.clamp(0.03, 0.97)))
                .fast_min(f32x8::ONE - phase_step),
        )
    } else {
        (None, f32x8::ZERO)
    };
    with_fixed_warp!(
        prepare_fixed_warp8(phase_step, warp_mode, warp_amount),
        PreparedWarp8,
        |warp| {
            for frame in 0..SAMPLES {
                let current = phase;
                let next = phase + phase_step;
                phase = next.cmp_lt(f32x8::ONE).blend(next, next - f32x8::ONE);
                let sample = if mix >= 1.0 {
                    curve.eval8(warp.warp_position(current))
                } else {
                    let (warped_phase, warped_step) = warp.warp_phase(current);
                    let canonical = sample_shape8_warped_at_impl(
                        current,
                        phase_step,
                        warped_phase,
                        warped_step,
                        shape,
                        pulse_width,
                        antialiasing,
                        width,
                        pulse_edge,
                    );
                    (curve.eval8(warped_phase) - canonical).mul_add(f32x8::splat(mix), canonical)
                };
                left[frame] = sample.mul_add(left_gain, left[frame]);
                right[frame] = sample.mul_add(right_gain, right[frame]);
            }
        }
    );
    let wrapped: [f32; 8] = phase.into();
    for (oscillator, phase) in oscillators.iter_mut().zip(wrapped) {
        oscillator.phase = phase;
    }
}

pub fn accumulate_custom8_block<const SAMPLES: usize>(
    oscillators: &mut [VaOscillator],
    phase_steps: [f32x8; SAMPLES],
    left_gain: f32x8,
    right_gain: f32x8,
    left: &mut [f32x8; SAMPLES],
    right: &mut [f32x8; SAMPLES],
    curve: WaveCurveRt,
    mix: f32,
    shape: f32,
    pulse_width: f32,
    antialiasing: Antialiasing,
    warp_mode: PhaseWarpMode,
    warp_amount: f32,
) {
    let mut phase = f32x8::from(std::array::from_fn(|index| oscillators[index].phase));
    for frame in 0..SAMPLES {
        let current = phase;
        let next = phase + phase_steps[frame];
        phase = next.cmp_lt(f32x8::ONE).blend(next, next - f32x8::ONE);
        let sample = if mix >= 1.0 {
            curve.eval8(warp_phase_position8(
                current,
                phase_steps[frame],
                warp_mode,
                warp_amount,
            ))
        } else {
            let (warped_phase, warped_step) =
                warp_phase8(current, phase_steps[frame], warp_mode, warp_amount);
            let canonical = sample_shape8_warped_at_auto_edge(
                current,
                phase_steps[frame],
                warped_phase,
                warped_step,
                shape,
                pulse_width,
                antialiasing,
                warp_mode,
                warp_amount,
            );
            (curve.eval8(warped_phase) - canonical).mul_add(f32x8::splat(mix), canonical)
        };
        left[frame] = sample.mul_add(left_gain, left[frame]);
        right[frame] = sample.mul_add(right_gain, right[frame]);
    }
    let wrapped: [f32; 8] = phase.into();
    for (oscillator, phase) in oscillators.iter_mut().zip(wrapped) {
        oscillator.phase = phase;
    }
}

#[allow(clippy::too_many_arguments)]
#[inline]
fn spline_shape8_precomputed(
    phase: f32x8,
    phase_step: f32x8,
    active: f32x8,
    support: f32x8,
    inverse_step: f32x8,
    shape: f32,
    morph_gain: f32,
    pulse_width: f32,
    optimized: bool,
) -> f32x8 {
    let (first, blend_scalar) = shape_segment(shape.clamp(0.0, 3.0));
    spline_shape8_segment_precomputed(
        phase,
        phase_step,
        active,
        support,
        inverse_step,
        first,
        blend_scalar,
        morph_gain,
        pulse_width,
        optimized,
    )
}

#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn spline_shape8_segment_precomputed(
    phase: f32x8,
    phase_step: f32x8,
    active: f32x8,
    support: f32x8,
    inverse_step: f32x8,
    first: Waveform,
    blend_scalar: f32,
    morph_gain: f32,
    pulse_width: f32,
    optimized: bool,
) -> f32x8 {
    let one = f32x8::ONE;
    let blend = f32x8::splat(blend_scalar);
    match first {
        Waveform::Sine => {
            let sine = aligned_sine_phase8(phase);
            if blend_scalar <= f32::EPSILON {
                sine
            } else {
                let triangle = spline_triangle8_precomputed(
                    phase,
                    phase_step,
                    active,
                    support,
                    inverse_step,
                    optimized,
                );
                (triangle - sine).mul_add(blend, sine)
            }
        }
        Waveform::Triangle => {
            let triangle = spline_triangle8_precomputed(
                phase,
                phase_step,
                active,
                support,
                inverse_step,
                optimized,
            );
            let saw = phase * f32x8::splat(2.0)
                - one
                - spline_blep8_precomputed(phase, active, support, inverse_step, optimized);
            (saw - triangle).mul_add(blend, triangle) * f32x8::splat(morph_gain)
        }
        Waveform::Saw | Waveform::Pulse => {
            let width = phase_step
                .fast_max(f32x8::splat(pulse_width.clamp(0.03, 0.97)))
                .fast_min(one - phase_step);
            let saw = phase * f32x8::splat(2.0) - one;
            let pulse = phase.cmp_lt(width).blend(one, f32x8::splat(-1.0));
            let shifted = wrap_phase8(phase + one - width);
            let wrap = spline_blep8_precomputed(phase, active, support, inverse_step, optimized);
            let edge = spline_blep8_precomputed(shifted, active, support, inverse_step, optimized);
            if first == Waveform::Pulse {
                pulse + wrap - edge
            } else {
                let raw = (pulse - saw).mul_add(blend, saw);
                raw + (blend * f32x8::splat(2.0) - one).mul_add(wrap, -(blend * edge))
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
#[inline]
fn spline_shape4_precomputed(
    phase: f32x4,
    phase_step: f32x4,
    active: f32x4,
    support: f32x4,
    inverse_step: f32x4,
    shape: f32,
    morph_gain: f32,
    pulse_width: f32,
    optimized: bool,
) -> f32x4 {
    let (first, blend_scalar) = shape_segment(shape.clamp(0.0, 3.0));
    spline_shape4_segment_precomputed(
        phase,
        phase_step,
        active,
        support,
        inverse_step,
        first,
        blend_scalar,
        morph_gain,
        pulse_width,
        optimized,
    )
}

#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn spline_shape4_segment_precomputed(
    phase: f32x4,
    phase_step: f32x4,
    active: f32x4,
    support: f32x4,
    inverse_step: f32x4,
    first: Waveform,
    blend_scalar: f32,
    morph_gain: f32,
    pulse_width: f32,
    optimized: bool,
) -> f32x4 {
    let width = phase_step
        .fast_max(f32x4::splat(pulse_width.clamp(0.03, 0.97)))
        .fast_min(f32x4::ONE - phase_step);
    spline_shape4_segment_precomputed_with_width(
        phase,
        phase_step,
        active,
        support,
        inverse_step,
        first,
        blend_scalar,
        morph_gain,
        width,
        optimized,
    )
}

#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn spline_shape4_segment_precomputed_with_width(
    phase: f32x4,
    phase_step: f32x4,
    active: f32x4,
    support: f32x4,
    inverse_step: f32x4,
    first: Waveform,
    blend_scalar: f32,
    morph_gain: f32,
    width: f32x4,
    optimized: bool,
) -> f32x4 {
    let one = f32x4::ONE;
    let blend = f32x4::splat(blend_scalar);
    match first {
        Waveform::Sine => {
            let sine = aligned_sine_phase4(phase);
            if blend_scalar <= f32::EPSILON {
                sine
            } else {
                let triangle = spline_triangle4_precomputed(
                    phase,
                    phase_step,
                    active,
                    support,
                    inverse_step,
                    optimized,
                );
                (triangle - sine).mul_add(blend, sine)
            }
        }
        Waveform::Triangle => {
            let triangle = spline_triangle4_precomputed(
                phase,
                phase_step,
                active,
                support,
                inverse_step,
                optimized,
            );
            let saw = phase * f32x4::splat(2.0)
                - one
                - spline_blep4_precomputed(phase, active, support, inverse_step, optimized);
            (saw - triangle).mul_add(blend, triangle) * f32x4::splat(morph_gain)
        }
        Waveform::Saw | Waveform::Pulse => {
            let saw = phase * f32x4::splat(2.0) - one;
            let pulse = phase.cmp_lt(width).blend(one, f32x4::splat(-1.0));
            let shifted = wrap_phase4(phase + one - width);
            let wrap = spline_blep4_precomputed(phase, active, support, inverse_step, optimized);
            let edge = spline_blep4_precomputed(shifted, active, support, inverse_step, optimized);
            if first == Waveform::Pulse {
                pulse + wrap - edge
            } else {
                let raw = (pulse - saw).mul_add(blend, saw);
                raw + (blend * f32x4::splat(2.0) - one).mul_add(wrap, -(blend * edge))
            }
        }
    }
}

pub fn accumulate_shape8_block_morphing<const SAMPLES: usize>(
    oscillators: &mut [VaOscillator],
    phase_step: f32x8,
    left_gain: f32x8,
    right_gain: f32x8,
    left: &mut [f32x8; SAMPLES],
    right: &mut [f32x8; SAMPLES],
    shapes: &[f32; SAMPLES],
    morph_gains: &[f32; SAMPLES],
    pulse_width: f32,
    antialiasing: Antialiasing,
) {
    debug_assert!(oscillators.len() >= 8);
    let mut phase = f32x8::from(std::array::from_fn(|index| oscillators[index].phase));
    if matches!(
        antialiasing,
        Antialiasing::Spline | Antialiasing::SplineOptimized
    ) {
        let one = f32x8::ONE;
        let active = phase_step.cmp_gt(f32x8::splat(f32::EPSILON));
        let support = phase_step * f32x8::splat(2.0);
        let inverse_step = one / active.blend(phase_step, one);
        let optimized = antialiasing == Antialiasing::SplineOptimized;
        let first = shape_segment(shapes[0].clamp(0.0, 3.0)).0;
        let same_segment = shapes
            .iter()
            .all(|shape| shape_segment(shape.clamp(0.0, 3.0)).0 == first);
        for frame in 0..SAMPLES {
            let current = phase;
            let next = phase + phase_step;
            phase = next.cmp_lt(one).blend(next, next - one);
            let sample = if same_segment {
                let blend = shapes[frame] - waveform_index(first);
                spline_shape8_segment_precomputed(
                    current,
                    phase_step,
                    active,
                    support,
                    inverse_step,
                    first,
                    blend,
                    morph_gains[frame],
                    pulse_width,
                    optimized,
                )
            } else {
                spline_shape8_precomputed(
                    current,
                    phase_step,
                    active,
                    support,
                    inverse_step,
                    shapes[frame],
                    morph_gains[frame],
                    pulse_width,
                    optimized,
                )
            };
            left[frame] = sample.mul_add(left_gain, left[frame]);
            right[frame] = sample.mul_add(right_gain, right[frame]);
        }
        let wrapped: [f32; 8] = phase.into();
        for (oscillator, phase) in oscillators.iter_mut().zip(wrapped) {
            oscillator.phase = phase;
        }
        return;
    }
    for frame in 0..SAMPLES {
        let current = phase;
        let next = phase + phase_step;
        phase = next.cmp_lt(f32x8::ONE).blend(next, next - f32x8::ONE);
        let sample = sample_shape8_at(
            current,
            phase_step,
            shapes[frame],
            pulse_width,
            antialiasing,
        );
        left[frame] = sample.mul_add(left_gain, left[frame]);
        right[frame] = sample.mul_add(right_gain, right[frame]);
    }
    let wrapped: [f32; 8] = phase.into();
    for (oscillator, phase) in oscillators.iter_mut().zip(wrapped) {
        oscillator.phase = phase;
    }
}

pub fn accumulate_shape8_block_dynamic<const SAMPLES: usize>(
    oscillators: &mut [VaOscillator],
    phase_steps: [f32x8; SAMPLES],
    left_gain: f32x8,
    right_gain: f32x8,
    left: &mut [f32x8; SAMPLES],
    right: &mut [f32x8; SAMPLES],
    shapes: &[f32; SAMPLES],
    morph_gains: &[f32; SAMPLES],
    pulse_width: f32,
    antialiasing: Antialiasing,
) {
    debug_assert!(oscillators.len() >= 8);
    let mut phase = f32x8::from(std::array::from_fn(|index| oscillators[index].phase));
    if matches!(
        antialiasing,
        Antialiasing::Spline | Antialiasing::SplineOptimized
    ) {
        let one = f32x8::ONE;
        let reference_step = (phase_steps[0] + phase_steps[SAMPLES - 1]) * f32x8::splat(0.5);
        let relative_drift = (phase_steps[SAMPLES - 1] - phase_steps[0]).abs()
            / reference_step.fast_max(f32x8::splat(f32::EPSILON));
        if relative_drift
            .cmp_lt(f32x8::splat(MAX_PRECOMPUTED_STEP_DRIFT))
            .all()
        {
            let refine_step = !relative_drift
                .cmp_lt(f32x8::splat(MAX_UNREFINED_STEP_DRIFT))
                .all();
            let active = reference_step.cmp_gt(f32x8::splat(f32::EPSILON));
            let support = reference_step * f32x8::splat(2.0);
            let inverse_step = one / active.blend(reference_step, one);
            let inverse_step_squared = inverse_step * inverse_step;
            let optimized = antialiasing == Antialiasing::SplineOptimized;
            let first = shape_segment(shapes[0].clamp(0.0, 3.0)).0;
            let same_segment = shapes
                .iter()
                .all(|shape| shape_segment(shape.clamp(0.0, 3.0)).0 == first);
            let frame_inverse_steps = if refine_step {
                std::array::from_fn(|frame| {
                    (reference_step - phase_steps[frame])
                        .mul_add(inverse_step_squared, inverse_step)
                })
            } else {
                [inverse_step; SAMPLES]
            };
            for frame in 0..SAMPLES {
                let frame_step = phase_steps[frame];
                let current = phase;
                let next = phase + frame_step;
                phase = next.cmp_lt(one).blend(next, next - one);
                let sample = if same_segment {
                    let blend = shapes[frame] - waveform_index(first);
                    spline_shape8_segment_precomputed(
                        current,
                        frame_step,
                        active,
                        support,
                        frame_inverse_steps[frame],
                        first,
                        blend,
                        morph_gains[frame],
                        pulse_width,
                        optimized,
                    )
                } else {
                    spline_shape8_precomputed(
                        current,
                        frame_step,
                        active,
                        support,
                        frame_inverse_steps[frame],
                        shapes[frame],
                        morph_gains[frame],
                        pulse_width,
                        optimized,
                    )
                };
                #[cfg(debug_assertions)]
                {
                    let exact = sample_shape8_at(
                        current,
                        frame_step,
                        shapes[frame],
                        pulse_width,
                        antialiasing,
                    );
                    let error: [f32; 8] = (sample - exact).abs().into();
                    debug_assert!(error.into_iter().all(|value| value < 1.0e-5));
                }
                left[frame] = sample.mul_add(left_gain, left[frame]);
                right[frame] = sample.mul_add(right_gain, right[frame]);
            }
            let wrapped: [f32; 8] = phase.into();
            for (oscillator, phase) in oscillators.iter_mut().zip(wrapped) {
                oscillator.phase = phase;
            }
            return;
        }
    }
    for frame in 0..SAMPLES {
        let current = phase;
        let next = phase + phase_steps[frame];
        phase = next.cmp_lt(f32x8::ONE).blend(next, next - f32x8::ONE);
        let sample = sample_shape8_at(
            current,
            phase_steps[frame],
            shapes[frame],
            pulse_width,
            antialiasing,
        );
        left[frame] = sample.mul_add(left_gain, left[frame]);
        right[frame] = sample.mul_add(right_gain, right[frame]);
    }
    let wrapped: [f32; 8] = phase.into();
    for (oscillator, phase) in oscillators.iter_mut().zip(wrapped) {
        oscillator.phase = phase;
    }
}

pub fn accumulate_saw4_block<const SAMPLES: usize>(
    oscillators: &mut [VaOscillator],
    phase_steps: [f32x4; SAMPLES],
    left_gain: f32x4,
    right_gain: f32x4,
    left: &mut [f32x8; SAMPLES],
    right: &mut [f32x8; SAMPLES],
    antialiasing: Antialiasing,
) {
    debug_assert!(oscillators.len() >= 4);
    let mut phase = f32x4::from(std::array::from_fn(|index| oscillators[index].phase));
    for frame in 0..SAMPLES {
        let current = phase;
        let next = phase + phase_steps[frame];
        phase = next.cmp_lt(f32x4::ONE).blend(next, next - f32x4::ONE);
        let sample = bandlimited_saw4(current, phase_steps[frame], antialiasing);
        add4_to8(&mut left[frame], sample * left_gain);
        add4_to8(&mut right[frame], sample * right_gain);
    }
    let wrapped: [f32; 4] = phase.into();
    for (oscillator, phase) in oscillators.iter_mut().zip(wrapped) {
        oscillator.phase = phase;
    }
}

pub fn accumulate_saw4_block_dynamic_gains<const SAMPLES: usize>(
    oscillators: &mut [VaOscillator],
    phase_steps: [f32x4; SAMPLES],
    left_gains: [f32x4; SAMPLES],
    right_gains: [f32x4; SAMPLES],
    left: &mut [f32x8; SAMPLES],
    right: &mut [f32x8; SAMPLES],
    antialiasing: Antialiasing,
) {
    debug_assert!(oscillators.len() >= 4);
    let mut phase = f32x4::from(std::array::from_fn(|index| oscillators[index].phase));
    for frame in 0..SAMPLES {
        let current = phase;
        let next = phase + phase_steps[frame];
        phase = next.cmp_lt(f32x4::ONE).blend(next, next - f32x4::ONE);
        let sample = bandlimited_saw4(current, phase_steps[frame], antialiasing);
        add4_to8(&mut left[frame], sample * left_gains[frame]);
        add4_to8(&mut right[frame], sample * right_gains[frame]);
    }
    let wrapped: [f32; 4] = phase.into();
    for (oscillator, phase) in oscillators.iter_mut().zip(wrapped) {
        oscillator.phase = phase;
    }
}

pub fn accumulate_saw4_block_static_gains<const SAMPLES: usize>(
    oscillators: &mut [VaOscillator],
    mut phase_step: f32x4,
    phase_step_delta: f32x4,
    left_gain: f32x4,
    right_gain: f32x4,
    left: &mut [f32x8; SAMPLES],
    right: &mut [f32x8; SAMPLES],
    antialiasing: Antialiasing,
) -> f32x4 {
    debug_assert!(oscillators.len() >= 4);
    let mut phase = f32x4::from(std::array::from_fn(|index| oscillators[index].phase));
    for frame in 0..SAMPLES {
        phase_step += phase_step_delta;
        let current = phase;
        let next = phase + phase_step;
        phase = next.cmp_lt(f32x4::ONE).blend(next, next - f32x4::ONE);
        let sample = bandlimited_saw4(current, phase_step, antialiasing);
        add4_to8(&mut left[frame], sample * left_gain);
        add4_to8(&mut right[frame], sample * right_gain);
    }
    let wrapped: [f32; 4] = phase.into();
    for (oscillator, phase) in oscillators.iter_mut().zip(wrapped) {
        oscillator.phase = phase;
    }
    phase_step
}

pub fn accumulate_saw4_block_constant<const SAMPLES: usize>(
    oscillators: &mut [VaOscillator],
    phase_step: f32x4,
    left_gain: f32x4,
    right_gain: f32x4,
    left: &mut [f32x8; SAMPLES],
    right: &mut [f32x8; SAMPLES],
    antialiasing: Antialiasing,
) {
    debug_assert!(oscillators.len() >= 4);
    let mut phase = f32x4::from(std::array::from_fn(|index| oscillators[index].phase));
    for frame in 0..SAMPLES {
        let current = phase;
        let next = phase + phase_step;
        phase = next.cmp_lt(f32x4::ONE).blend(next, next - f32x4::ONE);
        let sample = bandlimited_saw4(current, phase_step, antialiasing);
        add4_to8(&mut left[frame], sample * left_gain);
        add4_to8(&mut right[frame], sample * right_gain);
    }
    let wrapped: [f32; 4] = phase.into();
    for (oscillator, phase) in oscillators.iter_mut().zip(wrapped) {
        oscillator.phase = phase;
    }
}

pub fn accumulate_shape4_block_constant<const SAMPLES: usize>(
    oscillators: &mut [VaOscillator],
    phase_step: f32x4,
    left_gain: f32x4,
    right_gain: f32x4,
    left: &mut [f32x8; SAMPLES],
    right: &mut [f32x8; SAMPLES],
    shape: f32,
    pulse_width: f32,
    antialiasing: Antialiasing,
) {
    debug_assert!(oscillators.len() >= 4);
    let mut phase = f32x4::from(std::array::from_fn(|index| oscillators[index].phase));
    if shape == 0.0 {
        let one = f32x4::ONE;
        for frame in 0..SAMPLES {
            let current = phase;
            let next = phase + phase_step;
            phase = next.cmp_lt(one).blend(next, next - one);
            let sample = aligned_sine_phase4(current);
            add4_to8(&mut left[frame], sample * left_gain);
            add4_to8(&mut right[frame], sample * right_gain);
        }
        let wrapped: [f32; 4] = phase.into();
        for (oscillator, phase) in oscillators.iter_mut().zip(wrapped) {
            oscillator.phase = phase;
        }
        return;
    }
    if matches!(
        antialiasing,
        Antialiasing::Spline | Antialiasing::SplineOptimized
    ) && shape >= 2.0
    {
        let one = f32x4::ONE;
        let active = phase_step.cmp_gt(f32x4::splat(f32::EPSILON));
        let support = phase_step * f32x4::splat(2.0);
        let inverse_step = one / active.blend(phase_step, one);
        let optimized = antialiasing == Antialiasing::SplineOptimized;
        let (first, blend) = shape_segment(shape.clamp(0.0, 3.0));
        let gain = morph_gain(first, blend);
        let width = phase_step
            .fast_max(f32x4::splat(pulse_width.clamp(0.03, 0.97)))
            .fast_min(one - phase_step);
        for frame in 0..SAMPLES {
            let current = phase;
            let next = phase + phase_step;
            phase = next.cmp_lt(one).blend(next, next - one);
            let sample = spline_shape4_segment_precomputed_with_width(
                current,
                phase_step,
                active,
                support,
                inverse_step,
                first,
                blend,
                gain,
                width,
                optimized,
            );
            add4_to8(&mut left[frame], sample * left_gain);
            add4_to8(&mut right[frame], sample * right_gain);
        }
        let wrapped: [f32; 4] = phase.into();
        for (oscillator, phase) in oscillators.iter_mut().zip(wrapped) {
            oscillator.phase = phase;
        }
        return;
    }
    for frame in 0..SAMPLES {
        let current = phase;
        let next = phase + phase_step;
        phase = next.cmp_lt(f32x4::ONE).blend(next, next - f32x4::ONE);
        let sample = sample_shape4_at(current, phase_step, shape, pulse_width, antialiasing);
        add4_to8(&mut left[frame], sample * left_gain);
        add4_to8(&mut right[frame], sample * right_gain);
    }
    let wrapped: [f32; 4] = phase.into();
    for (oscillator, phase) in oscillators.iter_mut().zip(wrapped) {
        oscillator.phase = phase;
    }
}

pub fn accumulate_shape4_block_constant_warped<const SAMPLES: usize>(
    oscillators: &mut [VaOscillator],
    phase_step: f32x4,
    left_gain: f32x4,
    right_gain: f32x4,
    left: &mut [f32x8; SAMPLES],
    right: &mut [f32x8; SAMPLES],
    shape: f32,
    pulse_width: f32,
    antialiasing: Antialiasing,
    warp_mode: PhaseWarpMode,
    warp_amount: f32,
) {
    debug_assert!(oscillators.len() >= 4);
    let mut raw_phase = f32x4::from(std::array::from_fn(|index| oscillators[index].phase));
    let (pulse_edge, width) = if shape > 2.0 {
        (
            warped_pulse_edge4(phase_step, pulse_width, warp_mode, warp_amount),
            phase_step
                .fast_max(f32x4::splat(pulse_width.clamp(0.03, 0.97)))
                .fast_min(f32x4::ONE - phase_step),
        )
    } else {
        (None, f32x4::ZERO)
    };
    with_fixed_warp!(
        prepare_fixed_warp4(phase_step, warp_mode, warp_amount),
        PreparedWarp4,
        |warp| {
            for frame in 0..SAMPLES {
                let current = raw_phase;
                let next = current + phase_step;
                raw_phase = next.cmp_lt(f32x4::ONE).blend(next, next - f32x4::ONE);
                let (phase, warped_step) = warp.warp_phase(current);
                let sample = sample_shape4_warped_at_impl(
                    current,
                    phase_step,
                    phase,
                    warped_step,
                    shape,
                    pulse_width,
                    antialiasing,
                    width,
                    pulse_edge,
                );
                add4_to8(&mut left[frame], sample * left_gain);
                add4_to8(&mut right[frame], sample * right_gain);
            }
        }
    );
    let wrapped: [f32; 4] = raw_phase.into();
    for (oscillator, phase) in oscillators.iter_mut().zip(wrapped) {
        oscillator.phase = phase;
    }
}

pub fn accumulate_custom4_block_constant<const SAMPLES: usize>(
    oscillators: &mut [VaOscillator],
    phase_step: f32x4,
    left_gain: f32x4,
    right_gain: f32x4,
    left: &mut [f32x8; SAMPLES],
    right: &mut [f32x8; SAMPLES],
    curve: WaveCurveRt,
    mix: f32,
    shape: f32,
    pulse_width: f32,
    antialiasing: Antialiasing,
    warp_mode: PhaseWarpMode,
    warp_amount: f32,
) {
    let mut phase = f32x4::from(std::array::from_fn(|index| oscillators[index].phase));
    let (pulse_edge, width) = if !(mix >= 1.0) && shape > 2.0 {
        (
            warped_pulse_edge4(phase_step, pulse_width, warp_mode, warp_amount),
            phase_step
                .fast_max(f32x4::splat(pulse_width.clamp(0.03, 0.97)))
                .fast_min(f32x4::ONE - phase_step),
        )
    } else {
        (None, f32x4::ZERO)
    };
    with_fixed_warp!(
        prepare_fixed_warp4(phase_step, warp_mode, warp_amount),
        PreparedWarp4,
        |warp| {
            for frame in 0..SAMPLES {
                let current = phase;
                let next = phase + phase_step;
                phase = next.cmp_lt(f32x4::ONE).blend(next, next - f32x4::ONE);
                let sample = if mix >= 1.0 {
                    curve.eval4(warp.warp_position(current))
                } else {
                    let (warped_phase, warped_step) = warp.warp_phase(current);
                    let canonical = sample_shape4_warped_at_impl(
                        current,
                        phase_step,
                        warped_phase,
                        warped_step,
                        shape,
                        pulse_width,
                        antialiasing,
                        width,
                        pulse_edge,
                    );
                    (curve.eval4(warped_phase) - canonical).mul_add(f32x4::splat(mix), canonical)
                };
                add4_to8(&mut left[frame], sample * left_gain);
                add4_to8(&mut right[frame], sample * right_gain);
            }
        }
    );
    let wrapped: [f32; 4] = phase.into();
    for (oscillator, phase) in oscillators.iter_mut().zip(wrapped) {
        oscillator.phase = phase;
    }
}

pub fn accumulate_custom4_block<const SAMPLES: usize>(
    oscillators: &mut [VaOscillator],
    phase_steps: [f32x4; SAMPLES],
    left_gain: f32x4,
    right_gain: f32x4,
    left: &mut [f32x8; SAMPLES],
    right: &mut [f32x8; SAMPLES],
    curve: WaveCurveRt,
    mix: f32,
    shape: f32,
    pulse_width: f32,
    antialiasing: Antialiasing,
    warp_mode: PhaseWarpMode,
    warp_amount: f32,
) {
    let mut phase = f32x4::from(std::array::from_fn(|index| oscillators[index].phase));
    for frame in 0..SAMPLES {
        let current = phase;
        let next = phase + phase_steps[frame];
        phase = next.cmp_lt(f32x4::ONE).blend(next, next - f32x4::ONE);
        let sample = if mix >= 1.0 {
            curve.eval4(warp_phase_position4(
                current,
                phase_steps[frame],
                warp_mode,
                warp_amount,
            ))
        } else {
            let (warped_phase, warped_step) =
                warp_phase4(current, phase_steps[frame], warp_mode, warp_amount);
            let canonical = sample_shape4_warped_at_auto_edge(
                current,
                phase_steps[frame],
                warped_phase,
                warped_step,
                shape,
                pulse_width,
                antialiasing,
                warp_mode,
                warp_amount,
            );
            (curve.eval4(warped_phase) - canonical).mul_add(f32x4::splat(mix), canonical)
        };
        add4_to8(&mut left[frame], sample * left_gain);
        add4_to8(&mut right[frame], sample * right_gain);
    }
    let wrapped: [f32; 4] = phase.into();
    for (oscillator, phase) in oscillators.iter_mut().zip(wrapped) {
        oscillator.phase = phase;
    }
}

pub fn accumulate_shape4_block_morphing<const SAMPLES: usize>(
    oscillators: &mut [VaOscillator],
    phase_step: f32x4,
    left_gain: f32x4,
    right_gain: f32x4,
    left: &mut [f32x8; SAMPLES],
    right: &mut [f32x8; SAMPLES],
    shapes: &[f32; SAMPLES],
    morph_gains: &[f32; SAMPLES],
    pulse_width: f32,
    antialiasing: Antialiasing,
) {
    debug_assert!(oscillators.len() >= 4);
    let mut phase = f32x4::from(std::array::from_fn(|index| oscillators[index].phase));
    if matches!(
        antialiasing,
        Antialiasing::Spline | Antialiasing::SplineOptimized
    ) {
        let one = f32x4::ONE;
        let active = phase_step.cmp_gt(f32x4::splat(f32::EPSILON));
        let support = phase_step * f32x4::splat(2.0);
        let inverse_step = one / active.blend(phase_step, one);
        let optimized = antialiasing == Antialiasing::SplineOptimized;
        let first = shape_segment(shapes[0].clamp(0.0, 3.0)).0;
        let same_segment = shapes
            .iter()
            .all(|shape| shape_segment(shape.clamp(0.0, 3.0)).0 == first);
        for frame in 0..SAMPLES {
            let current = phase;
            let next = phase + phase_step;
            phase = next.cmp_lt(one).blend(next, next - one);
            let sample = if same_segment {
                let blend = shapes[frame] - waveform_index(first);
                spline_shape4_segment_precomputed(
                    current,
                    phase_step,
                    active,
                    support,
                    inverse_step,
                    first,
                    blend,
                    morph_gains[frame],
                    pulse_width,
                    optimized,
                )
            } else {
                spline_shape4_precomputed(
                    current,
                    phase_step,
                    active,
                    support,
                    inverse_step,
                    shapes[frame],
                    morph_gains[frame],
                    pulse_width,
                    optimized,
                )
            };
            add4_to8(&mut left[frame], sample * left_gain);
            add4_to8(&mut right[frame], sample * right_gain);
        }
        let wrapped: [f32; 4] = phase.into();
        for (oscillator, phase) in oscillators.iter_mut().zip(wrapped) {
            oscillator.phase = phase;
        }
        return;
    }
    for frame in 0..SAMPLES {
        let current = phase;
        let next = phase + phase_step;
        phase = next.cmp_lt(f32x4::ONE).blend(next, next - f32x4::ONE);
        let sample = sample_shape4_at(
            current,
            phase_step,
            shapes[frame],
            pulse_width,
            antialiasing,
        );
        add4_to8(&mut left[frame], sample * left_gain);
        add4_to8(&mut right[frame], sample * right_gain);
    }
    let wrapped: [f32; 4] = phase.into();
    for (oscillator, phase) in oscillators.iter_mut().zip(wrapped) {
        oscillator.phase = phase;
    }
}

pub fn accumulate_shape4_block_dynamic<const SAMPLES: usize>(
    oscillators: &mut [VaOscillator],
    phase_steps: [f32x4; SAMPLES],
    left_gain: f32x4,
    right_gain: f32x4,
    left: &mut [f32x8; SAMPLES],
    right: &mut [f32x8; SAMPLES],
    shapes: &[f32; SAMPLES],
    morph_gains: &[f32; SAMPLES],
    pulse_width: f32,
    antialiasing: Antialiasing,
) {
    debug_assert!(oscillators.len() >= 4);
    let mut phase = f32x4::from(std::array::from_fn(|index| oscillators[index].phase));
    if matches!(
        antialiasing,
        Antialiasing::Spline | Antialiasing::SplineOptimized
    ) {
        let one = f32x4::ONE;
        let reference_step = (phase_steps[0] + phase_steps[SAMPLES - 1]) * f32x4::splat(0.5);
        let relative_drift = (phase_steps[SAMPLES - 1] - phase_steps[0]).abs()
            / reference_step.fast_max(f32x4::splat(f32::EPSILON));
        if relative_drift
            .cmp_lt(f32x4::splat(MAX_PRECOMPUTED_STEP_DRIFT))
            .all()
        {
            let refine_step = !relative_drift
                .cmp_lt(f32x4::splat(MAX_UNREFINED_STEP_DRIFT))
                .all();
            let active = reference_step.cmp_gt(f32x4::splat(f32::EPSILON));
            let support = reference_step * f32x4::splat(2.0);
            let inverse_step = one / active.blend(reference_step, one);
            let inverse_step_squared = inverse_step * inverse_step;
            let optimized = antialiasing == Antialiasing::SplineOptimized;
            let first = shape_segment(shapes[0].clamp(0.0, 3.0)).0;
            let same_segment = shapes
                .iter()
                .all(|shape| shape_segment(shape.clamp(0.0, 3.0)).0 == first);
            let frame_inverse_steps = if refine_step {
                std::array::from_fn(|frame| {
                    (reference_step - phase_steps[frame])
                        .mul_add(inverse_step_squared, inverse_step)
                })
            } else {
                [inverse_step; SAMPLES]
            };
            for frame in 0..SAMPLES {
                let frame_step = phase_steps[frame];
                let current = phase;
                let next = phase + frame_step;
                phase = next.cmp_lt(one).blend(next, next - one);
                let sample = if same_segment {
                    let blend = shapes[frame] - waveform_index(first);
                    spline_shape4_segment_precomputed(
                        current,
                        frame_step,
                        active,
                        support,
                        frame_inverse_steps[frame],
                        first,
                        blend,
                        morph_gains[frame],
                        pulse_width,
                        optimized,
                    )
                } else {
                    spline_shape4_precomputed(
                        current,
                        frame_step,
                        active,
                        support,
                        frame_inverse_steps[frame],
                        shapes[frame],
                        morph_gains[frame],
                        pulse_width,
                        optimized,
                    )
                };
                #[cfg(debug_assertions)]
                {
                    let exact = sample_shape4_at(
                        current,
                        frame_step,
                        shapes[frame],
                        pulse_width,
                        antialiasing,
                    );
                    let error: [f32; 4] = (sample - exact).abs().into();
                    debug_assert!(error.into_iter().all(|value| value < 1.0e-5));
                }
                add4_to8(&mut left[frame], sample * left_gain);
                add4_to8(&mut right[frame], sample * right_gain);
            }
            let wrapped: [f32; 4] = phase.into();
            for (oscillator, phase) in oscillators.iter_mut().zip(wrapped) {
                oscillator.phase = phase;
            }
            return;
        }
    }
    for frame in 0..SAMPLES {
        let current = phase;
        let next = phase + phase_steps[frame];
        phase = next.cmp_lt(f32x4::ONE).blend(next, next - f32x4::ONE);
        let sample = sample_shape4_at(
            current,
            phase_steps[frame],
            shapes[frame],
            pulse_width,
            antialiasing,
        );
        add4_to8(&mut left[frame], sample * left_gain);
        add4_to8(&mut right[frame], sample * right_gain);
    }
    let wrapped: [f32; 4] = phase.into();
    for (oscillator, phase) in oscillators.iter_mut().zip(wrapped) {
        oscillator.phase = phase;
    }
}

#[inline]
fn add4_to8(output: &mut f32x8, contribution: f32x4) {
    let [a, b, c, d]: [f32; 4] = contribution.into();
    *output += f32x8::from([a, b, c, d, 0.0, 0.0, 0.0, 0.0]);
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "f64 phase is retained; oscillator output is intentionally f32"
)]
pub fn generate_pulse8(
    oscillators: &mut [VaOscillator],
    phase_steps: [f32; 8],
    pulse_width: f32,
    antialiasing: Antialiasing,
) -> f32x8 {
    let phases = advance8(oscillators, phase_steps);
    bandlimited_pulse8(phases, f32x8::from(phase_steps), pulse_width, antialiasing)
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "the oscillator and editor both consume f32 audio samples"
)]
pub fn generate_shape4(
    oscillators: &mut [VaOscillator],
    shape: f32,
    phase_steps: [f32; 4],
    pulse_width: f32,
    antialiasing: Antialiasing,
) -> f32x4 {
    let phases = advance4(oscillators, phase_steps);
    sample_shape4_at(
        phases,
        f32x4::from(phase_steps),
        shape,
        pulse_width,
        antialiasing,
    )
}

pub fn generate_shape4_warped(
    oscillators: &mut [VaOscillator],
    shape: f32,
    phase_steps: [f32; 4],
    pulse_width: f32,
    antialiasing: Antialiasing,
    warp_mode: PhaseWarpMode,
    warp_amount: f32,
) -> f32x4 {
    let raw_phases = advance4(oscillators, phase_steps);
    let raw_steps = f32x4::from(phase_steps);
    let (phases, warped_steps) = warp_phase4(raw_phases, raw_steps, warp_mode, warp_amount);
    sample_shape4_warped_at_auto_edge(
        raw_phases,
        raw_steps,
        phases,
        warped_steps,
        shape,
        pulse_width,
        antialiasing,
        warp_mode,
        warp_amount,
    )
}

pub fn generate_custom4(
    oscillators: &mut [VaOscillator],
    shape: f32,
    phase_steps: [f32; 4],
    pulse_width: f32,
    antialiasing: Antialiasing,
    warp_mode: PhaseWarpMode,
    warp_amount: f32,
    curve: WaveCurveRt,
    mix: f32,
) -> f32x4 {
    let raw_phases = advance4(oscillators, phase_steps);
    let raw_steps = f32x4::from(phase_steps);
    if mix >= 1.0 {
        curve.eval4(warp_phase_position4(
            raw_phases,
            raw_steps,
            warp_mode,
            warp_amount,
        ))
    } else {
        let (phases, steps) = warp_phase4(raw_phases, raw_steps, warp_mode, warp_amount);
        let custom = curve.eval4(phases);
        let canonical = sample_shape4_warped_at_auto_edge(
            raw_phases,
            raw_steps,
            phases,
            steps,
            shape,
            pulse_width,
            antialiasing,
            warp_mode,
            warp_amount,
        );
        (custom - canonical).mul_add(f32x4::splat(mix.clamp(0.0, 1.0)), canonical)
    }
}

fn sample_shape4_at(
    phases: f32x4,
    phase_steps: f32x4,
    shape: f32,
    pulse_width: f32,
    antialiasing: Antialiasing,
) -> f32x4 {
    let shape = shape.clamp(0.0, 3.0);
    let (first, blend) = shape_segment(shape);
    if blend > f32::EPSILON && first == Waveform::Saw {
        return bandlimited_saw_pulse_morph4(phases, phase_steps, pulse_width, blend, antialiasing);
    }
    let a = sample_waveform4(first, phases, phase_steps, pulse_width, antialiasing);
    if blend <= f32::EPSILON {
        a
    } else {
        let b = sample_waveform4(
            next_waveform(first),
            phases,
            phase_steps,
            pulse_width,
            antialiasing,
        );
        (b - a).mul_add(f32x4::splat(blend), a) * f32x4::splat(morph_gain(first, blend))
    }
}

#[inline]
fn sample_shape4_warped_at_auto_edge(
    raw_phase: f32x4,
    raw_step: f32x4,
    phase: f32x4,
    phase_step: f32x4,
    shape: f32,
    pulse_width: f32,
    antialiasing: Antialiasing,
    warp_mode: PhaseWarpMode,
    warp_amount: f32,
) -> f32x4 {
    let (pulse_edge, width) = if shape > 2.0 {
        let one = f32x4::ONE;
        (
            warped_pulse_edge4(raw_step, pulse_width, warp_mode, warp_amount),
            raw_step
                .fast_max(f32x4::splat(pulse_width.clamp(0.03, 0.97)))
                .fast_min(one - raw_step),
        )
    } else {
        (None, f32x4::ZERO)
    };
    sample_shape4_warped_at_impl(
        raw_phase,
        raw_step,
        phase,
        phase_step,
        shape,
        pulse_width,
        antialiasing,
        width,
        pulse_edge,
    )
}

#[inline]
fn sample_shape4_warped_at_impl(
    raw_phase: f32x4,
    raw_step: f32x4,
    phase: f32x4,
    phase_step: f32x4,
    shape: f32,
    pulse_width: f32,
    antialiasing: Antialiasing,
    width: f32x4,
    pulse_edge: Option<f32x4>,
) -> f32x4 {
    // See the eight-lane path: cycle-reset timing belongs to the raw phase clock.
    let shape = shape.clamp(0.0, 3.0);
    let (first, blend) = shape_segment(shape);
    if first == Waveform::Sine || first == Waveform::Triangle && blend <= f32::EPSILON {
        return sample_shape4_at(phase, phase_step, shape, pulse_width, antialiasing);
    }
    let sample = |waveform| match waveform {
        Waveform::Saw => {
            phase * f32x4::splat(2.0) - f32x4::ONE - edge_blep4(raw_phase, raw_step, antialiasing)
        }
        Waveform::Pulse => {
            let one = f32x4::ONE;
            let (shifted, edge_step) = pulse_edge.map_or_else(
                || (wrap_phase4(phase + one - width), phase_step),
                |edge| (wrap_phase4(raw_phase + one - edge), raw_step),
            );
            phase.cmp_lt(width).blend(one, -one) + edge_blep4(raw_phase, raw_step, antialiasing)
                - edge_blep4(shifted, edge_step, antialiasing)
        }
        _ => sample_waveform4(waveform, phase, phase_step, pulse_width, antialiasing),
    };
    let a = sample(first);
    if blend <= f32::EPSILON {
        a
    } else {
        let b = sample(next_waveform(first));
        (b - a).mul_add(f32x4::splat(blend), a) * f32x4::splat(morph_gain(first, blend))
    }
}

pub fn generate_shape4_pair(
    oscillators: &mut [VaOscillator],
    shape: f32,
    phase_steps: [[f32; 4]; 2],
    pulse_width: f32,
    antialiasing: Antialiasing,
) -> [f32x4; 2] {
    let [phases0, phases1] = advance4_pair(oscillators, phase_steps);
    [
        sample_shape4_at(
            phases0,
            f32x4::from(phase_steps[0]),
            shape,
            pulse_width,
            antialiasing,
        ),
        sample_shape4_at(
            phases1,
            f32x4::from(phase_steps[1]),
            shape,
            pulse_width,
            antialiasing,
        ),
    ]
}

pub fn generate_shape4_pair_warped(
    oscillators: &mut [VaOscillator],
    shape: f32,
    phase_steps: [[f32; 4]; 2],
    pulse_width: f32,
    antialiasing: Antialiasing,
    warp_mode: PhaseWarpMode,
    warp_amount: f32,
) -> [f32x4; 2] {
    let [raw_phases0, raw_phases1] = advance4_pair(oscillators, phase_steps);
    let (phases0, steps0) = warp_phase4(
        raw_phases0,
        f32x4::from(phase_steps[0]),
        warp_mode,
        warp_amount,
    );
    let (phases1, steps1) = warp_phase4(
        raw_phases1,
        f32x4::from(phase_steps[1]),
        warp_mode,
        warp_amount,
    );
    let raw_steps0 = f32x4::from(phase_steps[0]);
    let raw_steps1 = f32x4::from(phase_steps[1]);
    [
        sample_shape4_warped_at_auto_edge(
            raw_phases0,
            raw_steps0,
            phases0,
            steps0,
            shape,
            pulse_width,
            antialiasing,
            warp_mode,
            warp_amount,
        ),
        sample_shape4_warped_at_auto_edge(
            raw_phases1,
            raw_steps1,
            phases1,
            steps1,
            shape,
            pulse_width,
            antialiasing,
            warp_mode,
            warp_amount,
        ),
    ]
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "the oscillator output intentionally enters the f32 audio path"
)]
pub fn generate_triangle4(
    oscillators: &mut [VaOscillator],
    phase_steps: [f32; 4],
    antialiasing: Antialiasing,
) -> f32x4 {
    let phases = advance4(oscillators, phase_steps);
    bandlimited_triangle4(phases, f32x4::from(phase_steps), antialiasing)
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "the oscillator output intentionally enters the f32 audio path"
)]
pub fn generate_saw4(
    oscillators: &mut [VaOscillator],
    phase_steps: [f32; 4],
    antialiasing: Antialiasing,
) -> f32x4 {
    bandlimited_saw4(
        advance4(oscillators, phase_steps),
        f32x4::from(phase_steps),
        antialiasing,
    )
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "the oscillator output intentionally enters the f32 audio path"
)]
pub fn generate_pulse4(
    oscillators: &mut [VaOscillator],
    phase_steps: [f32; 4],
    pulse_width: f32,
    antialiasing: Antialiasing,
) -> f32x4 {
    bandlimited_pulse4(
        advance4(oscillators, phase_steps),
        f32x4::from(phase_steps),
        pulse_width,
        antialiasing,
    )
}

#[inline]

fn advance4(oscillators: &mut [VaOscillator], phase_steps: [f32; 4]) -> f32x4 {
    debug_assert!(oscillators.len() >= 4);
    let phases = f32x4::from([
        oscillators[0].phase,
        oscillators[1].phase,
        oscillators[2].phase,
        oscillators[3].phase,
    ]);
    let next = phases + f32x4::from(phase_steps);
    let wrapped = next.cmp_lt(f32x4::ONE).blend(next, next - f32x4::ONE);
    let wrapped: [f32; 4] = wrapped.into();
    for (oscillator, phase) in oscillators.iter_mut().zip(wrapped) {
        oscillator.phase = phase;
    }
    phases
}

fn advance8(oscillators: &mut [VaOscillator], phase_steps: [f32; 8]) -> f32x8 {
    debug_assert!(oscillators.len() >= 8);
    let phases = f32x8::from(std::array::from_fn(|index| oscillators[index].phase));
    let next = phases + f32x8::from(phase_steps);
    let wrapped = next.cmp_lt(f32x8::ONE).blend(next, next - f32x8::ONE);
    let wrapped: [f32; 8] = wrapped.into();
    for (oscillator, phase) in oscillators.iter_mut().zip(wrapped) {
        oscillator.phase = phase;
    }
    phases
}

fn advance4_pair(oscillators: &mut [VaOscillator], phase_steps: [[f32; 4]; 2]) -> [f32x4; 2] {
    debug_assert!(oscillators.len() >= 4);
    let phases0 = f32x4::from([
        oscillators[0].phase,
        oscillators[1].phase,
        oscillators[2].phase,
        oscillators[3].phase,
    ]);
    let next0 = phases0 + f32x4::from(phase_steps[0]);
    let phases1 = next0.cmp_lt(f32x4::ONE).blend(next0, next0 - f32x4::ONE);
    let next1 = phases1 + f32x4::from(phase_steps[1]);
    let wrapped = next1.cmp_lt(f32x4::ONE).blend(next1, next1 - f32x4::ONE);
    let wrapped: [f32; 4] = wrapped.into();
    for (oscillator, phase) in oscillators.iter_mut().zip(wrapped) {
        oscillator.phase = phase;
    }
    [phases0, phases1]
}

fn advance8_pair(oscillators: &mut [VaOscillator], phase_steps: [[f32; 8]; 2]) -> [f32x8; 2] {
    debug_assert!(oscillators.len() >= 8);
    let phases0 = f32x8::from(std::array::from_fn(|index| oscillators[index].phase));
    let next0 = phases0 + f32x8::from(phase_steps[0]);
    let phases1 = next0.cmp_lt(f32x8::ONE).blend(next0, next0 - f32x8::ONE);
    let next1 = phases1 + f32x8::from(phase_steps[1]);
    let wrapped = next1.cmp_lt(f32x8::ONE).blend(next1, next1 - f32x8::ONE);
    let wrapped: [f32; 8] = wrapped.into();
    for (oscillator, phase) in oscillators.iter_mut().zip(wrapped) {
        oscillator.phase = phase;
    }
    [phases0, phases1]
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "the oscillator and editor both consume f32 audio samples"
)]
#[cfg(test)]
pub fn sample_shape(shape: f32, phase: f64, phase_step: f64, pulse_width: f32) -> f32 {
    sample_shape_with_antialiasing(shape, phase, phase_step, pulse_width, Antialiasing::Legacy)
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "the oscillator and editor both consume f32 audio samples"
)]
#[cfg(test)]
pub fn sample_shape_with_antialiasing(
    shape: f32,
    phase: f64,
    phase_step: f64,
    pulse_width: f32,
    antialiasing: Antialiasing,
) -> f32 {
    sample_shape_normalized(shape, wrap01(phase), phase_step, pulse_width, antialiasing)
}

pub fn sample_custom_shape_with_antialiasing_warped(
    shape: f32,
    phase: f64,
    phase_step: f64,
    pulse_width: f32,
    antialiasing: Antialiasing,
    warp_mode: PhaseWarpMode,
    warp_amount: f32,
    curve: WaveCurveRt,
    mix: f32,
) -> f32 {
    let raw_phase = wrap01(phase) as f32;
    let raw_step = phase_step as f32;
    if mix >= 1.0 {
        return curve.eval(warp_phase_position_scalar(
            raw_phase,
            raw_step,
            warp_mode,
            warp_amount,
        ));
    }
    let (phase, phase_step) = warp_phase_scalar(raw_phase, raw_step, warp_mode, warp_amount);
    let canonical = sample_shape_normalized_warped_auto_edge(
        shape,
        f64::from(raw_phase),
        f64::from(raw_step),
        f64::from(phase),
        f64::from(phase_step),
        pulse_width,
        antialiasing,
        warp_mode,
        warp_amount,
    );
    (curve.eval(phase) - canonical).mul_add(mix.clamp(0.0, 1.0), canonical)
}

pub(super) fn sample_shape_normalized(
    shape: f32,
    phase: f64,
    phase_step: f64,
    pulse_width: f32,
    antialiasing: Antialiasing,
) -> f32 {
    let shape = shape.clamp(0.0, 3.0);
    let (first, blend) = shape_segment(shape);
    let a = sample_waveform_with_antialiasing(first, phase, phase_step, pulse_width, antialiasing);
    if blend <= f32::EPSILON {
        return a;
    }
    let second = next_waveform(first);
    let b = sample_waveform_with_antialiasing(second, phase, phase_step, pulse_width, antialiasing);
    blend.mul_add(b - a, a) * morph_gain(first, blend)
}

#[inline]
#[allow(
    clippy::cast_possible_truncation,
    reason = "raw_step originated as f32 and is converted back for the scalar inverse"
)]
pub(super) fn sample_shape_normalized_warped_auto_edge(
    shape: f32,
    raw_phase: f64,
    raw_step: f64,
    phase: f64,
    phase_step: f64,
    pulse_width: f32,
    antialiasing: Antialiasing,
    warp_mode: PhaseWarpMode,
    warp_amount: f32,
) -> f32 {
    let pulse_edge = if shape > 2.0 {
        warped_pulse_edge_scalar(raw_step as f32, pulse_width, warp_mode, warp_amount)
            .map(f64::from)
    } else {
        None
    };
    sample_shape_normalized_warped_impl(
        shape,
        raw_phase,
        raw_step,
        phase,
        phase_step,
        pulse_width,
        antialiasing,
        pulse_edge,
    )
}

#[inline]
fn sample_shape_normalized_warped_impl(
    shape: f32,
    raw_phase: f64,
    raw_step: f64,
    phase: f64,
    phase_step: f64,
    pulse_width: f32,
    antialiasing: Antialiasing,
    pulse_edge: Option<f64>,
) -> f32 {
    // See the SIMD paths: phase warp does not move the raw cycle boundary.
    let shape = shape.clamp(0.0, 3.0);
    let (first, blend) = shape_segment(shape);
    if first == Waveform::Sine || first == Waveform::Triangle && blend <= f32::EPSILON {
        return sample_shape_normalized(shape, phase, phase_step, pulse_width, antialiasing);
    }
    let sample = |waveform| match waveform {
        Waveform::Saw => {
            (2.0_f64.mul_add(phase, -1.0) - edge_blep(raw_phase, raw_step, antialiasing)) as f32
        }
        Waveform::Pulse => {
            let width_step = if pulse_edge.is_some() {
                raw_step
            } else {
                phase_step
            };
            let minimum_width = width_step.max(0.03);
            let width = f64::from(pulse_width).clamp(minimum_width, 1.0 - minimum_width);
            let (shifted, edge_step) = pulse_edge.map_or_else(
                || (wrap01(phase + 1.0 - width), phase_step),
                |edge| (wrap01(raw_phase + 1.0 - edge), raw_step),
            );
            let sample = if phase < width { 1.0 } else { -1.0 };
            (sample + edge_blep(raw_phase, raw_step, antialiasing)
                - edge_blep(shifted, edge_step, antialiasing)) as f32
        }
        _ => sample_waveform_with_antialiasing(
            waveform,
            phase,
            phase_step,
            pulse_width,
            antialiasing,
        ),
    };
    let a = sample(first);
    if blend <= f32::EPSILON {
        a
    } else {
        let b = sample(next_waveform(first));
        blend.mul_add(b - a, a) * morph_gain(first, blend)
    }
}

fn shape_segment(shape: f32) -> (Waveform, f32) {
    if shape < 1.0 {
        (Waveform::Sine, shape)
    } else if shape < 2.0 {
        (Waveform::Triangle, shape - 1.0)
    } else if shape < 3.0 {
        (Waveform::Saw, shape - 2.0)
    } else {
        (Waveform::Pulse, 0.0)
    }
}

const fn waveform_index(waveform: Waveform) -> f32 {
    match waveform {
        Waveform::Sine => 0.0,
        Waveform::Triangle => 1.0,
        Waveform::Saw => 2.0,
        Waveform::Pulse => 3.0,
    }
}

fn morph_gain(first: Waveform, blend: f32) -> f32 {
    if first != Waveform::Triangle {
        return 1.0;
    }
    let inverse = 1.0 - blend;
    inverse.mul_add(inverse, blend * blend).sqrt().recip()
}

pub fn shape_morph_gain(shape: f32) -> f32 {
    let (first, blend) = shape_segment(shape.clamp(0.0, 3.0));
    morph_gain(first, blend)
}

const fn next_waveform(waveform: Waveform) -> Waveform {
    match waveform {
        Waveform::Sine => Waveform::Triangle,
        Waveform::Triangle => Waveform::Saw,
        Waveform::Saw | Waveform::Pulse => Waveform::Pulse,
    }
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "the oscillator and editor both consume f32 audio samples"
)]
#[cfg(test)]
fn sample_waveform_normalized(
    waveform: Waveform,
    phase: f64,
    phase_step: f64,
    pulse_width: f32,
) -> f32 {
    sample_waveform_with_antialiasing(
        waveform,
        phase,
        phase_step,
        pulse_width,
        Antialiasing::Legacy,
    )
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "the oscillator and editor both consume f32 audio samples"
)]
fn sample_waveform_with_antialiasing(
    waveform: Waveform,
    phase: f64,
    phase_step: f64,
    pulse_width: f32,
    antialiasing: Antialiasing,
) -> f32 {
    match waveform {
        Waveform::Saw => bandlimited_saw(phase, phase_step, antialiasing) as f32,
        Waveform::Pulse => {
            bandlimited_pulse(phase, phase_step, f64::from(pulse_width), antialiasing) as f32
        }
        Waveform::Triangle => bandlimited_triangle(phase, phase_step, antialiasing) as f32,
        Waveform::Sine => aligned_sine_phase(phase as f32),
    }
}

fn sample_waveform4(
    waveform: Waveform,
    phase: f32x4,
    phase_step: f32x4,
    pulse_width: f32,
    antialiasing: Antialiasing,
) -> f32x4 {
    match waveform {
        Waveform::Saw => bandlimited_saw4(phase, phase_step, antialiasing),
        Waveform::Pulse => bandlimited_pulse4(phase, phase_step, pulse_width, antialiasing),
        Waveform::Triangle => bandlimited_triangle4(phase, phase_step, antialiasing),
        Waveform::Sine => aligned_sine_phase4(phase),
    }
}

fn sample_waveform8(
    waveform: Waveform,
    phase: f32x8,
    phase_step: f32x8,
    pulse_width: f32,
    antialiasing: Antialiasing,
) -> f32x8 {
    match waveform {
        Waveform::Saw => bandlimited_saw8(phase, phase_step, antialiasing),
        Waveform::Pulse => bandlimited_pulse8(phase, phase_step, pulse_width, antialiasing),
        Waveform::Triangle => bandlimited_triangle8(phase, phase_step, antialiasing),
        Waveform::Sine => aligned_sine_phase8(phase),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Antialiasing, Waveform, bandlimited_pulse, bandlimited_pulse8, bandlimited_saw,
        bandlimited_saw8, bandlimited_triangle, bandlimited_triangle8, sample_shape,
        sample_waveform_normalized,
    };
    use truce_simd::simd::f32x8;

    #[test]
    fn shape_midpoint_is_real_sample_interpolation() {
        let phase = 0.37;
        let phase_step = 220.0 / 48_000.0;
        let sine = sample_waveform_normalized(Waveform::Sine, phase, phase_step, 0.5);
        let triangle = sample_waveform_normalized(Waveform::Triangle, phase, phase_step, 0.5);
        let midpoint = sample_shape(0.5, phase, phase_step, 0.5);
        assert!((midpoint - (sine + triangle) * 0.5).abs() < 1.0e-6);
    }

    #[test]
    fn lagrange_scalar_and_simd_paths_match() {
        let phases = [0.0, 0.009, 0.031, 0.49, 0.509, 0.73, 0.969, 0.991];
        let step = 0.017_f32;
        let phase8 = f32x8::from(phases);
        let step8 = f32x8::splat(step);
        let saw8: [f32; 8] = bandlimited_saw8(phase8, step8, Antialiasing::Lagrange).into();
        let pulse8: [f32; 8] =
            bandlimited_pulse8(phase8, step8, 0.37, Antialiasing::Lagrange).into();
        let triangle8: [f32; 8] =
            bandlimited_triangle8(phase8, step8, Antialiasing::Lagrange).into();

        for (index, phase) in phases.into_iter().enumerate() {
            let phase = f64::from(phase);
            let step = f64::from(step);
            assert!(
                (f64::from(saw8[index]) - bandlimited_saw(phase, step, Antialiasing::Lagrange))
                    .abs()
                    < 2.0e-6
            );
            assert!(
                (f64::from(pulse8[index])
                    - bandlimited_pulse(phase, step, 0.37, Antialiasing::Lagrange))
                .abs()
                    < 2.0e-6
            );
            assert!(
                (f64::from(triangle8[index])
                    - bandlimited_triangle(phase, step, Antialiasing::Lagrange))
                .abs()
                    < 2.0e-6
            );
        }
    }
}
