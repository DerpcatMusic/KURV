//! Pan-shape curve and handle painting.

use crate::pan_curve::PanShapeCurveData;
use crate::{editor_theme, editor_widgets};

use super::{PanShapePointDragTarget, pan_shape_curve_handle_pos, pan_shape_knot_pos};

const CURVE_POINTS: u16 = 96;

pub(super) fn draw_pan_shape_curve(
    painter: &egui::Painter,
    plot: egui::Rect,
    center_x: f32,
    data: &PanShapeCurveData,
    hovered: Option<PanShapePointDragTarget>,
    active: Option<PanShapePointDragTarget>,
    reveal_handles: bool,
    handle_radius: f32,
) {
    let color = editor_theme::semantic().pan_shape;
    let center_line_x = egui::lerp(plot.left()..=plot.right(), center_x.clamp(0.05, 0.95));
    let draw_half = |left: bool| -> Vec<egui::Pos2> {
        let segments = data.half(left).compile_rt();
        (0..=CURVE_POINTS)
            .map(|index| {
                let input = f32::from(index) / f32::from(CURVE_POINTS);
                let x = if left {
                    egui::lerp(center_line_x..=plot.left(), input)
                } else {
                    egui::lerp(center_line_x..=plot.right(), input)
                };
                egui::pos2(
                    x,
                    egui::lerp(plot.bottom()..=plot.top(), segments.eval(input)),
                )
            })
            .collect()
    };
    let left_points = draw_half(true);
    let right_points = draw_half(false);
    let fill_alpha = if reveal_handles { 88 } else { 56 };
    editor_widgets::gradient_area_to_bottom(
        painter,
        &left_points,
        plot.bottom(),
        color,
        fill_alpha,
    );
    editor_widgets::gradient_area_to_bottom(
        painter,
        &right_points,
        plot.bottom(),
        color,
        fill_alpha,
    );
    painter.add(egui::Shape::line(
        left_points,
        egui::Stroke::new(editor_theme::font::CAPTION_SIZE * 0.18, color),
    ));
    painter.add(egui::Shape::line(
        right_points,
        egui::Stroke::new(editor_theme::font::CAPTION_SIZE * 0.18, color),
    ));
    for (left, half) in [(true, &data.left), (false, &data.right)] {
        let Some(first) = half.knots.first().copied() else {
            continue;
        };
        let Some(last) = half.knots.last().copied() else {
            continue;
        };
        let center_active = active == Some(PanShapePointDragTarget::Center);
        let center_hovered = hovered == Some(PanShapePointDragTarget::Center);
        let endpoint_active = active == Some(PanShapePointDragTarget::Endpoint { left });
        let endpoint_hovered = hovered == Some(PanShapePointDragTarget::Endpoint { left });
        let center = pan_shape_knot_pos(plot, center_x, left, first);
        let endpoint = pan_shape_knot_pos(plot, center_x, left, last);
        if left {
            draw_shape_handle(
                painter,
                center,
                color,
                center_hovered,
                center_active,
                false,
                handle_radius,
            );
        }
        draw_shape_handle(
            painter,
            endpoint,
            color,
            endpoint_hovered,
            endpoint_active,
            false,
            handle_radius,
        );

        for (index, knot) in half
            .knots
            .iter()
            .copied()
            .enumerate()
            .skip(1)
            .take(half.knots.len().saturating_sub(2))
        {
            let position = pan_shape_knot_pos(plot, center_x, left, knot);
            let knot_target = PanShapePointDragTarget::Knot { left, index };
            draw_shape_handle(
                painter,
                position,
                color,
                hovered == Some(knot_target),
                active == Some(knot_target),
                false,
                handle_radius,
            );
        }

        for index in 0..half.knots.len().saturating_sub(1) {
            let curve = pan_shape_curve_handle_pos(plot, center_x, left, half, index);
            let curve_target = PanShapePointDragTarget::Curve { left, index };
            let curve_active = active == Some(curve_target);
            let curve_hovered = hovered == Some(curve_target);
            if reveal_handles || curve_hovered || curve_active {
                draw_shape_handle(
                    painter,
                    curve,
                    color,
                    curve_hovered,
                    curve_active,
                    true,
                    handle_radius,
                );
            }
        }
    }
}

fn draw_shape_handle(
    painter: &egui::Painter,
    position: egui::Pos2,
    color: egui::Color32,
    hovered: bool,
    active: bool,
    curve: bool,
    base_radius: f32,
) {
    let radius = base_radius
        * if active {
            1.36
        } else if hovered {
            1.18
        } else if curve {
            0.72
        } else {
            0.92
        };
    painter.circle_filled(
        position,
        radius,
        if curve {
            editor_theme::semantic().surface
        } else {
            color.gamma_multiply(if active || hovered { 1.0 } else { 0.76 })
        },
    );
    painter.circle_stroke(
        position,
        radius,
        egui::Stroke::new(
            if active { 1.5_f32 } else { 1.0_f32 },
            color.gamma_multiply(if active || hovered { 1.0 } else { 0.62 }),
        ),
    );
    if active {
        painter.circle_stroke(
            position,
            radius * 1.65,
            egui::Stroke::new(1.0_f32, color.gamma_multiply(0.42)),
        );
    }
}
