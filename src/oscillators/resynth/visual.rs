//! Immutable, bounded visual analysis for RESYNTH.
//!
//! This module is deliberately separate from [`super::artifact`]. Its
//! arrays are compiled by the import/build worker and are intended for the
//! editor only; no type here is part of the realtime artifact or audio-thread
//! ownership graph.  A caller should retain the returned `Arc` and publish or
//! hand it to the editor after analysis completes.

use std::sync::Arc;

use super::{
    ResynthAlgorithm, ResynthRtArtifact,
    analysis::PitchTrack,
    artifact::{
        ProductionResynthArtifact, RICH_FRAME_COUNT, RICH_FRAME_SAMPLES, RICH_ZONE_COUNT,
        RichVocoderArtifact, VOCODER_ENVELOPE_BINS,
    },
};
use crate::dsp::{Complex, fft};
#[cfg(test)]
use crate::wave_curve::bandlimit::TABLE_SIZE;

/// Number of fixed source-time waveform buckets.
pub const SOURCE_WAVE_BINS: usize = 256;
#[cfg(test)]
/// Number of source-time frames in the fixed spectrogram.
pub const SOURCE_STFT_FRAMES: usize = 64;
#[cfg(test)]
/// Number of logarithmically spaced frequency buckets in each STFT frame.
pub const SOURCE_STFT_BINS: usize = 96;
#[cfg(test)]
/// Length of the fixed Hann-windowed source STFT.
pub const SOURCE_STFT_SIZE: usize = 1_024;
#[cfg(test)]
/// Lowest displayed source-spectrogram frequency in Hz.
pub const SOURCE_STFT_MIN_HZ: f32 = 20.0;
#[cfg(test)]
/// Highest displayed source-spectrogram frequency in Hz before Nyquist clamp.
pub const SOURCE_STFT_MAX_HZ: f32 = 20_000.0;
/// Calibrated dB floor used for source STFT values.
pub const SOURCE_STFT_DB_FLOOR: f32 = -96.0;

#[cfg(test)]
/// Number of fixed bins used when viewing one compiled Algorithm cycle.
pub const ALGORITHM_VISUAL_WAVE_BINS: usize = 128;
/// Number of fixed log-frequency bins used when viewing one compiled cycle.
pub const ALGORITHM_VISUAL_SPECTRUM_BINS: usize = 96;
/// Maximum number of immutable sounding-artifact zones in one cache.
pub const ALGORITHM_VISUAL_ZONE_CAP: usize = RICH_ZONE_COUNT;
/// Fixed capacity for Grain transient/candidate markers.
pub const GRAIN_VISUAL_CANDIDATES: usize = 128;

/// A source-time waveform envelope bucket.
///
/// `min` and `max` preserve bipolar/transient material that a bucket mean
/// would hide.  `rms` is the calibrated energy of the same samples after the
/// visual finite/amplitude policy is applied.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct SourceWaveBin {
    pub min: f32,
    pub max: f32,
    pub rms: f32,
}

/// Errors returned by bounded source visual analysis.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceVisualError {
    Empty,
    NonFinite,
    InvalidSampleRate,
    TooManyFrames,
}

/// Immutable source waveform visual cache.
///
/// The payload size is independent of source duration.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceVisualCache {
    waveform_bins: Box<[SourceWaveBin; SOURCE_WAVE_BINS]>,
    #[cfg(test)]
    rms: Box<[f32; SOURCE_WAVE_BINS]>,
    #[cfg(test)]
    spectrum: Box<[[f32; SOURCE_STFT_BINS]; SOURCE_STFT_FRAMES]>,
    source_digest: [u8; 32],
    #[cfg(test)]
    sample_rate: u32,
    source_frames: u32,
}

impl SourceVisualCache {
    /// Analyze bounded source PCM into an immutable, fixed-size cache.
    ///
    /// The input is expected to be the same mono analysis projection used by
    /// RESYNTH import (`(L + R) / 2` for stereo).  Every source sample is
    /// assigned to exactly one waveform bucket.
    pub fn compile(
        source: &[f32],
        sample_rate: u32,
        source_digest: [u8; 32],
    ) -> Result<Self, SourceVisualError> {
        if source.is_empty() {
            return Err(SourceVisualError::Empty);
        }
        if source.len() > u32::MAX as usize {
            return Err(SourceVisualError::TooManyFrames);
        }
        if !(8_000..=384_000).contains(&sample_rate) {
            return Err(SourceVisualError::InvalidSampleRate);
        }
        if source.iter().any(|sample| !sample.is_finite()) {
            return Err(SourceVisualError::NonFinite);
        }

        let waveform_bins = std::array::from_fn(|index| waveform_bin(source, index));
        #[cfg(test)]
        let rms = std::array::from_fn(|index| waveform_bins[index].rms);
        #[cfg(test)]
        let mut spectrum = analyze_stft(source, sample_rate);
        #[cfg(test)]
        sanitize_stft_array(&mut spectrum);
        Ok(Self {
            waveform_bins: Box::new(waveform_bins),
            #[cfg(test)]
            rms: Box::new(rms),
            #[cfg(test)]
            spectrum: Box::new(spectrum),
            source_digest,
            #[cfg(test)]
            sample_rate,
            source_frames: source.len() as u32,
        })
    }

    /// Compatibility constructor used by worker import code that already has
    /// validated mono PCM.  The digest is derived from the PCM values and
    /// sample rate, never from mutable editor state.
    #[must_use]
    pub fn analyze(source: &[f32], sample_rate: u32) -> Arc<Self> {
        Self::analyze_with_digest(source, sample_rate, digest_pcm(source, sample_rate))
    }

