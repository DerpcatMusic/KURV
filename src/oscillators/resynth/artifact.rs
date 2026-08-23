//! Production RESYNTH runtime artifacts.
//!
//! Every compiler in this module is document/worker-side. Evaluation and the
//! grain scheduler are fixed-work, allocation-free, lock-free, and I/O-free.

mod grain;
mod rich;
mod sample;
mod shared;
mod spectral_tune;

#[cfg(test)]
pub use grain::GrainLayerState;
pub use grain::{GrainSchedulerState, GrainSourceArtifact};
pub use rich::RichZoneArtifact;
pub(crate) use rich::{RichSourceAnalysis, rich_source_analysis_with_cancel};
pub use sample::{SampleLoopArtifact, SourceAuditionArtifact, SourceAuditionState};
pub(crate) use shared::bandlimit_source_by_stride_with_cancel;
pub(super) use shared::remove_dc_and_peak_normalize;
pub use shared::{
    ArtifactBuildError, GRAIN_MAX_SOURCE_FRAMES, GRAIN_TELEMETRY, MAX_SOURCE_ABS_SAMPLE,
    RICH_FRAME_COUNT, RICH_FRAME_SAMPLES, RICH_ZONE_COUNT, RICH_ZONE_SAMPLES, SAMPLE_MAX_FRAMES,
};
#[cfg(test)]
pub use shared::{GRAIN_LAYERS, RICH_ASSET_SAMPLE_RATE, RICH_STORAGE_BYTES};

#[cfg(test)]
use super::{GrainDirection, PitchMode, ResynthControls, TargetSet};
#[cfg(test)]
use crate::dsp::{Complex, fft};
#[cfg(test)]
use grain::{grain_density_count, grain_window_shaped, reset_source_reads, source_reads};
#[cfg(test)]
use shared::{MIDI_ZERO_HZ, grain_antialiased_sample, reflected_integral_prefix};
#[cfg(test)]
use std::f32::consts::TAU;

#[derive(Clone)]
pub enum ProductionResynthArtifact {
    Sample(Box<SampleLoopArtifact>),
    Grain(Box<GrainSourceArtifact>),
    Rich(Box<RichZoneArtifact>),
}
#[cfg(test)]
mod tests {
    use super::*;

    fn broadband_source() -> Vec<f32> {
        (0..24_000)
            .map(|index| {
                let time = index as f32 / RICH_ASSET_SAMPLE_RATE;
                (TAU * 110.0 * time).sin() * 0.4
                    + (TAU * 9_600.0 * time).sin() * 0.2
                    + (TAU * 14_000.0 * time).sin() * 0.1
            })
            .collect()
    }

    #[test]
    fn rich_has_exact_fixed_storage_and_upper_spectrum_at_midi_zero() {
        assert_eq!(RICH_STORAGE_BYTES, 2_883_584);
        assert!(RICH_STORAGE_BYTES < 6 * 1024 * 1024);
        let artifact = RichZoneArtifact::compile(
            &broadband_source(),
            RICH_ASSET_SAMPLE_RATE as u32,
            110.0,
            ResynthControls::default(),
        )
        .expect("rich");
        assert_eq!(artifact.slabs().len(), 22);
        let zone = artifact.zone_for_frequency(MIDI_ZERO_HZ);
        let slab = &artifact.slabs()[zone];
        let mut spectrum = slab
            .iter()
            .map(|sample| Complex::new(f64::from(*sample), 0.0))
            .collect::<Vec<_>>();
        fft(&mut spectrum, false);
        let bin_hz = RICH_ASSET_SAMPLE_RATE / RICH_ZONE_SAMPLES as f32;
        let high = spectrum
            .iter()
            .enumerate()
            .filter(|(bin, _)| {
                let hz = *bin as f32 * bin_hz;
                (8_000.0..=16_000.0).contains(&hz)
            })
            .map(|(_, bin)| bin.re.mul_add(bin.re, bin.im * bin.im))
            .sum::<f64>();
        assert!(high > 1.0e-5, "high-band power {high}");

        let target_hz = MIDI_ZERO_HZ;
        let increment = artifact.phase_increment(zone, target_hz, RICH_ASSET_SAMPLE_RATE);
        let mut phase = 0.0_f32;
        let mut rendered = vec![Complex::ZERO; RICH_ZONE_SAMPLES];
        for sample in &mut rendered {
            sample.re = f64::from(artifact.eval(zone, phase));
            phase = (phase + increment).rem_euclid(1.0);
        }
        fft(&mut rendered, false);
        let rendered_high = rendered
            .iter()
            .enumerate()
            .filter(|(bin, _)| {
                let hz = *bin as f32 * bin_hz;
                (8_000.0..=16_000.0).contains(&hz)
            })
            .map(|(_, bin)| bin.re.mul_add(bin.re, bin.im * bin.im))
            .sum::<f64>();
        assert!(
            rendered_high > 1.0e-5,
            "rendered high-band power {rendered_high}"
        );
    }

    #[test]
    fn sample_position_changes_the_bounded_loop_window() {
        let source = (0..24_000)
            .map(|index| {
                let time = index as f32 / 48_000.0;
                (TAU * 220.0 * time).sin() * 0.7 + index as f32 / 24_000.0 * 0.2
            })
            .collect::<Vec<_>>();
        let start =
            SampleLoopArtifact::compile(&source, 48_000, Some(220.0), 0.0).expect("start loop");
        let end = SampleLoopArtifact::compile(&source, 48_000, Some(220.0), 1.0).expect("end loop");
        assert_eq!(start.frames(), end.frames());
        let difference = start
            .samples()
            .iter()
            .zip(end.samples())
            .map(|(a, b)| (a - b).abs())
            .sum::<f32>();
        assert!(difference > 1.0, "loop position was ignored: {difference}");
    }

    #[test]
    fn grain_ads_envelope_is_silent_at_edges() {
        assert!(grain_window_shaped(0.0, 0.0, 0.2, 0.5, 0.3) < 0.05);
        assert!(grain_window_shaped(1.0, 0.0, 0.2, 0.5, 0.3) < 0.05);
        assert!(grain_window_shaped(0.5, 0.0, 0.2, 0.5, 0.3) > 0.9);
    }

    #[test]
    fn grain_high_density_cloud_stays_bounded() {
        let source = broadband_source();
        let mut controls = ResynthControls::default();
        controls.grain_density = 2_000.0;
        controls.grain_size = 1.0;
        let artifact =
            GrainSourceArtifact::compile(&source, 48_000, None, controls).expect("grain");
        let mut scheduler = GrainSchedulerState::default();
        for frame in 0..2_000_u64 {
            let (left, right) =
                scheduler.render_cloud(&artifact, 110.0, 48_000.0, 1, frame, controls, 0.3, 0.1);
            assert!(left.is_finite() && right.is_finite());
        }
        assert!(scheduler.active_count() < GRAIN_LAYERS);
    }

    #[test]
    fn grain_density_reduces_before_pool_stealing() {
        let source = broadband_source();
        let mut controls = ResynthControls::default();
        controls.grain_density = 2_000.0;
        controls.grain_size = 1.0;
        let artifact =
            GrainSourceArtifact::compile(&source, 48_000, None, controls).expect("grain");
        let mut scheduler = GrainSchedulerState::default();
        for frame in 0..2_000_u64 {
            let _ =
                scheduler.render_cloud(&artifact, 110.0, 48_000.0, 1, frame, controls, 0.3, 0.1);
        }
        assert!(scheduler.active_count() > 0);
        assert!(scheduler.active_count() < GRAIN_LAYERS);
    }

