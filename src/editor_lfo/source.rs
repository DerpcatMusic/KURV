use super::*;

#[derive(Clone, Copy)]
pub(super) struct LfoParams {
    pub(super) rate: P,
    pub(super) rate_mode: P,
    pub(super) mode: P,
    pub(super) phase: P,
    pub(super) sync: P,
    pub(super) bipolar: P,
}

#[derive(Clone, Copy)]
pub(super) struct EnvelopeParams {
    pub(super) attack: P,
    pub(super) decay: P,
    pub(super) sustain: P,
    pub(super) release: P,
}

pub(super) fn lfo_phase_meter(state: &PluginContext<KurvParams>, index: usize) -> f32 {
    if index >= LEGACY_MODULATION_SOURCES {
        return state.params().modulator_rack.ui_snapshot(index).0;
    }
    let params = state.params();
    let meter = match index {
        0 => &params.lfo1_phase_meter,
        1 => &params.lfo2_phase_meter,
        2 => &params.lfo3_phase_meter,
        3 => &params.lfo4_phase_meter,
        4 => &params.lfo5_phase_meter,
        5 => &params.lfo6_phase_meter,
        6 => &params.lfo7_phase_meter,
        _ => &params.lfo8_phase_meter,
    };
    state.get_meter(meter)
}

pub(super) fn source_value_meter(state: &PluginContext<KurvParams>, index: usize) -> f32 {
    if index >= LEGACY_MODULATION_SOURCES {
        return state.params().modulator_rack.ui_snapshot(index).1;
    }
    let params = state.params();
    let meter = match index {
        0 => &params.lfo1_value_meter,
        1 => &params.lfo2_value_meter,
        2 => &params.lfo3_value_meter,
        3 => &params.lfo4_value_meter,
        4 => &params.lfo5_value_meter,
        5 => &params.lfo6_value_meter,
        6 => &params.lfo7_value_meter,
        _ => &params.lfo8_value_meter,
    };
    state.get_meter(meter)
}

pub(super) fn source_is_envelope(state: &PluginContext<KurvParams>, index: usize) -> bool {
    if index < LEGACY_MODULATION_SOURCES {
        state.get_param(source_envelope_param(index)) >= 0.5
    } else {
        state.params().modulator_rack.config(index).kind == SourceKind::Envelope
    }
}

pub(super) const fn source_envelope_param(index: usize) -> P {
    match index {
        0 => P::Source1Envelope,
        1 => P::Source2Envelope,
        2 => P::Source3Envelope,
        3 => P::Source4Envelope,
        4 => P::Source5Envelope,
        5 => P::Source6Envelope,
        6 => P::Source7Envelope,
        _ => P::Source8Envelope,
    }
}

pub(super) const fn envelope_params(index: usize) -> EnvelopeParams {
    match index {
        0 => EnvelopeParams {
            attack: P::Source1Attack,
            decay: P::Source1Decay,
            sustain: P::Source1Sustain,
            release: P::Source1Release,
        },
        1 => EnvelopeParams {
            attack: P::Source2Attack,
            decay: P::Source2Decay,
            sustain: P::Source2Sustain,
            release: P::Source2Release,
        },
        2 => EnvelopeParams {
            attack: P::Source3Attack,
            decay: P::Source3Decay,
            sustain: P::Source3Sustain,
            release: P::Source3Release,
        },
        3 => EnvelopeParams {
            attack: P::Source4Attack,
            decay: P::Source4Decay,
            sustain: P::Source4Sustain,
            release: P::Source4Release,
        },
        4 => EnvelopeParams {
            attack: P::Source5Attack,
            decay: P::Source5Decay,
            sustain: P::Source5Sustain,
            release: P::Source5Release,
        },
        5 => EnvelopeParams {
            attack: P::Source6Attack,
            decay: P::Source6Decay,
            sustain: P::Source6Sustain,
            release: P::Source6Release,
        },
        6 => EnvelopeParams {
            attack: P::Source7Attack,
            decay: P::Source7Decay,
            sustain: P::Source7Sustain,
            release: P::Source7Release,
        },
        _ => EnvelopeParams {
            attack: P::Source8Attack,
            decay: P::Source8Decay,
            sustain: P::Source8Sustain,
            release: P::Source8Release,
        },
    }
}

