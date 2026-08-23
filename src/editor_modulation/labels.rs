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
    }
}

pub(super) fn modular_target_color_index(target: ModulationRouteTarget) -> usize {
    match target {
        ModulationRouteTarget::Legacy { target } => usize::from(target),
        ModulationRouteTarget::Oscillator { slot, .. } => slot.index(),
        ModulationRouteTarget::Group { group_id, .. } => group_id as usize,
        ModulationRouteTarget::Filter { slot, .. } => MAX_OSCILLATORS + slot.index(),
    }
}
