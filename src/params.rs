use std::sync::{Mutex, atomic::AtomicU64};

use truce::prelude::*;

use crate::{pan_curve::PanShapeCurveState, wave_curve::WaveCurveState};

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
        range = "discrete(0, 63)",
        default = 0,
        flags = "hidden | automatable"
    )]
    pub mod1_target_ext: IntParam,
    #[param(
        id = 229,
        name = "Mod 2 Extended Target",
        range = "discrete(0, 63)",
        default = 0,
        flags = "hidden | automatable"
    )]
    pub mod2_target_ext: IntParam,
    #[param(
        id = 230,
        name = "Mod 3 Extended Target",
        range = "discrete(0, 63)",
        default = 0,
        flags = "hidden | automatable"
    )]
    pub mod3_target_ext: IntParam,
    #[param(
        id = 231,
        name = "Mod 4 Extended Target",
        range = "discrete(0, 63)",
        default = 0,
        flags = "hidden | automatable"
    )]
    pub mod4_target_ext: IntParam,
    #[param(
        id = 232,
        name = "Mod 5 Extended Target",
        range = "discrete(0, 63)",
        default = 0,
        flags = "hidden | automatable"
    )]
    pub mod5_target_ext: IntParam,
    #[param(
        id = 233,
        name = "Mod 6 Extended Target",
        range = "discrete(0, 63)",
        default = 0,
        flags = "hidden | automatable"
    )]
    pub mod6_target_ext: IntParam,
    #[param(
        id = 234,
        name = "Mod 7 Extended Target",
        range = "discrete(0, 63)",
        default = 0,
        flags = "hidden | automatable"
    )]
    pub mod7_target_ext: IntParam,
    #[param(
        id = 235,
        name = "Mod 8 Extended Target",
        range = "discrete(0, 63)",
        default = 0,
        flags = "hidden | automatable"
    )]
    pub mod8_target_ext: IntParam,
    #[param(
        id = 236,
        name = "Mod 9 Extended Target",
        range = "discrete(0, 63)",
        default = 0,
        flags = "hidden | automatable"
    )]
    pub mod9_target_ext: IntParam,
    #[param(
        id = 237,
        name = "Mod 10 Extended Target",
        range = "discrete(0, 63)",
        default = 0,
        flags = "hidden | automatable"
    )]
    pub mod10_target_ext: IntParam,
    #[param(
        id = 238,
        name = "Mod 11 Extended Target",
        range = "discrete(0, 63)",
        default = 0,
        flags = "hidden | automatable"
    )]
    pub mod11_target_ext: IntParam,
    #[param(
        id = 239,
        name = "Mod 12 Extended Target",
        range = "discrete(0, 63)",
        default = 0,
        flags = "hidden | automatable"
    )]
    pub mod12_target_ext: IntParam,
    #[param(
        id = 240,
        name = "Mod 13 Extended Target",
        range = "discrete(0, 63)",
        default = 0,
        flags = "hidden | automatable"
    )]
    pub mod13_target_ext: IntParam,
    #[param(
        id = 241,
        name = "Mod 14 Extended Target",
        range = "discrete(0, 63)",
        default = 0,
        flags = "hidden | automatable"
    )]
    pub mod14_target_ext: IntParam,
    #[param(
        id = 242,
        name = "Mod 15 Extended Target",
        range = "discrete(0, 63)",
        default = 0,
        flags = "hidden | automatable"
    )]
    pub mod15_target_ext: IntParam,
    #[param(
        id = 243,
        name = "Mod 16 Extended Target",
        range = "discrete(0, 63)",
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
    pub(crate) editor_host_scale_bits: AtomicU64,

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

pub(crate) use KurvParamsParamId as P;
