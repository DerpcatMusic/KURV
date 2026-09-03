use std::sync::OnceLock;

mod audio;

use self::audio::*;
use super::spectral::{MAX_RATIO, MIN_RATIO, ratio_brickwall_bypassed};

use crate::voices::fast_exp2;
use truce_simd::simd::f32x4;

pub(super) const DEFAULT_SAMPLE_RATE: f32 = 44_100.0;
pub(super) const MIN_CUTOFF_HZ: f32 = 5.0;
pub(super) const MAX_STORED_CUTOFF_HZ: f32 = 100_000.0;
pub(super) const NYQUIST_GUARD: f32 = 0.495;
pub(crate) const MIN_Q: f32 = 0.1;
pub(crate) const MAX_Q: f32 = 32.0;
pub(super) const Q_OCTAVES: f32 = 8.321_928;
pub(super) const NEUTRAL_SVF_Q: f32 = std::f32::consts::FRAC_1_SQRT_2;
pub(super) const MAX_RESONANCE_DB: f32 = 30.0;
pub(super) const RESONANCE_SKIRT_HEADROOM_DB: f32 = 0.75;
pub(crate) const MIN_SLOPE_DB: f32 = 6.0;
pub(super) const MIN_SVF_SLOPE_DB: f32 = 12.0;
pub(crate) const MAX_SLOPE_DB: f32 = 102.0;
pub(super) const MAX_SVF_STAGES: usize = 64;
pub(super) const MAX_ACTIVE_SVF_STAGES: usize = 16;
// 32-pole Chebyshev II, -3.0103 dB at 1.0 and -120 dB stopband.
// (pole frequency, damping, inverse squared zero/pole frequency ratio)
pub(super) const BRICKWALL_PROTOTYPE: [(f32, f32, f32); MAX_ACTIVE_SVF_STAGES] = [
    (1.000_988_1, 0.041_717_906, 0.819_285_04),
    (1.008_941_9, 0.125_743_21, 0.816_401_7),
    (1.025_110_7, 0.211_562_99, 0.810_470_16),
    (1.050_031_5, 0.300_461_2, 0.801_143_05),
    (1.084_545_9, 0.393_856_59, 0.787_855_45),
    (1.129_840_9, 0.493_361_53, 0.769_765_44),
    (1.187_497_9, 0.600_838_84, 0.745_667_5),
    (1.259_533_2, 0.718_442_2, 0.713_875_23),
    (1.348_387_8, 0.848_598_96, 0.672_081_53),
    (1.456_769, 0.993_840_2, 0.617_247_9),
    (1.587_119_7, 1.156_265_9, 0.545_686_6),
    (1.740_271_4, 1.336_221_3, 0.453_776_75),
    (1.912_555_1, 1.529_511_3, 0.340_273_23),
    (2.090_836, 1.722_676_8, 0.211_546_4),
    (2.247_304, 1.888_138, 0.089_122_705),
    (2.341_902_7, 1.986_751_4, 0.010_823_125),
];
pub(super) const MAX_PHASE_POLES: usize = 128;
pub(super) const MAX_PHASE_SECTIONS: usize = MAX_PHASE_POLES / 2;
pub(super) const CENTERED_PHASE_EXPONENTS: [f32; MAX_PHASE_POLES] = centered_phase_exponents();
pub(super) const COEFFICIENT_TABLE_SIZE: usize = 2_048;
pub(super) const PHASE_SPAN_TABLE_SIZE: usize = 256;
pub(super) const PHASE_RADIUS_TABLE_SIZE: usize = 2_048;
pub(super) const MIN_PHASE_SPAN_OCTAVES: f32 = 0.05;
pub(super) const MAX_PHASE_SPAN_OCTAVES: f32 = 10.0;
pub(super) const SCREAM_PREVIEW_INPUT_PEAK: f32 = 0.5;
pub(super) static COEFFICIENT_TABLE: OnceLock<Box<[f32]>> = OnceLock::new();
pub(super) static PHASE_RATIO_TABLE: OnceLock<Box<[f32]>> = OnceLock::new();
pub(super) static PHASE_SPAN_TABLE: OnceLock<Box<[f32]>> = OnceLock::new();
pub(super) static PHASE_RADIUS_TABLE: OnceLock<Box<[f32]>> = OnceLock::new();
pub(super) static SCREAM_HP_RATIO_TABLE: OnceLock<Box<[f32]>> = OnceLock::new();
pub(super) static SCREAM_FEEDBACK_TABLE: OnceLock<Box<[f32]>> = OnceLock::new();
pub(super) static BUTTERWORTH_DAMPING: OnceLock<Box<[f32]>> = OnceLock::new();

pub(crate) fn prepare() {
    let _ = coefficient_table();
    let _ = phase_ratio_table();
    let _ = phase_span_table();
    let _ = phase_radius_table();
    let _ = scream_hp_ratio_table();
    let _ = scream_feedback_table();
    let _ = butterworth_damping_table();
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FilterMode {
    #[default]
    Svf,
    Phaser,
    Scream,
    RatioBrickwall,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FilterDomain {
    Audio,
    Spectral,
}

impl FilterDomain {
    pub const ALL: [Self; 2] = [Self::Audio, Self::Spectral];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Audio => "AUDIO",
            Self::Spectral => "SPECTRAL",
        }
    }
}

impl FilterMode {
    pub const ALL: [Self; 4] = [Self::Svf, Self::Phaser, Self::Scream, Self::RatioBrickwall];

    #[must_use]
    pub const fn domain(self) -> FilterDomain {
        match self {
            Self::Svf | Self::Phaser | Self::Scream => FilterDomain::Audio,
            Self::RatioBrickwall => FilterDomain::Spectral,
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Svf => "SVF MORPH",
            Self::Phaser => "PHASER",
            Self::Scream => "SCREAM",
            Self::RatioBrickwall => "RATIO BRICKWALL",
        }
    }

