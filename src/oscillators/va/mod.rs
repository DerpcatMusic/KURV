//! Fast procedural virtual-analog oscillator.

mod antialias;
mod backend;
#[cfg(test)]
mod experiment;
pub(crate) mod function;
#[cfg(test)]
mod minblep_experiment;
#[cfg(feature = "experimental-1x-dsp")]
mod one_x_high;
mod ratio;
mod render;
mod table;
mod warp;

use crate::wave_curve::WaveCurveRt;
use truce_simd::simd::f32x8;
use wide::{CmpGe, CmpLt};

use antialias::{
    bandlimited_saw8, sine_cosine_phase4, sine_cosine_phase8, sine_phase4, sine_phase8,
    spline_blep_precomputed_scalar, spline_blep8_precomputed_static_with_bounds, wrap_phase4,
    wrap_phase8, wrap01,
};

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

pub use antialias::Antialiasing;
pub use backend::accumulate_saw8_block_constant;
pub(crate) use backend::calibrate_spline_backends;
pub(crate) use function::{DEFAULT_VA_FUNCTION, compile_va_function};
pub(crate) use ratio::{
    PreparedRatioSource, accumulate_shape4_ratio_block, accumulate_shape8_ratio_block,
    generate_shape8_ratio,
};
pub use render::{
    accumulate_custom4_block, accumulate_custom4_block_constant, accumulate_custom8_block,
    accumulate_custom8_block_constant, accumulate_saw4_block, accumulate_saw4_block_constant,
    accumulate_saw4_block_dynamic_gains, accumulate_saw4_block_static_gains, accumulate_saw8_block,
    accumulate_saw8_block_dynamic_gains, accumulate_saw8_block_static_gains,
    accumulate_saw8_block_static_gains_narrow_spline, accumulate_shape4_block_constant,
    accumulate_shape4_block_constant_warped, accumulate_shape4_block_dynamic,
    accumulate_shape4_block_morphing, accumulate_shape4_block_steps,
    accumulate_shape8_block_constant, accumulate_shape8_block_constant_warped,
    accumulate_shape8_block_dynamic, accumulate_shape8_block_morphing,
    accumulate_shape8_block_steps, accumulate_shape8_phase_modulated_block,
    accumulate_spline_saw4_phase_modulated_block, accumulate_spline_saw8_phase_modulated_block,
    accumulate_spline_saw8_phase_modulated_lanes_block, generate_custom4, generate_custom8,
    generate_pulse4, generate_pulse8, generate_saw4, generate_saw8, generate_shape_time8,
    generate_shape_time8_steps, generate_shape4, generate_shape4_pair, generate_shape4_pair_warped,
    generate_shape4_warped, generate_shape8, generate_shape8_pair, generate_shape8_pair_warped,
    generate_shape8_warped, generate_sine4, generate_sine8, generate_triangle4, generate_triangle8,
    is_narrow_spline_ramp, sample_custom_shape_with_antialiasing_warped, shape_morph_gain,
};
#[cfg(test)]
pub(crate) use render::{
    accumulate_custom4_block_constant_unprepared_blep_probe,
    accumulate_custom8_block_constant_unprepared_blep_probe,
};
use render::{
    sample_shape_normalized, sample_shape_normalized_warped_auto_edge,
    sample_shape_normalized_warped_impl,
};
pub(crate) use table::{
    ImportedVaTable, MAX_VA_TABLE_FILE_BYTES, MAX_VA_TABLE_FRAMES, VA_KEYFRAME_EPSILON,
    VaTableData, VaTableRt, VaTableState, nearest_frame_index, position_for_frame,
};
pub use warp::PhaseWarpMode;
#[cfg(test)]
use warp::{
    prepare_scalar_warp_depth, warp_phase_scalar_unprepared_probe, warp_phase_scalar_with_depth,
};
use warp::{warp_phase_position_scalar, warp_phase_scalar, warped_pulse_edge_scalar};

#[derive(Clone, Copy, Debug)]
pub struct VaOscillator {
    phase: f32,
    resynth_zone: u8,
    resynth_zone_from: u8,
    resynth_zone_fade_remaining: u8,
    rich_timeline_phase: [f32; 2],
    rich_timeline_step: [f32; 2],
    rich_timeline_generation: [u64; 2],
}

