use super::super::translucent;
use truce_core::editor::PluginContext;

use crate::generators::{
    GroupId, GroupOutput, MAX_OSCILLATORS, MAX_OUTPUT_PAIRS, ModuleId, ModuleKind, OscillatorSlot,
    Patch,
};
use crate::{KurvParams, editor_theme};

use super::actions::{
    add_filter_to_group, add_generator_group, add_oscillator_to_group, add_oscillator_to_new_group,
    cleanup_removed_group, next_filter_slot,
};
use super::add_menu::{self, GeneratorAddAction};
use super::group_card::{group_accent, group_accent_index};
use super::layout::{self, GeneratorInsertionTarget};

fn module_can_form_group(patch: &Patch, _module_id: ModuleId) -> bool {
    patch.groups().len() < MAX_OUTPUT_PAIRS
}

fn dragged_module_height(
    ui: &egui::Ui,
    patch: &Patch,
    card_height: f32,
    filter_height: f32,
) -> f32 {
    egui::DragAndDrop::payload::<ModuleId>(ui.ctx())
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
        .unwrap_or(card_height)
}

fn module_placeholder_id(group_id: GroupId, insertion: usize) -> egui::Id {
    egui::Id::new(("generator-module-placeholder", group_id.get(), insertion))
}

pub(super) fn active_group_drag_placeholder_height(
    ui: &egui::Ui,
    patch: &Patch,
    group_id: GroupId,
    card_height: f32,
    filter_height: f32,
) -> f32 {
    if !egui::DragAndDrop::has_payload_of_type::<ModuleId>(ui.ctx()) {
        return 0.0;
    }
    let Some(group) = patch.groups().iter().find(|group| group.id() == group_id) else {
        return 0.0;
    };
    let placeholder_open = (0..=group.modules().len()).any(|insertion| {
        ui.data(|data| {
            data.get_temp::<bool>(module_placeholder_id(group_id, insertion))
                .unwrap_or(false)
        })
    });
    if placeholder_open {
        dragged_module_height(ui, patch, card_height, filter_height)
    } else {
        0.0
    }
}

pub(super) fn draw_group_outside_drop_lane(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    patch: &Patch,
    group_rect: egui::Rect,
    group_index: usize,
) {
    let Some(module_id) = egui::DragAndDrop::payload::<ModuleId>(ui.ctx()) else {
        return;
    };
    let lane_width = layout::outside_lane_width(group_rect.width(), editor_theme::title_height(ui));
    let left_lane = egui::Rect::from_min_max(
        group_rect.min,
        egui::pos2(
            (group_rect.left() + lane_width).min(group_rect.right()),
            group_rect.bottom(),
        ),
    );
    let right_lane = egui::Rect::from_min_max(
        egui::pos2(
            (group_rect.right() - lane_width).max(group_rect.left()),
            group_rect.top(),
        ),
        group_rect.max,
    );
    let pointer = ui.ctx().pointer_interact_pos();
    let hovered =
        pointer.is_some_and(|pointer| left_lane.contains(pointer) || right_lane.contains(pointer));
    let at_capacity = hovered && !module_can_form_group(patch, *module_id);
    if hovered {
        ui.ctx().set_cursor_icon(if at_capacity {
            egui::CursorIcon::NotAllowed
        } else {
            egui::CursorIcon::Grabbing
        });
        let color = if at_capacity {
            editor_theme::semantic().text_muted
        } else {
            editor_theme::semantic().primary
        };
        let after = pointer.is_some_and(|pointer| pointer.y >= group_rect.center().y);
        let edge = if after {
            group_rect.bottom()
        } else {
            group_rect.top()
        };
        let preview_height = editor_theme::title_height(ui);
        let preview = egui::Rect::from_min_size(
            egui::pos2(group_rect.left(), edge - preview_height * 0.5),
            egui::vec2(group_rect.width(), preview_height),
        );
        ui.scope_builder(
            egui::UiBuilder::new().layer_id(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new(("generator-module-extraction-preview", group_index)),
            )),
            |ui| {
                paint_generator_drop_placeholder(
                    ui,
                    preview,
                    color,
                    if at_capacity {
                        "GROUP LIMIT"
                    } else {
                        "DROP MODULE · NEW GROUP"
                    },
                );
            },
        );
    }
    if hovered
        && !at_capacity
        && ui.input(|input| input.pointer.any_released())
        && let Some(module_id) = egui::DragAndDrop::take_payload::<ModuleId>(ui.ctx())
    {
        let insertion = pointer
            .is_some_and(|pointer| pointer.y >= group_rect.center().y)
            .then_some(group_index + 1)
            .unwrap_or(group_index);
        move_module_to_new_group(state, *module_id, insertion);
    }
}

