use truce_core::editor::PluginContext;

use crate::KurvParams;
use crate::generators::{
    FilterConfig, FilterSlot, Group, GroupId, GroupOutput, MAX_FILTERS, MAX_OUTPUT_PAIRS,
    ModuleKind, OscillatorSlot, Patch,
};

use super::super::{clear_group_bindings, clear_module_bindings};

pub(super) fn add_oscillator_to_group(
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

pub(super) fn add_resynth_to_group(
    state: &PluginContext<KurvParams>,
    group_id: GroupId,
    insertion: usize,
    slot: OscillatorSlot,
) {
    add_oscillator_to_group(state, group_id, insertion, slot);
    let mut config = state.generator_stack.oscillator_config(slot);
    config.engine = crate::generators::OscillatorEngineKind::Resynth;
    state.generator_stack.set_oscillator_config(slot, config);
}

pub(super) fn add_resynth_to_new_group(
    state: &PluginContext<KurvParams>,
    slot: OscillatorSlot,
    insertion: usize,
) {
    add_oscillator_to_new_group(state, slot, insertion);
    let mut config = state.generator_stack.oscillator_config(slot);
    config.engine = crate::generators::OscillatorEngineKind::Resynth;
    state.generator_stack.set_oscillator_config(slot, config);
}

pub(super) fn next_filter_slot(patch: &Patch) -> Option<FilterSlot> {
    (0..MAX_FILTERS)
        .filter_map(FilterSlot::from_index)
        .find(|slot| !patch.contains_filter_slot(*slot))
}

pub(super) fn add_filter_to_group(
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

pub(super) fn add_generator_group(state: &PluginContext<KurvParams>, insertion: usize) {
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

pub(super) fn add_oscillator_to_new_group(
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

pub(super) fn remove_generator_group(state: &PluginContext<KurvParams>, group_id: GroupId) {
    if let Ok(group) = state
        .generator_stack
        .edit(|patch| patch.remove_group(group_id))
    {
        cleanup_removed_group(state, group);
    }
}

pub(super) fn cleanup_removed_group(state: &PluginContext<KurvParams>, group: Group) {
    if let Ok(mut editor) = state.params().editor_state.lock() {
        editor
            .collapsed_group_ids
            .retain(|id| *id != group.id().get());
        editor
            .group_accents
            .retain(|accent| accent.group_id != group.id().get());
        editor
            .group_names
            .retain(|stored| stored.group_id != group.id().get());
    }
    clear_group_bindings(state, group.id());
    for module in group.modules() {
        clear_module_bindings(state, module.id());
        match module.kind() {
            ModuleKind::Oscillator(slot) => {
                let mut config = state.generator_stack.oscillator_config(slot);
                config.enabled = false;
                state.generator_stack.set_oscillator_config(slot, config);
                if let Some(asset) = state.resynth_assets.slot(slot.index()) {
                    asset.clear();
                }
            }
            ModuleKind::Filter(slot) => state
                .generator_stack
                .set_filter_config(slot, FilterConfig::default()),
        }
    }
}
