use truce_core::editor::PluginContext;

use crate::editor_widgets::with_child;
use crate::generators::{
    FilterConfig, FilterSlot, GroupId, GroupOutput, MAX_FILTERS, MAX_OSCILLATORS, MAX_OUTPUT_PAIRS,
    Module, ModuleId, ModuleKind, OscillatorSlot, Patch,
};
use crate::{KurvParams, editor_theme};

use super::group_output::{GroupOutputInteraction, draw_group_output};
use super::oscillator_card::draw_compact_oscillator;
use super::{clear_group_bindings, clear_module_bindings, draw_compact_filter, translucent};

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

#[derive(Clone, Copy, PartialEq, Eq)]
enum GeneratorInsertionTarget {
    Group(usize),
    Module(u64, usize),
}

#[derive(Clone, Copy)]
struct GeneratorInsertionCandidate {
    target: GeneratorInsertionTarget,
    left: f32,
    right: f32,
    edge: f32,
}

fn generator_insertion_menu_id(target: GeneratorInsertionTarget) -> egui::Id {
    match target {
        GeneratorInsertionTarget::Group(insertion) => {
            egui::Id::new(("generator-stack-insert-menu", insertion))
        }
        GeneratorInsertionTarget::Module(group, insertion) => {
            egui::Id::new(("generator-module-insert-menu", group, insertion))
        }
    }
}

fn generator_root_menu_id() -> egui::Id {
    egui::Id::new("generator-add-menu-root")
}

fn generator_active_insertion_id() -> egui::Id {
    egui::Id::new("generator-alt-insertion-active")
}

fn generator_insertion_menu_open(ui: &egui::Ui, target: GeneratorInsertionTarget) -> bool {
    ui.data(|data| {
        data.get_temp::<bool>(generator_insertion_menu_id(target))
            .unwrap_or(false)
    })
}

fn active_generator_insertion(
    ui: &egui::Ui,
    viewport: egui::Rect,
    candidates: &[GeneratorInsertionCandidate],
    sticky: Option<GeneratorInsertionTarget>,
) -> Option<GeneratorInsertionTarget> {
    if let Some(open) = candidates
        .iter()
        .find(|candidate| generator_insertion_menu_open(ui, candidate.target))
        .map(|candidate| candidate.target)
    {
        return Some(open);
    }

    let pointer = ui.input(|input| {
        (input.modifiers.alt
            && !egui::DragAndDrop::has_payload_of_type::<ModuleId>(ui.ctx())
            && !egui::DragAndDrop::has_payload_of_type::<GroupId>(ui.ctx())
            && !crate::editor_modulation::source_drag_active(ui))
        .then(|| input.pointer.latest_pos())
        .flatten()
    })?;
    if !viewport.contains(pointer) {
        return None;
    }

    let row_height = editor_theme::title_height(ui);
    if let Some(sticky) = sticky
        && let Some(candidate) = candidates
            .iter()
            .find(|candidate| candidate.target == sticky)
        && (candidate.left..=candidate.right).contains(&pointer.x)
        && (candidate.edge - row_height * 0.16..=candidate.edge + row_height).contains(&pointer.y)
    {
        return Some(sticky);
    }

    let activation_radius = editor_theme::title_height(ui) * 0.72;
    candidates
        .iter()
        .filter(|candidate| (candidate.left..=candidate.right).contains(&pointer.x))
        .filter(|candidate| (candidate.edge - pointer.y).abs() <= activation_radius)
        .min_by(|left, right| {
            (left.edge - pointer.y)
                .abs()
                .total_cmp(&(right.edge - pointer.y).abs())
        })
        .map(|candidate| candidate.target)
}

