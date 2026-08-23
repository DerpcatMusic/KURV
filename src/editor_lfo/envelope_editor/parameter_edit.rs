use truce::params::Params;
use truce_core::editor::{PluginContext, PluginContextReadF32};

use crate::modulators::state::LEGACY_MODULATION_SOURCES;
use crate::{KurvParams, P};

use super::super::source::envelope_params;
use super::{EnvelopeDrag, EnvelopeEditorUi};

pub(super) fn set_envelope_time(
    state: &PluginContext<KurvParams>,
    index: usize,
    stage: EnvelopeDrag,
    seconds: f32,
) {
    let (maximum, param) = match stage {
        EnvelopeDrag::Attack => (8.0, envelope_params(index).attack),
        EnvelopeDrag::DecaySustain => (8.0, envelope_params(index).decay),
        EnvelopeDrag::Release => (12.0, envelope_params(index).release),
        _ => return,
    };
    let plain_fraction = (seconds / maximum).clamp(0.0, 1.0);
    if index < LEGACY_MODULATION_SOURCES {
        state.set_param(param, f64::from(plain_fraction.powf(0.25)));
    } else {
        set_envelope_normalized(state, index, stage, plain_fraction);
    }
}

pub(super) fn begin_envelope_edit(
    state: &PluginContext<KurvParams>,
    index: usize,
    stage: EnvelopeDrag,
) {
    if index >= LEGACY_MODULATION_SOURCES {
        return;
    }
    let params = envelope_params(index);
    match stage {
        EnvelopeDrag::Attack => crate::editor::begin_edit(state, params.attack),
        EnvelopeDrag::AttackCurve | EnvelopeDrag::DecayCurve | EnvelopeDrag::ReleaseCurve => {}
        EnvelopeDrag::DecaySustain => {
            crate::editor::begin_edit(state, params.decay);
            crate::editor::begin_edit(state, params.sustain);
        }
        EnvelopeDrag::Sustain => crate::editor::begin_edit(state, params.sustain),
        EnvelopeDrag::Release => crate::editor::begin_edit(state, params.release),
    }
}

fn end_envelope_edit(state: &PluginContext<KurvParams>, index: usize, stage: EnvelopeDrag) {
    if index >= LEGACY_MODULATION_SOURCES {
        return;
    }
    let params = envelope_params(index);
    match stage {
        EnvelopeDrag::Attack => crate::editor::end_edit(state, params.attack),
        EnvelopeDrag::AttackCurve | EnvelopeDrag::DecayCurve | EnvelopeDrag::ReleaseCurve => {}
        EnvelopeDrag::DecaySustain => {
            crate::editor::end_edit(state, params.decay);
            crate::editor::end_edit(state, params.sustain);
        }
        EnvelopeDrag::Sustain => crate::editor::end_edit(state, params.sustain),
        EnvelopeDrag::Release => crate::editor::end_edit(state, params.release),
    }
}

pub(super) fn finish_envelope_drag(
    state: &PluginContext<KurvParams>,
    index: usize,
    editor: &mut EnvelopeEditorUi,
) {
    if let Some(stage) = editor.drag.take() {
        end_envelope_edit(state, index, stage);
    }
    editor.drag_pointer_origin = None;
    editor.drag_handle_origin = None;
    editor.drag_precision = 0.0;
}

pub(super) fn reset_envelope(
    state: &PluginContext<KurvParams>,
    index: usize,
    stage: Option<EnvelopeDrag>,
) {
    if index < LEGACY_MODULATION_SOURCES {
        let params = envelope_params(index);
        let targets: &[P] = match stage {
            Some(EnvelopeDrag::Attack) => &[params.attack],
            Some(
                EnvelopeDrag::AttackCurve | EnvelopeDrag::DecayCurve | EnvelopeDrag::ReleaseCurve,
            ) => &[],
            Some(EnvelopeDrag::DecaySustain) => &[params.decay, params.sustain],
            Some(EnvelopeDrag::Sustain) => &[params.sustain],
            Some(EnvelopeDrag::Release) => &[params.release],
            None => &[params.attack, params.decay, params.sustain, params.release],
        };
        for &param in targets {
            let raw = u32::from(param);
            let Some(default) = state
                .params()
                .param_infos()
                .into_iter()
                .find(|info| info.id == raw)
                .map(|info| info.range.normalize(info.default_plain))
            else {
                continue;
            };
            crate::editor::begin_edit(state, param);
            state.set_param(param, default);
            crate::editor::end_edit(state, param);
        }
        let defaults = crate::modulators::state::SourceConfig::default();
        let mut config = state.params().modulator_rack.config(index);
        match stage {
            Some(EnvelopeDrag::AttackCurve) => config.attack_curve = defaults.attack_curve,
            Some(EnvelopeDrag::DecayCurve) => config.decay_curve = defaults.decay_curve,
            Some(EnvelopeDrag::ReleaseCurve) => config.release_curve = defaults.release_curve,
            None => {
                config.attack_curve = defaults.attack_curve;
                config.decay_curve = defaults.decay_curve;
                config.release_curve = defaults.release_curve;
            }
            _ => {}
        }
        state.params().modulator_rack.set_config(index, config);
        return;
    }

    let defaults = crate::modulators::state::SourceConfig::default();
    let mut config = state.params().modulator_rack.config(index);
    match stage {
        Some(EnvelopeDrag::Attack) => config.attack = defaults.attack,
        Some(EnvelopeDrag::AttackCurve) => config.attack_curve = defaults.attack_curve,
        Some(EnvelopeDrag::DecaySustain) => {
            config.decay = defaults.decay;
            config.sustain = defaults.sustain;
        }
        Some(EnvelopeDrag::DecayCurve) => config.decay_curve = defaults.decay_curve,
        Some(EnvelopeDrag::Sustain) => config.sustain = defaults.sustain,
        Some(EnvelopeDrag::Release) => config.release = defaults.release,
        Some(EnvelopeDrag::ReleaseCurve) => config.release_curve = defaults.release_curve,
        None => {
            config.attack = defaults.attack;
            config.attack_curve = defaults.attack_curve;
            config.decay = defaults.decay;
            config.decay_curve = defaults.decay_curve;
            config.sustain = defaults.sustain;
            config.release = defaults.release;
            config.release_curve = defaults.release_curve;
        }
    }
    state.params().modulator_rack.set_config(index, config);
}

