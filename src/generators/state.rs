//! Persisted editor/state-thread storage for generator stack patches.
//!
//! The editable document locks and allocates. Audio reads the separately
//! published fixed-capacity oscillator snapshot through atomics only.

use std::sync::{
    RwLock,
    atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering},
};

use truce::State;
use truce_core::custom_state::{PersistField, StateCursor, StateField};

use crate::oscillators::{VaTableData, VaTableState};
use crate::pan_curve::{PanShapeCurveData, PanShapeCurveState};

use super::{GroupOutput, MAX_OSCILLATORS, MAX_OUTPUT_PAIRS, ModuleKind, OscillatorSlot, Patch};

const INITIAL_STATE_VERSION: u32 = 1;
const SECOND_STATE_VERSION: u32 = 2;
const PREVIOUS_STATE_VERSION: u32 = 3;
const PAN_SHAPE_STATE_VERSION: u32 = 4;
const STATE_VERSION: u32 = 6;
const OSCILLATOR_KIND: u8 = 0;
// Old sessions had no generator document and encoded three fixed host
// oscillators. This mask is read only while that compatibility overlay is on.
const LEGACY_OSCILLATOR_MASK: u32 = 0b111;
const FILTER_KIND: u8 = 1;
const DEFAULT_UNISON_RATE: f32 = 0.417_432;

/// Non-host-exposed controls for one oscillator slot.
///
/// Every oscillator module is an instance of this same configuration. The
/// shell keeps old fixed host parameters outside this structural module path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OscillatorConfig {
    pub enabled: bool,
    pub shape: f32,
    pub custom_shape: f32,
    pub pulse_width: f32,
    pub transpose: f32,
    pub cents: f32,
    pub level: f32,
    pub pan: f32,
    pub unison_voices: u8,
    pub unison_range: f32,
    pub unison_amount: f32,
    pub unison_curve: f32,
    pub unison_jitter: f32,
    pub unison_jitter_mode: u8,
    pub unison_rate: f32,
    pub unison_width: f32,
    pub unison_weight: f32,
    pub phase_position: f32,
    pub phase_random: f32,
    pub phase_warp_mode: u8,
    pub phase_warp_amount: f32,
    pub unison_alignment: f32,
    pub unison_alignment_mode: u8,
    pub unison_pan_curve: f32,
    pub unison_pan_center_x: f32,
    pub unison_stereo_x: f32,
    pub unison_stereo_alternate: f32,
}

impl OscillatorConfig {
    fn sanitized(self) -> Self {
        Self {
            enabled: self.enabled,
            shape: finite_or(self.shape, 2.0).clamp(0.0, 3.0),
            custom_shape: finite_or(self.custom_shape, 0.0).clamp(0.0, 1.0),
            pulse_width: finite_or(self.pulse_width, 0.5).clamp(0.03, 0.97),
            transpose: finite_or(self.transpose, 0.0).clamp(-48.0, 48.0),
            cents: finite_or(self.cents, 0.0).clamp(-100.0, 100.0),
            level: finite_or(self.level, 0.5).clamp(0.0, 1.0),
            pan: finite_or(self.pan, 0.0).clamp(-1.0, 1.0),
            unison_voices: self.unison_voices.clamp(1, 64),
            unison_range: finite_or(self.unison_range, 1.0).clamp(0.0, 48.0),
            unison_amount: finite_or(self.unison_amount, 1.0).clamp(0.0, 1.0),
            unison_curve: finite_or(self.unison_curve, 0.432_959_4).clamp(-1.0, 1.0),
            unison_jitter: finite_or(self.unison_jitter, 0.0).clamp(0.0, 1.0),
            unison_jitter_mode: self.unison_jitter_mode.min(1),
            unison_rate: finite_or(self.unison_rate, DEFAULT_UNISON_RATE).clamp(0.0, 1.0),
            unison_width: finite_or(self.unison_width, 1.0).clamp(0.0, 1.0),
            unison_weight: finite_or(self.unison_weight, 0.0).clamp(-1.0, 1.0),
            phase_position: finite_or(self.phase_position, 0.0).clamp(0.0, 1.0),
            phase_random: finite_or(self.phase_random, 1.0).clamp(0.0, 1.0),
            phase_warp_mode: self.phase_warp_mode.min(3),
            phase_warp_amount: finite_or(self.phase_warp_amount, 0.0).clamp(0.0, 1.0),
            unison_alignment: finite_or(self.unison_alignment, 0.0).clamp(0.0, 1.0),
            unison_alignment_mode: self.unison_alignment_mode.min(3),
            unison_pan_curve: finite_or(self.unison_pan_curve, 0.0).clamp(-1.0, 1.0),
            unison_pan_center_x: finite_or(self.unison_pan_center_x, 0.5).clamp(0.05, 0.95),
            unison_stereo_x: finite_or(self.unison_stereo_x, 1.0).clamp(0.0, 1.0),
            unison_stereo_alternate: finite_or(self.unison_stereo_alternate, 0.0).clamp(0.0, 1.0),
        }
    }
}

