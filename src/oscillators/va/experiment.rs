use std::hint::black_box;
use std::time::Instant;

use truce_simd::simd::f32x8;
use wide::{CmpGt, CmpLt};

use crate::dsp::{Complex, fft};
use crate::oversampling::StereoOversampler;
use crate::wave_curve::{WaveCurveData, WaveCurveRt, WaveKnot};

use super::warp::{warp_phase_position_scalar, warped_pulse_edge_scalar};
use super::{Antialiasing, PhaseWarpMode, VaOscillator, accumulate_saw8_block_constant};

const REFERENCE_SAMPLES: usize = 65_536;
const TARGET_FREQUENCIES: [f64; 3] = [110.0, 880.0, 7_040.0];
const SAMPLE_RATES: [f64; 3] = [44_100.0, 48_000.0, 96_000.0];

#[derive(Clone, Copy)]
enum Shape {
    Saw,
    Square,
    Pulse,
    Triangle,
    Custom,
}

impl Shape {
    const ALL: [Self; 5] = [
        Self::Saw,
        Self::Square,
        Self::Pulse,
        Self::Triangle,
        Self::Custom,
    ];

    const fn name(self) -> &'static str {
        match self {
            Self::Saw => "saw",
            Self::Square => "square",
            Self::Pulse => "pulse31",
            Self::Triangle => "triangle",
            Self::Custom => "drawn",
        }
    }

    const fn shape(self) -> f32 {
        match self {
            Self::Saw => 2.0,
            Self::Square | Self::Pulse => 3.0,
            Self::Triangle => 1.0,
            Self::Custom => 0.0,
        }
    }

    const fn pulse_width(self) -> f32 {
        match self {
            Self::Pulse => 0.31,
            _ => 0.5,
        }
    }
}

fn drawn_curve() -> WaveCurveRt {
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
    }
    .compile_rt()
}

fn next_shipping(
    oscillator: &mut VaOscillator,
    shape: Shape,
    step: f32,
    curve: WaveCurveRt,
) -> f32 {
    if matches!(shape, Shape::Custom) {
        oscillator.generate_custom_step(
            0.0,
            step,
            0.5,
            Antialiasing::SplineOptimized,
            PhaseWarpMode::None,
            0.0,
            curve,
            1.0,
        )
    } else {
        oscillator.generate_shape_step(
            shape.shape(),
            step,
            shape.pulse_width(),
            Antialiasing::SplineOptimized,
        )
    }
}

fn render_shipping(
    shape: Shape,
    period: usize,
    samples: usize,
    factor: u8,
    curve: WaveCurveRt,
) -> Vec<f64> {
    let step = 1.0 / (period * usize::from(factor)) as f32;
    let mut oscillator = VaOscillator::default();
    let mut oversampler = StereoOversampler::default();
    oversampler.reset(factor);
    oversampler.set_spline_correction_immediate(true);
    let mut output = Vec::with_capacity(samples);
    for host_sample in 0..samples + period * 8 {
        for _ in 0..factor {
            let sample = next_shipping(&mut oscillator, shape, step, curve);
            oversampler.push(sample, sample);
        }
        let sample = oversampler.output().0;
        if host_sample >= period * 8 {
            output.push(f64::from(sample));
        }
    }
    output
}

fn render_shipping_warped(
    shape: Shape,
    period: usize,
    samples: usize,
    factor: u8,
    mode: PhaseWarpMode,
    amount: f32,
) -> Vec<f64> {
    let step = 1.0 / (period * usize::from(factor)) as f32;
    let mut oscillator = VaOscillator::default();
    let mut oversampler = StereoOversampler::default();
    oversampler.reset(factor);
    oversampler.set_spline_correction_immediate(true);
    let mut output = Vec::with_capacity(samples);
    for host_sample in 0..samples + period * 8 {
        for _ in 0..factor {
            let sample = oscillator.generate_shape_step_warped(
                shape.shape(),
                step,
                shape.pulse_width(),
                Antialiasing::SplineOptimized,
                mode,
                amount,
            );
            oversampler.push(sample, sample);
        }
        let sample = oversampler.output().0;
        if host_sample >= period * 8 {
            output.push(f64::from(sample));
        }
    }
    output
}

