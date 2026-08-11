use crate::*;

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
    let worthwhile_generic_job = generic_shape && internal_samples >= 128;
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
        } else if state.internal_pool_enabled() && !full_coarse_job && !generic_shape {
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
        let output_gain = output_gains.map_or(1.0, |gains| gains[frame]);
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
    let mut peak_left = 0.0_f32;
    let mut peak_right = 0.0_f32;
    for frame in 0..SAMPLES / factor {
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
        let output_gain = output_gains.get(frame).copied().unwrap_or(1.0);
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
    let mut structural = structural;
    if let Some(structural) = structural.as_deref_mut() {
        prepare_structural_modulation(structural, routes, state);
    }
    if !state.lfos.is_active() && !routes.mod_wheel_active {
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
            accumulate_modulation(
                modulation,
                descriptor,
                route_source_value(route.source, sources, mod_wheel),
                amount,
            );
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
            accumulate_structural_modulation(
                structural,
                target,
                route_source_value(route.source, sources, mod_wheel),
                amount,
            );
        }
    }
}

#[inline(always)]
pub(crate) fn route_source_value(
    source: ResolvedRouteSource,
    rack: Option<&[f32; LFO_COUNT]>,
    mod_wheel: f32,
) -> f32 {
    match source {
        ResolvedRouteSource::Rack(index) => rack.map_or(0.0, |values| values[usize::from(index)]),
        ResolvedRouteSource::ModWheel => mod_wheel,
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
            settings.modulate_oscillator(
                oscillator,
                oscillator_modulation.pitch_semitones,
                oscillator_modulation.shape,
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

    if routes.unison_layout_mask != 0 && (state.lfos.is_active() || routes.mod_wheel_active) {
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
                | OscillatorControl::UnisonStereoPosition
                | OscillatorControl::UnisonStereoAlternate => {}
            }
        }
        ResolvedModularTarget::Group { index, control } => {
            let destination = &mut modulation.groups[usize::from(index)];
            match control {
                GroupControl::Gain => destination.gain += value * 2.0,
                GroupControl::Pan => destination.pan += value,
                GroupControl::Attack
                | GroupControl::AttackCurve
                | GroupControl::Decay
                | GroupControl::DecayCurve
                | GroupControl::Sustain
                | GroupControl::Release
                | GroupControl::ReleaseCurve => {}
            }
        }
        ResolvedModularTarget::Filter { slot, control } => {
            let destination = &mut modulation.filters[usize::from(slot)];
            match control {
                FilterControl::Cutoff => destination.cutoff_octaves += value * 4.0,
                FilterControl::Resonance => destination.resonance_octaves += value * 4.0,
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
        target.shape = (base.shape + delta.shape).clamp(0.0, 3.0);
        target.pulse_width = (base.pulse_width + delta.pulse_width).clamp(0.03, 0.97);
        target.pitch_ratio =
            OscillatorSettings::pitch_ratio(base.transpose + delta.pitch_semitones, base.cents);
        target.phase_position = (base.phase_position + delta.phase_position).rem_euclid(1.0);
        target.phase_warp_amount = (base.phase_warp_amount + delta.warp).clamp(0.0, 1.0);
        target.unison_jitter = (base.unison_jitter + delta.unison_jitter).clamp(0.0, 1.0);
        target.unison_rate = (base.unison_rate + delta.unison_rate).clamp(0.0, 1.0);
        target.left_gain = level * (1.0 - pan).sqrt();
        target.right_gain = level * (1.0 + pan).sqrt();
    }
    control
}

pub(crate) fn refresh_host_automation_targets(
    state: &mut KurvDspState,
    params: &KurvParams,
) -> bool {
    let Some((generation, targets)) = params
        .host_automation_targets
        .try_rt_snapshot_after(state.host_automation_generation)
    else {
        return false;
    };
    state.host_automation_generation = generation;
    state.host_automation_targets = targets.map(|target| {
        target.filter(|target| !matches!(target, ModulationRouteTarget::Legacy { .. }))
    });
    state.host_automation_len = 0;
    for (slot, target) in state.host_automation_targets.iter().enumerate() {
        if target.is_some() {
            state.host_automation_slots[usize::from(state.host_automation_len)] = slot as u8;
            state.host_automation_len += 1;
        }
    }
    true
}

pub(crate) fn host_automated_generator_configuration(
    state: &KurvDspState,
    params: &KurvParams,
) -> (
    [generators::OscillatorConfig; generators::MAX_OSCILLATORS],
    [filters::FilterConfig; generators::MAX_FILTERS],
    [generators::GroupOutput; generators::MAX_OUTPUT_PAIRS],
) {
    let mut oscillators = state.generator_oscillators;
    let mut filters = state.generator_filters;
    let mut groups = state.generator_group_outputs;
    for slot in state.host_automation_slots[..usize::from(state.host_automation_len)]
        .iter()
        .copied()
        .map(usize::from)
    {
        let Some(target) = state.host_automation_targets[slot] else {
            continue;
        };
        let normalized = params
            .get_normalized(u32::from(HOST_AUTOMATION_PARAMS[slot]))
            .unwrap_or_default() as f32;
        match target {
            ModulationRouteTarget::Legacy { .. } => {}
            ModulationRouteTarget::Oscillator {
                module_id,
                slot,
                control,
            } => {
                let index = slot.index();
                if state.generator_module_ids[index] != module_id {
                    continue;
                }
                control.apply_normalized(&mut oscillators[index], normalized);
            }
            ModulationRouteTarget::Group { group_id, control } => {
                let Some(index) = state.generator_group_ids[..state.generator_group_count]
                    .iter()
                    .position(|id| *id == group_id)
                else {
                    continue;
                };
                control.apply_normalized(&mut groups[index], normalized);
            }
            ModulationRouteTarget::Filter {
                module_id,
                slot,
                control,
            } => {
                let index = slot.index();
                if state.generator_filter_module_ids[index] != module_id {
                    continue;
                }
                control.apply_normalized(&mut filters[index], normalized);
            }
        }
    }
    (oscillators, filters, groups)
}

#[inline(always)]
pub(crate) fn accumulate_modulation(
    modulation: &mut lfo::ModulationFrame,
    target: modulation_target::TargetDescriptor,
    source_value: f32,
    amount: f32,
) {
    use modulation_target::{GlobalTarget, OscTarget, TargetKind, UnisonTarget};

    let value = source_value * amount.clamp(-1.0, 1.0);
    let scaled = value * target.scale;
    match target.kind {
        TargetKind::Oscillator {
            oscillator,
            control,
        } => {
            let destination = &mut modulation.oscillator[usize::from(oscillator)];
            match control {
                OscTarget::Pitch => destination.pitch_semitones += scaled,
                OscTarget::Shape => destination.shape += scaled,
                OscTarget::PulseWidth => destination.pulse_width += scaled,
                OscTarget::Warp => destination.warp += scaled,
                OscTarget::CustomShape => destination.custom_shape += scaled,
                OscTarget::Level => destination.level += scaled,
                OscTarget::Pan => destination.pan += scaled,
            }
        }
        TargetKind::Unison {
            oscillator,
            control,
        } => {
            let destination = &mut modulation.unison[usize::from(oscillator)];
            match control {
                UnisonTarget::DetuneAmount => destination.detune_amount += scaled,
                UnisonTarget::DetuneRange => destination.detune_cents += scaled,
                UnisonTarget::HarmonicAlign => destination.harmonic_align += scaled,
                UnisonTarget::Stereo => destination.stereo += scaled,
                UnisonTarget::PhaseRandom => destination.phase_random += scaled,
                UnisonTarget::Curve => destination.curve += scaled,
                UnisonTarget::JitterAmount => destination.jitter_amount += scaled,
                UnisonTarget::JitterRate => destination.jitter_rate_normalized += value,
                UnisonTarget::StereoX => destination.stereo_x += scaled,
                UnisonTarget::StereoY => destination.stereo_y += scaled,
                UnisonTarget::Weight => destination.weight += scaled,
                UnisonTarget::PanCenter => destination.pan_center += scaled,
                UnisonTarget::PanLeft => destination.pan_left += scaled,
                UnisonTarget::PanRight => destination.pan_right += scaled,
                UnisonTarget::PanCenterX => destination.pan_center_x += scaled,
            }
        }
        TargetKind::Global(control) => match control {
            GlobalTarget::Output => modulation.global.output_db += scaled,
            GlobalTarget::Attack => modulation.global.attack += scaled,
            GlobalTarget::Decay => modulation.global.decay += scaled,
            GlobalTarget::Sustain => modulation.global.sustain += scaled,
            GlobalTarget::Release => modulation.global.release += scaled,
            GlobalTarget::AttackCurve => modulation.global.attack_curve += scaled,
            GlobalTarget::DecayCurve => modulation.global.decay_curve += scaled,
            GlobalTarget::ReleaseCurve => modulation.global.release_curve += scaled,
            GlobalTarget::AttackCurveTime => modulation.global.attack_curve_time += scaled,
            GlobalTarget::DecayCurveTime => modulation.global.decay_curve_time += scaled,
            GlobalTarget::ReleaseCurveTime => modulation.global.release_curve_time += scaled,
            GlobalTarget::Velocity => modulation.global.velocity += scaled,
            GlobalTarget::Pressure => modulation.global.pressure += scaled,
            GlobalTarget::Timbre => modulation.global.timbre += scaled,
            GlobalTarget::Glide => modulation.global.glide += scaled,
        },
    }
}
