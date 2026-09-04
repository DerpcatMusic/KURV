//! Stereo spectral-lock RICH. The worker bakes a circular spectral trajectory
//! and phase-scrambled source residual; the callback only interpolates fixed
//! arrays and advances a bounded oscillator bank.

use std::f32::consts::{PI, TAU};

use super::super::ResynthControls;
use super::super::analysis::{PitchTrack, PitchTrackFrame};
use super::super::quality::ResynthQuality;
use super::shared::{ArtifactBuildError, MAX_SOURCE_ABS_SAMPLE, validate_source};
use crate::dsp::{Complex, fft, splitmix64};

pub const VOCODER_ENVELOPE_BINS: usize = 64;
pub const VOCODER_MAX_HARMONICS: usize = 128;
pub const VOCODER_MAX_FRAMES: usize = 8_192;
const LIFTER_SECONDS: f32 = 0.01;
const MIN_ENVELOPE_HZ: f32 = 20.0;
const CONTROL_INTERVAL: usize = 32;
const RESIDUAL_MAX_FRAMES: usize = 256;
const MIN_TONAL: f32 = 0.02;
pub const VOCODER_MAX_RESIDUAL_SAMPLES: usize = RESIDUAL_MAX_FRAMES * 1_024;

#[derive(Clone, Copy, Debug)]
pub struct RichVocoderFrame {
    pub f0_hz: f32,
    pub voiced: f32,
    pub gain: f32,
    pub aperiodicity: f32,
    pub envelope: [f32; VOCODER_ENVELOPE_BINS],
    pub phase: [f32; VOCODER_MAX_HARMONICS],
}

#[derive(Clone, Debug)]
pub struct RichVocoderArtifact {
    pub sample_rate: f32,
    pub source_frames: u32,
    pub root_hz: f32,
    pub nyquist: f32,
    pub synth_gain: f32,
    pub quality: ResynthQuality,
    pub pitch_track: PitchTrack,
    frames: Box<[RichVocoderFrame]>,
    right_envelopes: Box<[[f32; VOCODER_ENVELOPE_BINS]]>,
    residual_left: Box<[f32]>,
    residual_right: Box<[f32]>,
}

impl RichVocoderArtifact {
    #[must_use]
    pub fn frames(&self) -> &[RichVocoderFrame] {
        &self.frames
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    #[must_use]
    pub fn right_envelopes(&self) -> &[[f32; VOCODER_ENVELOPE_BINS]] {
        &self.right_envelopes
    }

    #[must_use]
    pub fn residual_channels(&self) -> (&[f32], &[f32]) {
        (&self.residual_left, &self.residual_right)
    }

    #[must_use]
    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.frames.len() * std::mem::size_of::<RichVocoderFrame>()
            + self.right_envelopes.len() * std::mem::size_of::<[f32; VOCODER_ENVELOPE_BINS]>()
            + (self.residual_left.len() + self.residual_right.len()) * std::mem::size_of::<f32>()
    }

    pub(crate) fn compile_with_cancel(
        source: &[f32],
        source_sample_rate: u32,
        root_hz: f32,
        quality: ResynthQuality,
        should_cancel: &dyn Fn() -> bool,
    ) -> Result<Self, ArtifactBuildError> {
        Self::compile_channels_with_cancel(
            source,
            None,
            source_sample_rate,
            root_hz,
            quality,
            should_cancel,
        )
    }

