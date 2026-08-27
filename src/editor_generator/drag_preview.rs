use std::hash::Hash;

use crate::editor_controls::fit_font_to_width;
use crate::editor_theme;
use crate::filters::FilterMode;

#[derive(Clone, Copy)]
pub(super) enum GeneratorDragGhostKind {
    Oscillator,
    Filter(FilterMode),
    Group { module_count: usize },
}

pub(super) fn paint_generator_drag_ghost(
    ui: &egui::Ui,
    id: impl Hash,
    pointer: egui::Pos2,
    size: egui::Vec2,
    accent: egui::Color32,
    title: &str,
    detail: &str,
    kind: GeneratorDragGhostKind,
) {
    let palette = editor_theme::semantic();
    let screen = ui.ctx().content_rect().shrink(editor_theme::space::XXS);
    let pointer_gap = editor_theme::title_height(ui) * 0.52;
    let left_room = (pointer.x - screen.left() - pointer_gap).max(0.0);
    let right_room = (screen.right() - pointer.x - pointer_gap).max(0.0);
    let top_room = (pointer.y - screen.top() - pointer_gap).max(0.0);
    let bottom_room = (screen.bottom() - pointer.y - pointer_gap).max(0.0);
    let scale = (left_room.max(right_room) / size.x.max(1.0))
        .min(top_room.max(bottom_room) / size.y.max(1.0))
        .min(1.0);
    let size = egui::vec2(
        (size.x * scale).min(screen.width()),
        (size.y * scale).min(screen.height()),
    );
    let proposed = egui::pos2(
        if right_room >= left_room {
            pointer.x + pointer_gap
        } else {
            pointer.x - pointer_gap - size.x
        },
        if bottom_room >= top_room {
            pointer.y + pointer_gap
        } else {
            pointer.y - pointer_gap - size.y
        },
    );
    let rect = egui::Rect::from_min_size(
        egui::pos2(
            proposed.x.clamp(screen.left(), screen.right() - size.x),
            proposed.y.clamp(screen.top(), screen.bottom() - size.y),
        ),
        size,
    );
    let painter = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new(("generator-drag-ghost", id)),
    ));
    let radius = editor_theme::shape::CONTROL_RADIUS.min(size.y * 0.08);
    painter.rect_filled(rect, radius, palette.surface);
    painter.rect_stroke(
        rect,
        radius,
        egui::Stroke::new(editor_theme::shape::FOCUS_STROKE, accent),
        egui::StrokeKind::Inside,
    );

    match kind {
        GeneratorDragGhostKind::Oscillator => {
            let identity = egui::Rect::from_min_max(
                rect.min,
                egui::pos2(rect.left() + rect.width() * 0.12, rect.bottom()),
            );
            painter.rect_filled(identity, radius, accent.gamma_multiply(0.16));
            painter.line_segment(
                [identity.right_top(), identity.right_bottom()],
                egui::Stroke::new(editor_theme::shape::STROKE, accent),
            );
            painter.text(
                identity.center(),
                egui::Align2::CENTER_CENTER,
                title.replace(' ', "\n"),
                fit_font_to_width(
                    &painter,
                    title,
                    editor_theme::font::caption(),
                    identity.width() * 0.78,
                ),
                palette.text,
            );
            let content = egui::Rect::from_min_max(
                egui::pos2(identity.right() + size.y * 0.08, rect.top() + size.y * 0.12),
                egui::pos2(rect.right() - size.y * 0.08, rect.bottom() - size.y * 0.12),
            );
            let oscillator_right = content.left() + content.width() * 0.40;
            let unison_right = content.left() + content.width() * 0.80;
            for x in [oscillator_right, unison_right] {
                painter.line_segment(
                    [
                        egui::pos2(x, content.top()),
                        egui::pos2(x, content.bottom()),
                    ],
                    egui::Stroke::new(editor_theme::shape::STROKE, palette.grid),
                );
            }
            painter.add(egui::Shape::line(
                vec![
                    egui::pos2(content.left(), content.center().y),
                    egui::pos2(
                        oscillator_right - content.width() * 0.10,
                        content.top() + content.height() * 0.24,
                    ),
                    egui::pos2(
                        oscillator_right - content.width() * 0.10,
                        content.bottom() - content.height() * 0.22,
                    ),
                    egui::pos2(
                        oscillator_right - content.width() * 0.02,
                        content.center().y,
                    ),
                ],
                egui::Stroke::new(editor_theme::shape::FOCUS_STROKE, accent),
            ));
            for bar in 0..5 {
                let phase = (bar + 1) as f32 / 6.0;
                let x = egui::lerp(oscillator_right..=unison_right, phase);
                let half = content.height() * (0.18 + (phase - 0.5).abs() * 0.38);
                painter.line_segment(
                    [
                        egui::pos2(x, content.center().y - half),
                        egui::pos2(x, content.center().y + half),
                    ],
                    egui::Stroke::new(editor_theme::shape::STROKE, accent.gamma_multiply(0.78)),
                );
            }
            let pan_center = egui::pos2((unison_right + content.right()) * 0.5, content.center().y);
            painter.line_segment(
                [
                    egui::pos2(unison_right, pan_center.y),
                    egui::pos2(content.right(), pan_center.y),
                ],
                egui::Stroke::new(editor_theme::shape::STROKE, palette.grid),
            );
            painter.line_segment(
                [
                    egui::pos2(pan_center.x, content.top()),
                    egui::pos2(pan_center.x, content.bottom()),
                ],
                egui::Stroke::new(editor_theme::shape::STROKE, palette.grid),
            );
            painter.circle_filled(pan_center, size.y * 0.045, accent);
            painter.text(
                egui::pos2(content.left(), rect.bottom() - size.y * 0.06),
                egui::Align2::LEFT_BOTTOM,
                detail,
                editor_theme::font::caption(),
                palette.text_muted,
            );
        }
        GeneratorDragGhostKind::Filter(mode) => {
            let header = egui::Rect::from_min_max(
                rect.min,
                egui::pos2(rect.right(), rect.top() + rect.height() * 0.30),
            );
            painter.rect_filled(header, radius, accent.gamma_multiply(0.12));
            painter.text(
                header.left_center() + egui::vec2(size.y * 0.10, 0.0),
                egui::Align2::LEFT_CENTER,
                title,
                editor_theme::font::label(),
                palette.text,
            );
            painter.text(
                header.right_center() - egui::vec2(size.y * 0.10, 0.0),
                egui::Align2::RIGHT_CENTER,
                detail,
                editor_theme::font::caption(),
                accent,
            );
            let preview = egui::Rect::from_min_max(
                egui::pos2(rect.left() + size.y * 0.12, header.bottom() + size.y * 0.06),
                egui::pos2(rect.right() - size.y * 0.12, rect.bottom() - size.y * 0.10),
            );
            let points = match mode {
                FilterMode::Svf => vec![
                    egui::pos2(preview.left(), preview.top() + preview.height() * 0.28),
                    egui::pos2(preview.center().x, preview.top() + preview.height() * 0.28),
                    egui::pos2(preview.right(), preview.bottom()),
                ],
                FilterMode::Phaser => (0..=8)
                    .map(|index| {
                        let x = index as f32 / 8.0;
                        egui::pos2(
                            egui::lerp(preview.left()..=preview.right(), x),
                            egui::lerp(
                                preview.bottom()..=preview.top(),
                                0.72 - 0.5 * (x * std::f32::consts::TAU * 2.5).sin().abs(),
                            ),
                        )
                    })
                    .collect(),
                FilterMode::Scream => vec![
                    egui::pos2(preview.left(), preview.bottom()),
                    egui::pos2(preview.center().x, preview.top() + preview.height() * 0.18),
                    egui::pos2(preview.right(), preview.bottom()),
                ],
            };
            painter.add(egui::Shape::line(
                points,
                egui::Stroke::new(editor_theme::shape::FOCUS_STROKE, accent),
            ));
        }
        GeneratorDragGhostKind::Group { module_count } => {
            let inner = rect.shrink2(egui::vec2(size.y * 0.10, size.y * 0.08));
            let footer_height = inner.height() * 0.24;
            let preview = egui::Rect::from_min_max(
                inner.min,
                egui::pos2(inner.right(), inner.bottom() - footer_height),
            );
            let lane_count = module_count.clamp(1, 3);
            let lane_gap = size.y * 0.05;
            let lane_height = (preview.height() - lane_gap * lane_count.saturating_sub(1) as f32)
                / lane_count as f32;
            for lane in 0..lane_count {
                let lane_rect = egui::Rect::from_min_size(
                    egui::pos2(
                        preview.left(),
                        preview.top() + lane as f32 * (lane_height + lane_gap),
                    ),
                    egui::vec2(preview.width(), lane_height),
                );
                painter.rect_filled(lane_rect, radius * 0.5, palette.well);
                let identity_right = lane_rect.left() + lane_rect.width() * 0.10;
                painter.rect_filled(
                    egui::Rect::from_min_max(
                        lane_rect.min,
                        egui::pos2(identity_right, lane_rect.bottom()),
                    ),
                    radius * 0.5,
                    accent.gamma_multiply(0.14),
                );
                for x in [
                    lane_rect.left() + lane_rect.width() * 0.46,
                    lane_rect.left() + lane_rect.width() * 0.82,
                ] {
                    painter.line_segment(
                        [
                            egui::pos2(x, lane_rect.top()),
                            egui::pos2(x, lane_rect.bottom()),
                        ],
                        egui::Stroke::new(editor_theme::shape::STROKE, palette.grid),
                    );
                }
                painter.line_segment(
                    [
                        egui::pos2(
                            identity_right + lane_rect.width() * 0.04,
                            lane_rect.center().y,
                        ),
                        egui::pos2(
                            lane_rect.left() + lane_rect.width() * 0.42,
                            lane_rect.top() + lane_rect.height() * 0.28,
                        ),
                    ],
                    egui::Stroke::new(editor_theme::shape::STROKE, accent.gamma_multiply(0.82)),
                );
            }
            let footer = egui::Rect::from_min_max(
                egui::pos2(inner.left(), preview.bottom()),
                inner.right_bottom(),
            );
            painter.line_segment(
                [footer.left_top(), footer.right_top()],
                egui::Stroke::new(editor_theme::shape::STROKE, accent.gamma_multiply(0.62)),
            );
            painter.text(
                footer.left_center(),
                egui::Align2::LEFT_CENTER,
                title,
                editor_theme::font::label(),
                palette.text,
            );
            painter.text(
                footer.right_center(),
                egui::Align2::RIGHT_CENTER,
                format!(
                    "{module_count} MODULE{} · {detail}",
                    if module_count == 1 { "" } else { "S" }
                ),
                editor_theme::font::caption(),
                palette.text_muted,
            );
        }
    }
}
