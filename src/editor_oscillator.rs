//! Oscillator waveform preview and quality controls.

use truce_core::editor::{PluginContext, PluginContextReadF32};

use crate::generators::{OscillatorConfig, OscillatorSlot};
use crate::oscillators::{
    Antialiasing, MAX_VA_TABLE_FRAMES, PhaseWarpMode, VaTableState,
    sample_custom_shape_with_antialiasing_warped,
};
use crate::wave_curve::{
    WaveCurveData, WaveCurveState, fit_freehand_curve, insert_knot, move_knot, remove_knot,
};
use crate::{KurvParams, P, editor_theme, editor_widgets};

const HOST_PREVIEW_SAMPLE_RATE: f32 = 48_000.0;
const PREVIEW_POINTS: u16 = 512;

#[derive(Clone, Default)]
struct FreehandStroke {
    points: Vec<(f32, f32)>,
}

/// Full VA-table editor for structurally-added oscillator slots.
pub(crate) fn oscillator_waveform_view(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    width: f32,
    height: f32,
    slot: OscillatorSlot,
    config: &mut OscillatorConfig,
) -> bool {
    let table_state = state.params().generator_stack.va_table(slot);
    let table = table_state
        .try_table_rt(0)
        .map_or_else(|| table_state.snapshot().compile_rt(), |(_, table)| table);
    let fallback = WaveCurveData::default();
    let selection = table.select(fallback.compile_rt(), config.custom_shape);
    let (response, painter) =
        ui.allocate_painter(egui::vec2(width, height), egui::Sense::click_and_drag());
    let phase_step = 110.0_f64 / f64::from(HOST_PREVIEW_SAMPLE_RATE);
    let plot = paint_cycle(ui, &painter, response.rect, |normalized| {
        sample_custom_shape_with_antialiasing_warped(
            config.shape.clamp(0.0, 3.0),
            f64::from((normalized + config.phase_position).rem_euclid(1.0)),
            phase_step,
            config.pulse_width.clamp(0.03, 0.97),
            Antialiasing::Spline,
            PhaseWarpMode::from_index(config.phase_warp_mode),
            config.phase_warp_amount,
            selection.curve,
            selection.mix,
        )
    });
    let table_frames = table.frame_count();
    let custom_frames = table_frames.max(1);
    let selected_frame =
        ((config.custom_shape * custom_frames as f32).round() as usize).clamp(1, custom_frames) - 1;
    let (table_response, morph_value, label_rect) = va_table_label(
        ui,
        &painter,
        plot,
        &response,
        config.custom_shape,
        selected_frame,
        custom_frames,
    );
    let mut changed = false;
    if table_response.dragged() {
        config.custom_shape = morph_value;
        changed = true;
    }
    table_response.context_menu(|ui| {
        if ui
            .add_enabled(
                custom_frames < MAX_VA_TABLE_FRAMES,
                egui::Button::new("Duplicate as next VA frame"),
            )
            .clicked()
            && let Some(new_selected) =
                table_state.duplicate_after(selected_frame, fallback.clone())
        {
            let new_frame_count = table_state.snapshot().frames.len().max(1);
            config.custom_shape = (new_selected + 1) as f32 / new_frame_count as f32;
            changed = true;
            ui.close();
        }
        if ui
            .add_enabled(table_frames > 0, egui::Button::new("Remove this VA frame"))
            .clicked()
            && table_state.remove_frame(selected_frame)
        {
            let new_frame_count = table_state.snapshot().frames.len().max(1);
            let new_selected = selected_frame.min(new_frame_count - 1);
            config.custom_shape = (new_selected + 1) as f32 / new_frame_count as f32;
            changed = true;
            ui.close();
        }
    });
    let pointer_over_label = response
        .interact_pointer_pos()
        .is_some_and(|pointer| label_rect.contains(pointer));
    let reset_requested = response.secondary_clicked() && !pointer_over_label;
    if reset_requested {
        table_state.replace(Default::default());
        config.custom_shape = 0.0;
        changed = true;
    }
    if table_frames > 0 && config.custom_shape > 0.001 && !reset_requested {
        edit_wave_curve_target(
            ui,
            &response,
            plot,
            CurveTarget::Table(table_state, selected_frame),
            slot.index(),
            editor_theme::palette().accent,
            true,
        );
    } else if response.double_clicked() && !pointer_over_label {
        let _ = table_state.materialize(fallback);
        config.custom_shape = 1.0;
        changed = true;
    }
    response.on_hover_text(
        "Double-click to draw a custom periodic cycle; right-click to reset the VA wavetable",
    );
    changed
}