    #[test]
    fn grain_high_unison_cheap_path_is_at_least_four_times_faster() {
        let source = broadband_source();
        let mut controls = ResynthControls::default();
        controls.grain_density = 16.0;
        controls.grain_size = 0.1;
        let artifact =
            GrainSourceArtifact::compile(&source, 48_000, None, controls).expect("grain");
        let mut scheduler = GrainSchedulerState::default();
        for frame in 0..512_u64 {
            let _ =
                scheduler.render_cloud(&artifact, 220.0, 48_000.0, 1, frame, controls, 0.2, 0.0);
        }
        assert!(scheduler.active_count() <= GRAIN_LAYERS);
    }

    #[test]
    fn grain_phase_random_does_not_change_source_spray_when_spray_is_zero() {
        let source = broadband_source();
        let mut controls = ResynthControls {
            grain_density: 24.0,
            grain_size: 0.25,
            grain_spray: 0.0,
            ..ResynthControls::default()
        };
        let artifact =
            GrainSourceArtifact::compile(&source, 48_000, None, controls).expect("grain");
        let mut phase_zero = GrainSchedulerState::default();
        let mut phase_one = GrainSchedulerState::default();
        let mut zero_output = Vec::new();
        let mut one_output = Vec::new();
        reset_source_reads();
        for frame in 0..128_u64 {
            zero_output.push(
                phase_zero.render_cloud(&artifact, 220.0, 48_000.0, 7, frame, controls, 0.5, 0.0),
            );
        }
        let zero_reads = source_reads();
        reset_source_reads();
        for frame in 0..128_u64 {
            one_output.push(
                phase_one.render_cloud(&artifact, 220.0, 48_000.0, 7, frame, controls, 0.5, 1.0),
            );
        }
        assert_eq!(zero_reads, source_reads());
        assert_eq!(zero_output, one_output);

        controls.grain_spray = 1.0;
        let mut sprayed = GrainSchedulerState::default();
        let mut sprayed_output = Vec::new();
        for frame in 0..128_u64 {
            sprayed_output.push(
                sprayed.render_cloud(&artifact, 220.0, 48_000.0, 7, frame, controls, 0.5, 0.0),
            );
        }
        assert_ne!(zero_output, sprayed_output);
    }

    #[test]
    fn grain_tune_endpoints_read_only_one_stereo_source() {
        let source = (0..24_000)
            .map(|index| (TAU * 220.0 * index as f32 / 48_000.0).sin())
            .collect::<Vec<_>>();
        let side = source
            .iter()
            .map(|sample| sample * 0.25)
            .collect::<Vec<_>>();
        let mut controls = ResynthControls {
            grain_density: 1.0,
            grain_size: 1.0,
            ..ResynthControls::default()
        };
        let artifact = GrainSourceArtifact::compile_channels_with_cancel(
            &source,
            Some(&side),
            48_000,
            Some(220.0),
            controls,
            &|| false,
        )
        .expect("grain");
        for tune in [0.0, 1.0] {
            controls.grain_tune = tune;
            let mut scheduler = GrainSchedulerState::default();
            let _ = scheduler.render_cloud(&artifact, 220.0, 48_000.0, 1, 0, controls, 0.5, 0.0);
            reset_source_reads();
            let _ = scheduler.render_cloud(&artifact, 220.0, 48_000.0, 1, 1, controls, 0.5, 0.0);
            assert_eq!(source_reads(), scheduler.active_count() * 2);
        }
    }

    #[test]
    fn grain_pitch_modes_reach_the_prepared_spectral_renderer() {
        let source = (0..24_000)
            .map(|index| {
                let phase = TAU * 220.0 * index as f32 / 48_000.0;
                phase.sin() * 0.7 + (phase * 2.0).sin() * 0.2
            })
            .collect::<Vec<_>>();
        let artifact =
            GrainSourceArtifact::compile(&source, 48_000, Some(220.0), ResynthControls::default())
                .expect("grain");
        let render = |mode| {
            let mut controls = ResynthControls::default();
            controls.pitch_mode = mode;
            controls.grain_tune = 1.0;
            let mut scheduler = GrainSchedulerState::default();
            (0..512_u64)
                .map(|frame| {
                    scheduler.render_cloud(
                        &artifact,
                        330.0,
                        48_000.0,
                        41,
                        frame,
                        controls,
                        controls.position,
                        0.0,
                    )
                })
                .collect::<Vec<_>>()
        };
        let classic = render(PitchMode::Classic);
        let spectral = render(PitchMode::Spectral);
        let target = render(PitchMode::Target(TargetSet::PlayedNote));
        assert!(
            classic
                .iter()
                .all(|sample| sample.0.is_finite() && sample.1.is_finite())
        );
        assert!(
            spectral
                .iter()
                .all(|sample| sample.0.is_finite() && sample.1.is_finite())
        );
        assert!(
            target
                .iter()
                .all(|sample| sample.0.is_finite() && sample.1.is_finite())
        );
        assert_ne!(classic, spectral);
        assert_ne!(classic, target);
    }

    #[test]
    fn grain_spectral_tune_collapses_every_source_octave_to_the_exact_root_octave() {
        const SAMPLE_RATE: f32 = 48_000.0;
        const SEGMENT: usize = 12_000;
        const ROOT: f32 = 440.0;
        let pitches = [110.0_f32, 220.0, 440.0, 880.0];
        let source = pitches
            .iter()
            .flat_map(|pitch| {
                (0..SEGMENT).map(move |index| {
                    let phase = TAU * *pitch * index as f32 / SAMPLE_RATE;
                    phase.sin() * 0.08 + (phase * 2.0).sin() * 0.72 + (phase * 3.0).sin() * 0.2
                })
            })
            .collect::<Vec<_>>();
        let artifact = GrainSourceArtifact::compile(
            &source,
            SAMPLE_RATE as u32,
            Some(ROOT),
            ResynthControls::default(),
        )
        .expect("spectral grain");
        let amplitude = |samples: &[f32], hz: f32| {
            let (mut re, mut im) = (0.0_f64, 0.0_f64);
            for (index, sample) in samples.iter().copied().enumerate() {
                let phase = f64::from(TAU * hz * index as f32 / SAMPLE_RATE);
                re += f64::from(sample) * phase.cos();
                im -= f64::from(sample) * phase.sin();
            }
            re.hypot(im)
        };
        for (segment, source_pitch) in pitches.into_iter().enumerate() {
            let start = segment * SEGMENT + 2_048;
            let end = (segment + 1) * SEGMENT - 2_048;
            let tuned = &artifact.tuned_samples[start..end];
            let strongest_second = (1_600..=1_920)
                .map(|step| step as f32 * 0.5)
                .max_by(|left, right| amplitude(tuned, *left).total_cmp(&amplitude(tuned, *right)))
                .unwrap_or(0.0);
            let detected = strongest_second * 0.5;
            let cents = 1_200.0 * (detected / ROOT).log2();
            assert!(
                cents.abs() < 15.0,
                "source {source_pitch} Hz tuned to {detected} Hz ({cents} cents)"
            );
        }
    }

    #[test]
    fn grain_density_count_clamps_to_pool() {
        let mut controls = ResynthControls::default();
        controls.grain_density = 1.0;
        assert_eq!(grain_density_count(controls), 1);
        controls.grain_density = 100.0;
        assert_eq!(grain_density_count(controls), GRAIN_LAYERS);
        controls.grain_density = 250.0;
        assert_eq!(grain_density_count(controls), GRAIN_LAYERS);
    }

    #[test]
    fn grain_reverse_and_hold_keep_default_spawn_center() {
        let source = broadband_source();
        let artifact =
            GrainSourceArtifact::compile(&source, 48_000, None, ResynthControls::default())
                .expect("grain");
        let mut forward = GrainSchedulerState::default();
        let mut reversed = GrainSchedulerState::default();
        let mut reverse = ResynthControls::default();
        reverse.grain_direction = GrainDirection::Backward as u8;
        let _ = forward.render(&artifact, 110.0, 48_000.0, 3, 0);
        let _ = reversed.render_lane_with(&artifact, 110.0, 48_000.0, 3, 0, 0, reverse, 1);
        let fwd = forward
            .layers
            .iter()
            .find(|layer| layer.active)
            .expect("forward grain");
        let back = reversed
            .layers
            .iter()
            .find(|layer| layer.active)
            .expect("reversed grain");
        assert!(fwd.source_step > 0.0);
        assert!(back.source_step < 0.0);
    }