fn generator_insertion_candidates(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    patch: &Patch,
    card_height: f32,
    filter_height: f32,
    output_height: f32,
    section_gap: f32,
    reserved: Option<GeneratorInsertionTarget>,
) -> Vec<GeneratorInsertionCandidate> {
    let row_height = editor_theme::title_height(ui);
    let left = ui.cursor().left();
    let right = ui.cursor().right();
    let outside_lane_width = (row_height + editor_theme::space::SM).max(card_height * 0.30);
    let lane_edge = (left + outside_lane_width).min(right);
    let collapsed = state.params().editor_state.lock().ok();
    let mut candidates = Vec::new();
    let mut edge = ui.cursor().top();

    for (group_index, group) in patch.groups().iter().enumerate() {
        let group_target = GeneratorInsertionTarget::Group(group_index);
        candidates.push(GeneratorInsertionCandidate {
            target: group_target,
            left,
            right: lane_edge,
            edge,
        });
        if generator_insertion_menu_open(ui, group_target) || reserved == Some(group_target) {
            edge += row_height;
        }

        let group_id = group.id();
        let modules = group.modules();
        let is_collapsed = collapsed
            .as_ref()
            .is_some_and(|editor| editor.collapsed_group_ids.contains(&group_id.get()));
        let module_range = if is_collapsed {
            modules.len()..=modules.len()
        } else {
            0..=modules.len()
        };
        for insertion in module_range {
            let target = GeneratorInsertionTarget::Module(group_id.get(), insertion);
            candidates.push(GeneratorInsertionCandidate {
                target,
                left: lane_edge,
                right,
                edge,
            });
            if generator_insertion_menu_open(ui, target) || reserved == Some(target) {
                edge += row_height;
            }
            if !is_collapsed && insertion < modules.len() {
                edge += match modules[insertion].kind() {
                    ModuleKind::Oscillator(_) => card_height,
                    ModuleKind::Filter(_) => filter_height,
                };
            }
        }
        edge += section_gap * 0.35 + output_height;
    }
    candidates
}

