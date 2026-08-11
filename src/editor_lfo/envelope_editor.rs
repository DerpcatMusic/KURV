use super::*;

mod interaction;
mod painting;
mod parameter_edit;

use interaction::{
    envelope_handles, envelope_points, envelope_stage_label, envelope_time_at_x,
    nearest_envelope_target,
};
use parameter_edit::{
    begin_envelope_edit, envelope_normalized, envelope_sustain_normalized, finish_envelope_drag,
    reset_envelope, set_envelope_normalized, set_envelope_sustain_normalized, set_envelope_time,
};

pub(super) fn draw_envelope_curve(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    index: usize,
    width: f32,
    height: f32,
) {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(width, height),
        egui::Sense::CLICK | egui::Sense::DRAG,
    );
    let graph_inset = editor_theme::graph_inset(ui);
    let handle_radius = (rect.height() * 0.035).clamp(3.5, 6.0);
    let content_inset = (handle_radius * 1.55 + editor_theme::shape::FOCUS_STROKE).max(graph_inset);
    let plot = rect.shrink(content_inset);
    let painter = ui.painter_at(rect);
    let [attack, decay, sustain, release] = envelope_values(state.params(), index);
    let curves = envelope_curve_values(state.params(), index);
    let points = envelope_points(plot, attack, decay, sustain, release);
    if crate::editor_modulation::source_drag_active(ui) {
        painting::paint_source_drag_curve(&painter, &points, curves, plot, source_color(index));
        return;
    }
    let editor_id = response.id.with("envelope-editor");
    let mut editor = ui
        .data(|store| store.get_temp::<EnvelopeEditorUi>(editor_id))
        .unwrap_or_default();
    let handles = envelope_handles(&points, curves);
    let grab_radius = (ui.spacing().interact_size.y * 0.55).max(handle_radius * 2.8);
    let pointer = response
        .interact_pointer_pos()
        .or_else(|| response.hover_pos());
    let hovered = pointer.and_then(|pointer| {
        nearest_envelope_target(&handles, &points, curves, pointer, grab_radius)
    });

    if response.secondary_clicked() {
        editor.context_target = hovered;
    }
    let context_target = editor.context_target.or(hovered);
    let mut reset_stage = None;
    let mut reset_all = false;
    response.context_menu(|ui| {
        if let Some(stage) = context_target
            && ui
                .button(format!("RESET {}", envelope_stage_label(stage)))
                .clicked()
        {
            reset_stage = Some(stage);
            ui.close();
        }
        if ui.button("RESET ENVELOPE").clicked() {
            reset_all = true;
            ui.close();
        }
    });
    if !response.context_menu_opened() {
        editor.context_target = None;
    }
    if reset_all {
        finish_envelope_drag(state, index, &mut editor);
        reset_envelope(state, index, None);
        editor.selected = None;
    } else if let Some(stage) = reset_stage {
        finish_envelope_drag(state, index, &mut editor);
        reset_envelope(state, index, Some(stage));
        editor.selected = Some(stage);
    } else if response.double_clicked()
        && let Some(stage) = hovered
    {
        finish_envelope_drag(state, index, &mut editor);
        reset_envelope(state, index, Some(stage));
        editor.selected = Some(stage);
    } else if response.is_pointer_button_down_on()
        && ui.input(|input| input.pointer.primary_pressed())
        && let Some(stage) = hovered
    {
        begin_envelope_edit(state, index, stage);
        editor.selected = Some(stage);
        editor.drag = Some(stage);
        editor.drag_pointer_origin = response.interact_pointer_pos();
        editor.drag_handle_origin = handles
            .iter()
            .find_map(|(candidate, position)| (*candidate == stage).then_some(*position));
        editor.drag_precision = if ui.input(|input| input.modifiers.shift) {
            0.18
        } else {
            1.0
        };
        ui.ctx().set_dragged_id(response.id);
    }
    let drag_aborted =
        editor.drag.is_some() && ui.input(|input| !input.focused || !input.pointer.primary_down());
    let pointer_delta = ui.input(|input| input.pointer.delta());
    if !drag_aborted
        && editor.drag.is_some()
        && pointer_delta != egui::Vec2::ZERO
        && let Some(stage) = editor.drag
    {
        let requested_precision = if ui.input(|input| input.modifiers.shift) {
            0.18
        } else {
            1.0
        };
        let pointer = response.interact_pointer_pos();
        if matches!(
            stage,
            EnvelopeDrag::Attack | EnvelopeDrag::DecaySustain | EnvelopeDrag::Release
        ) && (editor.drag_precision - requested_precision).abs() > f32::EPSILON
        {
            editor.drag_pointer_origin = pointer;
            editor.drag_handle_origin = handles
                .iter()
                .find_map(|(candidate, position)| (*candidate == stage).then_some(*position));
            editor.drag_precision = requested_precision;
        }
        let y = pointer_delta.y / plot.height().max(1.0) * requested_precision;
        let dragged_x = |pointer: egui::Pos2, fallback: f32| {
            editor
                .drag_pointer_origin
                .zip(editor.drag_handle_origin)
                .map(|(pointer_origin, handle_origin)| {
                    handle_origin.x + (pointer.x - pointer_origin.x) * editor.drag_precision
                })
                .unwrap_or(fallback)
        };
        match stage {
            EnvelopeDrag::Attack => {
                if let Some(pointer) = pointer {
                    let x = dragged_x(pointer, points[1].x);
                    let seconds =
                        envelope_time_at_x(EnvelopeDrag::Attack, x, plot, attack, decay, release);
                    set_envelope_time(state, index, EnvelopeDrag::Attack, seconds);
                }
            }
            EnvelopeDrag::AttackCurve => {
                set_envelope_normalized(
                    state,
                    index,
                    EnvelopeDrag::AttackCurve,
                    envelope_normalized(state, index, EnvelopeDrag::AttackCurve) - y,
                );
            }
            EnvelopeDrag::DecaySustain => {
                if let Some(pointer) = pointer {
                    let x = dragged_x(pointer, points[2].x);
                    let seconds = envelope_time_at_x(
                        EnvelopeDrag::DecaySustain,
                        x,
                        plot,
                        attack,
                        decay,
                        release,
                    );
                    set_envelope_time(state, index, EnvelopeDrag::DecaySustain, seconds);
                }
                set_envelope_sustain_normalized(
                    state,
                    index,
                    envelope_sustain_normalized(state, index) - y,
                );
            }
            EnvelopeDrag::DecayCurve => {
                set_envelope_normalized(
                    state,
                    index,
                    EnvelopeDrag::DecayCurve,
                    envelope_normalized(state, index, EnvelopeDrag::DecayCurve) + y,
                );
            }
            EnvelopeDrag::Sustain => {
                set_envelope_sustain_normalized(
                    state,
                    index,
                    envelope_sustain_normalized(state, index) - y,
                );
            }
            EnvelopeDrag::Release => {
                if let Some(pointer) = pointer {
                    let x = dragged_x(pointer, points[3].x);
                    let seconds =
                        envelope_time_at_x(EnvelopeDrag::Release, x, plot, attack, decay, release);
                    set_envelope_time(state, index, EnvelopeDrag::Release, seconds);
                }
            }
            EnvelopeDrag::ReleaseCurve => {
                set_envelope_normalized(
                    state,
                    index,
                    EnvelopeDrag::ReleaseCurve,
                    envelope_normalized(state, index, EnvelopeDrag::ReleaseCurve) + y,
                );
            }
        }
        editor_theme::request_display_repaint(ui);
    }
    if response.drag_stopped() || drag_aborted {
        finish_envelope_drag(state, index, &mut editor);
    }

    let color = source_color(index);
    painting::paint_editor_curve(
        ui,
        &painter,
        &response,
        painting::EnvelopeCurvePaint {
            points: &points,
            curves,
            handles: &handles,
            plot,
            color,
            hovered,
            editor: &editor,
            handle_radius,
        },
    );
    let value = source_value_meter(state, index).clamp(0.0, 1.0);
    painting::paint_meter(&painter, plot, value, color);
    let meter_moving = meter_is_moving(
        &mut editor.last_meter,
        &mut editor.meter_motion_frames,
        value,
        false,
    );
    request_graph_repaint(ui, meter_moving);
    ui.data_mut(|store| store.insert_temp(editor_id, editor));
}

pub(super) fn envelope_path(points: &[egui::Pos2; 5], curves: [f32; 3]) -> Vec<egui::Pos2> {
    interaction::envelope_path(points, curves)
}
