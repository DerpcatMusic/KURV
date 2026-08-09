//! Oscillator waveform preview and quality controls.

use truce::params::Params;
use truce_core::editor::{PluginContext, PluginContextReadF32};

use crate::generators::{OscillatorConfig, OscillatorSlot};
use crate::oscillators::{
    Antialiasing, MAX_VA_TABLE_FRAMES, PhaseWarpMode, VaTableState,
    sample_custom_shape_with_antialiasing_warped,
};
use crate::wave_curve::{
    WaveCurveData, WaveCurveState, fit_freehand_curve, insert_knot, move_knot, remove_knot,
};
use crate::{KurvParams, P, editor_modulation, editor_theme, editor_widgets};

const HOST_PREVIEW_SAMPLE_RATE: f32 = 48_000.0;
const PREVIEW_POINTS: u16 = 512;

#[derive(Clone, Default)]
struct FreehandStroke {
    points: Vec<(f32, f32)>,
}

pub(crate) fn waveform_view(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    width: f32,
    height: f32,
    shape_param: P,
    pulse_width_param: P,
    warp_mode_param: P,
    warp_amount_param: P,
    custom_shape_param: P,
    oscillator: usize,
) {
    let params = state.params();
    let shape = editor_modulation::effective_plain_value(state, shape_param);
    let pulse_width = editor_modulation::effective_plain_value(state, pulse_width_param);
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the phase-warp parameter is clamped to four discrete values"
    )]
    let warp_mode = PhaseWarpMode::from_index(
        plain_param_value(state, warp_mode_param)
            .round()
            .clamp(0.0, 3.0) as u8,
    );
    let warp_amount = editor_modulation::effective_plain_value(state, warp_amount_param);
    let custom_mix = editor_modulation::effective_plain_value(state, custom_shape_param);
    editor_theme::request_display_repaint(ui);
    let curve_state = wave_curve_state(params, oscillator);
    let table_state = va_table_state(params, oscillator);
    let table = table_state
        .try_table_rt(0)
        .map_or_else(|| table_state.snapshot().compile_rt(), |(_, table)| table);
    let selection = table.select(curve_state.try_curve_rt().unwrap_or_default(), custom_mix);
    let factor = params.oversampling.value_u8().clamp(1, 4);
    let antialiasing = Antialiasing::Spline.for_factor(factor);
    let frequency = 110.0;
    let preview_sample_rate = HOST_PREVIEW_SAMPLE_RATE * f32::from(factor);
    let phase_step = f64::from(frequency / preview_sample_rate);
    let (response, painter) =
        ui.allocate_painter(egui::vec2(width, height), egui::Sense::click_and_drag());
    let rect = response.rect;
    let plot = paint_cycle(ui, &painter, rect, |normalized| {
        sample_custom_shape_with_antialiasing_warped(
            shape,
            f64::from(normalized),
            phase_step,
            pulse_width,
            antialiasing,
            warp_mode,
            warp_amount,
            selection.curve,
            selection.mix,
        )
    });

    let table_frames = table.frame_count();
    let custom_frames = table_frames.max(1);
    let selected_frame =
        ((custom_mix * custom_frames as f32).round() as usize).clamp(1, custom_frames) - 1;
    painter.text(
        egui::pos2(plot.right() - 4.0, plot.top() + 3.0),
        egui::Align2::RIGHT_TOP,
        format!("VA {}/{}", selected_frame + 1, custom_frames),
        editor_theme::font::caption(),
        editor_theme::semantic().text_muted,
    );

    let (morph_response, editable_plot, morph_value) =
        va_morph_strip(ui, &painter, plot, &response, custom_mix, custom_frames);
    if morph_response.drag_started() {
        state.begin_edit(custom_shape_param);
    }
    if morph_response.dragged() || morph_response.clicked() {
        if morph_response.clicked() {
            state.begin_edit(custom_shape_param);
        }
        state.set_param(custom_shape_param, f64::from(morph_value));
        if morph_response.clicked() {
            state.end_edit(custom_shape_param);
        }
    }
    if morph_response.drag_stopped() {
        state.end_edit(custom_shape_param);
    }
    response.context_menu(|ui| {
        if ui
            .add_enabled(
                custom_frames < MAX_VA_TABLE_FRAMES,
                egui::Button::new("Duplicate as next VA frame"),
            )
            .clicked()
            && let Some(new_selected) =
                table_state.duplicate_after(selected_frame, curve_state.snapshot())
        {
            let new_frame_count = table_state.snapshot().frames.len().max(1);
            let new_position = (new_selected + 1) as f32 / new_frame_count as f32;
            state.begin_edit(custom_shape_param);
            state.set_param(custom_shape_param, f64::from(new_position));
            state.end_edit(custom_shape_param);
            ui.close();
        }
        if ui
            .add_enabled(table_frames > 0, egui::Button::new("Remove this VA frame"))
            .clicked()
            && table_state.remove_frame(selected_frame)
        {
            let new_frame_count = table_state.snapshot().frames.len().max(1);
            let new_selected = selected_frame.min(new_frame_count - 1);
            let new_position = (new_selected + 1) as f32 / new_frame_count as f32;
            state.begin_edit(custom_shape_param);
            state.set_param(custom_shape_param, f64::from(new_position));
            state.end_edit(custom_shape_param);
            ui.close();
        }
    });

    if custom_mix > 0.001 {
        edit_wave_curve_target(
            ui,
            &response,
            editable_plot,
            if table_frames == 0 {
                CurveTarget::Legacy(curve_state)
            } else {
                CurveTarget::Table(table_state, selected_frame)
            },
            oscillator,
            editor_theme::palette().accent,
            true,
        );
    } else {
        if response.double_clicked() {
            state.begin_edit(custom_shape_param);
            state.set_param(custom_shape_param, 1.0);
            state.end_edit(custom_shape_param);
        }
        response
            .clone()
            .on_hover_text("Double-click to draw a custom periodic cycle");
    }
}

