//! Fixed-capacity, sample-rate modulation engine.
//!
//! Curves are evaluated procedurally from KURV's periodic spline
//! coefficients. No sampled LFO table, allocation, lock, or collection is
//! touched by the audio thread.

use truce_core::events::TransportInfo;

#[path = "envelope.rs"]
pub(crate) mod envelope;

use crate::voices::LEGACY_OSCILLATOR_COUNT;
use crate::wave_curve::WaveCurveRt;
use envelope::{ENVELOPE_COUNT, EnvelopeBank, EnvelopeConfig};

pub const LFO_COUNT: usize = super::state::MAX_MODULATION_SOURCES;
pub const HOST_LFO_COUNT: usize = super::state::LEGACY_MODULATION_SOURCES;
pub const ROUTE_COUNT: usize = 16;

const MAX_RATE_HZ: f32 = 20_000.0;
const NYQUIST_GUARD: f32 = 0.45;
const MAX_INTERPOLATION_PHASE_SPAN: f64 = 1.0 / 1_024.0;
const MAX_INTERPOLATION_SAMPLES: usize = 64;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum LfoMode {
    #[default]
    Free,
    Retrigger,
    Sync,
    OneShot,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum LfoRateMode {
    #[default]
    Hertz,
    Milliseconds,
    Beat,
    Keytrack,
}

impl LfoRateMode {
    pub const fn from_index(index: u8) -> Self {
        match index {
            1 => Self::Milliseconds,
            2 => Self::Beat,
            3 => Self::Keytrack,
            _ => Self::Hertz,
        }
    }
}

impl LfoMode {
    pub const fn from_index(index: u8) -> Self {
        match index {
            1 => Self::Retrigger,
            2 => Self::Sync,
            3 => Self::OneShot,
            _ => Self::Free,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LfoConfig {
    pub rate_hz: f32,
    pub rate_mode: LfoRateMode,
    pub mode: LfoMode,
    pub phase_offset: f32,
    pub sync_division: u8,
    pub bipolar: bool,
    pub envelope: bool,
    pub envelope_config: EnvelopeConfig,
}

impl Default for LfoConfig {
    fn default() -> Self {
        Self {
            rate_hz: 1.0,
            rate_mode: LfoRateMode::Hertz,
            mode: LfoMode::Free,
            phase_offset: 0.0,
            sync_division: 4,
            bipolar: true,
            envelope: false,
            envelope_config: EnvelopeConfig::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RouteConfig {
    /// Zero disables the route; 1..=64 selects source slot 1..=64.
    pub source: u8,
    /// Zero disables the destination; remaining values are decoded by
    /// [`ModulationFrame::accumulate`].
    pub target: u8,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct OscillatorModulation {
    pub pitch_semitones: f32,
    pub shape: f32,
    pub pulse_width: f32,
    pub warp: f32,
    pub custom_shape: f32,
    pub level: f32,
    pub pan: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UnisonModulation {
    pub detune_amount: f32,
    pub detune_cents: f32,
    pub harmonic_align: f32,
    pub stereo: f32,
    pub phase_random: f32,
    pub curve: f32,
    pub jitter_amount: f32,
    pub jitter_rate_normalized: f32,
    pub stereo_x: f32,
    pub stereo_y: f32,
    pub weight: f32,
    pub pan_center: f32,
    pub pan_left: f32,
    pub pan_right: f32,
    pub pan_center_x: f32,
}

impl UnisonModulation {
    pub const fn frame_active(&self) -> bool {
        self.detune_amount.to_bits() != 0
            || self.detune_cents.to_bits() != 0
            || self.harmonic_align.to_bits() != 0
            || self.stereo.to_bits() != 0
            || self.curve.to_bits() != 0
            || self.stereo_x.to_bits() != 0
            || self.stereo_y.to_bits() != 0
            || self.weight.to_bits() != 0
            || self.pan_center.to_bits() != 0
            || self.pan_left.to_bits() != 0
            || self.pan_right.to_bits() != 0
            || self.pan_center_x.to_bits() != 0
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct GlobalModulation {
    pub output_db: f32,
    pub attack: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
    pub attack_curve: f32,
    pub decay_curve: f32,
    pub release_curve: f32,
    pub attack_curve_time: f32,
    pub decay_curve_time: f32,
    pub release_curve_time: f32,
    pub velocity: f32,
    pub pressure: f32,
    pub timbre: f32,
    pub glide: f32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ModulationFrame {
    pub oscillator: [OscillatorModulation; LEGACY_OSCILLATOR_COUNT],
    pub unison: [UnisonModulation; LEGACY_OSCILLATOR_COUNT],
    pub global: GlobalModulation,
}

pub struct LfoBank {
    envelopes: EnvelopeBank,
    phases: Box<[f64; LFO_COUNT]>,
    last_advanced_sample: Box<[u64; LFO_COUNT]>,
    one_shot_complete: Box<[bool; LFO_COUNT]>,
    configs: Box<[LfoConfig; LFO_COUNT]>,
    control_rates: Box<[f32; LFO_COUNT]>,
    effective_rates: Box<[f64; LFO_COUNT]>,
    phase_steps: Box<[f64; LFO_COUNT]>,
    curves: Box<[WaveCurveRt; LFO_COUNT]>,
    ui_phases: Box<[f32; LFO_COUNT]>,
    ui_values: Box<[f32; LFO_COUNT]>,
    values: Box<[f32; LFO_COUNT]>,
    interpolation_samples: Box<[u8; LFO_COUNT]>,
    interpolation_steps: Box<[f32; LFO_COUNT]>,
    interpolation_remaining: Box<[u8; LFO_COUNT]>,
    interpolation_mask: u64,
    active_mask: u64,
    source_mask: u64,
    envelope_mask: u64,
    modulation_mask: u64,
    modulation_indices: Box<[u8; LFO_COUNT]>,
    modulation_count: u8,
    direct_phase_mask: u64,
    direct_free_bipolar_mask: u64,
    direct_phase_catch_up_mask: u64,
    sample_clock: u64,
    sample_rate: f32,
    tempo: f64,
    transport_beats: f64,
    transport_seconds: f64,
    transport_beat_step: f64,
    transport_second_step: f64,
    transport_playing: bool,
    keytrack_hz: f32,
}

impl Default for LfoBank {
    fn default() -> Self {
        Self {
            envelopes: EnvelopeBank::default(),
            phases: boxed_array(0.0),
            last_advanced_sample: boxed_array(0),
            one_shot_complete: boxed_array(false),
            configs: boxed_array(LfoConfig::default()),
            control_rates: boxed_array(0.0),
            effective_rates: boxed_array(0.0),
            phase_steps: boxed_array(0.0),
            curves: boxed_array(WaveCurveRt::default()),
            ui_phases: boxed_array(0.0),
            ui_values: boxed_array(0.0),
            values: boxed_array(0.0),
            interpolation_samples: boxed_array(1),
            interpolation_steps: boxed_array(0.0),
            interpolation_remaining: boxed_array(0),
            interpolation_mask: 0,
            active_mask: 0,
            source_mask: 0,
            envelope_mask: 0,
            modulation_mask: 0,
            modulation_indices: boxed_array(0),
            modulation_count: 0,
            direct_phase_mask: 0,
            direct_free_bipolar_mask: 0,
            direct_phase_catch_up_mask: 0,
            sample_clock: 0,
            sample_rate: 44_100.0,
            tempo: 120.0,
            transport_beats: 0.0,
            transport_seconds: 0.0,
            transport_beat_step: 120.0 / 60.0 / 44_100.0,
            transport_second_step: 1.0 / 44_100.0,
            transport_playing: false,
            keytrack_hz: 261.625_55,
        }
    }
}

impl LfoBank {
    pub fn reset(&mut self, sample_rate: f32) {
        self.envelopes.reset(sample_rate);
        self.phases.fill(0.0);
        self.last_advanced_sample.fill(0);
        self.one_shot_complete.fill(false);
        self.ui_phases.fill(0.0);
        self.ui_values.fill(0.0);
        self.values.fill(0.0);
        self.interpolation_samples.fill(1);
        self.interpolation_steps.fill(0.0);
        self.interpolation_remaining.fill(0);
        self.interpolation_mask = 0;
        self.active_mask = 0;
        self.source_mask = 0;
        self.modulation_mask = 0;
        self.modulation_indices.fill(0);
        self.modulation_count = 0;
        self.direct_phase_mask = 0;
        self.direct_free_bipolar_mask = 0;
        self.direct_phase_catch_up_mask = 0;
        self.sample_clock = 0;
        self.sample_rate = sample_rate.max(1.0);
        self.refresh_phase_steps();
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.catch_up_all();
        self.interpolation_remaining.fill(0);
        self.sample_rate = sample_rate.max(1.0);
        self.envelopes.set_sample_rate(self.sample_rate);
        self.refresh_phase_steps();
    }

    pub fn configure(
        &mut self,
        configs: [LfoConfig; LFO_COUNT],
        curves: [Option<WaveCurveRt>; LFO_COUNT],
        active_mask: u64,
        transport: &TransportInfo,
        host_sample_rate: f32,
    ) {
        let tempo = if transport.tempo.is_finite() && transport.tempo > 0.0 {
            transport.tempo
        } else {
            120.0
        };
        let transport_beats = if transport.position_beats.is_finite() {
            transport.position_beats
        } else {
            0.0
        };
        let transport_seconds = if transport.position_seconds.is_finite()
            && (transport.position_seconds != 0.0 || transport.position_samples == 0)
        {
            transport.position_seconds
        } else {
            transport.position_samples as f64 / f64::from(host_sample_rate.max(1.0))
        };
        let tempo_changed = self.tempo.to_bits() != tempo.to_bits();
        let configs_changed = self.configs.as_ref() != &configs;
        let lfo_configs_changed = self.configs.iter().zip(configs).any(|(current, update)| {
            current.rate_hz != update.rate_hz
                || current.rate_mode != update.rate_mode
                || current.mode != update.mode
                || current.phase_offset != update.phase_offset
                || current.sync_division != update.sync_division
                || current.bipolar != update.bipolar
                || current.envelope != update.envelope
        });
        let curves_changed = self
            .curves
            .iter()
            .zip(curves.iter())
            .any(|(current, update)| update.is_some_and(|update| update != *current));

        self.transport_beats = transport_beats;
        self.transport_seconds = transport_seconds;
        self.transport_playing = transport.playing;
        if !configs_changed && !curves_changed && !tempo_changed {
            self.active_mask = active_mask;
            return;
        }

        self.catch_up_all();
        self.configs.copy_from_slice(&configs);
        self.envelope_mask = configs
            .into_iter()
            .enumerate()
            .fold(0, |mask, (index, config)| {
                mask | if config.envelope { 1_u64 << index } else { 0 }
            });
        self.envelopes.configure(&configs, self.envelope_mask);
        if lfo_configs_changed {
            self.interpolation_remaining.fill(0);
            self.direct_phase_mask =
                configs
                    .into_iter()
                    .enumerate()
                    .fold(0, |mask, (index, config)| {
                        mask | if !config.envelope
                            && config.mode != LfoMode::Sync
                            && config.phase_offset == 0.0
                        {
                            1_u64 << index
                        } else {
                            0
                        }
                    });
            self.direct_free_bipolar_mask =
                configs
                    .into_iter()
                    .enumerate()
                    .fold(0, |mask, (index, config)| {
                        mask | if !config.envelope
                            && config.mode == LfoMode::Free
                            && config.phase_offset == 0.0
                            && config.bipolar
                        {
                            1_u64 << index
                        } else {
                            0
                        }
                    });
            self.direct_phase_catch_up_mask |= self.modulation_mask;
        }
        self.refresh_modulation_mask();
        for (current, update) in self.curves.iter_mut().zip(curves) {
            if let Some(update) = update {
                *current = update;
            }
        }
        self.active_mask = active_mask;
        self.tempo = tempo;
        self.direct_phase_catch_up_mask |= self.modulation_mask;
        if tempo_changed || lfo_configs_changed {
            self.interpolation_remaining.fill(0);
            self.refresh_phase_steps();
        }
    }

    pub fn note_on(&mut self, note: u8, channel: u8) {
        self.envelopes.note_on(note, channel);
        self.catch_up_all();
        self.interpolation_remaining.fill(0);
        self.keytrack_hz = 440.0 * 2.0_f32.powf((f32::from(note) - 69.0) / 12.0);
        for index in 0..LFO_COUNT {
            if matches!(
                self.configs[index].mode,
                LfoMode::Retrigger | LfoMode::OneShot
            ) {
                self.phases[index] = 0.0;
                self.one_shot_complete[index] = false;
                self.last_advanced_sample[index] = self.sample_clock;
            }
        }
        self.refresh_phase_steps();
    }

    pub const fn is_active(&self) -> bool {
        self.source_mask != 0
    }

    pub(crate) fn filter_control_stride(
        &self,
        source_mask: u64,
        dynamic_host_control_mask: u8,
        mod_wheel: bool,
        max_stride: u8,
    ) -> u8 {
        let max_stride = max_stride.max(1);
        if mod_wheel
            || source_mask & self.envelope_mask != 0
            || source_mask & u64::from(dynamic_host_control_mask) != 0
        {
            return 1;
        }

        let mut selected = source_mask;
        let mut max_rate = 0.0_f64;
        while selected != 0 {
            let index = selected.trailing_zeros() as usize;
            selected &= selected - 1;
            max_rate = max_rate.max(self.effective_rates[index]);
        }
        if max_rate == 0.0 {
            return max_stride;
        }

        (f64::from(self.sample_rate) / (max_rate * 64.0))
            .floor()
            .clamp(1.0, f64::from(max_stride)) as u8
    }

    pub fn set_active_mask(&mut self, active_mask: u64) {
        self.active_mask = active_mask;
    }

    pub fn set_modulation_mask(&mut self, modulation_mask: u64) {
        self.source_mask = modulation_mask;
        self.refresh_modulation_mask();
    }

    fn refresh_modulation_mask(&mut self) {
        let modulation_mask = self.source_mask & !self.envelope_mask;
        if self.modulation_mask == modulation_mask {
            return;
        }
        self.interpolation_remaining.fill(0);
        let removed = self.modulation_mask & !modulation_mask;
        for index in 0..LFO_COUNT {
            if removed & (1_u64 << index) != 0 {
                self.values[index] = 0.0;
            }
        }
        let added = modulation_mask & !self.modulation_mask;
        self.modulation_mask = modulation_mask;
        self.direct_phase_catch_up_mask |= added;
        let mut count = 0;
        for index in 0..LFO_COUNT {
            if modulation_mask & (1_u64 << index) != 0 {
                self.modulation_indices[count] = index as u8;
                count += 1;
            }
        }
        self.modulation_count = count as u8;
    }

    pub fn next_ref(&mut self) -> &[f32; LFO_COUNT] {
        self.interpolation_remaining.fill(0);
        self.advance_values_general();
        self.envelopes.next_into(self.source_mask, &mut self.values);
        self.values.as_ref()
    }

    pub fn next_direct_ref(&mut self) -> &[f32; LFO_COUNT] {
        self.advance_values_direct();
        self.envelopes.next_into(self.source_mask, &mut self.values);
        self.values.as_ref()
    }

    pub const fn direct_phase_active(&self) -> bool {
        self.modulation_mask & !self.direct_phase_mask == 0
    }

    fn advance_values_general(&mut self) {
        if self.modulation_count == 1 {
            let index = usize::from(self.modulation_indices[0]);
            self.catch_up_phase(index);
            let phase = self.current_phase(index);
            self.values[index] = self.current_value(index, phase);
            self.advance_phase(index);
        } else {
            for offset in 0..usize::from(self.modulation_count) {
                let index = usize::from(self.modulation_indices[offset]);
                self.catch_up_phase_if_needed(index);
                let phase = self.current_phase(index);
                self.values[index] = self.current_value(index, phase);
                self.advance_phase(index);
            }
        }
        self.sample_clock = self.sample_clock.wrapping_add(1);
        self.advance_transport();
    }

    fn advance_values_direct(&mut self) {
        self.catch_up_direct_phases_if_needed();
        if self.modulation_mask & !self.direct_free_bipolar_mask == 0 {
            self.advance_values_direct_free_bipolar();
            return;
        }
        self.interpolation_remaining.fill(0);
        if self.modulation_count == 1 {
            let index = usize::from(self.modulation_indices[0]);
            let phase = self.phases[index] as f32;
            self.values[index] = self.current_value(index, phase);
            self.advance_phase(index);
        } else {
            for offset in 0..usize::from(self.modulation_count) {
                let index = usize::from(self.modulation_indices[offset]);
                let phase = self.phases[index] as f32;
                self.values[index] = self.current_value(index, phase);
                self.advance_phase(index);
            }
        }
        self.sample_clock = self.sample_clock.wrapping_add(1);
        self.advance_transport();
    }

    #[inline(always)]
    fn advance_values_direct_free_bipolar(&mut self) {
        if self.modulation_mask & self.interpolation_mask == 0 {
            if self.modulation_count == LFO_COUNT as u8 {
                for index in 0..LFO_COUNT {
                    self.values[index] = self.curves[index]
                        .eval(self.phases[index] as f32)
                        .clamp(-1.0, 1.0);
                    self.advance_free_phase(index);
                }
            } else {
                for offset in 0..usize::from(self.modulation_count) {
                    let index = usize::from(self.modulation_indices[offset]);
                    self.values[index] = self.curves[index]
                        .eval(self.phases[index] as f32)
                        .clamp(-1.0, 1.0);
                    self.advance_free_phase(index);
                }
            }
            self.sample_clock = self.sample_clock.wrapping_add(1);
            self.advance_transport();
            return;
        }
        if self.modulation_count == 1 {
            let index = usize::from(self.modulation_indices[0]);
            self.advance_interpolated_free_value(index);
            self.advance_free_phase(index);
        } else {
            for offset in 0..usize::from(self.modulation_count) {
                let index = usize::from(self.modulation_indices[offset]);
                self.advance_interpolated_free_value(index);
                self.advance_free_phase(index);
            }
        }
        self.sample_clock = self.sample_clock.wrapping_add(1);
        self.advance_transport();
    }

    #[inline(always)]
    fn advance_interpolated_free_value(&mut self, index: usize) {
        if self.interpolation_mask & (1_u64 << index) == 0 {
            self.values[index] = self.curves[index]
                .eval(self.phases[index] as f32)
                .clamp(-1.0, 1.0);
            return;
        }
        if self.interpolation_remaining[index] != 0 {
            self.values[index] += self.interpolation_steps[index];
            self.interpolation_remaining[index] -= 1;
            return;
        }

        let phase = self.phases[index];
        let value = self.curves[index].eval(phase as f32).clamp(-1.0, 1.0);
        self.values[index] = value;
        let samples = usize::from(self.interpolation_samples[index]);
        let end_phase = (phase + self.phase_steps[index] * samples as f64).rem_euclid(1.0);
        let end_value = self.curves[index].eval(end_phase as f32).clamp(-1.0, 1.0);
        self.interpolation_steps[index] = (end_value - value) / samples as f32;
        self.interpolation_remaining[index] = (samples - 1) as u8;
    }

    #[inline(always)]
    fn advance_free_phase(&mut self, index: usize) {
        let next = self.phases[index] + self.phase_steps[index];
        self.phases[index] = if next >= 1.0 { next - 1.0 } else { next };
        self.last_advanced_sample[index] = self.sample_clock.wrapping_add(1);
    }

    #[inline(always)]
    fn catch_up_direct_phases_if_needed(&mut self) {
        let mut pending = self.direct_phase_catch_up_mask & self.modulation_mask;
        if pending == 0 {
            return;
        }
        while pending != 0 {
            let index = pending.trailing_zeros() as usize;
            self.catch_up_phase(index);
            pending &= pending - 1;
        }
        self.direct_phase_catch_up_mask &= !self.modulation_mask;
    }

    fn advance_values_with_controls<const CONTROL_BLOCK: usize>(
        &mut self,
        dynamic_control_mask: u8,
        rate_hz: &[[f32; CONTROL_BLOCK]],
        phase_offsets: &[[f32; CONTROL_BLOCK]],
        frame: usize,
    ) {
        self.interpolation_remaining.fill(0);
        if self.modulation_count == 1 {
            let index = usize::from(self.modulation_indices[0]);
            let dynamic_controls =
                index < HOST_LFO_COUNT && dynamic_control_mask & (1_u8 << index) != 0;
            if dynamic_controls {
                let rate = rate_hz[index][frame];
                if rate.to_bits() != self.control_rates[index].to_bits() {
                    self.refresh_phase_step(index, rate);
                    self.control_rates[index] = rate;
                }
            }
            self.catch_up_phase(index);
            let phase = if dynamic_controls {
                self.current_phase_with_offset(index, phase_offsets[index][frame])
            } else {
                self.current_phase(index)
            };
            self.values[index] = self.current_value(index, phase);
            self.advance_phase(index);
        } else {
            for offset in 0..usize::from(self.modulation_count) {
                let index = usize::from(self.modulation_indices[offset]);
                let dynamic_controls =
                    index < HOST_LFO_COUNT && dynamic_control_mask & (1_u8 << index) != 0;
                if dynamic_controls {
                    let rate = rate_hz[index][frame];
                    if rate.to_bits() != self.control_rates[index].to_bits() {
                        self.refresh_phase_step(index, rate);
                        self.control_rates[index] = rate;
                    }
                }
                self.catch_up_phase_if_needed(index);
                let phase = if dynamic_controls {
                    self.current_phase_with_offset(index, phase_offsets[index][frame])
                } else {
                    self.current_phase(index)
                };
                self.values[index] = self.current_value(index, phase);
                self.advance_phase(index);
            }
        }
        self.sample_clock = self.sample_clock.wrapping_add(1);
        self.advance_transport();
    }

    pub fn next_with_controls_ref<const CONTROL_BLOCK: usize>(
        &mut self,
        dynamic_control_mask: u8,
        rate_hz: &[[f32; CONTROL_BLOCK]],
        phase_offsets: &[[f32; CONTROL_BLOCK]],
        frame: usize,
    ) -> &[f32; LFO_COUNT] {
        self.advance_values_with_controls(dynamic_control_mask, rate_hz, phase_offsets, frame);
        self.envelopes.next_into(self.source_mask, &mut self.values);
        self.values.as_ref()
    }

    pub fn ui_snapshot(&mut self) -> (&[f32; LFO_COUNT], &[f32; LFO_COUNT]) {
        self.catch_up_all();
        for index in 0..LFO_COUNT {
            if self.active_mask & (1_u64 << index) == 0
                || self.envelope_mask & (1_u64 << index) != 0
            {
                continue;
            }
            let phase = self.current_phase(index);
            self.ui_phases[index] = phase;
            self.ui_values[index] = self.current_value(index, phase);
        }
        let (envelope_phases, envelope_values) = self.envelopes.ui_snapshot();
        for index in 0..ENVELOPE_COUNT {
            if self.active_mask & self.envelope_mask & (1_u64 << index) != 0 {
                self.ui_phases[index] = envelope_phases[index];
                self.ui_values[index] = envelope_values[index];
            }
        }
        (self.ui_phases.as_ref(), self.ui_values.as_ref())
    }

    pub fn advance_silent(&mut self, samples: usize) {
        self.interpolation_remaining.fill(0);
        self.envelopes.advance_by(samples);
        self.sample_clock = self.sample_clock.wrapping_add(samples as u64);
        self.advance_transport_by(samples as u64);
    }

    pub fn note_off(&mut self, note: u8, channel: u8) {
        self.envelopes.note_off(note, channel);
    }

    pub fn sustain(&mut self, channel: u8, held: bool) {
        self.envelopes.sustain(channel, held);
    }

    pub fn all_notes_off(&mut self, channel: u8) {
        self.envelopes.all_notes_off(channel);
    }

    pub fn all_sound_off(&mut self, channel: u8) {
        self.envelopes.all_sound_off(channel);
    }

    pub fn reset_controllers(&mut self, channel: u8) {
        self.envelopes.reset_controllers(channel);
    }

    fn advance_phase(&mut self, index: usize) {
        let config = self.configs[index];
        if config.mode == LfoMode::Sync
            || (config.mode == LfoMode::OneShot && self.one_shot_complete[index])
        {
            self.last_advanced_sample[index] = self.sample_clock.wrapping_add(1);
            return;
        }
        let next = self.phases[index] + self.phase_steps[index];
        if config.mode == LfoMode::OneShot && next >= 1.0 {
            self.phases[index] = 1.0 - f64::EPSILON;
            self.one_shot_complete[index] = true;
        } else {
            self.phases[index] = if next >= 1.0 { next - 1.0 } else { next };
        }
        self.last_advanced_sample[index] = self.sample_clock.wrapping_add(1);
    }

    #[inline(always)]
    fn catch_up_phase_if_needed(&mut self, index: usize) {
        if self.sample_clock != self.last_advanced_sample[index] {
            self.catch_up_phase(index);
        }
    }

    fn current_phase(&self, index: usize) -> f32 {
        self.current_phase_with_offset(index, self.configs[index].phase_offset)
    }

    fn current_phase_with_offset(&self, index: usize, phase_offset: f32) -> f32 {
        let config = self.configs[index];
        if config.mode == LfoMode::Sync {
            let cycles = if config.rate_mode == LfoRateMode::Beat {
                self.transport_beats / sync_beats(config.sync_division)
            } else {
                self.transport_seconds * self.effective_rates[index]
            };
            (cycles + f64::from(phase_offset)).rem_euclid(1.0) as f32
        } else {
            if phase_offset == 0.0 {
                return self.phases[index] as f32;
            }
            let shifted = self.phases[index] + f64::from(phase_offset);
            (if shifted >= 1.0 {
                shifted - 1.0
            } else {
                shifted
            }) as f32
        }
    }

    fn current_value(&self, index: usize, phase: f32) -> f32 {
        let config = self.configs[index];
        let raw = if config.mode == LfoMode::OneShot && self.one_shot_complete[index] {
            self.curves[index].eval(1.0 - f32::EPSILON)
        } else {
            self.curves[index].eval(phase)
        };
        if config.bipolar {
            raw.clamp(-1.0, 1.0)
        } else {
            raw.mul_add(0.5, 0.5).clamp(0.0, 1.0)
        }
    }

    fn catch_up_all(&mut self) {
        for index in 0..LFO_COUNT {
            self.catch_up_phase(index);
        }
    }

    fn catch_up_phase(&mut self, index: usize) {
        let samples = self
            .sample_clock
            .saturating_sub(self.last_advanced_sample[index]);
        if samples == 0 {
            return;
        }
        let config = self.configs[index];
        if config.mode == LfoMode::OneShot {
            let next = self.phases[index] + self.phase_steps[index] * samples as f64;
            if next >= 1.0 {
                self.phases[index] = 1.0 - f64::EPSILON;
                self.one_shot_complete[index] = true;
            } else {
                self.phases[index] = next;
            }
        } else if config.mode != LfoMode::Sync {
            self.phases[index] =
                (self.phases[index] + self.phase_steps[index] * samples as f64).rem_euclid(1.0);
        }
        self.last_advanced_sample[index] = self.sample_clock;
    }

    fn advance_transport(&mut self) {
        self.advance_transport_by(1);
    }

    fn advance_transport_by(&mut self, samples: u64) {
        if self.transport_playing {
            let samples = samples as f64;
            self.transport_beats += self.transport_beat_step * samples;
            self.transport_seconds += self.transport_second_step * samples;
        }
    }

    fn refresh_phase_steps(&mut self) {
        let sample_rate = self.sample_rate;
        let tempo = self.tempo;
        let keytrack_hz = self.keytrack_hz;
        self.transport_second_step = 1.0 / f64::from(sample_rate);
        self.transport_beat_step = tempo / 60.0 * self.transport_second_step;
        for index in 0..LFO_COUNT {
            let config = self.configs[index];
            self.set_phase_step(index, config.rate_hz, sample_rate, tempo, keytrack_hz);
            self.control_rates[index] = config.rate_hz;
        }
    }

    fn refresh_phase_step(&mut self, index: usize, rate_hz: f32) {
        self.set_phase_step(
            index,
            rate_hz,
            self.sample_rate,
            self.tempo,
            self.keytrack_hz,
        );
    }

    fn set_phase_step(
        &mut self,
        index: usize,
        rate_hz: f32,
        sample_rate: f32,
        tempo: f64,
        keytrack_hz: f32,
    ) {
        let mut config = self.configs[index];
        config.rate_hz = rate_hz;
        let rate = f64::from(effective_rate(config, sample_rate, tempo, keytrack_hz));
        self.effective_rates[index] = rate;
        self.phase_steps[index] = rate / f64::from(sample_rate);
        let interpolation_samples = (MAX_INTERPOLATION_PHASE_SPAN
            / self.phase_steps[index].max(f64::EPSILON))
        .floor()
        .clamp(1.0, MAX_INTERPOLATION_SAMPLES as f64) as u8;
        self.interpolation_samples[index] = interpolation_samples;
        if interpolation_samples > 1 {
            self.interpolation_mask |= 1_u64 << index;
        } else {
            self.interpolation_mask &= !(1_u64 << index);
        }
    }
}

fn boxed_array<T: Clone, const N: usize>(value: T) -> Box<[T; N]> {
    Vec::from_iter(std::iter::repeat_n(value, N))
        .into_boxed_slice()
        .try_into()
        .unwrap_or_else(|_| unreachable!())
}

fn effective_rate(config: LfoConfig, sample_rate: f32, tempo: f64, keytrack_hz: f32) -> f32 {
    let rate = match config.rate_mode {
        LfoRateMode::Hertz => config.rate_hz,
        LfoRateMode::Milliseconds => 1_000.0 / config.rate_hz.max(0.01),
        LfoRateMode::Beat => (tempo as f32 / 60.0) / sync_beats(config.sync_division) as f32,
        LfoRateMode::Keytrack => keytrack_hz * keytrack_multiplier(config.rate_hz),
    };
    rate.clamp(0.0, MAX_RATE_HZ.min(sample_rate * NYQUIST_GUARD))
}

pub fn keytrack_multiplier(rate_value: f32) -> f32 {
    let rate_value = rate_value.clamp(0.01, 20_000.0);
    if rate_value <= 1.0 {
        2.0_f32.powf(5.0 * rate_value.log10() / 2.0)
    } else {
        2.0_f32.powf(5.0 * rate_value.log10() / 20_000.0_f32.log10())
    }
}

/// Cycle duration in quarter-note beats. This includes straight, triplet,
/// and dotted choices without approximating tempo as an LFO-rate ramp.
pub const fn sync_beats(index: u8) -> f64 {
    const BEATS: [f64; 16] = [
        1.0 / 16.0,
        1.0 / 12.0,
        1.0 / 8.0,
        1.0 / 6.0,
        1.0 / 4.0,
        1.0 / 3.0,
        1.0 / 2.0,
        2.0 / 3.0,
        1.0,
        4.0 / 3.0,
        2.0,
        8.0 / 3.0,
        4.0,
        8.0,
        16.0,
        32.0,
    ];
    BEATS[if index > 15 { 15 } else { index } as usize]
}
