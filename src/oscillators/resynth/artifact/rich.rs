use std::f32::consts::TAU;

use super::super::ResynthControls;
use super::shared::*;
use crate::dsp::{Complex, fft, shortest_angle, splitmix64};

#[derive(Clone)]
pub struct RichZoneArtifact {
    pub source_sample_rate: f32,
    pub source_frames: u32,
    pub source_boundaries: [u32; RICH_FRAME_COUNT + 1],
    pub center_hz: [f32; RICH_ZONE_COUNT],
    pub fundamental_bins: [u16; RICH_ZONE_COUNT],
    pub frame_gains: [f32; RICH_FRAME_COUNT],
    dynamic: f32,
    pub(crate) slabs: Box<[[f32; RICH_ZONE_SAMPLES]; RICH_ZONE_COUNT]>,
    sequence: Option<Box<super::GrainSourceArtifact>>,
    vocoder: Option<Box<super::vocoder::RichVocoderArtifact>>,
    locked_density: f32,
    locked_size: f32,
}

type RichAnalysisFrame = (Vec<Complex>, Vec<f64>, f32);

#[derive(Clone, Debug)]
pub(crate) struct RichSourceAnalysis {
    frames: Vec<RichAnalysisFrame>,
    source_bin_hz: f32,
    source_boundaries: [u32; RICH_FRAME_COUNT + 1],
}

impl RichSourceAnalysis {
    pub(crate) fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self
                .frames
                .iter()
                .map(|(spectrum, envelope, _)| {
                    spectrum.len() * std::mem::size_of::<Complex>()
                        + envelope.len() * std::mem::size_of::<f64>()
                })
                .sum::<usize>()
    }
}

pub(crate) fn rich_source_analysis_with_cancel(
    source: &[f32],
    source_sample_rate: u32,
    should_cancel: &dyn Fn() -> bool,
) -> Result<RichSourceAnalysis, ArtifactBuildError> {
    // Preserve the declared 17.2 kHz source band while sampling the complete
    // source timeline into a bounded sequence of spectral frames.
    let stride = (source_sample_rate as usize / RICH_ASSET_SAMPLE_RATE as usize).max(1);
    let window_source_frames = RICH_FRAME_SAMPLES.saturating_mul(stride);
    // Short sources cannot provide eight full-size windows. Use bounded
    // adjacent segments instead of analyzing the same window eight times.
    let source_span = if source.len() <= window_source_frames {
        source
            .len()
            .div_ceil(RICH_FRAME_COUNT)
            .max(1)
            .min(source.len().max(1))
    } else {
        window_source_frames
    };
    let mut source_boundaries = [0_u32; RICH_FRAME_COUNT + 1];
    for (index, boundary) in source_boundaries.iter_mut().enumerate() {
        *boundary = u32::try_from(source.len().saturating_mul(index) / RICH_FRAME_COUNT)
            .unwrap_or(u32::MAX);
    }
    let last_start = source.len().saturating_sub(source_span);
    let mut frames = Vec::with_capacity(RICH_FRAME_COUNT);
    for frame in 0..RICH_FRAME_COUNT {
        if should_cancel() {
            return Err(ArtifactBuildError::Cancelled);
        }
        let interval_start = usize::try_from(source_boundaries[frame]).unwrap_or(source.len());
        let interval_end = usize::try_from(source_boundaries[frame + 1]).unwrap_or(source.len());
        let center = interval_start.saturating_add(interval_end) / 2;
        let start = center.saturating_sub(source_span / 2).min(last_start);
        let retained = bandlimited_decimate_with_cancel(
            &source[start..start + source_span],
            stride,
            should_cancel,
        )?;
        let mut spectrum = vec![Complex::ZERO; RICH_FRAME_SAMPLES];
        let denominator = retained.len().saturating_sub(1).max(1) as f32;
        for (index, sample) in retained.iter().copied().enumerate() {
            let window = 0.5 - 0.5 * (TAU * index as f32 / denominator).cos();
            spectrum[index].re = f64::from(sample * window);
        }
        fft(&mut spectrum, false);
        let scale = RICH_FRAME_SAMPLES as f64 / retained.len().max(1) as f64;
        let power = spectrum
            .iter()
            .take(RICH_FRAME_SAMPLES / 2 + 1)
            .map(|bin| bin.re.mul_add(bin.re, bin.im * bin.im) * scale * scale)
            .collect::<Vec<_>>();
        let mut log_envelope = vec![0.0_f64; power.len()];
        for index in 0..log_envelope.len() {
            let lo = index.saturating_sub(3);
            let hi = (index + 3).min(power.len() - 1);
            let local_power = power[lo..=hi].iter().sum::<f64>() / (hi - lo + 1) as f64;
            log_envelope[index] = (local_power.sqrt() + 1.0e-12).ln();
        }
        let gain = retained.iter().copied().map(f32::abs).fold(0.0, f32::max);
        frames.push((spectrum, log_envelope, gain));
    }
    let effective_rate = source_sample_rate as f32 / stride as f32;
    Ok(RichSourceAnalysis {
        frames,
        source_bin_hz: effective_rate / RICH_FRAME_SAMPLES as f32,
        source_boundaries,
    })
}