fn va_table_label(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    plot: egui::Rect,
    parent: &egui::Response,
    position: f32,
    selected_frame: usize,
    custom_frames: usize,
) -> (egui::Response, f32, egui::Rect) {
    let label_rect = egui::Rect::from_min_max(
        egui::pos2((plot.right() - 46.0).max(plot.left()), plot.top()),
        egui::pos2(plot.right(), (plot.top() + 15.0).min(plot.bottom())),
    );
    let response = ui
        .interact(
            label_rect,
            parent.id.with("va-table-label"),
            egui::Sense::click_and_drag(),
        )
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
        .on_hover_text("Drag to morph the VA table; right-click to edit its frames");
    let value = if response.dragged() {
        (position + response.drag_motion().x / 140.0).clamp(0.0, 1.0)
    } else {
        position
    };
    painter.text(
        label_rect.right_top() + egui::vec2(-2.0, 2.0),
        egui::Align2::RIGHT_TOP,
        format!("VA {}/{}", selected_frame + 1, custom_frames),
        editor_theme::font::caption(),
        if response.is_pointer_button_down_on() {
            editor_theme::semantic().primary
        } else {
            editor_theme::semantic().text_muted
        },
    );
    (response, value, label_rect)
}

fn paint_cycle(
    _ui: &egui::Ui,
    painter: &egui::Painter,
    rect: egui::Rect,
    mut sample_at: impl FnMut(f32) -> f32,
) -> egui::Rect {
    let inset = editor_theme::space::XS.min(rect.height() * 0.12);
    let plot = rect.shrink2(egui::vec2(inset, inset * 0.65));
    painter.rect_filled(rect, 0.0, editor_theme::semantic().well);
    let points: Vec<_> = (0..=PREVIEW_POINTS)
        .map(|index| {
            let normalized = f32::from(index) / f32::from(PREVIEW_POINTS);
            let sample = sample_at(normalized);
            egui::pos2(
                plot.width().mul_add(normalized, plot.left()),
                (sample * plot.height()).mul_add(-0.42, plot.center().y),
            )
        })
        .collect();
    let color = editor_theme::palette().accent;
    editor_widgets::gradient_area_to_baseline(painter, &points, plot.center().y, color, 42);
    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new(1.45_f32, color),
    ));
    plot
}

pub(crate) fn edit_wave_curve_colored_mapped(
    ui: &egui::Ui,
    response: &egui::Response,
    plot: egui::Rect,
    curve: &WaveCurveState,
    oscillator: usize,
    color: egui::Color32,
    bipolar: bool,
) {
    edit_wave_curve_target(
        ui,
        response,
        plot,
        CurveTarget::Legacy(curve),
        oscillator,
        color,
        bipolar,
    );
}

