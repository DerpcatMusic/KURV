//! RESYNTH pack wire format and artifact codec.

use crate::oscillators::{
    GrainSourceArtifact, LEGACY_RICH_FRAME_COUNT, LEGACY_RICH_ZONE_SAMPLES, PitchMode, PitchTrack,
    PitchTrackFrame, ProductionResynthArtifact, RICH_FRAME_COUNT, RICH_FRAME_SAMPLES,
    RICH_ZONE_COUNT, RICH_ZONE_SAMPLES, ResynthAlgorithm, ResynthControls, ResynthQuality,
    ResynthRtArtifact, RichVocoderArtifact, RichVocoderFrame, RichZoneArtifact, SampleLoopArtifact,
    SourceAuditionArtifact, VOCODER_ENVELOPE_BINS, VOCODER_MAX_FRAMES, VOCODER_MAX_HARMONICS,
    VOCODER_MAX_RESIDUAL_SAMPLES,
};
#[cfg(test)]
use crate::oscillators::{ScaleId, TargetSet};

pub(super) const MAGIC: &[u8; 8] = b"KRVRSY01";
pub(super) const LEGACY_PACK_VERSION: u16 = 3;
pub(super) const SAMPLE_RECEIPT_PACK_VERSION: u16 = 4;
pub(super) const GRAIN_PLAY_PACK_VERSION: u16 = 5;
pub(super) const GRAIN_ENVELOPE_PACK_VERSION: u16 = 6;
pub(super) const GRAIN_OFFSET_PACK_VERSION: u16 = 7;
pub(super) const GRAIN_EFFECTS_PACK_VERSION: u16 = 8;
pub(super) const GRAIN_TUNE_PACK_VERSION: u16 = 9;
pub(super) const COMPACT_ANALYSIS_PACK_VERSION: u16 = 10;
pub(super) const MODE_CONTROLS_PACK_VERSION: u16 = 11;
pub(super) const SPECTRAL_GRAIN_PACK_VERSION: u16 = 12;
pub(super) const PITCH_MODE_PACK_VERSION: u16 = 13;
pub(super) const RICH_TIMELINE_PACK_VERSION: u16 = 14;
pub(super) const RICH_SEQUENCE_PACK_VERSION: u16 = 15;
pub(super) const RICH_VOCODER_PACK_VERSION: u16 = 16;
pub(super) const GRAIN_SPEED_PACK_VERSION: u16 = 17;
pub(super) const RICH_STEREO_PACK_VERSION: u16 = 18;
pub(super) const GRAIN_PITCH_TRACK_PACK_VERSION: u16 = 19;
pub(super) const GRAIN_NORMALIZE_PACK_VERSION: u16 = 22;
pub(super) const RICH_RT_PACK_VERSION: u16 = 27;
pub(super) const RICH_PHASE_PACK_VERSION: u16 = 28;
pub(super) const PAD_ENGINE_PACK_VERSION: u16 = 29;
pub(super) const DSP_REBUILD_PACK_VERSION: u16 = RICH_PHASE_PACK_VERSION;
pub(super) const PACK_VERSION: u16 = PAD_ENGINE_PACK_VERSION;

pub(super) fn pack_has_sample_receipt(pack_version: u16) -> bool {
    pack_version >= SAMPLE_RECEIPT_PACK_VERSION
}

pub(super) fn pack_has_grain_play(pack_version: u16) -> bool {
    pack_version >= GRAIN_PLAY_PACK_VERSION
}

pub(super) fn pack_has_grain_envelope(pack_version: u16) -> bool {
    pack_version >= GRAIN_ENVELOPE_PACK_VERSION
}

pub(super) fn pack_has_grain_offsets(pack_version: u16) -> bool {
    pack_version >= GRAIN_OFFSET_PACK_VERSION
}

pub(super) fn pack_has_grain_effects(pack_version: u16) -> bool {
    pack_version >= GRAIN_EFFECTS_PACK_VERSION
}

pub(super) fn pack_has_grain_tune(pack_version: u16) -> bool {
    pack_version >= GRAIN_TUNE_PACK_VERSION
}

pub(super) fn pack_has_preview_cycles(pack_version: u16) -> bool {
    pack_version < COMPACT_ANALYSIS_PACK_VERSION
}

pub(super) fn pack_has_continuous_mode_controls(pack_version: u16) -> bool {
    pack_version >= MODE_CONTROLS_PACK_VERSION
}

pub(super) fn pack_has_spectral_grain(pack_version: u16) -> bool {
    pack_version >= SPECTRAL_GRAIN_PACK_VERSION
}

pub(super) fn pack_has_pitch_mode(pack_version: u16) -> bool {
    pack_version >= PITCH_MODE_PACK_VERSION
}

#[inline]
fn pack_has_full_rich_timeline(pack_version: u16) -> bool {
    pack_version >= RICH_TIMELINE_PACK_VERSION
}

#[inline]
fn pack_has_rich_sequence(pack_version: u16) -> bool {
    pack_version >= RICH_SEQUENCE_PACK_VERSION
}

#[inline]
fn pack_has_rich_vocoder(pack_version: u16) -> bool {
    pack_version >= RICH_VOCODER_PACK_VERSION
}

#[inline]
fn pack_has_rich_stereo(pack_version: u16) -> bool {
    pack_version >= RICH_STEREO_PACK_VERSION
}

