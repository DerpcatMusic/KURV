use std::sync::atomic::{AtomicU32, AtomicU64, Ordering, fence};

/// Number of fixed grain lanes exposed to the editor telemetry path.
pub const RESYNTH_TELEMETRY_GRAIN_LANES: usize = 8;

/// Number of audio callbacks kept interested by one UI telemetry read.
pub(super) const RESYNTH_TELEMETRY_INTEREST_CALLBACKS: u8 = 64;

/// One immutable-at-snapshot grain-lane projection.
///
/// The scheduler itself remains audio-owned.  This fixed copy is only the
/// bounded monitor representation; it is never used to drive rendering.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GrainTelemetryLane {
    pub active: bool,
    pub position: f32,
    pub progress: f32,
    pub gain: f32,
    pub phase: f32,
    pub pan: f32,
    pub pitch: f32,
}
/// Coherent RESYNTH monitor frame.
///
/// `publish_frame` identifies the monitor publication and `audio_frame` is the
/// corresponding absolute audio position.  They are deliberately distinct
/// from `generation`: an unchanged artifact still produces new monitor frames.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ResynthTelemetrySnapshot {
    /// Compatibility identity for the dynamic payload; always the exact `to`
    /// layer generation for frames emitted by the current renderer.
    pub generation: u64,
    pub from_generation: u64,
    pub from_revision: u64,
    pub to_generation: u64,
    pub to_revision: u64,
    pub transition_from_gain: f32,
    pub transition_to_gain: f32,
    pub transition_progress: f32,
    pub publish_frame: u64,
    pub audio_frame: u64,
    /// Compatibility alias for clients that only need a monotonically
    /// increasing publication count.
    pub publish_count: u64,
    pub phase: f32,
    pub envelope_proxy: f32,
    /// Compatibility alias retained for the first monitor API.
    pub amplitude: f32,
    pub source_mix: f32,
    pub source_target: f32,
    pub rich_zone: u16,
    pub rich_from_zone: u16,
    pub rich_to_zone: u16,
    pub rich_transition_progress: f32,
    /// Compatibility alias retained for the first monitor API.
    pub zone: u16,
    pub active: bool,
    pub grain_lanes: [GrainTelemetryLane; RESYNTH_TELEMETRY_GRAIN_LANES],
    /// Compatibility projections for older visual clients.
    pub grain_positions: [f32; RESYNTH_TELEMETRY_GRAIN_LANES],
    pub grain_progress: [f32; RESYNTH_TELEMETRY_GRAIN_LANES],
    pub grain_gains: [f32; RESYNTH_TELEMETRY_GRAIN_LANES],
    /// Set when a reader exhausted the bounded seqlock retry budget.
    pub stale: bool,
}

/// Short name used by realtime publication code and future visual clients.
pub type ResynthTelemetryFrame = ResynthTelemetrySnapshot;

struct AtomicGrainTelemetryLane {
    active: AtomicU32,
    position: AtomicU32,
    progress: AtomicU32,
    gain: AtomicU32,
    phase: AtomicU32,
    pan: AtomicU32,
    pitch: AtomicU32,
}

impl AtomicGrainTelemetryLane {
    fn new() -> Self {
        Self {
            active: AtomicU32::new(0),
            position: AtomicU32::new(0),
            progress: AtomicU32::new(0),
            gain: AtomicU32::new(0),
            phase: AtomicU32::new(0),
            pan: AtomicU32::new(0),
            pitch: AtomicU32::new(0),
        }
    }

    #[inline]
    fn publish(&self, lane: GrainTelemetryLane) {
        self.active.store(u32::from(lane.active), Ordering::Relaxed);
        self.position
            .store(lane.position.to_bits(), Ordering::Relaxed);
        self.progress
            .store(lane.progress.to_bits(), Ordering::Relaxed);
        self.gain.store(lane.gain.to_bits(), Ordering::Relaxed);
        self.phase.store(lane.phase.to_bits(), Ordering::Relaxed);
        self.pan.store(lane.pan.to_bits(), Ordering::Relaxed);
        self.pitch.store(lane.pitch.to_bits(), Ordering::Relaxed);
    }

    #[inline]
    fn snapshot(&self) -> GrainTelemetryLane {
        GrainTelemetryLane {
            active: self.active.load(Ordering::Relaxed) != 0,
            position: f32::from_bits(self.position.load(Ordering::Relaxed)),
            progress: f32::from_bits(self.progress.load(Ordering::Relaxed)),
            gain: f32::from_bits(self.gain.load(Ordering::Relaxed)),
            phase: f32::from_bits(self.phase.load(Ordering::Relaxed)),
            pan: f32::from_bits(self.pan.load(Ordering::Relaxed)),
            pitch: f32::from_bits(self.pitch.load(Ordering::Relaxed)),
        }
    }
}