    #[test]
    fn grain_scheduler_is_deterministic_and_evolves_beyond_one_period() {
        let source = broadband_source();
        let artifact =
            GrainSourceArtifact::compile(&source, 48_000, None, ResynthControls::default())
                .expect("grain");
        let mut first = GrainSchedulerState::default();
        let mut second = GrainSchedulerState::default();
        let a = (0..10_000)
            .map(|frame| first.render(&artifact, 110.0, 48_000.0, 7, frame).to_bits())
            .collect::<Vec<_>>();
        let b = (0..10_000)
            .map(|frame| {
                second
                    .render(&artifact, 110.0, 48_000.0, 7, frame)
                    .to_bits()
            })
            .collect::<Vec<_>>();
        assert_eq!(a, b);
        assert_ne!(&a[..2_048], &a[2_048..4_096]);
    }

    #[test]
    fn sample_loop_wrap_is_continuous_for_many_recurrences() {
        let source = broadband_source();
        let artifact =
            SampleLoopArtifact::compile(&source, 48_000, Some(110.0), 0.5).expect("sample");
        let before = artifact.eval(1.0 - 1.0 / artifact.frames() as f32);
        let after = artifact.eval(0.0);
        assert!((after - before).abs() < 0.2, "{}", (after - before).abs());
        assert!(artifact.samples().iter().all(|sample| sample.is_finite()));
    }
}

