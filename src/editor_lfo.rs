use truce::params::FloatParamReadF32;
use truce_core::editor::{PluginContext, PluginContextReadF32};

use crate::editor_controls::{enum_cycle_field, param_field_sized, param_field_sized_value};
use crate::editor_modulation::{clear_source, source_color, source_handle, used_source_mask};
use crate::editor_oscillator::edit_wave_curve_colored_mapped;
use crate::wave_curve::WaveCurveState;
use crate::{KurvParams, P, editor_theme, editor_widgets};

const MODES: [&str; 4] = ["FREE", "RETRIG", "SYNC", "ONE SHOT"];
const RATE_MODES: [&str; 4] = ["Hz", "ms", "BEAT", "KEY"];

#[derive(Clone, Copy, Default)]
struct ModulationUi {
    selected: usize,
}

#[derive(Clone, Copy)]
struct LfoParams {
    rate: P,
    rate_mode: P,
    mode: P,
    phase: P,
    sync: P,
    bipolar: P,
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
    let mut active = active_lfo_mask(state);
    if active & (1 << view.selected) == 0 {
        view.selected = active.trailing_zeros().min(7) as usize;
    }

    let tab_height = 24.0_f32.min(height * 0.16).max(19.0);
    draw_tabs(ui, state, &mut view, &mut active, width, tab_height);
    ui.add_space(4.0);
    let body_height = ui.available_height().max(40.0);
    let gap = ui.spacing().item_spacing.x;
    let controls_width = (width * 0.24).clamp(82.0, 116.0);
    let graph_width = (width - controls_width - gap).max(72.0);
    ui.allocate_ui_with_layout(
        egui::vec2(width, body_height),
        egui::Layout::left_to_right(egui::Align::Min),
        |ui| {
            draw_curve(ui, state, view.selected, graph_width, body_height);
            draw_controls(ui, state, view.selected, controls_width, body_height);
        },
    );
    ui.data_mut(|data| data.insert_temp(id, view));
}

fn draw_tabs(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    view: &mut ModulationUi,
    active: &mut u8,
    width: f32,
    height: f32,
) {
    egui::ScrollArea::horizontal()
        .id_salt("active-lfo-tabs")
        .max_height(height + 2.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.set_width(width);
            ui.horizontal(|ui| {
                for index in 0..8 {
                    if *active & (1 << index) == 0 {
                        continue;
                    }
                    let selected = view.selected == index;
                    let color = source_color(index);
                    let tab = ui.add_sized(
                        [64.0, height],
                        egui::Button::selectable(
                            selected,
                            egui::RichText::new(format!("LFO {}", index + 1)).color(color),
                        )
                        .frame(true)
                        .sense(egui::Sense::click_and_drag()),
                    );
                    source_handle(ui, state, index, &tab);
                    let remove_center = tab.rect.right_top() + egui::vec2(-6.0, 6.0);
                    let remove_rect =
                        egui::Rect::from_center_size(remove_center, egui::vec2(14.0, 14.0));
                    let remove =
                        ui.interact(remove_rect, tab.id.with("remove"), egui::Sense::click());
                    let tab_hovered = ui
                        .input(|input| input.pointer.hover_pos())
                        .is_some_and(|pointer| tab.rect.contains(pointer));
                    if active.count_ones() > 1 && (tab_hovered || remove.hovered()) {
                        ui.painter().circle_filled(
                            remove_center,
                            6.0,
                            editor_theme::semantic().danger,
                        );
                        ui.painter().text(
                            remove_center,
                            egui::Align2::CENTER_CENTER,
                            "×",
                            editor_theme::font::caption(),
                            egui::Color32::WHITE,
                        );
                    }
                    if active.count_ones() > 1 && remove.clicked() {
                        clear_source(state, (index + 1) as u8);
                        *active &= !(1 << index);
                        store_active_lfos(state, *active);
                        view.selected = active.trailing_zeros().min(7) as usize;
                    } else if tab.clicked() {
                        view.selected = index;
                    }
                }
                if let Some(index) = (0..8).find(|index| *active & (1 << index) == 0)
                    && ui
                        .add_sized(
                            [24.0, height],
                            egui::Button::new(
                                egui::RichText::new("+")
                                    .size(16.0)
                                    .color(editor_theme::semantic().text),
                            ),
                        )
                        .on_hover_text("Add LFO")
                        .clicked()
                {
                    *active |= 1 << index;
                    store_active_lfos(state, *active);
                    view.selected = index;
                }
            });
        });
}

