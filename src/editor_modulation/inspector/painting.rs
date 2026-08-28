use crate::editor_modulation::{
    TrackAxis, modulation_knob_radius, modulation_source_color, modulation_unit,
};
use crate::modulators::routing::ResolvedRouteSource;
use crate::{editor_modulation::route_bank::UiRoute, editor_theme};

use super::route_handle_position;

#[allow(clippy::too_many_arguments)]
pub(super) fn paint_routes(
    ui: &egui::Ui,
    track: egui::Rect,
    axis: TrackAxis,
    base: f32,
    span: f32,
    routes: &[UiRoute],
    hovered_source: Option<ResolvedRouteSource>,
    hovered_route: Option<usize>,
    active_route: Option<usize>,
    show_handles: bool,
    clip_rect: egui::Rect,
    unit: f32,
) {
    for (lane, (route, source, amount, bipolar)) in routes.iter().enumerate() {
        let (start_value, end_value) = route_range(base, span, *amount, *bipolar);
        let offset = lane as f32 * editor_theme::shape::FOCUS_STROKE;
        let color = modulation_source_color(*source);
        let stroke = if Some(*source) == hovered_source {
            egui::Stroke::new(
                editor_theme::shape::FOCUS_STROKE + editor_theme::shape::STROKE,
                color,
            )
        } else {
            egui::Stroke::new(editor_theme::shape::STROKE, color)
        };
        match axis {
            TrackAxis::Horizontal => ui.painter().line_segment(
                [
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
            ),
            TrackAxis::Vertical => ui.painter().line_segment(
                [
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
            ),
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
        if show_handles {
            let handle = route_handle_position(track, lane, routes.len(), *amount, clip_rect, unit);
            let hovered = hovered_route == Some(*route) || Some(*source) == hovered_source;
            let painter = ui.ctx().layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("kurv-modulation-knobs"),
            ));
            paint_modulation_knob(
                &painter,
                handle,
                color,
                *amount,
                unit,
                hovered,
                active_route == Some(*route),
            );
        }
    }
}

pub(in crate::editor_modulation) fn paint_modulation_knob(
    painter: &egui::Painter,
    center: egui::Pos2,
    color: egui::Color32,
    amount: f32,
    unit: f32,
    hovered: bool,
    active: bool,
) {
    const START: f32 = std::f32::consts::FRAC_PI_2 * 1.5;
    const SWEEP: f32 = std::f32::consts::TAU * 0.75;
    let base_radius = modulation_knob_radius(unit);
    let radius = if active {
        base_radius + editor_theme::shape::FOCUS_STROKE
    } else if hovered {
        base_radius + editor_theme::shape::STROKE
    } else {
        base_radius
    };
    let depth = amount.abs().clamp(0.0, 1.0);
    painter.circle_filled(center, radius, editor_theme::semantic().well);
    painter.circle_stroke(
        center,
        radius - editor_theme::shape::STROKE * 0.5,
        egui::Stroke::new(
            if active {
                editor_theme::shape::FOCUS_STROKE
            } else {
                editor_theme::shape::STROKE
            },
            if active {
                color.gamma_multiply(0.8)
            } else {
                editor_theme::semantic().grid
            },
        ),
    );
    painter.add(egui::Shape::line(
        modulation_arc_points(center, radius - editor_theme::space::XXS, START, SWEEP, 24),
        egui::Stroke::new(
            editor_theme::shape::STROKE,
            editor_theme::semantic().control_hover,
        ),
    ));
    if depth > f32::EPSILON {
        let arc_start = if amount < 0.0 { START + SWEEP } else { START };
        let arc_sweep = if amount < 0.0 {
            -SWEEP * depth
        } else {
            SWEEP * depth
        };
        painter.add(egui::Shape::line(
            modulation_arc_points(
                center,
                radius - editor_theme::space::XXS,
                arc_start,
                arc_sweep,
                24,
            ),
            egui::Stroke::new(
                if hovered {
                    editor_theme::shape::FOCUS_STROKE + editor_theme::shape::STROKE
                } else {
                    editor_theme::shape::FOCUS_STROKE
                },
                color,
            ),
        ));
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

pub(super) fn paint_live_value(
    ui: &egui::Ui,
    track: egui::Rect,
    axis: TrackAxis,
    value: f32,
    color: egui::Color32,
) {
    let point = match axis {
        TrackAxis::Horizontal => egui::pos2(
            egui::lerp(track.left()..=track.right(), value),
            track.center().y,
        ),
        TrackAxis::Vertical => egui::pos2(
            track.center().x,
            egui::lerp(track.bottom()..=track.top(), value),
        ),
        TrackAxis::Radial => {
            let angle = (-std::f32::consts::PI * 0.75)
                + std::f32::consts::PI * 1.5 * value;
            track.center()
                + egui::Vec2::angled(angle) * (track.width().min(track.height()) * 0.5)
        }
    };
    let radius = modulation_unit(ui) * 0.12;
    ui.painter().circle_filled(point, radius, color);
    ui.painter().circle_stroke(
        point,
        radius + editor_theme::space::XXS,
        egui::Stroke::new(editor_theme::shape::STROKE, color.gamma_multiply(0.75)),
    );
}
