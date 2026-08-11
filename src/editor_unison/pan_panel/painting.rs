//! Paint-only helpers for the compact pan panel.

use crate::editor_theme;

pub(super) fn paint_stereo_square(
    painter: &egui::Painter,
    plot: egui::Rect,
    point: egui::Pos2,
    hovered: bool,
    active: bool,
) {
    let palette = editor_theme::semantic();
    let accent = palette.pan_shape;
    let emphasis = if active {
        1.0
    } else if hovered {
        0.78
    } else {
        0.52
    };
    painter.rect(
        plot,
        editor_theme::font::CAPTION_SIZE * 0.18,
        plugcat::theme::mix(palette.well, accent, if active { 0.10 } else { 0.055 }),
        egui::Stroke::new(1.0_f32, accent.gamma_multiply(emphasis)),
        egui::StrokeKind::Inside,
    );
    let guide = egui::Stroke::new(
        1.0_f32,
        accent.gamma_multiply(if hovered || active { 0.38 } else { 0.24 }),
    );
    painter.line_segment(
        [
            egui::pos2(plot.center().x, plot.top()),
            egui::pos2(plot.center().x, plot.bottom()),
        ],
        guide,
    );
    painter.line_segment(
        [
            egui::pos2(plot.left(), plot.center().y),
            egui::pos2(plot.right(), plot.center().y),
        ],
        guide,
    );
    let point_radius = editor_theme::font::CAPTION_SIZE
        * if active {
            0.60
        } else if hovered {
            0.54
        } else {
            0.46
        };
    painter.circle_filled(point, point_radius, accent);
    if hovered || active {
        painter.circle_stroke(
            point,
            point_radius * 1.55,
            egui::Stroke::new(1.0_f32, accent.gamma_multiply(emphasis)),
        );
    }
    let label_inset = editor_theme::font::CAPTION_SIZE * 0.65;
    let compact = plot.width() < editor_theme::font::CAPTION_SIZE * 9.5
        || plot.height() < editor_theme::font::CAPTION_SIZE * 6.0;
    let show_labels = plot.width().min(plot.height()) >= editor_theme::font::CAPTION_SIZE * 4.0;
    for (position, align, compact_label, label) in [
        (
            plot.left_top() + egui::Vec2::splat(label_inset),
            egui::Align2::LEFT_TOP,
            "A",
            "ALTR",
        ),
        (
            plot.right_top() + egui::vec2(-label_inset, label_inset),
            egui::Align2::RIGHT_TOP,
            "P",
            "PAIR",
        ),
        (
            plot.left_bottom() + egui::vec2(label_inset, -label_inset),
            egui::Align2::LEFT_BOTTOM,
            "R",
            "RAND",
        ),
        (
            plot.right_bottom() - egui::Vec2::splat(label_inset),
            egui::Align2::RIGHT_BOTTOM,
            "S",
            "SHAPE",
        ),
    ]
    .into_iter()
    .filter(|_| show_labels)
    {
        painter.text(
            position,
            align,
            if compact { compact_label } else { label },
            editor_theme::font::caption(),
            accent.gamma_multiply(if hovered || active { 0.90 } else { 0.64 }),
        );
    }
}
