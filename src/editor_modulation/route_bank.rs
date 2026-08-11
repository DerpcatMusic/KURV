//! Fixed modulation-route bank adapter for the editor.
//!
//! The first 16 slots retain their host parameter encoding. The remaining 48
//! use persisted overflow state while sharing the same source and target IDs.

use truce::params::Params;
use truce_core::editor::{PluginContext, PluginContextReadF32};

use crate::modulation_target;
use crate::modulators::routing::{
    HOST_MODULATION_ROUTE_COUNT, MODULATION_ROUTE_COUNT, ModulationRouteTarget, ResolvedRouteSource,
};
use crate::modulators::state::MAX_MODULATION_SOURCES;
use crate::{KurvParams, P};

mod cache;
mod host_slots;

use cache::RouteScan;
pub(super) use cache::{
    RouteAssignmentSnapshot, RouteBucket, route_destinations, routes_for_modular_target,
    routes_for_source, routes_for_target,
};
use host_slots::ROUTES;

pub(super) const ROUTE_COUNT: usize = MODULATION_ROUTE_COUNT;
const HOST_ROUTE_COUNT: usize = HOST_MODULATION_ROUTE_COUNT;

pub(super) type UiRoute = (usize, ResolvedRouteSource, f32, bool);

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
    if exact {
        return;
    }
    let (source_param, target_param, amount_param, ext_param) = ROUTES[route];
    state.params().modulation_route_targets.clear(route);
    automate_if_changed(state, amount_param, 0.5);
    set_host_route_source(state, route, source, source_param);
    if target <= modulation_target::LEGACY_TARGET_COUNT {
        automate_if_changed(
            state,
            target_param,
            f64::from(target) / f64::from(modulation_target::LEGACY_TARGET_COUNT),
        );
        automate_if_changed(state, ext_param, 0.0);
    } else {
        automate_if_changed(state, target_param, 0.0);
        automate_if_changed(
            state,
            ext_param,
            f64::from(target - modulation_target::LEGACY_TARGET_COUNT)
                / f64::from(modulation_target::EXTENDED_TARGET_COUNT),
        );
    }
    automate_if_changed(state, amount_param, 0.625);
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
    if exact {
        return;
    }
    if route < HOST_ROUTE_COUNT {
        let (source_param, target_param, amount_param, ext_param) = ROUTES[route];
        automate_if_changed(state, amount_param, 0.5);
        set_host_route_source(state, route, source, source_param);
        automate_if_changed(state, target_param, 0.0);
        automate_if_changed(state, ext_param, 0.0);
        automate_if_changed(state, amount_param, 0.625);
    } else {
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
    // Internal structural targets are not represented by the stable host route
    // parameters. Prefer the persisted overflow bank so a normal cable drop
    // does not synchronously emit host automation gestures; retain host slots
    // as a compatibility fallback once the overflow bank is full.
    (HOST_ROUTE_COUNT..ROUTE_COUNT)
        .chain(0..HOST_ROUTE_COUNT)
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
    automate_if_changed(
        state,
        source_param,
        f64::from(source.encoded()) / MAX_MODULATION_SOURCES as f64,
    );
    if source == ResolvedRouteSource::ModWheel {
        set_mod_wheel_route(state, route, true);
    }
}

fn automate_if_changed(state: &PluginContext<KurvParams>, param: P, normalized: f64) {
    let current = state
        .params()
        .get_normalized(u32::from(param))
        .unwrap_or_default();
    if (current - normalized).abs() > f64::from(f32::EPSILON) {
        state.automate(param, normalized);
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

pub(super) fn clear_route(state: &PluginContext<KurvParams>, route: usize) {
    set_mod_wheel_route(state, route, false);
    if route < HOST_ROUTE_COUNT {
        let (source, target, amount, ext) = ROUTES[route];
        automate_if_changed(state, amount, 0.5);
        automate_if_changed(state, target, 0.0);
        automate_if_changed(state, ext, 0.0);
        automate_if_changed(state, source, 0.0);
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
