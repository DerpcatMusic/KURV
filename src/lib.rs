use std::sync::{Arc, Mutex, atomic::AtomicU64};

use truce::prelude::*;
use truce_core::midi::{norm_7bit, norm_pitch_bend, per_note_bend_semitones};

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
mod lfo;
mod modulation_target;
mod oscillator;
mod oversampling;
mod pan_curve;
mod performance;
mod voice;
mod wave_curve;

use lfo::{LFO_COUNT, LfoBank, LfoConfig, LfoMode, LfoRateMode, ROUTE_COUNT, RouteConfig};
use oscillator::{Antialiasing, PhaseWarpMode};
use oversampling::{DEFAULT_FACTOR, StereoOversampler};
use pan_curve::{PanShapeCurveData, PanShapeCurveState, PanShapeSegmentsRt};
#[cfg(test)]
use voice::VaVoice;
use voice::{
    BLOCK_INTERNAL_SAMPLES, EnvelopeSettings, FACTOR3_BLOCK_INTERNAL_SAMPLES, InternalRtPool,
    MAX_JOB_SAMPLES, OscillatorSettings, PanShapeSettings, PolySynth, SwarmMode, UnisonSettings,
    VoiceSettings,
};
use wave_curve::{WaveCurveRt, WaveCurveState};

const CONTROL_BLOCK: usize = 1_024;

#[derive(Clone, PartialEq, State)]
pub struct KurvEditorState {
    pub width: u32,
    pub height: u32,
    pub ui_scale: u8,
    pub theme_schema: u8,
    pub theme_preset: u8,
    pub background_red: u8,
    pub background_green: u8,
    pub background_blue: u8,
    pub theme_tint: u8,
    pub theme_contrast: u8,
    pub primary_red: u8,
    pub primary_green: u8,
    pub primary_blue: u8,
    pub secondary_red: u8,
    pub secondary_green: u8,
    pub secondary_blue: u8,
    pub tertiary_red: u8,
    pub tertiary_green: u8,
    pub tertiary_blue: u8,
}

impl Default for KurvEditorState {
    fn default() -> Self {
        Self {
            width: 1120,
            height: 720,
            ui_scale: 1,
            theme_schema: 2,
            theme_preset: 0,
            background_red: 18,
            background_green: 20,
            background_blue: 23,
            theme_tint: 8,
            theme_contrast: 100,
            primary_red: 38,
            primary_green: 210,
            primary_blue: 204,
            secondary_red: 245,
            secondary_green: 173,
            secondary_blue: 71,
            tertiary_red: 176,
            tertiary_green: 126,
            tertiary_blue: 247,
        }
    }
}

#[derive(Clone, Copy)]
struct UnisonPitchControlBlock {
    detune_cents: [f32; CONTROL_BLOCK],
    detune_amount: [f32; CONTROL_BLOCK],
    harmonic_align: [f32; CONTROL_BLOCK],
    curve: [f32; CONTROL_BLOCK],
    stereo: [f32; CONTROL_BLOCK],
    stereo_x: [f32; CONTROL_BLOCK],
    stereo_y: [f32; CONTROL_BLOCK],
    weight: [f32; CONTROL_BLOCK],
}

impl Default for UnisonPitchControlBlock {
    fn default() -> Self {
        Self {
            detune_cents: [0.0; CONTROL_BLOCK],
            detune_amount: [0.0; CONTROL_BLOCK],
            harmonic_align: [0.0; CONTROL_BLOCK],
            curve: [0.0; CONTROL_BLOCK],
            stereo: [0.0; CONTROL_BLOCK],
            stereo_x: [0.0; CONTROL_BLOCK],
            stereo_y: [0.0; CONTROL_BLOCK],
            weight: [0.0; CONTROL_BLOCK],
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
            && slice_is_static(&self.stereo[start..end])
            && slice_is_static(&self.stereo_x[start..end])
            && slice_is_static(&self.stereo_y[start..end])
            && slice_is_static(&self.weight[start..end])
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
}

#[derive(Clone, Copy, Default)]
struct ActiveRoute {
    config: RouteConfig,
    amount_index: usize,
    descriptor: Option<modulation_target::TargetDescriptor>,
}

struct ActiveRoutes {
    entries: [ActiveRoute; ROUTE_COUNT],
    len: usize,
    source_mask: u8,
    unison_layout_mask: u8,
}

impl Default for ActiveRoutes {
    fn default() -> Self {
        Self {
            entries: [ActiveRoute::default(); ROUTE_COUNT],
            len: 0,
            source_mask: 0,
            unison_layout_mask: 0,
        }
    }
}

impl ActiveRoutes {
    fn as_slice(&self) -> &[ActiveRoute] {
        &self.entries[..self.len]
    }
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
                &params.unison_stereo,
                &params.stereo_x,
                &params.stereo_alternate,
                &params.unison_weight,
            ),
            (
                &params.osc2_unison_detune,
                &params.osc2_unison_detune_amount,
                &params.osc2_unison_harmonic_align,
                &params.osc2_unison_curve,
                &params.osc2_unison_stereo,
                &params.osc2_stereo_x,
                &params.osc2_stereo_alternate,
                &params.osc2_unison_weight,
            ),
            (
                &params.osc3_unison_detune,
                &params.osc3_unison_detune_amount,
                &params.osc3_unison_harmonic_align,
                &params.osc3_unison_curve,
                &params.osc3_unison_stereo,
                &params.osc3_stereo_x,
                &params.osc3_stereo_alternate,
                &params.osc3_unison_weight,
            ),
        ];
        for (control, (detune, amount, align, curve, stereo, stereo_x, stereo_y, weight)) in
            self.unison_pitch.iter_mut().zip(unison_pitch_params)
        {
            detune.read_into(&mut control.detune_cents[..len]);
            amount.read_into(&mut control.detune_amount[..len]);
            align.read_into(&mut control.harmonic_align[..len]);
            curve.read_into(&mut control.curve[..len]);
            stereo.read_into(&mut control.stereo[..len]);
            stereo_x.read_into(&mut control.stereo_x[..len]);
            stereo_y.read_into(&mut control.stereo_y[..len]);
            weight.read_into(&mut control.weight[..len]);
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
        (self.output_db[0].to_bits() == self.output_db[len - 1].to_bits())
            .then(|| db_to_linear(self.output_db[0]))
    }

    fn active_lfo_mask(&self, routes: &ActiveRoutes, len: usize) -> u8 {
        routes.as_slice().iter().fold(0, |mask, route| {
            if self.modulation_amounts[route.amount_index][..len]
                .iter()
                .any(|amount| amount.abs() > f32::EPSILON)
            {
                mask | (1_u8 << (route.config.source - 1))
            } else {
                mask
            }
        })
    }

    fn lfo_controls_static(&self, mask: u8, len: usize, configs: &[LfoConfig; LFO_COUNT]) -> bool {
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
                return false;
            }
        }
        true
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
                    .all(|value| value.to_bits() == base.level_curve().to_bits());
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

fn slice_is_static(values: &[f32]) -> bool {
    let bits = values[0].to_bits();
    values[1..].iter().all(|value| value.to_bits() == bits)
}

