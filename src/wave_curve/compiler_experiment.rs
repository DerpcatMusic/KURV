use std::hint::black_box;
use std::mem::size_of;
use std::time::Instant;

use crate::dsp::{Complex, fft};

use super::{
    MAX_WAVE_KNOTS, RT_SEGMENTS, SourceCurve, WaveCurveData, WaveCurveRt, WaveKnot,
    coefficient_index, has_tight_transition, sanitize_knots,
};

const GRID: usize = 65_536;

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

fn uniform_least_squares_c1(source: &SourceCurve, knots: &[WaveKnot]) -> WaveCurveRt {
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
        if !hard {
            let left = find(&parent, (boundary - 1) * 2 + 1);
            let right = find(&parent, boundary * 2);
            parent[right] = left;
        }
    }
    let wrap_hard =
        (source_value_slope(source, 0.0, false).1 - source_value_slope(source, 0.0, true).1).abs()
            > 1.0e-3;
    if !wrap_hard {
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
            let target = source.eval(width.mul_add(t, start)) - y0 * h00 - y1 * h01;
            for (row, row_feature) in [(left, features[0]), (right, features[1])] {
                for (column, column_feature) in [(left, features[0]), (right, features[1])] {
                    normal[row][column] += row_feature * column_feature;
                }
                normal[row][variable_count] += row_feature * target;
            }
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
                assert_eq!(data.compile_rt(), shipping);
                continue;
            }
            let production_selects = cheap_candidate_rms <= cheap_shipping_rms * 0.75
                && cheap_candidate_peak <= cheap_shipping_peak + 1.0e-7;
            assert_eq!(
                data.compile_rt(),
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
