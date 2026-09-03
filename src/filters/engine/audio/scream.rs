use truce_simd::simd::f32x4;

use super::super::{
    COEFFICIENT_TABLE_SIZE, ComplexResponse, FilterCoefficients, MAX_SLOPE_DB, MAX_SVF_STAGES,
    MIN_SLOPE_DB, SCREAM_FEEDBACK_TABLE, SCREAM_HP_RATIO_TABLE, SCREAM_PREVIEW_INPUT_PEAK,
    StageCoefficients, StereoState, lerp, slope_table_position, svf_stage_response, tick_svf,
};
use crate::voices::fast_exp2;

pub(in crate::filters::engine) fn scream_response(
    coefficients: FilterCoefficients,
    frequency: f32,
    sample_rate: f32,
) -> ComplexResponse {
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
    let open_loop = low
        .multiply(high)
        .multiply(delay)
        .scale(coefficients.scream_feedback);
    let probe = SCREAM_PREVIEW_INPUT_PEAK;
    let mut drive_gain = soft_saturator_fundamental_gain(probe);
    let mut feedback_gain = 1.0;
    let mut wet = low.scale(drive_gain);
    for _ in 0..16 {
        let loop_gain = open_loop.scale(drive_gain * feedback_gain);
        let loop_magnitude = loop_gain.magnitude();
        if loop_magnitude >= 0.97 {
            let compress = (0.96 / loop_magnitude.max(1.0e-6)).sqrt();
            drive_gain = (drive_gain * compress).clamp(1.0e-4, 1.0);
            feedback_gain = (feedback_gain * compress).clamp(1.0e-4, 1.0);
            continue;
        }
        wet = low
            .scale(drive_gain)
            .divide(ComplexResponse::ONE.subtract(loop_gain));
        let max_low = low.magnitude() / probe.max(1.0e-6);
        if wet.magnitude() > max_low {
            wet = wet.scale(max_low / wet.magnitude().max(1.0e-6));
        }
        let feedback = high.scale(coefficients.scream_feedback).multiply(wet);
        let driven_amp = probe
            * ComplexResponse::ONE
                .add(feedback.scale(feedback_gain))
                .magnitude();
        let feedback_amp = probe * feedback.magnitude();
        drive_gain = lerp(
            drive_gain,
            soft_saturator_fundamental_gain(driven_amp),
            0.65,
        );
        feedback_gain = lerp(
            feedback_gain,
            soft_saturator_fundamental_gain(feedback_amp),
            0.65,
        );
    }
    ComplexResponse::ONE.add(wet.subtract(ComplexResponse::ONE).scale(coefficients.morph))
}

pub(in crate::filters::engine) fn soft_saturator_fundamental_gain(amplitude: f32) -> f32 {
    if amplitude <= 1.0e-4 {
        return 1.0;
    }
    const STEPS: usize = 16;
    let mut projection = 0.0;
    for index in 0..STEPS {
        let phase = std::f32::consts::PI * (index as f32 + 0.5) / STEPS as f32;
        let sine = phase.sin();
        let value = amplitude * sine;
        projection += value * (1.0 + value * value).sqrt().recip() * sine;
    }
    (2.0 * projection / (STEPS as f32 * amplitude)).clamp(0.0, 1.0)
}

// Topology follows Cure Audio's MIT-licensed Scream:
// https://github.com/Cure-Audio/Scream
#[inline]
pub(in crate::filters::engine) fn process_scream(
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
pub(in crate::filters::engine) fn soft_saturate(value: f32x4) -> f32x4 {
    value * value.mul_add(value, f32x4::ONE).recip_sqrt()
}

pub(in crate::filters::engine) fn scream_hp_ratio_table() -> &'static [f32] {
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

pub(in crate::filters::engine) fn scream_hp_ratio(slope_db_oct: f32) -> f32 {
    let (index, amount) = slope_table_position(slope_db_oct);
    let table = scream_hp_ratio_table();
    lerp(table[index], table[index + 1], amount)
}

pub(in crate::filters::engine) fn scream_feedback_table() -> &'static [f32] {
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

pub(in crate::filters::engine) fn scream_feedback(resonance: f32) -> f32 {
    let position = resonance.clamp(0.0, 1.0) * COEFFICIENT_TABLE_SIZE as f32;
    let index = (position as usize).min(COEFFICIENT_TABLE_SIZE - 1);
    let table = scream_feedback_table();
    lerp(table[index], table[index + 1], position - index as f32)
}
