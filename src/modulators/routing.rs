//! Persisted modular modulation targets with lock-free audio publication.

use std::sync::{
    RwLock,
    atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering},
};

use truce::State;
use truce_core::custom_state::{PersistField, StateCursor, StateField};

use crate::generators::{
    FilterConfig, FilterSlot, GroupId, GroupOutput, MAX_FILTERS, MAX_OSCILLATORS, ModuleId,
    OscillatorConfig, OscillatorSlot,
};

pub const HOST_MODULATION_ROUTE_COUNT: usize = 16;
pub const MODULATION_ROUTE_COUNT: usize = 64;
pub const EXTRA_MODULATION_ROUTE_COUNT: usize =
    MODULATION_ROUTE_COUNT - HOST_MODULATION_ROUTE_COUNT;
pub const HOST_AUTOMATION_SLOT_COUNT: usize = 64;
const STATE_VERSION: u32 = 1;
const TARGET_NONE: u8 = 0;
const TARGET_OSCILLATOR: u8 = 1;
const TARGET_GROUP: u8 = 2;
const TARGET_FILTER: u8 = 3;

/// A live modulation source after the persisted route encoding has been
/// resolved. Rack indices are zero-based and always stay inside the 64-source
/// bank; the performance wheel is deliberately not represented as index 64.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum ResolvedRouteSource {
    Rack(u8),
    ModWheel,
}

impl ResolvedRouteSource {
    pub(crate) const fn decode(encoded: u8, mod_wheel_mask: u64, route: usize) -> Option<Self> {
        if encoded == 0 {
            if mod_wheel_mask & (1_u64 << route) != 0 {
                Some(Self::ModWheel)
            } else {
                None
            }
        } else if encoded <= MODULATION_ROUTE_COUNT as u8 {
            Some(Self::Rack(encoded - 1))
        } else {
            None
        }
    }

    pub(crate) const fn encoded(self) -> u8 {
        match self {
            Self::Rack(index) => index + 1,
            Self::ModWheel => 0,
        }
    }

    pub(crate) const fn rack_index(self) -> Option<usize> {
        match self {
            Self::Rack(index) => Some(index as usize),
            Self::ModWheel => None,
        }
    }
}

/// Continuous controls addressable on a modular oscillator.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum OscillatorControl {
    Shape = 0,
    TablePosition = 1,
    PulseWidth = 2,
    Transpose = 3,
    Cents = 4,
    Level = 5,
    Pan = 6,
    PhasePosition = 7,
    PhaseRandom = 8,
    PhaseWarpAmount = 9,
    UnisonVoices = 10,
    UnisonRange = 11,
    UnisonAmount = 12,
    UnisonCurve = 13,
    UnisonJitter = 14,
    UnisonRate = 15,
    UnisonWidth = 16,
    UnisonWeight = 17,
    UnisonAlignment = 18,
    UnisonPanCurve = 19,
    UnisonPanCenter = 20,
    UnisonStereoPosition = 21,
    UnisonStereoAlternate = 22,
}

impl OscillatorControl {
    pub(crate) const INTERNAL_TARGET_COUNT: usize = 10;

    pub(crate) const fn supports_internal_modulation(self) -> bool {
        matches!(
            self,
            Self::Shape
                | Self::PulseWidth
                | Self::Transpose
                | Self::Cents
                | Self::Level
                | Self::Pan
                | Self::PhasePosition
                | Self::PhaseWarpAmount
                | Self::UnisonJitter
                | Self::UnisonRate
        )
    }

