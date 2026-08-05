//! Fixed-allocation polyphonic virtual-analog voice engine.

#[path = "internal_rt_pool.rs"]
mod internal_rt_pool;

pub use internal_rt_pool::{InternalRtPool, MAX_JOB_SAMPLES};

use crate::oscillator::{
    Antialiasing, PhaseWarpMode, VaOscillator, accumulate_saw4_block,
    accumulate_saw4_block_constant, accumulate_saw4_block_static_gains, accumulate_saw8_block,
    accumulate_saw8_block_constant, accumulate_saw8_block_static_gains,
    accumulate_shape4_block_constant, accumulate_shape8_block_constant, generate_pulse4,
    generate_pulse8, generate_saw4, generate_saw8, generate_shape4, generate_shape4_pair,
    generate_shape4_pair_warped, generate_shape4_warped, generate_shape8, generate_shape8_pair,
    generate_shape8_pair_warped, generate_shape8_warped, generate_sine4, generate_sine8,
    generate_spectral_saw8, generate_spectral_shape8, generate_triangle4, generate_triangle8,
};
use crate::pan_curve::{PanShapeCurveData, PanShapeSegmentsRt};
use truce_simd::{
    math::exp2_block,
    simd::{f32x4, f32x8},
};

pub const POLYPHONY: usize = 32;
pub const MAX_UNISON: usize = 64;
pub const OSCILLATOR_COUNT: usize = 3;
const POLYPHONY_U8: u8 = 32;
const MAX_UNISON_U8: u8 = 64;
const MASTER_HEADROOM: f32 = 0.8;
const OSCILLATOR_FADE_SECONDS: f32 = 0.005;
const CONFIGURATION_FADE_SECONDS: f32 = 0.005;
const SWARM_MIN_UPDATE_INTERVAL: u16 = 32;
const SWARM_MAX_UPDATE_INTERVAL: u16 = 1_024;
pub const BLOCK_INTERNAL_SAMPLES: usize = 32;
pub const FACTOR3_BLOCK_INTERNAL_SAMPLES: usize = 24;
#[allow(
    dead_code,
    reason = "legacy source compatibility for the old generator example"
)]
pub const WANDER_BLOCK_INTERNAL_SAMPLES: usize = BLOCK_INTERNAL_SAMPLES;

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

#[derive(Clone, Copy, Debug)]
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

    fn active(self) -> bool {
        self.mode != PhaseWarpMode::None && self.amount > f32::EPSILON
    }
}

#[derive(Clone, Copy, Debug)]
pub struct OscillatorSettings {
    pub enabled: bool,
    pub shape: f32,
    pub pulse_width: f32,
    pub pitch_ratio: f32,
    pub level: f32,
    pub pan: f32,
    pub phase_warp: PhaseWarpControl,
    left_gain: f32,
    right_gain: f32,
}

impl OscillatorSettings {
    pub fn new(
        enabled: bool,
        shape: f32,
        pulse_width: f32,
        pitch_ratio: f32,
        level: f32,
        pan: f32,
    ) -> Self {
        let level = level.clamp(0.0, 1.0);
        let pan = pan.clamp(-1.0, 1.0);
        Self {
            enabled,
            shape: shape.clamp(0.0, 3.0),
            pulse_width: pulse_width.clamp(0.03, 0.97),
            pitch_ratio: pitch_ratio.clamp(1.0 / 16.0, 16.0),
            level,
            pan,
            phase_warp: PhaseWarpControl::NONE,
            left_gain: level * (1.0 - pan).sqrt(),
            right_gain: level * (1.0 + pan).sqrt(),
        }
    }

    pub fn pitch_ratio(transpose: f32, cents: f32) -> f32 {
        ((transpose.clamp(-48.0, 48.0) + cents.clamp(-100.0, 100.0) * 0.01) / 12.0).exp2()
    }

    const fn legacy(shape: f32, pulse_width: f32) -> Self {
        Self {
            enabled: true,
            shape,
            pulse_width,
            pitch_ratio: 1.0,
            level: 1.0,
            pan: 0.0,
            phase_warp: PhaseWarpControl::NONE,
            left_gain: 1.0,
            right_gain: 1.0,
        }
    }

    const fn disabled() -> Self {
        Self {
            enabled: false,
            shape: 2.0,
            pulse_width: 0.5,
            pitch_ratio: 1.0,
            level: 1.0,
            pan: 0.0,
            phase_warp: PhaseWarpControl::NONE,
            left_gain: 1.0,
            right_gain: 1.0,
        }
    }

    fn channel_gains(self) -> (f32, f32) {
        (self.left_gain, self.right_gain)
    }

    pub const fn with_phase_warp(mut self, mode: PhaseWarpMode, amount: f32) -> Self {
        self.phase_warp = PhaseWarpControl::new(mode, amount);
        self
    }

    fn phase_warp_active(self) -> bool {
        self.phase_warp.active()
    }

    fn scale_level(&mut self, scale: f32) {
        self.level *= scale;
        self.left_gain *= scale;
        self.right_gain *= scale;
    }
}

#[derive(Clone, Copy, Debug)]
pub struct VoiceSettings {
    pub shape: f32,
    pub frequency_hz: f32,
    pub pulse_width: f32,
    pub velocity_amount: f32,
    pub pressure_amount: f32,
    pub timbre_amount: f32,
    pub antialiasing: Antialiasing,
    pub oscillators: [OscillatorSettings; OSCILLATOR_COUNT],
}

impl VoiceSettings {
    pub const fn new(
        shape: f32,
        frequency_hz: f32,
        pulse_width: f32,
        velocity_amount: f32,
        pressure_amount: f32,
        timbre_amount: f32,
    ) -> Self {
        Self {
            shape,
            frequency_hz,
            pulse_width,
            velocity_amount,
            pressure_amount,
            timbre_amount,
            antialiasing: Antialiasing::Legacy,
            oscillators: [
                OscillatorSettings::legacy(shape, pulse_width),
                OscillatorSettings::disabled(),
                OscillatorSettings::disabled(),
            ],
        }
    }

    pub const fn with_antialiasing(mut self, antialiasing: Antialiasing) -> Self {
        self.antialiasing = antialiasing;
        self
    }

    pub const fn with_oscillators(
        mut self,
        oscillators: [OscillatorSettings; OSCILLATOR_COUNT],
    ) -> Self {
        self.oscillators = oscillators;
        self
    }

    fn oscillator(self, index: usize) -> OscillatorSettings {
        let mut oscillator = self.oscillators[index];
        if index == 0 {
            oscillator.shape = self.shape;
            oscillator.pulse_width = self.pulse_width;
        }
        oscillator
    }

    fn has_secondary_oscillators(self) -> bool {
        self.oscillators[1..]
            .iter()
            .any(|oscillator| oscillator.enabled)
    }

