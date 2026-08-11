use std::sync::Arc;

use super::*;

fn group_output_envelope(output: generators::GroupOutput) -> EnvelopeSettings {
    EnvelopeSettings {
        attack: output.attack,
        decay: output.decay,
        sustain: output.sustain,
        release: output.release,
        attack_curve: output.attack_curve,
        decay_curve: output.decay_curve,
        release_curve: output.release_curve,
        attack_curve_time: 0.0,
        decay_curve_time: 0.0,
        release_curve_time: 0.0,
    }
}

pub struct Kurv;

impl PluginLogic for Kurv {
    type Params = KurvParams;
    type DspState = KurvDspState;

    const PRESERVE_DSP_STATE: bool = false;

    fn bus_layouts() -> Vec<BusLayout> {
        let mut layout = BusLayout::new();
        for (index, name) in [
            "Output 1/2",
            "Output 3/4",
            "Output 5/6",
            "Output 7/8",
            "Output 9/10",
            "Output 11/12",
            "Output 13/14",
            "Output 15/16",
        ]
        .into_iter()
        .enumerate()
        {
            layout = layout.with_output(name, ChannelConfig::Stereo);
            if index != 0 {
                layout.outputs[index].kind = BusKind::Sidechain;
            }
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
        state.reset_lfo_curve_generations();
        state.lfos.reset(state.dsp_sample_rate);
        state.mod_wheel_ramp = RouteAmountRamp::default();
        state
            .mod_wheel_ramp
            .retarget(params.mod_wheel.value(), state.dsp_sample_rate);
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
        let structural_render = params.generator_stack.is_materialized();
        let materialized_changed = structural_render != state.generator_materialized;
        let mut oscillator_configs_dirty = materialized_changed;
        state.generator_materialized = structural_render;
        if materialized_changed {
            state.pan_shape_generations.fill(u32::MAX);
        }

        let (requested_factor, requested_antialiasing) = generator_configuration(params);
        state.set_oversampling(requested_factor, requested_antialiasing);
        if let Some((generation, snapshot)) = params
            .generator_stack
            .try_rt_snapshot_after(state.generator_rt_generation)
        {
            state.generator_rt_generation = generation;
            let previous_oscillators = state.generator_oscillators;
            let previous_group_count = state.generator_group_count;
            let previous_group_masks = state.generator_group_masks;
            state.generator_oscillators = *snapshot.oscillators();
            oscillator_configs_dirty |= previous_oscillators != state.generator_oscillators;
            state.generator_module_ids = *snapshot.module_ids();
            state.generator_group_count = snapshot.group_count().max(1);
            state.generator_group_masks.fill(0);
            state.generator_group_ids.fill(0);
            state
                .generator_group_outputs
                .fill(generators::GroupOutput::default());
            for (index, group) in snapshot.groups().iter().copied().enumerate() {
                state.generator_group_ids[index] = group.id();
                state.generator_group_masks[index] = group.oscillator_mask();
                state.generator_group_outputs[index] = group.output();
            }
            state.generator_active_mask = state.generator_group_masks
                [..state.generator_group_count]
                .iter()
                .fold(0, |mask, group| mask | group);
            state.generator_oscillator_groups.fill(0);
            for (group_index, mask) in state.generator_group_masks[..state.generator_group_count]
                .iter()
                .copied()
                .enumerate()
            {
                for oscillator in 0..generators::MAX_OSCILLATORS {
                    if mask & (1_u32 << oscillator) != 0 {
                        state.generator_oscillator_groups[oscillator] = group_index as u8;
                    }
                }
            }
            if previous_group_count != state.generator_group_count
                || previous_group_masks != state.generator_group_masks
            {
                let factor = state.oversampler.factor();
                state.oversampler.reset(factor);
                for oversampler in &mut *state.group_oversamplers {
                    oversampler.reset(factor);
                }
            }
        }
        if let Some((generation, targets)) = params
            .modulation_route_targets
            .try_rt_snapshot_after(state.modular_route_generation)
        {
            state.modular_route_generation = generation;
            state.modular_route_targets = targets;
        }
        if let Some((generation, routes)) = params
            .modulation_route_overflow
            .try_rt_snapshot_after(state.overflow_route_generation)
        {
            state.overflow_route_generation = generation;
            let sample_rate = state.dsp_sample_rate;
            for (ramp, route) in state.overflow_route_ramps.iter_mut().zip(routes) {
                ramp.retarget(route.amount, sample_rate);
            }
            state.overflow_routes = routes;
        }
        refresh_host_automation_targets(state, params);
        let (effective_oscillators, effective_groups) =
            host_automated_generator_configuration(state, params);
        oscillator_configs_dirty |= effective_oscillators != state.effective_generator_oscillators;
        state.effective_generator_oscillators = effective_oscillators;
        state.effective_generator_group_outputs = effective_groups;
        state.synth.configure_output_groups(
            effective_groups.map(group_output_envelope),
            effective_groups.map(|output| output.receive_midi_channel),
            state.generator_group_count,
        );
        let modulation_routes = modulation_routes(params);
        // The fixed three-value branch exists only to play projects saved
        // before the structural stack. New patches configure the 32-slot bank.
        let oscillator_enabled = if structural_render {
            [false; LEGACY_OSCILLATOR_COUNT]
        } else {
            [
                params.osc1_enabled.value() && state.generator_active_mask & 1 != 0,
                params.osc2_enabled.value() && state.generator_active_mask & 2 != 0,
                params.osc3_enabled.value() && state.generator_active_mask & 4 != 0,
            ]
        };
        let active_routes = active_modulation_routes(
            &modulation_routes,
            &state.modular_route_targets,
            &state.overflow_routes,
            params.mod_wheel_route_mask.load(),
            &state.generator_module_ids,
            &state.generator_group_ids,
            state.generator_group_count,
            oscillator_enabled,
        );
        let grouped_render = state.generator_group_count > 1
            || (structural_render && active_routes.modular_group_mask != 0);
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
            let mut lfo_curves = [None; LFO_COUNT];
            for (index, curve) in lfo_curve_states.iter().enumerate() {
                if lfo_sources & (1 << index) != 0
                    && let Some((generation, compiled)) =
                        curve.try_curve_rt_after(state.lfo_curve_generations[index])
                {
                    state.lfo_curve_generations[index] = generation;
                    lfo_curves[index] = Some(compiled);
                }
            }
            for (index, generation) in state
                .lfo_curve_generations
                .iter_mut()
                .enumerate()
                .skip(LEGACY_MODULATION_SOURCES)
            {
                if lfo_sources & (1_u64 << index) == 0 {
                    continue;
                }
                if let Some(curve) = params.modulator_rack.curve(index)
                    && let Some((next_generation, compiled)) = curve.try_curve_rt_after(*generation)
                {
                    *generation = next_generation;
                    lfo_curves[index] = Some(compiled);
                }
            }
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
        for oversampler in &mut *state.group_oversamplers {
            oversampler
                .set_spline_correction(matches!(antialiasing, Antialiasing::SplineOptimized));
        }
        if !structural_render {
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
        }
        if structural_render {
            for oscillator in 0..generators::MAX_OSCILLATORS {
                if state.generator_active_mask & (1_u32 << oscillator) != 0
                    && let Some(slot) = generators::OscillatorSlot::from_index(oscillator)
                    && let Some((generation, segments)) = params
                        .generator_stack
                        .pan_shape_curve(slot)
                        .try_segments_rt_after(state.pan_shape_generations[oscillator])
                {
                    state.pan_shape_generations[oscillator] = generation;
                    state.pan_shape_segments[oscillator] = segments;
                    oscillator_configs_dirty = true;
                }
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
                oscillator_configs_dirty = true;
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
        if oscillator_configs_dirty {
            let oscillators = std::array::from_fn(|index| {
                let config = state.effective_generator_oscillators[index];
                let table =
                    state.va_tables[index].select(state.base_wave_curve, config.custom_shape);
                OscillatorDspConfig {
                    enabled: structural_render && config.enabled,
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
                    unison_jitter_mode: config.unison_jitter_mode,
                    unison_rate: config.unison_rate,
                    unison_width: config.unison_width,
                    unison_weight: config.unison_weight,
                    phase_position: config.phase_position,
                    phase_random: config.phase_random,
                    phase_warp_mode: config.phase_warp_mode,
                    phase_warp_amount: config.phase_warp_amount,
                    unison_alignment: config.unison_alignment,
                    unison_alignment_mode: config.unison_alignment_mode,
                    unison_pan_curve: config.unison_pan_curve,
                    unison_pan_center_x: config.unison_pan_center_x,
                    unison_pan_segments: state.pan_shape_segments[index],
                    unison_stereo_x: config.unison_stereo_x,
                    unison_stereo_alternate: config.unison_stereo_alternate,
                }
            });
            state.synth.configure_oscillators(oscillators);
        }
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
            let route_modulation_active = state.lfos.is_active() || active_routes.mod_wheel_active;
            let block_morph_lfo = route_modulation_active
                && block_morph_modulation(&active_routes)
                && direct_unison_pitch_mask == 0
                && direct_unison_motion_mask == 0
                && !unison_settings
                    .iter()
                    .any(|settings| settings.motion_active());
            let block_pitch_lfo = route_modulation_active
                && block_pitch_modulation(&active_routes)
                && direct_unison_pitch_mask == 0
                && direct_unison_motion_mask == 0
                && !unison_settings
                    .iter()
                    .any(|settings| settings.motion_active());
            let block_control_lfo = route_modulation_active
                && block_parameter_modulation(&active_routes)
                && direct_unison_pitch_mask == 0
                && direct_unison_motion_mask == 0
                && !unison_settings
                    .iter()
                    .any(|settings| settings.motion_active());
            let block_motion_lfo = route_modulation_active
                && block_motion_modulation(&active_routes)
                && direct_unison_pitch_mask == 0
                && direct_unison_motion_mask == 0;

            let mut offset = 0;
            let mut modulation = lfo::ModulationFrame::default();
            let mut structural_modulation = StructuralModulationFrame::default();
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
                dispatch_events(state, params, events, &mut next_event, sample_index);
                if !state.synth.is_active() && state.decimator_tail == 0 {
                    state
                        .lfos
                        .advance_silent(usize::from(state.oversampler.factor()));
                    if active_routes.mod_wheel_active {
                        state.mod_wheel_ramp.finish();
                    }
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
                let envelope = if structural_render {
                    group_output_envelope(state.effective_generator_group_outputs[0])
                } else {
                    EnvelopeSettings {
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
                    }
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
                if lfo_pitch_block && state.block_major_enabled() && !grouped_render {
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
                if lfo_motion_block && state.block_major_enabled() && !grouped_render {
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
                if lfo_control_block && state.block_major_enabled() && !grouped_render {
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
                    && !grouped_render
                    && (!route_modulation_active || lfo_morph_block)
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
                let modulation_active = route_modulation_active || direct_unison_pitch_mask != 0;
                if !modulation_active {
                    clear_modulation_frame(
                        &mut modulation,
                        &active_routes,
                        direct_unison_pitch_mask,
                    );
                }
                let mut grouped_stems = [(0.0_f32, 0.0_f32); generators::MAX_OUTPUT_PAIRS];
                let (mut left, mut right) = if grouped_render {
                    if state.oversampler.factor() == 1 {
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
                                &mut structural_modulation,
                            );
                        }
                        let render_envelope =
                            if active_routes.global_mask & GLOBAL_ENVELOPE_MASK != 0 {
                                modulated_envelope(envelope, modulation.global)
                            } else {
                                envelope
                            };
                        let structural_control =
                            structural_oscillator_frame_control(state, &structural_modulation);
                        let rendered = if modulation_active {
                            state
                                .synth
                                .render_grouped_with_modulation_and_structural_frame(
                                    settings,
                                    render_envelope,
                                    modulation.unison,
                                    &structural_control,
                                    &state.generator_oscillator_groups,
                                    state.generator_group_count,
                                )
                        } else {
                            state.synth.render_grouped_neutral(
                                settings,
                                render_envelope,
                                &state.generator_oscillator_groups,
                                state.generator_group_count,
                            )
                        };
                        for group in 0..state.generator_group_count {
                            grouped_stems[group] = state.group_oversamplers[group]
                                .process_direct(rendered[group].0, rendered[group].1);
                        }
                    } else {
                        let reuse_direct_modulation = modulation_active
                            && !route_modulation_active
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
                                        &mut structural_modulation,
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
                            let structural_control =
                                structural_oscillator_frame_control(state, &structural_modulation);
                            let rendered = if modulation_active {
                                state
                                    .synth
                                    .render_grouped_with_modulation_and_structural_frame(
                                        render_settings,
                                        render_envelope,
                                        modulation.unison,
                                        &structural_control,
                                        &state.generator_oscillator_groups,
                                        state.generator_group_count,
                                    )
                            } else {
                                state.synth.render_grouped_neutral(
                                    render_settings,
                                    render_envelope,
                                    &state.generator_oscillator_groups,
                                    state.generator_group_count,
                                )
                            };
                            for group in 0..state.generator_group_count {
                                state.group_oversamplers[group]
                                    .push(rendered[group].0, rendered[group].1);
                            }
                        }
                        for (group, output) in grouped_stems[..state.generator_group_count]
                            .iter_mut()
                            .enumerate()
                        {
                            *output = state.group_oversamplers[group].output();
                        }
                    }
                    (0.0, 0.0)
                } else if state.oversampler.factor() == 1 {
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
                            &mut structural_modulation,
                        );
                    }
                    let render_envelope = if active_routes.global_mask & GLOBAL_ENVELOPE_MASK != 0 {
                        modulated_envelope(envelope, modulation.global)
                    } else {
                        envelope
                    };
                    let structural_control =
                        structural_oscillator_frame_control(state, &structural_modulation);
                    let (left, right) = if modulation_active {
                        state.synth.render_with_modulation_and_structural_frame(
                            settings,
                            render_envelope,
                            modulation.unison,
                            &structural_control,
                        )
                    } else {
                        state.synth.render_neutral(settings, render_envelope)
                    };
                    state.oversampler.process_direct(left, right)
                } else if state.oversampler.factor() == 2
                    && !state.synth.is_gliding()
                    && !route_modulation_active
                    && direct_unison_pitch_mask == 0
                {
                    for (left, right) in state.synth.render_pair(settings, envelope) {
                        state.oversampler.push(left, right);
                    }
                    state.oversampler.output()
                } else {
                    let reuse_direct_modulation = modulation_active
                        && !route_modulation_active
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
                                    &mut structural_modulation,
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
                        let structural_control =
                            structural_oscillator_frame_control(state, &structural_modulation);
                        let (left, right) = if modulation_active {
                            state.synth.render_with_modulation_and_structural_frame(
                                render_settings,
                                render_envelope,
                                modulation.unison,
                                &structural_control,
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
                if grouped_render {
                    for stem in &mut grouped_stems[..state.generator_group_count] {
                        stem.0 *= gain;
                        stem.1 *= gain;
                    }
                    let (frame_peak_left, frame_peak_right) = route_group_frame(
                        buffer,
                        sample_index,
                        &grouped_stems[..state.generator_group_count],
                        &state.effective_generator_group_outputs[..state.generator_group_count],
                        &structural_modulation,
                        output_channels,
                    );
                    peak_left = peak_left.max(frame_peak_left);
                    peak_right = peak_right.max(frame_peak_right);
                } else {
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
                }
                offset += 1;
            }
            block_start += block_len;
        }

        let (peak_left, peak_right) = if grouped_render {
            (peak_left, peak_right)
        } else {
            route_group_output(
                buffer,
                state.effective_generator_group_outputs[0],
                output_channels,
            )
        };
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
        reason = "the bounded host sample rate and 20-second group release fit comfortably in u32"
    )]
    fn tail(state: &KurvDspState) -> u32 {
        (state.host_sample_rate * 20.0).round() as u32 + u32::from(oversampling::TAIL_SAMPLES)
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

fn route_group_frame(
    buffer: &mut AudioBuffer,
    sample: usize,
    stems: &[(f32, f32)],
    outputs: &[generators::GroupOutput],
    modulation: &StructuralModulationFrame,
    output_channels: usize,
) -> (f32, f32) {
    for channel in 0..output_channels {
        buffer.output(channel)[sample] = 0.0;
    }
    for (group, ((left, right), output)) in stems
        .iter()
        .copied()
        .zip(outputs.iter().copied())
        .enumerate()
    {
        let target = usize::from(output.pair) * 2;
        if target + 1 >= output_channels {
            continue;
        }
        let delta = if modulation.group_mask & (1 << group) != 0 {
            modulation.groups[group]
        } else {
            StructuralGroupDelta::default()
        };
        let gain = (output.gain + delta.gain).clamp(0.0, 2.0);
        let pan = (output.pan + delta.pan).clamp(-1.0, 1.0);
        buffer.output(target)[sample] += left * gain * (1.0 - pan).sqrt();
        buffer.output(target + 1)[sample] += right * gain * (1.0 + pan).sqrt();
    }
    let mut peak_left = 0.0_f32;
    let mut peak_right = 0.0_f32;
    for pair in 0..output_channels / 2 {
        peak_left = peak_left.max(buffer.output(pair * 2)[sample].abs());
        peak_right = peak_right.max(buffer.output(pair * 2 + 1)[sample].abs());
    }
    (peak_left, peak_right)
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
