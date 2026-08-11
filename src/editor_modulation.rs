//! Direct-manipulation modulation routing for the editor.
//!
//! The audio engine still consumes the fixed, host-automatable route bank. This
//! module only gives that bank a source-drag/destination-overlay interface.

use truce_core::editor::{PluginContext, PluginContextReadF32};

use crate::editor_theme;
use crate::generators::{MAX_OSCILLATORS, MAX_OUTPUT_PAIRS};
use crate::modulation_target;
use crate::modulators::routing::{
    GroupControl, HOST_AUTOMATION_SLOT_COUNT, HOST_MODULATION_ROUTE_COUNT, MODULATION_ROUTE_COUNT,
    ModulationRouteTarget, OscillatorControl, ResolvedRouteSource,
};
use crate::modulators::state::{MAX_MODULATION_SOURCES, SourceKind};
use crate::params::HOST_AUTOMATION_PARAMS;
use crate::{KurvParams, P};

const UI_STATE_ID: &str = "kurv-direct-modulation";
const ROUTE_CACHE_ID: &str = "kurv-direct-modulation-routes";
const TARGET_COUNT: usize = modulation_target::TARGETS.len();
const TARGET_COUNT_U8: u8 = TARGET_COUNT as u8;
const ROUTE_COUNT: usize = MODULATION_ROUTE_COUNT;
const HOST_ROUTE_COUNT: usize = HOST_MODULATION_ROUTE_COUNT;
const MODULAR_TARGET_CAPACITY: usize = MAX_OSCILLATORS * OscillatorControl::INTERNAL_TARGET_COUNT
    + MAX_OUTPUT_PAIRS * GroupControl::INTERNAL_TARGET_COUNT;

type UiRoute = (usize, ResolvedRouteSource, f32, bool);

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

#[derive(Clone, Copy)]
struct RouteBucket {
    entries: [UiRoute; ROUTE_COUNT],
    len: usize,
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
    fn as_slice(&self) -> &[UiRoute] {
        &self.entries[..self.len]
    }
}

#[derive(Clone, Copy)]
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