    fn legacy_primary_fast_path(self) -> bool {
        let primary = self.oscillator(0);
        primary.enabled
            && !self.has_secondary_oscillators()
            && !primary.phase_warp_active()
            && primary.pitch_ratio.to_bits() == 1.0_f32.to_bits()
            && primary.level.to_bits() == 1.0_f32.to_bits()
            && primary.pan.to_bits() == 0.0_f32.to_bits()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnvelopeSettings {
    pub attack: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
    pub attack_curve: f32,
    pub decay_curve: f32,
    pub release_curve: f32,
    pub attack_curve_time: f32,
    pub decay_curve_time: f32,
    pub release_curve_time: f32,
}

impl Default for EnvelopeSettings {
    fn default() -> Self {
        Self {
            attack: 0.0,
            decay: 0.1,
            sustain: 1.0,
            release: 0.0,
            attack_curve: 0.0,
            decay_curve: 0.0,
            release_curve: 0.0,
            attack_curve_time: 0.0,
            decay_curve_time: 0.0,
            release_curve_time: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EnvelopeStage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
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
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnisonSettings {
    voices: u8,
    detune_cents: f32,
    stereo: f32,
    phase_random: f32,
    curve: f32,
    stereo_alternate: f32,
    stereo_x: f32,
    level_curve: f32,
    detune_amount: f32,
    pan_shape: PanShapeSettings,
    swarm_amount: f32,
    swarm_rate: f32,
    swarm_mode: SwarmMode,
}

impl UnisonSettings {
    pub fn new(voices: u8, detune_cents: f32, stereo: f32, phase_random: f32, curve: f32) -> Self {
        Self {
            voices: voices.clamp(1, MAX_UNISON_U8),
            detune_cents: detune_cents.clamp(0.0, 4_800.0),
            stereo: stereo.clamp(0.0, 1.0),
            phase_random: phase_random.clamp(0.0, 1.0),
            curve: curve.clamp(-1.0, 1.0),
            stereo_alternate: 1.0,
            stereo_x: 0.0,
            level_curve: 0.0,
            detune_amount: 1.0,
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

    pub const fn with_pan_shape(mut self, shape: PanShapeSettings) -> Self {
        self.pan_shape = shape;
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

    const fn motion_active(self) -> bool {
        self.voices > 1 && self.swarm_amount > f32::EPSILON && self.detune_cents > f32::EPSILON
    }
}

#[derive(Clone, Debug)]
struct UnisonLayout {
    settings: UnisonSettings,
    ratios: [f32; MAX_UNISON],
    left: [f32; MAX_UNISON],
    right: [f32; MAX_UNISON],
    gain: f32,
    random_seed: f32,
}

impl Default for UnisonLayout {
    fn default() -> Self {
        Self {
            settings: UnisonSettings::new(1, 0.0, 0.0, 1.0, 0.0),
            ratios: [1.0; MAX_UNISON],
            left: [1.0; MAX_UNISON],
            right: [1.0; MAX_UNISON],
            gain: 1.0,
            random_seed: 0.5,
        }
    }
}

impl UnisonLayout {
    fn configure(&mut self, settings: UnisonSettings) -> bool {
        let layout_changed = self.settings.voices != settings.voices
            || self.settings.detune_cents.to_bits() != settings.detune_cents.to_bits()
            || self.settings.stereo.to_bits() != settings.stereo.to_bits()
            || self.settings.curve.to_bits() != settings.curve.to_bits()
            || self.settings.stereo_alternate.to_bits() != settings.stereo_alternate.to_bits()
            || self.settings.stereo_x.to_bits() != settings.stereo_x.to_bits()
            || self.settings.level_curve.to_bits() != settings.level_curve.to_bits()
            || self.settings.detune_amount.to_bits() != settings.detune_amount.to_bits()
            || self.settings.pan_shape != settings.pan_shape;
        let motion_changed = self.settings.swarm_amount.to_bits()
            != settings.swarm_amount.to_bits()
            || self.settings.swarm_rate.to_bits() != settings.swarm_rate.to_bits()
            || self.settings.swarm_mode != settings.swarm_mode;
        self.settings.phase_random = settings.phase_random;
        if !layout_changed && !motion_changed {
            return false;
        }

        self.settings = settings;
        if layout_changed {
            self.rebuild();
        }
        true
    }

    fn set_random_seed(&mut self, random_seed: f32) {
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
        let settings = self.settings;
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
                self.random_seed,
            );
            self.ratios[index] =
                (detune_position * settings.detune_cents * settings.detune_amount / 1_200.0).exp2();
            self.left[index] = pan_position;
            self.right[index] = weight;
            let lane_energy = weight * weight;
            weighted_pan = pan_position.mul_add(lane_energy, weighted_pan);
            weight_sum += lane_energy;
            energy += lane_energy;
        }
        let pan_center = weighted_pan / weight_sum.max(f32::EPSILON);
        let pan_scale = self.left[..usize::from(settings.voices)]
            .iter()
            .fold(0.0_f32, |maximum, pan| {
                maximum.max((*pan - pan_center).abs())
            })
            .max(f32::EPSILON)
            .recip();
        for index in 0..usize::from(settings.voices) {
            let weight = self.right[index];
            let pan =
                ((self.left[index] - pan_center) * pan_scale * settings.stereo).clamp(-1.0, 1.0);
            self.left[index] = weight * (1.0 - pan).sqrt();
            self.right[index] = weight * (1.0 + pan).sqrt();
        }
        let density = 1.0 + 0.2 * f32::from(settings.voices - 1) / 63.0;
        self.gain = if energy > 0.0 {
            density / energy.sqrt()
        } else {
            0.0
        };
    }
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

    let core_count: u8 = u8::from(!voices.is_multiple_of(2));
    let (structured_detune, alternate_pan, pair_pan, shape_pan, radius) = if index
        < usize::from(core_count)
    {
        (0.0, 0.0, 0.0, 0.0, 0.0)
    } else {
        let pair_count = usize::from(voices - core_count) / 2;
        let satellite = index - usize::from(core_count);
        let pair = satellite / 2 + 1;
        #[allow(
            clippy::cast_precision_loss,
            reason = "the unison stack has at most 64 oscillators"
        )]
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
            detune_sign * ring_sign * pan_shape_curve_value_side(radius, detune_sign, pan_shape),
            radius,
        )
    };
    let [alternate_weight, pair_weight, random_weight, shape_weight] =
        stereo_square_weights(stereo_alternate, stereo_x);
    let random_pan = stratified_random_pan(index, voices, random_seed);
    let detune = structured_detune;
    let pan = alternate_weight.mul_add(
        alternate_pan,
        pair_weight.mul_add(
            pair_pan,
            random_weight.mul_add(random_pan, shape_weight * shape_pan),
        ),
    );
    let level_curve = level_curve.clamp(-1.0, 1.0);
    let weight_radius = radius;
    let profile = if level_curve < 0.0 {
        let center = 1.0 - weight_radius;
        center * center * center * center
    } else {
        let sides = weight_radius * weight_radius;
        sides * sides
    };
    let weight = level_curve.abs().mul_add(profile - 1.0, 1.0);
    (detune, pan, weight)
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
    jitter_offset: f32,
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
    (
        base.0.mul_add(detune_amount.clamp(0.0, 1.0), jitter_offset),
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

fn jitter_pitch_ratios(
    output: &mut [f32],
    offsets: &mut [f32],
    detune_cents: f32,
    mode: SwarmMode,
) {
    let cents_scale = detune_cents / 1_200.0;
    if mode == SwarmMode::Sine {
        for exponent in &mut *offsets {
            *exponent *= cents_scale;
        }
        exp2_block(output, offsets);
    } else {
        for (ratio, &offset) in output.iter_mut().zip(offsets.iter()) {
            *ratio = (offset * cents_scale).exp2();
        }
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

fn stereo_square_weights(vertical: f32, horizontal: f32) -> [f32; 4] {
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

const AUTOMATION_CONTINUITY_SECONDS: f32 = 0.0005;

#[derive(Clone, Copy, Debug, Default)]
struct OutputContinuity {
    last: (f32, f32),
    correction: (f32, f32),
    remaining: u16,
    pending: bool,
}

impl OutputContinuity {
    fn active(self) -> bool {
        self.pending || self.remaining != 0
    }

    const fn mark(&mut self) {
        self.pending = true;
    }

    fn process(&mut self, raw: (f32, f32), sample_rate: f32) -> (f32, f32) {
        if !self.active() {
            self.last = raw;
            return raw;
        }
        let samples = (sample_rate * AUTOMATION_CONTINUITY_SECONDS)
            .round()
            .clamp(1.0, f32::from(u16::MAX)) as u16;
        if self.pending {
            self.correction = (self.last.0 - raw.0, self.last.1 - raw.1);
            self.remaining = samples;
            self.pending = false;
        }
        let position = f32::from(self.remaining) / f32::from(samples);
        let weight = position * position * (3.0 - 2.0 * position);
        self.remaining = self.remaining.saturating_sub(1);
        let output = (
            self.correction.0.mul_add(weight, raw.0),
            self.correction.1.mul_add(weight, raw.1),
        );
        self.last = output;
        output
    }

    const fn track(&mut self, output: (f32, f32)) {
        self.last = output;
    }
}

pub struct VaVoice {
    oscillators: [[VaOscillator; MAX_UNISON]; OSCILLATOR_COUNT],
    unison: UnisonLayout,
    current_note: Option<u8>,
    voice_id: Option<i32>,
    channel: u8,
    age: u64,
    frequency_hz: f32,
    glide_target_hz: f32,
    glide_multiplier: f32,
    glide_remaining: u32,
    pitch_ratio: f32,
    sample_rate: f32,
    phase_steps: [f32; MAX_UNISON],
    phase_steps_dirty: bool,
    swarm_clock: f32,
    swarm_update_remaining: u16,
    swarm_pitch_step: [f32; MAX_UNISON],
    enabled_oscillator_mask: u8,
    note_seed: u64,
    velocity: f32,
    pressure: f32,
    timbre: f32,
    envelope_level: f32,
    envelope_start: f32,
    envelope_progress: f32,
    envelope_step: f32,
    stage: EnvelopeStage,
    held: bool,
    sustained: bool,
    envelope: EnvelopeSettings,
    secondary_unison: [UnisonLayout; OSCILLATOR_COUNT - 1],
    secondary_phase_steps: [[f32; MAX_UNISON]; OSCILLATOR_COUNT - 1],
    secondary_phase_steps_dirty: [bool; OSCILLATOR_COUNT - 1],
    secondary_swarm_clock: [f32; OSCILLATOR_COUNT - 1],
    secondary_swarm_update_remaining: [u16; OSCILLATOR_COUNT - 1],
    secondary_swarm_pitch_step: [[f32; MAX_UNISON]; OSCILLATOR_COUNT - 1],
    spectral_harmonic: [[u16; MAX_UNISON]; OSCILLATOR_COUNT],
    spectral_target: [[u16; MAX_UNISON]; OSCILLATOR_COUNT],
    spectral_remaining: [[u8; MAX_UNISON]; OSCILLATOR_COUNT],
    spectral_top_gain: [[f32; MAX_UNISON]; OSCILLATOR_COUNT],
    spectral_check_remaining: [u8; OSCILLATOR_COUNT],
    output_continuity: OutputContinuity,
}

impl Default for VaVoice {
    fn default() -> Self {
        Self {
            oscillators: std::array::from_fn(|_| std::array::from_fn(|_| VaOscillator::default())),
            unison: UnisonLayout::default(),
            current_note: None,
            voice_id: None,
            channel: 0,
            age: 0,
            frequency_hz: 110.0,
            glide_target_hz: 110.0,
            glide_multiplier: 1.0,
            glide_remaining: 0,
            pitch_ratio: 1.0,
            sample_rate: 44_100.0,
            phase_steps: [0.0; MAX_UNISON],
            phase_steps_dirty: true,
            swarm_clock: 0.0,
            swarm_update_remaining: 0,
            swarm_pitch_step: [0.0; MAX_UNISON],
            enabled_oscillator_mask: 1,
            note_seed: 0,
            velocity: 1.0,
            pressure: 0.0,
            timbre: 0.5,
            envelope_level: 0.0,
            envelope_start: 0.0,
            envelope_progress: 0.0,
            envelope_step: 1.0,
            stage: EnvelopeStage::Idle,
            held: false,
            sustained: false,
            envelope: EnvelopeSettings::default(),
            secondary_unison: std::array::from_fn(|_| UnisonLayout::default()),
            secondary_phase_steps: [[0.0; MAX_UNISON]; OSCILLATOR_COUNT - 1],
            secondary_phase_steps_dirty: [true; OSCILLATOR_COUNT - 1],
            secondary_swarm_clock: [0.0; OSCILLATOR_COUNT - 1],
            secondary_swarm_update_remaining: [0; OSCILLATOR_COUNT - 1],
            secondary_swarm_pitch_step: [[0.0; MAX_UNISON]; OSCILLATOR_COUNT - 1],
            spectral_harmonic: [[0; MAX_UNISON]; OSCILLATOR_COUNT],
            spectral_target: [[0; MAX_UNISON]; OSCILLATOR_COUNT],
            spectral_remaining: [[0; MAX_UNISON]; OSCILLATOR_COUNT],
            spectral_top_gain: [[1.0; MAX_UNISON]; OSCILLATOR_COUNT],
            spectral_check_remaining: [0; OSCILLATOR_COUNT],
            output_continuity: OutputContinuity::default(),
        }
    }
}

impl VaVoice {
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        let sample_rate = sample_rate.max(1.0);
        if self.sample_rate.to_bits() != sample_rate.to_bits() {
            self.swarm_update_remaining = 0;
            self.secondary_swarm_update_remaining.fill(0);
            self.spectral_check_remaining.fill(0);
        }
        self.sample_rate = sample_rate;
        self.refresh_envelope_step();
        self.phase_steps_dirty = true;
        self.secondary_phase_steps_dirty.fill(true);
    }

    pub fn reset(&mut self) {
        self.reset_oscillators();
        self.current_note = None;
        self.voice_id = None;
        self.frequency_hz = 110.0;
        self.glide_target_hz = 110.0;
        self.glide_multiplier = 1.0;
        self.glide_remaining = 0;
        self.pitch_ratio = 1.0;
        self.phase_steps.fill(0.0);
        self.phase_steps_dirty = true;
        self.secondary_phase_steps.fill([0.0; MAX_UNISON]);
        self.secondary_phase_steps_dirty.fill(true);
        self.swarm_clock = 0.0;
        self.secondary_swarm_clock.fill(0.0);
        self.reset_all_swarm_motion();
        self.note_seed = 0;
        self.velocity = 1.0;
        self.pressure = 0.0;
        self.timbre = 0.5;
        self.envelope_level = 0.0;
        self.envelope_start = 0.0;
        self.envelope_progress = 0.0;
        self.envelope_step = 1.0;
        self.stage = EnvelopeStage::Idle;
        self.held = false;
        self.sustained = false;
        self.output_continuity = OutputContinuity::default();
    }

    pub fn start(&mut self, note: u8, velocity: f32, channel: u8, voice_id: Option<i32>, age: u64) {
        self.current_note = Some(note);
        self.voice_id = voice_id;
        self.channel = channel.min(15);
        self.age = age;
        let seed = note_phase_seed(note, self.channel, voice_id, age);
        self.note_seed = seed;
        self.randomize_oscillators(seed);
        self.seed_enabled_unison_layouts(seed);
        self.reset_enabled_swarm_motion();
        self.frequency_hz = midi_note_to_hz(note);
        self.glide_target_hz = self.frequency_hz;
        self.glide_multiplier = 1.0;
        self.glide_remaining = 0;
        self.phase_steps_dirty = true;
        self.secondary_phase_steps_dirty.fill(true);
        self.velocity = velocity.clamp(0.0, 1.0);
        self.pressure = 0.0;
        self.timbre = 0.5;
        self.envelope_level = 0.0;
        self.output_continuity = OutputContinuity::default();
        self.begin_attack();
        self.held = true;
        self.sustained = false;
    }

    fn retrigger(&mut self, velocity: f32, voice_id: Option<i32>, age: u64) {
        self.voice_id = voice_id;
        self.age = age;
        let seed = note_phase_seed(self.current_note.unwrap_or(69), self.channel, voice_id, age);
        self.note_seed = seed;
        self.randomize_oscillators(seed);
        self.seed_enabled_unison_layouts(seed);
        self.reset_enabled_swarm_motion();
        self.velocity = velocity.clamp(0.0, 1.0);
        self.pressure = 0.0;
        self.output_continuity = OutputContinuity::default();
        self.begin_attack();
        self.held = true;
        self.sustained = false;
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::cast_sign_loss,
        reason = "the glide duration is clamped to five seconds at the bounded DSP sample rate"
    )]
    fn legato_to(
        &mut self,
        note: u8,
        velocity: f32,
        channel: u8,
        voice_id: Option<i32>,
        age: u64,
        glide_time: f32,
    ) {
        self.current_note = Some(note);
        self.voice_id = voice_id;
        self.channel = channel.min(15);
        self.age = age;
        self.note_seed = note_phase_seed(note, self.channel, voice_id, age);
        self.glide_target_hz = midi_note_to_hz(note);
        let samples = (glide_time.clamp(0.0, 5.0) * self.sample_rate).round() as u32;
        if samples == 0 || (self.glide_target_hz - self.frequency_hz).abs() <= f32::EPSILON {
            self.frequency_hz = self.glide_target_hz;
            self.glide_multiplier = 1.0;
            self.glide_remaining = 0;
            self.phase_steps_dirty = true;
            self.secondary_phase_steps_dirty.fill(true);
        } else {
            self.glide_multiplier =
                ((self.glide_target_hz / self.frequency_hz).log2() / samples as f32).exp2();
            self.glide_remaining = samples;
        }
        self.velocity = velocity.clamp(0.0, 1.0);
        self.pressure = 0.0;
        self.held = true;
        self.sustained = false;
    }

    const fn is_gliding(&self) -> bool {
        self.glide_remaining != 0
    }

    pub fn configure(&mut self, envelope: EnvelopeSettings) {
        if self.envelope == envelope {
            return;
        }
        let duration_changed = match self.stage {
            EnvelopeStage::Attack => self.envelope.attack.to_bits() != envelope.attack.to_bits(),
            EnvelopeStage::Decay => self.envelope.decay.to_bits() != envelope.decay.to_bits(),
            EnvelopeStage::Release => self.envelope.release.to_bits() != envelope.release.to_bits(),
            EnvelopeStage::Idle | EnvelopeStage::Sustain => false,
        };
        self.envelope = envelope;
        if duration_changed {
            self.refresh_envelope_step();
        }
    }

    pub fn configure_unison(&mut self, settings: UnisonSettings) -> bool {
        let voice_count_changed = self.unison.settings.voices != settings.voices;
        let mut previous_without_motion = self.unison.settings;
        previous_without_motion.swarm_amount = settings.swarm_amount;
        previous_without_motion.swarm_rate = settings.swarm_rate;
        previous_without_motion.swarm_mode = settings.swarm_mode;
        let motion_change_only = previous_without_motion == settings;
        let layout_changed = self.unison.configure(settings);
        self.phase_steps_dirty |=
            layout_changed && (!motion_change_only || !settings.motion_active());
        if layout_changed && !motion_change_only {
            if self.active() && !voice_count_changed {
                self.output_continuity.mark();
            }
            self.reset_swarm_motion();
        } else if motion_change_only && settings.motion_active() {
            self.swarm_update_remaining = self
                .swarm_update_remaining
                .min(self.swarm_update_interval());
        }
        layout_changed
    }

    pub fn configure_secondary_unison(
        &mut self,
        oscillator: usize,
        settings: UnisonSettings,
    ) -> bool {
        let index = oscillator - 1;
        let voice_count_changed = self.secondary_unison[index].settings.voices != settings.voices;
        let mut previous_without_motion = self.secondary_unison[index].settings;
        previous_without_motion.swarm_amount = settings.swarm_amount;
        previous_without_motion.swarm_rate = settings.swarm_rate;
        previous_without_motion.swarm_mode = settings.swarm_mode;
        let motion_change_only = previous_without_motion == settings;
        let layout_changed = self.secondary_unison[index].configure(settings);
        self.secondary_phase_steps_dirty[index] |=
            layout_changed && (!motion_change_only || !settings.motion_active());
        if layout_changed && !motion_change_only {
            if self.active() && !voice_count_changed {
                self.output_continuity.mark();
            }
            self.reset_secondary_swarm_motion(index);
        } else if motion_change_only && settings.motion_active() {
            self.secondary_swarm_update_remaining[index] = self.secondary_swarm_update_remaining
                [index]
                .min(self.secondary_swarm_update_interval(index));
        }
        layout_changed
    }

    const fn set_swarm_clock(&mut self, time: f32) {
        self.swarm_clock = time;
    }

    const fn set_secondary_swarm_clock(&mut self, oscillator: usize, time: f32) {
        self.secondary_swarm_clock[oscillator - 1] = time;
    }

    fn set_enabled_oscillator_mask(&mut self, mask: u8) {
        let mask = mask & ((1 << OSCILLATOR_COUNT) - 1);
        let newly_enabled = mask & !self.enabled_oscillator_mask;
        self.enabled_oscillator_mask = mask;
        if !self.active() || newly_enabled == 0 {
            return;
        }
        for oscillator in 0..OSCILLATOR_COUNT {
            if newly_enabled & (1 << oscillator) != 0 {
                self.seed_unison_layout(oscillator, self.note_seed);
                self.randomize_oscillator_bank(oscillator, self.note_seed);
                if oscillator == 0 {
                    self.phase_steps_dirty = true;
                    self.reset_swarm_motion();
                } else {
                    self.secondary_phase_steps_dirty[oscillator - 1] = true;
                    self.reset_secondary_swarm_motion(oscillator - 1);
                }
            }
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "the waveform-specialized SIMD banks share one phase and gain tail"
    )]
    pub fn render(
        &mut self,
        settings: VoiceSettings,
        sample_rate: f32,
        force_gate: bool,
    ) -> (f32, f32) {
        if force_gate && self.stage == EnvelopeStage::Idle {
            let seed = 0x4452_4f4e_452d_4b56;
            self.randomize_oscillators(seed);
            self.velocity = 1.0;
            self.pressure = 0.0;
            self.timbre = 0.5;
            self.begin_attack();
        }
        self.advance_envelope(sample_rate, force_gate);

        let primary = settings.oscillator(0);
        if primary.enabled && !force_gate && self.phase_steps_dirty {
            self.refresh_phase_steps();
        }
        if !force_gate {
            self.advance_glide();
        }
        if primary.enabled && self.unison.settings.motion_active() {
            self.advance_swarm();
        }
        let dynamic_base_step = force_gate
            .then(|| self.base_phase_step(settings.frequency_hz * self.pitch_ratio, sample_rate));
        let velocity_gain = settings
            .velocity_amount
            .clamp(0.0, 1.0)
            .mul_add(self.velocity - 1.0, 1.0);
        let pressure_gain = settings
            .pressure_amount
            .clamp(0.0, 1.0)
            .mul_add(self.pressure, 1.0);
        let amplitude = self.envelope_level * velocity_gain * pressure_gain;
        let shape = self.effective_shape(settings);
        let spectral_check = if primary.enabled && settings.antialiasing == Antialiasing::Spectral {
            if self.spectral_check_remaining[0] == 0 {
                self.spectral_check_remaining[0] = 31;
                true
            } else {
                self.spectral_check_remaining[0] -= 1;
                false
            }
        } else {
            false
        };
        let mut left;
        let mut right;
        let voice_count = if primary.enabled {
            usize::from(self.unison.settings.voices)
        } else {
            0
        };
        let mut index = 0;
        let mut left8 = f32x8::ZERO;
        let mut right8 = f32x8::ZERO;
        if shape <= f32::EPSILON {
            while index + 8 <= voice_count {
                let phase_steps = std::array::from_fn(|lane| {
                    self.oscillator_phase_step(index + lane, primary.pitch_ratio, dynamic_base_step)
                });
                let samples = if primary.phase_warp_active() {
                    generate_shape8_warped(
                        &mut self.oscillators[0][index..index + 8],
                        shape,
                        phase_steps,
                        primary.pulse_width,
                        settings.antialiasing,
                        primary.phase_warp.mode,
                        primary.phase_warp.amount,
                    )
                } else {
                    generate_sine8(&mut self.oscillators[0][index..index + 8], phase_steps)
                };
                let left_gains =
                    f32x8::from(std::array::from_fn(|lane| self.unison.left[index + lane]));
                let right_gains =
                    f32x8::from(std::array::from_fn(|lane| self.unison.right[index + lane]));
                left8 = samples.mul_add(left_gains, left8);
                right8 = samples.mul_add(right_gains, right8);
                index += 8;
            }
            left = left8.reduce_add();
            right = right8.reduce_add();

            let mut left4 = f32x4::ZERO;
            let mut right4 = f32x4::ZERO;
            while index + 4 <= voice_count {
                let phase_steps = std::array::from_fn(|lane| {
                    self.oscillator_phase_step(index + lane, primary.pitch_ratio, dynamic_base_step)
                });
                let samples4 = if primary.phase_warp_active() {
                    generate_shape4_warped(
                        &mut self.oscillators[0][index..index + 4],
                        shape,
                        phase_steps,
                        primary.pulse_width,
                        settings.antialiasing,
                        primary.phase_warp.mode,
                        primary.phase_warp.amount,
                    )
                } else {
                    generate_sine4(&mut self.oscillators[0][index..index + 4], phase_steps)
                };
                let left_gains = f32x4::from([
                    self.unison.left[index],
                    self.unison.left[index + 1],
                    self.unison.left[index + 2],
                    self.unison.left[index + 3],
                ]);
                let right_gains = f32x4::from([
                    self.unison.right[index],
                    self.unison.right[index + 1],
                    self.unison.right[index + 2],
                    self.unison.right[index + 3],
                ]);
                left4 = samples4.mul_add(left_gains, left4);
                right4 = samples4.mul_add(right_gains, right4);
                index += 4;
            }
            left += left4.reduce_add();
            right += right4.reduce_add();
        } else {
            while index + 8 <= voice_count {
                let phase_steps = std::array::from_fn(|lane| {
                    self.oscillator_phase_step(index + lane, primary.pitch_ratio, dynamic_base_step)
                });
                let oscillators = &mut self.oscillators[0][index..index + 8];
                let samples = if primary.phase_warp_active() {
                    generate_shape8_warped(
                        oscillators,
                        shape,
                        phase_steps,
                        primary.pulse_width,
                        settings.antialiasing,
                        primary.phase_warp.mode,
                        primary.phase_warp.amount,
                    )
                } else if settings.antialiasing == Antialiasing::Spectral
                    && (shape - 2.0).abs() <= f32::EPSILON
                {
                    generate_spectral_saw8(
                        oscillators,
                        &mut self.spectral_harmonic[0][index..index + 8],
                        &mut self.spectral_target[0][index..index + 8],
                        &mut self.spectral_remaining[0][index..index + 8],
                        &mut self.spectral_top_gain[0][index..index + 8],
                        phase_steps,
                        spectral_check,
                    )
                } else if settings.antialiasing == Antialiasing::Spectral {
                    generate_spectral_shape8(
                        oscillators,
                        &mut self.spectral_harmonic[0][index..index + 8],
                        &mut self.spectral_target[0][index..index + 8],
                        &mut self.spectral_remaining[0][index..index + 8],
                        shape,
                        phase_steps,
                        primary.pulse_width,
                        spectral_check,
                    )
                } else if (shape - 1.0).abs() <= f32::EPSILON {
                    generate_triangle8(oscillators, phase_steps, settings.antialiasing)
                } else if (shape - 2.0).abs() <= f32::EPSILON {
                    generate_saw8(oscillators, phase_steps, settings.antialiasing)
                } else if shape >= 3.0 - f32::EPSILON {
                    generate_pulse8(
                        oscillators,
                        phase_steps,
                        primary.pulse_width,
                        settings.antialiasing,
                    )
                } else {
                    generate_shape8(
                        oscillators,
                        shape,
                        phase_steps,
                        primary.pulse_width,
                        settings.antialiasing,
                    )
                };
                let left_gains =
                    f32x8::from(std::array::from_fn(|lane| self.unison.left[index + lane]));
                let right_gains =
                    f32x8::from(std::array::from_fn(|lane| self.unison.right[index + lane]));
                left8 = samples.mul_add(left_gains, left8);
                right8 = samples.mul_add(right_gains, right8);
                index += 8;
            }
            left = left8.reduce_add();
            right = right8.reduce_add();

            let mut left4 = f32x4::ZERO;
            let mut right4 = f32x4::ZERO;
            while index + 4 <= voice_count {
                let phase_steps = std::array::from_fn(|lane| {
                    self.oscillator_phase_step(index + lane, primary.pitch_ratio, dynamic_base_step)
                });
                let oscillators = &mut self.oscillators[0][index..index + 4];
                let samples = if primary.phase_warp_active() {
                    generate_shape4_warped(
                        oscillators,
                        shape,
                        phase_steps,
                        primary.pulse_width,
                        settings.antialiasing,
                        primary.phase_warp.mode,
                        primary.phase_warp.amount,
                    )
                } else if (shape - 1.0).abs() <= f32::EPSILON {
                    generate_triangle4(oscillators, phase_steps, settings.antialiasing)
                } else if (shape - 2.0).abs() <= f32::EPSILON {
                    generate_saw4(oscillators, phase_steps, settings.antialiasing)
                } else if shape >= 3.0 - f32::EPSILON {
                    generate_pulse4(
                        oscillators,
                        phase_steps,
                        primary.pulse_width,
                        settings.antialiasing,
                    )
                } else {
                    generate_shape4(
                        oscillators,
                        shape,
                        phase_steps,
                        primary.pulse_width,
                        settings.antialiasing,
                    )
                };
                let left_gains = f32x4::from([
                    self.unison.left[index],
                    self.unison.left[index + 1],
                    self.unison.left[index + 2],
                    self.unison.left[index + 3],
                ]);
                let right_gains = f32x4::from([
                    self.unison.right[index],
                    self.unison.right[index + 1],
                    self.unison.right[index + 2],
                    self.unison.right[index + 3],
                ]);
                left4 = samples.mul_add(left_gains, left4);
                right4 = samples.mul_add(right_gains, right4);
                index += 4;
            }
            left += left4.reduce_add();
            right += right4.reduce_add();
        }
        while index < voice_count {
            let phase_step =
                self.oscillator_phase_step(index, primary.pitch_ratio, dynamic_base_step);
            let sample = if primary.phase_warp_active() {
                self.oscillators[0][index].generate_shape_step_warped(
                    shape,
                    phase_step,
                    primary.pulse_width,
                    settings.antialiasing,
                    primary.phase_warp.mode,
                    primary.phase_warp.amount,
                )
            } else {
                self.oscillators[0][index].generate_shape_step(
                    shape,
                    phase_step,
                    primary.pulse_width,
                    settings.antialiasing,
                )
            };
            left = sample.mul_add(self.unison.left[index], left);
            right = sample.mul_add(self.unison.right[index], right);
            index += 1;
        }
        let (primary_left, primary_right) = if primary.enabled {
            primary.channel_gains()
        } else {
            (0.0, 0.0)
        };
        left *= primary_left;
        right *= primary_right;
        let gain = amplitude * self.unison.gain;
        let output = if !settings.oscillator(1).enabled && !settings.oscillator(2).enabled {
            (left * gain, right * gain)
        } else {
            let mut extra_left = 0.0;
            let mut extra_right = 0.0;
            for oscillator in 1..OSCILLATOR_COUNT {
                let (oscillator_left, oscillator_right) =
                    self.render_secondary_oscillator(settings, oscillator, dynamic_base_step);
                extra_left += oscillator_left;
                extra_right += oscillator_right;
            }
            (
                extra_left.mul_add(amplitude, left * gain),
                extra_right.mul_add(amplitude, right * gain),
            )
        };
        self.output_continuity.process(output, sample_rate)
    }

    #[allow(
        clippy::too_many_lines,
        reason = "two internal samples share oscillator, gain, and Swarm lane setup"
    )]
    pub fn render_pair(
        &mut self,
        settings: VoiceSettings,
        sample_rate: f32,
        swarm_clocks: [f32; 2],
    ) -> ([(f32, f32); 2], bool) {
        debug_assert!(self.active());
        debug_assert!(!self.is_gliding());
        debug_assert_ne!(settings.antialiasing, Antialiasing::Spectral);
        self.advance_envelope(sample_rate, false);
        let envelope0 = self.envelope_level;
        let render_second = self.active();
        if render_second {
            self.advance_envelope(sample_rate, false);
        }
        if self.phase_steps_dirty {
            self.refresh_phase_steps();
        }
        let swarm = self.unison.settings.motion_active();
        let mut first_swarm_frame_advanced = false;
        if swarm {
            if render_second {
                first_swarm_frame_advanced = self.prepare_swarm_pair(swarm_clocks);
            } else {
                self.set_swarm_clock(swarm_clocks[0]);
                self.advance_swarm();
            }
        }
        let envelope1 = if render_second {
            self.envelope_level
        } else {
            0.0
        };

        let velocity_gain = settings
            .velocity_amount
            .clamp(0.0, 1.0)
            .mul_add(self.velocity - 1.0, 1.0);
        let pressure_gain = settings
            .pressure_amount
            .clamp(0.0, 1.0)
            .mul_add(self.pressure, 1.0);
        let amplitude = [
            envelope0 * velocity_gain * pressure_gain,
            envelope1 * velocity_gain * pressure_gain,
        ];
        let shape = self.effective_shape(settings);
        let primary = settings.oscillator(0);
        let voice_count = usize::from(self.unison.settings.voices);
        let mut index = 0;
        let mut left8 = [f32x8::ZERO; 2];
        let mut right8 = [f32x8::ZERO; 2];
        while index + 8 <= voice_count {
            let phase_steps = if swarm {
                self.advance_jitter_phase_steps8_pair(
                    index,
                    render_second,
                    first_swarm_frame_advanced,
                )
            } else {
                let steps = std::array::from_fn(|lane| self.phase_steps[index + lane]);
                [steps, if render_second { steps } else { [0.0; 8] }]
            };
            let samples = if primary.phase_warp_active() {
                generate_shape8_pair_warped(
                    &mut self.oscillators[0][index..index + 8],
                    shape,
                    phase_steps,
                    primary.pulse_width,
                    settings.antialiasing,
                    primary.phase_warp.mode,
                    primary.phase_warp.amount,
                )
            } else {
                generate_shape8_pair(
                    &mut self.oscillators[0][index..index + 8],
                    shape,
                    phase_steps,
                    settings.pulse_width,
                    settings.antialiasing,
                )
            };
            let left0 = f32x8::from(std::array::from_fn(|lane| self.unison.left[index + lane]));
            let right0 = f32x8::from(std::array::from_fn(|lane| self.unison.right[index + lane]));
            left8[0] = samples[0].mul_add(left0, left8[0]);
            right8[0] = samples[0].mul_add(right0, right8[0]);
            left8[1] = samples[1].mul_add(left0, left8[1]);
            right8[1] = samples[1].mul_add(right0, right8[1]);
            index += 8;
        }
        let mut left = [left8[0].reduce_add(), left8[1].reduce_add()];
        let mut right = [right8[0].reduce_add(), right8[1].reduce_add()];

        let mut left4 = [f32x4::ZERO; 2];
        let mut right4 = [f32x4::ZERO; 2];
        while index + 4 <= voice_count {
            let phase_steps = if swarm {
                self.advance_jitter_phase_steps4_pair(
                    index,
                    render_second,
                    first_swarm_frame_advanced,
                )
            } else {
                let steps = std::array::from_fn(|lane| self.phase_steps[index + lane]);
                [steps, if render_second { steps } else { [0.0; 4] }]
            };
            let samples = if primary.phase_warp_active() {
                generate_shape4_pair_warped(
                    &mut self.oscillators[0][index..index + 4],
                    shape,
                    phase_steps,
                    primary.pulse_width,
                    settings.antialiasing,
                    primary.phase_warp.mode,
                    primary.phase_warp.amount,
                )
            } else {
                generate_shape4_pair(
                    &mut self.oscillators[0][index..index + 4],
                    shape,
                    phase_steps,
                    settings.pulse_width,
                    settings.antialiasing,
                )
            };
            let left0 = f32x4::from(std::array::from_fn(|lane| self.unison.left[index + lane]));
            let right0 = f32x4::from(std::array::from_fn(|lane| self.unison.right[index + lane]));
            left4[0] = samples[0].mul_add(left0, left4[0]);
            right4[0] = samples[0].mul_add(right0, right4[0]);
            left4[1] = samples[1].mul_add(left0, left4[1]);
            right4[1] = samples[1].mul_add(right0, right4[1]);
            index += 4;
        }
        for frame in 0..2 {
            left[frame] += left4[frame].reduce_add();
            right[frame] += right4[frame].reduce_add();
        }
        while index < voice_count {
            let phase_steps = if swarm {
                let phase_steps = self.advance_jitter_phase_steps_pair::<1>(
                    index,
                    render_second,
                    first_swarm_frame_advanced,
                );
                [phase_steps[0][0], phase_steps[1][0]]
            } else {
                [
                    self.phase_steps[index],
                    if render_second {
                        self.phase_steps[index]
                    } else {
                        0.0
                    },
                ]
            };
            let samples = if primary.phase_warp_active() {
                self.oscillators[0][index].generate_shape_step_pair_warped(
                    shape,
                    phase_steps,
                    primary.pulse_width,
                    settings.antialiasing,
                    primary.phase_warp.mode,
                    primary.phase_warp.amount,
                )
            } else {
                self.oscillators[0][index].generate_shape_step_pair(
                    shape,
                    phase_steps,
                    settings.pulse_width,
                    settings.antialiasing,
                )
            };
            for frame in 0..2 {
                left[frame] = samples[frame].mul_add(self.unison.left[index], left[frame]);
                right[frame] = samples[frame].mul_add(self.unison.right[index], right[frame]);
            }
            index += 1;
        }
        let gains = [
            amplitude[0] * self.unison.gain,
            amplitude[1] * self.unison.gain,
        ];
        let mut output = [
            (left[0] * gains[0], right[0] * gains[0]),
            (left[1] * gains[1], right[1] * gains[1]),
        ];
        output[0] = self.output_continuity.process(output[0], sample_rate);
        if render_second {
            output[1] = self.output_continuity.process(output[1], sample_rate);
        }
        (output, render_second)
    }

    pub fn render_saw_block<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        sample_rate: f32,
        swarm_clocks: [[f32; SAMPLES]; OSCILLATOR_COUNT],
    ) -> [(f32, f32); SAMPLES] {
        debug_assert!(self.active());
        debug_assert!(self.held);
        debug_assert!(!self.is_gliding());
        debug_assert_ne!(settings.antialiasing, Antialiasing::Spectral);
        debug_assert!(
            settings
                .oscillators
                .iter()
                .all(|oscillator| !oscillator.enabled || !oscillator.phase_warp_active()),
            "an active phase warp must never enter the unwarped block kernel"
        );
        let primary = settings.oscillator(0);
        let primary_shape = self.effective_shape(settings);
        debug_assert!((8..=BLOCK_INTERNAL_SAMPLES).contains(&SAMPLES));
        if primary.enabled && self.phase_steps_dirty {
            self.refresh_phase_steps();
        }

        let velocity_gain = settings
            .velocity_amount
            .clamp(0.0, 1.0)
            .mul_add(self.velocity - 1.0, 1.0);
        let pressure_gain = settings
            .pressure_amount
            .clamp(0.0, 1.0)
            .mul_add(self.pressure, 1.0);
        let mut amplitude = [0.0; SAMPLES];
        for value in &mut amplitude {
            self.advance_envelope(sample_rate, false);
            *value = self.envelope_level * velocity_gain * pressure_gain;
        }

        let voice_count = if primary.enabled {
            usize::from(self.unison.settings.voices)
        } else {
            0
        };
        let packs = voice_count / 8;
        let mut left = [f32x8::ZERO; SAMPLES];
        let mut right = [f32x8::ZERO; SAMPLES];
        if !primary.enabled || !self.unison.settings.motion_active() {
            for pack in 0..packs {
                let index = pack * 8;
                let steps = f32x8::from(std::array::from_fn(|lane| {
                    tuned_phase_step(self.phase_steps[index + lane], primary.pitch_ratio)
                }));
                let left_gain =
                    f32x8::from(std::array::from_fn(|lane| self.unison.left[index + lane]));
                let right_gain =
                    f32x8::from(std::array::from_fn(|lane| self.unison.right[index + lane]));
                if (primary_shape - 2.0).abs() <= f32::EPSILON {
                    accumulate_saw8_block_constant(
                        &mut self.oscillators[0][index..index + 8],
                        steps,
                        left_gain,
                        right_gain,
                        &mut left,
                        &mut right,
                        settings.antialiasing,
                    );
                } else {
                    accumulate_shape8_block_constant(
                        &mut self.oscillators[0][index..index + 8],
                        steps,
                        left_gain,
                        right_gain,
                        &mut left,
                        &mut right,
                        primary_shape,
                        primary.pulse_width,
                        settings.antialiasing,
                    );
                }
            }
            let mut tail_start = packs * 8;
            if voice_count - tail_start >= 4 {
                let steps = f32x4::from(std::array::from_fn(|lane| {
                    tuned_phase_step(self.phase_steps[tail_start + lane], primary.pitch_ratio)
                }));
                let left_gain = f32x4::from(std::array::from_fn(|lane| {
                    self.unison.left[tail_start + lane]
                }));
                let right_gain = f32x4::from(std::array::from_fn(|lane| {
                    self.unison.right[tail_start + lane]
                }));
                if (primary_shape - 2.0).abs() <= f32::EPSILON {
                    accumulate_saw4_block_constant(
                        &mut self.oscillators[0][tail_start..tail_start + 4],
                        steps,
                        left_gain,
                        right_gain,
                        &mut left,
                        &mut right,
                        settings.antialiasing,
                    );
                } else {
                    accumulate_shape4_block_constant(
                        &mut self.oscillators[0][tail_start..tail_start + 4],
                        steps,
                        left_gain,
                        right_gain,
                        &mut left,
                        &mut right,
                        primary_shape,
                        primary.pulse_width,
                        settings.antialiasing,
                    );
                }
                tail_start += 4;
            }
            for index in tail_start..voice_count {
                let phase_step = tuned_phase_step(self.phase_steps[index], primary.pitch_ratio);
                for frame in 0..SAMPLES {
                    let sample = self.oscillators[0][index].generate_shape_step(
                        primary_shape,
                        phase_step,
                        primary.pulse_width,
                        settings.antialiasing,
                    );
                    left[frame] += f32x8::splat(sample * self.unison.left[index] * 0.125);
                    right[frame] += f32x8::splat(sample * self.unison.right[index] * 0.125);
                }
            }
        } else {
            self.set_swarm_clock(swarm_clocks[0][0]);
            if self.swarm_update_remaining == 0 {
                let update_interval = self.swarm_update_interval();
                self.prepare_swarm_jitter_target(update_interval);
                self.swarm_update_remaining = update_interval;
            }
            let neutral_tune = primary.pitch_ratio.to_bits() == 1.0_f32.to_bits();
            let constant_ramp = usize::from(self.swarm_update_remaining) >= SAMPLES;
            let tuned_final_steps = if neutral_tune || !constant_ramp {
                None
            } else {
                constant_jitter_ramp_final::<SAMPLES>(
                    &self.phase_steps,
                    &self.swarm_pitch_step,
                    voice_count,
                    primary.pitch_ratio,
                )
            };
            if constant_ramp && (neutral_tune || tuned_final_steps.is_some()) {
                for pack in 0..packs {
                    let index = pack * 8;
                    let dynamic_step = f32x8::from(std::array::from_fn(|lane| {
                        tuned_phase_step(self.phase_steps[index + lane], primary.pitch_ratio)
                    }));
                    let delta = f32x8::from(std::array::from_fn(|lane| {
                        self.swarm_pitch_step[index + lane] * primary.pitch_ratio
                    }));
                    let left_gain =
                        f32x8::from(std::array::from_fn(|lane| self.unison.left[index + lane]));
                    let right_gain =
                        f32x8::from(std::array::from_fn(|lane| self.unison.right[index + lane]));
                    let final_steps: [f32; 8] = accumulate_saw8_block_static_gains(
                        &mut self.oscillators[0][index..index + 8],
                        dynamic_step,
                        delta,
                        left_gain,
                        right_gain,
                        &mut left,
                        &mut right,
                        settings.antialiasing,
                    )
                    .into();
                    if neutral_tune {
                        self.phase_steps[index..index + 8].copy_from_slice(&final_steps);
                    }
                }
                let mut tail_start = packs * 8;
                if voice_count - tail_start >= 4 {
                    let dynamic_step = f32x4::from(std::array::from_fn(|lane| {
                        tuned_phase_step(self.phase_steps[tail_start + lane], primary.pitch_ratio)
                    }));
                    let delta = f32x4::from(std::array::from_fn(|lane| {
                        self.swarm_pitch_step[tail_start + lane] * primary.pitch_ratio
                    }));
                    let left_gain = f32x4::from(std::array::from_fn(|lane| {
                        self.unison.left[tail_start + lane]
                    }));
                    let right_gain = f32x4::from(std::array::from_fn(|lane| {
                        self.unison.right[tail_start + lane]
                    }));
                    let final_steps: [f32; 4] = accumulate_saw4_block_static_gains(
                        &mut self.oscillators[0][tail_start..tail_start + 4],
                        dynamic_step,
                        delta,
                        left_gain,
                        right_gain,
                        &mut left,
                        &mut right,
                        settings.antialiasing,
                    )
                    .into();
                    if neutral_tune {
                        self.phase_steps[tail_start..tail_start + 4].copy_from_slice(&final_steps);
                    }
                    tail_start += 4;
                }
                for index in tail_start..voice_count {
                    let mut phase_step =
                        tuned_phase_step(self.phase_steps[index], primary.pitch_ratio);
                    let phase_step_delta = self.swarm_pitch_step[index] * primary.pitch_ratio;
                    for frame in 0..SAMPLES {
                        phase_step += phase_step_delta;
                        let sample = self.oscillators[0][index].generate_shape_step(
                            2.0,
                            phase_step,
                            primary.pulse_width,
                            settings.antialiasing,
                        );
                        left[frame] += f32x8::splat(sample * self.unison.left[index] * 0.125);
                        right[frame] += f32x8::splat(sample * self.unison.right[index] * 0.125);
                    }
                    if neutral_tune {
                        self.phase_steps[index] = phase_step;
                    }
                }
                if let Some(final_steps) = tuned_final_steps {
                    self.phase_steps[..voice_count].copy_from_slice(&final_steps[..voice_count]);
                }
                self.swarm_update_remaining -= SAMPLES as u16;
                self.set_swarm_clock(swarm_clocks[0][SAMPLES - 2]);
                let output = std::array::from_fn(|frame| {
                    let gain = amplitude[frame] * self.unison.gain;
                    (
                        left[frame].reduce_add() * gain,
                        right[frame].reduce_add() * gain,
                    )
                });
                return self.finish_saw_block(output, &amplitude, settings, &swarm_clocks);
            }

            let mut steps = [[f32x8::ZERO; SAMPLES]; MAX_UNISON / 8];
            let mut tail_steps = [[0.0_f32; SAMPLES]; 7];
            let mut left_gains = [[f32x8::ZERO; SAMPLES]; MAX_UNISON / 8];
            let mut right_gains = [[f32x8::ZERO; SAMPLES]; MAX_UNISON / 8];
            for frame in 0..SAMPLES {
                self.set_swarm_clock(swarm_clocks[0][frame]);
                self.advance_swarm();
                for pack in 0..packs {
                    let index = pack * 8;
                    steps[pack][frame] = f32x8::from(std::array::from_fn(|lane| {
                        tuned_phase_step(
                            self.lane_phase_step(index + lane, None),
                            primary.pitch_ratio,
                        )
                    }));
                    left_gains[pack][frame] =
                        f32x8::from(std::array::from_fn(|lane| self.unison.left[index + lane]));
                    right_gains[pack][frame] =
                        f32x8::from(std::array::from_fn(|lane| self.unison.right[index + lane]));
                }
                for (tail, index) in (packs * 8..voice_count).enumerate() {
                    tail_steps[tail][frame] =
                        tuned_phase_step(self.lane_phase_step(index, None), primary.pitch_ratio);
                }
            }
            for pack in 0..packs {
                let index = pack * 8;
                accumulate_saw8_block(
                    &mut self.oscillators[0][index..index + 8],
                    steps[pack],
                    left_gains[pack],
                    right_gains[pack],
                    &mut left,
                    &mut right,
                    settings.antialiasing,
                );
            }
            let mut tail_start = packs * 8;
            if voice_count - tail_start >= 4 {
                let steps4 = std::array::from_fn(|frame| {
                    f32x4::from(std::array::from_fn(|lane| tail_steps[lane][frame]))
                });
                let left_gain = f32x4::from(std::array::from_fn(|lane| {
                    self.unison.left[tail_start + lane]
                }));
                let right_gain = f32x4::from(std::array::from_fn(|lane| {
                    self.unison.right[tail_start + lane]
                }));
                accumulate_saw4_block(
                    &mut self.oscillators[0][tail_start..tail_start + 4],
                    steps4,
                    left_gain,
                    right_gain,
                    &mut left,
                    &mut right,
                    settings.antialiasing,
                );
                tail_start += 4;
            }
            for (tail, index) in (tail_start..voice_count).enumerate() {
                let tail = tail + tail_start - packs * 8;
                for frame in 0..SAMPLES {
                    let sample = self.oscillators[0][index].generate_shape_step(
                        2.0,
                        tail_steps[tail][frame],
                        primary.pulse_width,
                        settings.antialiasing,
                    );
                    left[frame] += f32x8::splat(sample * self.unison.left[index] * 0.125);
                    right[frame] += f32x8::splat(sample * self.unison.right[index] * 0.125);
                }
            }
        }

        let output = std::array::from_fn(|frame| {
            let gain = amplitude[frame] * self.unison.gain;
            (
                left[frame].reduce_add() * gain,
                right[frame].reduce_add() * gain,
            )
        });
        self.finish_saw_block(output, &amplitude, settings, &swarm_clocks)
    }

    fn render_generic_block<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        sample_rate: f32,
        swarm_clocks: [[f32; SAMPLES]; OSCILLATOR_COUNT],
    ) -> [(f32, f32); SAMPLES] {
        std::array::from_fn(|frame| {
            if settings.oscillator(0).enabled {
                self.set_swarm_clock(swarm_clocks[0][frame]);
            }
            for oscillator in 1..OSCILLATOR_COUNT {
                if settings.oscillator(oscillator).enabled {
                    self.set_secondary_swarm_clock(oscillator, swarm_clocks[oscillator][frame]);
                }
            }
            self.render(settings, sample_rate, false)
        })
    }

    fn finish_saw_block<const SAMPLES: usize>(
        &mut self,
        mut output: [(f32, f32); SAMPLES],
        amplitude: &[f32; SAMPLES],
        settings: VoiceSettings,
        swarm_clocks: &[[f32; SAMPLES]; OSCILLATOR_COUNT],
    ) -> [(f32, f32); SAMPLES] {
        let primary = settings.oscillator(0);
        if primary.enabled
            && (primary.level.to_bits() != 1.0_f32.to_bits()
                || primary.pan.to_bits() != 0.0_f32.to_bits())
        {
            let (left, right) = primary.channel_gains();
            for sample in &mut output {
                sample.0 *= left;
                sample.1 *= right;
            }
        }
        for oscillator in 1..OSCILLATOR_COUNT {
            if settings.oscillator(oscillator).enabled {
                self.mix_secondary_saw_block(
                    &mut output,
                    amplitude,
                    settings,
                    oscillator,
                    &swarm_clocks[oscillator],
                );
            }
        }
        self.output_continuity.track(output[SAMPLES - 1]);
        output
    }

    fn mix_secondary_saw_block<const SAMPLES: usize>(
        &mut self,
        output: &mut [(f32, f32); SAMPLES],
        amplitude: &[f32; SAMPLES],
        settings: VoiceSettings,
        oscillator_index: usize,
        swarm_clocks: &[f32; SAMPLES],
    ) {
        let oscillator = settings.oscillator(oscillator_index);
        let secondary = oscillator_index - 1;
        let shape = self.effective_oscillator_shape(settings, oscillator_index);
        if self.secondary_phase_steps_dirty[secondary] {
            self.refresh_secondary_phase_steps(secondary);
        }
        let unison_settings = self.secondary_unison[secondary].settings;
        let unison_gain = self.secondary_unison[secondary].gain;
        let voice_count = usize::from(unison_settings.voices);
        let packs = voice_count / 8;
        let has_simd4_tail = voice_count % 8 >= 4;
        let mut left = [f32x8::ZERO; SAMPLES];
        let mut right = [f32x8::ZERO; SAMPLES];
        if !unison_settings.motion_active() {
            for pack in 0..packs {
                let index = pack * 8;
                let steps = f32x8::from(std::array::from_fn(|lane| {
                    self.secondary_oscillator_phase_step(
                        secondary,
                        index + lane,
                        oscillator.pitch_ratio,
                        None,
                    )
                }));
                let left_gain = f32x8::from(std::array::from_fn(|lane| {
                    self.secondary_unison[secondary].left[index + lane]
                }));
                let right_gain = f32x8::from(std::array::from_fn(|lane| {
                    self.secondary_unison[secondary].right[index + lane]
                }));
                if (shape - 2.0).abs() <= f32::EPSILON {
                    accumulate_saw8_block_constant(
                        &mut self.oscillators[oscillator_index][index..index + 8],
                        steps,
                        left_gain,
                        right_gain,
                        &mut left,
                        &mut right,
                        settings.antialiasing,
                    );
                } else {
                    accumulate_shape8_block_constant(
                        &mut self.oscillators[oscillator_index][index..index + 8],
                        steps,
                        left_gain,
                        right_gain,
                        &mut left,
                        &mut right,
                        shape,
                        oscillator.pulse_width,
                        settings.antialiasing,
                    );
                }
            }
            let mut tail_start = packs * 8;
            if has_simd4_tail {
                let steps = f32x4::from(std::array::from_fn(|lane| {
                    self.secondary_oscillator_phase_step(
                        secondary,
                        tail_start + lane,
                        oscillator.pitch_ratio,
                        None,
                    )
                }));
                let left_gain = f32x4::from(std::array::from_fn(|lane| {
                    self.secondary_unison[secondary].left[tail_start + lane]
                }));
                let right_gain = f32x4::from(std::array::from_fn(|lane| {
                    self.secondary_unison[secondary].right[tail_start + lane]
                }));
                if (shape - 2.0).abs() <= f32::EPSILON {
                    accumulate_saw4_block_constant(
                        &mut self.oscillators[oscillator_index][tail_start..tail_start + 4],
                        steps,
                        left_gain,
                        right_gain,
                        &mut left,
                        &mut right,
                        settings.antialiasing,
                    );
                } else {
                    accumulate_shape4_block_constant(
                        &mut self.oscillators[oscillator_index][tail_start..tail_start + 4],
                        steps,
                        left_gain,
                        right_gain,
                        &mut left,
                        &mut right,
                        shape,
                        oscillator.pulse_width,
                        settings.antialiasing,
                    );
                }
                tail_start += 4;
            }
            for index in tail_start..voice_count {
                let phase_step = self.secondary_oscillator_phase_step(
                    secondary,
                    index,
                    oscillator.pitch_ratio,
                    None,
                );
                for frame in 0..SAMPLES {
                    let sample = self.oscillators[oscillator_index][index].generate_shape_step(
                        shape,
                        phase_step,
                        oscillator.pulse_width,
                        settings.antialiasing,
                    );
                    left[frame] +=
                        f32x8::splat(sample * self.secondary_unison[secondary].left[index] * 0.125);
                    right[frame] += f32x8::splat(
                        sample * self.secondary_unison[secondary].right[index] * 0.125,
                    );
                }
            }
        } else {
            self.set_secondary_swarm_clock(oscillator_index, swarm_clocks[0]);
            if self.secondary_swarm_update_remaining[secondary] == 0 {
                let update_interval = self.secondary_swarm_update_interval(secondary);
                self.prepare_secondary_swarm_jitter_target(secondary, update_interval);
                self.secondary_swarm_update_remaining[secondary] = update_interval;
            }
            let neutral_tune = oscillator.pitch_ratio.to_bits() == 1.0_f32.to_bits();
            let constant_ramp =
                usize::from(self.secondary_swarm_update_remaining[secondary]) >= SAMPLES;
            let tuned_final_steps = if neutral_tune || !constant_ramp {
                None
            } else {
                constant_jitter_ramp_final::<SAMPLES>(
                    &self.secondary_phase_steps[secondary],
                    &self.secondary_swarm_pitch_step[secondary],
                    usize::from(unison_settings.voices),
                    oscillator.pitch_ratio,
                )
            };
            if constant_ramp && (neutral_tune || tuned_final_steps.is_some()) {
                for pack in 0..packs {
                    let index = pack * 8;
                    let dynamic_step = f32x8::from(std::array::from_fn(|lane| {
                        tuned_phase_step(
                            self.secondary_phase_steps[secondary][index + lane],
                            oscillator.pitch_ratio,
                        )
                    }));
                    let delta = f32x8::from(std::array::from_fn(|lane| {
                        self.secondary_swarm_pitch_step[secondary][index + lane]
                            * oscillator.pitch_ratio
                    }));
                    let left_gain = f32x8::from(std::array::from_fn(|lane| {
                        self.secondary_unison[secondary].left[index + lane]
                    }));
                    let right_gain = f32x8::from(std::array::from_fn(|lane| {
                        self.secondary_unison[secondary].right[index + lane]
                    }));
                    let final_steps: [f32; 8] = accumulate_saw8_block_static_gains(
                        &mut self.oscillators[oscillator_index][index..index + 8],
                        dynamic_step,
                        delta,
                        left_gain,
                        right_gain,
                        &mut left,
                        &mut right,
                        settings.antialiasing,
                    )
                    .into();
                    if neutral_tune {
                        self.secondary_phase_steps[secondary][index..index + 8]
                            .copy_from_slice(&final_steps);
                    }
                }
                let mut tail_start = packs * 8;
                if has_simd4_tail {
                    let dynamic_step = f32x4::from(std::array::from_fn(|lane| {
                        tuned_phase_step(
                            self.secondary_phase_steps[secondary][tail_start + lane],
                            oscillator.pitch_ratio,
                        )
                    }));
                    let delta = f32x4::from(std::array::from_fn(|lane| {
                        self.secondary_swarm_pitch_step[secondary][tail_start + lane]
                            * oscillator.pitch_ratio
                    }));
                    let left_gain = f32x4::from(std::array::from_fn(|lane| {
                        self.secondary_unison[secondary].left[tail_start + lane]
                    }));
                    let right_gain = f32x4::from(std::array::from_fn(|lane| {
                        self.secondary_unison[secondary].right[tail_start + lane]
                    }));
                    let final_steps: [f32; 4] = accumulate_saw4_block_static_gains(
                        &mut self.oscillators[oscillator_index][tail_start..tail_start + 4],
                        dynamic_step,
                        delta,
                        left_gain,
                        right_gain,
                        &mut left,
                        &mut right,
                        settings.antialiasing,
                    )
                    .into();
                    if neutral_tune {
                        self.secondary_phase_steps[secondary][tail_start..tail_start + 4]
                            .copy_from_slice(&final_steps);
                    }
                    tail_start += 4;
                }
                for index in tail_start..voice_count {
                    let mut phase_step = tuned_phase_step(
                        self.secondary_phase_steps[secondary][index],
                        oscillator.pitch_ratio,
                    );
                    let phase_step_delta =
                        self.secondary_swarm_pitch_step[secondary][index] * oscillator.pitch_ratio;
                    for frame in 0..SAMPLES {
                        phase_step += phase_step_delta;
                        let sample = self.oscillators[oscillator_index][index].generate_shape_step(
                            2.0,
                            phase_step,
                            oscillator.pulse_width,
                            settings.antialiasing,
                        );
                        left[frame] += f32x8::splat(
                            sample * self.secondary_unison[secondary].left[index] * 0.125,
                        );
                        right[frame] += f32x8::splat(
                            sample * self.secondary_unison[secondary].right[index] * 0.125,
                        );
                    }
                    if neutral_tune {
                        self.secondary_phase_steps[secondary][index] = phase_step;
                    }
                }
                if let Some(final_steps) = tuned_final_steps {
                    let voices = usize::from(unison_settings.voices);
                    self.secondary_phase_steps[secondary][..voices]
                        .copy_from_slice(&final_steps[..voices]);
                }
                self.secondary_swarm_update_remaining[secondary] -= SAMPLES as u16;
                self.set_secondary_swarm_clock(oscillator_index, swarm_clocks[SAMPLES - 2]);
            } else {
                let mut steps = [[f32x8::ZERO; SAMPLES]; MAX_UNISON / 8];
                let mut tail_steps = [[0.0_f32; SAMPLES]; 7];
                let mut left_gains = [[f32x8::ZERO; SAMPLES]; MAX_UNISON / 8];
                let mut right_gains = [[f32x8::ZERO; SAMPLES]; MAX_UNISON / 8];
                for frame in 0..SAMPLES {
                    self.set_secondary_swarm_clock(oscillator_index, swarm_clocks[frame]);
                    self.advance_secondary_swarm(secondary);
                    for pack in 0..packs {
                        let index = pack * 8;
                        steps[pack][frame] = f32x8::from(std::array::from_fn(|lane| {
                            self.secondary_oscillator_phase_step(
                                secondary,
                                index + lane,
                                oscillator.pitch_ratio,
                                None,
                            )
                        }));
                        left_gains[pack][frame] = f32x8::from(std::array::from_fn(|lane| {
                            self.secondary_unison[secondary].left[index + lane]
                        }));
                        right_gains[pack][frame] = f32x8::from(std::array::from_fn(|lane| {
                            self.secondary_unison[secondary].right[index + lane]
                        }));
                    }
                    for (tail, index) in
                        (packs * 8..usize::from(unison_settings.voices)).enumerate()
                    {
                        tail_steps[tail][frame] = self.secondary_oscillator_phase_step(
                            secondary,
                            index,
                            oscillator.pitch_ratio,
                            None,
                        );
                    }
                }
                for pack in 0..packs {
                    let index = pack * 8;
                    accumulate_saw8_block(
                        &mut self.oscillators[oscillator_index][index..index + 8],
                        steps[pack],
                        left_gains[pack],
                        right_gains[pack],
                        &mut left,
                        &mut right,
                        settings.antialiasing,
                    );
                }
                let mut tail_start = packs * 8;
                if has_simd4_tail {
                    let steps4 = std::array::from_fn(|frame| {
                        f32x4::from(std::array::from_fn(|lane| tail_steps[lane][frame]))
                    });
                    let left_gain = f32x4::from(std::array::from_fn(|lane| {
                        self.secondary_unison[secondary].left[tail_start + lane]
                    }));
                    let right_gain = f32x4::from(std::array::from_fn(|lane| {
                        self.secondary_unison[secondary].right[tail_start + lane]
                    }));
                    accumulate_saw4_block(
                        &mut self.oscillators[oscillator_index][tail_start..tail_start + 4],
                        steps4,
                        left_gain,
                        right_gain,
                        &mut left,
                        &mut right,
                        settings.antialiasing,
                    );
                    tail_start += 4;
                }
                for (tail, index) in (tail_start..voice_count).enumerate() {
                    let tail = tail + tail_start - packs * 8;
                    for frame in 0..SAMPLES {
                        let sample = self.oscillators[oscillator_index][index].generate_shape_step(
                            2.0,
                            tail_steps[tail][frame],
                            oscillator.pulse_width,
                            settings.antialiasing,
                        );
                        left[frame] += f32x8::splat(
                            sample * self.secondary_unison[secondary].left[index] * 0.125,
                        );
                        right[frame] += f32x8::splat(
                            sample * self.secondary_unison[secondary].right[index] * 0.125,
                        );
                    }
                }
            }
        }
        let (channel_left, channel_right) = oscillator.channel_gains();
        for frame in 0..SAMPLES {
            let gain = amplitude[frame] * unison_gain;
            output[frame].0 += left[frame].reduce_add() * (gain * channel_left);
            output[frame].1 += right[frame].reduce_add() * (gain * channel_right);
        }
    }

    #[inline]
    fn advance_jitter_phase_steps8_pair(
        &mut self,
        index: usize,
        render_second: bool,
        first_frame_advanced: bool,
    ) -> [[f32; 8]; 2] {
        let current = std::array::from_fn(|lane| self.phase_steps[index + lane]);
        if !render_second {
            return [current, [0.0; 8]];
        }
        let current = f32x8::from(current);
        let step = f32x8::from(std::array::from_fn(|lane| {
            self.swarm_pitch_step[index + lane]
        }));
        let first = if first_frame_advanced {
            current
        } else {
            current + step
        };
        let second = first + step;
        let second_array: [f32; 8] = second.into();
        self.phase_steps[index..index + 8].copy_from_slice(&second_array);
        [first.into(), second_array]
    }

    #[inline]
    fn advance_jitter_phase_steps4_pair(
        &mut self,
        index: usize,
        render_second: bool,
        first_frame_advanced: bool,
    ) -> [[f32; 4]; 2] {
        let current = std::array::from_fn(|lane| self.phase_steps[index + lane]);
        if !render_second {
            return [current, [0.0; 4]];
        }
        let current = f32x4::from(current);
        let step = f32x4::from(std::array::from_fn(|lane| {
            self.swarm_pitch_step[index + lane]
        }));
        let first = if first_frame_advanced {
            current
        } else {
            current + step
        };
        let second = first + step;
        let second_array: [f32; 4] = second.into();
        self.phase_steps[index..index + 4].copy_from_slice(&second_array);
        [first.into(), second_array]
    }

    #[inline]
    fn advance_jitter_phase_steps_pair<const LANES: usize>(
        &mut self,
        index: usize,
        render_second: bool,
        first_frame_advanced: bool,
    ) -> [[f32; LANES]; 2] {
        let mut phase_steps = [[0.0; LANES]; 2];
        for lane in 0..LANES {
            let index = index + lane;
            if !render_second {
                phase_steps[0][lane] = self.phase_steps[index];
                continue;
            }
            let step = self.swarm_pitch_step[index];
            let first = if first_frame_advanced {
                self.phase_steps[index]
            } else {
                self.phase_steps[index] + step
            };
            phase_steps[0][lane] = first;
            phase_steps[1][lane] = first + step;
            self.phase_steps[index] = phase_steps[1][lane];
        }
        phase_steps
    }

    fn reset_oscillators(&mut self) {
        for bank in &mut self.oscillators {
            for oscillator in bank {
                oscillator.reset();
            }
        }
        self.spectral_harmonic.fill([0; MAX_UNISON]);
        self.spectral_target.fill([0; MAX_UNISON]);
        self.spectral_remaining.fill([0; MAX_UNISON]);
        self.spectral_top_gain.fill([1.0; MAX_UNISON]);
        self.spectral_check_remaining.fill(0);
    }

    fn randomize_oscillators(&mut self, seed: u64) {
        for bank in 0..OSCILLATOR_COUNT {
            if self.enabled_oscillator_mask & (1 << bank) != 0 {
                self.randomize_oscillator_bank(bank, seed);
            }
        }
    }

    fn randomize_oscillator_bank(&mut self, bank: usize, seed: u64) {
        let amount = f64::from(if bank == 0 {
            self.unison.settings.phase_random
        } else {
            self.secondary_unison[bank - 1].settings.phase_random
        });
        let bank_seed = seed ^ (bank as u64).wrapping_mul(0x4f53_435f_4241_4e4b);
        for (index, oscillator) in self.oscillators[bank].iter_mut().enumerate() {
            let lane_seed =
                bank_seed.wrapping_add((index as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15));
            oscillator.set_phase(unit_hash(lane_seed) * amount);
        }
        self.spectral_harmonic[bank].fill(0);
        self.spectral_target[bank].fill(0);
        self.spectral_remaining[bank].fill(0);
        self.spectral_top_gain[bank].fill(1.0);
        self.spectral_check_remaining[bank] = 0;
    }

    fn seed_enabled_unison_layouts(&mut self, seed: u64) {
        for oscillator in 0..OSCILLATOR_COUNT {
            if self.enabled_oscillator_mask & (1 << oscillator) != 0 {
                self.seed_unison_layout(oscillator, seed);
            }
        }
    }

    fn seed_unison_layout(&mut self, oscillator: usize, seed: u64) {
        let seed = oscillator_stereo_seed(seed, oscillator);
        if oscillator == 0 {
            self.unison.set_random_seed(seed);
        } else {
            self.secondary_unison[oscillator - 1].set_random_seed(seed);
        }
    }

    fn set_pitch_bend(&mut self, semitones: f32) {
        let pitch_ratio = (semitones / 12.0).exp2();
        let scale = pitch_ratio / self.pitch_ratio;
        if self.unison.settings.motion_active() && !self.phase_steps_dirty {
            self.scale_primary_phase_steps(scale);
        } else {
            self.phase_steps_dirty = true;
        }
        for secondary in 0..OSCILLATOR_COUNT - 1 {
            if self.enabled_oscillator_mask & (1 << (secondary + 1)) == 0 {
                continue;
            }
            if self.secondary_unison[secondary].settings.motion_active()
                && !self.secondary_phase_steps_dirty[secondary]
            {
                self.scale_secondary_phase_steps(secondary, scale);
            } else {
                self.secondary_phase_steps_dirty[secondary] = true;
            }
        }
        self.pitch_ratio = pitch_ratio;
        self.spectral_check_remaining.fill(0);
    }

    fn advance_glide(&mut self) {
        if self.glide_remaining == 0 {
            return;
        }
        let mut scale = self.glide_multiplier;
        self.frequency_hz *= scale;
        self.glide_remaining -= 1;
        if self.glide_remaining == 0 {
            let correction = self.glide_target_hz / self.frequency_hz;
            self.frequency_hz = self.glide_target_hz;
            self.glide_multiplier = 1.0;
            scale *= correction;
        }
        self.scale_cached_phase_steps(scale);
        self.spectral_check_remaining.fill(0);
    }

    fn scale_cached_phase_steps(&mut self, scale: f32) {
        if self.enabled_oscillator_mask & 1 != 0 {
            self.scale_primary_phase_steps(scale);
        }
        for secondary in 0..OSCILLATOR_COUNT - 1 {
            if self.enabled_oscillator_mask & (1 << (secondary + 1)) != 0 {
                self.scale_secondary_phase_steps(secondary, scale);
            }
        }
    }

    fn scale_primary_phase_steps(&mut self, scale: f32) {
        for index in 0..usize::from(self.unison.settings.voices) {
            self.phase_steps[index] = (self.phase_steps[index] * scale).min(0.45);
            self.swarm_pitch_step[index] *= scale;
        }
    }

    fn scale_secondary_phase_steps(&mut self, secondary: usize, scale: f32) {
        for index in 0..usize::from(self.secondary_unison[secondary].settings.voices) {
            self.secondary_phase_steps[secondary][index] =
                (self.secondary_phase_steps[secondary][index] * scale).min(0.45);
            self.secondary_swarm_pitch_step[secondary][index] *= scale;
        }
    }

    fn refresh_phase_steps(&mut self) {
        let base_phase_step =
            self.base_phase_step(self.frequency_hz * self.pitch_ratio, self.sample_rate);
        for index in 0..usize::from(self.unison.settings.voices) {
            self.phase_steps[index] = base_phase_step * self.unison.ratios[index];
        }
        self.phase_steps_dirty = false;
    }

    fn refresh_secondary_phase_steps(&mut self, secondary: usize) {
        let base_phase_step = self.secondary_base_phase_step(
            secondary,
            self.frequency_hz * self.pitch_ratio,
            self.sample_rate,
        );
        for index in 0..usize::from(self.secondary_unison[secondary].settings.voices) {
            self.secondary_phase_steps[secondary][index] =
                base_phase_step * self.secondary_unison[secondary].ratios[index];
        }
        self.secondary_phase_steps_dirty[secondary] = false;
    }

    #[inline]
    fn lane_phase_step(&self, index: usize, dynamic_base_step: Option<f32>) -> f32 {
        dynamic_base_step
            .map_or(self.phase_steps[index], |base_step| {
                base_step * self.unison.ratios[index]
            })
            .min(0.45)
    }

    #[inline]
    fn oscillator_phase_step(
        &self,
        index: usize,
        pitch_ratio: f32,
        dynamic_base_step: Option<f32>,
    ) -> f32 {
        (self.lane_phase_step(index, dynamic_base_step) * pitch_ratio).min(0.45)
    }

    #[inline]
    fn secondary_oscillator_phase_step(
        &self,
        secondary: usize,
        index: usize,
        pitch_ratio: f32,
        dynamic_base_step: Option<f32>,
    ) -> f32 {
        let phase_step =
            dynamic_base_step.map_or(self.secondary_phase_steps[secondary][index], |_| {
                self.secondary_base_phase_step(
                    secondary,
                    self.frequency_hz * self.pitch_ratio,
                    self.sample_rate,
                ) * self.secondary_unison[secondary].ratios[index]
            });
        (phase_step * pitch_ratio).min(0.45)
    }

    fn render_secondary_oscillator(
        &mut self,
        settings: VoiceSettings,
        oscillator_index: usize,
        dynamic_base_step: Option<f32>,
    ) -> (f32, f32) {
        let oscillator = settings.oscillator(oscillator_index);
        if !oscillator.enabled {
            return (0.0, 0.0);
        }
        let secondary = oscillator_index - 1;
        if self.secondary_phase_steps_dirty[secondary] {
            self.refresh_secondary_phase_steps(secondary);
        }
        if self.secondary_unison[secondary].settings.motion_active() {
            self.advance_secondary_swarm(secondary);
        }
        let shape = self.effective_oscillator_shape(settings, oscillator_index);
        let (left_gain, right_gain) = oscillator.channel_gains();
        let spectral_check = if settings.antialiasing == Antialiasing::Spectral {
            if self.spectral_check_remaining[oscillator_index] == 0 {
                self.spectral_check_remaining[oscillator_index] = 31;
                true
            } else {
                self.spectral_check_remaining[oscillator_index] -= 1;
                false
            }
        } else {
            false
        };
        let voice_count = usize::from(self.secondary_unison[secondary].settings.voices);
        let mut index = 0;
        let mut left8 = f32x8::ZERO;
        let mut right8 = f32x8::ZERO;
        while index + 8 <= voice_count {
            let phase_steps = std::array::from_fn(|lane| {
                self.secondary_oscillator_phase_step(
                    secondary,
                    index + lane,
                    oscillator.pitch_ratio,
                    dynamic_base_step,
                )
            });
            let oscillators = &mut self.oscillators[oscillator_index][index..index + 8];
            let samples = if oscillator.phase_warp_active() {
                generate_shape8_warped(
                    oscillators,
                    shape,
                    phase_steps,
                    oscillator.pulse_width,
                    settings.antialiasing,
                    oscillator.phase_warp.mode,
                    oscillator.phase_warp.amount,
                )
            } else if shape <= f32::EPSILON {
                generate_sine8(oscillators, phase_steps)
            } else if settings.antialiasing == Antialiasing::Spectral
                && (shape - 2.0).abs() <= f32::EPSILON
            {
                generate_spectral_saw8(
                    oscillators,
                    &mut self.spectral_harmonic[oscillator_index][index..index + 8],
                    &mut self.spectral_target[oscillator_index][index..index + 8],
                    &mut self.spectral_remaining[oscillator_index][index..index + 8],
                    &mut self.spectral_top_gain[oscillator_index][index..index + 8],
                    phase_steps,
                    spectral_check,
                )
            } else if settings.antialiasing == Antialiasing::Spectral {
                generate_spectral_shape8(
                    oscillators,
                    &mut self.spectral_harmonic[oscillator_index][index..index + 8],
                    &mut self.spectral_target[oscillator_index][index..index + 8],
                    &mut self.spectral_remaining[oscillator_index][index..index + 8],
                    shape,
                    phase_steps,
                    oscillator.pulse_width,
                    spectral_check,
                )
            } else if (shape - 1.0).abs() <= f32::EPSILON {
                generate_triangle8(oscillators, phase_steps, settings.antialiasing)
            } else if (shape - 2.0).abs() <= f32::EPSILON {
                generate_saw8(oscillators, phase_steps, settings.antialiasing)
            } else if shape >= 3.0 - f32::EPSILON {
                generate_pulse8(
                    oscillators,
                    phase_steps,
                    oscillator.pulse_width,
                    settings.antialiasing,
                )
            } else {
                generate_shape8(
                    oscillators,
                    shape,
                    phase_steps,
                    oscillator.pulse_width,
                    settings.antialiasing,
                )
            };
            let left_gains = f32x8::from(std::array::from_fn(|lane| {
                self.secondary_unison[secondary].left[index + lane]
            }));
            let right_gains = f32x8::from(std::array::from_fn(|lane| {
                self.secondary_unison[secondary].right[index + lane]
            }));
            left8 = samples.mul_add(left_gains, left8);
            right8 = samples.mul_add(right_gains, right8);
            index += 8;
        }
        let mut left = left8.reduce_add();
        let mut right = right8.reduce_add();
        let mut left4 = f32x4::ZERO;
        let mut right4 = f32x4::ZERO;
        while index + 4 <= voice_count {
            let phase_steps = std::array::from_fn(|lane| {
                self.secondary_oscillator_phase_step(
                    secondary,
                    index + lane,
                    oscillator.pitch_ratio,
                    dynamic_base_step,
                )
            });
            let oscillators = &mut self.oscillators[oscillator_index][index..index + 4];
            let samples = if oscillator.phase_warp_active() {
                generate_shape4_warped(
                    oscillators,
                    shape,
                    phase_steps,
                    oscillator.pulse_width,
                    settings.antialiasing,
                    oscillator.phase_warp.mode,
                    oscillator.phase_warp.amount,
                )
            } else if shape <= f32::EPSILON {
                generate_sine4(oscillators, phase_steps)
            } else if (shape - 1.0).abs() <= f32::EPSILON {
                generate_triangle4(oscillators, phase_steps, settings.antialiasing)
            } else if (shape - 2.0).abs() <= f32::EPSILON {
                generate_saw4(oscillators, phase_steps, settings.antialiasing)
            } else if shape >= 3.0 - f32::EPSILON {
                generate_pulse4(
                    oscillators,
                    phase_steps,
                    oscillator.pulse_width,
                    settings.antialiasing,
                )
            } else {
                generate_shape4(
                    oscillators,
                    shape,
                    phase_steps,
                    oscillator.pulse_width,
                    settings.antialiasing,
                )
            };
            let left_gains = f32x4::from(std::array::from_fn(|lane| {
                self.secondary_unison[secondary].left[index + lane]
            }));
            let right_gains = f32x4::from(std::array::from_fn(|lane| {
                self.secondary_unison[secondary].right[index + lane]
            }));
            left4 = samples.mul_add(left_gains, left4);
            right4 = samples.mul_add(right_gains, right4);
            index += 4;
        }
        left += left4.reduce_add();
        right += right4.reduce_add();
        while index < voice_count {
            let phase_step = self.secondary_oscillator_phase_step(
                secondary,
                index,
                oscillator.pitch_ratio,
                dynamic_base_step,
            );
            let sample = if oscillator.phase_warp_active() {
                self.oscillators[oscillator_index][index].generate_shape_step_warped(
                    shape,
                    phase_step,
                    oscillator.pulse_width,
                    settings.antialiasing,
                    oscillator.phase_warp.mode,
                    oscillator.phase_warp.amount,
                )
            } else {
                self.oscillators[oscillator_index][index].generate_shape_step(
                    shape,
                    phase_step,
                    oscillator.pulse_width,
                    settings.antialiasing,
                )
            };
            left = sample.mul_add(self.secondary_unison[secondary].left[index], left);
            right = sample.mul_add(self.secondary_unison[secondary].right[index], right);
            index += 1;
        }
        let gain = self.secondary_unison[secondary].gain;
        (left * (left_gain * gain), right * (right_gain * gain))
    }

    fn base_phase_step(&self, frequency_hz: f32, sample_rate: f32) -> f32 {
        let highest_ratio = self.unison.ratios[usize::from(self.unison.settings.voices) - 1];
        (frequency_hz.max(0.0) / sample_rate.max(1.0)).min(0.45 / highest_ratio)
    }

    fn secondary_base_phase_step(
        &self,
        secondary: usize,
        frequency_hz: f32,
        sample_rate: f32,
    ) -> f32 {
        let layout = &self.secondary_unison[secondary];
        let highest_ratio = layout.ratios[usize::from(layout.settings.voices) - 1];
        (frequency_hz.max(0.0) / sample_rate.max(1.0)).min(0.45 / highest_ratio)
    }

    fn reset_swarm_motion(&mut self) {
        let interval = self.swarm_update_interval();
        self.swarm_update_remaining = 1 + (self.note_seed as u16 % interval);
        self.swarm_pitch_step.fill(0.0);
    }

    fn reset_all_swarm_motion(&mut self) {
        self.reset_swarm_motion();
        for secondary in 0..OSCILLATOR_COUNT - 1 {
            self.reset_secondary_swarm_motion(secondary);
        }
    }

    fn reset_enabled_swarm_motion(&mut self) {
        if self.enabled_oscillator_mask & 1 != 0 {
            self.reset_swarm_motion();
        }
        for secondary in 0..OSCILLATOR_COUNT - 1 {
            if self.enabled_oscillator_mask & (1 << (secondary + 1)) != 0 {
                self.reset_secondary_swarm_motion(secondary);
            }
        }
    }

    fn reset_secondary_swarm_motion(&mut self, secondary: usize) {
        let interval = self.secondary_swarm_update_interval(secondary);
        let seed = self.note_seed.rotate_left((secondary as u32 + 1) * 17);
        self.secondary_swarm_update_remaining[secondary] = 1 + (seed as u16 % interval);
        self.secondary_swarm_pitch_step[secondary].fill(0.0);
    }

    #[cold]
    #[inline(never)]
    fn prepare_swarm_jitter_target(&mut self, update_interval: u16) {
        let settings = self.unison.settings;
        let interval = f32::from(update_interval);
        let base_phase_step =
            self.base_phase_step(self.frequency_hz * self.pitch_ratio, self.sample_rate);
        let voices = usize::from(settings.voices);
        let target_clock =
            wrap_swarm_clock(self.swarm_clock + interval * settings.swarm_rate / self.sample_rate);
        fill_unison_jitter_offsets_mode(
            &mut self.swarm_pitch_step[..voices],
            self.unison.random_seed,
            settings.swarm_amount,
            target_clock,
            settings.swarm_mode,
        );
        let mut ratios = [0.0; MAX_UNISON];
        jitter_pitch_ratios(
            &mut ratios[..voices],
            &mut self.swarm_pitch_step[..voices],
            settings.detune_cents,
            settings.swarm_mode,
        );
        for index in 0..voices {
            let target = (base_phase_step * self.unison.ratios[index] * ratios[index]).min(0.45);
            self.swarm_pitch_step[index] = (target - self.phase_steps[index]) / interval;
        }
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the clamped positive control interval fits in u16"
    )]
    fn swarm_update_interval(&self) -> u16 {
        let settings = self.unison.settings;
        let update_rate = settings.swarm_rate.max(0.02)
            * if settings.swarm_mode == SwarmMode::Sine {
                8.0
            } else {
                1.0
            };
        (self.sample_rate / update_rate).round().clamp(
            f32::from(SWARM_MIN_UPDATE_INTERVAL),
            f32::from(SWARM_MAX_UPDATE_INTERVAL),
        ) as u16
    }

    #[inline(never)]
    fn advance_swarm(&mut self) {
        debug_assert!(self.unison.settings.motion_active());
        if self.swarm_update_remaining == 0 {
            let update_interval = self.swarm_update_interval();
            self.prepare_swarm_jitter_target(update_interval);
            self.swarm_update_remaining = update_interval;
        }
        let voices = usize::from(self.unison.settings.voices);
        for index in 0..voices {
            self.phase_steps[index] =
                (self.phase_steps[index] + self.swarm_pitch_step[index]).min(0.45);
        }
        self.swarm_update_remaining -= 1;
    }

    #[cold]
    #[inline(never)]
    fn prepare_secondary_swarm_jitter_target(&mut self, secondary: usize, update_interval: u16) {
        let settings = self.secondary_unison[secondary].settings;
        let interval = f32::from(update_interval);
        let base_phase_step = self.secondary_base_phase_step(
            secondary,
            self.frequency_hz * self.pitch_ratio,
            self.sample_rate,
        );
        let voices = usize::from(settings.voices);
        let target_clock = wrap_swarm_clock(
            self.secondary_swarm_clock[secondary]
                + interval * settings.swarm_rate / self.sample_rate,
        );
        fill_unison_jitter_offsets_mode(
            &mut self.secondary_swarm_pitch_step[secondary][..voices],
            self.secondary_unison[secondary].random_seed,
            settings.swarm_amount,
            target_clock,
            settings.swarm_mode,
        );
        let mut ratios = [0.0; MAX_UNISON];
        jitter_pitch_ratios(
            &mut ratios[..voices],
            &mut self.secondary_swarm_pitch_step[secondary][..voices],
            settings.detune_cents,
            settings.swarm_mode,
        );
        for index in 0..voices {
            let target =
                (base_phase_step * self.secondary_unison[secondary].ratios[index] * ratios[index])
                    .min(0.45);
            self.secondary_swarm_pitch_step[secondary][index] =
                (target - self.secondary_phase_steps[secondary][index]) / interval;
        }
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the clamped positive control interval fits in u16"
    )]
    fn secondary_swarm_update_interval(&self, secondary: usize) -> u16 {
        let settings = self.secondary_unison[secondary].settings;
        let update_rate = settings.swarm_rate.max(0.02)
            * if settings.swarm_mode == SwarmMode::Sine {
                8.0
            } else {
                1.0
            };
        (self.sample_rate / update_rate).round().clamp(
            f32::from(SWARM_MIN_UPDATE_INTERVAL),
            f32::from(SWARM_MAX_UPDATE_INTERVAL),
        ) as u16
    }