// RESYNTH_ARTIFACT_FOCUSED_REGRESSIONS
#[cfg(test)]
mod focused_regression_tests {
    use super::*;

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len().max(1) as f32)
            .sqrt()
    }

    #[test]
    fn worker_artifact_compilers_observe_cancellation() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let source = vec![0.25_f32; 192_000];
        let polls = AtomicUsize::new(0);
        let grain = GrainSourceArtifact::compile_with_cancel(
            &source,
            48_000,
            Some(220.0),
            ResynthControls::default(),
            &|| polls.fetch_add(1, Ordering::Relaxed) >= 2,
        );
        assert!(matches!(grain, Err(ArtifactBuildError::Cancelled)));

        let polls = AtomicUsize::new(0);
        let sample =
            SampleLoopArtifact::compile_with_cancel(&source, 48_000, Some(220.0), 0.5, &|| {
                polls.fetch_add(1, Ordering::Relaxed) >= 2
            });
        assert!(matches!(sample, Err(ArtifactBuildError::Cancelled)));

        let polls = AtomicUsize::new(0);
        let audition = SourceAuditionArtifact::compile_with_cancel(&source, 48_000, &|| {
            polls.fetch_add(1, Ordering::Relaxed) >= 2
        });
        assert!(matches!(audition, Err(ArtifactBuildError::Cancelled)));

        let rich = RichZoneArtifact::compile_with_cancel(
            &source,
            48_000,
            220.0,
            ResynthControls::default(),
            &|| true,
        );
        assert!(matches!(rich, Err(ArtifactBuildError::Cancelled)));
    }

    #[test]
    fn hostile_finite_pcm_is_rejected_before_artifact_math() {
        let hostile = vec![f32::MAX; 65_536];
        assert!(matches!(
            SourceAuditionArtifact::compile(&hostile, 48_000),
            Err(ArtifactBuildError::NonFinite)
        ));
        assert!(matches!(
            SampleLoopArtifact::compile(&hostile, 48_000, Some(220.0), 0.5),
            Err(ArtifactBuildError::NonFinite)
        ));
        assert!(matches!(
            GrainSourceArtifact::compile(&hostile, 48_000, Some(220.0), ResynthControls::default(),),
            Err(ArtifactBuildError::NonFinite)
        ));
        assert!(matches!(
            RichZoneArtifact::compile(&hostile, 48_000, 220.0, ResynthControls::default()),
            Err(ArtifactBuildError::NonFinite)
        ));
    }

    #[test]
    fn sample_position_materially_changes_loop_start_and_output() {
        let source = (0..96_000)
            .map(|index| {
                let time = index as f32 / 48_000.0;
                if index < 48_000 {
                    (TAU * 220.0 * time).sin() * 0.75 + (TAU * 440.0 * time).sin() * 0.2
                } else {
                    (TAU * 880.0 * time).sin() * 0.6 + (TAU * 1_320.0 * time).sin() * 0.3
                }
            })
            .collect::<Vec<_>>();
        let early = SampleLoopArtifact::compile(&source, 48_000, Some(220.0), 0.0).expect("early");
        let late = SampleLoopArtifact::compile(&source, 48_000, Some(220.0), 1.0).expect("late");

        assert_eq!(early.source_total_frames(), source.len());
        assert_eq!(late.source_total_frames(), source.len());
        assert_eq!(early.source_span_frames(), late.source_span_frames());
        assert_eq!(early.crossfade_frames(), late.crossfade_frames());
        assert!(early.crossfade_frames() > 0);
        assert!(
            late.source_start_frames() - early.source_start_frames() > source.len() / 8,
            "POSITION selected nearly the same source start: {} vs {}",
            early.source_start_frames(),
            late.source_start_frames()
        );

        let difference = early
            .samples()
            .iter()
            .zip(late.samples())
            .map(|(a, b)| (a - b) * (a - b))
            .sum::<f32>();
        let difference_rms = (difference / early.frames() as f32).sqrt();
        assert!(
            difference_rms > 0.2,
            "POSITION produced nearly the same loop: difference RMS {difference_rms}"
        );
    }

    #[test]
    fn legal_384khz_twenty_hz_sample_quantizes_to_complete_source_periods() {
        const SOURCE_RATE: u32 = 384_000;
        const ROOT_HZ: f32 = 20.0;
        let source = (0..150_000)
            .map(|index| (TAU * ROOT_HZ * index as f32 / SOURCE_RATE as f32).sin() * 0.8)
            .collect::<Vec<_>>();
        let compiled = SampleLoopArtifact::compile(&source, SOURCE_RATE, Some(ROOT_HZ), 0.5)
            .expect("legal endpoint loop");

        let source_period = SOURCE_RATE as f32 / ROOT_HZ;
        let retained_periods = compiled.frames() as f32 / source_period;
        assert_eq!(compiled.source_sample_rate, SOURCE_RATE as f32);
        assert_eq!(compiled.frames(), 96_000);
        assert_eq!(retained_periods, 5.0);
        let source_frames_per_output =
            compiled.phase_increment(ROOT_HZ, 48_000.0) * compiled.frames() as f32;
        assert_eq!(source_frames_per_output, 8.0);
    }

    #[test]
    fn short_low_root_sample_uses_one_effective_overlap_and_round_trips_receipt() {
        let source = (0..1_024)
            .map(|index| (TAU * 20.0 * index as f32 / 48_000.0).sin())
            .collect::<Vec<_>>();
        let compiled =
            SampleLoopArtifact::compile(&source, 48_000, Some(20.0), 0.75).expect("short loop");

        assert_eq!(compiled.frames(), 512);
        assert_eq!(compiled.crossfade_frames(), compiled.frames());
        assert_eq!(
            compiled.source_span_frames(),
            compiled.frames() + compiled.crossfade_frames()
        );
        assert!(
            compiled.source_start_frames() + compiled.source_span_frames()
                <= compiled.source_total_frames()
        );
        let restored = SampleLoopArtifact::from_persisted_with_receipt(
            compiled.source_sample_rate,
            compiled.root_hz,
            compiled.samples.clone(),
            compiled.source_start_frames(),
            compiled.source_span_frames(),
            compiled.source_total_frames(),
            compiled.crossfade_frames(),
        )
        .expect("exact receipt");
        assert_eq!(restored.samples(), compiled.samples());
        assert_eq!(
            (
                restored.source_start_frames(),
                restored.source_span_frames(),
                restored.source_total_frames(),
                restored.crossfade_frames(),
            ),
            (
                compiled.source_start_frames(),
                compiled.source_span_frames(),
                compiled.source_total_frames(),
                compiled.crossfade_frames(),
            )
        );
    }

    #[test]
    fn grain_end_position_high_pitch_keeps_moving_and_bounds_alias_energy() {
        let mut moving_source = (0..32_769)
            .map(|index| {
                let time = index as f32 / 48_000.0;
                (TAU * 317.0 * time).sin() * 0.7 + (TAU * 911.0 * time).sin() * 0.2
            })
            .collect::<Vec<_>>();
        moving_source[32_257..].fill(0.75);
        let controls = ResynthControls {
            position: 1.0,
            grain_size: 0.05,
            grain_density: 1.0,
            grain_spray: 0.0,
            ..ResynthControls::default()
        };
        let artifact = GrainSourceArtifact::compile(&moving_source, 48_000, Some(220.0), controls)
            .expect("moving grain");
        let mut scheduler = GrainSchedulerState::default();
        let mut rendered = Vec::with_capacity(2_048);
        let mut minimum_read_position = 1.0_f32;
        for frame in 0..2_048_u64 {
            rendered.push(scheduler.render(&artifact, 1_760.0, 48_000.0, 9, frame));
            let mut positions = [0.0; GRAIN_TELEMETRY];
            let mut progress = [0.0; GRAIN_TELEMETRY];
            let mut gains = [0.0; GRAIN_TELEMETRY];
            let active_mask = scheduler.write_telemetry(
                artifact.samples.len(),
                &mut positions,
                &mut progress,
                &mut gains,
            );
            for (index, (position, gain)) in positions.into_iter().zip(gains).enumerate() {
                if active_mask & (1_u8 << index) != 0 && gain > 0.01 {
                    minimum_read_position = minimum_read_position.min(position);
                }
            }
        }
        let difference_rms = rms(&rendered
            .windows(2)
            .map(|pair| pair[1] - pair[0])
            .collect::<Vec<_>>());
        assert!(
            minimum_read_position < 0.95,
            "position=1 grain held the final source frame: minimum {minimum_read_position}"
        );
        assert!(
            difference_rms > 0.01,
            "high-pitch grain collapsed to a held tail: difference RMS {difference_rms}"
        );

        // An alternating source is entirely above the output passband at an
        // 8x read ratio. It must not fold into a loud held/low-frequency tone.
        let alternating = (0..32_769)
            .map(|index| if index & 1 == 0 { 1.0 } else { -1.0 })
            .collect::<Vec<_>>();
        let alias_controls = ResynthControls {
            position: 0.5,
            grain_size: 0.05,
            grain_density: 0.0,
            grain_spray: 0.0,
            ..ResynthControls::default()
        };
        let alias_artifact =
            GrainSourceArtifact::compile(&alternating, 48_000, Some(220.0), alias_controls)
                .expect("alias probe");
        let mut alias_scheduler = GrainSchedulerState::default();
        let aliased = (0..4_096_u64)
            .map(|frame| alias_scheduler.render(&alias_artifact, 1_760.0, 48_000.0, 11, frame))
            .collect::<Vec<_>>();
        assert!(aliased.iter().all(|sample| sample.is_finite()));
        assert!(
            aliased.iter().copied().map(f32::abs).fold(0.0, f32::max) <= 1.0,
            "high-ratio grain exceeded the source bound"
        );
        assert!(
            rms(&aliased) < 0.08,
            "8x Nyquist probe folded excessive energy: RMS {}",
            rms(&aliased)
        );
    }

    #[test]
    fn zero_window_grain_admission_does_not_rescale_existing_layers() {
        let source = (0..65_536)
            .map(|index| (TAU * 137.0 * index as f32 / 48_000.0).sin() * 0.8)
            .collect::<Vec<_>>();
        let controls = ResynthControls {
            position: 0.5,
            grain_size: 1.0,
            grain_density: 1.0,
            grain_spray: 0.0,
            ..ResynthControls::default()
        };
        let artifact = GrainSourceArtifact::compile(&source, 48_000, Some(220.0), controls)
            .expect("normalization grain");
        let mut scheduler = GrainSchedulerState::default();
        let mut heard = false;
        for frame in 0..20_000_u64 {
            let sample = scheduler.render(&artifact, 220.0, 48_000.0, 43, frame);
            assert!(sample.is_finite());
            heard |= sample.abs() > 0.05;
        }
        assert!(heard, "grain produced no audible output");
    }

    #[test]
    fn grain_pool_stays_at_requested_density() {
        let source = (0..65_536)
            .map(|index| (TAU * 220.0 * index as f32 / 48_000.0).sin() * 0.8)
            .collect::<Vec<_>>();
        let mut controls = ResynthControls::default();
        controls.grain_density = 1_000.0;
        controls.grain_size = 0.2;
        let artifact = GrainSourceArtifact::compile(&source, 48_000, Some(220.0), controls)
            .expect("density grain");
        let mut scheduler = GrainSchedulerState::default();
        for frame in 0..384_u64 {
            let _ =
                scheduler.render_cloud(&artifact, 220.0, 48_000.0, 41, frame, controls, 0.5, 0.0);
        }
        assert_eq!(scheduler.active_count(), 8);
    }

    #[test]
    fn grain_max_size_and_density_never_evicts_a_live_layer() {
        let source = (0..65_536)
            .map(|index| (TAU * 220.0 * index as f32 / 48_000.0).sin() * 0.8)
            .collect::<Vec<_>>();
        let controls = ResynthControls {
            position: 0.5,
            grain_size: 1.0,
            grain_density: 1.0,
            grain_spray: 0.0,
            ..ResynthControls::default()
        };
        let artifact = GrainSourceArtifact::compile(&source, 48_000, Some(220.0), controls)
            .expect("maximum-density grain");
        let mut scheduler = GrainSchedulerState::default();
        for frame in 0..2_000_u64 {
            let output = scheduler.render(&artifact, 220.0, 48_000.0, 41, frame);
            assert!(output.is_finite());
            assert!(scheduler.active_count() <= 1);
        }
    }

    #[test]
    fn grain_cloud_is_stereo_when_panned() {
        let source = (0..8_192)
            .map(|index| (TAU * 220.0 * index as f32 / 48_000.0).sin() * 0.8)
            .collect::<Vec<_>>();
        let mut controls = ResynthControls::default();
        controls.grain_density = 4.0;
        controls.grain_size = 0.2;
        controls.grain_pan_spread = 1.0;
        let artifact = GrainSourceArtifact::compile(&source, 48_000, Some(220.0), controls)
            .expect("pan grain");
        let mut scheduler = GrainSchedulerState::default();
        let mut left_power = 0.0_f64;
        let mut right_power = 0.0_f64;
        for frame in 0..4_000_u64 {
            let (left, right) =
                scheduler.render_cloud(&artifact, 220.0, 48_000.0, 11, frame, controls, 0.4, 0.2);
            left_power += f64::from(left * left);
            right_power += f64::from(right * right);
        }
        assert!(left_power > 0.0 && right_power > 0.0);
    }

    #[test]
    #[cfg(any())]
    fn grain_modulated_detuned_lane_positions_integrate_without_retroactive_jumps() {
        let source = (0..65_536)
            .map(|index| {
                let time = index as f32 / 48_000.0;
                (TAU * 220.0 * time).sin() * 0.7 + (TAU * 440.0 * time).sin() * 0.2
            })
            .collect::<Vec<_>>();
        let controls = ResynthControls {
            position: 0.5,
            grain_size: 1.0,
            grain_density: 0.0,
            grain_spray: 0.0,
            ..ResynthControls::default()
        };
        let artifact = GrainSourceArtifact::compile(&source, 48_000, Some(220.0), controls)
            .expect("modulated unison grain");
        let mut scheduler = GrainSchedulerState::default();
        for frame in 0..4_000_u64 {
            scheduler.render_lane(&artifact, 220.0, 48_000.0, 29, frame, 0);
            scheduler.render_lane(&artifact, 222.2, 48_000.0, 29, frame, 1);
        }
        let active = scheduler
            .layers
            .iter()
            .position(|layer| layer.active)
            .expect("active layer");
        let before = scheduler.layers[active].lane_positions[1];
        scheduler.render_lane(&artifact, 440.0, 48_000.0, 29, 4_000, 0);
        scheduler.render_lane(&artifact, 444.4, 48_000.0, 29, 4_000, 1);
        let after = scheduler.layers[active].lane_positions[1];
        let abrupt_step = after - before;
        assert!(
            (1.9..=2.2).contains(&abrupt_step),
            "detuned lane retroactively jumped {abrupt_step} source frames"
        );

        let mut prior = after;
        let mut step_sum = 0.0_f32;
        let mut maximum_step = 0.0_f32;
        for offset in 1..=1_000_u64 {
            let base_ratio = 2.0 + 0.5 * offset as f32 / 1_000.0;
            let frame = 4_000 + offset;
            scheduler.render_lane(&artifact, 220.0 * base_ratio, 48_000.0, 29, frame, 0);
            scheduler.render_lane(&artifact, 220.0 * base_ratio * 1.01, 48_000.0, 29, frame, 1);
            let position = scheduler.layers[active].lane_positions[1];
            let step = position - prior;
            maximum_step = maximum_step.max(step);
            step_sum += step;
            prior = position;
        }
        let mean_step = step_sum / 1_000.0;
        assert!(
            maximum_step < 2.6,
            "detuned ramp step overshot: {maximum_step}"
        );
        assert!(
            (2.2..=2.35).contains(&mean_step),
            "detuned ramp mean step was {mean_step}"
        );
        assert_eq!(scheduler.layers[active].age, 5_000);
    }

    #[test]
    #[cfg(any())]
    fn grain_same_frame_unison_targets_reinterpret_without_advancing_scheduler() {
        let source = (0..65_536)
            .map(|index| {
                let time = index as f32 / 48_000.0;
                (TAU * 220.0 * time).sin() * 0.7 + (TAU * 660.0 * time).sin() * 0.2
            })
            .collect::<Vec<_>>();
        let controls = ResynthControls {
            position: 0.5,
            grain_size: 1.0,
            grain_density: 0.0,
            grain_spray: 0.0,
            ..ResynthControls::default()
        };
        let artifact = GrainSourceArtifact::compile(&source, 48_000, Some(220.0), controls)
            .expect("unison grain");
        let mut scheduler = GrainSchedulerState::default();
        for frame in 0..2_000_u64 {
            scheduler.render_lane(&artifact, 198.0, 48_000.0, 23, frame, 0);
            scheduler.render_lane(&artifact, 242.0, 48_000.0, 23, frame, 1);
        }
        let lower = scheduler.render_lane(&artifact, 198.0, 48_000.0, 23, 2_000, 0);
        let ages = scheduler.layers.map(|layer| layer.age);
        let lane_zero_positions = scheduler
            .layers
            .map(|layer| layer.lane_positions[0].to_bits());
        let event = scheduler.event;
        let until_spawn = scheduler.until_spawn;
        let upper = scheduler.render_lane(&artifact, 242.0, 48_000.0, 23, 2_000, 1);

        assert!(lower.is_finite() && upper.is_finite());
        assert_ne!(
            lower.to_bits(),
            upper.to_bits(),
            "unison lanes were identical"
        );
        assert_eq!(scheduler.layers.map(|layer| layer.age), ages);
        assert_eq!(
            scheduler
                .layers
                .map(|layer| layer.lane_positions[0].to_bits()),
            lane_zero_positions
        );
        assert_eq!(scheduler.event, event);
        assert_eq!(scheduler.until_spawn, until_spawn);
        assert_eq!(scheduler.cached_frame, 2_000);

        let all_positions = scheduler
            .layers
            .map(|layer| layer.lane_positions.map(f32::to_bits));
        let upper_again = scheduler.render_lane(&artifact, 242.0, 48_000.0, 23, 2_000, 1);
        assert_eq!(upper_again.to_bits(), upper.to_bits());
        assert_eq!(
            scheduler
                .layers
                .map(|layer| layer.lane_positions.map(f32::to_bits)),
            all_positions
        );
    }

    #[test]
    #[cfg(any())]
    fn grain_live_pitch_changes_integrate_without_retroactive_position_jumps() {
        let source = (0..65_536)
            .map(|index| (TAU * 220.0 * index as f32 / 48_000.0).sin() * 0.8)
            .collect::<Vec<_>>();
        let controls = ResynthControls {
            position: 0.5,
            grain_size: 1.0,
            grain_density: 0.0,
            grain_spray: 0.0,
            ..ResynthControls::default()
        };
        let artifact = GrainSourceArtifact::compile(&source, 48_000, Some(220.0), controls)
            .expect("pitched grain");
        let mut scheduler = GrainSchedulerState::default();
        let source_max = artifact.samples.len().saturating_sub(1) as f32;
        let read_position = |scheduler: &GrainSchedulerState| {
            let mut positions = [0.0; GRAIN_TELEMETRY];
            let mut progress = [0.0; GRAIN_TELEMETRY];
            let mut gains = [0.0; GRAIN_TELEMETRY];
            let active_mask = scheduler.write_telemetry(
                artifact.samples.len(),
                &mut positions,
                &mut progress,
                &mut gains,
            );
            let lane = active_mask.trailing_zeros() as usize;
            assert!(lane < GRAIN_LAYERS, "no active grain");
            positions[lane] * source_max
        };

        let mut previous_sample = 0.0_f32;
        for frame in 0..4_000_u64 {
            previous_sample = scheduler.render(&artifact, 220.0, 48_000.0, 17, frame);
        }
        let before = read_position(&scheduler);
        let after_sample = scheduler.render(&artifact, 222.2, 48_000.0, 17, 4_000);
        let after = read_position(&scheduler);
        let abrupt_step = after - before;
        assert!(
            (0.9..=1.2).contains(&abrupt_step),
            "live bend retroactively jumped {abrupt_step} source frames"
        );
        assert!(
            (after_sample - previous_sample).abs() < 0.2,
            "live bend caused an output discontinuity: {}",
            (after_sample - previous_sample).abs()
        );

        let mut prior = after;
        let mut step_sum = 0.0_f32;
        let mut maximum_step = 0.0_f32;
        for offset in 1..=1_000_u64 {
            let ratio = 1.01 + 0.1 * offset as f32 / 1_000.0;
            scheduler.render(&artifact, 220.0 * ratio, 48_000.0, 17, 4_000 + offset);
            let position = read_position(&scheduler);
            let step = position - prior;
            maximum_step = maximum_step.max(step);
            step_sum += step;
            prior = position;
        }
        let mean_step = step_sum / 1_000.0;
        assert!(
            maximum_step < 1.2,
            "pitch ramp overshot its requested read increment: {maximum_step}"
        );
        assert!(
            (1.04..=1.08).contains(&mean_step),
            "pitch ramp mean read increment was {mean_step}, expected about 1.06"
        );
    }

    #[test]
    fn legal_sixteen_x_sample_and_grain_reads_remain_audible() {
        let source = (0..65_536)
            .map(|index| (TAU * 220.0 * index as f32 / 384_000.0).sin() * 0.8)
            .collect::<Vec<_>>();

        let sample =
            SampleLoopArtifact::compile(&source, 384_000, Some(220.0), 0.5).expect("sample");
        let sample_increment = sample.phase_increment(440.0, 48_000.0);
        let sample_rate = sample_increment * sample.frames() as f32;
        assert!((sample_rate - 16.0).abs() < 1.0e-4);
        let mut sample_phase = 0.0_f32;
        let sample_render = (0..4_096)
            .map(|_| {
                let output = sample.eval_bandlimited(sample_phase, sample_rate);
                sample_phase = (sample_phase + sample_increment).rem_euclid(1.0);
                output
            })
            .collect::<Vec<_>>();
        assert!(
            rms(&sample_render) > 0.1,
            "legal 16x Sample read was muted: RMS {}",
            rms(&sample_render)
        );

        let controls = ResynthControls {
            position: 0.5,
            grain_size: 0.05,
            grain_density: 0.0,
            grain_spray: 0.0,
            ..ResynthControls::default()
        };
        let grain =
            GrainSourceArtifact::compile(&source, 384_000, Some(220.0), controls).expect("grain");
        let mut scheduler = GrainSchedulerState::default();
        let grain_render = (0..4_096_u64)
            .map(|frame| scheduler.render(&grain, 440.0, 48_000.0, 13, frame))
            .collect::<Vec<_>>();
        assert!(
            rms(&grain_render) > 0.02,
            "legal 16x Grain read was muted: RMS {}",
            rms(&grain_render)
        );
    }

    #[test]
    fn odd_periodic_mips_preserve_loop_extent_pitch_and_seam() {
        const FRAMES: usize = 19_199;
        const BIN: usize = 19;
        const RATE: f32 = 32.0;
        let source = (0..FRAMES)
            .map(|index| (TAU * BIN as f32 * index as f32 / FRAMES as f32).sin() * 0.8)
            .collect::<Vec<_>>();
        let sample =
            SampleLoopArtifact::from_persisted(384_000.0, 220.0, source.into_boxed_slice());
        let seam_left = sample.eval_bandlimited(1.0 - 1.0e-6, RATE);
        let seam_right = sample.eval_bandlimited(1.0e-6, RATE);
        assert!(
            (seam_left - seam_right).abs() < 0.01,
            "odd mip seam drifted"
        );

        let mut phase = 0.0_f32;
        let rendered = (0..4_096)
            .map(|_| {
                let output = sample.eval_bandlimited(phase, RATE);
                phase = (phase + RATE / FRAMES as f32).rem_euclid(1.0);
                output
            })
            .collect::<Vec<_>>();
        let expected_cycles_per_output = BIN as f32 * RATE / FRAMES as f32;
        assert!(
            tone_amplitude(&rendered, expected_cycles_per_output, 1.0) > 0.7,
            "odd mip changed pitch or attenuated its low band"
        );
    }

    #[test]
    fn short_terminal_mips_average_high_bands_without_a_hard_mute() {
        const FRAMES: usize = 256;
        const RATE: f32 = 1_024.0;
        let dc = vec![0.75_f32; FRAMES];
        let alternating = (0..FRAMES)
            .map(|index| if index & 1 == 0 { 0.75 } else { -0.75 })
            .collect::<Vec<_>>();
        for (source_index, source) in [&dc, &alternating].into_iter().enumerate() {
            let sample = SampleLoopArtifact::from_persisted(
                48_000.0,
                220.0,
                source.clone().into_boxed_slice(),
            );
            let grain = GrainSourceArtifact::from_persisted(
                48_000.0,
                Some(220.0),
                ResynthControls::default(),
                source.clone().into_boxed_slice(),
                Vec::new().into_boxed_slice(),
            );
            let audition = SourceAuditionArtifact::compile(source, 48_000).expect("audition");
            let outputs = [
                sample.eval_bandlimited(0.25, RATE),
                grain.sample_filtered(100.0, RATE),
                audition.sample_one_shot_filtered(100.0, RATE),
            ];
            assert!(outputs.iter().all(|sample| sample.is_finite()));
            if source_index == 0 {
                assert!(outputs[0].abs() > 0.5 && outputs[1].abs() > 0.5);
            } else {
                assert!(outputs.iter().all(|sample| sample.abs() < 0.02));
            }
        }
    }

    #[test]
    fn persisted_pcm_regenerates_identical_prepared_pitch_frames() {
        let source = (0..32_768)
            .map(|index| (TAU * 220.0 * index as f32 / 48_000.0).sin() * 0.8)
            .collect::<Vec<_>>();
        let controls = ResynthControls::default();
        let fresh = GrainSourceArtifact::compile(&source, 48_000, Some(220.0), controls)
            .expect("fresh grain");
        let restored = GrainSourceArtifact::from_persisted(
            fresh.source_sample_rate,
            fresh.root_hz,
            controls,
            fresh.samples.clone(),
            fresh.transients.clone(),
        );
        assert_eq!(fresh.pitch_frames, restored.pitch_frames);
        assert_eq!(
            fresh.pitch_frames.frame_at(0.5),
            restored.pitch_frames.frame_at(0.5)
        );
    }

    #[test]
    fn persisted_pcm_regenerates_identical_mips_and_outputs() {
        let source = (0..65_536)
            .map(|index| {
                (TAU * 0.0037 * index as f32).sin() * 0.6 + (TAU * 0.071 * index as f32).sin() * 0.2
            })
            .collect::<Vec<_>>();
        let fresh_sample =
            SampleLoopArtifact::compile(&source, 48_000, Some(220.0), 0.4).expect("sample");
        let restored_sample = SampleLoopArtifact::from_persisted(
            fresh_sample.source_sample_rate,
            fresh_sample.root_hz,
            fresh_sample.samples.clone(),
        );
        assert!(
            fresh_sample
                .periodic_mips
                .iter()
                .map(|level| level.samples.len())
                .sum::<usize>()
                < fresh_sample.samples.len()
        );
        for rate in [1.0_f32, 1.7, 2.0, 31.9, 32.0, 1_024.0] {
            for phase in [0.0_f32, 0.173, 0.499, 0.999] {
                assert_eq!(
                    fresh_sample.eval_bandlimited(phase, rate).to_bits(),
                    restored_sample.eval_bandlimited(phase, rate).to_bits()
                );
            }
        }

        let fresh_grain =
            GrainSourceArtifact::compile(&source, 48_000, Some(220.0), ResynthControls::default())
                .expect("grain");
        let restored_grain = GrainSourceArtifact::from_persisted(
            fresh_grain.source_sample_rate,
            fresh_grain.root_hz,
            fresh_grain.controls,
            fresh_grain.samples.clone(),
            fresh_grain.transients.clone(),
        );
        assert!(
            fresh_grain
                .reflected_mips
                .iter()
                .map(|level| level.samples.len())
                .sum::<usize>()
                < fresh_grain.samples.len()
        );
        for rate in [1.0_f32, 1.7, 2.0, 31.9, 32.0, 1_024.0] {
            for position in [0.0_f32, 1_337.25, 32_768.5, 65_000.0] {
                assert_eq!(
                    fresh_grain.sample_filtered(position, rate).to_bits(),
                    restored_grain.sample_filtered(position, rate).to_bits()
                );
            }
        }
    }

    #[test]
    fn dyadic_mips_reject_resurgent_and_higher_images_for_all_pcm_readers() {
        const RATE: f32 = 32.0;
        const FRAMES: usize = 131_072;
        const OUTPUT_FRAMES: usize = 2_048;
        const START: f32 = 16_384.0;

        for destination_frequency in [0.64_f32, 1.70, 2.64, 4.70, 8.64] {
            let source_bin = (destination_frequency * FRAMES as f32 / RATE).round() as usize;
            let source = (0..FRAMES)
                .map(|index| (TAU * source_bin as f32 * index as f32 / FRAMES as f32).sin())
                .collect::<Vec<_>>();
            let sample = SampleLoopArtifact::from_persisted(
                384_000.0,
                220.0,
                source.clone().into_boxed_slice(),
            );
            let grain = GrainSourceArtifact::from_persisted(
                384_000.0,
                Some(220.0),
                ResynthControls::default(),
                source.clone().into_boxed_slice(),
                Vec::new().into_boxed_slice(),
            );
            let audition = SourceAuditionArtifact::compile(&source, 384_000).expect("audition");
            let mut phase = START / FRAMES as f32;
            let mut sample_render = Vec::with_capacity(OUTPUT_FRAMES);
            let mut grain_render = Vec::with_capacity(OUTPUT_FRAMES);
            let mut source_render = Vec::with_capacity(OUTPUT_FRAMES);
            for frame in 0..OUTPUT_FRAMES {
                let position = START + frame as f32 * RATE;
                sample_render.push(sample.eval_bandlimited(phase, RATE));
                grain_render.push(grain.sample_filtered(position, RATE));
                source_render.push(audition.sample_one_shot_filtered(f64::from(position), RATE));
                phase = (phase + RATE / FRAMES as f32).rem_euclid(1.0);
            }
            for (reader, rendered) in [
                ("Sample", sample_render.as_slice()),
                ("Grain", grain_render.as_slice()),
                ("Source", source_render.as_slice()),
            ] {
                assert!(
                    rms(rendered) < 0.01,
                    "{reader} leaked F={destination_frequency} at RMS {}",
                    rms(rendered)
                );
            }
        }
    }

    #[test]
    fn dyadic_boundary_retains_low_passband_and_is_continuous() {
        const FRAMES: usize = 65_536;
        for boundary in [2.0_f32, 4.0, 8.0, 32.0, 1_024.0] {
            let source_bin = (0.10 * FRAMES as f32 / boundary).round().max(1.0) as usize;
            let source = (0..FRAMES)
                .map(|index| (TAU * source_bin as f32 * index as f32 / FRAMES as f32).sin() * 0.8)
                .collect::<Vec<_>>();
            let sample =
                SampleLoopArtifact::from_persisted(384_000.0, 220.0, source.into_boxed_slice());
            let phase = 0.25 / source_bin as f32;
            let below = sample.eval_bandlimited(phase, boundary * (1.0 - 1.0e-5));
            let at = sample.eval_bandlimited(phase, boundary);
            let above = sample.eval_bandlimited(phase, boundary * (1.0 + 1.0e-5));
            assert!(below.is_finite() && at.is_finite() && above.is_finite());
            assert!(
                (below - at).abs() < 0.01,
                "click below {boundary}: {below} vs {at}"
            );
            assert!(
                (above - at).abs() < 0.01,
                "click above {boundary}: {above} vs {at}"
            );
            assert!(at.abs() > 0.5, "low passband muted at {boundary}: {at}");
        }
    }

    #[test]
    fn rate_thirty_two_integrated_fir_rejects_just_above_nyquist_and_retains_passband() {
        const RATE: f32 = 32.0;
        const FRAMES: usize = 131_072;
        const OUTPUT_FRAMES: usize = 2_048;
        const START: f32 = 8_192.0;
        // F = source_bin / FRAMES * RATE in destination-normalized cycles.
        // 2621 lands at F ~= 0.64 (above Nyquist); 410 lands at F ~= 0.10.
        fn probe(source_bin: usize) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
            let source = (0..FRAMES)
                .map(|index| (TAU * source_bin as f32 * index as f32 / FRAMES as f32).sin())
                .collect::<Vec<_>>();
            let sample = SampleLoopArtifact::from_persisted(
                384_000.0,
                220.0,
                source.clone().into_boxed_slice(),
            );
            let source_audition =
                SourceAuditionArtifact::compile(&source, 384_000).expect("Source audition");
            let reflected_integral = reflected_integral_prefix(&source);
            let mut phase = START / FRAMES as f32;
            let mut periodic = Vec::with_capacity(OUTPUT_FRAMES);
            let mut reflected = Vec::with_capacity(OUTPUT_FRAMES);
            let mut one_shot = Vec::with_capacity(OUTPUT_FRAMES);
            for frame in 0..OUTPUT_FRAMES {
                periodic.push(sample.eval_bandlimited(phase, RATE));
                reflected.push(grain_antialiased_sample(
                    &source,
                    &reflected_integral,
                    START + frame as f32 * RATE,
                    RATE,
                ));
                one_shot.push(
                    source_audition
                        .sample_one_shot_filtered(f64::from(START + frame as f32 * RATE), RATE),
                );
                phase = (phase + RATE / FRAMES as f32).rem_euclid(1.0);
            }
            (periodic, reflected, one_shot)
        }

        let (sample_stop, grain_stop, source_stop) = probe(2_621);
        for (reader, rendered) in [
            ("Sample", sample_stop.as_slice()),
            ("Grain", grain_stop.as_slice()),
            ("Source", source_stop.as_slice()),
        ] {
            assert!(rendered.iter().all(|sample| sample.is_finite()));
            assert!(
                rms(rendered) < 0.003,
                "{reader} folded a just-above-Nyquist rate-32 tone: RMS {}",
                rms(rendered)
            );
        }

        let (sample_pass, grain_pass, source_pass) = probe(410);
        for (reader, rendered) in [
            ("Sample", sample_pass.as_slice()),
            ("Grain", grain_pass.as_slice()),
            ("Source", source_pass.as_slice()),
        ] {
            assert!(rendered.iter().all(|sample| sample.is_finite()));
            assert!(
                rms(rendered) > 0.68,
                "{reader} attenuated a rate-32 passband tone: RMS {}",
                rms(rendered)
            );
        }
    }

    #[test]
    fn legal_rate_thirty_two_readers_reject_comb_alias_without_muting_tones() {
        const RATE: f32 = 32.0;
        const FRAMES: usize = 131_072;
        let alternating = (0..FRAMES)
            .map(|index| if index & 1 == 0 { 1.0 } else { -1.0 })
            .collect::<Vec<_>>();

        let sample = SampleLoopArtifact::from_persisted(
            384_000.0,
            220.0,
            alternating.clone().into_boxed_slice(),
        );
        let mut sample_phase = 0.0_f32;
        let sample_alias = (0..2_048)
            .map(|_| {
                let output = sample.eval_bandlimited(sample_phase, RATE);
                sample_phase = (sample_phase + RATE / sample.frames() as f32).rem_euclid(1.0);
                output
            })
            .collect::<Vec<_>>();

        let controls = ResynthControls {
            position: 0.0,
            grain_size: 0.05,
            grain_density: 0.0,
            grain_spray: 0.0,
            ..ResynthControls::default()
        };
        let grain = GrainSourceArtifact::compile(&alternating, 384_000, Some(220.0), controls)
            .expect("alternating grain");
        let mut grain_scheduler = GrainSchedulerState::default();
        let grain_alias = (0..2_048_u64)
            .map(|frame| grain_scheduler.render(&grain, 880.0, 48_000.0, 31, frame))
            .collect::<Vec<_>>();

        let source = SourceAuditionArtifact::compile(&alternating, 384_000)
            .expect("alternating source audition");
        let source_alias = (0..2_048)
            .map(|frame| source.sample_one_shot_filtered(2_048.0 + f64::from(frame) * 32.0, RATE))
            .collect::<Vec<_>>();

        for (reader, rendered) in [
            ("Sample", sample_alias.as_slice()),
            ("Grain", grain_alias.as_slice()),
            ("Source", source_alias.as_slice()),
        ] {
            assert!(rendered.iter().all(|sample| sample.is_finite()));
            assert!(
                rms(rendered) < 0.03,
                "{reader} rate-32 Nyquist probe folded a comb image: RMS {}",
                rms(rendered)
            );
        }

        let tone = (0..FRAMES)
            .map(|index| (TAU * 220.0 * index as f32 / 384_000.0).sin() * 0.8)
            .collect::<Vec<_>>();
        let tone_sample =
            SampleLoopArtifact::from_persisted(384_000.0, 220.0, tone.clone().into_boxed_slice());
        let mut tone_phase = 0.0_f32;
        let sample_tone = (0..2_048)
            .map(|_| {
                let output = tone_sample.eval_bandlimited(tone_phase, RATE);
                tone_phase = (tone_phase + RATE / tone_sample.frames() as f32).rem_euclid(1.0);
                output
            })
            .collect::<Vec<_>>();
        let tone_grain = GrainSourceArtifact::compile(&tone, 384_000, Some(220.0), controls)
            .expect("tonal grain");
        let mut tone_scheduler = GrainSchedulerState::default();
        let grain_tone = (0..2_048_u64)
            .map(|frame| tone_scheduler.render(&tone_grain, 880.0, 48_000.0, 37, frame))
            .collect::<Vec<_>>();
        let tone_source =
            SourceAuditionArtifact::compile(&tone, 384_000).expect("tonal source audition");
        let source_tone = (0..2_048)
            .map(|frame| {
                tone_source.sample_one_shot_filtered(2_048.0 + f64::from(frame) * 32.0, RATE)
            })
            .collect::<Vec<_>>();
        for (reader, rendered) in [
            ("Sample", sample_tone.as_slice()),
            ("Grain", grain_tone.as_slice()),
            ("Source", source_tone.as_slice()),
        ] {
            assert!(
                rms(rendered) > 0.05,
                "{reader} muted a legal rate-32 low tone: RMS {}",
                rms(rendered)
            );
        }
    }

    #[test]
    fn grain_compile_filters_above_retained_nyquist_before_bounding() {
        let source = (0..1_048_576)
            .map(|index| (TAU * 18_000.0 * index as f32 / 48_000.0).sin() * 0.8)
            .collect::<Vec<_>>();
        let grain =
            GrainSourceArtifact::compile(&source, 48_000, Some(220.0), ResynthControls::default())
                .expect("bounded grain");

        assert_eq!(grain.source_sample_rate, 24_000.0);
        assert!(grain.samples.len() <= GRAIN_MAX_SOURCE_FRAMES);
        let folded = tone_amplitude(&grain.samples, 6_000.0, grain.source_sample_rate);
        assert!(
            folded < 0.03,
            "18 kHz folded into the retained 6 kHz band at amplitude {folded}"
        );
    }

    #[test]
    fn rich_long_stationary_source_preserves_exact_high_harmonic_delta() {
        let make_source = |frames: usize, high_gain: f32| {
            (0..frames)
                .map(|index| {
                    let time = index as f32 / 48_000.0;
                    (TAU * 110.0 * time).sin() * 0.08 + (TAU * 14_080.0 * time).sin() * high_gain
                })
                .collect::<Vec<_>>()
        };
        let compile = |frames, high_gain| {
            RichZoneArtifact::compile(
                &make_source(frames, high_gain),
                48_000,
                110.0,
                ResynthControls::default(),
            )
            .expect("rich harmonic probe")
        };
        let short_root = compile(24_000, 0.0);
        let short_high = compile(24_000, 0.8);
        let long_root = compile(192_000, 0.0);
        let long_high = compile(192_000, 0.8);
        let harmonic_amplitude = |artifact: &RichZoneArtifact| {
            let zone = artifact.zone_for_frequency(110.0);
            tone_amplitude(
                &artifact.slabs()[zone],
                artifact.center_hz[zone] * 128.0,
                RICH_ASSET_SAMPLE_RATE,
            )
        };
        let short_delta = (harmonic_amplitude(&short_high) - harmonic_amplitude(&short_root)).abs();
        let long_delta = (harmonic_amplitude(&long_high) - harmonic_amplitude(&long_root)).abs();
        assert!(
            short_delta > 1.0e-6,
            "short Rich probe did not expose its 128th harmonic: {short_delta}"
        );
        assert!(
            long_delta > short_delta * 0.5,
            "long Rich analysis lost its exact high harmonic: {long_delta} vs {short_delta}"
        );
    }

    #[test]
    fn grain_telemetry_reports_physical_active_lane_mask_and_clears_holes() {
        let mut scheduler = GrainSchedulerState::default();
        scheduler.layers[1] = GrainLayerState {
            position: 10.0,
            age: 4,
            length: 32,
            source_step: 1.0,
            gain: 1.0,
            pan: 0.0,
            pitch: 0.0,
            active: true,
        };
        scheduler.layers[3] = GrainLayerState {
            position: 30.0,
            age: 8,
            length: 64,
            source_step: 1.0,
            gain: 1.0,
            pan: 0.0,
            pitch: 0.0,
            active: true,
        };
        let mut positions = [f32::NAN; GRAIN_TELEMETRY];
        let mut progress = [f32::NAN; GRAIN_TELEMETRY];
        let mut gains = [f32::NAN; GRAIN_TELEMETRY];
        let active_mask = scheduler.write_telemetry(128, &mut positions, &mut progress, &mut gains);

        assert_eq!(scheduler.active_count(), 2);
        assert_eq!(active_mask, 0b0000_0011);
        for index in 2..GRAIN_TELEMETRY {
            assert_eq!(
                (positions[index], progress[index], gains[index]),
                (0.0, 0.0, 0.0)
            );
        }
        assert!(positions[0] > 0.0 && gains[0] > 0.0);
        assert!(positions[1] > positions[0] && gains[1] > 0.0);
    }

    fn rich_source() -> Vec<f32> {
        (0..48_000)
            .map(|index| {
                let phase = TAU * 220.0 * index as f32 / 48_000.0;
                (1..=12)
                    .map(|harmonic| (phase * harmonic as f32).sin() * (0.7 / harmonic as f32))
                    .sum::<f32>()
            })
            .collect()
    }

    fn tone_amplitude(samples: &[f32], hz: f32, sample_rate: f32) -> f32 {
        let (re, im) = samples.iter().copied().enumerate().fold(
            (0.0_f64, 0.0_f64),
            |(re, im), (index, sample)| {
                let angle =
                    std::f64::consts::TAU * f64::from(hz) * index as f64 / f64::from(sample_rate);
                (
                    re + f64::from(sample) * angle.cos(),
                    im - f64::from(sample) * angle.sin(),
                )
            },
        );
        (2.0 * re.hypot(im) / samples.len().max(1) as f64) as f32
    }

    #[test]
    fn rich_source_root_is_preserved_at_110_220_and_440_hz() {
        let artifact =
            RichZoneArtifact::compile(&rich_source(), 48_000, 220.0, ResynthControls::default())
                .expect("rich source");

        for target_hz in [110.0_f32, 220.0, 440.0] {
            let zone = artifact.zone_for_frequency(target_hz);
            let increment = artifact.phase_increment(zone, target_hz, 48_000.0);
            let source_frames_per_output = increment * RICH_ZONE_SAMPLES as f32;
            let mut phase = 0.0_f32;
            let rendered = (0..48_000)
                .map(|_| {
                    let sample = artifact.eval_bandlimited(zone, phase, source_frames_per_output);
                    phase = (phase + increment).rem_euclid(1.0);
                    sample
                })
                .collect::<Vec<_>>();
            let fundamental = tone_amplitude(&rendered, target_hz, 48_000.0);
            let subharmonic = tone_amplitude(&rendered, target_hz * 0.5, 48_000.0);
            let octave = tone_amplitude(&rendered, target_hz * 2.0, 48_000.0);
            assert!(
                fundamental > 0.08,
                "target {target_hz} Hz lost its root: amplitude {fundamental}"
            );
            assert!(
                fundamental > subharmonic * 8.0,
                "target {target_hz} Hz produced a subharmonic root: {fundamental} vs {subharmonic}"
            );
            assert!(
                fundamental > octave * 1.25,
                "target {target_hz} Hz jumped an octave: {fundamental} vs {octave}"
            );
        }
    }
}
