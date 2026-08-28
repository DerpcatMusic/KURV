use std::f64::consts::TAU;

use super::super::analysis::{PitchTrack, PitchTrackFrame};
use super::super::quality::ResynthQuality;
use super::shared::ArtifactBuildError;
use crate::dsp::{Complex, fft, shortest_angle};

const MIN_HARMONIC_HZ: f64 = 40.0;
const MAX_HARMONIC_HZ: f64 = 2_000.0;
const SEARCH_STEPS_PER_OCTAVE: f64 = 48.0;
const MAX_HARMONICS: usize = 32;
const MAX_SPECTRAL_PEAKS: usize = 64;

#[derive(Clone, Copy, Default)]
struct SpectralFrame {
    center: usize,
    anchor_hz: f64,
    confidence: f64,
    onset: f32,
}

pub(super) struct SpectralTuneResult {
    pub(super) tuned_mid: Vec<f32>,
    pub(super) tuned_side: Vec<f32>,
    pub(super) pitch_track: PitchTrack,
    pub(super) transients: Vec<u32>,
}

pub(super) fn tune_stereo_with_cancel(
    mid: &[f32],
    side: &[f32],
    sample_rate: f32,
    root_hz: Option<f32>,
    quality: ResynthQuality,
    should_cancel: &dyn Fn() -> bool,
) -> Result<SpectralTuneResult, ArtifactBuildError> {
    let fft_size = quality.fft_size();
    let hop_size = quality
        .reconstruction_hop()
        .max(mid.len().div_ceil(quality.max_points()).max(1));
    let (frames, transients) = analyze_spectral_frames_with_cancel(
        mid,
        side,
        sample_rate,
        fft_size,
        hop_size,
        should_cancel,
    )?;
    let pitch_track = pitch_track_from_frames(&frames);

    let Some(root_hz) = root_hz.filter(|root| root.is_finite() && *root > 0.0) else {
        return Ok(SpectralTuneResult {
            tuned_mid: Vec::new(),
            tuned_side: Vec::new(),
            pitch_track,
            transients,
        });
    };
    let mut tuned_mid = tune_channel(
        mid,
        sample_rate,
        root_hz,
        fft_size,
        hop_size,
        &frames,
        should_cancel,
    )?;
    let mut tuned_side = if side.is_empty() {
        Vec::new()
    } else {
        tune_channel(
            side,
            sample_rate,
            root_hz,
            fft_size,
            hop_size,
            &frames,
            should_cancel,
        )?
    };
    match_dry_level(mid, side, &mut tuned_mid, &mut tuned_side);
    Ok(SpectralTuneResult {
        tuned_mid,
        tuned_side,
        pitch_track,
        transients,
    })
}

pub(super) fn spectral_pitch_track_with_cancel(
    mid: &[f32],
    side: &[f32],
    sample_rate: f32,
    quality: ResynthQuality,
    should_cancel: &dyn Fn() -> bool,
) -> Result<PitchTrack, ArtifactBuildError> {
    let fft_size = quality.fft_size();
    let hop_size = quality
        .reconstruction_hop()
        .max(mid.len().div_ceil(quality.max_points()).max(1));
    let (frames, _) = analyze_spectral_frames_with_cancel(
        mid,
        side,
        sample_rate,
        fft_size,
        hop_size,
        should_cancel,
    )?;
    Ok(pitch_track_from_frames(&frames))
}

fn analyze_spectral_frames_with_cancel(
    mid: &[f32],
    side: &[f32],
    sample_rate: f32,
    fft_size: usize,
    hop_size: usize,
    should_cancel: &dyn Fn() -> bool,
) -> Result<(Vec<SpectralFrame>, Vec<u32>), ArtifactBuildError> {
    let mut frames = analyze_frames(mid, side, sample_rate, fft_size, hop_size, should_cancel)?;
    let transients = mark_spectral_onsets(&mut frames, sample_rate, mid.len());
    Ok((frames, transients))
}

