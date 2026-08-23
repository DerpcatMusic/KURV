//! Generator rack composition and structural editing.

use truce_core::editor::PluginContext;

use crate::editor_filter::draw_ordered_filter_module;
use crate::generators::{FilterConfig, FilterSlot, GroupId, ModuleId, ModuleKind};
use crate::modulators::routing::{FilterControl, ModulationRouteTarget};
use crate::{KurvParams, editor_theme};

mod drag_preview;
mod group_output;
mod insertion;
mod oscillator_card;

use drag_preview::{GeneratorDragGhostKind, paint_generator_drag_ghost};
pub(crate) use insertion::show;

const MODULE_IDENTITY_SHARE: f32 = 0.055;

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
    let interaction = draw_ordered_filter_module(
        ui,
        rect,
        module_id.get(),
        slot.index() + 1,
        &mut config,
        group_accent,
        state.params().editor_dsp_sample_rate(),
    );
    interaction.drag_response.dnd_set_drag_payload(module_id);
    interaction.drag_response.context_menu(|ui| {
        if ui.button("RESET FILTER").clicked() {
            config = FilterConfig::default();
            ui.close();
        }
        if ui.button("REMOVE FILTER").clicked() {
            ui.data_mut(|data| {
                data.insert_temp(egui::Id::new(("filter-remove-menu", module_id.get())), true);
            });
            ui.close();
        }
    });
    let dragging = interaction.drag_response.dragged();
    if dragging {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        if let Some(pointer) = ui.ctx().pointer_interact_pos() {
            paint_generator_drag_ghost(
                ui,
                ("filter", module_id.get()),
                pointer,
                interaction.rect.size(),
                group_accent,
                &format!("FILTER {}", slot.index() + 1),
                config.mode.label(),
                GeneratorDragGhostKind::Filter(config.mode),
            );
        }
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
        (
            FilterControl::Slope,
            &interaction.slope_response,
            displayed_config.slope_db_oct,
        ),
        (
            FilterControl::Morph,
            &interaction.morph_response,
            displayed_config.morph,
        ),
    ] {
        let target = ModulationRouteTarget::filter(module_id, slot, control);
        let owns_modulation =
            crate::editor_modulation::modular_owns_gesture(ui, state, target, response);
        if owns_modulation {
            match control {
                FilterControl::Cutoff => config.cutoff_hz = before,
                FilterControl::Resonance => config.q = before,
                FilterControl::Slope => config.slope_db_oct = before,
                FilterControl::Morph => config.morph = before,
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
        destination_response.rect = response.rect;
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
                FilterControl::Slope => config.slope_db_oct.to_bits() != before.to_bits(),
                FilterControl::Morph => config.morph.to_bits() != before.to_bits(),
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
                FilterControl::Slope => config.slope_db_oct = base_config.slope_db_oct,
                FilterControl::Morph => config.morph = base_config.morph,
            }
        }
    }
    if interaction.changed || config != base_config {
        state.generator_stack.set_filter_config(slot, config);
    }
    let remove_menu = ui.data_mut(|data| {
        data.remove_temp::<bool>(egui::Id::new(("filter-remove-menu", module_id.get())))
            .unwrap_or(false)
    });
    if (interaction.remove || remove_menu)
        && let Ok(module) = state
            .generator_stack
            .edit(|patch| patch.remove_module(module_id))
    {
        clear_module_bindings(state, module.id());
        match module.kind() {
            ModuleKind::Oscillator(slot) => {
                let mut config = state.generator_stack.oscillator_config(slot);
                config.enabled = false;
                state.generator_stack.set_oscillator_config(slot, config);
            }
            ModuleKind::Filter(slot) => state
                .generator_stack
                .set_filter_config(slot, FilterConfig::default()),
        }
    }
}

fn apply_host_automation_to_filter(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    module_id: ModuleId,
    slot: FilterSlot,
    config: &mut FilterConfig,
) {
    for control in FilterControl::ALL.iter().copied() {
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
    let gap = editor_theme::space::XXS.min(rect.height() * 0.08);
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
    label: &str,
    value_text: &str,
    size: egui::Vec2,
) -> (egui::Rect, egui::Response, bool) {
    let minimum = editor_theme::font::VALUE_SIZE + editor_theme::font::CAPTION_SIZE;
    let (id, rect) = ui.allocate_space(egui::vec2(size.x.max(minimum), size.y.max(minimum)));
    let interaction = crate::editor_controls::metric_text_bounds(ui, rect, label, value_text);
    let response = ui.interact(
        interaction,
        id.with("metric-value"),
        egui::Sense::click_and_drag(),
    );
    let response = response.on_hover_cursor(egui::CursorIcon::ResizeVertical);
    let changed = crate::editor_controls::update_custom_value_drag(
        ui, &response, value, range, speed, default,
    );
    let response = response.on_hover_text(
        "Drag vertically to change. Hold Shift for fine control; double-click to reset.",
    );
    (rect, response, changed)
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
