//! Persisted mixed modulation rack with a fixed lock-free runtime snapshot.

use std::sync::{
    RwLock,
    atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering},
};

use truce::State;
use truce_core::custom_state::{PersistField, StateCursor, StateField};

use crate::wave_curve::{WaveCurveData, WaveCurveState};

pub const MAX_MODULATION_SOURCES: usize = 64;
pub const LEGACY_MODULATION_SOURCES: usize = 8;
const INITIAL_STATE_VERSION: u32 = 1;
const PREVIOUS_STATE_VERSION: u32 = 2;
const STATE_VERSION: u32 = 3;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum SourceKind {
    #[default]
    Lfo,
    Envelope,
}

impl SourceKind {
    pub const fn from_index(index: u8) -> Self {
        if index == 1 {
            Self::Envelope
        } else {
            Self::Lfo
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
    pub attack: f32,
    pub attack_curve: f32,
    pub decay: f32,
    pub decay_curve: f32,
    pub sustain: f32,
    pub release: f32,
    pub release_curve: f32,
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
            attack: finite_or(self.attack, 0.01).clamp(0.0, 8.0),
            attack_curve: finite_or(self.attack_curve, 0.0).clamp(-1.0, 1.0),
            decay: finite_or(self.decay, 0.1).clamp(0.0, 8.0),
            decay_curve: finite_or(self.decay_curve, 0.0).clamp(-1.0, 1.0),
            sustain: finite_or(self.sustain, 0.8).clamp(0.0, 1.0),
            release: finite_or(self.release, 0.2).clamp(0.0, 12.0),
            release_curve: finite_or(self.release_curve, 0.0).clamp(-1.0, 1.0),
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

struct RtSourceConfig {
    active: AtomicU8,
    kind: AtomicU8,
    rate_hz: AtomicU32,
    rate_mode: AtomicU8,
    mode: AtomicU8,
    phase_offset: AtomicU32,
    sync_division: AtomicU8,
    bipolar: AtomicU8,
    attack: AtomicU32,
    attack_curve: AtomicU32,
    decay: AtomicU32,
    decay_curve: AtomicU32,
    sustain: AtomicU32,
    release: AtomicU32,
    release_curve: AtomicU32,
}

impl RtSourceConfig {
    fn new(config: SourceConfig) -> Self {
        Self {
            active: AtomicU8::new(u8::from(config.active)),
            kind: AtomicU8::new(config.kind as u8),
            rate_hz: AtomicU32::new(config.rate_hz.to_bits()),
            rate_mode: AtomicU8::new(config.rate_mode),
            mode: AtomicU8::new(config.mode),
            phase_offset: AtomicU32::new(config.phase_offset.to_bits()),
            sync_division: AtomicU8::new(config.sync_division),
            bipolar: AtomicU8::new(u8::from(config.bipolar)),
            attack: AtomicU32::new(config.attack.to_bits()),
            attack_curve: AtomicU32::new(config.attack_curve.to_bits()),
            decay: AtomicU32::new(config.decay.to_bits()),
            decay_curve: AtomicU32::new(config.decay_curve.to_bits()),
            sustain: AtomicU32::new(config.sustain.to_bits()),
            release: AtomicU32::new(config.release.to_bits()),
            release_curve: AtomicU32::new(config.release_curve.to_bits()),
        }
    }

    fn store(&self, config: SourceConfig) {
        let config = config.sanitized();
        self.active
            .store(u8::from(config.active), Ordering::Relaxed);
        self.kind.store(config.kind as u8, Ordering::Relaxed);
        self.rate_hz
            .store(config.rate_hz.to_bits(), Ordering::Relaxed);
        self.rate_mode.store(config.rate_mode, Ordering::Relaxed);
        self.mode.store(config.mode, Ordering::Relaxed);
        self.phase_offset
            .store(config.phase_offset.to_bits(), Ordering::Relaxed);
        self.sync_division
            .store(config.sync_division, Ordering::Relaxed);
        self.bipolar
            .store(u8::from(config.bipolar), Ordering::Relaxed);
        self.attack
            .store(config.attack.to_bits(), Ordering::Relaxed);
        self.attack_curve
            .store(config.attack_curve.to_bits(), Ordering::Relaxed);
        self.decay.store(config.decay.to_bits(), Ordering::Relaxed);
        self.decay_curve
            .store(config.decay_curve.to_bits(), Ordering::Relaxed);
        self.sustain
            .store(config.sustain.to_bits(), Ordering::Relaxed);
        self.release
            .store(config.release.to_bits(), Ordering::Relaxed);
        self.release_curve
            .store(config.release_curve.to_bits(), Ordering::Relaxed);
    }

    fn load(&self) -> SourceConfig {
        SourceConfig {
            active: self.active.load(Ordering::Relaxed) != 0,
            kind: SourceKind::from_index(self.kind.load(Ordering::Relaxed)),
            rate_hz: f32::from_bits(self.rate_hz.load(Ordering::Relaxed)),
            rate_mode: self.rate_mode.load(Ordering::Relaxed),
            mode: self.mode.load(Ordering::Relaxed),
            phase_offset: f32::from_bits(self.phase_offset.load(Ordering::Relaxed)),
            sync_division: self.sync_division.load(Ordering::Relaxed),
            bipolar: self.bipolar.load(Ordering::Relaxed) != 0,
            attack: f32::from_bits(self.attack.load(Ordering::Relaxed)),
            attack_curve: f32::from_bits(self.attack_curve.load(Ordering::Relaxed)),
            decay: f32::from_bits(self.decay.load(Ordering::Relaxed)),
            decay_curve: f32::from_bits(self.decay_curve.load(Ordering::Relaxed)),
            sustain: f32::from_bits(self.sustain.load(Ordering::Relaxed)),
            release: f32::from_bits(self.release.load(Ordering::Relaxed)),
            release_curve: f32::from_bits(self.release_curve.load(Ordering::Relaxed)),
        }
    }
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
        }
    }
}

impl SourceDocument {
    fn into_config(self) -> SourceConfig {
        SourceConfig {
            active: self.active,
            kind: SourceKind::from_index(self.kind),
            rate_hz: self.rate_hz,
            rate_mode: self.rate_mode,
            mode: self.mode,
            phase_offset: self.phase_offset,
            sync_division: self.sync_division,
            bipolar: self.bipolar,
            attack: self.attack,
            attack_curve: self.attack_curve,
            decay: self.decay,
            decay_curve: self.decay_curve,
            sustain: self.sustain,
            release: self.release,
            release_curve: self.release_curve,
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
            rt_sources: boxed_from_fn(|_| RtSourceConfig::new(SourceConfig::default())),
            curves: boxed_from_fn(|_| WaveCurveState::default()),
            ui_phases: boxed_from_fn(|_| AtomicU32::new(0)),
            ui_values: boxed_from_fn(|_| AtomicU32::new(0)),
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

    pub fn publish_ui_snapshot(&self, phases: &[f32], values: &[f32]) {
        for index in 0..MAX_MODULATION_SOURCES.min(phases.len()).min(values.len()) {
            self.ui_phases[index].store(phases[index].to_bits(), Ordering::Relaxed);
            self.ui_values[index].store(values[index].to_bits(), Ordering::Relaxed);
        }
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
                INITIAL_STATE_VERSION | PREVIOUS_STATE_VERSION | STATE_VERSION
            )
        }) else {
            return;
        };
        let presentation_order = normalized_presentation_order(&document.presentation_order);
        let mut sources = boxed_array(SourceConfig::default());
        for (target, source) in sources.iter_mut().zip(document.sources) {
            *target = source.into_config();
        }
        for (target, curve) in self.curves.iter().zip(document.curves) {
            target.replace(curve);
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