impl Default for OscillatorConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            shape: 2.0,
            custom_shape: 0.0,
            pulse_width: 0.5,
            transpose: 0.0,
            cents: 0.0,
            level: 0.5,
            pan: 0.0,
            unison_voices: 1,
            unison_range: 1.0,
            unison_amount: 1.0,
            unison_curve: 0.432_959_4,
            unison_jitter: 0.0,
            unison_jitter_mode: 0,
            unison_rate: DEFAULT_UNISON_RATE,
            unison_width: 1.0,
            unison_weight: 0.0,
            phase_position: 0.0,
            phase_random: 1.0,
            phase_warp_mode: 0,
            phase_warp_amount: 0.0,
            unison_alignment: 0.0,
            unison_alignment_mode: 0,
            unison_pan_curve: 0.0,
            unison_pan_center_x: 0.5,
            unison_stereo_x: 1.0,
            unison_stereo_alternate: 0.0,
        }
    }
}

/// One ordered generator group's fixed audio-thread routing record.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeneratorRtGroup {
    oscillator_mask: u32,
    output: GroupOutput,
}

impl GeneratorRtGroup {
    const EMPTY: Self = Self {
        oscillator_mask: 0,
        output: GroupOutput {
            pair: 0,
            gain: 1.0,
            pan: 0.0,
            attack: 0.0,
            decay: 0.1,
            sustain: 1.0,
            release: 0.0,
        },
    };

    /// Oscillator slots owned by this group.
    #[must_use]
    pub const fn oscillator_mask(self) -> u32 {
        self.oscillator_mask
    }

    /// Shared mix and host-output destination for this group.
    #[must_use]
    pub const fn output(self) -> GroupOutput {
        self.output
    }
}

/// One coherent, fixed-capacity generator snapshot for the audio thread.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeneratorRtSnapshot {
    oscillators: [OscillatorConfig; MAX_OSCILLATORS],
    groups: [GeneratorRtGroup; MAX_OUTPUT_PAIRS],
    group_count: u8,
}

impl GeneratorRtSnapshot {
    /// All stable oscillator slots. `enabled` is false for slots outside every
    /// published group.
    #[must_use]
    pub const fn oscillators(&self) -> &[OscillatorConfig; MAX_OSCILLATORS] {
        &self.oscillators
    }

    /// Number of ordered groups in this snapshot.
    #[must_use]
    pub const fn group_count(&self) -> usize {
        self.group_count as usize
    }

