use truce_core::editor::PluginContext;

use crate::KurvParams;
use crate::generators::{
    AuxConfig, AuxSlot, FilterConfig, FilterSlot, Group, GroupId, MAX_AUX_MODULES, MAX_FILTERS,
    ModuleKind, OscillatorConfig, OscillatorEngineKind, OscillatorSlot, Patch,
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
    state.generator_stack.set_oscillator_config(
        slot,
        OscillatorConfig::for_engine(OscillatorEngineKind::Resynth),
    );
}

pub(super) fn add_resynth_to_new_group(
    state: &PluginContext<KurvParams>,
    slot: OscillatorSlot,
    insertion: usize,
) {
    add_oscillator_to_new_group(state, slot, insertion);
    state.generator_stack.set_oscillator_config(
        slot,
        OscillatorConfig::for_engine(OscillatorEngineKind::Resynth),
    );
}

pub(super) fn add_grain_to_group(
    state: &PluginContext<KurvParams>,
    group_id: GroupId,
    insertion: usize,
    slot: OscillatorSlot,
) {
    add_oscillator_to_group(state, group_id, insertion, slot);
    state.generator_stack.set_oscillator_config(
        slot,
        OscillatorConfig::for_engine(OscillatorEngineKind::Grain),
    );
}

pub(super) fn add_grain_to_new_group(
    state: &PluginContext<KurvParams>,
    slot: OscillatorSlot,
    insertion: usize,
) {
    add_oscillator_to_new_group(state, slot, insertion);
    state.generator_stack.set_oscillator_config(
        slot,
        OscillatorConfig::for_engine(OscillatorEngineKind::Grain),
    );
}

pub(super) fn add_noise_to_group(
    state: &PluginContext<KurvParams>,
    group_id: GroupId,
    insertion: usize,
    slot: OscillatorSlot,
) {
    add_oscillator_to_group(state, group_id, insertion, slot);
    state.generator_stack.set_oscillator_config(
        slot,
        OscillatorConfig::for_engine(OscillatorEngineKind::Noise),
    );
}

pub(super) fn add_noise_to_new_group(
    state: &PluginContext<KurvParams>,
    slot: OscillatorSlot,
    insertion: usize,
) {
    add_oscillator_to_new_group(state, slot, insertion);
    state.generator_stack.set_oscillator_config(
        slot,
        OscillatorConfig::for_engine(OscillatorEngineKind::Noise),
    );
}

pub(super) fn next_filter_slot(patch: &Patch) -> Option<FilterSlot> {
    (0..MAX_FILTERS)
        .filter_map(FilterSlot::from_index)
        .find(|slot| !patch.contains_filter_slot(*slot))
}

pub(super) fn next_aux_slot(patch: &Patch) -> Option<AuxSlot> {
    (0..MAX_AUX_MODULES)
        .filter_map(AuxSlot::from_index)
        .find(|slot| !patch.contains_aux_slot(*slot))
}

pub(super) fn add_aux_to_group(
    state: &PluginContext<KurvParams>,
    group_id: GroupId,
    insertion: usize,
    slot: AuxSlot,
) {
    let inserted = state.generator_stack.edit(|patch| {
        let insert_at = patch
            .groups()
            .iter()
            .find(|group| group.id() == group_id)
            .map_or(0, |group| insertion.min(group.modules().len()));
        patch
            .insert_aux_with_slot(group_id, insert_at, slot)
            .is_ok()
    });
    if inserted {
        state
            .generator_stack
            .set_aux_config(slot, AuxConfig::default());
    }
}

pub(super) fn add_aux_to_new_group(
    state: &PluginContext<KurvParams>,
    slot: AuxSlot,
    insertion: usize,
) {
    let inserted = state.generator_stack.edit(|patch| {
        let insertion = insertion.min(patch.groups().len());
        let Ok(group_id) = patch.insert_group(insertion) else {
            return false;
        };
        if patch.insert_aux_with_slot(group_id, 0, slot).is_err() {
            let _ = patch.remove_group(group_id);
            return false;
        }
        true
    });
    if inserted {
        state
            .generator_stack
            .set_aux_config(slot, AuxConfig::default());
    }
}

