use truce_core::editor::PluginContext;

mod painting;

use crate::{KurvParams, editor_theme};

use super::route_bank::{
    ROUTE_COUNT, RouteAssignmentSnapshot, RouteBucket, UiRoute, begin_route_amount_edit,
    clear_route, end_route_amount_edit, route_amount, set_route_amount,
};
use super::{
    AmountDrag, DirectModulationState, TrackAxis, UI_STATE_ID, modulation_handle_hit_radius,
    modulation_handle_lane_spacing, modulation_pointer_position, modulation_unit,
};

pub(super) use painting::{
    paint_parent_route_marker, paint_route_depth_modulation, paint_route_marker,
};

#[allow(clippy::too_many_arguments)]
pub(super) fn paint_destination_routes(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    _response: &egui::Response,
    target: super::UiDestination,
    track: egui::Rect,
    axis: TrackAxis,
    anchor: egui::Pos2,
    base: f32,
    span: f32,
    routes: &RouteBucket,
) -> bool {
    let id = egui::Id::new(UI_STATE_ID);
    let unit = modulation_unit(ui);
    // Route markers are interactive in the root overlay so they remain above
    // compact controls without inheriting a nested child clip.
    let clip_rect = ui.ctx().content_rect();
    let (dragging_source, hovered_source, amount_drag, parent_route_focus_mask) =
        ui.data_mut(|data| {
            let direct = data.get_temp_mut_or_default::<DirectModulationState>(id);
            (
                direct.dragging_source,
                direct.hovered_source,
                direct.amount_drag,
                direct.parent_route_focus_mask,
            )
        });
    let pointer = ui.input(|input| input.pointer.latest_pos());
    let assigning = super::assignment_source(ui);
    let has_assignment_plus = assigning.is_some_and(|source| match target {
        super::UiDestination::Host(target) => {
            RouteAssignmentSnapshot::capture(ui, state, source).accepts_host(target)
        }
        super::UiDestination::Modular(target) => {
            RouteAssignmentSnapshot::capture(ui, state, source).accepts_modular(target)
        }
    });
    let child_row = !has_assignment_plus;
    let group_revealed = pointer.is_some_and(|pointer| {
        route_handle_group_rect(anchor, routes.len(), clip_rect, unit, child_row).contains(pointer)
    }) || assigning.is_some()
        || amount_drag.is_some_and(|drag| {
            routes
                .as_slice()
                .iter()
                .any(|(route, ..)| *route == drag.route)
        })
        || hovered_source.is_some_and(|source| {
            routes
                .as_slice()
                .iter()
                .any(|(_, route_source, ..)| *route_source == source)
        })
        || routes
            .as_slice()
            .iter()
            .any(|(route, ..)| parent_route_focus_mask & (1_u64 << *route) != 0);
    let group_reveal = ui.ctx().animate_bool_with_time_and_easing(
        egui::Id::new((
            "kurv-modulation-route-group-reveal",
            routes
                .as_slice()
                .first()
                .map_or(usize::MAX, |(route, ..)| *route),
        )),
        group_revealed,
        0.18,
        egui::emath::easing::cubic_in_out,
    );
    let mut route_positions = [egui::Pos2::ZERO; ROUTE_COUNT];
    let mut route_mask = 0_u64;
    for (lane, (route, _, _, _)) in routes.as_slice().iter().enumerate() {
        route_positions[*route] = route_handle_position(
            anchor,
            lane,
            routes.len(),
            clip_rect,
            unit,
            group_reveal,
            route_handle_settle(ui, *route, child_row),
        );
        route_mask |= 1_u64 << *route;
    }
    ui.data_mut(|data| {
        let direct = data.get_temp_mut_or_default::<DirectModulationState>(id);
        for (route, ..) in routes.as_slice() {
            direct.route_handle_positions[*route] = route_positions[*route];
        }
        direct.route_handle_mask |= route_mask;
    });
    let hovered_route = dragging_source
        .is_none()
        .then(|| pointer)
        .flatten()
        .and_then(|pointer| {
            route_handle_hit(
                ui,
                pointer,
                anchor,
                routes.as_slice(),
                clip_rect,
                unit,
                group_reveal,
                child_row,
            )
        });
    let reveal_live_value = painting::paint_routes(
        ui,
        state,
        track,
        axis,
        anchor,
        base,
        span,
        routes.as_slice(),
        hovered_source,
        hovered_route,
        amount_drag.map(|drag| drag.route),
        group_reveal,
        clip_rect,
        unit,
        child_row,
    );
    if reveal_live_value {
        editor_theme::request_display_repaint(ui);
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
            let coarse = ui.input(|input| input.modifiers.ctrl);
            let last_pointer = modulation_pointer_position(ui, &handle_response);
            ui.data_mut(|data| {
                data.get_temp_mut_or_default::<DirectModulationState>(id)
                    .amount_drag = Some(AmountDrag {
                    route: *route,
                    amount,
                    raw_amount: amount,
                    initial_amount: amount,
                    last_pointer,
                    coarse,
                    created: false,
                });
            });
        }
        let route_dragging = ui.data_mut(|data| {
            data.get_temp_mut_or_default::<DirectModulationState>(id)
                .amount_drag
                .is_some_and(|drag| drag.route == *route)
        });
        if route_dragging {
            if handle_response.dragged() || handle_response.drag_stopped() {
                update_route_amount(ui, state, &handle_response, id, *route);
                editor_theme::request_display_repaint(ui);
                if handle_response.dragged() {
                    return true;
                }
            }
            if handle_response.drag_stopped() {
                finish_amount_drag(ui, state, id, false);
                return true;
            }
        }
    }

    if response.double_clicked() && !routes.as_slice().is_empty() {
        finish_amount_drag(ui, state, id, true);
        for (route, _, _, _) in routes.as_slice() {
            clear_route(state, *route);
        }
        return true;
    }

    // Keep the old parent-response path as a fallback for compact controls
    // whose clip rect cannot contain an external handle.
    let clip_rect = ui.ctx().content_rect();
    let unit = modulation_unit(ui);
    let hovered = ui
        .input(|input| input.pointer.latest_pos())
        .and_then(|pointer| {
            route_handle_hit(
                ui,
                pointer,
                modulation_drop_center(ui, response.rect),
                routes.as_slice(),
                clip_rect,
                unit,
                1.0,
                false,
            )
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
                created: false,
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
        if response.dragged() || response.drag_stopped() {
            update_route_amount(ui, state, response, id, drag.route);
            editor_theme::request_display_repaint(ui);
            if response.dragged() {
                return true;
            }
        }
        if response.drag_stopped() {
            finish_amount_drag(ui, state, id, false);
            return true;
        }
    }
    false
}