    #[inline(never)]
    fn advance_secondary_swarm(&mut self, secondary: usize) {
        debug_assert!(self.secondary_unison[secondary].settings.motion_active());
        if self.secondary_swarm_update_remaining[secondary] == 0 {
            let update_interval = self.secondary_swarm_update_interval(secondary);
            self.prepare_secondary_swarm_jitter_target(secondary, update_interval);
            self.secondary_swarm_update_remaining[secondary] = update_interval;
        }
        let voices = usize::from(self.secondary_unison[secondary].settings.voices);
        for index in 0..voices {
            self.secondary_phase_steps[secondary][index] = (self.secondary_phase_steps[secondary]
                [index]
                + self.secondary_swarm_pitch_step[secondary][index])
                .min(0.45);
        }
        self.secondary_swarm_update_remaining[secondary] -= 1;
    }

    #[inline(never)]
    fn prepare_swarm_pair(&mut self, clocks: [f32; 2]) -> bool {
        debug_assert!(self.unison.settings.motion_active());
        self.set_swarm_clock(clocks[0]);
        if self.swarm_update_remaining == 0 {
            let update_interval = self.swarm_update_interval();
            self.prepare_swarm_jitter_target(update_interval);
            self.swarm_update_remaining = update_interval;
        }
        if self.swarm_update_remaining == 1 {
            self.advance_swarm();
            self.set_swarm_clock(clocks[1]);
            let update_interval = self.swarm_update_interval();
            self.prepare_swarm_jitter_target(update_interval);
            self.swarm_update_remaining = update_interval - 1;
            return true;
        }
        self.swarm_update_remaining -= 2;
        false
    }