impl Default for VaOscillator {
    fn default() -> Self {
        Self {
            phase: 0.0,
            resynth_zone: 0,
            resynth_zone_from: 0,
            resynth_zone_fade_remaining: 0,
            rich_timeline_phase: [0.0; 2],
            rich_timeline_step: [0.0; 2],
            rich_timeline_generation: [0; 2],
        }
    }
}

impl VaOscillator {
    pub const fn reset(&mut self) {
        self.phase = 0.0;
        self.resynth_zone = 0;
        self.resynth_zone_from = 0;
        self.resynth_zone_fade_remaining = 0;
        self.rich_timeline_phase = [0.0; 2];
        self.rich_timeline_step = [0.0; 2];
        self.rich_timeline_generation = [0; 2];
    }

    #[inline]
    pub(crate) fn advance_rich_timeline(
        &mut self,
        layer: usize,
        generation: u64,
        source_frames: u32,
        source_sample_rate: f32,
        host_sample_rate: f32,
    ) -> f32 {
        let layer = layer.min(1);
        if self.rich_timeline_generation[layer] != generation {
            let other = 1 - layer;
            if self.rich_timeline_generation[other] != 0 {
                self.rich_timeline_phase[layer] = self.rich_timeline_phase[other];
            }
            self.rich_timeline_generation[layer] = generation;
            self.rich_timeline_step[layer] =
                source_sample_rate / source_frames.max(1) as f32 / host_sample_rate.max(1.0);
        }
        let phase = self.rich_timeline_phase[layer];
        let next = phase + self.rich_timeline_step[layer];
        self.rich_timeline_phase[layer] = next - next.floor();
        phase
    }

    #[inline]
    pub(crate) fn restart_rich_timeline(&mut self, phase: f32) {
        let phase = wrap_phase_f32(phase);
        self.rich_timeline_phase = [phase; 2];
        self.rich_timeline_step = [0.0; 2];
        self.rich_timeline_generation = [0; 2];
    }

    #[inline]
    pub(crate) fn rich_timeline_for_generation(&self, generation: u64) -> Option<f32> {
        self.rich_timeline_generation
            .iter()
            .position(|candidate| *candidate == generation)
            .map(|layer| self.rich_timeline_phase[layer])
    }