fn render_rich_zone(
    analysis_frames: &[RichAnalysisFrame],
    source_bin_hz: f32,
    requested_center: f32,
    root_hz: f32,
    controls: ResynthControls,
    slab: &mut [f32],
    should_cancel: &dyn Fn() -> bool,
) -> Result<(f32, u16), ArtifactBuildError> {
    debug_assert_eq!(slab.len(), RICH_ZONE_SAMPLES);
    let asset_bin_hz = RICH_ASSET_SAMPLE_RATE / RICH_FRAME_SAMPLES as f32;
    let fundamental_bin = (requested_center / asset_bin_hz).round().max(1.0) as usize;
    let actual_center = fundamental_bin as f32 * asset_bin_hz;
    let formant_ratio = 2.0_f32.powf(controls.rich_formant_semitones / 12.0);
    let air_gain = 10.0_f32.powf(controls.rich_air_db / 20.0);
    let balance = controls.rich_balance;
    let (tonal_gain, source_gain, residual_gain) = if balance <= 0.0 {
        let angle = f64::from(balance + 1.0) * std::f64::consts::FRAC_PI_2;
        (angle.cos(), angle.sin(), 0.0)
    } else {
        let angle = f64::from(balance) * std::f64::consts::FRAC_PI_2;
        (0.0, angle.cos(), angle.sin())
    };
    let max_harmonic = (RICH_GUARD_HZ / actual_center).floor() as usize;
    let mut frame_spectrum = vec![Complex::ZERO; RICH_FRAME_SAMPLES];
    for (frame_index, (spectrum, log_envelope, _)) in analysis_frames.iter().enumerate() {
        if should_cancel() {
            return Err(ArtifactBuildError::Cancelled);
        }
        frame_spectrum.fill(Complex::ZERO);
        let fundamental_phase_bin = (root_hz / source_bin_hz)
            .round()
            .clamp(1.0, (spectrum.len() / 2) as f32) as usize;
        let fundamental = spectrum[fundamental_phase_bin];
        let phase_anchor = fundamental.im.atan2(fundamental.re);
        for harmonic in 1..=max_harmonic {
            let target_bin = harmonic * fundamental_bin;
            if target_bin >= RICH_FRAME_SAMPLES / 2 {
                break;
            }
            let target_hz = target_bin as f32 * asset_bin_hz;
            let source_bin = target_hz / actual_center.max(f32::MIN_POSITIVE) * root_hz
                / formant_ratio
                / source_bin_hz.max(f32::MIN_POSITIVE);
            let magnitude = envelope_at(log_envelope, source_bin).exp();
            let local_tonal = f64::from(tonal_fraction(log_envelope, source_bin));
            let tonal_magnitude = magnitude * local_tonal;
            let residual_magnitude = (magnitude * magnitude - tonal_magnitude * tonal_magnitude)
                .max(0.0)
                .sqrt();
            let shelf = if target_hz >= 8_000.0 { air_gain } else { 1.0 };
            let phase_index = source_bin.round().clamp(0.0, (spectrum.len() / 2) as f32) as usize;
            let phase = spectrum[phase_index];
            let measured_phase = phase.im.atan2(phase.re);
            let source_phase = if measured_phase.is_finite() && magnitude > 1.0e-8 {
                (measured_phase - harmonic as f64 * phase_anchor).rem_euclid(std::f64::consts::TAU)
            } else {
                hash_phase(controls.seed, frame_index as u64, harmonic as u64)
            };
            let random_phase = hash_phase(controls.seed, frame_index as u64, harmonic as u64);
            let diffuse_phase = source_phase
                + shortest_angle(source_phase, random_phase) * f64::from(controls.rich_diffuse);
            let tonal_and_source = tonal_gain * tonal_magnitude + source_gain * magnitude;
            let re = (tonal_and_source * source_phase.cos()
                + residual_gain * residual_magnitude * diffuse_phase.cos())
                * f64::from(shelf);
            let im = (tonal_and_source * source_phase.sin()
                + residual_gain * residual_magnitude * diffuse_phase.sin())
                * f64::from(shelf);
            frame_spectrum[target_bin] = Complex { re, im };
            frame_spectrum[RICH_FRAME_SAMPLES - target_bin] = Complex { re, im: -im };
        }
        fft(&mut frame_spectrum, true);
        let start = frame_index * RICH_FRAME_SAMPLES;
        let frame = &mut slab[start..start + RICH_FRAME_SAMPLES];
        for (sample, bin) in frame.iter_mut().zip(&frame_spectrum) {
            *sample = bin.re as f32;
        }
        remove_dc_and_peak_normalize(frame);
    }
    Ok((
        actual_center,
        u16::try_from(fundamental_bin).unwrap_or(u16::MAX),
    ))
}