    fn advance_envelope(&mut self, sample_rate: f32, force_gate: bool) {
        debug_assert!((self.sample_rate - sample_rate.max(1.0)).abs() <= f32::EPSILON);
        match self.stage {
            EnvelopeStage::Idle => self.envelope_level = 0.0,
            EnvelopeStage::Attack => {
                self.advance_envelope_progress();
                self.envelope_level = shaped_progress(
                    self.envelope_progress,
                    self.envelope.attack_curve_time,
                    self.envelope.attack_curve,
                )
                .mul_add(1.0 - self.envelope_start, self.envelope_start);
                if self.envelope_progress >= 1.0 {
                    self.envelope_level = 1.0;
                    self.begin_decay();
                }
            }
            EnvelopeStage::Decay => {
                let sustain = self.envelope.sustain.clamp(0.0, 1.0);
                if sustain >= self.envelope_start {
                    self.envelope_level = sustain;
                    self.stage = EnvelopeStage::Sustain;
                } else {
                    self.advance_envelope_progress();
                    self.envelope_level = shaped_progress(
                        self.envelope_progress,
                        self.envelope.decay_curve_time,
                        self.envelope.decay_curve,
                    )
                    .mul_add(sustain - self.envelope_start, self.envelope_start);
                    if self.envelope_progress >= 1.0 {
                        self.envelope_level = sustain;
                        self.stage = EnvelopeStage::Sustain;
                    }
                }
            }
            EnvelopeStage::Sustain => {
                self.envelope_level = self.envelope.sustain.clamp(0.0, 1.0);
            }
            EnvelopeStage::Release => {
                self.advance_envelope_progress();
                self.envelope_level =
                    (1.0 - shaped_progress(
                        self.envelope_progress,
                        self.envelope.release_curve_time,
                        self.envelope.release_curve,
                    )) * self.envelope_start;
                if self.envelope_progress >= 1.0 || self.envelope_level <= 1.0e-5 {
                    self.finish_envelope();
                }
            }
        }
        if force_gate && self.stage == EnvelopeStage::Release {
            self.begin_attack();
        }
    }