pub(crate) fn show(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    rect: egui::Rect,
    gap: f32,
    section_gap: f32,
) {
    let patch = state.generator_stack.snapshot();
    let root_menu_id = generator_root_menu_id();
    let root_menu_open = ui
        .data(|data| data.get_temp::<bool>(root_menu_id))
        .unwrap_or(false);
    let compact_text_height = editor_theme::font::caption().size + editor_theme::font::value().size;
    let card_height = (rect.width() * 0.23)
        .min(rect.height() * 0.52)
        .max(compact_text_height * 5.4);
    let output_height = (card_height * 0.16).max(compact_text_height * 1.55);
    let filter_height = (card_height * 0.46)
        .max(compact_text_height * 2.45)
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
                    let keep_rack_interactions_alive = ui.ctx().any_popup_open();
                    for (group_index, group) in patch.groups().iter().enumerate() {
                        draw_generator_insert_zone(
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
                                .sum()
                        } + editor_theme::title_height(ui)
                            * module_insertions as f32
                            + section_gap * 0.35
                            + output_height;
                        let group_background = egui::Rect::from_min_size(
                            egui::pos2(ui.cursor().left(), group_top),
                            egui::vec2(ui.available_width(), group_height),
                        );
                        let group_visible = rack_item_visible(ui, group_background);
                        if !collapsed {
                            for (visible, module) in modules.iter().enumerate() {
                                let module_height = match module.kind() {
                                    ModuleKind::Oscillator(_) => card_height,
                                    ModuleKind::Filter(_) => filter_height,
                                };
                                draw_group_module_insert_zone(
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
                                if rack_item_visible(ui, card) || keep_rack_interactions_alive {
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
                            }
                        }
                        draw_group_module_insert_zone(
                            ui,
                            state,
                            &patch,
                            group_id,
                            modules.len(),
                            active_insertion,
                            card_height,
                            filter_height,
                        );

                        ui.add_space(section_gap * 0.35);
                        let (_, footer) =
                            ui.allocate_space(egui::vec2(ui.available_width(), output_height));
                        let interaction =
                            if rack_item_visible(ui, footer) || keep_rack_interactions_alive {
                                draw_group_output(
                                    ui,
                                    state,
                                    footer,
                                    group_id,
                                    group_index,
                                    patch.groups().len() > 1,
                                    modules.len(),
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
                        if interaction.dragging && (group_visible || keep_rack_interactions_alive) {
                            ui.painter().rect_filled(
                                group_rect.shrink(editor_theme::shape::STROKE),
                                editor_theme::shape::CONTROL_RADIUS,
                                translucent(editor_theme::semantic().chrome, 156),
                            );
                            ui.painter().rect_stroke(
                                group_rect,
                                2.0,
                                egui::Stroke::new(
                                    editor_theme::shape::FOCUS_STROKE,
                                    group_accent.gamma_multiply(0.88),
                                ),
                                egui::StrokeKind::Inside,
                            );
                        }

                        if interaction.remove {
                            remove_generator_group(state, group_id, modules);
                        }
                    }
                    draw_generator_insert_zone(
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
                    let next_filter = next_filter_slot(&patch);
                    if active_insertion.is_none() {
                        if let Some(action) = draw_generator_add_menu(
                            ui,
                            root_menu_id,
                            next_oscillator.is_some(),
                            next_filter.is_some(),
                            patch.groups().len() < MAX_OUTPUT_PAIRS,
                        ) {
                            match action {
                                GeneratorAddAction::Oscillator => {
                                    if let Some(slot) = next_oscillator {
                                        add_oscillator_to_new_group(
                                            state,
                                            slot,
                                            patch.groups().len(),
                                        );
                                    }
                                }
                                GeneratorAddAction::Filter => {
                                    if let (Some(slot), Some(group)) =
                                        (next_filter, patch.groups().last())
                                    {
                                        add_filter_to_group(
                                            state,
                                            group.id(),
                                            group.modules().len(),
                                            slot,
                                        );
                                    }
                                }
                                GeneratorAddAction::Group => {
                                    add_generator_group(state, patch.groups().len());
                                }
                            }
                        }
                    }
                });
        },
    );
}

fn draw_generator_insert_zone(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    patch: &Patch,
    insertion: usize,
    active_insertion: Option<GeneratorInsertionTarget>,
    card_height: f32,
    filter_height: f32,
) {
    let module_drag = egui::DragAndDrop::has_payload_of_type::<ModuleId>(ui.ctx());
    let group_drag = egui::DragAndDrop::has_payload_of_type::<GroupId>(ui.ctx());
    let target_id = GeneratorInsertionTarget::Group(insertion);
    let menu_id = generator_insertion_menu_id(target_id);
    let menu_open = ui
        .data(|data| data.get_temp::<bool>(menu_id))
        .unwrap_or(false);
    let row_height = editor_theme::title_height(ui);
    let edge = ui.cursor().top();
    let outside_lane_width = (row_height + editor_theme::space::SM).max(card_height * 0.30);
    let has_trailing_add = insertion == patch.groups().len();
    if !has_trailing_add && active_insertion == Some(target_id) {
        let (button_rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), row_height),
            egui::Sense::click(),
        );
        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
        paint_generator_add_button(ui, button_rect, &response, true, menu_open);
        if let Some(action) = generator_add_popup(
            ui,
            menu_id,
            button_rect,
            &response,
            patch.oscillator_count() < MAX_OSCILLATORS,
            false,
            patch.groups().len() < MAX_OUTPUT_PAIRS,
        ) {
            match action {
                GeneratorAddAction::Oscillator => {
                    let next = (0..MAX_OSCILLATORS)
                        .filter_map(OscillatorSlot::from_index)
                        .find(|slot| !patch.contains_oscillator_slot(*slot));
                    if let Some(slot) = next {
                        add_oscillator_to_new_group(state, slot, insertion);
                    }
                }
                GeneratorAddAction::Filter => {}
                GeneratorAddAction::Group => add_generator_group(state, insertion),
            }
        }
        return;
    }
    if !module_drag && !group_drag {
        return;
    }

    let target = egui::Rect::from_min_max(
        egui::pos2(ui.cursor().left(), edge - row_height * 0.50),
        egui::pos2(ui.cursor().right(), edge + row_height * 0.50),
    );
    let outside_target = egui::Rect::from_min_max(
        target.min,
        egui::pos2(
            (target.left() + outside_lane_width).min(target.right()),
            target.bottom(),
        ),
    );
    let group_response = ui
        .interact(
            target,
            egui::Id::new(("generator-group-stack-insert", insertion)),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::Grabbing);
    let module_response = ui
        .interact(
            outside_target,
            egui::Id::new(("generator-module-new-group-insert", insertion)),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::Grabbing);
    let module_hovered = module_response.dnd_hover_payload::<ModuleId>().is_some();
    let dragged_module = egui::DragAndDrop::payload::<ModuleId>(ui.ctx())
        .as_deref()
        .and_then(|module_id| {
            patch.groups().iter().find_map(|group| {
                group
                    .modules()
                    .iter()
                    .find(|module| module.id() == *module_id)
                    .map(|module| (group, module))
            })
        });
    let dragged_module_height = dragged_module
        .map(|(_, module)| match module.kind() {
            ModuleKind::Oscillator(_) => card_height,
            ModuleKind::Filter(_) => filter_height,
        })
        .unwrap_or(card_height);
    let moving_existing_group = dragged_module.is_some_and(|(group, _)| group.modules().len() == 1);
    let group_hovered = group_response.dnd_hover_payload::<GroupId>().is_some();
    let placeholder_id = egui::Id::new(("generator-new-group-placeholder", insertion));
    let placeholder_open = module_drag
        && ui
            .data(|data| data.get_temp::<bool>(placeholder_id))
            .unwrap_or(false);
    let module_at_capacity = (module_hovered || placeholder_open)
        && patch.groups().len() >= MAX_OUTPUT_PAIRS
        && !moving_existing_group;
    let color = if module_at_capacity {
        editor_theme::semantic().text_muted
    } else {
        editor_theme::semantic().primary
    };
    let show_placeholder = !module_at_capacity && (module_hovered || placeholder_open);
    let mut placeholder_release = None;
    if show_placeholder {
        let (placeholder, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), dragged_module_height),
            egui::Sense::click(),
        );
        let response = response.on_hover_cursor(egui::CursorIcon::Grabbing);
        placeholder_release = response.dnd_release_payload::<ModuleId>();
        paint_generator_drop_placeholder(
            ui,
            placeholder,
            color,
            "DROP MODULE · NEW GROUP",
            row_height,
        );
        let keep_open = ui.input(|input| {
            input.pointer.primary_down()
                && input.pointer.latest_pos().is_some_and(|pointer| {
                    placeholder.expand(row_height * 0.35).contains(pointer)
                        || outside_target.contains(pointer)
                })
        });
        ui.data_mut(|data| data.insert_temp(placeholder_id, keep_open));
    } else {
        ui.data_mut(|data| data.insert_temp(placeholder_id, false));
    }
    let group_placeholder_id = egui::Id::new(("generator-group-placeholder", insertion));
    let group_placeholder_open = group_drag
        && ui
            .data(|data| data.get_temp::<bool>(group_placeholder_id))
            .unwrap_or(false);
    let mut group_placeholder_release = None;
    if group_hovered || group_placeholder_open {
        let (placeholder, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), row_height),
            egui::Sense::click(),
        );
        let response = response.on_hover_cursor(egui::CursorIcon::Grabbing);
        group_placeholder_release = response.dnd_release_payload::<GroupId>();
        paint_generator_drop_placeholder(ui, placeholder, color, "DROP GROUP", row_height);
        let keep_open = ui.input(|input| {
            input.pointer.primary_down()
                && input.pointer.latest_pos().is_some_and(|pointer| {
                    placeholder.expand(row_height * 0.35).contains(pointer)
                        || target.contains(pointer)
                })
        });
        ui.data_mut(|data| data.insert_temp(group_placeholder_id, keep_open));
    } else {
        ui.data_mut(|data| data.insert_temp(group_placeholder_id, false));
    }
    if module_at_capacity {
        let line_inset = target.width() * 0.012;
        ui.painter().text(
            target.right_center() - egui::vec2(line_inset, 0.0),
            egui::Align2::RIGHT_CENTER,
            format!("{MAX_OUTPUT_PAIRS} GROUP LIMIT"),
            editor_theme::font::caption(),
            color,
        );
    }
    if let Some(module_id) =
        placeholder_release.or_else(|| module_response.dnd_release_payload::<ModuleId>())
        && (patch.groups().len() < MAX_OUTPUT_PAIRS || moving_existing_group)
    {
        move_module_to_new_group(state, patch, *module_id, insertion);
        ui.data_mut(|data| data.insert_temp(placeholder_id, false));
    } else if let Some(group_id) =
        group_placeholder_release.or_else(|| group_response.dnd_release_payload::<GroupId>())
    {
        move_group_to_insertion(state, patch, *group_id, insertion);
        ui.data_mut(|data| data.insert_temp(group_placeholder_id, false));
    }
}