    /// Worker-side constructor when the immutable Source Master byte digest is
    /// already available.  Container bytes, not controls or the selected
    /// Algorithm, determine the identity of this cache.
    #[must_use]
    pub fn analyze_with_digest(
        source: &[f32],
        sample_rate: u32,
        source_digest: [u8; 32],
    ) -> Arc<Self> {
        match Self::compile(source, sample_rate, source_digest) {
            Ok(cache) => Arc::new(cache),
            Err(_) => Arc::new(Self::empty(sample_rate)),
        }
    }

    fn empty(_sample_rate: u32) -> Self {
        Self {
            waveform_bins: Box::new([SourceWaveBin::default(); SOURCE_WAVE_BINS]),
            #[cfg(test)]
            rms: Box::new([0.0; SOURCE_WAVE_BINS]),
            #[cfg(test)]
            spectrum: Box::new([[SOURCE_STFT_DB_FLOOR; SOURCE_STFT_BINS]; SOURCE_STFT_FRAMES]),
            source_digest: [0; 32],
            #[cfg(test)]
            sample_rate: _sample_rate,
            source_frames: 0,
        }
    }

    #[must_use]
    pub fn source_digest(&self) -> [u8; 32] {
        self.source_digest
    }

    #[cfg(test)]
    #[must_use]
    pub const fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    #[must_use]
    pub const fn source_frames(&self) -> u32 {
        self.source_frames
    }

    #[must_use]
    pub fn waveform(&self) -> &[SourceWaveBin] {
        self.waveform_bins.as_ref()
    }

    #[cfg(test)]
    #[must_use]
    pub fn waveform_rms(&self) -> &[f32] {
        self.rms.as_ref()
    }

    #[cfg(test)]
    #[must_use]
    pub fn stft_db(&self) -> &[[f32; SOURCE_STFT_BINS]; SOURCE_STFT_FRAMES] {
        &self.spectrum
    }

    #[cfg(test)]
    #[must_use]
    pub fn stft_frame(&self, index: usize) -> Option<&[f32; SOURCE_STFT_BINS]> {
        self.spectrum.get(index)
    }

    #[cfg(test)]
    #[must_use]
    pub fn stft_db_at(&self, frame: usize, bin: usize) -> Option<f32> {
        self.spectrum.get(frame)?.get(bin).copied()
    }

    #[cfg(test)]
    #[must_use]
    pub fn spectrum_db(&self) -> &[[f32; SOURCE_STFT_BINS]; SOURCE_STFT_FRAMES] {
        self.stft_db()
    }

    #[cfg(test)]
    #[must_use]
    pub fn stft(&self) -> &[[f32; SOURCE_STFT_BINS]; SOURCE_STFT_FRAMES] {
        self.stft_db()
    }

    #[cfg(test)]
    /// Frequency represented by a log-frequency STFT bucket.
    #[must_use]
    pub fn stft_frequency_hz(&self, bin: usize) -> Option<f32> {
        if bin >= SOURCE_STFT_BINS {
            return None;
        }
        Some(stft_frequency_hz(self.sample_rate, bin))
    }

    #[cfg(test)]
    /// Source-time position represented by a fixed frame, in `[0, 1]`.
    #[must_use]
    pub const fn stft_time_normalized(frame: usize) -> Option<f32> {
        if frame >= SOURCE_STFT_FRAMES {
            return None;
        }
        if SOURCE_STFT_FRAMES <= 1 {
            Some(0.0)
        } else {
            Some(frame as f32 / (SOURCE_STFT_FRAMES - 1) as f32)
        }
    }
}

/// Static visual facts about one of the three user-visible Algorithms.
///
/// This is metadata, not a playback mode.  The enum intentionally has no
/// variants beyond Sample, Grain, and Rich.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AlgorithmVisualMetadata {
    pub algorithm: ResynthAlgorithm,
    pub label: &'static str,
    pub requires_root: bool,
    pub supports_unpitched_source: bool,
    pub waveform_is_cyclic: bool,
    pub visual_kind: AlgorithmVisualKind,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlgorithmVisualKind {
    SourceLoop,
    GranularCloud,
    HarmonicZones,
}

#[cfg(test)]
#[must_use]
pub const fn algorithm_visual_metadata(algorithm: ResynthAlgorithm) -> AlgorithmVisualMetadata {
    match algorithm {
        ResynthAlgorithm::Sample => AlgorithmVisualMetadata {
            algorithm,
            label: "Sample",
            requires_root: true,
            supports_unpitched_source: false,
            waveform_is_cyclic: true,
            visual_kind: AlgorithmVisualKind::SourceLoop,
        },
        ResynthAlgorithm::Grain => AlgorithmVisualMetadata {
            algorithm,
            label: "Grain",
            requires_root: false,
            supports_unpitched_source: true,
            waveform_is_cyclic: false,
            visual_kind: AlgorithmVisualKind::GranularCloud,
        },
        ResynthAlgorithm::Rich => AlgorithmVisualMetadata {
            algorithm,
            label: "Rich",
            requires_root: true,
            supports_unpitched_source: false,
            waveform_is_cyclic: true,
            visual_kind: AlgorithmVisualKind::HarmonicZones,
        },
    }
}

/// Immutable Source-region facts for a compiled Sample loop.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SampleLoopVisualMetadata {
    pub source_start_frame: u32,
    pub source_span_frames: u32,
    pub source_total_frames: u32,
    pub crossfade_frames: u32,
}

impl SampleLoopVisualMetadata {
    #[must_use]
    pub fn start_normalized(self) -> f32 {
        self.source_start_frame as f32 / self.source_total_frames.max(1) as f32
    }

    #[must_use]
    pub fn end_normalized(self) -> f32 {
        self.source_start_frame
            .saturating_add(self.source_span_frames) as f32
            / self.source_total_frames.max(1) as f32
    }

    #[must_use]
    pub fn crossfade_normalized(self) -> f32 {
        let artifact_loop_frames = self
            .source_span_frames
            .saturating_sub(self.crossfade_frames)
            .max(1);
        self.crossfade_frames as f32 / artifact_loop_frames as f32
    }
}