/// Fixed-size, single-writer seqlock transport for one RESYNTH slot.
///
/// Every payload member is an atomic bit representation, so a reader never
/// creates a Rust data race while it retries a concurrent audio publication.
/// The audio callback is the sole writer; editor/worker readers use
/// [`Self::snapshot`].  The transport owns no `Arc`, lock, allocation, or
/// dynamic container and `publish` is bounded by eight lane stores.
pub struct ResynthTelemetryTransport {
    pub(super) sequence: AtomicU64,
    frame: AtomicU64,
    audio_frame: AtomicU64,
    generation: AtomicU64,
    from_generation: AtomicU64,
    from_revision: AtomicU64,
    to_generation: AtomicU64,
    to_revision: AtomicU64,
    transition_from_gain: AtomicU32,
    transition_to_gain: AtomicU32,
    transition_progress: AtomicU32,
    phase: AtomicU32,
    envelope_proxy: AtomicU32,
    source_mix: AtomicU32,
    source_target: AtomicU32,
    rich_zone: AtomicU32,
    rich_from_zone: AtomicU32,
    rich_to_zone: AtomicU32,
    rich_transition_progress: AtomicU32,
    active: AtomicU32,
    grain_lanes: [AtomicGrainTelemetryLane; RESYNTH_TELEMETRY_GRAIN_LANES],
}

impl ResynthTelemetryTransport {
    #[must_use]
    pub fn new() -> Self {
        Self {
            sequence: AtomicU64::new(0),
            frame: AtomicU64::new(0),
            audio_frame: AtomicU64::new(0),
            generation: AtomicU64::new(0),
            from_generation: AtomicU64::new(0),
            from_revision: AtomicU64::new(0),
            to_generation: AtomicU64::new(0),
            to_revision: AtomicU64::new(0),
            transition_from_gain: AtomicU32::new(0),
            transition_to_gain: AtomicU32::new(0),
            transition_progress: AtomicU32::new(0),
            phase: AtomicU32::new(0),
            envelope_proxy: AtomicU32::new(0),
            source_mix: AtomicU32::new(0),
            source_target: AtomicU32::new(0),
            rich_zone: AtomicU32::new(0),
            rich_from_zone: AtomicU32::new(0),
            rich_to_zone: AtomicU32::new(0),
            rich_transition_progress: AtomicU32::new(0),
            active: AtomicU32::new(0),
            grain_lanes: std::array::from_fn(|_| AtomicGrainTelemetryLane::new()),
        }
    }

    /// Publish one complete frame.  This is a single-writer API: the writer
    /// opens an odd sequence, stores the fixed payload with relaxed ordering,
    /// then closes with an even release sequence.
    #[inline]
    pub(crate) fn publish(&self, mut value: ResynthTelemetryFrame) {
        // Older callers did not provide publication identities.  Preserve a
        // bounded monotonic fallback without making the audio caller allocate
        // or consult another synchronization primitive.
        if value.publish_frame == 0 && value.publish_count == 0 {
            value.publish_frame = self.frame.load(Ordering::Relaxed).wrapping_add(1);
        } else if value.publish_frame == 0 {
            value.publish_frame = value.publish_count;
        }
        if value.publish_count == 0 {
            value.publish_count = value.publish_frame;
        }
        // Keep compatibility fields coherent even when an older caller only
        // fills `amplitude`/`zone` and the legacy grain arrays.
        if value.envelope_proxy.to_bits() == 0 {
            value.envelope_proxy = value.amplitude;
        }
        value.amplitude = value.envelope_proxy;
        if value.rich_zone == 0 {
            value.rich_zone = value.zone;
        }
        value.zone = value.rich_zone;
        for index in 0..RESYNTH_TELEMETRY_GRAIN_LANES {
            let lane = value.grain_lanes[index];
            let legacy = GrainTelemetryLane {
                active: lane.active
                    || value.grain_gains[index].to_bits() != 0
                    || value.grain_progress[index].to_bits() != 0,
                position: if lane.position.to_bits() == 0 {
                    value.grain_positions[index]
                } else {
                    lane.position
                },
                progress: if lane.progress.to_bits() == 0 {
                    value.grain_progress[index]
                } else {
                    lane.progress
                },
                gain: if lane.gain.to_bits() == 0 {
                    value.grain_gains[index]
                } else {
                    lane.gain
                },
                phase: lane.phase,
                pan: lane.pan,
                pitch: lane.pitch,
            };
            value.grain_lanes[index] = legacy;
            value.grain_positions[index] = legacy.position;
            value.grain_progress[index] = legacy.progress;
            value.grain_gains[index] = legacy.gain;
        }
        // `generation` remains a compatibility alias for the exact dynamic
        // payload layer. New renderers always fill the explicit `to` identity;
        // older callers may still provide only the alias.
        if value.to_generation == 0 {
            value.to_generation = value.generation;
        }
        if value.generation == 0 {
            value.generation = value.to_generation;
        }
        // The caller supplies these identities.  The transport does not make
        // audio-frame progress depend on callback size or number of slots.
        self.sequence.fetch_add(1, Ordering::AcqRel);
        self.frame.store(value.publish_frame, Ordering::Relaxed);
        self.audio_frame.store(value.audio_frame, Ordering::Relaxed);
        self.generation.store(value.generation, Ordering::Relaxed);
        self.from_generation
            .store(value.from_generation, Ordering::Relaxed);
        self.from_revision
            .store(value.from_revision, Ordering::Relaxed);
        self.to_generation
            .store(value.to_generation, Ordering::Relaxed);
        self.to_revision.store(value.to_revision, Ordering::Relaxed);
        self.transition_from_gain
            .store(value.transition_from_gain.to_bits(), Ordering::Relaxed);
        self.transition_to_gain
            .store(value.transition_to_gain.to_bits(), Ordering::Relaxed);
        self.transition_progress
            .store(value.transition_progress.to_bits(), Ordering::Relaxed);
        self.phase.store(value.phase.to_bits(), Ordering::Relaxed);
        self.envelope_proxy
            .store(value.envelope_proxy.to_bits(), Ordering::Relaxed);
        self.source_mix
            .store(value.source_mix.to_bits(), Ordering::Relaxed);
        self.source_target
            .store(value.source_target.to_bits(), Ordering::Relaxed);
        self.rich_zone
            .store(u32::from(value.rich_zone), Ordering::Relaxed);
        self.rich_from_zone
            .store(u32::from(value.rich_from_zone), Ordering::Relaxed);
        self.rich_to_zone
            .store(u32::from(value.rich_to_zone), Ordering::Relaxed);
        self.rich_transition_progress
            .store(value.rich_transition_progress.to_bits(), Ordering::Relaxed);
        self.active
            .store(u32::from(value.active), Ordering::Relaxed);
        for (target, lane) in self.grain_lanes.iter().zip(value.grain_lanes) {
            target.publish(lane);
        }
        self.sequence.fetch_add(1, Ordering::Release);
    }