fn draw_controls(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    index: usize,
    width: f32,
    height: f32,
) {
    let params = lfo_params(index);
    let gap = ui.spacing().item_spacing.y;
    let field_height = ((height - gap * 4.0) / 5.0).max(18.0);
    ui.vertical(|ui| {
        enum_cycle_field(
            ui,
            state,
            params.rate_mode,
            "RATE",
            &RATE_MODES,
            width,
            field_height,
        );
        if rate_mode(state, params.rate_mode) == 2 || discrete_mode(state, params.mode) == 2 {
            param_field_sized(ui, state, params.sync, "DIV", width, field_height);
        } else {
            let text = rate_text(state, index, params.rate_mode);
            param_field_sized_value(
                ui,
                state,
                params.rate,
                "VALUE",
                width,
                field_height,
                Some(&text),
            );
        }
        enum_cycle_field(ui, state, params.mode, "MODE", &MODES, width, field_height);
        param_field_sized(ui, state, params.phase, "PHASE", width, field_height);
        enum_cycle_field(
            ui,
            state,
            params.bipolar,
            "POLAR",
            &["UNI", "BI"],
            width,
            field_height,
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
    let bipolar = state.get_param(lfo_params(index).bipolar) >= 0.5;
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click_and_drag());
    let plot = rect.shrink(8.0);
    let painter = ui.painter_at(rect);
    editor_widgets::graph_frame(&painter, rect);
    editor_widgets::graph_grid(&painter, plot, 4, if bipolar { 2 } else { 1 });
    let curve = lfo_curve(state.params(), index);
    let compiled = curve
        .try_curve_rt()
        .unwrap_or_else(|| curve.snapshot().compile_rt());
    let baseline = if bipolar {
        plot.center().y
    } else {
        plot.bottom()
    };
    let points: Vec<_> = (0..=256)
        .map(|point| {
            let phase = point as f32 / 256.0;
            let raw = compiled.eval(phase);
            let y = if bipolar {
                (-raw * plot.height() * 0.42).mul_add(1.0, plot.center().y)
            } else {
                plot.bottom() - raw.mul_add(0.5, 0.5) * plot.height() * 0.9
            };
            egui::pos2(phase.mul_add(plot.width(), plot.left()), y)
        })
        .collect();
    let color = source_color(index);
    editor_widgets::gradient_area_to_baseline(&painter, &points, baseline, color, 72);
    painter.add(egui::Shape::line(points, egui::Stroke::new(2.0_f32, color)));
    edit_wave_curve_colored_mapped(ui, &response, plot, curve, 100 + index, color, bipolar);
}

fn active_lfo_mask(state: &PluginContext<KurvParams>) -> u8 {
    let stored = active_params()
        .into_iter()
        .enumerate()
        .fold(0, |mask, (index, param)| {
            if state.get_param(param) >= 0.5 {
                mask | (1 << index)
            } else {
                mask
            }
        });
    let active = stored | used_source_mask(state);
    if active == 0 { 1 } else { active }
}

fn store_active_lfos(state: &PluginContext<KurvParams>, active: u8) {
    let active = if active == 0 { 1 } else { active };
    for (index, param) in active_params().into_iter().enumerate() {
        let enabled = active & (1 << index) != 0;
        if (state.get_param(param) >= 0.5) != enabled {
            state.automate(param, if enabled { 1.0 } else { 0.0 });
        }
    }
}

const fn active_params() -> [P; 8] {
    [
        P::Lfo1Active,
        P::Lfo2Active,
        P::Lfo3Active,
        P::Lfo4Active,
        P::Lfo5Active,
        P::Lfo6Active,
        P::Lfo7Active,
        P::Lfo8Active,
    ]
}

fn rate_mode(state: &PluginContext<KurvParams>, param: P) -> u8 {
    (state.get_param(param).clamp(0.0, 1.0) * 3.0).round() as u8
}

fn rate_text(state: &PluginContext<KurvParams>, index: usize, rate_mode_param: P) -> String {
    let rate = lfo_rate(state.params(), index).max(0.000_01);
    match rate_mode(state, rate_mode_param) {
        1 => {
            let milliseconds = rate;
            if milliseconds < 10.0 {
                format!("{milliseconds:.2} ms")
            } else {
                format!("{milliseconds:.0} ms")
            }
        }
        3 => format!("{:.2}×", crate::lfo::keytrack_multiplier(rate)),
        _ if rate < 10.0 => format!("{rate:.2} Hz"),
        _ => format!("{rate:.0} Hz"),
    }
}

fn discrete_mode(state: &PluginContext<KurvParams>, param: P) -> u8 {
    (state.get_param(param).clamp(0.0, 1.0) * 3.0).round() as u8
}

fn lfo_rate(params: &KurvParams, index: usize) -> f32 {
    match index {
        0 => params.lfo1_rate.value(),
        1 => params.lfo2_rate.value(),
        2 => params.lfo3_rate.value(),
        3 => params.lfo4_rate.value(),
        4 => params.lfo5_rate.value(),
        5 => params.lfo6_rate.value(),
        6 => params.lfo7_rate.value(),
        _ => params.lfo8_rate.value(),
    }
}

fn lfo_params(index: usize) -> LfoParams {
    match index {
        0 => LfoParams {
            rate: P::Lfo1Rate,
            rate_mode: P::Lfo1RateMode,
            mode: P::Lfo1Mode,
            phase: P::Lfo1Phase,
            sync: P::Lfo1Sync,
            bipolar: P::Lfo1Bipolar,
        },
        1 => LfoParams {
            rate: P::Lfo2Rate,
            rate_mode: P::Lfo2RateMode,
            mode: P::Lfo2Mode,
            phase: P::Lfo2Phase,
            sync: P::Lfo2Sync,
            bipolar: P::Lfo2Bipolar,
        },
        2 => LfoParams {
            rate: P::Lfo3Rate,
            rate_mode: P::Lfo3RateMode,
            mode: P::Lfo3Mode,
            phase: P::Lfo3Phase,
            sync: P::Lfo3Sync,
            bipolar: P::Lfo3Bipolar,
        },
        3 => LfoParams {
            rate: P::Lfo4Rate,
            rate_mode: P::Lfo4RateMode,
            mode: P::Lfo4Mode,
            phase: P::Lfo4Phase,
            sync: P::Lfo4Sync,
            bipolar: P::Lfo4Bipolar,
        },
        4 => LfoParams {
            rate: P::Lfo5Rate,
            rate_mode: P::Lfo5RateMode,
            mode: P::Lfo5Mode,
            phase: P::Lfo5Phase,
            sync: P::Lfo5Sync,
            bipolar: P::Lfo5Bipolar,
        },
        5 => LfoParams {
            rate: P::Lfo6Rate,
            rate_mode: P::Lfo6RateMode,
            mode: P::Lfo6Mode,
            phase: P::Lfo6Phase,
            sync: P::Lfo6Sync,
            bipolar: P::Lfo6Bipolar,
        },
        6 => LfoParams {
            rate: P::Lfo7Rate,
            rate_mode: P::Lfo7RateMode,
            mode: P::Lfo7Mode,
            phase: P::Lfo7Phase,
            sync: P::Lfo7Sync,
            bipolar: P::Lfo7Bipolar,
        },
        _ => LfoParams {
            rate: P::Lfo8Rate,
            rate_mode: P::Lfo8RateMode,
            mode: P::Lfo8Mode,
            phase: P::Lfo8Phase,
            sync: P::Lfo8Sync,
            bipolar: P::Lfo8Bipolar,
        },
    }
}

fn lfo_curve(params: &KurvParams, index: usize) -> &WaveCurveState {
    match index {
        0 => &params.lfo1_curve_state,
        1 => &params.lfo2_curve_state,
        2 => &params.lfo3_curve_state,
        3 => &params.lfo4_curve_state,
        4 => &params.lfo5_curve_state,
        5 => &params.lfo6_curve_state,
        6 => &params.lfo7_curve_state,
        _ => &params.lfo8_curve_state,
    }
}
