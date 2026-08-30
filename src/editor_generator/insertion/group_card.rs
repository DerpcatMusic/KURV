use truce_core::editor::PluginContext;

use crate::KurvParams;
use crate::generators::{GroupId, GroupOutput, ModuleKind, Patch};
use crate::{editor_theme, editor_widgets};

use super::GeneratorInsertionTarget;
use super::actions::remove_generator_group;
use super::drag_reorder;
use crate::editor_generator::draw_compact_filter;
use crate::editor_generator::group_output::{GroupOutputInteraction, draw_group_header};
use crate::editor_generator::oscillator_card::draw_compact_oscillator;

#[derive(Clone, Copy)]
pub(super) struct GroupCardMetrics {
    pub(super) card_height: f32,
    pub(super) header_height: f32,
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
        // The group envelope, gain, pan, and routing affordance live in one
        // header row. There is deliberately no separate output footer.
        let header_height = (metric_row_height * 1.28)
            .max(editor_theme::title_height(ui) * 2.0)
            .min(card_height * 0.24);
        let filter_height = (card_height * 0.58)
            .max(metric_row_height * 2.85)
            .min(card_height);
        Self {
            card_height,
            header_height,
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
) -> Option<(GroupId, GroupOutput)> {
    let group = &patch.groups()[group_index];
    let group_id = group.id();
    let group_accent = group_accent(state, group_id);
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
    let module_gap = 0.0;
    let footer_height = if collapsed {
        0.0
    } else {
        editor_theme::title_height(ui) * 0.36
    };
    let group_height = metrics.header_height
        + if collapsed {
            0.0
        } else {
            modules
                .iter()
                .map(|module| {
                    super::module_height(
                        state,
                        module.kind(),
                        metrics.card_height,
                        metrics.filter_height,
                    )
                })
                .sum::<f32>()
                + module_gap * modules.len().saturating_sub(1) as f32
        }
        + editor_theme::title_height(ui) * module_insertions as f32
        + if collapsed { 0.0 } else { module_gap }
        + footer_height;
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
    let interaction = if rack_item_visible(ui, header)
        || group_routing_popup_open(ui, group_id)
        || group_accent_popup_open(ui, group_id)
    {
        draw_group_header(
            ui,
            state,
            header,
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
    let group_output_update = interaction.output;
    if collapsed {
        drag_reorder::draw_collapsed_group_drop_zone(ui, state, patch, group_id, header);
    }
    if !collapsed {
        for (visible, module) in modules.iter().enumerate() {
            let module_height = super::module_height(
                state,
                module.kind(),
                metrics.card_height,
                metrics.filter_height,
            );
            drag_reorder::draw_group_module_insert_zone(
                ui,
                state,
                patch,
                group_id,
                visible,
                active_insertion,
                false,
            );
            let (_, card) = ui.allocate_space(egui::vec2(ui.available_width(), module_height));
            if visible > 0 {
                ui.painter().line_segment(
                    [card.left_top(), card.right_top()],
                    egui::Stroke::new(
                        editor_theme::shape::STROKE,
                        editor_theme::semantic().grid.gamma_multiply(0.82),
                    ),
                );
            }
            let owner_popup_open = match module.kind() {
                ModuleKind::Filter(_) => {
                    crate::editor_filter::filter_type_popup_open(ui, module.id().get())
                }
                ModuleKind::Oscillator(_) => false,
            };
            if rack_item_visible(ui, card) || owner_popup_open {
                let module_id = module.id();
                let dragged = egui::DragAndDrop::payload::<crate::generators::ModuleId>(ui.ctx())
                    .is_some_and(|payload| *payload == module_id);
                editor_widgets::with_dragged_layer(
                    ui,
                    egui::Id::new(("generator-module-drag-layer", module_id.get())),
                    dragged,
                    |ui| match module.kind() {
                        ModuleKind::Oscillator(slot) => draw_compact_oscillator(
                            ui,
                            state,
                            card,
                            slot,
                            module_id,
                            gap,
                            group_accent,
                        ),
                        ModuleKind::Filter(slot) => {
                            draw_compact_filter(ui, state, card, slot, module_id, group_accent)
                        }
                    },
                );
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
        false,
    );
    let mut footer_gap = 0.0;
    if !collapsed {
        ui.add_space(module_gap);
        let (_, footer) = ui.allocate_space(egui::vec2(ui.available_width(), footer_height));
        drag_reorder::draw_expanded_group_drop_zone(
            ui,
            state,
            patch,
            group_id,
            group_background,
            header.bottom(),
            metrics.card_height,
            metrics.filter_height,
            module_gap,
        );
        footer_gap =
            drag_reorder::draw_group_footer_add(ui, state, patch, group_id, footer, group_accent);
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
        let enabled = group_output_update.unwrap_or(group.output()).enabled;
        if !enabled && !collapsed {
            ui.painter().rect_filled(
                egui::Rect::from_min_max(header.left_bottom(), group_background.right_bottom()),
                0.0,
                editor_theme::semantic().background.gamma_multiply(0.68),
            );
        }
        paint_group_border(
            ui,
            group_background,
            group_accent.gamma_multiply(if enabled { 1.0 } else { 0.28 }),
            footer_gap,
        );
    }
    if interaction.remove {
        remove_generator_group(state, group_id);
    }
    ui.add_space(section_gap.max(editor_theme::space::LG + editor_theme::space::SM));
    group_output_update.map(|output| (group_id, output))
}

fn paint_group_border(ui: &egui::Ui, rect: egui::Rect, accent: egui::Color32, _footer_gap: f32) {
    ui.painter().line_segment(
        [rect.left_top(), rect.right_top()],
        egui::Stroke::new(editor_theme::shape::GROUP_STROKE, accent),
    );
    let steps = 24;
    for step in 0..steps {
        let top = egui::lerp(rect.top()..=rect.bottom(), step as f32 / steps as f32);
        let bottom = egui::lerp(rect.top()..=rect.bottom(), (step + 1) as f32 / steps as f32);
        let alpha = egui::lerp(1.0..=0.28, (step + 1) as f32 / steps as f32);
        let stroke = egui::Stroke::new(
            editor_theme::shape::GROUP_STROKE,
            accent.gamma_multiply(alpha),
        );
        ui.painter().line_segment(
            [
                egui::pos2(rect.left(), top),
                egui::pos2(rect.left(), bottom),
            ],
            stroke,
        );
        ui.painter().line_segment(
            [
                egui::pos2(rect.right(), top),
                egui::pos2(rect.right(), bottom),
            ],
            stroke,
        );
    }
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        egui::Stroke::new(
            editor_theme::shape::GROUP_STROKE,
            accent.gamma_multiply(0.28),
        ),
    );
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

pub(super) fn group_accent(state: &PluginContext<KurvParams>, group_id: GroupId) -> egui::Color32 {
    let accents = editor_theme::group_accents();
    let fallback_index = group_id.get().wrapping_mul(0x9E37_79B9) as usize % accents.len();
    let fallback = accents[fallback_index];
    state
        .params()
        .editor_state
        .lock()
        .ok()
        .map_or(fallback, |editor| {
            editor.group_accent_color(group_id.get(), fallback, &accents)
        })
}

fn set_group_accent(state: &PluginContext<KurvParams>, group_id: GroupId, accent: egui::Color32) {
    if let Ok(mut editor) = state.params().editor_state.lock() {
        editor.set_group_accent_color(group_id.get(), accent);
    }
}

fn group_accent_popup_open(ui: &egui::Ui, group_id: GroupId) -> bool {
    let response_id = egui::Id::new(("generator-group-accent", group_id.get()));
    egui::Popup::is_id_open(ui.ctx(), response_id.with("popup"))
}

fn group_routing_popup_open(ui: &egui::Ui, group_id: GroupId) -> bool {
    let response_id = egui::Id::new(("group-routing", group_id.get()));
    egui::Popup::is_id_open(ui.ctx(), response_id.with("popup"))
}

pub(super) fn rack_item_visible(ui: &egui::Ui, rect: egui::Rect) -> bool {
    ui.is_rect_visible(rect) && rect.intersect(ui.clip_rect()).is_positive()
}
