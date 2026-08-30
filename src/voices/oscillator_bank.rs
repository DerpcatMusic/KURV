use super::unison::{
    PanShapeSettings, SwarmMode, UnisonAlignmentMode, UnisonLayout, UnisonSettings,
    extended_unison_position, extended_unison_rate, unison_static_pitch_ratio,
};
use super::{MAX_UNISON, MAX_UNISON_U8, OSCILLATOR_BANK_SIZE, OscillatorMask};
use crate::generators::{GeneratorModMode, MAX_OSCILLATORS, OscillatorEngineKind};
use crate::pan_curve::PanShapeSegmentsRt;
use crate::wave_curve::WaveCurveRt;
use crate::{
    oscillators::{
        GrainSchedulerState, NoiseState, PhaseWarpMode, ResynthControls, RichVocoderState,
        SourceAuditionState, VaOscillator,
    },
    resynth_state::{ResynthArtifactView, ResynthPublicationIdentity},
};

const OSCILLATOR_TOPOLOGY_FADE_SECONDS: f32 = 0.008;
const RESYNTH_ARTIFACT_FADE_SECONDS: f32 = 0.005;

#[derive(Clone, Copy, Debug)]
pub(crate) struct ResynthPlaybackPlan {
    pub(super) from: ResynthArtifactView,
    pub(super) to: ResynthArtifactView,
    pub(super) remaining: u32,
    total: u32,
    pub(super) from_gain: f32,
    pub(super) to_gain: f32,
    pub(super) source_mix: f32,
    source_target: f32,
    source_remaining: u32,
    source_total: u32,
    pub(super) source_epoch: u64,
    pub(super) grain_live: bool,
    pub(super) grain_controls: ResynthControls,
    pub(super) grain_curve: WaveCurveRt,
}

impl Default for ResynthPlaybackPlan {
    fn default() -> Self {
        Self {
            from: ResynthArtifactView::NONE,
            to: ResynthArtifactView::NONE,
            remaining: 0,
            total: 0,
            from_gain: 0.0,
            to_gain: 0.0,
            source_mix: 0.0,
            source_target: 0.0,
            source_remaining: 0,
            source_total: 0,
            source_epoch: 0,
            grain_live: false,
            grain_controls: ResynthControls::default(),
            grain_curve: crate::wave_curve::default_grain_curve().compile_rt(),
        }
    }
}

impl ResynthPlaybackPlan {
    pub(crate) const fn transitioning(&self) -> bool {
        self.remaining != 0 || self.source_remaining != 0
    }

    #[must_use]
    pub(crate) fn sounding_algorithm(&self) -> Option<crate::oscillators::ResynthAlgorithm> {
        self.to.algorithm()
    }

    #[must_use]
    pub(crate) fn can_retarget(&self, next: ResynthArtifactView, voices_active: bool) -> bool {
        self.to.generation() == next.generation()
            || !voices_active
            || (self.remaining == 0
                && self.source_remaining == 0
                && self.source_mix <= f32::EPSILON)
    }

    pub(crate) fn retarget(
        &mut self,
        next: ResynthArtifactView,
        voices_active: bool,
        sample_rate: f32,
    ) -> bool {
        if !self.can_retarget(next, voices_active) {
            return false;
        }
        if self.to.generation() == next.generation() {
            return true;
        }
        if !voices_active {
            // No render can observe an intermediate state. Accept the newest
            // artifact immediately while preserving the requested Source A/B
            // side for a same-block retrigger.
            self.from = next;
            self.to = next;
            self.remaining = 0;
            self.total = 0;
            self.from_gain = 0.0;
            self.to_gain = 1.0;
            self.source_mix = self.source_target;
            self.source_remaining = 0;
            self.source_total = 0;
            return true;
        }
        if self.to.generation() == 0 {
            self.from = next;
            self.to = next;
            self.remaining = 0;
            self.total = 0;
            self.from_gain = 0.0;
            self.to_gain = 1.0;
            return true;
        }
        self.from = self.to;
        self.to = next;
        self.total = (sample_rate.max(1.0) * RESYNTH_ARTIFACT_FADE_SECONDS)
            .round()
            .max(1.0) as u32;
        self.remaining = self.total;
        self.from_gain = 1.0;
        self.to_gain = 0.0;
        true
    }

    /// Collapse an artifact transition once no voice can advance it while
    /// preserving the requested Source A/B side for a same-block retrigger.
    pub(crate) fn settle_artifact_when_idle(&mut self) {
        self.from = self.to;
        self.remaining = 0;
        self.total = 0;
        self.from_gain = 0.0;
        self.to_gain = f32::from(u8::from(self.to.generation() != 0));
        self.source_mix = self.source_target;
        self.source_remaining = 0;
        self.source_total = 0;
    }

    /// Collapse every monitor/render transition to the newest immutable layer.
    /// A host reset additionally forces the ephemeral Source audition off.
    pub(crate) fn settle_after_reset(&mut self) {
        self.from = self.to;
        self.remaining = 0;
        self.total = 0;
        self.from_gain = 0.0;
        self.to_gain = f32::from(u8::from(self.to.generation() != 0));
        self.source_mix = 0.0;
        self.source_target = 0.0;
        self.source_remaining = 0;
        self.source_total = 0;
    }

    pub(crate) fn advance(&mut self) {
        if self.remaining != 0 {
            self.remaining -= 1;
            let progress = 1.0 - self.remaining as f32 / self.total.max(1) as f32;
            self.from_gain = (1.0 - progress).sqrt();
            self.to_gain = progress.sqrt();
            if self.remaining == 0 {
                self.from = self.to;
                self.from_gain = 0.0;
                self.to_gain = 1.0;
            }
        }
        if self.source_remaining != 0 {
            self.source_remaining -= 1;
            let progress = 1.0 - self.source_remaining as f32 / self.source_total.max(1) as f32;
            let start = if self.source_target > 0.5 { 0.0 } else { 1.0 };
            self.source_mix = start + (self.source_target - start) * progress;
            if self.source_remaining == 0 {
                self.source_mix = self.source_target;
            }
        }
    }