fn analytic_coefficient(shape: Shape, harmonic: usize) -> Complex {
    if harmonic == 0 {
        return match shape {
            Shape::Pulse => Complex::new(-0.38, 0.0),
            _ => Complex::ZERO,
        };
    }
    let k = harmonic as f64;
    match shape {
        Shape::Saw => Complex::new(0.0, 1.0 / (std::f64::consts::PI * k)),
        Shape::Square | Shape::Pulse => {
            let width = f64::from(shape.pulse_width());
            let angle = -std::f64::consts::TAU * k * width;
            (Complex::new(1.0, 0.0) - Complex::from_polar(1.0, angle))
                / Complex::new(0.0, std::f64::consts::PI * k)
        }
        Shape::Triangle => {
            let coefficient = if harmonic % 2 == 1 {
                -4.0 / (std::f64::consts::PI.powi(2) * k * k)
            } else {
                0.0
            };
            Complex::new(coefficient, 0.0)
        }
        Shape::Custom => unreachable!("custom coefficients are transformed from its evaluator"),
    }
}

fn custom_coefficients(curve: WaveCurveRt) -> Vec<Complex> {
    let mut samples = (0..REFERENCE_SAMPLES)
        .map(|index| {
            Complex::new(
                f64::from(curve.eval(index as f32 / REFERENCE_SAMPLES as f32)),
                0.0,
            )
        })
        .collect::<Vec<_>>();
    fft(&mut samples, false);
    for coefficient in &mut samples {
        *coefficient /= REFERENCE_SAMPLES as f64;
    }
    samples
}

fn raw_shape(shape: Shape, phase: f32) -> f32 {
    match shape {
        Shape::Saw => phase.mul_add(2.0, -1.0),
        Shape::Square | Shape::Pulse => {
            if phase < shape.pulse_width() {
                1.0
            } else {
                -1.0
            }
        }
        Shape::Triangle => 1.0 - 4.0 * (phase - 0.5).abs(),
        Shape::Custom => unreachable!("custom uses its compiled evaluator"),
    }
}

fn warped_coefficients(
    shape: Shape,
    phase_step: f32,
    mode: PhaseWarpMode,
    amount: f32,
) -> Vec<Complex> {
    let mut samples = (0..REFERENCE_SAMPLES)
        .map(|index| {
            let phase = index as f32 / REFERENCE_SAMPLES as f32;
            let warped = warp_phase_position_scalar(phase, phase_step, mode, amount);
            Complex::new(f64::from(raw_shape(shape, warped)), 0.0)
        })
        .collect::<Vec<_>>();
    fft(&mut samples, false);
    for coefficient in &mut samples {
        *coefficient /= REFERENCE_SAMPLES as f64;
    }
    samples
}

fn reference(
    shape: Shape,
    period: usize,
    samples: usize,
    custom: &[Complex],
) -> (Vec<f64>, Vec<Complex>) {
    let cycles = samples / period;
    let harmonic_count = (period - 1) / 2;
    let coefficient = |harmonic| {
        if matches!(shape, Shape::Custom) {
            custom[harmonic]
        } else {
            analytic_coefficient(shape, harmonic)
        }
    };
    let mut coefficients = vec![Complex::ZERO; samples];
    coefficients[0] = coefficient(0);
    for harmonic in 1..=harmonic_count {
        let bin = harmonic * cycles;
        coefficients[bin] = coefficient(harmonic);
        coefficients[samples - bin] = coefficient(harmonic).conj();
    }
    let mut spectrum = coefficients
        .iter()
        .map(|coefficient| *coefficient * samples as f64)
        .collect::<Vec<_>>();
    fft(&mut spectrum, true);
    (
        spectrum.into_iter().map(|sample| sample.re).collect(),
        coefficients,
    )
}

