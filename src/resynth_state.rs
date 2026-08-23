//! Bounded, byte-exact RESYNTH asset persistence and lock-free RT publication.

use std::{
    collections::HashSet,
    sync::{
        Arc, Mutex, MutexGuard, OnceLock, RwLock, Weak,
        atomic::{AtomicBool, AtomicU8, AtomicU16, AtomicU32, AtomicU64, Ordering, fence},
    },
};

use truce_core::custom_state::{PersistField, StateCursor};

use crate::{
    generators::MAX_OSCILLATORS,
    oscillators::{
        AlgorithmVisualCache, PitchMode, ProductionResynthArtifact, RICH_ZONE_COUNT,
        RICH_ZONE_SAMPLES, ResynthAlgorithm, ResynthAnalysisModel, ResynthControls,
        ResynthRtArtifact, ResynthSourceMaster, ResynthVisualModel,
        analyze_sounding_artifact_visuals, analyze_sounding_artifact_visuals_with_cancel,
        analyze_wav_with_root_override_and_visuals_with_cancel, compile_rt_artifact_with_cancel,
        compile_source_audition,
    },
    wave_curve::bandlimit::TABLE_SIZE,
};

#[cfg(test)]
use crate::oscillators::{GrainSourceArtifact, RESYNTH_ALGORITHM_COUNT, SourceAuditionArtifact};

pub const MAX_RESYNTH_PACK_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_AGGREGATE_SOURCE_BYTES: usize = 20 * 1024 * 1024;
/// Hard bound for distinct immutable RESYNTH packs retained by editor history.
pub(crate) const MAX_RESYNTH_HISTORY_BYTES: usize = 128 * 1024 * 1024;

const RESYNTH_BUILD_FAILED: u8 = u8::MAX;

struct ResynthBuildStatus(AtomicU8);

impl ResynthBuildStatus {
    const fn new() -> Self {
        Self(AtomicU8::new(100))
    }

    fn set_progress(&self, percent: u8) {
        self.0.store(percent.min(100), Ordering::Release);
    }

    fn fail(&self) {
        self.0.store(RESYNTH_BUILD_FAILED, Ordering::Release);
    }

    fn snapshot(&self) -> (u8, bool) {
        let value = self.0.load(Ordering::Acquire);
        (value.min(100), value == RESYNTH_BUILD_FAILED)
    }
}

/// Process-wide lease for decoded PCM, FFT, and artifact scratch work.
///
/// Slot workers acquire this before any expansion of embedded WAV bytes and
/// retain it through artifact publication. Superseded jobs check their exact
/// revision after admission and cooperatively cancel inside every major loop.
static RESYNTH_BUILD_WORK_PERMIT: Mutex<()> = Mutex::new(());

/// Reserve the one process-wide decoded-PCM/analysis budget.
///
/// The caller acquires this before copying or decoding Source Master bytes and
/// retains it through compilation. This is never called by the audio thread.
pub(crate) fn acquire_resynth_analysis_work() -> MutexGuard<'static, ()> {
    RESYNTH_BUILD_WORK_PERMIT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
mod telemetry;

use telemetry::RESYNTH_TELEMETRY_INTEREST_CALLBACKS;
pub use telemetry::{GrainTelemetryLane, ResynthTelemetrySnapshot, ResynthTelemetryTransport};
#[cfg(test)]
use telemetry::{RESYNTH_TELEMETRY_GRAIN_LANES, ResynthTelemetryFrame};

fn admit_aggregate_source_bytes(total: usize, next: usize) -> Option<usize> {
    total
        .checked_add(next)
        .filter(|sum| *sum <= MAX_AGGREGATE_SOURCE_BYTES)
}

fn worst_resynth_entry_bytes(source_bytes: usize, name_bytes: usize) -> Option<usize> {
    192_usize
        .checked_add(name_bytes.max(512))
        .and_then(|bytes| bytes.checked_add(source_bytes))
        .and_then(|bytes| bytes.checked_add(1 + 5 + 4))
        .and_then(|bytes| bytes.checked_add(RICH_ZONE_COUNT * 4 + RICH_ZONE_COUNT * 2))
        .and_then(|bytes| {
            bytes.checked_add(RICH_ZONE_COUNT * RICH_ZONE_SAMPLES * std::mem::size_of::<f32>())
        })
}
mod codec;

use codec::*;

fn detached_resynth_work_is_safe() -> bool {
    static IMAGE_PINNED: OnceLock<bool> = OnceLock::new();
    *IMAGE_PINNED.get_or_init(truce_egui::pin_current_image_for_detached_work)
}

mod publication;

use publication::AtomicResynthArtifact;
pub(crate) use publication::{
    ResynthArtifactView, ResynthPublicationIdentity, ResynthRtPlanAck, ResynthRtUpdate,
};

#[derive(Clone)]
struct ResynthSlotDocument {
    revision: u64,
    selected: ResynthAlgorithm,
    controls: ResynthControls,
    model: Arc<ResynthAnalysisModel>,
    artifact: Arc<ResynthRtArtifact>,
    artifact_visuals: Arc<AlgorithmVisualCache>,
    artifact_generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ResynthHistoryKey {
    revisions: [u64; MAX_OSCILLATORS],
}

struct ResynthHistoryPack {
    key: ResynthHistoryKey,
    slots: [Option<ResynthSlotDocument>; MAX_OSCILLATORS],
    retained_bytes: usize,
}

/// Immutable, cheaply cloned editor-history owner for one complete RESYNTH pack.
#[derive(Clone)]
pub(crate) struct ResynthHistoryReceipt(Arc<ResynthHistoryPack>);

impl PartialEq for ResynthHistoryReceipt {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl ResynthHistoryReceipt {
    pub(crate) fn retained_bytes(&self) -> usize {
        self.0.retained_bytes
    }

    #[cfg(test)]
    pub(crate) fn allocation_id(&self) -> usize {
        Arc::as_ptr(&self.0) as usize
    }