    const fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            0 => Some(Self::Shape),
            1 => Some(Self::TablePosition),
            2 => Some(Self::PulseWidth),
            3 => Some(Self::Transpose),
            4 => Some(Self::Cents),
            5 => Some(Self::Level),
            6 => Some(Self::Pan),
            7 => Some(Self::PhasePosition),
            8 => Some(Self::PhaseRandom),
            9 => Some(Self::PhaseWarpAmount),
            10 => Some(Self::UnisonVoices),
            11 => Some(Self::UnisonRange),
            12 => Some(Self::UnisonAmount),
            13 => Some(Self::UnisonCurve),
            14 => Some(Self::UnisonJitter),
            15 => Some(Self::UnisonRate),
            16 => Some(Self::UnisonWidth),
            17 => Some(Self::UnisonWeight),
            18 => Some(Self::UnisonAlignment),
            19 => Some(Self::UnisonPanCurve),
            20 => Some(Self::UnisonPanCenter),
            21 => Some(Self::UnisonStereoPosition),
            22 => Some(Self::UnisonStereoAlternate),
            _ => None,
        }
    }

    pub(crate) fn apply_normalized(self, config: &mut OscillatorConfig, normalized: f32) {
        let value = normalized.clamp(0.0, 1.0);
        match self {
            Self::Shape => config.shape = value * 3.0,
            Self::TablePosition => config.custom_shape = value,
            Self::PulseWidth => config.pulse_width = value.mul_add(0.94, 0.03),
            Self::Transpose => config.transpose = value.mul_add(96.0, -48.0),
            Self::Cents => config.cents = value.mul_add(200.0, -100.0),
            Self::Level => config.level = value,
            Self::Pan => config.pan = value.mul_add(2.0, -1.0),
            Self::PhasePosition => config.phase_position = value,
            Self::PhaseRandom => config.phase_random = value,
            Self::PhaseWarpAmount => config.phase_warp_amount = value,
            Self::UnisonVoices => config.unison_voices = value.mul_add(63.0, 1.0).round() as u8,
            Self::UnisonRange => config.unison_range = value * 48.0,
            Self::UnisonAmount => config.unison_amount = value,
            Self::UnisonCurve => config.unison_curve = value.mul_add(2.0, -1.0),
            Self::UnisonJitter => config.unison_jitter = value,
            Self::UnisonRate => config.unison_rate = value,
            Self::UnisonWidth => config.unison_width = value,
            Self::UnisonWeight => config.unison_weight = value.mul_add(2.0, -1.0),
            Self::UnisonAlignment => config.unison_alignment = value,
            Self::UnisonPanCurve => config.unison_pan_curve = value.mul_add(2.0, -1.0),
            Self::UnisonPanCenter => config.unison_pan_center_x = value.mul_add(0.90, 0.05),
            Self::UnisonStereoPosition => config.unison_stereo_x = value,
            Self::UnisonStereoAlternate => config.unison_stereo_alternate = value,
        }
    }

    pub(crate) fn normalized_value(self, config: OscillatorConfig) -> f32 {
        match self {
            Self::Shape => config.shape / 3.0,
            Self::TablePosition => config.custom_shape,
            Self::PulseWidth => (config.pulse_width - 0.03) / 0.94,
            Self::Transpose => config.transpose / 96.0 + 0.5,
            Self::Cents => config.cents / 200.0 + 0.5,
            Self::Level => config.level,
            Self::Pan => config.pan.mul_add(0.5, 0.5),
            Self::PhasePosition => config.phase_position,
            Self::PhaseRandom => config.phase_random,
            Self::PhaseWarpAmount => config.phase_warp_amount,
            Self::UnisonVoices => (f32::from(config.unison_voices) - 1.0) / 63.0,
            Self::UnisonRange => config.unison_range / 48.0,
            Self::UnisonAmount => config.unison_amount,
            Self::UnisonCurve => config.unison_curve.mul_add(0.5, 0.5),
            Self::UnisonJitter => config.unison_jitter,
            Self::UnisonRate => config.unison_rate,
            Self::UnisonWidth => config.unison_width,
            Self::UnisonWeight => config.unison_weight.mul_add(0.5, 0.5),
            Self::UnisonAlignment => config.unison_alignment,
            Self::UnisonPanCurve => config.unison_pan_curve.mul_add(0.5, 0.5),
            Self::UnisonPanCenter => (config.unison_pan_center_x - 0.05) / 0.90,
            Self::UnisonStereoPosition => config.unison_stereo_x,
            Self::UnisonStereoAlternate => config.unison_stereo_alternate,
        }
        .clamp(0.0, 1.0)
    }
}

/// Continuous controls addressable on an ordered generator filter.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum FilterControl {
    Cutoff = 0,
    Resonance = 1,
}

impl FilterControl {
    pub(crate) const INTERNAL_TARGET_COUNT: usize = 2;

