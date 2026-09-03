//! Direct-manipulation modulation routing for the editor.
//!
//! The audio engine still consumes the fixed, host-automatable route bank. This
//! module only gives that bank a click-to-assign destination overlay.

mod host_automation;
mod inspector;
mod labels;
mod overlay;
mod route_bank;
mod source_widget;

pub(crate) use host_automation::{
    host_automation_binding, host_automation_destination, host_automation_menu,
    paint_host_automation_badge, update_host_automation_gesture,
};
pub(crate) use overlay::{cancel_interaction, draw_overlay};
pub(crate) use source_widget::{
    generator_source_drag_active, source_assignment_active, source_color, source_drag_active,
    source_handle_for,
};

use truce::params::Params;
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
    finish_amount_drag, inset_clamp, modulation_drop_center, modulation_route_row_step,
    owns_routes_gesture, paint_destination_routes, paint_parent_route_marker,
    paint_route_depth_modulation, paint_route_marker, route_handle_hit_rect, route_handle_id,
    update_created_route_drag, update_route_amount,
};
use labels::{target_label, target_parent_color};
use route_bank::{
    ROUTE_COUNT, RouteAssignmentSnapshot, assign_modular_route, assign_route,
    begin_route_amount_edit, clear_resolved_source, clear_route, display_span, route_amount,
    route_destinations, route_for_assignment, route_for_modular_assignment, route_source,
    routes_for_modular_target, routes_for_source, routes_for_target, set_route_amount,
    target_for_param,
};

pub(crate) fn clear_generator_source(
    state: &PluginContext<KurvParams>,
    slot: crate::generators::OscillatorSlot,
) {
    clear_resolved_source(state, ResolvedRouteSource::Generator(slot.index() as u8));
}

pub(crate) fn clear_module_routes(state: &PluginContext<KurvParams>, module_id: u64) {
    let targets = state.params().modulation_route_targets.snapshot();
    for (route, target) in targets.iter().copied().enumerate() {
        if matches!(
            target,
            Some(
                ModulationRouteTarget::Oscillator { module_id: id, .. }
                    | ModulationRouteTarget::Filter { module_id: id, .. }
                    | ModulationRouteTarget::Aux { module_id: id, .. }
            ) if id == module_id
        ) {
            clear_route(state, route);
        }
    }
}

pub(crate) fn clear_group_routes(state: &PluginContext<KurvParams>, group_id: u64) {
    let targets = state.params().modulation_route_targets.snapshot();
    for (route, target) in targets.iter().copied().enumerate() {
        if matches!(
            target,
            Some(ModulationRouteTarget::Group { group_id: id, .. }) if id == group_id
        ) {
            clear_route(state, route);
        }
    }
}

pub(crate) fn generator_preview_routes(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    module_id: crate::generators::ModuleId,
    slot: crate::generators::OscillatorSlot,
) -> Vec<(u8, OscillatorControl, f32)> {
    let mut preview = Vec::new();
    for control in OscillatorControl::ALL.iter().copied() {
        let target = ModulationRouteTarget::oscillator(module_id, slot, control);
        for (_, source, amount, _) in routes_for_modular_target(ui, state, target).as_slice() {
            if let ResolvedRouteSource::Generator(source) = source {
                preview.push((*source, control, *amount));
            }
        }
    }
    preview
}
use source_widget::{
    armed_source, assignment_source, clear_source_interaction, modulation_handle_hit_radius,
    modulation_handle_lane_spacing, modulation_route_marker_radius, modulation_source_color,
    modulation_unit, paint_modulation_plus,
};

const UI_STATE_ID: &str = "kurv-direct-modulation";
const MODULATED_RESPONSE_ID: &str = "kurv-modulated-readouts";
const SOURCE_GEOMETRY_COUNT: usize =
    crate::modulators::state::MAX_MODULATION_SOURCES + MAX_OSCILLATORS + 3;
const TARGET_COUNT: usize = modulation_target::TARGETS.len();
const MODULAR_TARGET_CAPACITY: usize = MAX_OSCILLATORS * OscillatorControl::INTERNAL_TARGET_COUNT
    + MAX_OUTPUT_PAIRS * GroupControl::INTERNAL_TARGET_COUNT
    + MAX_FILTERS * FilterControl::INTERNAL_TARGET_COUNT
    + ROUTE_COUNT;
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
    center: egui::Pos2,
}