    /// Add only uniquely retained immutable allocations to an editor history
    /// live-set. Documents in adjacent receipts commonly share these Arcs.
    pub(crate) fn accumulate_retained_bytes(&self, allocations: &mut HashSet<usize>) -> usize {
        let mut total = 0_usize;
        let pack_id = Arc::as_ptr(&self.0) as usize;
        if allocations.insert(pack_id) {
            total = total.saturating_add(std::mem::size_of::<ResynthHistoryPack>());
        }
        for document in self.0.slots.iter().flatten() {
            let model_id = Arc::as_ptr(&document.model) as usize;
            if allocations.insert(model_id) {
                total = total.saturating_add(model_retained_bytes(&document.model));
            }
            let source_visual_id = Arc::as_ptr(&document.model.visuals) as usize;
            if allocations.insert(source_visual_id) {
                total = total.saturating_add(SOURCE_VISUAL_RETAINED_UPPER_BOUND);
            }
            let artifact_id = Arc::as_ptr(&document.artifact) as usize;
            if allocations.insert(artifact_id) {
                total = total.saturating_add(artifact_retained_bytes(&document.artifact));
            }
            let visual_id = Arc::as_ptr(&document.artifact_visuals) as usize;
            if allocations.insert(visual_id) {
                total = total.saturating_add(ALGORITHM_VISUAL_RETAINED_UPPER_BOUND);
            }
        }
        total
    }
}

// Fixed caches are much smaller than these named conservative bounds. The
// estimates deliberately include Box payloads rather than only Arc headers.
const SOURCE_VISUAL_RETAINED_UPPER_BOUND: usize = 2 * 1024 * 1024;
const ALGORITHM_VISUAL_RETAINED_UPPER_BOUND: usize = 2 * 1024 * 1024;

fn model_retained_bytes(model: &ResynthAnalysisModel) -> usize {
    let bytes = std::mem::size_of::<ResynthAnalysisModel>()
        .saturating_add(model.source.file_name.len())
        .saturating_add(model.source.original_bytes.len())
        .saturating_add(model.rich_analysis_retained_bytes());
    #[cfg(test)]
    let bytes = bytes.saturating_add(
        RESYNTH_ALGORITHM_COUNT.saturating_mul(std::mem::size_of::<Option<[f32; TABLE_SIZE]>>()),
    );
    bytes
}

fn artifact_retained_bytes(artifact: &ResynthRtArtifact) -> usize {
    // One-shot mip PCM is a strict geometric sum below the authoritative PCM;
    // block integrals add less than one byte per frame. Ten bytes/frame is a
    // conservative allocation upper bound for the complete audition family.
    let source_frames = artifact.source_audition.samples.len();
    let mut total =
        std::mem::size_of::<ResynthRtArtifact>().saturating_add(source_frames.saturating_mul(10));
    total = total.saturating_add(match &artifact.data {
        // Periodic/reflected base integrals plus all mip samples/integrals are
        // bounded conservatively at 64 bytes per authoritative artifact frame.
        ProductionResynthArtifact::Sample(sample) => sample.samples.len().saturating_mul(64),
        // Each reflected mip family is smaller than its base PCM. Twelve
        // bytes/base-frame covers the base, every mip and allocation slack.
        ProductionResynthArtifact::Grain(grain) => grain
            .samples
            .len()
            .saturating_add(grain.side_samples.len())
            .saturating_add(grain.tuned_samples.len())
            .saturating_add(grain.tuned_side_samples.len())
            .saturating_mul(12)
            .saturating_add(grain.pitch_frames.len().saturating_mul(128)),
        ProductionResynthArtifact::Rich(_) => RICH_ZONE_COUNT
            .saturating_mul(RICH_ZONE_SAMPLES)
            .saturating_mul(4),
    });
    total
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ResynthHistoryRestore {
    Unchanged,
    Committed,
    Busy,
}

#[derive(Clone)]
pub struct ResynthAlgorithmVisualSnapshot {
    pub algorithm: ResynthAlgorithm,
    pub generation: u64,
    pub revision: u64,
    pub cache: Arc<AlgorithmVisualCache>,
}

mod build;

use build::*;

pub struct ResynthSlotState {
    document: RwLock<Option<ResynthSlotDocument>>,
    rt: AtomicResynthArtifact,
    desired_revision: AtomicU64,
    sounding_revision: AtomicU64,
    build_status: ResynthBuildStatus,
    pending_build: Mutex<Option<ResynthBuildJob>>,
    worker_running: AtomicBool,
    source_audition_lease: AtomicBool,
    pending_commit: Mutex<Option<PendingResynthCommit>>,
    desired_spec: Mutex<Option<ResynthDesiredSpec>>,
    pending_retry_running: AtomicBool,
    retry_weak: OnceLock<Weak<ResynthSlotState>>,
    telemetry_interest: AtomicU8,
    telemetry: ResynthTelemetryTransport,
    live_controls: [[AtomicU32; 25]; 2],
    live_seed: [AtomicU64; 2],
    live_direction: [AtomicU8; 2],
    live_pitch_wire: [AtomicU16; 2],
    live_sequence: AtomicU64,
}

impl ResynthSlotState {
    fn new() -> Self {
        Self {
            document: RwLock::new(None),
            rt: AtomicResynthArtifact::new(),
            desired_revision: AtomicU64::new(0),
            sounding_revision: AtomicU64::new(0),
            build_status: ResynthBuildStatus::new(),
            pending_build: Mutex::new(None),
            worker_running: AtomicBool::new(false),
            source_audition_lease: AtomicBool::new(false),
            pending_commit: Mutex::new(None),
            desired_spec: Mutex::new(None),
            pending_retry_running: AtomicBool::new(false),
            retry_weak: OnceLock::new(),
            telemetry_interest: AtomicU8::new(0),
            telemetry: ResynthTelemetryTransport::new(),
            live_controls: std::array::from_fn(|_| std::array::from_fn(|_| AtomicU32::new(0))),
            live_seed: std::array::from_fn(|_| AtomicU64::new(0)),
            live_direction: std::array::from_fn(|_| AtomicU8::new(0)),
            live_pitch_wire: std::array::from_fn(|_| AtomicU16::new(0)),
            live_sequence: AtomicU64::new(0),
        }
    }

    fn store_live_controls(&self, controls: ResynthControls) {
        let controls = controls.sanitized();
        // Claim the writer sequence off the audio reader. UI/build writers are
        // rare, so a CAS retry is acceptable here and never runs on audio.
        let (sequence, index) = loop {
            let sequence = self.live_sequence.load(Ordering::Acquire);
            if sequence & 1 != 0 {
                std::hint::spin_loop();
                continue;
            }
            let next = sequence.wrapping_add(1);
            if self
                .live_sequence
                .compare_exchange(sequence, next, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                let index = ((sequence / 2 + 1) & 1) as usize;
                break (sequence, index);
            }
        };
        let floats = [
            controls.position,
            controls.grain_size,
            controls.grain_density,
            controls.grain_spray,
            controls.rich_balance,
            controls.rich_formant_semitones,
            controls.rich_air_db,
            controls.rich_diffuse,
            controls.grain_envelope,
            controls.grain_timing,
            controls.grain_pitch_spread,
            controls.grain_level_spread,
            controls.grain_pan_spread,
            controls.grain_reverse,
            controls.grain_attack,
            controls.grain_hold,
            controls.grain_release,
            controls.grain_pitch,
            controls.grain_pan,
            controls.grain_level,
            controls.grain_blur,
            controls.grain_filter_cutoff,
            controls.grain_tune,
            controls.grain_stereo,
            controls.rich_dynamic,
        ];
        for (slot, value) in self.live_controls[index].iter().zip(floats) {
            slot.store(value.to_bits(), Ordering::Relaxed);
        }
        self.live_seed[index].store(controls.seed, Ordering::Relaxed);
        self.live_direction[index].store(controls.grain_direction, Ordering::Relaxed);
        let (mode, scale) = controls.pitch_mode.to_wire();
        self.live_pitch_wire[index]
            .store((u16::from(mode) << 8) | u16::from(scale), Ordering::Relaxed);
        // The selected slot is immutable while this even sequence is visible.
        self.live_sequence
            .store(sequence.wrapping_add(2), Ordering::Release);
    }

    #[must_use]
    pub(crate) fn rt_grain_controls(&self) -> Option<ResynthControls> {
        let sequence = self.live_sequence.load(Ordering::Acquire);
        if sequence == 0 || sequence & 1 != 0 {
            return None;
        }
        let index = ((sequence / 2) & 1) as usize;
        let controls = ResynthControls {
            position: f32::from_bits(self.live_controls[index][0].load(Ordering::Relaxed)),
            grain_size: f32::from_bits(self.live_controls[index][1].load(Ordering::Relaxed)),
            grain_density: f32::from_bits(self.live_controls[index][2].load(Ordering::Relaxed)),
            grain_spray: f32::from_bits(self.live_controls[index][3].load(Ordering::Relaxed)),
            rich_balance: f32::from_bits(self.live_controls[index][4].load(Ordering::Relaxed)),
            rich_formant_semitones: f32::from_bits(
                self.live_controls[index][5].load(Ordering::Relaxed),
            ),
            rich_air_db: f32::from_bits(self.live_controls[index][6].load(Ordering::Relaxed)),
            rich_diffuse: f32::from_bits(self.live_controls[index][7].load(Ordering::Relaxed)),
            grain_envelope: f32::from_bits(self.live_controls[index][8].load(Ordering::Relaxed)),
            grain_timing: f32::from_bits(self.live_controls[index][9].load(Ordering::Relaxed)),
            grain_pitch_spread: f32::from_bits(
                self.live_controls[index][10].load(Ordering::Relaxed),
            ),
            grain_level_spread: f32::from_bits(
                self.live_controls[index][11].load(Ordering::Relaxed),
            ),
            grain_pan_spread: f32::from_bits(self.live_controls[index][12].load(Ordering::Relaxed)),
            grain_reverse: f32::from_bits(self.live_controls[index][13].load(Ordering::Relaxed)),
            grain_attack: f32::from_bits(self.live_controls[index][14].load(Ordering::Relaxed)),
            grain_hold: f32::from_bits(self.live_controls[index][15].load(Ordering::Relaxed)),
            grain_release: f32::from_bits(self.live_controls[index][16].load(Ordering::Relaxed)),
            grain_pitch: f32::from_bits(self.live_controls[index][17].load(Ordering::Relaxed)),
            grain_pan: f32::from_bits(self.live_controls[index][18].load(Ordering::Relaxed)),
            grain_level: f32::from_bits(self.live_controls[index][19].load(Ordering::Relaxed)),
            grain_blur: f32::from_bits(self.live_controls[index][20].load(Ordering::Relaxed)),
            grain_filter_cutoff: f32::from_bits(
                self.live_controls[index][21].load(Ordering::Relaxed),
            ),
            grain_tune: f32::from_bits(self.live_controls[index][22].load(Ordering::Relaxed)),
            grain_stereo: f32::from_bits(self.live_controls[index][23].load(Ordering::Relaxed)),
            rich_dynamic: f32::from_bits(self.live_controls[index][24].load(Ordering::Relaxed)),
            grain_direction: self.live_direction[index].load(Ordering::Relaxed),
            pitch_mode: {
                let wire = self.live_pitch_wire[index].load(Ordering::Relaxed);
                PitchMode::from_wire((wire >> 8) as u8, wire as u8).unwrap_or(PitchMode::Classic)
            },
            seed: self.live_seed[index].load(Ordering::Relaxed),
        };
        fence(Ordering::Acquire);
        if self.live_sequence.load(Ordering::Acquire) != sequence {
            return None;
        }
        Some(controls.sanitized())
    }

    pub fn apply_live_controls(&self, controls: ResynthControls) {
        let controls = controls.sanitized();
        self.store_live_controls(controls);
        if let Ok(mut desired) = self.desired_spec.lock()
            && let Some(spec) = desired.as_mut()
        {
            spec.controls = controls;
        }
        if let Ok(mut document) = self.document.write()
            && let Some(document) = document.as_mut()
        {
            document.controls = controls;
        }
    }

    /// Activates the non-persisted press-and-hold Source audition. The editor
    /// explicitly clears it on release and on close; the audio thread only
    /// performs a wait-free load, so callback size cannot make the hold chatter.
    pub fn renew_source_audition(&self) {
        self.source_audition_lease.store(true, Ordering::Release);
    }

    pub(crate) fn consume_source_audition_lease(&self) -> bool {
        self.source_audition_lease.load(Ordering::Acquire)
    }

    pub fn reset_source_audition(&self) {
        self.source_audition_lease.store(false, Ordering::Release);
    }

    pub(crate) fn publish_telemetry(&self, value: ResynthTelemetrySnapshot) {
        self.telemetry.publish(value);
    }

    /// Consume one callback from the fixed UI-interest lease.
    ///
    /// The audio callback is the sole decrementing caller. UI readers only
    /// renew the counter, so the load followed by `fetch_sub` cannot underflow
    /// and remains a fixed two-atomic RT path.
    #[inline]
    pub(crate) fn consume_telemetry_interest(&self) -> bool {
        if self.telemetry_interest.load(Ordering::Acquire) == 0 {
            return false;
        }
        self.telemetry_interest.fetch_sub(1, Ordering::Relaxed);
        true
    }

    #[must_use]
    pub fn telemetry_snapshot(&self) -> ResynthTelemetrySnapshot {
        self.telemetry_interest
            .store(RESYNTH_TELEMETRY_INTEREST_CALLBACKS, Ordering::Release);
        self.telemetry.snapshot()
    }

    #[must_use]
    pub fn visual_model(&self) -> Option<Arc<crate::oscillators::ResynthVisualModel>> {
        if let Some(visuals) = self
            .document
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .map(|document| Arc::clone(&document.model.visuals))
        {
            return Some(visuals);
        }
        self.desired_spec
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .map(|desired| Arc::clone(&desired.visuals))
    }

    #[must_use]
    pub fn algorithm_visual_snapshot(&self) -> Option<ResynthAlgorithmVisualSnapshot> {
        let document = self
            .document
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let document = document.as_ref()?;
        Some(ResynthAlgorithmVisualSnapshot {
            algorithm: document.selected,
            generation: document.artifact_generation,
            revision: document.revision,
            cache: Arc::clone(&document.artifact_visuals),
        })
    }

    #[must_use]
    pub fn sounding_artifact(
        &self,
    ) -> Option<std::sync::Arc<crate::oscillators::ResynthRtArtifact>> {
        self.document
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .map(|document| std::sync::Arc::clone(&document.artifact))
    }

    #[must_use]
    pub fn has_source(&self) -> bool {
        self.document
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_some()
    }

    #[must_use]
    pub fn source_summary(&self) -> Option<ResynthSourceSummary> {
        self.try_commit_pending();
        self.rt.collect();
        let (desired_revision, sounding_revision) = loop {
            let desired_before = self.desired_revision.load(Ordering::Acquire);
            let sounding = self.sounding_revision.load(Ordering::Acquire);
            fence(Ordering::Acquire);
            let desired_after = self.desired_revision.load(Ordering::Relaxed);
            if desired_before == desired_after {
                break (desired_after, sounding);
            }
        };
        let pending = desired_revision != sounding_revision;
        let (raw_progress, build_failed) = self.build_status.snapshot();
        let progress_percent = if !pending && !build_failed {
            100
        } else if build_failed {
            raw_progress
        } else {
            raw_progress.min(99)
        };
        let desired = self
            .desired_spec
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if pending
            && let Some(desired) = desired
            && desired.revision == desired_revision
        {
            return Some(ResynthSourceSummary {
                file_name: desired.file_name,
                source_bytes: desired.bytes.len(),
                sample_rate: desired.sample_rate,
                channels: desired.channels,
                frames: desired.frames,
                estimated_root_hz: desired.detected_root_hz,
                root_override_hz: desired.root_override_hz,
                pitch_confidence: desired.pitch_confidence,
                selected: desired.selected,
                controls: desired.controls,
                desired_revision,
                sounding_revision,
                progress_percent,
                build_failed,
            });
        }
        let document = self
            .document
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let document = document.as_ref()?;
        Some(ResynthSourceSummary {
            file_name: document.model.source.file_name.clone(),
            source_bytes: document.model.source.original_bytes.len(),
            sample_rate: document.model.source.sample_rate,
            channels: document.model.source.channels,
            frames: document.model.source.frames,
            estimated_root_hz: document.model.source.estimated_root_hz,
            root_override_hz: document.model.root_override_hz,
            pitch_confidence: document.model.source.pitch_confidence,
            selected: document.selected,
            controls: document.controls,
            desired_revision,
            sounding_revision,
            progress_percent,
            build_failed,
        })
    }

    #[must_use]
    pub fn source_export_snapshot(&self) -> Option<ResynthSourceExportSnapshot> {
        let stored = self
            .document
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let source = &stored.as_ref()?.model.source;
        Some(ResynthSourceExportSnapshot {
            file_name: source.file_name.clone(),
            original_bytes: source.original_bytes.clone(),
        })
    }

    #[cfg(test)]
    #[must_use]
    pub fn preview_cycle(&self, algorithm: ResynthAlgorithm) -> Option<[f32; 128]> {
        let stored = self
            .document
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let cycle = stored.as_ref()?.model.cycles[algorithm.index()].as_ref()?;
        Some(std::array::from_fn(|index| cycle[index * TABLE_SIZE / 128]))
    }

    pub(crate) fn try_rt_view_after(&self, observed: u64) -> Option<ResynthArtifactView> {
        self.rt.try_view_after(observed)
    }

    pub(crate) fn published_rt_generation(&self) -> u64 {
        self.rt.published_generation()
    }

    pub(crate) fn acknowledge_rt(&self, seen: u64, plan: ResynthRtPlanAck) {
        self.rt.acknowledge(seen, plan);
        if plan.accepted.is_present() {
            self.sounding_revision
                .store(plan.accepted.revision, Ordering::Release);
        }
    }

    fn snapshot_intent(
        stored: &Option<ResynthSlotDocument>,
        desired: &Option<ResynthDesiredSpec>,
        desired_revision: u64,
        build_failed: bool,
    ) -> ResynthSerializableSnapshot {
        let committed = stored.clone();
        if build_failed
            || committed
                .as_ref()
                .is_some_and(|document| document.revision == desired_revision)
        {
            return ResynthSerializableSnapshot::Committed(committed);
        }
        match desired.as_ref() {
            Some(desired) if desired.revision == desired_revision => {
                ResynthSerializableSnapshot::Desired(desired.clone())
            }
            // A newer revision with no desired source is a pending clear, not
            // permission to serialize the older committed document.
            None => ResynthSerializableSnapshot::Committed(None),
            _ => ResynthSerializableSnapshot::Committed(committed),
        }
    }

    fn materialize_snapshot(intent: ResynthSerializableSnapshot) -> Option<ResynthSlotDocument> {
        let desired = match intent {
            ResynthSerializableSnapshot::Committed(document) => return document,
            ResynthSerializableSnapshot::Desired(desired) => desired,
        };

        // Host state serialization is explicitly off the audio thread and may
        // compile a private immutable snapshot of a still-pending request. It
        // shares the same decoded-PCM/scratch lease as import and rebuild work.
        let _work = acquire_resynth_analysis_work();
        let model = analyze_wav_with_root_override_and_visuals_with_cancel(
            desired.file_name,
            desired.bytes.to_vec(),
            desired.controls,
            desired.root_override_hz,
            None,
            || false,
        )
        .ok()?;
        let artifact = Arc::new(
            compile_rt_artifact_with_cancel(&model, desired.selected, desired.controls, || false)
                .ok()?,
        );
        let artifact_visuals = analyze_sounding_artifact_visuals(&artifact);
        Some(ResynthSlotDocument {
            revision: desired.revision,
            selected: desired.selected,
            controls: desired.controls,
            model: Arc::new(model),
            artifact,
            artifact_visuals,
            artifact_generation: 0,
        })
    }

    fn snapshot(&self) -> Option<ResynthSlotDocument> {
        self.try_commit_pending();
        self.rt.collect();
        // Desired revision writers take `document` before `desired_spec`, so
        // these guards define one exact serialization point. Never infer the
        // returned document's identity from `sounding_revision`: audio may not
        // have accepted an already committed document yet.
        let stored = self
            .document
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let desired = self
            .desired_spec
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let intent = Self::snapshot_intent(
            &stored,
            &desired,
            self.desired_revision.load(Ordering::Acquire),
            self.build_status.snapshot().1,
        );
        drop(desired);
        drop(stored);
        Self::materialize_snapshot(intent)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResynthSourceExportSnapshot {
    pub file_name: String,
    pub original_bytes: Vec<u8>,
}

pub struct ResynthSourceSummary {
    pub file_name: String,
    pub source_bytes: usize,
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: u32,
    pub estimated_root_hz: Option<f32>,
    pub root_override_hz: Option<f32>,
    pub pitch_confidence: f32,
    pub selected: ResynthAlgorithm,
    pub controls: ResynthControls,
    pub desired_revision: u64,
    pub sounding_revision: u64,
    pub progress_percent: u8,
    pub build_failed: bool,
}

struct PendingResynthPackCommit {
    incoming: [Option<ResynthSlotDocument>; MAX_OSCILLATORS],
    /// Desired revisions observed while every slot document gate was held.
    /// Any later per-slot intent makes the complete aggregate transaction stale.
    accepted_revisions: Option<[u64; MAX_OSCILLATORS]>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PendingPackCommitResult {
    Empty,
    Stale,
    Backpressured,
    Committed,
}

/// Off-thread owner for one decoded aggregate restore and its retry worker.
///
/// Keeping this behind an `Arc` lets the retry task hold only weak slot owners:
/// dropping the pack therefore terminates a blocked retry instead of extending
/// the lifetime of the complete plugin state.
struct ResynthPackRestoreState {
    pending: Mutex<Option<PendingResynthPackCommit>>,
    retry_running: AtomicBool,
    /// Even while stable and odd while one aggregate pointer/document commit
    /// is in progress. The pending mutex serializes the sole aggregate writer.
    publication_epoch: AtomicU64,
    slots: Box<[Weak<ResynthSlotState>]>,
}

pub struct ResynthAssetPackState {
    slots: Box<[Arc<ResynthSlotState>]>,
    import_budget: Mutex<()>,
    restore: Arc<ResynthPackRestoreState>,
    history_cache: Mutex<Option<(ResynthHistoryKey, ResynthHistoryReceipt)>>,
}

impl ResynthPackRestoreState {
    fn upgrade_slots(&self) -> Option<Vec<Arc<ResynthSlotState>>> {
        let mut slots = Vec::with_capacity(MAX_OSCILLATORS);
        for slot in &self.slots {
            slots.push(slot.upgrade()?);
        }
        Some(slots)
    }

    fn schedule_retry(self: &Arc<Self>) {
        if !detached_resynth_work_is_safe()
            || self
                .retry_running
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
        {
            return;
        }
        let restore = Arc::clone(self);
        let spawn = std::thread::Builder::new()
            .name("kurv-resynth-pack-publish".to_owned())
            .spawn(move || {
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    let Some(slots) = restore.upgrade_slots() else {
                        restore.retry_running.store(false, Ordering::Release);
                        break;
                    };
                    if ResynthAssetPackState::try_commit_pending_restore_for(
                        restore.as_ref(),
                        &slots,
                    ) == PendingPackCommitResult::Backpressured
                    {
                        continue;
                    }
                    restore.retry_running.store(false, Ordering::Release);
                    // Close the race with a newer decoded transaction arriving
                    // immediately before the running flag was cleared.
                    let pending = restore
                        .pending
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .is_some();
                    if pending {
                        restore.schedule_retry();
                    }
                    break;
                }
            });
        if spawn.is_err() {
            // The decoded documents remain retained. A later off-thread retry
            // can publish them without asking the host to resend state.
            self.retry_running.store(false, Ordering::Release);
        }
    }
}

impl ResynthAssetPackState {
    #[must_use]
    pub fn new() -> Self {
        let slots: Box<[Arc<ResynthSlotState>]> = (0..MAX_OSCILLATORS)
            .map(|_| Arc::new(ResynthSlotState::new()))
            .collect();
        for slot in &slots {
            let initialized = slot.retry_weak.set(Arc::downgrade(slot)).is_ok();
            debug_assert!(
                initialized,
                "fresh RESYNTH slot retry owner must initialize once"
            );
        }
        let restore = Arc::new(ResynthPackRestoreState {
            pending: Mutex::new(None),
            retry_running: AtomicBool::new(false),
            publication_epoch: AtomicU64::new(0),
            slots: slots.iter().map(Arc::downgrade).collect(),
        });
        Self {
            slots,
            import_budget: Mutex::new(()),
            restore,
            history_cache: Mutex::new(None),
        }
    }

    #[must_use]
    pub fn slot(&self, index: usize) -> Option<&ResynthSlotState> {
        self.slots.get(index).map(Arc::as_ref)
    }

    #[must_use]
    pub fn slot_arc(&self, index: usize) -> Option<Arc<ResynthSlotState>> {
        self.slots.get(index).cloned()
    }

    fn history_key(&self) -> ResynthHistoryKey {
        ResynthHistoryKey {
            revisions: std::array::from_fn(|index| {
                self.slots[index].sounding_revision.load(Ordering::Acquire)
            }),
        }
    }

    fn published_history_key(&self) -> ResynthHistoryKey {
        let _aggregate = self
            .restore
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let documents = self
            .slots
            .iter()
            .map(|slot| {
                slot.document
                    .read()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
            })
            .collect::<Vec<_>>();
        ResynthHistoryKey {
            revisions: std::array::from_fn(|index| {
                documents[index].as_ref().map_or_else(
                    || self.slots[index].desired_revision.load(Ordering::Acquire),
                    |document| document.revision,
                )
            }),
        }
    }

    /// Capture the committed document set without serializing or compiling it.
    /// Repeated editor gestures reuse the exact same immutable Arc receipt.
    pub(crate) fn history_receipt(
        &self,
        previous: Option<&ResynthHistoryReceipt>,
    ) -> Option<ResynthHistoryReceipt> {
        let _aggregate = self
            .restore
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let documents = self
            .slots
            .iter()
            .map(|slot| {
                slot.document
                    .read()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
            })
            .collect::<Vec<_>>();
        let key = loop {
            let before = self.history_key();
            fence(Ordering::Acquire);
            let after = self.history_key();
            if before == after {
                break after;
            }
        };
        let all_audio_accepted = self.slots.iter().enumerate().all(|(index, slot)| {
            let sounding = key.revisions[index];
            documents[index].as_ref().map_or_else(
                || slot.desired_revision.load(Ordering::Acquire) == sounding,
                |document| document.revision == sounding,
            )
        });

        let mut cache = self
            .history_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !all_audio_accepted {
            if let Some(previous) = previous {
                return Some(previous.clone());
            }
            return cache
                .as_ref()
                .and_then(|(cached_key, receipt)| (*cached_key == key).then(|| receipt.clone()));
        }
        if let Some((cached_key, receipt)) = cache.as_ref()
            && *cached_key == key
        {
            return Some(receipt.clone());
        }
        if let Some(previous) = previous
            && previous.0.key == key
        {
            *cache = Some((key, previous.clone()));
            return Some(previous.clone());
        }

        let slots = std::array::from_fn(|index| documents[index].clone());
        let mut allocations = HashSet::new();
        let mut retained_bytes = std::mem::size_of::<ResynthHistoryPack>();
        for document in slots.iter().flatten() {
            let model_id = Arc::as_ptr(&document.model) as usize;
            if allocations.insert(model_id) {
                retained_bytes =
                    retained_bytes.saturating_add(model_retained_bytes(&document.model));
            }
            let source_visual_id = Arc::as_ptr(&document.model.visuals) as usize;
            if allocations.insert(source_visual_id) {
                retained_bytes = retained_bytes.saturating_add(SOURCE_VISUAL_RETAINED_UPPER_BOUND);
            }
            let artifact_id = Arc::as_ptr(&document.artifact) as usize;
            if allocations.insert(artifact_id) {
                retained_bytes =
                    retained_bytes.saturating_add(artifact_retained_bytes(&document.artifact));
            }
            let visual_id = Arc::as_ptr(&document.artifact_visuals) as usize;
            if allocations.insert(visual_id) {
                retained_bytes =
                    retained_bytes.saturating_add(ALGORITHM_VISUAL_RETAINED_UPPER_BOUND);
            }
        }
        let receipt = ResynthHistoryReceipt(Arc::new(ResynthHistoryPack {
            key,
            slots,
            retained_bytes,
        }));
        *cache = Some((key, receipt.clone()));
        Some(receipt)
    }

    pub(crate) fn matches_history(&self, receipt: &ResynthHistoryReceipt) -> bool {
        // History application targets the coherently published document set.
        // Capture remains stricter and only creates new receipts after audio
        // has accepted those documents; this alias prevents duplicate editor
        // commits while the accepted plan catches up to a restored receipt.
        let key = self.published_history_key();
        if receipt.0.key == key {
            return true;
        }
        self.history_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .is_some_and(|(cached_key, cached)| {
                *cached_key == key && Arc::ptr_eq(&cached.0, &receipt.0)
            })
    }

    /// Restore one editor-history pack immediately or leave all live state
    /// untouched. Host state recall keeps its separate retained retry path.
    pub(crate) fn try_restore_history(
        &self,
        receipt: &ResynthHistoryReceipt,
    ) -> ResynthHistoryRestore {
        if self.matches_history(receipt) {
            return ResynthHistoryRestore::Unchanged;
        }
        let transaction = PendingResynthPackCommit {
            incoming: receipt.0.slots.clone(),
            accepted_revisions: None,
        };
        let result = {
            let mut retained = self
                .restore
                .pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let mut immediate = Some(transaction);
            let result = Self::try_commit_pending_restore_locked(
                &self.slots,
                &self.restore.publication_epoch,
                &mut immediate,
            );
            if result == PendingPackCommitResult::Committed {
                // A successful editor restore is newer than any retained host
                // transaction that was waiting behind the same aggregate gate.
                *retained = None;
            }
            result
        };
        if result != PendingPackCommitResult::Committed {
            return ResynthHistoryRestore::Busy;
        }

        // Cache the future accepted revision vector. Until the audio plan
        // accepts it, `matches_history` still reports the prior sounding pack.
        let key = self.published_history_key();
        *self
            .history_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some((key, receipt.clone()));
        ResynthHistoryRestore::Committed
    }

    /// Attempt one allocation-free, lock-free coherent publication read.
    ///
    /// Aggregate restore opens an odd pack epoch before its first slot store and
    /// closes it after its last document install. A callback that intersects
    /// that window retains every previous plan and retries next block.
    #[must_use]
    pub(crate) fn try_rt_update_after(
        &self,
        observed: &[u64; MAX_OSCILLATORS],
    ) -> Option<ResynthRtUpdate> {
        let before = self.restore.publication_epoch.load(Ordering::Acquire);
        if before & 1 != 0 {
            return None;
        }
        let mut views = [ResynthArtifactView::NONE; MAX_OSCILLATORS];
        let mut changed_mask = 0_u32;
        for (index, slot) in self.slots.iter().enumerate() {
            if let Some(view) = slot.try_rt_view_after(observed[index]) {
                views[index] = view;
                changed_mask |= 1_u32 << index;
            }
        }
        fence(Ordering::Acquire);
        let after = self.restore.publication_epoch.load(Ordering::Relaxed);
        (changed_mask != 0 && before == after && after & 1 == 0).then_some(ResynthRtUpdate {
            changed_mask,
            views,
        })
    }

    /// Read one fixed monitor frame for a logical oscillator slot.
    #[must_use]
    pub fn telemetry_snapshot(&self, index: usize) -> ResynthTelemetrySnapshot {
        self.slots
            .get(index)
            .map_or_else(ResynthTelemetrySnapshot::default, |slot| {
                slot.telemetry_snapshot()
            })
    }

    #[must_use]
    pub fn request_import(
        &self,
        index: usize,
        model: ResynthAnalysisModel,
        selected: ResynthAlgorithm,
        controls: ResynthControls,
    ) -> Option<u64> {
        let _budget = self
            .import_budget
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !self.can_replace_source(index, model.source.original_bytes.len()) {
            return None;
        }
        self.slots
            .get(index)?
            .request_import(model, selected, controls)
    }

    pub fn can_replace_source(&self, index: usize, new_bytes: usize) -> bool {
        if index >= self.slots.len() || new_bytes > crate::oscillators::MAX_RESYNTH_SOURCE_BYTES {
            return false;
        }
        let mut source_total = 0_usize;
        let mut pack_total = MAGIC.len() + 2 + 4 + HASH_BYTES + 2;
        for (slot_index, slot) in self.slots.iter().enumerate() {
            if slot_index == index {
                continue;
            }
            // Desired specs reserve their Source bytes immediately, before an
            // async compiler publishes, so concurrent first imports cannot
            // oversubscribe a pack that decode would later reject.
            let desired = slot
                .desired_spec
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            let committed = slot
                .document
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let (source_bytes, name_bytes) = if let Some(desired) = desired {
                (desired.bytes.len(), desired.file_name.len())
            } else if let Some(document) = committed.as_ref() {
                (
                    document.model.source.original_bytes.len(),
                    document.model.source.file_name.len(),
                )
            } else {
                continue;
            };
            source_total = match source_total.checked_add(source_bytes) {
                Some(total) => total,
                None => return false,
            };
            let Some(entry) = worst_resynth_entry_bytes(source_bytes, name_bytes) else {
                return false;
            };
            pack_total = match pack_total.checked_add(entry) {
                Some(total) => total,
                None => return false,
            };
        }
        let source_ok = source_total
            .checked_add(new_bytes)
            .is_some_and(|bytes| bytes <= MAX_AGGREGATE_SOURCE_BYTES);
        source_ok
            && worst_resynth_entry_bytes(
                new_bytes,
                crate::oscillators::MAX_RESYNTH_SOURCE_NAME_BYTES,
            )
            .and_then(|entry| pack_total.checked_add(entry))
            .is_some_and(|bytes| bytes <= MAX_RESYNTH_PACK_BYTES)
    }

    pub fn reset_source_auditions(&self) {
        for slot in &self.slots {
            slot.reset_source_audition();
        }
    }

    pub fn clear(&self) {
        self.reset_source_auditions();
        // Empty host state/factory reset uses the same retained all-slot
        // transaction as a decoded pack. A blocked slot therefore cannot leave
        // earlier documents cleared while later documents remain live.
        let _ = self.replace_all(Vec::new());
    }

    fn encode(&self) -> Option<Vec<u8>> {
        self.encode_version(PACK_VERSION)
    }

    fn encode_version(&self, pack_version: u16) -> Option<Vec<u8>> {
        if !(LEGACY_PACK_VERSION..=PACK_VERSION).contains(&pack_version) {
            return None;
        }

        // Commit any already-built slot result before taking the ordered
        // aggregate read snapshot. No writer can advance desired intent while
        // all document gates below are held.
        for slot in &self.slots {
            slot.try_commit_pending();
            slot.rt.collect();
        }
        // Serialize against aggregate admission/retry. If the pending request
        // is still current, get-state must reflect that retained set-state
        // intent rather than the documents that happen to remain sounding.
        let mut pending = self
            .restore
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let document_guards = self
            .slots
            .iter()
            .map(|slot| {
                slot.document
                    .read()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
            })
            .collect::<Vec<_>>();
        let desired_guards = self
            .slots
            .iter()
            .map(|slot| {
                slot.desired_spec
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
            })
            .collect::<Vec<_>>();
        let stale = pending.as_ref().is_some_and(|transaction| {
            transaction.accepted_revisions.is_some_and(|accepted| {
                self.slots.iter().enumerate().any(|(index, slot)| {
                    slot.desired_revision.load(Ordering::Acquire) != accepted[index]
                })
            })
        });
        if stale {
            *pending = None;
        }

        let incoming = pending
            .as_ref()
            .map(|transaction| transaction.incoming.clone());
        let intents = incoming.is_none().then(|| {
            self.slots
                .iter()
                .enumerate()
                .map(|(index, slot)| {
                    ResynthSlotState::snapshot_intent(
                        &document_guards[index],
                        &desired_guards[index],
                        slot.desired_revision.load(Ordering::Acquire),
                        slot.build_status.snapshot().1,
                    )
                })
                .collect::<Vec<_>>()
        });
        drop(desired_guards);
        drop(document_guards);
        drop(pending);

        let mut payload = Vec::new();
        payload.push(u8::try_from(MAX_OSCILLATORS).ok()?);
        if let Some(incoming) = incoming {
            for (index, document) in incoming.iter().enumerate() {
                if let Some(document) = document {
                    Self::encode_document(&mut payload, index, document, pack_version)?;
                }
            }
        } else if let Some(intents) = intents {
            for (index, intent) in intents.into_iter().enumerate() {
                if let Some(document) = ResynthSlotState::materialize_snapshot(intent) {
                    Self::encode_document(&mut payload, index, &document, pack_version)?;
                }
            }
        }
        Self::finish_encoded_pack(payload, pack_version)
    }

    fn encode_document(
        payload: &mut Vec<u8>,
        index: usize,
        document: &ResynthSlotDocument,
        pack_version: u16,
    ) -> Option<()> {
        let name_bytes = document.model.source.file_name.len();
        if name_bytes > crate::oscillators::MAX_RESYNTH_SOURCE_NAME_BYTES {
            return None;
        }
        let source_bytes = document.model.source.original_bytes.len();
        let fixed_bytes = 2_usize // sparse slot + Algorithm
            .checked_add(8 * 4 + 8)? // legacy controls
            .checked_add(if pack_has_grain_play(pack_version) {
                4 + 6 * 4
                    + if pack_has_grain_envelope(pack_version) {
                        3 * 4
                    } else {
                        0
                    }
                    + if pack_has_grain_offsets(pack_version) {
                        3 * 4
                    } else {
                        0
                    }
                    + if pack_has_grain_effects(pack_version) {
                        2 * 4
                    } else {
                        0
                    }
                    + if pack_has_continuous_mode_controls(pack_version) {
                        3 * 4
                    } else {
                        usize::from(pack_has_grain_tune(pack_version))
                    }
                    + if pack_has_pitch_mode(pack_version) {
                        2
                    } else {
                        0
                    }
            } else {
                0
            })?
            .checked_add(4 + 2 + 4 + (1 + 4) + 4 + (1 + 4) + 2 + 4)? // source metadata/roots/lengths
            .checked_add(if pack_has_preview_cycles(pack_version) {
                1 + crate::oscillators::RESYNTH_ALGORITHM_COUNT * TABLE_SIZE * 4
            } else {
                0
            })?
            .checked_add(artifact_persisted_bytes(
                document.artifact.as_ref(),
                pack_version,
            )?)?;
        let required = fixed_bytes
            .checked_add(name_bytes)?
            .checked_add(source_bytes)?;
        let envelope_bytes = MAGIC.len() + 2 + 4 + HASH_BYTES;
        if payload
            .len()
            .checked_add(required)
            .and_then(|bytes| bytes.checked_add(1 + envelope_bytes))
            .is_none_or(|bytes| bytes > MAX_RESYNTH_PACK_BYTES)
        {
            return None;
        }
        payload.push(u8::try_from(index).ok()?);
        payload.push(document.selected as u8);
        write_controls(payload, document.controls, pack_version);
        write_u32(payload, document.model.source.sample_rate);
        write_u16(payload, document.model.source.channels);
        write_u32(payload, document.model.source.frames);
        write_option_f32(payload, document.model.source.estimated_root_hz);
        write_f32(payload, document.model.source.pitch_confidence);
        write_option_f32(payload, document.model.root_override_hz);
        #[cfg(test)]
        let cycle_mask = document
            .model
            .cycles
            .iter()
            .enumerate()
            .fold(0_u8, |mask, (index, cycle)| {
                mask | (u8::from(cycle.is_some()) << index)
            });
        let name = document.model.source.file_name.as_bytes();
        write_u16(payload, u16::try_from(name.len()).ok()?);
        payload.extend_from_slice(name);
        write_u32(
            payload,
            u32::try_from(document.model.source.original_bytes.len()).ok()?,
        );
        payload.extend_from_slice(&document.model.source.original_bytes);
        if pack_has_preview_cycles(pack_version) {
            #[cfg(test)]
            {
                payload.push(cycle_mask);
                for cycle in document.model.cycles.iter() {
                    if let Some(cycle) = cycle {
                        for sample in cycle {
                            write_f32(payload, *sample);
                        }
                    } else {
                        for _ in 0..TABLE_SIZE {
                            write_f32(payload, 0.0);
                        }
                    }
                }
            }
            #[cfg(not(test))]
            return None;
        }
        write_artifact(payload, document.artifact.as_ref(), pack_version)?;
        (payload.len() <= MAX_RESYNTH_PACK_BYTES).then_some(())
    }

    fn finish_encoded_pack(mut payload: Vec<u8>, pack_version: u16) -> Option<Vec<u8>> {
        // `0xff` terminates sparse slots; slot indexes are always 0..31.
        payload.push(u8::MAX);
        let mut output = Vec::with_capacity(8 + 2 + 4 + HASH_BYTES + payload.len());
        output.extend_from_slice(MAGIC);
        write_u16(&mut output, pack_version);
        write_u32(&mut output, u32::try_from(payload.len()).ok()?);
        output.extend_from_slice(blake3::hash(&payload).as_bytes());
        output.extend_from_slice(&payload);
        (output.len() <= MAX_RESYNTH_PACK_BYTES).then_some(output)
    }

    fn decode(data: &[u8]) -> Option<Vec<(usize, ResynthSlotDocument)>> {
        if data.len() > MAX_RESYNTH_PACK_BYTES || data.len() < 8 + 2 + 4 + HASH_BYTES + 2 {
            return None;
        }
        let mut input = Reader::new(data);
        if input.bytes(8)? != MAGIC {
            return None;
        }
        let pack_version = input.u16()?;
        if !(LEGACY_PACK_VERSION..=PACK_VERSION).contains(&pack_version) {
            return None;
        }
        let payload_len = usize::try_from(input.u32()?).ok()?;
        let expected_hash = input.bytes(HASH_BYTES)?;
        let payload = input.bytes(payload_len)?;
        if input.remaining() != 0 || blake3::hash(payload).as_bytes() != expected_hash {
            return None;
        }
        let mut input = Reader::new(payload);
        if usize::from(input.u8()?) != MAX_OSCILLATORS {
            return None;
        }
        let mut output = Vec::new();
        let mut seen = 0_u32;
        let mut aggregate_source_bytes = 0_usize;
        loop {
            let index = input.u8()?;
            if index == u8::MAX {
                break;
            }
            let index = usize::from(index);
            if index >= MAX_OSCILLATORS || seen & (1 << index) != 0 {
                return None;
            }
            seen |= 1 << index;
            let selected = ResynthAlgorithm::from_u8(input.u8()?)?;
            let controls = read_controls(&mut input, pack_version)?.sanitized();
            let sample_rate = input.u32()?;
            let channels = input.u16()?;
            let frames = input.u32()?;
            let estimated_root_hz = read_option_f32(&mut input)?;
            let pitch_confidence = input.f32()?;
            let root_override_hz = read_option_f32(&mut input)?;
            if !(8_000..=384_000).contains(&sample_rate)
                || !(1..=2).contains(&channels)
                || frames == 0
                || usize::try_from(frames).ok()? > crate::oscillators::MAX_RESYNTH_DECODED_FRAMES
                || estimated_root_hz
                    .is_some_and(|root| !root.is_finite() || !(20.0..=2_000.0).contains(&root))
                || root_override_hz
                    .is_some_and(|root| !root.is_finite() || !(20.0..=2_000.0).contains(&root))
                || !pitch_confidence.is_finite()
                || !(0.0..=1.0).contains(&pitch_confidence)
            {
                return None;
            }
            let name_len = usize::from(input.u16()?);
            if name_len > crate::oscillators::MAX_RESYNTH_SOURCE_NAME_BYTES {
                return None;
            }
            let file_name = String::from_utf8(input.bytes(name_len)?.to_vec()).ok()?;
            let source_len = usize::try_from(input.u32()?).ok()?;
            if source_len > crate::oscillators::MAX_RESYNTH_SOURCE_BYTES {
                return None;
            }
            aggregate_source_bytes =
                admit_aggregate_source_bytes(aggregate_source_bytes, source_len)?;
            let original_bytes = input.bytes(source_len)?.to_vec();
            let embedded =
                hound::WavReader::new(std::io::Cursor::new(original_bytes.as_slice())).ok()?;
            let embedded_spec = embedded.spec();
            if embedded_spec.sample_rate != sample_rate
                || embedded_spec.channels != channels
                || embedded.duration() != frames
            {
                return None;
            }
            let effective_root_hz = root_override_hz.or(estimated_root_hz);
            #[cfg(test)]
            let mut cycles: Box<
                [Option<[f32; TABLE_SIZE]>; crate::oscillators::RESYNTH_ALGORITHM_COUNT],
            > = Box::new(std::array::from_fn(|_| None));
            if pack_has_preview_cycles(pack_version) {
                let cycle_mask = input.u8()?;
                if cycle_mask & !0b111 != 0 {
                    return None;
                }
                for cycle_index in 0..crate::oscillators::RESYNTH_ALGORITHM_COUNT {
                    let mut decoded = [0.0_f32; TABLE_SIZE];
                    for sample in &mut decoded {
                        *sample = input.f32()?;
                        if !sample.is_finite() || sample.abs() > MAX_ARTIFACT_ABS_SAMPLE {
                            return None;
                        }
                    }
                    if cycle_mask & (1 << cycle_index) == 0
                        && decoded.iter().any(|sample| sample.to_bits() != 0)
                    {
                        return None;
                    }
                    #[cfg(test)]
                    if cycle_mask & (1 << cycle_index) != 0 {
                        cycles[cycle_index] = Some(decoded);
                    }
                }
                if cycle_mask & (1 << ResynthAlgorithm::Grain.index()) == 0
                    || cycle_mask & (1 << selected.index()) == 0
                    || ((cycle_mask & (1 << ResynthAlgorithm::Sample.index()) != 0)
                        != effective_root_hz.is_some())
                    || ((cycle_mask & (1 << ResynthAlgorithm::Rich.index()) != 0)
                        != effective_root_hz.is_some())
                {
                    return None;
                }
            } else {
                #[cfg(test)]
                {
                    cycles[ResynthAlgorithm::Grain.index()] = Some([0.0; TABLE_SIZE]);
                    if effective_root_hz.is_some() {
                        cycles[ResynthAlgorithm::Sample.index()] = Some([0.0; TABLE_SIZE]);
                        cycles[ResynthAlgorithm::Rich.index()] = Some([0.0; TABLE_SIZE]);
                    }
                }
            }
            let source_audition = compile_source_audition(&original_bytes).ok()?;
            let visuals = ResynthVisualModel::analyze(&source_audition.samples, sample_rate);
            let artifact = read_artifact(&mut input, controls, source_audition, pack_version)?;
            let artifact_algorithm = artifact.algorithm;
            let artifact_root_hz = artifact.source_root_hz;
            if artifact_algorithm != selected
                || artifact_root_hz
                    .is_some_and(|root| !root.is_finite() || !(20.0..=2_000.0).contains(&root))
                || artifact_root_hz != effective_root_hz
                || (selected == ResynthAlgorithm::Rich && artifact_root_hz.is_none())
            {
                return None;
            }
            let model = Arc::new(ResynthAnalysisModel {
                source: ResynthSourceMaster {
                    file_name,
                    original_bytes,
                    sample_rate,
                    channels,
                    frames,
                    estimated_root_hz,
                    pitch_confidence,
                },
                root_override_hz,
                #[cfg(test)]
                cycles,
                visuals,
                rich_analysis: None,
            });
            let artifact = Arc::new(
                if selected == ResynthAlgorithm::Grain && !pack_has_spectral_grain(pack_version) {
                    compile_rt_artifact_with_cancel(&model, selected, controls, &|| false).ok()?
                } else {
                    artifact
                },
            );
            let artifact_visuals = analyze_sounding_artifact_visuals(&artifact);
            output.push((
                index,
                ResynthSlotDocument {
                    revision: 0,
                    selected,
                    controls,
                    model,
                    artifact,
                    artifact_visuals,
                    artifact_generation: 0,
                },
            ));
        }
        (input.remaining() == 0).then_some(output)
    }

    fn documents_persistently_equal(
        index: usize,
        current: &ResynthSlotDocument,
        incoming: &ResynthSlotDocument,
    ) -> bool {
        let mut current_bytes = Vec::new();
        let mut incoming_bytes = Vec::new();
        Self::encode_document(&mut current_bytes, index, current, PACK_VERSION).is_some()
            && Self::encode_document(&mut incoming_bytes, index, incoming, PACK_VERSION).is_some()
            && current_bytes == incoming_bytes
    }

    fn prepare_restore(
        documents: Vec<(usize, ResynthSlotDocument)>,
    ) -> Option<PendingResynthPackCommit> {
        let mut incoming = std::array::from_fn(|_| None);
        for (index, document) in documents {
            if index >= MAX_OSCILLATORS || incoming[index].replace(document).is_some() {
                return None;
            }
        }
        Some(PendingResynthPackCommit {
            incoming,
            accepted_revisions: None,
        })
    }

    fn replace_all(&self, documents: Vec<(usize, ResynthSlotDocument)>) -> bool {
        let Some(transaction) = Self::prepare_restore(documents) else {
            return false;
        };
        let result = {
            let mut pending = self
                .restore
                .pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            // One bounded latest-wins transaction owns the decoded documents.
            // Holding this guard through the first attempt also linearizes a
            // concurrent retry against a newer host restore.
            *pending = Some(transaction);
            Self::try_commit_pending_restore_locked(
                &self.slots,
                &self.restore.publication_epoch,
                &mut pending,
            )
        };
        if result == PendingPackCommitResult::Backpressured {
            self.restore.schedule_retry();
        }
        result == PendingPackCommitResult::Committed
    }

    #[cfg(test)]
    fn try_commit_pending_restore(&self) -> PendingPackCommitResult {
        Self::try_commit_pending_restore_for(self.restore.as_ref(), &self.slots)
    }

    fn try_commit_pending_restore_for(
        restore: &ResynthPackRestoreState,
        slots: &[Arc<ResynthSlotState>],
    ) -> PendingPackCommitResult {
        let mut pending = restore
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Self::try_commit_pending_restore_locked(slots, &restore.publication_epoch, &mut pending)
    }

    fn try_commit_pending_restore_locked(
        slots: &[Arc<ResynthSlotState>],
        publication_epoch: &AtomicU64,
        pending: &mut Option<PendingResynthPackCommit>,
    ) -> PendingPackCommitResult {
        let Some(transaction) = pending.as_mut() else {
            return PendingPackCommitResult::Empty;
        };
        if slots.len() != MAX_OSCILLATORS {
            return PendingPackCommitResult::Backpressured;
        }

        // Lock every document in slot order. Workers use the same per-slot
        // publication gate, so admission, every RT store, and all document
        // replacements are one aggregate off-thread transaction.
        let mut guards = slots
            .iter()
            .map(|slot| {
                slot.document
                    .write()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
            })
            .collect::<Vec<_>>();
        let observed_revisions =
            std::array::from_fn(|index| slots[index].desired_revision.load(Ordering::Acquire));
        let accepted_revisions = match transaction.accepted_revisions {
            Some(accepted) if accepted != observed_revisions => {
                // A per-slot desired-revision writer can only advance while
                // holding its document gate. With every gate held here, this
                // comparison is a stable whole-pack stale decision.
                *pending = None;
                return PendingPackCommitResult::Stale;
            }
            Some(accepted) => accepted,
            None => {
                transaction.accepted_revisions = Some(observed_revisions);
                observed_revisions
            }
        };
        let changed = std::array::from_fn::<_, MAX_OSCILLATORS, _>(|index| {
            let document_changed =
                match (guards[index].as_ref(), transaction.incoming[index].as_ref()) {
                    (None, None) => false,
                    (Some(current), Some(next)) => {
                        !Self::documents_persistently_equal(index, current, next)
                    }
                    _ => true,
                };
            // A host restore to an apparently empty/unchanged slot must still
            // supersede an accepted async per-slot intent. Give that transition
            // a real generation-bearing publication rather than only bumping
            // control revisions.
            document_changed
                || accepted_revisions[index]
                    != slots[index].sounding_revision.load(Ordering::Acquire)
        });
        let changed_mask = changed
            .iter()
            .enumerate()
            .fold(0_u32, |mask, (index, changed)| {
                mask | (u32::from(*changed) << index)
            });
        if changed_mask == 0 {
            *pending = None;
            return PendingPackCommitResult::Committed;
        }

        // Capacity and revision exhaustion are checked for every changed slot
        // before the first pointer publication. Document guards exclude every
        // other producer; RT acknowledgement can only free retired nodes.
        if changed
            .iter()
            .enumerate()
            .any(|(index, changed)| *changed && !slots[index].rt.can_store())
        {
            return PendingPackCommitResult::Backpressured;
        }
        let mut committed_revisions = accepted_revisions;
        for index in 0..MAX_OSCILLATORS {
            if !changed[index] {
                continue;
            }
            let Some(revision) = accepted_revisions[index].checked_add(1) else {
                *pending = None;
                return PendingPackCommitResult::Stale;
            };
            committed_revisions[index] = revision;
            if let Some(document) = transaction.incoming[index].as_mut() {
                document.revision = revision;
            }
        }
        // Finish source-byte cloning before opening the aggregate epoch. Once
        // odd, only document-gated, preflighted stores and fixed assignments run.
        let mut desired_specs: [Option<ResynthDesiredSpec>; MAX_OSCILLATORS] =
            std::array::from_fn(|index| {
                if changed[index] {
                    transaction.incoming[index]
                        .as_ref()
                        .map(ResynthSlotState::desired_spec_for_document)
                } else {
                    None
                }
            });

        let stable_epoch = publication_epoch.load(Ordering::Acquire);
        let Some(closed_epoch) = stable_epoch.checked_add(2) else {
            return PendingPackCommitResult::Backpressured;
        };
        if stable_epoch & 1 != 0
            || publication_epoch
                .compare_exchange(
                    stable_epoch,
                    stable_epoch + 1,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_err()
        {
            return PendingPackCommitResult::Backpressured;
        }

        let mut published_generations = [0_u64; MAX_OSCILLATORS];
        for index in 0..MAX_OSCILLATORS {
            if !changed[index] {
                continue;
            }
            let revision = committed_revisions[index];
            let artifact = transaction.incoming[index]
                .as_ref()
                .map(|document| Arc::clone(&document.artifact));
            let Some(generation) = slots[index].rt.store(revision, artifact) else {
                // All producers require the held document gate, so the complete
                // preflight above makes failure an invariant violation.
                unreachable!("RESYNTH aggregate store violated its preflight");
            };
            published_generations[index] = generation;
        }

        for index in 0..MAX_OSCILLATORS {
            if !changed[index] {
                continue;
            }
            let revision = committed_revisions[index];
            slots[index]
                .desired_revision
                .store(revision, Ordering::Release);
            *slots[index]
                .pending_commit
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
            *slots[index]
                .pending_build
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
            if let Some(document) = transaction.incoming[index].as_mut() {
                document.artifact_generation = published_generations[index];
            }
            *guards[index] = transaction.incoming[index].take();
            if let Some(document) = guards[index].as_ref() {
                slots[index].store_live_controls(document.controls);
            } else {
                slots[index].live_sequence.store(0, Ordering::Release);
            }
            *slots[index]
                .desired_spec
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = desired_specs[index].take();
            // Audio acceptance, not off-thread publication, completes READY.
            slots[index].build_status.set_progress(99);
            slots[index].reset_source_audition();
        }
        publication_epoch.store(closed_epoch, Ordering::Release);

        *pending = None;
        PendingPackCommitResult::Committed
    }
}

impl Default for ResynthAssetPackState {
    fn default() -> Self {
        Self::new()
    }
}

impl PersistField for ResynthAssetPackState {
    fn persist_write(&self, buf: &mut Vec<u8>) {
        if let Some(encoded) = self.encode() {
            buf.extend_from_slice(&encoded);
        }
    }

    fn persist_read(&self, cursor: &mut StateCursor) {
        let remaining = cursor.remaining();
        if remaining == 0 {
            self.clear();
            return;
        }
        if remaining > MAX_RESYNTH_PACK_BYTES {
            return;
        }
        let Some(data) = cursor.read_bytes(remaining) else {
            return;
        };
        // Host restore reconstructs decoded PCM plus private mip/visual
        // derivatives; serialize that bounded scratch with import/rebuild work.
        let _work = acquire_resynth_analysis_work();
        let Some(documents) = Self::decode(data) else {
            return;
        };
        let _ = self.replace_all(documents);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{Arc, atomic::AtomicBool},
        thread,
    };

    fn tone(frequency: f32, seconds: f32) -> Vec<u8> {
        let frames = (48_000.0 * seconds) as usize;
        crate::wav_test::wav_i16(
            1,
            48_000,
            (0..frames).map(|index| {
                let sample = (std::f32::consts::TAU * frequency * index as f32 / 48_000.0).sin();
                (sample * 24_000.0) as i16
            }),
        )
    }

    fn install(
        state: &ResynthAssetPackState,
        slot: usize,
        bytes: Vec<u8>,
        algorithm: ResynthAlgorithm,
    ) -> u64 {
        let controls = ResynthControls::default();
        let model =
            crate::oscillators::analyze_wav("source.wav", bytes, controls).expect("analyze");
        state
            .slot(slot)
            .expect("slot")
            .replace(model, algorithm, controls)
            .expect("replace")
    }

    fn acknowledge_live(slot: &ResynthSlotState, seen: u64, live_generations: [u64; 2]) {
        let accepted_generation = if live_generations[1] != 0 {
            live_generations[1]
        } else {
            live_generations[0]
        };
        slot.acknowledge_rt(
            seen,
            ResynthRtPlanAck {
                live_generations,
                accepted: ResynthPublicationIdentity {
                    generation: accepted_generation,
                    revision: slot.desired_revision.load(Ordering::Acquire),
                },
            },
        );
    }

    fn sample_receipt(artifact: &ResynthRtArtifact) -> [usize; 4] {
        let ProductionResynthArtifact::Sample(sample) = &artifact.data else {
            panic!("Sample artifact");
        };
        [
            sample.source_start_frames(),
            sample.source_span_frames(),
            sample.source_total_frames(),
            sample.crossfade_frames(),
        ]
    }

    #[test]
    fn history_receipt_reuses_one_immutable_pack_for_unchanged_state() {
        let state = ResynthAssetPackState::new();
        let generation = install(&state, 0, tone(220.0, 0.08), ResynthAlgorithm::Grain);
        acknowledge_live(state.slot(0).expect("slot"), generation, [generation, 0]);
        let first = state.history_receipt(None).expect("accepted receipt");
        let second = state.history_receipt(Some(&first)).expect("reused receipt");
        assert_eq!(first.allocation_id(), second.allocation_id());
        assert!(state.matches_history(&first));
    }

    #[test]
    fn busy_history_restore_is_all_or_none_and_retryable() {
        let state = ResynthAssetPackState::new();
        let source_a = tone(220.0, 0.08);
        let generation_a = install(&state, 0, source_a.clone(), ResynthAlgorithm::Grain);
        acknowledge_live(
            state.slot(0).expect("slot"),
            generation_a,
            [generation_a, 0],
        );
        let receipt_a = state.history_receipt(None).expect("accepted receipt");
        install(&state, 0, tone(330.0, 0.08), ResynthAlgorithm::Grain);
        let current_generation = install(&state, 0, tone(440.0, 0.08), ResynthAlgorithm::Grain);
        let slot = state.slot(0).expect("slot");
        let before_revision = slot.desired_revision.load(Ordering::Acquire);
        let before_source = slot.source_export_snapshot().expect("latest source");
        // Model audio acceptance without changing the deliberately saturated
        // retirement acknowledgement used to exercise the Busy preflight.
        slot.sounding_revision
            .store(before_revision, Ordering::Release);

        assert_eq!(
            state.try_restore_history(&receipt_a),
            ResynthHistoryRestore::Busy
        );
        assert_eq!(slot.published_rt_generation(), current_generation);
        assert_eq!(
            slot.desired_revision.load(Ordering::Acquire),
            before_revision
        );
        assert_eq!(slot.source_export_snapshot(), Some(before_source));

        acknowledge_live(slot, current_generation, [current_generation, 0]);
        assert_eq!(
            state.try_restore_history(&receipt_a),
            ResynthHistoryRestore::Committed
        );
        assert_eq!(
            slot.source_export_snapshot()
                .expect("restored source")
                .original_bytes,
            source_a
        );
        assert!(state.matches_history(&receipt_a));
    }

    #[test]
    fn source_master_and_dynamic_artifact_round_trip_byte_exact() {
        let source = tone(220.0, 0.4);
        let state = ResynthAssetPackState::new();
        install(&state, 0, source.clone(), ResynthAlgorithm::Sample);
        let original = state.slot(0).expect("slot").snapshot().expect("document");
        let original_receipt = sample_receipt(original.artifact.as_ref());
        assert!(original_receipt[1] > 0);
        assert!(original_receipt[2] >= original_receipt[1]);
        let encoded = state.encode().expect("encode");
        let decoded = ResynthAssetPackState::decode(&encoded).expect("decode");
        let recalled = ResynthAssetPackState::new();
        assert!(recalled.replace_all(decoded));
        assert_eq!(recalled.encode().expect("re-encode"), encoded);
        let snapshot = recalled
            .slot(0)
            .expect("slot")
            .snapshot()
            .expect("document");
        assert_eq!(snapshot.model.source.original_bytes, source);
        assert_eq!(sample_receipt(snapshot.artifact.as_ref()), original_receipt);
        let ProductionResynthArtifact::Sample(sample) = &snapshot.artifact.data else {
            panic!("Sample artifact");
        };
        assert!(
            sample
                .samples
                .iter()
                .all(|value| value.is_finite() && value.abs() <= 16.0)
        );
    }

    #[test]
    fn algorithm_visual_snapshot_tracks_committed_generation_and_revision() {
        let state = ResynthAssetPackState::new();
        let first_generation = install(&state, 0, tone(220.0, 0.2), ResynthAlgorithm::Sample);
        let slot = state.slot(0).expect("slot");
        let first = slot.algorithm_visual_snapshot().expect("sample visual");
        assert_eq!(first.algorithm, ResynthAlgorithm::Sample);
        assert_eq!(first.generation, first_generation);
        assert_eq!(
            first.revision,
            slot.desired_revision.load(Ordering::Acquire)
        );
        assert_ne!(
            first.revision,
            slot.sounding_revision.load(Ordering::Acquire),
            "publication must remain pending until the playback plan accepts it",
        );

        let pending_revision = {
            let _gate = slot
                .document
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            slot.desired_revision
                .fetch_add(1, Ordering::AcqRel)
                .wrapping_add(1)
        };
        let completed = completed_build(pending_revision, 330.0);
        let pending_cache = Arc::clone(&completed.artifact_visuals);
        *slot
            .pending_commit
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            Some(PendingResynthCommit::Artifact(completed));
        let PendingCommitResult::Committed(pending_generation) = slot.try_commit_pending() else {
            panic!("pending artifact should commit");
        };
        let pending = slot.algorithm_visual_snapshot().expect("pending visual");
        assert_eq!(pending.algorithm, ResynthAlgorithm::Grain);
        assert_eq!(pending.generation, pending_generation);
        assert_eq!(pending.revision, pending_revision);
        assert!(Arc::ptr_eq(&pending.cache, &pending_cache));

        let second_generation = slot
            .select_algorithm(ResynthAlgorithm::Rich)
            .expect("select Rich")
            .expect("source present");
        let second = slot.algorithm_visual_snapshot().expect("Rich visual");
        assert_eq!(second.algorithm, ResynthAlgorithm::Rich);
        assert_eq!(second.generation, second_generation);
        assert_eq!(
            second.revision,
            slot.desired_revision.load(Ordering::Acquire)
        );
        assert!(!Arc::ptr_eq(&pending.cache, &second.cache));

        let decoded =
            ResynthAssetPackState::decode(&state.encode().expect("encode")).expect("decode");
        let recalled = ResynthAssetPackState::new();
        assert!(recalled.replace_all(decoded));
        let recalled_slot = recalled.slot(0).expect("recalled slot");
        let recalled_visual = recalled_slot
            .algorithm_visual_snapshot()
            .expect("recalled visual");
        assert_eq!(
            recalled_visual.generation,
            recalled_slot.published_rt_generation()
        );
        assert_eq!(
            recalled_visual.revision,
            recalled_slot.desired_revision.load(Ordering::Acquire)
        );
    }

    #[test]
    fn legacy_v3_sample_receipt_remains_explicitly_unavailable() {
        let state = ResynthAssetPackState::new();
        install(&state, 0, tone(220.0, 0.2), ResynthAlgorithm::Sample);
        let legacy = state
            .encode_version(LEGACY_PACK_VERSION)
            .expect("encode v3");
        assert_eq!(
            u16::from_le_bytes(
                legacy[MAGIC.len()..MAGIC.len() + 2]
                    .try_into()
                    .expect("version bytes")
            ),
            LEGACY_PACK_VERSION
        );
        let decoded = ResynthAssetPackState::decode(&legacy).expect("decode v3");
        assert_eq!(sample_receipt(decoded[0].1.artifact.as_ref()), [0; 4]);

        // Re-saving a legacy receipt writes the v4 all-zero unavailable
        // sentinel, which remains decodable rather than fabricating a region.
        let recalled = ResynthAssetPackState::new();
        assert!(recalled.replace_all(decoded));
        let upgraded = recalled.encode().expect("upgrade to v4");
        let upgraded = ResynthAssetPackState::decode(&upgraded).expect("decode upgraded v4");
        assert_eq!(sample_receipt(upgraded[0].1.artifact.as_ref()), [0; 4]);
    }

    #[test]
    fn v4_sample_receipt_rejects_out_of_bounds_source_region() {
        let state = ResynthAssetPackState::new();
        install(&state, 0, tone(220.0, 0.2), ResynthAlgorithm::Sample);
        let document = state.slot(0).expect("slot").snapshot().expect("document");
        let mut encoded_artifact = Vec::new();
        write_artifact(
            &mut encoded_artifact,
            document.artifact.as_ref(),
            PACK_VERSION,
        )
        .expect("encode artifact");

        // Algorithm + optional root + audition gain + sample rate + root +
        // payload length precede the four receipt u32s.
        let receipt_start = 1 + (1 + 4) + 4 + 4 + 4 + 4;
        encoded_artifact[receipt_start..receipt_start + 4].copy_from_slice(&2_u32.to_le_bytes());
        encoded_artifact[receipt_start + 4..receipt_start + 8]
            .copy_from_slice(&2_u32.to_le_bytes());
        encoded_artifact[receipt_start + 8..receipt_start + 12]
            .copy_from_slice(&3_u32.to_le_bytes());
        let mut reader = Reader::new(&encoded_artifact);
        assert!(
            read_artifact(
                &mut reader,
                ResynthControls::default(),
                Box::new(SourceAuditionArtifact::silence()),
                PACK_VERSION,
            )
            .is_none()
        );
    }

    #[test]
    fn artifact_reader_accepts_sample_source_rate_without_grain_decimation() {
        let state = ResynthAssetPackState::new();
        // The grain path must retain the source rate whenever the decoded
        // source fits inside GRAIN_MAX_SOURCE_FRAMES; `Sample` remains a
        // decoder-only legacy value that compiles to a SampleLoopArtifact.
        install(&state, 0, tone(220.0, 3.0), ResynthAlgorithm::Grain);
        let decoded = ResynthAssetPackState::decode(&state.encode().expect("encode"))
            .expect("decode decimated artifact");
        let ProductionResynthArtifact::Grain(grain) = &decoded[0].1.artifact.data else {
            panic!("Grain artifact");
        };
        assert!(grain.source_sample_rate.is_finite());
        assert!(grain.source_sample_rate >= 20_000.0);
    }

    #[test]
    fn artifact_reader_rejects_mismatched_inner_rate_and_root() {
        let state = ResynthAssetPackState::new();
        install(&state, 0, tone(220.0, 0.2), ResynthAlgorithm::Sample);
        let document = state.slot(0).expect("slot").snapshot().expect("document");
        let mut encoded = Vec::new();
        write_artifact(&mut encoded, document.artifact.as_ref(), PACK_VERSION)
            .expect("encode artifact");

        // Algorithm + optional outer root + audition gain precede the inner
        // Sample rate; its redundant root follows immediately.
        let inner_rate = 1 + (1 + 4) + 4;
        let mut bad_rate = encoded.clone();
        bad_rate[inner_rate..inner_rate + 4].copy_from_slice(&f32::MAX.to_bits().to_le_bytes());
        assert!(
            read_artifact(
                &mut Reader::new(&bad_rate),
                ResynthControls::default(),
                Box::new(SourceAuditionArtifact::silence()),
                PACK_VERSION,
            )
            .is_none()
        );

        let mut bad_root = encoded;
        bad_root[inner_rate + 4..inner_rate + 8]
            .copy_from_slice(&440.0_f32.to_bits().to_le_bytes());
        assert!(
            read_artifact(
                &mut Reader::new(&bad_root),
                ResynthControls::default(),
                Box::new(SourceAuditionArtifact::silence()),
                PACK_VERSION,
            )
            .is_none()
        );
    }

    #[test]
    fn unknown_pitch_grain_round_trip_does_not_invent_a_root() {
        let mut bytes = tone(220.0, 0.01);
        // A very short non-silent clip is below the pitch detector's stable window.
        bytes.truncate(bytes.len());
        let controls = ResynthControls::default();
        let mut model =
            crate::oscillators::analyze_wav("short.wav", bytes, controls).expect("analyze");
        model.source.estimated_root_hz = None;
        model.root_override_hz = None;
        model.cycles[ResynthAlgorithm::Sample.index()] = None;
        model.cycles[ResynthAlgorithm::Rich.index()] = None;
        let state = ResynthAssetPackState::new();
        state
            .slot(0)
            .expect("slot")
            .replace(model, ResynthAlgorithm::Grain, controls)
            .expect("grain");
        let encoded = state.encode().expect("encode");
        let decoded = ResynthAssetPackState::decode(&encoded).expect("decode");
        assert!(decoded[0].1.model.source.estimated_root_hz.is_none());
        assert!(decoded[0].1.artifact.source_root_hz.is_none());
    }

    #[test]
    fn aggregate_source_budget_rejects_a_second_individually_valid_source() {
        let state = ResynthAssetPackState::new();
        let controls = ResynthControls::default();
        let bytes = tone(220.0, 0.1);
        let model = crate::oscillators::analyze_wav("one.wav", bytes, controls).expect("analyze");
        state
            .slot(0)
            .expect("slot")
            .replace(model, ResynthAlgorithm::Sample, controls)
            .expect("replace");
        assert!(!state.can_replace_source(1, MAX_AGGREGATE_SOURCE_BYTES));
    }

    #[test]
    fn incoming_name_reservation_uses_legal_cap_at_pack_boundary() {
        let legacy_reservation = worst_resynth_entry_bytes(0, 512).expect("512-byte name");
        let capped_reservation =
            worst_resynth_entry_bytes(0, crate::oscillators::MAX_RESYNTH_SOURCE_NAME_BYTES)
                .expect("capped name");
        assert_eq!(
            capped_reservation - legacy_reservation,
            crate::oscillators::MAX_RESYNTH_SOURCE_NAME_BYTES - 512
        );

        let boundary = MAX_RESYNTH_PACK_BYTES - legacy_reservation;
        assert_eq!(boundary + legacy_reservation, MAX_RESYNTH_PACK_BYTES);
        assert!(boundary + capped_reservation > MAX_RESYNTH_PACK_BYTES);
    }

    #[test]
    fn manually_constructed_overlong_source_name_cannot_replace_persisted_state() {
        let state = ResynthAssetPackState::new();
        install(&state, 0, tone(220.0, 0.08), ResynthAlgorithm::Sample);
        let before = state.encode().expect("valid state");
        let controls = ResynthControls::default();
        let mut model = crate::oscillators::analyze_wav("valid.wav", tone(330.0, 0.08), controls)
            .expect("analyze");
        model.source.file_name = "x".repeat(crate::oscillators::MAX_RESYNTH_SOURCE_NAME_BYTES + 1);
        let result =
            state
                .slot(0)
                .expect("slot")
                .replace(model, ResynthAlgorithm::Sample, controls);
        assert!(matches!(
            result,
            Err(crate::oscillators::ResynthImportError::SourceNameTooLong { .. })
        ));
        assert_eq!(state.encode().expect("state retained"), before);
    }

    #[test]
    fn corrupt_or_truncated_pack_leaves_live_state_untouched() {
        let state = ResynthAssetPackState::new();
        install(&state, 0, tone(220.0, 0.2), ResynthAlgorithm::Sample);
        let original = state.encode().expect("encode");
        let mut corrupt = original.clone();
        let last = corrupt.len() - 1;
        corrupt[last] ^= 0x55;
        assert!(ResynthAssetPackState::decode(&corrupt).is_none());
        assert!(ResynthAssetPackState::decode(&original[..original.len() - 1]).is_none());
        assert!(state.slot(0).expect("slot").has_source());
    }

    #[test]
    fn sounding_revision_advances_only_for_the_plan_to_identity() {
        let state = ResynthAssetPackState::new();
        let generation = install(&state, 0, tone(220.0, 0.08), ResynthAlgorithm::Sample);
        let slot = state.slot(0).expect("slot");
        let view = slot.try_rt_view_after(0).expect("view");
        let desired = slot.desired_revision.load(Ordering::Acquire);
        assert_eq!(slot.sounding_revision.load(Ordering::Acquire), 0);

        slot.acknowledge_rt(
            generation,
            ResynthRtPlanAck {
                live_generations: [generation, 0],
                accepted: ResynthPublicationIdentity::NONE,
            },
        );
        assert_eq!(slot.sounding_revision.load(Ordering::Acquire), 0);
        slot.acknowledge_rt(
            generation,
            ResynthRtPlanAck {
                live_generations: [generation, 0],
                accepted: view.publication_identity(),
            },
        );
        assert_eq!(slot.sounding_revision.load(Ordering::Acquire), desired);
        assert_eq!(
            slot.source_summary().expect("summary").progress_percent,
            100
        );
    }

    #[test]
    fn clear_publishes_generation_bearing_silence() {
        let state = ResynthAssetPackState::new();
        let first = install(&state, 0, tone(220.0, 0.2), ResynthAlgorithm::Sample);
        let slot = state.slot(0).expect("slot");
        let first_view = slot.try_rt_view_after(0).expect("first view");
        assert_eq!(first_view.generation(), first);
        acknowledge_live(slot, first, [first, 0]);
        slot.clear();
        assert_ne!(
            slot.sounding_revision.load(Ordering::Acquire),
            slot.desired_revision.load(Ordering::Acquire),
            "generation-bearing silence is not sounding before plan acceptance",
        );
        let silence = slot.try_rt_view_after(first).expect("silence view");
        assert!(silence.generation() > first);
        // SAFETY: the view is live until the acknowledgement below.
        assert!(unsafe { silence.artifact() }.is_none());
        acknowledge_live(slot, silence.generation(), [silence.generation(), 0]);
        assert_eq!(
            slot.sounding_revision.load(Ordering::Acquire),
            slot.desired_revision.load(Ordering::Acquire),
        );
        slot.rt.collect();
    }

    #[test]
    fn publication_identities_never_wrap_or_reuse_zero() {
        let publication = AtomicResynthArtifact::new();
        publication
            .owners
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .next_generation = u64::MAX;
        assert!(!publication.can_store());
        assert!(publication.store(1, None).is_none());
        assert!(publication.store(0, None).is_none());

        let slot = ResynthSlotState::new();
        slot.desired_revision.store(u64::MAX, Ordering::Release);
        assert_eq!(slot.next_desired_revision(), None);
        assert_eq!(slot.desired_revision.load(Ordering::Acquire), u64::MAX);
    }

    #[test]
    fn pointer_publication_is_coherent_under_concurrent_revisions() {
        let publication = Arc::new(AtomicResynthArtifact::new());
        let running = Arc::new(AtomicBool::new(true));
        let writer_publication = Arc::clone(&publication);
        let writer_running = Arc::clone(&running);
        let writer = thread::spawn(move || {
            for revision in 1..=200_u64 {
                let value = revision as f32 / 200.0;
                let artifact = Arc::new(ResynthRtArtifact {
                    algorithm: ResynthAlgorithm::Grain,
                    source_root_hz: None,
                    data: ProductionResynthArtifact::Grain(Box::new(
                        GrainSourceArtifact::from_persisted(
                            48_000.0,
                            None,
                            ResynthControls::default(),
                            vec![value; 64].into_boxed_slice(),
                            Vec::new().into_boxed_slice(),
                        ),
                    )),
                    source_audition: Box::new(SourceAuditionArtifact::silence()),
                    source_audition_gain: 1.0,
                });
                loop {
                    if writer_publication
                        .store(revision, Some(Arc::clone(&artifact)))
                        .is_some()
                    {
                        break;
                    }
                    writer_publication.collect();
                    thread::yield_now();
                }
            }
            writer_running.store(false, Ordering::Release);
        });
        let mut observed = 0;
        while running.load(Ordering::Acquire) {
            if let Some(view) = publication.try_view_after(observed) {
                observed = view.generation();
                // SAFETY: acknowledgement follows this immediate immutable read.
                let artifact = unsafe { view.artifact() }.expect("artifact");
                let ProductionResynthArtifact::Grain(grain) = &artifact.data else {
                    panic!("torn algorithm/data");
                };
                let first = grain.samples[0].to_bits();
                assert!(grain.samples.iter().all(|sample| sample.to_bits() == first));
                publication.acknowledge(
                    observed,
                    ResynthRtPlanAck {
                        live_generations: [observed, 0],
                        accepted: view.publication_identity(),
                    },
                );
            }
        }
        writer.join().expect("writer");
    }
    #[test]
    fn validly_hashed_extreme_artifact_sample_is_rejected() {
        let state = ResynthAssetPackState::new();
        install(&state, 0, tone(220.0, 0.2), ResynthAlgorithm::Sample);
        let snapshot = state.slot(0).expect("slot").snapshot().expect("document");
        let ProductionResynthArtifact::Sample(sample) = &snapshot.artifact.data else {
            panic!("Sample artifact");
        };
        let needle = sample.samples[sample.samples.len() / 2]
            .to_bits()
            .to_le_bytes();
        let mut encoded = state.encode().expect("encode");
        let payload_start = MAGIC.len() + 2 + 4 + HASH_BYTES;
        let relative = encoded[payload_start..]
            .windows(4)
            .rposition(|window| window == needle)
            .expect("artifact sample");
        let offset = payload_start + relative;
        encoded[offset..offset + 4].copy_from_slice(&f32::MAX.to_bits().to_le_bytes());
        let hash = blake3::hash(&encoded[payload_start..]);
        let hash_start = MAGIC.len() + 2 + 4;
        encoded[hash_start..hash_start + HASH_BYTES].copy_from_slice(hash.as_bytes());
        assert!(ResynthAssetPackState::decode(&encoded).is_none());
    }

    #[test]
    fn multiple_slots_round_trip_as_one_transaction() {
        let state = ResynthAssetPackState::new();
        install(&state, 0, tone(220.0, 0.15), ResynthAlgorithm::Sample);
        install(&state, 7, tone(330.0, 0.15), ResynthAlgorithm::Rich);
        let encoded = state.encode().expect("aggregate encode");
        let decoded = ResynthAssetPackState::decode(&encoded).expect("aggregate decode");
        assert_eq!(decoded.len(), 2);
        let recalled = ResynthAssetPackState::new();
        let untouched_revision = recalled
            .slot(1)
            .expect("untouched slot")
            .desired_revision
            .load(Ordering::Acquire);
        assert!(recalled.replace_all(decoded));
        assert!(recalled.slot(0).expect("slot0").has_source());
        assert!(recalled.slot(7).expect("slot7").has_source());
        assert_eq!(recalled.encode().expect("re-encode"), encoded);
        let untouched = recalled.slot(1).expect("untouched slot");
        assert_eq!(
            untouched.desired_revision.load(Ordering::Acquire),
            untouched_revision,
            "no publication must mean no accepted revision"
        );
        assert_eq!(
            untouched.sounding_revision.load(Ordering::Acquire),
            untouched_revision
        );
        assert_eq!(untouched.published_rt_generation(), 0);
    }

    #[test]
    fn identical_accepted_restore_is_a_true_pack_no_op() {
        let state = ResynthAssetPackState::new();
        let generation = install(&state, 0, tone(220.0, 0.08), ResynthAlgorithm::Sample);
        let slot = state.slot(0).expect("slot 0");
        acknowledge_live(slot, generation, [generation, 0]);
        let revision = slot.desired_revision.load(Ordering::Acquire);
        let epoch = state.restore.publication_epoch.load(Ordering::Acquire);
        let encoded = state.encode().expect("encode");
        let decoded = ResynthAssetPackState::decode(&encoded).expect("decode");

        assert!(state.replace_all(decoded));
        assert_eq!(slot.published_rt_generation(), generation);
        assert_eq!(slot.desired_revision.load(Ordering::Acquire), revision);
        assert_eq!(slot.sounding_revision.load(Ordering::Acquire), revision);
        assert_eq!(
            state.restore.publication_epoch.load(Ordering::Acquire),
            epoch
        );
    }

    #[test]
    fn aggregate_restore_yields_one_even_epoch_and_exact_fixed_rt_set() {
        let incoming = ResynthAssetPackState::new();
        install(&incoming, 0, tone(330.0, 0.08), ResynthAlgorithm::Rich);
        install(&incoming, 7, tone(550.0, 0.08), ResynthAlgorithm::Sample);
        let decoded =
            ResynthAssetPackState::decode(&incoming.encode().expect("encode")).expect("decode");

        let state = ResynthAssetPackState::new();
        let old_0 = install(&state, 0, tone(110.0, 0.08), ResynthAlgorithm::Sample);
        let old_7 = install(&state, 7, tone(220.0, 0.08), ResynthAlgorithm::Sample);
        let observed = std::array::from_fn(|index| {
            state
                .slot(index)
                .map_or(0, ResynthSlotState::published_rt_generation)
        });
        assert_eq!(observed[0], old_0);
        assert_eq!(observed[7], old_7);

        assert!(state.replace_all(decoded));
        assert_eq!(state.restore.publication_epoch.load(Ordering::Acquire), 2);
        let update = state
            .try_rt_update_after(&observed)
            .expect("one coherent changed set");
        assert_eq!(update.changed_mask, (1_u32 << 0) | (1_u32 << 7));
        for index in [0, 7] {
            let document = state
                .slot(index)
                .expect("slot")
                .snapshot()
                .expect("document");
            let identity = update.views[index].publication_identity();
            assert_eq!(identity.generation, document.artifact_generation);
            assert_eq!(identity.revision, document.revision);
        }

        state.restore.publication_epoch.store(3, Ordering::Release);
        assert!(state.try_rt_update_after(&[0; MAX_OSCILLATORS]).is_none());
        state.restore.publication_epoch.store(2, Ordering::Release);
    }

    #[test]
    fn persist_read_retains_aggregate_restore_until_all_slots_can_publish() {
        let incoming = ResynthAssetPackState::new();
        let incoming_slot_0 = tone(220.0, 0.08);
        let incoming_slot_7 = tone(330.0, 0.08);
        install(
            &incoming,
            0,
            incoming_slot_0.clone(),
            ResynthAlgorithm::Sample,
        );
        install(
            &incoming,
            7,
            incoming_slot_7.clone(),
            ResynthAlgorithm::Rich,
        );
        let encoded = incoming.encode().expect("aggregate host state");

        let restored = ResynthAssetPackState::new();
        install(&restored, 0, tone(440.0, 0.08), ResynthAlgorithm::Sample);
        install(&restored, 0, tone(550.0, 0.08), ResynthAlgorithm::Sample);
        let blocked_generation = install(&restored, 0, tone(660.0, 0.08), ResynthAlgorithm::Sample);
        let other_generation = install(&restored, 7, tone(770.0, 0.08), ResynthAlgorithm::Sample);
        let old_slot_0 = restored
            .slot(0)
            .expect("slot 0")
            .source_export_snapshot()
            .expect("old slot 0")
            .original_bytes;
        let old_slot_7 = restored
            .slot(7)
            .expect("slot 7")
            .source_export_snapshot()
            .expect("old slot 7")
            .original_bytes;

        // Keep the regression deterministic: retain the transaction without
        // racing the 20 ms off-thread retry, then exercise exactly one retry.
        restored
            .restore
            .retry_running
            .store(true, Ordering::Release);
        PersistField::persist_read(&restored, &mut StateCursor::new(&encoded));

        // One blocked slot must prevent every publication and document change.
        assert_eq!(
            restored
                .slot(0)
                .expect("slot 0")
                .source_export_snapshot()
                .expect("old slot 0 retained")
                .original_bytes,
            old_slot_0
        );
        assert_eq!(
            restored
                .slot(7)
                .expect("slot 7")
                .source_export_snapshot()
                .expect("old slot 7 retained")
                .original_bytes,
            old_slot_7
        );
        assert_eq!(
            restored.slot(0).expect("slot 0").published_rt_generation(),
            blocked_generation
        );
        assert_eq!(
            restored.slot(7).expect("slot 7").published_rt_generation(),
            other_generation
        );
        assert!(
            restored
                .restore
                .pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_some(),
            "decoded aggregate transaction must remain owned by the pack"
        );

        acknowledge_live(
            restored.slot(0).expect("slot 0"),
            blocked_generation,
            [blocked_generation, 0],
        );
        assert_eq!(
            restored.try_commit_pending_restore(),
            PendingPackCommitResult::Committed
        );
        restored
            .restore
            .retry_running
            .store(false, Ordering::Release);

        assert_eq!(
            restored
                .slot(0)
                .expect("slot 0")
                .source_export_snapshot()
                .expect("restored slot 0")
                .original_bytes,
            incoming_slot_0
        );
        assert_eq!(
            restored
                .slot(7)
                .expect("slot 7")
                .source_export_snapshot()
                .expect("restored slot 7")
                .original_bytes,
            incoming_slot_7
        );
        assert!(
            restored
                .restore
                .pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_none()
        );
    }

    #[test]
    fn persist_write_serializes_retained_aggregate_restore_intent() {
        let incoming = ResynthAssetPackState::new();
        install(&incoming, 0, tone(220.0, 0.08), ResynthAlgorithm::Sample);
        install(&incoming, 7, tone(330.0, 0.08), ResynthAlgorithm::Rich);
        let encoded = incoming.encode().expect("aggregate host state");

        let restored = ResynthAssetPackState::new();
        install(&restored, 0, tone(440.0, 0.08), ResynthAlgorithm::Sample);
        install(&restored, 0, tone(550.0, 0.08), ResynthAlgorithm::Sample);
        install(&restored, 0, tone(660.0, 0.08), ResynthAlgorithm::Sample);
        restored
            .restore
            .retry_running
            .store(true, Ordering::Release);
        PersistField::persist_read(&restored, &mut StateCursor::new(&encoded));

        let mut saved = Vec::new();
        PersistField::persist_write(&restored, &mut saved);
        restored
            .restore
            .retry_running
            .store(false, Ordering::Release);
        assert_eq!(saved, encoded, "get-state must preserve pending set-state");
    }

    #[test]
    fn deferred_aggregate_restore_is_stale_after_newer_slot_intent() {
        let incoming = ResynthAssetPackState::new();
        install(&incoming, 0, tone(220.0, 0.08), ResynthAlgorithm::Sample);
        install(&incoming, 7, tone(330.0, 0.08), ResynthAlgorithm::Rich);
        let encoded = incoming.encode().expect("aggregate host state");

        let restored = ResynthAssetPackState::new();
        install(&restored, 0, tone(440.0, 0.08), ResynthAlgorithm::Sample);
        install(&restored, 0, tone(550.0, 0.08), ResynthAlgorithm::Sample);
        let blocked_generation = install(&restored, 0, tone(660.0, 0.08), ResynthAlgorithm::Sample);
        install(&restored, 7, tone(770.0, 0.08), ResynthAlgorithm::Sample);
        let old_slot_0 = restored
            .slot(0)
            .expect("slot 0")
            .source_export_snapshot()
            .expect("old slot 0")
            .original_bytes;

        restored
            .restore
            .retry_running
            .store(true, Ordering::Release);
        PersistField::persist_read(&restored, &mut StateCursor::new(&encoded));
        let newer_slot_7 = tone(880.0, 0.08);
        install(&restored, 7, newer_slot_7.clone(), ResynthAlgorithm::Sample);
        acknowledge_live(
            restored.slot(0).expect("slot 0"),
            blocked_generation,
            [blocked_generation, 0],
        );

        assert_eq!(
            restored.try_commit_pending_restore(),
            PendingPackCommitResult::Stale
        );
        restored
            .restore
            .retry_running
            .store(false, Ordering::Release);
        assert_eq!(
            restored
                .slot(0)
                .expect("slot 0")
                .source_export_snapshot()
                .expect("slot 0 remains old")
                .original_bytes,
            old_slot_0
        );
        assert_eq!(
            restored
                .slot(7)
                .expect("slot 7")
                .source_export_snapshot()
                .expect("newer slot 7")
                .original_bytes,
            newer_slot_7
        );
        assert!(
            restored
                .restore
                .pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_none()
        );
    }

    #[test]
    fn empty_host_restore_waits_for_one_aggregate_clear_commit() {
        let restored = ResynthAssetPackState::new();
        install(&restored, 0, tone(440.0, 0.08), ResynthAlgorithm::Sample);
        install(&restored, 0, tone(550.0, 0.08), ResynthAlgorithm::Sample);
        let blocked_generation = install(&restored, 0, tone(660.0, 0.08), ResynthAlgorithm::Sample);
        let other_generation = install(&restored, 7, tone(770.0, 0.08), ResynthAlgorithm::Rich);
        let slot_0 = restored.slot(0).expect("slot 0");
        slot_0.pending_retry_running.store(true, Ordering::Release);
        restored
            .restore
            .retry_running
            .store(true, Ordering::Release);

        PersistField::persist_read(&restored, &mut StateCursor::new(&[]));

        assert!(slot_0.has_source(), "blocked slot must remain committed");
        assert!(
            restored.slot(7).expect("slot 7").has_source(),
            "unblocked slot must not clear ahead of the pack"
        );
        assert_eq!(slot_0.published_rt_generation(), blocked_generation);
        assert_eq!(
            restored.slot(7).expect("slot 7").published_rt_generation(),
            other_generation
        );
        assert!(
            restored
                .restore
                .pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_some()
        );

        acknowledge_live(slot_0, blocked_generation, [blocked_generation, 0]);
        assert_eq!(
            restored.try_commit_pending_restore(),
            PendingPackCommitResult::Committed
        );
        slot_0.pending_retry_running.store(false, Ordering::Release);
        restored
            .restore
            .retry_running
            .store(false, Ordering::Release);
        assert!(!slot_0.has_source());
        assert!(!restored.slot(7).expect("slot 7").has_source());
    }

    #[test]
    fn source_audition_is_ephemeral_and_never_changes_serialized_state() {
        let state = ResynthAssetPackState::new();
        install(&state, 0, tone(220.0, 0.2), ResynthAlgorithm::Sample);
        let before = state.encode().expect("before");
        let slot = state.slot(0).expect("slot");
        slot.renew_source_audition();
        assert!(slot.consume_source_audition_lease());
        slot.reset_source_audition();
        assert_eq!(state.encode().expect("after"), before);
    }

    #[test]
    fn aggregate_decode_budget_rejects_before_source_allocation() {
        assert_eq!(
            admit_aggregate_source_bytes(MAX_AGGREGATE_SOURCE_BYTES - 1, 1),
            Some(MAX_AGGREGATE_SOURCE_BYTES)
        );
        assert_eq!(
            admit_aggregate_source_bytes(MAX_AGGREGATE_SOURCE_BYTES - 1, 2),
            None
        );
        assert_eq!(admit_aggregate_source_bytes(usize::MAX, 1), None);
    }

    #[test]
    fn telemetry_transport_preserves_one_coherent_fixed_frame() {
        let transport = ResynthTelemetryTransport::new();
        let lane = GrainTelemetryLane {
            active: true,
            position: 0.25,
            progress: 0.5,
            gain: 0.75,
            phase: 0.125,
            pan: -0.25,
            pitch: 3.0,
        };
        let frame = ResynthTelemetryFrame {
            generation: 9,
            from_generation: 8,
            from_revision: 31,
            to_generation: 9,
            to_revision: 32,
            transition_from_gain: 0.8,
            transition_to_gain: 0.6,
            transition_progress: 0.36,
            publish_frame: 17,
            audio_frame: 1_024,
            publish_count: 17,
            phase: 0.3,
            envelope_proxy: 0.8,
            amplitude: 0.8,
            source_mix: 0.2,
            source_target: 1.0,
            rich_zone: 5,
            rich_from_zone: 4,
            rich_to_zone: 5,
            rich_transition_progress: 0.25,
            zone: 5,
            active: true,
            grain_lanes: [lane; RESYNTH_TELEMETRY_GRAIN_LANES],
            ..ResynthTelemetryFrame::default()
        };
        transport.publish(frame);
        let snapshot = transport.snapshot();
        assert!(!snapshot.stale);
        assert_eq!(snapshot.generation, 9);
        assert_eq!((snapshot.from_generation, snapshot.from_revision), (8, 31));
        assert_eq!((snapshot.to_generation, snapshot.to_revision), (9, 32));
        assert_eq!(snapshot.transition_from_gain.to_bits(), 0.8_f32.to_bits());
        assert_eq!(snapshot.transition_to_gain.to_bits(), 0.6_f32.to_bits());
        assert_eq!(snapshot.transition_progress.to_bits(), 0.36_f32.to_bits());
        assert_eq!(snapshot.source_target.to_bits(), 1.0_f32.to_bits());
        assert_eq!((snapshot.rich_from_zone, snapshot.rich_to_zone), (4, 5));
        assert_eq!(
            snapshot.rich_transition_progress.to_bits(),
            0.25_f32.to_bits(),
        );
        assert_eq!(snapshot.publish_frame, 17);
        assert_eq!(snapshot.publish_count, 17);
        assert_eq!(snapshot.audio_frame, 1_024);
        assert_eq!(snapshot.rich_zone, 5);
        assert_eq!(snapshot.zone, 5);
        assert_eq!(snapshot.grain_lanes, [lane; RESYNTH_TELEMETRY_GRAIN_LANES]);
        assert_eq!(
            snapshot.grain_positions,
            [0.25; RESYNTH_TELEMETRY_GRAIN_LANES]
        );
    }

    #[test]
    fn telemetry_transport_seqlock_keeps_frame_identities_coherent() {
        let transport = Arc::new(ResynthTelemetryTransport::new());
        let writer_transport = Arc::clone(&transport);
        let writer = thread::spawn(move || {
            for frame in 1..=10_000_u64 {
                writer_transport.publish(ResynthTelemetryFrame {
                    generation: frame,
                    from_generation: frame.saturating_sub(1),
                    from_revision: frame + 10_000,
                    to_generation: frame,
                    to_revision: frame + 20_000,
                    transition_from_gain: 0.75,
                    transition_to_gain: 0.5,
                    transition_progress: frame as f32 / 10_000.0,
                    publish_frame: frame,
                    audio_frame: frame * 64,
                    publish_count: frame,
                    phase: frame as f32 / 10_000.0,
                    envelope_proxy: 0.5,
                    amplitude: 0.5,
                    source_mix: 0.25,
                    source_target: 1.0,
                    rich_zone: 3,
                    rich_from_zone: 2,
                    rich_to_zone: 3,
                    rich_transition_progress: 0.4,
                    zone: 3,
                    active: true,
                    ..ResynthTelemetryFrame::default()
                });
            }
        });
        for _ in 0..10_000 {
            let frame = transport.snapshot();
            if frame.stale || frame.publish_frame == 0 {
                continue;
            }
            assert_eq!(frame.publish_count, frame.publish_frame);
            assert_eq!(frame.generation, frame.publish_frame);
            assert_eq!(frame.from_generation, frame.publish_frame.saturating_sub(1));
            assert_eq!(frame.from_revision, frame.publish_frame + 10_000);
            assert_eq!(frame.to_generation, frame.publish_frame);
            assert_eq!(frame.to_revision, frame.publish_frame + 20_000);
            assert_eq!(frame.audio_frame, frame.publish_frame * 64);
            assert_eq!((frame.rich_from_zone, frame.rich_to_zone), (2, 3));
            assert_eq!(frame.rich_zone, frame.zone);
            assert!((frame.envelope_proxy - frame.amplitude).abs() <= f32::EPSILON);
        }
        writer.join().expect("telemetry writer");
    }

    #[test]
    fn telemetry_transport_marks_snapshot_stale_after_bounded_retry_budget() {
        let transport = ResynthTelemetryTransport::new();
        transport.sequence.store(1, Ordering::Relaxed);
        let snapshot = transport.snapshot();
        assert!(snapshot.stale);
        assert_eq!(snapshot.publish_frame, 0);
        assert!(!snapshot.active);
    }

    #[test]
    fn telemetry_transport_assigns_monotonic_compatibility_frames() {
        let transport = ResynthTelemetryTransport::new();
        transport.publish(ResynthTelemetryFrame::default());
        let first = transport.snapshot();
        transport.publish(ResynthTelemetryFrame::default());
        let second = transport.snapshot();
        assert!(first.publish_frame > 0);
        assert_eq!(second.publish_frame, first.publish_frame + 1);
        assert_eq!(second.publish_count, second.publish_frame);
    }

    #[test]
    fn telemetry_interest_lease_is_fixed_and_renewed_by_snapshot() {
        let state = ResynthAssetPackState::new();
        let slot = state.slot(0).expect("slot");
        assert!(!slot.consume_telemetry_interest());

        let _ = slot.telemetry_snapshot();
        for _ in 0..RESYNTH_TELEMETRY_INTEREST_CALLBACKS {
            assert!(slot.consume_telemetry_interest());
        }
        assert!(!slot.consume_telemetry_interest());

        let _ = slot.telemetry_snapshot();
        assert!(slot.consume_telemetry_interest());
    }

    #[test]
    fn persistence_compiles_the_requested_revision_when_worker_is_pending() {
        let state = ResynthAssetPackState::new();
        install(&state, 0, tone(220.0, 0.08), ResynthAlgorithm::Sample);
        let slot = state.slot(0).expect("slot");
        let mut desired = slot.committed_desired_spec().expect("desired base");
        desired.selected = ResynthAlgorithm::Rich;
        desired.controls.seed = 0x1234_5678_9abc_def0;
        {
            let _intent = slot
                .document
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let mut desired_spec = slot
                .desired_spec
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            desired.revision = slot.next_desired_revision().expect("revision");
            *desired_spec = Some(desired);
        }
        let encoded = state.encode().expect("encode desired revision");
        let decoded = ResynthAssetPackState::decode(&encoded).expect("decode desired revision");
        let document = &decoded[0].1;
        assert_eq!(document.selected, ResynthAlgorithm::Rich);
        assert_eq!(document.controls.seed, 0x1234_5678_9abc_def0);
        assert!(matches!(
            document.artifact.data,
            ProductionResynthArtifact::Rich(_)
        ));
    }

    fn completed_build(revision: u64, frequency: f32) -> CompletedResynthBuild {
        let controls = ResynthControls::default();
        let model = crate::oscillators::analyze_wav("pending.wav", tone(frequency, 0.08), controls)
            .expect("analyze pending build");
        let artifact = Arc::new(
            compile_rt_artifact_with_cancel(&model, ResynthAlgorithm::Grain, controls, || false)
                .expect("compile pending build"),
        );
        let artifact_visuals = analyze_sounding_artifact_visuals(&artifact);
        CompletedResynthBuild {
            revision,
            selected: ResynthAlgorithm::Grain,
            controls,
            model,
            artifact,
            artifact_visuals,
        }
    }

    #[test]
    fn stale_completion_never_replaces_newer_desired_revision() {
        let state = ResynthAssetPackState::new();
        let first_generation = install(&state, 0, tone(220.0, 0.08), ResynthAlgorithm::Sample);
        let slot = state.slot(0).expect("slot");
        let stale_revision = slot.desired_revision.load(Ordering::Acquire);
        *slot
            .pending_commit
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(
            PendingResynthCommit::Artifact(completed_build(stale_revision, 330.0)),
        );

        // Model a newer intent through the same document gate used by every
        // production revision writer. The stale completion must be rejected
        // before AtomicResynthArtifact::store can advance publication identity.
        {
            let _intent = slot
                .document
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            slot.desired_revision.fetch_add(1, Ordering::AcqRel);
        }

        assert_eq!(slot.try_commit_pending(), PendingCommitResult::Stale);
        assert_eq!(slot.published_rt_generation(), first_generation);
        assert!(
            slot.pending_commit
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_none()
        );
        let view = slot.try_rt_view_after(0).expect("original publication");
        assert_eq!(view.generation(), first_generation);
        // SAFETY: the original node remains current for this immediate read.
        assert!(unsafe { view.artifact() }.is_some());
    }

    #[test]
    fn clear_intent_survives_retirement_backpressure() {
        let state = ResynthAssetPackState::new();
        install(&state, 0, tone(220.0, 0.08), ResynthAlgorithm::Sample);
        install(&state, 0, tone(330.0, 0.08), ResynthAlgorithm::Sample);
        let current_generation = install(&state, 0, tone(440.0, 0.08), ResynthAlgorithm::Sample);
        let slot = state.slot(0).expect("slot");
        assert_eq!(
            slot.rt
                .owners
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .retired
                .len(),
            2
        );

        // Keep this test deterministic: exercise one explicit retry rather
        // than racing the background retry worker's 20 ms cadence.
        slot.pending_retry_running.store(true, Ordering::Release);
        slot.clear();
        let clear_revision = slot.desired_revision.load(Ordering::Acquire);
        assert!(matches!(
            slot.pending_commit
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_ref(),
            Some(PendingResynthCommit::Clear { revision }) if *revision == clear_revision
        ));
        assert_eq!(slot.published_rt_generation(), current_generation);
        assert!(slot.has_source());
        assert!(slot.snapshot().is_none());

        acknowledge_live(slot, current_generation, [current_generation, 0]);
        let committed = slot.try_commit_pending();
        let PendingCommitResult::Committed(silence_generation) = committed else {
            panic!("expected a committed clear retry, got {committed:?}");
        };
        slot.pending_retry_running.store(false, Ordering::Release);

        assert!(silence_generation > current_generation);
        assert_eq!(slot.published_rt_generation(), silence_generation);
        assert!(!slot.has_source());
        assert_eq!(
            slot.sounding_revision.load(Ordering::Acquire),
            clear_revision
        );
        assert!(
            slot.pending_commit
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_none()
        );
        assert!(
            slot.rt
                .owners
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .retired
                .len()
                <= 2
        );
        let silence = slot
            .try_rt_view_after(current_generation)
            .expect("generation-bearing silence");
        // SAFETY: the silence node remains current for this immediate read.
        assert!(unsafe { silence.artifact() }.is_none());
    }
    #[test]
    fn live_grain_controls_do_not_advance_revision() {
        let state = ResynthAssetPackState::new();
        install(&state, 0, tone(220.0, 0.2), ResynthAlgorithm::Grain);
        let slot = state.slot(0).expect("slot");
        let before = slot.desired_revision.load(Ordering::Acquire);
        let mut controls = ResynthControls::default();
        controls.grain_density = 8.0;
        controls.grain_direction = crate::oscillators::GrainDirection::PingPong as u8;
        slot.apply_live_controls(controls);
        assert_eq!(slot.desired_revision.load(Ordering::Acquire), before);
        let live = slot.rt_grain_controls().expect("live");
        assert!((live.grain_density - 8.0).abs() < 1.0e-6);
        assert_eq!(
            live.grain_direction(),
            crate::oscillators::GrainDirection::PingPong
        );
    }

    #[test]
    fn live_controls_are_complete_snapshots_under_concurrent_updates() {
        let slot = Arc::new(ResynthSlotState::new());
        let mut first = ResynthControls::default();
        first.position = 0.11;
        first.grain_density = 11.0;
        first.grain_tune = 0.21;
        first.seed = 11;
        let mut second = ResynthControls::default();
        second.position = 0.89;
        second.grain_density = 889.0;
        second.grain_tune = 0.79;
        second.pitch_mode = crate::oscillators::PitchMode::Spectral;
        second.seed = 89;
        slot.store_live_controls(first);
        let writer_slot = Arc::clone(&slot);
        let writer = std::thread::spawn(move || {
            for index in 0..10_000 {
                writer_slot.store_live_controls(if index & 1 == 0 { first } else { second });
            }
        });
        for _ in 0..10_000 {
            if let Some(observed) = slot.rt_grain_controls() {
                assert!(
                    observed == first || observed == second,
                    "torn snapshot: {observed:?}"
                );
            }
        }
        writer.join().expect("writer");
    }

    #[test]
    fn v4_pack_still_decodes_after_grain_play_fields() {
        let state = ResynthAssetPackState::new();
        install(&state, 0, tone(220.0, 0.2), ResynthAlgorithm::Grain);
        let v4 = state
            .encode_version(SAMPLE_RECEIPT_PACK_VERSION)
            .expect("encode v4");
        let decoded = ResynthAssetPackState::decode(&v4).expect("decode v4");
        assert_eq!(
            decoded[0].1.controls.grain_direction,
            crate::oscillators::GrainDirection::Forward as u8
        );
        assert_eq!(decoded[0].1.controls.grain_reverse, 0.0);
    }

    #[test]
    fn persisted_grain_accepts_fractional_effective_rate_and_low_audition_gain() {
        let controls = ResynthControls::default();
        // Grain sources persist up to GRAIN_MAX_SOURCE_FRAMES without
        // decimation, so the stride derives from that cap.
        let source_frames = crate::oscillators::GRAIN_MAX_SOURCE_FRAMES * 6 + 1;
        let source = vec![0.0_f32; source_frames];
        let source_audition =
            SourceAuditionArtifact::compile(&source, 48_000).expect("source audition");
        let stride = source_frames.div_ceil(crate::oscillators::GRAIN_MAX_SOURCE_FRAMES);
        assert_eq!(stride, 7);
        let effective_rate = 48_000.0 / stride as f32;
        let grain = GrainSourceArtifact::from_persisted(
            effective_rate,
            Some(220.0),
            controls,
            vec![0.0; 32].into_boxed_slice(),
            Vec::new().into_boxed_slice(),
        );
        // Audition gain persists only as the canonical unity value; anything
        // else is rejected as a corrupted or hostile pack.
        let artifact = ResynthRtArtifact {
            algorithm: ResynthAlgorithm::Grain,
            source_root_hz: Some(220.0),
            data: ProductionResynthArtifact::Grain(Box::new(grain)),
            source_audition: Box::new(source_audition),
            source_audition_gain: 1.0,
        };
        let mut encoded = Vec::new();
        write_artifact(&mut encoded, &artifact, PACK_VERSION).expect("encode artifact");
        let decoded_source =
            SourceAuditionArtifact::compile(&source, 48_000).expect("decoded source audition");
        let mut reader = Reader::new(&encoded);
        let decoded = read_artifact(
            &mut reader,
            controls,
            Box::new(decoded_source),
            PACK_VERSION,
        )
        .expect("valid build-produced artifact");
        assert_eq!(reader.remaining(), 0);
        assert_eq!(decoded.source_audition_gain.to_bits(), 1.0_f32.to_bits());
        let mut attenuated_bytes = Vec::new();
        let attenuated = ResynthRtArtifact {
            source_audition_gain: 0.1,
            ..artifact
        };
        write_artifact(&mut attenuated_bytes, &attenuated, PACK_VERSION)
            .expect("encode attenuated artifact");
        let mut attenuated_reader = Reader::new(&attenuated_bytes);
        let decoded_source =
            SourceAuditionArtifact::compile(&source, 48_000).expect("decoded source audition");
        assert!(
            read_artifact(
                &mut attenuated_reader,
                controls,
                Box::new(decoded_source),
                PACK_VERSION,
            )
            .is_none(),
            "non-unity audition gain must be rejected"
        );
        let ProductionResynthArtifact::Grain(decoded_grain) = decoded.data else {
            panic!("expected Grain artifact");
        };
        assert_eq!(
            decoded_grain.source_sample_rate.to_bits(),
            effective_rate.to_bits()
        );
    }

    #[test]
    fn analysis_work_permit_serializes_pcm_expansion() {
        let first = acquire_resynth_analysis_work();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (acquired_tx, acquired_rx) = std::sync::mpsc::channel();
        let waiter = thread::spawn(move || {
            started_tx.send(()).expect("started");
            let _second = acquire_resynth_analysis_work();
            acquired_tx.send(()).expect("acquired");
        });
        started_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("waiter started");
        assert!(
            acquired_rx
                .recv_timeout(std::time::Duration::from_millis(20))
                .is_err(),
            "a second decoded-PCM worker entered the process-wide budget"
        );
        drop(first);
        acquired_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("waiter admitted after release");
        waiter.join().expect("waiter joined");
    }
}
