use truce_core::editor::{PluginContext, PluginContextReadF32};

use crate::editor_controls::{enum_cycle_field, param_field_sized, param_toggle_dot};
use crate::editor_modulation::{source_color, source_handle};
use crate::editor_oscillator::edit_wave_curve_colored;
use crate::wave_curve::WaveCurveState;
use crate::{KurvParams, P, editor_theme, editor_widgets};

const LFO_TABS: [&str; 5] = ["LFO 1", "LFO 2", "LFO 3", "LFO 4", "PERF"];
const MODES: [&str; 4] = ["FREE", "RETRIG", "SYNC", "ONE SHOT"];

#[derive(Clone, Copy, Default)]
struct ModulationUi {
    selected: usize,
}

pub(crate) fn modulation_view(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    width: f32,
    height: f32,
) {
    let id = ui.id().with("modulation-ui");
    let mut view = ui
        .data(|data| data.get_temp::<ModulationUi>(id))
        .unwrap_or_default();
    let tab_height = 24.0_f32.min(height * 0.16).max(18.0);
    ui.horizontal(|ui| {
        let gaps = ui.spacing().item_spacing.x * (LFO_TABS.len() - 1) as f32;
        let tab_width = ((ui.available_width() - gaps) / LFO_TABS.len() as f32).max(32.0);
        for (index, label) in LFO_TABS.into_iter().enumerate() {
            let selected = view.selected == index;
            let text = egui::RichText::new(label).color(if index < 4 {
                source_color(index)
            } else {
                editor_theme::semantic().text_muted
            });
            let response = ui.add_sized(
                [tab_width, tab_height],
                egui::Button::selectable(selected, text).frame(false),
            );
            if response.clicked() {
                view.selected = index;
            }
        }
    });
    ui.add_space(3.0);
    let body_height = (height - tab_height - 7.0).max(1.0);
    match view.selected {
        0..=3 => draw_lfo(ui, state, view.selected, width, body_height),
        _ => crate::editor::performance_view(ui, state, width, body_height),
    }
    ui.data_mut(|data| data.insert_temp(id, view));
}

fn lfo_params(index: usize) -> (P, P, P, P, P) {
    match index {
        0 => (
            P::Lfo1Rate,
            P::Lfo1Mode,
            P::Lfo1Phase,
            P::Lfo1Sync,
            P::Lfo1Bipolar,
        ),
        1 => (
            P::Lfo2Rate,
            P::Lfo2Mode,
            P::Lfo2Phase,
            P::Lfo2Sync,
            P::Lfo2Bipolar,
        ),
        2 => (
            P::Lfo3Rate,
            P::Lfo3Mode,
            P::Lfo3Phase,
            P::Lfo3Sync,
            P::Lfo3Bipolar,
        ),
        _ => (
            P::Lfo4Rate,
            P::Lfo4Mode,
            P::Lfo4Phase,
            P::Lfo4Sync,
            P::Lfo4Bipolar,
        ),
    }
}

fn lfo_curve(params: &KurvParams, index: usize) -> &WaveCurveState {
    match index {
        0 => &params.lfo1_curve_state,
        1 => &params.lfo2_curve_state,
        2 => &params.lfo3_curve_state,
        _ => &params.lfo4_curve_state,
    }
}

fn draw_lfo(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    index: usize,
    width: f32,
    height: f32,
) {
    let controls_width = (width * 0.28).clamp(96.0, 132.0);
    let graph_width = (width - controls_width - 6.0).max(80.0);
    ui.horizontal(|ui| {
        draw_curve(ui, state, index, graph_width, height);
        ui.allocate_ui_with_layout(
            egui::vec2(controls_width, height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                let (rate, mode, phase, sync, bipolar) = lfo_params(index);
                let field_height = ((height - 34.0) / 4.0).clamp(20.0, 38.0);
                enum_cycle_field(
                    ui,
                    state,
                    mode,
                    "MODE",
                    &MODES,
                    controls_width,
                    field_height,
                );
                if state.get_param(mode) >= 0.5 && state.get_param(mode) < 5.0 / 6.0 {
                    param_field_sized(ui, state, sync, "DIV", controls_width, field_height);
                } else {
                    param_field_sized(ui, state, rate, "RATE", controls_width, field_height);
                }
                param_field_sized(ui, state, phase, "PHASE", controls_width, field_height);
                ui.horizontal(|ui| {
                    param_toggle_dot(ui, state, bipolar, 22.0);
                    ui.label(if state.get_param(bipolar) >= 0.5 {
                        "BIPOLAR"
                    } else {
                        "UNIPOLAR"
                    });
                });
                source_handle(ui, state, index, controls_width, 22.0);
            },
        );
    });
}

fn draw_curve(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    index: usize,
    width: f32,
    height: f32,
) {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click_and_drag());
    let plot = rect.shrink(8.0);
    let painter = ui.painter_at(rect);
    editor_widgets::graph_frame(&painter, rect);
    editor_widgets::graph_grid(&painter, plot, 4, 2);
    let curve = lfo_curve(state.params(), index);
    let compiled = curve.snapshot().compile_rt();
    let points: Vec<_> = (0..=256)
        .map(|point| {
            let phase = point as f32 / 256.0;
            egui::pos2(
                phase.mul_add(plot.width(), plot.left()),
                (-compiled.eval(phase) * plot.height() * 0.44).mul_add(1.0, plot.center().y),
            )
        })
        .collect();
    editor_widgets::gradient_area_to_baseline(
        &painter,
        &points,
        plot.center().y,
        source_color(index),
        72,
    );
    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new(2.0_f32, source_color(index)),
    ));
    edit_wave_curve_colored(ui, &response, plot, curve, 100 + index, source_color(index));
}