#[derive(Params)]
pub struct KurvParams {
    #[param(
        id = 0,
        name = "Output",
        range = "linear(-48, 6)",
        default = -9.0,
        unit = "dB",
        smooth = "linear(5)"
    )]
    pub output_db: FloatParam,

    #[param(
        id = 1,
        name = "Shape",
        range = "linear(0, 3)",
        default = 2.0,
        smooth = "linear(8)",
        format = "format_shape"
    )]
    pub shape: FloatParam,

    #[param(
        id = 2,
        name = "Pulse Width",
        short_name = "Pulse",
        range = "linear(0.03, 0.97)",
        default = 0.5,
        smooth = "linear(8)"
    )]
    pub pulse_width: FloatParam,

    #[param(
        id = 3,
        name = "Attack",
        range = "skewed(0, 8, 0.25)",
        default = 0.002,
        unit = "s"
    )]
    pub attack: FloatParam,

    #[param(
        id = 4,
        name = "Decay",
        range = "skewed(0, 8, 0.25)",
        default = 0.1,
        unit = "s"
    )]
    pub decay: FloatParam,

    #[param(
        id = 5,
        name = "Sustain Level",
        short_name = "Sustain",
        range = "linear(0, 1)",
        default = 1.0,
        unit = "%",
        smooth = "linear(15)"
    )]
    pub sustain: FloatParam,

    #[param(
        id = 6,
        name = "Release",
        range = "skewed(0, 12, 0.25)",
        default = 0.002,
        unit = "s"
    )]
    pub release: FloatParam,

    #[param(
        id = 7,
        name = "Legacy Audition Drone",
        default = false,
        flags = "hidden | automatable"
    )]
    pub drone: BoolParam,

    #[param(
        id = 8,
        name = "Drone Frequency",
        short_name = "Drone Hz",
        range = "log(20, 5000)",
        default = 110.0,
        unit = "Hz",
        smooth = "linear(20)",
        flags = "hidden | automatable"
    )]
    pub drone_frequency: FloatParam,

    #[param(
        id = 9,
        name = "Pitch Bend",
        range = "linear(-1, 1)",
        flags = "hidden | automatable"
    )]
    pub pitch_bend: FloatParam,

    #[param(
        id = 10,
        name = "Sustain Pedal",
        range = "linear(0, 1)",
        flags = "hidden | automatable",
        midi_cc = 64
    )]
    pub sustain_pedal: FloatParam,

    #[param(
        id = 11,
        name = "Unison Voices",
        short_name = "Voices",
        range = "discrete(1, 64)",
        default = 1
    )]
    pub unison_voices: IntParam,

    #[param(
        id = 12,
        name = "Unison Pitch Range",
        short_name = "Range",
        range = "linear(0, 48)",
        default = 1.0,
        format = "format_semitones"
    )]
    pub unison_detune: FloatParam,

    #[param(
        id = 244,
        name = "Unison Alignment",
        short_name = "Alignment",
        range = "linear(0, 1)",
        default = 0.0,
        unit = "%"
    )]
    pub unison_harmonic_align: FloatParam,

    #[param(
        id = 247,
        name = "Unison Alignment Mode",
        short_name = "Align Mode",
        range = "discrete(0, 3)",
        default = 0,
        format = "format_unison_alignment_mode"
    )]
    pub unison_alignment_mode: IntParam,

    #[param(
        id = 13,
        name = "Unison Stereo",
        short_name = "Stereo",
        range = "linear(0, 1)",
        default = 1.0,
        unit = "%"
    )]
    pub unison_stereo: FloatParam,

    #[param(
        id = 14,
        name = "Rand Phase",
        short_name = "Rand Phase",
        range = "linear(0, 1)",
        default = 1.0,
        unit = "%"
    )]
    pub phase_random: FloatParam,

    #[param(
        id = 15,
        name = "Unison Pitch Distribution",
        short_name = "Pitch Curve",
        range = "linear(-1, 1)",
        default = 0.4329594,
        format = "format_unison_curve"
    )]
    pub unison_curve: FloatParam,

    #[param(
        id = 16,
        name = "Velocity Amount",
        short_name = "Velocity",
        range = "linear(0, 1)",
        default = 1.0,
        unit = "%",
        smooth = "linear(10)"
    )]
    pub velocity_amount: FloatParam,

    #[param(
        id = 17,
        name = "Pressure Amount",
        short_name = "Pressure",
        range = "linear(0, 1)",
        default = 0.35,
        unit = "%",
        smooth = "linear(10)"
    )]
    pub pressure_amount: FloatParam,

    #[param(
        id = 18,
        name = "MPE Timbre Amount",
        short_name = "Timbre",
        range = "linear(0, 1)",
        default = 0.5,
        unit = "%",
        smooth = "linear(10)"
    )]
    pub timbre_amount: FloatParam,

    #[param(
        id = 19,
        name = "MPE Pitch Bend Range",
        short_name = "MPE Bend",
        range = "discrete(1, 96)",
        default = 48,
        unit = "st"
    )]
    pub mpe_bend_range: IntParam,

    #[param(
        id = 20,
        name = "Unison Jitter Amount",
        short_name = "Jitter",
        range = "linear(0, 1)",
        default = 0.0,
        unit = "%"
    )]
    pub unison_swarm: FloatParam,

    #[param(
        id = 21,
        name = "Unison Jitter Rate",
        short_name = "Jitter Rate",
        range = "log(0.02, 100)",
        default = 0.7,
        format = "format_swarm_rate"
    )]
    pub unison_swarm_rate: FloatParam,

    #[param(
        id = 47,
        name = "Legacy Unison Motion Mode",
        short_name = "Legacy Motion",
        range = "discrete(0, 1)",
        default = 0,
        flags = "hidden"
    )]
    pub legacy_unison_swarm_mode: IntParam,

    #[param(
        id = 118,
        name = "Unison Jitter Mode",
        short_name = "Jitter Mode",
        range = "discrete(0, 1)",
        default = 0,
        format = "format_swarm_mode"
    )]
    pub unison_swarm_mode: IntParam,

    #[param(
        id = 22,
        name = "Legacy Stereo Layout",
        short_name = "Legacy Layout",
        range = "discrete(0, 3)",
        default = 1.0,
        format = "format_stereo_pattern",
        flags = "hidden | automatable"
    )]
    pub stereo_pattern: FloatParam,

    #[param(
        id = 24,
        name = "Attack Curve",
        short_name = "Attack Curve",
        range = "linear(-1, 1)",
        default = 0.0,
        format = "format_envelope_curve"
    )]
    pub attack_curve: FloatParam,

    #[param(
        id = 25,
        name = "Decay Curve",
        short_name = "Decay Curve",
        range = "linear(-1, 1)",
        default = 0.0,
        format = "format_envelope_curve"
    )]
    pub decay_curve: FloatParam,

    #[param(
        id = 26,
        name = "Release Curve",
        short_name = "Release Curve",
        range = "linear(-1, 1)",
        default = 0.0,
        format = "format_envelope_curve"
    )]
    pub release_curve: FloatParam,

    #[param(
        id = 27,
        name = "Attack Curve Time",
        short_name = "Attack Curve X",
        range = "linear(-1, 1)",
        default = 0.0,
        format = "format_envelope_curve_time"
    )]
    pub attack_curve_time: FloatParam,

    #[param(
        id = 28,
        name = "Decay Curve Time",
        short_name = "Decay Curve X",
        range = "linear(-1, 1)",
        default = 0.0,
        format = "format_envelope_curve_time"
    )]
    pub decay_curve_time: FloatParam,

    #[param(
        id = 29,
        name = "Release Curve Time",
        short_name = "Release Curve X",
        range = "linear(-1, 1)",
        default = 0.0,
        format = "format_envelope_curve_time"
    )]
    pub release_curve_time: FloatParam,

    #[param(
        id = 30,
        name = "Stereo Square Y",
        short_name = "Stereo Y",
        range = "linear(0, 1)",
        default = 1.0,
        unit = "%"
    )]
    pub stereo_alternate: FloatParam,

    #[param(
        id = 31,
        name = "Stereo Square X",
        short_name = "Stereo X",
        range = "linear(0, 1)",
        default = 0.0,
        unit = "%"
    )]
    pub stereo_x: FloatParam,

    #[param(
        id = 32,
        name = "Unison Voice Weight",
        short_name = "Voice Weight",
        range = "linear(-1, 1)",
        default = 0.0,
        format = "format_unison_weight"
    )]
    pub unison_weight: FloatParam,

    #[param(
        id = 33,
        name = "Oscillator Quality",
        short_name = "Quality",
        range = "discrete(1, 4)",
        default = 2,
        format = "format_oversampling",
        flags = "hidden",
        chunk = false
    )]
    pub oversampling: IntParam,

    #[param(
        id = 34,
        name = "Unison Detune Amount",
        short_name = "Detune Amount",
        range = "linear(0, 1)",
        default = 1.0,
        unit = "%"
    )]
    pub unison_detune_amount: FloatParam,

    #[param(
        id = 35,
        name = "Shape Center",
        short_name = "Shape Center",
        range = "linear(0, 1)",
        default = 0.0,
        unit = "%"
    )]
    pub pan_shape_center: FloatParam,

    #[param(
        id = 36,
        name = "Shape Edge",
        short_name = "Shape Edge",
        range = "linear(0, 1)",
        default = 1.0,
        unit = "%"
    )]
    pub pan_shape_edge: FloatParam,

    #[param(
        id = 37,
        name = "Shape Curve",
        short_name = "Shape Curve",
        range = "linear(-1, 1)",
        default = 0.35,
        format = "format_envelope_curve"
    )]
    pub pan_shape_curve: FloatParam,

    #[param(
        id = 39,
        name = "Shape Curve Time",
        short_name = "Shape Curve Time",
        range = "linear(0.05, 0.95)",
        default = 0.5,
        unit = "%",
        flags = "hidden"
    )]
    pub pan_shape_curve_time: FloatParam,

    #[param(
        id = 38,
        name = "Antialiasing",
        short_name = "AA",
        range = "discrete(0, 2)",
        default = 1,
        format = "format_antialiasing",
        flags = "hidden",
        chunk = false
    )]
    pub antialiasing: IntParam,

    #[param(
        id = 48,
        name = "Generator Engine",
        short_name = "Engine",
        range = "discrete(0, 1)",
        default = 0,
        format = "format_generator_engine",
        flags = "hidden",
        chunk = false
    )]
    pub generator_engine: IntParam,

    #[param(
        id = 40,
        name = "Shape Left Edge",
        short_name = "Shape Left",
        range = "linear(0, 1)",
        default = 1.0,
        unit = "%"
    )]
    pub pan_shape_left: FloatParam,

    #[param(
        id = 41,
        name = "Shape Right Edge",
        short_name = "Shape Right",
        range = "linear(0, 1)",
        default = 1.0,
        unit = "%"
    )]
    pub pan_shape_right: FloatParam,

    #[param(
        id = 42,
        name = "Shape Left Curve",
        short_name = "Shape Left Curve",
        range = "linear(-1, 1)",
        default = 0.35,
        format = "format_envelope_curve"
    )]
    pub pan_shape_left_curve: FloatParam,

    #[param(
        id = 43,
        name = "Shape Right Curve",
        short_name = "Shape Right Curve",
        range = "linear(-1, 1)",
        default = 0.35,
        format = "format_envelope_curve"
    )]
    pub pan_shape_right_curve: FloatParam,

    #[param(
        id = 44,
        name = "Shape Left Curve Time",
        short_name = "Shape Left Curve X",
        range = "linear(0.05, 0.95)",
        default = 0.5,
        unit = "%",
        flags = "hidden"
    )]
    pub pan_shape_left_curve_time: FloatParam,

    #[param(
        id = 45,
        name = "Shape Right Curve Time",
        short_name = "Shape Right Curve X",
        range = "linear(0.05, 0.95)",
        default = 0.5,
        unit = "%",
        flags = "hidden"
    )]
    pub pan_shape_right_curve_time: FloatParam,

    #[param(
        id = 46,
        name = "Shape Center X",
        short_name = "Shape Center X",
        range = "linear(0.05, 0.95)",
        default = 0.5,
        unit = "%",
        flags = "hidden"
    )]
    pub pan_shape_center_x: FloatParam,

    #[param(
        id = 49,
        name = "Transpose Semitones",
        short_name = "Transpose",
        range = "discrete(-12, 12)",
        default = 0,
        format = "format_signed_semitones"
    )]
    pub transpose: IntParam,

    #[param(
        id = 50,
        name = "Octave Shift",
        short_name = "Octave",
        range = "discrete(-4, 4)",
        default = 0,
        format = "format_octaves"
    )]
    pub octave_shift: IntParam,

    #[param(
        id = 51,
        name = "Voice Mode",
        short_name = "Voices",
        range = "discrete(0, 32)",
        default = 32,
        format = "format_voice_mode"
    )]
    pub voice_mode: IntParam,

    #[param(
        id = 52,
        name = "Legato Glide Time",
        short_name = "Glide",
        range = "skewed(0, 5, 0.2)",
        default = 0.08,
        unit = "s",
        format = "format_glide_time"
    )]
    pub glide_time: FloatParam,

    #[param(
        id = 53,
        name = "Oscillator 1 Enabled",
        short_name = "Osc 1",
        default = true
    )]
    pub osc1_enabled: BoolParam,

    #[param(
        id = 54,
        name = "Oscillator 1 Transpose",
        short_name = "Osc 1 Tune",
        range = "discrete(-48, 48)",
        default = 0,
        format = "format_signed_semitones"
    )]
    pub osc1_transpose: IntParam,

    #[param(
        id = 55,
        name = "Oscillator 1 Cents",
        short_name = "Osc 1 Fine",
        range = "linear(-100, 100)",
        default = 0.0,
        format = "format_cents"
    )]
    pub osc1_cents: FloatParam,

    #[param(
        id = 56,
        name = "Oscillator 1 Level",
        short_name = "Osc 1 Level",
        range = "linear(0, 1)",
        default = 1.0,
        unit = "%",
        smooth = "linear(5)"
    )]
    pub osc1_level: FloatParam,

    #[param(
        id = 57,
        name = "Oscillator 1 Pan",
        short_name = "Osc 1 Pan",
        range = "linear(-1, 1)",
        default = 0.0,
        unit = "pan",
        smooth = "linear(5)"
    )]
    pub osc1_pan: FloatParam,

    #[param(
        id = 58,
        name = "Oscillator 2 Enabled",
        short_name = "Osc 2",
        default = false
    )]
    pub osc2_enabled: BoolParam,

    #[param(
        id = 59,
        name = "Oscillator 2 Shape",
        short_name = "Osc 2 Shape",
        range = "linear(0, 3)",
        default = 2.0,
        smooth = "linear(8)",
        format = "format_shape"
    )]
    pub osc2_shape: FloatParam,

    #[param(
        id = 60,
        name = "Oscillator 2 Pulse Width",
        short_name = "Osc 2 PWM",
        range = "linear(0.03, 0.97)",
        default = 0.5,
        smooth = "linear(8)"
    )]
    pub osc2_pulse_width: FloatParam,

    #[param(
        id = 61,
        name = "Oscillator 2 Transpose",
        short_name = "Osc 2 Tune",
        range = "discrete(-48, 48)",
        default = 0,
        format = "format_signed_semitones"
    )]
    pub osc2_transpose: IntParam,

    #[param(
        id = 62,
        name = "Oscillator 2 Cents",
        short_name = "Osc 2 Fine",
        range = "linear(-100, 100)",
        default = 0.0,
        format = "format_cents"
    )]
    pub osc2_cents: FloatParam,

    #[param(
        id = 63,
        name = "Oscillator 2 Level",
        short_name = "Osc 2 Level",
        range = "linear(0, 1)",
        default = 1.0,
        unit = "%",
        smooth = "linear(5)"
    )]
    pub osc2_level: FloatParam,

    #[param(
        id = 64,
        name = "Oscillator 2 Pan",
        short_name = "Osc 2 Pan",
        range = "linear(-1, 1)",
        default = 0.0,
        unit = "pan",
        smooth = "linear(5)"
    )]
    pub osc2_pan: FloatParam,

    #[param(
        id = 65,
        name = "Oscillator 3 Enabled",
        short_name = "Osc 3",
        default = false
    )]
    pub osc3_enabled: BoolParam,

    #[param(
        id = 66,
        name = "Oscillator 3 Shape",
        short_name = "Osc 3 Shape",
        range = "linear(0, 3)",
        default = 2.0,
        smooth = "linear(8)",
        format = "format_shape"
    )]
    pub osc3_shape: FloatParam,

    #[param(
        id = 67,
        name = "Oscillator 3 Pulse Width",
        short_name = "Osc 3 PWM",
        range = "linear(0.03, 0.97)",
        default = 0.5,
        smooth = "linear(8)"
    )]
    pub osc3_pulse_width: FloatParam,

    #[param(
        id = 68,
        name = "Oscillator 3 Transpose",
        short_name = "Osc 3 Tune",
        range = "discrete(-48, 48)",
        default = 0,
        format = "format_signed_semitones"
    )]
    pub osc3_transpose: IntParam,

    #[param(
        id = 69,
        name = "Oscillator 3 Cents",
        short_name = "Osc 3 Fine",
        range = "linear(-100, 100)",
        default = 0.0,
        format = "format_cents"
    )]
    pub osc3_cents: FloatParam,

    #[param(
        id = 70,
        name = "Oscillator 3 Level",
        short_name = "Osc 3 Level",
        range = "linear(0, 1)",
        default = 1.0,
        unit = "%",
        smooth = "linear(5)"
    )]
    pub osc3_level: FloatParam,

    #[param(
        id = 71,
        name = "Oscillator 3 Pan",
        short_name = "Osc 3 Pan",
        range = "linear(-1, 1)",
        default = 0.0,
        unit = "pan",
        smooth = "linear(5)"
    )]
    pub osc3_pan: FloatParam,

    #[param(
        id = 72,
        name = "Oscillator 2 Unison Voices",
        short_name = "Osc 2 Voices",
        range = "discrete(1, 64)",
        default = 1
    )]
    pub osc2_unison_voices: IntParam,

    #[param(
        id = 73,
        name = "Oscillator 2 Unison Pitch Range",
        short_name = "Osc 2 Range",
        range = "linear(0, 48)",
        default = 1.0,
        format = "format_semitones"
    )]
    pub osc2_unison_detune: FloatParam,

    #[param(
        id = 245,
        name = "Oscillator 2 Unison Alignment",
        short_name = "Osc 2 Alignment",
        range = "linear(0, 1)",
        default = 0.0,
        unit = "%"
    )]
    pub osc2_unison_harmonic_align: FloatParam,

    #[param(
        id = 248,
        name = "Oscillator 2 Unison Alignment Mode",
        short_name = "Osc 2 Align Mode",
        range = "discrete(0, 3)",
        default = 0,
        format = "format_unison_alignment_mode"
    )]
    pub osc2_unison_alignment_mode: IntParam,

    #[param(
        id = 74,
        name = "Oscillator 2 Unison Detune Amount",
        short_name = "Osc 2 Detune",
        range = "linear(0, 1)",
        default = 1.0,
        unit = "%"
    )]
    pub osc2_unison_detune_amount: FloatParam,

    #[param(
        id = 75,
        name = "Oscillator 2 Unison Stereo",
        short_name = "Osc 2 Stereo",
        range = "linear(0, 1)",
        default = 1.0,
        unit = "%"
    )]
    pub osc2_unison_stereo: FloatParam,

    #[param(
        id = 76,
        name = "Oscillator 2 Rand Phase",
        short_name = "Osc 2 Phase",
        range = "linear(0, 1)",
        default = 1.0,
        unit = "%"
    )]
    pub osc2_phase_random: FloatParam,

    #[param(
        id = 77,
        name = "Oscillator 2 Unison Pitch Distribution",
        short_name = "Osc 2 Curve",
        range = "linear(-1, 1)",
        default = 0.4329594,
        format = "format_unison_curve"
    )]
    pub osc2_unison_curve: FloatParam,

    #[param(
        id = 78,
        name = "Oscillator 2 Unison Jitter Amount",
        short_name = "Osc 2 Jitter",
        range = "linear(0, 1)",
        default = 0.0,
        unit = "%"
    )]
    pub osc2_unison_jitter: FloatParam,

    #[param(
        id = 79,
        name = "Oscillator 2 Unison Jitter Rate",
        short_name = "Osc 2 Jitter Rate",
        range = "log(0.02, 100)",
        default = 0.7,
        format = "format_swarm_rate"
    )]
    pub osc2_unison_jitter_rate: FloatParam,

    #[param(
        id = 80,
        name = "Oscillator 2 Stereo Square Y",
        short_name = "Osc 2 Stereo Y",
        range = "linear(0, 1)",
        default = 1.0,
        unit = "%"
    )]
    pub osc2_stereo_alternate: FloatParam,

    #[param(
        id = 81,
        name = "Oscillator 2 Stereo Square X",
        short_name = "Osc 2 Stereo X",
        range = "linear(0, 1)",
        default = 0.0,
        unit = "%"
    )]
    pub osc2_stereo_x: FloatParam,

    #[param(
        id = 82,
        name = "Oscillator 2 Unison Voice Weight",
        short_name = "Osc 2 Weight",
        range = "linear(-1, 1)",
        default = 0.0,
        format = "format_unison_weight"
    )]
    pub osc2_unison_weight: FloatParam,

    #[param(
        id = 83,
        name = "Oscillator 2 Shape Center",
        short_name = "Osc 2 Center",
        range = "linear(0, 1)",
        default = 0.0,
        unit = "%"
    )]
    pub osc2_pan_shape_center: FloatParam,

    #[param(
        id = 84,
        name = "Oscillator 2 Shape Left Edge",
        short_name = "Osc 2 Left",
        range = "linear(0, 1)",
        default = 1.0,
        unit = "%"
    )]
    pub osc2_pan_shape_left: FloatParam,

    #[param(
        id = 85,
        name = "Oscillator 2 Shape Right Edge",
        short_name = "Osc 2 Right",
        range = "linear(0, 1)",
        default = 1.0,
        unit = "%"
    )]
    pub osc2_pan_shape_right: FloatParam,

    #[param(
        id = 86,
        name = "Oscillator 2 Shape Left Curve",
        short_name = "Osc 2 Left Curve",
        range = "linear(-1, 1)",
        default = 0.35,
        format = "format_envelope_curve"
    )]
    pub osc2_pan_shape_left_curve: FloatParam,

    #[param(
        id = 87,
        name = "Oscillator 2 Shape Right Curve",
        short_name = "Osc 2 Right Curve",
        range = "linear(-1, 1)",
        default = 0.35,
        format = "format_envelope_curve"
    )]
    pub osc2_pan_shape_right_curve: FloatParam,

    #[param(
        id = 88,
        name = "Oscillator 2 Shape Left Curve Time",
        short_name = "Osc 2 Left Curve X",
        range = "linear(0.05, 0.95)",
        default = 0.5,
        unit = "%",
        flags = "hidden"
    )]
    pub osc2_pan_shape_left_curve_time: FloatParam,

    #[param(
        id = 89,
        name = "Oscillator 2 Shape Right Curve Time",
        short_name = "Osc 2 Right Curve X",
        range = "linear(0.05, 0.95)",
        default = 0.5,
        unit = "%",
        flags = "hidden"
    )]
    pub osc2_pan_shape_right_curve_time: FloatParam,

    #[param(
        id = 90,
        name = "Oscillator 2 Shape Center X",
        short_name = "Osc 2 Center X",
        range = "linear(0.05, 0.95)",
        default = 0.5,
        unit = "%",
        flags = "hidden"
    )]
    pub osc2_pan_shape_center_x: FloatParam,

    #[param(
        id = 91,
        name = "Oscillator 3 Unison Voices",
        short_name = "Osc 3 Voices",
        range = "discrete(1, 64)",
        default = 1
    )]
    pub osc3_unison_voices: IntParam,

    #[param(
        id = 92,
        name = "Oscillator 3 Unison Pitch Range",
        short_name = "Osc 3 Range",
        range = "linear(0, 48)",
        default = 1.0,
        format = "format_semitones"
    )]
    pub osc3_unison_detune: FloatParam,

    #[param(
        id = 246,
        name = "Oscillator 3 Unison Alignment",
        short_name = "Osc 3 Alignment",
        range = "linear(0, 1)",
        default = 0.0,
        unit = "%"
    )]
    pub osc3_unison_harmonic_align: FloatParam,

    #[param(
        id = 249,
        name = "Oscillator 3 Unison Alignment Mode",
        short_name = "Osc 3 Align Mode",
        range = "discrete(0, 3)",
        default = 0,
        format = "format_unison_alignment_mode"
    )]
    pub osc3_unison_alignment_mode: IntParam,

    #[param(
        id = 93,
        name = "Oscillator 3 Unison Detune Amount",
        short_name = "Osc 3 Detune",
        range = "linear(0, 1)",
        default = 1.0,
        unit = "%"
    )]
    pub osc3_unison_detune_amount: FloatParam,

    #[param(
        id = 94,
        name = "Oscillator 3 Unison Stereo",
        short_name = "Osc 3 Stereo",
        range = "linear(0, 1)",
        default = 1.0,
        unit = "%"
    )]
    pub osc3_unison_stereo: FloatParam,

    #[param(
        id = 95,
        name = "Oscillator 3 Rand Phase",
        short_name = "Osc 3 Phase",
        range = "linear(0, 1)",
        default = 1.0,
        unit = "%"
    )]
    pub osc3_phase_random: FloatParam,

    #[param(
        id = 96,
        name = "Oscillator 3 Unison Pitch Distribution",
        short_name = "Osc 3 Curve",
        range = "linear(-1, 1)",
        default = 0.4329594,
        format = "format_unison_curve"
    )]
    pub osc3_unison_curve: FloatParam,

    #[param(
        id = 97,
        name = "Oscillator 3 Unison Jitter Amount",
        short_name = "Osc 3 Jitter",
        range = "linear(0, 1)",
        default = 0.0,
        unit = "%"
    )]
    pub osc3_unison_jitter: FloatParam,

    #[param(
        id = 98,
        name = "Oscillator 3 Unison Jitter Rate",
        short_name = "Osc 3 Jitter Rate",
        range = "log(0.02, 100)",
        default = 0.7,
        format = "format_swarm_rate"
    )]
    pub osc3_unison_jitter_rate: FloatParam,

    #[param(
        id = 99,
        name = "Oscillator 3 Stereo Square Y",
        short_name = "Osc 3 Stereo Y",
        range = "linear(0, 1)",
        default = 1.0,
        unit = "%"
    )]
    pub osc3_stereo_alternate: FloatParam,

    #[param(
        id = 100,
        name = "Oscillator 3 Stereo Square X",
        short_name = "Osc 3 Stereo X",
        range = "linear(0, 1)",
        default = 0.0,
        unit = "%"
    )]
    pub osc3_stereo_x: FloatParam,

    #[param(
        id = 101,
        name = "Oscillator 3 Unison Voice Weight",
        short_name = "Osc 3 Weight",
        range = "linear(-1, 1)",
        default = 0.0,
        format = "format_unison_weight"
    )]
    pub osc3_unison_weight: FloatParam,

    #[param(
        id = 102,
        name = "Oscillator 3 Shape Center",
        short_name = "Osc 3 Center",
        range = "linear(0, 1)",
        default = 0.0,
        unit = "%"
    )]
    pub osc3_pan_shape_center: FloatParam,

    #[param(
        id = 103,
        name = "Oscillator 3 Shape Left Edge",
        short_name = "Osc 3 Left",
        range = "linear(0, 1)",
        default = 1.0,
        unit = "%"
    )]
    pub osc3_pan_shape_left: FloatParam,

    #[param(
        id = 104,
        name = "Oscillator 3 Shape Right Edge",
        short_name = "Osc 3 Right",
        range = "linear(0, 1)",
        default = 1.0,
        unit = "%"
    )]
    pub osc3_pan_shape_right: FloatParam,

    #[param(
        id = 105,
        name = "Oscillator 3 Shape Left Curve",
        short_name = "Osc 3 Left Curve",
        range = "linear(-1, 1)",
        default = 0.35,
        format = "format_envelope_curve"
    )]
    pub osc3_pan_shape_left_curve: FloatParam,

    #[param(
        id = 106,
        name = "Oscillator 3 Shape Right Curve",
        short_name = "Osc 3 Right Curve",
        range = "linear(-1, 1)",
        default = 0.35,
        format = "format_envelope_curve"
    )]
    pub osc3_pan_shape_right_curve: FloatParam,

    #[param(
        id = 107,
        name = "Oscillator 3 Shape Left Curve Time",
        short_name = "Osc 3 Left Curve X",
        range = "linear(0.05, 0.95)",
        default = 0.5,
        unit = "%",
        flags = "hidden"
    )]
    pub osc3_pan_shape_left_curve_time: FloatParam,

    #[param(
        id = 108,
        name = "Oscillator 3 Shape Right Curve Time",
        short_name = "Osc 3 Right Curve X",
        range = "linear(0.05, 0.95)",
        default = 0.5,
        unit = "%",
        flags = "hidden"
    )]
    pub osc3_pan_shape_right_curve_time: FloatParam,

    #[param(
        id = 109,
        name = "Oscillator 3 Shape Center X",
        short_name = "Osc 3 Center X",
        range = "linear(0.05, 0.95)",
        default = 0.5,
        unit = "%",
        flags = "hidden"
    )]
    pub osc3_pan_shape_center_x: FloatParam,

    #[param(
        id = 110,
        name = "Oscillator 2 Jitter Mode",
        short_name = "Osc 2 Jitter Mode",
        range = "discrete(0, 1)",
        default = 0,
        format = "format_swarm_mode"
    )]
    pub osc2_jitter_mode: IntParam,

    #[param(
        id = 111,
        name = "Oscillator 3 Jitter Mode",
        short_name = "Osc 3 Jitter Mode",
        range = "discrete(0, 1)",
        default = 0,
        format = "format_swarm_mode"
    )]
    pub osc3_jitter_mode: IntParam,

    #[param(
        id = 112,
        name = "Oscillator 1 Phase Warp",
        short_name = "Osc 1 Warp",
        range = "discrete(0, 3)",
        default = 0,
        format = "format_phase_warp_mode"
    )]
    pub osc1_warp_mode: IntParam,

    #[param(
        id = 113,
        name = "Oscillator 2 Phase Warp",
        short_name = "Osc 2 Warp",
        range = "discrete(0, 3)",
        default = 0,
        format = "format_phase_warp_mode"
    )]
    pub osc2_warp_mode: IntParam,

    #[param(
        id = 114,
        name = "Oscillator 3 Phase Warp",
        short_name = "Osc 3 Warp",
        range = "discrete(0, 3)",
        default = 0,
        format = "format_phase_warp_mode"
    )]
    pub osc3_warp_mode: IntParam,

    #[param(
        id = 115,
        name = "Oscillator 1 Warp Amount",
        short_name = "Osc 1 Warp Amount",
        range = "linear(0, 1)",
        default = 0.0,
        unit = "%",
        smooth = "linear(8)"
    )]
    pub osc1_warp_amount: FloatParam,

    #[param(
        id = 116,
        name = "Oscillator 2 Warp Amount",
        short_name = "Osc 2 Warp Amount",
        range = "linear(0, 1)",
        default = 0.0,
        unit = "%",
        smooth = "linear(8)"
    )]
    pub osc2_warp_amount: FloatParam,

    #[param(
        id = 117,
        name = "Oscillator 3 Warp Amount",
        short_name = "Osc 3 Warp Amount",
        range = "linear(0, 1)",
        default = 0.0,
        unit = "%",
        smooth = "linear(8)"
    )]
    pub osc3_warp_amount: FloatParam,

    #[param(
        id = 119,
        name = "Oscillator 1 Custom Shape",
        short_name = "Osc 1 Curve",
        range = "linear(0, 1)",
        default = 0.0,
        unit = "%",
        smooth = "linear(8)"
    )]
    pub osc1_custom_shape: FloatParam,

    #[param(
        id = 120,
        name = "Oscillator 2 Custom Shape",
        short_name = "Osc 2 Curve",
        range = "linear(0, 1)",
        default = 0.0,
        unit = "%",
        smooth = "linear(8)"
    )]
    pub osc2_custom_shape: FloatParam,

    #[param(
        id = 121,
        name = "Oscillator 3 Custom Shape",
        short_name = "Osc 3 Curve",
        range = "linear(0, 1)",
        default = 0.0,
        unit = "%",
        smooth = "linear(8)"
    )]
    pub osc3_custom_shape: FloatParam,

    #[param(
        id = 122,
        name = "LFO 1 Rate",
        range = "log(0.01, 20000)",
        default = 1.0
    )]
    pub lfo1_rate: FloatParam,
    #[param(
        id = 123,
        name = "LFO 1 Mode",
        range = "discrete(0, 3)",
        default = 0,
        format = "format_lfo_mode"
    )]
    pub lfo1_mode: IntParam,
    #[param(
        id = 124,
        name = "LFO 1 Phase",
        range = "linear(0, 1)",
        default = 0.0,
        unit = "%"
    )]
    pub lfo1_phase: FloatParam,
    #[param(
        id = 125,
        name = "LFO 1 Sync Division",
        short_name = "LFO 1 Sync",
        range = "discrete(0, 15)",
        default = 8,
        format = "format_lfo_sync"
    )]
    pub lfo1_sync: IntParam,
    #[param(id = 126, name = "LFO 1 Bipolar", default = true)]
    pub lfo1_bipolar: BoolParam,

    #[param(
        id = 127,
        name = "LFO 2 Rate",
        range = "log(0.01, 20000)",
        default = 2.0
    )]
    pub lfo2_rate: FloatParam,
    #[param(
        id = 128,
        name = "LFO 2 Mode",
        range = "discrete(0, 3)",
        default = 0,
        format = "format_lfo_mode"
    )]
    pub lfo2_mode: IntParam,
    #[param(
        id = 129,
        name = "LFO 2 Phase",
        range = "linear(0, 1)",
        default = 0.0,
        unit = "%"
    )]
    pub lfo2_phase: FloatParam,
    #[param(
        id = 130,
        name = "LFO 2 Sync Division",
        short_name = "LFO 2 Sync",
        range = "discrete(0, 15)",
        default = 10,
        format = "format_lfo_sync"
    )]
    pub lfo2_sync: IntParam,
    #[param(id = 131, name = "LFO 2 Bipolar", default = true)]
    pub lfo2_bipolar: BoolParam,

    #[param(
        id = 132,
        name = "LFO 3 Rate",
        range = "log(0.01, 20000)",
        default = 0.25
    )]
    pub lfo3_rate: FloatParam,
    #[param(
        id = 133,
        name = "LFO 3 Mode",
        range = "discrete(0, 3)",
        default = 0,
        format = "format_lfo_mode"
    )]
    pub lfo3_mode: IntParam,
    #[param(
        id = 134,
        name = "LFO 3 Phase",
        range = "linear(0, 1)",
        default = 0.0,
        unit = "%"
    )]
    pub lfo3_phase: FloatParam,
    #[param(
        id = 135,
        name = "LFO 3 Sync Division",
        short_name = "LFO 3 Sync",
        range = "discrete(0, 15)",
        default = 12,
        format = "format_lfo_sync"
    )]
    pub lfo3_sync: IntParam,
    #[param(id = 136, name = "LFO 3 Bipolar", default = true)]
    pub lfo3_bipolar: BoolParam,

    #[param(
        id = 137,
        name = "LFO 4 Rate",
        range = "log(0.01, 20000)",
        default = 8.0
    )]
    pub lfo4_rate: FloatParam,
    #[param(
        id = 138,
        name = "LFO 4 Mode",
        range = "discrete(0, 3)",
        default = 0,
        format = "format_lfo_mode"
    )]
    pub lfo4_mode: IntParam,
    #[param(
        id = 139,
        name = "LFO 4 Phase",
        range = "linear(0, 1)",
        default = 0.0,
        unit = "%"
    )]
    pub lfo4_phase: FloatParam,
    #[param(
        id = 140,
        name = "LFO 4 Sync Division",
        short_name = "LFO 4 Sync",
        range = "discrete(0, 15)",
        default = 6,
        format = "format_lfo_sync"
    )]
    pub lfo4_sync: IntParam,
    #[param(id = 141, name = "LFO 4 Bipolar", default = true)]
    pub lfo4_bipolar: BoolParam,

    #[param(
        id = 142,
        name = "Mod 1 Source",
        range = "discrete(0, 8)",
        default = 0,
        format = "format_mod_source"
    )]
    pub mod1_source: IntParam,
    #[param(
        id = 143,
        name = "Mod 1 Target",
        range = "discrete(0, 21)",
        default = 0,
        format = "format_mod_target"
    )]
    pub mod1_target: IntParam,
    #[param(
        id = 144,
        name = "Mod 1 Amount",
        range = "linear(-1, 1)",
        default = 0.0,
        unit = "%",
        smooth = "linear(5)"
    )]
    pub mod1_amount: FloatParam,
    #[param(
        id = 145,
        name = "Mod 2 Source",
        range = "discrete(0, 8)",
        default = 0,
        format = "format_mod_source"
    )]
    pub mod2_source: IntParam,
    #[param(
        id = 146,
        name = "Mod 2 Target",
        range = "discrete(0, 21)",
        default = 0,
        format = "format_mod_target"
    )]
    pub mod2_target: IntParam,
    #[param(
        id = 147,
        name = "Mod 2 Amount",
        range = "linear(-1, 1)",
        default = 0.0,
        unit = "%",
        smooth = "linear(5)"
    )]
    pub mod2_amount: FloatParam,
    #[param(
        id = 148,
        name = "Mod 3 Source",
        range = "discrete(0, 8)",
        default = 0,
        format = "format_mod_source"
    )]
    pub mod3_source: IntParam,
    #[param(
        id = 149,
        name = "Mod 3 Target",
        range = "discrete(0, 21)",
        default = 0,
        format = "format_mod_target"
    )]
    pub mod3_target: IntParam,
    #[param(
        id = 150,
        name = "Mod 3 Amount",
        range = "linear(-1, 1)",
        default = 0.0,
        unit = "%",
        smooth = "linear(5)"
    )]
    pub mod3_amount: FloatParam,
    #[param(
        id = 151,
        name = "Mod 4 Source",
        range = "discrete(0, 8)",
        default = 0,
        format = "format_mod_source"
    )]
    pub mod4_source: IntParam,
    #[param(
        id = 152,
        name = "Mod 4 Target",
        range = "discrete(0, 21)",
        default = 0,
        format = "format_mod_target"
    )]
    pub mod4_target: IntParam,
    #[param(
        id = 153,
        name = "Mod 4 Amount",
        range = "linear(-1, 1)",
        default = 0.0,
        unit = "%",
        smooth = "linear(5)"
    )]
    pub mod4_amount: FloatParam,
    #[param(
        id = 154,
        name = "Mod 5 Source",
        range = "discrete(0, 8)",
        default = 0,
        format = "format_mod_source"
    )]
    pub mod5_source: IntParam,
    #[param(
        id = 155,
        name = "Mod 5 Target",
        range = "discrete(0, 21)",
        default = 0,
        format = "format_mod_target"
    )]
    pub mod5_target: IntParam,
    #[param(
        id = 156,
        name = "Mod 5 Amount",
        range = "linear(-1, 1)",
        default = 0.0,
        unit = "%",
        smooth = "linear(5)"
    )]
    pub mod5_amount: FloatParam,
    #[param(
        id = 157,
        name = "Mod 6 Source",
        range = "discrete(0, 8)",
        default = 0,
        format = "format_mod_source"
    )]
    pub mod6_source: IntParam,
    #[param(
        id = 158,
        name = "Mod 6 Target",
        range = "discrete(0, 21)",
        default = 0,
        format = "format_mod_target"
    )]
    pub mod6_target: IntParam,
    #[param(
        id = 159,
        name = "Mod 6 Amount",
        range = "linear(-1, 1)",
        default = 0.0,
        unit = "%",
        smooth = "linear(5)"
    )]
    pub mod6_amount: FloatParam,
    #[param(
        id = 160,
        name = "Mod 7 Source",
        range = "discrete(0, 8)",
        default = 0,
        format = "format_mod_source"
    )]
    pub mod7_source: IntParam,
    #[param(
        id = 161,
        name = "Mod 7 Target",
        range = "discrete(0, 21)",
        default = 0,
        format = "format_mod_target"
    )]
    pub mod7_target: IntParam,
    #[param(
        id = 162,
        name = "Mod 7 Amount",
        range = "linear(-1, 1)",
        default = 0.0,
        unit = "%",
        smooth = "linear(5)"
    )]
    pub mod7_amount: FloatParam,
    #[param(
        id = 163,
        name = "Mod 8 Source",
        range = "discrete(0, 8)",
        default = 0,
        format = "format_mod_source"
    )]
    pub mod8_source: IntParam,
    #[param(
        id = 164,
        name = "Mod 8 Target",
        range = "discrete(0, 21)",
        default = 0,
        format = "format_mod_target"
    )]
    pub mod8_target: IntParam,
    #[param(
        id = 165,
        name = "Mod 8 Amount",
        range = "linear(-1, 1)",
        default = 0.0,
        unit = "%",
        smooth = "linear(5)"
    )]
    pub mod8_amount: FloatParam,

    #[param(
        id = 166,
        name = "LFO 5 Rate",
        range = "log(0.01, 20000)",
        default = 1.0
    )]
    pub lfo5_rate: FloatParam,
    #[param(
        id = 167,
        name = "LFO 5 Mode",
        range = "discrete(0, 3)",
        default = 0,
        format = "format_lfo_mode"
    )]
    pub lfo5_mode: IntParam,
    #[param(
        id = 168,
        name = "LFO 5 Phase",
        range = "linear(0, 1)",
        default = 0.0,
        unit = "%"
    )]
    pub lfo5_phase: FloatParam,
    #[param(
        id = 169,
        name = "LFO 5 Sync Division",
        range = "discrete(0, 15)",
        default = 8,
        format = "format_lfo_sync"
    )]
    pub lfo5_sync: IntParam,
    #[param(id = 170, name = "LFO 5 Bipolar", default = true)]
    pub lfo5_bipolar: BoolParam,

    #[param(
        id = 171,
        name = "LFO 6 Rate",
        range = "log(0.01, 20000)",
        default = 1.0
    )]
    pub lfo6_rate: FloatParam,
    #[param(
        id = 172,
        name = "LFO 6 Mode",
        range = "discrete(0, 3)",
        default = 0,
        format = "format_lfo_mode"
    )]
    pub lfo6_mode: IntParam,
    #[param(
        id = 173,
        name = "LFO 6 Phase",
        range = "linear(0, 1)",
        default = 0.0,
        unit = "%"
    )]
    pub lfo6_phase: FloatParam,
    #[param(
        id = 174,
        name = "LFO 6 Sync Division",
        range = "discrete(0, 15)",
        default = 8,
        format = "format_lfo_sync"
    )]
    pub lfo6_sync: IntParam,
    #[param(id = 175, name = "LFO 6 Bipolar", default = true)]
    pub lfo6_bipolar: BoolParam,

    #[param(
        id = 176,
        name = "LFO 7 Rate",
        range = "log(0.01, 20000)",
        default = 1.0
    )]
    pub lfo7_rate: FloatParam,
    #[param(
        id = 177,
        name = "LFO 7 Mode",
        range = "discrete(0, 3)",
        default = 0,
        format = "format_lfo_mode"
    )]
    pub lfo7_mode: IntParam,
    #[param(
        id = 178,
        name = "LFO 7 Phase",
        range = "linear(0, 1)",
        default = 0.0,
        unit = "%"
    )]
    pub lfo7_phase: FloatParam,
    #[param(
        id = 179,
        name = "LFO 7 Sync Division",
        range = "discrete(0, 15)",
        default = 8,
        format = "format_lfo_sync"
    )]
    pub lfo7_sync: IntParam,
    #[param(id = 180, name = "LFO 7 Bipolar", default = true)]
    pub lfo7_bipolar: BoolParam,

    #[param(
        id = 181,
        name = "LFO 8 Rate",
        range = "log(0.01, 20000)",
        default = 1.0
    )]
    pub lfo8_rate: FloatParam,
    #[param(
        id = 182,
        name = "LFO 8 Mode",
        range = "discrete(0, 3)",
        default = 0,
        format = "format_lfo_mode"
    )]
    pub lfo8_mode: IntParam,
    #[param(
        id = 183,
        name = "LFO 8 Phase",
        range = "linear(0, 1)",
        default = 0.0,
        unit = "%"
    )]
    pub lfo8_phase: FloatParam,
    #[param(
        id = 184,
        name = "LFO 8 Sync Division",
        range = "discrete(0, 15)",
        default = 8,
        format = "format_lfo_sync"
    )]
    pub lfo8_sync: IntParam,
    #[param(id = 185, name = "LFO 8 Bipolar", default = true)]
    pub lfo8_bipolar: BoolParam,

    #[param(
        id = 186,
        name = "LFO 1 Rate Unit",
        range = "discrete(0, 3)",
        default = 0,
        format = "format_lfo_rate_mode"
    )]
    pub lfo1_rate_mode: IntParam,
    #[param(
        id = 187,
        name = "LFO 2 Rate Unit",
        range = "discrete(0, 3)",
        default = 0,
        format = "format_lfo_rate_mode"
    )]
    pub lfo2_rate_mode: IntParam,
    #[param(
        id = 188,
        name = "LFO 3 Rate Unit",
        range = "discrete(0, 3)",
        default = 0,
        format = "format_lfo_rate_mode"
    )]
    pub lfo3_rate_mode: IntParam,
    #[param(
        id = 189,
        name = "LFO 4 Rate Unit",
        range = "discrete(0, 3)",
        default = 0,
        format = "format_lfo_rate_mode"
    )]
    pub lfo4_rate_mode: IntParam,
    #[param(
        id = 190,
        name = "LFO 5 Rate Unit",
        range = "discrete(0, 3)",
        default = 0,
        format = "format_lfo_rate_mode"
    )]
    pub lfo5_rate_mode: IntParam,
    #[param(
        id = 191,
        name = "LFO 6 Rate Unit",
        range = "discrete(0, 3)",
        default = 0,
        format = "format_lfo_rate_mode"
    )]
    pub lfo6_rate_mode: IntParam,
    #[param(
        id = 192,
        name = "LFO 7 Rate Unit",
        range = "discrete(0, 3)",
        default = 0,
        format = "format_lfo_rate_mode"
    )]
    pub lfo7_rate_mode: IntParam,
    #[param(
        id = 193,
        name = "LFO 8 Rate Unit",
        range = "discrete(0, 3)",
        default = 0,
        format = "format_lfo_rate_mode"
    )]
    pub lfo8_rate_mode: IntParam,

    #[param(
        id = 194,
        name = "Mod 9 Source",
        range = "discrete(0, 8)",
        default = 0,
        format = "format_mod_source"
    )]
    pub mod9_source: IntParam,
    #[param(
        id = 195,
        name = "Mod 9 Target",
        range = "discrete(0, 21)",
        default = 0,
        format = "format_mod_target"
    )]
    pub mod9_target: IntParam,
    #[param(
        id = 196,
        name = "Mod 9 Amount",
        range = "linear(-1, 1)",
        default = 0.0,
        unit = "%",
        smooth = "linear(5)"
    )]
    pub mod9_amount: FloatParam,
    #[param(
        id = 197,
        name = "Mod 10 Source",
        range = "discrete(0, 8)",
        default = 0,
        format = "format_mod_source"
    )]
    pub mod10_source: IntParam,
    #[param(
        id = 198,
        name = "Mod 10 Target",
        range = "discrete(0, 21)",
        default = 0,
        format = "format_mod_target"
    )]
    pub mod10_target: IntParam,
    #[param(
        id = 199,
        name = "Mod 10 Amount",
        range = "linear(-1, 1)",
        default = 0.0,
        unit = "%",
        smooth = "linear(5)"
    )]
    pub mod10_amount: FloatParam,
    #[param(
        id = 200,
        name = "Mod 11 Source",
        range = "discrete(0, 8)",
        default = 0,
        format = "format_mod_source"
    )]
    pub mod11_source: IntParam,
    #[param(
        id = 201,
        name = "Mod 11 Target",
        range = "discrete(0, 21)",
        default = 0,
        format = "format_mod_target"
    )]
    pub mod11_target: IntParam,
    #[param(
        id = 202,
        name = "Mod 11 Amount",
        range = "linear(-1, 1)",
        default = 0.0,
        unit = "%",
        smooth = "linear(5)"
    )]
    pub mod11_amount: FloatParam,
    #[param(
        id = 203,
        name = "Mod 12 Source",
        range = "discrete(0, 8)",
        default = 0,
        format = "format_mod_source"
    )]
    pub mod12_source: IntParam,
    #[param(
        id = 204,
        name = "Mod 12 Target",
        range = "discrete(0, 21)",
        default = 0,
        format = "format_mod_target"
    )]
    pub mod12_target: IntParam,
    #[param(
        id = 205,
        name = "Mod 12 Amount",
        range = "linear(-1, 1)",
        default = 0.0,
        unit = "%",
        smooth = "linear(5)"
    )]
    pub mod12_amount: FloatParam,
    #[param(
        id = 206,
        name = "Mod 13 Source",
        range = "discrete(0, 8)",
        default = 0,
        format = "format_mod_source"
    )]
    pub mod13_source: IntParam,
    #[param(
        id = 207,
        name = "Mod 13 Target",
        range = "discrete(0, 21)",
        default = 0,
        format = "format_mod_target"
    )]
    pub mod13_target: IntParam,
    #[param(
        id = 208,
        name = "Mod 13 Amount",
        range = "linear(-1, 1)",
        default = 0.0,
        unit = "%",
        smooth = "linear(5)"
    )]
    pub mod13_amount: FloatParam,
    #[param(
        id = 209,
        name = "Mod 14 Source",
        range = "discrete(0, 8)",
        default = 0,
        format = "format_mod_source"
    )]
    pub mod14_source: IntParam,
    #[param(
        id = 210,
        name = "Mod 14 Target",
        range = "discrete(0, 21)",
        default = 0,
        format = "format_mod_target"
    )]
    pub mod14_target: IntParam,
    #[param(
        id = 211,
        name = "Mod 14 Amount",
        range = "linear(-1, 1)",
        default = 0.0,
        unit = "%",
        smooth = "linear(5)"
    )]
    pub mod14_amount: FloatParam,
    #[param(
        id = 212,
        name = "Mod 15 Source",
        range = "discrete(0, 8)",
        default = 0,
        format = "format_mod_source"
    )]
    pub mod15_source: IntParam,
    #[param(
        id = 213,
        name = "Mod 15 Target",
        range = "discrete(0, 21)",
        default = 0,
        format = "format_mod_target"
    )]
    pub mod15_target: IntParam,
    #[param(
        id = 214,
        name = "Mod 15 Amount",
        range = "linear(-1, 1)",
        default = 0.0,
        unit = "%",
        smooth = "linear(5)"
    )]
    pub mod15_amount: FloatParam,
    #[param(
        id = 215,
        name = "Mod 16 Source",
        range = "discrete(0, 8)",
        default = 0,
        format = "format_mod_source"
    )]
    pub mod16_source: IntParam,
    #[param(
        id = 216,
        name = "Mod 16 Target",
        range = "discrete(0, 21)",
        default = 0,
        format = "format_mod_target"
    )]
    pub mod16_target: IntParam,
    #[param(
        id = 217,
        name = "Mod 16 Amount",
        range = "linear(-1, 1)",
        default = 0.0,
        unit = "%",
        smooth = "linear(5)"
    )]
    pub mod16_amount: FloatParam,

    #[param(
        id = 218,
        name = "LFO 1 Active",
        default = true,
        flags = "hidden | automatable"
    )]
    pub lfo1_active: BoolParam,
    #[param(
        id = 219,
        name = "LFO 2 Active",
        default = false,
        flags = "hidden | automatable"
    )]
    pub lfo2_active: BoolParam,
    #[param(
        id = 220,
        name = "LFO 3 Active",
        default = false,
        flags = "hidden | automatable"
    )]
    pub lfo3_active: BoolParam,
    #[param(
        id = 221,
        name = "LFO 4 Active",
        default = false,
        flags = "hidden | automatable"
    )]
    pub lfo4_active: BoolParam,
    #[param(
        id = 222,
        name = "LFO 5 Active",
        default = false,
        flags = "hidden | automatable"
    )]
    pub lfo5_active: BoolParam,
    #[param(
        id = 223,
        name = "LFO 6 Active",
        default = false,
        flags = "hidden | automatable"
    )]
    pub lfo6_active: BoolParam,
    #[param(
        id = 224,
        name = "LFO 7 Active",
        default = false,
        flags = "hidden | automatable"
    )]
    pub lfo7_active: BoolParam,
    #[param(
        id = 225,
        name = "LFO 8 Active",
        default = false,
        flags = "hidden | automatable"
    )]
    pub lfo8_active: BoolParam,

    #[param(
        id = 226,
        name = "Pitch Wheel Range",
        short_name = "PB Range",
        range = "discrete(1, 96)",
        default = 2,
        unit = "st"
    )]
    pub pitch_bend_range: IntParam,

    #[param(
        id = 227,
        name = "Mod Wheel",
        short_name = "Mod",
        range = "linear(0, 1)",
        default = 0.0,
        unit = "%",
        midi_cc = 1,
        flags = "hidden | automatable"
    )]
    pub mod_wheel: FloatParam,

    #[param(
        id = 228,
        name = "Mod 1 Extended Target",
        range = "discrete(0, 60)",
        default = 0,
        flags = "hidden | automatable"
    )]
    pub mod1_target_ext: IntParam,
    #[param(
        id = 229,
        name = "Mod 2 Extended Target",
        range = "discrete(0, 60)",
        default = 0,
        flags = "hidden | automatable"
    )]
    pub mod2_target_ext: IntParam,
    #[param(
        id = 230,
        name = "Mod 3 Extended Target",
        range = "discrete(0, 60)",
        default = 0,
        flags = "hidden | automatable"
    )]
    pub mod3_target_ext: IntParam,
    #[param(
        id = 231,
        name = "Mod 4 Extended Target",
        range = "discrete(0, 60)",
        default = 0,
        flags = "hidden | automatable"
    )]
    pub mod4_target_ext: IntParam,
    #[param(
        id = 232,
        name = "Mod 5 Extended Target",
        range = "discrete(0, 60)",
        default = 0,
        flags = "hidden | automatable"
    )]
    pub mod5_target_ext: IntParam,
    #[param(
        id = 233,
        name = "Mod 6 Extended Target",
        range = "discrete(0, 60)",
        default = 0,
        flags = "hidden | automatable"
    )]
    pub mod6_target_ext: IntParam,
    #[param(
        id = 234,
        name = "Mod 7 Extended Target",
        range = "discrete(0, 60)",
        default = 0,
        flags = "hidden | automatable"
    )]
    pub mod7_target_ext: IntParam,
    #[param(
        id = 235,
        name = "Mod 8 Extended Target",
        range = "discrete(0, 60)",
        default = 0,
        flags = "hidden | automatable"
    )]
    pub mod8_target_ext: IntParam,
    #[param(
        id = 236,
        name = "Mod 9 Extended Target",
        range = "discrete(0, 60)",
        default = 0,
        flags = "hidden | automatable"
    )]
    pub mod9_target_ext: IntParam,
    #[param(
        id = 237,
        name = "Mod 10 Extended Target",
        range = "discrete(0, 60)",
        default = 0,
        flags = "hidden | automatable"
    )]
    pub mod10_target_ext: IntParam,
    #[param(
        id = 238,
        name = "Mod 11 Extended Target",
        range = "discrete(0, 60)",
        default = 0,
        flags = "hidden | automatable"
    )]
    pub mod11_target_ext: IntParam,
    #[param(
        id = 239,
        name = "Mod 12 Extended Target",
        range = "discrete(0, 60)",
        default = 0,
        flags = "hidden | automatable"
    )]
    pub mod12_target_ext: IntParam,
    #[param(
        id = 240,
        name = "Mod 13 Extended Target",
        range = "discrete(0, 60)",
        default = 0,
        flags = "hidden | automatable"
    )]
    pub mod13_target_ext: IntParam,
    #[param(
        id = 241,
        name = "Mod 14 Extended Target",
        range = "discrete(0, 60)",
        default = 0,
        flags = "hidden | automatable"
    )]
    pub mod14_target_ext: IntParam,
    #[param(
        id = 242,
        name = "Mod 15 Extended Target",
        range = "discrete(0, 60)",
        default = 0,
        flags = "hidden | automatable"
    )]
    pub mod15_target_ext: IntParam,
    #[param(
        id = 243,
        name = "Mod 16 Extended Target",
        range = "discrete(0, 60)",
        default = 0,
        flags = "hidden | automatable"
    )]
    pub mod16_target_ext: IntParam,

    /// The editable left/right Shape spline is persisted as custom state,
    /// because arbitrary knots cannot be represented by a fixed automation
    /// parameter list.  Its compiled runtime snapshot is lock-free on audio.
    #[persist = "pan-shape-curve"]
    pub pan_shape_curve_state: PanShapeCurveState,

    #[persist = "osc2-pan-shape-curve"]
    pub osc2_pan_shape_curve_state: PanShapeCurveState,

    #[persist = "osc3-pan-shape-curve"]
    pub osc3_pan_shape_curve_state: PanShapeCurveState,

    #[persist = "osc1-wave-curve"]
    pub osc1_wave_curve_state: WaveCurveState,

    #[persist = "osc2-wave-curve"]
    pub osc2_wave_curve_state: WaveCurveState,

    #[persist = "osc3-wave-curve"]
    pub osc3_wave_curve_state: WaveCurveState,

    #[persist = "lfo1-curve"]
    pub lfo1_curve_state: WaveCurveState,

    #[persist = "lfo2-curve"]
    pub lfo2_curve_state: WaveCurveState,

    #[persist = "lfo3-curve"]
    pub lfo3_curve_state: WaveCurveState,

    #[persist = "lfo4-curve"]
    pub lfo4_curve_state: WaveCurveState,

    #[persist = "lfo5-curve"]
    pub lfo5_curve_state: WaveCurveState,

    #[persist = "lfo6-curve"]
    pub lfo6_curve_state: WaveCurveState,

    #[persist = "lfo7-curve"]
    pub lfo7_curve_state: WaveCurveState,

    #[persist = "lfo8-curve"]
    pub lfo8_curve_state: WaveCurveState,

    #[persist = "editor-state"]
    pub editor_state: Mutex<KurvEditorState>,

    #[skip]
    editor_host_scale_bits: AtomicU64,

    #[meter]
    pub meter_left: MeterSlot,

    #[meter]
    pub meter_right: MeterSlot,

    #[meter]
    pub stereo_seed: MeterSlot,

    #[meter]
    pub swarm_phase: MeterSlot,

    #[meter]
    pub osc2_stereo_seed: MeterSlot,

    #[meter]
    pub osc2_swarm_phase: MeterSlot,

    #[meter]
    pub osc3_stereo_seed: MeterSlot,

    #[meter]
    pub osc3_swarm_phase: MeterSlot,

    #[meter]
    pub lfo1_phase_meter: MeterSlot,
    #[meter]
    pub lfo2_phase_meter: MeterSlot,
    #[meter]
    pub lfo3_phase_meter: MeterSlot,
    #[meter]
    pub lfo4_phase_meter: MeterSlot,
    #[meter]
    pub lfo5_phase_meter: MeterSlot,
    #[meter]
    pub lfo6_phase_meter: MeterSlot,
    #[meter]
    pub lfo7_phase_meter: MeterSlot,
    #[meter]
    pub lfo8_phase_meter: MeterSlot,

    #[meter]
    pub lfo1_value_meter: MeterSlot,
    #[meter]
    pub lfo2_value_meter: MeterSlot,
    #[meter]
    pub lfo3_value_meter: MeterSlot,
    #[meter]
    pub lfo4_value_meter: MeterSlot,
    #[meter]
    pub lfo5_value_meter: MeterSlot,
    #[meter]
    pub lfo6_value_meter: MeterSlot,
    #[meter]
    pub lfo7_value_meter: MeterSlot,
    #[meter]
    pub lfo8_value_meter: MeterSlot,
}