    /// Ordered groups, excluding unused fixed-capacity storage.
    #[must_use]
    pub fn groups(&self) -> &[GeneratorRtGroup] {
        &self.groups[..self.group_count()]
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GeneratorStackSnapshot {
    patch: Patch,
    oscillators: [OscillatorConfig; MAX_OSCILLATORS],
    va_tables: [VaTableData; MAX_OSCILLATORS],
    pan_shape_curves: [PanShapeCurveData; MAX_OSCILLATORS],
}

impl GeneratorStackSnapshot {
    pub(crate) fn patch(&self) -> &Patch {
        &self.patch
    }
}

struct GeneratorDocument {
    patch: Patch,
    oscillators: [OscillatorConfig; MAX_OSCILLATORS],
}

impl Default for GeneratorDocument {
    fn default() -> Self {
        Self {
            patch: Patch::default(),
            oscillators: [OscillatorConfig::default(); MAX_OSCILLATORS],
        }
    }
}

struct RtOscillatorConfig {
    enabled: AtomicBool,
    shape: AtomicU32,
    custom_shape: AtomicU32,
    pulse_width: AtomicU32,
    transpose: AtomicU32,
    cents: AtomicU32,
    level: AtomicU32,
    pan: AtomicU32,
    unison_voices: AtomicU8,
    unison_range: AtomicU32,
    unison_amount: AtomicU32,
    unison_curve: AtomicU32,
    unison_jitter: AtomicU32,
    unison_jitter_mode: AtomicU8,
    unison_rate: AtomicU32,
    unison_width: AtomicU32,
    unison_weight: AtomicU32,
    phase_position: AtomicU32,
    phase_random: AtomicU32,
    phase_warp_mode: AtomicU8,
    phase_warp_amount: AtomicU32,
    unison_alignment: AtomicU32,
    unison_alignment_mode: AtomicU8,
    unison_pan_curve: AtomicU32,
    unison_pan_center_x: AtomicU32,
    unison_stereo_x: AtomicU32,
    unison_stereo_alternate: AtomicU32,
}

impl RtOscillatorConfig {
    fn new(config: OscillatorConfig) -> Self {
        Self {
            enabled: AtomicBool::new(config.enabled),
            shape: AtomicU32::new(config.shape.to_bits()),
            custom_shape: AtomicU32::new(config.custom_shape.to_bits()),
            pulse_width: AtomicU32::new(config.pulse_width.to_bits()),
            transpose: AtomicU32::new(config.transpose.to_bits()),
            cents: AtomicU32::new(config.cents.to_bits()),
            level: AtomicU32::new(config.level.to_bits()),
            pan: AtomicU32::new(config.pan.to_bits()),
            unison_voices: AtomicU8::new(config.unison_voices),
            unison_range: AtomicU32::new(config.unison_range.to_bits()),
            unison_amount: AtomicU32::new(config.unison_amount.to_bits()),
            unison_curve: AtomicU32::new(config.unison_curve.to_bits()),
            unison_jitter: AtomicU32::new(config.unison_jitter.to_bits()),
            unison_jitter_mode: AtomicU8::new(config.unison_jitter_mode),
            unison_rate: AtomicU32::new(config.unison_rate.to_bits()),
            unison_width: AtomicU32::new(config.unison_width.to_bits()),
            unison_weight: AtomicU32::new(config.unison_weight.to_bits()),
            phase_position: AtomicU32::new(config.phase_position.to_bits()),
            phase_random: AtomicU32::new(config.phase_random.to_bits()),
            phase_warp_mode: AtomicU8::new(config.phase_warp_mode),
            phase_warp_amount: AtomicU32::new(config.phase_warp_amount.to_bits()),
            unison_alignment: AtomicU32::new(config.unison_alignment.to_bits()),
            unison_alignment_mode: AtomicU8::new(config.unison_alignment_mode),
            unison_pan_curve: AtomicU32::new(config.unison_pan_curve.to_bits()),
            unison_pan_center_x: AtomicU32::new(config.unison_pan_center_x.to_bits()),
            unison_stereo_x: AtomicU32::new(config.unison_stereo_x.to_bits()),
            unison_stereo_alternate: AtomicU32::new(config.unison_stereo_alternate.to_bits()),
        }
    }

    fn store(&self, config: OscillatorConfig) {
        let config = config.sanitized();
        self.enabled.store(config.enabled, Ordering::Relaxed);
        self.shape.store(config.shape.to_bits(), Ordering::Relaxed);
        self.custom_shape
            .store(config.custom_shape.to_bits(), Ordering::Relaxed);
        self.pulse_width
            .store(config.pulse_width.to_bits(), Ordering::Relaxed);
        self.transpose
            .store(config.transpose.to_bits(), Ordering::Relaxed);
        self.cents.store(config.cents.to_bits(), Ordering::Relaxed);
        self.level.store(config.level.to_bits(), Ordering::Relaxed);
        self.pan.store(config.pan.to_bits(), Ordering::Relaxed);
        self.unison_voices
            .store(config.unison_voices, Ordering::Relaxed);
        self.unison_range
            .store(config.unison_range.to_bits(), Ordering::Relaxed);
        self.unison_amount
            .store(config.unison_amount.to_bits(), Ordering::Relaxed);
        self.unison_curve
            .store(config.unison_curve.to_bits(), Ordering::Relaxed);
        self.unison_jitter
            .store(config.unison_jitter.to_bits(), Ordering::Relaxed);
        self.unison_jitter_mode
            .store(config.unison_jitter_mode, Ordering::Relaxed);
        self.unison_rate
            .store(config.unison_rate.to_bits(), Ordering::Relaxed);
        self.unison_width
            .store(config.unison_width.to_bits(), Ordering::Relaxed);
        self.unison_weight
            .store(config.unison_weight.to_bits(), Ordering::Relaxed);
        self.phase_position
            .store(config.phase_position.to_bits(), Ordering::Relaxed);
        self.phase_random
            .store(config.phase_random.to_bits(), Ordering::Relaxed);
        self.phase_warp_mode
            .store(config.phase_warp_mode, Ordering::Relaxed);
        self.phase_warp_amount
            .store(config.phase_warp_amount.to_bits(), Ordering::Relaxed);
        self.unison_alignment
            .store(config.unison_alignment.to_bits(), Ordering::Relaxed);
        self.unison_alignment_mode
            .store(config.unison_alignment_mode, Ordering::Relaxed);
        self.unison_pan_curve
            .store(config.unison_pan_curve.to_bits(), Ordering::Relaxed);
        self.unison_pan_center_x
            .store(config.unison_pan_center_x.to_bits(), Ordering::Relaxed);
        self.unison_stereo_x
            .store(config.unison_stereo_x.to_bits(), Ordering::Relaxed);
        self.unison_stereo_alternate
            .store(config.unison_stereo_alternate.to_bits(), Ordering::Relaxed);
    }

    fn load(&self) -> OscillatorConfig {
        OscillatorConfig {
            enabled: self.enabled.load(Ordering::Relaxed),
            shape: f32::from_bits(self.shape.load(Ordering::Relaxed)),
            custom_shape: f32::from_bits(self.custom_shape.load(Ordering::Relaxed)),
            pulse_width: f32::from_bits(self.pulse_width.load(Ordering::Relaxed)),
            transpose: f32::from_bits(self.transpose.load(Ordering::Relaxed)),
            cents: f32::from_bits(self.cents.load(Ordering::Relaxed)),
            level: f32::from_bits(self.level.load(Ordering::Relaxed)),
            pan: f32::from_bits(self.pan.load(Ordering::Relaxed)),
            unison_voices: self.unison_voices.load(Ordering::Relaxed),
            unison_range: f32::from_bits(self.unison_range.load(Ordering::Relaxed)),
            unison_amount: f32::from_bits(self.unison_amount.load(Ordering::Relaxed)),
            unison_curve: f32::from_bits(self.unison_curve.load(Ordering::Relaxed)),
            unison_jitter: f32::from_bits(self.unison_jitter.load(Ordering::Relaxed)),
            unison_jitter_mode: self.unison_jitter_mode.load(Ordering::Relaxed),
            unison_rate: f32::from_bits(self.unison_rate.load(Ordering::Relaxed)),
            unison_width: f32::from_bits(self.unison_width.load(Ordering::Relaxed)),
            unison_weight: f32::from_bits(self.unison_weight.load(Ordering::Relaxed)),
            phase_position: f32::from_bits(self.phase_position.load(Ordering::Relaxed)),
            phase_random: f32::from_bits(self.phase_random.load(Ordering::Relaxed)),
            phase_warp_mode: self.phase_warp_mode.load(Ordering::Relaxed),
            phase_warp_amount: f32::from_bits(self.phase_warp_amount.load(Ordering::Relaxed)),
            unison_alignment: f32::from_bits(self.unison_alignment.load(Ordering::Relaxed)),
            unison_alignment_mode: self.unison_alignment_mode.load(Ordering::Relaxed),
            unison_pan_curve: f32::from_bits(self.unison_pan_curve.load(Ordering::Relaxed)),
            unison_pan_center_x: f32::from_bits(self.unison_pan_center_x.load(Ordering::Relaxed)),
            unison_stereo_x: f32::from_bits(self.unison_stereo_x.load(Ordering::Relaxed)),
            unison_stereo_alternate: f32::from_bits(
                self.unison_stereo_alternate.load(Ordering::Relaxed),
            ),
        }
    }
}

struct RtGroup {
    oscillator_mask: AtomicU32,
    output_pair: AtomicU8,
    output_gain: AtomicU32,
    output_pan: AtomicU32,
    output_attack: AtomicU32,
    output_decay: AtomicU32,
    output_sustain: AtomicU32,
    output_release: AtomicU32,
}

impl RtGroup {
    fn new() -> Self {
        Self {
            oscillator_mask: AtomicU32::new(0),
            output_pair: AtomicU8::new(0),
            output_gain: AtomicU32::new(1.0_f32.to_bits()),
            output_pan: AtomicU32::new(0.0_f32.to_bits()),
            output_attack: AtomicU32::new(0.0_f32.to_bits()),
            output_decay: AtomicU32::new(0.1_f32.to_bits()),
            output_sustain: AtomicU32::new(1.0_f32.to_bits()),
            output_release: AtomicU32::new(0.0_f32.to_bits()),
        }
    }

    fn store(&self, group: GeneratorRtGroup) {
        self.oscillator_mask
            .store(group.oscillator_mask, Ordering::Relaxed);
        self.output_pair.store(group.output.pair, Ordering::Relaxed);
        self.output_gain
            .store(group.output.gain.to_bits(), Ordering::Relaxed);
        self.output_pan
            .store(group.output.pan.to_bits(), Ordering::Relaxed);
        self.output_attack
            .store(group.output.attack.to_bits(), Ordering::Relaxed);
        self.output_decay
            .store(group.output.decay.to_bits(), Ordering::Relaxed);
        self.output_sustain
            .store(group.output.sustain.to_bits(), Ordering::Relaxed);
        self.output_release
            .store(group.output.release.to_bits(), Ordering::Relaxed);
    }

    fn load(&self) -> GeneratorRtGroup {
        GeneratorRtGroup {
            oscillator_mask: self.oscillator_mask.load(Ordering::Relaxed),
            output: GroupOutput {
                pair: self.output_pair.load(Ordering::Relaxed),
                gain: f32::from_bits(self.output_gain.load(Ordering::Relaxed)),
                pan: f32::from_bits(self.output_pan.load(Ordering::Relaxed)),
                attack: f32::from_bits(self.output_attack.load(Ordering::Relaxed)),
                decay: f32::from_bits(self.output_decay.load(Ordering::Relaxed)),
                sustain: f32::from_bits(self.output_sustain.load(Ordering::Relaxed)),
                release: f32::from_bits(self.output_release.load(Ordering::Relaxed)),
            },
        }
    }
}

#[derive(State)]
struct StackDocument {
    version: u32,
    next_group_id: u64,
    next_module_id: u64,
    materialized: bool,
    groups: Vec<GroupDocument>,
    oscillators: Vec<OscillatorDocument>,
    va_tables: Vec<VaTableData>,
    pan_shape_curves: Vec<PanShapeCurveData>,
}

impl Default for StackDocument {
    fn default() -> Self {
        Self {
            version: 0,
            next_group_id: 0,
            next_module_id: 0,
            materialized: false,
            groups: vec![GroupDocument::default()],
            oscillators: Vec::new(),
            va_tables: Vec::new(),
            pan_shape_curves: Vec::new(),
        }
    }
}

#[derive(State)]
struct GroupDocument {
    id: u64,
    modules: Vec<ModuleDocument>,
    output_pair: u8,
    output_gain: f32,
    output_pan: f32,
    // Appended for compatibility with Truce's legacy positional State blobs.
    output_attack: f32,
    output_decay: f32,
    output_sustain: f32,
    output_release: f32,
}

impl Default for GroupDocument {
    fn default() -> Self {
        Self {
            id: 0,
            modules: vec![ModuleDocument::default()],
            output_pair: 0,
            output_gain: 1.0,
            output_pan: 0.0,
            output_attack: 0.0,
            output_decay: 0.1,
            output_sustain: 1.0,
            output_release: 0.0,
        }
    }
}

#[derive(State)]
struct ModuleDocument {
    id: u64,
    kind: u8,
    oscillator_slot: u8,
}

impl Default for ModuleDocument {
    fn default() -> Self {
        Self {
            id: 0,
            kind: u8::MAX,
            oscillator_slot: u8::MAX,
        }
    }
}

#[derive(State)]
struct OscillatorDocument {
    enabled: bool,
    shape: f32,
    pulse_width: f32,
    transpose: f32,
    cents: f32,
    level: f32,
    pan: f32,
    unison_voices: u8,
    unison_range: f32,
    unison_amount: f32,
    unison_curve: f32,
    unison_jitter: f32,
    unison_rate: f32,
    unison_width: f32,
    // Appended for compatibility with Truce's legacy positional State blobs.
    custom_shape: f32,
    phase_position: f32,
    phase_random: f32,
    unison_alignment: f32,
    unison_alignment_mode: u8,
    unison_pan_curve: f32,
    unison_pan_center_x: f32,
    unison_stereo_x: f32,
    unison_stereo_alternate: f32,
    unison_jitter_mode: u8,
    unison_weight: f32,
    phase_warp_mode: u8,
    phase_warp_amount: f32,
}

impl Default for OscillatorDocument {
    fn default() -> Self {
        Self::from_config(OscillatorConfig::default())
    }
}

impl OscillatorDocument {
    fn from_config(config: OscillatorConfig) -> Self {
        Self {
            enabled: config.enabled,
            shape: config.shape,
            custom_shape: config.custom_shape,
            pulse_width: config.pulse_width,
            transpose: config.transpose,
            cents: config.cents,
            level: config.level,
            pan: config.pan,
            unison_voices: config.unison_voices,
            unison_range: config.unison_range,
            unison_amount: config.unison_amount,
            unison_curve: config.unison_curve,
            unison_jitter: config.unison_jitter,
            unison_jitter_mode: config.unison_jitter_mode,
            unison_rate: config.unison_rate,
            unison_width: config.unison_width,
            unison_weight: config.unison_weight,
            phase_position: config.phase_position,
            phase_random: config.phase_random,
            phase_warp_mode: config.phase_warp_mode,
            phase_warp_amount: config.phase_warp_amount,
            unison_alignment: config.unison_alignment,
            unison_alignment_mode: config.unison_alignment_mode,
            unison_pan_curve: config.unison_pan_curve,
            unison_pan_center_x: config.unison_pan_center_x,
            unison_stereo_x: config.unison_stereo_x,
            unison_stereo_alternate: config.unison_stereo_alternate,
        }
    }

    fn into_config(self) -> OscillatorConfig {
        OscillatorConfig {
            enabled: self.enabled,
            shape: self.shape,
            custom_shape: self.custom_shape,
            pulse_width: self.pulse_width,
            transpose: self.transpose,
            cents: self.cents,
            level: self.level,
            pan: self.pan,
            unison_voices: self.unison_voices,
            unison_range: self.unison_range,
            unison_amount: self.unison_amount,
            unison_curve: self.unison_curve,
            unison_jitter: self.unison_jitter,
            unison_jitter_mode: self.unison_jitter_mode,
            unison_rate: self.unison_rate,
            unison_width: self.unison_width,
            unison_weight: self.unison_weight,
            phase_position: self.phase_position,
            phase_random: self.phase_random,
            phase_warp_mode: self.phase_warp_mode,
            phase_warp_amount: self.phase_warp_amount,
            unison_alignment: self.unison_alignment,
            unison_alignment_mode: self.unison_alignment_mode,
            unison_pan_curve: self.unison_pan_curve,
            unison_pan_center_x: self.unison_pan_center_x,
            unison_stereo_x: self.unison_stereo_x,
            unison_stereo_alternate: self.unison_stereo_alternate,
        }
        .sanitized()
    }
}

impl StackDocument {
    fn from_document(
        document: &GeneratorDocument,
        va_tables: &[VaTableState; MAX_OSCILLATORS],
        pan_shape_curves: &[PanShapeCurveState; MAX_OSCILLATORS],
        materialized: bool,
    ) -> Self {
        let patch = &document.patch;
        Self {
            version: STATE_VERSION,
            next_group_id: patch.next_group_id(),
            next_module_id: patch.next_module_id(),
            materialized,
            groups: patch
                .groups()
                .iter()
                .map(|group| GroupDocument {
                    id: group.id().get(),
                    modules: group
                        .modules()
                        .iter()
                        .map(|module| ModuleDocument {
                            id: module.id().get(),
                            kind: match module.kind() {
                                ModuleKind::Oscillator(_) => OSCILLATOR_KIND,
                                ModuleKind::Filter => FILTER_KIND,
                            },
                            oscillator_slot: module
                                .oscillator_slot()
                                .map_or(0, OscillatorSlot::encoded),
                        })
                        .collect(),
                    output_pair: group.output().pair,
                    output_gain: group.output().gain,
                    output_pan: group.output().pan,
                    output_attack: group.output().attack,
                    output_decay: group.output().decay,
                    output_sustain: group.output().sustain,
                    output_release: group.output().release,
                })
                .collect(),
            oscillators: document
                .oscillators
                .iter()
                .copied()
                .map(OscillatorDocument::from_config)
                .collect(),
            va_tables: va_tables.iter().map(VaTableState::snapshot).collect(),
            pan_shape_curves: pan_shape_curves
                .iter()
                .map(PanShapeCurveState::snapshot)
                .collect(),
        }
    }

    fn into_document(
        self,
    ) -> Option<(
        GeneratorDocument,
        [VaTableData; MAX_OSCILLATORS],
        [PanShapeCurveData; MAX_OSCILLATORS],
        bool,
    )> {
        let version = self.version;
        if !matches!(
            version,
            INITIAL_STATE_VERSION
                | SECOND_STATE_VERSION
                | PREVIOUS_STATE_VERSION
                | PAN_SHAPE_STATE_VERSION
                | STATE_VERSION
        ) || self.next_group_id == 0
            || self.next_module_id == 0
        {
            return None;
        }

        let mut groups = Vec::with_capacity(self.groups.len());
        for group in self.groups {
            let mut modules = Vec::with_capacity(group.modules.len());
            for module in group.modules {
                let kind = match module.kind {
                    OSCILLATOR_KIND => ModuleKind::Oscillator(OscillatorSlot::from_index(
                        usize::from(module.oscillator_slot),
                    )?),
                    FILTER_KIND => ModuleKind::Filter,
                    _ => return None,
                };
                modules.push((module.id, kind));
            }
            groups.push((
                group.id,
                GroupOutput {
                    pair: group.output_pair,
                    gain: group.output_gain,
                    pan: group.output_pan,
                    attack: if version >= STATE_VERSION {
                        group.output_attack
                    } else {
                        GroupOutput::default().attack
                    },
                    decay: if version >= STATE_VERSION {
                        group.output_decay
                    } else {
                        GroupOutput::default().decay
                    },
                    sustain: if version >= STATE_VERSION {
                        group.output_sustain
                    } else {
                        GroupOutput::default().sustain
                    },
                    release: if version >= STATE_VERSION {
                        group.output_release
                    } else {
                        GroupOutput::default().release
                    },
                },
                modules,
            ));
        }

        let patch = Patch::restore(groups, self.next_group_id, self.next_module_id).ok()?;
        let mut oscillators = [OscillatorConfig::default(); MAX_OSCILLATORS];
        let defaults = OscillatorConfig::default();
        for (target, mut stored) in oscillators.iter_mut().zip(self.oscillators) {
            if version == INITIAL_STATE_VERSION {
                stored.phase_position = defaults.phase_position;
                stored.phase_random = defaults.phase_random;
                stored.unison_alignment = defaults.unison_alignment;
                stored.unison_alignment_mode = defaults.unison_alignment_mode;
                stored.unison_pan_curve = defaults.unison_pan_curve;
            }
            if matches!(version, INITIAL_STATE_VERSION | SECOND_STATE_VERSION) {
                stored.unison_stereo_x = defaults.unison_stereo_x;
                stored.unison_stereo_alternate = defaults.unison_stereo_alternate;
            }
            if version < PAN_SHAPE_STATE_VERSION {
                stored.unison_pan_center_x = defaults.unison_pan_center_x;
            }
            if version < STATE_VERSION {
                stored.unison_jitter_mode = defaults.unison_jitter_mode;
                stored.unison_weight = defaults.unison_weight;
                stored.phase_warp_mode = defaults.phase_warp_mode;
                stored.phase_warp_amount = defaults.phase_warp_amount;
            }
            *target = stored.into_config();
        }
        let mut va_tables = std::array::from_fn(|_| VaTableData::default());
        for (target, stored) in va_tables.iter_mut().zip(self.va_tables) {
            *target = stored;
        }
        let mut pan_shape_curves = std::array::from_fn(|index| {
            if version >= PAN_SHAPE_STATE_VERSION {
                PanShapeCurveData::default()
            } else {
                PanShapeCurveData::from_legacy(
                    0.0,
                    1.0,
                    1.0,
                    oscillators[index].unison_pan_curve,
                    oscillators[index].unison_pan_curve,
                    0.5,
                    0.5,
                )
            }
        });
        for (target, stored) in pan_shape_curves.iter_mut().zip(self.pan_shape_curves) {
            *target = stored;
        }
        Some((
            GeneratorDocument { patch, oscillators },
            va_tables,
            pan_shape_curves,
            self.materialized,
        ))
    }
}

/// Editable generator storage with a fixed lock-free audio snapshot.
pub struct GeneratorStackState {
    document: RwLock<GeneratorDocument>,
    va_tables: [VaTableState; MAX_OSCILLATORS],
    pan_shape_curves: [PanShapeCurveState; MAX_OSCILLATORS],
    materialized: AtomicBool,
    rt_generation: AtomicU32,
    rt_oscillators: [RtOscillatorConfig; MAX_OSCILLATORS],
    rt_group_count: AtomicU8,
    rt_groups: [RtGroup; MAX_OUTPUT_PAIRS],
}

impl GeneratorStackState {
    #[must_use]
    pub fn new() -> Self {
        let document = GeneratorDocument::default();
        let rt_groups = std::array::from_fn(|_| RtGroup::new());
        rt_groups[0].store(GeneratorRtGroup {
            oscillator_mask: 1,
            output: GroupOutput::default(),
        });
        Self {
            document: RwLock::new(document),
            va_tables: std::array::from_fn(|_| VaTableState::new()),
            pan_shape_curves: std::array::from_fn(|_| PanShapeCurveState::new()),
            materialized: AtomicBool::new(true),
            rt_generation: AtomicU32::new(0),
            rt_oscillators: std::array::from_fn(|_| {
                RtOscillatorConfig::new(OscillatorConfig::default())
            }),
            rt_group_count: AtomicU8::new(1),
            rt_groups,
        }
    }

    /// Whether the document has been reconciled with the legacy oscillator
    /// parameters. Old sessions can remain parameter-driven until this flips.
    #[must_use]
    pub fn is_materialized(&self) -> bool {
        self.materialized.load(Ordering::Acquire)
    }

    /// Clones the current editor-side patch.
    #[must_use]
    pub fn snapshot(&self) -> Patch {
        self.document
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .patch
            .clone()
    }

    #[must_use]
    pub fn oscillator_config(&self, slot: OscillatorSlot) -> OscillatorConfig {
        self.document
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .oscillators[slot.index()]
    }

    #[must_use]
    pub(crate) fn va_table(&self, slot: OscillatorSlot) -> &VaTableState {
        &self.va_tables[slot.index()]
    }

    #[must_use]
    pub(crate) fn pan_shape_curve(&self, slot: OscillatorSlot) -> &PanShapeCurveState {
        &self.pan_shape_curves[slot.index()]
    }

    pub fn set_oscillator_config(&self, slot: OscillatorSlot, config: OscillatorConfig) {
        let config = config.sanitized();
        let mut document = self
            .document
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if document.oscillators[slot.index()] == config {
            return;
        }
        document.oscillators[slot.index()] = config;
        self.publish_oscillator_rt(slot, config);
    }

    /// Publishes only one group output without rebuilding the oscillator bank.
    pub fn set_group_output(&self, group_id: super::GroupId, output: GroupOutput) -> bool {
        let output = output.sanitized();
        let mut document = self
            .document
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(index) = document
            .patch
            .groups()
            .iter()
            .position(|group| group.id() == group_id)
        else {
            return false;
        };
        if document.patch.groups()[index].output() == output {
            return false;
        }
        let _ = document.patch.set_group_output(group_id, output);
        let group = &document.patch.groups()[index];
        let rt_group = GeneratorRtGroup {
            oscillator_mask: oscillator_mask(group.modules()),
            output,
        };
        self.publish_group_rt(index, rt_group);
        true
    }

    /// Restores one reusable oscillator slot to the same state as a newly
    /// created module, including its VA table and pan-shape document.
    pub fn reset_oscillator(&self, slot: OscillatorSlot) {
        let mut document = self
            .document
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        document.oscillators[slot.index()] = OscillatorConfig::default();
        self.va_tables[slot.index()].replace(VaTableData::default());
        self.pan_shape_curves[slot.index()].replace(PanShapeCurveData::default());
        self.publish_oscillator_rt(slot, document.oscillators[slot.index()]);
    }

    /// Restores the complete structural generator area to the factory patch:
    /// one group, one oscillator, and default per-slot editor documents.
    pub fn reset_default(&self) {
        let mut document = self
            .document
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *document = GeneratorDocument::default();
        for table in &self.va_tables {
            table.replace(VaTableData::default());
        }
        for curve in &self.pan_shape_curves {
            curve.replace(PanShapeCurveData::default());
        }
        self.publish_rt(&document, true);
    }

    /// Attempts one bounded, allocation-free coherent read for the audio
    /// callback. Callers retain their previous snapshot on contention.
    #[must_use]
    pub fn try_rt_snapshot(&self) -> Option<GeneratorRtSnapshot> {
        self.try_rt_snapshot_after(u32::MAX)
            .map(|(_, snapshot)| snapshot)
    }

    /// Copies a coherent snapshot only when its published generation changed.
    #[must_use]
    pub fn try_rt_snapshot_after(
        &self,
        observed_generation: u32,
    ) -> Option<(u32, GeneratorRtSnapshot)> {
        let before = self.rt_generation.load(Ordering::Acquire);
        if before == observed_generation || before & 1 != 0 {
            return None;
        }
        let mut oscillators = [OscillatorConfig::default(); MAX_OSCILLATORS];
        let mut groups = [GeneratorRtGroup::EMPTY; MAX_OUTPUT_PAIRS];
        let materialized = self.materialized.load(Ordering::Relaxed);
        let group_count = if materialized {
            self.rt_group_count
                .load(Ordering::Relaxed)
                .min(MAX_OUTPUT_PAIRS as u8)
        } else {
            1
        };
        for (target, source) in groups.iter_mut().zip(&self.rt_groups) {
            *target = source.load();
        }
        if !materialized {
            groups[0].oscillator_mask = LEGACY_OSCILLATOR_MASK;
            groups[1..].fill(GeneratorRtGroup::EMPTY);
        }
        let active_mask = groups[..usize::from(group_count)]
            .iter()
            .fold(0, |mask, group| mask | group.oscillator_mask);
        for (index, (target, source)) in
            oscillators.iter_mut().zip(&self.rt_oscillators).enumerate()
        {
            *target = source.load();
            target.enabled &= active_mask & (1_u32 << index) != 0;
        }
        std::sync::atomic::fence(Ordering::Acquire);
        (before == self.rt_generation.load(Ordering::Relaxed)).then_some((
            before,
            GeneratorRtSnapshot {
                oscillators,
                groups,
                group_count,
            },
        ))
    }

    /// Edits the patch under its UI/state-thread write lock.
    pub fn edit<R>(&self, edit: impl FnOnce(&mut Patch) -> R) -> R {
        let mut document = self
            .document
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let result = edit(&mut document.patch);
        self.publish_rt(&document, true);
        result
    }

    pub(crate) fn history_snapshot(&self) -> GeneratorStackSnapshot {
        let document = self
            .document
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        GeneratorStackSnapshot {
            patch: document.patch.clone(),
            oscillators: document.oscillators,
            va_tables: std::array::from_fn(|index| self.va_tables[index].snapshot()),
            pan_shape_curves: std::array::from_fn(|index| self.pan_shape_curves[index].snapshot()),
        }
    }

    pub(crate) fn restore_snapshot(&self, snapshot: &GeneratorStackSnapshot) {
        let mut document = self
            .document
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        document.patch = snapshot.patch.clone();
        document.oscillators = snapshot.oscillators;
        for (state, data) in self.va_tables.iter().zip(&snapshot.va_tables) {
            state.replace(data.clone());
        }
        for (state, data) in self.pan_shape_curves.iter().zip(&snapshot.pan_shape_curves) {
            state.replace(data.clone());
        }
        self.publish_rt(&document, true);
    }

    pub(crate) fn reset_legacy(&self) {
        let mut document = self
            .document
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *document = GeneratorDocument::default();
        for table in &self.va_tables {
            table.replace(VaTableData::default());
        }
        for curve in &self.pan_shape_curves {
            curve.replace(PanShapeCurveData::default());
        }
        self.publish_rt(&document, false);
    }

    fn publish_rt(&self, document: &GeneratorDocument, materialized: bool) {
        self.rt_generation.fetch_add(1, Ordering::AcqRel);
        for (target, config) in self.rt_oscillators.iter().zip(document.oscillators) {
            target.store(config);
        }
        let groups = document.patch.groups();
        debug_assert!(groups.len() <= MAX_OUTPUT_PAIRS);
        for (index, target) in self.rt_groups.iter().enumerate() {
            let group =
                groups
                    .get(index)
                    .map_or(GeneratorRtGroup::EMPTY, |group| GeneratorRtGroup {
                        oscillator_mask: oscillator_mask(group.modules()),
                        output: group.output().sanitized(),
                    });
            target.store(group);
        }
        self.rt_group_count
            .store(groups.len().min(MAX_OUTPUT_PAIRS) as u8, Ordering::Relaxed);
        self.materialized.store(materialized, Ordering::Release);
        self.rt_generation.fetch_add(1, Ordering::Release);
    }

    fn publish_oscillator_rt(&self, slot: OscillatorSlot, config: OscillatorConfig) {
        self.rt_generation.fetch_add(1, Ordering::AcqRel);
        self.rt_oscillators[slot.index()].store(config);
        self.rt_generation.fetch_add(1, Ordering::Release);
    }

    fn publish_group_rt(&self, index: usize, group: GeneratorRtGroup) {
        if index >= MAX_OUTPUT_PAIRS {
            return;
        }
        self.rt_generation.fetch_add(1, Ordering::AcqRel);
        self.rt_groups[index].store(group);
        self.rt_generation.fetch_add(1, Ordering::Release);
    }
}

impl Default for GeneratorStackState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_output_publication_does_not_change_oscillator_configs() {
        let state = GeneratorStackState::new();
        let group_id = state.snapshot().groups()[0].id();
        let (generation, before) = state
            .try_rt_snapshot_after(u32::MAX)
            .expect("initial realtime snapshot");
        let output = GroupOutput {
            gain: 0.42,
            pan: -0.25,
            ..GroupOutput::default()
        };

        assert!(state.set_group_output(group_id, output));
        let (_, after) = state
            .try_rt_snapshot_after(generation)
            .expect("changed realtime snapshot");

        assert_eq!(before.oscillators(), after.oscillators());
        assert_eq!(after.groups()[0].output(), output);
    }
}

impl PersistField for GeneratorStackState {
    fn persist_write(&self, buf: &mut Vec<u8>) {
        let document = self
            .document
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        StackDocument::from_document(
            &document,
            &self.va_tables,
            &self.pan_shape_curves,
            self.is_materialized(),
        )
        .write_field(buf);
    }

    fn persist_read(&self, cursor: &mut StateCursor) {
        let Some((loaded, va_tables, pan_shape_curves, materialized)) =
            StackDocument::read_field(cursor).and_then(StackDocument::into_document)
        else {
            let document = self
                .document
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            self.publish_rt(&document, false);
            return;
        };
        let mut document = self
            .document
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *document = loaded;
        for (state, data) in self.va_tables.iter().zip(va_tables) {
            state.replace(data);
        }
        for (state, data) in self.pan_shape_curves.iter().zip(pan_shape_curves) {
            state.replace(data);
        }
        self.publish_rt(&document, materialized);
    }
}

fn oscillator_mask(modules: &[super::Module]) -> u32 {
    modules
        .iter()
        .filter_map(|module| module.oscillator_slot())
        .fold(0, |mask, slot| mask | (1_u32 << slot.index()))
}

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}
