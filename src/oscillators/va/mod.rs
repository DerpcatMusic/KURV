//! Fast procedural virtual-analog oscillator.

mod antialias;
mod backend;
mod render;
mod table;
mod warp;

use crate::wave_curve::WaveCurveRt;

use antialias::{
    bandlimited_saw8, cosine_phase4, cosine_phase8, sine_cosine_phase4, sine_cosine_phase8,
    sine_phase4, sine_phase8, spline_blep8_precomputed_static_with_bounds, wrap_phase4,
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
pub use render::{
    accumulate_custom4_block, accumulate_custom4_block_constant, accumulate_custom8_block,
    accumulate_custom8_block_constant, accumulate_saw4_block, accumulate_saw4_block_constant,
    accumulate_saw4_block_dynamic_gains, accumulate_saw4_block_static_gains, accumulate_saw8_block,
    accumulate_saw8_block_dynamic_gains, accumulate_saw8_block_static_gains,
    accumulate_saw8_block_static_gains_narrow_spline, accumulate_shape4_block_constant,
    accumulate_shape4_block_constant_warped, accumulate_shape4_block_dynamic,
    accumulate_shape4_block_morphing, accumulate_shape8_block_constant,
    accumulate_shape8_block_constant_warped, accumulate_shape8_block_dynamic,
    accumulate_shape8_block_morphing, generate_custom4, generate_custom8, generate_pulse4,
    generate_pulse8, generate_saw4, generate_saw8, generate_shape4, generate_shape4_pair,
    generate_shape4_pair_warped, generate_shape4_warped, generate_shape8, generate_shape8_pair,
    generate_shape8_pair_warped, generate_shape8_warped, generate_sine4, generate_sine8,
    generate_triangle4, generate_triangle8, is_narrow_spline_ramp,
    sample_custom_shape_with_antialiasing_warped, shape_morph_gain,
};
#[cfg(test)]
pub use render::{sample_shape, sample_shape_with_antialiasing};
use render::{sample_shape_normalized, sample_shape_normalized_warped_auto_edge};
pub(crate) use table::{MAX_VA_TABLE_FRAMES, VaTableData, VaTableRt, VaTableState};
pub use warp::PhaseWarpMode;
use warp::{warp_phase_position_scalar, warp_phase_scalar};

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
        sample_shape_normalized_warped_auto_edge(
            shape,
            f64::from(raw_phase),
            f64::from(phase_step),
            f64::from(phase),
            f64::from(warped_step),
            pulse_width,
            antialiasing,
            warp_mode,
            warp_amount,
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

#[inline]
fn wrap_phase_f32(phase: f32) -> f32 {
    if phase >= 1.0 { phase - 1.0 } else { phase }
}