impl KurvParams {
    #[allow(
        clippy::unused_self,
        reason = "Truce custom parameter formatters are instance methods"
    )]
    fn format_shape(&self, value: f64) -> String {
        const NAMES: [&str; 4] = ["SINE", "TRIANGLE", "SAW", "PULSE"];
        let value = value.clamp(0.0, 3.0);
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "shape position is clamped to the four canonical waveforms"
        )]
        let lower = value.floor() as usize;
        let blend = value.fract();
        if blend <= 0.001 || lower == 3 {
            NAMES[lower].to_owned()
        } else {
            format!(
                "{} → {} {:.0}%",
                NAMES[lower],
                NAMES[lower + 1],
                blend * 100.0
            )
        }
    }

    #[allow(
        clippy::unused_self,
        reason = "Truce custom parameter formatters are instance methods"
    )]
    fn format_unison_curve(&self, value: f64) -> String {
        let value = value.clamp(-1.0, 1.0);
        if value.abs() < 0.01 {
            "EVEN".to_owned()
        } else if value < 0.0 {
            format!("EDGES {:.0}%", -value * 100.0)
        } else {
            format!("CENTER {:.0}%", value * 100.0)
        }
    }

    #[allow(
        clippy::unused_self,
        reason = "Truce custom parameter formatters are instance methods"
    )]
    fn format_semitones(&self, value: f64) -> String {
        format!("{value:.2} st")
    }

    #[allow(
        clippy::unused_self,
        reason = "Truce custom parameter formatters are instance methods"
    )]
    fn format_swarm_rate(&self, value: f64) -> String {
        format!("{value:.2} Hz")
    }

    #[allow(
        clippy::unused_self,
        reason = "Truce custom parameter formatters are instance methods"
    )]
    fn format_swarm_mode(&self, value: f64) -> String {
        if value.round() >= 1.0 {
            "SINE".to_owned()
        } else {
            "NOISE".to_owned()
        }
    }

    #[allow(
        clippy::unused_self,
        reason = "Truce custom parameter formatters are instance methods"
    )]
    fn format_unison_alignment_mode(&self, value: f64) -> String {
        ["NOTE", "HARM", "ODD", "EVEN"][value.round().clamp(0.0, 3.0) as usize].to_owned()
    }

    #[allow(
        clippy::unused_self,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "phase-warp mode is clamped to four discrete labels"
    )]
    fn format_phase_warp_mode(&self, value: f64) -> String {
        const NAMES: [&str; 4] = ["NONE", "PWM", "BEND", "HARM"];
        NAMES[value.round().clamp(0.0, 3.0) as usize].to_owned()
    }

    #[allow(
        clippy::unused_self,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "LFO mode is clamped to four discrete labels"
    )]
    fn format_lfo_mode(&self, value: f64) -> String {
        const NAMES: [&str; 4] = ["FREE", "RETRIG", "SYNC", "ONE SHOT"];
        NAMES[value.round().clamp(0.0, 3.0) as usize].to_owned()
    }

    #[allow(
        clippy::unused_self,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "LFO rate mode is clamped to four discrete labels"
    )]
    fn format_lfo_rate_mode(&self, value: f64) -> String {
        const NAMES: [&str; 4] = ["Hz", "ms", "BEAT", "KEY"];
        NAMES[value.round().clamp(0.0, 3.0) as usize].to_owned()
    }

    #[allow(
        clippy::unused_self,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "sync division is clamped to the fixed musical division table"
    )]
    fn format_lfo_sync(&self, value: f64) -> String {
        const NAMES: [&str; 16] = [
            "1/64", "1/32T", "1/32", "1/16T", "1/16", "1/8T", "1/8", "1/4T", "1/4", "1/2T", "1/2",
            "1/1T", "1/1", "2/1", "4/1", "8/1",
        ];
        NAMES[value.round().clamp(0.0, 15.0) as usize].to_owned()
    }

    #[allow(
        clippy::unused_self,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "modulation source is clamped to off plus eight LFOs"
    )]
    fn format_mod_source(&self, value: f64) -> String {
        const NAMES: [&str; 9] = [
            "OFF", "LFO 1", "LFO 2", "LFO 3", "LFO 4", "LFO 5", "LFO 6", "LFO 7", "LFO 8",
        ];
        NAMES[value.round().clamp(0.0, 8.0) as usize].to_owned()
    }

    #[allow(
        clippy::unused_self,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "modulation target is clamped to the fixed oscillator target bank"
    )]
    fn format_mod_target(&self, value: f64) -> String {
        const NAMES: [&str; 22] = [
            "OFF",
            "O1 PITCH",
            "O1 SHAPE",
            "O1 PWM",
            "O1 WARP",
            "O1 LEVEL",
            "O1 PAN",
            "O2 PITCH",
            "O2 SHAPE",
            "O2 PWM",
            "O2 WARP",
            "O2 LEVEL",
            "O2 PAN",
            "O3 PITCH",
            "O3 SHAPE",
            "O3 PWM",
            "O3 WARP",
            "O3 LEVEL",
            "O3 PAN",
            "O1 DETUNE",
            "O2 DETUNE",
            "O3 DETUNE",
        ];
        NAMES[value.round().clamp(0.0, 21.0) as usize].to_owned()
    }

    #[allow(
        clippy::unused_self,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the discrete stereo layout is clamped to four labels"
    )]
    fn format_stereo_pattern(&self, value: f64) -> String {
        const NAMES: [&str; 4] = ["SHAPE", "ALTERNATE", "SHAPE", "RANDOM"];
        NAMES[value.round().clamp(0.0, 3.0) as usize].to_owned()
    }

    #[allow(
        clippy::unused_self,
        reason = "Truce custom parameter formatters are instance methods"
    )]
    fn format_envelope_curve(&self, value: f64) -> String {
        let value = value.clamp(-1.0, 1.0);
        if value.abs() < 0.01 {
            "LINEAR".to_owned()
        } else if value < 0.0 {
            format!("SLOW {:.0}%", -value * 100.0)
        } else {
            format!("FAST {:.0}%", value * 100.0)
        }
    }

    #[allow(
        clippy::unused_self,
        reason = "Truce custom parameter formatters are instance methods"
    )]
    fn format_envelope_curve_time(&self, value: f64) -> String {
        let value = value.clamp(-1.0, 1.0);
        if value.abs() < 0.01 {
            "CENTER".to_owned()
        } else if value < 0.0 {
            format!("EARLY {:.0}%", -value * 100.0)
        } else {
            format!("LATE {:.0}%", value * 100.0)
        }
    }

    #[allow(
        clippy::unused_self,
        reason = "Truce custom parameter formatters are instance methods"
    )]
    fn format_unison_weight(&self, value: f64) -> String {
        let value = value.clamp(-1.0, 1.0);
        if value.abs() < 0.01 {
            "EVEN".to_owned()
        } else if value < 0.0 {
            format!("CENTER {:.0}%", -value * 100.0)
        } else {
            format!("SIDES {:.0}%", value * 100.0)
        }
    }

    #[allow(
        clippy::unused_self,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the oversampling factor is clamped to the four visible quality modes"
    )]
    fn format_oversampling(&self, value: f64) -> String {
        const NAMES: [&str; 4] = ["ECO 1x", "NORMAL 2x", "HIGH 3x", "ULTRA 4x"];
        NAMES[value.round().clamp(1.0, 4.0) as usize - 1].to_owned()
    }

    #[allow(
        clippy::unused_self,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the antialiasing selector has exactly three labels"
    )]
    fn format_antialiasing(&self, value: f64) -> String {
        let _ = value;
        "SPLINE 4PT".to_owned()
    }

    #[allow(
        clippy::unused_self,
        reason = "Truce custom parameter formatters are instance methods"
    )]
    fn format_generator_engine(&self, value: f64) -> String {
        let _ = value;
        "SPLINE 4PT".to_owned()
    }

    #[allow(
        clippy::unused_self,
        reason = "Truce custom parameter formatters are instance methods"
    )]
    fn format_signed_semitones(&self, value: f64) -> String {
        format!("{:+.0} st", value.round())
    }

    #[allow(
        clippy::unused_self,
        reason = "Truce custom parameter formatters are instance methods"
    )]
    fn format_cents(&self, value: f64) -> String {
        format!("{value:+.1} ct")
    }

    #[allow(
        clippy::unused_self,
        reason = "Truce custom parameter formatters are instance methods"
    )]
    fn format_octaves(&self, value: f64) -> String {
        format!("{:+.0} oct", value.round())
    }

    #[allow(
        clippy::unused_self,
        clippy::cast_possible_truncation,
        reason = "Truce custom parameter formatters are instance methods"
    )]
    fn format_voice_mode(&self, value: f64) -> String {
        match value.round() as i32 {
            0 => "MONO".to_owned(),
            1 => "LEGATO".to_owned(),
            voices => format!("{voices} VOICES"),
        }
    }

    #[allow(
        clippy::unused_self,
        reason = "Truce custom parameter formatters are instance methods"
    )]
    fn format_glide_time(&self, value: f64) -> String {
        if value <= 0.000_5 {
            "OFF".to_owned()
        } else if value < 1.0 {
            format!("{:.0} ms", value * 1_000.0)
        } else {
            format!("{value:.2} s")
        }
    }
}

