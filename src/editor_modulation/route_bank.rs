//! Fixed modulation-route bank adapter for the editor.
//!
//! The first 16 slots retain their host parameter encoding. The remaining 48
//! use persisted overflow state while sharing the same source and target IDs.

use smallvec::SmallVec;
use truce_core::editor::{PluginContext, PluginContextReadF32};

use super::UiDestination;
use crate::modulation_target;
use crate::modulators::routing::{
    ExtraModulationRouteSnapshot, HOST_MODULATION_ROUTE_COUNT, MODULATION_ROUTE_COUNT,
    ModulationRouteTarget, ModulationRouteTargetSnapshot, ResolvedRouteSource,
};
use crate::modulators::state::{MAX_MODULATION_SOURCES, SourceKind};
use crate::{KurvParams, P};

const FRAME_ROUTE_CACHE_ID: &str = "kurv-direct-modulation-frame-routes";
const TARGET_COUNT: usize = modulation_target::TARGETS.len();
pub(super) const ROUTE_COUNT: usize = MODULATION_ROUTE_COUNT;
const HOST_ROUTE_COUNT: usize = HOST_MODULATION_ROUTE_COUNT;

pub(super) type UiRoute = (usize, ResolvedRouteSource, f32, bool);

#[derive(Clone, Default)]
pub(super) struct RouteBucket {
    entries: SmallVec<[UiRoute; 4]>,
}

impl RouteBucket {
    pub(super) fn as_slice(&self) -> &[UiRoute] {
        &self.entries
    }

    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn push(&mut self, route: UiRoute) {
        self.entries.push(route);
    }
}

#[derive(Clone, Copy)]
struct RouteScan {
    targets: ModulationRouteTargetSnapshot,
    overflow: ExtraModulationRouteSnapshot,
    mod_wheel_mask: u64,
}

#[derive(Clone, Copy, Default)]
struct ModularRouteMask {
    target: Option<ModulationRouteTarget>,
    routes: u64,
}

#[derive(Clone)]
struct FrameRouteCache {
    frame: u64,
    routes: [Option<UiRoute>; ROUTE_COUNT],
    destinations: [Option<UiDestination>; ROUTE_COUNT],
    host: [u64; TARGET_COUNT],
    modular: [ModularRouteMask; ROUTE_COUNT],
    modular_len: usize,
}

impl Default for FrameRouteCache {
    fn default() -> Self {
        Self {
            frame: u64::MAX,
            routes: [None; ROUTE_COUNT],
            destinations: [None; ROUTE_COUNT],
            host: [0; TARGET_COUNT],
            modular: [ModularRouteMask::default(); ROUTE_COUNT],
            modular_len: 0,
        }
    }
}

/// One lock-bounded view of route availability for a source-drag frame.
///
/// The overlay may paint dozens of visible destinations. Capturing the route
/// documents once keeps pointer motion from repeatedly taking the same state
/// locks for every highlight.
pub(super) struct RouteAssignmentSnapshot {
    host_free: bool,
    modular_free: bool,
    host_exact: [bool; TARGET_COUNT],
    modular_exact: [Option<ModulationRouteTarget>; ROUTE_COUNT],
    modular_exact_len: usize,
    destinations: [Option<UiDestination>; ROUTE_COUNT],
}

impl RouteAssignmentSnapshot {
    pub(super) fn capture(
        ui: &egui::Ui,
        state: &PluginContext<KurvParams>,
        source: ResolvedRouteSource,
    ) -> Self {
        let (destinations, routes) = frame_route_data(ui, state);
        let mut host_free = false;
        let mut modular_free = false;
        let mut host_exact = [false; TARGET_COUNT];
        let mut modular_exact = [None; ROUTE_COUNT];
        let mut modular_exact_len = 0;
        for route in 0..ROUTE_COUNT {
            let destination = destinations[route];
            modular_free |= destination.is_none();
            host_free |= route < HOST_ROUTE_COUNT && destination.is_none();
            if routes[route].is_none_or(|route| route.1 != source) {
                continue;
            }
            match destination {
                Some(UiDestination::Host(target)) if target > 0 => {
                    if let Some(exact) = host_exact.get_mut(usize::from(target - 1)) {
                        *exact = true;
                    }
                }
                Some(UiDestination::Modular(target)) => {
                    modular_exact[modular_exact_len] = Some(target);
                    modular_exact_len += 1;
                }
                _ => {}
            }
        }
        modular_exact[..modular_exact_len].sort_unstable();
        Self {
            host_free,
            modular_free,
            host_exact,
            modular_exact,
            modular_exact_len,
            destinations,
        }
    }

