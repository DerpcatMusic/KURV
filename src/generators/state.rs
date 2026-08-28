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

const DEFAULT_UNISON_RATE: f32 = 0.417_432;

/// Renderer selected by an oscillator module. The module remains an
/// `OscillatorSlot`, preserving routing, modulation and automation identity.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum OscillatorEngineKind {
    #[default]
    Va = 0,
    Resynth = 1,
    Noise = 2,
}

impl OscillatorEngineKind {
    pub const fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Resynth,
            2 => Self::Noise,
            _ => Self::Va,
        }
    }
}

/// Non-host-exposed controls for one oscillator slot.
///
/// Every oscillator module is an instance of this same configuration. The
/// shell keeps old fixed host parameters outside this structural module path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OscillatorConfig {
    pub enabled: bool,
    pub engine: OscillatorEngineKind,
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
    /// One-based oscillator slot used as an audio-rate phase source; zero disables PM.
    pub phase_mod_source: u8,
    pub phase_mod_amount: f32,
    pub unison_alignment: f32,
    pub unison_alignment_mode: u8,
    pub unison_pan_curve: f32,
    pub unison_pan_center_x: f32,
    pub unison_stereo_x: f32,
    pub unison_stereo_alternate: f32,
}

impl OscillatorConfig {
    #[must_use]
    pub fn for_engine(engine: OscillatorEngineKind) -> Self {
        let mut config = Self {
            engine,
            ..Self::default()
        };
        if engine == OscillatorEngineKind::Noise {
            config.shape = 1.5;
            config.pulse_width = 0.03;
            config.phase_warp_amount = 1.0;
        }
        config
    }

    fn sanitized(self) -> Self {
        Self {
            enabled: self.enabled,
            engine: self.engine,
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
            phase_mod_source: self.phase_mod_source.min(MAX_OSCILLATORS as u8),
            phase_mod_amount: finite_or(self.phase_mod_amount, 0.0).clamp(-1.0, 1.0),
            unison_alignment: finite_or(self.unison_alignment, 0.0).clamp(0.0, 1.0),
            unison_alignment_mode: self.unison_alignment_mode.min(3),
            unison_pan_curve: finite_or(self.unison_pan_curve, 0.0).clamp(-1.0, 1.0),
            unison_pan_center_x: finite_or(self.unison_pan_center_x, 0.5).clamp(0.05, 0.95),
            unison_stereo_x: finite_or(self.unison_stereo_x, 0.0).clamp(0.0, 1.0),
            unison_stereo_alternate: finite_or(self.unison_stereo_alternate, 1.0).clamp(0.0, 1.0),
        }
    }
}

impl Default for OscillatorConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            engine: OscillatorEngineKind::Va,
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
            phase_mod_source: 0,
            phase_mod_amount: 0.0,
            unison_alignment: 0.0,
            unison_alignment_mode: 0,
            unison_pan_curve: 0.0,
            unison_pan_center_x: 0.5,
            unison_stereo_x: 0.0,
            unison_stereo_alternate: 1.0,
        }
    }
}

/// One module in an audio-thread generator group's ordered program.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeneratorRtModule {
    Oscillator(OscillatorSlot),
    /// Transforms the running per-note bus produced by preceding modules.
    Filter(FilterSlot),
}

const EMPTY_RT_MODULE: GeneratorRtModule = GeneratorRtModule::Oscillator(OscillatorSlot::ZERO);

/// One ordered generator group's fixed audio-thread routing record.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeneratorRtGroup {
    id: u64,
    oscillator_mask: u32,
    filter_mask: u32,
    modules: [GeneratorRtModule; MAX_GENERATOR_MODULES],
    module_count: u8,
    terminal_filter_start: u8,
    output: GroupOutput,
}

