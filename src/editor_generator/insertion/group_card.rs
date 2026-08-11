use truce_core::editor::PluginContext;

use crate::KurvParams;
use crate::editor_theme;
use crate::generators::{
    GroupId, GroupOutput, MAX_OSCILLATORS, MAX_OUTPUT_PAIRS, ModuleKind, OscillatorSlot, Patch,
};

use super::GeneratorInsertionTarget;
use super::actions::{
    add_filter_to_group, add_generator_group, add_oscillator_to_group, next_filter_slot,
    remove_generator_group,
};
use super::add_menu::{self, GeneratorAddAction};
use super::drag_reorder;
use crate::editor_generator::draw_compact_filter;
use crate::editor_generator::group_output::{
    GroupOutputInteraction, draw_group_header, draw_group_output,
};
use crate::editor_generator::oscillator_card::draw_compact_oscillator;

#[derive(Clone, Copy)]
pub(super) struct GroupCardMetrics {
    pub(super) card_height: f32,
    pub(super) header_height: f32,
    pub(super) output_height: f32,
    pub(super) filter_height: f32,
}

impl GroupCardMetrics {
    pub(super) fn from_rack(ui: &egui::Ui, rect: egui::Rect) -> Self {
        let metric_row_height = editor_theme::font::CAPTION_SIZE
            + editor_theme::font::VALUE_SIZE
            + editor_theme::compact_gap(ui)
            + editor_theme::shape::STROKE * 2.0;
        let desired_card_height = editor_theme::title_height(ui) * 4.60 + metric_row_height;
        let card_height = desired_card_height
            .min(rect.height() * 0.42)
            .max(metric_row_height * 3.9);
        let header_height = (editor_theme::title_height(ui) * 1.55)
            .max(metric_row_height * 0.72)
            .min(card_height * 0.14);
        let output_height = (card_height * 0.16).max(metric_row_height * 1.55);
        let filter_height = (card_height * 0.46)
            .max(metric_row_height * 2.45)
            .min(card_height);
        Self {
            card_height,
            header_height,
            output_height,
            filter_height,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn show_group_card(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    patch: &Patch,
    group_index: usize,
    active_insertion: Option<GeneratorInsertionTarget>,
    metrics: GroupCardMetrics,
    gap: f32,
    section_gap: f32,
    show_permanent_add_rows: bool,
) -> Option<(GroupId, GroupOutput)> {
    let group = &patch.groups()[group_index];
    let group_id = group.id();
    let group_accent_index = group_accent_index(state, group_id);
    let group_accent = group_accent(group_accent_index);
    let modules = group.modules();
    let mut collapsed = state
        .params()
        .editor_state
        .lock()
        .is_ok_and(|editor| editor.collapsed_group_ids.contains(&group_id.get()));
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
                    == Some(GeneratorInsertionTarget::Module(group_id.get(), *insertion))
            })
            .count()
    };
    let drag_placeholder_height = drag_reorder::active_group_drag_placeholder_height(
        ui,
        patch,
        group_id,
        metrics.card_height,
        metrics.filter_height,
    );
    let module_gap = editor_theme::space::XXS;
    let group_height = metrics.header_height
        + if collapsed {
            0.0
        } else {
            modules
                .iter()
                .map(|module| match module.kind() {
                    ModuleKind::Oscillator(_) => metrics.card_height,
                    ModuleKind::Filter(_) => metrics.filter_height,
                })
                .sum::<f32>()
                + module_gap * modules.len().saturating_sub(1) as f32
        }
        + editor_theme::title_height(ui) * module_insertions as f32
        + drag_placeholder_height
        + if collapsed {
            0.0
        } else {
            module_gap
                + if show_permanent_add_rows {
                    editor_theme::title_height(ui)
                } else {
                    0.0
                }
                + metrics.output_height
        };
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
    let (_, header) = ui.allocate_space(egui::vec2(ui.available_width(), metrics.header_height));
    let interaction = if rack_item_visible(ui, header) || group_output_popup_open(ui, group_id) {
        draw_group_header(
            ui,
            header,
            group_id,
            group_index,
            patch.groups().len() > 1,
            modules.len(),
            group_background.size(),
            collapsed,
            group.output(),
            group_accent_index,
            group_accent,
        )
    } else {
        GroupOutputInteraction::default()
    };
    let mut group_output_update = interaction.output;
    drag_reorder::draw_group_outside_drop_lane(ui, state, patch, group_background, group_index);
    if collapsed {
        drag_reorder::draw_collapsed_group_drop_zone(ui, state, patch, group_id, header);
    }
    if !collapsed {
        for (visible, module) in modules.iter().enumerate() {
            let module_height = match module.kind() {
                ModuleKind::Oscillator(_) => metrics.card_height,
                ModuleKind::Filter(_) => metrics.filter_height,
            };
            drag_reorder::draw_group_module_insert_zone(
                ui,
                state,
                patch,
                group_id,
                visible,
                active_insertion,
                metrics.card_height,
                metrics.filter_height,
                false,
            );
            let (_, card) = ui.allocate_space(egui::vec2(ui.available_width(), module_height));
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
                    ModuleKind::Filter(slot) => {
                        draw_compact_filter(ui, state, card, slot, module.id(), group_accent)
                    }
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
        patch,
        group_id,
        modules.len(),
        active_insertion,
        metrics.card_height,
        metrics.filter_height,
        collapsed,
    );
    if !collapsed {
        ui.add_space(module_gap);
        if show_permanent_add_rows {
            let add_row = egui::Rect::from_min_size(
                ui.cursor().min,
                egui::vec2(ui.available_width(), editor_theme::title_height(ui)),
            );
            ui.painter()
                .rect_filled(add_row, 0.0, editor_theme::semantic().background);
            let next_oscillator = (0..MAX_OSCILLATORS)
                .filter_map(OscillatorSlot::from_index)
                .find(|slot| !patch.contains_oscillator_slot(*slot));
            let next_filter = next_filter_slot(patch);
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
                            add_oscillator_to_group(state, group_id, modules.len(), slot);
                        }
                    }
                    GeneratorAddAction::Filter => {
                        if let Some(slot) = next_filter {
                            add_filter_to_group(state, group_id, modules.len(), slot);
                        }
                    }
                    GeneratorAddAction::Group => add_generator_group(state, group_index + 1),
                }
            }
        }
    }
    if !collapsed {
        let (_, footer) =
            ui.allocate_space(egui::vec2(ui.available_width(), metrics.output_height));
        if (rack_item_visible(ui, footer) || group_output_popup_open(ui, group_id))
            && let Some(output) = draw_group_output(
                ui,
                state,
                footer,
                group_id,
                group_output_update.unwrap_or_else(|| group.output()),
                group_accent,
            )
        {
            group_output_update = Some(output);
        }
    }
    apply_interaction(
        state,
        patch,
        group_index,
        group_id,
        &mut collapsed,
        &interaction,
    );

    if group_visible {
        ui.painter().rect_stroke(
            group_background,
            editor_theme::shape::CONTROL_RADIUS,
            egui::Stroke::new(
                editor_theme::shape::GROUP_STROKE,
                group_accent.gamma_multiply(0.88),
            ),
            egui::StrokeKind::Inside,
        );
    }
    if interaction.remove {
        remove_generator_group(state, group_id);
    }
    ui.add_space(section_gap);
    group_output_update.map(|output| (group_id, output))
}

