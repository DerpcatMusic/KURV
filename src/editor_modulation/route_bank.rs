//! Fixed modulation-route bank adapter for the editor.
//!
//! The first 16 slots retain their host parameter encoding. The remaining 48
//! use persisted overflow state while sharing the same source and target IDs.

use truce_core::editor::{PluginContext, PluginContextReadF32};

use super::UiDestination;
use crate::modulation_target;
use crate::modulators::routing::{
    ExtraModulationRouteSnapshot, HOST_MODULATION_ROUTE_COUNT, MODULATION_ROUTE_COUNT,
    ModulationRouteTarget, ModulationRouteTargetSnapshot, ResolvedRouteSource,
};
use crate::modulators::state::{MAX_MODULATION_SOURCES, SourceKind};
use crate::{KurvParams, P};

const ROUTE_CACHE_ID: &str = "kurv-direct-modulation-routes";
const TARGET_COUNT: usize = modulation_target::TARGETS.len();
const TARGET_COUNT_U8: u8 = TARGET_COUNT as u8;
pub(super) const ROUTE_COUNT: usize = MODULATION_ROUTE_COUNT;
const HOST_ROUTE_COUNT: usize = HOST_MODULATION_ROUTE_COUNT;

pub(super) type UiRoute = (usize, ResolvedRouteSource, f32, bool);

#[derive(Clone, Copy)]
pub(super) struct RouteBucket {
    entries: [UiRoute; ROUTE_COUNT],
    pub(super) len: usize,
}

impl Default for RouteBucket {
    fn default() -> Self {
        Self {
            entries: [(0, ResolvedRouteSource::Rack(0), 0.0, false); ROUTE_COUNT],
            len: 0,
        }
    }
}

impl RouteBucket {
    pub(super) fn as_slice(&self) -> &[UiRoute] {
        &self.entries[..self.len]
    }
}

struct RouteScan {
    targets: ModulationRouteTargetSnapshot,
    overflow: ExtraModulationRouteSnapshot,
    mod_wheel_mask: u64,
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

#[derive(Clone)]
struct RouteCache {
    frame: u64,
    targets: [RouteBucket; TARGET_COUNT],
}

impl Default for RouteCache {
    fn default() -> Self {
        Self {
            frame: u64::MAX,
            targets: [RouteBucket::default(); TARGET_COUNT],
        }
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

pub(super) fn routes_for_target(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    target: u8,
) -> RouteBucket {
    let frame = ui.ctx().cumulative_frame_nr();
    let id = egui::Id::new(ROUTE_CACHE_ID);
    ui.data_mut(|data| {
        let cache = data.get_temp_mut_or_default::<RouteCache>(id);
        if cache.frame != frame {
            cache.frame = frame;
            cache.targets.fill(RouteBucket::default());
            let mod_wheel_mask = state.params().mod_wheel_route_mask.load();
            let modular_targets = state.params().modulation_route_targets.snapshot();
            for (index, (source, _, amount, _)) in ROUTES.iter().enumerate() {
                if modular_targets[index].is_some() {
                    continue;
                }
                let source = ResolvedRouteSource::decode(
                    host_route_source(state, *source),
                    mod_wheel_mask,
                    index,
                );
                let destination = route_target(state, index);
                let Some(source) = source else {
                    continue;
                };
                if destination == 0 || destination > TARGET_COUNT_U8 {
                    continue;
                }
                let bucket = &mut cache.targets[usize::from(destination - 1)];
                bucket.entries[bucket.len] = (
                    index,
                    source,
                    state.get_param(*amount).mul_add(2.0, -1.0),
                    source_is_bipolar(state, source),
                );
                bucket.len += 1;
            }
        }
        cache.targets[usize::from(target - 1)]
    })
}

pub(super) fn routes_for_modular_target(
    state: &PluginContext<KurvParams>,
    target: ModulationRouteTarget,
) -> RouteBucket {
    let mut bucket = RouteBucket::default();
    let routes = RouteScan::capture(state);
    for index in 0..ROUTE_COUNT {
        if routes.targets[index] != Some(target) {
            continue;
        }
        let Some(source) = routes.source(state, index) else {
            continue;
        };
        bucket.entries[bucket.len] = (
            index,
            source,
            routes.amount(state, index),
            source_is_bipolar(state, source),
        );
        bucket.len += 1;
    }
    bucket
}

pub(super) fn routes_for_source(
    state: &PluginContext<KurvParams>,
    source: ResolvedRouteSource,
) -> RouteBucket {
    let mut bucket = RouteBucket::default();
    let routes = RouteScan::capture(state);
    for index in 0..ROUTE_COUNT {
        if routes.source(state, index) != Some(source) || bucket.len == bucket.entries.len() {
            continue;
        }
        if routes.destination(state, index).is_none() {
            continue;
        }
        bucket.entries[bucket.len] = (
            index,
            source,
            routes.amount(state, index),
            source_is_bipolar(state, source),
        );
        bucket.len += 1;
    }
    bucket
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
    state: &PluginContext<KurvParams>,
) -> [Option<UiDestination>; ROUTE_COUNT] {
    let modular_targets = state.params().modulation_route_targets.snapshot();
    std::array::from_fn(|route| {
        if let Some(target) = modular_targets[route] {
            Some(UiDestination::Modular(target))
        } else {
            let target = route_target(state, route);
            (target != 0).then_some(UiDestination::Host(target))
        }
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

pub(super) fn effective_normalized(state: &PluginContext<KurvParams>, param: P) -> f32 {
    let Some(target) = target_for_param(param) else {
        return state.get_param(param);
    };
    let mut value = state.get_param(param);
    let modular_targets = state.params().modulation_route_targets.snapshot();
    for (index, (_, _, amount, _)) in ROUTES.iter().enumerate() {
        if modular_targets[index].is_some() || route_target(state, index) != target {
            continue;
        }
        let Some(source) = route_source(state, index) else {
            continue;
        };
        let source_value = lfo_value_meter(state, source);
        let amount = state.get_param(*amount).mul_add(2.0, -1.0);
        value += source_value * amount * display_span(target);
    }
    value.clamp(0.0, 1.0)
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
