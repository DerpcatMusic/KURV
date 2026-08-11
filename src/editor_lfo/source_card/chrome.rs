//! Source-card identity and reorder chrome.

use crate::editor_theme;

pub(super) fn paint_reorder_origin(
    ui: &egui::Ui,
    identity: egui::Rect,
    body: Option<egui::Rect>,
    active: bool,
    color: egui::Color32,
) {
    if !active {
        return;
    }
    if let Some(body) = body {
        ui.painter()
            .rect_filled(body, 0.0, egui::Color32::from_black_alpha(72));
    }
    ui.painter().line_segment(
        [identity.left_bottom(), identity.right_bottom()],
        egui::Stroke::new(editor_theme::shape::FOCUS_STROKE, color),
    );
}

pub(super) fn paint_source_module_edge(
    ui: &egui::Ui,
    rect: egui::Rect,
    color: egui::Color32,
    selected: bool,
    reorder_active: bool,
) {
    let palette = editor_theme::semantic();
    let stroke = editor_theme::shape::STROKE;
    let edge = rect.shrink(stroke * 0.5);
    let strength = if reorder_active {
        0.78
    } else if selected {
        0.54
    } else {
        0.24
    };
    ui.painter().rect_stroke(
        edge,
        editor_theme::shape::CONTROL_RADIUS,
        egui::Stroke::new(stroke, palette.grid.gamma_multiply(0.18)),
        egui::StrokeKind::Inside,
    );
    let inner = edge.shrink(editor_theme::shape::GROUP_STROKE);
    let corners = [
        (edge.left_top(), inner.left_top(), strength),
        (edge.right_top(), inner.right_top(), strength * 0.72),
        (edge.right_bottom(), inner.right_bottom(), strength * 0.48),
        (edge.left_bottom(), inner.left_bottom(), strength * 0.68),
    ];
    let mut perimeter = egui::Mesh::default();
    for (outer, inner, corner_strength) in corners {
        perimeter.colored_vertex(outer, color.gamma_multiply(corner_strength));
        perimeter.colored_vertex(inner, color.gamma_multiply(corner_strength * 0.16));
    }
    for (outer, next_outer, next_inner, inner) in
        [(0, 2, 3, 1), (2, 4, 5, 3), (4, 6, 7, 5), (6, 0, 1, 7)]
    {
        perimeter.add_triangle(outer, next_outer, next_inner);
        perimeter.add_triangle(outer, next_inner, inner);
    }
    ui.painter().add(egui::Shape::mesh(perimeter));
}