    pub(super) fn accepts_host(&self, target: u8) -> bool {
        self.host_free
            || target
                .checked_sub(1)
                .and_then(|target| self.host_exact.get(usize::from(target)))
                .copied()
                .unwrap_or(false)
    }

    pub(super) fn accepts_modular(&self, target: ModulationRouteTarget) -> bool {
        self.modular_free
            || self.modular_exact[..self.modular_exact_len]
                .binary_search(&Some(target))
                .is_ok()
    }

    pub(super) fn bank_full(&self) -> bool {
        !self.modular_free
    }

    pub(super) fn destinations(&self) -> &[Option<UiDestination>; ROUTE_COUNT] {
        &self.destinations
    }
}

impl RouteScan {
    fn capture(state: &PluginContext<KurvParams>) -> Self {
        Self {
            targets: state.params().modulation_route_targets.snapshot(),
            overflow: state.params().modulation_route_overflow.snapshot(),
            mod_wheel_mask: state.params().mod_wheel_route_mask.load(),
        }
    }

    fn source(
        &self,
        state: &PluginContext<KurvParams>,
        route: usize,
    ) -> Option<ResolvedRouteSource> {
        let encoded = if route < HOST_ROUTE_COUNT {
            host_route_source(state, ROUTES[route].0)
        } else {
            self.overflow[route - HOST_ROUTE_COUNT].source
        };
        ResolvedRouteSource::decode(encoded, self.mod_wheel_mask, route)
    }

    fn amount(&self, state: &PluginContext<KurvParams>, route: usize) -> f32 {
        if route < HOST_ROUTE_COUNT {
            state.get_param(ROUTES[route].2).mul_add(2.0, -1.0)
        } else {
            self.overflow[route - HOST_ROUTE_COUNT].amount
        }
    }

    fn destination(
        &self,
        state: &PluginContext<KurvParams>,
        route: usize,
    ) -> Option<UiDestination> {
        if let Some(target) = self.targets.get(route).copied().flatten() {
            return Some(UiDestination::Modular(target));
        }
        let target = route_target(state, route);
        (target != 0).then_some(UiDestination::Host(target))
    }
}

const ROUTES: [(P, P, P, P); HOST_ROUTE_COUNT] = [
    (
        P::Mod1Source,
        P::Mod1Target,
        P::Mod1Amount,
        P::Mod1TargetExt,
    ),
    (
        P::Mod2Source,
        P::Mod2Target,
        P::Mod2Amount,
        P::Mod2TargetExt,
    ),
    (
        P::Mod3Source,
        P::Mod3Target,
        P::Mod3Amount,
        P::Mod3TargetExt,
    ),
    (
        P::Mod4Source,
        P::Mod4Target,
        P::Mod4Amount,
        P::Mod4TargetExt,
    ),
    (
        P::Mod5Source,
        P::Mod5Target,
        P::Mod5Amount,
        P::Mod5TargetExt,
    ),
    (
        P::Mod6Source,
        P::Mod6Target,
        P::Mod6Amount,
        P::Mod6TargetExt,
    ),
    (
        P::Mod7Source,
        P::Mod7Target,
        P::Mod7Amount,
        P::Mod7TargetExt,
    ),
    (
        P::Mod8Source,
        P::Mod8Target,
        P::Mod8Amount,
        P::Mod8TargetExt,
    ),
    (
        P::Mod9Source,
        P::Mod9Target,
        P::Mod9Amount,
        P::Mod9TargetExt,
    ),
    (
        P::Mod10Source,
        P::Mod10Target,
        P::Mod10Amount,
        P::Mod10TargetExt,
    ),
    (
        P::Mod11Source,
        P::Mod11Target,
        P::Mod11Amount,
        P::Mod11TargetExt,
    ),
    (
        P::Mod12Source,
        P::Mod12Target,
        P::Mod12Amount,
        P::Mod12TargetExt,
    ),
    (
        P::Mod13Source,
        P::Mod13Target,
        P::Mod13Amount,
        P::Mod13TargetExt,
    ),
    (
        P::Mod14Source,
        P::Mod14Target,
        P::Mod14Amount,
        P::Mod14TargetExt,
    ),
    (
        P::Mod15Source,
        P::Mod15Target,
        P::Mod15Amount,
        P::Mod15TargetExt,
    ),
    (
        P::Mod16Source,
        P::Mod16Target,
        P::Mod16Amount,
        P::Mod16TargetExt,
    ),
];

