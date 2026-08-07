//! Direct-manipulation modulation routing for the editor.
//!
//! The audio engine still consumes the fixed, host-automatable route bank. This
//! module only gives that bank a source-drag/destination-overlay interface.

use truce::params::Params;
use truce_core::editor::{PluginContext, PluginContextReadF32};

use crate::editor_theme;
use crate::modulation_target;
use crate::{KurvParams, P};

const UI_STATE_ID: &str = "kurv-direct-modulation";
const ROUTE_CACHE_ID: &str = "kurv-direct-modulation-routes";
const MODULATION_KNOB_RADIUS: f32 = 7.0;
const MODULATION_HANDLE_HIT_RADIUS: f32 = 9.0;
const MODULATION_HANDLE_OUTSET: f32 = MODULATION_KNOB_RADIUS + 2.0;
const MODULATION_HANDLE_LANE_SPACING: f32 = 12.0;
const TARGET_COUNT: usize = modulation_target::TARGETS.len();
const TARGET_COUNT_U8: u8 = TARGET_COUNT as u8;
const ROUTE_COUNT: usize = 16;

type UiRoute = (usize, u8, f32, bool);

#[derive(Clone, Copy)]
struct RouteBucket {
    entries: [UiRoute; 16],
    len: usize,
}

impl Default for RouteBucket {
    fn default() -> Self {
        Self {
            entries: [(0, 0, 0.0, false); 16],
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

const ROUTES: [(P, P, P, P); ROUTE_COUNT] = [
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
    dragging_source: u8,
    hovered_source: u8,
    source_rect: egui::Rect,
    hovered_target: u8,
    hovered_rect: egui::Rect,
    inspector_rect: egui::Rect,
    target_rects: [egui::Rect; TARGET_COUNT],
    route_handle_positions: [egui::Pos2; ROUTE_COUNT],
    route_handle_mask: u16,
    target_rect_frame: u64,
    amount_drag: Option<AmountDrag>,
}

impl Default for DirectModulationState {
    fn default() -> Self {
        Self {
            dragging_source: 0,
            hovered_source: 0,
            source_rect: egui::Rect::NOTHING,
            hovered_target: 0,
            hovered_rect: egui::Rect::NOTHING,
            inspector_rect: egui::Rect::NOTHING,
            target_rects: [egui::Rect::NOTHING; TARGET_COUNT],
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
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrackAxis {
    Horizontal,
    Vertical,
}

pub(crate) const fn source_color(index: usize) -> egui::Color32 {
    match index {
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

pub(crate) fn source_handle(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    index: usize,
    response: &egui::Response,
) -> egui::Response {
    let source = (index + 1) as u8;
    let color = source_color(index);
    let active = response.dragged() || response.drag_started();
    if active {
        ui.painter().rect_stroke(
            response.rect.shrink(1.0),
            3.0,
            egui::Stroke::new(1.5_f32, color),
            egui::StrokeKind::Inside,
        );
    } else if response.hovered() {
        let origin = response.rect.left_center() + egui::vec2(7.0, -2.5);
        for column in 0..2 {
            for row in 0..3 {
                ui.painter().circle_filled(
                    origin + egui::vec2(column as f32 * 3.5, row as f32 * 3.0),
                    1.0,
                    color,
                );
            }
        }
    }

    let id = egui::Id::new(UI_STATE_ID);
    let pointer = ui.input(|input| input.pointer.latest_pos());
    ui.data_mut(|data| {
        let mut direct = data
            .get_temp::<DirectModulationState>(id)
            .unwrap_or_default();
        if response.hovered() {
            direct.hovered_source = source;
            direct.source_rect = response.rect;
        } else if direct.hovered_source == source
            && direct.dragging_source == 0
            && direct.amount_drag.is_none()
            && !pointer.is_some_and(|pointer| {
                response.rect.contains(pointer) || direct.inspector_rect.contains(pointer)
            })
        {
            direct.hovered_source = 0;
            direct.source_rect = egui::Rect::NOTHING;
        }
        if response.drag_started() {
            direct.dragging_source = source;
        }
        data.insert_temp(id, direct);
    });
    if response.drag_stopped() {
        let direct = ui
            .data(|data| data.get_temp::<DirectModulationState>(id))
            .unwrap_or_default();
        let pointer = ui.input(|input| input.pointer.latest_pos());
        if direct.dragging_source == source
            && direct.hovered_target != 0
            && pointer.is_some_and(|pointer| direct.hovered_rect.contains(pointer))
        {
            assign_route(state, source, direct.hovered_target);
        }
        ui.data_mut(|data| {
            let mut direct = data
                .get_temp::<DirectModulationState>(id)
                .unwrap_or_default();
            direct.dragging_source = 0;
            direct.hovered_target = 0;
            data.insert_temp(id, direct);
        });
    }
    if active {
        if let Some(pointer) = ui.input(|input| input.pointer.latest_pos()) {
            ui.ctx()
                .layer_painter(egui::LayerId::new(
                    egui::Order::Foreground,
                    egui::Id::new("kurv-modulation-cable"),
                ))
                .circle_filled(pointer, 6.0, color);
        }
        ui.ctx().request_repaint();
    }
    response
        .clone()
        .on_hover_cursor(if active {
            egui::CursorIcon::Grabbing
        } else {
            egui::CursorIcon::Grab
        })
        .on_hover_text(format!(
            "Click to edit; drag LFO {} onto a parameter",
            index + 1
        ))
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
    ui.data_mut(|data| {
        let mut direct = data
            .get_temp::<DirectModulationState>(id)
            .unwrap_or_default();
        if direct.target_rect_frame != frame {
            direct.target_rects = [egui::Rect::NOTHING; TARGET_COUNT];
            direct.route_handle_positions = [egui::Pos2::ZERO; ROUTE_COUNT];
            direct.route_handle_mask = 0;
            direct.target_rect_frame = frame;
        }
        direct.target_rects[usize::from(target - 1)] = response.rect;
        data.insert_temp(id, direct);
    });
    let direct = ui
        .data(|data| data.get_temp::<DirectModulationState>(id))
        .unwrap_or_default();
    if response.contains_pointer() {
        ui.data_mut(|data| {
            let mut direct = data
                .get_temp::<DirectModulationState>(id)
                .unwrap_or_default();
            direct.hovered_target = target;
            direct.hovered_rect = response.rect;
            data.insert_temp(id, direct);
        });
    } else if direct.hovered_target == target && direct.dragging_source == 0 {
        ui.data_mut(|data| {
            let mut direct = data
                .get_temp::<DirectModulationState>(id)
                .unwrap_or_default();
            direct.hovered_target = 0;
            direct.hovered_rect = egui::Rect::NOTHING;
            data.insert_temp(id, direct);
        });
    }

    let routes = routes_for_target(ui, state, target);
    let live_base = effective_normalized(state, param);
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
                route_handle_position(track, lane, routes.len, *amount, clip_rect);
            direct.route_handle_mask |= 1_u16 << *route;
        }
        data.insert_temp(id, direct);
    });
    let direct = ui
        .data(|data| data.get_temp::<DirectModulationState>(id))
        .unwrap_or_default();
    let span = display_span(target);
    let source_highlight = direct.hovered_source != 0
        && routes
            .as_slice()
            .iter()
            .any(|(_, source, _, _)| *source == direct.hovered_source);
    let pointer = ui.input(|input| input.pointer.latest_pos());
    let hovered_route =
        pointer.and_then(|pointer| route_handle_hit(pointer, track, routes.as_slice(), clip_rect));
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
        show_handles,
        clip_rect,
    );
    if source_highlight {
        brighten_control(
            ui,
            response.rect,
            source_color(usize::from(direct.hovered_source.saturating_sub(1))),
            22,
        );
    }
    if !routes.as_slice().is_empty() {
        paint_live_value(
            ui,
            track,
            axis,
            live_base,
            source_color(usize::from(target) % 8),
        );
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(16));
    }
    if direct.dragging_source != 0 && response.contains_pointer() {
        brighten_control(
            ui,
            response.rect,
            source_color(usize::from(direct.dragging_source.saturating_sub(1))),
            42,
        );
    }

    if hovered_route.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }

    false
}

pub(crate) fn destination_xy(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    x_param: P,
    y_param: P,
    response: &egui::Response,
    track: egui::Rect,
) {
    destination(
        ui,
        state,
        x_param,
        response,
        state.get_param(x_param),
        track,
        TrackAxis::Horizontal,
    );
    destination(
        ui,
        state,
        y_param,
        response,
        state.get_param(y_param),
        track,
        TrackAxis::Vertical,
    );
    let pointer = ui.input(|input| input.pointer.latest_pos());
    let Some(pointer) = pointer.filter(|pointer| response.rect.contains(*pointer)) else {
        return;
    };
    let vertical_edge_distance =
        (pointer.x - response.rect.left()).min(response.rect.right() - pointer.x);
    let horizontal_edge_distance =
        (pointer.y - response.rect.top()).min(response.rect.bottom() - pointer.y);
    let target = if vertical_edge_distance <= horizontal_edge_distance {
        target_for_param(x_param)
    } else {
        target_for_param(y_param)
    };
    let Some(target) = target else {
        return;
    };
    let id = egui::Id::new(UI_STATE_ID);
    ui.data_mut(|data| {
        let mut direct = data
            .get_temp::<DirectModulationState>(id)
            .unwrap_or_default();
        if direct.dragging_source != 0 {
            direct.hovered_target = target;
            direct.hovered_rect = response.rect;
        }
        data.insert_temp(id, direct);
    });
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
    let id = egui::Id::new(UI_STATE_ID);
    let mut direct = ui
        .data(|data| data.get_temp::<DirectModulationState>(id))
        .unwrap_or_default();
    let routes = routes_for_target(ui, state, target);

    // The real widget is registered by the final overlay pass, after all base
    // controls. Read that response here so the base parameter never steals the
    // handle gesture.
    for (route, _, _, _) in routes.as_slice() {
        let amount = state.get_param(ROUTES[*route].2).mul_add(2.0, -1.0);
        let Some(handle_response) = ui.ctx().read_response(route_handle_id(*route)) else {
            continue;
        };
        if handle_response.hovered() || direct.amount_drag.is_some_and(|drag| drag.route == *route)
        {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        if handle_response.double_clicked() {
            clear_route(state, *route);
            return true;
        }
        if handle_response.drag_started() {
            state.begin_edit(ROUTES[*route].2);
            direct.amount_drag = Some(AmountDrag {
                route: *route,
                amount,
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
                ui.ctx().request_repaint();
                return true;
            }
            if handle_response.drag_stopped() {
                state.end_edit(ROUTES[*route].2);
                let amount = state.get_param(ROUTES[*route].2).mul_add(2.0, -1.0);
                direct.amount_drag = None;
                ui.data_mut(|data| data.insert_temp(id, direct));
                if amount.abs() <= 0.005 {
                    clear_route(state, *route);
                }
                return true;
            }
        }
    }

    // Keep the old parent-response path as a fallback for compact controls
    // whose clip rect cannot contain an external handle.
    let clip_rect = ui.ctx().content_rect();
    let hovered = ui
        .input(|input| input.pointer.latest_pos())
        .and_then(|pointer| route_handle_hit(pointer, response.rect, routes.as_slice(), clip_rect));
    if response.double_clicked()
        && let Some(route) = hovered
    {
        clear_route(state, route);
        return true;
    }
    if response.drag_started()
        && let Some(route) = hovered
    {
        state.begin_edit(ROUTES[route].2);
        direct.amount_drag = Some(AmountDrag {
            route,
            amount: state.get_param(ROUTES[route].2).mul_add(2.0, -1.0),
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
            ui.ctx().request_repaint();
            return true;
        }
        if response.drag_stopped() {
            state.end_edit(ROUTES[drag.route].2);
            let amount = state.get_param(ROUTES[drag.route].2).mul_add(2.0, -1.0);
            direct.amount_drag = None;
            ui.data_mut(|data| data.insert_temp(id, direct));
            if amount.abs() <= 0.005 {
                clear_route(state, drag.route);
            }
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
) -> Option<usize> {
    routes
        .iter()
        .enumerate()
        .filter_map(|(lane, (route, _, amount, _))| {
            let handle = route_handle_position(track, lane, routes.len(), *amount, clip_rect);
            (pointer.distance(handle) <= MODULATION_HANDLE_HIT_RADIUS)
                .then_some((*route, pointer.distance(handle)))
        })
        .min_by(|left, right| left.1.total_cmp(&right.1))
        .map(|(route, _)| route)
}

fn route_handle_hit_rect(center: egui::Pos2) -> egui::Rect {
    egui::Rect::from_center_size(
        center,
        egui::vec2(
            MODULATION_HANDLE_HIT_RADIUS * 2.0,
            MODULATION_HANDLE_HIT_RADIUS * 2.0,
        ),
    )
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
    state.set_param(
        ROUTES[drag.route].2,
        f64::from(drag.amount.mul_add(0.5, 0.5)),
    );
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
        for (index, (source, _, amount, _)) in ROUTES.iter().enumerate() {
            let source = route_source(state, *source);
            let destination = route_target(state, index);
            if source == 0 || destination == 0 || destination > TARGET_COUNT_U8 {
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

fn paint_routes(
    ui: &egui::Ui,
    track: egui::Rect,
    axis: TrackAxis,
    base: f32,
    span: f32,
    routes: &[UiRoute],
    hovered_source: u8,
    hovered_route: Option<usize>,
    show_handles: bool,
    clip_rect: egui::Rect,
) {
    for (lane, (route, source, amount, bipolar)) in routes.iter().enumerate() {
        let (start_value, end_value) = route_range(base, span, *amount, *bipolar);
        let offset = lane as f32 * 1.5;
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
        let color = source_color(usize::from(source.saturating_sub(1)));
        let stroke = if *source == hovered_source {
            egui::Stroke::new(2.5_f32, color)
        } else {
            egui::Stroke::new(1.25_f32, color)
        };
        ui.painter().line_segment([start, finish], stroke);
        if show_handles {
            let handle = route_handle_position(track, lane, routes.len(), *amount, clip_rect);
            let hovered = hovered_route == Some(*route);
            let painter = ui.ctx().layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("kurv-modulation-knobs"),
            ));
            paint_modulation_knob(&painter, handle, color, *amount, hovered);
        }
    }
}

fn route_handle_position(
    track: egui::Rect,
    lane: usize,
    route_count: usize,
    amount: f32,
    clip_rect: egui::Rect,
) -> egui::Pos2 {
    let lane_center = route_count.saturating_sub(1) as f32 * 0.5;
    let y = track.center().y + (lane as f32 - lane_center) * MODULATION_HANDLE_LANE_SPACING;
    let x = if amount >= 0.0 {
        track.right() + MODULATION_HANDLE_OUTSET
    } else {
        track.left() - MODULATION_HANDLE_OUTSET
    };
    let outside = egui::pos2(x, y);
    if clip_rect.is_positive() && clip_rect.contains_rect(route_handle_hit_rect(outside)) {
        outside
    } else {
        egui::pos2(
            inset_clamp(
                if amount >= 0.0 {
                    track.right() - MODULATION_HANDLE_HIT_RADIUS
                } else {
                    track.left() + MODULATION_HANDLE_HIT_RADIUS
                },
                clip_rect.left(),
                clip_rect.right(),
                MODULATION_HANDLE_HIT_RADIUS,
            ),
            inset_clamp(
                y,
                clip_rect.top(),
                clip_rect.bottom(),
                MODULATION_HANDLE_HIT_RADIUS,
            ),
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

fn paint_modulation_knob(
    painter: &egui::Painter,
    center: egui::Pos2,
    color: egui::Color32,
    amount: f32,
    hovered: bool,
) {
    const START: f32 = std::f32::consts::FRAC_PI_2 * 1.5;
    const SWEEP: f32 = std::f32::consts::TAU * 0.75;
    let radius = if hovered {
        MODULATION_KNOB_RADIUS + 1.0
    } else {
        MODULATION_KNOB_RADIUS
    };
    let depth = amount.abs().clamp(0.0, 1.0);
    painter.circle_filled(center, radius, editor_theme::semantic().well);
    painter.circle_stroke(
        center,
        radius - 0.5,
        egui::Stroke::new(1.0_f32, editor_theme::semantic().grid),
    );
    painter.add(egui::Shape::line(
        modulation_arc_points(center, radius - 1.75, START, SWEEP, 24),
        egui::Stroke::new(1.0_f32, editor_theme::semantic().control_hover),
    ));
    if depth > f32::EPSILON {
        let arc_start = if amount < 0.0 { START + SWEEP } else { START };
        let arc_sweep = if amount < 0.0 {
            -SWEEP * depth
        } else {
            SWEEP * depth
        };
        painter.add(egui::Shape::line(
            modulation_arc_points(center, radius - 1.75, arc_start, arc_sweep, 24),
            egui::Stroke::new(if hovered { 2.0_f32 } else { 1.5_f32 }, color),
        ));
    }
}

fn brighten_control(ui: &egui::Ui, rect: egui::Rect, color: egui::Color32, alpha: u8) {
    let [red, green, blue, _] = color.to_array();
    ui.painter().rect_filled(
        rect,
        2.0,
        egui::Color32::from_rgba_unmultiplied(red, green, blue, alpha),
    );
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

fn source_is_bipolar(state: &PluginContext<KurvParams>, source: u8) -> bool {
    let param = match source {
        1 => P::Lfo1Bipolar,
        2 => P::Lfo2Bipolar,
        3 => P::Lfo3Bipolar,
        4 => P::Lfo4Bipolar,
        5 => P::Lfo5Bipolar,
        6 => P::Lfo6Bipolar,
        7 => P::Lfo7Bipolar,
        8 => P::Lfo8Bipolar,
        _ => return false,
    };
    state.get_param(param) >= 0.5
}

fn assign_route(state: &PluginContext<KurvParams>, source: u8, target: u8) {
    let exact = (0..16).find(|&index| {
        let (src, _, _, _) = ROUTES[index];
        route_source(state, src) == source && route_target(state, index) == target
    });
    let vacant = (0..16).find(|&index| {
        let (src, _, _, _) = ROUTES[index];
        route_source(state, src) == 0 || route_target(state, index) == 0
    });
    let Some(route) = exact.or(vacant) else {
        crate::diagnostics::trace(
            "modulation-route",
            "bank-full",
            source.into(),
            target.into(),
        );
        return;
    };
    let (source_param, target_param, amount_param, ext_param) = ROUTES[route];
    if exact.is_none() {
        state.automate(amount_param, 0.5);
    }
    state.automate(source_param, f64::from(source) / 8.0);
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
    if exact.is_none() {
        state.automate(amount_param, 0.625);
    }
}

pub(crate) fn used_source_mask(state: &PluginContext<KurvParams>) -> u8 {
    ROUTES
        .iter()
        .enumerate()
        .fold(0, |mask, (index, (source, _, amount, _))| {
            let source = route_source(state, *source);
            if source != 0
                && route_target(state, index) != 0
                && (state.get_param(*amount) - 0.5).abs() > f32::EPSILON
            {
                mask | (1 << (source - 1))
            } else {
                mask
            }
        })
}

pub(crate) fn clear_source(state: &PluginContext<KurvParams>, source: u8) {
    for (index, (source_param, _, _, _)) in ROUTES.iter().enumerate() {
        if route_source(state, *source_param) == source {
            clear_route(state, index);
        }
    }
}

/// Paints the source-hover route editor after every destination has registered
/// its current frame geometry. Destination controls keep their own base-value
/// hit testing; this final pass owns the modulation handles and popup.
pub(crate) fn draw_overlay(ui: &mut egui::Ui, state: &PluginContext<KurvParams>) {
    let id = egui::Id::new(UI_STATE_ID);
    let direct = ui
        .data(|data| data.get_temp::<DirectModulationState>(id))
        .unwrap_or_default();
    register_route_handle_widgets(ui, direct);
    if direct.dragging_source != 0 || direct.hovered_source == 0 {
        clear_inspector_rect(ui, id);
        return;
    }
    let source = direct.hovered_source;
    let routes = routes_for_source(state, source);
    if routes.len == 0 {
        clear_inspector_rect(ui, id);
        return;
    }
    let pointer = ui.input(|input| input.pointer.latest_pos());
    let dragging_source_route = direct
        .amount_drag
        .is_some_and(|drag| route_source(state, ROUTES[drag.route].0) == source);
    if !dragging_source_route
        && !pointer.is_some_and(|pointer| {
            direct.source_rect.contains(pointer) || direct.inspector_rect.contains(pointer)
        })
    {
        ui.data_mut(|data| {
            let mut direct = data
                .get_temp::<DirectModulationState>(id)
                .unwrap_or_default();
            direct.hovered_source = 0;
            direct.source_rect = egui::Rect::NOTHING;
            direct.inspector_rect = egui::Rect::NOTHING;
            data.insert_temp(id, direct);
        });
        return;
    }

    let width = 232.0;
    let height = 30.0 + routes.len as f32 * 30.0;
    let mut popup_rect =
        egui::Rect::from_min_size(direct.source_rect.left_bottom(), egui::vec2(width, height));
    let screen = ui.ctx().content_rect().shrink(4.0);
    if popup_rect.right() > screen.right() {
        popup_rect = popup_rect.translate(egui::vec2(screen.right() - popup_rect.right(), 0.0));
    }
    if popup_rect.bottom() > screen.bottom() {
        popup_rect = popup_rect.translate(egui::vec2(0.0, screen.bottom() - popup_rect.bottom()));
    }
    popup_rect = popup_rect.translate(egui::vec2(
        (screen.left() - popup_rect.left()).max(0.0),
        (screen.top() - popup_rect.top()).max(0.0),
    ));

    let mut hovered_link = None;
    let color = source_color(usize::from(source - 1));
    let output = egui::Area::new(egui::Id::new("kurv-source-routes"))
        .order(egui::Order::Foreground)
        .fixed_pos(popup_rect.min)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style())
                .fill(editor_theme::semantic().chrome)
                .stroke(egui::Stroke::new(1.0_f32, editor_theme::semantic().grid))
                .inner_margin(egui::Margin::same(7))
                .show(ui, |ui| {
                    ui.set_width(width - 14.0);
                    ui.label(
                        egui::RichText::new(format!("LFO {source} DESTINATIONS"))
                            .font(editor_theme::font::caption())
                            .color(color),
                    );
                    for &(route, _, amount, _) in routes.as_slice() {
                        let target = route_target(state, route);
                        let row = ui.horizontal(|ui| {
                            let knob = route_depth_knob(ui, state, route, color);
                            ui.label(
                                egui::RichText::new(target_label(target))
                                    .font(editor_theme::font::caption())
                                    .color(editor_theme::semantic().text),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        egui::RichText::new(format!("{:+.0}%", amount * 100.0))
                                            .font(editor_theme::font::caption())
                                            .color(editor_theme::semantic().text_muted),
                                    );
                                },
                            );
                            knob
                        });
                        if row.response.contains_pointer() || row.inner.hovered() {
                            hovered_link = Some((row.inner.rect.center(), target, route));
                        }
                    }
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
        let destination = direct.target_rects[usize::from(target.saturating_sub(1))];
        if destination.is_positive() {
            let end = if direct.route_handle_mask & (1_u16 << route) != 0 {
                direct.route_handle_positions[route]
            } else {
                destination.center()
            };
            let horizontal_span = (end.x - start.x).abs().max(24.0) * 0.35;
            let horizontal_direction = if end.x >= start.x { 1.0 } else { -1.0 };
            let control_a = start + egui::vec2(horizontal_direction * horizontal_span, 0.0);
            let control_b = end - egui::vec2(horizontal_direction * horizontal_span, 0.0);
            let path = cubic_bezier_points(start, control_a, control_b, end, 24);
            ui.painter().add(egui::Shape::dashed_line(
                &path,
                egui::Stroke::new(1.25_f32, color),
                5.0,
                4.0,
            ));
            brighten_control(ui, destination, color, 30);
        }
    }
}

fn register_route_handle_widgets(ui: &egui::Ui, direct: DirectModulationState) {
    for route in 0..ROUTE_COUNT {
        if direct.route_handle_mask & (1_u16 << route) == 0 {
            continue;
        }
        let response = ui.interact(
            route_handle_hit_rect(direct.route_handle_positions[route]),
            route_handle_id(route),
            egui::Sense::click_and_drag(),
        );
        if response.hovered() {
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
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::click_and_drag());
    let response = response
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
        .on_hover_text("Drag horizontally or vertically to set depth; double-click clears");
    let mut direct = ui
        .data(|data| data.get_temp::<DirectModulationState>(id))
        .unwrap_or_default();
    let mut clear_after_drag = false;
    if response.double_clicked() {
        clear_route(state, route);
        direct.amount_drag = None;
    } else if response.drag_started() {
        state.begin_edit(ROUTES[route].2);
        direct.amount_drag = Some(AmountDrag {
            route,
            amount: state.get_param(ROUTES[route].2).mul_add(2.0, -1.0),
        });
    }
    if direct.amount_drag.is_some_and(|drag| drag.route == route) {
        if response.dragged() {
            let drag = direct
                .amount_drag
                .as_mut()
                .expect("route drag checked above");
            update_route_amount(state, &response, drag);
            ui.ctx().request_repaint();
        }
        if response.drag_stopped() {
            state.end_edit(ROUTES[route].2);
            let amount = state.get_param(ROUTES[route].2).mul_add(2.0, -1.0);
            direct.amount_drag = None;
            if amount.abs() <= 0.005 {
                clear_after_drag = true;
            }
        }
    }
    ui.data_mut(|data| data.insert_temp(id, direct));
    if clear_after_drag {
        clear_route(state, route);
    }
    let amount = state.get_param(ROUTES[route].2).mul_add(2.0, -1.0);
    paint_modulation_knob(
        ui.painter(),
        rect.center(),
        color,
        amount,
        response.hovered(),
    );
    response
}

fn routes_for_source(state: &PluginContext<KurvParams>, source: u8) -> RouteBucket {
    let mut bucket = RouteBucket::default();
    for (index, (source_param, _, amount_param, _)) in ROUTES.iter().enumerate() {
        if route_source(state, *source_param) != source || bucket.len == bucket.entries.len() {
            continue;
        }
        let target = route_target(state, index);
        if target == 0 {
            continue;
        }
        bucket.entries[bucket.len] = (
            index,
            source,
            state.get_param(*amount_param).mul_add(2.0, -1.0),
            source_is_bipolar(state, source),
        );
        bucket.len += 1;
    }
    bucket
}

fn target_label(target: u8) -> &'static str {
    modulation_target::descriptor(target).map_or("DESTINATION", |target| target.label)
}

fn route_source(state: &PluginContext<KurvParams>, param: P) -> u8 {
    discrete_value(state.get_param(param), 8)
}

fn route_target(state: &PluginContext<KurvParams>, route: usize) -> u8 {
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

fn clear_route(state: &PluginContext<KurvParams>, route: usize) {
    let (source, target, amount, ext) = ROUTES[route];
    state.automate(amount, 0.5);
    state.automate(target, 0.0);
    state.automate(ext, 0.0);
    state.automate(source, 0.0);
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
        if route_target(state, index) != target {
            continue;
        }
        let source = route_source(state, ROUTES[index].0);
        let source_value = lfo_value_meter(state, source);
        let amount = state.get_param(*amount).mul_add(2.0, -1.0);
        value += source_value * amount * display_span(target);
    }
    value.clamp(0.0, 1.0)
}

pub(crate) fn effective_plain_value(state: &PluginContext<KurvParams>, param: P) -> f32 {
    let normalized = effective_normalized(state, param);
    state
        .params()
        .param_infos()
        .into_iter()
        .find(|info| info.id == u32::from(param))
        .map_or(normalized, |info| {
            info.range.denormalize(f64::from(normalized)) as f32
        })
}

fn lfo_value_meter(state: &PluginContext<KurvParams>, source: u8) -> f32 {
    let params = state.params();
    let meter = match source {
        1 => &params.lfo1_value_meter,
        2 => &params.lfo2_value_meter,
        3 => &params.lfo3_value_meter,
        4 => &params.lfo4_value_meter,
        5 => &params.lfo5_value_meter,
        6 => &params.lfo6_value_meter,
        7 => &params.lfo7_value_meter,
        8 => &params.lfo8_value_meter,
        _ => return 0.0,
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
    ui.painter().circle_filled(point, 3.0, color);
    ui.painter().circle_stroke(
        point,
        5.0,
        egui::Stroke::new(1.0_f32, color.gamma_multiply(0.75)),
    );
}
