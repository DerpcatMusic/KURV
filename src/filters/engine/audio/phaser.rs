use crate::voices::fast_exp2;
use truce_simd::simd::f32x4;

use super::super::{
    CENTERED_PHASE_EXPONENTS, COEFFICIENT_TABLE_SIZE, ComplexResponse, FilterMode, MAX_PHASE_POLES,
    MAX_PHASE_SPAN_OCTAVES, MAX_Q, MAX_SLOPE_DB, MAX_SVF_STAGES, MIN_PHASE_SPAN_OCTAVES, MIN_Q,
    MIN_SLOPE_DB, NYQUIST_GUARD, PHASE_RADIUS_TABLE, PHASE_RADIUS_TABLE_SIZE, PHASE_RATIO_TABLE,
    PHASE_SPAN_TABLE, PHASE_SPAN_TABLE_SIZE, Q_OCTAVES, StereoState, lerp, normalized_log,
    slope_table_position,
};

pub(in crate::filters::engine) fn phase_mix_response(
    wet: ComplexResponse,
    depth: f32,
) -> ComplexResponse {
    let notch = ComplexResponse::ONE.add(wet).scale(0.5);
    ComplexResponse::ONE.add(notch.subtract(ComplexResponse::ONE).scale(depth))
}

pub(in crate::filters::engine) fn phaser_notch_width(
    shape: f32,
    active: u8,
    blend: f32,
    span_octaves: f32,
) -> f32 {
    let shape = shape.clamp(0.0, 1.0);
    let standalone = 0.08 + 2.4 * shape * shape;
    let width_for = |count: usize| {
        if count <= 1 {
            standalone
        } else {
            let spacing = 2.0 * span_octaves / count.next_power_of_two() as f32;
            standalone.min(spacing * lerp(0.12, 0.82, shape * shape))
        }
    };
    let count = usize::from(active.max(1));
    if count == 1 {
        standalone
    } else {
        lerp(
            width_for(count - 1),
            width_for(count),
            blend.clamp(0.0, 1.0),
        )
    }
}

