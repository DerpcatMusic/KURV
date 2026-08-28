use std::sync::{
    Mutex,
    atomic::{AtomicU64, Ordering},
};

use truce::prelude::*;

#[path = "params/editor_state.rs"]
mod editor_state;
pub use editor_state::KurvEditorState;

#[path = "params/host_banks.rs"]
pub(crate) mod host_banks;

#[cfg(test)]
#[path = "params/group_envelope_tests.rs"]
mod group_envelope_tests;

#[cfg(test)]
#[path = "params/legacy_state_tests.rs"]
mod legacy_state_tests;

#[cfg(test)]
#[path = "params/xy_source_tests.rs"]
mod xy_source_tests;

const EDIT_TRACKER_WORDS: usize = 6;

/// Fixed, lock-free registry of host parameter gestures owned by the editor.
/// It is never touched by the audio thread; atomics make editor-close draining
/// race-free on Linux where baseview draws on a dedicated thread.
#[derive(Default)]
pub(crate) struct EditorEditTracker {
    active: [AtomicU64; EDIT_TRACKER_WORDS],
}

impl EditorEditTracker {
    #[cfg(test)]
    pub(crate) const fn capacity() -> usize {
        EDIT_TRACKER_WORDS * 64
    }

    pub(crate) fn begin(&self, id: u32) -> bool {
        let Ok(index) = usize::try_from(id) else {
            return false;
        };
        let (word, bit) = (index / 64, id % 64);
        self.active.get(word).is_some_and(|active| {
            active.fetch_or(1_u64 << bit, Ordering::AcqRel) & (1_u64 << bit) == 0
        })
    }

    pub(crate) fn end(&self, id: u32) -> bool {
        let Ok(index) = usize::try_from(id) else {
            return false;
        };
        let (word, bit) = (index / 64, id % 64);
        self.active.get(word).is_some_and(|active| {
            active.fetch_and(!(1_u64 << bit), Ordering::AcqRel) & (1_u64 << bit) != 0
        })
    }

    pub(crate) fn drain(&self, mut end: impl FnMut(u32)) -> usize {
        let mut count = 0;
        for (word, active) in self.active.iter().enumerate() {
            let mut bits = active.swap(0, Ordering::AcqRel);
            while bits != 0 {
                let bit = bits.trailing_zeros();
                let word = u32::try_from(word).unwrap_or(u32::MAX);
                end(word.saturating_mul(64).saturating_add(bit));
                bits &= bits - 1;
                count += 1;
            }
        }
        count
    }
}

use crate::{
    generators::GeneratorStackState,
    modulators::{
        routing::{
            ExtraModulationRouteState, HostAutomationTargetState, ModulationRouteTargetState,
        },
        state::ModulatorRackState,
    },
    pan_curve::PanShapeCurveState,
    resynth_state::ResynthAssetPackState,
    wave_curve::{LfoCurveState, WaveCurveState},
};

