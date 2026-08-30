use crate::voices::fast_exp2;
use truce_simd::simd::f32x4;

use super::{
    CENTERED_PHASE_EXPONENTS, COEFFICIENT_TABLE_SIZE, ComplexResponse, FilterMode, MAX_PHASE_POLES,
    MAX_PHASE_SPAN_OCTAVES, MAX_Q, MAX_SLOPE_DB, MAX_SVF_STAGES, MIN_PHASE_SPAN_OCTAVES, MIN_Q,
    MIN_SLOPE_DB, NYQUIST_GUARD, PHASE_RATIO_TABLE, PHASE_SPAN_TABLE, PHASE_SPAN_TABLE_SIZE,
    Q_OCTAVES, StereoState, lerp, normalized_log, slope_table_position,
};

pub(super) fn phase_mix_response(wet: ComplexResponse, depth: f32) -> ComplexResponse {
    let notch = ComplexResponse::ONE.add(wet).scale(0.5);
    ComplexResponse::ONE.add(notch.subtract(ComplexResponse::ONE).scale(depth))
}

pub(super) fn second_order_allpass_response(
    center_cos: f32,
    radius: f32,
    frequency: f32,
    sample_rate: f32,
) -> ComplexResponse {
    let angle = std::f32::consts::TAU * frequency / sample_rate;
    let delay = ComplexResponse {
        real: angle.cos(),
        imaginary: -angle.sin(),
    };
    let delay2 = delay.multiply(delay);
    let a1 = -2.0 * radius * center_cos;
    let radius2 = radius * radius;
    ComplexResponse {
        real: radius2,
        imaginary: 0.0,
    }
    .add(delay.scale(a1))
    .add(delay2)
    .divide(
        ComplexResponse::ONE
            .add(delay.scale(a1))
            .add(delay2.scale(radius2)),
    )
}

#[inline]
pub(super) fn process_phase_bank(
    states: &mut [StereoState; MAX_SVF_STAGES],
    stage_coefficients: &[f32; MAX_PHASE_POLES],
    input: f32x4,
    active: u8,
    blend: f32,
    depth: f32,
    shape: f32,
    previous_active: u8,
) -> f32x4 {
    let count = active.max(1) as usize;
    let mut wet = input;
    for index in 0..count {
        let (radius, pole_cos) = phaser_pole(shape, stage_coefficients[index]);
        if index >= usize::from(previous_active) {
            states[index].fill(wet * f32x4::splat(1.0 - radius * radius));
        }
        let staged = tick_second_order_allpass(&mut states[index], wet, pole_cos, radius);
        wet = if index + 1 == count {
            wet + (staged - wet) * f32x4::splat(blend)
        } else {
            staged
        };
    }
    let effected = (input + wet) * f32x4::splat(0.5);
    input + (effected - input) * f32x4::splat(depth)
}

#[inline]
pub(super) fn tick_second_order_allpass(
    state: &mut StereoState,
    input: f32x4,
    center_cos: f32,
    radius: f32,
) -> f32x4 {
    let radius2 = radius * radius;
    let a1 = f32x4::splat(-2.0 * radius * center_cos);
    let output = f32x4::splat(radius2).mul_add(input, state[0]);
    state[0] = a1 * (input - output) + state[1];
    state[1] = input - f32x4::splat(radius2) * output;
    output
}

#[inline]
pub(super) fn phaser_depth(q: f32) -> f32 {
    normalized_log(q, MIN_Q, MAX_Q).sqrt()
}

#[inline]
pub(super) fn modulated_phaser_depth(depth: f32, resonance_octaves: f32) -> f32 {
    (depth * depth + resonance_octaves / Q_OCTAVES)
        .clamp(0.0, 1.0)
        .sqrt()
}

pub(super) fn cluster_unit(unit: f32, skew: f32) -> f32 {
    let unit = unit.clamp(0.0, 1.0);
    let skew = skew.clamp(0.0, 1.0);
    if skew == 0.5 {
        return unit;
    }
    let factor = fast_exp2((0.5 - skew) * 8.0);
    unit / (unit * (1.0 - factor) + factor)
}

pub(super) const fn nested_phase_unit(index: usize) -> f32 {
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

pub(super) const fn centered_phase_exponents() -> [f32; MAX_PHASE_POLES] {
    let mut output = [0.0; MAX_PHASE_POLES];
    let mut index = 1;
    while index < MAX_PHASE_POLES {
        output[index] = (nested_phase_unit(index) - 0.5) * 2.0;
        index += 1;
    }
    output
}

pub(super) fn phase_frequency_ratio(index: usize, span_octaves: f32, skew: f32) -> f32 {
    if index == 0 {
        return 1.0;
    }
    let warped = cluster_unit(nested_phase_unit(index), skew);
    fast_exp2((warped - 0.5) * 2.0 * span_octaves)
}

pub(super) fn stage_frequency(
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

pub(super) fn phase_ratio_table() -> &'static [f32] {
    PHASE_RATIO_TABLE.get_or_init(|| {
        (0..=PHASE_SPAN_TABLE_SIZE)
            .flat_map(|span| {
                let octaves = MIN_PHASE_SPAN_OCTAVES
                    + (MAX_PHASE_SPAN_OCTAVES - MIN_PHASE_SPAN_OCTAVES) * span as f32
                        / PHASE_SPAN_TABLE_SIZE as f32;
                CENTERED_PHASE_EXPONENTS.map(|exponent| fast_exp2(exponent * octaves))
            })
            .collect::<Vec<_>>()
            .into_boxed_slice()
    })
}

pub(super) fn phase_span_table() -> &'static [f32] {
    PHASE_SPAN_TABLE.get_or_init(|| {
        (0..=COEFFICIENT_TABLE_SIZE)
            .map(|index| {
                let slope = lerp(
                    MIN_SLOPE_DB,
                    MAX_SLOPE_DB,
                    index as f32 / COEFFICIENT_TABLE_SIZE as f32,
                );
                lerp(
                    MIN_PHASE_SPAN_OCTAVES,
                    MAX_PHASE_SPAN_OCTAVES,
                    normalized_log(slope, MIN_SLOPE_DB, MAX_SLOPE_DB),
                )
            })
            .collect::<Vec<_>>()
            .into_boxed_slice()
    })
}