fn aligned(candidate: &[f64], reference: &[f64], period: usize) -> (f64, Vec<f64>) {
    let mut best = (0, f64::INFINITY);
    let search = period as isize / 2;
    for lag in -search..=search {
        let error = candidate
            .iter()
            .enumerate()
            .map(|(index, _)| {
                let shifted = (index as isize + lag).rem_euclid(candidate.len() as isize) as usize;
                (candidate[shifted] - reference[index]).powi(2)
            })
            .sum::<f64>();
        if error < best.1 {
            best = (lag, error);
        }
    }
    let mut fractional_best = (best.0 as f64, best.1);
    for offset in -16..=16 {
        let lag = best.0 as f64 + f64::from(offset) / 16.0;
        let integer = lag.floor() as isize;
        let fraction = lag - integer as f64;
        let error = candidate
            .iter()
            .enumerate()
            .map(|(index, _)| {
                let first =
                    (index as isize + integer).rem_euclid(candidate.len() as isize) as usize;
                let second = (first + 1) % candidate.len();
                let shifted =
                    (candidate[second] - candidate[first]).mul_add(fraction, candidate[first]);
                (shifted - reference[index]).powi(2)
            })
            .sum::<f64>();
        if error < fractional_best.1 {
            fractional_best = (lag, error);
        }
    }
    let mut spectrum = candidate
        .iter()
        .map(|sample| Complex::new(*sample, 0.0))
        .collect::<Vec<_>>();
    fft(&mut spectrum, false);
    let length = spectrum.len();
    for (bin, value) in spectrum.iter_mut().enumerate() {
        let signed_bin = if bin <= length / 2 {
            bin as f64
        } else {
            bin as f64 - length as f64
        };
        let angle = std::f64::consts::TAU * signed_bin * fractional_best.0 / length as f64;
        *value *= Complex::from_polar(1.0, angle);
    }
    fft(&mut spectrum, true);
    (
        fractional_best.0,
        spectrum.into_iter().map(|sample| sample.re).collect(),
    )
}

fn db_ratio(numerator: f64, denominator: f64) -> f64 {
    10.0 * (numerator.max(f64::MIN_POSITIVE) / denominator.max(f64::MIN_POSITIVE)).log10()
}

fn report_case(
    shape: Shape,
    sample_rate: f64,
    period: usize,
    factor: u8,
    curve: WaveCurveRt,
    custom: &[Complex],
) {
    let samples = period * 32;
    let shipping = render_shipping(shape, period, samples, factor, curve);
    let (reference, reference_bins) = reference(shape, period, samples, custom);
    let (lag, shipping) = aligned(&shipping, &reference, period);
    let error = shipping
        .iter()
        .zip(&reference)
        .map(|(actual, expected)| actual - expected)
        .collect::<Vec<_>>();
    let reference_energy = reference.iter().map(|value| value * value).sum::<f64>();
    let error_energy = error.iter().map(|value| value * value).sum::<f64>();
    let curve_rms = (error_energy / samples as f64).sqrt();
    let curve_max = error.iter().copied().map(f64::abs).fold(0.0, f64::max);
    let dc = shipping.iter().sum::<f64>() / samples as f64;
    let gain = (shipping.iter().map(|value| value * value).sum::<f64>() / reference_energy).sqrt();

    let mut shipping_bins = shipping
        .iter()
        .map(|sample| Complex::new(*sample, 0.0))
        .collect::<Vec<_>>();
    fft(&mut shipping_bins, false);
    for bin in &mut shipping_bins {
        *bin /= samples as f64;
    }
    let cycles = samples / period;
    let mut harmonic_error = 0.0;
    let mut harmonic_energy = 0.0;
    let mut max_harmonic_db = 0.0_f64;
    let fundamental = reference_bins[cycles].norm();
    for harmonic in 1..=(period - 1) / 2 {
        let bin = harmonic * cycles;
        let wanted = reference_bins[bin].norm();
        let actual = shipping_bins[bin].norm();
        harmonic_error += (actual - wanted).powi(2);
        harmonic_energy += wanted * wanted;
        if wanted > fundamental * 1.0e-3 {
            max_harmonic_db = max_harmonic_db
                .max((20.0 * (actual / wanted).max(f64::MIN_POSITIVE).log10()).abs());
        }
    }

    let mut boundary_step = 0.0_f64;
    let mut global_step = 0.0_f64;
    for index in 1..samples {
        let step = (error[index] - error[index - 1]).abs();
        global_step = global_step.max(step);
        if index % period <= 2 || index % period >= period.saturating_sub(2) {
            boundary_step = boundary_step.max(step);
        }
    }
    println!(
        "quality,wave={},factor={factor}x,sample_rate={sample_rate:.0},frequency_hz={:.6},period={period},lag_samples={lag:.4},curve_rms={curve_rms:.9},curve_max={curve_max:.9},wanted_amp_error_db={:.3},wanted_amp_max_db={max_harmonic_db:.3},alias_error_db={:.3},dc={dc:.9},gain={gain:.9},boundary_residual_step={boundary_step:.9},global_residual_step={global_step:.9}",
        shape.name(),
        sample_rate / period as f64,
        db_ratio(harmonic_error, harmonic_energy),
        db_ratio(error_energy, reference_energy),
    );
}

