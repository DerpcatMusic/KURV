//! Fixed-capacity monophonic modulation envelopes.

use super::LfoConfig;

pub const ENVELOPE_COUNT: usize = super::LFO_COUNT;
const MIDI_CHANNELS: usize = 16;
const MIDI_NOTES: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnvelopeConfig {
    pub attack: f32,
    pub attack_curve: f32,
    pub decay: f32,
    pub decay_curve: f32,
    pub sustain: f32,
    pub release: f32,
    pub release_curve: f32,
}

impl Default for EnvelopeConfig {
    fn default() -> Self {
        Self {
            attack: 0.01,
            attack_curve: 0.0,
            decay: 0.1,
            decay_curve: 0.0,
            sustain: 0.8,
            release: 0.2,
            release_curve: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Stage {
    #[default]
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

#[derive(Clone, Copy, Debug, Default)]
struct EnvelopeState {
    stage: Stage,
    value: f32,
    start: f32,
    elapsed: u64,
}

impl EnvelopeState {
    fn trigger(&mut self) {
        self.stage = Stage::Attack;
        self.start = self.value;
        self.elapsed = 0;
    }

    fn release(&mut self) {
        if self.stage != Stage::Idle && self.stage != Stage::Release {
            self.stage = Stage::Release;
            self.start = self.value;
            self.elapsed = 0;
        }
    }

    fn reset(&mut self) {
        *self = Self::default();
    }

    #[inline(always)]
    fn next(&mut self, config: EnvelopeConfig, sample_rate: f32) -> f32 {
        match self.stage {
            Stage::Idle => self.value = 0.0,
            Stage::Attack => {
                if self.advance_toward(1.0, config.attack, config.attack_curve, sample_rate) {
                    self.stage = Stage::Decay;
                    self.start = 1.0;
                    self.elapsed = 0;
                }
            }
            Stage::Decay => {
                let sustain = config.sustain.clamp(0.0, 1.0);
                if self.advance_toward(sustain, config.decay, config.decay_curve, sample_rate) {
                    self.stage = Stage::Sustain;
                    self.value = sustain;
                }
            }
            Stage::Sustain => self.value = config.sustain.clamp(0.0, 1.0),
            Stage::Release => {
                if self.advance_toward(0.0, config.release, config.release_curve, sample_rate) {
                    self.reset();
                }
            }
        }
        self.value
    }

    #[inline(always)]
    fn advance_toward(&mut self, target: f32, seconds: f32, curve: f32, sample_rate: f32) -> bool {
        let samples = (seconds.max(0.0) * sample_rate.max(1.0)).round() as u64;
        if samples == 0 {
            self.value = target;
            return true;
        }
        self.elapsed = self.elapsed.saturating_add(1);
        let progress = (self.elapsed as f32 / samples as f32).min(1.0);
        let progress = shaped_progress(progress, curve);
        self.value = (target - self.start).mul_add(progress, self.start);
        self.elapsed >= samples
    }

    fn phase(self, config: EnvelopeConfig, sample_rate: f32) -> f32 {
        let seconds = match self.stage {
            Stage::Attack => config.attack,
            Stage::Decay => config.decay,
            Stage::Release => config.release,
            Stage::Sustain => return 1.0,
            Stage::Idle => return 0.0,
        };
        let samples = (seconds.max(0.0) * sample_rate.max(1.0)).round() as u64;
        if samples == 0 {
            1.0
        } else {
            (self.elapsed as f32 / samples as f32).min(1.0)
        }
    }
}

#[inline(always)]
pub(crate) fn shaped_progress(progress: f32, curve: f32) -> f32 {
    crate::dsp::curve_progress(progress, curve)
}

pub struct EnvelopeBank {
    states: Box<[EnvelopeState; ENVELOPE_COUNT]>,
    configs: Box<[EnvelopeConfig; ENVELOPE_COUNT]>,
    held_notes: Box<[u8; MIDI_CHANNELS * MIDI_NOTES]>,
    held_count: u16,
    sustain_mask: u16,
    envelope_mask: u64,
    sample_rate: f32,
}

impl Default for EnvelopeBank {
    fn default() -> Self {
        Self {
            states: boxed_array(EnvelopeState::default()),
            configs: boxed_array(EnvelopeConfig::default()),
            held_notes: boxed_array(0),
            held_count: 0,
            sustain_mask: 0,
            envelope_mask: 0,
            sample_rate: 44_100.0,
        }
    }
}

impl EnvelopeBank {
    pub fn reset(&mut self, sample_rate: f32) {
        self.states.fill(EnvelopeState::default());
        self.held_notes.fill(0);
        self.held_count = 0;
        self.sustain_mask = 0;
        self.sample_rate = sample_rate.max(1.0);
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate.max(1.0);
    }

    pub fn configure(&mut self, configs: &[LfoConfig; ENVELOPE_COUNT], mask: u64) {
        for (target, source) in self.configs.iter_mut().zip(configs) {
            *target = source.envelope_config;
        }
        let added = mask & !self.envelope_mask;
        let removed = self.envelope_mask & !mask;
        for index in 0..ENVELOPE_COUNT {
            let bit = 1_u64 << index;
            if removed & bit != 0 || added & bit != 0 && self.held_count == 0 {
                self.states[index].reset();
            } else if added & bit != 0 {
                self.states[index].trigger();
            }
        }
        self.envelope_mask = mask;
    }

    pub fn note_on(&mut self, note: u8, channel: u8) {
        let index = note_index(note, channel);
        if self.held_notes[index] != u8::MAX {
            self.held_notes[index] += 1;
            self.held_count = self.held_count.saturating_add(1);
        }
        for index in 0..ENVELOPE_COUNT {
            if self.envelope_mask & (1_u64 << index) != 0 {
                self.states[index].trigger();
            }
        }
    }

    pub fn note_off(&mut self, note: u8, channel: u8) {
        let index = note_index(note, channel);
        if self.held_notes[index] != 0 {
            self.held_notes[index] -= 1;
            self.held_count -= 1;
        }
        self.release_if_unheld();
    }

    pub fn sustain(&mut self, channel: u8, held: bool) {
        let bit = 1_u16 << channel.min(15);
        if held {
            self.sustain_mask |= bit;
        } else {
            self.sustain_mask &= !bit;
            self.release_if_unheld();
        }
    }

    pub fn all_notes_off(&mut self, channel: u8) {
        self.clear_channel(channel);
        self.release_if_unheld();
    }

    pub fn all_sound_off(&mut self, channel: u8) {
        self.clear_channel(channel);
        self.sustain_mask &= !(1_u16 << channel.min(15));
        if self.held_count == 0 {
            self.states.fill(EnvelopeState::default());
        }
    }

    pub fn reset_controllers(&mut self, channel: u8) {
        self.sustain(channel, false);
    }

    #[inline(always)]
    pub fn next_into(&mut self, source_mask: u64, values: &mut [f32; ENVELOPE_COUNT]) {
        let mut active = source_mask & self.envelope_mask;
        while active != 0 {
            let index = active.trailing_zeros() as usize;
            values[index] = self.states[index].next(self.configs[index], self.sample_rate);
            active &= active - 1;
        }
    }

    pub fn advance_by(&mut self, samples: usize) {
        for _ in 0..samples {
            let mut active = self.envelope_mask;
            while active != 0 {
                let index = active.trailing_zeros() as usize;
                self.states[index].next(self.configs[index], self.sample_rate);
                active &= active - 1;
            }
        }
    }

    pub fn ui_snapshot(&self) -> ([f32; ENVELOPE_COUNT], [f32; ENVELOPE_COUNT]) {
        let phases = std::array::from_fn(|index| {
            self.states[index].phase(self.configs[index], self.sample_rate)
        });
        let values = std::array::from_fn(|index| self.states[index].value.clamp(0.0, 1.0));
        (phases, values)
    }

    fn clear_channel(&mut self, channel: u8) {
        let start = usize::from(channel.min(15)) * MIDI_NOTES;
        for count in &mut self.held_notes[start..start + MIDI_NOTES] {
            self.held_count = self.held_count.saturating_sub(u16::from(*count));
            *count = 0;
        }
    }

    fn release_if_unheld(&mut self) {
        if self.held_count == 0 && self.sustain_mask == 0 {
            for index in 0..ENVELOPE_COUNT {
                if self.envelope_mask & (1_u64 << index) != 0 {
                    self.states[index].release();
                }
            }
        }
    }
}

fn note_index(note: u8, channel: u8) -> usize {
    usize::from(channel.min(15)) * MIDI_NOTES + usize::from(note.min(127))
}

fn boxed_array<T: Clone, const N: usize>(value: T) -> Box<[T; N]> {
    Vec::from_iter(std::iter::repeat_n(value, N))
        .into_boxed_slice()
        .try_into()
        .unwrap_or_else(|_| unreachable!())
}