impl RichZoneArtifact {
    pub fn compile(
        source: &[f32],
        source_sample_rate: u32,
        root_hz: f32,
        controls: ResynthControls,
    ) -> Result<Self, ArtifactBuildError> {
        Self::compile_with_cancel(source, source_sample_rate, root_hz, controls, &|| false)
    }

    pub(crate) fn from_persisted(
        source_sample_rate: f32,
        source_frames: u32,
        source_boundaries: [u32; RICH_FRAME_COUNT + 1],
        center_hz: [f32; RICH_ZONE_COUNT],
        fundamental_bins: [u16; RICH_ZONE_COUNT],
        frame_gains: [f32; RICH_FRAME_COUNT],
        dynamic: f32,
        slabs: Box<[[f32; RICH_ZONE_SAMPLES]; RICH_ZONE_COUNT]>,
    ) -> Self {
        Self {
            source_sample_rate,
            source_frames,
            source_boundaries,
            center_hz,
            fundamental_bins,
            frame_gains,
            dynamic: dynamic.clamp(0.0, 1.0),
            slabs,
            sequence: None,
            vocoder: None,
            locked_density: crate::oscillators::ResynthQuality::Standard.locked_grain_density(),
            locked_size: crate::oscillators::ResynthQuality::Standard.locked_grain_size(),
        }
    }

    pub(crate) fn from_legacy_persisted(
        source_sample_rate: f32,
        source_frames: u32,
        center_hz: [f32; RICH_ZONE_COUNT],
        fundamental_bins: [u16; RICH_ZONE_COUNT],
        legacy_gains: [f32; LEGACY_RICH_FRAME_COUNT],
        dynamic: f32,
        legacy_slabs: Box<[[f32; LEGACY_RICH_ZONE_SAMPLES]; RICH_ZONE_COUNT]>,
    ) -> Self {
        let mut source_boundaries = [0_u32; RICH_FRAME_COUNT + 1];
        for (index, boundary) in source_boundaries.iter_mut().enumerate() {
            *boundary = source_frames.saturating_mul(index as u32) / RICH_FRAME_COUNT as u32;
        }
        let expansion = RICH_FRAME_COUNT / LEGACY_RICH_FRAME_COUNT;
        let mut frame_gains = [1.0_f32; RICH_FRAME_COUNT];
        for (legacy, gain) in legacy_gains.into_iter().enumerate() {
            for value in &mut frame_gains[legacy * expansion..(legacy + 1) * expansion] {
                *value = gain;
            }
        }
        let mut slabs = Box::<[[f32; RICH_ZONE_SAMPLES]; RICH_ZONE_COUNT]>::new_uninit();
        // SAFETY: zero is valid and each active element is copied below.
        let mut slabs = unsafe {
            std::ptr::write_bytes(slabs.as_mut_ptr(), 0, 1);
            slabs.assume_init()
        };
        for (zone, legacy_slab) in legacy_slabs.iter().enumerate() {
            for legacy_frame in 0..LEGACY_RICH_FRAME_COUNT {
                let source_start = legacy_frame * LEGACY_RICH_FRAME_SAMPLES;
                for repeat in 0..expansion {
                    let target_start = (legacy_frame * expansion + repeat) * RICH_FRAME_SAMPLES;
                    slabs[zone][target_start..target_start + RICH_FRAME_SAMPLES].copy_from_slice(
                        &legacy_slab[source_start..source_start + LEGACY_RICH_FRAME_SAMPLES],
                    );
                }
            }
        }
        Self::from_persisted(
            source_sample_rate,
            source_frames,
            source_boundaries,
            center_hz,
            fundamental_bins,
            frame_gains,
            dynamic,
            slabs,
        )
    }