#[derive(Clone, Copy)]
struct DirectModulationState {
    dragging_source: Option<ResolvedRouteSource>,
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

pub(crate) const fn source_color(index: usize) -> egui::Color32 {
    match index % 8 {
        0 => egui::Color32::from_rgb(67, 214, 151),
        1 => egui::Color32::from_rgb(62, 169, 255),
        2 => egui::Color32::from_rgb(198, 112, 255),
        3 => egui::Color32::from_rgb(255, 188, 65),
        4 => egui::Color32::from_rgb(255, 104, 132),
        5 => egui::Color32::from_rgb(80, 220, 224),
        6 => egui::Color32::from_rgb(183, 224, 78),
        _ => egui::Color32::from_rgb(255, 139, 71),
    }
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
    source_handle_for(
        ui,
        state,
        ResolvedRouteSource::Rack(index as u8),
        label,
        response,
    )
}

pub(crate) fn source_handle_for(
    ui: &egui::Ui,
    _state: &PluginContext<KurvParams>,
    source: ResolvedRouteSource,
    label: &str,
    response: &egui::Response,
) -> egui::Response {
    let color = modulation_source_color(source);
    let id = egui::Id::new(UI_STATE_ID);
    let frame = ui.ctx().cumulative_frame_nr();
    if response.drag_started() || response.dragged() {
        ui.data_mut(|data| {
            let mut direct = data
                .get_temp::<DirectModulationState>(id)
                .unwrap_or_default();
            if direct.dragging_source.is_none() {
                direct.dragging_source = Some(source);
                direct.hovered_source = Some(source);
                direct.source_rect = response.rect;
                direct.source_rect_frame = frame;
                direct.hovered_target = None;
                direct.hovered_target_valid = false;
                direct.hovered_rect = egui::Rect::NOTHING;
                direct.inspector_rect = egui::Rect::NOTHING;
            }
            data.insert_temp(id, direct);
        });
    }
    let active = ui
        .data(|data| data.get_temp::<DirectModulationState>(id))
        .is_some_and(|direct| direct.dragging_source == Some(source));
    let radius = (response.rect.height() * 0.20).max(editor_theme::shape::FOCUS_STROKE);
    let center = response.rect.center();
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
                editor_theme::semantic().text
            } else {
                color.gamma_multiply(0.42)
            },
        ),
    );

    let pointer = ui.input(|input| input.pointer.latest_pos());
    ui.data_mut(|data| {
        let mut direct = data
            .get_temp::<DirectModulationState>(id)
            .unwrap_or_default();
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
        data.insert_temp(id, direct);
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
    ui.data(|data| {
        data.get_temp::<DirectModulationState>(egui::Id::new(UI_STATE_ID))
            .is_some_and(|direct| direct.dragging_source.is_some())
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

fn paint_source_drag_feedback(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    direct: &DirectModulationState,
) {
    let Some(pointer) = ui.input(|input| input.pointer.latest_pos()) else {
        return;
    };
    let Some(source) = direct.dragging_source else {
        return;
    };
    if !direct.source_rect.is_positive() {
        return;
    }
    let color = modulation_source_color(source);
    let invalid = direct.hovered_target.is_some() && !direct.hovered_target_valid;
    let bank_full = (0..ROUTE_COUNT).all(|route| route_destination(state, route).is_some());
    let feedback_color = if invalid || (bank_full && direct.hovered_target.is_none()) {
        editor_theme::semantic().danger
    } else {
        color
    };
    let source_label = modulation_source_label(state, source);
    let drag_label = match direct.hovered_target {
        Some(target) if direct.hovered_target_valid => {
            format!("{source_label}  →  {}", target_label(target))
        }
        Some(_) if invalid => format!("{source_label}  ·  ROUTE BANK FULL"),
        None if bank_full => format!("{source_label}  ·  ROUTE BANK FULL"),
        _ => source_label,
    };
    let painter = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("kurv-modulation-cable"),
    ));
    let height = editor_theme::title_height(ui);
    let galley = painter.layout_no_wrap(
        drag_label.clone(),
        editor_theme::font::label(),
        feedback_color,
    );
    let ghost_size = egui::vec2(galley.size().x + height * 1.45, height * 0.86);
    let offset = egui::vec2(height * 0.42, height * 0.38);
    let screen = ui.ctx().content_rect().shrink(editor_theme::space::XS);
    let visual_pointer = clamp_point(pointer, screen, height * 0.23);
    let mut ghost = egui::Rect::from_min_size(visual_pointer + offset, ghost_size);
    if ghost.right() > screen.right() {
        ghost = egui::Rect::from_min_size(
            visual_pointer - egui::vec2(ghost_size.x + offset.x, -offset.y),
            ghost_size,
        );
    }
    ghost = clamp_overlay_rect(ghost, screen);

    let start = clamp_point(direct.source_rect.center(), screen, 0.0);
    let bend = (visual_pointer.x - start.x).abs().max(height) * 0.38;
    let direction = if visual_pointer.x >= start.x {
        1.0
    } else {
        -1.0
    };
    painter.add(egui::Shape::line(
        cubic_bezier_points(
            start,
            start + egui::vec2(direction * bend, 0.0),
            visual_pointer - egui::vec2(direction * bend, 0.0),
            visual_pointer,
            24,
        ),
        egui::Stroke::new(height * 0.055, feedback_color.gamma_multiply(0.72)),
    ));
    painter.circle_filled(visual_pointer, height * 0.14, feedback_color);
    painter.circle_stroke(
        visual_pointer,
        height * 0.23,
        egui::Stroke::new(height * 0.045, feedback_color.gamma_multiply(0.68)),
    );
    painter.rect_filled(
        ghost,
        editor_theme::shape::CONTROL_RADIUS,
        editor_theme::semantic().surface,
    );
    painter.rect_stroke(
        ghost,
        editor_theme::shape::CONTROL_RADIUS,
        egui::Stroke::new(editor_theme::shape::FOCUS_STROKE, feedback_color),
        egui::StrokeKind::Inside,
    );
    let grip = ghost.left_center() + egui::vec2(height * 0.44, 0.0);
    for column in 0..2 {
        for row in 0..3 {
            painter.circle_filled(
                grip + egui::vec2(
                    column as f32 * height * 0.13,
                    (row as f32 - 1.0) * height * 0.14,
                ),
                height * 0.045,
                feedback_color,
            );
        }
    }
    painter.text(
        ghost.left_center() + egui::vec2(height * 0.92, 0.0),
        egui::Align2::LEFT_CENTER,
        drag_label,
        editor_theme::font::label(),
        feedback_color,
    );
}

fn modulation_source_label(
    state: &PluginContext<KurvParams>,
    source: ResolvedRouteSource,
) -> String {
    let ResolvedRouteSource::Rack(source) = source else {
        return "MOD WHEEL".to_owned();
    };
    let index = usize::from(source);
    let envelope = if index < 8 {
        let kind = match index {
            0 => P::Source1Envelope,
            1 => P::Source2Envelope,
            2 => P::Source3Envelope,
            3 => P::Source4Envelope,
            4 => P::Source5Envelope,
            5 => P::Source6Envelope,
            6 => P::Source7Envelope,
            _ => P::Source8Envelope,
        };
        state.get_param(kind) >= 0.5
    } else {
        state.params().modulator_rack.config(index).kind == SourceKind::Envelope
    };
    format!("{} {}", if envelope { "ENV" } else { "LFO" }, index + 1)
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
    _base: f32,
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
        let mut direct = data
            .get_temp::<DirectModulationState>(id)
            .unwrap_or_default();
        prepare_target_frame(&mut direct, frame);
        direct.target_rects[usize::from(target - 1)] = visible_rect;
        data.insert_temp(id, direct);
    });
    let routes = routes_for_target(ui, state, target);
    let live_base = effective_normalized(state, param);
    paint_destination_routes(
        ui,
        response,
        track,
        axis,
        live_base,
        display_span(target),
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
        host_automation_context_menu(ui, state, target, response, base);
        return false;
    }
    let visible_rect = response.interact_rect.intersect(ui.clip_rect());
    if !visible_rect.is_positive() {
        return false;
    }
    let id = egui::Id::new(UI_STATE_ID);
    let frame = ui.ctx().cumulative_frame_nr();
    host_automation_context_menu(ui, state, target, response, base);
    ui.data_mut(|data| {
        let mut direct = data
            .get_temp::<DirectModulationState>(id)
            .unwrap_or_default();
        prepare_target_frame(&mut direct, frame);
        if let Some(existing) = direct.modular_target_rects[..direct.modular_target_len]
            .iter_mut()
            .find(|entry| entry.target == Some(target))
        {
            existing.rect = visible_rect;
        } else if direct.modular_target_len < MODULAR_TARGET_CAPACITY {
            direct.modular_target_rects[direct.modular_target_len] = ModularTargetRect {
                target: Some(target),
                rect: visible_rect,
            };
            direct.modular_target_len += 1;
        }
        data.insert_temp(id, direct);
    });
    let routes = routes_for_modular_target(state, target);
    let live_base = routes
        .as_slice()
        .iter()
        .fold(base, |value, (route, source, _, _)| {
            let amount = route_amount(state, *route);
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
pub(crate) fn host_automation_destination(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    target: ModulationRouteTarget,
    response: &egui::Response,
    base: f32,
) {
    host_automation_context_menu(ui, state, target, response, base);
}

pub(crate) fn update_host_automation_gesture(
    state: &PluginContext<KurvParams>,
    param: P,
    response: &egui::Response,
    normalized: f32,
    changed: bool,
) {
    if response.drag_started() {
        state.begin_edit(param);
    }
    if changed {
        if response.dragged() {
            state.set_param(param, f64::from(normalized.clamp(0.0, 1.0)));
        } else {
            state.begin_edit(param);
            state.set_param(param, f64::from(normalized.clamp(0.0, 1.0)));
            state.end_edit(param);
        }
    }
    if response.drag_stopped() {
        state.end_edit(param);
    }
}

fn host_automation_context_menu(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    target: ModulationRouteTarget,
    response: &egui::Response,
    base: f32,
) {
    response.context_menu(|ui| host_automation_menu(ui, state, target, base));
    paint_host_automation_badge(ui, state, target, response);
}

pub(crate) fn host_automation_menu(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    target: ModulationRouteTarget,
    base: f32,
) {
    let assigned = host_automation_slot(state, target);
    ui.spacing_mut().item_spacing.y = editor_theme::compact_gap(ui);
    ui.set_min_width(editor_theme::title_height(ui) * 7.0);
    ui.label(
        egui::RichText::new(target_label(UiDestination::Modular(target)))
            .font(editor_theme::font::caption())
            .color(editor_theme::semantic().text_muted),
    );
    if let Some(slot) = assigned {
        ui.label(
            egui::RichText::new(format!("HOST {:02}", slot + 1))
                .font(editor_theme::font::value())
                .color(editor_theme::semantic().primary),
        );
        if ui.button("Remove host assignment").clicked() {
            let normalized = state.get_param(HOST_AUTOMATION_PARAMS[slot]);
            commit_host_value_to_target(state, target, normalized);
            state.params().host_automation_targets.clear(slot);
            ui.close();
        }
    } else if let Some(slot) = first_free_host_automation_slot(state) {
        if ui.button("Make host modulatable").clicked() {
            let param = HOST_AUTOMATION_PARAMS[slot];
            state.begin_edit(param);
            state.set_param(param, f64::from(base.clamp(0.0, 1.0)));
            state.end_edit(param);
            state.params().host_automation_targets.set(slot, target);
            ui.close();
        }
    } else {
        ui.add_enabled(false, egui::Button::new("Host automation bank full"));
    }
}

fn paint_host_automation_badge(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    target: ModulationRouteTarget,
    response: &egui::Response,
) {
    let Some(slot) = host_automation_slot(state, target) else {
        return;
    };
    let visible = response.rect.intersect(ui.clip_rect());
    if !visible.is_positive() {
        return;
    }
    let accent = editor_theme::semantic().primary;
    let color = accent.gamma_multiply(if response.hovered() { 1.0 } else { 0.68 });
    let galley = ui.painter().layout_no_wrap(
        format!("H{:02}", slot + 1),
        editor_theme::font::caption(),
        color,
    );
    let padding = egui::vec2(editor_theme::space::XXS, editor_theme::shape::STROKE);
    let size = galley.size() + padding * 2.0;
    if visible.width() < size.x + editor_theme::space::XXS
        || visible.height() < size.y + editor_theme::space::XXS
    {
        return;
    }
    let rect = egui::Rect::from_min_size(
        visible.right_top()
            + egui::vec2(-size.x - editor_theme::space::XXS, editor_theme::space::XXS),
        size,
    );
    ui.painter().rect_filled(
        rect,
        editor_theme::shape::CONTROL_RADIUS,
        editor_theme::semantic().well,
    );
    ui.painter().rect_stroke(
        rect,
        editor_theme::shape::CONTROL_RADIUS,
        egui::Stroke::new(editor_theme::shape::STROKE, color.gamma_multiply(0.55)),
        egui::StrokeKind::Inside,
    );
    ui.painter().galley(rect.min + padding, galley, color);
}

fn host_automation_slot(
    state: &PluginContext<KurvParams>,
    target: ModulationRouteTarget,
) -> Option<usize> {
    (0..HOST_AUTOMATION_SLOT_COUNT)
        .find(|slot| state.params().host_automation_targets.get(*slot) == Some(target))
}

pub(crate) fn host_automation_binding(
    state: &PluginContext<KurvParams>,
    target: ModulationRouteTarget,
) -> Option<(usize, P, f32)> {
    let slot = host_automation_slot(state, target)?;
    let param = HOST_AUTOMATION_PARAMS[slot];
    Some((slot, param, state.get_param(param).clamp(0.0, 1.0)))
}

fn first_free_host_automation_slot(state: &PluginContext<KurvParams>) -> Option<usize> {
    (0..HOST_AUTOMATION_SLOT_COUNT)
        .find(|slot| state.params().host_automation_targets.get(*slot).is_none())
}

fn commit_host_value_to_target(
    state: &PluginContext<KurvParams>,
    target: ModulationRouteTarget,
    normalized: f32,
) {
    match target {
        ModulationRouteTarget::Oscillator {
            module_id,
            slot,
            control,
        } => {
            let patch = state.generator_stack.snapshot();
            let valid = patch.groups().iter().any(|group| {
                group.modules().iter().any(|module| {
                    module.id().get() == module_id && module.oscillator_slot() == Some(slot)
                })
            });
            if valid {
                let mut config = state.generator_stack.oscillator_config(slot);
                control.apply_normalized(&mut config, normalized);
                state.generator_stack.set_oscillator_config(slot, config);
            }
        }
        ModulationRouteTarget::Group { group_id, control } => {
            let patch = state.generator_stack.snapshot();
            if let Some(group) = patch
                .groups()
                .iter()
                .find(|group| group.id().get() == group_id)
            {
                let mut output = group.output();
                control.apply_normalized(&mut output, normalized);
                state.generator_stack.set_group_output(group.id(), output);
            }
        }
    }
}

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

#[allow(clippy::too_many_arguments)]
fn paint_destination_routes(
    ui: &egui::Ui,
    response: &egui::Response,
    track: egui::Rect,
    axis: TrackAxis,
    live_base: f32,
    span: f32,
    routes: &RouteBucket,
    color_index: usize,
) -> bool {
    let id = egui::Id::new(UI_STATE_ID);
    let unit = modulation_unit(ui);
    // Handle widgets are registered by the root overlay, so use the viewport
    // bounds here instead of the destination's nested child clip. This lets a
    // compact control place its modulation knob outside its own rectangle.
    let clip_rect = ui.ctx().content_rect();
    ui.data_mut(|data| {
        let mut direct = data
            .get_temp::<DirectModulationState>(id)
            .unwrap_or_default();
        for (lane, (route, _, amount, _)) in routes.as_slice().iter().enumerate() {
            direct.route_handle_positions[*route] =
                route_handle_position(track, lane, routes.len, *amount, clip_rect, unit);
            direct.route_handle_mask |= 1_u64 << *route;
        }
        data.insert_temp(id, direct);
    });
    let direct = ui
        .data(|data| data.get_temp::<DirectModulationState>(id))
        .unwrap_or_default();
    let source_highlight = direct.hovered_source.is_some()
        && routes
            .as_slice()
            .iter()
            .any(|(_, source, _, _)| Some(*source) == direct.hovered_source);
    let pointer = ui.input(|input| input.pointer.latest_pos());
    let hovered_route = pointer
        .and_then(|pointer| route_handle_hit(pointer, track, routes.as_slice(), clip_rect, unit));
    let show_handles = response.hovered()
        || hovered_route.is_some()
        || source_highlight
        || direct.amount_drag.is_some_and(|drag| {
            routes
                .as_slice()
                .iter()
                .any(|(route, _, _, _)| *route == drag.route)
        });
    paint_routes(
        ui,
        track,
        axis,
        live_base,
        span,
        routes.as_slice(),
        direct.hovered_source,
        hovered_route,
        direct.amount_drag.map(|drag| drag.route),
        show_handles,
        clip_rect,
        unit,
    );
    if source_highlight {
        let source = direct
            .hovered_source
            .expect("source highlight requires a hovered source");
        brighten_control(ui, response.rect, modulation_source_color(source), 22);
    }
    if !routes.as_slice().is_empty() {
        paint_live_value(ui, track, axis, live_base, source_color(color_index % 8));
        editor_theme::request_display_repaint(ui);
    }
    if hovered_route.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }

    false
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
    let routes = routes_for_modular_target(state, target);
    owns_routes_gesture(ui, state, response, &routes)
}

fn owns_routes_gesture(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    response: &egui::Response,
    routes: &RouteBucket,
) -> bool {
    let id = egui::Id::new(UI_STATE_ID);
    let mut direct = ui
        .data(|data| data.get_temp::<DirectModulationState>(id))
        .unwrap_or_default();
    // The real widget is registered by the final overlay pass, after all base
    // controls. Read that response here so the base parameter never steals the
    // handle gesture.
    for (route, _, _, _) in routes.as_slice() {
        let amount = route_amount(state, *route);
        let Some(handle_response) = ui.ctx().read_response(route_handle_id(*route)) else {
            continue;
        };
        if handle_response.hovered() || direct.amount_drag.is_some_and(|drag| drag.route == *route)
        {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        if handle_response.double_clicked() {
            finish_amount_drag(state, &mut direct, true);
            clear_route(state, *route);
            ui.data_mut(|data| data.insert_temp(id, direct));
            return true;
        }
        if handle_response.drag_started() {
            finish_amount_drag(state, &mut direct, false);
            begin_route_amount_edit(state, *route);
            direct.amount_drag = Some(AmountDrag {
                route: *route,
                amount,
                initial_amount: amount,
            });
            ui.data_mut(|data| data.insert_temp(id, direct));
        }
        if direct.amount_drag.is_some_and(|drag| drag.route == *route) {
            if handle_response.dragged() {
                let drag = direct
                    .amount_drag
                    .as_mut()
                    .expect("route drag checked above");
                update_route_amount(state, &handle_response, drag);
                ui.data_mut(|data| data.insert_temp(id, direct));
                editor_theme::request_display_repaint(ui);
                return true;
            }
            if handle_response.drag_stopped() {
                finish_amount_drag(state, &mut direct, false);
                ui.data_mut(|data| data.insert_temp(id, direct));
                return true;
            }
        }
    }

    // Keep the old parent-response path as a fallback for compact controls
    // whose clip rect cannot contain an external handle.
    let clip_rect = ui.ctx().content_rect();
    let unit = modulation_unit(ui);
    let hovered = ui
        .input(|input| input.pointer.latest_pos())
        .and_then(|pointer| {
            route_handle_hit(pointer, response.rect, routes.as_slice(), clip_rect, unit)
        });
    if response.double_clicked()
        && let Some(route) = hovered
    {
        finish_amount_drag(state, &mut direct, true);
        clear_route(state, route);
        ui.data_mut(|data| data.insert_temp(id, direct));
        return true;
    }
    if response.drag_started()
        && let Some(route) = hovered
    {
        finish_amount_drag(state, &mut direct, false);
        let amount = route_amount(state, route);
        begin_route_amount_edit(state, route);
        direct.amount_drag = Some(AmountDrag {
            route,
            amount,
            initial_amount: amount,
        });
        ui.data_mut(|data| data.insert_temp(id, direct));
    }

    if let Some(drag) = direct.amount_drag
        && routes
            .as_slice()
            .iter()
            .any(|(route, _, _, _)| *route == drag.route)
    {
        if response.dragged() {
            let drag = direct
                .amount_drag
                .as_mut()
                .expect("route drag checked above");
            update_route_amount(state, response, drag);
            ui.data_mut(|data| data.insert_temp(id, direct));
            editor_theme::request_display_repaint(ui);
            return true;
        }
        if response.drag_stopped() {
            finish_amount_drag(state, &mut direct, false);
            ui.data_mut(|data| data.insert_temp(id, direct));
            return true;
        }
    }
    false
}

fn route_handle_hit(
    pointer: egui::Pos2,
    track: egui::Rect,
    routes: &[UiRoute],
    clip_rect: egui::Rect,
    unit: f32,
) -> Option<usize> {
    routes
        .iter()
        .enumerate()
        .filter_map(|(lane, (route, _, amount, _))| {
            let handle = route_handle_position(track, lane, routes.len(), *amount, clip_rect, unit);
            (pointer.distance(handle) <= modulation_handle_hit_radius(unit))
                .then_some((*route, pointer.distance(handle)))
        })
        .min_by(|left, right| left.1.total_cmp(&right.1))
        .map(|(route, _)| route)
}

fn route_handle_hit_rect(center: egui::Pos2, unit: f32) -> egui::Rect {
    let diameter = modulation_handle_hit_radius(unit) * 2.0;
    egui::Rect::from_center_size(center, egui::vec2(diameter, diameter))
}

fn route_handle_id(route: usize) -> egui::Id {
    egui::Id::new((UI_STATE_ID, "modulation-handle", route))
}

fn modulation_drag_delta(response: &egui::Response) -> f32 {
    let delta = response.drag_motion();
    delta.x - delta.y
}

fn update_route_amount(
    state: &PluginContext<KurvParams>,
    response: &egui::Response,
    drag: &mut AmountDrag,
) {
    drag.amount = (drag.amount + modulation_drag_delta(response) / 120.0).clamp(-1.0, 1.0);
    set_route_amount(state, drag.route, drag.amount);
}

fn finish_amount_drag(
    state: &PluginContext<KurvParams>,
    direct: &mut DirectModulationState,
    cancelled: bool,
) {
    let Some(drag) = direct.amount_drag.take() else {
        return;
    };
    if cancelled {
        set_route_amount(state, drag.route, drag.initial_amount);
    }
    end_route_amount_edit(state, drag.route);
    if !cancelled && route_amount(state, drag.route).abs() <= 0.005 {
        clear_route(state, drag.route);
    }
}

fn routes_for_target(ui: &egui::Ui, state: &PluginContext<KurvParams>, target: u8) -> RouteBucket {
    let frame = ui.ctx().cumulative_frame_nr();
    let id = egui::Id::new(ROUTE_CACHE_ID);
    let mut cache = ui
        .data(|data| data.get_temp::<RouteCache>(id))
        .unwrap_or_default();
    if cache.frame != frame {
        cache = RouteCache {
            frame,
            ..RouteCache::default()
        };
        let mod_wheel_mask = state.params().mod_wheel_route_mask.load();
        for (index, (source, _, amount, _)) in ROUTES.iter().enumerate() {
            if state.params().modulation_route_targets.get(index).is_some() {
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
        ui.data_mut(|data| data.insert_temp(id, cache));
    }
    cache.targets[usize::from(target - 1)]
}

fn routes_for_modular_target(
    state: &PluginContext<KurvParams>,
    target: ModulationRouteTarget,
) -> RouteBucket {
    let mut bucket = RouteBucket::default();
    for index in 0..ROUTE_COUNT {
        if state.params().modulation_route_targets.get(index) != Some(target) {
            continue;
        }
        let source = route_source(state, index);
        let Some(source) = source else {
            continue;
        };
        bucket.entries[bucket.len] = (
            index,
            source,
            route_amount(state, index),
            source_is_bipolar(state, source),
        );
        bucket.len += 1;
    }
    bucket
}

fn paint_routes(
    ui: &egui::Ui,
    track: egui::Rect,
    axis: TrackAxis,
    base: f32,
    span: f32,
    routes: &[UiRoute],
    hovered_source: Option<ResolvedRouteSource>,
    hovered_route: Option<usize>,
    active_route: Option<usize>,
    show_handles: bool,
    clip_rect: egui::Rect,
    unit: f32,
) {
    for (lane, (route, source, amount, bipolar)) in routes.iter().enumerate() {
        let (start_value, end_value) = route_range(base, span, *amount, *bipolar);
        let offset = lane as f32 * editor_theme::shape::FOCUS_STROKE;
        let (start, finish) = match axis {
            TrackAxis::Horizontal => (
                egui::pos2(
                    egui::lerp(track.left()..=track.right(), start_value),
                    track.bottom() - offset,
                ),
                egui::pos2(
                    egui::lerp(track.left()..=track.right(), end_value),
                    track.bottom() - offset,
                ),
            ),
            TrackAxis::Vertical => (
                egui::pos2(
                    track.right() - offset,
                    egui::lerp(track.bottom()..=track.top(), start_value),
                ),
                egui::pos2(
                    track.right() - offset,
                    egui::lerp(track.bottom()..=track.top(), end_value),
                ),
            ),
        };
        let color = modulation_source_color(*source);
        let stroke = if Some(*source) == hovered_source {
            egui::Stroke::new(
                editor_theme::shape::FOCUS_STROKE + editor_theme::shape::STROKE,
                color,
            )
        } else {
            egui::Stroke::new(editor_theme::shape::STROKE, color)
        };
        ui.painter().line_segment([start, finish], stroke);
        if show_handles {
            let handle = route_handle_position(track, lane, routes.len(), *amount, clip_rect, unit);
            let hovered = hovered_route == Some(*route);
            let painter = ui.ctx().layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("kurv-modulation-knobs"),
            ));
            paint_modulation_knob(
                &painter,
                handle,
                color,
                *amount,
                unit,
                hovered,
                active_route == Some(*route),
            );
        }
    }
}

fn route_handle_position(
    track: egui::Rect,
    lane: usize,
    route_count: usize,
    amount: f32,
    clip_rect: egui::Rect,
    unit: f32,
) -> egui::Pos2 {
    let lane_center = route_count.saturating_sub(1) as f32 * 0.5;
    let y = track.center().y + (lane as f32 - lane_center) * modulation_handle_lane_spacing(unit);
    let outset = modulation_knob_radius(unit) + editor_theme::space::XXS;
    let x = if amount >= 0.0 {
        track.right() + outset
    } else {
        track.left() - outset
    };
    let outside = egui::pos2(x, y);
    let hit_radius = modulation_handle_hit_radius(unit);
    if clip_rect.is_positive() && clip_rect.contains_rect(route_handle_hit_rect(outside, unit)) {
        outside
    } else {
        egui::pos2(
            inset_clamp(
                if amount >= 0.0 {
                    track.right() - hit_radius
                } else {
                    track.left() + hit_radius
                },
                clip_rect.left(),
                clip_rect.right(),
                hit_radius,
            ),
            inset_clamp(y, clip_rect.top(), clip_rect.bottom(), hit_radius),
        )
    }
}

fn inset_clamp(value: f32, min: f32, max: f32, inset: f32) -> f32 {
    let low = min + inset;
    let high = max - inset;
    if low <= high {
        value.clamp(low, high)
    } else {
        (min + max) * 0.5
    }
}

fn clamp_point(point: egui::Pos2, bounds: egui::Rect, inset: f32) -> egui::Pos2 {
    egui::pos2(
        inset_clamp(point.x, bounds.left(), bounds.right(), inset),
        inset_clamp(point.y, bounds.top(), bounds.bottom(), inset),
    )
}

fn clamp_overlay_rect(rect: egui::Rect, bounds: egui::Rect) -> egui::Rect {
    let max_x = (bounds.right() - rect.width()).max(bounds.left());
    let max_y = (bounds.bottom() - rect.height()).max(bounds.top());
    egui::Rect::from_min_size(
        egui::pos2(
            rect.left().clamp(bounds.left(), max_x),
            rect.top().clamp(bounds.top(), max_y),
        ),
        rect.size(),
    )
}

fn paint_modulation_knob(
    painter: &egui::Painter,
    center: egui::Pos2,
    color: egui::Color32,
    amount: f32,
    unit: f32,
    hovered: bool,
    active: bool,
) {
    const START: f32 = std::f32::consts::FRAC_PI_2 * 1.5;
    const SWEEP: f32 = std::f32::consts::TAU * 0.75;
    let base_radius = modulation_knob_radius(unit);
    let radius = if active {
        base_radius + editor_theme::shape::FOCUS_STROKE
    } else if hovered {
        base_radius + editor_theme::shape::STROKE
    } else {
        base_radius
    };
    let depth = amount.abs().clamp(0.0, 1.0);
    painter.circle_filled(center, radius, editor_theme::semantic().well);
    painter.circle_stroke(
        center,
        radius - editor_theme::shape::STROKE * 0.5,
        egui::Stroke::new(
            if active {
                editor_theme::shape::FOCUS_STROKE
            } else {
                editor_theme::shape::STROKE
            },
            if active {
                color.gamma_multiply(0.8)
            } else {
                editor_theme::semantic().grid
            },
        ),
    );
    painter.add(egui::Shape::line(
        modulation_arc_points(center, radius - editor_theme::space::XXS, START, SWEEP, 24),
        egui::Stroke::new(
            editor_theme::shape::STROKE,
            editor_theme::semantic().control_hover,
        ),
    ));
    if depth > f32::EPSILON {
        let arc_start = if amount < 0.0 { START + SWEEP } else { START };
        let arc_sweep = if amount < 0.0 {
            -SWEEP * depth
        } else {
            SWEEP * depth
        };
        painter.add(egui::Shape::line(
            modulation_arc_points(
                center,
                radius - editor_theme::space::XXS,
                arc_start,
                arc_sweep,
                24,
            ),
            egui::Stroke::new(
                if hovered {
                    editor_theme::shape::FOCUS_STROKE + editor_theme::shape::STROKE
                } else {
                    editor_theme::shape::FOCUS_STROKE
                },
                color,
            ),
        ));
    }
}

fn brighten_control(ui: &egui::Ui, rect: egui::Rect, color: egui::Color32, alpha: u8) {
    let [red, green, blue, _] = color.to_array();
    ui.painter().rect_filled(
        rect,
        editor_theme::shape::CONTROL_RADIUS,
        egui::Color32::from_rgba_unmultiplied(red, green, blue, alpha),
    );
}

fn paint_drop_target(
    painter: &egui::Painter,
    rect: egui::Rect,
    color: egui::Color32,
    hovered: bool,
    valid: bool,
) {
    if !valid && !hovered {
        return;
    }
    let feedback = if valid {
        color
    } else if hovered {
        editor_theme::semantic().danger
    } else {
        editor_theme::semantic().disabled_text
    };
    let [red, green, blue, _] = feedback.to_array();
    painter.rect_filled(
        rect,
        editor_theme::shape::CONTROL_RADIUS,
        egui::Color32::from_rgba_unmultiplied(
            red,
            green,
            blue,
            if hovered && valid {
                44
            } else if hovered {
                24
            } else if valid {
                12
            } else {
                4
            },
        ),
    );
    painter.rect_stroke(
        rect.shrink(editor_theme::shape::STROKE * 0.5),
        editor_theme::shape::CONTROL_RADIUS,
        egui::Stroke::new(
            if hovered {
                editor_theme::shape::FOCUS_STROKE
            } else {
                editor_theme::shape::STROKE
            },
            feedback.gamma_multiply(if hovered { 1.0 } else { 0.38 }),
        ),
        egui::StrokeKind::Inside,
    );
    if hovered && !valid {
        let half = (rect.width().min(rect.height()) * 0.12)
            .clamp(editor_theme::space::XXS, editor_theme::space::XS);
        let center = rect.center();
        painter.line_segment(
            [
                center - egui::vec2(half, half),
                center + egui::vec2(half, half),
            ],
            egui::Stroke::new(editor_theme::shape::FOCUS_STROKE, feedback),
        );
        painter.line_segment(
            [
                center + egui::vec2(-half, half),
                center + egui::vec2(half, -half),
            ],
            egui::Stroke::new(editor_theme::shape::FOCUS_STROKE, feedback),
        );
    }
}

fn modulation_arc_points(
    center: egui::Pos2,
    radius: f32,
    start: f32,
    sweep: f32,
    segments: usize,
) -> Vec<egui::Pos2> {
    (0..=segments)
        .map(|index| {
            let t = index as f32 / segments.max(1) as f32;
            let angle = start + sweep * t;
            center + egui::Vec2::angled(angle) * radius
        })
        .collect()
}

fn cubic_bezier_points(
    start: egui::Pos2,
    control_a: egui::Pos2,
    control_b: egui::Pos2,
    end: egui::Pos2,
    segments: usize,
) -> Vec<egui::Pos2> {
    (0..=segments)
        .map(|index| {
            let t = index as f32 / segments.max(1) as f32;
            let inverse = 1.0 - t;
            let point = start.to_vec2() * inverse.powi(3)
                + control_a.to_vec2() * (3.0 * inverse.powi(2) * t)
                + control_b.to_vec2() * (3.0 * inverse * t.powi(2))
                + end.to_vec2() * t.powi(3);
            egui::pos2(point.x, point.y)
        })
        .collect()
}

fn route_range(base: f32, span: f32, amount: f32, bipolar: bool) -> (f32, f32) {
    if bipolar {
        let extent = amount.abs() * span;
        (
            (base - extent).clamp(0.0, 1.0),
            (base + extent).clamp(0.0, 1.0),
        )
    } else {
        (base, amount.mul_add(span, base).clamp(0.0, 1.0))
    }
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

fn assign_route(state: &PluginContext<KurvParams>, source: ResolvedRouteSource, target: u8) {
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

fn assign_modular_route(
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

fn route_for_assignment(
    state: &PluginContext<KurvParams>,
    source: ResolvedRouteSource,
    target: u8,
) -> Option<(usize, bool)> {
    if let Some(route) = (0..HOST_ROUTE_COUNT).find(|&route| {
        state.params().modulation_route_targets.get(route).is_none()
            && route_source(state, route) == Some(source)
            && route_target(state, route) == target
    }) {
        return Some((route, true));
    }
    (0..HOST_ROUTE_COUNT)
        .find(|&route| route_destination(state, route).is_none())
        .map(|route| (route, false))
}

fn route_for_modular_assignment(
    state: &PluginContext<KurvParams>,
    source: ResolvedRouteSource,
    target: ModulationRouteTarget,
) -> Option<(usize, bool)> {
    if let Some(route) = (0..ROUTE_COUNT).find(|&route| {
        route_source(state, route) == Some(source)
            && state.params().modulation_route_targets.get(route) == Some(target)
    }) {
        return Some((route, true));
    }
    (0..ROUTE_COUNT)
        .find(|&route| route_destination(state, route).is_none())
        .map(|route| (route, false))
}

pub(crate) fn used_source_mask(state: &PluginContext<KurvParams>) -> u64 {
    (0..ROUTE_COUNT).fold(0, |mask, route| {
        let source = route_source(state, route);
        if let Some(ResolvedRouteSource::Rack(source)) = source
            && route_destination(state, route).is_some()
            && route_amount(state, route).abs() > f32::EPSILON
        {
            mask | (1_u64 << source)
        } else {
            mask
        }
    })
}

pub(crate) fn clear_source(state: &PluginContext<KurvParams>, source: u8) {
    let source = ResolvedRouteSource::Rack(source.saturating_sub(1));
    for route in 0..ROUTE_COUNT {
        if route_source(state, route) == Some(source) {
            clear_route(state, route);
        }
    }
}

/// Paints the source-hover route editor after every destination has registered
/// its current frame geometry. Destination controls keep their own base-value
/// hit testing; this final pass owns the modulation handles and popup.
pub(crate) fn draw_overlay(ui: &mut egui::Ui, state: &PluginContext<KurvParams>) {
    let id = egui::Id::new(UI_STATE_ID);
    let mut direct = ui
        .data(|data| data.get_temp::<DirectModulationState>(id))
        .unwrap_or_default();
    let frame = ui.ctx().cumulative_frame_nr();
    prepare_target_frame(&mut direct, frame);
    let (escape_pressed, primary_down) = ui.input(|input| {
        (
            input.key_pressed(egui::Key::Escape),
            input.pointer.primary_down(),
        )
    });
    if direct.amount_drag.is_some() && (escape_pressed || !primary_down) {
        finish_amount_drag(state, &mut direct, escape_pressed);
        ui.data_mut(|data| data.insert_temp(id, direct));
    }
    if direct.source_rect_frame != frame && direct.amount_drag.is_none() {
        clear_source_interaction(&mut direct);
    }
    if direct.dragging_source.is_some() {
        update_drop_targets(ui, state, &mut direct);
        paint_source_drag_feedback(ui, state, &direct);
        let (released, pointer) = ui.input(|input| {
            (
                input.pointer.button_released(egui::PointerButton::Primary),
                input.pointer.latest_pos(),
            )
        });
        if escape_pressed || released || !primary_down {
            if released
                && !escape_pressed
                && direct.hovered_target_valid
                && pointer.is_some_and(|pointer| direct.hovered_rect.contains(pointer))
                && let Some(target) = direct.hovered_target
            {
                let source = direct
                    .dragging_source
                    .expect("active source drag requires a source");
                match target {
                    UiDestination::Host(target) => {
                        assign_route(state, source, target);
                    }
                    UiDestination::Modular(target) => {
                        assign_modular_route(state, source, target);
                    }
                }
            }
            clear_source_interaction(&mut direct);
        }
        ui.data_mut(|data| data.insert_temp(id, direct));
    }
    register_route_handle_widgets(ui, state, direct);
    if direct.dragging_source.is_some() || direct.hovered_source.is_none() {
        clear_inspector_rect(ui, id);
        return;
    }
    let source = direct.hovered_source.expect("hovered source checked above");
    let routes = routes_for_source(state, source);
    if routes.len == 0 {
        clear_inspector_rect(ui, id);
        return;
    }
    let pointer = ui.input(|input| input.pointer.latest_pos());
    let dragging_source_route = direct
        .amount_drag
        .is_some_and(|drag| route_source(state, drag.route) == Some(source));
    if !dragging_source_route
        && !pointer.is_some_and(|pointer| {
            direct.source_rect.contains(pointer) || direct.inspector_rect.contains(pointer)
        })
    {
        ui.data_mut(|data| {
            let mut direct = data
                .get_temp::<DirectModulationState>(id)
                .unwrap_or_default();
            direct.hovered_source = None;
            direct.source_rect = egui::Rect::NOTHING;
            direct.inspector_rect = egui::Rect::NOTHING;
            data.insert_temp(id, direct);
        });
        return;
    }

    let title_height = editor_theme::title_height(ui);
    let row_height = title_height * 0.88;
    let inset = editor_theme::space::XS;
    let compact_gap = editor_theme::compact_gap(ui);
    let screen = ui.ctx().content_rect().shrink(editor_theme::space::XXS);
    let width = (direct.source_rect.width() * 1.12)
        .clamp(title_height * 7.6, title_height * 10.0)
        .min(screen.width());
    let header_height = title_height * 0.72;
    let ideal_rows_height =
        routes.len as f32 * row_height + routes.len.saturating_sub(1) as f32 * compact_gap;
    let rows_height = ideal_rows_height
        .min((screen.height() - inset * 2.0 - header_height - compact_gap).max(0.0));
    let height = inset * 2.0 + header_height + compact_gap + rows_height;
    let below =
        egui::Rect::from_min_size(direct.source_rect.left_bottom(), egui::vec2(width, height));
    let mut popup_rect =
        if below.bottom() <= screen.bottom() || direct.source_rect.top() - height < screen.top() {
            below
        } else {
            egui::Rect::from_min_size(
                egui::pos2(direct.source_rect.left(), direct.source_rect.top() - height),
                egui::vec2(width, height),
            )
        };
    popup_rect = clamp_overlay_rect(popup_rect, screen);

    let mut hovered_link = None;
    let color = modulation_source_color(source);
    let output = egui::Area::new(egui::Id::new("kurv-source-routes"))
        .order(egui::Order::Foreground)
        .fixed_pos(popup_rect.min)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style())
                .fill(editor_theme::semantic().chrome)
                .stroke(egui::Stroke::new(
                    editor_theme::shape::STROKE,
                    editor_theme::semantic().grid,
                ))
                .inner_margin(egui::Margin::same(inset.round() as i8))
                .show(ui, |ui| {
                    ui.set_width(width - inset * 2.0);
                    ui.spacing_mut().item_spacing =
                        egui::vec2(editor_theme::space::XXS, compact_gap);
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(modulation_source_label(state, source))
                                .font(editor_theme::font::label())
                                .color(color),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} {}",
                                    routes.len,
                                    if routes.len == 1 { "ROUTE" } else { "ROUTES" }
                                ))
                                .font(editor_theme::font::caption())
                                .color(editor_theme::semantic().text_muted),
                            );
                        });
                    });
                    egui::ScrollArea::vertical()
                        .id_salt(("kurv-source-routes-scroll", source))
                        .auto_shrink([false, false])
                        .max_height(rows_height)
                        .show(ui, |ui| {
                            ui.set_width(width - inset * 2.0);
                            for &(route, _, _, _) in routes.as_slice() {
                                let Some(target) = route_destination(state, route) else {
                                    continue;
                                };
                                let active =
                                    direct.amount_drag.is_some_and(|drag| drag.route == route);
                                let row = ui.allocate_ui_with_layout(
                                    egui::vec2(width - inset * 2.0, row_height),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        let knob = route_depth_knob(ui, state, route, color);
                                        let amount = route_amount(state, route);
                                        ui.label(
                                            egui::RichText::new(target_label(target))
                                                .font(editor_theme::font::label())
                                                .color(if active {
                                                    color
                                                } else {
                                                    editor_theme::semantic().text
                                                }),
                                        );
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                ui.label(
                                                    egui::RichText::new(format!(
                                                        "{:+.0}%",
                                                        amount * 100.0
                                                    ))
                                                    .font(editor_theme::font::value())
                                                    .color(if active {
                                                        color
                                                    } else {
                                                        editor_theme::semantic().text_muted
                                                    }),
                                                );
                                            },
                                        );
                                        knob
                                    },
                                );
                                let hovered =
                                    row.response.contains_pointer() || row.inner.hovered();
                                row.response.context_menu(|ui| {
                                    ui.spacing_mut().item_spacing.y = editor_theme::compact_gap(ui);
                                    if ui.button("Remove route").clicked() {
                                        clear_route(state, route);
                                        ui.close();
                                    }
                                });
                                if hovered || active {
                                    hovered_link =
                                        Some((row.response.rect.center(), target, route));
                                }
                                if hovered || active {
                                    ui.painter().rect_stroke(
                                        row.response.rect.shrink(editor_theme::shape::STROKE * 0.5),
                                        editor_theme::shape::CONTROL_RADIUS,
                                        egui::Stroke::new(
                                            if active {
                                                editor_theme::shape::FOCUS_STROKE
                                            } else {
                                                editor_theme::shape::STROKE
                                            },
                                            color.gamma_multiply(if active { 0.74 } else { 0.34 }),
                                        ),
                                        egui::StrokeKind::Inside,
                                    );
                                }
                            }
                        });
                });
        });
    let rect = output.response.rect;
    ui.data_mut(|data| {
        let mut direct = data
            .get_temp::<DirectModulationState>(id)
            .unwrap_or_default();
        direct.inspector_rect = rect;
        data.insert_temp(id, direct);
    });

    if let Some((start, target, route)) = hovered_link {
        let direct = ui
            .data(|data| data.get_temp::<DirectModulationState>(id))
            .unwrap_or_default();
        let destination = destination_rect(&direct, target);
        if destination.is_positive() {
            let end = if direct.route_handle_mask & (1_u64 << route) != 0 {
                direct.route_handle_positions[route]
            } else {
                destination.center()
            };
            let horizontal_span = (end.x - start.x).abs().max(title_height) * 0.35;
            let horizontal_direction = if end.x >= start.x { 1.0 } else { -1.0 };
            let control_a = start + egui::vec2(horizontal_direction * horizontal_span, 0.0);
            let control_b = end - egui::vec2(horizontal_direction * horizontal_span, 0.0);
            let path = cubic_bezier_points(start, control_a, control_b, end, 24);
            ui.painter().add(egui::Shape::dashed_line(
                &path,
                egui::Stroke::new(editor_theme::shape::FOCUS_STROKE, color),
                editor_theme::space::XS,
                editor_theme::space::XXS,
            ));
            brighten_control(ui, destination, color, 30);
        }
    }
}

