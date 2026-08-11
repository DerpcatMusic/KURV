use truce_core::editor::PluginContext;

use crate::editor_widgets::{drag_edge_scroll, with_child};
use crate::generators::{
    FilterConfig, FilterSlot, Group, GroupId, GroupOutput, MAX_FILTERS, MAX_OSCILLATORS,
    MAX_OUTPUT_PAIRS, ModuleId, ModuleKind, OscillatorSlot, Patch,
};
use crate::{KurvParams, editor_theme};

use super::group_output::{GroupOutputInteraction, draw_group_output};
use super::oscillator_card::draw_compact_oscillator;
use super::{clear_group_bindings, clear_module_bindings, draw_compact_filter};

mod add_menu;
mod drag_reorder;
mod layout;

use add_menu::GeneratorAddAction;
use layout::{
    GeneratorInsertionTarget, active_generator_insertion, generator_active_insertion_id,
    generator_insertion_candidates,
};

fn group_accent(group_id: GroupId) -> egui::Color32 {
    let palette = editor_theme::semantic();
    let accents = [
        palette.primary,
        palette.unison,
        palette.pan_shape,
        crate::editor_modulation::source_color(0),
        crate::editor_modulation::source_color(1),
        crate::editor_modulation::source_color(2),
        crate::editor_modulation::source_color(3),
        crate::editor_modulation::source_color(5),
    ];
    let index = group_id.get().wrapping_mul(0x9E37_79B9) as usize % accents.len();
    accents[index]
}