impl FrameRouteCache {
    fn capture(frame: u64, state: &PluginContext<KurvParams>) -> Self {
        let scan = RouteScan::capture(state);
        let mut cache = Self {
            frame,
            ..Self::default()
        };
        for index in 0..ROUTE_COUNT {
            let destination = scan.destination(state, index);
            cache.destinations[index] = destination;
            let Some(destination) = destination else {
                continue;
            };
            let Some(source) = scan.source(state, index) else {
                continue;
            };
            let route = (
                index,
                source,
                scan.amount(state, index),
                source_is_bipolar(state, source),
            );
            cache.routes[index] = Some(route);
            let route_bit = 1_u64 << index;
            match destination {
                UiDestination::Host(target) => {
                    if let Some(routes) = target
                        .checked_sub(1)
                        .and_then(|target| cache.host.get_mut(usize::from(target)))
                    {
                        *routes |= route_bit;
                    }
                }
                UiDestination::Modular(target) => {
                    let entry = cache.modular[..cache.modular_len]
                        .iter()
                        .position(|entry| entry.target == Some(target))
                        .unwrap_or_else(|| {
                            let entry = cache.modular_len;
                            cache.modular[entry].target = Some(target);
                            cache.modular_len += 1;
                            entry
                        });
                    cache.modular[entry].routes |= route_bit;
                }
            }
        }
        cache.modular[..cache.modular_len].sort_unstable_by_key(|entry| entry.target);
        cache
    }

    fn bucket(&self, mut routes: u64) -> RouteBucket {
        let mut bucket = RouteBucket::default();
        while routes != 0 {
            let route = routes.trailing_zeros() as usize;
            routes &= routes - 1;
            if let Some(entry) = self.routes[route] {
                bucket.push(entry);
            }
        }
        bucket
    }

    fn source_bucket(&self, source: ResolvedRouteSource) -> RouteBucket {
        let mut bucket = RouteBucket::default();
        for route in self.routes.iter().flatten() {
            if route.1 == source {
                bucket.push(*route);
            }
        }
        bucket
    }
}

fn ensure_frame_route_cache(ui: &egui::Ui, state: &PluginContext<KurvParams>) {
    let frame = ui.ctx().cumulative_frame_nr();
    let id = egui::Id::new(FRAME_ROUTE_CACHE_ID);
    let stale =
        ui.data_mut(|data| data.get_temp_mut_or_default::<FrameRouteCache>(id).frame != frame);
    if stale {
        let cache = FrameRouteCache::capture(frame, state);
        ui.data_mut(|data| data.insert_temp(id, cache));
    }
}

fn frame_route_data(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
) -> (
    [Option<UiDestination>; ROUTE_COUNT],
    [Option<UiRoute>; ROUTE_COUNT],
) {
    ensure_frame_route_cache(ui, state);
    ui.data_mut(|data| {
        let cache =
            data.get_temp_mut_or_default::<FrameRouteCache>(egui::Id::new(FRAME_ROUTE_CACHE_ID));
        (cache.destinations, cache.routes)
    })
}

