use super::*;

const SOURCE_DRAG_POINTS: u8 = 64;

pub(super) fn draw_curve(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    index: usize,
    width: f32,
    height: f32,
) {
    let dynamic = index >= LEGACY_MODULATION_SOURCES;
    let dynamic_config = dynamic.then(|| state.params().modulator_rack.config(index));
    let bipolar = dynamic_config.map_or_else(
        || state.get_param(lfo_params(index).bipolar) >= 0.5,
        |config| config.bipolar,
    );
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(width, height),
        egui::Sense::CLICK | egui::Sense::DRAG,
    );
    let graph_inset = editor_theme::graph_inset(ui);
    let plot = rect.shrink2(egui::vec2(
        graph_inset,
        editor_theme::compact_gap(ui).min(graph_inset),
    ));
    let painter = ui.painter_at(rect);
    let curve = if dynamic {
        state.params().modulator_rack.curve(index)
    } else {
        Some(lfo_curve(state.params(), index))
    };
    if crate::editor_modulation::source_drag_active(ui) {
        let compiled = curve
            .and_then(WaveCurveState::try_curve_rt)
            .unwrap_or_else(|| {
                curve.map_or_else(WaveCurveRt::default, |curve| curve.snapshot().compile_rt())
            });
        let points = (0..=SOURCE_DRAG_POINTS)
            .map(|point| {
                let phase = f32::from(point) / f32::from(SOURCE_DRAG_POINTS);
                spline_pos(plot, phase, compiled.eval(phase), bipolar)
            })
            .collect();
        painter.add(egui::Shape::line(
            points,
            egui::Stroke::new(
                (plot.height() * 0.014).clamp(1.25, 2.0),
                source_color(index),
            ),
        ));
        return;
    }
    let editor_id = response.id.with("spline-editor");
    let mut editor = ui
        .data(|store| store.get_temp::<SplineEditorUi>(editor_id))
        .unwrap_or_default();
    let mut draft_active = editor.draft.is_some();
    let mut data = curve.map(|curve| editor.draft.take().unwrap_or_else(|| curve.snapshot()));
    let dragging = editor.drag.is_some();
    let mut compiled = data.as_ref().map_or_else(WaveCurveRt::default, |data| {
        if draft_active && !dragging {
            data.compile_rt()
        } else {
            curve
                .and_then(WaveCurveState::try_curve_rt)
                .unwrap_or_else(|| data.compile_rt())
        }
    });
    let pointer = response
        .interact_pointer_pos()
        .or_else(|| response.hover_pos());
    let point_radius = (plot.height() * 0.035).clamp(3.5, 6.0);
    let grab_radius = (ui.spacing().interact_size.y * 0.55).max(point_radius * 2.5);
    let hit = editor.drag.or_else(|| {
        data.as_ref().and_then(|data| {
            nearest_spline_target(
                data,
                compiled,
                plot,
                pointer?,
                bipolar,
                point_radius,
                grab_radius,
            )
        })
    });
    let mut point_hit = match hit {
        Some(SplineDrag::Point(point)) => Some(point),
        _ => None,
    };
    let mut handle_hit = match hit {
        Some(SplineDrag::Tension(handle)) => Some(handle),
        _ => None,
    };

    if let (Some(curve), Some(data)) = (curve, data.as_mut()) {
        if response.secondary_clicked() {
            editor.context_target = hit;
        }
        let context_target = editor.context_target.or(hit);
        let mut remove_point = None;
        let mut reset_segment = None;
        let mut reset_curve = false;
        response.context_menu(|ui| {
            if let Some(SplineDrag::Point(point)) = context_target {
                let removable = point > 0 && data.knots.len() > MIN_WAVE_KNOTS;
                let disabled_reason = if point == 0 {
                    "The first point anchors the cycle"
                } else {
                    "A curve needs at least three points"
                };
                if ui
                    .add_enabled(removable, egui::Button::new("REMOVE POINT"))
                    .on_disabled_hover_text(disabled_reason)
                    .clicked()
                {
                    remove_point = Some(point);
                    ui.close();
                }
            } else if let Some(SplineDrag::Tension(segment)) = context_target
                && ui.button("RESET BEND").clicked()
            {
                reset_segment = Some(segment);
                ui.close();
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
            editor.context_target = None;
        }
        if let Some(point) = remove_point {
            if remove_knot(data, point) {
                curve.replace(data.clone());
                editor.selected = None;
            }
            editor.drag = None;
            draft_active = false;
            editor.snap_phase = None;
            editor.snap_value = None;
        } else if let Some(segment) = reset_segment {
            if set_segment_curve(data, segment, 0.0) {
                curve.replace(data.clone());
                editor.selected = Some(SplineDrag::Tension(segment));
            }
            editor.drag = None;
            draft_active = false;
            editor.snap_phase = None;
            editor.snap_value = None;
        } else if reset_curve {
            *data = WaveCurveData::default();
            curve.replace(data.clone());
            editor.selected = None;
            editor.drag = None;
            draft_active = false;
            editor.snap_phase = None;
            editor.snap_value = None;
        } else if response.double_clicked() {
            match hit {
                Some(SplineDrag::Point(point)) => {
                    if remove_knot(data, point) {
                        curve.replace(data.clone());
                        editor.selected = None;
                    }
                }
                Some(SplineDrag::Tension(segment)) => {
                    if set_segment_curve(data, segment, 0.0) {
                        curve.replace(data.clone());
                        editor.selected = Some(SplineDrag::Tension(segment));
                    }
                }
                None => {
                    if let Some(pointer) = pointer {
                        let (phase, value) = spline_values_from_pos(plot, pointer, bipolar);
                        if insert_knot(data, phase, value) {
                            curve.replace(data.clone());
                            editor.selected = nearest_knot(data, phase).map(SplineDrag::Point);
                        }
                    }
                }
            }
            editor.drag = None;
            draft_active = false;
            editor.snap_phase = None;
            editor.snap_value = None;
        } else if response.is_pointer_button_down_on()
            && ui.input(|input| input.pointer.primary_pressed())
        {
            if let Some(drag) = hit {
                editor.selected = Some(drag);
                editor.drag = Some(drag);
                draft_active = true;
                ui.ctx().set_dragged_id(response.id);
            }
        } else if response.clicked() {
            editor.selected = hit;
            editor.snap_phase = None;
            editor.snap_value = None;
        }

        let drag_aborted = editor.drag.is_some()
            && ui.input(|input| !input.focused || !input.pointer.primary_down());
        let pointer_delta = ui.input(|input| input.pointer.delta());
        if !drag_aborted
            && editor.drag.is_some()
            && pointer_delta != egui::Vec2::ZERO
            && let (Some(drag), Some(pointer)) = (editor.drag, response.interact_pointer_pos())
        {
            let (pointer_phase, value) = spline_values_from_pos(plot, pointer, bipolar);
            match drag {
                SplineDrag::Point(point) => {
                    let alt = ui.input(|input| input.modifiers.alt);
                    let (phase, value, snap_phase, snap_value) =
                        snap_spline_point(plot, pointer_phase, value, bipolar, point_radius, alt);
                    move_knot(data, point, phase, value);
                    let moved = data.knots[point];
                    editor.snap_phase = snap_phase
                        .filter(|target| (moved.phase - target).abs() <= f32::EPSILON * 8.0);
                    editor.snap_value = snap_value
                        .filter(|target| (moved.value - target).abs() <= f32::EPSILON * 8.0);
                    point_hit = Some(point);
                    handle_hit = None;
                }
                SplineDrag::Tension(segment) => {
                    let precision = if ui.input(|input| input.modifiers.shift) {
                        0.18
                    } else {
                        1.0
                    };
                    let curve = data.knots[segment].curve
                        - pointer_delta.y / plot.height().max(1.0) * 3.0 * precision;
                    set_segment_curve(data, segment, curve);
                    editor.snap_phase = None;
                    editor.snap_value = None;
                    point_hit = None;
                    handle_hit = Some(segment);
                }
            }
            editor_theme::request_display_repaint(ui);
        }
        if response.drag_stopped() {
            if draft_active {
                curve.replace(data.clone());
                *data = curve.snapshot();
            }
            draft_active = false;
            editor.drag = None;
            editor.snap_phase = None;
            editor.snap_value = None;
        } else if drag_aborted {
            *data = curve.snapshot();
            editor.drag = None;
            draft_active = false;
            editor.snap_phase = None;
            editor.snap_value = None;
        }
        if editor.selected.is_some_and(|target| match target {
            SplineDrag::Point(point) | SplineDrag::Tension(point) => point >= data.knots.len(),
        }) {
            editor.selected = None;
        }
    }
    if let (Some(curve), Some(data)) = (curve, data.as_ref()) {
        compiled = if draft_active {
            data.compile_rt()
        } else {
            curve.try_curve_rt().unwrap_or_else(|| data.compile_rt())
        };
    }

    let baseline = if bipolar {
        plot.center().y
    } else {
        plot.bottom()
    };
    let points: Vec<_> = (0..=192)
        .map(|point| {
            let phase = point as f32 / 192.0;
            spline_pos(plot, phase, compiled.eval(phase), bipolar)
        })
        .collect();
    let color = source_color(index);
    if let Some(phase) = editor.snap_phase {
        let x = egui::lerp(plot.left()..=plot.right(), phase);
        painter.line_segment(
            [egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())],
            egui::Stroke::new(1.0_f32, color.gamma_multiply(0.32)),
        );
    }
    if let Some(value) = editor.snap_value {
        let y = spline_pos(plot, 0.0, value, bipolar).y;
        painter.line_segment(
            [egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)],
            egui::Stroke::new(1.0_f32, color.gamma_multiply(0.32)),
        );
    }
    editor_widgets::gradient_area_to_baseline(&painter, &points, baseline, color, 72);
    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new((plot.height() * 0.014).clamp(1.25, 2.0), color),
    ));
    let phase = lfo_phase_meter(state, index).clamp(0.0, 1.0);
    let playhead_x = egui::lerp(plot.left()..=plot.right(), phase);
    painter.line_segment(
        [
            egui::pos2(playhead_x, plot.top()),
            egui::pos2(playhead_x, plot.bottom()),
        ],
        egui::Stroke::new(3.0_f32, color.gamma_multiply(0.18)),
    );
    painter.line_segment(
        [
            egui::pos2(playhead_x, plot.top()),
            egui::pos2(playhead_x, plot.bottom()),
        ],
        egui::Stroke::new(1.0_f32, color),
    );
    painter.circle_filled(
        egui::pos2(playhead_x, plot.top() + point_radius * 0.5),
        point_radius * 0.42,
        color,
    );

    if let Some(data) = data.as_ref() {
        paint_spline_handles(
            ui,
            &painter,
            data,
            compiled,
            plot,
            bipolar,
            color,
            point_hit,
            handle_hit,
            editor.selected,
            editor.drag,
            point_radius,
        );
    }
    if response.hovered() {
        let hint = match editor.drag.or(hit).or(editor.selected) {
            Some(SplineDrag::Point(_)) => "POINT · DRAG X/Y",
            Some(SplineDrag::Tension(_)) => "BEND · DRAG Y · SHIFT FINE",
            None => "DOUBLE-CLICK · ADD POINT",
        };
        painter.text(
            plot.right_top() + egui::vec2(-editor_theme::space::XS, editor_theme::space::XXS),
            egui::Align2::RIGHT_TOP,
            hint,
            editor_theme::font::caption(),
            color.gamma_multiply(0.78),
        );
        let cursor = if editor.drag.is_some() {
            egui::CursorIcon::Grabbing
        } else if point_hit.is_some() {
            egui::CursorIcon::Grab
        } else if handle_hit.is_some() {
            egui::CursorIcon::ResizeVertical
        } else {
            egui::CursorIcon::Crosshair
        };
        ui.output_mut(|output| output.cursor_icon = cursor);
    }
    response.clone().on_hover_text(
        "Drag points in X/Y; hold Alt to bypass nearby snaps. Drag a curve or its segment handle vertically to bend; hold Shift for fine adjustment. Double-click empty space to add, a point to remove, or a bend to reset. Right-click for target-aware reset actions.",
    );
    let meter_moving = meter_is_moving(
        &mut editor.last_meter,
        &mut editor.meter_motion_frames,
        phase,
        true,
    );
    request_graph_repaint(ui, meter_moving);
    editor.draft = draft_active.then_some(data).flatten();
    ui.data_mut(|store| store.insert_temp(editor_id, editor));
}