#[derive(Params)]
#[params(post_load = "reconcile_legacy_generator_stack")]
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
        format = "format_shape",
        parse = "parse_shape"
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
        default = 0.0,
        flags = "hidden | automatable"
    )]
    pub pitch_bend: FloatParam,

    #[param(
        id = 10,
        name = "Sustain Pedal",
        range = "linear(0, 1)",
        default = 0.0,
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
        id = 250,
        name = "Oscillator 1 Phase Position",
        short_name = "Osc 1 Position",
        range = "linear(0, 1)",
        default = 0.0,
        format = "format_phase_position"
    )]
    pub osc1_phase_position: FloatParam,

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
        format = "format_unison_weight",
        parse = "parse_unison_weight"
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
        format = "format_shape",
        parse = "parse_shape"
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
        format = "format_shape",
        parse = "parse_shape"
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
        id = 251,
        name = "Oscillator 2 Phase Position",
        short_name = "Osc 2 Position",
        range = "linear(0, 1)",
        default = 0.0,
        format = "format_phase_position"
    )]
    pub osc2_phase_position: FloatParam,

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
        format = "format_unison_weight",
        parse = "parse_unison_weight"
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
        id = 252,
        name = "Oscillator 3 Phase Position",
        short_name = "Osc 3 Position",
        range = "linear(0, 1)",
        default = 0.0,
        format = "format_phase_position"
    )]
    pub osc3_phase_position: FloatParam,

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
        format = "format_unison_weight",
        parse = "parse_unison_weight"
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
        default = 2,
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
    #[param(id = 126, name = "LFO 1 Bipolar", default = false)]
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
        range = "discrete(0, 64)",
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
        range = "discrete(0, 64)",
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
        range = "discrete(0, 64)",
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
        range = "discrete(0, 64)",
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
        range = "discrete(0, 64)",
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
        range = "discrete(0, 64)",
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
        range = "discrete(0, 64)",
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
        range = "discrete(0, 64)",
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
        default = 2,
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
        range = "discrete(0, 64)",
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
        range = "discrete(0, 64)",
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
        range = "discrete(0, 64)",
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
        range = "discrete(0, 64)",
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
        range = "discrete(0, 64)",
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
        range = "discrete(0, 64)",
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
        range = "discrete(0, 64)",
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
        range = "discrete(0, 64)",
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

    /// Dedicated, host-automatable XY modulation source. Both axes are
    /// unipolar and independent; route identity is persisted separately so
    /// existing `Mod N Source` parameter normalization remains unchanged.
    #[param(
        id = 366,
        name = "XY Source X",
        short_name = "XY X",
        range = "linear(0, 1)",
        default = 0.5,
        unit = "%",
        smooth = "linear(5)"
    )]
    pub xy_source_x: FloatParam,

    #[param(
        id = 367,
        name = "XY Source Y",
        short_name = "XY Y",
        range = "linear(0, 1)",
        default = 0.5,
        unit = "%",
        smooth = "linear(5)"
    )]
    pub xy_source_y: FloatParam,

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

    #[param(
        id = 253,
        name = "Source 1 Envelope",
        default = false,
        flags = "hidden | automatable"
    )]
    pub source1_envelope: BoolParam,
    #[param(
        id = 254,
        name = "Source 2 Envelope",
        default = false,
        flags = "hidden | automatable"
    )]
    pub source2_envelope: BoolParam,
    #[param(
        id = 255,
        name = "Source 3 Envelope",
        default = false,
        flags = "hidden | automatable"
    )]
    pub source3_envelope: BoolParam,
    #[param(
        id = 256,
        name = "Source 4 Envelope",
        default = false,
        flags = "hidden | automatable"
    )]
    pub source4_envelope: BoolParam,
    #[param(
        id = 257,
        name = "Source 5 Envelope",
        default = false,
        flags = "hidden | automatable"
    )]
    pub source5_envelope: BoolParam,
    #[param(
        id = 258,
        name = "Source 6 Envelope",
        default = false,
        flags = "hidden | automatable"
    )]
    pub source6_envelope: BoolParam,
    #[param(
        id = 259,
        name = "Source 7 Envelope",
        default = false,
        flags = "hidden | automatable"
    )]
    pub source7_envelope: BoolParam,
    #[param(
        id = 260,
        name = "Source 8 Envelope",
        default = false,
        flags = "hidden | automatable"
    )]
    pub source8_envelope: BoolParam,

    #[param(
        id = 261,
        name = "Source 1 Attack",
        range = "skewed(0, 8, 0.25)",
        default = 0.01,
        unit = "s",
        flags = "hidden | automatable"
    )]
    pub source1_attack: FloatParam,
    #[param(
        id = 262,
        name = "Source 1 Decay",
        range = "skewed(0, 8, 0.25)",
        default = 0.1,
        unit = "s",
        flags = "hidden | automatable"
    )]
    pub source1_decay: FloatParam,
    #[param(
        id = 263,
        name = "Source 1 Sustain",
        range = "linear(0, 1)",
        default = 0.8,
        unit = "%",
        flags = "hidden | automatable"
    )]
    pub source1_sustain: FloatParam,
    #[param(
        id = 264,
        name = "Source 1 Release",
        range = "skewed(0, 12, 0.25)",
        default = 0.2,
        unit = "s",
        flags = "hidden | automatable"
    )]
    pub source1_release: FloatParam,

    #[param(
        id = 265,
        name = "Source 2 Attack",
        range = "skewed(0, 8, 0.25)",
        default = 0.01,
        unit = "s",
        flags = "hidden | automatable"
    )]
    pub source2_attack: FloatParam,
    #[param(
        id = 266,
        name = "Source 2 Decay",
        range = "skewed(0, 8, 0.25)",
        default = 0.1,
        unit = "s",
        flags = "hidden | automatable"
    )]
    pub source2_decay: FloatParam,
    #[param(
        id = 267,
        name = "Source 2 Sustain",
        range = "linear(0, 1)",
        default = 0.8,
        unit = "%",
        flags = "hidden | automatable"
    )]
    pub source2_sustain: FloatParam,
    #[param(
        id = 268,
        name = "Source 2 Release",
        range = "skewed(0, 12, 0.25)",
        default = 0.2,
        unit = "s",
        flags = "hidden | automatable"
    )]
    pub source2_release: FloatParam,

    #[param(
        id = 269,
        name = "Source 3 Attack",
        range = "skewed(0, 8, 0.25)",
        default = 0.01,
        unit = "s",
        flags = "hidden | automatable"
    )]
    pub source3_attack: FloatParam,
    #[param(
        id = 270,
        name = "Source 3 Decay",
        range = "skewed(0, 8, 0.25)",
        default = 0.1,
        unit = "s",
        flags = "hidden | automatable"
    )]
    pub source3_decay: FloatParam,
    #[param(
        id = 271,
        name = "Source 3 Sustain",
        range = "linear(0, 1)",
        default = 0.8,
        unit = "%",
        flags = "hidden | automatable"
    )]
    pub source3_sustain: FloatParam,
    #[param(
        id = 272,
        name = "Source 3 Release",
        range = "skewed(0, 12, 0.25)",
        default = 0.2,
        unit = "s",
        flags = "hidden | automatable"
    )]
    pub source3_release: FloatParam,

    #[param(
        id = 273,
        name = "Source 4 Attack",
        range = "skewed(0, 8, 0.25)",
        default = 0.01,
        unit = "s",
        flags = "hidden | automatable"
    )]
    pub source4_attack: FloatParam,
    #[param(
        id = 274,
        name = "Source 4 Decay",
        range = "skewed(0, 8, 0.25)",
        default = 0.1,
        unit = "s",
        flags = "hidden | automatable"
    )]
    pub source4_decay: FloatParam,
    #[param(
        id = 275,
        name = "Source 4 Sustain",
        range = "linear(0, 1)",
        default = 0.8,
        unit = "%",
        flags = "hidden | automatable"
    )]
    pub source4_sustain: FloatParam,
    #[param(
        id = 276,
        name = "Source 4 Release",
        range = "skewed(0, 12, 0.25)",
        default = 0.2,
        unit = "s",
        flags = "hidden | automatable"
    )]
    pub source4_release: FloatParam,

    #[param(
        id = 277,
        name = "Source 5 Attack",
        range = "skewed(0, 8, 0.25)",
        default = 0.01,
        unit = "s",
        flags = "hidden | automatable"
    )]
    pub source5_attack: FloatParam,
    #[param(
        id = 278,
        name = "Source 5 Decay",
        range = "skewed(0, 8, 0.25)",
        default = 0.1,
        unit = "s",
        flags = "hidden | automatable"
    )]
    pub source5_decay: FloatParam,
    #[param(
        id = 279,
        name = "Source 5 Sustain",
        range = "linear(0, 1)",
        default = 0.8,
        unit = "%",
        flags = "hidden | automatable"
    )]
    pub source5_sustain: FloatParam,
    #[param(
        id = 280,
        name = "Source 5 Release",
        range = "skewed(0, 12, 0.25)",
        default = 0.2,
        unit = "s",
        flags = "hidden | automatable"
    )]
    pub source5_release: FloatParam,

    #[param(
        id = 281,
        name = "Source 6 Attack",
        range = "skewed(0, 8, 0.25)",
        default = 0.01,
        unit = "s",
        flags = "hidden | automatable"
    )]
    pub source6_attack: FloatParam,
    #[param(
        id = 282,
        name = "Source 6 Decay",
        range = "skewed(0, 8, 0.25)",
        default = 0.1,
        unit = "s",
        flags = "hidden | automatable"
    )]
    pub source6_decay: FloatParam,
    #[param(
        id = 283,
        name = "Source 6 Sustain",
        range = "linear(0, 1)",
        default = 0.8,
        unit = "%",
        flags = "hidden | automatable"
    )]
    pub source6_sustain: FloatParam,
    #[param(
        id = 284,
        name = "Source 6 Release",
        range = "skewed(0, 12, 0.25)",
        default = 0.2,
        unit = "s",
        flags = "hidden | automatable"
    )]
    pub source6_release: FloatParam,

    #[param(
        id = 285,
        name = "Source 7 Attack",
        range = "skewed(0, 8, 0.25)",
        default = 0.01,
        unit = "s",
        flags = "hidden | automatable"
    )]
    pub source7_attack: FloatParam,
    #[param(
        id = 286,
        name = "Source 7 Decay",
        range = "skewed(0, 8, 0.25)",
        default = 0.1,
        unit = "s",
        flags = "hidden | automatable"
    )]
    pub source7_decay: FloatParam,
    #[param(
        id = 287,
        name = "Source 7 Sustain",
        range = "linear(0, 1)",
        default = 0.8,
        unit = "%",
        flags = "hidden | automatable"
    )]
    pub source7_sustain: FloatParam,
    #[param(
        id = 288,
        name = "Source 7 Release",
        range = "skewed(0, 12, 0.25)",
        default = 0.2,
        unit = "s",
        flags = "hidden | automatable"
    )]
    pub source7_release: FloatParam,

    #[param(
        id = 289,
        name = "Source 8 Attack",
        range = "skewed(0, 8, 0.25)",
        default = 0.01,
        unit = "s",
        flags = "hidden | automatable"
    )]
    pub source8_attack: FloatParam,
    #[param(
        id = 290,
        name = "Source 8 Decay",
        range = "skewed(0, 8, 0.25)",
        default = 0.1,
        unit = "s",
        flags = "hidden | automatable"
    )]
    pub source8_decay: FloatParam,
    #[param(
        id = 291,
        name = "Source 8 Sustain",
        range = "linear(0, 1)",
        default = 0.8,
        unit = "%",
        flags = "hidden | automatable"
    )]
    pub source8_sustain: FloatParam,
    #[param(
        id = 292,
        name = "Source 8 Release",
        range = "skewed(0, 12, 0.25)",
        default = 0.2,
        unit = "s",
        flags = "hidden | automatable"
    )]
    pub source8_release: FloatParam,

    #[param(
        id = 293,
        name = "Host 01",
        short_name = "H01",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_01: FloatParam,

    #[param(
        id = 294,
        name = "Host 02",
        short_name = "H02",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_02: FloatParam,

    #[param(
        id = 295,
        name = "Host 03",
        short_name = "H03",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_03: FloatParam,

    #[param(
        id = 296,
        name = "Host 04",
        short_name = "H04",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_04: FloatParam,

    #[param(
        id = 297,
        name = "Host 05",
        short_name = "H05",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_05: FloatParam,

    #[param(
        id = 298,
        name = "Host 06",
        short_name = "H06",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_06: FloatParam,

    #[param(
        id = 299,
        name = "Host 07",
        short_name = "H07",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_07: FloatParam,

    #[param(
        id = 300,
        name = "Host 08",
        short_name = "H08",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_08: FloatParam,

    #[param(
        id = 301,
        name = "Host 09",
        short_name = "H09",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_09: FloatParam,

    #[param(
        id = 302,
        name = "Host 10",
        short_name = "H10",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_10: FloatParam,

    #[param(
        id = 303,
        name = "Host 11",
        short_name = "H11",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_11: FloatParam,

    #[param(
        id = 304,
        name = "Host 12",
        short_name = "H12",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_12: FloatParam,

    #[param(
        id = 305,
        name = "Host 13",
        short_name = "H13",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_13: FloatParam,

    #[param(
        id = 306,
        name = "Host 14",
        short_name = "H14",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_14: FloatParam,

    #[param(
        id = 307,
        name = "Host 15",
        short_name = "H15",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_15: FloatParam,

    #[param(
        id = 308,
        name = "Host 16",
        short_name = "H16",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_16: FloatParam,

    #[param(
        id = 309,
        name = "Host 17",
        short_name = "H17",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_17: FloatParam,

    #[param(
        id = 310,
        name = "Host 18",
        short_name = "H18",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_18: FloatParam,

    #[param(
        id = 311,
        name = "Host 19",
        short_name = "H19",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_19: FloatParam,

    #[param(
        id = 312,
        name = "Host 20",
        short_name = "H20",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_20: FloatParam,

    #[param(
        id = 313,
        name = "Host 21",
        short_name = "H21",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_21: FloatParam,

    #[param(
        id = 314,
        name = "Host 22",
        short_name = "H22",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_22: FloatParam,

    #[param(
        id = 315,
        name = "Host 23",
        short_name = "H23",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_23: FloatParam,

    #[param(
        id = 316,
        name = "Host 24",
        short_name = "H24",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_24: FloatParam,

    #[param(
        id = 317,
        name = "Host 25",
        short_name = "H25",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_25: FloatParam,

    #[param(
        id = 318,
        name = "Host 26",
        short_name = "H26",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_26: FloatParam,

    #[param(
        id = 319,
        name = "Host 27",
        short_name = "H27",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_27: FloatParam,

    #[param(
        id = 320,
        name = "Host 28",
        short_name = "H28",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_28: FloatParam,

    #[param(
        id = 321,
        name = "Host 29",
        short_name = "H29",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_29: FloatParam,

    #[param(
        id = 322,
        name = "Host 30",
        short_name = "H30",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_30: FloatParam,

    #[param(
        id = 323,
        name = "Host 31",
        short_name = "H31",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_31: FloatParam,

    #[param(
        id = 324,
        name = "Host 32",
        short_name = "H32",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_32: FloatParam,

    #[param(
        id = 325,
        name = "Host 33",
        short_name = "H33",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_33: FloatParam,

    #[param(
        id = 326,
        name = "Host 34",
        short_name = "H34",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_34: FloatParam,

    #[param(
        id = 327,
        name = "Host 35",
        short_name = "H35",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_35: FloatParam,

    #[param(
        id = 328,
        name = "Host 36",
        short_name = "H36",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_36: FloatParam,

    #[param(
        id = 329,
        name = "Host 37",
        short_name = "H37",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_37: FloatParam,

    #[param(
        id = 330,
        name = "Host 38",
        short_name = "H38",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_38: FloatParam,

    #[param(
        id = 331,
        name = "Host 39",
        short_name = "H39",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_39: FloatParam,

    #[param(
        id = 332,
        name = "Host 40",
        short_name = "H40",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_40: FloatParam,

    #[param(
        id = 333,
        name = "Host 41",
        short_name = "H41",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_41: FloatParam,

    #[param(
        id = 334,
        name = "Host 42",
        short_name = "H42",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_42: FloatParam,

    #[param(
        id = 335,
        name = "Host 43",
        short_name = "H43",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_43: FloatParam,

    #[param(
        id = 336,
        name = "Host 44",
        short_name = "H44",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_44: FloatParam,

    #[param(
        id = 337,
        name = "Host 45",
        short_name = "H45",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_45: FloatParam,

    #[param(
        id = 338,
        name = "Host 46",
        short_name = "H46",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_46: FloatParam,

    #[param(
        id = 339,
        name = "Host 47",
        short_name = "H47",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_47: FloatParam,

    #[param(
        id = 340,
        name = "Host 48",
        short_name = "H48",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_48: FloatParam,

    #[param(
        id = 341,
        name = "Host 49",
        short_name = "H49",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_49: FloatParam,

    #[param(
        id = 342,
        name = "Host 50",
        short_name = "H50",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_50: FloatParam,

    #[param(
        id = 343,
        name = "Host 51",
        short_name = "H51",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_51: FloatParam,

    #[param(
        id = 344,
        name = "Host 52",
        short_name = "H52",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_52: FloatParam,

    #[param(
        id = 345,
        name = "Host 53",
        short_name = "H53",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_53: FloatParam,

    #[param(
        id = 346,
        name = "Host 54",
        short_name = "H54",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_54: FloatParam,

    #[param(
        id = 347,
        name = "Host 55",
        short_name = "H55",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_55: FloatParam,

    #[param(
        id = 348,
        name = "Host 56",
        short_name = "H56",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_56: FloatParam,

    #[param(
        id = 349,
        name = "Host 57",
        short_name = "H57",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_57: FloatParam,

    #[param(
        id = 350,
        name = "Host 58",
        short_name = "H58",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_58: FloatParam,

    #[param(
        id = 351,
        name = "Host 59",
        short_name = "H59",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_59: FloatParam,

    #[param(
        id = 352,
        name = "Host 60",
        short_name = "H60",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_60: FloatParam,

    #[param(
        id = 353,
        name = "Host 61",
        short_name = "H61",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_61: FloatParam,

    #[param(
        id = 354,
        name = "Host 62",
        short_name = "H62",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_62: FloatParam,

    #[param(
        id = 355,
        name = "Host 63",
        short_name = "H63",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_63: FloatParam,

    #[param(
        id = 356,
        name = "Host 64",
        short_name = "H64",
        group = "HOST AUTOMATION",
        range = "linear(0, 1)",
        default = 0.0
    )]
    pub host_64: FloatParam,

    #[param(
        id = 357,
        name = "LFO 1 Shape",
        range = "discrete(0, 3)",
        default = 0,
        format = "format_lfo_shape"
    )]
    pub lfo1_shape: IntParam,
    #[param(
        id = 358,
        name = "LFO 2 Shape",
        range = "discrete(0, 3)",
        default = 0,
        format = "format_lfo_shape"
    )]
    pub lfo2_shape: IntParam,
    #[param(
        id = 359,
        name = "LFO 3 Shape",
        range = "discrete(0, 3)",
        default = 0,
        format = "format_lfo_shape"
    )]
    pub lfo3_shape: IntParam,
    #[param(
        id = 360,
        name = "LFO 4 Shape",
        range = "discrete(0, 3)",
        default = 0,
        format = "format_lfo_shape"
    )]
    pub lfo4_shape: IntParam,
    #[param(
        id = 361,
        name = "LFO 5 Shape",
        range = "discrete(0, 3)",
        default = 0,
        format = "format_lfo_shape"
    )]
    pub lfo5_shape: IntParam,
    #[param(
        id = 362,
        name = "LFO 6 Shape",
        range = "discrete(0, 3)",
        default = 0,
        format = "format_lfo_shape"
    )]
    pub lfo6_shape: IntParam,
    #[param(
        id = 363,
        name = "LFO 7 Shape",
        range = "discrete(0, 3)",
        default = 0,
        format = "format_lfo_shape"
    )]
    pub lfo7_shape: IntParam,
    #[param(
        id = 364,
        name = "LFO 8 Shape",
        range = "discrete(0, 3)",
        default = 0,
        format = "format_lfo_shape"
    )]
    pub lfo8_shape: IntParam,
    /// Hidden host-visible edge used only to mark changes to `#[persist]`
    /// documents dirty. Structural editor state has no ordinary parameter ID,
    /// so toggling this revision gives CLAP/VST3 hosts a save-worthy edit and
    /// uses their normal queued parameter flush path.
    #[param(
        id = 365,
        name = "State Revision",
        default = false,
        flags = "hidden | automatable"
    )]
    pub state_revision: BoolParam,

    /// Dynamic source slots 9..=64 live in custom state. Slots 1..=8 retain
    /// their existing host parameters and stable automation identities.
    #[persist = "modulator-rack"]
    pub modulator_rack: ModulatorRackState,

    /// Stable modular destinations for all internal routes. The first 16 retain
    /// their existing host parameter identities; overflow routes stay internal.
    #[persist = "modulation-route-targets"]
    pub modulation_route_targets: ModulationRouteTargetState,

    /// Source/depth storage for internal routes 17..=64. These deliberately do
    /// not add another 144 parameters to the host automation list.
    #[persist = "modulation-route-overflow"]
    pub modulation_route_overflow: ExtraModulationRouteState,

    /// Sidecar identity for Performance MOD routes. Source encodings remain
    /// 0..=64 so old hosts and presets retain their exact normalization.
    #[persist = "mod-wheel-route-mask"]
    pub mod_wheel_route_mask: AtomicCell<u64>,

    /// Route identities for the two dedicated XY outputs. They are sidecars
    /// rather than new source numbers to preserve all existing host automation.
    #[persist = "xy-source-x-route-mask"]
    pub xy_source_x_route_mask: AtomicCell<u64>,

    #[persist = "xy-source-y-route-mask"]
    pub xy_source_y_route_mask: AtomicCell<u64>,

    /// Fixed DAW automation slots mapped onto stable dynamic controls.
    #[persist = "host-automation-targets"]
    pub host_automation_targets: HostAutomationTargetState,

    /// Ordered generator modules are structural state, not a fixed bank of
    /// host parameters. Existing oscillator parameters remain stable while
    /// this document records which modules exist and how they are ordered.
    #[persist = "generator-stack"]
    #[persist_missing]
    pub generator_stack: GeneratorStackState,

    /// Byte-exact RESYNTH Source Masters, editable analysis cycles, and the
    /// current bounded RT artifacts. This host-thread persist lane is separate
    /// from the audio-thread `PluginLogic::load_state` extra-state path.
    #[persist = "resynth-asset-pack"]
    #[persist_missing]
    pub resynth_assets: ResynthAssetPackState,

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
    pub lfo1_curve_state: LfoCurveState,

    #[persist = "lfo2-curve"]
    pub lfo2_curve_state: LfoCurveState,

    #[persist = "lfo3-curve"]
    pub lfo3_curve_state: LfoCurveState,

    #[persist = "lfo4-curve"]
    pub lfo4_curve_state: LfoCurveState,

    #[persist = "lfo5-curve"]
    pub lfo5_curve_state: LfoCurveState,

    #[persist = "lfo6-curve"]
    pub lfo6_curve_state: LfoCurveState,

    #[persist = "lfo7-curve"]
    pub lfo7_curve_state: LfoCurveState,

    #[persist = "lfo8-curve"]
    pub lfo8_curve_state: LfoCurveState,

    #[persist = "editor-state"]
    pub editor_state: Mutex<KurvEditorState>,

    #[skip]
    pub(crate) editor_host_scale_bits: AtomicU64,

    #[skip]
    pub(crate) editor_dsp_sample_rate_bits: AtomicU64,

    #[skip]
    pub(crate) editor_persist_snapshot: Mutex<Vec<u8>>,

    #[skip]
    pub(crate) editor_edits: EditorEditTracker,

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