pub(super) fn envelope_values(params: &KurvParams, index: usize) -> [f32; 4] {
    if index >= LEGACY_MODULATION_SOURCES {
        let config = params.modulator_rack.config(index);
        return [config.attack, config.decay, config.sustain, config.release];
    }
    match index {
        0 => [
            params.source1_attack.value(),
            params.source1_decay.value(),
            params.source1_sustain.value(),
            params.source1_release.value(),
        ],
        1 => [
            params.source2_attack.value(),
            params.source2_decay.value(),
            params.source2_sustain.value(),
            params.source2_release.value(),
        ],
        2 => [
            params.source3_attack.value(),
            params.source3_decay.value(),
            params.source3_sustain.value(),
            params.source3_release.value(),
        ],
        3 => [
            params.source4_attack.value(),
            params.source4_decay.value(),
            params.source4_sustain.value(),
            params.source4_release.value(),
        ],
        4 => [
            params.source5_attack.value(),
            params.source5_decay.value(),
            params.source5_sustain.value(),
            params.source5_release.value(),
        ],
        5 => [
            params.source6_attack.value(),
            params.source6_decay.value(),
            params.source6_sustain.value(),
            params.source6_release.value(),
        ],
        6 => [
            params.source7_attack.value(),
            params.source7_decay.value(),
            params.source7_sustain.value(),
            params.source7_release.value(),
        ],
        _ => [
            params.source8_attack.value(),
            params.source8_decay.value(),
            params.source8_sustain.value(),
            params.source8_release.value(),
        ],
    }
}

pub(super) fn envelope_curve_values(params: &KurvParams, index: usize) -> [f32; 3] {
    let config = params.modulator_rack.config(index);
    [
        config.attack_curve,
        config.decay_curve,
        config.release_curve,
    ]
}

pub(super) fn active_source_mask(state: &PluginContext<KurvParams>) -> u64 {
    let stored = active_params()
        .into_iter()
        .enumerate()
        .fold(0, |mask, (index, param)| {
            if state.get_param(param) >= 0.5 {
                mask | (1_u64 << index)
            } else {
                mask
            }
        });
    stored | state.params().modulator_rack.active_mask() | used_source_mask(state)
}

pub(super) fn set_source_active(
    state: &PluginContext<KurvParams>,
    index: usize,
    active: bool,
    kind: SourceKind,
) {
    if let Ok(mut editor) = state.params().editor_state.lock() {
        editor.collapsed_modulators &= !(1_u64 << index);
    }
    if index < LEGACY_MODULATION_SOURCES {
        state.automate(active_params()[index], if active { 1.0 } else { 0.0 });
        state.automate(
            source_envelope_param(index),
            if kind == SourceKind::Envelope {
                1.0
            } else {
                0.0
            },
        );
    } else {
        let mut config = state.params().modulator_rack.config(index);
        config.active = active;
        if active {
            config.kind = kind;
        }
        state.params().modulator_rack.set_config(index, config);
    }
}

pub(super) const fn active_params() -> [P; 8] {
    [
        P::Lfo1Active,
        P::Lfo2Active,
        P::Lfo3Active,
        P::Lfo4Active,
        P::Lfo5Active,
        P::Lfo6Active,
        P::Lfo7Active,
        P::Lfo8Active,
    ]
}

pub(super) fn rate_mode(state: &PluginContext<KurvParams>, param: P) -> u8 {
    (state.get_param(param).clamp(0.0, 1.0) * 3.0).round() as u8
}