/// Fixed-size worker projection of the immutable, currently sounding RT
/// artifact. The cache owns copied display data only; it does not retain an RT
/// artifact, pointer node, `Arc`, scheduler, or mutable audio state.
#[derive(Clone, Debug, PartialEq)]
pub struct AlgorithmVisualCache {
    #[cfg(test)]
    metadata: AlgorithmVisualMetadata,
    #[cfg(test)]
    waveforms: Box<[[f32; ALGORITHM_VISUAL_WAVE_BINS]; ALGORITHM_VISUAL_ZONE_CAP]>,
    #[cfg(test)]
    waveform_mins: Box<[[f32; ALGORITHM_VISUAL_WAVE_BINS]; ALGORITHM_VISUAL_ZONE_CAP]>,
    #[cfg(test)]
    waveform_maxs: Box<[[f32; ALGORITHM_VISUAL_WAVE_BINS]; ALGORITHM_VISUAL_ZONE_CAP]>,
    #[cfg(test)]
    amplitudes: Box<[[f32; ALGORITHM_VISUAL_WAVE_BINS]; ALGORITHM_VISUAL_ZONE_CAP]>,
    spectra_db: Box<[[f32; ALGORITHM_VISUAL_SPECTRUM_BINS]; ALGORITHM_VISUAL_ZONE_CAP]>,
    rich_timeline_db: [[f32; ALGORITHM_VISUAL_SPECTRUM_BINS]; RICH_FRAME_COUNT],
    rich_waveform: Box<[SourceWaveBin; SOURCE_WAVE_BINS]>,
    #[cfg(test)]
    rms: [f32; ALGORITHM_VISUAL_ZONE_CAP],
    #[cfg(test)]
    peak: [f32; ALGORITHM_VISUAL_ZONE_CAP],
    zone_count: u8,
    default_zone: u8,
    sample_loop: Option<SampleLoopVisualMetadata>,
    grain_candidates: Box<[f32; GRAIN_VISUAL_CANDIDATES]>,
    grain_candidate_count: u8,
    pitch_curve: Box<[f32; SOURCE_WAVE_BINS]>,
}

impl AlgorithmVisualCache {
    /// Compatibility compiler for legacy analysis cycles. Production previews
    /// should use [`Self::compile_artifact`] so they agree with rendered data.
    #[cfg(test)]
    pub fn compile(algorithm: ResynthAlgorithm, cycle: &[f32; TABLE_SIZE]) -> Arc<Self> {
        let mut cache = Self::empty(algorithm);
        cache.write_test_zone(0, cycle);
        Arc::new(cache)
    }

    /// Compile a bounded display copy of the actual immutable playback
    /// artifact. This function is worker/editor-side and may perform FFT work.
    #[must_use]
    pub fn compile_artifact(artifact: &ResynthRtArtifact) -> Arc<Self> {
        Self::compile_artifact_with_cancel(artifact, &|| false)
            .expect("non-cancellable artifact visualization")
    }

    /// Compile a playback-accurate cache while allowing stale worker jobs to
    /// stop between bounded artifact zones.
    pub fn compile_artifact_with_cancel(
        artifact: &ResynthRtArtifact,
        should_cancel: &dyn Fn() -> bool,
    ) -> Option<Arc<Self>> {
        if should_cancel() {
            return None;
        }
        let mut cache = Self::empty(artifact.algorithm);
        match &artifact.data {
            ProductionResynthArtifact::Sample(sample) => {
                // Source-region fields are populated from the artifact's
                // immutable compiler receipt when available.
                if sample.source_total_frames() != 0 && sample.source_span_frames() != 0 {
                    cache.sample_loop = Some(SampleLoopVisualMetadata {
                        source_start_frame: u32::try_from(sample.source_start_frames())
                            .unwrap_or(u32::MAX),
                        source_span_frames: u32::try_from(sample.source_span_frames())
                            .unwrap_or(u32::MAX),
                        source_total_frames: u32::try_from(sample.source_total_frames())
                            .unwrap_or(u32::MAX),
                        crossfade_frames: u32::try_from(sample.crossfade_frames())
                            .unwrap_or(u32::MAX),
                    });
                }
            }
            ProductionResynthArtifact::Grain(grain) => {
                let count = grain.transients.len().min(GRAIN_VISUAL_CANDIDATES);
                let denominator = grain.samples.len().saturating_sub(1).max(1) as f32;
                for (target, transient) in cache
                    .grain_candidates
                    .iter_mut()
                    .zip(grain.transients.iter().take(count))
                {
                    *target = (*transient as f32 / denominator).clamp(0.0, 1.0);
                }
                cache.grain_candidate_count = count as u8;
                cache.pitch_curve = Box::new(pitch_curve_bins(&grain.pitch_track));
            }
            ProductionResynthArtifact::Rich(rich) => {
                if let Some(vocoder) = rich.vocoder() {
                    cache.pitch_curve = Box::new(pitch_curve_bins(&vocoder.pitch_track));
                    cache.zone_count = 1;
                    cache.default_zone = 0;
                    cache.rich_waveform = Box::new(std::array::from_fn(|index| {
                        vocoder_waveform_bin(vocoder, index)
                    }));
                    for (frame, spectrum) in cache.rich_timeline_db.iter_mut().enumerate() {
                        *spectrum = vocoder_spectrum_db(vocoder, frame);
                    }
                } else if let Some(sequence) = rich.sequence() {
                    cache.rich_waveform = Box::new(std::array::from_fn(|index| {
                        waveform_bin(&sequence.samples, index)
                    }));
                    cache.pitch_curve = Box::new(pitch_curve_bins(&sequence.pitch_track));
                    cache.zone_count = 1;
                    cache.default_zone = 0;
                } else {
                    let Some(slabs) = rich.slabs.as_deref() else {
                        return None;
                    };
                    cache.zone_count = RICH_ZONE_COUNT as u8;
                    cache.default_zone = artifact
                        .source_root_hz
                        .map_or(0, |root| rich.zone_for_frequency(root))
                        .min(RICH_ZONE_COUNT - 1) as u8;
                    #[cfg(test)]
                    for zone in 0..RICH_ZONE_COUNT {
                        if should_cancel() {
                            return None;
                        }
                        cache.spectra_db[zone] = artifact_spectrum(&slabs[zone]);
                    }
                    let slab = &slabs[usize::from(cache.default_zone)];
                    cache.rich_waveform =
                        Box::new(std::array::from_fn(|index| waveform_bin(slab, index)));
                    for (frame, spectrum) in cache.rich_timeline_db.iter_mut().enumerate() {
                        let start = frame * RICH_FRAME_SAMPLES;
                        *spectrum = artifact_spectrum(&slab[start..start + RICH_FRAME_SAMPLES]);
                    }
                }
            }
        }
        if should_cancel() {
            return None;
        }
        Some(Arc::new(cache))
    }

