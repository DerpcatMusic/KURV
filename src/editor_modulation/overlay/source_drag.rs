//! Source-drag state, target hit-testing, and assignment feedback.

use super::*;

#[derive(Clone, Copy)]
pub(super) struct DirectModulationSnapshot {
    pub(super) dragging_source: Option<ResolvedRouteSource>,
    pub(super) armed_source: Option<ResolvedRouteSource>,
    pub(super) hovered_source: Option<ResolvedRouteSource>,
    pub(super) source_rect: egui::Rect,
    pub(super) hovered_target: Option<UiDestination>,
    pub(super) hovered_target_valid: bool,
    pub(super) inspector_rect: egui::Rect,
    pub(super) amount_drag: Option<AmountDrag>,
}

#[derive(Clone)]
pub(super) struct DropTargetSnapshot {
    assignment_source: Option<ResolvedRouteSource>,
    hovered_target: Option<UiDestination>,
    geometry: Arc<DropTargetGeometry>,
}

impl DirectModulationState {
    pub(super) fn snapshot(&self) -> DirectModulationSnapshot {
        DirectModulationSnapshot {
            dragging_source: self.dragging_source,
            armed_source: self.armed_source,
            hovered_source: self.hovered_source,
            source_rect: self.source_rect,
            hovered_target: self.hovered_target,
            hovered_target_valid: self.hovered_target_valid,
            inspector_rect: self.inspector_rect,
            amount_drag: self.amount_drag,
        }
    }

    pub(super) fn drop_target_snapshot(&self) -> DropTargetSnapshot {
        DropTargetSnapshot {
            assignment_source: self.dragging_source.or(self.armed_source),
            hovered_target: self.hovered_target,
            geometry: Arc::clone(&self.target_geometry),
        }
    }
}

