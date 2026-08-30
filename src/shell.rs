use std::sync::Arc;

use super::*;

mod host_audio_block;

pub struct Kurv;

impl PluginLogic for Kurv {
    type Params = KurvParams;
    type DspState = KurvDspState;

    const PRESERVE_DSP_STATE: bool = false;

    fn init(_params: &KurvParams, _cx: &InitContext) -> KurvDspState {
        let mut diagnostics = diagnostics::DiagnosticSession::begin();
        let mut state = KurvDspState::default();
        if let Some(diagnostics) = diagnostics.as_mut() {
            diagnostics.initialized();
        }
        state.diagnostics = diagnostics;
        state
    }

    fn bus_layouts() -> Vec<BusLayout> {
        // Stereo first so hosts that keep only the preferred layout stay on
        // the eight group pairs. Mono is advertised so instrument tracks that
        // filter by channel still see KURV.
        let mut stereo = BusLayout::new();
        for name in [
            "Output 1/2",
            "Output 3/4",
            "Output 5/6",
            "Output 7/8",
            "Output 9/10",
            "Output 11/12",
            "Output 13/14",
            "Output 15/16",
        ] {
            stereo = stereo.with_output(name, ChannelConfig::Stereo);
        }
        vec![
            stereo,
            BusLayout::new().with_output("Output 1/2", ChannelConfig::Mono),
        ]
    }

    #[allow(
        clippy::cast_possible_truncation,
        reason = "the host sample rate is finite and exactly representable at audio-rate magnitudes"
    )]
    fn reset(state: &mut KurvDspState, params: &KurvParams, config: &AudioConfig) {
        params.reconcile_legacy_generator_stack();
        let resynth_quality = params
            .editor_state
            .lock()
            .map(|editor| crate::oscillators::ResynthQuality::from_u8(editor.resynth_quality))
            .unwrap_or_default();
        crate::oscillators::ResynthQuality::set_current(resynth_quality);
        state.host_sample_rate = config.sample_rate.max(1.0) as f32;
        let (factor, requested_antialiasing) = generator_configuration(params);
        state.dsp_sample_rate = state.host_sample_rate * f32::from(factor);
        state.refresh_filter_coefficients();
        state.generator_filter_modulation_mask = 0;
        state.generator_filter_modulation_tick = 0;
        state.generator_filter_modulation_stride = 1;
        state.synth.set_sample_rate(state.dsp_sample_rate);
        state.synth.reset();
        state.reset_lfo_curve_generations();
        state.lfos.reset(state.dsp_sample_rate);
        state.mod_wheel_ramp = RouteAmountRamp::default();
        state
            .mod_wheel_ramp
            .retarget(params.mod_wheel.value(), state.dsp_sample_rate);
        state.host_param_mod.fill(0.0);
        state.pitch_bend_mod = 0.0;
        state.mod_wheel_mod = 0.0;
        state.oversampler.reset(factor);
        for oversampler in &mut *state.group_oversamplers {
            oversampler.reset(factor);
        }
        let antialiasing = requested_antialiasing.for_factor(factor);
        state
            .oversampler
            .set_spline_correction_immediate(matches!(antialiasing, Antialiasing::SplineOptimized));
        for oversampler in &mut *state.group_oversamplers {
            oversampler.set_spline_correction_immediate(matches!(
                antialiasing,
                Antialiasing::SplineOptimized
            ));
        }
        state.decimator_tail = 0;
        state.mpe_bend_range = 48.0;
        state.pitch_bend_range = 2.0;
        state.glide_time_control = f32::NAN;
        state.pitch_bend_control = f32::NAN;
        state.meter_left = 0.0;
        state.meter_right = 0.0;
        #[cfg(test)]
        {
            state.block_major_chunks = 0;
            state.internal_pool_coarse_jobs = 0;
            state.internal_pool_partial_serial_jobs = 0;
        }
    }

    fn process(
        state: &mut KurvDspState,
        params: &KurvParams,
        buffer: &mut AudioBuffer,
        events: &EventList,
        context: &mut ProcessContext,
    ) -> ProcessStatus {
        if let Some(diagnostics) = state.diagnostics.as_mut() {
            diagnostics.record_process(buffer.num_samples(), buffer.num_output_channels());
        }
        let status = host_audio_block::process(state, params, buffer, events, context);
        if buffer.num_output_channels() != 0 {
            params.scope.publish(buffer.output(0));
        }
        status
    }

    fn latency(_state: &KurvDspState) -> u32 {
        oversampling::LATENCY_SAMPLES
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the bounded host sample rate and 20-second group release fit comfortably in u32"
    )]
    fn tail(state: &KurvDspState) -> u32 {
        (state.host_sample_rate * 20.0).round() as u32 + u32::from(oversampling::TAIL_SAMPLES)
    }

    fn migrate_state(foreign: &ForeignState) -> Option<MigratedState> {
        let ForeignState::Raw { bytes, .. } = foreign else {
            return None;
        };
        let root: serde_json::Value = match serde_json::from_slice(bytes) {
            Ok(root) => root,
            Err(_) => return None,
        };
        let Some(old) = root.get("params").and_then(serde_json::Value::as_object) else {
            return None;
        };
        let mappings = [
            ("gain", P::OutputDb.into()),
            ("wave", P::Shape.into()),
            ("pw", P::PulseWidth.into()),
            ("attack", P::Attack.into()),
            ("decay", P::Decay.into()),
            ("sustain", P::Sustain.into()),
            ("release", P::Release.into()),
            ("drone", P::Drone.into()),
            ("freq", P::DroneFrequency.into()),
        ];
        let params = mappings
            .into_iter()
            .filter_map(|(old_id, new_id)| old_plain_value(old.get(old_id)?).map(|v| (new_id, v)))
            .collect::<Vec<_>>();
        if params.is_empty() {
            return None;
        }
        Some(MigratedState {
            params,
            ..MigratedState::default()
        })
    }

    fn editor(params: Arc<KurvParams>) -> Box<dyn Editor> {
        editor::create(params)
    }
}