    fn empty(_algorithm: ResynthAlgorithm) -> Self {
        Self {
            #[cfg(test)]
            metadata: algorithm_visual_metadata(_algorithm),
            #[cfg(test)]
            waveforms: Box::new([[0.0; ALGORITHM_VISUAL_WAVE_BINS]; ALGORITHM_VISUAL_ZONE_CAP]),
            #[cfg(test)]
            waveform_mins: Box::new([[0.0; ALGORITHM_VISUAL_WAVE_BINS]; ALGORITHM_VISUAL_ZONE_CAP]),
            #[cfg(test)]
            waveform_maxs: Box::new([[0.0; ALGORITHM_VISUAL_WAVE_BINS]; ALGORITHM_VISUAL_ZONE_CAP]),
            #[cfg(test)]
            amplitudes: Box::new([[0.0; ALGORITHM_VISUAL_WAVE_BINS]; ALGORITHM_VISUAL_ZONE_CAP]),
            spectra_db: Box::new(
                [[SOURCE_STFT_DB_FLOOR; ALGORITHM_VISUAL_SPECTRUM_BINS]; ALGORITHM_VISUAL_ZONE_CAP],
            ),
            rich_timeline_db: [[SOURCE_STFT_DB_FLOOR; ALGORITHM_VISUAL_SPECTRUM_BINS];
                RICH_FRAME_COUNT],
            rich_waveform: Box::new([SourceWaveBin::default(); SOURCE_WAVE_BINS]),
            #[cfg(test)]
            rms: [0.0; ALGORITHM_VISUAL_ZONE_CAP],
            #[cfg(test)]
            peak: [0.0; ALGORITHM_VISUAL_ZONE_CAP],
            zone_count: 1,
            default_zone: 0,
            sample_loop: None,
            grain_candidates: Box::new([0.0; GRAIN_VISUAL_CANDIDATES]),
            grain_candidate_count: 0,
            pitch_curve: Box::new([0.0; SOURCE_WAVE_BINS]),
        }
    }

    #[cfg(test)]
    fn write_test_zone(&mut self, zone: usize, samples: &[f32]) {
        let zone = zone.min(ALGORITHM_VISUAL_ZONE_CAP - 1);
        let (waveform, waveform_min, waveform_max, amplitude, spectrum, rms, peak) =
            artifact_projection(samples);
        self.waveforms[zone] = waveform;
        self.waveform_mins[zone] = waveform_min;
        self.waveform_maxs[zone] = waveform_max;
        self.amplitudes[zone] = amplitude;
        self.spectra_db[zone] = spectrum;
        self.rms[zone] = rms;
        self.peak[zone] = peak;
    }

    #[cfg(test)]
    #[must_use]
    pub const fn metadata(&self) -> AlgorithmVisualMetadata {
        self.metadata
    }

    #[must_use]
    pub const fn zone_count(&self) -> usize {
        self.zone_count as usize
    }

    #[must_use]
    pub const fn default_zone(&self) -> usize {
        self.default_zone as usize
    }

    #[cfg(test)]
    #[must_use]
    pub fn waveform(&self) -> &[f32] {
        self.waveform_for_zone(self.default_zone())
    }

    #[cfg(test)]
    #[must_use]
    pub fn waveform_for_zone(&self, zone: usize) -> &[f32] {
        &self.waveforms[zone.min(self.zone_count().saturating_sub(1))]
    }

    #[cfg(test)]
    #[must_use]
    pub fn waveform_min_for_zone(&self, zone: usize) -> &[f32] {
        &self.waveform_mins[zone.min(self.zone_count().saturating_sub(1))]
    }

    #[cfg(test)]
    #[must_use]
    pub fn waveform_max_for_zone(&self, zone: usize) -> &[f32] {
        &self.waveform_maxs[zone.min(self.zone_count().saturating_sub(1))]
    }

    #[cfg(test)]
    #[must_use]
    pub fn amplitude_for_zone(&self, zone: usize) -> &[f32] {
        &self.amplitudes[zone.min(self.zone_count().saturating_sub(1))]
    }

    #[cfg(test)]
    #[must_use]
    pub fn spectrum_db(&self) -> &[f32] {
        self.spectrum_db_for_zone(self.default_zone())
    }

    #[must_use]
    pub fn spectrum_db_for_zone(&self, zone: usize) -> &[f32] {
        &self.spectra_db[zone.min(self.zone_count().saturating_sub(1))]
    }

    #[must_use]
    pub const fn rich_timeline_db(
        &self,
    ) -> &[[f32; ALGORITHM_VISUAL_SPECTRUM_BINS]; RICH_FRAME_COUNT] {
        &self.rich_timeline_db
    }

    #[must_use]
    pub fn rich_waveform(&self) -> &[SourceWaveBin] {
        self.rich_waveform.as_ref()
    }

