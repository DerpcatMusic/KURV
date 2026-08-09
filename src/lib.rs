use truce::prelude::*;
use truce_core::midi::{norm_7bit, norm_pitch_bend, per_note_bend_semitones};

mod core;
mod diagnostics;
mod editor;
mod editor_controls;
mod editor_envelope;
mod editor_history;
mod editor_lfo;
mod editor_modulation;
mod editor_oscillator;
mod editor_presets;
mod editor_shell;
mod editor_theme;
mod editor_unison;
mod editor_widgets;
mod modulation_target;
mod modulators;
mod oscillators;
mod pan_curve;
mod params;
mod performance;
mod shell;
mod voices;
mod wave_curve;

use core::oversampling::{self, DEFAULT_FACTOR, StereoOversampler};
use modulators::lfo::{
    self, LFO_COUNT, LfoBank, LfoConfig, LfoMode, LfoRateMode, ROUTE_COUNT, RouteConfig,
};
use oscillators::{Antialiasing, PhaseWarpMode};
use pan_curve::{PanShapeCurveData, PanShapeSegmentsRt};
pub(crate) use params::P;
pub use params::{KurvEditorState, KurvParams, KurvParamsParamId};
pub use shell::Kurv;
#[cfg(test)]
use voices::VaVoice;
use voices::{
    BLOCK_INTERNAL_SAMPLES, EnvelopeSettings, FACTOR3_BLOCK_INTERNAL_SAMPLES, InternalRtPool,
    MAX_JOB_SAMPLES, OscillatorSettings, PanShapeSettings, PolySynth, SwarmMode, UnisonSettings,
    VoiceSettings,
};
use wave_curve::WaveCurveRt;

const CONTROL_BLOCK: usize = 1_024;

#[derive(Clone, Copy)]
struct UnisonPitchControlBlock {
    detune_cents: [f32; CONTROL_BLOCK],
    detune_amount: [f32; CONTROL_BLOCK],
    harmonic_align: [f32; CONTROL_BLOCK],
    curve: [f32; CONTROL_BLOCK],
    phase_random: [f32; CONTROL_BLOCK],
    jitter_amount: [f32; CONTROL_BLOCK],
    jitter_rate: [f32; CONTROL_BLOCK],
    stereo: [f32; CONTROL_BLOCK],
    stereo_x: [f32; CONTROL_BLOCK],
    stereo_y: [f32; CONTROL_BLOCK],
    weight: [f32; CONTROL_BLOCK],
    pan_center: [f32; CONTROL_BLOCK],
    pan_left: [f32; CONTROL_BLOCK],
    pan_right: [f32; CONTROL_BLOCK],
    pan_center_x: [f32; CONTROL_BLOCK],
}

impl Default for UnisonPitchControlBlock {
    fn default() -> Self {
        Self {
            detune_cents: [0.0; CONTROL_BLOCK],
            detune_amount: [0.0; CONTROL_BLOCK],
            harmonic_align: [0.0; CONTROL_BLOCK],
            curve: [0.0; CONTROL_BLOCK],
            phase_random: [0.0; CONTROL_BLOCK],
            jitter_amount: [0.0; CONTROL_BLOCK],
            jitter_rate: [0.0; CONTROL_BLOCK],
            stereo: [0.0; CONTROL_BLOCK],
            stereo_x: [0.0; CONTROL_BLOCK],
            stereo_y: [0.0; CONTROL_BLOCK],
            weight: [0.0; CONTROL_BLOCK],
            pan_center: [0.0; CONTROL_BLOCK],
            pan_left: [0.0; CONTROL_BLOCK],
            pan_right: [0.0; CONTROL_BLOCK],
            pan_center_x: [0.0; CONTROL_BLOCK],
        }
    }
}

impl UnisonPitchControlBlock {
    fn is_static(&self, start: usize, len: usize) -> bool {
        let end = start + len;
        slice_is_static(&self.detune_cents[start..end])
            && slice_is_static(&self.detune_amount[start..end])
            && slice_is_static(&self.harmonic_align[start..end])
            && slice_is_static(&self.curve[start..end])
            && slice_is_static(&self.phase_random[start..end])
            && slice_is_static(&self.jitter_amount[start..end])
            && slice_is_static(&self.jitter_rate[start..end])
            && slice_is_static(&self.stereo[start..end])
            && slice_is_static(&self.stereo_x[start..end])
            && slice_is_static(&self.stereo_y[start..end])
            && slice_is_static(&self.weight[start..end])
            && slice_is_static(&self.pan_center[start..end])
            && slice_is_static(&self.pan_left[start..end])
            && slice_is_static(&self.pan_right[start..end])
            && slice_is_static(&self.pan_center_x[start..end])
    }
}

struct ControlBlock {
    shape: [f32; CONTROL_BLOCK],
    pulse_width: [f32; CONTROL_BLOCK],
    osc1_warp_amount: [f32; CONTROL_BLOCK],
    osc1_custom_shape: [f32; CONTROL_BLOCK],
    osc1_cents: [f32; CONTROL_BLOCK],
    osc1_curve_fade: [f32; CONTROL_BLOCK],
    osc1_level: [f32; CONTROL_BLOCK],
    osc1_pan: [f32; CONTROL_BLOCK],
    osc2_shape: [f32; CONTROL_BLOCK],
    osc2_pulse_width: [f32; CONTROL_BLOCK],
    osc2_warp_amount: [f32; CONTROL_BLOCK],
    osc2_custom_shape: [f32; CONTROL_BLOCK],
    osc2_cents: [f32; CONTROL_BLOCK],
    osc2_curve_fade: [f32; CONTROL_BLOCK],
    osc2_level: [f32; CONTROL_BLOCK],
    osc2_pan: [f32; CONTROL_BLOCK],
    osc3_shape: [f32; CONTROL_BLOCK],
    osc3_pulse_width: [f32; CONTROL_BLOCK],
    osc3_warp_amount: [f32; CONTROL_BLOCK],
    osc3_custom_shape: [f32; CONTROL_BLOCK],
    osc3_cents: [f32; CONTROL_BLOCK],
    osc3_curve_fade: [f32; CONTROL_BLOCK],
    osc3_level: [f32; CONTROL_BLOCK],
    osc3_pan: [f32; CONTROL_BLOCK],
    velocity: [f32; CONTROL_BLOCK],
    pressure: [f32; CONTROL_BLOCK],
    timbre: [f32; CONTROL_BLOCK],
    attack: [f32; CONTROL_BLOCK],
    decay: [f32; CONTROL_BLOCK],
    sustain: [f32; CONTROL_BLOCK],
    release: [f32; CONTROL_BLOCK],
    attack_curve: [f32; CONTROL_BLOCK],
    decay_curve: [f32; CONTROL_BLOCK],
    release_curve: [f32; CONTROL_BLOCK],
    attack_curve_time: [f32; CONTROL_BLOCK],
    decay_curve_time: [f32; CONTROL_BLOCK],
    release_curve_time: [f32; CONTROL_BLOCK],
    glide_time: [f32; CONTROL_BLOCK],
    pitch_bend: [f32; CONTROL_BLOCK],
    lfo_rate: [[f32; CONTROL_BLOCK]; LFO_COUNT],
    lfo_phase: [[f32; CONTROL_BLOCK]; LFO_COUNT],
    output_db: [f32; CONTROL_BLOCK],
    unison_pitch: [UnisonPitchControlBlock; 3],
    modulation_amounts: [[f32; CONTROL_BLOCK]; ROUTE_COUNT],
    cached_len: usize,
    cached_oscillator_mask: u8,
    cached_static: bool,
    cached_static_except_shape: bool,
}

#[derive(Clone, Copy, Default)]
struct ActiveRoute {
    amount_index: usize,
    source_index: usize,
    descriptor: Option<modulation_target::TargetDescriptor>,
}

struct ActiveRoutes {
    entries: [ActiveRoute; ROUTE_COUNT],
    len: usize,
    source_mask: u8,
    unison_layout_mask: u8,
    oscillator_mask: u8,
    oscillator_shape_mask: u8,
    unison_frame_mask: u8,
    global_mask: u16,
}

const GLOBAL_OUTPUT_MASK: u16 = 1 << 0;
const GLOBAL_ENVELOPE_MASK: u16 = 1 << 1;
const GLOBAL_VELOCITY_MASK: u16 = 1 << 2;
const GLOBAL_PRESSURE_MASK: u16 = 1 << 3;
const GLOBAL_TIMBRE_MASK: u16 = 1 << 4;
const GLOBAL_GLIDE_MASK: u16 = 1 << 5;

impl Default for ActiveRoutes {
    fn default() -> Self {
        Self {
            entries: [ActiveRoute::default(); ROUTE_COUNT],
            len: 0,
            source_mask: 0,
            unison_layout_mask: 0,
            oscillator_mask: 0,
            oscillator_shape_mask: 0,
            unison_frame_mask: 0,
            global_mask: 0,
        }
    }
}

impl ActiveRoutes {
    fn as_slice(&self) -> &[ActiveRoute] {
        &self.entries[..self.len]
    }
}

fn block_morph_modulation(routes: &ActiveRoutes) -> bool {
    routes.len != 0
        && routes.unison_layout_mask == 0
        && routes.unison_frame_mask == 0
        && routes.global_mask & !GLOBAL_OUTPUT_MASK == 0
        && routes.oscillator_mask == routes.oscillator_shape_mask
        && routes.as_slice().iter().all(|route| {
            matches!(
                route.descriptor,
                Some(modulation_target::TargetDescriptor {
                    kind: modulation_target::TargetKind::Oscillator {
                        control: modulation_target::OscTarget::Shape,
                        ..
                    },
                    ..
                }) | Some(modulation_target::TargetDescriptor {
                    kind: modulation_target::TargetKind::Global(
                        modulation_target::GlobalTarget::Output,
                    ),
                    ..
                })
            )
        })
}

fn block_pitch_modulation(routes: &ActiveRoutes) -> bool {
    routes.len != 0
        && routes.unison_layout_mask == 0
        && routes.global_mask & !GLOBAL_OUTPUT_MASK == 0
        && routes.as_slice().iter().any(|route| {
            matches!(
                route.descriptor,
                Some(modulation_target::TargetDescriptor {
                    kind: modulation_target::TargetKind::Oscillator {
                        control: modulation_target::OscTarget::Pitch,
                        ..
                    },
                    ..
                }) | Some(modulation_target::TargetDescriptor {
                    kind: modulation_target::TargetKind::Unison {
                        control: modulation_target::UnisonTarget::DetuneAmount
                            | modulation_target::UnisonTarget::DetuneRange
                            | modulation_target::UnisonTarget::HarmonicAlign
                            | modulation_target::UnisonTarget::Curve
                            | modulation_target::UnisonTarget::Stereo
                            | modulation_target::UnisonTarget::StereoX
                            | modulation_target::UnisonTarget::StereoY
                            | modulation_target::UnisonTarget::Weight
                            | modulation_target::UnisonTarget::PanCenter
                            | modulation_target::UnisonTarget::PanLeft
                            | modulation_target::UnisonTarget::PanRight
                            | modulation_target::UnisonTarget::PanCenterX,
                        ..
                    },
                    ..
                })
            )
        })
        && routes.as_slice().iter().all(|route| {
            matches!(
                route.descriptor,
                Some(modulation_target::TargetDescriptor {
                    kind: modulation_target::TargetKind::Oscillator {
                        control: modulation_target::OscTarget::Pitch,
                        ..
                    },
                    ..
                }) | Some(modulation_target::TargetDescriptor {
                    kind: modulation_target::TargetKind::Unison {
                        control: modulation_target::UnisonTarget::DetuneAmount
                            | modulation_target::UnisonTarget::DetuneRange
                            | modulation_target::UnisonTarget::HarmonicAlign
                            | modulation_target::UnisonTarget::Curve
                            | modulation_target::UnisonTarget::Stereo
                            | modulation_target::UnisonTarget::StereoX
                            | modulation_target::UnisonTarget::StereoY
                            | modulation_target::UnisonTarget::Weight
                            | modulation_target::UnisonTarget::PanCenter
                            | modulation_target::UnisonTarget::PanLeft
                            | modulation_target::UnisonTarget::PanRight
                            | modulation_target::UnisonTarget::PanCenterX,
                        ..
                    },
                    ..
                }) | Some(modulation_target::TargetDescriptor {
                    kind: modulation_target::TargetKind::Global(
                        modulation_target::GlobalTarget::Output,
                    ),
                    ..
                })
            )
        })
}

fn block_parameter_modulation(routes: &ActiveRoutes) -> bool {
    routes.len != 0
        && routes.unison_layout_mask == 0
        && routes.unison_frame_mask == 0
        && routes.global_mask
            & !(GLOBAL_OUTPUT_MASK
                | GLOBAL_VELOCITY_MASK
                | GLOBAL_PRESSURE_MASK
                | GLOBAL_TIMBRE_MASK
                | GLOBAL_ENVELOPE_MASK)
            == 0
        && routes.as_slice().iter().any(|route| {
            matches!(
                route.descriptor,
                Some(modulation_target::TargetDescriptor {
                    kind: modulation_target::TargetKind::Oscillator { .. },
                    ..
                }) | Some(modulation_target::TargetDescriptor {
                    kind: modulation_target::TargetKind::Global(
                        modulation_target::GlobalTarget::Velocity
                            | modulation_target::GlobalTarget::Pressure
                            | modulation_target::GlobalTarget::Timbre
                            | modulation_target::GlobalTarget::Attack
                            | modulation_target::GlobalTarget::Decay
                            | modulation_target::GlobalTarget::Sustain
                            | modulation_target::GlobalTarget::Release
                            | modulation_target::GlobalTarget::AttackCurve
                            | modulation_target::GlobalTarget::DecayCurve
                            | modulation_target::GlobalTarget::ReleaseCurve
                            | modulation_target::GlobalTarget::AttackCurveTime
                            | modulation_target::GlobalTarget::DecayCurveTime
                            | modulation_target::GlobalTarget::ReleaseCurveTime,
                    ),
                    ..
                })
            )
        })
        && routes.as_slice().iter().all(|route| {
            matches!(
                route.descriptor,
                Some(modulation_target::TargetDescriptor {
                    kind: modulation_target::TargetKind::Oscillator { .. },
                    ..
                }) | Some(modulation_target::TargetDescriptor {
                    kind: modulation_target::TargetKind::Global(
                        modulation_target::GlobalTarget::Output
                            | modulation_target::GlobalTarget::Velocity
                            | modulation_target::GlobalTarget::Pressure
                            | modulation_target::GlobalTarget::Timbre
                            | modulation_target::GlobalTarget::Attack
                            | modulation_target::GlobalTarget::Decay
                            | modulation_target::GlobalTarget::Sustain
                            | modulation_target::GlobalTarget::Release
                            | modulation_target::GlobalTarget::AttackCurve
                            | modulation_target::GlobalTarget::DecayCurve
                            | modulation_target::GlobalTarget::ReleaseCurve
                            | modulation_target::GlobalTarget::AttackCurveTime
                            | modulation_target::GlobalTarget::DecayCurveTime
                            | modulation_target::GlobalTarget::ReleaseCurveTime,
                    ),
                    ..
                })
            )
        })
}

