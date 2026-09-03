//! Human-readable labels and stable color indexing for modulation targets.

use super::*;

pub(super) fn target_label(target: UiDestination) -> String {
    match target {
        UiDestination::Host(target) => modulation_target::descriptor(target)
            .map_or("DESTINATION", |target| target.label)
            .to_owned(),
        UiDestination::Modular(ModulationRouteTarget::Legacy { target }) => {
            modulation_target::descriptor(target)
                .map_or("DESTINATION", |target| target.label)
                .to_owned()
        }
        UiDestination::Modular(ModulationRouteTarget::Oscillator { slot, control, .. }) => {
            format!("OSC {} {}", slot.index() + 1, control.label())
        }
        UiDestination::Modular(ModulationRouteTarget::Group { control, .. }) => {
            format!("GROUP {}", control.label())
        }
        UiDestination::Modular(ModulationRouteTarget::Filter { slot, control, .. }) => {
            format!("FILTER {} {}", slot.index() + 1, control.label())
        }
        UiDestination::Modular(ModulationRouteTarget::Aux { slot, .. }) => {
            format!("AUX {} INPUT", slot.index() + 1)
        }
        UiDestination::Modular(ModulationRouteTarget::RouteDepth { route }) => {
            format!("ROUTE {} DEPTH", route + 1)
        }
        UiDestination::Modular(ModulationRouteTarget::MacroPack { source }) => {
            format!("MACROPACK {} VALUE", source + 1)
        }
    }
}

pub(super) fn target_parent_color(
    state: &PluginContext<KurvParams>,
    target: UiDestination,
    patch: &crate::generators::Patch,
    editor: Option<&crate::KurvEditorState>,
) -> egui::Color32 {
    let group_id = match target {
        UiDestination::Modular(ModulationRouteTarget::Group { group_id, .. }) => Some(group_id),
        UiDestination::Modular(
            ModulationRouteTarget::Oscillator { module_id, .. }
            | ModulationRouteTarget::Filter { module_id, .. }
            | ModulationRouteTarget::Aux { module_id, .. },
        ) => patch
            .groups()
            .iter()
            .find(|group| {
                group
                    .modules()
                    .iter()
                    .any(|module| module.id().get() == module_id)
            })
            .map(|group| group.id().get()),
        _ => None,
    };
    if let Some(group_id) = group_id {
        let palette = editor_theme::group_accents();
        let fallback = palette[group_id.saturating_sub(1) as usize % palette.len()];
        return editor.map_or(fallback, |editor| {
            editor.group_accent_color(group_id, fallback, &palette)
        });
    }
    match target {
        UiDestination::Host(target)
        | UiDestination::Modular(ModulationRouteTarget::Legacy { target }) => {
            source_color(usize::from(target.saturating_sub(1)))
        }
        UiDestination::Modular(ModulationRouteTarget::RouteDepth { route }) => {
            route_source(state, usize::from(route))
                .map_or_else(|| source_color(usize::from(route)), modulation_source_color)
        }
        UiDestination::Modular(ModulationRouteTarget::MacroPack { source }) => {
            modulation_source_color(ResolvedRouteSource::Rack(source))
        }
        UiDestination::Modular(
            ModulationRouteTarget::Oscillator { module_id, .. }
            | ModulationRouteTarget::Filter { module_id, .. }
            | ModulationRouteTarget::Aux { module_id, .. },
        ) => source_color(module_id as usize),
        UiDestination::Modular(ModulationRouteTarget::Group { group_id, .. }) => {
            source_color(group_id as usize)
        }
    }
}
