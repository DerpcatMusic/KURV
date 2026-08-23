//! Direct-manipulation modulation routing for the editor.
//!
//! The audio engine still consumes the fixed, host-automatable route bank. This
//! module only gives that bank a source-drag/destination-overlay interface.

mod host_automation;
mod inspector;
mod labels;
mod overlay;
mod route_bank;
mod source_widget;

pub(crate) use host_automation::{
    host_automation_binding, host_automation_destination, host_automation_menu,
    update_host_automation_gesture,
};
pub(crate) use overlay::{cancel_interaction, draw_overlay};
pub(crate) use source_widget::{
    source_color, source_drag_active, source_handle, source_handle_for,
};

use truce_core::editor::{PluginContext, PluginContextReadF32};

use std::sync::Arc;

use crate::editor_theme;
use crate::generators::{MAX_FILTERS, MAX_OSCILLATORS, MAX_OUTPUT_PAIRS};
use crate::modulation_target;
use crate::modulators::routing::{
    FilterControl, GroupControl, ModulationRouteTarget, OscillatorControl, ResolvedRouteSource,
};
use crate::modulators::state::SourceKind;
use crate::{KurvParams, P};
use inspector::{
    finish_amount_drag, inset_clamp, owns_routes_gesture, paint_destination_routes,
    paint_modulation_knob, route_handle_hit_rect, route_handle_id, update_route_amount,
};
use labels::{modular_target_color_index, target_label};
use route_bank::{
    ROUTE_COUNT, RouteAssignmentSnapshot, assign_modular_route, assign_route,
    begin_route_amount_edit, clear_route, display_span, lfo_value_meter, route_amount,
    route_destinations, route_for_modular_assignment, route_source, routes_for_modular_target,
    routes_for_source, routes_for_target, set_route_amount, target_for_param,
};
use source_widget::{
    clear_source_interaction, modulation_handle_hit_radius, modulation_handle_lane_spacing,
    modulation_knob_radius, modulation_source_color, modulation_unit,
};

const UI_STATE_ID: &str = "kurv-direct-modulation";
const SOURCE_GEOMETRY_COUNT: usize = crate::modulators::state::MAX_MODULATION_SOURCES + 3;
const TARGET_COUNT: usize = modulation_target::TARGETS.len();
const MODULAR_TARGET_CAPACITY: usize = MAX_OSCILLATORS * OscillatorControl::INTERNAL_TARGET_COUNT
    + MAX_OUTPUT_PAIRS * GroupControl::INTERNAL_TARGET_COUNT
    + MAX_FILTERS * FilterControl::INTERNAL_TARGET_COUNT;
const _: () = assert!(TARGET_COUNT <= u128::BITS as usize);

