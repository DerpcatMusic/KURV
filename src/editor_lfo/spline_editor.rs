use super::*;

mod interaction;
mod painting;

use interaction::{SplineGeometry, nearest_knot};

#[derive(Clone, Copy)]
struct TensionDragOrigin {
    pointer: egui::Pos2,
    curve: f32,
    precision: f32,
}

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
    let point_radius = (rect.height() * 0.035).clamp(3.5, 6.0);
    let content_inset = (point_radius * 1.45 + editor_theme::shape::FOCUS_STROKE).max(graph_inset);
    let plot = rect.shrink(content_inset);
    let painter = ui.painter_at(rect);
    let geometry = SplineGeometry::new(plot, bipolar);
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
        painting::paint_source_drag_curve(&painter, geometry, compiled, source_color(index));
        return;
    }
    let editor_id = response.id.with("spline-editor");
    let tension_origin_id = editor_id.with("tension-drag-origin");
    let mut editor = ui
        .data(|store| store.get_temp::<SplineEditorUi>(editor_id))
        .unwrap_or_default();
    let mut draft_active = editor.draft.is_some();
    let mut data = curve.map(|curve| editor.draft.take().unwrap_or_else(|| curve.snapshot()));
    let pointer = response
        .interact_pointer_pos()
        .or_else(|| response.hover_pos());
    let grab_radius = (ui.spacing().interact_size.y * 0.55).max(point_radius * 2.5);
    let hit = editor.drag.or_else(|| {
        data.as_ref()
            .and_then(|data| geometry.nearest_target(data, pointer?, point_radius, grab_radius))
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
                        let (phase, value) = geometry.values_from_pos(pointer);
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
                if let SplineDrag::Tension(segment) = drag
                    && let Some(pointer) = response.interact_pointer_pos()
                {
                    ui.data_mut(|store| {
                        store.insert_temp(
                            tension_origin_id,
                            TensionDragOrigin {
                                pointer,
                                curve: data.knots[segment].curve,
                                precision: tension_precision(ui),
                            },
                        )
                    });
                }
                ui.ctx().set_dragged_id(response.id);
            }
        } else if response.clicked() {
            editor.selected = hit;
            editor.snap_phase = None;
            editor.snap_value = None;
        }

        let drag_aborted = editor.drag.is_some()
            && ui.input(|input| !input.focused || !input.pointer.primary_down());
        if !drag_aborted
            && response.dragged()
            && let (Some(drag), Some(pointer)) = (editor.drag, response.interact_pointer_pos())
        {
            let (pointer_phase, value) = geometry.values_from_pos(pointer);
            match drag {
                SplineDrag::Point(point) => {
                    let alt = ui.input(|input| input.modifiers.alt);
                    let (phase, value, snap_phase, snap_value) =
                        geometry.snap_point(pointer_phase, value, point_radius, alt);
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
                    let precision = tension_precision(ui);
                    let mut origin = ui
                        .data(|store| store.get_temp::<TensionDragOrigin>(tension_origin_id))
                        .unwrap_or(TensionDragOrigin {
                            pointer,
                            curve: data.knots[segment].curve,
                            precision,
                        });
                    if (origin.precision - precision).abs() > f32::EPSILON {
                        origin = TensionDragOrigin {
                            pointer,
                            curve: data.knots[segment].curve,
                            precision,
                        };
                    }
                    let curve = origin.curve
                        - (pointer.y - origin.pointer.y) / plot.height().max(1.0)
                            * 3.0
                            * precision
                            * tension_direction(data, segment);
                    set_segment_curve(data, segment, curve);
                    ui.data_mut(|store| store.insert_temp(tension_origin_id, origin));
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
            ui.data_mut(|store| store.remove::<TensionDragOrigin>(tension_origin_id));
        } else if drag_aborted {
            *data = curve.snapshot();
            editor.drag = None;
            draft_active = false;
            editor.snap_phase = None;
            editor.snap_value = None;
            ui.data_mut(|store| store.remove::<TensionDragOrigin>(tension_origin_id));
        }
        if editor.selected.is_some_and(|target| match target {
            SplineDrag::Point(point) | SplineDrag::Tension(point) => point >= data.knots.len(),
        }) {
            editor.selected = None;
        }
    }
    let color = source_color(index);
    let phase = lfo_phase_meter(state, index).clamp(0.0, 1.0);
    painting::paint_editor_curve(
        ui,
        &painter,
        &response,
        painting::EditorCurvePaint {
            data: data.as_ref(),
            geometry,
            color,
            hit,
            point_hit,
            handle_hit,
            editor: &editor,
            playhead_phase: phase,
            point_radius,
        },
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

fn tension_precision(ui: &egui::Ui) -> f32 {
    if ui.input(|input| input.modifiers.shift) {
        0.18
    } else {
        1.0
    }
}

fn tension_direction(data: &WaveCurveData, segment: usize) -> f32 {
    let Some(start) = data.knots.get(segment) else {
        return 1.0;
    };
    let Some(end) = data.knots.get((segment + 1) % data.knots.len()) else {
        return 1.0;
    };
    let direction = (end.value - start.value).signum();
    if direction == 0.0 { 1.0 } else { direction }
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