pub(super) fn meter_is_moving(
    previous: &mut Option<f32>,
    motion_frames: &mut u8,
    value: f32,
    wraps: bool,
) -> bool {
    let changed = previous.is_some_and(|previous| {
        let delta = (value - previous).abs();
        let delta = if wraps { delta.min(1.0 - delta) } else { delta };
        delta > 0.000_5
    });
    *previous = Some(value);
    *motion_frames = if changed {
        2
    } else {
        (*motion_frames).saturating_sub(1)
    };
    *motion_frames > 0
}

pub(super) fn request_graph_repaint(ui: &egui::Ui, meter_moving: bool) {
    if crate::editor_modulation::source_drag_active(ui) {
        return;
    }
    if ui.ctx().dragged_id().is_some() {
        editor_theme::request_display_repaint(ui);
    } else {
        ui.ctx().request_repaint_after(if meter_moving {
            LIVE_METER_REPAINT
        } else {
            IDLE_METER_REPAINT
        });
    }
}

#[derive(Clone, Copy)]
struct SegmentHandle {
    index: usize,
    position: egui::Pos2,
}

fn segment_handles(
    data: &WaveCurveData,
    compiled: WaveCurveRt,
    plot: egui::Rect,
    bipolar: bool,
    point_radius: f32,
) -> impl Iterator<Item = SegmentHandle> + '_ {
    data.knots
        .iter()
        .enumerate()
        .filter_map(move |(index, knot)| {
            let end = data.knots.get(index + 1).map_or(1.0, |next| next.phase);
            let phase = (knot.phase + end) * 0.5;
            ((end - knot.phase) * plot.width() >= point_radius * 4.0).then(|| {
                let value = compiled.eval(phase);
                SegmentHandle {
                    index,
                    position: spline_pos(plot, phase, value, bipolar),
                }
            })
        })
}

