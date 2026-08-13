//! Persisted editor/state-thread storage for generator stack patches.
//!
//! The editable document locks and allocates. Audio reads the separately
//! published fixed-capacity oscillator snapshot through atomics only.

mod persistence;

use std::sync::{
    Arc, RwLock,
    atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering},
};

use crate::oscillators::{VaTableData, VaTableState};
use crate::pan_curve::{PanShapeCurveData, PanShapeCurveState};

use super::{
    FilterConfig, FilterMode, FilterSlot, GroupOutput, MAX_FILTERS, MAX_GENERATOR_MODULES,
    MAX_OSCILLATORS, MAX_OUTPUT_PAIRS, ModuleKind, OscillatorSlot, Patch,
};

// Old sessions had no generator document and encoded three fixed host
// oscillators. This mask is read only while that compatibility overlay is on.
const LEGACY_OSCILLATOR_MASK: u32 = 0b111;
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

/// One module in an audio-thread generator group's ordered program.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeneratorRtModule {
    Oscillator(OscillatorSlot),
    Filter(FilterSlot),
}

const EMPTY_RT_MODULE: GeneratorRtModule = GeneratorRtModule::Oscillator(OscillatorSlot::ZERO);

/// One ordered generator group's fixed audio-thread routing record.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeneratorRtGroup {
    id: u64,
    oscillator_mask: u32,
    modules: [GeneratorRtModule; MAX_GENERATOR_MODULES],
    module_count: u8,
    output: GroupOutput,
}

impl GeneratorRtGroup {
    pub(crate) const EMPTY: Self = Self {
        id: 0,
        oscillator_mask: 0,
        modules: [EMPTY_RT_MODULE; MAX_GENERATOR_MODULES],
        module_count: 0,
        output: GroupOutput {
            pair: 0,
            receive_midi_channel: 0,
            gain: 1.0,
            pan: 0.0,
            attack: 0.0,
            attack_curve: 0.0,
            decay: 0.1,
            decay_curve: 0.0,
            sustain: 1.0,
            release: 0.0,
            release_curve: 0.0,
        },
    };

    /// Stable group identity used to validate internal modulation routes.
    #[must_use]
    pub const fn id(self) -> u64 {
        self.id
    }

    /// Oscillator slots owned by this group.
    #[must_use]
    pub const fn oscillator_mask(self) -> u32 {
        self.oscillator_mask
    }