fn pitch_track_from_frames(frames: &[SpectralFrame]) -> PitchTrack {
    PitchTrack::from_frames(
        frames
            .iter()
            .map(|frame| PitchTrackFrame {
                f0_hz: frame.anchor_hz as f32,
                confidence: frame.confidence as f32,
                onset: frame.onset,
            })
            .collect(),
    )
}

fn analyze_frames(
    mid: &[f32],
    side: &[f32],
    sample_rate: f32,
    fft_size: usize,
    hop_size: usize,
    should_cancel: &dyn Fn() -> bool,
) -> Result<Vec<SpectralFrame>, ArtifactBuildError> {
    let frame_count = mid.len().div_ceil(hop_size) + 1;
    let half = fft_size / 2;
    let mut mid_spectrum = vec![Complex::ZERO; fft_size];
    let mut side_spectrum = vec![Complex::ZERO; fft_size];
    let mut magnitudes = vec![0.0_f64; half + 1];
    let mut previous_magnitudes = vec![0.0_f64; half + 1];
    let mut flux = Vec::with_capacity(frame_count);
    let mut frames = Vec::with_capacity(frame_count);
    let mut previous_anchor = 0.0_f64;

    for frame_index in 0..frame_count {
        if should_cancel() {
            return Err(ArtifactBuildError::Cancelled);
        }
        let center = frame_index * hop_size;
        let start = center as isize - fft_size as isize / 2;
        for index in 0..fft_size {
            let source_index = start + index as isize;
            let window = 0.5 - 0.5 * (TAU * index as f64 / fft_size as f64).cos();
            mid_spectrum[index] = Complex::new(
                if (0..mid.len() as isize).contains(&source_index) {
                    f64::from(mid[source_index as usize]) * window
                } else {
                    0.0
                },
                0.0,
            );
            side_spectrum[index] = Complex::new(
                if (0..side.len() as isize).contains(&source_index) {
                    f64::from(side[source_index as usize]) * window
                } else {
                    0.0
                },
                0.0,
            );
        }
        fft(&mut mid_spectrum, false);
        if !side.is_empty() {
            fft(&mut side_spectrum, false);
        }

        let mut positive_flux = 0.0_f64;
        let mut spectral_mass = 0.0_f64;
        for bin in 1..=half {
            let mid_power = mid_spectrum[bin].norm_sqr();
            let side_power = if side.is_empty() {
                0.0
            } else {
                side_spectrum[bin].norm_sqr()
            };
            let linear_magnitude = (mid_power + side_power).sqrt();
            let magnitude = linear_magnitude.ln_1p();
            positive_flux += (magnitude - previous_magnitudes[bin]).max(0.0);
            spectral_mass += magnitude;
            previous_magnitudes[bin] = magnitude;
            magnitudes[bin] = linear_magnitude;
        }
        flux.push(positive_flux / spectral_mass.max(1.0e-12));
        let (anchor_hz, confidence) = harmonic_anchor(&magnitudes, sample_rate, previous_anchor);
        if confidence >= 0.15 {
            previous_anchor = anchor_hz;
        }
        frames.push(SpectralFrame {
            center,
            anchor_hz,
            confidence,
            onset: 0.0,
        });
    }

    // Store flux temporarily in onset. `mark_spectral_onsets` replaces it with a gate.
    for (frame, flux) in frames.iter_mut().zip(flux) {
        frame.onset = flux as f32;
    }
    Ok(frames)
}

