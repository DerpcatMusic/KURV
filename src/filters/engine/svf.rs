use truce_simd::simd::f32x4;

use super::{
    BRICKWALL_PROTOTYPE, BUTTERWORTH_DAMPING, ComplexResponse, FilterCoefficients,
    MAX_ACTIVE_SVF_STAGES, MAX_PHASE_POLES, MAX_Q, MAX_RESONANCE_DB, MAX_SLOPE_DB, MAX_SVF_STAGES,
    MIN_SVF_SLOPE_DB, NEUTRAL_SVF_Q, RESONANCE_SKIRT_HEADROOM_DB, StageCoefficients, StereoState,
    finite_or, lerp, normalized_log, svf_stage_response, tick_svf,
};
use crate::voices::fast_exp2;

pub(super) fn svf_resonance_response(
    coefficients: FilterCoefficients,
    frequency: f32,
    sample_rate: f32,
) -> ComplexResponse {
    let amount = coefficients.damping;
    if amount <= f32::EPSILON {
        return ComplexResponse::ONE;
    }
    let damping = svf_resonance_damping(amount);
    let band = svf_stage_response(
        StageCoefficients::from_g(coefficients.g, damping),
        frequency,
        sample_rate,
    )
    .1;
    ComplexResponse::ONE.add(band.scale(svf_resonance_mix_gain(amount, damping)))
}

#[inline]
pub(super) fn process_svf(
    states: &mut [StereoState; MAX_SVF_STAGES],
    resonance_state: &mut StereoState,
    cached_coefficients: &[f32; MAX_PHASE_POLES],
    cached_stage_values: &[f32; MAX_PHASE_POLES],
    cached_damping: &[f32; MAX_SVF_STAGES],
    cached_low_mix: &[f32; MAX_SVF_STAGES],
    cached_band_mix: &[f32; MAX_SVF_STAGES],
    cached_high_mix: &[f32; MAX_SVF_STAGES],
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
            low_mix: cached_low_mix[index],
            band_mix: cached_band_mix[index],
            high_mix: cached_high_mix[index],
        }
    };
    let input = process_svf_resonance(resonance_state, input, coefficients);
    process_svf_stages(states, input, coefficients, stage_at)
        * f32x4::splat(coefficients.morph_gain)
}

#[inline]
pub(super) fn process_svf_resonance(
    state: &mut StereoState,
    input: f32x4,
    coefficients: FilterCoefficients,
) -> f32x4 {
    let amount = coefficients.damping;
    if amount <= f32::EPSILON {
        return input;
    }
    let damping = svf_resonance_damping(amount);
    let band = tick_svf(
        state,
        input,
        StageCoefficients::from_g(coefficients.g, damping),
    )
    .1;
    band.mul_add(f32x4::splat(svf_resonance_mix_gain(amount, damping)), input)
}