#[cfg(test)]
mod bus_layout_tests {
    use super::*;

    #[test]
    fn group_output_pairs_have_one_main_stereo_bus() {
        let layouts = <Kurv as PluginLogic>::bus_layouts();
        assert_eq!(layouts.len(), 2);
        let stereo = &layouts[0];
        assert!(stereo.inputs.is_empty());
        assert_eq!(stereo.outputs.len(), generators::MAX_OUTPUT_PAIRS);
        for (index, bus) in stereo.outputs.iter().enumerate() {
            assert_eq!(
                bus.kind,
                if index == 0 {
                    BusKind::Main
                } else {
                    BusKind::Sidechain
                }
            );
            assert_eq!(bus.channels.channel_count(), 2);
        }
        assert!(stereo.total_output_channels() <= 32);

        let mono = &layouts[1];
        assert!(mono.inputs.is_empty());
        assert_eq!(mono.outputs.len(), 1);
        assert_eq!(mono.outputs[0].kind, BusKind::Main);
        assert_eq!(mono.outputs[0].channels.channel_count(), 1);
    }

    #[test]
    fn dsp_state_fits_a_windows_host_stack_frame() {
        // Windows host threads default to a 1 MiB stack. VST3 create /
        // setActive construct plugin state on that stack before boxing.
        let params = std::mem::size_of::<KurvParams>();
        let dsp = std::mem::size_of::<KurvDspState>();
        let voice = std::mem::size_of::<crate::voices::VaVoice>();
        let stack = std::mem::size_of::<generators::GeneratorStackState>();
        eprintln!(
            "KurvParams={params} KurvDspState={dsp} VaVoice={voice} GeneratorStackState={stack}"
        );
        assert!(dsp < 256 * 1024, "KurvDspState is {dsp} bytes");
        assert!(voice < 128 * 1024, "VaVoice is {voice} bytes");
        assert!(params < 256 * 1024, "KurvParams is {params} bytes");
    }
}
