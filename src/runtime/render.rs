use crate::*;

#[allow(clippy::too_many_arguments)]
fn write_host_block<G>(
    oversampler: &mut StereoOversampler,
    samples: &[(f32, f32)],
    factor: usize,
    host_frames: usize,
    mut apply_gain: G,
    buffer: &mut AudioBuffer,
    output_channels: usize,
    sample_index: usize,
    terminal: Option<(usize, bool)>,
) -> (f32, f32)
where
    G: FnMut(usize, f32, f32) -> (f32, f32),
{
    let mut peak_left = 0.0_f32;
    let mut peak_right = 0.0_f32;
    for frame in 0..host_frames {
        let (left, right) = if factor == 1 {
            let (left, right) = samples[frame];
            oversampler.process_direct(left, right)
        } else {
            for (left, right) in samples[frame * factor..(frame + 1) * factor]
                .iter()
                .copied()
            {
                oversampler.push(left, right);
            }
            oversampler.output()
        };
        let (left, right) = apply_gain(frame, left, right);
        if terminal.is_none_or(|(_, routed)| routed) {
            peak_left = peak_left.max(left.abs());
            peak_right = peak_right.max(right.abs());
        }
        let output_index = sample_index + frame;
        if let Some((target, routed)) = terminal {
            for channel in 0..output_channels {
                buffer.output(channel)[output_index] = if routed && channel == target {
                    left
                } else if routed && channel == target + 1 {
                    right
                } else {
                    0.0
                };
            }
        } else if output_channels == 1 {
            buffer.output(0)[output_index] = (left + right) * 0.5;
        } else {
            buffer.output(0)[output_index] = left;
            buffer.output(1)[output_index] = right;
        }
    }
    (peak_left, peak_right)
}

pub(crate) fn render_saw_host_block<const SAMPLES: usize>(
    state: &mut KurvDspState,
    buffer: &mut AudioBuffer,
    output_channels: usize,
    sample_index: usize,
    chunks: usize,
    settings: VoiceSettings,
    envelope: EnvelopeSettings,
    gain: f32,
    shapes: Option<&[[f32; MAX_JOB_SAMPLES]; LEGACY_OSCILLATOR_COUNT]>,
    output_gains: Option<&[f32; MAX_JOB_SAMPLES]>,
) -> (f32, f32) {
    let factor = usize::from(state.oversampler.factor());
    debug_assert_eq!(SAMPLES % factor, 0);
    let internal_samples = SAMPLES * chunks;
    debug_assert!(internal_samples <= MAX_JOB_SAMPLES);
    let mut samples = [(0.0_f32, 0.0_f32); MAX_JOB_SAMPLES];
    let full_coarse_job = chunks == MAX_JOB_SAMPLES / SAMPLES;
    let generic_shape = !state.synth.exact_saw_banks_eligible(settings);
    // Event-split tails must stay serial. Only a complete coarse window has
    // enough work to justify publishing a generic shadow job.
    let worthwhile_generic_job = generic_shape && full_coarse_job && internal_samples >= 128;
    let pooled = ((full_coarse_job || worthwhile_generic_job) && state.internal_pool_enabled())
        .then(|| match shapes {
            Some(shapes) => state.internal_pool.render_morph_job::<SAMPLES>(
                &mut state.synth,
                settings,
                envelope,
                chunks,
                shapes,
            ),
            None => state.internal_pool.render_block_job::<SAMPLES>(
                &mut state.synth,
                settings,
                envelope,
                chunks,
            ),
        })
        .flatten();
    #[cfg(test)]
    {
        if pooled.is_some() {
            state.internal_pool_coarse_jobs += 1;
        } else if state.internal_pool_enabled() && !full_coarse_job {
            state.internal_pool_partial_serial_jobs += 1;
        }
    }
    if let Some(block) = pooled {
        debug_assert_eq!(block.len, internal_samples);
        samples = block.samples;
    } else {
        for chunk in 0..chunks {
            let rendered = if let Some(shapes) = shapes {
                let offset = chunk * SAMPLES;
                let shapes = std::array::from_fn(|oscillator| {
                    std::array::from_fn(|frame| shapes[oscillator][offset + frame])
                });
                state
                    .synth
                    .render_morph_block::<SAMPLES>(settings, envelope, &shapes)
            } else {
                state.synth.render_block::<SAMPLES>(settings, envelope)
            };
            samples[chunk * SAMPLES..(chunk + 1) * SAMPLES].copy_from_slice(&rendered);
        }
    }
    write_host_block(
        &mut state.oversampler,
        &samples,
        factor,
        internal_samples / factor,
        |frame, left, right| {
            let output_gain = output_gains.map_or(1.0, |gains| gains[frame]);
            (left * gain * output_gain, right * gain * output_gain)
        },
        buffer,
        output_channels,
        sample_index,
        None,
    )
}

