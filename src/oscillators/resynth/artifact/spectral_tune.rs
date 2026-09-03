use std::f64::consts::TAU;

use super::super::analysis::{PitchTrack, PitchTrackFrame};
use super::super::quality::ResynthQuality;
use super::shared::ArtifactBuildError;
use crate::dsp::{Complex, fft};

const MIN_NOTE_HZ: f64 = 40.0;
const MAX_NOTE_HZ: f64 = 2_000.0;
const MAX_HARMONICS: f64 = 128.0;
const MAX_NOTES: usize = 4;
pub(super) const MAX_SPECTRAL_PEAKS: usize = 64;
const HARMONIC_CENTS: f64 = 55.0;
const DUPLICATE_CENTS: f64 = 40.0;

#[derive(Clone, Copy, Default)]
struct Note {
    hz: f64,
    confidence: f64,
}

#[derive(Clone, Copy)]
struct SpectralFrame {
    center: usize,
    notes: [Note; MAX_NOTES],
    note_count: u8,
    confidence: f64,
    onset: f32,
    partials: [GrainSpectralPartial; MAX_SPECTRAL_PEAKS],
    partial_count: u8,
}

impl Default for SpectralFrame {
    fn default() -> Self {
        Self {
            center: 0,
            notes: [Note::default(); MAX_NOTES],
            note_count: 0,
            confidence: 0.0,
            onset: 0.0,
            partials: [GrainSpectralPartial::default(); MAX_SPECTRAL_PEAKS],
            partial_count: 0,
        }
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct GrainSpectralPartial {
    pub(super) ratio: f32,
    pub(super) mid_amplitude: f32,
    pub(super) mid_phase: f32,
    pub(super) side_amplitude: f32,
    pub(super) side_phase: f32,
}

#[derive(Clone, Copy)]
pub(super) struct GrainSpectralFrame {
    pub(super) partials: [GrainSpectralPartial; MAX_SPECTRAL_PEAKS],
    pub(super) partial_count: u8,
}

impl Default for GrainSpectralFrame {
    fn default() -> Self {
        Self {
            partials: [GrainSpectralPartial::default(); MAX_SPECTRAL_PEAKS],
            partial_count: 0,
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct GrainSpectralBank {
    frames: Box<[GrainSpectralFrame]>,
}

impl GrainSpectralBank {
    pub(super) fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub(super) fn lookup(&self, position: f32) -> GrainSpectralFrame {
        if self.frames.is_empty() {
            return GrainSpectralFrame::default();
        }
        let index = (position.clamp(0.0, 1.0) * self.frames.len().saturating_sub(1) as f32).round()
            as usize;
        self.frames[index]
    }

    pub(super) fn retained_bytes(&self) -> usize {
        self.frames.len() * std::mem::size_of::<GrainSpectralFrame>()
    }
}

#[derive(Clone, Copy, Default)]
struct Peak {
    hz: f64,
    magnitude: f64,
}

pub(super) struct SpectralTuneResult {
    pub(super) tuned_mid: Vec<f32>,
    pub(super) tuned_side: Vec<f32>,
    pub(super) pitch_track: PitchTrack,
    pub(super) transients: Vec<u32>,
    pub(super) residual_mid: Vec<f32>,
    pub(super) residual_side: Vec<f32>,
    pub(super) grain_spectrum: GrainSpectralBank,
}

pub(super) fn tune_stereo_with_cancel(
    mid: &[f32],
    side: &[f32],
    sample_rate: f32,
    _root_hz: Option<f32>,
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
        quality.pitch_fft_size(),
        hop_size,
        should_cancel,
    )?;
    let pitch_track = pitch_track_from_frames(&frames);
    let grain_spectrum = GrainSpectralBank {
        frames: frames
            .iter()
            .map(|frame| GrainSpectralFrame {
                partials: frame.partials,
                partial_count: frame.partial_count,
            })
            .collect(),
    };
    let residual_mid =
        residual_channel(mid, sample_rate, fft_size, hop_size, &frames, should_cancel)?;
    let residual_side = if side.is_empty() {
        Vec::new()
    } else {
        residual_channel(
            side,
            sample_rate,
            fft_size,
            hop_size,
            &frames,
            should_cancel,
        )?
    };
    Ok(SpectralTuneResult {
        tuned_mid: Vec::new(),
        tuned_side: Vec::new(),
        pitch_track,
        transients,
        residual_mid,
        residual_side,
        grain_spectrum,
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
    let fft_size = quality.pitch_fft_size();
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
    let coherent = (0..fft_size)
        .map(|index| window_value(Window::Hann, index, fft_size))
        .sum::<f64>()
        .max(1.0);

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
        let peaks = pick_peaks(&magnitudes, sample_rate, fft_size);
        let notes = detect_notes(&peaks, previous_anchor);
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
        let mut partials = [GrainSpectralPartial::default(); MAX_SPECTRAL_PEAKS];
        let mut partial_count = 0_u8;
        for peak in peaks {
            let Some(note) = assign_note(peak.hz, &notes, usize::from(note_count)) else {
                continue;
            };
            let bin = (peak.hz / (f64::from(sample_rate) / fft_size as f64)).round() as usize;
            let Some(mid) = mid_spectrum.get(bin).copied() else {
                continue;
            };
            let side = side_spectrum.get(bin).copied().unwrap_or(Complex::ZERO);
            let slot = usize::from(partial_count);
            if slot == MAX_SPECTRAL_PEAKS {
                break;
            }
            let lock = confidence.clamp(0.0, 1.0);
            partials[slot] = GrainSpectralPartial {
                ratio: (peak.hz / note.hz) as f32,
                mid_amplitude: (2.0 * mid.norm() * lock / coherent) as f32,
                mid_phase: mid.arg() as f32,
                side_amplitude: (2.0 * side.norm() * lock / coherent) as f32,
                side_phase: side.arg() as f32,
            };
            partial_count += 1;
        }
        frames.push(SpectralFrame {
            center,
            notes,
            note_count,
            confidence,
            onset: 0.0,
            partials,
            partial_count,
        });
    }

    for (frame, flux) in frames.iter_mut().zip(flux) {
        frame.onset = flux as f32;
    }
    Ok(frames)
}

fn detect_notes(peaks: &[Peak], previous: f64) -> [Note; MAX_NOTES] {
    let mut notes = [Note::default(); MAX_NOTES];
    if peaks.is_empty() {
        return notes;
    }
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
        for harmonic in 1..=16 {
            push_seed(peak.hz / f64::from(harmonic));
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
            confidence: note_confidence(peaks, refined),
        };
        note_count += 1;
    }
    notes
}

fn note_confidence(peaks: &[Peak], f0: f64) -> f64 {
    let total = peaks.iter().map(|peak| peak.magnitude).sum::<f64>();
    if total <= 1.0e-12 {
        return 0.0;
    }
    (peaks
        .iter()
        .copied()
        .map(|peak| {
            let harmonic = (peak.hz / f0).round();
            if !(1.0..=MAX_HARMONICS).contains(&harmonic) {
                return 0.0;
            }
            let agreement =
                (1.0 - cents(peak.hz, f0 * harmonic).abs() / HARMONIC_CENTS).clamp(0.0, 1.0);
            peak.magnitude * agreement * agreement
        })
        .sum::<f64>()
        / total)
        .clamp(0.0, 1.0)
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
    if has_fundamental || harmonics >= 3.0 {
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

fn pick_peaks(magnitudes: &[f64], sample_rate: f32, fft_size: usize) -> Vec<Peak> {
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
        let insertion = peaks
            .iter()
            .position(|other: &Peak| magnitude > other.magnitude)
            .unwrap_or(peaks.len());
        if insertion >= MAX_SPECTRAL_PEAKS {
            continue;
        }
        peaks.insert(insertion, Peak { hz, magnitude });
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
    for frame in frames {
        let retain = 1.0 - frame.onset;
        for partial in frame
            .partials
            .iter_mut()
            .take(usize::from(frame.partial_count))
        {
            partial.mid_amplitude *= retain;
            partial.side_amplitude *= retain;
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

fn attenuate_peak(spectrum: &mut [Complex], hz: f64, bin_hz: f64, fft_size: usize, retain: f64) {
    let half = fft_size / 2;
    let center = (hz / bin_hz).round() as isize;
    for delta in -2..=2 {
        let bin = center + delta;
        if bin > 0 && (bin as usize) < half {
            spectrum[bin as usize] = spectrum[bin as usize] * retain;
        }
    }
}

fn cents(left: f64, right: f64) -> f64 {
    if left <= 0.0 || right <= 0.0 {
        return 10_000.0;
    }
    1_200.0 * (left / right).log2()
}

fn residual_channel(
    source: &[f32],
    sample_rate: f32,
    fft_size: usize,
    _hop_size: usize,
    frames: &[SpectralFrame],
    should_cancel: &dyn Fn() -> bool,
) -> Result<Vec<f32>, ArtifactBuildError> {
    if source.is_empty() {
        return Ok(Vec::new());
    }
    let mut output = vec![0.0_f64; source.len()];
    let mut weight = vec![0.0_f64; source.len()];
    let mut spectrum = vec![Complex::ZERO; fft_size];
    let mut magnitudes = vec![0.0_f64; fft_size / 2 + 1];
    let bin_hz = f64::from(sample_rate) / fft_size as f64;
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
        for bin in 1..=fft_size / 2 {
            magnitudes[bin] = spectrum[bin].norm();
        }
        let lock = (frame.confidence * (1.0 - f64::from(frame.onset))).clamp(0.0, 1.0);
        if lock > 0.05 && frame.note_count > 0 {
            for peak in pick_peaks(&magnitudes, sample_rate, fft_size) {
                if assign_note(peak.hz, &frame.notes, usize::from(frame.note_count)).is_some() {
                    attenuate_peak(&mut spectrum, peak.hz, bin_hz, fft_size, 1.0 - lock);
                }
            }
        }
        spectrum[0].im = 0.0;
        spectrum[fft_size / 2].im = 0.0;
        for bin in 1..fft_size / 2 {
            spectrum[fft_size - bin] = spectrum[bin].conj();
        }
        fft(&mut spectrum, true);
        for (index, bin) in spectrum.iter().enumerate() {
            let output_index = start + index as isize;
            if !(0..output.len() as isize).contains(&output_index) {
                continue;
            }
            let hann = window_value(Window::Hann, index, fft_size);
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
