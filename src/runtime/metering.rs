use crate::params::host_banks::host_lfo_schema;
use crate::*;

macro_rules! host_lfo_meters {
    ($params:expr; $(($index:literal, $active_field:ident, $active:ident, $envelope_field:ident, $envelope:ident, $rate_field:ident, $rate:ident, $rate_mode_field:ident, $rate_mode:ident, $mode_field:ident, $mode:ident, $phase_field:ident, $phase:ident, $sync_field:ident, $sync:ident, $bipolar_field:ident, $bipolar:ident, $shape_field:ident, $shape:ident, $attack_field:ident, $attack:ident, $decay_field:ident, $decay:ident, $sustain_field:ident, $sustain:ident, $release_field:ident, $release:ident, $phase_meter:ident, $value_meter:ident, $curve:ident)),+ $(,)?) => {
        ([$(&$params.$phase_meter),+], [$(&$params.$value_meter),+])
    };
}

#[allow(
    clippy::cast_precision_loss,
    reason = "audio buffers are far smaller than f32's exact integer range"
)]
pub(crate) fn publish_meters(
    state: &mut KurvDspState,
    params: &KurvParams,
    context: &ProcessContext,
    samples: usize,
    peak_left: f32,
    peak_right: f32,
) {
    let release = (1.0 - samples as f32 / (state.host_sample_rate * 0.18)).clamp(0.0, 1.0);
    state.meter_left = peak_left.max(state.meter_left * release);
    state.meter_right = peak_right.max(state.meter_right * release);
    context.set_meter(&params.meter_left, state.meter_left.min(1.0));
    context.set_meter(&params.meter_right, state.meter_right.min(1.0));
    context.set_meter(&params.stereo_seed, state.synth.latest_stereo_seed(0));
    context.set_meter(&params.swarm_phase, state.synth.swarm_time());
    context.set_meter(&params.osc2_stereo_seed, state.synth.latest_stereo_seed(1));
    context.set_meter(
        &params.osc2_swarm_phase,
        state.synth.secondary_swarm_time(1),
    );
    context.set_meter(&params.osc3_stereo_seed, state.synth.latest_stereo_seed(2));
    context.set_meter(
        &params.osc3_swarm_phase,
        state.synth.secondary_swarm_time(2),
    );
    let lfo_running_mask = if state.synth.is_active() {
        state.lfos.active_mask()
    } else {
        0
    };
    let (global_lfo_phases, global_lfo_values) = state.lfos.ui_snapshot();
    let mut lfo_phases = *global_lfo_phases;
    let mut lfo_values = *global_lfo_values;
    if let Some((voice_phases, voice_values, voice_mask)) = state.synth.voice_lfo_snapshot() {
        let mut active = voice_mask;
        while active != 0 {
            let index = active.trailing_zeros() as usize;
            active &= active - 1;
            lfo_phases[index] = voice_phases[index];
            lfo_values[index] = voice_values[index];
        }
    }
    let (phase_meters, value_meters) = host_lfo_schema!(host_lfo_meters, params);
    params
        .modulator_rack
        .publish_ui_snapshot(&lfo_phases, &lfo_values, lfo_running_mask);
    for index in 0..HOST_LFO_COUNT {
        context.set_meter(phase_meters[index], lfo_phases[index]);
        context.set_meter(value_meters[index], lfo_values[index]);
    }
}

pub(crate) const fn current_process_status(state: &KurvDspState) -> ProcessStatus {
    if state.synth.is_active() || state.decimator_tail != 0 {
        ProcessStatus::Normal
    } else {
        ProcessStatus::Tail(0)
    }
}