    pub(crate) fn compile_with_cancel(
        source: &[f32],
        source_sample_rate: u32,
        root_hz: f32,
        controls: ResynthControls,
        should_cancel: &dyn Fn() -> bool,
    ) -> Result<Self, ArtifactBuildError> {
        validate_source(source)?;
        if should_cancel() {
            return Err(ArtifactBuildError::Cancelled);
        }
        if !root_hz.is_finite() || !(20.0..=2_000.0).contains(&root_hz) {
            return Err(ArtifactBuildError::RootRequired);
        }
        let controls = controls.sanitized();
        let analysis = rich_source_analysis_with_cancel(source, source_sample_rate, should_cancel)?;
        Self::compile_from_analysis_with_cancel(
            &analysis,
            source_sample_rate,
            source.len(),
            root_hz,
            controls,
            should_cancel,
        )
    }

    pub(crate) fn unrendered(
        source_sample_rate: u32,
        source_frames: usize,
        _root_hz: f32,
        controls: ResynthControls,
    ) -> Self {
        let controls = controls.sanitized();
        let mut source_boundaries = [0_u32; RICH_FRAME_COUNT + 1];
        for (index, boundary) in source_boundaries.iter_mut().enumerate() {
            *boundary = u32::try_from(source_frames.saturating_mul(index) / RICH_FRAME_COUNT)
                .unwrap_or(u32::MAX);
        }
        let mut slabs = Box::<[[f32; RICH_ZONE_SAMPLES]; RICH_ZONE_COUNT]>::new_uninit();
        // SAFETY: zero is valid f32 and sequence playback does not read slabs.
        let slabs = unsafe {
            std::ptr::write_bytes(slabs.as_mut_ptr(), 0, 1);
            slabs.assume_init()
        };
        Self {
            source_sample_rate: source_sample_rate as f32,
            source_frames: u32::try_from(source_frames).unwrap_or(u32::MAX),
            source_boundaries,
            center_hz: std::array::from_fn(|zone| MIDI_ZERO_HZ * 2.0_f32.powf(zone as f32 * 0.5)),
            fundamental_bins: [1; RICH_ZONE_COUNT],
            frame_gains: [1.0; RICH_FRAME_COUNT],
            dynamic: controls.rich_dynamic,
            slabs,
            sequence: None,
            vocoder: None,
            locked_density: crate::oscillators::ResynthQuality::current().locked_grain_density(),
            locked_size: crate::oscillators::ResynthQuality::current().locked_grain_size(),
        }
    }

    pub(crate) fn compile_from_analysis_with_cancel(
        analysis: &RichSourceAnalysis,
        source_sample_rate: u32,
        source_frames: usize,
        root_hz: f32,
        controls: ResynthControls,
        should_cancel: &dyn Fn() -> bool,
    ) -> Result<Self, ArtifactBuildError> {
        if should_cancel() {
            return Err(ArtifactBuildError::Cancelled);
        }
        if !root_hz.is_finite() || !(20.0..=2_000.0).contains(&root_hz) {
            return Err(ArtifactBuildError::RootRequired);
        }
        let controls = controls.sanitized();
        let maximum_gain = analysis
            .frames
            .iter()
            .map(|frame| frame.2)
            .fold(0.0_f32, f32::max)
            .max(f32::MIN_POSITIVE);
        let frame_gains =
            std::array::from_fn(|index| (analysis.frames[index].2 / maximum_gain).clamp(0.0, 1.0));
        let mut slabs = Box::<[[f32; RICH_ZONE_SAMPLES]; RICH_ZONE_COUNT]>::new_uninit();
        // SAFETY: the destination is an array of f32; all-zero is valid and
        // each slab is fully overwritten below before assume_init.
        let mut slabs = unsafe {
            std::ptr::write_bytes(slabs.as_mut_ptr(), 0, 1);
            slabs.assume_init()
        };
        let mut center_hz = [0.0_f32; RICH_ZONE_COUNT];
        let mut fundamental_bins = [0_u16; RICH_ZONE_COUNT];
        for zone in 0..RICH_ZONE_COUNT {
            if should_cancel() {
                return Err(ArtifactBuildError::Cancelled);
            }
            let requested_center = MIDI_ZERO_HZ * 2.0_f32.powf(zone as f32 * 0.5);
            (center_hz[zone], fundamental_bins[zone]) = render_rich_zone(
                &analysis.frames,
                analysis.source_bin_hz,
                requested_center,
                root_hz,
                controls,
                &mut slabs[zone],
                should_cancel,
            )?;
        }
        Ok(Self {
            source_sample_rate: source_sample_rate as f32,
            source_frames: u32::try_from(source_frames).unwrap_or(u32::MAX),
            source_boundaries: analysis.source_boundaries,
            center_hz,
            fundamental_bins,
            frame_gains,
            dynamic: controls.rich_dynamic,
            slabs,
            sequence: None,
            vocoder: None,
            locked_density: crate::oscillators::ResynthQuality::current().locked_grain_density(),
            locked_size: crate::oscillators::ResynthQuality::current().locked_grain_size(),
        })
    }

