use super::*;
use crate::wave_curve::MAX_VERTICAL_CURVE;

mod interaction;
mod painting;

use interaction::{SplineGeometry, curve_value, nearest_knot, segment_curve_for_value};

const FINE_DRAG_SCALE: f32 = 0.18;
const SINE_ARC_CURVE: f32 = 0.83;

#[derive(Clone, Copy)]
struct TensionDragOrigin {
    pointer: egui::Pos2,
    handle: egui::Pos2,
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
    let (_rect, response) = ui.allocate_exact_size(
        egui::vec2(width, height),
        egui::Sense::CLICK | egui::Sense::DRAG,
    );
    let curve = if dynamic {
        state.params().modulator_rack.curve(index)
    } else {
        Some(lfo_curve(state.params(), index))
    };
    let running = source_is_running(state, index);
    let phase_offset = dynamic_config.map_or_else(
        || state.get_param(lfo_params(index).phase).clamp(0.0, 1.0),
        |config| config.phase_offset,
    );
    draw_curve_state_impl(
        ui,
        curve,
        &response,
        bipolar,
        source_color(index),
        &WaveCurveData::default(),
        running.then(|| lfo_phase_meter(state, index).clamp(0.0, 1.0)),
        phase_offset,
    );
}

pub(super) fn draw_random_preview(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    index: usize,
    smooth: bool,
    width: f32,
    height: f32,
) {
    let (_, response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let config = source_config(state, index);
    let plot = response.rect.shrink(editor_theme::graph_inset(ui));
    let geometry = SplineGeometry::new(plot, config.bipolar);
    let color = source_color(index);
    let seed = crate::modulators::lfo::random_seed_for_source(index);
    let running = source_is_running(state, index);
    let phase = if running {
        lfo_phase_meter(state, index).clamp(0.0, 1.0)
    } else {
        config.phase_offset
    };
    if running {
        editor_theme::request_display_repaint(ui);
    }
    let points = plot.width().ceil().clamp(192.0, 512.0) as usize;
    let mesh = editor_widgets::cached_gradient_stroke_mesh(
        ui,
        response.id.with("random-preview"),
        (
            smooth,
            phase.to_bits(),
            config.bipolar,
            [
                plot.min.x.to_bits(),
                plot.min.y.to_bits(),
                plot.width().to_bits(),
                plot.height().to_bits(),
            ],
            ui.ctx().pixels_per_point().to_bits(),
            color.to_array(),
        ),
        || {
            (0..=points)
                .map(|point| {
                    let display_phase = point as f32 / points as f32;
                    let timeline = display_phase.mul_add(8.0, phase);
                    let cycle = timeline.floor() as i64;
                    let phase = timeline.fract();
                    let start = crate::modulators::lfo::seeded_random(seed, cycle);
                    let value = if smooth {
                        let end = crate::modulators::lfo::seeded_random(seed, cycle + 1);
                        let progress = phase * phase * (3.0 - 2.0 * phase);
                        (end - start).mul_add(progress, start)
                    } else {
                        start
                    };
                    geometry.position_display(display_phase, value)
                })
                .collect()
        },
        if config.bipolar {
            plot.center().y
        } else {
            plot.bottom()
        },
        color,
        72,
        egui::Stroke::new((plot.height() * 0.014).clamp(1.25, 2.0), color),
    );
    ui.painter().add(mesh);
    response.on_hover_text(if smooth {
        "Deterministic random-smooth output across eight cycles"
    } else {
        "Deterministic random-hold output across eight cycles"
    });
}

pub(crate) fn draw_curve_state_in_rect(
    ui: &mut egui::Ui,
    curve: &WaveCurveState,
    rect: egui::Rect,
    id_salt: impl std::hash::Hash,
    color: egui::Color32,
    default: &WaveCurveData,
) {
    let response = ui.interact(
        rect,
        ui.id().with(id_salt),
        egui::Sense::CLICK | egui::Sense::DRAG,
    );
    draw_curve_state_impl(ui, Some(curve), &response, false, color, default, None, 0.0);
}

pub(crate) fn edit_curve_data_in_rect(
    ui: &mut egui::Ui,
    data: &mut WaveCurveData,
    rect: egui::Rect,
    id_salt: impl std::hash::Hash,
    bipolar: bool,
    color: egui::Color32,
) -> bool {
    let curve = WaveCurveState::with_data(data.clone());
    let response = ui.interact(
        rect,
        ui.id().with(id_salt),
        egui::Sense::CLICK | egui::Sense::DRAG,
    );
    draw_curve_state_impl(
        ui,
        Some(&curve),
        &response,
        bipolar,
        color,
        &WaveCurveData::default(),
        None,
        0.0,
    );
    let edited = curve.snapshot();
    if edited == *data {
        false
    } else {
        *data = edited;
        true
    }
}

pub(crate) fn clear_curve_data_edit_state(ui: &egui::Ui, id_salt: impl std::hash::Hash) {
    let editor_id = ui.id().with(id_salt).with("spline-editor");
    ui.data_mut(|store| {
        store.remove::<SplineEditorUi>(editor_id);
        store.remove::<TensionDragOrigin>(editor_id.with("tension-drag-origin"));
    });
}

fn draw_curve_state_impl(
    ui: &mut egui::Ui,
    curve: Option<&WaveCurveState>,
    response: &egui::Response,
    bipolar: bool,
    color: egui::Color32,
    default: &WaveCurveData,
    playhead: Option<f32>,
    phase_offset: f32,
) {
    let rect = response.rect;
    let graph_inset = editor_theme::graph_inset(ui);
    let point_radius = (rect.height() * 0.035).clamp(3.5, 6.0);
    let content_inset = (point_radius * 1.45 + editor_theme::shape::FOCUS_STROKE).max(graph_inset);
    let plot = rect.shrink(content_inset);
    let painter = ui.painter_at(rect);
    let geometry = SplineGeometry::new(plot, bipolar).with_phase_offset(phase_offset);
    if crate::editor_modulation::source_drag_active(ui) {
        let generation = curve.map_or(0, WaveCurveState::history_generation);
        let compiled = curve
            .and_then(WaveCurveState::try_curve_rt)
            .unwrap_or_default();
        painting::paint_source_drag_curve(
            ui,
            &painter,
            response.id.with("source-drag-curve"),
            geometry,
            generation,
            compiled,
            color,
        );
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
                if ui.button("RESET SEGMENT").clicked() {
                    reset_segment = Some(point);
                    ui.close();
                }
            } else if let Some(SplineDrag::Tension(segment)) = context_target {
                if ui.button("RESET BEND").clicked() {
                    reset_segment = Some(segment);
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
            editor.context_target = None;
        }
        if let Some(point) = remove_point {
            if remove_knot(data, point) {
                curve.replace(data.clone());
                editor.selected = None;
            }
            editor.drag = None;
            editor.drag_origin = None;
            editor.last_publish_time = 0.0;
            draft_active = false;
            editor.snap_phase = None;
            editor.snap_value = None;
        } else if let Some(segment) = reset_segment {
            if set_segment_bend(data, segment, 0.0, 0.0) {
                curve.replace(data.clone());
                editor.selected = Some(SplineDrag::Tension(segment));
            }
            editor.drag = None;
            editor.drag_origin = None;
            editor.last_publish_time = 0.0;
            draft_active = false;
            editor.snap_phase = None;
            editor.snap_value = None;
        } else if reset_curve {
            *data = default.clone();
            curve.replace(data.clone());
            editor.selected = None;
            editor.drag = None;
            editor.drag_origin = None;
            editor.last_publish_time = 0.0;
            draft_active = false;
            editor.snap_phase = None;
            editor.snap_value = None;
        } else if response.double_clicked() {
            match hit {
                Some(SplineDrag::Point(point)) => {
                    if set_segment_bend(data, point, 0.0, 0.0) {
                        curve.replace(data.clone());
                        editor.selected = Some(SplineDrag::Point(point));
                    }
                }
                Some(SplineDrag::Tension(segment)) => {
                    if set_segment_bend(data, segment, 0.0, 0.0) {
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
            editor.drag_origin = None;
            editor.last_publish_time = 0.0;
            draft_active = false;
            editor.snap_phase = None;
            editor.snap_value = None;
        } else if response.is_pointer_button_down_on()
            && ui.input(|input| input.pointer.primary_pressed())
        {
            if let Some(drag) = hit {
                editor.selected = Some(drag);
                editor.drag = Some(drag);
                editor.drag_origin = Some(data.clone());
                draft_active = true;
                if let SplineDrag::Tension(segment) = drag
                    && let Some(pointer) = response.interact_pointer_pos()
                {
                    let precision = tension_precision(ui);
                    ui.data_mut(|store| {
                        store.insert_temp(
                            tension_origin_id,
                            TensionDragOrigin {
                                pointer,
                                handle: tension_handle_position(data, segment, geometry)
                                    .unwrap_or(pointer),
                                precision,
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
        let mut draft_changed = false;
        if !drag_aborted
            && response.dragged()
            && let (Some(drag), Some(pointer)) = (editor.drag, response.interact_pointer_pos())
        {
            let (pointer_phase, value) = geometry.values_from_pos(pointer);
            match drag {
                SplineDrag::Point(point) => {
                    let (fine, alt) =
                        ui.input(|input| (input.modifiers.shift, input.modifiers.alt));
                    let origin = editor
                        .drag_origin
                        .as_ref()
                        .and_then(|origin| origin.knots.get(point))
                        .map_or((pointer_phase, value), |knot| (knot.phase, knot.value));
                    let (phase, value) = point_drag_target(
                        origin,
                        (pointer_phase, value),
                        if fine { FINE_DRAG_SCALE } else { 1.0 },
                    );
                    let (phase, value, snap_phase, snap_value) =
                        geometry.snap_point(phase, value, point_radius, alt);
                    draft_changed = move_knot(data, point, phase, value);
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
                            handle: tension_handle_position(data, segment, geometry)
                                .unwrap_or(pointer),
                            precision,
                        });
                    if (origin.precision - precision).abs() > f32::EPSILON {
                        origin = TensionDragOrigin {
                            pointer,
                            handle: tension_handle_position(data, segment, geometry)
                                .unwrap_or(pointer),
                            precision,
                        };
                    }
                    let target = origin.handle + (pointer - origin.pointer) * precision;
                    let (curve, curve_x) = if ui.input(|input| input.modifiers.ctrl) {
                        snapped_tension_target(data, segment, geometry, target)
                    } else {
                        tension_pointer_target(data, segment, geometry, target)
                    };
                    draft_changed = set_segment_bend(data, segment, curve, curve_x);
                    ui.data_mut(|store| store.insert_temp(tension_origin_id, origin));
                    editor.snap_phase = None;
                    editor.snap_value = None;
                    point_hit = None;
                    handle_hit = Some(segment);
                }
            }
            editor_theme::request_display_repaint(ui);
        }
        if draft_changed {
            let now = ui.input(|input| input.time);
            if now - editor.last_publish_time >= SPLINE_EDIT_PUBLISH_INTERVAL_SECONDS {
                curve.replace(data.clone());
                editor.last_publish_time = now;
            }
        }
        if response.drag_stopped() {
            if draft_active {
                curve.replace(data.clone());
                *data = curve.snapshot();
            }
            editor.last_publish_time = 0.0;
            draft_active = false;
            editor.drag = None;
            editor.drag_origin = None;
            editor.snap_phase = None;
            editor.snap_value = None;
            ui.data_mut(|store| store.remove::<TensionDragOrigin>(tension_origin_id));
        } else if drag_aborted {
            if let Some(origin) = editor.drag_origin.take() {
                curve.replace(origin.clone());
                *data = origin;
            } else {
                *data = curve.snapshot();
            }
            editor.last_publish_time = 0.0;
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
    painting::paint_editor_curve(
        ui,
        &painter,
        &response,
        painting::EditorCurvePaint {
            data: data.as_ref(),
            geometry,
            color,
            point_hit,
            handle_hit,
            editor: &editor,
            playhead_phase: playhead,
            point_radius,
        },
    );
    let meter_moving = playhead.is_some_and(|phase| {
        meter_is_moving(
            &mut editor.last_meter,
            &mut editor.meter_motion_frames,
            phase,
            true,
        )
    });
    request_graph_repaint(ui, meter_moving);
    editor.draft = draft_active.then_some(data).flatten();
    ui.data_mut(|store| store.insert_temp(editor_id, editor));
}

fn point_drag_target(origin: (f32, f32), pointer_target: (f32, f32), precision: f32) -> (f32, f32) {
    (
        (pointer_target.0 - origin.0).mul_add(precision, origin.0),
        (pointer_target.1 - origin.1).mul_add(precision, origin.1),
    )
}

fn tension_precision(ui: &egui::Ui) -> f32 {
    if ui.input(|input| input.modifiers.shift) {
        FINE_DRAG_SCALE
    } else {
        1.0
    }
}

fn tension_pointer_target(
    data: &WaveCurveData,
    segment: usize,
    geometry: SplineGeometry,
    pointer: egui::Pos2,
) -> (f32, f32) {
    let (phase, value) = geometry.values_from_pos(pointer);
    let start = data.knots[segment].phase;
    let end = data.knots.get(segment + 1).map_or(1.0, |knot| knot.phase);
    let handle = (phase - start) / (end - start).max(f32::EPSILON);
    (
        segment_curve_for_value(data, segment, value),
        curve_x_from_handle_progress(handle),
    )
}

fn snapped_tension_target(
    data: &WaveCurveData,
    segment: usize,
    geometry: SplineGeometry,
    pointer: egui::Pos2,
) -> (f32, f32) {
    let (_, value) = geometry.values_from_pos(pointer);
    let candidate = segment_curve_for_value(data, segment, value);
    let curve = [
        -MAX_VERTICAL_CURVE,
        -SINE_ARC_CURVE,
        0.0,
        SINE_ARC_CURVE,
        MAX_VERTICAL_CURVE,
    ]
    .into_iter()
    .min_by(|left, right| {
        (left - candidate)
            .abs()
            .total_cmp(&(right - candidate).abs())
    })
    .unwrap_or(0.0);
    (curve, 0.0)
}

fn tension_handle_position(
    data: &WaveCurveData,
    segment: usize,
    geometry: SplineGeometry,
) -> Option<egui::Pos2> {
    let phase = segment_handle_phase(data, segment)?;
    Some(geometry.position(phase, curve_value(data, phase)))
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
    ui.ctx().request_repaint_after(if meter_moving {
        LIVE_METER_REPAINT
    } else {
        IDLE_METER_REPAINT
    });
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

#[cfg(test)]
mod tests {
    use super::{FINE_DRAG_SCALE, point_drag_target};

    #[test]
    fn point_drag_at_full_precision_tracks_the_pointer() {
        let target = point_drag_target((0.4, -0.2), (0.9, 0.8), 1.0);

        assert!((target.0 - 0.9).abs() < f32::EPSILON);
        assert!((target.1 - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn point_drag_at_fine_precision_scales_both_axes_around_the_origin() {
        let target = point_drag_target((0.4, -0.2), (0.9, 0.8), FINE_DRAG_SCALE);

        assert!((target.0 - 0.49).abs() < 1.0e-6);
        assert!((target.1 - (-0.02)).abs() < 1.0e-6);
    }
}
