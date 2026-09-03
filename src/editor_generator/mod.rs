//! Generator rack composition and structural editing.

use truce_core::editor::PluginContext;

use crate::editor_filter::draw_ordered_filter_module;
use crate::generators::{
    AuxConfig, AuxSlot, FilterConfig, FilterSlot, GroupId, Module, ModuleId, ModuleKind,
};
use crate::modulators::routing::{FilterControl, ModulationRouteTarget};
use crate::{KurvParams, editor_theme};

mod group_output;
mod insertion;
mod oscillator_card;

pub(crate) use insertion::show;

const MODULE_IDENTITY_SHARE: f32 = 0.055;

fn translucent(color: egui::Color32, alpha: u8) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

fn clear_module_bindings(state: &PluginContext<KurvParams>, module: &Module) {
    crate::editor_modulation::clear_module_routes(state, module.id().get());
    state
        .params()
        .host_automation_targets
        .clear_module(module.id().get());
    if let ModuleKind::Oscillator(source) = module.kind() {
        crate::editor_modulation::clear_generator_source(state, source);
        let legacy_source = source.index() as u8 + 1;
        let patch = state.generator_stack.snapshot();
        for slot in patch
            .groups()
            .iter()
            .flat_map(|group| group.modules())
            .filter_map(|module| module.oscillator_slot())
        {
            let mut config = state.generator_stack.oscillator_config(slot);
            if config.phase_mod_source == legacy_source {
                config.phase_mod_source = 0;
                config.phase_mod_amount = 0.0;
                state.generator_stack.set_oscillator_config(slot, config);
            }
        }
    }
}

