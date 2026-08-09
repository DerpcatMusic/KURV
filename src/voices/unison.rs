use super::{MAX_UNISON, MAX_UNISON_U8, fast_exp2, unit_hash};
use crate::pan_curve::{PanShapeCurveData, PanShapeSegmentsRt};

const UNISON_LANE_FADE_SECONDS: f32 = 0.005;
const UNISON_GAIN_QUANTIZATION: f32 = 32_767.5;
const TRANSITION_TUNING: u8 = 1;
const TRANSITION_SPATIAL: u8 = 2;

/// Maximum pitch excursion of one jitter lane at 100% amount. This is deliberately
/// independent of the static unison range so a collapsed stack can still move.
pub const JITTER_EXCURSION_CENTS: f32 = 50.0;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UnisonAlignmentMode {
    #[default]
    Note,
    Harmonic,
    Odd,
    Even,
}

impl UnisonAlignmentMode {
    pub const fn from_index(index: u8) -> Self {
        match index {
            1 => Self::Harmonic,
            2 => Self::Odd,
            3 => Self::Even,
            _ => Self::Note,
        }
    }

    pub const fn index(self) -> u8 {
        match self {
            Self::Note => 0,
            Self::Harmonic => 1,
            Self::Odd => 2,
            Self::Even => 3,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Note => "NOTE",
            Self::Harmonic => "HARM",
            Self::Odd => "ODD",
            Self::Even => "EVEN",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct AlignmentCandidate {
    pub(super) ratio: f32,
    pub(super) cents: f32,
}

pub(super) const EMPTY_ALIGNMENT_CANDIDATE: AlignmentCandidate = AlignmentCandidate {
    ratio: 1.0,
    cents: 0.0,
};

// The lattice is bounded to the first 16 partials. It is built once per synth
// and dynamic lookups use bounded binary searches over the cached candidates.
const HARMONIC_PARTIAL_LIMIT: u32 = 16;
const HARMONIC_OCTAVE_LIMIT: u32 = 4;
pub(super) const HARMONIC_CANDIDATE_CAP: usize =
    HARMONIC_PARTIAL_LIMIT as usize * (HARMONIC_OCTAVE_LIMIT as usize + 1);
pub(super) const ALIGNMENT_EPSILON: f32 = 0.000_001;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SwarmMode {
    #[default]
    Noise,
    Sine,
    #[doc(hidden)]
    Jitter,
    #[doc(hidden)]
    Wander,
}

impl SwarmMode {
    pub const fn from_index(index: u8) -> Self {
        if index == 1 { Self::Sine } else { Self::Noise }
    }

    const fn canonical(self) -> Self {
        if matches!(self, Self::Sine) {
            Self::Sine
        } else {
            Self::Noise
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanShapeSettings {
    pub center: f32,
    pub center_x: f32,
    pub left_edge: f32,
    pub right_edge: f32,
    pub left_curve: f32,
    pub right_curve: f32,
    pub left_curve_time: f32,
    pub right_curve_time: f32,
    pub left_segments: PanShapeSegmentsRt,
    pub right_segments: PanShapeSegmentsRt,
}

impl Default for PanShapeSettings {
    fn default() -> Self {
        Self {
            center: 0.0,
            center_x: 0.5,
            left_edge: 1.0,
            right_edge: 1.0,
            left_curve: 0.0,
            right_curve: 0.0,
            left_curve_time: 0.5,
            right_curve_time: 0.5,
            left_segments: PanShapeSegmentsRt::identity(),
            right_segments: PanShapeSegmentsRt::identity(),
        }
    }
}

impl PanShapeSettings {
    pub fn new(center: f32, edge: f32, curve: f32) -> Self {
        Self {
            center: center.clamp(0.0, 1.0),
            center_x: 0.5,
            left_edge: edge.clamp(0.0, 1.0),
            right_edge: edge.clamp(0.0, 1.0),
            left_curve: curve.clamp(-1.0, 1.0),
            right_curve: curve.clamp(-1.0, 1.0),
            left_curve_time: 0.5,
            right_curve_time: 0.5,
            left_segments: PanShapeSegmentsRt::identity(),
            right_segments: PanShapeSegmentsRt::identity(),
        }
    }

    pub fn symmetric_curve(curve: f32) -> Self {
        let curve = curve.clamp(-1.0, 1.0);
        let mut segments = PanShapeSegmentsRt::identity();
        segments.seg_p1[0] = curve.mul_add(0.5, 0.5);
        Self::new(0.0, 1.0, curve).with_segments((segments, segments))
    }

    pub fn with_sides(
        mut self,
        left_edge: f32,
        right_edge: f32,
        left_curve: f32,
        right_curve: f32,
    ) -> Self {
        self.left_edge = left_edge.clamp(0.0, 1.0);
        self.right_edge = right_edge.clamp(0.0, 1.0);
        self.left_curve = left_curve.clamp(-1.0, 1.0);
        self.right_curve = right_curve.clamp(-1.0, 1.0);
        self
    }

    pub fn with_curve_times(mut self, left: f32, right: f32) -> Self {
        self.left_curve_time = left.clamp(0.05, 0.95);
        self.right_curve_time = right.clamp(0.05, 0.95);
        self
    }

    pub fn with_center_x(mut self, center_x: f32) -> Self {
        self.center_x = center_x.clamp(0.05, 0.95);
        self
    }

    pub fn with_segments(mut self, segments: (PanShapeSegmentsRt, PanShapeSegmentsRt)) -> Self {
        self.left_segments = segments.0;
        self.right_segments = segments.1;
        self
    }

    pub fn with_curve_data(mut self, data: &PanShapeCurveData) -> Self {
        let (left, right) = data.compile_rt();
        self.left_segments = left;
        self.right_segments = right;
        self
    }

    fn modulated(mut self, center: f32, left: f32, right: f32, center_x: f32) -> Self {
        self.center_x = (self.center_x + center_x).clamp(0.05, 0.95);
        self.center = (self.center + center).clamp(0.0, 1.0);
        self.left_edge = (self.left_edge + left).clamp(0.0, 1.0);
        self.right_edge = (self.right_edge + right).clamp(0.0, 1.0);
        for (segments, edge) in [
            (&mut self.left_segments, left),
            (&mut self.right_segments, right),
        ] {
            for index in 0..usize::from(segments.count) {
                let start_delta = center + (edge - center) * segments.seg_x0[index];
                let end_delta = center + (edge - center) * segments.seg_x1[index];
                segments.seg_p0[index] = (segments.seg_p0[index] + start_delta).clamp(0.0, 1.0);
                segments.seg_p3[index] = (segments.seg_p3[index] + end_delta).clamp(0.0, 1.0);
            }
        }
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnisonSettings {
    pub(super) voices: u8,
    pub(super) detune_cents: f32,
    pub(super) stereo: f32,
    pub(super) phase_random: f32,
    pub(super) phase_position: f32,
    pub(super) curve: f32,
    pub(super) stereo_alternate: f32,
    pub(super) stereo_x: f32,
    pub(super) level_curve: f32,
    pub(super) detune_amount: f32,
    pub(super) harmonic_align: f32,
    pub(super) alignment_mode: UnisonAlignmentMode,
    pub(super) pan_shape: PanShapeSettings,
    pub(super) swarm_amount: f32,
    pub(super) swarm_rate: f32,
    pub(super) swarm_mode: SwarmMode,
}

impl UnisonSettings {
    pub fn new(voices: u8, detune_cents: f32, stereo: f32, phase_random: f32, curve: f32) -> Self {
        Self {
            voices: voices.clamp(1, MAX_UNISON_U8),
            detune_cents: detune_cents.clamp(0.0, 4_800.0),
            stereo: stereo.clamp(0.0, 1.0),
            phase_random: phase_random.clamp(0.0, 1.0),
            phase_position: 0.0,
            curve: curve.clamp(-1.0, 1.0),
            stereo_alternate: 1.0,
            stereo_x: 0.0,
            level_curve: 0.0,
            detune_amount: 1.0,
            harmonic_align: 0.0,
            alignment_mode: UnisonAlignmentMode::Note,
            pan_shape: PanShapeSettings::new(0.0, 1.0, 0.0),
            swarm_amount: 0.0,
            swarm_rate: 0.7,
            swarm_mode: SwarmMode::Noise,
        }
    }

    pub fn with_stereo_square(mut self, vertical: f32, horizontal: f32) -> Self {
        self.stereo_alternate = vertical.clamp(0.0, 1.0);
        self.stereo_x = horizontal.clamp(0.0, 1.0);
        self
    }

    #[allow(dead_code, reason = "legacy source compatibility")]
    pub fn with_stereo_triangle(self, alternate: f32, x: f32) -> Self {
        self.with_stereo_square(alternate, x)
    }

    pub const fn with_level_curve(mut self, curve: f32) -> Self {
        self.level_curve = curve.clamp(-1.0, 1.0);
        self
    }

    pub const fn with_detune_amount(mut self, amount: f32) -> Self {
        self.detune_amount = amount.clamp(0.0, 1.0);
        self
    }

    pub const fn with_harmonic_align(mut self, amount: f32) -> Self {
        self.harmonic_align = amount.clamp(0.0, 1.0);
        self
    }

    pub const fn with_alignment_mode(mut self, mode: u8) -> Self {
        self.alignment_mode = UnisonAlignmentMode::from_index(mode);
        self
    }

    pub const fn with_pan_shape(mut self, shape: PanShapeSettings) -> Self {
        self.pan_shape = shape;
        self
    }

    pub const fn with_phase_random(mut self, amount: f32) -> Self {
        self.phase_random = amount.clamp(0.0, 1.0);
        self
    }

    pub const fn with_phase_position(mut self, position: f32) -> Self {
        self.phase_position = position.clamp(0.0, 1.0);
        self
    }

    pub const fn with_swarm(mut self, amount: f32, rate: f32) -> Self {
        self.swarm_amount = amount.clamp(0.0, 1.0);
        self.swarm_rate = rate.clamp(0.02, 100.0);
        self
    }

    pub const fn with_swarm_mode(mut self, mode: SwarmMode) -> Self {
        self.swarm_mode = mode.canonical();
        self
    }

    pub const fn with_motion(
        mut self,
        phase_random: f32,
        swarm_amount: f32,
        swarm_rate: f32,
    ) -> Self {
        self.phase_random = phase_random.clamp(0.0, 1.0);
        self.swarm_amount = swarm_amount.clamp(0.0, 1.0);
        self.swarm_rate = swarm_rate.clamp(0.02, 100.0);
        self
    }

    pub fn modulated(mut self, modulation: crate::modulators::lfo::UnisonModulation) -> Self {
        self.detune_cents = (self.detune_cents + modulation.detune_cents).clamp(0.0, 4_800.0);
        self.stereo = (self.stereo + modulation.stereo).clamp(0.0, 1.0);
        self.phase_random = (self.phase_random + modulation.phase_random).clamp(0.0, 1.0);
        self.curve = (self.curve + modulation.curve).clamp(-1.0, 1.0);
        self.swarm_amount = (self.swarm_amount + modulation.jitter_amount).clamp(0.0, 1.0);
        self.swarm_rate = (self.swarm_rate
            * 5_000.0_f32.powf(modulation.jitter_rate_normalized.clamp(-1.0, 1.0)))
        .clamp(0.02, 100.0);
        self.stereo_x = (self.stereo_x + modulation.stereo_x).clamp(0.0, 1.0);
        self.stereo_alternate = (self.stereo_alternate + modulation.stereo_y).clamp(0.0, 1.0);
        self.level_curve = (self.level_curve + modulation.weight).clamp(-1.0, 1.0);
        self.pan_shape = self.pan_shape.modulated(
            modulation.pan_center,
            modulation.pan_left,
            modulation.pan_right,
            modulation.pan_center_x,
        );
        self
    }

    pub const fn detune_cents(self) -> f32 {
        self.detune_cents
    }

    pub const fn detune_amount(self) -> f32 {
        self.detune_amount
    }

    pub const fn harmonic_align(self) -> f32 {
        self.harmonic_align
    }

    pub const fn phase_random(self) -> f32 {
        self.phase_random
    }

    pub const fn phase_position(self) -> f32 {
        self.phase_position
    }

    pub const fn swarm_amount(self) -> f32 {
        self.swarm_amount
    }

    pub const fn swarm_rate(self) -> f32 {
        self.swarm_rate
    }

    pub const fn curve(self) -> f32 {
        self.curve
    }

    pub const fn pan_shape(self) -> PanShapeSettings {
        self.pan_shape
    }

    pub const fn stereo(self) -> f32 {
        self.stereo
    }

    pub const fn stereo_alternate(self) -> f32 {
        self.stereo_alternate
    }

    pub const fn stereo_x(self) -> f32 {
        self.stereo_x
    }

    pub const fn level_curve(self) -> f32 {
        self.level_curve
    }

    pub(crate) const fn motion_active(self) -> bool {
        self.voices > 1 && self.swarm_amount > f32::EPSILON
    }
}

#[derive(Debug)]
pub(super) struct UnisonLayout {
    pub(super) settings: UnisonSettings,
    pub(super) ratios: [f32; MAX_UNISON],
    pub(super) ratio_reciprocals: [f32; MAX_UNISON],
    pub(super) harmonic_targets: [AlignmentCandidate; MAX_UNISON],
    pub(super) detune_positions: [f32; MAX_UNISON],
    pub(super) left: [f32; MAX_UNISON],
    pub(super) right: [f32; MAX_UNISON],
    spatial_alternate: [f32; MAX_UNISON],
    spatial_pair: [f32; MAX_UNISON],
    spatial_random: [f32; MAX_UNISON],
    spatial_shape: [f32; MAX_UNISON],
    pub(super) gain: f32,
    // Allocated once with the voice; live retargets only mutate its fixed arrays.
    target: Box<UnisonTarget>,
    pub(super) render_voices: u8,
    transition_remaining: u16,
    transition_mask: u8,
    pub(super) random_seed: f32,
}

#[derive(Debug)]
struct UnisonTarget {
    ratios: [f32; MAX_UNISON],
    detune_positions: [f32; MAX_UNISON],
    left: [u16; MAX_UNISON],
    right: [u16; MAX_UNISON],
    density: f32,
    target_density: f32,
    phase_ratio_bound: f32,
    tuning: bool,
}

impl Default for UnisonLayout {
    fn default() -> Self {
        Self {
            settings: UnisonSettings::new(1, 0.0, 0.0, 1.0, 0.0),
            ratios: [1.0; MAX_UNISON],
            ratio_reciprocals: [1.0; MAX_UNISON],
            harmonic_targets: [EMPTY_ALIGNMENT_CANDIDATE; MAX_UNISON],
            detune_positions: [0.0; MAX_UNISON],
            left: [1.0; MAX_UNISON],
            right: [1.0; MAX_UNISON],
            spatial_alternate: [0.0; MAX_UNISON],
            spatial_pair: [0.0; MAX_UNISON],
            spatial_random: [0.0; MAX_UNISON],
            spatial_shape: [0.0; MAX_UNISON],
            gain: 1.0,
            target: Box::new(UnisonTarget {
                ratios: [1.0; MAX_UNISON],
                detune_positions: [0.0; MAX_UNISON],
                left: [32_768; MAX_UNISON],
                right: [32_768; MAX_UNISON],
                density: 1.0,
                target_density: 1.0,
                phase_ratio_bound: 1.0,
                tuning: false,
            }),
            render_voices: 1,
            transition_remaining: 0,
            transition_mask: 0,
            random_seed: 0.5,
        }
    }
}

impl UnisonLayout {
    pub(super) fn configure(
        &mut self,
        settings: UnisonSettings,
        sample_rate: f32,
        fade_lanes: bool,
    ) -> bool {
        self.configure_with_prepared(settings, sample_rate, fade_lanes, None)
    }

    pub(super) fn configure_motion(&mut self, settings: UnisonSettings) -> bool {
        let changed = self.settings.phase_random.to_bits() != settings.phase_random.to_bits()
            || self.settings.swarm_amount.to_bits() != settings.swarm_amount.to_bits()
            || self.settings.swarm_rate.to_bits() != settings.swarm_rate.to_bits()
            || self.settings.swarm_mode != settings.swarm_mode;
        self.settings.phase_random = settings.phase_random;
        self.settings.swarm_amount = settings.swarm_amount;
        self.settings.swarm_rate = settings.swarm_rate;
        self.settings.swarm_mode = settings.swarm_mode;
        changed
    }

    pub(super) fn configure_with_prepared(
        &mut self,
        settings: UnisonSettings,
        sample_rate: f32,
        fade_lanes: bool,
        prepared: Option<&Self>,
    ) -> bool {
        let voices_changed = self.settings.voices != settings.voices;
        let tuning_changed = voices_changed
            || self.settings.detune_cents.to_bits() != settings.detune_cents.to_bits()
            || self.settings.curve.to_bits() != settings.curve.to_bits()
            || self.settings.detune_amount.to_bits() != settings.detune_amount.to_bits()
            || self.settings.harmonic_align.to_bits() != settings.harmonic_align.to_bits()
            || self.settings.alignment_mode != settings.alignment_mode;
        let spatial_changed = voices_changed
            || self.settings.stereo.to_bits() != settings.stereo.to_bits()
            || self.settings.curve.to_bits() != settings.curve.to_bits()
            || self.settings.stereo_alternate.to_bits() != settings.stereo_alternate.to_bits()
            || self.settings.stereo_x.to_bits() != settings.stereo_x.to_bits()
            || self.settings.level_curve.to_bits() != settings.level_curve.to_bits()
            || self.settings.pan_shape != settings.pan_shape;
        let layout_changed = tuning_changed || spatial_changed;
        let motion_changed = self.settings.swarm_amount.to_bits()
            != settings.swarm_amount.to_bits()
            || self.settings.swarm_rate.to_bits() != settings.swarm_rate.to_bits()
            || self.settings.swarm_mode != settings.swarm_mode;
        self.settings.phase_random = settings.phase_random;
        self.settings.phase_position = settings.phase_position;
        if !layout_changed && !motion_changed {
            return false;
        }

        self.settings = settings;
        self.target.phase_ratio_bound = Self::phase_ratio_bound(settings);
        if layout_changed {
            if fade_lanes {
                if let Some(prepared) = prepared {
                    self.retarget_from_prepared(
                        settings,
                        sample_rate,
                        tuning_changed,
                        spatial_changed,
                        prepared,
                    );
                } else {
                    self.retarget(settings, sample_rate, tuning_changed, spatial_changed);
                }
            } else if tuning_changed && spatial_changed {
                self.rebuild();
            } else if tuning_changed {
                self.rebuild_tuning(settings);
            } else {
                self.rebuild_spatial(settings);
            }
        }
        true
    }

    pub(super) fn set_random_seed(&mut self, random_seed: f32) {
        let random_seed = random_seed.clamp(0.0, 1.0);
        if self.random_seed.to_bits() != random_seed.to_bits() {
            self.random_seed = random_seed;
            if stereo_square_weights(self.settings.stereo_alternate, self.settings.stereo_x)[2]
                > f32::EPSILON
            {
                self.rebuild();
            }
        }
    }

    fn rebuild(&mut self) {
        self.gain = Self::build(
            self.settings,
            self.random_seed,
            &mut self.ratios,
            &mut self.detune_positions,
            &mut self.left,
            &mut self.right,
        );
        self.refresh_ratio_reciprocals();
        self.refresh_spatial_components();
        self.target.ratios = self.ratios;
        self.target.detune_positions = self.detune_positions;
        self.target.left = self.left.map(Self::encode_gain);
        self.target.right = self.right.map(Self::encode_gain);
        self.target.density = Self::density(self.settings.voices);
        self.target.target_density = self.target.density;
        self.target.tuning = false;
        self.render_voices = self.settings.voices;
        self.transition_remaining = 0;
        self.transition_mask = 0;
    }

    fn rebuild_tuning(&mut self, settings: UnisonSettings) {
        for index in 0..usize::from(settings.voices) {
            self.ratios[index] = unison_static_pitch(
                self.detune_positions[index],
                settings.detune_cents,
                settings.detune_amount,
                settings.harmonic_align,
                settings.alignment_mode,
            )
            .ratio;
        }
        self.refresh_ratio_reciprocals();
        self.target.ratios = self.ratios;
        self.target.detune_positions = self.detune_positions;
        self.target.left = self.left.map(Self::encode_gain);
        self.target.right = self.right.map(Self::encode_gain);
        self.target.density = Self::density(settings.voices);
        self.target.target_density = self.target.density;
        self.target.tuning = false;
        self.render_voices = settings.voices;
        self.transition_remaining = 0;
        self.transition_mask = 0;
    }

    fn rebuild_spatial(&mut self, settings: UnisonSettings) {
        self.gain =
            Self::build_spatial(settings, self.random_seed, &mut self.left, &mut self.right);
        self.refresh_spatial_components();
        self.target.ratios = self.ratios;
        self.target.detune_positions = self.detune_positions;
        self.target.left = self.left.map(Self::encode_gain);
        self.target.right = self.right.map(Self::encode_gain);
        self.target.density = Self::density(settings.voices);
        self.target.target_density = self.target.density;
        self.target.tuning = false;
        self.render_voices = settings.voices;
        self.transition_remaining = 0;
        self.transition_mask = 0;
    }

    fn refresh_ratio_reciprocals(&mut self) {
        for (reciprocal, ratio) in self.ratio_reciprocals.iter_mut().zip(self.ratios) {
            *reciprocal = ratio.max(f32::EPSILON).recip();
        }
    }

    fn retarget(
        &mut self,
        settings: UnisonSettings,
        sample_rate: f32,
        tuning_changed: bool,
        spatial_changed: bool,
    ) {
        let mut target_left = [0.0; MAX_UNISON];
        let mut target_right = [0.0; MAX_UNISON];
        let _ = Self::build(
            settings,
            self.random_seed,
            &mut self.target.ratios,
            &mut self.target.detune_positions,
            &mut target_left,
            &mut target_right,
        );
        self.target.left = target_left.map(Self::encode_gain);
        self.target.right = target_right.map(Self::encode_gain);
        self.target.target_density = Self::density(settings.voices);
        let previous_voices = self.render_voices;
        self.render_voices = previous_voices.max(settings.voices);
        for index in usize::from(previous_voices)..usize::from(settings.voices) {
            self.ratios[index] = self.target.ratios[index];
            self.detune_positions[index] = self.target.detune_positions[index];
            self.left[index] = 0.0;
            self.right[index] = 0.0;
        }
        for index in usize::from(settings.voices)..usize::from(self.render_voices) {
            self.target.ratios[index] = self.ratios[index];
            self.target.left[index] = 0;
            self.target.right[index] = 0;
        }
        self.transition_remaining = (sample_rate * UNISON_LANE_FADE_SECONDS)
            .round()
            .clamp(1.0, f32::from(u16::MAX)) as u16;
        self.transition_mask = u8::from(tuning_changed) * TRANSITION_TUNING
            | u8::from(spatial_changed) * TRANSITION_SPATIAL;
        self.target.tuning |= tuning_changed;
    }

    fn refresh_spatial_components(&mut self) {
        for index in 0..usize::from(self.settings.voices) {
            let (_, alternate, pair, random, shape, _) = unison_lane_stereo_components(
                self.settings.voices,
                index,
                self.settings.curve,
                self.settings.pan_shape,
                self.random_seed,
            );
            self.spatial_alternate[index] = alternate;
            self.spatial_pair[index] = pair;
            self.spatial_random[index] = random;
            self.spatial_shape[index] = shape;
        }
    }

    fn retarget_from_prepared(
        &mut self,
        settings: UnisonSettings,
        sample_rate: f32,
        tuning_changed: bool,
        spatial_changed: bool,
        prepared: &Self,
    ) {
        self.target.ratios = prepared.target.ratios;
        self.target.detune_positions = prepared.target.detune_positions;
        if stereo_square_weights(settings.stereo_alternate, settings.stereo_x)[2] <= f32::EPSILON {
            self.target.left = prepared.target.left;
            self.target.right = prepared.target.right;
        } else {
            let mut target_left = [0.0; MAX_UNISON];
            let mut target_right = [0.0; MAX_UNISON];
            let _ = build_spatial_from_prepared_components(
                prepared,
                settings,
                self.random_seed,
                &mut target_left,
                &mut target_right,
            );
            self.target.left = target_left.map(Self::encode_gain);
            self.target.right = target_right.map(Self::encode_gain);
        }
        self.target.target_density = Self::density(settings.voices);
        let previous_voices = self.render_voices;
        self.render_voices = previous_voices.max(settings.voices);
        for index in usize::from(previous_voices)..usize::from(settings.voices) {
            self.ratios[index] = self.target.ratios[index];
            self.detune_positions[index] = self.target.detune_positions[index];
            self.left[index] = 0.0;
            self.right[index] = 0.0;
        }
        for index in usize::from(settings.voices)..usize::from(self.render_voices) {
            self.target.ratios[index] = self.ratios[index];
            self.target.left[index] = 0;
            self.target.right[index] = 0;
        }
        self.transition_remaining = (sample_rate * UNISON_LANE_FADE_SECONDS)
            .round()
            .clamp(1.0, f32::from(u16::MAX)) as u16;
        self.transition_mask = u8::from(tuning_changed) * TRANSITION_TUNING
            | u8::from(spatial_changed) * TRANSITION_SPATIAL;
        self.target.tuning |= tuning_changed;
    }

    pub(super) fn advance_transition(&mut self) -> bool {
        if self.transition_remaining == 0 {
            return false;
        }
        let tuning_changed = self.target.tuning;
        let amount = f32::from(self.transition_remaining).recip();
        if self.transition_mask == TRANSITION_TUNING {
            for index in 0..usize::from(self.render_voices) {
                self.ratios[index] += (self.target.ratios[index] - self.ratios[index]) * amount;
            }
        } else if self.transition_mask == TRANSITION_SPATIAL {
            let mut energy = 0.0;
            for index in 0..usize::from(self.render_voices) {
                self.left[index] +=
                    (Self::decode_gain(self.target.left[index]) - self.left[index]) * amount;
                self.right[index] +=
                    (Self::decode_gain(self.target.right[index]) - self.right[index]) * amount;
                energy += (self.left[index] * self.left[index]
                    + self.right[index] * self.right[index])
                    * 0.5;
            }
            self.target.density += (self.target.target_density - self.target.density) * amount;
            self.gain = self.target.density / energy.max(f32::EPSILON).sqrt();
        } else {
            let mut energy = 0.0;
            for index in 0..usize::from(self.render_voices) {
                self.ratios[index] += (self.target.ratios[index] - self.ratios[index]) * amount;
                self.left[index] +=
                    (Self::decode_gain(self.target.left[index]) - self.left[index]) * amount;
                self.right[index] +=
                    (Self::decode_gain(self.target.right[index]) - self.right[index]) * amount;
                energy += (self.left[index] * self.left[index]
                    + self.right[index] * self.right[index])
                    * 0.5;
            }
            self.target.density += (self.target.target_density - self.target.density) * amount;
            self.gain = self.target.density / energy.max(f32::EPSILON).sqrt();
        }
        self.transition_remaining -= 1;
        if self.transition_remaining == 0 {
            self.rebuild();
        }
        tuning_changed
    }

    pub(super) const fn transition_active(&self) -> bool {
        self.transition_remaining != 0
    }

    fn phase_ratio_bound(settings: UnisonSettings) -> f32 {
        if settings.voices <= 1 {
            return 1.0;
        }
        let jitter_ratio = (settings.swarm_amount * JITTER_EXCURSION_CENTS / 1_200.0).exp2();
        let free_ratio = (settings.detune_cents.abs() * settings.detune_amount / 1_200.0).exp2();
        // Harmonic targets are constrained to the effective static range, so
        // the free-detune bound also bounds every aligned target.
        free_ratio * jitter_ratio
    }

    pub(super) fn settle(&mut self) {
        if self.transition_active() {
            self.rebuild();
        }
    }

    pub(super) fn copy_render_state_from(&mut self, source: &Self) {
        self.settings = source.settings;
        self.ratios = source.ratios;
        self.ratio_reciprocals = source.ratio_reciprocals;
        self.harmonic_targets = source.harmonic_targets;
        self.detune_positions = source.detune_positions;
        self.left = source.left;
        self.right = source.right;
        self.spatial_alternate = source.spatial_alternate;
        self.spatial_pair = source.spatial_pair;
        self.spatial_random = source.spatial_random;
        self.spatial_shape = source.spatial_shape;
        self.gain = source.gain;
        self.render_voices = source.render_voices;
        self.transition_remaining = 0;
        self.transition_mask = 0;
        self.random_seed = source.random_seed;
        self.target.phase_ratio_bound = source.target.phase_ratio_bound;
    }

    pub(super) fn copy_prepared_from(&mut self, source: &Self) {
        self.copy_render_state_from(source);
        self.target.ratios = source.target.ratios;
        self.target.detune_positions = source.target.detune_positions;
        self.target.left = source.target.left;
        self.target.right = source.target.right;
        self.target.density = source.target.density;
        self.target.target_density = source.target.target_density;
        self.target.tuning = false;
    }

    fn density(voices: u8) -> f32 {
        1.0 + 0.2 * f32::from(voices - 1) / 63.0
    }

    fn encode_gain(gain: f32) -> u16 {
        (gain.clamp(0.0, 2.0) * UNISON_GAIN_QUANTIZATION).round() as u16
    }

    fn decode_gain(gain: u16) -> f32 {
        f32::from(gain) / UNISON_GAIN_QUANTIZATION
    }

    fn build(
        settings: UnisonSettings,
        random_seed: f32,
        ratios: &mut [f32; MAX_UNISON],
        detune_positions: &mut [f32; MAX_UNISON],
        left: &mut [f32; MAX_UNISON],
        right: &mut [f32; MAX_UNISON],
    ) -> f32 {
        let mut energy = 0.0;
        let mut weighted_pan = 0.0;
        let mut weight_sum = 0.0;
        for index in 0..usize::from(settings.voices) {
            let (detune_position, pan_position, weight) = unison_lane_position_stereo_seeded(
                settings.voices,
                index,
                settings.curve,
                settings.stereo_alternate,
                settings.stereo_x,
                settings.level_curve,
                settings.pan_shape,
                random_seed,
            );
            detune_positions[index] = detune_position;
            ratios[index] = unison_static_pitch(
                detune_position,
                settings.detune_cents,
                settings.detune_amount,
                settings.harmonic_align,
                settings.alignment_mode,
            )
            .ratio;
            left[index] = pan_position;
            right[index] = weight;
            let lane_energy = weight * weight;
            weighted_pan = pan_position.mul_add(lane_energy, weighted_pan);
            weight_sum += lane_energy;
            energy += lane_energy;
        }
        Self::finish_spatial(settings, left, right, energy, weighted_pan, weight_sum)
    }

    fn build_spatial(
        settings: UnisonSettings,
        random_seed: f32,
        left: &mut [f32; MAX_UNISON],
        right: &mut [f32; MAX_UNISON],
    ) -> f32 {
        let mut energy = 0.0;
        let mut weighted_pan = 0.0;
        let mut weight_sum = 0.0;
        for index in 0..usize::from(settings.voices) {
            let (_, pan_position, weight) = unison_lane_position_stereo_seeded(
                settings.voices,
                index,
                settings.curve,
                settings.stereo_alternate,
                settings.stereo_x,
                settings.level_curve,
                settings.pan_shape,
                random_seed,
            );
            left[index] = pan_position;
            right[index] = weight;
            let lane_energy = weight * weight;
            weighted_pan = pan_position.mul_add(lane_energy, weighted_pan);
            weight_sum += lane_energy;
            energy += lane_energy;
        }
        Self::finish_spatial(settings, left, right, energy, weighted_pan, weight_sum)
    }

    pub(super) fn build_spatial_from_positions(
        settings: UnisonSettings,
        random_seed: f32,
        detune_positions: &[f32; MAX_UNISON],
        left: &mut [f32; MAX_UNISON],
        right: &mut [f32; MAX_UNISON],
    ) -> f32 {
        let [alternate_weight, pair_weight, random_weight, shape_weight] =
            stereo_square_weights(settings.stereo_alternate, settings.stereo_x);
        let voices = usize::from(settings.voices);
        if random_weight <= f32::EPSILON && shape_weight <= f32::EPSILON {
            let core_count = usize::from(!settings.voices.is_multiple_of(2));
            let pair_count = usize::from(settings.voices - core_count as u8) / 2;
            let mut energy = 0.0;
            let mut weighted_pan = 0.0;
            for index in 0..voices {
                let (alternate_pan, pair_pan, radius) = if index < core_count {
                    (0.0, 0.0, 0.0)
                } else {
                    let satellite = index - core_count;
                    let pair = satellite / 2 + 1;
                    let detune_sign = if satellite.is_multiple_of(2) {
                        -1.0
                    } else {
                        1.0
                    };
                    let ring_sign = if pair.is_multiple_of(2) { -1.0 } else { 1.0 };
                    (
                        detune_sign * ring_sign,
                        if pair_count == 1 {
                            detune_sign
                        } else {
                            ring_sign
                        },
                        detune_positions[index].abs(),
                    )
                };
                let pan =
                    alternate_weight.mul_add(alternate_pan, pair_weight.mul_add(pair_pan, 0.0));
                let weight = unison_lane_weight(radius, settings.level_curve);
                left[index] = pan;
                right[index] = weight;
                let lane_energy = weight * weight;
                weighted_pan = pan.mul_add(lane_energy, weighted_pan);
                energy += lane_energy;
            }
            return Self::finish_spatial(settings, left, right, energy, weighted_pan, energy);
        }
        if random_weight <= f32::EPSILON {
            let core_count = usize::from(!settings.voices.is_multiple_of(2));
            let pair_count = usize::from(settings.voices - core_count as u8) / 2;
            let mut energy = 0.0;
            let mut weighted_pan = 0.0;
            let mut weight_sum = 0.0;
            for index in 0..voices {
                let (alternate_pan, pair_pan, shape_pan, radius) = if index < core_count {
                    (0.0, 0.0, 0.0, 0.0)
                } else {
                    let satellite = index - core_count;
                    let pair = satellite / 2 + 1;
                    let detune_sign = if satellite.is_multiple_of(2) {
                        -1.0
                    } else {
                        1.0
                    };
                    let ring_sign = if pair.is_multiple_of(2) { -1.0 } else { 1.0 };
                    let radius = detune_positions[index].abs();
                    (
                        detune_sign * ring_sign,
                        if pair_count == 1 {
                            detune_sign
                        } else {
                            ring_sign
                        },
                        detune_sign
                            * ring_sign
                            * pan_shape_curve_value_side(radius, detune_sign, settings.pan_shape),
                        radius,
                    )
                };
                let pan = alternate_weight.mul_add(
                    alternate_pan,
                    pair_weight.mul_add(
                        pair_pan,
                        random_weight.mul_add(0.0, shape_weight * shape_pan),
                    ),
                );
                let weight = unison_lane_weight(radius, settings.level_curve);
                left[index] = pan;
                right[index] = weight;
                let lane_energy = weight * weight;
                weighted_pan = pan.mul_add(lane_energy, weighted_pan);
                weight_sum += lane_energy;
                energy += lane_energy;
            }
            return Self::finish_spatial(settings, left, right, energy, weighted_pan, weight_sum);
        }
        let mut energy = 0.0;
        let mut weighted_pan = 0.0;
        let mut weight_sum = 0.0;
        for index in 0..voices {
            let (_, alternate_pan, pair_pan, random_pan, shape_pan, radius) =
                unison_lane_stereo_components_at_position(
                    settings.voices,
                    index,
                    detune_positions[index],
                    settings.pan_shape,
                    random_seed,
                );
            let pan = alternate_weight.mul_add(
                alternate_pan,
                pair_weight.mul_add(
                    pair_pan,
                    random_weight.mul_add(random_pan, shape_weight * shape_pan),
                ),
            );
            let weight = unison_lane_weight(radius, settings.level_curve);
            left[index] = pan;
            right[index] = weight;
            let lane_energy = weight * weight;
            weighted_pan = pan.mul_add(lane_energy, weighted_pan);
            weight_sum += lane_energy;
            energy += lane_energy;
        }
        Self::finish_spatial(settings, left, right, energy, weighted_pan, weight_sum)
    }

    fn finish_spatial(
        settings: UnisonSettings,
        left: &mut [f32; MAX_UNISON],
        right: &mut [f32; MAX_UNISON],
        energy: f32,
        weighted_pan: f32,
        weight_sum: f32,
    ) -> f32 {
        let pan_center = weighted_pan / weight_sum.max(f32::EPSILON);
        let pan_scale = left[..usize::from(settings.voices)]
            .iter()
            .fold(0.0_f32, |maximum, pan| {
                maximum.max((*pan - pan_center).abs())
            })
            .max(f32::EPSILON)
            .recip();
        for index in 0..usize::from(settings.voices) {
            let weight = right[index];
            let pan = ((left[index] - pan_center) * pan_scale * settings.stereo).clamp(-1.0, 1.0);
            left[index] = weight * (1.0 - pan).sqrt();
            right[index] = weight * (1.0 + pan).sqrt();
        }
        let density = Self::density(settings.voices);
        if energy > 0.0 {
            density / energy.sqrt()
        } else {
            0.0
        }
    }
}

pub(super) fn build_spatial_from_components(
    layout: &UnisonLayout,
    settings: UnisonSettings,
    left: &mut [f32; MAX_UNISON],
    right: &mut [f32; MAX_UNISON],
) -> f32 {
    let voices = usize::from(settings.voices);
    let [alternate_weight, pair_weight, random_weight, shape_weight] =
        stereo_square_weights(settings.stereo_alternate, settings.stereo_x);
    let mut pan_positions = [0.0; MAX_UNISON];
    let mut weighted_pan = 0.0;
    let mut energy = 0.0;
    for index in 0..voices {
        let pan = alternate_weight.mul_add(
            layout.spatial_alternate[index],
            pair_weight.mul_add(
                layout.spatial_pair[index],
                random_weight.mul_add(
                    layout.spatial_random[index],
                    shape_weight * layout.spatial_shape[index],
                ),
            ),
        );
        let weight = unison_lane_weight(layout.detune_positions[index].abs(), settings.level_curve);
        pan_positions[index] = pan;
        right[index] = weight;
        weighted_pan = pan.mul_add(weight * weight, weighted_pan);
        energy += weight * weight;
    }
    let pan_center = weighted_pan / energy.max(f32::EPSILON);
    let pan_scale = pan_positions[..voices]
        .iter()
        .fold(0.0_f32, |maximum, pan| {
            maximum.max((*pan - pan_center).abs())
        })
        .max(f32::EPSILON)
        .recip();
    for index in 0..voices {
        let pan =
            ((pan_positions[index] - pan_center) * pan_scale * settings.stereo).clamp(-1.0, 1.0);
        let weight = right[index];
        left[index] = weight * (1.0 - pan).sqrt();
        right[index] = weight * (1.0 + pan).sqrt();
    }
    UnisonLayout::density(settings.voices) / energy.max(f32::EPSILON).sqrt()
}

fn build_spatial_from_prepared_components(
    prepared: &UnisonLayout,
    settings: UnisonSettings,
    random_seed: f32,
    left: &mut [f32; MAX_UNISON],
    right: &mut [f32; MAX_UNISON],
) -> f32 {
    let voices = usize::from(settings.voices);
    let [alternate_weight, pair_weight, random_weight, shape_weight] =
        stereo_square_weights(settings.stereo_alternate, settings.stereo_x);
    let mut pan_positions = [0.0; MAX_UNISON];
    let mut weighted_pan = 0.0;
    let mut energy = 0.0;
    for index in 0..voices {
        let pan = alternate_weight.mul_add(
            prepared.spatial_alternate[index],
            pair_weight.mul_add(
                prepared.spatial_pair[index],
                random_weight.mul_add(
                    stratified_random_pan(index, settings.voices, random_seed),
                    shape_weight * prepared.spatial_shape[index],
                ),
            ),
        );
        let weight =
            unison_lane_weight(prepared.detune_positions[index].abs(), settings.level_curve);
        pan_positions[index] = pan;
        right[index] = weight;
        weighted_pan = pan.mul_add(weight * weight, weighted_pan);
        energy += weight * weight;
    }
    let pan_center = weighted_pan / energy.max(f32::EPSILON);
    let pan_scale = pan_positions[..voices]
        .iter()
        .fold(0.0_f32, |maximum, pan| {
            maximum.max((*pan - pan_center).abs())
        })
        .max(f32::EPSILON)
        .recip();
    for index in 0..voices {
        let pan =
            ((pan_positions[index] - pan_center) * pan_scale * settings.stereo).clamp(-1.0, 1.0);
        let weight = right[index];
        left[index] = weight * (1.0 - pan).sqrt();
        right[index] = weight * (1.0 + pan).sqrt();
    }
    UnisonLayout::density(settings.voices) / energy.max(f32::EPSILON).sqrt()
}

#[inline]
pub(super) fn fill_unison_detune_positions(output: &mut [f32; MAX_UNISON], voices: u8, curve: f32) {
    let voices = voices.clamp(1, MAX_UNISON_U8);
    if voices <= 1 {
        return;
    }
    let core_count = usize::from(!voices.is_multiple_of(2));
    let pair_count = usize::from(voices - core_count as u8) / 2;
    let power = curve.clamp(-1.0, 1.0) * 5.0;
    let linear = power.abs() < 0.005;
    let denominator = (!linear).then(|| power.exp_m1()).unwrap_or(1.0);
    let exponential_step = (!linear)
        .then(|| (power / pair_count as f32).exp())
        .unwrap_or(1.0);
    let mut exponential = 1.0;
    for index in 0..usize::from(voices) {
        if index < core_count {
            output[index] = 0.0;
            continue;
        }
        let satellite = index - core_count;
        let pair = satellite / 2 + 1;
        let position = pair as f32 / pair_count as f32;
        let radius = if linear {
            position
        } else {
            if satellite.is_multiple_of(2) {
                exponential *= exponential_step;
            }
            (exponential - 1.0) / denominator
        };
        let sign = if satellite.is_multiple_of(2) {
            -1.0
        } else {
            1.0
        };
        output[index] = sign * radius;
    }
}

#[inline]
fn unison_lane_weight(radius: f32, level_curve: f32) -> f32 {
    let level_curve = level_curve.clamp(-1.0, 1.0);
    let profile = if level_curve < 0.0 {
        let center = 1.0 - radius;
        center * center * center * center
    } else {
        let sides = radius * radius;
        sides * sides
    };
    level_curve.abs().mul_add(profile - 1.0, 1.0)
}

fn unison_lane_stereo_components(
    voices: u8,
    index: usize,
    curve: f32,
    pan_shape: PanShapeSettings,
    random_seed: f32,
) -> (f32, f32, f32, f32, f32, f32) {
    let voices = voices.clamp(1, MAX_UNISON_U8);
    if voices == 1 {
        return (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    }
    let core_count = usize::from(!voices.is_multiple_of(2));
    if index < core_count {
        return (
            0.0,
            0.0,
            0.0,
            stratified_random_pan(index, voices, random_seed),
            0.0,
            0.0,
        );
    }
    let pair_count = usize::from(voices - core_count as u8) / 2;
    let satellite = index - core_count;
    let pair = satellite / 2 + 1;
    let radius = vital_detune_scale(pair as f32 / pair_count as f32, curve);
    let detune_sign = if satellite.is_multiple_of(2) {
        -1.0
    } else {
        1.0
    };
    let ring_sign = if pair.is_multiple_of(2) { -1.0 } else { 1.0 };
    let pair_pan = if pair_count == 1 {
        detune_sign
    } else {
        ring_sign
    };
    (
        detune_sign * radius,
        detune_sign * ring_sign,
        pair_pan,
        stratified_random_pan(index, voices, random_seed),
        detune_sign * ring_sign * pan_shape_curve_value_side(radius, detune_sign, pan_shape),
        radius,
    )
}

#[inline]
fn unison_lane_stereo_components_at_position(
    voices: u8,
    index: usize,
    detune_position: f32,
    pan_shape: PanShapeSettings,
    random_seed: f32,
) -> (f32, f32, f32, f32, f32, f32) {
    let voices = voices.clamp(1, MAX_UNISON_U8);
    if voices == 1 {
        return (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    }
    let core_count = usize::from(!voices.is_multiple_of(2));
    if index < core_count {
        return (
            0.0,
            0.0,
            0.0,
            stratified_random_pan(index, voices, random_seed),
            0.0,
            0.0,
        );
    }
    let pair_count = usize::from(voices - core_count as u8) / 2;
    let satellite = index - core_count;
    let pair = satellite / 2 + 1;
    let radius = detune_position.abs();
    let detune_sign = if satellite.is_multiple_of(2) {
        -1.0
    } else {
        1.0
    };
    let ring_sign = if pair.is_multiple_of(2) { -1.0 } else { 1.0 };
    let pair_pan = if pair_count == 1 {
        detune_sign
    } else {
        ring_sign
    };
    (
        detune_sign * radius,
        detune_sign * ring_sign,
        pair_pan,
        stratified_random_pan(index, voices, random_seed),
        detune_sign * ring_sign * pan_shape_curve_value_side(radius, detune_sign, pan_shape),
        radius,
    )
}

pub fn unison_lane_position_stereo_seeded(
    voices: u8,
    index: usize,
    curve: f32,
    stereo_alternate: f32,
    stereo_x: f32,
    level_curve: f32,
    pan_shape: PanShapeSettings,
    random_seed: f32,
) -> (f32, f32, f32) {
    let voices = voices.clamp(1, MAX_UNISON_U8);
    if voices == 1 {
        return (0.0, 0.0, 1.0);
    }

    let (structured_detune, alternate_pan, pair_pan, random_pan, shape_pan, radius) =
        unison_lane_stereo_components(voices, index, curve, pan_shape, random_seed);
    let [alternate_weight, pair_weight, random_weight, shape_weight] =
        stereo_square_weights(stereo_alternate, stereo_x);
    let detune = structured_detune;
    let pan = alternate_weight.mul_add(
        alternate_pan,
        pair_weight.mul_add(
            pair_pan,
            random_weight.mul_add(random_pan, shape_weight * shape_pan),
        ),
    );
    let weight = unison_lane_weight(radius, level_curve);
    (detune, pan, weight)
}

#[derive(Clone, Copy)]
struct UnisonStaticPitch {
    cents: f32,
    ratio: f32,
}

#[inline]
fn nearest_note_candidate(raw_cents: f32, range_cents: f32) -> AlignmentCandidate {
    let sign = raw_cents.signum();
    let range_cents = range_cents.max(0.0);
    let mut best = AlignmentCandidate {
        ratio: 1.0,
        cents: 0.0,
    };
    let mut best_distance = f32::INFINITY;
    for semitone in -48..=48 {
        let cents = semitone as f32 * 100.0;
        if cents.abs() > range_cents + ALIGNMENT_EPSILON
            || sign != 0.0 && cents * sign < -ALIGNMENT_EPSILON
        {
            continue;
        }
        let distance = (cents - raw_cents).abs();
        if distance < best_distance {
            best_distance = distance;
            best = AlignmentCandidate {
                ratio: (semitone as f32 / 12.0).exp2(),
                cents,
            };
        }
    }
    best
}

pub(super) fn build_harmonic_candidates(
    mode: UnisonAlignmentMode,
) -> ([AlignmentCandidate; HARMONIC_CANDIDATE_CAP], usize) {
    let mut candidates = [EMPTY_ALIGNMENT_CANDIDATE; HARMONIC_CANDIDATE_CAP];
    let mut count = 0;
    if mode == UnisonAlignmentMode::Note {
        for semitone in 0..=48 {
            let cents = semitone as f32 * 100.0;
            candidates[count] = AlignmentCandidate {
                ratio: (semitone as f32 / 12.0).exp2(),
                cents,
            };
            count += 1;
        }
    } else {
        for partial in 1..=HARMONIC_PARTIAL_LIMIT {
            if matches!(mode, UnisonAlignmentMode::Odd) && partial.is_multiple_of(2)
                || matches!(mode, UnisonAlignmentMode::Even) && !partial.is_multiple_of(2)
            {
                continue;
            }
            let divisor = 1_u32 << (31 - partial.leading_zeros());
            let base_ratio = partial as f32 / divisor as f32;
            for octave in 0..=HARMONIC_OCTAVE_LIMIT {
                let ratio = base_ratio * (1_u32 << octave) as f32;
                candidates[count] = AlignmentCandidate {
                    ratio,
                    cents: 1_200.0 * ratio.log2(),
                };
                count += 1;
            }
        }
    }
    for index in 1..count {
        let candidate = candidates[index];
        let mut insert = index;
        while insert > 0 && candidates[insert - 1].cents > candidate.cents {
            candidates[insert] = candidates[insert - 1];
            insert -= 1;
        }
        candidates[insert] = candidate;
    }
    (candidates, count)
}

#[inline]
pub(super) fn nearest_harmonic_candidate_lattice(
    raw_cents: f32,
    candidates: &[AlignmentCandidate; HARMONIC_CANDIDATE_CAP],
    upper: usize,
) -> AlignmentCandidate {
    let raw_abs = raw_cents.abs();
    if upper == 0 {
        return EMPTY_ALIGNMENT_CANDIDATE;
    }

    let mut low = 0;
    let mut high = upper;
    while low < high {
        let middle = (low + high) / 2;
        if candidates[middle].cents < raw_abs {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    let mut best = candidates[low.min(upper - 1)];
    let best_distance = (best.cents - raw_abs).abs();
    if low > 0 {
        let previous = candidates[low - 1];
        let distance = (previous.cents - raw_abs).abs();
        if distance < best_distance {
            best = previous;
        }
    }
    if raw_cents < 0.0 {
        best.ratio = best.ratio.recip();
        best.cents = -best.cents;
    }
    best
}

#[inline]
pub(super) fn harmonic_candidate_upper(
    range_cents: f32,
    candidates: &[AlignmentCandidate; HARMONIC_CANDIDATE_CAP],
    candidate_count: usize,
) -> usize {
    let range_cents = range_cents.max(0.0);
    let mut low = 0;
    let mut high = candidate_count;
    while low < high {
        let middle = (low + high) / 2;
        if candidates[middle].cents <= range_cents + ALIGNMENT_EPSILON {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    low
}

#[inline]
fn nearest_harmonic_candidate(
    raw_cents: f32,
    range_cents: f32,
    mode: UnisonAlignmentMode,
) -> AlignmentCandidate {
    let sign = raw_cents.signum();
    let range_cents = range_cents.max(0.0);
    let mut best = AlignmentCandidate {
        ratio: 1.0,
        cents: 0.0,
    };
    let mut best_distance = raw_cents.abs();

    for partial in 1..=HARMONIC_PARTIAL_LIMIT {
        if matches!(mode, UnisonAlignmentMode::Odd) && partial.is_multiple_of(2)
            || matches!(mode, UnisonAlignmentMode::Even) && !partial.is_multiple_of(2)
        {
            continue;
        }

        let divisor = 1_u32 << (31 - partial.leading_zeros());
        let base_ratio = partial as f32 / divisor as f32;
        for octave in 0..=HARMONIC_OCTAVE_LIMIT {
            let harmonic_ratio = base_ratio * (1_u32 << octave) as f32;
            let ratio = if sign < 0.0 {
                harmonic_ratio.recip()
            } else {
                harmonic_ratio
            };
            let cents = 1_200.0 * ratio.log2();
            if cents.abs() > range_cents + ALIGNMENT_EPSILON {
                continue;
            }

            let distance = (cents - raw_cents).abs();
            if distance < best_distance {
                best_distance = distance;
                best = AlignmentCandidate { ratio, cents };
            }
        }
    }
    best
}

#[inline]
fn nearest_alignment_candidate(
    raw_cents: f32,
    range_cents: f32,
    mode: UnisonAlignmentMode,
) -> AlignmentCandidate {
    match mode {
        UnisonAlignmentMode::Note => nearest_note_candidate(raw_cents, range_cents),
        _ => nearest_harmonic_candidate(raw_cents, range_cents, mode),
    }
}

#[inline]
fn unison_static_pitch(
    detune_position: f32,
    detune_cents: f32,
    detune_amount: f32,
    harmonic_align: f32,
    alignment_mode: UnisonAlignmentMode,
) -> UnisonStaticPitch {
    let detune_cents = detune_cents.max(0.0);
    let detune_amount = detune_amount.clamp(0.0, 1.0);
    let raw_cents = detune_position * detune_cents * detune_amount;
    let harmonic_align = harmonic_align.clamp(0.0, 1.0);
    if harmonic_align <= ALIGNMENT_EPSILON {
        return UnisonStaticPitch {
            cents: raw_cents,
            ratio: (raw_cents / 1_200.0).exp2(),
        };
    }

    let target =
        nearest_alignment_candidate(raw_cents, detune_cents * detune_amount, alignment_mode);
    let cents = raw_cents + harmonic_align * (target.cents - raw_cents);
    let ratio = if harmonic_align >= 1.0 {
        target.ratio
    } else {
        (cents / 1_200.0).exp2()
    };
    UnisonStaticPitch { cents, ratio }
}

#[inline]
pub(crate) fn unison_static_pitch_cents(
    detune_position: f32,
    detune_cents: f32,
    detune_amount: f32,
    harmonic_align: f32,
    alignment_mode: UnisonAlignmentMode,
) -> f32 {
    unison_static_pitch(
        detune_position,
        detune_cents,
        detune_amount,
        harmonic_align,
        alignment_mode,
    )
    .cents
}

#[inline]
pub(super) fn unison_static_pitch_ratio(
    detune_position: f32,
    detune_cents: f32,
    detune_amount: f32,
    harmonic_align: f32,
    alignment_mode: UnisonAlignmentMode,
) -> f32 {
    unison_static_pitch(
        detune_position,
        detune_cents,
        detune_amount,
        harmonic_align,
        alignment_mode,
    )
    .ratio
}

#[allow(
    clippy::too_many_arguments,
    clippy::cast_possible_truncation,
    reason = "the preview and DSP share one explicit scalar lane model"
)]
pub fn unison_lane_position_stereo_jitter_seeded(
    voices: u8,
    index: usize,
    curve: f32,
    stereo_alternate: f32,
    stereo_x: f32,
    level_curve: f32,
    pan_shape: PanShapeSettings,
    random_seed: f32,
    detune_amount: f32,
    harmonic_align: f32,
    alignment_mode: UnisonAlignmentMode,
    jitter_offset: f32,
    detune_cents: f32,
) -> (f32, f32, f32) {
    let base = unison_lane_position_stereo_seeded(
        voices,
        index,
        curve,
        stereo_alternate,
        stereo_x,
        level_curve,
        pan_shape,
        random_seed,
    );
    let pitch = unison_static_pitch(
        base.0,
        detune_cents,
        detune_amount,
        harmonic_align,
        alignment_mode,
    );
    (
        pitch.cents + jitter_offset * JITTER_EXCURSION_CENTS,
        base.1,
        base.2,
    )
}

/// Smooth deterministic pitch motion shared by the DSP and editor. Every lane
/// follows its own value-noise trajectory. Removing the instantaneous
/// stack mean keeps the perceived note centered without coupling pan or gain.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "the bounded preview clock and 64-lane index intentionally enter the deterministic hash"
)]
#[inline]
pub fn fill_unison_jitter_offsets(output: &mut [f32], seed: f32, amount: f32, time: f32) {
    fill_unison_jitter_offsets_mode(output, seed, amount, time, SwarmMode::Noise);
}

pub fn fill_extended_unison_jitter_offsets(output: &mut [f32], seed: f32, amount: f32, time: f32) {
    if output.len() == 1 && amount > f32::EPSILON {
        output[0] = unison_lane_jitter_raw(0, seed, time) * amount;
    } else {
        fill_unison_jitter_offsets(output, seed, amount, time);
    }
}

pub fn fill_unison_jitter_offsets_mode(
    output: &mut [f32],
    seed: f32,
    amount: f32,
    time: f32,
    mode: SwarmMode,
) {
    if output.len() <= 1 || amount <= f32::EPSILON {
        output.fill(0.0);
        return;
    }
    if mode == SwarmMode::Sine {
        const PHASE_STRIDE: f32 = 0.618_034;
        let phase = unit_hash(u64::from(seed.to_bits()) ^ 0x4a49_5454_4552_5349) as f32;
        for (index, value) in output.iter_mut().enumerate() {
            *value = fast_sine_cycle((index as f32).mul_add(PHASE_STRIDE, time.max(0.0) + phase));
        }
    } else {
        for (index, value) in output.iter_mut().enumerate() {
            *value = unison_lane_jitter_raw(index, seed, time);
        }
    }
    center_and_scale_jitter(output, amount);
}

fn center_and_scale_jitter(output: &mut [f32], amount: f32) {
    let sum = output.iter().sum::<f32>();
    let center = sum / output.len() as f32;
    let maximum = output.iter().fold(1.0_f32, |maximum, value| {
        maximum.max((*value - center).abs())
    });
    let scale = amount.clamp(0.0, 1.0) / maximum;
    for value in output {
        *value = (*value - center) * scale;
    }
}

pub(super) fn jitter_pitch_ratios(output: &mut [f32], offsets: &mut [f32], _mode: SwarmMode) {
    let cents_scale = JITTER_EXCURSION_CENTS / 1_200.0;
    for (ratio, &offset) in output.iter_mut().zip(offsets.iter()) {
        *ratio = fast_exp2(offset * cents_scale);
    }
}

#[inline]
fn fast_sine_cycle(phase: f32) -> f32 {
    let phase = phase - phase.floor();
    let folded = 0.25 - ((phase - 0.5).abs() - 0.25).abs();
    let folded2 = folded * folded;
    let folded4 = folded2 * folded2;
    let low = (-41.341_7_f32).mul_add(folded2, std::f32::consts::TAU);
    let middle = (-76.705_86_f32).mul_add(folded2, 81.605_25);
    let high = (-15.094_643_f32).mul_add(folded2, 42.058_693);
    let sine = folded * high.mul_add(folded4, middle).mul_add(folded4, low);
    if phase > 0.5 { -sine } else { sine }
}

#[inline]
fn unison_lane_jitter_raw(index: usize, seed: f32, time: f32) -> f32 {
    let lane_seed = motion_seed(seed, index);
    let phase = unit_hash(lane_seed ^ 0x4a49_5454_4552_5048) as f32;
    smooth_value_noise(lane_seed ^ 0x4a49_5454_4552_4c4f, time.max(0.0) + phase)
}

#[inline]
fn smooth_value_noise(seed: u64, time: f32) -> f32 {
    let absolute_cell = time.floor() as u64;
    let cell = absolute_cell & 4_095;
    let next = cell.wrapping_add(1) & 4_095;
    let fraction = time - absolute_cell as f32;
    let start = bipolar_hash(seed ^ cell.wrapping_mul(0x9e37_79b9_7f4a_7c15));
    let end = bipolar_hash(seed ^ next.wrapping_mul(0x9e37_79b9_7f4a_7c15));
    let smooth = fraction * fraction * (3.0 - 2.0 * fraction);
    (end - start).mul_add(smooth, start)
}

pub fn stereo_pattern_center_seeded(
    voices: u8,
    curve: f32,
    stereo_alternate: f32,
    stereo_x: f32,
    level_curve: f32,
    pan_shape: PanShapeSettings,
    random_seed: f32,
) -> (f32, f32) {
    let voices = voices.clamp(1, MAX_UNISON_U8);
    let mut weighted_pan = 0.0;
    let mut weight_sum = 0.0;
    for index in 0..usize::from(voices) {
        let (_, pan, weight) = unison_lane_position_stereo_seeded(
            voices,
            index,
            curve,
            stereo_alternate,
            stereo_x,
            level_curve,
            pan_shape,
            random_seed,
        );
        let energy = weight * weight;
        weighted_pan = pan.mul_add(energy, weighted_pan);
        weight_sum += energy;
    }
    let center = weighted_pan / weight_sum.max(f32::EPSILON);
    let mut maximum: f32 = 0.0;
    for index in 0..usize::from(voices) {
        let (_, pan, _) = unison_lane_position_stereo_seeded(
            voices,
            index,
            curve,
            stereo_alternate,
            stereo_x,
            level_curve,
            pan_shape,
            random_seed,
        );
        maximum = maximum.max((pan - center).abs());
    }
    (center, maximum.max(f32::EPSILON).recip())
}

#[inline]
fn motion_seed(seed: f32, index: usize) -> u64 {
    u64::from(seed.to_bits()).wrapping_add(
        (index as u64)
            .wrapping_mul(0xd6e8_feb8_6659_fd93)
            .wrapping_add(0x5357_4152_4d5f_4c46),
    )
}

#[inline]
#[allow(
    clippy::cast_possible_truncation,
    reason = "the deterministic unit hash intentionally enters the f32 realtime lane model"
)]
fn bipolar_hash(seed: u64) -> f32 {
    (unit_hash(seed) as f32).mul_add(2.0, -1.0)
}

pub(super) fn stereo_square_weights(vertical: f32, horizontal: f32) -> [f32; 4] {
    let vertical = vertical.clamp(0.0, 1.0);
    let horizontal = horizontal.clamp(0.0, 1.0);
    let left = 1.0 - horizontal;
    let bottom = 1.0 - vertical;
    [
        left * vertical,
        horizontal * vertical,
        left * bottom,
        horizontal * bottom,
    ]
}

#[inline]
pub fn pan_shape_curve_value_side(position: f32, side: f32, shape: PanShapeSettings) -> f32 {
    let signed_position = position.clamp(0.0, 1.0) * if side < 0.0 { -1.0 } else { 1.0 };
    let split = shape.center_x.mul_add(2.0, -1.0).clamp(-0.9, 0.9);
    if signed_position < split {
        let input = ((split - signed_position) / (split + 1.0)).clamp(0.0, 1.0);
        shape.left_segments.eval_fast(input)
    } else {
        let input = ((signed_position - split) / (1.0 - split)).clamp(0.0, 1.0);
        shape.right_segments.eval_fast(input)
    }
}

#[inline]
fn stratified_random_pan(index: usize, voices: u8, random_seed: f32) -> f32 {
    let voices = usize::from(voices.clamp(1, MAX_UNISON_U8));
    if voices == 1 {
        return 0.0;
    }
    let seed = u64::from(random_seed.to_bits());
    let rotation = unit_hash(seed ^ 0x5041_4e5f_524f_5441) as f32;
    let jitter = unit_hash(motion_seed(random_seed, index) ^ 0x5041_4e5f_4a49_5452) as f32;
    let position = ((index as f32 + jitter) / voices as f32 + rotation).fract();
    position.mul_add(2.0, -1.0)
}

/// Vital's detune-power curve, with its public -5..5 range normalized to -1..1.
fn vital_detune_scale(position: f32, curve: f32) -> f32 {
    let power = curve.clamp(-1.0, 1.0) * 5.0;
    if power.abs() < 0.005 {
        position
    } else {
        (power * position).exp_m1() / power.exp_m1()
    }
}

#[inline]
pub(super) fn extended_unison_position(voices: u8, index: usize, curve: f32) -> f32 {
    if voices <= 1 {
        return 0.0;
    }
    let position = (index as f32 / f32::from(voices - 1)).mul_add(2.0, -1.0);
    position.signum() * extended_detune_scale(position.abs(), curve)
}

#[inline]
pub(super) fn extended_unison_rate(normalized: f32) -> f32 {
    0.02 * fast_exp2(normalized.clamp(0.0, 1.0) * 12.287_712)
}

#[inline]
pub(crate) fn extended_detune_scale(position: f32, curve: f32) -> f32 {
    let position = position.clamp(0.0, 1.0);
    let bend = curve.abs().clamp(0.0, 1.0) * 4.0;
    if curve >= 0.0 {
        position / bend.mul_add(1.0 - position, 1.0)
    } else {
        position * (1.0 + bend) / bend.mul_add(position, 1.0)
    }
}
