use std::hint::black_box;
use std::mem::size_of;
use std::time::Instant;

use crate::dsp::{Complex, fft};
use truce_simd::simd::{f32x4, f32x8};
use wide::CmpGt;

use super::{
    MAX_WAVE_KNOTS, RT_SEGMENTS, SourceCurve, WaveCurveData, WaveCurveRt, WaveKnot,
    coefficient_index, has_tight_transition, sanitize_knots,
};

const GRID: usize = 65_536;
const POLY_SYSTEM_MAX: usize = 112;

#[derive(Clone, Copy)]
struct UniformQuartic12 {
    coefficients: [f32; 64],
}

#[derive(Clone, Copy)]
struct UniformQuintic10 {
    coefficients: [f32; 64],
}

fn constrained_polynomial_fit(
    source: &SourceCurve,
    knots: &[WaveKnot],
    segments: usize,
    degree: usize,
) -> [f32; 64] {
    const VARIABLES: usize = 60;
    const MAX_CONSTRAINTS: usize = 52;
    let mut normal = [[0.0_f64; VARIABLES + 1]; VARIABLES];
    for segment in 0..segments {
        let start = segment as f64 / segments as f64;
        let width = 1.0 / segments as f64;
        for sample in 1..16 {
            let t = sample as f64 / 16.0;
            let phase = width.mul_add(t, start);
            let mut features = [0.0; 6];
            features[0] = 1.0;
            for power in 1..=degree {
                features[power] = features[power - 1] * t;
            }
            let target = source.eval(phase);
            for row_power in 0..=degree {
                let row = segment * (degree + 1) + row_power;
                for column_power in 0..=degree {
                    let column = segment * (degree + 1) + column_power;
                    normal[row][column] += features[row_power] * features[column_power];
                }
                normal[row][VARIABLES] += features[row_power] * target;
            }
        }
    }
    for index in 0..VARIABLES {
        normal[index][index] += 1.0e-12;
    }

    let mut constraints = [([0.0_f64; VARIABLES], 0.0_f64); MAX_CONSTRAINTS];
    let mut constraint_count = 0;
    let mut push = |row: [f64; VARIABLES], target: f64| {
        constraints[constraint_count] = (row, target);
        constraint_count += 1;
    };
    for segment in 0..segments {
        let mut start_row = [0.0; VARIABLES];
        start_row[segment * (degree + 1)] = 1.0;
        push(start_row, source.eval(segment as f64 / segments as f64));
        let mut end_row = [0.0; VARIABLES];
        for power in 0..=degree {
            end_row[segment * (degree + 1) + power] = 1.0;
        }
        push(end_row, source.eval((segment + 1) as f64 / segments as f64));
    }
    let mut join = |left: usize, right: usize| {
        let mut slope = [0.0; VARIABLES];
        for power in 1..=degree {
            slope[left * (degree + 1) + power] = power as f64;
        }
        slope[right * (degree + 1) + 1] -= 1.0;
        push(slope, 0.0);
        let mut curvature = [0.0; VARIABLES];
        for power in 2..=degree {
            curvature[left * (degree + 1) + power] = (power * (power - 1)) as f64;
        }
        curvature[right * (degree + 1) + 2] -= 2.0;
        push(curvature, 0.0);
    };
    for boundary in 1..segments {
        let phase = boundary as f32 / segments as f32;
        let hard = knots.iter().any(|knot| {
            (knot.phase - phase).abs() < 1.0e-6
                && (source_value_slope(source, phase, false).1
                    - source_value_slope(source, phase, true).1)
                    .abs()
                    > 1.0e-3
        });
        if !hard {
            join(boundary - 1, boundary);
        }
    }
    if (source_value_slope(source, 0.0, false).1 - source_value_slope(source, 0.0, true).1).abs()
        <= 1.0e-3
    {
        join(segments - 1, 0);
    }

    let system_count = VARIABLES + constraint_count;
    let mut system = [[0.0_f64; POLY_SYSTEM_MAX + 1]; POLY_SYSTEM_MAX];
    for row in 0..VARIABLES {
        system[row][..VARIABLES].copy_from_slice(&normal[row][..VARIABLES]);
        system[row][system_count] = normal[row][VARIABLES];
    }
    for (constraint, (coefficients, target)) in constraints[..constraint_count].iter().enumerate() {
        let row = VARIABLES + constraint;
        for column in 0..VARIABLES {
            system[column][row] = coefficients[column];
            system[row][column] = coefficients[column];
        }
        system[row][system_count] = *target;
    }
    for pivot in 0..system_count {
        let best = (pivot..system_count)
            .max_by(|&left, &right| {
                system[left][pivot]
                    .abs()
                    .total_cmp(&system[right][pivot].abs())
            })
            .unwrap_or(pivot);
        system.swap(pivot, best);
        let scale = system[pivot][pivot];
        if scale.abs() <= 1.0e-14 {
            continue;
        }
        for column in pivot..=system_count {
            system[pivot][column] /= scale;
        }
        for row in 0..system_count {
            if row == pivot {
                continue;
            }
            let scale = system[row][pivot];
            for column in pivot..=system_count {
                system[row][column] -= scale * system[pivot][column];
            }
        }
    }
    let mut result = [0.0; 64];
    for index in 0..VARIABLES {
        result[index] = system[index][system_count] as f32;
    }
    result
}

impl UniformQuartic12 {
    fn compile(source: &SourceCurve, knots: &[WaveKnot]) -> Self {
        Self {
            coefficients: constrained_polynomial_fit(source, knots, 12, 4),
        }
    }

    fn eval_raw(self, phase: f32) -> f32 {
        eval_fixed_polynomial(self.coefficients, phase, 12, 4)
    }
}

impl UniformQuintic10 {
    fn compile(source: &SourceCurve, knots: &[WaveKnot]) -> Self {
        Self {
            coefficients: constrained_polynomial_fit(source, knots, 10, 5),
        }
    }

    fn eval_raw(self, phase: f32) -> f32 {
        eval_fixed_polynomial(self.coefficients, phase, 10, 5)
    }
}

fn eval_fixed_polynomial(
    coefficients: [f32; 64],
    phase: f32,
    segments: usize,
    degree: usize,
) -> f32 {
    let position = phase * segments as f32;
    let segment = (position as usize).min(segments - 1);
    let t = position - segment as f32;
    let base = segment * (degree + 1);
    let mut value = coefficients[base + degree];
    for power in (0..degree).rev() {
        value = value.mul_add(t, coefficients[base + power]);
    }
    value
}

fn polynomial_event_metrics(
    coefficients: [f32; 64],
    segments: usize,
    degree: usize,
    hard: &[f32],
) -> (f32, f64, f32, f64, usize, f32, f32) {
    let derivative = |segment: usize, at_end: bool| {
        let base = segment * (degree + 1);
        if at_end {
            (1..=degree)
                .map(|power| power as f32 * coefficients[base + power])
                .sum::<f32>()
                * segments as f32
        } else {
            coefficients[base + 1] * segments as f32
        }
    };
    let curvature = |segment: usize, at_end: bool| {
        let base = segment * (degree + 1);
        if at_end {
            (2..=degree)
                .map(|power| (power * (power - 1)) as f32 * coefficients[base + power])
                .sum::<f32>()
                * (segments * segments) as f32
        } else {
            2.0 * coefficients[base + 2] * (segments * segments) as f32
        }
    };
    let mut max_slope = 0.0_f32;
    let mut slope_energy = 0.0;
    let mut max_curvature = 0.0_f32;
    let mut curvature_energy = 0.0;
    let mut event_count = 0;
    let mut hard_jump = 0.0_f32;
    for boundary in 1..segments {
        let phase = boundary as f32 / segments as f32;
        let slope_jump = (derivative(boundary, false) - derivative(boundary - 1, true)).abs();
        let curvature_jump = (curvature(boundary, false) - curvature(boundary - 1, true)).abs();
        if hard.iter().any(|hard| (*hard - phase).abs() < 1.0e-6) {
            hard_jump = hard_jump.max(slope_jump);
        } else {
            max_slope = max_slope.max(slope_jump);
            max_curvature = max_curvature.max(curvature_jump);
            slope_energy += f64::from(slope_jump * slope_jump);
            curvature_energy += f64::from(curvature_jump * curvature_jump);
            event_count += usize::from(slope_jump > 1.0e-3 || curvature_jump > 1.0e-2);
        }
    }
    let wrap = (derivative(0, false) - derivative(segments - 1, true)).abs();
    (
        max_slope,
        slope_energy,
        max_curvature,
        curvature_energy,
        event_count,
        hard_jump,
        wrap,
    )
}

#[derive(Clone, Copy)]
struct UniformQuadratic21 {
    coefficients: [f32; 64],
}

impl UniformQuadratic21 {
    fn compile(source: &SourceCurve) -> Self {
        let mut coefficients = [0.0; 64];
        for segment in 0..21 {
            let start = segment as f64 / 21.0;
            let width = 1.0 / 21.0;
            let y0 = source.eval(start);
            let y1 = source.eval(start + width);
            let mut numerator = 0.0;
            let mut denominator = 0.0;
            for sample in 1..32 {
                let t = sample as f64 / 32.0;
                let feature = t * t - t;
                let target = source.eval(width.mul_add(t, start)) - y0 - (y1 - y0) * t;
                numerator += feature * target;
                denominator += feature * feature;
            }
            let a = numerator / denominator;
            coefficients[segment * 3] = a as f32;
            coefficients[segment * 3 + 1] = (y1 - y0 - a) as f32;
            coefficients[segment * 3 + 2] = y0 as f32;
        }
        Self { coefficients }
    }

    #[inline]
    fn eval_raw(self, phase: f32) -> f32 {
        let position = phase * 21.0;
        let segment = (position as usize).min(20);
        let t = position - segment as f32;
        self.coefficients[segment * 3]
            .mul_add(t, self.coefficients[segment * 3 + 1])
            .mul_add(t, self.coefficients[segment * 3 + 2])
    }
}

#[derive(Clone, Copy)]
struct UniformLinear32 {
    coefficients: [f32; 64],
}

impl UniformLinear32 {
    fn compile(source: &SourceCurve) -> Self {
        let mut coefficients = [0.0; 64];
        for segment in 0..32 {
            let start = segment as f64 / 32.0;
            let y0 = source.eval(start);
            let y1 = source.eval(start + 1.0 / 32.0);
            coefficients[segment * 2] = (y1 - y0) as f32;
            coefficients[segment * 2 + 1] = y0 as f32;
        }
        Self { coefficients }
    }

    #[inline]
    fn eval_raw(self, phase: f32) -> f32 {
        let position = phase * 32.0;
        let segment = (position as usize).min(31);
        let t = position - segment as f32;
        self.coefficients[segment * 2].mul_add(t, self.coefficients[segment * 2 + 1])
    }
}

fn quadratic_derivative_metrics(curve: UniformQuadratic21, hard: &[f32]) -> (f32, f32, f32) {
    let mut smooth = 0.0_f32;
    let mut hard_jump = 0.0_f32;
    for boundary in 1..21 {
        let left = boundary - 1;
        let left_slope =
            (2.0 * curve.coefficients[left * 3] + curve.coefficients[left * 3 + 1]) * 21.0;
        let right_slope = curve.coefficients[boundary * 3 + 1] * 21.0;
        let jump = (right_slope - left_slope).abs();
        if hard
            .iter()
            .any(|phase| (*phase - boundary as f32 / 21.0).abs() < 1.0e-6)
        {
            hard_jump = hard_jump.max(jump);
        } else {
            smooth = smooth.max(jump);
        }
    }
    let wrap = (curve.coefficients[1] * 21.0
        - (2.0 * curve.coefficients[60] + curve.coefficients[61]) * 21.0)
        .abs();
    (smooth, hard_jump, wrap)
}

fn linear_derivative_metrics(curve: UniformLinear32, hard: &[f32]) -> (f32, f32, f32) {
    let mut smooth = 0.0_f32;
    let mut hard_jump = 0.0_f32;
    for boundary in 1..32 {
        let jump = ((curve.coefficients[boundary * 2] - curve.coefficients[(boundary - 1) * 2])
            * 32.0)
            .abs();
        if hard
            .iter()
            .any(|phase| (*phase - boundary as f32 / 32.0).abs() < 1.0e-6)
        {
            hard_jump = hard_jump.max(jump);
        } else {
            smooth = smooth.max(jump);
        }
    }
    let wrap = ((curve.coefficients[0] - curve.coefficients[62]) * 32.0).abs();
    (smooth, hard_jump, wrap)
}

fn quadratic_crossings(curve: UniformQuadratic21) -> usize {
    let mut crossings = 0;
    for segment in 0..21 {
        let a = curve.coefficients[segment * 3];
        let b = curve.coefficients[segment * 3 + 1];
        let d = curve.coefficients[segment * 3 + 2];
        let mut values = [d, a + b + d, 0.0];
        let mut count = 2;
        if a.abs() > f32::EPSILON {
            let root = -b / (2.0 * a);
            if (0.0..1.0).contains(&root) {
                values[2] = a.mul_add(root, b).mul_add(root, d);
                count = 3;
            }
        }
        crossings += values[..count]
            .iter()
            .filter(|&&value| !(-1.0..=1.0).contains(&value))
            .count();
    }
    crossings
}

#[derive(Clone, Copy)]
struct AdaptiveCubic {
    count: usize,
    boundaries: [f32; RT_SEGMENTS + 1],
    coefficients: [[f32; 4]; RT_SEGMENTS],
}

impl AdaptiveCubic {
    fn compile(data: &WaveCurveData) -> Self {
        let knots = sanitize_knots(&data.knots);
        let source = SourceCurve::compile(&knots);
        let mut boundaries = [1.0; RT_SEGMENTS + 1];
        boundaries[..source.count].copy_from_slice(&source.x0[..source.count]);
        boundaries[source.count] = 1.0;
        let mut count = source.count;

        while count < RT_SEGMENTS {
            let split = (0..count)
                .max_by(|&left, &right| {
                    interval_error(&source, boundaries[left], boundaries[left + 1]).total_cmp(
                        &interval_error(&source, boundaries[right], boundaries[right + 1]),
                    )
                })
                .unwrap_or(0);
            boundaries.copy_within(split + 1..=count, split + 2);
            boundaries[split + 1] = (boundaries[split] + boundaries[split + 2]) * 0.5;
            count += 1;
        }

        let coefficients = std::array::from_fn(|index| {
            if index >= count {
                return [0.0; 4];
            }
            cubic_coefficients(&source, boundaries[index], boundaries[index + 1])
        });
        Self {
            count,
            boundaries,
            coefficients,
        }
    }

    #[inline]
    fn eval_raw(self, phase: f32) -> f32 {
        let index = self.boundaries[1..self.count].partition_point(|boundary| phase >= *boundary);
        let width = self.boundaries[index + 1] - self.boundaries[index];
        let t = (phase - self.boundaries[index]) / width;
        let [a, b, c, d] = self.coefficients[index];
        a.mul_add(t, b).mul_add(t, c).mul_add(t, d)
    }

    fn extrema(self) -> (f32, f32, usize) {
        let mut minimum = f32::INFINITY;
        let mut maximum = f32::NEG_INFINITY;
        let mut crossings = 0;
        for index in 0..self.count {
            let [a, b, c, d] = self.coefficients[index];
            let mut values = [d, a + b + c + d, 0.0, 0.0];
            let mut value_count = 2;
            let discriminant = 4.0 * b * b - 12.0 * a * c;
            if a.abs() <= f32::EPSILON {
                if b.abs() > f32::EPSILON {
                    let root = -c / (2.0 * b);
                    if (0.0..1.0).contains(&root) {
                        values[value_count] = a.mul_add(root, b).mul_add(root, c).mul_add(root, d);
                        value_count += 1;
                    }
                }
            } else if discriminant >= 0.0 {
                let root = discriminant.sqrt();
                for t in [(-2.0 * b - root) / (6.0 * a), (-2.0 * b + root) / (6.0 * a)] {
                    if (0.0..1.0).contains(&t) {
                        values[value_count] = a.mul_add(t, b).mul_add(t, c).mul_add(t, d);
                        value_count += 1;
                    }
                }
            }
            for value in &values[..value_count] {
                minimum = minimum.min(*value);
                maximum = maximum.max(*value);
                crossings += usize::from(!(-1.0..=1.0).contains(value));
            }
        }
        (minimum, maximum, crossings)
    }
}

