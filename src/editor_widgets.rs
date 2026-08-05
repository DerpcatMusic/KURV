//! Reusable, low-ceremony building blocks for KURV editor shells.

use egui::{Align2, Rect, Stroke, Ui, vec2};

use crate::editor_theme::{self, font, space};

pub(crate) fn graph_frame(painter: &egui::Painter, rect: Rect) {
    painter.rect_filled(rect, 2.0, editor_theme::semantic().well);
}

pub(crate) fn graph_plot(rect: Rect, ui: &Ui, top: f32, bottom: f32) -> Rect {
    let inset = crate::editor_theme::graph_inset(ui);
    Rect::from_min_max(
        rect.left_top() + vec2(inset, top),
        rect.right_bottom() - vec2(inset, bottom),
    )
}

pub(crate) fn graph_grid(painter: &egui::Painter, rect: Rect, columns: usize, rows: usize) {
    let grid = Stroke::new(
        1.0_f32,
        editor_theme::palette().border.linear_multiply(0.52),
    );
    for index in 1..columns {
        let x = egui::lerp(rect.left()..=rect.right(), index as f32 / columns as f32);
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            grid,
        );
    }
    for index in 1..rows {
        let y = egui::lerp(rect.top()..=rect.bottom(), index as f32 / rows as f32);
        painter.line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            grid,
        );
    }
}

pub(crate) fn graph_title(painter: &egui::Painter, rect: Rect, title: &str) {
    painter.text(
        rect.left_top() + vec2(space::SM, space::SM),
        Align2::LEFT_TOP,
        title,
        font::title(),
        editor_theme::palette().text,
    );
}

pub(crate) fn gradient_area_to_bottom(
    painter: &egui::Painter,
    points: &[egui::Pos2],
    bottom: f32,
    color: egui::Color32,
    top_alpha: u8,
) {
    if points.len() < 2 {
        return;
    }
    let top = egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), top_alpha);
    let transparent = egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 0);
    let mut mesh = egui::Mesh::default();
    mesh.reserve_vertices((points.len() - 1) * 4);
    mesh.reserve_triangles((points.len() - 1) * 2);
    for pair in points.windows(2) {
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(pair[0], top);
        mesh.colored_vertex(pair[1], top);
        mesh.colored_vertex(egui::pos2(pair[1].x, bottom), transparent);
        mesh.colored_vertex(egui::pos2(pair[0].x, bottom), transparent);
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    }
    painter.add(mesh);
}

pub(crate) fn gradient_area_to_baseline(
    painter: &egui::Painter,
    points: &[egui::Pos2],
    baseline: f32,
    color: egui::Color32,
    edge_alpha: u8,
) {
    if points.len() < 2 {
        return;
    }
    let edge = egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), edge_alpha);
    let zero = egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 0);
    let mut mesh = egui::Mesh::default();
    mesh.reserve_vertices((points.len() - 1) * 4);
    mesh.reserve_triangles((points.len() - 1) * 2);
    for pair in points.windows(2) {
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(pair[0], edge);
        mesh.colored_vertex(pair[1], edge);
        mesh.colored_vertex(egui::pos2(pair[1].x, baseline), zero);
        mesh.colored_vertex(egui::pos2(pair[0].x, baseline), zero);
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    }
    painter.add(mesh);
}