fn nearest_spline_target(
    data: &WaveCurveData,
    compiled: WaveCurveRt,
    plot: egui::Rect,
    pointer: egui::Pos2,
    bipolar: bool,
    point_radius: f32,
    grab_radius: f32,
) -> Option<SplineDrag> {
    let grab_radius_sq = grab_radius.powi(2);
    let point = data
        .knots
        .iter()
        .enumerate()
        .map(|(index, knot)| {
            (
                SplineDrag::Point(index),
                spline_pos(plot, knot.phase, knot.value, bipolar).distance_sq(pointer),
            )
        })
        .min_by(|left, right| left.1.total_cmp(&right.1))
        .filter(|(_, distance)| *distance <= grab_radius_sq)
        .map(|(target, _)| target);
    if point.is_some() {
        return point;
    }

    let handle = segment_handles(data, compiled, plot, bipolar, point_radius)
        .map(|handle| {
            (
                SplineDrag::Tension(handle.index),
                handle.position.distance_sq(pointer),
            )
        })
        .min_by(|left, right| left.1.total_cmp(&right.1))
        .filter(|(_, distance)| *distance <= grab_radius_sq)
        .map(|(target, _)| target);
    if handle.is_some() {
        return handle;
    }

    let phase = ((pointer.x - plot.left()) / plot.width().max(1.0)).clamp(0.0, 1.0);
    let segment = data
        .knots
        .partition_point(|knot| knot.phase <= phase)
        .saturating_sub(1)
        .min(data.knots.len().saturating_sub(1));
    (spline_pos(plot, phase, compiled.eval(phase), bipolar).distance_sq(pointer) <= grab_radius_sq)
        .then_some(SplineDrag::Tension(segment))
}