fn block_motion_modulation(routes: &ActiveRoutes) -> bool {
    routes.len != 0
        && routes.unison_layout_mask != 0
        && routes.unison_frame_mask == 0
        && routes.oscillator_mask == 0
        && routes.global_mask & !GLOBAL_OUTPUT_MASK == 0
        && routes.as_slice().iter().all(|route| {
            matches!(
                route.descriptor,
                Some(modulation_target::TargetDescriptor {
                    kind: modulation_target::TargetKind::Unison {
                        control: modulation_target::UnisonTarget::PhaseRandom
                            | modulation_target::UnisonTarget::JitterAmount
                            | modulation_target::UnisonTarget::JitterRate,
                        ..
                    },
                    ..
                }) | Some(modulation_target::TargetDescriptor {
                    kind: modulation_target::TargetKind::Global(
                        modulation_target::GlobalTarget::Output,
                    ),
                    ..
                })
            )
        })
}

impl Default for ControlBlock {
    fn default() -> Self {
        Self {
            shape: [0.0; CONTROL_BLOCK],
            pulse_width: [0.0; CONTROL_BLOCK],
            osc1_warp_amount: [0.0; CONTROL_BLOCK],
            osc1_custom_shape: [0.0; CONTROL_BLOCK],
            osc1_cents: [0.0; CONTROL_BLOCK],
            osc1_curve_fade: [1.0; CONTROL_BLOCK],
            osc1_level: [0.0; CONTROL_BLOCK],
            osc1_pan: [0.0; CONTROL_BLOCK],
            osc2_shape: [0.0; CONTROL_BLOCK],
            osc2_pulse_width: [0.0; CONTROL_BLOCK],
            osc2_warp_amount: [0.0; CONTROL_BLOCK],
            osc2_custom_shape: [0.0; CONTROL_BLOCK],
            osc2_cents: [0.0; CONTROL_BLOCK],
            osc2_curve_fade: [1.0; CONTROL_BLOCK],
            osc2_level: [0.0; CONTROL_BLOCK],
            osc2_pan: [0.0; CONTROL_BLOCK],
            osc3_shape: [0.0; CONTROL_BLOCK],
            osc3_pulse_width: [0.0; CONTROL_BLOCK],
            osc3_warp_amount: [0.0; CONTROL_BLOCK],
            osc3_custom_shape: [0.0; CONTROL_BLOCK],
            osc3_cents: [0.0; CONTROL_BLOCK],
            osc3_curve_fade: [1.0; CONTROL_BLOCK],
            osc3_level: [0.0; CONTROL_BLOCK],
            osc3_pan: [0.0; CONTROL_BLOCK],
            velocity: [0.0; CONTROL_BLOCK],
            pressure: [0.0; CONTROL_BLOCK],
            timbre: [0.0; CONTROL_BLOCK],
            attack: [0.0; CONTROL_BLOCK],
            decay: [0.0; CONTROL_BLOCK],
            sustain: [0.0; CONTROL_BLOCK],
            release: [0.0; CONTROL_BLOCK],
            attack_curve: [0.0; CONTROL_BLOCK],
            decay_curve: [0.0; CONTROL_BLOCK],
            release_curve: [0.0; CONTROL_BLOCK],
            attack_curve_time: [0.0; CONTROL_BLOCK],
            decay_curve_time: [0.0; CONTROL_BLOCK],
            release_curve_time: [0.0; CONTROL_BLOCK],
            glide_time: [0.0; CONTROL_BLOCK],
            pitch_bend: [0.0; CONTROL_BLOCK],
            lfo_rate: [[0.0; CONTROL_BLOCK]; LFO_COUNT],
            lfo_phase: [[0.0; CONTROL_BLOCK]; LFO_COUNT],
            output_db: [0.0; CONTROL_BLOCK],
            unison_pitch: [UnisonPitchControlBlock::default(); 3],
            modulation_amounts: [[0.0; CONTROL_BLOCK]; ROUTE_COUNT],
            cached_len: 0,
            cached_oscillator_mask: 0,
            cached_static: false,
            cached_static_except_shape: false,
        }
    }
}

impl ControlBlock {
    fn read(
        &mut self,
        params: &KurvParams,
        len: usize,
        oscillator_enabled: [bool; 3],
        active_routes: &ActiveRoutes,
        lfo_mask: u8,
    ) -> Option<f32> {
        params.shape.read_into(&mut self.shape[..len]);
        params.pulse_width.read_into(&mut self.pulse_width[..len]);
        params
            .osc1_warp_amount
            .read_into(&mut self.osc1_warp_amount[..len]);
        params
            .osc1_custom_shape
            .read_into(&mut self.osc1_custom_shape[..len]);
        params.osc1_cents.read_into(&mut self.osc1_cents[..len]);
        params.osc1_level.read_into(&mut self.osc1_level[..len]);
        params.osc1_pan.read_into(&mut self.osc1_pan[..len]);
        if oscillator_enabled[1] {
            params.osc2_shape.read_into(&mut self.osc2_shape[..len]);
            params
                .osc2_pulse_width
                .read_into(&mut self.osc2_pulse_width[..len]);
            params
                .osc2_warp_amount
                .read_into(&mut self.osc2_warp_amount[..len]);
            params
                .osc2_custom_shape
                .read_into(&mut self.osc2_custom_shape[..len]);
            params.osc2_cents.read_into(&mut self.osc2_cents[..len]);
            params.osc2_level.read_into(&mut self.osc2_level[..len]);
            params.osc2_pan.read_into(&mut self.osc2_pan[..len]);
        }
        if oscillator_enabled[2] {
            params.osc3_shape.read_into(&mut self.osc3_shape[..len]);
            params
                .osc3_pulse_width
                .read_into(&mut self.osc3_pulse_width[..len]);
            params
                .osc3_warp_amount
                .read_into(&mut self.osc3_warp_amount[..len]);
            params
                .osc3_custom_shape
                .read_into(&mut self.osc3_custom_shape[..len]);
            params.osc3_cents.read_into(&mut self.osc3_cents[..len]);
            params.osc3_level.read_into(&mut self.osc3_level[..len]);
            params.osc3_pan.read_into(&mut self.osc3_pan[..len]);
        }
        params.velocity_amount.read_into(&mut self.velocity[..len]);
        params.pressure_amount.read_into(&mut self.pressure[..len]);
        params.timbre_amount.read_into(&mut self.timbre[..len]);
        params.attack.read_into(&mut self.attack[..len]);
        params.decay.read_into(&mut self.decay[..len]);
        params.sustain.read_into(&mut self.sustain[..len]);
        params.release.read_into(&mut self.release[..len]);
        params.attack_curve.read_into(&mut self.attack_curve[..len]);
        params.decay_curve.read_into(&mut self.decay_curve[..len]);
        params
            .release_curve
            .read_into(&mut self.release_curve[..len]);
        params
            .attack_curve_time
            .read_into(&mut self.attack_curve_time[..len]);
        params
            .decay_curve_time
            .read_into(&mut self.decay_curve_time[..len]);
        params
            .release_curve_time
            .read_into(&mut self.release_curve_time[..len]);
        params.glide_time.read_into(&mut self.glide_time[..len]);
        params.pitch_bend.read_into(&mut self.pitch_bend[..len]);
        params.output_db.read_into(&mut self.output_db[..len]);
        let unison_pitch_params = [
            (
                &params.unison_detune,
                &params.unison_detune_amount,
                &params.unison_harmonic_align,
                &params.unison_curve,
                &params.phase_random,
                &params.unison_swarm,
                &params.unison_swarm_rate,
                &params.unison_stereo,
                &params.stereo_x,
                &params.stereo_alternate,
                &params.unison_weight,
                &params.pan_shape_center,
                &params.pan_shape_left,
                &params.pan_shape_right,
                &params.pan_shape_center_x,
            ),
            (
                &params.osc2_unison_detune,
                &params.osc2_unison_detune_amount,
                &params.osc2_unison_harmonic_align,
                &params.osc2_unison_curve,
                &params.osc2_phase_random,
                &params.osc2_unison_jitter,
                &params.osc2_unison_jitter_rate,
                &params.osc2_unison_stereo,
                &params.osc2_stereo_x,
                &params.osc2_stereo_alternate,
                &params.osc2_unison_weight,
                &params.osc2_pan_shape_center,
                &params.osc2_pan_shape_left,
                &params.osc2_pan_shape_right,
                &params.osc2_pan_shape_center_x,
            ),
            (
                &params.osc3_unison_detune,
                &params.osc3_unison_detune_amount,
                &params.osc3_unison_harmonic_align,
                &params.osc3_unison_curve,
                &params.osc3_phase_random,
                &params.osc3_unison_jitter,
                &params.osc3_unison_jitter_rate,
                &params.osc3_unison_stereo,
                &params.osc3_stereo_x,
                &params.osc3_stereo_alternate,
                &params.osc3_unison_weight,
                &params.osc3_pan_shape_center,
                &params.osc3_pan_shape_left,
                &params.osc3_pan_shape_right,
                &params.osc3_pan_shape_center_x,
            ),
        ];
        for (
            control,
            (
                detune,
                amount,
                align,
                curve,
                phase_random,
                jitter_amount,
                jitter_rate,
                stereo,
                stereo_x,
                stereo_y,
                weight,
                pan_center,
                pan_left,
                pan_right,
                pan_center_x,
            ),
        ) in self.unison_pitch.iter_mut().zip(unison_pitch_params)
        {
            detune.read_into(&mut control.detune_cents[..len]);
            amount.read_into(&mut control.detune_amount[..len]);
            align.read_into(&mut control.harmonic_align[..len]);
            curve.read_into(&mut control.curve[..len]);
            phase_random.read_into(&mut control.phase_random[..len]);
            jitter_amount.read_into(&mut control.jitter_amount[..len]);
            jitter_rate.read_into(&mut control.jitter_rate[..len]);
            stereo.read_into(&mut control.stereo[..len]);
            stereo_x.read_into(&mut control.stereo_x[..len]);
            stereo_y.read_into(&mut control.stereo_y[..len]);
            weight.read_into(&mut control.weight[..len]);
            pan_center.read_into(&mut control.pan_center[..len]);
            pan_left.read_into(&mut control.pan_left[..len]);
            pan_right.read_into(&mut control.pan_right[..len]);
            pan_center_x.read_into(&mut control.pan_center_x[..len]);
        }
        if lfo_mask != 0 {
            let lfo_params = [
                (&params.lfo1_rate, &params.lfo1_phase),
                (&params.lfo2_rate, &params.lfo2_phase),
                (&params.lfo3_rate, &params.lfo3_phase),
                (&params.lfo4_rate, &params.lfo4_phase),
                (&params.lfo5_rate, &params.lfo5_phase),
                (&params.lfo6_rate, &params.lfo6_phase),
                (&params.lfo7_rate, &params.lfo7_phase),
                (&params.lfo8_rate, &params.lfo8_phase),
            ];
            for (index, (rate, phase)) in lfo_params.into_iter().enumerate() {
                if lfo_mask & (1 << index) == 0 {
                    continue;
                }
                rate.read_into(&mut self.lfo_rate[index][..len]);
                phase.read_into(&mut self.lfo_phase[index][..len]);
            }
        }
        let amount_params = [
            &params.mod1_amount,
            &params.mod2_amount,
            &params.mod3_amount,
            &params.mod4_amount,
            &params.mod5_amount,
            &params.mod6_amount,
            &params.mod7_amount,
            &params.mod8_amount,
            &params.mod9_amount,
            &params.mod10_amount,
            &params.mod11_amount,
            &params.mod12_amount,
            &params.mod13_amount,
            &params.mod14_amount,
            &params.mod15_amount,
            &params.mod16_amount,
        ];
        for route in active_routes.as_slice() {
            let index = route.amount_index;
            amount_params[index].read_into(&mut self.modulation_amounts[index][..len]);
        }
        let oscillator_mask = oscillator_enabled_mask(oscillator_enabled);
        self.cached_len = len;
        self.cached_oscillator_mask = oscillator_mask;
        self.cached_static = self.compute_is_static(0, len, oscillator_enabled);
        self.cached_static_except_shape =
            self.cached_static || self.compute_is_static_except_shape(0, len, oscillator_enabled);
        (self.output_db[0].to_bits() == self.output_db[len - 1].to_bits())
            .then(|| db_to_linear(self.output_db[0]))
    }

    fn active_lfo_mask(&self, routes: &ActiveRoutes, len: usize) -> u8 {
        routes.as_slice().iter().fold(0, |mask, route| {
            if self.modulation_amounts[route.amount_index][..len]
                .iter()
                .any(|amount| amount.abs() > f32::EPSILON)
            {
                mask | (1_u8 << route.source_index)
            } else {
                mask
            }
        })
    }

    fn lfo_control_dynamic_mask(
        &self,
        mask: u8,
        len: usize,
        configs: &[LfoConfig; LFO_COUNT],
    ) -> u8 {
        let mut dynamic = 0;
        for index in 0..LFO_COUNT {
            if mask & (1 << index) == 0 {
                continue;
            }
            let rate = &self.lfo_rate[index][..len];
            let phase = &self.lfo_phase[index][..len];
            if !slice_is_static(rate)
                || !slice_is_static(phase)
                || rate[0].to_bits() != configs[index].rate_hz.to_bits()
                || phase[0].to_bits() != configs[index].phase_offset.to_bits()
            {
                dynamic |= 1 << index;
            }
        }
        dynamic
    }