const fn warp_name(mode: PhaseWarpMode) -> &'static str {
    match mode {
        PhaseWarpMode::None => "none",
        PhaseWarpMode::Pwm => "pwm",
        PhaseWarpMode::PhaseBend => "phase_bend",
        PhaseWarpMode::Harmonic => "harmonic",
    }
}

fn report_warp_case(
    shape: Shape,
    sample_rate: f64,
    period: usize,
    factor: u8,
    mode: PhaseWarpMode,
    amount: f32,
) {
    let samples = period * 32;
    let phase_step = 1.0 / (period * usize::from(factor)) as f32;
    let coefficients = warped_coefficients(shape, phase_step, mode, amount);
    let shipping = render_shipping_warped(shape, period, samples, factor, mode, amount);
    let (reference, reference_bins) = reference(Shape::Custom, period, samples, &coefficients);
    let (lag, shipping) = aligned(&shipping, &reference, period);
    let error = shipping
        .iter()
        .zip(&reference)
        .map(|(actual, expected)| actual - expected)
        .collect::<Vec<_>>();
    let reference_energy = reference.iter().map(|value| value * value).sum::<f64>();
    let error_energy = error.iter().map(|value| value * value).sum::<f64>();
    let curve_rms = (error_energy / samples as f64).sqrt();
    let curve_max = error.iter().copied().map(f64::abs).fold(0.0, f64::max);
    let dc = shipping.iter().sum::<f64>() / samples as f64;
    let gain = (shipping.iter().map(|value| value * value).sum::<f64>() / reference_energy).sqrt();

    let mut shipping_bins = shipping
        .iter()
        .map(|sample| Complex::new(*sample, 0.0))
        .collect::<Vec<_>>();
    fft(&mut shipping_bins, false);
    for bin in &mut shipping_bins {
        *bin /= samples as f64;
    }
    let cycles = samples / period;
    let mut harmonic_error = 0.0;
    let mut harmonic_energy = 0.0;
    for harmonic in 1..=(period - 1) / 2 {
        let bin = harmonic * cycles;
        harmonic_error += (shipping_bins[bin].norm() - reference_bins[bin].norm()).powi(2);
        harmonic_energy += reference_bins[bin].norm_sqr();
    }

    let mut boundary_step = 0.0_f64;
    let mut global_step = 0.0_f64;
    for index in 1..samples {
        let step = (error[index] - error[index - 1]).abs();
        global_step = global_step.max(step);
        if index % period <= 2 || index % period >= period.saturating_sub(2) {
            boundary_step = boundary_step.max(step);
        }
    }
    println!(
        "warp_quality,wave={},mode={},amount={amount:.2},factor={factor}x,sample_rate={sample_rate:.0},frequency_hz={:.6},period={period},lag_samples={lag:.4},curve_rms={curve_rms:.9},curve_max={curve_max:.9},wanted_amp_error_db={:.3},alias_error_db={:.3},dc={dc:.9},gain={gain:.9},boundary_residual_step={boundary_step:.9},global_residual_step={global_step:.9}",
        shape.name(),
        warp_name(mode),
        sample_rate / period as f64,
        db_ratio(harmonic_error, harmonic_energy),
        db_ratio(error_energy, reference_energy),
    );
}