pub(crate) const HOST_AUTOMATION_PARAMS: [P; 64] = [
    P::Host01,
    P::Host02,
    P::Host03,
    P::Host04,
    P::Host05,
    P::Host06,
    P::Host07,
    P::Host08,
    P::Host09,
    P::Host10,
    P::Host11,
    P::Host12,
    P::Host13,
    P::Host14,
    P::Host15,
    P::Host16,
    P::Host17,
    P::Host18,
    P::Host19,
    P::Host20,
    P::Host21,
    P::Host22,
    P::Host23,
    P::Host24,
    P::Host25,
    P::Host26,
    P::Host27,
    P::Host28,
    P::Host29,
    P::Host30,
    P::Host31,
    P::Host32,
    P::Host33,
    P::Host34,
    P::Host35,
    P::Host36,
    P::Host37,
    P::Host38,
    P::Host39,
    P::Host40,
    P::Host41,
    P::Host42,
    P::Host43,
    P::Host44,
    P::Host45,
    P::Host46,
    P::Host47,
    P::Host48,
    P::Host49,
    P::Host50,
    P::Host51,
    P::Host52,
    P::Host53,
    P::Host54,
    P::Host55,
    P::Host56,
    P::Host57,
    P::Host58,
    P::Host59,
    P::Host60,
    P::Host61,
    P::Host62,
    P::Host63,
    P::Host64,
];