#[inline]
fn pack_has_grain_pitch_track(pack_version: u16) -> bool {
    pack_version >= GRAIN_PITCH_TRACK_PACK_VERSION
}

#[inline]
pub(super) fn pack_has_grain_speed(pack_version: u16) -> bool {
    pack_version >= GRAIN_SPEED_PACK_VERSION
}

#[inline]
pub(super) fn pack_has_loop_region(pack_version: u16) -> bool {
    pack_version >= DSP_REBUILD_PACK_VERSION
}

#[inline]
fn pack_has_rich_rt(pack_version: u16) -> bool {
    pack_version >= RICH_RT_PACK_VERSION
}

#[inline]
fn pack_has_rich_phase(pack_version: u16) -> bool {
    pack_version >= RICH_PHASE_PACK_VERSION
}

pub(super) const HASH_BYTES: usize = 32;
pub(super) const MAX_ARTIFACT_ABS_SAMPLE: f32 = 16.0;

pub(super) fn write_controls(output: &mut Vec<u8>, controls: ResynthControls, pack_version: u16) {
    for value in [
        controls.position,
        controls.grain_size,
        controls.grain_density,
        controls.grain_spray,
        controls.rich_balance,
        controls.rich_formant_semitones,
        controls.rich_air_db,
        controls.rich_diffuse,
    ] {
        write_f32(output, value);
    }
    output.extend_from_slice(&controls.seed.to_le_bytes());
    if pack_has_grain_play(pack_version) {
        output.push(controls.grain_direction);
        output.extend_from_slice(&[0, 0, 0]);
        for value in [
            controls.grain_envelope,
            controls.grain_timing,
            controls.grain_pitch_spread,
            controls.grain_level_spread,
            controls.grain_pan_spread,
            controls.grain_reverse,
        ] {
            write_f32(output, value);
        }
        if pack_has_grain_envelope(pack_version) {
            for value in [
                controls.grain_attack,
                controls.grain_hold,
                controls.grain_release,
            ] {
                write_f32(output, value);
            }
        }
        if pack_has_grain_offsets(pack_version) {
            for value in [
                controls.grain_pitch,
                controls.grain_pan,
                controls.grain_level,
            ] {
                write_f32(output, value);
            }
        }
        if pack_has_grain_effects(pack_version) {
            write_f32(output, controls.grain_blur);
            write_f32(output, controls.grain_normalize);
        }
        if pack_has_grain_tune(pack_version) {
            if pack_has_continuous_mode_controls(pack_version) {
                write_f32(output, controls.grain_tune);
                write_f32(output, controls.grain_stereo);
                write_f32(output, controls.rich_dynamic);
            } else {
                output.push(u8::from(controls.grain_tune >= 0.5));
            }
        }
    }
    if pack_has_pitch_mode(pack_version) {
        let (mode, scale) = controls.pitch_mode.to_wire();
        output.push(mode);
        output.push(scale);
    }
    if pack_has_grain_speed(pack_version) {
        write_f32(output, controls.grain_speed);
    }
    if pack_has_loop_region(pack_version) {
        write_f32(output, controls.loop_start);
        write_f32(output, controls.loop_end);
    }
    if pack_has_rich_rt(pack_version) {
        write_f32(output, controls.rich_rt);
    }
}
pub(super) fn read_controls(input: &mut Reader<'_>, pack_version: u16) -> Option<ResynthControls> {
    let mut controls = ResynthControls {
        position: input.f32()?,
        grain_size: input.f32()?,
        grain_density: input.f32()?,
        grain_spray: input.f32()?,
        rich_balance: input.f32()?,
        rich_formant_semitones: input.f32()?,
        rich_air_db: input.f32()?,
        rich_diffuse: input.f32()?,
        seed: input.u64()?,
        ..ResynthControls::default()
    };
    if pack_has_grain_play(pack_version) {
        controls.grain_direction = input.u8()?;
        let _ = input.bytes(3)?;
        controls.grain_envelope = input.f32()?;
        controls.grain_timing = input.f32()?;
        controls.grain_pitch_spread = input.f32()?;
        controls.grain_level_spread = input.f32()?;
        controls.grain_pan_spread = input.f32()?;
        controls.grain_reverse = input.f32()?;
        if pack_has_grain_envelope(pack_version) {
            controls.grain_attack = input.f32()?;
            controls.grain_hold = input.f32()?;
            controls.grain_release = input.f32()?;
        }
        if pack_has_grain_offsets(pack_version) {
            controls.grain_pitch = input.f32()?;
            controls.grain_pan = input.f32()?;
            controls.grain_level = input.f32()?;
        }
        if pack_has_grain_effects(pack_version) {
            controls.grain_blur = input.f32()?;
            let value = input.f32()?;
            controls.grain_normalize = if pack_version < GRAIN_NORMALIZE_PACK_VERSION {
                1.0 - value
            } else {
                value
            };
        }
        if pack_has_grain_tune(pack_version) {
            if pack_has_continuous_mode_controls(pack_version) {
                controls.grain_tune = input.f32()?;
                controls.grain_stereo = input.f32()?;
                controls.rich_dynamic = input.f32()?;
            } else {
                controls.grain_tune = f32::from(input.u8()? != 0);
            }
        }
    }
    if pack_has_pitch_mode(pack_version) {
        controls.pitch_mode = PitchMode::from_wire(input.u8()?, input.u8()?)?;
        if matches!(controls.pitch_mode, PitchMode::Target(_))
            && controls.grain_tune <= f32::EPSILON
        {
            controls.grain_tune = 1.0;
        }
    } else {
        // Before v13 Tune was the source-to-tuned spectral blend. Preserve
        // that audible behavior while new controls default to explicit
        // Classic mode.
        controls.pitch_mode = PitchMode::Target(crate::oscillators::TargetSet::PlayedNote);
    }
    if pack_has_grain_speed(pack_version) {
        controls.grain_speed = input.f32()?;
    }
    if pack_has_loop_region(pack_version) {
        controls.loop_start = input.f32()?;
        controls.loop_end = input.f32()?;
    }
    if pack_has_rich_rt(pack_version) {
        controls.rich_rt = input.f32()?;
    }
    Some(controls)
}
pub(super) fn write_u16(output: &mut Vec<u8>, value: u16) {
    output.extend_from_slice(&value.to_le_bytes());
}
pub(super) fn write_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes());
}
pub(super) fn artifact_persisted_bytes(
    artifact: &ResynthRtArtifact,
    pack_version: u16,
) -> Option<usize> {
    let base = 1_usize.checked_add(1 + 4 + 4)?;
    match &artifact.data {
        ProductionResynthArtifact::Sample(sample) => base
            .checked_add(4 + 4 + 4)?
            .checked_add(if pack_has_sample_receipt(pack_version) {
                4 * 4
            } else {
                0
            })?
            .checked_add(sample.samples.len().checked_mul(4)?),
        ProductionResynthArtifact::Grain(grain) => base
            .checked_add(4 + (1 + 4) + 4 + 4 + 2)?
            .checked_add(grain.samples.len().checked_mul(4)?)?
            .checked_add(if pack_has_continuous_mode_controls(pack_version) {
                4_usize.checked_add(grain.side_samples.len().checked_mul(4)?)?
            } else {
                0
            })?
            .checked_add(grain.tuned_samples.len().checked_mul(4)?)?
            .checked_add(if pack_has_spectral_grain(pack_version) {
                4_usize.checked_add(grain.tuned_side_samples.len().checked_mul(4)?)?
            } else {
                0
            })?
            .checked_add(grain.transients.len().checked_mul(4)?)?
            .checked_add(if pack_has_grain_pitch_track(pack_version) {
                4_usize.checked_add(grain.pitch_track.len().checked_mul(12)?)?
            } else {
                0
            }),
        ProductionResynthArtifact::Rich(rich) => {
            let header = base.checked_add(4 + 4)?;
            if pack_has_rich_vocoder(pack_version)
                && let Some(vocoder) = rich.vocoder()
            {
                let bytes =
                    header
                        .checked_add(1 + 4 + 1 + 4)?
                        .checked_add(vocoder.len().checked_mul(
                            16 + VOCODER_ENVELOPE_BINS * 4
                                + if pack_has_rich_phase(pack_version) {
                                    VOCODER_MAX_HARMONICS * 4
                                } else {
                                    0
                                },
                        )?)?;
                if !pack_has_rich_stereo(pack_version) {
                    return Some(bytes);
                }
                let (residual_left, residual_right) = vocoder.residual_channels();
                if vocoder.right_envelopes().len() != vocoder.len()
                    || residual_left.len() != residual_right.len()
                    || residual_left.len() > VOCODER_MAX_RESIDUAL_SAMPLES
                {
                    return None;
                }
                return bytes
                    .checked_add(vocoder.len().checked_mul(VOCODER_ENVELOPE_BINS * 4)?)?
                    .checked_add(4)?
                    .checked_add(residual_left.len().checked_mul(8)?);
            }
            if pack_has_rich_sequence(pack_version) {
                if let Some(sequence) = rich.sequence() {
                    let bytes = header
                        .checked_add(1 + 4 + 4)?
                        .checked_add(sequence.samples.len().checked_mul(4)?)?
                        .checked_add(2)?
                        .checked_add(sequence.transients.len().checked_mul(4)?)?;
                    return if pack_has_rich_rt(pack_version) && !sequence.side_samples.is_empty() {
                        bytes
                            .checked_add(4)?
                            .checked_add(sequence.side_samples.len().checked_mul(4)?)
                    } else {
                        Some(bytes)
                    };
                }
                let slabs = rich.slabs.as_deref()?;
                header.checked_add(1)?.checked_add(
                    RICH_ZONE_COUNT
                        .checked_mul(std::mem::size_of_val(&slabs[0]))?
                        .checked_add((RICH_FRAME_COUNT + 1) * 4)?
                        .checked_add(RICH_ZONE_COUNT * 4 + RICH_ZONE_COUNT * 2)?
                        .checked_add(RICH_FRAME_COUNT * 4)?,
                )
            } else {
                let slabs = rich.slabs.as_deref()?;
                header
                    .checked_add(if pack_has_full_rich_timeline(pack_version) {
                        (RICH_FRAME_COUNT + 1) * 4
                    } else {
                        0
                    })
                    .and_then(|bytes| bytes.checked_add(RICH_ZONE_COUNT * 4 + RICH_ZONE_COUNT * 2))?
                    .checked_add(if pack_has_continuous_mode_controls(pack_version) {
                        if pack_has_full_rich_timeline(pack_version) {
                            RICH_FRAME_COUNT * 4
                        } else {
                            LEGACY_RICH_FRAME_COUNT * 4
                        }
                    } else {
                        0
                    })?
                    .checked_add(if pack_has_full_rich_timeline(pack_version) {
                        std::mem::size_of_val(slabs)
                    } else {
                        RICH_ZONE_COUNT
                            .checked_mul(LEGACY_RICH_ZONE_SAMPLES)?
                            .checked_mul(4)?
                    })
            }
        }
    }
}

