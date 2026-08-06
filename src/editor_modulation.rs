//! Direct-manipulation modulation routing for the editor.
//!
//! The audio engine still consumes the fixed, host-automatable route bank. This
//! module only gives that bank a source-drag/destination-overlay interface.

use truce_core::editor::{PluginContext, PluginContextReadF32};

use crate::editor_theme;
use crate::{KurvParams, P};

const UI_STATE_ID: &str = "kurv-direct-modulation";
const ROUTE_CACHE_ID: &str = "kurv-direct-modulation-routes";
const MODULATION_KNOB_RADIUS: f32 = 7.0;
const TARGET_COUNT: usize = 21;
const TARGET_COUNT_U8: u8 = TARGET_COUNT as u8;

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

const ROUTES: [(P, P, P); 16] = [
    (P::Mod1Source, P::Mod1Target, P::Mod1Amount),
    (P::Mod2Source, P::Mod2Target, P::Mod2Amount),
    (P::Mod3Source, P::Mod3Target, P::Mod3Amount),
    (P::Mod4Source, P::Mod4Target, P::Mod4Amount),
    (P::Mod5Source, P::Mod5Target, P::Mod5Amount),
    (P::Mod6Source, P::Mod6Target, P::Mod6Amount),
    (P::Mod7Source, P::Mod7Target, P::Mod7Amount),
    (P::Mod8Source, P::Mod8Target, P::Mod8Amount),
    (P::Mod9Source, P::Mod9Target, P::Mod9Amount),
    (P::Mod10Source, P::Mod10Target, P::Mod10Amount),
    (P::Mod11Source, P::Mod11Target, P::Mod11Amount),
    (P::Mod12Source, P::Mod12Target, P::Mod12Amount),
    (P::Mod13Source, P::Mod13Target, P::Mod13Amount),
    (P::Mod14Source, P::Mod14Target, P::Mod14Amount),
    (P::Mod15Source, P::Mod15Target, P::Mod15Amount),
    (P::Mod16Source, P::Mod16Target, P::Mod16Amount),
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
            target_rect_frame: u64::MAX,
            amount_drag: None,
        }
    }
}