fn source_value_slope(source: &SourceCurve, phase: f32, from_left: bool) -> (f32, f32) {
    let probe = if from_left {
        (phase - f32::EPSILON).rem_euclid(1.0)
    } else {
        phase.rem_euclid(1.0)
    };
    let index = (0..source.count)
        .find(|&index| probe < source.x1[index])
        .unwrap_or(source.count - 1);
    let width = source.x1[index] - source.x0[index];
    let t = ((probe - source.x0[index]) / width).clamp(0.0, 1.0);
    let curve = source.curve[index];
    let curve_x = source.curve_x[index];
    let mut value = t - curve_x * t * (1.0 - t);
    let mut slope = 1.0 - curve_x + 2.0 * curve_x * t;
    let direction = curve.signum();
    let magnitude = curve.abs().min(4.0);
    for _ in 0..magnitude.floor() as usize {
        slope *= 1.0 + direction * (1.0 - 2.0 * value);
        value += direction * value * (1.0 - value);
    }
    let fraction = magnitude.fract();
    slope *= 1.0 + direction * fraction * (1.0 - 2.0 * value);
    value += direction * fraction * value * (1.0 - value);
    let delta = source.c[index];
    (delta.mul_add(value, source.d[index]), delta * slope / width)
}

fn cubic_coefficients(source: &SourceCurve, start: f32, end: f32) -> [f32; 4] {
    let width = end - start;
    let (y0, slope0) = source_value_slope(source, start, false);
    let (y1, slope1) = source_value_slope(source, end, true);
    let m0 = slope0 * width;
    let m1 = slope1 * width;
    [
        2.0 * y0 - 2.0 * y1 + m0 + m1,
        -3.0 * y0 + 3.0 * y1 - 2.0 * m0 - m1,
        m0,
        y0,
    ]
}

fn eval_cubic(coefficients: [f32; 4], t: f32) -> f32 {
    coefficients[0]
        .mul_add(t, coefficients[1])
        .mul_add(t, coefficients[2])
        .mul_add(t, coefficients[3])
}

fn interval_error(source: &SourceCurve, start: f32, end: f32) -> f32 {
    let coefficients = cubic_coefficients(source, start, end);
    (1..32)
        .map(|sample| {
            let t = sample as f32 / 32.0;
            let phase = (end - start).mul_add(t, start);
            (eval_cubic(coefficients, t) - source.eval(f64::from(phase)) as f32).abs()
        })
        .fold(0.0, f32::max)
}

fn shipping_raw(curve: WaveCurveRt, phase: f32) -> f32 {
    let position = phase * RT_SEGMENTS as f32;
    let index = (position as usize).min(RT_SEGMENTS - 1);
    let t = position - index as f32;
    let coefficients = curve.coefficients();
    coefficients[coefficient_index(index, 0)]
        .mul_add(t, coefficients[coefficient_index(index, 1)])
        .mul_add(t, coefficients[coefficient_index(index, 2)])
        .mul_add(t, coefficients[coefficient_index(index, 3)])
}

fn shipping_as_cubic(curve: WaveCurveRt) -> AdaptiveCubic {
    let coefficients = curve.coefficients();
    AdaptiveCubic {
        count: RT_SEGMENTS,
        boundaries: std::array::from_fn(|index| index as f32 / RT_SEGMENTS as f32),
        coefficients: std::array::from_fn(|segment| {
            std::array::from_fn(|coefficient| coefficients[coefficient_index(segment, coefficient)])
        }),
    }
}

fn uniform_c1(source: &SourceCurve) -> WaveCurveRt {
    let mut coefficients = [0.0; RT_SEGMENTS * 4];
    for segment in 0..RT_SEGMENTS {
        let start = segment as f32 / RT_SEGMENTS as f32;
        let end = (segment + 1) as f32 / RT_SEGMENTS as f32;
        for (coefficient, value) in cubic_coefficients(source, start, end)
            .into_iter()
            .enumerate()
        {
            coefficients[coefficient_index(segment, coefficient)] = value;
        }
    }
    WaveCurveRt::from_coefficients(coefficients)
}

fn fit_uniform_least_squares_c1(
    source: &SourceCurve,
    knots: &[WaveKnot],
    weight_curve: Option<WaveCurveRt>,
    jump_lambda: Option<f64>,
    curvature_lambda: Option<f64>,
) -> WaveCurveRt {
    const ENDPOINTS: usize = RT_SEGMENTS * 2;
    const FIT_SAMPLES: usize = 32;
    let mut parent = std::array::from_fn::<_, ENDPOINTS, _>(|index| index);
    let find = |parent: &[usize; ENDPOINTS], mut index: usize| {
        while parent[index] != index {
            index = parent[index];
        }
        index
    };
    for boundary in 1..RT_SEGMENTS {
        let phase = boundary as f32 / RT_SEGMENTS as f32;
        let hard = knots.iter().any(|knot| {
            (knot.phase - phase).abs() < 1.0e-6
                && (source_value_slope(source, phase, false).1
                    - source_value_slope(source, phase, true).1)
                    .abs()
                    > 1.0e-3
        });
        if jump_lambda.is_none() && !hard {
            let left = find(&parent, (boundary - 1) * 2 + 1);
            let right = find(&parent, boundary * 2);
            parent[right] = left;
        }
    }
    let wrap_hard =
        (source_value_slope(source, 0.0, false).1 - source_value_slope(source, 0.0, true).1).abs()
            > 1.0e-3;
    if jump_lambda.is_none() && !wrap_hard {
        let left = find(&parent, ENDPOINTS - 1);
        let right = find(&parent, 0);
        parent[right] = left;
    }
    let mut variable = [usize::MAX; ENDPOINTS];
    let mut variable_count = 0;
    for endpoint in 0..ENDPOINTS {
        let root = find(&parent, endpoint);
        if variable[root] == usize::MAX {
            variable[root] = variable_count;
            variable_count += 1;
        }
        variable[endpoint] = variable[root];
    }

    let weight_peak = weight_curve.map_or(1.0, |curve| {
        (0..512)
            .map(|sample| {
                let phase = sample as f32 / 512.0;
                (shipping_raw(curve, phase) - source.eval(f64::from(phase)) as f32).abs()
            })
            .fold(f32::EPSILON, f32::max)
    });
    let mut normal = [[0.0_f64; ENDPOINTS + 1]; ENDPOINTS];
    for segment in 0..RT_SEGMENTS {
        let start = segment as f64 / RT_SEGMENTS as f64;
        let width = 1.0 / RT_SEGMENTS as f64;
        let y0 = source.eval(start);
        let y1 = source.eval(start + width);
        let left = variable[segment * 2];
        let right = variable[segment * 2 + 1];
        for sample in 1..FIT_SAMPLES {
            let t = sample as f64 / FIT_SAMPLES as f64;
            let t2 = t * t;
            let t3 = t2 * t;
            let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
            let h01 = -2.0 * t3 + 3.0 * t2;
            let features = [width * (t3 - 2.0 * t2 + t), width * (t3 - t2)];
            let phase = width.mul_add(t, start);
            let target = source.eval(phase) - y0 * h00 - y1 * h01;
            let weight = weight_curve.map_or(1.0, |curve| {
                let error = f64::from(
                    (shipping_raw(curve, phase as f32) - source.eval(phase) as f32).abs()
                        / weight_peak,
                );
                1.0 + 16.0 * error * error
            });
            for (row, row_feature) in [(left, features[0]), (right, features[1])] {
                for (column, column_feature) in [(left, features[0]), (right, features[1])] {
                    normal[row][column] += weight * row_feature * column_feature;
                }
                normal[row][variable_count] += weight * row_feature * target;
            }
        }
    }
    if let Some(lambda) = jump_lambda {
        let mut penalize = |left: usize, right: usize| {
            normal[left][left] += lambda;
            normal[right][right] += lambda;
            normal[left][right] -= lambda;
            normal[right][left] -= lambda;
        };
        for boundary in 1..RT_SEGMENTS {
            let phase = boundary as f32 / RT_SEGMENTS as f32;
            let hard = knots.iter().any(|knot| {
                (knot.phase - phase).abs() < 1.0e-6
                    && (source_value_slope(source, phase, false).1
                        - source_value_slope(source, phase, true).1)
                        .abs()
                        > 1.0e-3
            });
            if !hard {
                penalize((boundary - 1) * 2 + 1, boundary * 2);
            }
        }
        if !wrap_hard {
            penalize(ENDPOINTS - 1, 0);
        }
    }
    if let Some(lambda) = curvature_lambda {
        let width = 1.0 / RT_SEGMENTS as f64;
        let mut penalize = |left_segment: usize, right_segment: usize| {
            let left_start = left_segment as f64 / RT_SEGMENTS as f64;
            let right_start = right_segment as f64 / RT_SEGMENTS as f64;
            let left_y0 = source.eval(left_start);
            let left_y1 = source.eval(left_start + width);
            let right_y0 = source.eval(right_start);
            let right_y1 = source.eval(right_start + width);
            let constant = -6.0 * right_y0 + 6.0 * right_y1 - 6.0 * left_y0 + 6.0 * left_y1;
            let features = [
                (variable[left_segment * 2], -2.0 * width),
                (variable[left_segment * 2 + 1], -4.0 * width),
                (variable[right_segment * 2], -4.0 * width),
                (variable[right_segment * 2 + 1], -2.0 * width),
            ];
            for (row, row_feature) in features {
                for (column, column_feature) in features {
                    normal[row][column] += lambda * row_feature * column_feature;
                }
                normal[row][variable_count] -= lambda * row_feature * constant;
            }
        };
        for boundary in 1..RT_SEGMENTS {
            let phase = boundary as f32 / RT_SEGMENTS as f32;
            let hard = knots.iter().any(|knot| {
                (knot.phase - phase).abs() < 1.0e-6
                    && (source_value_slope(source, phase, false).1
                        - source_value_slope(source, phase, true).1)
                        .abs()
                        > 1.0e-3
            });
            if !hard {
                penalize(boundary - 1, boundary);
            }
        }
        if !wrap_hard {
            penalize(RT_SEGMENTS - 1, 0);
        }
    }
    for pivot in 0..variable_count {
        let best = (pivot..variable_count)
            .max_by(|&left, &right| {
                normal[left][pivot]
                    .abs()
                    .total_cmp(&normal[right][pivot].abs())
            })
            .unwrap_or(pivot);
        normal.swap(pivot, best);
        let scale = normal[pivot][pivot];
        if scale.abs() <= f64::EPSILON {
            continue;
        }
        for column in pivot..=variable_count {
            normal[pivot][column] /= scale;
        }
        for row in 0..variable_count {
            if row == pivot {
                continue;
            }
            let scale = normal[row][pivot];
            for column in pivot..=variable_count {
                normal[row][column] -= scale * normal[pivot][column];
            }
        }
    }
    let slopes = std::array::from_fn::<_, ENDPOINTS, _>(|endpoint| {
        normal[variable[endpoint]][variable_count] as f32
    });
    let mut coefficients = [0.0; RT_SEGMENTS * 4];
    for segment in 0..RT_SEGMENTS {
        let start = segment as f32 / RT_SEGMENTS as f32;
        let end = (segment + 1) as f32 / RT_SEGMENTS as f32;
        let width = end - start;
        let y0 = source.eval(f64::from(start)) as f32;
        let y1 = source.eval(f64::from(end)) as f32;
        let m0 = slopes[segment * 2] * width;
        let m1 = slopes[segment * 2 + 1] * width;
        for (coefficient, value) in [
            2.0 * y0 - 2.0 * y1 + m0 + m1,
            -3.0 * y0 + 3.0 * y1 - 2.0 * m0 - m1,
            m0,
            y0,
        ]
        .into_iter()
        .enumerate()
        {
            coefficients[coefficient_index(segment, coefficient)] = value;
        }
    }
    WaveCurveRt::from_coefficients(coefficients)
}

fn uniform_least_squares_c1(source: &SourceCurve, knots: &[WaveKnot]) -> WaveCurveRt {
    fit_uniform_least_squares_c1(source, knots, None, None, None)
}

fn uniform_error_shaped_c1(source: &SourceCurve, knots: &[WaveKnot]) -> WaveCurveRt {
    let first = uniform_least_squares_c1(source, knots);
    let second = fit_uniform_least_squares_c1(source, knots, Some(first), None, None);
    fit_uniform_least_squares_c1(source, knots, Some(second), None, None)
}

fn uniform_jump_regularized(source: &SourceCurve, knots: &[WaveKnot], lambda: f64) -> WaveCurveRt {
    fit_uniform_least_squares_c1(source, knots, None, Some(lambda), None)
}

fn uniform_curvature_regularized(
    source: &SourceCurve,
    knots: &[WaveKnot],
    curvature_lambda: f64,
) -> WaveCurveRt {
    fit_uniform_least_squares_c1(source, knots, None, Some(1.0e-6), Some(curvature_lambda))
}

fn range_safe_uniform_c1(source: &SourceCurve, knots: &[WaveKnot]) -> WaveCurveRt {
    let curve = uniform_least_squares_c1(source, knots);
    let coefficients = curve.coefficients();
    let mut values = [[0.0_f32; 2]; RT_SEGMENTS];
    let mut slopes = [[0.0_f32; 2]; RT_SEGMENTS];
    for segment in 0..RT_SEGMENTS {
        let [a, b, c, d] =
            std::array::from_fn(|index| coefficients[coefficient_index(segment, index)]);
        values[segment] = [d, a + b + c + d];
        slopes[segment] = [c, 3.0 * a + 2.0 * b + c];
        slopes[segment][0] = slopes[segment][0].clamp(3.0 * (-1.0 - d), 3.0 * (1.0 - d));
        let y1 = values[segment][1];
        slopes[segment][1] = slopes[segment][1].clamp(3.0 * (y1 - 1.0), 3.0 * (y1 + 1.0));
    }
    for boundary in 1..RT_SEGMENTS {
        let phase = boundary as f32 / RT_SEGMENTS as f32;
        let hard = knots.iter().any(|knot| {
            (knot.phase - phase).abs() < 1.0e-6
                && (source_value_slope(source, phase, false).1
                    - source_value_slope(source, phase, true).1)
                    .abs()
                    > 1.0e-3
        });
        if !hard {
            let y = values[boundary][0];
            let slope = ((slopes[boundary - 1][1] + slopes[boundary][0]) * 0.5)
                .clamp(3.0 * (y - 1.0), 3.0 * (y + 1.0))
                .clamp(3.0 * (-1.0 - y), 3.0 * (1.0 - y));
            slopes[boundary - 1][1] = slope;
            slopes[boundary][0] = slope;
        }
    }
    if (source_value_slope(source, 0.0, false).1 - source_value_slope(source, 0.0, true).1).abs()
        <= 1.0e-3
    {
        let y = values[0][0];
        let slope = ((slopes[RT_SEGMENTS - 1][1] + slopes[0][0]) * 0.5)
            .clamp(3.0 * (y - 1.0), 3.0 * (y + 1.0))
            .clamp(3.0 * (-1.0 - y), 3.0 * (1.0 - y));
        slopes[RT_SEGMENTS - 1][1] = slope;
        slopes[0][0] = slope;
    }
    let mut safe = [0.0; RT_SEGMENTS * 4];
    for segment in 0..RT_SEGMENTS {
        let [y0, y1] = values[segment];
        let [m0, m1] = slopes[segment];
        for (coefficient, value) in [
            2.0 * y0 - 2.0 * y1 + m0 + m1,
            -3.0 * y0 + 3.0 * y1 - 2.0 * m0 - m1,
            m0,
            y0,
        ]
        .into_iter()
        .enumerate()
        {
            safe[coefficient_index(segment, coefficient)] = value;
        }
    }
    WaveCurveRt::from_coefficients(safe)
}

