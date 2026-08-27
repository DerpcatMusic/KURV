//! Evoke-style source-filter RICH: pitch, spectral envelope, and residual
//! stay independent. The worker bakes frames; the audio thread synthesizes
//! from preallocated oscillators. No FFT, alloc, or locks on the callback.

use std::f32::consts::{PI, TAU};

use super::super::ResynthControls;
use super::super::analysis::{PitchTrack, PitchTrackFrame};
use super::super::quality::ResynthQuality;
use super::shared::{ArtifactBuildError, validate_source};
use crate::dsp::{Complex, fft};

pub const VOCODER_ENVELOPE_BINS: usize = 64;
pub const VOCODER_MAX_HARMONICS: usize = 128;
pub const VOCODER_MAX_FRAMES: usize = 8_192;
const RESIDUAL_BANDS: [f32; 4] = [1_500.0, 3_500.0, 7_000.0, 12_000.0];
const LIFTER_SECONDS: f32 = 0.002_5;
const MIN_VOICED: f32 = 0.05;

#[derive(Clone, Copy, Debug)]
pub struct RichVocoderFrame {
    pub f0_hz: f32,
    pub voiced: f32,
    pub gain: f32,
    pub aperiodicity: f32,
    pub envelope: [f32; VOCODER_ENVELOPE_BINS],
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

    pub(crate) fn compile_with_cancel(
        source: &[f32],
        source_sample_rate: u32,
        root_hz: f32,
        quality: ResynthQuality,
        should_cancel: &dyn Fn() -> bool,
    ) -> Result<Self, ArtifactBuildError> {
        validate_source(source)?;
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
        let mut spectrum = vec![Complex::ZERO; fft_size];
        let mut frames = Vec::with_capacity(points);
        let mut track = Vec::with_capacity(points);
        let mut peak_gain = f32::MIN_POSITIVE;
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
            let slice = &source[start..end];
            let (f0_hz, confidence) = super::super::estimate_root_window_with_cancel(
                slice,
                source_sample_rate,
                should_cancel,
            )
            .map_err(|_| ArtifactBuildError::Cancelled)?;
            let voiced = if f0_hz > 0.0 && confidence >= 0.2 {
                confidence.clamp(0.0, 1.0)
            } else {
                0.0
            };
            let f0_hz = if voiced > 0.0 { f0_hz } else { 0.0 };
            let (envelope, aperiodicity, gain) =
                analyze_envelope(slice, &mut spectrum, fft_size, sample_rate, f0_hz, voiced);
            peak_gain = peak_gain.max(gain);
            frames.push(RichVocoderFrame {
                f0_hz,
                voiced,
                gain,
                aperiodicity,
                envelope,
            });
            track.push(PitchTrackFrame {
                f0_hz,
                confidence: voiced,
                onset: 0.0,
            });
        }
        if frames.is_empty() {
            return Err(ArtifactBuildError::Empty);
        }
        let gain_norm = peak_gain.max(f32::MIN_POSITIVE);
        for frame in &mut frames {
            frame.gain = (frame.gain / gain_norm).clamp(0.0, 1.0);
        }
        let mut artifact = Self {
            sample_rate,
            source_frames: u32::try_from(source.len()).unwrap_or(u32::MAX),
            root_hz,
            nyquist,
            synth_gain: 1.0,
            quality,
            pitch_track: PitchTrack::from_frames(track),
            frames: frames.into_boxed_slice(),
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
        if !sample_rate.is_finite()
            || sample_rate <= 0.0
            || source_frames == 0
            || !root_hz.is_finite()
            || !(20.0..=2_000.0).contains(&root_hz)
            || !synth_gain.is_finite()
            || !(0.0..=16.0).contains(&synth_gain)
            || frames.is_empty()
            || frames.len() > VOCODER_MAX_FRAMES
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
        })
    }

    #[inline]
    pub fn lookup(&self, position: f32) -> InterpolatedFrame {
        let frames = self.frames.as_ref();
        if frames.is_empty() {
            return InterpolatedFrame::SILENT;
        }
        let scaled = position.clamp(0.0, 1.0) * frames.len().saturating_sub(1) as f32;
        let lower = scaled.floor() as usize;
        let upper = (lower + 1).min(frames.len() - 1);
        let mix = scaled - lower as f32;
        InterpolatedFrame::lerp(&frames[lower], &frames[upper], mix)
    }
}