impl GeneratorRtGroup {
    pub(crate) const EMPTY: Self = Self {
        id: 0,
        oscillator_mask: 0,
        filter_mask: 0,
        modules: [EMPTY_RT_MODULE; MAX_GENERATOR_MODULES],
        module_count: 0,
        terminal_filter_start: u8::MAX,
        output: GroupOutput {
            pair: 0,
            receive_midi_channel: 0,
            gain: 1.0,
            pan: 0.0,
            dry: 1.0,
            send: 0.0,
            sidechain: 0.0,
            send_pair: 0,
            attack: 0.0,
            attack_curve: 0.0,
            attack_curve_time: 0.0,
            decay: 0.1,
            decay_curve: 0.0,
            decay_curve_time: 0.0,
            sustain: 1.0,
            release: 0.0,
            release_curve: 0.0,
            release_curve_time: 0.0,
            envelope_enabled: true,
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

    /// Filter slots referenced by this group's ordered program.
    #[must_use]
    pub const fn filter_mask(self) -> u32 {
        self.filter_mask
    }

    /// Ordered oscillator/filter program for this group.
    #[must_use]
    pub fn modules(&self) -> &[GeneratorRtModule] {
        &self.modules[..usize::from(self.module_count)]
    }

    /// The terminal filter chain, when no oscillator follows its first filter.
    #[must_use]
    pub fn terminal_filters(&self) -> Option<&[GeneratorRtModule]> {
        let start = usize::from(self.terminal_filter_start);
        (start < usize::from(self.module_count))
            .then(|| &self.modules[start..usize::from(self.module_count)])
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

macro_rules! oscillator_atomic {
    (@type bool) => {
        AtomicBool
    };
    (@type u8) => {
        AtomicU8
    };
    (@type f32) => {
        AtomicU32
    };
    (@type engine) => {
        AtomicU8
    };
    (@new bool, $value:expr) => {
        AtomicBool::new($value)
    };
    (@new u8, $value:expr) => {
        AtomicU8::new($value)
    };
    (@new f32, $value:expr) => {
        AtomicU32::new($value.to_bits())
    };
    (@new engine, $value:expr) => {
        AtomicU8::new($value as u8)
    };
    (@store bool, $target:expr, $value:expr) => {
        $target.store($value, Ordering::Relaxed)
    };
    (@store u8, $target:expr, $value:expr) => {
        $target.store($value, Ordering::Relaxed)
    };
    (@store f32, $target:expr, $value:expr) => {
        $target.store($value.to_bits(), Ordering::Relaxed)
    };
    (@store engine, $target:expr, $value:expr) => {
        $target.store($value as u8, Ordering::Relaxed)
    };
    (@load bool, $source:expr) => {
        $source.load(Ordering::Relaxed)
    };
    (@load u8, $source:expr) => {
        $source.load(Ordering::Relaxed)
    };
    (@load f32, $source:expr) => {
        f32::from_bits($source.load(Ordering::Relaxed))
    };
    (@load engine, $source:expr) => {
        OscillatorEngineKind::from_u8($source.load(Ordering::Relaxed))
    };
}

macro_rules! rt_oscillator_config {
    ($($field:ident: $codec:ident),+ $(,)?) => {
        struct RtOscillatorConfig {
            $($field: oscillator_atomic!(@type $codec)),+
        }

        impl RtOscillatorConfig {
            fn new(config: OscillatorConfig) -> Self {
                Self {
                    $($field: oscillator_atomic!(@new $codec, config.$field)),+
                }
            }

            fn store(&self, config: OscillatorConfig) {
                let config = config.sanitized();
                $(oscillator_atomic!(@store $codec, self.$field, config.$field);)+
            }

            fn load(&self) -> OscillatorConfig {
                OscillatorConfig {
                    $($field: oscillator_atomic!(@load $codec, self.$field)),+
                }
            }
        }
    };
}

rt_oscillator_config! {
    enabled: bool,
    engine: engine,
    shape: f32,
    custom_shape: f32,
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
    unison_jitter_mode: u8,
    unison_rate: f32,
    unison_width: f32,
    unison_weight: f32,
    phase_position: f32,
    phase_random: f32,
    phase_warp_mode: u8,
    phase_warp_amount: f32,
    phase_mod_source: u8,
    phase_mod_amount: f32,
    unison_alignment: f32,
    unison_alignment_mode: u8,
    unison_pan_curve: f32,
    unison_pan_center_x: f32,
    unison_stereo_x: f32,
    unison_stereo_alternate: f32,
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
    filter_mask: AtomicU32,
    modules: [AtomicU8; MAX_GENERATOR_MODULES],
    module_count: AtomicU8,
    terminal_filter_start: AtomicU8,
    output_pair: AtomicU8,
    output_receive_midi_channel: AtomicU8,
    output_gain: AtomicU32,
    output_pan: AtomicU32,
    output_dry: AtomicU32,
    output_send: AtomicU32,
    output_sidechain: AtomicU32,
    output_send_pair: AtomicU8,
    output_attack: AtomicU32,
    output_attack_curve: AtomicU32,
    output_attack_curve_time: AtomicU32,
    output_decay: AtomicU32,
    output_decay_curve: AtomicU32,
    output_decay_curve_time: AtomicU32,
    output_sustain: AtomicU32,
    output_release: AtomicU32,
    output_release_curve: AtomicU32,
    output_release_curve_time: AtomicU32,
    output_envelope_enabled: AtomicBool,
}

impl RtGroup {
    fn new() -> Self {
        Self {
            id: AtomicU64::new(0),
            oscillator_mask: AtomicU32::new(0),
            filter_mask: AtomicU32::new(0),
            modules: std::array::from_fn(|_| AtomicU8::new(0)),
            module_count: AtomicU8::new(0),
            terminal_filter_start: AtomicU8::new(u8::MAX),
            output_pair: AtomicU8::new(0),
            output_receive_midi_channel: AtomicU8::new(0),
            output_gain: AtomicU32::new(1.0_f32.to_bits()),
            output_pan: AtomicU32::new(0.0_f32.to_bits()),
            output_dry: AtomicU32::new(1.0_f32.to_bits()),
            output_send: AtomicU32::new(0.0_f32.to_bits()),
            output_sidechain: AtomicU32::new(0.0_f32.to_bits()),
            output_send_pair: AtomicU8::new(0),
            output_attack: AtomicU32::new(0.0_f32.to_bits()),
            output_attack_curve: AtomicU32::new(0.0_f32.to_bits()),
            output_attack_curve_time: AtomicU32::new(0.0_f32.to_bits()),
            output_decay: AtomicU32::new(0.1_f32.to_bits()),
            output_decay_curve: AtomicU32::new(0.0_f32.to_bits()),
            output_decay_curve_time: AtomicU32::new(0.0_f32.to_bits()),
            output_sustain: AtomicU32::new(1.0_f32.to_bits()),
            output_release: AtomicU32::new(0.0_f32.to_bits()),
            output_release_curve: AtomicU32::new(0.0_f32.to_bits()),
            output_release_curve_time: AtomicU32::new(0.0_f32.to_bits()),
            output_envelope_enabled: AtomicBool::new(true),
        }
    }

    fn store(&self, group: GeneratorRtGroup) {
        self.id.store(group.id, Ordering::Relaxed);
        self.oscillator_mask
            .store(group.oscillator_mask, Ordering::Relaxed);
        self.filter_mask.store(group.filter_mask, Ordering::Relaxed);
        for (target, module) in self.modules.iter().zip(group.modules) {
            target.store(encode_rt_module(module), Ordering::Relaxed);
        }
        self.module_count
            .store(group.module_count, Ordering::Relaxed);
        self.terminal_filter_start
            .store(group.terminal_filter_start, Ordering::Relaxed);
        self.output_pair.store(group.output.pair, Ordering::Relaxed);
        self.output_receive_midi_channel
            .store(group.output.receive_midi_channel, Ordering::Relaxed);
        self.output_gain
            .store(group.output.gain.to_bits(), Ordering::Relaxed);
        self.output_pan
            .store(group.output.pan.to_bits(), Ordering::Relaxed);
        self.output_dry
            .store(group.output.dry.to_bits(), Ordering::Relaxed);
        self.output_send
            .store(group.output.send.to_bits(), Ordering::Relaxed);
        self.output_sidechain
            .store(group.output.sidechain.to_bits(), Ordering::Relaxed);
        self.output_send_pair
            .store(group.output.send_pair, Ordering::Relaxed);
        self.output_attack
            .store(group.output.attack.to_bits(), Ordering::Relaxed);
        self.output_attack_curve
            .store(group.output.attack_curve.to_bits(), Ordering::Relaxed);
        self.output_attack_curve_time
            .store(group.output.attack_curve_time.to_bits(), Ordering::Relaxed);
        self.output_decay
            .store(group.output.decay.to_bits(), Ordering::Relaxed);
        self.output_decay_curve
            .store(group.output.decay_curve.to_bits(), Ordering::Relaxed);
        self.output_decay_curve_time
            .store(group.output.decay_curve_time.to_bits(), Ordering::Relaxed);
        self.output_sustain
            .store(group.output.sustain.to_bits(), Ordering::Relaxed);
        self.output_release
            .store(group.output.release.to_bits(), Ordering::Relaxed);
        self.output_release_curve
            .store(group.output.release_curve.to_bits(), Ordering::Relaxed);
        self.output_release_curve_time
            .store(group.output.release_curve_time.to_bits(), Ordering::Relaxed);
        self.output_envelope_enabled
            .store(group.output.envelope_enabled, Ordering::Relaxed);
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
        self.output_dry
            .store(output.dry.to_bits(), Ordering::Relaxed);
        self.output_send
            .store(output.send.to_bits(), Ordering::Relaxed);
        self.output_sidechain
            .store(output.sidechain.to_bits(), Ordering::Relaxed);
        self.output_send_pair
            .store(output.send_pair, Ordering::Relaxed);
        self.output_attack
            .store(output.attack.to_bits(), Ordering::Relaxed);
        self.output_attack_curve
            .store(output.attack_curve.to_bits(), Ordering::Relaxed);
        self.output_attack_curve_time
            .store(output.attack_curve_time.to_bits(), Ordering::Relaxed);
        self.output_decay
            .store(output.decay.to_bits(), Ordering::Relaxed);
        self.output_decay_curve
            .store(output.decay_curve.to_bits(), Ordering::Relaxed);
        self.output_decay_curve_time
            .store(output.decay_curve_time.to_bits(), Ordering::Relaxed);
        self.output_sustain
            .store(output.sustain.to_bits(), Ordering::Relaxed);
        self.output_release
            .store(output.release.to_bits(), Ordering::Relaxed);
        self.output_release_curve
            .store(output.release_curve.to_bits(), Ordering::Relaxed);
        self.output_release_curve_time
            .store(output.release_curve_time.to_bits(), Ordering::Relaxed);
        self.output_envelope_enabled
            .store(output.envelope_enabled, Ordering::Relaxed);
    }

    fn load(&self) -> GeneratorRtGroup {
        let mut modules = [EMPTY_RT_MODULE; MAX_GENERATOR_MODULES];
        for (target, source) in modules.iter_mut().zip(&self.modules) {
            *target = decode_rt_module(source.load(Ordering::Relaxed));
        }
        GeneratorRtGroup {
            id: self.id.load(Ordering::Relaxed),
            oscillator_mask: self.oscillator_mask.load(Ordering::Relaxed),
            filter_mask: self.filter_mask.load(Ordering::Relaxed),
            modules,
            module_count: self
                .module_count
                .load(Ordering::Relaxed)
                .min(MAX_GENERATOR_MODULES as u8),
            terminal_filter_start: self.terminal_filter_start.load(Ordering::Relaxed),
            output: self.load_output(),
        }
    }

    fn load_output(&self) -> GroupOutput {
        GroupOutput {
            pair: self.output_pair.load(Ordering::Relaxed),
            receive_midi_channel: self.output_receive_midi_channel.load(Ordering::Relaxed),
            gain: f32::from_bits(self.output_gain.load(Ordering::Relaxed)),
            pan: f32::from_bits(self.output_pan.load(Ordering::Relaxed)),
            dry: f32::from_bits(self.output_dry.load(Ordering::Relaxed)),
            send: f32::from_bits(self.output_send.load(Ordering::Relaxed)),
            sidechain: f32::from_bits(self.output_sidechain.load(Ordering::Relaxed)),
            send_pair: self.output_send_pair.load(Ordering::Relaxed),
            attack: f32::from_bits(self.output_attack.load(Ordering::Relaxed)),
            attack_curve: f32::from_bits(self.output_attack_curve.load(Ordering::Relaxed)),
            attack_curve_time: f32::from_bits(
                self.output_attack_curve_time.load(Ordering::Relaxed),
            ),
            decay: f32::from_bits(self.output_decay.load(Ordering::Relaxed)),
            decay_curve: f32::from_bits(self.output_decay_curve.load(Ordering::Relaxed)),
            decay_curve_time: f32::from_bits(self.output_decay_curve_time.load(Ordering::Relaxed)),
            sustain: f32::from_bits(self.output_sustain.load(Ordering::Relaxed)),
            release: f32::from_bits(self.output_release.load(Ordering::Relaxed)),
            release_curve: f32::from_bits(self.output_release_curve.load(Ordering::Relaxed)),
            release_curve_time: f32::from_bits(
                self.output_release_curve_time.load(Ordering::Relaxed),
            ),
            envelope_enabled: self.output_envelope_enabled.load(Ordering::Relaxed),
        }
    }
}

/// Editable generator storage with a fixed lock-free audio snapshot.
pub struct GeneratorStackState {
    document: RwLock<GeneratorDocument>,
    va_tables: [VaTableState; MAX_OSCILLATORS],
    pan_shape_curves: [PanShapeCurveState; MAX_OSCILLATORS],
    materialized: AtomicBool,
    legacy_migration_pending: AtomicBool,
    legacy_host_automation_bridge: AtomicBool,
    legacy_automation_oscillator_masks: [AtomicU32; 3],
    legacy_automation_group_mask: AtomicU32,
    legacy_automation_oscillator_released: [AtomicU32; 3],
    legacy_automation_group_released: AtomicU32,
    legacy_pan_automation_masks: [AtomicU32; 3],
    legacy_pan_automation_released: [AtomicU32; 3],
    legacy_automation_epoch: AtomicU32,
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
            legacy_migration_pending: AtomicBool::new(false),
            legacy_host_automation_bridge: AtomicBool::new(false),
            legacy_automation_oscillator_masks: std::array::from_fn(|_| AtomicU32::new(0)),
            legacy_automation_group_mask: AtomicU32::new(0),
            legacy_automation_oscillator_released: std::array::from_fn(|_| AtomicU32::new(0)),
            legacy_automation_group_released: AtomicU32::new(0),
            legacy_pan_automation_masks: std::array::from_fn(|_| AtomicU32::new(0)),
            legacy_pan_automation_released: std::array::from_fn(|_| AtomicU32::new(0)),
            legacy_automation_epoch: AtomicU32::new(0),
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

    /// Whether the editable document is the audio source of truth.
    /// Host load translates pre-modular sessions into this document before
    /// the next process block; audio never stays on the hidden renderer.
    #[must_use]
    pub fn is_materialized(&self) -> bool {
        self.materialized.load(Ordering::Acquire)
    }

    /// True only when the keyed generator document was absent during state
    /// restore. Early structural documents may legitimately encode
    /// `materialized = false`, so absence must be tracked separately.
    #[must_use]
    pub(crate) fn legacy_migration_pending(&self) -> bool {
        self.legacy_migration_pending.load(Ordering::Acquire)
    }

    pub(crate) fn legacy_host_automation_bridge_enabled(&self) -> bool {
        self.legacy_host_automation_bridge.load(Ordering::Acquire)
    }

    pub(crate) fn legacy_automation_epoch(&self) -> u32 {
        self.legacy_automation_epoch.load(Ordering::Acquire)
    }

    pub(crate) fn legacy_automation_masks(
        &self,
    ) -> ([u32; 3], u16, [u32; 3], u16, [u16; 3], [u16; 3]) {
        (
            std::array::from_fn(|index| {
                self.legacy_automation_oscillator_masks[index].load(Ordering::Acquire)
            }),
            self.legacy_automation_group_mask.load(Ordering::Acquire) as u16,
            std::array::from_fn(|index| {
                self.legacy_automation_oscillator_released[index].load(Ordering::Acquire)
            }),
            self.legacy_automation_group_released
                .load(Ordering::Acquire) as u16,
            std::array::from_fn(|index| {
                self.legacy_pan_automation_masks[index].load(Ordering::Acquire) as u16
            }),
            std::array::from_fn(|index| {
                self.legacy_pan_automation_released[index].load(Ordering::Acquire) as u16
            }),
        )
    }

    pub(crate) fn set_legacy_automation_masks(
        &self,
        oscillators: [u32; 3],
        group: u16,
        oscillator_released: [u32; 3],
        group_released: u16,
        pan: [u16; 3],
        pan_released: [u16; 3],
    ) {
        for (target, mask) in self
            .legacy_automation_oscillator_masks
            .iter()
            .zip(oscillators)
        {
            target.store(mask, Ordering::Release);
        }
        self.legacy_automation_group_mask
            .store(u32::from(group), Ordering::Release);
        for (target, mask) in self
            .legacy_automation_oscillator_released
            .iter()
            .zip(oscillator_released)
        {
            target.store(mask, Ordering::Release);
        }
        self.legacy_automation_group_released
            .store(u32::from(group_released), Ordering::Release);
        for (target, mask) in self.legacy_pan_automation_masks.iter().zip(pan) {
            target.store(u32::from(mask), Ordering::Release);
        }
        for (target, mask) in self.legacy_pan_automation_released.iter().zip(pan_released) {
            target.store(u32::from(mask), Ordering::Release);
        }
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
        self.legacy_migration_pending
            .store(false, Ordering::Release);
        self.legacy_host_automation_bridge
            .store(false, Ordering::Release);
        self.set_legacy_automation_masks([0; 3], 0, [0; 3], 0, [0; 3], [0; 3]);
        self.legacy_automation_epoch.fetch_add(1, Ordering::AcqRel);
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
        let group_count = self
            .rt_group_count
            .load(Ordering::Relaxed)
            .min(MAX_OUTPUT_PAIRS as u8);
        for (target, source) in groups.iter_mut().zip(&self.rt_groups) {
            *target = source.load();
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

    /// The compatibility renderer is selected when a document is not
    /// materialized. A one-oscillator factory topology in that state is the
    /// placeholder written by `reset_legacy`, not a user-authored modular
    /// patch, so the next `post_load` must copy the hidden host parameters.
    fn is_legacy_placeholder(document: &GeneratorDocument) -> bool {
        let groups = document.patch.groups();
        groups.len() == 1
            && matches!(
                groups[0].modules(),
                [module]
                    if matches!(module.kind(), ModuleKind::Oscillator(slot) if slot.index() == 0)
            )
    }

    /// Translate the fixed three-oscillator representation used by pre-modular
    /// projects into the editable generator document. This runs only on the
    /// host state-load thread, never in `process()`.
    pub(crate) fn materialize_legacy(
        &self,
        oscillators: [OscillatorConfig; 3],
        output: GroupOutput,
        va_tables: [VaTableData; 3],
        pan_shape_curves: [PanShapeCurveData; 3],
    ) {
        if !self.legacy_migration_pending() {
            return;
        }
        let mut document = GeneratorDocument::default();
        let group_id = document.patch.groups()[0].id();
        let patch = Arc::make_mut(&mut document.patch);
        // Slot zero is present in the default patch. Fixed legacy projects
        // always owned three oscillator slots even when some were disabled.
        let _ = patch.insert_oscillator_with_slot(
            group_id,
            1,
            OscillatorSlot::from_index(1).expect("legacy slot 1"),
        );
        let _ = patch.insert_oscillator_with_slot(
            group_id,
            2,
            OscillatorSlot::from_index(2).expect("legacy slot 2"),
        );
        let _ = patch.set_group_output(group_id, output.legacy_global_envelope());
        document.oscillators[..3].copy_from_slice(&oscillators);
        for (state, data) in self.va_tables[..3].iter().zip(va_tables) {
            state.replace(data);
        }
        for (state, data) in self.pan_shape_curves[..3].iter().zip(pan_shape_curves) {
            state.replace(data);
        }
        let mut current = self
            .document
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *current = document;
        self.legacy_migration_pending
            .store(false, Ordering::Release);
        self.legacy_host_automation_bridge
            .store(true, Ordering::Release);
        self.set_legacy_automation_masks([0; 3], 0, [0; 3], 0, [0; 3], [0; 3]);
        self.legacy_automation_epoch.fetch_add(1, Ordering::AcqRel);
        self.publish_rt(&current, true);
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
        self.legacy_migration_pending.store(true, Ordering::Release);
        self.legacy_host_automation_bridge
            .store(false, Ordering::Release);
        self.set_legacy_automation_masks([0; 3], 0, [0; 3], 0, [0; 3], [0; 3]);
        self.legacy_automation_epoch.fetch_add(1, Ordering::AcqRel);
        self.publish_rt(&document, false);
    }

    #[cfg(test)]
    pub(crate) fn publish_unmaterialized_for_test(&self) {
        let document = self
            .document
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.legacy_migration_pending
            .store(false, Ordering::Release);
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
    fn engine_defaults_keep_noise_character_and_shared_controls() {
        let va = OscillatorConfig::for_engine(OscillatorEngineKind::Va);
        let noise = OscillatorConfig::for_engine(OscillatorEngineKind::Noise);

        assert_eq!(
            (noise.shape, noise.pulse_width, noise.phase_warp_amount),
            (1.5, 0.03, 1.0)
        );
        assert_eq!(
            (noise.level, noise.pan, noise.unison_voices),
            (va.level, va.pan, va.unison_voices)
        );
    }

    #[test]
    fn oscillator_default_uses_alternating_stereo_for_center_satellites() {
        let config = OscillatorConfig::default();
        let left = crate::voices::unison_lane_position_stereo_seeded(
            4,
            0,
            config.unison_curve,
            config.unison_stereo_alternate,
            config.unison_stereo_x,
            0.0,
            crate::voices::PanShapeSettings::default(),
            0.5,
        );
        let right = crate::voices::unison_lane_position_stereo_seeded(
            4,
            1,
            config.unison_curve,
            config.unison_stereo_alternate,
            config.unison_stereo_x,
            0.0,
            crate::voices::PanShapeSettings::default(),
            0.5,
        );

        assert_eq!(
            (config.unison_stereo_x, config.unison_stereo_alternate),
            (0.0, 1.0)
        );
        assert!(left.1 < -0.9 && right.1 > 0.9);
    }

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

fn filter_mask(modules: &[super::Module]) -> u32 {
    modules
        .iter()
        .filter_map(|module| module.filter_slot())
        .fold(0, |mask, slot| mask | (1_u32 << slot.index()))
}

fn terminal_filter_start(modules: &[super::Module]) -> u8 {
    modules
        .iter()
        .position(|module| module.filter_slot().is_some())
        .filter(|&first| {
            modules[first..]
                .iter()
                .all(|module| module.filter_slot().is_some())
        })
        .map_or(u8::MAX, |first| first as u8)
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
        filter_mask: filter_mask(group.modules()),
        modules,
        module_count: group.modules().len().min(MAX_GENERATOR_MODULES) as u8,
        terminal_filter_start: terminal_filter_start(group.modules()),
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
        FilterMode::Scream => 11,
    }
}

fn filter_mode_from_encoded(encoded: u8) -> FilterMode {
    match encoded {
        7 | 9 => FilterMode::Phaser,
        10 => FilterMode::Phaser,
        11 => FilterMode::Scream,
        _ => FilterMode::Svf,
    }
}

fn sanitize_filter_config(config: FilterConfig) -> FilterConfig {
    config.sanitized()
}

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}