    #[cfg(test)]
    #[must_use]
    pub fn rms(&self) -> f32 {
        self.rms[self.default_zone()]
    }

    #[cfg(test)]
    #[must_use]
    pub fn rms_for_zone(&self, zone: usize) -> f32 {
        self.rms[zone.min(self.zone_count().saturating_sub(1))]
    }

    #[cfg(test)]
    #[must_use]
    pub fn peak(&self) -> f32 {
        self.peak[self.default_zone()]
    }

    #[must_use]
    pub const fn sample_loop(&self) -> Option<SampleLoopVisualMetadata> {
        self.sample_loop
    }

    #[must_use]
    pub fn grain_candidates(&self) -> &[f32] {
        &self.grain_candidates[..usize::from(self.grain_candidate_count)]
    }

    #[must_use]
    pub fn pitch_curve(&self) -> &[f32] {
        self.pitch_curve.as_ref()
    }
}

/// Build the source-independent view cache from the exact artifact that will
/// be published. Keeping this API beside analysis makes worker call sites
/// explicit and prevents UI code from peeking through RT pointers.
#[must_use]
pub fn analyze_sounding_artifact_visuals(
    artifact: &ResynthRtArtifact,
) -> Arc<AlgorithmVisualCache> {
    AlgorithmVisualCache::compile_artifact(artifact)
}

/// Cancellable worker-side variant used by latest-wins state rebuilds.
pub fn analyze_sounding_artifact_visuals_with_cancel(
    artifact: &ResynthRtArtifact,
    should_cancel: &dyn Fn() -> bool,
) -> Option<Arc<AlgorithmVisualCache>> {
    AlgorithmVisualCache::compile_artifact_with_cancel(artifact, should_cancel)
}

#[cfg(test)]
fn artifact_projection(
    samples: &[f32],
) -> (
    [f32; ALGORITHM_VISUAL_WAVE_BINS],
    [f32; ALGORITHM_VISUAL_WAVE_BINS],
    [f32; ALGORITHM_VISUAL_WAVE_BINS],
    [f32; ALGORITHM_VISUAL_WAVE_BINS],
    [f32; ALGORITHM_VISUAL_SPECTRUM_BINS],
    f32,
    f32,
) {
    if samples.is_empty() {
        return (
            [0.0; ALGORITHM_VISUAL_WAVE_BINS],
            [0.0; ALGORITHM_VISUAL_WAVE_BINS],
            [0.0; ALGORITHM_VISUAL_WAVE_BINS],
            [0.0; ALGORITHM_VISUAL_WAVE_BINS],
            [SOURCE_STFT_DB_FLOOR; ALGORITHM_VISUAL_SPECTRUM_BINS],
            0.0,
            0.0,
        );
    }
    let mut waveform = [0.0_f32; ALGORITHM_VISUAL_WAVE_BINS];
    let mut waveform_min = [0.0_f32; ALGORITHM_VISUAL_WAVE_BINS];
    let mut waveform_max = [0.0_f32; ALGORITHM_VISUAL_WAVE_BINS];
    let mut amplitude = [0.0_f32; ALGORITHM_VISUAL_WAVE_BINS];
    for index in 0..ALGORITHM_VISUAL_WAVE_BINS {
        let start = index * samples.len() / ALGORITHM_VISUAL_WAVE_BINS;
        let end = ((index + 1) * samples.len() / ALGORITHM_VISUAL_WAVE_BINS)
            .max(start + 1)
            .min(samples.len());
        let mut minimum = f32::INFINITY;
        let mut maximum = f32::NEG_INFINITY;
        let mut sum = 0.0_f64;
        let mut square_sum = 0.0_f64;
        for sample in samples[start..end].iter().copied().map(finite_or_zero) {
            minimum = minimum.min(sample);
            maximum = maximum.max(sample);
            sum += f64::from(sample);
            square_sum += f64::from(sample) * f64::from(sample);
        }
        let count = (end - start) as f64;
        waveform[index] = finite_or_zero((sum / count) as f32).clamp(-1.0, 1.0);
        waveform_min[index] = finite_or_zero(minimum).clamp(-1.0, 1.0);
        waveform_max[index] = finite_or_zero(maximum).clamp(-1.0, 1.0);
        amplitude[index] = (square_sum / count).sqrt().min(1.0) as f32;
    }
    let square_sum = samples
        .iter()
        .copied()
        .map(finite_or_zero)
        .map(|sample| f64::from(sample) * f64::from(sample))
        .sum::<f64>();
    let rms = (square_sum / samples.len() as f64).sqrt().min(1.0) as f32;
    let peak = samples
        .iter()
        .copied()
        .map(finite_or_zero)
        .map(f32::abs)
        .fold(0.0_f32, f32::max)
        .min(1.0);
    (
        waveform,
        waveform_min,
        waveform_max,
        amplitude,
        artifact_spectrum(samples),
        finite_or_zero(rms),
        finite_or_zero(peak),
    )
}

