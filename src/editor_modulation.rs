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
    route_destinations, route_source, routes_for_modular_target, routes_for_source,
    routes_for_target, target_for_param,
};
use source_widget::{
    clear_source_interaction, modulation_handle_hit_radius, modulation_handle_lane_spacing,
    modulation_knob_radius, modulation_source_color, modulation_unit,
};

const UI_STATE_ID: &str = "kurv-direct-modulation";
const TARGET_COUNT: usize = modulation_target::TARGETS.len();
const MODULAR_TARGET_CAPACITY: usize = MAX_OSCILLATORS * OscillatorControl::INTERNAL_TARGET_COUNT
    + MAX_OUTPUT_PAIRS * GroupControl::INTERNAL_TARGET_COUNT
    + MAX_FILTERS * FilterControl::INTERNAL_TARGET_COUNT;

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
struct DirectModulationState {
    dragging_source: Option<ResolvedRouteSource>,
    source_drag_cancelled_until_release: bool,
    hovered_source: Option<ResolvedRouteSource>,
    source_rect: egui::Rect,
    source_rect_frame: u64,
    hovered_target: Option<UiDestination>,
    hovered_target_valid: bool,
    hovered_rect: egui::Rect,
    inspector_rect: egui::Rect,
    target_rects: [egui::Rect; TARGET_COUNT],
    modular_target_rects: [ModularTargetRect; MODULAR_TARGET_CAPACITY],
    modular_target_len: usize,
    route_handle_positions: [egui::Pos2; ROUTE_COUNT],
    route_handle_mask: u64,
    target_rect_frame: u64,
    amount_drag: Option<AmountDrag>,
}

impl Default for DirectModulationState {
    fn default() -> Self {
        Self {
            dragging_source: None,
            source_drag_cancelled_until_release: false,
            hovered_source: None,
            source_rect: egui::Rect::NOTHING,
            source_rect_frame: u64::MAX,
            hovered_target: None,
            hovered_target_valid: false,
            hovered_rect: egui::Rect::NOTHING,
            inspector_rect: egui::Rect::NOTHING,
            target_rects: [egui::Rect::NOTHING; TARGET_COUNT],
            modular_target_rects: [ModularTargetRect::EMPTY; MODULAR_TARGET_CAPACITY],
            modular_target_len: 0,
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
        direct.target_rects[usize::from(target - 1)] = visible_rect;
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
    if !target.supports_internal_modulation() {
        host_automation_destination(ui, state, target, response, base);
        return false;
    }
    let visible_rect = response.interact_rect.intersect(ui.clip_rect());
    if !visible_rect.is_positive() {
        return false;
    }
    let id = egui::Id::new(UI_STATE_ID);
    let frame = ui.ctx().cumulative_frame_nr();
    host_automation_destination(ui, state, target, response, base);
    ui.data_mut(|data| {
        let direct = data.get_temp_mut_or_default::<DirectModulationState>(id);
        prepare_target_frame(direct, frame);
        // Destination controls are unique by construction within one editor
        // frame. Appending directly keeps registration linear as racks grow.
        if direct.modular_target_len < MODULAR_TARGET_CAPACITY {
            direct.modular_target_rects[direct.modular_target_len] = ModularTargetRect {
                target: Some(target),
                rect: visible_rect,
            };
            direct.modular_target_len += 1;
        }
    });
    if source_drag_active(ui) {
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
    direct.target_rects = [egui::Rect::NOTHING; TARGET_COUNT];
    direct.modular_target_rects = [ModularTargetRect::EMPTY; MODULAR_TARGET_CAPACITY];
    direct.modular_target_len = 0;
    direct.route_handle_positions = [egui::Pos2::ZERO; ROUTE_COUNT];
    direct.route_handle_mask = 0;
    direct.target_rect_frame = frame;
}

pub(crate) fn owns_gesture(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    param: P,
    response: &egui::Response,
) -> bool {
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
    let routes = routes_for_modular_target(ui, state, target);
    owns_routes_gesture(ui, state, response, &routes)
}

pub(crate) fn used_source_mask(state: &PluginContext<KurvParams>) -> u64 {
    route_bank::used_source_mask(state)
}

pub(crate) fn clear_source(state: &PluginContext<KurvParams>, source: u8) {
    route_bank::clear_source(state, source);
}
