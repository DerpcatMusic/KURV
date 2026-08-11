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
            format!(
                "OSC {} {}",
                slot.index() + 1,
                oscillator_control_label(control)
            )
        }
        UiDestination::Modular(ModulationRouteTarget::Group { control, .. }) => {
            format!("GROUP {}", group_control_label(control))
        }
        UiDestination::Modular(ModulationRouteTarget::Filter { slot, control, .. }) => {
            format!(
                "FILTER {} {}",
                slot.index() + 1,
                filter_control_label(control)
            )
        }
    }
}

fn oscillator_control_label(control: OscillatorControl) -> &'static str {
    match control {
        OscillatorControl::Shape => "SHAPE",
        OscillatorControl::TablePosition => "VA POSITION",
        OscillatorControl::PulseWidth => "PULSE",
        OscillatorControl::Transpose => "SEMI",
        OscillatorControl::Cents => "CENT",
        OscillatorControl::Level => "LEVEL",
        OscillatorControl::Pan => "PAN",
        OscillatorControl::PhasePosition => "PHASE",
        OscillatorControl::PhaseRandom => "RANDOM PHASE",
        OscillatorControl::PhaseWarpAmount => "WARP",
        OscillatorControl::UnisonVoices => "VOICES",
        OscillatorControl::UnisonRange => "RANGE",
        OscillatorControl::UnisonAmount => "DETUNE",
        OscillatorControl::UnisonCurve => "DISTRIBUTION",
        OscillatorControl::UnisonJitter => "JITTER",
        OscillatorControl::UnisonRate => "JITTER RATE",
        OscillatorControl::UnisonWidth => "WIDTH",
        OscillatorControl::UnisonWeight => "WEIGHT",
        OscillatorControl::UnisonAlignment => "ALIGN",
        OscillatorControl::UnisonPanCurve => "PAN SHAPE",
        OscillatorControl::UnisonPanCenter => "PAN CENTER",
        OscillatorControl::UnisonStereoPosition => "PAN X",
        OscillatorControl::UnisonStereoAlternate => "PAN Y",
    }
}

fn group_control_label(control: GroupControl) -> &'static str {
    match control {
        GroupControl::Gain => "GAIN",
        GroupControl::Pan => "PAN",
        GroupControl::Attack => "ATTACK",
        GroupControl::AttackCurve => "ATTACK CURVE",
        GroupControl::Decay => "DECAY",
        GroupControl::DecayCurve => "DECAY CURVE",
        GroupControl::Sustain => "SUSTAIN",
        GroupControl::Release => "RELEASE",
        GroupControl::ReleaseCurve => "RELEASE CURVE",
    }
}

fn filter_control_label(control: FilterControl) -> &'static str {
    match control {
        FilterControl::Cutoff => "CUTOFF",
        FilterControl::Resonance => "RESONANCE",
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