#[path = "params/formatters.rs"]
mod formatters;
pub(crate) use KurvParamsParamId as P;

impl KurvParams {
    pub(crate) fn set_editor_dsp_sample_rate(&self, sample_rate: f32) {
        self.editor_dsp_sample_rate_bits
            .store(f64::from(sample_rate.max(1.0)).to_bits(), Ordering::Relaxed);
    }

    pub(crate) fn editor_dsp_sample_rate(&self) -> f32 {
        let sample_rate =
            f64::from_bits(self.editor_dsp_sample_rate_bits.load(Ordering::Relaxed)) as f32;
        if sample_rate.is_finite() && sample_rate >= 1.0 {
            sample_rate
        } else {
            48_000.0
        }
    }

    /// Convert a pre-modular fixed-oscillator state into the editable generator
    /// document immediately after state restore. The fixed compatibility path
    /// is never persisted again: the next host save writes current schema v11.
    pub(crate) fn legacy_group_output(&self) -> crate::generators::GroupOutput {
        crate::generators::GroupOutput {
            attack: self.attack.value(),
            attack_curve: self.attack_curve.value(),
            attack_curve_time: self.attack_curve_time.value(),
            decay: self.decay.value(),
            decay_curve: self.decay_curve.value(),
            decay_curve_time: self.decay_curve_time.value(),
            sustain: self.sustain.value(),
            release: self.release.value(),
            release_curve: self.release_curve.value(),
            release_curve_time: self.release_curve_time.value(),
            ..crate::generators::GroupOutput::default()
        }
    }