/// Creates a tempo-locked unipolar gate source and patches it to a group gain
/// destination. Dynamic rack slots keep the workflow non-destructive to the
/// eight host-automatable LFOs; all mutation happens on the editor thread.
pub(crate) fn create_trance_gate(
    state: &PluginContext<KurvParams>,
    target: ModulationRouteTarget,
) -> Option<usize> {
    let slots = crate::modulators::state::LEGACY_MODULATION_SOURCES
        ..crate::modulators::state::MAX_MODULATION_SOURCES;
    for slot in slots.clone() {
        let config = state.params().modulator_rack.config(slot);
        if config.active
            && config.kind == SourceKind::Lfo
            && config.rate_mode == 2
            && config.mode == 2
            && config.sync_division == 4
            && !config.bipolar
            && config.shape == crate::modulators::lfo::LfoShape::Gate as u8
        {
            let source = ResolvedRouteSource::Rack(slot as u8);
            let Some((route, exact)) = route_for_modular_assignment(state, source, target) else {
                // A fresh source would need the same unavailable route slot.
                return None;
            };
            if !exact {
                assign_modular_route(state, source, target);
            }
            let route = if exact {
                route
            } else {
                let Some((route, true)) = route_for_modular_assignment(state, source, target)
                else {
                    return None;
                };
                route
            };
            set_route_amount(state, route, 0.5);
            return Some(slot);
        }
    }

    let slot = slots
        .clone()
        .find(|&index| !state.params().modulator_rack.config(index).active)?;
    let mut config = state.params().modulator_rack.config(slot);
    config.active = true;
    config.kind = SourceKind::Lfo;
    config.rate_mode = 2;
    config.mode = 2;
    config.sync_division = 4;
    config.phase_offset = 0.0;
    config.bipolar = false;
    config.shape = crate::modulators::lfo::LfoShape::Gate as u8;
    state.params().modulator_rack.set_config(slot, config);

    let source = ResolvedRouteSource::Rack(slot as u8);
    assign_modular_route(state, source, target);
    let Some((route, true)) = route_for_modular_assignment(state, source, target) else {
        config.active = false;
        state.params().modulator_rack.set_config(slot, config);
        return None;
    };
    // Group gain modulation scales by 2.0, so 0.5 maps the unipolar gate to
    // unity while the group's base gain remains at silence.
    set_route_amount(state, route, 0.5);
    Some(slot)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UiDestination {
    Host(u8),
    Modular(ModulationRouteTarget),
}

#[derive(Clone, Copy)]
struct ModularTargetRect {
    target: Option<ModulationRouteTarget>,
    rect: egui::Rect,
}

impl ModularTargetRect {
    const EMPTY: Self = Self {
        target: None,
        rect: egui::Rect::NOTHING,
    };
}

#[derive(Clone)]
struct DropTargetGeometry {
    target_rects: [egui::Rect; TARGET_COUNT],
    host_target_mask: u128,
    modular_target_rects: [ModularTargetRect; MODULAR_TARGET_CAPACITY],
    modular_target_len: usize,
}

impl Default for DropTargetGeometry {
    fn default() -> Self {
        Self {
            target_rects: [egui::Rect::NOTHING; TARGET_COUNT],
            host_target_mask: 0,
            modular_target_rects: [ModularTargetRect::EMPTY; MODULAR_TARGET_CAPACITY],
            modular_target_len: 0,
        }
    }
}

#[derive(Clone)]
struct DirectModulationState {
    dragging_source: Option<ResolvedRouteSource>,
    drag_assignment: Option<RouteAssignmentSnapshot>,
    source_drag_cancelled_until_release: bool,
    hovered_source: Option<ResolvedRouteSource>,
    source_rect: egui::Rect,
    source_rect_frame: u64,
    source_rects: [egui::Rect; SOURCE_GEOMETRY_COUNT],
    source_rect_frames: [u64; SOURCE_GEOMETRY_COUNT],
    hovered_target: Option<UiDestination>,
    hovered_target_valid: bool,
    hovered_rect: egui::Rect,
    inspector_rect: egui::Rect,
    target_geometry: Arc<DropTargetGeometry>,
    route_handle_positions: [egui::Pos2; ROUTE_COUNT],
    route_handle_mask: u64,
    target_rect_frame: u64,
    amount_drag: Option<AmountDrag>,
}

impl Default for DirectModulationState {
    fn default() -> Self {
        Self {
            dragging_source: None,
            drag_assignment: None,
            source_drag_cancelled_until_release: false,
            hovered_source: None,
            source_rect: egui::Rect::NOTHING,
            source_rect_frame: u64::MAX,
            source_rects: [egui::Rect::NOTHING; SOURCE_GEOMETRY_COUNT],
            source_rect_frames: [u64::MAX; SOURCE_GEOMETRY_COUNT],
            hovered_target: None,
            hovered_target_valid: false,
            hovered_rect: egui::Rect::NOTHING,
            inspector_rect: egui::Rect::NOTHING,
            target_geometry: Arc::new(DropTargetGeometry::default()),
            route_handle_positions: [egui::Pos2::ZERO; ROUTE_COUNT],
            route_handle_mask: 0,
            target_rect_frame: u64::MAX,
            amount_drag: None,
        }
    }
}

#[derive(Clone, Copy)]
struct AmountDrag {
    route: usize,
    amount: f32,
    initial_amount: f32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrackAxis {
    Horizontal,
    Vertical,
}

/// Registers a supported destination, edits route depth from its side handle, and
/// paints each route as a thin source-colored range around the base value.
/// Returns true while the gesture owns the control so its base value is not
/// changed at the same time.
pub(crate) fn destination(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    param: P,
    response: &egui::Response,
    base: f32,
    track: egui::Rect,
    axis: TrackAxis,
) -> bool {
    let Some(target) = target_for_param(param) else {
        return false;
    };
    let id = egui::Id::new(UI_STATE_ID);
    let frame = ui.ctx().cumulative_frame_nr();
    let visible_rect = response.interact_rect.intersect(ui.clip_rect());
    if !visible_rect.is_positive() {
        return false;
    }
    ui.data_mut(|data| {
        let direct = data.get_temp_mut_or_default::<DirectModulationState>(id);
        prepare_target_frame(direct, frame);
        let index = usize::from(target - 1);
        let geometry = Arc::make_mut(&mut direct.target_geometry);
        geometry.target_rects[index] = visible_rect;
        geometry.host_target_mask |= 1_u128 << index;
    });
    // A source drag only needs destination geometry. Rebuilding and painting
    // every destination's existing routes here multiplies route snapshots and
    // live-meter reads across the full visible rack while the overlay is
    // already replacing that feedback with drop highlights.
    if source_drag_active(ui) {
        return true;
    }
    let routes = routes_for_target(ui, state, target);
    let span = display_span(target);
    let live_base = routes
        .as_slice()
        .iter()
        .fold(base, |value, (_, source, amount, _)| {
            value + lfo_value_meter(state, *source) * amount * span
        })
        .clamp(0.0, 1.0);
    paint_destination_routes(
        ui,
        response,
        track,
        axis,
        live_base,
        span,
        &routes,
        usize::from(target),
    )
}

/// Registers one structural module/group destination in the direct-routing
/// overlay. Only visible targets are retained, keeping editor work bounded even
/// when a patch owns the full 32-oscillator bank.
pub(crate) fn modular_destination(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    target: ModulationRouteTarget,
    response: &egui::Response,
    base: f32,
    track: egui::Rect,
    axis: TrackAxis,
    span: f32,
) -> bool {
    let source_dragging = source_drag_active(ui);
    if !target.supports_internal_modulation() {
        if !source_dragging {
            host_automation_destination(ui, state, target, response, base);
        }
        return false;
    }
    let visible_rect = response.interact_rect.intersect(ui.clip_rect());
    if !visible_rect.is_positive() {
        return false;
    }
    let id = egui::Id::new(UI_STATE_ID);
    let frame = ui.ctx().cumulative_frame_nr();
    if !source_dragging {
        host_automation_destination(ui, state, target, response, base);
    }
    ui.data_mut(|data| {
        let direct = data.get_temp_mut_or_default::<DirectModulationState>(id);
        prepare_target_frame(direct, frame);
        // Destination controls are unique by construction within one editor
        // frame. Appending directly keeps registration linear as racks grow.
        let geometry = Arc::make_mut(&mut direct.target_geometry);
        if geometry.modular_target_len < MODULAR_TARGET_CAPACITY {
            geometry.modular_target_rects[geometry.modular_target_len] = ModularTargetRect {
                target: Some(target),
                rect: visible_rect,
            };
            geometry.modular_target_len += 1;
        }
    });
    if source_dragging {
        return true;
    }
    let routes = routes_for_modular_target(ui, state, target);
    let live_base = routes
        .as_slice()
        .iter()
        .fold(base, |value, (_, source, amount, _)| {
            value + lfo_value_meter(state, *source) * amount * span
        });
    paint_destination_routes(
        ui,
        response,
        track,
        axis,
        live_base.clamp(0.0, 1.0),
        span,
        &routes,
        modular_target_color_index(target),
    )
}

/// Adds only the fixed-bank host automation affordance to a structural
/// control. Use this for controls that are safe to update at process-block
/// boundaries but are not yet valid audio-rate internal modulation targets.
fn prepare_target_frame(direct: &mut DirectModulationState, frame: u64) {
    if direct.target_rect_frame == frame {
        return;
    }
    let geometry = Arc::make_mut(&mut direct.target_geometry);
    let mut host_targets = geometry.host_target_mask;
    while host_targets != 0 {
        let target = host_targets.trailing_zeros() as usize;
        host_targets &= host_targets - 1;
        geometry.target_rects[target] = egui::Rect::NOTHING;
    }
    geometry.host_target_mask = 0;
    // Compact structural targets are overwritten from index zero as visible
    // controls register. Resetting the length is sufficient; clearing the full
    // maximum-size rack array made every editor frame write hundreds of dead
    // entries even when the patch contained one oscillator.
    geometry.modular_target_len = 0;
    // Handle positions are likewise guarded by the mask and overwritten before
    // each active bit is set.
    direct.route_handle_mask = 0;
    direct.target_rect_frame = frame;
}

pub(crate) fn owns_gesture(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    param: P,
    response: &egui::Response,
) -> bool {
    if source_drag_active(ui) {
        return true;
    }
    let Some(target) = target_for_param(param) else {
        return false;
    };
    let routes = routes_for_target(ui, state, target);
    owns_routes_gesture(ui, state, response, &routes)
}

pub(crate) fn modular_owns_gesture(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    target: ModulationRouteTarget,
    response: &egui::Response,
) -> bool {
    if source_drag_active(ui) {
        return true;
    }
    let routes = routes_for_modular_target(ui, state, target);
    owns_routes_gesture(ui, state, response, &routes)
}

pub(crate) fn used_source_mask(state: &PluginContext<KurvParams>) -> u64 {
    route_bank::used_source_mask(state)
}

pub(crate) fn clear_source(state: &PluginContext<KurvParams>, source: u8) {
    route_bank::clear_source(state, source);
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use truce::params::Params;
    use truce_core::editor::{ClosureBridge, PluginContext};

    use super::*;

    fn test_context() -> PluginContext<KurvParams> {
        let params = Arc::new(KurvParams::default());
        let params_for_set = Arc::clone(&params);
        let params_for_get = Arc::clone(&params);
        let params_for_plain = Arc::clone(&params);
        let params_for_format = Arc::clone(&params);
        PluginContext::new(
            Arc::new(ClosureBridge {
                begin_edit: Box::new(|_| {}),
                set_param: Box::new(move |id, normalized| {
                    params_for_set.set_normalized(id, normalized);
                }),
                end_edit: Box::new(|_| {}),
                request_resize: Box::new(|_, _| false),
                get_param: Box::new(move |id| {
                    params_for_get.get_normalized(id).unwrap_or_default()
                }),
                get_param_plain: Box::new(move |id| {
                    params_for_plain.get_plain(id).unwrap_or_default()
                }),
                format_param: Box::new(move |id| {
                    let plain = params_for_format.get_plain(id).unwrap_or_default();
                    params_for_format
                        .format_value(id, plain)
                        .unwrap_or_default()
                }),
                get_meter: Box::new(|_| 0.0),
                get_state: Box::new(Vec::new),
                set_state: Box::new(|_| {}),
                transport: Box::new(|| None),
            }),
            params,
        )
    }

    #[test]
    fn trance_gate_reuses_one_source_for_additional_targets() {
        let state = test_context();
        let group = state.params().generator_stack.snapshot().groups()[0].id();
        let gain = ModulationRouteTarget::group(group, GroupControl::Gain);
        let dry = ModulationRouteTarget::group(group, GroupControl::Dry);

        let first = create_trance_gate(&state, gain).expect("first trance gate");
        let second = create_trance_gate(&state, dry).expect("reused trance gate");

        assert_eq!(second, first);
        assert_eq!(
            (crate::modulators::state::LEGACY_MODULATION_SOURCES
                ..crate::modulators::state::MAX_MODULATION_SOURCES)
                .filter(|&slot| state.params().modulator_rack.config(slot).active)
                .count(),
            1
        );
        let source = ResolvedRouteSource::Rack(first as u8);
        assert!(matches!(
            route_for_modular_assignment(&state, source, gain),
            Some((_, true))
        ));
        assert!(matches!(
            route_for_modular_assignment(&state, source, dry),
            Some((_, true))
        ));
    }
}