    fn unison_pitch_active_mask(&self, len: usize, base_unison: &[UnisonSettings; 3]) -> u8 {
        let mut active = 0;
        for (oscillator, (control, base)) in self.unison_pitch.iter().zip(base_unison).enumerate() {
            let static_control = control.detune_cents[..len]
                .iter()
                .all(|value| value.to_bits() == (base.detune_cents() / 100.0).to_bits())
                && control.detune_amount[..len]
                    .iter()
                    .all(|value| value.to_bits() == base.detune_amount().to_bits())
                && control.harmonic_align[..len]
                    .iter()
                    .all(|value| value.to_bits() == base.harmonic_align().to_bits())
                && control.curve[..len]
                    .iter()
                    .all(|value| value.to_bits() == base.curve().to_bits())
                && control.stereo[..len]
                    .iter()
                    .all(|value| value.to_bits() == base.stereo().to_bits())
                && control.stereo_x[..len]
                    .iter()
                    .all(|value| value.to_bits() == base.stereo_x().to_bits())
                && control.stereo_y[..len]
                    .iter()
                    .all(|value| value.to_bits() == base.stereo_alternate().to_bits())
                && control.weight[..len]
                    .iter()
                    .all(|value| value.to_bits() == base.level_curve().to_bits())
                && control.pan_center[..len]
                    .iter()
                    .all(|value| value.to_bits() == base.pan_shape().center.to_bits())
                && control.pan_left[..len]
                    .iter()
                    .all(|value| value.to_bits() == base.pan_shape().left_edge.to_bits())
                && control.pan_right[..len]
                    .iter()
                    .all(|value| value.to_bits() == base.pan_shape().right_edge.to_bits())
                && control.pan_center_x[..len]
                    .iter()
                    .all(|value| value.to_bits() == base.pan_shape().center_x.to_bits());
            if !static_control {
                active |= 1 << oscillator;
            }
        }
        active
    }

    fn unison_motion_active_mask(&self, len: usize, base_unison: &[UnisonSettings; 3]) -> u8 {
        let mut active = 0;
        for (oscillator, (control, base)) in self.unison_pitch.iter().zip(base_unison).enumerate() {
            let static_control = control.phase_random[..len]
                .iter()
                .all(|value| value.to_bits() == base.phase_random().to_bits())
                && control.jitter_amount[..len]
                    .iter()
                    .all(|value| value.to_bits() == base.swarm_amount().to_bits())
                && control.jitter_rate[..len]
                    .iter()
                    .all(|value| value.to_bits() == base.swarm_rate().to_bits());
            if !static_control {
                active |= 1 << oscillator;
            }
        }
        active
    }

    fn envelope_is_static(&self, start: usize, len: usize) -> bool {
        let end = start + len;
        [
            &self.attack[start..end],
            &self.decay[start..end],
            &self.sustain[start..end],
            &self.release[start..end],
            &self.attack_curve[start..end],
            &self.decay_curve[start..end],
            &self.release_curve[start..end],
            &self.attack_curve_time[start..end],
            &self.decay_curve_time[start..end],
            &self.release_curve_time[start..end],
            &self.glide_time[start..end],
            &self.pitch_bend[start..end],
        ]
        .into_iter()
        .all(slice_is_static)
    }

    fn is_static(&self, start: usize, len: usize, oscillator_enabled: [bool; 3]) -> bool {
        if self.cached_static && self.cached_range_matches(start, len, oscillator_enabled) {
            return true;
        }
        self.compute_is_static(start, len, oscillator_enabled)
    }

    fn compute_is_static(&self, start: usize, len: usize, oscillator_enabled: [bool; 3]) -> bool {
        let end = start + len;
        let primary_static = [
            &self.shape[start..end],
            &self.pulse_width[start..end],
            &self.osc1_warp_amount[start..end],
            &self.osc1_custom_shape[start..end],
            &self.osc1_cents[start..end],
            &self.osc1_curve_fade[start..end],
            &self.osc1_level[start..end],
            &self.osc1_pan[start..end],
            &self.velocity[start..end],
            &self.pressure[start..end],
            &self.timbre[start..end],
            &self.glide_time[start..end],
            &self.pitch_bend[start..end],
            &self.output_db[start..end],
        ]
        .into_iter()
        .all(|values| {
            let bits = values[0].to_bits();
            values[1..].iter().all(|value| value.to_bits() == bits)
        });
        primary_static
            && self.envelope_is_static(start, len)
            && self
                .unison_pitch
                .iter()
                .all(|control| control.is_static(start, len))
            && (!oscillator_enabled[1]
                || [
                    &self.osc2_shape[start..end],
                    &self.osc2_pulse_width[start..end],
                    &self.osc2_warp_amount[start..end],
                    &self.osc2_custom_shape[start..end],
                    &self.osc2_cents[start..end],
                    &self.osc2_curve_fade[start..end],
                    &self.osc2_level[start..end],
                    &self.osc2_pan[start..end],
                ]
                .into_iter()
                .all(slice_is_static))
            && (!oscillator_enabled[2]
                || [
                    &self.osc3_shape[start..end],
                    &self.osc3_pulse_width[start..end],
                    &self.osc3_warp_amount[start..end],
                    &self.osc3_custom_shape[start..end],
                    &self.osc3_cents[start..end],
                    &self.osc3_curve_fade[start..end],
                    &self.osc3_level[start..end],
                    &self.osc3_pan[start..end],
                ]
                .into_iter()
                .all(slice_is_static))
    }

    fn is_static_except_shape(
        &self,
        start: usize,
        len: usize,
        oscillator_enabled: [bool; 3],
    ) -> bool {
        if self.cached_static_except_shape
            && self.cached_range_matches(start, len, oscillator_enabled)
        {
            return true;
        }
        self.compute_is_static_except_shape(start, len, oscillator_enabled)
    }

    fn compute_is_static_except_shape(
        &self,
        start: usize,
        len: usize,
        oscillator_enabled: [bool; 3],
    ) -> bool {
        let end = start + len;
        [
            &self.pulse_width[start..end],
            &self.osc1_warp_amount[start..end],
            &self.osc1_custom_shape[start..end],
            &self.osc1_cents[start..end],
            &self.osc1_curve_fade[start..end],
            &self.osc1_level[start..end],
            &self.osc1_pan[start..end],
            &self.velocity[start..end],
            &self.pressure[start..end],
            &self.timbre[start..end],
            &self.glide_time[start..end],
            &self.pitch_bend[start..end],
            &self.output_db[start..end],
        ]
        .into_iter()
        .all(slice_is_static)
            && self.envelope_is_static(start, len)
            && self
                .unison_pitch
                .iter()
                .all(|control| control.is_static(start, len))
            && (!oscillator_enabled[1]
                || [
                    &self.osc2_pulse_width[start..end],
                    &self.osc2_warp_amount[start..end],
                    &self.osc2_custom_shape[start..end],
                    &self.osc2_cents[start..end],
                    &self.osc2_curve_fade[start..end],
                    &self.osc2_level[start..end],
                    &self.osc2_pan[start..end],
                ]
                .into_iter()
                .all(slice_is_static))
            && (!oscillator_enabled[2]
                || [
                    &self.osc3_pulse_width[start..end],
                    &self.osc3_warp_amount[start..end],
                    &self.osc3_custom_shape[start..end],
                    &self.osc3_cents[start..end],
                    &self.osc3_curve_fade[start..end],
                    &self.osc3_level[start..end],
                    &self.osc3_pan[start..end],
                ]
                .into_iter()
                .all(slice_is_static))
    }

    fn cached_range_matches(
        &self,
        start: usize,
        len: usize,
        oscillator_enabled: [bool; 3],
    ) -> bool {
        let oscillator_mask = oscillator_enabled_mask(oscillator_enabled);
        self.cached_len != 0
            && self.cached_oscillator_mask == oscillator_mask
            && start <= self.cached_len
            && len <= self.cached_len - start
    }

    fn expanded_shapes(
        &self,
        start: usize,
        host_frames: usize,
        factor: usize,
    ) -> [[f32; MAX_JOB_SAMPLES]; 3] {
        let controls = [&self.shape, &self.osc2_shape, &self.osc3_shape];
        std::array::from_fn(|oscillator| {
            let mut output = [0.0; MAX_JOB_SAMPLES];
            for frame in 0..host_frames {
                output[frame * factor..(frame + 1) * factor]
                    .fill(controls[oscillator][start + frame]);
            }
            output
        })
    }
}

#[inline]
fn oscillator_enabled_mask(enabled: [bool; 3]) -> u8 {
    u8::from(enabled[0]) | (u8::from(enabled[1]) << 1) | (u8::from(enabled[2]) << 2)
}

fn slice_is_static(values: &[f32]) -> bool {
    let bits = values[0].to_bits();
    values[1..].iter().all(|value| value.to_bits() == bits)
}

fn generator_configuration(params: &KurvParams) -> (u8, Antialiasing) {
    let factor = params.oversampling.value_u8().clamp(1, 4);
    (factor, Antialiasing::Spline.for_factor(factor))
}

fn lfo_configuration(params: &KurvParams) -> [LfoConfig; LFO_COUNT] {
    [
        LfoConfig {
            rate_hz: params.lfo1_rate.value(),
            rate_mode: LfoRateMode::from_index(params.lfo1_rate_mode.value_u8()),
            mode: LfoMode::from_index(params.lfo1_mode.value_u8()),
            phase_offset: params.lfo1_phase.value(),
            sync_division: params.lfo1_sync.value_u8(),
            bipolar: params.lfo1_bipolar.value(),
        },
        LfoConfig {
            rate_hz: params.lfo2_rate.value(),
            rate_mode: LfoRateMode::from_index(params.lfo2_rate_mode.value_u8()),
            mode: LfoMode::from_index(params.lfo2_mode.value_u8()),
            phase_offset: params.lfo2_phase.value(),
            sync_division: params.lfo2_sync.value_u8(),
            bipolar: params.lfo2_bipolar.value(),
        },
        LfoConfig {
            rate_hz: params.lfo3_rate.value(),
            rate_mode: LfoRateMode::from_index(params.lfo3_rate_mode.value_u8()),
            mode: LfoMode::from_index(params.lfo3_mode.value_u8()),
            phase_offset: params.lfo3_phase.value(),
            sync_division: params.lfo3_sync.value_u8(),
            bipolar: params.lfo3_bipolar.value(),
        },
        LfoConfig {
            rate_hz: params.lfo4_rate.value(),
            rate_mode: LfoRateMode::from_index(params.lfo4_rate_mode.value_u8()),
            mode: LfoMode::from_index(params.lfo4_mode.value_u8()),
            phase_offset: params.lfo4_phase.value(),
            sync_division: params.lfo4_sync.value_u8(),
            bipolar: params.lfo4_bipolar.value(),
        },
        LfoConfig {
            rate_hz: params.lfo5_rate.value(),
            rate_mode: LfoRateMode::from_index(params.lfo5_rate_mode.value_u8()),
            mode: LfoMode::from_index(params.lfo5_mode.value_u8()),
            phase_offset: params.lfo5_phase.value(),
            sync_division: params.lfo5_sync.value_u8(),
            bipolar: params.lfo5_bipolar.value(),
        },
        LfoConfig {
            rate_hz: params.lfo6_rate.value(),
            rate_mode: LfoRateMode::from_index(params.lfo6_rate_mode.value_u8()),
            mode: LfoMode::from_index(params.lfo6_mode.value_u8()),
            phase_offset: params.lfo6_phase.value(),
            sync_division: params.lfo6_sync.value_u8(),
            bipolar: params.lfo6_bipolar.value(),
        },
        LfoConfig {
            rate_hz: params.lfo7_rate.value(),
            rate_mode: LfoRateMode::from_index(params.lfo7_rate_mode.value_u8()),
            mode: LfoMode::from_index(params.lfo7_mode.value_u8()),
            phase_offset: params.lfo7_phase.value(),
            sync_division: params.lfo7_sync.value_u8(),
            bipolar: params.lfo7_bipolar.value(),
        },
        LfoConfig {
            rate_hz: params.lfo8_rate.value(),
            rate_mode: LfoRateMode::from_index(params.lfo8_rate_mode.value_u8()),
            mode: LfoMode::from_index(params.lfo8_mode.value_u8()),
            phase_offset: params.lfo8_phase.value(),
            sync_division: params.lfo8_sync.value_u8(),
            bipolar: params.lfo8_bipolar.value(),
        },
    ]
}

fn configured_lfo_mask(params: &KurvParams) -> u8 {
    [
        params.lfo1_active.value(),
        params.lfo2_active.value(),
        params.lfo3_active.value(),
        params.lfo4_active.value(),
        params.lfo5_active.value(),
        params.lfo6_active.value(),
        params.lfo7_active.value(),
        params.lfo8_active.value(),
    ]
    .into_iter()
    .enumerate()
    .fold(0, |mask, (index, active)| {
        mask | if active { 1 << index } else { 0 }
    })
}

fn modulation_routes(params: &KurvParams) -> [RouteConfig; ROUTE_COUNT] {
    [
        RouteConfig {
            source: params.mod1_source.value_u8(),
            target: resolved_modulation_target(
                params.mod1_target.value_u8(),
                params.mod1_target_ext.value_u8(),
            ),
        },
        RouteConfig {
            source: params.mod2_source.value_u8(),
            target: resolved_modulation_target(
                params.mod2_target.value_u8(),
                params.mod2_target_ext.value_u8(),
            ),
        },
        RouteConfig {
            source: params.mod3_source.value_u8(),
            target: resolved_modulation_target(
                params.mod3_target.value_u8(),
                params.mod3_target_ext.value_u8(),
            ),
        },
        RouteConfig {
            source: params.mod4_source.value_u8(),
            target: resolved_modulation_target(
                params.mod4_target.value_u8(),
                params.mod4_target_ext.value_u8(),
            ),
        },
        RouteConfig {
            source: params.mod5_source.value_u8(),
            target: resolved_modulation_target(
                params.mod5_target.value_u8(),
                params.mod5_target_ext.value_u8(),
            ),
        },
        RouteConfig {
            source: params.mod6_source.value_u8(),
            target: resolved_modulation_target(
                params.mod6_target.value_u8(),
                params.mod6_target_ext.value_u8(),
            ),
        },
        RouteConfig {
            source: params.mod7_source.value_u8(),
            target: resolved_modulation_target(
                params.mod7_target.value_u8(),
                params.mod7_target_ext.value_u8(),
            ),
        },
        RouteConfig {
            source: params.mod8_source.value_u8(),
            target: resolved_modulation_target(
                params.mod8_target.value_u8(),
                params.mod8_target_ext.value_u8(),
            ),
        },
        RouteConfig {
            source: params.mod9_source.value_u8(),
            target: resolved_modulation_target(
                params.mod9_target.value_u8(),
                params.mod9_target_ext.value_u8(),
            ),
        },
        RouteConfig {
            source: params.mod10_source.value_u8(),
            target: resolved_modulation_target(
                params.mod10_target.value_u8(),
                params.mod10_target_ext.value_u8(),
            ),
        },
        RouteConfig {
            source: params.mod11_source.value_u8(),
            target: resolved_modulation_target(
                params.mod11_target.value_u8(),
                params.mod11_target_ext.value_u8(),
            ),
        },
        RouteConfig {
            source: params.mod12_source.value_u8(),
            target: resolved_modulation_target(
                params.mod12_target.value_u8(),
                params.mod12_target_ext.value_u8(),
            ),
        },
        RouteConfig {
            source: params.mod13_source.value_u8(),
            target: resolved_modulation_target(
                params.mod13_target.value_u8(),
                params.mod13_target_ext.value_u8(),
            ),
        },
        RouteConfig {
            source: params.mod14_source.value_u8(),
            target: resolved_modulation_target(
                params.mod14_target.value_u8(),
                params.mod14_target_ext.value_u8(),
            ),
        },
        RouteConfig {
            source: params.mod15_source.value_u8(),
            target: resolved_modulation_target(
                params.mod15_target.value_u8(),
                params.mod15_target_ext.value_u8(),
            ),
        },
        RouteConfig {
            source: params.mod16_source.value_u8(),
            target: resolved_modulation_target(
                params.mod16_target.value_u8(),
                params.mod16_target_ext.value_u8(),
            ),
        },
    ]
}