fn harmonic_anchor(magnitudes: &[f64], sample_rate: f32, previous: f64) -> (f64, f64) {
    let fft_size = (magnitudes.len().saturating_sub(1) * 2).max(2);
    let bin_hz = f64::from(sample_rate) / fft_size as f64;
    let nyquist = f64::from(sample_rate) * 0.5;
    let upper = MAX_HARMONIC_HZ.min(nyquist * 0.5).max(MIN_HARMONIC_HZ);
    let total = magnitudes.iter().skip(1).sum::<f64>();
    if total <= 1.0e-12 || upper <= MIN_HARMONIC_HZ {
        return (0.0, 0.0);
    }

    let maximum = magnitudes.iter().copied().fold(0.0_f64, f64::max);
    let mut peaks = [(0.0_f64, 0.0_f64); MAX_SPECTRAL_PEAKS];
    let mut peak_count = 0_usize;
    for bin in 2..magnitudes.len().saturating_sub(1) {
        let magnitude = magnitudes[bin];
        if magnitude < maximum * 0.02
            || magnitude <= magnitudes[bin - 1]
            || magnitude < magnitudes[bin + 1]
        {
            continue;
        }
        let denominator = magnitudes[bin - 1] - 2.0 * magnitude + magnitudes[bin + 1];
        let offset = if denominator.abs() > 1.0e-12 {
            (0.5 * (magnitudes[bin - 1] - magnitudes[bin + 1]) / denominator).clamp(-0.5, 0.5)
        } else {
            0.0
        };
        let peak = ((bin as f64 + offset) * bin_hz, magnitude);
        let insertion = peaks[..peak_count]
            .iter()
            .position(|other| magnitude > other.1)
            .unwrap_or(peak_count);
        if insertion >= MAX_SPECTRAL_PEAKS {
            continue;
        }
        let last = peak_count.min(MAX_SPECTRAL_PEAKS - 1);
        for index in (insertion..last).rev() {
            peaks[index + 1] = peaks[index];
        }
        peaks[insertion] = peak;
        peak_count = (peak_count + 1).min(MAX_SPECTRAL_PEAKS);
    }
    if peak_count == 0 {
        return (0.0, 0.0);
    }

    let steps = ((upper / MIN_HARMONIC_HZ).log2() * SEARCH_STEPS_PER_OCTAVE).ceil() as usize;
    let mut best_hz = 0.0_f64;
    let mut best_score = 0.0_f64;
    let mut best_selection = 0.0_f64;
    for step in 0..=steps {
        let candidate = MIN_HARMONIC_HZ * 2.0_f64.powf(step as f64 / SEARCH_STEPS_PER_OCTAVE);
        if candidate > upper {
            break;
        }
        let score = harmonic_score(&peaks[..peak_count], candidate);
        let continuity = if previous > 0.0 {
            1.0 + 0.025 / (1.0 + (candidate / previous).log2().abs() * 8.0)
        } else {
            1.0
        };
        let selection_score = score * continuity;
        if selection_score > best_selection {
            best_selection = selection_score;
            best_score = score;
            best_hz = candidate;
        }
    }
    if best_hz <= 0.0 {
        return (0.0, 0.0);
    }

    let refinement_step = 1.0 / (SEARCH_STEPS_PER_OCTAVE * 4.0);
    for offset in -4..=4 {
        let candidate = best_hz * 2.0_f64.powf(offset as f64 * refinement_step);
        let score = harmonic_score(&peaks[..peak_count], candidate);
        if score > best_score {
            best_score = score;
            best_hz = candidate;
        }
    }
    let tonality = (best_score / total * 3.0).clamp(0.0, 1.0);
    let confidence = tonality;
    if confidence < 0.12 {
        (0.0, 0.0)
    } else {
        (best_hz, confidence)
    }
}

fn harmonic_score(peaks: &[(f64, f64)], anchor: f64) -> f64 {
    let mut score = 0.0_f64;
    for (frequency, magnitude) in peaks.iter().copied() {
        let harmonic = (frequency / anchor).round();
        if !(1.0..=MAX_HARMONICS as f64).contains(&harmonic) {
            continue;
        }
        let target = anchor * harmonic;
        let detune_cents = 1_200.0 * (frequency / target).log2().abs();
        let agreement = (1.0 - detune_cents / 55.0).clamp(0.0, 1.0);
        score += magnitude * agreement * agreement / harmonic.sqrt().sqrt();
    }
    score
}

