use super::*;

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
    let outside_lane_width = (row_height + editor_theme::space::SM).max(card_height * 0.30);
    if insertion < patch.groups().len() && active_insertion == Some(target_id) {
        if let Some(action) = add_menu::show_insertion(
            ui,
            target_id,
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

pub(super) fn draw_group_module_insert_zone(
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
    let row_height = editor_theme::title_height(ui);
    let edge = ui.cursor().top();
    let outside_lane_width = (row_height + editor_theme::space::SM).max(card_height * 0.30);
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