const fn resolved_modulation_target(legacy: u8, extended: u8) -> u8 {
    if extended == 0 {
        legacy
    } else {
        modulation_target::LEGACY_TARGET_COUNT + extended
    }
}

fn active_modulation_routes(
    routes: &[RouteConfig; ROUTE_COUNT],
    oscillator_enabled: [bool; 3],
) -> ActiveRoutes {
    let mut active = ActiveRoutes::default();
    for (index, route) in routes.iter().copied().enumerate() {
        let enabled = modulation_target::target_oscillator(route.target)
            .is_none_or(|oscillator| oscillator_enabled[oscillator]);
        if (1..=8).contains(&route.source) && route.target != 0 && enabled {
            let descriptor = modulation_target::descriptor(route.target);
            active.entries[active.len] = ActiveRoute {
                amount_index: index,
                source_index: usize::from(route.source - 1),
                descriptor: descriptor.copied(),
            };
            active.len += 1;
            active.source_mask |= 1_u8 << (route.source - 1);
            if let Some(descriptor) = descriptor {
                match descriptor.kind {
                    modulation_target::TargetKind::Oscillator {
                        oscillator,
                        control,
                    } => {
                        active.oscillator_mask |= 1 << oscillator;
                        if matches!(control, modulation_target::OscTarget::Shape) {
                            active.oscillator_shape_mask |= 1 << oscillator;
                        }
                    }
                    modulation_target::TargetKind::Unison {
                        oscillator,
                        control,
                    } => {
                        if matches!(
                            control,
                            modulation_target::UnisonTarget::DetuneAmount
                                | modulation_target::UnisonTarget::DetuneRange
                                | modulation_target::UnisonTarget::HarmonicAlign
                                | modulation_target::UnisonTarget::Stereo
                                | modulation_target::UnisonTarget::Curve
                                | modulation_target::UnisonTarget::StereoX
                                | modulation_target::UnisonTarget::StereoY
                                | modulation_target::UnisonTarget::Weight
                                | modulation_target::UnisonTarget::PanCenter
                                | modulation_target::UnisonTarget::PanLeft
                                | modulation_target::UnisonTarget::PanRight
                                | modulation_target::UnisonTarget::PanCenterX
                        ) {
                            active.unison_frame_mask |= 1 << oscillator;
                        } else {
                            active.unison_layout_mask |= 1 << oscillator;
                        }
                    }
                    modulation_target::TargetKind::Global(control) => {
                        active.global_mask |= match control {
                            modulation_target::GlobalTarget::Output => GLOBAL_OUTPUT_MASK,
                            modulation_target::GlobalTarget::Attack
                            | modulation_target::GlobalTarget::Decay
                            | modulation_target::GlobalTarget::Sustain
                            | modulation_target::GlobalTarget::Release
                            | modulation_target::GlobalTarget::AttackCurve
                            | modulation_target::GlobalTarget::DecayCurve
                            | modulation_target::GlobalTarget::ReleaseCurve
                            | modulation_target::GlobalTarget::AttackCurveTime
                            | modulation_target::GlobalTarget::DecayCurveTime
                            | modulation_target::GlobalTarget::ReleaseCurveTime => {
                                GLOBAL_ENVELOPE_MASK
                            }
                            modulation_target::GlobalTarget::Velocity => GLOBAL_VELOCITY_MASK,
                            modulation_target::GlobalTarget::Pressure => GLOBAL_PRESSURE_MASK,
                            modulation_target::GlobalTarget::Timbre => GLOBAL_TIMBRE_MASK,
                            modulation_target::GlobalTarget::Glide => GLOBAL_GLIDE_MASK,
                        };
                    }
                }
            }
        }
    }
    active
}

pub(crate) fn pan_shape_settings(params: &KurvParams) -> PanShapeSettings {
    let legacy_edge = params.pan_shape_edge.value();
    let legacy_curve = params.pan_shape_curve.value();
    let left_edge = params.pan_shape_left.value();
    let right_edge = params.pan_shape_right.value();
    let use_legacy_edges = (left_edge - 1.0).abs() <= f32::EPSILON
        && (right_edge - 1.0).abs() <= f32::EPSILON
        && (legacy_edge - 1.0).abs() > f32::EPSILON;
    let left_curve = params.pan_shape_left_curve.value();
    let right_curve = params.pan_shape_right_curve.value();
    let use_legacy_curve = left_curve.abs() <= f32::EPSILON
        && right_curve.abs() <= f32::EPSILON
        && legacy_curve.abs() > f32::EPSILON;
    let legacy_time = params.pan_shape_curve_time.value();
    let left_time = params.pan_shape_left_curve_time.value();
    let right_time = params.pan_shape_right_curve_time.value();
    let use_legacy_time = (left_time - 0.5).abs() <= f32::EPSILON
        && (right_time - 0.5).abs() <= f32::EPSILON
        && (legacy_time - 0.5).abs() > f32::EPSILON;
    let data = if params.pan_shape_curve_state.is_initialized() {
        params.pan_shape_curve_state.snapshot()
    } else {
        PanShapeCurveData::from_legacy(
            params.pan_shape_center.value(),
            if use_legacy_edges {
                legacy_edge
            } else {
                left_edge
            },
            if use_legacy_edges {
                legacy_edge
            } else {
                right_edge
            },
            if use_legacy_curve {
                legacy_curve
            } else {
                left_curve
            },
            if use_legacy_curve {
                legacy_curve
            } else {
                right_curve
            },
            if use_legacy_time {
                legacy_time
            } else {
                left_time
            },
            if use_legacy_time {
                legacy_time
            } else {
                right_time
            },
        )
    };
    let center = data
        .left
        .knots
        .first()
        .map_or(params.pan_shape_center.value(), |knot| knot.out_lin);
    let left_edge = data
        .left
        .knots
        .last()
        .map_or(left_edge, |knot| knot.out_lin);
    let right_edge = data
        .right
        .knots
        .last()
        .map_or(right_edge, |knot| knot.out_lin);
    PanShapeSettings::new(center, legacy_edge, legacy_curve)
        .with_center_x(params.pan_shape_center_x.value())
        .with_sides(left_edge, right_edge, left_curve, right_curve)
        .with_curve_times(
            if use_legacy_time {
                legacy_time
            } else {
                left_time
            },
            if use_legacy_time {
                legacy_time
            } else {
                right_time
            },
        )
        .with_curve_data(&data)
}

#[allow(
    clippy::too_many_arguments,
    reason = "each oscillator exposes the pan-shaper coordinates as independent host parameters"
)]
fn oscillator_pan_shape_settings(
    segments: (PanShapeSegmentsRt, PanShapeSegmentsRt),
    initialized: bool,
    center: f32,
    left: f32,
    right: f32,
    left_curve: f32,
    right_curve: f32,
    left_time: f32,
    right_time: f32,
    center_x: f32,
) -> PanShapeSettings {
    let (left_segments, right_segments) = segments;
    let center = if initialized {
        left_segments.seg_p0[0]
    } else {
        center
    };
    let left = if initialized {
        left_segments.seg_p3[usize::from(left_segments.count.saturating_sub(1))]
    } else {
        left
    };
    let right = if initialized {
        right_segments.seg_p3[usize::from(right_segments.count.saturating_sub(1))]
    } else {
        right
    };
    PanShapeSettings::new(center, 1.0, 0.0)
        .with_center_x(center_x)
        .with_sides(left, right, left_curve, right_curve)
        .with_curve_times(left_time, right_time)
        .with_segments((left_segments, right_segments))
}

fn unison_configurations(params: &KurvParams, state: &KurvDspState) -> [UnisonSettings; 3] {
    [
        UnisonSettings::new(
            params.unison_voices.value_u8(),
            params.unison_detune.value() * 100.0,
            params.unison_stereo.value(),
            params.phase_random.value(),
            params.unison_curve.value(),
        )
        .with_detune_amount(params.unison_detune_amount.value())
        .with_harmonic_align(params.unison_harmonic_align.value())
        .with_alignment_mode(params.unison_alignment_mode.value_u8())
        .with_pan_shape(
            PanShapeSettings::new(
                params.pan_shape_center.value(),
                params.pan_shape_edge.value(),
                params.pan_shape_curve.value(),
            )
            .with_center_x(params.pan_shape_center_x.value())
            .with_sides(
                params.pan_shape_left.value(),
                params.pan_shape_right.value(),
                params.pan_shape_left_curve.value(),
                params.pan_shape_right_curve.value(),
            )
            .with_curve_times(
                params.pan_shape_left_curve_time.value(),
                params.pan_shape_right_curve_time.value(),
            )
            .with_segments(state.pan_shape_segments[0]),
        )
        .with_stereo_square(params.stereo_alternate.value(), params.stereo_x.value())
        .with_swarm(
            params.unison_swarm.value(),
            params.unison_swarm_rate.value(),
        )
        .with_swarm_mode(SwarmMode::from_index(params.unison_swarm_mode.value_u8()))
        .with_level_curve(params.unison_weight.value()),
        UnisonSettings::new(
            params.osc2_unison_voices.value_u8(),
            params.osc2_unison_detune.value() * 100.0,
            params.osc2_unison_stereo.value(),
            params.osc2_phase_random.value(),
            params.osc2_unison_curve.value(),
        )
        .with_detune_amount(params.osc2_unison_detune_amount.value())
        .with_harmonic_align(params.osc2_unison_harmonic_align.value())
        .with_alignment_mode(params.osc2_unison_alignment_mode.value_u8())
        .with_pan_shape(oscillator_pan_shape_settings(
            state.pan_shape_segments[1],
            params.osc2_pan_shape_curve_state.is_initialized(),
            params.osc2_pan_shape_center.value(),
            params.osc2_pan_shape_left.value(),
            params.osc2_pan_shape_right.value(),
            params.osc2_pan_shape_left_curve.value(),
            params.osc2_pan_shape_right_curve.value(),
            params.osc2_pan_shape_left_curve_time.value(),
            params.osc2_pan_shape_right_curve_time.value(),
            params.osc2_pan_shape_center_x.value(),
        ))
        .with_stereo_square(
            params.osc2_stereo_alternate.value(),
            params.osc2_stereo_x.value(),
        )
        .with_swarm(
            params.osc2_unison_jitter.value(),
            params.osc2_unison_jitter_rate.value(),
        )
        .with_swarm_mode(SwarmMode::from_index(params.osc2_jitter_mode.value_u8()))
        .with_level_curve(params.osc2_unison_weight.value()),
        UnisonSettings::new(
            params.osc3_unison_voices.value_u8(),
            params.osc3_unison_detune.value() * 100.0,
            params.osc3_unison_stereo.value(),
            params.osc3_phase_random.value(),
            params.osc3_unison_curve.value(),
        )
        .with_detune_amount(params.osc3_unison_detune_amount.value())
        .with_harmonic_align(params.osc3_unison_harmonic_align.value())
        .with_alignment_mode(params.osc3_unison_alignment_mode.value_u8())
        .with_pan_shape(oscillator_pan_shape_settings(
            state.pan_shape_segments[2],
            params.osc3_pan_shape_curve_state.is_initialized(),
            params.osc3_pan_shape_center.value(),
            params.osc3_pan_shape_left.value(),
            params.osc3_pan_shape_right.value(),
            params.osc3_pan_shape_left_curve.value(),
            params.osc3_pan_shape_right_curve.value(),
            params.osc3_pan_shape_left_curve_time.value(),
            params.osc3_pan_shape_right_curve_time.value(),
            params.osc3_pan_shape_center_x.value(),
        ))
        .with_stereo_square(
            params.osc3_stereo_alternate.value(),
            params.osc3_stereo_x.value(),
        )
        .with_swarm(
            params.osc3_unison_jitter.value(),
            params.osc3_unison_jitter_rate.value(),
        )
        .with_swarm_mode(SwarmMode::from_index(params.osc3_jitter_mode.value_u8()))
        .with_level_curve(params.osc3_unison_weight.value()),
    ]
}

#[derive(Clone, Copy)]
struct WaveCurveTransition {
    previous: WaveCurveRt,
    current: WaveCurveRt,
    progress: f32,
}

impl Default for WaveCurveTransition {
    fn default() -> Self {
        let curve = WaveCurveRt::default();
        Self {
            previous: curve,
            current: curve,
            progress: 1.0,
        }
    }
}

impl WaveCurveTransition {
    fn retarget(&mut self, curve: WaveCurveRt, audible: bool) {
        if curve != self.current {
            self.previous = WaveCurveRt::interpolate(self.previous, self.current, self.progress);
            self.current = curve;
            self.progress = if audible { 0.0 } else { 1.0 };
        }
    }

    fn value(self, progress: f32) -> WaveCurveRt {
        if progress >= 1.0 {
            self.current
        } else {
            WaveCurveRt::interpolate(self.previous, self.current, progress)
        }
    }
}

pub struct KurvDspState {
    synth: PolySynth,
    internal_pool: InternalRtPool,
    host_sample_rate: f32,
    dsp_sample_rate: f32,
    oversampler: StereoOversampler,
    decimator_tail: u8,
    mpe_bend_range: f32,
    pitch_bend_range: f32,
    glide_time_control: f32,
    pitch_bend_control: f32,
    controls: ControlBlock,
    meter_left: f32,
    meter_right: f32,
    pan_shape_segments: [(PanShapeSegmentsRt, PanShapeSegmentsRt); 3],
    wave_curves: [WaveCurveTransition; 3],
    lfos: LfoBank,
    lfo_modulation_block: [modulators::lfo::ModulationFrame; BLOCK_INTERNAL_SAMPLES],
    #[cfg(test)]
    block_major_enabled: bool,
    #[cfg(test)]
    block_major_chunks: usize,
    #[cfg(test)]
    internal_pool_enabled: bool,
    #[cfg(test)]
    internal_pool_coarse_jobs: usize,
    #[cfg(test)]
    internal_pool_partial_serial_jobs: usize,
}