pub(super) fn phase_span_octaves(slope_db_oct: f32) -> f32 {
    let (index, amount) = slope_table_position(slope_db_oct);
    let table = phase_span_table();
    lerp(table[index], table[index + 1], amount)
}

#[inline]
pub(super) fn phase_coefficient(position: f32) -> f32 {
    phase_tangent(
        position.clamp(0.0, COEFFICIENT_TABLE_SIZE as f32)
            * (std::f32::consts::PI * NYQUIST_GUARD / COEFFICIENT_TABLE_SIZE as f32)
            - std::f32::consts::FRAC_PI_4,
    )
}

#[inline]
pub(super) fn phase_center_cos(position: f32) -> f32 {
    let coefficient = phase_coefficient(position);
    -2.0 * coefficient / coefficient.mul_add(coefficient, 1.0)
}

#[inline]
pub(super) fn phaser_section_response(
    pole_cos: f32,
    radius: f32,
    frequency: f32,
    sample_rate: f32,
    participation: f32,
) -> ComplexResponse {
    let stage = second_order_allpass_response(pole_cos, radius, frequency, sample_rate);
    if participation >= 1.0 - f32::EPSILON {
        stage
    } else {
        ComplexResponse::ONE
            .scale(1.0 - participation)
            .add(stage.scale(participation))
    }
}

/// Second-order allpass mixed with dry is a notch. Brick widens that notch;
/// Broad keeps it thin so the passbands between stages stay wide.
///
/// The stored coefficient is `cos(ω)` of the *notch* center. Pole angle is
/// compensated so changing width cannot walk the notch off that frequency.
pub(super) fn phaser_pole(shape: f32, notch_cos: f32) -> (f32, f32) {
    let notch_cos = notch_cos.clamp(-1.0, 1.0);
    let omega = notch_cos.acos();
    let shape = shape.clamp(0.0, 1.0);
    let octaves = 0.08 + 2.4 * shape * shape;
    let half = octaves * 0.5;
    let lo = (omega * fast_exp2(-half)).max(1.0e-5);
    let hi = (omega * fast_exp2(half)).min(std::f32::consts::PI - 1.0e-4);
    let bandwidth = (hi - lo).max(1.0e-4);
    let radius = (-0.5 * bandwidth)
        .exp()
        .clamp(min_radius_for_notch(notch_cos), 0.9995);
    (radius, compensated_pole_cos(notch_cos, radius))
}

pub(super) fn min_radius_for_notch(notch_cos: f32) -> f32 {
    let cosine = notch_cos.abs().clamp(0.0, 1.0);
    if cosine <= 1.0e-5 {
        0.0
    } else {
        let sine = (1.0 - cosine * cosine).max(0.0).sqrt();
        ((1.0 - sine) / cosine).clamp(0.0, 0.9995)
    }
}

pub(super) fn compensated_pole_cos(notch_cos: f32, radius: f32) -> f32 {
    let radius = radius.max(1.0e-4);
    (notch_cos * radius.mul_add(radius, 1.0) / (2.0 * radius)).clamp(-1.0, 1.0)
}

#[inline]
pub(super) fn phase_coefficients4(ratios: f32x4, scale: f32, minimum: f32) -> f32x4 {
    let positions = (ratios * f32x4::splat(scale))
        .max(f32x4::splat(minimum))
        .min(f32x4::splat(COEFFICIENT_TABLE_SIZE as f32));
    phase_tangent4(
        positions
            * f32x4::splat(std::f32::consts::PI * NYQUIST_GUARD / COEFFICIENT_TABLE_SIZE as f32)
            - f32x4::splat(std::f32::consts::FRAC_PI_4),
    )
}

#[inline]
pub(super) fn phase_center_cos4(ratios: f32x4, scale: f32, minimum: f32) -> f32x4 {
    let coefficient = phase_coefficients4(ratios, scale, minimum);
    f32x4::splat(-2.0) * coefficient / (coefficient * coefficient + f32x4::ONE)
}

#[inline]
pub(super) fn phase_tangent(value: f32) -> f32 {
    let square = value * value;
    value * (135_135.0 + square * (-17_325.0 + square * (378.0 - square)))
        / (135_135.0 + square * (-62_370.0 + square * (3_150.0 - 28.0 * square)))
}

#[inline]
pub(super) fn phase_tangent4(value: f32x4) -> f32x4 {
    let square = value * value;
    value
        * (f32x4::splat(135_135.0)
            + square * (f32x4::splat(-17_325.0) + square * (f32x4::splat(378.0) - square)))
        / (f32x4::splat(135_135.0)
            + square
                * (f32x4::splat(-62_370.0)
                    + square * (f32x4::splat(3_150.0) - f32x4::splat(28.0) * square)))
}
