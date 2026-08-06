//! Fast procedural virtual-analog oscillator.

use std::f64::consts::TAU;

use truce_simd::simd::{f32x4, f32x8};
use wide::{CmpGt, CmpLt};

use crate::wave_curve::WaveCurveRt;

const MAX_PRECOMPUTED_STEP_DRIFT: f32 = 1.0e-4;
// Smaller drift already stays below the sample-error bound without refinement.
const MAX_UNREFINED_STEP_DRIFT: f32 = 2.0e-5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Waveform {
    Saw,
    Pulse,
    Triangle,
    Sine,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PhaseWarpMode {
    #[default]
    None,
    Pwm,
    PhaseBend,
    Harmonic,
}

impl PhaseWarpMode {
    pub const fn from_index(index: u8) -> Self {
        match index {
            1 => Self::Pwm,
            2 => Self::PhaseBend,
            3 => Self::Harmonic,
            _ => Self::None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Antialiasing {
    #[default]
    Legacy,
    Spline,
    SplineOptimized,
    Lagrange,
    Spectral,
}

impl Antialiasing {
    pub const fn from_index(index: u8) -> Self {
        match index {
            0 => Self::Legacy,
            1 => Self::Spline,
            _ => Self::Lagrange,
        }
    }

    pub const fn for_factor(self, factor: u8) -> Self {
        if matches!(self, Self::Spline) && factor == 2 {
            Self::SplineOptimized
        } else {
            self
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct VaOscillator {
    phase: f32,
}

impl Default for VaOscillator {
    fn default() -> Self {
        Self { phase: 0.0 }
    }
}

impl VaOscillator {
    pub const fn reset(&mut self) {
        self.phase = 0.0;
    }

    #[allow(
        clippy::cast_possible_truncation,
        reason = "randomized phase enters the eight-wide f32 oscillator state"
    )]
    pub fn set_phase(&mut self, phase: f64) {
        self.phase = wrap01(phase) as f32;
    }

    #[allow(
        clippy::cast_possible_truncation,
        reason = "the plugin output sample type is f32"
    )]
    pub fn generate_shape_step(
        &mut self,
        shape: f32,
        phase_step: f32,
        pulse_width: f32,
        antialiasing: Antialiasing,
    ) -> f32 {
        let phase = self.phase;
        let next_phase = phase + phase_step;
        self.phase = if next_phase >= 1.0 {
            next_phase - 1.0
        } else {
            next_phase
        };
        sample_shape_normalized(
            shape,
            f64::from(phase),
            f64::from(phase_step),
            pulse_width,
            antialiasing,
        )
    }

    pub fn generate_shape_step_warped(
        &mut self,
        shape: f32,
        phase_step: f32,
        pulse_width: f32,
        antialiasing: Antialiasing,
        warp_mode: PhaseWarpMode,
        warp_amount: f32,
    ) -> f32 {
        let raw_phase = self.phase;
        let next_phase = raw_phase + phase_step;
        self.phase = if next_phase >= 1.0 {
            next_phase - 1.0
        } else {
            next_phase
        };
        let (phase, warped_step) = warp_phase_scalar(raw_phase, phase_step, warp_mode, warp_amount);
        sample_shape_normalized_warped(
            shape,
            f64::from(raw_phase),
            f64::from(phase_step),
            f64::from(phase),
            f64::from(warped_step),
            pulse_width,
            antialiasing,
        )
    }

    pub fn generate_custom_step(
        &mut self,
        shape: f32,
        phase_step: f32,
        pulse_width: f32,
        antialiasing: Antialiasing,
        warp_mode: PhaseWarpMode,
        warp_amount: f32,
        curve: WaveCurveRt,
        mix: f32,
    ) -> f32 {
        let raw_phase = self.phase;
        self.phase = wrap_phase_f32(raw_phase + phase_step);
        if mix >= 1.0 {
            curve.eval(warp_phase_position_scalar(
                raw_phase,
                phase_step,
                warp_mode,
                warp_amount,
            ))
        } else {
            let (phase, warped_step) =
                warp_phase_scalar(raw_phase, phase_step, warp_mode, warp_amount);
            let custom = curve.eval(phase);
            let canonical = sample_shape_normalized_warped(
                shape,
                f64::from(raw_phase),
                f64::from(phase_step),
                f64::from(phase),
                f64::from(warped_step),
                pulse_width,
                antialiasing,
            );
            (custom - canonical).mul_add(mix.clamp(0.0, 1.0), canonical)
        }
    }

    #[allow(
        clippy::cast_possible_truncation,
        reason = "the plugin output sample type is f32"
    )]
    pub fn generate_shape_step_pair(
        &mut self,
        shape: f32,
        phase_steps: [f32; 2],
        pulse_width: f32,
        antialiasing: Antialiasing,
    ) -> [f32; 2] {
        let phase0 = self.phase;
        let phase1 = wrap_phase_f32(phase0 + phase_steps[0]);
        self.phase = wrap_phase_f32(phase1 + phase_steps[1]);
        [
            sample_shape_normalized(
                shape,
                f64::from(phase0),
                f64::from(phase_steps[0]),
                pulse_width,
                antialiasing,
            ),
            sample_shape_normalized(
                shape,
                f64::from(phase1),
                f64::from(phase_steps[1]),
                pulse_width,
                antialiasing,
            ),
        ]
    }

    pub fn generate_shape_step_pair_warped(
        &mut self,
        shape: f32,
        phase_steps: [f32; 2],
        pulse_width: f32,
        antialiasing: Antialiasing,
        warp_mode: PhaseWarpMode,
        warp_amount: f32,
    ) -> [f32; 2] {
        let raw_phase0 = self.phase;
        let raw_phase1 = wrap_phase_f32(raw_phase0 + phase_steps[0]);
        self.phase = wrap_phase_f32(raw_phase1 + phase_steps[1]);
        let (phase0, step0) = warp_phase_scalar(raw_phase0, phase_steps[0], warp_mode, warp_amount);
        let (phase1, step1) = warp_phase_scalar(raw_phase1, phase_steps[1], warp_mode, warp_amount);
        [
            sample_shape_normalized_warped(
                shape,
                f64::from(raw_phase0),
                f64::from(phase_steps[0]),
                f64::from(phase0),
                f64::from(step0),
                pulse_width,
                antialiasing,
            ),
            sample_shape_normalized_warped(
                shape,
                f64::from(raw_phase1),
                f64::from(phase_steps[1]),
                f64::from(phase1),
                f64::from(step1),
                pulse_width,
                antialiasing,
            ),
        ]
    }
}

#[inline]
fn wrap_phase_f32(phase: f32) -> f32 {
    if phase >= 1.0 { phase - 1.0 } else { phase }
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
    sample_shape8_warped_at(
        raw_phases,
        raw_steps,
        phases,
        warped_steps,
        shape,
        pulse_width,
        antialiasing,
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
        let canonical = sample_shape8_warped_at(
            raw_phases,
            raw_steps,
            phases,
            steps,
            shape,
            pulse_width,
            antialiasing,
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

fn sample_shape8_warped_at(
    raw_phase: f32x8,
    raw_step: f32x8,
    phase: f32x8,
    phase_step: f32x8,
    shape: f32,
    pulse_width: f32,
    antialiasing: Antialiasing,
) -> f32x8 {
    // The warp changes sample position, not the time of the cycle reset. Keep the
    // BLEP centered on raw phase so its fractional discontinuity time stays exact.
    let shape = shape.clamp(0.0, 3.0);
    let (first, blend) = shape_segment(shape);
    if antialiasing == Antialiasing::Spectral
        || first == Waveform::Sine
        || first == Waveform::Triangle && blend <= f32::EPSILON
    {
        return sample_shape8_at(phase, phase_step, shape, pulse_width, antialiasing);
    }
    let sample = |waveform| match waveform {
        Waveform::Saw => {
            phase * f32x8::splat(2.0) - f32x8::ONE - edge_blep8(raw_phase, raw_step, antialiasing)
        }
        Waveform::Pulse => {
            let one = f32x8::ONE;
            let width = phase_step
                .fast_max(f32x8::splat(pulse_width.clamp(0.03, 0.97)))
                .fast_min(one - phase_step);
            let shifted = wrap_phase8(phase + one - width);
            phase.cmp_lt(width).blend(one, -one) + edge_blep8(raw_phase, raw_step, antialiasing)
                - edge_blep8(shifted, phase_step, antialiasing)
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
    debug_assert_ne!(antialiasing, Antialiasing::Spectral);
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
    debug_assert_ne!(antialiasing, Antialiasing::Spectral);
    let [raw_phases0, raw_phases1] = advance8_pair(oscillators, phase_steps);
    let raw_steps0 = f32x8::from(phase_steps[0]);
    let raw_steps1 = f32x8::from(phase_steps[1]);
    let (phases0, steps0) = warp_phase8(raw_phases0, raw_steps0, warp_mode, warp_amount);
    let (phases1, steps1) = warp_phase8(raw_phases1, raw_steps1, warp_mode, warp_amount);
    [
        sample_shape8_warped_at(
            raw_phases0,
            raw_steps0,
            phases0,
            steps0,
            shape,
            pulse_width,
            antialiasing,
        ),
        sample_shape8_warped_at(
            raw_phases1,
            raw_steps1,
            phases1,
            steps1,
            shape,
            pulse_width,
            antialiasing,
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
    debug_assert_ne!(antialiasing, Antialiasing::Spectral);
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
    debug_assert_ne!(antialiasing, Antialiasing::Spectral);
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

pub fn accumulate_saw8_block_constant<const SAMPLES: usize>(
    oscillators: &mut [VaOscillator],
    phase_step: f32x8,
    left_gain: f32x8,
    right_gain: f32x8,
    left: &mut [f32x8; SAMPLES],
    right: &mut [f32x8; SAMPLES],
    antialiasing: Antialiasing,
) {
    debug_assert!(oscillators.len() >= 8);
    debug_assert_ne!(antialiasing, Antialiasing::Spectral);
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
        for frame in 0..SAMPLES {
            let current = phase;
            let next = phase + phase_step;
            phase = next.cmp_lt(one).blend(next, next - one);
            let sample = current * f32x8::splat(2.0)
                - one
                - spline_blep8_precomputed(current, active, support, inverse_step, optimized);
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
        let sample = bandlimited_saw8(current, phase_step, antialiasing);
        left[frame] = sample.mul_add(left_gain, left[frame]);
        right[frame] = sample.mul_add(right_gain, right[frame]);
    }
    let wrapped: [f32; 8] = phase.into();
    for (oscillator, phase) in oscillators.iter_mut().zip(wrapped) {
        oscillator.phase = phase;
    }
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
    debug_assert_ne!(antialiasing, Antialiasing::Spectral);
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
    for frame in 0..SAMPLES {
        let current = phase;
        let next = phase + phase_step;
        phase = next.cmp_lt(f32x8::ONE).blend(next, next - f32x8::ONE);
        let sample = if mix >= 1.0 {
            curve.eval8(warp_phase_position8(
                current,
                phase_step,
                warp_mode,
                warp_amount,
            ))
        } else {
            let (warped_phase, warped_step) =
                warp_phase8(current, phase_step, warp_mode, warp_amount);
            let canonical = sample_shape8_warped_at(
                current,
                phase_step,
                warped_phase,
                warped_step,
                shape,
                pulse_width,
                antialiasing,
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
            let canonical = sample_shape8_warped_at(
                current,
                phase_steps[frame],
                warped_phase,
                warped_step,
                shape,
                pulse_width,
                antialiasing,
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
            let width = phase_step
                .fast_max(f32x4::splat(pulse_width.clamp(0.03, 0.97)))
                .fast_min(one - phase_step);
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
    debug_assert_ne!(antialiasing, Antialiasing::Spectral);
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
    debug_assert_ne!(antialiasing, Antialiasing::Spectral);
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
    debug_assert_ne!(antialiasing, Antialiasing::Spectral);
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
    debug_assert_ne!(antialiasing, Antialiasing::Spectral);
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
    debug_assert_ne!(antialiasing, Antialiasing::Spectral);
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
    debug_assert_ne!(antialiasing, Antialiasing::Spectral);
    let mut phase = f32x4::from(std::array::from_fn(|index| oscillators[index].phase));
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
    for frame in 0..SAMPLES {
        let current = phase;
        let next = phase + phase_step;
        phase = next.cmp_lt(f32x4::ONE).blend(next, next - f32x4::ONE);
        let sample = if mix >= 1.0 {
            curve.eval4(warp_phase_position4(
                current,
                phase_step,
                warp_mode,
                warp_amount,
            ))
        } else {
            let (warped_phase, warped_step) =
                warp_phase4(current, phase_step, warp_mode, warp_amount);
            let canonical = sample_shape4_warped_at(
                current,
                phase_step,
                warped_phase,
                warped_step,
                shape,
                pulse_width,
                antialiasing,
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
            let canonical = sample_shape4_warped_at(
                current,
                phase_steps[frame],
                warped_phase,
                warped_step,
                shape,
                pulse_width,
                antialiasing,
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
    debug_assert_ne!(antialiasing, Antialiasing::Spectral);
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
    debug_assert_ne!(antialiasing, Antialiasing::Spectral);
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
    sample_shape4_warped_at(
        raw_phases,
        raw_steps,
        phases,
        warped_steps,
        shape,
        pulse_width,
        antialiasing,
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
        let canonical = sample_shape4_warped_at(
            raw_phases,
            raw_steps,
            phases,
            steps,
            shape,
            pulse_width,
            antialiasing,
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

fn sample_shape4_warped_at(
    raw_phase: f32x4,
    raw_step: f32x4,
    phase: f32x4,
    phase_step: f32x4,
    shape: f32,
    pulse_width: f32,
    antialiasing: Antialiasing,
) -> f32x4 {
    // See the eight-lane path: cycle-reset timing belongs to the raw phase clock.
    let shape = shape.clamp(0.0, 3.0);
    let (first, blend) = shape_segment(shape);
    if antialiasing == Antialiasing::Spectral
        || first == Waveform::Sine
        || first == Waveform::Triangle && blend <= f32::EPSILON
    {
        return sample_shape4_at(phase, phase_step, shape, pulse_width, antialiasing);
    }
    let sample = |waveform| match waveform {
        Waveform::Saw => {
            phase * f32x4::splat(2.0) - f32x4::ONE - edge_blep4(raw_phase, raw_step, antialiasing)
        }
        Waveform::Pulse => {
            let one = f32x4::ONE;
            let width = phase_step
                .fast_max(f32x4::splat(pulse_width.clamp(0.03, 0.97)))
                .fast_min(one - phase_step);
            let shifted = wrap_phase4(phase + one - width);
            phase.cmp_lt(width).blend(one, -one) + edge_blep4(raw_phase, raw_step, antialiasing)
                - edge_blep4(shifted, phase_step, antialiasing)
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
    debug_assert_ne!(antialiasing, Antialiasing::Spectral);
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
    debug_assert_ne!(antialiasing, Antialiasing::Spectral);
    let [phases0, phases1] = advance4_pair(oscillators, phase_steps);
    let (phases0, steps0) =
        warp_phase4(phases0, f32x4::from(phase_steps[0]), warp_mode, warp_amount);
    let (phases1, steps1) =
        warp_phase4(phases1, f32x4::from(phase_steps[1]), warp_mode, warp_amount);
    [
        sample_shape4_at(phases0, steps0, shape, pulse_width, antialiasing),
        sample_shape4_at(phases1, steps1, shape, pulse_width, antialiasing),
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
fn warp_phase_scalar(phase: f32, phase_step: f32, mode: PhaseWarpMode, amount: f32) -> (f32, f32) {
    let amount = amount.clamp(0.0, 1.0);
    if mode == PhaseWarpMode::None || amount <= f32::EPSILON {
        return (phase, phase_step);
    }
    match mode {
        PhaseWarpMode::None => (phase, phase_step),
        PhaseWarpMode::Pwm => {
            let depth = (amount * 0.95).min((0.45 / phase_step.max(f32::EPSILON) - 1.0).max(0.0));
            const PWM_NORMALIZATION: f32 = 0.058_174_6;
            let angle = std::f32::consts::TAU * phase;
            let displacement = (angle.cos() - (2.0 * angle).cos()) * PWM_NORMALIZATION;
            let derivative = (-std::f32::consts::TAU * angle.sin()
                + 2.0 * std::f32::consts::TAU * (2.0 * angle).sin())
                * PWM_NORMALIZATION;
            (
                phase - depth * displacement,
                phase_step * (1.0 - depth * derivative),
            )
        }
        PhaseWarpMode::PhaseBend => {
            let depth = (amount * 0.95).min((0.45 / phase_step.max(f32::EPSILON) - 1.0).max(0.0));
            let angle = 2.0 * std::f32::consts::TAU * phase;
            let displacement = angle.sin() / (2.0 * std::f32::consts::TAU);
            let derivative = angle.cos();
            (
                phase - depth * displacement,
                phase_step * (1.0 - depth * derivative),
            )
        }
        PhaseWarpMode::Harmonic => {
            let depth = (amount * 0.95).min((0.45 / phase_step.max(f32::EPSILON) - 1.0).max(0.0));
            let angle = std::f32::consts::TAU * phase;
            (
                phase - depth * angle.sin() / std::f32::consts::TAU,
                phase_step * (1.0 - depth * angle.cos()),
            )
        }
    }
}

#[inline]
fn warp_phase_position_scalar(
    phase: f32,
    phase_step: f32,
    mode: PhaseWarpMode,
    amount: f32,
) -> f32 {
    let amount = amount.clamp(0.0, 1.0);
    if mode == PhaseWarpMode::None || amount <= f32::EPSILON {
        return phase;
    }
    let depth = (amount * 0.95).min((0.45 / phase_step.max(f32::EPSILON) - 1.0).max(0.0));
    match mode {
        PhaseWarpMode::None => phase,
        PhaseWarpMode::Pwm => {
            const NORMALIZATION: f32 = 0.058_174_6;
            let angle = std::f32::consts::TAU * phase;
            phase - depth * (angle.cos() - (2.0 * angle).cos()) * NORMALIZATION
        }
        PhaseWarpMode::PhaseBend => {
            phase
                - depth * (2.0 * std::f32::consts::TAU * phase).sin()
                    / (2.0 * std::f32::consts::TAU)
        }
        PhaseWarpMode::Harmonic => {
            phase - depth * (std::f32::consts::TAU * phase).sin() / std::f32::consts::TAU
        }
    }
}

#[inline]
fn warp_phase4(
    phase: f32x4,
    phase_step: f32x4,
    mode: PhaseWarpMode,
    amount: f32,
) -> (f32x4, f32x4) {
    let amount = amount.clamp(0.0, 1.0);
    if mode == PhaseWarpMode::None || amount <= f32::EPSILON {
        return (phase, phase_step);
    }
    match mode {
        PhaseWarpMode::None => (phase, phase_step),
        PhaseWarpMode::Pwm => {
            let depth = f32x4::splat(amount * 0.95).fast_min(
                (f32x4::splat(0.45) / phase_step.fast_max(f32x4::splat(f32::EPSILON)) - f32x4::ONE)
                    .fast_max(f32x4::ZERO),
            );
            let normalization = f32x4::splat(0.058_174_6);
            let second_phase = wrap_phase4(phase * f32x4::splat(2.0));
            let sine = sine_phase4(phase);
            let second_sine = sine_phase4(second_phase);
            let displacement = (cosine_phase4(phase) - cosine_phase4(second_phase)) * normalization;
            let derivative = (second_sine * f32x4::splat(2.0) - sine)
                * f32x4::splat(std::f32::consts::TAU)
                * normalization;
            (
                phase - depth * displacement,
                phase_step * (f32x4::ONE - depth * derivative),
            )
        }
        PhaseWarpMode::PhaseBend => {
            let depth = f32x4::splat(amount * 0.95).fast_min(
                (f32x4::splat(0.45) / phase_step.fast_max(f32x4::splat(f32::EPSILON)) - f32x4::ONE)
                    .fast_max(f32x4::ZERO),
            );
            let second_phase = wrap_phase4(phase * f32x4::splat(2.0));
            let displacement =
                sine_phase4(second_phase) * f32x4::splat((2.0 * std::f32::consts::TAU).recip());
            let derivative = cosine_phase4(second_phase);
            (
                phase - depth * displacement,
                phase_step * (f32x4::ONE - depth * derivative),
            )
        }
        PhaseWarpMode::Harmonic => {
            let depth = f32x4::splat(amount * 0.95).fast_min(
                (f32x4::splat(0.45) / phase_step.fast_max(f32x4::splat(f32::EPSILON)) - f32x4::ONE)
                    .fast_max(f32x4::ZERO),
            );
            let sine = sine_phase4(phase);
            let cosine = cosine_phase4(phase);
            (
                phase - depth * sine * f32x4::splat(std::f32::consts::TAU.recip()),
                phase_step * (f32x4::ONE - depth * cosine),
            )
        }
    }
}

#[inline]
fn warp_phase_position4(
    phase: f32x4,
    phase_step: f32x4,
    mode: PhaseWarpMode,
    amount: f32,
) -> f32x4 {
    let amount = amount.clamp(0.0, 1.0);
    if mode == PhaseWarpMode::None || amount <= f32::EPSILON {
        return phase;
    }
    let depth = f32x4::splat(amount * 0.95).fast_min(
        (f32x4::splat(0.45) / phase_step.fast_max(f32x4::splat(f32::EPSILON)) - f32x4::ONE)
            .fast_max(f32x4::ZERO),
    );
    match mode {
        PhaseWarpMode::None => phase,
        PhaseWarpMode::Pwm => {
            let second_phase = wrap_phase4(phase * f32x4::splat(2.0));
            phase
                - depth
                    * (cosine_phase4(phase) - cosine_phase4(second_phase))
                    * f32x4::splat(0.058_174_6)
        }
        PhaseWarpMode::PhaseBend => {
            let second_phase = wrap_phase4(phase * f32x4::splat(2.0));
            phase
                - depth
                    * sine_phase4(second_phase)
                    * f32x4::splat((2.0 * std::f32::consts::TAU).recip())
        }
        PhaseWarpMode::Harmonic => {
            phase - depth * sine_phase4(phase) * f32x4::splat(std::f32::consts::TAU.recip())
        }
    }
}

#[inline]
fn warp_phase8(
    phase: f32x8,
    phase_step: f32x8,
    mode: PhaseWarpMode,
    amount: f32,
) -> (f32x8, f32x8) {
    let amount = amount.clamp(0.0, 1.0);
    if mode == PhaseWarpMode::None || amount <= f32::EPSILON {
        return (phase, phase_step);
    }
    match mode {
        PhaseWarpMode::None => (phase, phase_step),
        PhaseWarpMode::Pwm => {
            let depth = f32x8::splat(amount * 0.95).fast_min(
                (f32x8::splat(0.45) / phase_step.fast_max(f32x8::splat(f32::EPSILON)) - f32x8::ONE)
                    .fast_max(f32x8::ZERO),
            );
            let normalization = f32x8::splat(0.058_174_6);
            let second_phase = wrap_phase8(phase * f32x8::splat(2.0));
            let sine = sine_phase8(phase);
            let second_sine = sine_phase8(second_phase);
            let displacement = (cosine_phase8(phase) - cosine_phase8(second_phase)) * normalization;
            let derivative = (second_sine * f32x8::splat(2.0) - sine)
                * f32x8::splat(std::f32::consts::TAU)
                * normalization;
            (
                phase - depth * displacement,
                phase_step * (f32x8::ONE - depth * derivative),
            )
        }
        PhaseWarpMode::PhaseBend => {
            let depth = f32x8::splat(amount * 0.95).fast_min(
                (f32x8::splat(0.45) / phase_step.fast_max(f32x8::splat(f32::EPSILON)) - f32x8::ONE)
                    .fast_max(f32x8::ZERO),
            );
            let second_phase = wrap_phase8(phase * f32x8::splat(2.0));
            let displacement =
                sine_phase8(second_phase) * f32x8::splat((2.0 * std::f32::consts::TAU).recip());
            let derivative = cosine_phase8(second_phase);
            (
                phase - depth * displacement,
                phase_step * (f32x8::ONE - depth * derivative),
            )
        }
        PhaseWarpMode::Harmonic => {
            let depth = f32x8::splat(amount * 0.95).fast_min(
                (f32x8::splat(0.45) / phase_step.fast_max(f32x8::splat(f32::EPSILON)) - f32x8::ONE)
                    .fast_max(f32x8::ZERO),
            );
            let sine = sine_phase8(phase);
            let cosine = cosine_phase8(phase);
            (
                phase - depth * sine * f32x8::splat(std::f32::consts::TAU.recip()),
                phase_step * (f32x8::ONE - depth * cosine),
            )
        }
    }
}

#[inline]
fn warp_phase_position8(
    phase: f32x8,
    phase_step: f32x8,
    mode: PhaseWarpMode,
    amount: f32,
) -> f32x8 {
    let amount = amount.clamp(0.0, 1.0);
    if mode == PhaseWarpMode::None || amount <= f32::EPSILON {
        return phase;
    }
    let depth = f32x8::splat(amount * 0.95).fast_min(
        (f32x8::splat(0.45) / phase_step.fast_max(f32x8::splat(f32::EPSILON)) - f32x8::ONE)
            .fast_max(f32x8::ZERO),
    );
    match mode {
        PhaseWarpMode::None => phase,
        PhaseWarpMode::Pwm => {
            let second_phase = wrap_phase8(phase * f32x8::splat(2.0));
            phase
                - depth
                    * (cosine_phase8(phase) - cosine_phase8(second_phase))
                    * f32x8::splat(0.058_174_6)
        }
        PhaseWarpMode::PhaseBend => {
            let second_phase = wrap_phase8(phase * f32x8::splat(2.0));
            phase
                - depth
                    * sine_phase8(second_phase)
                    * f32x8::splat((2.0 * std::f32::consts::TAU).recip())
        }
        PhaseWarpMode::Harmonic => {
            phase - depth * sine_phase8(phase) * f32x8::splat(std::f32::consts::TAU.recip())
        }
    }
}

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
pub fn sample_shape_with_antialiasing(
    shape: f32,
    phase: f64,
    phase_step: f64,
    pulse_width: f32,
    antialiasing: Antialiasing,
) -> f32 {
    sample_shape_normalized(shape, wrap01(phase), phase_step, pulse_width, antialiasing)
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "the editor preview mirrors the f32 realtime phase-warp path"
)]
pub fn sample_shape_with_antialiasing_warped(
    shape: f32,
    phase: f64,
    phase_step: f64,
    pulse_width: f32,
    antialiasing: Antialiasing,
    warp_mode: PhaseWarpMode,
    warp_amount: f32,
) -> f32 {
    let raw_phase = wrap01(phase) as f32;
    let raw_step = phase_step as f32;
    let (phase, phase_step) = warp_phase_scalar(raw_phase, raw_step, warp_mode, warp_amount);
    sample_shape_normalized_warped(
        shape,
        f64::from(raw_phase),
        f64::from(raw_step),
        f64::from(phase),
        f64::from(phase_step),
        pulse_width,
        antialiasing,
    )
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
    let canonical = sample_shape_normalized_warped(
        shape,
        f64::from(raw_phase),
        f64::from(raw_step),
        f64::from(phase),
        f64::from(phase_step),
        pulse_width,
        antialiasing,
    );
    (curve.eval(phase) - canonical).mul_add(mix.clamp(0.0, 1.0), canonical)
}

fn sample_shape_normalized(
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

fn sample_shape_normalized_warped(
    shape: f32,
    raw_phase: f64,
    raw_step: f64,
    phase: f64,
    phase_step: f64,
    pulse_width: f32,
    antialiasing: Antialiasing,
) -> f32 {
    // See the SIMD paths: phase warp does not move the raw cycle boundary.
    let shape = shape.clamp(0.0, 3.0);
    let (first, blend) = shape_segment(shape);
    if antialiasing == Antialiasing::Spectral
        || first == Waveform::Sine
        || first == Waveform::Triangle && blend <= f32::EPSILON
    {
        return sample_shape_normalized(shape, phase, phase_step, pulse_width, antialiasing);
    }
    let sample = |waveform| match waveform {
        Waveform::Saw => {
            (2.0_f64.mul_add(phase, -1.0) - edge_blep(raw_phase, raw_step, antialiasing)) as f32
        }
        Waveform::Pulse => {
            let minimum_width = phase_step.max(0.03);
            let width = f64::from(pulse_width).clamp(minimum_width, 1.0 - minimum_width);
            let shifted = wrap01(phase + 1.0 - width);
            let sample = if phase < width { 1.0 } else { -1.0 };
            (sample + edge_blep(raw_phase, raw_step, antialiasing)
                - edge_blep(shifted, phase_step, antialiasing)) as f32
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
    let sample = match waveform {
        Waveform::Saw => bandlimited_saw(phase, phase_step, antialiasing),
        Waveform::Pulse => {
            bandlimited_pulse(phase, phase_step, f64::from(pulse_width), antialiasing)
        }
        Waveform::Triangle => bandlimited_triangle(phase, phase_step, antialiasing),
        Waveform::Sine => -(TAU * phase).cos(),
    };
    sample as f32
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

#[inline]
fn wrap_phase4(phase: f32x4) -> f32x4 {
    phase.cmp_lt(f32x4::ONE).blend(phase, phase - f32x4::ONE)
}

#[inline]
fn wrap_phase8(phase: f32x8) -> f32x8 {
    phase.cmp_lt(f32x8::ONE).blend(phase, phase - f32x8::ONE)
}

#[inline]
fn cosine_phase4(phase: f32x4) -> f32x4 {
    sine_phase4(wrap_phase4(phase + f32x4::splat(0.25)))
}

#[inline]
fn cosine_phase8(phase: f32x8) -> f32x8 {
    sine_phase8(wrap_phase8(phase + f32x8::splat(0.25)))
}

#[inline]
fn aligned_sine_phase4(phase: f32x4) -> f32x4 {
    -cosine_phase4(phase)
}

#[inline]
fn aligned_sine_phase8(phase: f32x8) -> f32x8 {
    -cosine_phase8(phase)
}

fn sine_phase4(phase: f32x4) -> f32x4 {
    let half = f32x4::splat(0.5);
    let quarter = f32x4::splat(0.25);
    let folded = quarter - ((phase - half).abs() - quarter).abs();
    let folded2 = folded * folded;
    let folded4 = folded2 * folded2;
    let low = f32x4::splat(-41.341_7).mul_add(folded2, f32x4::splat(std::f32::consts::TAU));
    let middle = f32x4::splat(-76.705_86).mul_add(folded2, f32x4::splat(81.605_25));
    let high = f32x4::splat(-15.094_643).mul_add(folded2, f32x4::splat(42.058_693));
    let polynomial = high.mul_add(folded4, middle).mul_add(folded4, low);
    let sine = folded * polynomial;
    phase.cmp_gt(half).blend(-sine, sine)
}

fn sine_phase8(phase: f32x8) -> f32x8 {
    let half = f32x8::splat(0.5);
    let quarter = f32x8::splat(0.25);
    let folded = quarter - ((phase - half).abs() - quarter).abs();
    let folded2 = folded * folded;
    let folded4 = folded2 * folded2;
    let low = f32x8::splat(-41.341_7).mul_add(folded2, f32x8::splat(std::f32::consts::TAU));
    let middle = f32x8::splat(-76.705_86).mul_add(folded2, f32x8::splat(81.605_25));
    let high = f32x8::splat(-15.094_643).mul_add(folded2, f32x8::splat(42.058_693));
    let polynomial = high.mul_add(folded4, middle).mul_add(folded4, low);
    let sine = folded * polynomial;
    phase.cmp_gt(half).blend(-sine, sine)
}

fn bandlimited_triangle(phase: f64, phase_step: f64, antialiasing: Antialiasing) -> f64 {
    if antialiasing == Antialiasing::Spectral {
        return f64::from(spectral_triangle(phase as f32, phase_step as f32));
    }
    let sample = (-4.0_f64).mul_add((phase - 0.5).abs(), 1.0);
    if antialiasing == Antialiasing::Legacy {
        return sample;
    }
    let peak_phase = wrap01(phase + 0.5);
    let correction = match antialiasing {
        Antialiasing::Legacy => unreachable!(),
        Antialiasing::Spline => {
            spline_blamp(phase, phase_step, false) - spline_blamp(peak_phase, phase_step, false)
        }
        Antialiasing::SplineOptimized => {
            spline_blamp(phase, phase_step, true) - spline_blamp(peak_phase, phase_step, true)
        }
        Antialiasing::Lagrange => {
            lagrange_blamp(phase, phase_step) - lagrange_blamp(peak_phase, phase_step)
        }
        Antialiasing::Spectral => {
            spline_blamp(phase, phase_step, true) - spline_blamp(peak_phase, phase_step, true)
        }
    };
    (8.0 * phase_step).mul_add(correction, sample)
}

fn bandlimited_triangle4(phase: f32x4, phase_step: f32x4, antialiasing: Antialiasing) -> f32x4 {
    if antialiasing == Antialiasing::Spectral {
        return spectral_triangle4(phase, phase_step);
    }
    let half = f32x4::splat(0.5);
    let sample = (phase - half).abs() * f32x4::splat(-4.0) + f32x4::ONE;
    if antialiasing == Antialiasing::Legacy {
        return sample;
    }
    let shifted = phase + half;
    let peak_phase = shifted
        .cmp_lt(f32x4::ONE)
        .blend(shifted, shifted - f32x4::ONE);
    let correction = match antialiasing {
        Antialiasing::Legacy => unreachable!(),
        Antialiasing::Spline => {
            spline_blamp4(phase, phase_step, false) - spline_blamp4(peak_phase, phase_step, false)
        }
        Antialiasing::SplineOptimized => {
            spline_blamp4(phase, phase_step, true) - spline_blamp4(peak_phase, phase_step, true)
        }
        Antialiasing::Lagrange => {
            lagrange_blamp4(phase, phase_step) - lagrange_blamp4(peak_phase, phase_step)
        }
        Antialiasing::Spectral => {
            spline_blamp4(phase, phase_step, true) - spline_blamp4(peak_phase, phase_step, true)
        }
    };
    (phase_step * f32x4::splat(8.0)).mul_add(correction, sample)
}

fn bandlimited_triangle8(phase: f32x8, phase_step: f32x8, antialiasing: Antialiasing) -> f32x8 {
    if antialiasing == Antialiasing::Spectral {
        return spectral_triangle8(phase, phase_step);
    }
    let half = f32x8::splat(0.5);
    let sample = (phase - half).abs() * f32x8::splat(-4.0) + f32x8::ONE;
    if antialiasing == Antialiasing::Legacy {
        return sample;
    }
    let shifted = phase + half;
    let peak_phase = shifted
        .cmp_lt(f32x8::ONE)
        .blend(shifted, shifted - f32x8::ONE);
    let correction = match antialiasing {
        Antialiasing::Legacy => unreachable!(),
        Antialiasing::Spline => {
            spline_blamp8(phase, phase_step, false) - spline_blamp8(peak_phase, phase_step, false)
        }
        Antialiasing::SplineOptimized => {
            spline_blamp8(phase, phase_step, true) - spline_blamp8(peak_phase, phase_step, true)
        }
        Antialiasing::Lagrange => {
            lagrange_blamp8(phase, phase_step) - lagrange_blamp8(peak_phase, phase_step)
        }
        Antialiasing::Spectral => {
            spline_blamp8(phase, phase_step, true) - spline_blamp8(peak_phase, phase_step, true)
        }
    };
    (phase_step * f32x8::splat(8.0)).mul_add(correction, sample)
}

const SPECTRAL_TABLE_SIZE: usize = 4096;
const SPECTRAL_TABLE_STRIDE: usize = SPECTRAL_TABLE_SIZE;
const SPECTRAL_MAX_HARMONICS: usize = 128;
const SPECTRAL_EXACT_HARMONICS: usize = 128;
const SPECTRAL_SAW_ROWS: usize = SPECTRAL_MAX_HARMONICS + 1;
const SPECTRAL_TRIANGLE_ROWS: usize = 129;
#[repr(align(4096))]
struct AlignedSpectralSaw([u8; SPECTRAL_SAW_ROWS * SPECTRAL_TABLE_STRIDE * 4]);

#[repr(align(4096))]
struct AlignedSpectralTriangle([u8; SPECTRAL_TRIANGLE_ROWS * SPECTRAL_TABLE_STRIDE * 4]);

static SPECTRAL_SAW: AlignedSpectralSaw =
    AlignedSpectralSaw(*include_bytes!("spectral-saw-f32le.bin"));
static SPECTRAL_TRIANGLE: AlignedSpectralTriangle =
    AlignedSpectralTriangle(*include_bytes!("spectral-triangle-f32le.bin"));

const SPECTRAL_TRANSITION_SAMPLES: u8 = 128;

struct SpectralRows8 {
    current: [i32; 8],
    target: [i32; 8],
    mix: f32x8,
    transitioning: bool,
}

pub fn generate_spectral_shape8(
    oscillators: &mut [VaOscillator],
    current: &mut [u16],
    target: &mut [u16],
    remaining: &mut [u8],
    shape: f32,
    phase_steps: [f32; 8],
    pulse_width: f32,
    check_harmonics: bool,
) -> f32x8 {
    debug_assert!(oscillators.len() >= 8);
    debug_assert!(current.len() >= 8 && target.len() >= 8 && remaining.len() >= 8);
    let phases = advance8(oscillators, phase_steps);
    let spectral_minimum_step = (0.5 - f32::EPSILON) / (SPECTRAL_MAX_HARMONICS as f32 + 1.0);
    if phase_steps.iter().all(|step| *step < spectral_minimum_step) {
        current[..8].fill(0);
        target[..8].fill(0);
        remaining[..8].fill(0);
        return spectral_low_fallback8(phases, f32x8::from(phase_steps), shape, pulse_width);
    }
    if !check_harmonics && current[0] != 0 && remaining[..8] == [0; 8] {
        return spectral_cached_shape8(phases, current, shape, pulse_width);
    }
    let rows = spectral_rows8(current, target, remaining, phase_steps, check_harmonics);
    let shape = shape.clamp(0.0, 3.0);
    let (first, blend) = shape_segment(shape);
    if blend <= f32::EPSILON {
        return spectral_waveform8(first, phases, pulse_width, &rows);
    }
    let a = spectral_waveform8(first, phases, pulse_width, &rows);
    let b = spectral_waveform8(next_waveform(first), phases, pulse_width, &rows);
    (b - a).mul_add(f32x8::splat(blend), a) * f32x8::splat(morph_gain(first, blend))
}

#[inline(always)]
fn spectral_cached_shape8(phase: f32x8, rows: &[u16], shape: f32, pulse_width: f32) -> f32x8 {
    let shape = shape.clamp(0.0, 3.0);
    if (shape - 1.0).abs() <= f32::EPSILON {
        return spectral_lookup_u16_rows8(&SPECTRAL_TRIANGLE.0, phase, rows);
    }
    if (shape - 2.0).abs() <= f32::EPSILON {
        return spectral_lookup_saw_u16_rows8(phase, rows);
    }
    let width = pulse_width.clamp(0.03, 0.97);
    let shifted = phase + f32x8::splat(1.0 - width);
    let shifted = shifted
        .cmp_lt(f32x8::ONE)
        .blend(shifted, shifted - f32x8::ONE);
    if shape >= 3.0 - f32::EPSILON {
        return spectral_lookup_saw_u16_rows8(shifted, rows)
            - spectral_lookup_saw_u16_rows8(phase, rows)
            + f32x8::splat(width.mul_add(2.0, -1.0));
    }
    if shape > 2.0 {
        let blend = shape - 2.0;
        let shifted = spectral_lookup_saw_u16_rows8(shifted, rows);
        let dc = f32x8::splat(blend * width.mul_add(2.0, -1.0));
        if (blend - 0.5).abs() <= f32::EPSILON {
            return shifted.mul_add(f32x8::splat(blend), dc);
        }
        return spectral_lookup_saw_u16_rows8(phase, rows).mul_add(
            f32x8::splat(1.0 - 2.0 * blend),
            shifted * f32x8::splat(blend),
        ) + dc;
    }
    let (first, blend) = shape_segment(shape);
    let a = match first {
        Waveform::Sine => aligned_sine_phase8(phase),
        Waveform::Triangle => spectral_lookup_u16_rows8(&SPECTRAL_TRIANGLE.0, phase, rows),
        Waveform::Saw => spectral_lookup_saw_u16_rows8(phase, rows),
        Waveform::Pulse => unreachable!(),
    };
    if blend <= f32::EPSILON {
        return a;
    }
    let b = match next_waveform(first) {
        Waveform::Triangle => spectral_lookup_u16_rows8(&SPECTRAL_TRIANGLE.0, phase, rows),
        Waveform::Saw => spectral_lookup_saw_u16_rows8(phase, rows),
        Waveform::Sine | Waveform::Pulse => unreachable!(),
    };
    (b - a).mul_add(f32x8::splat(blend), a) * f32x8::splat(morph_gain(first, blend))
}

#[inline(never)]
pub fn generate_spectral_saw8(
    oscillators: &mut [VaOscillator],
    current: &mut [u16],
    target: &mut [u16],
    remaining: &mut [u8],
    top_gain: &mut [f32],
    phase_steps: [f32; 8],
    check_harmonics: bool,
) -> f32x8 {
    debug_assert!(oscillators.len() >= 8);
    let phases = advance8(oscillators, phase_steps);
    let spectral_minimum_step = (0.5 - f32::EPSILON) / (SPECTRAL_MAX_HARMONICS as f32 + 1.0);
    if phase_steps.iter().all(|step| *step < spectral_minimum_step) {
        current[..8].fill(0);
        target[..8].fill(0);
        remaining[..8].fill(0);
        top_gain[..8].fill(1.0);
        return bandlimited_saw8(
            phases,
            f32x8::from(phase_steps),
            Antialiasing::SplineOptimized,
        );
    }
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    {
        let (direct, rows, gains) = if check_harmonics || current[0] == 0 {
            let (direct, desired, clearance) = spectral_saw8_direct_avx2(phases, phase_steps);
            let gains = clearance.map(|value| {
                let position = (value * 16.0).clamp(0.0, 1.0);
                position * position * (-2.0f32).mul_add(position, 3.0)
            });
            for lane in 0..8 {
                current[lane] = desired[lane] as u16;
                target[lane] = current[lane];
                remaining[lane] = 0;
                top_gain[lane] = gains[lane];
            }
            (direct, desired, gains)
        } else {
            (
                spectral_lookup_saw_u16_rows8(phases, current),
                std::array::from_fn(|lane| i32::from(current[lane])),
                std::array::from_fn(|lane| top_gain[lane]),
            )
        };
        if gains.iter().all(|gain| *gain >= 1.0) {
            return direct;
        }
        let top = spectral_saw_top_harmonic8(phases, rows);
        return (f32x8::ONE - f32x8::from(gains)).mul_add(-top, direct);
    }
    #[cfg(not(all(target_arch = "x86_64", target_feature = "avx2")))]
    {
        let rows = spectral_rows8(current, target, remaining, phase_steps, check_harmonics);
        spectral_lookup_saw8(phases, &rows)
    }
}

#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
#[inline(never)]
fn spectral_saw8_direct_avx2(phase: f32x8, phase_steps: [f32; 8]) -> (f32x8, [i32; 8], [f32; 8]) {
    use core::arch::x86_64::{
        _mm256_add_epi32, _mm256_and_si256, _mm256_cvtepi32_ps, _mm256_cvttps_epi32, _mm256_div_ps,
        _mm256_fmadd_ps, _mm256_i32gather_ps, _mm256_loadu_ps, _mm256_max_epi32, _mm256_min_epi32,
        _mm256_mul_ps, _mm256_mullo_epi32, _mm256_set1_epi32, _mm256_set1_ps, _mm256_storeu_ps,
        _mm256_storeu_si256, _mm256_sub_ps,
    };

    let phase: [f32; 8] = phase.into();
    let mut output = [0.0; 8];
    let mut desired = [0; 8];
    let mut clearance_output = [0.0; 8];
    // SAFETY: phase and step arrays contain eight initialized values. Harmonic rows are
    // clamped to the immutable bank and phase indices are masked to its power-of-two row.
    unsafe {
        let phase = _mm256_loadu_ps(phase.as_ptr());
        let steps = _mm256_loadu_ps(phase_steps.as_ptr());
        let position = _mm256_mul_ps(phase, _mm256_set1_ps(SPECTRAL_TABLE_SIZE as f32));
        let index = _mm256_cvttps_epi32(position);
        let fraction = _mm256_sub_ps(position, _mm256_cvtepi32_ps(index));
        let limit = _mm256_div_ps(_mm256_set1_ps(0.5 - f32::EPSILON), steps);
        let rows = _mm256_cvttps_epi32(limit);
        let rows = _mm256_max_epi32(
            _mm256_set1_epi32(1),
            _mm256_min_epi32(_mm256_set1_epi32(SPECTRAL_MAX_HARMONICS as i32), rows),
        );
        _mm256_storeu_si256(desired.as_mut_ptr().cast(), rows);
        _mm256_storeu_ps(
            clearance_output.as_mut_ptr(),
            _mm256_sub_ps(limit, _mm256_cvtepi32_ps(rows)),
        );
        let table = _mm256_mullo_epi32(rows, _mm256_set1_epi32(SPECTRAL_TABLE_STRIDE as i32));
        let absolute = _mm256_add_epi32(index, table);
        let next_index = _mm256_and_si256(
            _mm256_add_epi32(index, _mm256_set1_epi32(1)),
            _mm256_set1_epi32((SPECTRAL_TABLE_SIZE - 1) as i32),
        );
        let next_absolute = _mm256_add_epi32(next_index, table);
        let base = SPECTRAL_SAW.0.as_ptr().cast::<f32>();
        let first = _mm256_i32gather_ps(base, absolute, 4);
        let second = _mm256_i32gather_ps(base, next_absolute, 4);
        let result = _mm256_fmadd_ps(_mm256_sub_ps(second, first), fraction, first);
        _mm256_storeu_ps(output.as_mut_ptr(), result);
    }
    (f32x8::from(output), desired, clearance_output)
}

fn spectral_low_fallback8(
    phases: f32x8,
    phase_steps: f32x8,
    shape: f32,
    pulse_width: f32,
) -> f32x8 {
    let shape = shape.clamp(0.0, 3.0);
    let (first, blend) = shape_segment(shape);
    if blend > f32::EPSILON && first == Waveform::Saw {
        return bandlimited_saw_pulse_morph8(
            phases,
            phase_steps,
            pulse_width,
            blend,
            Antialiasing::SplineOptimized,
        );
    }
    let a = sample_waveform8(
        first,
        phases,
        phase_steps,
        pulse_width,
        Antialiasing::SplineOptimized,
    );
    if blend <= f32::EPSILON {
        a
    } else {
        let b = sample_waveform8(
            next_waveform(first),
            phases,
            phase_steps,
            pulse_width,
            Antialiasing::SplineOptimized,
        );
        (b - a).mul_add(f32x8::splat(blend), a)
    }
}

fn spectral_rows8(
    current: &mut [u16],
    target: &mut [u16],
    remaining: &mut [u8],
    phase_steps: [f32; 8],
    check_harmonics: bool,
) -> SpectralRows8 {
    if !check_harmonics && current[0] != 0 && remaining[..8] == [0; 8] {
        let rows = std::array::from_fn(|lane| i32::from(current[lane]));
        return SpectralRows8 {
            current: rows,
            target: rows,
            mix: f32x8::ZERO,
            transitioning: false,
        };
    }
    if !check_harmonics && current[0] != 0 {
        let desired = std::array::from_fn(|lane| i32::from(target[lane]));
        return spectral_rows8_transition(current, target, remaining, desired);
    }
    let desired = spectral_harmonics8(phase_steps);
    spectral_rows8_desired(current, target, remaining, desired)
}

#[inline]
fn spectral_rows8_desired(
    current: &mut [u16],
    target: &mut [u16],
    remaining: &mut [u8],
    desired: [i32; 8],
) -> SpectralRows8 {
    let steady =
        (0..8).all(|lane| remaining[lane] == 0 && i32::from(current[lane]) == desired[lane]);
    if steady {
        return SpectralRows8 {
            current: desired,
            target: desired,
            mix: f32x8::ZERO,
            transitioning: false,
        };
    }
    spectral_rows8_transition(current, target, remaining, desired)
}

#[cold]
#[inline(never)]
fn spectral_rows8_transition(
    current: &mut [u16],
    target: &mut [u16],
    remaining: &mut [u8],
    desired: [i32; 8],
) -> SpectralRows8 {
    let mut current_rows = [0; 8];
    let mut target_rows = [0; 8];
    let mut mixes = [0.0; 8];
    let mut transitioning = false;
    for lane in 0..8 {
        let desired = desired[lane] as u16;
        if current[lane] == 0 {
            current[lane] = desired;
            target[lane] = desired;
        } else if remaining[lane] == 0 && desired != current[lane] {
            target[lane] = desired;
            remaining[lane] = SPECTRAL_TRANSITION_SAMPLES;
        }
        if remaining[lane] != 0 {
            transitioning = true;
            mixes[lane] = f32::from(SPECTRAL_TRANSITION_SAMPLES - remaining[lane] + 1)
                / f32::from(SPECTRAL_TRANSITION_SAMPLES);
            remaining[lane] -= 1;
            if remaining[lane] == 0 {
                current[lane] = target[lane];
            }
        }
        current_rows[lane] = i32::from(current[lane]);
        target_rows[lane] = i32::from(target[lane]);
    }
    SpectralRows8 {
        current: current_rows,
        target: target_rows,
        mix: f32x8::from(mixes),
        transitioning,
    }
}

#[inline]
fn spectral_harmonics8(phase_steps: [f32; 8]) -> [i32; 8] {
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    {
        use core::arch::x86_64::{
            _mm256_cvttps_epi32, _mm256_div_ps, _mm256_loadu_ps, _mm256_max_epi32,
            _mm256_min_epi32, _mm256_set1_epi32, _mm256_set1_ps, _mm256_storeu_si256,
        };
        let mut harmonics = [0; 8];
        // SAFETY: input and output are initialized contiguous eight-lane arrays and the
        // x86-64-v3 build guarantees AVX2. Clamping occurs before values leave the vector.
        unsafe {
            let steps = _mm256_loadu_ps(phase_steps.as_ptr());
            let limits = _mm256_div_ps(_mm256_set1_ps(0.5 - f32::EPSILON), steps);
            let values = _mm256_cvttps_epi32(limits);
            let values = _mm256_max_epi32(
                _mm256_set1_epi32(1),
                _mm256_min_epi32(_mm256_set1_epi32(SPECTRAL_MAX_HARMONICS as i32), values),
            );
            _mm256_storeu_si256(harmonics.as_mut_ptr().cast(), values);
        }
        for harmonic in &mut harmonics {
            *harmonic = spectral_saw_row(*harmonic as usize) as i32;
        }
        return harmonics;
    }
    #[cfg(not(all(target_arch = "x86_64", target_feature = "avx2")))]
    {
        std::array::from_fn(|lane| {
            let harmonic = (spectral_harmonic_limit(phase_steps[lane]) as usize)
                .clamp(1, SPECTRAL_MAX_HARMONICS);
            spectral_saw_row(harmonic) as i32
        })
    }
}

const fn spectral_saw_row(harmonic: usize) -> usize {
    if harmonic > SPECTRAL_MAX_HARMONICS {
        SPECTRAL_MAX_HARMONICS
    } else {
        harmonic
    }
}

const fn spectral_saw_harmonics(row: usize) -> usize {
    row
}

fn spectral_waveform8(
    waveform: Waveform,
    phase: f32x8,
    pulse_width: f32,
    rows: &SpectralRows8,
) -> f32x8 {
    match waveform {
        Waveform::Sine => aligned_sine_phase8(phase),
        Waveform::Triangle => spectral_lookup8(&SPECTRAL_TRIANGLE.0, phase, rows, true),
        Waveform::Saw => spectral_lookup_saw8(phase, rows),
        Waveform::Pulse => spectral_pulse8(phase, pulse_width, rows),
    }
}

fn spectral_pulse8(phase: f32x8, pulse_width: f32, rows: &SpectralRows8) -> f32x8 {
    let width = pulse_width.clamp(0.03, 0.97);
    let shifted = phase + f32x8::splat(1.0 - width);
    let shifted = shifted
        .cmp_lt(f32x8::ONE)
        .blend(shifted, shifted - f32x8::ONE);
    spectral_lookup_saw8(shifted, rows) - spectral_lookup_saw8(phase, rows)
        + f32x8::splat(width.mul_add(2.0, -1.0))
}

#[inline(always)]
fn spectral_lookup_saw8(phase: f32x8, rows: &SpectralRows8) -> f32x8 {
    let current = spectral_lookup_saw_rows8(phase, rows.current);
    if !rows.transitioning {
        return current;
    }
    let target = spectral_lookup_saw_rows8(phase, rows.target);
    (target - current).mul_add(rows.mix, current)
}

#[inline(always)]
fn spectral_lookup_saw_rows8(phase: f32x8, rows: [i32; 8]) -> f32x8 {
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    {
        return spectral_lookup_saw_rows8_avx2(phase, rows);
    }
    #[cfg(not(all(target_arch = "x86_64", target_feature = "avx2")))]
    {
        let phase: [f32; 8] = phase.into();
        f32x8::from(std::array::from_fn(|lane| {
            spectral_lookup(&SPECTRAL_SAW.0, rows[lane] as usize, phase[lane])
        }))
    }
}

#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
#[inline(always)]
fn spectral_lookup_saw_u16_rows8(phase: f32x8, rows: &[u16]) -> f32x8 {
    use core::arch::x86_64::{_mm_loadu_si128, _mm256_cvtepu16_epi32, _mm256_storeu_si256};

    debug_assert!(rows.len() >= 8);
    let mut widened = [0; 8];
    // SAFETY: `rows` contains at least eight initialized u16 values, `widened` has room
    // for eight i32 values, and the x86-64-v3 target guarantees AVX2.
    unsafe {
        let packed = _mm_loadu_si128(rows.as_ptr().cast());
        _mm256_storeu_si256(widened.as_mut_ptr().cast(), _mm256_cvtepu16_epi32(packed));
    }
    spectral_lookup_saw_rows8_avx2(phase, widened)
}

#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
#[inline(always)]
fn spectral_lookup_u16_rows8(bank: &[u8], phase: f32x8, rows: &[u16]) -> f32x8 {
    use core::arch::x86_64::{_mm_loadu_si128, _mm256_cvtepu16_epi32, _mm256_storeu_si256};

    debug_assert!(rows.len() >= 8);
    let mut widened = [0; 8];
    // SAFETY: `rows` contains eight initialized u16 values, the output contains eight
    // i32 lanes, and the x86-64-v3 target guarantees AVX2.
    unsafe {
        let packed = _mm_loadu_si128(rows.as_ptr().cast());
        _mm256_storeu_si256(widened.as_mut_ptr().cast(), _mm256_cvtepu16_epi32(packed));
    }
    spectral_lookup_rows8_avx2(bank, phase, widened)
}

#[cfg(not(all(target_arch = "x86_64", target_feature = "avx2")))]
fn spectral_lookup_u16_rows8(bank: &[u8], phase: f32x8, rows: &[u16]) -> f32x8 {
    debug_assert!(rows.len() >= 8);
    spectral_lookup_rows8(
        bank,
        phase,
        std::array::from_fn(|lane| i32::from(rows[lane])),
    )
}

#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
#[inline(always)]
fn spectral_saw_top_harmonic8(phase: f32x8, rows: [i32; 8]) -> f32x8 {
    use core::arch::x86_64::{
        _CMP_GT_OQ, _mm256_andnot_ps, _mm256_blendv_ps, _mm256_cmp_ps, _mm256_cvtepi32_ps,
        _mm256_cvttps_epi32, _mm256_div_ps, _mm256_fmadd_ps, _mm256_loadu_ps, _mm256_loadu_si256,
        _mm256_mul_ps, _mm256_set1_ps, _mm256_storeu_ps, _mm256_sub_ps,
    };

    let phase: [f32; 8] = phase.into();
    let mut output = [0.0; 8];
    // SAFETY: both inputs and the output are initialized eight-lane arrays. Rows are
    // maintained in 1..=128 and the x86-64-v3 target guarantees AVX2 and FMA.
    unsafe {
        let phase = _mm256_loadu_ps(phase.as_ptr());
        let harmonic = _mm256_loadu_si256(rows.as_ptr().cast());
        let harmonic_f = _mm256_cvtepi32_ps(harmonic);
        let position = _mm256_mul_ps(phase, harmonic_f);
        let top_phase = _mm256_sub_ps(position, _mm256_cvtepi32_ps(_mm256_cvttps_epi32(position)));
        let sign_mask = _mm256_set1_ps(-0.0);
        let half = _mm256_set1_ps(0.5);
        let quarter = _mm256_set1_ps(0.25);
        let folded = _mm256_sub_ps(
            quarter,
            _mm256_andnot_ps(
                sign_mask,
                _mm256_sub_ps(
                    _mm256_andnot_ps(sign_mask, _mm256_sub_ps(top_phase, half)),
                    quarter,
                ),
            ),
        );
        let folded2 = _mm256_mul_ps(folded, folded);
        let folded4 = _mm256_mul_ps(folded2, folded2);
        let low = _mm256_fmadd_ps(
            _mm256_set1_ps(-41.341_7),
            folded2,
            _mm256_set1_ps(std::f32::consts::TAU),
        );
        let middle = _mm256_fmadd_ps(
            _mm256_set1_ps(-76.705_86),
            folded2,
            _mm256_set1_ps(81.605_25),
        );
        let high = _mm256_fmadd_ps(
            _mm256_set1_ps(-15.094_643),
            folded2,
            _mm256_set1_ps(42.058_693),
        );
        let polynomial = _mm256_fmadd_ps(_mm256_fmadd_ps(high, folded4, middle), folded4, low);
        let sine = _mm256_mul_ps(folded, polynomial);
        let sine = _mm256_blendv_ps(
            sine,
            _mm256_sub_ps(_mm256_set1_ps(0.0), sine),
            _mm256_cmp_ps(top_phase, half, _CMP_GT_OQ),
        );
        let coefficient = _mm256_div_ps(_mm256_set1_ps(-2.0 / std::f32::consts::PI), harmonic_f);
        _mm256_storeu_ps(output.as_mut_ptr(), _mm256_mul_ps(sine, coefficient));
    }
    f32x8::from(output)
}

#[cfg(not(all(target_arch = "x86_64", target_feature = "avx2")))]
fn spectral_lookup_saw_u16_rows8(phase: f32x8, rows: &[u16]) -> f32x8 {
    debug_assert!(rows.len() >= 8);
    spectral_lookup_saw_rows8(phase, std::array::from_fn(|lane| i32::from(rows[lane])))
}

#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
#[inline(always)]
fn spectral_lookup_saw_rows8_avx2(phase: f32x8, rows: [i32; 8]) -> f32x8 {
    use core::arch::x86_64::{
        _mm256_add_epi32, _mm256_and_si256, _mm256_cvtepi32_ps, _mm256_cvttps_epi32,
        _mm256_fmadd_ps, _mm256_i32gather_ps, _mm256_loadu_ps, _mm256_loadu_si256, _mm256_mul_ps,
        _mm256_mullo_epi32, _mm256_set1_epi32, _mm256_set1_ps, _mm256_storeu_ps, _mm256_sub_ps,
    };

    let phase: [f32; 8] = phase.into();
    let mut output = [0.0; 8];
    // SAFETY: phase and output are initialized eight-float arrays. Rows are maintained in
    // 1..=128 by `spectral_rows8`, and normalized phase keeps both interpolation points
    // within the aligned immutable saw bank. The build target guarantees AVX2.
    unsafe {
        let phase = _mm256_loadu_ps(phase.as_ptr());
        let rows = _mm256_loadu_si256(rows.as_ptr().cast());
        let position = _mm256_mul_ps(phase, _mm256_set1_ps(SPECTRAL_TABLE_SIZE as f32));
        let index = _mm256_cvttps_epi32(position);
        let fraction = _mm256_sub_ps(position, _mm256_cvtepi32_ps(index));
        let table = _mm256_mullo_epi32(rows, _mm256_set1_epi32(SPECTRAL_TABLE_STRIDE as i32));
        let absolute = _mm256_add_epi32(index, table);
        let next_index = _mm256_and_si256(
            _mm256_add_epi32(index, _mm256_set1_epi32(1)),
            _mm256_set1_epi32((SPECTRAL_TABLE_SIZE - 1) as i32),
        );
        let next_absolute = _mm256_add_epi32(next_index, table);
        let base = SPECTRAL_SAW.0.as_ptr().cast::<f32>();
        let first = _mm256_i32gather_ps(base, absolute, 4);
        let second = _mm256_i32gather_ps(base, next_absolute, 4);
        let result = _mm256_fmadd_ps(_mm256_sub_ps(second, first), fraction, first);
        _mm256_storeu_ps(output.as_mut_ptr(), result);
    }
    f32x8::from(output)
}

#[inline(always)]
fn spectral_lookup8(bank: &[u8], phase: f32x8, rows: &SpectralRows8, triangle: bool) -> f32x8 {
    let current_rows = if triangle {
        rows.current
            .map(|row| row.min(SPECTRAL_EXACT_HARMONICS as i32))
    } else {
        rows.current
    };
    let current = spectral_lookup_rows8(bank, phase, current_rows);
    if !rows.transitioning {
        return current;
    }
    let target_rows = if triangle {
        rows.target
            .map(|row| row.min(SPECTRAL_EXACT_HARMONICS as i32))
    } else {
        rows.target
    };
    let target = spectral_lookup_rows8(bank, phase, target_rows);
    (target - current).mul_add(rows.mix, current)
}

#[inline(always)]
fn spectral_lookup_rows8(bank: &[u8], phase: f32x8, rows: [i32; 8]) -> f32x8 {
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    {
        return spectral_lookup_rows8_avx2(bank, phase, rows);
    }
    #[cfg(not(all(target_arch = "x86_64", target_feature = "avx2")))]
    {
        let phase: [f32; 8] = phase.into();
        f32x8::from(std::array::from_fn(|lane| {
            spectral_lookup(bank, rows[lane] as usize, phase[lane])
        }))
    }
}

#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
#[inline(always)]
fn spectral_lookup_rows8_avx2(bank: &[u8], phase: f32x8, rows: [i32; 8]) -> f32x8 {
    use core::arch::x86_64::{
        _mm256_add_epi32, _mm256_and_si256, _mm256_cvtepi32_ps, _mm256_cvttps_epi32,
        _mm256_fmadd_ps, _mm256_i32gather_ps, _mm256_loadu_ps, _mm256_loadu_si256, _mm256_mul_ps,
        _mm256_mullo_epi32, _mm256_set1_epi32, _mm256_set1_ps, _mm256_storeu_ps, _mm256_sub_ps,
    };

    let phase: [f32; 8] = phase.into();
    let mut output = [0.0; 8];
    // SAFETY: phase and output are initialized eight-float arrays. `rows` is produced by
    // `spectral_rows8` in 1..=128 and phase comes from normalized oscillator state, so both
    // interpolated gather addresses are inside the aligned immutable 129 x 4097-float bank.
    unsafe {
        let phase = _mm256_loadu_ps(phase.as_ptr());
        let rows = _mm256_loadu_si256(rows.as_ptr().cast());
        let position = _mm256_mul_ps(phase, _mm256_set1_ps(SPECTRAL_TABLE_SIZE as f32));
        let index = _mm256_cvttps_epi32(position);
        let fraction = _mm256_sub_ps(position, _mm256_cvtepi32_ps(index));
        let table = _mm256_mullo_epi32(rows, _mm256_set1_epi32(SPECTRAL_TABLE_STRIDE as i32));
        let absolute = _mm256_add_epi32(index, table);
        let next_index = _mm256_and_si256(
            _mm256_add_epi32(index, _mm256_set1_epi32(1)),
            _mm256_set1_epi32((SPECTRAL_TABLE_SIZE - 1) as i32),
        );
        let next_absolute = _mm256_add_epi32(next_index, table);
        let base = bank.as_ptr().cast::<f32>();
        let first = _mm256_i32gather_ps(base, absolute, 4);
        let second = _mm256_i32gather_ps(base, next_absolute, 4);
        let result = _mm256_fmadd_ps(_mm256_sub_ps(second, first), fraction, first);
        _mm256_storeu_ps(output.as_mut_ptr(), result);
    }
    f32x8::from(output)
}

#[inline]
fn spectral_saw(phase: f32, phase_step: f32) -> f32 {
    let limit = spectral_harmonic_limit(phase_step);
    if limit > SPECTRAL_MAX_HARMONICS as f32 {
        return phase.mul_add(2.0, -1.0)
            - spline_blep(f64::from(phase), f64::from(phase_step), true) as f32;
    }
    let row = spectral_saw_row((limit as usize).clamp(1, SPECTRAL_MAX_HARMONICS));
    spectral_lookup_faded(
        &SPECTRAL_SAW.0,
        row,
        spectral_saw_harmonics(row),
        limit,
        phase,
    )
}

#[inline]
fn spectral_triangle(phase: f32, phase_step: f32) -> f32 {
    let limit = spectral_harmonic_limit(phase_step);
    let row = (limit as usize).clamp(1, SPECTRAL_EXACT_HARMONICS);
    spectral_lookup_faded(&SPECTRAL_TRIANGLE.0, row, row, limit, phase)
}

#[inline]
fn spectral_harmonic_limit(phase_step: f32) -> f32 {
    if phase_step > f32::EPSILON {
        (0.5 - f32::EPSILON) / phase_step
    } else {
        SPECTRAL_MAX_HARMONICS as f32
    }
}

#[inline]
fn spectral_top_gain(clearance: f32) -> f32 {
    let position = (clearance * 4.0).clamp(0.0, 1.0);
    position * position * (-2.0f32).mul_add(position, 3.0)
}

#[inline]
fn spectral_lookup_faded(
    bank: &[u8],
    row: usize,
    stored_harmonics: usize,
    limit: f32,
    phase: f32,
) -> f32 {
    let current = spectral_lookup(bank, row, phase);
    if row == 0 {
        return current;
    }
    let previous = spectral_lookup(bank, row - 1, phase);
    (current - previous).mul_add(spectral_top_gain(limit - stored_harmonics as f32), previous)
}

#[inline]
fn spectral_lookup(bank: &[u8], row: usize, phase: f32) -> f32 {
    let position = phase * SPECTRAL_TABLE_SIZE as f32;
    let index = position as usize;
    let fraction = position - index as f32;
    let table = row * SPECTRAL_TABLE_STRIDE;
    let first = spectral_bank_sample(bank, table + index);
    let second = spectral_bank_sample(bank, table + ((index + 1) & (SPECTRAL_TABLE_SIZE - 1)));
    (second - first).mul_add(fraction, first)
}

#[inline]
fn spectral_saw4(phase: f32x4, phase_step: f32x4) -> f32x4 {
    let phase: [f32; 4] = phase.into();
    let phase_step: [f32; 4] = phase_step.into();
    f32x4::from(std::array::from_fn(|lane| {
        spectral_saw(phase[lane], phase_step[lane])
    }))
}

#[inline]
fn spectral_triangle4(phase: f32x4, phase_step: f32x4) -> f32x4 {
    let phase: [f32; 4] = phase.into();
    let phase_step: [f32; 4] = phase_step.into();
    f32x4::from(std::array::from_fn(|lane| {
        spectral_triangle(phase[lane], phase_step[lane])
    }))
}

#[inline]
fn spectral_saw8(phase: f32x8, phase_step: f32x8) -> f32x8 {
    let phase: [f32; 8] = phase.into();
    let phase_step: [f32; 8] = phase_step.into();
    f32x8::from(std::array::from_fn(|lane| {
        spectral_saw(phase[lane], phase_step[lane])
    }))
}

#[inline]
fn spectral_triangle8(phase: f32x8, phase_step: f32x8) -> f32x8 {
    let phase: [f32; 8] = phase.into();
    let phase_step: [f32; 8] = phase_step.into();
    f32x8::from(std::array::from_fn(|lane| {
        spectral_triangle(phase[lane], phase_step[lane])
    }))
}

#[inline]
fn spectral_bank_sample(bank: &[u8], index: usize) -> f32 {
    let offset = index * 4;
    f32::from_le_bytes([
        bank[offset],
        bank[offset + 1],
        bank[offset + 2],
        bank[offset + 3],
    ])
}

fn bandlimited_saw(phase: f64, phase_step: f64, antialiasing: Antialiasing) -> f64 {
    if antialiasing == Antialiasing::Spectral {
        return f64::from(spectral_saw(phase as f32, phase_step as f32));
    }
    2.0_f64.mul_add(phase, -1.0) - edge_blep(phase, phase_step, antialiasing)
}

fn edge_blep(phase: f64, phase_step: f64, antialiasing: Antialiasing) -> f64 {
    match antialiasing {
        Antialiasing::Legacy => poly_blep(phase, phase_step),
        Antialiasing::Spline => spline_blep(phase, phase_step, false),
        Antialiasing::SplineOptimized => spline_blep(phase, phase_step, true),
        Antialiasing::Lagrange => lagrange_blep(phase, phase_step),
        Antialiasing::Spectral => spline_blep(phase, phase_step, true),
    }
}

fn bandlimited_saw4(phase: f32x4, phase_step: f32x4, antialiasing: Antialiasing) -> f32x4 {
    if antialiasing == Antialiasing::Spectral {
        return spectral_saw4(phase, phase_step);
    }
    phase * f32x4::splat(2.0) - f32x4::ONE - edge_blep4(phase, phase_step, antialiasing)
}

fn edge_blep4(phase: f32x4, phase_step: f32x4, antialiasing: Antialiasing) -> f32x4 {
    match antialiasing {
        Antialiasing::Legacy => poly_blep4(phase, phase_step),
        Antialiasing::Spline => spline_blep4(phase, phase_step, false),
        Antialiasing::SplineOptimized => spline_blep4(phase, phase_step, true),
        Antialiasing::Lagrange => lagrange_blep4(phase, phase_step),
        Antialiasing::Spectral => spline_blep4(phase, phase_step, true),
    }
}

fn bandlimited_saw8(phase: f32x8, phase_step: f32x8, antialiasing: Antialiasing) -> f32x8 {
    if antialiasing == Antialiasing::Spectral {
        return spectral_saw8(phase, phase_step);
    }
    phase * f32x8::splat(2.0) - f32x8::ONE - edge_blep8(phase, phase_step, antialiasing)
}

fn edge_blep8(phase: f32x8, phase_step: f32x8, antialiasing: Antialiasing) -> f32x8 {
    match antialiasing {
        Antialiasing::Legacy => poly_blep8(phase, phase_step),
        Antialiasing::Spline => spline_blep8(phase, phase_step, false),
        Antialiasing::SplineOptimized => spline_blep8(phase, phase_step, true),
        Antialiasing::Lagrange => lagrange_blep8(phase, phase_step),
        Antialiasing::Spectral => spline_blep8(phase, phase_step, true),
    }
}

fn bandlimited_saw_pulse_morph4(
    phase: f32x4,
    phase_step: f32x4,
    pulse_width: f32,
    blend: f32,
    antialiasing: Antialiasing,
) -> f32x4 {
    if antialiasing == Antialiasing::Spectral {
        let width = pulse_width.clamp(0.03, 0.97);
        let shifted = phase + f32x4::splat(1.0 - width);
        let shifted = shifted
            .cmp_lt(f32x4::ONE)
            .blend(shifted, shifted - f32x4::ONE);
        let saw = spectral_saw4(phase, phase_step);
        let pulse =
            spectral_saw4(shifted, phase_step) - saw + f32x4::splat(width.mul_add(2.0, -1.0));
        return (pulse - saw).mul_add(f32x4::splat(blend), saw);
    }
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

fn bandlimited_saw_pulse_morph8(
    phase: f32x8,
    phase_step: f32x8,
    pulse_width: f32,
    blend: f32,
    antialiasing: Antialiasing,
) -> f32x8 {
    if antialiasing == Antialiasing::Spectral {
        let width = pulse_width.clamp(0.03, 0.97);
        let shifted = phase + f32x8::splat(1.0 - width);
        let shifted = shifted
            .cmp_lt(f32x8::ONE)
            .blend(shifted, shifted - f32x8::ONE);
        let saw = spectral_saw8(phase, phase_step);
        let pulse =
            spectral_saw8(shifted, phase_step) - saw + f32x8::splat(width.mul_add(2.0, -1.0));
        return (pulse - saw).mul_add(f32x8::splat(blend), saw);
    }
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

fn bandlimited_pulse(
    phase: f64,
    phase_step: f64,
    pulse_width: f64,
    antialiasing: Antialiasing,
) -> f64 {
    if antialiasing == Antialiasing::Spectral {
        let width = pulse_width.clamp(0.03, 0.97) as f32;
        let shifted = wrap01(phase + 1.0 - f64::from(width)) as f32;
        return f64::from(
            spectral_saw(shifted, phase_step as f32)
                - spectral_saw(phase as f32, phase_step as f32)
                + width.mul_add(2.0, -1.0),
        );
    }
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

fn bandlimited_pulse4(
    phase: f32x4,
    phase_step: f32x4,
    pulse_width: f32,
    antialiasing: Antialiasing,
) -> f32x4 {
    if antialiasing == Antialiasing::Spectral {
        let width = pulse_width.clamp(0.03, 0.97);
        let shifted = phase + f32x4::splat(1.0 - width);
        let shifted = shifted
            .cmp_lt(f32x4::ONE)
            .blend(shifted, shifted - f32x4::ONE);
        return spectral_saw4(shifted, phase_step) - spectral_saw4(phase, phase_step)
            + f32x4::splat(width.mul_add(2.0, -1.0));
    }
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

fn bandlimited_pulse8(
    phase: f32x8,
    phase_step: f32x8,
    pulse_width: f32,
    antialiasing: Antialiasing,
) -> f32x8 {
    if antialiasing == Antialiasing::Spectral {
        let width = pulse_width.clamp(0.03, 0.97);
        let shifted = phase + f32x8::splat(1.0 - width);
        let shifted = shifted
            .cmp_lt(f32x8::ONE)
            .blend(shifted, shifted - f32x8::ONE);
        return spectral_saw8(shifted, phase_step) - spectral_saw8(phase, phase_step)
            + f32x8::splat(width.mul_add(2.0, -1.0));
    }
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

fn lagrange_blep(phase: f64, phase_step: f64) -> f64 {
    if phase_step <= f64::EPSILON {
        return 0.0;
    }
    let support = 2.0 * phase_step;
    if support < 0.5 && phase >= support && phase <= 1.0 - support {
        return 0.0;
    }
    let inverse_step = phase_step.recip();
    2.0 * (lagrange_blep_residual(phase * inverse_step)
        + lagrange_blep_residual((phase - 1.0) * inverse_step))
}

fn lagrange_blep_residual(position: f64) -> f64 {
    let distance = position.abs();
    if distance >= 2.0 {
        return 0.0;
    }
    let residual = if distance < 1.0 {
        let residual = 0.125_f64.mul_add(distance, -1.0 / 3.0);
        let residual = residual.mul_add(distance, -0.25);
        let residual = residual.mul_add(distance, 1.0);
        residual.mul_add(distance, -0.5)
    } else {
        let tail = distance - 1.0;
        let residual = (-1.0 / 24.0_f64).mul_add(tail, 1.0 / 6.0);
        let residual = residual.mul_add(tail, -1.0 / 6.0) * tail;
        residual.mul_add(tail, 1.0 / 24.0)
    };
    if position < 0.0 { -residual } else { residual }
}

fn lagrange_blamp(phase: f64, phase_step: f64) -> f64 {
    if phase_step <= f64::EPSILON {
        return 0.0;
    }
    let support = 2.0 * phase_step;
    if support < 0.5 && phase >= support && phase <= 1.0 - support {
        return 0.0;
    }
    let inverse_step = phase_step.recip();
    lagrange_blamp_residual(phase * inverse_step)
        + lagrange_blamp_residual((phase - 1.0) * inverse_step)
}

fn lagrange_blamp_residual(position: f64) -> f64 {
    let distance = position.abs();
    if distance >= 2.0 {
        return 0.0;
    }
    if distance < 1.0 {
        let residual = (1.0 / 40.0_f64).mul_add(distance, -1.0 / 12.0);
        let residual = residual.mul_add(distance, -1.0 / 12.0);
        let residual = residual.mul_add(distance, 0.5);
        let residual = residual.mul_add(distance, -0.5);
        residual.mul_add(distance, 11.0 / 90.0)
    } else {
        let tail = distance - 1.0;
        let residual = (-1.0 / 120.0_f64).mul_add(tail, 1.0 / 24.0);
        let residual = residual.mul_add(tail, -1.0 / 18.0) * tail;
        let residual = residual.mul_add(tail, 1.0 / 24.0);
        residual.mul_add(tail, -7.0 / 360.0)
    }
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

fn poly_blep(phase: f64, phase_step: f64) -> f64 {
    if phase_step <= f64::EPSILON {
        return 0.0;
    }
    if phase < phase_step {
        let position = phase / phase_step;
        let edge = position - 1.0;
        return -edge * edge;
    }
    if phase > 1.0 - phase_step {
        let position = (phase - 1.0) / phase_step;
        let edge = position + 1.0;
        return edge * edge;
    }
    0.0
}

fn lagrange_blep4(phase: f32x4, phase_step: f32x4) -> f32x4 {
    let zero = f32x4::ZERO;
    let one = f32x4::ONE;
    let active = phase_step.cmp_gt(f32x4::splat(f32::EPSILON));
    let support = phase_step * f32x4::splat(2.0);
    let event = active & (phase.cmp_lt(support) | phase.cmp_gt(one - support));
    if !event.any() {
        return zero;
    }
    let inverse_step = one / event.blend(phase_step, one);
    let correction = (lagrange_blep_residual4(phase * inverse_step)
        + lagrange_blep_residual4((phase - one) * inverse_step))
        * f32x4::splat(2.0);
    event.blend(correction, zero)
}

fn lagrange_blep_residual4(position: f32x4) -> f32x4 {
    let zero = f32x4::ZERO;
    let distance = position.abs();
    let inner = distance
        .mul_add(f32x4::splat(0.125), f32x4::splat(-1.0 / 3.0))
        .mul_add(distance, f32x4::splat(-0.25))
        .mul_add(distance, f32x4::ONE)
        .mul_add(distance, f32x4::splat(-0.5));
    let tail = distance - f32x4::ONE;
    let outer = tail
        .mul_add(f32x4::splat(-1.0 / 24.0), f32x4::splat(1.0 / 6.0))
        .mul_add(tail, f32x4::splat(-1.0 / 6.0))
        .mul_add(tail, zero)
        .mul_add(tail, f32x4::splat(1.0 / 24.0));
    let residual = distance.cmp_lt(f32x4::ONE).blend(inner, outer);
    let residual = distance.cmp_lt(f32x4::splat(2.0)).blend(residual, zero);
    position.cmp_lt(zero).blend(-residual, residual)
}

fn lagrange_blamp4(phase: f32x4, phase_step: f32x4) -> f32x4 {
    let zero = f32x4::ZERO;
    let one = f32x4::ONE;
    let active = phase_step.cmp_gt(f32x4::splat(f32::EPSILON));
    let support = phase_step * f32x4::splat(2.0);
    let event = active & (phase.cmp_lt(support) | phase.cmp_gt(one - support));
    if !event.any() {
        return zero;
    }
    let inverse_step = one / event.blend(phase_step, one);
    let correction = lagrange_blamp_residual4(phase * inverse_step)
        + lagrange_blamp_residual4((phase - one) * inverse_step);
    event.blend(correction, zero)
}

fn lagrange_blamp_residual4(position: f32x4) -> f32x4 {
    let zero = f32x4::ZERO;
    let distance = position.abs();
    let inner = distance
        .mul_add(f32x4::splat(1.0 / 40.0), f32x4::splat(-1.0 / 12.0))
        .mul_add(distance, f32x4::splat(-1.0 / 12.0))
        .mul_add(distance, f32x4::splat(0.5))
        .mul_add(distance, f32x4::splat(-0.5))
        .mul_add(distance, f32x4::splat(11.0 / 90.0));
    let tail = distance - f32x4::ONE;
    let outer = tail
        .mul_add(f32x4::splat(-1.0 / 120.0), f32x4::splat(1.0 / 24.0))
        .mul_add(tail, f32x4::splat(-1.0 / 18.0));
    let outer = (outer * tail)
        .mul_add(tail, f32x4::splat(1.0 / 24.0))
        .mul_add(tail, f32x4::splat(-7.0 / 360.0));
    let residual = distance.cmp_lt(f32x4::ONE).blend(inner, outer);
    distance.cmp_lt(f32x4::splat(2.0)).blend(residual, zero)
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

fn spline_blep4_precomputed(
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

fn spline_triangle4_precomputed(
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

fn lagrange_blep8(phase: f32x8, phase_step: f32x8) -> f32x8 {
    let zero = f32x8::ZERO;
    let one = f32x8::ONE;
    let active = phase_step.cmp_gt(f32x8::splat(f32::EPSILON));
    let support = phase_step * f32x8::splat(2.0);
    let event = active & (phase.cmp_lt(support) | phase.cmp_gt(one - support));
    if !event.any() {
        return zero;
    }
    let inverse_step = one / event.blend(phase_step, one);
    let correction = (lagrange_blep_residual8(phase * inverse_step)
        + lagrange_blep_residual8((phase - one) * inverse_step))
        * f32x8::splat(2.0);
    event.blend(correction, zero)
}

fn lagrange_blep_residual8(position: f32x8) -> f32x8 {
    let zero = f32x8::ZERO;
    let distance = position.abs();
    let inner = distance
        .mul_add(f32x8::splat(0.125), f32x8::splat(-1.0 / 3.0))
        .mul_add(distance, f32x8::splat(-0.25))
        .mul_add(distance, f32x8::ONE)
        .mul_add(distance, f32x8::splat(-0.5));
    let tail = distance - f32x8::ONE;
    let outer = tail
        .mul_add(f32x8::splat(-1.0 / 24.0), f32x8::splat(1.0 / 6.0))
        .mul_add(tail, f32x8::splat(-1.0 / 6.0))
        .mul_add(tail, zero)
        .mul_add(tail, f32x8::splat(1.0 / 24.0));
    let residual = distance.cmp_lt(f32x8::ONE).blend(inner, outer);
    let residual = distance.cmp_lt(f32x8::splat(2.0)).blend(residual, zero);
    position.cmp_lt(zero).blend(-residual, residual)
}

fn lagrange_blamp8(phase: f32x8, phase_step: f32x8) -> f32x8 {
    let zero = f32x8::ZERO;
    let one = f32x8::ONE;
    let active = phase_step.cmp_gt(f32x8::splat(f32::EPSILON));
    let support = phase_step * f32x8::splat(2.0);
    let event = active & (phase.cmp_lt(support) | phase.cmp_gt(one - support));
    if !event.any() {
        return zero;
    }
    let inverse_step = one / event.blend(phase_step, one);
    let correction = lagrange_blamp_residual8(phase * inverse_step)
        + lagrange_blamp_residual8((phase - one) * inverse_step);
    event.blend(correction, zero)
}

fn lagrange_blamp_residual8(position: f32x8) -> f32x8 {
    let zero = f32x8::ZERO;
    let distance = position.abs();
    let inner = distance
        .mul_add(f32x8::splat(1.0 / 40.0), f32x8::splat(-1.0 / 12.0))
        .mul_add(distance, f32x8::splat(-1.0 / 12.0))
        .mul_add(distance, f32x8::splat(0.5))
        .mul_add(distance, f32x8::splat(-0.5))
        .mul_add(distance, f32x8::splat(11.0 / 90.0));
    let tail = distance - f32x8::ONE;
    let outer = tail
        .mul_add(f32x8::splat(-1.0 / 120.0), f32x8::splat(1.0 / 24.0))
        .mul_add(tail, f32x8::splat(-1.0 / 18.0));
    let outer = (outer * tail)
        .mul_add(tail, f32x8::splat(1.0 / 24.0))
        .mul_add(tail, f32x8::splat(-7.0 / 360.0));
    let residual = distance.cmp_lt(f32x8::ONE).blend(inner, outer);
    distance.cmp_lt(f32x8::splat(2.0)).blend(residual, zero)
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
            optimized_cubic_blep_residual8(position)
        } else {
            cubic_blep_residual8(position)
        }
    } else if optimized {
        optimized_cubic_blep_residual8(phase * inverse_step)
            + optimized_cubic_blep_residual8((phase - one) * inverse_step)
    } else {
        cubic_blep_residual8(phase * inverse_step)
            + cubic_blep_residual8((phase - one) * inverse_step)
    } * f32x8::splat(2.0);
    event.blend(correction, zero)
}

#[inline]
fn spline_saw8_narrow(phase: f32x8, phase_step: f32x8, optimized: bool) -> f32x8 {
    let zero = f32x8::ZERO;
    let one = f32x8::ONE;
    let support = phase_step * f32x8::splat(2.0);
    let event = phase.cmp_lt(support) | phase.cmp_gt(one - support);
    let correction = if event.any() {
        let inverse_step = one / event.blend(phase_step, one);
        let position = phase.cmp_lt(f32x8::splat(0.5)).blend(phase, phase - one) * inverse_step;
        let residual = if optimized {
            optimized_cubic_blep_residual8(position)
        } else {
            cubic_blep_residual8(position)
        };
        event.blend(residual, zero) * f32x8::splat(2.0)
    } else {
        zero
    };
    phase * f32x8::splat(2.0) - one - correction
}

fn spline_blep8_precomputed(
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
    let narrow = support.cmp_lt(f32x8::splat(0.5)).all();
    let correction = if narrow {
        let position = phase.cmp_lt(f32x8::splat(0.5)).blend(phase, phase - one) * inverse_step;
        if optimized {
            optimized_cubic_blep_residual8(position)
        } else {
            cubic_blep_residual8(position)
        }
    } else if optimized {
        optimized_cubic_blep_residual8(phase * inverse_step)
            + optimized_cubic_blep_residual8((phase - one) * inverse_step)
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

fn optimized_cubic_blep_residual8(position: f32x8) -> f32x8 {
    let zero = f32x8::ZERO;
    let distance = position.abs();
    let inner = f32x8::splat(0.116_560_56).mul_add(distance, f32x8::splat(-0.316_694_7));
    let inner = inner.mul_add(distance, f32x8::splat(0.024_084_598));
    let inner = inner
        .mul_add(distance, f32x8::splat(0.623_499_63))
        .mul_add(distance, f32x8::splat(-0.5));
    let tail = f32x8::splat(2.0) - distance;
    let outer = f32x8::splat(-0.038_711_853).mul_add(tail, f32x8::splat(-0.006_173_230_2));
    let outer = outer.mul_add(tail, f32x8::splat(-0.007_354_877_4));
    let outer = outer.mul_add(tail, f32x8::splat(-0.000_309_994_82)) * tail;
    let residual = distance.cmp_lt(f32x8::ONE).blend(inner, outer);
    let residual = distance.cmp_lt(f32x8::splat(2.0)).blend(residual, zero);
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
        optimized_cubic_blamp_residual8(phase * inverse_step)
            + optimized_cubic_blamp_residual8((phase - one) * inverse_step)
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
        optimized_cubic_blamp_residual8(phase * inverse_step)
            + optimized_cubic_blamp_residual8((phase - one) * inverse_step)
    } else {
        cubic_blamp_residual8(phase * inverse_step)
            + cubic_blamp_residual8((phase - one) * inverse_step)
    };
    event.blend(correction, zero)
}

#[inline]
fn spline_triangle8_precomputed(
    phase: f32x8,
    phase_step: f32x8,
    active: f32x8,
    support: f32x8,
    inverse_step: f32x8,
    optimized: bool,
) -> f32x8 {
    let half = f32x8::splat(0.5);
    let sample = (phase - half).abs() * f32x8::splat(-4.0) + f32x8::ONE;
    if support.cmp_lt(f32x8::splat(0.25)).all() {
        let one = f32x8::ONE;
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
            optimized_cubic_blamp_residual8(position)
        } else {
            cubic_blamp_residual8(position)
        };
        let correction = peak_event.blend(-correction, correction);
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

fn optimized_cubic_blamp_residual8(position: f32x8) -> f32x8 {
    let zero = f32x8::ZERO;
    let distance = position.abs();
    let inner = f32x8::splat(0.023_312_11).mul_add(distance, f32x8::splat(-0.079_173_68));
    let inner = inner.mul_add(distance, f32x8::splat(0.008_028_2));
    let inner = inner.mul_add(distance, f32x8::splat(0.311_749_82));
    let inner = inner
        .mul_add(distance, f32x8::splat(-0.5))
        .mul_add(distance, f32x8::splat(0.247_975_87));
    let tail = f32x8::splat(2.0) - distance;
    let outer = f32x8::splat(0.007_742_371).mul_add(tail, f32x8::splat(0.001_543_307_7));
    let outer = outer.mul_add(tail, f32x8::splat(0.002_451_625_8));
    let outer = outer.mul_add(tail, f32x8::splat(0.000_154_997_42)) * tail * tail;
    let residual = distance.cmp_lt(f32x8::ONE).blend(inner, outer);
    distance.cmp_lt(f32x8::splat(2.0)).blend(residual, zero)
}

fn poly_blep4(phase: f32x4, phase_step: f32x4) -> f32x4 {
    let zero = f32x4::ZERO;
    let one = f32x4::ONE;
    let start_mask = phase.cmp_lt(phase_step);
    let end_mask = phase.cmp_gt(one - phase_step);
    if !(start_mask | end_mask).any() {
        return zero;
    }
    let inverse_step = one / phase_step;
    let start_edge = phase * inverse_step - one;
    let start = -(start_edge * start_edge);
    let end_edge = (phase - one) * inverse_step + one;
    let end = end_edge * end_edge;
    let correction = end_mask.blend(end, zero);
    start_mask.blend(start, correction)
}

fn poly_blep8(phase: f32x8, phase_step: f32x8) -> f32x8 {
    let zero = f32x8::ZERO;
    let one = f32x8::ONE;
    let start_mask = phase.cmp_lt(phase_step);
    let end_mask = phase.cmp_gt(one - phase_step);
    if !(start_mask | end_mask).any() {
        return zero;
    }
    let inverse_step = one / phase_step;
    let start_edge = phase * inverse_step - one;
    let start = -(start_edge * start_edge);
    let end_edge = (phase - one) * inverse_step + one;
    let end = end_edge * end_edge;
    let correction = end_mask.blend(end, zero);
    start_mask.blend(start, correction)
}

fn wrap01(value: f64) -> f64 {
    value - value.floor()
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
