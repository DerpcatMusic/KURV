//! Persisted mixed modulation rack with a fixed lock-free runtime snapshot.

use std::sync::{
    RwLock,
    atomic::{AtomicU8, AtomicU16, AtomicU32, AtomicU64, Ordering},
};

use truce::State;
use truce_core::custom_state::{PersistField, StateCursor, StateField};

use crate::wave_curve::{WaveCurveData, WaveCurveState};

pub const MAX_MODULATION_SOURCES: usize = 64;
pub const LEGACY_MODULATION_SOURCES: usize = 8;
pub const GATE_STEP_COUNT: usize = 16;
pub const DEFAULT_GATE_PATTERN: u16 = u16::MAX;
pub const DEFAULT_GATE_PROBABILITIES: [u8; GATE_STEP_COUNT] = [100; GATE_STEP_COUNT];
const INITIAL_STATE_VERSION: u32 = 1;
const CURVE_STATE_VERSION: u32 = 2;
const ORDER_STATE_VERSION: u32 = 3;
const SHAPE_STATE_VERSION: u32 = 4;
const GATE_STATE_VERSION: u32 = 5;
const STATIC_SOURCE_STATE_VERSION: u32 = 6;
const STATE_VERSION: u32 = 7;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum SourceKind {
    #[default]
    Lfo,
    Envelope,
    Keytrack,
    Macro,
    Button,
}