    #[expect(dead_code, reason = "v15 sequence compile retained for recalled packs")]
    pub(crate) fn attach_sequence(
        &mut self,
        source: &[f32],
        source_sample_rate: u32,
        root_hz: f32,
        controls: ResynthControls,
        quality: crate::oscillators::ResynthQuality,
        should_cancel: &dyn Fn() -> bool,
    ) -> Result<(), ArtifactBuildError> {
        let mut grain_controls = controls.sanitized();
        grain_controls.grain_density = quality.locked_grain_density_at(source_sample_rate as f32);
        grain_controls.grain_size = quality.locked_grain_size_at(source_sample_rate as f32);
        grain_controls.grain_spray = 0.0;
        let mut sequence = super::GrainSourceArtifact::compile_channels_with_cancel(
            source,
            None,
            source_sample_rate,
            Some(root_hz),
            grain_controls,
            quality,
            should_cancel,
        )?;
        let reconstructed = reconstruct_timeline(
            &sequence.samples,
            sequence.source_sample_rate,
            controls,
            quality,
            should_cancel,
        )?;
        sequence.replace_pcm_keep_pitch(reconstructed, should_cancel)?;
        self.locked_density = quality.locked_grain_density_at(sequence.source_sample_rate);
        self.locked_size = quality.locked_grain_size_at(sequence.source_sample_rate);
        self.sequence = Some(Box::new(sequence));
        Ok(())
    }

    pub(crate) fn attach_vocoder(
        &mut self,
        source: &[f32],
        source_sample_rate: u32,
        root_hz: f32,
        quality: crate::oscillators::ResynthQuality,
        should_cancel: &dyn Fn() -> bool,
    ) -> Result<(), ArtifactBuildError> {
        let vocoder = super::vocoder::RichVocoderArtifact::compile_with_cancel(
            source,
            source_sample_rate,
            root_hz,
            quality,
            should_cancel,
        )?;
        self.vocoder = Some(Box::new(vocoder));
        Ok(())
    }

    #[must_use]
    pub(crate) fn sequence(&self) -> Option<&super::GrainSourceArtifact> {
        self.sequence.as_deref()
    }

    #[must_use]
    pub(crate) fn vocoder(&self) -> Option<&super::vocoder::RichVocoderArtifact> {
        self.vocoder.as_deref()
    }

    pub(crate) fn restore_vocoder(&mut self, vocoder: super::vocoder::RichVocoderArtifact) {
        self.vocoder = Some(Box::new(vocoder));
    }

    pub(crate) fn restore_sequence(
        &mut self,
        sequence: super::GrainSourceArtifact,
        locked_density: f32,
        locked_size: f32,
    ) {
        self.locked_density = locked_density.clamp(1.0, 2_000.0);
        self.locked_size = locked_size.clamp(0.0, 1.0);
        self.sequence = Some(Box::new(sequence));
    }

    #[must_use]
    pub fn locked_density(&self) -> f32 {
        self.locked_density
    }

    #[must_use]
    pub fn locked_size(&self) -> f32 {
        self.locked_size
    }

    #[cfg(test)]
    #[must_use]
    pub fn slabs(&self) -> &[[f32; RICH_ZONE_SAMPLES]; RICH_ZONE_COUNT] {
        &self.slabs
    }

