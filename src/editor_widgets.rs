//! Reusable, low-ceremony building blocks for KURV editor shells.

use egui::{Rect, Stroke, StrokeKind};

use crate::editor_theme;

pub(crate) fn graph_frame(painter: &egui::Painter, rect: Rect) {
    if !rect.is_positive() || !rect.intersects(painter.clip_rect()) {
        return;
    }
    let palette = editor_theme::semantic();
    let radius = editor_theme::shape::CONTROL_RADIUS;
    painter.rect_filled(rect, radius, palette.well);
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(
            editor_theme::shape::STROKE,
            palette.grid.gamma_multiply(0.38),
        ),
        StrokeKind::Inside,
    );
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
