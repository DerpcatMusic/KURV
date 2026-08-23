//! Fixed-allocation polyphonic virtual-analog voice engine.

#[path = "voice/block_render.rs"]
mod block_render;
#[path = "voice/envelope.rs"]
mod envelope;
#[path = "internal_rt_pool.rs"]
mod internal_rt_pool;
#[path = "voice/render.rs"]
mod render;
#[path = "voice/resynth.rs"]
mod resynth;

pub use envelope::EnvelopeSettings;
use envelope::{EnvelopeStage, GroupVoiceEnvelope, shaped_progress};
#[cfg(test)]
use resynth::generate_resynth_step;
use resynth::{
    apply_resynth_bus_mix, generate_resynth_step_modulated, grain_uses_single_oscillator_lane,
};

use super::oscillator_bank::{
    ActiveOscillatorRenderEntry, ActiveOscillatorRenderSet, OscillatorBankVoiceState,
    OscillatorDspSettings, PhaseWarpControl, StructuralOscillatorAbsoluteControl,
    StructuralOscillatorFrameControl, fill_oscillator_unison_layout, shortest_phase_delta,
    unit_hash,
};
use super::poly_synth::{
    PolySynth, UnisonFrameControl, VoiceStructuralRouteFrame, merge_voice_structural_block_control,
    voice_filter_coefficient,
};
use super::unison::{
    ALIGNMENT_EPSILON, SwarmMode, UnisonLayout, UnisonSettings, build_spatial_from_components,
    extended_unison_rate, fill_extended_unison_jitter_offsets, fill_unison_jitter_offsets_mode,
    jitter_pitch_ratios, stereo_square_weights,
};
use super::{MAX_UNISON, OscillatorMask};

pub use internal_rt_pool::{InternalRtPool, MAX_JOB_SAMPLES};

use crate::filters::{FilterCoefficients, FilterConfig, StereoTptSvf};
use crate::generators::{
    GeneratorRtGroup, GeneratorRtModule, MAX_FILTERS, MAX_OSCILLATORS, MAX_OUTPUT_PAIRS,
    OscillatorEngineKind,
};
use crate::modulators::lfo::{UnisonModulation, VoiceLfoProgram, VoiceLfoState};
use crate::oscillators::{
    Antialiasing, GrainSchedulerState, PhaseWarpMode, ProductionResynthArtifact,
    SourceAuditionState, VaOscillator, accumulate_custom4_block, accumulate_custom4_block_constant,
    accumulate_custom8_block, accumulate_custom8_block_constant, accumulate_saw4_block,
    accumulate_saw4_block_constant, accumulate_saw4_block_dynamic_gains,
    accumulate_saw4_block_static_gains, accumulate_saw8_block, accumulate_saw8_block_constant,
    accumulate_saw8_block_dynamic_gains, accumulate_saw8_block_static_gains,
    accumulate_saw8_block_static_gains_narrow_spline, accumulate_shape4_block_constant,
    accumulate_shape4_block_constant_warped, accumulate_shape4_block_dynamic,
    accumulate_shape4_block_morphing, accumulate_shape4_block_steps,
    accumulate_shape8_block_constant, accumulate_shape8_block_constant_warped,
    accumulate_shape8_block_dynamic, accumulate_shape8_block_morphing,
    accumulate_shape8_block_steps, generate_custom4, generate_custom8, generate_pulse4,
    generate_pulse8, generate_saw4, generate_saw8, generate_shape4, generate_shape4_pair,
    generate_shape4_pair_warped, generate_shape4_warped, generate_shape8, generate_shape8_pair,
    generate_shape8_pair_warped, generate_shape8_warped, generate_sine4, generate_sine8,
    generate_triangle4, generate_triangle8, is_narrow_spline_ramp, shape_morph_gain,
};
use crate::wave_curve::WaveCurveRt;
use truce_simd::simd::{f32x4, f32x8};

pub const POLYPHONY: usize = 32;
// Frozen host/state compatibility width. New oscillator modules never use this
// as their capacity; the reusable structural bank below is always 32 slots.
pub const LEGACY_OSCILLATOR_COUNT: usize = 3;
const LEGACY_OSCILLATOR_MASK: OscillatorMask =
    OscillatorMask::MAX >> (MAX_OSCILLATORS - LEGACY_OSCILLATOR_COUNT);
pub(super) const POLYPHONY_U8: u8 = 32;
pub(super) const MASTER_HEADROOM: f32 = 0.8;
const RICH_ZONE_HANDOVER_SAMPLES: u8 = 64;

const SWARM_MIN_UPDATE_INTERVAL: u16 = 32;
const SWARM_MAX_UPDATE_INTERVAL: u16 = 1_024;
pub const BLOCK_INTERNAL_SAMPLES: usize = 32;
pub const FACTOR3_BLOCK_INTERNAL_SAMPLES: usize = 24;
#[allow(
    dead_code,
    reason = "legacy source compatibility for the old generator example"
)]
pub const WANDER_BLOCK_INTERNAL_SAMPLES: usize = BLOCK_INTERNAL_SAMPLES;

#[derive(Clone, Copy)]
struct LegacyScalarFrame {
    primary: (f32, f32),
    secondary: [(f32, f32); LEGACY_OSCILLATOR_COUNT - 1],
    amplitude: f32,
    has_secondary: bool,
}

impl LegacyScalarFrame {
    fn mixed(self) -> (f32, f32) {
        if !self.has_secondary {
            return self.primary;
        }
        let mut extra_left = 0.0;
        let mut extra_right = 0.0;
        for (left, right) in self.secondary {
            extra_left += left;
            extra_right += right;
        }
        (
            extra_left.mul_add(self.amplitude, self.primary.0),
            extra_right.mul_add(self.amplitude, self.primary.1),
        )
    }

    fn accumulate_grouped(
        self,
        stems: &mut [(f32, f32); MAX_OUTPUT_PAIRS],
        oscillator_groups: &[u8; MAX_OSCILLATORS],
        group_count: usize,
        envelope_gains: &[f32; MAX_OUTPUT_PAIRS],
    ) {
        let primary_group = oscillator_group(oscillator_groups, group_count, 0);
        stems[primary_group].0 += self.primary.0 * envelope_gains[primary_group];
        stems[primary_group].1 += self.primary.1 * envelope_gains[primary_group];
        for (secondary, (left, right)) in self.secondary.into_iter().enumerate() {
            let slot = secondary + 1;
            let group = oscillator_group(oscillator_groups, group_count, slot);
            let gain = self.amplitude * envelope_gains[group];
            stems[group].0 += left * gain;
            stems[group].1 += right * gain;
        }
    }
}

#[inline]
fn oscillator_group(
    oscillator_groups: &[u8; MAX_OSCILLATORS],
    group_count: usize,
    slot: usize,
) -> usize {
    let group = usize::from(oscillator_groups[slot]);
    if group < group_count { group } else { 0 }
}

#[inline]
pub(super) const fn midi_channel_matches(filter: u8, channel: u8) -> bool {
    filter == 0 || filter == channel.saturating_add(1)
}

#[derive(Clone, Copy, Debug)]
pub struct OscillatorSettings {
    pub enabled: bool,
    pub shape: f32,
    pub pulse_width: f32,
    pub pitch_ratio: f32,
    pub unison_detune_amount: f32,
    pub level: f32,
    pub pan: f32,
    pub phase_warp: PhaseWarpControl,
    pub custom_curve: WaveCurveRt,
    pub custom_mix: f32,
    pub positioned_wave: bool,
    left_gain: f32,
    right_gain: f32,
}

impl OscillatorSettings {
    pub fn new(
        enabled: bool,
        shape: f32,
        pulse_width: f32,
        pitch_ratio: f32,
        level: f32,
        pan: f32,
    ) -> Self {
        let level = level.clamp(0.0, 1.0);
        let pan = pan.clamp(-1.0, 1.0);
        Self {
            enabled,
            shape: shape.clamp(0.0, 3.0),
            pulse_width: pulse_width.clamp(0.03, 0.97),
            pitch_ratio: pitch_ratio.clamp(1.0 / 16.0, 16.0),
            unison_detune_amount: 1.0,
            level,
            pan,
            phase_warp: PhaseWarpControl::NONE,
            custom_curve: WaveCurveRt::zero(),
            custom_mix: 0.0,
            positioned_wave: false,
            left_gain: level * (1.0 - pan).sqrt(),
            right_gain: level * (1.0 + pan).sqrt(),
        }
    }

    pub fn pitch_ratio(transpose: f32, cents: f32) -> f32 {
        ((transpose.clamp(-48.0, 48.0) + cents.clamp(-100.0, 100.0) * 0.01) / 12.0).exp2()
    }

    const fn legacy(shape: f32, pulse_width: f32) -> Self {
        Self {
            enabled: true,
            shape,
            pulse_width,
            pitch_ratio: 1.0,
            unison_detune_amount: 1.0,
            level: 1.0,
            pan: 0.0,
            phase_warp: PhaseWarpControl::NONE,
            custom_curve: WaveCurveRt::zero(),
            custom_mix: 0.0,
            positioned_wave: false,
            left_gain: 1.0,
            right_gain: 1.0,
        }
    }

    const fn disabled() -> Self {
        Self {
            enabled: false,
            shape: 2.0,
            pulse_width: 0.5,
            pitch_ratio: 1.0,
            unison_detune_amount: 1.0,
            level: 1.0,
            pan: 0.0,
            phase_warp: PhaseWarpControl::NONE,
            custom_curve: WaveCurveRt::zero(),
            custom_mix: 0.0,
            positioned_wave: false,
            left_gain: 1.0,
            right_gain: 1.0,
        }
    }

    fn channel_gains(self) -> (f32, f32) {
        (self.left_gain, self.right_gain)
    }

    pub const fn with_phase_warp(mut self, mode: PhaseWarpMode, amount: f32) -> Self {
        self.phase_warp = PhaseWarpControl::new(mode, amount);
        self
    }

    pub const fn with_unison_detune_amount(mut self, amount: f32) -> Self {
        self.unison_detune_amount = amount.clamp(0.0, 1.0);
        self
    }

    pub fn with_custom_curve(mut self, curve: WaveCurveRt, mix: f32) -> Self {
        self.custom_curve = curve;
        self.custom_mix = mix.clamp(0.0, 1.0);
        self
    }

    pub const fn with_positioned_wave(mut self, positioned: bool) -> Self {
        self.positioned_wave = positioned;
        self
    }

    pub(super) fn custom_active(self) -> bool {
        self.custom_mix > f32::EPSILON
    }