impl Default for KurvDspState {
    fn default() -> Self {
        diagnostics::startup();
        diagnostics::lifecycle("dsp-default-enter");
        performance::initialize();
        let state = Self {
            synth: PolySynth::default(),
            internal_pool: InternalRtPool::new(),
            host_sample_rate: 44_100.0,
            dsp_sample_rate: 44_100.0 * f32::from(DEFAULT_FACTOR),
            oversampler: StereoOversampler::default(),
            decimator_tail: 0,
            mpe_bend_range: 48.0,
            pitch_bend_range: 2.0,
            glide_time_control: f32::NAN,
            pitch_bend_control: f32::NAN,
            controls: ControlBlock::default(),
            meter_left: 0.0,
            meter_right: 0.0,
            pan_shape_segments: [(
                PanShapeSegmentsRt::identity(),
                PanShapeSegmentsRt::identity(),
            ); 3],
            wave_curves: [WaveCurveTransition::default(); 3],
            lfos: LfoBank::default(),
            lfo_modulation_block: [modulators::lfo::ModulationFrame::default();
                BLOCK_INTERNAL_SAMPLES],
            #[cfg(test)]
            block_major_enabled: true,
            #[cfg(test)]
            block_major_chunks: 0,
            #[cfg(test)]
            internal_pool_enabled: true,
            #[cfg(test)]
            internal_pool_coarse_jobs: 0,
            #[cfg(test)]
            internal_pool_partial_serial_jobs: 0,
        };
        diagnostics::lifecycle("dsp-default-return");
        state
    }
}

impl KurvDspState {
    fn fill_wave_curve_fades(&mut self, len: usize) {
        let step = 1.0 / (self.host_sample_rate * 0.004).max(1.0);
        for (transition, output) in self.wave_curves.iter_mut().zip([
            &mut self.controls.osc1_curve_fade,
            &mut self.controls.osc2_curve_fade,
            &mut self.controls.osc3_curve_fade,
        ]) {
            for value in &mut output[..len] {
                transition.progress = (transition.progress + step).min(1.0);
                *value = transition.progress;
            }
        }
    }

    const fn block_major_enabled(&self) -> bool {
        #[cfg(test)]
        {
            self.block_major_enabled
        }
        #[cfg(not(test))]
        {
            true
        }
    }

    const fn internal_pool_enabled(&self) -> bool {
        #[cfg(test)]
        {
            self.internal_pool_enabled
        }
        #[cfg(not(test))]
        {
            true
        }
    }

    fn set_oversampling(&mut self, factor: u8, antialiasing: Antialiasing) -> bool {
        if factor == self.oversampler.factor() || self.synth.is_active() || self.decimator_tail != 0
        {
            return false;
        }
        self.oversampler.reset(factor);
        self.oversampler.set_spline_correction_immediate(matches!(
            antialiasing.for_factor(factor),
            Antialiasing::SplineOptimized
        ));
        self.dsp_sample_rate = self.host_sample_rate * f32::from(factor);
        self.synth.set_sample_rate(self.dsp_sample_rate);
        self.lfos.set_sample_rate(self.dsp_sample_rate);
        true
    }
}

fn render_saw_host_block<const SAMPLES: usize>(
    state: &mut KurvDspState,
    buffer: &mut AudioBuffer,
    output_channels: usize,
    sample_index: usize,
    chunks: usize,
    settings: VoiceSettings,
    envelope: EnvelopeSettings,
    gain: f32,
    shapes: Option<&[[f32; MAX_JOB_SAMPLES]; 3]>,
    output_gains: Option<&[f32; MAX_JOB_SAMPLES]>,
) -> (f32, f32) {
    let factor = usize::from(state.oversampler.factor());
    debug_assert_eq!(SAMPLES % factor, 0);
    let internal_samples = SAMPLES * chunks;
    debug_assert!(internal_samples <= MAX_JOB_SAMPLES);
    let mut samples = [(0.0_f32, 0.0_f32); MAX_JOB_SAMPLES];
    let full_coarse_job = chunks == MAX_JOB_SAMPLES / SAMPLES;
    let generic_shape = !state.synth.exact_saw_banks_eligible(settings);
    let worthwhile_generic_job = generic_shape && internal_samples >= 128;
    let pooled = ((full_coarse_job || worthwhile_generic_job) && state.internal_pool_enabled())
        .then(|| match shapes {
            Some(shapes) => state.internal_pool.render_morph_job::<SAMPLES>(
                &mut state.synth,
                settings,
                envelope,
                chunks,
                shapes,
            ),
            None => state.internal_pool.render_block_job::<SAMPLES>(
                &mut state.synth,
                settings,
                envelope,
                chunks,
            ),
        })
        .flatten();
    #[cfg(test)]
    {
        if pooled.is_some() {
            state.internal_pool_coarse_jobs += 1;
        } else if state.internal_pool_enabled() && !full_coarse_job && !generic_shape {
            state.internal_pool_partial_serial_jobs += 1;
        }
    }
    if let Some(block) = pooled {
        debug_assert_eq!(block.len, internal_samples);
        samples = block.samples;
    } else {
        for chunk in 0..chunks {
            let rendered = if let Some(shapes) = shapes {
                let offset = chunk * SAMPLES;
                let shapes = std::array::from_fn(|oscillator| {
                    std::array::from_fn(|frame| shapes[oscillator][offset + frame])
                });
                state
                    .synth
                    .render_morph_block::<SAMPLES>(settings, envelope, &shapes)
            } else {
                state.synth.render_block::<SAMPLES>(settings, envelope)
            };
            samples[chunk * SAMPLES..(chunk + 1) * SAMPLES].copy_from_slice(&rendered);
        }
    }
    let mut peak_left = 0.0_f32;
    let mut peak_right = 0.0_f32;
    for frame in 0..internal_samples / factor {
        let (left, right) = if factor == 1 {
            let (left, right) = samples[frame];
            state.oversampler.process_direct(left, right)
        } else {
            for (left, right) in samples[frame * factor..(frame + 1) * factor]
                .iter()
                .copied()
            {
                state.oversampler.push(left, right);
            }
            state.oversampler.output()
        };
        let output_gain = output_gains.map_or(1.0, |gains| gains[frame]);
        let left = left * gain * output_gain;
        let right = right * gain * output_gain;
        peak_left = peak_left.max(left.abs());
        peak_right = peak_right.max(right.abs());
        let output_index = sample_index + frame;
        if output_channels == 1 {
            buffer.output(0)[output_index] = (left + right) * 0.5;
        } else {
            buffer.output(0)[output_index] = left;
            buffer.output(1)[output_index] = right;
        }
        for channel in 2..output_channels {
            buffer.output(channel)[output_index] = (left + right) * 0.5;
        }
    }
    (peak_left, peak_right)
}

fn render_saw_host_pitch_block<const SAMPLES: usize>(
    state: &mut KurvDspState,
    buffer: &mut AudioBuffer,
    output_channels: usize,
    sample_index: usize,
    settings: VoiceSettings,
    envelope: EnvelopeSettings,
    gain: f32,
    output_gains: &[f32],
    unison_modulation_mask: u8,
) -> (f32, f32) {
    let factor = usize::from(state.oversampler.factor());
    debug_assert_eq!(SAMPLES % factor, 0);
    let samples = state.synth.render_pitch_block::<SAMPLES>(
        settings,
        envelope,
        &state.lfo_modulation_block[..SAMPLES],
        unison_modulation_mask,
    );
    let mut peak_left = 0.0_f32;
    let mut peak_right = 0.0_f32;
    for frame in 0..SAMPLES / factor {
        let (left, right) = if factor == 1 {
            let (left, right) = samples[frame];
            state.oversampler.process_direct(left, right)
        } else {
            for (left, right) in samples[frame * factor..(frame + 1) * factor]
                .iter()
                .copied()
            {
                state.oversampler.push(left, right);
            }
            state.oversampler.output()
        };
        let output_gain = output_gains.get(frame).copied().unwrap_or(1.0);
        let left = left * gain * output_gain;
        let right = right * gain * output_gain;
        peak_left = peak_left.max(left.abs());
        peak_right = peak_right.max(right.abs());
        let output_index = sample_index + frame;
        if output_channels == 1 {
            buffer.output(0)[output_index] = (left + right) * 0.5;
        } else {
            buffer.output(0)[output_index] = left;
            buffer.output(1)[output_index] = right;
        }
        for channel in 2..output_channels {
            buffer.output(channel)[output_index] = (left + right) * 0.5;
        }
    }
    (peak_left, peak_right)
}

fn modulated_envelope(
    base: EnvelopeSettings,
    modulation: lfo::GlobalModulation,
) -> EnvelopeSettings {
    EnvelopeSettings {
        attack: (base.attack + modulation.attack).clamp(0.0, 8.0),
        decay: (base.decay + modulation.decay).clamp(0.0, 8.0),
        sustain: (base.sustain + modulation.sustain).clamp(0.0, 1.0),
        release: (base.release + modulation.release).clamp(0.0, 12.0),
        attack_curve: (base.attack_curve + modulation.attack_curve).clamp(-1.0, 1.0),
        decay_curve: (base.decay_curve + modulation.decay_curve).clamp(-1.0, 1.0),
        release_curve: (base.release_curve + modulation.release_curve).clamp(-1.0, 1.0),
        attack_curve_time: (base.attack_curve_time + modulation.attack_curve_time)
            .clamp(0.05, 0.95),
        decay_curve_time: (base.decay_curve_time + modulation.decay_curve_time).clamp(0.05, 0.95),
        release_curve_time: (base.release_curve_time + modulation.release_curve_time)
            .clamp(0.05, 0.95),
    }
}

#[inline(always)]
fn advance_lfo_modulation(
    state: &mut KurvDspState,
    routes: &ActiveRoutes,
    direct_unison_mask: u8,
    lfo_control_dynamic_mask: u8,
    frame: usize,
    modulation: &mut lfo::ModulationFrame,
) {
    clear_modulation_frame(modulation, routes, direct_unison_mask);
    if !state.lfos.is_active() {
        return;
    }
    let sources = if lfo_control_dynamic_mask == 0 {
        if state.lfos.direct_phase_active() {
            state.lfos.next_direct_ref()
        } else {
            state.lfos.next_ref()
        }
    } else {
        state.lfos.next_with_controls_ref(
            lfo_control_dynamic_mask,
            &state.controls.lfo_rate,
            &state.controls.lfo_phase,
            frame,
        )
    };
    for route in routes.as_slice() {
        let amount_index = route.amount_index;
        if let Some(descriptor) = route.descriptor {
            accumulate_modulation(
                modulation,
                descriptor,
                route.source_index,
                state.controls.modulation_amounts[amount_index][frame],
                &sources,
            );
        }
    }
}

fn apply_modulation(
    state: &mut KurvDspState,
    settings: &mut VoiceSettings,
    routes: &ActiveRoutes,
    base_unison: &[UnisonSettings; 3],
    base_glide: f32,
    direct_unison_mask: u8,
    lfo_control_dynamic_mask: u8,
    frame: usize,
    modulation: &mut lfo::ModulationFrame,
) {
    advance_lfo_modulation(
        state,
        routes,
        direct_unison_mask,
        lfo_control_dynamic_mask,
        frame,
        modulation,
    );
    if direct_unison_mask != 0 {
        for oscillator in 0..3 {
            if direct_unison_mask & (1 << oscillator) == 0 {
                continue;
            }
            let control = &state.controls.unison_pitch[oscillator];
            let base = base_unison[oscillator];
            modulation.unison[oscillator].detune_cents +=
                control.detune_cents[frame].mul_add(100.0, -base.detune_cents());
            modulation.unison[oscillator].detune_amount +=
                control.detune_amount[frame] - base.detune_amount();
            modulation.unison[oscillator].harmonic_align +=
                control.harmonic_align[frame] - base.harmonic_align();
            modulation.unison[oscillator].curve += control.curve[frame] - base.curve();
            modulation.unison[oscillator].stereo += control.stereo[frame] - base.stereo();
            modulation.unison[oscillator].stereo_x += control.stereo_x[frame] - base.stereo_x();
            modulation.unison[oscillator].stereo_y +=
                control.stereo_y[frame] - base.stereo_alternate();
            modulation.unison[oscillator].weight += control.weight[frame] - base.level_curve();
            let pan_shape = base.pan_shape();
            modulation.unison[oscillator].pan_center +=
                control.pan_center[frame] - pan_shape.center;
            modulation.unison[oscillator].pan_left += control.pan_left[frame] - pan_shape.left_edge;
            modulation.unison[oscillator].pan_right +=
                control.pan_right[frame] - pan_shape.right_edge;
            modulation.unison[oscillator].pan_center_x +=
                control.pan_center_x[frame] - pan_shape.center_x;
        }
    }
    for oscillator in 0..3 {
        let bit = 1 << oscillator;
        if routes.oscillator_mask & bit != 0 {
            let oscillator_modulation = modulation.oscillator[oscillator];
            settings.modulate_oscillator(
                oscillator,
                oscillator_modulation.pitch_semitones,
                oscillator_modulation.shape,
                oscillator_modulation.pulse_width,
                oscillator_modulation.warp,
                oscillator_modulation.custom_shape,
                oscillator_modulation.level,
                oscillator_modulation.pan,
            );
        }
        if (routes.unison_frame_mask | direct_unison_mask) & bit != 0 {
            settings.modulate_unison_detune_amount(
                oscillator,
                modulation.unison[oscillator].detune_amount,
            );
        }
    }
    if routes.global_mask & GLOBAL_VELOCITY_MASK != 0 {
        settings.velocity_amount =
            (settings.velocity_amount + modulation.global.velocity).clamp(0.0, 1.0);
    }
    if routes.global_mask & GLOBAL_PRESSURE_MASK != 0 {
        settings.pressure_amount =
            (settings.pressure_amount + modulation.global.pressure).clamp(0.0, 1.0);
    }
    if routes.global_mask & GLOBAL_TIMBRE_MASK != 0 {
        settings.timbre_amount =
            (settings.timbre_amount + modulation.global.timbre).clamp(0.0, 1.0);
    }
    if routes.global_mask & GLOBAL_GLIDE_MASK != 0 {
        state
            .synth
            .set_glide_time((base_glide + modulation.global.glide).clamp(0.0, 5.0));
    }

    if routes.unison_layout_mask != 0 && state.lfos.is_active() {
        for oscillator in 0..3 {
            if routes.unison_layout_mask & (1 << oscillator) == 0 {
                continue;
            }
            let control = &state.controls.unison_pitch[oscillator];
            let settings = unison_motion_settings(
                base_unison[oscillator],
                control.phase_random[frame],
                control.jitter_amount[frame],
                control.jitter_rate[frame],
                modulation.unison[oscillator],
            );
            state.synth.configure_unison_motion(oscillator, settings);
        }
    }
}

