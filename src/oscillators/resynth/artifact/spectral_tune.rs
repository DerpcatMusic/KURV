use std::f64::consts::TAU;

use super::super::analysis::{PitchTrack, PitchTrackFrame};
use super::super::quality::ResynthQuality;
use super::shared::ArtifactBuildError;
use crate::dsp::{Complex, fft};

const MIN_NOTE_HZ: f64 = 40.0;
const MAX_NOTE_HZ: f64 = 2_000.0;
const MAX_HARMONICS: f64 = 16.0;
const MAX_NOTES: usize = 4;
const MAX_SPECTRAL_PEAKS: usize = 64;
const MAX_LIVE_PEAKS: usize = 32;
const HARMONIC_CENTS: f64 = 55.0;
const DUPLICATE_CENTS: f64 = 40.0;
const TRACK_CENTS: f64 = 80.0;

#[derive(Clone, Copy, Default)]
struct Note {
    hz: f64,
    confidence: f64,
}

#[derive(Clone, Copy, Default)]
struct SpectralFrame {
    center: usize,
    notes: [Note; MAX_NOTES],
    note_count: u8,
    confidence: f64,
    onset: f32,
}

#[derive(Clone, Copy, Default)]
struct Peak {
    hz: f64,
    magnitude: f64,
    phase: f64,
}

#[derive(Clone, Copy, Default)]
struct LivePeak {
    src_hz: f64,
    synth_phase: f64,
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
    spectral_pitch_track_and_transients_with_cancel(mid, side, sample_rate, quality, should_cancel)
        .map(|(track, _)| track)
}

pub(super) fn spectral_pitch_track_and_transients_with_cancel(
    mid: &[f32],
    side: &[f32],
    sample_rate: f32,
    quality: ResynthQuality,
    should_cancel: &dyn Fn() -> bool,
) -> Result<(PitchTrack, Vec<u32>), ArtifactBuildError> {
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
    Ok((pitch_track_from_frames(&frames), transients))
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
                f0_hz: frame.notes[0].hz as f32,
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
        fill_windowed_spectrum(&mut mid_spectrum, mid, center, fft_size, Window::Hann);
        if side.is_empty() {
            side_spectrum.fill(Complex::ZERO);
        } else {
            fill_windowed_spectrum(&mut side_spectrum, side, center, fft_size, Window::Hann);
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
        let peaks = pick_peaks(&magnitudes, None, sample_rate, fft_size);
        let notes = detect_notes(&peaks, &magnitudes, sample_rate, fft_size, previous_anchor);
        let confidence = notes[0].confidence;
        if confidence >= 0.15 {
            previous_anchor = notes[0].hz;
        }
        let mut note_count = 0_u8;
        for note in notes {
            if note.hz > 0.0 {
                note_count += 1;
            }
        }
        frames.push(SpectralFrame {
            center,
            notes,
            note_count,
            confidence,
            onset: 0.0,
        });
    }

    for (frame, flux) in frames.iter_mut().zip(flux) {
        frame.onset = flux as f32;
    }
    Ok(frames)
}

fn bin_magnitude(magnitudes: &[f64], hz: f64, bin_hz: f64) -> f64 {
    if bin_hz <= 0.0 {
        return 0.0;
    }
    let bin = (hz / bin_hz).round() as usize;
    magnitudes.get(bin).copied().unwrap_or(0.0)
}

