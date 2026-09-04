//! Source-preserving RESYNTH analysis and bounded realtime artifacts.
//!
//! Import, pitch estimation and artifact compilation are editor/worker-thread
//! operations. Grain playback is a preallocated scheduler. Rich playback is a
//! worker-baked PAD spectra: playback only interpolates periodic tables, with
//! no FFT or allocation on the audio thread.

use std::sync::Arc;

pub(super) mod analysis;
pub(super) mod artifact;
pub(super) mod decode;
pub(super) mod quality;
#[cfg(test)]
pub(super) mod scheduler;
#[cfg(test)]
pub(super) mod spectral;
pub(super) mod targeting;
pub(crate) use analysis::{PitchTrack, PitchTrackFrame};
pub use quality::ResynthQuality;
pub use targeting::{PitchMode, ScaleId, TargetSet};
pub(super) mod visual;

use crate::dsp::{Complex, fft};
#[cfg(test)]
use crate::dsp::{shortest_angle, splitmix64};
use crate::wave_curve::bandlimit::CompileError;
#[cfg(test)]
use crate::wave_curve::bandlimit::TABLE_SIZE;

#[cfg(test)]
use artifact::GRAIN_LAYERS;
#[cfg(test)]
use artifact::GrainSchedulerState;
use artifact::{
    ArtifactBuildError, GrainSourceArtifact, ProductionResynthArtifact, RichZoneArtifact,
    SampleLoopArtifact, SourceAuditionArtifact, bandlimit_source_by_stride_with_cancel,
    remove_dc_and_peak_normalize,
};

pub const MAX_RESYNTH_SOURCE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_RESYNTH_SOURCE_NAME_BYTES: usize = 1_024;
pub const MAX_RESYNTH_DECODED_FRAMES: usize = 8 * 1024 * 1024;
pub const RESYNTH_ALGORITHM_COUNT: usize = 3;

use visual::SourceVisualCache;

pub type ResynthVisualModel = SourceVisualCache;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum ResynthAlgorithm {
    #[default]
    Sample = 0,
    Grain = 1,
    Rich = 2,
}

impl ResynthAlgorithm {
    pub const ALL: [Self; RESYNTH_ALGORITHM_COUNT] = [Self::Sample, Self::Grain, Self::Rich];
    /// Algorithms exposed by the current editor. `Sample` remains a decoder-only
    /// compatibility value for older saved states.
    pub const VISIBLE: [Self; 2] = [Self::Grain, Self::Rich];
    pub const fn label(self) -> &'static str {
        match self {
            Self::Sample => "Legacy",
            Self::Grain => "Grain",
            Self::Rich => "Rich",
        }
    }
    pub const fn index(self) -> usize {
        self as usize
    }
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Sample),
            1 => Some(Self::Grain),
            2 => Some(Self::Rich),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum GrainDirection {
    #[default]
    Hold = 0,
    Forward = 1,
    Backward = 2,
    PingPong = 3,
}

impl GrainDirection {
    pub const ALL: [Self; 4] = [Self::Hold, Self::Forward, Self::Backward, Self::PingPong];

    #[must_use]
    pub const fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Forward,
            2 => Self::Backward,
            3 => Self::PingPong,
            _ => Self::Hold,
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Hold => "HOLD",
            Self::Forward => "FWD",
            Self::Backward => "BACK",
            Self::PingPong => "PONG",
        }
    }
}

