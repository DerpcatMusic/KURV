use std::f32::consts::TAU;

use super::super::ResynthControls;
use super::shared::*;
use crate::dsp::{Complex, fft, shortest_angle, splitmix64};

#[derive(Clone)]
pub struct RichZoneArtifact {
    pub source_sample_rate: f32,
    pub source_frames: u32,
    pub center_hz: [f32; RICH_ZONE_COUNT],
    pub fundamental_bins: [u16; RICH_ZONE_COUNT],
    pub frame_gains: [f32; RICH_FRAME_COUNT],
    dynamic: f32,
    pub(crate) slabs: Box<[[f32; RICH_ZONE_SAMPLES]; RICH_ZONE_COUNT]>,
}

type RichAnalysisFrame = (Vec<Complex>, Vec<f64>, f32);

#[derive(Clone, Debug)]
pub(crate) struct RichSourceAnalysis {
    frames: Vec<RichAnalysisFrame>,
    source_bin_hz: f32,
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
    let last_start = source.len().saturating_sub(source_span);
    let mut frames = Vec::with_capacity(RICH_FRAME_COUNT);
    for frame in 0..RICH_FRAME_COUNT {
        if should_cancel() {
            return Err(ArtifactBuildError::Cancelled);
        }
        let start = last_start * frame / RICH_FRAME_COUNT.saturating_sub(1).max(1);
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
        center_hz: [f32; RICH_ZONE_COUNT],
        fundamental_bins: [u16; RICH_ZONE_COUNT],
        frame_gains: [f32; RICH_FRAME_COUNT],
        dynamic: f32,
        slabs: Box<[[f32; RICH_ZONE_SAMPLES]; RICH_ZONE_COUNT]>,
    ) -> Self {
        Self {
            source_sample_rate,
            source_frames,
            center_hz,
            fundamental_bins,
            frame_gains,
            dynamic: dynamic.clamp(0.0, 1.0),
            slabs,
        }
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
            center_hz,
            fundamental_bins,
            frame_gains,
            dynamic: controls.rich_dynamic,
            slabs,
        })
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

#[cfg(test)]
mod tests {
    use super::*;

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
