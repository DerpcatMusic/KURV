//! Paint-only helpers for the VA waveform curve editor.

use super::{CurveDragTarget, curve_handle_pos, knot_pos, value_pos};
use crate::{
    editor_theme,
    wave_curve::{WaveCurveData, WaveCurveRt},
};

pub(super) fn paint_freehand_stroke(
    painter: &egui::Painter,
    points: Vec<egui::Pos2>,
    color: egui::Color32,
) {
    painter.add(egui::Shape::line(points, egui::Stroke::new(1.5_f32, color)));
}

#[allow(clippy::too_many_arguments)]
pub(super) fn paint_curve_edit_overlay(
    painter: &egui::Painter,
    data: &WaveCurveData,
    curve: &WaveCurveRt,
    plot: egui::Rect,
    bipolar: bool,
    color: egui::Color32,
    active: Option<CurveDragTarget>,
    hovered: Option<CurveDragTarget>,
    selected: Option<CurveDragTarget>,
    drawing: bool,
    pointer: Option<egui::Pos2>,
) {
    let handle_radius = (plot.height() * 0.022).clamp(
        editor_theme::space::XXS * 0.72,
        editor_theme::space::XS * 0.62,
    );
    let emphasized_segment = active
        .or(hovered)
        .or(selected)
        .and_then(|target| match target {
            CurveDragTarget::Segment(index) => Some(index),
            CurveDragTarget::Knot(_) => None,
        });
    if let Some(index) = emphasized_segment {
        paint_curve_segment(
            painter,
            data,
            curve,
            index,
            plot,
            bipolar,
            color.gamma_multiply(if active == Some(CurveDragTarget::Segment(index)) {
                1.0
            } else {
                0.72
            }),
        );
    }
    for (index, knot) in data.knots.iter().enumerate() {
        let position = knot_pos(plot, *knot, bipolar);
        let captured = active == Some(CurveDragTarget::Knot(index));
        let chosen = selected == Some(CurveDragTarget::Knot(index));
        let hot = captured || hovered == Some(CurveDragTarget::Knot(index));
        if hot || chosen {
            painter.circle_filled(
                position,
                handle_radius * if captured { 1.9 } else { 1.65 },
                color.gamma_multiply(if captured {
                    0.24
                } else if hot {
                    0.14
                } else {
                    0.08
                }),
            );
        }
        painter.circle_filled(
            position,
            handle_radius
                * if captured {
                    1.0
                } else if hot || chosen {
                    0.9
                } else {
                    0.64
                },
            if captured {
                editor_theme::semantic().text
            } else if hot || chosen {
                color
            } else {
                editor_theme::semantic().well
            },
        );
        painter.circle_stroke(
            position,
            handle_radius * if hot || chosen { 1.0 } else { 0.7 },
            egui::Stroke::new(
                if chosen {
                    editor_theme::shape::FOCUS_STROKE
                } else {
                    editor_theme::shape::STROKE
                },
                color.gamma_multiply(if hot || chosen { 1.0 } else { 0.62 }),
            ),
        );
    }
    for index in 0..data.knots.len() {
        let position = curve_handle_pos(data, curve, index, plot, bipolar);
        let captured = active == Some(CurveDragTarget::Segment(index));
        let chosen = selected == Some(CurveDragTarget::Segment(index));
        let hot = captured || hovered == Some(CurveDragTarget::Segment(index));
        if !hot && !chosen && data.knots[index].curve.abs() <= f32::EPSILON {
            continue;
        }
        let radius = handle_radius
            * if captured {
                0.82
            } else if hot || chosen {
                0.72
            } else {
                0.46
            };
        painter.circle_filled(
            position,
            radius,
            if captured {
                editor_theme::semantic().text
            } else {
                color.gamma_multiply(if hot || chosen { 0.82 } else { 0.42 })
            },
        );
        painter.circle_stroke(
            position,
            radius * 1.22,
            egui::Stroke::new(
                if chosen {
                    editor_theme::shape::FOCUS_STROKE
                } else {
                    editor_theme::shape::STROKE
                },
                color.gamma_multiply(if hot || chosen { 0.9 } else { 0.3 }),
            ),
        );
    }
    if let Some(pointer) = pointer.filter(|_| {
        !matches!(hovered, Some(CurveDragTarget::Knot(_)))
            && !matches!(hovered, Some(CurveDragTarget::Segment(_)))
            && active.is_none()
            && !drawing
    }) {
        painter.circle_stroke(
            pointer,
            handle_radius * 0.68,
            egui::Stroke::new(editor_theme::shape::STROKE, color.gamma_multiply(0.42)),
        );
    }
}

fn paint_curve_segment(
    painter: &egui::Painter,
    data: &WaveCurveData,
    curve: &WaveCurveRt,
    index: usize,
    plot: egui::Rect,
    bipolar: bool,
    color: egui::Color32,
) {
    let Some(knot) = data.knots.get(index) else {
        return;
    };
    let start = knot.phase;
    let end = data.knots.get(index + 1).map_or(1.0, |next| next.phase);
    let points = (0..=24)
        .map(|step| {
            let phase = (end - start).mul_add(step as f32 / 24.0, start);
            value_pos(plot, phase, curve.eval(phase), bipolar)
        })
        .collect();
    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new(editor_theme::shape::FOCUS_STROKE, color),
    ));
}