    fn release(&mut self, immediate: bool, sample_rate: f32) {
        self.held = false;
        if self.sustained && !immediate {
            return;
        }
        if immediate {
            self.finish_envelope();
        } else if self.stage != EnvelopeStage::Idle {
            debug_assert!((self.sample_rate - sample_rate.max(1.0)).abs() <= f32::EPSILON);
            if self.envelope.release <= 0.0 {
                self.finish_envelope();
            } else {
                self.begin_stage(EnvelopeStage::Release);
            }
        }
    }

    fn begin_attack(&mut self) {
        if self.envelope.attack <= 0.0 {
            self.envelope_level = 1.0;
            self.begin_decay();
            return;
        }
        self.begin_stage(EnvelopeStage::Attack);
        let remaining = (1.0 - self.envelope_start).max(f32::EPSILON);
        self.envelope_step = 1.0 / (self.envelope.attack * self.sample_rate * remaining).max(1.0);
    }

    fn begin_decay(&mut self) {
        let sustain = self.envelope.sustain.clamp(0.0, 1.0);
        if sustain >= self.envelope_level || self.envelope.decay <= 0.0 {
            self.envelope_level = sustain;
            self.stage = EnvelopeStage::Sustain;
        } else {
            self.begin_stage(EnvelopeStage::Decay);
        }
    }