fn artifact_spectrum(samples: &[f32]) -> [f32; ALGORITHM_VISUAL_SPECTRUM_BINS] {
    if samples.len() < 2 {
        return [SOURCE_STFT_DB_FLOOR; ALGORITHM_VISUAL_SPECTRUM_BINS];
    }
    // Never point-subsample a sounding artifact: that would fold Rich upper
    // harmonics into false low bins. Zero-padding retains every compiled
    // sample and gives the radix-2 worker FFT a bounded input (all production
    // artifacts are capped at 131_072 samples).
    let fft_len = samples.len().next_power_of_two();
    let mut values = vec![Complex::ZERO; fft_len];
    for (target, source) in values.iter_mut().zip(samples.iter().copied()) {
        target.re = f64::from(finite_or_zero(source));
    }
    fft(&mut values, false);
    let upper_bin = fft_len / 2 - 1;
    let mut output = [SOURCE_STFT_DB_FLOOR; ALGORITHM_VISUAL_SPECTRUM_BINS];
    for (display, value) in output.iter_mut().enumerate() {
        let denominator = (ALGORITHM_VISUAL_SPECTRUM_BINS - 1) as f32;
        let lower_fraction = (display as f32 - 0.5).max(0.0) / denominator;
        let upper_fraction = (display as f32 + 0.5).min(denominator) / denominator;
        let lower = (upper_bin as f32)
            .powf(lower_fraction)
            .floor()
            .clamp(1.0, upper_bin as f32) as usize;
        let upper = (upper_bin as f32)
            .powf(upper_fraction)
            .ceil()
            .clamp(lower as f32, upper_bin as f32) as usize;
        // A single point sample between FFT bins makes narrow harmonics
        // disappear. The maximum calibrated magnitude in each non-overlapping
        // log-frequency display cell preserves the actual compiled peaks.
        let magnitude = values[lower..=upper]
            .iter()
            .copied()
            .map(Complex::norm)
            .fold(0.0_f64, f64::max);
        let normalized = magnitude / (samples.len() as f64 * 0.5).max(1.0);
        *value = (20.0 * normalized.max(1.0e-12).log10()) as f32;
    }
    sanitize_spectrum(output)
}

fn vocoder_waveform_bin(vocoder: &RichVocoderArtifact, bin: usize) -> SourceWaveBin {
    let frames = vocoder.frames();
    if frames.is_empty() {
        return SourceWaveBin::default();
    }
    let start = bin * frames.len() / SOURCE_WAVE_BINS;
    let end = ((bin + 1) * frames.len() / SOURCE_WAVE_BINS).max(start + 1);
    let mut min = 0.0_f32;
    let mut max = 0.0_f32;
    let mut power = 0.0_f32;
    let mut count = 0.0_f32;
    for frame in &frames[start.min(frames.len() - 1)..end.min(frames.len())] {
        let amplitude = frame.gain.clamp(0.0, 1.0);
        min = min.min(-amplitude);
        max = max.max(amplitude);
        power += amplitude * amplitude;
        count += 1.0;
    }
    SourceWaveBin {
        min,
        max,
        rms: (power / count.max(1.0)).sqrt(),
    }
}

fn vocoder_spectrum_db(
    vocoder: &RichVocoderArtifact,
    display_frame: usize,
) -> [f32; ALGORITHM_VISUAL_SPECTRUM_BINS] {
    let frames = vocoder.frames();
    if frames.is_empty() {
        return [SOURCE_STFT_DB_FLOOR; ALGORITHM_VISUAL_SPECTRUM_BINS];
    }
    let index = if RICH_FRAME_COUNT <= 1 {
        0
    } else {
        display_frame * frames.len().saturating_sub(1) / (RICH_FRAME_COUNT - 1)
    }
    .min(frames.len() - 1);
    let envelope = &frames[index].envelope;
    let nyquist = vocoder.nyquist.max(1.0);
    let mut output = [SOURCE_STFT_DB_FLOOR; ALGORITHM_VISUAL_SPECTRUM_BINS];
    for (display, value) in output.iter_mut().enumerate() {
        let fraction = display as f32 / (ALGORITHM_VISUAL_SPECTRUM_BINS - 1) as f32;
        let hz = 20.0 * (nyquist / 20.0).powf(fraction);
        let pos = (hz / nyquist).clamp(0.0, 1.0) * (VOCODER_ENVELOPE_BINS - 1) as f32;
        let lo = pos.floor() as usize;
        let hi = (lo + 1).min(VOCODER_ENVELOPE_BINS - 1);
        let mix = pos - lo as f32;
        let log_mag = envelope[lo] + (envelope[hi] - envelope[lo]) * mix;
        *value = 20.0 * log_mag * std::f32::consts::LOG10_E;
    }
    sanitize_spectrum(output)
}

fn pitch_curve_bins(track: &PitchTrack) -> [f32; SOURCE_WAVE_BINS] {
    const MIN_HZ: f32 = 20.0;
    const MAX_HZ: f32 = 2_000.0;
    let span = MAX_HZ.ln() - MIN_HZ.ln();
    std::array::from_fn(|index| {
        let position = index as f32 / (SOURCE_WAVE_BINS.saturating_sub(1).max(1) as f32);
        let frame = track.lookup(position);
        if frame.f0_hz <= 0.0 || frame.confidence < 0.2 {
            0.0
        } else {
            ((frame.f0_hz.clamp(MIN_HZ, MAX_HZ).ln() - MIN_HZ.ln()) / span).clamp(0.0, 1.0)
        }
    })
}

fn waveform_bin(source: &[f32], bin: usize) -> SourceWaveBin {
    let start = bin * source.len() / SOURCE_WAVE_BINS;
    let mut end = (bin + 1) * source.len() / SOURCE_WAVE_BINS;
    // For a source shorter than the fixed display, each occupied bucket gets
    // exactly one source sample and the remaining buckets repeat the nearest
    // endpoint.  This keeps the cache total and makes the final source sample
    // visible rather than silently dropping it.
    if end <= start {
        let index = ((bin * source.len()) / SOURCE_WAVE_BINS).min(source.len() - 1);
        return scalar_wave_bin(source[index]);
    }
    end = end.min(source.len());
    let mut min = f32::INFINITY;
    let mut max = f32::NEG_INFINITY;
    let mut square_sum = 0.0_f64;
    for &sample in &source[start..end] {
        let sample = visual_sample(sample);
        min = min.min(sample);
        max = max.max(sample);
        square_sum += f64::from(sample) * f64::from(sample);
    }
    let rms = (square_sum / (end - start) as f64).sqrt() as f32;
    SourceWaveBin {
        min: finite_or_zero(min),
        max: finite_or_zero(max),
        rms: finite_or_zero(rms.min(1.0)),
    }
}

