//! Oscillator waveform preview and quality controls.

use truce::params::Params;
use truce_core::editor::{PluginContext, PluginContextReadF32};

use crate::oscillator::{
    Antialiasing, PhaseWarpMode, sample_custom_shape_with_antialiasing_warped,
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
    let curve = curve_state.try_curve_rt().unwrap_or_default();
    let factor = params.oversampling.value_u8().clamp(1, 4);
    let antialiasing = Antialiasing::Spline.for_factor(factor);
    let frequency = 110.0;
    let preview_sample_rate = HOST_PREVIEW_SAMPLE_RATE * f32::from(factor);
    let phase_step = f64::from(frequency / preview_sample_rate);
    let (response, painter) = ui.allocate_painter(
        egui::vec2(width, height),
        if custom_mix > 0.001 {
            egui::Sense::click_and_drag()
        } else {
            egui::Sense::hover()
        },
    );
    let rect = response.rect;
    let plot =
        editor_widgets::graph_plot(rect, ui, editor_theme::space::XS, editor_theme::space::XS);

    editor_widgets::graph_frame(&painter, rect);
    editor_widgets::graph_grid(&painter, plot, 4, 2);
    painter.line_segment(
        [
            egui::pos2(plot.left(), plot.center().y),
            egui::pos2(plot.right(), plot.center().y),
        ],
        egui::Stroke::new(1.0_f32, editor_theme::semantic().grid),
    );

    let points: Vec<egui::Pos2> = (0..=PREVIEW_POINTS)
        .map(|index| {
            let normalized = f32::from(index) / f32::from(PREVIEW_POINTS);
            let phase = f64::from(index) / f64::from(PREVIEW_POINTS);
            let sample = sample_custom_shape_with_antialiasing_warped(
                shape,
                phase,
                phase_step,
                pulse_width,
                antialiasing,
                warp_mode,
                warp_amount,
                curve,
                custom_mix,
            );
            egui::pos2(
                plot.width().mul_add(normalized, plot.left()),
                (sample * plot.height()).mul_add(-0.42, plot.center().y),
            )
        })
        .collect();
    let waveform_color = editor_theme::palette().accent;
    editor_widgets::gradient_area_to_baseline(
        &painter,
        &points,
        plot.center().y,
        waveform_color,
        84,
    );
    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new(2.0_f32, waveform_color),
    ));

    if custom_mix > 0.001 {
        edit_wave_curve(ui, &response, plot, curve_state, oscillator);
    }
}

fn wave_curve_state(params: &KurvParams, oscillator: usize) -> &WaveCurveState {
    match oscillator {
        0 => &params.osc1_wave_curve_state,
        1 => &params.osc2_wave_curve_state,
        _ => &params.osc3_wave_curve_state,
    }
}

pub(crate) fn edit_wave_curve(
    ui: &mut egui::Ui,
    response: &egui::Response,
    plot: egui::Rect,
    curve: &WaveCurveState,
    oscillator: usize,
) {
    edit_wave_curve_colored(
        ui,
        response,
        plot,
        curve,
        oscillator,
        editor_theme::palette().accent,
    );
}

pub(crate) fn edit_wave_curve_colored(
    ui: &egui::Ui,
    response: &egui::Response,
    plot: egui::Rect,
    curve: &WaveCurveState,
    oscillator: usize,
    color: egui::Color32,
) {
    edit_wave_curve_colored_mapped(ui, response, plot, curve, oscillator, color, true);
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
    let drag_id = response.id.with(("wave-curve-drag", oscillator));
    let stroke_id = response.id.with(("wave-curve-stroke", oscillator));
    let mut data = curve.snapshot();
    let pointer = response.interact_pointer_pos();
    let hit = pointer.and_then(|pointer| hit_knot(&data, plot, pointer, bipolar));

    if response.double_clicked() && hit.is_none() {
        if let Some(pointer) = pointer {
            let (phase, value) = values_from_pos(plot, pointer, bipolar);
            curve.edit(|data| insert_knot(data, phase, value));
            data = curve.snapshot();
        }
    } else if response.secondary_clicked() {
        if let Some(index) = hit {
            curve.edit(|data| remove_knot(data, index));
            data = curve.snapshot();
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
            curve.edit(|data| move_knot(data, index, point.0, point.1));
            data = curve.snapshot();
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
            curve.replace(fit_freehand_curve(&data, &stroke.points));
            data = curve.snapshot();
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