fn snap_spline_point(
    plot: egui::Rect,
    phase: f32,
    value: f32,
    bipolar: bool,
    point_radius: f32,
    disabled: bool,
) -> (f32, f32, Option<f32>, Option<f32>) {
    if disabled {
        return (phase, value, None, None);
    }
    let proximity = point_radius * 1.5;
    let snap_phase = [0.25_f32, 0.5, 0.75]
        .into_iter()
        .map(|target| (target, (target - phase).abs() * plot.width()))
        .min_by(|left, right| left.1.total_cmp(&right.1))
        .filter(|(_, distance)| *distance <= proximity)
        .map(|(target, _)| target);
    let value_scale = plot.height() * if bipolar { 0.42 } else { 0.45 };
    let snap_value = [-1.0_f32, -0.5, 0.0, 0.5, 1.0]
        .into_iter()
        .map(|target| (target, (target - value).abs() * value_scale))
        .min_by(|left, right| left.1.total_cmp(&right.1))
        .filter(|(_, distance)| *distance <= proximity)
        .map(|(target, _)| target);
    (
        snap_phase.unwrap_or(phase),
        snap_value.unwrap_or(value),
        snap_phase,
        snap_value,
    )
}

#[allow(clippy::too_many_arguments)]
fn paint_spline_handles(
    ui: &egui::Ui,
    painter: &egui::Painter,
    data: &WaveCurveData,
    compiled: WaveCurveRt,
    plot: egui::Rect,
    bipolar: bool,
    color: egui::Color32,
    hovered_point: Option<usize>,
    hovered_handle: Option<usize>,
    selected: Option<SplineDrag>,
    active_drag: Option<SplineDrag>,
    point_radius: f32,
) {
    let palette = editor_theme::semantic();
    let removing = ui.input(|input| input.pointer.button_down(egui::PointerButton::Secondary));
    for handle in segment_handles(data, compiled, plot, bipolar, point_radius) {
        let hovered = hovered_handle == Some(handle.index);
        let selected = selected == Some(SplineDrag::Tension(handle.index));
        let active = active_drag == Some(SplineDrag::Tension(handle.index));
        let radius = point_radius
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
            painter.circle_filled(handle.position, radius * 1.55, color.gamma_multiply(0.14));
        }
        painter.circle_filled(
            handle.position,
            radius,
            if active {
                color
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
                (point_radius * 0.2).clamp(0.8, 1.25),
                color.gamma_multiply(if active || selected || hovered {
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
                (point_radius * 0.14).clamp(0.7, 1.0),
                if active || selected || hovered {
                    color
                } else {
                    palette.text_muted
                },
            ),
        );
    }
    for (index, knot) in data.knots.iter().enumerate() {
        let position = spline_pos(plot, knot.phase, knot.value, bipolar);
        let hovered = hovered_point == Some(index);
        let selected = selected == Some(SplineDrag::Point(index));
        let active = active_drag == Some(SplineDrag::Point(index));
        let removing = hovered && removing;
        let radius = point_radius
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
                    (point_radius * 0.22).clamp(0.9, 1.4),
                    if removing {
                        palette.danger.gamma_multiply(0.72)
                    } else {
                        color.gamma_multiply(0.52)
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
                color
            } else {
                palette.well
            },
        );
        painter.circle_stroke(
            position,
            radius,
            egui::Stroke::new(
                (point_radius * 0.2).clamp(0.8, 1.25),
                if removing {
                    palette.text
                } else if active || selected || hovered {
                    palette.text
                } else {
                    color
                },
            ),
        );
    }
}