    const fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            0 => Some(Self::Cutoff),
            1 => Some(Self::Resonance),
            _ => None,
        }
    }

    pub(crate) fn apply_normalized(self, config: &mut FilterConfig, normalized: f32) {
        let value = normalized.clamp(0.0, 1.0);
        match self {
            Self::Cutoff => config.cutoff_hz = 20.0 * 1_000.0_f32.powf(value),
            Self::Resonance => config.q = 0.1 * 320.0_f32.powf(value),
        }
    }

    pub(crate) fn normalized_value(self, config: FilterConfig) -> f32 {
        match self {
            Self::Cutoff => (config.cutoff_hz.clamp(20.0, 20_000.0) / 20.0).ln() / 1_000.0_f32.ln(),
            Self::Resonance => (config.q.clamp(0.1, 32.0) / 0.1).ln() / 320.0_f32.ln(),
        }
        .clamp(0.0, 1.0)
    }
}

/// Continuous controls addressable on a generator group output.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum GroupControl {
    Gain = 0,
    Pan = 1,
    Attack = 2,
    AttackCurve = 3,
    Decay = 4,
    DecayCurve = 5,
    Sustain = 6,
    Release = 7,
    ReleaseCurve = 8,
}

impl GroupControl {
    pub(crate) const INTERNAL_TARGET_COUNT: usize = 2;

    pub(crate) const fn supports_internal_modulation(self) -> bool {
        matches!(self, Self::Gain | Self::Pan)
    }

    const fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            0 => Some(Self::Gain),
            1 => Some(Self::Pan),
            2 => Some(Self::Attack),
            3 => Some(Self::AttackCurve),
            4 => Some(Self::Decay),
            5 => Some(Self::DecayCurve),
            6 => Some(Self::Sustain),
            7 => Some(Self::Release),
            8 => Some(Self::ReleaseCurve),
            _ => None,
        }
    }

    pub(crate) fn apply_normalized(self, output: &mut GroupOutput, normalized: f32) {
        let value = normalized.clamp(0.0, 1.0);
        match self {
            Self::Gain => output.gain = value * 2.0,
            Self::Pan => output.pan = value.mul_add(2.0, -1.0),
            Self::Attack => output.attack = value * 20.0,
            Self::AttackCurve => output.attack_curve = value.mul_add(2.0, -1.0),
            Self::Decay => output.decay = value * 20.0,
            Self::DecayCurve => output.decay_curve = value.mul_add(2.0, -1.0),
            Self::Sustain => output.sustain = value,
            Self::Release => output.release = value * 20.0,
            Self::ReleaseCurve => output.release_curve = value.mul_add(2.0, -1.0),
        }
    }

    pub(crate) fn normalized_value(self, output: GroupOutput) -> f32 {
        match self {
            Self::Gain => output.gain * 0.5,
            Self::Pan => output.pan.mul_add(0.5, 0.5),
            Self::Attack => output.attack / 20.0,
            Self::AttackCurve => output.attack_curve.mul_add(0.5, 0.5),
            Self::Decay => output.decay / 20.0,
            Self::DecayCurve => output.decay_curve.mul_add(0.5, 0.5),
            Self::Sustain => output.sustain,
            Self::Release => output.release / 20.0,
            Self::ReleaseCurve => output.release_curve.mul_add(0.5, 0.5),
        }
        .clamp(0.0, 1.0)
    }
}

/// One modular destination. Numeric IDs remain stable across reordering and
/// match the IDs published by the generator stack's realtime snapshot.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ModulationRouteTarget {
    Oscillator {
        module_id: u64,
        slot: OscillatorSlot,
        control: OscillatorControl,
    },
    Group {
        group_id: u64,
        control: GroupControl,
    },
    Filter {
        module_id: u64,
        slot: FilterSlot,
        control: FilterControl,
    },
}

impl ModulationRouteTarget {
    #[must_use]
    pub const fn oscillator(
        module_id: ModuleId,
        slot: OscillatorSlot,
        control: OscillatorControl,
    ) -> Self {
        Self::Oscillator {
            module_id: module_id.get(),
            slot,
            control,
        }
    }

