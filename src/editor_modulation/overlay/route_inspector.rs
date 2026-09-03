//! Inline source routes, destination links, and modulation-depth handles.

use super::source_drag::DirectModulationSnapshot;
use super::*;

pub(super) fn paint_persistent_cables(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    id: egui::Id,
) {
    let enabled = state
        .params()
        .editor_state
        .lock()
        .is_ok_and(|editor| editor.persistent_modulation_cables);
    if !enabled {
        return;
    }
    let frame = ui.ctx().cumulative_frame_nr();
    let destinations = route_destinations(ui, state);
    let (source_rects, source_frames, geometry, handle_positions, handle_mask) =
        ui.data_mut(|data| {
            let direct = data.get_temp_mut_or_default::<DirectModulationState>(id);
            (
                direct.source_rects,
                direct.source_rect_frames,
                Arc::clone(&direct.target_geometry),
                direct.route_handle_positions,
                direct.route_handle_mask,
            )
        });
    let painter = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("kurv-persistent-modulation-cables"),
    ));
    for (route, target) in destinations.into_iter().enumerate() {
        let Some(source) = route_source(state, route) else {
            continue;
        };
        let source_index = match source {
            ResolvedRouteSource::Rack(index) => usize::from(index),
            ResolvedRouteSource::Generator(index) => {
                crate::modulators::state::MAX_MODULATION_SOURCES + usize::from(index)
            }
            ResolvedRouteSource::XyX => SOURCE_GEOMETRY_COUNT - 3,
            ResolvedRouteSource::XyY => SOURCE_GEOMETRY_COUNT - 2,
            ResolvedRouteSource::ModWheel => SOURCE_GEOMETRY_COUNT - 1,
        };
        if source_frames[source_index] != frame || !source_rects[source_index].is_positive() {
            continue;
        }
        let Some(target) = target else {
            continue;
        };
        let destination = destination_rect(&geometry, target);
        if !destination.is_positive() {
            continue;
        }
        let start = source_rects[source_index].center();
        let end = if handle_mask & (1_u64 << route) != 0 {
            handle_positions[route]
        } else {
            destination.center()
        };
        let span = (end.x - start.x).abs().max(editor_theme::title_height(ui)) * 0.38;
        let direction = if end.x >= start.x { 1.0 } else { -1.0 };
        let path = cubic_bezier_points(
            start,
            start + egui::vec2(direction * span, 0.0),
            end - egui::vec2(direction * span, 0.0),
            end,
            20,
        );
        let color = modulation_source_color(source).gamma_multiply(0.42);
        painter.add(egui::Shape::line(
            path,
            egui::Stroke::new(editor_theme::shape::STROKE, color),
        ));
        painter.circle_filled(end, editor_theme::shape::FOCUS_STROKE, color);
    }
}