pub(crate) fn show(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    rect: egui::Rect,
    gap: f32,
    section_gap: f32,
) {
    let patch = state.generator_stack.snapshot();
    let root_menu_open = add_menu::root_open(ui);
    let metric_row_height = editor_theme::font::CAPTION_SIZE
        + editor_theme::font::VALUE_SIZE
        + editor_theme::compact_gap(ui)
        + editor_theme::shape::STROKE * 2.0;
    let desired_card_height = editor_theme::title_height(ui) * 6.0 + metric_row_height;
    let card_height = desired_card_height
        .min(rect.height() * 0.46)
        .max(metric_row_height * 4.8);
    let output_height = (card_height * 0.16).max(metric_row_height * 1.55);
    let filter_height = (card_height * 0.46)
        .max(metric_row_height * 2.45)
        .min(card_height);
    with_child(
        ui,
        rect,
        "generator-groups",
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            egui::ScrollArea::vertical()
                .id_salt("generator-groups-scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                    let structural_drag =
                        egui::DragAndDrop::has_payload_of_type::<ModuleId>(ui.ctx())
                            || egui::DragAndDrop::has_payload_of_type::<GroupId>(ui.ctx());
                    drag_edge_scroll(ui, rect, structural_drag);
                    let active_id = generator_active_insertion_id();
                    let previous_insertion = (!root_menu_open)
                        .then(|| {
                            ui.data(|data| data.get_temp::<GeneratorInsertionTarget>(active_id))
                        })
                        .flatten();
                    let insertion_candidates = generator_insertion_candidates(
                        ui,
                        state,
                        &patch,
                        card_height,
                        filter_height,
                        output_height,
                        section_gap,
                        previous_insertion,
                    );
                    let active_insertion = active_generator_insertion(
                        ui,
                        rect,
                        &insertion_candidates,
                        previous_insertion,
                    )
                    .filter(|_| !root_menu_open);
                    ui.data_mut(|data| {
                        if let Some(active) = active_insertion {
                            data.insert_temp(active_id, active);
                        } else {
                            data.remove::<GeneratorInsertionTarget>(active_id);
                        }
                    });
                    // Structural drop zones are laid out independently of module painting. Keep
                    // offscreen modules culled during drags; rendering every destination made a
                    // single gesture multiply all route lookups across the entire 32-slot rack.
                    let module_gap = editor_theme::space::XXS;
                    for (group_index, group) in patch.groups().iter().enumerate() {
                        drag_reorder::draw_generator_insert_zone(
                            ui,
                            state,
                            &patch,
                            group_index,
                            active_insertion,
                            card_height,
                            filter_height,
                        );
                        let group_id = group.id();
                        let group_accent = group_accent(group_id);
                        let modules = group.modules();
                        let mut collapsed =
                            state.params().editor_state.lock().is_ok_and(|editor| {
                                editor.collapsed_group_ids.contains(&group_id.get())
                            });
                        let group_top = ui.cursor().top();
                        let module_insertions = if collapsed {
                            usize::from(
                                active_insertion
                                    == Some(GeneratorInsertionTarget::Module(
                                        group_id.get(),
                                        modules.len(),
                                    )),
                            )
                        } else {
                            (0..=modules.len())
                                .filter(|insertion| {
                                    active_insertion
                                        == Some(GeneratorInsertionTarget::Module(
                                            group_id.get(),
                                            *insertion,
                                        ))
                                })
                                .count()
                        };
                        let group_height = if collapsed {
                            0.0
                        } else {
                            modules
                                .iter()
                                .map(|module| match module.kind() {
                                    ModuleKind::Oscillator(_) => card_height,
                                    ModuleKind::Filter(_) => filter_height,
                                })
                                .sum::<f32>()
                                + module_gap * modules.len().saturating_sub(1) as f32
                        } + editor_theme::title_height(ui)
                            * module_insertions as f32
                            + if collapsed {
                                0.0
                            } else {
                                module_gap + editor_theme::title_height(ui)
                            }
                            + output_height;
                        let group_background = egui::Rect::from_min_size(
                            egui::pos2(ui.cursor().left(), group_top),
                            egui::vec2(ui.available_width(), group_height),
                        );
                        let group_visible = rack_item_visible(ui, group_background);
                        if group_visible {
                            ui.painter().rect_filled(
                                group_background,
                                editor_theme::shape::CONTROL_RADIUS,
                                editor_theme::semantic().surface,
                            );
                        }
                        if !collapsed {
                            for (visible, module) in modules.iter().enumerate() {
                                let module_height = match module.kind() {
                                    ModuleKind::Oscillator(_) => card_height,
                                    ModuleKind::Filter(_) => filter_height,
                                };
                                drag_reorder::draw_group_module_insert_zone(
                                    ui,
                                    state,
                                    &patch,
                                    group_id,
                                    visible,
                                    active_insertion,
                                    card_height,
                                    filter_height,
                                );
                                let (_, card) = ui.allocate_space(egui::vec2(
                                    ui.available_width(),
                                    module_height,
                                ));
                                if rack_item_visible(ui, card) {
                                    match module.kind() {
                                        ModuleKind::Oscillator(slot) => draw_compact_oscillator(
                                            ui,
                                            state,
                                            card,
                                            slot,
                                            module.id(),
                                            gap,
                                            group_accent,
                                        ),
                                        ModuleKind::Filter(slot) => draw_compact_filter(
                                            ui,
                                            state,
                                            card,
                                            slot,
                                            module.id(),
                                            group_accent,
                                        ),
                                    }
                                }
                                if visible + 1 < modules.len() {
                                    ui.add_space(module_gap);
                                }
                            }
                        }
                        drag_reorder::draw_group_module_insert_zone(
                            ui,
                            state,
                            &patch,
                            group_id,
                            modules.len(),
                            active_insertion,
                            card_height,
                            filter_height,
                        );
                        if !collapsed {
                            ui.add_space(module_gap);
                            let next_oscillator = (0..MAX_OSCILLATORS)
                                .filter_map(OscillatorSlot::from_index)
                                .find(|slot| !patch.contains_oscillator_slot(*slot));
                            let next_filter = next_filter_slot(&patch);
                            if let Some(action) = add_menu::show_group(
                                ui,
                                group_id,
                                next_oscillator.is_some(),
                                next_filter.is_some(),
                                patch.groups().len() < MAX_OUTPUT_PAIRS,
                            ) {
                                match action {
                                    GeneratorAddAction::Oscillator => {
                                        if let Some(slot) = next_oscillator {
                                            add_oscillator_to_group(
                                                state,
                                                group_id,
                                                modules.len(),
                                                slot,
                                            );
                                        }
                                    }
                                    GeneratorAddAction::Filter => {
                                        if let Some(slot) = next_filter {
                                            add_filter_to_group(
                                                state,
                                                group_id,
                                                modules.len(),
                                                slot,
                                            );
                                        }
                                    }
                                    GeneratorAddAction::Group => {
                                        add_generator_group(state, group_index + 1);
                                    }
                                }
                            }
                        }
                        let (_, footer) =
                            ui.allocate_space(egui::vec2(ui.available_width(), output_height));
                        let interaction = if rack_item_visible(ui, footer)
                            || group_output_popup_open(ui, group_id)
                        {
                            draw_group_output(
                                ui,
                                state,
                                footer,
                                group_id,
                                group_index,
                                patch.groups().len() > 1,
                                modules.len(),
                                group_background.size(),
                                collapsed,
                                group.output(),
                                group_accent,
                            )
                        } else {
                            GroupOutputInteraction::default()
                        };
                        if interaction.toggle_collapse {
                            collapsed = !collapsed;
                            if let Ok(mut editor) = state.params().editor_state.lock() {
                                if collapsed {
                                    if !editor.collapsed_group_ids.contains(&group_id.get()) {
                                        editor.collapsed_group_ids.push(group_id.get());
                                    }
                                } else {
                                    editor
                                        .collapsed_group_ids
                                        .retain(|id| *id != group_id.get());
                                }
                            }
                        }
                        if interaction.reorder != 0 {
                            let target = group_index
                                .saturating_add_signed(isize::from(interaction.reorder))
                                .min(patch.groups().len().saturating_sub(1));
                            if target != group_index {
                                state.generator_stack.edit(|patch| {
                                    let _ = patch.move_group(group_id, target);
                                });
                            }
                        }

                        let group_rect = egui::Rect::from_min_max(
                            egui::pos2(footer.left(), group_top),
                            footer.right_bottom(),
                        );
                        if group_visible {
                            ui.painter().rect_stroke(
                                group_rect,
                                editor_theme::shape::CONTROL_RADIUS,
                                egui::Stroke::new(
                                    editor_theme::shape::GROUP_STROKE,
                                    group_accent.gamma_multiply(0.78),
                                ),
                                egui::StrokeKind::Inside,
                            );
                        }

                        if interaction.remove {
                            remove_generator_group(state, group_id);
                        }
                        ui.add_space(section_gap);
                    }
                    drag_reorder::draw_generator_insert_zone(
                        ui,
                        state,
                        &patch,
                        patch.groups().len(),
                        active_insertion,
                        card_height,
                        filter_height,
                    );
                    let next_oscillator = (0..MAX_OSCILLATORS)
                        .filter_map(OscillatorSlot::from_index)
                        .find(|slot| !patch.contains_oscillator_slot(*slot));
                    let can_add_group = patch.groups().len() < MAX_OUTPUT_PAIRS;
                    if let Some(action) = add_menu::show_root(
                        ui,
                        next_oscillator.is_some() && can_add_group,
                        can_add_group,
                    ) {
                        match action {
                            GeneratorAddAction::Oscillator => {
                                if can_add_group && let Some(slot) = next_oscillator {
                                    add_oscillator_to_new_group(state, slot, patch.groups().len());
                                }
                            }
                            GeneratorAddAction::Filter => {}
                            GeneratorAddAction::Group => {
                                add_generator_group(state, patch.groups().len());
                            }
                        }
                    }
                });
        },
    );
}

