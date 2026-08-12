use super::{
    interaction::{curve_value, segment_handles},
    *,
};
use std::hash::{Hash, Hasher};

const SOURCE_DRAG_POINTS: u8 = 64;

pub(super) fn paint_source_drag_curve(
    ui: &egui::Ui,
    painter: &egui::Painter,
    cache_id: egui::Id,
    geometry: SplineGeometry,
    generation: u32,
    compiled: WaveCurveRt,
    color: egui::Color32,
) {
    let plot = geometry.plot();
    let key = (
        generation,
        [
            plot.min.x.to_bits(),
            plot.min.y.to_bits(),
            plot.width().to_bits(),
            plot.height().to_bits(),
        ],
        geometry.bipolar(),
        painter.ctx().pixels_per_point().to_bits(),
        color.to_array(),
    );
    let mesh = editor_widgets::cached_stroke_mesh(
        ui,
        cache_id,
        key,
        || {
            (0..=SOURCE_DRAG_POINTS)
                .map(|point| {
                    let phase = f32::from(point) / f32::from(SOURCE_DRAG_POINTS);
                    geometry.position(phase, compiled.eval(phase))
                })
                .collect()
        },
        egui::Stroke::new((plot.height() * 0.014).clamp(1.25, 2.0), color),
    );
    painter.add(mesh);
}

pub(super) struct EditorCurvePaint<'a> {
    pub(super) data: Option<&'a WaveCurveData>,
    pub(super) geometry: SplineGeometry,
    pub(super) color: egui::Color32,
    pub(super) point_hit: Option<usize>,
    pub(super) handle_hit: Option<usize>,
    pub(super) editor: &'a SplineEditorUi,
    pub(super) playhead_phase: f32,
    pub(super) point_radius: f32,
}

pub(super) fn paint_editor_curve(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    response: &egui::Response,
    frame: EditorCurvePaint<'_>,
) {
    let plot = frame.geometry.plot();
    let baseline = if frame.geometry.bipolar() {
        plot.center().y
    } else {
        plot.bottom()
    };
    let mut data_hash = std::hash::DefaultHasher::new();
    if let Some(data) = frame.data {
        for knot in &data.knots {
            knot.phase.to_bits().hash(&mut data_hash);
            knot.value.to_bits().hash(&mut data_hash);
            knot.curve.to_bits().hash(&mut data_hash);
            knot.curve_x.to_bits().hash(&mut data_hash);
        }
    }
    if let Some(phase) = frame.editor.snap_phase {
        let x = egui::lerp(plot.left()..=plot.right(), phase);
        painter.line_segment(
            [egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())],
            egui::Stroke::new(1.0_f32, frame.color.gamma_multiply(0.32)),
        );
    }
    if let Some(value) = frame.editor.snap_value {
        let y = frame.geometry.position(0.0, value).y;
        painter.line_segment(
            [egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)],
            egui::Stroke::new(1.0_f32, frame.color.gamma_multiply(0.32)),
        );
    }
    let stroke = egui::Stroke::new((plot.height() * 0.014).clamp(1.25, 2.0), frame.color);
    let mesh = editor_widgets::cached_gradient_stroke_mesh(
        ui,
        response.id.with("curve-static-mesh"),
        (
            data_hash.finish(),
            [
                plot.min.x.to_bits(),
                plot.min.y.to_bits(),
                plot.width().to_bits(),
                plot.height().to_bits(),
            ],
            frame.geometry.bipolar(),
            painter.ctx().pixels_per_point().to_bits(),
            frame.color.to_array(),
        ),
        || {
            (0..=192)
                .map(|point| {
                    let phase = point as f32 / 192.0;
                    frame.geometry.position(
                        phase,
                        frame.data.map_or(0.0, |data| curve_value(data, phase)),
                    )
                })
                .collect()
        },
        baseline,
        frame.color,
        72,
        stroke,
    );
    painter.add(mesh);
    let playhead_x = egui::lerp(plot.left()..=plot.right(), frame.playhead_phase);
    painter.line_segment(
        [
            egui::pos2(playhead_x, plot.top()),
            egui::pos2(playhead_x, plot.bottom()),
        ],
        egui::Stroke::new(3.0_f32, frame.color.gamma_multiply(0.18)),
    );
    painter.line_segment(
        [
            egui::pos2(playhead_x, plot.top()),
            egui::pos2(playhead_x, plot.bottom()),
        ],
        egui::Stroke::new(1.0_f32, frame.color),
    );
    painter.circle_filled(
        egui::pos2(playhead_x, plot.top() + frame.point_radius * 0.5),
        frame.point_radius * 0.42,
        frame.color,
    );

    if let Some(data) = frame.data {
        paint_spline_handles(ui, painter, data, &frame);
    }
    if response.hovered() {
        let cursor = if frame.editor.drag.is_some() {
            egui::CursorIcon::Grabbing
        } else if frame.point_hit.is_some() {
            egui::CursorIcon::Grab
        } else if frame.handle_hit.is_some() {
            egui::CursorIcon::Grab
        } else {
            egui::CursorIcon::Crosshair
        };
        ui.output_mut(|output| output.cursor_icon = cursor);
    }
    response.clone().on_hover_text(
        "Drag points in X/Y; hold Alt to bypass nearby snaps. Drag a curve or its segment handle in X/Y to reshape its timing and bend; hold Shift for fine adjustment. Double-click empty space to add, a point to remove, or a bend to reset. Right-click for target-aware reset actions.",
    );
}

