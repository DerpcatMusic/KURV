//! Virtual-analog rendering, sampling, and block generation.

use truce_simd::simd::{f32x4, f32x8};
use wide::{CmpGt, CmpLt};

use crate::wave_curve::WaveCurveRt;

use super::antialias::{
    aligned_sine_phase, aligned_sine_phase4, aligned_sine_phase8, bandlimited_pulse,
    bandlimited_pulse4, bandlimited_pulse8, bandlimited_saw, bandlimited_saw_pulse_morph4,
    bandlimited_saw_pulse_morph8, bandlimited_saw4, bandlimited_saw8, bandlimited_triangle,
    bandlimited_triangle4, bandlimited_triangle8, edge_blep, edge_blep4, edge_blep8,
    spline_blep4_precomputed, spline_blep8_precomputed, spline_saw4_narrow, spline_saw8_narrow,
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
    let phase_steps = f32x8::from(phase_steps);
    if shape == 2.0 {
        return bandlimited_saw8(phases, phase_steps, antialiasing);
    }
    if shape == 3.0 {
        return generate_pulse8_dynamic(phases, phase_steps, pulse_width, antialiasing);
    }
    sample_shape8_at(phases, phase_steps, shape, pulse_width, antialiasing)
}

pub fn accumulate_shape8_phase_modulated_block<const SAMPLES: usize>(
    oscillators: &mut [VaOscillator],
    shape: f32,
    phase_steps: [f32; 8],
    phase_modulation: &[f32; SAMPLES],
    pulse_width: f32,
    antialiasing: Antialiasing,
    left_gains: [f32; 8],
    right_gains: [f32; 8],
    output: &mut [(f32, f32); SAMPLES],
) {
    debug_assert!(oscillators.len() >= 8);
    if shape == 2.0
        && phase_steps.iter().all(|step| *step < 0.25)
        && matches!(
            antialiasing,
            Antialiasing::Spline | Antialiasing::SplineOptimized
        )
        && super::backend::accumulate_saw8_phase_modulated(
            oscillators,
            phase_steps,
            phase_modulation,
            antialiasing == Antialiasing::SplineOptimized,
            left_gains,
            right_gains,
            output,
        )
    {
        return;
    }
    let mut phase = f32x8::from(std::array::from_fn(|index| oscillators[index].phase));
    let phase_steps = f32x8::from(phase_steps);
    let narrow = phase_steps.cmp_lt(f32x8::splat(0.25)).all();
    for frame in 0..SAMPLES {
        let current = phase + f32x8::splat(phase_modulation[frame]);
        let current = current
            .cmp_lt(f32x8::ZERO)
            .blend(current + f32x8::ONE, current);
        let current = wrap_phase8(current);
        phase += phase_steps;
        phase = phase.cmp_lt(f32x8::ONE).blend(phase, phase - f32x8::ONE);
        let samples: [f32; 8] = if shape == 2.0 {
            if narrow
                && matches!(
                    antialiasing,
                    Antialiasing::Spline | Antialiasing::SplineOptimized
                )
            {
                spline_saw8_narrow(
                    current,
                    phase_steps,
                    antialiasing == Antialiasing::SplineOptimized,
                )
            } else {
                bandlimited_saw8(current, phase_steps, antialiasing)
            }
        } else {
            sample_shape8_at(current, phase_steps, shape, pulse_width, antialiasing)
        }
        .into();
        for lane in 0..8 {
            output[frame].0 += samples[lane] * left_gains[lane];
            output[frame].1 += samples[lane] * right_gains[lane];
        }
    }
    for (oscillator, phase) in oscillators.iter_mut().zip(<[f32; 8]>::from(phase)) {
        oscillator.phase = phase;
    }
}

pub fn accumulate_spline_saw8_phase_modulated_block<const SAMPLES: usize>(
    oscillators: &mut [VaOscillator],
    phase_steps: [f32; 8],
    phase_modulation: &[f32; SAMPLES],
    optimized: bool,
    left_gains: [f32; 8],
    right_gains: [f32; 8],
    output: &mut [(f32, f32); SAMPLES],
) {
    debug_assert!(oscillators.len() >= 8);
    if super::backend::accumulate_saw8_phase_modulated(
        oscillators,
        phase_steps,
        phase_modulation,
        optimized,
        left_gains,
        right_gains,
        output,
    ) {
        return;
    }
    let mut phase = f32x8::from(std::array::from_fn(|index| oscillators[index].phase));
    let phase_steps = f32x8::from(phase_steps);
    debug_assert!(phase_steps.cmp_lt(f32x8::splat(0.25)).all());
    for frame in 0..SAMPLES {
        let current = phase + f32x8::splat(phase_modulation[frame]);
        let current = current
            .cmp_lt(f32x8::ZERO)
            .blend(current + f32x8::ONE, current);
        let current = wrap_phase8(current);
        phase += phase_steps;
        phase = phase.cmp_lt(f32x8::ONE).blend(phase, phase - f32x8::ONE);
        let samples: [f32; 8] = spline_saw8_narrow(current, phase_steps, optimized).into();
        for lane in 0..8 {
            output[frame].0 += samples[lane] * left_gains[lane];
            output[frame].1 += samples[lane] * right_gains[lane];
        }
    }
    for (oscillator, phase) in oscillators.iter_mut().zip(<[f32; 8]>::from(phase)) {
        oscillator.phase = phase;
    }
}

pub fn accumulate_spline_saw8_phase_modulated_lanes_block<const SAMPLES: usize>(
    oscillators: &mut [VaOscillator],
    phase_steps: [f32; 8],
    phase_modulation: Option<&[f32; SAMPLES]>,
    optimized: bool,
    left_gain: f32x8,
    right_gain: f32x8,
    left: &mut [f32x8; SAMPLES],
    right: &mut [f32x8; SAMPLES],
) {
    debug_assert!(oscillators.len() >= 8);
    if let Some(pm) = phase_modulation {
        if super::backend::accumulate_saw8_phase_modulated_lanes(
            oscillators,
            phase_steps,
            pm,
            optimized,
            left_gain.into(),
            right_gain.into(),
            left,
            right,
        ) {
            return;
        }
    }
    let mut phase = f32x8::from(std::array::from_fn(|index| oscillators[index].phase));
    let phase_steps = f32x8::from(phase_steps);
    debug_assert!(phase_steps.cmp_lt(f32x8::splat(0.25)).all());
    if let Some(phase_modulation) = phase_modulation {
        for frame in 0..SAMPLES {
            let current = phase + f32x8::splat(phase_modulation[frame]);
            let current = current
                .cmp_lt(f32x8::ZERO)
                .blend(current + f32x8::ONE, current);
            let current = wrap_phase8(current);
            phase += phase_steps;
            phase = phase.cmp_lt(f32x8::ONE).blend(phase, phase - f32x8::ONE);
            let samples = spline_saw8_narrow(current, phase_steps, optimized);
            left[frame] = samples.mul_add(left_gain, left[frame]);
            right[frame] = samples.mul_add(right_gain, right[frame]);
        }
    } else {
        for frame in 0..SAMPLES {
            let current = phase;
            phase += phase_steps;
            phase = phase.cmp_lt(f32x8::ONE).blend(phase, phase - f32x8::ONE);
            let samples = spline_saw8_narrow(current, phase_steps, optimized);
            left[frame] = samples.mul_add(left_gain, left[frame]);
            right[frame] = samples.mul_add(right_gain, right[frame]);
        }
    }
    for (oscillator, phase) in oscillators.iter_mut().zip(<[f32; 8]>::from(phase)) {
        oscillator.phase = phase;
    }
}

pub fn accumulate_spline_saw4_phase_modulated_block<const SAMPLES: usize>(
    oscillators: &mut [VaOscillator],
    phase_steps: [f32; 4],
    phase_modulation: &[f32; SAMPLES],
    optimized: bool,
    left_gains: [f32; 4],
    right_gains: [f32; 4],
    output: &mut [(f32, f32); SAMPLES],
) {
    debug_assert!(oscillators.len() >= 4);
    let mut phase = f32x4::from(std::array::from_fn(|index| oscillators[index].phase));
    let phase_steps = f32x4::from(phase_steps);
    debug_assert!(phase_steps.cmp_lt(f32x4::splat(0.25)).all());
    for frame in 0..SAMPLES {
        let current = phase + f32x4::splat(phase_modulation[frame]);
        let current = current
            .cmp_lt(f32x4::ZERO)
            .blend(current + f32x4::ONE, current);
        let current = wrap_phase4(current);
        phase += phase_steps;
        phase = phase.cmp_lt(f32x4::ONE).blend(phase, phase - f32x4::ONE);
        let samples: [f32; 4] = spline_saw4_narrow(current, phase_steps, optimized).into();
        for lane in 0..4 {
            output[frame].0 += samples[lane] * left_gains[lane];
            output[frame].1 += samples[lane] * right_gains[lane];
        }
    }
    for (oscillator, phase) in oscillators.iter_mut().zip(<[f32; 4]>::from(phase)) {
        oscillator.phase = phase;
    }
}

#[inline(never)]
fn generate_pulse8_dynamic(
    phases: f32x8,
    phase_steps: f32x8,
    pulse_width: f32,
    antialiasing: Antialiasing,
) -> f32x8 {
    bandlimited_pulse8(phases, phase_steps, pulse_width, antialiasing)
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
    sample_shape8_at_prepared(
        phases,
        phase_steps,
        prepare_shape8(shape),
        pulse_width,
        antialiasing,
    )
}

#[derive(Clone, Copy)]
pub(super) struct PreparedShape8 {
    shape: f32,
    first: Waveform,
    blend: f32,
    gain: f32,
}

#[inline]
pub(super) fn prepare_shape8(shape: f32) -> PreparedShape8 {
    let shape = shape.clamp(0.0, 3.0);
    let (first, blend) = shape_segment(shape);
    PreparedShape8 {
        shape,
        first,
        blend,
        gain: if blend > f32::EPSILON {
            morph_gain(first, blend)
        } else {
            1.0
        },
    }
}

#[inline]
pub(super) fn sample_shape8_at_prepared(
    phases: f32x8,
    phase_steps: f32x8,
    prepared: PreparedShape8,
    pulse_width: f32,
    antialiasing: Antialiasing,
) -> f32x8 {
    let PreparedShape8 {
        first, blend, gain, ..
    } = prepared;
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
        (b - a).mul_add(f32x8::splat(blend), a) * f32x8::splat(gain)
    }
}

#[inline]
pub(super) fn sample_shape8_warped_at_auto_edge(
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
    sample_shape8_warped_at_auto_edge_prepared(
        raw_phase,
        raw_step,
        phase,
        phase_step,
        prepare_shape8(shape),
        pulse_width,
        antialiasing,
        warp_mode,
        warp_amount,
    )
}

