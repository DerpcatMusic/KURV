//! Unison distribution and pan-shaping views.

mod distribution;
mod pan_panel;
mod pan_shape;

pub(crate) use distribution::{
    custom_unison_distribution_view, normalized_unison_rate, paint_vertical_selector,
    vertical_selector_value,
};
pub(crate) use pan_panel::custom_pan_panel_view;

use truce_core::editor::PluginContext;

use crate::modulators::routing::ModulationRouteTarget;
use crate::{KurvParams, editor_theme};

fn host_axes_context_menu(
    response: &egui::Response,
    state: &PluginContext<KurvParams>,
    axes: &[(&str, ModulationRouteTarget, f32)],
) {
    response.context_menu(|ui| {
        ui.label(
            egui::RichText::new("HOST AUTOMATION")
                .font(editor_theme::font::caption())
                .color(editor_theme::semantic().text_muted),
        );
        for (label, target, base) in axes.iter().copied() {
            ui.menu_button(label, |ui| {
                crate::editor_modulation::host_automation_menu(ui, state, target, base);
            });
        }
    });
}

fn update_host_axis(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    target: ModulationRouteTarget,
    response: &egui::Response,
    normalized: f32,
    changed: bool,
) {
    if let Some((_, param, _)) =
        crate::editor_modulation::host_automation_binding(ui, state, target)
    {
        crate::editor_modulation::update_host_automation_gesture(
            state, param, response, normalized, changed,
        );
    }
}