pub(in crate::filters::engine) fn second_order_allpass_response(
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
pub(in crate::filters::engine) fn process_phase_bank(
    states: &mut [StereoState; MAX_SVF_STAGES],
    stage_radii: &mut [f32; MAX_SVF_STAGES],
    target_radii: &[f32; MAX_SVF_STAGES],
    target_pole_cos: &[f32; MAX_SVF_STAGES],
    input: f32x4,
    active: u8,
    blend: f32,
    depth: f32,
    previous_active: u8,
) -> f32x4 {
    let count = active.max(1) as usize;
    let mut wet = input;
    for index in 0..count {
        let radius = target_radii[index];
        if index >= usize::from(previous_active) {
            states[index].fill(wet * f32x4::splat(1.0 - radius * radius));
        } else {
            let previous_radius = stage_radii[index];
            states[index][0] +=
                wet * f32x4::splat((previous_radius - radius) * (previous_radius + radius));
        }
        stage_radii[index] = radius;
        let phased =
            tick_second_order_allpass(&mut states[index], wet, target_pole_cos[index], radius);
        let staged = (wet + phased) * f32x4::splat(0.5);
        wet = if index + 1 == count {
            wet + (staged - wet) * f32x4::splat(blend)
        } else {
            staged
        };
    }
    input + (wet - input) * f32x4::splat(depth)
}

#[inline]
pub(in crate::filters::engine) fn tick_second_order_allpass(
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
pub(in crate::filters::engine) fn phaser_depth(q: f32) -> f32 {
    normalized_log(q, MIN_Q, MAX_Q).sqrt()
}

#[inline]
pub(in crate::filters::engine) fn modulated_phaser_depth(
    depth: f32,
    resonance_octaves: f32,
) -> f32 {
    (depth * depth + resonance_octaves / Q_OCTAVES)
        .clamp(0.0, 1.0)
        .sqrt()
}

pub(in crate::filters::engine) fn cluster_unit(unit: f32, skew: f32) -> f32 {
    let unit = unit.clamp(0.0, 1.0);
    let skew = skew.clamp(0.0, 1.0);
    if skew == 0.5 {
        return unit;
    }
    let factor = fast_exp2((0.5 - skew) * 8.0);
    unit / (unit * (1.0 - factor) + factor)
}

pub(in crate::filters::engine) const fn nested_phase_unit(index: usize) -> f32 {
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

pub(in crate::filters::engine) const fn centered_phase_exponents() -> [f32; MAX_PHASE_POLES] {
    let mut output = [0.0; MAX_PHASE_POLES];
    let mut index = 1;
    while index < MAX_PHASE_POLES {
        output[index] = (nested_phase_unit(index) - 0.5) * 2.0;
        index += 1;
    }
    output
}

pub(in crate::filters::engine) fn phase_frequency_ratio(
    index: usize,
    span_octaves: f32,
    skew: f32,
) -> f32 {
    if index == 0 {
        return 1.0;
    }
    let warped = cluster_unit(nested_phase_unit(index), skew);
    fast_exp2((warped - 0.5) * 2.0 * span_octaves)
}

pub(in crate::filters::engine) fn stage_frequency(
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

pub(in crate::filters::engine) fn phase_ratio_table() -> &'static [f32] {
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

pub(in crate::filters::engine) fn phase_span_table() -> &'static [f32] {
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

pub(in crate::filters::engine) fn phase_span_octaves(slope_db_oct: f32) -> f32 {
    let (index, amount) = slope_table_position(slope_db_oct);
    let table = phase_span_table();
    lerp(table[index], table[index + 1], amount)
}

#[inline]
pub(in crate::filters::engine) fn phase_coefficient(position: f32) -> f32 {
    phase_tangent(
        position.clamp(0.0, COEFFICIENT_TABLE_SIZE as f32)
            * (std::f32::consts::PI * NYQUIST_GUARD / COEFFICIENT_TABLE_SIZE as f32)
            - std::f32::consts::FRAC_PI_4,
    )
}

#[inline]
pub(in crate::filters::engine) fn phase_center_cos(position: f32) -> f32 {
    let coefficient = phase_coefficient(position);
    -2.0 * coefficient / coefficient.mul_add(coefficient, 1.0)
}

/// The stored coefficient is `cos(ω)` of the *notch* center. Pole angle is
/// compensated so changing width cannot walk the notch off that frequency.
pub(in crate::filters::engine) fn phaser_pole(width_octaves: f32, notch_cos: f32) -> (f32, f32) {
    let notch_cos = notch_cos.clamp(-1.0, 1.0);
    phaser_pole_at_angle(
        phaser_width_ratios(width_octaves),
        notch_cos,
        notch_cos.acos(),
        min_radius_for_notch(notch_cos),
    )
}

#[inline]
pub(in crate::filters::engine) fn phaser_pole_at_position(
    width_ratios: [f32; 2],
    position: f32,
) -> (f32, f32) {
    let position = position.clamp(0.0, COEFFICIENT_TABLE_SIZE as f32);
    let coefficient = phase_coefficient(position);
    phaser_pole_at_prepared_position(width_ratios, position, coefficient)
}

#[inline]
pub(in crate::filters::engine) fn phaser_pole_at_prepared_position(
    width_ratios: [f32; 2],
    position: f32,
    coefficient: f32,
) -> (f32, f32) {
    let notch_cos = -2.0 * coefficient / (coefficient * coefficient + 1.0);
    let omega = position * (std::f32::consts::TAU * NYQUIST_GUARD / COEFFICIENT_TABLE_SIZE as f32);
    phaser_pole_at_angle(width_ratios, notch_cos, omega, coefficient.abs())
}

#[inline]
pub(in crate::filters::engine) fn phaser_poles4_at_prepared_positions(
    width_ratios: [f32; 2],
    positions: [f32; 4],
    coefficients: [f32; 4],
) -> ([f32; 4], [f32; 4]) {
    let positions = f32x4::from(positions);
    let coefficients = f32x4::from(coefficients);
    let one = f32x4::splat(1.0);
    let notch_cos = f32x4::splat(-2.0) * coefficients / coefficients.mul_add(coefficients, one);
    let omega = positions
        * f32x4::splat(std::f32::consts::TAU * NYQUIST_GUARD / COEFFICIENT_TABLE_SIZE as f32);
    let lo = (omega * f32x4::splat(width_ratios[0])).max(f32x4::splat(1.0e-5));
    let hi =
        (omega * f32x4::splat(width_ratios[1])).min(f32x4::splat(std::f32::consts::PI - 1.0e-4));
    let radius = phase_radius4((hi - lo).max(f32x4::splat(1.0e-4)))
        .max(coefficients.abs().min(f32x4::splat(0.9995)))
        .min(f32x4::splat(0.9995));
    let pole_cos = (notch_cos * radius.mul_add(radius, one) / (radius * f32x4::splat(2.0)))
        .max(f32x4::splat(-1.0))
        .min(one);
    (radius.to_array(), pole_cos.to_array())
}

#[inline]
pub(in crate::filters::engine) fn phaser_width_ratios(width_octaves: f32) -> [f32; 2] {
    let half = width_octaves.clamp(1.0e-4, 2.48) * 0.5;
    [fast_exp2(-half), fast_exp2(half)]
}

#[inline]
fn phaser_pole_at_angle(
    width_ratios: [f32; 2],
    notch_cos: f32,
    omega: f32,
    minimum_radius: f32,
) -> (f32, f32) {
    let lo = (omega * width_ratios[0]).max(1.0e-5);
    let hi = (omega * width_ratios[1]).min(std::f32::consts::PI - 1.0e-4);
    let bandwidth = (hi - lo).max(1.0e-4);
    let radius = phase_radius(bandwidth).clamp(minimum_radius.min(0.9995), 0.9995);
    (radius, compensated_pole_cos(notch_cos, radius))
}

pub(in crate::filters::engine) fn phase_radius_table() -> &'static [f32] {
    PHASE_RADIUS_TABLE.get_or_init(|| {
        (0..=PHASE_RADIUS_TABLE_SIZE)
            .map(|index| {
                (-0.5 * std::f32::consts::PI * index as f32 / PHASE_RADIUS_TABLE_SIZE as f32).exp()
            })
            .collect::<Vec<_>>()
            .into_boxed_slice()
    })
}

#[inline]
fn phase_radius(bandwidth: f32) -> f32 {
    let position = bandwidth.clamp(0.0, std::f32::consts::PI)
        * (PHASE_RADIUS_TABLE_SIZE as f32 / std::f32::consts::PI);
    let index = (position as usize).min(PHASE_RADIUS_TABLE_SIZE - 1);
    lerp(
        phase_radius_table()[index],
        phase_radius_table()[index + 1],
        position - index as f32,
    )
}

#[inline]
fn phase_radius4(bandwidth: f32x4) -> f32x4 {
    let position = bandwidth
        .max(f32x4::ZERO)
        .min(f32x4::splat(std::f32::consts::PI))
        * f32x4::splat(PHASE_RADIUS_TABLE_SIZE as f32 / std::f32::consts::PI);
    let positions = position.to_array();
    let indices = positions.map(|value| (value as usize).min(PHASE_RADIUS_TABLE_SIZE - 1));
    let table = phase_radius_table();
    let lower = f32x4::from(indices.map(|index| table[index]));
    let upper = f32x4::from(indices.map(|index| table[index + 1]));
    lower + (upper - lower) * (position - f32x4::from(indices.map(|index| index as f32)))
}

pub(in crate::filters::engine) fn min_radius_for_notch(notch_cos: f32) -> f32 {
    let cosine = notch_cos.abs().clamp(0.0, 1.0);
    if cosine <= 1.0e-5 {
        0.0
    } else {
        let sine = (1.0 - cosine * cosine).max(0.0).sqrt();
        ((1.0 - sine) / cosine).clamp(0.0, 0.9995)
    }
}

pub(in crate::filters::engine) fn compensated_pole_cos(notch_cos: f32, radius: f32) -> f32 {
    let radius = radius.max(1.0e-4);
    (notch_cos * (radius * radius + 1.0) / (2.0 * radius)).clamp(-1.0, 1.0)
}

#[inline]
pub(in crate::filters::engine) fn phase_tangent(value: f32) -> f32 {
    let square = value * value;
    value * (135_135.0 + square * (-17_325.0 + square * (378.0 - square)))
        / (135_135.0 + square * (-62_370.0 + square * (3_150.0 - 28.0 * square)))
}