fn bezier_safe(curve: WaveCurveRt) -> bool {
    let coefficients = curve.coefficients();
    (0..RT_SEGMENTS).all(|segment| {
        let [a, b, c, d] =
            std::array::from_fn(|index| coefficients[coefficient_index(segment, index)]);
        [
            d,
            d + c / 3.0,
            a + b + c + d - (3.0 * a + 2.0 * b + c) / 3.0,
            a + b + c + d,
        ]
        .into_iter()
        .all(|control| (-1.0 - 2.0e-6..=1.0 + 2.0e-6).contains(&control))
    })
}

fn structural_eval4<const CLAMP: bool>(curve: WaveCurveRt, phase: f32x4) -> f32x4 {
    let coefficients = curve.coefficients();
    let mut index = f32x4::ZERO;
    let mut selected: [f32x4; 4] = std::array::from_fn(|coefficient| {
        f32x4::splat(coefficients[coefficient_index(0, coefficient)])
    });
    for segment in 1..RT_SEGMENTS {
        let mask = phase.cmp_gt(f32x4::splat(segment as f32 / RT_SEGMENTS as f32));
        index = mask.blend(f32x4::splat(segment as f32), index);
        for coefficient in 0..4 {
            selected[coefficient] = mask.blend(
                f32x4::splat(coefficients[coefficient_index(segment, coefficient)]),
                selected[coefficient],
            );
        }
    }
    let t = phase.mul_add(f32x4::splat(RT_SEGMENTS as f32), -index);
    let sample = selected[0]
        .mul_add(t, selected[1])
        .mul_add(t, selected[2])
        .mul_add(t, selected[3]);
    if CLAMP {
        sample.fast_max(-f32x4::ONE).fast_min(f32x4::ONE)
    } else {
        sample
    }
}

fn structural_eval8<const CLAMP: bool>(curve: WaveCurveRt, phase: f32x8) -> f32x8 {
    let phases: [f32; 8] = phase.into();
    f32x8::from(phases.map(|phase| {
        let sample = shipping_raw(curve, phase);
        if CLAMP {
            sample.clamp(-1.0, 1.0)
        } else {
            sample
        }
    }))
}

fn measure_eval_ns(mut evaluate: impl FnMut(usize) -> f32) -> f64 {
    const REPEATS: usize = 4_000_000;
    let started = Instant::now();
    let mut sum = 0.0;
    for index in 0..REPEATS {
        sum += black_box(evaluate(black_box(index)));
    }
    black_box(sum);
    started.elapsed().as_nanos() as f64 / REPEATS as f64
}

fn range_eval_ns(curve: WaveCurveRt) -> [f64; 6] {
    [
        measure_eval_ns(|index| curve.eval((index & 65_535) as f32 / 65_536.0)),
        measure_eval_ns(|index| shipping_raw(curve, (index & 65_535) as f32 / 65_536.0)),
        measure_eval_ns(|index| {
            let base = (index * 4) & 65_535;
            let phase = f32x4::from(std::array::from_fn(|lane| {
                ((base + lane) & 65_535) as f32 / 65_536.0
            }));
            let values: [f32; 4] = structural_eval4::<true>(curve, phase).into();
            values[0]
        }),
        measure_eval_ns(|index| {
            let base = (index * 4) & 65_535;
            let phase = f32x4::from(std::array::from_fn(|lane| {
                ((base + lane) & 65_535) as f32 / 65_536.0
            }));
            let values: [f32; 4] = structural_eval4::<false>(curve, phase).into();
            values[0]
        }),
        measure_eval_ns(|index| {
            let base = (index * 8) & 65_535;
            let phase = f32x8::from(std::array::from_fn(|lane| {
                ((base + lane) & 65_535) as f32 / 65_536.0
            }));
            let values: [f32; 8] = structural_eval8::<true>(curve, phase).into();
            values[0]
        }),
        measure_eval_ns(|index| {
            let base = (index * 8) & 65_535;
            let phase = f32x8::from(std::array::from_fn(|lane| {
                ((base + lane) & 65_535) as f32 / 65_536.0
            }));
            let values: [f32; 8] = structural_eval8::<false>(curve, phase).into();
            values[0]
        }),
    ]
}

