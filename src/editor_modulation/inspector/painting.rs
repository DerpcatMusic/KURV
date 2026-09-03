use crate::editor_modulation::{
    TrackAxis, modulation_route_marker_radius, modulation_source_color,
};
use crate::modulators::routing::{ModulationRouteTarget, ResolvedRouteSource};
use crate::{KurvParams, editor_modulation::route_bank::UiRoute, editor_theme};
use truce_core::editor::PluginContext;

use super::{route_handle_position, route_handle_settle};

#[allow(clippy::too_many_arguments)]
pub(super) fn paint_routes(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    track: egui::Rect,
    axis: TrackAxis,
    anchor: egui::Pos2,
    base: f32,
    span: f32,
    routes: &[UiRoute],
    hovered_source: Option<ResolvedRouteSource>,
    hovered_route: Option<usize>,
    active_route: Option<usize>,
    group_reveal: f32,
    clip_rect: egui::Rect,
    unit: f32,
    child_row: bool,
) -> bool {
    let mut reveal_live_value = false;
    for (lane, (route, source, amount, bipolar)) in routes.iter().enumerate() {
        let child_editing = hovered_route == Some(*route) || active_route == Some(*route);
        let route_revealed = child_editing || Some(*source) == hovered_source;
        let route_reveal = ui.ctx().animate_bool_with_time_and_easing(
            egui::Id::new(("kurv-modulation-route-range-reveal", *route)),
            route_revealed,
            0.18,
            egui::emath::easing::cubic_in_out,
        );
        reveal_live_value |= group_reveal > 0.001;
        let (start_value, end_value) = route_range(base, span, *amount, *bipolar);
        let offset = lane as f32 * editor_theme::shape::FOCUS_STROKE;
        let color = modulation_source_color(*source);
        if route_reveal > 0.001 {
            let alpha = (route_reveal * 220.0).round() as u8;
            let stroke = egui::Stroke::new(
                editor_theme::shape::FOCUS_STROKE,
                egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha),
            );
            match axis {
                TrackAxis::Horizontal => ui.painter().add(egui::Shape::dashed_line(
                    &[
                        egui::pos2(
                            egui::lerp(track.left()..=track.right(), start_value),
                            track.bottom() - offset,
                        ),
                        egui::pos2(
                            egui::lerp(track.left()..=track.right(), end_value),
                            track.bottom() - offset,
                        ),
                    ],
                    stroke,
                    editor_theme::space::XS,
                    editor_theme::space::XXS,
                )),
                TrackAxis::Vertical => ui.painter().add(egui::Shape::dashed_line(
                    &[
                        egui::pos2(
                            track.right() - offset,
                            egui::lerp(track.bottom()..=track.top(), start_value),
                        ),
                        egui::pos2(
                            track.right() - offset,
                            egui::lerp(track.bottom()..=track.top(), end_value),
                        ),
                    ],
                    stroke,
                    editor_theme::space::XS,
                    editor_theme::space::XXS,
                )),
                TrackAxis::Radial => {
                    const START: f32 = -std::f32::consts::PI * 0.75;
                    const SWEEP: f32 = std::f32::consts::PI * 1.5;
                    ui.painter().add(egui::Shape::line(
                        modulation_arc_points(
                            track.center(),
                            track.width().min(track.height()) * 0.5 + offset,
                            START + SWEEP * start_value,
                            SWEEP * (end_value - start_value),
                            24,
                        ),
                        stroke,
                    ))
                }
            };
        }
        let handle = route_handle_position(
            anchor,
            lane,
            routes.len(),
            clip_rect,
            unit,
            group_reveal,
            route_handle_settle(ui, *route, child_row),
        );
        let painter = ui.ctx().layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("kurv-modulation-markers"),
        ));
        paint_route_marker(&painter, handle, color, *amount, unit, group_reveal);
        let progress_hidden = ui.ctx().animate_bool_with_time_and_easing(
            egui::Id::new(("kurv-parent-progress-hidden", *route)),
            child_editing,
            0.14,
            egui::emath::easing::cubic_in_out,
        );
        paint_route_depth_modulation(
            ui,
            state,
            &painter,
            handle,
            *route,
            unit,
            group_reveal,
            1.0 - progress_hidden,
        );
    }
    reveal_live_value
}

