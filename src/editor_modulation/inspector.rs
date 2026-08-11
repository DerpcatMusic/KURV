use truce_core::editor::PluginContext;

use crate::modulators::routing::ResolvedRouteSource;
use crate::{KurvParams, editor_theme};

use super::route_bank::{
    RouteBucket, UiRoute, begin_route_amount_edit, clear_route, end_route_amount_edit,
    route_amount, set_route_amount,
};
use super::{
    AmountDrag, DirectModulationState, TrackAxis, UI_STATE_ID, modulation_handle_hit_radius,
    modulation_handle_lane_spacing, modulation_knob_radius, modulation_source_color,
    modulation_unit, source_color,
};

#[allow(clippy::too_many_arguments)]
pub(super) fn paint_destination_routes(
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
    let (dragging_source, hovered_source, amount_drag) = ui.data_mut(|data| {
        let direct = data.get_temp_mut_or_default::<DirectModulationState>(id);
        (
            direct.dragging_source,
            direct.hovered_source,
            direct.amount_drag,
        )
    });
    if dragging_source.is_none() {
        ui.data_mut(|data| {
            let direct = data.get_temp_mut_or_default::<DirectModulationState>(id);
            for (lane, (route, _, amount, _)) in routes.as_slice().iter().enumerate() {
                direct.route_handle_positions[*route] =
                    route_handle_position(track, lane, routes.len(), *amount, clip_rect, unit);
                direct.route_handle_mask |= 1_u64 << *route;
            }
        });
    }
    let source_highlight = hovered_source.is_some()
        && routes
            .as_slice()
            .iter()
            .any(|(_, source, _, _)| Some(*source) == hovered_source);
    let pointer = ui.input(|input| input.pointer.latest_pos());
    let hovered_route = dragging_source
        .is_none()
        .then(|| pointer)
        .flatten()
        .and_then(|pointer| route_handle_hit(pointer, track, routes.as_slice(), clip_rect, unit));
    let show_handles = dragging_source.is_none()
        && (response.hovered()
            || hovered_route.is_some()
            || source_highlight
            || amount_drag.is_some_and(|drag| {
                routes
                    .as_slice()
                    .iter()
                    .any(|(route, _, _, _)| *route == drag.route)
            }));
    paint_routes(
        ui,
        track,
        axis,
        live_base,
        span,
        routes.as_slice(),
        hovered_source,
        hovered_route,
        amount_drag.map(|drag| drag.route),
        show_handles,
        clip_rect,
        unit,
    );
    if source_highlight {
        let source = hovered_source.expect("source highlight requires a hovered source");
        brighten_control(ui, response.rect, modulation_source_color(source), 22);
    }
    if !routes.as_slice().is_empty() {
        paint_live_value(ui, track, axis, live_base, source_color(color_index % 8));
        if dragging_source.is_none() {
            editor_theme::request_display_repaint(ui);
        }
    }
    if hovered_route.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }

    false
}