fn paint_spline_handles(
    ui: &egui::Ui,
    painter: &egui::Painter,
    data: &WaveCurveData,
    frame: &EditorCurvePaint<'_>,
) {
    let palette = editor_theme::semantic();
    let removing = ui.input(|input| input.pointer.button_down(egui::PointerButton::Secondary));
    for handle in segment_handles(data, frame.geometry, frame.point_radius) {
        let hovered = frame.handle_hit == Some(handle.index);
        let selected = frame.editor.selected == Some(SplineDrag::Tension(handle.index));
        let active = frame.editor.drag == Some(SplineDrag::Tension(handle.index));
        let radius = frame.point_radius
            * if active {
                1.0
            } else if selected {
                0.84
            } else if hovered {
                0.82
            } else {
                0.60
            };
        if hovered || selected || active {
            painter.circle_filled(
                handle.position,
                radius * 1.55,
                frame.color.gamma_multiply(0.14),
            );
        }
        painter.circle_filled(
            handle.position,
            radius,
            if active {
                frame.color
            } else if hovered {
                palette.control_hover
            } else {
                palette.well
            },
        );
        painter.circle_stroke(
            handle.position,
            radius,
            egui::Stroke::new(
                (frame.point_radius * 0.2).clamp(0.8, 1.25),
                frame
                    .color
                    .gamma_multiply(if active || selected || hovered {
                        0.9
                    } else {
                        0.48
                    }),
            ),
        );
        painter.line_segment(
            [
                handle.position - egui::vec2(0.0, radius * 0.5),
                handle.position + egui::vec2(0.0, radius * 0.5),
            ],
            egui::Stroke::new(
                (frame.point_radius * 0.14).clamp(0.7, 1.0),
                if active || selected || hovered {
                    frame.color
                } else {
                    palette.text_muted
                },
            ),
        );
        painter.line_segment(
            [
                handle.position - egui::vec2(radius * 0.5, 0.0),
                handle.position + egui::vec2(radius * 0.5, 0.0),
            ],
            egui::Stroke::new(
                (frame.point_radius * 0.14).clamp(0.7, 1.0),
                if active || selected || hovered {
                    frame.color
                } else {
                    palette.text_muted
                },
            ),
        );
    }
    for (index, knot) in data.knots.iter().enumerate() {
        let position = frame.geometry.position(knot.phase, knot.value);
        let hovered = frame.point_hit == Some(index);
        let selected = frame.editor.selected == Some(SplineDrag::Point(index));
        let active = frame.editor.drag == Some(SplineDrag::Point(index));
        let removing = hovered && removing;
        let radius = frame.point_radius
            * if active {
                1.16
            } else if selected {
                1.0
            } else if hovered {
                0.88
            } else {
                0.72
            };
        if active || selected || removing {
            painter.circle_stroke(
                position,
                radius * 1.45,
                egui::Stroke::new(
                    (frame.point_radius * 0.22).clamp(0.9, 1.4),
                    if removing {
                        palette.danger.gamma_multiply(0.72)
                    } else {
                        frame.color.gamma_multiply(0.52)
                    },
                ),
            );
        }
        painter.circle_filled(
            position,
            radius,
            if removing {
                palette.danger
            } else if active || selected || hovered {
                frame.color
            } else {
                palette.well
            },
        );
        painter.circle_stroke(
            position,
            radius,
            egui::Stroke::new(
                (frame.point_radius * 0.2).clamp(0.8, 1.25),
                if removing {
                    palette.text
                } else if active || selected || hovered {
                    palette.text
                } else {
                    frame.color
                },
            ),
        );
    }
}