pub(in crate::editor_modulation) fn paint_route_depth_modulation(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    painter: &egui::Painter,
    center: egui::Pos2,
    route: usize,
    unit: f32,
    reveal: f32,
    opacity: f32,
) {
    let incoming = crate::editor_modulation::route_bank::routes_for_modular_target(
        ui,
        state,
        ModulationRouteTarget::route_depth(route),
    );
    if incoming.is_empty() || opacity <= 0.001 {
        return;
    }
    let marker_radius = modulation_route_marker_radius(unit, reveal);
    let lane_step = marker_radius * 0.42 / incoming.len() as f32;
    let width = (lane_step * 0.72).max(editor_theme::shape::STROKE);
    for (lane, (_, source, amount, _)) in incoming.as_slice().iter().enumerate() {
        let live = match source {
            ResolvedRouteSource::Rack(index) => {
                let index = usize::from(*index);
                if crate::editor_lfo::source_is_running(state, index) {
                    editor_theme::request_display_repaint(ui);
                }
                crate::editor_lfo::source_value_meter(state, index).abs()
            }
            _ => 1.0,
        };
        let radius = marker_radius - (lane as f32 + 0.5) * lane_step;
        let color = modulation_source_color(*source);
        let amount_sweep = std::f32::consts::TAU
            * amount.abs().clamp(0.0, 1.0)
            * if *amount < 0.0 { -1.0 } else { 1.0 };
        if amount_sweep.abs() > f32::EPSILON {
            let points = modulation_arc_points(
                center,
                radius,
                -std::f32::consts::FRAC_PI_2,
                amount_sweep,
                24,
            );
            painter.add(egui::Shape::line(
                points.clone(),
                egui::Stroke::new(
                    width + editor_theme::shape::STROKE,
                    editor_theme::semantic().well.gamma_multiply(opacity),
                ),
            ));
            painter.add(egui::Shape::line(
                points,
                egui::Stroke::new(width, color.gamma_multiply(0.58 * opacity)),
            ));
            let live_sweep = amount_sweep * live;
            if live_sweep.abs() > f32::EPSILON {
                painter.add(egui::Shape::line(
                    modulation_arc_points(
                        center,
                        radius,
                        -std::f32::consts::FRAC_PI_2,
                        live_sweep,
                        24,
                    ),
                    egui::Stroke::new(width, color.gamma_multiply(opacity)),
                ));
            }
        }
    }
}

pub(in crate::editor_modulation) fn paint_parent_route_marker(
    painter: &egui::Painter,
    center: egui::Pos2,
    color: egui::Color32,
    amount: f32,
    unit: f32,
    reveal: f32,
) {
    paint_route_marker(painter, center, color, amount, unit, reveal);
    let radius = modulation_route_marker_radius(unit, reveal);
    painter.circle_stroke(
        center,
        radius,
        egui::Stroke::new(
            editor_theme::shape::FOCUS_STROKE * 1.5,
            editor_theme::semantic().well,
        ),
    );
    let points = modulation_arc_points(center, radius, 0.0, std::f32::consts::TAU, 32);
    painter.add(egui::Shape::dashed_line(
        &points,
        egui::Stroke::new(
            editor_theme::shape::FOCUS_STROKE,
            color.gamma_multiply(reveal),
        ),
        editor_theme::space::XS,
        editor_theme::space::XXS,
    ));
}

pub(in crate::editor_modulation) fn paint_route_marker(
    painter: &egui::Painter,
    center: egui::Pos2,
    color: egui::Color32,
    amount: f32,
    unit: f32,
    reveal: f32,
) {
    let radius = modulation_route_marker_radius(unit, reveal);
    if reveal <= 0.001 {
        painter.circle_filled(center, radius, color);
        return;
    }
    if reveal > 0.5 {
        let rect = egui::Rect::from_center_size(center, egui::Vec2::splat(radius * 2.0));
        painter.add(egui::Shape::Rect(
            egui::epaint::Shadow {
                offset: [0, 2],
                blur: 5,
                spread: 0,
                color: egui::Color32::from_black_alpha((40.0 * reveal).round() as u8),
            }
            .as_shape(rect, radius),
        ));
    }
    let background = editor_theme::semantic()
        .well
        .blend(egui::Color32::from_rgba_unmultiplied(
            color.r(),
            color.g(),
            color.b(),
            28,
        ));
    painter.circle_filled(center, radius, background);
    let sweep = std::f32::consts::TAU
        * egui::lerp(1.0..=amount.abs().clamp(0.0, 1.0), reveal)
        * if amount < 0.0 { -1.0 } else { 1.0 };
    if sweep.abs() > f32::EPSILON {
        let start = -std::f32::consts::FRAC_PI_2;
        const SEGMENTS: u32 = 24;
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(center, color);
        for step in 0..=SEGMENTS {
            let angle = start + sweep * step as f32 / SEGMENTS as f32;
            mesh.colored_vertex(center + egui::Vec2::angled(angle) * radius, color);
            if step > 0 {
                mesh.add_triangle(0, step, step + 1);
            }
        }
        painter.add(egui::Shape::mesh(mesh));
    }
    let inner_radius = radius * 0.34 * reveal;
    painter.circle_filled(center, inner_radius, background);
    if reveal > 0.001 {
        painter.circle_stroke(
            center,
            radius,
            egui::Stroke::new(
                editor_theme::shape::STROKE,
                color.gamma_multiply(0.72 + reveal * 0.28),
            ),
        );
    }
}

fn modulation_arc_points(
    center: egui::Pos2,
    radius: f32,
    start: f32,
    sweep: f32,
    segments: usize,
) -> Vec<egui::Pos2> {
    (0..=segments)
        .map(|index| {
            let t = index as f32 / segments.max(1) as f32;
            let angle = start + sweep * t;
            center + egui::Vec2::angled(angle) * radius
        })
        .collect()
}

fn route_range(base: f32, span: f32, amount: f32, bipolar: bool) -> (f32, f32) {
    if bipolar {
        let extent = amount.abs() * span;
        (
            (base - extent).clamp(0.0, 1.0),
            (base + extent).clamp(0.0, 1.0),
        )
    } else {
        (base, amount.mul_add(span, base).clamp(0.0, 1.0))
    }
}