    pub(super) fn phase_warp_active(self) -> bool {
        self.phase_warp.active()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct VoiceSettings {
    pub shape: f32,
    pub frequency_hz: f32,
    pub pulse_width: f32,
    pub velocity_amount: f32,
    pub pressure_amount: f32,
    pub timbre_amount: f32,
    pub antialiasing: Antialiasing,
    pub oscillators: [OscillatorSettings; LEGACY_OSCILLATOR_COUNT],
}

impl VoiceSettings {
    pub const fn new(
        shape: f32,
        frequency_hz: f32,
        pulse_width: f32,
        velocity_amount: f32,
        pressure_amount: f32,
        timbre_amount: f32,
    ) -> Self {
        Self {
            shape,
            frequency_hz,
            pulse_width,
            velocity_amount,
            pressure_amount,
            timbre_amount,
            antialiasing: Antialiasing::Spline,
            oscillators: [
                OscillatorSettings::legacy(shape, pulse_width),
                OscillatorSettings::disabled(),
                OscillatorSettings::disabled(),
            ],
        }
    }

    pub const fn with_antialiasing(mut self, antialiasing: Antialiasing) -> Self {
        self.antialiasing = antialiasing;
        self
    }

    pub const fn with_oscillators(
        mut self,
        oscillators: [OscillatorSettings; LEGACY_OSCILLATOR_COUNT],
    ) -> Self {
        self.oscillators = oscillators;
        self
    }

    pub fn modulate_oscillator(
        &mut self,
        index: usize,
        pitch_semitones: f32,
        shape: f32,
        pulse_width: f32,
        warp: f32,
        custom_shape: f32,
        level: f32,
        pan: f32,
    ) {
        let Some(oscillator) = self.oscillators.get_mut(index) else {
            return;
        };
        if pitch_semitones != 0.0 {
            oscillator.pitch_ratio = (oscillator.pitch_ratio
                * (pitch_semitones.clamp(-96.0, 96.0) / 12.0).exp2())
            .clamp(1.0 / 256.0, 256.0);
        }
        if shape != 0.0 {
            oscillator.shape = (oscillator.shape + shape).clamp(0.0, 3.0);
        }
        if pulse_width != 0.0 {
            oscillator.pulse_width = (oscillator.pulse_width + pulse_width).clamp(0.03, 0.97);
        }
        if warp != 0.0 {
            oscillator.phase_warp.amount = (oscillator.phase_warp.amount + warp).clamp(0.0, 1.0);
        }
        if custom_shape != 0.0 {
            oscillator.custom_mix = (oscillator.custom_mix + custom_shape).clamp(0.0, 1.0);
        }
        if level != 0.0 || pan != 0.0 {
            oscillator.level = (oscillator.level + level).clamp(0.0, 1.0);
            oscillator.pan = (oscillator.pan + pan).clamp(-1.0, 1.0);
            oscillator.left_gain = oscillator.level * (1.0 - oscillator.pan).sqrt();
            oscillator.right_gain = oscillator.level * (1.0 + oscillator.pan).sqrt();
        }
        if index == 0 {
            self.shape = oscillator.shape;
            self.pulse_width = oscillator.pulse_width;
        }
    }

    pub fn modulate_unison_detune_amount(&mut self, index: usize, amount: f32) {
        if amount == 0.0 {
            return;
        }
        if let Some(oscillator) = self.oscillators.get_mut(index) {
            oscillator.unison_detune_amount =
                (oscillator.unison_detune_amount + amount).clamp(0.0, 1.0);
        }
    }

    pub(super) fn oscillator(self, index: usize) -> OscillatorSettings {
        let mut oscillator = self.oscillators[index];
        if index == 0 {
            oscillator.shape = self.shape;
            oscillator.pulse_width = self.pulse_width;
        }
        oscillator
    }

    fn has_secondary_oscillators(self) -> bool {
        self.oscillators[1..]
            .iter()
            .any(|oscillator| oscillator.enabled)
    }

    pub(super) fn legacy_primary_fast_path(self) -> bool {
        let primary = self.oscillator(0);
        primary.enabled
            && !self.has_secondary_oscillators()
            && !primary.phase_warp_active()
            && !primary.custom_active()
            && primary.pitch_ratio.to_bits() == 1.0_f32.to_bits()
            && primary.level.to_bits() == 1.0_f32.to_bits()
            && primary.pan.to_bits() == 0.0_f32.to_bits()
    }
}

pub struct VaVoice {
    pub(super) modulation: VoiceLfoState,
    oscillators: [[VaOscillator; MAX_UNISON]; LEGACY_OSCILLATOR_COUNT],
    pub(super) oscillator_bank: Box<OscillatorBankVoiceState>,
    filters: [StereoTptSvf; MAX_FILTERS],
    enabled_filter_mask: u32,
    pub(super) unison: UnisonLayout,
    current_note: Option<u8>,
    voice_id: Option<i32>,
    pub(super) channel: u8,
    pub(super) age: u64,
    frequency_hz: f32,
    glide_target_hz: f32,
    glide_multiplier: f32,
    glide_remaining: u32,
    pitch_ratio: f32,
    sample_rate: f32,
    phase_steps: [f32; MAX_UNISON],
    pub(super) phase_steps_dirty: bool,
    swarm_clock: f32,
    swarm_clock_offset: f32,
    swarm_update_remaining: u16,
    swarm_pitch_step: [f32; MAX_UNISON],
    enabled_oscillator_mask: OscillatorMask,
    pub(super) note_seed: u64,
    velocity: f32,
    pub(super) pressure: f32,
    pub(super) timbre: f32,
    pub(super) envelope_level: f32,
    envelope_start: f32,
    envelope_progress: f32,
    envelope_step: f32,
    stage: EnvelopeStage,
    pub(super) held: bool,
    pub(super) sustained: bool,
    envelope: EnvelopeSettings,
    group_envelopes: [GroupVoiceEnvelope; MAX_OUTPUT_PAIRS],
    group_midi_channels: [u8; MAX_OUTPUT_PAIRS],
    group_envelope_count: u8,
    group_active_mask: u8,
    pub(super) secondary_unison: [UnisonLayout; LEGACY_OSCILLATOR_COUNT - 1],
    secondary_phase_steps: [[f32; MAX_UNISON]; LEGACY_OSCILLATOR_COUNT - 1],
    pub(super) secondary_phase_steps_dirty: [bool; LEGACY_OSCILLATOR_COUNT - 1],
    secondary_swarm_clock: [f32; LEGACY_OSCILLATOR_COUNT - 1],
    secondary_swarm_clock_offset: [f32; LEGACY_OSCILLATOR_COUNT - 1],
    secondary_swarm_update_remaining: [u16; LEGACY_OSCILLATOR_COUNT - 1],
    secondary_swarm_pitch_step: [[f32; MAX_UNISON]; LEGACY_OSCILLATOR_COUNT - 1],
    dynamic_unison_left: [[f32; MAX_UNISON]; LEGACY_OSCILLATOR_COUNT],
    dynamic_unison_right: [[f32; MAX_UNISON]; LEGACY_OSCILLATOR_COUNT],
    dynamic_unison_gain: [f32; LEGACY_OSCILLATOR_COUNT],
    dynamic_spatial_modulation: [crate::modulators::lfo::UnisonModulation; LEGACY_OSCILLATOR_COUNT],
    dynamic_spatial_valid: OscillatorMask,
}

#[derive(Clone, Copy)]
pub(crate) struct PitchModulationFrame {
    pub oscillator_pitch_ratios: [f32; LEGACY_OSCILLATOR_COUNT],
    pub unison_pitch_correction: [[f32; MAX_UNISON]; LEGACY_OSCILLATOR_COUNT],
    pub unison_active_mask: OscillatorMask,
    pub unison_spatial_left: [[f32; MAX_UNISON]; LEGACY_OSCILLATOR_COUNT],
    pub unison_spatial_right: [[f32; MAX_UNISON]; LEGACY_OSCILLATOR_COUNT],
    pub unison_spatial_gain: [f32; LEGACY_OSCILLATOR_COUNT],
    pub unison_spatial_active_mask: OscillatorMask,
}

impl Default for PitchModulationFrame {
    fn default() -> Self {
        Self {
            oscillator_pitch_ratios: [1.0; LEGACY_OSCILLATOR_COUNT],
            unison_pitch_correction: [[1.0; MAX_UNISON]; LEGACY_OSCILLATOR_COUNT],
            unison_active_mask: 0,
            unison_spatial_left: [[1.0; MAX_UNISON]; LEGACY_OSCILLATOR_COUNT],
            unison_spatial_right: [[1.0; MAX_UNISON]; LEGACY_OSCILLATOR_COUNT],
            unison_spatial_gain: [1.0; LEGACY_OSCILLATOR_COUNT],
            unison_spatial_active_mask: 0,
        }
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct UnisonMotionFrame {
    pub phase_random: f32,
    pub swarm_amount: f32,
    pub swarm_rate: f32,
}

impl Default for VaVoice {
    fn default() -> Self {
        Self {
            modulation: VoiceLfoState::default(),
            oscillators: std::array::from_fn(|_| std::array::from_fn(|_| VaOscillator::default())),
            oscillator_bank: Box::new(OscillatorBankVoiceState::default()),
            filters: std::array::from_fn(|_| StereoTptSvf::default()),
            enabled_filter_mask: 0,
            unison: UnisonLayout::default(),
            current_note: None,
            voice_id: None,
            channel: 0,
            age: 0,
            frequency_hz: 110.0,
            glide_target_hz: 110.0,
            glide_multiplier: 1.0,
            glide_remaining: 0,
            pitch_ratio: 1.0,
            sample_rate: 44_100.0,
            phase_steps: [0.0; MAX_UNISON],
            phase_steps_dirty: true,
            swarm_clock: 0.0,
            swarm_clock_offset: 0.0,
            swarm_update_remaining: 0,
            swarm_pitch_step: [0.0; MAX_UNISON],
            enabled_oscillator_mask: 1,
            note_seed: 0,
            velocity: 1.0,
            pressure: 0.0,
            timbre: 0.5,
            envelope_level: 0.0,
            envelope_start: 0.0,
            envelope_progress: 0.0,
            envelope_step: 1.0,
            stage: EnvelopeStage::Idle,
            held: false,
            sustained: false,
            envelope: EnvelopeSettings::default(),
            group_envelopes: [GroupVoiceEnvelope::default(); MAX_OUTPUT_PAIRS],
            group_midi_channels: [0; MAX_OUTPUT_PAIRS],
            group_envelope_count: 0,
            group_active_mask: 1,
            secondary_unison: std::array::from_fn(|_| UnisonLayout::default()),
            secondary_phase_steps: [[0.0; MAX_UNISON]; LEGACY_OSCILLATOR_COUNT - 1],
            secondary_phase_steps_dirty: [true; LEGACY_OSCILLATOR_COUNT - 1],
            secondary_swarm_clock: [0.0; LEGACY_OSCILLATOR_COUNT - 1],
            secondary_swarm_clock_offset: [0.0; LEGACY_OSCILLATOR_COUNT - 1],
            secondary_swarm_update_remaining: [0; LEGACY_OSCILLATOR_COUNT - 1],
            secondary_swarm_pitch_step: [[0.0; MAX_UNISON]; LEGACY_OSCILLATOR_COUNT - 1],
            dynamic_unison_left: [[0.0; MAX_UNISON]; LEGACY_OSCILLATOR_COUNT],
            dynamic_unison_right: [[0.0; MAX_UNISON]; LEGACY_OSCILLATOR_COUNT],
            dynamic_unison_gain: [0.0; LEGACY_OSCILLATOR_COUNT],
            dynamic_spatial_modulation: [crate::modulators::lfo::UnisonModulation::default();
                LEGACY_OSCILLATOR_COUNT],
            dynamic_spatial_valid: 0,
        }
    }
}

impl VaVoice {
    pub(super) fn trigger_modulation(&mut self, program: &VoiceLfoProgram) {
        if let Some(note) = self.current_note {
            self.modulation.trigger(note, self.note_seed, program);
        }
    }

    pub(super) fn activate_modulation_sources(
        &mut self,
        program: &VoiceLfoProgram,
        source_mask: u64,
    ) {
        if let Some(note) = self.current_note {
            self.modulation
                .activate(note, self.note_seed, program, source_mask);
            if !self.held && !self.sustained {
                self.modulation.release(program);
            }
        }
    }

    pub(super) fn release_modulation(&mut self, program: &VoiceLfoProgram) {
        if !self.held && !self.sustained {
            self.modulation.release(program);
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        let sample_rate = sample_rate.max(1.0);
        if self.sample_rate.to_bits() != sample_rate.to_bits() {
            self.swarm_update_remaining = 0;
            self.secondary_swarm_update_remaining.fill(0);
        }
        self.sample_rate = sample_rate;
        self.refresh_envelope_step();
        for envelope in &mut self.group_envelopes {
            envelope.refresh_step(sample_rate);
        }
        self.phase_steps_dirty = true;
        self.secondary_phase_steps_dirty.fill(true);
    }

    pub fn reset(&mut self) {
        self.modulation = VoiceLfoState::default();
        self.dynamic_spatial_valid = 0;
        self.reset_oscillators();
        self.reset_filters();
        self.current_note = None;
        self.voice_id = None;
        self.frequency_hz = 110.0;
        self.glide_target_hz = 110.0;
        self.glide_multiplier = 1.0;
        self.glide_remaining = 0;
        self.pitch_ratio = 1.0;
        self.phase_steps.fill(0.0);
        self.phase_steps_dirty = true;
        self.secondary_phase_steps.fill([0.0; MAX_UNISON]);
        self.secondary_phase_steps_dirty.fill(true);
        self.swarm_clock = 0.0;
        self.swarm_clock_offset = 0.0;
        self.secondary_swarm_clock.fill(0.0);
        self.secondary_swarm_clock_offset.fill(0.0);
        self.reset_all_swarm_motion();
        self.note_seed = 0;
        self.velocity = 1.0;
        self.pressure = 0.0;
        self.timbre = 0.5;
        self.envelope_level = 0.0;
        self.envelope_start = 0.0;
        self.envelope_progress = 0.0;
        self.envelope_step = 1.0;
        self.stage = EnvelopeStage::Idle;
        self.held = false;
        self.sustained = false;
        for envelope in &mut self.group_envelopes {
            envelope.finish();
        }
    }

    pub fn start(&mut self, note: u8, velocity: f32, channel: u8, voice_id: Option<i32>, age: u64) {
        self.dynamic_spatial_valid = 0;
        self.current_note = Some(note);
        self.voice_id = voice_id;
        self.channel = channel.min(15);
        self.age = age;
        let seed = note_phase_seed(note, self.channel, voice_id, age);
        self.note_seed = seed;
        self.seed_swarm_clocks(seed);
        self.randomize_oscillators(seed);
        self.seed_enabled_unison_layouts(seed);
        self.reset_enabled_swarm_motion();
        self.frequency_hz = crate::dsp::midi_note_hz(f32::from(note));
        self.glide_target_hz = self.frequency_hz;
        self.glide_multiplier = 1.0;
        self.glide_remaining = 0;
        self.phase_steps_dirty = true;
        self.secondary_phase_steps_dirty.fill(true);
        self.velocity = velocity.clamp(0.0, 1.0);
        self.pressure = 0.0;
        self.timbre = 0.5;
        self.envelope_level = 0.0;
        if self.group_envelope_count == 0 {
            self.begin_attack();
        } else {
            self.begin_group_envelopes();
        }
        self.held = true;
        self.sustained = false;
    }

    pub(super) fn retrigger(&mut self, velocity: f32, voice_id: Option<i32>, age: u64) {
        self.dynamic_spatial_valid = 0;
        self.voice_id = voice_id;
        self.age = age;
        let seed = note_phase_seed(self.current_note.unwrap_or(69), self.channel, voice_id, age);
        self.note_seed = seed;
        self.seed_swarm_clocks(seed);
        self.randomize_oscillators(seed);
        self.seed_enabled_unison_layouts(seed);
        self.reset_enabled_swarm_motion();
        self.velocity = velocity.clamp(0.0, 1.0);
        self.pressure = 0.0;
        if self.group_envelope_count == 0 {
            self.begin_attack();
        } else {
            self.begin_group_envelopes();
        }
        self.held = true;
        self.sustained = false;
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::cast_sign_loss,
        reason = "the glide duration is clamped to five seconds at the bounded DSP sample rate"
    )]
    pub(super) fn legato_to(
        &mut self,
        note: u8,
        velocity: f32,
        channel: u8,
        voice_id: Option<i32>,
        age: u64,
        glide_time: f32,
    ) {
        self.current_note = Some(note);
        self.voice_id = voice_id;
        self.channel = channel.min(15);
        self.age = age;
        self.note_seed = note_phase_seed(note, self.channel, voice_id, age);
        self.glide_target_hz = crate::dsp::midi_note_hz(f32::from(note));
        let samples = (glide_time.clamp(0.0, 5.0) * self.sample_rate).round() as u32;
        if samples == 0 || (self.glide_target_hz - self.frequency_hz).abs() <= f32::EPSILON {
            self.frequency_hz = self.glide_target_hz;
            self.glide_multiplier = 1.0;
            self.glide_remaining = 0;
            self.phase_steps_dirty = true;
            self.secondary_phase_steps_dirty.fill(true);
        } else {
            self.glide_multiplier =
                ((self.glide_target_hz / self.frequency_hz).log2() / samples as f32).exp2();
            self.glide_remaining = samples;
        }
        self.velocity = velocity.clamp(0.0, 1.0);
        self.pressure = 0.0;
        self.held = true;
        self.sustained = false;
        if self.group_envelope_count != 0 {
            self.sync_group_midi(false);
        }
    }

    pub(super) const fn is_gliding(&self) -> bool {
        self.glide_remaining != 0
    }

    pub fn configure(&mut self, envelope: EnvelopeSettings) {
        if self.envelope == envelope {
            return;
        }
        let duration_changed = match self.stage {
            EnvelopeStage::Attack => self.envelope.attack.to_bits() != envelope.attack.to_bits(),
            EnvelopeStage::Decay => self.envelope.decay.to_bits() != envelope.decay.to_bits(),
            EnvelopeStage::Release => self.envelope.release.to_bits() != envelope.release.to_bits(),
            EnvelopeStage::Idle | EnvelopeStage::Sustain => false,
        };
        self.envelope = envelope;
        if duration_changed {
            self.refresh_envelope_step();
        }
    }

    pub(super) fn configure_output_groups(
        &mut self,
        envelopes: [EnvelopeSettings; MAX_OUTPUT_PAIRS],
        midi_channels: [u8; MAX_OUTPUT_PAIRS],
        count: usize,
        active_mask: u8,
        envelopes_enabled: bool,
    ) {
        let previous_count = usize::from(self.group_envelope_count);
        let count = count.clamp(1, MAX_OUTPUT_PAIRS);
        self.group_midi_channels = midi_channels;
        self.group_active_mask = active_mask & ((1_u16 << count) - 1) as u8;
        for (state, settings) in self.group_envelopes.iter_mut().zip(envelopes) {
            state.configure(settings, self.sample_rate);
        }
        self.group_envelope_count = if envelopes_enabled { count as u8 } else { 0 };
        if !self.active() {
            return;
        }
        if self.group_envelope_count == 0 {
            for envelope in &mut self.group_envelopes[..previous_count] {
                envelope.finish();
            }
            if self.group_active_mask & 1 == 0
                || !midi_channel_matches(self.group_midi_channels[0], self.channel)
            {
                self.finish_envelope();
            }
        } else {
            self.sync_group_midi(false);
            if !self.active_group_envelope_exists() {
                self.finish_envelope();
            }
        }
    }

    fn begin_group_envelopes(&mut self) {
        self.sync_group_midi(true);
        if self.active_group_envelope_exists() {
            self.envelope_level = 1.0;
            self.stage = EnvelopeStage::Attack;
        } else {
            self.finish_envelope();
        }
    }

    fn sync_group_midi(&mut self, retrigger: bool) {
        for group in 0..usize::from(self.group_envelope_count) {
            if self.group_active_mask & (1 << group) != 0
                && midi_channel_matches(self.group_midi_channels[group], self.channel)
            {
                if retrigger
                    || !self.group_envelopes[group].active() && (self.held || self.sustained)
                {
                    self.group_envelopes[group].note_on(self.sample_rate);
                }
            } else {
                self.group_envelopes[group].finish();
            }
        }
    }

    fn active_group_envelope_exists(&self) -> bool {
        self.group_envelopes[..usize::from(self.group_envelope_count)]
            .iter()
            .copied()
            .enumerate()
            .any(|(group, envelope)| {
                self.group_active_mask & (1 << group) != 0 && envelope.active()
            })
    }

    fn group_envelope_gains(&self) -> [f32; MAX_OUTPUT_PAIRS] {
        if self.group_envelope_count == 0 {
            return [1.0; MAX_OUTPUT_PAIRS];
        }
        std::array::from_fn(|group| self.group_envelopes[group].level)
    }

    /// Amplitude envelope for the ungrouped 1-group render path.
    /// Group envelopes hold `envelope_level` at 1.0 while the voice is alive;
    /// the sounding gain is the first group's ADSR.
    fn amplitude_level(&self) -> f32 {
        if self.group_envelope_count == 0 {
            self.envelope_level
        } else {
            self.envelope_level * self.group_envelopes[0].level
        }
    }

    pub fn configure_unison(&mut self, settings: UnisonSettings) -> bool {
        self.configure_unison_with_prepared(settings, None)
    }

    pub fn configure_unison_motion(&mut self, settings: UnisonSettings) {
        let changed = self.unison.configure_motion(settings);
        if changed && settings.motion_active() {
            self.swarm_update_remaining = self
                .swarm_update_remaining
                .min(self.swarm_update_interval());
        }
    }

    pub(super) fn configure_unison_with_prepared(
        &mut self,
        settings: UnisonSettings,
        prepared: Option<&UnisonLayout>,
    ) -> bool {
        let voice_count_changed = self.unison.settings.voices != settings.voices;
        let tuning_changed = voice_count_changed
            || self.unison.settings.detune_cents.to_bits() != settings.detune_cents.to_bits()
            || self.unison.settings.curve.to_bits() != settings.curve.to_bits()
            || self.unison.settings.detune_amount.to_bits() != settings.detune_amount.to_bits()
            || self.unison.settings.harmonic_align.to_bits() != settings.harmonic_align.to_bits()
            || self.unison.settings.alignment_mode != settings.alignment_mode;
        let mut previous_without_motion = self.unison.settings;
        previous_without_motion.swarm_amount = settings.swarm_amount;
        previous_without_motion.swarm_rate = settings.swarm_rate;
        previous_without_motion.swarm_mode = settings.swarm_mode;
        let motion_change_only = previous_without_motion == settings;
        let layout_changed = self.unison.configure_with_prepared(
            settings,
            self.sample_rate,
            self.active(),
            prepared,
        );
        if layout_changed {
            self.dynamic_spatial_valid = 0;
        }
        self.phase_steps_dirty |= layout_changed && tuning_changed;
        if layout_changed && !motion_change_only {
            if tuning_changed {
                self.reset_swarm_motion();
            }
        } else if motion_change_only && settings.motion_active() {
            self.swarm_update_remaining = self
                .swarm_update_remaining
                .min(self.swarm_update_interval());
        }
        layout_changed
    }

    pub fn configure_secondary_unison(
        &mut self,
        oscillator: usize,
        settings: UnisonSettings,
    ) -> bool {
        self.configure_secondary_unison_with_prepared(oscillator, settings, None)
    }

    pub fn configure_secondary_unison_motion(
        &mut self,
        oscillator: usize,
        settings: UnisonSettings,
    ) {
        let index = oscillator - 1;
        let changed = self.secondary_unison[index].configure_motion(settings);
        if changed && settings.motion_active() {
            self.secondary_swarm_update_remaining[index] = self.secondary_swarm_update_remaining
                [index]
                .min(self.secondary_swarm_update_interval(index));
        }
    }

    pub(super) fn configure_secondary_unison_with_prepared(
        &mut self,
        oscillator: usize,
        settings: UnisonSettings,
        prepared: Option<&UnisonLayout>,
    ) -> bool {
        let index = oscillator - 1;
        let voice_count_changed = self.secondary_unison[index].settings.voices != settings.voices;
        let tuning_changed = voice_count_changed
            || self.secondary_unison[index].settings.detune_cents.to_bits()
                != settings.detune_cents.to_bits()
            || self.secondary_unison[index].settings.curve.to_bits() != settings.curve.to_bits()
            || self.secondary_unison[index]
                .settings
                .detune_amount
                .to_bits()
                != settings.detune_amount.to_bits()
            || self.secondary_unison[index]
                .settings
                .harmonic_align
                .to_bits()
                != settings.harmonic_align.to_bits()
            || self.secondary_unison[index].settings.alignment_mode != settings.alignment_mode;
        let mut previous_without_motion = self.secondary_unison[index].settings;
        previous_without_motion.swarm_amount = settings.swarm_amount;
        previous_without_motion.swarm_rate = settings.swarm_rate;
        previous_without_motion.swarm_mode = settings.swarm_mode;
        let motion_change_only = previous_without_motion == settings;
        let layout_changed = self.secondary_unison[index].configure_with_prepared(
            settings,
            self.sample_rate,
            self.active(),
            prepared,
        );
        if layout_changed {
            self.dynamic_spatial_valid = 0;
        }
        self.secondary_phase_steps_dirty[index] |= layout_changed && tuning_changed;
        if layout_changed && !motion_change_only {
            if tuning_changed {
                self.reset_secondary_swarm_motion(index);
            }
        } else if motion_change_only && settings.motion_active() {
            self.secondary_swarm_update_remaining[index] = self.secondary_swarm_update_remaining
                [index]
                .min(self.secondary_swarm_update_interval(index));
        }
        layout_changed
    }

    fn prepare_dynamic_unison_spatial(&mut self, control: &UnisonFrameControl) {
        for oscillator in 0..LEGACY_OSCILLATOR_COUNT {
            if control.spatial_mask & (1 << oscillator) == 0
                || control.spatial_shared_mask & (1 << oscillator) != 0
            {
                continue;
            }
            let layout = if oscillator == 0 {
                &self.unison
            } else {
                &self.secondary_unison[oscillator - 1]
            };
            let (settings, random_seed) = (layout.settings, layout.random_seed);
            let settings = settings.modulated(control.spatial[oscillator]);
            let dynamic = control.spatial[oscillator];
            let bit = 1 << oscillator;
            let simple = dynamic.curve.abs() <= ALIGNMENT_EPSILON
                && dynamic.pan_center.abs() <= f32::EPSILON
                && dynamic.pan_left.abs() <= f32::EPSILON
                && dynamic.pan_right.abs() <= f32::EPSILON
                && dynamic.pan_center_x.abs() <= f32::EPSILON;
            let transition_active = if oscillator == 0 {
                self.unison.transition_active()
            } else {
                self.secondary_unison[oscillator - 1].transition_active()
            };
            if !transition_active
                && self.dynamic_spatial_valid & bit != 0
                && self.dynamic_spatial_modulation[oscillator] == dynamic
            {
                continue;
            }
            if simple && !transition_active {
                self.dynamic_unison_gain[oscillator] = if oscillator == 0 {
                    build_spatial_from_components(
                        &self.unison,
                        settings,
                        &mut self.dynamic_unison_left[oscillator],
                        &mut self.dynamic_unison_right[oscillator],
                    )
                } else {
                    build_spatial_from_components(
                        &self.secondary_unison[oscillator - 1],
                        settings,
                        &mut self.dynamic_unison_left[oscillator],
                        &mut self.dynamic_unison_right[oscillator],
                    )
                };
            } else {
                self.dynamic_unison_gain[oscillator] =
                    if control.dynamic_position_mask & (1 << oscillator) != 0 {
                        UnisonLayout::build_spatial_from_positions(
                            settings,
                            random_seed,
                            &control.dynamic_detune_positions[oscillator],
                            &mut self.dynamic_unison_left[oscillator],
                            &mut self.dynamic_unison_right[oscillator],
                        )
                    } else {
                        UnisonLayout::build_spatial_from_positions(
                            settings,
                            random_seed,
                            &layout.detune_positions,
                            &mut self.dynamic_unison_left[oscillator],
                            &mut self.dynamic_unison_right[oscillator],
                        )
                    };
            }
            if transition_active {
                self.dynamic_spatial_valid &= !bit;
            } else {
                self.dynamic_spatial_modulation[oscillator] = dynamic;
                self.dynamic_spatial_valid |= bit;
            }
        }
    }

    #[inline]
    fn unison_left_gain<const DYNAMIC_UNISON: bool>(
        &self,
        oscillator: usize,
        index: usize,
        control: &UnisonFrameControl,
    ) -> f32 {
        if !DYNAMIC_UNISON {
            return if oscillator == 0 {
                self.unison.left[index]
            } else {
                self.secondary_unison[oscillator - 1].left[index]
            };
        }
        if control.spatial_shared_mask & (1 << oscillator) != 0 {
            control.spatial_left[oscillator][index]
        } else if control.spatial_mask & (1 << oscillator) != 0 {
            self.dynamic_unison_left[oscillator][index]
        } else if oscillator == 0 {
            self.unison.left[index]
        } else {
            self.secondary_unison[oscillator - 1].left[index]
        }
    }

    #[inline]
    fn unison_right_gain<const DYNAMIC_UNISON: bool>(
        &self,
        oscillator: usize,
        index: usize,
        control: &UnisonFrameControl,
    ) -> f32 {
        if !DYNAMIC_UNISON {
            return if oscillator == 0 {
                self.unison.right[index]
            } else {
                self.secondary_unison[oscillator - 1].right[index]
            };
        }
        if control.spatial_shared_mask & (1 << oscillator) != 0 {
            control.spatial_right[oscillator][index]
        } else if control.spatial_mask & (1 << oscillator) != 0 {
            self.dynamic_unison_right[oscillator][index]
        } else if oscillator == 0 {
            self.unison.right[index]
        } else {
            self.secondary_unison[oscillator - 1].right[index]
        }
    }

    #[inline]
    fn unison_gains8<const DYNAMIC_UNISON: bool>(
        &self,
        oscillator: usize,
        index: usize,
        control: &UnisonFrameControl,
    ) -> (f32x8, f32x8) {
        if !DYNAMIC_UNISON {
            return if oscillator == 0 {
                (
                    f32x8::from(std::array::from_fn(|lane| self.unison.left[index + lane])),
                    f32x8::from(std::array::from_fn(|lane| self.unison.right[index + lane])),
                )
            } else {
                let layout = &self.secondary_unison[oscillator - 1];
                (
                    f32x8::from(std::array::from_fn(|lane| layout.left[index + lane])),
                    f32x8::from(std::array::from_fn(|lane| layout.right[index + lane])),
                )
            };
        }
        let bit = 1 << oscillator;
        if control.spatial_shared_mask & bit != 0 {
            (
                f32x8::from(std::array::from_fn(|lane| {
                    control.spatial_left[oscillator][index + lane]
                })),
                f32x8::from(std::array::from_fn(|lane| {
                    control.spatial_right[oscillator][index + lane]
                })),
            )
        } else if control.spatial_mask & bit != 0 {
            (
                f32x8::from(std::array::from_fn(|lane| {
                    self.dynamic_unison_left[oscillator][index + lane]
                })),
                f32x8::from(std::array::from_fn(|lane| {
                    self.dynamic_unison_right[oscillator][index + lane]
                })),
            )
        } else if oscillator == 0 {
            (
                f32x8::from(std::array::from_fn(|lane| self.unison.left[index + lane])),
                f32x8::from(std::array::from_fn(|lane| self.unison.right[index + lane])),
            )
        } else {
            let layout = &self.secondary_unison[oscillator - 1];
            (
                f32x8::from(std::array::from_fn(|lane| layout.left[index + lane])),
                f32x8::from(std::array::from_fn(|lane| layout.right[index + lane])),
            )
        }
    }

    #[inline]
    fn unison_gains4<const DYNAMIC_UNISON: bool>(
        &self,
        oscillator: usize,
        index: usize,
        control: &UnisonFrameControl,
    ) -> (f32x4, f32x4) {
        if !DYNAMIC_UNISON {
            return if oscillator == 0 {
                (
                    f32x4::from(std::array::from_fn(|lane| self.unison.left[index + lane])),
                    f32x4::from(std::array::from_fn(|lane| self.unison.right[index + lane])),
                )
            } else {
                let layout = &self.secondary_unison[oscillator - 1];
                (
                    f32x4::from(std::array::from_fn(|lane| layout.left[index + lane])),
                    f32x4::from(std::array::from_fn(|lane| layout.right[index + lane])),
                )
            };
        }
        let bit = 1 << oscillator;
        if control.spatial_shared_mask & bit != 0 {
            (
                f32x4::from(std::array::from_fn(|lane| {
                    control.spatial_left[oscillator][index + lane]
                })),
                f32x4::from(std::array::from_fn(|lane| {
                    control.spatial_right[oscillator][index + lane]
                })),
            )
        } else if control.spatial_mask & bit != 0 {
            (
                f32x4::from(std::array::from_fn(|lane| {
                    self.dynamic_unison_left[oscillator][index + lane]
                })),
                f32x4::from(std::array::from_fn(|lane| {
                    self.dynamic_unison_right[oscillator][index + lane]
                })),
            )
        } else if oscillator == 0 {
            (
                f32x4::from(std::array::from_fn(|lane| self.unison.left[index + lane])),
                f32x4::from(std::array::from_fn(|lane| self.unison.right[index + lane])),
            )
        } else {
            let layout = &self.secondary_unison[oscillator - 1];
            (
                f32x4::from(std::array::from_fn(|lane| layout.left[index + lane])),
                f32x4::from(std::array::from_fn(|lane| layout.right[index + lane])),
            )
        }
    }

    #[inline]
    fn unison_layout_gain<const DYNAMIC_UNISON: bool>(
        &self,
        oscillator: usize,
        control: &UnisonFrameControl,
    ) -> f32 {
        if !DYNAMIC_UNISON {
            return if oscillator == 0 {
                self.unison.gain
            } else {
                self.secondary_unison[oscillator - 1].gain
            };
        }
        if control.spatial_shared_mask & (1 << oscillator) != 0 {
            control.spatial_gain[oscillator]
        } else if control.spatial_mask & (1 << oscillator) != 0 {
            self.dynamic_unison_gain[oscillator]
        } else if oscillator == 0 {
            self.unison.gain
        } else {
            self.secondary_unison[oscillator - 1].gain
        }
    }

    pub(super) fn set_swarm_clock(&mut self, time: f32) {
        self.swarm_clock = wrap_swarm_clock(time + self.swarm_clock_offset);
    }

    pub(super) fn set_secondary_swarm_clock(&mut self, oscillator: usize, time: f32) {
        self.secondary_swarm_clock[oscillator - 1] =
            wrap_swarm_clock(time + self.secondary_swarm_clock_offset[oscillator - 1]);
    }

    fn seed_swarm_clocks(&mut self, seed: u64) {
        self.swarm_clock_offset = unit_hash(seed ^ 0x5357_4152_4d5f_4e31) as f32 * 4_096.0;
        for secondary in 0..LEGACY_OSCILLATOR_COUNT - 1 {
            self.secondary_swarm_clock_offset[secondary] =
                unit_hash(seed.rotate_left((secondary as u32 + 1) * 19) ^ 0x5357_4152_4d5f_5343)
                    as f32
                    * 4_096.0;
        }
    }

    fn advance_unison_transitions(&mut self) {
        if self.enabled_oscillator_mask & 1 != 0 {
            self.phase_steps_dirty |= self.unison.advance_transition();
        }
        for secondary in 0..LEGACY_OSCILLATOR_COUNT - 1 {
            if self.enabled_oscillator_mask & (1 << (secondary + 1)) == 0 {
                continue;
            }
            self.secondary_phase_steps_dirty[secondary] |=
                self.secondary_unison[secondary].advance_transition();
        }
    }

    pub(super) fn unison_transitions_steady(&self) -> bool {
        (self.enabled_oscillator_mask & 1 == 0 || !self.unison.transition_active())
            && self
                .secondary_unison
                .iter()
                .enumerate()
                .all(|(secondary, layout)| {
                    self.enabled_oscillator_mask & (1 << (secondary + 1)) == 0
                        || !layout.transition_active()
                })
    }

    pub(super) fn set_enabled_oscillator_mask(&mut self, mask: OscillatorMask) {
        let mask = mask & LEGACY_OSCILLATOR_MASK;
        let newly_enabled = mask & !self.enabled_oscillator_mask;
        self.enabled_oscillator_mask = mask;
        if !self.active() || newly_enabled == 0 {
            return;
        }
        for oscillator in 0..LEGACY_OSCILLATOR_COUNT {
            if newly_enabled & (1 << oscillator) != 0 {
                self.seed_unison_layout(oscillator, self.note_seed);
                self.randomize_oscillator_bank(oscillator, self.note_seed);
                if oscillator == 0 {
                    self.phase_steps_dirty = true;
                    self.reset_swarm_motion();
                } else {
                    self.secondary_phase_steps_dirty[oscillator - 1] = true;
                    self.reset_secondary_swarm_motion(oscillator - 1);
                }
            }
        }
    }

    pub(super) fn set_enabled_filter_mask(&mut self, mask: u32) {
        let mut newly_enabled = mask & !self.enabled_filter_mask;
        self.enabled_filter_mask = mask;
        while newly_enabled != 0 {
            let slot = newly_enabled.trailing_zeros() as usize;
            newly_enabled &= newly_enabled - 1;
            self.filters[slot].reset();
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "the waveform-specialized SIMD banks share one phase and gain tail"
    )]
    pub fn render(
        &mut self,
        settings: VoiceSettings,
        sample_rate: f32,
        force_gate: bool,
    ) -> (f32, f32) {
        self.render_controlled::<false>(
            settings,
            sample_rate,
            force_gate,
            &UnisonFrameControl::NEUTRAL,
        )
    }

    pub(super) fn render_controlled<const DYNAMIC_UNISON: bool>(
        &mut self,
        settings: VoiceSettings,
        sample_rate: f32,
        force_gate: bool,
        unison_control: &UnisonFrameControl,
    ) -> (f32, f32) {
        self.render_controlled_frame::<DYNAMIC_UNISON>(
            settings,
            sample_rate,
            force_gate,
            unison_control,
        )
        .mixed()
    }

    pub(super) fn render_controlled_grouped<const DYNAMIC_UNISON: bool>(
        &mut self,
        settings: VoiceSettings,
        sample_rate: f32,
        unison_control: &UnisonFrameControl,
        stems: &mut [(f32, f32); MAX_OUTPUT_PAIRS],
        oscillator_groups: &[u8; MAX_OSCILLATORS],
        group_count: usize,
    ) {
        let frame = self.render_controlled_frame::<DYNAMIC_UNISON>(
            settings,
            sample_rate,
            false,
            unison_control,
        );
        let envelope_gains = self.group_envelope_gains();
        frame.accumulate_grouped(stems, oscillator_groups, group_count, &envelope_gains);
    }

    #[inline]
    fn advance_jitter_phase_steps8_pair(
        &mut self,
        index: usize,
        render_second: bool,
        first_frame_advanced: bool,
    ) -> [[f32; 8]; 2] {
        let current = std::array::from_fn(|lane| self.phase_steps[index + lane]);
        if !render_second {
            return [current, [0.0; 8]];
        }
        let current = f32x8::from(current);
        let step = f32x8::from(std::array::from_fn(|lane| {
            self.swarm_pitch_step[index + lane]
        }));
        let first = if first_frame_advanced {
            current
        } else {
            current + step
        };
        let second = first + step;
        let second_array: [f32; 8] = second.into();
        self.phase_steps[index..index + 8].copy_from_slice(&second_array);
        [first.into(), second_array]
    }

    #[inline]
    fn advance_jitter_phase_steps4_pair(
        &mut self,
        index: usize,
        render_second: bool,
        first_frame_advanced: bool,
    ) -> [[f32; 4]; 2] {
        let current = std::array::from_fn(|lane| self.phase_steps[index + lane]);
        if !render_second {
            return [current, [0.0; 4]];
        }
        let current = f32x4::from(current);
        let step = f32x4::from(std::array::from_fn(|lane| {
            self.swarm_pitch_step[index + lane]
        }));
        let first = if first_frame_advanced {
            current
        } else {
            current + step
        };
        let second = first + step;
        let second_array: [f32; 4] = second.into();
        self.phase_steps[index..index + 4].copy_from_slice(&second_array);
        [first.into(), second_array]
    }

    #[inline]
    fn advance_jitter_phase_steps_pair<const LANES: usize>(
        &mut self,
        index: usize,
        render_second: bool,
        first_frame_advanced: bool,
    ) -> [[f32; LANES]; 2] {
        let mut phase_steps = [[0.0; LANES]; 2];
        for lane in 0..LANES {
            let index = index + lane;
            if !render_second {
                phase_steps[0][lane] = self.phase_steps[index];
                continue;
            }
            let step = self.swarm_pitch_step[index];
            let first = if first_frame_advanced {
                self.phase_steps[index]
            } else {
                self.phase_steps[index] + step
            };
            phase_steps[0][lane] = first;
            phase_steps[1][lane] = first + step;
            self.phase_steps[index] = phase_steps[1][lane];
        }
        phase_steps
    }

    fn reset_oscillators(&mut self) {
        for bank in &mut self.oscillators {
            for oscillator in bank {
                oscillator.reset();
            }
        }
        self.oscillator_bank.reset();
    }

    pub(super) fn reset_filters(&mut self) {
        let mut mask = self.enabled_filter_mask;
        while mask != 0 {
            let slot = mask.trailing_zeros() as usize;
            mask &= mask - 1;
            self.filters[slot].reset();
        }
    }

    fn randomize_oscillators(&mut self, seed: u64) {
        self.reset_filters();
        for bank in 0..LEGACY_OSCILLATOR_COUNT {
            if self.enabled_oscillator_mask & (1 << bank) != 0 {
                self.randomize_oscillator_bank(bank, seed);
            }
        }
    }

    pub(super) fn seed_oscillator_bank(&mut self, settings: &ActiveOscillatorRenderSet) {
        self.oscillator_bank.seed_all(self.note_seed, settings);
    }

    fn randomize_oscillator_bank(&mut self, bank: usize, seed: u64) {
        let settings = if bank == 0 {
            self.unison.settings
        } else {
            self.secondary_unison[bank - 1].settings
        };
        let amount = f64::from(settings.phase_random);
        let position = f64::from(settings.phase_position);
        let bank_seed = seed ^ (bank as u64).wrapping_mul(0x4f53_435f_4241_4e4b);
        for (index, oscillator) in self.oscillators[bank].iter_mut().enumerate() {
            let lane_seed =
                bank_seed.wrapping_add((index as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15));
            oscillator.set_phase(
                (position + unit_hash(lane_seed).mul_add(2.0, -1.0) * amount).rem_euclid(1.0),
            );
            oscillator.restart_rich_timeline(unit_hash(lane_seed ^ 0x5249_4348_5f54_494d) as f32);
        }
    }

    fn seed_enabled_unison_layouts(&mut self, seed: u64) {
        self.dynamic_spatial_valid = 0;
        for oscillator in 0..LEGACY_OSCILLATOR_COUNT {
            if self.enabled_oscillator_mask & (1 << oscillator) != 0 {
                self.seed_unison_layout(oscillator, seed);
            }
        }
    }

    fn seed_unison_layout(&mut self, oscillator: usize, seed: u64) {
        let seed = oscillator_stereo_seed(seed, oscillator);
        if oscillator == 0 {
            self.unison.set_random_seed(seed);
            self.unison.settle();
        } else {
            let unison = &mut self.secondary_unison[oscillator - 1];
            unison.set_random_seed(seed);
            unison.settle();
        }
    }

    pub(super) fn set_pitch_bend(&mut self, semitones: f32) {
        let pitch_ratio = (semitones / 12.0).exp2();
        let scale = pitch_ratio / self.pitch_ratio;
        if self.unison.settings.motion_active() && !self.phase_steps_dirty {
            self.scale_primary_phase_steps(scale);
        } else {
            self.phase_steps_dirty = true;
        }
        for secondary in 0..LEGACY_OSCILLATOR_COUNT - 1 {
            if self.enabled_oscillator_mask & (1 << (secondary + 1)) == 0 {
                continue;
            }
            if self.secondary_unison[secondary].settings.motion_active()
                && !self.secondary_phase_steps_dirty[secondary]
            {
                self.scale_secondary_phase_steps(secondary, scale);
            } else {
                self.secondary_phase_steps_dirty[secondary] = true;
            }
        }
        self.pitch_ratio = pitch_ratio;
    }

    fn advance_glide(&mut self) {
        if self.glide_remaining == 0 {
            return;
        }
        let mut scale = self.glide_multiplier;
        self.frequency_hz *= scale;
        self.glide_remaining -= 1;
        if self.glide_remaining == 0 {
            let correction = self.glide_target_hz / self.frequency_hz;
            self.frequency_hz = self.glide_target_hz;
            self.glide_multiplier = 1.0;
            scale *= correction;
        }
        self.scale_cached_phase_steps(scale);
    }

    fn scale_cached_phase_steps(&mut self, scale: f32) {
        if self.enabled_oscillator_mask & 1 != 0 {
            self.scale_primary_phase_steps(scale);
        }
        for secondary in 0..LEGACY_OSCILLATOR_COUNT - 1 {
            if self.enabled_oscillator_mask & (1 << (secondary + 1)) != 0 {
                self.scale_secondary_phase_steps(secondary, scale);
            }
        }
    }

    fn scale_primary_phase_steps(&mut self, scale: f32) {
        for index in 0..usize::from(self.unison.render_voices) {
            self.phase_steps[index] = (self.phase_steps[index] * scale).min(0.45);
            self.swarm_pitch_step[index] *= scale;
        }
    }

    fn scale_secondary_phase_steps(&mut self, secondary: usize, scale: f32) {
        for index in 0..usize::from(self.secondary_unison[secondary].render_voices) {
            self.secondary_phase_steps[secondary][index] =
                (self.secondary_phase_steps[secondary][index] * scale).min(0.45);
            self.secondary_swarm_pitch_step[secondary][index] *= scale;
        }
    }

    fn refresh_phase_steps(&mut self) {
        let base_phase_step =
            self.base_phase_step(self.frequency_hz * self.pitch_ratio, self.sample_rate);
        for index in 0..usize::from(self.unison.render_voices) {
            self.phase_steps[index] = base_phase_step * self.unison.ratios[index];
        }
        self.phase_steps_dirty = false;
    }

    fn refresh_secondary_phase_steps(&mut self, secondary: usize) {
        let base_phase_step = self.secondary_base_phase_step(
            secondary,
            self.frequency_hz * self.pitch_ratio,
            self.sample_rate,
        );
        for index in 0..usize::from(self.secondary_unison[secondary].render_voices) {
            self.secondary_phase_steps[secondary][index] =
                base_phase_step * self.secondary_unison[secondary].ratios[index];
        }
        self.secondary_phase_steps_dirty[secondary] = false;
    }

    #[inline]
    fn lane_phase_step(&self, index: usize, dynamic_base_step: Option<f32>) -> f32 {
        dynamic_base_step
            .map_or(self.phase_steps[index], |base_step| {
                base_step * self.unison.ratios[index]
            })
            .min(0.45)
    }

    #[inline]
    fn oscillator_phase_step<const DYNAMIC_UNISON: bool>(
        &self,
        index: usize,
        pitch_ratio: f32,
        dynamic_base_step: Option<f32>,
        unison_control: &UnisonFrameControl,
    ) -> f32 {
        let phase_step = self.lane_phase_step(index, dynamic_base_step) * pitch_ratio;
        if !DYNAMIC_UNISON {
            return phase_step.min(0.45);
        }
        if unison_control.active_mask & 1 == 0 {
            phase_step.min(0.45)
        } else {
            (phase_step * unison_control.pitch_correction[0][index]).min(0.45)
        }
    }

    #[inline]
    fn secondary_oscillator_phase_step(
        &self,
        secondary: usize,
        index: usize,
        pitch_ratio: f32,
        dynamic_base_step: Option<f32>,
    ) -> f32 {
        let phase_step =
            dynamic_base_step.map_or(self.secondary_phase_steps[secondary][index], |_| {
                self.secondary_base_phase_step(
                    secondary,
                    self.frequency_hz * self.pitch_ratio,
                    self.sample_rate,
                ) * self.secondary_unison[secondary].ratios[index]
            });
        (phase_step * pitch_ratio).min(0.45)
    }

    #[inline]
    fn controlled_secondary_phase_step<const DYNAMIC_UNISON: bool>(
        &self,
        secondary: usize,
        index: usize,
        pitch_ratio: f32,
        dynamic_base_step: Option<f32>,
        unison_control: &UnisonFrameControl,
        oscillator_index: usize,
    ) -> f32 {
        let phase_step =
            self.secondary_oscillator_phase_step(secondary, index, pitch_ratio, dynamic_base_step);
        if !DYNAMIC_UNISON {
            return phase_step;
        }
        if unison_control.active_mask & (1 << oscillator_index) == 0 {
            phase_step
        } else {
            (phase_step * unison_control.pitch_correction[oscillator_index][index]).min(0.45)
        }
    }

    fn render_secondary_oscillator<const DYNAMIC_UNISON: bool>(
        &mut self,
        settings: VoiceSettings,
        oscillator_index: usize,
        dynamic_base_step: Option<f32>,
        unison_control: &UnisonFrameControl,
    ) -> (f32, f32) {
        let oscillator = settings.oscillator(oscillator_index);
        if !oscillator.enabled {
            return (0.0, 0.0);
        }
        let secondary = oscillator_index - 1;
        if self.secondary_phase_steps_dirty[secondary] {
            self.refresh_secondary_phase_steps(secondary);
        }
        if self.secondary_unison[secondary].settings.motion_active() {
            self.advance_secondary_swarm(secondary);
        }
        let shape = self.effective_oscillator_shape(settings, oscillator_index);
        let (left_gain, right_gain) = oscillator.channel_gains();
        let voice_count = usize::from(self.secondary_unison[secondary].render_voices);
        let mut index = 0;
        let mut left8 = f32x8::ZERO;
        let mut right8 = f32x8::ZERO;
        while index + 8 <= voice_count {
            let phase_steps = std::array::from_fn(|lane| {
                self.controlled_secondary_phase_step::<DYNAMIC_UNISON>(
                    secondary,
                    index + lane,
                    oscillator.pitch_ratio,
                    dynamic_base_step,
                    unison_control,
                    oscillator_index,
                )
            });
            let oscillators = &mut self.oscillators[oscillator_index][index..index + 8];
            let samples = if oscillator.custom_active() {
                generate_custom8(
                    oscillators,
                    shape,
                    phase_steps,
                    oscillator.pulse_width,
                    settings.antialiasing,
                    oscillator.phase_warp.mode,
                    oscillator.phase_warp.amount,
                    oscillator.custom_curve,
                    oscillator.custom_mix,
                )
            } else if oscillator.phase_warp_active() {
                generate_shape8_warped(
                    oscillators,
                    shape,
                    phase_steps,
                    oscillator.pulse_width,
                    settings.antialiasing,
                    oscillator.phase_warp.mode,
                    oscillator.phase_warp.amount,
                )
            } else if shape <= f32::EPSILON {
                generate_sine8(oscillators, phase_steps)
            } else if (shape - 1.0).abs() <= f32::EPSILON {
                generate_triangle8(oscillators, phase_steps, settings.antialiasing)
            } else if (shape - 2.0).abs() <= f32::EPSILON {
                generate_saw8(oscillators, phase_steps, settings.antialiasing)
            } else if shape >= 3.0 - f32::EPSILON {
                generate_pulse8(
                    oscillators,
                    phase_steps,
                    oscillator.pulse_width,
                    settings.antialiasing,
                )
            } else {
                generate_shape8(
                    oscillators,
                    shape,
                    phase_steps,
                    oscillator.pulse_width,
                    settings.antialiasing,
                )
            };
            let (left_gains, right_gains) =
                self.unison_gains8::<DYNAMIC_UNISON>(oscillator_index, index, unison_control);
            left8 = samples.mul_add(left_gains, left8);
            right8 = samples.mul_add(right_gains, right8);
            index += 8;
        }
        let mut left = left8.reduce_add();
        let mut right = right8.reduce_add();
        let mut left4 = f32x4::ZERO;
        let mut right4 = f32x4::ZERO;
        while index + 4 <= voice_count {
            let phase_steps = std::array::from_fn(|lane| {
                self.controlled_secondary_phase_step::<DYNAMIC_UNISON>(
                    secondary,
                    index + lane,
                    oscillator.pitch_ratio,
                    dynamic_base_step,
                    unison_control,
                    oscillator_index,
                )
            });
            let oscillators = &mut self.oscillators[oscillator_index][index..index + 4];
            let samples = if oscillator.custom_active() {
                generate_custom4(
                    oscillators,
                    shape,
                    phase_steps,
                    oscillator.pulse_width,
                    settings.antialiasing,
                    oscillator.phase_warp.mode,
                    oscillator.phase_warp.amount,
                    oscillator.custom_curve,
                    oscillator.custom_mix,
                )
            } else if oscillator.phase_warp_active() {
                generate_shape4_warped(
                    oscillators,
                    shape,
                    phase_steps,
                    oscillator.pulse_width,
                    settings.antialiasing,
                    oscillator.phase_warp.mode,
                    oscillator.phase_warp.amount,
                )
            } else if shape <= f32::EPSILON {
                generate_sine4(oscillators, phase_steps)
            } else if (shape - 1.0).abs() <= f32::EPSILON {
                generate_triangle4(oscillators, phase_steps, settings.antialiasing)
            } else if (shape - 2.0).abs() <= f32::EPSILON {
                generate_saw4(oscillators, phase_steps, settings.antialiasing)
            } else if shape >= 3.0 - f32::EPSILON {
                generate_pulse4(
                    oscillators,
                    phase_steps,
                    oscillator.pulse_width,
                    settings.antialiasing,
                )
            } else {
                generate_shape4(
                    oscillators,
                    shape,
                    phase_steps,
                    oscillator.pulse_width,
                    settings.antialiasing,
                )
            };
            let (left_gains, right_gains) =
                self.unison_gains4::<DYNAMIC_UNISON>(oscillator_index, index, unison_control);
            left4 = samples.mul_add(left_gains, left4);
            right4 = samples.mul_add(right_gains, right4);
            index += 4;
        }
        left += left4.reduce_add();
        right += right4.reduce_add();
        while index < voice_count {
            let phase_step = self.controlled_secondary_phase_step::<DYNAMIC_UNISON>(
                secondary,
                index,
                oscillator.pitch_ratio,
                dynamic_base_step,
                unison_control,
                oscillator_index,
            );
            let sample = if oscillator.custom_active() {
                self.oscillators[oscillator_index][index].generate_custom_step(
                    shape,
                    phase_step,
                    oscillator.pulse_width,
                    settings.antialiasing,
                    oscillator.phase_warp.mode,
                    oscillator.phase_warp.amount,
                    oscillator.custom_curve,
                    oscillator.custom_mix,
                )
            } else if oscillator.phase_warp_active() {
                self.oscillators[oscillator_index][index].generate_shape_step_warped(
                    shape,
                    phase_step,
                    oscillator.pulse_width,
                    settings.antialiasing,
                    oscillator.phase_warp.mode,
                    oscillator.phase_warp.amount,
                )
            } else {
                self.oscillators[oscillator_index][index].generate_shape_step(
                    shape,
                    phase_step,
                    oscillator.pulse_width,
                    settings.antialiasing,
                )
            };
            left = sample.mul_add(
                self.unison_left_gain::<DYNAMIC_UNISON>(oscillator_index, index, unison_control),
                left,
            );
            right = sample.mul_add(
                self.unison_right_gain::<DYNAMIC_UNISON>(oscillator_index, index, unison_control),
                right,
            );
            index += 1;
        }
        let gain = self.unison_layout_gain::<DYNAMIC_UNISON>(oscillator_index, unison_control);
        (left * (left_gain * gain), right * (right_gain * gain))
    }

    fn base_phase_step(&self, frequency_hz: f32, sample_rate: f32) -> f32 {
        let highest_ratio = self.unison.ratios[..usize::from(self.unison.render_voices)]
            .iter()
            .copied()
            .fold(1.0_f32, f32::max);
        (frequency_hz.max(0.0) / sample_rate.max(1.0)).min(0.45 / highest_ratio)
    }

    fn secondary_base_phase_step(
        &self,
        secondary: usize,
        frequency_hz: f32,
        sample_rate: f32,
    ) -> f32 {
        let layout = &self.secondary_unison[secondary];
        let highest_ratio = layout.ratios[..usize::from(layout.render_voices)]
            .iter()
            .copied()
            .fold(1.0_f32, f32::max);
        (frequency_hz.max(0.0) / sample_rate.max(1.0)).min(0.45 / highest_ratio)
    }

    fn reset_swarm_motion(&mut self) {
        let interval = self.swarm_update_interval();
        self.swarm_update_remaining = 1 + (self.note_seed as u16 % interval);
        self.swarm_pitch_step.fill(0.0);
    }

    fn reset_all_swarm_motion(&mut self) {
        self.reset_swarm_motion();
        for secondary in 0..LEGACY_OSCILLATOR_COUNT - 1 {
            self.reset_secondary_swarm_motion(secondary);
        }
    }

    fn reset_enabled_swarm_motion(&mut self) {
        if self.enabled_oscillator_mask & 1 != 0 {
            self.reset_swarm_motion();
        }
        for secondary in 0..LEGACY_OSCILLATOR_COUNT - 1 {
            if self.enabled_oscillator_mask & (1 << (secondary + 1)) != 0 {
                self.reset_secondary_swarm_motion(secondary);
            }
        }
    }

    fn reset_secondary_swarm_motion(&mut self, secondary: usize) {
        let interval = self.secondary_swarm_update_interval(secondary);
        let seed = self.note_seed.rotate_left((secondary as u32 + 1) * 17);
        self.secondary_swarm_update_remaining[secondary] = 1 + (seed as u16 % interval);
        self.secondary_swarm_pitch_step[secondary].fill(0.0);
    }

    #[cold]
    #[inline(never)]
    fn prepare_swarm_jitter_target(&mut self, update_interval: u16) {
        let settings = self.unison.settings;
        let interval = f32::from(update_interval);
        let base_phase_step =
            self.base_phase_step(self.frequency_hz * self.pitch_ratio, self.sample_rate);
        let voices = usize::from(settings.voices);
        let target_clock =
            wrap_swarm_clock(self.swarm_clock + interval * settings.swarm_rate / self.sample_rate);
        fill_unison_jitter_offsets_mode(
            &mut self.swarm_pitch_step[..voices],
            self.unison.random_seed,
            settings.swarm_amount,
            target_clock,
            settings.swarm_mode,
        );
        let mut ratios = [0.0; MAX_UNISON];
        jitter_pitch_ratios(
            &mut ratios[..voices],
            &mut self.swarm_pitch_step[..voices],
            self.unison.swarm_depth_cents,
        );
        for index in 0..voices {
            let target = (base_phase_step * self.unison.ratios[index] * ratios[index]).min(0.45);
            self.swarm_pitch_step[index] = (target - self.phase_steps[index]) / interval;
        }
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the clamped positive control interval fits in u16"
    )]
    fn swarm_update_interval(&self) -> u16 {
        let settings = self.unison.settings;
        let update_rate = settings.swarm_rate.max(0.02)
            * if settings.swarm_mode == SwarmMode::Sine {
                8.0
            } else {
                1.0
            };
        (self.sample_rate / update_rate).round().clamp(
            f32::from(SWARM_MIN_UPDATE_INTERVAL),
            f32::from(SWARM_MAX_UPDATE_INTERVAL),
        ) as u16
    }

    #[inline(never)]
    fn advance_swarm(&mut self) {
        debug_assert!(self.unison.settings.motion_active());
        if self.swarm_update_remaining == 0 {
            let update_interval = self.swarm_update_interval();
            self.prepare_swarm_jitter_target(update_interval);
            self.swarm_update_remaining = update_interval;
        }
        let voices = usize::from(self.unison.settings.voices);
        for index in 0..voices {
            self.phase_steps[index] =
                (self.phase_steps[index] + self.swarm_pitch_step[index]).min(0.45);
        }
        self.swarm_update_remaining -= 1;
    }

    #[cold]
    #[inline(never)]
    fn prepare_secondary_swarm_jitter_target(&mut self, secondary: usize, update_interval: u16) {
        let settings = self.secondary_unison[secondary].settings;
        let interval = f32::from(update_interval);
        let base_phase_step = self.secondary_base_phase_step(
            secondary,
            self.frequency_hz * self.pitch_ratio,
            self.sample_rate,
        );
        let voices = usize::from(settings.voices);
        let target_clock = wrap_swarm_clock(
            self.secondary_swarm_clock[secondary]
                + interval * settings.swarm_rate / self.sample_rate,
        );
        fill_unison_jitter_offsets_mode(
            &mut self.secondary_swarm_pitch_step[secondary][..voices],
            self.secondary_unison[secondary].random_seed,
            settings.swarm_amount,
            target_clock,
            settings.swarm_mode,
        );
        let mut ratios = [0.0; MAX_UNISON];
        jitter_pitch_ratios(
            &mut ratios[..voices],
            &mut self.secondary_swarm_pitch_step[secondary][..voices],
            self.secondary_unison[secondary].swarm_depth_cents,
        );
        for index in 0..voices {
            let target =
                (base_phase_step * self.secondary_unison[secondary].ratios[index] * ratios[index])
                    .min(0.45);
            self.secondary_swarm_pitch_step[secondary][index] =
                (target - self.secondary_phase_steps[secondary][index]) / interval;
        }
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the clamped positive control interval fits in u16"
    )]
    fn secondary_swarm_update_interval(&self, secondary: usize) -> u16 {
        let settings = self.secondary_unison[secondary].settings;
        let update_rate = settings.swarm_rate.max(0.02)
            * if settings.swarm_mode == SwarmMode::Sine {
                8.0
            } else {
                1.0
            };
        (self.sample_rate / update_rate).round().clamp(
            f32::from(SWARM_MIN_UPDATE_INTERVAL),
            f32::from(SWARM_MAX_UPDATE_INTERVAL),
        ) as u16
    }

    #[inline(never)]
    fn advance_secondary_swarm(&mut self, secondary: usize) {
        debug_assert!(self.secondary_unison[secondary].settings.motion_active());
        if self.secondary_swarm_update_remaining[secondary] == 0 {
            let update_interval = self.secondary_swarm_update_interval(secondary);
            self.prepare_secondary_swarm_jitter_target(secondary, update_interval);
            self.secondary_swarm_update_remaining[secondary] = update_interval;
        }
        let voices = usize::from(self.secondary_unison[secondary].settings.voices);
        for index in 0..voices {
            self.secondary_phase_steps[secondary][index] = (self.secondary_phase_steps[secondary]
                [index]
                + self.secondary_swarm_pitch_step[secondary][index])
                .min(0.45);
        }
        self.secondary_swarm_update_remaining[secondary] -= 1;
    }

    #[inline(never)]
    fn prepare_swarm_pair(&mut self, clocks: [f32; 2]) -> bool {
        debug_assert!(self.unison.settings.motion_active());
        self.set_swarm_clock(clocks[0]);
        if self.swarm_update_remaining == 0 {
            let update_interval = self.swarm_update_interval();
            self.prepare_swarm_jitter_target(update_interval);
            self.swarm_update_remaining = update_interval;
        }
        if self.swarm_update_remaining == 1 {
            self.advance_swarm();
            self.set_swarm_clock(clocks[1]);
            let update_interval = self.swarm_update_interval();
            self.prepare_swarm_jitter_target(update_interval);
            self.swarm_update_remaining = update_interval - 1;
            return true;
        }
        self.swarm_update_remaining -= 2;
        false
    }

    fn advance_envelope(&mut self, sample_rate: f32, force_gate: bool) {
        debug_assert!((self.sample_rate - sample_rate.max(1.0)).abs() <= f32::EPSILON);
        if self.group_envelope_count != 0 && !force_gate {
            let mut active = false;
            for (group, envelope) in self.group_envelopes[..usize::from(self.group_envelope_count)]
                .iter_mut()
                .enumerate()
            {
                if self.group_active_mask & (1 << group) == 0 {
                    continue;
                }
                envelope.advance(sample_rate);
                active |= envelope.active();
            }
            if active {
                self.envelope_level = 1.0;
                self.stage = if self.held || self.sustained {
                    EnvelopeStage::Sustain
                } else {
                    EnvelopeStage::Release
                };
            } else {
                self.finish_envelope();
            }
            return;
        }
        match self.stage {
            EnvelopeStage::Idle => self.envelope_level = 0.0,
            EnvelopeStage::Attack => {
                self.advance_envelope_progress();
                self.envelope_level = shaped_progress(
                    self.envelope_progress,
                    self.envelope.attack_curve_time,
                    self.envelope.attack_curve,
                )
                .mul_add(1.0 - self.envelope_start, self.envelope_start);
                if self.envelope_progress >= 1.0 {
                    self.envelope_level = 1.0;
                    self.begin_decay();
                }
            }
            EnvelopeStage::Decay => {
                let sustain = self.envelope.sustain.clamp(0.0, 1.0);
                if sustain >= self.envelope_start {
                    self.envelope_level = sustain;
                    self.stage = EnvelopeStage::Sustain;
                } else {
                    self.advance_envelope_progress();
                    self.envelope_level = shaped_progress(
                        self.envelope_progress,
                        self.envelope.decay_curve_time,
                        self.envelope.decay_curve,
                    )
                    .mul_add(sustain - self.envelope_start, self.envelope_start);
                    if self.envelope_progress >= 1.0 {
                        self.envelope_level = sustain;
                        self.stage = EnvelopeStage::Sustain;
                    }
                }
            }
            EnvelopeStage::Sustain => {
                self.envelope_level = self.envelope.sustain.clamp(0.0, 1.0);
            }
            EnvelopeStage::Release => {
                self.advance_envelope_progress();
                self.envelope_level =
                    (1.0 - shaped_progress(
                        self.envelope_progress,
                        self.envelope.release_curve_time,
                        self.envelope.release_curve,
                    )) * self.envelope_start;
                if self.envelope_progress >= 1.0 || self.envelope_level <= 1.0e-5 {
                    self.finish_envelope();
                }
            }
        }
        if force_gate && self.stage == EnvelopeStage::Release {
            self.begin_attack();
        }
    }

    pub(super) fn release(&mut self, immediate: bool, sample_rate: f32) {
        self.held = false;
        if self.sustained && !immediate {
            return;
        }
        if self.group_envelope_count != 0 {
            if immediate {
                self.finish_envelope();
                return;
            }
            let mut active = false;
            for (group, envelope) in self.group_envelopes[..usize::from(self.group_envelope_count)]
                .iter_mut()
                .enumerate()
            {
                if self.group_active_mask & (1 << group) == 0 {
                    continue;
                }
                envelope.note_off(sample_rate);
                active |= envelope.active();
            }
            if active {
                self.envelope_level = 1.0;
                self.stage = EnvelopeStage::Release;
            } else {
                self.finish_envelope();
            }
            return;
        }
        if immediate {
            self.finish_envelope();
        } else if self.stage != EnvelopeStage::Idle {
            debug_assert!((self.sample_rate - sample_rate.max(1.0)).abs() <= f32::EPSILON);
            if self.envelope.release <= 0.0 {
                self.finish_envelope();
            } else {
                self.begin_stage(EnvelopeStage::Release);
            }
        }
    }

    fn begin_attack(&mut self) {
        if self.envelope.attack <= 0.0 {
            self.envelope_level = 1.0;
            self.begin_decay();
            return;
        }
        self.begin_stage(EnvelopeStage::Attack);
        let remaining = (1.0 - self.envelope_start).max(f32::EPSILON);
        self.envelope_step = 1.0 / (self.envelope.attack * self.sample_rate * remaining).max(1.0);
    }

    fn begin_decay(&mut self) {
        let sustain = self.envelope.sustain.clamp(0.0, 1.0);
        if sustain >= self.envelope_level || self.envelope.decay <= 0.0 {
            self.envelope_level = sustain;
            self.stage = EnvelopeStage::Sustain;
        } else {
            self.begin_stage(EnvelopeStage::Decay);
        }
    }

    fn begin_stage(&mut self, stage: EnvelopeStage) {
        self.stage = stage;
        self.envelope_start = self.envelope_level;
        self.envelope_progress = 0.0;
        self.refresh_envelope_step();
    }

    fn refresh_envelope_step(&mut self) {
        let seconds = match self.stage {
            EnvelopeStage::Attack => self.envelope.attack.max(f32::EPSILON),
            EnvelopeStage::Decay => self.envelope.decay.max(f32::EPSILON),
            EnvelopeStage::Release => self.envelope.release.max(f32::EPSILON),
            EnvelopeStage::Idle | EnvelopeStage::Sustain => {
                self.envelope_step = 1.0;
                return;
            }
        };
        self.envelope_step = 1.0 / (seconds * self.sample_rate).max(1.0);
    }

    #[inline]
    fn advance_envelope_progress(&mut self) {
        self.envelope_progress = (self.envelope_progress + self.envelope_step).min(1.0);
    }

    fn finish_envelope(&mut self) {
        self.envelope_level = 0.0;
        self.envelope_start = 0.0;
        self.envelope_progress = 0.0;
        self.stage = EnvelopeStage::Idle;
        self.current_note = None;
        self.voice_id = None;
        self.glide_remaining = 0;
        self.glide_multiplier = 1.0;
        for envelope in &mut self.group_envelopes[..usize::from(self.group_envelope_count)] {
            envelope.finish();
        }
    }

    pub(super) fn matches(&self, note: u8, channel: u8, voice_id: Option<i32>) -> bool {
        self.current_note == Some(note)
            && self.channel == channel
            && voice_id.is_none_or(|id| self.voice_id == Some(id))
    }

    pub(super) fn active(&self) -> bool {
        self.stage != EnvelopeStage::Idle
    }

    fn effective_shape(&self, settings: VoiceSettings) -> f32 {
        self.effective_oscillator_shape(settings, 0)
    }

    fn effective_oscillator_shape(&self, settings: VoiceSettings, oscillator: usize) -> f32 {
        self.effective_oscillator_shape_value(
            settings,
            oscillator,
            settings.oscillator(oscillator).shape,
        )
    }

    fn effective_oscillator_shape_value(
        &self,
        settings: VoiceSettings,
        oscillator: usize,
        shape: f32,
    ) -> f32 {
        if settings.oscillator(oscillator).positioned_wave {
            shape
        } else {
            ((self.timbre - 0.5) * 2.0)
                .mul_add(settings.timbre_amount.clamp(0.0, 1.0), shape)
                .clamp(0.0, 3.0)
        }
    }

    fn oscillator_timbre(oscillator: &OscillatorDspSettings, timbre: f32) -> f32 {
        if oscillator.positioned_wave {
            0.0
        } else {
            timbre
        }
    }

    pub(super) fn render_oscillator_bank(
        &mut self,
        active: &ActiveOscillatorRenderSet,
        settings: VoiceSettings,
        sample_rate: f32,
        structural_control: &StructuralOscillatorFrameControl,
    ) -> (f32, f32) {
        if !active.active() || self.amplitude_level() <= f32::EPSILON {
            return (0.0, 0.0);
        }
        let velocity_gain = settings
            .velocity_amount
            .clamp(0.0, 1.0)
            .mul_add(self.velocity - 1.0, 1.0);
        let pressure_gain = settings
            .pressure_amount
            .clamp(0.0, 1.0)
            .mul_add(self.pressure, 1.0);
        let amplitude = self.amplitude_level() * velocity_gain * pressure_gain;
        let base_step = (self.frequency_hz * self.pitch_ratio / sample_rate.max(1.0)).min(0.45);
        let timbre = (self.timbre - 0.5) * 2.0 * settings.timbre_amount.clamp(0.0, 1.0);
        let mut left = 0.0;
        let mut right = 0.0;
        for entry in active.entries() {
            let slot = usize::from(entry.slot);
            let oscillator = &entry.current;
            let absolute = structural_control.get(slot);
            let timbre = Self::oscillator_timbre(oscillator, timbre);
            let shape = (absolute.map_or(oscillator.shape, |control| control.shape) + timbre)
                .clamp(0.0, 3.0);
            self.accumulate_structural_oscillator(
                slot,
                slot,
                oscillator,
                absolute,
                settings,
                sample_rate,
                base_step,
                shape,
                &mut left,
                &mut right,
            );
        }
        (left * amplitude, right * amplitude)
    }

    pub(super) fn render_oscillator_bank_grouped(
        &mut self,
        active: &ActiveOscillatorRenderSet,
        settings: VoiceSettings,
        sample_rate: f32,
        structural_control: &StructuralOscillatorFrameControl,
        stems: &mut [(f32, f32); MAX_OUTPUT_PAIRS],
        oscillator_groups: &[u8; MAX_OSCILLATORS],
        group_count: usize,
    ) {
        if !active.active() || self.envelope_level <= f32::EPSILON {
            return;
        }
        let velocity_gain = settings
            .velocity_amount
            .clamp(0.0, 1.0)
            .mul_add(self.velocity - 1.0, 1.0);
        let pressure_gain = settings
            .pressure_amount
            .clamp(0.0, 1.0)
            .mul_add(self.pressure, 1.0);
        let amplitude = self.envelope_level * velocity_gain * pressure_gain;
        let base_step = (self.frequency_hz * self.pitch_ratio / sample_rate.max(1.0)).min(0.45);
        let timbre = (self.timbre - 0.5) * 2.0 * settings.timbre_amount.clamp(0.0, 1.0);
        let mut grouped = [(0.0, 0.0); MAX_OUTPUT_PAIRS];
        for entry in active.entries() {
            let slot = usize::from(entry.slot);
            let oscillator = &entry.current;
            let absolute = structural_control.get(slot);
            let timbre = Self::oscillator_timbre(oscillator, timbre);
            let shape = (absolute.map_or(oscillator.shape, |control| control.shape) + timbre)
                .clamp(0.0, 3.0);
            let group = oscillator_group(oscillator_groups, group_count, slot);
            let (left, right) = &mut grouped[group];
            self.accumulate_structural_oscillator(
                slot,
                slot,
                oscillator,
                absolute,
                settings,
                sample_rate,
                base_step,
                shape,
                left,
                right,
            );
        }
        for group in 0..group_count {
            let gain = amplitude * self.group_envelopes[group].level;
            stems[group].0 += grouped[group].0 * gain;
            stems[group].1 += grouped[group].1 * gain;
        }
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the ordered generator program keeps its fixed render context allocation-free"
    )]
    pub(super) fn render_ordered_oscillator_groups(
        &mut self,
        active: &ActiveOscillatorRenderSet,
        settings: VoiceSettings,
        sample_rate: f32,
        structural_control: &StructuralOscillatorFrameControl,
        stems: &mut [(f32, f32); MAX_OUTPUT_PAIRS],
        groups: &[GeneratorRtGroup],
        group_count: usize,
        filters: &[FilterCoefficients; MAX_FILTERS],
    ) {
        if !active.active() || self.envelope_level <= f32::EPSILON {
            return;
        }
        let velocity_gain = settings
            .velocity_amount
            .clamp(0.0, 1.0)
            .mul_add(self.velocity - 1.0, 1.0);
        let pressure_gain = settings
            .pressure_amount
            .clamp(0.0, 1.0)
            .mul_add(self.pressure, 1.0);
        let amplitude = self.envelope_level * velocity_gain * pressure_gain;
        let base_step = (self.frequency_hz * self.pitch_ratio / sample_rate.max(1.0)).min(0.45);
        let timbre = (self.timbre - 0.5) * 2.0 * settings.timbre_amount.clamp(0.0, 1.0);

        for (group_index, group) in groups
            .iter()
            .take(group_count.min(MAX_OUTPUT_PAIRS))
            .enumerate()
        {
            if group.oscillator_mask() == 0 {
                continue;
            }
            let mut left = 0.0;
            let mut right = 0.0;
            for module in group.modules() {
                match *module {
                    GeneratorRtModule::Oscillator(slot) => {
                        let slot = slot.index();
                        if active.mask & (1 << slot) == 0 {
                            continue;
                        }
                        let oscillator = &active.entry(slot).current;
                        let absolute = structural_control.get(slot);
                        let timbre = Self::oscillator_timbre(oscillator, timbre);
                        let shape = (absolute.map_or(oscillator.shape, |control| control.shape)
                            + timbre)
                            .clamp(0.0, 3.0);
                        self.accumulate_structural_oscillator(
                            slot,
                            slot,
                            oscillator,
                            absolute,
                            settings,
                            sample_rate,
                            base_step,
                            shape,
                            &mut left,
                            &mut right,
                        );
                    }
                    GeneratorRtModule::Filter(slot) => {
                        let slot = slot.index();
                        (left, right) = self.filters[slot].process(filters[slot], left, right);
                    }
                }
            }
            let envelope_gain = if self.group_envelope_count == 0 {
                1.0
            } else {
                self.group_envelopes[group_index].level
            };
            let gain = amplitude * envelope_gain;
            stems[group_index].0 += left * gain;
            stems[group_index].1 += right * gain;
        }
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the structural oscillator keeps its fixed render context allocation-free"
    )]
    pub(super) fn write_resynth_telemetry(
        &self,
        state_index: usize,
        source_len: usize,
        expected_generation: u64,
        output: &mut crate::resynth_state::ResynthTelemetrySnapshot,
    ) -> bool {
        if !self.active() || state_index >= self.oscillator_bank.oscillators.len() {
            return false;
        }
        let oscillator = &self.oscillator_bank.oscillators[state_index][0];
        output.grain_positions.fill(0.0);
        output.grain_progress.fill(0.0);
        output.grain_gains.fill(0.0);
        output
            .grain_lanes
            .fill(crate::resynth_state::GrainTelemetryLane::default());
        output.active = true;
        output.phase = oscillator
            .rich_timeline_for_generation(expected_generation)
            .unwrap_or_else(|| oscillator.phase())
            .clamp(0.0, 1.0);
        output.envelope_proxy = self.envelope_level.clamp(0.0, 1.0);
        output.amplitude = output.envelope_proxy;
        let rich_remaining = oscillator.resynth_zone_fade_remaining();
        output.rich_from_zone = u16::from(if rich_remaining == 0 {
            oscillator.resynth_zone()
        } else {
            oscillator.resynth_zone_from()
        });
        output.rich_to_zone = u16::from(oscillator.resynth_zone());
        output.rich_transition_progress = if rich_remaining == 0 {
            1.0
        } else {
            f32::from(RICH_ZONE_HANDOVER_SAMPLES.saturating_sub(rich_remaining))
                / f32::from(RICH_ZONE_HANDOVER_SAMPLES)
        };
        output.rich_zone = output.rich_to_zone;
        output.zone = output.rich_zone;
        let generations = self.oscillator_bank.resynth_grain_generations[state_index];
        let scheduler = match (
            generations[0] == expected_generation,
            generations[1] == expected_generation,
        ) {
            (true, false) => Some(&self.oscillator_bank.resynth_grain[state_index][0]),
            (false, true) => Some(&self.oscillator_bank.resynth_grain[state_index][1]),
            (true, true) => {
                let first = &self.oscillator_bank.resynth_grain[state_index][0];
                let second = &self.oscillator_bank.resynth_grain[state_index][1];
                Some(if second.active_count() > first.active_count() {
                    second
                } else {
                    first
                })
            }
            (false, false) => None,
        };
        let Some(scheduler) = scheduler else {
            return true;
        };
        let mut pans = [0.0; crate::oscillators::GRAIN_TELEMETRY];
        let mut pitches = [0.0; crate::oscillators::GRAIN_TELEMETRY];
        let active_mask = scheduler.write_telemetry_ex(
            source_len,
            &mut output.grain_positions,
            &mut output.grain_progress,
            &mut output.grain_gains,
            Some(&mut pans),
            Some(&mut pitches),
        );
        // Preserve physical scheduler-lane identity. Active layers can leave
        // holes after their independent lifetimes expire, so first-N inference
        // would fabricate activity on the wrong lanes.
        let positions = output.grain_positions;
        let progresses = output.grain_progress;
        let gains = output.grain_gains;
        for (index, lane) in output.grain_lanes.iter_mut().enumerate() {
            lane.active = active_mask & (1_u8 << index) != 0;
            lane.position = positions[index];
            lane.progress = progresses[index];
            lane.gain = gains[index];
            lane.phase = progresses[index];
            lane.pan = pans[index];
            lane.pitch = pitches[index];
        }
        true
    }