    #[must_use]
    pub const fn group(group_id: GroupId, control: GroupControl) -> Self {
        Self::Group {
            group_id: group_id.get(),
            control,
        }
    }

    #[must_use]
    pub const fn filter(module_id: ModuleId, slot: FilterSlot, control: FilterControl) -> Self {
        Self::Filter {
            module_id: module_id.get(),
            slot,
            control,
        }
    }

    pub(crate) const fn supports_internal_modulation(self) -> bool {
        match self {
            Self::Oscillator { control, .. } => control.supports_internal_modulation(),
            Self::Group { control, .. } => control.supports_internal_modulation(),
            Self::Filter { .. } => true,
        }
    }

    fn sanitized(self) -> Option<Self> {
        match self {
            Self::Oscillator {
                module_id, slot, ..
            } if module_id != 0 && slot.index() < MAX_OSCILLATORS => Some(self),
            Self::Group { group_id, .. } if group_id != 0 => Some(self),
            Self::Filter {
                module_id, slot, ..
            } if module_id != 0 && slot.index() < MAX_FILTERS => Some(self),
            _ => None,
        }
    }
}

pub type ModulationRouteTargetSnapshot = [Option<ModulationRouteTarget>; MODULATION_ROUTE_COUNT];
pub type HostAutomationTargetSnapshot = [Option<ModulationRouteTarget>; HOST_AUTOMATION_SLOT_COUNT];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExtraModulationRoute {
    pub source: u8,
    pub amount: f32,
}

impl ExtraModulationRoute {
    pub const EMPTY: Self = Self {
        source: 0,
        amount: 0.0,
    };

    fn sanitized(self) -> Self {
        Self {
            source: self.source.min(64),
            amount: if self.amount.is_finite() {
                self.amount.clamp(-1.0, 1.0)
            } else {
                0.0
            },
        }
    }
}

pub type ExtraModulationRouteSnapshot = [ExtraModulationRoute; EXTRA_MODULATION_ROUTE_COUNT];

struct AtomicRouteTarget {
    kind: AtomicU8,
    identity: AtomicU64,
    slot: AtomicU8,
    control: AtomicU8,
}

impl AtomicRouteTarget {
    fn new() -> Self {
        Self {
            kind: AtomicU8::new(TARGET_NONE),
            identity: AtomicU64::new(0),
            slot: AtomicU8::new(0),
            control: AtomicU8::new(0),
        }
    }

    fn store(&self, target: Option<ModulationRouteTarget>) {
        let (kind, identity, slot, control) = encode_target(target);
        self.kind.store(kind, Ordering::Relaxed);
        self.identity.store(identity, Ordering::Relaxed);
        self.slot.store(slot, Ordering::Relaxed);
        self.control.store(control, Ordering::Relaxed);
    }

    fn load(&self) -> Option<ModulationRouteTarget> {
        decode_target(
            self.kind.load(Ordering::Relaxed),
            self.identity.load(Ordering::Relaxed),
            self.slot.load(Ordering::Relaxed),
            self.control.load(Ordering::Relaxed),
        )
    }
}

#[derive(Default, State)]
struct RouteDocument {
    kind: u8,
    identity: u64,
    slot: u8,
    control: u8,
}

impl RouteDocument {
    fn from_target(target: Option<ModulationRouteTarget>) -> Self {
        let (kind, identity, slot, control) = encode_target(target);
        Self {
            kind,
            identity,
            slot,
            control,
        }
    }

    fn into_target(self) -> Option<ModulationRouteTarget> {
        decode_target(self.kind, self.identity, self.slot, self.control)
    }
}

#[derive(Default, State)]
struct RoutingDocument {
    version: u32,
    routes: Vec<RouteDocument>,
}

/// Fixed-capacity stable target bank. Editor/state access may lock; audio
/// access reads only fixed atomics guarded by a generation counter.
pub struct RouteTargetState<const SLOTS: usize> {
    document: RwLock<[Option<ModulationRouteTarget>; SLOTS]>,
    rt_generation: AtomicU32,
    rt_routes: [AtomicRouteTarget; SLOTS],
}

