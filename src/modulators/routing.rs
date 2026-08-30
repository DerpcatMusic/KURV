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
use crate::modulation_target;

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
const TARGET_LEGACY: u8 = 4;

/// A live modulation source after the persisted route encoding has been
/// resolved. Rack indices are zero-based and always stay inside the 64-source
/// bank. Performance and XY sources use mutually-exclusive sidecar masks so
/// the stable host source parameters keep their historical `0..=64` range.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum ResolvedRouteSource {
    Rack(u8),
    Generator(u8),
    ModWheel,
    XyX,
    XyY,
}

impl ResolvedRouteSource {
    pub(crate) const fn decode(
        encoded: u8,
        mod_wheel_mask: u64,
        xy_x_mask: u64,
        xy_y_mask: u64,
        route: usize,
    ) -> Option<Self> {
        if encoded == 0 {
            let bit = 1_u64 << route;
            if xy_x_mask & bit != 0 {
                Some(Self::XyX)
            } else if xy_y_mask & bit != 0 {
                Some(Self::XyY)
            } else if mod_wheel_mask & bit != 0 {
                Some(Self::ModWheel)
            } else {
                None
            }
        } else if encoded <= MODULATION_ROUTE_COUNT as u8 {
            Some(Self::Rack(encoded - 1))
        } else if encoded <= MODULATION_ROUTE_COUNT as u8 + MAX_OSCILLATORS as u8 {
            Some(Self::Generator(encoded - MODULATION_ROUTE_COUNT as u8 - 1))
        } else {
            None
        }
    }

    pub(crate) const fn encoded(self) -> u8 {
        match self {
            Self::Rack(index) => index + 1,
            Self::Generator(index) => MODULATION_ROUTE_COUNT as u8 + index + 1,
            Self::ModWheel | Self::XyX | Self::XyY => 0,
        }
    }

    pub(crate) const fn rack_index(self) -> Option<usize> {
        match self {
            Self::Rack(index) => Some(index as usize),
            Self::Generator(_) | Self::ModWheel | Self::XyX | Self::XyY => None,
        }
    }
}

macro_rules! control_catalog {
    (
        $(#[$attribute:meta])*
        pub enum $name:ident {
            $($variant:ident = $tag:literal => $label:literal, internal = $internal:literal),+ $(,)?
        }
    ) => {
        $(#[$attribute])*
        #[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
        #[repr(u8)]
        pub enum $name {
            $($variant = $tag),+
        }

        impl $name {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];
            pub(crate) const INTERNAL_TARGET_COUNT: usize = 0 $(+ $internal as usize)+;

            pub(crate) const fn supports_internal_modulation(self) -> bool {
                match self {
                    $(Self::$variant => $internal),+
                }
            }

            const fn from_tag(tag: u8) -> Option<Self> {
                match tag {
                    $($tag => Some(Self::$variant),)+
                    _ => None,
                }
            }

            pub(crate) const fn label(self) -> &'static str {
                match self {
                    $(Self::$variant => $label),+
                }
            }
        }
    };
}

control_catalog! {
    /// Continuous controls addressable on a modular oscillator.
    pub enum OscillatorControl {
        Shape = 0 => "SHAPE", internal = true,
        TablePosition = 1 => "VA POSITION", internal = false,
        PulseWidth = 2 => "PULSE", internal = true,
        Transpose = 3 => "SEMI", internal = true,
        Cents = 4 => "CENT", internal = true,
        Level = 5 => "LEVEL", internal = true,
        Pan = 6 => "PAN", internal = true,
        PhasePosition = 7 => "PHASE", internal = true,
        PhaseRandom = 8 => "RANDOM PHASE", internal = false,
        PhaseWarpAmount = 9 => "WARP", internal = true,
        UnisonVoices = 10 => "VOICES", internal = false,
        UnisonRange = 11 => "RANGE", internal = false,
        UnisonAmount = 12 => "DETUNE", internal = false,
        UnisonCurve = 13 => "DISTRIBUTION", internal = false,
        UnisonJitter = 14 => "JITTER", internal = true,
        UnisonRate = 15 => "JITTER RATE", internal = true,
        UnisonWidth = 16 => "WIDTH", internal = false,
        UnisonWeight = 17 => "WEIGHT", internal = false,
        UnisonAlignment = 18 => "ALIGN", internal = false,
        UnisonPanCurve = 19 => "PAN SHAPE", internal = false,
        UnisonPanCenter = 20 => "PAN CENTER", internal = false,
        UnisonStereoPosition = 21 => "PAN X · AUDIO", internal = true,
        UnisonStereoAlternate = 22 => "PAN Y · AUDIO", internal = true,
        GrainTune = 23 => "GRAIN TUNE", internal = true,
        GrainStereo = 24 => "GRAIN STEREO", internal = true,
        RichBalance = 25 => "RICH BALANCE", internal = false,
        RichFormant = 26 => "RICH FORMANT", internal = false,
        RichAir = 27 => "RICH AIR", internal = false,
        RichDiffuse = 28 => "RICH DIFFUSE", internal = false,
        RichDynamic = 29 => "RICH DYNAMIC", internal = true,
        PhaseModAmount = 30 => "PM DEPTH", internal = false,
    }
}