    pub(crate) fn legacy_oscillator_configs(&self) -> [crate::generators::OscillatorConfig; 3] {
        let normalized_rate =
            |hz: f32| ((hz.max(0.02) / 0.02).ln() / 5_000.0_f32.ln()).clamp(0.0, 1.0);
        [
            crate::generators::OscillatorConfig {
                enabled: self.osc1_enabled.value(),
                engine: crate::generators::OscillatorEngineKind::Va,
                shape: self.shape.value(),
                custom_shape: self.osc1_custom_shape.value(),
                pulse_width: self.pulse_width.value(),
                transpose: self.osc1_transpose.value_f32(),
                cents: self.osc1_cents.value(),
                level: self.osc1_level.value(),
                pan: self.osc1_pan.value(),
                unison_voices: self.unison_voices.value_u8(),
                unison_range: self.unison_detune.value(),
                unison_amount: self.unison_detune_amount.value(),
                unison_curve: self.unison_curve.value(),
                unison_jitter: self.unison_swarm.value(),
                unison_jitter_mode: self.unison_swarm_mode.value_u8(),
                unison_rate: normalized_rate(self.unison_swarm_rate.value()),
                unison_width: self.unison_stereo.value(),
                unison_weight: self.unison_weight.value(),
                phase_position: self.osc1_phase_position.value(),
                phase_random: self.phase_random.value(),
                phase_warp_mode: self.osc1_warp_mode.value_u8(),
                phase_warp_amount: self.osc1_warp_amount.value(),
                phase_mod_source: 0,
                phase_mod_amount: 0.0,
                modulation_mode: crate::generators::GeneratorModMode::Phase,
                unison_alignment: self.unison_harmonic_align.value(),
                unison_alignment_mode: self.unison_alignment_mode.value_u8(),
                unison_pan_curve: self.pan_shape_curve.value(),
                unison_pan_center_x: self.pan_shape_center_x.value(),
                unison_stereo_x: self.stereo_x.value(),
                unison_stereo_alternate: self.stereo_alternate.value(),
            },
            crate::generators::OscillatorConfig {
                enabled: self.osc2_enabled.value(),
                engine: crate::generators::OscillatorEngineKind::Va,
                shape: self.osc2_shape.value(),
                custom_shape: self.osc2_custom_shape.value(),
                pulse_width: self.osc2_pulse_width.value(),
                transpose: self.osc2_transpose.value_f32(),
                cents: self.osc2_cents.value(),
                level: self.osc2_level.value(),
                pan: self.osc2_pan.value(),
                unison_voices: self.osc2_unison_voices.value_u8(),
                unison_range: self.osc2_unison_detune.value(),
                unison_amount: self.osc2_unison_detune_amount.value(),
                unison_curve: self.osc2_unison_curve.value(),
                unison_jitter: self.osc2_unison_jitter.value(),
                unison_jitter_mode: self.osc2_jitter_mode.value_u8(),
                unison_rate: normalized_rate(self.osc2_unison_jitter_rate.value()),
                unison_width: self.osc2_unison_stereo.value(),
                unison_weight: self.osc2_unison_weight.value(),
                phase_position: self.osc2_phase_position.value(),
                phase_random: self.osc2_phase_random.value(),
                phase_warp_mode: self.osc2_warp_mode.value_u8(),
                phase_warp_amount: self.osc2_warp_amount.value(),
                phase_mod_source: 0,
                phase_mod_amount: 0.0,
                modulation_mode: crate::generators::GeneratorModMode::Phase,
                unison_alignment: self.osc2_unison_harmonic_align.value(),
                unison_alignment_mode: self.osc2_unison_alignment_mode.value_u8(),
                unison_pan_curve: 0.0,
                unison_pan_center_x: self.osc2_pan_shape_center_x.value(),
                unison_stereo_x: self.osc2_stereo_x.value(),
                unison_stereo_alternate: self.osc2_stereo_alternate.value(),
            },
            crate::generators::OscillatorConfig {
                enabled: self.osc3_enabled.value(),
                engine: crate::generators::OscillatorEngineKind::Va,
                shape: self.osc3_shape.value(),
                custom_shape: self.osc3_custom_shape.value(),
                pulse_width: self.osc3_pulse_width.value(),
                transpose: self.osc3_transpose.value_f32(),
                cents: self.osc3_cents.value(),
                level: self.osc3_level.value(),
                pan: self.osc3_pan.value(),
                unison_voices: self.osc3_unison_voices.value_u8(),
                unison_range: self.osc3_unison_detune.value(),
                unison_amount: self.osc3_unison_detune_amount.value(),
                unison_curve: self.osc3_unison_curve.value(),
                unison_jitter: self.osc3_unison_jitter.value(),
                unison_jitter_mode: self.osc3_jitter_mode.value_u8(),
                unison_rate: normalized_rate(self.osc3_unison_jitter_rate.value()),
                unison_width: self.osc3_unison_stereo.value(),
                unison_weight: self.osc3_unison_weight.value(),
                phase_position: self.osc3_phase_position.value(),
                phase_random: self.osc3_phase_random.value(),
                phase_warp_mode: self.osc3_warp_mode.value_u8(),
                phase_warp_amount: self.osc3_warp_amount.value(),
                phase_mod_source: 0,
                phase_mod_amount: 0.0,
                modulation_mode: crate::generators::GeneratorModMode::Phase,
                unison_alignment: self.osc3_unison_harmonic_align.value(),
                unison_alignment_mode: self.osc3_unison_alignment_mode.value_u8(),
                unison_pan_curve: 0.0,
                unison_pan_center_x: self.osc3_pan_shape_center_x.value(),
                unison_stereo_x: self.osc3_stereo_x.value(),
                unison_stereo_alternate: self.osc3_stereo_alternate.value(),
            },
        ]
    }