    pub(super) fn resynth_timeline_phase(&self, state_index: usize, duration_frames: f64) -> f32 {
        let frame = self
            .oscillator_bank
            .resynth_frame
            .get(state_index)
            .copied()
            .unwrap_or(0);
        (frame as f64 / duration_frames.max(1.0)).rem_euclid(1.0) as f32
    }

    fn accumulate_structural_oscillator(
        &mut self,
        state_index: usize,
        slot: usize,
        oscillator: &OscillatorDspSettings,
        absolute: Option<&StructuralOscillatorAbsoluteControl>,
        settings: VoiceSettings,
        sample_rate: f32,
        base_step: f32,
        shape: f32,
        left: &mut f32,
        right: &mut f32,
    ) {
        let before_left = *left;
        let before_right = *right;
        let grain_single_lane = grain_uses_single_oscillator_lane(oscillator);
        let render_voices = if grain_single_lane {
            1
        } else {
            oscillator.render_voices
        };
        let phase_position =
            absolute.map_or(oscillator.phase_position, |control| control.phase_position);
        let phase_delta = shortest_phase_delta(
            self.oscillator_bank.applied_phase_positions[state_index],
            phase_position,
        );
        if phase_delta != 0.0 {
            for lane in
                &mut self.oscillator_bank.oscillators[state_index][..usize::from(render_voices)]
            {
                lane.offset_phase(phase_delta);
            }
            self.oscillator_bank.applied_phase_positions[state_index] = phase_position;
        }
        let pulse_width = absolute.map_or(oscillator.pulse_width, |control| control.pulse_width);
        let pitch_ratio = absolute.map_or(oscillator.pitch_ratio, |control| control.pitch_ratio);
        let phase_warp_amount = absolute.map_or(oscillator.phase_warp.amount, |control| {
            control.phase_warp_amount
        });
        let left_gain = absolute.map_or(oscillator.left_gain, |control| control.left_gain);
        let right_gain = absolute.map_or(oscillator.right_gain, |control| control.right_gain);
        let spatial_gains = absolute
            .filter(|control| control.stereo_x != 0.0 || control.stereo_y != 0.0)
            .map(|control| {
                let spatial = oscillator.spatial_settings.modulated(UnisonModulation {
                    stereo_x: control.stereo_x,
                    stereo_y: control.stereo_y,
                    ..UnisonModulation::default()
                });
                let mut detune_positions = [0.0; MAX_UNISON];
                let mut left = [0.0; MAX_UNISON];
                let mut right = [0.0; MAX_UNISON];
                fill_oscillator_unison_layout(
                    spatial,
                    &mut detune_positions,
                    &mut left,
                    &mut right,
                );
                (left, right)
            });
        let lane_left_gains = spatial_gains
            .as_ref()
            .map_or(&oscillator.lane_left_gains, |(left, _)| left);
        let lane_right_gains = spatial_gains
            .as_ref()
            .map_or(&oscillator.lane_right_gains, |(_, right)| right);
        let mut jitter_settings = *oscillator;
        if let Some(control) = absolute {
            jitter_settings.unison_jitter = control.unison_jitter;
            jitter_settings.jitter_rate_hz = extended_unison_rate(control.unison_rate);
        }
        let grain_frame = self.oscillator_bank.resynth_frame[state_index];
        if grain_single_lane || render_voices == 1 && !jitter_settings.jitter_active() {
            self.oscillator_bank.jitter_ratios[state_index][0] = 1.0;
            self.oscillator_bank.jitter_steps[state_index][0] = 0.0;
            self.oscillator_bank.jitter_remaining[state_index] = 0;
            let sample = if oscillator.engine == OscillatorEngineKind::Resynth {
                let (grain_left, grain_right) = generate_resynth_step_modulated(
                    &mut self.oscillator_bank.oscillators[state_index][0],
                    oscillator,
                    &mut self.oscillator_bank.resynth_grain[state_index],
                    &mut self.oscillator_bank.resynth_grain_generations[state_index],
                    (base_step * pitch_ratio).min(0.45) * sample_rate,
                    sample_rate,
                    self.note_seed,
                    0,
                    grain_frame,
                    absolute,
                );
                *left += grain_left * left_gain;
                *right += grain_right * right_gain;
                apply_resynth_bus_mix(
                    oscillator,
                    &mut self.oscillator_bank.resynth_source[state_index],
                    grain_frame,
                    sample_rate,
                    left_gain,
                    right_gain,
                    before_left,
                    before_right,
                    left,
                    right,
                );
                self.oscillator_bank.resynth_frame[state_index] = grain_frame.wrapping_add(1);
                return;
            } else if oscillator.custom_mix > f32::EPSILON {
                self.oscillator_bank.oscillators[state_index][0].generate_custom_step(
                    shape,
                    (base_step * pitch_ratio).min(0.45),
                    pulse_width,
                    settings.antialiasing,
                    oscillator.phase_warp.mode,
                    phase_warp_amount,
                    oscillator.custom_curve,
                    oscillator.custom_mix,
                )
            } else if oscillator.phase_warp.mode != PhaseWarpMode::None
                && phase_warp_amount > f32::EPSILON
            {
                self.oscillator_bank.oscillators[state_index][0].generate_shape_step_warped(
                    shape,
                    (base_step * pitch_ratio).min(0.45),
                    pulse_width,
                    settings.antialiasing,
                    oscillator.phase_warp.mode,
                    phase_warp_amount,
                )
            } else {
                self.oscillator_bank.oscillators[state_index][0].generate_shape_step(
                    shape,
                    (base_step * pitch_ratio).min(0.45),
                    pulse_width,
                    settings.antialiasing,
                )
            };
            *left += sample * left_gain;
            *right += sample * right_gain;
            apply_resynth_bus_mix(
                oscillator,
                &mut self.oscillator_bank.resynth_source[state_index],
                grain_frame,
                sample_rate,
                left_gain,
                right_gain,
                before_left,
                before_right,
                left,
                right,
            );
            if oscillator.engine == OscillatorEngineKind::Resynth {
                self.oscillator_bank.resynth_frame[state_index] = grain_frame.wrapping_add(1);
            }
            return;
        }
        self.advance_structural_jitter(state_index, slot, &jitter_settings, sample_rate);
        let voices = usize::from(render_voices);
        let oscillator_step = base_step * pitch_ratio;
        let mut lane = 0;
        while lane + 8 <= voices {
            let phase_steps = std::array::from_fn(|offset| {
                let index = lane + offset;
                self.oscillator_bank.jitter_ratios[state_index][index] +=
                    self.oscillator_bank.jitter_steps[state_index][index];
                (oscillator_step
                    * oscillator.lane_pitch_ratios[index]
                    * self.oscillator_bank.jitter_ratios[state_index][index])
                    .min(0.45)
            });
            let oscillators = &mut self.oscillator_bank.oscillators[state_index][lane..lane + 8];
            let samples: [f32; 8] = if oscillator.engine == OscillatorEngineKind::Resynth {
                f32x8::from(std::array::from_fn(|offset| {
                    let (left, right) = generate_resynth_step_modulated(
                        &mut oscillators[offset],
                        oscillator,
                        &mut self.oscillator_bank.resynth_grain[state_index],
                        &mut self.oscillator_bank.resynth_grain_generations[state_index],
                        phase_steps[offset] * sample_rate,
                        sample_rate,
                        self.note_seed,
                        lane + offset,
                        grain_frame,
                        absolute,
                    );
                    (left + right) * 0.5
                }))
            } else if oscillator.custom_mix > f32::EPSILON {
                generate_custom8(
                    oscillators,
                    shape,
                    phase_steps,
                    pulse_width,
                    settings.antialiasing,
                    oscillator.phase_warp.mode,
                    phase_warp_amount,
                    oscillator.custom_curve,
                    oscillator.custom_mix,
                )
            } else if oscillator.phase_warp.mode != PhaseWarpMode::None
                && phase_warp_amount > f32::EPSILON
            {
                generate_shape8_warped(
                    oscillators,
                    shape,
                    phase_steps,
                    pulse_width,
                    settings.antialiasing,
                    oscillator.phase_warp.mode,
                    phase_warp_amount,
                )
            } else {
                generate_shape8(
                    oscillators,
                    shape,
                    phase_steps,
                    pulse_width,
                    settings.antialiasing,
                )
            }
            .into();
            for (offset, sample) in samples.into_iter().enumerate() {
                let index = lane + offset;
                *left += sample * left_gain * lane_left_gains[index];
                *right += sample * right_gain * lane_right_gains[index];
            }
            lane += 8;
        }
        if lane + 4 <= voices {
            let phase_steps = std::array::from_fn(|offset| {
                let index = lane + offset;
                self.oscillator_bank.jitter_ratios[state_index][index] +=
                    self.oscillator_bank.jitter_steps[state_index][index];
                (oscillator_step
                    * oscillator.lane_pitch_ratios[index]
                    * self.oscillator_bank.jitter_ratios[state_index][index])
                    .min(0.45)
            });
            let oscillators = &mut self.oscillator_bank.oscillators[state_index][lane..lane + 4];
            let samples: [f32; 4] = if oscillator.engine == OscillatorEngineKind::Resynth {
                f32x4::from(std::array::from_fn(|offset| {
                    let (left, right) = generate_resynth_step_modulated(
                        &mut oscillators[offset],
                        oscillator,
                        &mut self.oscillator_bank.resynth_grain[state_index],
                        &mut self.oscillator_bank.resynth_grain_generations[state_index],
                        phase_steps[offset] * sample_rate,
                        sample_rate,
                        self.note_seed,
                        lane + offset,
                        grain_frame,
                        absolute,
                    );
                    (left + right) * 0.5
                }))
            } else if oscillator.custom_mix > f32::EPSILON {
                generate_custom4(
                    oscillators,
                    shape,
                    phase_steps,
                    pulse_width,
                    settings.antialiasing,
                    oscillator.phase_warp.mode,
                    phase_warp_amount,
                    oscillator.custom_curve,
                    oscillator.custom_mix,
                )
            } else if oscillator.phase_warp.mode != PhaseWarpMode::None
                && phase_warp_amount > f32::EPSILON
            {
                generate_shape4_warped(
                    oscillators,
                    shape,
                    phase_steps,
                    pulse_width,
                    settings.antialiasing,
                    oscillator.phase_warp.mode,
                    phase_warp_amount,
                )
            } else {
                generate_shape4(
                    oscillators,
                    shape,
                    phase_steps,
                    pulse_width,
                    settings.antialiasing,
                )
            }
            .into();
            for (offset, sample) in samples.into_iter().enumerate() {
                let index = lane + offset;
                *left += sample * left_gain * lane_left_gains[index];
                *right += sample * right_gain * lane_right_gains[index];
            }
            lane += 4;
        }
        while lane < voices {
            self.oscillator_bank.jitter_ratios[state_index][lane] +=
                self.oscillator_bank.jitter_steps[state_index][lane];
            let phase_step = (oscillator_step
                * oscillator.lane_pitch_ratios[lane]
                * self.oscillator_bank.jitter_ratios[state_index][lane])
                .min(0.45);
            let sample = if oscillator.engine == OscillatorEngineKind::Resynth {
                let (left, right) = generate_resynth_step_modulated(
                    &mut self.oscillator_bank.oscillators[state_index][lane],
                    oscillator,
                    &mut self.oscillator_bank.resynth_grain[state_index],
                    &mut self.oscillator_bank.resynth_grain_generations[state_index],
                    phase_step * sample_rate,
                    sample_rate,
                    self.note_seed,
                    lane,
                    grain_frame,
                    absolute,
                );
                (left + right) * 0.5
            } else if oscillator.custom_mix > f32::EPSILON {
                self.oscillator_bank.oscillators[state_index][lane].generate_custom_step(
                    shape,
                    phase_step,
                    pulse_width,
                    settings.antialiasing,
                    oscillator.phase_warp.mode,
                    phase_warp_amount,
                    oscillator.custom_curve,
                    oscillator.custom_mix,
                )
            } else if oscillator.phase_warp.mode != PhaseWarpMode::None
                && phase_warp_amount > f32::EPSILON
            {
                self.oscillator_bank.oscillators[state_index][lane].generate_shape_step_warped(
                    shape,
                    phase_step,
                    pulse_width,
                    settings.antialiasing,
                    oscillator.phase_warp.mode,
                    phase_warp_amount,
                )
            } else {
                self.oscillator_bank.oscillators[state_index][lane].generate_shape_step(
                    shape,
                    phase_step,
                    pulse_width,
                    settings.antialiasing,
                )
            };
            *left += sample * left_gain * lane_left_gains[lane];
            *right += sample * right_gain * lane_right_gains[lane];
            lane += 1;
        }
        apply_resynth_bus_mix(
            oscillator,
            &mut self.oscillator_bank.resynth_source[state_index],
            grain_frame,
            sample_rate,
            left_gain,
            right_gain,
            before_left,
            before_right,
            left,
            right,
        );
        if oscillator.engine == OscillatorEngineKind::Resynth {
            self.oscillator_bank.resynth_frame[state_index] = grain_frame.wrapping_add(1);
        }
    }