impl ModularTargetRect {
    const EMPTY: Self = Self {
        target: None,
        rect: egui::Rect::NOTHING,
        center: egui::Pos2::ZERO,
    };
}

#[derive(Clone)]
struct DropTargetGeometry {
    target_rects: [egui::Rect; TARGET_COUNT],
    target_centers: [egui::Pos2; TARGET_COUNT],
    host_target_mask: u128,
    modular_target_rects: [ModularTargetRect; MODULAR_TARGET_CAPACITY],
    modular_target_len: usize,
}

impl Default for DropTargetGeometry {
    fn default() -> Self {
        Self {
            target_rects: [egui::Rect::NOTHING; TARGET_COUNT],
            target_centers: [egui::Pos2::ZERO; TARGET_COUNT],
            host_target_mask: 0,
            modular_target_rects: [ModularTargetRect::EMPTY; MODULAR_TARGET_CAPACITY],
            modular_target_len: 0,
        }
    }
}

#[derive(Clone)]
struct DirectModulationState {
    dragging_source: Option<ResolvedRouteSource>,
    armed_source: Option<ResolvedRouteSource>,
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
    parent_route_focus_mask: u64,
    target_rect_frame: u64,
    amount_drag: Option<AmountDrag>,
}

impl Default for DirectModulationState {
    fn default() -> Self {
        Self {
            dragging_source: None,
            armed_source: None,
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
            parent_route_focus_mask: 0,
            target_rect_frame: u64::MAX,
            amount_drag: None,
        }
    }
}

#[derive(Clone, Copy)]
struct AmountDrag {
    route: usize,
    amount: f32,
    raw_amount: f32,
    initial_amount: f32,
    last_pointer: Option<egui::Pos2>,
    coarse: bool,
    created: bool,
}

fn modulation_pointer_position(ui: &egui::Ui, response: &egui::Response) -> Option<egui::Pos2> {
    response
        .interact_pointer_pos()
        .or_else(|| ui.input(|input| input.pointer.interact_pos()))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrackAxis {
    Horizontal,
    Vertical,
    Radial,
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
    let drop_center = modulation_drop_center(ui, response.rect);
    ui.data_mut(|data| {
        let direct = data.get_temp_mut_or_default::<DirectModulationState>(id);
        prepare_target_frame(direct, frame);
        let index = usize::from(target - 1);
        let geometry = Arc::make_mut(&mut direct.target_geometry);
        geometry.target_rects[index] = visible_rect;
        geometry.target_centers[index] = drop_center;
        geometry.host_target_mask |= 1_u128 << index;
    });
    let assigning = assignment_source(ui).is_some();
    let routes = routes_for_target(ui, state, target);
    set_response_modulated(ui, response.id, !assigning && !routes.is_empty());
    let span = display_span(target);
    paint_destination_routes(
        ui,
        state,
        response,
        UiDestination::Host(target),
        track,
        axis,
        drop_center,
        base,
        span,
        &routes,
    ) || assigning
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
    let source_dragging = assignment_source(ui).is_some();
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
    if !source_dragging && !matches!(target, ModulationRouteTarget::RouteDepth { .. }) {
        host_automation_destination(ui, state, target, response, base);
    }
    let center_rect = if matches!(target, ModulationRouteTarget::RouteDepth { .. }) {
        track
    } else {
        response.rect
    };
    let drop_center = modulation_drop_center(ui, center_rect);
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
                center: drop_center,
            };
            geometry.modular_target_len += 1;
        }
    });
    let routes = routes_for_modular_target(ui, state, target);
    set_response_modulated(ui, response.id, !source_dragging && !routes.is_empty());
    if matches!(target, ModulationRouteTarget::RouteDepth { .. }) {
        source_dragging
    } else {
        paint_destination_routes(
            ui,
            state,
            response,
            UiDestination::Modular(target),
            track,
            axis,
            drop_center,
            base,
            span,
            &routes,
        ) || source_dragging
    }
}