impl OscillatorControl {
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
            Self::PhaseModAmount => config.phase_mod_amount = value.mul_add(2.0, -1.0),
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
            Self::GrainTune
            | Self::GrainStereo
            | Self::RichBalance
            | Self::RichFormant
            | Self::RichAir
            | Self::RichDiffuse
            | Self::RichDynamic => {}
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
            Self::PhaseModAmount => config.phase_mod_amount.mul_add(0.5, 0.5),
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
            Self::GrainTune
            | Self::GrainStereo
            | Self::RichBalance
            | Self::RichFormant
            | Self::RichAir
            | Self::RichDiffuse
            | Self::RichDynamic => 0.0,
        }
        .clamp(0.0, 1.0)
    }

    pub(crate) fn apply_resynth_normalized(
        self,
        controls: &mut crate::oscillators::ResynthControls,
        normalized: f32,
    ) -> bool {
        let value = normalized.clamp(0.0, 1.0);
        match self {
            Self::GrainTune => controls.grain_tune = value,
            Self::GrainStereo => controls.grain_stereo = value,
            Self::RichBalance => controls.rich_balance = value.mul_add(2.0, -1.0),
            Self::RichFormant => controls.rich_formant_semitones = value.mul_add(48.0, -24.0),
            Self::RichAir => controls.rich_air_db = value.mul_add(24.0, -12.0),
            Self::RichDiffuse => controls.rich_diffuse = value,
            Self::RichDynamic => controls.rich_dynamic = value,
            _ => return false,
        }
        true
    }
}

control_catalog! {
    /// Continuous controls addressable on an ordered generator filter.
    pub enum FilterControl {
        Cutoff = 0 => "CUTOFF", internal = true,
        Resonance = 1 => "RESONANCE", internal = true,
        Slope = 2 => "DB/OCT", internal = true,
        Morph = 3 => "MORPH", internal = true,
        Shape = 4 => "SHAPE", internal = true,
    }
}

impl FilterControl {
    pub(crate) fn apply_normalized(self, config: &mut FilterConfig, normalized: f32) {
        let value = normalized.clamp(0.0, 1.0);
        match self {
            Self::Cutoff => config.cutoff_hz = 20.0 * 1_000.0_f32.powf(value),
            Self::Resonance => config.q = 0.1 * 320.0_f32.powf(value),
            Self::Slope => {
                config.slope_db_oct = crate::filters::MIN_SLOPE_DB
                    * (crate::filters::MAX_SLOPE_DB / crate::filters::MIN_SLOPE_DB).powf(value);
            }
            Self::Morph => config.morph = value,
            Self::Shape => config.shape = value,
        }
    }

    pub(crate) fn normalized_value(self, config: FilterConfig) -> f32 {
        match self {
            Self::Cutoff => (config.cutoff_hz.clamp(20.0, 20_000.0) / 20.0).ln() / 1_000.0_f32.ln(),
            Self::Resonance => (config.q.clamp(0.1, 32.0) / 0.1).ln() / 320.0_f32.ln(),
            Self::Slope => {
                (config
                    .slope_db_oct
                    .clamp(crate::filters::MIN_SLOPE_DB, crate::filters::MAX_SLOPE_DB)
                    / crate::filters::MIN_SLOPE_DB)
                    .ln()
                    / (crate::filters::MAX_SLOPE_DB / crate::filters::MIN_SLOPE_DB).ln()
            }
            Self::Morph => config.morph.clamp(0.0, 1.0),
            Self::Shape => config.shape.clamp(0.0, 1.0),
        }
        .clamp(0.0, 1.0)
    }
}