    fn migrate_legacy_modulation_routes(&self) {
        use crate::modulation_target::{OscTarget, TargetKind, UnisonTarget};
        use crate::modulators::routing::{ModulationRouteTarget, OscillatorControl};

        self.modulator_rack.reset_to_default();
        self.modulation_route_targets.clear_all();
        self.modulation_route_overflow
            .restore_snapshot([crate::modulators::routing::ExtraModulationRoute::EMPTY; 48]);
        self.mod_wheel_route_mask.store(0);
        self.xy_source_x_route_mask.store(0);
        self.xy_source_y_route_mask.store(0);
        self.host_automation_targets.clear_all();
        let Some(snapshot) = self.generator_stack.try_rt_snapshot() else {
            return;
        };
        for (route, legacy) in crate::runtime::configuration::modulation_routes(self)
            .into_iter()
            .enumerate()
        {
            let Some(descriptor) = crate::modulation_target::descriptor(legacy.target) else {
                continue;
            };
            let (oscillator, control) = match descriptor.kind {
                TargetKind::Oscillator {
                    oscillator,
                    control,
                } => {
                    let control = match control {
                        OscTarget::Pitch => OscillatorControl::Transpose,
                        OscTarget::Shape => OscillatorControl::Shape,
                        OscTarget::PulseWidth => OscillatorControl::PulseWidth,
                        OscTarget::Warp => OscillatorControl::PhaseWarpAmount,
                        OscTarget::CustomShape => OscillatorControl::TablePosition,
                        OscTarget::Level => OscillatorControl::Level,
                        OscTarget::Pan => OscillatorControl::Pan,
                    };
                    (oscillator, control)
                }
                TargetKind::Unison {
                    oscillator,
                    control,
                } => {
                    let control = match control {
                        UnisonTarget::JitterAmount => OscillatorControl::UnisonJitter,
                        UnisonTarget::JitterRate => OscillatorControl::UnisonRate,
                        UnisonTarget::StereoX => OscillatorControl::UnisonStereoPosition,
                        UnisonTarget::StereoY => OscillatorControl::UnisonStereoAlternate,
                        _ => continue,
                    };
                    (oscillator, control)
                }
                TargetKind::Global(_) => continue,
            };
            let Some(slot) = crate::generators::OscillatorSlot::from_index(usize::from(oscillator))
            else {
                continue;
            };
            let Some(module_id) =
                crate::generators::ModuleId::from_raw(snapshot.module_ids()[slot.index()])
            else {
                continue;
            };
            let _ = self.modulation_route_targets.set(
                route,
                ModulationRouteTarget::oscillator(module_id, slot, control),
            );
        }
    }