fn scalar_wave_bin(sample: f32) -> SourceWaveBin {
    let sample = visual_sample(sample);
    SourceWaveBin {
        min: sample,
        max: sample,
        rms: sample.abs(),
    }
}

fn visual_sample(sample: f32) -> f32 {
    if sample.is_finite() {
        sample.clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

#[cfg(test)]
fn analyze_stft(source: &[f32], sample_rate: u32) -> [[f32; SOURCE_STFT_BINS]; SOURCE_STFT_FRAMES] {
    let mut output = [[SOURCE_STFT_DB_FLOOR; SOURCE_STFT_BINS]; SOURCE_STFT_FRAMES];
    let mut frame = 0;
    while frame < SOURCE_STFT_FRAMES {
        let center = if SOURCE_STFT_FRAMES <= 1 || source.len() <= 1 {
            0
        } else {
            frame * (source.len() - 1) / (SOURCE_STFT_FRAMES - 1)
        };
        let mut spectrum = [Complex::ZERO; SOURCE_STFT_SIZE];
        for (index, sample) in spectrum.iter_mut().enumerate() {
            let relative = index as isize - (SOURCE_STFT_SIZE / 2) as isize;
            let source_index = center as isize + relative;
            let value = usize::try_from(source_index)
                .ok()
                .and_then(|index| source.get(index).copied())
                .map_or(0.0, visual_sample);
            let phase = std::f64::consts::TAU * index as f64 / SOURCE_STFT_SIZE as f64;
            let window = (0.5 - 0.5 * phase.cos()) as f32;
            *sample = Complex::new(f64::from(value * window), 0.0);
        }
        fft(&mut spectrum, false);
        for bin in 0..SOURCE_STFT_BINS {
            let frequency = stft_frequency_hz(sample_rate, bin);
            let position = frequency * SOURCE_STFT_SIZE as f32 / sample_rate as f32;
            output[frame][bin] = spectrum_magnitude_db(&spectrum, position);
        }
        frame += 1;
    }
    output
}

#[cfg(test)]
fn sanitize_stft_array(stft: &mut [[f32; SOURCE_STFT_BINS]; SOURCE_STFT_FRAMES]) {
    for frame in stft {
        for value in frame {
            if !value.is_finite() {
                *value = SOURCE_STFT_DB_FLOOR;
            } else {
                *value = value.clamp(SOURCE_STFT_DB_FLOOR, 0.0);
            }
        }
    }
}

#[cfg(test)]
fn stft_frequency_hz(sample_rate: u32, bin: usize) -> f32 {
    let nyquist = sample_rate as f32 * 0.5;
    let upper = SOURCE_STFT_MAX_HZ
        .min(nyquist * 0.95)
        .max(SOURCE_STFT_MIN_HZ);
    if SOURCE_STFT_BINS <= 1 {
        SOURCE_STFT_MIN_HZ
    } else {
        let fraction = bin.min(SOURCE_STFT_BINS - 1) as f32 / (SOURCE_STFT_BINS - 1) as f32;
        SOURCE_STFT_MIN_HZ * (upper / SOURCE_STFT_MIN_HZ).powf(fraction)
    }
}

#[cfg(test)]
fn spectrum_magnitude_db(spectrum: &[Complex; SOURCE_STFT_SIZE], position: f32) -> f32 {
    let position = position.clamp(0.0, (SOURCE_STFT_SIZE / 2) as f32);
    let first = position.floor() as usize;
    let second = (first + 1).min(SOURCE_STFT_SIZE / 2);
    let mix = f64::from(position - first as f32);
    let first_magnitude = spectrum[first].norm();
    let second_magnitude = spectrum[second].norm();
    // A Hann-windowed, unnormalised DFT has approximately N/2 gain for a
    // full-scale sinusoid.  This is only a display projection, but calibrated
    // dBFS makes amplitudes comparable across source files and frames.
    let magnitude = first_magnitude + (second_magnitude - first_magnitude) * mix;
    let normalized = magnitude / (SOURCE_STFT_SIZE as f64 * 0.5).max(1.0);
    (20.0 * normalized.max(1.0e-12).log10()) as f32
}

fn sanitize_spectrum(
    mut spectrum: [f32; ALGORITHM_VISUAL_SPECTRUM_BINS],
) -> [f32; ALGORITHM_VISUAL_SPECTRUM_BINS] {
    for value in &mut spectrum {
        if !value.is_finite() {
            *value = SOURCE_STFT_DB_FLOOR;
        } else {
            *value = value.clamp(SOURCE_STFT_DB_FLOOR, 0.0);
        }
    }
    spectrum
}

fn digest_pcm(source: &[f32], sample_rate: u32) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"kurv-resynth-source-visual-v1");
    hasher.update(&sample_rate.to_le_bytes());
    for sample in source {
        hasher.update(&sample.to_bits().to_le_bytes());
    }
    *hasher.finalize().as_bytes()
}

