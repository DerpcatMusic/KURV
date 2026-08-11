//! Generator rack composition and structural editing.

use truce_core::editor::PluginContext;

use crate::editor_filter::draw_ordered_filter_module;
use crate::filters::FilterMode;
use crate::generators::{FilterConfig, FilterSlot, GroupId, ModuleId};
use crate::modulators::routing::{FilterControl, ModulationRouteTarget};
use crate::{KurvParams, editor_theme};

mod drag_preview;
mod group_output;
mod insertion;
mod oscillator_card;

use drag_preview::{GeneratorDragGhostKind, paint_generator_drag_ghost};
pub(crate) use insertion::show;

fn translucent(color: egui::Color32, alpha: u8) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

fn clear_module_bindings(state: &PluginContext<KurvParams>, module_id: ModuleId) {
    state
        .params()
        .modulation_route_targets
        .clear_module(module_id.get());
    state
        .params()
        .host_automation_targets
        .clear_module(module_id.get());
}

fn clear_group_bindings(state: &PluginContext<KurvParams>, group_id: GroupId) {
    state
        .params()
        .modulation_route_targets
        .clear_group(group_id.get());
    state
        .params()
        .host_automation_targets
        .clear_group(group_id.get());
}

fn draw_compact_filter(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    rect: egui::Rect,
    slot: FilterSlot,
    module_id: ModuleId,
    group_accent: egui::Color32,
) {
    let base_config = state.generator_stack.filter_config(slot);
    let mut config = base_config;
    apply_host_automation_to_filter(ui, state, module_id, slot, &mut config);
    let displayed_config = config;
    let interaction =
        draw_ordered_filter_module(ui, rect, module_id.get(), &mut config, group_accent);
    interaction.drag_response.dnd_set_drag_payload(module_id);
    interaction.drag_response.context_menu(|ui| {
        if ui.button("RESET FILTER").clicked() {
            config = FilterConfig::default();
            ui.close();
        }
    });
    let dragging = interaction.drag_response.dragged();
    if dragging {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        if let Some(pointer) = ui.ctx().pointer_interact_pos() {
            let mode_label = match config.mode {
                FilterMode::LowPass => "LOW PASS",
                FilterMode::BandPass => "BAND PASS",
                FilterMode::HighPass => "HIGH PASS",
            };
            paint_generator_drag_ghost(
                ui,
                ("filter", module_id.get()),
                pointer,
                interaction.rect.size() * 0.58,
                group_accent,
                &format!("FILTER {}", slot.index() + 1),
                mode_label,
                GeneratorDragGhostKind::Filter(config.mode),
            );
        }
        ui.painter().rect_filled(
            interaction.rect.shrink(editor_theme::shape::STROKE),
            editor_theme::shape::CONTROL_RADIUS,
            translucent(editor_theme::semantic().chrome, 148),
        );
        ui.painter().rect_stroke(
            interaction.rect.shrink(editor_theme::shape::STROKE),
            editor_theme::shape::CONTROL_RADIUS,
            egui::Stroke::new(editor_theme::shape::FOCUS_STROKE, group_accent),
            egui::StrokeKind::Inside,
        );
    }
    let preview_automation_gesture = interaction.preview_response.drag_started()
        || interaction.preview_response.dragged()
        || interaction.preview_response.drag_stopped();
    for (control, response, before) in [
        (
            FilterControl::Cutoff,
            &interaction.cutoff_response,
            displayed_config.cutoff_hz,
        ),
        (
            FilterControl::Resonance,
            &interaction.resonance_response,
            displayed_config.q,
        ),
    ] {
        let target = ModulationRouteTarget::filter(module_id, slot, control);
        let owns_modulation =
            crate::editor_modulation::modular_owns_gesture(ui, state, target, response);
        if owns_modulation {
            match control {
                FilterControl::Cutoff => config.cutoff_hz = before,
                FilterControl::Resonance => config.q = before,
            }
        }
        let normalized = control.normalized_value(config);
        let track = egui::Rect::from_min_max(
            egui::pos2(
                response.rect.left(),
                response.rect.bottom() - editor_theme::shape::FOCUS_STROKE,
            ),
            response.rect.right_bottom(),
        );
        let mut destination_response = response.clone();
        destination_response.rect = match control {
            FilterControl::Cutoff => egui::Rect::from_min_max(
                interaction.rect.min,
                egui::pos2(interaction.rect.center().x, interaction.rect.bottom()),
            ),
            FilterControl::Resonance => egui::Rect::from_min_max(
                egui::pos2(interaction.rect.center().x, interaction.rect.top()),
                interaction.rect.max,
            ),
        };
        destination_response.interact_rect = destination_response.rect.intersect(ui.clip_rect());
        let host_binding = crate::editor_modulation::host_automation_binding(ui, state, target);
        crate::editor_modulation::modular_destination(
            ui,
            state,
            target,
            &destination_response,
            normalized,
            track,
            crate::editor_modulation::TrackAxis::Horizontal,
            1.0,
        );
        if let Some((_, param, _)) = host_binding {
            let changed = match control {
                FilterControl::Cutoff => config.cutoff_hz.to_bits() != before.to_bits(),
                FilterControl::Resonance => config.q.to_bits() != before.to_bits(),
            };
            let automation_response = if preview_automation_gesture {
                &interaction.preview_response
            } else {
                response
            };
            crate::editor_modulation::update_host_automation_gesture(
                state,
                param,
                automation_response,
                normalized,
                changed,
            );
            match control {
                FilterControl::Cutoff => config.cutoff_hz = base_config.cutoff_hz,
                FilterControl::Resonance => config.q = base_config.q,
            }
        }
    }
    if interaction.changed || config != base_config {
        state.generator_stack.set_filter_config(slot, config);
    }
    if interaction.remove
        && state
            .generator_stack
            .edit(|patch| patch.remove_module(module_id).is_ok())
    {
        clear_module_bindings(state, module_id);
        state
            .generator_stack
            .set_filter_config(slot, FilterConfig::default());
    }
}