/// Full VA-table editor for structurally-added oscillator slots.
pub(crate) fn extended_waveform_view(
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
            f64::from(normalized),
            phase_step,
            config.pulse_width.clamp(0.03, 0.97),
            Antialiasing::Spline,
            PhaseWarpMode::None,
            0.0,
            selection.curve,
            selection.mix,
        )
    });
    let table_frames = table.frame_count();
    let custom_frames = table_frames.max(1);
    let selected_frame =
        ((config.custom_shape * custom_frames as f32).round() as usize).clamp(1, custom_frames) - 1;
    painter.text(
        egui::pos2(plot.right() - 4.0, plot.top() + 3.0),
        egui::Align2::RIGHT_TOP,
        format!("VA {}/{}", selected_frame + 1, custom_frames),
        editor_theme::font::caption(),
        editor_theme::semantic().text_muted,
    );
    let (morph_response, editable_plot, morph_value) = va_morph_strip(
        ui,
        &painter,
        plot,
        &response,
        config.custom_shape,
        custom_frames,
    );
    let mut changed = false;
    if morph_response.dragged() || morph_response.clicked() {
        config.custom_shape = morph_value;
        changed = true;
    }
    response.context_menu(|ui| {
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
    if table_frames > 0 && config.custom_shape > 0.001 {
        edit_wave_curve_target(
            ui,
            &response,
            editable_plot,
            CurveTarget::Table(table_state, selected_frame),
            slot.index(),
            editor_theme::palette().accent,
            true,
        );
    } else if response.double_clicked() {
        let _ = table_state.materialize(fallback);
        config.custom_shape = 1.0;
        changed = true;
    }
    response.on_hover_text("Drag the morph strip; right-click to add or remove VA frames");
    changed
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
    editor_widgets::gradient_area_to_baseline(painter, &points, plot.center().y, color, 84);
    painter.add(egui::Shape::line(points, egui::Stroke::new(2.0_f32, color)));
    plot
}

fn va_morph_strip(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    plot: egui::Rect,
    response: &egui::Response,
    position: f32,
    custom_frames: usize,
) -> (egui::Response, egui::Rect, f32) {
    let track = egui::Rect::from_min_max(
        egui::pos2(plot.right() - 10.0, plot.top() + 16.0),
        egui::pos2(plot.right(), plot.bottom() - 6.0),
    );
    let x = track.center().x;
    painter.line_segment(
        [egui::pos2(x, track.top()), egui::pos2(x, track.bottom())],
        egui::Stroke::new(1.0_f32, editor_theme::semantic().grid),
    );
    for source in 0..=custom_frames {
        let y = egui::lerp(track.y_range(), 1.0 - source as f32 / custom_frames as f32);
        painter.circle_filled(egui::pos2(x, y), 1.75, editor_theme::semantic().text_muted);
    }
    painter.circle_filled(
        egui::pos2(
            x,
            egui::lerp(track.y_range(), 1.0 - position.clamp(0.0, 1.0)),
        ),
        3.5,
        editor_theme::palette().accent,
    );
    let response = ui
        .interact(
            track.expand2(egui::vec2(4.0, 2.0)),
            response.id.with("va-table-morph"),
            egui::Sense::click_and_drag(),
        )
        .on_hover_text("Vertical VA-table morph: procedural → custom frames");
    let value = response.interact_pointer_pos().map_or(position, |pointer| {
        ((track.bottom() - pointer.y) / track.height()).clamp(0.0, 1.0)
    });
    let editable_plot = egui::Rect::from_min_max(
        plot.min,
        egui::pos2((track.left() - 2.0).max(plot.left()), plot.bottom()),
    );
    (response, editable_plot, value)
}

fn wave_curve_state(params: &KurvParams, oscillator: usize) -> &WaveCurveState {
    match oscillator {
        0 => &params.osc1_wave_curve_state,
        1 => &params.osc2_wave_curve_state,
        _ => &params.osc3_wave_curve_state,
    }
}

fn va_table_state(params: &KurvParams, oscillator: usize) -> &VaTableState {
    params.generator_stack.va_table(
        OscillatorSlot::from_index(oscillator)
            .expect("oscillator editor indices are bounded by MAX_OSCILLATORS"),
    )
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
            Self::Table(table, index) => table.snapshot().frames.get(index).cloned(),
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
    let pointer = response
        .interact_pointer_pos()
        .filter(|pointer| plot.contains(*pointer));
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
        && let Some(pointer) = pointer
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

    for (index, knot) in data.knots.iter().enumerate() {
        let position = knot_pos(plot, *knot, bipolar);
        ui.painter()
            .circle_filled(position, if index == 0 { 4.0 } else { 3.5 }, color);
        ui.painter().circle_stroke(
            position,
            5.5,
            egui::Stroke::new(1.0_f32, editor_theme::semantic().well),
        );
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
        .position(|knot| knot_pos(plot, *knot, bipolar).distance(pointer) <= 10.0)
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

fn plain_param_value(state: &PluginContext<KurvParams>, id: P) -> f32 {
    state
        .params()
        .param_infos()
        .into_iter()
        .find(|info| info.id == u32::from(id))
        .map_or_else(
            || state.get_param(id),
            |info| info.range.denormalize(f64::from(state.get_param(id))) as f32,
        )
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
