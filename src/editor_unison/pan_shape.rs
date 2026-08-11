//! Direct pan-shape curve editing, painting, and geometry.

use crate::pan_curve::{
    PanShapeCurveData, PanShapeCurveState, PanShapeKnot, insert_knot, move_center, move_endpoint,
    move_knot, remove_knot, set_segment_curve,
};
use crate::{editor_theme, editor_widgets};

const CURVE_POINTS: u16 = 96;

#[derive(Clone)]
struct PanShapePointDrag {
    target: PanShapePointDragTarget,
    anchor: egui::Pos2,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PanShapePointDragTarget {
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
        .on_hover_text("Drag points or curve handles; double-click to add, right-click to remove");
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

    if active.is_none()
        && response.double_clicked_by(egui::PointerButton::Primary)
        && let Some(pointer) = pointer.filter(|pointer| plot.contains(*pointer))
        && hovered_target.is_none()
    {
        let (left, input, output) = pan_shape_values_from_pos(plot, *center_x, pointer);
        let mirror = ui.input(|input| input.modifiers.shift);
        changed = curve_state.edit(|curve| {
            let mut candidate = curve.clone();
            if !insert_knot(candidate.half_mut(left), input, output)
                || (mirror && !insert_knot(candidate.half_mut(!left), input, output))
            {
                return false;
            }
            *curve = candidate;
            true
        });
        if changed {
            data = curve_state.snapshot();
        }
    }

    if active.is_none()
        && response.clicked_by(egui::PointerButton::Secondary)
        && let Some(PanShapePointDragTarget::Knot { left, index }) = hovered_target
    {
        let mirror = ui.input(|input| input.modifiers.shift);
        let mirror_index = mirror
            .then(|| matching_knot_index(data.half(!left), data.half(left).knots[index].in_lin));
        changed |= curve_state.edit(|curve| {
            let mut candidate = curve.clone();
            if !remove_knot(candidate.half_mut(left), index)
                || (mirror
                    && !mirror_index
                        .flatten()
                        .is_some_and(|index| remove_knot(candidate.half_mut(!left), index)))
            {
                return false;
            }
            *curve = candidate;
            true
        });
        if changed {
            data = curve_state.snapshot();
        }
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
        let (pointer, mirror) = ui.input(|input| {
            (
                constrain_drag(
                    drag.anchor,
                    pointer,
                    input.modifiers.alt
                        && !matches!(drag.target, PanShapePointDragTarget::Endpoint { .. }),
                ),
                input.modifiers.shift,
            )
        });
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
                if mirror {
                    move_endpoint(curve.half_mut(!left), output);
                }
            }
            PanShapePointDragTarget::Knot { left, index } => {
                let (input, output) = pan_shape_values_from_side(plot, *center_x, left, pointer);
                let mirror_index = mirror.then(|| {
                    matching_knot_index(curve.half(!left), curve.half(left).knots[index].in_lin)
                });
                move_knot(curve.half_mut(left), index, input, output);
                if let Some(Some(index)) = mirror_index {
                    move_knot(curve.half_mut(!left), index, input, output);
                }
            }
            PanShapePointDragTarget::Curve { left, index } => {
                let (input, output) = pan_shape_values_from_side(plot, *center_x, left, pointer);
                let mirror_index = mirror.then(|| {
                    let half = curve.half(left);
                    matching_segment_index(
                        curve.half(!left),
                        (half.knots[index].in_lin + half.knots[index + 1].in_lin) * 0.5,
                    )
                });
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
                if let Some(Some(index)) = mirror_index {
                    set_segment_curve(curve.half_mut(!left), index, vertical, horizontal);
                }
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
        active,
        response.hovered(),
        handle_radius,
    );
    (changed, response)
}

fn matching_knot_index(half: &crate::pan_curve::PanShapeHalf, input: f32) -> Option<usize> {
    half.knots
        .iter()
        .enumerate()
        .skip(1)
        .take(half.knots.len().saturating_sub(2))
        .min_by(|(_, left), (_, right)| {
            (left.in_lin - input)
                .abs()
                .total_cmp(&(right.in_lin - input).abs())
        })
        .map(|(index, _)| index)
}

fn matching_segment_index(half: &crate::pan_curve::PanShapeHalf, input: f32) -> Option<usize> {
    half.knots
        .windows(2)
        .enumerate()
        .min_by(|(_, left), (_, right)| {
            let left = ((left[0].in_lin + left[1].in_lin) * 0.5 - input).abs();
            let right = ((right[0].in_lin + right[1].in_lin) * 0.5 - input).abs();
            left.total_cmp(&right)
        })
        .map(|(index, _)| index)
}

fn draw_pan_shape_curve(
    painter: &egui::Painter,
    plot: egui::Rect,
    center_x: f32,
    data: &PanShapeCurveData,
    hovered: Option<PanShapePointDragTarget>,
    drag: Option<PanShapePointDrag>,
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
        let center_active = drag
            .as_ref()
            .is_some_and(|drag| matches!(drag.target, PanShapePointDragTarget::Center));
        let center_hovered = hovered == Some(PanShapePointDragTarget::Center);
        let endpoint_active = drag.as_ref().is_some_and(|drag| {
            matches!(drag.target, PanShapePointDragTarget::Endpoint { left: side } if side == left)
        });
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
            let knot_active = drag.as_ref().is_some_and(|drag| {
                matches!(drag.target, PanShapePointDragTarget::Knot { left: side, index: target } if side == left && target == index)
            });
            let knot_hovered = hovered == Some(PanShapePointDragTarget::Knot { left, index });
            draw_shape_handle(
                painter,
                position,
                color,
                knot_hovered,
                knot_active,
                false,
                handle_radius,
            );
        }

        for index in 0..half.knots.len().saturating_sub(1) {
            let curve = pan_shape_curve_handle_pos(plot, center_x, left, half, index);
            let curve_active = drag.as_ref().is_some_and(|drag| {
                matches!(drag.target, PanShapePointDragTarget::Curve { left: side, index: target } if side == left && target == index)
            });
            let curve_hovered = hovered == Some(PanShapePointDragTarget::Curve { left, index });
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

fn pan_shape_curve_handle_pos(
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

fn pan_shape_knot_pos(
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