#[derive(Clone, Copy)]
pub struct InterpolatedFrame {
    pub f0_hz: f32,
    pub voiced: f32,
    pub gain: f32,
    pub aperiodicity: f32,
    pub envelope: [f32; VOCODER_ENVELOPE_BINS],
}

impl InterpolatedFrame {
    const SILENT: Self = Self {
        f0_hz: 0.0,
        voiced: 0.0,
        gain: 0.0,
        aperiodicity: 1.0,
        envelope: [0.0; VOCODER_ENVELOPE_BINS],
    };

    fn lerp(first: &RichVocoderFrame, second: &RichVocoderFrame, mix: f32) -> Self {
        let mix = mix.clamp(0.0, 1.0);
        let f0_hz = if first.voiced > MIN_VOICED && second.voiced > MIN_VOICED {
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
        Self {
            f0_hz,
            voiced: first.voiced + (second.voiced - first.voiced) * mix,
            gain: first.gain + (second.gain - first.gain) * mix,
            aperiodicity: first.aperiodicity + (second.aperiodicity - first.aperiodicity) * mix,
            envelope,
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
    phases: [f32; VOCODER_MAX_HARMONICS],
    noise: u32,
    noise_lp: f32,
    bands: [f32; 4],
}

impl Default for RichVocoderState {
    fn default() -> Self {
        Self {
            phases: [0.0; VOCODER_MAX_HARMONICS],
            noise: 0xA341_316C,
            noise_lp: 0.0,
            bands: [0.0; 4],
        }
    }
}

impl RichVocoderState {
    #[inline]
    pub fn reset(&mut self) {
        *self = Self::default();
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
        let frame = artifact.lookup(timeline);
        let amount = controls.grain_tune.clamp(0.0, 1.0) * frame.voiced.clamp(0.0, 1.0);
        let f0_out = retune_f0(frame.f0_hz, played_hz, artifact.root_hz, amount);
        let formant = 2.0_f32
            .powf(controls.rich_formant_semitones / 12.0)
            .clamp(0.25, 4.0);
        let air_gain = 10.0_f32.powf(controls.rich_air_db / 20.0);
        let balance = controls.rich_balance.clamp(-1.0, 1.0);
        let (tonal_gain, residual_gain) = if balance <= 0.0 {
            let angle = (balance + 1.0) * 0.5 * PI;
            (angle.cos() + angle.sin(), angle.sin() * 0.35)
        } else {
            let angle = balance * 0.5 * PI;
            (angle.cos(), angle.sin())
        };
        let nyquist = sample_rate.max(1.0) * 0.45;
        let host_nyquist = artifact.nyquist.max(1.0);
        let mut harmonic = 0.0_f32;
        if f0_out > 20.0 && frame.voiced > MIN_VOICED {
            let max_h = artifact
                .quality
                .max_harmonics()
                .min((nyquist / f0_out).floor() as usize)
                .min(VOCODER_MAX_HARMONICS)
                .max(1);
            for index in 0..max_h {
                let hz = f0_out * (index + 1) as f32;
                if hz >= nyquist {
                    break;
                }
                let env_hz = (hz / formant).clamp(0.0, host_nyquist);
                let mag = envelope_at(&frame.envelope, env_hz, host_nyquist)
                    .clamp(-24.0, 4.0)
                    .exp();
                let shelf = if hz >= 8_000.0 { air_gain } else { 1.0 };
                let step = hz / sample_rate.max(1.0);
                let phase = (self.phases[index] + step).rem_euclid(1.0);
                self.phases[index] = phase;
                harmonic += mag * shelf * (phase * TAU).sin();
            }
        }
        let white = next_noise(&mut self.noise);
        let centroid = spectral_centroid(&frame.envelope, host_nyquist).clamp(200.0, 8_000.0);
        let coeff = 1.0 - (-TAU * centroid / sample_rate.max(1.0)).exp();
        self.noise_lp += coeff * (white - self.noise_lp);
        if self.noise_lp.abs() < 1.0e-20 {
            self.noise_lp = 0.0;
        }
        let mut split = white;
        let mut band_residual = 0.0_f32;
        for (index, cutoff) in RESIDUAL_BANDS.iter().copied().enumerate() {
            let a = 1.0 - (-TAU * cutoff / sample_rate.max(1.0)).exp();
            self.bands[index] += a * (split - self.bands[index]);
            if self.bands[index].abs() < 1.0e-20 {
                self.bands[index] = 0.0;
            }
            let low = self.bands[index];
            let high = split - low;
            let mag = envelope_at(&frame.envelope, cutoff.min(host_nyquist), host_nyquist)
                .clamp(-24.0, 4.0)
                .exp();
            band_residual += low * mag;
            split = high;
        }
        let env_rms = envelope_rms(&frame.envelope);
        let breath = controls.rich_diffuse.clamp(0.0, 1.0);
        let aper = frame.aperiodicity.clamp(0.0, 1.0);
        let residual = (self.noise_lp * (1.0 - breath * 0.35)
            + white * (0.12 + breath * 0.35)
            + band_residual * 0.45)
            * aper
            * env_rms.max(0.05)
            * residual_gain;
        let dynamic = (frame.gain - 1.0).mul_add(controls.rich_dynamic.clamp(0.0, 1.0), 1.0);
        (harmonic * tonal_gain * frame.voiced.max(MIN_VOICED) + residual)
            * dynamic
            * artifact.synth_gain
    }
}

fn analyze_envelope(
    slice: &[f32],
    spectrum: &mut [Complex],
    fft_size: usize,
    sample_rate: f32,
    f0_hz: f32,
    voiced: f32,
) -> ([f32; VOCODER_ENVELOPE_BINS], f32, f32) {
    spectrum.fill(Complex::ZERO);
    let count = slice.len().min(fft_size);
    let mut window_sum = 0.0_f64;
    let mut peak = 0.0_f32;
    let denominator = count.saturating_sub(1).max(1) as f32;
    for (index, sample) in slice.iter().copied().take(count).enumerate() {
        let hann = 0.5 - 0.5 * (TAU * index as f32 / denominator).cos();
        peak = peak.max(sample.abs());
        window_sum += f64::from(hann);
        spectrum[index].re = f64::from(sample * hann);
    }
    fft(spectrum, false);
    let half = fft_size / 2;
    let scale = 2.0 / window_sum.max(1.0e-9);
    let mut total_power = 0.0_f64;
    let mut harmonic_power = 0.0_f64;
    for bin in 0..=half {
        let mag = spectrum[bin].norm() * scale;
        let power = mag * mag;
        total_power += power;
        spectrum[bin] = Complex {
            re: (mag + 1.0e-12).ln(),
            im: 0.0,
        };
        if bin > 0 && bin < half {
            spectrum[fft_size - bin] = spectrum[bin];
        }
    }
    if f0_hz > 20.0 && voiced > MIN_VOICED {
        let bin_hz = f64::from(sample_rate) / fft_size as f64;
        let max_h = ((f64::from(sample_rate) * 0.45) / f64::from(f0_hz)).floor() as usize;
        for harmonic in 1..=max_h.min(64) {
            let center = (f64::from(f0_hz) * harmonic as f64 / bin_hz).round() as usize;
            if center == 0 || center >= half {
                break;
            }
            let lo = center.saturating_sub(1);
            let hi = (center + 1).min(half);
            for bin in lo..=hi {
                let mag = (spectrum[bin].re).exp();
                harmonic_power += mag * mag;
            }
        }
    }
    fft(spectrum, true);
    let lifter = if f0_hz > 20.0 {
        ((0.45 * sample_rate / f0_hz).min(sample_rate * LIFTER_SECONDS)) as usize
    } else {
        (sample_rate * LIFTER_SECONDS) as usize
    }
    .clamp(4, half / 4);
    for (index, bin) in spectrum.iter_mut().enumerate() {
        if index > lifter && index < fft_size - lifter {
            *bin = Complex::ZERO;
        }
    }
    fft(spectrum, false);
    let nyquist = sample_rate * 0.5;
    let bin_hz = sample_rate / fft_size as f32;
    let mut envelope = [0.0_f32; VOCODER_ENVELOPE_BINS];
    for (index, slot) in envelope.iter_mut().enumerate() {
        let hz = index as f32 / (VOCODER_ENVELOPE_BINS - 1) as f32 * nyquist;
        let pos = (hz / bin_hz).clamp(0.0, half as f32);
        let lo = pos.floor() as usize;
        let hi = (lo + 1).min(half);
        let mix = f64::from(pos - lo as f32);
        let log_mag = spectrum[lo].re + (spectrum[hi].re - spectrum[lo].re) * mix;
        *slot = log_mag as f32;
    }
    let aperiodicity = if total_power > 1.0e-20 {
        (1.0 - (harmonic_power / total_power) as f32).clamp(0.0, 1.0)
    } else {
        1.0
    };
    let aperiodicity = if voiced > MIN_VOICED {
        aperiodicity.max(1.0 - voiced)
    } else {
        1.0
    };
    (envelope, aperiodicity, peak)
}

#[inline]
fn envelope_at(envelope: &[f32; VOCODER_ENVELOPE_BINS], hz: f32, nyquist: f32) -> f32 {
    let pos = (hz / nyquist.max(1.0)).clamp(0.0, 1.0) * (VOCODER_ENVELOPE_BINS - 1) as f32;
    let lo = pos.floor() as usize;
    let hi = (lo + 1).min(VOCODER_ENVELOPE_BINS - 1);
    let mix = pos - lo as f32;
    envelope[lo] + (envelope[hi] - envelope[lo]) * mix
}

fn envelope_rms(envelope: &[f32; VOCODER_ENVELOPE_BINS]) -> f32 {
    let sum = envelope
        .iter()
        .map(|bin| {
            let mag = bin.exp();
            mag * mag
        })
        .sum::<f32>();
    (sum / VOCODER_ENVELOPE_BINS as f32).sqrt()
}

fn spectral_centroid(envelope: &[f32; VOCODER_ENVELOPE_BINS], nyquist: f32) -> f32 {
    let mut weighted = 0.0_f32;
    let mut total = 0.0_f32;
    for (index, bin) in envelope.iter().enumerate() {
        let mag = bin.exp();
        let hz = index as f32 / (VOCODER_ENVELOPE_BINS - 1) as f32 * nyquist;
        weighted += mag * hz;
        total += mag;
    }
    if total > 1.0e-8 {
        weighted / total
    } else {
        1_000.0
    }
}

fn next_noise(state: &mut u32) -> f32 {
    let mut value = *state;
    value ^= value << 13;
    value ^= value >> 17;
    value ^= value << 5;
    *state = value.max(1);
    (value as i32 as f32) * (1.0 / 2_147_483_648.0)
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
        let sample = state.render(
            artifact,
            timeline,
            artifact.root_hz,
            artifact.sample_rate,
            controls,
        );
        peak = peak.max(sample.abs());
    }
    (0.65 / peak.max(0.05)).clamp(0.05, 8.0)
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