fn nearest_knot(data: &WaveCurveData, phase: f32) -> Option<usize> {
    data.knots
        .iter()
        .enumerate()
        .min_by(|(_, left), (_, right)| {
            (left.phase - phase)
                .abs()
                .total_cmp(&(right.phase - phase).abs())
        })
        .map(|(index, _)| index)
}

fn spline_pos(plot: egui::Rect, phase: f32, value: f32, bipolar: bool) -> egui::Pos2 {
    let y = if bipolar {
        (-value * plot.height() * 0.42).mul_add(1.0, plot.center().y)
    } else {
        plot.bottom() - value.mul_add(0.5, 0.5) * plot.height() * 0.9
    };
    egui::pos2(phase.mul_add(plot.width(), plot.left()), y)
}

fn spline_values_from_pos(plot: egui::Rect, position: egui::Pos2, bipolar: bool) -> (f32, f32) {
    let phase = ((position.x - plot.left()) / plot.width()).clamp(0.0, 1.0);
    let value = if bipolar {
        (plot.center().y - position.y) / (plot.height() * 0.42)
    } else {
        ((plot.bottom() - position.y) / (plot.height() * 0.9)).mul_add(2.0, -1.0)
    }
    .clamp(-1.0, 1.0);
    (phase, value)
}

pub(super) fn draw_in_rect(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    id_salt: impl std::hash::Hash,
    add: impl FnOnce(&mut egui::Ui),
) {
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(id_salt)
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    child.spacing_mut().item_spacing = egui::Vec2::ZERO;
    add(&mut child);
}