fn move_module_to_new_group(
    state: &PluginContext<KurvParams>,
    patch: &Patch,
    module_id: ModuleId,
    insertion: usize,
) {
    let Some(source_group) = patch.groups().iter().find(|group| {
        group
            .modules()
            .iter()
            .any(|module| module.id() == module_id)
    }) else {
        return;
    };
    if source_group.modules().len() == 1 {
        move_group_to_insertion(state, patch, source_group.id(), insertion);
        return;
    }
    state.generator_stack.edit(|patch| {
        if let Ok(group_id) = patch.insert_group(insertion) {
            let output = GroupOutput {
                pair: (insertion % MAX_OUTPUT_PAIRS) as u8,
                ..GroupOutput::default()
            };
            let _ = patch.set_group_output(group_id, output);
            if patch.move_module(module_id, group_id, 0).is_err() {
                let _ = patch.remove_group(group_id);
            }
        }
    });
}

fn draw_group_module_insert_zone(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    patch: &Patch,
    group_id: GroupId,
    insertion: usize,
    active_insertion: Option<GeneratorInsertionTarget>,
    card_height: f32,
    filter_height: f32,
) {
    let module_drag = egui::DragAndDrop::has_payload_of_type::<ModuleId>(ui.ctx());
    let target_id = GeneratorInsertionTarget::Module(group_id.get(), insertion);
    let menu_id = generator_insertion_menu_id(target_id);
    let menu_open = ui
        .data(|data| data.get_temp::<bool>(menu_id))
        .unwrap_or(false);
    let row_height = editor_theme::title_height(ui);
    let edge = ui.cursor().top();
    let outside_lane_width = (row_height + editor_theme::space::SM).max(card_height * 0.30);
    if active_insertion == Some(target_id) {
        let (button_rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), row_height),
            egui::Sense::click(),
        );
        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
        paint_generator_add_button(ui, button_rect, &response, true, menu_open);
        let next_oscillator = (0..MAX_OSCILLATORS)
            .filter_map(OscillatorSlot::from_index)
            .find(|slot| !patch.contains_oscillator_slot(*slot));
        let next_filter = next_filter_slot(patch);
        if let Some(action) = generator_add_popup(
            ui,
            menu_id,
            button_rect,
            &response,
            next_oscillator.is_some(),
            next_filter.is_some(),
            patch.groups().len() < MAX_OUTPUT_PAIRS,
        ) {
            match action {
                GeneratorAddAction::Oscillator => {
                    if let Some(slot) = next_oscillator {
                        add_oscillator_to_group(state, group_id, insertion, slot);
                    }
                }
                GeneratorAddAction::Filter => {
                    if let Some(slot) = next_filter {
                        add_filter_to_group(state, group_id, insertion, slot);
                    }
                }
                GeneratorAddAction::Group => {
                    state.generator_stack.edit(|patch| {
                        let _ = patch.split_group_at(group_id, insertion);
                    });
                }
            }
        }
        return;
    }
    if !module_drag {
        return;
    }

    let target = egui::Rect::from_min_max(
        egui::pos2(ui.cursor().left(), edge - row_height * 0.50),
        egui::pos2(ui.cursor().right(), edge + row_height * 0.50),
    );
    let inside_target = egui::Rect::from_min_max(
        egui::pos2(
            (target.left() + outside_lane_width).min(target.right()),
            target.top(),
        ),
        target.max,
    );
    let response = ui
        .interact(
            inside_target,
            egui::Id::new(("generator-module-insert", group_id.get(), insertion)),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::Grabbing);
    let hovered_module = response.dnd_hover_payload::<ModuleId>();
    let dragged_module = egui::DragAndDrop::payload::<ModuleId>(ui.ctx());
    let dragged_module_height = dragged_module
        .as_deref()
        .and_then(|module_id| {
            patch.groups().iter().find_map(|group| {
                group
                    .modules()
                    .iter()
                    .find(|module| module.id() == *module_id)
                    .map(|module| match module.kind() {
                        ModuleKind::Oscillator(_) => card_height,
                        ModuleKind::Filter(_) => filter_height,
                    })
            })
        })
        .unwrap_or(card_height);
    let source_group = dragged_module.as_deref().and_then(|module_id| {
        patch.groups().iter().find_map(|group| {
            group
                .modules()
                .iter()
                .any(|module| module.id() == *module_id)
                .then_some(group.id())
        })
    });
    let valid = source_group.is_some();
    let color = group_accent(group_id);
    let placeholder_id = egui::Id::new(("generator-module-placeholder", group_id.get(), insertion));
    let placeholder_open = module_drag
        && ui
            .data(|data| data.get_temp::<bool>(placeholder_id))
            .unwrap_or(false);
    let mut placeholder_release = None;
    if valid && (hovered_module.is_some() || placeholder_open) {
        let (placeholder, placeholder_response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), dragged_module_height),
            egui::Sense::click(),
        );
        let placeholder_response = placeholder_response.on_hover_cursor(egui::CursorIcon::Grabbing);
        placeholder_release = placeholder_response.dnd_release_payload::<ModuleId>();
        paint_generator_drop_placeholder(ui, placeholder, color, "DROP MODULE", row_height);
        let keep_open = ui.input(|input| {
            input.pointer.primary_down()
                && input.pointer.latest_pos().is_some_and(|pointer| {
                    placeholder.expand(row_height * 0.35).contains(pointer)
                        || inside_target.contains(pointer)
                })
        });
        ui.data_mut(|data| data.insert_temp(placeholder_id, keep_open));
    } else {
        ui.data_mut(|data| data.insert_temp(placeholder_id, false));
    }
    if let Some(module_id) =
        placeholder_release.or_else(|| response.dnd_release_payload::<ModuleId>())
        && valid
    {
        move_module_to_group(state, patch, *module_id, group_id, insertion);
        ui.data_mut(|data| data.insert_temp(placeholder_id, false));
    }
}