fn finite_or_zero(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest() -> [u8; 32] {
        [0x5a; 32]
    }

    #[test]
    fn source_cache_is_fixed_size_and_repeatable() {
        let source = (0..4_321)
            .map(|index| (std::f32::consts::TAU * 220.0 * index as f32 / 48_000.0).sin())
            .collect::<Vec<_>>();
        let a = SourceVisualCache::compile(&source, 48_000, digest()).expect("cache");
        let b = SourceVisualCache::compile(&source, 48_000, digest()).expect("cache");
        assert_eq!(a, b);
        assert_eq!(a.waveform().len(), SOURCE_WAVE_BINS);
        assert_eq!(a.stft_db().len(), SOURCE_STFT_FRAMES);
        assert_eq!(a.stft_db()[0].len(), SOURCE_STFT_BINS);
    }

    #[test]
    fn bipolar_waveform_keeps_extrema_and_energy() {
        let source = (0..4_800)
            .map(|index| (std::f32::consts::TAU * 220.0 * index as f32 / 48_000.0).sin())
            .collect::<Vec<_>>();
        let cache = SourceVisualCache::compile(&source, 48_000, digest()).expect("cache");
        let min = cache
            .waveform()
            .iter()
            .map(|bin| bin.min)
            .fold(1.0, f32::min);
        let max = cache
            .waveform()
            .iter()
            .map(|bin| bin.max)
            .fold(-1.0, f32::max);
        let rms = (cache
            .waveform()
            .iter()
            .map(|bin| bin.rms * bin.rms)
            .sum::<f32>()
            / SOURCE_WAVE_BINS as f32)
            .sqrt();
        assert!(min < -0.8 && max > 0.8, "min {min}, max {max}");
        assert!(rms > 0.4 && rms < 0.8, "rms {rms}");
    }

    #[test]
    fn waveform_buckets_cover_the_last_source_sample() {
        let mut source = vec![0.0_f32; SOURCE_WAVE_BINS + 17];
        *source.last_mut().expect("sample") = 0.9;
        let cache = SourceVisualCache::compile(&source, 48_000, digest()).expect("cache");
        let last = cache.waveform().last().copied().expect("last bin");
        assert!(last.max >= 0.89 && last.rms > 0.0);
    }

    #[test]
    fn short_and_silent_sources_are_safe() {
        for source in [vec![0.0], vec![0.2, -0.2, 0.1], vec![0.0; 2_047]] {
            let cache = SourceVisualCache::compile(&source, 44_100, digest()).expect("cache");
            assert!(
                cache
                    .waveform()
                    .iter()
                    .all(|bin| bin.min.is_finite() && bin.max.is_finite() && bin.rms.is_finite())
            );
            assert!(
                cache
                    .stft_db()
                    .iter()
                    .flatten()
                    .all(|value| value.is_finite() && (SOURCE_STFT_DB_FLOOR..=0.0).contains(value))
            );
        }
    }

    #[test]
    fn tone_has_a_repeatable_stft_ridge() {
        let source = (0..48_000)
            .map(|index| 0.5 * (std::f32::consts::TAU * 440.0 * index as f32 / 48_000.0).sin())
            .collect::<Vec<_>>();
        let cache = SourceVisualCache::compile(&source, 48_000, digest()).expect("cache");
        let frame = cache.stft_db()[SOURCE_STFT_FRAMES / 2];
        let (peak, _) = frame
            .iter()
            .copied()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.total_cmp(b))
            .expect("bins");
        let peak_hz = cache.stft_frequency_hz(peak).expect("frequency");
        assert!((250.0..=800.0).contains(&peak_hz), "{peak_hz} Hz");
        assert!(frame[peak] > -18.0, "{} dB", frame[peak]);
    }

    #[test]
    fn algorithm_visual_cache_is_bounded_and_finite() {
        let mut cycle = [0.0_f32; TABLE_SIZE];
        cycle[0] = f32::NAN;
        cycle[17] = 0.4;
        let cache = AlgorithmVisualCache::compile(ResynthAlgorithm::Rich, &cycle);
        assert_eq!(cache.waveform().len(), ALGORITHM_VISUAL_WAVE_BINS);
        assert!(cache.waveform().iter().all(|sample| sample.is_finite()));
        assert!(
            cache
                .spectrum_db()
                .iter()
                .all(|value| { value.is_finite() && (SOURCE_STFT_DB_FLOOR..=0.0).contains(value) })
        );
        assert!(cache.rms().is_finite() && cache.peak().is_finite());
    }

    #[test]
    fn artifact_waveform_buckets_preserve_phase_locked_extrema() {
        const LENGTH: usize = 32_768;
        const BIN: usize = 128;
        let source = (0..LENGTH)
            .map(|index| (std::f32::consts::TAU * BIN as f32 * index as f32 / LENGTH as f32).sin())
            .collect::<Vec<_>>();
        let (_, minimum, maximum, _, _, _, _) = artifact_projection(&source);
        let visible = minimum
            .iter()
            .zip(maximum.iter())
            .filter(|(low, high)| **low < -0.9 && **high > 0.9)
            .count();
        assert!(
            visible > ALGORITHM_VISUAL_WAVE_BINS / 2,
            "only {visible} buckets retained the sounding waveform extrema"
        );
    }

    #[test]
    fn artifact_spectrum_keeps_upper_harmonics_out_of_low_bins() {
        const LENGTH: usize = 32_768;
        const HARMONIC: usize = 4_000;
        let source = (0..LENGTH)
            .map(|index| {
                (std::f32::consts::TAU * HARMONIC as f32 * index as f32 / LENGTH as f32).sin()
            })
            .collect::<Vec<_>>();
        let spectrum = artifact_spectrum(&source);
        let expected = ((HARMONIC as f32).ln() / ((LENGTH / 2 - 1) as f32).ln()
            * (ALGORITHM_VISUAL_SPECTRUM_BINS - 1) as f32)
            .round() as usize;
        let peak = spectrum
            .iter()
            .enumerate()
            .max_by(|(_, first), (_, second)| first.total_cmp(second))
            .map_or(0, |(index, _)| index);
        assert!(
            peak.abs_diff(expected) <= 1,
            "peak {peak}, expected {expected}"
        );
        assert!(spectrum[peak] > -6.0, "{} dB", spectrum[peak]);
        assert!(
            spectrum[8] < -40.0,
            "false low-bin energy {} dB",
            spectrum[8]
        );
    }

    #[test]
    fn metadata_preserves_exact_three_algorithms() {
        assert_eq!(
            [
                ResynthAlgorithm::Sample,
                ResynthAlgorithm::Grain,
                ResynthAlgorithm::Rich
            ]
            .map(algorithm_visual_metadata)
            .iter()
            .map(|metadata| metadata.label)
            .collect::<Vec<_>>(),
            vec!["Sample", "Grain", "Rich"]
        );
    }
}