    fn begin_stage(&mut self, stage: EnvelopeStage) {
        self.stage = stage;
        self.envelope_start = self.envelope_level;
        self.envelope_progress = 0.0;
        self.refresh_envelope_step();
    }

    fn refresh_envelope_step(&mut self) {
        let seconds = match self.stage {
            EnvelopeStage::Attack => self.envelope.attack.max(f32::EPSILON),
            EnvelopeStage::Decay => self.envelope.decay.max(f32::EPSILON),
            EnvelopeStage::Release => self.envelope.release.max(f32::EPSILON),
            EnvelopeStage::Idle | EnvelopeStage::Sustain => {
                self.envelope_step = 1.0;
                return;
            }
        };
        self.envelope_step = 1.0 / (seconds * self.sample_rate).max(1.0);
    }

    #[inline]
    fn advance_envelope_progress(&mut self) {
        self.envelope_progress = (self.envelope_progress + self.envelope_step).min(1.0);
    }

    const fn finish_envelope(&mut self) {
        self.envelope_level = 0.0;
        self.envelope_start = 0.0;
        self.envelope_progress = 0.0;
        self.stage = EnvelopeStage::Idle;
        self.current_note = None;
        self.voice_id = None;
        self.glide_remaining = 0;
        self.glide_multiplier = 1.0;
    }

    fn matches(&self, note: u8, channel: u8, voice_id: Option<i32>) -> bool {
        self.current_note == Some(note)
            && self.channel == channel
            && voice_id.is_none_or(|id| self.voice_id == Some(id))
    }

    fn active(&self) -> bool {
        self.stage != EnvelopeStage::Idle
    }

    fn effective_shape(&self, settings: VoiceSettings) -> f32 {
        self.effective_oscillator_shape(settings, 0)
    }

    fn effective_oscillator_shape(&self, settings: VoiceSettings, oscillator: usize) -> f32 {
        ((self.timbre - 0.5) * 2.0)
            .mul_add(
                settings.timbre_amount.clamp(0.0, 1.0),
                settings.oscillator(oscillator).shape,
            )
            .clamp(0.0, 3.0)
    }

    fn block_shape_banks_eligible(&self, settings: VoiceSettings) -> bool {
        if self.output_continuity.active() {
            return false;
        }
        let mut any = false;
        for oscillator in 0..OSCILLATOR_COUNT {
            let oscillator_settings = settings.oscillator(oscillator);
            if oscillator_settings.enabled {
                any = true;
                let shape = self.effective_oscillator_shape(settings, oscillator);
                let motion = if oscillator == 0 {
                    self.unison.settings.motion_active()
                } else {
                    self.secondary_unison[oscillator - 1]
                        .settings
                        .motion_active()
                };
                if oscillator_settings.phase_warp_active()
                    || (shape - 2.0).abs() > f32::EPSILON && motion
                {
                    return false;
                }
            }
        }
        any
    }

    fn exact_saw_banks_eligible(&self, settings: VoiceSettings) -> bool {
        self.block_shape_banks_eligible(settings)
            && (0..OSCILLATOR_COUNT).all(|oscillator| {
                !settings.oscillator(oscillator).enabled
                    || (self.effective_oscillator_shape(settings, oscillator) - 2.0).abs()
                        <= f32::EPSILON
            })
    }
}

#[derive(Clone, Copy, Default)]
struct HeldNote {
    note: u8,
    velocity: f32,
    channel: u8,
    voice_id: Option<i32>,
    per_note_bend: f32,
    per_note_timbre: Option<f32>,
}

pub struct PolySynth {
    voices: [VaVoice; POLYPHONY],
    envelope: EnvelopeSettings,
    sample_rate: f32,
    age: u64,
    active_count: u8,
    sustain: [bool; 16],
    parameter_bend: f32,
    pitch_bend: [f32; 16],
    per_note_bend: [f32; POLYPHONY],
    per_note_timbre: [Option<f32>; POLYPHONY],
    timbre: [f32; 16],
    latest_stereo_seed: [f32; OSCILLATOR_COUNT],
    swarm_time: f64,
    swarm_step: f64,
    secondary_swarm_time: [f64; OSCILLATOR_COUNT - 1],
    secondary_swarm_step: [f64; OSCILLATOR_COUNT - 1],
    enabled_oscillator_mask: u8,
    oscillator_mix: [f32; OSCILLATOR_COUNT],
    unison_settings: [UnisonSettings; OSCILLATOR_COUNT],
    pending_unison: [UnisonSettings; OSCILLATOR_COUNT],
    unison_configuration_fade: [ConfigurationFade; OSCILLATOR_COUNT],
    phase_warp_mode: [PhaseWarpMode; OSCILLATOR_COUNT],
    pending_phase_warp_mode: [PhaseWarpMode; OSCILLATOR_COUNT],
    phase_warp_configuration_fade: [ConfigurationFade; OSCILLATOR_COUNT],
    voice_mode: u8,
    transpose_semitones: f32,
    glide_time: f32,
    mono_stack: [HeldNote; POLYPHONY],
    mono_stack_len: u8,
}

#[derive(Clone, Copy, Debug, Default)]
struct ConfigurationFade {
    gain: f32,
    stage: u8,
}

impl ConfigurationFade {
    const STEADY: u8 = 0;
    const FADE_OUT: u8 = 1;
    const FADE_IN: u8 = 2;

    const fn steady() -> Self {
        Self {
            gain: 1.0,
            stage: Self::STEADY,
        }
    }

    const fn is_steady(self) -> bool {
        self.stage == Self::STEADY
    }
}

impl Default for PolySynth {
    fn default() -> Self {
        Self {
            voices: std::array::from_fn(|_| VaVoice::default()),
            envelope: EnvelopeSettings::default(),
            sample_rate: 44_100.0,
            age: 0,
            active_count: 0,
            sustain: [false; 16],
            parameter_bend: 0.0,
            pitch_bend: [0.0; 16],
            per_note_bend: [0.0; POLYPHONY],
            per_note_timbre: [None; POLYPHONY],
            timbre: [0.5; 16],
            latest_stereo_seed: [0.5; OSCILLATOR_COUNT],
            swarm_time: 0.0,
            swarm_step: 0.7 / 44_100.0,
            secondary_swarm_time: [0.0; OSCILLATOR_COUNT - 1],
            secondary_swarm_step: [0.7 / 44_100.0; OSCILLATOR_COUNT - 1],
            enabled_oscillator_mask: 1,
            oscillator_mix: [1.0, 0.0, 0.0],
            unison_settings: std::array::from_fn(|_| UnisonSettings::new(1, 0.0, 0.0, 1.0, 0.0)),
            pending_unison: std::array::from_fn(|_| UnisonSettings::new(1, 0.0, 0.0, 1.0, 0.0)),
            unison_configuration_fade: [ConfigurationFade::steady(); OSCILLATOR_COUNT],
            phase_warp_mode: [PhaseWarpMode::None; OSCILLATOR_COUNT],
            pending_phase_warp_mode: [PhaseWarpMode::None; OSCILLATOR_COUNT],
            phase_warp_configuration_fade: [ConfigurationFade::steady(); OSCILLATOR_COUNT],
            voice_mode: POLYPHONY_U8,
            transpose_semitones: 0.0,
            glide_time: 0.08,
            mono_stack: [HeldNote::default(); POLYPHONY],
            mono_stack_len: 0,
        }
    }
}