fn report_cpu(shape: Shape, factor: u8, curve: WaveCurveRt) {
    const SAMPLES: usize = 2_000_000;
    const REPEATS: usize = 9;
    let mut measurements = Vec::with_capacity(REPEATS);
    let mut checksum = 0.0_f32;
    for _ in 0..REPEATS {
        let mut oscillator = VaOscillator::default();
        let mut oversampler = StereoOversampler::default();
        oversampler.reset(factor);
        oversampler.set_spline_correction_immediate(true);
        let step = 440.0 / (48_000.0 * f32::from(factor));
        for _ in 0..4096 {
            for _ in 0..factor {
                let sample = black_box(next_shipping(&mut oscillator, shape, step, curve));
                oversampler.push(sample, sample);
            }
            checksum += black_box(oversampler.output().0);
        }
        let started = Instant::now();
        for _ in 0..SAMPLES {
            for _ in 0..factor {
                let sample = black_box(next_shipping(&mut oscillator, shape, step, curve));
                oversampler.push(sample, sample);
            }
            checksum += black_box(oversampler.output().0);
        }
        measurements.push(started.elapsed().as_nanos() as f64 / SAMPLES as f64);
    }
    measurements.sort_by(f64::total_cmp);
    let mean = measurements.iter().sum::<f64>() / REPEATS as f64;
    let standard_deviation = (measurements
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / REPEATS as f64)
        .sqrt();
    println!(
        "cpu,wave={},factor={factor}x,rate=48000,frequency_hz=440,samples={SAMPLES},repeats={REPEATS},median_ns_per_sample={:.3},min={:.3},max={:.3},stddev={standard_deviation:.3},checksum={checksum:.9}",
        shape.name(),
        measurements[REPEATS / 2],
        measurements[0],
        measurements[REPEATS - 1],
    );
}

fn report_warp_cpu(shape: Shape, factor: u8, mode: PhaseWarpMode, amount: f32) {
    const SAMPLES: usize = 500_000;
    const REPEATS: usize = 5;
    let step = 440.0 / (48_000.0 * f32::from(factor));
    let mut baseline = Vec::with_capacity(REPEATS);
    let mut warped = Vec::with_capacity(REPEATS);
    let mut prepared = Vec::with_capacity(REPEATS);
    let mut checksum = 0.0_f32;
    let pulse_edge = warped_pulse_edge_scalar(step, shape.pulse_width(), mode, amount);
    let mut auto = VaOscillator::default();
    let mut precomputed = VaOscillator::default();
    for _ in 0..8_192 {
        let expected = auto.generate_shape_step_warped(
            shape.shape(),
            step,
            shape.pulse_width(),
            Antialiasing::SplineOptimized,
            mode,
            amount,
        );
        let actual = precomputed.generate_shape_step_warped_with_edge(
            shape.shape(),
            step,
            shape.pulse_width(),
            Antialiasing::SplineOptimized,
            mode,
            amount,
            pulse_edge,
        );
        assert_eq!(actual.to_bits(), expected.to_bits());
    }
    for _ in 0..REPEATS {
        let mut oscillator = VaOscillator::default();
        let mut oversampler = StereoOversampler::default();
        oversampler.reset(factor);
        oversampler.set_spline_correction_immediate(true);
        let started = Instant::now();
        for _ in 0..SAMPLES {
            for _ in 0..factor {
                let sample = oscillator.generate_shape_step(
                    shape.shape(),
                    step,
                    shape.pulse_width(),
                    Antialiasing::SplineOptimized,
                );
                oversampler.push(sample, sample);
            }
            checksum += black_box(oversampler.output().0);
        }
        baseline.push(started.elapsed().as_nanos() as f64 / SAMPLES as f64);

        let mut oscillator = VaOscillator::default();
        oversampler.reset(factor);
        let started = Instant::now();
        for _ in 0..SAMPLES {
            for _ in 0..factor {
                let sample = oscillator.generate_shape_step_warped(
                    shape.shape(),
                    step,
                    shape.pulse_width(),
                    Antialiasing::SplineOptimized,
                    mode,
                    amount,
                );
                oversampler.push(sample, sample);
            }
            checksum += black_box(oversampler.output().0);
        }
        warped.push(started.elapsed().as_nanos() as f64 / SAMPLES as f64);

        let mut oscillator = VaOscillator::default();
        oversampler.reset(factor);
        let started = Instant::now();
        for _ in 0..SAMPLES {
            for _ in 0..factor {
                let sample = oscillator.generate_shape_step_warped_with_edge(
                    shape.shape(),
                    step,
                    shape.pulse_width(),
                    Antialiasing::SplineOptimized,
                    mode,
                    amount,
                    pulse_edge,
                );
                oversampler.push(sample, sample);
            }
            checksum += black_box(oversampler.output().0);
        }
        prepared.push(started.elapsed().as_nanos() as f64 / SAMPLES as f64);
    }
    baseline.sort_by(f64::total_cmp);
    warped.sort_by(f64::total_cmp);
    prepared.sort_by(f64::total_cmp);
    println!(
        "warp_cpu,wave={},mode={},amount={amount:.2},factor={factor}x,baseline_ns={:.3},warped_ns={:.3},prepared_ns={:.3},warped_ratio={:.3},prepared_ratio={:.3},checksum={checksum:.9}",
        shape.name(),
        warp_name(mode),
        baseline[REPEATS / 2],
        warped[REPEATS / 2],
        prepared[REPEATS / 2],
        warped[REPEATS / 2] / baseline[REPEATS / 2],
        prepared[REPEATS / 2] / baseline[REPEATS / 2],
    );
}