    #[allow(
        clippy::cast_possible_truncation,
        reason = "randomized phase enters the eight-wide f32 oscillator state"
    )]
    pub fn set_phase(&mut self, phase: f64) {
        self.phase = wrap01(phase) as f32;
    }

    #[inline]
    pub(crate) const fn resynth_zone(&self) -> u8 {
        self.resynth_zone
    }

    #[inline]
    pub(crate) const fn resynth_zone_from(&self) -> u8 {
        self.resynth_zone_from
    }

    #[inline]
    pub(crate) const fn resynth_zone_fade_remaining(&self) -> u8 {
        self.resynth_zone_fade_remaining
    }

    #[inline]
    pub(crate) const fn begin_resynth_zone_handover(&mut self, zone: u8, samples: u8) {
        self.resynth_zone_from = self.resynth_zone;
        self.resynth_zone = zone;
        self.resynth_zone_fade_remaining = samples;
    }

    #[inline]
    pub(crate) const fn advance_resynth_zone_handover(&mut self) {
        self.resynth_zone_fade_remaining = self.resynth_zone_fade_remaining.saturating_sub(1);
        if self.resynth_zone_fade_remaining == 0 {
            self.resynth_zone_from = self.resynth_zone;
        }
    }

    #[inline]
    pub(crate) const fn cancel_resynth_zone_handover(&mut self) {
        self.resynth_zone_from = self.resynth_zone;
        self.resynth_zone_fade_remaining = 0;
    }

    #[inline]
    pub(crate) const fn phase(&self) -> f32 {
        self.phase
    }

    #[inline]
    pub(crate) fn advance_phase(&mut self, phase_step: f32) {
        let next_phase = self.phase + phase_step;
        self.phase = if next_phase.is_finite() {
            next_phase - next_phase.floor()
        } else {
            0.0
        };
    }

    #[inline]
    pub(crate) fn offset_phase(&mut self, delta: f32) {
        let phase = self.phase + delta;
        self.phase = if phase < 0.0 {
            phase + 1.0
        } else if phase >= 1.0 {
            phase - 1.0
        } else {
            phase
        };
    }

    #[inline]
    pub(crate) fn offset_phases(oscillators: &mut [Self], delta: f32) {
        let delta8 = f32x8::splat(delta);
        let mut chunks = oscillators.chunks_exact_mut(8);
        for lanes in &mut chunks {
            let phases = f32x8::from(std::array::from_fn(|index| lanes[index].phase)) + delta8;
            let wrapped = phases.cmp_lt(f32x8::ZERO).blend(
                phases + f32x8::ONE,
                phases.cmp_ge(f32x8::ONE).blend(phases - f32x8::ONE, phases),
            );
            let wrapped: [f32; 8] = wrapped.into();
            for (oscillator, phase) in lanes.iter_mut().zip(wrapped) {
                oscillator.phase = phase;
            }
        }
        for oscillator in chunks.into_remainder() {
            oscillator.offset_phase(delta);
        }
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

    pub(crate) fn generate_shape_step_ratio(
        &mut self,
        shape: f32,
        phase_step: f32,
        pulse_width: f32,
        warp_mode: PhaseWarpMode,
        warp_amount: f32,
        ratio_band: (f32, f32),
    ) -> f32 {
        let Some(source) =
            PreparedRatioSource::new(shape, pulse_width, warp_mode, warp_amount, ratio_band)
        else {
            return 0.0;
        };
        self.generate_shape_step_prepared_ratio(phase_step, &source)
    }

    pub(crate) fn generate_shape_step_prepared_ratio(
        &mut self,
        phase_step: f32,
        source: &PreparedRatioSource,
    ) -> f32 {
        let phase = self.phase;
        self.advance_phase(phase_step);
        ratio::sample_prepared_ratio(source, phase, phase_step)
    }

    pub(crate) fn preview_shape_ratio(
        shape: f32,
        phase: f32,
        phase_step: f32,
        pulse_width: f32,
        warp_mode: PhaseWarpMode,
        warp_amount: f32,
        ratio_band: (f32, f32),
    ) -> f32 {
        ratio::sample_shape_ratio(
            shape,
            phase,
            phase_step,
            pulse_width,
            warp_mode,
            warp_amount,
            ratio_band.0,
            ratio_band.1,
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
        // Fourier truncation is valid before phase warping; retain the existing
        // discontinuity-aware warped renderer when a warp is active.
        let antialiasing = if antialiasing.is_one_x()
            && warp_mode != PhaseWarpMode::None
            && warp_amount > f32::EPSILON
        {
            Antialiasing::SplineOptimized
        } else {
            antialiasing
        };

        let pulse_edge = if shape > 2.0 {
            warped_pulse_edge_scalar(phase_step, pulse_width, warp_mode, warp_amount)
        } else {
            None
        };
        self.generate_shape_step_warped_with_edge(
            shape,
            phase_step,
            pulse_width,
            antialiasing,
            pulse_edge,
            |phase| warp_phase_scalar(phase, phase_step, warp_mode, warp_amount),
        )
    }

    fn generate_shape_step_warped_with_edge(
        &mut self,
        shape: f32,
        phase_step: f32,
        pulse_width: f32,
        antialiasing: Antialiasing,
        pulse_edge: Option<f32>,
        warp: impl FnOnce(f32) -> (f32, f32),
    ) -> f32 {
        let raw_phase = self.phase;
        let next_phase = raw_phase + phase_step;
        self.phase = if next_phase >= 1.0 {
            next_phase - 1.0
        } else {
            next_phase
        };
        let (phase, warped_step) = warp(raw_phase);
        sample_shape_normalized_warped_impl(
            shape,
            f64::from(raw_phase),
            f64::from(phase_step),
            f64::from(phase),
            f64::from(warped_step),
            pulse_width,
            antialiasing,
            pulse_edge.map(f64::from),
        )
    }

    pub(crate) fn generate_shape_block_warped<const SAMPLES: usize>(
        &mut self,
        shape: f32,
        phase_step: f32,
        pulse_width: f32,
        antialiasing: Antialiasing,
        warp_mode: PhaseWarpMode,
        warp_amount: f32,
    ) -> [f32; SAMPLES] {
        // Fourier truncation is valid before phase warping; retain the existing
        // discontinuity-aware warped renderer when a warp is active.
        let antialiasing = if antialiasing.is_one_x()
            && warp_mode != PhaseWarpMode::None
            && warp_amount > f32::EPSILON
        {
            Antialiasing::SplineOptimized
        } else {
            antialiasing
        };

        let pulse_edge = warped_pulse_edge_scalar(phase_step, pulse_width, warp_mode, warp_amount);
        std::array::from_fn(|_| {
            self.generate_shape_step_warped_with_edge(
                shape,
                phase_step,
                pulse_width,
                antialiasing,
                pulse_edge,
                |phase| warp_phase_scalar(phase, phase_step, warp_mode, warp_amount),
            )
        })
    }

    #[cfg(test)]
    pub(crate) fn generate_shape_block_warped_unprepared_probe<const SAMPLES: usize>(
        &mut self,
        shape: f32,
        phase_step: f32,
        pulse_width: f32,
        antialiasing: Antialiasing,
        warp_mode: PhaseWarpMode,
        warp_amount: f32,
    ) -> [f32; SAMPLES] {
        // Fourier truncation is valid before phase warping; retain the existing
        // discontinuity-aware warped renderer when a warp is active.
        let antialiasing = if antialiasing.is_one_x()
            && warp_mode != PhaseWarpMode::None
            && warp_amount > f32::EPSILON
        {
            Antialiasing::SplineOptimized
        } else {
            antialiasing
        };

        let pulse_edge = warped_pulse_edge_scalar(phase_step, pulse_width, warp_mode, warp_amount);
        std::array::from_fn(|_| {
            self.generate_shape_step_warped_with_edge(
                shape,
                phase_step,
                pulse_width,
                antialiasing,
                pulse_edge,
                |phase| {
                    warp_phase_scalar_unprepared_probe(phase, phase_step, warp_mode, warp_amount)
                },
            )
        })
    }

    #[cfg(test)]
    pub(crate) fn generate_shape_block_warped_prepared_probe<const SAMPLES: usize>(
        &mut self,
        shape: f32,
        phase_step: f32,
        pulse_width: f32,
        antialiasing: Antialiasing,
        warp_mode: PhaseWarpMode,
        warp_amount: f32,
    ) -> [f32; SAMPLES] {
        // Fourier truncation is valid before phase warping; retain the existing
        // discontinuity-aware warped renderer when a warp is active.
        let antialiasing = if antialiasing.is_one_x()
            && warp_mode != PhaseWarpMode::None
            && warp_amount > f32::EPSILON
        {
            Antialiasing::SplineOptimized
        } else {
            antialiasing
        };

        let pulse_edge = warped_pulse_edge_scalar(phase_step, pulse_width, warp_mode, warp_amount);
        let warp_depth = prepare_scalar_warp_depth(phase_step, warp_mode, warp_amount);
        std::array::from_fn(|_| {
            self.generate_shape_step_warped_with_edge(
                shape,
                phase_step,
                pulse_width,
                antialiasing,
                pulse_edge,
                |phase| {
                    warp_depth.map_or((phase, phase_step), |depth| {
                        warp_phase_scalar_with_depth(phase, phase_step, warp_mode, depth)
                    })
                },
            )
        })
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
        // Fourier truncation is valid before phase warping; retain the existing
        // discontinuity-aware warped renderer when a warp is active.
        let antialiasing = if antialiasing.is_one_x()
            && warp_mode != PhaseWarpMode::None
            && warp_amount > f32::EPSILON
        {
            Antialiasing::SplineOptimized
        } else {
            antialiasing
        };

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
            let canonical = sample_shape_normalized_warped_auto_edge(
                shape,
                f64::from(raw_phase),
                f64::from(phase_step),
                f64::from(phase),
                f64::from(warped_step),
                pulse_width,
                antialiasing,
                warp_mode,
                warp_amount,
            );
            (custom - canonical).mul_add(mix.clamp(0.0, 1.0), canonical)
        }
    }

    pub(crate) fn generate_custom_block<const SAMPLES: usize>(
        &mut self,
        shape: f32,
        phase_step: f32,
        pulse_width: f32,
        antialiasing: Antialiasing,
        warp_mode: PhaseWarpMode,
        warp_amount: f32,
        curve: WaveCurveRt,
        mix: f32,
    ) -> [f32; SAMPLES] {
        // Fourier truncation is valid before phase warping; retain the existing
        // discontinuity-aware warped renderer when a warp is active.
        let antialiasing = if antialiasing.is_one_x()
            && warp_mode != PhaseWarpMode::None
            && warp_amount > f32::EPSILON
        {
            Antialiasing::SplineOptimized
        } else {
            antialiasing
        };

        debug_assert!(mix < 1.0 && (shape == 2.0 || shape == 3.0));
        // A new quality mode must reach its renderer, including the canonical
        // part of a custom-wave blend. Do not collapse its identity to a bool.
        if !antialiasing.supports_precomputed_spline() {
            return std::array::from_fn(|_| {
                self.generate_custom_step(
                    shape,
                    phase_step,
                    pulse_width,
                    antialiasing,
                    warp_mode,
                    warp_amount,
                    curve,
                    mix,
                )
            });
        }
        let raw_step = f64::from(phase_step);
        let active = raw_step > f64::EPSILON;
        let support = 2.0 * raw_step;
        let inverse_step = if active { raw_step.recip() } else { 1.0 };
        let optimized = antialiasing.uses_optimized_spline();
        let pulse_edge = (shape == 3.0)
            .then(|| warped_pulse_edge_scalar(phase_step, pulse_width, warp_mode, warp_amount))
            .flatten()
            .map(f64::from);
        let minimum_width = raw_step.max(0.03);
        let width = f64::from(pulse_width).clamp(minimum_width, 1.0 - minimum_width);
        let mix = mix.clamp(0.0, 1.0);
        std::array::from_fn(|_| {
            let raw_phase = self.phase;
            self.phase = wrap_phase_f32(raw_phase + phase_step);
            let (phase, _) = warp_phase_scalar(raw_phase, phase_step, warp_mode, warp_amount);
            let raw_phase = f64::from(raw_phase);
            let phase64 = f64::from(phase);
            let wrap_correction =
                spline_blep_precomputed_scalar(raw_phase, active, support, inverse_step, optimized);
            let canonical = if shape == 2.0 {
                (2.0_f64.mul_add(phase64, -1.0) - wrap_correction) as f32
            } else {
                let shifted = pulse_edge.map_or_else(
                    || wrap01(phase64 + 1.0 - width),
                    |edge| wrap01(raw_phase + 1.0 - edge),
                );
                let edge_correction = spline_blep_precomputed_scalar(
                    shifted,
                    active,
                    support,
                    inverse_step,
                    optimized,
                );
                let sample = if phase64 < width { 1.0 } else { -1.0 };
                (sample + wrap_correction - edge_correction) as f32
            };
            (curve.eval(phase) - canonical).mul_add(mix, canonical)
        })
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
        // Fourier truncation is valid before phase warping; retain the existing
        // discontinuity-aware warped renderer when a warp is active.
        let antialiasing = if antialiasing.is_one_x()
            && warp_mode != PhaseWarpMode::None
            && warp_amount > f32::EPSILON
        {
            Antialiasing::SplineOptimized
        } else {
            antialiasing
        };

        let raw_phase0 = self.phase;
        let raw_phase1 = wrap_phase_f32(raw_phase0 + phase_steps[0]);
        self.phase = wrap_phase_f32(raw_phase1 + phase_steps[1]);
        let (phase0, step0) = warp_phase_scalar(raw_phase0, phase_steps[0], warp_mode, warp_amount);
        let (phase1, step1) = warp_phase_scalar(raw_phase1, phase_steps[1], warp_mode, warp_amount);
        [
            sample_shape_normalized_warped_auto_edge(
                shape,
                f64::from(raw_phase0),
                f64::from(phase_steps[0]),
                f64::from(phase0),
                f64::from(step0),
                pulse_width,
                antialiasing,
                warp_mode,
                warp_amount,
            ),
            sample_shape_normalized_warped_auto_edge(
                shape,
                f64::from(raw_phase1),
                f64::from(phase_steps[1]),
                f64::from(phase1),
                f64::from(step1),
                pulse_width,
                antialiasing,
                warp_mode,
                warp_amount,
            ),
        ]
    }
}