fn fill_lfo_morph_block(
    state: &mut KurvDspState,
    routes: &ActiveRoutes,
    oscillator_shape_mask: u8,
    output_active: bool,
    lfo_control_dynamic_mask: u8,
    control_start: usize,
    host_frames: usize,
    factor: usize,
    shapes: Option<&mut [[f32; MAX_JOB_SAMPLES]; 3]>,
    output_gains: &mut [f32; MAX_JOB_SAMPLES],
    modulation: &mut lfo::ModulationFrame,
) {
    debug_assert!(host_frames * factor <= MAX_JOB_SAMPLES);
    let mut shapes = shapes;
    for host_frame in 0..host_frames {
        let frame = control_start + host_frame;
        for internal_frame in 0..factor {
            advance_lfo_modulation(
                state,
                routes,
                0,
                lfo_control_dynamic_mask,
                frame,
                modulation,
            );
            let index = host_frame * factor + internal_frame;
            if let Some(shapes) = shapes.as_deref_mut() {
                for oscillator in 0..3 {
                    if oscillator_shape_mask & (1 << oscillator) == 0 {
                        continue;
                    }
                    shapes[oscillator][index] = (shapes[oscillator][index]
                        + modulation.oscillator[oscillator].shape)
                        .clamp(0.0, 3.0);
                }
            }
        }
        if output_active {
            output_gains[host_frame] = db_to_linear(modulation.global.output_db);
        }
    }
}

fn fill_lfo_pitch_block(
    state: &mut KurvDspState,
    routes: &ActiveRoutes,
    lfo_control_dynamic_mask: u8,
    control_start: usize,
    host_frames: usize,
    factor: usize,
    modulation: &mut lfo::ModulationFrame,
    output_gains: &mut [f32],
) {
    let internal_samples = host_frames * factor;
    debug_assert!(internal_samples <= state.lfo_modulation_block.len());
    for host_frame in 0..host_frames {
        let frame = control_start + host_frame;
        for internal_frame in 0..factor {
            advance_lfo_modulation(
                state,
                routes,
                0,
                lfo_control_dynamic_mask,
                frame,
                modulation,
            );
            state.lfo_modulation_block[host_frame * factor + internal_frame] = *modulation;
        }
        output_gains[host_frame] = db_to_linear(modulation.global.output_db);
    }
}

fn render_lfo_pitch_chunk<const SAMPLES: usize>(
    state: &mut KurvDspState,
    buffer: &mut AudioBuffer,
    output_channels: usize,
    sample_index: usize,
    control_start: usize,
    settings: VoiceSettings,
    envelope: EnvelopeSettings,
    gain: f32,
    routes: &ActiveRoutes,
    lfo_control_dynamic_mask: u8,
    modulation: &mut lfo::ModulationFrame,
) -> (f32, f32) {
    let factor = usize::from(state.oversampler.factor());
    debug_assert_eq!(SAMPLES % factor, 0);
    let host_frames = SAMPLES / factor;
    let mut output_gains = [1.0_f32; MAX_JOB_SAMPLES];
    fill_lfo_pitch_block(
        state,
        routes,
        lfo_control_dynamic_mask,
        control_start,
        host_frames,
        factor,
        modulation,
        &mut output_gains[..host_frames],
    );
    render_saw_host_pitch_block::<SAMPLES>(
        state,
        buffer,
        output_channels,
        sample_index,
        settings,
        envelope,
        gain,
        &output_gains[..host_frames],
        routes.unison_frame_mask,
    )
}

fn render_lfo_motion_chunk<const SAMPLES: usize>(
    state: &mut KurvDspState,
    buffer: &mut AudioBuffer,
    output_channels: usize,
    sample_index: usize,
    control_start: usize,
    settings: VoiceSettings,
    envelope: EnvelopeSettings,
    gain: f32,
    routes: &ActiveRoutes,
    base_unison: &[UnisonSettings; 3],
    lfo_control_dynamic_mask: u8,
    modulation: &mut lfo::ModulationFrame,
) -> (f32, f32) {
    let factor = usize::from(state.oversampler.factor());
    debug_assert_eq!(SAMPLES % factor, 0);
    let host_frames = SAMPLES / factor;
    let mut output_gains = [1.0_f32; MAX_JOB_SAMPLES];
    fill_lfo_pitch_block(
        state,
        routes,
        lfo_control_dynamic_mask,
        control_start,
        host_frames,
        factor,
        modulation,
        &mut output_gains[..host_frames],
    );
    let samples = state.synth.render_motion_block::<SAMPLES>(
        settings,
        envelope,
        &state.lfo_modulation_block[..SAMPLES],
        routes.unison_layout_mask,
        base_unison,
    );
    let mut peak_left = 0.0_f32;
    let mut peak_right = 0.0_f32;
    for frame in 0..host_frames {
        let (left, right) = if factor == 1 {
            let (left, right) = samples[frame];
            state.oversampler.process_direct(left, right)
        } else {
            for (left, right) in samples[frame * factor..(frame + 1) * factor]
                .iter()
                .copied()
            {
                state.oversampler.push(left, right);
            }
            state.oversampler.output()
        };
        let output_gain = output_gains[frame];
        let left = left * gain * output_gain;
        let right = right * gain * output_gain;
        peak_left = peak_left.max(left.abs());
        peak_right = peak_right.max(right.abs());
        let output_index = sample_index + frame;
        if output_channels == 1 {
            buffer.output(0)[output_index] = (left + right) * 0.5;
        } else {
            buffer.output(0)[output_index] = left;
            buffer.output(1)[output_index] = right;
        }
        for channel in 2..output_channels {
            buffer.output(channel)[output_index] = (left + right) * 0.5;
        }
    }
    (peak_left, peak_right)
}

fn render_lfo_control_chunk<const SAMPLES: usize>(
    state: &mut KurvDspState,
    buffer: &mut AudioBuffer,
    output_channels: usize,
    sample_index: usize,
    control_start: usize,
    settings: VoiceSettings,
    envelope: EnvelopeSettings,
    gain: f32,
    routes: &ActiveRoutes,
    lfo_control_dynamic_mask: u8,
    modulation: &mut lfo::ModulationFrame,
) -> (f32, f32) {
    let factor = usize::from(state.oversampler.factor());
    debug_assert_eq!(SAMPLES % factor, 0);
    let host_frames = SAMPLES / factor;
    let mut output_gains = [1.0_f32; MAX_JOB_SAMPLES];
    fill_lfo_pitch_block(
        state,
        routes,
        lfo_control_dynamic_mask,
        control_start,
        host_frames,
        factor,
        modulation,
        &mut output_gains[..host_frames],
    );
    let samples = state.synth.render_modulation_block::<SAMPLES>(
        settings,
        envelope,
        &state.lfo_modulation_block[..SAMPLES],
    );
    let mut peak_left = 0.0_f32;
    let mut peak_right = 0.0_f32;
    for frame in 0..host_frames {
        let (left, right) = if factor == 1 {
            let (left, right) = samples[frame];
            state.oversampler.process_direct(left, right)
        } else {
            for (left, right) in samples[frame * factor..(frame + 1) * factor]
                .iter()
                .copied()
            {
                state.oversampler.push(left, right);
            }
            state.oversampler.output()
        };
        let output_gain = output_gains[frame];
        let left = left * gain * output_gain;
        let right = right * gain * output_gain;
        peak_left = peak_left.max(left.abs());
        peak_right = peak_right.max(right.abs());
        let output_index = sample_index + frame;
        if output_channels == 1 {
            buffer.output(0)[output_index] = (left + right) * 0.5;
        } else {
            buffer.output(0)[output_index] = left;
            buffer.output(1)[output_index] = right;
        }
        for channel in 2..output_channels {
            buffer.output(channel)[output_index] = (left + right) * 0.5;
        }
    }
    (peak_left, peak_right)
}

#[inline]
fn unison_motion_settings(
    base: UnisonSettings,
    phase_random: f32,
    jitter_amount: f32,
    jitter_rate: f32,
    modulation: lfo::UnisonModulation,
) -> UnisonSettings {
    let rate_scale = if modulation.jitter_rate_normalized == 0.0 {
        1.0
    } else {
        5_000.0_f32.powf(modulation.jitter_rate_normalized.clamp(-1.0, 1.0))
    };
    base.with_motion(
        phase_random + modulation.phase_random,
        jitter_amount + modulation.jitter_amount,
        jitter_rate * rate_scale,
    )
}

fn configure_direct_unison_motion(
    state: &mut KurvDspState,
    base_unison: &[UnisonSettings; 3],
    motion_mask: u8,
    frame: usize,
) {
    for oscillator in 0..3 {
        if motion_mask & (1 << oscillator) == 0 {
            continue;
        }
        let control = &state.controls.unison_pitch[oscillator];
        let settings = base_unison[oscillator].with_motion(
            control.phase_random[frame],
            control.jitter_amount[frame],
            control.jitter_rate[frame],
        );
        state.synth.configure_unison_motion(oscillator, settings);
    }
}

fn clear_modulation_frame(
    modulation: &mut lfo::ModulationFrame,
    routes: &ActiveRoutes,
    direct_unison_mask: u8,
) {
    for oscillator in 0..3 {
        let bit = 1 << oscillator;
        if routes.oscillator_mask & bit != 0 {
            modulation.oscillator[oscillator] = lfo::OscillatorModulation::default();
        }
        if (routes.unison_layout_mask | routes.unison_frame_mask | direct_unison_mask) & bit != 0 {
            modulation.unison[oscillator] = lfo::UnisonModulation::default();
        }
    }
    if routes.global_mask != 0 {
        modulation.global = lfo::GlobalModulation::default();
    }
}

#[inline(always)]
fn accumulate_modulation(
    modulation: &mut lfo::ModulationFrame,
    target: modulation_target::TargetDescriptor,
    source_index: usize,
    amount: f32,
    sources: &[f32; LFO_COUNT],
) {
    use modulation_target::{GlobalTarget, OscTarget, TargetKind, UnisonTarget};

    let value = sources[source_index] * amount.clamp(-1.0, 1.0);
    let scaled = value * target.scale;
    match target.kind {
        TargetKind::Oscillator {
            oscillator,
            control,
        } => {
            let destination = &mut modulation.oscillator[usize::from(oscillator)];
            match control {
                OscTarget::Pitch => destination.pitch_semitones += scaled,
                OscTarget::Shape => destination.shape += scaled,
                OscTarget::PulseWidth => destination.pulse_width += scaled,
                OscTarget::Warp => destination.warp += scaled,
                OscTarget::CustomShape => destination.custom_shape += scaled,
                OscTarget::Level => destination.level += scaled,
                OscTarget::Pan => destination.pan += scaled,
            }
        }
        TargetKind::Unison {
            oscillator,
            control,
        } => {
            let destination = &mut modulation.unison[usize::from(oscillator)];
            match control {
                UnisonTarget::DetuneAmount => destination.detune_amount += scaled,
                UnisonTarget::DetuneRange => destination.detune_cents += scaled,
                UnisonTarget::HarmonicAlign => destination.harmonic_align += scaled,
                UnisonTarget::Stereo => destination.stereo += scaled,
                UnisonTarget::PhaseRandom => destination.phase_random += scaled,
                UnisonTarget::Curve => destination.curve += scaled,
                UnisonTarget::JitterAmount => destination.jitter_amount += scaled,
                UnisonTarget::JitterRate => destination.jitter_rate_normalized += value,
                UnisonTarget::StereoX => destination.stereo_x += scaled,
                UnisonTarget::StereoY => destination.stereo_y += scaled,
                UnisonTarget::Weight => destination.weight += scaled,
                UnisonTarget::PanCenter => destination.pan_center += scaled,
                UnisonTarget::PanLeft => destination.pan_left += scaled,
                UnisonTarget::PanRight => destination.pan_right += scaled,
                UnisonTarget::PanCenterX => destination.pan_center_x += scaled,
            }
        }
        TargetKind::Global(control) => match control {
            GlobalTarget::Output => modulation.global.output_db += scaled,
            GlobalTarget::Attack => modulation.global.attack += scaled,
            GlobalTarget::Decay => modulation.global.decay += scaled,
            GlobalTarget::Sustain => modulation.global.sustain += scaled,
            GlobalTarget::Release => modulation.global.release += scaled,
            GlobalTarget::AttackCurve => modulation.global.attack_curve += scaled,
            GlobalTarget::DecayCurve => modulation.global.decay_curve += scaled,
            GlobalTarget::ReleaseCurve => modulation.global.release_curve += scaled,
            GlobalTarget::AttackCurveTime => modulation.global.attack_curve_time += scaled,
            GlobalTarget::DecayCurveTime => modulation.global.decay_curve_time += scaled,
            GlobalTarget::ReleaseCurveTime => modulation.global.release_curve_time += scaled,
            GlobalTarget::Velocity => modulation.global.velocity += scaled,
            GlobalTarget::Pressure => modulation.global.pressure += scaled,
            GlobalTarget::Timbre => modulation.global.timbre += scaled,
            GlobalTarget::Glide => modulation.global.glide += scaled,
        },
    }
}

#[allow(
    clippy::cast_precision_loss,
    reason = "audio buffers are far smaller than f32's exact integer range"
)]
fn publish_meters(
    state: &mut KurvDspState,
    params: &KurvParams,
    context: &ProcessContext,
    samples: usize,
    peak_left: f32,
    peak_right: f32,
) {
    let release = (1.0 - samples as f32 / (state.host_sample_rate * 0.18)).clamp(0.0, 1.0);
    state.meter_left = peak_left.max(state.meter_left * release);
    state.meter_right = peak_right.max(state.meter_right * release);
    context.set_meter(&params.meter_left, state.meter_left.min(1.0));
    context.set_meter(&params.meter_right, state.meter_right.min(1.0));
    context.set_meter(&params.stereo_seed, state.synth.latest_stereo_seed(0));
    context.set_meter(&params.swarm_phase, state.synth.swarm_time());
    context.set_meter(&params.osc2_stereo_seed, state.synth.latest_stereo_seed(1));
    context.set_meter(
        &params.osc2_swarm_phase,
        state.synth.secondary_swarm_time(1),
    );
    context.set_meter(&params.osc3_stereo_seed, state.synth.latest_stereo_seed(2));
    context.set_meter(
        &params.osc3_swarm_phase,
        state.synth.secondary_swarm_time(2),
    );
    let (lfo_phases, lfo_values) = state.lfos.ui_snapshot();
    let phase_meters = [
        &params.lfo1_phase_meter,
        &params.lfo2_phase_meter,
        &params.lfo3_phase_meter,
        &params.lfo4_phase_meter,
        &params.lfo5_phase_meter,
        &params.lfo6_phase_meter,
        &params.lfo7_phase_meter,
        &params.lfo8_phase_meter,
    ];
    let value_meters = [
        &params.lfo1_value_meter,
        &params.lfo2_value_meter,
        &params.lfo3_value_meter,
        &params.lfo4_value_meter,
        &params.lfo5_value_meter,
        &params.lfo6_value_meter,
        &params.lfo7_value_meter,
        &params.lfo8_value_meter,
    ];
    for index in 0..LFO_COUNT {
        context.set_meter(phase_meters[index], lfo_phases[index]);
        context.set_meter(value_meters[index], lfo_values[index]);
    }
}

