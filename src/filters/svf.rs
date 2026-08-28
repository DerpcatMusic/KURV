use std::sync::OnceLock;

use crate::voices::fast_exp2;
use truce_simd::simd::f32x4;

const DEFAULT_SAMPLE_RATE: f32 = 44_100.0;
const MIN_CUTOFF_HZ: f32 = 5.0;
const MAX_STORED_CUTOFF_HZ: f32 = 100_000.0;
const NYQUIST_GUARD: f32 = 0.495;
pub(crate) const MIN_Q: f32 = 0.1;
pub(crate) const MAX_Q: f32 = 32.0;
const Q_OCTAVES: f32 = 8.321_928;
pub(crate) const MIN_SLOPE_DB: f32 = 6.0;
const MIN_SVF_SLOPE_DB: f32 = 12.0;
pub(crate) const MAX_SLOPE_DB: f32 = 768.0;
const MAX_SVF_STAGES: usize = 64;
const MAX_PHASE_POLES: usize = 128;
const CENTERED_PHASE_EXPONENTS: [f32; MAX_PHASE_POLES] = centered_phase_exponents();
const COEFFICIENT_TABLE_SIZE: usize = 2_048;
const PHASE_SPAN_TABLE_SIZE: usize = 256;
static COEFFICIENT_TABLE: OnceLock<Box<[f32]>> = OnceLock::new();
static PHASE_RATIO_TABLE: OnceLock<Box<[f32]>> = OnceLock::new();
static PHASE_SPAN_TABLE: OnceLock<Box<[f32]>> = OnceLock::new();
static SCREAM_HP_RATIO_TABLE: OnceLock<Box<[f32]>> = OnceLock::new();
static SCREAM_FEEDBACK_TABLE: OnceLock<Box<[f32]>> = OnceLock::new();
static BUTTERWORTH_DAMPING: OnceLock<Box<[f32]>> = OnceLock::new();

pub(crate) fn prepare() {
    let _ = coefficient_table();
    let _ = phase_ratio_table();
    let _ = phase_span_table();
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
}

impl FilterMode {
    pub const ALL: [Self; 3] = [Self::Svf, Self::Phaser, Self::Scream];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Svf => "SVF MORPH",
            Self::Phaser => "PHASER",
            Self::Scream => "SCREAM",
        }
    }

    #[must_use]
    pub const fn short_label(self) -> &'static str {
        match self {
            Self::Svf => "SVF",
            Self::Phaser => "PHASE",
            Self::Scream => "SCREAM",
        }
    }

    #[must_use]
    pub const fn resonance_label(self) -> &'static str {
        match self {
            Self::Svf | Self::Phaser => "Q",
            Self::Scream => "RESO",
        }
    }

    #[must_use]
    pub const fn slope_label(self) -> &'static str {
        match self {
            Self::Svf => "DB/OCT",
            Self::Phaser => "SPACING",
            Self::Scream => "SCREAM",
        }
    }

    #[must_use]
    pub const fn morph_label(self) -> &'static str {
        match self {
            Self::Svf => "MORPH",
            Self::Phaser => "POLES",
            Self::Scream => "MIX",
        }
    }

    #[must_use]
    pub const fn resonance_help(self) -> &'static str {
        match self {
            Self::Svf => "Q",
            Self::Phaser => "Notch depth from dry to full cancellation",
            Self::Scream => "Feedback drive and high-pass resonance",
        }
    }

    #[must_use]
    pub const fn slope_help(self) -> &'static str {
        match self {
            Self::Svf => "Order morph from 12 dB/oct to a 128-pole brickwall",
            Self::Phaser => "Logarithmic spacing between Phaser stages",
            Self::Scream => "Feedback high-pass position relative to cutoff",
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
}