pub(super) fn routes_for_target(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    target: u8,
) -> RouteBucket {
    ensure_frame_route_cache(ui, state);
    let target_index = usize::from(target.saturating_sub(1));
    ui.data_mut(|data| {
        let cache =
            data.get_temp_mut_or_default::<FrameRouteCache>(egui::Id::new(FRAME_ROUTE_CACHE_ID));
        cache.bucket(cache.host.get(target_index).copied().unwrap_or_default())
    })
}

pub(super) fn routes_for_modular_target(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    target: ModulationRouteTarget,
) -> RouteBucket {
    ensure_frame_route_cache(ui, state);
    ui.data_mut(|data| {
        let cache =
            data.get_temp_mut_or_default::<FrameRouteCache>(egui::Id::new(FRAME_ROUTE_CACHE_ID));
        cache.modular[..cache.modular_len]
            .binary_search_by_key(&Some(target), |entry| entry.target)
            .map_or_else(
                |_| RouteBucket::default(),
                |entry| cache.bucket(cache.modular[entry].routes),
            )
    })
}

pub(super) fn routes_for_source(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    source: ResolvedRouteSource,
) -> RouteBucket {
    ensure_frame_route_cache(ui, state);
    ui.data_mut(|data| {
        data.get_temp_mut_or_default::<FrameRouteCache>(egui::Id::new(FRAME_ROUTE_CACHE_ID))
            .source_bucket(source)
    })
}

fn source_is_bipolar(state: &PluginContext<KurvParams>, source: ResolvedRouteSource) -> bool {
    let ResolvedRouteSource::Rack(source) = source else {
        return false;
    };
    let (kind, bipolar) = match source {
        0 => (P::Source1Envelope, P::Lfo1Bipolar),
        1 => (P::Source2Envelope, P::Lfo2Bipolar),
        2 => (P::Source3Envelope, P::Lfo3Bipolar),
        3 => (P::Source4Envelope, P::Lfo4Bipolar),
        4 => (P::Source5Envelope, P::Lfo5Bipolar),
        5 => (P::Source6Envelope, P::Lfo6Bipolar),
        6 => (P::Source7Envelope, P::Lfo7Bipolar),
        7 => (P::Source8Envelope, P::Lfo8Bipolar),
        _ => {
            let source = state.params().modulator_rack.config(usize::from(source));
            return source.kind == SourceKind::Lfo && source.bipolar;
        }
    };
    state.get_param(kind) < 0.5 && state.get_param(bipolar) >= 0.5
}

pub(super) fn assign_route(
    state: &PluginContext<KurvParams>,
    source: ResolvedRouteSource,
    target: u8,
) {
    let Some((route, exact)) = route_for_assignment(state, source, target) else {
        crate::diagnostics::trace(
            "modulation-route",
            "bank-full",
            f32::from(source.encoded()),
            target.into(),
        );
        return;
    };
    let (source_param, target_param, amount_param, ext_param) = ROUTES[route];
    state.params().modulation_route_targets.clear(route);
    if !exact {
        state.automate(amount_param, 0.5);
    }
    set_host_route_source(state, route, source, source_param);
    if target <= modulation_target::LEGACY_TARGET_COUNT {
        state.automate(
            target_param,
            f64::from(target) / f64::from(modulation_target::LEGACY_TARGET_COUNT),
        );
        state.automate(ext_param, 0.0);
    } else {
        state.automate(target_param, 0.0);
        state.automate(
            ext_param,
            f64::from(target - modulation_target::LEGACY_TARGET_COUNT)
                / f64::from(modulation_target::EXTENDED_TARGET_COUNT),
        );
    }
    if !exact {
        state.automate(amount_param, 0.625);
    }
}