pub(super) fn write_artifact(
    output: &mut Vec<u8>,
    artifact: &ResynthRtArtifact,
    pack_version: u16,
) -> Option<()> {
    output.push(artifact.algorithm as u8);
    write_option_f32(output, artifact.source_root_hz);
    write_f32(output, artifact.source_audition_gain);
    match &artifact.data {
        ProductionResynthArtifact::Sample(sample) => {
            write_f32(output, sample.source_sample_rate);
            write_f32(output, sample.root_hz);
            write_u32(output, u32::try_from(sample.samples.len()).ok()?);
            if pack_has_sample_receipt(pack_version) {
                let source_start_frames = sample.source_start_frames();
                let source_span_frames = sample.source_span_frames();
                let source_total_frames = sample.source_total_frames();
                let crossfade_frames = sample.crossfade_frames();
                if source_start_frames
                    .checked_add(source_span_frames)
                    .is_none_or(|end| end > source_total_frames)
                    || crossfade_frames > source_span_frames
                {
                    return None;
                }
                write_u32(output, u32::try_from(source_start_frames).ok()?);
                write_u32(output, u32::try_from(source_span_frames).ok()?);
                write_u32(output, u32::try_from(source_total_frames).ok()?);
                write_u32(output, u32::try_from(crossfade_frames).ok()?);
            }
            for value in sample.samples.iter().copied() {
                write_f32(output, value);
            }
        }
        ProductionResynthArtifact::Grain(grain) => {
            write_f32(output, grain.source_sample_rate);
            write_option_f32(output, grain.root_hz);
            write_u32(output, u32::try_from(grain.samples.len()).ok()?);
            for value in grain.samples.iter().copied() {
                write_f32(output, value);
            }
            if pack_has_continuous_mode_controls(pack_version) {
                if !grain.side_samples.is_empty() && grain.side_samples.len() != grain.samples.len()
                {
                    return None;
                }
                write_u32(output, u32::try_from(grain.side_samples.len()).ok()?);
                for value in grain.side_samples.iter().copied() {
                    write_f32(output, value);
                }
            }
            if !grain.tuned_samples.is_empty() && grain.tuned_samples.len() != grain.samples.len() {
                return None;
            }
            write_u32(output, u32::try_from(grain.tuned_samples.len()).ok()?);
            for value in grain.tuned_samples.iter().copied() {
                write_f32(output, value);
            }
            if pack_has_spectral_grain(pack_version) {
                if !grain.tuned_side_samples.is_empty()
                    && grain.tuned_side_samples.len() != grain.samples.len()
                {
                    return None;
                }
                write_u32(output, u32::try_from(grain.tuned_side_samples.len()).ok()?);
                for value in grain.tuned_side_samples.iter().copied() {
                    write_f32(output, value);
                }
            }
            write_u16(output, u16::try_from(grain.transients.len()).ok()?);
            for transient in grain.transients.iter().copied() {
                write_u32(output, transient);
            }
            if pack_has_grain_pitch_track(pack_version) {
                write_u32(output, u32::try_from(grain.pitch_track.len()).ok()?);
                for frame in grain.pitch_track.frames() {
                    write_f32(output, frame.f0_hz);
                    write_f32(output, frame.confidence);
                    write_f32(output, frame.onset);
                }
            }
        }
        ProductionResynthArtifact::Rich(rich) => {
            write_f32(output, rich.source_sample_rate);
            write_u32(output, rich.source_frames);
            if pack_has_rich_vocoder(pack_version)
                && let Some(vocoder) = rich.vocoder()
            {
                output.push(if pack_has_rich_stereo(pack_version) {
                    3
                } else {
                    2
                });
                write_f32(output, vocoder.synth_gain);
                output.push(vocoder.quality as u8);
                write_u32(output, u32::try_from(vocoder.len()).ok()?);
                for frame in vocoder.frames() {
                    write_f32(output, frame.f0_hz);
                    write_f32(output, frame.voiced);
                    write_f32(output, frame.gain);
                    write_f32(output, frame.aperiodicity);
                    for value in frame.envelope {
                        write_f32(output, value);
                    }
                    if pack_has_rich_phase(pack_version) {
                        for value in frame.phase {
                            write_f32(output, value);
                        }
                    }
                }
                if pack_has_rich_stereo(pack_version) {
                    if vocoder.right_envelopes().len() != vocoder.len() {
                        return None;
                    }
                    for envelope in vocoder.right_envelopes() {
                        for value in envelope {
                            write_f32(output, *value);
                        }
                    }
                    let (residual_left, residual_right) = vocoder.residual_channels();
                    if residual_left.len() != residual_right.len()
                        || residual_left.len() > VOCODER_MAX_RESIDUAL_SAMPLES
                    {
                        return None;
                    }
                    write_u32(output, u32::try_from(residual_left.len()).ok()?);
                    for value in residual_left.iter().chain(residual_right) {
                        write_f32(output, *value);
                    }
                }
                return Some(());
            }
            if pack_has_rich_sequence(pack_version) {
                if let Some(sequence) = rich.sequence() {
                    let stereo_sequence =
                        pack_has_rich_rt(pack_version) && !sequence.side_samples.is_empty();
                    output.push(if stereo_sequence { 4 } else { 1 });
                    write_f32(output, rich.locked_density());
                    write_f32(output, rich.locked_size());
                    write_u32(output, u32::try_from(sequence.samples.len()).ok()?);
                    for value in sequence.samples.iter().copied() {
                        write_f32(output, value);
                    }
                    write_u16(output, u16::try_from(sequence.transients.len()).ok()?);
                    for transient in sequence.transients.iter().copied() {
                        write_u32(output, transient);
                    }
                    if stereo_sequence {
                        write_u32(output, u32::try_from(sequence.side_samples.len()).ok()?);
                        for value in sequence.side_samples.iter().copied() {
                            write_f32(output, value);
                        }
                    }
                    return Some(());
                }
                output.push(0);
            }
            let slabs = rich.slabs.as_deref()?;
            if pack_has_full_rich_timeline(pack_version) {
                for value in rich.source_boundaries {
                    write_u32(output, value);
                }
            }
            for value in rich.center_hz {
                write_f32(output, value);
            }
            for value in rich.fundamental_bins {
                write_u16(output, value);
            }
            if pack_has_continuous_mode_controls(pack_version) {
                if pack_has_full_rich_timeline(pack_version) {
                    for value in rich.frame_gains {
                        write_f32(output, value);
                    }
                } else {
                    let expansion = RICH_FRAME_COUNT / LEGACY_RICH_FRAME_COUNT;
                    for index in 0..LEGACY_RICH_FRAME_COUNT {
                        write_f32(output, rich.frame_gains[index * expansion]);
                    }
                }
            }
            if pack_has_full_rich_timeline(pack_version) {
                for slab in slabs {
                    for value in slab.iter().copied() {
                        write_f32(output, value);
                    }
                }
            } else {
                for slab in slabs {
                    for frame in 0..LEGACY_RICH_FRAME_COUNT {
                        let start = frame * RICH_FRAME_SAMPLES;
                        for value in slab[start..start + RICH_FRAME_SAMPLES].iter().copied() {
                            write_f32(output, value);
                        }
                    }
                }
            }
        }
    }
    Some(())
}