    #[inline]
    #[must_use]
    pub fn zone_for_frequency(&self, target_hz: f32) -> usize {
        const HALF_OCTAVE_RATIO: f32 = std::f32::consts::SQRT_2;
        const FIRST_UPPER_BOUND_HZ: f32 = MIDI_ZERO_HZ * 1.189_207_1;
        let target_hz = target_hz.max(MIDI_ZERO_HZ);
        let mut zone = 0_usize;
        let mut upper_bound = FIRST_UPPER_BOUND_HZ;
        while zone + 1 < RICH_ZONE_COUNT && target_hz >= upper_bound {
            zone += 1;
            upper_bound *= HALF_OCTAVE_RATIO;
        }
        zone
    }

    #[inline]
    #[must_use]
    pub fn zone_for_frequency_hysteretic(&self, current: usize, target_hz: f32) -> usize {
        let mut zone = current.min(RICH_ZONE_COUNT - 1);
        // Nominal boundaries are quarter-octaves; the additional 3% log2
        // margin is folded into constants so selection has no RT transcendental.
        const UPPER_RATIO_WITH_MARGIN: f32 = 1.214_194_9;
        const LOWER_RATIO_WITH_MARGIN: f32 = 0.823_591;
        while zone + 1 < RICH_ZONE_COUNT
            && target_hz > self.center_hz[zone] * UPPER_RATIO_WITH_MARGIN
        {
            zone += 1;
        }
        while zone > 0 && target_hz < self.center_hz[zone] * LOWER_RATIO_WITH_MARGIN {
            zone -= 1;
        }
        zone
    }

    #[cfg(test)]
    #[inline]
    #[must_use]
    pub fn eval(&self, zone: usize, phase: f32) -> f32 {
        periodic_cubic(self.frame(zone, 0), phase)
    }

    #[inline]
    #[must_use]
    pub fn eval_bandlimited(&self, zone: usize, phase: f32, source_frames_per_output: f32) -> f32 {
        periodic_antialiased_sample(
            self.frame(zone, 0),
            phase,
            source_frames_per_output / RICH_FRAME_COUNT as f32,
        )
    }

    #[inline]
    #[must_use]
    pub fn eval_morphed(
        &self,
        zone: usize,
        phase: f32,
        source_frames_per_output: f32,
        playback_frame: u64,
        host_sample_rate: f32,
    ) -> f32 {
        self.eval_morphed_with_dynamic(
            zone,
            phase,
            source_frames_per_output,
            playback_frame,
            host_sample_rate,
            self.dynamic,
        )
    }

    #[inline]
    #[must_use]
    pub fn eval_morphed_with_dynamic(
        &self,
        zone: usize,
        phase: f32,
        source_frames_per_output: f32,
        playback_frame: u64,
        host_sample_rate: f32,
        dynamic: f32,
    ) -> f32 {
        let duration =
            self.source_frames.max(1) as f64 / f64::from(self.source_sample_rate.max(1.0));
        let timeline = (playback_frame as f64 / f64::from(host_sample_rate.max(1.0)) / duration)
            .rem_euclid(1.0)
            * RICH_FRAME_COUNT as f64;
        self.eval_at_timeline(
            zone,
            phase,
            source_frames_per_output,
            (timeline / RICH_FRAME_COUNT as f64) as f32,
            host_sample_rate,
            dynamic,
        )
    }

    #[inline]
    #[must_use]
    pub fn eval_at_timeline(
        &self,
        zone: usize,
        phase: f32,
        source_frames_per_output: f32,
        timeline_phase: f32,
        host_sample_rate: f32,
        dynamic: f32,
    ) -> f32 {
        let timeline = timeline_phase.clamp(0.0, 1.0 - f32::EPSILON) * RICH_FRAME_COUNT as f32;
        let whole = timeline.floor();
        let first_frame = whole as usize;
        let second_frame = (first_frame + 1) % RICH_FRAME_COUNT;
        // Hold each analyzed timbre for most of its source interval. A short
        // boundary handover prevents clicks without permanently smearing two
        // different moments together.
        let mix = ((timeline - whole - 0.875) * 8.0).clamp(0.0, 1.0);
        let direct =
            source_frames_per_output.abs() * RICH_GUARD_HZ <= host_sample_rate.max(1.0) * 0.5;
        let sample_frame = |frame| {
            if direct {
                periodic_cubic(self.frame(zone, frame), phase)
            } else {
                periodic_antialiased_sample(
                    self.frame(zone, frame),
                    phase,
                    source_frames_per_output,
                )
            }
        };
        let first = sample_frame(first_frame);
        let (sample, measured_gain) = if mix <= f32::EPSILON {
            (first, self.frame_gains[first_frame])
        } else {
            let second = sample_frame(second_frame);
            (
                (second - first).mul_add(mix, first),
                (self.frame_gains[second_frame] - self.frame_gains[first_frame])
                    .mul_add(mix, self.frame_gains[first_frame]),
            )
        };
        sample * (measured_gain - 1.0).mul_add(dynamic.clamp(0.0, 1.0), 1.0)
    }