pub(super) fn rate_text(
    state: &PluginContext<KurvParams>,
    index: usize,
    rate_mode_param: P,
) -> String {
    let rate = lfo_rate(state.params(), index).max(0.000_01);
    match rate_mode(state, rate_mode_param) {
        1 => {
            let milliseconds = rate;
            if milliseconds < 10.0 {
                format!("{milliseconds:.2} ms")
            } else {
                format!("{milliseconds:.0} ms")
            }
        }
        3 => format!("{:.2}×", crate::modulators::lfo::keytrack_multiplier(rate)),
        _ if rate < 10.0 => format!("{rate:.2} Hz"),
        _ => format!("{rate:.0} Hz"),
    }
}

pub(super) fn lfo_rate(params: &KurvParams, index: usize) -> f32 {
    match index {
        0 => params.lfo1_rate.value(),
        1 => params.lfo2_rate.value(),
        2 => params.lfo3_rate.value(),
        3 => params.lfo4_rate.value(),
        4 => params.lfo5_rate.value(),
        5 => params.lfo6_rate.value(),
        6 => params.lfo7_rate.value(),
        _ => params.lfo8_rate.value(),
    }
}

pub(super) fn lfo_params(index: usize) -> LfoParams {
    match index {
        0 => LfoParams {
            rate: P::Lfo1Rate,
            rate_mode: P::Lfo1RateMode,
            mode: P::Lfo1Mode,
            phase: P::Lfo1Phase,
            sync: P::Lfo1Sync,
            bipolar: P::Lfo1Bipolar,
        },
        1 => LfoParams {
            rate: P::Lfo2Rate,
            rate_mode: P::Lfo2RateMode,
            mode: P::Lfo2Mode,
            phase: P::Lfo2Phase,
            sync: P::Lfo2Sync,
            bipolar: P::Lfo2Bipolar,
        },
        2 => LfoParams {
            rate: P::Lfo3Rate,
            rate_mode: P::Lfo3RateMode,
            mode: P::Lfo3Mode,
            phase: P::Lfo3Phase,
            sync: P::Lfo3Sync,
            bipolar: P::Lfo3Bipolar,
        },
        3 => LfoParams {
            rate: P::Lfo4Rate,
            rate_mode: P::Lfo4RateMode,
            mode: P::Lfo4Mode,
            phase: P::Lfo4Phase,
            sync: P::Lfo4Sync,
            bipolar: P::Lfo4Bipolar,
        },
        4 => LfoParams {
            rate: P::Lfo5Rate,
            rate_mode: P::Lfo5RateMode,
            mode: P::Lfo5Mode,
            phase: P::Lfo5Phase,
            sync: P::Lfo5Sync,
            bipolar: P::Lfo5Bipolar,
        },
        5 => LfoParams {
            rate: P::Lfo6Rate,
            rate_mode: P::Lfo6RateMode,
            mode: P::Lfo6Mode,
            phase: P::Lfo6Phase,
            sync: P::Lfo6Sync,
            bipolar: P::Lfo6Bipolar,
        },
        6 => LfoParams {
            rate: P::Lfo7Rate,
            rate_mode: P::Lfo7RateMode,
            mode: P::Lfo7Mode,
            phase: P::Lfo7Phase,
            sync: P::Lfo7Sync,
            bipolar: P::Lfo7Bipolar,
        },
        _ => LfoParams {
            rate: P::Lfo8Rate,
            rate_mode: P::Lfo8RateMode,
            mode: P::Lfo8Mode,
            phase: P::Lfo8Phase,
            sync: P::Lfo8Sync,
            bipolar: P::Lfo8Bipolar,
        },
    }
}

pub(super) fn lfo_curve(params: &KurvParams, index: usize) -> &WaveCurveState {
    match index {
        0 => &params.lfo1_curve_state,
        1 => &params.lfo2_curve_state,
        2 => &params.lfo3_curve_state,
        3 => &params.lfo4_curve_state,
        4 => &params.lfo5_curve_state,
        5 => &params.lfo6_curve_state,
        6 => &params.lfo7_curve_state,
        _ => &params.lfo8_curve_state,
    }
}