impl PolySynth {
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate.max(1.0);
        self.swarm_step =
            f64::from(self.unison_settings[0].swarm_rate) / f64::from(self.sample_rate);
        for secondary in 0..OSCILLATOR_COUNT - 1 {
            self.secondary_swarm_step[secondary] =
                f64::from(self.unison_settings[secondary + 1].swarm_rate)
                    / f64::from(self.sample_rate);
        }
        for voice in &mut self.voices {
            voice.set_sample_rate(self.sample_rate);
        }
    }

    pub fn reset(&mut self) {
        for voice in &mut self.voices {
            voice.reset();
        }
        self.age = 0;
        self.active_count = 0;
        self.sustain.fill(false);
        self.parameter_bend = 0.0;
        self.pitch_bend.fill(0.0);
        self.per_note_bend.fill(0.0);
        self.per_note_timbre.fill(None);
        self.timbre.fill(0.5);
        self.latest_stereo_seed.fill(0.5);
        self.swarm_time = 0.0;
        self.secondary_swarm_time.fill(0.0);
        self.oscillator_mix = std::array::from_fn(|oscillator| {
            if self.enabled_oscillator_mask & (1 << oscillator) != 0 {
                1.0
            } else {
                0.0
            }
        });
        self.unison_configuration_fade = [ConfigurationFade::steady(); OSCILLATOR_COUNT];
        self.pending_unison = self.unison_settings;
        self.phase_warp_configuration_fade = [ConfigurationFade::steady(); OSCILLATOR_COUNT];
        self.pending_phase_warp_mode = self.phase_warp_mode;
        self.mono_stack_len = 0;
    }

    pub fn configure_voice_mode(&mut self, mode: u8) {
        let mode = mode.clamp(0, POLYPHONY_U8);
        if self.voice_mode == mode {
            return;
        }
        for voice in &mut self.voices {
            if voice.active() {
                voice.release(true, self.sample_rate);
            }
        }
        self.active_count = 0;
        self.per_note_bend.fill(0.0);
        self.per_note_timbre.fill(None);
        self.mono_stack_len = 0;
        self.voice_mode = mode;
    }

    pub const fn set_glide_time(&mut self, seconds: f32) {
        self.glide_time = seconds.clamp(0.0, 5.0);
    }

    pub fn configure_oscillator_enabled(&mut self, enabled: [bool; OSCILLATOR_COUNT]) {
        let mask = enabled
            .into_iter()
            .enumerate()
            .fold(0_u8, |mask, (oscillator, enabled)| {
                mask | (u8::from(enabled) << oscillator)
            });
        if self.enabled_oscillator_mask == mask {
            return;
        }
        self.enabled_oscillator_mask = mask;
        for voice in &mut self.voices {
            voice.set_enabled_oscillator_mask(mask);
        }
        if self.active_count == 0 {
            self.oscillator_mix = std::array::from_fn(|oscillator| {
                if mask & (1 << oscillator) != 0 {
                    1.0
                } else {
                    0.0
                }
            });
        }
    }

    pub fn set_transpose(&mut self, semitones: f32) {
        let semitones = semitones.clamp(-60.0, 60.0);
        if self.transpose_semitones.to_bits() == semitones.to_bits() {
            return;
        }
        self.transpose_semitones = semitones;
        let global_bend = self.parameter_bend + self.pitch_bend[0];
        for (index, voice) in self.voices.iter_mut().enumerate() {
            if voice.active() {
                let member = if voice.channel == 0 {
                    0.0
                } else {
                    self.pitch_bend[voice.channel as usize]
                };
                voice.set_pitch_bend(semitones + global_bend + member + self.per_note_bend[index]);
            }
        }
    }

    pub fn note_on(&mut self, note: u8, velocity: f32, channel: u8, voice_id: Option<i32>) {
        if self.voice_mode < 2 {
            self.note_on_mono(note, velocity, channel, voice_id);
            return;
        }
        self.age = self.age.wrapping_add(1);
        let channel = channel.min(15);
        self.set_latest_stereo_seeds(note_phase_seed(note, channel, voice_id, self.age));
        if let Some(index) = self
            .voices
            .iter()
            .position(|voice| voice.held && voice.matches(note, channel, voice_id))
        {
            self.per_note_bend[index] = 0.0;
            self.per_note_timbre[index] = None;
            self.voices[index].retrigger(velocity, voice_id, self.age);
            self.voices[index]
                .set_pitch_bend(self.transpose_semitones + self.effective_pitch_bend(channel));
            self.voices[index].timbre = self.effective_timbre(channel);
            return;
        }

        let voice_limit = usize::from(self.voice_mode.clamp(2, POLYPHONY_U8));
        let index = self.voices[..voice_limit]
            .iter()
            .position(|voice| !voice.active())
            .unwrap_or_else(|| {
                self.voices[..voice_limit]
                    .iter()
                    .enumerate()
                    .min_by(|(_, left), (_, right)| {
                        left.envelope_level
                            .total_cmp(&right.envelope_level)
                            .then_with(|| left.age.cmp(&right.age))
                    })
                    .map_or(0, |(index, _)| index)
            });
        let was_active = self.voices[index].active();
        self.per_note_bend[index] = 0.0;
        self.per_note_timbre[index] = None;
        self.prepare_voice_unison(index);
        self.voices[index].start(note, velocity, channel, voice_id, self.age);
        self.voices[index]
            .set_pitch_bend(self.transpose_semitones + self.effective_pitch_bend(channel));
        self.voices[index].timbre = self.effective_timbre(channel);
        if !was_active {
            self.active_count += 1;
        }
    }

    fn note_on_mono(&mut self, note: u8, velocity: f32, channel: u8, voice_id: Option<i32>) {
        let channel = channel.min(15);
        self.remove_mono_note(note, channel, voice_id);
        if usize::from(self.mono_stack_len) == POLYPHONY {
            self.mono_stack.copy_within(1..POLYPHONY, 0);
            self.mono_stack_len -= 1;
        }
        self.mono_stack[usize::from(self.mono_stack_len)] = HeldNote {
            note,
            velocity,
            channel,
            voice_id,
            per_note_bend: 0.0,
            per_note_timbre: None,
        };
        self.mono_stack_len += 1;

        self.age = self.age.wrapping_add(1);
        let next_seed = note_phase_seed(note, channel, voice_id, self.age);
        let pitch_bend = self.transpose_semitones + self.effective_pitch_bend(channel);
        let timbre = self.effective_timbre(channel);
        let voice = &mut self.voices[0];
        let connect_legato = voice.active() && voice.held;
        self.per_note_bend[0] = 0.0;
        self.per_note_timbre[0] = None;
        if self.voice_mode == 1 && connect_legato {
            voice.legato_to(note, velocity, channel, voice_id, self.age, self.glide_time);
        } else {
            self.latest_stereo_seed =
                std::array::from_fn(|oscillator| oscillator_stereo_seed(next_seed, oscillator));
            for oscillator in 0..OSCILLATOR_COUNT {
                if oscillator == 0 {
                    voice.configure_unison(self.unison_settings[oscillator]);
                } else {
                    voice.configure_secondary_unison(oscillator, self.unison_settings[oscillator]);
                }
            }
            voice.start(note, velocity, channel, voice_id, self.age);
        }
        voice.set_pitch_bend(pitch_bend);
        voice.timbre = timbre;
        self.active_count = 1;
    }

    pub fn note_off(&mut self, note: u8, channel: u8, voice_id: Option<i32>) {
        if self.voice_mode < 2 {
            self.note_off_mono(note, channel, voice_id);
            return;
        }
        let channel = channel.min(15);
        let finished = if let Some(voice) = self
            .voices
            .iter_mut()
            .filter(|voice| voice.matches(note, channel, voice_id) && voice.held)
            .max_by_key(|voice| voice.age)
        {
            voice.sustained = self.sustain[channel as usize];
            voice.release(false, self.sample_rate);
            !voice.active()
        } else {
            false
        };
        if finished {
            self.active_count -= 1;
        }
    }

    fn note_off_mono(&mut self, note: u8, channel: u8, voice_id: Option<i32>) {
        let channel = channel.min(15);
        let was_current = self.voices[0].matches(note, channel, voice_id);
        self.remove_mono_note(note, channel, voice_id);
        if !was_current {
            return;
        }
        if self.mono_stack_len != 0 {
            let held = self.mono_stack[usize::from(self.mono_stack_len - 1)];
            self.age = self.age.wrapping_add(1);
            let next_seed = note_phase_seed(held.note, held.channel, held.voice_id, self.age);
            let pitch_bend = self.transpose_semitones
                + self.effective_pitch_bend(held.channel)
                + held.per_note_bend;
            let timbre = self.effective_timbre(held.channel);
            let voice = &mut self.voices[0];
            if self.voice_mode == 1 {
                voice.legato_to(
                    held.note,
                    held.velocity,
                    held.channel,
                    held.voice_id,
                    self.age,
                    self.glide_time,
                );
            } else {
                self.latest_stereo_seed =
                    std::array::from_fn(|oscillator| oscillator_stereo_seed(next_seed, oscillator));
                voice.start(
                    held.note,
                    held.velocity,
                    held.channel,
                    held.voice_id,
                    self.age,
                );
            }
            voice.set_pitch_bend(pitch_bend);
            self.per_note_bend[0] = held.per_note_bend;
            self.per_note_timbre[0] = held.per_note_timbre;
            voice.timbre = held.per_note_timbre.unwrap_or(timbre);
            return;
        }
        let voice = &mut self.voices[0];
        voice.sustained = self.sustain[channel as usize];
        voice.release(false, self.sample_rate);
        if !voice.active() {
            self.active_count = 0;
        }
    }

    fn remove_mono_note(&mut self, note: u8, channel: u8, voice_id: Option<i32>) {
        let len = usize::from(self.mono_stack_len);
        if let Some(index) = self.mono_stack[..len].iter().position(|held| {
            held.note == note
                && held.channel == channel
                && voice_id.is_none_or(|id| held.voice_id == Some(id))
        }) {
            self.mono_stack.copy_within(index + 1..len, index);
            self.mono_stack_len -= 1;
        }
    }

    pub fn all_notes_off(&mut self, channel: u8) {
        let channel = channel.min(15);
        if self.voice_mode < 2 {
            self.clear_mono_channel(channel);
        }
        let mut finished = 0_u8;
        for voice in &mut self.voices {
            if (channel == 0 || voice.channel == channel) && voice.active() && voice.held {
                voice.sustained = self.sustain[voice.channel as usize];
                voice.release(false, self.sample_rate);
                finished += u8::from(!voice.active());
            }
        }
        self.active_count -= finished;
    }

    pub fn all_sound_off(&mut self, channel: u8) {
        let channel = channel.min(15);
        if self.voice_mode < 2 {
            self.clear_mono_channel(channel);
        }
        for voice in &mut self.voices {
            if (channel == 0 || voice.channel == channel) && voice.active() {
                voice.sustained = false;
                voice.release(true, self.sample_rate);
            }
        }
        self.refresh_voice_count();
    }

    pub fn reset_controllers(&mut self, channel: u8) {
        let channel = channel.min(15);
        self.sustain(channel, false);
        self.pitch_bend(channel, 0.0, 2.0);
        self.timbre(channel, 0.5);
    }

    pub fn pressure(&mut self, note: u8, channel: u8, voice_id: Option<i32>, pressure: f32) {
        for voice in &mut self.voices {
            if voice.matches(note, channel, voice_id) {
                voice.pressure = pressure.clamp(0.0, 1.0);
            }
        }
    }

    pub fn channel_pressure(&mut self, channel: u8, pressure: f32) {
        let channel = channel.min(15);
        for voice in &mut self.voices {
            if (channel == 0 || voice.channel == channel) && voice.active() {
                voice.pressure = pressure.clamp(0.0, 1.0);
            }
        }
    }

    pub fn pitch_bend(&mut self, channel: u8, bipolar: f32, mpe_range: f32) {
        let channel = channel.min(15);
        let range = mpe_range.clamp(1.0, 96.0);
        let semitones = bipolar.clamp(-1.0, 1.0) * range;
        if self.pitch_bend[channel as usize].to_bits() == semitones.to_bits() {
            return;
        }
        self.pitch_bend[channel as usize] = semitones;
        for (index, voice) in self.voices.iter_mut().enumerate() {
            if channel == 0 || voice.channel == channel {
                let member = if voice.channel == 0 {
                    0.0
                } else {
                    self.pitch_bend[voice.channel as usize]
                };
                voice.set_pitch_bend(
                    self.transpose_semitones
                        + self.parameter_bend
                        + self.pitch_bend[0]
                        + member
                        + self.per_note_bend[index],
                );
            }
        }
    }

    pub fn parameter_pitch_bend(&mut self, bipolar: f32, range: f32) {
        let semitones = bipolar.clamp(-1.0, 1.0) * range.clamp(1.0, 96.0);
        if self.parameter_bend.to_bits() == semitones.to_bits() {
            return;
        }
        self.parameter_bend = semitones;
        for (index, voice) in self.voices.iter_mut().enumerate() {
            if voice.active() {
                let member = if voice.channel == 0 {
                    0.0
                } else {
                    self.pitch_bend[voice.channel as usize]
                };
                voice.set_pitch_bend(
                    self.transpose_semitones
                        + self.parameter_bend
                        + self.pitch_bend[0]
                        + member
                        + self.per_note_bend[index],
                );
            }
        }
    }

    pub fn per_note_pitch_bend(&mut self, note: u8, channel: u8, semitones: f32) {
        let channel = channel.min(15);
        let channel_bend = self.transpose_semitones + self.effective_pitch_bend(channel);
        let semitones = semitones.clamp(-48.0, 48.0);
        for (index, voice) in self.voices.iter_mut().enumerate() {
            if voice.matches(note, channel, None) {
                self.per_note_bend[index] = semitones;
                voice.set_pitch_bend(channel_bend + semitones);
            }
        }
        for held in &mut self.mono_stack[..usize::from(self.mono_stack_len)] {
            if held.note == note && held.channel == channel {
                held.per_note_bend = semitones;
            }
        }
    }

    pub fn per_note_timbre(&mut self, note: u8, channel: u8, value: f32) {
        let channel = channel.min(15);
        let value = value.clamp(0.0, 1.0);
        for (index, voice) in self.voices.iter_mut().enumerate() {
            if voice.matches(note, channel, None) {
                self.per_note_timbre[index] = Some(value);
                voice.timbre = value;
            }
        }
        for held in &mut self.mono_stack[..usize::from(self.mono_stack_len)] {
            if held.note == note && held.channel == channel {
                held.per_note_timbre = Some(value);
            }
        }
    }

    pub fn reset_per_note_controllers(&mut self, note: u8, channel: u8) {
        let channel = channel.min(15);
        let fallback = self.effective_timbre(channel);
        let pitch_bend = self.transpose_semitones + self.effective_pitch_bend(channel);
        for (index, voice) in self.voices.iter_mut().enumerate() {
            if voice.matches(note, channel, None) {
                self.per_note_bend[index] = 0.0;
                self.per_note_timbre[index] = None;
                voice.set_pitch_bend(pitch_bend);
                voice.timbre = fallback;
            }
        }
        for held in &mut self.mono_stack[..usize::from(self.mono_stack_len)] {
            if held.note == note && held.channel == channel {
                held.per_note_bend = 0.0;
                held.per_note_timbre = None;
            }
        }
    }

    pub fn timbre(&mut self, channel: u8, value: f32) {
        let channel = channel.min(15);
        let value = value.clamp(0.0, 1.0);
        self.timbre[channel as usize] = value;
        for (index, voice) in self.voices.iter_mut().enumerate() {
            if (channel == 0 || voice.channel == channel) && voice.active() {
                let member = if voice.channel == 0 {
                    0.5
                } else {
                    self.timbre[voice.channel as usize]
                };
                voice.timbre = self.per_note_timbre[index]
                    .unwrap_or_else(|| (self.timbre[0] + member - 0.5).clamp(0.0, 1.0));
            }
        }
    }

    pub fn configure_unison(&mut self, settings: UnisonSettings) {
        self.schedule_unison_configuration(0, settings);
    }

    pub fn configure_secondary_unison(&mut self, oscillator: usize, settings: UnisonSettings) {
        self.schedule_unison_configuration(oscillator, settings);
    }

    fn schedule_unison_configuration(&mut self, oscillator: usize, settings: UnisonSettings) {
        let current = self.unison_settings[oscillator];
        self.pending_unison[oscillator] = settings;
        if self.active_count == 0 {
            self.apply_unison_configuration(oscillator, settings);
            self.unison_configuration_fade[oscillator] = ConfigurationFade::steady();
            return;
        }
        if current.voices == settings.voices {
            self.apply_unison_configuration(oscillator, settings);
            let fade = &mut self.unison_configuration_fade[oscillator];
            if fade.stage == ConfigurationFade::FADE_OUT {
                fade.stage = ConfigurationFade::FADE_IN;
            }
            return;
        }
        self.unison_configuration_fade[oscillator].stage = ConfigurationFade::FADE_OUT;
    }

    fn apply_unison_configuration(&mut self, oscillator: usize, settings: UnisonSettings) {
        self.unison_settings[oscillator] = settings;
        if oscillator == 0 {
            for voice in self.voices.iter_mut().filter(|voice| voice.active()) {
                voice.configure_unison(settings);
            }
            self.swarm_step = f64::from(settings.swarm_rate) / f64::from(self.sample_rate);
        } else {
            for voice in self.voices.iter_mut().filter(|voice| voice.active()) {
                voice.configure_secondary_unison(oscillator, settings);
            }
            self.secondary_swarm_step[oscillator - 1] =
                f64::from(settings.swarm_rate) / f64::from(self.sample_rate);
        }
    }

    fn prepare_voice_unison(&mut self, index: usize) {
        let voice = &mut self.voices[index];
        voice.configure_unison(self.unison_settings[0]);
        for oscillator in 1..OSCILLATOR_COUNT {
            voice.configure_secondary_unison(oscillator, self.unison_settings[oscillator]);
        }
    }

    pub fn configure_phase_warp_modes(&mut self, modes: [PhaseWarpMode; OSCILLATOR_COUNT]) {
        for (oscillator, mode) in modes.into_iter().enumerate() {
            if self.active_count == 0 {
                self.phase_warp_mode[oscillator] = mode;
                self.pending_phase_warp_mode[oscillator] = mode;
                self.phase_warp_configuration_fade[oscillator] = ConfigurationFade::steady();
                continue;
            }
            let fade = &mut self.phase_warp_configuration_fade[oscillator];
            if self.phase_warp_mode[oscillator] == mode {
                self.pending_phase_warp_mode[oscillator] = mode;
                if fade.stage == ConfigurationFade::FADE_OUT {
                    fade.stage = ConfigurationFade::FADE_IN;
                }
                continue;
            }
            if self.pending_phase_warp_mode[oscillator] == mode && !fade.is_steady() {
                continue;
            }
            self.pending_phase_warp_mode[oscillator] = mode;
            fade.stage = ConfigurationFade::FADE_OUT;
        }
    }

    pub fn sustain(&mut self, channel: u8, enabled: bool) {
        let channel = channel.min(15);
        if channel == 0 {
            self.sustain.fill(enabled);
        } else {
            self.sustain[channel as usize] = enabled;
        }
        if !enabled {
            let mut finished = 0_u8;
            for voice in &mut self.voices {
                if (channel == 0 || voice.channel == channel) && voice.sustained && !voice.held {
                    voice.sustained = false;
                    voice.release(false, self.sample_rate);
                    finished += u8::from(!voice.active());
                }
            }
            self.active_count -= finished;
        }
    }

    fn oscillator_mix_steady(&self) -> bool {
        (0..OSCILLATOR_COUNT).all(|oscillator| {
            let target: f32 = if self.enabled_oscillator_mask & (1 << oscillator) != 0 {
                1.0
            } else {
                0.0
            };
            self.oscillator_mix[oscillator].to_bits() == target.to_bits()
                && self.unison_configuration_fade[oscillator].is_steady()
                && self.phase_warp_configuration_fade[oscillator].is_steady()
        })
    }

    fn advance_configuration_fades(&mut self, settings: &mut VoiceSettings) {
        let step = 2.0 / (CONFIGURATION_FADE_SECONDS * self.sample_rate).max(1.0);
        for oscillator in 0..OSCILLATOR_COUNT {
            let mut apply_unison = false;
            {
                let fade = &mut self.unison_configuration_fade[oscillator];
                match fade.stage {
                    ConfigurationFade::FADE_OUT => {
                        fade.gain = (fade.gain - step).max(0.0);
                        if fade.gain <= f32::EPSILON {
                            fade.gain = 0.0;
                            fade.stage = ConfigurationFade::FADE_IN;
                            apply_unison = true;
                        }
                    }
                    ConfigurationFade::FADE_IN => {
                        fade.gain = (fade.gain + step).min(1.0);
                        if fade.gain >= 1.0 - f32::EPSILON {
                            *fade = ConfigurationFade::steady();
                        }
                    }
                    _ => {}
                }
            }
            if apply_unison {
                self.apply_unison_configuration(oscillator, self.pending_unison[oscillator]);
            }

            {
                let fade = &mut self.phase_warp_configuration_fade[oscillator];
                match fade.stage {
                    ConfigurationFade::FADE_OUT => {
                        fade.gain = (fade.gain - step).max(0.0);
                        if fade.gain <= f32::EPSILON {
                            fade.gain = 0.0;
                            self.phase_warp_mode[oscillator] =
                                self.pending_phase_warp_mode[oscillator];
                            fade.stage = ConfigurationFade::FADE_IN;
                        }
                    }
                    ConfigurationFade::FADE_IN => {
                        fade.gain = (fade.gain + step).min(1.0);
                        if fade.gain >= 1.0 - f32::EPSILON {
                            *fade = ConfigurationFade::steady();
                        }
                    }
                    _ => {}
                }
            }

            settings.oscillators[oscillator].phase_warp.mode = self.phase_warp_mode[oscillator];
            settings.oscillators[oscillator].scale_level(
                self.unison_configuration_fade[oscillator].gain
                    * self.phase_warp_configuration_fade[oscillator].gain,
            );
        }
    }

    fn advance_oscillator_mix(&mut self, mut settings: VoiceSettings) -> VoiceSettings {
        self.advance_configuration_fades(&mut settings);
        let step = 1.0 / (OSCILLATOR_FADE_SECONDS * self.sample_rate).max(1.0);
        for oscillator in 0..OSCILLATOR_COUNT {
            let enabled = self.enabled_oscillator_mask & (1 << oscillator) != 0;
            let target = if enabled { 1.0 } else { 0.0 };
            let current = self.oscillator_mix[oscillator];
            self.oscillator_mix[oscillator] = if current < target {
                (current + step).min(target)
            } else {
                (current - step).max(target)
            };
            settings.oscillators[oscillator].enabled =
                enabled || self.oscillator_mix[oscillator] > f32::EPSILON;
            settings.oscillators[oscillator].scale_level(self.oscillator_mix[oscillator]);
        }
        settings
    }

    pub fn render(&mut self, settings: VoiceSettings, envelope: EnvelopeSettings) -> (f32, f32) {
        if self.active_count == 0 {
            return (0.0, 0.0);
        }

        if self.envelope != envelope {
            self.envelope = envelope;
            for voice in &mut self.voices {
                voice.configure(envelope);
            }
        }

        let settings = self.advance_oscillator_mix(settings);
        let mut left = 0.0;
        let mut right = 0.0;
        if settings.oscillator(0).enabled {
            self.swarm_time = wrap_swarm_time(self.swarm_time + self.swarm_step);
        }
        for secondary in 0..OSCILLATOR_COUNT - 1 {
            if settings.oscillator(secondary + 1).enabled {
                self.secondary_swarm_time[secondary] = wrap_swarm_time(
                    self.secondary_swarm_time[secondary] + self.secondary_swarm_step[secondary],
                );
            }
        }
        let mut remaining = self.active_count;
        for voice in &mut self.voices {
            if voice.active() {
                voice.set_swarm_clock(self.swarm_time as f32);
                for secondary in 0..OSCILLATOR_COUNT - 1 {
                    if settings.oscillator(secondary + 1).enabled {
                        voice.set_secondary_swarm_clock(
                            secondary + 1,
                            self.secondary_swarm_time[secondary] as f32,
                        );
                    }
                }
                let (voice_left, voice_right) = voice.render(settings, self.sample_rate, false);
                left += voice_left;
                right += voice_right;
                if !voice.active() {
                    self.active_count -= 1;
                }
                remaining -= 1;
                if remaining == 0 {
                    break;
                }
            }
        }
        (left * MASTER_HEADROOM, right * MASTER_HEADROOM)
    }

    pub fn render_pair(
        &mut self,
        settings: VoiceSettings,
        envelope: EnvelopeSettings,
    ) -> [(f32, f32); 2] {
        debug_assert_ne!(settings.antialiasing, Antialiasing::Spectral);
        if !self.oscillator_mix_steady() || !settings.legacy_primary_fast_path() {
            return [
                self.render(settings, envelope),
                self.render(settings, envelope),
            ];
        }
        if self.active_count == 0 {
            return [(0.0, 0.0); 2];
        }
        if self.envelope != envelope {
            self.envelope = envelope;
            for voice in &mut self.voices {
                voice.configure(envelope);
            }
        }

        let clock0 = wrap_swarm_time(self.swarm_time + self.swarm_step);
        let clock1 = wrap_swarm_time(clock0 + self.swarm_step);
        self.swarm_time = clock1;
        let mut output = [(0.0_f32, 0.0_f32); 2];
        let mut rendered_second = false;
        let mut remaining = self.active_count;
        for voice in &mut self.voices {
            if voice.active() {
                let (samples, voice_rendered_second) =
                    voice.render_pair(settings, self.sample_rate, [clock0 as f32, clock1 as f32]);
                rendered_second |= voice_rendered_second;
                for frame in 0..2 {
                    output[frame].0 += samples[frame].0;
                    output[frame].1 += samples[frame].1;
                }
                if !voice.active() {
                    self.active_count -= 1;
                }
                remaining -= 1;
                if remaining == 0 {
                    break;
                }
            }
        }
        for sample in &mut output {
            sample.0 *= MASTER_HEADROOM;
            sample.1 *= MASTER_HEADROOM;
        }
        if !rendered_second {
            self.swarm_time = clock0;
        }
        output
    }

    pub(crate) fn exact_saw_banks_eligible(&self, settings: VoiceSettings) -> bool {
        self.voices
            .iter()
            .filter(|voice| voice.active())
            .all(|voice| voice.exact_saw_banks_eligible(settings))
    }

    pub(crate) fn block_shape_banks_eligible(&self, settings: VoiceSettings) -> bool {
        self.voices
            .iter()
            .filter(|voice| voice.active())
            .all(|voice| voice.block_shape_banks_eligible(settings))
    }

    pub fn block_internal_samples(
        &self,
        settings: VoiceSettings,
        oversampling_factor: u8,
    ) -> Option<usize> {
        let eligible = self.active_count != 0
            && self.oscillator_mix_steady()
            && settings.antialiasing != Antialiasing::Spectral
            && self
                .voices
                .iter()
                .filter(|voice| voice.active())
                .all(|voice| voice.held && !voice.is_gliding());
        eligible.then(|| {
            if oversampling_factor == 3 {
                FACTOR3_BLOCK_INTERNAL_SAMPLES
            } else {
                BLOCK_INTERNAL_SAMPLES
            }
        })
    }

    pub fn render_block<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        envelope: EnvelopeSettings,
    ) -> [(f32, f32); SAMPLES] {
        if self.block_shape_banks_eligible(settings) {
            return self.render_saw_block(settings, envelope);
        }
        debug_assert!(self.active_count != 0);
        debug_assert_ne!(settings.antialiasing, Antialiasing::Spectral);
        if self.envelope != envelope {
            self.envelope = envelope;
            for voice in &mut self.voices {
                voice.configure(envelope);
            }
        }

        let mut clocks = [[0.0; SAMPLES]; OSCILLATOR_COUNT];
        for oscillator in 0..OSCILLATOR_COUNT {
            if settings.oscillator(oscillator).enabled {
                let (time, step) = if oscillator == 0 {
                    (&mut self.swarm_time, self.swarm_step)
                } else {
                    (
                        &mut self.secondary_swarm_time[oscillator - 1],
                        self.secondary_swarm_step[oscillator - 1],
                    )
                };
                for clock in &mut clocks[oscillator] {
                    *time = wrap_swarm_time(*time + step);
                    *clock = *time as f32;
                }
            }
        }
        let mut output = [(0.0_f32, 0.0_f32); SAMPLES];
        let mut remaining = self.active_count;
        for voice in &mut self.voices {
            if voice.active() {
                let samples = voice.render_generic_block(settings, self.sample_rate, clocks);
                for frame in 0..SAMPLES {
                    output[frame].0 += samples[frame].0;
                    output[frame].1 += samples[frame].1;
                }
                if !voice.active() {
                    self.active_count -= 1;
                }
                remaining -= 1;
                if remaining == 0 {
                    break;
                }
            }
        }
        for sample in &mut output {
            sample.0 *= MASTER_HEADROOM;
            sample.1 *= MASTER_HEADROOM;
        }
        output
    }

    pub fn render_saw_block<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        envelope: EnvelopeSettings,
    ) -> [(f32, f32); SAMPLES] {
        debug_assert!(self.active_count != 0);
        debug_assert_ne!(settings.antialiasing, Antialiasing::Spectral);
        if self.envelope != envelope {
            self.envelope = envelope;
            for voice in &mut self.voices {
                voice.configure(envelope);
            }
        }

        let mut clocks = [[0.0; SAMPLES]; OSCILLATOR_COUNT];
        for oscillator in 0..OSCILLATOR_COUNT {
            if settings.oscillator(oscillator).enabled {
                let (time, step) = if oscillator == 0 {
                    (&mut self.swarm_time, self.swarm_step)
                } else {
                    (
                        &mut self.secondary_swarm_time[oscillator - 1],
                        self.secondary_swarm_step[oscillator - 1],
                    )
                };
                for clock in &mut clocks[oscillator] {
                    *time = wrap_swarm_time(*time + step);
                    *clock = *time as f32;
                }
            }
        }
        let mut output = [(0.0_f32, 0.0_f32); SAMPLES];
        let mut remaining = self.active_count;
        for voice in &mut self.voices {
            if voice.active() {
                let samples = voice.render_saw_block(settings, self.sample_rate, clocks);
                for frame in 0..SAMPLES {
                    output[frame].0 += samples[frame].0;
                    output[frame].1 += samples[frame].1;
                }
                remaining -= 1;
                if remaining == 0 {
                    break;
                }
            }
        }
        for sample in &mut output {
            sample.0 *= MASTER_HEADROOM;
            sample.1 *= MASTER_HEADROOM;
        }
        output
    }

    pub const fn is_active(&self) -> bool {
        self.active_count != 0
    }

    pub const fn is_gliding(&self) -> bool {
        self.voice_mode == 1 && self.voices[0].is_gliding()
    }

    pub const fn latest_stereo_seed(&self, oscillator: usize) -> f32 {
        self.latest_stereo_seed[oscillator]
    }

    pub const fn swarm_time(&self) -> f32 {
        self.swarm_time as f32
    }

    pub const fn secondary_swarm_time(&self, oscillator: usize) -> f32 {
        self.secondary_swarm_time[oscillator - 1] as f32
    }

    fn set_latest_stereo_seeds(&mut self, seed: u64) {
        self.latest_stereo_seed =
            std::array::from_fn(|oscillator| oscillator_stereo_seed(seed, oscillator));
    }

    fn effective_pitch_bend(&self, channel: u8) -> f32 {
        self.parameter_bend
            + self.pitch_bend[0]
            + if channel == 0 {
                0.0
            } else {
                self.pitch_bend[channel as usize]
            }
    }

    fn clear_mono_channel(&mut self, channel: u8) {
        let len = usize::from(self.mono_stack_len);
        let mut output = 0;
        for index in 0..len {
            let held = self.mono_stack[index];
            if channel != 0 && held.channel != channel {
                self.mono_stack[output] = held;
                output += 1;
            }
        }
        #[allow(
            clippy::cast_possible_truncation,
            reason = "the fixed mono note stack contains at most 32 entries"
        )]
        {
            self.mono_stack_len = output as u8;
        }
    }

    fn effective_timbre(&self, channel: u8) -> f32 {
        let member = if channel == 0 {
            0.5
        } else {
            self.timbre[channel as usize]
        };
        (self.timbre[0] + member - 0.5).clamp(0.0, 1.0)
    }

    fn refresh_voice_count(&mut self) {
        #[allow(
            clippy::cast_possible_truncation,
            reason = "the fixed pool has exactly 32 note voices"
        )]
        {
            self.active_count = self.voices.iter().filter(|voice| voice.active()).count() as u8;
        }
    }
}

