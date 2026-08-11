use std::sync::Arc;

use crate::editor_theme;
use crate::generators::OscillatorConfig;
use crate::oscillators::{
    Antialiasing, PhaseWarpMode, VaTableRt, VaTableState,
    sample_custom_shape_with_antialiasing_warped,
};
use crate::wave_curve::WaveCurveRt;

const HOST_PREVIEW_SAMPLE_RATE: f32 = 48_000.0;
const PREVIEW_POINTS: u16 = 512;

#[derive(Clone)]
pub(super) struct VaPreviewCache {
    generation: u32,
    table: Arc<VaTableRt>,
    geometry: Option<Arc<VaPreviewGeometry>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct VaPreviewGeometryKey {
    generation: u32,
    rect: [u32; 4],
    shape: u32,
    custom_shape: u32,
    pulse_width: u32,
    phase_position: u32,
    phase_warp_mode: u8,
    phase_warp_amount: u32,
    accent: [u8; 4],
}

struct VaPreviewGeometry {
    key: VaPreviewGeometryKey,
    points: Arc<[egui::Pos2]>,
    fill: Arc<egui::Mesh>,
}

impl VaPreviewCache {
    pub(super) fn load(ui: &egui::Ui, cache_id: egui::Id, table_state: &VaTableState) -> Self {
        let mut cache = ui.data(|store| store.get_temp::<Self>(cache_id));
        if let Some((generation, table)) =
            table_state.try_table_rt(cache.as_ref().map_or(0, |cache| cache.generation))
        {
            cache = Some(Self {
                generation,
                table: Arc::new(table),
                geometry: None,
            });
        }
        cache.unwrap_or_else(|| Self {
            generation: 0,
            table: Arc::new(table_state.snapshot().compile_rt()),
            geometry: None,
        })
    }

    pub(super) fn table(&self) -> Arc<VaTableRt> {
        Arc::clone(&self.table)
    }

    pub(super) fn store(self, ui: &egui::Ui, cache_id: egui::Id) {
        ui.data_mut(|store| store.insert_temp(cache_id, self));
    }
}

pub(super) fn cycle_plot(rect: egui::Rect) -> egui::Rect {
    let inset = editor_theme::space::XS.min(rect.height() * 0.12);
    rect.shrink2(egui::vec2(inset, inset * 0.65))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn paint_cached_cycle(
    cache: &mut VaPreviewCache,
    painter: &egui::Painter,
    rect: egui::Rect,
    plot: egui::Rect,
    config: &OscillatorConfig,
    curve: WaveCurveRt,
    mix: f32,
    editing: bool,
    accent: egui::Color32,
) {
    let geometry_key = VaPreviewGeometryKey {
        generation: cache.generation,
        rect: [
            rect.min.x.to_bits(),
            rect.min.y.to_bits(),
            rect.width().to_bits(),
            rect.height().to_bits(),
        ],
        shape: config.shape.to_bits(),
        custom_shape: config.custom_shape.to_bits(),
        pulse_width: config.pulse_width.to_bits(),
        phase_position: config.phase_position.to_bits(),
        phase_warp_mode: config.phase_warp_mode,
        phase_warp_amount: config.phase_warp_amount.to_bits(),
        accent: accent.to_array(),
    };
    let geometry = if !editing
        && let Some(geometry) = cache
            .geometry
            .as_ref()
            .filter(|geometry| geometry.key == geometry_key)
    {
        Arc::clone(geometry)
    } else {
        let phase_step = 110.0_f64 / f64::from(HOST_PREVIEW_SAMPLE_RATE);
        let points = build_cycle_points(plot, |normalized| {
            sample_custom_shape_with_antialiasing_warped(
                config.shape.clamp(0.0, 3.0),
                f64::from((normalized + config.phase_position).rem_euclid(1.0)),
                phase_step,
                config.pulse_width.clamp(0.03, 0.97),
                Antialiasing::Spline,
                PhaseWarpMode::from_index(config.phase_warp_mode),
                config.phase_warp_amount,
                curve,
                mix,
            )
        });
        let geometry = Arc::new(VaPreviewGeometry {
            key: geometry_key,
            fill: build_cycle_fill(&points, plot.center().y, accent, 42),
            points,
        });
        if !editing {
            cache.geometry = Some(Arc::clone(&geometry));
        }
        geometry
    };
    paint_cycle(painter, rect, &geometry, accent);
}

fn build_cycle_points(
    plot: egui::Rect,
    mut sample_at: impl FnMut(f32) -> f32,
) -> Arc<[egui::Pos2]> {
    (0..=PREVIEW_POINTS)
        .map(|index| {
            let normalized = f32::from(index) / f32::from(PREVIEW_POINTS);
            let sample = sample_at(normalized);
            egui::pos2(
                plot.width().mul_add(normalized, plot.left()),
                (sample * plot.height()).mul_add(-0.42, plot.center().y),
            )
        })
        .collect::<Vec<_>>()
        .into()
}

fn build_cycle_fill(
    points: &[egui::Pos2],
    baseline: f32,
    color: egui::Color32,
    edge_alpha: u8,
) -> Arc<egui::Mesh> {
    let edge = egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), edge_alpha);
    let transparent = egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 0);
    let mut mesh = egui::Mesh::default();
    mesh.reserve_vertices((points.len() - 1) * 4);
    mesh.reserve_triangles((points.len() - 1) * 2);
    for pair in points.windows(2) {
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(pair[0], edge);
        mesh.colored_vertex(pair[1], edge);
        mesh.colored_vertex(egui::pos2(pair[1].x, baseline), transparent);
        mesh.colored_vertex(egui::pos2(pair[0].x, baseline), transparent);
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    }
    Arc::new(mesh)
}

fn paint_cycle(
    painter: &egui::Painter,
    rect: egui::Rect,
    geometry: &VaPreviewGeometry,
    color: egui::Color32,
) {
    painter.rect_filled(rect, 0.0, editor_theme::semantic().well);
    painter.add(Arc::clone(&geometry.fill));
    painter.add(egui::Shape::line(
        geometry.points.to_vec(),
        egui::Stroke::new(editor_theme::shape::FOCUS_STROKE, color),
    ));
}