fn update_drop_targets(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    direct: &mut DirectModulationState,
) {
    direct.hovered_target = None;
    direct.hovered_target_valid = false;
    direct.hovered_rect = egui::Rect::NOTHING;
    if direct.target_rect_frame != ui.ctx().cumulative_frame_nr() {
        return;
    }
    let Some(source) = direct.dragging_source else {
        return;
    };
    let pointer = ui.input(|input| input.pointer.latest_pos());
    let color = modulation_source_color(source);
    let painter = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("kurv-modulation-targets"),
    ));
    let mut hovered = None;
    if let Some(pointer) = pointer {
        for (index, rect) in direct.target_rects.iter().copied().enumerate() {
            if !rect.is_positive() || !rect.contains(pointer) {
                continue;
            }
            let target = index as u8 + 1;
            let valid = route_for_assignment(state, source, target).is_some();
            let area = rect.width() * rect.height();
            if hovered.is_none_or(|(_, _, _, hovered_area)| area < hovered_area) {
                hovered = Some((UiDestination::Host(target), rect, valid, area));
            }
        }
        for entry in direct.modular_target_rects[..direct.modular_target_len]
            .iter()
            .copied()
        {
            let Some(target) = entry.target else {
                continue;
            };
            if !entry.rect.contains(pointer) {
                continue;
            }
            let valid = route_for_modular_assignment(state, source, target).is_some();
            let area = entry.rect.width() * entry.rect.height();
            if hovered.is_none_or(|(_, _, _, hovered_area)| area < hovered_area) {
                hovered = Some((UiDestination::Modular(target), entry.rect, valid, area));
            }
        }
    }
    if let Some((target, rect, valid, _)) = hovered {
        direct.hovered_target = Some(target);
        direct.hovered_target_valid = valid;
        direct.hovered_rect = rect;
        ui.ctx().set_cursor_icon(if valid {
            egui::CursorIcon::Grabbing
        } else {
            egui::CursorIcon::NotAllowed
        });
    }
    for (index, rect) in direct.target_rects.iter().copied().enumerate() {
        let target = index as u8 + 1;
        if !rect.is_positive() {
            continue;
        }
        let valid = route_for_assignment(state, source, target).is_some();
        paint_drop_target(
            &painter,
            rect,
            color,
            direct.hovered_target == Some(UiDestination::Host(target)),
            valid,
        );
    }
    for entry in direct.modular_target_rects[..direct.modular_target_len]
        .iter()
        .copied()
    {
        let Some(target) = entry.target else {
            continue;
        };
        let valid = route_for_modular_assignment(state, source, target).is_some();
        paint_drop_target(
            &painter,
            entry.rect,
            color,
            direct.hovered_target == Some(UiDestination::Modular(target)),
            valid,
        );
    }
}

