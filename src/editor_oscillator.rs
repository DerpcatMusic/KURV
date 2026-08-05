//! Oscillator waveform preview and quality controls.

use truce::params::Params;
use truce_core::editor::{PluginContext, PluginContextReadF32};

use crate::oscillator::{
    Antialiasing, PhaseWarpMode, sample_custom_shape_with_antialiasing_warped,
};
use crate::wave_curve::{WaveCurveData, WaveCurveState, insert_knot, move_knot, remove_knot};
use crate::{KurvParams, P, editor_theme, editor_widgets};

const HOST_PREVIEW_SAMPLE_RATE: f32 = 48_000.0;
const PREVIEW_POINTS: u16 = 512;

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
    let shape = plain_param_value(state, shape_param);
    let pulse_width = plain_param_value(state, pulse_width_param);
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
    let warp_amount = plain_param_value(state, warp_amount_param);
    let custom_mix = plain_param_value(state, custom_shape_param);
    let curve_state = wave_curve_state(params, oscillator);
    let curve = curve_state.try_curve_rt().unwrap_or_default();
    let spectral = params.generator_engine.value_u8() == 1;
    let factor = if spectral {
        1
    } else {
        params.oversampling.value_u8().clamp(1, 4)
    };
    let antialiasing = if spectral {
        Antialiasing::Spectral
    } else {
        Antialiasing::from_index(params.antialiasing.value_u8()).for_factor(factor)
    };
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

fn edit_wave_curve(
    ui: &mut egui::Ui,
    response: &egui::Response,
    plot: egui::Rect,
    curve: &WaveCurveState,
    oscillator: usize,
) {
    let drag_id = response.id.with(("wave-curve-drag", oscillator));
    let mut data = curve.snapshot();
    let pointer = response.interact_pointer_pos();
    let hit = pointer.and_then(|pointer| hit_knot(&data, plot, pointer));

    if response.double_clicked() && hit.is_none() {
        if let Some(pointer) = pointer {
            let (phase, value) = values_from_pos(plot, pointer);
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
        }
    }

    if response.dragged()
        && let Some(index) = ui.data(|store| store.get_temp::<usize>(drag_id))
        && let Some(pointer) = pointer
    {
        let (phase, value) = values_from_pos(plot, pointer);
        curve.edit(|data| move_knot(data, index, phase, value));
        data = curve.snapshot();
        ui.ctx().request_repaint();
    }
    if response.drag_stopped() {
        ui.data_mut(|store| store.remove::<usize>(drag_id));
    }

    for (index, knot) in data.knots.iter().enumerate() {
        let position = knot_pos(plot, *knot);
        ui.painter().circle_filled(
            position,
            if index == 0 { 4.0 } else { 3.5 },
            editor_theme::palette().accent,
        );
        ui.painter().circle_stroke(
            position,
            5.5,
            egui::Stroke::new(1.0_f32, editor_theme::semantic().well),
        );
    }
    response.clone().on_hover_text(
        "Drag points. Double-click to add. Right-click a point to remove. The cycle is periodic.",
    );
}

fn hit_knot(data: &WaveCurveData, plot: egui::Rect, pointer: egui::Pos2) -> Option<usize> {
    data.knots
        .iter()
        .position(|knot| knot_pos(plot, *knot).distance(pointer) <= 10.0)
}

fn knot_pos(plot: egui::Rect, knot: crate::wave_curve::WaveKnot) -> egui::Pos2 {
    egui::pos2(
        knot.phase.mul_add(plot.width(), plot.left()),
        (-knot.value * plot.height() * 0.42).mul_add(1.0, plot.center().y),
    )
}

fn values_from_pos(plot: egui::Rect, position: egui::Pos2) -> (f32, f32) {
    let phase = ((position.x - plot.left()) / plot.width()).clamp(0.0, 1.0);
    let value = ((plot.center().y - position.y) / (plot.height() * 0.42)).clamp(-1.0, 1.0);
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
            let spectral = state.params().generator_engine.value_u8() == 1;
            for (index, label) in ["LEGACY 2PT", "SPLINE 4PT", "LAGRANGE 4PT", "SPECTRAL 1x"]
                .into_iter()
                .enumerate()
            {
                let selected = if index == 3 {
                    spectral
                } else {
                    #[allow(
                        clippy::cast_precision_loss,
                        reason = "the first three entries map to the three VA modes"
                    )]
                    let normalized = index as f32 / 2.0;
                    !spectral && (state.get_param(P::Antialiasing) - normalized).abs() < 0.01
                };
                if ui.selectable_label(selected, label).clicked() {
                    if index == 3 {
                        state.begin_edit(P::GeneratorEngine);
                        state.set_param(P::GeneratorEngine, 1.0);
                        state.end_edit(P::GeneratorEngine);
                    } else {
                        #[allow(
                            clippy::cast_precision_loss,
                            reason = "the first three entries map to the three VA modes"
                        )]
                        let normalized = index as f64 / 2.0;
                        state.begin_edit(P::GeneratorEngine);
                        state.set_param(P::GeneratorEngine, 0.0);
                        state.end_edit(P::GeneratorEngine);
                        state.begin_edit(P::Antialiasing);
                        state.set_param(P::Antialiasing, normalized);
                        state.end_edit(P::Antialiasing);
                    }
                }
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
    let spectral = state.params().generator_engine.value_u8() == 1;
    ui.add_enabled_ui(!spectral, |ui| {
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
    });
}

pub(crate) fn antialiasing_label(state: &PluginContext<KurvParams>) -> String {
    if state.params().generator_engine.value_u8() == 1 {
        state.format_param(P::GeneratorEngine)
    } else {
        state.format_param(P::Antialiasing)
    }
}

pub(crate) fn quality_label(state: &PluginContext<KurvParams>) -> String {
    if state.params().generator_engine.value_u8() == 1 {
        "FIXED 1x".to_owned()
    } else {
        state.format_param(P::Oversampling)
    }
}
