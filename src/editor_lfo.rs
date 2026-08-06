use truce_core::editor::{PluginContext, PluginContextReadF32};

use crate::editor_controls::{enum_cycle_field, param_field_sized, param_toggle_dot};
use crate::editor_oscillator::edit_wave_curve;
use crate::wave_curve::WaveCurveState;
use crate::{KurvParams, P, editor_theme, editor_widgets};

const LFO_TABS: [&str; 6] = ["LFO 1", "LFO 2", "LFO 3", "LFO 4", "MATRIX", "PERF"];
const MODES: [&str; 4] = ["FREE", "RETRIG", "SYNC", "ONE SHOT"];
const SOURCES: [&str; 5] = ["OFF", "LFO 1", "LFO 2", "LFO 3", "LFO 4"];
const TARGETS: [&str; 19] = [
    "OFF", "O1 PITCH", "O1 SHAPE", "O1 PWM", "O1 WARP", "O1 LEVEL", "O1 PAN", "O2 PITCH",
    "O2 SHAPE", "O2 PWM", "O2 WARP", "O2 LEVEL", "O2 PAN", "O3 PITCH", "O3 SHAPE", "O3 PWM",
    "O3 WARP", "O3 LEVEL", "O3 PAN",
];

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
        let tab_width = (width / LFO_TABS.len() as f32 - 2.0).max(42.0);
        for (index, label) in LFO_TABS.into_iter().enumerate() {
            let selected = view.selected == index;
            let response = ui.add_sized(
                [tab_width, tab_height],
                egui::Button::selectable(selected, label).frame(false),
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
        4 => draw_matrix(ui, state, width, body_height),
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
                let field_height = ((height - 12.0) / 4.0).clamp(24.0, 42.0);
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
        editor_theme::palette().accent,
        72,
    );
    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new(2.0_f32, editor_theme::palette().accent),
    ));
    edit_wave_curve(ui, &response, plot, curve, 100 + index);
}

fn draw_matrix(ui: &mut egui::Ui, state: &PluginContext<KurvParams>, width: f32, height: f32) {
    let routes = [
        (P::Mod1Source, P::Mod1Target, P::Mod1Amount),
        (P::Mod2Source, P::Mod2Target, P::Mod2Amount),
        (P::Mod3Source, P::Mod3Target, P::Mod3Amount),
        (P::Mod4Source, P::Mod4Target, P::Mod4Amount),
        (P::Mod5Source, P::Mod5Target, P::Mod5Amount),
        (P::Mod6Source, P::Mod6Target, P::Mod6Amount),
        (P::Mod7Source, P::Mod7Target, P::Mod7Amount),
        (P::Mod8Source, P::Mod8Target, P::Mod8Amount),
    ];
    let row_height = 30.0;
    egui::ScrollArea::vertical()
        .max_height(height)
        .show(ui, |ui| {
            for (index, (source, target, amount)) in routes.into_iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [20.0, row_height],
                        egui::Label::new((index + 1).to_string()),
                    );
                    enum_cycle_field(
                        ui,
                        state,
                        source,
                        "SRC",
                        &SOURCES,
                        (width * 0.24).max(64.0),
                        row_height,
                    );
                    enum_cycle_field(
                        ui,
                        state,
                        target,
                        "TARGET",
                        &TARGETS,
                        (width * 0.38).max(88.0),
                        row_height,
                    );
                    param_field_sized(
                        ui,
                        state,
                        amount,
                        "AMT",
                        (width * 0.22).max(64.0),
                        row_height,
                    );
                });
                ui.add_space(2.0);
            }
        });
}