    #[inline]
    #[must_use]
    pub const fn dynamic(&self) -> f32 {
        self.dynamic
    }

    #[inline]
    #[must_use]
    pub fn phase_increment(&self, zone: usize, target_hz: f32, host_sample_rate: f32) -> f32 {
        let fundamental_bin = f32::from(self.fundamental_bins[zone.min(RICH_ZONE_COUNT - 1)]);
        target_hz.max(0.0) / host_sample_rate.max(1.0) / fundamental_bin.max(1.0)
    }

    #[inline]
    fn frame(&self, zone: usize, frame: usize) -> &[f32] {
        let start = frame.min(RICH_FRAME_COUNT - 1) * RICH_FRAME_SAMPLES;
        &self.slabs[zone.min(RICH_ZONE_COUNT - 1)][start..start + RICH_FRAME_SAMPLES]
    }
}

fn envelope_at(envelope: &[f64], position: f32) -> f64 {
    let position = position.clamp(0.0, envelope.len().saturating_sub(1) as f32);
    let first = position.floor() as usize;
    let second = (first + 1).min(envelope.len() - 1);
    let mix = f64::from(position - first as f32);
    envelope[first] + (envelope[second] - envelope[first]) * mix
}

fn tonal_fraction(envelope: &[f64], position: f32) -> f32 {
    if envelope.is_empty() {
        return 0.0;
    }
    let center = position.round().clamp(0.0, (envelope.len() - 1) as f32) as usize;
    let lo = center.saturating_sub(2);
    let hi = (center + 2).min(envelope.len() - 1);
    let local_mean = envelope[lo..=hi].iter().sum::<f64>() / (hi - lo + 1) as f64;
    let prominence = (envelope[center] - local_mean).max(0.0);
    (1.0 - (-prominence * 4.0).exp()) as f32
}

fn hash_phase(seed: u64, zone: u64, harmonic: u64) -> f64 {
    splitmix64(seed ^ zone.wrapping_mul(0xd6e8_feb8_6659_fd93) ^ harmonic) as f64 / u64::MAX as f64
        * std::f64::consts::TAU
}