pub(super) fn draw_rack_background_drop_zone(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    patch: &Patch,
    card_height: f32,
    filter_height: f32,
) {
    if !egui::DragAndDrop::has_payload_of_type::<ModuleId>(ui.ctx()) {
        return;
    }
    let background = egui::Rect::from_min_max(
        ui.cursor().left_top(),
        egui::pos2(ui.cursor().right(), ui.clip_rect().bottom()),
    )
    .intersect(ui.clip_rect());
    if !background.is_positive() {
        return;
    }
    let response = ui
        .interact(
            background,
            egui::Id::new("generator-module-new-group-background"),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::Grabbing);
    let hovered_module = response.dnd_hover_payload::<ModuleId>();
    let hovered = hovered_module.is_some();
    let at_capacity = hovered_module
        .as_deref()
        .is_some_and(|module_id| !module_can_form_group(patch, *module_id));
    if hovered {
        let module_height = egui::DragAndDrop::payload::<ModuleId>(ui.ctx())
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
        let preview = egui::Rect::from_min_size(
            background.min,
            egui::vec2(background.width(), module_height.min(background.height())),
        );
        paint_generator_drop_placeholder(
            ui,
            preview,
            if at_capacity {
                editor_theme::semantic().text_muted
            } else {
                editor_theme::semantic().primary
            },
            if at_capacity {
                "GROUP LIMIT"
            } else {
                "DROP MODULE · NEW GROUP"
            },
        );
    }
    if let Some(module_id) = response.dnd_release_payload::<ModuleId>()
        && module_can_form_group(patch, *module_id)
    {
        move_module_to_new_group(state, *module_id, patch.groups().len());
    }
}