fn register_route_handle_widgets(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    direct: DirectModulationState,
) {
    let unit = modulation_unit(ui);
    for route in 0..ROUTE_COUNT {
        if direct.route_handle_mask & (1_u64 << route) == 0 {
            continue;
        }
        let response = ui
            .interact(
                route_handle_hit_rect(direct.route_handle_positions[route], unit),
                route_handle_id(route),
                egui::Sense::click_and_drag(),
            )
            .on_hover_text(format!(
                "{} · {:+.0}% depth · drag to adjust · double-click to clear",
                route_destination(state, route)
                    .map(target_label)
                    .unwrap_or_else(|| "DESTINATION".to_owned()),
                route_amount(state, route) * 100.0
            ));
        if response.hovered() || direct.amount_drag.is_some_and(|drag| drag.route == route) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
    }
}

fn clear_inspector_rect(ui: &egui::Ui, id: egui::Id) {
    ui.data_mut(|data| {
        let mut direct = data
            .get_temp::<DirectModulationState>(id)
            .unwrap_or_default();
        direct.inspector_rect = egui::Rect::NOTHING;
        data.insert_temp(id, direct);
    });
}

fn route_depth_knob(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    route: usize,
    color: egui::Color32,
) -> egui::Response {
    let id = egui::Id::new(UI_STATE_ID);
    let unit = modulation_unit(ui);
    let side = editor_theme::title_height(ui) * 0.82;
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click_and_drag());
    let response = response
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
        .on_hover_text("Drag horizontally or vertically to set depth; double-click clears");
    let mut direct = ui
        .data(|data| data.get_temp::<DirectModulationState>(id))
        .unwrap_or_default();
    if response.double_clicked() {
        finish_amount_drag(state, &mut direct, true);
        clear_route(state, route);
    } else if response.drag_started() {
        finish_amount_drag(state, &mut direct, false);
        let amount = route_amount(state, route);
        begin_route_amount_edit(state, route);
        direct.amount_drag = Some(AmountDrag {
            route,
            amount,
            initial_amount: amount,
        });
    }
    if direct.amount_drag.is_some_and(|drag| drag.route == route) {
        if response.dragged() {
            let drag = direct
                .amount_drag
                .as_mut()
                .expect("route drag checked above");
            update_route_amount(state, &response, drag);
            editor_theme::request_display_repaint(ui);
        }
        if response.drag_stopped() {
            finish_amount_drag(state, &mut direct, false);
        }
    }
    ui.data_mut(|data| data.insert_temp(id, direct));
    let amount = route_amount(state, route);
    paint_modulation_knob(
        ui.painter(),
        rect.center(),
        color,
        amount,
        unit,
        response.hovered(),
        response.dragged() || direct.amount_drag.is_some_and(|drag| drag.route == route),
    );
    response
}