    pub(crate) fn set_source_audition(&mut self, active: bool, sample_rate: f32) -> bool {
        let target = f32::from(u8::from(active));
        if (self.source_target - target).abs() <= f32::EPSILON {
            return true;
        }
        if self.remaining != 0 || self.source_remaining != 0 {
            return false;
        }
        if active {
            self.source_epoch = self.source_epoch.wrapping_add(1).max(1);
        }
        self.source_target = target;
        self.source_total = (sample_rate.max(1.0) * OSCILLATOR_TOPOLOGY_FADE_SECONDS)
            .round()
            .max(1.0) as u32;
        self.source_remaining = self.source_total;
        true
    }

    pub(crate) fn snap_source_audition(&mut self, active: bool) {
        let target = f32::from(u8::from(active));
        if active && self.source_target <= 0.5 {
            self.source_epoch = self.source_epoch.wrapping_add(1).max(1);
        }
        self.source_target = target;
        self.source_mix = target;
        self.source_remaining = 0;
        self.source_total = 0;
    }

    #[must_use]
    pub(crate) fn transition_progress(self) -> f32 {
        if self.from.generation() == self.to.generation() || self.total == 0 {
            1.0
        } else {
            (1.0 - self.remaining as f32 / self.total as f32).clamp(0.0, 1.0)
        }
    }

    #[must_use]
    pub(crate) fn source_target(self) -> f32 {
        self.source_target
    }

    #[must_use]
    pub(crate) fn accepted_publication(self) -> ResynthPublicationIdentity {
        self.to.publication_identity()
    }

    #[must_use]
    pub(crate) fn live_generations(self) -> [u64; 2] {
        let from = self.from.generation();
        let to = self.to.generation();
        [from, if to == from { 0 } else { to }]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ResynthPlaybackPtr(*const ResynthPlaybackPlan);

impl ResynthPlaybackPtr {
    pub(crate) const NONE: Self = Self(std::ptr::null());

    /// # Safety
    /// `plan` must remain address-stable and valid for every copied pointer.
    pub(crate) const unsafe fn new(plan: *const ResynthPlaybackPlan) -> Self {
        Self(plan)
    }

    /// # Safety
    /// The plan owner must outlive the immediate render operation.
    pub(super) const unsafe fn get(self) -> Option<&'static ResynthPlaybackPlan> {
        // SAFETY: required by the caller; null represents no artifact.
        unsafe { self.0.as_ref() }
    }
}

// SAFETY: the pointer targets immutable reads from address-stable PolySynth
// storage; mutation is serialized between render samples on the audio thread.
unsafe impl Send for ResynthPlaybackPtr {}
// SAFETY: helper rendering remains disabled for active RESYNTH plans.
unsafe impl Sync for ResynthPlaybackPtr {}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhaseWarpControl {
    pub mode: PhaseWarpMode,
    pub amount: f32,
}

impl PhaseWarpControl {
    pub const NONE: Self = Self {
        mode: PhaseWarpMode::None,
        amount: 0.0,
    };

    pub const fn new(mode: PhaseWarpMode, amount: f32) -> Self {
        Self {
            mode,
            amount: amount.clamp(0.0, 1.0),
        }
    }