fn mark_spectral_onsets(
    frames: &mut [SpectralFrame],
    sample_rate: f32,
    source_len: usize,
) -> Vec<u32> {
    let mut transients = Vec::with_capacity(128);
    let minimum_gap = (sample_rate * 0.02).round().max(1.0) as usize;
    let mut last = None;
    let flux = frames.iter().map(|frame| frame.onset).collect::<Vec<_>>();
    for frame in frames.iter_mut() {
        frame.onset = 0.0;
    }
    for index in 1..frames.len().saturating_sub(1) {
        let start = index.saturating_sub(12);
        let baseline = flux[start..index].iter().sum::<f32>() / (index - start).max(1) as f32;
        let current_flux = flux[index];
        let is_peak = current_flux >= flux[index - 1] && current_flux > flux[index + 1];
        let separated =
            last.is_none_or(|last| frames[index].center.saturating_sub(last) >= minimum_gap);
        let onset = transients.len() < 128
            && is_peak
            && current_flux > 0.01
            && current_flux > baseline.mul_add(2.25, 0.002)
            && separated;
        frames[index].onset = f32::from(onset);
        if onset {
            transients.push(
                u32::try_from(frames[index].center.min(source_len.saturating_sub(1)))
                    .unwrap_or(u32::MAX),
            );
            last = Some(frames[index].center);
        }
    }
    transients
}

fn tune_channel(
    source: &[f32],
    sample_rate: f32,
    root_hz: f32,
    fft_size: usize,
    hop_size: usize,
    frames: &[SpectralFrame],
    should_cancel: &dyn Fn() -> bool,
) -> Result<Vec<f32>, ArtifactBuildError> {
    if source.is_empty() {
        return Ok(Vec::new());
    }
    let mut output = vec![0.0_f64; source.len()];
    let mut weight = vec![0.0_f64; source.len()];
    let mut spectrum = vec![Complex::ZERO; fft_size];
    let mut shifted = vec![Complex::ZERO; fft_size];
    let mut previous_phase = vec![0.0_f64; fft_size / 2 + 1];
    let mut synthesis_phase = vec![0.0_f64; fft_size / 2 + 1];
    let mut initialized = false;
    let bin_hz = f64::from(sample_rate) / fft_size as f64;

    for frame in frames {
        if should_cancel() {
            return Err(ArtifactBuildError::Cancelled);
        }
        let start = frame.center as isize - fft_size as isize / 2;
        for (index, bin) in spectrum.iter_mut().enumerate() {
            let source_index = start + index as isize;
            let hann = 0.5 - 0.5 * (TAU * index as f64 / fft_size as f64).cos();
            bin.re = if (0..source.len() as isize).contains(&source_index) {
                f64::from(source[source_index as usize]) * hann.sqrt()
            } else {
                0.0
            };
            bin.im = 0.0;
        }
        fft(&mut spectrum, false);
        shifted.fill(Complex::ZERO);
        let anchor = frame.anchor_hz;
        let ratio = if anchor > 0.0 {
            (f64::from(root_hz) / anchor).clamp(0.01, 100.0)
        } else {
            1.0
        };
        let lock = (frame.confidence * (1.0 - f64::from(frame.onset))).clamp(0.0, 1.0);
        let harmonic_width = if anchor > 0.0 {
            (bin_hz * 1.75 / anchor).clamp(0.05, 0.42)
        } else {
            0.0
        };
        for bin in 1..fft_size / 2 {
            let value = spectrum[bin];
            let magnitude = value.norm();
            let phase = value.arg();
            let expected = TAU * bin as f64 * hop_size as f64 / fft_size as f64;
            let delta = shortest_angle(previous_phase[bin] + expected, phase);
            previous_phase[bin] = phase;
            let true_bin = bin as f64 + delta * fft_size as f64 / (TAU * hop_size as f64);
            let frequency = true_bin.max(0.0) * bin_hz;
            let harmonic = if anchor > 0.0 {
                frequency / anchor
            } else {
                0.0
            };
            let distance = (harmonic - harmonic.round()).abs();
            let mut tonal = if harmonic >= 0.75 && harmonic_width > 0.0 {
                (1.0 - distance / harmonic_width).clamp(0.0, 1.0) * lock
            } else {
                0.0
            };
            if frequency > 8_000.0 {
                let high_span = (f64::from(sample_rate) * 0.5 - 8_000.0).max(1.0);
                tonal *= 1.0 - 0.75 * ((frequency - 8_000.0) / high_span).clamp(0.0, 1.0);
            }
            shifted[bin] += value * (1.0 - tonal);
            if tonal <= f64::EPSILON {
                continue;
            }
            let target = true_bin * ratio;
            if !(1.0..fft_size as f64 / 2.0).contains(&target) {
                continue;
            }
            if initialized {
                synthesis_phase[bin] += TAU * target * hop_size as f64 / fft_size as f64;
            } else {
                synthesis_phase[bin] = phase;
            }
            let first = target.floor() as usize;
            let mix = target - first as f64;
            let partial = Complex::from_polar(magnitude * tonal, synthesis_phase[bin]);
            shifted[first] += partial * (1.0 - mix);
            shifted[first + 1] += partial * mix;
        }
        initialized = true;
        shifted[0] = spectrum[0];
        shifted[fft_size / 2] = spectrum[fft_size / 2];
        for bin in 1..fft_size / 2 {
            shifted[fft_size - bin] = shifted[bin].conj();
        }
        fft(&mut shifted, true);
        for (index, bin) in shifted.iter().enumerate() {
            let output_index = start + index as isize;
            if !(0..output.len() as isize).contains(&output_index) {
                continue;
            }
            let hann = 0.5 - 0.5 * (TAU * index as f64 / fft_size as f64).cos();
            output[output_index as usize] += bin.re * hann.sqrt();
            weight[output_index as usize] += hann;
        }
    }
    Ok(output
        .into_iter()
        .zip(weight)
        .map(|(sample, weight)| (sample / weight.max(1.0e-9)) as f32)
        .collect())
}

