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
    let perimeter_points = [
        (edge.left_top(), inner.left_top(), 1.0),
        (edge.center_top(), inner.center_top(), 0.82),
        (edge.right_top(), inner.right_top(), 0.38),
        (edge.right_center(), inner.right_center(), 0.14),
        (edge.right_bottom(), inner.right_bottom(), 0.08),
        (edge.center_bottom(), inner.center_bottom(), 0.14),
        (edge.left_bottom(), inner.left_bottom(), 0.34),
        (edge.left_center(), inner.left_center(), 0.74),
    ];
    let mut perimeter = egui::Mesh::default();
    for (outer, inner, falloff) in perimeter_points {
        perimeter.colored_vertex(outer, color.gamma_multiply(strength * falloff));
        perimeter.colored_vertex(inner, color.gamma_multiply(strength * falloff * 0.12));
    }
    for point in 0..perimeter_points.len() {
        let outer = point as u32 * 2;
        let inner = outer + 1;
        let next_outer = ((point + 1) % perimeter_points.len()) as u32 * 2;
        let next_inner = next_outer + 1;
        perimeter.add_triangle(outer, next_outer, next_inner);
        perimeter.add_triangle(outer, next_inner, inner);
    }
    ui.painter().add(egui::Shape::mesh(perimeter));
}