fn detect_notes(
    peaks: &[Peak],
    magnitudes: &[f64],
    sample_rate: f32,
    fft_size: usize,
    previous: f64,
) -> [Note; MAX_NOTES] {
    let mut notes = [Note::default(); MAX_NOTES];
    if peaks.is_empty() {
        return notes;
    }
    let bin_hz = f64::from(sample_rate) / fft_size.max(2) as f64;
    let maximum = magnitudes.iter().copied().fold(0.0_f64, f64::max);
    let mut seeds = [0.0_f64; MAX_SPECTRAL_PEAKS * 2];
    let mut seed_count = 0_usize;
    let mut push_seed = |hz: f64| {
        if !(MIN_NOTE_HZ..=MAX_NOTE_HZ).contains(&hz) || seed_count >= seeds.len() {
            return;
        }
        if seeds[..seed_count]
            .iter()
            .any(|existing| cents(*existing, hz).abs() < 20.0)
        {
            return;
        }
        seeds[seed_count] = hz;
        seed_count += 1;
    };
    for peak in peaks.iter().copied() {
        push_seed(peak.hz);
        let half = peak.hz * 0.5;
        let half_mag = bin_magnitude(magnitudes, half, bin_hz);
        if half_mag > peak.magnitude * 0.05 && half_mag > maximum * 0.01 {
            push_seed(half);
        }
    }
    let mut candidates = [(0.0_f64, 0.0_f64); MAX_SPECTRAL_PEAKS];
    let mut candidate_count = 0_usize;
    let mut best_score = 0.0_f64;
    for &hz in seeds.iter().take(seed_count) {
        let mut score = note_score(peaks, hz);
        if previous > 0.0 {
            let jump = cents(hz, previous).abs();
            score *= 1.0 + 0.04 / (1.0 + jump / 100.0);
        }
        if score <= 0.0 {
            continue;
        }
        best_score = best_score.max(score);
        let insertion = candidates[..candidate_count]
            .iter()
            .position(|other| score > other.1)
            .unwrap_or(candidate_count);
        if insertion >= MAX_SPECTRAL_PEAKS {
            continue;
        }
        let last = candidate_count.min(MAX_SPECTRAL_PEAKS - 1);
        for index in (insertion..last).rev() {
            candidates[index + 1] = candidates[index];
        }
        candidates[insertion] = (hz, score);
        candidate_count = (candidate_count + 1).min(MAX_SPECTRAL_PEAKS);
    }
    let floor = (best_score * 0.12).max(1.0e-12);
    let mut note_count = 0_usize;
    for &(hz, score) in candidates.iter().take(candidate_count) {
        if note_count >= MAX_NOTES || score < floor {
            break;
        }
        if notes[..note_count]
            .iter()
            .any(|note| notes_are_harmonic(hz, note.hz))
        {
            continue;
        }
        let refined = refine_note(peaks, hz);
        notes[note_count] = Note {
            hz: refined,
            confidence: (score / best_score).clamp(0.0, 1.0),
        };
        note_count += 1;
    }
    notes
}

fn note_score(peaks: &[Peak], f0: f64) -> f64 {
    if f0 <= 0.0 {
        return 0.0;
    }
    let mut score = 0.0_f64;
    let mut harmonics = 0.0_f64;
    let mut has_fundamental = false;
    for peak in peaks.iter().copied() {
        let harmonic = (peak.hz / f0).round();
        if !(1.0..=MAX_HARMONICS).contains(&harmonic) {
            continue;
        }
        let target = f0 * harmonic;
        let agreement = (1.0 - cents(peak.hz, target).abs() / HARMONIC_CENTS).clamp(0.0, 1.0);
        if agreement <= 0.0 {
            continue;
        }
        if harmonic <= 1.0 {
            has_fundamental = true;
        }
        harmonics += 1.0;
        score += peak.magnitude * agreement * agreement / harmonic.sqrt();
    }
    if has_fundamental {
        score * (1.0 + 0.35 * (harmonics - 1.0).max(0.0))
    } else {
        0.0
    }
}

fn refine_note(peaks: &[Peak], f0: f64) -> f64 {
    let step = 1.0 / 192.0;
    let mut best_hz = f0;
    let mut best_score = note_score(peaks, f0);
    for offset in -4..=4 {
        if offset == 0 {
            continue;
        }
        let candidate = f0 * 2.0_f64.powf(f64::from(offset) * step);
        let score = note_score(peaks, candidate);
        if score > best_score {
            best_score = score;
            best_hz = candidate;
        }
    }
    best_hz
}

fn notes_are_harmonic(left: f64, right: f64) -> bool {
    let (low, high) = if left <= right {
        (left, right)
    } else {
        (right, left)
    };
    if low <= 0.0 {
        return false;
    }
    let ratio = high / low;
    let harmonic = ratio.round();
    (2.0..=8.0).contains(&harmonic) && cents(ratio, harmonic).abs() < DUPLICATE_CENTS
}

fn pick_peaks(
    magnitudes: &[f64],
    spectrum: Option<&[Complex]>,
    sample_rate: f32,
    fft_size: usize,
) -> Vec<Peak> {
    let bin_hz = f64::from(sample_rate) / fft_size.max(2) as f64;
    let maximum = magnitudes.iter().copied().fold(0.0_f64, f64::max);
    if maximum <= 1.0e-12 {
        return Vec::new();
    }
    let mut peaks = Vec::with_capacity(32);
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
        let hz = (bin as f64 + offset) * bin_hz;
        if hz < MIN_NOTE_HZ || hz > f64::from(sample_rate) * 0.48 {
            continue;
        }
        let phase = spectrum
            .and_then(|spectrum| spectrum.get(bin))
            .map_or(0.0, |bin| bin.arg());
        let insertion = peaks
            .iter()
            .position(|other: &Peak| magnitude > other.magnitude)
            .unwrap_or(peaks.len());
        if insertion >= MAX_SPECTRAL_PEAKS {
            continue;
        }
        peaks.insert(
            insertion,
            Peak {
                hz,
                magnitude,
                phase,
            },
        );
        if peaks.len() > MAX_SPECTRAL_PEAKS {
            peaks.pop();
        }
    }
    peaks
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

