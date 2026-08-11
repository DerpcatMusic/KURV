//! Source-drag state, target hit-testing, and assignment feedback.

use super::*;

#[derive(Clone, Copy)]
pub(super) struct DirectModulationSnapshot {
    pub(super) dragging_source: Option<ResolvedRouteSource>,
    pub(super) hovered_source: Option<ResolvedRouteSource>,
    pub(super) source_rect: egui::Rect,
    pub(super) hovered_target: Option<UiDestination>,
    pub(super) hovered_target_valid: bool,
    pub(super) inspector_rect: egui::Rect,
    pub(super) amount_drag: Option<AmountDrag>,
}

impl DirectModulationState {
    pub(super) fn snapshot(&self) -> DirectModulationSnapshot {
        DirectModulationSnapshot {
            dragging_source: self.dragging_source,
            hovered_source: self.hovered_source,
            source_rect: self.source_rect,
            hovered_target: self.hovered_target,
            hovered_target_valid: self.hovered_target_valid,
            inspector_rect: self.inspector_rect,
            amount_drag: self.amount_drag,
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

fn clamp_point(point: egui::Pos2, bounds: egui::Rect, inset: f32) -> egui::Pos2 {
    egui::pos2(
        inset_clamp(point.x, bounds.left(), bounds.right(), inset),
        inset_clamp(point.y, bounds.top(), bounds.bottom(), inset),
    )
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
    let inset = editor_theme::space::XXS.min(rect.width() * 0.18);
    let baseline = rect.bottom() - editor_theme::shape::STROKE;
    painter.line_segment(
        [
            egui::pos2(rect.left() + inset, baseline),
            egui::pos2(rect.right() - inset, baseline),
        ],
        egui::Stroke::new(
            if hovered {
                editor_theme::shape::FOCUS_STROKE
            } else {
                editor_theme::shape::STROKE
            },
            feedback.gamma_multiply(if hovered { 1.0 } else { 0.52 }),
        ),
    );
    if hovered && valid {
        painter.circle_filled(
            egui::pos2(rect.center().x, baseline),
            editor_theme::shape::FOCUS_STROKE,
            feedback,
        );
    }
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

pub(super) fn update_drop_targets(
    availability: &RouteAssignmentSnapshot,
    direct: &mut DirectModulationState,
    frame: u64,
    pointer: Option<egui::Pos2>,
    painter: &egui::Painter,
) -> Option<bool> {
    direct.hovered_target = None;
    direct.hovered_target_valid = false;
    direct.hovered_rect = egui::Rect::NOTHING;
    if direct.target_rect_frame != frame {
        return None;
    }
    let Some(source) = direct.dragging_source else {
        return None;
    };
    let color = modulation_source_color(source);
    let mut hovered = None;
    if let Some(pointer) = pointer {
        for (index, rect) in direct.target_rects.iter().copied().enumerate() {
            if !rect.is_positive() || !rect.contains(pointer) {
                continue;
            }
            let target = index as u8 + 1;
            let valid = availability.accepts_host(target);
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
            let valid = availability.accepts_modular(target);
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
    }
    for (index, rect) in direct.target_rects.iter().copied().enumerate() {
        let target = index as u8 + 1;
        if !rect.is_positive() {
            continue;
        }
        let valid = availability.accepts_host(target);
        paint_drop_target(
            painter,
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
        let valid = availability.accepts_modular(target);
        paint_drop_target(
            painter,
            entry.rect,
            color,
            direct.hovered_target == Some(UiDestination::Modular(target)),
            valid,
        );
    }
    hovered.map(|(_, _, valid, _)| valid)
}