fn curves() -> [(&'static str, WaveCurveData); 4] {
    [
        ("default", WaveCurveData::default()),
        (
            "drawn",
            WaveCurveData {
                knots: vec![
                    WaveKnot {
                        phase: 0.0,
                        value: -0.72,
                        curve: 0.8,
                        curve_x: -0.25,
                    },
                    WaveKnot {
                        phase: 0.13,
                        value: 0.91,
                        curve: -0.45,
                        curve_x: 0.3,
                    },
                    WaveKnot {
                        phase: 0.29,
                        value: 0.18,
                        curve: 1.2,
                        curve_x: -0.15,
                    },
                    WaveKnot {
                        phase: 0.52,
                        value: 0.63,
                        curve: 0.0,
                        curve_x: 0.0,
                    },
                    WaveKnot {
                        phase: 0.71,
                        value: -0.88,
                        curve: -0.7,
                        curve_x: 0.2,
                    },
                    WaveKnot {
                        phase: 0.89,
                        value: -0.24,
                        curve: 0.35,
                        curve_x: -0.1,
                    },
                ],
            },
        ),
        (
            "tight",
            WaveCurveData {
                knots: vec![
                    WaveKnot {
                        phase: 0.0,
                        value: -1.0,
                        ..WaveKnot::default()
                    },
                    WaveKnot {
                        phase: 0.496,
                        value: -0.85,
                        curve: 1.5,
                        curve_x: -0.5,
                    },
                    WaveKnot {
                        phase: 0.5,
                        value: 0.95,
                        curve: -1.0,
                        curve_x: 0.4,
                    },
                    WaveKnot {
                        phase: 0.82,
                        value: 0.1,
                        ..WaveKnot::default()
                    },
                ],
            },
        ),
        (
            "max_knots",
            WaveCurveData {
                knots: (0..MAX_WAVE_KNOTS)
                    .map(|index| WaveKnot {
                        phase: index as f32 / MAX_WAVE_KNOTS as f32,
                        value: if index % 2 == 0 { -0.92 } else { 0.92 },
                        curve: if index % 3 == 0 { 1.0 } else { -0.6 },
                        curve_x: if index % 2 == 0 { -0.4 } else { 0.3 },
                    })
                    .collect(),
            },
        ),
    ]
}

fn legacy_curve(data: &WaveCurveData) -> WaveCurveRt {
    let knots = sanitize_knots(&data.knots);
    let source = SourceCurve::compile(&knots);
    if knots.len() <= 3 || has_tight_transition(&knots) {
        WaveCurveRt::from_sampled_source(&source)
    } else {
        WaveCurveRt::from_source(&source)
    }
}

fn pre_regularized_curve(data: &WaveCurveData) -> WaveCurveRt {
    let knots = sanitize_knots(&data.knots);
    let source = SourceCurve::compile(&knots);
    let legacy = legacy_curve(data);
    let shared = WaveCurveRt::from_shared_slope_source(&source, &knots);
    if shared.proves_better_than(legacy, &source, &knots) {
        shared
    } else {
        legacy
    }
}

fn direct_metrics(source: &SourceCurve, evaluator: impl Fn(f32) -> f32) -> (f64, f32) {
    direct_metrics_grid(source, evaluator, GRID)
}

fn direct_metrics_grid(
    source: &SourceCurve,
    evaluator: impl Fn(f32) -> f32,
    grid: usize,
) -> (f64, f32) {
    let mut squared = 0.0;
    let mut peak = 0.0_f32;
    for index in 0..grid {
        let phase = index as f32 / grid as f32;
        let error = evaluator(phase) - source.eval(f64::from(phase)) as f32;
        squared += f64::from(error * error);
        peak = peak.max(error.abs());
    }
    ((squared / grid as f64).sqrt(), peak)
}

fn knot_peak(knots: &[WaveKnot], evaluator: impl Fn(f32) -> f32) -> f32 {
    knots
        .iter()
        .map(|knot| (evaluator(knot.phase) - knot.value).abs())
        .fold(0.0, f32::max)
}

fn derivative_metrics(curve: AdaptiveCubic, hard: &[f32]) -> (f32, f32, f32) {
    let mut smooth_slope = 0.0_f32;
    let mut hard_slope = 0.0_f32;
    for index in 1..curve.count {
        let left = curve.coefficients[index - 1];
        let right = curve.coefficients[index];
        let left_width = curve.boundaries[index] - curve.boundaries[index - 1];
        let right_width = curve.boundaries[index + 1] - curve.boundaries[index];
        let left_slope = (3.0 * left[0] + 2.0 * left[1] + left[2]) / left_width;
        let right_slope = right[2] / right_width;
        let jump = (right_slope - left_slope).abs();
        if hard
            .iter()
            .any(|phase| (*phase - curve.boundaries[index]).abs() < 1.0e-6)
        {
            hard_slope = hard_slope.max(jump);
        } else {
            smooth_slope = smooth_slope.max(jump);
        }
    }
    let left = curve.coefficients[curve.count - 1];
    let right = curve.coefficients[0];
    let wrap = ((right[2] / (curve.boundaries[1] - curve.boundaries[0]))
        - (3.0 * left[0] + 2.0 * left[1] + left[2])
            / (curve.boundaries[curve.count] - curve.boundaries[curve.count - 1]))
        .abs();
    (smooth_slope, hard_slope, wrap)
}

fn smooth_derivative_energy(curve: AdaptiveCubic, hard: &[f32]) -> f64 {
    let mut energy = 0.0;
    for index in 1..curve.count {
        if hard
            .iter()
            .any(|phase| (*phase - curve.boundaries[index]).abs() < 1.0e-6)
        {
            continue;
        }
        let left = curve.coefficients[index - 1];
        let right = curve.coefficients[index];
        let left_width = curve.boundaries[index] - curve.boundaries[index - 1];
        let right_width = curve.boundaries[index + 1] - curve.boundaries[index];
        let jump = right[2] / right_width - (3.0 * left[0] + 2.0 * left[1] + left[2]) / left_width;
        energy += f64::from(jump * jump);
    }
    energy
}

fn smooth_curvature_event_energy(curve: AdaptiveCubic, hard: &[f32]) -> f64 {
    let mut energy = 0.0;
    for index in 1..curve.count {
        if hard
            .iter()
            .any(|phase| (*phase - curve.boundaries[index]).abs() < 1.0e-6)
        {
            continue;
        }
        let left = curve.coefficients[index - 1];
        let right = curve.coefficients[index];
        let left_width = curve.boundaries[index] - curve.boundaries[index - 1];
        let right_width = curve.boundaries[index + 1] - curve.boundaries[index];
        let jump = 2.0 * right[1] / (right_width * right_width)
            - (6.0 * left[0] + 2.0 * left[1]) / (left_width * left_width);
        energy += f64::from(jump * jump);
    }
    if !hard.iter().any(|phase| phase.abs() < 1.0e-6) {
        let left = curve.coefficients[curve.count - 1];
        let right = curve.coefficients[0];
        let left_width = curve.boundaries[curve.count] - curve.boundaries[curve.count - 1];
        let right_width = curve.boundaries[1] - curve.boundaries[0];
        let jump = 2.0 * right[1] / (right_width * right_width)
            - (6.0 * left[0] + 2.0 * left[1]) / (left_width * left_width);
        energy += f64::from(jump * jump);
    }
    energy
}

fn spectrum(evaluator: impl Fn(f32) -> f32) -> Vec<Complex> {
    spectrum_grid(evaluator, GRID)
}

fn spectrum_grid(evaluator: impl Fn(f32) -> f32, grid: usize) -> Vec<Complex> {
    let mut values = (0..grid)
        .map(|index| Complex::new(f64::from(evaluator(index as f32 / grid as f32)), 0.0))
        .collect::<Vec<_>>();
    fft(&mut values, false);
    for value in &mut values {
        *value /= grid as f64;
    }
    values
}

fn bandlimited_error(reference: &[Complex], candidate: &[Complex], period: usize) -> f64 {
    let harmonics = (period - 1) / 2;
    let mut signal = reference[0].norm_sqr();
    let mut error = (candidate[0] - reference[0]).norm_sqr();
    for harmonic in 1..=harmonics {
        signal += 2.0 * reference[harmonic].norm_sqr();
        error += 2.0 * (candidate[harmonic] - reference[harmonic]).norm_sqr();
    }
    10.0 * (error.max(f64::MIN_POSITIVE) / signal.max(f64::MIN_POSITIVE)).log10()
}

fn compile_ns(data: &WaveCurveData, adaptive: bool) -> f64 {
    const REPEATS: usize = 20_000;
    let started = Instant::now();
    for _ in 0..REPEATS {
        if adaptive {
            black_box(AdaptiveCubic::compile(black_box(data)));
        } else {
            black_box(legacy_curve(black_box(data)));
        }
    }
    started.elapsed().as_nanos() as f64 / REPEATS as f64
}

fn eval_ns(shipping: WaveCurveRt, adaptive: AdaptiveCubic) -> (f64, f64) {
    const REPEATS: usize = 8_000_000;
    let started = Instant::now();
    let mut sum = 0.0;
    for index in 0..REPEATS {
        sum += black_box(shipping.eval(black_box((index & 65_535) as f32 / 65_536.0)));
    }
    let shipping_ns = started.elapsed().as_nanos() as f64 / REPEATS as f64;
    let started = Instant::now();
    for index in 0..REPEATS {
        sum += black_box(adaptive.eval_raw(black_box((index & 65_535) as f32 / 65_536.0)));
    }
    let adaptive_ns = started.elapsed().as_nanos() as f64 / REPEATS as f64;
    black_box(sum);
    (shipping_ns, adaptive_ns)
}

fn compile_uniform_ns(data: &WaveCurveData, candidate: bool) -> f64 {
    const REPEATS: usize = 20_000;
    let knots = sanitize_knots(&data.knots);
    let source = SourceCurve::compile(&knots);
    let started = Instant::now();
    for _ in 0..REPEATS {
        if candidate {
            black_box(uniform_c1(black_box(&source)));
        } else {
            black_box(legacy_curve(black_box(data)));
        }
    }
    started.elapsed().as_nanos() as f64 / REPEATS as f64
}

fn compile_least_squares_ns(data: &WaveCurveData) -> f64 {
    const REPEATS: usize = 2_000;
    let knots = sanitize_knots(&data.knots);
    let source = SourceCurve::compile(&knots);
    let started = Instant::now();
    for _ in 0..REPEATS {
        black_box(uniform_least_squares_c1(
            black_box(&source),
            black_box(&knots),
        ));
    }
    started.elapsed().as_nanos() as f64 / REPEATS as f64
}

fn compile_error_shaped_ns(data: &WaveCurveData) -> f64 {
    const REPEATS: usize = 20;
    let knots = sanitize_knots(&data.knots);
    let source = SourceCurve::compile(&knots);
    let started = Instant::now();
    for _ in 0..REPEATS {
        black_box(uniform_error_shaped_c1(
            black_box(&source),
            black_box(&knots),
        ));
    }
    started.elapsed().as_nanos() as f64 / REPEATS as f64
}

#[test]
#[ignore = "manual release-mode uniform compiler experiment"]
fn uniform_c1_compiler_report() {
    println!(
        "contract,segments=16,shipping_bytes={},candidate_bytes={},grid={GRID},candidate=uniform-cubic-Hermite-shared-source-slopes",
        size_of::<WaveCurveRt>(),
        size_of::<WaveCurveRt>(),
    );
    for (name, data) in curves() {
        let knots = sanitize_knots(&data.knots);
        let source = SourceCurve::compile(&knots);
        let shipping = legacy_curve(&data);
        let candidate = uniform_c1(&source);
        let shipping_cubic = shipping_as_cubic(shipping);
        let candidate_cubic = shipping_as_cubic(candidate);
        let (shipping_rms, shipping_peak) =
            direct_metrics(&source, |phase| shipping_raw(shipping, phase));
        let (candidate_rms, candidate_peak) =
            direct_metrics(&source, |phase| shipping_raw(candidate, phase));
        let shipping_knot_peak = knot_peak(&knots, |phase| shipping_raw(shipping, phase));
        let candidate_knot_peak = knot_peak(&knots, |phase| shipping_raw(candidate, phase));
        let (_, _, shipping_crossings) = shipping_cubic.extrema();
        let (_, _, candidate_crossings) = candidate_cubic.extrema();
        let hard = knots.iter().map(|knot| knot.phase).collect::<Vec<_>>();
        let (shipping_smooth_jump, shipping_hard_jump, shipping_wrap_jump) =
            derivative_metrics(shipping_cubic, &hard);
        let (candidate_smooth_jump, candidate_hard_jump, candidate_wrap_jump) =
            derivative_metrics(candidate_cubic, &hard);
        let reference_spectrum = spectrum(|phase| source.eval(f64::from(phase)) as f32);
        let shipping_spectrum = spectrum(|phase| shipping_raw(shipping, phase));
        let candidate_spectrum = spectrum(|phase| shipping_raw(candidate, phase));
        let eligible = candidate_rms <= shipping_rms + 1.0e-7
            && candidate_peak <= shipping_peak + 1.0e-6
            && candidate_knot_peak <= shipping_knot_peak + 1.0e-6
            && candidate_crossings <= shipping_crossings
            && candidate_smooth_jump <= shipping_smooth_jump + 1.0e-3;
        println!(
            "uniform,name={name},selected={},shipping_rms={shipping_rms:.9},candidate_rms={candidate_rms:.9},shipping_peak={shipping_peak:.9},candidate_peak={candidate_peak:.9},shipping_knot_peak={shipping_knot_peak:.9},candidate_knot_peak={candidate_knot_peak:.9},shipping_smooth_jump={shipping_smooth_jump:.6},candidate_smooth_jump={candidate_smooth_jump:.6},shipping_hard_jump={shipping_hard_jump:.6},candidate_hard_jump={candidate_hard_jump:.6},shipping_wrap_jump={shipping_wrap_jump:.6},candidate_wrap_jump={candidate_wrap_jump:.6},shipping_crossings={shipping_crossings},candidate_crossings={candidate_crossings},shipping_compile_ns={:.1},candidate_compile_ns={:.1}",
            if eligible { "c1" } else { "legacy" },
            compile_uniform_ns(&data, false),
            compile_uniform_ns(&data, true),
        );
        for period in [436, 55, 7] {
            println!(
                "uniform_bandlimited,name={name},period={period},shipping_error_db={:.3},candidate_error_db={:.3}",
                bandlimited_error(&reference_spectrum, &shipping_spectrum, period),
                bandlimited_error(&reference_spectrum, &candidate_spectrum, period),
            );
        }
    }
}

#[test]
#[ignore = "manual release-mode uniform compiler experiment"]
fn uniform_least_squares_c1_compiler_report() {
    println!(
        "contract,segments=16,shipping_bytes={},candidate_bytes={},grid={GRID},candidate=uniform-constrained-least-squares-cubic-Hermite-C1",
        size_of::<WaveCurveRt>(),
        size_of::<WaveCurveRt>(),
    );
    for (name, data) in curves() {
        let knots = sanitize_knots(&data.knots);
        let source = SourceCurve::compile(&knots);
        let shipping = legacy_curve(&data);
        let candidate = uniform_least_squares_c1(&source, &knots);
        let shipping_cubic = shipping_as_cubic(shipping);
        let candidate_cubic = shipping_as_cubic(candidate);
        let (shipping_rms, shipping_peak) =
            direct_metrics(&source, |phase| shipping_raw(shipping, phase));
        let (candidate_rms, candidate_peak) =
            direct_metrics(&source, |phase| shipping_raw(candidate, phase));
        let shipping_knot_peak = knot_peak(&knots, |phase| shipping_raw(shipping, phase));
        let candidate_knot_peak = knot_peak(&knots, |phase| shipping_raw(candidate, phase));
        let (_, _, shipping_crossings) = shipping_cubic.extrema();
        let (_, _, candidate_crossings) = candidate_cubic.extrema();
        let hard = knots.iter().map(|knot| knot.phase).collect::<Vec<_>>();
        let (shipping_smooth_jump, shipping_hard_jump, shipping_wrap_jump) =
            derivative_metrics(shipping_cubic, &hard);
        let (candidate_smooth_jump, candidate_hard_jump, candidate_wrap_jump) =
            derivative_metrics(candidate_cubic, &hard);
        let reference_spectrum = spectrum(|phase| source.eval(f64::from(phase)) as f32);
        let shipping_spectrum = spectrum(|phase| shipping_raw(shipping, phase));
        let candidate_spectrum = spectrum(|phase| shipping_raw(candidate, phase));
        let eligible = candidate_rms <= shipping_rms + 1.0e-7
            && candidate_peak <= shipping_peak + 1.0e-6
            && candidate_knot_peak <= shipping_knot_peak + 1.0e-6
            && candidate_crossings <= shipping_crossings
            && candidate_smooth_jump <= shipping_smooth_jump + 1.0e-3;
        println!(
            "least_squares,name={name},selected={},shipping_rms={shipping_rms:.9},candidate_rms={candidate_rms:.9},shipping_peak={shipping_peak:.9},candidate_peak={candidate_peak:.9},shipping_knot_peak={shipping_knot_peak:.9},candidate_knot_peak={candidate_knot_peak:.9},shipping_smooth_jump={shipping_smooth_jump:.6},candidate_smooth_jump={candidate_smooth_jump:.6},shipping_hard_jump={shipping_hard_jump:.6},candidate_hard_jump={candidate_hard_jump:.6},shipping_wrap_jump={shipping_wrap_jump:.6},candidate_wrap_jump={candidate_wrap_jump:.6},shipping_crossings={shipping_crossings},candidate_crossings={candidate_crossings},shipping_compile_ns={:.1},candidate_compile_ns={:.1}",
            if eligible {
                "least_squares_c1"
            } else {
                "legacy"
            },
            compile_uniform_ns(&data, false),
            compile_least_squares_ns(&data),
        );
        for period in [436, 55, 7] {
            println!(
                "least_squares_bandlimited,name={name},period={period},shipping_error_db={:.3},candidate_error_db={:.3}",
                bandlimited_error(&reference_spectrum, &shipping_spectrum, period),
                bandlimited_error(&reference_spectrum, &candidate_spectrum, period),
            );
        }
    }
}

const CORPUS_GRID: usize = 8_192;
const CORPUS_CASES_PER_CATEGORY: usize = 64;
const CORPUS_CATEGORIES: [&str; 8] = [
    "smooth",
    "hard",
    "clustered",
    "near_duplicate",
    "extrema",
    "wrap",
    "max_knots",
    "random",
];

fn random_unit(state: &mut u64) -> f32 {
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    (*state >> 40) as f32 / (1_u32 << 24) as f32
}

fn corpus_curve(category: usize, case: usize, state: &mut u64) -> WaveCurveData {
    let bipolar = |state: &mut u64| random_unit(state).mul_add(2.0, -1.0);
    let knots = match category {
        0 => {
            let count = 4 + case % 5;
            (0..count)
                .map(|index| {
                    let phase = index as f32 / count as f32;
                    let angle = std::f32::consts::TAU * phase;
                    WaveKnot {
                        phase,
                        value: (angle.sin() * 0.72 + (2.0 * angle).sin() * 0.18).clamp(-1.0, 1.0),
                        curve: bipolar(state) * 0.7,
                        curve_x: bipolar(state) * 0.35,
                    }
                })
                .collect()
        }
        1 => {
            let count = [4, 8, 16][case % 3];
            (0..count)
                .map(|index| WaveKnot {
                    phase: index as f32 / count as f32,
                    value: if index % 2 == 0 { -0.9 } else { 0.9 },
                    curve: if index % 3 == 0 { 1.4 } else { -0.9 },
                    curve_x: bipolar(state) * 0.6,
                })
                .collect()
        }
        2 => {
            let center = 0.15 + random_unit(state) * 0.7;
            vec![
                WaveKnot {
                    phase: 0.0,
                    value: bipolar(state),
                    ..WaveKnot::default()
                },
                WaveKnot {
                    phase: (center - 0.008).max(0.01),
                    value: -0.95,
                    curve: 1.8,
                    curve_x: -0.7,
                },
                WaveKnot {
                    phase: (center - 0.004).max(0.014),
                    value: 0.92,
                    curve: -1.3,
                    curve_x: 0.6,
                },
                WaveKnot {
                    phase: center,
                    value: -0.82,
                    curve: 2.2,
                    curve_x: -0.5,
                },
                WaveKnot {
                    phase: (center + 0.006).min(0.96),
                    value: 0.88,
                    curve: -1.7,
                    curve_x: 0.4,
                },
                WaveKnot {
                    phase: 0.98,
                    value: bipolar(state),
                    ..WaveKnot::default()
                },
            ]
        }
        3 => {
            let phase = 0.2 + random_unit(state) * 0.5;
            vec![
                WaveKnot {
                    phase: 0.0,
                    value: bipolar(state),
                    ..WaveKnot::default()
                },
                WaveKnot {
                    phase,
                    value: -0.8,
                    curve: 0.8,
                    curve_x: -0.2,
                },
                WaveKnot {
                    phase: phase + 0.001,
                    value: 0.9,
                    curve: -0.8,
                    curve_x: 0.2,
                },
                WaveKnot {
                    phase: phase + 0.002,
                    value: -0.7,
                    ..WaveKnot::default()
                },
                WaveKnot {
                    phase: 0.85,
                    value: bipolar(state),
                    ..WaveKnot::default()
                },
            ]
        }
        4 => {
            let count = 3 + case % 6;
            (0..count)
                .map(|index| WaveKnot {
                    phase: index as f32 / count as f32,
                    value: if index % 2 == 0 { -0.99 } else { 0.99 },
                    curve: if index % 2 == 0 { 4.0 } else { -4.0 },
                    curve_x: if index % 3 == 0 { -1.0 } else { 1.0 },
                })
                .collect()
        }
        5 => vec![
            WaveKnot {
                phase: 0.0,
                value: if case % 2 == 0 { -0.98 } else { 0.98 },
                curve: bipolar(state) * 2.0,
                curve_x: bipolar(state),
            },
            WaveKnot {
                phase: 0.25,
                value: bipolar(state),
                curve: bipolar(state) * 2.0,
                curve_x: bipolar(state),
            },
            WaveKnot {
                phase: 0.61,
                value: bipolar(state),
                curve: bipolar(state) * 2.0,
                curve_x: bipolar(state),
            },
            WaveKnot {
                phase: 0.996,
                value: if case % 2 == 0 { 0.98 } else { -0.98 },
                curve: bipolar(state) * 2.0,
                curve_x: bipolar(state),
            },
        ],
        6 => (0..MAX_WAVE_KNOTS)
            .map(|index| WaveKnot {
                phase: index as f32 / MAX_WAVE_KNOTS as f32,
                value: bipolar(state) * 0.96,
                curve: bipolar(state) * 2.5,
                curve_x: bipolar(state) * 0.9,
            })
            .collect(),
        _ => {
            let count = 2 + case % 15;
            (0..count)
                .map(|index| {
                    let base = index as f32 / count as f32;
                    let jitter = if index == 0 {
                        0.0
                    } else {
                        bipolar(state) * 0.35 / count as f32
                    };
                    WaveKnot {
                        phase: (base + jitter).clamp(0.0, 0.995),
                        value: bipolar(state),
                        curve: bipolar(state) * 4.0,
                        curve_x: bipolar(state),
                    }
                })
                .collect()
        }
    };
    WaveCurveData { knots }
}

fn overshoot(extrema: (f32, f32, usize)) -> f32 {
    (-1.0 - extrema.0).max(extrema.1 - 1.0).max(0.0)
}

fn every_knot_no_worse(knots: &[WaveKnot], shipping: WaveCurveRt, candidate: WaveCurveRt) -> bool {
    knots.iter().all(|knot| {
        (shipping_raw(candidate, knot.phase) - knot.value).abs()
            <= (shipping_raw(shipping, knot.phase) - knot.value).abs() + 1.0e-6
    })
}

#[test]
#[ignore = "manual release-mode seeded compiler property corpus"]
fn seeded_uniform_least_squares_c1_property_report() {
    let mut state = 0x4b55_5256_c101_2026;
    let mut selected_by_category = [0_usize; CORPUS_CATEGORIES.len()];
    let mut improvements = Vec::new();
    let mut bandlimited_deltas = Vec::new();
    let mut candidate_compile_ns = Vec::new();
    let mut legacy_compile_ns = Vec::new();
    let mut evaluated = 0;

    for category in 0..CORPUS_CATEGORIES.len() {
        for case in 0..CORPUS_CASES_PER_CATEGORY {
            let data = corpus_curve(category, case, &mut state);
            let knots = sanitize_knots(&data.knots);
            let source = SourceCurve::compile(&knots);
            let shipping = legacy_curve(&data);
            let candidate = uniform_least_squares_c1(&source, &knots);
            assert_eq!(
                candidate,
                WaveCurveRt::from_shared_slope_source(&source, &knots)
            );
            let shipping_cubic = shipping_as_cubic(shipping);
            let candidate_cubic = shipping_as_cubic(candidate);
            let (shipping_rms, shipping_peak) =
                direct_metrics_grid(&source, |phase| shipping_raw(shipping, phase), CORPUS_GRID);
            let (candidate_rms, candidate_peak) =
                direct_metrics_grid(&source, |phase| shipping_raw(candidate, phase), CORPUS_GRID);
            let shipping_extrema = shipping_cubic.extrema();
            let candidate_extrema = candidate_cubic.extrema();
            let hard = knots.iter().map(|knot| knot.phase).collect::<Vec<_>>();
            let (shipping_smooth, shipping_hard, shipping_wrap) =
                derivative_metrics(shipping_cubic, &hard);
            let (candidate_smooth, candidate_hard, candidate_wrap) =
                derivative_metrics(candidate_cubic, &hard);
            let reference_spectrum =
                spectrum_grid(|phase| source.eval(f64::from(phase)) as f32, CORPUS_GRID);
            let shipping_spectrum =
                spectrum_grid(|phase| shipping_raw(shipping, phase), CORPUS_GRID);
            let candidate_spectrum =
                spectrum_grid(|phase| shipping_raw(candidate, phase), CORPUS_GRID);
            let deltas = [436, 55, 7].map(|period| {
                bandlimited_error(&reference_spectrum, &candidate_spectrum, period)
                    - bandlimited_error(&reference_spectrum, &shipping_spectrum, period)
            });
            let source_gate =
                candidate_rms <= shipping_rms * 0.999 && candidate_peak <= shipping_peak + 1.0e-6;
            let knot_gate = every_knot_no_worse(&knots, shipping, candidate);
            let range_gate = candidate_extrema.2 <= shipping_extrema.2
                && overshoot(candidate_extrema) <= overshoot(shipping_extrema) + 1.0e-6;
            let derivative_gate = candidate_smooth <= shipping_smooth + 1.0e-3
                && (shipping_hard <= 1.0e-3 || candidate_hard >= shipping_hard * 0.95)
                && (shipping_wrap <= 1.0e-3 || candidate_wrap >= shipping_wrap * 0.95);
            let bandlimited_gate = deltas.iter().all(|delta| *delta <= 0.1);
            if source_gate && knot_gate && range_gate && derivative_gate && bandlimited_gate {
                selected_by_category[category] += 1;
                improvements.push((1.0 - candidate_rms / shipping_rms) * 100.0);
                bandlimited_deltas.extend(deltas);
            }
            legacy_compile_ns.push(compile_uniform_ns(&data, false));
            candidate_compile_ns.push(compile_least_squares_ns(&data));
            evaluated += 1;
        }
    }

    improvements.sort_by(f64::total_cmp);
    bandlimited_deltas.sort_by(f64::total_cmp);
    legacy_compile_ns.sort_by(f64::total_cmp);
    candidate_compile_ns.sort_by(f64::total_cmp);
    let selected = improvements.len();
    println!(
        "corpus,seed=0x4b555256c1012026,cases={evaluated},grid={CORPUS_GRID},selected={selected},selection_percent={:.3},shipping_bytes={},candidate_bytes={},legacy_compile_median_ns={:.1},candidate_compile_median_ns={:.1}",
        selected as f64 / evaluated as f64 * 100.0,
        size_of::<WaveCurveRt>(),
        size_of::<WaveCurveRt>(),
        legacy_compile_ns[evaluated / 2],
        candidate_compile_ns[evaluated / 2],
    );
    for (category, selected) in CORPUS_CATEGORIES.iter().zip(selected_by_category) {
        println!(
            "corpus_category,name={category},cases={CORPUS_CASES_PER_CATEGORY},selected={selected}"
        );
    }
    if selected > 0 {
        println!(
            "corpus_benefit,rms_reduction_percent_min={:.6},median={:.6},max={:.6},bandlimited_delta_db_min={:.6},median={:.6},max={:.6}",
            improvements[0],
            improvements[selected / 2],
            improvements[selected - 1],
            bandlimited_deltas[0],
            bandlimited_deltas[bandlimited_deltas.len() / 2],
            bandlimited_deltas[bandlimited_deltas.len() - 1],
        );
    }
    assert_eq!(
        evaluated,
        CORPUS_CASES_PER_CATEGORY * CORPUS_CATEGORIES.len()
    );
    assert_eq!(size_of::<WaveCurveRt>(), 256);
}

#[test]
#[ignore = "manual release-mode cheap selector sweep"]
fn cheap_uniform_least_squares_c1_selector_sweep() {
    const CHEAP_GRID: usize = 256;
    const RMS_REDUCTIONS: [f64; 6] = [0.001, 0.01, 0.05, 0.10, 0.25, 0.50];
    const PEAK_RATIOS: [f32; 4] = [1.0, 0.9, 0.75, 0.5];
    let mut state = 0x4b55_5256_c101_2026;
    let mut selected = [[0_usize; PEAK_RATIOS.len()]; RMS_REDUCTIONS.len()];
    let mut regressions = [[0_usize; PEAK_RATIOS.len()]; RMS_REDUCTIONS.len()];
    let mut full_source_regressions = [[0_usize; PEAK_RATIOS.len()]; RMS_REDUCTIONS.len()];
    let mut chosen_by_category = [0_usize; CORPUS_CATEGORIES.len()];
    let mut chosen_rms_reductions = Vec::new();
    let mut chosen_bl_deltas = Vec::new();

    for category in 0..CORPUS_CATEGORIES.len() {
        for case in 0..CORPUS_CASES_PER_CATEGORY {
            let data = corpus_curve(category, case, &mut state);
            let knots = sanitize_knots(&data.knots);
            let source = SourceCurve::compile(&knots);
            let shipping = legacy_curve(&data);
            let candidate = uniform_least_squares_c1(&source, &knots);
            let shipping_cubic = shipping_as_cubic(shipping);
            let candidate_cubic = shipping_as_cubic(candidate);
            let (cheap_shipping_rms, cheap_shipping_peak) =
                direct_metrics_grid(&source, |phase| shipping_raw(shipping, phase), CHEAP_GRID);
            let (cheap_candidate_rms, cheap_candidate_peak) =
                direct_metrics_grid(&source, |phase| shipping_raw(candidate, phase), CHEAP_GRID);
            let (full_shipping_rms, full_shipping_peak) =
                direct_metrics_grid(&source, |phase| shipping_raw(shipping, phase), CORPUS_GRID);
            let (full_candidate_rms, full_candidate_peak) =
                direct_metrics_grid(&source, |phase| shipping_raw(candidate, phase), CORPUS_GRID);
            let shipping_extrema = shipping_cubic.extrema();
            let candidate_extrema = candidate_cubic.extrema();
            let hard = knots.iter().map(|knot| knot.phase).collect::<Vec<_>>();
            let (shipping_smooth, shipping_hard, shipping_wrap) =
                derivative_metrics(shipping_cubic, &hard);
            let (candidate_smooth, candidate_hard, candidate_wrap) =
                derivative_metrics(candidate_cubic, &hard);
            let guards = every_knot_no_worse(&knots, shipping, candidate)
                && candidate_extrema.2 <= shipping_extrema.2
                && overshoot(candidate_extrema) <= overshoot(shipping_extrema) + 1.0e-6
                && candidate_smooth <= shipping_smooth + 1.0e-3
                && (shipping_hard <= 1.0e-3 || candidate_hard >= shipping_hard * 0.95)
                && (shipping_wrap <= 1.0e-3 || candidate_wrap >= shipping_wrap * 0.95);
            if !guards {
                assert_eq!(pre_regularized_curve(&data), shipping);
                continue;
            }
            let production_selects = cheap_candidate_rms <= cheap_shipping_rms * 0.75
                && cheap_candidate_peak <= cheap_shipping_peak + 1.0e-7;
            assert_eq!(
                pre_regularized_curve(&data),
                if production_selects {
                    candidate
                } else {
                    shipping
                }
            );
            let reference_spectrum =
                spectrum_grid(|phase| source.eval(f64::from(phase)) as f32, CORPUS_GRID);
            let shipping_spectrum =
                spectrum_grid(|phase| shipping_raw(shipping, phase), CORPUS_GRID);
            let candidate_spectrum =
                spectrum_grid(|phase| shipping_raw(candidate, phase), CORPUS_GRID);
            let worst_bl_delta = [436, 55, 7]
                .map(|period| {
                    bandlimited_error(&reference_spectrum, &candidate_spectrum, period)
                        - bandlimited_error(&reference_spectrum, &shipping_spectrum, period)
                })
                .into_iter()
                .fold(f64::NEG_INFINITY, f64::max);
            for (rms_index, reduction) in RMS_REDUCTIONS.into_iter().enumerate() {
                for (peak_index, peak_ratio) in PEAK_RATIOS.into_iter().enumerate() {
                    if cheap_candidate_rms <= cheap_shipping_rms * (1.0 - reduction)
                        && cheap_candidate_peak <= cheap_shipping_peak * peak_ratio + 1.0e-7
                    {
                        selected[rms_index][peak_index] += 1;
                        regressions[rms_index][peak_index] += usize::from(worst_bl_delta > 0.0);
                        full_source_regressions[rms_index][peak_index] += usize::from(
                            full_candidate_rms > full_shipping_rms
                                || full_candidate_peak > full_shipping_peak + 1.0e-6,
                        );
                        if reduction == 0.25 && peak_ratio == 1.0 {
                            chosen_by_category[category] += 1;
                            chosen_rms_reductions
                                .push((1.0 - full_candidate_rms / full_shipping_rms) * 100.0);
                            chosen_bl_deltas.push(worst_bl_delta);
                        }
                    }
                }
            }
        }
    }

    for (rms_index, reduction) in RMS_REDUCTIONS.into_iter().enumerate() {
        for (peak_index, peak_ratio) in PEAK_RATIOS.into_iter().enumerate() {
            println!(
                "selector_sweep,grid={CHEAP_GRID},rms_reduction={reduction:.3},peak_ratio={peak_ratio:.2},selected={},full_source_regressions={},bl_regressions={}",
                selected[rms_index][peak_index],
                full_source_regressions[rms_index][peak_index],
                regressions[rms_index][peak_index],
            );
        }
    }
    chosen_rms_reductions.sort_by(f64::total_cmp);
    chosen_bl_deltas.sort_by(f64::total_cmp);
    for (category, count) in CORPUS_CATEGORIES.iter().zip(chosen_by_category) {
        println!("cheap_selector_category,name={category},selected={count}");
    }
    println!(
        "cheap_selector_benefit,rms_reduction_percent_min={:.6},median={:.6},max={:.6},worst_bl_delta_db_min={:.6},median={:.6},max={:.6}",
        chosen_rms_reductions[0],
        chosen_rms_reductions[chosen_rms_reductions.len() / 2],
        chosen_rms_reductions[chosen_rms_reductions.len() - 1],
        chosen_bl_deltas[0],
        chosen_bl_deltas[chosen_bl_deltas.len() / 2],
        chosen_bl_deltas[chosen_bl_deltas.len() - 1],
    );
}

#[test]
#[ignore = "manual release-mode residual-weighted compiler experiment"]
fn residual_weighted_uniform_c1_selector_sweep() {
    const CHEAP_GRID: usize = 256;
    const RMS_REDUCTIONS: [f64; 5] = [0.001, 0.01, 0.05, 0.10, 0.25];
    let mut state = 0x4b55_5256_c101_2026;
    let mut selected = [0_usize; RMS_REDUCTIONS.len()];
    let mut full_source_regressions = [0_usize; RMS_REDUCTIONS.len()];
    let mut bl_regressions = [0_usize; RMS_REDUCTIONS.len()];
    let mut selected_by_category = [[0_usize; CORPUS_CATEGORIES.len()]; RMS_REDUCTIONS.len()];

    for category in 0..CORPUS_CATEGORIES.len() {
        for case in 0..CORPUS_CASES_PER_CATEGORY {
            let data = corpus_curve(category, case, &mut state);
            let knots = sanitize_knots(&data.knots);
            let source = SourceCurve::compile(&knots);
            let shipping = data.compile_rt();
            let candidate = uniform_error_shaped_c1(&source, &knots);
            let (cheap_shipping_rms, cheap_shipping_peak) =
                direct_metrics_grid(&source, |phase| shipping_raw(shipping, phase), CHEAP_GRID);
            let (cheap_candidate_rms, cheap_candidate_peak) =
                direct_metrics_grid(&source, |phase| shipping_raw(candidate, phase), CHEAP_GRID);
            let (full_shipping_rms, full_shipping_peak) =
                direct_metrics_grid(&source, |phase| shipping_raw(shipping, phase), CORPUS_GRID);
            let (full_candidate_rms, full_candidate_peak) =
                direct_metrics_grid(&source, |phase| shipping_raw(candidate, phase), CORPUS_GRID);
            let shipping_cubic = shipping_as_cubic(shipping);
            let candidate_cubic = shipping_as_cubic(candidate);
            let shipping_extrema = shipping_cubic.extrema();
            let candidate_extrema = candidate_cubic.extrema();
            let hard = knots.iter().map(|knot| knot.phase).collect::<Vec<_>>();
            let (shipping_smooth, shipping_hard, shipping_wrap) =
                derivative_metrics(shipping_cubic, &hard);
            let (candidate_smooth, candidate_hard, candidate_wrap) =
                derivative_metrics(candidate_cubic, &hard);
            let guards = cheap_candidate_peak <= cheap_shipping_peak + 1.0e-7
                && every_knot_no_worse(&knots, shipping, candidate)
                && candidate_extrema.2 <= shipping_extrema.2
                && overshoot(candidate_extrema) <= overshoot(shipping_extrema) + 1.0e-6
                && candidate_smooth <= shipping_smooth + 1.0e-3
                && (shipping_hard <= 1.0e-3 || candidate_hard >= shipping_hard * 0.95)
                && (shipping_wrap <= 1.0e-3 || candidate_wrap >= shipping_wrap * 0.95);
            if guards {
                let reference_spectrum =
                    spectrum_grid(|phase| source.eval(f64::from(phase)) as f32, CORPUS_GRID);
                let shipping_spectrum =
                    spectrum_grid(|phase| shipping_raw(shipping, phase), CORPUS_GRID);
                let candidate_spectrum =
                    spectrum_grid(|phase| shipping_raw(candidate, phase), CORPUS_GRID);
                let worst_bl_delta = [436, 55, 7]
                    .map(|period| {
                        bandlimited_error(&reference_spectrum, &candidate_spectrum, period)
                            - bandlimited_error(&reference_spectrum, &shipping_spectrum, period)
                    })
                    .into_iter()
                    .fold(f64::NEG_INFINITY, f64::max);
                for (index, reduction) in RMS_REDUCTIONS.into_iter().enumerate() {
                    if cheap_candidate_rms <= cheap_shipping_rms * (1.0 - reduction) {
                        selected[index] += 1;
                        selected_by_category[index][category] += 1;
                        full_source_regressions[index] += usize::from(
                            full_candidate_rms > full_shipping_rms
                                || full_candidate_peak > full_shipping_peak + 1.0e-6,
                        );
                        bl_regressions[index] += usize::from(worst_bl_delta > 0.0);
                    }
                }
            }
        }
    }

    for (index, reduction) in RMS_REDUCTIONS.into_iter().enumerate() {
        println!(
            "residual_weighted_selector,rms_reduction={reduction:.3},selected={},full_source_regressions={},bl_regressions={},categories={:?}",
            selected[index],
            full_source_regressions[index],
            bl_regressions[index],
            selected_by_category[index],
        );
    }
    let representative = &curves()[1].1;
    println!(
        "residual_weighted_cost,shared_ls_representative_ns={:.1},residual_weighted_representative_ns={:.1},bytes={}",
        compile_least_squares_ns(representative),
        compile_error_shaped_ns(representative),
        size_of::<WaveCurveRt>(),
    );
    assert_eq!(size_of::<WaveCurveRt>(), 256);
}

#[test]
#[ignore = "manual release-mode range-safe compiler experiment"]
fn range_safe_bezier_compiler_report() {
    let mut state = 0x4b55_5256_c101_2026;
    let mut shipping_safe = 0;
    let mut candidate_safe = 0;
    let mut unchanged_from_shared = 0;
    let mut source_better = 0;
    let mut source_regressions = 0;
    let mut bl_better = 0;
    let mut bl_regressions = 0;
    let mut topology_regressions = 0;
    let mut interpolation_failures = 0;
    let mut previous = None;
    let mut bl_deltas = Vec::new();

    for category in 0..CORPUS_CATEGORIES.len() {
        for case in 0..CORPUS_CASES_PER_CATEGORY {
            let data = corpus_curve(category, case, &mut state);
            let knots = sanitize_knots(&data.knots);
            let source = SourceCurve::compile(&knots);
            let shipping = data.compile_rt();
            let shared = uniform_least_squares_c1(&source, &knots);
            let candidate = range_safe_uniform_c1(&source, &knots);
            shipping_safe += usize::from(bezier_safe(shipping));
            candidate_safe += usize::from(bezier_safe(candidate));
            unchanged_from_shared += usize::from(candidate == shared);
            let candidate_extrema = shipping_as_cubic(candidate).extrema();
            assert_eq!(candidate_extrema.2, 0);
            assert!(candidate_extrema.0 >= -1.0 - 2.0e-6);
            assert!(candidate_extrema.1 <= 1.0 + 2.0e-6);

            let (shipping_rms, shipping_peak) =
                direct_metrics_grid(&source, |phase| shipping_raw(shipping, phase), CORPUS_GRID);
            let (candidate_rms, candidate_peak) =
                direct_metrics_grid(&source, |phase| shipping_raw(candidate, phase), CORPUS_GRID);
            source_better += usize::from(
                candidate_rms < shipping_rms && candidate_peak <= shipping_peak + 1.0e-6,
            );
            source_regressions += usize::from(
                candidate_rms > shipping_rms + 1.0e-9 || candidate_peak > shipping_peak + 1.0e-6,
            );

            let hard = knots.iter().map(|knot| knot.phase).collect::<Vec<_>>();
            let (shipping_smooth, shipping_hard, shipping_wrap) =
                derivative_metrics(shipping_as_cubic(shipping), &hard);
            let (candidate_smooth, candidate_hard, candidate_wrap) =
                derivative_metrics(shipping_as_cubic(candidate), &hard);
            topology_regressions += usize::from(
                candidate_smooth > shipping_smooth + 1.0e-3
                    || (shipping_hard > 1.0e-3 && candidate_hard < shipping_hard * 0.95)
                    || (shipping_wrap > 1.0e-3 && candidate_wrap < shipping_wrap * 0.95),
            );

            let reference_spectrum =
                spectrum_grid(|phase| source.eval(f64::from(phase)) as f32, CORPUS_GRID);
            let shipping_spectrum =
                spectrum_grid(|phase| shipping_raw(shipping, phase), CORPUS_GRID);
            let candidate_spectrum =
                spectrum_grid(|phase| shipping_raw(candidate, phase), CORPUS_GRID);
            let worst_delta = [436, 55, 7]
                .map(|period| {
                    bandlimited_error(&reference_spectrum, &candidate_spectrum, period)
                        - bandlimited_error(&reference_spectrum, &shipping_spectrum, period)
                })
                .into_iter()
                .fold(f64::NEG_INFINITY, f64::max);
            bl_better += usize::from(worst_delta < 0.0);
            bl_regressions += usize::from(worst_delta > 0.0);
            bl_deltas.push(worst_delta);

            if let Some(previous) = previous {
                for mix in [0.25, 0.5, 0.75] {
                    interpolation_failures += usize::from(!bezier_safe(WaveCurveRt::interpolate(
                        previous, candidate, mix,
                    )));
                }
            }
            previous = Some(candidate);
        }
    }

    bl_deltas.sort_by(f64::total_cmp);
    let representative = &curves()[1].1;
    let knots = sanitize_knots(&representative.knots);
    let source = SourceCurve::compile(&knots);
    let compile_started = Instant::now();
    for _ in 0..2_000 {
        black_box(range_safe_uniform_c1(black_box(&source), black_box(&knots)));
    }
    let compile_ns = compile_started.elapsed().as_nanos() as f64 / 2_000.0;
    let safe = range_safe_uniform_c1(&source, &knots);
    let [eval, raw, eval4, raw4, eval8, raw8] = range_eval_ns(safe);
    println!(
        "range_safe,cases=512,shipping_bezier_safe={shipping_safe},candidate_bezier_safe={candidate_safe},unchanged_from_shared={unchanged_from_shared},source_better={source_better},source_regressions={source_regressions},bl_better={bl_better},bl_regressions={bl_regressions},topology_regressions={topology_regressions},interpolation_failures={interpolation_failures},bl_delta_min={:.6},bl_delta_median={:.6},bl_delta_max={:.6},compile_ns={compile_ns:.1},bytes={}",
        bl_deltas[0],
        bl_deltas[bl_deltas.len() / 2],
        bl_deltas[bl_deltas.len() - 1],
        size_of::<WaveCurveRt>(),
    );
    println!(
        "range_safe_eval,scalar_clamped_ns={eval:.3},scalar_raw_ns={raw:.3},eval4_clamped_ns={eval4:.3},eval4_raw_ns={raw4:.3},eval8_clamped_ns={eval8:.3},eval8_raw_ns={raw8:.3}"
    );
    assert_eq!(candidate_safe, 512);
    assert_eq!(interpolation_failures, 0);
}

#[test]
#[ignore = "manual release-mode fixed-budget layout experiment"]
fn fixed_256_byte_quadratic_and_linear_report() {
    let mut state = 0x4b55_5256_c101_2026;
    let mut quadratic_source_regressions = 0;
    let mut linear_source_regressions = 0;
    let mut quadratic_bl_regressions = 0;
    let mut linear_bl_regressions = 0;
    let mut quadratic_topology_regressions = 0;
    let mut linear_topology_regressions = 0;
    let mut quadratic_knot_regressions = 0;
    let mut linear_knot_regressions = 0;
    let mut quadratic_crossing_cases = 0;
    let mut quadratic_interpolation_failures = 0;
    let mut linear_interpolation_failures = 0;
    let mut previous_quadratic: Option<UniformQuadratic21> = None;
    let mut previous_linear: Option<UniformLinear32> = None;
    let mut quadratic_bl_deltas = Vec::new();
    let mut linear_bl_deltas = Vec::new();

    for category in 0..CORPUS_CATEGORIES.len() {
        for case in 0..CORPUS_CASES_PER_CATEGORY {
            let data = corpus_curve(category, case, &mut state);
            let knots = sanitize_knots(&data.knots);
            let source = SourceCurve::compile(&knots);
            let shipping = data.compile_rt();
            let quadratic = UniformQuadratic21::compile(&source);
            let linear = UniformLinear32::compile(&source);
            let (shipping_rms, shipping_peak) =
                direct_metrics_grid(&source, |phase| shipping_raw(shipping, phase), CORPUS_GRID);
            let (quadratic_rms, quadratic_peak) =
                direct_metrics_grid(&source, |phase| quadratic.eval_raw(phase), CORPUS_GRID);
            let (linear_rms, linear_peak) =
                direct_metrics_grid(&source, |phase| linear.eval_raw(phase), CORPUS_GRID);
            quadratic_source_regressions += usize::from(
                quadratic_rms > shipping_rms + 1.0e-9 || quadratic_peak > shipping_peak + 1.0e-6,
            );
            linear_source_regressions += usize::from(
                linear_rms > shipping_rms + 1.0e-9 || linear_peak > shipping_peak + 1.0e-6,
            );
            quadratic_knot_regressions += usize::from(knots.iter().any(|knot| {
                (quadratic.eval_raw(knot.phase) - knot.value).abs()
                    > (shipping_raw(shipping, knot.phase) - knot.value).abs() + 1.0e-6
            }));
            linear_knot_regressions += usize::from(knots.iter().any(|knot| {
                (linear.eval_raw(knot.phase) - knot.value).abs()
                    > (shipping_raw(shipping, knot.phase) - knot.value).abs() + 1.0e-6
            }));
            quadratic_crossing_cases += usize::from(quadratic_crossings(quadratic) > 0);

            let hard = knots.iter().map(|knot| knot.phase).collect::<Vec<_>>();
            let (shipping_smooth, shipping_hard, shipping_wrap) =
                derivative_metrics(shipping_as_cubic(shipping), &hard);
            let (quadratic_smooth, quadratic_hard, quadratic_wrap) =
                quadratic_derivative_metrics(quadratic, &hard);
            let (linear_smooth, linear_hard, linear_wrap) =
                linear_derivative_metrics(linear, &hard);
            quadratic_topology_regressions += usize::from(
                quadratic_smooth > shipping_smooth + 1.0e-3
                    || (shipping_hard > 1.0e-3 && quadratic_hard < shipping_hard * 0.95)
                    || (shipping_wrap > 1.0e-3 && quadratic_wrap < shipping_wrap * 0.95),
            );
            linear_topology_regressions += usize::from(
                linear_smooth > shipping_smooth + 1.0e-3
                    || (shipping_hard > 1.0e-3 && linear_hard < shipping_hard * 0.95)
                    || (shipping_wrap > 1.0e-3 && linear_wrap < shipping_wrap * 0.95),
            );

            let reference_spectrum =
                spectrum_grid(|phase| source.eval(f64::from(phase)) as f32, CORPUS_GRID);
            let shipping_spectrum =
                spectrum_grid(|phase| shipping_raw(shipping, phase), CORPUS_GRID);
            let quadratic_spectrum = spectrum_grid(|phase| quadratic.eval_raw(phase), CORPUS_GRID);
            let linear_spectrum = spectrum_grid(|phase| linear.eval_raw(phase), CORPUS_GRID);
            let quadratic_delta = [436, 55, 7]
                .map(|period| {
                    bandlimited_error(&reference_spectrum, &quadratic_spectrum, period)
                        - bandlimited_error(&reference_spectrum, &shipping_spectrum, period)
                })
                .into_iter()
                .fold(f64::NEG_INFINITY, f64::max);
            let linear_delta = [436, 55, 7]
                .map(|period| {
                    bandlimited_error(&reference_spectrum, &linear_spectrum, period)
                        - bandlimited_error(&reference_spectrum, &shipping_spectrum, period)
                })
                .into_iter()
                .fold(f64::NEG_INFINITY, f64::max);
            quadratic_bl_regressions += usize::from(quadratic_delta > 0.0);
            linear_bl_regressions += usize::from(linear_delta > 0.0);
            quadratic_bl_deltas.push(quadratic_delta);
            linear_bl_deltas.push(linear_delta);

            if let (Some(previous_quadratic), Some(previous_linear)) =
                (previous_quadratic, previous_linear)
            {
                for mix in [0.25_f32, 0.5, 0.75] {
                    let mixed_quadratic = UniformQuadratic21 {
                        coefficients: std::array::from_fn(|index| {
                            (quadratic.coefficients[index] - previous_quadratic.coefficients[index])
                                .mul_add(mix, previous_quadratic.coefficients[index])
                        }),
                    };
                    let mixed_linear = UniformLinear32 {
                        coefficients: std::array::from_fn(|index| {
                            (linear.coefficients[index] - previous_linear.coefficients[index])
                                .mul_add(mix, previous_linear.coefficients[index])
                        }),
                    };
                    for phase in [0.013, 0.271, 0.509, 0.887] {
                        let expected_quadratic = (quadratic.eval_raw(phase)
                            - previous_quadratic.eval_raw(phase))
                        .mul_add(mix, previous_quadratic.eval_raw(phase));
                        let expected_linear = (linear.eval_raw(phase)
                            - previous_linear.eval_raw(phase))
                        .mul_add(mix, previous_linear.eval_raw(phase));
                        quadratic_interpolation_failures += usize::from(
                            (mixed_quadratic.eval_raw(phase) - expected_quadratic).abs() > 2.0e-6,
                        );
                        linear_interpolation_failures += usize::from(
                            (mixed_linear.eval_raw(phase) - expected_linear).abs() > 2.0e-6,
                        );
                    }
                }
            }
            previous_quadratic = Some(quadratic);
            previous_linear = Some(linear);
        }
    }

    quadratic_bl_deltas.sort_by(f64::total_cmp);
    linear_bl_deltas.sort_by(f64::total_cmp);
    let representative = &curves()[1].1;
    let knots = sanitize_knots(&representative.knots);
    let source = SourceCurve::compile(&knots);
    let shipping = representative.compile_rt();
    let quadratic = UniformQuadratic21::compile(&source);
    let linear = UniformLinear32::compile(&source);
    let scalar_shipping =
        measure_eval_ns(|index| shipping.eval((index & 65_535) as f32 / 65_536.0));
    let scalar_quadratic =
        measure_eval_ns(|index| quadratic.eval_raw((index & 65_535) as f32 / 65_536.0));
    let scalar_linear =
        measure_eval_ns(|index| linear.eval_raw((index & 65_535) as f32 / 65_536.0));
    let batch4_shipping = measure_eval_ns(|index| {
        let base = (index * 4) & 65_535;
        let phase = f32x4::from(std::array::from_fn(|lane| {
            ((base + lane) & 65_535) as f32 / 65_536.0
        }));
        let values: [f32; 4] = shipping.eval4(phase).into();
        values.into_iter().sum()
    });
    let batch4_quadratic = measure_eval_ns(|index| {
        let base = (index * 4) & 65_535;
        (0..4)
            .map(|lane| quadratic.eval_raw(((base + lane) & 65_535) as f32 / 65_536.0))
            .sum()
    });
    let batch4_linear = measure_eval_ns(|index| {
        let base = (index * 4) & 65_535;
        (0..4)
            .map(|lane| linear.eval_raw(((base + lane) & 65_535) as f32 / 65_536.0))
            .sum()
    });
    let batch8_quadratic = measure_eval_ns(|index| {
        let base = (index * 8) & 65_535;
        (0..8)
            .map(|lane| quadratic.eval_raw(((base + lane) & 65_535) as f32 / 65_536.0))
            .sum()
    });
    let batch8_linear = measure_eval_ns(|index| {
        let base = (index * 8) & 65_535;
        (0..8)
            .map(|lane| linear.eval_raw(((base + lane) & 65_535) as f32 / 65_536.0))
            .sum()
    });
    let batch8_shipping = measure_eval_ns(|index| {
        let base = (index * 8) & 65_535;
        let phase = f32x8::from(std::array::from_fn(|lane| {
            ((base + lane) & 65_535) as f32 / 65_536.0
        }));
        let values: [f32; 8] = shipping.eval8(phase).into();
        values.into_iter().sum()
    });
    println!(
        "fixed_layout,cases=512,quadratic_source_regressions={quadratic_source_regressions},linear_source_regressions={linear_source_regressions},quadratic_bl_regressions={quadratic_bl_regressions},linear_bl_regressions={linear_bl_regressions},quadratic_topology_regressions={quadratic_topology_regressions},linear_topology_regressions={linear_topology_regressions},quadratic_knot_regressions={quadratic_knot_regressions},linear_knot_regressions={linear_knot_regressions},quadratic_crossing_cases={quadratic_crossing_cases},quadratic_interpolation_failures={quadratic_interpolation_failures},linear_interpolation_failures={linear_interpolation_failures},quadratic_bl_min={:.6},quadratic_bl_median={:.6},quadratic_bl_max={:.6},linear_bl_min={:.6},linear_bl_median={:.6},linear_bl_max={:.6},shipping_bytes={},quadratic_bytes={},linear_bytes={}",
        quadratic_bl_deltas[0],
        quadratic_bl_deltas[256],
        quadratic_bl_deltas[511],
        linear_bl_deltas[0],
        linear_bl_deltas[256],
        linear_bl_deltas[511],
        size_of::<WaveCurveRt>(),
        size_of::<UniformQuadratic21>(),
        size_of::<UniformLinear32>(),
    );
    println!(
        "fixed_layout_cpu,shipping_scalar_ns={scalar_shipping:.3},quadratic_scalar_ns={scalar_quadratic:.3},linear_scalar_ns={scalar_linear:.3},shipping4_ns={batch4_shipping:.3},quadratic4_ns={batch4_quadratic:.3},linear4_ns={batch4_linear:.3},shipping8_ns={batch8_shipping:.3},quadratic8_ns={batch8_quadratic:.3},linear8_ns={batch8_linear:.3}"
    );
    assert_eq!(size_of::<UniformQuadratic21>(), 256);
    assert_eq!(size_of::<UniformLinear32>(), 256);
    assert_eq!(quadratic_interpolation_failures, 0);
    assert_eq!(linear_interpolation_failures, 0);
}

#[test]
#[ignore = "manual release-mode derivative-jump regularization experiment"]
fn derivative_jump_regularized_selector_report() {
    const LAMBDAS: [f64; 4] = [1.0e-6, 1.0e-4, 1.0e-2, 1.0];
    let mut state = 0x4b55_5256_c101_2026;
    let mut source_better = [0_usize; LAMBDAS.len()];
    let mut energy_better = [0_usize; LAMBDAS.len()];
    let mut selected_1 = [0_usize; LAMBDAS.len()];
    let mut selected_25 = [0_usize; LAMBDAS.len()];
    let mut bl_regressions_1 = [0_usize; LAMBDAS.len()];
    let mut bl_regressions_25 = [0_usize; LAMBDAS.len()];
    let mut bl_wins_1 = [0_usize; LAMBDAS.len()];
    let mut bl_wins_25 = [0_usize; LAMBDAS.len()];
    let mut dense_regressions_1 = [0_usize; LAMBDAS.len()];
    let mut dense_regressions_25 = [0_usize; LAMBDAS.len()];
    let mut topology_rejections = [0_usize; LAMBDAS.len()];
    let mut interpolation_failures = [0_usize; LAMBDAS.len()];
    let mut previous: [Option<WaveCurveRt>; LAMBDAS.len()] = [None; LAMBDAS.len()];
    let mut chosen_by_category = [0_usize; CORPUS_CATEGORIES.len()];
    let mut chosen_source_reductions = Vec::new();
    let mut chosen_bl_deltas = Vec::new();
    let mut chosen_energy_reductions = Vec::new();

    for category in 0..CORPUS_CATEGORIES.len() {
        for case in 0..CORPUS_CASES_PER_CATEGORY {
            let data = corpus_curve(category, case, &mut state);
            let knots = sanitize_knots(&data.knots);
            let source = SourceCurve::compile(&knots);
            let shipping = pre_regularized_curve(&data);
            let shipping_cubic = shipping_as_cubic(shipping);
            let shipping_extrema = shipping_cubic.extrema();
            let hard = knots.iter().map(|knot| knot.phase).collect::<Vec<_>>();
            let intentional_hard = knots
                .iter()
                .filter(|knot| {
                    (source_value_slope(&source, knot.phase, false).1
                        - source_value_slope(&source, knot.phase, true).1)
                        .abs()
                        > 1.0e-3
                })
                .map(|knot| knot.phase)
                .collect::<Vec<_>>();
            let (shipping_smooth, shipping_hard, shipping_wrap) =
                derivative_metrics(shipping_cubic, &hard);
            let shipping_energy = smooth_derivative_energy(shipping_cubic, &intentional_hard);
            let (cheap_shipping_rms, cheap_shipping_peak) =
                direct_metrics_grid(&source, |phase| shipping_raw(shipping, phase), 256);
            let reference_spectrum =
                spectrum_grid(|phase| source.eval(f64::from(phase)) as f32, CORPUS_GRID);
            let shipping_spectrum =
                spectrum_grid(|phase| shipping_raw(shipping, phase), CORPUS_GRID);

            for (index, lambda) in LAMBDAS.into_iter().enumerate() {
                let candidate = uniform_jump_regularized(&source, &knots, lambda);
                let candidate_cubic = shipping_as_cubic(candidate);
                let candidate_extrema = candidate_cubic.extrema();
                let (candidate_smooth, candidate_hard, candidate_wrap) =
                    derivative_metrics(candidate_cubic, &hard);
                let candidate_energy = smooth_derivative_energy(candidate_cubic, &intentional_hard);
                let (cheap_candidate_rms, cheap_candidate_peak) =
                    direct_metrics_grid(&source, |phase| shipping_raw(candidate, phase), 256);
                source_better[index] += usize::from(
                    cheap_candidate_rms < cheap_shipping_rms
                        && cheap_candidate_peak <= cheap_shipping_peak + 1.0e-7,
                );
                energy_better[index] += usize::from(candidate_energy < shipping_energy);
                let topology_ok = candidate_smooth <= shipping_smooth + 1.0e-6
                    && candidate_energy <= shipping_energy
                    && (shipping_hard <= 1.0e-3 || candidate_hard >= shipping_hard * 0.95)
                    && (shipping_wrap <= 1.0e-3 || candidate_wrap >= shipping_wrap * 0.95)
                    && candidate_extrema.2 <= shipping_extrema.2
                    && overshoot(candidate_extrema) <= overshoot(shipping_extrema) + 1.0e-6
                    && every_knot_no_worse(&knots, shipping, candidate)
                    && cheap_candidate_peak <= cheap_shipping_peak + 1.0e-7;
                topology_rejections[index] += usize::from(!topology_ok);
                let select_1 = topology_ok && cheap_candidate_rms <= cheap_shipping_rms * 0.99;
                let select_25 = topology_ok && cheap_candidate_rms <= cheap_shipping_rms * 0.75;
                if index == 0 {
                    assert_eq!(
                        data.compile_rt(),
                        if select_1 { candidate } else { shipping }
                    );
                    assert_eq!(
                        candidate,
                        WaveCurveRt::from_jump_regularized_source(&source, &knots)
                    );
                }
                if select_1 || select_25 {
                    let (full_shipping_rms, full_shipping_peak) = direct_metrics_grid(
                        &source,
                        |phase| shipping_raw(shipping, phase),
                        CORPUS_GRID,
                    );
                    let (full_candidate_rms, full_candidate_peak) = direct_metrics_grid(
                        &source,
                        |phase| shipping_raw(candidate, phase),
                        CORPUS_GRID,
                    );
                    let dense_regression = full_candidate_rms > full_shipping_rms
                        || full_candidate_peak > full_shipping_peak + 1.0e-6;
                    let candidate_spectrum =
                        spectrum_grid(|phase| shipping_raw(candidate, phase), CORPUS_GRID);
                    let worst_delta = [436, 55, 7]
                        .map(|period| {
                            bandlimited_error(&reference_spectrum, &candidate_spectrum, period)
                                - bandlimited_error(&reference_spectrum, &shipping_spectrum, period)
                        })
                        .into_iter()
                        .fold(f64::NEG_INFINITY, f64::max);
                    if select_1 {
                        selected_1[index] += 1;
                        bl_regressions_1[index] += usize::from(worst_delta > 0.0);
                        bl_wins_1[index] += usize::from(worst_delta < 0.0);
                        dense_regressions_1[index] += usize::from(dense_regression);
                        if index == 0 {
                            chosen_by_category[category] += 1;
                            chosen_source_reductions
                                .push((1.0 - full_candidate_rms / full_shipping_rms) * 100.0);
                            chosen_bl_deltas.push(worst_delta);
                            chosen_energy_reductions.push(shipping_energy - candidate_energy);
                        }
                    }
                    if select_25 {
                        selected_25[index] += 1;
                        bl_regressions_25[index] += usize::from(worst_delta > 0.0);
                        bl_wins_25[index] += usize::from(worst_delta < 0.0);
                        dense_regressions_25[index] += usize::from(dense_regression);
                    }
                }
                if let Some(previous) = previous[index] {
                    let mixed = WaveCurveRt::interpolate(previous, candidate, 0.37);
                    for phase in [0.013, 0.271, 0.509, 0.887] {
                        let expected = (shipping_raw(candidate, phase)
                            - shipping_raw(previous, phase))
                        .mul_add(0.37, shipping_raw(previous, phase));
                        interpolation_failures[index] +=
                            usize::from((shipping_raw(mixed, phase) - expected).abs() > 2.0e-6);
                    }
                }
                previous[index] = Some(candidate);
            }
        }
    }

    let representative = &curves()[1].1;
    let knots = sanitize_knots(&representative.knots);
    let source = SourceCurve::compile(&knots);
    for (index, lambda) in LAMBDAS.into_iter().enumerate() {
        let started = Instant::now();
        for _ in 0..2_000 {
            black_box(uniform_jump_regularized(
                black_box(&source),
                black_box(&knots),
                lambda,
            ));
        }
        let compile_ns = started.elapsed().as_nanos() as f64 / 2_000.0;
        println!(
            "jump_regularized,lambda={lambda:.0e},source_better={},energy_better={},topology_rejections={},selected_1pct={},dense_regressions_1pct={},bl_wins_1pct={},bl_regressions_1pct={},selected_25pct={},dense_regressions_25pct={},bl_wins_25pct={},bl_regressions_25pct={},interpolation_failures={},compile_ns={compile_ns:.1}",
            source_better[index],
            energy_better[index],
            topology_rejections[index],
            selected_1[index],
            dense_regressions_1[index],
            bl_wins_1[index],
            bl_regressions_1[index],
            selected_25[index],
            dense_regressions_25[index],
            bl_wins_25[index],
            bl_regressions_25[index],
            interpolation_failures[index],
        );
    }
    chosen_source_reductions.sort_by(f64::total_cmp);
    chosen_bl_deltas.sort_by(f64::total_cmp);
    chosen_energy_reductions.sort_by(f64::total_cmp);
    println!(
        "jump_regularized_chosen,categories={chosen_by_category:?},source_reduction_percent_min={:.6},source_reduction_percent_median={:.6},source_reduction_percent_max={:.6},bl_delta_min={:.6},bl_delta_median={:.6},bl_delta_max={:.6},event_energy_reduction_min={:.6},event_energy_reduction_median={:.6},event_energy_reduction_max={:.6}",
        chosen_source_reductions[0],
        chosen_source_reductions[chosen_source_reductions.len() / 2],
        chosen_source_reductions[chosen_source_reductions.len() - 1],
        chosen_bl_deltas[0],
        chosen_bl_deltas[chosen_bl_deltas.len() / 2],
        chosen_bl_deltas[chosen_bl_deltas.len() - 1],
        chosen_energy_reductions[0],
        chosen_energy_reductions[chosen_energy_reductions.len() / 2],
        chosen_energy_reductions[chosen_energy_reductions.len() - 1],
    );
    assert_eq!(size_of::<WaveCurveRt>(), 256);
    assert!(
        interpolation_failures
            .into_iter()
            .all(|failures| failures == 0)
    );
}

#[test]
#[ignore = "manual release-mode derivative lambda portfolio experiment"]
fn derivative_lambda_portfolio_report() {
    const LAMBDAS: [f64; 4] = [1.0e-8, 1.0e-7, 1.0e-5, 1.0e-4];
    let mut state = 0x4b55_5256_c101_2026;
    let mut eligible = [0_usize; LAMBDAS.len()];
    let mut selected_by_lambda = [0_usize; LAMBDAS.len()];
    let mut selected_by_category = [0_usize; CORPUS_CATEGORIES.len()];
    let mut selected = 0;
    let mut dense_regressions = 0;
    let mut bl_regressions = 0;
    let mut topology_regressions = 0;
    let mut source_reductions = Vec::new();
    let mut bl_deltas = Vec::new();

    for category in 0..CORPUS_CATEGORIES.len() {
        for case in 0..CORPUS_CASES_PER_CATEGORY {
            let data = corpus_curve(category, case, &mut state);
            let knots = sanitize_knots(&data.knots);
            let source = SourceCurve::compile(&knots);
            let current = data.compile_rt();
            let mut best = None;
            let mut best_squared = f64::INFINITY;
            for (index, lambda) in LAMBDAS.into_iter().enumerate() {
                let candidate = uniform_jump_regularized(&source, &knots, lambda);
                if !candidate.proves_regularized_better_than(current, &source, &knots) {
                    continue;
                }
                eligible[index] += 1;
                let squared = (0..256)
                    .map(|sample| {
                        let phase = sample as f32 / 256.0;
                        let error =
                            shipping_raw(candidate, phase) - source.eval(f64::from(phase)) as f32;
                        f64::from(error * error)
                    })
                    .sum::<f64>();
                if squared < best_squared {
                    best = Some((index, candidate));
                    best_squared = squared;
                }
            }
            let Some((lambda_index, candidate)) = best else {
                continue;
            };
            selected += 1;
            selected_by_lambda[lambda_index] += 1;
            selected_by_category[category] += 1;
            let (current_rms, current_peak) =
                direct_metrics_grid(&source, |phase| shipping_raw(current, phase), CORPUS_GRID);
            let (candidate_rms, candidate_peak) =
                direct_metrics_grid(&source, |phase| shipping_raw(candidate, phase), CORPUS_GRID);
            dense_regressions +=
                usize::from(candidate_rms > current_rms || candidate_peak > current_peak + 1.0e-6);
            source_reductions.push((1.0 - candidate_rms / current_rms) * 100.0);
            let current_cubic = shipping_as_cubic(current);
            let candidate_cubic = shipping_as_cubic(candidate);
            let current_extrema = current_cubic.extrema();
            let candidate_extrema = candidate_cubic.extrema();
            let hard = knots.iter().map(|knot| knot.phase).collect::<Vec<_>>();
            let current_jumps = derivative_metrics(current_cubic, &hard);
            let candidate_jumps = derivative_metrics(candidate_cubic, &hard);
            let intentional_hard = knots
                .iter()
                .filter(|knot| {
                    (source_value_slope(&source, knot.phase, false).1
                        - source_value_slope(&source, knot.phase, true).1)
                        .abs()
                        > 1.0e-3
                })
                .map(|knot| knot.phase)
                .collect::<Vec<_>>();
            topology_regressions += usize::from(
                candidate_jumps.0 > current_jumps.0 + 1.0e-6
                    || smooth_derivative_energy(candidate_cubic, &intentional_hard)
                        > smooth_derivative_energy(current_cubic, &intentional_hard)
                    || (current_jumps.1 > 1.0e-3 && candidate_jumps.1 < current_jumps.1 * 0.95)
                    || (current_jumps.2 > 1.0e-3 && candidate_jumps.2 < current_jumps.2 * 0.95)
                    || candidate_extrema.2 > current_extrema.2
                    || overshoot(candidate_extrema) > overshoot(current_extrema) + 1.0e-6
                    || !every_knot_no_worse(&knots, current, candidate),
            );
            let reference_spectrum =
                spectrum_grid(|phase| source.eval(f64::from(phase)) as f32, CORPUS_GRID);
            let current_spectrum = spectrum_grid(|phase| shipping_raw(current, phase), CORPUS_GRID);
            let candidate_spectrum =
                spectrum_grid(|phase| shipping_raw(candidate, phase), CORPUS_GRID);
            let worst_delta = [436, 55, 7]
                .map(|period| {
                    bandlimited_error(&reference_spectrum, &candidate_spectrum, period)
                        - bandlimited_error(&reference_spectrum, &current_spectrum, period)
                })
                .into_iter()
                .fold(f64::NEG_INFINITY, f64::max);
            bl_regressions += usize::from(worst_delta > 0.0);
            bl_deltas.push(worst_delta);
        }
    }

    source_reductions.sort_by(f64::total_cmp);
    bl_deltas.sort_by(f64::total_cmp);
    let representative = &curves()[1].1;
    let knots = sanitize_knots(&representative.knots);
    let source = SourceCurve::compile(&knots);
    let started = Instant::now();
    for _ in 0..2_000 {
        for lambda in LAMBDAS {
            black_box(uniform_jump_regularized(
                black_box(&source),
                black_box(&knots),
                lambda,
            ));
        }
    }
    let portfolio_compile_ns = started.elapsed().as_nanos() as f64 / 2_000.0;
    println!(
        "jump_portfolio,lambdas={LAMBDAS:?},eligible={eligible:?},selected={selected},selected_by_lambda={selected_by_lambda:?},selected_by_category={selected_by_category:?},dense_regressions={dense_regressions},bl_regressions={bl_regressions},topology_regressions={topology_regressions},portfolio_compile_ns={portfolio_compile_ns:.1}"
    );
    if selected > 0 {
        println!(
            "jump_portfolio_benefit,source_reduction_percent_min={:.6},median={:.6},max={:.6},bl_delta_min={:.6},median={:.6},max={:.6}",
            source_reductions[0],
            source_reductions[selected / 2],
            source_reductions[selected - 1],
            bl_deltas[0],
            bl_deltas[selected / 2],
            bl_deltas[selected - 1],
        );
    }
    assert_eq!(size_of::<WaveCurveRt>(), 256);
}

#[test]
#[ignore = "manual release-mode curvature-event regularization experiment"]
fn curvature_event_regularized_report() {
    const LAMBDAS: [f64; 4] = [1.0e-8, 1.0e-6, 1.0e-4, 1.0e-2];
    let mut state = 0x4b55_5256_c101_2026;
    let mut eligible = [0_usize; LAMBDAS.len()];
    let mut dense_regressions = [0_usize; LAMBDAS.len()];
    let mut bl_regressions = [0_usize; LAMBDAS.len()];
    let mut bl_wins = [0_usize; LAMBDAS.len()];
    let mut slope_energy_better = [0_usize; LAMBDAS.len()];
    let mut curvature_energy_better = [0_usize; LAMBDAS.len()];
    let mut portfolio_selected = 0;
    let mut portfolio_by_lambda = [0_usize; LAMBDAS.len()];
    let mut portfolio_dense_regressions = 0;
    let mut portfolio_bl_regressions = 0;
    let mut portfolio_topology_regressions = 0;

    for category in 0..CORPUS_CATEGORIES.len() {
        for case in 0..CORPUS_CASES_PER_CATEGORY {
            let data = corpus_curve(category, case, &mut state);
            let knots = sanitize_knots(&data.knots);
            let source = SourceCurve::compile(&knots);
            let current = data.compile_rt();
            let current_cubic = shipping_as_cubic(current);
            let intentional_hard = knots
                .iter()
                .filter(|knot| {
                    (source_value_slope(&source, knot.phase, false).1
                        - source_value_slope(&source, knot.phase, true).1)
                        .abs()
                        > 1.0e-3
                })
                .map(|knot| knot.phase)
                .collect::<Vec<_>>();
            let current_slope_energy = smooth_derivative_energy(current_cubic, &intentional_hard);
            let current_curvature_energy =
                smooth_curvature_event_energy(current_cubic, &intentional_hard);
            let (current_rms, current_peak) =
                direct_metrics_grid(&source, |phase| shipping_raw(current, phase), CORPUS_GRID);
            let reference_spectrum =
                spectrum_grid(|phase| source.eval(f64::from(phase)) as f32, CORPUS_GRID);
            let current_spectrum = spectrum_grid(|phase| shipping_raw(current, phase), CORPUS_GRID);
            let mut best = None;
            let mut best_squared = f64::INFINITY;

            for (index, lambda) in LAMBDAS.into_iter().enumerate() {
                let candidate = uniform_curvature_regularized(&source, &knots, lambda);
                let candidate_cubic = shipping_as_cubic(candidate);
                let candidate_slope_energy =
                    smooth_derivative_energy(candidate_cubic, &intentional_hard);
                let candidate_curvature_energy =
                    smooth_curvature_event_energy(candidate_cubic, &intentional_hard);
                slope_energy_better[index] +=
                    usize::from(candidate_slope_energy < current_slope_energy);
                curvature_energy_better[index] +=
                    usize::from(candidate_curvature_energy < current_curvature_energy);
                if candidate_curvature_energy > current_curvature_energy
                    || !candidate.proves_regularized_better_than(current, &source, &knots)
                {
                    continue;
                }
                eligible[index] += 1;
                let (candidate_rms, candidate_peak) = direct_metrics_grid(
                    &source,
                    |phase| shipping_raw(candidate, phase),
                    CORPUS_GRID,
                );
                dense_regressions[index] += usize::from(
                    candidate_rms > current_rms || candidate_peak > current_peak + 1.0e-6,
                );
                let candidate_spectrum =
                    spectrum_grid(|phase| shipping_raw(candidate, phase), CORPUS_GRID);
                let worst_delta = [436, 55, 7]
                    .map(|period| {
                        bandlimited_error(&reference_spectrum, &candidate_spectrum, period)
                            - bandlimited_error(&reference_spectrum, &current_spectrum, period)
                    })
                    .into_iter()
                    .fold(f64::NEG_INFINITY, f64::max);
                bl_regressions[index] += usize::from(worst_delta > 0.0);
                bl_wins[index] += usize::from(worst_delta < 0.0);
                let squared = (0..256)
                    .map(|sample| {
                        let phase = sample as f32 / 256.0;
                        let error =
                            shipping_raw(candidate, phase) - source.eval(f64::from(phase)) as f32;
                        f64::from(error * error)
                    })
                    .sum::<f64>();
                if squared < best_squared {
                    best = Some((index, candidate, candidate_rms, candidate_peak, worst_delta));
                    best_squared = squared;
                }
            }
            if let Some((index, candidate, candidate_rms, candidate_peak, worst_delta)) = best {
                portfolio_selected += 1;
                portfolio_by_lambda[index] += 1;
                portfolio_dense_regressions += usize::from(
                    candidate_rms > current_rms || candidate_peak > current_peak + 1.0e-6,
                );
                portfolio_bl_regressions += usize::from(worst_delta > 0.0);
                let candidate_cubic = shipping_as_cubic(candidate);
                portfolio_topology_regressions += usize::from(
                    smooth_derivative_energy(candidate_cubic, &intentional_hard)
                        > current_slope_energy
                        || smooth_curvature_event_energy(candidate_cubic, &intentional_hard)
                            > current_curvature_energy,
                );
            }
        }
    }

    let representative = &curves()[1].1;
    let knots = sanitize_knots(&representative.knots);
    let source = SourceCurve::compile(&knots);
    let started = Instant::now();
    for _ in 0..2_000 {
        for lambda in LAMBDAS {
            black_box(uniform_curvature_regularized(
                black_box(&source),
                black_box(&knots),
                lambda,
            ));
        }
    }
    let portfolio_compile_ns = started.elapsed().as_nanos() as f64 / 2_000.0;
    println!(
        "curvature_regularized,lambdas={LAMBDAS:?},eligible={eligible:?},dense_regressions={dense_regressions:?},bl_wins={bl_wins:?},bl_regressions={bl_regressions:?},slope_energy_better={slope_energy_better:?},curvature_energy_better={curvature_energy_better:?},portfolio_selected={portfolio_selected},portfolio_by_lambda={portfolio_by_lambda:?},portfolio_dense_regressions={portfolio_dense_regressions},portfolio_bl_regressions={portfolio_bl_regressions},portfolio_topology_regressions={portfolio_topology_regressions},portfolio_compile_ns={portfolio_compile_ns:.1},bytes={}",
        size_of::<WaveCurveRt>()
    );
    assert_eq!(size_of::<WaveCurveRt>(), 256);
}

#[test]
#[ignore = "manual release-mode higher-order fixed-budget experiment"]
fn fixed_256_byte_quartic_and_quintic_report() {
    let mut state = 0x4b55_5256_c101_2026;
    let mut source_regressions = [0_usize; 2];
    let mut knot_regressions = [0_usize; 2];
    let mut topology_regressions = [0_usize; 2];
    let mut bl_regressions = [0_usize; 2];
    let mut bl_wins = [0_usize; 2];
    let mut event_cases = [0_usize; 2];
    let mut slope_energy_better = [0_usize; 2];
    let mut curvature_energy_better = [0_usize; 2];
    let mut interpolation_failures = [0_usize; 2];
    let mut previous_quartic: Option<UniformQuartic12> = None;
    let mut previous_quintic: Option<UniformQuintic10> = None;

    for category in 0..CORPUS_CATEGORIES.len() {
        for case in 0..CORPUS_CASES_PER_CATEGORY {
            let data = corpus_curve(category, case, &mut state);
            let knots = sanitize_knots(&data.knots);
            let source = SourceCurve::compile(&knots);
            let shipping = data.compile_rt();
            let quartic = UniformQuartic12::compile(&source, &knots);
            let quintic = UniformQuintic10::compile(&source, &knots);
            let candidates: [(Box<dyn Fn(f32) -> f32>, [f32; 64], usize, usize); 2] = [
                (
                    Box::new(move |phase| quartic.eval_raw(phase)),
                    quartic.coefficients,
                    12,
                    4,
                ),
                (
                    Box::new(move |phase| quintic.eval_raw(phase)),
                    quintic.coefficients,
                    10,
                    5,
                ),
            ];
            let (shipping_rms, shipping_peak) =
                direct_metrics_grid(&source, |phase| shipping_raw(shipping, phase), CORPUS_GRID);
            let hard = knots
                .iter()
                .filter(|knot| {
                    (source_value_slope(&source, knot.phase, false).1
                        - source_value_slope(&source, knot.phase, true).1)
                        .abs()
                        > 1.0e-3
                })
                .map(|knot| knot.phase)
                .collect::<Vec<_>>();
            let shipping_cubic = shipping_as_cubic(shipping);
            let shipping_slope_energy = smooth_derivative_energy(shipping_cubic, &hard);
            let shipping_curvature_energy = smooth_curvature_event_energy(shipping_cubic, &hard);
            let shipping_jumps = derivative_metrics(shipping_cubic, &hard);
            let reference_spectrum =
                spectrum_grid(|phase| source.eval(f64::from(phase)) as f32, CORPUS_GRID);
            let shipping_spectrum =
                spectrum_grid(|phase| shipping_raw(shipping, phase), CORPUS_GRID);
            for (index, (evaluate, coefficients, segments, degree)) in
                candidates.into_iter().enumerate()
            {
                let (rms, peak) = direct_metrics_grid(&source, &evaluate, CORPUS_GRID);
                source_regressions[index] +=
                    usize::from(rms > shipping_rms || peak > shipping_peak + 1.0e-6);
                knot_regressions[index] += usize::from(knots.iter().any(|knot| {
                    (evaluate(knot.phase) - knot.value).abs()
                        > (shipping_raw(shipping, knot.phase) - knot.value).abs() + 1.0e-6
                }));
                let metrics = polynomial_event_metrics(coefficients, segments, degree, &hard);
                event_cases[index] += usize::from(metrics.4 > 0);
                slope_energy_better[index] += usize::from(metrics.1 < shipping_slope_energy);
                curvature_energy_better[index] +=
                    usize::from(metrics.3 < shipping_curvature_energy);
                topology_regressions[index] += usize::from(
                    metrics.0 > shipping_jumps.0 + 1.0e-3
                        || metrics.1 > shipping_slope_energy + 1.0e-6
                        || metrics.3 > shipping_curvature_energy + 1.0e-4
                        || (shipping_jumps.1 > 1.0e-3 && metrics.5 < shipping_jumps.1 * 0.95)
                        || (shipping_jumps.2 > 1.0e-3 && metrics.6 < shipping_jumps.2 * 0.95),
                );
                let spectrum = spectrum_grid(&evaluate, CORPUS_GRID);
                let worst_delta = [436, 55, 7]
                    .map(|period| {
                        bandlimited_error(&reference_spectrum, &spectrum, period)
                            - bandlimited_error(&reference_spectrum, &shipping_spectrum, period)
                    })
                    .into_iter()
                    .fold(f64::NEG_INFINITY, f64::max);
                bl_regressions[index] += usize::from(worst_delta > 0.0);
                bl_wins[index] += usize::from(worst_delta < 0.0);
            }
            if let Some(previous) = previous_quartic {
                let mixed = UniformQuartic12 {
                    coefficients: std::array::from_fn(|index| {
                        (quartic.coefficients[index] - previous.coefficients[index])
                            .mul_add(0.37, previous.coefficients[index])
                    }),
                };
                for phase in [0.013, 0.271, 0.509, 0.887] {
                    let expected = (quartic.eval_raw(phase) - previous.eval_raw(phase))
                        .mul_add(0.37, previous.eval_raw(phase));
                    interpolation_failures[0] +=
                        usize::from((mixed.eval_raw(phase) - expected).abs() > 2.0e-6);
                }
            }
            if let Some(previous) = previous_quintic {
                let mixed = UniformQuintic10 {
                    coefficients: std::array::from_fn(|index| {
                        (quintic.coefficients[index] - previous.coefficients[index])
                            .mul_add(0.37, previous.coefficients[index])
                    }),
                };
                for phase in [0.013, 0.271, 0.509, 0.887] {
                    let expected = (quintic.eval_raw(phase) - previous.eval_raw(phase))
                        .mul_add(0.37, previous.eval_raw(phase));
                    interpolation_failures[1] +=
                        usize::from((mixed.eval_raw(phase) - expected).abs() > 2.0e-6);
                }
            }
            previous_quartic = Some(quartic);
            previous_quintic = Some(quintic);
        }
    }

    let representative = &curves()[1].1;
    let knots = sanitize_knots(&representative.knots);
    let source = SourceCurve::compile(&knots);
    let started = Instant::now();
    for _ in 0..50 {
        black_box(UniformQuartic12::compile(
            black_box(&source),
            black_box(&knots),
        ));
    }
    let quartic_compile_ns = started.elapsed().as_nanos() as f64 / 50.0;
    let started = Instant::now();
    for _ in 0..50 {
        black_box(UniformQuintic10::compile(
            black_box(&source),
            black_box(&knots),
        ));
    }
    let quintic_compile_ns = started.elapsed().as_nanos() as f64 / 50.0;
    println!(
        "higher_order_fixed,source_regressions={source_regressions:?},knot_regressions={knot_regressions:?},topology_regressions={topology_regressions:?},bl_wins={bl_wins:?},bl_regressions={bl_regressions:?},event_cases={event_cases:?},slope_energy_better={slope_energy_better:?},curvature_energy_better={curvature_energy_better:?},interpolation_failures={interpolation_failures:?},quartic_compile_ns={quartic_compile_ns:.1},quintic_compile_ns={quintic_compile_ns:.1},quartic_bytes={},quintic_bytes={},cpu_skipped=true",
        size_of::<UniformQuartic12>(),
        size_of::<UniformQuintic10>(),
    );
    assert_eq!(size_of::<UniformQuartic12>(), 256);
    assert_eq!(size_of::<UniformQuintic10>(), 256);
}

#[test]
#[ignore = "manual release-mode compiler experiment"]
fn adaptive_c1_compiler_report() {
    println!(
        "contract,segments=16,shipping_bytes={},adaptive_bytes={},grid={GRID},adaptive=editor-boundaries+worst-interval-bisection+cubic-Hermite-C1",
        size_of::<WaveCurveRt>(),
        size_of::<AdaptiveCubic>(),
    );
    for (name, data) in curves() {
        let knots = sanitize_knots(&data.knots);
        let source = SourceCurve::compile(&knots);
        let shipping = legacy_curve(&data);
        let shipping_cubic = shipping_as_cubic(shipping);
        let adaptive = AdaptiveCubic::compile(&data);
        let (shipping_rms, shipping_peak) =
            direct_metrics(&source, |phase| shipping_raw(shipping, phase));
        let (adaptive_rms, adaptive_peak) =
            direct_metrics(&source, |phase| adaptive.eval_raw(phase));
        let (shipping_minimum, shipping_maximum, shipping_crossings) = shipping_cubic.extrema();
        let (adaptive_minimum, adaptive_maximum, adaptive_crossings) = adaptive.extrema();
        let hard = knots.iter().map(|knot| knot.phase).collect::<Vec<_>>();
        let (shipping_smooth_slope_jump, shipping_hard_slope_jump, shipping_wrap_slope_jump) =
            derivative_metrics(shipping_cubic, &hard);
        let (smooth_slope_jump, hard_slope_jump, wrap_slope_jump) =
            derivative_metrics(adaptive, &hard);
        let reference_spectrum = spectrum(|phase| source.eval(f64::from(phase)) as f32);
        let shipping_spectrum = spectrum(|phase| shipping_raw(shipping, phase));
        let adaptive_spectrum = spectrum(|phase| adaptive.eval_raw(phase));
        let (shipping_eval_ns, adaptive_eval_ns) = eval_ns(shipping, adaptive);
        println!(
            "curve,name={name},knots={},shipping_rms={shipping_rms:.9},shipping_peak={shipping_peak:.9},adaptive_rms={adaptive_rms:.9},adaptive_peak={adaptive_peak:.9},shipping_smooth_slope_jump={shipping_smooth_slope_jump:.6},shipping_hard_slope_jump={shipping_hard_slope_jump:.6},shipping_wrap_slope_jump={shipping_wrap_slope_jump:.6},adaptive_smooth_slope_jump={smooth_slope_jump:.6},adaptive_hard_slope_jump={hard_slope_jump:.6},adaptive_wrap_slope_jump={wrap_slope_jump:.6},shipping_minimum={shipping_minimum:.9},shipping_maximum={shipping_maximum:.9},shipping_clamp_crossings={shipping_crossings},adaptive_minimum={adaptive_minimum:.9},adaptive_maximum={adaptive_maximum:.9},adaptive_clamp_crossings={adaptive_crossings},shipping_compile_ns={:.1},adaptive_compile_ns={:.1},shipping_eval_ns={shipping_eval_ns:.3},adaptive_eval_ns={adaptive_eval_ns:.3}",
            knots.len(),
            compile_ns(&data, false),
            compile_ns(&data, true),
        );
        for period in [436, 55, 7] {
            println!(
                "bandlimited,name={name},period={period},shipping_error_db={:.3},adaptive_error_db={:.3}",
                bandlimited_error(&reference_spectrum, &shipping_spectrum, period),
                bandlimited_error(&reference_spectrum, &adaptive_spectrum, period),
            );
        }
        assert!(
            smooth_slope_jump < hard_slope_jump.max(wrap_slope_jump).max(1.0) * 2.0e-4,
            "{name}: artificial join was not C1"
        );
        assert_eq!(
            adaptive_crossings, 0,
            "{name}: analytic extrema prove a clamp crossing"
        );
    }
}