    pub(crate) fn compile_channels_with_cancel(
        mid: &[f32],
        side: Option<&[f32]>,
        source_sample_rate: u32,
        root_hz: f32,
        quality: ResynthQuality,
        should_cancel: &dyn Fn() -> bool,
    ) -> Result<Self, ArtifactBuildError> {
        let source = mid;
        validate_source(source)?;
        if side.is_some_and(|side| side.len() != source.len()) {
            return Err(ArtifactBuildError::Empty);
        }
        if should_cancel() {
            return Err(ArtifactBuildError::Cancelled);
        }
        if !root_hz.is_finite() || !(20.0..=2_000.0).contains(&root_hz) {
            return Err(ArtifactBuildError::RootRequired);
        }
        let sample_rate = source_sample_rate as f32;
        let fft_size = quality.fft_size();
        let hop = (sample_rate * quality.hop_seconds()).round().max(1.0) as usize;
        let points = source.len().div_ceil(hop).clamp(1, quality.max_points());
        let window = (sample_rate * quality.window_seconds())
            .round()
            .clamp(64.0, fft_size as f32) as usize;
        let nyquist = sample_rate * 0.5;
        let mut left_spectrum = vec![Complex::ZERO; fft_size];
        let mut right_spectrum = vec![Complex::ZERO; fft_size];
        let mut frames = Vec::with_capacity(points);
        let mut right_envelopes = Vec::with_capacity(points);
        let mut track = Vec::with_capacity(points);
        let mut peak_gain = f32::MIN_POSITIVE;
        let mut previous_envelope = [0.0_f32; VOCODER_ENVELOPE_BINS];
        let pitch_track = super::spectral_tune::spectral_pitch_track_with_cancel(
            source,
            side.unwrap_or(&[]),
            sample_rate,
            quality,
            should_cancel,
        )?;
        for point in 0..points {
            if should_cancel() {
                return Err(ArtifactBuildError::Cancelled);
            }
            let center = if points == 1 {
                source.len() / 2
            } else {
                point * source.len().saturating_sub(1) / (points - 1)
            };
            let start = center
                .saturating_sub(window / 2)
                .min(source.len().saturating_sub(window.max(1)));
            let end = (start + window).min(source.len());
            let pitch = pitch_track.lookup(if points == 1 {
                0.0
            } else {
                point as f32 / (points - 1) as f32
            });
            let (f0_hz, confidence) = (pitch.f0_hz, pitch.confidence);
            let (left_envelope, left_phase, left_flatness, left_gain) = analyze_envelope(
                source,
                side,
                1.0,
                start,
                end,
                &mut left_spectrum,
                fft_size,
                sample_rate,
                f0_hz,
            );
            let (right_envelope, _, right_flatness, right_gain) = if side.is_some() {
                analyze_envelope(
                    source,
                    side,
                    -1.0,
                    start,
                    end,
                    &mut right_spectrum,
                    fft_size,
                    sample_rate,
                    f0_hz,
                )
            } else {
                (left_envelope, left_phase, left_flatness, left_gain)
            };
            let gain = left_gain.max(right_gain);
            let aperiodicity = (left_flatness + right_flatness) * 0.5;
            let voiced = if f0_hz > 0.0 && confidence >= 0.2 {
                confidence.clamp(0.0, 1.0)
            } else {
                0.0
            };
            let f0_hz = if voiced > 0.0 { f0_hz } else { 0.0 };
            let onset = left_envelope
                .iter()
                .zip(previous_envelope)
                .map(|(current, previous)| (current - previous).max(0.0))
                .sum::<f32>()
                / VOCODER_ENVELOPE_BINS as f32;
            previous_envelope = left_envelope;
            peak_gain = peak_gain.max(gain);
            frames.push(RichVocoderFrame {
                f0_hz,
                voiced,
                gain,
                aperiodicity,
                envelope: left_envelope,
                phase: left_phase,
            });
            right_envelopes.push(right_envelope);
            track.push(PitchTrackFrame {
                f0_hz,
                confidence: voiced,
                onset: (onset * 0.2).clamp(0.0, 1.0),
            });
        }
        if frames.is_empty() {
            return Err(ArtifactBuildError::Empty);
        }
        let gain_norm = peak_gain.max(f32::MIN_POSITIVE);
        for frame in &mut frames {
            frame.gain = (frame.gain / gain_norm).clamp(0.0, 1.0);
        }
        let (residual_left, residual_right) =
            build_residual_loop(source, side, fft_size, should_cancel)?;
        let mut artifact = Self {
            sample_rate,
            source_frames: u32::try_from(source.len()).unwrap_or(u32::MAX),
            root_hz,
            nyquist,
            synth_gain: 1.0,
            quality,
            pitch_track: PitchTrack::from_frames(track),
            frames: frames.into_boxed_slice(),
            right_envelopes: right_envelopes.into_boxed_slice(),
            residual_left,
            residual_right,
        };
        artifact.synth_gain = calibrate_synth_gain(&artifact);
        Ok(artifact)
    }

