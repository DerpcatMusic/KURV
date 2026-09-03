use super::*;
use crate::modulators::routing::ResolvedRouteSource;
use crate::params::host_banks::{HOST_LFO_PARAMETER_IDS, HostLfoParameterIds, host_lfo_schema};

macro_rules! host_lfo_refs {
    ($params:expr; $(($index:literal, $active_field:ident, $active:ident, $envelope_field:ident, $envelope:ident, $rate_field:ident, $rate:ident, $rate_mode_field:ident, $rate_mode:ident, $mode_field:ident, $mode:ident, $phase_field:ident, $phase:ident, $sync_field:ident, $sync:ident, $bipolar_field:ident, $bipolar:ident, $shape_field:ident, $shape:ident, $attack_field:ident, $attack:ident, $decay_field:ident, $decay:ident, $sustain_field:ident, $sustain:ident, $release_field:ident, $release:ident, $phase_meter:ident, $value_meter:ident, $curve:ident)),+ $(,)?) => {
        (
            [$(&$params.$phase_meter),+],
            [$(&$params.$value_meter),+],
            [$(&$params.$curve),+],
        )
    };
}

pub(super) fn lfo_phase_meter(state: &PluginContext<KurvParams>, index: usize) -> f32 {
    if index >= LEGACY_MODULATION_SOURCES {
        return state.params().modulator_rack.ui_snapshot(index).0;
    }
    let params = state.params();
    let (phase_meters, _, _) = host_lfo_schema!(host_lfo_refs, params);
    state.get_meter(phase_meters[index])
}

pub(crate) fn source_is_running(state: &PluginContext<KurvParams>, index: usize) -> bool {
    state.params().modulator_rack.ui_running(index)
}

pub(crate) fn source_value_meter(state: &PluginContext<KurvParams>, index: usize) -> f32 {
    if index >= LEGACY_MODULATION_SOURCES {
        return state.params().modulator_rack.ui_snapshot(index).1;
    }
    let params = state.params();
    let (_, value_meters, _) = host_lfo_schema!(host_lfo_refs, params);
    state.get_meter(value_meters[index])
}

pub(super) fn source_is_envelope(state: &PluginContext<KurvParams>, index: usize) -> bool {
    if index < LEGACY_MODULATION_SOURCES {
        state.get_param(source_envelope_param(index)) >= 0.5
    } else {
        state.params().modulator_rack.config(index).kind == SourceKind::Envelope
    }
}

pub(super) fn source_kind(state: &PluginContext<KurvParams>, index: usize) -> SourceKind {
    if source_is_envelope(state, index) {
        SourceKind::Envelope
    } else if index >= LEGACY_MODULATION_SOURCES {
        state.params().modulator_rack.config(index).kind
    } else {
        SourceKind::Lfo
    }
}

pub(super) const fn source_kind_is_static(kind: SourceKind) -> bool {
    matches!(kind, SourceKind::Macro | SourceKind::Button)
}

pub(super) fn macro_pack_sources(
    state: &PluginContext<KurvParams>,
    active: u64,
) -> ([usize; MACRO_PACK_CAPACITY], usize) {
    let mut sources = [0; MACRO_PACK_CAPACITY];
    let mut len = 0;
    for slot in state.params().modulator_rack.presentation_order() {
        let index = usize::from(slot);
        if active & (1_u64 << index) != 0 && source_kind_is_static(source_kind(state, index)) {
            if len == MACRO_PACK_CAPACITY {
                break;
            }
            sources[len] = index;
            len += 1;
        }
    }
    (sources, len)
}

pub(super) fn macro_pack_mask(state: &PluginContext<KurvParams>, active: u64) -> u64 {
    let (sources, len) = macro_pack_sources(state, active);
    sources[..len]
        .iter()
        .fold(0, |mask, index| mask | (1_u64 << index))
}

pub(super) fn source_is_gate(state: &PluginContext<KurvParams>, index: usize) -> bool {
    if index < LEGACY_MODULATION_SOURCES {
        (state.get_param(lfo_params(index).shape).clamp(0.0, 1.0) * 3.0).round() as u8
            == crate::modulators::lfo::LfoShape::Gate as u8
    } else {
        state.params().modulator_rack.config(index).shape
            == crate::modulators::lfo::LfoShape::Gate as u8
    }
}

pub(super) const fn source_envelope_param(index: usize) -> P {
    HOST_LFO_PARAMETER_IDS[host_lfo_index(index)].envelope
}

