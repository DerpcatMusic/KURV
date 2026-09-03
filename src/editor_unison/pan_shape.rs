//! Direct pan-shape curve editing and geometry.

mod painting;

use crate::editor_theme;
use crate::pan_curve::{
    PanShapeCurveData, PanShapeCurveState, PanShapeKnot, insert_knot, move_center, move_endpoint,
    move_knot, remove_knot, set_segment_curve,
};

use painting::draw_pan_shape_curve;

#[derive(Clone)]
struct PanShapePointDrag {
    target: PanShapePointDragTarget,
    anchor: egui::Pos2,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum PanShapePointDragTarget {
    Center,
    Endpoint { left: bool },
    Knot { left: bool, index: usize },
    Curve { left: bool, index: usize },
}

pub(super) fn constrain_drag(anchor: egui::Pos2, pointer: egui::Pos2, enabled: bool) -> egui::Pos2 {
    if !enabled {
        return pointer;
    }
    let delta = pointer - anchor;
    let diagonal = std::f32::consts::FRAC_1_SQRT_2;
    [
        (1.0, 0.0),
        (0.0, 1.0),
        (diagonal, diagonal),
        (diagonal, -diagonal),
    ]
    .into_iter()
    .map(|(x, y)| {
        let direction = egui::vec2(x, y);
        let projected = anchor + direction * delta.dot(direction);
        (projected.distance_sq(pointer), projected)
    })
    .min_by(|left, right| left.0.total_cmp(&right.0))
    .map_or(pointer, |(_, projected)| projected)
}

pub(super) fn custom_pan_shape_curve_view(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    rect: egui::Rect,
    id: egui::Id,
    curve_state: &PanShapeCurveState,
    center_x: &mut f32,
) -> (bool, egui::Response) {
    let plot = rect.shrink2(egui::vec2(
        editor_theme::space::XXS.min(rect.width() * 0.035),
        editor_theme::space::XXS.min(rect.height() * 0.04),
    ));
    let response = ui.interact(plot, id, egui::Sense::CLICK | egui::Sense::DRAG);
    response
        .clone()
        .on_hover_text("Drag points or bends; Shift moves finely. Double-click empty space to add or a point/bend to make its outgoing segment linear. Right-click for remove and reset actions.");
    let hit_radius = editor_theme::title_height(ui) * 0.52;
    let handle_radius = editor_theme::font::CAPTION_SIZE * 0.38;
    let pointer = response
        .interact_pointer_pos()
        .or_else(|| response.hover_pos());
    let drag_id = id.with("point-drag");
    let mut data = curve_state.snapshot();
    let mut active = ui.data(|store| store.get_temp::<PanShapePointDrag>(drag_id));
    let mut hovered_target = pointer
        .and_then(|pointer| pan_shape_hit_target(&data, plot, *center_x, pointer, hit_radius));
    let mut changed = false;

    if active.is_none() && response.double_clicked_by(egui::PointerButton::Primary) {
        changed = match hovered_target {
            Some(PanShapePointDragTarget::Center) => curve_state.edit(|curve| {
                set_segment_curve(&mut curve.left, 0, 0.0, 0.0)
                    | set_segment_curve(&mut curve.right, 0, 0.0, 0.0)
            }),
            Some(PanShapePointDragTarget::Knot { left, index })
            | Some(PanShapePointDragTarget::Curve { left, index }) => {
                curve_state.edit(|curve| set_segment_curve(curve.half_mut(left), index, 0.0, 0.0))
            }
            Some(PanShapePointDragTarget::Endpoint { .. }) => false,
            None => pointer
                .filter(|pointer| plot.contains(*pointer))
                .is_some_and(|pointer| {
                    let (left, input, output) = pan_shape_values_from_pos(plot, *center_x, pointer);
                    curve_state.edit(|curve| insert_knot(curve.half_mut(left), input, output))
                }),
        };
        if changed {
            data = curve_state.snapshot();
        }
    }

    let context_id = id.with("context-target");
    if response.secondary_clicked() {
        ui.data_mut(|store| {
            if let Some(target) = hovered_target {
                store.insert_temp(context_id, target);
            } else {
                store.remove::<PanShapePointDragTarget>(context_id);
            }
        });
    }
    let context_target = ui.data(|store| store.get_temp::<PanShapePointDragTarget>(context_id));
    let mut remove = false;
    let mut reset_segment = false;
    let mut reset_curve = false;
    response.context_menu(|ui| {
        if matches!(context_target, Some(PanShapePointDragTarget::Knot { .. }))
            && ui.button("REMOVE POINT").clicked()
        {
            remove = true;
            ui.close();
        }
        if let Some(
            target @ (PanShapePointDragTarget::Knot { .. } | PanShapePointDragTarget::Curve { .. }),
        ) = context_target
        {
            let label = if matches!(target, PanShapePointDragTarget::Knot { .. }) {
                "RESET SEGMENT"
            } else {
                "RESET BEND"
            };
            if ui.button(label).clicked() {
                reset_segment = true;
                ui.close();
            }
        }
        if context_target.is_some() {
            ui.separator();
        }
        if ui.button("RESET CURVE").clicked() {
            reset_curve = true;
            ui.close();
        }
    });
    if !response.context_menu_opened() {
        ui.data_mut(|store| store.remove::<PanShapePointDragTarget>(context_id));
    }
    changed |= match context_target {
        Some(PanShapePointDragTarget::Knot { left, index }) if remove => {
            curve_state.edit(|curve| remove_knot(curve.half_mut(left), index))
        }
        Some(PanShapePointDragTarget::Knot { left, index }) if reset_segment => {
            curve_state.edit(|curve| set_segment_curve(curve.half_mut(left), index, 0.0, 0.0))
        }
        Some(PanShapePointDragTarget::Curve { left, index }) if reset_segment => {
            curve_state.edit(|curve| set_segment_curve(curve.half_mut(left), index, 0.0, 0.0))
        }
        _ if reset_curve => {
            curve_state.replace(PanShapeCurveData::default());
            true
        }
        _ => false,
    };
    if changed {
        data = curve_state.snapshot();
    }

    if active.is_none()
        && response.drag_started_by(egui::PointerButton::Primary)
        && let Some(target) = hovered_target
    {
        let drag = PanShapePointDrag {
            target,
            anchor: pan_shape_target_pos(&data, plot, *center_x, target),
        };
        ui.data_mut(|store| store.insert_temp(drag_id, drag.clone()));
        active = Some(drag);
    }

    if let Some(drag) = active.as_ref()
        && response.dragged_by(egui::PointerButton::Primary)
        && let Some(pointer) = pointer
    {
        let pointer = if ui.input(|input| input.modifiers.shift) {
            drag.anchor + (pointer - drag.anchor) * 0.18
        } else {
            pointer
        };
        let target = drag.target;
        curve_state.edit(|curve| match target {
            PanShapePointDragTarget::Center => {
                let (_, _, output) = pan_shape_values_from_pos(plot, *center_x, pointer);
                let normalized_x =
                    ((pointer.x - plot.left()) / plot.width().max(1.0)).clamp(0.0, 1.0);
                move_center(curve, output);
                *center_x = normalized_x.mul_add(0.9, 0.05);
            }
            PanShapePointDragTarget::Endpoint { left } => {
                let (_, output) = pan_shape_values_from_side(plot, *center_x, left, pointer);
                move_endpoint(curve.half_mut(left), output);
            }
            PanShapePointDragTarget::Knot { left, index } => {
                let (input, output) = pan_shape_values_from_side(plot, *center_x, left, pointer);
                move_knot(curve.half_mut(left), index, input, output);
            }
            PanShapePointDragTarget::Curve { left, index } => {
                let (input, output) = pan_shape_values_from_side(plot, *center_x, left, pointer);
                let half = curve.half_mut(left);
                let start = half.knots[index].out_lin;
                let end = half.knots[index + 1].out_lin;
                let segment_start = half.knots[index].in_lin;
                let segment_end = half.knots[index + 1].in_lin;
                let vertical = if (end - start).abs() > f32::EPSILON {
                    ((output - start) / (end - start)).clamp(0.0, 1.0)
                } else {
                    0.5
                }
                .mul_add(2.0, -1.0);
                let horizontal = ((((input - segment_start)
                    / (segment_end - segment_start).max(f32::EPSILON))
                .clamp(0.0, 1.0)
                    - 0.5)
                    / 0.44)
                    .clamp(-1.0, 1.0);
                set_segment_curve(half, index, vertical, horizontal);
            }
        });
        data = curve_state.snapshot();
        changed = true;
        editor_theme::request_display_repaint(ui);
    }

    if response.drag_stopped_by(egui::PointerButton::Primary) {
        ui.data_mut(|store| store.remove::<PanShapePointDrag>(drag_id));
        active = None;
    }
    hovered_target = pointer
        .and_then(|pointer| pan_shape_hit_target(&data, plot, *center_x, pointer, hit_radius));
    if response.hovered() {
        ui.output_mut(|output| {
            output.cursor_icon = if active.is_some() {
                egui::CursorIcon::Grabbing
            } else if hovered_target.is_some() {
                egui::CursorIcon::Grab
            } else {
                egui::CursorIcon::Crosshair
            };
        });
    }
    draw_pan_shape_curve(
        painter,
        plot,
        *center_x,
        &data,
        hovered_target,
        active.as_ref().map(|drag| drag.target),
        response.hovered(),
        handle_radius,
    );
    (changed, response)
}

pub(super) fn pan_shape_curve_handle_pos(
    plot: egui::Rect,
    center_x: f32,
    left: bool,
    half: &crate::pan_curve::PanShapeHalf,
    index: usize,
) -> egui::Pos2 {
    let Some(start) = half.knots.get(index).copied() else {
        return plot.center();
    };
    let Some(end) = half.knots.get(index + 1).copied() else {
        return pan_shape_knot_pos(plot, center_x, left, start);
    };
    let segments = half.compile_rt();
    let y = segments.seg_p1[index].clamp(0.0, 1.0);
    let start = pan_shape_knot_pos(plot, center_x, left, start);
    let end = pan_shape_knot_pos(plot, center_x, left, end);
    egui::pos2(
        egui::lerp(start.x..=end.x, segments.seg_cx1[index].clamp(0.0, 1.0)),
        egui::lerp(start.y..=end.y, y),
    )
}

pub(super) fn pan_shape_knot_pos(
    plot: egui::Rect,
    center_x: f32,
    left: bool,
    knot: PanShapeKnot,
) -> egui::Pos2 {
    let center = egui::lerp(plot.left()..=plot.right(), center_x.clamp(0.05, 0.95));
    let x = if left {
        egui::lerp(center..=plot.left(), knot.in_lin)
    } else {
        egui::lerp(center..=plot.right(), knot.in_lin)
    };
    egui::pos2(x, egui::lerp(plot.bottom()..=plot.top(), knot.out_lin))
}

fn pan_shape_endpoint(
    data: &PanShapeCurveData,
    plot: egui::Rect,
    center_x: f32,
    left: bool,
) -> egui::Pos2 {
    let knot = data.half(left).knots.last().copied().unwrap_or_default();
    pan_shape_knot_pos(plot, center_x, left, knot)
}

fn pan_shape_target_pos(
    data: &PanShapeCurveData,
    plot: egui::Rect,
    center_x: f32,
    target: PanShapePointDragTarget,
) -> egui::Pos2 {
    match target {
        PanShapePointDragTarget::Center => data
            .left
            .knots
            .first()
            .copied()
            .map_or(plot.center(), |knot| {
                pan_shape_knot_pos(plot, center_x, true, knot)
            }),
        PanShapePointDragTarget::Endpoint { left } => {
            pan_shape_endpoint(data, plot, center_x, left)
        }
        PanShapePointDragTarget::Knot { left, index } => data
            .half(left)
            .knots
            .get(index)
            .copied()
            .map_or(plot.center(), |knot| {
                pan_shape_knot_pos(plot, center_x, left, knot)
            }),
        PanShapePointDragTarget::Curve { left, index } => {
            pan_shape_curve_handle_pos(plot, center_x, left, data.half(left), index)
        }
    }
}

fn pan_shape_hit_target(
    data: &PanShapeCurveData,
    plot: egui::Rect,
    center_x: f32,
    pointer: egui::Pos2,
    radius: f32,
) -> Option<PanShapePointDragTarget> {
    let mut nearest = None;
    let mut consider = |target, position: egui::Pos2| {
        let distance = pointer.distance_sq(position);
        if distance <= radius * radius && nearest.as_ref().is_none_or(|(best, _)| distance < *best)
        {
            nearest = Some((distance, target));
        }
    };

    if let Some(center) = data.left.knots.first().copied() {
        consider(
            PanShapePointDragTarget::Center,
            pan_shape_knot_pos(plot, center_x, true, center),
        );
    }
    for left in [true, false] {
        consider(
            PanShapePointDragTarget::Endpoint { left },
            pan_shape_endpoint(data, plot, center_x, left),
        );
        let half = data.half(left);
        for (index, knot) in half
            .knots
            .iter()
            .copied()
            .enumerate()
            .skip(1)
            .take(half.knots.len().saturating_sub(2))
        {
            consider(
                PanShapePointDragTarget::Knot { left, index },
                pan_shape_knot_pos(plot, center_x, left, knot),
            );
        }
        for index in 0..half.knots.len().saturating_sub(1) {
            consider(
                PanShapePointDragTarget::Curve { left, index },
                pan_shape_curve_handle_pos(plot, center_x, left, half, index),
            );
        }
    }
    nearest.map(|(_, target)| target)
}

fn pan_shape_values_from_side(
    plot: egui::Rect,
    center_x: f32,
    left: bool,
    pointer: egui::Pos2,
) -> (f32, f32) {
    let center = egui::lerp(plot.left()..=plot.right(), center_x.clamp(0.05, 0.95));
    let input = if left {
        (center - pointer.x) / (center - plot.left()).max(1.0)
    } else {
        (pointer.x - center) / (plot.right() - center).max(1.0)
    };
    let output = (plot.bottom() - pointer.y) / plot.height().max(1.0);
    (input.clamp(0.0, 1.0), output.clamp(0.0, 1.0))
}

fn pan_shape_values_from_pos(
    plot: egui::Rect,
    center_x: f32,
    pointer: egui::Pos2,
) -> (bool, f32, f32) {
    let center = egui::lerp(plot.left()..=plot.right(), center_x.clamp(0.05, 0.95));
    let left = pointer.x < center;
    let (input, output) = pan_shape_values_from_side(plot, center_x, left, pointer);
    (left, input, output)
}
