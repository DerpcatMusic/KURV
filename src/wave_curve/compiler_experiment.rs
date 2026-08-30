use std::hint::black_box;
use std::mem::size_of;
use std::time::Instant;

use crate::dsp::{Complex, fft};

use super::{
    MAX_WAVE_KNOTS, RT_SEGMENTS, SourceCurve, WaveCurveData, WaveCurveRt, WaveKnot,
    coefficient_index, sanitize_knots,
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

fn direct_metrics(source: &SourceCurve, evaluator: impl Fn(f32) -> f32) -> (f64, f32) {
    let mut squared = 0.0;
    let mut peak = 0.0_f32;
    for index in 0..GRID {
        let phase = index as f32 / GRID as f32;
        let error = evaluator(phase) - source.eval(f64::from(phase)) as f32;
        squared += f64::from(error * error);
        peak = peak.max(error.abs());
    }
    ((squared / GRID as f64).sqrt(), peak)
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
    let mut values = (0..GRID)
        .map(|index| Complex::new(f64::from(evaluator(index as f32 / GRID as f32)), 0.0))
        .collect::<Vec<_>>();
    fft(&mut values, false);
    for value in &mut values {
        *value /= GRID as f64;
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
            black_box(black_box(data).compile_rt());
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
        let shipping = data.compile_rt();
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