pub(crate) fn response_is_modulated(ui: &egui::Ui, response: &egui::Response) -> bool {
    ui.data(|data| {
        data.get_temp::<std::collections::HashSet<egui::Id>>(egui::Id::new(MODULATED_RESPONSE_ID))
            .is_some_and(|ids| ids.contains(&response.id))
    })
}

fn set_response_modulated(ui: &egui::Ui, id: egui::Id, modulated: bool) {
    ui.data_mut(|data| {
        let ids = data.get_temp_mut_or_default::<std::collections::HashSet<egui::Id>>(
            egui::Id::new(MODULATED_RESPONSE_ID),
        );
        if modulated {
            ids.insert(id);
        } else {
            ids.remove(&id);
        }
    });
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
        geometry.target_centers[target] = egui::Pos2::ZERO;
    }
    geometry.host_target_mask = 0;
    if direct.armed_source.is_some() {
        direct.drag_assignment = None;
    }
    // Compact structural targets are overwritten from index zero as visible
    // controls register. Resetting the length is sufficient; clearing the full
    // maximum-size rack array made every editor frame write hundreds of dead
    // entries even when the patch contained one oscillator.
    geometry.modular_target_len = 0;
    // Handle positions are likewise guarded by the mask and overwritten before
    // each active bit is set.
    if direct.dragging_source.is_none() && direct.armed_source.is_none() {
        direct.route_handle_mask = 0;
    }
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
    if let Some(source) = armed_source(ui) {
        return assignment_amount_gesture(ui, state, response, source, UiDestination::Host(target));
    }
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
    if let Some(source) = armed_source(ui) {
        return assignment_amount_gesture(
            ui,
            state,
            response,
            source,
            UiDestination::Modular(target),
        );
    }
    let routes = routes_for_modular_target(ui, state, target);
    owns_routes_gesture(ui, state, response, &routes)
}

fn assignment_amount_gesture(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    response: &egui::Response,
    source: ResolvedRouteSource,
    target: UiDestination,
) -> bool {
    assignment_amount_gesture_with_start(ui, state, response, source, target, false)
}

fn assignment_amount_gesture_with_start(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    response: &egui::Response,
    source: ResolvedRouteSource,
    target: UiDestination,
    forced_start: bool,
) -> bool {
    let id = egui::Id::new(UI_STATE_ID);
    let availability = ui
        .data(|data| {
            data.get_temp::<DirectModulationState>(id)
                .and_then(|direct| direct.drag_assignment)
        })
        .unwrap_or_else(|| {
            let availability = RouteAssignmentSnapshot::capture(ui, state, source);
            ui.data_mut(|data| {
                data.get_temp_mut_or_default::<DirectModulationState>(id)
                    .drag_assignment = Some(availability);
            });
            availability
        });
    let valid = match target {
        UiDestination::Host(target) => availability.accepts_host(target),
        UiDestination::Modular(target) => availability.accepts_modular(target),
    };
    if (response.drag_started() || forced_start) && valid {
        finish_amount_drag(ui, state, id, false);
        let Some((route, exact)) = (match target {
            UiDestination::Host(target) => route_for_assignment(state, source, target),
            UiDestination::Modular(target) => route_for_modular_assignment(state, source, target),
        }) else {
            return true;
        };
        if !exact {
            match target {
                UiDestination::Host(target) => assign_route(state, source, target),
                UiDestination::Modular(target) => assign_modular_route(state, source, target),
            }
            set_route_amount(state, route, 0.0);
        }
        let amount = route_amount(state, route);
        begin_route_amount_edit(state, route);
        let coarse = ui.input(|input| input.modifiers.ctrl);
        let last_pointer = modulation_pointer_position(ui, response);
        ui.data_mut(|data| {
            data.get_temp_mut_or_default::<DirectModulationState>(id)
                .amount_drag = Some(AmountDrag {
                route,
                amount,
                raw_amount: amount,
                initial_amount: amount,
                last_pointer,
                coarse,
                created: !exact,
            });
        });
    }
    let dragging = ui.data(|data| {
        data.get_temp::<DirectModulationState>(id)
            .is_some_and(|direct| direct.amount_drag.is_some())
    });
    if dragging && (response.dragged() || response.drag_stopped()) {
        let route = ui.data(|data| {
            data.get_temp::<DirectModulationState>(id)
                .and_then(|direct| direct.amount_drag.map(|drag| drag.route))
        });
        if let Some(route) = route {
            update_route_amount(ui, state, response, id, route);
            editor_theme::request_display_repaint(ui);
        }
    }
    if dragging && response.drag_stopped() {
        finish_amount_drag(ui, state, id, false);
    }
    true
}