#[inline]
pub(super) fn sample_shape8_warped_at_auto_edge_prepared(
    raw_phase: f32x8,
    raw_step: f32x8,
    phase: f32x8,
    phase_step: f32x8,
    prepared: PreparedShape8,
    pulse_width: f32,
    antialiasing: Antialiasing,
    warp_mode: PhaseWarpMode,
    warp_amount: f32,
) -> f32x8 {
    let (pulse_edge, width) = if prepared.shape > 2.0 {
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
        prepared,
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
    prepared: PreparedShape8,
    pulse_width: f32,
    antialiasing: Antialiasing,
    width: f32x8,
    pulse_edge: Option<f32x8>,
) -> f32x8 {
    // The warp changes sample position, not the time of the cycle reset. Keep the
    // BLEP centered on raw phase so its fractional discontinuity time stays exact.
    let PreparedShape8 {
        first, blend, gain, ..
    } = prepared;
    if first == Waveform::Sine || first == Waveform::Triangle && blend <= f32::EPSILON {
        return sample_shape8_at_prepared(phase, phase_step, prepared, pulse_width, antialiasing);
    }
    let wrap_correction = edge_blep8(raw_phase, raw_step, antialiasing);
    let sample = |waveform| match waveform {
        Waveform::Saw => phase * f32x8::splat(2.0) - f32x8::ONE - wrap_correction,
        Waveform::Pulse => {
            let one = f32x8::ONE;
            let (shifted, edge_step) = pulse_edge.map_or_else(
                || (wrap_phase8(phase + one - width), phase_step),
                |edge| (wrap_phase8(raw_phase + one - edge), raw_step),
            );
            phase.cmp_lt(width).blend(one, -one) + wrap_correction
                - edge_blep8(shifted, edge_step, antialiasing)
        }
        _ => sample_waveform8(waveform, phase, phase_step, pulse_width, antialiasing),
    };
    let a = sample(first);
    if blend <= f32::EPSILON {
        a
    } else {
        let b = sample(next_waveform(first));
        (b - a).mul_add(f32x8::splat(blend), a) * f32x8::splat(gain)
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
    if shape > 2.0 && shape < 3.0 {
        let one = f32x8::ONE;
        let blend = f32x8::splat(shape - 2.0);
        with_fixed_warp!(
            prepare_fixed_warp8(phase_step, warp_mode, warp_amount),
            PreparedWarp8,
            |warp| {
                for frame in 0..SAMPLES {
                    let current = raw_phase;
                    let next = current + phase_step;
                    raw_phase = next.cmp_lt(one).blend(next, next - one);
                    let (phase, warped_step) = warp.warp_phase(current);
                    let wrap_correction = edge_blep8(current, phase_step, antialiasing);
                    let saw = phase * f32x8::splat(2.0) - one - wrap_correction;
                    let (shifted, edge_step) = pulse_edge.map_or_else(
                        || (wrap_phase8(phase + one - width), warped_step),
                        |edge| (wrap_phase8(current + one - edge), phase_step),
                    );
                    let pulse = phase.cmp_lt(width).blend(one, -one) + wrap_correction
                        - edge_blep8(shifted, edge_step, antialiasing);
                    let sample = (pulse - saw).mul_add(blend, saw) * one;
                    left[frame] = sample.mul_add(left_gain, left[frame]);
                    right[frame] = sample.mul_add(right_gain, right[frame]);
                }
            }
        );
        let wrapped: [f32; 8] = raw_phase.into();
        for (oscillator, phase) in oscillators.iter_mut().zip(wrapped) {
            oscillator.phase = phase;
        }
        return;
    }
    let prepared_shape = prepare_shape8(shape);
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
                    prepared_shape,
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

pub fn accumulate_shape8_block_steps<const SAMPLES: usize>(
    oscillators: &mut [VaOscillator],
    phase_steps: [f32x8; SAMPLES],
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
    if super::backend::accumulate_shape8_block_steps_avx2(
        oscillators,
        &phase_steps,
        left_gain,
        right_gain,
        left,
        right,
        shape,
        pulse_width,
        antialiasing,
        warp_mode,
        warp_amount,
    ) {
        return;
    }
    debug_assert!(oscillators.len() >= 8);
    let warped = warp_mode != PhaseWarpMode::None && warp_amount > f32::EPSILON;
    let prepared_shape = prepare_shape8(shape);
    let mut phase = f32x8::from(std::array::from_fn(|index| oscillators[index].phase));
    for frame in 0..SAMPLES {
        let step = phase_steps[frame];
        let current = phase;
        let next = phase + step;
        phase = next.cmp_lt(f32x8::ONE).blend(next, next - f32x8::ONE);
        let sample = if warped {
            let (warped_phase, warped_step) = warp_phase8(current, step, warp_mode, warp_amount);
            sample_shape8_warped_at_auto_edge_prepared(
                current,
                step,
                warped_phase,
                warped_step,
                prepared_shape,
                pulse_width,
                antialiasing,
                warp_mode,
                warp_amount,
            )
        } else {
            sample_shape8_at_prepared(current, step, prepared_shape, pulse_width, antialiasing)
        };
        left[frame] = sample.mul_add(left_gain, left[frame]);
        right[frame] = sample.mul_add(right_gain, right[frame]);
    }
    let wrapped: [f32; 8] = phase.into();
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
    let prepared_shape = prepare_shape8(shape);
    with_fixed_warp!(
        prepare_fixed_warp8(phase_step, warp_mode, warp_amount),
        PreparedWarp8,
        |warp| {
            if mix >= 1.0 {
                for frame in 0..SAMPLES {
                    let current = phase;
                    let next = phase + phase_step;
                    phase = next.cmp_lt(f32x8::ONE).blend(next, next - f32x8::ONE);
                    let sample = curve.eval8(warp.warp_position(current));
                    left[frame] = sample.mul_add(left_gain, left[frame]);
                    right[frame] = sample.mul_add(right_gain, right[frame]);
                }
            } else if shape == 2.0 {
                let one = f32x8::ONE;
                let active = phase_step.cmp_gt(f32x8::splat(f32::EPSILON));
                let support = phase_step * f32x8::splat(2.0);
                let inverse_step = one / active.blend(phase_step, one);
                let optimized = antialiasing == Antialiasing::SplineOptimized;
                let mix_vector = f32x8::splat(mix);
                for frame in 0..SAMPLES {
                    let current = phase;
                    let next = phase + phase_step;
                    phase = next.cmp_lt(one).blend(next, next - one);
                    let (warped_phase, _) = warp.warp_phase(current);
                    let canonical = warped_phase * f32x8::splat(2.0)
                        - one
                        - spline_blep8_precomputed(
                            current,
                            active,
                            support,
                            inverse_step,
                            optimized,
                        );
                    let sample =
                        (curve.eval8(warped_phase) - canonical).mul_add(mix_vector, canonical);
                    left[frame] = sample.mul_add(left_gain, left[frame]);
                    right[frame] = sample.mul_add(right_gain, right[frame]);
                }
            } else if shape == 3.0 {
                let one = f32x8::ONE;
                let active = phase_step.cmp_gt(f32x8::splat(f32::EPSILON));
                let support = phase_step * f32x8::splat(2.0);
                let inverse_step = one / active.blend(phase_step, one);
                let optimized = antialiasing == Antialiasing::SplineOptimized;
                let mix_vector = f32x8::splat(mix);
                for frame in 0..SAMPLES {
                    let current = phase;
                    let next = phase + phase_step;
                    phase = next.cmp_lt(f32x8::ONE).blend(next, next - f32x8::ONE);
                    let (warped_phase, warped_step) = warp.warp_phase(current);
                    let wrap_correction =
                        spline_blep8_precomputed(current, active, support, inverse_step, optimized);
                    let shifted = pulse_edge
                        .map_or_else(
                            || (wrap_phase8(warped_phase + one - width), warped_step),
                            |edge| (wrap_phase8(current + one - edge), phase_step),
                        )
                        .0;
                    let canonical = warped_phase.cmp_lt(width).blend(one, -one) + wrap_correction
                        - spline_blep8_precomputed(
                            shifted,
                            active,
                            support,
                            inverse_step,
                            optimized,
                        );
                    let sample =
                        (curve.eval8(warped_phase) - canonical).mul_add(mix_vector, canonical);
                    left[frame] = sample.mul_add(left_gain, left[frame]);
                    right[frame] = sample.mul_add(right_gain, right[frame]);
                }
            } else {
                let mix_vector = f32x8::splat(mix);
                for frame in 0..SAMPLES {
                    let current = phase;
                    let next = phase + phase_step;
                    phase = next.cmp_lt(f32x8::ONE).blend(next, next - f32x8::ONE);
                    let (warped_phase, warped_step) = warp.warp_phase(current);
                    let canonical = sample_shape8_warped_at_impl(
                        current,
                        phase_step,
                        warped_phase,
                        warped_step,
                        prepared_shape,
                        pulse_width,
                        antialiasing,
                        width,
                        pulse_edge,
                    );
                    let sample =
                        (curve.eval8(warped_phase) - canonical).mul_add(mix_vector, canonical);
                    left[frame] = sample.mul_add(left_gain, left[frame]);
                    right[frame] = sample.mul_add(right_gain, right[frame]);
                }
            }
        }
    );
    let wrapped: [f32; 8] = phase.into();
    for (oscillator, phase) in oscillators.iter_mut().zip(wrapped) {
        oscillator.phase = phase;
    }
}

#[cfg(test)]
#[allow(
    clippy::too_many_arguments,
    reason = "the paired probe changes only constant custom-x8 BLEP preparation"
)]
pub fn accumulate_custom8_block_constant_unprepared_blep_probe<const SAMPLES: usize>(
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
    let prepared_shape = prepare_shape8(shape);
    with_fixed_warp!(
        prepare_fixed_warp8(phase_step, warp_mode, warp_amount),
        PreparedWarp8,
        |warp| {
            if mix >= 1.0 {
                for frame in 0..SAMPLES {
                    let current = phase;
                    let next = phase + phase_step;
                    phase = next.cmp_lt(f32x8::ONE).blend(next, next - f32x8::ONE);
                    let sample = curve.eval8(warp.warp_position(current));
                    left[frame] = sample.mul_add(left_gain, left[frame]);
                    right[frame] = sample.mul_add(right_gain, right[frame]);
                }
            } else if shape == 2.0 {
                let mix_vector = f32x8::splat(mix);
                for frame in 0..SAMPLES {
                    let current = phase;
                    let next = phase + phase_step;
                    phase = next.cmp_lt(f32x8::ONE).blend(next, next - f32x8::ONE);
                    let (warped_phase, _) = warp.warp_phase(current);
                    let canonical = warped_phase * f32x8::splat(2.0)
                        - f32x8::ONE
                        - edge_blep8(current, phase_step, antialiasing);
                    let sample =
                        (curve.eval8(warped_phase) - canonical).mul_add(mix_vector, canonical);
                    left[frame] = sample.mul_add(left_gain, left[frame]);
                    right[frame] = sample.mul_add(right_gain, right[frame]);
                }
            } else if shape == 3.0 {
                let mix_vector = f32x8::splat(mix);
                for frame in 0..SAMPLES {
                    let current = phase;
                    let next = phase + phase_step;
                    phase = next.cmp_lt(f32x8::ONE).blend(next, next - f32x8::ONE);
                    let (warped_phase, warped_step) = warp.warp_phase(current);
                    let wrap_correction = edge_blep8(current, phase_step, antialiasing);
                    let one = f32x8::ONE;
                    let (shifted, edge_step) = pulse_edge.map_or_else(
                        || (wrap_phase8(warped_phase + one - width), warped_step),
                        |edge| (wrap_phase8(current + one - edge), phase_step),
                    );
                    let canonical = warped_phase.cmp_lt(width).blend(one, -one) + wrap_correction
                        - edge_blep8(shifted, edge_step, antialiasing);
                    let sample =
                        (curve.eval8(warped_phase) - canonical).mul_add(mix_vector, canonical);
                    left[frame] = sample.mul_add(left_gain, left[frame]);
                    right[frame] = sample.mul_add(right_gain, right[frame]);
                }
            } else {
                let mix_vector = f32x8::splat(mix);
                for frame in 0..SAMPLES {
                    let current = phase;
                    let next = phase + phase_step;
                    phase = next.cmp_lt(f32x8::ONE).blend(next, next - f32x8::ONE);
                    let (warped_phase, warped_step) = warp.warp_phase(current);
                    let canonical = sample_shape8_warped_at_impl(
                        current,
                        phase_step,
                        warped_phase,
                        warped_step,
                        prepared_shape,
                        pulse_width,
                        antialiasing,
                        width,
                        pulse_edge,
                    );
                    let sample =
                        (curve.eval8(warped_phase) - canonical).mul_add(mix_vector, canonical);
                    left[frame] = sample.mul_add(left_gain, left[frame]);
                    right[frame] = sample.mul_add(right_gain, right[frame]);
                }
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
    let prepared_shape = prepare_shape8(shape);
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
            let canonical = sample_shape8_warped_at_auto_edge_prepared(
                current,
                phase_steps[frame],
                warped_phase,
                warped_step,
                prepared_shape,
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

pub fn accumulate_shape4_block_steps<const SAMPLES: usize>(
    oscillators: &mut [VaOscillator],
    phase_steps: [f32x4; SAMPLES],
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
    let warped = warp_mode != PhaseWarpMode::None && warp_amount > f32::EPSILON;
    let mut phase = f32x4::from(std::array::from_fn(|index| oscillators[index].phase));
    for frame in 0..SAMPLES {
        let step = phase_steps[frame];
        let current = phase;
        let next = phase + step;
        phase = next.cmp_lt(f32x4::ONE).blend(next, next - f32x4::ONE);
        let sample = if warped {
            let (warped_phase, warped_step) = warp_phase4(current, step, warp_mode, warp_amount);
            sample_shape4_warped_at_auto_edge(
                current,
                step,
                warped_phase,
                warped_step,
                shape,
                pulse_width,
                antialiasing,
                warp_mode,
                warp_amount,
            )
        } else {
            sample_shape4_at(current, step, shape, pulse_width, antialiasing)
        };
        add4_to8(&mut left[frame], sample * left_gain);
        add4_to8(&mut right[frame], sample * right_gain);
    }
    let wrapped: [f32; 4] = phase.into();
    for (oscillator, phase) in oscillators.iter_mut().zip(wrapped) {
        oscillator.phase = phase;
    }
}

#[cfg(test)]
#[allow(
    clippy::too_many_arguments,
    reason = "the paired probe preserves the former constant custom-x4 BLEP path"
)]
pub fn accumulate_custom4_block_constant_unprepared_blep_probe<const SAMPLES: usize>(
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
    let prepared_shape = shape.clamp(0.0, 3.0);
    let (first, blend_scalar) = shape_segment(prepared_shape);
    let blend = f32x4::splat(blend_scalar);
    let shape_gain = f32x4::splat(morph_gain(first, blend_scalar));
    let mix_vector = f32x4::splat(mix);
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
                    let canonical = sample_shape4_warped_at_prepared_impl(
                        current,
                        phase_step,
                        warped_phase,
                        warped_step,
                        prepared_shape,
                        pulse_width,
                        antialiasing,
                        width,
                        pulse_edge,
                        first,
                        blend_scalar,
                        blend,
                        shape_gain,
                    );
                    (curve.eval4(warped_phase) - canonical).mul_add(mix_vector, canonical)
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
    let prepared_shape = shape.clamp(0.0, 3.0);
    let (first, blend_scalar) = shape_segment(prepared_shape);
    let blend = f32x4::splat(blend_scalar);
    let shape_gain = f32x4::splat(morph_gain(first, blend_scalar));
    let mix_vector = f32x4::splat(mix);
    with_fixed_warp!(
        prepare_fixed_warp4(phase_step, warp_mode, warp_amount),
        PreparedWarp4,
        |warp| {
            if mix >= 1.0 {
                for frame in 0..SAMPLES {
                    let current = phase;
                    let next = phase + phase_step;
                    phase = next.cmp_lt(f32x4::ONE).blend(next, next - f32x4::ONE);
                    let sample = curve.eval4(warp.warp_position(current));
                    add4_to8(&mut left[frame], sample * left_gain);
                    add4_to8(&mut right[frame], sample * right_gain);
                }
            } else if shape == 2.0 {
                let one = f32x4::ONE;
                let active = phase_step.cmp_gt(f32x4::splat(f32::EPSILON));
                let support = phase_step * f32x4::splat(2.0);
                let inverse_step = one / active.blend(phase_step, one);
                let optimized = antialiasing == Antialiasing::SplineOptimized;
                for frame in 0..SAMPLES {
                    let current = phase;
                    let next = phase + phase_step;
                    phase = next.cmp_lt(one).blend(next, next - one);
                    let (warped_phase, _) = warp.warp_phase(current);
                    let canonical = warped_phase * f32x4::splat(2.0)
                        - one
                        - spline_blep4_precomputed(
                            current,
                            active,
                            support,
                            inverse_step,
                            optimized,
                        );
                    let sample =
                        (curve.eval4(warped_phase) - canonical).mul_add(mix_vector, canonical);
                    add4_to8(&mut left[frame], sample * left_gain);
                    add4_to8(&mut right[frame], sample * right_gain);
                }
            } else if shape == 3.0 {
                let one = f32x4::ONE;
                let active = phase_step.cmp_gt(f32x4::splat(f32::EPSILON));
                let support = phase_step * f32x4::splat(2.0);
                let inverse_step = one / active.blend(phase_step, one);
                let optimized = antialiasing == Antialiasing::SplineOptimized;
                for frame in 0..SAMPLES {
                    let current = phase;
                    let next = phase + phase_step;
                    phase = next.cmp_lt(one).blend(next, next - one);
                    let (warped_phase, warped_step) = warp.warp_phase(current);
                    let wrap_correction =
                        spline_blep4_precomputed(current, active, support, inverse_step, optimized);
                    let shifted = pulse_edge
                        .map_or_else(
                            || (wrap_phase4(warped_phase + one - width), warped_step),
                            |edge| (wrap_phase4(current + one - edge), phase_step),
                        )
                        .0;
                    let canonical = warped_phase.cmp_lt(width).blend(one, -one) + wrap_correction
                        - spline_blep4_precomputed(
                            shifted,
                            active,
                            support,
                            inverse_step,
                            optimized,
                        );
                    let sample =
                        (curve.eval4(warped_phase) - canonical).mul_add(mix_vector, canonical);
                    add4_to8(&mut left[frame], sample * left_gain);
                    add4_to8(&mut right[frame], sample * right_gain);
                }
            } else {
                for frame in 0..SAMPLES {
                    let current = phase;
                    let next = phase + phase_step;
                    phase = next.cmp_lt(f32x4::ONE).blend(next, next - f32x4::ONE);
                    let (warped_phase, warped_step) = warp.warp_phase(current);
                    let canonical = sample_shape4_warped_at_prepared_impl(
                        current,
                        phase_step,
                        warped_phase,
                        warped_step,
                        prepared_shape,
                        pulse_width,
                        antialiasing,
                        width,
                        pulse_edge,
                        first,
                        blend_scalar,
                        blend,
                        shape_gain,
                    );
                    let sample =
                        (curve.eval4(warped_phase) - canonical).mul_add(mix_vector, canonical);
                    add4_to8(&mut left[frame], sample * left_gain);
                    add4_to8(&mut right[frame], sample * right_gain);
                }
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

/// Renders eight consecutive samples from one oscillator in one SIMD evaluation.
/// The phase accumulator remains scalar so this is suitable for feed-forward
/// audio-rate modulation without cloning oscillator state.
pub fn generate_shape_time8(
    oscillator: &mut VaOscillator,
    shape: f32,
    phase_step: f32,
    phase_modulation: [f32; 8],
    pulse_width: f32,
    antialiasing: Antialiasing,
) -> [f32; 8] {
    let mut phase = oscillator.phase;
    let raw_phases: [f32; 8] = if phase_step * 8.0 < 1.0 - phase - 8.0 * f32::EPSILON {
        std::array::from_fn(|_| {
            let current = phase;
            phase += phase_step;
            current
        })
    } else {
        std::array::from_fn(|_| {
            let current = phase;
            phase += phase_step;
            if phase >= 1.0 {
                phase -= 1.0;
            }
            current
        })
    };
    oscillator.phase = phase;
    let phases = f32x8::from(raw_phases) + f32x8::from(phase_modulation);
    let phases = phases
        .cmp_lt(f32x8::ZERO)
        .blend(phases + f32x8::ONE, phases);
    let phases = wrap_phase8(phases);
    let phase_steps = f32x8::splat(phase_step);
    if shape == 2.0 {
        if phase_step < 0.25
            && matches!(
                antialiasing,
                Antialiasing::Spline | Antialiasing::SplineOptimized
            )
        {
            return spline_saw8_narrow(
                phases,
                phase_steps,
                antialiasing == Antialiasing::SplineOptimized,
            )
            .into();
        }
        return bandlimited_saw8(phases, phase_steps, antialiasing).into();
    }
    sample_shape8_at(phases, phase_steps, shape, pulse_width, antialiasing).into()
}

/// Renders eight consecutive samples with sample-varying phase increments.
pub fn generate_shape_time8_steps(
    oscillator: &mut VaOscillator,
    shape: f32,
    phase_steps: [f32; 8],
    phase_modulation: [f32; 8],
    pulse_width: f32,
    antialiasing: Antialiasing,
) -> [f32; 8] {
    let mut phase = oscillator.phase;
    let raw_phases: [f32; 8] = std::array::from_fn(|index| {
        let current = phase;
        phase += phase_steps[index];
        if phase >= 1.0 {
            phase -= 1.0;
        }
        current
    });
    oscillator.phase = phase;
    let phases = f32x8::from(raw_phases) + f32x8::from(phase_modulation);
    let phases = phases
        .cmp_lt(f32x8::ZERO)
        .blend(phases + f32x8::ONE, phases);
    let phases = wrap_phase8(phases);
    let phase_steps = f32x8::from(phase_steps);
    if shape == 2.0 {
        return bandlimited_saw8(phases, phase_steps, antialiasing).into();
    }
    sample_shape8_at(phases, phase_steps, shape, pulse_width, antialiasing).into()
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
    sample_shape4_warped_at_prepared_impl(
        raw_phase,
        raw_step,
        phase,
        phase_step,
        shape,
        pulse_width,
        antialiasing,
        width,
        pulse_edge,
        first,
        blend,
        f32x4::splat(blend),
        f32x4::splat(morph_gain(first, blend)),
    )
}

#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn sample_shape4_warped_at_prepared_impl(
    raw_phase: f32x4,
    raw_step: f32x4,
    phase: f32x4,
    phase_step: f32x4,
    shape: f32,
    pulse_width: f32,
    antialiasing: Antialiasing,
    width: f32x4,
    pulse_edge: Option<f32x4>,
    first: Waveform,
    blend_scalar: f32,
    blend: f32x4,
    shape_gain: f32x4,
) -> f32x4 {
    if first == Waveform::Sine || first == Waveform::Triangle && blend_scalar <= f32::EPSILON {
        return sample_shape4_at(phase, phase_step, shape, pulse_width, antialiasing);
    }
    let wrap_correction = edge_blep4(raw_phase, raw_step, antialiasing);
    let sample = |waveform| match waveform {
        Waveform::Saw => phase * f32x4::splat(2.0) - f32x4::ONE - wrap_correction,
        Waveform::Pulse => {
            let one = f32x4::ONE;
            let (shifted, edge_step) = pulse_edge.map_or_else(
                || (wrap_phase4(phase + one - width), phase_step),
                |edge| (wrap_phase4(raw_phase + one - edge), raw_step),
            );
            phase.cmp_lt(width).blend(one, -one) + wrap_correction
                - edge_blep4(shifted, edge_step, antialiasing)
        }
        _ => sample_waveform4(waveform, phase, phase_step, pulse_width, antialiasing),
    };
    let a = sample(first);
    if blend_scalar <= f32::EPSILON {
        a
    } else {
        let b = sample(next_waveform(first));
        (b - a).mul_add(blend, a) * shape_gain
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
pub(super) fn sample_shape_normalized_warped_impl(
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
        Antialiasing, PhaseWarpMode, VaOscillator, Waveform, accumulate_custom4_block_constant,
        accumulate_custom4_block_constant_unprepared_blep_probe, accumulate_custom8_block_constant,
        accumulate_custom8_block_constant_unprepared_blep_probe, bandlimited_pulse,
        bandlimited_pulse8, bandlimited_saw, bandlimited_saw8, bandlimited_triangle,
        bandlimited_triangle8, sample_shape, sample_waveform_normalized,
    };
    use truce_simd::simd::{f32x4, f32x8};
    use wide::{CmpGt, CmpLt};

    const PROBE_SEGMENTS: usize = 16;
    const PROBE_SLEW: u16 = 1024;

    fn assert_custom_constant_blep_bit_identity<const SAMPLES: usize>() {
        let curve = crate::wave_curve::WaveCurveRt::default();
        let left_gain = f32x8::from([0.71, -0.42, 0.33, 0.25, -0.18, 0.91, -0.57, 0.64]);
        let right_gain = f32x8::from([-0.29, 0.81, 0.44, -0.63, 0.52, 0.17, 0.38, -0.76]);
        for steps in [
            [0.0, 0.001, 0.004, 0.009, 0.013, 0.021, 0.034, 0.055],
            [0.119, 0.181, 0.249, 0.251, 0.307, 0.361, 0.419, 0.45],
        ] {
            let phase_step = f32x8::from(steps);
            for antialiasing in [
                Antialiasing::Legacy,
                Antialiasing::Spline,
                Antialiasing::SplineOptimized,
                Antialiasing::Lagrange,
                Antialiasing::Spectral,
            ] {
                for (warp_mode, warp_amount) in [
                    (PhaseWarpMode::None, 0.0),
                    (PhaseWarpMode::Pwm, 0.71),
                    (PhaseWarpMode::PhaseBend, 1.0),
                    (PhaseWarpMode::Harmonic, 0.43),
                ] {
                    for (shape, width) in [(2.0, 0.5), (3.0, 0.31), (3.0, 0.5)] {
                        let oscillators: [VaOscillator; 8] = std::array::from_fn(|lane| {
                            let mut oscillator = VaOscillator::default();
                            oscillator.phase = (0.017 + lane as f32 * 0.127) % 1.0;
                            oscillator
                        });
                        let mut precomputed_oscillators = oscillators;
                        let mut unprepared_oscillators = oscillators;
                        for block in 0..64 {
                            let initial_left: [f32x8; SAMPLES] = std::array::from_fn(|frame| {
                                f32x8::splat((block * SAMPLES + frame) as f32 * 1.0e-5 - 0.125)
                            });
                            let initial_right: [f32x8; SAMPLES] = std::array::from_fn(|frame| {
                                f32x8::splat(0.25 - (block * SAMPLES + frame) as f32 * 7.0e-6)
                            });
                            let mut precomputed_left = initial_left;
                            let mut precomputed_right = initial_right;
                            let mut unprepared_left = initial_left;
                            let mut unprepared_right = initial_right;
                            accumulate_custom8_block_constant(
                                &mut precomputed_oscillators,
                                phase_step,
                                left_gain,
                                right_gain,
                                &mut precomputed_left,
                                &mut precomputed_right,
                                curve,
                                0.63,
                                shape,
                                width,
                                antialiasing,
                                warp_mode,
                                warp_amount,
                            );
                            accumulate_custom8_block_constant_unprepared_blep_probe(
                                &mut unprepared_oscillators,
                                phase_step,
                                left_gain,
                                right_gain,
                                &mut unprepared_left,
                                &mut unprepared_right,
                                curve,
                                0.63,
                                shape,
                                width,
                                antialiasing,
                                warp_mode,
                                warp_amount,
                            );
                            for frame in 0..SAMPLES {
                                for lane in 0..8 {
                                    assert_eq!(
                                        <[f32; 8]>::from(precomputed_left[frame])[lane].to_bits(),
                                        <[f32; 8]>::from(unprepared_left[frame])[lane].to_bits(),
                                        "left mismatch: samples={SAMPLES}, block={block}, frame={frame}, lane={lane}, shape={shape}, width={width}, antialiasing={antialiasing:?}, warp={warp_mode:?}"
                                    );
                                    assert_eq!(
                                        <[f32; 8]>::from(precomputed_right[frame])[lane].to_bits(),
                                        <[f32; 8]>::from(unprepared_right[frame])[lane].to_bits(),
                                        "right mismatch: samples={SAMPLES}, block={block}, frame={frame}, lane={lane}, shape={shape}, width={width}, antialiasing={antialiasing:?}, warp={warp_mode:?}"
                                    );
                                }
                            }
                            for lane in 0..8 {
                                assert_eq!(
                                    precomputed_oscillators[lane].phase.to_bits(),
                                    unprepared_oscillators[lane].phase.to_bits(),
                                    "phase mismatch: samples={SAMPLES}, block={block}, lane={lane}, shape={shape}, width={width}, antialiasing={antialiasing:?}, warp={warp_mode:?}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    #[ignore = "constant custom-x8 precomputed BLEP bit-identity experiment"]
    fn custom_constant_precomputed_blep_bit_identity() {
        assert_custom_constant_blep_bit_identity::<24>();
        assert_custom_constant_blep_bit_identity::<32>();
    }

    fn assert_custom4_constant_blep_bit_identity<const SAMPLES: usize>() {
        let curve = crate::wave_curve::WaveCurveRt::default();
        let left_gain = f32x4::from([0.71, -0.42, 0.33, 0.25]);
        let right_gain = f32x4::from([-0.29, 0.81, 0.44, -0.63]);
        for steps in [[0.0, 0.001, 0.021, 0.055], [0.249, 0.251, 0.361, 0.45]] {
            let phase_step = f32x4::from(steps);
            for antialiasing in [
                Antialiasing::Legacy,
                Antialiasing::Spline,
                Antialiasing::SplineOptimized,
                Antialiasing::Lagrange,
                Antialiasing::Spectral,
            ] {
                for (warp_mode, warp_amount) in [
                    (PhaseWarpMode::None, 0.0),
                    (PhaseWarpMode::Pwm, 0.71),
                    (PhaseWarpMode::PhaseBend, 1.0),
                    (PhaseWarpMode::Harmonic, 0.43),
                ] {
                    for (shape, width) in [(2.0, 0.5), (3.0, 0.31), (3.0, 0.5)] {
                        let oscillators: [VaOscillator; 4] = std::array::from_fn(|lane| {
                            let mut oscillator = VaOscillator::default();
                            oscillator.phase = (0.017 + lane as f32 * 0.127) % 1.0;
                            oscillator
                        });
                        let mut current_oscillators = oscillators;
                        let mut candidate_oscillators = oscillators;
                        for block in 0..64 {
                            let initial_left: [f32x8; SAMPLES] = std::array::from_fn(|frame| {
                                f32x8::splat((block * SAMPLES + frame) as f32 * 1.0e-5 - 0.125)
                            });
                            let initial_right: [f32x8; SAMPLES] = std::array::from_fn(|frame| {
                                f32x8::splat(0.25 - (block * SAMPLES + frame) as f32 * 7.0e-6)
                            });
                            let mut current_left = initial_left;
                            let mut current_right = initial_right;
                            let mut candidate_left = initial_left;
                            let mut candidate_right = initial_right;
                            accumulate_custom4_block_constant(
                                &mut current_oscillators,
                                phase_step,
                                left_gain,
                                right_gain,
                                &mut current_left,
                                &mut current_right,
                                curve,
                                0.63,
                                shape,
                                width,
                                antialiasing,
                                warp_mode,
                                warp_amount,
                            );
                            accumulate_custom4_block_constant_unprepared_blep_probe(
                                &mut candidate_oscillators,
                                phase_step,
                                left_gain,
                                right_gain,
                                &mut candidate_left,
                                &mut candidate_right,
                                curve,
                                0.63,
                                shape,
                                width,
                                antialiasing,
                                warp_mode,
                                warp_amount,
                            );
                            for frame in 0..SAMPLES {
                                let current_left_lanes: [f32; 8] = current_left[frame].into();
                                let current_right_lanes: [f32; 8] = current_right[frame].into();
                                let candidate_left_lanes: [f32; 8] = candidate_left[frame].into();
                                let candidate_right_lanes: [f32; 8] = candidate_right[frame].into();
                                for lane in 0..8 {
                                    assert_eq!(
                                        candidate_left_lanes[lane].to_bits(),
                                        current_left_lanes[lane].to_bits(),
                                        "x4 left mismatch: samples={SAMPLES}, block={block}, frame={frame}, lane={lane}, shape={shape}, width={width}, antialiasing={antialiasing:?}, warp={warp_mode:?}"
                                    );
                                    assert_eq!(
                                        candidate_right_lanes[lane].to_bits(),
                                        current_right_lanes[lane].to_bits(),
                                        "x4 right mismatch: samples={SAMPLES}, block={block}, frame={frame}, lane={lane}, shape={shape}, width={width}, antialiasing={antialiasing:?}, warp={warp_mode:?}"
                                    );
                                }
                            }
                            for lane in 0..4 {
                                assert_eq!(
                                    candidate_oscillators[lane].phase.to_bits(),
                                    current_oscillators[lane].phase.to_bits(),
                                    "x4 phase mismatch: samples={SAMPLES}, block={block}, lane={lane}, shape={shape}, width={width}, antialiasing={antialiasing:?}, warp={warp_mode:?}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    #[ignore = "constant custom-x4 precomputed BLEP bit-identity experiment"]
    fn custom4_constant_precomputed_blep_bit_identity() {
        assert_custom4_constant_blep_bit_identity::<24>();
        assert_custom4_constant_blep_bit_identity::<32>();
    }

    #[derive(Clone, Copy)]
    struct ProbeCurve([f32; 64]);

    impl ProbeCurve {
        #[inline]
        fn eval8(self, phase: f32x8) -> f32x8 {
            #[cfg(all(
                target_arch = "x86_64",
                target_feature = "avx2",
                target_feature = "fma"
            ))]
            unsafe {
                use core::arch::x86_64::{
                    _CMP_GT_OQ, _mm256_and_si256, _mm256_blendv_ps, _mm256_cmp_ps,
                    _mm256_cvtepi32_ps, _mm256_cvttps_epi32, _mm256_fmadd_ps, _mm256_loadu_ps,
                    _mm256_mul_ps, _mm256_permutevar8x32_ps, _mm256_set1_ps, _mm256_storeu_ps,
                    _mm256_sub_ps,
                };

                let phase: [f32; 8] = phase.into();
                let phase = _mm256_loadu_ps(phase.as_ptr());
                let position = _mm256_mul_ps(phase, _mm256_set1_ps(PROBE_SEGMENTS as f32));
                let segments = _mm256_cvttps_epi32(position);
                let segment_f = _mm256_cvtepi32_ps(segments);
                let t = _mm256_sub_ps(position, segment_f);
                let upper = _mm256_cmp_ps(segment_f, _mm256_set1_ps(7.0), _CMP_GT_OQ);
                let indexes =
                    _mm256_and_si256(segments, core::mem::transmute::<[i32; 8], _>([7; 8]));
                let gather = |plane: usize| {
                    let base = self.0.as_ptr().add(plane * PROBE_SEGMENTS);
                    let low = _mm256_permutevar8x32_ps(_mm256_loadu_ps(base), indexes);
                    let high = _mm256_permutevar8x32_ps(_mm256_loadu_ps(base.add(8)), indexes);
                    _mm256_blendv_ps(low, high, upper)
                };
                let value = _mm256_fmadd_ps(
                    _mm256_fmadd_ps(_mm256_fmadd_ps(gather(0), t, gather(1)), t, gather(2)),
                    t,
                    gather(3),
                );
                let mut output = [0.0; 8];
                _mm256_storeu_ps(output.as_mut_ptr(), value);
                return f32x8::from(output);
            }

            #[cfg(not(all(
                target_arch = "x86_64",
                target_feature = "avx2",
                target_feature = "fma"
            )))]
            {
                let phases: [f32; 8] = phase.into();
                f32x8::from(phases.map(|phase| {
                    let position = phase * PROBE_SEGMENTS as f32;
                    let segment = (position as usize).min(PROBE_SEGMENTS - 1);
                    let t = position - segment as f32;
                    self.0[probe_index(segment, 0)]
                        .mul_add(t, self.0[probe_index(segment, 1)])
                        .mul_add(t, self.0[probe_index(segment, 2)])
                        .mul_add(t, self.0[probe_index(segment, 3)])
                }))
            }
        }
    }

    #[derive(Clone, Copy)]
    enum ProbeShape {
        Saw,
        Square,
    }

    fn probe_index(segment: usize, plane: usize) -> usize {
        if cfg!(all(
            target_arch = "x86_64",
            target_feature = "avx2",
            target_feature = "fma"
        )) {
            plane * PROBE_SEGMENTS + segment
        } else {
            segment * 4 + plane
        }
    }

    fn projected(shape: ProbeShape, cap: usize, phase: f32) -> f32 {
        let angle = std::f32::consts::TAU * phase;
        match shape {
            ProbeShape::Saw => (1..=cap).fold(0.0, |sum, harmonic| {
                sum - 2.0 * (angle * harmonic as f32).sin()
                    / (std::f32::consts::PI * harmonic as f32)
            }),
            ProbeShape::Square => (1..=cap).step_by(2).fold(0.0, |sum, harmonic| {
                sum + 4.0 * (angle * harmonic as f32).sin()
                    / (std::f32::consts::PI * harmonic as f32)
            }),
        }
    }

    fn compile_probe(shape: ProbeShape, cap: usize) -> ProbeCurve {
        let mut coefficients = [0.0; 64];
        for segment in 0..PROBE_SEGMENTS {
            let phase = segment as f32 / PROBE_SEGMENTS as f32;
            let step = 1.0 / PROBE_SEGMENTS as f32;
            let y0 = projected(shape, cap, phase);
            let y1 = projected(shape, cap, phase + step / 3.0);
            let y2 = projected(shape, cap, phase + 2.0 * step / 3.0);
            let y3 = projected(shape, cap, phase + step);
            let p = y1 - y0;
            let q = y2 - y0;
            let r = y3 - y0;
            for (plane, value) in [
                4.5_f32.mul_add(r, 13.5 * (p - q)),
                (-4.5_f32).mul_add(r, (-22.5_f32).mul_add(p, 18.0 * q)),
                9.0_f32.mul_add(p, (-4.5_f32).mul_add(q, r)),
                y0,
            ]
            .into_iter()
            .enumerate()
            {
                coefficients[probe_index(segment, plane)] = value;
            }
        }
        ProbeCurve(coefficients)
    }

    fn current8(shape: ProbeShape, phase: f32x8, step: f32x8) -> f32x8 {
        match shape {
            ProbeShape::Saw => bandlimited_saw8(phase, step, Antialiasing::SplineOptimized),
            ProbeShape::Square => {
                bandlimited_pulse8(phase, step, 0.5, Antialiasing::SplineOptimized)
            }
        }
    }

    fn advance_selector(state: &mut [u16; 8], eligible: bool) -> f32x8 {
        f32x8::from(std::array::from_fn(|lane| {
            state[lane] = if eligible {
                state[lane].saturating_add(1).min(PROBE_SLEW)
            } else {
                state[lane].saturating_sub(1)
            };
            f32::from(state[lane]) / f32::from(PROBE_SLEW)
        }))
    }

    fn render_probe_block(
        shape: ProbeShape,
        curve: ProbeCurve,
        phases: &mut f32x8,
        steps: f32x8,
        selector: &mut [u16; 8],
        eligible: bool,
        output: &mut [f32x8; 64],
    ) {
        for sample in output {
            let phase = *phases;
            let mix = advance_selector(selector, eligible);
            let projected = curve.eval8(phase);
            *sample = if selector.iter().all(|value| *value == PROBE_SLEW) {
                projected
            } else if selector.iter().all(|value| *value == 0) {
                current8(shape, phase, steps)
            } else {
                let current = current8(shape, phase, steps);
                (projected - current).mul_add(mix, current)
            };
            let next = *phases + steps;
            *phases = next.cmp_lt(f32x8::ONE).blend(next, next - f32x8::ONE);
        }
    }

    fn probe_sample_kernel<const SAMPLES: usize>(
        oscillators: &mut [super::VaOscillator],
        phase_step: f32x8,
        left_gain: f32x8,
        right_gain: f32x8,
        left: &mut [f32x8; SAMPLES],
        right: &mut [f32x8; SAMPLES],
        mut generate: impl FnMut(f32x8, f32x8) -> f32x8,
    ) {
        let mut phase = f32x8::from(std::array::from_fn(|lane| oscillators[lane].phase));
        for frame in 0..SAMPLES {
            let current = phase;
            let next = phase + phase_step;
            phase = next.cmp_lt(f32x8::ONE).blend(next, next - f32x8::ONE);
            let sample = generate(current, phase_step);
            left[frame] = sample.mul_add(left_gain, left[frame]);
            right[frame] = sample.mul_add(right_gain, right[frame]);
        }
        let phases: [f32; 8] = phase.into();
        for (oscillator, phase) in oscillators.iter_mut().zip(phases) {
            oscillator.phase = phase;
        }
    }

    fn probe_scheduled_saw8<const SAMPLES: usize>(
        oscillators: &mut [super::VaOscillator],
        phase_step: f32x8,
        left_gain: f32x8,
        right_gain: f32x8,
        left: &mut [f32x8; SAMPLES],
        right: &mut [f32x8; SAMPLES],
    ) -> usize {
        let one = f32x8::ONE;
        let active = phase_step.cmp_gt(f32x8::splat(f32::EPSILON));
        let support = phase_step * f32x8::splat(2.0);
        let inverse_step = one / active.blend(phase_step, one);
        let mut scheduled_phase = f32x8::from(std::array::from_fn(|lane| oscillators[lane].phase));
        let schedule: [bool; SAMPLES] = std::array::from_fn(|_| {
            let event =
                active & (scheduled_phase.cmp_lt(support) | scheduled_phase.cmp_gt(one - support));
            let next = scheduled_phase + phase_step;
            scheduled_phase = next.cmp_lt(one).blend(next, next - one);
            event.any()
        });
        let event_frames = schedule.into_iter().filter(|event| *event).count();
        let mut phase = f32x8::from(std::array::from_fn(|lane| oscillators[lane].phase));
        for frame in 0..SAMPLES {
            let current = phase;
            let next = phase + phase_step;
            phase = next.cmp_lt(one).blend(next, next - one);
            let correction = if schedule[frame] {
                super::spline_blep8_precomputed(current, active, support, inverse_step, true)
            } else {
                f32x8::ZERO
            };
            let sample = current * f32x8::splat(2.0) - one - correction;
            left[frame] = sample.mul_add(left_gain, left[frame]);
            right[frame] = sample.mul_add(right_gain, right[frame]);
        }
        let phases: [f32; 8] = phase.into();
        for (oscillator, phase) in oscillators.iter_mut().zip(phases) {
            oscillator.phase = phase;
        }
        event_frames
    }

    fn probe_analytic_saw8(
        oscillators: &mut [super::VaOscillator],
        phase_step: f32x8,
        left_gain: f32x8,
        right_gain: f32x8,
        left: &mut [f32x8; 64],
        right: &mut [f32x8; 64],
    ) -> u64 {
        let phases = std::array::from_fn(|lane| oscillators[lane].phase);
        let steps: [f32; 8] = phase_step.into();
        let mut events = 0_u64;
        for lane in 0..8 {
            if steps[lane] <= f32::EPSILON {
                continue;
            }
            let final_cycle = (phases[lane] + steps[lane] * 64.0).ceil() as i32;
            for cycle in 0..=final_cycle + 1 {
                let crossing = ((cycle as f32 - phases[lane]) / steps[lane]).ceil() as i32;
                for frame in crossing - 3..=crossing + 2 {
                    if (0..64).contains(&frame) {
                        events |= 1_u64 << frame;
                    }
                }
            }
        }
        let one = f32x8::ONE;
        let active = phase_step.cmp_gt(f32x8::splat(f32::EPSILON));
        let support = phase_step * f32x8::splat(2.0);
        let inverse_step = one / active.blend(phase_step, one);
        let mut phase = f32x8::from(phases);
        for frame in 0..64 {
            let current = phase;
            let next = phase + phase_step;
            phase = next.cmp_lt(one).blend(next, next - one);
            let correction = if events & (1_u64 << frame) != 0 {
                super::spline_blep8_precomputed(current, active, support, inverse_step, true)
            } else {
                f32x8::ZERO
            };
            let sample = current * f32x8::splat(2.0) - one - correction;
            left[frame] = sample.mul_add(left_gain, left[frame]);
            right[frame] = sample.mul_add(right_gain, right[frame]);
        }
        let phases: [f32; 8] = phase.into();
        for (oscillator, phase) in oscillators.iter_mut().zip(phases) {
            oscillator.phase = phase;
        }
        events
    }

    #[inline]
    fn probe_scalar_blep(position: f32) -> f32 {
        let distance = position.abs();
        let residual = if distance < 1.0 {
            0.094_483_666_f32
                .mul_add(distance, -0.273_396_5)
                .mul_add(distance, -0.000_368_514_85)
                .mul_add(distance, 0.626_745_1)
                .mul_add(distance, -0.5)
        } else if distance < 2.0 {
            let tail = 2.0 - distance;
            -0.029_106_615_f32
                .mul_add(tail, -0.026_743_31)
                .mul_add(tail, 0.005_957_221_6)
                .mul_add(tail, -0.002_643_542_6)
                * tail
        } else {
            0.0
        };
        if position < 0.0 { -residual } else { residual }
    }

    fn probe_adaptive_blep8(phase: f32x8, step: f32x8, threshold: usize) -> f32x8 {
        let phases: [f32; 8] = phase.into();
        let steps: [f32; 8] = step.into();
        let active: [bool; 8] = std::array::from_fn(|lane| {
            steps[lane] > f32::EPSILON
                && (phases[lane] < steps[lane] * 2.0 || phases[lane] > 1.0 - steps[lane] * 2.0)
        });
        if active.into_iter().filter(|active| *active).count() <= threshold {
            return f32x8::from(std::array::from_fn(|lane| {
                if active[lane] {
                    let nearest = if phases[lane] < 0.5 {
                        phases[lane]
                    } else {
                        phases[lane] - 1.0
                    };
                    probe_scalar_blep(nearest * (1.0 / steps[lane])) * 2.0
                } else {
                    0.0
                }
            }));
        }
        let active = step.cmp_gt(f32x8::splat(f32::EPSILON));
        let support = step * f32x8::splat(2.0);
        super::spline_blep8_precomputed(
            phase,
            active,
            support,
            f32x8::ONE / active.blend(step, f32x8::ONE),
            true,
        )
    }

    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "avx512f,avx512vl,avx2,fma")]
    #[allow(
        unsafe_op_in_unsafe_fn,
        reason = "runtime-guarded AVX-512 experiment kernel"
    )]
    unsafe fn probe_avx512_residual(position: f32x8, event: u8, blamp: bool) -> f32x8 {
        use core::arch::x86_64::*;
        let values: [f32; 8] = position.into();
        let position = _mm256_loadu_ps(values.as_ptr());
        let sign = _mm256_and_ps(position, _mm256_set1_ps(-0.0));
        let distance = _mm256_andnot_ps(_mm256_set1_ps(-0.0), position);
        let inside = event & _mm256_cmp_ps_mask::<_CMP_LT_OQ>(distance, _mm256_set1_ps(2.0));
        let inner_mask = inside & _mm256_cmp_ps_mask::<_CMP_LT_OQ>(distance, _mm256_set1_ps(1.0));
        let outer_mask = inside & !inner_mask;
        let residual = if blamp {
            let mut inner = _mm256_maskz_fmadd_ps(
                inner_mask,
                _mm256_set1_ps(0.018_896_732),
                distance,
                _mm256_set1_ps(-0.068_349_12),
            );
            inner = _mm256_maskz_fmadd_ps(
                inner_mask,
                inner,
                distance,
                _mm256_set1_ps(-0.000_122_838_29),
            );
            inner =
                _mm256_maskz_fmadd_ps(inner_mask, inner, distance, _mm256_set1_ps(0.313_372_55));
            inner = _mm256_maskz_fmadd_ps(inner_mask, inner, distance, _mm256_set1_ps(-0.5));
            inner =
                _mm256_maskz_fmadd_ps(inner_mask, inner, distance, _mm256_set1_ps(0.248_045_86));
            let tail = _mm256_sub_ps(_mm256_set1_ps(2.0), distance);
            let mut outer = _mm256_maskz_fmadd_ps(
                outer_mask,
                _mm256_set1_ps(0.005_821_323),
                tail,
                _mm256_set1_ps(0.006_685_827_4),
            );
            outer =
                _mm256_maskz_fmadd_ps(outer_mask, outer, tail, _mm256_set1_ps(-0.001_985_740_6));
            outer = _mm256_maskz_fmadd_ps(outer_mask, outer, tail, _mm256_set1_ps(0.001_321_771_3));
            outer = _mm256_maskz_mul_ps(outer_mask, outer, tail);
            outer = _mm256_maskz_mul_ps(outer_mask, outer, tail);
            _mm256_add_ps(inner, outer)
        } else {
            let mut inner = _mm256_maskz_fmadd_ps(
                inner_mask,
                _mm256_set1_ps(0.094_483_666),
                distance,
                _mm256_set1_ps(-0.273_396_5),
            );
            inner = _mm256_maskz_fmadd_ps(
                inner_mask,
                inner,
                distance,
                _mm256_set1_ps(-0.000_368_514_85),
            );
            inner = _mm256_maskz_fmadd_ps(inner_mask, inner, distance, _mm256_set1_ps(0.626_745_1));
            inner = _mm256_maskz_fmadd_ps(inner_mask, inner, distance, _mm256_set1_ps(-0.5));
            let tail = _mm256_sub_ps(_mm256_set1_ps(2.0), distance);
            let mut outer = _mm256_maskz_fmadd_ps(
                outer_mask,
                _mm256_set1_ps(-0.029_106_615),
                tail,
                _mm256_set1_ps(-0.026_743_31),
            );
            outer = _mm256_maskz_fmadd_ps(outer_mask, outer, tail, _mm256_set1_ps(0.005_957_221_6));
            outer =
                _mm256_maskz_fmadd_ps(outer_mask, outer, tail, _mm256_set1_ps(-0.002_643_542_6));
            outer = _mm256_maskz_mul_ps(outer_mask, outer, tail);
            _mm256_xor_ps(_mm256_add_ps(inner, outer), sign)
        };
        let mut output = [0.0; 8];
        _mm256_storeu_ps(output.as_mut_ptr(), residual);
        f32x8::from(output)
    }

    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "avx512f,avx512vl,avx2,fma")]
    #[allow(
        unsafe_op_in_unsafe_fn,
        reason = "runtime-guarded AVX-512 experiment wrapper"
    )]
    unsafe fn probe_avx512_blep(phase: f32x8, step: f32x8, inverse: f32x8) -> f32x8 {
        use core::arch::x86_64::*;
        let phases: [f32; 8] = phase.into();
        let steps: [f32; 8] = step.into();
        let inverses: [f32; 8] = inverse.into();
        let phase = _mm256_loadu_ps(phases.as_ptr());
        let step = _mm256_loadu_ps(steps.as_ptr());
        let inverse = _mm256_loadu_ps(inverses.as_ptr());
        let one = _mm256_set1_ps(1.0);
        let support = _mm256_add_ps(step, step);
        let before = _mm256_cmp_ps_mask::<_CMP_LT_OQ>(phase, support);
        let after = _mm256_cmp_ps_mask::<_CMP_GT_OQ>(phase, _mm256_sub_ps(one, support));
        let active = _mm256_cmp_ps_mask::<_CMP_GT_OQ>(step, _mm256_set1_ps(f32::EPSILON));
        let mask = active & (before | after);
        let nearest = _mm256_mask_sub_ps(phase, !before, phase, one);
        let position = _mm256_mul_ps(nearest, inverse);
        let mut positions = [0.0; 8];
        _mm256_storeu_ps(positions.as_mut_ptr(), position);
        (unsafe { probe_avx512_residual(f32x8::from(positions), mask, false) }) * f32x8::splat(2.0)
    }

    #[cfg(target_arch = "x86_64")]
    unsafe fn probe_avx512_triangle(phase: f32x8, step: f32x8) -> f32x8 {
        let phases: [f32; 8] = phase.into();
        let steps: [f32; 8] = step.into();
        let mut mask = 0_u8;
        let mut peak = [false; 8];
        let positions = std::array::from_fn(|lane| {
            let peak_distance = (phases[lane] - 0.5).abs();
            peak[lane] = peak_distance < 0.25;
            let corner_distance = if peak[lane] {
                peak_distance
            } else {
                0.5 - peak_distance
            };
            if steps[lane] > f32::EPSILON && corner_distance < steps[lane] * 2.0 {
                mask |= 1 << lane;
            }
            corner_distance * (1.0 / steps[lane])
        });
        let residual: [f32; 8] =
            unsafe { probe_avx512_residual(f32x8::from(positions), mask, true) }.into();
        f32x8::from(std::array::from_fn(|lane| {
            let raw = 1.0 - 4.0 * (phases[lane] - 0.5).abs();
            let correction = if peak[lane] {
                -residual[lane]
            } else {
                residual[lane]
            };
            (steps[lane] * 8.0).mul_add(correction, raw)
        }))
    }

    #[cfg(target_arch = "x86_64")]
    #[inline(never)]
    #[target_feature(enable = "avx512f,avx512vl")]
    unsafe fn probe_avx512_call_lower_bound(value: f32x8) -> f32x8 {
        std::hint::black_box(value)
    }

    #[cfg(target_arch = "x86_64")]
    #[inline(always)]
    unsafe fn probe_avx512_blep_register(
        position: core::arch::x86_64::__m256,
        event: u8,
    ) -> core::arch::x86_64::__m256 {
        use core::arch::x86_64::*;
        let sign = unsafe { _mm256_and_ps(position, _mm256_set1_ps(-0.0)) };
        let distance = unsafe { _mm256_andnot_ps(_mm256_set1_ps(-0.0), position) };
        let inside =
            event & unsafe { _mm256_cmp_ps_mask::<_CMP_LT_OQ>(distance, _mm256_set1_ps(2.0)) };
        let inner_mask =
            inside & unsafe { _mm256_cmp_ps_mask::<_CMP_LT_OQ>(distance, _mm256_set1_ps(1.0)) };
        let outer_mask = inside & !inner_mask;
        let mut inner = unsafe {
            _mm256_maskz_fmadd_ps(
                inner_mask,
                _mm256_set1_ps(0.094_483_666),
                distance,
                _mm256_set1_ps(-0.273_396_5),
            )
        };
        inner = unsafe {
            _mm256_maskz_fmadd_ps(
                inner_mask,
                inner,
                distance,
                _mm256_set1_ps(-0.000_368_514_85),
            )
        };
        inner = unsafe {
            _mm256_maskz_fmadd_ps(inner_mask, inner, distance, _mm256_set1_ps(0.626_745_1))
        };
        inner = unsafe { _mm256_maskz_fmadd_ps(inner_mask, inner, distance, _mm256_set1_ps(-0.5)) };
        let tail = unsafe { _mm256_sub_ps(_mm256_set1_ps(2.0), distance) };
        let mut outer = unsafe {
            _mm256_maskz_fmadd_ps(
                outer_mask,
                _mm256_set1_ps(-0.029_106_615),
                tail,
                _mm256_set1_ps(-0.026_743_31),
            )
        };
        outer = unsafe {
            _mm256_maskz_fmadd_ps(outer_mask, outer, tail, _mm256_set1_ps(0.005_957_221_6))
        };
        outer = unsafe {
            _mm256_maskz_fmadd_ps(outer_mask, outer, tail, _mm256_set1_ps(-0.002_643_542_6))
        };
        outer = unsafe { _mm256_maskz_mul_ps(outer_mask, outer, tail) };
        unsafe { _mm256_xor_ps(_mm256_add_ps(inner, outer), sign) }
    }

    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "avx512f,avx512vl,avx2,fma")]
    #[allow(
        unsafe_op_in_unsafe_fn,
        reason = "runtime-guarded whole-block AVX-512 experiment"
    )]
    unsafe fn probe_avx512_pulse_block<const SAMPLES: usize>(
        oscillators: &mut [super::VaOscillator],
        step: f32x8,
        left_gain: f32x8,
        right_gain: f32x8,
        left: &mut [f32x8; SAMPLES],
        right: &mut [f32x8; SAMPLES],
        width: f32,
    ) {
        use core::arch::x86_64::*;
        let phases: [f32; 8] = std::array::from_fn(|lane| oscillators[lane].phase);
        let steps: [f32; 8] = step.into();
        let left_gains: [f32; 8] = left_gain.into();
        let right_gains: [f32; 8] = right_gain.into();
        let mut phase = _mm256_loadu_ps(phases.as_ptr());
        let step = _mm256_loadu_ps(steps.as_ptr());
        let left_gain = _mm256_loadu_ps(left_gains.as_ptr());
        let right_gain = _mm256_loadu_ps(right_gains.as_ptr());
        let one = _mm256_set1_ps(1.0);
        let support = _mm256_add_ps(step, step);
        let inverse = _mm256_div_ps(one, step);
        let width = _mm256_min_ps(
            _mm256_max_ps(step, _mm256_set1_ps(width.clamp(0.03, 0.97))),
            _mm256_sub_ps(one, step),
        );
        let active = _mm256_cmp_ps_mask::<_CMP_GT_OQ>(step, _mm256_set1_ps(f32::EPSILON));
        let correction = |edge_phase: __m256| {
            let before = _mm256_cmp_ps_mask::<_CMP_LT_OQ>(edge_phase, support);
            let after = _mm256_cmp_ps_mask::<_CMP_GT_OQ>(edge_phase, _mm256_sub_ps(one, support));
            let nearest = _mm256_mask_sub_ps(edge_phase, !before, edge_phase, one);
            let position = _mm256_mul_ps(nearest, inverse);
            let residual = probe_avx512_blep_register(position, active & (before | after));
            _mm256_add_ps(residual, residual)
        };
        for frame in 0..SAMPLES {
            let current = phase;
            let next = _mm256_add_ps(phase, step);
            phase = _mm256_mask_blend_ps(
                _mm256_cmp_ps_mask::<_CMP_LT_OQ>(next, one),
                _mm256_sub_ps(next, one),
                next,
            );
            let shifted_unwrapped = _mm256_add_ps(_mm256_sub_ps(current, width), one);
            let shifted = _mm256_mask_blend_ps(
                _mm256_cmp_ps_mask::<_CMP_LT_OQ>(shifted_unwrapped, one),
                _mm256_sub_ps(shifted_unwrapped, one),
                shifted_unwrapped,
            );
            let raw = _mm256_mask_blend_ps(
                _mm256_cmp_ps_mask::<_CMP_LT_OQ>(current, width),
                _mm256_set1_ps(-1.0),
                one,
            );
            let sample =
                _mm256_sub_ps(_mm256_add_ps(raw, correction(current)), correction(shifted));
            let left_values: [f32; 8] = left[frame].into();
            let right_values: [f32; 8] = right[frame].into();
            let left_sample =
                _mm256_fmadd_ps(sample, left_gain, _mm256_loadu_ps(left_values.as_ptr()));
            let right_sample =
                _mm256_fmadd_ps(sample, right_gain, _mm256_loadu_ps(right_values.as_ptr()));
            let mut values = [0.0; 8];
            _mm256_storeu_ps(values.as_mut_ptr(), left_sample);
            left[frame] = f32x8::from(values);
            _mm256_storeu_ps(values.as_mut_ptr(), right_sample);
            right[frame] = f32x8::from(values);
        }
        let mut values = [0.0; 8];
        _mm256_storeu_ps(values.as_mut_ptr(), phase);
        for (oscillator, phase) in oscillators.iter_mut().zip(values) {
            oscillator.phase = phase;
        }
    }

    #[cfg(target_arch = "x86_64")]
    #[inline]
    fn probe_avx512_pulse_selector<const SAMPLES: usize>(
        oscillators: &mut [super::VaOscillator],
        step: f32x8,
        left_gain: f32x8,
        right_gain: f32x8,
        left: &mut [f32x8; SAMPLES],
        right: &mut [f32x8; SAMPLES],
        width: f32,
    ) -> bool {
        if step.cmp_gt(f32x8::splat(0.04)).all()
            && std::arch::is_x86_feature_detected!("avx512f")
            && std::arch::is_x86_feature_detected!("avx512vl")
            && std::arch::is_x86_feature_detected!("avx2")
            && std::arch::is_x86_feature_detected!("fma")
        {
            // SAFETY: every target feature was detected immediately above.
            unsafe {
                probe_avx512_pulse_block(
                    oscillators,
                    step,
                    left_gain,
                    right_gain,
                    left,
                    right,
                    width,
                );
            }
            true
        } else {
            super::accumulate_shape8_block_constant(
                oscillators,
                step,
                left_gain,
                right_gain,
                left,
                right,
                3.0,
                width,
                super::Antialiasing::SplineOptimized,
            );
            false
        }
    }

    #[cfg(target_arch = "x86_64")]
    fn report_avx512_pulse_chunk<const SAMPLES: usize>() {
        use std::hint::black_box;
        use std::time::Instant;

        let gain_l = f32x8::from([0.117, 0.121, 0.125, 0.129, 0.133, 0.137, 0.141, 0.145]);
        let gain_r = f32x8::from([0.149, 0.145, 0.141, 0.137, 0.133, 0.129, 0.125, 0.121]);
        for (range, base_step) in [
            ("low", 0.0046_f32),
            ("threshold", 0.040_1),
            ("mid", 0.041),
            ("high", 0.083),
        ] {
            let step = f32x8::from(std::array::from_fn(|lane| {
                base_step * (1.0 + lane as f32 * 0.000_13)
            }));
            for coherent in [false, true] {
                let phases = if coherent {
                    [0.997; 8]
                } else {
                    [0.997, 0.121, 0.247, 0.373, 0.499, 0.625, 0.751, 0.877]
                };
                for width in [0.03, 0.37, 0.97] {
                    let run = |mode: u8, blocks: usize| {
                        let mut oscillators = [super::VaOscillator::default(); 8];
                        for (oscillator, phase) in oscillators.iter_mut().zip(phases) {
                            oscillator.phase = phase;
                        }
                        let mut left = [f32x8::ZERO; SAMPLES];
                        let mut right = [f32x8::ZERO; SAMPLES];
                        let started = Instant::now();
                        for _ in 0..blocks {
                            left.fill(f32x8::ZERO);
                            right.fill(f32x8::ZERO);
                            match mode {
                                1 => {
                                    unsafe {
                                        probe_avx512_pulse_block(
                                            &mut oscillators,
                                            step,
                                            gain_l,
                                            gain_r,
                                            &mut left,
                                            &mut right,
                                            width,
                                        )
                                    };
                                }
                                2 => {
                                    probe_avx512_pulse_selector(
                                        &mut oscillators,
                                        step,
                                        gain_l,
                                        gain_r,
                                        &mut left,
                                        &mut right,
                                        width,
                                    );
                                }
                                _ => super::accumulate_shape8_block_constant(
                                    &mut oscillators,
                                    step,
                                    gain_l,
                                    gain_r,
                                    &mut left,
                                    &mut right,
                                    3.0,
                                    width,
                                    super::Antialiasing::SplineOptimized,
                                ),
                            }
                            black_box((&left, &right, &oscillators));
                        }
                        (
                            started.elapsed().as_nanos() as f64 / (blocks * SAMPLES) as f64,
                            left,
                            right,
                            oscillators,
                        )
                    };
                    let (_, baseline_l, baseline_r, baseline_osc) = run(0, 1);
                    let (_, candidate_l, candidate_r, candidate_osc) = run(1, 1);
                    let mut square = 0.0_f64;
                    let mut peak = 0.0_f32;
                    for (a, b) in baseline_l
                        .into_iter()
                        .chain(baseline_r)
                        .zip(candidate_l.into_iter().chain(candidate_r))
                    {
                        for error in <[f32; 8]>::from(a - b) {
                            square += f64::from(error) * f64::from(error);
                            peak = peak.max(error.abs());
                        }
                    }
                    let phase_peak = baseline_osc
                        .iter()
                        .zip(candidate_osc.iter())
                        .map(|(a, b)| (a.phase - b.phase).abs())
                        .fold(0.0_f32, f32::max);
                    let mut timings = [[0.0; 9]; 3];
                    for repeat in 0..9 {
                        let order = match repeat % 3 {
                            0 => [0, 1, 2],
                            1 => [2, 0, 1],
                            _ => [1, 2, 0],
                        };
                        for mode in order {
                            timings[mode as usize][repeat] = run(mode, 50_000).0;
                        }
                    }
                    for timing in &mut timings {
                        timing.sort_by(f64::total_cmp);
                    }
                    let baseline = timings[0][4];
                    let candidate = timings[1][4];
                    let selected = timings[2][4];
                    println!(
                        "avx512_whole_pulse,frames={SAMPLES},range={range},width={width:.2},coherent={coherent},baseline_ns={baseline:.3},candidate_ns={candidate:.3},candidate_delta_pct={:.2},selected_ns={selected:.3},selected_delta_pct={:.2},difference_rms={:.9},difference_peak={peak:.9},phase_peak={phase_peak:.9}",
                        (candidate / baseline - 1.0) * 100.0,
                        (selected / baseline - 1.0) * 100.0,
                        (square / (SAMPLES * 16) as f64).sqrt(),
                    );
                }
            }
        }
    }

    #[cfg(target_arch = "x86_64")]
    fn report_avx512_pulse_transitions<const SAMPLES: usize>() {
        let gain_l = f32x8::from([0.117, 0.121, 0.125, 0.129, 0.133, 0.137, 0.141, 0.145]);
        let gain_r = f32x8::from([0.149, 0.145, 0.141, 0.137, 0.133, 0.129, 0.125, 0.121]);
        let coherent = [0.997; 8];
        let decorrelated = [0.997, 0.121, 0.247, 0.373, 0.499, 0.625, 0.751, 0.877];
        let reset_edges = [
            0.0, 0.000_001, 0.999_999, 0.369_999, 0.37, 0.370_001, 0.5, 0.75,
        ];
        let stereo = |left: f32x8, right: f32x8| {
            let mut samples = [0.0; 16];
            samples[..8].copy_from_slice(&<[f32; 8]>::from(left));
            samples[8..].copy_from_slice(&<[f32; 8]>::from(right));
            samples
        };

        for (lane_mode, initial) in [("coherent", coherent), ("decorrelated", decorrelated)] {
            for (transition, stages) in [
                (
                    "pitch",
                    [
                        (0.0046, 0.37, None),
                        (0.041, 0.37, None),
                        (0.083, 0.37, None),
                        (0.0046, 0.37, None),
                    ],
                ),
                (
                    "width",
                    [
                        (0.083, 0.37, None),
                        (0.083, 0.03, None),
                        (0.083, 0.97, None),
                        (0.083, 0.37, None),
                    ],
                ),
                (
                    "reset",
                    [
                        (0.083, 0.37, None),
                        (
                            0.083,
                            0.37,
                            Some(if lane_mode == "coherent" {
                                [0.0; 8]
                            } else {
                                reset_edges
                            }),
                        ),
                        (0.083, 0.37, None),
                        (0.083, 0.37, None),
                    ],
                ),
            ] {
                let mut baseline = initial.map(|phase| super::VaOscillator {
                    phase,
                    ..super::VaOscillator::default()
                });
                let mut selected = baseline;
                let mut previous_baseline: Option<[f32; 16]> = None;
                let mut previous_selected: Option<[f32; 16]> = None;
                let mut difference_square = 0.0_f64;
                let mut signal_square = 0.0_f64;
                let mut difference_peak = 0.0_f32;
                let mut boundary_regression = 0.0_f32;
                let mut bit_mismatches = 0_u32;
                let mut selected_blocks = 0_u32;

                for (base_step, width, reset) in stages {
                    if let Some(phases) = reset {
                        for ((baseline, selected), phase) in
                            baseline.iter_mut().zip(selected.iter_mut()).zip(phases)
                        {
                            baseline.phase = phase;
                            selected.phase = phase;
                        }
                    }
                    let step = f32x8::from(std::array::from_fn(|lane| {
                        base_step * (1.0 + lane as f32 * 0.000_13)
                    }));
                    let mut baseline_l = [f32x8::ZERO; SAMPLES];
                    let mut baseline_r = [f32x8::ZERO; SAMPLES];
                    let mut selected_l = [f32x8::ZERO; SAMPLES];
                    let mut selected_r = [f32x8::ZERO; SAMPLES];
                    super::accumulate_shape8_block_constant(
                        &mut baseline,
                        step,
                        gain_l,
                        gain_r,
                        &mut baseline_l,
                        &mut baseline_r,
                        3.0,
                        width,
                        super::Antialiasing::SplineOptimized,
                    );
                    selected_blocks += u32::from(probe_avx512_pulse_selector(
                        &mut selected,
                        step,
                        gain_l,
                        gain_r,
                        &mut selected_l,
                        &mut selected_r,
                        width,
                    ));
                    for (reference, candidate) in baseline_l
                        .iter()
                        .chain(baseline_r.iter())
                        .zip(selected_l.iter().chain(selected_r.iter()))
                    {
                        for (reference, candidate) in <[f32; 8]>::from(*reference)
                            .into_iter()
                            .zip(<[f32; 8]>::from(*candidate))
                        {
                            let difference = candidate - reference;
                            difference_square += f64::from(difference) * f64::from(difference);
                            signal_square += f64::from(reference) * f64::from(reference);
                            difference_peak = difference_peak.max(difference.abs());
                            bit_mismatches += u32::from(candidate.to_bits() != reference.to_bits());
                        }
                    }
                    if let (Some(previous_baseline), Some(previous_selected)) =
                        (previous_baseline, previous_selected)
                    {
                        let baseline_first = stereo(baseline_l[0], baseline_r[0]);
                        let selected_first = stereo(selected_l[0], selected_r[0]);
                        for (
                            (baseline_before, selected_before),
                            (baseline_after, selected_after),
                        ) in previous_baseline
                            .into_iter()
                            .zip(previous_selected)
                            .zip(baseline_first.into_iter().zip(selected_first))
                        {
                            boundary_regression = boundary_regression.max(
                                (selected_after - selected_before).abs()
                                    - (baseline_after - baseline_before).abs(),
                            );
                        }
                    }
                    previous_baseline =
                        Some(stereo(baseline_l[SAMPLES - 1], baseline_r[SAMPLES - 1]));
                    previous_selected =
                        Some(stereo(selected_l[SAMPLES - 1], selected_r[SAMPLES - 1]));
                }

                let phase_peak = baseline
                    .iter()
                    .zip(selected.iter())
                    .map(|(baseline, selected)| (baseline.phase - selected.phase).abs())
                    .fold(0.0_f32, f32::max);
                let delta_db = if difference_square > 0.0 {
                    10.0 * (difference_square / signal_square).log10()
                } else {
                    f64::NEG_INFINITY
                };
                println!(
                    "avx512_pulse_transition,frames={SAMPLES},lanes={lane_mode},transition={transition},selected_blocks={selected_blocks}/4,bit_mismatches={bit_mismatches},difference_db={delta_db:.2},difference_peak={difference_peak:.9},boundary_regression={boundary_regression:.9},phase_peak={phase_peak:.9}"
                );
                assert_eq!(phase_peak, 0.0);
                assert!(difference_peak <= 4.0e-6);
                assert!(boundary_regression <= 4.0e-6);
            }
        }
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    #[ignore = "whole-block AVX-512 pulse experiment"]
    fn avx512_whole_pulse_block_report() {
        if !(std::arch::is_x86_feature_detected!("avx512f")
            && std::arch::is_x86_feature_detected!("avx512vl")
            && std::arch::is_x86_feature_detected!("avx2")
            && std::arch::is_x86_feature_detected!("fma"))
        {
            println!("avx512_whole_pulse,skipped=unsupported");
            return;
        }
        report_avx512_pulse_chunk::<24>();
        report_avx512_pulse_chunk::<32>();
        report_avx512_pulse_transitions::<24>();
        report_avx512_pulse_transitions::<32>();
    }

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

    #[test]
    #[ignore = "dedicated canonical coefficient backend experiment"]
    fn canonical_coefficient_x8_transition_report() {
        use std::hint::black_box;
        use std::time::Instant;

        let lane_steps = |midi: f32| {
            f32x8::from(std::array::from_fn(|lane| {
                let frequency = 440.0 * 2.0_f32.powf((midi - 69.0) / 12.0);
                frequency * (1.0 + (lane as f32 - 3.5) * 0.000_1) / 48_000.0
            }))
        };
        for shape in [ProbeShape::Saw, ProbeShape::Square] {
            let name = match shape {
                ProbeShape::Saw => "saw",
                ProbeShape::Square => "square",
            };
            for (midi, cap) in [(105.0, 6), (117.0, 3), (123.0, 2)] {
                let curve = compile_probe(shape, cap);
                let mut square = 0.0_f64;
                let mut peak = 0.0_f32;
                let mut range = 0.0_f32;
                for index in 0..65_536 {
                    let phase = index as f32 / 65_536.0;
                    let sample: [f32; 8] = curve.eval8(f32x8::splat(phase)).into();
                    let error = sample[0] - projected(shape, cap, phase);
                    square += f64::from(error) * f64::from(error);
                    peak = peak.max(error.abs());
                    range = range.max(sample[0].abs());
                }
                println!(
                    "canonical_coeff_quality,shape={name},midi={midi:.0},cap={cap},rms={:.9},peak={peak:.9},range={range:.9}",
                    (square / 65_536.0).sqrt()
                );
            }

            let curve = compile_probe(shape, 6);
            let steps = lane_steps(105.0);
            let mut phases = f32x8::from([0.173, 0.271, 0.369, 0.467, 0.565, 0.663, 0.761, 0.859]);
            let mut selector = [0_u16; 8];
            let mut output = [f32x8::ZERO; 64];
            let mut representation_step = 0.0_f32;
            let mut reversal_step = 0.0_f32;
            for sample in 0..4096 {
                let eligible = (sample / 137) & 1 == 0;
                let phase = phases;
                let before = f32x8::from(selector.map(|value| f32::from(value) / 1024.0));
                let projected = curve.eval8(phase);
                let current = current8(shape, phase, steps);
                let mix = advance_selector(&mut selector, eligible);
                let delta: [f32; 8] = ((projected - current) * (mix - before)).into();
                reversal_step = reversal_step.max(
                    delta
                        .into_iter()
                        .fold(0.0_f32, |peak, value| peak.max(value.abs())),
                );
                representation_step = representation_step.max(
                    <[f32; 8]>::from((projected - current) * f32x8::splat(1.0 / 1024.0))
                        .into_iter()
                        .fold(0.0_f32, |peak, value| peak.max(value.abs())),
                );
                let next = phases + steps;
                phases = next.cmp_lt(f32x8::ONE).blend(next, next - f32x8::ONE);
            }
            let stale_before = selector;
            for _ in 0..2048 {
                render_probe_block(
                    shape,
                    curve,
                    &mut phases,
                    steps,
                    &mut selector,
                    false,
                    &mut output,
                );
            }
            let stale_cleared = selector == [0; 8];
            render_probe_block(
                shape,
                curve,
                &mut phases,
                steps,
                &mut selector,
                true,
                &mut output,
            );
            let resumed = selector == [64; 8];
            println!(
                "canonical_coeff_transition,shape={name},step_bound={representation_step:.9},reversal_step={reversal_step:.9},stale_before={stale_before:?},stale_cleared={stale_cleared},resume_after_block={resumed},side_state_bytes={}",
                std::mem::size_of_val(&selector)
            );

            let benchmark = |mode: usize, blocks: usize| {
                let mut times = Vec::new();
                for _ in 0..5 {
                    let mut phases =
                        f32x8::from([0.173, 0.271, 0.369, 0.467, 0.565, 0.663, 0.761, 0.859]);
                    let mut selector = [0_u16; 8];
                    let mut output = [f32x8::ZERO; 64];
                    let started = Instant::now();
                    for block in 0..blocks {
                        match mode {
                            0 => {
                                for sample in &mut output {
                                    let phase = phases;
                                    *sample = current8(shape, phase, steps);
                                    let next = phases + steps;
                                    phases = next.cmp_lt(f32x8::ONE).blend(next, next - f32x8::ONE);
                                }
                            }
                            1 => {
                                selector = [PROBE_SLEW; 8];
                                render_probe_block(
                                    shape,
                                    curve,
                                    &mut phases,
                                    steps,
                                    &mut selector,
                                    true,
                                    &mut output,
                                );
                            }
                            2 => {
                                let eligible = (block / 16) & 1 == 0;
                                render_probe_block(
                                    shape,
                                    curve,
                                    &mut phases,
                                    steps,
                                    &mut selector,
                                    eligible,
                                    &mut output,
                                );
                            }
                            _ => {
                                let eligible = block >= 6;
                                render_probe_block(
                                    shape,
                                    curve,
                                    &mut phases,
                                    steps,
                                    &mut selector,
                                    eligible,
                                    &mut output,
                                );
                            }
                        }
                        black_box(output);
                    }
                    times.push(started.elapsed().as_nanos() as f64 / (blocks * 64) as f64);
                }
                times.sort_by(f64::total_cmp);
                times[times.len() / 2]
            };
            println!(
                "canonical_coeff_cpu,shape={name},current_ns={:.3},steady_ns={:.3},continuous_transition_ns={:.3},note100ms_ns={:.3},note500ms_ns={:.3},smoother_samples=384,selector_samples=1024,full_latency_samples=1408",
                benchmark(0, 20_000),
                benchmark(1, 20_000),
                benchmark(2, 20_000),
                benchmark(3, 75),
                benchmark(3, 375),
            );
        }
    }

    #[test]
    #[ignore = "constant-x8 returned-sample seam experiment"]
    fn constant_x8_sample_kernel_report() {
        use std::hint::black_box;
        use std::time::Instant;

        const SAMPLES: usize = 64;
        let step = f32x8::from([
            0.0831, 0.0832, 0.0833, 0.0834, 0.0835, 0.0836, 0.0837, 0.0838,
        ]);
        let left_gain = f32x8::from([0.11, 0.12, 0.13, 0.14, 0.15, 0.16, 0.17, 0.18]);
        let right_gain = f32x8::from([0.18, 0.17, 0.16, 0.15, 0.14, 0.13, 0.12, 0.11]);
        let curve = crate::wave_curve::WaveCurveRt::default();

        macro_rules! direct_loop {
            ($oscillators:expr, $left:expr, $right:expr, |$phase:ident, $phase_step:ident| $sample:expr) => {{
                let mut phase = f32x8::from(std::array::from_fn(|lane| $oscillators[lane].phase));
                for frame in 0..SAMPLES {
                    let $phase = phase;
                    let $phase_step = step;
                    let next = phase + step;
                    phase = next.cmp_lt(f32x8::ONE).blend(next, next - f32x8::ONE);
                    let sample = $sample;
                    $left[frame] = sample.mul_add(left_gain, $left[frame]);
                    $right[frame] = sample.mul_add(right_gain, $right[frame]);
                }
                let phases: [f32; 8] = phase.into();
                for (oscillator, phase) in $oscillators.iter_mut().zip(phases) {
                    oscillator.phase = phase;
                }
            }};
        }

        for mode in 0..4 {
            let name = ["saw", "square", "custom", "warp"][mode];
            let run = |candidate: bool, blocks: usize| {
                let mut oscillators = [super::VaOscillator::default(); 8];
                for (lane, oscillator) in oscillators.iter_mut().enumerate() {
                    oscillator.phase = 0.071 + lane as f32 * 0.103;
                }
                let mut left = [f32x8::ZERO; SAMPLES];
                let mut right = [f32x8::ZERO; SAMPLES];
                let started = Instant::now();
                for _ in 0..blocks {
                    if candidate {
                        match mode {
                            0 => probe_sample_kernel(
                                &mut oscillators,
                                step,
                                left_gain,
                                right_gain,
                                &mut left,
                                &mut right,
                                |phase, step| {
                                    super::bandlimited_saw8(
                                        phase,
                                        step,
                                        super::Antialiasing::SplineOptimized,
                                    )
                                },
                            ),
                            1 => probe_sample_kernel(
                                &mut oscillators,
                                step,
                                left_gain,
                                right_gain,
                                &mut left,
                                &mut right,
                                |phase, step| {
                                    super::bandlimited_pulse8(
                                        phase,
                                        step,
                                        0.5,
                                        super::Antialiasing::SplineOptimized,
                                    )
                                },
                            ),
                            2 => probe_sample_kernel(
                                &mut oscillators,
                                step,
                                left_gain,
                                right_gain,
                                &mut left,
                                &mut right,
                                |phase, step| {
                                    let canonical = super::bandlimited_saw8(
                                        phase,
                                        step,
                                        super::Antialiasing::SplineOptimized,
                                    );
                                    (curve.eval8(phase) - canonical)
                                        .mul_add(f32x8::splat(0.5), canonical)
                                },
                            ),
                            _ => probe_sample_kernel(
                                &mut oscillators,
                                step,
                                left_gain,
                                right_gain,
                                &mut left,
                                &mut right,
                                |raw, step| {
                                    let (phase, warped_step) = super::warp_phase8(
                                        raw,
                                        step,
                                        super::PhaseWarpMode::PhaseBend,
                                        0.35,
                                    );
                                    super::sample_shape8_warped_at_auto_edge(
                                        raw,
                                        step,
                                        phase,
                                        warped_step,
                                        2.0,
                                        0.5,
                                        super::Antialiasing::SplineOptimized,
                                        super::PhaseWarpMode::PhaseBend,
                                        0.35,
                                    )
                                },
                            ),
                        }
                    } else {
                        match mode {
                            0 => super::super::backend::accumulate_saw8_block_constant(
                                &mut oscillators,
                                step,
                                left_gain,
                                right_gain,
                                &mut left,
                                &mut right,
                                super::Antialiasing::SplineOptimized,
                            ),
                            1 => super::accumulate_shape8_block_constant(
                                &mut oscillators,
                                step,
                                left_gain,
                                right_gain,
                                &mut left,
                                &mut right,
                                3.0,
                                0.5,
                                super::Antialiasing::SplineOptimized,
                            ),
                            2 => super::accumulate_custom8_block_constant(
                                &mut oscillators,
                                step,
                                left_gain,
                                right_gain,
                                &mut left,
                                &mut right,
                                curve,
                                0.5,
                                2.0,
                                0.5,
                                super::Antialiasing::SplineOptimized,
                                super::PhaseWarpMode::None,
                                0.0,
                            ),
                            _ => super::accumulate_shape8_block_constant_warped(
                                &mut oscillators,
                                step,
                                left_gain,
                                right_gain,
                                &mut left,
                                &mut right,
                                2.0,
                                0.5,
                                super::Antialiasing::SplineOptimized,
                                super::PhaseWarpMode::PhaseBend,
                                0.35,
                            ),
                        }
                    }
                    black_box((&left, &right));
                }
                (
                    started.elapsed().as_nanos() as f64 / (blocks * SAMPLES) as f64,
                    oscillators,
                    left,
                    right,
                )
            };
            let run_direct = |blocks: usize| {
                let mut oscillators = [super::VaOscillator::default(); 8];
                for (lane, oscillator) in oscillators.iter_mut().enumerate() {
                    oscillator.phase = 0.071 + lane as f32 * 0.103;
                }
                let mut left = [f32x8::ZERO; SAMPLES];
                let mut right = [f32x8::ZERO; SAMPLES];
                let started = Instant::now();
                for _ in 0..blocks {
                    match mode {
                        0 => direct_loop!(&mut oscillators, left, right, |phase, phase_step| {
                            super::bandlimited_saw8(
                                phase,
                                phase_step,
                                super::Antialiasing::SplineOptimized,
                            )
                        }),
                        1 => direct_loop!(&mut oscillators, left, right, |phase, phase_step| {
                            super::bandlimited_pulse8(
                                phase,
                                phase_step,
                                0.5,
                                super::Antialiasing::SplineOptimized,
                            )
                        }),
                        2 => direct_loop!(&mut oscillators, left, right, |phase, phase_step| {
                            let canonical = super::bandlimited_saw8(
                                phase,
                                phase_step,
                                super::Antialiasing::SplineOptimized,
                            );
                            (curve.eval8(phase) - canonical).mul_add(f32x8::splat(0.5), canonical)
                        }),
                        _ => direct_loop!(&mut oscillators, left, right, |raw, phase_step| {
                            let (phase, warped_step) = super::warp_phase8(
                                raw,
                                phase_step,
                                super::PhaseWarpMode::PhaseBend,
                                0.35,
                            );
                            super::sample_shape8_warped_at_auto_edge(
                                raw,
                                phase_step,
                                phase,
                                warped_step,
                                2.0,
                                0.5,
                                super::Antialiasing::SplineOptimized,
                                super::PhaseWarpMode::PhaseBend,
                                0.35,
                            )
                        }),
                    }
                    black_box((&left, &right));
                }
                (
                    started.elapsed().as_nanos() as f64 / (blocks * SAMPLES) as f64,
                    oscillators,
                    left,
                    right,
                )
            };
            let (_, baseline_osc, baseline_left, baseline_right) = run(false, 1);
            let (_, candidate_osc, candidate_left, candidate_right) = run(true, 1);
            let phase_peak = baseline_osc
                .iter()
                .zip(candidate_osc)
                .fold(0.0_f32, |peak, (a, b)| peak.max((a.phase - b.phase).abs()));
            let output_peak = baseline_left
                .into_iter()
                .chain(baseline_right)
                .zip(candidate_left.into_iter().chain(candidate_right))
                .fold(0.0_f32, |peak, (a, b)| {
                    <[f32; 8]>::from(a - b)
                        .into_iter()
                        .fold(peak, |peak, error| peak.max(error.abs()))
                });
            let baseline = (0..5)
                .map(|_| run(false, 20_000).0)
                .min_by(f64::total_cmp)
                .unwrap();
            let candidate = (0..5)
                .map(|_| run(true, 20_000).0)
                .min_by(f64::total_cmp)
                .unwrap();
            let (_, direct_osc, direct_left, direct_right) = run_direct(1);
            let direct_phase_peak = baseline_osc
                .iter()
                .zip(direct_osc)
                .fold(0.0_f32, |peak, (a, b)| peak.max((a.phase - b.phase).abs()));
            let direct_output_peak = baseline_left
                .into_iter()
                .chain(baseline_right)
                .zip(direct_left.into_iter().chain(direct_right))
                .fold(0.0_f32, |peak, (a, b)| {
                    <[f32; 8]>::from(a - b)
                        .into_iter()
                        .fold(peak, |peak, error| peak.max(error.abs()))
                });
            let direct = (0..5)
                .map(|_| run_direct(20_000).0)
                .min_by(f64::total_cmp)
                .unwrap();
            println!(
                "constant_x8_sample_kernel,mode={name},baseline_ns={baseline:.3},candidate_ns={candidate:.3},delta_pct={:.2},direct_ns={direct:.3},direct_delta_pct={:.2},phase_peak={phase_peak:.12},output_peak={output_peak:.9},direct_phase_peak={direct_phase_peak:.12},direct_output_peak={direct_output_peak:.9},kernel_state_bytes={}",
                (candidate / baseline - 1.0) * 100.0,
                (direct / baseline - 1.0) * 100.0,
                std::mem::size_of::<f32x8>()
            );
        }
    }

    #[test]
    #[ignore = "sparse canonical BLEP block schedule experiment"]
    fn sparse_saw_event_schedule_report() {
        use std::hint::black_box;
        use std::time::Instant;

        const SAMPLES: usize = 64;
        let gains = f32x8::from([0.11, 0.12, 0.13, 0.14, 0.15, 0.16, 0.17, 0.18]);
        for (name, base_step) in [("low", 0.0046_f32), ("mid", 0.041), ("high", 0.083)] {
            let step = f32x8::from(std::array::from_fn(|lane| {
                base_step * (1.0 + lane as f32 * 0.000_13)
            }));
            let run = |scheduled: bool, blocks: usize| {
                let mut oscillators = [super::VaOscillator::default(); 8];
                for (lane, oscillator) in oscillators.iter_mut().enumerate() {
                    oscillator.phase = (0.997 + lane as f32 * 0.101).fract();
                }
                let mut left = [f32x8::ZERO; SAMPLES];
                let mut right = [f32x8::ZERO; SAMPLES];
                let mut event_frames = 0;
                let started = Instant::now();
                for _ in 0..blocks {
                    if scheduled {
                        event_frames += probe_scheduled_saw8(
                            &mut oscillators,
                            step,
                            gains,
                            gains,
                            &mut left,
                            &mut right,
                        );
                    } else {
                        super::super::backend::accumulate_saw8_block_constant(
                            &mut oscillators,
                            step,
                            gains,
                            gains,
                            &mut left,
                            &mut right,
                            super::Antialiasing::SplineOptimized,
                        );
                    }
                    black_box((&left, &right));
                }
                (
                    started.elapsed().as_nanos() as f64 / (blocks * SAMPLES) as f64,
                    oscillators,
                    left,
                    right,
                    event_frames,
                )
            };
            let (_, baseline_osc, baseline_left, baseline_right, _) = run(false, 1);
            let (_, scheduled_osc, scheduled_left, scheduled_right, event_frames) = run(true, 1);
            let phase_peak = baseline_osc
                .iter()
                .zip(scheduled_osc)
                .fold(0.0_f32, |peak, (a, b)| peak.max((a.phase - b.phase).abs()));
            let output_peak = baseline_left
                .into_iter()
                .chain(baseline_right)
                .zip(scheduled_left.into_iter().chain(scheduled_right))
                .fold(0.0_f32, |peak, (a, b)| {
                    <[f32; 8]>::from(a - b)
                        .into_iter()
                        .fold(peak, |peak, error| peak.max(error.abs()))
                });
            let baseline = (0..5)
                .map(|_| run(false, 20_000).0)
                .min_by(f64::total_cmp)
                .unwrap();
            let scheduled = (0..5)
                .map(|_| run(true, 20_000).0)
                .min_by(f64::total_cmp)
                .unwrap();

            let scalar = |scheduled: bool, blocks: usize| {
                let mut phase = 0.997_f64;
                let step = f64::from(base_step);
                let support = step * 2.0;
                let started = Instant::now();
                let mut sum = 0.0;
                for _ in 0..blocks {
                    let schedule: [bool; SAMPLES] = if scheduled {
                        let mut scheduled_phase = phase;
                        std::array::from_fn(|_| {
                            let event =
                                scheduled_phase < support || scheduled_phase > 1.0 - support;
                            scheduled_phase = (scheduled_phase + step).fract();
                            event
                        })
                    } else {
                        [false; SAMPLES]
                    };
                    for event in schedule {
                        let sample = if !scheduled || event {
                            super::bandlimited_saw(
                                phase,
                                step,
                                super::Antialiasing::SplineOptimized,
                            )
                        } else {
                            phase.mul_add(2.0, -1.0)
                        };
                        sum += sample;
                        phase = (phase + step).fract();
                    }
                }
                black_box(sum);
                started.elapsed().as_nanos() as f64 / (blocks * SAMPLES) as f64
            };
            let scalar_current = (0..5)
                .map(|_| scalar(false, 20_000))
                .min_by(f64::total_cmp)
                .unwrap();
            let scalar_scheduled = (0..5)
                .map(|_| scalar(true, 20_000))
                .min_by(f64::total_cmp)
                .unwrap();
            println!(
                "sparse_saw_schedule,range={name},event_frames={event_frames}/{SAMPLES},x8_current_ns={baseline:.3},x8_scheduled_ns={scheduled:.3},x8_delta_pct={:.2},scalar_current_ns={scalar_current:.3},scalar_scheduled_ns={scalar_scheduled:.3},scalar_delta_pct={:.2},phase_peak={phase_peak:.12},output_peak={output_peak:.9},schedule_bytes={}",
                (scheduled / baseline - 1.0) * 100.0,
                (scalar_scheduled / scalar_current - 1.0) * 100.0,
                std::mem::size_of::<[bool; SAMPLES]>()
            );
        }
    }

    #[test]
    #[ignore = "analytic sparse canonical BLEP iterator experiment"]
    fn analytic_saw_event_iterator_report() {
        use std::hint::black_box;
        use std::time::Instant;

        let gains = f32x8::from([0.11, 0.12, 0.13, 0.14, 0.15, 0.16, 0.17, 0.18]);
        for (name, base_step) in [("low", 0.0046_f32), ("mid", 0.041), ("high", 0.083)] {
            let step = f32x8::from(std::array::from_fn(|lane| {
                base_step * (1.0 + lane as f32 * 0.000_13)
            }));
            let run = |analytic: bool, blocks: usize| {
                let mut oscillators = [super::VaOscillator::default(); 8];
                for (lane, oscillator) in oscillators.iter_mut().enumerate() {
                    oscillator.phase = (0.997 + lane as f32 * 0.101).fract();
                }
                let mut left = [f32x8::ZERO; 64];
                let mut right = [f32x8::ZERO; 64];
                let mut event_bits = 0_u64;
                let started = Instant::now();
                for _ in 0..blocks {
                    if analytic {
                        event_bits |= probe_analytic_saw8(
                            &mut oscillators,
                            step,
                            gains,
                            gains,
                            &mut left,
                            &mut right,
                        );
                    } else {
                        super::super::backend::accumulate_saw8_block_constant(
                            &mut oscillators,
                            step,
                            gains,
                            gains,
                            &mut left,
                            &mut right,
                            super::Antialiasing::SplineOptimized,
                        );
                    }
                    black_box((&left, &right));
                }
                (
                    started.elapsed().as_nanos() as f64 / (blocks * 64) as f64,
                    oscillators,
                    left,
                    right,
                    event_bits,
                )
            };
            let (_, baseline_osc, baseline_left, baseline_right, _) = run(false, 1);
            let (_, analytic_osc, analytic_left, analytic_right, event_bits) = run(true, 1);
            let phase_peak = baseline_osc
                .iter()
                .zip(analytic_osc)
                .fold(0.0_f32, |peak, (a, b)| peak.max((a.phase - b.phase).abs()));
            let output_peak = baseline_left
                .into_iter()
                .chain(baseline_right)
                .zip(analytic_left.into_iter().chain(analytic_right))
                .fold(0.0_f32, |peak, (a, b)| {
                    <[f32; 8]>::from(a - b)
                        .into_iter()
                        .fold(peak, |peak, error| peak.max(error.abs()))
                });
            let current = (0..5)
                .map(|_| run(false, 20_000).0)
                .min_by(f64::total_cmp)
                .unwrap();
            let analytic = (0..5)
                .map(|_| run(true, 20_000).0)
                .min_by(f64::total_cmp)
                .unwrap();

            let scalar = |analytic: bool, blocks: usize| {
                let mut phase = 0.997_f64;
                let step = f64::from(base_step);
                let support = step * 2.0;
                let started = Instant::now();
                let mut sum = 0.0;
                for _ in 0..blocks {
                    let mut events = 0_u64;
                    if analytic {
                        let final_cycle = (phase + step * 64.0).ceil() as i32;
                        for cycle in 0..=final_cycle + 1 {
                            let crossing = ((f64::from(cycle) - phase) / step).ceil() as i32;
                            for frame in crossing - 3..=crossing + 2 {
                                if (0..64).contains(&frame) {
                                    events |= 1_u64 << frame;
                                }
                            }
                        }
                    }
                    for frame in 0..64 {
                        let event = phase < support || phase > 1.0 - support;
                        let sample = if !analytic || events & (1_u64 << frame) != 0 {
                            super::bandlimited_saw(
                                phase,
                                step,
                                super::Antialiasing::SplineOptimized,
                            )
                        } else {
                            debug_assert!(!event);
                            phase.mul_add(2.0, -1.0)
                        };
                        sum += sample;
                        phase = (phase + step).fract();
                    }
                }
                black_box(sum);
                started.elapsed().as_nanos() as f64 / (blocks * 64) as f64
            };
            let scalar_current = (0..5)
                .map(|_| scalar(false, 20_000))
                .min_by(f64::total_cmp)
                .unwrap();
            let scalar_analytic = (0..5)
                .map(|_| scalar(true, 20_000))
                .min_by(f64::total_cmp)
                .unwrap();
            println!(
                "analytic_saw_iterator,range={name},candidate_frames={}/64,x8_current_ns={current:.3},x8_analytic_ns={analytic:.3},x8_delta_pct={:.2},scalar_current_ns={scalar_current:.3},scalar_analytic_ns={scalar_analytic:.3},scalar_delta_pct={:.2},phase_peak={phase_peak:.12},output_peak={output_peak:.9},iterator_bytes={}",
                event_bits.count_ones(),
                (analytic / current - 1.0) * 100.0,
                (scalar_analytic / scalar_current - 1.0) * 100.0,
                std::mem::size_of::<u64>()
            );
        }
    }

    #[test]
    #[ignore = "density-adaptive canonical residual experiment"]
    fn adaptive_event_lane_report() {
        use std::hint::black_box;
        use std::time::Instant;

        for (range, base_step) in [("low", 0.0046_f32), ("mid", 0.041), ("high", 0.083)] {
            for coherent in [false, true] {
                let phase0 = if coherent {
                    [0.997; 8]
                } else {
                    [0.997, 0.121, 0.247, 0.373, 0.499, 0.625, 0.751, 0.877]
                };
                let step = f32x8::from(std::array::from_fn(|lane| {
                    base_step * (1.0 + lane as f32 * 0.000_13)
                }));
                for shape in 0..3 {
                    let shape_name = ["saw", "square", "pulse37"][shape];
                    let width = if shape == 2 { 0.37 } else { 0.5 };
                    for threshold in 1..=3 {
                        let render = |adaptive: bool, blocks: usize| {
                            let mut phase = f32x8::from(phase0);
                            let mut left = [f32x8::ZERO; 64];
                            let gain = f32x8::splat(0.137);
                            let started = Instant::now();
                            for _ in 0..blocks {
                                for output in &mut left {
                                    let current = phase;
                                    let next = phase + step;
                                    phase = next.cmp_lt(f32x8::ONE).blend(next, next - f32x8::ONE);
                                    let correction = if adaptive {
                                        probe_adaptive_blep8(current, step, threshold)
                                    } else {
                                        let active = step.cmp_gt(f32x8::splat(f32::EPSILON));
                                        let support = step * f32x8::splat(2.0);
                                        super::spline_blep8_precomputed(
                                            current,
                                            active,
                                            support,
                                            f32x8::ONE / active.blend(step, f32x8::ONE),
                                            true,
                                        )
                                    };
                                    let sample = if shape == 0 {
                                        current * f32x8::splat(2.0) - f32x8::ONE - correction
                                    } else {
                                        let width8 = f32x8::splat(width);
                                        let shifted =
                                            super::wrap_phase8(current + f32x8::ONE - width8);
                                        let width_correction = if adaptive {
                                            probe_adaptive_blep8(shifted, step, threshold)
                                        } else {
                                            let active = step.cmp_gt(f32x8::splat(f32::EPSILON));
                                            let support = step * f32x8::splat(2.0);
                                            super::spline_blep8_precomputed(
                                                shifted,
                                                active,
                                                support,
                                                f32x8::ONE / active.blend(step, f32x8::ONE),
                                                true,
                                            )
                                        };
                                        current.cmp_lt(width8).blend(f32x8::ONE, -f32x8::ONE)
                                            + correction
                                            - width_correction
                                    };
                                    *output = sample.mul_add(gain, *output);
                                }
                                black_box(&left);
                            }
                            (
                                started.elapsed().as_nanos() as f64 / (blocks * 64) as f64,
                                phase,
                                left,
                            )
                        };
                        let (_, current_phase, current_output) = render(false, 1);
                        let (_, adaptive_phase, adaptive_output) = render(true, 1);
                        let phase_peak = <[f32; 8]>::from(current_phase - adaptive_phase)
                            .into_iter()
                            .fold(0.0_f32, |peak, value| peak.max(value.abs()));
                        let output_peak = current_output
                            .into_iter()
                            .zip(adaptive_output)
                            .flat_map(|(a, b)| <[f32; 8]>::from(a - b))
                            .fold(0.0_f32, |peak, value| peak.max(value.abs()));
                        let current = (0..3)
                            .map(|_| render(false, 8_000).0)
                            .min_by(f64::total_cmp)
                            .unwrap();
                        let adaptive = (0..3)
                            .map(|_| render(true, 8_000).0)
                            .min_by(f64::total_cmp)
                            .unwrap();
                        println!(
                            "adaptive_event_lanes,range={range},coherent={coherent},shape={shape_name},threshold={threshold},current_ns={current:.3},adaptive_ns={adaptive:.3},delta_pct={:.2},phase_peak={phase_peak:.12},output_peak={output_peak:.9}",
                            (adaptive / current - 1.0) * 100.0
                        );
                    }
                }
            }
        }
    }

    #[test]
    #[ignore = "branchy versus branchless cubic residual experiment"]
    fn branchless_residual_report() {
        use std::hint::black_box;
        use std::time::Instant;

        for (range, base_step) in [("low", 0.0046_f32), ("mid", 0.041), ("high", 0.083)] {
            let step = f32x8::from(std::array::from_fn(|lane| {
                base_step * (1.0 + lane as f32 * 0.000_13)
            }));
            let harmonic_cap = (0.5 / base_step).floor() as usize;
            for shape in 0..4 {
                let shape_name = ["saw", "square", "pulse37", "triangle"][shape];
                let mut optimized_square = 0.0_f64;
                let mut branchless_square = 0.0_f64;
                let mut optimized_peak = 0.0_f64;
                let mut branchless_peak = 0.0_f64;
                for index in 0..65_536 {
                    let phase = index as f64 / 65_536.0;
                    let angle = std::f64::consts::TAU * phase;
                    let ideal = match shape {
                        0 => (1..=harmonic_cap).fold(0.0, |sum, harmonic| {
                            sum - 2.0 * (angle * harmonic as f64).sin()
                                / (std::f64::consts::PI * harmonic as f64)
                        }),
                        1 => (1..=harmonic_cap).step_by(2).fold(0.0, |sum, harmonic| {
                            sum + 4.0 * (angle * harmonic as f64).sin()
                                / (std::f64::consts::PI * harmonic as f64)
                        }),
                        2 => (1..=harmonic_cap).fold(-0.26, |sum, harmonic| {
                            let harmonic = harmonic as f64;
                            sum + 4.0
                                * (std::f64::consts::PI * harmonic * 0.37).sin()
                                * (std::f64::consts::TAU * harmonic * (phase - 0.185)).cos()
                                / (std::f64::consts::PI * harmonic)
                        }),
                        _ => (1..=harmonic_cap).step_by(2).fold(0.0, |sum, harmonic| {
                            let harmonic = harmonic as f64;
                            sum - 8.0 * (angle * harmonic).cos()
                                / (std::f64::consts::PI.powi(2) * harmonic * harmonic)
                        }),
                    };
                    let sample = |antialiasing| match shape {
                        0 => super::bandlimited_saw(phase, f64::from(base_step), antialiasing),
                        1 => {
                            super::bandlimited_pulse(phase, f64::from(base_step), 0.5, antialiasing)
                        }
                        2 => super::bandlimited_pulse(
                            phase,
                            f64::from(base_step),
                            0.37,
                            antialiasing,
                        ),
                        _ => super::bandlimited_triangle(phase, f64::from(base_step), antialiasing),
                    };
                    let optimized = sample(super::Antialiasing::SplineOptimized) - ideal;
                    let branchless = sample(super::Antialiasing::Spline) - ideal;
                    optimized_square += optimized * optimized;
                    branchless_square += branchless * branchless;
                    optimized_peak = optimized_peak.max(optimized.abs());
                    branchless_peak = branchless_peak.max(branchless.abs());
                }
                println!(
                    "branchless_quality,range={range},shape={shape_name},cap={harmonic_cap},optimized_rms={:.9},branchless_rms={:.9},optimized_peak={optimized_peak:.9},branchless_peak={branchless_peak:.9}",
                    (optimized_square / 65_536.0).sqrt(),
                    (branchless_square / 65_536.0).sqrt()
                );
            }
            for coherent in [false, true] {
                let phases = if coherent {
                    [0.997; 8]
                } else {
                    [0.997, 0.121, 0.247, 0.373, 0.499, 0.625, 0.751, 0.877]
                };
                for shape in 0..4 {
                    let shape_name = ["saw", "square", "pulse37", "triangle"][shape];
                    let width = if shape == 2 { 0.37 } else { 0.5 };
                    let run = |optimized: bool, blocks: usize| {
                        let mut oscillators = [super::VaOscillator::default(); 8];
                        for (oscillator, phase) in oscillators.iter_mut().zip(phases) {
                            oscillator.phase = phase;
                        }
                        let mut left = [f32x8::ZERO; 64];
                        let mut right = [f32x8::ZERO; 64];
                        let gain = f32x8::splat(0.137);
                        let antialiasing = if optimized {
                            super::Antialiasing::SplineOptimized
                        } else {
                            super::Antialiasing::Spline
                        };
                        let started = Instant::now();
                        for _ in 0..blocks {
                            if shape == 0 {
                                super::super::backend::accumulate_saw8_block_constant(
                                    &mut oscillators,
                                    step,
                                    gain,
                                    gain,
                                    &mut left,
                                    &mut right,
                                    antialiasing,
                                );
                            } else {
                                let shape_value = if shape == 3 { 1.0 } else { 3.0 };
                                super::accumulate_shape8_block_constant(
                                    &mut oscillators,
                                    step,
                                    gain,
                                    gain,
                                    &mut left,
                                    &mut right,
                                    shape_value,
                                    width,
                                    antialiasing,
                                );
                            }
                            black_box((&left, &right));
                        }
                        (
                            started.elapsed().as_nanos() as f64 / (blocks * 64) as f64,
                            left,
                        )
                    };
                    let (_, optimized_output) = run(true, 1);
                    let (_, branchless_output) = run(false, 1);
                    let mut square = 0.0_f64;
                    let mut peak = 0.0_f32;
                    for (a, b) in optimized_output.into_iter().zip(branchless_output) {
                        for error in <[f32; 8]>::from(a - b) {
                            square += f64::from(error) * f64::from(error);
                            peak = peak.max(error.abs());
                        }
                    }
                    let optimized = (0..5)
                        .map(|_| run(true, 12_000).0)
                        .min_by(f64::total_cmp)
                        .unwrap();
                    let branchless = (0..5)
                        .map(|_| run(false, 12_000).0)
                        .min_by(f64::total_cmp)
                        .unwrap();
                    println!(
                        "branchless_residual,range={range},coherent={coherent},shape={shape_name},optimized_ns={optimized:.3},branchless_ns={branchless:.3},delta_pct={:.2},difference_rms={:.9},difference_peak={peak:.9}",
                        (branchless / optimized - 1.0) * 100.0,
                        (square / 512.0).sqrt()
                    );
                }
            }
        }
    }

    #[test]
    #[ignore = "stateless pitch crossover residual experiment"]
    fn pitch_crossover_residual_report() {
        use std::hint::black_box;
        use std::time::Instant;

        const CENTER: f32 = 0.025;
        for band in [0.0_f32, 0.004, 0.012] {
            for (range, base_step) in [
                ("low", 0.0046_f32),
                ("band", CENTER),
                ("mid", 0.041),
                ("high", 0.083),
            ] {
                let mix = if band == 0.0 {
                    f32::from(base_step >= CENTER)
                } else {
                    ((base_step - (CENTER - band * 0.5)) / band).clamp(0.0, 1.0)
                };
                let step = f32x8::from(std::array::from_fn(|lane| {
                    base_step * (1.0 + lane as f32 * 0.000_13)
                }));
                for coherent in [false, true] {
                    let phases = if coherent {
                        [0.997; 8]
                    } else {
                        [0.997, 0.121, 0.247, 0.373, 0.499, 0.625, 0.751, 0.877]
                    };
                    for shape in 0..3 {
                        let shape_name = ["saw", "pulse37", "triangle"][shape];
                        let run = |candidate: bool, blocks: usize| {
                            let mut oscillators = [super::VaOscillator::default(); 8];
                            for (oscillator, phase) in oscillators.iter_mut().zip(phases) {
                                oscillator.phase = phase;
                            }
                            let mut left = [f32x8::ZERO; 64];
                            let mut right = [f32x8::ZERO; 64];
                            let gain = f32x8::splat(0.137);
                            let started = Instant::now();
                            for _ in 0..blocks {
                                if !candidate || mix == 0.0 {
                                    if shape == 0 {
                                        super::super::backend::accumulate_saw8_block_constant(
                                            &mut oscillators,
                                            step,
                                            gain,
                                            gain,
                                            &mut left,
                                            &mut right,
                                            super::Antialiasing::SplineOptimized,
                                        );
                                    } else {
                                        super::accumulate_shape8_block_constant(
                                            &mut oscillators,
                                            step,
                                            gain,
                                            gain,
                                            &mut left,
                                            &mut right,
                                            if shape == 1 { 3.0 } else { 1.0 },
                                            0.37,
                                            super::Antialiasing::SplineOptimized,
                                        );
                                    }
                                } else if mix == 1.0 {
                                    if shape == 0 {
                                        super::super::backend::accumulate_saw8_block_constant(
                                            &mut oscillators,
                                            step,
                                            gain,
                                            gain,
                                            &mut left,
                                            &mut right,
                                            super::Antialiasing::Spline,
                                        );
                                    } else {
                                        super::accumulate_shape8_block_constant(
                                            &mut oscillators,
                                            step,
                                            gain,
                                            gain,
                                            &mut left,
                                            &mut right,
                                            if shape == 1 { 3.0 } else { 1.0 },
                                            0.37,
                                            super::Antialiasing::Spline,
                                        );
                                    }
                                } else {
                                    let mut phase = f32x8::from(std::array::from_fn(|lane| {
                                        oscillators[lane].phase
                                    }));
                                    let active = step.cmp_gt(f32x8::splat(f32::EPSILON));
                                    let support = step * f32x8::splat(2.0);
                                    let inverse = f32x8::ONE / active.blend(step, f32x8::ONE);
                                    for frame in 0..64 {
                                        let current = phase;
                                        let next = phase + step;
                                        phase =
                                            next.cmp_lt(f32x8::ONE).blend(next, next - f32x8::ONE);
                                        let sample = if shape == 2 {
                                            let optimized = super::spline_triangle8_precomputed(
                                                current, step, active, support, inverse, true,
                                            );
                                            let branchless = super::spline_triangle8_precomputed(
                                                current, step, active, support, inverse, false,
                                            );
                                            (branchless - optimized)
                                                .mul_add(f32x8::splat(mix), optimized)
                                        } else {
                                            let correction = |phase| {
                                                let optimized = super::spline_blep8_precomputed(
                                                    phase, active, support, inverse, true,
                                                );
                                                let branchless = super::spline_blep8_precomputed(
                                                    phase, active, support, inverse, false,
                                                );
                                                (branchless - optimized)
                                                    .mul_add(f32x8::splat(mix), optimized)
                                            };
                                            let wrap = correction(current);
                                            if shape == 0 {
                                                current * f32x8::splat(2.0) - f32x8::ONE - wrap
                                            } else {
                                                let width = f32x8::splat(0.37);
                                                let shifted = super::wrap_phase8(
                                                    current + f32x8::ONE - width,
                                                );
                                                current.cmp_lt(width).blend(f32x8::ONE, -f32x8::ONE)
                                                    + wrap
                                                    - correction(shifted)
                                            }
                                        };
                                        left[frame] = sample.mul_add(gain, left[frame]);
                                        right[frame] = sample.mul_add(gain, right[frame]);
                                    }
                                    let phases: [f32; 8] = phase.into();
                                    for (oscillator, phase) in oscillators.iter_mut().zip(phases) {
                                        oscillator.phase = phase;
                                    }
                                }
                                black_box((&left, &right));
                            }
                            started.elapsed().as_nanos() as f64 / (blocks * 64) as f64
                        };
                        let current = (0..3)
                            .map(|_| run(false, 8_000))
                            .min_by(f64::total_cmp)
                            .unwrap();
                        let candidate = (0..3)
                            .map(|_| run(true, 8_000))
                            .min_by(f64::total_cmp)
                            .unwrap();
                        println!(
                            "pitch_crossover,band={band:.3},range={range},mix={mix:.3},coherent={coherent},shape={shape_name},current_ns={current:.3},candidate_ns={candidate:.3},delta_pct={:.2}",
                            (candidate / current - 1.0) * 100.0
                        );
                    }
                }
            }
            for shape in 0..3 {
                let shape_name = ["saw", "pulse37", "triangle"][shape];
                let mut phase = 0.317_f64;
                let mut step = 0.015_f32;
                let mut direction = 1.0_f32;
                let mut previous_delta = 0.0_f64;
                let mut previous_mix = 0.0_f64;
                let mut artifact_peak = 0.0_f64;
                for _ in 0..4096 {
                    let mix = if band == 0.0 {
                        f64::from(step >= CENTER)
                    } else {
                        f64::from(((step - (CENTER - band * 0.5)) / band).clamp(0.0, 1.0))
                    };
                    let sample = |antialiasing| match shape {
                        0 => super::bandlimited_saw(phase, f64::from(step), antialiasing),
                        1 => super::bandlimited_pulse(phase, f64::from(step), 0.37, antialiasing),
                        _ => super::bandlimited_triangle(phase, f64::from(step), antialiasing),
                    };
                    let optimized = sample(super::Antialiasing::SplineOptimized);
                    let branchless = sample(super::Antialiasing::Spline);
                    let delta = (branchless - optimized) * mix;
                    if mix != previous_mix {
                        artifact_peak = artifact_peak.max((delta - previous_delta).abs());
                    }
                    previous_delta = delta;
                    previous_mix = mix;
                    phase = (phase + f64::from(step)).fract();
                    step += direction * 0.000_05;
                    if step >= 0.035 || step <= 0.015 {
                        direction = -direction;
                    }
                }
                println!(
                    "pitch_crossover_artifact,band={band:.3},shape={shape_name},rapid_sweep_delta_diff_peak={artifact_peak:.9}"
                );
            }
        }
    }

    #[test]
    #[ignore = "AVX-512 masked residual experiment"]
    fn avx512_masked_residual_report() {
        use std::hint::black_box;
        use std::time::Instant;

        if !std::arch::is_x86_feature_detected!("avx512f")
            || !std::arch::is_x86_feature_detected!("avx512vl")
        {
            println!("avx512_masked_residual,unsupported=true");
            return;
        }
        let calls = 5_000_000;
        let value = f32x8::splat(0.317);
        let started = Instant::now();
        for _ in 0..calls {
            black_box(value);
        }
        let baseline_call_ns = started.elapsed().as_nanos() as f64 / calls as f64;
        let started = Instant::now();
        for _ in 0..calls {
            black_box(unsafe { probe_avx512_call_lower_bound(value) });
        }
        let direct_call_ns = started.elapsed().as_nanos() as f64 / calls as f64;
        let started = Instant::now();
        for _ in 0..calls / 64 {
            if std::arch::is_x86_feature_detected!("avx512f")
                && std::arch::is_x86_feature_detected!("avx512vl")
            {
                for _ in 0..64 {
                    black_box(unsafe { probe_avx512_call_lower_bound(value) });
                }
            }
        }
        let block_dispatch_ns = started.elapsed().as_nanos() as f64 / (calls / 64 * 64) as f64;
        println!(
            "avx512_call_lower_bound,baseline_ns={baseline_call_ns:.3},direct_ns={direct_call_ns:.3},direct_overhead_ns={:.3},block_dispatch_ns={block_dispatch_ns:.3}",
            direct_call_ns - baseline_call_ns
        );
        for (range, base_step) in [("low", 0.0046_f32), ("mid", 0.041), ("high", 0.083)] {
            let step = f32x8::from(std::array::from_fn(|lane| {
                base_step * (1.0 + lane as f32 * 0.000_13)
            }));
            for coherent in [false, true] {
                let phases = if coherent {
                    [0.997; 8]
                } else {
                    [0.997, 0.121, 0.247, 0.373, 0.499, 0.625, 0.751, 0.877]
                };
                for shape in 0..4 {
                    let shape_name = ["saw", "square", "pulse37", "triangle"][shape];
                    let run = |avx512: bool, blocks: usize| {
                        let mut phase = f32x8::from(phases);
                        let mut output = [f32x8::ZERO; 64];
                        let active = step.cmp_gt(f32x8::splat(f32::EPSILON));
                        let support = step * f32x8::splat(2.0);
                        let inverse = f32x8::ONE / active.blend(step, f32x8::ONE);
                        let started = Instant::now();
                        for _ in 0..blocks {
                            for sample in &mut output {
                                let current = phase;
                                let next = phase + step;
                                phase = next.cmp_lt(f32x8::ONE).blend(next, next - f32x8::ONE);
                                *sample = if avx512 {
                                    if shape == 3 {
                                        unsafe { probe_avx512_triangle(current, step) }
                                    } else {
                                        let correction =
                                            unsafe { probe_avx512_blep(current, step, inverse) };
                                        if shape == 0 {
                                            current * f32x8::splat(2.0) - f32x8::ONE - correction
                                        } else {
                                            let width =
                                                f32x8::splat(if shape == 1 { 0.5 } else { 0.37 });
                                            let shifted =
                                                super::wrap_phase8(current + f32x8::ONE - width);
                                            current.cmp_lt(width).blend(f32x8::ONE, -f32x8::ONE)
                                                + correction
                                                - unsafe {
                                                    probe_avx512_blep(shifted, step, inverse)
                                                }
                                        }
                                    }
                                } else if shape == 3 {
                                    super::spline_triangle8_precomputed(
                                        current, step, active, support, inverse, true,
                                    )
                                } else {
                                    let correction = super::spline_blep8_precomputed(
                                        current, active, support, inverse, true,
                                    );
                                    if shape == 0 {
                                        current * f32x8::splat(2.0) - f32x8::ONE - correction
                                    } else {
                                        let width =
                                            f32x8::splat(if shape == 1 { 0.5 } else { 0.37 });
                                        let shifted =
                                            super::wrap_phase8(current + f32x8::ONE - width);
                                        current.cmp_lt(width).blend(f32x8::ONE, -f32x8::ONE)
                                            + correction
                                            - super::spline_blep8_precomputed(
                                                shifted, active, support, inverse, true,
                                            )
                                    }
                                };
                            }
                            black_box(&output);
                        }
                        (
                            started.elapsed().as_nanos() as f64 / (blocks * 64) as f64,
                            output,
                        )
                    };
                    let (_, current_output) = run(false, 1);
                    let (_, avx_output) = run(true, 1);
                    let mut square = 0.0_f64;
                    let mut peak = 0.0_f32;
                    for (a, b) in current_output.into_iter().zip(avx_output) {
                        for error in <[f32; 8]>::from(a - b) {
                            square += f64::from(error) * f64::from(error);
                            peak = peak.max(error.abs());
                        }
                    }
                    let current = (0..5)
                        .map(|_| run(false, 12_000).0)
                        .min_by(f64::total_cmp)
                        .unwrap();
                    let avx512 = (0..5)
                        .map(|_| run(true, 12_000).0)
                        .min_by(f64::total_cmp)
                        .unwrap();
                    println!(
                        "avx512_masked_residual,range={range},coherent={coherent},shape={shape_name},avx2_ns={current:.3},avx512_ns={avx512:.3},delta_pct={:.2},difference_rms={:.9},difference_peak={peak:.9}",
                        (avx512 / current - 1.0) * 100.0,
                        (square / 512.0).sqrt()
                    );
                }
            }
        }
    }
}