pub(super) fn assign_modular_route(
    state: &PluginContext<KurvParams>,
    source: ResolvedRouteSource,
    target: ModulationRouteTarget,
) {
    let Some((route, exact)) = route_for_modular_assignment(state, source, target) else {
        crate::diagnostics::trace(
            "modulation-route",
            "bank-full-modular",
            f32::from(source.encoded()),
            0.0,
        );
        return;
    };
    if route < HOST_ROUTE_COUNT {
        let (source_param, target_param, amount_param, ext_param) = ROUTES[route];
        if !exact {
            state.automate(amount_param, 0.5);
        }
        set_host_route_source(state, route, source, source_param);
        state.automate(target_param, 0.0);
        state.automate(ext_param, 0.0);
        if !exact {
            state.automate(amount_param, 0.625);
        }
    } else if !exact {
        set_mod_wheel_route(state, route, false);
        state
            .params()
            .modulation_route_overflow
            .set(route, source.encoded(), 0.25);
        set_mod_wheel_route(state, route, source == ResolvedRouteSource::ModWheel);
    }
    state.params().modulation_route_targets.set(route, target);
}

pub(super) fn route_for_assignment(
    state: &PluginContext<KurvParams>,
    source: ResolvedRouteSource,
    target: u8,
) -> Option<(usize, bool)> {
    let modular_targets = state.params().modulation_route_targets.snapshot();
    if let Some(route) = (0..HOST_ROUTE_COUNT).find(|&route| {
        modular_targets[route].is_none()
            && route_source(state, route) == Some(source)
            && route_target(state, route) == target
    }) {
        return Some((route, true));
    }
    (0..HOST_ROUTE_COUNT)
        .find(|&route| modular_targets[route].is_none() && route_target(state, route) == 0)
        .map(|route| (route, false))
}

pub(super) fn route_for_modular_assignment(
    state: &PluginContext<KurvParams>,
    source: ResolvedRouteSource,
    target: ModulationRouteTarget,
) -> Option<(usize, bool)> {
    let routes = RouteScan::capture(state);
    if let Some(route) = (0..ROUTE_COUNT).find(|&route| {
        routes.source(state, route) == Some(source) && routes.targets[route] == Some(target)
    }) {
        return Some((route, true));
    }
    (0..ROUTE_COUNT)
        .find(|&route| routes.destination(state, route).is_none())
        .map(|route| (route, false))
}

pub(super) fn used_source_mask(state: &PluginContext<KurvParams>) -> u64 {
    let routes = RouteScan::capture(state);
    (0..ROUTE_COUNT).fold(0, |mask, route| {
        let source = routes.source(state, route);
        if let Some(ResolvedRouteSource::Rack(source)) = source
            && routes.destination(state, route).is_some()
            && routes.amount(state, route).abs() > f32::EPSILON
        {
            mask | (1_u64 << source)
        } else {
            mask
        }
    })
}

pub(super) fn clear_source(state: &PluginContext<KurvParams>, source: u8) {
    let source = ResolvedRouteSource::Rack(source.saturating_sub(1));
    let routes = RouteScan::capture(state);
    for route in 0..ROUTE_COUNT {
        if routes.source(state, route) == Some(source) {
            clear_route(state, route);
        }
    }
}

fn host_route_source(state: &PluginContext<KurvParams>, param: P) -> u8 {
    discrete_value(state.get_param(param), MAX_MODULATION_SOURCES as u8)
}

pub(super) fn route_source(
    state: &PluginContext<KurvParams>,
    route: usize,
) -> Option<ResolvedRouteSource> {
    let encoded = if route < HOST_ROUTE_COUNT {
        host_route_source(state, ROUTES[route].0)
    } else {
        state.params().modulation_route_overflow.get(route).source
    };
    ResolvedRouteSource::decode(encoded, state.params().mod_wheel_route_mask.load(), route)
}

fn set_mod_wheel_route(state: &PluginContext<KurvParams>, route: usize, enabled: bool) {
    let bit = 1_u64 << route;
    if enabled {
        state.params().mod_wheel_route_mask.fetch_or(bit);
    } else {
        state.params().mod_wheel_route_mask.fetch_and(!bit);
    }
}

fn set_host_route_source(
    state: &PluginContext<KurvParams>,
    route: usize,
    source: ResolvedRouteSource,
    source_param: P,
) {
    set_mod_wheel_route(state, route, false);
    state.automate(
        source_param,
        f64::from(source.encoded()) / MAX_MODULATION_SOURCES as f64,
    );
    if source == ResolvedRouteSource::ModWheel {
        set_mod_wheel_route(state, route, true);
    }
}

