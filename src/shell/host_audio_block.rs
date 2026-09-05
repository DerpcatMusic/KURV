use super::*;
use crate::cpu_profile;
use crate::runtime::events::apply_incoming_param_mods;

fn publish_resynth_rt(state: &mut KurvDspState, params: &KurvParams) -> u32 {
    let mut accepted_mask = 0;
    if let Some(update) = params
        .resynth_assets
        .try_rt_update_after(&state.resynth_observed)
        && state.synth.try_retarget_resynth_batch(&update)
    {
        accepted_mask = update.changed_mask;
        for (index, observed) in state.resynth_observed.iter_mut().enumerate() {
            if update.changed_mask & (1_u32 << index) != 0 {
                *observed = update.views[index].generation();
            }
        }
    }
    for index in 0..generators::MAX_OSCILLATORS {
        let Some(slot) = params.resynth_assets.slot(index) else {
            continue;
        };
        let _ = state
            .synth
            .set_resynth_source_audition(index, slot.consume_source_audition_lease());
        if let Some(controls) = slot.rt_grain_controls() {
            state.synth.set_resynth_grain_controls(index, controls);
        }
        slot.acknowledge_rt(
            state.resynth_observed[index],
            state.synth.resynth_plan_ack(index),
        );
    }
    accepted_mask
}

fn publish_resynth_telemetry(state: &mut KurvDspState, params: &KurvParams, audio_frames: usize) {
    let mut interested = 0_u32;
    for index in 0..generators::MAX_OSCILLATORS {
        if params
            .resynth_assets
            .slot(index)
            .is_some_and(|slot| slot.consume_telemetry_interest())
        {
            interested |= 1_u32 << index;
        }
    }
    if interested == 0 {
        return;
    }
    let (publish_frame, audio_frame) = state.synth.begin_resynth_telemetry_block(audio_frames);
    for index in 0..generators::MAX_OSCILLATORS {
        if interested & (1_u32 << index) == 0 {
            continue;
        }
        let Some(slot) = params.resynth_assets.slot(index) else {
            continue;
        };
        let mut snapshot = crate::resynth_state::ResynthTelemetrySnapshot::default();
        state.synth.write_resynth_telemetry(index, &mut snapshot);
        snapshot.publish_frame = publish_frame;
        snapshot.audio_frame = audio_frame;
        snapshot.publish_count = publish_frame;
        slot.publish_telemetry(snapshot);
    }
}

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

fn host_input_frame(buffer: &AudioBuffer, frame: usize) -> (f32, f32) {
    match buffer.num_input_channels() {
        0 => (0.0, 0.0),
        1 => {
            let sample = buffer.input(0)[frame];
            (sample, sample)
        }
        _ => (buffer.input(0)[frame], buffer.input(1)[frame]),
    }
}

fn render_global_aux_input(
    state: &mut KurvDspState,
    input: (f32, f32),
    stems: &mut [(f32, f32); generators::MAX_OUTPUT_PAIRS],
) {
    if state.generator_audio_input_group_mask == 0 {
        state.global_audio_input_tap = input;
        return;
    }
    let mut next_taps = [(0.0_f32, 0.0_f32); generators::MAX_OUTPUT_PAIRS];
    for (group_index, group) in state.generator_groups[..state.generator_group_count]
        .iter()
        .enumerate()
    {
        let mut signal = (0.0_f32, 0.0_f32);
        for module in group.modules() {
            match *module {
                generators::GeneratorRtModule::Oscillator(_) => {}
                generators::GeneratorRtModule::Filter(slot) => {
                    signal = state.global_filters[slot.index()].process(
                        state.generator_filter_coefficients[slot.index()],
                        signal.0,
                        signal.1,
                    );
                }
                generators::GeneratorRtModule::Aux(slot) => {
                    let config = state.generator_aux[slot.index()];
                    let source = match config.source {
                        generators::AuxSource::AudioInput => state.global_audio_input_tap,
                        generators::AuxSource::Group(source) => state.generator_group_ids
                            [..state.generator_group_count]
                            .iter()
                            .position(|id| *id == source.get())
                            .map_or((0.0, 0.0), |index| state.global_aux_group_taps[index]),
                    };
                    signal.0 += source.0 * config.gain;
                    signal.1 += source.1 * config.gain;
                }
            }
        }
        signal.0 = if signal.0.is_finite() { signal.0 } else { 0.0 };
        signal.1 = if signal.1.is_finite() { signal.1 } else { 0.0 };
        next_taps[group_index] = signal;
        stems[group_index].0 += signal.0;
        stems[group_index].1 += signal.1;
    }
    state.global_audio_input_tap = input;
    state.global_aux_group_taps = next_taps;
}