    pub(crate) fn reconcile_legacy_generator_stack(&self) {
        if !self.generator_stack.legacy_migration_pending() {
            return;
        }
        let oscillators = self.legacy_oscillator_configs();
        let va_tables = [
            crate::oscillators::VaTableData {
                frames: vec![self.osc1_wave_curve_state.snapshot()],
                positions: Vec::new(),
            },
            crate::oscillators::VaTableData {
                frames: vec![self.osc2_wave_curve_state.snapshot()],
                positions: Vec::new(),
            },
            crate::oscillators::VaTableData {
                frames: vec![self.osc3_wave_curve_state.snapshot()],
                positions: Vec::new(),
            },
        ];
        let pan_shape_curves = [
            if self.pan_shape_curve_state.is_initialized() {
                self.pan_shape_curve_state.snapshot()
            } else {
                crate::pan_curve::PanShapeCurveData::from_legacy(
                    self.pan_shape_center.value(),
                    self.pan_shape_left.value(),
                    self.pan_shape_right.value(),
                    self.pan_shape_left_curve.value(),
                    self.pan_shape_right_curve.value(),
                    self.pan_shape_left_curve_time.value(),
                    self.pan_shape_right_curve_time.value(),
                )
            },
            if self.osc2_pan_shape_curve_state.is_initialized() {
                self.osc2_pan_shape_curve_state.snapshot()
            } else {
                crate::pan_curve::PanShapeCurveData::from_legacy(
                    self.osc2_pan_shape_center.value(),
                    self.osc2_pan_shape_left.value(),
                    self.osc2_pan_shape_right.value(),
                    self.osc2_pan_shape_left_curve.value(),
                    self.osc2_pan_shape_right_curve.value(),
                    self.osc2_pan_shape_left_curve_time.value(),
                    self.osc2_pan_shape_right_curve_time.value(),
                )
            },
            if self.osc3_pan_shape_curve_state.is_initialized() {
                self.osc3_pan_shape_curve_state.snapshot()
            } else {
                crate::pan_curve::PanShapeCurveData::from_legacy(
                    self.osc3_pan_shape_center.value(),
                    self.osc3_pan_shape_left.value(),
                    self.osc3_pan_shape_right.value(),
                    self.osc3_pan_shape_left_curve.value(),
                    self.osc3_pan_shape_right_curve.value(),
                    self.osc3_pan_shape_left_curve_time.value(),
                    self.osc3_pan_shape_right_curve_time.value(),
                )
            },
        ];
        let output = self.legacy_group_output();
        self.generator_stack
            .materialize_legacy(oscillators, output, va_tables, pan_shape_curves);
        self.migrate_legacy_modulation_routes();
    }
}