#[test]
#[ignore = "manual release-mode VA quality and CPU experiment"]
fn shipping_1x_va_quality_and_cpu_report() {
    let curve = drawn_curve();
    let custom = custom_coefficients(curve);
    println!(
        "contract,shipping=VaOscillator::generate_*_step+StereoOversampler,antialiasing=SplineOptimized,factors=1x|2x,reference=ideal_harmonic_projection,custom_source_samples={REFERENCE_SAMPLES}"
    );
    for shape in Shape::ALL {
        for factor in [1, 2] {
            for sample_rate in SAMPLE_RATES {
                for target in TARGET_FREQUENCIES {
                    let period = (sample_rate / target).round().max(3.0) as usize;
                    report_case(shape, sample_rate, period, factor, curve, &custom);
                }
            }
            report_cpu(shape, factor, curve);
        }
    }
}

#[test]
#[ignore = "manual release-mode static phase-warp quality and CPU experiment"]
fn shipping_static_phase_warp_quality_and_cpu_report() {
    const SHAPES: [Shape; 4] = [Shape::Saw, Shape::Square, Shape::Pulse, Shape::Triangle];
    const MODES: [PhaseWarpMode; 3] = [
        PhaseWarpMode::Pwm,
        PhaseWarpMode::PhaseBend,
        PhaseWarpMode::Harmonic,
    ];
    println!(
        "warp_contract,shipping=VaOscillator::generate_shape_step_warped+StereoOversampler,reference=ideal_harmonic_projection_of_continuous_warped_curve,factors=1x|2x"
    );
    for shape in SHAPES {
        for mode in MODES {
            for amount in [0.5, 1.0] {
                for factor in [1, 2] {
                    for target in [880.0, 7_040.0] {
                        let period = (48_000.0_f64 / target).round().max(3.0) as usize;
                        report_warp_case(shape, 48_000.0, period, factor, mode, amount);
                    }
                }
            }
            for factor in [1, 2] {
                report_warp_cpu(shape, factor, mode, 1.0);
            }
        }
    }
}

#[inline(always)]
fn probe_support3_estrin(phase: f32x8, step: f32x8, inverse_step: f32x8) -> f32x8 {
    let raw = phase.mul_add(f32x8::splat(2.0), -f32x8::ONE);
    raw - probe_support3_edge(phase, step, inverse_step)
}

#[inline(always)]
fn probe_support3_edge(phase: f32x8, step: f32x8, inverse_step: f32x8) -> f32x8 {
    let support = step * f32x8::splat(3.0);
    let event = phase.cmp_lt(support) | phase.cmp_gt(f32x8::ONE - support);
    if !event.any() {
        return f32x8::ZERO;
    }
    let edge = phase
        .cmp_lt(f32x8::splat(0.5))
        .blend(phase, phase - f32x8::ONE);
    let position = edge * inverse_step;
    let distance = position.abs();
    let d2 = distance * distance;
    let d4 = d2 * d2;
    let p0 = f32x8::splat(0.871_725_8).mul_add(distance, f32x8::splat(-0.5));
    let p1 = f32x8::splat(-2.108_1).mul_add(distance, f32x8::splat(0.767_631_7));
    let p2 = f32x8::splat(-0.420_318_2).mul_add(distance, f32x8::splat(1.423_189_2));
    let p3 = f32x8::splat(-0.002_115_283_7).mul_add(distance, f32x8::splat(0.054_019_138));
    let residual = p3.mul_add(d2, p2).mul_add(d4, p1.mul_add(d2, p0));
    let residual = distance
        .cmp_lt(f32x8::splat(3.0))
        .blend(residual, f32x8::ZERO);
    let residual = position.cmp_lt(f32x8::ZERO).blend(-residual, residual);
    event.blend(residual * f32x8::splat(2.0), f32x8::ZERO)
}