const fn current_process_status(state: &KurvDspState) -> ProcessStatus {
    if state.synth.is_active() || state.decimator_tail != 0 {
        ProcessStatus::Normal
    } else {
        ProcessStatus::Tail(0)
    }
}

#[allow(
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    reason = "the event boundary keeps every supported MIDI 1 and MIDI 2 expression mapping explicit"
)]
fn dispatch_events(
    state: &mut KurvDspState,
    events: &EventList,
    next_event: &mut usize,
    sample_index: usize,
) {
    while let Some(event) = events.get(*next_event) {
        if event.sample_offset as usize > sample_index {
            break;
        }
        match &event.body {
            EventBody::NoteOn {
                channel,
                note,
                velocity,
                ..
            } => {
                state.lfos.note_on(*note);
                state
                    .synth
                    .note_on(*note, norm_7bit(*velocity), *channel, None);
            }
            EventBody::NoteOn2 {
                channel,
                note,
                velocity,
                ..
            } => {
                state.lfos.note_on(*note);
                state
                    .synth
                    .note_on(*note, norm_u16(*velocity), *channel, None);
            }
            EventBody::NoteOff { channel, note, .. } => {
                state.synth.note_off(*note, *channel, None);
            }
            EventBody::NoteOff2 { channel, note, .. } => {
                state.synth.note_off(*note, *channel, None);
            }
            EventBody::Aftertouch {
                channel,
                note,
                pressure,
                ..
            } => state
                .synth
                .pressure(*note, *channel, None, norm_7bit(*pressure)),
            EventBody::PolyPressure2 {
                channel,
                note,
                pressure,
                ..
            } => state
                .synth
                .pressure(*note, *channel, None, norm_u32(*pressure)),
            EventBody::ChannelPressure {
                channel, pressure, ..
            } => state.synth.channel_pressure(*channel, norm_7bit(*pressure)),
            EventBody::ChannelPressure2 {
                channel, pressure, ..
            } => state.synth.channel_pressure(*channel, norm_u32(*pressure)),
            EventBody::PitchBend { channel, value, .. } => {
                state
                    .synth
                    .pitch_bend(*channel, norm_pitch_bend(*value), state.pitch_bend_range);
            }
            EventBody::PitchBend2 { channel, value, .. } => {
                state.synth.pitch_bend(
                    *channel,
                    norm_pitch_bend_32(*value),
                    state.pitch_bend_range,
                );
            }
            EventBody::PerNotePitchBend {
                channel,
                note,
                value,
                ..
            } => state.synth.per_note_pitch_bend(
                *note,
                *channel,
                (per_note_bend_semitones(*value) / 48.0 * f64::from(state.mpe_bend_range)) as f32,
            ),
            EventBody::PerNoteCC {
                channel,
                note,
                cc: 74,
                value,
                ..
            } => state
                .synth
                .per_note_timbre(*note, *channel, norm_u32(*value)),
            EventBody::PerNoteManagement {
                channel,
                note,
                flags,
                ..
            } if flags & 0b10 != 0 => {
                state.synth.reset_per_note_controllers(*note, *channel);
            }
            EventBody::ParamChange { id, value } if *id == u32::from(P::PitchBend) => {
                #[allow(
                    clippy::cast_possible_truncation,
                    reason = "the pitch wheel parameter is bounded to -1..1 before entering f32 DSP"
                )]
                state
                    .synth
                    .parameter_pitch_bend((*value).clamp(-1.0, 1.0) as f32, state.pitch_bend_range);
            }
            EventBody::ControlChange {
                channel, cc, value, ..
            } => match cc {
                64 => state.synth.sustain(*channel, *value >= 64),
                74 => state.synth.timbre(*channel, norm_7bit(*value)),
                120 => state.synth.all_sound_off(*channel),
                121 => state.synth.reset_controllers(*channel),
                123..=127 => state.synth.all_notes_off(*channel),
                _ => {}
            },
            EventBody::ControlChange2 {
                channel, cc, value, ..
            } => match cc {
                64 => state.synth.sustain(*channel, *value >= 0x8000_0000),
                74 => state.synth.timbre(*channel, norm_u32(*value)),
                120 => state.synth.all_sound_off(*channel),
                121 => state.synth.reset_controllers(*channel),
                123..=127 => state.synth.all_notes_off(*channel),
                _ => {}
            },
            _ => {}
        }
        *next_event += 1;
    }
}

#[allow(
    clippy::cast_precision_loss,
    reason = "MIDI 2.0 normalized controls intentionally enter the f32 DSP domain"
)]
fn norm_u16(value: u16) -> f32 {
    f32::from(value) / f32::from(u16::MAX)
}

#[allow(
    clippy::cast_precision_loss,
    reason = "MIDI 2.0 normalized controls intentionally enter the f32 DSP domain"
)]
fn norm_u32(value: u32) -> f32 {
    value as f32 / u32::MAX as f32
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "the normalized bend is bounded to -1..1 before entering the f32 DSP domain"
)]
fn norm_pitch_bend_32(value: u32) -> f32 {
    ((f64::from(value) - 2_147_483_648.0) / 2_147_483_648.0) as f32
}

fn old_plain_value(value: &serde_json::Value) -> Option<f64> {
    let tagged = value.as_object()?;
    tagged
        .get("f32")
        .or_else(|| tagged.get("i32"))
        .and_then(serde_json::Value::as_f64)
        .or_else(|| {
            tagged
                .get("bool")
                .and_then(serde_json::Value::as_bool)
                .map(f64::from)
        })
}

truce::plugin! {
    logic: Kurv,
    params: KurvParams,
}

truce::enable_rt_paranoid!();

#[cfg(test)]
mod tests {
    use super::*;

    const PROCESS_TEST_FRAMES: usize = 128;

    fn render_process_test(
        events: &[Event],
        block_major_enabled: bool,
        smooth_shape: bool,
        swarm_mode: Option<SwarmMode>,
        oversampling_factor: u8,
    ) -> (Vec<(f32, f32)>, usize) {
        let params = KurvParams::default();
        params.unison_voices.set_value(64);
        params
            .oversampling
            .set_value(i64::from(oversampling_factor));
        params.phase_random.set_value(0.0);
        if let Some(mode) = swarm_mode {
            params.unison_swarm.set_value(1.0);
            params
                .unison_swarm_mode
                .set_value(i64::from(mode == SwarmMode::Jitter));
        }
        params.set_sample_rate(48_000.0);
        params.snap_smoothers();

        let mut state = KurvDspState {
            block_major_enabled,
            ..KurvDspState::default()
        };
        <Kurv as PluginLogic>::reset(
            &mut state,
            &params,
            &AudioConfig::new(48_000.0, PROCESS_TEST_FRAMES),
        );
        if smooth_shape {
            params.shape.set_value(2.5);
        }

        let mut input_events = EventList::with_capacity(events.len());
        for event in events {
            input_events.push(*event);
        }
        let mut output_events = EventList::with_capacity(0);
        let transport = TransportInfo::default();
        let mut context = ProcessContext::new(
            &transport,
            48_000.0,
            PROCESS_TEST_FRAMES,
            &mut output_events,
        );
        let mut left = vec![0.0; PROCESS_TEST_FRAMES];
        let mut right = vec![0.0; PROCESS_TEST_FRAMES];
        {
            let inputs: [&[f32]; 0] = [];
            let mut outputs: [&mut [f32]; 2] = [&mut left, &mut right];
            let mut buffer =
                AudioBuffer::from_slices_checked(&inputs, &mut outputs, PROCESS_TEST_FRAMES);
            let _ = <Kurv as PluginLogic>::process(
                &mut state,
                &params,
                &mut buffer,
                &input_events,
                &mut context,
            );
        }
        (
            left.into_iter().zip(right).collect(),
            state.block_major_chunks,
        )
    }

    fn assert_process_paths_equal(
        events: &[Event],
        smooth_shape: bool,
        swarm_mode: Option<SwarmMode>,
        oversampling_factor: u8,
    ) -> usize {
        let (reference, _) =
            render_process_test(events, false, smooth_shape, swarm_mode, oversampling_factor);
        let (candidate, chunks) =
            render_process_test(events, true, smooth_shape, swarm_mode, oversampling_factor);
        assert_eq!(candidate, reference);
        chunks
    }

    fn note_on(offset: u32) -> Event {
        Event::new(
            offset,
            EventBody::NoteOn {
                group: 0,
                channel: 1,
                note: 60,
                velocity: 127,
            },
        )
    }

    fn configure_audio_rate_routes(params: &KurvParams, targets: &[u8]) {
        params.lfo1_active.set_value(true);
        params.lfo1_rate.set_value(17.0);
        params.lfo1_bipolar.set_value(true);
        let sources = [
            &params.mod1_source,
            &params.mod2_source,
            &params.mod3_source,
            &params.mod4_source,
            &params.mod5_source,
            &params.mod6_source,
            &params.mod7_source,
            &params.mod8_source,
            &params.mod9_source,
            &params.mod10_source,
            &params.mod11_source,
            &params.mod12_source,
            &params.mod13_source,
            &params.mod14_source,
            &params.mod15_source,
            &params.mod16_source,
        ];
        let route_targets = [
            &params.mod1_target,
            &params.mod2_target,
            &params.mod3_target,
            &params.mod4_target,
            &params.mod5_target,
            &params.mod6_target,
            &params.mod7_target,
            &params.mod8_target,
            &params.mod9_target,
            &params.mod10_target,
            &params.mod11_target,
            &params.mod12_target,
            &params.mod13_target,
            &params.mod14_target,
            &params.mod15_target,
            &params.mod16_target,
        ];
        let target_exts = [
            &params.mod1_target_ext,
            &params.mod2_target_ext,
            &params.mod3_target_ext,
            &params.mod4_target_ext,
            &params.mod5_target_ext,
            &params.mod6_target_ext,
            &params.mod7_target_ext,
            &params.mod8_target_ext,
            &params.mod9_target_ext,
            &params.mod10_target_ext,
            &params.mod11_target_ext,
            &params.mod12_target_ext,
            &params.mod13_target_ext,
            &params.mod14_target_ext,
            &params.mod15_target_ext,
            &params.mod16_target_ext,
        ];
        let amounts = [
            &params.mod1_amount,
            &params.mod2_amount,
            &params.mod3_amount,
            &params.mod4_amount,
            &params.mod5_amount,
            &params.mod6_amount,
            &params.mod7_amount,
            &params.mod8_amount,
            &params.mod9_amount,
            &params.mod10_amount,
            &params.mod11_amount,
            &params.mod12_amount,
            &params.mod13_amount,
            &params.mod14_amount,
            &params.mod15_amount,
            &params.mod16_amount,
        ];
        for (index, target) in targets.iter().copied().enumerate() {
            sources[index].set_value(1);
            amounts[index].set_value(1.0);
            if target <= modulation_target::LEGACY_TARGET_COUNT {
                route_targets[index].set_value(i64::from(target));
                target_exts[index].set_value(0);
            } else {
                route_targets[index].set_value(0);
                target_exts[index]
                    .set_value(i64::from(target - modulation_target::LEGACY_TARGET_COUNT));
            }
        }
    }

    fn render_audio_rate_route_test(
        target: u8,
        block_major_enabled: bool,
        frames: usize,
    ) -> (Vec<(f32, f32)>, usize, std::time::Duration) {
        render_audio_rate_route_config_test(&[target], block_major_enabled, frames, 64, 8, 2, 1)
    }

    fn render_audio_rate_route_config_test(
        targets: &[u8],
        block_major_enabled: bool,
        frames: usize,
        unison_voices: i64,
        polyphony: i64,
        oversampling_factor: i64,
        oscillator_count: usize,
    ) -> (Vec<(f32, f32)>, usize, std::time::Duration) {
        let params = KurvParams::default();
        params.unison_voices.set_value(unison_voices);
        params.osc2_unison_voices.set_value(unison_voices);
        params.osc3_unison_voices.set_value(unison_voices);
        params.voice_mode.set_value(polyphony);
        params.osc2_enabled.set_value(oscillator_count >= 2);
        params.osc3_enabled.set_value(oscillator_count >= 3);
        params.oversampling.set_value(oversampling_factor);
        params.phase_random.set_value(0.0);
        params.unison_detune.set_value(48.0);
        params.unison_detune_amount.set_value(1.0);
        configure_audio_rate_routes(&params, targets);
        params.set_sample_rate(48_000.0);
        params.snap_smoothers();

        let mut state = KurvDspState {
            block_major_enabled,
            ..KurvDspState::default()
        };
        <Kurv as PluginLogic>::reset(&mut state, &params, &AudioConfig::new(48_000.0, frames));
        let mut input_events = EventList::with_capacity(polyphony as usize);
        for voice in 0..polyphony {
            input_events.push(Event::new(
                0,
                EventBody::NoteOn {
                    group: 0,
                    channel: 1,
                    note: 48 + voice as u8,
                    velocity: 127,
                },
            ));
        }
        let mut output_events = EventList::with_capacity(0);
        let transport = TransportInfo::default();
        let mut context = ProcessContext::new(&transport, 48_000.0, frames, &mut output_events);
        let mut left = vec![0.0; frames];
        let mut right = vec![0.0; frames];
        let start = std::time::Instant::now();
        {
            let inputs: [&[f32]; 0] = [];
            let mut outputs: [&mut [f32]; 2] = [&mut left, &mut right];
            let mut buffer = AudioBuffer::from_slices_checked(&inputs, &mut outputs, frames);
            let _ = <Kurv as PluginLogic>::process(
                &mut state,
                &params,
                &mut buffer,
                &input_events,
                &mut context,
            );
        }
        (
            left.into_iter().zip(right).collect(),
            state.block_major_chunks,
            start.elapsed(),
        )
    }

    fn dense_note_events(tail_pitch_bend: Option<u32>) -> Vec<Event> {
        let mut events = (0..24)
            .map(|voice| {
                Event::new(
                    0,
                    EventBody::NoteOn {
                        group: 0,
                        channel: 1,
                        note: 48 + voice,
                        velocity: 127,
                    },
                )
            })
            .collect::<Vec<_>>();
        if let Some(offset) = tail_pitch_bend {
            events.push(Event::new(
                offset,
                EventBody::PitchBend {
                    group: 0,
                    channel: 1,
                    value: 12_288,
                },
            ));
        }
        events
    }