pub(crate) fn prepare_ratio_filter() {
    ratio::prepare();
}

#[inline]
fn wrap_phase_f32(phase: f32) -> f32 {
    if phase >= 1.0 { phase - 1.0 } else { phase }
}

#[cfg(test)]
mod phase_tests {
    use std::hint::black_box;
    use std::time::Instant;

    use truce_simd::simd::f32x8;

    use super::{Antialiasing, PhaseWarpMode, VaOscillator};

    fn assert_custom_scalar_prepared_blep_identity<const SAMPLES: usize>() {
        let curve = crate::wave_curve::WaveCurveRt::default();
        let warp_cases = [
            (PhaseWarpMode::None, 0.0),
            (PhaseWarpMode::Pwm, 0.000_1),
            (PhaseWarpMode::Pwm, 0.63),
            (PhaseWarpMode::Pwm, 1.0),
            (PhaseWarpMode::PhaseBend, 0.000_1),
            (PhaseWarpMode::PhaseBend, 0.63),
            (PhaseWarpMode::PhaseBend, 1.0),
            (PhaseWarpMode::Harmonic, 0.000_1),
            (PhaseWarpMode::Harmonic, 0.63),
            (PhaseWarpMode::Harmonic, 1.0),
        ];
        for shape in [2.0, 3.0] {
            for step in [0.0, 0.000_01, 440.0 / 48_000.0, 0.249, 0.25, 0.251, 0.45] {
                for width in [0.03, 0.31, 0.5, 0.97] {
                    for antialiasing in [
                        Antialiasing::Legacy,
                        Antialiasing::Spline,
                        Antialiasing::SplineOptimized,
                        Antialiasing::Lagrange,
                        Antialiasing::Spectral,
                    ] {
                        for (warp_mode, warp_amount) in warp_cases {
                            for mix in [0.000_1, 0.63, f32::from_bits(1.0_f32.to_bits() - 1)] {
                                let mut current = VaOscillator::default();
                                let mut candidate = VaOscillator::default();
                                current.set_phase(0.713);
                                candidate.set_phase(0.713);
                                for block in 0..64 {
                                    let expected: [f32; SAMPLES] = std::array::from_fn(|_| {
                                        current.generate_custom_step(
                                            shape,
                                            step,
                                            width,
                                            antialiasing,
                                            warp_mode,
                                            warp_amount,
                                            curve,
                                            mix,
                                        )
                                    });
                                    let actual = candidate.generate_custom_block::<SAMPLES>(
                                        shape,
                                        step,
                                        width,
                                        antialiasing,
                                        warp_mode,
                                        warp_amount,
                                        curve,
                                        mix,
                                    );
                                    for frame in 0..SAMPLES {
                                        assert_eq!(
                                            actual[frame].to_bits(),
                                            expected[frame].to_bits(),
                                            "custom scalar mismatch: samples={SAMPLES}, block={block}, frame={frame}, shape={shape}, step={step}, width={width}, antialiasing={antialiasing:?}, warp={warp_mode:?}, amount={warp_amount}, mix={mix}"
                                        );
                                    }
                                    assert_eq!(
                                        candidate.phase.to_bits(),
                                        current.phase.to_bits(),
                                        "custom scalar phase mismatch: samples={SAMPLES}, block={block}, shape={shape}, step={step}, width={width}, antialiasing={antialiasing:?}, warp={warp_mode:?}, amount={warp_amount}, mix={mix}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    #[ignore = "constant custom scalar prepared BLEP bit-identity experiment"]
    fn custom_scalar_prepared_blep_bit_identity() {
        assert_custom_scalar_prepared_blep_identity::<24>();
        assert_custom_scalar_prepared_blep_identity::<32>();
    }

    fn assert_probe_identity<const SAMPLES: usize>() {
        for shape in [2.001, 2.5, 3.0] {
            for step in [0.000_01, 440.0 / 48_000.0, 0.083, 0.44] {
                for width in [0.03, 0.31, 0.5, 0.97] {
                    for mode in [
                        PhaseWarpMode::Pwm,
                        PhaseWarpMode::PhaseBend,
                        PhaseWarpMode::Harmonic,
                    ] {
                        for amount in [0.000_1, 0.5, 1.0] {
                            for antialiasing in
                                [Antialiasing::Spline, Antialiasing::SplineOptimized]
                            {
                                let mut current = VaOscillator::default();
                                let mut inline = VaOscillator::default();
                                let mut prepared = VaOscillator::default();
                                current.set_phase(0.713);
                                inline.set_phase(0.713);
                                prepared.set_phase(0.713);
                                for _ in 0..64 {
                                    let expected = current
                                        .generate_shape_block_warped_unprepared_probe::<SAMPLES>(
                                            shape,
                                            step,
                                            width,
                                            antialiasing,
                                            mode,
                                            amount,
                                        );
                                    let inline_output = inline
                                        .generate_shape_block_warped::<SAMPLES>(
                                            shape,
                                            step,
                                            width,
                                            antialiasing,
                                            mode,
                                            amount,
                                        );
                                    let actual = prepared
                                        .generate_shape_block_warped_prepared_probe::<SAMPLES>(
                                            shape,
                                            step,
                                            width,
                                            antialiasing,
                                            mode,
                                            amount,
                                        );
                                    for (actual, expected) in actual.into_iter().zip(expected) {
                                        assert_eq!(actual.to_bits(), expected.to_bits());
                                    }
                                    for (actual, expected) in
                                        inline_output.into_iter().zip(expected)
                                    {
                                        assert_eq!(actual.to_bits(), expected.to_bits());
                                    }
                                    assert_eq!(prepared.phase.to_bits(), current.phase.to_bits());
                                    assert_eq!(inline.phase.to_bits(), current.phase.to_bits());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn measure_probe_blocks<const SAMPLES: usize>(
        mut render: impl FnMut() -> [f32; SAMPLES],
        blocks: usize,
        structural: bool,
    ) -> (f64, f32) {
        let mut checksum = 0.0_f32;
        let started = Instant::now();
        for _ in 0..blocks {
            let samples = render();
            if structural {
                let mut left = [f32x8::ZERO; SAMPLES];
                let mut right = [f32x8::ZERO; SAMPLES];
                for frame in 0..SAMPLES {
                    let sample = f32x8::splat(samples[frame] * 0.125);
                    left[frame] += sample;
                    right[frame] += sample;
                }
                checksum += black_box(left[SAMPLES - 1].reduce_add());
                black_box(right[SAMPLES - 1]);
            } else {
                checksum += black_box(samples[SAMPLES - 1]);
            }
        }
        (
            started.elapsed().as_nanos() as f64 / blocks as f64,
            checksum,
        )
    }

    fn report_probe_cpu<const SAMPLES: usize>(shape: f32, mode: PhaseWarpMode) {
        const BLOCKS: usize = 100_000;
        const REPEATS: usize = 5;
        let step = 440.0 / 48_000.0;
        for structural in [false, true] {
            let mut current_times = [0.0; REPEATS];
            let mut inline_times = [0.0; REPEATS];
            let mut prepared_times = [0.0; REPEATS];
            let mut checksum = 0.0_f32;
            for repeat in 0..REPEATS {
                for variant in [repeat % 3, (repeat + 1) % 3, (repeat + 2) % 3] {
                    match variant {
                        0 => {
                            let mut current = VaOscillator::default();
                            let (time, sum) = measure_probe_blocks(
                                || {
                                    current.generate_shape_block_warped_unprepared_probe::<SAMPLES>(
                                        shape,
                                        step,
                                        0.31,
                                        Antialiasing::SplineOptimized,
                                        mode,
                                        1.0,
                                    )
                                },
                                BLOCKS,
                                structural,
                            );
                            current_times[repeat] = time;
                            checksum += sum;
                        }
                        1 => {
                            let mut inline = VaOscillator::default();
                            let (time, sum) = measure_probe_blocks(
                                || {
                                    inline.generate_shape_block_warped::<SAMPLES>(
                                        shape,
                                        step,
                                        0.31,
                                        Antialiasing::SplineOptimized,
                                        mode,
                                        1.0,
                                    )
                                },
                                BLOCKS,
                                structural,
                            );
                            inline_times[repeat] = time;
                            checksum += sum;
                        }
                        _ => {
                            let mut prepared = VaOscillator::default();
                            let (time, sum) = measure_probe_blocks(
                                || {
                                    prepared.generate_shape_block_warped_prepared_probe::<SAMPLES>(
                                        shape,
                                        step,
                                        0.31,
                                        Antialiasing::SplineOptimized,
                                        mode,
                                        1.0,
                                    )
                                },
                                BLOCKS,
                                structural,
                            );
                            prepared_times[repeat] = time;
                            checksum += sum;
                        }
                    }
                }
            }
            current_times.sort_by(f64::total_cmp);
            inline_times.sort_by(f64::total_cmp);
            prepared_times.sort_by(f64::total_cmp);
            let current = current_times[REPEATS / 2];
            let inline = inline_times[REPEATS / 2];
            let prepared = prepared_times[REPEATS / 2];
            println!(
                "prepared_scalar_warp,path={},shape={shape:.3},mode={mode:?},samples={SAMPLES},current_ns_block={current:.3},inline_ns_block={inline:.3},prepared_ns_block={prepared:.3},inline_ratio={:.3},prepared_ratio={:.3},checksum={checksum:.9}",
                if structural { "structural" } else { "legacy" },
                inline / current,
                prepared / current,
            );
        }
    }

    #[test]
    #[ignore = "manual fixed scalar warp preparation identity gate"]
    fn fixed_scalar_warp_preparation_matches_current_bits() {
        assert_probe_identity::<24>();
        assert_probe_identity::<32>();
    }

    #[test]
    #[ignore = "manual release-mode fixed scalar warp preparation CPU experiment"]
    fn fixed_scalar_warp_preparation_cpu_report() {
        for shape in [2.5, 3.0] {
            for mode in [
                PhaseWarpMode::Pwm,
                PhaseWarpMode::PhaseBend,
                PhaseWarpMode::Harmonic,
            ] {
                report_probe_cpu::<24>(shape, mode);
                report_probe_cpu::<32>(shape, mode);
            }
        }
    }

    #[test]
    fn fixed_warped_pulse_blocks_match_scalar_bits() {
        for shape in [2.001, 2.5, 3.0] {
            for step in [0.000_01, 440.0 / 48_000.0, 0.083, 0.44] {
                for width in [0.03, 0.31, 0.5, 0.97] {
                    for mode in [
                        PhaseWarpMode::Pwm,
                        PhaseWarpMode::PhaseBend,
                        PhaseWarpMode::Harmonic,
                    ] {
                        for amount in [0.000_1, 0.5, 1.0] {
                            for antialiasing in
                                [Antialiasing::Spline, Antialiasing::SplineOptimized]
                            {
                                let mut scalar = VaOscillator::default();
                                let mut block = VaOscillator::default();
                                scalar.set_phase(0.713);
                                block.set_phase(0.713);
                                let expected: [f32; 32] = std::array::from_fn(|_| {
                                    scalar.generate_shape_step_warped(
                                        shape,
                                        step,
                                        width,
                                        antialiasing,
                                        mode,
                                        amount,
                                    )
                                });
                                let actual: [f32; 32] = block.generate_shape_block_warped(
                                    shape,
                                    step,
                                    width,
                                    antialiasing,
                                    mode,
                                    amount,
                                );
                                for (actual, expected) in actual.into_iter().zip(expected) {
                                    assert_eq!(actual.to_bits(), expected.to_bits());
                                }
                                assert_eq!(block.phase().to_bits(), scalar.phase().to_bits());
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn scalar_phase_wraps_arbitrary_resynth_steps() {
        let mut oscillator = VaOscillator::default();
        oscillator.advance_phase(65.25);
        assert!((oscillator.phase() - 0.25).abs() < f32::EPSILON);
        for _ in 0..100_000 {
            oscillator.advance_phase(17.125);
            assert!((0.0..1.0).contains(&oscillator.phase()));
        }
    }
}