pub(super) fn route_amount(state: &PluginContext<KurvParams>, route: usize) -> f32 {
    if route < HOST_ROUTE_COUNT {
        state.get_param(ROUTES[route].2).mul_add(2.0, -1.0)
    } else {
        state.params().modulation_route_overflow.get(route).amount
    }
}

pub(super) fn set_route_amount(state: &PluginContext<KurvParams>, route: usize, amount: f32) {
    let amount = amount.clamp(-1.0, 1.0);
    if route < HOST_ROUTE_COUNT {
        state.set_param(ROUTES[route].2, f64::from(amount.mul_add(0.5, 0.5)));
    } else {
        state
            .params()
            .modulation_route_overflow
            .set_amount(route, amount);
    }
}

pub(super) fn begin_route_amount_edit(state: &PluginContext<KurvParams>, route: usize) {
    if route < HOST_ROUTE_COUNT {
        state.begin_edit(ROUTES[route].2);
    }
}

pub(super) fn end_route_amount_edit(state: &PluginContext<KurvParams>, route: usize) {
    if route < HOST_ROUTE_COUNT {
        state.end_edit(ROUTES[route].2);
    }
}

fn route_target(state: &PluginContext<KurvParams>, route: usize) -> u8 {
    if route >= HOST_ROUTE_COUNT {
        return 0;
    }
    let (_, target, _, extended) = ROUTES[route];
    let extension = discrete_value(
        state.get_param(extended),
        modulation_target::EXTENDED_TARGET_COUNT,
    );
    if extension == 0 {
        discrete_value(
            state.get_param(target),
            modulation_target::LEGACY_TARGET_COUNT,
        )
    } else {
        modulation_target::LEGACY_TARGET_COUNT + extension
    }
}

pub(super) fn route_destinations(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
) -> [Option<UiDestination>; ROUTE_COUNT] {
    ensure_frame_route_cache(ui, state);
    ui.data_mut(|data| {
        data.get_temp_mut_or_default::<FrameRouteCache>(egui::Id::new(FRAME_ROUTE_CACHE_ID))
            .destinations
    })
}

pub(super) fn clear_route(state: &PluginContext<KurvParams>, route: usize) {
    set_mod_wheel_route(state, route, false);
    if route < HOST_ROUTE_COUNT {
        let (source, target, amount, ext) = ROUTES[route];
        state.automate(amount, 0.5);
        state.automate(target, 0.0);
        state.automate(ext, 0.0);
        state.automate(source, 0.0);
    } else {
        state.params().modulation_route_overflow.clear(route);
    }
    state.params().modulation_route_targets.clear(route);
}

fn discrete_value(normalized: f32, maximum: u8) -> u8 {
    (normalized.clamp(0.0, 1.0) * f32::from(maximum)).round() as u8
}

pub(super) fn target_for_param(param: P) -> Option<u8> {
    modulation_target::target_for_param(param)
}

pub(super) fn display_span(target: u8) -> f32 {
    modulation_target::descriptor(target).map_or(1.0, |target| target.normalized_span)
}

pub(super) fn lfo_value_meter(
    state: &PluginContext<KurvParams>,
    source: ResolvedRouteSource,
) -> f32 {
    let params = state.params();
    let ResolvedRouteSource::Rack(source) = source else {
        return state.get_param(P::ModWheel);
    };
    let meter = match source {
        0 => &params.lfo1_value_meter,
        1 => &params.lfo2_value_meter,
        2 => &params.lfo3_value_meter,
        3 => &params.lfo4_value_meter,
        4 => &params.lfo5_value_meter,
        5 => &params.lfo6_value_meter,
        6 => &params.lfo7_value_meter,
        7 => &params.lfo8_value_meter,
        _ => return params.modulator_rack.ui_snapshot(usize::from(source)).1,
    };
    state.get_meter(meter)
}
