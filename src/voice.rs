//! Fixed-allocation polyphonic voice engine.

use crate::post::ResonantLowpass;
use crate::shape_osc::{ShapeSettings, ShapeVaOscillator};
use crate::va::{SpectralEffect, Waveform};

pub const POLYPHONY: usize = 16;

#[derive(Clone, Copy, Debug)]
pub struct VoiceSettings {
    pub waveform: Waveform,
    pub frequency_hz: f32,
    pub pulse_width: f32,
    pub center_hz: f32,
    pub spread_octaves: f32,
    pub mix: f32,
    pub sweep_phase: f32,
    pub keytrack: f32,
    pub stereo_offset: f32,
    pub cutoff_hz: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct EnvelopeSettings {
    pub attack: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
}

impl Default for EnvelopeSettings {
    fn default() -> Self {
        Self {
            attack: 0.008,
            decay: 0.18,
            sustain: 0.72,
            release: 0.28,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EnvelopeStage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

pub struct VaDispersionVoice {
    osc: ShapeVaOscillator,
    lp_l: ResonantLowpass,
    lp_r: ResonantLowpass,
    current_note: Option<u8>,
    voice_id: Option<i32>,
    channel: u8,
    age: u64,
    frequency_hz: f32,
    tuning_semitones: f32,
    pitch_bend: f32,
    velocity: f32,
    pressure: f32,
    env: f32,
    release_step: f32,
    stage: EnvelopeStage,
    held: bool,
    sustained: bool,
    envelope: EnvelopeSettings,
    resonance: f32,
    filter_env: f32,
}

impl Default for VaDispersionVoice {
    fn default() -> Self {
        Self {
            osc: ShapeVaOscillator::default(),
            lp_l: ResonantLowpass::default(),
            lp_r: ResonantLowpass::default(),
            current_note: None,
            voice_id: None,
            channel: 0,
            age: 0,
            frequency_hz: 110.0,
            tuning_semitones: 0.0,
            pitch_bend: 0.0,
            velocity: 1.0,
            pressure: 1.0,
            env: 0.0,
            release_step: 0.0,
            stage: EnvelopeStage::Idle,
            held: false,
            sustained: false,
            envelope: EnvelopeSettings::default(),
            resonance: 0.0,
            filter_env: 0.0,
        }
    }
}

impl VaDispersionVoice {
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.osc.set_sample_rate(sample_rate);
    }

    pub fn reset(&mut self) {
        self.osc.reset();
        self.lp_l.reset();
        self.lp_r.reset();
        self.current_note = None;
        self.voice_id = None;
        self.frequency_hz = 110.0;
        self.env = 0.0;
        self.stage = EnvelopeStage::Idle;
        self.held = false;
        self.sustained = false;
    }

    pub fn start(&mut self, note: u8, velocity: f32, channel: u8, voice_id: Option<i32>, age: u64) {
        self.osc.reset();
        self.lp_l.reset();
        self.lp_r.reset();
        self.current_note = Some(note);
        self.voice_id = voice_id;
        self.channel = channel.min(15);
        self.age = age;
        self.frequency_hz = midi_note_to_hz(note);
        self.tuning_semitones = 0.0;
        self.velocity = velocity.clamp(0.0, 1.0);
        self.pressure = 1.0;
        self.env = 0.0;
        self.stage = EnvelopeStage::Attack;
        self.held = true;
        self.sustained = false;
    }

    pub fn configure(
        &mut self,
        effect: SpectralEffect,
        envelope: EnvelopeSettings,
        resonance: f32,
        filter_env: f32,
    ) {
        self.osc.set_effect(effect);
        self.envelope = envelope;
        self.resonance = resonance.clamp(0.0, 1.0);
        self.filter_env = filter_env.clamp(-1.0, 1.0);
    }

    pub fn render(
        &mut self,
        settings: VoiceSettings,
        sample_rate: f32,
        force_gate: bool,
    ) -> (f32, f32) {
        if force_gate && self.stage == EnvelopeStage::Idle {
            self.stage = EnvelopeStage::Attack;
            self.velocity = 1.0;
            self.pressure = 1.0;
        }
        self.advance_envelope(sample_rate, force_gate);

        let bend_ratio = 2.0_f32.powf((self.tuning_semitones + self.pitch_bend) / 12.0);
        let frequency_hz = if force_gate {
            settings.frequency_hz
        } else {
            self.frequency_hz * bend_ratio
        };
        let osc_settings = ShapeSettings {
            waveform: settings.waveform,
            frequency_hz,
            pulse_width: settings.pulse_width,
            center_hz: settings.center_hz,
            spread_octaves: settings.spread_octaves,
            mix: settings.mix,
            sweep_phase: settings.sweep_phase,
            keytrack: settings.keytrack,
            stereo_offset: settings.stereo_offset,
        };

        let (mut left, mut right) = self.osc.generate(osc_settings);
        let envelope_cutoff = settings.cutoff_hz * 2.0_f32.powf(self.filter_env * self.env * 5.0);
        left = self
            .lp_l
            .process(left, envelope_cutoff, self.resonance, sample_rate);
        right = self
            .lp_r
            .process(right, envelope_cutoff, self.resonance, sample_rate);

        let amplitude = self.env * self.velocity * self.pressure;
        (left * amplitude, right * amplitude)
    }

    fn advance_envelope(&mut self, sample_rate: f32, force_gate: bool) {
        let sample_rate = sample_rate.max(1.0);
        match self.stage {
            EnvelopeStage::Idle => self.env = 0.0,
            EnvelopeStage::Attack => {
                self.env += 1.0 / (self.envelope.attack.max(0.0005) * sample_rate);
                if self.env >= 1.0 {
                    self.env = 1.0;
                    self.stage = EnvelopeStage::Decay;
                }
            }
            EnvelopeStage::Decay => {
                self.env -=
                    (1.0 - self.envelope.sustain) / (self.envelope.decay.max(0.001) * sample_rate);
                if self.env <= self.envelope.sustain {
                    self.env = self.envelope.sustain;
                    self.stage = EnvelopeStage::Sustain;
                }
            }
            EnvelopeStage::Sustain => self.env = self.envelope.sustain,
            EnvelopeStage::Release => {
                self.env = (self.env - self.release_step).max(0.0);
                if self.env <= 1.0e-5 {
                    self.env = 0.0;
                    self.stage = EnvelopeStage::Idle;
                    self.current_note = None;
                    self.voice_id = None;
                }
            }
        }
        if force_gate && self.stage == EnvelopeStage::Release {
            self.stage = EnvelopeStage::Attack;
        }
    }

    fn release(&mut self, immediate: bool, sample_rate: f32) {
        self.held = false;
        if self.sustained && !immediate {
            return;
        }
        if immediate {
            self.env = 0.0;
            self.stage = EnvelopeStage::Idle;
            self.current_note = None;
            self.voice_id = None;
        } else if self.stage != EnvelopeStage::Idle {
            self.release_step =
                self.env / (self.envelope.release.max(0.002) * sample_rate.max(1.0));
            self.stage = EnvelopeStage::Release;
        }
    }

    fn matches(&self, note: u8, channel: u8, voice_id: Option<i32>) -> bool {
        self.current_note == Some(note)
            && self.channel == channel
            && voice_id.is_none_or(|id| self.voice_id == Some(id))
    }

    fn active(&self) -> bool {
        self.stage != EnvelopeStage::Idle
    }
}

pub struct PolySynth {
    voices: [VaDispersionVoice; POLYPHONY],
    sample_rate: f32,
    age: u64,
    sustain: [bool; 16],
    pitch_bend: [f32; 16],
}

impl Default for PolySynth {
    fn default() -> Self {
        Self {
            voices: std::array::from_fn(|_| VaDispersionVoice::default()),
            sample_rate: 44_100.0,
            age: 0,
            sustain: [false; 16],
            pitch_bend: [0.0; 16],
        }
    }
}

impl PolySynth {
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate.max(1.0);
        for voice in &mut self.voices {
            voice.set_sample_rate(self.sample_rate);
        }
    }

    pub fn reset(&mut self) {
        for voice in &mut self.voices {
            voice.reset();
        }
        self.sustain.fill(false);
        self.pitch_bend.fill(0.0);
    }

    pub fn note_on(&mut self, note: u8, velocity: f32, channel: u8, voice_id: Option<i32>) {
        self.age = self.age.wrapping_add(1);
        let index = self
            .voices
            .iter()
            .position(|voice| !voice.active())
            .unwrap_or_else(|| {
                self.voices
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, voice)| voice.age)
                    .map_or(0, |(index, _)| index)
            });
        let channel = channel.min(15);
        self.voices[index].start(note, velocity, channel, voice_id, self.age);
        self.voices[index].pitch_bend = self.pitch_bend[channel as usize];
    }