pub(super) fn draw(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    id: egui::Id,
    direct: DirectModulationSnapshot,
) {
    if direct.dragging_source.is_some() || direct.armed_source.is_some() {
        clear_inspector_rect(ui, id);
        return;
    }
    let Some(source) = direct.hovered_source else {
        clear_inspector_rect(ui, id);
        return;
    };
    let routes = routes_for_source(ui, state, source);
    if routes.is_empty() {
        clear_inspector_rect(ui, id);
        return;
    }
    let pointer = ui.input(|input| input.pointer.latest_pos());
    let dragging_route = direct
        .amount_drag
        .is_some_and(|drag| route_source(state, drag.route) == Some(source));
    if !dragging_route
        && !pointer.is_some_and(|pointer| {
            direct
                .source_rect
                .expand(editor_theme::space::XS)
                .contains(pointer)
                || direct
                    .inspector_rect
                    .expand(editor_theme::space::XS)
                    .contains(pointer)
        })
    {
        ui.data_mut(|data| {
            let direct = data.get_temp_mut_or_default::<DirectModulationState>(id);
            direct.hovered_source = None;
            direct.source_rect = egui::Rect::NOTHING;
            direct.inspector_rect = egui::Rect::NOTHING;
        });
        return;
    }

    let unit = modulation_unit(ui);
    let reveal = ui.ctx().animate_bool_with_time_and_easing(
        egui::Id::new(("kurv-source-route-group", source)),
        true,
        0.18,
        egui::emath::easing::cubic_in_out,
    );
    let radius = modulation_route_marker_radius(unit, reveal);
    let spacing = radius * 2.0 + editor_theme::space::XS;
    let lane_center = routes.len().saturating_sub(1) as f32 * 0.5;
    let hit_radius = modulation_handle_hit_radius(unit);
    let half_width = lane_center * spacing + hit_radius;
    let screen = ui.ctx().content_rect();
    let center_x = inset_clamp(
        direct.source_rect.center().x,
        screen.left(),
        screen.right(),
        half_width,
    );
    let center_y = inset_clamp(
        direct.source_rect.bottom() + editor_theme::space::XS + unit * 0.27,
        screen.top(),
        screen.bottom(),
        hit_radius,
    );
    let group_rect = egui::Rect::from_center_size(
        egui::pos2(center_x, center_y),
        egui::vec2(half_width * 2.0, hit_radius * 2.0),
    );
    ui.data_mut(|data| {
        data.get_temp_mut_or_default::<DirectModulationState>(id)
            .inspector_rect = group_rect;
    });

    let destinations = route_destinations(ui, state);
    let (geometry, route_handle_mask, route_handle_positions) = ui.data_mut(|data| {
        let direct = data.get_temp_mut_or_default::<DirectModulationState>(id);
        (
            Arc::clone(&direct.target_geometry),
            direct.route_handle_mask,
            direct.route_handle_positions,
        )
    });
    let painter = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("kurv-source-route-group"),
    ));
    for (lane, &(route, _, amount, _)) in routes.as_slice().iter().enumerate() {
        let center = egui::pos2(center_x + (lane as f32 - lane_center) * spacing, center_y);
        let response = ui
            .interact(
                route_handle_hit_rect(center, unit),
                egui::Id::new((UI_STATE_ID, "source-route-depth", route)),
                egui::Sense::click_and_drag(),
            )
            .on_hover_cursor(egui::CursorIcon::ResizeVertical)
            .on_hover_text(format!(
                "{} · {:+.0}% depth · drag to adjust · double-click to clear",
                destinations[route]
                    .map(target_label)
                    .unwrap_or_else(|| "DESTINATION".to_owned()),
                amount * 100.0
            ));
        edit_route_depth(ui, state, id, route, &response);
        route_polarity_menu(&response, state, route);
        paint_route_marker(
            &painter,
            center,
            modulation_source_color(source),
            amount,
            unit,
            reveal,
        );
        let progress_hidden = ui.ctx().animate_bool_with_time_and_easing(
            egui::Id::new(("kurv-source-parent-progress-hidden", route)),
            response.hovered() || dragging_route,
            0.14,
            egui::emath::easing::cubic_in_out,
        );
        paint_route_depth_modulation(
            ui,
            state,
            &painter,
            center,
            route,
            unit,
            reveal,
            1.0 - progress_hidden,
        );

        let Some(target) = destinations[route] else {
            continue;
        };
        let destination = destination_rect(&geometry, target);
        if !destination.is_positive() {
            continue;
        }
        let end = if route_handle_mask & (1_u64 << route) != 0 {
            route_handle_positions[route]
        } else {
            destination.center()
        };
        paint_route_link(
            &painter,
            center,
            end,
            unit,
            modulation_source_color(source),
            reveal,
        );
    }
}

fn paint_route_link(
    painter: &egui::Painter,
    start: egui::Pos2,
    end: egui::Pos2,
    unit: f32,
    color: egui::Color32,
    reveal: f32,
) {
    let span = (end.x - start.x).abs().max(unit) * 0.35;
    let direction = if end.x >= start.x { 1.0 } else { -1.0 };
    let path = cubic_bezier_points(
        start,
        start + egui::vec2(direction * span, 0.0),
        end - egui::vec2(direction * span, 0.0),
        end,
        24,
    );
    painter.add(egui::Shape::dashed_line(
        &path,
        egui::Stroke::new(
            editor_theme::shape::STROKE,
            egui::Color32::from_rgba_unmultiplied(
                color.r(),
                color.g(),
                color.b(),
                (reveal * 210.0).round() as u8,
            ),
        ),
        editor_theme::space::XS,
        editor_theme::space::XXS,
    ));
}