    pub(crate) fn from_persisted(
        sample_rate: f32,
        source_frames: u32,
        root_hz: f32,
        synth_gain: f32,
        quality: ResynthQuality,
        frames: Vec<RichVocoderFrame>,
    ) -> Option<Self> {
        if frames.is_empty() {
            return None;
        }
        let right_envelopes = frames
            .iter()
            .map(|frame| frame.envelope)
            .collect::<Vec<_>>();
        let residual = synthesize_legacy_residual(&frames, sample_rate);
        Self::from_persisted_channels(
            sample_rate,
            source_frames,
            root_hz,
            synth_gain,
            quality,
            frames,
            right_envelopes,
            residual.clone(),
            residual,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_persisted_channels(
        sample_rate: f32,
        source_frames: u32,
        root_hz: f32,
        synth_gain: f32,
        quality: ResynthQuality,
        frames: Vec<RichVocoderFrame>,
        right_envelopes: Vec<[f32; VOCODER_ENVELOPE_BINS]>,
        residual_left: Box<[f32]>,
        residual_right: Box<[f32]>,
    ) -> Option<Self> {
        if !sample_rate.is_finite()
            || sample_rate <= 0.0
            || source_frames == 0
            || !root_hz.is_finite()
            || !(20.0..=2_000.0).contains(&root_hz)
            || !synth_gain.is_finite()
            || !(0.0..=16.0).contains(&synth_gain)
            || frames.is_empty()
            || frames.len() > VOCODER_MAX_FRAMES
            || frames.iter().any(|frame| {
                !frame.f0_hz.is_finite()
                    || !(0.0..=4_000.0).contains(&frame.f0_hz)
                    || !frame.voiced.is_finite()
                    || !(0.0..=1.0).contains(&frame.voiced)
                    || !frame.gain.is_finite()
                    || !(0.0..=1.0).contains(&frame.gain)
                    || !frame.aperiodicity.is_finite()
                    || !(0.0..=1.0).contains(&frame.aperiodicity)
                    || frame
                        .envelope
                        .iter()
                        .any(|value| !value.is_finite() || !(-200.0..=40.0).contains(value))
                    || frame
                        .phase
                        .iter()
                        .any(|value| !value.is_finite() || value.abs() > PI)
            })
            || right_envelopes.len() != frames.len()
            || residual_left.len() != residual_right.len()
            || !(2..=VOCODER_MAX_RESIDUAL_SAMPLES).contains(&residual_left.len())
            || right_envelopes
                .iter()
                .flatten()
                .any(|value| !value.is_finite() || !(-200.0..=40.0).contains(value))
            || residual_left
                .iter()
                .chain(residual_right.iter())
                .any(|value| !value.is_finite() || value.abs() > MAX_SOURCE_ABS_SAMPLE)
        {
            return None;
        }
        let track = frames
            .iter()
            .map(|frame| PitchTrackFrame {
                f0_hz: frame.f0_hz,
                confidence: frame.voiced,
                onset: 0.0,
            })
            .collect::<Vec<_>>();
        Some(Self {
            sample_rate,
            source_frames,
            root_hz,
            nyquist: sample_rate * 0.5,
            synth_gain,
            quality,
            pitch_track: PitchTrack::from_frames(track),
            frames: frames.into_boxed_slice(),
            right_envelopes: right_envelopes.into_boxed_slice(),
            residual_left,
            residual_right,
        })
    }

    #[inline]
    pub fn lookup(&self, position: f32) -> InterpolatedFrame {
        let frames = self.frames.as_ref();
        if frames.is_empty() {
            return InterpolatedFrame::SILENT;
        }
        let scaled = position.rem_euclid(1.0) * frames.len() as f32;
        let lower = scaled.floor() as usize;
        let upper = (lower + 1) % frames.len();
        let mix = scaled - lower as f32;
        InterpolatedFrame::lerp(
            &frames[lower],
            &frames[upper],
            &self.right_envelopes[lower],
            &self.right_envelopes[upper],
            mix,
        )
    }
}

#[derive(Clone, Copy)]
pub struct InterpolatedFrame {
    pub f0_hz: f32,
    pub voiced: f32,
    pub gain: f32,
    pub aperiodicity: f32,
    pub envelope: [f32; VOCODER_ENVELOPE_BINS],
    pub right_envelope: [f32; VOCODER_ENVELOPE_BINS],
    pub phase: [f32; VOCODER_MAX_HARMONICS],
}

impl InterpolatedFrame {
    const SILENT: Self = Self {
        f0_hz: 0.0,
        voiced: 0.0,
        gain: 0.0,
        aperiodicity: 1.0,
        envelope: [0.0; VOCODER_ENVELOPE_BINS],
        right_envelope: [0.0; VOCODER_ENVELOPE_BINS],
        phase: [0.0; VOCODER_MAX_HARMONICS],
    };

    fn lerp(
        first: &RichVocoderFrame,
        second: &RichVocoderFrame,
        first_right: &[f32; VOCODER_ENVELOPE_BINS],
        second_right: &[f32; VOCODER_ENVELOPE_BINS],
        mix: f32,
    ) -> Self {
        let mix = mix.clamp(0.0, 1.0);
        let f0_hz = if first.voiced > MIN_TONAL && second.voiced > MIN_TONAL {
            first.f0_hz + (second.f0_hz - first.f0_hz) * mix
        } else if second.voiced > first.voiced {
            second.f0_hz
        } else {
            first.f0_hz
        };
        let mut envelope = first.envelope;
        for (slot, (a, b)) in envelope
            .iter_mut()
            .zip(first.envelope.iter().zip(second.envelope.iter()))
        {
            *slot = a + (b - a) * mix;
        }
        let mut right_envelope = *first_right;
        for (slot, (a, b)) in right_envelope
            .iter_mut()
            .zip(first_right.iter().zip(second_right.iter()))
        {
            *slot = a + (b - a) * mix;
        }
        let mut phase = first.phase;
        for (slot, (a, b)) in phase
            .iter_mut()
            .zip(first.phase.iter().zip(second.phase.iter()))
        {
            *slot = *a + crate::dsp::shortest_angle(f64::from(*a), f64::from(*b)) as f32 * mix;
        }
        Self {
            f0_hz,
            voiced: first.voiced + (second.voiced - first.voiced) * mix,
            gain: first.gain + (second.gain - first.gain) * mix,
            aperiodicity: first.aperiodicity + (second.aperiodicity - first.aperiodicity) * mix,
            envelope,
            right_envelope,
            phase,
        }
    }
}

/// Keyboard transpose at amount=0, flatten to the played note at amount=1.
/// `amount` is already voiced; do not multiply confidence again.
#[must_use]
pub fn retune_f0(source_f0: f32, played_hz: f32, root_hz: f32, amount: f32) -> f32 {
    if source_f0 <= 0.0 {
        return 0.0;
    }
    let played = played_hz.max(20.0);
    let root = root_hz.max(20.0);
    let transposed = source_f0 * (played / root);
    let amount = amount.clamp(0.0, 1.0);
    transposed.mul_add(1.0 - amount, played * amount)
}

#[derive(Clone, Copy, Debug)]
pub struct RichVocoderState {
    phase_sin: [f32; VOCODER_MAX_HARMONICS],
    phase_cos: [f32; VOCODER_MAX_HARMONICS],
    step_sin: [f32; VOCODER_MAX_HARMONICS],
    step_cos: [f32; VOCODER_MAX_HARMONICS],
    amplitude_left: [f32; VOCODER_MAX_HARMONICS],
    amplitude_right: [f32; VOCODER_MAX_HARMONICS],
    amplitude_step_left: [f32; VOCODER_MAX_HARMONICS],
    amplitude_step_right: [f32; VOCODER_MAX_HARMONICS],
    active_harmonics: usize,
    control_remaining: usize,
    residual_lp_left: f32,
    residual_lp_right: f32,
    residual_air_gain: f32,
    residual_lp_coeff: f32,
    tonal_gain: f32,
    residual_gain: f32,
    dynamic_gain: f32,
    diffuse: f32,
    stereo_mix: f32,
}

impl Default for RichVocoderState {
    fn default() -> Self {
        Self {
            phase_sin: [0.0; VOCODER_MAX_HARMONICS],
            phase_cos: [1.0; VOCODER_MAX_HARMONICS],
            step_sin: [0.0; VOCODER_MAX_HARMONICS],
            step_cos: [1.0; VOCODER_MAX_HARMONICS],
            amplitude_left: [0.0; VOCODER_MAX_HARMONICS],
            amplitude_right: [0.0; VOCODER_MAX_HARMONICS],
            amplitude_step_left: [0.0; VOCODER_MAX_HARMONICS],
            amplitude_step_right: [0.0; VOCODER_MAX_HARMONICS],
            active_harmonics: 0,
            control_remaining: 0,
            residual_lp_left: 0.0,
            residual_lp_right: 0.0,
            residual_air_gain: 1.0,
            residual_lp_coeff: 0.5,
            tonal_gain: 1.0,
            residual_gain: 0.35,
            dynamic_gain: 1.0,
            diffuse: 0.5,
            stereo_mix: 1.0,
        }
    }
}

impl RichVocoderState {
    #[inline]
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    #[inline]
    pub(crate) fn next_stereo(&mut self, target: f32, sample_rate: f32) -> f32 {
        let step = (200.0 / sample_rate.max(1.0)).min(1.0);
        self.stereo_mix += (target.clamp(0.0, 1.0) - self.stereo_mix).clamp(-step, step);
        self.stereo_mix
    }

    #[inline]
    pub fn render(
        &mut self,
        artifact: &RichVocoderArtifact,
        timeline: f32,
        played_hz: f32,
        sample_rate: f32,
        controls: ResynthControls,
    ) -> f32 {
        let (left, right) =
            self.render_stereo(artifact, timeline, played_hz, sample_rate, controls);
        (left + right) * 0.5
    }

    #[inline]
    pub fn render_stereo(
        &mut self,
        artifact: &RichVocoderArtifact,
        timeline: f32,
        played_hz: f32,
        sample_rate: f32,
        controls: ResynthControls,
    ) -> (f32, f32) {
        if self.control_remaining == 0 {
            self.refresh_targets(artifact, timeline, played_hz, sample_rate, controls);
        }
        self.control_remaining -= 1;
        let mut harmonic_left = 0.0_f32;
        let mut harmonic_right = 0.0_f32;
        for index in 0..self.active_harmonics {
            self.amplitude_left[index] += self.amplitude_step_left[index];
            self.amplitude_right[index] += self.amplitude_step_right[index];
            let sin = self.phase_sin[index];
            let cos = self.phase_cos[index];
            harmonic_left += sin * self.amplitude_left[index];
            harmonic_right += sin * self.amplitude_right[index];
            self.phase_sin[index] = sin.mul_add(self.step_cos[index], cos * self.step_sin[index]);
            self.phase_cos[index] = cos.mul_add(self.step_cos[index], -sin * self.step_sin[index]);
        }
        let residual_a = residual_at(artifact, timeline);
        let residual_b = residual_at(artifact, timeline + 0.173_205_08);
        let residual_left = (residual_b.0 - residual_a.0).mul_add(self.diffuse, residual_a.0);
        let residual_right = (residual_b.1 - residual_a.1).mul_add(self.diffuse, residual_a.1);
        self.residual_lp_left += self.residual_lp_coeff * (residual_left - self.residual_lp_left);
        self.residual_lp_right +=
            self.residual_lp_coeff * (residual_right - self.residual_lp_right);
        let residual_left = residual_left
            + (self.residual_air_gain - 1.0) * (residual_left - self.residual_lp_left);
        let residual_right = residual_right
            + (self.residual_air_gain - 1.0) * (residual_right - self.residual_lp_right);
        let gain = artifact.synth_gain * self.dynamic_gain;
        (
            (harmonic_left * self.tonal_gain + residual_left * self.residual_gain) * gain,
            (harmonic_right * self.tonal_gain + residual_right * self.residual_gain) * gain,
        )
    }

    fn refresh_targets(
        &mut self,
        artifact: &RichVocoderArtifact,
        timeline: f32,
        played_hz: f32,
        sample_rate: f32,
        controls: ResynthControls,
    ) {
        let frame = artifact.lookup(timeline);
        let amount = controls.grain_tune.clamp(0.0, 1.0) * frame.voiced.clamp(0.0, 1.0);
        let f0_out = retune_f0(frame.f0_hz, played_hz, artifact.root_hz, amount);
        let formant = 2.0_f32
            .powf(controls.rich_formant_semitones / 12.0)
            .clamp(0.25, 4.0);
        let air_gain = 10.0_f32.powf(controls.rich_air_db / 20.0);
        let balance = controls.rich_balance.clamp(-1.0, 1.0);
        (self.tonal_gain, self.residual_gain) = if balance <= 0.0 {
            let angle = (balance + 1.0) * 0.5 * PI;
            (angle.cos() + angle.sin(), angle.sin() * 0.35)
        } else {
            let angle = balance * 0.5 * PI;
            (angle.cos(), angle.sin())
        };
        self.residual_gain *= frame.aperiodicity;
        let nyquist = sample_rate.max(1.0) * 0.45;
        let host_nyquist = artifact.nyquist.max(1.0);
        let max_h = artifact
            .quality
            .max_harmonics()
            .min((nyquist / f0_out).floor() as usize)
            .min(VOCODER_MAX_HARMONICS);
        let fundamental_angle = TAU * f0_out / sample_rate.max(1.0);
        let (fundamental_sin, fundamental_cos) = fundamental_angle.sin_cos();
        let mut step_sin = fundamental_sin;
        let mut step_cos = fundamental_cos;
        let normalizer = (max_h.max(1) as f32).sqrt().recip();
        for index in 0..VOCODER_MAX_HARMONICS {
            if index < max_h {
                let hz = f0_out * (index + 1) as f32;
                let env_hz = (hz / formant).clamp(0.0, host_nyquist);
                let left = envelope_at(&frame.envelope, env_hz, host_nyquist)
                    .clamp(-24.0, 4.0)
                    .exp();
                let right = envelope_at(&frame.right_envelope, env_hz, host_nyquist)
                    .clamp(-24.0, 4.0)
                    .exp();
                let shelf = if hz >= 8_000.0 { air_gain } else { 1.0 };
                let tonal = frame.voiced.max(MIN_TONAL);
                let target_left = left * shelf * tonal * normalizer;
                let target_right = right * shelf * tonal * normalizer;
                self.amplitude_step_left[index] =
                    (target_left - self.amplitude_left[index]) / CONTROL_INTERVAL as f32;
                self.amplitude_step_right[index] =
                    (target_right - self.amplitude_right[index]) / CONTROL_INTERVAL as f32;
                if self.amplitude_left[index].abs() + self.amplitude_right[index].abs() < 1.0e-7 {
                    let random = splitmix64(controls.seed ^ index as u64) as f64 / u64::MAX as f64
                        * std::f64::consts::TAU;
                    let source = f64::from(frame.phase[index]);
                    let phase = source
                        + crate::dsp::shortest_angle(source, random)
                            * f64::from(controls.rich_diffuse.clamp(0.0, 1.0));
                    (self.phase_sin[index], self.phase_cos[index]) = (phase as f32).sin_cos();
                }
                self.step_sin[index] = step_sin;
                self.step_cos[index] = step_cos;
                let next_sin = step_sin.mul_add(fundamental_cos, step_cos * fundamental_sin);
                let next_cos = step_cos.mul_add(fundamental_cos, -step_sin * fundamental_sin);
                step_sin = next_sin;
                step_cos = next_cos;
                let norm = self.phase_sin[index]
                    .mul_add(
                        self.phase_sin[index],
                        self.phase_cos[index] * self.phase_cos[index],
                    )
                    .sqrt()
                    .max(f32::MIN_POSITIVE);
                self.phase_sin[index] /= norm;
                self.phase_cos[index] /= norm;
            } else {
                self.amplitude_step_left[index] =
                    -self.amplitude_left[index] / CONTROL_INTERVAL as f32;
                self.amplitude_step_right[index] =
                    -self.amplitude_right[index] / CONTROL_INTERVAL as f32;
            }
        }
        self.active_harmonics = self.active_harmonics.max(max_h);
        while self.active_harmonics > max_h
            && self.amplitude_left[self.active_harmonics - 1].abs() < 1.0e-7
            && self.amplitude_right[self.active_harmonics - 1].abs() < 1.0e-7
        {
            self.active_harmonics -= 1;
        }
        self.dynamic_gain = (frame.gain - 1.0).mul_add(controls.rich_dynamic.clamp(0.0, 1.0), 1.0);
        self.diffuse = controls.rich_diffuse.clamp(0.0, 1.0);
        self.residual_air_gain = air_gain;
        self.residual_lp_coeff = 1.0 - (-TAU * 6_000.0 / sample_rate.max(1.0)).exp();
        self.control_remaining = CONTROL_INTERVAL;
    }
}

fn analyze_envelope(
    mid: &[f32],
    side: Option<&[f32]>,
    side_sign: f32,
    start: usize,
    end: usize,
    spectrum: &mut [Complex],
    fft_size: usize,
    sample_rate: f32,
    f0_hz: f32,
) -> (
    [f32; VOCODER_ENVELOPE_BINS],
    [f32; VOCODER_MAX_HARMONICS],
    f32,
    f32,
) {
    spectrum.fill(Complex::ZERO);
    let count = end.saturating_sub(start).min(fft_size);
    let mut window_sum = 0.0_f64;
    let mut peak = 0.0_f32;
    let denominator = count.saturating_sub(1).max(1) as f32;
    let mut log_sum = 0.0_f64;
    for index in 0..count {
        let source_index = start + index;
        let sample = mid[source_index] + side.map_or(0.0, |side| side[source_index] * side_sign);
        let hann = 0.5 - 0.5 * (TAU * index as f32 / denominator).cos();
        peak = peak.max(sample.abs());
        window_sum += f64::from(hann);
        spectrum[index].re = f64::from(sample * hann);
    }
    fft(spectrum, false);
    let half = fft_size / 2;
    let bin_hz = sample_rate / fft_size as f32;
    let mut phase = [0.0_f32; VOCODER_MAX_HARMONICS];
    if f0_hz >= MIN_ENVELOPE_HZ {
        let fundamental = complex_at(spectrum, f0_hz / bin_hz).arg();
        for (index, slot) in phase.iter_mut().enumerate() {
            let harmonic = index + 1;
            let partial = complex_at(spectrum, f0_hz / bin_hz * harmonic as f32);
            *slot = wrap_angle(partial.arg() - harmonic as f64 * fundamental) as f32;
        }
    }
    let scale = 2.0 / window_sum.max(1.0e-9);
    let mut total_power = 0.0_f64;
    let mut magnitude_sum = 0.0_f64;
    for bin in 0..=half {
        let mag = spectrum[bin].norm() * scale;
        let power = mag * mag;
        total_power += power;
        magnitude_sum += mag;
        log_sum += (mag + 1.0e-12).ln();
        spectrum[bin] = Complex {
            re: (mag + 1.0e-12).ln(),
            im: 0.0,
        };
        if bin > 0 && bin < half {
            spectrum[fft_size - bin] = spectrum[bin];
        }
    }
    fft(spectrum, true);
    let lifter = (sample_rate * LIFTER_SECONDS) as usize;
    let lifter = lifter.clamp(4, half / 4);
    for (index, bin) in spectrum.iter_mut().enumerate() {
        if index > lifter && index < fft_size - lifter {
            *bin = Complex::ZERO;
        }
    }
    fft(spectrum, false);
    let nyquist = sample_rate * 0.5;
    let mut envelope = [0.0_f32; VOCODER_ENVELOPE_BINS];
    for (index, slot) in envelope.iter_mut().enumerate() {
        let unit = index as f32 / (VOCODER_ENVELOPE_BINS - 1) as f32;
        let hz = MIN_ENVELOPE_HZ * (nyquist / MIN_ENVELOPE_HZ).powf(unit);
        let pos = (hz / bin_hz).clamp(0.0, half as f32);
        let lo = pos.floor() as usize;
        let hi = (lo + 1).min(half);
        let mix = f64::from(pos - lo as f32);
        let log_mag = spectrum[lo].re + (spectrum[hi].re - spectrum[lo].re) * mix;
        *slot = log_mag as f32;
    }
    let bin_count = (half + 1) as f64;
    let flatness = if total_power > 1.0e-20 && magnitude_sum > 1.0e-12 {
        ((log_sum / bin_count).exp() / (magnitude_sum / bin_count)).clamp(0.0, 1.0) as f32
    } else {
        1.0
    };
    (envelope, phase, flatness, peak)
}

#[inline]
fn envelope_at(envelope: &[f32; VOCODER_ENVELOPE_BINS], hz: f32, nyquist: f32) -> f32 {
    let pos = (hz.max(MIN_ENVELOPE_HZ) / MIN_ENVELOPE_HZ).ln()
        / (nyquist.max(MIN_ENVELOPE_HZ + 1.0) / MIN_ENVELOPE_HZ).ln()
        * (VOCODER_ENVELOPE_BINS - 1) as f32;
    let pos = pos.clamp(0.0, (VOCODER_ENVELOPE_BINS - 1) as f32);
    let lo = pos.floor() as usize;
    let hi = (lo + 1).min(VOCODER_ENVELOPE_BINS - 1);
    let mix = pos - lo as f32;
    envelope[lo] + (envelope[hi] - envelope[lo]) * mix
}

fn complex_at(spectrum: &[Complex], position: f32) -> Complex {
    let position = position.clamp(0.0, spectrum.len().saturating_div(2) as f32);
    let first = position.floor() as usize;
    let second = (first + 1).min(spectrum.len() / 2);
    let mix = f64::from(position - first as f32);
    Complex {
        re: spectrum[first].re + (spectrum[second].re - spectrum[first].re) * mix,
        im: spectrum[first].im + (spectrum[second].im - spectrum[first].im) * mix,
    }
}

fn wrap_angle(angle: f64) -> f64 {
    (angle + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
}

fn residual_at(artifact: &RichVocoderArtifact, position: f32) -> (f32, f32) {
    let len = artifact.residual_left.len();
    if len == 0 {
        return (0.0, 0.0);
    }
    let scaled = position.rem_euclid(1.0) * len as f32;
    let first = scaled.floor() as usize % len;
    let second = (first + 1) % len;
    let mix = scaled - scaled.floor();
    (
        (artifact.residual_left[second] - artifact.residual_left[first])
            .mul_add(mix, artifact.residual_left[first]),
        (artifact.residual_right[second] - artifact.residual_right[first])
            .mul_add(mix, artifact.residual_right[first]),
    )
}

fn build_residual_loop(
    mid: &[f32],
    side: Option<&[f32]>,
    requested_fft_size: usize,
    should_cancel: &dyn Fn() -> bool,
) -> Result<(Box<[f32]>, Box<[f32]>), ArtifactBuildError> {
    let fft_size = requested_fft_size.min(4_096).max(256);
    let hop = fft_size / 4;
    let frame_count = mid.len().div_ceil(hop).clamp(2, RESIDUAL_MAX_FRAMES);
    let output_len = frame_count * hop;
    let mut left = build_residual_channel(
        mid,
        side,
        1.0,
        fft_size,
        frame_count,
        output_len,
        should_cancel,
    )?;
    let has_stereo_residual = side.is_some_and(|side| {
        let side_power = side
            .iter()
            .map(|sample| f64::from(*sample) * f64::from(*sample))
            .sum::<f64>();
        let mid_power = mid
            .iter()
            .map(|sample| f64::from(*sample) * f64::from(*sample))
            .sum::<f64>();
        side_power > mid_power * 1.0e-6
    });
    let mut right = if has_stereo_residual {
        build_residual_channel(
            mid,
            side,
            -1.0,
            fft_size,
            frame_count,
            output_len,
            should_cancel,
        )?
    } else {
        left.clone()
    };
    let peak = left
        .iter()
        .chain(right.iter())
        .copied()
        .map(f32::abs)
        .fold(0.0_f32, f32::max)
        .max(f32::MIN_POSITIVE);
    let scale = 0.35 / peak;
    for sample in left.iter_mut().chain(right.iter_mut()) {
        *sample *= scale;
    }
    Ok((left.into_boxed_slice(), right.into_boxed_slice()))
}

fn build_residual_channel(
    mid: &[f32],
    side: Option<&[f32]>,
    side_sign: f32,
    fft_size: usize,
    frame_count: usize,
    output_len: usize,
    should_cancel: &dyn Fn() -> bool,
) -> Result<Vec<f32>, ArtifactBuildError> {
    let hop = fft_size / 4;
    let half = fft_size / 2;
    let mut output = vec![0.0_f64; output_len];
    let mut weight = vec![0.0_f64; output_len];
    let mut spectrum = vec![Complex::ZERO; fft_size];
    for frame in 0..frame_count {
        if should_cancel() {
            return Err(ArtifactBuildError::Cancelled);
        }
        let center = if frame_count == 1 {
            mid.len() / 2
        } else {
            frame * mid.len().saturating_sub(1) / (frame_count - 1)
        };
        let start = center.saturating_sub(fft_size / 2);
        spectrum.fill(Complex::ZERO);
        for (index, bin) in spectrum.iter_mut().enumerate() {
            let source_index = start + index;
            if source_index >= mid.len() {
                break;
            }
            let hann = 0.5 - 0.5 * (TAU * index as f32 / fft_size as f32).cos();
            let sample =
                mid[source_index] + side.map_or(0.0, |side| side[source_index] * side_sign);
            bin.re = f64::from(sample * hann);
        }
        fft(&mut spectrum, false);
        for bin in 1..half {
            let lo = bin.saturating_sub(4);
            let hi = (bin + 4).min(half);
            // Keep the local broadband floor, not narrow source harmonics.
            // The played-note oscillator bank owns every pitched partial.
            let local_floor = spectrum[lo..=hi]
                .iter()
                .map(|bin| bin.norm())
                .fold(f64::INFINITY, f64::min);
            let magnitude = spectrum[bin].norm().min(local_floor * 2.0);
            let channel = if side_sign < 0.0 {
                0xD6E8_FEB8_6659_FD93
            } else {
                0
            };
            let hash = splitmix64((frame as u64).rotate_left(23) ^ bin as u64 ^ channel);
            let phase = hash as f64 / u64::MAX as f64 * std::f64::consts::TAU;
            spectrum[bin] = Complex::from_polar(magnitude, phase);
            spectrum[fft_size - bin] = spectrum[bin].conj();
        }
        spectrum[0] = Complex::ZERO;
        spectrum[half] = Complex::ZERO;
        fft(&mut spectrum, true);
        for (index, bin) in spectrum.iter().enumerate() {
            let target = (frame * hop + index) % output_len;
            let hann = 0.5 - 0.5 * (TAU * index as f32 / fft_size as f32).cos();
            output[target] += bin.re * f64::from(hann);
            weight[target] += f64::from(hann * hann);
        }
    }
    let output = output
        .into_iter()
        .zip(weight)
        .map(|(sample, weight)| (sample / weight.max(1.0e-9)) as f32)
        .collect::<Vec<_>>();
    Ok(output)
}

fn synthesize_legacy_residual(frames: &[RichVocoderFrame], sample_rate: f32) -> Box<[f32]> {
    let len = 4_096;
    let mut output = Vec::with_capacity(len);
    let mut noise = 0xA341_316C_u32;
    let coeff = 1.0 - (-TAU * 4_000.0 / sample_rate.max(1.0)).exp();
    let mut low = 0.0_f32;
    for index in 0..len {
        noise ^= noise << 13;
        noise ^= noise >> 17;
        noise ^= noise << 5;
        let white = noise as i32 as f32 / 2_147_483_648.0;
        low += coeff * (white - low);
        let frame = &frames[index * frames.len() / len];
        output.push((white - low) * frame.aperiodicity * frame.gain * 0.2);
    }
    output.into_boxed_slice()
}

fn calibrate_synth_gain(artifact: &RichVocoderArtifact) -> f32 {
    let mut state = RichVocoderState::default();
    let mut controls = ResynthControls::default();
    controls.grain_tune = 0.0;
    controls.rich_dynamic = 1.0;
    let mut peak = 0.0_f32;
    let samples = 2_048_usize;
    for index in 0..samples {
        let timeline = index as f32 / samples.saturating_sub(1).max(1) as f32 * 0.25 + 0.1;
        let (left, right) = state.render_stereo(
            artifact,
            timeline,
            artifact.root_hz,
            artifact.sample_rate,
            controls,
        );
        peak = peak.max(left.abs()).max(right.abs());
    }
    (0.65 / peak.max(0.025)).clamp(0.05, 16.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn formant_source(
        frames: usize,
        sample_rate: f32,
        f0: f32,
        formant: f32,
        bandwidth: f32,
    ) -> Vec<f32> {
        let max_h = (sample_rate * 0.45 / f0).floor() as usize;
        let mut samples = (0..frames)
            .map(|index| {
                let time = index as f32 / sample_rate;
                let mut sum = 0.0_f32;
                for harmonic in 1..=max_h {
                    let hz = harmonic as f32 * f0;
                    let saw = 1.0 / harmonic as f32;
                    let bump = (-0.5 * ((hz - formant) / bandwidth).powi(2)).exp();
                    sum += (saw * 0.35 + bump) * (TAU * hz * time).sin();
                }
                sum
            })
            .collect::<Vec<_>>();
        let peak = samples
            .iter()
            .copied()
            .map(f32::abs)
            .fold(0.0_f32, f32::max)
            .max(f32::MIN_POSITIVE);
        for sample in &mut samples {
            *sample /= peak;
        }
        samples
    }

    fn dft_power(samples: &[f32], sample_rate: f32, hz: f32) -> f64 {
        let mut re = 0.0_f64;
        let mut im = 0.0_f64;
        for (index, sample) in samples.iter().copied().enumerate() {
            let angle =
                -std::f64::consts::TAU * f64::from(hz) * index as f64 / f64::from(sample_rate);
            re += f64::from(sample) * angle.cos();
            im += f64::from(sample) * angle.sin();
        }
        re * re + im * im
    }

    fn band_peak(samples: &[f32], sample_rate: f32, lo: f32, hi: f32, step: f32) -> f32 {
        let mut best_hz = lo;
        let mut best = 0.0_f64;
        let mut hz = lo;
        while hz <= hi {
            let power = dft_power(samples, sample_rate, hz);
            if power > best {
                best = power;
                best_hz = hz;
            }
            hz += step;
        }
        best_hz
    }

    fn render_artifact(
        artifact: &RichVocoderArtifact,
        played: f32,
        tune: f32,
        samples: usize,
    ) -> Vec<f32> {
        let mut state = RichVocoderState::default();
        let mut controls = ResynthControls::default();
        controls.grain_tune = tune;
        controls.rich_dynamic = 1.0;
        let duration = artifact.source_frames.max(1) as f32 / artifact.sample_rate.max(1.0);
        (0..samples)
            .map(|index| {
                let timeline = (index as f32 / artifact.sample_rate / duration).min(0.999);
                state.render(artifact, timeline, played, artifact.sample_rate, controls)
            })
            .collect()
    }

    #[test]
    fn retune_amount_transposes_then_flattens() {
        let transposed = retune_f0(330.0, 440.0, 220.0, 0.0);
        let flattened = retune_f0(330.0, 440.0, 220.0, 1.0);
        assert!((transposed - 660.0).abs() < 1.0, "{transposed}");
        assert!((flattened - 440.0).abs() < 1.0, "{flattened}");
        let original = retune_f0(220.0, 220.0, 220.0, 0.0);
        assert!((original - 220.0).abs() < 0.1, "{original}");
    }

    #[test]
    fn retune_does_not_double_gate_voicing() {
        let source = 330.0;
        let played = 440.0;
        let root = 220.0;
        let voiced = 0.5;
        let once = retune_f0(source, played, root, voiced);
        let twice = retune_f0(source, played, root, voiced * voiced);
        assert!((once - 550.0).abs() < 1.0, "{once}");
        assert!(twice > once);
    }

    #[test]
    fn formants_stay_when_pitch_doubles() {
        const SR: f32 = 48_000.0;
        const F0: f32 = 220.0;
        const FORMANT: f32 = 2_000.0;
        let source = formant_source(24_000, SR, F0, FORMANT, 280.0);
        let artifact = RichVocoderArtifact::compile_with_cancel(
            &source,
            SR as u32,
            F0,
            ResynthQuality::Standard,
            &|| false,
        )
        .expect("vocoder");
        let mid = artifact.lookup(0.4);
        assert!(
            mid.f0_hz > 80.0 && mid.voiced > 0.2,
            "analysis f0 {}",
            mid.f0_hz
        );
        let rendered = render_artifact(&artifact, 440.0, 1.0, 16_384);
        let settled = &rendered[4_096..];
        assert!(settled.iter().all(|sample| sample.is_finite()));
        let peak = settled
            .iter()
            .copied()
            .map(f32::abs)
            .fold(0.0_f32, f32::max);
        assert!(peak > 0.02, "silent vocoder {peak}");
        let p220 = dft_power(settled, SR, 220.0);
        let p440 = dft_power(settled, SR, 440.0);
        assert!(
            p440 > p220 * 2.0,
            "retuned F0 energy 440={p440} vs 220={p220}"
        );
        let near_2k = dft_power(settled, SR, 2_000.0)
            + dft_power(settled, SR, 1_760.0)
            + dft_power(settled, SR, 2_200.0);
        let near_4k = dft_power(settled, SR, 4_000.0)
            + dft_power(settled, SR, 3_520.0)
            + dft_power(settled, SR, 4_400.0);
        assert!(
            near_2k > near_4k * 1.5,
            "formant tracked the octave: 2k={near_2k} 4k={near_4k} (chipmunk would invert this)"
        );
        let formant_peak = band_peak(settled, SR, 1_200.0, 5_000.0, 40.0);
        assert!(formant_peak < 3_200.0, "formant moved to {formant_peak}");
    }

    #[test]
    fn melody_flattens_only_at_full_tune() {
        const SR: f32 = 48_000.0;
        let source = (0..24_000)
            .map(|index| {
                let f0 = if index < 12_000 { 220.0 } else { 330.0 };
                (TAU * f0 * index as f32 / SR).sin() * 0.8
            })
            .collect::<Vec<_>>();
        let artifact = RichVocoderArtifact::compile_with_cancel(
            &source,
            SR as u32,
            220.0,
            ResynthQuality::Standard,
            &|| false,
        )
        .expect("vocoder");
        let first = artifact.lookup(0.2).f0_hz;
        let last = artifact.lookup(0.8).f0_hz;
        assert!(
            first > 180.0 && last > first * 1.15,
            "first={first} last={last}"
        );

        let flat = render_artifact(&artifact, 440.0, 1.0, 24_000);
        let early = band_peak(&flat[2_048..10_000], SR, 300.0, 800.0, 10.0);
        let late = band_peak(&flat[14_000..22_000], SR, 300.0, 800.0, 10.0);
        assert!((early - 440.0).abs() < 50.0, "early {early}");
        assert!((late - 440.0).abs() < 50.0, "late {late}");

        let transposed = render_artifact(&artifact, 440.0, 0.0, 24_000);
        let early_t = band_peak(&transposed[2_048..10_000], SR, 300.0, 900.0, 10.0);
        let late_t = band_peak(&transposed[14_000..22_000], SR, 400.0, 900.0, 10.0);
        assert!((early_t - 440.0).abs() < 50.0, "transposed early {early_t}");
        assert!(
            late_t > early_t * 1.2,
            "tune=0 should keep the melody: early {early_t} late {late_t}"
        );
    }

    #[test]
    fn compile_observes_cancellation() {
        let source = vec![0.25_f32; 48_000];
        let result = RichVocoderArtifact::compile_with_cancel(
            &source,
            48_000,
            220.0,
            ResynthQuality::Eco,
            &|| true,
        );
        assert!(matches!(result, Err(ArtifactBuildError::Cancelled)));
    }
}