pub(super) fn read_artifact(
    input: &mut Reader<'_>,
    controls: ResynthControls,
    source_audition: Box<SourceAuditionArtifact>,
    pack_version: u16,
) -> Option<ResynthRtArtifact> {
    let algorithm = ResynthAlgorithm::from_u8(input.u8()?)?;
    let source_root_hz = read_option_f32(input)?;
    let source_frames = source_audition.samples.len();
    let source_audition_gain = input.f32()?;
    if source_audition_gain.to_bits() != 1.0_f32.to_bits() {
        return None;
    }
    let data = match algorithm {
        ResynthAlgorithm::Sample => {
            let source_sample_rate = input.f32()?;
            let root_hz = input.f32()?;
            let length = usize::try_from(input.u32()?).ok()?;
            if !source_sample_rate.is_finite()
                || source_sample_rate.to_bits() != source_audition.source_sample_rate.to_bits()
                || Some(root_hz) != source_root_hz
                || !root_hz.is_finite()
                || !(20.0..=2_000.0).contains(&root_hz)
                || length < 2
                || length > crate::oscillators::SAMPLE_MAX_FRAMES
            {
                return None;
            }
            let receipt = if pack_has_sample_receipt(pack_version) {
                Some([
                    usize::try_from(input.u32()?).ok()?,
                    usize::try_from(input.u32()?).ok()?,
                    usize::try_from(input.u32()?).ok()?,
                    usize::try_from(input.u32()?).ok()?,
                ])
            } else {
                None
            };
            let samples = read_artifact_samples(input, length)?;
            let sample = match receipt {
                None | Some([0, 0, 0, 0]) => {
                    SampleLoopArtifact::from_persisted(source_sample_rate, root_hz, samples)
                }
                Some(
                    [
                        source_start_frames,
                        source_span_frames,
                        source_total_frames,
                        crossfade_frames,
                    ],
                ) => SampleLoopArtifact::from_persisted_with_receipt(
                    source_sample_rate,
                    root_hz,
                    samples,
                    source_start_frames,
                    source_span_frames,
                    source_total_frames,
                    crossfade_frames,
                )?,
            };
            ProductionResynthArtifact::Sample(Box::new(sample))
        }
        ResynthAlgorithm::Grain => {
            let source_sample_rate = input.f32()?;
            let root_hz = read_option_f32(input)?;
            let length = usize::try_from(input.u32()?).ok()?;
            let maximum_frames = if pack_has_spectral_grain(pack_version) {
                crate::oscillators::GRAIN_MAX_SOURCE_FRAMES
            } else {
                crate::oscillators::SAMPLE_MAX_FRAMES
            };
            let source_stride = source_frames.div_ceil(maximum_frames).max(1);
            let expected_sample_rate = source_audition.source_sample_rate / source_stride as f32;
            if !source_sample_rate.is_finite()
                || source_sample_rate.to_bits() != expected_sample_rate.to_bits()
                || root_hz != source_root_hz
                || root_hz
                    .is_some_and(|root| !root.is_finite() || !(20.0..=2_000.0).contains(&root))
                || length == 0
                || length > maximum_frames
            {
                return None;
            }
            let samples = read_artifact_samples(input, length)?;
            let side_samples = if pack_has_continuous_mode_controls(pack_version) {
                let side_length = usize::try_from(input.u32()?).ok()?;
                if side_length != 0 && side_length != length {
                    return None;
                }
                read_artifact_samples(input, side_length)?
            } else {
                Vec::new().into_boxed_slice()
            };
            let tuned_length = usize::try_from(input.u32()?).ok()?;
            if tuned_length != 0 && tuned_length != length {
                return None;
            }
            let tuned_samples = read_artifact_samples(input, tuned_length)?;
            let tuned_side_samples = if pack_has_spectral_grain(pack_version) {
                let tuned_side_length = usize::try_from(input.u32()?).ok()?;
                if tuned_side_length != 0 && tuned_side_length != length {
                    return None;
                }
                read_artifact_samples(input, tuned_side_length)?
            } else {
                Vec::new().into_boxed_slice()
            };
            let transient_count = usize::from(input.u16()?);
            if transient_count > 128 {
                return None;
            }
            let mut transients = Vec::with_capacity(transient_count);
            let mut previous = None;
            for _ in 0..transient_count {
                let transient = input.u32()?;
                if usize::try_from(transient).ok()? >= length
                    || previous.is_some_and(|value| transient <= value)
                {
                    return None;
                }
                previous = Some(transient);
                transients.push(transient);
            }
            let pitch_track = if pack_has_grain_pitch_track(pack_version) {
                let frame_count = usize::try_from(input.u32()?).ok()?;
                if frame_count > 8_192 {
                    return None;
                }
                let mut frames = Vec::with_capacity(frame_count);
                for _ in 0..frame_count {
                    let frame = PitchTrackFrame {
                        f0_hz: input.f32()?,
                        confidence: input.f32()?,
                        onset: input.f32()?,
                    };
                    if !frame.f0_hz.is_finite()
                        || !(0.0..=4_000.0).contains(&frame.f0_hz)
                        || !frame.confidence.is_finite()
                        || !(0.0..=1.0).contains(&frame.confidence)
                        || !frame.onset.is_finite()
                        || !(0.0..=1.0).contains(&frame.onset)
                    {
                        return None;
                    }
                    frames.push(frame);
                }
                PitchTrack::from_frames(frames)
            } else {
                PitchTrack::default()
            };
            ProductionResynthArtifact::Grain(Box::new(
                GrainSourceArtifact::from_persisted_with_channels(
                    source_sample_rate,
                    root_hz,
                    controls,
                    samples,
                    side_samples,
                    tuned_samples,
                    tuned_side_samples,
                    transients.into_boxed_slice(),
                    pitch_track,
                ),
            ))
        }
        ResynthAlgorithm::Rich => {
            let source_sample_rate = input.f32()?;
            let source_frames = input.u32()?;
            if !source_sample_rate.is_finite() || source_sample_rate <= 0.0 || source_frames == 0 {
                return None;
            }
            if pack_has_rich_sequence(pack_version) {
                let flag = input.u8()?;
                if pack_has_rich_vocoder(pack_version)
                    && (flag == 2 || (pack_has_rich_stereo(pack_version) && flag == 3))
                {
                    let synth_gain = input.f32()?;
                    let quality = ResynthQuality::from_u8(input.u8()?);
                    let frame_count = usize::try_from(input.u32()?).ok()?;
                    if !(1..=VOCODER_MAX_FRAMES).contains(&frame_count) {
                        return None;
                    }
                    let mut frames = Vec::with_capacity(frame_count);
                    for _ in 0..frame_count {
                        let f0_hz = input.f32()?;
                        let voiced = input.f32()?;
                        let gain = input.f32()?;
                        let aperiodicity = input.f32()?;
                        if !f0_hz.is_finite()
                            || f0_hz < 0.0
                            || f0_hz > 4_000.0
                            || !voiced.is_finite()
                            || !(0.0..=1.0).contains(&voiced)
                            || !gain.is_finite()
                            || !(0.0..=1.0).contains(&gain)
                            || !aperiodicity.is_finite()
                            || !(0.0..=1.0).contains(&aperiodicity)
                        {
                            return None;
                        }
                        let mut envelope = [0.0_f32; VOCODER_ENVELOPE_BINS];
                        for slot in &mut envelope {
                            *slot = input.f32()?;
                            if !slot.is_finite() || !(-200.0..=40.0).contains(slot) {
                                return None;
                            }
                        }
                        let mut phase = [0.0_f32; VOCODER_MAX_HARMONICS];
                        if pack_has_rich_phase(pack_version) {
                            for slot in &mut phase {
                                *slot = input.f32()?;
                                if !slot.is_finite() || slot.abs() > std::f32::consts::PI {
                                    return None;
                                }
                            }
                        }
                        frames.push(RichVocoderFrame {
                            f0_hz,
                            voiced,
                            gain,
                            aperiodicity,
                            envelope,
                            phase,
                        });
                    }
                    let vocoder = if flag == 3 {
                        let mut right_envelopes = Vec::with_capacity(frame_count);
                        for _ in 0..frame_count {
                            let mut envelope = [0.0_f32; VOCODER_ENVELOPE_BINS];
                            for slot in &mut envelope {
                                *slot = input.f32()?;
                                if !slot.is_finite() || !(-200.0..=40.0).contains(slot) {
                                    return None;
                                }
                            }
                            right_envelopes.push(envelope);
                        }
                        let residual_len = usize::try_from(input.u32()?).ok()?;
                        if residual_len == 0 || residual_len > VOCODER_MAX_RESIDUAL_SAMPLES {
                            return None;
                        }
                        let residual_left = read_artifact_samples(input, residual_len)?;
                        let residual_right = read_artifact_samples(input, residual_len)?;
                        RichVocoderArtifact::from_persisted_channels(
                            source_sample_rate,
                            source_frames,
                            source_root_hz.unwrap_or(220.0),
                            synth_gain,
                            quality,
                            frames,
                            right_envelopes,
                            residual_left,
                            residual_right,
                        )?
                    } else {
                        RichVocoderArtifact::from_persisted(
                            source_sample_rate,
                            source_frames,
                            source_root_hz.unwrap_or(220.0),
                            synth_gain,
                            quality,
                            frames,
                        )?
                    };
                    let mut rich = RichZoneArtifact::unrendered(
                        source_sample_rate as u32,
                        usize::try_from(source_frames).ok()?,
                        source_root_hz.unwrap_or(220.0),
                        controls,
                    );
                    rich.restore_vocoder(vocoder);
                    ProductionResynthArtifact::Rich(Box::new(rich))
                } else if flag == 1 || (flag == 4 && pack_has_rich_rt(pack_version)) {
                    let locked_density = input.f32()?;
                    let locked_size = input.f32()?;
                    if !locked_density.is_finite() || !locked_size.is_finite() {
                        return None;
                    }
                    let length = usize::try_from(input.u32()?).ok()?;
                    if length < 2 {
                        return None;
                    }
                    let samples = read_artifact_samples(input, length)?;
                    let transient_count = usize::from(input.u16()?);
                    if transient_count > 128 {
                        return None;
                    }
                    let mut transients = Vec::with_capacity(transient_count);
                    let mut previous = None;
                    for _ in 0..transient_count {
                        let transient = input.u32()?;
                        if usize::try_from(transient).ok()? >= length
                            || previous.is_some_and(|value| transient <= value)
                        {
                            return None;
                        }
                        previous = Some(transient);
                        transients.push(transient);
                    }
                    let side_samples = if flag == 4 {
                        let side_len = usize::try_from(input.u32()?).ok()?;
                        if side_len != length {
                            return None;
                        }
                        read_artifact_samples(input, side_len)?
                    } else {
                        Vec::new().into_boxed_slice()
                    };
                    let mut rich = RichZoneArtifact::unrendered(
                        source_sample_rate as u32,
                        usize::try_from(source_frames).ok()?,
                        source_root_hz.unwrap_or(220.0),
                        controls,
                    );
                    rich.restore_sequence(
                        GrainSourceArtifact::from_persisted_with_channels(
                            source_sample_rate,
                            source_root_hz,
                            controls,
                            samples,
                            side_samples,
                            Vec::new().into_boxed_slice(),
                            Vec::new().into_boxed_slice(),
                            transients.into_boxed_slice(),
                            PitchTrack::default(),
                        ),
                        locked_density,
                        locked_size,
                    );
                    ProductionResynthArtifact::Rich(Box::new(rich))
                } else if flag == 0 {
                    read_rich_slab_artifact(
                        input,
                        pack_version,
                        source_sample_rate,
                        source_frames,
                        controls,
                    )?
                } else {
                    return None;
                }
            } else {
                read_rich_slab_artifact(
                    input,
                    pack_version,
                    source_sample_rate,
                    source_frames,
                    controls,
                )?
            }
        }
    };
    Some(ResynthRtArtifact {
        algorithm,
        source_root_hz,
        data,
        source_audition,
        source_audition_gain,
    })
}