impl SourceKind {
    pub const fn from_index(index: u8) -> Self {
        match index {
            1 => Self::Envelope,
            2 => Self::Keytrack,
            3 => Self::Macro,
            4 => Self::Button,
            _ => Self::Lfo,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SourceConfig {
    pub active: bool,
    pub kind: SourceKind,
    pub rate_hz: f32,
    pub rate_mode: u8,
    pub mode: u8,
    pub phase_offset: f32,
    pub sync_division: u8,
    pub bipolar: bool,
    pub shape: u8,
    /// Enabled steps in the fixed 16-step gate sequence, least-significant bit first.
    pub gate_pattern: u16,
    /// Delays odd steps by up to half a step while preserving each two-step pair.
    pub gate_swing: f32,
    /// Per-step trigger chance in whole percent. Values are evaluated deterministically.
    pub gate_probabilities: [u8; GATE_STEP_COUNT],
    pub attack: f32,
    pub attack_curve: f32,
    pub decay: f32,
    pub decay_curve: f32,
    pub sustain: f32,
    pub release: f32,
    pub release_curve: f32,
    /// Static source value. Buttons quantize this to zero or one.
    pub value: f32,
    /// MIDI note that produces zero from a bipolar Keytrack source.
    pub keytrack_root: f32,
}

impl SourceConfig {
    fn sanitized(self) -> Self {
        Self {
            active: self.active,
            kind: self.kind,
            rate_hz: finite_or(self.rate_hz, 1.0).clamp(0.01, 20_000.0),
            rate_mode: self.rate_mode.min(3),
            mode: self.mode.min(3),
            phase_offset: finite_or(self.phase_offset, 0.0).rem_euclid(1.0),
            sync_division: self.sync_division.min(15),
            bipolar: self.bipolar,
            shape: self.shape.min(3),
            gate_pattern: self.gate_pattern,
            gate_swing: finite_or(self.gate_swing, 0.0).clamp(0.0, 1.0),
            gate_probabilities: self
                .gate_probabilities
                .map(|probability| probability.min(100)),
            attack: finite_or(self.attack, 0.01).clamp(0.0, 8.0),
            attack_curve: finite_or(self.attack_curve, 0.0).clamp(-1.0, 1.0),
            decay: finite_or(self.decay, 0.1).clamp(0.0, 8.0),
            decay_curve: finite_or(self.decay_curve, 0.0).clamp(-1.0, 1.0),
            sustain: finite_or(self.sustain, 0.8).clamp(0.0, 1.0),
            release: finite_or(self.release, 0.2).clamp(0.0, 12.0),
            release_curve: finite_or(self.release_curve, 0.0).clamp(-1.0, 1.0),
            value: finite_or(self.value, 0.0).clamp(0.0, 1.0),
            keytrack_root: finite_or(self.keytrack_root, 60.0).clamp(0.0, 127.0),
        }
    }
}

impl Default for SourceConfig {
    fn default() -> Self {
        Self {
            active: false,
            kind: SourceKind::Lfo,
            rate_hz: 1.0,
            rate_mode: 0,
            mode: 0,
            phase_offset: 0.0,
            sync_division: 4,
            bipolar: true,
            shape: 0,
            gate_pattern: DEFAULT_GATE_PATTERN,
            gate_swing: 0.0,
            gate_probabilities: DEFAULT_GATE_PROBABILITIES,
            attack: 0.01,
            attack_curve: 0.0,
            decay: 0.1,
            decay_curve: 0.0,
            sustain: 0.8,
            release: 0.2,
            release_curve: 0.0,
            value: 0.0,
            keytrack_root: 60.0,
        }
    }
}

macro_rules! source_atomic {
    (@type bool) => {
        AtomicU8
    };
    (@type kind) => {
        AtomicU8
    };
    (@type u8) => {
        AtomicU8
    };
    (@type u16) => {
        AtomicU16
    };
    (@type f32) => {
        AtomicU32
    };
    (@new bool, $value:expr) => {
        AtomicU8::new(u8::from($value))
    };
    (@new kind, $value:expr) => {
        AtomicU8::new($value as u8)
    };
    (@new u8, $value:expr) => {
        AtomicU8::new($value)
    };
    (@new u16, $value:expr) => {
        AtomicU16::new($value)
    };
    (@new f32, $value:expr) => {
        AtomicU32::new($value.to_bits())
    };
    (@store bool, $target:expr, $value:expr) => {
        $target.store(u8::from($value), Ordering::Relaxed)
    };
    (@store kind, $target:expr, $value:expr) => {
        $target.store($value as u8, Ordering::Relaxed)
    };
    (@store u8, $target:expr, $value:expr) => {
        $target.store($value, Ordering::Relaxed)
    };
    (@store u16, $target:expr, $value:expr) => {
        $target.store($value, Ordering::Relaxed)
    };
    (@store f32, $target:expr, $value:expr) => {
        $target.store($value.to_bits(), Ordering::Relaxed)
    };
    (@load bool, $source:expr) => {
        $source.load(Ordering::Relaxed) != 0
    };
    (@load kind, $source:expr) => {
        SourceKind::from_index($source.load(Ordering::Relaxed))
    };
    (@load u8, $source:expr) => {
        $source.load(Ordering::Relaxed)
    };
    (@load u16, $source:expr) => {
        $source.load(Ordering::Relaxed)
    };
    (@load f32, $source:expr) => {
        f32::from_bits($source.load(Ordering::Relaxed))
    };
}

macro_rules! rt_source_config {
    (
        $($before_field:ident: $before_codec:ident),+;
        $probabilities:ident => ($probabilities_low:ident, $probabilities_high:ident);
        $($after_field:ident: $after_codec:ident),+
        $(,)?
    ) => {
        struct RtSourceConfig {
            $($before_field: source_atomic!(@type $before_codec),)+
            $probabilities_low: AtomicU64,
            $probabilities_high: AtomicU64,
            $($after_field: source_atomic!(@type $after_codec),)+
        }

        impl RtSourceConfig {
            fn new(config: SourceConfig) -> Self {
                Self {
                    $($before_field: source_atomic!(@new $before_codec, config.$before_field),)+
                    $probabilities_low: AtomicU64::new(pack_probabilities(
                        config.$probabilities,
                        0,
                    )),
                    $probabilities_high: AtomicU64::new(pack_probabilities(
                        config.$probabilities,
                        8,
                    )),
                    $($after_field: source_atomic!(@new $after_codec, config.$after_field),)+
                }
            }

            fn store(&self, config: SourceConfig) {
                let config = config.sanitized();
                $(source_atomic!(@store $before_codec, self.$before_field, config.$before_field);)+
                self.$probabilities_low.store(
                    pack_probabilities(config.$probabilities, 0),
                    Ordering::Relaxed,
                );
                self.$probabilities_high.store(
                    pack_probabilities(config.$probabilities, 8),
                    Ordering::Relaxed,
                );
                $(source_atomic!(@store $after_codec, self.$after_field, config.$after_field);)+
            }

            fn load(&self) -> SourceConfig {
                SourceConfig {
                    $($before_field: source_atomic!(@load $before_codec, self.$before_field),)+
                    $probabilities: unpack_probabilities(
                        self.$probabilities_low.load(Ordering::Relaxed),
                        self.$probabilities_high.load(Ordering::Relaxed),
                    ),
                    $($after_field: source_atomic!(@load $after_codec, self.$after_field),)+
                }
            }
        }
    };
}

rt_source_config! {
    active: bool,
    kind: kind,
    rate_hz: f32,
    rate_mode: u8,
    mode: u8,
    phase_offset: f32,
    sync_division: u8,
    bipolar: bool,
    shape: u8,
    gate_pattern: u16,
    gate_swing: f32;
    gate_probabilities => (gate_probabilities_low, gate_probabilities_high);
    attack: f32,
    attack_curve: f32,
    decay: f32,
    decay_curve: f32,
    sustain: f32,
    release: f32,
    release_curve: f32,
    value: f32,
    keytrack_root: f32,
}

#[derive(Clone, Default, State)]
struct SourceDocument {
    active: bool,
    kind: u8,
    rate_hz: f32,
    rate_mode: u8,
    mode: u8,
    phase_offset: f32,
    sync_division: u8,
    bipolar: bool,
    attack: f32,
    decay: f32,
    sustain: f32,
    release: f32,
    // Keep curve fields at the tail for legacy positional state blobs.
    attack_curve: f32,
    decay_curve: f32,
    release_curve: f32,
    shape: u8,
    // Gate fields stay at the tail so v1-v4 positional documents keep decoding.
    gate_pattern: u16,
    gate_swing: f32,
    gate_probabilities: Vec<u8>,
    // Static-source fields stay at the tail for v1-v5 positional documents.
    value: f32,
    // Keytrack fields stay at the tail for v1-v6 positional documents.
    keytrack_root: f32,
}

impl From<SourceConfig> for SourceDocument {
    fn from(config: SourceConfig) -> Self {
        Self {
            active: config.active,
            kind: config.kind as u8,
            rate_hz: config.rate_hz,
            rate_mode: config.rate_mode,
            mode: config.mode,
            phase_offset: config.phase_offset,
            sync_division: config.sync_division,
            bipolar: config.bipolar,
            attack: config.attack,
            decay: config.decay,
            sustain: config.sustain,
            release: config.release,
            attack_curve: config.attack_curve,
            decay_curve: config.decay_curve,
            release_curve: config.release_curve,
            shape: config.shape,
            gate_pattern: config.gate_pattern,
            gate_swing: config.gate_swing,
            gate_probabilities: config.gate_probabilities.to_vec(),
            value: config.value,
            keytrack_root: config.keytrack_root,
        }
    }
}

impl SourceDocument {
    fn into_config(self, version: u32) -> SourceConfig {
        let legacy_gate_defaults = self.gate_pattern == 0 && self.gate_probabilities.is_empty();
        let mut gate_probabilities = DEFAULT_GATE_PROBABILITIES;
        for (target, probability) in gate_probabilities.iter_mut().zip(self.gate_probabilities) {
            *target = probability;
        }
        SourceConfig {
            active: self.active,
            kind: SourceKind::from_index(self.kind),
            rate_hz: self.rate_hz,
            rate_mode: self.rate_mode,
            mode: self.mode,
            phase_offset: self.phase_offset,
            sync_division: self.sync_division,
            bipolar: self.bipolar,
            shape: self.shape,
            gate_pattern: if legacy_gate_defaults {
                DEFAULT_GATE_PATTERN
            } else {
                self.gate_pattern
            },
            gate_swing: self.gate_swing,
            gate_probabilities,
            attack: self.attack,
            attack_curve: self.attack_curve,
            decay: self.decay,
            decay_curve: self.decay_curve,
            sustain: self.sustain,
            release: self.release,
            release_curve: self.release_curve,
            value: self.value,
            keytrack_root: if version < STATE_VERSION {
                60.0
            } else {
                self.keytrack_root
            },
        }
        .sanitized()
    }
}

#[derive(Default, State)]
struct RackDocument {
    version: u32,
    sources: Vec<SourceDocument>,
    curves: Vec<WaveCurveData>,
    // Keep new fields at the tail for legacy positional state blobs.
    presentation_order: Vec<u8>,
}

#[derive(Clone, PartialEq)]
pub(crate) struct ModulatorRackHistorySnapshot {
    configs: Box<[SourceConfig; MAX_MODULATION_SOURCES]>,
    curves: Box<[WaveCurveData; MAX_MODULATION_SOURCES]>,
    curve_generations: [u32; MAX_MODULATION_SOURCES],
    presentation_order: [u8; MAX_MODULATION_SOURCES],
}

impl ModulatorRackHistorySnapshot {
    pub(crate) fn retained_bytes(&self) -> usize {
        std::mem::size_of_val(self.configs.as_ref())
            + std::mem::size_of_val(self.curves.as_ref())
            + self
                .curves
                .iter()
                .map(|curve| std::mem::size_of_val(curve.knots.as_slice()))
                .sum::<usize>()
    }
}

pub struct ModulatorRackState {
    document: RwLock<Box<[SourceConfig; MAX_MODULATION_SOURCES]>>,
    // Editor-only presentation state; source identity remains the array index.
    presentation_order: RwLock<[u8; MAX_MODULATION_SOURCES]>,
    active_mask: AtomicU64,
    ui_running_mask: AtomicU64,
    rt_sources: Box<[RtSourceConfig; MAX_MODULATION_SOURCES]>,
    curves: Box<[WaveCurveState; MAX_MODULATION_SOURCES]>,
    ui_phases: Box<[AtomicU32; MAX_MODULATION_SOURCES]>,
    ui_values: Box<[AtomicU32; MAX_MODULATION_SOURCES]>,
}

impl ModulatorRackState {
    pub fn new() -> Self {
        let sources = boxed_array(SourceConfig::default());
        Self {
            document: RwLock::new(sources),
            presentation_order: RwLock::new(default_presentation_order()),
            active_mask: AtomicU64::new(0),
            ui_running_mask: AtomicU64::new(0),
            rt_sources: boxed_from_fn(|_| RtSourceConfig::new(SourceConfig::default())),
            curves: boxed_from_fn(|_| WaveCurveState::default()),
            ui_phases: boxed_from_fn(|_| AtomicU32::new(0)),
            ui_values: boxed_from_fn(|_| AtomicU32::new(0)),
        }
    }

    pub(crate) fn reset_to_default(&self) {
        self.replace(boxed_array(SourceConfig::default()));
        *self
            .presentation_order
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = default_presentation_order();
        for curve in &*self.curves {
            curve.replace(WaveCurveData::default());
        }
        self.ui_running_mask.store(0, Ordering::Release);
        for (phase, value) in self.ui_phases.iter().zip(self.ui_values.iter()) {
            phase.store(0.0_f32.to_bits(), Ordering::Relaxed);
            value.store(0.0_f32.to_bits(), Ordering::Relaxed);
        }
    }

    pub fn config(&self, index: usize) -> SourceConfig {
        self.document
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(index)
            .copied()
            .unwrap_or_default()
    }

    pub fn set_config(&self, index: usize, config: SourceConfig) -> bool {
        if index >= MAX_MODULATION_SOURCES {
            return false;
        }
        let config = config.sanitized();
        let mut document = self
            .document
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if document[index] == config {
            return false;
        }
        document[index] = config;
        self.publish(index, config);
        true
    }

    pub fn presentation_order(&self) -> [u8; MAX_MODULATION_SOURCES] {
        *self
            .presentation_order
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn history_snapshot(&self) -> ModulatorRackHistorySnapshot {
        let configs = self
            .document
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let presentation_order = *self
            .presentation_order
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        ModulatorRackHistorySnapshot {
            configs,
            curves: Box::new(std::array::from_fn(|index| self.curves[index].snapshot())),
            curve_generations: std::array::from_fn(|index| self.curves[index].history_generation()),
            presentation_order,
        }
    }

    pub(crate) fn matches_history_snapshot(&self, snapshot: &ModulatorRackHistorySnapshot) -> bool {
        let configs_match = self
            .document
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            == snapshot.configs.as_ref();
        let order_matches = *self
            .presentation_order
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            == snapshot.presentation_order;
        configs_match
            && order_matches
            && self
                .curves
                .iter()
                .zip(snapshot.curve_generations)
                .all(|(curve, generation)| curve.history_generation() == generation)
    }

    pub(crate) fn restore_history_snapshot(&self, snapshot: &ModulatorRackHistorySnapshot) {
        for (curve, (data, generation)) in self
            .curves
            .iter()
            .zip(snapshot.curves.iter().zip(snapshot.curve_generations))
        {
            if curve.history_generation() != generation {
                curve.replace(data.clone());
            }
        }
        self.replace(snapshot.configs.clone());
        *self
            .presentation_order
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = snapshot.presentation_order;
    }

    pub fn move_source_slot(&self, source_slot: usize, insertion_index: usize) -> bool {
        if source_slot >= MAX_MODULATION_SOURCES {
            return false;
        }
        let mut order = self
            .presentation_order
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(current_index) = order
            .iter()
            .position(|&slot| usize::from(slot) == source_slot)
        else {
            return false;
        };
        let mut target_index = insertion_index.min(MAX_MODULATION_SOURCES);
        if current_index < target_index {
            target_index -= 1;
        }
        if current_index == target_index {
            return false;
        }
        if current_index < target_index {
            order[current_index..=target_index].rotate_left(1);
        } else {
            order[target_index..=current_index].rotate_right(1);
        }
        true
    }

    pub fn move_source_slots(&self, source_mask: u64, insertion_index: usize) -> bool {
        if source_mask == 0 {
            return false;
        }
        let mut order = self
            .presentation_order
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let before = *order;
        let mut members = [0_u8; MAX_MODULATION_SOURCES];
        let mut rest = [0_u8; MAX_MODULATION_SOURCES];
        let mut member_len = 0;
        let mut rest_len = 0;
        for slot in before {
            if source_mask & (1_u64 << slot) != 0 {
                members[member_len] = slot;
                member_len += 1;
            } else {
                rest[rest_len] = slot;
                rest_len += 1;
            }
        }
        let insertion = insertion_index.min(rest_len);
        order[..insertion].copy_from_slice(&rest[..insertion]);
        order[insertion..insertion + member_len].copy_from_slice(&members[..member_len]);
        order[insertion + member_len..].copy_from_slice(&rest[insertion..rest_len]);
        *order != before
    }

    pub fn active_mask(&self) -> u64 {
        self.active_mask.load(Ordering::Acquire)
    }

    pub fn rt_config(&self, index: usize) -> SourceConfig {
        if index >= MAX_MODULATION_SOURCES {
            return SourceConfig::default();
        }
        self.rt_sources[index].load().sanitized()
    }

    pub fn curve(&self, index: usize) -> Option<&WaveCurveState> {
        self.curves.get(index)
    }

    pub fn publish_ui_snapshot(&self, phases: &[f32], values: &[f32], running_mask: u64) {
        self.ui_running_mask.store(running_mask, Ordering::Release);
        for index in 0..MAX_MODULATION_SOURCES.min(phases.len()).min(values.len()) {
            self.ui_phases[index].store(phases[index].to_bits(), Ordering::Relaxed);
            self.ui_values[index].store(values[index].to_bits(), Ordering::Relaxed);
        }
    }

    pub fn ui_running(&self, index: usize) -> bool {
        index < MAX_MODULATION_SOURCES
            && self.ui_running_mask.load(Ordering::Acquire) & (1_u64 << index) != 0
    }

    pub fn ui_snapshot(&self, index: usize) -> (f32, f32) {
        if index >= MAX_MODULATION_SOURCES {
            return (0.0, 0.0);
        }
        (
            f32::from_bits(self.ui_phases[index].load(Ordering::Relaxed)),
            f32::from_bits(self.ui_values[index].load(Ordering::Relaxed)),
        )
    }

    fn publish(&self, index: usize, config: SourceConfig) {
        self.rt_sources[index].store(config);
        let bit = 1_u64 << index;
        if config.active {
            self.active_mask.fetch_or(bit, Ordering::Release);
        } else {
            self.active_mask.fetch_and(!bit, Ordering::Release);
        }
    }

    fn replace(&self, sources: Box<[SourceConfig; MAX_MODULATION_SOURCES]>) {
        let mut document = self
            .document
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *document = sources;
        let mut mask = 0_u64;
        for (index, (target, config)) in self.rt_sources.iter().zip(document.iter()).enumerate() {
            target.store(*config);
            if config.active {
                mask |= 1_u64 << index;
            }
        }
        self.active_mask.store(mask, Ordering::Release);
    }
}

impl Default for ModulatorRackState {
    fn default() -> Self {
        Self::new()
    }
}

impl PersistField for ModulatorRackState {
    fn persist_write(&self, buf: &mut Vec<u8>) {
        let presentation_order = self.presentation_order();
        let document = self
            .document
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        RackDocument {
            version: STATE_VERSION,
            sources: document.iter().copied().map(SourceDocument::from).collect(),
            curves: self.curves.iter().map(WaveCurveState::snapshot).collect(),
            presentation_order: presentation_order.to_vec(),
        }
        .write_field(buf);
    }

    fn persist_read(&self, cursor: &mut StateCursor) {
        let Some(document) = RackDocument::read_field(cursor).filter(|document| {
            matches!(
                document.version,
                INITIAL_STATE_VERSION
                    | CURVE_STATE_VERSION
                    | ORDER_STATE_VERSION
                    | SHAPE_STATE_VERSION
                    | GATE_STATE_VERSION
                    | STATIC_SOURCE_STATE_VERSION
                    | STATE_VERSION
            )
        }) else {
            return;
        };
        let version = document.version;
        let presentation_order = normalized_presentation_order(&document.presentation_order);
        let mut sources = boxed_array(SourceConfig::default());
        let mut legacy_keytrack_mask = 0_u64;
        for (index, (target, source)) in sources.iter_mut().zip(document.sources).enumerate() {
            *target = source.into_config(version);
            if version < STATE_VERSION && target.kind == SourceKind::Keytrack {
                legacy_keytrack_mask |= 1_u64 << index;
            }
        }
        for (index, (target, curve)) in self.curves.iter().zip(document.curves).enumerate() {
            target.replace(if legacy_keytrack_mask & (1_u64 << index) != 0 {
                crate::wave_curve::default_keytrack_curve()
            } else {
                curve
            });
        }
        self.replace(sources);
        *self
            .presentation_order
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = presentation_order;
    }
}

fn default_presentation_order() -> [u8; MAX_MODULATION_SOURCES] {
    std::array::from_fn(|index| index as u8)
}

fn normalized_presentation_order(stored: &[u8]) -> [u8; MAX_MODULATION_SOURCES] {
    let mut order = [0; MAX_MODULATION_SOURCES];
    let mut seen = 0_u64;
    let mut length = 0;
    for slot in stored
        .iter()
        .copied()
        .chain(0..MAX_MODULATION_SOURCES as u8)
    {
        if usize::from(slot) >= MAX_MODULATION_SOURCES || seen & (1_u64 << slot) != 0 {
            continue;
        }
        order[length] = slot;
        length += 1;
        seen |= 1_u64 << slot;
        if length == MAX_MODULATION_SOURCES {
            break;
        }
    }
    order
}

fn boxed_array<T: Clone, const N: usize>(value: T) -> Box<[T; N]> {
    Vec::from_iter(std::iter::repeat_n(value, N))
        .into_boxed_slice()
        .try_into()
        .unwrap_or_else(|_| unreachable!())
}

fn boxed_from_fn<T, const N: usize>(mut value: impl FnMut(usize) -> T) -> Box<[T; N]> {
    (0..N)
        .map(&mut value)
        .collect::<Vec<_>>()
        .into_boxed_slice()
        .try_into()
        .unwrap_or_else(|_| unreachable!())
}

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

const fn pack_probabilities(probabilities: [u8; GATE_STEP_COUNT], offset: usize) -> u64 {
    let mut packed = 0_u64;
    let mut index = 0;
    while index < 8 {
        packed |= (probabilities[offset + index] as u64) << (index * 8);
        index += 1;
    }
    packed
}

const fn unpack_probabilities(low: u64, high: u64) -> [u8; GATE_STEP_COUNT] {
    let mut probabilities = [0; GATE_STEP_COUNT];
    let mut index = 0;
    while index < 8 {
        probabilities[index] = ((low >> (index * 8)) & 0xff) as u8;
        probabilities[index + 8] = ((high >> (index * 8)) & 0xff) as u8;
        index += 1;
    }
    probabilities
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rack_persistence_round_trips_lfo_shape() {
        let source = ModulatorRackState::new();
        let config = SourceConfig {
            active: true,
            shape: 3,
            rate_mode: 2,
            sync_division: 6,
            gate_pattern: 0xa55a,
            gate_swing: 0.37,
            gate_probabilities: std::array::from_fn(|step| (step * 6) as u8),
            ..SourceConfig::default()
        };
        assert!(source.set_config(11, config));
        let mut bytes = Vec::new();
        source.persist_write(&mut bytes);

        let restored = ModulatorRackState::new();
        restored.persist_read(&mut StateCursor::new(&bytes));

        assert_eq!(restored.config(11), config);
        assert_eq!(restored.rt_config(11).shape, 3);
        assert_eq!(restored.rt_config(11).gate_pattern, 0xa55a);
        assert_eq!(restored.rt_config(11).gate_probabilities[15], 90);
    }

    #[test]
    fn older_rack_documents_default_to_curve_shape() {
        let legacy = RackDocument {
            version: ORDER_STATE_VERSION,
            sources: vec![SourceDocument {
                active: true,
                rate_hz: 3.0,
                ..SourceDocument::default()
            }],
            ..RackDocument::default()
        };
        let mut bytes = Vec::new();
        legacy.write_field(&mut bytes);
        let restored = ModulatorRackState::new();
        restored.persist_read(&mut StateCursor::new(&bytes));

        assert_eq!(restored.config(0).shape, 0);
    }
}
