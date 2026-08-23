//! Direct VA waveform curve editing and interaction geometry.

mod painting;

use crate::editor_theme;
use crate::oscillators::VaTableState;
use crate::wave_curve::{
    WaveCurveData, WaveCurveRt, curve_x_from_handle_progress, insert_knot, move_knot,
    segment_handle_phase, segment_handle_progress, set_segment_bend,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CurveDragTarget {
    Knot(usize),
    Segment(usize),
}

#[derive(Clone, Default)]
struct CurveDraft {
    frame: usize,
    data: WaveCurveData,
}

pub(super) fn clear_edit_state(ui: &egui::Ui, response_id: egui::Id, oscillator: usize) {
    ui.data_mut(|store| {
        store.remove::<CurveDragTarget>(response_id.with(("wave-curve-drag", oscillator)));
        store.remove::<CurveDraft>(response_id.with(("wave-curve-draft", oscillator)));
        for frame in 0..crate::oscillators::MAX_VA_TABLE_FRAMES {
            store.remove::<CurveDragTarget>(response_id.with((
                "wave-curve-selection",
                oscillator,
                frame,
            )));
        }
    });
}

#[allow(clippy::too_many_arguments)]
pub(super) fn edit_wave_curve_target(
    ui: &egui::Ui,
    response: &egui::Response,
    plot: egui::Rect,
    table: &VaTableState,
    frame: usize,
    display_override: Option<&WaveCurveData>,
    oscillator: usize,
    color: egui::Color32,
    bipolar: bool,
) {
    let drag_id = response.id.with(("wave-curve-drag", oscillator));
    let draft_id = response.id.with(("wave-curve-draft", oscillator));
    let selection_id = response
        .id
        .with(("wave-curve-selection", oscillator, frame));
    let mut data = if let Some(draft) = ui.data(|store| store.get_temp::<CurveDraft>(draft_id)) {
        draft.data
    } else if let Some(data) = display_override {
        data.clone()
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
    if response.double_clicked() {
        let changed = match hit {
            Some(CurveDragTarget::Segment(index)) => table
                .edit_frame(frame, |data| set_segment_bend(data, index, 0.0, 0.0))
                .is_some(),
            Some(CurveDragTarget::Knot(_)) => false,
            None => pointer.is_some_and(|pointer| {
                let (phase, value) = values_from_pos(plot, pointer, bipolar);
                table
                    .edit_frame(frame, |data| insert_knot(data, phase, value))
                    .is_some()
            }),
        };
        if changed {
            crate::editor_shell::request_structural_commit(ui);
        }
        data = table.frame_snapshot(frame).unwrap_or_default();
    } else if response.drag_started()
        && let Some(target) = hit
    {
        ui.data_mut(|store| {
            store.insert_temp(drag_id, target);
            store.insert_temp(selection_id, target);
            store.insert_temp(
                draft_id,
                CurveDraft {
                    frame,
                    data: data.clone(),
                },
            );
        });
    }

    if response.dragged()
        && let Some(pointer) = drag_pointer
        && let Some(drag) = ui.data(|store| store.get_temp::<CurveDragTarget>(drag_id))
        && let Some(mut draft) = ui.data(|store| store.get_temp::<CurveDraft>(draft_id))
    {
        let point = values_from_pos(plot, pointer, bipolar);
        match drag {
            CurveDragTarget::Knot(index) => {
                let snap = !ui.input(|input| input.modifiers.alt);
                let (phase, value) = if snap {
                    snap_curve_point(point, plot)
                } else {
                    point
                };
                move_knot(&mut draft.data, index, phase, value);
            }
            CurveDragTarget::Segment(index) => {
                let precision = if ui.input(|input| input.modifiers.shift) {
                    0.25
                } else {
                    1.0
                };
                let knot = draft.data.knots[index];
                let motion = response.drag_motion();
                let end = draft
                    .data
                    .knots
                    .get(index + 1)
                    .map_or(1.0, |next| next.phase);
                let end_value = draft
                    .data
                    .knots
                    .get((index + 1) % draft.data.knots.len())
                    .map_or(knot.value, |next| next.value);
                let direction = (end_value - knot.value).signum();
                let curve = if direction == 0.0 {
                    knot.curve
                } else {
                    knot.curve - motion.y / plot.height().max(1.0) * 3.0 * precision * direction
                };
                let segment_pixels = (end - knot.phase).max(f32::EPSILON) * plot.width().max(1.0);
                let handle = segment_handle_progress(knot.curve_x)
                    + motion.x / segment_pixels.max(1.0) * precision;
                set_segment_bend(
                    &mut draft.data,
                    index,
                    curve,
                    curve_x_from_handle_progress(handle),
                );
            }
        }
        data = draft.data.clone();
        ui.data_mut(|store| store.insert_temp(draft_id, draft));
        editor_theme::request_display_repaint(ui);
    }

    let gesture_ended = response.drag_stopped()
        || (crate::editor_controls::pointer_gesture_aborted(ui)
            && !response.is_pointer_button_down_on());
    if gesture_ended {
        let draft = ui.data_mut(|store| {
            let draft = store.remove_temp::<CurveDraft>(draft_id);
            store.remove::<CurveDragTarget>(drag_id);
            draft
        });
        if let Some(draft) = draft {
            data = commit_curve_draft(table, draft);
            crate::editor_shell::request_structural_commit(ui);
        }
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
    let active = ui.data(|store| store.get_temp::<CurveDragTarget>(drag_id));
    if pointer.is_some() {
        ui.output_mut(|output| {
            output.cursor_icon = if active.is_some() {
                egui::CursorIcon::Grabbing
            } else if hovered.is_some() {
                egui::CursorIcon::Grab
            } else {
                egui::CursorIcon::Crosshair
            };
        });
    }
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
        pointer,
    );
}

fn commit_curve_draft(table: &VaTableState, draft: CurveDraft) -> WaveCurveData {
    let frame = draft.frame;
    let _ = table.replace_frame(frame, draft.data);
    table.frame_snapshot(frame).unwrap_or_default()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_snapping_uses_the_spline_grid() {
        let plot = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(320.0, 160.0));
        let snapped = snap_curve_point((0.249, 0.51), plot);
        assert!((snapped.0 - 0.25).abs() < f32::EPSILON);
        assert!((snapped.1 - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn curve_draft_commits_to_its_originating_frame() {
        let table = VaTableState::new();
        let mut first = WaveCurveData::default();
        first.knots[0].value = 0.1;
        let mut second = WaveCurveData::default();
        second.knots[0].value = 0.2;
        table.replace(crate::oscillators::VaTableData {
            frames: vec![first, second.clone()],
            positions: Vec::new(),
        });
        let mut edited = WaveCurveData::default();
        edited.knots[0].value = 0.9;

        commit_curve_draft(
            &table,
            CurveDraft {
                frame: 0,
                data: edited.clone(),
            },
        );

        assert_eq!(table.frame_snapshot(0), Some(edited.sanitized()));
        assert_eq!(table.frame_snapshot(1), Some(second.sanitized()));
    }

    #[test]
    fn empty_space_is_not_a_freehand_edit_target() {
        let plot = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(320.0, 160.0));
        let data = WaveCurveData::default();
        let curve = data.compile_rt();
        let pointer = egui::pos2(plot.center().x, plot.top());
        assert!(hit_curve_target(&data, &curve, plot, pointer, true).is_none());
    }
}
