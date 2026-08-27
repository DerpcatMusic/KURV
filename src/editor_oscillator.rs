//! Oscillator waveform editor and quality controls.

mod preview;
mod va_table;

use truce_core::editor::{PluginContext, PluginContextReadF32};

use crate::{KurvParams, P};

pub(crate) use va_table::oscillator_waveform_view;

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
                crate::editor::begin_edit(state, P::Antialiasing);
                state.set_param(P::Antialiasing, 0.5);
                crate::editor::end_edit(state, P::Antialiasing);
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
                    crate::editor::begin_edit(state, P::Oversampling);
                    state.set_param(P::Oversampling, f64::from(normalized));
                    crate::editor::end_edit(state, P::Oversampling);
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

pub(crate) fn resynth_quality_selector_compact(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    width: f32,
) {
    let mut quality = crate::oscillators::ResynthQuality::from_u8(
        state
            .params()
            .editor_state
            .lock()
            .map(|editor| editor.resynth_quality)
            .unwrap_or(crate::oscillators::ResynthQuality::Standard as u8),
    );
    crate::oscillators::ResynthQuality::set_current(quality);
    egui::ComboBox::from_id_salt("resynth_quality_menu")
        .selected_text(quality.label())
        .width(width)
        .show_ui(ui, |ui| {
            for next in crate::oscillators::ResynthQuality::ALL {
                if ui
                    .selectable_label(quality == next, next.label())
                    .on_hover_text(next.hint())
                    .clicked()
                    && next != quality
                {
                    quality = next;
                    crate::oscillators::ResynthQuality::set_current(next);
                    if let Ok(mut editor) = state.params().editor_state.lock() {
                        editor.resynth_quality = next as u8;
                    }
                    state.resynth_assets.rebuild_resynth_analysis();
                }
            }
        });
}