fn match_dry_level(mid: &[f32], side: &[f32], tuned_mid: &mut [f32], tuned_side: &mut [f32]) {
    let mean = |samples: &[f32]| {
        samples.iter().map(|sample| f64::from(*sample)).sum::<f64>() / samples.len().max(1) as f64
    };
    let tuned_mid_mean = mean(tuned_mid);
    let tuned_side_mean = mean(tuned_side);
    for sample in tuned_mid.iter_mut() {
        *sample = (f64::from(*sample) - tuned_mid_mean) as f32;
    }
    for sample in tuned_side.iter_mut() {
        *sample = (f64::from(*sample) - tuned_side_mean) as f32;
    }

    let stereo_power = |mid: &[f32], side: &[f32]| {
        mid.iter()
            .enumerate()
            .map(|(index, mid)| {
                let mid = f64::from(*mid);
                let side = f64::from(side.get(index).copied().unwrap_or(0.0));
                (mid + side).mul_add(mid + side, (mid - side) * (mid - side)) * 0.5
            })
            .sum::<f64>()
            / mid.len().max(1) as f64
    };
    let dry_rms = stereo_power(mid, side).sqrt();
    let tuned_rms = stereo_power(tuned_mid, tuned_side).sqrt();
    let mut gain = if tuned_rms > 1.0e-12 {
        (dry_rms / tuned_rms).clamp(0.25, 4.0)
    } else {
        1.0
    };
    let peak = tuned_mid
        .iter()
        .enumerate()
        .map(|(index, mid)| {
            let side = tuned_side.get(index).copied().unwrap_or(0.0);
            (mid + side).abs().max((mid - side).abs()) as f64
        })
        .fold(0.0_f64, f64::max);
    if peak * gain > 1.0 {
        gain = 1.0 / peak.max(1.0e-12);
    }
    for sample in tuned_mid.iter_mut().chain(tuned_side.iter_mut()) {
        *sample = (f64::from(*sample) * gain) as f32;
    }
}