#[derive(Clone, Copy)]
enum Window {
    Hann,
    SqrtHann,
}

fn window_value(kind: Window, index: usize, fft_size: usize) -> f64 {
    let hann = 0.5 - 0.5 * (TAU * index as f64 / fft_size as f64).cos();
    match kind {
        Window::Hann => hann,
        Window::SqrtHann => hann.sqrt(),
    }
}

fn fill_windowed_spectrum(
    spectrum: &mut [Complex],
    source: &[f32],
    center: usize,
    fft_size: usize,
    window: Window,
) {
    let start = center as isize - fft_size as isize / 2;
    for (index, bin) in spectrum.iter_mut().enumerate() {
        let source_index = start + index as isize;
        let weight = window_value(window, index, fft_size);
        bin.re = if (0..source.len() as isize).contains(&source_index) {
            f64::from(source[source_index as usize]) * weight
        } else {
            0.0
        };
        bin.im = 0.0;
    }
}

fn assign_note(peak_hz: f64, notes: &[Note], note_count: usize) -> Option<Note> {
    let mut best: Option<(Note, f64, usize)> = None;
    for (index, note) in notes.iter().copied().take(note_count).enumerate() {
        if note.hz <= 0.0 {
            continue;
        }
        let harmonic = (peak_hz / note.hz).round();
        if !(1.0..=MAX_HARMONICS).contains(&harmonic) {
            continue;
        }
        let error = cents(peak_hz, note.hz * harmonic).abs();
        if error > HARMONIC_CENTS {
            continue;
        }
        let better = match best {
            None => true,
            Some((_, current_error, current_index)) => {
                error < current_error
                    || ((error - current_error).abs() <= 1.0 && index < current_index)
            }
        };
        if better {
            best = Some((note, error, index));
        }
    }
    best.map(|(note, _, _)| note)
}

fn notch_peak(spectrum: &mut [Complex], hz: f64, bin_hz: f64, fft_size: usize) {
    let half = fft_size / 2;
    let center = (hz / bin_hz).round() as isize;
    for delta in -2..=2 {
        let bin = center + delta;
        if bin > 0 && (bin as usize) < half {
            spectrum[bin as usize] = Complex::ZERO;
        }
    }
}

#[derive(Clone, Copy, Default)]
struct DestPartial {
    hz: f64,
    magnitude: f64,
    phase: f64,
}

fn accumulate_dest(
    dests: &mut [DestPartial],
    count: &mut usize,
    hz: f64,
    magnitude: f64,
    phase: f64,
) {
    if let Some(slot) = dests
        .iter_mut()
        .take(*count)
        .find(|dest| cents(dest.hz, hz).abs() < 10.0)
    {
        slot.magnitude += magnitude;
        return;
    }
    if *count < dests.len() {
        dests[*count] = DestPartial {
            hz,
            magnitude,
            phase,
        };
        *count += 1;
    }
}

fn cents(left: f64, right: f64) -> f64 {
    if left <= 0.0 || right <= 0.0 {
        return 10_000.0;
    }
    1_200.0 * (left / right).log2()
}

fn match_live_peak(live: &[LivePeak], count: usize, hz: f64) -> Option<f64> {
    let mut best = None;
    for peak in live.iter().copied().take(count) {
        let error = cents(peak.src_hz, hz).abs();
        if error > TRACK_CENTS {
            continue;
        }
        if best.is_none_or(|(_, best_error)| error < best_error) {
            best = Some((peak.synth_phase, error));
        }
    }
    best.map(|(phase, _)| phase)
}

