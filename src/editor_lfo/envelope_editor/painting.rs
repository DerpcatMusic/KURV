use super::{
    interaction::{envelope_curve_for_stage, envelope_path, envelope_segment, envelope_stage_path},
    *,
};

pub(super) fn paint_source_drag_curve(
    ui: &egui::Ui,
    painter: &egui::Painter,
    cache_id: egui::Id,
    points: &[egui::Pos2; 5],
    curves: [f32; 3],
    plot: egui::Rect,
    color: egui::Color32,
) {
    let key = (
        points.map(|point| [point.x.to_bits(), point.y.to_bits()]),
        curves.map(f32::to_bits),
        painter.ctx().pixels_per_point().to_bits(),
        color.to_array(),
    );
    let mesh = editor_widgets::cached_stroke_mesh(
        ui,
        cache_id,
        key,
        || envelope_path(points, curves),
        egui::Stroke::new((plot.height() * 0.014).clamp(1.25, 2.0), color),
    );
    painter.add(mesh);
}

pub(super) struct EnvelopeCurvePaint<'a> {
    pub(super) points: &'a [egui::Pos2; 5],
    pub(super) curves: [f32; 3],
    pub(super) handles: &'a [(EnvelopeDrag, egui::Pos2)],
    pub(super) plot: egui::Rect,
    pub(super) color: egui::Color32,
    pub(super) hovered: Option<EnvelopeDrag>,
    pub(super) editor: &'a EnvelopeEditorUi,
    pub(super) handle_radius: f32,
}

pub(super) fn paint_editor_curve(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    response: &egui::Response,
    frame: EnvelopeCurvePaint<'_>,
) {
    let stroke = egui::Stroke::new((frame.plot.height() * 0.014).clamp(1.25, 2.0), frame.color);
    let mesh = editor_widgets::cached_gradient_stroke_mesh(
        ui,
        response.id.with("envelope-static-mesh"),
        (
            frame
                .points
                .map(|point| [point.x.to_bits(), point.y.to_bits()]),
            frame.curves.map(f32::to_bits),
            painter.ctx().pixels_per_point().to_bits(),
            frame.color.to_array(),
        ),
        || envelope_path(frame.points, frame.curves),
        frame.plot.bottom(),
        frame.color,
        64,
        stroke,
    );
    painter.add(mesh);
    if let Some(stage) = frame
        .editor
        .drag
        .or(frame.hovered)
        .or(frame.editor.selected)
    {
        let (start, end) = envelope_segment(frame.points, stage);
        painter.add(egui::Shape::line(
            envelope_stage_path(start, end, envelope_curve_for_stage(frame.curves, stage)),
            egui::Stroke::new(
                (frame.plot.height() * 0.05).clamp(3.0, 5.0),
                frame
                    .color
                    .gamma_multiply(if frame.editor.drag == Some(stage) {
                        0.28
                    } else {
                        0.14
                    }),
            ),
        ));
    }
    for &(stage, position) in frame.handles {
        let active = frame.editor.drag == Some(stage);
        let hot = active || frame.hovered == Some(stage);
        let selected = frame.editor.selected == Some(stage);
        let curve_handle = matches!(
            stage,
            EnvelopeDrag::AttackCurve | EnvelopeDrag::DecayCurve | EnvelopeDrag::ReleaseCurve
        );
        let handle_radius = frame.handle_radius * if curve_handle { 0.90 } else { 1.0 };
        if hot {
            painter.circle_filled(
                position,
                handle_radius * 1.55,
                frame.color.gamma_multiply(0.18),
            );
        }
        if selected {
            painter.circle_stroke(
                position,
                handle_radius * 1.25,
                egui::Stroke::new(
                    (handle_radius * 0.22).clamp(0.9, 1.35),
                    frame.color.gamma_multiply(0.58),
                ),
            );
        }
        painter.circle_filled(
            position,
            if active {
                handle_radius * 1.08
            } else if hot || selected {
                handle_radius * 0.86
            } else {
                handle_radius * 0.68
            },
            if active {
                editor_theme::semantic().text
            } else {
                editor_theme::semantic().well
            },
        );
        painter.circle_stroke(
            position,
            if active {
                handle_radius * 1.08
            } else if hot || selected {
                handle_radius * 0.86
            } else {
                handle_radius * 0.68
            },
            egui::Stroke::new(
                (handle_radius * 0.2).clamp(0.8, 1.2),
                if active {
                    editor_theme::semantic().text
                } else {
                    frame.color
                },
            ),
        );
        if hot || selected {
            let label = match stage {
                EnvelopeDrag::Attack => "A",
                EnvelopeDrag::AttackCurve => "A CURVE",
                EnvelopeDrag::DecaySustain => "D/S",
                EnvelopeDrag::DecayCurve => "D CURVE",
                EnvelopeDrag::Release => "R",
                EnvelopeDrag::ReleaseCurve => "R CURVE",
            };
            let label_y = if position.y - frame.plot.top()
                < editor_theme::font::CAPTION_SIZE + handle_radius * 2.0
            {
                position.y + handle_radius * 1.4
            } else {
                position.y - handle_radius * 1.4
            };
            painter.text(
                egui::pos2(position.x, label_y),
                if label_y > position.y {
                    egui::Align2::CENTER_TOP
                } else {
                    egui::Align2::CENTER_BOTTOM
                },
                label,
                editor_theme::font::caption(),
                if active {
                    editor_theme::semantic().text
                } else {
                    frame.color
                },
            );
        }
    }
    if response.hovered() {
        ui.output_mut(|output| {
            output.cursor_icon = match frame.editor.drag.or(frame.hovered) {
                Some(_) if frame.editor.drag.is_some() => egui::CursorIcon::Grabbing,
                Some(EnvelopeDrag::Attack | EnvelopeDrag::Release) => {
                    egui::CursorIcon::ResizeHorizontal
                }
                Some(EnvelopeDrag::DecaySustain) => egui::CursorIcon::ResizeNwSe,
                Some(
                    EnvelopeDrag::AttackCurve
                    | EnvelopeDrag::DecayCurve
                    | EnvelopeDrag::ReleaseCurve,
                ) => egui::CursorIcon::ResizeVertical,
                None => egui::CursorIcon::Default,
            };
        });
    }
    response.clone().on_hover_text(
        "Drag ADSR points or segments; drag midpoint handles vertically to bend stages. Hold Shift for fine adjustment. Double-click a stage or bend to reset it; right-click to reset the envelope.",
    );
}

pub(super) fn paint_meter(
    painter: &egui::Painter,
    plot: egui::Rect,
    value: f32,
    color: egui::Color32,
) {
    painter.circle_filled(
        egui::pos2(plot.right(), egui::lerp(plot.bottom()..=plot.top(), value)),
        (plot.height() * 0.025).max(2.0),
        color,
    );
}