    #[allow(
        clippy::cast_possible_truncation,
        reason = "the deterministic per-note hash intentionally enters the f32 jitter model"
    )]
    #[cold]
    #[inline(never)]
    fn prepare_structural_jitter_target(
        &mut self,
        state_index: usize,
        slot: usize,
        settings: &OscillatorDspSettings,
        sample_rate: f32,
        update_interval: u16,
    ) {
        let voices = usize::from(settings.render_voices);
        let rate = settings.jitter_rate_hz;
        let target_clock = wrap_swarm_clock(
            self.oscillator_bank.jitter_clocks[state_index]
                + f32::from(update_interval) * rate / sample_rate.max(1.0),
        );
        let seed = unit_hash(
            self.note_seed
                ^ (slot as u64).wrapping_mul(0x4558_545f_554e_4953)
                ^ 0x4a49_5454_4552_5349,
        ) as f32;
        let mut offsets = [0.0; MAX_UNISON];
        let mut targets = [1.0; MAX_UNISON];
        if settings.jitter_active() {
            if settings.unison_jitter_mode == SwarmMode::Noise {
                fill_extended_unison_jitter_offsets(
                    &mut offsets[..voices],
                    seed,
                    settings.unison_jitter,
                    target_clock,
                );
            } else {
                fill_unison_jitter_offsets_mode(
                    &mut offsets[..voices],
                    seed,
                    settings.unison_jitter,
                    target_clock,
                    settings.unison_jitter_mode,
                );
            }
            jitter_pitch_ratios(
                &mut targets[..voices],
                &mut offsets[..voices],
                settings.swarm_depth_cents,
            );
        }
        let interval = f32::from(update_interval);
        for lane in 0..voices {
            self.oscillator_bank.jitter_steps[state_index][lane] =
                (targets[lane] - self.oscillator_bank.jitter_ratios[state_index][lane]) / interval;
        }
        self.oscillator_bank.jitter_ratios[state_index][voices..].fill(1.0);
        self.oscillator_bank.jitter_steps[state_index][voices..].fill(0.0);
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the clamped positive control interval fits in u16"
    )]
    fn advance_structural_jitter(
        &mut self,
        state_index: usize,
        slot: usize,
        settings: &OscillatorDspSettings,
        sample_rate: f32,
    ) {
        let rate = settings.jitter_rate_hz;
        if self.oscillator_bank.jitter_remaining[state_index] == 0 {
            let update_rate = rate
                * if settings.unison_jitter_mode == SwarmMode::Sine {
                    8.0
                } else {
                    1.0
                };
            let interval = (sample_rate.max(1.0) / update_rate).round().clamp(
                f32::from(SWARM_MIN_UPDATE_INTERVAL),
                f32::from(SWARM_MAX_UPDATE_INTERVAL),
            ) as u16;
            self.prepare_structural_jitter_target(
                state_index,
                slot,
                settings,
                sample_rate,
                interval,
            );
            self.oscillator_bank.jitter_remaining[state_index] = interval;
        }
        self.oscillator_bank.jitter_remaining[state_index] -= 1;
        self.oscillator_bank.jitter_clocks[state_index] = wrap_swarm_clock(
            self.oscillator_bank.jitter_clocks[state_index] + rate / sample_rate.max(1.0),
        );
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the clamped positive control interval fits in u16"
    )]
    #[inline(always)]
    fn advance_settled_structural_jitter_block<const SAMPLES: usize>(
        &mut self,
        state_index: usize,
        settings: &OscillatorDspSettings,
        sample_rate: f32,
    ) {
        debug_assert!(!settings.jitter_active());
        let rate = settings.jitter_rate_hz;
        let samples = SAMPLES as u32;
        let previous_remaining = u32::from(self.oscillator_bank.jitter_remaining[state_index]);
        let (remaining, refreshed) = if samples <= previous_remaining {
            (previous_remaining - samples, false)
        } else {
            let update_rate = rate
                * if settings.unison_jitter_mode == SwarmMode::Sine {
                    8.0
                } else {
                    1.0
                };
            let interval = (sample_rate.max(1.0) / update_rate).round().clamp(
                f32::from(SWARM_MIN_UPDATE_INTERVAL),
                f32::from(SWARM_MAX_UPDATE_INTERVAL),
            ) as u32;
            let offset = (samples - previous_remaining) % interval;
            (if offset == 0 { 0 } else { interval - offset }, true)
        };
        if refreshed {
            let voices = usize::from(settings.render_voices);
            self.oscillator_bank.jitter_ratios[state_index][voices..].fill(1.0);
            self.oscillator_bank.jitter_steps[state_index][voices..].fill(0.0);
        }
        self.oscillator_bank.jitter_remaining[state_index] = remaining as u16;
        let clock_step = rate / sample_rate.max(1.0);
        let initial_clock = self.oscillator_bank.jitter_clocks[state_index];
        let mut clock = initial_clock;
        for _ in 0..SAMPLES {
            clock += clock_step;
        }
        if clock >= 4_096.0 {
            clock = initial_clock;
            for _ in 0..SAMPLES {
                clock = wrap_swarm_clock(clock + clock_step);
            }
        }
        self.oscillator_bank.jitter_clocks[state_index] = clock;
    }

    pub(super) fn block_shape_banks_eligible(&self, settings: VoiceSettings) -> bool {
        if !self.unison_transitions_steady() {
            return false;
        }
        let mut any = false;
        for oscillator in 0..LEGACY_OSCILLATOR_COUNT {
            let oscillator_settings = settings.oscillator(oscillator);
            if oscillator_settings.enabled {
                any = true;
                let shape = self.effective_oscillator_shape(settings, oscillator);
                let motion = if oscillator == 0 {
                    self.unison.settings.motion_active()
                } else {
                    self.secondary_unison[oscillator - 1]
                        .settings
                        .motion_active()
                };
                if !oscillator_settings.custom_active()
                    && ((oscillator_settings.phase_warp_active() && motion)
                        || (shape - 2.0).abs() > f32::EPSILON && motion)
                {
                    return false;
                }
            }
        }
        any
    }

    pub(super) fn pitch_block_eligible(&self) -> bool {
        !self.is_gliding()
            && (self.enabled_oscillator_mask & 1 == 0 || !self.unison.settings.motion_active())
            && self
                .secondary_unison
                .iter()
                .enumerate()
                .all(|(secondary, layout)| {
                    self.enabled_oscillator_mask & (1 << (secondary + 1)) == 0
                        || !layout.settings.motion_active()
                })
    }

    pub(super) fn control_block_eligible(&self) -> bool {
        !self.is_gliding()
            && self.unison_transitions_steady()
            && (self.enabled_oscillator_mask & 1 == 0 || !self.unison.settings.motion_active())
            && self
                .secondary_unison
                .iter()
                .enumerate()
                .all(|(secondary, layout)| {
                    self.enabled_oscillator_mask & (1 << (secondary + 1)) == 0
                        || !layout.settings.motion_active()
                })
    }

    pub(super) fn spatial_block_eligible(&self) -> bool {
        (self.enabled_oscillator_mask & 1 == 0
            || stereo_square_weights(
                self.unison.settings.stereo_alternate,
                self.unison.settings.stereo_x,
            )[2] <= f32::EPSILON)
            && self
                .secondary_unison
                .iter()
                .enumerate()
                .all(|(secondary, layout)| {
                    self.enabled_oscillator_mask & (1 << (secondary + 1)) == 0
                        || stereo_square_weights(
                            layout.settings.stereo_alternate,
                            layout.settings.stereo_x,
                        )[2] <= f32::EPSILON
                })
    }

    pub(super) fn exact_saw_banks_eligible(&self, settings: VoiceSettings) -> bool {
        self.block_shape_banks_eligible(settings)
            && (0..LEGACY_OSCILLATOR_COUNT).all(|oscillator| {
                !settings.oscillator(oscillator).enabled
                    || !settings.oscillator(oscillator).custom_active()
                        && (self.effective_oscillator_shape(settings, oscillator) - 2.0).abs()
                            <= f32::EPSILON
            })
    }
}

