use super::super::translucent;
use truce_core::editor::PluginContext;

use crate::generators::{
    GroupId, GroupOutput, MAX_OSCILLATORS, MAX_OUTPUT_PAIRS, ModuleId, OscillatorSlot, Patch,
};
use crate::{KurvParams, editor_theme};

use super::actions::{
    add_filter_to_group, add_generator_group, add_noise_to_group, add_noise_to_new_group,
    add_oscillator_to_group, add_oscillator_to_new_group, add_resynth_to_group,
    add_resynth_to_new_group, cleanup_removed_group, next_filter_slot,
};
use super::add_menu::{self, GeneratorAddAction};
use super::group_card::group_accent;
use super::layout::GeneratorInsertionTarget;

fn module_new_group_target(
    left: f32,
    right: f32,
    edge: f32,
    leading_height: f32,
    trailing_bottom: Option<f32>,
) -> egui::Rect {
    egui::Rect::from_min_max(
        egui::pos2(left, edge - leading_height.max(0.0)),
        egui::pos2(right, trailing_bottom.unwrap_or(edge).max(edge)),
    )
}

fn module_can_form_group(patch: &Patch, module_id: ModuleId) -> bool {
    patch.groups().len() < MAX_OUTPUT_PAIRS
        || patch
            .groups()
            .iter()
            .any(|group| group.modules().len() == 1 && group.modules()[0].id() == module_id)
}

