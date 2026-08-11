//! Direct VA waveform curve editing and interaction geometry.

mod painting;

use crate::editor_theme;
use crate::oscillators::VaTableState;
use crate::wave_curve::{
    WaveCurveData, WaveCurveRt, curve_x_from_handle_progress, fit_freehand_curve, insert_knot,
    move_knot, remove_knot, segment_handle_phase, segment_handle_progress, set_segment_bend,
};

#[derive(Clone, Default)]
pub(super) struct FreehandStroke {
    points: Vec<(f32, f32)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum CurveDragTarget {
    Knot(usize),
    Segment(usize),
}

#[allow(clippy::too_many_arguments)]
pub(super) fn edit_wave_curve_target(
    ui: &egui::Ui,
    response: &egui::Response,
    plot: egui::Rect,
    table: &VaTableState,
    frame: usize,
    oscillator: usize,
    color: egui::Color32,
    bipolar: bool,
) {
    let drag_id = response.id.with(("wave-curve-drag", oscillator));
    let stroke_id = response.id.with(("wave-curve-stroke", oscillator));
    let draft_id = response.id.with(("wave-curve-draft", oscillator));
    let selection_id = response
        .id
        .with(("wave-curve-selection", oscillator, frame));
    let mut data = if let Some(draft) = ui.data(|store| store.get_temp(draft_id)) {
        draft
    } else {
        let Some(data) = table.frame_snapshot(frame) else {
            return;
        };
        data
    };
    let drag_pointer = response.interact_pointer_pos();
    let pointer = drag_pointer.filter(|pointer| plot.contains(*pointer));
    let hit_curve = data.compile_rt();
    let hit =
        pointer.and_then(|pointer| hit_curve_target(&data, &hit_curve, plot, pointer, bipolar));
    let knot_hit = match hit {
        Some(CurveDragTarget::Knot(index)) => Some(index),
        _ => None,
    };
    let curve_hit = match hit {
        Some(CurveDragTarget::Segment(index)) => Some(index),
        _ => None,
    };

    if response.double_clicked() {
        if let Some(index) = curve_hit {
            let _ = table.edit_frame(frame, |data| set_segment_bend(data, index, 0.0, 0.0));
            data = table.frame_snapshot(frame).unwrap_or_default();
        } else if knot_hit.is_none()
            && let Some(pointer) = pointer
        {
            let (phase, value) = values_from_pos(plot, pointer, bipolar);
            let _ = table.edit_frame(frame, |data| insert_knot(data, phase, value));
            data = table.frame_snapshot(frame).unwrap_or_default();
        }
    } else if response.secondary_clicked() {
        if let Some(index) = knot_hit {
            let _ = table.edit_frame(frame, |data| remove_knot(data, index));
            data = table.frame_snapshot(frame).unwrap_or_default();
            ui.data_mut(|store| store.remove::<CurveDragTarget>(selection_id));
        } else if let Some(index) = curve_hit {
            let _ = table.edit_frame(frame, |data| set_segment_bend(data, index, 0.0, 0.0));
            data = table.frame_snapshot(frame).unwrap_or_default();
        }
    } else if response.drag_started() {
        if let Some(index) = knot_hit {
            ui.data_mut(|store| {
                store.insert_temp(drag_id, CurveDragTarget::Knot(index));
                store.insert_temp(selection_id, CurveDragTarget::Knot(index));
                store.insert_temp(draft_id, data.clone());
            });
        } else if let Some(index) = curve_hit {
            ui.data_mut(|store| {
                store.insert_temp(drag_id, CurveDragTarget::Segment(index));
                store.insert_temp(selection_id, CurveDragTarget::Segment(index));
                store.insert_temp(draft_id, data.clone());
            });
        } else if let Some(pointer) = pointer {
            ui.data_mut(|store| {
                store.insert_temp(
                    stroke_id,
                    FreehandStroke {
                        points: vec![values_from_pos(plot, pointer, bipolar)],
                    },
                );
            });
        }
    }

    if response.dragged()
        && let Some(pointer) = drag_pointer
    {
        let point = values_from_pos(plot, pointer, bipolar);
        if let Some(drag) = ui.data(|store| store.get_temp::<CurveDragTarget>(drag_id)) {
            if let Some(mut draft) = ui.data(|store| store.get_temp::<WaveCurveData>(draft_id)) {
                match drag {
                    CurveDragTarget::Knot(index) => {
                        let snap = !ui.input(|input| input.modifiers.alt);
                        let (phase, value) = if snap {
                            snap_curve_point(point, plot)
                        } else {
                            point
                        };
                        move_knot(&mut draft, index, phase, value);
                    }
                    CurveDragTarget::Segment(index) => {
                        let precision = if ui.input(|input| input.modifiers.shift) {
                            0.25
                        } else {
                            1.0
                        };
                        let knot = draft.knots[index];
                        let motion = response.drag_motion();
                        let end = draft.knots.get(index + 1).map_or(1.0, |next| next.phase);
                        let end_value = draft
                            .knots
                            .get((index + 1) % draft.knots.len())
                            .map_or(knot.value, |next| next.value);
                        let direction = (end_value - knot.value).signum();
                        let curve = if direction == 0.0 {
                            knot.curve
                        } else {
                            knot.curve
                                - motion.y / plot.height().max(1.0) * 3.0 * precision * direction
                        };
                        let segment_pixels =
                            (end - knot.phase).max(f32::EPSILON) * plot.width().max(1.0);
                        let handle = segment_handle_progress(knot.curve_x)
                            + motion.x / segment_pixels.max(1.0) * precision;
                        let curve_x = curve_x_from_handle_progress(handle);
                        set_segment_bend(&mut draft, index, curve, curve_x);
                    }
                }
                data = draft.clone();
                ui.data_mut(|store| store.insert_temp(draft_id, draft));
            }
        } else if let Some(mut stroke) =
            ui.data_mut(|store| store.remove_temp::<FreehandStroke>(stroke_id))
        {
            if stroke.points.last().is_none_or(|last| {
                (last.0 - point.0).abs() > 0.001 || (last.1 - point.1).abs() > 0.002
            }) {
                stroke.points.push(point);
            }
            ui.data_mut(|store| store.insert_temp(stroke_id, stroke));
        }
        editor_theme::request_display_repaint(ui);
    }
    if response.drag_stopped() {
        let draft = ui.data_mut(|store| {
            let draft = store.remove_temp::<WaveCurveData>(draft_id);
            store.remove::<CurveDragTarget>(drag_id);
            draft
        });
        if let Some(draft) = draft {
            let _ = table.replace_frame(frame, draft);
            data = table.frame_snapshot(frame).unwrap_or_default();
        } else if let Some(stroke) =
            ui.data_mut(|store| store.remove_temp::<FreehandStroke>(stroke_id))
            && stroke.points.len() >= 2
        {
            let _ = table.replace_frame(frame, fit_freehand_curve(&data, &stroke.points));
            data = table.frame_snapshot(frame).unwrap_or_default();
        }
    }

    if let Some(stroke) = ui.data_mut(|store| store.remove_temp::<FreehandStroke>(stroke_id)) {
        let points = stroke
            .points
            .iter()
            .map(|(phase, value)| value_pos(plot, *phase, *value, bipolar))
            .collect();
        painting::paint_freehand_stroke(ui.painter(), points, color);
        ui.data_mut(|store| store.insert_temp(stroke_id, stroke));
    }

    let painted_curve = data.compile_rt();
    let hovered =
        pointer.and_then(|pointer| hit_curve_target(&data, &painted_curve, plot, pointer, bipolar));
    if response.clicked() || response.double_clicked() {
        ui.data_mut(|store| {
            if let Some(target) = hovered {
                store.insert_temp(selection_id, target);
            } else {
                store.remove::<CurveDragTarget>(selection_id);
            }
        });
    }
    let selected = ui
        .data(|store| store.get_temp::<CurveDragTarget>(selection_id))
        .filter(|target| match *target {
            CurveDragTarget::Knot(index) | CurveDragTarget::Segment(index) => {
                index < data.knots.len()
            }
        });
    if selected.is_none() {
        ui.data_mut(|store| store.remove::<CurveDragTarget>(selection_id));
    }
    let knot_hit = match hovered {
        Some(CurveDragTarget::Knot(index)) => Some(index),
        _ => None,
    };
    let curve_hit = match hovered {
        Some(CurveDragTarget::Segment(index)) => Some(index),
        _ => None,
    };
    let active = ui.data(|store| store.get_temp::<CurveDragTarget>(drag_id));
    let drawing = ui
        .data(|store| store.get_temp::<FreehandStroke>(stroke_id))
        .is_some();
    if pointer.is_some() {
        ui.output_mut(|output| {
            output.cursor_icon = if active.is_some() || drawing {
                egui::CursorIcon::Grabbing
            } else if knot_hit.is_some() || curve_hit.is_some() {
                egui::CursorIcon::Grab
            } else {
                egui::CursorIcon::Crosshair
            };
        });
    }
    if response.hovered() || active.is_some() || selected.is_some() {
        painting::paint_curve_edit_overlay(
            ui.painter(),
            &data,
            &painted_curve,
            plot,
            bipolar,
            color,
            active,
            hovered,
            selected,
            drawing,
            pointer,
        );
    }
    response.clone().on_hover_text(if knot_hit.is_some() {
        "Drag this point to reshape the cycle. Right-click to remove it."
    } else if curve_hit.is_some() {
        "Drag in X/Y to reshape this segment's timing and bend. Double-click or right-click to reset it."
    } else {
        "Drag to draw a cycle. Double-click to add a point. Hold Alt to bypass snapping."
    });
}

fn snap_curve_point((phase, value): (f32, f32), plot: egui::Rect) -> (f32, f32) {
    let radius_x = (editor_theme::space::XS / plot.width().max(1.0)).clamp(0.008, 0.04);
    let radius_y = (editor_theme::space::XS / plot.height().max(1.0)).clamp(0.015, 0.08);
    let phase_step = 1.0 / 16.0;
    let value_step = 0.25;
    let snapped_phase = (phase / phase_step).round() * phase_step;
    let snapped_value = (value / value_step).round() * value_step;
    (
        if (phase - snapped_phase).abs() <= radius_x {
            snapped_phase.clamp(0.0, 1.0)
        } else {
            phase
        },
        if (value - snapped_value).abs() <= radius_y {
            snapped_value.clamp(-1.0, 1.0)
        } else {
            value
        },
    )
}

pub(super) fn hit_curve_target(
    data: &WaveCurveData,
    curve: &WaveCurveRt,
    plot: egui::Rect,
    pointer: egui::Pos2,
    bipolar: bool,
) -> Option<CurveDragTarget> {
    let knot_radius = (plot.height() * 0.065).clamp(
        editor_theme::space::SM + editor_theme::space::XXS,
        editor_theme::space::LG,
    );
    if let Some((index, _)) = data
        .knots
        .iter()
        .enumerate()
        .map(|(index, knot)| (index, knot_pos(plot, *knot, bipolar).distance_sq(pointer)))
        .filter(|(_, distance)| *distance <= knot_radius * knot_radius)
        .min_by(|left, right| left.1.total_cmp(&right.1))
    {
        return Some(CurveDragTarget::Knot(index));
    }

    let segment_radius = (plot.height() * 0.055).clamp(
        editor_theme::space::SM,
        editor_theme::space::MD + editor_theme::space::XXS,
    );
    (0..data.knots.len())
        .filter_map(|index| {
            let distance = curve_segment_distance_sq(data, curve, index, plot, pointer, bipolar)?;
            (distance <= segment_radius * segment_radius).then_some((index, distance))
        })
        .min_by(|left, right| left.1.total_cmp(&right.1))
        .map(|(index, _)| CurveDragTarget::Segment(index))
}

fn curve_segment_distance_sq(
    data: &WaveCurveData,
    curve: &WaveCurveRt,
    index: usize,
    plot: egui::Rect,
    pointer: egui::Pos2,
    bipolar: bool,
) -> Option<f32> {
    let knot = data.knots.get(index)?;
    let start = knot.phase;
    let end = data.knots.get(index + 1).map_or(1.0, |next| next.phase);
    let steps = 16;
    let mut previous = value_pos(plot, start, curve.eval(start), bipolar);
    let mut nearest = f32::INFINITY;
    for step in 1..=steps {
        let phase = (end - start).mul_add(step as f32 / steps as f32, start);
        let current = value_pos(plot, phase, curve.eval(phase), bipolar);
        nearest = nearest.min(distance_to_segment_sq(pointer, previous, current));
        previous = current;
    }
    Some(nearest)
}

fn distance_to_segment_sq(point: egui::Pos2, start: egui::Pos2, end: egui::Pos2) -> f32 {
    let segment = end - start;
    let length_sq = segment.length_sq();
    if length_sq <= f32::EPSILON {
        return point.distance_sq(start);
    }
    let position = ((point - start).dot(segment) / length_sq).clamp(0.0, 1.0);
    point.distance_sq(start + segment * position)
}

fn curve_handle_pos(
    data: &WaveCurveData,
    curve: &WaveCurveRt,
    index: usize,
    plot: egui::Rect,
    bipolar: bool,
) -> egui::Pos2 {
    let phase = segment_handle_phase(data, index).unwrap_or(data.knots[index].phase);
    value_pos(plot, phase, curve.eval(phase), bipolar)
}

fn knot_pos(plot: egui::Rect, knot: crate::wave_curve::WaveKnot, bipolar: bool) -> egui::Pos2 {
    value_pos(plot, knot.phase, knot.value, bipolar)
}

fn value_pos(plot: egui::Rect, phase: f32, value: f32, bipolar: bool) -> egui::Pos2 {
    let y = if bipolar {
        (-value * plot.height() * 0.42).mul_add(1.0, plot.center().y)
    } else {
        plot.bottom() - value.mul_add(0.5, 0.5) * plot.height() * 0.9
    };
    egui::pos2(phase.mul_add(plot.width(), plot.left()), y)
}

fn values_from_pos(plot: egui::Rect, position: egui::Pos2, bipolar: bool) -> (f32, f32) {
    let phase = ((position.x - plot.left()) / plot.width()).clamp(0.0, 1.0);
    let value = if bipolar {
        (plot.center().y - position.y) / (plot.height() * 0.42)
    } else {
        ((plot.bottom() - position.y) / (plot.height() * 0.9)).mul_add(2.0, -1.0)
    }
    .clamp(-1.0, 1.0);
    (phase, value)
}