fn clear_group_bindings(state: &PluginContext<KurvParams>, group_id: GroupId) {
    crate::editor_modulation::clear_group_routes(state, group_id.get());
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
    if interaction.drag_response.dragged() {
        ui.ctx()
            .set_cursor_icon(if ui.input(|input| input.modifiers.ctrl) {
                egui::CursorIcon::Copy
            } else {
                egui::CursorIcon::Grabbing
            });
    }
    let preview_automation_gesture = interaction.preview_response.drag_started()
        || interaction.preview_response.dragged()
        || interaction.preview_response.drag_stopped();
    for (control, response, before) in [
        Some((
            FilterControl::Cutoff,
            &interaction.cutoff_response,
            displayed_config.cutoff_hz,
        )),
        interaction
            .resonance_response
            .as_ref()
            .map(|response| (FilterControl::Resonance, response, displayed_config.q)),
        interaction.slope_response.as_ref().map(|response| {
            (
                FilterControl::Slope,
                response,
                displayed_config.slope_db_oct,
            )
        }),
        interaction
            .morph_response
            .as_ref()
            .map(|response| (FilterControl::Morph, response, displayed_config.morph)),
        interaction
            .shape_response
            .as_ref()
            .map(|response| (FilterControl::Shape, response, displayed_config.shape)),
    ]
    .into_iter()
    .flatten()
    {
        let target = ModulationRouteTarget::filter(module_id, slot, control);
        let owns_modulation =
            crate::editor_modulation::modular_owns_gesture(ui, state, target, response);
        if owns_modulation {
            match control {
                FilterControl::Cutoff => config.cutoff_hz = before,
                FilterControl::Resonance => config.q = before,
                FilterControl::Slope => config.slope_db_oct = before,
                FilterControl::Morph => config.morph = before,
                FilterControl::Shape => config.shape = before,
            }
        }
        let normalized = control.normalized_value(config);
        let knob_radius =
            (response.rect.width().min(response.rect.height()) * 0.24).clamp(8.0, 22.0);
        let track = egui::Rect::from_center_size(
            egui::pos2(
                response.rect.center().x,
                response.rect.center().y + editor_theme::space::XXS,
            ),
            egui::Vec2::splat(knob_radius * 2.0),
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
            crate::editor_modulation::TrackAxis::Radial,
            1.0,
        );
        if let Some((_, param, _)) = host_binding {
            let changed = match control {
                FilterControl::Cutoff => config.cutoff_hz.to_bits() != before.to_bits(),
                FilterControl::Resonance => config.q.to_bits() != before.to_bits(),
                FilterControl::Slope => config.slope_db_oct.to_bits() != before.to_bits(),
                FilterControl::Morph => config.morph.to_bits() != before.to_bits(),
                FilterControl::Shape => config.shape.to_bits() != before.to_bits(),
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
                FilterControl::Shape => config.shape = base_config.shape,
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
        clear_module_bindings(state, &module);
        match module.kind() {
            ModuleKind::Oscillator(slot) => {
                let mut config = state.generator_stack.oscillator_config(slot);
                config.enabled = false;
                state.generator_stack.set_oscillator_config(slot, config);
            }
            ModuleKind::Filter(slot) => state
                .generator_stack
                .set_filter_config(slot, FilterConfig::default()),
            ModuleKind::Aux(slot) => state
                .generator_stack
                .set_aux_config(slot, AuxConfig::default()),
        }
    }
}

fn draw_compact_aux(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    rect: egui::Rect,
    slot: AuxSlot,
    module_id: ModuleId,
    _group_id: GroupId,
    group_accent: egui::Color32,
) {
    let before = state.generator_stack.aux_config(slot);
    let mut config = before;
    let mut remove = false;
    ui.painter()
        .rect_filled(rect, 0.0, editor_theme::semantic().surface);
    let inner = rect.shrink(editor_theme::space::XS);
    let identity = egui::Rect::from_min_max(
        inner.min,
        egui::pos2(
            inner.left()
                + (inner.width() * MODULE_IDENTITY_SHARE).max(editor_theme::title_height(ui)),
            inner.bottom(),
        ),
    );
    let close_side = identity.width() * 0.42;
    let close_rect = egui::Rect::from_center_size(
        egui::pos2(
            identity.right() - close_side * 0.42,
            identity.top() + close_side * 0.42,
        ),
        egui::Vec2::splat(close_side),
    );
    let drag_rect = egui::Rect::from_min_max(
        egui::pos2(identity.left(), close_rect.bottom()),
        identity.right_bottom(),
    );
    let drag = ui
        .interact(
            drag_rect,
            egui::Id::new(("aux-drag", module_id.get())),
            egui::Sense::drag(),
        )
        .on_hover_cursor(egui::CursorIcon::Grab)
        .on_hover_text("Drag AUX to move; hold Ctrl to duplicate");
    drag.dnd_set_drag_payload(module_id);
    drag.context_menu(|ui| {
        if ui.button("RESET AUX").clicked() {
            config = AuxConfig::default();
            ui.close();
        }
        if ui.button("REMOVE AUX").clicked() {
            remove = true;
            ui.close();
        }
    });
    let close = ui
        .interact(
            close_rect,
            egui::Id::new(("aux-remove", module_id.get())),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Remove AUX");
    remove |= close.clicked();
    if ui.rect_contains_pointer(identity) || close.hovered() || close.is_pointer_button_down_on() {
        if close.is_pointer_button_down_on() {
            ui.painter().rect_filled(
                close_rect,
                editor_theme::shape::CONTROL_RADIUS,
                translucent(editor_theme::semantic().danger, 48),
            );
        }
        ui.painter().text(
            close_rect.center(),
            egui::Align2::CENTER_CENTER,
            "×",
            editor_theme::font::caption(),
            if close.hovered() {
                editor_theme::semantic().danger
            } else {
                editor_theme::semantic().text_muted.gamma_multiply(0.72)
            },
        );
    }
    ui.painter().text(
        identity.center(),
        egui::Align2::CENTER_CENTER,
        "AUX",
        editor_theme::font::caption(),
        group_accent,
    );
    let controls = egui::Rect::from_min_max(
        egui::pos2(identity.right() + editor_theme::space::SM, inner.top()),
        inner.right_bottom(),
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(controls), |ui| {
        ui.horizontal_centered(|ui| {
            let input = ui
                .label("INPUT · DROP OSC")
                .on_hover_text("Drop an oscillator audio-rate source here");
            crate::editor_modulation::modular_destination(
                ui,
                state,
                ModulationRouteTarget::aux(module_id, slot),
                &input,
                0.5,
                input.rect,
                crate::editor_modulation::TrackAxis::Horizontal,
                0.5,
            );
            ui.label("LEVEL");
            let inverted = config.gain.is_sign_negative();
            let mut level = config.gain.abs();
            let level_response = ui.add(
                egui::DragValue::new(&mut level)
                    .range(0.0..=2.0)
                    .speed(0.01),
            );
            if level_response.changed() {
                if ui.input(|input| input.modifiers.ctrl) {
                    level = crate::editor_controls::semantic_snap(
                        level,
                        crate::editor_controls::ValueSemantic::Percent,
                        true,
                    )
                    .clamp(0.0, 2.0);
                }
                config.gain = if inverted { -level } else { level };
            }
            if ui.selectable_label(inverted, "INVERT").clicked() {
                config.gain = -config.gain;
            }
        });
    });
    if config != before {
        state.generator_stack.set_aux_config(slot, config);
    }
    if remove
        && let Ok(module) = state
            .generator_stack
            .edit(|patch| patch.remove_module(module_id))
    {
        clear_module_bindings(state, &module);
        state
            .generator_stack
            .set_aux_config(slot, AuxConfig::default());
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
        ui,
        &response,
        value,
        range.clone(),
        speed,
        default,
        crate::editor_controls::ValueSemantic::from_label(label, &range),
    );
    let response = response.on_hover_text(
        "Drag vertically to change. Hold Shift for fine control or Ctrl for semantic snap; double-click to reset.",
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