pub(super) fn draw_generator_insert_zone(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    patch: &Patch,
    insertion: usize,
    active_insertion: Option<GeneratorInsertionTarget>,
    _card_height: f32,
    _filter_height: f32,
    section_gap: f32,
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
                GeneratorAddAction::Resynth => {
                    let next = (0..MAX_OSCILLATORS)
                        .filter_map(OscillatorSlot::from_index)
                        .find(|slot| !patch.contains_oscillator_slot(*slot));
                    if let Some(slot) = next {
                        add_resynth_to_new_group(state, slot, insertion);
                    }
                }
                GeneratorAddAction::Noise => {
                    let next = (0..MAX_OSCILLATORS)
                        .filter_map(OscillatorSlot::from_index)
                        .find(|slot| !patch.contains_oscillator_slot(*slot));
                    if let Some(slot) = next {
                        add_noise_to_new_group(state, slot, insertion);
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

    let group_target = egui::Rect::from_min_max(
        egui::pos2(ui.cursor().left(), edge - row_height * 0.50),
        egui::pos2(ui.cursor().right(), edge + row_height * 0.50),
    );
    // A module may form a new group only from the real gap before this edge.
    // Keeping this target out of the following header lets a collapsed header
    // unambiguously mean “append to this group”. Group reordering retains the
    // larger boundary target so the first group remains reachable.
    let trailing_bottom = (insertion == patch.groups().len()).then(|| ui.clip_rect().bottom());
    let module_target = module_new_group_target(
        ui.cursor().left(),
        ui.cursor().right(),
        edge,
        if insertion == 0 {
            row_height * 0.5
        } else {
            section_gap
        },
        trailing_bottom,
    );
    let group_response = ui
        .interact(
            group_target,
            egui::Id::new(("generator-group-stack-insert", insertion)),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::Grabbing);
    let module_response = ui
        .interact(
            module_target,
            egui::Id::new(("generator-module-new-group-insert", insertion)),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::Grabbing);
    let module_hovered = module_response.dnd_hover_payload::<ModuleId>().is_some();
    let dragged_module_id = egui::DragAndDrop::payload::<ModuleId>(ui.ctx());
    let group_hovered = group_response.dnd_hover_payload::<GroupId>().is_some();
    let module_at_capacity = module_hovered
        && dragged_module_id
            .as_deref()
            .is_some_and(|module_id| !module_can_form_group(patch, *module_id));
    let color = if module_at_capacity {
        editor_theme::semantic().text_muted
    } else {
        editor_theme::semantic().primary
    };
    if module_hovered && !module_at_capacity {
        let pointer_y = ui
            .input(|input| input.pointer.latest_pos())
            .map_or(edge, |pointer| {
                pointer.y.clamp(module_target.top(), module_target.bottom())
            });
        let marker = egui::Rect::from_center_size(
            egui::pos2(module_target.center().x, pointer_y),
            egui::vec2(module_target.width(), row_height),
        )
        .intersect(module_target.expand2(egui::vec2(0.0, row_height * 0.5)));
        paint_generator_drop_placeholder(ui, marker, color, "MOVE TO OWN GROUP");
    }
    if group_hovered {
        paint_module_insertion_marker(ui, group_target, edge, color);
    }
    if module_at_capacity {
        let line_inset = module_target.width() * 0.012;
        ui.painter().text(
            module_target.right_center() - egui::vec2(line_inset, 0.0),
            egui::Align2::RIGHT_CENTER,
            format!("{MAX_OUTPUT_PAIRS} GROUP LIMIT"),
            editor_theme::font::caption(),
            color,
        );
    }
    if let Some(module_id) = module_response.dnd_release_payload::<ModuleId>()
        && module_can_form_group(patch, *module_id)
    {
        move_module_to_new_group(state, *module_id, insertion);
    } else if let Some(group_id) = group_response.dnd_release_payload::<GroupId>() {
        move_group_to_insertion(state, patch, *group_id, insertion);
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
    let target = header;
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
            group_accent(state, group_id),
            "MOVE INTO GROUP",
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
        move_module_to_group(state, *module_id, group_id, insertion);
        if let Ok(mut editor) = state.params().editor_state.lock() {
            editor
                .collapsed_group_ids
                .retain(|collapsed| *collapsed != group_id.get());
        }
        editor_theme::request_display_repaint(ui);
    }
}

fn module_insertion_geometry(
    pointer_y: f32,
    module_top: f32,
    heights: &[f32],
    gap: f32,
) -> (usize, f32) {
    let mut best = (0, module_top);
    let mut best_distance = (pointer_y - module_top).abs();
    let mut bottom = module_top;
    for (index, height) in heights.iter().copied().enumerate() {
        bottom += height;
        let edge = if index + 1 < heights.len() {
            bottom + gap * 0.5
        } else {
            bottom
        };
        let distance = (pointer_y - edge).abs();
        // Stable ties prefer the earlier/less structural insertion.
        if distance < best_distance {
            best = (index + 1, edge);
            best_distance = distance;
        }
        if index + 1 < heights.len() {
            bottom += gap;
        }
    }
    best
}

pub(super) fn draw_expanded_group_drop_zone(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    patch: &Patch,
    group_id: GroupId,
    group_bounds: egui::Rect,
    module_top: f32,
    card_height: f32,
    filter_height: f32,
    gap: f32,
) {
    if !egui::DragAndDrop::has_payload_of_type::<ModuleId>(ui.ctx()) {
        return;
    }
    let Some(group) = patch.groups().iter().find(|group| group.id() == group_id) else {
        return;
    };
    let heights = group
        .modules()
        .iter()
        .map(|module| super::module_height(state, module.kind(), card_height, filter_height))
        .collect::<Vec<_>>();
    let response = ui
        .interact(
            group_bounds,
            egui::Id::new(("generator-expanded-group-drop", group_id.get())),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::Grabbing);
    let pointer_y = ui
        .input(|input| input.pointer.latest_pos())
        .map_or(module_top, |pointer| pointer.y);
    let (insertion, edge) = module_insertion_geometry(pointer_y, module_top, &heights, gap);
    if response.dnd_hover_payload::<ModuleId>().is_some() {
        paint_module_insertion_marker(ui, group_bounds, edge, group_accent(state, group_id));
    }
    if let Some(module_id) = response.dnd_release_payload::<ModuleId>() {
        move_module_to_group(state, *module_id, group_id, insertion);
        editor_theme::request_display_repaint(ui);
    }
}

fn move_module_to_new_group(
    state: &PluginContext<KurvParams>,
    module_id: ModuleId,
    insertion: usize,
) {
    let removed_group = state.generator_stack.edit(|patch| {
        let insertion = insertion.min(patch.groups().len());
        let Some((source_group, source_index, source_was_sole)) = patch
            .groups()
            .iter()
            .enumerate()
            .find_map(|(index, group)| {
                group
                    .modules()
                    .iter()
                    .any(|module| module.id() == module_id)
                    .then_some((group.id(), index, group.modules().len() == 1))
            })
        else {
            return None;
        };
        if source_was_sole {
            let target = if source_index < insertion {
                insertion.saturating_sub(1)
            } else {
                insertion
            }
            .min(patch.groups().len().saturating_sub(1));
            if target != source_index {
                let _ = patch.move_group(source_group, target);
            }
            return None;
        }
        let Ok(group_id) = patch.insert_group(insertion) else {
            return None;
        };
        let output = GroupOutput {
            pair: (insertion % MAX_OUTPUT_PAIRS) as u8,
            ..GroupOutput::default()
        };
        if patch.set_group_output(group_id, output).is_err()
            || patch.move_module(module_id, group_id, 0).is_err()
        {
            let _ = patch.remove_group(group_id);
            return None;
        }
        source_was_sole
            .then(|| patch.remove_group(source_group).ok())
            .flatten()
    });
    if let Some(group) = removed_group {
        cleanup_removed_group(state, group);
    }
}

pub(super) fn draw_group_footer_add(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    patch: &Patch,
    group_id: GroupId,
    footer: egui::Rect,
    accent: egui::Color32,
) -> f32 {
    let next_oscillator = (0..MAX_OSCILLATORS)
        .filter_map(OscillatorSlot::from_index)
        .find(|slot| !patch.contains_oscillator_slot(*slot));
    let next_filter = next_filter_slot(patch);
    let side = editor_theme::title_height(ui) * 0.72;
    let plus = egui::Rect::from_center_size(
        egui::pos2(footer.center().x, footer.bottom()),
        egui::Vec2::splat(side),
    );
    if let Some(action) = add_menu::show_group_footer_add(
        ui,
        group_id.get(),
        plus,
        accent,
        next_oscillator.is_some(),
        next_filter.is_some(),
        patch.groups().len() < MAX_OUTPUT_PAIRS,
    ) {
        let insertion = patch
            .groups()
            .iter()
            .find(|group| group.id() == group_id)
            .map_or(0, |group| group.modules().len());
        match action {
            GeneratorAddAction::Oscillator => {
                if let Some(slot) = next_oscillator {
                    add_oscillator_to_group(state, group_id, insertion, slot);
                }
            }
            GeneratorAddAction::Resynth => {
                if let Some(slot) = next_oscillator {
                    add_resynth_to_group(state, group_id, insertion, slot);
                }
            }
            GeneratorAddAction::Noise => {
                if let Some(slot) = next_oscillator {
                    add_noise_to_group(state, group_id, insertion, slot);
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
    plus.width() + editor_theme::space::SM
}

pub(super) fn draw_group_module_insert_zone(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    patch: &Patch,
    group_id: GroupId,
    insertion: usize,
    active_insertion: Option<GeneratorInsertionTarget>,
    always: bool,
) {
    let target_id = GeneratorInsertionTarget::Module(group_id.get(), insertion);
    if always || active_insertion == Some(target_id) {
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
                GeneratorAddAction::Resynth => {
                    if let Some(slot) = next_oscillator {
                        add_resynth_to_group(state, group_id, insertion, slot);
                    }
                }
                GeneratorAddAction::Noise => {
                    if let Some(slot) = next_oscillator {
                        add_noise_to_group(state, group_id, insertion, slot);
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
}

fn paint_module_insertion_marker(
    ui: &egui::Ui,
    target: egui::Rect,
    edge: f32,
    color: egui::Color32,
) {
    let y = edge.clamp(target.top(), target.bottom());
    let inset = editor_theme::space::XS;
    ui.painter().line_segment(
        [
            egui::pos2(target.left() + inset, y),
            egui::pos2(target.right() - inset, y),
        ],
        egui::Stroke::new(editor_theme::shape::FOCUS_STROKE * 1.5, color),
    );
    for x in [target.left() + inset, target.right() - inset] {
        ui.painter().circle_filled(
            egui::pos2(x, y),
            editor_theme::shape::FOCUS_STROKE * 1.5,
            color,
        );
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
    module_id: ModuleId,
    destination: GroupId,
    insertion: usize,
) {
    let removed_group = state.generator_stack.edit(|patch| {
        let Some((source_group, source_index, source_was_sole)) =
            patch.groups().iter().find_map(|group| {
                group
                    .modules()
                    .iter()
                    .position(|module| module.id() == module_id)
                    .map(|index| (group.id(), index, group.modules().len() == 1))
            })
        else {
            return None;
        };
        let Some(destination_len) = patch
            .groups()
            .iter()
            .find(|group| group.id() == destination)
            .map(|group| group.modules().len())
        else {
            return None;
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
        if source_group == destination && source_index == target {
            return None;
        }
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

#[cfg(test)]
mod tests {
    use super::{module_can_form_group, module_insertion_geometry, module_new_group_target};
    use crate::generators::{MAX_OUTPUT_PAIRS, Patch};

    #[test]
    fn new_group_module_target_does_not_overlap_the_following_header() {
        let edge = 100.0;
        let target = module_new_group_target(0.0, 400.0, edge, 12.0, None);
        let previous_group =
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(400.0, edge - 12.0));
        let header =
            egui::Rect::from_min_max(egui::pos2(0.0, edge), egui::pos2(400.0, edge + 24.0));
        assert!(!target.intersect(previous_group).is_positive());
        assert!(!target.intersect(header).is_positive());
    }

    #[test]
    fn trailing_empty_rack_is_a_new_group_target() {
        let target = module_new_group_target(0.0, 400.0, 100.0, 12.0, Some(500.0));
        assert_eq!(target.top(), 88.0);
        assert_eq!(target.bottom(), 500.0);
        assert!(target.contains(egui::pos2(200.0, 420.0)));
    }

    #[test]
    fn whole_expanded_group_resolves_to_the_closest_module_boundary() {
        let heights = [160.0, 100.0];
        assert_eq!(
            module_insertion_geometry(40.0, 100.0, &heights, 4.0),
            (0, 100.0)
        );
        assert_eq!(
            module_insertion_geometry(180.0, 100.0, &heights, 4.0),
            (0, 100.0)
        );
        assert_eq!(
            module_insertion_geometry(181.0, 100.0, &heights, 4.0),
            (0, 100.0)
        );
        assert_eq!(
            module_insertion_geometry(182.0, 100.0, &heights, 4.0),
            (1, 262.0)
        );
        assert_eq!(
            module_insertion_geometry(262.0, 100.0, &heights, 4.0),
            (1, 262.0)
        );
        assert_eq!(
            module_insertion_geometry(313.0, 100.0, &heights, 4.0),
            (1, 262.0)
        );
        assert_eq!(
            module_insertion_geometry(314.0, 100.0, &heights, 4.0),
            (2, 364.0)
        );
        assert_eq!(
            module_insertion_geometry(380.0, 100.0, &heights, 4.0),
            (2, 364.0)
        );
    }

    #[test]
    fn sole_module_group_can_move_at_group_capacity_without_creating_another_group() {
        let mut patch = Patch::new();
        let module = patch.groups()[0].modules()[0].id();
        while patch.groups().len() < MAX_OUTPUT_PAIRS {
            patch
                .insert_group(patch.groups().len())
                .expect("group fits");
        }
        assert!(module_can_form_group(&patch, module));
    }

    #[test]
    fn multi_module_group_requires_capacity_for_extraction() {
        let mut patch = Patch::new();
        let group = patch.groups()[0].id();
        let module = patch.groups()[0].modules()[0].id();
        patch.insert_oscillator(group, 1).expect("oscillator fits");
        while patch.groups().len() < MAX_OUTPUT_PAIRS {
            patch
                .insert_group(patch.groups().len())
                .expect("group fits");
        }
        assert!(!module_can_form_group(&patch, module));
    }
}