macro_rules! resynth_controls {
    ($($(#[$attribute:meta])* $field:ident = ($default:expr, $minimum:expr, $maximum:expr);)+) => {
        #[derive(Clone, Copy, Debug, PartialEq)]
        pub struct ResynthControls {
            $($(#[$attribute])* pub $field: f32,)+
            pub grain_direction: u8,
            pub pitch_mode: PitchMode,
            pub seed: u64,
        }

        impl Default for ResynthControls {
            fn default() -> Self {
                Self {
                    $($field: $default,)+
                    grain_direction: GrainDirection::Forward as u8,
                    pitch_mode: PitchMode::Classic,
                    seed: 0x4b55_5256_5245_5359,
                }
            }
        }

        impl ResynthControls {
            #[must_use]
            pub fn sanitized(self) -> Self {
                Self {
                    $($field: finite_clamp(self.$field, $minimum, $maximum, $default),)+
                    grain_direction: GrainDirection::from_u8(self.grain_direction) as u8,
                    pitch_mode: PitchMode::from_wire(
                        self.pitch_mode.to_wire().0,
                        self.pitch_mode.to_wire().1,
                    )
                    .unwrap_or(PitchMode::Classic),
                    seed: self.seed,
                }
            }
        }
    };
}

resynth_controls! {
    position = (0.5, 0.0, 1.0);
    loop_start = (0.0, 0.0, 1.0);
    loop_end = (1.0, 0.0, 1.0);
    grain_size = (0.65, 0.0, 1.0);
    /// Grain onset rate in Hz. Simultaneous load is derived from Rate x Size.
    grain_density = (24.0, 1.0, 2_000.0);
    /// Source-timeline speed. Grain pitch remains independently controlled.
    grain_speed = (1.0, 0.125, 4.0);
    grain_spray = (0.0, 0.0, 1.0);
    /// Blend from the source pitch contour to the played note.
    grain_tune = (0.0, 0.0, 1.0);
    /// Retained Source Master stereo width. One preserves the original field.
    grain_stereo = (1.0, 0.0, 1.0);
    grain_envelope = (0.0, 0.0, 1.0);
    grain_timing = (0.0, 0.0, 1.0);
    grain_pitch = (0.0, -24.0, 24.0);
    grain_pitch_spread = (0.0, 0.0, 24.0);
    grain_level = (1.0, 0.0, 1.0);
    grain_level_spread = (0.0, 0.0, 1.0);
    grain_pan = (0.0, -1.0, 1.0);
    grain_pan_spread = (0.0, 0.0, 1.0);
    grain_reverse = (0.0, 0.0, 1.0);
    grain_blur = (0.0, 0.0, 1.0);
    /// Blend into worker-derived local peak normalization.
    grain_normalize = (0.0, 0.0, 1.0);
    grain_attack = (0.5, 0.0, 1.0);
    grain_hold = (0.0, 0.0, 1.0);
    grain_release = (0.5, 0.0, 1.0);
    rich_balance = (0.0, -1.0, 1.0);
    rich_formant_semitones = (0.0, -24.0, 24.0);
    rich_air_db = (0.0, -12.0, 12.0);
    rich_diffuse = (0.0, 0.0, 1.0);
    /// Blend from static maxima to the measured source gain envelope.
    rich_dynamic = (0.0, 0.0, 1.0);
    /// Select live spectral-frame reconstruction instead of a worker-baked loop.
    rich_rt = (0.0, 0.0, 1.0);
}

impl ResynthControls {
    #[must_use]
    pub fn grain_direction(self) -> GrainDirection {
        GrainDirection::from_u8(self.grain_direction)
    }

    #[must_use]
    pub fn loop_bounds(self) -> (f32, f32) {
        let start = self.loop_start.min(self.loop_end).clamp(0.0, 1.0);
        let end = self.loop_start.max(self.loop_end).clamp(0.0, 1.0);
        if end - start < 1.0e-4 {
            ((end - 1.0e-4).max(0.0), (start + 1.0e-4).min(1.0))
        } else {
            (start, end)
        }
    }
}

fn finite_clamp(value: f32, min: f32, max: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResynthSourceMaster {
    pub file_name: String,
    /// Byte-exact original container data. This is authoritative on recall.
    pub original_bytes: Vec<u8>,
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: u32,
    pub estimated_root_hz: Option<f32>,
    pub pitch_confidence: f32,
}

#[derive(Clone, Debug)]
pub struct ResynthAnalysisModel {
    pub source: ResynthSourceMaster,
    /// Exact user correction; automatic detection remains separately truthful.
    pub root_override_hz: Option<f32>,
    /// Legacy preview projection; production compiles the selected RT Artifact
    /// directly from the Source Master.
    #[cfg(test)]
    pub cycles: Box<[Option<[f32; TABLE_SIZE]>; RESYNTH_ALGORITHM_COUNT]>,
    pub visuals: Arc<ResynthVisualModel>,
}

impl ResynthAnalysisModel {
    #[must_use]
    pub fn effective_root_hz(&self) -> Option<f32> {
        self.root_override_hz.or(self.source.estimated_root_hz)
    }

    #[cfg(test)]
    #[must_use]
    pub fn supports_algorithm(&self, algorithm: ResynthAlgorithm) -> bool {
        self.cycles[algorithm.index()].is_some()
    }

    #[cfg(test)]
    /// Clone only the immutable pointer to worker-produced source visuals.
    /// The cache is never part of [`ResynthRtArtifact`] or an audio callback.
    #[must_use]
    pub fn source_visual_cache(&self) -> Arc<SourceVisualCache> {
        Arc::clone(&self.visuals)
    }
}

#[derive(Clone)]
pub struct ResynthRtArtifact {
    pub algorithm: ResynthAlgorithm,
    pub source_root_hz: Option<f32>,
    pub data: ProductionResynthArtifact,
    /// Ephemeral raw mono audition cache rebuilt from the embedded Source Master.
    pub source_audition: Box<SourceAuditionArtifact>,
    pub source_audition_gain: f32,
}

impl Default for ResynthRtArtifact {
    fn default() -> Self {
        Self {
            algorithm: ResynthAlgorithm::Grain,
            source_root_hz: None,
            data: ProductionResynthArtifact::Grain(Box::new(GrainSourceArtifact::silence())),
            source_audition: Box::new(SourceAuditionArtifact::silence()),
            source_audition_gain: 1.0,
        }
    }
}

impl ResynthRtArtifact {
    pub(crate) fn preview_cycle_sample(&self, position: f32, phase: f32) -> f32 {
        fn source_cycle(source: &GrainSourceArtifact, position: f32, phase: f32) -> f32 {
            let root = source.root_hz.unwrap_or(110.0).max(1.0);
            let start = position.clamp(0.0, 1.0) * source.samples.len().saturating_sub(1) as f32;
            let at = start + phase * source.source_sample_rate / root;
            let index = at.floor() as usize % source.samples.len().max(1);
            let next = (index + 1).min(source.samples.len().saturating_sub(1));
            let mix = at.fract();
            source.samples[index].mul_add(1.0 - mix, source.samples[next] * mix)
        }

        match &self.data {
            ProductionResynthArtifact::Sample(source) => {
                let period = source.source_sample_rate / source.root_hz.max(1.0);
                source.eval_bandlimited(phase * period / source.samples.len().max(1) as f32, 1.0)
            }
            ProductionResynthArtifact::Grain(source) => source_cycle(source, position, phase),
            ProductionResynthArtifact::Rich(source) => source
                .sequence()
                .map_or(0.0, |sequence| source_cycle(sequence, position, phase)),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImportError {
    Empty,
    Oversize { bytes: usize, limit: usize },
    SourceNameTooLong { bytes: usize, limit: usize },
    UnsupportedWav,
    UnsupportedChannels(u16),
    UnsupportedSampleRate(u32),
    TooManyFrames,
    Silent,
    NoStablePitch,
    Compile(CompileError),
    Artifact(ArtifactBuildError),
    PublicationBusy,
    Cancelled,
}

impl From<CompileError> for ImportError {
    fn from(value: CompileError) -> Self {
        Self::Compile(value)
    }
}
impl From<ArtifactBuildError> for ImportError {
    fn from(value: ArtifactBuildError) -> Self {
        match value {
            ArtifactBuildError::Cancelled => Self::Cancelled,
            other => Self::Artifact(other),
        }
    }
}

/// Decodes one bounded WAV, retains its exact bytes, estimates pitch and
/// builds the bounded source analysis shared by the RESYNTH algorithms.
#[cfg(test)]
pub fn analyze_wav(
    file_name: impl Into<String>,
    bytes: Vec<u8>,
    controls: ResynthControls,
) -> Result<ResynthAnalysisModel, ImportError> {
    analyze_wav_with_visual_cache_and_cancel(file_name, bytes, controls, None, None, &|| false)
}

/// Analyze a WAV cooperatively on a worker thread. Cancellation is sampled
/// during decoding and every major bounded analysis loop; no UI type enters DSP.
pub fn analyze_wav_with_cancel(
    file_name: impl Into<String>,
    bytes: Vec<u8>,
    controls: ResynthControls,
    should_cancel: impl Fn() -> bool,
) -> Result<ResynthAnalysisModel, ImportError> {
    analyze_wav_with_visual_cache_and_cancel(file_name, bytes, controls, None, None, &should_cancel)
}

fn analyze_wav_with_visual_cache_and_cancel(
    file_name: impl Into<String>,
    bytes: Vec<u8>,
    controls: ResynthControls,
    root_override_hz: Option<f32>,
    visual_cache: Option<Arc<SourceVisualCache>>,
    should_cancel: &dyn Fn() -> bool,
) -> Result<ResynthAnalysisModel, ImportError> {
    #[cfg(not(test))]
    let _ = controls;
    if should_cancel() {
        return Err(ImportError::Cancelled);
    }
    if bytes.is_empty() {
        return Err(ImportError::Empty);
    }
    if bytes.len() > MAX_RESYNTH_SOURCE_BYTES {
        return Err(ImportError::Oversize {
            bytes: bytes.len(),
            limit: MAX_RESYNTH_SOURCE_BYTES,
        });
    }
    let file_name = file_name.into();
    if file_name.len() > MAX_RESYNTH_SOURCE_NAME_BYTES {
        return Err(ImportError::SourceNameTooLong {
            bytes: file_name.len(),
            limit: MAX_RESYNTH_SOURCE_NAME_BYTES,
        });
    }
    let (mut decoded, spec, frames) =
        decode::decode_source_with_cancel(bytes.as_slice(), should_cancel)?;
    let mono = decoded.analysis_mut();
    if mono.is_empty() {
        return Err(ImportError::Empty);
    }
    remove_dc_and_peak_normalize(mono);
    let source_rms = rms(mono);
    if !source_rms.is_finite() || source_rms < 1.0e-6 {
        return Err(ImportError::Silent);
    }
    let (estimated_root_hz, confidence) =
        estimate_root_with_cancel(mono, spec.sample_rate, should_cancel)?;
    // Zero is the explicit, persisted "unknown pitch" state. Grain can work
    // without a fundamental; Sample and Rich require later root correction.
    let detected_root_hz =
        (estimated_root_hz > 0.0 && confidence >= 0.2).then_some(estimated_root_hz);
    #[cfg(test)]
    let cycles = {
        let effective_root_hz = root_override_hz.or(detected_root_hz);
        let controls = controls.sanitized();
        let grain =
            compile_grain_cycle_with_cancel(mono, spec.sample_rate, controls, should_cancel)?;
        let (sample, rich) = if let Some(root_hz) = effective_root_hz {
            let sample = extract_cycle_with_cancel(
                mono,
                spec.sample_rate,
                root_hz,
                controls.position,
                should_cancel,
            )?;
            let rich = compile_rich_cycle_with_cancel(
                &sample,
                spec.sample_rate,
                root_hz,
                controls,
                should_cancel,
            )?;
            (Some(sample), Some(rich))
        } else {
            (None, None)
        };
        Box::new([sample, Some(grain), rich])
    };
    if should_cancel() {
        return Err(ImportError::Cancelled);
    }
    let source_digest = *blake3::hash(&bytes).as_bytes();
    let visuals =
        if let Some(cache) = visual_cache.filter(|cache| cache.source_digest() == source_digest) {
            cache
        } else {
            let visuals =
                ResynthVisualModel::analyze_with_digest(&mono, spec.sample_rate, source_digest);
            if should_cancel() {
                return Err(ImportError::Cancelled);
            }
            visuals
        };
    let source = ResynthSourceMaster {
        file_name,
        original_bytes: bytes,
        sample_rate: spec.sample_rate,
        channels: spec.channels,
        frames: u32::try_from(frames).unwrap_or(u32::MAX),
        estimated_root_hz: detected_root_hz,
        pitch_confidence: confidence,
    };
    Ok(ResynthAnalysisModel {
        source,
        root_override_hz,
        #[cfg(test)]
        cycles,
        visuals,
    })
}

pub fn analyze_wav_with_root_override(
    file_name: impl Into<String>,
    bytes: Vec<u8>,
    controls: ResynthControls,
    root_override_hz: Option<f32>,
) -> Result<ResynthAnalysisModel, ImportError> {
    analyze_wav_with_root_override_and_visuals_with_cancel(
        file_name,
        bytes,
        controls,
        root_override_hz,
        None,
        || false,
    )
}

/// Root-aware worker rebuild that retains a cache from the unchanged Source
/// Master instead of recomputing waveform/STFT visuals.
#[cfg(test)]
pub fn analyze_wav_with_root_override_and_visuals(
    file_name: impl Into<String>,
    bytes: Vec<u8>,
    controls: ResynthControls,
    root_override_hz: Option<f32>,
    visual_cache: Option<Arc<SourceVisualCache>>,
) -> Result<ResynthAnalysisModel, ImportError> {
    analyze_wav_with_root_override_and_visuals_with_cancel(
        file_name,
        bytes,
        controls,
        root_override_hz,
        visual_cache,
        || false,
    )
}

/// Root-aware rebuild with cooperative cancellation through decode, pitch,
/// cycle extraction, and Rich-cycle analysis.
pub fn analyze_wav_with_root_override_and_visuals_with_cancel(
    file_name: impl Into<String>,
    bytes: Vec<u8>,
    controls: ResynthControls,
    root_override_hz: Option<f32>,
    visual_cache: Option<Arc<SourceVisualCache>>,
    should_cancel: impl Fn() -> bool,
) -> Result<ResynthAnalysisModel, ImportError> {
    if root_override_hz.is_some_and(|root| !root.is_finite() || !(20.0..=2_000.0).contains(&root)) {
        return Err(ImportError::NoStablePitch);
    }
    analyze_wav_with_visual_cache_and_cancel(
        file_name,
        bytes,
        controls,
        root_override_hz,
        visual_cache,
        &should_cancel,
    )
}

#[cfg(test)]
pub fn compile_rt_artifact(
    model: &ResynthAnalysisModel,
    algorithm: ResynthAlgorithm,
    controls: ResynthControls,
) -> Result<ResynthRtArtifact, ImportError> {
    compile_rt_artifact_with_cancel(model, algorithm, controls, || false)
}

pub fn compile_rt_artifact_with_cancel(
    model: &ResynthAnalysisModel,
    algorithm: ResynthAlgorithm,
    controls: ResynthControls,
    should_cancel: impl Fn() -> bool,
) -> Result<ResynthRtArtifact, ImportError> {
    compile_rt_artifact_with_cancel_ref(model, algorithm, controls, &should_cancel)
}

fn compile_rt_artifact_with_cancel_ref(
    model: &ResynthAnalysisModel,
    algorithm: ResynthAlgorithm,
    controls: ResynthControls,
    should_cancel: &dyn Fn() -> bool,
) -> Result<ResynthRtArtifact, ImportError> {
    if should_cancel() {
        return Err(ImportError::Cancelled);
    }
    if matches!(algorithm, ResynthAlgorithm::Sample | ResynthAlgorithm::Rich)
        && model.effective_root_hz().is_none()
    {
        return Err(ImportError::NoStablePitch);
    }
    let (decoded, source_sample_rate) =
        decode_artifact_source_with_cancel(&model.source.original_bytes, should_cancel)?;
    let mono = decoded.analysis();
    let source_audition = Box::new(SourceAuditionArtifact::compile_with_cancel(
        &mono,
        source_sample_rate,
        should_cancel,
    )?);
    if should_cancel() {
        return Err(ImportError::Cancelled);
    }
    let root_hz = model.effective_root_hz();
    let data = match algorithm {
        ResynthAlgorithm::Sample => {
            ProductionResynthArtifact::Sample(Box::new(SampleLoopArtifact::compile_with_cancel(
                &mono,
                source_sample_rate,
                root_hz,
                controls.position,
                should_cancel,
            )?))
        }
        ResynthAlgorithm::Grain => ProductionResynthArtifact::Grain(Box::new(
            GrainSourceArtifact::compile_channels_with_cancel(
                decoded.mid(),
                decoded.side(),
                source_sample_rate,
                root_hz,
                controls,
                crate::oscillators::ResynthQuality::current(),
                should_cancel,
            )?,
        )),
        ResynthAlgorithm::Rich => {
            let root = root_hz.ok_or(ImportError::NoStablePitch)?;
            let rich = RichZoneArtifact::compile_with_cancel(
                &mono,
                source_sample_rate,
                root,
                controls,
                should_cancel,
            )?;
            ProductionResynthArtifact::Rich(Box::new(rich))
        }
    };
    if should_cancel() {
        return Err(ImportError::Cancelled);
    }
    Ok(ResynthRtArtifact {
        algorithm,
        source_root_hz: root_hz,
        data,
        source_audition,
        source_audition_gain: 1.0,
    })
}

pub(crate) fn compile_source_audition(
    bytes: &[u8],
) -> Result<Box<SourceAuditionArtifact>, ImportError> {
    let (decoded, sample_rate) = decode_artifact_source(bytes)?;
    Ok(Box::new(SourceAuditionArtifact::compile(
        decoded.analysis(),
        sample_rate,
    )?))
}

fn decode_artifact_source(bytes: &[u8]) -> Result<(decode::DecodedSourcePcm, u32), ImportError> {
    decode_artifact_source_with_cancel(bytes, &|| false)
}

fn decode_artifact_source_with_cancel(
    bytes: &[u8],
    should_cancel: &dyn Fn() -> bool,
) -> Result<(decode::DecodedSourcePcm, u32), ImportError> {
    let (decoded, spec, _) = decode::decode_source_with_cancel(bytes, should_cancel)?;
    Ok((decoded, spec.sample_rate))
}

fn rms(samples: &[f32]) -> f32 {
    (samples
        .iter()
        .map(|sample| f64::from(*sample) * f64::from(*sample))
        .sum::<f64>()
        / samples.len().max(1) as f64)
        .sqrt() as f32
}

fn remove_dc(samples: &mut [f32]) {
    let mean =
        samples.iter().map(|sample| f64::from(*sample)).sum::<f64>() / samples.len().max(1) as f64;
    for sample in samples {
        *sample = (f64::from(*sample) - mean) as f32;
    }
}

#[cfg(test)]
fn estimate_root(samples: &[f32], sample_rate: u32) -> (f32, f32) {
    estimate_root_with_cancel(samples, sample_rate, &|| false).unwrap_or((0.0, 0.0))
}

fn estimate_root_with_cancel(
    samples: &[f32],
    sample_rate: u32,
    should_cancel: &dyn Fn() -> bool,
) -> Result<(f32, f32), ImportError> {
    if should_cancel() {
        return Err(ImportError::Cancelled);
    }
    if samples.len() < 64 {
        return Ok((0.0, 0.0));
    }
    let window_frames = (sample_rate as usize / 2).clamp(64, samples.len());
    let last_start = samples.len().saturating_sub(window_frames);
    let starts = std::array::from_fn::<_, 5, _>(|index| index * last_start / 4);
    let mut candidates = Vec::with_capacity(starts.len());
    let mut previous_start = None;
    for start in starts {
        if previous_start == Some(start) {
            continue;
        }
        if should_cancel() {
            return Err(ImportError::Cancelled);
        }
        previous_start = Some(start);
        let (root, confidence) = estimate_root_window_with_cancel(
            &samples[start..start + window_frames],
            sample_rate,
            should_cancel,
        )?;
        if root > 0.0 && confidence >= 0.2 {
            candidates.push((root, confidence));
        }
    }
    let Some(&(single_root, single_confidence)) = candidates.first() else {
        return Ok((0.0, 0.0));
    };
    if candidates.len() == 1 {
        return Ok((single_root, single_confidence));
    }
    let total_confidence = candidates
        .iter()
        .map(|(_, confidence)| *confidence)
        .sum::<f32>()
        .max(f32::MIN_POSITIVE);
    let (representative, _, matching_count) = candidates
        .iter()
        .copied()
        .map(|candidate| {
            let matches = candidates
                .iter()
                .copied()
                .filter(|other| pitch_distance_cents(candidate.0, other.0) <= 70.0)
                .collect::<Vec<_>>();
            let score = matches
                .iter()
                .map(|(_, confidence)| *confidence)
                .sum::<f32>();
            (candidate.0, score, matches.len())
        })
        .max_by(|first, second| first.1.total_cmp(&second.1))
        .unwrap_or((single_root, single_confidence, 1));
    if candidates.len() >= 3 && matching_count * 5 < candidates.len() * 3 {
        return Ok((
            0.0,
            (matching_count as f32 / candidates.len() as f32).clamp(0.0, 1.0),
        ));
    }
    let mut log_sum = 0.0_f32;
    let mut matching_confidence = 0.0_f32;
    let mut confidence_sum = 0.0_f32;
    for (root, confidence) in candidates.iter().copied() {
        if pitch_distance_cents(representative, root) <= 70.0 {
            log_sum += root.ln() * confidence;
            matching_confidence += confidence;
            confidence_sum += confidence * confidence;
        }
    }
    let root = (log_sum / matching_confidence.max(f32::MIN_POSITIVE)).exp();
    let average_confidence = confidence_sum / matching_confidence.max(f32::MIN_POSITIVE);
    let consensus = matching_confidence / total_confidence;
    Ok((
        root.clamp(20.0, 2_000.0),
        (average_confidence * consensus).clamp(0.0, 1.0),
    ))
}

fn pitch_distance_cents(first: f32, second: f32) -> f32 {
    (1_200.0 * (first / second).log2()).abs()
}

fn estimate_root_window_with_cancel(
    samples: &[f32],
    sample_rate: u32,
    should_cancel: &dyn Fn() -> bool,
) -> Result<(f32, f32), ImportError> {
    // Root detection is deliberately spectral: preserve enough bandwidth for
    // a harmonic-series vote, then search a log-frequency candidate grid.
    let stride = sample_rate.div_ceil(16_000).max(1) as usize;
    let input_rms = rms(samples);
    let projection =
        bandlimit_source_by_stride_with_cancel(samples, sample_rate as f32, stride, should_cancel)?;
    debug_assert_eq!(projection.stride, stride);
    let mut x = projection.samples;
    if x.len() < 64 {
        return Ok((0.0, 0.0));
    }
    remove_dc(&mut x);
    let retained_rms = rms(&x);
    if retained_rms < 1.0e-6 || retained_rms < input_rms * 0.05 {
        return Ok((0.0, 0.0));
    }
    let fft_len = x
        .len()
        .next_power_of_two()
        .saturating_mul(2)
        .clamp(128, 32_768);
    let mut spectrum = vec![Complex::ZERO; fft_len];
    let denominator = x.len().saturating_sub(1).max(1) as f32;
    for (index, sample) in x.iter().copied().take(fft_len).enumerate() {
        if index & 4_095 == 0 && should_cancel() {
            return Err(ImportError::Cancelled);
        }
        let window = 0.5 - 0.5 * (std::f32::consts::TAU * index as f32 / denominator).cos();
        spectrum[index].re = f64::from(sample * window);
    }
    fft(&mut spectrum, false);
    if should_cancel() {
        return Err(ImportError::Cancelled);
    }
    let magnitudes = spectrum[..=fft_len / 2]
        .iter()
        .map(|bin| bin.re.hypot(bin.im) as f32)
        .collect::<Vec<_>>();
    let bin_hz = projection.sample_rate / fft_len as f32;
    let min_bin = (20.0 / bin_hz).ceil().max(1.0) as usize;
    let max_bin = ((2_000.0 / bin_hz).floor() as usize).min(magnitudes.len() - 2);
    if max_bin <= min_bin {
        return Ok((0.0, 0.0));
    }
    let spectral_mean = magnitudes[1..].iter().copied().sum::<f32>()
        / magnitudes.len().saturating_sub(1).max(1) as f32;
    let root_peak = magnitudes[min_bin..=max_bin]
        .iter()
        .copied()
        .fold(0.0_f32, f32::max);
    if root_peak <= spectral_mean * 2.5 {
        return Ok((0.0, 0.0));
    }
    let root_band = &magnitudes[min_bin..=max_bin];
    let flatness_floor = root_peak * 1.0e-12;
    let arithmetic_mean = root_band.iter().copied().sum::<f32>() / root_band.len() as f32;
    let geometric_mean = (root_band
        .iter()
        .map(|magnitude| magnitude.max(flatness_floor).ln())
        .sum::<f32>()
        / root_band.len() as f32)
        .exp();
    let spectral_flatness = geometric_mean / arithmetic_mean.max(f32::MIN_POSITIVE);
    if spectral_flatness > 0.55 {
        return Ok((0.0, 0.0));
    }
    let spectrum_peak = magnitudes.iter().copied().fold(root_peak, f32::max);
    let magnitude_at = |frequency: f32| {
        let bin = (frequency / bin_hz).round() as usize;
        let start = bin.saturating_sub(1).max(1);
        let end = (bin + 1).min(magnitudes.len() - 1);
        magnitudes[start..=end]
            .iter()
            .copied()
            .fold(0.0_f32, f32::max)
    };
    let mut candidates = Vec::with_capacity(320);
    let mut frequency = 20.0_f32;
    let step = 2.0_f32.powf(1.0 / 48.0);
    while frequency <= 2_000.0 {
        if candidates.len() & 63 == 0 && should_cancel() {
            return Err(ImportError::Cancelled);
        }
        let direct = magnitude_at(frequency) / root_peak.max(f32::MIN_POSITIVE);
        let mut harmonic_sum = 0.0_f32;
        let mut weight_sum = 0.0_f32;
        for harmonic in 1..=12 {
            let harmonic_hz = frequency * harmonic as f32;
            if harmonic_hz >= projection.sample_rate * 0.5 {
                break;
            }
            let weight = (harmonic as f32).sqrt().recip();
            harmonic_sum +=
                magnitude_at(harmonic_hz) / spectrum_peak.max(f32::MIN_POSITIVE) * weight;
            weight_sum += weight;
        }
        let harmonic_salience = harmonic_sum / weight_sum.max(f32::MIN_POSITIVE);
        candidates.push((
            frequency,
            direct.mul_add(0.7, harmonic_salience * 0.3),
            direct,
        ));
        frequency *= step;
    }
    let Some(&(candidate_hz, best_score, direct)) = candidates
        .iter()
        .max_by(|left, right| left.1.total_cmp(&right.1))
    else {
        return Ok((0.0, 0.0));
    };
    let runner = candidates
        .iter()
        .filter(|candidate| pitch_distance_cents(candidate_hz, candidate.0) > 80.0)
        .map(|candidate| candidate.1)
        .fold(0.0_f32, f32::max);
    let contrast = ((best_score - runner) / best_score.max(f32::MIN_POSITIVE)).clamp(0.0, 1.0);
    let prominence = ((root_peak - spectral_mean * 2.5) / root_peak).clamp(0.0, 1.0);
    let center = (candidate_hz / bin_hz)
        .round()
        .clamp(min_bin as f32, max_bin as f32) as usize;
    let peak_bin = (center.saturating_sub(2)..=(center + 2).min(max_bin))
        .max_by(|left, right| magnitudes[*left].total_cmp(&magnitudes[*right]))
        .unwrap_or(center);
    let neighborhood_start = peak_bin.saturating_sub(12).max(min_bin);
    let neighborhood_end = (peak_bin + 12).min(max_bin);
    let (background_sum, background_bins) = (neighborhood_start..=neighborhood_end)
        .filter(|bin| bin.abs_diff(peak_bin) > 3)
        .fold((0.0_f32, 0_usize), |(sum, count), bin| {
            (sum + magnitudes[bin], count + 1)
        });
    let local_background = background_sum / background_bins.max(1) as f32;
    let peak_support = ((magnitudes[peak_bin] - local_background)
        / magnitudes[peak_bin].max(f32::MIN_POSITIVE))
    .clamp(0.0, 1.0);
    let confidence =
        prominence * direct.mul_add(0.85, contrast * 0.15) * (peak_support / 0.75).clamp(0.0, 1.0);
    if direct < 0.08 || peak_support < 0.35 || confidence < 0.2 {
        return Ok((0.0, confidence));
    }
    let left = magnitudes[peak_bin.saturating_sub(1)]
        .max(f32::MIN_POSITIVE)
        .ln();
    let middle = magnitudes[peak_bin].max(f32::MIN_POSITIVE).ln();
    let right = magnitudes[(peak_bin + 1).min(magnitudes.len() - 1)]
        .max(f32::MIN_POSITIVE)
        .ln();
    let curvature = left - 2.0 * middle + right;
    let offset = if curvature.abs() > 1.0e-6 {
        0.5 * (left - right) / curvature
    } else {
        0.0
    };
    Ok((
        ((peak_bin as f32 + offset.clamp(-0.5, 0.5)) * bin_hz).clamp(20.0, 2_000.0),
        confidence.clamp(0.0, 1.0),
    ))
}

#[cfg(test)]
fn extract_cycle_with_cancel(
    samples: &[f32],
    sample_rate: u32,
    root_hz: f32,
    position: f32,
    should_cancel: &dyn Fn() -> bool,
) -> Result<[f32; TABLE_SIZE], ImportError> {
    // 384 kHz / 20 Hz is the admitted longest source period.
    let period = (sample_rate as f32 / root_hz.clamp(20.0, 2_000.0)).clamp(4.0, 19_200.0);
    let periods = 12_usize;
    let span = (period.ceil() as usize * periods).min(samples.len());
    let center = ((samples.len().saturating_sub(span)) as f32 * position.clamp(0.0, 1.0)) as usize;
    let mut cycle = [0.0_f32; TABLE_SIZE];
    for (index, output) in cycle.iter_mut().enumerate() {
        if index & 127 == 0 && should_cancel() {
            return Err(ImportError::Cancelled);
        }
        let phase = index as f32 / TABLE_SIZE as f32;
        let mut sum = 0.0;
        let mut weight_sum = 0.0;
        for p in 0..periods {
            let weight =
                0.5 - 0.5 * (std::f32::consts::TAU * (p as f32 + 0.5) / periods as f32).cos();
            sum += sample_linear(samples, center as f32 + (p as f32 + phase) * period) * weight;
            weight_sum += weight;
        }
        *output = sum / weight_sum.max(1.0e-6);
    }
    normalize_cycle(&mut cycle);
    Ok(cycle)
}

#[cfg(test)]
fn compile_grain_cycle_with_cancel(
    samples: &[f32],
    sample_rate: u32,
    controls: ResynthControls,
    should_cancel: &dyn Fn() -> bool,
) -> Result<[f32; TABLE_SIZE], ImportError> {
    let grain_seconds = 0.005 * 200.0_f32.powf(controls.grain_size.clamp(0.0, 1.0));
    let grain_frames = sample_rate as f32 * grain_seconds;
    let grain_count = (controls.grain_density * grain_seconds)
        .ceil()
        .clamp(1.0, GRAIN_LAYERS as f32) as usize;
    let mut output = [0.0_f32; TABLE_SIZE];
    let mut weights = [0.0_f32; TABLE_SIZE];
    let mut random = controls.seed;
    for grain in 0..grain_count {
        if should_cancel() {
            return Err(ImportError::Cancelled);
        }
        random = splitmix64(random.wrapping_add(grain as u64));
        let base = controls.position * samples.len().saturating_sub(1) as f32;
        let spray = (random as u32) as f32 / u32::MAX as f32 * 2.0 - 1.0;
        let source = (base + spray * controls.grain_spray * samples.len() as f32 * 0.45)
            .clamp(0.0, samples.len().saturating_sub(1) as f32);
        let length = ((TABLE_SIZE as f32 * (0.5 + controls.grain_size * 1.5)).round() as usize)
            .clamp(64, TABLE_SIZE * 2);
        let destination = grain * TABLE_SIZE / grain_count;
        for index in 0..length {
            let phase = index as f32 / length as f32;
            let window = 0.5 - 0.5 * (std::f32::consts::TAU * phase).cos();
            let out = (destination + index) & (TABLE_SIZE - 1);
            output[out] += sample_linear(
                samples,
                source + (index as f32 - length as f32 * 0.5) * grain_frames / length as f32,
            ) * window;
            weights[out] += window;
        }
    }
    for (sample, weight) in output.iter_mut().zip(weights) {
        *sample /= weight.max(1.0e-6);
    }
    normalize_cycle(&mut output);
    Ok(output)
}

#[cfg(test)]
fn compile_rich_cycle_with_cancel(
    sample: &[f32; TABLE_SIZE],
    sample_rate: u32,
    root_hz: f32,
    controls: ResynthControls,
    should_cancel: &dyn Fn() -> bool,
) -> Result<[f32; TABLE_SIZE], ImportError> {
    let max_measured =
        ((sample_rate as f32 * 0.5 / root_hz.max(20.0)) as usize).clamp(8, TABLE_SIZE / 2 - 1);
    let formant_ratio = 2.0_f32.powf(controls.rich_formant_semitones / 12.0);
    let air_gain = 10.0_f32.powf(controls.rich_air_db / 20.0);
    let mut measured_magnitude = vec![0.0_f64; max_measured + 1];
    let mut measured_phase = vec![0.0_f64; max_measured + 1];
    for harmonic in 1..=max_measured {
        if harmonic & 15 == 0 && should_cancel() {
            return Err(ImportError::Cancelled);
        }
        let mut source_re = 0.0_f64;
        let mut source_im = 0.0_f64;
        for (n, value) in sample.iter().copied().enumerate() {
            let angle = -std::f64::consts::TAU * harmonic as f64 * n as f64 / TABLE_SIZE as f64;
            source_re += f64::from(value) * angle.cos();
            source_im += f64::from(value) * angle.sin();
        }
        measured_magnitude[harmonic] = (source_re * source_re + source_im * source_im).sqrt();
        measured_phase[harmonic] = source_im.atan2(source_re);
    }

    let mut re = vec![0.0_f64; TABLE_SIZE / 2];
    let mut im = vec![0.0_f64; TABLE_SIZE / 2];
    for harmonic in 1..TABLE_SIZE / 2 {
        if harmonic & 31 == 0 && should_cancel() {
            return Err(ImportError::Cancelled);
        }
        let mapped = harmonic as f32 / formant_ratio;
        let source_harmonic = (mapped.round() as usize).clamp(1, max_measured);
        let source_magnitude = measured_magnitude[source_harmonic];
        let lo = source_harmonic.saturating_sub(2).max(1);
        let hi = (source_harmonic + 2).min(max_measured);
        let local_power = measured_magnitude[lo..=hi]
            .iter()
            .map(|magnitude| magnitude * magnitude)
            .sum::<f64>()
            / (hi - lo + 1) as f64;
        // Tonal and residual are a power-preserving decomposition of measured
        // source energy. BALANCE then uses two equal-power arcs:
        // tonal -> source -> residual. It never creates residual energy.
        let tonal_magnitude = local_power.sqrt().min(source_magnitude);
        let residual_magnitude = (source_magnitude * source_magnitude
            - tonal_magnitude * tonal_magnitude)
            .max(0.0)
            .sqrt();
        let balance = controls.rich_balance;
        let (tonal_gain, source_gain, residual_gain) = if balance <= 0.0 {
            let angle = f64::from(balance + 1.0) * std::f64::consts::FRAC_PI_2;
            (angle.cos(), angle.sin(), 0.0)
        } else {
            let angle = f64::from(balance) * std::f64::consts::FRAC_PI_2;
            (0.0, angle.cos(), angle.sin())
        };
        let source_phase = measured_phase[source_harmonic];
        let diffuse_phase = source_phase
            + shortest_angle(source_phase, hash_phase(controls.seed, harmonic as u64))
                * f64::from(controls.rich_diffuse);
        let tonal_and_source = tonal_gain * tonal_magnitude + source_gain * source_magnitude;
        let mut component_re = tonal_and_source * source_phase.cos()
            + residual_gain * residual_magnitude * diffuse_phase.cos();
        let mut component_im = tonal_and_source * source_phase.sin()
            + residual_gain * residual_magnitude * diffuse_phase.sin();

        // Re-populating harmonics above the directly measured grid is only
        // permitted when AIR explicitly requests it. Otherwise upper bins are
        // muted instead of extrapolating invented energy.
        if harmonic > max_measured {
            if controls.rich_air_db <= 0.0 {
                component_re = 0.0;
                component_im = 0.0;
            } else {
                let rolloff = (harmonic as f64 / max_measured as f64).powf(-1.35);
                component_re *= rolloff;
                component_im *= rolloff;
            }
        }
        if harmonic as f32 * root_hz >= 8_000.0 {
            component_re *= f64::from(air_gain);
            component_im *= f64::from(air_gain);
        }
        re[harmonic] = component_re;
        im[harmonic] = component_im;
    }
    let mut output = [0.0_f32; TABLE_SIZE];
    for (n, value) in output.iter_mut().enumerate() {
        if n & 31 == 0 && should_cancel() {
            return Err(ImportError::Cancelled);
        }
        let mut sum = 0.0_f64;
        for harmonic in 1..TABLE_SIZE / 2 {
            let angle = std::f64::consts::TAU * harmonic as f64 * n as f64 / TABLE_SIZE as f64;
            sum += re[harmonic] * angle.cos() - im[harmonic] * angle.sin();
        }
        *value = (2.0 * sum / TABLE_SIZE as f64) as f32;
    }
    normalize_cycle(&mut output);
    Ok(output)
}

#[cfg(test)]
fn hash_phase(seed: u64, index: u64) -> f64 {
    splitmix64(seed ^ index.wrapping_mul(0x9e37_79b9_7f4a_7c15)) as f64 / u64::MAX as f64
        * std::f64::consts::TAU
}
#[cfg(test)]
fn sample_linear(samples: &[f32], position: f32) -> f32 {
    let position = position.clamp(0.0, samples.len().saturating_sub(1) as f32);
    let first = position.floor() as usize;
    let next = (first + 1).min(samples.len() - 1);
    let mix = position - position.floor();
    (samples[next] - samples[first]).mul_add(mix, samples[first])
}
#[cfg(test)]
fn normalize_cycle(samples: &mut [f32; TABLE_SIZE]) {
    remove_dc(samples);
    let peak = samples
        .iter()
        .copied()
        .map(f32::abs)
        .fold(0.0_f32, f32::max);
    if peak > 1.0e-8 {
        let gain = peak.recip();
        for sample in samples {
            *sample *= gain;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wav_test::{wav_f32, wav_i16, wav_sine};

    fn wav_stereo_antiphase(frequency: f32, seconds: f32) -> Vec<u8> {
        let frames = (48_000.0 * seconds) as usize;
        wav_i16(
            2,
            48_000,
            (0..frames).flat_map(|index| {
                let sample = ((std::f32::consts::TAU * frequency * index as f32 / 48_000.0).sin()
                    * 20_000.0) as i16;
                [sample, -sample]
            }),
        )
    }

    fn wav_tone_at_edges(frequency: f32) -> Vec<u8> {
        wav_i16(
            1,
            48_000,
            (0..72_000).map(|index| {
                let sounding = index < 19_200 || index >= 52_800;
                if sounding {
                    ((std::f32::consts::TAU * frequency * index as f32 / 48_000.0).sin() * 20_000.0)
                        as i16
                } else {
                    0
                }
            }),
        )
    }

    fn wav_hostile_float() -> Vec<u8> {
        wav_f32(1, 48_000, std::iter::repeat_n(f32::MAX, 4_096))
    }

    fn wav_impulse() -> Vec<u8> {
        wav_i16(
            1,
            48_000,
            (0..4_800).map(|index| if index == 100 { 24_000_i16 } else { 0 }),
        )
    }

    #[test]
    fn cancellation_is_observed_during_worker_analysis() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let polls = AtomicUsize::new(0);
        let result = analyze_wav_with_cancel(
            "cancel.wav",
            wav_sine(220.0, 1.5),
            ResynthControls::default(),
            || polls.fetch_add(1, Ordering::Relaxed) >= 3,
        );
        assert!(matches!(result, Err(ImportError::Cancelled)));
        assert!(polls.load(Ordering::Relaxed) >= 4);
    }

    #[test]
    fn hostile_float_wav_is_rejected_before_analysis() {
        assert!(matches!(
            analyze_wav(
                "hostile.wav",
                wav_hostile_float(),
                ResynthControls::default(),
            ),
            Err(ImportError::UnsupportedWav)
        ));
    }

    #[test]
    fn high_only_sources_do_not_alias_into_a_confident_root() {
        for frequency in [5_000.0_f32, 8_000.0, 14_000.0, 20_000.0] {
            let source = (0..48_000)
                .map(|index| (std::f32::consts::TAU * frequency * index as f32 / 48_000.0).sin())
                .collect::<Vec<_>>();
            let (root, confidence) = estimate_root(&source, 48_000);
            assert!(
                root == 0.0 || confidence < 0.2,
                "{frequency} Hz aliased into root {root} at confidence {confidence}"
            );
        }
    }

    #[test]
    fn pitch_detection_uses_multiple_source_time_windows() {
        let model = analyze_wav(
            "edge-tone.wav",
            wav_tone_at_edges(220.0),
            ResynthControls::default(),
        )
        .expect("analysis");
        let root = model.source.estimated_root_hz.expect("edge root");
        assert!((root - 220.0).abs() < 3.0, "{root}");
        assert!(model.source.pitch_confidence >= 0.2);
    }

    #[test]
    fn source_master_keeps_exact_container_bytes_and_detects_pitch() {
        let bytes = wav_sine(220.0, 0.5);
        let model =
            analyze_wav("root.wav", bytes.clone(), ResynthControls::default()).expect("analyze");
        assert_eq!(model.source.original_bytes, bytes);
        let root = model.source.estimated_root_hz.expect("sine root");
        assert!((root - 220.0).abs() < 3.0, "{root}");
        assert!(model.source.pitch_confidence > 0.8);
    }

    #[test]
    fn stereo_antiphase_source_uses_side_projection_without_losing_pitch() {
        let model = analyze_wav(
            "antiphase.wav",
            wav_stereo_antiphase(220.0, 0.5),
            ResynthControls::default(),
        )
        .expect("anti-phase source must remain audible");
        let root = model.source.estimated_root_hz.expect("root");
        assert!((root - 220.0).abs() < 3.0, "{root}");
        assert!(model.source.pitch_confidence > 0.8);
        assert!(
            model
                .visuals
                .waveform_rms()
                .iter()
                .any(|level| *level > 0.1)
        );
    }

    #[test]
    fn stereo_wav_duration_uses_per_channel_frames() {
        let bytes = wav_i16(
            2,
            48_000,
            (0..12_000).flat_map(|index| {
                let sample = ((std::f32::consts::TAU * 220.0 * index as f32 / 48_000.0).sin()
                    * 20_000.0) as i16;
                [sample, sample]
            }),
        );
        let model = analyze_wav("stereo.wav", bytes, ResynthControls::default()).expect("analyze");
        assert_eq!(model.source.channels, 2);
        assert_eq!(model.source.frames, 12_000);
    }

    #[test]
    fn root_rebuild_reuses_immutable_source_visual_cache() {
        let controls = ResynthControls::default();
        let bytes = wav_sine(220.0, 0.25);
        let initial = analyze_wav("cache.wav", bytes.clone(), controls).expect("analyze");
        let visuals = initial.source_visual_cache();
        let rebuilt = analyze_wav_with_root_override_and_visuals(
            "cache.wav",
            bytes,
            controls,
            Some(220.0),
            Some(Arc::clone(&visuals)),
        )
        .expect("rebuild");
        assert!(Arc::ptr_eq(&visuals, &rebuilt.visuals));
        assert_eq!(
            rebuilt.visuals.source_frames(),
            initial.visuals.source_frames()
        );
    }

    #[test]
    fn all_three_algorithms_compile_to_distinct_production_artifacts() {
        let controls = ResynthControls::default();
        let model = analyze_wav("tone.wav", wav_sine(110.0, 0.6), controls).expect("analyze");
        for algorithm in ResynthAlgorithm::ALL {
            let artifact = compile_rt_artifact(&model, algorithm, controls).expect("compile");
            assert_eq!(artifact.algorithm, algorithm);
            match (algorithm, &artifact.data) {
                (ResynthAlgorithm::Sample, ProductionResynthArtifact::Sample(sample)) => {
                    assert!(sample.frames() >= 256);
                    assert!(sample.eval(0.125).is_finite());
                }
                (ResynthAlgorithm::Grain, ProductionResynthArtifact::Grain(grain)) => {
                    let mut scheduler = crate::oscillators::GrainSchedulerState::default();
                    assert!(scheduler.render(grain, 110.0, 48_000.0, 3, 0).is_finite());
                }
                (ResynthAlgorithm::Rich, ProductionResynthArtifact::Rich(rich)) => {
                    assert!(rich.has_slabs());
                    assert!(rich.zone_for_frequency(220.0) < crate::oscillators::RICH_ZONE_COUNT);
                }
                _ => panic!("algorithm/artifact mismatch"),
            }
        }
    }

    #[test]
    fn grain_fixed_position_stays_audible_at_low_and_high_density() {
        let model = analyze_wav(
            "grain-level.wav",
            wav_sine(220.0, 1.5),
            ResynthControls::default(),
        )
        .expect("analysis");
        for density in [24.0_f32, 200.0] {
            let controls = ResynthControls {
                grain_density: density,
                grain_spray: 0.0,
                ..ResynthControls::default()
            };
            let artifact =
                compile_rt_artifact(&model, ResynthAlgorithm::Grain, controls).expect("grain");
            let ProductionResynthArtifact::Grain(grain) = &artifact.data else {
                panic!("Grain payload");
            };
            let mut scheduler = GrainSchedulerState::default();
            let mut power = 0.0_f64;
            let warmup = 24_000_u64;
            let measured = 48_000_u64;
            for frame in 0..warmup + measured {
                let sample = scheduler.render(grain, 220.0, 48_000.0, 99, frame);
                if frame >= warmup {
                    power += f64::from(sample) * f64::from(sample);
                }
            }
            let grain_rms = (power / measured as f64).sqrt() as f32;
            let source_rms = rms(&artifact.source_audition.samples) * artifact.source_audition_gain;
            let level_error_db = 20.0 * (grain_rms / source_rms.max(1.0e-9)).log10();
            assert!(
                (-18.0..=6.0).contains(&level_error_db),
                "density {density} fixed-position grain level was {level_error_db} dB"
            );
        }
    }

    #[test]
    fn rich_low_note_post_zone_retains_upper_spectrum() {
        let bytes = wav_i16(
            1,
            48_000,
            (0..32_768).map(|n| {
                let time = n as f32 / 48_000.0;
                let sample = (std::f32::consts::TAU * 110.0 * time).sin() * 0.35
                    + (std::f32::consts::TAU * 9_600.0 * time).sin() * 0.2
                    + (std::f32::consts::TAU * 14_000.0 * time).sin() * 0.1;
                (sample * 24_000.0) as i16
            }),
        );
        let controls = ResynthControls::default();
        let model = analyze_wav("bright.wav", bytes, controls).expect("analyze");
        let artifact = compile_rt_artifact(&model, ResynthAlgorithm::Rich, controls).expect("rich");
        let ProductionResynthArtifact::Rich(rich) = &artifact.data else {
            panic!("Rich payload");
        };
        assert!(rich.has_slabs());
        let zone = rich.zone_for_frequency(110.0);
        let phase_increment = rich.phase_increment(zone, 110.0, 48_000.0);
        let mut samples = vec![0.0_f32; 8_192];
        let mut phase = 0.0_f32;
        for sample in &mut samples {
            let (l, _r) = rich.eval_at_timeline_stereo(
                zone,
                phase,
                phase_increment * crate::oscillators::RICH_FRAME_SAMPLES as f32,
                0.4,
                48_000.0,
                controls.rich_dynamic,
                controls.rich_diffuse,
            );
            *sample = l;
            phase = (phase + phase_increment).rem_euclid(1.0);
        }
        let mut high_energy = 0.0_f64;
        for frequency in [9_600.0_f32] {
            let mut re = 0.0_f64;
            let mut im = 0.0_f64;
            for (index, sample) in samples.iter().copied().enumerate() {
                let angle = -std::f64::consts::TAU * f64::from(frequency) * index as f64 / 48_000.0;
                re += f64::from(sample) * angle.cos();
                im += f64::from(sample) * angle.sin();
            }
            high_energy += re * re + im * im;
        }
        assert!(
            high_energy > 1.0e-4,
            "vocoder high-band power {high_energy}"
        );
    }

    #[test]
    fn sample_artifact_repeats_without_a_seam_spike() {
        let controls = ResynthControls::default();
        let model = analyze_wav("loop.wav", wav_sine(220.0, 0.5), controls).expect("analyze");
        let artifact =
            compile_rt_artifact(&model, ResynthAlgorithm::Sample, controls).expect("sample");
        let ProductionResynthArtifact::Sample(sample) = &artifact.data else {
            panic!("Sample payload");
        };
        let before = sample.eval(1.0 - 1.0 / sample.frames() as f32);
        let after = sample.eval(0.0);
        assert!((after - before).abs() < 0.2);
        for wrap in 0..100 {
            assert_eq!(
                sample.eval(wrap as f32 + 0.25).to_bits(),
                sample.eval(0.25).to_bits()
            );
        }
    }

    #[test]
    fn oversized_source_name_is_rejected_before_commit() {
        let name = "x".repeat(MAX_RESYNTH_SOURCE_NAME_BYTES + 1);
        let error = analyze_wav(name, wav_sine(220.0, 0.1), ResynthControls::default())
            .expect_err("oversized source name must fail");
        assert!(matches!(error, ImportError::SourceNameTooLong { .. }));
    }

    #[test]
    fn oversized_source_is_rejected_before_decode() {
        let error = analyze_wav(
            "huge.wav",
            vec![0; MAX_RESYNTH_SOURCE_BYTES + 1],
            ResynthControls::default(),
        )
        .expect_err("oversize must fail");
        assert!(matches!(error, ImportError::Oversize { .. }));
    }

    #[test]
    fn grain_is_deterministic_for_a_persisted_seed() {
        let bytes = wav_sine(196.0, 0.5);
        let a = analyze_wav("a.wav", bytes.clone(), ResynthControls::default()).expect("a");
        let b = analyze_wav("b.wav", bytes, ResynthControls::default()).expect("b");
        assert_eq!(
            a.cycles[ResynthAlgorithm::Grain.index()],
            b.cycles[ResynthAlgorithm::Grain.index()]
        );
    }
    #[test]
    fn unpitched_impulse_is_accepted_without_inventing_a_root() {
        let bytes = wav_impulse();
        let controls = ResynthControls::default();
        let model = analyze_wav("impulse.wav", bytes, controls).expect("accepted");
        assert!(model.source.estimated_root_hz.is_none());
        assert!(model.cycles[ResynthAlgorithm::Grain.index()].is_some());
        assert!(model.cycles[ResynthAlgorithm::Sample.index()].is_none());
        assert!(compile_rt_artifact(&model, ResynthAlgorithm::Grain, controls).is_ok());
        assert!(matches!(
            compile_rt_artifact(&model, ResynthAlgorithm::Sample, controls),
            Err(ImportError::NoStablePitch)
        ));
    }

    #[test]
    fn manual_root_enables_sample_and_rich_without_rewriting_detection() {
        let controls = ResynthControls::default();
        let model =
            analyze_wav_with_root_override("impulse.wav", wav_impulse(), controls, Some(173.42))
                .expect("manual root");
        assert!(model.source.estimated_root_hz.is_none());
        assert_eq!(
            model.root_override_hz.map(f32::to_bits),
            Some(173.42_f32.to_bits())
        );
        assert!(model.supports_algorithm(ResynthAlgorithm::Sample));
        assert!(model.supports_algorithm(ResynthAlgorithm::Rich));
        assert!(compile_rt_artifact(&model, ResynthAlgorithm::Rich, controls).is_ok());
    }
    #[test]
    fn root_projection_observes_cancellation_inside_bandlimited_decimation() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let samples = (0..192_000)
            .map(|index| (std::f32::consts::TAU * 220.0 * index as f32 / 384_000.0).sin())
            .collect::<Vec<_>>();
        let polls = AtomicUsize::new(0);
        let result = estimate_root_window_with_cancel(&samples, 384_000, &|| {
            polls.fetch_add(1, Ordering::Relaxed) >= 2
        });
        assert!(matches!(result, Err(ImportError::Cancelled)));
        assert!(polls.load(Ordering::Relaxed) >= 3);
    }
}
