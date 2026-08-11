//! Reusable, low-ceremony building blocks for KURV editor shells.

use std::hash::Hash;

pub(crate) fn icon_font_ready(ui: &egui::Ui) -> bool {
    let id = egui::Id::new("kurv-phosphor-font-ready");
    ui.data(|data| data.get_temp::<u64>(id))
        .is_some_and(|registered| registered < ui.ctx().cumulative_frame_nr())
}

pub(crate) fn with_child(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    id_salt: impl Hash,
    layout: egui::Layout,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(id_salt)
            .max_rect(rect)
            .layout(layout),
    );
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    add_contents(&mut child);
}

pub(crate) fn gradient_area_to_bottom(
    painter: &egui::Painter,
    points: &[egui::Pos2],
    bottom: f32,
    color: egui::Color32,
    top_alpha: u8,
) {
    gradient_area(painter, points, bottom, color, top_alpha);
}

pub(crate) fn gradient_area_to_baseline(
    painter: &egui::Painter,
    points: &[egui::Pos2],
    baseline: f32,
    color: egui::Color32,
    edge_alpha: u8,
) {
    gradient_area(painter, points, baseline, color, edge_alpha);
}

fn gradient_area(
    painter: &egui::Painter,
    points: &[egui::Pos2],
    baseline: f32,
    color: egui::Color32,
    edge_alpha: u8,
) {
    if points.len() < 2 || edge_alpha == 0 || !baseline.is_finite() {
        return;
    }
    let edge = with_alpha(color, edge_alpha);
    let transparent = with_alpha(color, 0);
    let mut mesh = egui::Mesh::default();
    mesh.reserve_vertices((points.len() - 1) * 4);
    mesh.reserve_triangles((points.len() - 1) * 2);
    for pair in points.windows(2) {
        if pair
            .iter()
            .any(|point| !point.x.is_finite() || !point.y.is_finite())
        {
            continue;
        }
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(pair[0], edge);
        mesh.colored_vertex(pair[1], edge);
        mesh.colored_vertex(egui::pos2(pair[1].x, baseline), transparent);
        mesh.colored_vertex(egui::pos2(pair[0].x, baseline), transparent);
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    }
    if !mesh.is_empty() {
        painter.add(mesh);
    }
}

fn with_alpha(color: egui::Color32, alpha: u8) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}
