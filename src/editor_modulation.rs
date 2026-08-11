//! Direct-manipulation modulation routing for the editor.
//!
//! The audio engine still consumes the fixed, host-automatable route bank. This
//! module only gives that bank a source-drag/destination-overlay interface.

mod host_automation;
mod inspector;
mod overlay;
mod route_bank;

pub(crate) use host_automation::{
    host_automation_binding, host_automation_destination, host_automation_menu,
    update_host_automation_gesture,
};
pub(crate) use overlay::{cancel_interaction, draw_overlay};

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
use route_bank::{
    ROUTE_COUNT, RouteAssignmentSnapshot, assign_modular_route, assign_route,
    begin_route_amount_edit, clear_route, display_span, lfo_value_meter, route_amount,
    route_destinations, route_source, routes_for_modular_target, routes_for_source,
    routes_for_target, target_for_param,
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

pub(crate) fn source_color(index: usize) -> egui::Color32 {
    editor_theme::modulation_source_accent(index)
}

fn modulation_source_color(source: ResolvedRouteSource) -> egui::Color32 {
    match source {
        ResolvedRouteSource::Rack(index) => source_color(usize::from(index)),
        ResolvedRouteSource::ModWheel => editor_theme::semantic().primary,
    }
}

fn modulation_unit(ui: &egui::Ui) -> f32 {
    editor_theme::title_height(ui)
}

fn modulation_knob_radius(unit: f32) -> f32 {
    unit * 0.29
}

fn modulation_handle_hit_radius(unit: f32) -> f32 {
    unit * 0.38
}

fn modulation_handle_lane_spacing(unit: f32) -> f32 {
    unit * 0.5
}

pub(crate) fn source_handle(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    index: usize,
    label: &str,
    response: &egui::Response,
) -> egui::Response {
    source_handle_impl(
        ui,
        state,
        ResolvedRouteSource::Rack(index as u8),
        label,
        response,
        true,
    )
}

pub(crate) fn source_handle_for(
    ui: &egui::Ui,
    _state: &PluginContext<KurvParams>,
    source: ResolvedRouteSource,
    label: &str,
    response: &egui::Response,
) -> egui::Response {
    source_handle_impl(ui, _state, source, label, response, false)
}

fn source_handle_impl(
    ui: &egui::Ui,
    _state: &PluginContext<KurvParams>,
    source: ResolvedRouteSource,
    label: &str,
    response: &egui::Response,
    paint_label: bool,
) -> egui::Response {
    let color = modulation_source_color(source);
    let id = egui::Id::new(UI_STATE_ID);
    let frame = ui.ctx().cumulative_frame_nr();
    // Arm the dedicated source affordance on press instead of waiting for
    // egui's drag threshold. That keeps scroll areas and quick pointer moves
    // from swallowing the first frame of the gesture.
    if response.is_pointer_button_down_on() || response.drag_started() || response.dragged() {
        ui.data_mut(|data| {
            let direct = data.get_temp_mut_or_default::<DirectModulationState>(id);
            if direct.dragging_source.is_none() && !direct.source_drag_cancelled_until_release {
                direct.dragging_source = Some(source);
                direct.hovered_source = Some(source);
                direct.source_rect = response.rect;
                direct.source_rect_frame = frame;
                direct.hovered_target = None;
                direct.hovered_target_valid = false;
                direct.hovered_rect = egui::Rect::NOTHING;
                direct.inspector_rect = egui::Rect::NOTHING;
            }
        });
    }
    let active = ui.data_mut(|data| {
        data.get_temp_mut_or_default::<DirectModulationState>(id)
            .dragging_source
            == Some(source)
    });
    let palette = editor_theme::semantic();
    let focused = response.has_focus();
    let chip = response.rect.shrink2(if paint_label {
        egui::vec2(editor_theme::shape::STROKE, editor_theme::space::XXS)
    } else {
        egui::Vec2::ZERO
    });
    let radius = if paint_label {
        (chip.height() * 0.16).max(editor_theme::shape::FOCUS_STROKE)
    } else {
        (chip.height() * 0.20).max(editor_theme::shape::FOCUS_STROKE)
    };
    let center = if paint_label {
        egui::pos2(
            chip.left() + editor_theme::space::XS + radius,
            chip.center().y,
        )
    } else {
        chip.center()
    };
    ui.painter().circle_filled(
        center,
        radius,
        color.gamma_multiply(if active || response.hovered() {
            1.0
        } else {
            0.76
        }),
    );
    ui.painter().circle_stroke(
        center,
        radius + editor_theme::shape::FOCUS_STROKE,
        egui::Stroke::new(
            if active {
                editor_theme::shape::FOCUS_STROKE
            } else {
                editor_theme::shape::STROKE
            },
            if active || response.hovered() {
                palette.text
            } else {
                color.gamma_multiply(0.42)
            },
        ),
    );
    if paint_label {
        ui.painter().with_clip_rect(chip).text(
            egui::pos2(center.x + radius + editor_theme::space::XS, chip.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            editor_theme::font::label(),
            if active {
                palette.text
            } else if response.hovered() || focused {
                color
            } else {
                color.gamma_multiply(0.82)
            },
        );
    }

    let pointer = ui.input(|input| input.pointer.latest_pos());
    ui.data_mut(|data| {
        let direct = data.get_temp_mut_or_default::<DirectModulationState>(id);
        if direct.dragging_source == Some(source) {
            direct.source_rect = response.rect;
            direct.source_rect_frame = frame;
        } else if direct.dragging_source.is_none() && response.hovered() {
            direct.hovered_source = Some(source);
            direct.source_rect = response.rect;
            direct.source_rect_frame = frame;
        } else if direct.dragging_source.is_none() && direct.hovered_source == Some(source) {
            direct.source_rect_frame = frame;
            if direct.amount_drag.is_none()
                && !pointer.is_some_and(|pointer| {
                    response.rect.contains(pointer) || direct.inspector_rect.contains(pointer)
                })
            {
                direct.hovered_source = None;
                direct.source_rect = egui::Rect::NOTHING;
                direct.source_rect_frame = u64::MAX;
            }
        }
    });
    if active {
        editor_theme::request_display_repaint(ui);
    }
    response
        .clone()
        .on_hover_cursor(if active {
            egui::CursorIcon::Grabbing
        } else {
            egui::CursorIcon::Grab
        })
        .on_hover_text(format!("Drag {label} onto a highlighted parameter"))
}

pub(crate) fn source_drag_active(ui: &egui::Ui) -> bool {
    ui.data_mut(|data| {
        data.get_temp_mut_or_default::<DirectModulationState>(egui::Id::new(UI_STATE_ID))
            .dragging_source
            .is_some()
    })
}

fn clear_source_interaction(direct: &mut DirectModulationState) {
    direct.dragging_source = None;
    direct.hovered_source = None;
    direct.source_rect = egui::Rect::NOTHING;
    direct.source_rect_frame = u64::MAX;
    direct.hovered_target = None;
    direct.hovered_target_valid = false;
    direct.hovered_rect = egui::Rect::NOTHING;
    direct.inspector_rect = egui::Rect::NOTHING;
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

fn target_label(target: UiDestination) -> String {
    match target {
        UiDestination::Host(target) => modulation_target::descriptor(target)
            .map_or("DESTINATION", |target| target.label)
            .to_owned(),
        UiDestination::Modular(ModulationRouteTarget::Oscillator { slot, control, .. }) => {
            format!(
                "OSC {} {}",
                slot.index() + 1,
                oscillator_control_label(control)
            )
        }
        UiDestination::Modular(ModulationRouteTarget::Group { control, .. }) => {
            format!("GROUP {}", group_control_label(control))
        }
        UiDestination::Modular(ModulationRouteTarget::Filter { slot, control, .. }) => {
            format!(
                "FILTER {} {}",
                slot.index() + 1,
                filter_control_label(control)
            )
        }
    }
}

fn oscillator_control_label(control: OscillatorControl) -> &'static str {
    match control {
        OscillatorControl::Shape => "SHAPE",
        OscillatorControl::TablePosition => "VA POSITION",
        OscillatorControl::PulseWidth => "PULSE",
        OscillatorControl::Transpose => "SEMI",
        OscillatorControl::Cents => "CENT",
        OscillatorControl::Level => "LEVEL",
        OscillatorControl::Pan => "PAN",
        OscillatorControl::PhasePosition => "PHASE",
        OscillatorControl::PhaseRandom => "RANDOM PHASE",
        OscillatorControl::PhaseWarpAmount => "WARP",
        OscillatorControl::UnisonVoices => "VOICES",
        OscillatorControl::UnisonRange => "RANGE",
        OscillatorControl::UnisonAmount => "DETUNE",
        OscillatorControl::UnisonCurve => "DISTRIBUTION",
        OscillatorControl::UnisonJitter => "JITTER",
        OscillatorControl::UnisonRate => "JITTER RATE",
        OscillatorControl::UnisonWidth => "WIDTH",
        OscillatorControl::UnisonWeight => "WEIGHT",
        OscillatorControl::UnisonAlignment => "ALIGN",
        OscillatorControl::UnisonPanCurve => "PAN SHAPE",
        OscillatorControl::UnisonPanCenter => "PAN CENTER",
        OscillatorControl::UnisonStereoPosition => "PAN X",
        OscillatorControl::UnisonStereoAlternate => "PAN Y",
    }
}

fn group_control_label(control: GroupControl) -> &'static str {
    match control {
        GroupControl::Gain => "GAIN",
        GroupControl::Pan => "PAN",
        GroupControl::Attack => "ATTACK",
        GroupControl::AttackCurve => "ATTACK CURVE",
        GroupControl::Decay => "DECAY",
        GroupControl::DecayCurve => "DECAY CURVE",
        GroupControl::Sustain => "SUSTAIN",
        GroupControl::Release => "RELEASE",
        GroupControl::ReleaseCurve => "RELEASE CURVE",
    }
}

fn filter_control_label(control: FilterControl) -> &'static str {
    match control {
        FilterControl::Cutoff => "CUTOFF",
        FilterControl::Resonance => "RESONANCE",
    }
}

fn modular_target_color_index(target: ModulationRouteTarget) -> usize {
    match target {
        ModulationRouteTarget::Oscillator { slot, .. } => slot.index(),
        ModulationRouteTarget::Group { group_id, .. } => group_id as usize,
        ModulationRouteTarget::Filter { slot, .. } => MAX_OSCILLATORS + slot.index(),
    }
}
