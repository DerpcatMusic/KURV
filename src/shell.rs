use std::sync::Arc;

use super::*;

pub struct Kurv;

impl PluginLogic for Kurv {
    type Params = KurvParams;
    type DspState = KurvDspState;

    const PRESERVE_DSP_STATE: bool = false;

    fn bus_layouts() -> Vec<BusLayout> {
        let mut layout = BusLayout::new();
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
            layout = layout.with_output(name, ChannelConfig::Stereo);
        }
        vec![layout]
    }

    #[allow(
        clippy::cast_possible_truncation,
        reason = "the host sample rate is finite and exactly representable at audio-rate magnitudes"
    )]
    fn reset(state: &mut KurvDspState, params: &KurvParams, config: &AudioConfig) {
        state.host_sample_rate = config.sample_rate.max(1.0) as f32;
        let (factor, requested_antialiasing) = generator_configuration(params);
        diagnostics::trace(
            "lifecycle",
            "reset-enter",
            state.host_sample_rate,
            f32::from(factor),
        );
        state.dsp_sample_rate = state.host_sample_rate * f32::from(factor);
        state.synth.set_sample_rate(state.dsp_sample_rate);
        state.synth.reset();
        state.lfos.reset(state.dsp_sample_rate);
        state.oversampler.reset(factor);
        let antialiasing = requested_antialiasing.for_factor(factor);
        state
            .oversampler
            .set_spline_correction_immediate(matches!(antialiasing, Antialiasing::SplineOptimized));
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
        diagnostics::trace(
            "lifecycle",
            "reset-return",
            state.dsp_sample_rate,
            f32::from(state.oversampler.factor()),
        );
    }

    #[allow(
        clippy::too_many_lines,
        reason = "the audio callback keeps event dispatch, synthesis, oversampling, and meters in one RT boundary"
    )]
    fn process(
        state: &mut KurvDspState,
        params: &KurvParams,
        buffer: &mut AudioBuffer,
        events: &EventList,
        context: &mut ProcessContext,
    ) -> ProcessStatus {
        let output_channels = buffer.num_output_channels();
        if output_channels == 0 {
            return ProcessStatus::Tail(0);
        }

        let (requested_factor, requested_antialiasing) = generator_configuration(params);
        state.set_oversampling(requested_factor, requested_antialiasing);
        if let Some((oscillators, output, active_mask)) = params.generator_stack.try_rt_state() {
            state.generator_oscillators = oscillators;
            state.generator_output = output;
            state.generator_active_mask = active_mask;
        }
        let modulation_routes = modulation_routes(params);
        let oscillator_enabled = [
            params.osc1_enabled.value() && state.generator_active_mask & 1 != 0,
            params.osc2_enabled.value() && state.generator_active_mask & 2 != 0,
            params.osc3_enabled.value() && state.generator_active_mask & 4 != 0,
        ];
        let active_routes = active_modulation_routes(&modulation_routes, oscillator_enabled);
        let configured_lfos = configured_lfo_mask(params);
        let lfo_curve_states = [
            &params.lfo1_curve_state,
            &params.lfo2_curve_state,
            &params.lfo3_curve_state,
            &params.lfo4_curve_state,
            &params.lfo5_curve_state,
            &params.lfo6_curve_state,
            &params.lfo7_curve_state,
            &params.lfo8_curve_state,
        ];
        let lfo_sources = active_routes.source_mask | configured_lfos;
        let lfo_configs = if lfo_sources != 0 {
            let lfo_curves = std::array::from_fn(|index| {
                if lfo_sources & (1 << index) != 0 {
                    lfo_curve_states[index].try_curve_rt()
                } else {
                    None
                }
            });
            let configs = lfo_configuration(params);
            state.lfos.configure(
                configs,
                lfo_curves,
                0,
                context.transport,
                state.host_sample_rate,
            );
            configs
        } else {
            state.lfos.set_active_mask(0);
            state.lfos.set_modulation_mask(0);
            [LfoConfig::default(); LFO_COUNT]
        };
        let mut antialiasing = requested_antialiasing.for_factor(state.oversampler.factor());
        state
            .oversampler
            .set_spline_correction(matches!(antialiasing, Antialiasing::SplineOptimized));
        for (oscillator, curve) in [
            &params.pan_shape_curve_state,
            &params.osc2_pan_shape_curve_state,
            &params.osc3_pan_shape_curve_state,
        ]
        .into_iter()
        .enumerate()
        {
            if (oscillator == 0 || oscillator_enabled[oscillator])
                && let Some(segments) = curve.try_segments_rt()
            {
                state.pan_shape_segments[oscillator] = segments;
            }
        }
        for (oscillator, curve) in [
            &params.osc1_wave_curve_state,
            &params.osc2_wave_curve_state,
            &params.osc3_wave_curve_state,
        ]
        .into_iter()
        .enumerate()
        {
            if (oscillator == 0 || oscillator_enabled[oscillator])
                && let Some(compiled) = curve.try_curve_rt()
            {
                let audible = oscillator_enabled[oscillator]
                    && match oscillator {
                        0 => params.osc1_custom_shape.value() > f32::EPSILON,
                        1 => params.osc2_custom_shape.value() > f32::EPSILON,
                        _ => params.osc3_custom_shape.value() > f32::EPSILON,
                    };
                state.wave_curves[oscillator].retarget(compiled, audible);
            }
        }
        for oscillator in 0..generators::MAX_OSCILLATORS {
            let active = state.generator_active_mask & (1_u32 << oscillator) != 0;
            if active
                && let Some(slot) = generators::OscillatorSlot::from_index(oscillator)
                && let Some((generation, compiled)) = params
                    .generator_stack
                    .va_table(slot)
                    .try_table_rt(state.va_table_generations[oscillator])
            {
                state.va_table_generations[oscillator] = generation;
                if oscillator < LEGACY_OSCILLATOR_COUNT {
                    let audible = oscillator_enabled[oscillator]
                        && match oscillator {
                            0 => params.osc1_custom_shape.value() > f32::EPSILON,
                            1 => params.osc2_custom_shape.value() > f32::EPSILON,
                            _ => params.osc3_custom_shape.value() > f32::EPSILON,
                        };
                    state.va_table_transitions[oscillator].retarget(&compiled, audible);
                }
                state.va_tables[oscillator] = compiled;
            }
        }

        let mut unison_settings = unison_configurations(params, state);
        unison_settings[0] =
            unison_settings[0].with_phase_position(params.osc1_phase_position.value());
        unison_settings[1] =
            unison_settings[1].with_phase_position(params.osc2_phase_position.value());
        unison_settings[2] =
            unison_settings[2].with_phase_position(params.osc3_phase_position.value());
        for oscillator in 0..LEGACY_OSCILLATOR_COUNT {
            if !oscillator_enabled[oscillator] {
                continue;
            }
            if oscillator == 0 {
                state.synth.configure_unison(unison_settings[oscillator]);
            } else {
                state
                    .synth
                    .configure_secondary_unison(oscillator, unison_settings[oscillator]);
            }
        }
        state
            .synth
            .configure_voice_mode(params.voice_mode.value_u8());
        state.synth.set_transpose(
            params
                .octave_shift
                .value_f32()
                .mul_add(12.0, params.transpose.value_f32()),
        );
        state.mpe_bend_range = f32::from(params.mpe_bend_range.value_u8());
        state.pitch_bend_range = f32::from(params.pitch_bend_range.value_u8());

        state.synth.configure_oscillator_enabled(oscillator_enabled);
        let extended_oscillators = std::array::from_fn(|index| {
            let config = state.generator_oscillators[index];
            let table = state.va_tables[index].select(WaveCurveRt::default(), config.custom_shape);
            ExtendedOscillatorConfig {
                enabled: config.enabled,
                shape: config.shape,
                pulse_width: config.pulse_width,
                custom_curve: table.curve,
                custom_mix: table.mix,
                transpose: config.transpose,
                cents: config.cents,
                level: config.level,
                pan: config.pan,
                unison_voices: config.unison_voices,
                unison_range: config.unison_range,
                unison_amount: config.unison_amount,
                unison_curve: config.unison_curve,
                unison_jitter: config.unison_jitter,
                unison_rate: config.unison_rate,
                unison_width: config.unison_width,
                phase_position: config.phase_position,
                phase_random: config.phase_random,
                unison_alignment: config.unison_alignment,
                unison_alignment_mode: config.unison_alignment_mode,
                unison_pan_curve: config.unison_pan_curve,
            }
        });
        state
            .synth
            .configure_extended_oscillators(extended_oscillators);
        let oscillator_transpose = [
            params.osc1_transpose.value_f32(),
            params.osc2_transpose.value_f32(),
            params.osc3_transpose.value_f32(),
        ];
        let oscillator_warp_mode = [
            PhaseWarpMode::from_index(params.osc1_warp_mode.value_u8()),
            PhaseWarpMode::from_index(params.osc2_warp_mode.value_u8()),
            PhaseWarpMode::from_index(params.osc3_warp_mode.value_u8()),
        ];
        state.synth.configure_phase_warp_modes(oscillator_warp_mode);

        let mut next_event = 0;
        let mut block_start = 0;
        let mut peak_left = 0.0_f32;
        let mut peak_right = 0.0_f32;
        while block_start < buffer.num_samples() {
            let block_len = (buffer.num_samples() - block_start).min(CONTROL_BLOCK);
            let static_gain = state.controls.read(
                params,
                block_len,
                oscillator_enabled,
                &active_routes,
                active_routes.source_mask,
            );
            let modulation_mask = state.controls.active_lfo_mask(&active_routes, block_len);
            let lfo_control_dynamic_mask =
                state
                    .controls
                    .lfo_control_dynamic_mask(modulation_mask, block_len, &lfo_configs);
            let pitch_bend_static = slice_is_static(&state.controls.pitch_bend[..block_len]);
            state
                .lfos
                .set_active_mask(modulation_mask | configured_lfos);
            state.lfos.set_modulation_mask(modulation_mask);
            state.fill_wave_curve_fades(block_len);
            let direct_unison_pitch_mask = state
                .controls
                .unison_pitch_active_mask(block_len, &unison_settings);
            let direct_unison_motion_mask = state
                .controls
                .unison_motion_active_mask(block_len, &unison_settings);
            let block_morph_lfo = state.lfos.is_active()
                && block_morph_modulation(&active_routes)
                && direct_unison_pitch_mask == 0
                && direct_unison_motion_mask == 0
                && !unison_settings
                    .iter()
                    .any(|settings| settings.motion_active());
            let block_pitch_lfo = state.lfos.is_active()
                && block_pitch_modulation(&active_routes)
                && direct_unison_pitch_mask == 0
                && direct_unison_motion_mask == 0
                && !unison_settings
                    .iter()
                    .any(|settings| settings.motion_active());
            let block_control_lfo = state.lfos.is_active()
                && block_parameter_modulation(&active_routes)
                && direct_unison_pitch_mask == 0
                && direct_unison_motion_mask == 0
                && !unison_settings
                    .iter()
                    .any(|settings| settings.motion_active());
            let block_motion_lfo = state.lfos.is_active()
                && block_motion_modulation(&active_routes)
                && direct_unison_pitch_mask == 0
                && direct_unison_motion_mask == 0;

            let mut offset = 0;
            let mut modulation = lfo::ModulationFrame::default();
            while offset < block_len {
                let sample_index = block_start + offset;
                let glide_time = state.controls.glide_time[offset];
                if glide_time.to_bits() != state.glide_time_control.to_bits() {
                    state.synth.set_glide_time(glide_time);
                    state.glide_time_control = glide_time;
                }
                if !pitch_bend_static || offset == 0 {
                    let pitch_bend = state.controls.pitch_bend[offset];
                    if pitch_bend.to_bits() != state.pitch_bend_control.to_bits() {
                        state
                            .synth
                            .parameter_pitch_bend(pitch_bend, state.pitch_bend_range);
                        state.pitch_bend_control = pitch_bend;
                    }
                }
                if direct_unison_motion_mask != 0 {
                    configure_direct_unison_motion(
                        state,
                        &unison_settings,
                        direct_unison_motion_mask,
                        offset,
                    );
                }
                dispatch_events(state, events, &mut next_event, sample_index);
                if !state.synth.is_active() && state.decimator_tail == 0 {
                    state
                        .lfos
                        .advance_silent(usize::from(state.oversampler.factor()));
                    for channel in 0..output_channels {
                        buffer.output(channel)[sample_index] = 0.0;
                    }
                    offset += 1;
                    continue;
                }

                let table_selections: [_; LEGACY_OSCILLATOR_COUNT] =
                    std::array::from_fn(|oscillator| {
                        let (curve_fade, position) = match oscillator {
                            0 => (
                                state.controls.osc1_curve_fade[offset],
                                state.controls.osc1_custom_shape[offset],
                            ),
                            1 => (
                                state.controls.osc2_curve_fade[offset],
                                state.controls.osc2_custom_shape[offset],
                            ),
                            _ => (
                                state.controls.osc3_curve_fade[offset],
                                state.controls.osc3_custom_shape[offset],
                            ),
                        };
                        state.va_table_transitions[oscillator].advance(state.host_sample_rate);
                        state.va_table_transitions[oscillator]
                            .select(state.wave_curves[oscillator].value(curve_fade), position)
                    });
                let mut settings = VoiceSettings::new(
                    state.controls.shape[offset],
                    110.0,
                    state.controls.pulse_width[offset],
                    state.controls.velocity[offset],
                    state.controls.pressure[offset],
                    state.controls.timbre[offset],
                )
                .with_antialiasing(antialiasing)
                .with_oscillators([
                    OscillatorSettings::new(
                        oscillator_enabled[0],
                        state.controls.shape[offset],
                        state.controls.pulse_width[offset],
                        OscillatorSettings::pitch_ratio(
                            oscillator_transpose[0],
                            state.controls.osc1_cents[offset],
                        ),
                        state.controls.osc1_level[offset],
                        state.controls.osc1_pan[offset],
                    )
                    .with_unison_detune_amount(params.unison_detune_amount.value())
                    .with_phase_warp(
                        oscillator_warp_mode[0],
                        state.controls.osc1_warp_amount[offset],
                    )
                    .with_custom_curve(table_selections[0].0, table_selections[0].1),
                    OscillatorSettings::new(
                        oscillator_enabled[1],
                        state.controls.osc2_shape[offset],
                        state.controls.osc2_pulse_width[offset],
                        OscillatorSettings::pitch_ratio(
                            oscillator_transpose[1],
                            state.controls.osc2_cents[offset],
                        ),
                        state.controls.osc2_level[offset],
                        state.controls.osc2_pan[offset],
                    )
                    .with_unison_detune_amount(params.osc2_unison_detune_amount.value())
                    .with_phase_warp(
                        oscillator_warp_mode[1],
                        state.controls.osc2_warp_amount[offset],
                    )
                    .with_custom_curve(table_selections[1].0, table_selections[1].1),
                    OscillatorSettings::new(
                        oscillator_enabled[2],
                        state.controls.osc3_shape[offset],
                        state.controls.osc3_pulse_width[offset],
                        OscillatorSettings::pitch_ratio(
                            oscillator_transpose[2],
                            state.controls.osc3_cents[offset],
                        ),
                        state.controls.osc3_level[offset],
                        state.controls.osc3_pan[offset],
                    )
                    .with_unison_detune_amount(params.osc3_unison_detune_amount.value())
                    .with_phase_warp(
                        oscillator_warp_mode[2],
                        state.controls.osc3_warp_amount[offset],
                    )
                    .with_custom_curve(table_selections[2].0, table_selections[2].1),
                ]);
                let envelope = EnvelopeSettings {
                    attack: state.controls.attack[offset],
                    decay: state.controls.decay[offset],
                    sustain: state.controls.sustain[offset],
                    release: state.controls.release[offset],
                    attack_curve: state.controls.attack_curve[offset],
                    decay_curve: state.controls.decay_curve[offset],
                    release_curve: state.controls.release_curve[offset],
                    attack_curve_time: state.controls.attack_curve_time[offset],
                    decay_curve_time: state.controls.decay_curve_time[offset],
                    release_curve_time: state.controls.release_curve_time[offset],
                };
                let oversampling_factor = state.oversampler.factor();
                let block_samples = state
                    .synth
                    .block_internal_samples(settings, oversampling_factor);
                let block_internal = block_samples.unwrap_or(0);
                let base_host_frames = block_internal / usize::from(oversampling_factor);
                let available_frames = block_len - offset;
                let event_free_frames = events.get(next_event).map_or(available_frames, |event| {
                    (event.sample_offset as usize)
                        .saturating_sub(sample_index)
                        .min(available_frames)
                });
                let table_transitioning = state
                    .va_table_transitions
                    .iter()
                    .any(VaTableTransition::active);
                let mut chunks = if base_host_frames == 0 || table_transitioning {
                    0
                } else {
                    (event_free_frames / base_host_frames).min(MAX_JOB_SAMPLES / block_internal)
                };
                let mut morphing = false;
                while chunks != 0 {
                    let frames = base_host_frames * chunks;
                    if state.controls.is_static(offset, frames, oscillator_enabled) {
                        break;
                    }
                    if state
                        .controls
                        .is_static_except_shape(offset, frames, oscillator_enabled)
                        && state.synth.morph_block_eligible(settings)
                    {
                        morphing = true;
                        break;
                    }
                    chunks -= 1;
                }
                let host_frames = base_host_frames * chunks;
                let lfo_morph_block = chunks != 0
                    && block_morph_lfo
                    && state.controls.is_static_except_shape(
                        offset,
                        host_frames,
                        oscillator_enabled,
                    )
                    && state.synth.morph_block_eligible(settings);
                let lfo_pitch_block = chunks != 0
                    && block_pitch_lfo
                    && state
                        .controls
                        .is_static(offset, host_frames, oscillator_enabled)
                    && state.synth.pitch_block_eligible(settings)
                    && state.synth.spatial_block_eligible();
                if lfo_pitch_block && state.block_major_enabled() {
                    let gain = static_gain
                        .unwrap_or_else(|| db_to_linear(state.controls.output_db[offset]));
                    let mut block_peak_left = 0.0_f32;
                    let mut block_peak_right = 0.0_f32;
                    for chunk in 0..chunks {
                        let host_chunk_start = offset + chunk * base_host_frames;
                        let sample_chunk_start = sample_index + chunk * base_host_frames;
                        let (peak_left, peak_right) = match block_samples {
                            Some(FACTOR3_BLOCK_INTERNAL_SAMPLES) => {
                                render_lfo_pitch_chunk::<FACTOR3_BLOCK_INTERNAL_SAMPLES>(
                                    state,
                                    buffer,
                                    output_channels,
                                    sample_chunk_start,
                                    host_chunk_start,
                                    settings,
                                    envelope,
                                    gain,
                                    &active_routes,
                                    lfo_control_dynamic_mask,
                                    &mut modulation,
                                )
                            }
                            Some(BLOCK_INTERNAL_SAMPLES) => {
                                render_lfo_pitch_chunk::<BLOCK_INTERNAL_SAMPLES>(
                                    state,
                                    buffer,
                                    output_channels,
                                    sample_chunk_start,
                                    host_chunk_start,
                                    settings,
                                    envelope,
                                    gain,
                                    &active_routes,
                                    lfo_control_dynamic_mask,
                                    &mut modulation,
                                )
                            }
                            _ => unreachable!(),
                        };
                        block_peak_left = block_peak_left.max(peak_left);
                        block_peak_right = block_peak_right.max(peak_right);
                    }
                    peak_left = peak_left.max(block_peak_left);
                    peak_right = peak_right.max(block_peak_right);
                    state.decimator_tail = oversampling::TAIL_SAMPLES;
                    #[cfg(test)]
                    {
                        state.block_major_chunks += chunks;
                    }
                    offset += host_frames;
                    continue;
                }
                let lfo_motion_block = chunks != 0
                    && block_motion_lfo
                    && state
                        .controls
                        .is_static(offset, host_frames, oscillator_enabled)
                    && state.synth.motion_block_eligible(settings);
                if lfo_motion_block && state.block_major_enabled() {
                    let gain = static_gain
                        .unwrap_or_else(|| db_to_linear(state.controls.output_db[offset]));
                    let mut block_peak_left = 0.0_f32;
                    let mut block_peak_right = 0.0_f32;
                    for chunk in 0..chunks {
                        let host_chunk_start = offset + chunk * base_host_frames;
                        let sample_chunk_start = sample_index + chunk * base_host_frames;
                        let (peak_left, peak_right) = match block_samples {
                            Some(FACTOR3_BLOCK_INTERNAL_SAMPLES) => {
                                render_lfo_motion_chunk::<FACTOR3_BLOCK_INTERNAL_SAMPLES>(
                                    state,
                                    buffer,
                                    output_channels,
                                    sample_chunk_start,
                                    host_chunk_start,
                                    settings,
                                    envelope,
                                    gain,
                                    &active_routes,
                                    &unison_settings,
                                    lfo_control_dynamic_mask,
                                    &mut modulation,
                                )
                            }
                            Some(BLOCK_INTERNAL_SAMPLES) => {
                                render_lfo_motion_chunk::<BLOCK_INTERNAL_SAMPLES>(
                                    state,
                                    buffer,
                                    output_channels,
                                    sample_chunk_start,
                                    host_chunk_start,
                                    settings,
                                    envelope,
                                    gain,
                                    &active_routes,
                                    &unison_settings,
                                    lfo_control_dynamic_mask,
                                    &mut modulation,
                                )
                            }
                            _ => unreachable!(),
                        };
                        block_peak_left = block_peak_left.max(peak_left);
                        block_peak_right = block_peak_right.max(peak_right);
                    }
                    peak_left = peak_left.max(block_peak_left);
                    peak_right = peak_right.max(block_peak_right);
                    state.decimator_tail = oversampling::TAIL_SAMPLES;
                    #[cfg(test)]
                    {
                        state.block_major_chunks += chunks;
                    }
                    offset += host_frames;
                    continue;
                }
                let lfo_control_block = chunks != 0
                    && block_control_lfo
                    && !lfo_morph_block
                    && !lfo_pitch_block
                    && state
                        .controls
                        .is_static(offset, host_frames, oscillator_enabled)
                    && state.synth.control_block_eligible();
                if lfo_control_block && state.block_major_enabled() {
                    let gain = static_gain
                        .unwrap_or_else(|| db_to_linear(state.controls.output_db[offset]));
                    let mut block_peak_left = 0.0_f32;
                    let mut block_peak_right = 0.0_f32;
                    for chunk in 0..chunks {
                        let host_chunk_start = offset + chunk * base_host_frames;
                        let sample_chunk_start = sample_index + chunk * base_host_frames;
                        let (peak_left, peak_right) = match block_samples {
                            Some(FACTOR3_BLOCK_INTERNAL_SAMPLES) => {
                                render_lfo_control_chunk::<FACTOR3_BLOCK_INTERNAL_SAMPLES>(
                                    state,
                                    buffer,
                                    output_channels,
                                    sample_chunk_start,
                                    host_chunk_start,
                                    settings,
                                    envelope,
                                    gain,
                                    &active_routes,
                                    lfo_control_dynamic_mask,
                                    &mut modulation,
                                )
                            }
                            Some(BLOCK_INTERNAL_SAMPLES) => {
                                render_lfo_control_chunk::<BLOCK_INTERNAL_SAMPLES>(
                                    state,
                                    buffer,
                                    output_channels,
                                    sample_chunk_start,
                                    host_chunk_start,
                                    settings,
                                    envelope,
                                    gain,
                                    &active_routes,
                                    lfo_control_dynamic_mask,
                                    &mut modulation,
                                )
                            }
                            _ => unreachable!(),
                        };
                        block_peak_left = block_peak_left.max(peak_left);
                        block_peak_right = block_peak_right.max(peak_right);
                    }
                    peak_left = peak_left.max(block_peak_left);
                    peak_right = peak_right.max(block_peak_right);
                    state.decimator_tail = oversampling::TAIL_SAMPLES;
                    #[cfg(test)]
                    {
                        state.block_major_chunks += chunks;
                    }
                    offset += host_frames;
                    continue;
                }
                if chunks != 0
                    && state.block_major_enabled()
                    && (!state.lfos.is_active() || lfo_morph_block)
                {
                    let gain = static_gain
                        .unwrap_or_else(|| db_to_linear(state.controls.output_db[offset]));
                    let output_active = active_routes.global_mask & GLOBAL_OUTPUT_MASK != 0;
                    let mut shapes = (morphing
                        || (lfo_morph_block && active_routes.oscillator_shape_mask != 0))
                        .then(|| {
                            state.controls.expanded_shapes(
                                offset,
                                host_frames,
                                usize::from(oversampling_factor),
                            )
                        });
                    let mut output_gains = [1.0; MAX_JOB_SAMPLES];
                    if lfo_morph_block {
                        fill_lfo_morph_block(
                            state,
                            &active_routes,
                            active_routes.oscillator_shape_mask,
                            output_active,
                            lfo_control_dynamic_mask,
                            offset,
                            host_frames,
                            usize::from(oversampling_factor),
                            shapes.as_mut(),
                            &mut output_gains,
                            &mut modulation,
                        );
                    }
                    let output_gains = (lfo_morph_block && output_active).then_some(&output_gains);
                    let (block_peak_left, block_peak_right) = match block_samples {
                        Some(FACTOR3_BLOCK_INTERNAL_SAMPLES) => {
                            render_saw_host_block::<FACTOR3_BLOCK_INTERNAL_SAMPLES>(
                                state,
                                buffer,
                                output_channels,
                                sample_index,
                                chunks,
                                settings,
                                envelope,
                                gain,
                                shapes.as_ref(),
                                output_gains,
                            )
                        }
                        Some(BLOCK_INTERNAL_SAMPLES) => {
                            render_saw_host_block::<BLOCK_INTERNAL_SAMPLES>(
                                state,
                                buffer,
                                output_channels,
                                sample_index,
                                chunks,
                                settings,
                                envelope,
                                gain,
                                shapes.as_ref(),
                                output_gains,
                            )
                        }
                        _ => unreachable!(),
                    };
                    peak_left = peak_left.max(block_peak_left);
                    peak_right = peak_right.max(block_peak_right);
                    if !lfo_morph_block {
                        state
                            .lfos
                            .advance_silent(host_frames * usize::from(oversampling_factor));
                    }
                    state.decimator_tail = oversampling::TAIL_SAMPLES;
                    #[cfg(test)]
                    {
                        if !morphing {
                            state.block_major_chunks += 1;
                        }
                    }
                    offset += host_frames;
                    continue;
                }
                let source_was_active = state.synth.is_active();
                let modulation_active = state.lfos.is_active() || direct_unison_pitch_mask != 0;
                if !modulation_active {
                    clear_modulation_frame(
                        &mut modulation,
                        &active_routes,
                        direct_unison_pitch_mask,
                    );
                }
                let (mut left, mut right) = if state.oversampler.factor() == 1 {
                    if modulation_active {
                        apply_modulation(
                            state,
                            &mut settings,
                            &active_routes,
                            &unison_settings,
                            state.controls.glide_time[offset],
                            direct_unison_pitch_mask,
                            lfo_control_dynamic_mask,
                            offset,
                            &mut modulation,
                        );
                    }
                    let render_envelope = if active_routes.global_mask & GLOBAL_ENVELOPE_MASK != 0 {
                        modulated_envelope(envelope, modulation.global)
                    } else {
                        envelope
                    };
                    let (left, right) = if modulation_active {
                        state.synth.render_with_modulation(
                            settings,
                            render_envelope,
                            modulation.unison,
                        )
                    } else {
                        state.synth.render_neutral(settings, render_envelope)
                    };
                    state.oversampler.process_direct(left, right)
                } else if state.oversampler.factor() == 2
                    && !state.synth.is_gliding()
                    && !state.lfos.is_active()
                    && direct_unison_pitch_mask == 0
                {
                    for (left, right) in state.synth.render_pair(settings, envelope) {
                        state.oversampler.push(left, right);
                    }
                    state.oversampler.output()
                } else {
                    let reuse_direct_modulation = modulation_active
                        && !state.lfos.is_active()
                        && active_routes.unison_layout_mask == 0;
                    for internal_sample in 0..usize::from(state.oversampler.factor()) {
                        let render_settings = if modulation_active {
                            if reuse_direct_modulation && internal_sample != 0 {
                                settings
                            } else {
                                let mut modulated = settings;
                                apply_modulation(
                                    state,
                                    &mut modulated,
                                    &active_routes,
                                    &unison_settings,
                                    state.controls.glide_time[offset],
                                    direct_unison_pitch_mask,
                                    lfo_control_dynamic_mask,
                                    offset,
                                    &mut modulation,
                                );
                                modulated
                            }
                        } else {
                            settings
                        };
                        let render_envelope =
                            if active_routes.global_mask & GLOBAL_ENVELOPE_MASK != 0 {
                                modulated_envelope(envelope, modulation.global)
                            } else {
                                envelope
                            };
                        let (left, right) = if modulation_active {
                            state.synth.render_with_modulation(
                                render_settings,
                                render_envelope,
                                modulation.unison,
                            )
                        } else {
                            state.synth.render_neutral(render_settings, render_envelope)
                        };
                        state.oversampler.push(left, right);
                    }
                    state.oversampler.output()
                };
                if !modulation_active {
                    state
                        .lfos
                        .advance_silent(usize::from(state.oversampler.factor()));
                }

                if source_was_active || state.synth.is_active() {
                    state.decimator_tail = oversampling::TAIL_SAMPLES;
                } else {
                    state.decimator_tail = state.decimator_tail.saturating_sub(1);
                    if state.set_oversampling(requested_factor, requested_antialiasing) {
                        antialiasing =
                            requested_antialiasing.for_factor(state.oversampler.factor());
                    }
                }

                let gain = if active_routes.global_mask & GLOBAL_OUTPUT_MASK != 0 {
                    static_gain.map_or_else(
                        || {
                            db_to_linear(
                                state.controls.output_db[offset] + modulation.global.output_db,
                            )
                        },
                        |base| base * db_to_linear(modulation.global.output_db),
                    )
                } else {
                    static_gain.unwrap_or_else(|| db_to_linear(state.controls.output_db[offset]))
                };
                left *= gain;
                right *= gain;
                peak_left = peak_left.max(left.abs());
                peak_right = peak_right.max(right.abs());
                if output_channels == 1 {
                    buffer.output(0)[sample_index] = (left + right) * 0.5;
                } else {
                    buffer.output(0)[sample_index] = left;
                    buffer.output(1)[sample_index] = right;
                }
                offset += 1;
            }
            block_start += block_len;
        }

        let (peak_left, peak_right) =
            route_group_output(buffer, state.generator_output, output_channels);
        publish_meters(
            state,
            params,
            context,
            buffer.num_samples(),
            peak_left,
            peak_right,
        );

        current_process_status(state)
    }

    fn latency(_state: &KurvDspState) -> u32 {
        oversampling::LATENCY_SAMPLES
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the bounded host sample rate and 12-second release fit comfortably in u32"
    )]
    fn tail(state: &KurvDspState) -> u32 {
        (state.host_sample_rate * 12.0).round() as u32 + u32::from(oversampling::TAIL_SAMPLES)
    }

    fn migrate_state(foreign: &ForeignState) -> Option<MigratedState> {
        diagnostics::lifecycle("migrate-state-enter");
        let ForeignState::Raw { bytes, .. } = foreign else {
            diagnostics::lifecycle("migrate-state-not-raw");
            return None;
        };
        diagnostics::trace("state", "migrate-state-bytes", bytes.len() as f32, 0.0);
        let root: serde_json::Value = match serde_json::from_slice(bytes) {
            Ok(root) => root,
            Err(_) => {
                diagnostics::lifecycle("migrate-state-json-failed");
                return None;
            }
        };
        let Some(old) = root.get("params").and_then(serde_json::Value::as_object) else {
            diagnostics::lifecycle("migrate-state-params-missing");
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
            diagnostics::lifecycle("migrate-state-no-legacy-params");
            return None;
        }
        diagnostics::trace("state", "migrate-state-return", params.len() as f32, 0.0);
        Some(MigratedState {
            params,
            ..MigratedState::default()
        })
    }

    fn editor(params: Arc<KurvParams>) -> Box<dyn Editor> {
        editor::create(params)
    }
}