impl<const SLOTS: usize> RouteTargetState<SLOTS> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            document: RwLock::new([None; SLOTS]),
            rt_generation: AtomicU32::new(0),
            rt_routes: std::array::from_fn(|_| AtomicRouteTarget::new()),
        }
    }

    /// Returns one editor-side target, or `None` for an empty/invalid route.
    #[must_use]
    pub fn get(&self, route: usize) -> Option<ModulationRouteTarget> {
        self.document
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(route)
            .copied()
            .flatten()
    }

    /// Copies the complete editor-side bank under one read lock.
    #[must_use]
    pub fn snapshot(&self) -> [Option<ModulationRouteTarget>; SLOTS] {
        *self
            .document
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn restore_snapshot(&self, routes: [Option<ModulationRouteTarget>; SLOTS]) {
        self.replace(routes);
    }

    pub(crate) fn clear_all(&self) {
        self.replace([None; SLOTS]);
    }

    /// Assigns one sanitized modular target. Returns whether state changed.
    pub fn set(&self, route: usize, target: ModulationRouteTarget) -> bool {
        let Some(target) = target.sanitized() else {
            return false;
        };
        let mut document = self
            .document
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(current) = document.get_mut(route) else {
            return false;
        };
        if *current == Some(target) {
            return false;
        }
        *current = Some(target);
        self.publish_route(route, Some(target));
        true
    }

    /// Clears one route. Returns whether state changed.
    pub fn clear(&self, route: usize) -> bool {
        let mut document = self
            .document
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(current) = document.get_mut(route) else {
            return false;
        };
        if current.is_none() {
            return false;
        }
        *current = None;
        self.publish_route(route, None);
        true
    }

    /// Clears every route owned by one removed oscillator module.
    pub fn clear_module(&self, module_id: u64) -> usize {
        self.clear_matching(|target| {
            matches!(
                target,
                ModulationRouteTarget::Oscillator { module_id: id, .. }
                    | ModulationRouteTarget::Filter { module_id: id, .. }
                    if id == module_id
            )
        })
    }

    /// Clears every route owned by one removed generator group.
    pub fn clear_group(&self, group_id: u64) -> usize {
        self.clear_matching(|target| {
            matches!(target, ModulationRouteTarget::Group { group_id: id, .. } if id == group_id)
        })
    }

    fn clear_matching(&self, predicate: impl Fn(ModulationRouteTarget) -> bool) -> usize {
        let mut document = self
            .document
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut cleared = 0;
        for (route, target) in document.iter_mut().enumerate() {
            if target.as_ref().copied().is_some_and(&predicate) {
                if cleared == 0 {
                    self.rt_generation.fetch_add(1, Ordering::AcqRel);
                }
                *target = None;
                self.rt_routes[route].store(None);
                cleared += 1;
            }
        }
        if cleared != 0 {
            self.rt_generation.fetch_add(1, Ordering::Release);
        }
        cleared
    }

    /// Copies a coherent snapshot only when its published generation changed.
    /// This path is bounded, allocation-free, and lock-free for audio use.
    #[must_use]
    pub fn try_rt_snapshot_after(
        &self,
        observed_generation: u32,
    ) -> Option<(u32, [Option<ModulationRouteTarget>; SLOTS])> {
        let before = self.rt_generation.load(Ordering::Acquire);
        if before == observed_generation || before & 1 != 0 {
            return None;
        }
        let routes = std::array::from_fn(|index| self.rt_routes[index].load());
        std::sync::atomic::fence(Ordering::Acquire);
        (before == self.rt_generation.load(Ordering::Relaxed)).then_some((before, routes))
    }

    fn publish_route(&self, route: usize, target: Option<ModulationRouteTarget>) {
        self.rt_generation.fetch_add(1, Ordering::AcqRel);
        self.rt_routes[route].store(target);
        self.rt_generation.fetch_add(1, Ordering::Release);
    }

    fn replace(&self, routes: [Option<ModulationRouteTarget>; SLOTS]) {
        let mut document = self
            .document
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *document = routes;
        self.rt_generation.fetch_add(1, Ordering::AcqRel);
        for (target, route) in self.rt_routes.iter().zip(routes) {
            target.store(route);
        }
        self.rt_generation.fetch_add(1, Ordering::Release);
    }
}