pub(super) fn duplicate_module_to_group(
    state: &PluginContext<KurvParams>,
    module_id: crate::generators::ModuleId,
    group_id: GroupId,
    insertion: usize,
) -> bool {
    let patch = state.generator_stack.snapshot();
    let Some(kind) = patch
        .groups()
        .iter()
        .flat_map(|group| group.modules())
        .find(|module| module.id() == module_id)
        .map(|module| module.kind())
    else {
        return false;
    };
    match kind {
        ModuleKind::Oscillator(source) => {
            let Some(destination) = (0..crate::generators::MAX_OSCILLATORS)
                .filter_map(OscillatorSlot::from_index)
                .find(|slot| !patch.contains_oscillator_slot(*slot))
            else {
                return false;
            };
            let inserted = state.generator_stack.edit(|patch| {
                let insertion = patch
                    .groups()
                    .iter()
                    .find(|group| group.id() == group_id)
                    .map_or(0, |group| insertion.min(group.modules().len()));
                patch
                    .insert_oscillator_with_slot(group_id, insertion, destination)
                    .ok()
            });
            if let Some(inserted) = inserted {
                state.generator_stack.copy_oscillator(source, destination);
                if state
                    .generator_stack
                    .oscillator_config(source)
                    .engine
                    .uses_sample_asset()
                    && !state
                        .resynth_assets
                        .duplicate_slot(source.index(), destination.index())
                {
                    let _ = state
                        .generator_stack
                        .edit(|patch| patch.remove_module(inserted));
                    state.generator_stack.reset_oscillator(destination);
                    return false;
                }
            }
            inserted.is_some()
        }
        ModuleKind::Filter(source) => {
            let Some(destination) = next_filter_slot(&patch) else {
                return false;
            };
            let inserted = state.generator_stack.edit(|patch| {
                let insertion = patch
                    .groups()
                    .iter()
                    .find(|group| group.id() == group_id)
                    .map_or(0, |group| insertion.min(group.modules().len()));
                patch
                    .insert_filter_with_slot(group_id, insertion, destination)
                    .is_ok()
            });
            if inserted {
                state
                    .generator_stack
                    .set_filter_config(destination, state.generator_stack.filter_config(source));
            }
            inserted
        }
        ModuleKind::Aux(source) => {
            let Some(destination) = next_aux_slot(&patch) else {
                return false;
            };
            let inserted = state.generator_stack.edit(|patch| {
                let insertion = patch
                    .groups()
                    .iter()
                    .find(|group| group.id() == group_id)
                    .map_or(0, |group| insertion.min(group.modules().len()));
                patch
                    .insert_aux_with_slot(group_id, insertion, destination)
                    .is_ok()
            });
            if inserted {
                state
                    .generator_stack
                    .set_aux_config(destination, state.generator_stack.aux_config(source));
            }
            inserted
        }
    }
}

pub(super) fn duplicate_module_to_new_group(
    state: &PluginContext<KurvParams>,
    module_id: crate::generators::ModuleId,
    insertion: usize,
) -> bool {
    let group_id = state
        .generator_stack
        .edit(|patch| patch.insert_group(insertion.min(patch.groups().len())).ok());
    let Some(group_id) = group_id else {
        return false;
    };
    if duplicate_module_to_group(state, module_id, group_id, 0) {
        true
    } else {
        let _ = state
            .generator_stack
            .edit(|patch| patch.remove_group(group_id));
        false
    }
}

pub(super) fn duplicate_group(
    state: &PluginContext<KurvParams>,
    source_id: GroupId,
    insertion: usize,
) -> bool {
    let patch = state.generator_stack.snapshot();
    let Some(source) = patch.groups().iter().find(|group| group.id() == source_id) else {
        return false;
    };
    if patch.groups().len() >= crate::generators::MAX_OUTPUT_PAIRS
        || patch.oscillator_count()
            + source
                .modules()
                .iter()
                .filter(|module| matches!(module.kind(), ModuleKind::Oscillator(_)))
                .count()
            > crate::generators::MAX_OSCILLATORS
        || patch.filter_count()
            + source
                .modules()
                .iter()
                .filter(|module| matches!(module.kind(), ModuleKind::Filter(_)))
                .count()
            > MAX_FILTERS
        || patch.aux_count()
            + source
                .modules()
                .iter()
                .filter(|module| matches!(module.kind(), ModuleKind::Aux(_)))
                .count()
            > MAX_AUX_MODULES
    {
        return false;
    }
    let modules = source
        .modules()
        .iter()
        .map(|module| module.id())
        .collect::<Vec<_>>();
    let output = source.output();
    let Some(group_id) = state.generator_stack.edit(|patch| {
        let group_id = patch
            .insert_group(insertion.min(patch.groups().len()))
            .ok()?;
        let _ = patch.set_group_output(group_id, output);
        Some(group_id)
    }) else {
        return false;
    };
    for module_id in modules {
        if !duplicate_module_to_group(state, module_id, group_id, usize::MAX) {
            remove_generator_group(state, group_id);
            return false;
        }
    }
    if let Ok(mut editor) = state.params().editor_state.lock() {
        let accent = editor
            .group_accents
            .iter()
            .find(|stored| stored.group_id == source_id.get())
            .cloned();
        let name = editor.group_name(source_id.get()).map(str::to_owned);
        if let Some(mut accent) = accent {
            accent.group_id = group_id.get();
            editor.group_accents.push(accent);
        }
        if let Some(name) = name {
            editor.set_group_name(group_id.get(), &name);
        }
    }
    true
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
        let _ = patch.insert_group(insertion);
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
    let patch = state.generator_stack.snapshot();
    for slot in patch
        .groups()
        .iter()
        .flat_map(|group| group.modules())
        .filter_map(|module| module.aux_slot())
    {
        let config = state.generator_stack.aux_config(slot);
        if config.source == crate::generators::AuxSource::Group(group.id()) {
            state
                .generator_stack
                .set_aux_config(slot, AuxConfig::default());
        }
    }
    for module in group.modules() {
        clear_module_bindings(state, module);
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
            ModuleKind::Aux(slot) => state
                .generator_stack
                .set_aux_config(slot, AuxConfig::default()),
        }
    }
}