pub(super) const fn envelope_params(index: usize) -> HostLfoParameterIds {
    HOST_LFO_PARAMETER_IDS[host_lfo_index(index)]
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
    let stored = HOST_LFO_PARAMETER_IDS
        .iter()
        .map(|params| params.active)
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

pub(crate) fn source_config(state: &PluginContext<KurvParams>, index: usize) -> SourceConfig {
    let mut config = state.params().modulator_rack.config(index);
    if index >= LEGACY_MODULATION_SOURCES {
        config.active = true;
        return config;
    }
    let params = lfo_params(index);
    let [attack, decay, sustain, release] = envelope_values(state.params(), index);
    config.active = true;
    config.kind = source_kind(state, index);
    config.rate_hz = lfo_rate(state.params(), index);
    config.rate_mode = rate_mode(state, params.rate_mode);
    config.mode = (state.get_param(params.mode).clamp(0.0, 1.0) * 3.0).round() as u8;
    config.phase_offset = state.get_param(params.phase).clamp(0.0, 1.0);
    config.sync_division = (state.get_param(params.sync).clamp(0.0, 1.0) * 15.0).round() as u8;
    config.bipolar = state.get_param(params.bipolar) >= 0.5;
    config.shape = (state.get_param(params.shape).clamp(0.0, 1.0) * 3.0).round() as u8;
    config.attack = attack;
    config.decay = decay;
    config.sustain = sustain;
    config.release = release;
    config
}

pub(crate) fn source_bipolar(
    state: &PluginContext<KurvParams>,
    source: ResolvedRouteSource,
) -> Option<bool> {
    let ResolvedRouteSource::Rack(source) = source else {
        return None;
    };
    let index = usize::from(source);
    let config = source_config(state, index);
    matches!(config.kind, SourceKind::Lfo | SourceKind::Macro).then_some(config.bipolar)
}

pub(crate) fn set_source_bipolar(
    state: &PluginContext<KurvParams>,
    source: ResolvedRouteSource,
    bipolar: bool,
) -> bool {
    let ResolvedRouteSource::Rack(source) = source else {
        return false;
    };
    let index = usize::from(source);
    let config = source_config(state, index);
    if !matches!(config.kind, SourceKind::Lfo | SourceKind::Macro) {
        return false;
    }
    if index < LEGACY_MODULATION_SOURCES {
        crate::editor::automate(
            state,
            lfo_params(index).bipolar,
            if bipolar { 1.0 } else { 0.0 },
        );
    } else {
        let mut config = state.params().modulator_rack.config(index);
        config.bipolar = bipolar;
        state.params().modulator_rack.set_config(index, config);
    }
    true
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
        crate::editor::automate(
            state,
            HOST_LFO_PARAMETER_IDS[index].active,
            if active { 1.0 } else { 0.0 },
        );
        crate::editor::automate(
            state,
            source_envelope_param(index),
            if kind == SourceKind::Envelope {
                1.0
            } else {
                0.0
            },
        );
        if active && kind == SourceKind::Lfo {
            let params = lfo_params(index);
            crate::editor::automate(state, params.rate_mode, 2.0 / 3.0);
            crate::editor::automate(state, params.mode, 2.0 / 3.0);
            crate::editor::automate(state, params.sync, 8.0 / 15.0);
            crate::editor::automate(state, params.bipolar, 0.0);
            lfo_curve(state.params(), index).replace(crate::wave_curve::default_lfo_curve());
        }
    } else {
        let mut config = state.params().modulator_rack.config(index);
        config.active = active;
        if active {
            config.kind = kind;
            if kind == SourceKind::Lfo {
                config.rate_mode = 2;
                config.mode = 2;
                config.sync_division = 8;
                config.bipolar = false;
                if let Some(curve) = state.params().modulator_rack.curve(index) {
                    curve.replace(crate::wave_curve::default_lfo_curve());
                }
            } else if matches!(kind, SourceKind::Macro | SourceKind::Button) {
                // Static sources are global. Sync mode keeps them out of every
                // voice's per-note LFO program while preserving rack identity.
                config.mode = 2;
                config.value = 0.0;
                config.bipolar = false;
            }
        }
        state.params().modulator_rack.set_config(index, config);
    }
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
    macro_rules! host_lfo_rates {
        ($params:expr; $(($index:literal, $active_field:ident, $active:ident, $envelope_field:ident, $envelope:ident, $rate_field:ident, $($rest:tt)*)),+ $(,)?) => {
            [$($params.$rate_field.value()),+]
        };
    }
    let rates = host_lfo_schema!(host_lfo_rates, params);
    rates[index.min(LEGACY_MODULATION_SOURCES - 1)]
}

pub(super) const fn lfo_params(index: usize) -> HostLfoParameterIds {
    HOST_LFO_PARAMETER_IDS[host_lfo_index(index)]
}

pub(super) fn lfo_curve(params: &KurvParams, index: usize) -> &WaveCurveState {
    let (_, _, curves) = host_lfo_schema!(host_lfo_refs, params);
    curves[index.min(LEGACY_MODULATION_SOURCES - 1)]
}

const fn host_lfo_index(index: usize) -> usize {
    if index < LEGACY_MODULATION_SOURCES {
        index
    } else {
        LEGACY_MODULATION_SOURCES - 1
    }
}
