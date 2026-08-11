use truce_core::editor::PluginContext;

use crate::generators::{GroupId, ModuleId, ModuleKind, Patch};
use crate::{KurvParams, editor_theme};

use super::super::MODULE_IDENTITY_SHARE;
use super::add_menu;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum GeneratorInsertionTarget {
    Group(usize),
    Module(u64, usize),
}

#[derive(Clone, Copy)]
pub(super) struct GeneratorInsertionCandidate {
    target: GeneratorInsertionTarget,
    left: f32,
    right: f32,
    edge: f32,
}

pub(super) fn generator_active_insertion_id() -> egui::Id {
    egui::Id::new("generator-alt-insertion-active")
}

pub(super) fn outside_lane_width(width: f32, row_height: f32) -> f32 {
    (width * MODULE_IDENTITY_SHARE).max(row_height)
}

pub(super) fn active_generator_insertion(
    ui: &egui::Ui,
    viewport: egui::Rect,
    candidates: &[GeneratorInsertionCandidate],
    sticky: Option<GeneratorInsertionTarget>,
) -> Option<GeneratorInsertionTarget> {
    if let Some(open) = candidates
        .iter()
        .find(|candidate| add_menu::insertion_open(ui, candidate.target))
        .map(|candidate| candidate.target)
    {
        return Some(open);
    }

    let (alt, pointer) = ui.input(|input| (input.modifiers.alt, input.pointer.latest_pos()));
    if !alt
        || ui.ctx().dragged_id().is_some()
        || egui::DragAndDrop::has_payload_of_type::<ModuleId>(ui.ctx())
        || egui::DragAndDrop::has_payload_of_type::<GroupId>(ui.ctx())
        || crate::editor_modulation::source_drag_active(ui)
    {
        return None;
    }
    let pointer = pointer?;
    if !viewport.contains(pointer) {
        return None;
    }

    let row_height = editor_theme::title_height(ui);
    let sticky_radius = row_height * 0.72;
    if let Some(sticky) = sticky
        && let Some(candidate) = candidates
            .iter()
            .find(|candidate| candidate.target == sticky)
        && (candidate.left..=candidate.right).contains(&pointer.x)
        && (candidate.edge - pointer.y).abs() <= sticky_radius
    {
        return Some(sticky);
    }
    if ui.ctx().egui_is_using_pointer() {
        return None;
    }

    let discovery_radius =
        editor_theme::insertion_discovery_radius(ui).max(ui.spacing().item_spacing.y * 0.5);
    candidates
        .iter()
        .filter(|candidate| (candidate.left..=candidate.right).contains(&pointer.x))
        .filter(|candidate| (candidate.edge - pointer.y).abs() <= discovery_radius)
        .min_by(|left, right| {
            (left.edge - pointer.y)
                .abs()
                .total_cmp(&(right.edge - pointer.y).abs())
        })
        .map(|candidate| candidate.target)
}

pub(super) fn generator_insertion_candidates(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    patch: &Patch,
    card_height: f32,
    filter_height: f32,
    group_header_height: f32,
    output_height: f32,
    section_gap: f32,
    reserved: Option<GeneratorInsertionTarget>,
) -> Vec<GeneratorInsertionCandidate> {
    let row_height = editor_theme::title_height(ui);
    let module_gap = editor_theme::space::XXS;
    let left = ui.cursor().left();
    let right = ui.cursor().right();
    let collapsed = state.params().editor_state.lock().ok();
    let mut candidates = Vec::new();
    let mut edge = ui.cursor().top();

    for (group_index, group) in patch.groups().iter().enumerate() {
        let group_target = GeneratorInsertionTarget::Group(group_index);
        candidates.push(GeneratorInsertionCandidate {
            target: group_target,
            left,
            right,
            edge,
        });
        if add_menu::insertion_open(ui, group_target) || reserved == Some(group_target) {
            edge += row_height;
        }
        edge += group_header_height;

        let group_id = group.id();
        let modules = group.modules();
        let is_collapsed = collapsed
            .as_ref()
            .is_some_and(|editor| editor.collapsed_group_ids.contains(&group_id.get()));
        let module_range = if is_collapsed {
            modules.len()..modules.len() + 1
        } else {
            0..modules.len()
        };
        for insertion in module_range {
            let target = GeneratorInsertionTarget::Module(group_id.get(), insertion);
            candidates.push(GeneratorInsertionCandidate {
                target,
                left,
                right,
                edge,
            });
            if add_menu::insertion_open(ui, target) || reserved == Some(target) {
                edge += row_height;
            }
            if !is_collapsed && insertion < modules.len() {
                edge += match modules[insertion].kind() {
                    ModuleKind::Oscillator(_) => card_height,
                    ModuleKind::Filter(_) => filter_height,
                };
                if insertion + 1 < modules.len() {
                    edge += module_gap;
                }
            }
        }
        if !is_collapsed {
            edge += module_gap + row_height + output_height;
        }
        edge += section_gap;
    }
    candidates
}