#[inline]
fn wrap_swarm_clock(clock: f32) -> f32 {
    if clock >= 4_096.0 {
        clock - 4_096.0
    } else {
        clock
    }
}

#[inline]
pub(super) fn wrap_swarm_time(clock: f64) -> f64 {
    if clock >= 4_096.0 {
        clock - 4_096.0
    } else {
        clock
    }
}

#[inline]
fn tuned_phase_step(phase_step: f32, pitch_ratio: f32) -> f32 {
    if pitch_ratio.to_bits() == 1.0_f32.to_bits() {
        phase_step
    } else {
        (phase_step * pitch_ratio).min(0.45)
    }
}

fn constant_jitter_ramp_final<const SAMPLES: usize>(
    phase_steps: &[f32; MAX_UNISON],
    deltas: &[f32; MAX_UNISON],
    voices: usize,
    pitch_ratio: f32,
) -> Option<[f32; MAX_UNISON]> {
    let mut final_steps = *phase_steps;
    for index in 0..voices {
        for _ in 0..SAMPLES {
            final_steps[index] = (final_steps[index] + deltas[index]).min(0.45);
            if pitch_ratio > 1.0 && final_steps[index] * pitch_ratio > 0.45 {
                return None;
            }
        }
    }
    Some(final_steps)
}