fn apply_host_automation_to_filter(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    module_id: ModuleId,
    slot: FilterSlot,
    config: &mut FilterConfig,
) {
    for control in [FilterControl::Cutoff, FilterControl::Resonance] {
        let target = ModulationRouteTarget::filter(module_id, slot, control);
        if let Some((_, _, normalized)) =
            crate::editor_modulation::host_automation_binding(ui, state, target)
        {
            control.apply_normalized(config, normalized);
        }
    }
}

fn weighted_cells<const N: usize>(rect: egui::Rect, weights: [f32; N]) -> [egui::Rect; N] {
    let total: f32 = weights.iter().sum();
    let gap = editor_theme::space::XS.min(rect.height() * 0.08);
    let usable_width = (rect.width() - gap * N.saturating_sub(1) as f32).max(0.0);
    let mut cursor = rect.left();
    std::array::from_fn(|index| {
        let right = if index + 1 == N {
            rect.right()
        } else {
            cursor + usable_width * weights[index] / total
        };
        let cell = egui::Rect::from_min_max(
            egui::pos2(cursor, rect.top()),
            egui::pos2(right, rect.bottom()),
        );
        cursor = right + gap;
        cell
    })
}

fn config_scalar_drag(
    ui: &mut egui::Ui,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    speed: f32,
    default: f32,
    size: egui::Vec2,
) -> (egui::Rect, egui::Response, bool) {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(size.x.max(12.0), size.y.max(18.0)),
        egui::Sense::click_and_drag(),
    );
    let response = response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
    let before = *value;
    if response.dragged() {
        let delta = ui.input(|input| input.pointer.delta());
        let precision = if ui.input(|input| input.modifiers.shift) {
            0.1
        } else {
            1.0
        };
        *value =
            (*value + (delta.x - delta.y) * speed * precision).clamp(*range.start(), *range.end());
    } else if response.double_clicked() {
        *value = default;
    }
    let response = response
        .on_hover_text("Drag to change. Hold Shift for fine control; double-click to reset.");
    (rect, response, value.to_bits() != before.to_bits())
}

fn format_pan(value: f32) -> String {
    if value.abs() < 0.005 {
        "C".to_owned()
    } else if value < 0.0 {
        format!("L {:.0}", value.abs() * 100.0)
    } else {
        format!("R {:.0}", value * 100.0)
    }
}