pub(super) fn owns_routes_gesture(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    response: &egui::Response,
    routes: &RouteBucket,
) -> bool {
    let id = egui::Id::new(UI_STATE_ID);
    // The real widget is registered by the final overlay pass, after all base
    // controls. Read that response here so the base parameter never steals the
    // handle gesture.
    for (route, _, _, _) in routes.as_slice() {
        let amount = route_amount(state, *route);
        let Some(handle_response) = ui.ctx().read_response(route_handle_id(*route)) else {
            continue;
        };
        let route_dragging = ui.data_mut(|data| {
            data.get_temp_mut_or_default::<DirectModulationState>(id)
                .amount_drag
                .is_some_and(|drag| drag.route == *route)
        });
        if handle_response.hovered() || route_dragging {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        if handle_response.double_clicked() {
            ui.data_mut(|data| {
                finish_amount_drag(
                    state,
                    data.get_temp_mut_or_default::<DirectModulationState>(id),
                    true,
                );
            });
            clear_route(state, *route);
            return true;
        }
        if handle_response.drag_started() {
            ui.data_mut(|data| {
                finish_amount_drag(
                    state,
                    data.get_temp_mut_or_default::<DirectModulationState>(id),
                    false,
                );
            });
            begin_route_amount_edit(state, *route);
            ui.data_mut(|data| {
                data.get_temp_mut_or_default::<DirectModulationState>(id)
                    .amount_drag = Some(AmountDrag {
                    route: *route,
                    amount,
                    initial_amount: amount,
                });
            });
        }
        let route_dragging = ui.data_mut(|data| {
            data.get_temp_mut_or_default::<DirectModulationState>(id)
                .amount_drag
                .is_some_and(|drag| drag.route == *route)
        });
        if route_dragging {
            if handle_response.dragged() {
                ui.data_mut(|data| {
                    let drag = data
                        .get_temp_mut_or_default::<DirectModulationState>(id)
                        .amount_drag
                        .as_mut()
                        .expect("route drag checked above");
                    update_route_amount(state, &handle_response, drag);
                });
                editor_theme::request_display_repaint(ui);
                return true;
            }
            if handle_response.drag_stopped() {
                ui.data_mut(|data| {
                    finish_amount_drag(
                        state,
                        data.get_temp_mut_or_default::<DirectModulationState>(id),
                        false,
                    );
                });
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
        ui.data_mut(|data| {
            finish_amount_drag(
                state,
                data.get_temp_mut_or_default::<DirectModulationState>(id),
                true,
            );
        });
        clear_route(state, route);
        return true;
    }
    if response.drag_started()
        && let Some(route) = hovered
    {
        ui.data_mut(|data| {
            finish_amount_drag(
                state,
                data.get_temp_mut_or_default::<DirectModulationState>(id),
                false,
            );
        });
        let amount = route_amount(state, route);
        begin_route_amount_edit(state, route);
        ui.data_mut(|data| {
            data.get_temp_mut_or_default::<DirectModulationState>(id)
                .amount_drag = Some(AmountDrag {
                route,
                amount,
                initial_amount: amount,
            });
        });
    }

    let amount_drag = ui.data_mut(|data| {
        data.get_temp_mut_or_default::<DirectModulationState>(id)
            .amount_drag
    });
    if let Some(drag) = amount_drag
        && routes
            .as_slice()
            .iter()
            .any(|(route, _, _, _)| *route == drag.route)
    {
        if response.dragged() {
            ui.data_mut(|data| {
                let drag = data
                    .get_temp_mut_or_default::<DirectModulationState>(id)
                    .amount_drag
                    .as_mut()
                    .expect("route drag checked above");
                update_route_amount(state, response, drag);
            });
            editor_theme::request_display_repaint(ui);
            return true;
        }
        if response.drag_stopped() {
            ui.data_mut(|data| {
                finish_amount_drag(
                    state,
                    data.get_temp_mut_or_default::<DirectModulationState>(id),
                    false,
                );
            });
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

pub(super) fn route_handle_hit_rect(center: egui::Pos2, unit: f32) -> egui::Rect {
    let diameter = modulation_handle_hit_radius(unit) * 2.0;
    egui::Rect::from_center_size(center, egui::vec2(diameter, diameter))
}

pub(super) fn route_handle_id(route: usize) -> egui::Id {
    egui::Id::new((UI_STATE_ID, "modulation-handle", route))
}

fn modulation_drag_delta(response: &egui::Response) -> f32 {
    let delta = response.drag_motion();
    delta.x - delta.y
}

pub(super) fn update_route_amount(
    state: &PluginContext<KurvParams>,
    response: &egui::Response,
    drag: &mut AmountDrag,
) {
    drag.amount = (drag.amount + modulation_drag_delta(response) / 120.0).clamp(-1.0, 1.0);
    set_route_amount(state, drag.route, drag.amount);
}

pub(super) fn finish_amount_drag(
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

#[allow(clippy::too_many_arguments)]
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

pub(super) fn inset_clamp(value: f32, min: f32, max: f32, inset: f32) -> f32 {
    let low = min + inset;
    let high = max - inset;
    if low <= high {
        value.clamp(low, high)
    } else {
        (min + max) * 0.5
    }
}

pub(super) fn paint_modulation_knob(
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

pub(super) fn brighten_control(ui: &egui::Ui, rect: egui::Rect, color: egui::Color32, alpha: u8) {
    let [red, green, blue, _] = color.to_array();
    ui.painter().rect_filled(
        rect,
        editor_theme::shape::CONTROL_RADIUS,
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