fn rack_item_visible(ui: &egui::Ui, rect: egui::Rect) -> bool {
    ui.is_rect_visible(rect) && rect.intersect(ui.clip_rect()).is_positive()
}

fn group_output_popup_open(ui: &egui::Ui, group_id: GroupId) -> bool {
    ["group-midi-channel", "group-output-pair"]
        .into_iter()
        .any(|control| {
            let id_salt = (control, group_id.get());
            let child_id = ui.id().with(("group-dropdown", id_salt));
            egui::ComboBox::is_open(ui.ctx(), child_id.with(("group-dropdown-combo", id_salt)))
        })
}

fn add_oscillator_to_group(
    state: &PluginContext<KurvParams>,
    group_id: GroupId,
    insertion: usize,
    slot: OscillatorSlot,
) {
    let inserted = state.generator_stack.edit(|patch| {
        let insert_at = patch
            .groups()
            .iter()
            .find(|group| group.id() == group_id)
            .map_or(0, |group| insertion.min(group.modules().len()));
        patch
            .insert_oscillator_with_slot(group_id, insert_at, slot)
            .is_ok()
    });
    if inserted {
        state.generator_stack.reset_oscillator(slot);
    }
}

fn next_filter_slot(patch: &Patch) -> Option<FilterSlot> {
    (0..MAX_FILTERS)
        .filter_map(FilterSlot::from_index)
        .find(|slot| !patch.contains_filter_slot(*slot))
}

