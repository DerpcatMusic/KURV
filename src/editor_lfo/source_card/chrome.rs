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
    hovered: bool,
    reorder_active: bool,
) {
    let palette = editor_theme::semantic();
    let stroke = editor_theme::shape::STROKE;
    let edge = rect.shrink(stroke * 0.5);
    let strength = if reorder_active {
        0.78
    } else if selected {
        0.54
    } else if hovered {
        0.40
    } else {
        0.24
    };
    ui.painter().rect_stroke(
        edge,
        editor_theme::shape::CONTROL_RADIUS,
        egui::Stroke::new(stroke, palette.grid.gamma_multiply(0.18)),
        egui::StrokeKind::Inside,
    );
    for (points, edge_strength) in [
        ([edge.left_top(), edge.right_top()], strength),
        ([edge.left_top(), edge.left_bottom()], strength * 0.78),
        ([edge.right_top(), edge.right_bottom()], strength * 0.34),
        ([edge.left_bottom(), edge.right_bottom()], strength * 0.46),
    ] {
        ui.painter().line_segment(
            points,
            egui::Stroke::new(stroke, color.gamma_multiply(edge_strength)),
        );
    }

    let mut gradient = egui::Mesh::default();
    let top = edge.top();
    let bottom = top + editor_theme::shape::GROUP_STROKE;
    let start = gradient.vertices.len() as u32;
    gradient.colored_vertex(
        egui::pos2(edge.left(), top),
        color.gamma_multiply((strength + 0.34).min(1.0)),
    );
    gradient.colored_vertex(
        egui::pos2(edge.right(), top),
        color.gamma_multiply((strength * 0.72).max(0.16)),
    );
    gradient.colored_vertex(
        egui::pos2(edge.right(), bottom),
        color.gamma_multiply(strength * 0.20),
    );
    gradient.colored_vertex(
        egui::pos2(edge.left(), bottom),
        color.gamma_multiply(strength * 0.56),
    );
    gradient.add_triangle(start, start + 1, start + 2);
    gradient.add_triangle(start, start + 2, start + 3);
    ui.painter().add(egui::Shape::mesh(gradient));
}