    pub fn note_off(&mut self, note: u8, channel: u8, voice_id: Option<i32>) {
        if let Some(voice) = self
            .voices
            .iter_mut()
            .filter(|voice| voice.matches(note, channel, voice_id))
            .min_by_key(|voice| voice.age)
        {
            voice.sustained = self.sustain[channel.min(15) as usize];
            voice.release(false, self.sample_rate);
        }
    }

    pub fn choke(&mut self, note: u8, channel: u8, voice_id: Option<i32>) {
        for voice in &mut self.voices {
            if voice.matches(note, channel, voice_id) {
                voice.release(true, self.sample_rate);
            }
        }
    }

    pub fn pressure(&mut self, note: u8, channel: u8, voice_id: Option<i32>, pressure: f32) {
        for voice in &mut self.voices {
            if voice.matches(note, channel, voice_id) {
                voice.pressure = pressure.clamp(0.0, 1.0);
            }
        }
    }

    pub fn tuning(&mut self, note: u8, channel: u8, voice_id: Option<i32>, semitones: f32) {
        for voice in &mut self.voices {
            if voice.matches(note, channel, voice_id) {
                voice.tuning_semitones = semitones.clamp(-128.0, 128.0);
            }
        }
    }

    pub fn pitch_bend(&mut self, channel: u8, normalized: f32) {
        let channel = channel.min(15);
        let semitones = (normalized.clamp(0.0, 1.0) - 0.5) * 4.0;
        self.pitch_bend[channel as usize] = semitones;
        for voice in &mut self.voices {
            if voice.channel == channel {
                voice.pitch_bend = semitones;
            }
        }
    }