impl<const SLOTS: usize> Default for RouteTargetState<SLOTS> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const SLOTS: usize> PersistField for RouteTargetState<SLOTS> {
    fn persist_write(&self, buf: &mut Vec<u8>) {
        let document = self
            .document
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        RoutingDocument {
            version: STATE_VERSION,
            routes: document
                .iter()
                .copied()
                .map(RouteDocument::from_target)
                .collect(),
        }
        .write_field(buf);
    }

    fn persist_read(&self, cursor: &mut StateCursor) {
        let Some(document) =
            RoutingDocument::read_field(cursor).filter(|state| state.version == STATE_VERSION)
        else {
            return;
        };
        let mut routes = [None; SLOTS];
        for (target, persisted) in routes.iter_mut().zip(document.routes) {
            *target = persisted.into_target();
        }
        self.replace(routes);
    }
}

pub type ModulationRouteTargetState = RouteTargetState<MODULATION_ROUTE_COUNT>;
pub type HostAutomationTargetState = RouteTargetState<HOST_AUTOMATION_SLOT_COUNT>;

struct AtomicExtraRoute {
    source: AtomicU8,
    amount: AtomicU32,
}

impl AtomicExtraRoute {
    fn new() -> Self {
        Self {
            source: AtomicU8::new(0),
            amount: AtomicU32::new(0.0_f32.to_bits()),
        }
    }

    fn store(&self, route: ExtraModulationRoute) {
        let route = route.sanitized();
        self.source.store(route.source, Ordering::Relaxed);
        self.amount.store(route.amount.to_bits(), Ordering::Relaxed);
    }

    fn load(&self) -> ExtraModulationRoute {
        ExtraModulationRoute {
            source: self.source.load(Ordering::Relaxed),
            amount: f32::from_bits(self.amount.load(Ordering::Relaxed)),
        }
        .sanitized()
    }
}

#[derive(Default, State)]
struct ExtraRouteDocument {
    source: u8,
    amount: f32,
}

impl From<ExtraModulationRoute> for ExtraRouteDocument {
    fn from(route: ExtraModulationRoute) -> Self {
        Self {
            source: route.source,
            amount: route.amount,
        }
    }
}

impl ExtraRouteDocument {
    fn into_route(self) -> ExtraModulationRoute {
        ExtraModulationRoute {
            source: self.source,
            amount: self.amount,
        }
        .sanitized()
    }
}

#[derive(Default, State)]
struct ExtraRoutesDocument {
    version: u32,
    routes: Vec<ExtraRouteDocument>,
}

/// Internal-only overflow routes beyond the 16 stable host route parameters.
/// Editor access may lock; audio reads a coherent fixed atomic snapshot.
pub struct ExtraModulationRouteState {
    document: RwLock<ExtraModulationRouteSnapshot>,
    rt_generation: AtomicU32,
    rt_routes: [AtomicExtraRoute; EXTRA_MODULATION_ROUTE_COUNT],
}