#[inline]
fn wrap_swarm_clock(clock: f32) -> f32 {
    if clock >= 4_096.0 {
        clock - 4_096.0
    } else {
        clock
    }
}

#[inline]
fn wrap_swarm_time(clock: f64) -> f64 {
    if clock >= 4_096.0 {
        clock - 4_096.0
    } else {
        clock
    }
}

#[inline]
fn tuned_phase_step(phase_step: f32, pitch_ratio: f32) -> f32 {
    if pitch_ratio.to_bits() == 1.0_f32.to_bits() {
        phase_step
    } else {
        (phase_step * pitch_ratio).min(0.45)
    }
}

fn constant_jitter_ramp_final<const SAMPLES: usize>(
    phase_steps: &[f32; MAX_UNISON],
    deltas: &[f32; MAX_UNISON],
    voices: usize,
    pitch_ratio: f32,
) -> Option<[f32; MAX_UNISON]> {
    let mut final_steps = *phase_steps;
    for index in 0..voices {
        for _ in 0..SAMPLES {
            final_steps[index] = (final_steps[index] + deltas[index]).min(0.45);
            if pitch_ratio > 1.0 && final_steps[index] * pitch_ratio > 0.45 {
                return None;
            }
        }
    }
    Some(final_steps)
}

#[inline]
fn shaped_progress(progress: f32, curve_time: f32, curve_level: f32) -> f32 {
    let progress = progress.clamp(0.0, 1.0);
    let handle_x = curve_time.clamp(-1.0, 1.0).mul_add(0.5, 0.5);
    let handle_y = curve_level.clamp(-1.0, 1.0).mul_add(0.5, 0.5);
    let warped_time = schlick_bias(progress, 1.0 - handle_x);
    schlick_bias(warped_time, handle_y)
}

#[inline]
fn schlick_bias(value: f32, bias: f32) -> f32 {
    let bias = bias.clamp(0.005, 0.995);
    value / ((bias.recip() - 2.0).mul_add(1.0 - value, 1.0))
}

fn midi_note_to_hz(note: u8) -> f32 {
    440.0 * ((f32::from(note) - 69.0) / 12.0).exp2()
}

fn note_phase_seed(note: u8, channel: u8, voice_id: Option<i32>, age: u64) -> u64 {
    let voice_id = u64::from(voice_id.unwrap_or_default().cast_unsigned());
    age ^ (u64::from(note) << 48) ^ (u64::from(channel) << 40) ^ voice_id.rotate_left(17)
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "the shared random seed intentionally enters f32 meter and pan state"
)]
fn stereo_seed(seed: u64) -> f32 {
    unit_hash(seed.rotate_left(29) ^ 0x5354_4552_454f_4b56) as f32
}

fn oscillator_stereo_seed(seed: u64, oscillator: usize) -> f32 {
    let seed = if oscillator == 0 {
        seed
    } else {
        seed ^ (oscillator as u64 * 0x4f53_435f_554e_4953)
    };
    stereo_seed(seed)
}

#[allow(
    clippy::cast_precision_loss,
    reason = "all 53 retained hash bits are exactly representable in f64"
)]
fn unit_hash(seed: u64) -> f64 {
    let mut value = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^= value >> 31;
    (value >> 11) as f64 * (1.0 / ((1_u64 << 53) as f64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mpe_member_and_per_note_bends_compose_and_persist() {
        let mut synth = PolySynth::default();
        synth.note_on(60, 1.0, 1, None);
        synth.pitch_bend(1, 1.0, 48.0);
        assert!((synth.voices[0].pitch_ratio - 16.0).abs() < 1.0e-5);

        synth.per_note_pitch_bend(60, 1, 12.0);
        synth.set_transpose(-12.0);
        assert!((synth.voices[0].pitch_ratio - 16.0).abs() < 1.0e-5);

        synth.pitch_bend(1, 0.5, 48.0);
        assert!((synth.voices[0].pitch_ratio - 4.0).abs() < 1.0e-5);
    }

    #[test]
    fn legato_glides_without_retriggering_and_returns_to_held_note() {
        let mut synth = PolySynth::default();
        synth.set_sample_rate(1_000.0);
        synth.configure_voice_mode(1);
        synth.set_glide_time(0.1);
        synth.note_on(60, 1.0, 1, None);
        let stage = synth.voices[0].stage;
        let envelope = synth.voices[0].envelope_level;

        synth.note_on(72, 0.8, 1, None);
        assert_eq!(synth.voices[0].stage, stage);
        assert_eq!(synth.voices[0].envelope_level, envelope);
        assert_eq!(synth.voices[0].glide_remaining, 100);

        let settings = VoiceSettings::new(2.0, 440.0, 0.5, 0.0, 0.0, 0.0);
        for _ in 0..100 {
            synth.render(settings, EnvelopeSettings::default());
        }
        assert!(!synth.is_gliding());
        assert!((synth.voices[0].frequency_hz - midi_note_to_hz(72)).abs() < 1.0e-4);

        synth.note_off(72, 1, None);
        assert_eq!(synth.voices[0].current_note, Some(60));
        assert!(synth.is_gliding());
    }

    #[test]
    fn voice_mode_caps_the_active_pool() {
        let mut synth = PolySynth::default();
        synth.configure_voice_mode(4);
        for note in 60..68 {
            synth.note_on(note, 1.0, 0, None);
        }
        assert_eq!(synth.active_count, 4);
        assert_eq!(
            synth.voices.iter().filter(|voice| voice.active()).count(),
            4
        );
    }
}