fn read_rich_slab_artifact(
    input: &mut Reader<'_>,
    pack_version: u16,
    source_sample_rate: f32,
    source_frames: u32,
    controls: ResynthControls,
) -> Option<ProductionResynthArtifact> {
    let mut source_boundaries = [0_u32; RICH_FRAME_COUNT + 1];
    if pack_has_full_rich_timeline(pack_version) {
        for boundary in &mut source_boundaries {
            *boundary = input.u32()?;
        }
        if source_boundaries[0] != 0
            || source_boundaries[RICH_FRAME_COUNT] != source_frames
            || source_boundaries.windows(2).any(|pair| pair[0] > pair[1])
        {
            return None;
        }
    }
    let mut center_hz = [0.0_f32; RICH_ZONE_COUNT];
    for value in &mut center_hz {
        *value = input.f32()?;
        if !value.is_finite() || *value <= 0.0 {
            return None;
        }
    }
    let mut fundamental_bins = [0_u16; RICH_ZONE_COUNT];
    for value in &mut fundamental_bins {
        *value = input.u16()?;
        if *value == 0 {
            return None;
        }
    }
    let rich = if pack_has_full_rich_timeline(pack_version) {
        let mut frame_gains = [1.0_f32; RICH_FRAME_COUNT];
        if pack_has_continuous_mode_controls(pack_version) {
            for value in &mut frame_gains {
                *value = input.f32()?;
                if !value.is_finite() || !(0.0..=1.0).contains(value) {
                    return None;
                }
            }
        }
        let mut slabs = Box::<[[f32; RICH_ZONE_SAMPLES]; RICH_ZONE_COUNT]>::new_uninit();
        // SAFETY: all-zero is valid and every value is checked below.
        let mut slabs = unsafe {
            std::ptr::write_bytes(slabs.as_mut_ptr(), 0, 1);
            slabs.assume_init()
        };
        for slab in slabs.iter_mut() {
            for value in slab {
                *value = input.f32()?;
                if !value.is_finite() || value.abs() > MAX_ARTIFACT_ABS_SAMPLE {
                    return None;
                }
            }
        }
        RichZoneArtifact::from_persisted(
            source_sample_rate,
            source_frames,
            source_boundaries,
            center_hz,
            fundamental_bins,
            frame_gains,
            controls.rich_dynamic,
            slabs,
        )
    } else {
        let mut frame_gains = [1.0_f32; LEGACY_RICH_FRAME_COUNT];
        if pack_has_continuous_mode_controls(pack_version) {
            for value in &mut frame_gains {
                *value = input.f32()?;
                if !value.is_finite() || !(0.0..=1.0).contains(value) {
                    return None;
                }
            }
        }
        let mut slabs = Box::<[[f32; LEGACY_RICH_ZONE_SAMPLES]; RICH_ZONE_COUNT]>::new_uninit();
        // SAFETY: all-zero is valid and every value is checked below.
        let mut slabs = unsafe {
            std::ptr::write_bytes(slabs.as_mut_ptr(), 0, 1);
            slabs.assume_init()
        };
        for slab in slabs.iter_mut() {
            for value in slab {
                *value = input.f32()?;
                if !value.is_finite() || value.abs() > MAX_ARTIFACT_ABS_SAMPLE {
                    return None;
                }
            }
        }
        RichZoneArtifact::from_legacy_persisted(
            source_sample_rate,
            source_frames,
            center_hz,
            fundamental_bins,
            frame_gains,
            controls.rich_dynamic,
            slabs,
        )
    };
    Some(ProductionResynthArtifact::Rich(Box::new(rich)))
}

