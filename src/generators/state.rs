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

use super::{GroupOutput, MAX_OSCILLATORS, ModuleKind, OscillatorSlot, Patch};

const STATE_VERSION: u32 = 1;
const OSCILLATOR_KIND: u8 = 0;
const LEGACY_OSCILLATOR_MASK: u32 = 0b111;
const FILTER_KIND: u8 = 1;

/// Non-host-exposed controls for one oscillator slot.
///
/// The first three slots continue to use their existing host parameters. This
/// value supplies the extensible slots while KURV's macro layer is built.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OscillatorConfig {
    pub enabled: bool,
    pub shape: f32,
    pub pulse_width: f32,
    pub transpose: f32,
    pub cents: f32,
    pub level: f32,
    pub pan: f32,
}

impl OscillatorConfig {
    fn sanitized(self) -> Self {
        Self {
            enabled: self.enabled,
            shape: finite_or(self.shape, 2.0).clamp(0.0, 3.0),
            pulse_width: finite_or(self.pulse_width, 0.5).clamp(0.03, 0.97),
            transpose: finite_or(self.transpose, 0.0).clamp(-48.0, 48.0),
            cents: finite_or(self.cents, 0.0).clamp(-100.0, 100.0),
            level: finite_or(self.level, 0.5).clamp(0.0, 1.0),
            pan: finite_or(self.pan, 0.0).clamp(-1.0, 1.0),
        }
    }
}

impl Default for OscillatorConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            shape: 2.0,
            pulse_width: 0.5,
            transpose: 0.0,
            cents: 0.0,
            level: 0.5,
            pan: 0.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GeneratorStackSnapshot {
    patch: Patch,
    oscillators: [OscillatorConfig; MAX_OSCILLATORS],
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
    pulse_width: AtomicU32,
    transpose: AtomicU32,
    cents: AtomicU32,
    level: AtomicU32,
    pan: AtomicU32,
}

impl RtOscillatorConfig {
    fn new(config: OscillatorConfig) -> Self {
        Self {
            enabled: AtomicBool::new(config.enabled),
            shape: AtomicU32::new(config.shape.to_bits()),
            pulse_width: AtomicU32::new(config.pulse_width.to_bits()),
            transpose: AtomicU32::new(config.transpose.to_bits()),
            cents: AtomicU32::new(config.cents.to_bits()),
            level: AtomicU32::new(config.level.to_bits()),
            pan: AtomicU32::new(config.pan.to_bits()),
        }
    }

    fn store(&self, config: OscillatorConfig) {
        let config = config.sanitized();
        self.enabled.store(config.enabled, Ordering::Relaxed);
        self.shape.store(config.shape.to_bits(), Ordering::Relaxed);
        self.pulse_width
            .store(config.pulse_width.to_bits(), Ordering::Relaxed);
        self.transpose
            .store(config.transpose.to_bits(), Ordering::Relaxed);
        self.cents.store(config.cents.to_bits(), Ordering::Relaxed);
        self.level.store(config.level.to_bits(), Ordering::Relaxed);
        self.pan.store(config.pan.to_bits(), Ordering::Relaxed);
    }

    fn load(&self) -> OscillatorConfig {
        OscillatorConfig {
            enabled: self.enabled.load(Ordering::Relaxed),
            shape: f32::from_bits(self.shape.load(Ordering::Relaxed)),
            pulse_width: f32::from_bits(self.pulse_width.load(Ordering::Relaxed)),
            transpose: f32::from_bits(self.transpose.load(Ordering::Relaxed)),
            cents: f32::from_bits(self.cents.load(Ordering::Relaxed)),
            level: f32::from_bits(self.level.load(Ordering::Relaxed)),
            pan: f32::from_bits(self.pan.load(Ordering::Relaxed)),
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
}

impl Default for GroupDocument {
    fn default() -> Self {
        Self {
            id: 0,
            modules: vec![ModuleDocument::default()],
            output_pair: 0,
            output_gain: 1.0,
            output_pan: 0.0,
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
            pulse_width: config.pulse_width,
            transpose: config.transpose,
            cents: config.cents,
            level: config.level,
            pan: config.pan,
        }
    }

    fn into_config(self) -> OscillatorConfig {
        OscillatorConfig {
            enabled: self.enabled,
            shape: self.shape,
            pulse_width: self.pulse_width,
            transpose: self.transpose,
            cents: self.cents,
            level: self.level,
            pan: self.pan,
        }
        .sanitized()
    }
}

impl StackDocument {
    fn from_document(document: &GeneratorDocument, materialized: bool) -> Self {
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
                })
                .collect(),
            oscillators: document
                .oscillators
                .iter()
                .copied()
                .map(OscillatorDocument::from_config)
                .collect(),
        }
    }

    fn into_document(self) -> Option<(GeneratorDocument, bool)> {
        if self.version != STATE_VERSION || self.next_group_id == 0 || self.next_module_id == 0 {
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
                },
                modules,
            ));
        }

        let patch = Patch::restore(groups, self.next_group_id, self.next_module_id).ok()?;
        let mut oscillators = [OscillatorConfig::default(); MAX_OSCILLATORS];
        for (target, stored) in oscillators.iter_mut().zip(self.oscillators) {
            *target = stored.into_config();
        }
        Some((GeneratorDocument { patch, oscillators }, self.materialized))
    }
}