pub(super) fn note_phase_seed(note: u8, channel: u8, voice_id: Option<i32>, age: u64) -> u64 {
    let voice_id = u64::from(voice_id.unwrap_or_default().cast_unsigned());
    age ^ (u64::from(note) << 48) ^ (u64::from(channel) << 40) ^ voice_id.rotate_left(17)
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "the shared random seed intentionally enters f32 meter and pan state"
)]
fn stereo_seed(seed: u64) -> f32 {
    unit_hash(seed.rotate_left(29) ^ 0x5354_4552_454f_4b56) as f32
}

pub(super) fn oscillator_stereo_seed(seed: u64, oscillator: usize) -> f32 {
    let seed = if oscillator == 0 {
        seed
    } else {
        seed ^ (oscillator as u64 * 0x4f53_435f_554e_4953)
    };
    stereo_seed(seed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grain_ignores_hidden_oscillator_unison_lanes() {
        let controls = crate::oscillators::ResynthControls::default();
        let model = crate::oscillators::analyze_wav(
            "grain.wav",
            crate::wav_test::wav_i16(
                1,
                48_000,
                (0..2_048).map(|index| {
                    let sample = (std::f32::consts::TAU * 220.0 * index as f32 / 48_000.0).sin();
                    (sample * 24_000.0) as i16
                }),
            ),
            controls,
        )
        .expect("analysis");
        let assets = crate::resynth_state::ResynthAssetPackState::new();
        assets
            .slot(0)
            .expect("slot")
            .replace(model, crate::oscillators::ResynthAlgorithm::Grain, controls)
            .expect("publish Grain");
        let view = assets
            .slot(0)
            .expect("slot")
            .try_rt_view_after(0)
            .expect("view");
        let mut plan = super::super::oscillator_bank::ResynthPlaybackPlan::default();
        assert!(plan.retarget(view, false, 48_000.0));
        let mut settings = OscillatorDspSettings::default();
        settings.engine = OscillatorEngineKind::Resynth;
        settings.render_voices = MAX_UNISON as u8;
        // SAFETY: the local plan remains address-stable for this immediate check.
        settings.resynth_playback = unsafe {
            super::super::oscillator_bank::ResynthPlaybackPtr::new(std::ptr::from_ref(&plan))
        };

        assert!(grain_uses_single_oscillator_lane(&settings));
    }

    #[test]
    fn mpe_member_and_per_note_bends_compose_and_persist() {
        let mut synth = PolySynth::default();
        synth.note_on(60, 1.0, 1, None);
        synth.pitch_bend(1, 1.0, 48.0);
        assert!((synth.voices[0].pitch_ratio - 16.0).abs() < 1.0e-5);

        synth.per_note_pitch_bend(60, 1, 12.0);
        synth.set_transpose(-12.0);
        assert!((synth.voices[0].pitch_ratio - 16.0).abs() < 1.0e-5);

        synth.pitch_bend(1, 0.5, 48.0);
        assert!((synth.voices[0].pitch_ratio - 4.0).abs() < 1.0e-5);
    }

    #[test]
    fn legato_glides_without_retriggering_and_returns_to_held_note() {
        let mut synth = PolySynth::default();
        synth.set_sample_rate(1_000.0);
        synth.configure_voice_mode(1);
        synth.set_glide_time(0.1);
        synth.note_on(60, 1.0, 1, None);
        let stage = synth.voices[0].stage;
        let envelope = synth.voices[0].envelope_level;

        synth.note_on(72, 0.8, 1, None);
        assert_eq!(synth.voices[0].stage, stage);
        assert_eq!(synth.voices[0].envelope_level, envelope);
        assert_eq!(synth.voices[0].glide_remaining, 100);

        let settings = VoiceSettings::new(2.0, 440.0, 0.5, 0.0, 0.0, 0.0);
        for _ in 0..100 {
            synth.render(settings, EnvelopeSettings::default());
        }
        assert!(!synth.is_gliding());
        assert!((synth.voices[0].frequency_hz - crate::dsp::midi_note_hz(72.0)).abs() < 1.0e-4);

        synth.note_off(72, 1, None);
        assert_eq!(synth.voices[0].current_note, Some(60));
        assert!(synth.is_gliding());
    }

    #[test]
    fn voice_mode_caps_the_active_pool() {
        let mut synth = PolySynth::default();
        synth.configure_voice_mode(4);
        for note in 60..68 {
            synth.note_on(note, 1.0, 0, None);
        }
        assert_eq!(synth.active_count, 4);
        assert_eq!(
            synth.voices.iter().filter(|voice| voice.active()).count(),
            4
        );
    }

    #[test]
    fn positioned_wave_suppresses_raw_timbre_in_all_bank_render_paths() {
        let mut oscillator = OscillatorDspSettings::default();
        assert_eq!(VaVoice::oscillator_timbre(&oscillator, 0.75), 0.75);
        oscillator.positioned_wave = true;
        assert_eq!(VaVoice::oscillator_timbre(&oscillator, 0.75), 0.0);
    }

    #[test]
    fn resynth_renderer_is_bounded_finite_and_never_reaches_eof() {
        let bytes = crate::wav_test::wav_i16(
            1,
            48_000,
            (0..24_000).map(|index| {
                let sample = (std::f32::consts::TAU * 220.0 * index as f32 / 48_000.0).sin();
                (sample * 24_000.0) as i16
            }),
        );
        let controls = crate::oscillators::ResynthControls::default();
        let model = crate::oscillators::analyze_wav("tone.wav", bytes, controls).expect("analysis");
        let assets = crate::resynth_state::ResynthAssetPackState::new();
        let generation = assets
            .slot(0)
            .expect("slot")
            .replace(
                model,
                crate::oscillators::ResynthAlgorithm::Sample,
                controls,
            )
            .expect("replace");
        let view = assets
            .slot(0)
            .expect("slot")
            .try_rt_view_after(0)
            .expect("view");
        assert_eq!(view.generation(), generation);
        let mut plan = super::super::oscillator_bank::ResynthPlaybackPlan::default();
        assert!(plan.retarget(view, false, 48_000.0));
        let mut settings = OscillatorDspSettings::default();
        settings.engine = OscillatorEngineKind::Resynth;
        // SAFETY: `plan` remains alive and address-stable for the render loop.
        settings.resynth_playback = unsafe {
            super::super::oscillator_bank::ResynthPlaybackPtr::new(std::ptr::from_ref(&plan))
        };
        let mut oscillator = VaOscillator::default();
        let mut peak = 0.0_f32;
        let mut grain_states = [GrainSchedulerState::default(); 2];
        let mut grain_generations = [0; 2];
        for index in 0..160_000 {
            let (left, right) = generate_resynth_step(
                &mut oscillator,
                &settings,
                &mut grain_states,
                &mut grain_generations,
                32.703_197,
                48_000.0,
                0,
                0,
                index as u64,
            );
            let sample = (left + right) * 0.5;
            assert!(sample.is_finite());
            peak = peak.max(sample.abs());
        }
        assert!(peak > 0.1);
    }

    #[test]
    fn rich_zone_handover_uses_two_bounded_phase_continuous_layers() {
        let bytes = crate::wav_test::wav_i16(
            1,
            48_000,
            (0..24_000).map(|index| {
                let time = index as f32 / 48_000.0;
                let sample = (std::f32::consts::TAU * 220.0 * time).sin() * 0.55
                    + (std::f32::consts::TAU * 3_100.0 * time).sin() * 0.25;
                (sample * 24_000.0) as i16
            }),
        );
        let controls = crate::oscillators::ResynthControls::default();
        let model = crate::oscillators::analyze_wav("rich.wav", bytes, controls).expect("analysis");
        let assets = crate::resynth_state::ResynthAssetPackState::new();
        assets
            .slot(0)
            .expect("slot")
            .replace(model, crate::oscillators::ResynthAlgorithm::Rich, controls)
            .expect("replace");
        let view = assets
            .slot(0)
            .expect("slot")
            .try_rt_view_after(0)
            .expect("view");
        let mut plan = super::super::oscillator_bank::ResynthPlaybackPlan::default();
        assert!(plan.retarget(view, false, 48_000.0));
        let mut settings = OscillatorDspSettings::default();
        settings.engine = OscillatorEngineKind::Resynth;
        // SAFETY: `plan` remains address-stable for the render loop.
        settings.resynth_playback = unsafe {
            super::super::oscillator_bank::ResynthPlaybackPtr::new(std::ptr::from_ref(&plan))
        };
        let mut oscillator = VaOscillator::default();
        let mut grain_states = [GrainSchedulerState::default(); 2];
        let mut grain_generations = [0; 2];
        let mut previous = 0.0_f32;
        let mut maximum_step = 0.0_f32;
        let mut saw_handover = false;
        for index in 0..8_192 {
            let target = 80.0 * 2.0_f32.powf(index as f32 / 2_048.0);
            let (left, right) = generate_resynth_step(
                &mut oscillator,
                &settings,
                &mut grain_states,
                &mut grain_generations,
                target,
                48_000.0,
                7,
                0,
                index,
            );
            let sample = (left + right) * 0.5;
            assert!(sample.is_finite());
            if index > 0 {
                maximum_step = maximum_step.max((sample - previous).abs());
            }
            previous = sample;
            saw_handover |= oscillator.resynth_zone_fade_remaining() != 0;
            // Revision and zone fades are serialized, so the sampled-layer
            // ceiling remains two.
            let layers = if plan.remaining != 0 || oscillator.resynth_zone_fade_remaining() != 0 {
                2
            } else {
                1
            };
            assert!(layers <= 2);
            plan.advance();
        }
        assert!(saw_handover);
        assert!(maximum_step < 1.5, "zone step {maximum_step}");
    }

    #[test]
    fn grain_telemetry_uses_the_target_generation_scheduler() {
        let mut synth = PolySynth::default();
        synth.note_on(60, 1.0, 0, None);
        let mut new_controls = crate::oscillators::ResynthControls::default();
        new_controls.position = 0.95;
        let source = (0..2_048)
            .map(|index| index as f32 / 2_047.0)
            .collect::<Vec<_>>();
        let new =
            crate::oscillators::GrainSourceArtifact::compile(&source, 48_000, None, new_controls)
                .expect("new Grain");
        let voice = &mut synth.voices[0];
        let _ =
            voice.oscillator_bank.resynth_grain[0][1].render_lane(&new, 220.0, 48_000.0, 1, 0, 0);
        voice.oscillator_bank.resynth_grain_generations[0] = [11, 12];
        let mut telemetry = crate::resynth_state::ResynthTelemetrySnapshot::default();
        assert!(voice.write_resynth_telemetry(0, source.len(), 12, &mut telemetry));
        assert!(telemetry.grain_lanes[0].active);
        assert!(telemetry.grain_lanes[0].position > 0.8);
    }

    #[test]
    fn raw_source_audition_is_independent_of_unison_bus_sum() {
        let bytes = crate::wav_test::wav_i16(
            1,
            48_000,
            (0..4_800).map(|index| {
                let sample = (std::f32::consts::TAU * 317.0 * index as f32 / 48_000.0).sin();
                (sample * 24_000.0) as i16
            }),
        );
        let controls = crate::oscillators::ResynthControls::default();
        let model =
            crate::oscillators::analyze_wav("source.wav", bytes, controls).expect("analysis");
        let assets = crate::resynth_state::ResynthAssetPackState::new();
        assets
            .slot(0)
            .expect("slot")
            .replace(
                model,
                crate::oscillators::ResynthAlgorithm::Sample,
                controls,
            )
            .expect("replace");
        let view = assets
            .slot(0)
            .expect("slot")
            .try_rt_view_after(0)
            .expect("view");
        let mut plan = super::super::oscillator_bank::ResynthPlaybackPlan::default();
        assert!(plan.retarget(view, false, 48_000.0));
        assert!(plan.set_source_audition(true, 48_000.0));
        for _ in 0..1_000 {
            plan.advance();
        }
        let mut settings = OscillatorDspSettings::default();
        settings.engine = OscillatorEngineKind::Resynth;
        // SAFETY: the local plan remains address-stable through this test.
        settings.resynth_playback = unsafe {
            super::super::oscillator_bank::ResynthPlaybackPtr::new(std::ptr::from_ref(&plan))
        };
        let mut one_lane_source = SourceAuditionState::default();
        let mut sixty_four_lane_source = SourceAuditionState::default();
        let mut heard = false;
        for frame in 0..256_u64 {
            let (mut one_left, mut one_right) = (1.0, 1.0);
            let (mut many_left, mut many_right) = (64.0, -31.0);
            apply_resynth_bus_mix(
                &settings,
                &mut one_lane_source,
                frame,
                48_000.0,
                0.7,
                0.7,
                0.0,
                0.0,
                &mut one_left,
                &mut one_right,
            );
            apply_resynth_bus_mix(
                &settings,
                &mut sixty_four_lane_source,
                frame,
                48_000.0,
                0.7,
                0.7,
                0.0,
                0.0,
                &mut many_left,
                &mut many_right,
            );
            assert!((one_left - many_left).abs() <= 1.0e-7);
            assert!((one_right - many_right).abs() <= 1.0e-7);
            heard |= one_left.abs() > 1.0e-4;
        }
        assert!(heard);
    }
}