fn routes_for_source(
    state: &PluginContext<KurvParams>,
    source: ResolvedRouteSource,
) -> RouteBucket {
    let mut bucket = RouteBucket::default();
    for index in 0..ROUTE_COUNT {
        if route_source(state, index) != Some(source) || bucket.len == bucket.entries.len() {
            continue;
        }
        if route_destination(state, index).is_none() {
            continue;
        }
        bucket.entries[bucket.len] = (
            index,
            source,
            route_amount(state, index),
            source_is_bipolar(state, source),
        );
        bucket.len += 1;
    }
    bucket
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

fn host_route_source(state: &PluginContext<KurvParams>, param: P) -> u8 {
    discrete_value(state.get_param(param), MAX_MODULATION_SOURCES as u8)
}

fn route_source(state: &PluginContext<KurvParams>, route: usize) -> Option<ResolvedRouteSource> {
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

fn route_amount(state: &PluginContext<KurvParams>, route: usize) -> f32 {
    if route < HOST_ROUTE_COUNT {
        state.get_param(ROUTES[route].2).mul_add(2.0, -1.0)
    } else {
        state.params().modulation_route_overflow.get(route).amount
    }
}

fn set_route_amount(state: &PluginContext<KurvParams>, route: usize, amount: f32) {
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

fn begin_route_amount_edit(state: &PluginContext<KurvParams>, route: usize) {
    if route < HOST_ROUTE_COUNT {
        state.begin_edit(ROUTES[route].2);
    }
}

fn end_route_amount_edit(state: &PluginContext<KurvParams>, route: usize) {
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

fn route_destination(state: &PluginContext<KurvParams>, route: usize) -> Option<UiDestination> {
    if let Some(target) = state.params().modulation_route_targets.get(route) {
        return Some(UiDestination::Modular(target));
    }
    let target = route_target(state, route);
    (target != 0).then_some(UiDestination::Host(target))
}

fn destination_rect(direct: &DirectModulationState, target: UiDestination) -> egui::Rect {
    match target {
        UiDestination::Host(target) => direct.target_rects[usize::from(target.saturating_sub(1))],
        UiDestination::Modular(target) => direct.modular_target_rects[..direct.modular_target_len]
            .iter()
            .find(|entry| entry.target == Some(target))
            .map_or(egui::Rect::NOTHING, |entry| entry.rect),
    }
}

fn modular_target_color_index(target: ModulationRouteTarget) -> usize {
    match target {
        ModulationRouteTarget::Oscillator { slot, .. } => slot.index(),
        ModulationRouteTarget::Group { group_id, .. } => group_id as usize,
    }
}

fn clear_route(state: &PluginContext<KurvParams>, route: usize) {
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

fn target_for_param(param: P) -> Option<u8> {
    modulation_target::target_for_param(param)
}

fn display_span(target: u8) -> f32 {
    modulation_target::descriptor(target).map_or(1.0, |target| target.normalized_span)
}

pub(crate) fn effective_normalized(state: &PluginContext<KurvParams>, param: P) -> f32 {
    let Some(target) = target_for_param(param) else {
        return state.get_param(param);
    };
    let mut value = state.get_param(param);
    for (index, (_, _, amount, _)) in ROUTES.iter().enumerate() {
        if state.params().modulation_route_targets.get(index).is_some()
            || route_target(state, index) != target
        {
            continue;
        }
        let source = route_source(state, index);
        let Some(source) = source else {
            continue;
        };
        let source_value = lfo_value_meter(state, source);
        let amount = state.get_param(*amount).mul_add(2.0, -1.0);
        value += source_value * amount * display_span(target);
    }
    value.clamp(0.0, 1.0)
}

fn lfo_value_meter(state: &PluginContext<KurvParams>, source: ResolvedRouteSource) -> f32 {
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

fn paint_live_value(
    ui: &egui::Ui,
    track: egui::Rect,
    axis: TrackAxis,
    value: f32,
    color: egui::Color32,
) {
    let point = match axis {
        TrackAxis::Horizontal => egui::pos2(
            egui::lerp(track.left()..=track.right(), value),
            track.center().y,
        ),
        TrackAxis::Vertical => egui::pos2(
            track.center().x,
            egui::lerp(track.bottom()..=track.top(), value),
        ),
    };
    let radius = modulation_unit(ui) * 0.12;
    ui.painter().circle_filled(point, radius, color);
    ui.painter().circle_stroke(
        point,
        radius + editor_theme::space::XXS,
        egui::Stroke::new(editor_theme::shape::STROKE, color.gamma_multiply(0.75)),
    );
}