/// Editable generator storage with a fixed lock-free audio snapshot.
pub struct GeneratorStackState {
    document: RwLock<GeneratorDocument>,
    materialized: AtomicBool,
    rt_generation: AtomicU32,
    rt_active_mask: AtomicU32,
    rt_oscillators: [RtOscillatorConfig; MAX_OSCILLATORS],
    rt_output_pair: AtomicU8,
    rt_output_gain: AtomicU32,
    rt_output_pan: AtomicU32,
}

impl GeneratorStackState {
    #[must_use]
    pub fn new() -> Self {
        let document = GeneratorDocument::default();
        let active_mask = active_oscillator_mask(&document.patch);
        Self {
            document: RwLock::new(document),
            materialized: AtomicBool::new(false),
            rt_generation: AtomicU32::new(0),
            rt_active_mask: AtomicU32::new(active_mask),
            rt_oscillators: std::array::from_fn(|_| {
                RtOscillatorConfig::new(OscillatorConfig::default())
            }),
            rt_output_pair: AtomicU8::new(0),
            rt_output_gain: AtomicU32::new(1.0_f32.to_bits()),
            rt_output_pan: AtomicU32::new(0.0_f32.to_bits()),
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

    pub fn set_oscillator_config(&self, slot: OscillatorSlot, config: OscillatorConfig) {
        let mut document = self
            .document
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        document.oscillators[slot.index()] = config.sanitized();
        self.publish_rt(&document, true);
    }

    /// Attempts one bounded, allocation-free coherent read for the audio
    /// callback. Callers retain their previous snapshot on contention.
    #[must_use]
    pub fn try_rt_state(&self) -> Option<([OscillatorConfig; MAX_OSCILLATORS], GroupOutput, u32)> {
        let mut snapshot = [OscillatorConfig::default(); MAX_OSCILLATORS];
        let before = self.rt_generation.load(Ordering::Acquire);
        if before & 1 != 0 {
            return None;
        }
        let active = self.rt_active_mask.load(Ordering::Relaxed);
        let effective_active = if self.materialized.load(Ordering::Relaxed) {
            active
        } else {
            LEGACY_OSCILLATOR_MASK
        };
        for (index, (target, source)) in snapshot.iter_mut().zip(&self.rt_oscillators).enumerate() {
            *target = source.load();
            target.enabled &= effective_active & (1_u32 << index) != 0;
        }
        let output = GroupOutput {
            pair: self.rt_output_pair.load(Ordering::Relaxed),
            gain: f32::from_bits(self.rt_output_gain.load(Ordering::Relaxed)),
            pan: f32::from_bits(self.rt_output_pan.load(Ordering::Relaxed)),
        };
        (before == self.rt_generation.load(Ordering::Acquire)).then_some((
            snapshot,
            output,
            effective_active,
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
        }
    }

    pub(crate) fn restore_snapshot(&self, snapshot: &GeneratorStackSnapshot) {
        let mut document = self
            .document
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        document.patch = snapshot.patch.clone();
        document.oscillators = snapshot.oscillators;
        self.publish_rt(&document, true);
    }

    pub(crate) fn reset_legacy(&self) {
        let mut document = self
            .document
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *document = GeneratorDocument::default();
        self.publish_rt(&document, false);
    }

    fn publish_rt(&self, document: &GeneratorDocument, materialized: bool) {
        self.rt_generation.fetch_add(1, Ordering::AcqRel);
        for (target, config) in self.rt_oscillators.iter().zip(document.oscillators) {
            target.store(config);
        }
        self.rt_active_mask
            .store(active_oscillator_mask(&document.patch), Ordering::Relaxed);
        let output = document
            .patch
            .groups()
            .first()
            .map_or_else(GroupOutput::default, super::Group::output)
            .sanitized();
        self.rt_output_pair.store(output.pair, Ordering::Relaxed);
        self.rt_output_gain
            .store(output.gain.to_bits(), Ordering::Relaxed);
        self.rt_output_pan
            .store(output.pan.to_bits(), Ordering::Relaxed);
        self.materialized.store(materialized, Ordering::Release);
        self.rt_generation.fetch_add(1, Ordering::Release);
    }
}

impl Default for GeneratorStackState {
    fn default() -> Self {
        Self::new()
    }
}

impl PersistField for GeneratorStackState {
    fn persist_write(&self, buf: &mut Vec<u8>) {
        let document = self
            .document
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        StackDocument::from_document(&document, self.is_materialized()).write_field(buf);
    }

    fn persist_read(&self, cursor: &mut StateCursor) {
        let Some((loaded, materialized)) =
            StackDocument::read_field(cursor).and_then(StackDocument::into_document)
        else {
            return;
        };
        let mut document = self
            .document
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *document = loaded;
        self.publish_rt(&document, materialized);
    }
}

fn active_oscillator_mask(patch: &Patch) -> u32 {
    // The current renderer publishes one group stem. Keep later groups silent
    // until stems are split rather than leaking them through group 1's output.
    patch
        .groups()
        .first()
        .into_iter()
        .flat_map(super::Group::modules)
        .filter_map(|module| module.oscillator_slot())
        .fold(0, |mask, slot| mask | (1_u32 << slot.index()))
}

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}
