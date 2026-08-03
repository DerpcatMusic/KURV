use nice_plug::prelude::*;
use std::num::NonZeroU32;
use std::sync::Arc;

mod editor;
mod post;
mod shape_osc;
mod va;
mod voice;

use va::{SpectralEffect, Waveform};
use voice::{EnvelopeSettings, PolySynth, VaDispersionVoice, VoiceSettings};

pub struct PureVaDispersionCore {
    params: Arc<PureVaDispersionParams>,
    synth: PolySynth,
    drone_voice: VaDispersionVoice,
    sample_rate: f32,
    sweep_phase: f32,
}

#[derive(Enum, Clone, Copy, PartialEq, Eq)]
enum WaveMode {
    #[name = "Saw"]
    Saw,
    #[name = "Pulse"]
    Pulse,
}

impl From<WaveMode> for Waveform {
    fn from(mode: WaveMode) -> Self {
        match mode {
            WaveMode::Saw => Waveform::Saw,
            WaveMode::Pulse => Waveform::Pulse,
        }
    }
}

#[derive(Enum, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SpectralMode {
    #[name = "Phase Disperse"]
    PhaseDisperse,
    #[name = "Harmonic Stretch"]
    HarmonicStretch,
    #[name = "Formant"]
    Formant,
    #[name = "Spectral Fold"]
    SpectralFold,
}

impl From<SpectralMode> for SpectralEffect {
    fn from(mode: SpectralMode) -> Self {
        match mode {
            SpectralMode::PhaseDisperse => SpectralEffect::PhaseDisperse,
            SpectralMode::HarmonicStretch => SpectralEffect::HarmonicStretch,
            SpectralMode::Formant => SpectralEffect::Formant,
            SpectralMode::SpectralFold => SpectralEffect::SpectralFold,
        }
    }
}

#[derive(Params)]
pub(crate) struct PureVaDispersionParams {
    #[persist = "editor-state"]
    editor_state: Arc<nice_plug_egui::EguiState>,

    #[id = "gain"]
    output_db: FloatParam,

    #[id = "drone"]
    drone: BoolParam,

    #[id = "freq"]
    drone_frequency: FloatParam,

    #[id = "wave"]
    wave: EnumParam<WaveMode>,

    #[id = "spectral"]
    spectral_effect: EnumParam<SpectralMode>,

    #[id = "pw"]
    pulse_width: FloatParam,

    #[id = "center"]
    center_hz: FloatParam,

    #[id = "spread"]
    spread_octaves: FloatParam,

    #[id = "mix"]
    mix: FloatParam,

    #[id = "sweeprate"]
    sweep_rate_hz: FloatParam,

    #[id = "keytrack"]
    keytrack: FloatParam,

    #[id = "stereo"]
    stereo_offset_octaves: FloatParam,

    #[id = "cutoff"]
    cutoff_hz: FloatParam,

    #[id = "resonance"]
    resonance: FloatParam,

    #[id = "filterenv"]
    filter_env: FloatParam,

    #[id = "attack"]
    attack: FloatParam,

    #[id = "decay"]
    decay: FloatParam,

    #[id = "sustain"]
    sustain: FloatParam,

    #[id = "release"]
    release: FloatParam,
}

impl Default for PureVaDispersionCore {
    fn default() -> Self {
        Self {
            params: Arc::new(PureVaDispersionParams::default()),
            synth: PolySynth::default(),
            drone_voice: VaDispersionVoice::default(),
            sample_rate: 44_100.0,
            sweep_phase: 0.0,
        }
    }
}