fn add_filter_to_group(
    state: &PluginContext<KurvParams>,
    group_id: GroupId,
    insertion: usize,
    slot: FilterSlot,
) {
    let inserted = state.generator_stack.edit(|patch| {
        let insert_at = patch
            .groups()
            .iter()
            .find(|group| group.id() == group_id)
            .map_or(0, |group| insertion.min(group.modules().len()));
        patch
            .insert_filter_with_slot(group_id, insert_at, slot)
            .is_ok()
    });
    if inserted {
        state
            .generator_stack
            .set_filter_config(slot, FilterConfig::default());
    }
}

fn add_generator_group(state: &PluginContext<KurvParams>, insertion: usize) {
    state.generator_stack.edit(|patch| {
        if let Ok(id) = patch.insert_group(insertion) {
            let output = GroupOutput {
                pair: (insertion % MAX_OUTPUT_PAIRS) as u8,
                ..GroupOutput::default()
            };
            let _ = patch.set_group_output(id, output);
        }
    });
}

fn add_oscillator_to_new_group(
    state: &PluginContext<KurvParams>,
    slot: OscillatorSlot,
    insertion: usize,
) {
    let inserted = state.generator_stack.edit(|patch| {
        let insertion = insertion.min(patch.groups().len());
        let Ok(group_id) = patch.insert_group(insertion) else {
            return false;
        };
        let output = GroupOutput {
            pair: (insertion % MAX_OUTPUT_PAIRS) as u8,
            ..GroupOutput::default()
        };
        let _ = patch.set_group_output(group_id, output);
        if patch
            .insert_oscillator_with_slot(group_id, 0, slot)
            .is_err()
        {
            let _ = patch.remove_group(group_id);
            return false;
        }
        true
    });
    if inserted {
        state.generator_stack.reset_oscillator(slot);
    }
}

fn remove_generator_group(state: &PluginContext<KurvParams>, group_id: GroupId) {
    if let Ok(group) = state
        .generator_stack
        .edit(|patch| patch.remove_group(group_id))
    {
        cleanup_removed_group(state, group);
    }
}

fn cleanup_removed_group(state: &PluginContext<KurvParams>, group: Group) {
    if let Ok(mut editor) = state.params().editor_state.lock() {
        editor
            .collapsed_group_ids
            .retain(|id| *id != group.id().get());
    }
    clear_group_bindings(state, group.id());
    for module in group.modules() {
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