    fn render_dense_pool_process(
        oversampling_factor: u8,
        frames: usize,
        pool_enabled: bool,
        tail_pitch_bend: Option<u32>,
    ) -> (Vec<(f32, f32)>, usize, usize, [u64; 3]) {
        let params = KurvParams::default();
        params.unison_voices.set_value(64);
        params.voice_mode.set_value(24);
        params
            .oversampling
            .set_value(i64::from(oversampling_factor));
        params.phase_random.set_value(0.0);
        params.unison_swarm.set_value(1.0);
        params.set_sample_rate(48_000.0);
        params.snap_smoothers();

        let mut state = KurvDspState {
            internal_pool_enabled: pool_enabled,
            ..KurvDspState::default()
        };
        <Kurv as PluginLogic>::reset(&mut state, &params, &AudioConfig::new(48_000.0, frames));

        let events = dense_note_events(tail_pitch_bend);
        let mut rendered = Vec::with_capacity(frames * 2);
        for pass in 0..2 {
            let mut input_events =
                EventList::with_capacity(if pass == 0 { events.len() } else { 0 });
            if pass == 0 {
                for event in &events {
                    input_events.push(*event);
                }
            }
            let mut output_events = EventList::with_capacity(0);
            let transport = TransportInfo::default();
            let mut context = ProcessContext::new(&transport, 48_000.0, frames, &mut output_events);
            let mut left = vec![0.0; frames];
            let mut right = vec![0.0; frames];
            {
                let inputs: [&[f32]; 0] = [];
                let mut outputs: [&mut [f32]; 2] = [&mut left, &mut right];
                let mut buffer = AudioBuffer::from_slices_checked(&inputs, &mut outputs, frames);
                let _ = <Kurv as PluginLogic>::process(
                    &mut state,
                    &params,
                    &mut buffer,
                    &input_events,
                    &mut context,
                );
            }
            rendered.extend(left.into_iter().zip(right));
        }
        (
            rendered,
            state.internal_pool_coarse_jobs,
            state.internal_pool_partial_serial_jobs,
            state.internal_pool.worker_participation(),
        )
    }

    #[test]
    fn production_pool_matches_serial_for_24_by_64_jitter() {
        let mut total_participation = 0;
        for factor in 1..=4 {
            let (serial, serial_jobs, _, _) = render_dense_pool_process(factor, 512, false, None);
            let (pooled, pooled_jobs, _, participation) =
                render_dense_pool_process(factor, 512, true, None);
            assert_eq!(pooled, serial, "factor {factor} output and continuation");
            assert_eq!(serial_jobs, 0);
            assert!(
                pooled_jobs > 0,
                "factor {factor} never dispatched a coarse job"
            );
            total_participation += participation.into_iter().sum::<u64>();
        }
        assert!(total_participation > 0, "helpers never claimed a voice");
    }

    #[test]
    fn partial_event_tail_stays_serial() {
        let (serial, serial_jobs, _, _) = render_dense_pool_process(2, 128, false, Some(100));
        let (pooled, pooled_jobs, partial_serial_jobs, participation) =
            render_dense_pool_process(2, 128, true, Some(100));
        assert_eq!(pooled, serial);
        assert_eq!(serial_jobs, 0);
        assert_eq!(pooled_jobs, 0);
        assert!(partial_serial_jobs > 0);
        assert_eq!(participation, [0; 3]);
    }

    #[test]
    fn block_path_respects_pitch_and_mpe_event_boundaries() {
        let events = [
            note_on(0),
            Event::new(
                16,
                EventBody::PitchBend {
                    group: 0,
                    channel: 1,
                    value: 12_288,
                },
            ),
            Event::new(
                32,
                EventBody::PerNotePitchBend {
                    group: 0,
                    channel: 1,
                    note: 60,
                    value: 0x9000_0000,
                },
            ),
            Event::new(
                48,
                EventBody::ParamChange {
                    id: u32::from(P::PitchBend),
                    value: -0.25,
                },
            ),
            Event::new(
                71,
                EventBody::PitchBend2 {
                    group: 0,
                    channel: 1,
                    value: 0x7000_0000,
                },
            ),
        ];
        for factor in 1..=4 {
            assert!(assert_process_paths_equal(&events, false, None, factor) > 0);
        }
    }

    #[test]
    fn block_path_falls_back_for_smoothed_controls_and_release() {
        let held = [note_on(0)];
        for factor in 1..=4 {
            assert_eq!(assert_process_paths_equal(&held, true, None, factor), 0);
        }

        let released = [
            note_on(0),
            Event::new(
                32,
                EventBody::NoteOff {
                    group: 0,
                    channel: 1,
                    note: 60,
                    velocity: 0,
                },
            ),
        ];
        for factor in 1..=4 {
            assert_eq!(
                assert_process_paths_equal(&released, false, None, factor),
                1
            );
        }
    }

    #[test]
    fn audio_rate_modulation_route_matrix_stays_finite_and_sample_accurate() {
        let targets = [
            modulation_target::target_for_param(P::Shape).unwrap(),
            modulation_target::target_for_param(P::Osc1Transpose).unwrap(),
            modulation_target::target_for_param(P::UnisonDetune).unwrap(),
            modulation_target::target_for_param(P::UnisonCurve).unwrap(),
            modulation_target::target_for_param(P::UnisonHarmonicAlign).unwrap(),
            modulation_target::target_for_param(P::OutputDb).unwrap(),
        ];
        for target in targets {
            let (reference, _, _) = render_audio_rate_route_test(target, false, 512);
            let (candidate, _, _) = render_audio_rate_route_test(target, true, 512);
            let maximum_error = candidate
                .iter()
                .zip(&reference)
                .flat_map(
                    |((candidate_left, candidate_right), (reference_left, reference_right))| {
                        [
                            (candidate_left - reference_left).abs(),
                            (candidate_right - reference_right).abs(),
                        ]
                    },
                )
                .fold(0.0_f32, f32::max);
            eprintln!("target={target},max_abs_error={maximum_error:.9e}");
            assert!(maximum_error < 5.0e-2, "target {target}");
            assert!(
                reference
                    .iter()
                    .flat_map(|(left, right)| [left, right])
                    .all(|sample| sample.is_finite()),
                "target {target}"
            );
            assert!(
                reference.windows(2).any(|samples| samples[0] != samples[1]),
                "target {target} is static"
            );
        }
    }

    #[test]
    fn output_modulation_keeps_the_block_renderer_eligible() {
        let output = modulation_target::target_for_param(P::OutputDb).unwrap();
        let (reference, _, _) = render_audio_rate_route_test(output, false, 512);
        let (candidate, chunks, _) = render_audio_rate_route_test(output, true, 512);
        let maximum_error = candidate
            .iter()
            .zip(&reference)
            .flat_map(
                |((candidate_left, candidate_right), (reference_left, reference_right))| {
                    [
                        (candidate_left - reference_left).abs(),
                        (candidate_right - reference_right).abs(),
                    ]
                },
            )
            .fold(0.0_f32, f32::max);
        eprintln!("output_modulation_max_abs_error={maximum_error:.9e}");
        assert!(maximum_error < 1.0e-3);
        assert!(
            chunks > 0,
            "output modulation fell back to per-sample rendering"
        );
    }

    #[test]
    fn audio_rate_modulation_route_matrix_reports_controlled_costs() {
        let targets = [
            (
                "shape",
                modulation_target::target_for_param(P::Shape).unwrap(),
            ),
            (
                "osc-pitch",
                modulation_target::target_for_param(P::Osc1Transpose).unwrap(),
            ),
            (
                "unison-range",
                modulation_target::target_for_param(P::UnisonDetune).unwrap(),
            ),
            (
                "unison-distribution",
                modulation_target::target_for_param(P::UnisonCurve).unwrap(),
            ),
            (
                "unison-align",
                modulation_target::target_for_param(P::UnisonHarmonicAlign).unwrap(),
            ),
            (
                "output",
                modulation_target::target_for_param(P::OutputDb).unwrap(),
            ),
        ];
        for (name, target) in targets {
            let (_, serial_chunks, serial_time) =
                render_audio_rate_route_test(target, false, 4_096);
            let (_, block_chunks, block_time) = render_audio_rate_route_test(target, true, 4_096);
            eprintln!(
                "modulation_route={name},serial_ns_per_frame={:.3},block_ns_per_frame={:.3},serial_block_chunks={serial_chunks},block_block_chunks={block_chunks}",
                serial_time.as_nanos() as f64 / 4_096.0,
                block_time.as_nanos() as f64 / 4_096.0,
            );
        }
    }

    #[test]
    fn audio_rate_modulation_target_catalog_matches_the_scalar_reference() {
        for target in 1..=modulation_target::TARGET_COUNT {
            let descriptor = modulation_target::descriptor(target).unwrap();
            let (reference, _, serial_time) =
                render_audio_rate_route_config_test(&[target], false, 512, 64, 8, 2, 3);
            let (candidate, chunks, block_time) =
                render_audio_rate_route_config_test(&[target], true, 512, 64, 8, 2, 3);
            let maximum_error = candidate
                .iter()
                .zip(&reference)
                .flat_map(
                    |((candidate_left, candidate_right), (reference_left, reference_right))| {
                        [
                            (candidate_left - reference_left).abs(),
                            (candidate_right - reference_right).abs(),
                        ]
                    },
                )
                .fold(0.0_f32, f32::max);
            assert!(
                candidate
                    .iter()
                    .flat_map(|(left, right)| [left, right])
                    .all(|sample| sample.is_finite()),
                "target {target} ({}) produced a non-finite sample",
                descriptor.label
            );
            assert!(
                maximum_error < 5.0e-2,
                "target {target} ({}) diverged by {maximum_error:.9e}",
                descriptor.label
            );
            eprintln!(
                "catalog_target={target},label={},max_abs_error={maximum_error:.9e},serial_ns_per_frame={:.3},block_ns_per_frame={:.3},block_chunks={chunks}",
                descriptor.label,
                serial_time.as_nanos() as f64 / 512.0,
                block_time.as_nanos() as f64 / 512.0,
            );
        }
    }

    #[test]
    fn audio_rate_modulation_block_paths_survive_voice_and_oversampling_stress() {
        let target = |param| modulation_target::target_for_param(param).unwrap();
        let cases: [&[u8]; 5] = [
            &[target(P::Shape), target(P::OutputDb)],
            &[
                target(P::Osc1Transpose),
                target(P::UnisonDetune),
                target(P::UnisonCurve),
                target(P::UnisonHarmonicAlign),
            ],
            &[target(P::PulseWidth), target(P::Attack)],
            &[target(P::UnisonStereo), target(P::UnisonWeight)],
            &[
                target(P::Osc1Transpose),
                target(P::PulseWidth),
                target(P::OutputDb),
            ],
        ];
        let configurations = [(1, 1, 1, 1), (16, 8, 2, 1), (64, 8, 2, 3), (64, 24, 4, 3)];
        for (case_index, targets) in cases.into_iter().enumerate() {
            for &(voices, polyphony, oversampling, oscillators) in &configurations {
                let (reference, _, _) = render_audio_rate_route_config_test(
                    targets,
                    false,
                    1_024,
                    voices,
                    polyphony,
                    oversampling,
                    oscillators,
                );
                let (candidate, chunks, _) = render_audio_rate_route_config_test(
                    targets,
                    true,
                    1_024,
                    voices,
                    polyphony,
                    oversampling,
                    oscillators,
                );
                let maximum_error = candidate
                    .iter()
                    .zip(&reference)
                    .flat_map(
                        |((candidate_left, candidate_right), (reference_left, reference_right))| {
                            [
                                (candidate_left - reference_left).abs(),
                                (candidate_right - reference_right).abs(),
                            ]
                        },
                    )
                    .fold(0.0_f32, f32::max);
                assert!(
                    candidate
                        .iter()
                        .flat_map(|(left, right)| [left, right])
                        .all(|sample| sample.is_finite()),
                    "case {case_index} produced a non-finite sample"
                );
                assert!(
                    maximum_error < 5.0e-2,
                    "case {case_index} voices={voices} polyphony={polyphony} oversampling={oversampling} oscillators={oscillators} diverged by {maximum_error:.9e}"
                );
                assert!(
                    chunks > 0,
                    "case {case_index} voices={voices} polyphony={polyphony} oversampling={oversampling} oscillators={oscillators} did not use a block path"
                );
            }
        }
    }

    #[test]
    fn adaptive_wander_blocks_match_the_pair_path() {
        for factor in 1..=4 {
            assert!(
                assert_process_paths_equal(&[note_on(0)], false, Some(SwarmMode::Wander), factor,)
                    > 0
            );
        }
    }

    #[test]
    fn voice_renders_silence_when_idle() {
        let mut voice = VaVoice::default();
        voice.set_sample_rate(48_000.0);
        let settings = VoiceSettings::new(2.0, 220.0, 0.5, 1.0, 0.0, 0.0);

        let (l, r) = voice.render(settings, 48_000.0, false);
        assert_eq!(l, 0.0);
        assert_eq!(r, 0.0);
    }

    #[test]
    fn voice_renders_nonzero_in_drone_mode() {
        let mut voice = VaVoice::default();
        voice.set_sample_rate(48_000.0);
        let settings = VoiceSettings::new(2.0, 220.0, 0.5, 1.0, 0.0, 0.0);

        let mut peak = 0.0_f32;
        for _ in 0..512 {
            let (l, r) = voice.render(settings, 48_000.0, true);
            peak = peak.max(l.abs()).max(r.abs());
        }

        assert!(peak.is_finite());
        assert!(peak > 0.01);
    }

    #[test]
    fn phase_randomization_changes_a_new_notes_start_phase() {
        let settings = VoiceSettings::new(0.0, 220.0, 0.5, 1.0, 0.0, 0.0);
        let mut fixed = VaVoice::default();
        fixed.set_sample_rate(48_000.0);
        fixed.configure_unison(UnisonSettings::new(1, 0.0, 0.0, 0.0, 0.0));
        fixed.start(57, 1.0, 0, None, 1);

        let mut randomized = VaVoice::default();
        randomized.set_sample_rate(48_000.0);
        randomized.configure_unison(UnisonSettings::new(1, 0.0, 0.0, 1.0, 0.0));
        randomized.start(57, 1.0, 0, None, 1);

        let (fixed_sample, _) = fixed.render(settings, 48_000.0, false);
        let (random_sample, _) = randomized.render(settings, 48_000.0, false);
        assert!((fixed_sample - random_sample).abs() > 1.0e-5);
    }
}