impl Default for PureVaDispersionParams {
    fn default() -> Self {
        Self {
            editor_state: editor::default_editor_state(),

            output_db: FloatParam::new(
                "Output",
                -9.0,
                FloatRange::Linear {
                    min: -48.0,
                    max: 6.0,
                },
            )
            .with_smoother(SmoothingStyle::Linear(5.0))
            .with_step_size(0.1)
            .with_unit(" dB"),

            drone: BoolParam::new("Audition Drone", false),

            drone_frequency: hz_param("Drone Frequency", 110.0, 20.0, 5_000.0),

            wave: EnumParam::new("Wave", WaveMode::Saw),

            spectral_effect: EnumParam::new("Spectral Effect", SpectralMode::PhaseDisperse),

            pulse_width: FloatParam::new(
                "Pulse Width",
                0.50,
                FloatRange::Linear {
                    min: 0.03,
                    max: 0.97,
                },
            )
            .with_smoother(SmoothingStyle::Linear(8.0))
            .with_step_size(0.001),

            center_hz: hz_param("Spectral Focus", 1_100.0, 60.0, 12_000.0),

            spread_octaves: FloatParam::new(
                "Spectral Shape",
                2.4,
                FloatRange::Linear { min: 0.0, max: 6.0 },
            )
            .with_smoother(SmoothingStyle::Linear(20.0))
            .with_step_size(0.01)
            .with_unit(" oct"),

            mix: unit_param("Spectral Amount", 0.48),

            sweep_rate_hz: FloatParam::new(
                "Motion",
                0.08,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 8.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_smoother(SmoothingStyle::Linear(20.0))
            .with_step_size(0.001)
            .with_unit(" Hz"),

            keytrack: unit_param("Keytrack", 0.70),

            stereo_offset_octaves: FloatParam::new(
                "Stereo",
                0.08,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(20.0))
            .with_step_size(0.001)
            .with_unit(" oct"),

            cutoff_hz: hz_param("Filter Cutoff", 6_500.0, 40.0, 20_000.0),

            resonance: unit_param("Resonance", 0.18),

            filter_env: FloatParam::new(
                "Filter Envelope",
                0.35,
                FloatRange::Linear {
                    min: -1.0,
                    max: 1.0,
                },
            )
            .with_smoother(SmoothingStyle::Linear(15.0))
            .with_step_size(0.001),

            attack: seconds_param("Attack", 0.008, 0.0005, 8.0),
            decay: seconds_param("Decay", 0.18, 0.001, 8.0),
            sustain: unit_param("Sustain", 0.72),
            release: seconds_param("Release", 0.28, 0.002, 12.0),
        }
    }
}

fn unit_param(name: &'static str, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
        .with_smoother(SmoothingStyle::Linear(15.0))
        .with_step_size(0.001)
}

fn hz_param(name: &'static str, default: f32, min: f32, max: f32) -> FloatParam {
    FloatParam::new(
        name,
        default,
        FloatRange::Skewed {
            min,
            max,
            factor: FloatRange::skew_factor(-2.0),
        },
    )
    .with_smoother(SmoothingStyle::Linear(20.0))
    .with_value_to_string(formatters::v2s_f32_hz_then_khz(1))
    .with_string_to_value(formatters::s2v_f32_hz_then_khz())
}

fn seconds_param(name: &'static str, default: f32, min: f32, max: f32) -> FloatParam {
    FloatParam::new(
        name,
        default,
        FloatRange::Skewed {
            min,
            max,
            factor: FloatRange::skew_factor(-2.5),
        },
    )
    .with_step_size(0.001)
    .with_unit(" s")
}

impl Plugin for PureVaDispersionCore {
    const NAME: &'static str = "KURV";
    const VENDOR: &'static str = "Prototype Lab";
    const URL: &'static str = env!("CARGO_PKG_HOMEPAGE");
    const EMAIL: &'static str = "dev@example.com";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout {
            main_input_channels: None,
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        },
        AudioIOLayout {
            main_input_channels: None,
            main_output_channels: NonZeroU32::new(1),
            ..AudioIOLayout::const_default()
        },
    ];

    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::build(self.params.clone())
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate = buffer_config.sample_rate;
        self.synth.set_sample_rate(self.sample_rate);
        self.drone_voice.set_sample_rate(self.sample_rate);
        true
    }

    fn reset(&mut self) {
        self.synth.reset();
        self.drone_voice.reset();
        self.sweep_phase = 0.0;
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let mut next_event = context.next_event();

        for (sample_id, mut channel_samples) in buffer.iter_samples().enumerate() {
            while let Some(event) = next_event {
                if event.timing() > sample_id as u32 {
                    break;
                }

                match event {
                    NoteEvent::NoteOn {
                        voice_id,
                        channel,
                        note,
                        velocity,
                        ..
                    } => {
                        self.synth.note_on(note, velocity, channel, voice_id);
                    }
                    NoteEvent::NoteOff {
                        voice_id,
                        channel,
                        note,
                        ..
                    } => {
                        self.synth.note_off(note, channel, voice_id);
                    }
                    NoteEvent::Choke {
                        voice_id,
                        channel,
                        note,
                        ..
                    } => {
                        self.synth.choke(note, channel, voice_id);
                    }
                    NoteEvent::PolyPressure {
                        voice_id,
                        channel,
                        note,
                        pressure,
                        ..
                    } => {
                        self.synth.pressure(note, channel, voice_id, pressure);
                    }
                    NoteEvent::PolyTuning {
                        voice_id,
                        channel,
                        note,
                        tuning,
                        ..
                    } => {
                        self.synth.tuning(note, channel, voice_id, tuning);
                    }
                    NoteEvent::MidiPitchBend { channel, value, .. } => {
                        self.synth.pitch_bend(channel, value);
                    }
                    NoteEvent::MidiCC {
                        channel,
                        cc: 64,
                        value,
                        ..
                    } => {
                        self.synth.sustain(channel, value >= 0.5);
                    }
                    _ => {}
                }

                next_event = context.next_event();
            }

            let sweep_rate = self.params.sweep_rate_hz.smoothed.next();
            self.sweep_phase += sweep_rate / self.sample_rate;
            if self.sweep_phase >= 1.0 {
                self.sweep_phase -= 1.0;
            }

            let voice_settings = VoiceSettings {
                waveform: self.params.wave.value().into(),
                frequency_hz: self.params.drone_frequency.smoothed.next(),
                pulse_width: self.params.pulse_width.smoothed.next(),
                center_hz: self.params.center_hz.smoothed.next(),
                spread_octaves: self.params.spread_octaves.smoothed.next(),
                mix: self.params.mix.smoothed.next(),
                sweep_phase: self.sweep_phase,
                keytrack: self.params.keytrack.smoothed.next(),
                stereo_offset: self.params.stereo_offset_octaves.smoothed.next(),
                cutoff_hz: self.params.cutoff_hz.smoothed.next(),
            };

            let effect = self.params.spectral_effect.value().into();
            let envelope = EnvelopeSettings {
                attack: self.params.attack.value(),
                decay: self.params.decay.value(),
                sustain: self.params.sustain.smoothed.next(),
                release: self.params.release.value(),
            };
            let resonance = self.params.resonance.smoothed.next();
            let filter_env = self.params.filter_env.smoothed.next();
            let (mut left, mut right) =
                self.synth
                    .render(voice_settings, effect, envelope, resonance, filter_env);

            if self.params.drone.value() {
                self.drone_voice
                    .configure(effect, envelope, resonance, filter_env);
                let (drone_l, drone_r) =
                    self.drone_voice
                        .render(voice_settings, self.sample_rate, true);
                left += drone_l * 0.65;
                right += drone_r * 0.65;
            }

            let gain = util::db_to_gain_fast(self.params.output_db.smoothed.next());
            left *= gain;
            right *= gain;

            for (channel_idx, sample) in channel_samples.iter_mut().enumerate() {
                *sample = match channel_idx {
                    0 => left,
                    1 => right,
                    _ => (left + right) * 0.5,
                };
            }
        }

        ProcessStatus::Normal
    }
}