#[derive(Clone, Copy)]
enum CurveTarget<'a> {
    Legacy(&'a WaveCurveState),
    Table(&'a VaTableState, usize),
}

impl CurveTarget<'_> {
    fn snapshot(self) -> Option<WaveCurveData> {
        match self {
            Self::Legacy(curve) => Some(curve.snapshot()),
            Self::Table(table, index) => table.frame_snapshot(index),
        }
    }

    fn edit<R>(self, edit: impl FnOnce(&mut WaveCurveData) -> R) -> Option<R> {
        match self {
            Self::Legacy(curve) => Some(curve.edit(edit)),
            Self::Table(table, index) => table.edit_frame(index, edit),
        }
    }

    fn replace(self, data: WaveCurveData) -> bool {
        match self {
            Self::Legacy(curve) => {
                curve.replace(data);
                true
            }
            Self::Table(table, index) => table.replace_frame(index, data),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn edit_wave_curve_target(
    ui: &egui::Ui,
    response: &egui::Response,
    plot: egui::Rect,
    target: CurveTarget<'_>,
    oscillator: usize,
    color: egui::Color32,
    bipolar: bool,
) {
    let drag_id = response.id.with(("wave-curve-drag", oscillator));
    let stroke_id = response.id.with(("wave-curve-stroke", oscillator));
    let Some(mut data) = target.snapshot() else {
        return;
    };
    let drag_pointer = response.interact_pointer_pos();
    let pointer = drag_pointer.filter(|pointer| plot.contains(*pointer));
    let hit = pointer.and_then(|pointer| hit_knot(&data, plot, pointer, bipolar));

    if response.double_clicked() && hit.is_none() {
        if let Some(pointer) = pointer {
            let (phase, value) = values_from_pos(plot, pointer, bipolar);
            let _ = target.edit(|data| insert_knot(data, phase, value));
            data = target.snapshot().unwrap_or_default();
        }
    } else if response.secondary_clicked() {
        if let Some(index) = hit {
            let _ = target.edit(|data| remove_knot(data, index));
            data = target.snapshot().unwrap_or_default();
        }
    } else if response.drag_started() {
        if let Some(index) = hit {
            ui.data_mut(|store| store.insert_temp(drag_id, index));
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
        if let Some(index) = ui.data(|store| store.get_temp::<usize>(drag_id)) {
            let _ = target.edit(|data| move_knot(data, index, point.0, point.1));
            data = target.snapshot().unwrap_or_default();
        } else if let Some(mut stroke) =
            ui.data(|store| store.get_temp::<FreehandStroke>(stroke_id))
        {
            if stroke.points.last().is_none_or(|last| {
                (last.0 - point.0).abs() > 0.001 || (last.1 - point.1).abs() > 0.002
            }) {
                stroke.points.push(point);
                ui.data_mut(|store| store.insert_temp(stroke_id, stroke));
            }
        }
        editor_theme::request_display_repaint(ui);
    }
    if response.drag_stopped() {
        if let Some(stroke) = ui.data_mut(|store| {
            store.remove::<usize>(drag_id);
            let stroke = store.get_temp::<FreehandStroke>(stroke_id);
            store.remove::<FreehandStroke>(stroke_id);
            stroke
        }) && stroke.points.len() >= 2
        {
            let _ = target.replace(fit_freehand_curve(&data, &stroke.points));
            data = target.snapshot().unwrap_or_default();
        }
    }

    if let Some(stroke) = ui.data(|store| store.get_temp::<FreehandStroke>(stroke_id)) {
        let points = stroke
            .points
            .into_iter()
            .map(|(phase, value)| value_pos(plot, phase, value, bipolar))
            .collect();
        ui.painter()
            .add(egui::Shape::line(points, egui::Stroke::new(1.5_f32, color)));
    }

    if response.hovered() || response.is_pointer_button_down_on() {
        let editing = response.is_pointer_button_down_on();
        let active = ui.data(|store| store.get_temp::<usize>(drag_id));
        for (index, knot) in data.knots.iter().enumerate() {
            let position = knot_pos(plot, *knot, bipolar);
            let captured = active == Some(index);
            ui.painter().circle_filled(
                position,
                if captured {
                    4.25
                } else if editing {
                    3.25
                } else {
                    2.25
                },
                color,
            );
            if captured {
                ui.painter().circle_stroke(
                    position,
                    6.0,
                    egui::Stroke::new(1.0_f32, editor_theme::semantic().well),
                );
            }
        }
    }
    response.clone().on_hover_text(
        "Drag empty space to draw. Drag points to refine. Double-click to add; right-click to remove. The fitted cycle is periodic.",
    );
}

fn hit_knot(
    data: &WaveCurveData,
    plot: egui::Rect,
    pointer: egui::Pos2,
    bipolar: bool,
) -> Option<usize> {
    data.knots
        .iter()
        .enumerate()
        .map(|(index, knot)| (index, knot_pos(plot, *knot, bipolar).distance_sq(pointer)))
        .min_by(|left, right| left.1.total_cmp(&right.1))
        .filter(|(_, distance)| *distance <= 100.0)
        .map(|(index, _)| index)
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

pub(crate) fn antialiasing_selector_compact(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    width: f32,
) {
    antialiasing_menu(ui, state, width);
}

fn antialiasing_menu(ui: &mut egui::Ui, state: &PluginContext<KurvParams>, width: f32) {
    egui::ComboBox::from_id_salt("oscillator_antialiasing_menu")
        .selected_text(antialiasing_label(state))
        .width(width)
        .show_ui(ui, |ui| {
            let spline = true;
            if ui.selectable_label(spline, "SPLINE 4PT").clicked() {
                state.begin_edit(P::Antialiasing);
                state.set_param(P::Antialiasing, 0.5);
                state.end_edit(P::Antialiasing);
            }
        });
}

pub(crate) fn quality_selector_compact(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    width: f32,
) {
    quality_menu(ui, state, width);
}

fn quality_menu(ui: &mut egui::Ui, state: &PluginContext<KurvParams>, width: f32) {
    egui::ComboBox::from_id_salt("oscillator_quality_menu")
        .selected_text(quality_label(state))
        .width(width)
        .show_ui(ui, |ui| {
            for (index, label) in ["ECO 1x", "NORMAL 2x", "HIGH 3x", "ULTRA 4x"]
                .into_iter()
                .enumerate()
            {
                #[allow(
                    clippy::cast_precision_loss,
                    reason = "the dropdown has exactly four quality modes"
                )]
                let normalized = index as f32 / 3.0;
                let selected = (state.get_param(P::Oversampling) - normalized).abs() < 0.01;
                if ui.selectable_label(selected, label).clicked() {
                    state.begin_edit(P::Oversampling);
                    state.set_param(P::Oversampling, f64::from(normalized));
                    state.end_edit(P::Oversampling);
                }
            }
        });
}

pub(crate) fn antialiasing_label(state: &PluginContext<KurvParams>) -> String {
    state.format_param(P::Antialiasing)
}

pub(crate) fn quality_label(state: &PluginContext<KurvParams>) -> String {
    state.format_param(P::Oversampling)
}