    pub fn sustain(&mut self, channel: u8, enabled: bool) {
        let channel = channel.min(15);
        self.sustain[channel as usize] = enabled;
        if !enabled {
            for voice in &mut self.voices {
                if voice.channel == channel && voice.sustained && !voice.held {
                    voice.sustained = false;
                    voice.release(false, self.sample_rate);
                }
            }
        }
    }

    pub fn render(
        &mut self,
        settings: VoiceSettings,
        effect: SpectralEffect,
        envelope: EnvelopeSettings,
        resonance: f32,
        filter_env: f32,
    ) -> (f32, f32) {
        let active = self.voices.iter().filter(|voice| voice.active()).count();
        if active == 0 {
            return (0.0, 0.0);
        }
        let gain = 0.9 / (active as f32).sqrt();
        let mut left = 0.0;
        let mut right = 0.0;
        for voice in &mut self.voices {
            if voice.active() {
                voice.configure(effect, envelope, resonance, filter_env);
                let (voice_l, voice_r) = voice.render(settings, self.sample_rate, false);
                left += voice_l;
                right += voice_r;
            }
        }
        (left * gain, right * gain)
    }
}

fn midi_note_to_hz(note: u8) -> f32 {
    440.0 * 2.0_f32.powf((note as f32 - 69.0) / 12.0)
}