control_catalog! {
    /// Continuous controls addressable on a generator group output.
    pub enum GroupControl {
        Gain = 0 => "GAIN", internal = true,
        Pan = 1 => "PAN", internal = true,
        Attack = 2 => "ATTACK", internal = false,
        AttackCurve = 3 => "ATTACK CURVE · AUDIO", internal = true,
        Decay = 4 => "DECAY", internal = false,
        DecayCurve = 5 => "DECAY CURVE · AUDIO", internal = true,
        Sustain = 6 => "SUSTAIN", internal = false,
        Release = 7 => "RELEASE", internal = false,
        ReleaseCurve = 8 => "RELEASE CURVE · AUDIO", internal = true,
        Dry = 9 => "DRY", internal = false,
        Send = 10 => "PARALLEL SEND", internal = false,
        Sidechain = 11 => "SIDECHAIN DEPTH", internal = false,
    }
}

impl GroupControl {
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
            Self::Dry => output.dry = value,
            Self::Send => output.send = value,
            Self::Sidechain => output.sidechain = value,
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
            Self::Dry => output.dry,
            Self::Send => output.send,
            Self::Sidechain => output.sidechain,
        }
        .clamp(0.0, 1.0)
    }
}

/// One modular destination. Numeric IDs remain stable across reordering and
/// match the IDs published by the generator stack's realtime snapshot.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ModulationRouteTarget {
    Legacy {
        target: u8,
    },
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
    pub const fn legacy(target: u8) -> Self {
        Self::Legacy { target }
    }

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
            Self::Legacy { .. } => false,
            Self::Oscillator { control, .. } => control.supports_internal_modulation(),
            Self::Group { control, .. } => control.supports_internal_modulation(),
            Self::Filter { control, .. } => control.supports_internal_modulation(),
        }
    }

    fn sanitized(self) -> Option<Self> {
        match self {
            Self::Legacy { target } if modulation_target::descriptor(target).is_some() => {
                Some(self)
            }
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
            source: self
                .source
                .min(MODULATION_ROUTE_COUNT as u8 + MAX_OSCILLATORS as u8),
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
        Some(ModulationRouteTarget::Legacy { target }) => (TARGET_LEGACY, u64::from(target), 0, 0),
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
        TARGET_LEGACY => {
            let target = u8::try_from(identity).ok()?;
            modulation_target::descriptor(target)
                .is_some()
                .then_some(ModulationRouteTarget::Legacy { target })
        }
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

#[cfg(test)]
mod tests {
    use super::ResolvedRouteSource;

    #[test]
    fn xy_route_sidecars_preserve_the_stable_source_encoding() {
        let route = 11;
        let bit = 1_u64 << route;
        assert_eq!(ResolvedRouteSource::XyX.encoded(), 0);
        assert_eq!(ResolvedRouteSource::XyY.encoded(), 0);
        assert_eq!(
            ResolvedRouteSource::decode(0, 0, bit, 0, route),
            Some(ResolvedRouteSource::XyX)
        );
        assert_eq!(
            ResolvedRouteSource::decode(0, 0, 0, bit, route),
            Some(ResolvedRouteSource::XyY)
        );
    }

    #[test]
    fn encoded_rack_sources_ignore_special_sidecars() {
        let route = 7;
        let bit = 1_u64 << route;
        assert_eq!(
            ResolvedRouteSource::decode(64, bit, bit, bit, route),
            Some(ResolvedRouteSource::Rack(63))
        );
    }

    #[test]
    fn filter_slope_normalized_covers_six_db_to_brickwall() {
        let mut config = crate::filters::FilterConfig::default();
        super::FilterControl::Slope.apply_normalized(&mut config, 0.0);
        assert!((config.slope_db_oct - crate::filters::MIN_SLOPE_DB).abs() < 0.01);
        super::FilterControl::Slope.apply_normalized(&mut config, 1.0);
        assert!((config.slope_db_oct - crate::filters::MAX_SLOPE_DB).abs() < 0.5);
        assert!((super::FilterControl::Slope.normalized_value(config) - 1.0).abs() < 0.001);
    }

    #[test]
    fn corrupted_special_sidecars_decode_deterministically() {
        let route = 3;
        let bit = 1_u64 << route;
        assert_eq!(
            ResolvedRouteSource::decode(0, bit, bit, bit, route),
            Some(ResolvedRouteSource::XyX)
        );
    }
}