fn apply_interaction(
    state: &PluginContext<KurvParams>,
    patch: &Patch,
    group_index: usize,
    group_id: GroupId,
    collapsed: &mut bool,
    interaction: &GroupOutputInteraction,
) {
    if interaction.toggle_collapse {
        *collapsed = !*collapsed;
        if let Ok(mut editor) = state.params().editor_state.lock() {
            if *collapsed {
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
    if let Some(accent) = interaction.accent {
        set_group_accent(state, group_id, accent);
    }
}

pub(super) fn group_accent(index: usize) -> egui::Color32 {
    let accents = editor_theme::group_accents();
    accents[index % accents.len()]
}

pub(super) fn group_accent_index(state: &PluginContext<KurvParams>, group_id: GroupId) -> usize {
    let accent_count = editor_theme::group_accents().len();
    let fallback = group_id.get().wrapping_mul(0x9E37_79B9) as usize % accent_count;
    state
        .params()
        .editor_state
        .lock()
        .ok()
        .map_or(fallback, |editor| {
            editor.group_accent_index(group_id.get(), fallback)
        })
}

fn set_group_accent(state: &PluginContext<KurvParams>, group_id: GroupId, accent: usize) {
    if let Ok(mut editor) = state.params().editor_state.lock() {
        editor.set_group_accent(group_id.get(), accent % editor_theme::group_accents().len());
    }
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

pub(super) fn rack_item_visible(ui: &egui::Ui, rect: egui::Rect) -> bool {
    ui.is_rect_visible(rect) && rect.intersect(ui.clip_rect()).is_positive()
}
