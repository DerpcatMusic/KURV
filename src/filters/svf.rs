use std::sync::OnceLock;

use crate::voices::fast_exp2;
use truce_simd::simd::f32x4;

const DEFAULT_SAMPLE_RATE: f32 = 44_100.0;
const MIN_CUTOFF_HZ: f32 = 5.0;
const NYQUIST_GUARD: f32 = 0.495;
pub(crate) const MIN_Q: f32 = 0.1;
pub(crate) const MAX_Q: f32 = 32.0;
pub(crate) const MIN_SLOPE_DB: f32 = 6.0;
pub(crate) const MAX_SLOPE_DB: f32 = 768.0;
const MAX_STAGES: usize = 64;
const PHASE_DAMPING: f32 = 0.68;
const COEFFICIENT_TABLE_SIZE: usize = 2_048;
static COEFFICIENT_TABLE: OnceLock<Box<[f32]>> = OnceLock::new();

pub(crate) fn prepare() {
    let _ = coefficient_table();
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FilterMode {
    #[default]
    Svf,
    Phaser,
    Fibonacci,
}

impl FilterMode {
    pub const ALL: [Self; 3] = [Self::Svf, Self::Phaser, Self::Fibonacci];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Svf => "SVF MORPH",
            Self::Phaser => "PHASER",
            Self::Fibonacci => "FIBONACCI",
        }
    }

    #[must_use]
    pub const fn short_label(self) -> &'static str {
        match self {
            Self::Svf => "SVF",
            Self::Phaser => "PHASE",
            Self::Fibonacci => "FIB",
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
    fn sanitized(self, sample_rate: f32) -> Self {
        let maximum_cutoff = sample_rate * NYQUIST_GUARD;
        Self {
            mode: self.mode,
            cutoff_hz: finite_or(self.cutoff_hz, 20_000.0)
                .clamp(MIN_CUTOFF_HZ.min(maximum_cutoff), maximum_cutoff),
            q: finite_or(self.q, std::f32::consts::FRAC_1_SQRT_2).clamp(MIN_Q, MAX_Q),
            slope_db_oct: finite_or(self.slope_db_oct, MIN_SLOPE_DB)
                .clamp(MIN_SLOPE_DB, MAX_SLOPE_DB),
            morph: finite_or(self.morph, 0.0).clamp(0.0, 1.0),
        }
    }

    #[must_use]
    pub(crate) fn coefficients(self, sample_rate: f32) -> FilterCoefficients {
        let sample_rate = sanitize_sample_rate(sample_rate);
        let config = self.sanitized(sample_rate);
        let (active_stages, stage_blend) = slope_to_stages(config.slope_db_oct);
        let table_scale = COEFFICIENT_TABLE_SIZE as f32 / (sample_rate * NYQUIST_GUARD);
        let damping = match config.mode {
            FilterMode::Svf => {
                let stages = f32::from(active_stages).max(1.0);
                config.q.clamp(MIN_Q, MAX_Q).powf(stages.recip()).recip()
            }
            FilterMode::Phaser | FilterMode::Fibonacci => PHASE_DAMPING,
        };
        let g = coefficient(
            config.cutoff_hz.max(MIN_CUTOFF_HZ) * table_scale,
            coefficient_table(),
        );
        FilterCoefficients {
            mode: config.mode,
            g,
            damping,
            morph: config.morph,
            active_stages,
            stage_blend,
            span_octaves: stage_span_octaves(active_stages),
            focus: q_to_focus(config.q),
            table_scale,
            cutoff_hz: config.cutoff_hz,
        }
    }

    #[must_use]
    pub(crate) fn stage_count(self) -> u8 {
        slope_to_stages(self.slope_db_oct).0
    }

    #[must_use]
    pub(crate) fn stage_frequency(self, index: usize, sample_rate: f32) -> f32 {
        let sample_rate = sanitize_sample_rate(sample_rate);
        let config = self.sanitized(sample_rate);
        let (active_stages, _) = slope_to_stages(config.slope_db_oct);
        stage_frequency(
            config.mode,
            index,
            active_stages,
            config.cutoff_hz,
            stage_span_octaves(active_stages),
            q_to_focus(config.q),
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

    fn new(cutoff_hz: f32, damping: f32, table_scale: f32, table: &[f32]) -> Self {
        Self::from_g(
            coefficient(cutoff_hz.max(MIN_CUTOFF_HZ) * table_scale, table),
            damping,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FilterCoefficients {
    mode: FilterMode,
    g: f32,
    damping: f32,
    morph: f32,
    active_stages: u8,
    stage_blend: f32,
    span_octaves: f32,
    focus: f32,
    table_scale: f32,
    cutoff_hz: f32,
}

impl Default for FilterCoefficients {
    fn default() -> Self {
        FilterConfig::default().coefficients(DEFAULT_SAMPLE_RATE)
    }
}

impl FilterCoefficients {
    #[must_use]
    pub(crate) fn interpolate(self, target: Self, amount: f32) -> Self {
        let amount = amount.clamp(0.0, 1.0);
        Self {
            mode: target.mode,
            g: lerp(self.g, target.g, amount),
            damping: lerp(self.damping, target.damping, amount),
            morph: lerp(self.morph, target.morph, amount),
            active_stages: target.active_stages,
            stage_blend: lerp(self.stage_blend, target.stage_blend, amount),
            span_octaves: lerp(self.span_octaves, target.span_octaves, amount),
            focus: lerp(self.focus, target.focus, amount),
            table_scale: lerp(self.table_scale, target.table_scale, amount),
            cutoff_hz: lerp(self.cutoff_hz, target.cutoff_hz, amount),
        }
    }

    fn stage_at(self, index: usize) -> StageCoefficients {
        match self.mode {
            FilterMode::Svf => StageCoefficients::from_g(self.g, self.damping),
            FilterMode::Phaser | FilterMode::Fibonacci => StageCoefficients::new(
                stage_frequency(
                    self.mode,
                    index,
                    self.active_stages,
                    self.cutoff_hz,
                    self.span_octaves,
                    self.focus,
                ),
                self.damping,
                self.table_scale,
                coefficient_table(),
            ),
        }
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
            let stage_coeffs = coefficients.stage_at(0);
            let (low, band, high) = svf_stage_response(stage_coeffs, frequency, sample_rate);
            let stage = low
                .scale((-blend).max(0.0))
                .add(band.scale((1.0 - blend * blend).max(0.0).sqrt()))
                .add(high.scale(blend.max(0.0)));
            cascade_svf_response(stage, coefficients.active_stages, coefficients.stage_blend)
        }
        FilterMode::Phaser | FilterMode::Fibonacci => {
            let count = coefficients.active_stages.max(1) as usize;
            let position = coefficients.morph.clamp(0.0, 1.0) * (count - 1) as f32;
            let tap = position as usize;
            let frac = position - tap as f32;
            let mut wet = ComplexResponse::ONE;
            let mut tap_a = ComplexResponse::ONE;
            let mut tap_b = ComplexResponse::ONE;
            for index in 0..count {
                let stage = coefficients.stage_at(index);
                let (_, band, _) = svf_stage_response(stage, frequency, sample_rate);
                let allpass = ComplexResponse::ONE.subtract(band.scale(2.0 * stage.damping));
                wet = wet.multiply(allpass);
                let mixed = ComplexResponse::ONE
                    .add(wet)
                    .scale(std::f32::consts::FRAC_1_SQRT_2);
                if index == tap {
                    tap_a = mixed;
                }
                if index == tap + 1 {
                    tap_b = mixed;
                }
            }
            if tap + 1 >= count {
                tap_a
            } else {
                tap_a.add(tap_b.subtract(tap_a).scale(frac))
            }
        }
    }
}

fn cascade_svf_response(
    stage: ComplexResponse,
    active_stages: u8,
    stage_blend: f32,
) -> ComplexResponse {
    let count = active_stages.max(1) as usize;
    let blend = stage_blend.clamp(0.0, 1.0);
    if count == 1 {
        return ComplexResponse::ONE.add(stage.subtract(ComplexResponse::ONE).scale(blend));
    }
    let mut response = stage;
    for _ in 1..count - 1 {
        response = response.multiply(stage);
    }
    let last = response.multiply(stage);
    response.add(last.subtract(response).scale(blend))
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

#[derive(Clone, Copy, Debug, Default)]
struct StereoState {
    integrator_1: f32x4,
    integrator_2: f32x4,
}

#[derive(Clone, Copy, Debug)]
pub struct StereoTptSvf {
    states: [StereoState; MAX_STAGES],
    last_active: u8,
}

impl Default for StereoTptSvf {
    fn default() -> Self {
        Self {
            states: [StereoState::default(); MAX_STAGES],
            last_active: 0,
        }
    }
}

impl StereoTptSvf {
    pub fn reset(&mut self) {
        self.states.fill(StereoState::default());
        self.last_active = 0;
    }

    fn retire_unused(&mut self, active: u8) {
        let previous = self.last_active;
        if active < previous {
            self.states[usize::from(active)..usize::from(previous)].fill(StereoState::default());
        }
        self.last_active = active;
    }

    #[must_use]
    #[inline]
    pub(crate) fn process(
        &mut self,
        coefficients: FilterCoefficients,
        left: f32,
        right: f32,
    ) -> (f32, f32) {
        self.retire_unused(coefficients.active_stages);
        let input = f32x4::from([finite_or(left, 0.0), finite_or(right, 0.0), 0.0, 0.0]);
        let output = match coefficients.mode {
            FilterMode::Svf => process_svf(&mut self.states, input, coefficients),
            FilterMode::Phaser | FilterMode::Fibonacci => {
                process_phase_bank(&mut self.states, input, coefficients)
            }
        }
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
fn process_svf(
    states: &mut [StereoState; MAX_STAGES],
    input: f32x4,
    coefficients: FilterCoefficients,
) -> f32x4 {
    let blend = coefficients.morph.mul_add(2.0, -1.0);
    let low_amount = f32x4::splat((-blend).max(0.0));
    let high_amount = f32x4::splat(blend.max(0.0));
    let band_amount = f32x4::splat((1.0 - blend * blend).max(0.0).sqrt());
    let stage = coefficients.stage_at(0);
    let mix_stage = |state: &mut StereoState, signal: f32x4| {
        let (low, band, high) = tick_svf(state, signal, stage);
        low_amount.mul_add(low, band_amount.mul_add(band, high_amount * high))
    };
    let count = coefficients.active_stages.max(1) as usize;
    let first = mix_stage(&mut states[0], input);
    if count == 1 {
        return input + (first - input) * f32x4::splat(coefficients.stage_blend);
    }
    let mut signal = first;
    for state in states.iter_mut().take(count - 1).skip(1) {
        signal = mix_stage(state, signal);
    }
    let last = mix_stage(&mut states[count - 1], signal);
    signal + (last - signal) * f32x4::splat(coefficients.stage_blend)
}

#[inline]
fn process_phase_bank(
    states: &mut [StereoState; MAX_STAGES],
    input: f32x4,
    coefficients: FilterCoefficients,
) -> f32x4 {
    let count = coefficients.active_stages.max(1) as usize;
    let position = coefficients.morph.clamp(0.0, 1.0) * (count - 1) as f32;
    let tap = position as usize;
    let frac = f32x4::splat(position - tap as f32);
    let mut wet = input;
    let mut tap_a = input;
    let mut tap_b = input;
    for index in 0..count {
        wet = tick_allpass(&mut states[index], wet, coefficients.stage_at(index));
        let mixed = (input + wet) * f32x4::FRAC_1_SQRT_2;
        if index == tap {
            tap_a = mixed;
        }
        if index == tap + 1 {
            tap_b = mixed;
        }
    }
    if tap + 1 >= count {
        tap_a
    } else {
        tap_a + (tap_b - tap_a) * frac
    }
}

#[inline]
fn tick_allpass(state: &mut StereoState, input: f32x4, coefficients: StageCoefficients) -> f32x4 {
    let v3 = input - state.integrator_2;
    let band =
        f32x4::splat(coefficients.a1) * state.integrator_1 + f32x4::splat(coefficients.a2) * v3;
    let low = f32x4::splat(coefficients.a2) * state.integrator_1
        + f32x4::splat(coefficients.a3) * v3
        + state.integrator_2;
    state.integrator_1 = band * f32x4::splat(2.0) - state.integrator_1;
    state.integrator_2 = low * f32x4::splat(2.0) - state.integrator_2;
    input - f32x4::splat(2.0 * coefficients.damping) * band
}

#[inline]
fn tick_svf(
    state: &mut StereoState,
    input: f32x4,
    coefficients: StageCoefficients,
) -> (f32x4, f32x4, f32x4) {
    let v3 = input - state.integrator_2;
    let band =
        f32x4::splat(coefficients.a1) * state.integrator_1 + f32x4::splat(coefficients.a2) * v3;
    let low = f32x4::splat(coefficients.a2) * state.integrator_1
        + f32x4::splat(coefficients.a3) * v3
        + state.integrator_2;
    let high = input - low - f32x4::splat(coefficients.damping) * band;
    state.integrator_1 = band * f32x4::splat(2.0) - state.integrator_1;
    state.integrator_2 = low * f32x4::splat(2.0) - state.integrator_2;
    (low, band, high)
}

#[inline]
fn lerp(from: f32, to: f32, amount: f32) -> f32 {
    amount.mul_add(to - from, from)
}

fn slope_to_stages(slope_db_oct: f32) -> (u8, f32) {
    let stages = (finite_or(slope_db_oct, MIN_SLOPE_DB) / 12.0).clamp(0.5, MAX_STAGES as f32);
    let whole = stages.floor();
    let fraction = stages - whole;
    if whole < 1.0 {
        (1, fraction.clamp(0.5, 1.0))
    } else if fraction <= 1.0e-4 {
        (whole as u8, 1.0)
    } else {
        ((whole as u8 + 1).min(MAX_STAGES as u8), fraction)
    }
}

fn q_to_focus(q: f32) -> f32 {
    ((finite_or(q, std::f32::consts::FRAC_1_SQRT_2).clamp(MIN_Q, MAX_Q) / MIN_Q).ln()
        / (MAX_Q / MIN_Q).ln())
    .clamp(0.0, 1.0)
}

fn stage_span_octaves(active_stages: u8) -> f32 {
    (0.55 + 0.075 * f32::from(active_stages)).min(5.5)
}

fn cluster_unit(unit: f32, focus: f32) -> f32 {
    let unit = unit.clamp(0.0, 1.0);
    let focus = focus.clamp(0.0, 1.0);
    let amount = (focus - 0.5).abs() * 2.0;
    let center = {
        let offset = unit.mul_add(2.0, -1.0);
        0.5 + 0.5 * offset.signum() * offset.abs().powf(0.42)
    };
    if amount <= 1.0e-4 {
        return center;
    }
    let piled = if focus < 0.5 {
        unit.powf(1.0 + amount * 2.4)
    } else {
        1.0 - (1.0 - unit).powf(1.0 + amount * 2.4)
    };
    lerp(center, piled, amount)
}

fn stage_frequency(
    mode: FilterMode,
    index: usize,
    active_stages: u8,
    cutoff_hz: f32,
    span_octaves: f32,
    focus: f32,
) -> f32 {
    let count = active_stages.max(1) as usize;
    if matches!(mode, FilterMode::Svf) || count == 1 {
        return cutoff_hz;
    }
    let unit = (index as f32 + 0.5) / count as f32;
    let warped = cluster_unit(unit, focus);
    let octaves = match mode {
        FilterMode::Phaser => (warped - 0.5) * 2.0 * span_octaves,
        FilterMode::Fibonacci => warped * span_octaves * 2.321_928,
        FilterMode::Svf => 0.0,
    };
    cutoff_hz * fast_exp2(octaves)
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

#[inline]
fn coefficient(position: f32, table: &[f32]) -> f32 {
    let position = position.clamp(0.0, COEFFICIENT_TABLE_SIZE as f32);
    let index = (position as usize).min(COEFFICIENT_TABLE_SIZE - 1);
    let amount = position - index as f32;
    lerp(table[index], table[index + 1], amount)
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
    fn slope_maps_six_db_to_a_partial_first_stage() {
        assert_eq!(slope_to_stages(6.0), (1, 0.5));
        assert_eq!(slope_to_stages(12.0), (1, 1.0));
        assert_eq!(slope_to_stages(18.0), (2, 0.5));
        assert_eq!(slope_to_stages(768.0), (64, 1.0));
    }

    #[test]
    fn q_skews_phaser_density_left_center_or_right() {
        let left = cluster_unit(0.25, 0.0);
        let center = cluster_unit(0.25, 0.5);
        let right = cluster_unit(0.25, 1.0);
        assert!(left < center);
        assert!(center < right);
        let low = stage_frequency(FilterMode::Phaser, 0, 8, 1_000.0, 3.0, 0.0);
        let high = stage_frequency(FilterMode::Phaser, 0, 8, 1_000.0, 3.0, 1.0);
        assert!(low < high);
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

    fn measured_response(config: FilterConfig, frequency: f32) -> f32 {
        let coefficients = config.coefficients(TEST_SAMPLE_RATE);
        let mut filter = StereoTptSvf::default();
        let increment = std::f32::consts::TAU * frequency / TEST_SAMPLE_RATE;
        let settle_samples = TEST_SAMPLE_RATE as usize / 4;
        for index in 0..settle_samples {
            let input = (increment * index as f32).cos();
            let _ = filter.process(coefficients, input, input);
        }

        let mut in_phase = 0.0;
        let mut quadrature = 0.0;
        for index in 0..ANALYSIS_SAMPLES {
            let phase = increment * (settle_samples + index) as f32;
            let input = phase.cos();
            let (output, _) = filter.process(coefficients, input, input);
            in_phase = output.mul_add(phase.cos(), in_phase);
            quadrature = output.mul_add(phase.sin(), quadrature);
        }
        2.0 * in_phase.hypot(quadrature) / ANALYSIS_SAMPLES as f32
    }
}