#[derive(Clone, Copy)]
struct AmountDrag {
    route: usize,
    start_amount: f32,
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

/// Registers a supported destination, edits route depth from its edge line, and
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
    ui.data_mut(|data| {
        let mut direct = data
            .get_temp::<DirectModulationState>(id)
            .unwrap_or_default();
        if direct.target_rect_frame != frame {
            direct.target_rects = [egui::Rect::NOTHING; TARGET_COUNT];
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
    let span = display_span(target);
    let source_highlight = direct.hovered_source != 0
        && routes
            .as_slice()
            .iter()
            .any(|(_, source, _, _)| *source == direct.hovered_source);
    let hovered_route = response
        .hover_pos()
        .and_then(|pointer| route_handle_hit(pointer, track, axis, routes.as_slice()));
    let show_handles = response.hovered()
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
        base,
        span,
        routes.as_slice(),
        direct.hovered_source,
        hovered_route,
        show_handles,
    );
    if source_highlight {
        brighten_control(
            ui,
            response.rect,
            source_color(usize::from(direct.hovered_source.saturating_sub(1))),
            22,
        );
    }
    if direct.dragging_source != 0 && response.contains_pointer() {
        brighten_control(
            ui,
            response.rect,
            source_color(usize::from(direct.dragging_source.saturating_sub(1))),
            42,
        );
    }

    if response
        .hover_pos()
        .and_then(|pointer| route_handle_hit(pointer, track, axis, routes.as_slice()))
        .is_some()
    {
        ui.ctx().set_cursor_icon(match axis {
            // Modulation depth is deliberately a horizontal knob gesture for
            // both portrait and landscape destinations.
            TrackAxis::Horizontal | TrackAxis::Vertical => egui::CursorIcon::ResizeHorizontal,
        });
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
    let id = egui::Id::new(UI_STATE_ID);
    let mut direct = ui
        .data(|data| data.get_temp::<DirectModulationState>(id))
        .unwrap_or_default();
    let routes = routes_for_target(ui, state, target);
    let axis = if response.rect.height() > response.rect.width() * 1.15 {
        TrackAxis::Vertical
    } else {
        TrackAxis::Horizontal
    };
    let hovered = response
        .hover_pos()
        .and_then(|pointer| route_handle_hit(pointer, response.rect, axis, routes.as_slice()));
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
            start_amount: state.get_param(ROUTES[route].2).mul_add(2.0, -1.0),
        });
        ui.data_mut(|data| data.insert_temp(id, direct));
        return true;
    }
    let Some(drag) = direct.amount_drag else {
        return false;
    };
    if response.dragged() {
        let delta = response.drag_delta().x / response.rect.width().max(1.0);
        let amount = (drag.start_amount + delta).clamp(-1.0, 1.0);
        state.set_param(ROUTES[drag.route].2, f64::from(amount.mul_add(0.5, 0.5)));
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
    false
}

fn route_handle_hit(
    pointer: egui::Pos2,
    track: egui::Rect,
    axis: TrackAxis,
    routes: &[UiRoute],
) -> Option<usize> {
    routes
        .iter()
        .enumerate()
        .filter_map(|(lane, (route, _, _, _))| {
            let handle = route_handle_position(track, axis, lane, routes.len());
            (pointer.distance(handle) <= MODULATION_KNOB_RADIUS + 3.0)
                .then_some((*route, pointer.distance(handle)))
        })
        .min_by(|left, right| left.1.total_cmp(&right.1))
        .map(|(route, _)| route)
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
        for (index, (source, destination, amount)) in ROUTES.iter().enumerate() {
            let source = route_source(state, *source);
            let destination = discrete_value(state.get_param(*destination), TARGET_COUNT_U8);
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
            let handle = route_handle_position(track, axis, lane, routes.len());
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
    axis: TrackAxis,
    lane: usize,
    route_count: usize,
) -> egui::Pos2 {
    let inset = MODULATION_KNOB_RADIUS + 3.0;
    let position = if route_count <= 1 {
        0.5
    } else {
        lane as f32 / route_count.saturating_sub(1) as f32
    };
    match axis {
        TrackAxis::Horizontal => egui::pos2(
            egui::lerp(
                (track.left() + inset)..=(track.right() - inset),
                1.0 - position,
            ),
            inset_clamp(track.center().y, track.top(), track.bottom(), inset),
        ),
        TrackAxis::Vertical => egui::pos2(
            inset_clamp(track.center().x, track.left(), track.right(), inset),
            egui::lerp((track.top() + inset)..=(track.bottom() - inset), position),
        ),
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
        let (src, dst, _) = ROUTES[index];
        route_source(state, src) == source
            && discrete_value(state.get_param(dst), TARGET_COUNT_U8) == target
    });
    let vacant = (0..16).find(|&index| {
        let (src, dst, _) = ROUTES[index];
        route_source(state, src) == 0 || discrete_value(state.get_param(dst), TARGET_COUNT_U8) == 0
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
    let (source_param, target_param, amount_param) = ROUTES[route];
    if exact.is_none() {
        state.automate(amount_param, 0.5);
    }
    state.automate(source_param, f64::from(source) / 8.0);
    state.automate(target_param, f64::from(target) / f64::from(TARGET_COUNT_U8));
    if exact.is_none() {
        state.automate(amount_param, 0.625);
    }
}

pub(crate) fn used_source_mask(state: &PluginContext<KurvParams>) -> u8 {
    ROUTES.iter().fold(0, |mask, (source, target, amount)| {
        let source = route_source(state, *source);
        if source != 0
            && discrete_value(state.get_param(*target), TARGET_COUNT_U8) != 0
            && (state.get_param(*amount) - 0.5).abs() > f32::EPSILON
        {
            mask | (1 << (source - 1))
        } else {
            mask
        }
    })
}

pub(crate) fn clear_source(state: &PluginContext<KurvParams>, source: u8) {
    for (index, (source_param, _, _)) in ROUTES.iter().enumerate() {
        if route_source(state, *source_param) == source {
            clear_route(state, index);
        }
    }
}

/// Paints the source-hover route editor after every destination has registered
/// its current frame geometry. Destination controls keep their own hit testing;
/// this foreground pass only owns its compact depth knobs.
pub(crate) fn draw_overlay(ui: &mut egui::Ui, state: &PluginContext<KurvParams>) {
    let id = egui::Id::new(UI_STATE_ID);
    let direct = ui
        .data(|data| data.get_temp::<DirectModulationState>(id))
        .unwrap_or_default();
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
                        let target =
                            discrete_value(state.get_param(ROUTES[route].1), TARGET_COUNT_U8);
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
                            hovered_link = Some((row.response.rect.center(), target));
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

    if let Some((start, target)) = hovered_link {
        let direct = ui
            .data(|data| data.get_temp::<DirectModulationState>(id))
            .unwrap_or_default();
        let destination = direct.target_rects[usize::from(target.saturating_sub(1))];
        if destination.is_positive() {
            let end = destination.center();
            let bend_x = (start.x + end.x) * 0.5;
            let path = [
                start,
                egui::pos2(bend_x, start.y),
                egui::pos2(bend_x, end.y),
                end,
            ];
            let painter = ui.ctx().layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("kurv-modulation-link"),
            ));
            painter.extend(egui::Shape::dashed_line(
                &path,
                egui::Stroke::new(1.25_f32, color),
                5.0,
                4.0,
            ));
            brighten_control(ui, destination, color, 30);
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
    let response = response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
    let mut direct = ui
        .data(|data| data.get_temp::<DirectModulationState>(id))
        .unwrap_or_default();
    if response.double_clicked() {
        clear_route(state, route);
    } else if response.drag_started() {
        state.begin_edit(ROUTES[route].2);
        direct.amount_drag = Some(AmountDrag {
            route,
            start_amount: state.get_param(ROUTES[route].2).mul_add(2.0, -1.0),
        });
        ui.data_mut(|data| data.insert_temp(id, direct));
    }
    if direct.amount_drag.is_some_and(|drag| drag.route == route) {
        if response.dragged() {
            let drag = direct.amount_drag.expect("route drag checked above");
            let amount = (drag.start_amount + response.drag_delta().x / 120.0).clamp(-1.0, 1.0);
            state.set_param(ROUTES[route].2, f64::from(amount.mul_add(0.5, 0.5)));
        }
        if response.drag_stopped() {
            state.end_edit(ROUTES[route].2);
            let amount = state.get_param(ROUTES[route].2).mul_add(2.0, -1.0);
            direct.amount_drag = None;
            ui.data_mut(|data| data.insert_temp(id, direct));
            if amount.abs() <= 0.005 {
                clear_route(state, route);
            }
        }
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
    for (index, (source_param, _, amount_param)) in ROUTES.iter().enumerate() {
        if route_source(state, *source_param) != source || bucket.len == bucket.entries.len() {
            continue;
        }
        let target = discrete_value(state.get_param(ROUTES[index].1), TARGET_COUNT_U8);
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
    match target {
        1 => "OSC 1 TRANSPOSE",
        2 => "OSC 1 SHAPE",
        3 => "OSC 1 PULSE",
        4 => "OSC 1 WARP",
        5 => "OSC 1 LEVEL",
        6 => "OSC 1 PAN",
        7 => "OSC 2 TRANSPOSE",
        8 => "OSC 2 SHAPE",
        9 => "OSC 2 PULSE",
        10 => "OSC 2 WARP",
        11 => "OSC 2 LEVEL",
        12 => "OSC 2 PAN",
        13 => "OSC 3 TRANSPOSE",
        14 => "OSC 3 SHAPE",
        15 => "OSC 3 PULSE",
        16 => "OSC 3 WARP",
        17 => "OSC 3 LEVEL",
        18 => "OSC 3 PAN",
        19 => "OSC 1 DETUNE AMOUNT",
        20 => "OSC 2 DETUNE AMOUNT",
        21 => "OSC 3 DETUNE AMOUNT",
        _ => "DESTINATION",
    }
}

fn route_source(state: &PluginContext<KurvParams>, param: P) -> u8 {
    discrete_value(state.get_param(param), 8)
}

fn clear_route(state: &PluginContext<KurvParams>, route: usize) {
    let (source, target, amount) = ROUTES[route];
    state.automate(amount, 0.5);
    state.automate(target, 0.0);
    state.automate(source, 0.0);
}

fn discrete_value(normalized: f32, maximum: u8) -> u8 {
    (normalized.clamp(0.0, 1.0) * f32::from(maximum)).round() as u8
}

const fn target_for_param(param: P) -> Option<u8> {
    match param {
        P::Osc1Transpose => Some(1),
        P::Shape => Some(2),
        P::PulseWidth => Some(3),
        P::Osc1WarpAmount => Some(4),
        P::Osc1Level => Some(5),
        P::Osc1Pan => Some(6),
        P::Osc2Transpose => Some(7),
        P::Osc2Shape => Some(8),
        P::Osc2PulseWidth => Some(9),
        P::Osc2WarpAmount => Some(10),
        P::Osc2Level => Some(11),
        P::Osc2Pan => Some(12),
        P::Osc3Transpose => Some(13),
        P::Osc3Shape => Some(14),
        P::Osc3PulseWidth => Some(15),
        P::Osc3WarpAmount => Some(16),
        P::Osc3Level => Some(17),
        P::Osc3Pan => Some(18),
        P::UnisonDetuneAmount => Some(19),
        P::Osc2UnisonDetuneAmount => Some(20),
        P::Osc3UnisonDetuneAmount => Some(21),
        _ => None,
    }
}

const fn display_span(target: u8) -> f32 {
    if target > 18 {
        return 1.0;
    }
    match (target - 1) % 6 {
        0 | 2 | 5 => 0.5,
        _ => 1.0,
    }
}