impl ClapPlugin for PureVaDispersionCore {
    const CLAP_ID: &'static str = "com.prototypelab.kurv";
    const CLAP_DESCRIPTION: Option<&'static str> = Some(
        "KURV — polyphonic virtual-analog synthesis with oscillator-domain spectral morphing.",
    );
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
        ClapFeature::Stereo,
        ClapFeature::Mono,
    ];
}

impl Vst3Plugin for PureVaDispersionCore {
    const VST3_CLASS_ID: [u8; 16] = *b"KurvPluginCore01";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Instrument, Vst3SubCategory::Synth];
}

nice_export_clap!(PureVaDispersionCore);
nice_export_vst3!(PureVaDispersionCore);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_renders_silence_when_idle() {
        let mut voice = VaDispersionVoice::default();
        voice.set_sample_rate(48_000.0);
        let settings = VoiceSettings {
            waveform: Waveform::Saw,
            frequency_hz: 220.0,
            pulse_width: 0.5,
            center_hz: 800.0,
            spread_octaves: 2.0,
            mix: 0.5,
            sweep_phase: 0.0,
            keytrack: 0.0,
            stereo_offset: 0.0,
            cutoff_hz: 18_000.0,
        };

        let (l, r) = voice.render(settings, 48_000.0, false);
        assert_eq!(l, 0.0);
        assert_eq!(r, 0.0);
    }

    #[test]
    fn voice_renders_nonzero_in_drone_mode() {
        let mut voice = VaDispersionVoice::default();
        voice.set_sample_rate(48_000.0);
        let settings = VoiceSettings {
            waveform: Waveform::Saw,
            frequency_hz: 220.0,
            pulse_width: 0.5,
            center_hz: 800.0,
            spread_octaves: 2.0,
            mix: 0.7,
            sweep_phase: 0.0,
            keytrack: 0.2,
            stereo_offset: 0.05,
            cutoff_hz: 12_000.0,
        };

        let mut peak = 0.0_f32;
        for _ in 0..512 {
            let (l, r) = voice.render(settings, 48_000.0, true);
            peak = peak.max(l.abs()).max(r.abs());
        }

        assert!(peak.is_finite());
        assert!(peak > 0.01);
    }
}