fn paint_generator_drop_placeholder(
    ui: &egui::Ui,
    rect: egui::Rect,
    color: egui::Color32,
    label: &str,
    dash_unit: f32,
) {
    ui.painter().rect_filled(rect, 1.0, translucent(color, 14));
    let outline = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
        rect.left_top(),
    ];
    ui.painter().add(egui::Shape::dashed_line(
        &outline,
        egui::Stroke::new(editor_theme::shape::FOCUS_STROKE, color),
        dash_unit * 0.42,
        dash_unit * 0.30,
    ));
    ui.painter().text(
        rect.left_center() + egui::vec2(editor_theme::space::SM, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        editor_theme::font::label(),
        color,
    );
}

fn move_group_to_insertion(
    state: &PluginContext<KurvParams>,
    patch: &Patch,
    group_id: GroupId,
    insertion: usize,
) {
    let Some(source) = patch
        .groups()
        .iter()
        .position(|group| group.id() == group_id)
    else {
        return;
    };
    let target = if source < insertion {
        insertion.saturating_sub(1)
    } else {
        insertion
    }
    .min(patch.groups().len().saturating_sub(1));
    if target != source {
        state.generator_stack.edit(|patch| {
            let _ = patch.move_group(group_id, target);
        });
    }
}

fn move_module_to_group(
    state: &PluginContext<KurvParams>,
    patch: &Patch,
    module_id: ModuleId,
    destination: GroupId,
    insertion: usize,
) {
    let Some((source_group, source_index)) = patch.groups().iter().find_map(|group| {
        group
            .modules()
            .iter()
            .position(|module| module.id() == module_id)
            .map(|index| (group.id(), index))
    }) else {
        return;
    };
    let Some(destination_len) = patch
        .groups()
        .iter()
        .find(|group| group.id() == destination)
        .map(|group| group.modules().len())
    else {
        return;
    };
    let target = if source_group == destination && source_index < insertion {
        insertion.saturating_sub(1)
    } else {
        insertion
    };
    let target = if source_group == destination {
        target.min(destination_len.saturating_sub(1))
    } else {
        target.min(destination_len)
    };
    if source_group != destination || source_index != target {
        state.generator_stack.edit(|patch| {
            let _ = patch.move_module(module_id, destination, target);
        });
    }
}

