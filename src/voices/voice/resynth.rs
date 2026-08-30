//! RESYNTH voice rendering kernel.

use super::*;

#[inline]
#[cfg(test)]
pub(super) fn generate_resynth_step(
    oscillator: &mut VaOscillator,
    settings: &OscillatorDspSettings,
    grain_states: &mut [GrainSchedulerState; 2],
    grain_generations: &mut [u64; 2],
    vocoder_states: &mut [RichVocoderState; 2],
    vocoder_generations: &mut [u64; 2],
    target_hz: f32,
    sample_rate: f32,
    note_seed: u64,
    _grain_lane: usize,
    grain_frame: u64,
) -> (f32, f32) {
    generate_resynth_step_modulated(
        oscillator,
        settings,
        grain_states,
        grain_generations,
        vocoder_states,
        vocoder_generations,
        target_hz,
        sample_rate,
        note_seed,
        _grain_lane,
        grain_frame,
        None,
    )
}

#[inline]
#[allow(clippy::too_many_arguments)]
pub(super) fn generate_resynth_step_modulated(
    oscillator: &mut VaOscillator,
    settings: &OscillatorDspSettings,
    grain_states: &mut [GrainSchedulerState; 2],
    grain_generations: &mut [u64; 2],
    vocoder_states: &mut [RichVocoderState; 2],
    vocoder_generations: &mut [u64; 2],
    target_hz: f32,
    sample_rate: f32,
    note_seed: u64,
    _grain_lane: usize,
    grain_frame: u64,
    modulation: Option<&StructuralOscillatorAbsoluteControl>,
) -> (f32, f32) {
    let mut phase = oscillator.phase();
    let mut rich_zone = usize::from(oscillator.resynth_zone());
    // SAFETY: the pointer targets the owning PolySynth's boxed playback plan
    // and the borrow cannot escape this render operation.
    let Some(plan) = (unsafe { settings.resynth_playback.get() }) else {
        return (0.0, 0.0);
    };
    // SAFETY: this reference is consumed only during the current immediate
    // render operation, before the callback acknowledges its live generations.
    let rich_artifact = unsafe { plan.to.artifact() }.and_then(|artifact| match &artifact.data {
        ProductionResynthArtifact::Rich(rich) => Some(rich.as_ref()),
        _ => None,
    });
    if plan.remaining != 0
        || rich_artifact.is_some_and(|rich| rich.vocoder().is_some() || rich.sequence().is_some())
    {
        // Sequence Rich has no pitch-zone oscillator. Revision handover
        // already owns the two-layer budget.
        oscillator.cancel_resynth_zone_handover();
    } else if oscillator.resynth_zone_fade_remaining() == 0
        && let Some(rich) = rich_artifact
    {
        let next_zone = rich.zone_for_frequency_hysteretic(rich_zone, target_hz);
        if next_zone != rich_zone {
            let old_bin = f32::from(
                rich.fundamental_bins[rich_zone.min(crate::oscillators::RICH_ZONE_COUNT - 1)],
            )
            .max(1.0);
            let new_bin = f32::from(rich.fundamental_bins[next_zone]).max(1.0);
            phase = (phase * old_bin / new_bin).rem_euclid(1.0);
            oscillator.set_phase(f64::from(phase));
            oscillator.begin_resynth_zone_handover(
                u8::try_from(next_zone).unwrap_or(u8::MAX),
                RICH_ZONE_HANDOVER_SAMPLES,
            );
            rich_zone = next_zone;
        }
    }
    let from_generation = plan.from.generation();
    let to_generation = plan.to.generation();
    if from_generation != 0
        && grain_generations[0] != from_generation
        && grain_generations[1] == from_generation
    {
        grain_states[0] = grain_states[1];
        grain_generations[0] = from_generation;
    }
    let mut controls = plan.grain_controls;
    if let Some(modulation) = modulation {
        controls.grain_tune = (controls.grain_tune + modulation.grain_tune).clamp(0.0, 1.0);
        controls.grain_stereo = (controls.grain_stereo + modulation.grain_stereo).clamp(0.0, 1.0);
        controls.rich_dynamic = (controls.rich_dynamic + modulation.rich_dynamic).clamp(0.0, 1.0);
    }
    let grain_controls = Some(controls);
    let from_timeline =
        rich_timeline_for_view(oscillator, plan.from, 0, sample_rate, controls.position);
    let (from_left, from_right, from_step) = evaluate_resynth_layer(
        plan.from,
        phase,
        rich_zone,
        &mut grain_states[0],
        &mut grain_generations[0],
        &mut vocoder_states[0],
        &mut vocoder_generations[0],
        target_hz,
        sample_rate,
        note_seed,
        grain_frame,
        from_timeline,
        grain_controls,
        settings.phase_position,
        settings.phase_random,
        &plan.grain_curve,
    );
    let (to_left, to_right, to_step) = if to_generation == from_generation {
        (from_left, from_right, from_step)
    } else {
        let to_timeline =
            rich_timeline_for_view(oscillator, plan.to, 1, sample_rate, controls.position);
        evaluate_resynth_layer(
            plan.to,
            phase,
            rich_zone,
            &mut grain_states[1],
            &mut grain_generations[1],
            &mut vocoder_states[1],
            &mut vocoder_generations[1],
            target_hz,
            sample_rate,
            note_seed,
            grain_frame,
            to_timeline,
            grain_controls,
            settings.phase_position,
            settings.phase_random,
            &plan.grain_curve,
        )
    };
    oscillator.advance_phase(if to_generation != 0 {
        to_step
    } else {
        from_step
    });
    let mut left = if to_generation == from_generation {
        to_left
    } else {
        from_left.mul_add(plan.from_gain, to_left * plan.to_gain)
    };
    let mut right = if to_generation == from_generation {
        to_right
    } else {
        from_right.mul_add(plan.from_gain, to_right * plan.to_gain)
    };
    if plan.remaining == 0
        && oscillator.resynth_zone_fade_remaining() != 0
        && let Some(rich) =
            rich_artifact.filter(|rich| rich.vocoder().is_none() && rich.sequence().is_none())
    {
        let from_zone = usize::from(oscillator.resynth_zone_from());
        let to_bin = f32::from(rich.fundamental_bins[rich_zone]).max(1.0);
        let from_bin = f32::from(rich.fundamental_bins[from_zone]).max(1.0);
        let from_phase = (phase * to_bin / from_bin).rem_euclid(1.0);
        let from_increment = rich.phase_increment(from_zone, target_hz, sample_rate);
        let from_sample = rich.eval_at_timeline(
            from_zone,
            from_phase,
            from_increment * crate::oscillators::RICH_FRAME_SAMPLES as f32,
            from_timeline,
            sample_rate,
            grain_controls.map_or(rich.dynamic(), |controls| controls.rich_dynamic),
        );
        let progress =
            f32::from(RICH_ZONE_HANDOVER_SAMPLES - oscillator.resynth_zone_fade_remaining())
                / f32::from(RICH_ZONE_HANDOVER_SAMPLES);
        let angle = progress * std::f32::consts::FRAC_PI_2;
        let mixed = from_sample.mul_add(angle.cos(), left * angle.sin());
        left = mixed;
        right = mixed;
        oscillator.advance_resynth_zone_handover();
    }
    (left, right)
}