use KurvParamsParamId as P;

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
                config: route,
                amount_index: index,
                descriptor: descriptor.copied(),
            };
            active.len += 1;
            active.source_mask |= 1_u8 << (route.source - 1);
            if let Some(descriptor) = descriptor
                && let modulation_target::TargetKind::Unison {
                    oscillator,
                    control,
                } = descriptor.kind
                && !matches!(
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
                )
            {
                active.unison_layout_mask |= 1 << oscillator;
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

pub struct Kurv;

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
    unison_modulation_countdown: u32,
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
            unison_modulation_countdown: 0,
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
        let left = left * gain;
        let right = right * gain;
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

fn apply_modulation(
    state: &mut KurvDspState,
    settings: &mut VoiceSettings,
    routes: &ActiveRoutes,
    base_unison: &[UnisonSettings; 3],
    base_glide: f32,
    lfo_controls_static: bool,
    frame: usize,
) -> lfo::ModulationFrame {
    let mut modulation = lfo::ModulationFrame::default();
    if state.lfos.is_active() {
        let sources = if lfo_controls_static {
            state.lfos.next()
        } else {
            let rate_hz = std::array::from_fn(|index| state.controls.lfo_rate[index][frame]);
            let phase_offsets = std::array::from_fn(|index| state.controls.lfo_phase[index][frame]);
            state.lfos.next_with_controls(rate_hz, phase_offsets)
        };
        for route in routes.as_slice() {
            let amount_index = route.amount_index;
            if let Some(descriptor) = route.descriptor {
                accumulate_modulation(
                    &mut modulation,
                    descriptor,
                    route.config.source,
                    state.controls.modulation_amounts[amount_index][frame],
                    &sources,
                );
            }
        }
    }
    for oscillator in 0..3 {
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
        modulation.unison[oscillator].stereo_y += control.stereo_y[frame] - base.stereo_alternate();
        modulation.unison[oscillator].weight += control.weight[frame] - base.level_curve();
    }
    for oscillator in 0..3 {
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
        settings
            .modulate_unison_detune_amount(oscillator, modulation.unison[oscillator].detune_amount);
    }
    settings.velocity_amount =
        (settings.velocity_amount + modulation.global.velocity).clamp(0.0, 1.0);
    settings.pressure_amount =
        (settings.pressure_amount + modulation.global.pressure).clamp(0.0, 1.0);
    settings.timbre_amount = (settings.timbre_amount + modulation.global.timbre).clamp(0.0, 1.0);
    state
        .synth
        .set_glide_time((base_glide + modulation.global.glide).clamp(0.0, 5.0));

    if routes.unison_layout_mask != 0 && state.unison_modulation_countdown == 0 {
        for oscillator in 0..3 {
            if routes.unison_layout_mask & (1 << oscillator) == 0 {
                continue;
            }
            let settings = base_unison[oscillator].modulated(modulation.unison[oscillator]);
            if oscillator == 0 {
                state.synth.configure_unison(settings);
            } else {
                state.synth.configure_secondary_unison(oscillator, settings);
            }
        }
        state.unison_modulation_countdown = (state.dsp_sample_rate / 120.0).round().max(1.0) as u32;
    } else {
        state.unison_modulation_countdown = state.unison_modulation_countdown.saturating_sub(1);
    }
    modulation
}

fn accumulate_modulation(
    modulation: &mut lfo::ModulationFrame,
    target: modulation_target::TargetDescriptor,
    source: u8,
    amount: f32,
    sources: &[f32; LFO_COUNT],
) {
    use modulation_target::{GlobalTarget, OscTarget, TargetKind, UnisonTarget};

    let source = usize::from(source.saturating_sub(1));
    if source >= LFO_COUNT {
        return;
    }
    let value = sources[source] * amount.clamp(-1.0, 1.0);
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

impl PluginLogic for Kurv {
    type Params = KurvParams;
    type DspState = KurvDspState;

    const PRESERVE_DSP_STATE: bool = false;

    fn bus_layouts() -> Vec<BusLayout> {
        BusLayout::stereo_and_mono_output()
    }

    #[allow(
        clippy::cast_possible_truncation,
        reason = "the host sample rate is finite and exactly representable at audio-rate magnitudes"
    )]
    fn reset(state: &mut KurvDspState, params: &KurvParams, config: &AudioConfig) {
        state.host_sample_rate = config.sample_rate.max(1.0) as f32;
        let (factor, requested_antialiasing) = generator_configuration(params);
        diagnostics::trace(
            "lifecycle",
            "reset-enter",
            state.host_sample_rate,
            f32::from(factor),
        );
        state.dsp_sample_rate = state.host_sample_rate * f32::from(factor);
        state.synth.set_sample_rate(state.dsp_sample_rate);
        state.synth.reset();
        state.lfos.reset(state.dsp_sample_rate);
        state.unison_modulation_countdown = 0;
        state.oversampler.reset(factor);
        let antialiasing = requested_antialiasing.for_factor(factor);
        state
            .oversampler
            .set_spline_correction_immediate(matches!(antialiasing, Antialiasing::SplineOptimized));
        state.decimator_tail = 0;
        state.mpe_bend_range = 48.0;
        state.pitch_bend_range = 2.0;
        state.glide_time_control = f32::NAN;
        state.pitch_bend_control = f32::NAN;
        state.meter_left = 0.0;
        state.meter_right = 0.0;
        #[cfg(test)]
        {
            state.block_major_chunks = 0;
            state.internal_pool_coarse_jobs = 0;
            state.internal_pool_partial_serial_jobs = 0;
        }
        diagnostics::trace(
            "lifecycle",
            "reset-return",
            state.dsp_sample_rate,
            f32::from(state.oversampler.factor()),
        );
    }

    #[allow(
        clippy::too_many_lines,
        reason = "the audio callback keeps event dispatch, synthesis, oversampling, and meters in one RT boundary"
    )]
    fn process(
        state: &mut KurvDspState,
        params: &KurvParams,
        buffer: &mut AudioBuffer,
        events: &EventList,
        context: &mut ProcessContext,
    ) -> ProcessStatus {
        let output_channels = buffer.num_output_channels();
        if output_channels == 0 {
            return ProcessStatus::Tail(0);
        }

        let (requested_factor, requested_antialiasing) = generator_configuration(params);
        state.set_oversampling(requested_factor, requested_antialiasing);
        let modulation_routes = modulation_routes(params);
        let oscillator_enabled = [
            params.osc1_enabled.value(),
            params.osc2_enabled.value(),
            params.osc3_enabled.value(),
        ];
        let active_routes = active_modulation_routes(&modulation_routes, oscillator_enabled);
        let configured_lfos = configured_lfo_mask(params);
        let lfo_curve_states = [
            &params.lfo1_curve_state,
            &params.lfo2_curve_state,
            &params.lfo3_curve_state,
            &params.lfo4_curve_state,
            &params.lfo5_curve_state,
            &params.lfo6_curve_state,
            &params.lfo7_curve_state,
            &params.lfo8_curve_state,
        ];
        let lfo_curves = std::array::from_fn(|index| {
            if (active_routes.source_mask | configured_lfos) & (1 << index) != 0 {
                lfo_curve_states[index].try_curve_rt()
            } else {
                None
            }
        });
        let lfo_configs = lfo_configuration(params);
        state.lfos.configure(
            lfo_configs,
            lfo_curves,
            0,
            context.transport,
            state.host_sample_rate,
        );
        let mut antialiasing = requested_antialiasing.for_factor(state.oversampler.factor());
        state
            .oversampler
            .set_spline_correction(matches!(antialiasing, Antialiasing::SplineOptimized));
        for (oscillator, curve) in [
            &params.pan_shape_curve_state,
            &params.osc2_pan_shape_curve_state,
            &params.osc3_pan_shape_curve_state,
        ]
        .into_iter()
        .enumerate()
        {
            if (oscillator == 0 || oscillator_enabled[oscillator])
                && let Some(segments) = curve.try_segments_rt()
            {
                state.pan_shape_segments[oscillator] = segments;
            }
        }
        for (oscillator, curve) in [
            &params.osc1_wave_curve_state,
            &params.osc2_wave_curve_state,
            &params.osc3_wave_curve_state,
        ]
        .into_iter()
        .enumerate()
        {
            if (oscillator == 0 || oscillator_enabled[oscillator])
                && let Some(compiled) = curve.try_curve_rt()
            {
                let audible = oscillator_enabled[oscillator]
                    && match oscillator {
                        0 => params.osc1_custom_shape.value() > f32::EPSILON,
                        1 => params.osc2_custom_shape.value() > f32::EPSILON,
                        _ => params.osc3_custom_shape.value() > f32::EPSILON,
                    };
                state.wave_curves[oscillator].retarget(compiled, audible);
            }
        }

        let unison_settings = unison_configurations(params, state);
        for oscillator in 0..3 {
            if !oscillator_enabled[oscillator]
                || active_routes.unison_layout_mask & (1 << oscillator) != 0
            {
                continue;
            }
            if oscillator == 0 {
                state.synth.configure_unison(unison_settings[oscillator]);
            } else {
                state
                    .synth
                    .configure_secondary_unison(oscillator, unison_settings[oscillator]);
            }
        }
        state
            .synth
            .configure_voice_mode(params.voice_mode.value_u8());
        state.synth.set_transpose(
            params
                .octave_shift
                .value_f32()
                .mul_add(12.0, params.transpose.value_f32()),
        );
        state.mpe_bend_range = f32::from(params.mpe_bend_range.value_u8());
        state.pitch_bend_range = f32::from(params.pitch_bend_range.value_u8());

        state.synth.configure_oscillator_enabled(oscillator_enabled);
        let oscillator_transpose = [
            params.osc1_transpose.value_f32(),
            params.osc2_transpose.value_f32(),
            params.osc3_transpose.value_f32(),
        ];
        let oscillator_warp_mode = [
            PhaseWarpMode::from_index(params.osc1_warp_mode.value_u8()),
            PhaseWarpMode::from_index(params.osc2_warp_mode.value_u8()),
            PhaseWarpMode::from_index(params.osc3_warp_mode.value_u8()),
        ];
        state.synth.configure_phase_warp_modes(oscillator_warp_mode);

        let mut next_event = 0;
        let mut block_start = 0;
        let mut peak_left = 0.0_f32;
        let mut peak_right = 0.0_f32;
        while block_start < buffer.num_samples() {
            let block_len = (buffer.num_samples() - block_start).min(CONTROL_BLOCK);
            let static_gain = state.controls.read(
                params,
                block_len,
                oscillator_enabled,
                &active_routes,
                active_routes.source_mask,
            );
            let modulation_mask = state.controls.active_lfo_mask(&active_routes, block_len);
            let lfo_controls_static =
                state
                    .controls
                    .lfo_controls_static(modulation_mask, block_len, &lfo_configs);
            let pitch_bend_static = slice_is_static(&state.controls.pitch_bend[..block_len]);
            state
                .lfos
                .set_active_mask(modulation_mask | configured_lfos);
            state.lfos.set_modulation_mask(modulation_mask);
            state.fill_wave_curve_fades(block_len);
            if modulation_mask == 0 && active_routes.unison_layout_mask != 0 {
                for oscillator in 0..3 {
                    if active_routes.unison_layout_mask & (1 << oscillator) == 0 {
                        continue;
                    }
                    if oscillator == 0 {
                        state.synth.configure_unison(unison_settings[oscillator]);
                    } else {
                        state
                            .synth
                            .configure_secondary_unison(oscillator, unison_settings[oscillator]);
                    }
                }
                state.unison_modulation_countdown = 0;
            }
            let direct_unison_pitch_active = state
                .controls
                .unison_pitch_active_mask(block_len, &unison_settings)
                != 0;

            let mut offset = 0;
            while offset < block_len {
                let sample_index = block_start + offset;
                let glide_time = state.controls.glide_time[offset];
                if glide_time.to_bits() != state.glide_time_control.to_bits() {
                    state.synth.set_glide_time(glide_time);
                    state.glide_time_control = glide_time;
                }
                if !pitch_bend_static || offset == 0 {
                    let pitch_bend = state.controls.pitch_bend[offset];
                    if pitch_bend.to_bits() != state.pitch_bend_control.to_bits() {
                        state
                            .synth
                            .parameter_pitch_bend(pitch_bend, state.pitch_bend_range);
                        state.pitch_bend_control = pitch_bend;
                    }
                }
                dispatch_events(state, events, &mut next_event, sample_index);
                if !state.synth.is_active() && state.decimator_tail == 0 {
                    state
                        .lfos
                        .advance_silent(usize::from(state.oversampler.factor()));
                    for channel in 0..output_channels {
                        buffer.output(channel)[sample_index] = 0.0;
                    }
                    offset += 1;
                    continue;
                }

                let mut settings = VoiceSettings::new(
                    state.controls.shape[offset],
                    110.0,
                    state.controls.pulse_width[offset],
                    state.controls.velocity[offset],
                    state.controls.pressure[offset],
                    state.controls.timbre[offset],
                )
                .with_antialiasing(antialiasing)
                .with_oscillators([
                    OscillatorSettings::new(
                        oscillator_enabled[0],
                        state.controls.shape[offset],
                        state.controls.pulse_width[offset],
                        OscillatorSettings::pitch_ratio(
                            oscillator_transpose[0],
                            state.controls.osc1_cents[offset],
                        ),
                        state.controls.osc1_level[offset],
                        state.controls.osc1_pan[offset],
                    )
                    .with_unison_detune_amount(params.unison_detune_amount.value())
                    .with_phase_warp(
                        oscillator_warp_mode[0],
                        state.controls.osc1_warp_amount[offset],
                    )
                    .with_custom_curve(
                        state.wave_curves[0].value(state.controls.osc1_curve_fade[offset]),
                        state.controls.osc1_custom_shape[offset],
                    ),
                    OscillatorSettings::new(
                        oscillator_enabled[1],
                        state.controls.osc2_shape[offset],
                        state.controls.osc2_pulse_width[offset],
                        OscillatorSettings::pitch_ratio(
                            oscillator_transpose[1],
                            state.controls.osc2_cents[offset],
                        ),
                        state.controls.osc2_level[offset],
                        state.controls.osc2_pan[offset],
                    )
                    .with_unison_detune_amount(params.osc2_unison_detune_amount.value())
                    .with_phase_warp(
                        oscillator_warp_mode[1],
                        state.controls.osc2_warp_amount[offset],
                    )
                    .with_custom_curve(
                        state.wave_curves[1].value(state.controls.osc2_curve_fade[offset]),
                        state.controls.osc2_custom_shape[offset],
                    ),
                    OscillatorSettings::new(
                        oscillator_enabled[2],
                        state.controls.osc3_shape[offset],
                        state.controls.osc3_pulse_width[offset],
                        OscillatorSettings::pitch_ratio(
                            oscillator_transpose[2],
                            state.controls.osc3_cents[offset],
                        ),
                        state.controls.osc3_level[offset],
                        state.controls.osc3_pan[offset],
                    )
                    .with_unison_detune_amount(params.osc3_unison_detune_amount.value())
                    .with_phase_warp(
                        oscillator_warp_mode[2],
                        state.controls.osc3_warp_amount[offset],
                    )
                    .with_custom_curve(
                        state.wave_curves[2].value(state.controls.osc3_curve_fade[offset]),
                        state.controls.osc3_custom_shape[offset],
                    ),
                ]);
                let envelope = EnvelopeSettings {
                    attack: state.controls.attack[offset],
                    decay: state.controls.decay[offset],
                    sustain: state.controls.sustain[offset],
                    release: state.controls.release[offset],
                    attack_curve: state.controls.attack_curve[offset],
                    decay_curve: state.controls.decay_curve[offset],
                    release_curve: state.controls.release_curve[offset],
                    attack_curve_time: state.controls.attack_curve_time[offset],
                    decay_curve_time: state.controls.decay_curve_time[offset],
                    release_curve_time: state.controls.release_curve_time[offset],
                };
                let oversampling_factor = state.oversampler.factor();
                let block_samples = state
                    .synth
                    .block_internal_samples(settings, oversampling_factor);
                let block_internal = block_samples.unwrap_or(0);
                let base_host_frames = block_internal / usize::from(oversampling_factor);
                let available_frames = block_len - offset;
                let event_free_frames = events.get(next_event).map_or(available_frames, |event| {
                    (event.sample_offset as usize)
                        .saturating_sub(sample_index)
                        .min(available_frames)
                });
                let mut chunks = if base_host_frames == 0 {
                    0
                } else {
                    (event_free_frames / base_host_frames).min(MAX_JOB_SAMPLES / block_internal)
                };
                let mut morphing = false;
                while chunks != 0 {
                    let frames = base_host_frames * chunks;
                    if state.controls.is_static(offset, frames, oscillator_enabled) {
                        break;
                    }
                    if state
                        .controls
                        .is_static_except_shape(offset, frames, oscillator_enabled)
                        && state.synth.morph_block_eligible(settings)
                    {
                        morphing = true;
                        break;
                    }
                    chunks -= 1;
                }
                let host_frames = base_host_frames * chunks;
                if chunks != 0 && state.block_major_enabled() && !state.lfos.is_active() {
                    let gain = db_to_linear(state.controls.output_db[offset]);
                    let shapes = morphing.then(|| {
                        state.controls.expanded_shapes(
                            offset,
                            host_frames,
                            usize::from(oversampling_factor),
                        )
                    });
                    let (block_peak_left, block_peak_right) = match block_samples {
                        Some(FACTOR3_BLOCK_INTERNAL_SAMPLES) => {
                            render_saw_host_block::<FACTOR3_BLOCK_INTERNAL_SAMPLES>(
                                state,
                                buffer,
                                output_channels,
                                sample_index,
                                chunks,
                                settings,
                                envelope,
                                gain,
                                shapes.as_ref(),
                            )
                        }
                        Some(BLOCK_INTERNAL_SAMPLES) => {
                            render_saw_host_block::<BLOCK_INTERNAL_SAMPLES>(
                                state,
                                buffer,
                                output_channels,
                                sample_index,
                                chunks,
                                settings,
                                envelope,
                                gain,
                                shapes.as_ref(),
                            )
                        }
                        _ => unreachable!(),
                    };
                    peak_left = peak_left.max(block_peak_left);
                    peak_right = peak_right.max(block_peak_right);
                    state
                        .lfos
                        .advance_silent(host_frames * usize::from(oversampling_factor));
                    state.decimator_tail = oversampling::TAIL_SAMPLES;
                    #[cfg(test)]
                    {
                        if !morphing {
                            state.block_major_chunks += 1;
                        }
                    }
                    offset += host_frames;
                    continue;
                }
                let source_was_active = state.synth.is_active();
                let modulation_active = state.lfos.is_active() || direct_unison_pitch_active;
                let mut modulation = lfo::ModulationFrame::default();
                let (mut left, mut right) = if state.oversampler.factor() == 1 {
                    if modulation_active {
                        modulation = apply_modulation(
                            state,
                            &mut settings,
                            &active_routes,
                            &unison_settings,
                            state.controls.glide_time[offset],
                            lfo_controls_static,
                            offset,
                        );
                    }
                    let (left, right) = state.synth.render_with_modulation(
                        settings,
                        modulated_envelope(envelope, modulation.global),
                        modulation.unison,
                    );
                    state.oversampler.process_direct(left, right)
                } else if state.oversampler.factor() == 2
                    && !state.synth.is_gliding()
                    && !state.lfos.is_active()
                    && !direct_unison_pitch_active
                {
                    for (left, right) in state.synth.render_pair(settings, envelope) {
                        state.oversampler.push(left, right);
                    }
                    state.oversampler.output()
                } else {
                    let reuse_direct_modulation = modulation_active
                        && !state.lfos.is_active()
                        && active_routes.unison_layout_mask == 0;
                    for internal_sample in 0..usize::from(state.oversampler.factor()) {
                        let render_settings = if modulation_active {
                            if reuse_direct_modulation && internal_sample != 0 {
                                settings
                            } else {
                                let mut modulated = settings;
                                modulation = apply_modulation(
                                    state,
                                    &mut modulated,
                                    &active_routes,
                                    &unison_settings,
                                    state.controls.glide_time[offset],
                                    lfo_controls_static,
                                    offset,
                                );
                                modulated
                            }
                        } else {
                            settings
                        };
                        let (left, right) = state.synth.render_with_modulation(
                            render_settings,
                            modulated_envelope(envelope, modulation.global),
                            modulation.unison,
                        );
                        state.oversampler.push(left, right);
                    }
                    state.oversampler.output()
                };
                if !modulation_active {
                    state
                        .lfos
                        .advance_silent(usize::from(state.oversampler.factor()));
                }

                if source_was_active || state.synth.is_active() {
                    state.decimator_tail = oversampling::TAIL_SAMPLES;
                } else {
                    state.decimator_tail = state.decimator_tail.saturating_sub(1);
                    if state.set_oversampling(requested_factor, requested_antialiasing) {
                        antialiasing =
                            requested_antialiasing.for_factor(state.oversampler.factor());
                    }
                }

                let gain = static_gain.map_or_else(
                    || db_to_linear(state.controls.output_db[offset] + modulation.global.output_db),
                    |base| base * db_to_linear(modulation.global.output_db),
                );
                left *= gain;
                right *= gain;
                peak_left = peak_left.max(left.abs());
                peak_right = peak_right.max(right.abs());
                if output_channels == 1 {
                    buffer.output(0)[sample_index] = (left + right) * 0.5;
                } else {
                    buffer.output(0)[sample_index] = left;
                    buffer.output(1)[sample_index] = right;
                }
                for channel in 2..output_channels {
                    buffer.output(channel)[sample_index] = (left + right) * 0.5;
                }
                offset += 1;
            }
            block_start += block_len;
        }

        publish_meters(
            state,
            params,
            context,
            buffer.num_samples(),
            peak_left,
            peak_right,
        );

        current_process_status(state)
    }

    fn latency(_state: &KurvDspState) -> u32 {
        oversampling::LATENCY_SAMPLES
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the bounded host sample rate and 12-second release fit comfortably in u32"
    )]
    fn tail(state: &KurvDspState) -> u32 {
        (state.host_sample_rate * 12.0).round() as u32 + u32::from(oversampling::TAIL_SAMPLES)
    }

    fn migrate_state(foreign: &ForeignState) -> Option<MigratedState> {
        diagnostics::lifecycle("migrate-state-enter");
        let ForeignState::Raw { bytes, .. } = foreign else {
            diagnostics::lifecycle("migrate-state-not-raw");
            return None;
        };
        diagnostics::trace("state", "migrate-state-bytes", bytes.len() as f32, 0.0);
        let root: serde_json::Value = match serde_json::from_slice(bytes) {
            Ok(root) => root,
            Err(_) => {
                diagnostics::lifecycle("migrate-state-json-failed");
                return None;
            }
        };
        let Some(old) = root.get("params").and_then(serde_json::Value::as_object) else {
            diagnostics::lifecycle("migrate-state-params-missing");
            return None;
        };
        let mappings = [
            ("gain", P::OutputDb.into()),
            ("wave", P::Shape.into()),
            ("pw", P::PulseWidth.into()),
            ("attack", P::Attack.into()),
            ("decay", P::Decay.into()),
            ("sustain", P::Sustain.into()),
            ("release", P::Release.into()),
            ("drone", P::Drone.into()),
            ("freq", P::DroneFrequency.into()),
        ];
        let params = mappings
            .into_iter()
            .filter_map(|(old_id, new_id)| old_plain_value(old.get(old_id)?).map(|v| (new_id, v)))
            .collect::<Vec<_>>();
        if params.is_empty() {
            diagnostics::lifecycle("migrate-state-no-legacy-params");
            return None;
        }
        diagnostics::trace("state", "migrate-state-return", params.len() as f32, 0.0);
        Some(MigratedState {
            params,
            ..MigratedState::default()
        })
    }

    fn editor(params: Arc<KurvParams>) -> Box<dyn Editor> {
        editor::create(params)
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