fn edit_route_depth(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    id: egui::Id,
    route: usize,
    response: &egui::Response,
) {
    if response.double_clicked() {
        finish_amount_drag(ui, state, id, true);
        clear_route(state, route);
        return;
    }
    if response.drag_started() {
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
    let dragging = ui.data(|data| {
        data.get_temp::<DirectModulationState>(id)
            .is_some_and(|direct| direct.amount_drag.is_some_and(|drag| drag.route == route))
    });
    if dragging && (response.dragged() || response.drag_stopped()) {
        update_route_amount(ui, state, response, id, route);
        editor_theme::request_display_repaint(ui);
    }
    if dragging && response.drag_stopped() {
        finish_amount_drag(ui, state, id, false);
    }
}

pub(super) fn register_route_handle_widgets(ui: &egui::Ui, state: &PluginContext<KurvParams>) {
    let id = egui::Id::new(UI_STATE_ID);
    let unit = modulation_unit(ui);
    let (route_handle_mask, route_handle_positions, amount_drag, previous_parent_focus_mask) = ui
        .data_mut(|data| {
            let direct = data.get_temp_mut_or_default::<DirectModulationState>(id);
            (
                direct.route_handle_mask,
                direct.route_handle_positions,
                direct.amount_drag,
                direct.parent_route_focus_mask,
            )
        });
    if route_handle_mask == 0 {
        return;
    }
    let destinations = route_destinations(ui, state);
    let mut parent_focus_mask = 0_u64;
    for route in 0..ROUTE_COUNT {
        if route_handle_mask & (1_u64 << route) == 0 {
            continue;
        }
        if matches!(
            destinations[route],
            Some(UiDestination::Modular(
                ModulationRouteTarget::RouteDepth { .. }
            ))
        ) {
            continue;
        }
        let route_dragging = amount_drag.is_some_and(|drag| drag.route == route);
        let handle_rect = route_handle_hit_rect(route_handle_positions[route], unit);
        let mut response = ui.interact(
            handle_rect,
            route_handle_id(route),
            egui::Sense::click_and_drag(),
        );
        if paint_parent_route_group(
            ui,
            state,
            id,
            route,
            route_handle_positions[route],
            unit,
            amount_drag,
            response.hovered() || route_dragging,
            previous_parent_focus_mask & (1_u64 << route) != 0,
        ) {
            parent_focus_mask |= 1_u64 << route;
        }
        if response.hovered() {
            response = response.on_hover_text(format!(
                "{} · {:+.0}% depth · drag to adjust · double-click to clear",
                destinations[route]
                    .map(target_label)
                    .unwrap_or_else(|| "DESTINATION".to_owned()),
                route_amount(state, route) * 100.0
            ));
        }
        if response.double_clicked() {
            finish_amount_drag(ui, state, id, true);
            clear_route(state, route);
            continue;
        }
        route_polarity_menu(&response, state, route);
        if response.hovered() || route_dragging {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        }
    }
    ui.data_mut(|data| {
        data.get_temp_mut_or_default::<DirectModulationState>(id)
            .parent_route_focus_mask = parent_focus_mask;
    });
}

fn paint_parent_route_group(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    id: egui::Id,
    child_route: usize,
    child_center: egui::Pos2,
    unit: f32,
    amount_drag: Option<AmountDrag>,
    child_active: bool,
    was_focused: bool,
) -> bool {
    let parents =
        routes_for_modular_target(ui, state, ModulationRouteTarget::route_depth(child_route));
    let assigning = assignment_source(ui);
    let exact_lane = assigning.and_then(|source| {
        parents
            .as_slice()
            .iter()
            .position(|(_, parent_source, ..)| *parent_source == source)
    });
    let parent_target = ModulationRouteTarget::route_depth(child_route);
    let can_assign_parent = assigning.is_some_and(|source| {
        RouteAssignmentSnapshot::capture(ui, state, source).accepts_modular(parent_target)
    });
    let ghost_lane = assigning
        .filter(|_| can_assign_parent)
        .filter(|_| exact_lane.is_none())
        .map(|_| parents.len());
    let slot_count = parents.len() + usize::from(ghost_lane.is_some());
    if slot_count == 0 {
        return false;
    }
    let screen = ui.ctx().content_rect();
    let parent_anchor = child_center;
    let group_rect = parent_route_group_rect(parent_anchor, slot_count, screen, unit);
    let pointer = ui.input(|input| input.pointer.latest_pos());
    let dragging_parent = amount_drag.is_some_and(|drag| {
        parents
            .as_slice()
            .iter()
            .any(|(route, ..)| *route == drag.route)
    });
    let pointer_over_group =
        pointer.is_some_and(|pointer| group_rect.expand(editor_theme::space::XS).contains(pointer));
    let child_stage = ui.ctx().animate_bool_with_time_and_easing(
        egui::Id::new(("kurv-parent-child-stage", child_route)),
        child_active || dragging_parent || (was_focused && pointer_over_group),
        0.18,
        egui::emath::easing::cubic_in_out,
    );
    let revealed = dragging_parent
        || (child_active && child_stage >= 0.82)
        || (was_focused && pointer_over_group);
    let reveal = ui.ctx().animate_bool_with_time_and_easing(
        egui::Id::new(("kurv-parent-route-group", child_route)),
        revealed,
        0.18,
        egui::emath::easing::cubic_in_out,
    );
    let painter = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("kurv-parent-route-markers"),
    ));
    for (lane, &(route, source, amount, _)) in parents.as_slice().iter().enumerate() {
        let center = parent_route_center(parent_anchor, lane, slot_count, screen, unit);
        if reveal > 0.001 {
            paint_parent_route_marker(
                &painter,
                center,
                modulation_source_color(source),
                amount,
                unit,
                reveal,
            );
        }
        if reveal > 0.01 && assigning.is_none() {
            let response = ui
                .interact(
                    route_handle_hit_rect(center, unit),
                    egui::Id::new((UI_STATE_ID, "parent-route-depth", route)),
                    egui::Sense::click_and_drag(),
                )
                .on_hover_cursor(egui::CursorIcon::ResizeVertical)
                .on_hover_text(format!(
                    "{} parent · {:+.0}% · drag to adjust · double-click to clear",
                    modulation_source_label(state, source),
                    amount * 100.0
                ));
            edit_route_depth(ui, state, id, route, &response);
            route_polarity_menu(&response, state, route);
        }
    }
    if let Some(source) = assigning
        && let Some(lane) = exact_lane.or(ghost_lane)
    {
        let center = parent_route_center(parent_anchor, lane, slot_count, screen, unit);
        if ghost_lane.is_some() && reveal > 0.001 {
            paint_parent_route_marker(
                &painter,
                center,
                modulation_source_color(source),
                0.0,
                unit,
                reveal,
            );
        }
        let response = ui.interact(
            route_handle_hit_rect(center, unit),
            egui::Id::new((UI_STATE_ID, "parent-route-target", child_route)),
            egui::Sense::hover(),
        );
        modular_destination(
            ui,
            state,
            parent_target,
            &response,
            route_amount(state, child_route).mul_add(0.5, 0.5),
            egui::Rect::from_center_size(
                center - egui::vec2(0.0, editor_theme::space::XS + unit * 0.27),
                egui::Vec2::ZERO,
            ),
            TrackAxis::Radial,
            0.5,
        );
    } else if !parents.is_empty() {
        let center = group_rect.center();
        let response = ui.interact(
            route_handle_hit_rect(center, unit),
            egui::Id::new((UI_STATE_ID, "parent-route-anchor", child_route)),
            egui::Sense::hover(),
        );
        modular_destination(
            ui,
            state,
            parent_target,
            &response,
            route_amount(state, child_route).mul_add(0.5, 0.5),
            egui::Rect::from_center_size(
                center - egui::vec2(0.0, editor_theme::space::XS + unit * 0.27),
                egui::Vec2::ZERO,
            ),
            TrackAxis::Radial,
            0.5,
        );
    }
    reveal > 0.001 || dragging_parent
}