impl Default for FilterConfig {
    fn default() -> Self {
        Self {
            mode: FilterMode::Svf,
            cutoff_hz: 20_000.0,
            q: std::f32::consts::FRAC_1_SQRT_2,
            slope_db_oct: 12.0,
            morph: 0.0,
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
            },
            FilterMode::Scream => Self {
                mode,
                cutoff_hz: 5_000.0,
                q: 8.0,
                slope_db_oct: (MIN_SLOPE_DB * MAX_SLOPE_DB).sqrt(),
                morph: 1.0,
            },
        }
    }

    #[must_use]
    pub(crate) fn sanitized(self) -> Self {
        Self {
            mode: self.mode,
            cutoff_hz: finite_or(self.cutoff_hz, 20_000.0)
                .clamp(MIN_CUTOFF_HZ, MAX_STORED_CUTOFF_HZ),
            q: finite_or(self.q, std::f32::consts::FRAC_1_SQRT_2).clamp(MIN_Q, MAX_Q),
            slope_db_oct: finite_or(self.slope_db_oct, self.minimum_slope())
                .clamp(self.minimum_slope(), MAX_SLOPE_DB),
            morph: finite_or(self.morph, 0.0).clamp(0.0, 1.0),
        }
    }

    fn sanitized_for_sample_rate(self, sample_rate: f32) -> Self {
        let maximum_cutoff = sample_rate * NYQUIST_GUARD;
        let mut config = self.sanitized();
        config.cutoff_hz = config
            .cutoff_hz
            .clamp(MIN_CUTOFF_HZ.min(maximum_cutoff), maximum_cutoff);
        config
    }

    #[must_use]
    pub(crate) fn modulated(
        self,
        cutoff_octaves: f32,
        resonance_octaves: f32,
        slope: f32,
        morph: f32,
    ) -> Self {
        Self {
            cutoff_hz: self.cutoff_hz * fast_exp2(finite_or(cutoff_octaves, 0.0).clamp(-4.0, 4.0)),
            q: self.q * fast_exp2(finite_or(resonance_octaves, 0.0).clamp(-4.0, 4.0)),
            slope_db_oct: finite_or(slope, 0.0).mul_add(12.0, self.slope_db_oct),
            morph: self.morph + finite_or(morph, 0.0),
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
            FilterMode::Phaser | FilterMode::Scream => MIN_SLOPE_DB,
        }
    }

    #[must_use]
    pub(crate) fn coefficients(self, sample_rate: f32) -> FilterCoefficients {
        let sample_rate = sanitize_sample_rate(sample_rate);
        let config = self.sanitized_for_sample_rate(sample_rate);
        let stages = match config.mode {
            FilterMode::Svf => (config.slope_db_oct / 12.0).clamp(1.0, MAX_SVF_STAGES as f32),
            FilterMode::Phaser => config.morph.mul_add(MAX_PHASE_POLES as f32 - 1.0, 1.0),
            FilterMode::Scream => 2.0,
        };
        let (processing_stages, processing_blend) = processing_stage_shape(config.mode, stages);
        let table_scale = COEFFICIENT_TABLE_SIZE as f32 / (sample_rate * NYQUIST_GUARD);
        let scream_resonance = normalized_log(config.q, MIN_Q, MAX_Q);
        let damping = match config.mode {
            FilterMode::Svf => (std::f32::consts::FRAC_1_SQRT_2 / config.q).min(1.0),
            FilterMode::Phaser => normalized_log(config.q, MIN_Q, MAX_Q),
            FilterMode::Scream => scream_resonance,
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
        FilterCoefficients {
            mode: config.mode,
            q: config.q,
            slope_db_oct: config.slope_db_oct,
            g,
            damping,
            morph: config.morph,
            stages,
            processing_stages,
            processing_blend,
            span_octaves: match config.mode {
                FilterMode::Svf => stage_span_octaves(processing_stages),
                FilterMode::Phaser => phase_span_octaves(config.slope_db_oct),
                FilterMode::Scream => 0.0,
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
        }
    }

    #[must_use]
    pub(crate) fn stage_count(self) -> u8 {
        match self.mode {
            FilterMode::Svf => slope_to_svf_stages(self.slope_db_oct).0,
            FilterMode::Phaser => MAX_PHASE_POLES as u8,
            FilterMode::Scream => 2,
        }
    }

    #[must_use]
    pub(crate) fn response_stage_count(self) -> u8 {
        match self.mode {
            FilterMode::Svf => self.stage_count(),
            FilterMode::Phaser => self.effective_poles().ceil() as u8,
            FilterMode::Scream => 2,
        }
    }

    #[must_use]
    pub(crate) fn effective_poles(self) -> f32 {
        match self.mode {
            FilterMode::Svf => self.slope_db_oct / 6.0,
            FilterMode::Phaser => self
                .morph
                .clamp(0.0, 1.0)
                .mul_add(MAX_PHASE_POLES as f32 - 1.0, 1.0),
            FilterMode::Scream => 2.0,
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
    pub(crate) fn phaser_notch_frequency(
        self,
        notch: usize,
        sample_rate: f32,
        minimum: f32,
        maximum: f32,
    ) -> Option<f32> {
        if self.mode != FilterMode::Phaser || notch >= MAX_PHASE_POLES / 2 {
            return None;
        }
        let sample_rate = sanitize_sample_rate(sample_rate);
        let coefficients = self.coefficients(sample_rate);
        let mut low = minimum.clamp(0.0, sample_rate * NYQUIST_GUARD);
        let mut high = maximum.clamp(low, sample_rate * NYQUIST_GUARD);
        let target = -((2 * notch + 1) as f32) * std::f32::consts::PI;
        if coefficients.phaser_phase(low, sample_rate) <= target
            || coefficients.phaser_phase(high, sample_rate) > target
        {
            return None;
        }
        for _ in 0..24 {
            let middle = (low + high) * 0.5;
            if coefficients.phaser_phase(middle, sample_rate) > target {
                low = middle;
            } else {
                high = middle;
            }
        }
        Some((low + high) * 0.5)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct StageCoefficients {
    damping: f32,
    a1: f32,
    a2: f32,
    a3: f32,
}

impl StageCoefficients {
    fn from_g(g: f32, damping: f32) -> Self {
        let a1 = (1.0 + g * (g + damping)).recip();
        let a2 = g * a1;
        Self {
            damping,
            a1,
            a2,
            a3: g * a2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FilterCoefficients {
    mode: FilterMode,
    q: f32,
    slope_db_oct: f32,
    g: f32,
    damping: f32,
    morph: f32,
    stages: f32,
    processing_stages: u8,
    processing_blend: f32,
    span_octaves: f32,
    skew: f32,
    table_scale: f32,
    cutoff_hz: f32,
    scream_hp_g: f32,
    scream_hp_ratio: f32,
    scream_hp_damping: f32,
    scream_feedback: f32,
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
    pub(crate) fn modulated_cutoff(mut self, cutoff_octaves: f32) -> Self {
        self.cutoff_hz = (self.cutoff_hz
            * fast_exp2(finite_or(cutoff_octaves, 0.0).clamp(-4.0, 4.0)))
        .clamp(
            MIN_CUTOFF_HZ,
            COEFFICIENT_TABLE_SIZE as f32 / self.table_scale,
        );
        if self.mode != FilterMode::Phaser {
            self.g = coefficient(
                self.cutoff_hz * self.table_scale,
                coefficient_table(),
            );
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
                self.damping = (std::f32::consts::FRAC_1_SQRT_2 / self.q).min(1.0);
            }
            FilterMode::Phaser => {
                self.damping = (self.damping + resonance_octaves / Q_OCTAVES).clamp(0.0, 1.0);
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
        }
        self
    }

    #[must_use]
    pub(crate) fn modulated_slope(mut self, slope: f32) -> Self {
        self.slope_db_oct = (self.slope_db_oct + finite_or(slope, 0.0) * 12.0)
            .clamp(MIN_SLOPE_DB, MAX_SLOPE_DB);
        match self.mode {
            FilterMode::Svf => {
                self.stages = (self.slope_db_oct / 12.0).clamp(1.0, MAX_SVF_STAGES as f32);
                (self.processing_stages, self.processing_blend) =
                    processing_stage_shape(self.mode, self.stages);
                self.span_octaves = stage_span_octaves(self.processing_stages);
            }
            FilterMode::Phaser => {
                self.span_octaves = phase_span_octaves(self.slope_db_oct);
            }
            FilterMode::Scream => {
                self.scream_hp_ratio = scream_hp_ratio(self.slope_db_oct);
                self.scream_hp_g = coefficient(
                    (self.cutoff_hz * self.scream_hp_ratio).max(MIN_CUTOFF_HZ)
                        * self.table_scale,
                    coefficient_table(),
                );
            }
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
        }
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

    fn stage_at(self, index: usize, damping_table: &[f32]) -> StageCoefficients {
        debug_assert!(matches!(self.mode, FilterMode::Svf));
        StageCoefficients::from_g(self.g, self.stage_damping(index, damping_table))
    }

    fn stage_damping(self, index: usize, damping_table: &[f32]) -> f32 {
        let stages = self.stages.clamp(1.0, MAX_SVF_STAGES as f32);
        let lower = stages.floor().max(1.0) as usize;
        let upper = stages.ceil().max(1.0) as usize;
        let blend = stages.fract();
        let at = |count: usize, stage: usize| damping_table[(count - 1) * MAX_SVF_STAGES + stage];
        let mut damping = if lower == upper {
            at(upper, index)
        } else if index < lower {
            lerp(at(lower, index), at(upper, index), blend)
        } else {
            at(upper, index)
        };
        if lower == upper {
            if index + 1 == upper {
                damping *= self.damping;
            }
        } else if index + 1 == lower {
            damping *= lerp(self.damping, 1.0, blend);
        } else if index == lower {
            damping *= lerp(1.0, self.damping, blend);
        }
        damping.max(1.0e-4)
    }

    fn phase_coefficient_at(self, index: usize) -> f32 {
        let frequency = stage_frequency(
            self.mode,
            index,
            MAX_PHASE_POLES as u8,
            self.cutoff_hz,
            self.span_octaves,
            self.skew,
        );
        phase_coefficient(
            frequency.max(MIN_CUTOFF_HZ) * self.table_scale,
        )
    }

    fn phaser_phase(self, frequency: f32, sample_rate: f32) -> f32 {
        let count = usize::from(self.processing_stage_count().max(1));
        let blend = self.processing_stage_blend();
        let mut phase = 0.0;
        for index in 0..count {
            let target = self.phase_coefficient_at(index);
            let coefficient = if index + 1 == count {
                lerp(1.0, target, blend)
            } else {
                target
            };
            let response = first_order_allpass_response(coefficient, frequency, sample_rate);
            phase += response.imaginary.atan2(response.real);
        }
        phase
    }

    fn processing_stage_count(self) -> u8 {
        self.processing_stages
    }

    fn processing_stage_blend(self) -> f32 {
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
            && self.damping.to_bits() == other.damping.to_bits()
            && self.stages.to_bits() == other.stages.to_bits()
    }

    fn same_svf_layout(self, other: Self) -> bool {
        self.mode == other.mode
            && self.damping.to_bits() == other.damping.to_bits()
            && self.stages.to_bits() == other.stages.to_bits()
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct ComplexResponse {
    real: f32,
    imaginary: f32,
}

impl ComplexResponse {
    const ONE: Self = Self {
        real: 1.0,
        imaginary: 0.0,
    };

    fn magnitude(self) -> f32 {
        self.real.hypot(self.imaginary)
    }

    fn scale(self, amount: f32) -> Self {
        Self {
            real: self.real * amount,
            imaginary: self.imaginary * amount,
        }
    }

    fn add(self, other: Self) -> Self {
        Self {
            real: self.real + other.real,
            imaginary: self.imaginary + other.imaginary,
        }
    }

    fn subtract(self, other: Self) -> Self {
        Self {
            real: self.real - other.real,
            imaginary: self.imaginary - other.imaginary,
        }
    }

    fn multiply(self, other: Self) -> Self {
        Self {
            real: self
                .real
                .mul_add(other.real, -self.imaginary * other.imaginary),
            imaginary: self
                .real
                .mul_add(other.imaginary, self.imaginary * other.real),
        }
    }

    fn divide(self, other: Self) -> Self {
        let denominator = other
            .real
            .mul_add(other.real, other.imaginary * other.imaginary);
        Self {
            real: self
                .real
                .mul_add(other.real, self.imaginary * other.imaginary)
                / denominator,
            imaginary: self
                .imaginary
                .mul_add(other.real, -self.real * other.imaginary)
                / denominator,
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
            let blend = coefficients.morph.mul_add(2.0, -1.0);
            let low_amount = (-blend).max(0.0);
            let high_amount = blend.max(0.0);
            let band_amount = 1.0 - blend.abs();
            let count = usize::from(coefficients.processing_stage_count().max(1));
            let damping_table = butterworth_damping_table();
            let mut response = ComplexResponse::ONE;
            for index in 0..count {
                let stage_coeffs = coefficients.stage_at(index, damping_table);
                let (low, band, high) = svf_stage_response(stage_coeffs, frequency, sample_rate);
                let stage = low
                    .scale(low_amount)
                    .add(band.scale(band_amount * stage_coeffs.damping))
                    .add(high.scale(high_amount));
                let full = response.multiply(stage);
                response = if index + 1 == count {
                    response.add(
                        full.subtract(response)
                            .scale(coefficients.processing_stage_blend()),
                    )
                } else {
                    full
                };
            }
            response
        }
        FilterMode::Phaser => {
            let count = coefficients.processing_stage_count().max(1) as usize;
            let blend = coefficients.processing_stage_blend();
            let mut wet = ComplexResponse::ONE;
            for index in 0..count {
                let target = coefficients.phase_coefficient_at(index);
                let coefficient = if index + 1 == count {
                    lerp(1.0, target, blend)
                } else {
                    target
                };
                wet = wet.multiply(first_order_allpass_response(
                    coefficient,
                    frequency,
                    sample_rate,
                ));
            }
            phase_mix_response(wet, coefficients.damping)
        }
        FilterMode::Scream => {
            let low = svf_stage_response(
                StageCoefficients::from_g(coefficients.g, std::f32::consts::SQRT_2),
                frequency,
                sample_rate,
            )
            .0;
            let high = svf_stage_response(
                StageCoefficients::from_g(coefficients.scream_hp_g, coefficients.scream_hp_damping),
                frequency,
                sample_rate,
            )
            .2;
            let angle = std::f32::consts::TAU * frequency / sample_rate;
            let delay = ComplexResponse {
                real: angle.cos(),
                imaginary: -angle.sin(),
            };
            let loop_response = low
                .multiply(high)
                .multiply(delay)
                .scale(coefficients.scream_feedback);
            let wet = low.divide(ComplexResponse::ONE.subtract(loop_response));
            ComplexResponse::ONE.add(wet.subtract(ComplexResponse::ONE).scale(coefficients.morph))
        }
    }
}

fn phase_mix_response(wet: ComplexResponse, depth: f32) -> ComplexResponse {
    let effected = ComplexResponse::ONE.add(wet).scale(0.5);
    ComplexResponse::ONE.add(effected.subtract(ComplexResponse::ONE).scale(depth))
}

fn first_order_allpass_response(
    coefficient: f32,
    frequency: f32,
    sample_rate: f32,
) -> ComplexResponse {
    let angle = std::f32::consts::TAU * frequency / sample_rate;
    let delay = ComplexResponse {
        real: angle.cos(),
        imaginary: -angle.sin(),
    };
    ComplexResponse {
        real: coefficient,
        imaginary: 0.0,
    }
    .add(delay)
    .divide(ComplexResponse::ONE.add(delay.scale(coefficient)))
}

fn svf_stage_response(
    coefficients: StageCoefficients,
    frequency: f32,
    sample_rate: f32,
) -> (ComplexResponse, ComplexResponse, ComplexResponse) {
    let g = coefficients.a2 / coefficients.a1;
    let warped_frequency = (std::f32::consts::PI * frequency / sample_rate).tan();
    let denominator = ComplexResponse {
        real: g.mul_add(g, -warped_frequency * warped_frequency),
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

type StereoState = [f32x4; 2];

#[derive(Clone, Copy, Debug)]
pub struct StereoTptSvf {
    states: [StereoState; MAX_SVF_STAGES],
    cached_coefficients: [f32; MAX_PHASE_POLES],
    cached_stage_values: [f32; MAX_PHASE_POLES],
    cached_damping: [f32; MAX_SVF_STAGES],
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
            cached_coefficients: [0.0; MAX_PHASE_POLES],
            cached_stage_values: [0.0; MAX_PHASE_POLES],
            cached_damping: [0.0; MAX_SVF_STAGES],
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
            FilterMode::Phaser => active.div_ceil(2),
            FilterMode::Scream => 2,
        };
        self.states[..state_count].copy_from_slice(&source.states[..state_count]);
        match coefficients.mode {
            FilterMode::Svf => {
                self.cached_coefficients[..active]
                    .copy_from_slice(&source.cached_coefficients[..active]);
                self.cached_coefficients[MAX_SVF_STAGES..MAX_SVF_STAGES + active].copy_from_slice(
                    &source.cached_coefficients[MAX_SVF_STAGES..MAX_SVF_STAGES + active],
                );
                self.cached_damping[..active].copy_from_slice(&source.cached_damping[..active]);
            }
            FilterMode::Phaser => self.cached_coefficients[..active]
                .copy_from_slice(&source.cached_coefficients[..active]),
            FilterMode::Scream => {
                self.scream_feedback = source.scream_feedback;
                self.scream_peak = source.scream_peak;
            }
        }
        self.coefficient_cache = source.coefficient_cache;
        self.cached_stages = source.cached_stages;
        self.cached_stage_values = source.cached_stage_values;
        self.cached_phase_ratio_stages = source.cached_phase_ratio_stages;
        self.last_mode = source.last_mode;
        self.last_active = source.last_active;
    }

    fn prepare_phase_coefficients(&mut self, coefficients: FilterCoefficients) {
        let active = coefficients.processing_stage_count();
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
            let span_position = (coefficients.span_octaves - 0.25)
                * (PHASE_SPAN_TABLE_SIZE as f32 / 7.75);
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
            let ratio = phase_frequency_ratio(
                index,
                coefficients.span_octaves,
                coefficients.skew,
            );
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
        let (ratio_chunks, _) = self.cached_stage_values[index..end].as_chunks::<4>();
        let (coefficient_chunks, _) =
            self.cached_coefficients[index..end].as_chunks_mut::<4>();
        for (ratios, output) in ratio_chunks.iter().zip(coefficient_chunks) {
            *output = phase_coefficients4(f32x4::from(*ratios), scale, minimum).to_array();
        }
        index += ratio_chunks.len() * 4;
        for index in index..end {
            let frequency = coefficients.cutoff_hz * self.cached_stage_values[index];
            self.cached_coefficients[index] = phase_coefficient(
                frequency.max(MIN_CUTOFF_HZ) * coefficients.table_scale,
            );
        }
        self.cached_stages = if same_topology {
            self.cached_stages.max(active)
        } else {
            active
        };
        self.coefficient_cache = Some(coefficients);
    }

    fn prepare_svf_coefficients(&mut self, coefficients: FilterCoefficients) {
        if self
            .coefficient_cache
            .is_some_and(|cached| cached.same_svf_topology(coefficients))
        {
            return;
        }
        let count = usize::from(coefficients.processing_stage_count().max(1));
        let same_layout = self
            .coefficient_cache
            .is_some_and(|cached| cached.same_svf_layout(coefficients));
        let damping_start = if same_layout {
            usize::from(self.cached_stages).min(count)
        } else {
            0
        };
        let damping_table = butterworth_damping_table();
        let stages = coefficients.stages.clamp(1.0, MAX_SVF_STAGES as f32);
        let lower = stages.floor().max(1.0) as usize;
        let upper = stages.ceil().max(1.0) as usize;
        let blend = stages.fract();
        let at = |stage_count: usize, stage: usize| {
            damping_table[(stage_count - 1) * MAX_SVF_STAGES + stage]
        };
        for index in damping_start..count {
            let mut damping = if lower == upper {
                at(upper, index)
            } else if index < lower {
                lerp(at(lower, index), at(upper, index), blend)
            } else {
                at(upper, index)
            };
            if lower == upper && index + 1 == upper {
                damping *= coefficients.damping;
            } else if lower != upper && index + 1 == lower {
                damping *= lerp(coefficients.damping, 1.0, blend);
            } else if lower != upper && index == lower {
                damping *= lerp(1.0, coefficients.damping, blend);
            }
            self.cached_damping[index] = damping.max(1.0e-4);
        }
        for index in 0..count {
            let stage = StageCoefficients::from_g(coefficients.g, self.cached_damping[index]);
            self.cached_coefficients[index] = stage.a1;
            self.cached_coefficients[MAX_SVF_STAGES + index] = stage.a2;
            self.cached_stage_values[index] = stage.a3;
        }
        self.cached_stages = if same_layout {
            self.cached_stages.max(count as u8)
        } else {
            count as u8
        };
        self.coefficient_cache = Some(coefficients);
    }

    #[inline]
    pub(crate) fn prepare_phaser(&mut self, coefficients: FilterCoefficients) {
        debug_assert!(coefficients.is_phaser());
        if coefficients.mode != self.last_mode {
            self.reset();
            self.last_mode = coefficients.mode;
        }
        self.prepare_phase_coefficients(coefficients);
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
                    &self.cached_coefficients,
                    &self.cached_stage_values,
                    &self.cached_damping,
                    input,
                    coefficients,
                )
            }
            FilterMode::Phaser => {
                self.prepare_phase_coefficients(coefficients);
                process_phase_bank(
                    &mut self.states,
                    &self.cached_coefficients,
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
        let stages = modulated.stages.clamp(1.0, MAX_SVF_STAGES as f32);
        let lower = stages.floor().max(1.0) as usize;
        let upper = stages.ceil().max(1.0) as usize;
        let damping_table = butterworth_damping_table();
        let first_index = lower - 1;
        let first = modulated.stage_at(first_index, damping_table);
        let second = (lower != upper).then(|| (lower, modulated.stage_at(lower, damping_table)));
        let input = f32x4::from([finite_or(left, 0.0), finite_or(right, 0.0), 0.0, 0.0]);
        let cached_coefficients = &self.cached_coefficients;
        let cached_damping = &self.cached_damping;
        let cached_stage_values = &self.cached_stage_values;
        let output = process_svf_stages(
            &mut self.states,
            input,
            modulated,
            |index| {
                if index == first_index {
                    first
                } else if let Some((second_index, second_stage)) = second
                    && index == second_index
                {
                    second_stage
                } else {
                    let a2 = cached_coefficients[MAX_SVF_STAGES + index];
                    StageCoefficients {
                        damping: cached_damping[index],
                        a1: cached_coefficients[index],
                        a2,
                        a3: cached_stage_values[index],
                    }
                }
            },
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
        let depth = (coefficients.damping
            + finite_or(resonance_octaves, 0.0).clamp(-4.0, 4.0) / Q_OCTAVES)
            .clamp(0.0, 1.0);
        self.process_prepared_phaser_parts(
            coefficients.processing_stage_count(),
            coefficients.processing_stage_blend(),
            depth,
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
        left: f32,
        right: f32,
    ) -> (f32, f32) {
        let previous_active = self.last_active;
        self.last_active = active;
        let input = f32x4::from([finite_or(left, 0.0), finite_or(right, 0.0), 0.0, 0.0]);
        let output = process_phase_bank(
            &mut self.states,
            &self.cached_coefficients,
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

// Topology follows Cure Audio's MIT-licensed Scream:
// https://github.com/Cure-Audio/Scream
#[inline]
fn process_scream(
    states: &mut [StereoState; MAX_SVF_STAGES],
    feedback: &mut f32x4,
    peak: &mut f32x4,
    input: f32x4,
    g: f32,
    hp_g: f32,
    hp_damping: f32,
    feedback_gain: f32,
    morph: f32,
) -> f32x4 {
    let driven = soft_saturate(input + *feedback);
    let low = tick_svf(
        &mut states[0],
        driven,
        StageCoefficients::from_g(g, std::f32::consts::SQRT_2),
    )
    .0;
    let high = tick_svf(
        &mut states[1],
        low * f32x4::splat(feedback_gain),
        StageCoefficients::from_g(hp_g, hp_damping),
    )
    .2;
    *peak = input.abs().max(*peak * f32x4::splat(0.999));
    let gate = (*peak * f32x4::splat(1.0e6))
        .max(f32x4::ZERO)
        .min(f32x4::ONE);
    *feedback = soft_saturate(high) * gate;
    input + (low - input) * f32x4::splat(morph)
}

#[inline]
fn soft_saturate(value: f32x4) -> f32x4 {
    value * value.mul_add(value, f32x4::ONE).recip_sqrt()
}

#[inline]
fn process_svf(
    states: &mut [StereoState; MAX_SVF_STAGES],
    cached_coefficients: &[f32; MAX_PHASE_POLES],
    cached_stage_values: &[f32; MAX_PHASE_POLES],
    cached_damping: &[f32; MAX_SVF_STAGES],
    input: f32x4,
    coefficients: FilterCoefficients,
) -> f32x4 {
    let stage_at = |index: usize| {
        let a2 = cached_coefficients[MAX_SVF_STAGES + index];
        StageCoefficients {
            damping: cached_damping[index],
            a1: cached_coefficients[index],
            a2,
            a3: cached_stage_values[index],
        }
    };
    process_svf_stages(states, input, coefficients, stage_at)
}

#[inline]
fn process_svf_stages(
    states: &mut [StereoState; MAX_SVF_STAGES],
    input: f32x4,
    coefficients: FilterCoefficients,
    mut stage_at: impl FnMut(usize) -> StageCoefficients,
) -> f32x4 {
    let blend = coefficients.morph * 2.0 - 1.0;
    let count = coefficients.processing_stage_count().max(1) as usize;
    if blend <= -1.0 {
        return cascade_svf(
            states,
            input,
            count,
            coefficients,
            |state, signal, index| tick_svf(state, signal, stage_at(index)).0,
        );
    }
    if blend >= 1.0 {
        return cascade_svf(
            states,
            input,
            count,
            coefficients,
            |state, signal, index| tick_svf(state, signal, stage_at(index)).2,
        );
    }
    if blend.abs() <= f32::EPSILON {
        return cascade_svf(
            states,
            input,
            count,
            coefficients,
            |state, signal, index| {
                let stage = stage_at(index);
                tick_svf(state, signal, stage).1 * f32x4::splat(stage.damping)
            },
        );
    }
    let low_amount = f32x4::splat((-blend).max(0.0));
    let high_amount = f32x4::splat(blend.max(0.0));
    let band_amount = f32x4::splat(1.0 - blend.abs());
    cascade_svf(
        states,
        input,
        count,
        coefficients,
        |state, signal, index| {
            let stage = stage_at(index);
            let (low, band, high) = tick_svf(state, signal, stage);
            low_amount.mul_add(
                low,
                band_amount.mul_add(band * f32x4::splat(stage.damping), high_amount * high),
            )
        },
    )
}

#[inline]
fn cascade_svf(
    states: &mut [StereoState; MAX_SVF_STAGES],
    input: f32x4,
    count: usize,
    coefficients: FilterCoefficients,
    mut process_stage: impl FnMut(&mut StereoState, f32x4, usize) -> f32x4,
) -> f32x4 {
    let first = process_stage(&mut states[0], input, 0);
    if count == 1 {
        return input + (first - input) * f32x4::splat(coefficients.processing_stage_blend());
    }
    let mut signal = first;
    for index in 1..count - 1 {
        signal = process_stage(&mut states[index], signal, index);
    }
    let last = process_stage(&mut states[count - 1], signal, count - 1);
    signal + (last - signal) * f32x4::splat(coefficients.processing_stage_blend())
}

#[inline]
fn process_phase_bank(
    states: &mut [StereoState; MAX_SVF_STAGES],
    stage_coefficients: &[f32; MAX_PHASE_POLES],
    input: f32x4,
    active: u8,
    blend: f32,
    depth: f32,
    previous_active: u8,
) -> f32x4 {
    let states = states.as_flattened_mut();
    let count = active.max(1) as usize;
    let mut wet = input;
    let established = usize::from(previous_active).min(count);
    for index in 0..established {
        let target = stage_coefficients[index];
        let coefficient = if index + 1 == count {
            lerp(1.0, target, blend)
        } else {
            target
        };
        wet = tick_first_order_allpass(&mut states[index], wet, coefficient);
    }
    for index in established..count {
        let target = stage_coefficients[index];
        let coefficient = if index + 1 == count {
            lerp(1.0, target, blend)
        } else {
            target
        };
        let state = &mut states[index];
        *state = wet * f32x4::splat(1.0 - coefficient);
        wet = tick_first_order_allpass(state, wet, coefficient);
    }
    let effected = (input + wet) * f32x4::splat(0.5);
    input + (effected - input) * f32x4::splat(depth)
}

#[inline]
fn tick_first_order_allpass(state: &mut f32x4, input: f32x4, coefficient: f32) -> f32x4 {
    let coefficient = f32x4::splat(coefficient);
    let output = coefficient.mul_add(input, *state);
    *state = input - coefficient * output;
    output
}

#[inline]
fn tick_svf(
    state: &mut StereoState,
    input: f32x4,
    coefficients: StageCoefficients,
) -> (f32x4, f32x4, f32x4) {
    let v3 = input - state[1];
    let band =
        f32x4::splat(coefficients.a1) * state[0] + f32x4::splat(coefficients.a2) * v3;
    let low = f32x4::splat(coefficients.a2) * state[0]
        + f32x4::splat(coefficients.a3) * v3
        + state[1];
    let high = input - low - f32x4::splat(coefficients.damping) * band;
    state[0] = band * f32x4::splat(2.0) - state[0];
    state[1] = low * f32x4::splat(2.0) - state[1];
    (low, band, high)
}

#[inline]
fn lerp(from: f32, to: f32, amount: f32) -> f32 {
    amount.mul_add(to - from, from)
}

fn slope_to_svf_stages(slope_db_oct: f32) -> (u8, f32) {
    let stages = (finite_or(slope_db_oct, MIN_SVF_SLOPE_DB) / 12.0)
        .clamp(1.0, MAX_SVF_STAGES as f32);
    svf_stage_shape(stages)
}

fn svf_stage_shape(stages: f32) -> (u8, f32) {
    let whole = stages.floor();
    let fraction = stages - whole;
    if fraction <= 1.0e-4 {
        (whole as u8, 1.0)
    } else {
        ((whole as u8 + 1).min(MAX_SVF_STAGES as u8), fraction)
    }
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
        FilterMode::Phaser => progressive_stage_count(stages, MAX_PHASE_POLES),
        FilterMode::Scream => (2, 1.0),
    }
}

fn normalized_log(value: f32, minimum: f32, maximum: f32) -> f32 {
    (value.clamp(minimum, maximum) / minimum).ln() / (maximum / minimum).ln()
}

fn stage_span_octaves(active_stages: u8) -> f32 {
    (0.55 + 0.075 * f32::from(active_stages)).min(5.5)
}

fn cluster_unit(unit: f32, skew: f32) -> f32 {
    let unit = unit.clamp(0.0, 1.0);
    let skew = skew.clamp(0.0, 1.0);
    if skew == 0.5 {
        return unit;
    }
    let factor = fast_exp2((0.5 - skew) * 8.0);
    unit / unit.mul_add(1.0 - factor, factor)
}

const fn nested_phase_unit(index: usize) -> f32 {
    let mut value = index + 1;
    let mut fraction = 0.5;
    let mut result = 0.0;
    while value != 0 {
        if value & 1 != 0 {
            result += fraction;
        }
        value >>= 1;
        fraction *= 0.5;
    }
    result
}

const fn centered_phase_exponents() -> [f32; MAX_PHASE_POLES] {
    let mut output = [0.0; MAX_PHASE_POLES];
    let mut index = 1;
    while index < MAX_PHASE_POLES {
        output[index] = (nested_phase_unit(index) - 0.5) * 2.0;
        index += 1;
    }
    output
}

fn phase_frequency_ratio(index: usize, span_octaves: f32, skew: f32) -> f32 {
    if index == 0 {
        return 1.0;
    }
    let warped = cluster_unit(nested_phase_unit(index), skew);
    fast_exp2((warped - 0.5) * 2.0 * span_octaves)
}

fn stage_frequency(
    mode: FilterMode,
    index: usize,
    active_stages: u8,
    cutoff_hz: f32,
    span_octaves: f32,
    skew: f32,
) -> f32 {
    let count = active_stages.max(1) as usize;
    if !matches!(mode, FilterMode::Phaser) || count == 1 || index == 0 {
        return cutoff_hz;
    }
    cutoff_hz * phase_frequency_ratio(index, span_octaves, skew)
}

fn coefficient_table() -> &'static [f32] {
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

fn phase_ratio_table() -> &'static [f32] {
    PHASE_RATIO_TABLE.get_or_init(|| {
        (0..=PHASE_SPAN_TABLE_SIZE)
            .flat_map(|span| {
                let octaves = 0.25 + 7.75 * span as f32 / PHASE_SPAN_TABLE_SIZE as f32;
                CENTERED_PHASE_EXPONENTS.map(|exponent| fast_exp2(exponent * octaves))
            })
            .collect::<Vec<_>>()
            .into_boxed_slice()
    })
}

fn phase_span_table() -> &'static [f32] {
    PHASE_SPAN_TABLE.get_or_init(|| {
        (0..=COEFFICIENT_TABLE_SIZE)
            .map(|index| {
                let slope = lerp(
                    MIN_SLOPE_DB,
                    MAX_SLOPE_DB,
                    index as f32 / COEFFICIENT_TABLE_SIZE as f32,
                );
                lerp(
                    0.25,
                    8.0,
                    normalized_log(slope, MIN_SLOPE_DB, MAX_SLOPE_DB),
                )
            })
            .collect::<Vec<_>>()
            .into_boxed_slice()
    })
}

fn phase_span_octaves(slope_db_oct: f32) -> f32 {
    let (index, amount) = slope_table_position(slope_db_oct);
    let table = phase_span_table();
    lerp(table[index], table[index + 1], amount)
}

fn scream_hp_ratio_table() -> &'static [f32] {
    SCREAM_HP_RATIO_TABLE.get_or_init(|| {
        (0..=COEFFICIENT_TABLE_SIZE)
            .map(|index| {
                let slope = lerp(
                    MIN_SLOPE_DB,
                    MAX_SLOPE_DB,
                    index as f32 / COEFFICIENT_TABLE_SIZE as f32,
                );
                (slope / MIN_SLOPE_DB).powf(10.0 / 7.0) / 1_024.0
            })
            .collect::<Vec<_>>()
            .into_boxed_slice()
    })
}

fn scream_hp_ratio(slope_db_oct: f32) -> f32 {
    let (index, amount) = slope_table_position(slope_db_oct);
    let table = scream_hp_ratio_table();
    lerp(table[index], table[index + 1], amount)
}

fn scream_feedback_table() -> &'static [f32] {
    SCREAM_FEEDBACK_TABLE.get_or_init(|| {
        (0..=COEFFICIENT_TABLE_SIZE)
            .map(|index| {
                let resonance = index as f32 / COEFFICIENT_TABLE_SIZE as f32;
                fast_exp2(lerp(-12.0, 12.0, resonance) / 6.020_6)
            })
            .collect::<Vec<_>>()
            .into_boxed_slice()
    })
}

fn scream_feedback(resonance: f32) -> f32 {
    let position = resonance.clamp(0.0, 1.0) * COEFFICIENT_TABLE_SIZE as f32;
    let index = (position as usize).min(COEFFICIENT_TABLE_SIZE - 1);
    let table = scream_feedback_table();
    lerp(table[index], table[index + 1], position - index as f32)
}

fn slope_table_position(slope_db_oct: f32) -> (usize, f32) {
    let position = (slope_db_oct.clamp(MIN_SLOPE_DB, MAX_SLOPE_DB) - MIN_SLOPE_DB)
        * (COEFFICIENT_TABLE_SIZE as f32 / (MAX_SLOPE_DB - MIN_SLOPE_DB));
    let index = (position as usize).min(COEFFICIENT_TABLE_SIZE - 1);
    (index, position - index as f32)
}

fn butterworth_damping_table() -> &'static [f32] {
    BUTTERWORTH_DAMPING.get_or_init(|| {
        let mut table = vec![0.0; MAX_SVF_STAGES * MAX_SVF_STAGES];
        for count in 1..=MAX_SVF_STAGES {
            for index in 0..count {
                let angle = std::f32::consts::PI * (2 * index + 1) as f32 / (4 * count) as f32;
                table[(count - 1) * MAX_SVF_STAGES + index] = 2.0 * angle.cos();
            }
        }
        table.into_boxed_slice()
    })
}

#[inline]
fn coefficient(position: f32, table: &[f32]) -> f32 {
    let position = position.clamp(0.0, COEFFICIENT_TABLE_SIZE as f32);
    let index = (position as usize).min(COEFFICIENT_TABLE_SIZE - 1);
    let amount = position - index as f32;
    table[index] + amount * (table[index + 1] - table[index])
}

#[inline]
fn phase_coefficient(position: f32) -> f32 {
    phase_tangent(
        position.clamp(0.0, COEFFICIENT_TABLE_SIZE as f32)
            * (std::f32::consts::PI * NYQUIST_GUARD / COEFFICIENT_TABLE_SIZE as f32)
            - std::f32::consts::FRAC_PI_4,
    )
}

#[inline]
fn phase_coefficients4(ratios: f32x4, scale: f32, minimum: f32) -> f32x4 {
    let positions = (ratios * f32x4::splat(scale))
        .max(f32x4::splat(minimum))
        .min(f32x4::splat(COEFFICIENT_TABLE_SIZE as f32));
    phase_tangent4(
        positions
            * f32x4::splat(
                std::f32::consts::PI * NYQUIST_GUARD / COEFFICIENT_TABLE_SIZE as f32,
            )
            - f32x4::splat(std::f32::consts::FRAC_PI_4),
    )
}

#[inline]
fn phase_tangent(value: f32) -> f32 {
    let square = value * value;
    value
        * (135_135.0 + square * (-17_325.0 + square * (378.0 - square)))
        / (135_135.0 + square * (-62_370.0 + square * (3_150.0 - 28.0 * square)))
}

#[inline]
fn phase_tangent4(value: f32x4) -> f32x4 {
    let square = value * value;
    value
        * (f32x4::splat(135_135.0)
            + square
                * (f32x4::splat(-17_325.0)
                    + square * (f32x4::splat(378.0) - square)))
        / (f32x4::splat(135_135.0)
            + square
                * (f32x4::splat(-62_370.0)
                    + square * (f32x4::splat(3_150.0) - f32x4::splat(28.0) * square)))
}

fn sanitize_sample_rate(sample_rate: f32) -> f32 {
    if sample_rate.is_finite() && sample_rate >= 1.0 {
        sample_rate
    } else {
        DEFAULT_SAMPLE_RATE
    }
}

fn finite_or(value: f32, fallback: f32) -> f32 {
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
            };
            for frequency in [375.0, 1_500.0, 6_000.0] {
                let analytical = config.response_magnitude(frequency, TEST_SAMPLE_RATE);
                let measured = measured_response(config, frequency);
                assert!(
                    (analytical - measured).abs() < 0.003,
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
            };
            let base = config.coefficients(TEST_SAMPLE_RATE);
            let mut fast = StereoTptSvf::default();
            let mut full = StereoTptSvf::default();
            for sample in 0..256 {
                let modulation = (sample as f32 * 0.037).sin() * 3.5;
                let input = (sample as f32 * 0.23).sin() * 0.2;
                let fast_output = fast.process(base.modulated_cutoff(modulation), input, -input);
                let full_output = full.process(
                    config.modulated(modulation, 0.0, 0.0, 0.0).coefficients(TEST_SAMPLE_RATE),
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
            };
            for control in 0..3 {
                let base = config.coefficients(TEST_SAMPLE_RATE);
                let mut fast = StereoTptSvf::default();
                let mut full = StereoTptSvf::default();
                for sample in 0..256 {
                    let modulation = (sample as f32 * 0.037).sin() * 0.75;
                    let input = (sample as f32 * 0.23).sin() * 0.2;
                    let (fast_coefficients, full_config) = match control {
                        0 => (
                            base.modulated_resonance(modulation * 4.0),
                            config.modulated(0.0, modulation * 4.0, 0.0, 0.0),
                        ),
                        1 => (
                            base.modulated_slope(modulation),
                            config.modulated(0.0, 0.0, modulation, 0.0),
                        ),
                        _ => (
                            base.modulated_morph(modulation),
                            config.modulated(0.0, 0.0, 0.0, modulation),
                        ),
                    };
                    let fast_output = fast.process(fast_coefficients, input, -input);
                    let full_output = full.process(
                        full_config.coefficients(TEST_SAMPLE_RATE),
                        input,
                        -input,
                    );
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
            assert!(previous < 0.08, "slope={} weak stopband", config.slope_db_oct);
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
    fn fractional_phaser_pole_remains_allpass() {
        let coefficients = FilterConfig {
            mode: FilterMode::Phaser,
            q: MAX_Q,
            slope_db_oct: MAX_SLOPE_DB,
            morph: 63.5 / 127.0,
            ..FilterConfig::default()
        }
        .coefficients(TEST_SAMPLE_RATE);
        let count = usize::from(coefficients.processing_stage_count());
        let blend = coefficients.processing_stage_blend();
        for frequency in [20.0, 200.0, 2_000.0, 20_000.0] {
            let mut wet = ComplexResponse::ONE;
            for index in 0..count {
                let target = coefficients.phase_coefficient_at(index);
                let coefficient = if index + 1 == count {
                    lerp(1.0, target, blend)
                } else {
                    target
                };
                wet = wet.multiply(first_order_allpass_response(
                    coefficient,
                    frequency,
                    TEST_SAMPLE_RATE,
                ));
            }
            assert!((wet.magnitude() - 1.0).abs() < 1.0e-4);
        }
    }

    #[test]
    fn phaser_preview_samples_exact_cancellation_frequencies() {
        let config = FilterConfig {
            mode: FilterMode::Phaser,
            q: MAX_Q,
            slope_db_oct: 96.0,
            morph: 0.5,
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
            (65, 0.5)
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
    fn scream_maximum_feedback_stays_finite_and_bounded() {
        let coefficients = FilterConfig {
            mode: FilterMode::Scream,
            cutoff_hz: 18_000.0,
            q: f32::INFINITY,
            slope_db_oct: f32::INFINITY,
            morph: 1.0,
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

    fn measured_response(config: FilterConfig, frequency: f32) -> f32 {
        const AMPLITUDE: f32 = 1.0e-3;
        let coefficients = config.coefficients(TEST_SAMPLE_RATE);
        let mut filter = StereoTptSvf::default();
        let increment = std::f32::consts::TAU * frequency / TEST_SAMPLE_RATE;
        let settle_samples = TEST_SAMPLE_RATE as usize / 4;
        for index in 0..settle_samples {
            let input = AMPLITUDE * (increment * index as f32).cos();
            let _ = filter.process(coefficients, input, input);
        }

        let mut in_phase = 0.0;
        let mut quadrature = 0.0;
        for index in 0..ANALYSIS_SAMPLES {
            let phase = increment * (settle_samples + index) as f32;
            let input = AMPLITUDE * phase.cos();
            let (output, _) = filter.process(coefficients, input, input);
            in_phase = output.mul_add(phase.cos(), in_phase);
            quadrature = output.mul_add(phase.sin(), quadrature);
        }
        2.0 * in_phase.hypot(quadrature) / (ANALYSIS_SAMPLES as f32 * AMPLITUDE)
    }
}