pub(crate) fn route_group_frame(
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

pub(crate) fn render_grouped_host_block<const SAMPLES: usize>(
    state: &mut KurvDspState,
    buffer: &mut AudioBuffer,
    output_channels: usize,
    sample_index: usize,
    chunks: usize,
    settings: VoiceSettings,
    envelope: EnvelopeSettings,
    gain: f32,
    structural: &StructuralModulationFrame,
) -> (f32, f32) {
    let factor = usize::from(state.oversampler.factor());
    debug_assert_eq!(SAMPLES % factor, 0);
    debug_assert!(SAMPLES * chunks <= MAX_JOB_SAMPLES);
    let host_frames = SAMPLES / factor;
    let group_count = state
        .generator_group_count
        .clamp(1, generators::MAX_OUTPUT_PAIRS);
    let mut peak_left = 0.0_f32;
    let mut peak_right = 0.0_f32;
    for chunk in 0..chunks {
        let stems = state.synth.render_grouped_block::<SAMPLES>(
            settings,
            envelope,
            &state.generator_oscillator_groups,
            group_count,
        );
        for host_frame in 0..host_frames {
            let mut frame_stems = [(0.0_f32, 0.0_f32); generators::MAX_OUTPUT_PAIRS];
            let base = host_frame * factor;
            for group in 0..group_count {
                if state.generator_group_masks[group] == 0 {
                    continue;
                }
                let (left, right) = if factor == 1 {
                    state.group_oversamplers[group]
                        .process_direct(stems[group][base].0, stems[group][base].1)
                } else {
                    for sample in &stems[group][base..base + factor] {
                        state.group_oversamplers[group].push(sample.0, sample.1);
                    }
                    state.group_oversamplers[group].output()
                };
                frame_stems[group] = (left * gain, right * gain);
            }
            let (frame_peak_left, frame_peak_right) = route_group_frame(
                buffer,
                sample_index + chunk * host_frames + host_frame,
                &frame_stems[..group_count],
                &state.effective_generator_group_outputs[..group_count],
                structural,
                output_channels,
            );
            peak_left = peak_left.max(frame_peak_left);
            peak_right = peak_right.max(frame_peak_right);
        }
    }
    (peak_left, peak_right)
}

pub(crate) fn render_terminal_filter_host_block<const SAMPLES: usize>(
    state: &mut KurvDspState,
    buffer: &mut AudioBuffer,
    output_channels: usize,
    sample_index: usize,
    chunks: usize,
    settings: VoiceSettings,
    envelope: EnvelopeSettings,
    gain: f32,
) -> (f32, f32) {
    let factor = usize::from(state.oversampler.factor());
    let internal_samples = SAMPLES * chunks;
    debug_assert!(internal_samples <= MAX_JOB_SAMPLES);
    let group = state.generator_groups[0];
    let pooled = state.internal_pool.render_terminal_filter_job::<SAMPLES>(
        &mut state.synth,
        settings,
        envelope,
        chunks,
        &group,
        &state.effective_generator_filters,
        &state.generator_filter_coefficients,
        false,
    );
    let mut samples = [(0.0_f32, 0.0_f32); MAX_JOB_SAMPLES];
    if let Some(block) = pooled {
        samples = block.samples;
    } else {
        for chunk in 0..chunks {
            let rendered = state.synth.render_terminal_filter_block::<SAMPLES>(
                settings,
                envelope,
                &group,
                &state.generator_filter_coefficients,
            );
            samples[chunk * SAMPLES..(chunk + 1) * SAMPLES].copy_from_slice(&rendered);
        }
    }
    let output = state.effective_generator_group_outputs[0];
    let target = usize::from(output.pair) * 2;
    let routed = target + 1 < output_channels;
    let left_gain = gain * output.gain.clamp(0.0, 2.0) * (1.0 - output.pan).sqrt();
    let right_gain = gain * output.gain.clamp(0.0, 2.0) * (1.0 + output.pan).sqrt();
    write_host_block(
        &mut state.group_oversamplers[0],
        &samples,
        factor,
        internal_samples / factor,
        |_, left, right| (left * left_gain, right * right_gain),
        buffer,
        output_channels,
        sample_index,
        Some((target, routed)),
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_terminal_filter_modulated_host_block<const SAMPLES: usize>(
    state: &mut KurvDspState,
    buffer: &mut AudioBuffer,
    output_channels: usize,
    sample_index: usize,
    control_start: usize,
    chunks: usize,
    settings: VoiceSettings,
    envelope: EnvelopeSettings,
    gain: f32,
    routes: &ActiveRoutes,
    lfo_control_dynamic_mask: u8,
    modulation: &mut lfo::ModulationFrame,
    structural: &mut StructuralModulationFrame,
) -> (f32, f32) {
    let factor = usize::from(state.oversampler.factor());
    let internal_samples = SAMPLES * chunks;
    debug_assert!(internal_samples <= MAX_JOB_SAMPLES);
    for host_frame in 0..internal_samples / factor {
        let control_frame = control_start + host_frame;
        for internal_frame in 0..factor {
            state.advance_filter_coefficients();
            advance_lfo_modulation(
                state,
                routes,
                0,
                lfo_control_dynamic_mask,
                control_frame,
                modulation,
                Some(structural),
            );
            state.update_filter_modulation(Some(structural));
            let index = host_frame * factor + internal_frame;
            state.filter_coefficients_block[index] = state.generator_filter_coefficients;
        }
    }
    let mut samples = [(0.0_f32, 0.0_f32); MAX_JOB_SAMPLES];
    for chunk in 0..chunks {
        let start = chunk * SAMPLES;
        let rendered = state
            .synth
            .render_terminal_filter_modulated_block::<SAMPLES>(
                settings,
                envelope,
                &state.generator_groups[0],
                &state.filter_coefficients_block[start..start + SAMPLES],
            );
        samples[start..start + SAMPLES].copy_from_slice(&rendered);
    }
    let output = state.effective_generator_group_outputs[0];
    let target = usize::from(output.pair) * 2;
    let routed = target + 1 < output_channels;
    let left_gain = gain * output.gain.clamp(0.0, 2.0) * (1.0 - output.pan).sqrt();
    let right_gain = gain * output.gain.clamp(0.0, 2.0) * (1.0 + output.pan).sqrt();
    write_host_block(
        &mut state.group_oversamplers[0],
        &samples,
        factor,
        internal_samples / factor,
        |_, left, right| (left * left_gain, right * right_gain),
        buffer,
        output_channels,
        sample_index,
        Some((target, routed)),
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_terminal_filter_voice_modulated_host_block<const SAMPLES: usize>(
    state: &mut KurvDspState,
    buffer: &mut AudioBuffer,
    output_channels: usize,
    sample_index: usize,
    control_start: usize,
    chunks: usize,
    settings: VoiceSettings,
    envelope: EnvelopeSettings,
    gain: f32,
    routes: &ActiveRoutes,
    modulation: &mut lfo::ModulationFrame,
    structural: &mut StructuralModulationFrame,
) -> (f32, f32) {
    let factor = usize::from(state.oversampler.factor());
    let internal_samples = SAMPLES * chunks;
    debug_assert!(internal_samples <= MAX_JOB_SAMPLES);
    advance_lfo_modulation(
        state,
        routes,
        0,
        0,
        control_start,
        modulation,
        Some(structural),
    );
    let group = state.generator_groups[0];
    let pooled = state.internal_pool.render_terminal_filter_job::<SAMPLES>(
        &mut state.synth,
        settings,
        envelope,
        chunks,
        &group,
        &state.effective_generator_filters,
        &state.generator_filter_coefficients,
        true,
    );
    let mut samples = [(0.0_f32, 0.0_f32); MAX_JOB_SAMPLES];
    if let Some(block) = pooled {
        samples = block.samples;
    } else {
        for chunk in 0..chunks {
            let start = chunk * SAMPLES;
            let rendered = state
                .synth
                .render_terminal_filter_voice_modulated_block::<SAMPLES>(
                    settings,
                    envelope,
                    &group,
                    &state.effective_generator_filters,
                    &state.generator_filter_coefficients,
                );
            samples[start..start + SAMPLES].copy_from_slice(&rendered);
        }
    }
    let output = state.effective_generator_group_outputs[0];
    let target = usize::from(output.pair) * 2;
    let routed = target + 1 < output_channels;
    let left_gain = gain * output.gain.clamp(0.0, 2.0) * (1.0 - output.pan).sqrt();
    let right_gain = gain * output.gain.clamp(0.0, 2.0) * (1.0 + output.pan).sqrt();
    write_host_block(
        &mut state.group_oversamplers[0],
        &samples,
        factor,
        internal_samples / factor,
        |_, left, right| (left * left_gain, right * right_gain),
        buffer,
        output_channels,
        sample_index,
        Some((target, routed)),
    )
}

pub(crate) fn render_saw_host_pitch_block<const SAMPLES: usize>(
    state: &mut KurvDspState,
    buffer: &mut AudioBuffer,
    output_channels: usize,
    sample_index: usize,
    settings: VoiceSettings,
    envelope: EnvelopeSettings,
    gain: f32,
    output_gains: &[f32],
    unison_modulation_mask: OscillatorMask,
) -> (f32, f32) {
    let factor = usize::from(state.oversampler.factor());
    debug_assert_eq!(SAMPLES % factor, 0);
    let samples = state.synth.render_pitch_block::<SAMPLES>(
        settings,
        envelope,
        &state.lfo_modulation_block[..SAMPLES],
        unison_modulation_mask,
    );
    write_host_block(
        &mut state.oversampler,
        &samples,
        factor,
        SAMPLES / factor,
        |frame, left, right| {
            let output_gain = output_gains.get(frame).copied().unwrap_or(1.0);
            (left * gain * output_gain, right * gain * output_gain)
        },
        buffer,
        output_channels,
        sample_index,
        None,
    )
}

pub(crate) fn fill_structural_oscillator_block<const SAMPLES: usize>(
    state: &mut KurvDspState,
    routes: &ActiveRoutes,
    lfo_control_dynamic_mask: u8,
    control_start: usize,
    factor: usize,
    output_start: usize,
    modulation: &mut lfo::ModulationFrame,
    structural: &mut StructuralModulationFrame,
) {
    debug_assert_eq!(SAMPLES % factor, 0);
    let host_frames = SAMPLES / factor;
    for host_frame in 0..host_frames {
        let control_frame = control_start + host_frame;
        for internal_frame in 0..factor {
            advance_lfo_modulation(
                state,
                routes,
                0,
                lfo_control_dynamic_mask,
                control_frame,
                modulation,
                Some(structural),
            );
            let index = output_start + host_frame * factor + internal_frame;
            state.structural_control_block[index] =
                structural_oscillator_frame_control(state, structural);
        }
    }
}

pub(crate) fn render_structural_oscillator_job<const SAMPLES: usize>(
    state: &mut KurvDspState,
    buffer: &mut AudioBuffer,
    output_channels: usize,
    sample_index: usize,
    control_start: usize,
    chunks: usize,
    settings: VoiceSettings,
    envelope: EnvelopeSettings,
    gain: f32,
    routes: &ActiveRoutes,
    lfo_control_dynamic_mask: u8,
    modulation: &mut lfo::ModulationFrame,
    structural: &mut StructuralModulationFrame,
    voice_modulation: bool,
) -> (f32, f32) {
    let factor = usize::from(state.oversampler.factor());
    let internal_samples = SAMPLES * chunks;
    debug_assert!(internal_samples <= MAX_JOB_SAMPLES);
    for chunk in 0..chunks {
        fill_structural_oscillator_block::<SAMPLES>(
            state,
            routes,
            lfo_control_dynamic_mask,
            control_start + chunk * (SAMPLES / factor),
            factor,
            chunk * SAMPLES,
            modulation,
            structural,
        );
    }
    let pooled = if voice_modulation {
        state.internal_pool.render_voice_structural_job::<SAMPLES>(
            &mut state.synth,
            settings,
            envelope,
            chunks,
            &state.structural_control_block[..internal_samples],
        )
    } else {
        state.internal_pool.render_structural_job::<SAMPLES>(
            &mut state.synth,
            settings,
            envelope,
            chunks,
            &state.structural_control_block[..internal_samples],
        )
    };
    let mut samples = [(0.0_f32, 0.0_f32); MAX_JOB_SAMPLES];
    if let Some(block) = pooled {
        debug_assert_eq!(block.len, internal_samples);
        samples = block.samples;
    } else {
        for chunk in 0..chunks {
            let start = chunk * SAMPLES;
            let controls = &state.structural_control_block[start..start + SAMPLES];
            let rendered = if voice_modulation {
                state
                    .synth
                    .render_voice_structural_modulation_block::<SAMPLES>(
                        settings, envelope, controls,
                    )
            } else {
                state
                    .synth
                    .render_structural_modulation_block::<SAMPLES>(settings, envelope, controls)
            };
            samples[start..start + SAMPLES].copy_from_slice(&rendered);
        }
    }
    let mut peak_left = 0.0_f32;
    let mut peak_right = 0.0_f32;
    for frame in 0..internal_samples / factor {
        let (left, right) = if factor == 1 {
            let (left, right) = samples[frame];
            state.oversampler.process_direct(left, right)
        } else {
            for (left, right) in samples[frame * factor..(frame + 1) * factor]
                .iter()
                .copied()
            {
                state.oversampler.push(left, right);
            }
            state.oversampler.output()
        };
        let left = left * gain;
        let right = right * gain;
        peak_left = peak_left.max(left.abs());
        peak_right = peak_right.max(right.abs());
        let output_index = sample_index + frame;
        if output_channels == 1 {
            buffer.output(0)[output_index] = (left + right) * 0.5;
        } else {
            buffer.output(0)[output_index] = left;
            buffer.output(1)[output_index] = right;
        }
    }
    (peak_left, peak_right)
}

pub(crate) fn modulated_envelope(
    base: EnvelopeSettings,
    modulation: lfo::GlobalModulation,
) -> EnvelopeSettings {
    EnvelopeSettings {
        attack: (base.attack + modulation.attack).clamp(0.0, 8.0),
        decay: (base.decay + modulation.decay).clamp(0.0, 8.0),
        sustain: (base.sustain + modulation.sustain).clamp(0.0, 1.0),
        release: (base.release + modulation.release).clamp(0.0, 12.0),
        attack_curve: (base.attack_curve + modulation.attack_curve).clamp(-1.0, 1.0),
        decay_curve: (base.decay_curve + modulation.decay_curve).clamp(-1.0, 1.0),
        release_curve: (base.release_curve + modulation.release_curve).clamp(-1.0, 1.0),
        attack_curve_time: (base.attack_curve_time + modulation.attack_curve_time)
            .clamp(0.05, 0.95),
        decay_curve_time: (base.decay_curve_time + modulation.decay_curve_time).clamp(0.05, 0.95),
        release_curve_time: (base.release_curve_time + modulation.release_curve_time)
            .clamp(0.05, 0.95),
    }
}

#[inline(always)]
pub(crate) fn advance_lfo_modulation(
    state: &mut KurvDspState,
    routes: &ActiveRoutes,
    direct_unison_mask: OscillatorMask,
    lfo_control_dynamic_mask: u8,
    frame: usize,
    modulation: &mut lfo::ModulationFrame,
    structural: Option<&mut StructuralModulationFrame>,
) {
    clear_modulation_frame(modulation, routes, direct_unison_mask);
    state.synth.begin_voice_modulation_frame();
    let polyphonic_source_mask = state.synth.voice_polyphonic_mask();
    let mut dynamic_voice_controls = u64::from(lfo_control_dynamic_mask) & polyphonic_source_mask;
    while dynamic_voice_controls != 0 {
        let index = dynamic_voice_controls.trailing_zeros() as usize;
        dynamic_voice_controls &= dynamic_voice_controls - 1;
        state.synth.set_voice_lfo_control(
            index,
            state.controls.lfo_rate[index][frame],
            state.controls.lfo_phase[index][frame],
        );
    }
    let mut structural = structural;
    if let Some(structural) = structural.as_deref_mut() {
        prepare_structural_modulation(structural, routes, state);
    }
    if !state.lfos.is_active()
        && !state.synth.voice_modulation_active()
        && !routes.mod_wheel_active
        && !routes.xy_x_active
        && !routes.xy_y_active
    {
        return;
    }
    let mod_wheel = if routes.mod_wheel_active {
        state.mod_wheel_ramp.next()
    } else {
        0.0
    };
    let sources = if state.lfos.is_active() {
        Some(if lfo_control_dynamic_mask == 0 {
            if state.lfos.direct_phase_active() {
                state.lfos.next_direct_ref()
            } else {
                state.lfos.next_ref()
            }
        } else {
            state.lfos.next_with_controls_ref(
                lfo_control_dynamic_mask,
                &state.controls.lfo_rate,
                &state.controls.lfo_phase,
                frame,
            )
        })
    } else {
        None
    };
    for route in routes.as_slice() {
        if let Some(descriptor) = route.descriptor {
            let amount = if let Some(index) = route.host_amount_index {
                state.controls.modulation_amounts[usize::from(index)][frame]
            } else if let Some(index) = route.overflow_amount_index {
                state.overflow_route_ramps[usize::from(index)].next()
            } else {
                route.amount
            };
            if let ResolvedRouteSource::Rack(index) = route.source
                && polyphonic_source_mask & (1_u64 << index) != 0
            {
                state
                    .synth
                    .push_voice_modulation_route(index, amount, descriptor);
            } else {
                accumulate_modulation(
                    modulation,
                    descriptor,
                    route_source_value(
                        route.source,
                        sources,
                        mod_wheel,
                        state.controls.xy_source_x[frame],
                        state.controls.xy_source_y[frame],
                    ),
                    amount,
                );
            }
        }
    }
    if let Some(structural) = structural {
        for route in routes.modular_slice() {
            let Some(target) = route.target else {
                continue;
            };
            let amount = if let Some(index) = route.host_amount_index {
                state.controls.modulation_amounts[usize::from(index)][frame]
            } else if let Some(index) = route.overflow_amount_index {
                state.overflow_route_ramps[usize::from(index)].next()
            } else {
                route.amount
            };
            if let ResolvedRouteSource::Rack(index) = route.source
                && polyphonic_source_mask & (1_u64 << index) != 0
                && !matches!(target, ResolvedModularTarget::Group { .. })
            {
                state
                    .synth
                    .push_voice_structural_route(index, amount, target);
            } else {
                accumulate_structural_modulation(
                    structural,
                    target,
                    route_source_value(
                        route.source,
                        sources,
                        mod_wheel,
                        state.controls.xy_source_x[frame],
                        state.controls.xy_source_y[frame],
                    ),
                    amount,
                );
            }
        }
    }
}

#[inline(always)]
pub(crate) fn route_source_value(
    source: ResolvedRouteSource,
    rack: Option<&[f32; LFO_COUNT]>,
    mod_wheel: f32,
    xy_x: f32,
    xy_y: f32,
) -> f32 {
    match source {
        ResolvedRouteSource::Rack(index) => rack.map_or(0.0, |values| values[usize::from(index)]),
        ResolvedRouteSource::ModWheel => mod_wheel,
        ResolvedRouteSource::XyX => xy_x,
        ResolvedRouteSource::XyY => xy_y,
    }
}

pub(crate) fn apply_modulation(
    state: &mut KurvDspState,
    settings: &mut VoiceSettings,
    routes: &ActiveRoutes,
    base_unison: &[UnisonSettings; LEGACY_OSCILLATOR_COUNT],
    base_glide: f32,
    direct_unison_mask: OscillatorMask,
    lfo_control_dynamic_mask: u8,
    frame: usize,
    modulation: &mut lfo::ModulationFrame,
    structural: &mut StructuralModulationFrame,
) {
    advance_lfo_modulation(
        state,
        routes,
        direct_unison_mask,
        lfo_control_dynamic_mask,
        frame,
        modulation,
        Some(structural),
    );
    if direct_unison_mask != 0 {
        for oscillator in 0..LEGACY_OSCILLATOR_COUNT {
            if direct_unison_mask & (1 << oscillator) == 0 {
                continue;
            }
            let control = &state.controls.unison_pitch[oscillator];
            let base = base_unison[oscillator];
            modulation.unison[oscillator].detune_cents +=
                control.detune_cents[frame].mul_add(100.0, -base.detune_cents());
            modulation.unison[oscillator].detune_amount +=
                control.detune_amount[frame] - base.detune_amount();
            modulation.unison[oscillator].harmonic_align +=
                control.harmonic_align[frame] - base.harmonic_align();
            modulation.unison[oscillator].curve += control.curve[frame] - base.curve();
            modulation.unison[oscillator].stereo += control.stereo[frame] - base.stereo();
            modulation.unison[oscillator].stereo_x += control.stereo_x[frame] - base.stereo_x();
            modulation.unison[oscillator].stereo_y +=
                control.stereo_y[frame] - base.stereo_alternate();
            modulation.unison[oscillator].weight += control.weight[frame] - base.level_curve();
            let pan_shape = base.pan_shape();
            modulation.unison[oscillator].pan_center +=
                control.pan_center[frame] - pan_shape.center;
            modulation.unison[oscillator].pan_left += control.pan_left[frame] - pan_shape.left_edge;
            modulation.unison[oscillator].pan_right +=
                control.pan_right[frame] - pan_shape.right_edge;
            modulation.unison[oscillator].pan_center_x +=
                control.pan_center_x[frame] - pan_shape.center_x;
        }
    }
    for oscillator in 0..LEGACY_OSCILLATOR_COUNT {
        let bit = 1 << oscillator;
        if routes.oscillator_mask & bit != 0 {
            let oscillator_modulation = modulation.oscillator[oscillator];
            // A positioned WAVE needs a complete table selection, not a raw
            // procedural-shape delta with stale curve/mix. Positioned tables
            // expose host automation but deliberately reject internal routing.
            let shape_modulation = if state.va_tables[oscillator].is_positioned() {
                0.0
            } else {
                oscillator_modulation.shape
            };
            settings.modulate_oscillator(
                oscillator,
                oscillator_modulation.pitch_semitones,
                shape_modulation,
                oscillator_modulation.pulse_width,
                oscillator_modulation.warp,
                oscillator_modulation.custom_shape,
                oscillator_modulation.level,
                oscillator_modulation.pan,
            );
        }
        if (routes.unison_frame_mask | direct_unison_mask) & bit != 0 {
            settings.modulate_unison_detune_amount(
                oscillator,
                modulation.unison[oscillator].detune_amount,
            );
        }
    }
    if routes.global_mask & GLOBAL_VELOCITY_MASK != 0 {
        settings.velocity_amount =
            (settings.velocity_amount + modulation.global.velocity).clamp(0.0, 1.0);
    }
    if routes.global_mask & GLOBAL_PRESSURE_MASK != 0 {
        settings.pressure_amount =
            (settings.pressure_amount + modulation.global.pressure).clamp(0.0, 1.0);
    }
    if routes.global_mask & GLOBAL_TIMBRE_MASK != 0 {
        settings.timbre_amount =
            (settings.timbre_amount + modulation.global.timbre).clamp(0.0, 1.0);
    }
    if routes.global_mask & GLOBAL_GLIDE_MASK != 0 {
        state
            .synth
            .set_glide_time((base_glide + modulation.global.glide).clamp(0.0, 5.0));
    }

    if routes.unison_layout_mask != 0
        && (state.lfos.is_active()
            || routes.mod_wheel_active
            || routes.xy_x_active
            || routes.xy_y_active)
    {
        for oscillator in 0..LEGACY_OSCILLATOR_COUNT {
            if routes.unison_layout_mask & (1 << oscillator) == 0 {
                continue;
            }
            let control = &state.controls.unison_pitch[oscillator];
            let settings = unison_motion_settings(
                base_unison[oscillator],
                control.phase_random[frame],
                control.jitter_amount[frame],
                control.jitter_rate[frame],
                modulation.unison[oscillator],
            );
            state.synth.configure_unison_motion(oscillator, settings);
        }
    }
}

pub(crate) fn fill_lfo_morph_block(
    state: &mut KurvDspState,
    routes: &ActiveRoutes,
    oscillator_shape_mask: OscillatorMask,
    output_active: bool,
    lfo_control_dynamic_mask: u8,
    control_start: usize,
    host_frames: usize,
    factor: usize,
    shapes: Option<&mut [[f32; MAX_JOB_SAMPLES]; LEGACY_OSCILLATOR_COUNT]>,
    output_gains: &mut [f32; MAX_JOB_SAMPLES],
    modulation: &mut lfo::ModulationFrame,
) {
    debug_assert!(host_frames * factor <= MAX_JOB_SAMPLES);
    let mut shapes = shapes;
    for host_frame in 0..host_frames {
        let frame = control_start + host_frame;
        for internal_frame in 0..factor {
            advance_lfo_modulation(
                state,
                routes,
                0,
                lfo_control_dynamic_mask,
                frame,
                modulation,
                None,
            );
            let index = host_frame * factor + internal_frame;
            if let Some(shapes) = shapes.as_deref_mut() {
                for oscillator in 0..LEGACY_OSCILLATOR_COUNT {
                    if oscillator_shape_mask & (1 << oscillator) == 0 {
                        continue;
                    }
                    shapes[oscillator][index] = (shapes[oscillator][index]
                        + modulation.oscillator[oscillator].shape)
                        .clamp(0.0, 3.0);
                }
            }
        }
        if output_active {
            output_gains[host_frame] = db_to_linear(modulation.global.output_db);
        }
    }
}

pub(crate) fn fill_lfo_pitch_block(
    state: &mut KurvDspState,
    routes: &ActiveRoutes,
    lfo_control_dynamic_mask: u8,
    control_start: usize,
    host_frames: usize,
    factor: usize,
    modulation: &mut lfo::ModulationFrame,
    output_gains: &mut [f32],
) {
    let internal_samples = host_frames * factor;
    debug_assert!(internal_samples <= state.lfo_modulation_block.len());
    for host_frame in 0..host_frames {
        let frame = control_start + host_frame;
        for internal_frame in 0..factor {
            advance_lfo_modulation(
                state,
                routes,
                0,
                lfo_control_dynamic_mask,
                frame,
                modulation,
                None,
            );
            state.lfo_modulation_block[host_frame * factor + internal_frame] = *modulation;
        }
        output_gains[host_frame] = db_to_linear(modulation.global.output_db);
    }
}

pub(crate) fn render_lfo_pitch_chunk<const SAMPLES: usize>(
    state: &mut KurvDspState,
    buffer: &mut AudioBuffer,
    output_channels: usize,
    sample_index: usize,
    control_start: usize,
    settings: VoiceSettings,
    envelope: EnvelopeSettings,
    gain: f32,
    routes: &ActiveRoutes,
    lfo_control_dynamic_mask: u8,
    modulation: &mut lfo::ModulationFrame,
) -> (f32, f32) {
    let factor = usize::from(state.oversampler.factor());
    debug_assert_eq!(SAMPLES % factor, 0);
    let host_frames = SAMPLES / factor;
    let mut output_gains = [1.0_f32; MAX_JOB_SAMPLES];
    fill_lfo_pitch_block(
        state,
        routes,
        lfo_control_dynamic_mask,
        control_start,
        host_frames,
        factor,
        modulation,
        &mut output_gains[..host_frames],
    );
    render_saw_host_pitch_block::<SAMPLES>(
        state,
        buffer,
        output_channels,
        sample_index,
        settings,
        envelope,
        gain,
        &output_gains[..host_frames],
        routes.unison_frame_mask,
    )
}

pub(crate) fn render_lfo_motion_chunk<const SAMPLES: usize>(
    state: &mut KurvDspState,
    buffer: &mut AudioBuffer,
    output_channels: usize,
    sample_index: usize,
    control_start: usize,
    settings: VoiceSettings,
    envelope: EnvelopeSettings,
    gain: f32,
    routes: &ActiveRoutes,
    base_unison: &[UnisonSettings; LEGACY_OSCILLATOR_COUNT],
    lfo_control_dynamic_mask: u8,
    modulation: &mut lfo::ModulationFrame,
) -> (f32, f32) {
    let factor = usize::from(state.oversampler.factor());
    debug_assert_eq!(SAMPLES % factor, 0);
    let host_frames = SAMPLES / factor;
    let mut output_gains = [1.0_f32; MAX_JOB_SAMPLES];
    fill_lfo_pitch_block(
        state,
        routes,
        lfo_control_dynamic_mask,
        control_start,
        host_frames,
        factor,
        modulation,
        &mut output_gains[..host_frames],
    );
    let samples = state.synth.render_motion_block::<SAMPLES>(
        settings,
        envelope,
        &state.lfo_modulation_block[..SAMPLES],
        routes.unison_layout_mask,
        base_unison,
    );
    let mut peak_left = 0.0_f32;
    let mut peak_right = 0.0_f32;
    for frame in 0..host_frames {
        let (left, right) = if factor == 1 {
            let (left, right) = samples[frame];
            state.oversampler.process_direct(left, right)
        } else {
            for (left, right) in samples[frame * factor..(frame + 1) * factor]
                .iter()
                .copied()
            {
                state.oversampler.push(left, right);
            }
            state.oversampler.output()
        };
        let output_gain = output_gains[frame];
        let left = left * gain * output_gain;
        let right = right * gain * output_gain;
        peak_left = peak_left.max(left.abs());
        peak_right = peak_right.max(right.abs());
        let output_index = sample_index + frame;
        if output_channels == 1 {
            buffer.output(0)[output_index] = (left + right) * 0.5;
        } else {
            buffer.output(0)[output_index] = left;
            buffer.output(1)[output_index] = right;
        }
    }
    (peak_left, peak_right)
}

pub(crate) fn render_lfo_control_chunk<const SAMPLES: usize>(
    state: &mut KurvDspState,
    buffer: &mut AudioBuffer,
    output_channels: usize,
    sample_index: usize,
    control_start: usize,
    settings: VoiceSettings,
    envelope: EnvelopeSettings,
    gain: f32,
    routes: &ActiveRoutes,
    lfo_control_dynamic_mask: u8,
    modulation: &mut lfo::ModulationFrame,
) -> (f32, f32) {
    let factor = usize::from(state.oversampler.factor());
    debug_assert_eq!(SAMPLES % factor, 0);
    let host_frames = SAMPLES / factor;
    let mut output_gains = [1.0_f32; MAX_JOB_SAMPLES];
    fill_lfo_pitch_block(
        state,
        routes,
        lfo_control_dynamic_mask,
        control_start,
        host_frames,
        factor,
        modulation,
        &mut output_gains[..host_frames],
    );
    let samples = state.synth.render_modulation_block::<SAMPLES>(
        settings,
        envelope,
        &state.lfo_modulation_block[..SAMPLES],
    );
    let mut peak_left = 0.0_f32;
    let mut peak_right = 0.0_f32;
    for frame in 0..host_frames {
        let (left, right) = if factor == 1 {
            let (left, right) = samples[frame];
            state.oversampler.process_direct(left, right)
        } else {
            for (left, right) in samples[frame * factor..(frame + 1) * factor]
                .iter()
                .copied()
            {
                state.oversampler.push(left, right);
            }
            state.oversampler.output()
        };
        let output_gain = output_gains[frame];
        let left = left * gain * output_gain;
        let right = right * gain * output_gain;
        peak_left = peak_left.max(left.abs());
        peak_right = peak_right.max(right.abs());
        let output_index = sample_index + frame;
        if output_channels == 1 {
            buffer.output(0)[output_index] = (left + right) * 0.5;
        } else {
            buffer.output(0)[output_index] = left;
            buffer.output(1)[output_index] = right;
        }
    }
    (peak_left, peak_right)
}

#[inline]
pub(crate) fn unison_motion_settings(
    base: UnisonSettings,
    phase_random: f32,
    jitter_amount: f32,
    jitter_rate: f32,
    modulation: lfo::UnisonModulation,
) -> UnisonSettings {
    let rate_scale = if modulation.jitter_rate_normalized == 0.0 {
        1.0
    } else {
        5_000.0_f32.powf(modulation.jitter_rate_normalized.clamp(-1.0, 1.0))
    };
    base.with_motion(
        phase_random + modulation.phase_random,
        jitter_amount + modulation.jitter_amount,
        jitter_rate * rate_scale,
    )
}

pub(crate) fn configure_direct_unison_motion(
    state: &mut KurvDspState,
    base_unison: &[UnisonSettings; LEGACY_OSCILLATOR_COUNT],
    motion_mask: OscillatorMask,
    frame: usize,
) {
    for oscillator in 0..LEGACY_OSCILLATOR_COUNT {
        if motion_mask & (1 << oscillator) == 0 {
            continue;
        }
        let control = &state.controls.unison_pitch[oscillator];
        let settings = base_unison[oscillator].with_motion(
            control.phase_random[frame],
            control.jitter_amount[frame],
            control.jitter_rate[frame],
        );
        state.synth.configure_unison_motion(oscillator, settings);
    }
}

pub(crate) fn clear_modulation_frame(
    modulation: &mut lfo::ModulationFrame,
    routes: &ActiveRoutes,
    direct_unison_mask: OscillatorMask,
) {
    for oscillator in 0..LEGACY_OSCILLATOR_COUNT {
        let bit = 1 << oscillator;
        if routes.oscillator_mask & bit != 0 {
            modulation.oscillator[oscillator] = lfo::OscillatorModulation::default();
        }
        if (routes.unison_layout_mask | routes.unison_frame_mask | direct_unison_mask) & bit != 0 {
            modulation.unison[oscillator] = lfo::UnisonModulation::default();
        }
    }
    if routes.global_mask != 0 {
        modulation.global = lfo::GlobalModulation::default();
    }
}

pub(crate) fn prepare_structural_modulation(
    modulation: &mut StructuralModulationFrame,
    routes: &ActiveRoutes,
    state: &KurvDspState,
) {
    modulation.oscillator_mask = 0;
    modulation.group_mask = 0;
    modulation.filter_mask = 0;
    for route in routes.modular_slice() {
        match route.target {
            Some(ResolvedModularTarget::Oscillator { slot, .. }) => {
                let slot = usize::from(slot);
                let bit = 1 << slot;
                if modulation.oscillator_mask & bit == 0 {
                    modulation.oscillators[slot] = StructuralOscillatorDelta::default();
                    modulation.oscillator_mask |= bit;
                }
            }
            Some(ResolvedModularTarget::Group { index, .. }) => {
                let index = usize::from(index);
                let bit = 1 << index;
                if index < state.generator_group_count && modulation.group_mask & bit == 0 {
                    modulation.groups[index] = StructuralGroupDelta::default();
                    modulation.group_mask |= bit;
                }
            }
            Some(ResolvedModularTarget::Filter { slot, .. }) => {
                let slot = usize::from(slot);
                let bit = 1 << slot;
                if modulation.filter_mask & bit == 0 {
                    modulation.filters[slot] = StructuralFilterDelta::default();
                    modulation.filter_mask |= bit;
                }
            }
            None => {}
        }
    }
}

#[inline(always)]
pub(crate) fn accumulate_structural_modulation(
    modulation: &mut StructuralModulationFrame,
    target: ResolvedModularTarget,
    source_value: f32,
    amount: f32,
) {
    let value = source_value * amount.clamp(-1.0, 1.0);
    match target {
        ResolvedModularTarget::Oscillator { slot, control } => {
            let destination = &mut modulation.oscillators[usize::from(slot)];
            match control {
                OscillatorControl::Shape => destination.shape += value * 3.0,
                OscillatorControl::PulseWidth => destination.pulse_width += value * 0.47,
                OscillatorControl::Transpose => destination.pitch_semitones += value * 48.0,
                OscillatorControl::Cents => destination.pitch_semitones += value,
                OscillatorControl::Level => destination.level += value,
                OscillatorControl::Pan => destination.pan += value,
                OscillatorControl::PhasePosition => destination.phase_position += value,
                OscillatorControl::PhaseWarpAmount => destination.warp += value,
                OscillatorControl::UnisonJitter => destination.unison_jitter += value,
                OscillatorControl::UnisonRate => destination.unison_rate += value,
                OscillatorControl::UnisonStereoPosition => destination.stereo_x += value,
                OscillatorControl::UnisonStereoAlternate => destination.stereo_y += value,
                OscillatorControl::GrainTune => destination.grain_tune += value,
                OscillatorControl::GrainStereo => destination.grain_stereo += value,
                OscillatorControl::RichDynamic => destination.rich_dynamic += value,
                OscillatorControl::TablePosition
                | OscillatorControl::PhaseRandom
                | OscillatorControl::UnisonVoices
                | OscillatorControl::UnisonRange
                | OscillatorControl::UnisonAmount
                | OscillatorControl::UnisonCurve
                | OscillatorControl::UnisonWidth
                | OscillatorControl::UnisonWeight
                | OscillatorControl::UnisonAlignment
                | OscillatorControl::UnisonPanCurve
                | OscillatorControl::UnisonPanCenter
                | OscillatorControl::RichBalance
                | OscillatorControl::RichFormant
                | OscillatorControl::RichAir
                | OscillatorControl::RichDiffuse => {}
            }
        }
        ResolvedModularTarget::Group { index, control } => {
            let destination = &mut modulation.groups[usize::from(index)];
            match control {
                GroupControl::Gain => destination.gain += value * 2.0,
                GroupControl::Pan => destination.pan += value,
                GroupControl::AttackCurve => destination.attack_curve += value * 2.0,
                GroupControl::DecayCurve => destination.decay_curve += value * 2.0,
                GroupControl::ReleaseCurve => destination.release_curve += value * 2.0,
                GroupControl::Dry => destination.dry += value,
                GroupControl::Send => destination.send += value,
                GroupControl::Sidechain => destination.sidechain += value,
                GroupControl::Attack
                | GroupControl::Decay
                | GroupControl::Sustain
                | GroupControl::Release => {}
            }
        }
        ResolvedModularTarget::Filter { slot, control } => {
            let destination = &mut modulation.filters[usize::from(slot)];
            match control {
                FilterControl::Cutoff => destination.cutoff_octaves += value * 4.0,
                FilterControl::Resonance => destination.resonance_octaves += value * 4.0,
                FilterControl::Slope => destination.slope += value,
                FilterControl::Morph => destination.morph += value,
            }
        }
    }
}

pub(crate) fn structural_oscillator_frame_control(
    state: &KurvDspState,
    modulation: &StructuralModulationFrame,
) -> StructuralOscillatorFrameControl {
    let mut control = StructuralOscillatorFrameControl::default();
    control.mask = modulation.oscillator_mask;
    let mut mask = modulation.oscillator_mask;
    while mask != 0 {
        let slot = mask.trailing_zeros() as usize;
        mask &= mask - 1;
        let base = state.effective_generator_oscillators[slot];
        let delta = modulation.oscillators[slot];
        let level = (base.level + delta.level).clamp(0.0, 1.0);
        let pan = (base.pan + delta.pan).clamp(-1.0, 1.0);
        let target = &mut control.slots[slot];
        // Keep the coherent selection installed by oscillator configuration;
        // applying only a raw Shape delta would retain stale custom curve/mix.
        target.shape = if state.va_tables[slot].is_positioned() {
            state.va_tables[slot]
                .select(WaveCurveRt::default(), base.custom_shape, base.shape / 3.0)
                .shape
        } else {
            (base.shape + delta.shape).clamp(0.0, 3.0)
        };
        target.pulse_width = (base.pulse_width + delta.pulse_width).clamp(0.03, 0.97);
        target.pitch_ratio =
            OscillatorSettings::pitch_ratio(base.transpose + delta.pitch_semitones, base.cents);
        target.phase_position = (base.phase_position + delta.phase_position).rem_euclid(1.0);
        target.phase_warp_amount = (base.phase_warp_amount + delta.warp).clamp(0.0, 1.0);
        target.unison_jitter = (base.unison_jitter + delta.unison_jitter).clamp(0.0, 1.0);
        target.unison_rate = (base.unison_rate + delta.unison_rate).clamp(0.0, 1.0);
        target.left_gain = level * (1.0 - pan).sqrt();
        target.right_gain = level * (1.0 + pan).sqrt();
        target.stereo_x = delta.stereo_x;
        target.stereo_y = delta.stereo_y;
        target.grain_tune = delta.grain_tune;
        target.grain_stereo = delta.grain_stereo;
        target.rich_dynamic = delta.rich_dynamic;
    }
    control
}

#[cfg(test)]
pub(crate) use super::legacy_automation::{
    effective_legacy_pan_segments, refresh_legacy_materialized_automation,
    refresh_legacy_pan_automation,
};
pub(crate) use super::legacy_automation::{
    host_automated_generator_configuration, refresh_host_automation_targets,
};

#[inline(always)]
pub(crate) fn accumulate_modulation(
    modulation: &mut lfo::ModulationFrame,
    target: modulation_target::TargetDescriptor,
    source_value: f32,
    amount: f32,
) {
    modulation.accumulate(target, source_value, amount);
}