#[inline]
pub(super) fn process_svf_stages(
    states: &mut [StereoState; MAX_SVF_STAGES],
    input: f32x4,
    coefficients: FilterCoefficients,
    mut stage_at: impl FnMut(usize) -> StageCoefficients,
) -> f32x4 {
    let blend = coefficients.morph * 2.0 - 1.0;
    let count = coefficients.processing_stage_count().max(1) as usize;
    if coefficients.brickwall > f32::EPSILON {
        if coefficients.brickwall >= 1.0 - f32::EPSILON && blend <= -1.0 {
            return cascade_svf(
                states,
                input,
                count,
                coefficients,
                |state, signal, index| {
                    let stage = stage_at(index);
                    let (low, _, high) = tick_svf(state, signal, stage);
                    f32x4::splat(stage.high_mix).mul_add(high, low)
                },
            );
        }
        if coefficients.brickwall >= 1.0 - f32::EPSILON && blend >= 1.0 {
            return cascade_svf(
                states,
                input,
                count,
                coefficients,
                |state, signal, index| {
                    let stage = stage_at(index);
                    let (low, _, high) = tick_svf(state, signal, stage);
                    f32x4::splat(stage.low_mix).mul_add(low, high)
                },
            );
        }
        return cascade_svf(
            states,
            input,
            count,
            coefficients,
            |state, signal, index| {
                let stage = stage_at(index);
                let (low, band, high) = tick_svf(state, signal, stage);
                f32x4::splat(stage.low_mix).mul_add(
                    low,
                    f32x4::splat(stage.band_mix).mul_add(band, f32x4::splat(stage.high_mix) * high),
                )
            },
        );
    }
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
pub(super) fn cascade_svf(
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
    signal + (last - signal) * f32x4::splat(svf_processing_blend(coefficients))
}

pub(super) fn svf_shape(slope_db_oct: f32, morph: f32) -> (f32, f32) {
    const MAX_CONTINUOUS_SLOPE_DB: f32 = 96.0;

    let slope = finite_or(slope_db_oct, MIN_SVF_SLOPE_DB).clamp(MIN_SVF_SLOPE_DB, MAX_SLOPE_DB);
    let continuous_stages = slope.min(MAX_CONTINUOUS_SLOPE_DB) / 12.0;
    let slope_amount = ((slope - MAX_CONTINUOUS_SLOPE_DB)
        / (MAX_SLOPE_DB - MAX_CONTINUOUS_SLOPE_DB))
        .clamp(0.0, 1.0);
    let edge = (morph.clamp(0.0, 1.0) * 2.0 - 1.0).abs();
    let brickwall = slope_amount * edge;
    let stages = lerp(continuous_stages, MAX_ACTIVE_SVF_STAGES as f32, brickwall);
    (stages, brickwall)
}

pub(super) fn svf_resonance_amount(q: f32) -> f32 {
    normalized_log(q.max(NEUTRAL_SVF_Q), NEUTRAL_SVF_Q, MAX_Q)
}

pub(super) fn svf_resonance_damping(amount: f32) -> f32 {
    lerp(std::f32::consts::SQRT_2, 0.1, amount.clamp(0.0, 1.0))
}

pub(super) fn svf_resonance_mix_gain(amount: f32, damping: f32) -> f32 {
    let peak = fast_exp2(
        amount.clamp(0.0, 1.0) * (MAX_RESONANCE_DB - RESONANCE_SKIRT_HEADROOM_DB) / 6.020_6,
    );
    (peak - 1.0) * damping
}

pub(super) fn svf_cutoff_gain(mut coefficients: FilterCoefficients) -> f32 {
    if coefficients.brickwall > f32::EPSILON {
        let brickwall = coefficients.brickwall;
        coefficients.brickwall = 0.0;
        coefficients.stages = 8.0;
        (
            coefficients.processing_stages,
            coefficients.processing_blend,
        ) = svf_stage_shape(coefficients.stages);
        return lerp(svf_cutoff_gain(coefficients), 1.0, brickwall);
    }
    let blend = coefficients.morph.clamp(0.0, 1.0) * 2.0 - 1.0;
    let layout = SvfStageLayout::new(coefficients);
    let stages = layout.stages;
    let fraction = layout.fraction;
    if blend.abs() <= f32::EPSILON || fraction <= 1.0e-4 && blend.abs() >= 1.0 - f32::EPSILON {
        return 1.0;
    }
    if blend.abs() >= 1.0 - f32::EPSILON {
        let count = layout.count;
        let damping = butterworth_damping_table();
        let mut magnitude = 1.0;
        for index in 0..count {
            let x = if index + 1 == count {
                layout.pole_amount
            } else {
                1.0
            };
            let stage_damping = layout.damping_at(index, damping);
            let denominator = ((1.0 - x * x).powi(2) + (stage_damping * x).powi(2)).sqrt();
            magnitude *= if blend < 0.0 { 1.0 } else { x * x } / denominator;
        }
        return std::f32::consts::FRAC_1_SQRT_2 / magnitude.max(1.0e-6);
    }
    let low = (-blend).max(0.0);
    let high = blend.max(0.0);
    let band = 1.0 - blend.abs();
    let (count, participation) = svf_stage_shape(stages);
    let damping = butterworth_damping_table();
    let mut response = ComplexResponse::ONE;
    for index in 0..usize::from(count) {
        let stage_damping = layout.damping_at(index, damping);
        let stage = ComplexResponse {
            real: band,
            imaginary: (high - low) / stage_damping,
        };
        let full = response.multiply(stage);
        response = if index + 1 == usize::from(count) {
            response.add(full.subtract(response).scale(participation))
        } else {
            full
        };
    }
    let target = fast_exp2(-0.5 * (1.0 - band));
    target / response.magnitude().max(1.0e-6)
}

pub(super) fn svf_stage_damping(stages: f32, index: usize, table: &[f32]) -> f32 {
    let stages = stages.clamp(1.0, MAX_ACTIVE_SVF_STAGES as f32);
    let lower = stages.floor().max(1.0) as usize;
    let upper = stages.ceil().max(1.0) as usize;
    let blend = stages.fract();
    let at = |count: usize, stage: usize| table[(count - 1) * MAX_SVF_STAGES + stage];
    if lower == upper {
        at(upper, index)
    } else if index < lower {
        lerp(at(lower, index), at(upper, index), blend)
    } else {
        at(upper, index)
    }
    .max(1.0e-4)
}

#[derive(Clone, Copy)]
pub(super) struct SvfStageLayout {
    low: f32,
    band: f32,
    high: f32,
    stages: f32,
    fraction: f32,
    pole_amount: f32,
    lower: usize,
    upper: usize,
    count: usize,
    low_side: bool,
}

impl SvfStageLayout {
    #[inline]
    pub(super) fn new(coefficients: FilterCoefficients) -> Self {
        let blend = coefficients.morph * 2.0 - 1.0;
        let stages =
            (coefficients.slope_db_oct.min(96.0) / 12.0).clamp(1.0, MAX_ACTIVE_SVF_STAGES as f32);
        let lower = stages as usize;
        let fraction = stages - lower as f32;
        let upper = lower + usize::from(fraction > f32::EPSILON);
        let pole_curve = (1.1 / (fraction + 0.1)).sqrt();
        let pole_entry = fraction * pole_curve * pole_curve.sqrt();
        Self {
            low: (-blend).max(0.0),
            band: 1.0 - blend.abs(),
            high: blend.max(0.0),
            stages,
            fraction,
            // Enter near-linearly from bypass, then catch up to the useful
            // fractional-slope range without spawning an audible partial stage.
            pole_amount: pole_entry + 2.0 * fraction * (1.0 - fraction) * (1.0 - pole_entry),
            lower,
            upper,
            count: upper,
            low_side: blend < 0.0,
        }
    }

    #[inline]
    fn damping_at(self, index: usize, table: &[f32]) -> f32 {
        let at = |count: usize, stage: usize| table[(count - 1) * MAX_SVF_STAGES + stage];
        if self.lower == self.upper {
            at(self.upper, index)
        } else if index < self.lower {
            lerp(at(self.lower, index), at(self.upper, index), self.fraction)
        } else {
            at(self.upper, index)
        }
        .max(1.0e-4)
    }
}

#[inline]
pub(super) fn svf_stage_at_prepared(
    coefficients: FilterCoefficients,
    layout: SvfStageLayout,
    index: usize,
    damping_table: &[f32],
) -> StageCoefficients {
    let mut base = if index < layout.count {
        let damping = layout.damping_at(index, damping_table);
        let entering = index + 1 == layout.count && layout.fraction > f32::EPSILON;
        let g = if entering && layout.low >= 1.0 - f32::EPSILON {
            (coefficients.g / layout.pole_amount).min(1.0e6)
        } else if entering && layout.high >= 1.0 - f32::EPSILON {
            coefficients.g * layout.pole_amount
        } else {
            coefficients.g
        };
        let mut stage = StageCoefficients::from_g(g, damping);
        stage.low_mix = layout.low;
        stage.band_mix = layout.band * damping;
        stage.high_mix = layout.high;
        stage
    } else {
        let g = if layout.low_side { 1.0e6 } else { 0.0 };
        let mut stage = StageCoefficients::from_g(g, 1.0);
        stage.low_mix = if layout.low_side { 1.0 } else { 0.0 };
        stage.high_mix = if layout.low_side { 0.0 } else { 1.0 };
        stage
    };
    if coefficients.brickwall <= f32::EPSILON {
        return base;
    }

    let (pole_ratio, damping, inverse_zero_ratio_squared) = BRICKWALL_PROTOTYPE[index];
    let pole_ratio = if layout.low_side {
        pole_ratio
    } else {
        pole_ratio.recip()
    };
    let target_g = coefficients.g * pole_ratio;
    let g = if index < layout.count {
        lerp(coefficients.g, target_g, coefficients.brickwall)
    } else if layout.low_side {
        (target_g / coefficients.brickwall.max(1.0e-6)).min(1.0e6)
    } else {
        target_g * coefficients.brickwall
    };
    let mut target =
        StageCoefficients::from_g(g, lerp(base.damping, damping, coefficients.brickwall));
    target.low_mix = if layout.low_side {
        1.0
    } else {
        inverse_zero_ratio_squared
    };
    target.high_mix = if layout.low_side {
        inverse_zero_ratio_squared
    } else {
        1.0
    };
    base.low_mix = lerp(base.low_mix, target.low_mix, coefficients.brickwall);
    base.band_mix = lerp(base.band_mix, 0.0, coefficients.brickwall);
    base.high_mix = lerp(base.high_mix, target.high_mix, coefficients.brickwall);
    target.low_mix = base.low_mix;
    target.band_mix = base.band_mix;
    target.high_mix = base.high_mix;
    target
}

pub(super) fn svf_processing_blend(coefficients: FilterCoefficients) -> f32 {
    let edge = (coefficients.morph * 2.0 - 1.0).abs();
    if coefficients.brickwall <= f32::EPSILON
        && edge >= 1.0 - f32::EPSILON
        && coefficients.stages.fract() > f32::EPSILON
    {
        1.0
    } else {
        coefficients.processing_stage_blend()
    }
}

#[cfg(test)]
pub(super) fn slope_to_svf_stages(slope_db_oct: f32) -> (u8, f32) {
    let stages =
        (finite_or(slope_db_oct, MIN_SVF_SLOPE_DB) / 12.0).clamp(1.0, MAX_SVF_STAGES as f32);
    svf_stage_shape(stages)
}

pub(super) fn butterworth_damping_table() -> &'static [f32] {
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

pub(super) fn svf_stage_shape(stages: f32) -> (u8, f32) {
    let whole = stages.floor();
    let fraction = stages - whole;
    if fraction <= f32::EPSILON {
        (whole as u8, 1.0)
    } else {
        ((whole as u8 + 1).min(MAX_SVF_STAGES as u8), fraction)
    }
}
