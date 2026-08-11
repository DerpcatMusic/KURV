use truce_core::editor::PluginContext;

mod painting;

use crate::{KurvParams, editor_theme};

use super::route_bank::{
    RouteBucket, UiRoute, begin_route_amount_edit, clear_route, end_route_amount_edit,
    route_amount, set_route_amount,
};
use super::{
    AmountDrag, DirectModulationState, TrackAxis, UI_STATE_ID, modulation_handle_hit_radius,
    modulation_handle_lane_spacing, modulation_knob_radius, modulation_unit, source_color,
};

pub(super) use painting::paint_modulation_knob;

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
    painting::paint_routes(
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
    if !routes.as_slice().is_empty() {
        painting::paint_live_value(ui, track, axis, live_base, source_color(color_index % 8));
        if dragging_source.is_none() {
            editor_theme::request_display_repaint(ui);
        }
    }
    if hovered_route.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
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
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        }
        if handle_response.double_clicked() {
            finish_amount_drag(ui, state, id, true);
            clear_route(state, *route);
            return true;
        }
        if handle_response.drag_started() {
            finish_amount_drag(ui, state, id, false);
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
                update_route_amount(ui, state, &handle_response, id, *route);
                editor_theme::request_display_repaint(ui);
                return true;
            }
            if handle_response.drag_stopped() {
                finish_amount_drag(ui, state, id, false);
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
        finish_amount_drag(ui, state, id, true);
        clear_route(state, route);
        return true;
    }
    if response.drag_started()
        && let Some(route) = hovered
    {
        finish_amount_drag(ui, state, id, false);
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
            update_route_amount(ui, state, response, id, drag.route);
            editor_theme::request_display_repaint(ui);
            return true;
        }
        if response.drag_stopped() {
            finish_amount_drag(ui, state, id, false);
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
    -response.drag_motion().y
}

pub(super) fn update_route_amount(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    response: &egui::Response,
    id: egui::Id,
    route: usize,
) {
    let delta = modulation_drag_delta(response) / 120.0;
    let update = ui.data_mut(|data| {
        let drag = data
            .get_temp_mut_or_default::<DirectModulationState>(id)
            .amount_drag
            .as_mut()
            .filter(|drag| drag.route == route)?;
        drag.amount = (drag.amount + delta).clamp(-1.0, 1.0);
        Some((drag.route, drag.amount))
    });
    if let Some((route, amount)) = update {
        set_route_amount(state, route, amount);
    }
}

pub(super) fn finish_amount_drag(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    id: egui::Id,
    cancelled: bool,
) {
    let Some(drag) = ui.data_mut(|data| {
        data.get_temp_mut_or_default::<DirectModulationState>(id)
            .amount_drag
            .take()
    }) else {
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