#[inline]
pub(super) fn grain_uses_single_oscillator_lane(settings: &OscillatorDspSettings) -> bool {
    if settings.engine != OscillatorEngineKind::Resynth {
        return false;
    }
    // SAFETY: the plan is address-stable for the current audio callback and
    // this reference does not escape the immediate render decision.
    matches!(
        unsafe { settings.resynth_playback.get() }.and_then(|plan| plan.sounding_algorithm()),
        Some(
            crate::oscillators::ResynthAlgorithm::Grain
                | crate::oscillators::ResynthAlgorithm::Rich
        )
    )
}

pub(super) fn apply_resynth_bus_mix(
    settings: &OscillatorDspSettings,
    source_state: &mut SourceAuditionState,
    grain_frame: u64,
    sample_rate: f32,
    left_gain: f32,
    right_gain: f32,
    before_left: f32,
    before_right: f32,
    left: &mut f32,
    right: &mut f32,
) {
    if settings.engine != OscillatorEngineKind::Resynth {
        return;
    }
    // SAFETY: the plan is owned by the address-stable PolySynth for this
    // immediate render operation.
    let Some(plan) = (unsafe { settings.resynth_playback.get() }) else {
        return;
    };
    if plan.source_mix <= f32::EPSILON || plan.to.generation() == 0 || plan.remaining != 0 {
        return;
    }
    // SAFETY: the to-generation remains in the current callback's live plan.
    let Some(artifact) = (unsafe { plan.to.artifact() }) else {
        return;
    };
    let source_key = plan.to.generation() ^ plan.source_epoch.rotate_left(17);
    let source = source_state.render(
        artifact.source_audition.as_ref(),
        source_key,
        sample_rate,
        grain_frame,
    ) * artifact.source_audition_gain;
    let algorithm_gain = (1.0 - plan.source_mix).sqrt();
    let source_gain = plan.source_mix.sqrt();
    *left = before_left + (*left - before_left) * algorithm_gain + source * source_gain * left_gain;
    *right =
        before_right + (*right - before_right) * algorithm_gain + source * source_gain * right_gain;
}