#[expect(
    dead_code,
    reason = "v15 sequence reconstruct retained for recalled packs"
)]
fn reconstruct_timeline(
    source: &[f32],
    sample_rate: f32,
    controls: ResynthControls,
    quality: crate::oscillators::ResynthQuality,
    should_cancel: &dyn Fn() -> bool,
) -> Result<Vec<f32>, ArtifactBuildError> {
    validate_source(source)?;
    let fft_size = quality.fft_size();
    let hop = quality.reconstruction_hop().max(1);
    let mut output = vec![0.0_f64; source.len()];
    let mut weight = vec![0.0_f64; source.len()];
    let mut spectrum = vec![Complex::ZERO; fft_size];
    let formant_ratio = 2.0_f32
        .powf(controls.rich_formant_semitones / 12.0)
        .max(0.25);
    let air_gain = f64::from(10.0_f32.powf(controls.rich_air_db / 20.0));
    let diffuse = f64::from(controls.rich_diffuse.clamp(0.0, 1.0));
    let balance = controls.rich_balance;
    let (tonal_gain, residual_gain) = if balance <= 0.0 {
        (1.0 + f64::from(-balance), 1.0)
    } else {
        (1.0, 1.0 - f64::from(balance) * 0.85)
    };
    let nyquist = f64::from(sample_rate) * 0.5;
    let frames = source.len().div_ceil(hop) + 1;
    for frame in 0..frames {
        if should_cancel() {
            return Err(ArtifactBuildError::Cancelled);
        }
        let center = frame * hop;
        let start = center as isize - fft_size as isize / 2;
        for (index, bin) in spectrum.iter_mut().enumerate() {
            let source_index = start + index as isize;
            let hann = 0.5 - 0.5 * (std::f64::consts::TAU * index as f64 / fft_size as f64).cos();
            bin.re = if (0..source.len() as isize).contains(&source_index) {
                f64::from(source[source_index as usize]) * hann
            } else {
                0.0
            };
            bin.im = 0.0;
        }
        fft(&mut spectrum, false);
        let half = fft_size / 2;
        for bin in 1..half {
            let frequency = bin as f64 * f64::from(sample_rate) / fft_size as f64;
            let source_bin = (bin as f32 / formant_ratio).clamp(1.0, (half - 1) as f32);
            let lo = source_bin.floor() as usize;
            let hi = (lo + 1).min(half - 1);
            let mix = f64::from(source_bin - lo as f32);
            let magnitude = spectrum[lo].norm() + (spectrum[hi].norm() - spectrum[lo].norm()) * mix;
            let phase = spectrum[bin].arg();
            let random = hash_phase(controls.seed, frame as u64, bin as u64);
            let phase = phase + shortest_angle(phase, random) * diffuse;
            let shelf = if frequency >= 8_000.0 { air_gain } else { 1.0 };
            let mut mag = magnitude * shelf * residual_gain;
            if frequency > 80.0 {
                mag *= tonal_gain;
            }
            if frequency > nyquist * 0.92 {
                mag *= 0.35;
            }
            spectrum[bin] = Complex::from_polar(mag, phase);
            spectrum[fft_size - bin] = spectrum[bin].conj();
        }
        fft(&mut spectrum, true);
        for (index, bin) in spectrum.iter().enumerate() {
            let output_index = start + index as isize;
            if !(0..output.len() as isize).contains(&output_index) {
                continue;
            }
            let hann = 0.5 - 0.5 * (std::f64::consts::TAU * index as f64 / fft_size as f64).cos();
            output[output_index as usize] += bin.re * hann;
            weight[output_index as usize] += hann * hann;
        }
    }
    let mut samples = output
        .into_iter()
        .zip(weight)
        .map(|(sample, weight)| (sample / weight.max(1.0e-9)) as f32)
        .collect::<Vec<_>>();
    remove_dc_and_peak_normalize(&mut samples);
    Ok(samples)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_sources_have_ordered_full_timeline_coverage() {
        let source = (0..65_536)
            .map(|index| {
                let t = index as f32 / 48_000.0;
                let frequency = if index < 32_768 { 220.0 } else { 880.0 };
                (TAU * frequency * t).sin()
            })
            .collect::<Vec<_>>();
        let analysis =
            rich_source_analysis_with_cancel(&source, 48_000, &|| false).expect("rich analysis");
        assert_eq!(analysis.frames.len(), RICH_FRAME_COUNT);
        assert_eq!(analysis.source_boundaries[0], 0);
        assert_eq!(
            analysis.source_boundaries[RICH_FRAME_COUNT],
            source.len() as u32
        );
        assert!(
            analysis
                .source_boundaries
                .windows(2)
                .all(|pair| pair[0] <= pair[1])
        );
        let dominant = |spectrum: &[Complex]| {
            spectrum
                .iter()
                .take(RICH_FRAME_SAMPLES / 2)
                .enumerate()
                .max_by(|(_, left), (_, right)| left.norm_sqr().total_cmp(&right.norm_sqr()))
                .map_or(0, |(index, _)| index)
        };
        let first = dominant(&analysis.frames[0].0);
        let last = dominant(&analysis.frames[RICH_FRAME_COUNT - 1].0);
        assert!(first.abs_diff(last) > 10, "first={first} last={last}");
    }

    #[test]
    fn short_sources_fill_distinct_rich_timeline_frames() {
        let source = (0..8_192)
            .map(|index| {
                let t = index as f32 / 48_000.0;
                let frequency = if index < 4_096 { 220.0 } else { 880.0 };
                (TAU * frequency * t).sin()
            })
            .collect::<Vec<_>>();
        let analysis =
            rich_source_analysis_with_cancel(&source, 48_000, &|| false).expect("rich analysis");
        assert_eq!(analysis.frames.len(), RICH_FRAME_COUNT);
        let dominant = |spectrum: &[Complex]| {
            spectrum
                .iter()
                .take(RICH_FRAME_SAMPLES / 2)
                .enumerate()
                .max_by(|(_, left), (_, right)| left.norm_sqr().total_cmp(&right.norm_sqr()))
                .map_or(0, |(index, _)| index)
        };
        let first = dominant(&analysis.frames[0].0);
        let last = dominant(&analysis.frames[RICH_FRAME_COUNT - 1].0);
        assert!(first.abs_diff(last) > 10, "first={first} last={last}");
    }
}