impl ExtraModulationRouteState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            document: RwLock::new([ExtraModulationRoute::EMPTY; EXTRA_MODULATION_ROUTE_COUNT]),
            rt_generation: AtomicU32::new(0),
            rt_routes: std::array::from_fn(|_| AtomicExtraRoute::new()),
        }
    }

    fn local_index(route: usize) -> Option<usize> {
        route
            .checked_sub(HOST_MODULATION_ROUTE_COUNT)
            .filter(|index| *index < EXTRA_MODULATION_ROUTE_COUNT)
    }

    #[must_use]
    pub fn get(&self, route: usize) -> ExtraModulationRoute {
        let Some(index) = Self::local_index(route) else {
            return ExtraModulationRoute::EMPTY;
        };
        self.document
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())[index]
    }

    /// Copies the complete editor-side overflow bank under one read lock.
    #[must_use]
    pub fn snapshot(&self) -> ExtraModulationRouteSnapshot {
        *self
            .document
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn restore_snapshot(&self, routes: ExtraModulationRouteSnapshot) {
        self.replace(routes);
    }

    pub fn set(&self, route: usize, source: u8, amount: f32) -> bool {
        let Some(index) = Self::local_index(route) else {
            return false;
        };
        let value = ExtraModulationRoute { source, amount }.sanitized();
        let mut document = self
            .document
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if document[index] == value {
            return false;
        }
        document[index] = value;
        self.publish(index, value);
        true
    }

    pub fn set_amount(&self, route: usize, amount: f32) -> bool {
        let current = self.get(route);
        self.set(route, current.source, amount)
    }

    pub fn clear(&self, route: usize) -> bool {
        self.set(route, 0, 0.0)
    }

    #[must_use]
    pub fn try_rt_snapshot_after(
        &self,
        observed_generation: u32,
    ) -> Option<(u32, ExtraModulationRouteSnapshot)> {
        let before = self.rt_generation.load(Ordering::Acquire);
        if before == observed_generation || before & 1 != 0 {
            return None;
        }
        let routes = std::array::from_fn(|index| self.rt_routes[index].load());
        std::sync::atomic::fence(Ordering::Acquire);
        (before == self.rt_generation.load(Ordering::Relaxed)).then_some((before, routes))
    }

    fn publish(&self, index: usize, route: ExtraModulationRoute) {
        self.rt_generation.fetch_add(1, Ordering::AcqRel);
        self.rt_routes[index].store(route);
        self.rt_generation.fetch_add(1, Ordering::Release);
    }

    fn replace(&self, routes: ExtraModulationRouteSnapshot) {
        let mut document = self
            .document
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *document = routes;
        self.rt_generation.fetch_add(1, Ordering::AcqRel);
        for (atomic, route) in self.rt_routes.iter().zip(routes) {
            atomic.store(route);
        }
        self.rt_generation.fetch_add(1, Ordering::Release);
    }
}

impl Default for ExtraModulationRouteState {
    fn default() -> Self {
        Self::new()
    }
}

impl PersistField for ExtraModulationRouteState {
    fn persist_write(&self, buf: &mut Vec<u8>) {
        let document = self
            .document
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        ExtraRoutesDocument {
            version: STATE_VERSION,
            routes: document.iter().copied().map(Into::into).collect(),
        }
        .write_field(buf);
    }

    fn persist_read(&self, cursor: &mut StateCursor) {
        let Some(document) =
            ExtraRoutesDocument::read_field(cursor).filter(|state| state.version == STATE_VERSION)
        else {
            return;
        };
        let mut routes = [ExtraModulationRoute::EMPTY; EXTRA_MODULATION_ROUTE_COUNT];
        for (route, persisted) in routes.iter_mut().zip(document.routes) {
            *route = persisted.into_route();
        }
        self.replace(routes);
    }
}

fn encode_target(target: Option<ModulationRouteTarget>) -> (u8, u64, u8, u8) {
    match target {
        Some(ModulationRouteTarget::Oscillator {
            module_id,
            slot,
            control,
        }) => (
            TARGET_OSCILLATOR,
            module_id,
            slot.index() as u8,
            control as u8,
        ),
        Some(ModulationRouteTarget::Group { group_id, control }) => {
            (TARGET_GROUP, group_id, 0, control as u8)
        }
        Some(ModulationRouteTarget::Filter {
            module_id,
            slot,
            control,
        }) => (TARGET_FILTER, module_id, slot.index() as u8, control as u8),
        None => (TARGET_NONE, 0, 0, 0),
    }
}

fn decode_target(kind: u8, identity: u64, slot: u8, control: u8) -> Option<ModulationRouteTarget> {
    if identity == 0 {
        return None;
    }
    match kind {
        TARGET_OSCILLATOR => Some(ModulationRouteTarget::Oscillator {
            module_id: identity,
            slot: OscillatorSlot::from_index(usize::from(slot))?,
            control: OscillatorControl::from_tag(control)?,
        }),
        TARGET_GROUP => Some(ModulationRouteTarget::Group {
            group_id: identity,
            control: GroupControl::from_tag(control)?,
        }),
        TARGET_FILTER => Some(ModulationRouteTarget::Filter {
            module_id: identity,
            slot: FilterSlot::from_index(usize::from(slot))?,
            control: FilterControl::from_tag(control)?,
        }),
        _ => None,
    }
}