#[derive(Clone, Copy)]
enum GeneratorAddAction {
    Oscillator,
    Filter,
    Group,
}

fn draw_generator_add_menu(
    ui: &mut egui::Ui,
    menu_id: egui::Id,
    can_add_oscillator: bool,
    can_add_filter: bool,
    can_add_group: bool,
) -> Option<GeneratorAddAction> {
    let open = ui
        .data(|data| data.get_temp::<bool>(menu_id))
        .unwrap_or(false);
    let (id, button_rect) = ui.allocate_space(egui::vec2(
        ui.available_width(),
        editor_theme::title_height(ui),
    ));
    if !rack_item_visible(ui, button_rect) && !open {
        return None;
    }
    let response = ui
        .interact(button_rect, id, egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    paint_generator_add_button(ui, button_rect, &response, false, open);
    generator_add_popup(
        ui,
        menu_id,
        button_rect,
        &response,
        can_add_oscillator,
        can_add_filter,
        can_add_group,
    )
}

fn rack_item_visible(ui: &egui::Ui, rect: egui::Rect) -> bool {
    ui.is_rect_visible(rect) && rect.intersect(ui.clip_rect()).is_positive()
}

fn paint_generator_add_button(
    ui: &egui::Ui,
    button_rect: egui::Rect,
    response: &egui::Response,
    insertion: bool,
    open: bool,
) {
    let palette = editor_theme::semantic();
    let hovered = response.hovered();
    let pressed = response.is_pointer_button_down_on();
    if insertion || open || hovered {
        ui.painter().rect_filled(
            button_rect,
            1.0,
            if insertion {
                translucent(palette.primary, if pressed { 34 } else { 22 })
            } else if open || pressed {
                palette.control
            } else {
                palette.surface
            },
        );
    }
    ui.painter().rect_stroke(
        button_rect,
        1.0,
        egui::Stroke::new(
            if pressed || open {
                editor_theme::shape::FOCUS_STROKE
            } else {
                editor_theme::shape::STROKE
            },
            if insertion || hovered || open {
                palette.primary
            } else {
                palette.grid
            },
        ),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        button_rect.left_center() + egui::vec2(button_rect.height() * 0.5, 0.0),
        egui::Align2::LEFT_CENTER,
        "+ ADD MODULE",
        editor_theme::font::label(),
        if insertion {
            palette.primary
        } else if hovered || open || pressed {
            palette.text
        } else {
            palette.text_muted
        },
    );
}

fn generator_add_popup(
    ui: &mut egui::Ui,
    menu_id: egui::Id,
    button_rect: egui::Rect,
    response: &egui::Response,
    can_add_oscillator: bool,
    can_add_filter: bool,
    can_add_group: bool,
) -> Option<GeneratorAddAction> {
    let mut action = None;
    let mut open = ui
        .data(|data| data.get_temp::<bool>(menu_id))
        .unwrap_or(false);
    let clicked = response.clicked()
        || ui.input(|input| {
            input.pointer.primary_clicked()
                && input
                    .pointer
                    .latest_pos()
                    .is_some_and(|pointer| response.rect.contains(pointer))
        });
    if clicked {
        open = !open;
    }
    if open && ui.input(|input| input.key_pressed(egui::Key::Escape)) {
        open = false;
    }

    if open {
        let frame_margin = (ui.spacing().item_spacing.x * 0.5).round() as i8;
        let row_height = ui.spacing().interact_size.y * 0.9;
        let popup_width = (button_rect.width() * 0.24)
            .clamp(ui.spacing().interact_size.x * 5.0, button_rect.width());
        let popup_height = row_height * 3.0
            + editor_theme::font::caption().size
            + editor_theme::space::SM
            + f32::from(frame_margin) * 2.0;
        let screen = ui.ctx().content_rect().shrink(editor_theme::space::XXS);
        let popup_x = button_rect.left().clamp(
            screen.left(),
            (screen.right() - popup_width).max(screen.left()),
        );
        let popup_y = if button_rect.bottom() + popup_height <= screen.bottom() {
            button_rect.bottom()
        } else {
            (button_rect.top() - popup_height).max(screen.top())
        };
        let popup = egui::Area::new(menu_id.with("popup"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(popup_x, popup_y))
            .show(ui.ctx(), |ui| {
                egui::Frame::new()
                    .fill(editor_theme::semantic().surface)
                    .stroke(egui::Stroke::new(1.0_f32, editor_theme::semantic().grid))
                    .inner_margin(egui::Margin::same(frame_margin))
                    .show(ui, |ui| {
                        ui.set_min_width(popup_width);
                        let oscillator_key = ui.input_mut(|input| {
                            input.consume_key(egui::Modifiers::NONE, egui::Key::Num1)
                        });
                        let filter_key = ui.input_mut(|input| {
                            input.consume_key(egui::Modifiers::NONE, egui::Key::Num2)
                        });
                        let group_key = ui.input_mut(|input| {
                            input.consume_key(egui::Modifiers::NONE, egui::Key::Num3)
                        });
                        let oscillator = ui
                            .add_enabled(
                                can_add_oscillator,
                                egui::Button::new("1   OSCILLATOR")
                                    .min_size(egui::vec2(popup_width, row_height)),
                            )
                            .clicked()
                            || (can_add_oscillator && oscillator_key);
                        let filter = ui
                            .add_enabled(
                                can_add_filter,
                                egui::Button::new("2   FILTER")
                                    .min_size(egui::vec2(popup_width, row_height)),
                            )
                            .clicked()
                            || (can_add_filter && filter_key);
                        let group = ui
                            .add_enabled(
                                can_add_group,
                                egui::Button::new("3   GROUP")
                                    .min_size(egui::vec2(popup_width, row_height)),
                            )
                            .clicked()
                            || (can_add_group && group_key);
                        if oscillator {
                            action = Some(GeneratorAddAction::Oscillator);
                        } else if filter {
                            action = Some(GeneratorAddAction::Filter);
                        } else if group {
                            action = Some(GeneratorAddAction::Group);
                        }
                        ui.label(
                            egui::RichText::new("KEYS 1 / 2 / 3")
                                .font(editor_theme::font::caption())
                                .color(editor_theme::semantic().text_muted),
                        );
                    });
            });
        if ui.input(|input| {
            input.pointer.primary_clicked()
                && input.pointer.latest_pos().is_some_and(|pointer| {
                    !button_rect.contains(pointer) && !popup.response.rect.contains(pointer)
                })
        }) {
            open = false;
        }
    }
    if action.is_some() {
        open = false;
    }
    ui.data_mut(|data| data.insert_temp(menu_id, open));
    action
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
        patch.insert_oscillator_with_slot(group_id, 0, slot).is_ok()
    });
    if inserted {
        state.generator_stack.reset_oscillator(slot);
    }
}

fn remove_generator_group(
    state: &PluginContext<KurvParams>,
    group_id: GroupId,
    modules: &[Module],
) {
    if state
        .generator_stack
        .edit(|patch| patch.remove_group(group_id).is_ok())
    {
        if let Ok(mut editor) = state.params().editor_state.lock() {
            editor
                .collapsed_group_ids
                .retain(|id| *id != group_id.get());
        }
        clear_group_bindings(state, group_id);
        for module in modules {
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
}