fn route_group_output(
    buffer: &mut AudioBuffer,
    output: generators::GroupOutput,
    output_channels: usize,
) -> (f32, f32) {
    if output_channels == 0 {
        return (0.0, 0.0);
    }
    let requested = usize::from(output.pair) * 2;
    let target = (requested + 1 < output_channels).then_some(requested);
    let gain = output.gain.clamp(0.0, 2.0);
    let pan = output.pan.clamp(-1.0, 1.0);
    let left_gain = gain * (1.0 - pan).sqrt();
    let right_gain = gain * (1.0 + pan).sqrt();
    let mut peak_left = 0.0_f32;
    let mut peak_right = 0.0_f32;
    for sample in 0..buffer.num_samples() {
        let source_left = buffer.output(0)[sample];
        let source_right = if output_channels > 1 {
            buffer.output(1)[sample]
        } else {
            source_left
        };
        let left = source_left * left_gain;
        let right = source_right * right_gain;
        if target.is_some() {
            peak_left = peak_left.max(left.abs());
            peak_right = peak_right.max(right.abs());
        }
        for channel in 0..output_channels {
            buffer.output(channel)[sample] = if Some(channel) == target {
                left
            } else if target.is_some_and(|target| channel == target + 1) {
                right
            } else {
                0.0
            };
        }
    }
    (peak_left, peak_right)
}