pub(super) fn draw_generator_insert_zone(
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
    let row_height = editor_theme::title_height(ui);
    let edge = ui.cursor().top();
    if insertion < patch.groups().len() && active_insertion == Some(target_id) {
        let can_add_group = patch.groups().len() < MAX_OUTPUT_PAIRS;
        if let Some(action) = add_menu::show_insertion(
            ui,
            target_id,
            patch.oscillator_count() < MAX_OSCILLATORS && can_add_group,
            false,
            can_add_group,
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
    let group_response = ui
        .interact(
            target,
            egui::Id::new(("generator-group-stack-insert", insertion)),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::Grabbing);
    let module_response = ui
        .interact(
            target,
            egui::Id::new(("generator-module-new-group-insert", insertion)),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::Grabbing);
    let module_hovered = module_response.dnd_hover_payload::<ModuleId>().is_some();
    let dragged_module_id = egui::DragAndDrop::payload::<ModuleId>(ui.ctx());
    let dragged_module_height = dragged_module_height(ui, patch, card_height, filter_height);
    let group_hovered = group_response.dnd_hover_payload::<GroupId>().is_some();
    let placeholder_id = egui::Id::new(("generator-new-group-placeholder", insertion));
    let placeholder_open = module_drag
        && ui
            .data(|data| data.get_temp::<bool>(placeholder_id))
            .unwrap_or(false);
    let module_at_capacity = (module_hovered || placeholder_open)
        && dragged_module_id
            .as_deref()
            .is_some_and(|module_id| !module_can_form_group(patch, *module_id));
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
        paint_generator_drop_placeholder(ui, placeholder, color, "DROP MODULE · NEW GROUP");
        let keep_open = ui.input(|input| {
            input.pointer.primary_down()
                && input.pointer.latest_pos().is_some_and(|pointer| {
                    placeholder.expand(row_height * 0.35).contains(pointer)
                        || target.contains(pointer)
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
        paint_generator_drop_placeholder(ui, placeholder, color, "DROP GROUP");
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
        && module_can_form_group(patch, *module_id)
    {
        move_module_to_new_group(state, *module_id, insertion);
        ui.data_mut(|data| data.insert_temp(placeholder_id, false));
    } else if let Some(group_id) =
        group_placeholder_release.or_else(|| group_response.dnd_release_payload::<GroupId>())
    {
        move_group_to_insertion(state, patch, *group_id, insertion);
        ui.data_mut(|data| data.insert_temp(group_placeholder_id, false));
    }
}

pub(super) fn draw_collapsed_group_drop_zone(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    patch: &Patch,
    group_id: GroupId,
    header: egui::Rect,
) {
    if !egui::DragAndDrop::has_payload_of_type::<ModuleId>(ui.ctx()) {
        return;
    }
    let lane_width = layout::outside_lane_width(header.width(), editor_theme::title_height(ui));
    let target = egui::Rect::from_min_max(
        egui::pos2(
            (header.left() + lane_width).min(header.right()),
            header.top(),
        ),
        egui::pos2(
            (header.right() - lane_width).max(header.left()),
            header.bottom(),
        ),
    );
    let response = ui
        .interact(
            target,
            egui::Id::new(("generator-collapsed-group-drop", group_id.get())),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::Grabbing);
    let valid = egui::DragAndDrop::payload::<ModuleId>(ui.ctx())
        .as_deref()
        .is_some_and(|module_id| {
            patch.groups().iter().any(|group| {
                group
                    .modules()
                    .iter()
                    .any(|module| module.id() == *module_id)
            })
        });
    if valid && response.dnd_hover_payload::<ModuleId>().is_some() {
        paint_generator_drop_placeholder(
            ui,
            target.shrink(editor_theme::space::XXS),
            group_accent(group_accent_index(state, group_id)),
            "DROP MODULE",
        );
    }
    if valid
        && let Some(module_id) = response.dnd_release_payload::<ModuleId>()
        && let Some(insertion) = patch
            .groups()
            .iter()
            .find(|group| group.id() == group_id)
            .map(|group| group.modules().len())
    {
        move_module_to_group(state, patch, *module_id, group_id, insertion);
        if let Ok(mut editor) = state.params().editor_state.lock() {
            editor
                .collapsed_group_ids
                .retain(|collapsed| *collapsed != group_id.get());
        }
        editor_theme::request_display_repaint(ui);
    }
}

fn move_module_to_new_group(
    state: &PluginContext<KurvParams>,
    module_id: ModuleId,
    insertion: usize,
) {
    state.generator_stack.edit(|patch| {
        let insertion = insertion.min(patch.groups().len());
        let source_exists = patch.groups().iter().any(|group| {
            group
                .modules()
                .iter()
                .any(|module| module.id() == module_id)
        });
        if !source_exists {
            return;
        }
        let Ok(group_id) = patch.insert_group(insertion) else {
            return;
        };
        let output = GroupOutput {
            pair: (insertion % MAX_OUTPUT_PAIRS) as u8,
            ..GroupOutput::default()
        };
        if patch.set_group_output(group_id, output).is_err()
            || patch.move_module(module_id, group_id, 0).is_err()
        {
            let _ = patch.remove_group(group_id);
        }
    });
}

pub(super) fn draw_group_module_insert_zone(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    patch: &Patch,
    group_id: GroupId,
    insertion: usize,
    active_insertion: Option<GeneratorInsertionTarget>,
    card_height: f32,
    filter_height: f32,
    expand_on_drop: bool,
) {
    let module_drag = egui::DragAndDrop::has_payload_of_type::<ModuleId>(ui.ctx());
    let target_id = GeneratorInsertionTarget::Module(group_id.get(), insertion);
    let row_height = editor_theme::title_height(ui);
    let edge = ui.cursor().top();
    let outside_lane_width = layout::outside_lane_width(ui.available_width(), row_height);
    if active_insertion == Some(target_id) {
        let next_oscillator = (0..MAX_OSCILLATORS)
            .filter_map(OscillatorSlot::from_index)
            .find(|slot| !patch.contains_oscillator_slot(*slot));
        let next_filter = next_filter_slot(patch);
        if let Some(action) = add_menu::show_insertion(
            ui,
            target_id,
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
        egui::pos2(
            (target.right() - outside_lane_width).max(target.left()),
            target.bottom(),
        ),
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
    let dragged_module_height = dragged_module_height(ui, patch, card_height, filter_height);
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
    let color = group_accent(group_accent_index(state, group_id));
    let placeholder_id = module_placeholder_id(group_id, insertion);
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
        paint_generator_drop_placeholder(ui, placeholder, color, "DROP MODULE");
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
        if expand_on_drop && let Ok(mut editor) = state.params().editor_state.lock() {
            editor
                .collapsed_group_ids
                .retain(|collapsed| *collapsed != group_id.get());
        }
        ui.data_mut(|data| data.insert_temp(placeholder_id, false));
        editor_theme::request_display_repaint(ui);
    }
}

fn paint_generator_drop_placeholder(
    ui: &egui::Ui,
    rect: egui::Rect,
    color: egui::Color32,
    label: &str,
) {
    ui.painter().rect_filled(
        rect,
        editor_theme::shape::CONTROL_RADIUS,
        translucent(color, 14),
    );
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
        editor_theme::space::SM,
        editor_theme::space::XS,
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
    let Some((source_group, source_index, source_was_sole)) =
        patch.groups().iter().find_map(|group| {
            group
                .modules()
                .iter()
                .position(|module| module.id() == module_id)
                .map(|index| (group.id(), index, group.modules().len() == 1))
        })
    else {
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
        let removed_group = state.generator_stack.edit(|patch| {
            if patch.move_module(module_id, destination, target).is_err()
                || source_group == destination
                || !source_was_sole
            {
                return None;
            }
            patch.remove_group(source_group).ok()
        });
        if let Some(group) = removed_group {
            cleanup_removed_group(state, group);
        }
    }
}