fn route_handle_hit(
    ui: &egui::Ui,
    pointer: egui::Pos2,
    anchor: egui::Pos2,
    routes: &[UiRoute],
    clip_rect: egui::Rect,
    unit: f32,
    reveal: f32,
    child_row: bool,
) -> Option<usize> {
    routes
        .iter()
        .enumerate()
        .filter_map(|(lane, (route, _, _, _))| {
            let handle = route_handle_position(
                anchor,
                lane,
                routes.len(),
                clip_rect,
                unit,
                reveal,
                route_handle_settle(ui, *route, child_row),
            );
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

pub(super) fn modulation_drop_center(ui: &egui::Ui, carrier: egui::Rect) -> egui::Pos2 {
    let unit = modulation_unit(ui);
    egui::pos2(
        carrier.center().x,
        carrier.bottom() + editor_theme::space::XS + unit * 0.27,
    )
}

pub(super) fn modulation_route_row_step(unit: f32) -> f32 {
    unit
}

pub(super) fn update_route_amount(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    response: &egui::Response,
    id: egui::Id,
    route: usize,
) {
    update_route_amount_at_pointer(
        ui,
        state,
        id,
        route,
        super::modulation_pointer_position(ui, response),
        response.drag_delta().y,
    );
}

fn update_route_amount_at_pointer(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    id: egui::Id,
    route: usize,
    pointer: Option<egui::Pos2>,
    fallback_delta: f32,
) {
    let coarse = ui.input(|input| input.modifiers.ctrl);
    let update = ui.data_mut(|data| {
        let drag = data
            .get_temp_mut_or_default::<DirectModulationState>(id)
            .amount_drag
            .as_mut()
            .filter(|drag| drag.route == route)?;
        let pointer_delta = pointer
            .zip(drag.last_pointer)
            .map_or(fallback_delta, |(now, last)| {
                drag.last_pointer = Some(now);
                now.y - last.y
            });
        if drag.coarse != coarse {
            drag.raw_amount = drag.amount;
            drag.coarse = coarse;
        }
        drag.raw_amount = (drag.raw_amount - pointer_delta / 120.0).clamp(-1.0, 1.0);
        drag.amount = crate::editor_controls::semantic_snap(
            drag.raw_amount,
            crate::editor_controls::ValueSemantic::Percent,
            coarse,
        )
        .clamp(-1.0, 1.0);
        Some((drag.route, drag.amount))
    });
    if let Some((route, amount)) = update {
        set_route_amount(state, route, amount);
    }
}

pub(super) fn update_created_route_drag(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    id: egui::Id,
    primary_down: bool,
    released: bool,
) {
    let route = ui.data(|data| {
        data.get_temp::<DirectModulationState>(id)
            .and_then(|direct| direct.amount_drag.filter(|drag| drag.created))
            .map(|drag| drag.route)
    });
    let Some(route) = route else {
        return;
    };
    if primary_down || released {
        let pointer = ui.input(|input| input.pointer.interact_pos().or(input.pointer.latest_pos()));
        update_route_amount_at_pointer(ui, state, id, route, pointer, 0.0);
        if primary_down {
            editor_theme::request_display_repaint(ui);
        }
    }
    if released || !primary_down {
        finish_amount_drag(ui, state, id, false);
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
    if cancelled && drag.created {
        clear_route(state, drag.route);
    } else if cancelled {
        set_route_amount(state, drag.route, drag.initial_amount);
    }
    end_route_amount_edit(state, drag.route);
    if !cancelled && route_amount(state, drag.route).abs() <= 0.005 {
        clear_route(state, drag.route);
    }
}

fn route_handle_position(
    anchor: egui::Pos2,
    lane: usize,
    route_count: usize,
    clip_rect: egui::Rect,
    unit: f32,
    reveal: f32,
    settle: f32,
) -> egui::Pos2 {
    let hit_radius = modulation_handle_hit_radius(unit);
    let spacing = modulation_handle_lane_spacing(unit, reveal);
    let lane_center = route_count.saturating_sub(1) as f32 * 0.5;
    let half_width = lane_center * spacing + hit_radius;
    let center_x = inset_clamp(anchor.x, clip_rect.left(), clip_rect.right(), half_width);
    let child = egui::pos2(
        center_x + (lane as f32 - lane_center) * spacing,
        inset_clamp(
            anchor.y + modulation_route_row_step(unit),
            clip_rect.top(),
            clip_rect.bottom(),
            hit_radius,
        ),
    );
    egui::pos2(child.x, egui::lerp(child.y..=anchor.y, 1.0 - settle))
}

pub(super) const fn route_handle_settle(_ui: &egui::Ui, _route: usize, child_row: bool) -> f32 {
    if child_row { 0.0 } else { 1.0 }
}

fn route_handle_group_rect(
    anchor: egui::Pos2,
    route_count: usize,
    clip_rect: egui::Rect,
    unit: f32,
    child_row: bool,
) -> egui::Rect {
    if route_count == 0 {
        return egui::Rect::NOTHING;
    }
    let settle = f32::from(!child_row);
    let first = route_handle_position(anchor, 0, route_count, clip_rect, unit, 1.0, settle);
    let last = route_handle_position(
        anchor,
        route_count - 1,
        route_count,
        clip_rect,
        unit,
        1.0,
        settle,
    );
    egui::Rect::from_min_max(first.min(last), first.max(last))
        .expand(modulation_handle_hit_radius(unit))
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