    #[must_use]
    pub const fn short_label(self) -> &'static str {
        match self {
            Self::Svf => "SVF",
            Self::Phaser => "PHASE",
            Self::Scream => "SCREAM",
            Self::RatioBrickwall => "RATIO",
        }
    }

    #[must_use]
    pub const fn resonance_label(self) -> &'static str {
        match self {
            Self::Svf | Self::Phaser => "Q",
            Self::Scream => "RESO",
            Self::RatioBrickwall => "—",
        }
    }

    #[must_use]
    pub const fn slope_label(self) -> &'static str {
        match self {
            Self::Svf => "DB/OCT",
            Self::Phaser => "SPACING",
            Self::Scream => "SCREAM",
            Self::RatioBrickwall => "—",
        }
    }

    #[must_use]
    pub const fn morph_label(self) -> &'static str {
        match self {
            Self::Svf => "MORPH",
            Self::Phaser => "POLES",
            Self::Scream => "MIX",
            Self::RatioBrickwall => "—",
        }
    }

    #[must_use]
    pub const fn resonance_help(self) -> &'static str {
        match self {
            Self::Svf => "Q",
            Self::Phaser => "Notch depth from dry to full cancellation",
            Self::Scream => "Feedback drive and high-pass resonance",
            Self::RatioBrickwall => "Not used by a harmonic brickwall",
        }
    }

    #[must_use]
    pub const fn slope_help(self) -> &'static str {
        match self {
            Self::Svf => "Continuous slope to 96 dB/oct, then Brickwall",
            Self::Phaser => "Logarithmic spacing between Phaser stages",
            Self::Scream => "Feedback high-pass position relative to cutoff",
            Self::RatioBrickwall => "Not used by a harmonic brickwall",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FilterConfig {
    pub mode: FilterMode,
    pub cutoff_hz: f32,
    pub q: f32,
    pub slope_db_oct: f32,
    pub morph: f32,
    pub shape: f32,
}

impl Default for FilterConfig {
    fn default() -> Self {
        Self {
            mode: FilterMode::Svf,
            cutoff_hz: 20_000.0,
            q: std::f32::consts::FRAC_1_SQRT_2,
            slope_db_oct: 12.0,
            morph: 0.0,
            shape: 0.0,
        }
    }
}

impl FilterConfig {
    #[must_use]
    pub(crate) fn for_mode(mode: FilterMode) -> Self {
        match mode {
            FilterMode::Svf => Self::default(),
            FilterMode::Phaser => Self {
                mode,
                cutoff_hz: 800.0,
                q: MAX_Q,
                slope_db_oct: (MIN_SLOPE_DB * MAX_SLOPE_DB).sqrt(),
                morph: 0.25,
                shape: 0.0,
            },
            FilterMode::Scream => Self {
                mode,
                cutoff_hz: 5_000.0,
                q: 8.0,
                slope_db_oct: (MIN_SLOPE_DB * MAX_SLOPE_DB).sqrt(),
                morph: 1.0,
                shape: 0.0,
            },
            FilterMode::RatioBrickwall => Self {
                mode,
                cutoff_hz: MIN_RATIO,
                ..Self::default()
            },
        }
    }

    #[must_use]
    pub(crate) fn sanitized(self) -> Self {
        Self {
            mode: self.mode,
            cutoff_hz: if self.mode == FilterMode::RatioBrickwall {
                finite_or(self.cutoff_hz, MIN_RATIO).clamp(MIN_RATIO, MAX_RATIO)
            } else {
                finite_or(self.cutoff_hz, 20_000.0).clamp(MIN_CUTOFF_HZ, MAX_STORED_CUTOFF_HZ)
            },
            q: finite_or(self.q, std::f32::consts::FRAC_1_SQRT_2).clamp(MIN_Q, MAX_Q),
            slope_db_oct: finite_or(self.slope_db_oct, self.minimum_slope())
                .clamp(self.minimum_slope(), MAX_SLOPE_DB),
            morph: finite_or(self.morph, 0.0).clamp(0.0, 1.0),
            shape: finite_or(self.shape, 0.0).clamp(0.0, 1.0),
        }
    }

    fn sanitized_for_sample_rate(self, sample_rate: f32) -> Self {
        let maximum_cutoff = sample_rate * NYQUIST_GUARD;
        let mut config = self.sanitized();
        if config.mode != FilterMode::RatioBrickwall {
            config.cutoff_hz = config
                .cutoff_hz
                .clamp(MIN_CUTOFF_HZ.min(maximum_cutoff), maximum_cutoff);
        }
        config
    }

    #[must_use]
    pub(crate) fn modulated(
        self,
        cutoff_octaves: f32,
        resonance_octaves: f32,
        slope: f32,
        morph: f32,
        shape: f32,
    ) -> Self {
        Self {
            cutoff_hz: self.cutoff_hz * fast_exp2(finite_or(cutoff_octaves, 0.0).clamp(-4.0, 4.0)),
            q: self.q * fast_exp2(finite_or(resonance_octaves, 0.0).clamp(-4.0, 4.0)),
            slope_db_oct: finite_or(slope, 0.0) * 12.0 + self.slope_db_oct,
            morph: self.morph + finite_or(morph, 0.0),
            shape: self.shape + finite_or(shape, 0.0),
            ..self
        }
        .sanitized()
    }

    #[must_use]
    pub(crate) fn normalized_q(self) -> f32 {
        normalized_log(self.q, MIN_Q, MAX_Q)
    }

    #[must_use]
    pub(crate) fn normalized_slope(self) -> f32 {
        normalized_log(self.slope_db_oct, self.minimum_slope(), MAX_SLOPE_DB)
    }

    #[must_use]
    pub(crate) const fn minimum_slope(self) -> f32 {
        match self.mode {
            FilterMode::Svf => MIN_SVF_SLOPE_DB,
            FilterMode::Phaser | FilterMode::Scream | FilterMode::RatioBrickwall => MIN_SLOPE_DB,
        }
    }

    #[must_use]
    pub(crate) fn coefficients(self, sample_rate: f32) -> FilterCoefficients {
        let sample_rate = sanitize_sample_rate(sample_rate);
        let config = self.sanitized_for_sample_rate(sample_rate);
        let (svf_stages, brickwall) = svf_shape(config.slope_db_oct, config.morph);
        let stages = match config.mode {
            FilterMode::Svf => svf_stages,
            FilterMode::Phaser => config.morph * (MAX_PHASE_POLES as f32 - 1.0) + 1.0,
            FilterMode::Scream => 2.0,
            FilterMode::RatioBrickwall => 1.0,
        };
        let (processing_stages, processing_blend) = processing_stage_shape(config.mode, stages);
        let table_scale = COEFFICIENT_TABLE_SIZE as f32 / (sample_rate * NYQUIST_GUARD);
        let scream_resonance = normalized_log(config.q, MIN_Q, MAX_Q);
        let damping = match config.mode {
            FilterMode::Svf => svf_resonance_amount(config.q),
            FilterMode::Phaser => phaser_depth(config.q),
            FilterMode::Scream => scream_resonance,
            FilterMode::RatioBrickwall => 0.0,
        };
        let g = coefficient(
            config.cutoff_hz.max(MIN_CUTOFF_HZ) * table_scale,
            coefficient_table(),
        );
        let scream_hp_ratio = scream_hp_ratio(config.slope_db_oct);
        let scream_hp_hz = config.cutoff_hz * scream_hp_ratio;
        let scream_hp_g = coefficient(
            scream_hp_hz.max(MIN_CUTOFF_HZ) * table_scale,
            coefficient_table(),
        );
        let mut coefficients = FilterCoefficients {
            mode: config.mode,
            q: config.q,
            slope_db_oct: config.slope_db_oct,
            g,
            damping,
            morph: config.morph,
            shape: config.shape,
            morph_gain: 1.0,
            brickwall: if config.mode == FilterMode::Svf {
                brickwall
            } else {
                0.0
            },
            stages,
            processing_stages,
            processing_blend,
            span_octaves: match config.mode {
                FilterMode::Svf => stage_span_octaves(processing_stages),
                FilterMode::Phaser => phase_span_octaves(config.slope_db_oct),
                FilterMode::Scream => 0.0,
                FilterMode::RatioBrickwall => 0.0,
            },
            skew: 0.5,
            table_scale,
            cutoff_hz: config.cutoff_hz,
            scream_hp_g,
            scream_hp_ratio,
            scream_hp_damping: lerp(
                std::f32::consts::SQRT_2,
                std::f32::consts::FRAC_1_SQRT_2,
                scream_resonance,
            ),
            scream_feedback: scream_feedback(scream_resonance),
        };
        if config.mode == FilterMode::Svf {
            coefficients.morph_gain = svf_cutoff_gain(coefficients);
        }
        coefficients
    }

    #[must_use]
    pub(crate) fn stage_count(self) -> u8 {
        match self.mode {
            FilterMode::Svf => svf_stage_shape(svf_shape(self.slope_db_oct, self.morph).0).0,
            FilterMode::Phaser => MAX_PHASE_SECTIONS as u8,
            FilterMode::Scream => 2,
            FilterMode::RatioBrickwall => 1,
        }
    }

    #[must_use]
    pub(crate) fn response_stage_count(self) -> u8 {
        match self.mode {
            FilterMode::Svf => self.stage_count(),
            FilterMode::Phaser => self.effective_poles().mul_add(0.5, 0.5).floor() as u8,
            FilterMode::Scream => 2,
            FilterMode::RatioBrickwall => 1,
        }
    }

    #[must_use]
    pub(crate) fn effective_poles(self) -> f32 {
        match self.mode {
            FilterMode::Svf => svf_shape(self.slope_db_oct, self.morph).0 * 2.0,
            FilterMode::Phaser => self.morph.clamp(0.0, 1.0) * (MAX_PHASE_POLES as f32 - 1.0) + 1.0,
            FilterMode::Scream => 2.0,
            FilterMode::RatioBrickwall => 1.0,
        }
    }

    #[must_use]
    pub(crate) fn stage_frequency(self, index: usize, sample_rate: f32) -> f32 {
        let sample_rate = sanitize_sample_rate(sample_rate);
        let config = self.sanitized_for_sample_rate(sample_rate);
        let active_stages = config.stage_count();
        let span_octaves = match config.mode {
            FilterMode::Svf => stage_span_octaves(active_stages),
            FilterMode::Phaser => phase_span_octaves(config.slope_db_oct),
            FilterMode::Scream => 0.0,
            FilterMode::RatioBrickwall => 0.0,
        };
        stage_frequency(
            config.mode,
            index,
            active_stages,
            config.cutoff_hz,
            span_octaves,
            0.5,
        )
    }

    /// Returns the transfer-function magnitude of the realtime implementation.
    ///
    /// This uses the same sanitized configuration and coefficient lookup table as
    /// [`StereoTptSvf`], so editor plots follow the actual TPT filter rather than
    /// an unrelated analogue approximation.
    #[must_use]
    pub(crate) fn response_magnitude(self, frequency: f32, sample_rate: f32) -> f32 {
        let sample_rate = sanitize_sample_rate(sample_rate);
        let frequency = finite_or(frequency, 0.0).clamp(0.0, sample_rate * NYQUIST_GUARD);
        let magnitude =
            response_at(self.coefficients(sample_rate), frequency, sample_rate).magnitude();
        if magnitude.is_finite() {
            magnitude
        } else {
            0.0
        }
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) fn phaser_notch_frequency(
        self,
        notch: usize,
        sample_rate: f32,
        minimum: f32,
        maximum: f32,
    ) -> Option<f32> {
        if self.mode != FilterMode::Phaser || notch >= usize::from(self.response_stage_count()) {
            return None;
        }
        let sample_rate = sanitize_sample_rate(sample_rate);
        let frequency = self.stage_frequency(notch, sample_rate);
        let low = minimum.clamp(0.0, sample_rate * NYQUIST_GUARD);
        let high = maximum.clamp(low, sample_rate * NYQUIST_GUARD);
        (self.response_magnitude(frequency, sample_rate) < 1.0e-3)
            .then_some(frequency)
            .filter(|frequency| (low..=high).contains(frequency))
    }

    #[must_use]
    pub(crate) fn scream_feedback_frequency(self, sample_rate: f32) -> Option<f32> {
        (self.mode == FilterMode::Scream).then(|| {
            let config = self.sanitized_for_sample_rate(sanitize_sample_rate(sample_rate));
            config.cutoff_hz * scream_hp_ratio(config.slope_db_oct)
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct StageCoefficients {
    pub(super) damping: f32,
    pub(super) a1: f32,
    pub(super) a2: f32,
    pub(super) a3: f32,
    pub(super) low_mix: f32,
    pub(super) band_mix: f32,
    pub(super) high_mix: f32,
}

impl StageCoefficients {
    #[inline]
    pub(super) fn from_g(g: f32, damping: f32) -> Self {
        let a1 = (1.0 + g * (g + damping)).recip();
        let a2 = g * a1;
        Self {
            damping,
            a1,
            a2,
            a3: g * a2,
            low_mix: 1.0,
            band_mix: 0.0,
            high_mix: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FilterCoefficients {
    pub(super) mode: FilterMode,
    pub(super) q: f32,
    pub(super) slope_db_oct: f32,
    pub(super) g: f32,
    pub(super) damping: f32,
    pub(super) morph: f32,
    pub(super) shape: f32,
    pub(super) morph_gain: f32,
    pub(super) brickwall: f32,
    pub(super) stages: f32,
    pub(super) processing_stages: u8,
    pub(super) processing_blend: f32,
    pub(super) span_octaves: f32,
    pub(super) skew: f32,
    pub(super) table_scale: f32,
    pub(super) cutoff_hz: f32,
    pub(super) scream_hp_g: f32,
    pub(super) scream_hp_ratio: f32,
    pub(super) scream_hp_damping: f32,
    pub(super) scream_feedback: f32,
}

impl Default for FilterCoefficients {
    fn default() -> Self {
        FilterConfig::default().coefficients(DEFAULT_SAMPLE_RATE)
    }
}

impl FilterCoefficients {
    #[must_use]
    pub(crate) fn is_svf(self) -> bool {
        self.mode == FilterMode::Svf
    }

    #[must_use]
    pub(crate) fn is_phaser(self) -> bool {
        self.mode == FilterMode::Phaser
    }

    #[must_use]
    pub(crate) fn is_scream(self) -> bool {
        self.mode == FilterMode::Scream
    }

    #[must_use]
    pub(crate) fn is_ratio_brickwall(self) -> bool {
        self.mode == FilterMode::RatioBrickwall
    }

    #[must_use]
    pub(crate) fn ratio_cutoff(self) -> f32 {
        self.cutoff_hz.clamp(MIN_RATIO, MAX_RATIO)
    }

    #[must_use]
    pub(crate) fn ratio_is_lowpass(self) -> bool {
        self.shape >= 0.5
    }

    #[must_use]
    pub(crate) fn modulated_cutoff(mut self, cutoff_octaves: f32) -> Self {
        self.cutoff_hz = if self.mode == FilterMode::RatioBrickwall {
            (self.cutoff_hz * fast_exp2(finite_or(cutoff_octaves, 0.0).clamp(-4.0, 4.0)))
                .clamp(MIN_RATIO, MAX_RATIO)
        } else {
            (self.cutoff_hz * fast_exp2(finite_or(cutoff_octaves, 0.0).clamp(-4.0, 4.0))).clamp(
                MIN_CUTOFF_HZ,
                COEFFICIENT_TABLE_SIZE as f32 / self.table_scale,
            )
        };
        if matches!(self.mode, FilterMode::Svf | FilterMode::Scream) {
            self.g = coefficient(self.cutoff_hz * self.table_scale, coefficient_table());
        }
        if self.mode == FilterMode::Scream {
            self.scream_hp_g = coefficient(
                (self.cutoff_hz * self.scream_hp_ratio).max(MIN_CUTOFF_HZ) * self.table_scale,
                coefficient_table(),
            );
        }
        self
    }

    #[must_use]
    pub(crate) fn modulated_resonance(mut self, resonance_octaves: f32) -> Self {
        let resonance_octaves = finite_or(resonance_octaves, 0.0).clamp(-4.0, 4.0);
        match self.mode {
            FilterMode::Svf => {
                self.q = (self.q * fast_exp2(resonance_octaves)).clamp(MIN_Q, MAX_Q);
                self.damping = svf_resonance_amount(self.q);
            }
            FilterMode::Phaser => {
                self.q = (self.q * fast_exp2(resonance_octaves)).clamp(MIN_Q, MAX_Q);
                self.damping = phaser_depth(self.q);
            }
            FilterMode::Scream => {
                let resonance = (self.damping + resonance_octaves / Q_OCTAVES).clamp(0.0, 1.0);
                self.damping = resonance;
                self.scream_hp_damping = lerp(
                    std::f32::consts::SQRT_2,
                    std::f32::consts::FRAC_1_SQRT_2,
                    resonance,
                );
                self.scream_feedback = scream_feedback(resonance);
            }
            FilterMode::RatioBrickwall => {}
        }
        self
    }

    #[must_use]
    pub(crate) fn modulated_slope(mut self, slope: f32) -> Self {
        self.slope_db_oct =
            (self.slope_db_oct + finite_or(slope, 0.0) * 12.0).clamp(MIN_SLOPE_DB, MAX_SLOPE_DB);
        match self.mode {
            FilterMode::Svf => {
                (self.stages, self.brickwall) = svf_shape(self.slope_db_oct, self.morph);
                (self.processing_stages, self.processing_blend) =
                    processing_stage_shape(self.mode, self.stages);
                self.morph_gain = svf_cutoff_gain(self);
                self.span_octaves = stage_span_octaves(self.processing_stages);
            }
            FilterMode::Phaser => {
                self.span_octaves = phase_span_octaves(self.slope_db_oct);
            }
            FilterMode::Scream => {
                self.scream_hp_ratio = scream_hp_ratio(self.slope_db_oct);
                self.scream_hp_g = coefficient(
                    (self.cutoff_hz * self.scream_hp_ratio).max(MIN_CUTOFF_HZ) * self.table_scale,
                    coefficient_table(),
                );
            }
            FilterMode::RatioBrickwall => {}
        }
        self
    }

    #[must_use]
    pub(crate) fn modulated_morph(mut self, morph: f32) -> Self {
        self.morph = (self.morph + finite_or(morph, 0.0)).clamp(0.0, 1.0);
        if self.mode == FilterMode::Phaser {
            self.stages = 1.0 + self.morph * (MAX_PHASE_POLES as f32 - 1.0);
            (self.processing_stages, self.processing_blend) =
                processing_stage_shape(self.mode, self.stages);
        } else if self.mode == FilterMode::Svf {
            (self.stages, self.brickwall) = svf_shape(self.slope_db_oct, self.morph);
            (self.processing_stages, self.processing_blend) =
                processing_stage_shape(self.mode, self.stages);
            self.morph_gain = svf_cutoff_gain(self);
        }
        self
    }

    #[must_use]
    pub(crate) fn modulated_shape(mut self, shape: f32) -> Self {
        self.shape = (self.shape + finite_or(shape, 0.0)).clamp(0.0, 1.0);
        self
    }

    #[must_use]
    pub(crate) fn interpolate(self, target: Self, amount: f32) -> Self {
        let amount = amount.clamp(0.0, 1.0);
        let stages = lerp(self.stages, target.stages, amount);
        let (processing_stages, processing_blend) = processing_stage_shape(target.mode, stages);
        Self {
            mode: target.mode,
            q: lerp(self.q, target.q, amount),
            slope_db_oct: lerp(self.slope_db_oct, target.slope_db_oct, amount),
            g: lerp(self.g, target.g, amount),
            damping: lerp(self.damping, target.damping, amount),
            morph: lerp(self.morph, target.morph, amount),
            shape: lerp(self.shape, target.shape, amount),
            morph_gain: lerp(self.morph_gain, target.morph_gain, amount),
            brickwall: lerp(self.brickwall, target.brickwall, amount),
            stages,
            processing_stages,
            processing_blend,
            span_octaves: lerp(self.span_octaves, target.span_octaves, amount),
            skew: lerp(self.skew, target.skew, amount),
            table_scale: lerp(self.table_scale, target.table_scale, amount),
            cutoff_hz: lerp(self.cutoff_hz, target.cutoff_hz, amount),
            scream_hp_g: lerp(self.scream_hp_g, target.scream_hp_g, amount),
            scream_hp_ratio: lerp(self.scream_hp_ratio, target.scream_hp_ratio, amount),
            scream_hp_damping: lerp(self.scream_hp_damping, target.scream_hp_damping, amount),
            scream_feedback: lerp(self.scream_feedback, target.scream_feedback, amount),
        }
    }

    #[cfg(test)]
    fn stage_damping(self, index: usize, damping_table: &[f32]) -> f32 {
        svf_stage_damping(self.stages, index, damping_table)
    }

    fn phase_center_cos_at(self, index: usize) -> f32 {
        let frequency = stage_frequency(
            self.mode,
            index,
            MAX_PHASE_SECTIONS as u8,
            self.cutoff_hz,
            self.span_octaves,
            self.skew,
        );
        phase_center_cos(frequency.max(MIN_CUTOFF_HZ) * self.table_scale)
    }

    pub(super) fn processing_stage_count(self) -> u8 {
        self.processing_stages
    }

    pub(super) fn processing_stage_blend(self) -> f32 {
        self.processing_blend
    }

    fn same_phase_topology(self, other: Self) -> bool {
        self.mode == other.mode
            && self.span_octaves.to_bits() == other.span_octaves.to_bits()
            && self.skew.to_bits() == other.skew.to_bits()
            && self.table_scale.to_bits() == other.table_scale.to_bits()
            && self.cutoff_hz.to_bits() == other.cutoff_hz.to_bits()
    }

    fn same_svf_topology(self, other: Self) -> bool {
        self.mode == other.mode
            && self.g.to_bits() == other.g.to_bits()
            && self.stages.to_bits() == other.stages.to_bits()
            && self.morph.to_bits() == other.morph.to_bits()
            && self.brickwall.to_bits() == other.brickwall.to_bits()
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct ComplexResponse {
    pub(super) real: f32,
    pub(super) imaginary: f32,
}

impl ComplexResponse {
    pub(super) const ONE: Self = Self {
        real: 1.0,
        imaginary: 0.0,
    };

    pub(super) fn magnitude(self) -> f32 {
        self.real.hypot(self.imaginary)
    }

    pub(super) fn scale(self, amount: f32) -> Self {
        Self {
            real: self.real * amount,
            imaginary: self.imaginary * amount,
        }
    }

    pub(super) fn add(self, other: Self) -> Self {
        Self {
            real: self.real + other.real,
            imaginary: self.imaginary + other.imaginary,
        }
    }

    pub(super) fn subtract(self, other: Self) -> Self {
        Self {
            real: self.real - other.real,
            imaginary: self.imaginary - other.imaginary,
        }
    }

    pub(super) fn multiply(self, other: Self) -> Self {
        Self {
            real: self.real * other.real - self.imaginary * other.imaginary,
            imaginary: self.real * other.imaginary + self.imaginary * other.real,
        }
    }

    pub(super) fn divide(self, other: Self) -> Self {
        let denominator = other.real * other.real + other.imaginary * other.imaginary;
        Self {
            real: (self.real * other.real + self.imaginary * other.imaginary) / denominator,
            imaginary: (self.imaginary * other.real - self.real * other.imaginary) / denominator,
        }
    }
}

fn response_at(
    coefficients: FilterCoefficients,
    frequency: f32,
    sample_rate: f32,
) -> ComplexResponse {
    match coefficients.mode {
        FilterMode::Svf => {
            let count = usize::from(coefficients.processing_stage_count().max(1));
            let damping_table = butterworth_damping_table();
            let layout = SvfStageLayout::new(coefficients);
            let resonance = svf_resonance_response(coefficients, frequency, sample_rate);
            let mut response = ComplexResponse::ONE;
            for index in 0..count {
                let stage_coeffs =
                    svf_stage_at_prepared(coefficients, layout, index, damping_table);
                let (low, band, high) = svf_stage_response(stage_coeffs, frequency, sample_rate);
                let stage = low
                    .scale(stage_coeffs.low_mix)
                    .add(band.scale(stage_coeffs.band_mix))
                    .add(high.scale(stage_coeffs.high_mix));
                let full = response.multiply(stage);
                response = if index + 1 == count {
                    response.add(
                        full.subtract(response)
                            .scale(svf_processing_blend(coefficients)),
                    )
                } else {
                    full
                };
            }
            resonance.multiply(response.scale(coefficients.morph_gain))
        }
        FilterMode::Phaser => {
            let count = coefficients.processing_stage_count().max(1) as usize;
            let blend = coefficients.processing_stage_blend();
            let width = phaser_notch_width(
                coefficients.shape,
                coefficients.processing_stage_count(),
                blend,
                coefficients.span_octaves,
            );
            let mut wet = ComplexResponse::ONE;
            for index in 0..count {
                let participation = if index + 1 == count { blend } else { 1.0 };
                let (radius, pole_cos) =
                    phaser_pole(width, coefficients.phase_center_cos_at(index));
                let phased =
                    second_order_allpass_response(pole_cos, radius, frequency, sample_rate);
                wet = wet.multiply(phase_mix_response(phased, participation));
            }
            ComplexResponse::ONE.add(
                wet.subtract(ComplexResponse::ONE)
                    .scale(coefficients.damping),
            )
        }
        FilterMode::Scream => scream_response(coefficients, frequency, sample_rate),
        FilterMode::RatioBrickwall => {
            let cutoff = coefficients.cutoff_hz;
            if ratio_brickwall_bypassed(cutoff, coefficients.shape >= 0.5)
                || coefficients.shape >= 0.5 && frequency <= cutoff
                || coefficients.shape < 0.5 && frequency > cutoff
            {
                ComplexResponse::ONE
            } else {
                ComplexResponse::default()
            }
        }
    }
}

pub(super) fn svf_stage_response(
    coefficients: StageCoefficients,
    frequency: f32,
    sample_rate: f32,
) -> (ComplexResponse, ComplexResponse, ComplexResponse) {
    let g = coefficients.a2 / coefficients.a1;
    let warped_frequency = (std::f32::consts::PI * frequency / sample_rate).tan();
    let denominator = ComplexResponse {
        real: g * g - warped_frequency * warped_frequency,
        imaginary: coefficients.damping * g * warped_frequency,
    };
    let low = ComplexResponse {
        real: g * g,
        imaginary: 0.0,
    }
    .divide(denominator);
    let band = ComplexResponse {
        real: 0.0,
        imaginary: g * warped_frequency,
    }
    .divide(denominator);
    let high = ComplexResponse {
        real: -(warped_frequency * warped_frequency),
        imaginary: 0.0,
    }
    .divide(denominator);
    (low, band, high)
}

pub(super) type StereoState = [f32x4; 2];

#[derive(Clone, Copy, Debug)]
pub struct StereoTptSvf {
    states: [StereoState; MAX_SVF_STAGES],
    resonance_state: StereoState,
    cached_coefficients: [f32; MAX_PHASE_POLES],
    cached_stage_values: [f32; MAX_PHASE_POLES],
    cached_damping: [f32; MAX_SVF_STAGES],
    cached_low_mix: [f32; MAX_SVF_STAGES],
    cached_band_mix: [f32; MAX_SVF_STAGES],
    cached_high_mix: [f32; MAX_SVF_STAGES],
    coefficient_cache: Option<FilterCoefficients>,
    cached_stages: u8,
    cached_phase_ratio_stages: u8,
    last_mode: FilterMode,
    last_active: u8,
    scream_feedback: f32x4,
    scream_peak: f32x4,
}

impl Default for StereoTptSvf {
    fn default() -> Self {
        Self {
            states: [[f32x4::ZERO; 2]; MAX_SVF_STAGES],
            resonance_state: [f32x4::ZERO; 2],
            cached_coefficients: [0.0; MAX_PHASE_POLES],
            cached_stage_values: [0.0; MAX_PHASE_POLES],
            cached_damping: [0.0; MAX_SVF_STAGES],
            cached_low_mix: [0.0; MAX_SVF_STAGES],
            cached_band_mix: [0.0; MAX_SVF_STAGES],
            cached_high_mix: [0.0; MAX_SVF_STAGES],
            coefficient_cache: None,
            cached_stages: 0,
            cached_phase_ratio_stages: 0,
            last_mode: FilterMode::Svf,
            last_active: 0,
            scream_feedback: f32x4::ZERO,
            scream_peak: f32x4::ZERO,
        }
    }
}

impl StereoTptSvf {
    pub fn reset(&mut self) {
        self.states.fill([f32x4::ZERO; 2]);
        self.resonance_state = [f32x4::ZERO; 2];
        self.coefficient_cache = None;
        self.cached_stages = 0;
        self.cached_phase_ratio_stages = 0;
        self.last_active = 0;
        self.scream_feedback = f32x4::ZERO;
        self.scream_peak = f32x4::ZERO;
    }

    pub(crate) fn copy_static_state_from(
        &mut self,
        source: &Self,
        coefficients: FilterCoefficients,
    ) {
        let active = usize::from(coefficients.processing_stage_count());
        let state_count = match coefficients.mode {
            FilterMode::Svf => active,
            FilterMode::Phaser => active,
            FilterMode::Scream => 2,
            FilterMode::RatioBrickwall => 0,
        };
        self.states[..state_count].copy_from_slice(&source.states[..state_count]);
        self.resonance_state = source.resonance_state;
        match coefficients.mode {
            FilterMode::Svf => {
                self.cached_coefficients[..active]
                    .copy_from_slice(&source.cached_coefficients[..active]);
                self.cached_coefficients[MAX_SVF_STAGES..MAX_SVF_STAGES + active].copy_from_slice(
                    &source.cached_coefficients[MAX_SVF_STAGES..MAX_SVF_STAGES + active],
                );
                self.cached_damping[..active].copy_from_slice(&source.cached_damping[..active]);
                self.cached_low_mix[..active].copy_from_slice(&source.cached_low_mix[..active]);
                self.cached_band_mix[..active].copy_from_slice(&source.cached_band_mix[..active]);
                self.cached_high_mix[..active].copy_from_slice(&source.cached_high_mix[..active]);
            }
            FilterMode::Phaser => {
                self.cached_coefficients[..active]
                    .copy_from_slice(&source.cached_coefficients[..active]);
                self.cached_damping[..active].copy_from_slice(&source.cached_damping[..active]);
                self.cached_low_mix[..active].copy_from_slice(&source.cached_low_mix[..active]);
                self.cached_band_mix[..active].copy_from_slice(&source.cached_band_mix[..active]);
            }
            FilterMode::Scream => {
                self.scream_feedback = source.scream_feedback;
                self.scream_peak = source.scream_peak;
            }
            FilterMode::RatioBrickwall => {}
        }
        self.coefficient_cache = source.coefficient_cache;
        self.cached_stages = source.cached_stages;
        self.cached_stage_values = source.cached_stage_values;
        self.cached_phase_ratio_stages = source.cached_phase_ratio_stages;
        self.last_mode = source.last_mode;
        self.last_active = source.last_active;
    }

    fn prepare_phase_coefficients(&mut self, coefficients: FilterCoefficients) -> bool {
        let active = coefficients.processing_stage_count();
        let same_geometry = self.coefficient_cache.is_some_and(|cached| {
            cached.same_phase_topology(coefficients)
                && cached.processing_stage_count() == active
                && cached.processing_stage_blend().to_bits()
                    == coefficients.processing_stage_blend().to_bits()
                && cached.shape.to_bits() == coefficients.shape.to_bits()
        });
        let same_layout = self.coefficient_cache.is_some_and(|cached| {
            cached.mode == coefficients.mode
                && cached.span_octaves.to_bits() == coefficients.span_octaves.to_bits()
                && cached.skew.to_bits() == coefficients.skew.to_bits()
        });
        let ratio_start = if same_layout {
            self.cached_phase_ratio_stages
        } else {
            0
        };
        let mut ratio_index = usize::from(ratio_start);
        if coefficients.skew == 0.5 {
            let span_position = (coefficients.span_octaves - MIN_PHASE_SPAN_OCTAVES)
                * (PHASE_SPAN_TABLE_SIZE as f32
                    / (MAX_PHASE_SPAN_OCTAVES - MIN_PHASE_SPAN_OCTAVES));
            let span_index = (span_position as usize).min(PHASE_SPAN_TABLE_SIZE - 1);
            let span_blend = span_position - span_index as f32;
            let table = phase_ratio_table();
            let lower = span_index * MAX_PHASE_POLES;
            let upper = lower + MAX_PHASE_POLES;
            let end = usize::from(active);
            ratio_index = ratio_index.min(end);
            let (low_chunks, _) = table[lower + ratio_index..lower + end].as_chunks::<4>();
            let (high_chunks, _) = table[upper + ratio_index..upper + end].as_chunks::<4>();
            let (output_chunks, _) =
                self.cached_stage_values[ratio_index..end].as_chunks_mut::<4>();
            for ((low, high), output) in low_chunks.iter().zip(high_chunks).zip(output_chunks) {
                let low = f32x4::from(*low);
                let high = f32x4::from(*high);
                let ratios = low + (high - low) * f32x4::splat(span_blend);
                *output = ratios.to_array();
            }
            ratio_index += low_chunks.len() * 4;
        }
        for index in ratio_index..usize::from(active) {
            let ratio = phase_frequency_ratio(index, coefficients.span_octaves, coefficients.skew);
            self.cached_stage_values[index] = ratio;
        }
        self.cached_phase_ratio_stages = if same_layout {
            self.cached_phase_ratio_stages.max(active)
        } else {
            active
        };
        let same_topology = self
            .coefficient_cache
            .is_some_and(|cached| cached.same_phase_topology(coefficients));
        let start = if same_topology { self.cached_stages } else { 0 };
        let end = usize::from(active);
        let minimum = MIN_CUTOFF_HZ * coefficients.table_scale;
        let scale = coefficients.cutoff_hz * coefficients.table_scale;
        let mut index = usize::from(start).min(end);
        let coefficient_start = index;
        let (ratio_chunks, _) = self.cached_stage_values[index..end].as_chunks::<4>();
        let (coefficient_chunks, _) = self.cached_coefficients[index..end].as_chunks_mut::<4>();
        for (ratios, output) in ratio_chunks.iter().zip(coefficient_chunks) {
            *output = (f32x4::from(*ratios) * f32x4::splat(scale))
                .max(f32x4::splat(minimum))
                .min(f32x4::splat(COEFFICIENT_TABLE_SIZE as f32))
                .to_array();
        }
        index += ratio_chunks.len() * 4;
        for index in index..end {
            let frequency = coefficients.cutoff_hz * self.cached_stage_values[index];
            self.cached_coefficients[index] = (frequency.max(MIN_CUTOFF_HZ)
                * coefficients.table_scale)
                .min(COEFFICIENT_TABLE_SIZE as f32);
        }
        for index in coefficient_start..end {
            self.cached_high_mix[index] = phase_coefficient(self.cached_coefficients[index]);
        }
        self.cached_stages = if same_topology {
            self.cached_stages.max(active)
        } else {
            active
        };
        self.coefficient_cache = Some(coefficients);
        !same_geometry
    }

    fn prepare_phaser_geometry(&mut self, coefficients: FilterCoefficients) {
        let active = coefficients.processing_stage_count();
        let width = phaser_notch_width(
            coefficients.shape,
            active,
            coefficients.processing_stage_blend(),
            coefficients.span_octaves,
        );
        let width_ratios = phaser_width_ratios(width);
        let end = usize::from(active.max(1));
        let mut index = 0;
        while index + 4 <= end {
            let (radii, pole_cosines) = phaser_poles4_at_prepared_positions(
                width_ratios,
                self.cached_coefficients[index..index + 4]
                    .try_into()
                    .unwrap(),
                self.cached_high_mix[index..index + 4].try_into().unwrap(),
            );
            self.cached_low_mix[index..index + 4].copy_from_slice(&radii);
            self.cached_band_mix[index..index + 4].copy_from_slice(&pole_cosines);
            index += 4;
        }
        for index in index..end {
            let (radius, pole_cos) = phaser_pole_at_prepared_position(
                width_ratios,
                self.cached_coefficients[index],
                self.cached_high_mix[index],
            );
            self.cached_low_mix[index] = radius;
            self.cached_band_mix[index] = pole_cos;
        }
    }

    fn prepare_svf_coefficients(&mut self, coefficients: FilterCoefficients) {
        if self
            .coefficient_cache
            .is_some_and(|cached| cached.same_svf_topology(coefficients))
        {
            return;
        }
        let count = usize::from(coefficients.processing_stage_count().max(1));
        let damping_table = butterworth_damping_table();
        let layout = SvfStageLayout::new(coefficients);
        for index in 0..count {
            let stage = svf_stage_at_prepared(coefficients, layout, index, damping_table);
            self.cached_damping[index] = stage.damping;
            self.cached_coefficients[index] = stage.a1;
            self.cached_coefficients[MAX_SVF_STAGES + index] = stage.a2;
            self.cached_stage_values[index] = stage.a3;
            self.cached_low_mix[index] = stage.low_mix;
            self.cached_band_mix[index] = stage.band_mix;
            self.cached_high_mix[index] = stage.high_mix;
        }
        self.cached_stages = count as u8;
        self.coefficient_cache = Some(coefficients);
    }

    #[inline]
    pub(crate) fn prepare_phaser(&mut self, coefficients: FilterCoefficients) {
        debug_assert!(coefficients.is_phaser());
        if coefficients.mode != self.last_mode {
            self.reset();
            self.last_mode = coefficients.mode;
        }
        if self.prepare_phase_coefficients(coefficients) {
            self.prepare_phaser_geometry(coefficients);
        }
    }

    #[inline]
    pub(crate) fn prepare_svf(&mut self, coefficients: FilterCoefficients) {
        debug_assert!(coefficients.is_svf());
        if coefficients.mode != self.last_mode {
            self.reset();
            self.last_mode = coefficients.mode;
        }
        self.prepare_svf_coefficients(coefficients);
    }

    #[must_use]
    #[inline]
    pub(crate) fn process(
        &mut self,
        coefficients: FilterCoefficients,
        left: f32,
        right: f32,
    ) -> (f32, f32) {
        if coefficients.mode != self.last_mode {
            self.reset();
            self.last_mode = coefficients.mode;
        }
        let previous_active = self.last_active;
        self.last_active = coefficients.processing_stage_count();
        let input = f32x4::from([finite_or(left, 0.0), finite_or(right, 0.0), 0.0, 0.0]);
        let output = match coefficients.mode {
            FilterMode::Svf => {
                self.prepare_svf_coefficients(coefficients);
                process_svf(
                    &mut self.states,
                    &mut self.resonance_state,
                    &self.cached_coefficients,
                    &self.cached_stage_values,
                    &self.cached_damping,
                    &self.cached_low_mix,
                    &self.cached_band_mix,
                    &self.cached_high_mix,
                    input,
                    coefficients,
                )
            }
            FilterMode::Phaser => {
                if self.prepare_phase_coefficients(coefficients) {
                    self.prepare_phaser_geometry(coefficients);
                }
                process_phase_bank(
                    &mut self.states,
                    &mut self.cached_damping,
                    &self.cached_low_mix,
                    &self.cached_band_mix,
                    input,
                    coefficients.processing_stage_count(),
                    coefficients.processing_stage_blend(),
                    coefficients.damping,
                    previous_active,
                )
            }
            FilterMode::Scream => process_scream(
                &mut self.states,
                &mut self.scream_feedback,
                &mut self.scream_peak,
                input,
                coefficients.g,
                coefficients.scream_hp_g,
                coefficients.scream_hp_damping,
                coefficients.scream_feedback,
                coefficients.morph,
            ),
            FilterMode::RatioBrickwall => input,
        }
        .to_array();
        if output[0].is_finite() && output[1].is_finite() {
            (output[0], output[1])
        } else {
            self.reset();
            (0.0, 0.0)
        }
    }

    #[must_use]
    #[inline]
    pub(crate) fn process_prepared_phaser(
        &mut self,
        coefficients: FilterCoefficients,
        left: f32,
        right: f32,
    ) -> (f32, f32) {
        debug_assert!(coefficients.is_phaser());
        self.process_prepared_phaser_parts(
            coefficients.processing_stage_count(),
            coefficients.processing_stage_blend(),
            coefficients.damping,
            coefficients.shape,
            coefficients.span_octaves,
            false,
            left,
            right,
        )
    }

    #[must_use]
    #[inline]
    pub(crate) fn process_prepared_svf_resonance(
        &mut self,
        coefficients: &FilterCoefficients,
        resonance_octaves: f32,
        left: f32,
        right: f32,
    ) -> (f32, f32) {
        debug_assert!(coefficients.is_svf());
        let modulated = coefficients.modulated_resonance(resonance_octaves);
        let input = f32x4::from([finite_or(left, 0.0), finite_or(right, 0.0), 0.0, 0.0]);
        let output = process_svf(
            &mut self.states,
            &mut self.resonance_state,
            &self.cached_coefficients,
            &self.cached_stage_values,
            &self.cached_damping,
            &self.cached_low_mix,
            &self.cached_band_mix,
            &self.cached_high_mix,
            input,
            modulated,
        )
        .to_array();
        if output[0].is_finite() && output[1].is_finite() {
            (output[0], output[1])
        } else {
            self.reset();
            (0.0, 0.0)
        }
    }

    #[must_use]
    #[inline]
    pub(crate) fn process_scream_resonance(
        &mut self,
        coefficients: &FilterCoefficients,
        resonance_octaves: f32,
        left: f32,
        right: f32,
    ) -> (f32, f32) {
        debug_assert!(coefficients.is_scream());
        let resonance = (coefficients.damping
            + finite_or(resonance_octaves, 0.0).clamp(-4.0, 4.0) / Q_OCTAVES)
            .clamp(0.0, 1.0);
        if self.last_mode != FilterMode::Scream {
            self.reset();
            self.last_mode = FilterMode::Scream;
        }
        let input = f32x4::from([finite_or(left, 0.0), finite_or(right, 0.0), 0.0, 0.0]);
        let output = process_scream(
            &mut self.states,
            &mut self.scream_feedback,
            &mut self.scream_peak,
            input,
            coefficients.g,
            coefficients.scream_hp_g,
            lerp(
                std::f32::consts::SQRT_2,
                std::f32::consts::FRAC_1_SQRT_2,
                resonance,
            ),
            scream_feedback(resonance),
            coefficients.morph,
        )
        .to_array();
        if output[0].is_finite() && output[1].is_finite() {
            (output[0], output[1])
        } else {
            self.reset();
            (0.0, 0.0)
        }
    }

    #[must_use]
    pub(crate) fn process_scream_slope(
        &mut self,
        coefficients: &FilterCoefficients,
        slope: f32,
        left: f32,
        right: f32,
    ) -> (f32, f32) {
        debug_assert!(coefficients.is_scream());
        let slope_db_oct = (coefficients.slope_db_oct + finite_or(slope, 0.0) * 12.0)
            .clamp(MIN_SLOPE_DB, MAX_SLOPE_DB);
        let hp_ratio = scream_hp_ratio(slope_db_oct);
        let hp_g = coefficient(
            (coefficients.cutoff_hz * hp_ratio).max(MIN_CUTOFF_HZ) * coefficients.table_scale,
            coefficient_table(),
        );
        self.process_scream_parts(
            coefficients,
            hp_g,
            coefficients.scream_hp_damping,
            coefficients.scream_feedback,
            coefficients.morph,
            left,
            right,
        )
    }

    #[must_use]
    pub(crate) fn process_scream_morph(
        &mut self,
        coefficients: &FilterCoefficients,
        morph: f32,
        left: f32,
        right: f32,
    ) -> (f32, f32) {
        debug_assert!(coefficients.is_scream());
        self.process_scream_parts(
            coefficients,
            coefficients.scream_hp_g,
            coefficients.scream_hp_damping,
            coefficients.scream_feedback,
            (coefficients.morph + finite_or(morph, 0.0)).clamp(0.0, 1.0),
            left,
            right,
        )
    }

    #[inline]
    fn process_scream_parts(
        &mut self,
        coefficients: &FilterCoefficients,
        hp_g: f32,
        hp_damping: f32,
        feedback_gain: f32,
        morph: f32,
        left: f32,
        right: f32,
    ) -> (f32, f32) {
        if self.last_mode != FilterMode::Scream {
            self.reset();
            self.last_mode = FilterMode::Scream;
        }
        let input = f32x4::from([finite_or(left, 0.0), finite_or(right, 0.0), 0.0, 0.0]);
        let output = process_scream(
            &mut self.states,
            &mut self.scream_feedback,
            &mut self.scream_peak,
            input,
            coefficients.g,
            hp_g,
            hp_damping,
            feedback_gain,
            morph,
        )
        .to_array();
        if output[0].is_finite() && output[1].is_finite() {
            (output[0], output[1])
        } else {
            self.reset();
            (0.0, 0.0)
        }
    }

    #[must_use]
    #[inline]
    pub(crate) fn process_prepared_phaser_resonance(
        &mut self,
        coefficients: &FilterCoefficients,
        resonance_octaves: f32,
        left: f32,
        right: f32,
    ) -> (f32, f32) {
        debug_assert!(coefficients.is_phaser());
        let depth = modulated_phaser_depth(
            coefficients.damping,
            finite_or(resonance_octaves, 0.0).clamp(-4.0, 4.0),
        );
        self.process_prepared_phaser_parts(
            coefficients.processing_stage_count(),
            coefficients.processing_stage_blend(),
            depth,
            coefficients.shape,
            coefficients.span_octaves,
            false,
            left,
            right,
        )
    }

    #[must_use]
    #[inline]
    pub(crate) fn process_prepared_phaser_shape(
        &mut self,
        coefficients: &FilterCoefficients,
        shape: f32,
        left: f32,
        right: f32,
    ) -> (f32, f32) {
        debug_assert!(coefficients.is_phaser());
        self.process_prepared_phaser_parts(
            coefficients.processing_stage_count(),
            coefficients.processing_stage_blend(),
            coefficients.damping,
            (coefficients.shape + finite_or(shape, 0.0)).clamp(0.0, 1.0),
            coefficients.span_octaves,
            true,
            left,
            right,
        )
    }

    #[inline]
    fn process_prepared_phaser_parts(
        &mut self,
        active: u8,
        blend: f32,
        depth: f32,
        shape: f32,
        span_octaves: f32,
        geometry_dynamic: bool,
        left: f32,
        right: f32,
    ) -> (f32, f32) {
        let previous_active = self.last_active;
        self.last_active = active;
        let input = f32x4::from([finite_or(left, 0.0), finite_or(right, 0.0), 0.0, 0.0]);
        if geometry_dynamic {
            let width = phaser_notch_width(shape, active, blend, span_octaves);
            let width_ratios = phaser_width_ratios(width);
            for index in 0..usize::from(active.max(1)) {
                let (radius, pole_cos) =
                    phaser_pole_at_position(width_ratios, self.cached_coefficients[index]);
                self.cached_low_mix[index] = radius;
                self.cached_band_mix[index] = pole_cos;
            }
        }
        let output = process_phase_bank(
            &mut self.states,
            &mut self.cached_damping,
            &self.cached_low_mix,
            &self.cached_band_mix,
            input,
            active,
            blend,
            depth,
            previous_active,
        )
        .to_array();
        if output[0].is_finite() && output[1].is_finite() {
            (output[0], output[1])
        } else {
            self.reset();
            (0.0, 0.0)
        }
    }
}

#[inline]
pub(super) fn tick_svf(
    state: &mut StereoState,
    input: f32x4,
    coefficients: StageCoefficients,
) -> (f32x4, f32x4, f32x4) {
    let v3 = input - state[1];
    let band = f32x4::splat(coefficients.a1) * state[0] + f32x4::splat(coefficients.a2) * v3;
    let low =
        f32x4::splat(coefficients.a2) * state[0] + f32x4::splat(coefficients.a3) * v3 + state[1];
    let high = input - low - f32x4::splat(coefficients.damping) * band;
    state[0] = band * f32x4::splat(2.0) - state[0];
    state[1] = low * f32x4::splat(2.0) - state[1];
    (low, band, high)
}

#[inline]
pub(super) fn lerp(from: f32, to: f32, amount: f32) -> f32 {
    from + amount * (to - from)
}

fn progressive_stage_count(stages: f32, maximum: usize) -> (u8, f32) {
    let stages = stages.clamp(1.0, maximum as f32);
    let whole = stages.floor();
    let fraction = stages - whole;
    if fraction <= 1.0e-4 {
        (whole as u8, 1.0)
    } else {
        ((whole as u8 + 1).min(maximum as u8), fraction)
    }
}

fn processing_stage_shape(mode: FilterMode, stages: f32) -> (u8, f32) {
    match mode {
        FilterMode::Svf => svf_stage_shape(stages),
        FilterMode::Phaser => {
            let sections = stages * 0.5;
            if sections < 1.0 {
                (1, sections)
            } else {
                progressive_stage_count(sections, MAX_PHASE_SECTIONS)
            }
        }
        FilterMode::Scream => (2, 1.0),
        FilterMode::RatioBrickwall => (1, 1.0),
    }
}

pub(super) fn normalized_log(value: f32, minimum: f32, maximum: f32) -> f32 {
    (value.clamp(minimum, maximum) / minimum).ln() / (maximum / minimum).ln()
}

fn stage_span_octaves(active_stages: u8) -> f32 {
    (0.55 + 0.075 * f32::from(active_stages)).min(5.5)
}

pub(super) fn coefficient_table() -> &'static [f32] {
    COEFFICIENT_TABLE.get_or_init(|| {
        (0..=COEFFICIENT_TABLE_SIZE)
            .map(|index| {
                let ratio = NYQUIST_GUARD * index as f32 / COEFFICIENT_TABLE_SIZE as f32;
                (std::f32::consts::PI * ratio).tan()
            })
            .collect::<Vec<_>>()
            .into_boxed_slice()
    })
}

pub(super) fn slope_table_position(slope_db_oct: f32) -> (usize, f32) {
    let position = (slope_db_oct.clamp(MIN_SLOPE_DB, MAX_SLOPE_DB) - MIN_SLOPE_DB)
        * (COEFFICIENT_TABLE_SIZE as f32 / (MAX_SLOPE_DB - MIN_SLOPE_DB));
    let index = (position as usize).min(COEFFICIENT_TABLE_SIZE - 1);
    (index, position - index as f32)
}

#[inline]
pub(super) fn coefficient(position: f32, table: &[f32]) -> f32 {
    let position = position.clamp(0.0, COEFFICIENT_TABLE_SIZE as f32);
    let index = (position as usize).min(COEFFICIENT_TABLE_SIZE - 1);
    let amount = position - index as f32;
    table[index] + amount * (table[index + 1] - table[index])
}

#[inline]
fn sanitize_sample_rate(sample_rate: f32) -> f32 {
    if sample_rate.is_finite() && sample_rate >= 1.0 {
        sample_rate
    } else {
        DEFAULT_SAMPLE_RATE
    }
}

pub(super) fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_SAMPLE_RATE: f32 = 48_000.0;
    const ANALYSIS_SAMPLES: usize = 4_096;

    #[test]
    fn analytical_response_matches_realtime_filter_for_all_modes() {
        for mode in FilterMode::ALL {
            let config = FilterConfig {
                mode,
                cutoff_hz: 1_370.0,
                q: 1.4,
                slope_db_oct: 19.0,
                morph: 0.63,
                shape: 0.0,
            };
            for frequency in [375.0, 1_500.0, 6_000.0] {
                let analytical = config.response_magnitude(frequency, TEST_SAMPLE_RATE);
                let amplitude = if mode == FilterMode::Scream {
                    SCREAM_PREVIEW_INPUT_PEAK
                } else {
                    1.0e-3
                };
                let measured = measured_response(config, frequency, amplitude);
                assert!(
                    (analytical - measured).abs() < 0.03,
                    "{mode:?} at {frequency} Hz: analytical={analytical}, measured={measured}"
                );
            }
        }
    }

    #[test]
    fn cutoff_only_fast_path_matches_full_processing() {
        for mode in FilterMode::ALL {
            let config = FilterConfig {
                mode,
                cutoff_hz: 1_370.0,
                q: 1.4,
                slope_db_oct: 36.0,
                morph: 0.63,
                shape: 0.0,
            };
            let base = config.coefficients(TEST_SAMPLE_RATE);
            let mut fast = StereoTptSvf::default();
            let mut full = StereoTptSvf::default();
            for sample in 0..256 {
                let modulation = (sample as f32 * 0.037).sin() * 3.5;
                let input = (sample as f32 * 0.23).sin() * 0.2;
                let fast_output = fast.process(base.modulated_cutoff(modulation), input, -input);
                let full_output = full.process(
                    config
                        .modulated(modulation, 0.0, 0.0, 0.0, 0.0)
                        .coefficients(TEST_SAMPLE_RATE),
                    input,
                    -input,
                );
                assert!((fast_output.0 - full_output.0).abs() < 1.0e-6, "{mode:?}");
                assert!((fast_output.1 - full_output.1).abs() < 1.0e-6, "{mode:?}");
            }
        }
    }

    #[test]
    fn prepared_phaser_matches_checked_processing() {
        let coefficients = FilterConfig {
            mode: FilterMode::Phaser,
            cutoff_hz: 1_370.0,
            q: 1.4,
            slope_db_oct: 96.0,
            morph: 0.63,
            shape: 0.0,
        }
        .coefficients(TEST_SAMPLE_RATE);
        let mut prepared = StereoTptSvf::default();
        let mut checked = StereoTptSvf::default();
        prepared.prepare_phaser(coefficients);
        for sample in 0..256 {
            let input = (sample as f32 * 0.23).sin() * 0.2;
            assert_eq!(
                prepared.process_prepared_phaser(coefficients, input, -input),
                checked.process(coefficients, input, -input)
            );
        }
        for sample in 0..256 {
            let input = (sample as f32 * 0.19).sin() * 0.2;
            let shape = 0.5 + 0.5 * (sample as f32 * 0.07).sin();
            assert_eq!(
                prepared.process_prepared_phaser_shape(&coefficients, shape, input, -input),
                checked.process(coefficients.modulated_shape(shape), input, -input)
            );
        }
    }

    #[test]
    fn other_single_control_fast_paths_match_full_processing() {
        for mode in FilterMode::ALL {
            let config = FilterConfig {
                mode,
                cutoff_hz: 1_370.0,
                q: 1.4,
                slope_db_oct: 36.0,
                morph: 0.63,
                shape: 0.0,
            };
            for control in 0..4 {
                let base = config.coefficients(TEST_SAMPLE_RATE);
                let mut fast = StereoTptSvf::default();
                let mut full = StereoTptSvf::default();
                for sample in 0..256 {
                    let modulation = (sample as f32 * 0.037).sin() * 0.75;
                    let input = (sample as f32 * 0.23).sin() * 0.2;
                    let (fast_coefficients, full_config) = match control {
                        0 => (
                            base.modulated_resonance(modulation * 4.0),
                            config.modulated(0.0, modulation * 4.0, 0.0, 0.0, 0.0),
                        ),
                        1 => (
                            base.modulated_slope(modulation),
                            config.modulated(0.0, 0.0, modulation, 0.0, 0.0),
                        ),
                        2 => (
                            base.modulated_morph(modulation),
                            config.modulated(0.0, 0.0, 0.0, modulation, 0.0),
                        ),
                        _ => (
                            base.modulated_shape(modulation),
                            config.modulated(0.0, 0.0, 0.0, 0.0, modulation),
                        ),
                    };
                    let fast_output = fast.process(fast_coefficients, input, -input);
                    let full_output =
                        full.process(full_config.coefficients(TEST_SAMPLE_RATE), input, -input);
                    assert!((fast_output.0 - full_output.0).abs() < 1.0e-5, "{mode:?}");
                    assert!((fast_output.1 - full_output.1).abs() < 1.0e-5, "{mode:?}");
                }
            }
        }
    }

    #[test]
    fn maximum_order_morph_sweep_does_not_compound_gain() {
        let base = FilterConfig {
            mode: FilterMode::Svf,
            cutoff_hz: 1_000.0,
            q: std::f32::consts::FRAC_1_SQRT_2,
            slope_db_oct: MAX_SLOPE_DB,
            morph: 0.5,
            shape: 0.0,
        };
        let mut filter = StereoTptSvf::default();
        let mut peak = 0.0_f32;
        for sample in 0..48_000 {
            let time = sample as f32 / TEST_SAMPLE_RATE;
            let morph = 0.5 + 0.5 * (std::f32::consts::TAU * 17.0 * time).sin();
            let input = 0.2 * (std::f32::consts::TAU * 137.0 * time).sin();
            let (output, _) = filter.process(
                FilterConfig { morph, ..base }.coefficients(TEST_SAMPLE_RATE),
                input,
                input,
            );
            peak = peak.max(output.abs());
        }
        assert!(peak < 1.0, "morph sweep peak {peak}");
    }

    #[test]
    fn slope_starts_at_a_complete_twelve_db_stage() {
        assert_eq!(slope_to_svf_stages(6.0), (1, 1.0));
        assert_eq!(slope_to_svf_stages(12.0), (1, 1.0));
        assert_eq!(slope_to_svf_stages(18.0), (2, 0.5));
        assert_eq!(slope_to_svf_stages(768.0), (64, 1.0));
        assert_eq!(
            FilterConfig {
                slope_db_oct: 6.0,
                ..FilterConfig::default()
            }
            .sanitized()
            .slope_db_oct,
            12.0
        );
        assert_eq!(
            FilterConfig {
                mode: FilterMode::Phaser,
                slope_db_oct: 6.0,
                ..FilterConfig::default()
            }
            .sanitized()
            .slope_db_oct,
            6.0
        );
    }

    #[test]
    fn svf_pole_distribution_is_continuous_across_order_boundary() {
        let damping = butterworth_damping_table();
        let below = FilterConfig {
            slope_db_oct: 24.0 - 1.0e-3,
            ..FilterConfig::default()
        }
        .coefficients(TEST_SAMPLE_RATE);
        let above = FilterConfig {
            slope_db_oct: 24.0 + 1.0e-3,
            ..FilterConfig::default()
        }
        .coefficients(TEST_SAMPLE_RATE);
        for index in 0..2 {
            assert!(
                (below.stage_damping(index, damping) - above.stage_damping(index, damping)).abs()
                    < 1.0e-3
            );
        }
    }

    #[test]
    fn phaser_morph_scans_one_to_128_poles() {
        let mut config = FilterConfig {
            mode: FilterMode::Phaser,
            slope_db_oct: MAX_SLOPE_DB,
            ..FilterConfig::default()
        };
        config.morph = 0.0;
        assert_eq!(config.effective_poles(), 1.0);
        config.morph = 1.0;
        assert_eq!(config.effective_poles(), 128.0);
    }

    #[test]
    fn maximum_svf_slope_is_monotonic_without_low_q_peaking() {
        let config = FilterConfig {
            cutoff_hz: 1_000.0,
            q: MIN_Q,
            slope_db_oct: MAX_SLOPE_DB,
            morph: 0.0,
            ..FilterConfig::default()
        };
        let mut previous = 1.0;
        for frequency in (1..=200).map(|index| index as f32 * 10.0) {
            let magnitude = config.response_magnitude(frequency, TEST_SAMPLE_RATE);
            assert!(magnitude <= previous + 1.0e-4, "peak at {frequency} Hz");
            previous = magnitude;
        }
    }

    #[test]
    fn fractional_svf_orders_are_monotonic_without_low_q_peaking() {
        for step in 0..=127 {
            let config = FilterConfig {
                cutoff_hz: 1_000.0,
                q: MIN_Q,
                slope_db_oct: MIN_SVF_SLOPE_DB
                    + (MAX_SLOPE_DB - MIN_SVF_SLOPE_DB) * step as f32 / 127.0,
                morph: 0.0,
                ..FilterConfig::default()
            };
            let mut previous = config.response_magnitude(1_000.0, TEST_SAMPLE_RATE);
            for frequency in (101..=400).map(|index| index as f32 * 10.0) {
                let magnitude = config.response_magnitude(frequency, TEST_SAMPLE_RATE);
                assert!(
                    magnitude <= previous + 1.0e-4,
                    "slope={} peak at {frequency} Hz: {previous} -> {magnitude}",
                    config.slope_db_oct
                );
                previous = magnitude;
            }
            assert!(
                previous < 0.08,
                "slope={} weak stopband",
                config.slope_db_oct
            );
        }
    }

    #[test]
    fn svf_resonance_stays_finite_and_within_declared_q() {
        for step in 0..=127 {
            let config = FilterConfig {
                cutoff_hz: 1_000.0,
                q: MAX_Q,
                slope_db_oct: MIN_SVF_SLOPE_DB
                    + (MAX_SLOPE_DB - MIN_SVF_SLOPE_DB) * step as f32 / 127.0,
                morph: 0.0,
                ..FilterConfig::default()
            };
            let peak = (0..=1_024)
                .map(|index| {
                    let unit = index as f32 / 1_024.0;
                    let frequency = 20.0 * 1_000.0_f32.powf(unit);
                    config.response_magnitude(frequency, TEST_SAMPLE_RATE)
                })
                .fold(0.0_f32, f32::max);
            assert!(peak.is_finite());
            assert!(
                peak <= MAX_Q * 1.01,
                "slope={} exceeded Q bound: {peak}",
                config.slope_db_oct
            );
        }
    }

    #[test]
    fn minimum_phaser_q_is_dry() {
        let config = FilterConfig {
            mode: FilterMode::Phaser,
            q: MIN_Q,
            morph: 1.0,
            ..FilterConfig::default()
        };
        for frequency in [20.0, 1_000.0, 20_000.0] {
            assert!((config.response_magnitude(frequency, TEST_SAMPLE_RATE) - 1.0).abs() < 1.0e-5);
        }
    }

    #[test]
    fn phaser_shape_keeps_notches_on_the_designed_center() {
        let base = FilterConfig {
            mode: FilterMode::Phaser,
            q: MAX_Q,
            cutoff_hz: 1_000.0,
            morph: 1.0 / 127.0,
            slope_db_oct: MIN_SLOPE_DB,
            ..FilterConfig::default()
        };
        for shape in [0.0, 0.35, 0.7, 1.0] {
            let config = FilterConfig { shape, ..base };
            let notch = config
                .phaser_notch_frequency(0, TEST_SAMPLE_RATE, 20.0, 20_000.0)
                .expect("notch");
            let ratio = notch / config.cutoff_hz;
            assert!(
                (ratio.ln().abs()) < 0.08,
                "shape {shape} walked the notch to {notch} Hz (ratio {ratio})"
            );
            assert!(config.response_magnitude(notch, TEST_SAMPLE_RATE) < 1.0e-3);
        }
    }

    #[test]
    fn phaser_brick_widens_notches_instead_of_thinning_them() {
        let base = FilterConfig {
            mode: FilterMode::Phaser,
            q: MAX_Q,
            cutoff_hz: 1_000.0,
            morph: 1.0 / 127.0,
            slope_db_oct: MIN_SLOPE_DB,
            ..FilterConfig::default()
        };
        let width = |shape: f32| {
            let config = FilterConfig { shape, ..base };
            let notch = config
                .phaser_notch_frequency(0, TEST_SAMPLE_RATE, 20.0, 20_000.0)
                .expect("notch");
            let mut lo = notch;
            let mut hi = notch * 8.0;
            for _ in 0..24 {
                let middle = (lo * hi).sqrt();
                if config.response_magnitude(middle, TEST_SAMPLE_RATE) > 0.5 {
                    hi = middle;
                } else {
                    lo = middle;
                }
            }
            hi / notch
        };
        let broad = width(0.0);
        let mid = width(0.5);
        let brick = width(1.0);
        assert!(
            mid > broad,
            "mid shape should already widen the notch: {mid} vs {broad}"
        );
        assert!(brick > mid, "brick should be wider still: {brick} vs {mid}");
        assert!(
            brick > broad * 1.25,
            "brick should be substantially wider than broad: {brick} vs {broad}"
        );
    }

    #[test]
    fn svf_brickwall_thins_the_cutoff_pole() {
        let damping = butterworth_damping_table();
        let at = |slope: f32| {
            let coefficients = FilterConfig {
                cutoff_hz: 1_000.0,
                slope_db_oct: slope,
                morph: 0.0,
                ..FilterConfig::default()
            }
            .coefficients(TEST_SAMPLE_RATE);
            let layout = SvfStageLayout::new(coefficients);
            svf_stage_at_prepared(coefficients, layout, 0, damping).damping
        };
        let continuous = at(96.0);
        let approaching = at(99.0);
        let brick = at(MAX_SLOPE_DB);
        assert!(
            approaching < continuous,
            "approaching brick should already tighten the first pole: {approaching} vs {continuous}"
        );
        assert!(
            brick < approaching,
            "full brick should be thinner still: {brick} vs {approaching}"
        );
        assert!(brick < continuous * 0.5);
    }

    #[test]
    fn whole_phaser_sections_remain_allpass() {
        let coefficients = FilterConfig {
            mode: FilterMode::Phaser,
            q: MAX_Q,
            slope_db_oct: MAX_SLOPE_DB,
            morph: 63.0 / 127.0,
            ..FilterConfig::default()
        }
        .coefficients(TEST_SAMPLE_RATE);
        let count = usize::from(coefficients.processing_stage_count());
        let blend = coefficients.processing_stage_blend();
        for frequency in [20.0, 200.0, 2_000.0, 20_000.0] {
            let mut wet = ComplexResponse::ONE;
            for index in 0..count {
                let (radius, pole_cos) =
                    phaser_pole(coefficients.shape, coefficients.phase_center_cos_at(index));
                let stage =
                    second_order_allpass_response(pole_cos, radius, frequency, TEST_SAMPLE_RATE);
                wet = if index + 1 == count {
                    wet.multiply(
                        ComplexResponse::ONE.add(stage.subtract(ComplexResponse::ONE).scale(blend)),
                    )
                } else {
                    wet.multiply(stage)
                };
            }
            assert!(
                (wet.magnitude() - 1.0).abs() < 1.0e-2,
                "allpass mag {} at {frequency} Hz",
                wet.magnitude()
            );
        }
    }

    #[test]
    fn phaser_preview_samples_exact_cancellation_frequencies() {
        let config = FilterConfig {
            mode: FilterMode::Phaser,
            q: MAX_Q,
            slope_db_oct: 96.0,
            morph: 63.0 / 127.0,
            ..FilterConfig::default()
        };
        let mut found = 0;
        for notch in 0..MAX_PHASE_POLES / 2 {
            if let Some(frequency) =
                config.phaser_notch_frequency(notch, TEST_SAMPLE_RATE, MIN_CUTOFF_HZ, 20_000.0)
            {
                assert!(
                    config.response_magnitude(frequency, TEST_SAMPLE_RATE) < 1.0e-3,
                    "notch {notch} missed cancellation at {frequency} Hz"
                );
                found += 1;
            }
        }
        assert!(found > 4);
    }

    #[test]
    fn phaser_coefficient_smoothing_preserves_fractional_pole_motion() {
        let start = FilterConfig {
            mode: FilterMode::Phaser,
            slope_db_oct: MAX_SLOPE_DB,
            morph: 0.0,
            ..FilterConfig::default()
        }
        .coefficients(TEST_SAMPLE_RATE);
        let target = FilterConfig {
            mode: FilterMode::Phaser,
            slope_db_oct: MAX_SLOPE_DB,
            morph: 1.0,
            ..FilterConfig::default()
        }
        .coefficients(TEST_SAMPLE_RATE);
        let midpoint = start.interpolate(target, 0.5);
        assert_eq!(
            (
                midpoint.processing_stage_count(),
                midpoint.processing_stage_blend()
            ),
            (33, 0.25)
        );
    }

    #[test]
    fn newly_added_phaser_pole_enters_without_a_dc_transient() {
        let config = FilterConfig {
            mode: FilterMode::Phaser,
            slope_db_oct: MAX_SLOPE_DB,
            morph: 63.0 / 127.0,
            ..FilterConfig::default()
        };
        let mut filter = StereoTptSvf::default();
        for _ in 0..512 {
            let _ = filter.process(config.coefficients(TEST_SAMPLE_RATE), 1.0, 1.0);
        }
        let before = filter
            .process(config.coefficients(TEST_SAMPLE_RATE), 1.0, 1.0)
            .0;
        let after = filter
            .process(
                FilterConfig {
                    morph: 63.5 / 127.0,
                    ..config
                }
                .coefficients(TEST_SAMPLE_RATE),
                1.0,
                1.0,
            )
            .0;
        assert!((after - before).abs() < 1.0e-5);
    }

    #[test]
    fn order_morph_is_continuous_at_every_stage_boundary() {
        for stage in 1..MAX_SVF_STAGES {
            let center = FilterConfig {
                cutoff_hz: 2_000.0,
                slope_db_oct: stage as f32 * 12.0,
                ..FilterConfig::default()
            };
            let mut filter = StereoTptSvf::default();
            for sample in 0..128 {
                let input = (sample as f32 * 0.19).sin() * 0.2;
                let _ = filter.process(center.coefficients(TEST_SAMPLE_RATE), input, input);
            }
            let mut below = filter;
            let mut above = filter;
            let input = 0.137;
            let lower = below
                .process(
                    FilterConfig {
                        slope_db_oct: center.slope_db_oct - 1.0e-3,
                        ..center
                    }
                    .coefficients(TEST_SAMPLE_RATE),
                    input,
                    input,
                )
                .0;
            let upper = above
                .process(
                    FilterConfig {
                        slope_db_oct: center.slope_db_oct + 1.0e-3,
                        ..center
                    }
                    .coefficients(TEST_SAMPLE_RATE),
                    input,
                    input,
                )
                .0;
            assert!((upper - lower).abs() < 1.0e-3, "SVF stage {stage}");
        }
        for pole in 1..MAX_PHASE_POLES {
            let center = FilterConfig {
                mode: FilterMode::Phaser,
                q: MAX_Q,
                morph: (pole as f32 - 1.0) / (MAX_PHASE_POLES as f32 - 1.0),
                ..FilterConfig::default()
            };
            let mut filter = StereoTptSvf::default();
            for sample in 0..128 {
                let input = (sample as f32 * 0.19).sin() * 0.2;
                let _ = filter.process(center.coefficients(TEST_SAMPLE_RATE), input, input);
            }
            let mut below = filter;
            let mut above = filter;
            let delta = 1.0e-6;
            let lower = below
                .process(
                    FilterConfig {
                        morph: center.morph - delta,
                        ..center
                    }
                    .coefficients(TEST_SAMPLE_RATE),
                    0.137,
                    0.137,
                )
                .0;
            let upper = above
                .process(
                    FilterConfig {
                        morph: center.morph + delta,
                        ..center
                    }
                    .coefficients(TEST_SAMPLE_RATE),
                    0.137,
                    0.137,
                )
                .0;
            assert!((upper - lower).abs() < 1.0e-3, "Phaser pole {pole}");
        }
    }

    #[test]
    fn compact_static_state_copy_matches_full_filter_copy() {
        for mode in FilterMode::ALL {
            let coefficients = FilterConfig {
                mode,
                cutoff_hz: 1_700.0,
                q: 1.3,
                slope_db_oct: 24.0,
                morph: 0.2,
                shape: 0.0,
            }
            .coefficients(TEST_SAMPLE_RATE);
            let mut source = StereoTptSvf::default();
            for sample in 0..256 {
                let input = (sample as f32 * 0.13).sin() * 0.2;
                let _ = source.process(coefficients, input, -input);
            }
            let mut full = source;
            let mut compact = StereoTptSvf::default();
            compact.copy_static_state_from(&source, coefficients);
            for sample in 0..64 {
                let input = (sample as f32 * 0.17).cos() * 0.2;
                assert_eq!(
                    compact.process(coefficients, input, -input),
                    full.process(coefficients, input, -input),
                    "{mode:?}"
                );
            }
        }
    }

    #[test]
    fn phaser_spacing_expands_the_bank() {
        let narrow = FilterConfig {
            mode: FilterMode::Phaser,
            cutoff_hz: 1_000.0,
            slope_db_oct: MIN_SLOPE_DB,
            ..FilterConfig::default()
        };
        let wide = FilterConfig {
            slope_db_oct: MAX_SLOPE_DB,
            ..narrow
        };
        let narrow_distance = (narrow.stage_frequency(1, TEST_SAMPLE_RATE) / 1_000.0)
            .log2()
            .abs();
        let wide_distance = (wide.stage_frequency(1, TEST_SAMPLE_RATE) / 1_000.0)
            .log2()
            .abs();
        assert!(wide_distance > narrow_distance);
    }

    #[test]
    fn centered_phaser_skew_is_uniform() {
        for index in 0..8 {
            let unit = (index as f32 + 0.5) / 8.0;
            assert!((cluster_unit(unit, 0.5) - unit).abs() < f32::EPSILON);
        }
    }

    #[test]
    #[ignore = "manual release-mode DSP benchmark"]
    fn benchmark_maximum_order_filters() {
        use std::hint::black_box;
        use std::time::Instant;

        const VOICES: usize = 32;
        const SAMPLES: usize = 48_000;
        for mode in FilterMode::ALL {
            let coefficients = FilterConfig {
                mode,
                cutoff_hz: 1_000.0,
                q: std::f32::consts::FRAC_1_SQRT_2,
                slope_db_oct: MAX_SLOPE_DB,
                morph: 1.0,
                shape: 0.0,
            }
            .coefficients(TEST_SAMPLE_RATE);
            let mut filters = [StereoTptSvf::default(); VOICES];
            let started = Instant::now();
            for sample in 0..SAMPLES {
                let input = (sample as f32 * 0.01).sin();
                for filter in &mut filters {
                    black_box(filter.process(coefficients, input, input));
                }
            }
            let elapsed = started.elapsed();
            eprintln!(
                "{} maximum order: {elapsed:?}, {:.1} ns/note-sample",
                mode.label(),
                elapsed.as_nanos() as f64 / (VOICES * SAMPLES) as f64
            );
        }
    }

    #[test]
    fn response_magnitude_remains_finite_at_plot_boundaries() {
        for mode in FilterMode::ALL {
            let config = FilterConfig {
                mode,
                cutoff_hz: 20_000.0,
                q: MAX_Q,
                slope_db_oct: MAX_SLOPE_DB,
                morph: 1.0,
                shape: 0.0,
            };
            for frequency in [0.0, 20.0, 20_000.0, TEST_SAMPLE_RATE * NYQUIST_GUARD] {
                assert!(
                    config
                        .response_magnitude(frequency, TEST_SAMPLE_RATE)
                        .is_finite()
                );
            }
        }
    }

    #[test]
    fn scream_nonlinearity_is_visible_across_resonance() {
        for resonance in [0.2, 0.4, 0.62, 0.85] {
            let q = MIN_Q * (MAX_Q / MIN_Q).powf(resonance);
            let config = FilterConfig {
                mode: FilterMode::Scream,
                cutoff_hz: 5_000.0,
                q,
                slope_db_oct: (MIN_SLOPE_DB * MAX_SLOPE_DB).sqrt(),
                morph: 1.0,
                ..FilterConfig::default()
            };
            let analytical = config.response_magnitude(1_200.0, TEST_SAMPLE_RATE);
            let measured = measured_response(config, 1_200.0, SCREAM_PREVIEW_INPUT_PEAK);
            let tolerance = if resonance >= 0.7 { 0.35 } else { 0.08 };
            assert!(
                (analytical - measured).abs() < tolerance,
                "reso {resonance}: analytical={analytical} measured={measured}"
            );
            let tiny = measured_response(config, 1_200.0, 1.0e-3);
            assert!(
                (measured - tiny).abs() > 0.04,
                "reso {resonance} should already show saturator compression, loud={measured} tiny={tiny}"
            );
        }
    }

    #[test]
    fn scream_maximum_feedback_stays_finite_and_bounded() {
        let coefficients = FilterConfig {
            mode: FilterMode::Scream,
            cutoff_hz: 18_000.0,
            q: f32::INFINITY,
            slope_db_oct: f32::INFINITY,
            morph: 1.0,
            shape: 0.0,
        }
        .coefficients(TEST_SAMPLE_RATE);
        let mut filter = StereoTptSvf::default();
        let mut peak = 0.0_f32;
        for sample in 0..TEST_SAMPLE_RATE as usize * 2 {
            let input = if sample < 256 {
                (sample as f32 * 0.37).sin()
            } else {
                0.0
            };
            let (left, right) = filter.process(coefficients, input, -input);
            assert!(left.is_finite() && right.is_finite());
            peak = peak.max(left.abs()).max(right.abs());
        }
        assert!(peak < 4.0, "unsafe Scream peak: {peak}");
    }

    fn measured_response(config: FilterConfig, frequency: f32, amplitude: f32) -> f32 {
        let coefficients = config.coefficients(TEST_SAMPLE_RATE);
        let mut filter = StereoTptSvf::default();
        let increment = std::f32::consts::TAU * frequency / TEST_SAMPLE_RATE;
        let settle_samples = TEST_SAMPLE_RATE as usize / 4;
        for index in 0..settle_samples {
            let input = amplitude * (increment * index as f32).cos();
            let _ = filter.process(coefficients, input, input);
        }

        let mut in_phase = 0.0;
        let mut quadrature = 0.0;
        for index in 0..ANALYSIS_SAMPLES {
            let phase = increment * (settle_samples + index) as f32;
            let input = amplitude * phase.cos();
            let (output, _) = filter.process(coefficients, input, input);
            in_phase = output.mul_add(phase.cos(), in_phase);
            quadrature = output.mul_add(phase.sin(), quadrature);
        }
        2.0 * in_phase.hypot(quadrature) / (ANALYSIS_SAMPLES as f32 * amplitude)
    }
}
