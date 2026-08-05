//! Oscillator waveform preview and quality controls.

use truce::params::Params;
use truce_core::editor::{PluginContext, PluginContextReadF32};

use crate::oscillator::{Antialiasing, PhaseWarpMode, sample_shape_with_antialiasing_warped};
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
    let (response, painter) = ui.allocate_painter(egui::vec2(width, height), egui::Sense::hover());
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
            let sample = sample_shape_with_antialiasing_warped(
                shape,
                phase,
                phase_step,
                pulse_width,
                antialiasing,
                warp_mode,
                warp_amount,
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