    /// Read one coherent frame, bounded to four retries.
    #[must_use]
    pub fn snapshot(&self) -> ResynthTelemetryFrame {
        for _ in 0..4 {
            let before = self.sequence.load(Ordering::Acquire);
            if before & 1 != 0 {
                std::hint::spin_loop();
                continue;
            }
            let mut value = ResynthTelemetryFrame {
                generation: self.generation.load(Ordering::Relaxed),
                from_generation: self.from_generation.load(Ordering::Relaxed),
                from_revision: self.from_revision.load(Ordering::Relaxed),
                to_generation: self.to_generation.load(Ordering::Relaxed),
                to_revision: self.to_revision.load(Ordering::Relaxed),
                transition_from_gain: f32::from_bits(
                    self.transition_from_gain.load(Ordering::Relaxed),
                ),
                transition_to_gain: f32::from_bits(self.transition_to_gain.load(Ordering::Relaxed)),
                transition_progress: f32::from_bits(
                    self.transition_progress.load(Ordering::Relaxed),
                ),
                publish_frame: self.frame.load(Ordering::Relaxed),
                audio_frame: self.audio_frame.load(Ordering::Relaxed),
                phase: f32::from_bits(self.phase.load(Ordering::Relaxed)),
                envelope_proxy: f32::from_bits(self.envelope_proxy.load(Ordering::Relaxed)),
                source_mix: f32::from_bits(self.source_mix.load(Ordering::Relaxed)),
                source_target: f32::from_bits(self.source_target.load(Ordering::Relaxed)),
                rich_zone: self
                    .rich_zone
                    .load(Ordering::Relaxed)
                    .min(u32::from(u16::MAX)) as u16,
                rich_from_zone: self
                    .rich_from_zone
                    .load(Ordering::Relaxed)
                    .min(u32::from(u16::MAX)) as u16,
                rich_to_zone: self
                    .rich_to_zone
                    .load(Ordering::Relaxed)
                    .min(u32::from(u16::MAX)) as u16,
                rich_transition_progress: f32::from_bits(
                    self.rich_transition_progress.load(Ordering::Relaxed),
                ),
                active: self.active.load(Ordering::Relaxed) != 0,
                ..ResynthTelemetryFrame::default()
            };
            value.publish_count = value.publish_frame;
            value.amplitude = value.envelope_proxy;
            value.zone = value.rich_zone;
            for (index, lane) in self.grain_lanes.iter().enumerate() {
                value.grain_lanes[index] = lane.snapshot();
                value.grain_positions[index] = value.grain_lanes[index].position;
                value.grain_progress[index] = value.grain_lanes[index].progress;
                value.grain_gains[index] = value.grain_lanes[index].gain;
            }
            // Keep the payload reads before the closing validation load.
            fence(Ordering::Acquire);
            let after = self.sequence.load(Ordering::Relaxed);
            if before == after {
                return value;
            }
        }
        ResynthTelemetryFrame {
            stale: true,
            ..ResynthTelemetryFrame::default()
        }
    }
}

impl Default for ResynthTelemetryTransport {
    fn default() -> Self {
        Self::new()
    }
}
