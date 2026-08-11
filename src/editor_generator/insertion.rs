use truce_core::editor::PluginContext;

use crate::KurvParams;
use crate::editor_widgets::{drag_edge_scroll, with_child};
use crate::generators::{
    GroupId, GroupOutput, MAX_OSCILLATORS, MAX_OUTPUT_PAIRS, ModuleId, OscillatorSlot,
};

mod actions;
mod add_menu;
mod drag_reorder;
mod group_card;
mod layout;

use actions::{add_generator_group, add_oscillator_to_new_group};
use add_menu::GeneratorAddAction;
use group_card::{GroupCardMetrics, rack_item_visible, show_group_card};
use layout::{
    GeneratorInsertionTarget, active_generator_insertion, generator_active_insertion_id,
    generator_insertion_candidates,
};

pub(crate) fn show(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    rect: egui::Rect,
    gap: f32,
    section_gap: f32,
) {
    let patch = state.generator_stack.snapshot();
    let mut group_output_updates: [Option<(GroupId, GroupOutput)>; MAX_OUTPUT_PAIRS] =
        [None; MAX_OUTPUT_PAIRS];
    let patch_identity = std::sync::Arc::as_ptr(&patch) as usize;
    let patch_identity_id = egui::Id::new("generator-insertion-patch-identity");
    let patch_replaced = ui.data_mut(|data| {
        let previous = data.get_temp::<usize>(patch_identity_id);
        data.insert_temp(patch_identity_id, patch_identity);
        previous.is_some_and(|previous| previous != patch_identity)
    });
    if patch_replaced {
        add_menu::clear_insertion_open(ui);
        ui.data_mut(|data| {
            data.remove::<GeneratorInsertionTarget>(generator_active_insertion_id());
        });
    }
    let root_menu_open = add_menu::root_open(ui);
    let ordinary_menu_open = root_menu_open
        || patch
            .groups()
            .iter()
            .any(|group| add_menu::group_open(ui, group.id()));
    let metrics = GroupCardMetrics::from_rack(ui, rect);
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
                    let cable_drag = crate::editor_modulation::source_drag_active(ui);
                    drag_edge_scroll(ui, rect, structural_drag || cable_drag);
                    let active_id = generator_active_insertion_id();
                    let previous_insertion = (!ordinary_menu_open)
                        .then(|| {
                            ui.data(|data| data.get_temp::<GeneratorInsertionTarget>(active_id))
                        })
                        .flatten();
                    let insertion_requested = previous_insertion.is_some()
                        || (!structural_drag
                            && !cable_drag
                            && ui.ctx().dragged_id().is_none()
                            && ui.input(|input| input.modifiers.alt));
                    let active_insertion = if insertion_requested {
                        let candidates = generator_insertion_candidates(
                            ui,
                            state,
                            &patch,
                            metrics.card_height,
                            metrics.filter_height,
                            metrics.header_height,
                            metrics.output_height,
                            section_gap,
                            previous_insertion,
                        );
                        active_generator_insertion(ui, rect, &candidates, previous_insertion)
                    } else {
                        None
                    }
                    .filter(|_| !ordinary_menu_open);
                    ui.data_mut(|data| {
                        if let Some(active) = active_insertion {
                            data.insert_temp(active_id, active);
                        } else {
                            data.remove::<GeneratorInsertionTarget>(active_id);
                        }
                    });
                    let show_permanent_add_rows = active_insertion.is_none();
                    // Structural drop zones are laid out independently of module painting. Keep
                    // offscreen modules culled during drags; rendering every destination made a
                    // single gesture multiply all route lookups across the entire 32-slot rack.
                    for group_index in 0..patch.groups().len() {
                        drag_reorder::draw_generator_insert_zone(
                            ui,
                            state,
                            &patch,
                            group_index,
                            active_insertion,
                            metrics.card_height,
                            metrics.filter_height,
                        );
                        group_output_updates[group_index] = show_group_card(
                            ui,
                            state,
                            &patch,
                            group_index,
                            active_insertion,
                            metrics,
                            gap,
                            section_gap,
                            show_permanent_add_rows,
                        );
                    }
                    drag_reorder::draw_generator_insert_zone(
                        ui,
                        state,
                        &patch,
                        patch.groups().len(),
                        active_insertion,
                        metrics.card_height,
                        metrics.filter_height,
                    );
                    if show_permanent_add_rows {
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
                                        add_oscillator_to_new_group(
                                            state,
                                            slot,
                                            patch.groups().len(),
                                        );
                                    }
                                }
                                GeneratorAddAction::Filter => {}
                                GeneratorAddAction::Group => {
                                    add_generator_group(state, patch.groups().len());
                                }
                            }
                        }
                    }
                    drag_reorder::draw_rack_background_drop_zone(
                        ui,
                        state,
                        &patch,
                        metrics.card_height,
                        metrics.filter_height,
                    );
                });
        },
    );
    drop(patch);
    for (group_id, output) in group_output_updates.into_iter().flatten() {
        state.generator_stack.set_group_output(group_id, output);
    }
}