pub(super) fn envelope_normalized(
    state: &PluginContext<KurvParams>,
    index: usize,
    stage: EnvelopeDrag,
) -> f32 {
    if index < LEGACY_MODULATION_SOURCES {
        let params = envelope_params(index);
        let param = match stage {
            EnvelopeDrag::Attack => params.attack,
            EnvelopeDrag::AttackCurve => {
                return state
                    .params()
                    .modulator_rack
                    .config(index)
                    .attack_curve
                    .mul_add(0.5, 0.5);
            }
            EnvelopeDrag::DecaySustain => params.decay,
            EnvelopeDrag::DecayCurve => {
                return state
                    .params()
                    .modulator_rack
                    .config(index)
                    .decay_curve
                    .mul_add(0.5, 0.5);
            }
            EnvelopeDrag::Sustain => params.sustain,
            EnvelopeDrag::Release => params.release,
            EnvelopeDrag::ReleaseCurve => {
                return state
                    .params()
                    .modulator_rack
                    .config(index)
                    .release_curve
                    .mul_add(0.5, 0.5);
            }
        };
        return state.get_param(param);
    }
    let config = state.params().modulator_rack.config(index);
    match stage {
        EnvelopeDrag::Attack => config.attack / 8.0,
        EnvelopeDrag::AttackCurve => config.attack_curve.mul_add(0.5, 0.5),
        EnvelopeDrag::DecaySustain => config.decay / 8.0,
        EnvelopeDrag::DecayCurve => config.decay_curve.mul_add(0.5, 0.5),
        EnvelopeDrag::Sustain => config.sustain,
        EnvelopeDrag::Release => config.release / 12.0,
        EnvelopeDrag::ReleaseCurve => config.release_curve.mul_add(0.5, 0.5),
    }
}

pub(super) fn envelope_sustain_normalized(state: &PluginContext<KurvParams>, index: usize) -> f32 {
    if index < LEGACY_MODULATION_SOURCES {
        state.get_param(envelope_params(index).sustain)
    } else {
        state.params().modulator_rack.config(index).sustain
    }
}

pub(super) fn set_envelope_normalized(
    state: &PluginContext<KurvParams>,
    index: usize,
    stage: EnvelopeDrag,
    normalized: f32,
) {
    let normalized = normalized.clamp(0.0, 1.0);
    if index < LEGACY_MODULATION_SOURCES {
        let params = envelope_params(index);
        let param = match stage {
            EnvelopeDrag::Attack => params.attack,
            EnvelopeDrag::DecaySustain => params.decay,
            EnvelopeDrag::Sustain => params.sustain,
            EnvelopeDrag::Release => params.release,
            EnvelopeDrag::AttackCurve | EnvelopeDrag::DecayCurve | EnvelopeDrag::ReleaseCurve => {
                let mut config = state.params().modulator_rack.config(index);
                let curve = normalized.mul_add(2.0, -1.0);
                match stage {
                    EnvelopeDrag::AttackCurve => config.attack_curve = curve,
                    EnvelopeDrag::DecayCurve => config.decay_curve = curve,
                    EnvelopeDrag::ReleaseCurve => config.release_curve = curve,
                    _ => unreachable!(),
                }
                state.params().modulator_rack.set_config(index, config);
                return;
            }
        };
        state.set_param(param, f64::from(normalized));
        return;
    }
    let mut config = state.params().modulator_rack.config(index);
    match stage {
        EnvelopeDrag::Attack => config.attack = normalized * 8.0,
        EnvelopeDrag::AttackCurve => config.attack_curve = normalized.mul_add(2.0, -1.0),
        EnvelopeDrag::DecaySustain => config.decay = normalized * 8.0,
        EnvelopeDrag::DecayCurve => config.decay_curve = normalized.mul_add(2.0, -1.0),
        EnvelopeDrag::Sustain => config.sustain = normalized,
        EnvelopeDrag::Release => config.release = normalized * 12.0,
        EnvelopeDrag::ReleaseCurve => config.release_curve = normalized.mul_add(2.0, -1.0),
    }
    state.params().modulator_rack.set_config(index, config);
}

pub(super) fn set_envelope_sustain_normalized(
    state: &PluginContext<KurvParams>,
    index: usize,
    normalized: f32,
) {
    let normalized = normalized.clamp(0.0, 1.0);
    if index < LEGACY_MODULATION_SOURCES {
        state.set_param(envelope_params(index).sustain, f64::from(normalized));
        return;
    }
    let mut config = state.params().modulator_rack.config(index);
    config.sustain = normalized;
    state.params().modulator_rack.set_config(index, config);
}