fn parent_route_center(
    child: egui::Pos2,
    lane: usize,
    count: usize,
    screen: egui::Rect,
    unit: f32,
) -> egui::Pos2 {
    let spacing = modulation_handle_lane_spacing(unit, 1.0);
    let lane_center = count.saturating_sub(1) as f32 * 0.5;
    let group = parent_route_group_rect(child, count, screen, unit);
    egui::pos2(
        group.center().x + (lane as f32 - lane_center) * spacing,
        group.center().y,
    )
}

fn parent_route_group_rect(
    child: egui::Pos2,
    count: usize,
    screen: egui::Rect,
    unit: f32,
) -> egui::Rect {
    let hit_radius = modulation_handle_hit_radius(unit);
    let spacing = modulation_handle_lane_spacing(unit, 1.0);
    let width = (count.saturating_sub(1) as f32).mul_add(spacing, hit_radius * 2.0);
    let center_x = inset_clamp(child.x, screen.left(), screen.right(), width * 0.5);
    egui::Rect::from_center_size(
        egui::pos2(center_x, child.y + modulation_route_row_step(unit)),
        egui::vec2(width, hit_radius * 2.0),
    )
}

fn clear_inspector_rect(ui: &egui::Ui, id: egui::Id) {
    ui.data_mut(|data| {
        data.get_temp_mut_or_default::<DirectModulationState>(id)
            .inspector_rect = egui::Rect::NOTHING;
    });
}

fn destination_rect(geometry: &DropTargetGeometry, target: UiDestination) -> egui::Rect {
    match target {
        UiDestination::Host(target) => geometry.target_rects[usize::from(target.saturating_sub(1))],
        UiDestination::Modular(target) => geometry.modular_target_rects
            [..geometry.modular_target_len]
            .iter()
            .find(|entry| entry.target == Some(target))
            .map_or(egui::Rect::NOTHING, |entry| entry.rect),
    }
}

fn route_polarity_menu(response: &egui::Response, state: &PluginContext<KurvParams>, route: usize) {
    let Some(source) = route_source(state, route) else {
        return;
    };
    let Some(bipolar) = crate::editor_lfo::source_bipolar(state, source) else {
        return;
    };
    response.context_menu(|ui| {
        let label = if bipolar {
            "Use unipolar source"
        } else {
            "Use bipolar source"
        };
        if ui.button(label).clicked() {
            crate::editor_lfo::set_source_bipolar(state, source, !bipolar);
            ui.close();
        }
    });
}