    /// Ordered oscillator/filter program for this group.
    #[must_use]
    pub fn modules(&self) -> &[GeneratorRtModule] {
        &self.modules[..usize::from(self.module_count)]
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
    filters: [FilterConfig; MAX_FILTERS],
    module_ids: [u64; MAX_OSCILLATORS],
    filter_module_ids: [u64; MAX_FILTERS],
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

    /// All stable filter slots.
    #[must_use]
    pub const fn filters(&self) -> &[FilterConfig; MAX_FILTERS] {
        &self.filters
    }

    /// Stable module identity occupying each oscillator slot, or zero when unused.
    #[must_use]
    pub const fn module_ids(&self) -> &[u64; MAX_OSCILLATORS] {
        &self.module_ids
    }

    /// Stable module identity occupying each filter slot, or zero when unused.
    #[must_use]
    pub const fn filter_module_ids(&self) -> &[u64; MAX_FILTERS] {
        &self.filter_module_ids
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
    filters: [FilterConfig; MAX_FILTERS],
    va_tables: [VaTableData; MAX_OSCILLATORS],
    pan_shape_curves: [PanShapeCurveData; MAX_OSCILLATORS],
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct GeneratorHistoryStamp {
    document: u32,
    va_tables: [u32; MAX_OSCILLATORS],
    pan_shape_curves: [u32; MAX_OSCILLATORS],
}

impl GeneratorStackSnapshot {
    pub(crate) fn patch(&self) -> &Patch {
        &self.patch
    }
}

struct GeneratorDocument {
    patch: Arc<Patch>,
    oscillators: [OscillatorConfig; MAX_OSCILLATORS],
    filters: [FilterConfig; MAX_FILTERS],
}

impl Default for GeneratorDocument {
    fn default() -> Self {
        Self {
            patch: Arc::new(Patch::default()),
            oscillators: [OscillatorConfig::default(); MAX_OSCILLATORS],
            filters: [FilterConfig::default(); MAX_FILTERS],
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

struct RtFilterConfig {
    mode: AtomicU8,
    cutoff_hz: AtomicU32,
    q: AtomicU32,
    slope_db_oct: AtomicU32,
    morph: AtomicU32,
}

impl RtFilterConfig {
    fn new(config: FilterConfig) -> Self {
        let config = sanitize_filter_config(config);
        Self {
            mode: AtomicU8::new(filter_mode_encoded(config.mode)),
            cutoff_hz: AtomicU32::new(config.cutoff_hz.to_bits()),
            q: AtomicU32::new(config.q.to_bits()),
            slope_db_oct: AtomicU32::new(config.slope_db_oct.to_bits()),
            morph: AtomicU32::new(config.morph.to_bits()),
        }
    }

    fn store(&self, config: FilterConfig) {
        let config = sanitize_filter_config(config);
        self.mode
            .store(filter_mode_encoded(config.mode), Ordering::Relaxed);
        self.cutoff_hz
            .store(config.cutoff_hz.to_bits(), Ordering::Relaxed);
        self.q.store(config.q.to_bits(), Ordering::Relaxed);
        self.slope_db_oct
            .store(config.slope_db_oct.to_bits(), Ordering::Relaxed);
        self.morph.store(config.morph.to_bits(), Ordering::Relaxed);
    }

    fn load(&self) -> FilterConfig {
        sanitize_filter_config(FilterConfig {
            mode: filter_mode_from_encoded(self.mode.load(Ordering::Relaxed)),
            cutoff_hz: f32::from_bits(self.cutoff_hz.load(Ordering::Relaxed)),
            q: f32::from_bits(self.q.load(Ordering::Relaxed)),
            slope_db_oct: f32::from_bits(self.slope_db_oct.load(Ordering::Relaxed)),
            morph: f32::from_bits(self.morph.load(Ordering::Relaxed)),
        })
    }
}

struct RtGroup {
    id: AtomicU64,
    oscillator_mask: AtomicU32,
    modules: [AtomicU8; MAX_GENERATOR_MODULES],
    module_count: AtomicU8,
    output_pair: AtomicU8,
    output_receive_midi_channel: AtomicU8,
    output_gain: AtomicU32,
    output_pan: AtomicU32,
    output_attack: AtomicU32,
    output_attack_curve: AtomicU32,
    output_decay: AtomicU32,
    output_decay_curve: AtomicU32,
    output_sustain: AtomicU32,
    output_release: AtomicU32,
    output_release_curve: AtomicU32,
}

impl RtGroup {
    fn new() -> Self {
        Self {
            id: AtomicU64::new(0),
            oscillator_mask: AtomicU32::new(0),
            modules: std::array::from_fn(|_| AtomicU8::new(0)),
            module_count: AtomicU8::new(0),
            output_pair: AtomicU8::new(0),
            output_receive_midi_channel: AtomicU8::new(0),
            output_gain: AtomicU32::new(1.0_f32.to_bits()),
            output_pan: AtomicU32::new(0.0_f32.to_bits()),
            output_attack: AtomicU32::new(0.0_f32.to_bits()),
            output_attack_curve: AtomicU32::new(0.0_f32.to_bits()),
            output_decay: AtomicU32::new(0.1_f32.to_bits()),
            output_decay_curve: AtomicU32::new(0.0_f32.to_bits()),
            output_sustain: AtomicU32::new(1.0_f32.to_bits()),
            output_release: AtomicU32::new(0.0_f32.to_bits()),
            output_release_curve: AtomicU32::new(0.0_f32.to_bits()),
        }
    }

    fn store(&self, group: GeneratorRtGroup) {
        self.id.store(group.id, Ordering::Relaxed);
        self.oscillator_mask
            .store(group.oscillator_mask, Ordering::Relaxed);
        for (target, module) in self.modules.iter().zip(group.modules) {
            target.store(encode_rt_module(module), Ordering::Relaxed);
        }
        self.module_count
            .store(group.module_count, Ordering::Relaxed);
        self.output_pair.store(group.output.pair, Ordering::Relaxed);
        self.output_receive_midi_channel
            .store(group.output.receive_midi_channel, Ordering::Relaxed);
        self.output_gain
            .store(group.output.gain.to_bits(), Ordering::Relaxed);
        self.output_pan
            .store(group.output.pan.to_bits(), Ordering::Relaxed);
        self.output_attack
            .store(group.output.attack.to_bits(), Ordering::Relaxed);
        self.output_attack_curve
            .store(group.output.attack_curve.to_bits(), Ordering::Relaxed);
        self.output_decay
            .store(group.output.decay.to_bits(), Ordering::Relaxed);
        self.output_decay_curve
            .store(group.output.decay_curve.to_bits(), Ordering::Relaxed);
        self.output_sustain
            .store(group.output.sustain.to_bits(), Ordering::Relaxed);
        self.output_release
            .store(group.output.release.to_bits(), Ordering::Relaxed);
        self.output_release_curve
            .store(group.output.release_curve.to_bits(), Ordering::Relaxed);
    }

    fn store_output(&self, output: GroupOutput) {
        let output = output.sanitized();
        self.output_pair.store(output.pair, Ordering::Relaxed);
        self.output_receive_midi_channel
            .store(output.receive_midi_channel, Ordering::Relaxed);
        self.output_gain
            .store(output.gain.to_bits(), Ordering::Relaxed);
        self.output_pan
            .store(output.pan.to_bits(), Ordering::Relaxed);
        self.output_attack
            .store(output.attack.to_bits(), Ordering::Relaxed);
        self.output_attack_curve
            .store(output.attack_curve.to_bits(), Ordering::Relaxed);
        self.output_decay
            .store(output.decay.to_bits(), Ordering::Relaxed);
        self.output_decay_curve
            .store(output.decay_curve.to_bits(), Ordering::Relaxed);
        self.output_sustain
            .store(output.sustain.to_bits(), Ordering::Relaxed);
        self.output_release
            .store(output.release.to_bits(), Ordering::Relaxed);
        self.output_release_curve
            .store(output.release_curve.to_bits(), Ordering::Relaxed);
    }

    fn load(&self) -> GeneratorRtGroup {
        let mut modules = [EMPTY_RT_MODULE; MAX_GENERATOR_MODULES];
        for (target, source) in modules.iter_mut().zip(&self.modules) {
            *target = decode_rt_module(source.load(Ordering::Relaxed));
        }
        GeneratorRtGroup {
            id: self.id.load(Ordering::Relaxed),
            oscillator_mask: self.oscillator_mask.load(Ordering::Relaxed),
            modules,
            module_count: self
                .module_count
                .load(Ordering::Relaxed)
                .min(MAX_GENERATOR_MODULES as u8),
            output: self.load_output(),
        }
    }

    fn load_output(&self) -> GroupOutput {
        GroupOutput {
            pair: self.output_pair.load(Ordering::Relaxed),
            receive_midi_channel: self.output_receive_midi_channel.load(Ordering::Relaxed),
            gain: f32::from_bits(self.output_gain.load(Ordering::Relaxed)),
            pan: f32::from_bits(self.output_pan.load(Ordering::Relaxed)),
            attack: f32::from_bits(self.output_attack.load(Ordering::Relaxed)),
            attack_curve: f32::from_bits(self.output_attack_curve.load(Ordering::Relaxed)),
            decay: f32::from_bits(self.output_decay.load(Ordering::Relaxed)),
            decay_curve: f32::from_bits(self.output_decay_curve.load(Ordering::Relaxed)),
            sustain: f32::from_bits(self.output_sustain.load(Ordering::Relaxed)),
            release: f32::from_bits(self.output_release.load(Ordering::Relaxed)),
            release_curve: f32::from_bits(self.output_release_curve.load(Ordering::Relaxed)),
        }
    }
}

/// Editable generator storage with a fixed lock-free audio snapshot.
pub struct GeneratorStackState {
    document: RwLock<GeneratorDocument>,
    va_tables: [VaTableState; MAX_OSCILLATORS],
    pan_shape_curves: [PanShapeCurveState; MAX_OSCILLATORS],
    materialized: AtomicBool,
    rt_generation: AtomicU32,
    rt_topology_generation: AtomicU32,
    rt_oscillator_generation: AtomicU32,
    rt_oscillator_generations: [AtomicU32; MAX_OSCILLATORS],
    rt_filter_generation: AtomicU32,
    rt_filter_generations: [AtomicU32; MAX_FILTERS],
    rt_group_output_generation: AtomicU32,
    rt_group_output_generations: [AtomicU32; MAX_OUTPUT_PAIRS],
    rt_oscillators: [RtOscillatorConfig; MAX_OSCILLATORS],
    rt_filters: [RtFilterConfig; MAX_FILTERS],
    rt_module_ids: [AtomicU64; MAX_OSCILLATORS],
    rt_filter_module_ids: [AtomicU64; MAX_FILTERS],
    rt_group_count: AtomicU8,
    rt_groups: [RtGroup; MAX_OUTPUT_PAIRS],
}

impl GeneratorStackState {
    #[must_use]
    pub fn new() -> Self {
        let document = GeneratorDocument::default();
        let rt_groups = std::array::from_fn(|_| RtGroup::new());
        rt_groups[0].store(generator_rt_group(&document.patch.groups()[0]));
        Self {
            document: RwLock::new(document),
            va_tables: std::array::from_fn(|_| VaTableState::new()),
            pan_shape_curves: std::array::from_fn(|_| PanShapeCurveState::new()),
            materialized: AtomicBool::new(true),
            rt_generation: AtomicU32::new(0),
            rt_topology_generation: AtomicU32::new(0),
            rt_oscillator_generation: AtomicU32::new(0),
            rt_oscillator_generations: std::array::from_fn(|_| AtomicU32::new(0)),
            rt_filter_generation: AtomicU32::new(0),
            rt_filter_generations: std::array::from_fn(|_| AtomicU32::new(0)),
            rt_group_output_generation: AtomicU32::new(0),
            rt_group_output_generations: std::array::from_fn(|_| AtomicU32::new(0)),
            rt_oscillators: std::array::from_fn(|_| {
                RtOscillatorConfig::new(OscillatorConfig::default())
            }),
            rt_filters: std::array::from_fn(|_| RtFilterConfig::new(FilterConfig::default())),
            rt_module_ids: std::array::from_fn(|index| AtomicU64::new(u64::from(index == 0))),
            rt_filter_module_ids: std::array::from_fn(|_| AtomicU64::new(0)),
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

    /// Returns a cheap immutable editor-side patch snapshot.
    #[must_use]
    pub fn snapshot(&self) -> Arc<Patch> {
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
    pub fn filter_config(&self, slot: FilterSlot) -> FilterConfig {
        self.document
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .filters[slot.index()]
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

    pub fn set_filter_config(&self, slot: FilterSlot, config: FilterConfig) {
        let config = sanitize_filter_config(config);
        let mut document = self
            .document
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if document.filters[slot.index()] == config {
            return;
        }
        document.filters[slot.index()] = config;
        self.publish_filter_rt(slot, config);
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
        let _ = Arc::make_mut(&mut document.patch).set_group_output(group_id, output);
        self.publish_group_output_rt(group_id.get(), output);
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
        self.try_rt_snapshot_after_generation(observed_generation, &self.rt_generation)
    }

    /// Copies the complete fixed topology only when a structural edit changed it.
    #[must_use]
    pub(crate) fn try_rt_topology_snapshot_after(
        &self,
        observed_generation: u32,
    ) -> Option<(u32, GeneratorRtSnapshot)> {
        self.try_rt_snapshot_after_generation(observed_generation, &self.rt_topology_generation)
    }

    #[must_use]
    pub(crate) fn group_output_rt_generation(&self) -> u32 {
        self.rt_group_output_generation.load(Ordering::Acquire)
    }

    #[must_use]
    pub(crate) fn oscillator_rt_generation(&self) -> u32 {
        self.rt_oscillator_generation.load(Ordering::Acquire)
    }

    #[must_use]
    pub(crate) fn filter_rt_generation(&self) -> u32 {
        self.rt_filter_generation.load(Ordering::Acquire)
    }

    #[must_use]
    pub(crate) fn rt_coherence_generation(&self) -> u32 {
        self.rt_generation.load(Ordering::Acquire)
    }

    #[must_use]
    pub(crate) fn try_oscillator_rt_after(
        &self,
        slot: OscillatorSlot,
        observed_generation: u32,
    ) -> Option<(u32, OscillatorConfig)> {
        self.try_rt_value_after(
            &self.rt_oscillator_generations[slot.index()],
            observed_generation,
            || self.rt_oscillators[slot.index()].load(),
        )
    }

    #[must_use]
    pub(crate) fn try_filter_rt_after(
        &self,
        slot: FilterSlot,
        observed_generation: u32,
    ) -> Option<(u32, FilterConfig)> {
        self.try_rt_value_after(
            &self.rt_filter_generations[slot.index()],
            observed_generation,
            || self.rt_filters[slot.index()].load(),
        )
    }

    #[must_use]
    pub(crate) fn try_group_output_rt_after(
        &self,
        index: usize,
        observed_generation: u32,
    ) -> Option<(u32, u64, GroupOutput)> {
        let group = self.rt_groups.get(index)?;
        self.try_rt_value_after(
            &self.rt_group_output_generations[index],
            observed_generation,
            || (group.id.load(Ordering::Relaxed), group.load_output()),
        )
        .map(|(generation, (group_id, output))| (generation, group_id, output))
    }

    fn try_rt_value_after<T>(
        &self,
        published_generation: &AtomicU32,
        observed_generation: u32,
        load: impl FnOnce() -> T,
    ) -> Option<(u32, T)> {
        let generation = published_generation.load(Ordering::Acquire);
        if generation == observed_generation {
            return None;
        }
        let before = self.rt_generation.load(Ordering::Acquire);
        if before & 1 != 0 {
            return None;
        }
        let value = load();
        std::sync::atomic::fence(Ordering::Acquire);
        (before == self.rt_generation.load(Ordering::Relaxed)
            && generation == published_generation.load(Ordering::Relaxed))
        .then_some((generation, value))
    }

    fn try_rt_snapshot_after_generation(
        &self,
        observed_generation: u32,
        published_generation: &AtomicU32,
    ) -> Option<(u32, GeneratorRtSnapshot)> {
        let generation = published_generation.load(Ordering::Acquire);
        if generation == observed_generation {
            return None;
        }
        let before = self.rt_generation.load(Ordering::Acquire);
        if before & 1 != 0 {
            return None;
        }
        let mut oscillators = [OscillatorConfig::default(); MAX_OSCILLATORS];
        let mut filters = [FilterConfig::default(); MAX_FILTERS];
        let mut module_ids = [0_u64; MAX_OSCILLATORS];
        let mut filter_module_ids = [0_u64; MAX_FILTERS];
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
            groups[0].modules[..3].copy_from_slice(&[
                GeneratorRtModule::Oscillator(OscillatorSlot::from_index(0)?),
                GeneratorRtModule::Oscillator(OscillatorSlot::from_index(1)?),
                GeneratorRtModule::Oscillator(OscillatorSlot::from_index(2)?),
            ]);
            groups[0].module_count = 3;
            groups[1..].fill(GeneratorRtGroup::EMPTY);
        }
        let active_mask = groups[..usize::from(group_count)]
            .iter()
            .fold(0, |mask, group| mask | group.oscillator_mask);
        for (index, (target, source)) in
            oscillators.iter_mut().zip(&self.rt_oscillators).enumerate()
        {
            *target = source.load();
            module_ids[index] = self.rt_module_ids[index].load(Ordering::Relaxed);
            target.enabled &= active_mask & (1_u32 << index) != 0;
        }
        for (index, (target, source)) in filters.iter_mut().zip(&self.rt_filters).enumerate() {
            *target = source.load();
            filter_module_ids[index] = self.rt_filter_module_ids[index].load(Ordering::Relaxed);
        }
        std::sync::atomic::fence(Ordering::Acquire);
        (before == self.rt_generation.load(Ordering::Relaxed)
            && generation == published_generation.load(Ordering::Relaxed))
        .then_some((
            generation,
            GeneratorRtSnapshot {
                oscillators,
                filters,
                module_ids,
                filter_module_ids,
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
        let result = edit(Arc::make_mut(&mut document.patch));
        self.publish_rt(&document, true);
        result
    }

    pub(crate) fn history_snapshot(&self) -> GeneratorStackSnapshot {
        let document = self
            .document
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        GeneratorStackSnapshot {
            patch: document.patch.as_ref().clone(),
            oscillators: document.oscillators,
            filters: document.filters,
            va_tables: std::array::from_fn(|index| self.va_tables[index].snapshot()),
            pan_shape_curves: std::array::from_fn(|index| self.pan_shape_curves[index].snapshot()),
        }
    }

    pub(crate) fn history_stamp(&self) -> GeneratorHistoryStamp {
        GeneratorHistoryStamp {
            document: self.rt_generation.load(Ordering::Acquire),
            va_tables: std::array::from_fn(|index| self.va_tables[index].history_generation()),
            pan_shape_curves: std::array::from_fn(|index| {
                self.pan_shape_curves[index].history_generation()
            }),
        }
    }

    pub(crate) fn restore_snapshot(&self, snapshot: &GeneratorStackSnapshot) {
        let mut document = self
            .document
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        document.patch = Arc::new(snapshot.patch.clone());
        document.oscillators = snapshot.oscillators;
        document.filters = snapshot.filters;
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
        for (target, config) in self.rt_filters.iter().zip(document.filters) {
            target.store(config);
        }
        for target in &self.rt_module_ids {
            target.store(0, Ordering::Relaxed);
        }
        for target in &self.rt_filter_module_ids {
            target.store(0, Ordering::Relaxed);
        }
        let groups = document.patch.groups();
        debug_assert!(groups.len() <= MAX_OUTPUT_PAIRS);
        let mut rt_group_count = 0;
        for group in groups {
            let group = generator_rt_group(group);
            if group.oscillator_mask() == 0 {
                continue;
            }
            self.rt_groups[rt_group_count].store(group);
            rt_group_count += 1;
        }
        for target in &self.rt_groups[rt_group_count..] {
            target.store(GeneratorRtGroup::EMPTY);
        }
        for module in groups.iter().flat_map(|group| group.modules()) {
            if let Some(slot) = module.oscillator_slot() {
                self.rt_module_ids[slot.index()].store(module.id().get(), Ordering::Relaxed);
            } else if let Some(slot) = module.filter_slot() {
                self.rt_filter_module_ids[slot.index()].store(module.id().get(), Ordering::Relaxed);
            }
        }
        self.rt_group_count
            .store(rt_group_count as u8, Ordering::Relaxed);
        self.materialized.store(materialized, Ordering::Release);
        self.rt_topology_generation.fetch_add(1, Ordering::Relaxed);
        self.rt_generation.fetch_add(1, Ordering::Release);
    }

    fn publish_oscillator_rt(&self, slot: OscillatorSlot, config: OscillatorConfig) {
        self.rt_generation.fetch_add(1, Ordering::AcqRel);
        self.rt_oscillators[slot.index()].store(config);
        self.rt_oscillator_generations[slot.index()].fetch_add(1, Ordering::Relaxed);
        self.rt_generation.fetch_add(1, Ordering::Release);
        self.rt_oscillator_generation
            .fetch_add(1, Ordering::Release);
    }

    fn publish_filter_rt(&self, slot: FilterSlot, config: FilterConfig) {
        self.rt_generation.fetch_add(1, Ordering::AcqRel);
        self.rt_filters[slot.index()].store(config);
        self.rt_filter_generations[slot.index()].fetch_add(1, Ordering::Relaxed);
        self.rt_generation.fetch_add(1, Ordering::Release);
        self.rt_filter_generation.fetch_add(1, Ordering::Release);
    }

    fn publish_group_output_rt(&self, group_id: u64, output: GroupOutput) {
        let Some(index) = self
            .rt_groups
            .iter()
            .position(|group| group.id.load(Ordering::Relaxed) == group_id)
        else {
            return;
        };
        self.rt_generation.fetch_add(1, Ordering::AcqRel);
        self.rt_groups[index].store_output(output);
        self.rt_group_output_generations[index].fetch_add(1, Ordering::Relaxed);
        self.rt_generation.fetch_add(1, Ordering::Release);
        self.rt_group_output_generation
            .fetch_add(1, Ordering::Release);
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

fn oscillator_mask(modules: &[super::Module]) -> u32 {
    modules
        .iter()
        .filter_map(|module| module.oscillator_slot())
        .fold(0, |mask, slot| mask | (1_u32 << slot.index()))
}

fn generator_rt_group(group: &super::Group) -> GeneratorRtGroup {
    let mut modules = [EMPTY_RT_MODULE; MAX_GENERATOR_MODULES];
    for (target, module) in modules.iter_mut().zip(group.modules()) {
        *target = match module.kind() {
            ModuleKind::Oscillator(slot) => GeneratorRtModule::Oscillator(slot),
            ModuleKind::Filter(slot) => GeneratorRtModule::Filter(slot),
        };
    }
    GeneratorRtGroup {
        id: group.id().get(),
        oscillator_mask: oscillator_mask(group.modules()),
        modules,
        module_count: group.modules().len().min(MAX_GENERATOR_MODULES) as u8,
        output: group.output().sanitized(),
    }
}

fn encode_rt_module(module: GeneratorRtModule) -> u8 {
    match module {
        GeneratorRtModule::Oscillator(slot) => slot.encoded(),
        GeneratorRtModule::Filter(slot) => 0x80 | slot.encoded(),
    }
}

fn decode_rt_module(encoded: u8) -> GeneratorRtModule {
    if encoded & 0x80 == 0 {
        GeneratorRtModule::Oscillator(
            OscillatorSlot::from_index(usize::from(encoded)).unwrap_or(OscillatorSlot::ZERO),
        )
    } else {
        GeneratorRtModule::Filter(
            FilterSlot::from_index(usize::from(encoded & 0x7f)).unwrap_or(FilterSlot::ZERO),
        )
    }
}

fn filter_mode_encoded(mode: FilterMode) -> u8 {
    match mode {
        FilterMode::Svf => 8,
        FilterMode::Phaser => 9,
        FilterMode::Fibonacci => 10,
    }
}

fn filter_mode_from_encoded(encoded: u8) -> FilterMode {
    match encoded {
        7 | 9 => FilterMode::Phaser,
        10 => FilterMode::Fibonacci,
        _ => FilterMode::Svf,
    }
}

fn sanitize_filter_config(config: FilterConfig) -> FilterConfig {
    FilterConfig {
        mode: config.mode,
        cutoff_hz: finite_or(config.cutoff_hz, 20_000.0).clamp(5.0, 100_000.0),
        q: finite_or(config.q, std::f32::consts::FRAC_1_SQRT_2).clamp(0.1, 32.0),
        slope_db_oct: finite_or(config.slope_db_oct, 12.0).clamp(12.0, 24.0),
        morph: finite_or(config.morph, 0.0).clamp(0.0, 1.0),
    }
}

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}