    pub(super) fn active(self) -> bool {
        self.mode != PhaseWarpMode::None && self.amount > f32::EPSILON
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct OscillatorDspSettings {
    pub(super) engine: OscillatorEngineKind,
    pub(super) resynth_playback: ResynthPlaybackPtr,
    pub(super) shape: f32,
    pub(super) pulse_width: f32,
    pub(super) custom_curve: WaveCurveRt,
    pub(super) custom_mix: f32,
    pub(super) positioned_wave: bool,
    pub(super) phase_warp: PhaseWarpControl,
    pub(super) phase_mod_source: u8,
    pub(super) phase_mod_amount: f32,
    pub(super) modulation_mode: GeneratorModMode,
    pub(super) pitch_ratio: f32,
    pub(super) left_gain: f32,
    pub(super) right_gain: f32,
    pub(super) unison_voices: u8,
    pub(super) render_voices: u8,
    pub(super) unison_jitter: f32,
    pub(super) swarm_depth_cents: f32,
    pub(super) unison_jitter_mode: SwarmMode,
    pub(super) jitter_rate_hz: f32,
    pub(super) phase_position: f32,
    pub(super) phase_random: f32,
    pub(super) unison_alignment: f32,
    pub(super) unison_alignment_mode: UnisonAlignmentMode,
    pub(super) unison_pan_curve: f32,
    pub(super) spatial_settings: UnisonSettings,
    pub(super) lane_pitch_ratios: [f32; MAX_UNISON],
    pub(super) lane_left_gains: [f32; MAX_UNISON],
    pub(super) lane_right_gains: [f32; MAX_UNISON],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct StructuralOscillatorAbsoluteControl {
    pub(crate) shape: f32,
    pub(crate) pulse_width: f32,
    pub(crate) pitch_ratio: f32,
    pub(crate) phase_position: f32,
    pub(crate) phase_warp_amount: f32,
    pub(crate) phase_mod_amount: f32,
    pub(crate) left_gain: f32,
    pub(crate) right_gain: f32,
    pub(crate) unison_jitter: f32,
    pub(crate) unison_rate: f32,
    /// Sample-frame Pan X/Y deltas. The immutable spatial curve/settings stay
    /// in `OscillatorDspSettings` instead of being copied for every frame.
    pub(crate) stereo_x: f32,
    pub(crate) stereo_y: f32,
    pub(crate) grain_tune: f32,
    pub(crate) grain_stereo: f32,
    pub(crate) rich_dynamic: f32,
}

impl StructuralOscillatorAbsoluteControl {
    const NEUTRAL: Self = Self {
        shape: 0.0,
        pulse_width: 0.5,
        pitch_ratio: 1.0,
        phase_position: 0.0,
        phase_warp_amount: 0.0,
        phase_mod_amount: 0.0,
        left_gain: 0.0,
        right_gain: 0.0,
        unison_jitter: 0.0,
        unison_rate: 0.0,
        stereo_x: 0.0,
        stereo_y: 0.0,
        grain_tune: 0.0,
        grain_stereo: 0.0,
        rich_dynamic: 0.0,
    };
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct StructuralOscillatorFrameControl {
    pub(crate) mask: OscillatorMask,
    pub(crate) slots: [StructuralOscillatorAbsoluteControl; OSCILLATOR_BANK_SIZE],
}

impl StructuralOscillatorFrameControl {
    pub(crate) const NEUTRAL: Self = Self {
        mask: 0,
        slots: [StructuralOscillatorAbsoluteControl::NEUTRAL; OSCILLATOR_BANK_SIZE],
    };

    #[inline(always)]
    pub(super) fn get(&self, slot: usize) -> Option<&StructuralOscillatorAbsoluteControl> {
        (self.mask & (1 << slot) != 0).then_some(&self.slots[slot])
    }
}

impl Default for StructuralOscillatorFrameControl {
    fn default() -> Self {
        Self::NEUTRAL
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
/// Complete realtime configuration for one reusable oscillator instance.
/// The synth owns a fixed 32-slot bank and only walks the active slot list.
pub(crate) struct OscillatorDspConfig {
    pub enabled: bool,
    pub engine: OscillatorEngineKind,
    pub resynth_playback: ResynthPlaybackPtr,
    pub shape: f32,
    pub pulse_width: f32,
    pub custom_curve: WaveCurveRt,
    pub custom_mix: f32,
    pub positioned_wave: bool,
    pub phase_warp_mode: u8,
    pub phase_warp_amount: f32,
    pub phase_mod_source: u8,
    pub phase_mod_amount: f32,
    pub modulation_mode: GeneratorModMode,
    pub transpose: f32,
    pub cents: f32,
    pub level: f32,
    pub pan: f32,
    pub unison_voices: u8,
    pub unison_range: f32,
    pub unison_amount: f32,
    pub unison_curve: f32,
    pub unison_jitter: f32,
    pub unison_jitter_mode: u8,
    pub unison_rate: f32,
    pub unison_weight: f32,
    pub unison_width: f32,
    pub phase_position: f32,
    pub phase_random: f32,
    pub unison_alignment: f32,
    pub unison_alignment_mode: u8,
    pub unison_pan_curve: f32,
    pub unison_pan_center_x: f32,
    pub unison_pan_segments: (PanShapeSegmentsRt, PanShapeSegmentsRt),
    pub unison_stereo_x: f32,
    pub unison_stereo_alternate: f32,
}

pub(crate) fn fill_oscillator_unison_layout(
    settings: UnisonSettings,
    detune_positions: &mut [f32; MAX_UNISON],
    left: &mut [f32; MAX_UNISON],
    right: &mut [f32; MAX_UNISON],
) {
    detune_positions.fill(0.0);
    left.fill(0.0);
    right.fill(0.0);
    let voices = usize::from(settings.voices);
    let core_count = usize::from(!settings.voices.is_multiple_of(2));
    let center = voices / 2;
    for lane in 0..voices {
        let sorted_index = if lane < core_count {
            center
        } else {
            let satellite = lane - core_count;
            let pair = satellite / 2 + 1;
            if satellite.is_multiple_of(2) {
                center - pair
            } else {
                center + pair - (1 - core_count)
            }
        };
        detune_positions[lane] =
            extended_unison_position(settings.voices, sorted_index, settings.curve);
    }
    let _ = UnisonLayout::build_spatial_from_positions(
        settings,
        0.618_034,
        detune_positions,
        left,
        right,
    );
    let lane_gain = f32::from(settings.voices).sqrt().recip();
    for lane in 0..voices {
        left[lane] *= lane_gain;
        right[lane] *= lane_gain;
    }
}

impl Default for OscillatorDspSettings {
    fn default() -> Self {
        let mut lane_left_gains = [0.0; MAX_UNISON];
        let mut lane_right_gains = [0.0; MAX_UNISON];
        lane_left_gains[0] = 1.0;
        lane_right_gains[0] = 1.0;
        Self {
            engine: OscillatorEngineKind::Va,
            resynth_playback: ResynthPlaybackPtr::NONE,
            shape: 2.0,
            pulse_width: 0.5,
            custom_curve: WaveCurveRt::zero(),
            custom_mix: 0.0,
            positioned_wave: false,
            phase_warp: PhaseWarpControl::NONE,
            phase_mod_source: 0,
            phase_mod_amount: 0.0,
            modulation_mode: GeneratorModMode::Phase,
            pitch_ratio: 1.0,
            left_gain: 0.0,
            right_gain: 0.0,
            unison_voices: 1,
            render_voices: 1,
            unison_jitter: 0.0,
            swarm_depth_cents: 0.0,
            unison_jitter_mode: SwarmMode::Noise,
            jitter_rate_hz: 0.7,
            phase_position: 0.0,
            phase_random: 1.0,
            unison_alignment: 0.0,
            unison_alignment_mode: UnisonAlignmentMode::Note,
            unison_pan_curve: 0.0,
            spatial_settings: UnisonSettings::new(1, 0.0, 1.0, 1.0, 0.0),
            lane_pitch_ratios: [1.0; MAX_UNISON],
            lane_left_gains,
            lane_right_gains,
        }
    }
}

impl OscillatorDspSettings {
    pub(super) fn jitter_active(self) -> bool {
        self.render_voices > 1
            && self.swarm_depth_cents > f32::EPSILON
            && self.unison_jitter > f32::EPSILON
    }

    fn from_config(config: OscillatorDspConfig) -> Self {
        let level = config.level.clamp(0.0, 1.0);
        let pan = config.pan.clamp(-1.0, 1.0);
        let voices = config.unison_voices.clamp(1, MAX_UNISON_U8);
        let mut lane_pitch_ratios = [1.0; MAX_UNISON];
        let mut lane_left_gains = [0.0; MAX_UNISON];
        let mut lane_right_gains = [0.0; MAX_UNISON];
        let unison_range = config.unison_range.clamp(0.0, 48.0);
        let unison_amount = config.unison_amount.clamp(0.0, 1.0);
        let unison_alignment = config.unison_alignment.clamp(0.0, 1.0);
        let unison_alignment_mode = UnisonAlignmentMode::from_index(config.unison_alignment_mode);
        let unison_pan_curve = config.unison_pan_curve.clamp(-1.0, 1.0);
        let unison_jitter_mode = SwarmMode::from_index(config.unison_jitter_mode);
        let unison_weight = config.unison_weight.clamp(-1.0, 1.0);
        let pan_shape = PanShapeSettings::default()
            .with_center_x(config.unison_pan_center_x)
            .with_segments(config.unison_pan_segments);
        let spatial_settings = UnisonSettings::new(
            voices,
            unison_range * 100.0,
            config.unison_width,
            config.phase_random,
            config.unison_curve,
        )
        .with_stereo_square(config.unison_stereo_alternate, config.unison_stereo_x)
        .with_level_curve(unison_weight)
        .with_pan_shape(pan_shape);
        let mut detune_positions = [0.0; MAX_UNISON];
        fill_oscillator_unison_layout(
            spatial_settings,
            &mut detune_positions,
            &mut lane_left_gains,
            &mut lane_right_gains,
        );
        for lane in 0..usize::from(voices) {
            lane_pitch_ratios[lane] = unison_static_pitch_ratio(
                detune_positions[lane],
                unison_range * 100.0,
                unison_amount,
                unison_alignment,
                unison_alignment_mode,
            );
        }
        Self {
            engine: config.engine,
            resynth_playback: config.resynth_playback,
            shape: config.shape.clamp(0.0, 3.0),
            pulse_width: config.pulse_width.clamp(0.03, 0.97),
            custom_curve: config.custom_curve,
            custom_mix: config.custom_mix.clamp(0.0, 1.0),
            positioned_wave: config.positioned_wave,
            phase_warp: PhaseWarpControl::new(
                PhaseWarpMode::from_index(config.phase_warp_mode),
                config.phase_warp_amount,
            ),
            phase_mod_source: config.phase_mod_source,
            phase_mod_amount: config.phase_mod_amount.clamp(-1.0, 1.0),
            modulation_mode: config.modulation_mode,
            pitch_ratio: fast_exp2(
                (config.transpose.clamp(-48.0, 48.0) + config.cents.clamp(-100.0, 100.0) * 0.01)
                    / 12.0,
            ),
            left_gain: level * (1.0 - pan).sqrt(),
            right_gain: level * (1.0 + pan).sqrt(),
            unison_voices: voices,
            render_voices: voices,
            unison_jitter: config.unison_jitter.clamp(0.0, 1.0),
            swarm_depth_cents: unison_range * 100.0 * unison_amount,
            unison_jitter_mode,
            jitter_rate_hz: extended_unison_rate(config.unison_rate),
            phase_position: config.phase_position.clamp(0.0, 1.0),
            phase_random: config.phase_random.clamp(0.0, 1.0),
            unison_alignment,
            unison_alignment_mode,
            unison_pan_curve,
            spatial_settings,
            lane_pitch_ratios,
            lane_left_gains,
            lane_right_gains,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct ActiveOscillatorRenderEntry {
    pub(super) slot: u8,
    pub(super) current: OscillatorDspSettings,
    pub(super) target: OscillatorDspSettings,
    transition_remaining: u32,
}

pub(super) struct ActiveOscillatorRenderSet {
    pub(super) count: u8,
    pub(super) entries: [std::mem::MaybeUninit<ActiveOscillatorRenderEntry>; OSCILLATOR_BANK_SIZE],
    pub(super) mask: OscillatorMask,
    pub(super) target_mask: OscillatorMask,
    pub(super) transition_mask: OscillatorMask,
}

impl Default for ActiveOscillatorRenderSet {
    fn default() -> Self {
        Self {
            count: 0,
            entries: [std::mem::MaybeUninit::uninit(); OSCILLATOR_BANK_SIZE],
            mask: 0,
            target_mask: 0,
            transition_mask: 0,
        }
    }
}

impl ActiveOscillatorRenderSet {
    pub(super) fn entries(&self) -> &[ActiveOscillatorRenderEntry] {
        // SAFETY: count only includes the initialized prefix written by insert/copy_from.
        unsafe {
            std::slice::from_raw_parts(
                self.entries.as_ptr().cast::<ActiveOscillatorRenderEntry>(),
                usize::from(self.count),
            )
        }
    }

    pub(super) fn entries_mut(&mut self) -> &mut [ActiveOscillatorRenderEntry] {
        // SAFETY: count only includes the initialized prefix written by insert/copy_from.
        unsafe {
            std::slice::from_raw_parts_mut(
                self.entries
                    .as_mut_ptr()
                    .cast::<ActiveOscillatorRenderEntry>(),
                usize::from(self.count),
            )
        }
    }

    pub(super) fn entry(&self, slot: usize) -> &ActiveOscillatorRenderEntry {
        let index = self
            .entries()
            .binary_search_by_key(&slot, |entry| usize::from(entry.slot))
            .expect("active oscillator slot must have render settings");
        &self.entries()[index]
    }

    pub(super) fn entry_mut(&mut self, slot: usize) -> &mut ActiveOscillatorRenderEntry {
        let index = self
            .entries()
            .binary_search_by_key(&slot, |entry| usize::from(entry.slot))
            .expect("active oscillator slot must have render settings");
        &mut self.entries_mut()[index]
    }

    fn insert(&mut self, entry: ActiveOscillatorRenderEntry) {
        let count = usize::from(self.count);
        let index = self
            .entries()
            .partition_point(|active| active.slot < entry.slot);
        assert!(count < OSCILLATOR_BANK_SIZE);
        // SAFETY: the initialized suffix is moved one slot right inside the fixed allocation;
        // ActiveOscillatorRenderEntry is Copy, so duplicating the old bytes needs no drop.
        unsafe {
            std::ptr::copy(
                self.entries.as_ptr().add(index),
                self.entries.as_mut_ptr().add(index + 1),
                count - index,
            );
        }
        self.entries[index].write(entry);
        self.count += 1;
    }

    fn retain_mask(&mut self, mask: OscillatorMask) {
        let mut retained = 0;
        for index in 0..usize::from(self.count) {
            // SAFETY: index is inside the initialized prefix and the entry is Copy.
            let entry = unsafe { *self.entries[index].assume_init_ref() };
            if mask & (1 << entry.slot) != 0 {
                self.entries[retained].write(entry);
                retained += 1;
            }
        }
        self.count = retained as u8;
    }

    pub(super) fn copy_from(&mut self, source: &Self) {
        let count = usize::from(source.count);
        // SAFETY: source and destination are disjoint borrows with room for the initialized
        // prefix; entries are Copy and have no drop state.
        unsafe {
            std::ptr::copy_nonoverlapping(
                source.entries.as_ptr(),
                self.entries.as_mut_ptr(),
                count,
            );
        }
        self.count = source.count;
        self.mask = source.mask;
        self.target_mask = source.target_mask;
        self.transition_mask = source.transition_mask;
    }

    pub(super) fn advance(&mut self, _sample_rate: f32) {
        let target_mask = self.target_mask;
        let mut transition_mask = self.transition_mask;
        let mut finished = 0;
        for entry in self.entries_mut() {
            let slot = usize::from(entry.slot);
            let current = &mut entry.current;
            let target = &entry.target;
            let bit = 1 << slot;
            if transition_mask & bit != 0 {
                let step = entry.transition_remaining.max(1) as f32;
                current.left_gain += (target.left_gain - current.left_gain) / step;
                current.right_gain += (target.right_gain - current.right_gain) / step;
                current.phase_mod_amount +=
                    (target.phase_mod_amount - current.phase_mod_amount) / step;
                for lane in 0..usize::from(current.render_voices) {
                    current.lane_left_gains[lane] +=
                        (target.lane_left_gains[lane] - current.lane_left_gains[lane]) / step;
                    current.lane_right_gains[lane] +=
                        (target.lane_right_gains[lane] - current.lane_right_gains[lane]) / step;
                }
                entry.transition_remaining = entry.transition_remaining.saturating_sub(1);
                if entry.transition_remaining == 0 {
                    *current = *target;
                    transition_mask &= !bit;
                }
            }
            if target_mask & bit == 0 && transition_mask & bit == 0 {
                finished |= bit;
            }
        }
        self.transition_mask = transition_mask;
        if finished != 0 {
            self.mask &= !finished;
            self.retain_mask(self.mask);
        }
    }

    pub(super) const fn active(&self) -> bool {
        self.count != 0
    }

    pub(super) const fn transitioning(&self) -> bool {
        self.transition_mask != 0 || self.mask != self.target_mask
    }
}

#[inline]
pub(super) fn shortest_phase_delta(current: f32, target: f32) -> f32 {
    let delta = target - current;
    if delta > 0.5 {
        delta - 1.0
    } else if delta < -0.5 {
        delta + 1.0
    } else {
        delta
    }
}

pub(super) struct ActiveOscillatorSet {
    pub(super) render: ActiveOscillatorRenderSet,
    pub(super) configured: [Option<OscillatorDspConfig>; OSCILLATOR_BANK_SIZE],
}

impl Default for ActiveOscillatorSet {
    fn default() -> Self {
        Self {
            render: ActiveOscillatorRenderSet::default(),
            configured: [None; OSCILLATOR_BANK_SIZE],
        }
    }
}

impl ActiveOscillatorSet {
    pub(super) fn configure(
        &mut self,
        configs: [OscillatorDspConfig; MAX_OSCILLATORS],
        sample_rate: f32,
    ) -> OscillatorMask {
        let previous_render_mask = self.render.mask;
        let previous_target_mask = self.render.target_mask;
        let transition_samples = (sample_rate.max(1.0) * OSCILLATOR_TOPOLOGY_FADE_SECONDS)
            .round()
            .max(1.0) as u32;
        let mut target_mask = 0;
        for (slot, config) in configs.into_iter().enumerate() {
            let bit = 1 << slot;
            if !config.enabled {
                if previous_target_mask & bit != 0 {
                    let entry = self.render.entry_mut(slot);
                    entry.target = entry.current;
                    entry.target.left_gain = 0.0;
                    entry.target.right_gain = 0.0;
                    entry.transition_remaining = transition_samples;
                    self.render.transition_mask |= bit;
                }
                self.configured[slot] = Some(config);
                continue;
            }
            if self.render.mask & bit == 0 {
                self.render.insert(ActiveOscillatorRenderEntry {
                    slot: slot as u8,
                    current: OscillatorDspSettings::default(),
                    target: OscillatorDspSettings::default(),
                    transition_remaining: 0,
                });
                self.render.mask |= bit;
            }
            if self.configured[slot] != Some(config) {
                let next = OscillatorDspSettings::from_config(config);
                let entry = self.render.entry_mut(slot);
                if previous_target_mask & bit == 0 {
                    entry.current = next;
                    entry.current.left_gain = 0.0;
                    entry.current.right_gain = 0.0;
                    entry.target = next;
                    entry.transition_remaining = transition_samples;
                    self.render.transition_mask |= bit;
                } else if entry.current.render_voices != next.unison_voices {
                    let previous = entry.current;
                    entry.current = next;
                    entry.current.render_voices = previous.render_voices.max(next.unison_voices);
                    for lane in 0..usize::from(entry.current.render_voices) {
                        entry.current.lane_left_gains[lane] = previous.lane_left_gains[lane];
                        entry.current.lane_right_gains[lane] = previous.lane_right_gains[lane];
                    }
                    entry.target = next;
                    entry.transition_remaining = transition_samples;
                    self.render.transition_mask |= bit;
                } else if entry.current.phase_mod_source != next.phase_mod_source
                    || entry.current.phase_mod_amount != next.phase_mod_amount
                    || entry.current.modulation_mode != next.modulation_mode
                {
                    let previous = entry.current;
                    entry.current = next;
                    entry.current.phase_mod_source = if next.phase_mod_source == 0 {
                        previous.phase_mod_source
                    } else {
                        next.phase_mod_source
                    };
                    entry.current.modulation_mode = if next.phase_mod_source == 0
                        || previous.modulation_mode != next.modulation_mode
                    {
                        previous.modulation_mode
                    } else {
                        next.modulation_mode
                    };
                    entry.current.phase_mod_amount = if previous.phase_mod_source
                        == next.phase_mod_source
                        && previous.modulation_mode == next.modulation_mode
                        || next.phase_mod_source == 0
                    {
                        previous.phase_mod_amount
                    } else {
                        0.0
                    };
                    entry.target = next;
                    entry.transition_remaining = transition_samples;
                    self.render.transition_mask |= bit;
                } else {
                    entry.current = next;
                    entry.target = next;
                    entry.transition_remaining = 0;
                    self.render.transition_mask &= !bit;
                }
            }
            self.configured[slot] = Some(config);
            target_mask |= bit;
        }
        let newly_started = target_mask & !previous_render_mask;
        self.render.target_mask = target_mask;
        self.render.mask |= target_mask;
        newly_started
    }

    pub(super) fn advance(&mut self, sample_rate: f32) {
        self.render.advance(sample_rate);
    }

    pub(super) fn snap_to_targets(&mut self) {
        let target_mask = self.render.target_mask;
        for entry in self.render.entries_mut() {
            if target_mask & (1 << entry.slot) != 0 {
                entry.current = entry.target;
            }
        }
        self.render.mask = target_mask;
        self.render.transition_mask = 0;
        self.render.retain_mask(target_mask);
    }

    pub(super) const fn active(&self) -> bool {
        self.render.active()
    }

    pub(super) const fn transitioning(&self) -> bool {
        self.render.transitioning()
    }

    pub(super) const fn render(&self) -> &ActiveOscillatorRenderSet {
        &self.render
    }
}

/// Accurate over KURV's bounded pitch-control range without libm calls.
#[inline]
pub(crate) fn fast_exp2(exponent: f32) -> f32 {
    let exponent = exponent.clamp(-126.0, 126.0);
    let integer = exponent.floor();
    let fraction = exponent - integer;
    let y = fraction * std::f32::consts::LN_2;
    let polynomial = 1.0
        + y * (1.0
            + y * (0.5
                + y * (1.0 / 6.0
                    + y * (1.0 / 24.0 + y * (1.0 / 120.0 + y * (1.0 / 720.0 + y / 5_040.0))))));
    let scale = f32::from_bits(((integer as i32 + 127) as u32) << 23);
    scale * polynomial
}

#[derive(Debug)]
pub(super) struct OscillatorBankVoiceState {
    pub(super) oscillators: Box<[[VaOscillator; MAX_UNISON]]>,
    pub(super) applied_phase_positions: [f32; OSCILLATOR_BANK_SIZE],
    pub(super) jitter_ratios: [[f32; MAX_UNISON]; OSCILLATOR_BANK_SIZE],
    pub(super) jitter_steps: [[f32; MAX_UNISON]; OSCILLATOR_BANK_SIZE],
    pub(super) jitter_clocks: [f32; OSCILLATOR_BANK_SIZE],
    pub(super) jitter_remaining: [u16; OSCILLATOR_BANK_SIZE],
    pub(super) resynth_grain: Box<[[GrainSchedulerState; 2]]>,
    pub(super) resynth_grain_generations: [[u64; 2]; OSCILLATOR_BANK_SIZE],
    pub(super) resynth_vocoder: Box<[[RichVocoderState; 2]]>,
    pub(super) resynth_vocoder_generations: [[u64; 2]; OSCILLATOR_BANK_SIZE],
    pub(super) resynth_frame: [u64; OSCILLATOR_BANK_SIZE],
    pub(super) resynth_source: [SourceAuditionState; OSCILLATOR_BANK_SIZE],
    pub(super) noise: [NoiseState; OSCILLATOR_BANK_SIZE],
}

impl Default for OscillatorBankVoiceState {
    fn default() -> Self {
        Self {
            oscillators: (0..OSCILLATOR_BANK_SIZE)
                .map(|_| std::array::from_fn(|_| VaOscillator::default()))
                .collect(),
            applied_phase_positions: [0.0; OSCILLATOR_BANK_SIZE],
            jitter_ratios: [[1.0; MAX_UNISON]; OSCILLATOR_BANK_SIZE],
            jitter_steps: [[0.0; MAX_UNISON]; OSCILLATOR_BANK_SIZE],
            jitter_clocks: [0.0; OSCILLATOR_BANK_SIZE],
            jitter_remaining: [0; OSCILLATOR_BANK_SIZE],
            resynth_grain: (0..OSCILLATOR_BANK_SIZE)
                .map(|_| [GrainSchedulerState::default(); 2])
                .collect(),
            resynth_grain_generations: [[0; 2]; OSCILLATOR_BANK_SIZE],
            resynth_vocoder: (0..OSCILLATOR_BANK_SIZE)
                .map(|_| [RichVocoderState::default(); 2])
                .collect(),
            resynth_vocoder_generations: [[0; 2]; OSCILLATOR_BANK_SIZE],
            resynth_frame: [0; OSCILLATOR_BANK_SIZE],
            resynth_source: [SourceAuditionState::default(); OSCILLATOR_BANK_SIZE],
            noise: [NoiseState::default(); OSCILLATOR_BANK_SIZE],
        }
    }
}

impl OscillatorBankVoiceState {
    pub(super) fn copy_render_state_from(
        &mut self,
        source: &Self,
        settings: &ActiveOscillatorRenderSet,
    ) {
        for entry in settings.entries() {
            let slot = usize::from(entry.slot);
            let lanes = usize::from(entry.current.render_voices);
            self.oscillators[slot][..lanes].copy_from_slice(&source.oscillators[slot][..lanes]);
            self.applied_phase_positions[slot] = source.applied_phase_positions[slot];
            self.jitter_ratios[slot][..lanes].copy_from_slice(&source.jitter_ratios[slot][..lanes]);
            self.jitter_steps[slot][..lanes].copy_from_slice(&source.jitter_steps[slot][..lanes]);
            self.jitter_clocks[slot] = source.jitter_clocks[slot];
            self.jitter_remaining[slot] = source.jitter_remaining[slot];
            self.resynth_grain[slot] = source.resynth_grain[slot];
            self.resynth_grain_generations[slot] = source.resynth_grain_generations[slot];
            self.resynth_vocoder[slot] = source.resynth_vocoder[slot];
            self.resynth_vocoder_generations[slot] = source.resynth_vocoder_generations[slot];
            self.resynth_frame[slot] = source.resynth_frame[slot];
            self.resynth_source[slot] = source.resynth_source[slot];
            self.noise[slot] = source.noise[slot];
        }
    }

    pub(super) fn reset(&mut self) {
        for bank in &mut self.oscillators {
            for oscillator in bank {
                oscillator.reset();
            }
        }
        self.applied_phase_positions.fill(0.0);
        self.jitter_ratios.fill([1.0; MAX_UNISON]);
        self.jitter_steps.fill([0.0; MAX_UNISON]);
        self.jitter_clocks.fill(0.0);
        self.jitter_remaining.fill(0);
        self.resynth_grain.fill([GrainSchedulerState::default(); 2]);
        self.resynth_grain_generations.fill([0; 2]);
        self.resynth_vocoder.fill([RichVocoderState::default(); 2]);
        self.resynth_vocoder_generations.fill([0; 2]);
        self.resynth_frame.fill(0);
        self.resynth_source.fill(SourceAuditionState::default());
        self.noise.fill(NoiseState::default());
    }

    pub(super) fn seed_slot(
        &mut self,
        state_index: usize,
        slot: usize,
        seed: u64,
        settings: OscillatorDspSettings,
    ) {
        let slot_seed = seed ^ (slot as u64).wrapping_mul(0x4f53_435f_4241_4e4b);
        for (lane, oscillator) in self.oscillators[state_index].iter_mut().enumerate() {
            let lane_seed =
                slot_seed.wrapping_add((lane as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15));
            oscillator.set_phase(
                (f64::from(settings.phase_position)
                    + unit_hash(lane_seed).mul_add(2.0, -1.0) * f64::from(settings.phase_random))
                .rem_euclid(1.0),
            );
            oscillator.restart_rich_timeline(
                (f64::from(settings.phase_position)
                    + unit_hash(lane_seed ^ 0x5249_4348_5f54_494d).mul_add(2.0, -1.0)
                        * f64::from(settings.phase_random))
                .rem_euclid(1.0) as f32,
            );
        }
        self.applied_phase_positions[state_index] = settings.phase_position;
        self.jitter_ratios[state_index].fill(1.0);
        self.jitter_steps[state_index].fill(0.0);
        self.jitter_clocks[state_index] = 0.0;
        self.jitter_remaining[state_index] = 0;
        self.resynth_grain[state_index] = [GrainSchedulerState::default(); 2];
        self.resynth_grain_generations[state_index] = [0; 2];
        self.resynth_vocoder[state_index] = [RichVocoderState::default(); 2];
        self.resynth_vocoder_generations[state_index] = [0; 2];
        self.resynth_source[state_index] = SourceAuditionState::default();
        self.resynth_frame[state_index] = 0;
        self.noise[state_index].reset(slot_seed ^ 0x4e4f_4953_455f_5254);
    }

    pub(super) fn seed_all(&mut self, seed: u64, settings: &ActiveOscillatorRenderSet) {
        for entry in settings.entries() {
            let slot = usize::from(entry.slot);
            let mut seed_settings = entry.target;
            seed_settings.phase_position = entry.current.phase_position;
            self.seed_slot(slot, slot, seed, seed_settings);
        }
    }
}

#[allow(
    clippy::cast_precision_loss,
    reason = "all 53 retained hash bits are exactly representable in f64"
)]
pub(super) fn unit_hash(seed: u64) -> f64 {
    let mut value = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^= value >> 31;
    (value >> 11) as f64 * (1.0 / ((1_u64 << 53) as f64))
}

#[cfg(test)]
mod resynth_transition_tests {
    use super::*;

    fn tone() -> Vec<u8> {
        crate::wav_test::wav_i16(
            1,
            48_000,
            (0..24_000).map(|index| {
                let sample = (std::f32::consts::TAU * 220.0 * index as f32 / 48_000.0).sin();
                (sample * 24_000.0) as i16
            }),
        )
    }

    #[test]
    fn revision_and_source_transitions_never_admit_more_than_two_layers() {
        let assets = crate::resynth_state::ResynthAssetPackState::new();
        let controls = crate::oscillators::ResynthControls::default();
        let model = crate::oscillators::analyze_wav("tone.wav", tone(), controls).expect("analyze");
        let slot = assets.slot(0).expect("slot");
        let first = slot
            .replace(
                model,
                crate::oscillators::ResynthAlgorithm::Sample,
                controls,
            )
            .expect("Sample");
        let first_view = slot.try_rt_view_after(0).expect("first view");
        let mut plan = ResynthPlaybackPlan::default();
        assert!(plan.retarget(first_view, false, 48_000.0));
        slot.acknowledge_rt(
            first,
            crate::resynth_state::ResynthRtPlanAck {
                live_generations: [first, 0],
                accepted: plan.accepted_publication(),
            },
        );

        let second = slot
            .select_algorithm(crate::oscillators::ResynthAlgorithm::Rich)
            .expect("Rich")
            .expect("generation");
        let second_view = slot.try_rt_view_after(first).expect("second view");
        assert!(plan.retarget(second_view, true, 48_000.0));
        assert_eq!(plan.live_generations(), [first, second]);
        assert!(!plan.set_source_audition(true, 48_000.0));
        let samples = plan.remaining;
        for _ in 0..samples {
            plan.advance();
        }
        assert_eq!(plan.live_generations(), [second, 0]);
        assert!(plan.set_source_audition(true, 48_000.0));
        assert!(!plan.retarget(first_view, true, 48_000.0));
        for _ in 0..plan.source_remaining {
            plan.advance();
        }
        assert!((plan.source_mix - 1.0).abs() <= f32::EPSILON);
        assert!(plan.set_source_audition(false, 48_000.0));
    }

    #[test]
    fn idle_artifact_retarget_preserves_held_source_audition() {
        let assets = crate::resynth_state::ResynthAssetPackState::new();
        let controls = crate::oscillators::ResynthControls::default();
        let model = crate::oscillators::analyze_wav("tone.wav", tone(), controls).expect("analyze");
        let slot = assets.slot(0).expect("slot");
        let first = slot
            .replace(
                model,
                crate::oscillators::ResynthAlgorithm::Sample,
                controls,
            )
            .expect("Sample");
        let first_view = slot.try_rt_view_after(0).expect("first view");
        let mut plan = ResynthPlaybackPlan::default();
        assert!(plan.retarget(first_view, false, 48_000.0));
        plan.snap_source_audition(true);

        let second = slot
            .select_algorithm(crate::oscillators::ResynthAlgorithm::Rich)
            .expect("Rich")
            .expect("generation");
        let second_view = slot.try_rt_view_after(first).expect("second view");
        assert!(plan.retarget(second_view, false, 48_000.0));
        assert_eq!(plan.live_generations(), [second, 0]);
        assert_eq!(plan.source_mix, 1.0);
        assert_eq!(plan.source_target, 1.0);
        assert_eq!(plan.source_remaining, 0);
    }

    #[test]
    fn host_reset_settles_artifact_and_source_transitions_to_one_layer() {
        let assets = crate::resynth_state::ResynthAssetPackState::new();
        let controls = crate::oscillators::ResynthControls::default();
        let model = crate::oscillators::analyze_wav("tone.wav", tone(), controls).expect("analyze");
        let slot = assets.slot(0).expect("slot");
        let first = slot
            .replace(
                model,
                crate::oscillators::ResynthAlgorithm::Sample,
                controls,
            )
            .expect("Sample");
        let first_view = slot.try_rt_view_after(0).expect("first view");
        let mut plan = ResynthPlaybackPlan::default();
        assert!(plan.retarget(first_view, false, 48_000.0));
        let second = slot
            .select_algorithm(crate::oscillators::ResynthAlgorithm::Rich)
            .expect("Rich")
            .expect("generation");
        let second_view = slot.try_rt_view_after(first).expect("second view");
        assert!(plan.retarget(second_view, true, 48_000.0));
        plan.advance();
        assert_ne!(plan.remaining, 0);
        plan.settle_after_reset();
        assert_eq!(plan.remaining, 0);
        assert_eq!(plan.live_generations(), [second, 0]);
        assert_eq!(plan.from_gain, 0.0);
        assert_eq!(plan.to_gain, 1.0);
        assert_eq!(plan.source_mix, 0.0);
    }

    #[test]
    fn note_seed_resets_resynth_schedulers_source_and_frame_identity() {
        let mut state = OscillatorBankVoiceState::default();
        let artifact = crate::oscillators::GrainSourceArtifact::silence();
        let _ = state.resynth_grain[0][0].render(&artifact, 110.0, 48_000.0, 7, 0);
        assert!(state.resynth_grain[0][0].active_count() > 0);
        state.resynth_grain_generations[0] = [11, 12];
        state.resynth_source[0].position = 314.0;
        state.resynth_frame[0] = 99;
        state.seed_slot(0, 0, 123, OscillatorDspSettings::default());
        assert_eq!(state.resynth_grain[0][0].active_count(), 0);
        assert_eq!(state.resynth_grain[0][1].active_count(), 0);
        assert_eq!(state.resynth_grain_generations[0], [0, 0]);
        assert_eq!(state.resynth_source[0].position, 0.0);
        assert_eq!(state.resynth_frame[0], 0);
    }

    #[test]
    fn source_audition_snaps_when_no_voice_can_advance_the_fade() {
        let mut plan = ResynthPlaybackPlan::default();
        plan.snap_source_audition(true);
        assert_eq!(plan.source_mix, 1.0);
        assert_eq!(plan.source_remaining, 0);
        plan.snap_source_audition(false);
        assert!(plan.set_source_audition(true, 48_000.0));
        plan.advance();
        assert_ne!(plan.source_remaining, 0);
        plan.settle_artifact_when_idle();
        assert_eq!(plan.source_mix, 1.0);
        assert_eq!(plan.source_remaining, 0);
        plan.snap_source_audition(false);
        assert_eq!(plan.source_mix, 0.0);
        assert_eq!(plan.source_remaining, 0);
    }
}
