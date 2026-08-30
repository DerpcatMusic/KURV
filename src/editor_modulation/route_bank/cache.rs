//! One-frame route indexing for editor paint and drag hit-testing.

use truce_core::editor::{PluginContext, PluginContextReadF32};

use super::super::UiDestination;
use super::{HOST_ROUTE_COUNT, ROUTE_COUNT, ROUTES, UiRoute, host_route_source, route_target};
use crate::modulation_target;
use crate::modulators::routing::{
    ExtraModulationRouteSnapshot, ModulationRouteTarget, ModulationRouteTargetSnapshot,
    ResolvedRouteSource,
};
use crate::modulators::state::SourceKind;
use crate::{KurvParams, P};

const FRAME_ROUTE_CACHE_ID: &str = "kurv-direct-modulation-frame-routes";
const TARGET_COUNT: usize = modulation_target::TARGETS.len();

#[derive(Clone, Default)]
pub(in crate::editor_modulation) struct RouteBucket {
    entries: Vec<UiRoute>,
}

impl RouteBucket {
    pub(in crate::editor_modulation) fn as_slice(&self) -> &[UiRoute] {
        &self.entries
    }

    pub(in crate::editor_modulation) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(in crate::editor_modulation) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn push(&mut self, route: UiRoute) {
        self.entries.push(route);
    }
}

#[derive(Clone, Copy)]
pub(super) struct RouteScan {
    pub(super) targets: ModulationRouteTargetSnapshot,
    overflow: ExtraModulationRouteSnapshot,
    mod_wheel_mask: u64,
    xy_x_mask: u64,
    xy_y_mask: u64,
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
#[derive(Clone, Copy)]
pub(in crate::editor_modulation) struct RouteAssignmentSnapshot {
    host_free: bool,
    modular_free: bool,
    host_exact: [bool; TARGET_COUNT],
    modular_exact: [Option<ModulationRouteTarget>; ROUTE_COUNT],
    modular_exact_len: usize,
    generator_target_mask: Option<u32>,
    destinations: [Option<UiDestination>; ROUTE_COUNT],
}

impl RouteAssignmentSnapshot {
    pub(in crate::editor_modulation) fn capture(
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
        let generator_target_mask = if let ResolvedRouteSource::Generator(source) = source {
            let patch = state.generator_stack.snapshot();
            Some(patch.groups().iter().fold(0_u32, |mask, group| {
                let mut after_source = false;
                group.modules().iter().fold(mask, |mask, module| {
                    let Some(slot) = module.oscillator_slot() else {
                        return mask;
                    };
                    if slot.index() as u8 == source {
                        after_source = true;
                        mask
                    } else if after_source {
                        mask | (1 << slot.index())
                    } else {
                        mask
                    }
                })
            }))
        } else {
            None
        };
        for route in 0..ROUTE_COUNT {
            let destination = destinations[route];
            modular_free |= destination.is_none()
                && (generator_target_mask.is_none() || route >= HOST_ROUTE_COUNT);
            host_free |= destination.is_none() && generator_target_mask.is_none();
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
            generator_target_mask,
            destinations,
        }
    }

    pub(in crate::editor_modulation) fn accepts_host(&self, target: u8) -> bool {
        self.host_free
            || target
                .checked_sub(1)
                .and_then(|target| self.host_exact.get(usize::from(target)))
                .copied()
                .unwrap_or(false)
    }

    pub(in crate::editor_modulation) fn accepts_modular(
        &self,
        target: ModulationRouteTarget,
    ) -> bool {
        if let Some(mask) = self.generator_target_mask {
            let ModulationRouteTarget::Oscillator { slot, .. } = target else {
                return false;
            };
            if mask & (1 << slot.index()) == 0 {
                return false;
            }
        }
        self.modular_free
            || self.modular_exact[..self.modular_exact_len]
                .binary_search(&Some(target))
                .is_ok()
    }

    pub(in crate::editor_modulation) fn bank_full(&self) -> bool {
        !self.modular_free
    }

    pub(in crate::editor_modulation) fn destinations(
        &self,
    ) -> &[Option<UiDestination>; ROUTE_COUNT] {
        &self.destinations
    }
}

impl RouteScan {
    pub(super) fn capture(state: &PluginContext<KurvParams>) -> Self {
        Self {
            targets: state.params().modulation_route_targets.snapshot(),
            overflow: state.params().modulation_route_overflow.snapshot(),
            mod_wheel_mask: state.params().mod_wheel_route_mask.load(),
            xy_x_mask: state.params().xy_source_x_route_mask.load(),
            xy_y_mask: state.params().xy_source_y_route_mask.load(),
        }
    }

    pub(super) fn source(
        &self,
        state: &PluginContext<KurvParams>,
        route: usize,
    ) -> Option<ResolvedRouteSource> {
        let encoded = if route < HOST_ROUTE_COUNT {
            host_route_source(state, ROUTES[route].source)
        } else {
            self.overflow[route - HOST_ROUTE_COUNT].source
        };
        ResolvedRouteSource::decode(
            encoded,
            self.mod_wheel_mask,
            self.xy_x_mask,
            self.xy_y_mask,
            route,
        )
    }

    pub(super) fn amount(&self, state: &PluginContext<KurvParams>, route: usize) -> f32 {
        if route < HOST_ROUTE_COUNT {
            state.get_param(ROUTES[route].amount).mul_add(2.0, -1.0)
        } else {
            self.overflow[route - HOST_ROUTE_COUNT].amount
        }
    }

    pub(super) fn destination(
        &self,
        state: &PluginContext<KurvParams>,
        route: usize,
    ) -> Option<UiDestination> {
        if let Some(target) = self.targets.get(route).copied().flatten() {
            return Some(match target {
                ModulationRouteTarget::Legacy { target } => UiDestination::Host(target),
                target => UiDestination::Modular(target),
            });
        }
        let target = route_target(state, route);
        (target != 0).then_some(UiDestination::Host(target))
    }
}

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
        ui.data_mut(|data| *data.get_temp_mut_or_default::<FrameRouteCache>(id) = cache);
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

pub(in crate::editor_modulation) fn routes_for_target(
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

pub(in crate::editor_modulation) fn routes_for_modular_target(
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

pub(in crate::editor_modulation) fn routes_for_source(
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

pub(in crate::editor_modulation) fn route_destinations(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
) -> [Option<UiDestination>; ROUTE_COUNT] {
    ensure_frame_route_cache(ui, state);
    ui.data_mut(|data| {
        data.get_temp_mut_or_default::<FrameRouteCache>(egui::Id::new(FRAME_ROUTE_CACHE_ID))
            .destinations
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