fn store_live_peak(live: &mut [LivePeak], count: &mut usize, hz: f64, synth_phase: f64) {
    if let Some(slot) = live
        .iter_mut()
        .take(*count)
        .find(|peak| cents(peak.src_hz, hz).abs() <= TRACK_CENTS)
    {
        slot.src_hz = hz;
        slot.synth_phase = synth_phase;
        return;
    }
    if *count < live.len() {
        live[*count] = LivePeak {
            src_hz: hz,
            synth_phase,
        };
        *count += 1;
    }
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
    let mut magnitudes = vec![0.0_f64; fft_size / 2 + 1];
    let mut live = [LivePeak::default(); MAX_LIVE_PEAKS];
    let mut live_count = 0_usize;
    let mut dests = [DestPartial::default(); MAX_LIVE_PEAKS];
    let bin_hz = f64::from(sample_rate) / fft_size as f64;
    let nyquist = f64::from(sample_rate) * 0.49;
    let root = f64::from(root_hz);
    let coherent = (0..fft_size)
        .map(|index| window_value(Window::SqrtHann, index, fft_size))
        .sum::<f64>()
        .max(1.0);

    for frame in frames {
        if should_cancel() {
            return Err(ArtifactBuildError::Cancelled);
        }
        let start = frame.center as isize - fft_size as isize / 2;
        fill_windowed_spectrum(
            &mut spectrum,
            source,
            frame.center,
            fft_size,
            Window::SqrtHann,
        );
        fft(&mut spectrum, false);
        shifted.copy_from_slice(&spectrum);
        let lock = (frame.confidence * (1.0 - f64::from(frame.onset))).clamp(0.0, 1.0);
        let mut dest_count = 0_usize;
        dests.fill(DestPartial::default());
        if lock > 0.05 && frame.note_count > 0 {
            for bin in 1..=fft_size / 2 {
                magnitudes[bin] = spectrum[bin].norm();
            }
            let peaks = pick_peaks(&magnitudes, Some(&spectrum), sample_rate, fft_size);
            let mut next_live = [LivePeak::default(); MAX_LIVE_PEAKS];
            let mut next_count = 0_usize;
            for peak in peaks {
                let Some(note) = assign_note(peak.hz, &frame.notes, usize::from(frame.note_count))
                else {
                    continue;
                };
                notch_peak(&mut shifted, peak.hz, bin_hz, fft_size);
                let harmonic = (peak.hz / note.hz).round().max(1.0);
                let dest_hz = (harmonic * root).clamp(MIN_NOTE_HZ, nyquist);
                let phase = match_live_peak(&live, live_count, dest_hz)
                    .map_or(peak.phase, |previous| {
                        previous + TAU * dest_hz * hop_size as f64 / f64::from(sample_rate)
                    });
                accumulate_dest(
                    &mut dests,
                    &mut dest_count,
                    dest_hz,
                    peak.magnitude * lock,
                    phase,
                );
                if lock < 1.0 {
                    accumulate_dest(
                        &mut dests,
                        &mut dest_count,
                        peak.hz,
                        peak.magnitude * (1.0 - lock),
                        peak.phase,
                    );
                }
                store_live_peak(&mut next_live, &mut next_count, dest_hz, phase);
            }
            live = next_live;
            live_count = next_count;
        }
        shifted[0] = spectrum[0];
        shifted[fft_size / 2] = spectrum[fft_size / 2];
        for bin in 1..fft_size / 2 {
            shifted[fft_size - bin] = shifted[bin].conj();
        }
        fft(&mut shifted, true);
        let omega_scale = TAU / f64::from(sample_rate);
        for (index, bin) in shifted.iter().enumerate() {
            let output_index = start + index as isize;
            if !(0..output.len() as isize).contains(&output_index) {
                continue;
            }
            let hann = window_value(Window::Hann, index, fft_size);
            let mut sample = bin.re * hann.sqrt();
            for dest in dests.iter().copied().take(dest_count) {
                let amplitude = 2.0 * dest.magnitude / coherent;
                sample +=
                    amplitude * hann * (dest.phase + omega_scale * dest.hz * index as f64).cos();
            }
            output[output_index as usize] += sample;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oscillators::ResynthQuality;
    use std::f64::consts::TAU;

    const SAMPLE_RATE: f32 = 48_000.0;
    const ROOT: f32 = 220.0;

    fn tone(hz: f32, n: usize, amp: f32) -> Vec<f32> {
        (0..n)
            .map(|index| {
                (TAU * f64::from(hz) * index as f64 / f64::from(SAMPLE_RATE)).sin() as f32 * amp
            })
            .collect()
    }

    fn tone_amp(samples: &[f32], hz: f32) -> f32 {
        let (re, im) = samples.iter().copied().enumerate().fold(
            (0.0_f64, 0.0_f64),
            |(re, im), (index, sample)| {
                let angle = TAU * f64::from(hz) * index as f64 / f64::from(SAMPLE_RATE);
                (
                    re + f64::from(sample) * angle.cos(),
                    im - f64::from(sample) * angle.sin(),
                )
            },
        );
        (2.0 * re.hypot(im) / samples.len().max(1) as f64) as f32
    }

    fn grid(samples: &[f32]) -> String {
        [110.0, 165.0, 220.0, 330.0, 440.0, 495.0, 660.0, 880.0]
            .into_iter()
            .map(|hz| format!("{hz}={:.4}", tone_amp(samples, hz)))
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn tune(mid: &[f32]) -> SpectralTuneResult {
        tune_stereo_with_cancel(
            mid,
            &[],
            SAMPLE_RATE,
            Some(ROOT),
            ResynthQuality::Standard,
            &|| false,
        )
        .expect("tune")
    }

    #[test]
    fn offline_tuner_moves_330_sine_onto_220_root() {
        let source = tone(330.0, 24_000, 0.7);
        let result = tune(&source);
        assert_eq!(result.tuned_mid.len(), source.len());
        let body = &result.tuned_mid[4_000..20_000];
        let at_root = tone_amp(body, ROOT);
        let at_source = tone_amp(body, 330.0);
        let track = result.pitch_track.lookup(0.5);
        assert!(
            at_root > at_source * 4.0 && at_root > 0.3,
            "330 Hz sine was not flattened onto 220 Hz root: {} f0={} conf={}",
            grid(body),
            track.f0_hz,
            track.confidence
        );
    }

    #[test]
    fn offline_tuner_flattens_melody_onto_root() {
        let mut source = tone(220.0, 12_000, 0.7);
        source.extend(tone(330.0, 12_000, 0.7));
        let result = tune(&source);
        let early = &result.tuned_mid[2_000..10_000];
        let late = &result.tuned_mid[14_000..22_000];
        let early_220 = tone_amp(early, 220.0);
        let early_330 = tone_amp(early, 330.0);
        let late_220 = tone_amp(late, 220.0);
        let late_330 = tone_amp(late, 330.0);
        let first = result.pitch_track.lookup(0.2);
        let last = result.pitch_track.lookup(0.8);
        assert!(
            early_220 > early_330 * 4.0,
            "early tuned lost the 220 Hz root: {} f0={} conf={}",
            grid(early),
            first.f0_hz,
            first.confidence
        );
        assert!(
            late_220 > late_330 * 4.0 && late_220 > 0.3,
            "late 330 Hz note was not flattened onto 220 Hz: {} f0={} conf={}",
            grid(late),
            last.f0_hz,
            last.confidence
        );
    }

    #[test]
    fn offline_tuner_locks_both_notes_of_a_dyad_to_root() {
        let source = (0..24_000)
            .map(|index| {
                let t = index as f64 / f64::from(SAMPLE_RATE);
                ((TAU * 220.0 * t).sin() + (TAU * 330.0 * t).sin()) as f32 * 0.4
            })
            .collect::<Vec<_>>();
        let result = tune(&source);
        let body = &result.tuned_mid[4_000..20_000];
        let at_root = tone_amp(body, ROOT);
        let leftover = tone_amp(body, 330.0);
        let track = result.pitch_track.lookup(0.5);
        assert!(
            (track.f0_hz - 220.0).abs() < 20.0 || (track.f0_hz - 330.0).abs() < 20.0,
            "dyad detector used a missing GCD f0={} conf={}",
            track.f0_hz,
            track.confidence
        );
        assert!(
            (track.f0_hz - 110.0).abs() > 30.0,
            "dyad detector collapsed to GCD f0={}",
            track.f0_hz
        );
        assert!(
            at_root > leftover * 4.0 && at_root > 0.3,
            "dyad 220+330 was not hard-locked onto 220 Hz: {} f0={} conf={}",
            grid(body),
            track.f0_hz,
            track.confidence
        );
    }

    #[test]
    fn offline_tuner_keeps_a_harmonic_series_on_the_root() {
        let source = (0..24_000)
            .map(|index| {
                let t = index as f64 / f64::from(SAMPLE_RATE);
                (0.5 * (TAU * 220.0 * t).sin()
                    + 0.25 * (TAU * 440.0 * t).sin()
                    + 0.125 * (TAU * 660.0 * t).sin()) as f32
            })
            .collect::<Vec<_>>();
        let result = tune(&source);
        let body = &result.tuned_mid[4_000..20_000];
        let fund = tone_amp(body, 220.0);
        let second = tone_amp(body, 440.0);
        let third = tone_amp(body, 660.0);
        assert!(
            fund > 0.2 && second > fund * 0.2 && third > fund * 0.08,
            "saw-like series collapsed: {}",
            grid(body)
        );
    }
}