fn evaluate_resynth_layer(
    view: crate::resynth_state::ResynthArtifactView,
    phase: f32,
    rich_zone: usize,
    grain_state: &mut GrainSchedulerState,
    grain_generation: &mut u64,
    vocoder_state: &mut RichVocoderState,
    vocoder_generation: &mut u64,
    target_hz: f32,
    sample_rate: f32,
    note_seed: u64,
    grain_frame: u64,
    rich_timeline: f32,
    grain_controls: Option<crate::oscillators::ResynthControls>,
    phase_position: f32,
    phase_random: f32,
    grain_curve: &crate::wave_curve::WaveCurveRt,
) -> (f32, f32, f32) {
    let generation = view.generation();
    // SAFETY: the playback plan's live generations are acknowledged only after
    // this immediate evaluation, keeping the immutable node owned.
    let Some(artifact) = (unsafe { view.artifact() }) else {
        let step = target_hz / sample_rate.max(1.0);
        return (0.0, 0.0, step);
    };
    match &artifact.data {
        ProductionResynthArtifact::Sample(sample) => {
            let phase_increment = sample.phase_increment(target_hz, sample_rate);
            let sample = sample.eval_bandlimited(phase, phase_increment * sample.frames() as f32);
            (sample, sample, phase_increment)
        }
        ProductionResynthArtifact::Grain(grain) => {
            if *grain_generation != generation {
                grain_state.reset();
                *grain_generation = generation;
            }
            let controls = grain_controls.unwrap_or(grain.controls);
            let (left, right) = grain_state.render_cloud_with_curve(
                grain.as_ref(),
                target_hz,
                sample_rate,
                note_seed,
                grain_frame,
                controls,
                phase_position,
                phase_random,
                Some(grain_curve),
            );
            (left, right, target_hz / sample_rate.max(1.0))
        }
        ProductionResynthArtifact::Rich(rich) => {
            if let Some(vocoder) = rich.vocoder() {
                if *vocoder_generation != generation {
                    vocoder_state.reset();
                    *vocoder_generation = generation;
                }
                let controls = grain_controls.unwrap_or_default();
                let (left, right) = vocoder_state.render_stereo(
                    vocoder,
                    rich_timeline,
                    target_hz,
                    sample_rate,
                    controls,
                );
                (left, right, target_hz / sample_rate.max(1.0))
            } else if let Some(sequence) = rich.sequence() {
                let phase_increment = sequence.periodic_phase_increment(target_hz, sample_rate);
                let source_step = phase_increment * sequence.samples.len() as f32;
                let sample = sequence.eval_periodic(phase, source_step);
                (sample, sample, phase_increment)
            } else {
                let phase_increment = rich.phase_increment(rich_zone, target_hz, sample_rate);
                let sample = rich.eval_at_timeline(
                    rich_zone,
                    phase,
                    phase_increment * crate::oscillators::RICH_FRAME_SAMPLES as f32,
                    rich_timeline,
                    sample_rate,
                    grain_controls.map_or(rich.dynamic(), |controls| controls.rich_dynamic),
                );
                (sample, sample, phase_increment)
            }
        }
    }
}

#[inline]
fn rich_timeline_for_view(
    oscillator: &mut VaOscillator,
    view: crate::resynth_state::ResynthArtifactView,
    layer: usize,
    sample_rate: f32,
    position: f32,
) -> f32 {
    // SAFETY: the view remains pinned by the current playback plan for this
    // immediate render operation.
    let Some(artifact) = (unsafe { view.artifact() }) else {
        return 0.0;
    };
    let ProductionResynthArtifact::Rich(rich) = &artifact.data else {
        return 0.0;
    };
    (oscillator.advance_rich_timeline(
        layer,
        view.generation(),
        rich.source_frames,
        rich.source_sample_rate,
        sample_rate,
    ) + position)
        .rem_euclid(1.0)
}