pub(super) fn paint_source_drag_feedback(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    direct: DirectModulationSnapshot,
    bank_full: bool,
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
        Some(_) if invalid => format!("{source_label}  ·  INVALID TARGET"),
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
    painter.add(egui::Shape::CubicBezier(
        egui::epaint::CubicBezierShape::from_points_stroke(
            [
                start,
                start + egui::vec2(direction * bend, 0.0),
                visual_pointer - egui::vec2(direction * bend, 0.0),
                visual_pointer,
            ],
            false,
            egui::Color32::TRANSPARENT,
            egui::Stroke::new(height * 0.055, feedback_color.gamma_multiply(0.72)),
        ),
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
    painter.circle_filled(
        ghost.left_center() + egui::vec2(height * 0.52, 0.0),
        height * 0.13,
        feedback_color,
    );
    painter.text(
        ghost.left_center() + egui::vec2(height * 0.92, 0.0),
        egui::Align2::LEFT_CENTER,
        drag_label,
        editor_theme::font::label(),
        feedback_color,
    );
}

fn clamp_point(point: egui::Pos2, bounds: egui::Rect, inset: f32) -> egui::Pos2 {
    egui::pos2(
        inset_clamp(point.x, bounds.left(), bounds.right(), inset),
        inset_clamp(point.y, bounds.top(), bounds.bottom(), inset),
    )
}

fn paint_drop_target(
    painter: &egui::Painter,
    center: egui::Pos2,
    color: egui::Color32,
    hovered: bool,
    valid: bool,
    unit: f32,
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
    let radius = unit * if hovered { 0.28 } else { 0.24 };
    let half = radius * 0.48;
    if valid {
        paint_modulation_plus(painter, center, radius, feedback, hovered, hovered);
    } else if hovered {
        painter.circle_filled(center, radius, editor_theme::semantic().well);
        painter.circle_stroke(
            center,
            radius,
            egui::Stroke::new(editor_theme::shape::STROKE, feedback),
        );
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

#[allow(clippy::too_many_arguments)]
fn paint_assignment_target(
    state: &PluginContext<KurvParams>,
    painter: &egui::Painter,
    center: egui::Pos2,
    parent_color: egui::Color32,
    source: ResolvedRouteSource,
    target: UiDestination,
    hovered: bool,
    valid: bool,
    unit: f32,
    active_route: Option<usize>,
) {
    let route_depth = matches!(
        target,
        UiDestination::Modular(ModulationRouteTarget::RouteDepth { .. })
    );
    if route_depth {
        let route = match target {
            UiDestination::Host(target) => route_for_assignment(state, source, target),
            UiDestination::Modular(target) => route_for_modular_assignment(state, source, target),
        };
        if let Some((route, true)) = route {
            paint_route_depth_target(
                painter,
                center,
                modulation_source_color(source),
                hovered,
                true,
                unit,
                route_amount(state, route),
            );
        } else {
            paint_route_depth_target(painter, center, parent_color, hovered, valid, unit, 0.0);
        }
    } else {
        let route = match target {
            UiDestination::Host(target) => route_for_assignment(state, source, target),
            UiDestination::Modular(target) => route_for_modular_assignment(state, source, target),
        };
        if let Some((route, true)) = route.filter(|(route, _)| Some(*route) == active_route) {
            paint_route_marker(
                painter,
                center,
                modulation_source_color(source),
                route_amount(state, route),
                unit,
                1.0,
            );
        } else {
            paint_drop_target(painter, center, parent_color, hovered, valid, unit);
        }
    }
}

fn paint_route_depth_target(
    painter: &egui::Painter,
    center: egui::Pos2,
    color: egui::Color32,
    hovered: bool,
    valid: bool,
    unit: f32,
    amount: f32,
) {
    if !hovered {
        return;
    }
    let color = if valid {
        color
    } else {
        editor_theme::semantic().danger
    };
    paint_parent_route_marker(painter, center, color, amount, unit, 1.0);
}

fn drop_target_hit_rect(center: egui::Pos2, unit: f32) -> egui::Rect {
    route_handle_hit_rect(center, unit)
}

pub(super) fn update_drop_targets(
    availability: &RouteAssignmentSnapshot,
    direct: &mut DirectModulationState,
    frame: u64,
    pointer: Option<egui::Pos2>,
    unit: f32,
) -> Option<bool> {
    direct.hovered_target = None;
    direct.hovered_target_valid = false;
    direct.hovered_rect = egui::Rect::NOTHING;
    if direct.target_rect_frame != frame {
        return None;
    }
    if direct.dragging_source.is_none() && direct.armed_source.is_none() {
        return None;
    }
    let mut hovered = None;
    if let Some(pointer) = pointer {
        let mut host_targets = direct.target_geometry.host_target_mask;
        while host_targets != 0 {
            let index = host_targets.trailing_zeros() as usize;
            host_targets &= host_targets - 1;
            let rect = direct.target_geometry.target_rects[index];
            let center = direct.target_geometry.target_centers[index];
            let hit_rect = rect.union(drop_target_hit_rect(center, unit));
            if !hit_rect.contains(pointer) {
                continue;
            }
            let target = index as u8 + 1;
            let valid = availability.accepts_host(target);
            let area = rect.width() * rect.height();
            if hovered.is_none_or(|(_, _, _, hovered_area)| area < hovered_area) {
                hovered = Some((UiDestination::Host(target), hit_rect, valid, area));
            }
        }
        for entry in direct.target_geometry.modular_target_rects
            [..direct.target_geometry.modular_target_len]
            .iter()
            .copied()
        {
            let Some(target) = entry.target else {
                continue;
            };
            let hit_rect = entry.rect.union(drop_target_hit_rect(entry.center, unit));
            if !hit_rect.contains(pointer) {
                continue;
            }
            let valid = availability.accepts_modular(target);
            let area = entry.rect.width() * entry.rect.height();
            if hovered.is_none_or(|(_, _, _, hovered_area)| area < hovered_area) {
                hovered = Some((UiDestination::Modular(target), hit_rect, valid, area));
            }
        }
    }
    if let Some((target, rect, valid, _)) = hovered {
        direct.hovered_target = Some(target);
        direct.hovered_target_valid = valid;
        direct.hovered_rect = rect;
    }
    hovered.map(|(_, _, valid, _)| valid)
}

pub(super) fn interact_drop_targets(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    availability: &RouteAssignmentSnapshot,
    targets: &DropTargetSnapshot,
    unit: f32,
) {
    let Some(source) = targets.assignment_source else {
        return;
    };
    let mut host_targets = targets.geometry.host_target_mask;
    while host_targets != 0 {
        let index = host_targets.trailing_zeros() as usize;
        host_targets &= host_targets - 1;
        let target = index as u8 + 1;
        let hit_rect = drop_target_hit_rect(targets.geometry.target_centers[index], unit);
        let widget_id = egui::Id::new((UI_STATE_ID, "assignment-host-target", target));
        let double_clicked = ui.input(|input| {
            input
                .pointer
                .button_double_clicked(egui::PointerButton::Primary)
                && input
                    .pointer
                    .latest_pos()
                    .is_some_and(|point| hit_rect.contains(point))
        });
        if double_clicked {
            if let Some((route, true)) = route_for_assignment(state, source, target) {
                clear_route(state, route);
            }
            continue;
        }
        if !availability.accepts_host(target) {
            continue;
        }
        let forced_start = ui.input(|input| {
            input.pointer.button_pressed(egui::PointerButton::Primary)
                && input
                    .pointer
                    .latest_pos()
                    .is_some_and(|point| hit_rect.contains(point))
        });
        if forced_start {
            ui.ctx().set_dragged_id(widget_id);
        }
        let response = ui
            .interact(hit_rect, widget_id, egui::Sense::click_and_drag())
            .on_hover_cursor(egui::CursorIcon::ResizeVertical)
            .on_hover_text("Drag vertically to set modulation depth");
        assignment_amount_gesture_with_start(
            ui,
            state,
            &response,
            source,
            UiDestination::Host(target),
            forced_start,
        );
    }
    for (index, entry) in targets.geometry.modular_target_rects
        [..targets.geometry.modular_target_len]
        .iter()
        .copied()
        .enumerate()
    {
        let Some(target) = entry.target else {
            continue;
        };
        let hit_rect = drop_target_hit_rect(entry.center, unit);
        let widget_id = egui::Id::new((UI_STATE_ID, "assignment-modular-target", index));
        let double_clicked = ui.input(|input| {
            input
                .pointer
                .button_double_clicked(egui::PointerButton::Primary)
                && input
                    .pointer
                    .latest_pos()
                    .is_some_and(|point| hit_rect.contains(point))
        });
        if double_clicked {
            if let Some((route, true)) = route_for_modular_assignment(state, source, target) {
                clear_route(state, route);
            }
            continue;
        }
        if !availability.accepts_modular(target) {
            continue;
        }
        let forced_start = ui.input(|input| {
            input.pointer.button_pressed(egui::PointerButton::Primary)
                && input
                    .pointer
                    .latest_pos()
                    .is_some_and(|point| hit_rect.contains(point))
        });
        if forced_start {
            ui.ctx().set_dragged_id(widget_id);
        }
        let response = ui
            .interact(hit_rect, widget_id, egui::Sense::click_and_drag())
            .on_hover_cursor(egui::CursorIcon::ResizeVertical)
            .on_hover_text("Drag vertically to set modulation depth");
        assignment_amount_gesture_with_start(
            ui,
            state,
            &response,
            source,
            UiDestination::Modular(target),
            forced_start,
        );
    }
}

pub(super) fn paint_drop_targets(
    state: &PluginContext<KurvParams>,
    availability: &RouteAssignmentSnapshot,
    targets: &DropTargetSnapshot,
    painter: &egui::Painter,
    unit: f32,
    active_route: Option<usize>,
) {
    let Some(source) = targets.assignment_source else {
        return;
    };
    let patch = state.params().generator_stack.snapshot();
    let editor = state.params().editor_state.lock().ok();
    let mut host_targets = targets.geometry.host_target_mask;
    while host_targets != 0 {
        let index = host_targets.trailing_zeros() as usize;
        host_targets &= host_targets - 1;
        let center = targets.geometry.target_centers[index];
        let target = index as u8 + 1;
        let valid = availability.accepts_host(target);
        paint_assignment_target(
            state,
            painter,
            center,
            target_parent_color(
                state,
                UiDestination::Host(target),
                &patch,
                editor.as_deref(),
            ),
            source,
            UiDestination::Host(target),
            targets.hovered_target == Some(UiDestination::Host(target)),
            valid,
            unit,
            active_route,
        );
    }
    for entry in targets.geometry.modular_target_rects[..targets.geometry.modular_target_len]
        .iter()
        .copied()
    {
        let Some(target) = entry.target else {
            continue;
        };
        let valid = availability.accepts_modular(target);
        paint_assignment_target(
            state,
            painter,
            entry.center,
            target_parent_color(
                state,
                UiDestination::Modular(target),
                &patch,
                editor.as_deref(),
            ),
            source,
            UiDestination::Modular(target),
            targets.hovered_target == Some(UiDestination::Modular(target)),
            valid,
            unit,
            active_route,
        );
    }
}