#[allow(
    clippy::too_many_lines,
    reason = "the audio callback keeps event dispatch, synthesis, oversampling, and meters in one RT boundary"
)]
pub(super) fn process(
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
    if !params.activation.features_enabled() {
        for channel in 0..output_channels {
            buffer.output(channel).fill(0.0);
        }
        return ProcessStatus::Tail(0);
    }
    // The profile is off unless KURV_CPU_PROFILE is set, in which case every
    // call below is a predictable branch on a relaxed atomic.
    let mut profile =
        cpu_profile::BlockProfile::begin(buffer.num_samples(), state.oversampler.factor());
    profile.enter(cpu_profile::Item::Events);
    apply_incoming_param_mods(state, params, events);
    profile.enter(cpu_profile::Item::Configuration);
    // Load/activate copies pre-modular host parameters into the generator
    // document. Audio always plays that stack after the instance exists.
    let structural_render = true;
    let materialized_changed = structural_render != state.generator_materialized;
    let mut oscillator_configs_dirty = materialized_changed;
    let mut filter_configs_dirty = materialized_changed;
    let mut filter_modes_dirty = false;
    let mut aux_configs_dirty = materialized_changed;
    let mut aux_sources_dirty = materialized_changed;
    let mut group_outputs_dirty = materialized_changed;
    state.generator_materialized = structural_render;
    if materialized_changed {
        state.pan_shape_generations.fill(u32::MAX);
    }

    let (requested_factor, requested_antialiasing) = generator_configuration(params);
    state.set_oversampling(requested_factor);
    if let Some((generation, snapshot)) = params
        .generator_stack
        .try_rt_topology_snapshot_after(state.generator_topology_generation)
    {
        state.generator_topology_generation = generation;
        let previous_oscillators = state.generator_oscillators;
        let previous_filters = state.generator_filters;
        let previous_groups = state.generator_groups;
        let previous_group_outputs = state.generator_group_outputs;
        let previous_group_count = state.generator_group_count;
        state.generator_oscillators = *snapshot.oscillators();
        state.generator_filters = *snapshot.filters();
        state.generator_aux = *snapshot.aux();
        state.synth.configure_aux(state.generator_aux);
        oscillator_configs_dirty |= previous_oscillators != state.generator_oscillators;
        filter_configs_dirty |= previous_filters != state.generator_filters;
        state.generator_module_ids = *snapshot.module_ids();
        state.generator_filter_module_ids = *snapshot.filter_module_ids();
        state.generator_aux_module_ids = *snapshot.aux_module_ids();
        state.generator_group_count = snapshot.group_count().max(1);
        group_outputs_dirty |= previous_group_count != state.generator_group_count;
        state
            .generator_groups
            .fill(generators::GeneratorRtGroup::EMPTY);
        state.generator_group_masks.fill(0);
        state.generator_group_ids.fill(0);
        state
            .generator_group_outputs
            .fill(generators::GroupOutput::default());
        for (index, group) in snapshot.groups().iter().copied().enumerate() {
            state.generator_groups[index] = group;
            state.generator_group_ids[index] = group.id();
            state.generator_group_masks[index] = group.oscillator_mask();
            state.generator_group_outputs[index] = group.output();
        }
        state.generator_has_aux = state.generator_groups[..state.generator_group_count]
            .iter()
            .any(generators::GeneratorRtGroup::has_aux);
        state.generator_audio_input_group_mask = 0;
        group_outputs_dirty |= previous_group_outputs != state.generator_group_outputs;
        state.generator_filter_mask = state.generator_groups[..state.generator_group_count]
            .iter()
            .enumerate()
            .filter(|(_, group)| {
                group.output().enabled && (group.oscillator_mask() != 0 || group.has_aux())
            })
            .fold(0_u32, |mask, (_, group)| mask | group.filter_mask());
        state.generator_has_filters = state.generator_filter_mask != 0;
        state
            .synth
            .configure_filter_mask(state.generator_filter_mask);
        state.generator_active_mask = state.generator_group_masks[..state.generator_group_count]
            .iter()
            .zip(&state.generator_group_outputs[..state.generator_group_count])
            .fold(0, |mask, (group, output)| {
                mask | if output.enabled { *group } else { 0 }
            });
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
        let topology_changed = previous_group_count != state.generator_group_count
            || previous_groups[..previous_group_count]
                .iter()
                .zip(&state.generator_groups[..state.generator_group_count])
                .any(|(before, after)| {
                    before.id() != after.id() || before.modules() != after.modules()
                });
        if topology_changed {
            let factor = state.oversampler.factor();
            state.oversampler.reset(factor);
            for oversampler in &mut *state.group_oversamplers {
                oversampler.reset(factor);
            }
            state.synth.reset_filter_states();
            state.synth.reset_generator_taps();
            for filter in &mut *state.global_filters {
                filter.reset();
            }
            state.global_audio_input_tap = (0.0, 0.0);
            state.global_aux_group_taps.fill((0.0, 0.0));
        }
    }
    let oscillator_generation = params.generator_stack.oscillator_rt_generation();
    if oscillator_generation != state.generator_oscillator_generation {
        let coherence_generation = params.generator_stack.rt_coherence_generation();
        for index in 0..generators::MAX_OSCILLATORS {
            let Some(slot) = generators::OscillatorSlot::from_index(index) else {
                continue;
            };
            if let Some((generation, config)) = params
                .generator_stack
                .try_oscillator_rt_after(slot, state.generator_oscillator_generations[index])
            {
                state.generator_oscillator_generations[index] = generation;
                oscillator_configs_dirty |= state.generator_oscillators[index] != config;
                state.generator_oscillators[index] = config;
            }
        }
        if coherence_generation & 1 == 0
            && params.generator_stack.rt_coherence_generation() == coherence_generation
            && params.generator_stack.oscillator_rt_generation() == oscillator_generation
        {
            state.generator_oscillator_generation = oscillator_generation;
        }
    }
    let filter_generation = params.generator_stack.filter_rt_generation();
    if filter_generation != state.generator_filter_generation {
        let coherence_generation = params.generator_stack.rt_coherence_generation();
        for index in 0..generators::MAX_FILTERS {
            let Some(slot) = generators::FilterSlot::from_index(index) else {
                continue;
            };
            if let Some((generation, config)) = params
                .generator_stack
                .try_filter_rt_after(slot, state.generator_filter_generations[index])
            {
                state.generator_filter_generations[index] = generation;
                filter_modes_dirty |= state.generator_filters[index].mode != config.mode;
                filter_configs_dirty |= state.generator_filters[index] != config;
                state.generator_filters[index] = config;
            }
        }
        if coherence_generation & 1 == 0
            && params.generator_stack.rt_coherence_generation() == coherence_generation
            && params.generator_stack.filter_rt_generation() == filter_generation
        {
            state.generator_filter_generation = filter_generation;
        }
    }
    let aux_generation = params.generator_stack.aux_rt_generation();
    if aux_generation != state.generator_aux_generation {
        let coherence_generation = params.generator_stack.rt_coherence_generation();
        for index in 0..generators::MAX_AUX_MODULES {
            let Some(slot) = generators::AuxSlot::from_index(index) else {
                continue;
            };
            if let Some((generation, config)) = params
                .generator_stack
                .try_aux_rt_after(slot, state.generator_aux_generations[index])
            {
                state.generator_aux_generations[index] = generation;
                aux_sources_dirty |= state.generator_aux[index].source != config.source;
                aux_configs_dirty |= state.generator_aux[index] != config;
                state.generator_aux[index] = config;
            }
        }
        if coherence_generation & 1 == 0
            && params.generator_stack.rt_coherence_generation() == coherence_generation
            && params.generator_stack.aux_rt_generation() == aux_generation
        {
            state.generator_aux_generation = aux_generation;
        }
    }
    if aux_configs_dirty {
        state.synth.configure_aux(state.generator_aux);
        state.generator_audio_input_group_mask = 0;
        state.generator_filter_mask = state.generator_groups[..state.generator_group_count]
            .iter()
            .enumerate()
            .filter(|(_, group)| {
                group.output().enabled && (group.oscillator_mask() != 0 || group.has_aux())
            })
            .fold(0_u32, |mask, (_, group)| mask | group.filter_mask());
        state.generator_has_filters = state.generator_filter_mask != 0;
        state
            .synth
            .configure_filter_mask(state.generator_filter_mask);
        if aux_sources_dirty {
            state.global_audio_input_tap = (0.0, 0.0);
            state.global_aux_group_taps.fill((0.0, 0.0));
        }
    }
    if filter_modes_dirty {
        state.synth.reset_filter_states();
        for filter in &mut *state.global_filters {
            filter.reset();
        }
    }
    let group_output_generation = params.generator_stack.group_output_rt_generation();
    if group_output_generation != state.generator_group_output_generation {
        let coherence_generation = params.generator_stack.rt_coherence_generation();
        for index in 0..generators::MAX_OUTPUT_PAIRS {
            if let Some((generation, group_id, output)) = params
                .generator_stack
                .try_group_output_rt_after(index, state.generator_group_output_generations[index])
            {
                state.generator_group_output_generations[index] = generation;
                if index < state.generator_group_count
                    && state.generator_group_ids[index] == group_id
                {
                    group_outputs_dirty |= state.generator_group_outputs[index] != output;
                    state.generator_group_outputs[index] = output;
                }
            }
        }
        if coherence_generation & 1 == 0
            && params.generator_stack.rt_coherence_generation() == coherence_generation
            && params.generator_stack.group_output_rt_generation() == group_output_generation
        {
            state.generator_group_output_generation = group_output_generation;
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
    let host_automation_changed = refresh_host_automation_targets(state, params);
    if host_automation_changed
        || state.host_automation_len != 0
        || oscillator_configs_dirty
        || filter_configs_dirty
        || group_outputs_dirty
    {
        state.generator_active_mask = state.generator_group_masks[..state.generator_group_count]
            .iter()
            .zip(&state.generator_group_outputs[..state.generator_group_count])
            .fold(0, |mask, (group, output)| {
                mask | if output.enabled { *group } else { 0 }
            });
        let (mut effective_oscillators, effective_filters, effective_groups) =
            host_automated_generator_configuration(state, params);
        for (index, oscillator) in effective_oscillators.iter_mut().enumerate() {
            oscillator.enabled &= state.generator_active_mask & (1_u32 << index) != 0;
        }
        oscillator_configs_dirty |= effective_oscillators != state.effective_generator_oscillators;
        state.effective_generator_oscillators = effective_oscillators;
        let previous_effective_filters = state.effective_generator_filters;
        for (index, (before, after)) in previous_effective_filters
            .into_iter()
            .zip(effective_filters)
            .enumerate()
        {
            if before != after {
                state
                    .retarget_filter_coefficients(index, after.coefficients(state.dsp_sample_rate));
            }
        }
        state.effective_generator_filters = effective_filters;
        state.effective_generator_group_outputs = effective_groups;
        state.synth.configure_output_groups(
            effective_groups.map(group_output_envelope),
            effective_groups.map(|output| output.receive_midi_channel),
            state.generator_group_count,
            state.generator_group_masks[..state.generator_group_count]
                .iter()
                .enumerate()
                .fold(0_u8, |mask, (group, oscillators)| {
                    let active = (*oscillators != 0 || state.generator_groups[group].has_aux())
                        && effective_groups[group].enabled;
                    mask | (u8::from(active) << group)
                }),
            effective_groups[..state.generator_group_count]
                .iter()
                .any(|output| output.envelope_enabled),
        );
    }
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
        params.xy_source_x_route_mask.load(),
        params.xy_source_y_route_mask.load(),
        &state.generator_module_ids,
        &state.generator_groups,
        state.generator_group_count,
        &state.generator_filter_module_ids,
        &state.generator_aux_module_ids,
        &state.generator_group_ids,
        &state.generator_group_masks,
        state.generator_group_count,
        state.generator_filter_mask,
        oscillator_enabled,
    );
    let audible_oscillators = state.generator_group_masks[..state.generator_group_count]
        .iter()
        .zip(&state.effective_generator_group_outputs[..state.generator_group_count])
        .fold(0, |mask, (group, output)| {
            mask | if output.enabled { *group } else { 0 }
        });
    state
        .synth
        .configure_audible_oscillators(audible_oscillators);
    state.generator_active_mask = audible_oscillators | active_routes.generator_source_mask;
    for oscillator in 0..generators::MAX_OSCILLATORS {
        let enabled = state.generator_oscillators[oscillator].enabled
            && state.generator_active_mask & (1 << oscillator) != 0;
        oscillator_configs_dirty |=
            state.effective_generator_oscillators[oscillator].enabled != enabled;
        state.effective_generator_oscillators[oscillator].enabled = enabled;
    }
    let active_group_mask = state.generator_group_masks[..state.generator_group_count]
        .iter()
        .enumerate()
        .fold(0_u8, |mask, (group, oscillators)| {
            let audible = state.effective_generator_group_outputs[group].enabled;
            let modulation_source = *oscillators & active_routes.generator_source_mask != 0;
            mask | (u8::from(audible || modulation_source) << group)
        });
    state.synth.configure_output_groups(
        state
            .effective_generator_group_outputs
            .map(group_output_envelope),
        state
            .effective_generator_group_outputs
            .map(|output| output.receive_midi_channel),
        state.generator_group_count,
        active_group_mask,
        state.effective_generator_group_outputs[..state.generator_group_count]
            .iter()
            .any(|output| output.envelope_enabled),
    );
    state.synth.configure_voice_filters(
        &state.effective_generator_filters,
        state.generator_filter_mask,
    );
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
        let mut configs = lfo_configuration(params);
        apply_host_automated_macro_pack_values(state, params, &mut configs);
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
                    && (compiled.is_positioned()
                        || match oscillator {
                            0 => params.osc1_custom_shape.value() > f32::EPSILON,
                            1 => params.osc2_custom_shape.value() > f32::EPSILON,
                            _ => params.osc3_custom_shape.value() > f32::EPSILON,
                        });
                state.va_table_transitions[oscillator].retarget(&compiled, audible);
            }
            state.va_tables[oscillator] = compiled;
        }
    }

    let mut unison_settings = unison_configurations(params, state);
    unison_settings[0] = unison_settings[0].with_phase_position(params.osc1_phase_position.value());
    unison_settings[1] = unison_settings[1].with_phase_position(params.osc2_phase_position.value());
    unison_settings[2] = unison_settings[2].with_phase_position(params.osc3_phase_position.value());
    for (index, settings) in unison_settings.iter_mut().enumerate() {
        if state.effective_generator_oscillators[index].engine
            == crate::generators::OscillatorEngineKind::Noise
        {
            *settings = UnisonSettings::new(1, 0.0, 0.0, 0.0, 0.0);
        }
    }
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
    state
        .synth
        .set_reference_tuning(params.global_tuning_hz.value());
    state.synth.set_transpose(
        params
            .octave_shift
            .value_f32()
            .mul_add(12.0, params.transpose.value_f32()),
    );
    state.mpe_bend_range = f32::from(params.mpe_bend_range.value_u8());
    state.pitch_bend_range = f32::from(params.pitch_bend_range.value_u8());
    state.pitch_bend_down_range = f32::from(params.pitch_bend_down_range.value_u8());

    state.synth.configure_oscillator_enabled(oscillator_enabled);
    oscillator_configs_dirty |= publish_resynth_rt(state, params) != 0;
    for (index, curve) in [
        &params.osc1_grain_curve_state,
        &params.osc2_grain_curve_state,
        &params.osc3_grain_curve_state,
    ]
    .into_iter()
    .enumerate()
    {
        if let Some((generation, compiled)) =
            curve.try_curve_rt_after(state.grain_curve_generations[index])
        {
            state.grain_curve_generations[index] = generation;
            state.synth.set_resynth_grain_curve(index, compiled);
        }
    }
    if oscillator_configs_dirty {
        let oscillators = std::array::from_fn(|index| {
            let config = state.effective_generator_oscillators[index];
            let table = state.va_tables[index].select(
                state.base_wave_curve,
                config.custom_shape,
                config.shape / 3.0,
            );
            OscillatorDspConfig {
                enabled: structural_render && config.enabled,
                engine: config.engine,
                resynth_playback: crate::voices::ResynthPlaybackPtr::NONE,
                shape: table.shape,
                pulse_width: config.pulse_width,
                custom_curve: table.curve,
                custom_mix: table.mix,
                positioned_wave: state.va_tables[index].is_positioned(),
                tuning_mode: config.tuning_mode,
                frequency_offset_hz: config.frequency_offset_hz,
                frequency_ratio: config.frequency_ratio,
                transpose: config.transpose,
                cents: config.cents,
                level: config.level,
                pan: config.pan,
                unison_voices: {
                    let noise = config.engine == crate::generators::OscillatorEngineKind::Noise;
                    let grain = config.engine.uses_sample_asset()
                        && state
                            .synth
                            .sounding_resynth_algorithm(index)
                            .is_none_or(|algorithm| {
                                algorithm == crate::oscillators::ResynthAlgorithm::Grain
                            });
                    if noise || grain {
                        1
                    } else {
                        config.unison_voices
                    }
                },
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
                phase_mod_source: 0,
                phase_mod_amount: 0.0,
                modulation_mode: config.modulation_mode,
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
    let generator_audio_modulation = active_routes.generator_source_mask != 0;
    let grouped_render = state.generator_group_count > 1
        || state.generator_has_filters
        || state.generator_has_aux
        || generator_audio_modulation
        || (structural_render && active_routes.modular_group_mask != 0);

    let mut next_event = 0;
    let mut block_start = 0;
    let mut peak_left = 0.0_f32;
    let mut peak_right = 0.0_f32;
    profile.enter(cpu_profile::Item::Render);
    while block_start < buffer.num_samples() {
        let block_len = (buffer.num_samples() - block_start).min(CONTROL_BLOCK);
        let static_gain = state.controls.read(
            params,
            block_len,
            oscillator_enabled,
            &active_routes,
            active_routes.source_mask,
        );
        let modulation_mask =
            state
                .controls
                .active_lfo_mask(&active_routes, block_len, &state.overflow_route_ramps);
        state
            .synth
            .sync_voice_lfos(&state.lfos, active_routes.source_mask);
        let polyphonic_source_mask = state.synth.voice_polyphonic_mask();
        let lfo_control_dynamic_mask =
            state
                .controls
                .lfo_control_dynamic_mask(modulation_mask, block_len, &lfo_configs);
        let (filter_modulation_mask, _, _) = active_routes.active_filter_modulation(
            &state.controls,
            block_len,
            &state.overflow_route_ramps,
        );
        state.set_filter_modulation_mask(filter_modulation_mask);
        state.generator_filter_modulation_stride = 1;
        let pitch_bend_static = slice_is_static(&state.controls.pitch_bend[..block_len]);
        state.lfos.set_active_mask(modulation_mask);
        state
            .lfos
            .set_modulation_mask(modulation_mask & !polyphonic_source_mask);
        state.fill_wave_curve_fades(block_len);
        let noise_mask = state.effective_generator_oscillators[..LEGACY_OSCILLATOR_COUNT]
            .iter()
            .enumerate()
            .fold(0, |mask, (oscillator, config)| {
                mask | (u32::from(config.engine == generators::OscillatorEngineKind::Noise)
                    << oscillator)
            });
        let direct_unison_pitch_mask = state
            .controls
            .unison_pitch_active_mask(block_len, &unison_settings)
            & !noise_mask;
        let direct_unison_motion_mask = state
            .controls
            .unison_motion_active_mask(block_len, &unison_settings)
            & !noise_mask;
        let voice_modulation_active =
            state.synth.voice_modulation_active() || modulation_mask & polyphonic_source_mask != 0;
        let route_modulation_active = state.lfos.is_active()
            || voice_modulation_active
            || active_routes.mod_wheel_active
            || active_routes.xy_x_active
            || active_routes.xy_y_active
            || generator_audio_modulation;
        let polyphonic_filter_only = voice_modulation_active
            && active_routes.filter_only_modulation()
            && modulation_mask & !polyphonic_source_mask == 0
            && !active_routes.mod_wheel_active;
        let block_morph_lfo = route_modulation_active
            && !voice_modulation_active
            && block_morph_modulation(&active_routes)
            && direct_unison_pitch_mask == 0
            && direct_unison_motion_mask == 0
            && !unison_settings
                .iter()
                .any(|settings| settings.motion_active());
        let block_pitch_lfo = route_modulation_active
            && !voice_modulation_active
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
            && !voice_modulation_active
            && block_motion_modulation(&active_routes)
            && direct_unison_pitch_mask == 0
            && direct_unison_motion_mask == 0;
        let block_structural_lfo = route_modulation_active
            && !voice_modulation_active
            && block_structural_oscillator_modulation(&active_routes)
            && direct_unison_pitch_mask == 0
            && direct_unison_motion_mask == 0;
        let block_grouped_lfo = route_modulation_active
            && !voice_modulation_active
            && block_grouped_modulation(&active_routes)
            && direct_unison_pitch_mask == 0
            && direct_unison_motion_mask == 0;
        let block_voice_structural_lfo = route_modulation_active
            && voice_modulation_active
            && block_structural_oscillator_filter_modulation(&active_routes)
            && direct_unison_pitch_mask == 0
            && direct_unison_motion_mask == 0;
        let block_phase_mod_lfo = (block_structural_lfo || block_voice_structural_lfo)
            && generator_audio_modulation
            && block_phase_mod_depth_modulation(&active_routes);
        let block_generator_voice_depth = generator_audio_modulation
            && voice_modulation_active
            && block_generator_voice_depth_modulation(&active_routes, polyphonic_source_mask);
        let mut generator_route_amount_target = None;
        let block_depth_routes =
            active_routes
                .depth_slice()
                .iter()
                .all(|route| match route.source {
                    ResolvedRouteSource::Generator(_) => active_routes
                        .modular_slice()
                        .iter()
                        .find(|child| child.route_index == route.target_route)
                        .is_some_and(|child| match child.source {
                            ResolvedRouteSource::Generator(_) => true,
                            ResolvedRouteSource::Rack(source) => {
                                let config = lfo_configs[usize::from(source)];
                                if config.constant_value.is_some() {
                                    true
                                } else if !config.envelope
                                    && !config.keytrack
                                    && config.mode == modulators::lfo::LfoMode::Sync
                                    && generator_route_amount_target
                                        .is_none_or(|target| target == child.route_index)
                                {
                                    generator_route_amount_target = Some(child.route_index);
                                    true
                                } else {
                                    false
                                }
                            }
                            ResolvedRouteSource::XyX => {
                                slice_is_static(&state.controls.xy_source_x[..block_len])
                            }
                            ResolvedRouteSource::XyY => {
                                slice_is_static(&state.controls.xy_source_y[..block_len])
                            }
                            ResolvedRouteSource::ModWheel => state.mod_wheel_ramp.remaining == 0,
                        }),
                    ResolvedRouteSource::Rack(source) => {
                        lfo_configs[usize::from(source)].constant_value.is_some()
                    }
                    ResolvedRouteSource::XyX => {
                        slice_is_static(&state.controls.xy_source_x[..block_len])
                    }
                    ResolvedRouteSource::XyY => {
                        slice_is_static(&state.controls.xy_source_y[..block_len])
                    }
                    ResolvedRouteSource::ModWheel => state.mod_wheel_ramp.remaining == 0,
                });
        let block_phase_depth_routes = if block_phase_mod_lfo {
            block_depth_routes
        } else {
            active_routes
                .depth_slice()
                .iter()
                .all(|route| match route.source {
                    ResolvedRouteSource::Generator(_) => true,
                    ResolvedRouteSource::Rack(source) => {
                        let config = lfo_configs[usize::from(source)];
                        if config.constant_value.is_some() {
                            true
                        } else if !config.envelope
                            && !config.keytrack
                            && config.mode == modulators::lfo::LfoMode::Sync
                            && generator_route_amount_target
                                .is_none_or(|target| target == route.target_route)
                        {
                            generator_route_amount_target = Some(route.target_route);
                            true
                        } else {
                            false
                        }
                    }
                    ResolvedRouteSource::XyX => {
                        slice_is_static(&state.controls.xy_source_x[..block_len])
                    }
                    ResolvedRouteSource::XyY => {
                        slice_is_static(&state.controls.xy_source_y[..block_len])
                    }
                    ResolvedRouteSource::ModWheel => state.mod_wheel_ramp.remaining == 0,
                })
        };
        if !block_phase_depth_routes {
            generator_route_amount_target = None;
        }
        let block_generator_gain = generator_audio_modulation
            && (!voice_modulation_active || block_generator_voice_depth)
            && (block_phase_depth_routes || block_generator_voice_depth)
            && block_generator_gain_modulation(&active_routes);
        let block_generator_phase = generator_audio_modulation
            && (!voice_modulation_active || block_generator_voice_depth)
            && (block_phase_depth_routes || block_generator_voice_depth)
            && block_generator_phase_modulation(&active_routes);
        let block_generator_pitch = generator_audio_modulation
            && (!voice_modulation_active || block_generator_voice_depth)
            && (block_phase_depth_routes || block_generator_voice_depth)
            && block_generator_pitch_modulation(&active_routes);
        let block_generator_graph = generator_audio_modulation
            && (!voice_modulation_active || block_generator_voice_depth)
            && (block_phase_depth_routes || block_generator_voice_depth)
            && block_generator_graph_modulation(&active_routes);
        let block_generator = block_generator_gain
            || block_generator_phase
            || block_generator_pitch
            || block_generator_graph;

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
                    state.synth.parameter_pitch_bend_asymmetric(
                        pitch_bend,
                        state.pitch_bend_down_range,
                        state.pitch_bend_range,
                    );
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
            if !state.synth.is_active()
                && state.decimator_tail == 0
                && state.generator_audio_input_group_mask == 0
            {
                state.settle_filter_coefficients_for_silence();
                state
                    .lfos
                    .advance_silent(usize::from(state.oversampler.factor()));
                if active_routes.mod_wheel_active {
                    state.mod_wheel_ramp.finish();
                }
                for channel in 0..output_channels {
                    buffer.output(channel)[sample_index] = 0.0;
                }
                profile.count(cpu_profile::Item::RouteSerial, 1);
                if state.synth.declicking() {
                    profile.count(cpu_profile::Item::RouteDeclickGated, 1);
                }
                offset += 1;
                continue;
            }
            if state.generator_filter_modulation_mask == 0 {
                state.generator_filters_were_silent = false;
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
                    let wave_position = match oscillator {
                        0 => state.controls.shape[offset] / 3.0,
                        1 => state.controls.osc2_shape[offset] / 3.0,
                        _ => state.controls.osc3_shape[offset] / 3.0,
                    };
                    state.va_table_transitions[oscillator].select(
                        state.wave_curves[oscillator].value(curve_fade),
                        position,
                        wave_position,
                    )
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
            .with_fast_audio_rate_modulation(params.fast_audio_rate_modulation())
            .with_oscillators([
                OscillatorSettings::new(
                    oscillator_enabled[0],
                    table_selections[0].2,
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
                .with_custom_curve(table_selections[0].0, table_selections[0].1)
                .with_positioned_wave(state.va_tables[0].is_positioned()),
                OscillatorSettings::new(
                    oscillator_enabled[1],
                    table_selections[1].2,
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
                .with_custom_curve(table_selections[1].0, table_selections[1].1)
                .with_positioned_wave(state.va_tables[1].is_positioned()),
                OscillatorSettings::new(
                    oscillator_enabled[2],
                    table_selections[2].2,
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
                .with_custom_curve(table_selections[2].0, table_selections[2].1)
                .with_positioned_wave(state.va_tables[2].is_positioned()),
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
            let structural_lfo_block = chunks != 0
                && (block_structural_lfo || block_voice_structural_lfo)
                && state
                    .controls
                    .is_static(offset, host_frames, oscillator_enabled)
                && (!block_voice_structural_lfo
                    || (lfo_control_dynamic_mask == 0
                        && !active_routes.mod_wheel_active
                        && active_routes.amounts_static(
                            &state.controls,
                            offset,
                            host_frames,
                            &state.overflow_route_ramps,
                        )))
                && if block_voice_structural_lfo {
                    state
                        .synth
                        .voice_structural_modulation_block_eligible(settings)
                } else {
                    state.synth.structural_modulation_block_eligible(settings)
                };
            if structural_lfo_block && state.block_major_enabled() && !grouped_render {
                let gain =
                    static_gain.unwrap_or_else(|| db_to_linear(state.controls.output_db[offset]));
                let (block_peak_left, block_peak_right) = match block_samples {
                    Some(FACTOR3_BLOCK_INTERNAL_SAMPLES) => {
                        render_structural_oscillator_job::<FACTOR3_BLOCK_INTERNAL_SAMPLES>(
                            state,
                            buffer,
                            output_channels,
                            sample_index,
                            offset,
                            chunks,
                            settings,
                            envelope,
                            gain,
                            &active_routes,
                            lfo_control_dynamic_mask,
                            &mut modulation,
                            &mut structural_modulation,
                            block_voice_structural_lfo,
                        )
                    }
                    Some(BLOCK_INTERNAL_SAMPLES) => {
                        render_structural_oscillator_job::<BLOCK_INTERNAL_SAMPLES>(
                            state,
                            buffer,
                            output_channels,
                            sample_index,
                            offset,
                            chunks,
                            settings,
                            envelope,
                            gain,
                            &active_routes,
                            lfo_control_dynamic_mask,
                            &mut modulation,
                            &mut structural_modulation,
                            block_voice_structural_lfo,
                        )
                    }
                    _ => unreachable!(),
                };
                peak_left = peak_left.max(block_peak_left);
                peak_right = peak_right.max(block_peak_right);
                state.decimator_tail = oversampling::TAIL_SAMPLES;
                #[cfg(test)]
                {
                    state.block_major_chunks += chunks;
                }
                profile.count(cpu_profile::Item::RouteBlockMajor, host_frames as u32);
                offset += host_frames;
                continue;
            }
            let lfo_morph_block = chunks != 0
                && block_morph_lfo
                && state
                    .controls
                    .is_static_except_shape(offset, host_frames, oscillator_enabled)
                && state.synth.morph_block_eligible(settings);
            let lfo_pitch_block = chunks != 0
                && block_pitch_lfo
                && state
                    .controls
                    .is_static(offset, host_frames, oscillator_enabled)
                && state.synth.pitch_block_eligible(settings)
                && state.synth.spatial_block_eligible();
            if lfo_pitch_block && state.block_major_enabled() && !grouped_render {
                let gain =
                    static_gain.unwrap_or_else(|| db_to_linear(state.controls.output_db[offset]));
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
                profile.count(cpu_profile::Item::RouteBlockMajor, host_frames as u32);
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
                let gain =
                    static_gain.unwrap_or_else(|| db_to_linear(state.controls.output_db[offset]));
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
                profile.count(cpu_profile::Item::RouteBlockMajor, host_frames as u32);
                offset += host_frames;
                continue;
            }
            let lfo_control_block = chunks != 0
                && block_control_lfo
                && (!voice_modulation_active || lfo_control_dynamic_mask == 0)
                && active_routes.amounts_static(
                    &state.controls,
                    offset,
                    host_frames,
                    &state.overflow_route_ramps,
                )
                && !lfo_morph_block
                && !lfo_pitch_block
                && state
                    .controls
                    .is_static(offset, host_frames, oscillator_enabled)
                && state.synth.control_block_eligible();
            if lfo_control_block && state.block_major_enabled() && !grouped_render {
                let gain =
                    static_gain.unwrap_or_else(|| db_to_linear(state.controls.output_db[offset]));
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
                profile.count(cpu_profile::Item::RouteBlockMajor, host_frames as u32);
                offset += host_frames;
                continue;
            }
            let terminal_filter_block = chunks != 0
                && state.block_major_enabled()
                && state.generator_group_count == 1
                && state.generator_has_filters
                && !state.generator_has_aux
                && !generator_audio_modulation
                && state.generator_filter_smoothing_mask == 0
                && !route_modulation_active
                && direct_unison_pitch_mask == 0
                && !voice_modulation_active
                && !morphing
                && !state.generator_groups[0]
                    .has_ratio_brickwall(&state.effective_generator_filters)
                && state
                    .controls
                    .is_static(offset, host_frames, oscillator_enabled)
                && state.synth.terminal_filter_block_eligible(
                    settings,
                    envelope,
                    &state.generator_groups[0],
                );
            let terminal_filter_voice_modulation_block = chunks != 0
                && state.block_major_enabled()
                && state.generator_group_count == 1
                && state.generator_has_filters
                && !state.generator_has_aux
                && !generator_audio_modulation
                && state.generator_filter_smoothing_mask == 0
                && polyphonic_filter_only
                && direct_unison_pitch_mask == 0
                && direct_unison_motion_mask == 0
                && lfo_control_dynamic_mask == 0
                && !morphing
                && !state.generator_groups[0]
                    .has_ratio_brickwall(&state.effective_generator_filters)
                && active_routes.amounts_static(
                    &state.controls,
                    offset,
                    host_frames,
                    &state.overflow_route_ramps,
                )
                && state
                    .controls
                    .is_static(offset, host_frames, oscillator_enabled)
                && state.synth.terminal_filter_block_eligible(
                    settings,
                    envelope,
                    &state.generator_groups[0],
                );
            let filter_modulated = filter_modulation_mask != 0
                && active_routes.filter_only_modulation()
                && !voice_modulation_active;
            let grouped_modulation_block = chunks != 0
                && state.block_major_enabled()
                && grouped_render
                && block_grouped_lfo
                && !state.generator_has_aux
                && !generator_audio_modulation
                && !morphing
                && active_routes.amounts_static(
                    &state.controls,
                    offset,
                    host_frames,
                    &state.overflow_route_ramps,
                )
                && state
                    .controls
                    .is_static(offset, host_frames, oscillator_enabled)
                && state.synth.grouped_block_eligible(settings);
            if grouped_modulation_block {
                let gain =
                    static_gain.unwrap_or_else(|| db_to_linear(state.controls.output_db[offset]));
                let (block_peak_left, block_peak_right) = match block_samples {
                    Some(FACTOR3_BLOCK_INTERNAL_SAMPLES) => {
                        fill_grouped_modulation_block::<FACTOR3_BLOCK_INTERNAL_SAMPLES>(
                            state,
                            offset,
                            chunks,
                            &active_routes,
                            lfo_control_dynamic_mask,
                            &mut modulation,
                            &mut structural_modulation,
                        );
                        render_grouped_host_block::<FACTOR3_BLOCK_INTERNAL_SAMPLES>(
                            state,
                            buffer,
                            output_channels,
                            sample_index,
                            chunks,
                            settings,
                            envelope,
                            gain,
                            &structural_modulation,
                            true,
                            true,
                            false,
                            None,
                            filter_modulation_mask != 0,
                        )
                    }
                    Some(BLOCK_INTERNAL_SAMPLES) => {
                        fill_grouped_modulation_block::<BLOCK_INTERNAL_SAMPLES>(
                            state,
                            offset,
                            chunks,
                            &active_routes,
                            lfo_control_dynamic_mask,
                            &mut modulation,
                            &mut structural_modulation,
                        );
                        render_grouped_host_block::<BLOCK_INTERNAL_SAMPLES>(
                            state,
                            buffer,
                            output_channels,
                            sample_index,
                            chunks,
                            settings,
                            envelope,
                            gain,
                            &structural_modulation,
                            true,
                            true,
                            false,
                            None,
                            filter_modulation_mask != 0,
                        )
                    }
                    _ => unreachable!(),
                };
                peak_left = peak_left.max(block_peak_left);
                peak_right = peak_right.max(block_peak_right);
                state.decimator_tail = oversampling::TAIL_SAMPLES;
                profile.count(cpu_profile::Item::RouteBlockMajor, host_frames as u32);
                offset += host_frames;
                continue;
            }
            let dynamic_filter_block = chunks != 0
                && state.block_major_enabled()
                && grouped_render
                && state.generator_has_filters
                && !state.generator_has_aux
                && !generator_audio_modulation
                && (filter_modulated
                    || (state.generator_filter_smoothing_mask != 0 && !route_modulation_active))
                && direct_unison_pitch_mask == 0
                && direct_unison_motion_mask == 0
                && !morphing
                && state
                    .controls
                    .is_static(offset, host_frames, oscillator_enabled)
                && state.synth.dynamic_grouped_block_eligible(settings);
            if dynamic_filter_block {
                let gain =
                    static_gain.unwrap_or_else(|| db_to_linear(state.controls.output_db[offset]));
                let (block_peak_left, block_peak_right) = match block_samples {
                    Some(FACTOR3_BLOCK_INTERNAL_SAMPLES) => {
                        fill_filter_coefficients_block::<FACTOR3_BLOCK_INTERNAL_SAMPLES>(
                            state,
                            offset,
                            chunks,
                            &active_routes,
                            lfo_control_dynamic_mask,
                            &mut modulation,
                            &mut structural_modulation,
                            filter_modulated,
                        );
                        render_grouped_host_block::<FACTOR3_BLOCK_INTERNAL_SAMPLES>(
                            state,
                            buffer,
                            output_channels,
                            sample_index,
                            chunks,
                            settings,
                            envelope,
                            gain,
                            &structural_modulation,
                            false,
                            false,
                            false,
                            None,
                            true,
                        )
                    }
                    Some(BLOCK_INTERNAL_SAMPLES) => {
                        fill_filter_coefficients_block::<BLOCK_INTERNAL_SAMPLES>(
                            state,
                            offset,
                            chunks,
                            &active_routes,
                            lfo_control_dynamic_mask,
                            &mut modulation,
                            &mut structural_modulation,
                            filter_modulated,
                        );
                        render_grouped_host_block::<BLOCK_INTERNAL_SAMPLES>(
                            state,
                            buffer,
                            output_channels,
                            sample_index,
                            chunks,
                            settings,
                            envelope,
                            gain,
                            &structural_modulation,
                            false,
                            false,
                            false,
                            None,
                            true,
                        )
                    }
                    _ => unreachable!(),
                };
                peak_left = peak_left.max(block_peak_left);
                peak_right = peak_right.max(block_peak_right);
                if !filter_modulated {
                    state
                        .lfos
                        .advance_silent(host_frames * usize::from(oversampling_factor));
                }
                state.decimator_tail = oversampling::TAIL_SAMPLES;
                profile.count(cpu_profile::Item::RouteBlockMajor, host_frames as u32);
                offset += host_frames;
                continue;
            }
            if terminal_filter_voice_modulation_block {
                let gain =
                    static_gain.unwrap_or_else(|| db_to_linear(state.controls.output_db[offset]));
                let (block_peak_left, block_peak_right) = match block_samples {
                    Some(FACTOR3_BLOCK_INTERNAL_SAMPLES) => {
                        render_terminal_filter_voice_modulated_host_block::<
                            FACTOR3_BLOCK_INTERNAL_SAMPLES,
                        >(
                            state,
                            buffer,
                            output_channels,
                            sample_index,
                            offset,
                            chunks,
                            settings,
                            envelope,
                            gain,
                            &active_routes,
                            &mut modulation,
                            &mut structural_modulation,
                        )
                    }
                    Some(BLOCK_INTERNAL_SAMPLES) => {
                        render_terminal_filter_voice_modulated_host_block::<BLOCK_INTERNAL_SAMPLES>(
                            state,
                            buffer,
                            output_channels,
                            sample_index,
                            offset,
                            chunks,
                            settings,
                            envelope,
                            gain,
                            &active_routes,
                            &mut modulation,
                            &mut structural_modulation,
                        )
                    }
                    _ => unreachable!(),
                };
                peak_left = peak_left.max(block_peak_left);
                peak_right = peak_right.max(block_peak_right);
                state.decimator_tail = oversampling::TAIL_SAMPLES;
                profile.count(cpu_profile::Item::RouteBlockMajor, host_frames as u32);
                offset += host_frames;
                continue;
            }
            if terminal_filter_block {
                let gain =
                    static_gain.unwrap_or_else(|| db_to_linear(state.controls.output_db[offset]));
                let (block_peak_left, block_peak_right) = match block_samples {
                    Some(FACTOR3_BLOCK_INTERNAL_SAMPLES) => {
                        render_terminal_filter_host_block::<FACTOR3_BLOCK_INTERNAL_SAMPLES>(
                            state,
                            buffer,
                            output_channels,
                            sample_index,
                            chunks,
                            settings,
                            envelope,
                            gain,
                        )
                    }
                    Some(BLOCK_INTERNAL_SAMPLES) => {
                        render_terminal_filter_host_block::<BLOCK_INTERNAL_SAMPLES>(
                            state,
                            buffer,
                            output_channels,
                            sample_index,
                            chunks,
                            settings,
                            envelope,
                            gain,
                        )
                    }
                    _ => unreachable!(),
                };
                peak_left = peak_left.max(block_peak_left);
                peak_right = peak_right.max(block_peak_right);
                state
                    .lfos
                    .advance_silent(host_frames * usize::from(oversampling_factor));
                state.decimator_tail = oversampling::TAIL_SAMPLES;
                profile.count(cpu_profile::Item::RouteBlockMajor, host_frames as u32);
                offset += host_frames;
                continue;
            }
            let grouped_oscillator_block = chunks != 0
                && state.block_major_enabled()
                && grouped_render
                && (!generator_audio_modulation || block_generator)
                && (!state.generator_has_aux
                    || !generator_audio_modulation
                    || (block_generator_gain && state.synth.generator_aux_routes_active()))
                && (!state.generator_has_filters
                    || (block_generator
                        && state.generator_filter_smoothing_mask == 0
                        && (!voice_modulation_active || block_generator_voice_depth))
                    || (!generator_audio_modulation
                        && state.generator_filter_smoothing_mask == 0
                        && (filter_modulation_mask == 0 || block_voice_structural_lfo)
                        && (!route_modulation_active
                            || block_structural_lfo
                            || block_voice_structural_lfo)))
                && (!route_modulation_active
                    || block_structural_lfo
                    || block_voice_structural_lfo
                    || block_generator)
                && (!(block_structural_lfo || block_voice_structural_lfo || block_generator)
                    || active_routes.amounts_static(
                        &state.controls,
                        offset,
                        host_frames,
                        &state.overflow_route_ramps,
                    ))
                && direct_unison_pitch_mask == 0
                && (!voice_modulation_active
                    || block_voice_structural_lfo
                    || block_phase_mod_lfo
                    || block_generator_voice_depth)
                && !morphing
                && state
                    .controls
                    .is_static(offset, host_frames, oscillator_enabled)
                && if generator_audio_modulation || block_structural_lfo {
                    state.synth.grouped_block_eligible(settings)
                } else {
                    state.synth.dynamic_grouped_block_eligible(settings)
                };
            if grouped_oscillator_block {
                if block_generator && generator_route_amount_target.is_none() {
                    advance_lfo_modulation(
                        state,
                        &active_routes,
                        0,
                        lfo_control_dynamic_mask,
                        offset,
                        &mut modulation,
                        Some(&mut structural_modulation),
                    );
                }
                let gain =
                    static_gain.unwrap_or_else(|| db_to_linear(state.controls.output_db[offset]));
                let (block_peak_left, block_peak_right) = match block_samples {
                    Some(FACTOR3_BLOCK_INTERNAL_SAMPLES) => {
                        if block_structural_lfo || block_voice_structural_lfo {
                            for chunk in 0..chunks {
                                fill_structural_oscillator_block::<FACTOR3_BLOCK_INTERNAL_SAMPLES>(
                                    state,
                                    &active_routes,
                                    lfo_control_dynamic_mask,
                                    offset + chunk * base_host_frames,
                                    usize::from(oversampling_factor),
                                    chunk * block_internal,
                                    &mut modulation,
                                    &mut structural_modulation,
                                );
                            }
                        } else if let Some(target_route) = generator_route_amount_target {
                            for chunk in 0..chunks {
                                fill_generator_route_amount_block::<FACTOR3_BLOCK_INTERNAL_SAMPLES>(
                                    state,
                                    &active_routes,
                                    target_route,
                                    lfo_control_dynamic_mask,
                                    offset + chunk * base_host_frames,
                                    usize::from(oversampling_factor),
                                    chunk * block_internal,
                                    &mut modulation,
                                    &mut structural_modulation,
                                );
                            }
                        }
                        render_grouped_host_block::<FACTOR3_BLOCK_INTERNAL_SAMPLES>(
                            state,
                            buffer,
                            output_channels,
                            sample_index,
                            chunks,
                            settings,
                            envelope,
                            gain,
                            &structural_modulation,
                            false,
                            block_structural_lfo,
                            block_voice_structural_lfo
                                || block_phase_mod_lfo && voice_modulation_active
                                || block_generator_voice_depth,
                            generator_route_amount_target,
                            false,
                        )
                    }
                    Some(BLOCK_INTERNAL_SAMPLES) => {
                        if block_structural_lfo || block_voice_structural_lfo {
                            for chunk in 0..chunks {
                                fill_structural_oscillator_block::<BLOCK_INTERNAL_SAMPLES>(
                                    state,
                                    &active_routes,
                                    lfo_control_dynamic_mask,
                                    offset + chunk * base_host_frames,
                                    usize::from(oversampling_factor),
                                    chunk * block_internal,
                                    &mut modulation,
                                    &mut structural_modulation,
                                );
                            }
                        } else if let Some(target_route) = generator_route_amount_target {
                            for chunk in 0..chunks {
                                fill_generator_route_amount_block::<BLOCK_INTERNAL_SAMPLES>(
                                    state,
                                    &active_routes,
                                    target_route,
                                    lfo_control_dynamic_mask,
                                    offset + chunk * base_host_frames,
                                    usize::from(oversampling_factor),
                                    chunk * block_internal,
                                    &mut modulation,
                                    &mut structural_modulation,
                                );
                            }
                        }
                        render_grouped_host_block::<BLOCK_INTERNAL_SAMPLES>(
                            state,
                            buffer,
                            output_channels,
                            sample_index,
                            chunks,
                            settings,
                            envelope,
                            gain,
                            &structural_modulation,
                            false,
                            block_structural_lfo,
                            block_voice_structural_lfo
                                || block_phase_mod_lfo && voice_modulation_active
                                || block_generator_voice_depth,
                            generator_route_amount_target,
                            false,
                        )
                    }
                    _ => unreachable!(),
                };
                peak_left = peak_left.max(block_peak_left);
                peak_right = peak_right.max(block_peak_right);
                if !block_structural_lfo
                    && !block_voice_structural_lfo
                    && generator_route_amount_target.is_none()
                {
                    state
                        .lfos
                        .advance_silent(host_frames * usize::from(oversampling_factor));
                }
                state.decimator_tail = oversampling::TAIL_SAMPLES;
                #[cfg(test)]
                {
                    state.block_major_chunks += chunks;
                }
                profile.count(cpu_profile::Item::RouteBlockMajor, host_frames as u32);
                offset += host_frames;
                continue;
            }
            if chunks != 0
                && state.block_major_enabled()
                && !grouped_render
                && (!route_modulation_active || lfo_morph_block)
            {
                let gain =
                    static_gain.unwrap_or_else(|| db_to_linear(state.controls.output_db[offset]));
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
                profile.count(cpu_profile::Item::RouteBlockMajor, host_frames as u32);
                offset += host_frames;
                continue;
            }
            let source_was_active = state.synth.is_active();
            let input_current = host_input_frame(buffer, sample_index);
            let modulation_active = route_modulation_active || direct_unison_pitch_mask != 0;
            if !modulation_active {
                clear_modulation_frame(&mut modulation, &active_routes, direct_unison_pitch_mask);
            }
            let mut grouped_stems = [(0.0_f32, 0.0_f32); generators::MAX_OUTPUT_PAIRS];
            let (mut left, mut right) = if grouped_render {
                if state.oversampler.factor() == 1 {
                    if state.generator_has_filters {
                        state.advance_filter_coefficients();
                    }
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
                    state.update_filter_modulation(
                        modulation_active.then_some(&structural_modulation),
                    );
                    if state.generator_filter_mask != 0 {
                        state.synth.configure_voice_filters(
                            &state.generator_filter_modulated_configs,
                            state.generator_filter_mask,
                        );
                    }
                    let render_envelope = if active_routes.global_mask & GLOBAL_ENVELOPE_MASK != 0 {
                        modulated_envelope(envelope, modulation.global)
                    } else {
                        envelope
                    };
                    let structural_control =
                        structural_oscillator_frame_control(state, &structural_modulation);
                    let mut rendered = if modulation_active {
                        state
                            .synth
                            .render_grouped_with_modulation_and_structural_frame(
                                settings,
                                render_envelope,
                                modulation.unison,
                                &structural_control,
                                &structural_modulation,
                                &state.effective_generator_group_outputs
                                    [..state.generator_group_count],
                                &state.generator_oscillator_groups,
                                state.generator_group_count,
                                &state.generator_groups[..state.generator_group_count],
                                &state.generator_filter_coefficients,
                                state.generator_has_filters
                                    || state.generator_has_aux
                                    || generator_audio_modulation,
                            )
                    } else {
                        state.synth.render_grouped_neutral(
                            settings,
                            render_envelope,
                            &state.generator_oscillator_groups,
                            state.generator_group_count,
                            &state.generator_groups[..state.generator_group_count],
                            &state.generator_filter_coefficients,
                            state.generator_has_filters
                                || state.generator_has_aux
                                || generator_audio_modulation,
                        )
                    };
                    render_global_aux_input(state, input_current, &mut rendered);
                    for group in 0..state.generator_group_count {
                        if state.generator_group_masks[group] == 0
                            && !state.generator_groups[group].has_aux()
                            || !state.effective_generator_group_outputs[group].enabled
                        {
                            continue;
                        }
                        grouped_stems[group] = state.group_oversamplers[group]
                            .process_direct(rendered[group].0, rendered[group].1);
                    }
                } else {
                    let reuse_direct_modulation = modulation_active
                        && !route_modulation_active
                        && active_routes.unison_layout_mask == 0;
                    for internal_sample in 0..usize::from(state.oversampler.factor()) {
                        if state.generator_has_filters {
                            state.advance_filter_coefficients();
                        }
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
                        state.update_filter_modulation(
                            modulation_active.then_some(&structural_modulation),
                        );
                        if state.generator_filter_mask != 0 {
                            state.synth.configure_voice_filters(
                                &state.generator_filter_modulated_configs,
                                state.generator_filter_mask,
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
                        let mut rendered = if modulation_active {
                            state
                                .synth
                                .render_grouped_with_modulation_and_structural_frame(
                                    render_settings,
                                    render_envelope,
                                    modulation.unison,
                                    &structural_control,
                                    &structural_modulation,
                                    &state.effective_generator_group_outputs
                                        [..state.generator_group_count],
                                    &state.generator_oscillator_groups,
                                    state.generator_group_count,
                                    &state.generator_groups[..state.generator_group_count],
                                    &state.generator_filter_coefficients,
                                    state.generator_has_filters
                                        || state.generator_has_aux
                                        || generator_audio_modulation,
                                )
                        } else {
                            state.synth.render_grouped_neutral(
                                render_settings,
                                render_envelope,
                                &state.generator_oscillator_groups,
                                state.generator_group_count,
                                &state.generator_groups[..state.generator_group_count],
                                &state.generator_filter_coefficients,
                                state.generator_has_filters
                                    || state.generator_has_aux
                                    || generator_audio_modulation,
                            )
                        };
                        let interpolation =
                            (internal_sample + 1) as f32 / f32::from(state.oversampler.factor());
                        let input = (
                            (input_current.0 - state.input_previous.0)
                                .mul_add(interpolation, state.input_previous.0),
                            (input_current.1 - state.input_previous.1)
                                .mul_add(interpolation, state.input_previous.1),
                        );
                        render_global_aux_input(state, input, &mut rendered);
                        for group in 0..state.generator_group_count {
                            if state.generator_group_masks[group] == 0
                                && !state.generator_groups[group].has_aux()
                                || !state.effective_generator_group_outputs[group].enabled
                            {
                                continue;
                            }
                            state.group_oversamplers[group]
                                .push(rendered[group].0, rendered[group].1);
                        }
                    }
                    for (group, output) in grouped_stems[..state.generator_group_count]
                        .iter_mut()
                        .enumerate()
                    {
                        if state.generator_group_masks[group] != 0
                            || state.generator_groups[group].has_aux()
                        {
                            *output = state.group_oversamplers[group].output();
                        }
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
                    let render_envelope = if active_routes.global_mask & GLOBAL_ENVELOPE_MASK != 0 {
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
            state.input_previous = input_current;
            if !modulation_active {
                state
                    .lfos
                    .advance_silent(usize::from(state.oversampler.factor()));
            }

            let input_active = input_current.0.abs().max(input_current.1.abs()) > f32::EPSILON
                || state
                    .global_aux_group_taps
                    .iter()
                    .any(|tap| tap.0.abs().max(tap.1.abs()) > f32::EPSILON);
            if source_was_active || state.synth.is_active() || input_active {
                state.decimator_tail = oversampling::TAIL_SAMPLES;
            } else {
                state.decimator_tail = state.decimator_tail.saturating_sub(1);
                if state.set_oversampling(requested_factor) {
                    antialiasing = requested_antialiasing.for_factor(state.oversampler.factor());
                }
            }

            let gain = if active_routes.global_mask & GLOBAL_OUTPUT_MASK != 0 {
                static_gain.map_or_else(
                    || db_to_linear(state.controls.output_db[offset] + modulation.global.output_db),
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
                    if modulation_active {
                        state.synth.voice_group_modulation_mask()
                    } else {
                        0
                    },
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
            profile.count(cpu_profile::Item::RouteSerial, 1);
            if state.synth.declicking() {
                profile.count(cpu_profile::Item::RouteDeclickGated, 1);
            }
            offset += 1;
        }
        block_start += block_len;
    }

    profile.enter(cpu_profile::Item::Metering);
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
    publish_resynth_telemetry(state, params, buffer.num_samples());
    profile.finish(state.synth.active_voice_count());

    current_process_status(state)
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