pub(crate) fn used_source_mask(state: &PluginContext<KurvParams>) -> u64 {
    route_bank::used_source_mask(state)
}

pub(crate) fn clear_source(state: &PluginContext<KurvParams>, source: u8) {
    route_bank::clear_source(state, source);
}

pub(crate) fn source_route_display(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    source: ResolvedRouteSource,
    value: f32,
) -> Option<(String, String)> {
    let routes = routes_for_source(ui, state, source);
    let [(route, _, amount, _)] = routes.as_slice() else {
        return None;
    };
    let target = route_destinations(ui, state)[*route]?;
    let label = target_label(target);
    let formatted = match target {
        UiDestination::Host(target) => modulation_target::descriptor(target).and_then(|target| {
            state.params().format_value(
                u32::from(target.param),
                f64::from(value * amount.clamp(-1.0, 1.0) * target.scale),
            )
        }),
        UiDestination::Modular(ModulationRouteTarget::Legacy { target }) => {
            modulation_target::descriptor(target).and_then(|target| {
                state.params().format_value(
                    u32::from(target.param),
                    f64::from(value * amount.clamp(-1.0, 1.0) * target.scale),
                )
            })
        }
        UiDestination::Modular(target) => {
            format_modular_route_delta(target, value * amount.clamp(-1.0, 1.0))
        }
    }
    .unwrap_or_else(|| format!("{:+.0}%", value * amount * 100.0));
    Some((label, formatted))
}

fn format_modular_route_delta(target: ModulationRouteTarget, value: f32) -> Option<String> {
    let percent = || format!("{:+.0}%", value * 100.0);
    Some(match target {
        ModulationRouteTarget::Oscillator { control, .. } => match control {
            OscillatorControl::Transpose => format!("{:+.1} st", value * 48.0),
            // The DSP stores pitch modulation in semitones; one full CENT route
            // therefore spans one semitone, or one hundred cents.
            OscillatorControl::Cents => format!("{:+.0} ct", value * 100.0),
            OscillatorControl::PhasePosition => format!("{:+.0} deg", value * 360.0),
            OscillatorControl::PulseWidth => format!("{:+.0}%", value * 47.0),
            OscillatorControl::Shape => format!("{:+.0}%", value * 300.0),
            OscillatorControl::Level
            | OscillatorControl::Pan
            | OscillatorControl::PhaseWarpAmount
            | OscillatorControl::PhaseModAmount
            | OscillatorControl::RingModAmount
            | OscillatorControl::UnisonJitter
            | OscillatorControl::UnisonRate
            | OscillatorControl::UnisonStereoPosition
            | OscillatorControl::UnisonStereoAlternate
            | OscillatorControl::GrainTune
            | OscillatorControl::GrainStereo
            | OscillatorControl::RichDynamic => percent(),
            _ => return None,
        },
        ModulationRouteTarget::Group { control, .. } => match control {
            GroupControl::Gain
            | GroupControl::AttackCurve
            | GroupControl::DecayCurve
            | GroupControl::ReleaseCurve => format!("{:+.0}%", value * 200.0),
            GroupControl::Pan
            | GroupControl::Dry
            | GroupControl::Send
            | GroupControl::Sidechain => percent(),
            GroupControl::Attack
            | GroupControl::Decay
            | GroupControl::Sustain
            | GroupControl::Release => return None,
        },
        ModulationRouteTarget::Filter { control, .. } => match control {
            FilterControl::Cutoff | FilterControl::Resonance => {
                format!("{:+.2} oct", value * 4.0)
            }
            FilterControl::Slope | FilterControl::Morph | FilterControl::Shape => percent(),
        },
        ModulationRouteTarget::Aux { .. } => format!("{:+.0}%", value * 200.0),
        ModulationRouteTarget::RouteDepth { .. } => percent(),
        ModulationRouteTarget::MacroPack { .. } => percent(),
        ModulationRouteTarget::Legacy { .. } => return None,
    })
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
