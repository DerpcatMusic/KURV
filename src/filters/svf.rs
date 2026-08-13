use std::sync::OnceLock;

use crate::voices::fast_exp2;
use truce_simd::simd::f32x4;

const DEFAULT_SAMPLE_RATE: f32 = 44_100.0;
const MIN_CUTOFF_HZ: f32 = 5.0;
const NYQUIST_GUARD: f32 = 0.495;
const MIN_Q: f32 = 0.1;
const MAX_Q: f32 = 32.0;
const MIN_SLOPE_DB: f32 = 12.0;
const MAX_SLOPE_DB: f32 = 24.0;
const MAX_STAGES: usize = 4;
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
            slope_db_oct: MIN_SLOPE_DB,
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
        let damping = config.q.recip();
        let table_scale = COEFFICIENT_TABLE_SIZE as f32 / (sample_rate * NYQUIST_GUARD);
        let density = (config.slope_db_oct - MIN_SLOPE_DB) / (MAX_SLOPE_DB - MIN_SLOPE_DB);
        let table = coefficient_table();
        let stages = match config.mode {
            FilterMode::Svf => {
                [StageCoefficients::new(config.cutoff_hz, damping, table_scale, table); MAX_STAGES]
            }
            FilterMode::Phaser => {
                let spread = density.mul_add(1.5, 0.25);
                let inner = fast_exp2(spread * 0.5);
                let outer = fast_exp2(spread * 1.5);
                [
                    config.cutoff_hz / outer,
                    config.cutoff_hz / inner,
                    config.cutoff_hz * inner,
                    config.cutoff_hz * outer,
                ]
                .map(|frequency| StageCoefficients::new(frequency, damping, table_scale, table))
            }
            FilterMode::Fibonacci => {
                let spread = density.mul_add(0.35, 0.65);
                [
                    config.cutoff_hz,
                    config.cutoff_hz * fast_exp2(spread),
                    config.cutoff_hz * fast_exp2(spread * 1.584_962_5),
                    config.cutoff_hz * fast_exp2(spread * 2.321_928),
                ]
                .map(|frequency| StageCoefficients::new(frequency, damping, table_scale, table))
            }
        };
        FilterCoefficients {
            mode: config.mode,
            stages,
            slope_position: config.slope_db_oct / MIN_SLOPE_DB,
            morph: config.morph,
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
    fn new(cutoff_hz: f32, damping: f32, table_scale: f32, table: &[f32]) -> Self {
        let g = coefficient(cutoff_hz.max(MIN_CUTOFF_HZ) * table_scale, table);
        let a1 = (1.0 + g * (g + damping)).recip();
        let a2 = g * a1;
        Self {
            damping,
            a1,
            a2,
            a3: g * a2,
        }
    }

    fn interpolate(self, target: Self, amount: f32) -> Self {
        Self {
            damping: lerp(self.damping, target.damping, amount),
            a1: lerp(self.a1, target.a1, amount),
            a2: lerp(self.a2, target.a2, amount),
            a3: lerp(self.a3, target.a3, amount),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FilterCoefficients {
    mode: FilterMode,
    stages: [StageCoefficients; MAX_STAGES],
    slope_position: f32,
    morph: f32,
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
            stages: std::array::from_fn(|index| {
                self.stages[index].interpolate(target.stages[index], amount)
            }),
            slope_position: lerp(self.slope_position, target.slope_position, amount),
            morph: lerp(self.morph, target.morph, amount),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct StereoState {
    integrator_1: f32x4,
    integrator_2: f32x4,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct StereoTptSvf {
    states: [StereoState; MAX_STAGES],
}

impl StereoTptSvf {
    pub fn reset(&mut self) {
        self.states.fill(StereoState::default());
    }

    #[must_use]
    #[inline]
    pub(crate) fn process(
        &mut self,
        coefficients: FilterCoefficients,
        left: f32,
        right: f32,
    ) -> (f32, f32) {
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
    let (low, band, high) = tick_svf(&mut states[0], input, coefficients.stages[0]);
    let first = low_amount.mul_add(low, band_amount.mul_add(band, high_amount * high));
    let (low, band, high) = tick_svf(&mut states[1], first, coefficients.stages[1]);
    let second = low_amount.mul_add(low, band_amount.mul_add(band, high_amount * high));
    first + (second - first) * f32x4::splat(coefficients.slope_position - 1.0)
}

#[inline]
fn process_phase_bank(
    states: &mut [StereoState; MAX_STAGES],
    input: f32x4,
    coefficients: FilterCoefficients,
) -> f32x4 {
    let mut wet = input;
    let mut outputs = [input; MAX_STAGES];
    for index in 0..MAX_STAGES {
        wet = tick_allpass(&mut states[index], wet, coefficients.stages[index]);
        outputs[index] = (input + wet) * f32x4::FRAC_1_SQRT_2;
    }
    select_phase_stage(outputs, coefficients.morph)
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
fn select_phase_stage(outputs: [f32x4; MAX_STAGES], morph: f32) -> f32x4 {
    let position = morph.clamp(0.0, 1.0) * (MAX_STAGES - 1) as f32;
    let output = if position < 1.0 {
        outputs[0] + (outputs[1] - outputs[0]) * f32x4::splat(position)
    } else if position < 2.0 {
        outputs[1] + (outputs[2] - outputs[1]) * f32x4::splat(position - 1.0)
    } else {
        outputs[2] + (outputs[3] - outputs[2]) * f32x4::splat(position - 2.0)
    };
    output
}

#[inline]
fn lerp(from: f32, to: f32, amount: f32) -> f32 {
    amount.mul_add(to - from, from)
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