#[unsafe(no_mangle)]
#[inline(never)]
pub extern "C" fn kurv_probe_current_x8_blocks(blocks: u32, step: f32) -> f32 {
    const BLOCK: usize = 64;
    crate::performance::select_detected_backend_for_probe();
    let mut oscillators = [VaOscillator::default(); 8];
    let step = f32x8::splat(step);
    let mut left = [f32x8::ZERO; BLOCK];
    let mut right = [f32x8::ZERO; BLOCK];
    let mut checksum = 0.0;
    for _ in 0..blocks {
        accumulate_saw8_block_constant(
            &mut oscillators,
            step,
            f32x8::ONE,
            f32x8::ONE,
            &mut left,
            &mut right,
            Antialiasing::SplineOptimized,
        );
        checksum += left[BLOCK - 1].to_array().into_iter().sum::<f32>();
    }
    checksum
}

#[unsafe(no_mangle)]
#[inline(never)]
pub extern "C" fn kurv_probe_support3_x8_blocks(blocks: u32, step: f32) -> f32 {
    const BLOCK: usize = 64;
    let mut phase = f32x8::from([0.073, 0.173, 0.271, 0.389, 0.491, 0.593, 0.697, 0.811]);
    let step = f32x8::splat(step);
    // This probe models stable-note eligibility. Any future production path must
    // republish the reciprocal atomically with every step change.
    let inverse_step = f32x8::ONE / step;
    let mut left = [f32x8::ZERO; BLOCK];
    let mut right = [f32x8::ZERO; BLOCK];
    let mut checksum = 0.0;
    for _ in 0..blocks {
        for frame in 0..BLOCK {
            let sample = probe_support3_estrin(phase, step, inverse_step);
            left[frame] += sample;
            right[frame] += sample;
            let next = phase + step;
            phase = next.cmp_lt(f32x8::ONE).blend(next, next - f32x8::ONE);
        }
        checksum += left[BLOCK - 1].to_array().into_iter().sum::<f32>();
    }
    checksum + right[0].to_array()[0]
}

#[test]
#[ignore = "manual symbol-preserving native canonical x8 profiling probe"]
fn canonical_x8_symbol_probe() {
    let blocks = std::env::var("KURV_ASM_BLOCKS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(2_000_000);
    let step = std::env::var("KURV_ASM_HZ")
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .unwrap_or(7040.0)
        / 48_000.0;
    let checksum = match std::env::var("KURV_ASM_PROBE").as_deref() {
        Ok("support3") => kurv_probe_support3_x8_blocks(blocks, step),
        _ => kurv_probe_current_x8_blocks(blocks, step),
    };
    println!("canonical_x8_symbol_probe,blocks={blocks},step={step:.9},checksum={checksum:.9}");
}

#[test]
fn support3_reciprocal_matches_division() {
    for frequency in [110.0_f32, 880.0, 3_520.0, 7_040.0, 12_000.0] {
        let step = f32x8::splat(frequency / 48_000.0);
        let inverse_step = f32x8::ONE / step;
        for index in 0..4096 {
            let phase = f32x8::splat(index as f32 / 4096.0);
            let edge = phase
                .cmp_lt(f32x8::splat(0.5))
                .blend(phase, phase - f32x8::ONE);
            let multiplied = (edge * inverse_step).to_array()[0];
            let divided = (edge / step).to_array()[0];
            assert!((multiplied - divided).abs() <= 2.0 * f32::EPSILON * divided.abs().max(1.0));
        }
    }
}
