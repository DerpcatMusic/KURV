//! Distribution preview painting.

use crate::editor_theme;

pub(super) fn paint_compact_distribution(
    painter: &egui::Painter,
    plot: egui::Rect,
    points: &[egui::Pos2],
    weights: &[f32],
    maximum_weight: f32,
    control_point: egui::Pos2,
    opacity: f32,
    hovered: bool,
    active: bool,
) {
    let opacity = opacity.clamp(0.0, 1.0);
    let palette = editor_theme::semantic();
    if hovered || active {
        let guide = egui::Stroke::new(
            1.0_f32,
            palette
                .unison
                .linear_multiply(if active { 0.30 } else { 0.18 }),
        );
        painter.line_segment(
            [
                egui::pos2(plot.left(), control_point.y),
                egui::pos2(plot.right(), control_point.y),
            ],
            guide,
        );
        painter.line_segment(
            [
                egui::pos2(control_point.x, plot.top()),
                egui::pos2(control_point.x, plot.bottom()),
            ],
            guide,
        );
    }
    for (point, weight) in points.iter().zip(weights) {
        let relative = (weight / maximum_weight.max(f32::EPSILON)).sqrt();
        let half_height = plot.height() * relative.mul_add(0.055, 0.025);
        let color = palette
            .unison
            .linear_multiply(relative.mul_add(0.72, 0.28) * opacity);
        painter.line_segment(
            [
                *point - egui::vec2(0.0, half_height),
                *point + egui::vec2(0.0, half_height),
            ],
            egui::Stroke::new(editor_theme::font::CAPTION_SIZE * 0.20, color),
        );
    }
    let control_radius = editor_theme::font::CAPTION_SIZE
        * if active {
            0.48
        } else if hovered {
            0.42
        } else {
            0.35
        };
    painter.circle_filled(
        control_point,
        control_radius,
        palette.unison.linear_multiply(opacity),
    );
    if hovered || active {
        painter.circle_stroke(
            control_point,
            control_radius * 1.65,
            egui::Stroke::new(
                1.0_f32,
                palette
                    .unison
                    .linear_multiply(if active { opacity } else { opacity * 0.62 }),
            ),
        );
    }
}