pub(super) fn read_artifact_samples(input: &mut Reader<'_>, length: usize) -> Option<Box<[f32]>> {
    let mut values = Vec::with_capacity(length);
    for _ in 0..length {
        let value = input.f32()?;
        if !value.is_finite() || value.abs() > MAX_ARTIFACT_ABS_SAMPLE {
            return None;
        }
        values.push(value);
    }
    Some(values.into_boxed_slice())
}

pub(super) fn write_option_f32(output: &mut Vec<u8>, value: Option<f32>) {
    output.push(u8::from(value.is_some()));
    write_f32(output, value.unwrap_or(0.0));
}

pub(super) fn read_option_f32(input: &mut Reader<'_>) -> Option<Option<f32>> {
    match input.u8()? {
        0 => (input.f32()?.to_bits() == 0).then_some(None),
        1 => Some(Some(input.f32()?)),
        _ => None,
    }
}

pub(super) fn write_f32(output: &mut Vec<u8>, value: f32) {
    write_u32(output, value.to_bits());
}

pub(super) struct Reader<'a> {
    data: &'a [u8],
    position: usize,
}
impl<'a> Reader<'a> {
    pub(super) const fn new(data: &'a [u8]) -> Self {
        Self { data, position: 0 }
    }
    pub(super) fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.position)
    }
    pub(super) fn bytes(&mut self, count: usize) -> Option<&'a [u8]> {
        let end = self.position.checked_add(count)?;
        let bytes = self.data.get(self.position..end)?;
        self.position = end;
        Some(bytes)
    }
    pub(super) fn u8(&mut self) -> Option<u8> {
        Some(*self.bytes(1)?.first()?)
    }
    pub(super) fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.bytes(2)?.try_into().ok()?))
    }
    pub(super) fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.bytes(4)?.try_into().ok()?))
    }
    pub(super) fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.bytes(8)?.try_into().ok()?))
    }
    pub(super) fn f32(&mut self) -> Option<f32> {
        Some(f32::from_bits(self.u32()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pitch_mode_controls_round_trip_and_legacy_preserves_tune_mode() {
        let mut controls = ResynthControls::default();
        controls.pitch_mode = PitchMode::Target(TargetSet::Scale(ScaleId::Dorian));
        let mut encoded = Vec::new();
        write_controls(&mut encoded, controls, PACK_VERSION);
        let decoded = read_controls(&mut Reader::new(&encoded), PACK_VERSION).expect("controls");
        assert_eq!(decoded.pitch_mode, controls.pitch_mode);

        let mut legacy = Vec::new();
        write_controls(&mut legacy, controls, SPECTRAL_GRAIN_PACK_VERSION);
        let decoded = read_controls(&mut Reader::new(&legacy), SPECTRAL_GRAIN_PACK_VERSION)
            .expect("legacy controls");
        assert_eq!(decoded.pitch_mode, PitchMode::Target(TargetSet::PlayedNote));
    }
}
