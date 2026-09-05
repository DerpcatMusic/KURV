use std::hint::black_box;
use std::time::Instant;

use truce_simd::simd::f32x8;
use wide::{CmpGt, CmpLt};

use crate::dsp::{Complex, fft};
use crate::oversampling::StereoOversampler;
use crate::wave_curve::{WaveCurveData, WaveCurveRt, WaveKnot};

use super::warp::{warp_phase_position_scalar, warp_phase_scalar, warped_pulse_edge_scalar};
use super::{
    Antialiasing, PhaseWarpMode, VaOscillator, accumulate_saw8_block_constant,
    accumulate_shape8_block_constant,
};

const REFERENCE_SAMPLES: usize = 65_536;
const TARGET_FREQUENCIES: [f64; 3] = [110.0, 880.0, 7_040.0];
const SAMPLE_RATES: [f64; 3] = [44_100.0, 48_000.0, 96_000.0];

#[derive(Clone, Copy)]
pub(super) enum Shape {
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

    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::Saw => "saw",
            Self::Square => "square",
            Self::Pulse => "pulse31",
            Self::Triangle => "triangle",
            Self::Custom => "drawn",
        }
    }

    pub(super) const fn shape(self) -> f32 {
        match self {
            Self::Saw => 2.0,
            Self::Square | Self::Pulse => 3.0,
            Self::Triangle => 1.0,
            Self::Custom => 0.0,
        }
    }

    pub(super) const fn pulse_width(self) -> f32 {
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

pub(super) fn render_shipping(
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

pub(super) fn reference(
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

pub(super) fn aligned(candidate: &[f64], reference: &[f64], period: usize) -> (f64, Vec<f64>) {
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
    (
        fractional_best.0,
        circular_phase_shift(candidate, fractional_best.0),
    )
}

#[derive(Clone, Copy)]
struct CycleShapeMetrics {
    unaligned_rms: f64,
    unaligned_peak: f64,
    phase_offset_samples: f64,
    dc_offset: f64,
    gain: f64,
    residual_rms: f64,
    residual_peak: f64,
}

fn circular_phase_shift(signal: &[f64], shift_samples: f64) -> Vec<f64> {
    let mut spectrum = signal
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
        let angle = std::f64::consts::TAU * signed_bin * shift_samples / length as f64;
        *value *= Complex::from_polar(1.0, angle);
    }
    fft(&mut spectrum, true);
    spectrum.into_iter().map(|sample| sample.re).collect()
}

fn cycle_shape_metrics(candidate: &[f64], ideal: &[f64]) -> CycleShapeMetrics {
    assert_eq!(candidate.len(), ideal.len());
    assert!(!ideal.is_empty());
    let length = ideal.len() as f64;
    let ideal_mean = ideal.iter().sum::<f64>() / length;
    let ideal_energy = ideal
        .iter()
        .map(|sample| (sample - ideal_mean).powi(2))
        .sum::<f64>();
    assert!(ideal_energy > f64::EPSILON);
    let unaligned_rms = (candidate
        .iter()
        .zip(ideal)
        .map(|(actual, expected)| (actual - expected).powi(2))
        .sum::<f64>()
        / length)
        .sqrt();
    let unaligned_peak = candidate
        .iter()
        .zip(ideal)
        .map(|(actual, expected)| (actual - expected).abs())
        .fold(0.0, f64::max);

    let fit = |correction| {
        let shifted = circular_phase_shift(candidate, correction);
        let shifted_mean = shifted.iter().sum::<f64>() / length;
        let gain = shifted
            .iter()
            .zip(ideal)
            .map(|(actual, expected)| (actual - shifted_mean) * (expected - ideal_mean))
            .sum::<f64>()
            / ideal_energy;
        if gain.abs() <= f64::EPSILON {
            return (f64::INFINITY, correction, 0.0, gain, f64::INFINITY);
        }
        let dc_offset = shifted_mean - gain * ideal_mean;
        let (energy, peak) = shifted
            .iter()
            .zip(ideal)
            .map(|(actual, expected)| (actual - dc_offset) / gain - expected)
            .fold((0.0_f64, 0.0_f64), |(energy, peak), error| {
                (energy + error * error, peak.max(error.abs()))
            });
        (energy, correction, dc_offset, gain, peak)
    };
    let mut best = fit(0.0);
    for phase_step in -128..=128 {
        let correction = f64::from(phase_step) / 256.0;
        let trial = fit(correction);
        if trial.0 < best.0 {
            best = trial;
        }
    }
    let cell = 1.0 / 256.0;
    let mut low = (best.1 - cell).max(-0.5);
    let mut high = (best.1 + cell).min(0.5);
    for _ in 0..32 {
        let left = (2.0 * low + high) / 3.0;
        let right = (low + 2.0 * high) / 3.0;
        if fit(left).0 <= fit(right).0 {
            high = right;
        } else {
            low = left;
        }
    }
    let refined = fit((low + high) * 0.5);
    if refined.0 < best.0 {
        best = refined;
    }
    CycleShapeMetrics {
        unaligned_rms,
        unaligned_peak,
        phase_offset_samples: -best.1,
        dc_offset: best.2,
        gain: best.3,
        residual_rms: (best.0 / length).sqrt(),
        residual_peak: best.4,
    }
}

pub(super) fn db_ratio(numerator: f64, denominator: f64) -> f64 {
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
            pulse_edge,
            |phase| warp_phase_scalar(phase, step, mode, amount),
        );
        assert_eq!(actual.to_bits(), expected.to_bits());
    }
    for _ in 0..REPEATS {
        let mut oscillator = VaOscillator::default();
        let mut oversampler = StereoOversampler::default();
        oversampler.reset(factor);
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
                    pulse_edge,
                    |phase| warp_phase_scalar(phase, step, mode, amount),
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

#[derive(Clone, Copy)]
struct ProjectionBank {
    planes: [[f32; 8]; 6],
}

struct ProjectionSet {
    saw: [ProjectionBank; 6],
    triangle: [ProjectionBank; 6],
}

fn exact_capped_projection(shape: Shape, cap: usize, phase: f64, width: f64) -> f64 {
    match shape {
        Shape::Saw => (1..=cap)
            .map(|harmonic| {
                let harmonic = harmonic as f64;
                -2.0 * (std::f64::consts::TAU * harmonic * phase).sin()
                    / (std::f64::consts::PI * harmonic)
            })
            .sum(),
        Shape::Square | Shape::Pulse => {
            2.0 * width - 1.0
                + (1..=cap)
                    .map(|harmonic| {
                        let harmonic = harmonic as f64;
                        let angle = std::f64::consts::TAU * harmonic * phase;
                        let edge = std::f64::consts::TAU * harmonic * width;
                        2.0 * ((edge.sin() / (std::f64::consts::PI * harmonic)) * angle.cos()
                            + ((1.0 - edge.cos()) / (std::f64::consts::PI * harmonic))
                                * angle.sin())
                    })
                    .sum::<f64>()
        }
        Shape::Triangle => (1..=cap)
            .filter(|harmonic| harmonic % 2 == 1)
            .map(|harmonic| {
                let harmonic = harmonic as f64;
                -8.0 * (std::f64::consts::TAU * harmonic * phase).cos()
                    / (std::f64::consts::PI.powi(2) * harmonic * harmonic)
            })
            .sum(),
        Shape::Custom => unreachable!("custom is outside the canonical projection probe"),
    }
}

fn solve_projection_fit(mut matrix: [[f64; 7]; 6]) -> [f64; 6] {
    for pivot in 0..6 {
        let best = (pivot..6)
            .max_by(|&left, &right| {
                matrix[left][pivot]
                    .abs()
                    .total_cmp(&matrix[right][pivot].abs())
            })
            .expect("six-row fit has a pivot");
        matrix.swap(pivot, best);
        let scale = matrix[pivot][pivot];
        for column in pivot..=6 {
            matrix[pivot][column] /= scale;
        }
        for row in 0..6 {
            if row == pivot {
                continue;
            }
            let scale = matrix[row][pivot];
            for column in pivot..=6 {
                matrix[row][column] -= scale * matrix[pivot][column];
            }
        }
    }
    std::array::from_fn(|index| matrix[index][6])
}

fn fit_projection_bank(shape: Shape, cap: usize) -> ProjectionBank {
    let mut planes = [[0.0; 8]; 6];
    for piece in 0..8 {
        let mut normal = [[0.0; 7]; 6];
        for sample in 0..512 {
            let t = (sample as f64 + 0.5) / 512.0;
            let value = exact_capped_projection(
                shape,
                cap,
                (piece as f64 + t) / 8.0,
                f64::from(shape.pulse_width()),
            );
            let powers: [f64; 11] = std::array::from_fn(|order| t.powi(order as i32));
            for row in 0..6 {
                for column in 0..6 {
                    normal[row][column] += powers[row + column];
                }
                normal[row][6] += value * powers[row];
            }
        }
        let fitted = solve_projection_fit(normal);
        for plane in 0..6 {
            planes[plane][piece] = fitted[5 - plane] as f32;
        }
    }
    ProjectionBank { planes }
}

impl ProjectionSet {
    fn fit() -> Self {
        Self {
            saw: std::array::from_fn(|cap| fit_projection_bank(Shape::Saw, cap + 1)),
            triangle: std::array::from_fn(|cap| fit_projection_bank(Shape::Triangle, cap + 1)),
        }
    }

    fn bank(&self, shape: Shape, cap: usize) -> &ProjectionBank {
        match shape {
            Shape::Triangle => &self.triangle[cap - 1],
            Shape::Saw | Shape::Square | Shape::Pulse => &self.saw[cap - 1],
            Shape::Custom => unreachable!("custom is outside the canonical projection probe"),
        }
    }
}

#[inline(always)]
fn eval_projection_scalar(bank: &ProjectionBank, phase: f32) -> f32 {
    let position = phase * 8.0;
    let segment = (position as usize).min(7);
    let t = position - segment as f32;
    bank.planes[1..]
        .iter()
        .fold(bank.planes[0][segment], |value, plane| {
            value.mul_add(t, plane[segment])
        })
}

#[cfg(all(
    target_arch = "x86_64",
    target_feature = "avx2",
    target_feature = "fma"
))]
#[inline(always)]
fn eval_projection8(bank: &ProjectionBank, phase: f32x8) -> f32x8 {
    use core::arch::x86_64::*;

    let phase: [f32; 8] = phase.into();
    let mut output = [0.0; 8];
    // SAFETY: all loads contain eight f32 values; every permutation index is
    // clamped to the same eight-value plane before it is used.
    unsafe {
        let phase = _mm256_loadu_ps(phase.as_ptr());
        let position = _mm256_mul_ps(phase, _mm256_set1_ps(8.0));
        let segment = _mm256_min_epi32(
            _mm256_max_epi32(_mm256_cvttps_epi32(position), _mm256_setzero_si256()),
            _mm256_set1_epi32(7),
        );
        let t = _mm256_sub_ps(position, _mm256_cvtepi32_ps(segment));
        let mut value = _mm256_permutevar8x32_ps(_mm256_loadu_ps(bank.planes[0].as_ptr()), segment);
        for plane in &bank.planes[1..] {
            let coefficient = _mm256_permutevar8x32_ps(_mm256_loadu_ps(plane.as_ptr()), segment);
            value = _mm256_fmadd_ps(value, t, coefficient);
        }
        _mm256_storeu_ps(output.as_mut_ptr(), value);
    }
    f32x8::from(output)
}

#[cfg(not(all(
    target_arch = "x86_64",
    target_feature = "avx2",
    target_feature = "fma"
)))]
#[inline(always)]
fn eval_projection8(bank: &ProjectionBank, phase: f32x8) -> f32x8 {
    f32x8::from(<[f32; 8]>::from(phase).map(|phase| eval_projection_scalar(bank, phase)))
}

#[inline(always)]
fn projection_cap(step: f32) -> usize {
    if step < 0.5 / 6.0 {
        6
    } else if step < 0.5 / 5.0 {
        5
    } else if step < 0.5 / 4.0 {
        4
    } else if step < 0.5 / 3.0 {
        3
    } else if step < 0.5 / 2.0 {
        2
    } else {
        1
    }
}

#[inline(always)]
fn eval_capped_projection_scalar(
    set: &ProjectionSet,
    shape: Shape,
    cap: usize,
    phase: f32,
    width: f32,
) -> f32 {
    let bank = set.bank(shape, cap);
    if matches!(shape, Shape::Square | Shape::Pulse) {
        eval_projection_scalar(bank, (phase + 1.0 - width).fract())
            - eval_projection_scalar(bank, phase)
            + 2.0 * width
            - 1.0
    } else {
        eval_projection_scalar(bank, phase)
    }
}

#[inline(always)]
fn eval_capped_projection8(
    set: &ProjectionSet,
    shape: Shape,
    cap: usize,
    phase: f32x8,
    width: f32,
) -> f32x8 {
    let bank = set.bank(shape, cap);
    if matches!(shape, Shape::Square | Shape::Pulse) {
        let shifted = super::antialias::wrap_phase8(phase + f32x8::splat(1.0 - width));
        eval_projection8(bank, shifted) - eval_projection8(bank, phase)
            + f32x8::splat(2.0 * width - 1.0)
    } else {
        eval_projection8(bank, phase)
    }
}

#[inline(never)]
fn accumulate_projection8_block<const SAMPLES: usize>(
    oscillators: &mut [VaOscillator],
    phase_step: f32x8,
    left_gain: f32x8,
    right_gain: f32x8,
    left: &mut [f32x8; SAMPLES],
    right: &mut [f32x8; SAMPLES],
    shape: Shape,
    set: &ProjectionSet,
) {
    let steps = phase_step.to_array();
    let caps = steps.map(projection_cap);
    let common_cap = caps.iter().all(|&cap| cap == caps[0]).then_some(caps[0]);
    let width = shape.pulse_width();
    let mut phase = f32x8::from(std::array::from_fn(|lane| oscillators[lane].phase));
    for frame in 0..SAMPLES {
        let sample = if let Some(cap) = common_cap {
            eval_capped_projection8(set, shape, cap, phase, width)
        } else {
            let phases = phase.to_array();
            f32x8::from(std::array::from_fn(|lane| {
                eval_capped_projection_scalar(set, shape, caps[lane], phases[lane], width)
            }))
        };
        left[frame] = sample.mul_add(left_gain, left[frame]);
        right[frame] = sample.mul_add(right_gain, right[frame]);
        let next = phase + phase_step;
        phase = next.cmp_lt(f32x8::ONE).blend(next, next - f32x8::ONE);
    }
    for (oscillator, phase) in oscillators.iter_mut().zip(phase.to_array()) {
        oscillator.phase = phase;
    }
}

struct ProjectionQuality {
    curve_rms: f64,
    curve_peak: f64,
    wanted_rms: f64,
    wanted_db: f64,
    alias_rms: f64,
    alias_db: f64,
    wrap_error: f64,
    seam_error: f64,
}

fn projection_quality(set: &ProjectionSet, shape: Shape, cap: usize) -> ProjectionQuality {
    const SAMPLES: usize = 65_536;
    let width = shape.pulse_width();
    let mut candidate = (0..SAMPLES)
        .map(|index| {
            Complex::new(
                f64::from(eval_capped_projection_scalar(
                    set,
                    shape,
                    cap,
                    index as f32 / SAMPLES as f32,
                    width,
                )),
                0.0,
            )
        })
        .collect::<Vec<_>>();
    let reference = (0..SAMPLES)
        .map(|index| {
            exact_capped_projection(shape, cap, index as f64 / SAMPLES as f64, f64::from(width))
        })
        .collect::<Vec<_>>();
    let reference_rms =
        (reference.iter().map(|sample| sample * sample).sum::<f64>() / SAMPLES as f64).sqrt();
    let curve_rms = (candidate
        .iter()
        .zip(&reference)
        .map(|(actual, expected)| (actual.re - expected).powi(2))
        .sum::<f64>()
        / SAMPLES as f64)
        .sqrt();
    let curve_peak = candidate
        .iter()
        .zip(&reference)
        .map(|(actual, expected)| (actual.re - expected).abs())
        .fold(0.0, f64::max);
    let wrap_error = ((candidate[0].re - candidate[SAMPLES - 1].re)
        - (reference[0] - reference[SAMPLES - 1]))
        .abs();
    let seam_error = (0..8)
        .map(|piece| {
            let after = piece * SAMPLES / 8;
            let before = (after + SAMPLES - 1) % SAMPLES;
            ((candidate[after].re - candidate[before].re) - (reference[after] - reference[before]))
                .abs()
        })
        .fold(0.0, f64::max);
    fft(&mut candidate, false);
    for value in &mut candidate {
        *value /= SAMPLES as f64;
    }
    let wanted_rms = (0..=cap)
        .map(|harmonic| {
            let expected = analytic_coefficient(shape, harmonic);
            let weight = if harmonic == 0 { 1.0 } else { 2.0 };
            weight * (candidate[harmonic] - expected).norm_sqr()
        })
        .sum::<f64>()
        .sqrt();
    let alias_rms = (2.0
        * candidate[cap + 1..SAMPLES / 2]
            .iter()
            .map(|value| value.norm_sqr())
            .sum::<f64>()
        + candidate[SAMPLES / 2].norm_sqr())
    .sqrt();
    let relative_db = |error: f64| {
        20.0 * (error.max(f64::MIN_POSITIVE) / reference_rms.max(f64::MIN_POSITIVE)).log10()
    };
    ProjectionQuality {
        curve_rms,
        curve_peak,
        wanted_rms,
        wanted_db: relative_db(wanted_rms),
        alias_rms,
        alias_db: relative_db(alias_rms),
        wrap_error,
        seam_error,
    }
}

struct TransitionStats {
    exact_peak: f64,
    candidate_peak: f64,
    error_peak: f64,
    error_rms: f64,
}

fn projection_transition_stats(
    mut pair: impl FnMut(f64) -> (f64, f64, f64, f64),
) -> TransitionStats {
    const PHASES: usize = 16_384;
    let mut exact_peak = 0.0_f64;
    let mut candidate_peak = 0.0_f64;
    let mut error_peak = 0.0_f64;
    let mut error_energy = 0.0_f64;
    for index in 0..PHASES {
        let (candidate_before, candidate_after, exact_before, exact_after) =
            pair(index as f64 / PHASES as f64);
        let candidate_step = candidate_after - candidate_before;
        let exact_step = exact_after - exact_before;
        let error = candidate_step - exact_step;
        exact_peak = exact_peak.max(exact_step.abs());
        candidate_peak = candidate_peak.max(candidate_step.abs());
        error_peak = error_peak.max(error.abs());
        error_energy += error * error;
    }
    TransitionStats {
        exact_peak,
        candidate_peak,
        error_peak,
        error_rms: (error_energy / PHASES as f64).sqrt(),
    }
}

fn print_projection_transition(kind: &str, detail: &str, stats: TransitionStats) {
    println!(
        "projection_transition,kind={kind},{detail},exact_peak={:.9},candidate_peak={:.9},error_peak={:.9},error_rms={:.9}",
        stats.exact_peak, stats.candidate_peak, stats.error_peak, stats.error_rms
    );
}

fn projection_x8_scalar_peak(set: &ProjectionSet) -> f32 {
    const SHAPES: [Shape; 4] = [Shape::Saw, Shape::Square, Shape::Pulse, Shape::Triangle];
    let mut peak = 0.0_f32;
    for shape in SHAPES {
        for cap in 1..=6 {
            for batch in 0..1_024 {
                let phases = std::array::from_fn(|lane| (batch * 8 + lane) as f32 / 8_192.0);
                let vector = eval_capped_projection8(
                    set,
                    shape,
                    cap,
                    f32x8::from(phases),
                    shape.pulse_width(),
                )
                .to_array();
                for lane in 0..8 {
                    let scalar = eval_capped_projection_scalar(
                        set,
                        shape,
                        cap,
                        phases[lane],
                        shape.pulse_width(),
                    );
                    peak = peak.max((vector[lane] - scalar).abs());
                }
            }
        }
    }
    peak
}

#[test]
#[ignore = "manual fixed-bank quality and transition report"]
fn piecewise_projection_quality_transition_report() {
    const SHAPES: [Shape; 4] = [Shape::Saw, Shape::Square, Shape::Pulse, Shape::Triangle];
    let set = ProjectionSet::fit();
    assert_eq!(std::mem::size_of::<ProjectionBank>(), 192);
    assert_eq!(std::mem::size_of::<ProjectionSet>(), 2_304);
    let x8_scalar_peak = projection_x8_scalar_peak(&set);
    assert!(x8_scalar_peak <= 2.0e-6);
    println!(
        "projection_contract,pieces=8,degree=5,caps=1..6,banks=12,bytes={},runtime_state_bytes=0,fit=offline_least_squares,x8_scalar_peak={x8_scalar_peak:.9}",
        std::mem::size_of::<ProjectionSet>(),
    );
    for shape in SHAPES {
        for cap in 1..=6 {
            let quality = projection_quality(&set, shape, cap);
            println!(
                "projection_quality,shape={},cap={cap},curve_rms={:.9},curve_peak={:.9},wanted_rms={:.9},wanted_error_db={:.3},alias_rms={:.9},alias_error_db={:.3},wrap_error={:.9},seam_error={:.9}",
                shape.name(),
                quality.curve_rms,
                quality.curve_peak,
                quality.wanted_rms,
                quality.wanted_db,
                quality.alias_rms,
                quality.alias_db,
                quality.wrap_error,
                quality.seam_error,
            );
        }
    }

    for shape in SHAPES {
        let width = shape.pulse_width();
        for (from, to) in [(6, 5), (5, 4), (4, 3), (3, 2), (2, 1)] {
            let stats = projection_transition_stats(|phase| {
                (
                    f64::from(eval_capped_projection_scalar(
                        &set,
                        shape,
                        from,
                        phase as f32,
                        width,
                    )),
                    f64::from(eval_capped_projection_scalar(
                        &set,
                        shape,
                        to,
                        phase as f32,
                        width,
                    )),
                    exact_capped_projection(shape, from, phase, f64::from(width)),
                    exact_capped_projection(shape, to, phase, f64::from(width)),
                )
            });
            print_projection_transition(
                "cap_switch",
                &format!("shape={},from={from},to={to}", shape.name()),
                stats,
            );

            let old_step = (0.5 / from as f64) * (1.0 - 1.0e-6);
            let stats = projection_transition_stats(|phase| {
                let next = (phase + old_step).fract();
                (
                    f64::from(eval_capped_projection_scalar(
                        &set,
                        shape,
                        from,
                        phase as f32,
                        width,
                    )),
                    f64::from(eval_capped_projection_scalar(
                        &set,
                        shape,
                        to,
                        next as f32,
                        width,
                    )),
                    exact_capped_projection(shape, from, phase, f64::from(width)),
                    exact_capped_projection(shape, to, next, f64::from(width)),
                )
            });
            print_projection_transition(
                "cap_adjacent",
                &format!("shape={},from={from},to={to}", shape.name()),
                stats,
            );
        }

        let old_step = 7_000.0 / 48_000.0;
        let stats = projection_transition_stats(|phase| {
            let next = (phase + old_step).fract();
            (
                f64::from(eval_capped_projection_scalar(
                    &set,
                    shape,
                    3,
                    phase as f32,
                    width,
                )),
                f64::from(eval_capped_projection_scalar(
                    &set,
                    shape,
                    3,
                    next as f32,
                    width,
                )),
                exact_capped_projection(shape, 3, phase, f64::from(width)),
                exact_capped_projection(shape, 3, next, f64::from(width)),
            )
        });
        print_projection_transition(
            "pitch_same_cap",
            &format!("shape={},from_hz=7000,to_hz=7500,cap=3", shape.name()),
            stats,
        );

        let stats = projection_transition_stats(|phase| {
            (
                f64::from(eval_capped_projection_scalar(
                    &set,
                    shape,
                    3,
                    phase as f32,
                    width,
                )),
                f64::from(eval_capped_projection_scalar(&set, shape, 3, 0.0, width)),
                exact_capped_projection(shape, 3, phase, f64::from(width)),
                exact_capped_projection(shape, 3, 0.0, f64::from(width)),
            )
        });
        print_projection_transition(
            "reset",
            &format!("shape={},cap=3,to_phase=0", shape.name()),
            stats,
        );
    }

    for to in [0.20_f32, 0.40, 0.50] {
        let step = 7_000.0 / 48_000.0;
        let stats = projection_transition_stats(|phase| {
            let next = (phase + step).fract();
            (
                f64::from(eval_capped_projection_scalar(
                    &set,
                    Shape::Pulse,
                    3,
                    phase as f32,
                    0.31,
                )),
                f64::from(eval_capped_projection_scalar(
                    &set,
                    Shape::Pulse,
                    3,
                    next as f32,
                    to,
                )),
                exact_capped_projection(Shape::Pulse, 3, phase, 0.31),
                exact_capped_projection(Shape::Pulse, 3, next, f64::from(to)),
            )
        });
        print_projection_transition(
            "width",
            &format!("shape=pulse31,from=0.31,to={to:.2},cap=3"),
            stats,
        );
    }
}

#[derive(Clone, Copy)]
struct ProjectionBenchStats {
    median: f64,
    min: f64,
    max: f64,
}

fn projection_bench_pair(
    mut current: impl FnMut() -> f32,
    mut candidate: impl FnMut() -> f32,
) -> (ProjectionBenchStats, ProjectionBenchStats) {
    const REPEATS: usize = 7;
    const BLOCKS: usize = 20_000;
    let mut current_times = [0.0; REPEATS];
    let mut candidate_times = [0.0; REPEATS];
    let time = |render: &mut dyn FnMut() -> f32| {
        for _ in 0..1_024 {
            black_box(render());
        }
        let started = Instant::now();
        let mut checksum = 0.0;
        for _ in 0..BLOCKS {
            checksum += black_box(render());
        }
        black_box(checksum);
        started.elapsed().as_nanos() as f64 / BLOCKS as f64
    };
    for repeat in 0..REPEATS {
        if repeat % 2 == 0 {
            current_times[repeat] = time(&mut current);
            candidate_times[repeat] = time(&mut candidate);
        } else {
            candidate_times[repeat] = time(&mut candidate);
            current_times[repeat] = time(&mut current);
        }
    }
    current_times.sort_by(f64::total_cmp);
    candidate_times.sort_by(f64::total_cmp);
    let summarize = |times: [f64; REPEATS]| ProjectionBenchStats {
        median: times[REPEATS / 2],
        min: times[0],
        max: times[REPEATS - 1],
    };
    (summarize(current_times), summarize(candidate_times))
}

fn report_projection_cpu<const SAMPLES: usize>(
    set: &ProjectionSet,
    shape: Shape,
    frequency: f32,
    profile: &str,
    initial: [f32; 8],
    ratios: [f32; 8],
) {
    let steps = ratios.map(|ratio| frequency * ratio / 48_000.0);
    let caps = steps.map(projection_cap);
    let phase_step = f32x8::from(steps);
    let left_gain = f32x8::from([0.117, 0.121, 0.125, 0.129, 0.133, 0.137, 0.141, 0.145]);
    let right_gain = f32x8::from([0.145, 0.141, 0.137, 0.133, 0.129, 0.125, 0.121, 0.117]);
    let mut current_oscillators = std::array::from_fn::<_, 8, _>(|lane| {
        let mut oscillator = VaOscillator::default();
        oscillator.phase = initial[lane];
        oscillator
    });
    let mut candidate_oscillators = current_oscillators;
    let mut current_left = [f32x8::ZERO; SAMPLES];
    let mut current_right = [f32x8::ZERO; SAMPLES];
    let mut candidate_left = [f32x8::ZERO; SAMPLES];
    let mut candidate_right = [f32x8::ZERO; SAMPLES];
    let (current, candidate) = projection_bench_pair(
        || {
            if matches!(shape, Shape::Saw) {
                accumulate_saw8_block_constant(
                    &mut current_oscillators,
                    phase_step,
                    left_gain,
                    right_gain,
                    &mut current_left,
                    &mut current_right,
                    Antialiasing::SplineOptimized,
                );
            } else {
                accumulate_shape8_block_constant(
                    &mut current_oscillators,
                    phase_step,
                    left_gain,
                    right_gain,
                    &mut current_left,
                    &mut current_right,
                    shape.shape(),
                    shape.pulse_width(),
                    Antialiasing::SplineOptimized,
                );
            }
            current_left[SAMPLES - 1].to_array()[0]
        },
        || {
            accumulate_projection8_block(
                &mut candidate_oscillators,
                phase_step,
                left_gain,
                right_gain,
                &mut candidate_left,
                &mut candidate_right,
                shape,
                set,
            );
            candidate_left[SAMPLES - 1].to_array()[0]
        },
    );
    let cap_min = *caps.iter().min().expect("eight caps");
    let cap_max = *caps.iter().max().expect("eight caps");
    println!(
        "projection_cpu,shape={},block={SAMPLES},profile={profile},hz={frequency:.0},cap_min={cap_min},cap_max={cap_max},current_ns_per_frame={:.3},candidate_ns_per_frame={:.3},ratio={:.3},current_min={:.3},current_max={:.3},candidate_min={:.3},candidate_max={:.3}",
        shape.name(),
        current.median / SAMPLES as f64,
        candidate.median / SAMPLES as f64,
        candidate.median / current.median,
        current.min / SAMPLES as f64,
        current.max / SAMPLES as f64,
        candidate.min / SAMPLES as f64,
        candidate.max / SAMPLES as f64,
    );
}

#[test]
#[ignore = "manual pinned native fixed-bank x8 CPU report"]
fn piecewise_projection_cpu_report() {
    const SHAPES: [Shape; 4] = [Shape::Saw, Shape::Square, Shape::Pulse, Shape::Triangle];
    const FREQUENCIES: [f32; 6] = [3_500.0, 4_200.0, 5_000.0, 7_000.0, 9_000.0, 12_000.0];
    const COHERENT: [f32; 8] = [0.173; 8];
    const DECORRELATED: [f32; 8] = [0.073, 0.173, 0.271, 0.389, 0.491, 0.593, 0.697, 0.811];
    const UNIFORM: [f32; 8] = [1.0; 8];
    const DETUNED: [f32; 8] = [0.985, 0.990, 0.995, 1.000, 1.005, 1.010, 1.015, 1.020];
    crate::performance::select_detected_backend_for_probe();
    let set = ProjectionSet::fit();
    println!(
        "projection_cpu_contract,backend={:?},blocks=20000,repeats=7,lanes=8,blocksizes=24|32,current=production_constant_x8,candidate=p8_quintic_fixed_bank",
        crate::performance::spline_backend()
    );
    for shape in SHAPES {
        for frequency in FREQUENCIES {
            for (profile, initial, ratios) in [
                ("coherent", COHERENT, UNIFORM),
                ("decorrelated", DECORRELATED, UNIFORM),
                ("structural_detuned", DECORRELATED, DETUNED),
            ] {
                report_projection_cpu::<24>(&set, shape, frequency, profile, initial, ratios);
                report_projection_cpu::<32>(&set, shape, frequency, profile, initial, ratios);
            }
        }
    }
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

#[derive(Clone, Copy)]
enum DpwKernel {
    Dpw2,
    Dpw3,
    Dpw23,
}

impl DpwKernel {
    const ALL: [Self; 3] = [Self::Dpw2, Self::Dpw3, Self::Dpw23];

    const fn name(self) -> &'static str {
        match self {
            Self::Dpw2 => "dpw2_factored",
            Self::Dpw3 => "dpw3_factored",
            Self::Dpw23 => "dpw23_extrapolated",
        }
    }
}

#[inline(always)]
fn probe_wrap_phase(phase: f32) -> f32 {
    if phase < 0.0 {
        phase + 1.0
    } else if phase >= 1.0 {
        phase - 1.0
    } else {
        phase
    }
}

#[inline(always)]
fn probe_dpw2_saw(phase: f32, step: f32) -> f32 {
    if step <= f32::EPSILON {
        return phase.mul_add(2.0, -1.0);
    }
    let regular = phase.mul_add(2.0, -1.0 - step);
    if phase < step {
        (1.0 - step) * (1.0 - 2.0 * phase / step)
    } else {
        regular
    }
}

#[inline(always)]
fn probe_dpw3_saw(phase: f32, step: f32) -> f32 {
    if step <= f32::EPSILON {
        return phase.mul_add(2.0, -1.0);
    }
    let regular = phase.mul_add(2.0, -1.0 - 2.0 * step);
    if phase < step {
        let q = phase / step;
        1.0 - q * q + 2.0 * step * (q - 1.0)
    } else if phase < 2.0 * step {
        let q = phase / step;
        (q - 2.0).mul_add(q - 2.0, regular)
    } else {
        regular
    }
}

#[inline(always)]
fn probe_dpw_saw(kernel: DpwKernel, phase: f32, step: f32) -> f32 {
    match kernel {
        DpwKernel::Dpw2 => probe_dpw2_saw(phase, step),
        DpwKernel::Dpw3 => probe_dpw3_saw(phase, step),
        DpwKernel::Dpw23 => {
            let delayed = probe_wrap_phase(phase - 0.5 * step);
            2.0 * probe_dpw2_saw(delayed, step) - probe_dpw3_saw(phase, step)
        }
    }
}

#[inline(always)]
fn probe_dpw2_triangle(phase: f32, step: f32) -> f32 {
    if step <= f32::EPSILON {
        return 1.0 - 4.0 * (phase - 0.5).abs();
    }
    if phase < step {
        let q = phase / step;
        return -1.0 + 2.0 * step * ((1.0 - q).powi(2) + q * q);
    }
    if phase >= 0.5 && phase < 0.5 + step {
        let q = (phase - 0.5) / step;
        return 1.0 - 2.0 * step * ((1.0 - q).powi(2) + q * q);
    }
    let midpoint = phase - 0.5 * step;
    1.0 - 4.0 * (midpoint - 0.5).abs()
}

#[inline(always)]
fn probe_dpw_shape(kernel: DpwKernel, shape: Shape, phase: f32, step: f32) -> f32 {
    match shape {
        Shape::Saw => probe_dpw_saw(kernel, phase, step),
        Shape::Square | Shape::Pulse => {
            let width = shape
                .pulse_width()
                .clamp(step.max(0.03), 1.0 - step.max(0.03));
            let shifted = probe_wrap_phase(phase + 1.0 - width);
            probe_dpw_saw(kernel, shifted, step) - probe_dpw_saw(kernel, phase, step) + 2.0 * width
                - 1.0
        }
        Shape::Triangle => probe_dpw2_triangle(phase, step),
        Shape::Custom => unreachable!("DPW canonical probe excludes custom curves"),
    }
}

#[inline(always)]
fn probe_wrap_phase8(phase: f32x8) -> f32x8 {
    phase.cmp_lt(f32x8::ZERO).blend(phase + f32x8::ONE, phase)
}

#[inline(always)]
fn probe_dpw2_saw8(phase: f32x8, step: f32x8, inverse_step: f32x8) -> f32x8 {
    let regular = phase.mul_add(f32x8::splat(2.0), -f32x8::ONE - step);
    let edge = (f32x8::ONE - step) * (f32x8::ONE - phase * inverse_step * f32x8::splat(2.0));
    phase.cmp_lt(step).blend(edge, regular)
}

#[inline(always)]
fn probe_dpw3_saw8(phase: f32x8, step: f32x8, inverse_step: f32x8) -> f32x8 {
    let q = phase * inverse_step;
    let regular = phase.mul_add(f32x8::splat(2.0), -f32x8::ONE - step * f32x8::splat(2.0));
    let edge = f32x8::ONE - q * q + step * f32x8::splat(2.0) * (q - f32x8::ONE);
    let distance = q - f32x8::splat(2.0);
    let middle = distance.mul_add(distance, regular);
    phase.cmp_lt(step).blend(
        edge,
        phase
            .cmp_lt(step * f32x8::splat(2.0))
            .blend(middle, regular),
    )
}

#[inline(always)]
fn probe_dpw_saw8(kernel: DpwKernel, phase: f32x8, step: f32x8, inverse_step: f32x8) -> f32x8 {
    match kernel {
        DpwKernel::Dpw2 => probe_dpw2_saw8(phase, step, inverse_step),
        DpwKernel::Dpw3 => probe_dpw3_saw8(phase, step, inverse_step),
        DpwKernel::Dpw23 => {
            let delayed = probe_wrap_phase8(phase - step * f32x8::splat(0.5));
            probe_dpw2_saw8(delayed, step, inverse_step) * f32x8::splat(2.0)
                - probe_dpw3_saw8(phase, step, inverse_step)
        }
    }
}

#[inline(always)]
fn probe_dpw2_triangle8(phase: f32x8, step: f32x8, inverse_step: f32x8) -> f32x8 {
    let q = phase * inverse_step;
    let one_minus_q = f32x8::ONE - q;
    let edge = -f32x8::ONE + step * f32x8::splat(2.0) * (one_minus_q * one_minus_q + q * q);
    let half = f32x8::splat(0.5);
    let peak_q = (phase - half) * inverse_step;
    let one_minus_peak_q = f32x8::ONE - peak_q;
    let peak = f32x8::ONE
        - step * f32x8::splat(2.0) * (one_minus_peak_q * one_minus_peak_q + peak_q * peak_q);
    let midpoint = phase - step * f32x8::splat(0.5);
    let regular = f32x8::ONE - (midpoint - half).abs() * f32x8::splat(4.0);
    let at_peak = !phase.cmp_lt(half) & phase.cmp_lt(half + step);
    phase.cmp_lt(step).blend(edge, at_peak.blend(peak, regular))
}

#[inline(always)]
fn probe_dpw_shape8(
    kernel: DpwKernel,
    shape: Shape,
    phase: f32x8,
    step: f32x8,
    inverse_step: f32x8,
    pulse_width: f32,
) -> f32x8 {
    match shape {
        Shape::Saw => probe_dpw_saw8(kernel, phase, step, inverse_step),
        Shape::Square | Shape::Pulse => {
            let width = f32x8::splat(pulse_width);
            let shifted = super::wrap_phase8(phase + f32x8::ONE - width);
            probe_dpw_saw8(kernel, shifted, step, inverse_step)
                - probe_dpw_saw8(kernel, phase, step, inverse_step)
                + width * f32x8::splat(2.0)
                - f32x8::ONE
        }
        Shape::Triangle => probe_dpw2_triangle8(phase, step, inverse_step),
        Shape::Custom => unreachable!("DPW canonical probe excludes custom curves"),
    }
}

fn render_dpw(
    shape: Shape,
    kernel: DpwKernel,
    period: usize,
    samples: usize,
) -> (Vec<f64>, Vec<f64>) {
    let step = 1.0 / period as f32;
    let mut phase = 0.0_f32;
    let mut oversampler = StereoOversampler::default();
    oversampler.reset(1);
    let mut output = Vec::with_capacity(samples);
    let mut raw = Vec::with_capacity(samples);
    for index in 0..samples + period * 8 {
        let sample = probe_dpw_shape(kernel, shape, phase, step);
        phase = probe_wrap_phase(phase + step);
        oversampler.push(sample, sample);
        if index >= period * 8 {
            output.push(f64::from(oversampler.output().0));
            raw.push(f64::from(sample));
        }
    }
    (output, raw)
}

fn dpw_response(kernel: DpwKernel, harmonic: usize, period: usize) -> Complex {
    let half_angle = std::f64::consts::PI * harmonic as f64 / period as f64;
    let sinc = half_angle.sin() / half_angle;
    match kernel {
        DpwKernel::Dpw2 => Complex::from_polar(sinc, -half_angle),
        DpwKernel::Dpw3 => Complex::from_polar(sinc * sinc, -2.0 * half_angle),
        DpwKernel::Dpw23 => Complex::from_polar(2.0 * sinc - sinc * sinc, -2.0 * half_angle),
    }
}

fn aligned_error_metrics(
    candidate: &[f64],
    ideal: &[f64],
    period: usize,
) -> (f64, f64, f64, f64, f64, Vec<f64>) {
    let (lag, aligned) = aligned(candidate, ideal, period);
    let error = aligned
        .iter()
        .zip(ideal)
        .map(|(actual, expected)| actual - expected)
        .collect::<Vec<_>>();
    let error_energy = error.iter().map(|value| value * value).sum::<f64>();
    let ideal_energy = ideal.iter().map(|value| value * value).sum::<f64>();
    let rms = (error_energy / ideal.len() as f64).sqrt();
    let peak = error.iter().copied().map(f64::abs).fold(0.0, f64::max);
    let dc = aligned.iter().sum::<f64>() / aligned.len() as f64;
    let gain = (aligned.iter().map(|value| value * value).sum::<f64>() / ideal_energy).sqrt();
    (lag, rms, peak, dc, gain, aligned)
}

fn report_dpw_quality(shape: Shape, kernel: DpwKernel, period: usize, curve: WaveCurveRt) {
    let samples = period * 32;
    let (ideal, _) = reference(shape, period, samples, &[]);
    let current = render_shipping(shape, period, samples, 1, curve);
    let (candidate, raw_candidate) = render_dpw(shape, kernel, period, samples);
    let (current_lag, current_rms, current_peak, current_dc, current_gain, _) =
        aligned_error_metrics(&current, &ideal, period);
    let (lag, rms, peak, dc, gain, candidate) = aligned_error_metrics(&candidate, &ideal, period);
    let (raw_lag, _, _, _, _, raw_candidate) =
        aligned_error_metrics(&raw_candidate, &ideal, period);
    let ideal_energy = ideal.iter().map(|value| value * value).sum::<f64>();
    let candidate_error_energy = rms * rms * samples as f64;
    let current_error_energy = current_rms * current_rms * samples as f64;

    let mut bins = raw_candidate
        .iter()
        .map(|sample| Complex::new(*sample, 0.0))
        .collect::<Vec<_>>();
    fft(&mut bins, false);
    for bin in &mut bins {
        *bin /= samples as f64;
    }
    let cycles = samples / period;
    let mut wanted_energy = 0.0;
    let mut wanted_error = 0.0;
    let mut alias_numeric = 0.0;
    let mut legal_total_error = 0.0;
    for harmonic in 1..=(period - 1) / 2 {
        let coefficient = analytic_coefficient(shape, harmonic);
        let shift = Complex::from_polar(
            1.0,
            std::f64::consts::TAU * harmonic as f64 * raw_lag / period as f64,
        );
        let wanted = coefficient * dpw_response(kernel, harmonic, period) * shift;
        let actual = bins[harmonic * cycles];
        wanted_energy += coefficient.norm_sqr();
        wanted_error += (wanted - coefficient).norm_sqr();
        alias_numeric += (actual - wanted).norm_sqr();
        legal_total_error += (actual - coefficient).norm_sqr();
    }
    let off_grid_energy = bins[1..samples / 2]
        .iter()
        .enumerate()
        .filter(|(index, _)| (index + 1) % cycles != 0)
        .map(|(_, coefficient)| coefficient.norm_sqr())
        .sum::<f64>();
    let alias_artifact = alias_numeric + off_grid_energy;

    let error = candidate
        .iter()
        .zip(&ideal)
        .map(|(actual, expected)| actual - expected)
        .collect::<Vec<_>>();
    let mut boundary_residual = 0.0_f64;
    let mut global_residual = 0.0_f64;
    for index in 1..error.len() {
        let delta = (error[index] - error[index - 1]).abs();
        global_residual = global_residual.max(delta);
        if index % period <= 2 || index % period >= period.saturating_sub(2) {
            boundary_residual = boundary_residual.max(delta);
        }
    }
    println!(
        "dpw_quality,wave={},kernel={},frequency_hz={:.6},period={period},current_lag={current_lag:.4},candidate_lag={lag:.4},raw_analysis_lag={raw_lag:.4},current_rms={current_rms:.9},candidate_rms={rms:.9},current_peak={current_peak:.9},candidate_peak={peak:.9},current_ideal_db={:.3},candidate_ideal_db={:.3},wanted_transfer_error_db={:.3},alias_artifact_db={:.3},folded_alias_numeric_db={:.3},off_grid_artifact_db={:.3},legal_total_error_db={:.3},current_dc={current_dc:.9},candidate_dc={dc:.9},current_gain={current_gain:.9},candidate_gain={gain:.9},boundary_residual={boundary_residual:.9},global_residual={global_residual:.9}",
        shape.name(),
        kernel.name(),
        48_000.0 / period as f64,
        db_ratio(current_error_energy, ideal_energy),
        db_ratio(candidate_error_energy, ideal_energy),
        db_ratio(wanted_error, wanted_energy),
        db_ratio(alias_artifact, wanted_energy),
        db_ratio(alias_numeric, wanted_energy),
        db_ratio(off_grid_energy, wanted_energy),
        db_ratio(legal_total_error, wanted_energy),
    );
}

#[derive(Clone, Copy)]
struct DpwTransitionStats {
    peak: f32,
    rms: f64,
    dc: f64,
    global_step: f32,
    pitch_event_step: f32,
}

fn rapid_pitch_step(index: usize) -> (f32, bool) {
    let position = index % 112;
    let frequency = match position {
        0..24 => 440.0,
        24..56 => 7_040.0,
        56..80 => 110.0,
        _ => 12_000.0,
    };
    (
        frequency / 48_000.0,
        index > 0 && matches!(position, 0 | 24 | 56 | 80),
    )
}

fn transition_stats(mut sample: impl FnMut(f32) -> f32) -> DpwTransitionStats {
    const SAMPLES: usize = 8_192;
    let mut peak = 0.0_f32;
    let mut energy = 0.0_f64;
    let mut sum = 0.0_f64;
    let mut previous = 0.0_f32;
    let mut global_step = 0.0_f32;
    let mut pitch_event_step = 0.0_f32;
    for index in 0..SAMPLES {
        let (step, pitch_event) = rapid_pitch_step(index);
        let value = sample(step);
        peak = peak.max(value.abs());
        energy += f64::from(value) * f64::from(value);
        sum += f64::from(value);
        if index > 0 {
            let delta = (value - previous).abs();
            global_step = global_step.max(delta);
            if pitch_event {
                pitch_event_step = pitch_event_step.max(delta);
            }
        }
        previous = value;
    }
    DpwTransitionStats {
        peak,
        rms: (energy / SAMPLES as f64).sqrt(),
        dc: sum / SAMPLES as f64,
        global_step,
        pitch_event_step,
    }
}

fn current_transition_stats(shape: Shape) -> DpwTransitionStats {
    let mut oscillator = VaOscillator::default();
    oscillator.set_phase(0.137);
    transition_stats(|step| {
        oscillator.generate_shape_step(
            shape.shape(),
            step,
            shape.pulse_width(),
            Antialiasing::SplineOptimized,
        )
    })
}

fn dpw_transition_stats(shape: Shape, kernel: DpwKernel) -> DpwTransitionStats {
    let mut phase = 0.137_f32;
    transition_stats(|step| {
        let sample = probe_dpw_shape(kernel, shape, phase, step);
        phase = probe_wrap_phase(phase + step);
        sample
    })
}

fn reset_replay_error(shape: Shape, kernel: DpwKernel) -> (f32, f32) {
    const SAMPLES: usize = 64;
    let mut current_cold = VaOscillator::default();
    let mut current_warm = VaOscillator::default();
    let step = 440.0 / 48_000.0;
    for _ in 0..1024 {
        black_box(current_warm.generate_shape_step(
            shape.shape(),
            step,
            shape.pulse_width(),
            Antialiasing::SplineOptimized,
        ));
    }
    current_warm.reset();
    let mut current_error = 0.0_f32;
    let mut cold_phase = 0.0_f32;
    let mut reset_phase = 0.371_f32;
    for _ in 0..1024 {
        black_box(probe_dpw_shape(kernel, shape, reset_phase, step));
        reset_phase = probe_wrap_phase(reset_phase + step);
    }
    reset_phase = 0.0;
    let mut candidate_error = 0.0_f32;
    for _ in 0..SAMPLES {
        let cold = current_cold.generate_shape_step(
            shape.shape(),
            step,
            shape.pulse_width(),
            Antialiasing::SplineOptimized,
        );
        let reset = current_warm.generate_shape_step(
            shape.shape(),
            step,
            shape.pulse_width(),
            Antialiasing::SplineOptimized,
        );
        current_error = current_error.max((cold - reset).abs());
        let cold = probe_dpw_shape(kernel, shape, cold_phase, step);
        let reset = probe_dpw_shape(kernel, shape, reset_phase, step);
        candidate_error = candidate_error.max((cold - reset).abs());
        cold_phase = probe_wrap_phase(cold_phase + step);
        reset_phase = probe_wrap_phase(reset_phase + step);
    }
    (current_error, candidate_error)
}

fn report_dpw_transitions(shape: Shape, kernel: DpwKernel) {
    let current = current_transition_stats(shape);
    let candidate = dpw_transition_stats(shape, kernel);
    let (current_reset_error, candidate_reset_error) = reset_replay_error(shape, kernel);
    println!(
        "dpw_transition,wave={},kernel={},schedule=440x24|7040x32|110x24|12000x32,current_peak={:.9},candidate_peak={:.9},current_rms={:.9},candidate_rms={:.9},current_dc={:.9},candidate_dc={:.9},current_global_step={:.9},candidate_global_step={:.9},current_pitch_event_step={:.9},candidate_pitch_event_step={:.9},current_reset_replay_error={current_reset_error:.9},candidate_reset_replay_error={candidate_reset_error:.9}",
        shape.name(),
        kernel.name(),
        current.peak,
        candidate.peak,
        current.rms,
        candidate.rms,
        current.dc,
        candidate.dc,
        current.global_step,
        candidate.global_step,
        current.pitch_event_step,
        candidate.pitch_event_step,
    );
}

fn measure_workload(mut workload: impl FnMut() -> f32) -> [f64; 3] {
    const REPEATS: usize = 5;
    let mut timings = [0.0; REPEATS];
    for timing in &mut timings {
        let started = Instant::now();
        black_box(workload());
        *timing = started.elapsed().as_nanos() as f64;
    }
    timings.sort_by(f64::total_cmp);
    [timings[REPEATS / 2], timings[0], timings[REPEATS - 1]]
}

fn measure_current_scalar_block<const SAMPLES: usize>(shape: Shape, step: f32) -> [f64; 3] {
    const BLOCKS: usize = 20_000;
    let elapsed = measure_workload(|| {
        let mut oscillator = VaOscillator::default();
        oscillator.set_phase(0.137);
        let mut left = [0.0_f32; SAMPLES];
        let mut right = [0.0_f32; SAMPLES];
        for _ in 0..BLOCKS {
            for frame in 0..SAMPLES {
                let sample = oscillator.generate_shape_step(
                    shape.shape(),
                    step,
                    shape.pulse_width(),
                    Antialiasing::SplineOptimized,
                );
                left[frame] = sample.mul_add(0.371, left[frame]);
                right[frame] = sample.mul_add(-0.217, right[frame]);
            }
        }
        left[SAMPLES - 1] + right[0] + oscillator.phase()
    });
    elapsed.map(|value| value / (BLOCKS * SAMPLES) as f64)
}

fn measure_dpw_scalar_block<const SAMPLES: usize>(
    shape: Shape,
    kernel: DpwKernel,
    step: f32,
) -> [f64; 3] {
    const BLOCKS: usize = 20_000;
    let elapsed = measure_workload(|| {
        let mut phase = 0.137_f32;
        let mut left = [0.0_f32; SAMPLES];
        let mut right = [0.0_f32; SAMPLES];
        for _ in 0..BLOCKS {
            for frame in 0..SAMPLES {
                let sample = probe_dpw_shape(kernel, shape, phase, step);
                phase = probe_wrap_phase(phase + step);
                left[frame] = sample.mul_add(0.371, left[frame]);
                right[frame] = sample.mul_add(-0.217, right[frame]);
            }
        }
        left[SAMPLES - 1] + right[0] + phase
    });
    elapsed.map(|value| value / (BLOCKS * SAMPLES) as f64)
}

#[inline(never)]
fn accumulate_dpw8_block<const SAMPLES: usize>(
    oscillators: &mut [VaOscillator; 8],
    step: f32x8,
    left: &mut [f32x8; SAMPLES],
    right: &mut [f32x8; SAMPLES],
    shape: Shape,
    kernel: DpwKernel,
    pulse_width: f32,
) {
    let mut phase = f32x8::from(std::array::from_fn(|index| oscillators[index].phase()));
    let inverse_step = f32x8::ONE / step;
    for frame in 0..SAMPLES {
        let sample = probe_dpw_shape8(kernel, shape, phase, step, inverse_step, pulse_width);
        left[frame] = sample.mul_add(f32x8::splat(0.371), left[frame]);
        right[frame] = sample.mul_add(f32x8::splat(-0.217), right[frame]);
        let next = phase + step;
        phase = next.cmp_lt(f32x8::ONE).blend(next, next - f32x8::ONE);
    }
    for (oscillator, phase) in oscillators.iter_mut().zip(phase.to_array()) {
        oscillator.phase = phase;
    }
}

fn seeded_oscillators() -> [VaOscillator; 8] {
    let mut oscillators = [VaOscillator::default(); 8];
    for (oscillator, phase) in oscillators
        .iter_mut()
        .zip([0.073_f64, 0.173, 0.271, 0.389, 0.491, 0.593, 0.697, 0.811])
    {
        oscillator.set_phase(phase);
    }
    oscillators
}

fn measure_current_x8_block<const SAMPLES: usize>(shape: Shape, step: f32) -> [f64; 3] {
    const BLOCKS: usize = 20_000;
    let elapsed = measure_workload(|| {
        let mut oscillators = seeded_oscillators();
        let mut left = [f32x8::ZERO; SAMPLES];
        let mut right = [f32x8::ZERO; SAMPLES];
        let step = f32x8::splat(step);
        for _ in 0..BLOCKS {
            if matches!(shape, Shape::Saw) {
                accumulate_saw8_block_constant(
                    &mut oscillators,
                    step,
                    f32x8::splat(0.371),
                    f32x8::splat(-0.217),
                    &mut left,
                    &mut right,
                    Antialiasing::SplineOptimized,
                );
            } else {
                accumulate_shape8_block_constant(
                    &mut oscillators,
                    step,
                    f32x8::splat(0.371),
                    f32x8::splat(-0.217),
                    &mut left,
                    &mut right,
                    shape.shape(),
                    shape.pulse_width(),
                    Antialiasing::SplineOptimized,
                );
            }
        }
        left[SAMPLES - 1].to_array().into_iter().sum::<f32>()
            + right[0].to_array()[0]
            + oscillators[0].phase()
    });
    elapsed.map(|value| value / (BLOCKS * SAMPLES) as f64)
}

fn measure_dpw_x8_block<const SAMPLES: usize>(
    shape: Shape,
    kernel: DpwKernel,
    step: f32,
) -> [f64; 3] {
    const BLOCKS: usize = 20_000;
    let width = shape
        .pulse_width()
        .clamp(step.max(0.03), 1.0 - step.max(0.03));
    let elapsed = measure_workload(|| {
        let mut oscillators = seeded_oscillators();
        let mut left = [f32x8::ZERO; SAMPLES];
        let mut right = [f32x8::ZERO; SAMPLES];
        let step = f32x8::splat(step);
        for _ in 0..BLOCKS {
            accumulate_dpw8_block(
                &mut oscillators,
                step,
                &mut left,
                &mut right,
                shape,
                kernel,
                width,
            );
        }
        left[SAMPLES - 1].to_array().into_iter().sum::<f32>()
            + right[0].to_array()[0]
            + oscillators[0].phase()
    });
    elapsed.map(|value| value / (BLOCKS * SAMPLES) as f64)
}

fn print_dpw_cpu(
    shape: Shape,
    kernel: DpwKernel,
    frequency: f32,
    frames: usize,
    current_scalar: [f64; 3],
    candidate_scalar: [f64; 3],
    current_x8: [f64; 3],
    candidate_x8: [f64; 3],
) {
    println!(
        "dpw_cpu,wave={},kernel={},frequency_hz={frequency:.0},frames={frames},scalar_current_ns={:.3},scalar_candidate_ns={:.3},scalar_delta_pct={:+.2},scalar_current_range={:.3}..{:.3},scalar_candidate_range={:.3}..{:.3},x8_current_ns={:.3},x8_candidate_ns={:.3},x8_delta_pct={:+.2},x8_current_range={:.3}..{:.3},x8_candidate_range={:.3}..{:.3}",
        shape.name(),
        kernel.name(),
        current_scalar[0],
        candidate_scalar[0],
        (candidate_scalar[0] / current_scalar[0] - 1.0) * 100.0,
        current_scalar[1],
        current_scalar[2],
        candidate_scalar[1],
        candidate_scalar[2],
        current_x8[0],
        candidate_x8[0],
        (candidate_x8[0] / current_x8[0] - 1.0) * 100.0,
        current_x8[1],
        current_x8[2],
        candidate_x8[1],
        candidate_x8[2],
    );
}

fn report_dpw_cpu_for_block<const SAMPLES: usize>(shape: Shape, kernel: DpwKernel, frequency: f32) {
    let step = frequency / 48_000.0;
    print_dpw_cpu(
        shape,
        kernel,
        frequency,
        SAMPLES,
        measure_current_scalar_block::<SAMPLES>(shape, step),
        measure_dpw_scalar_block::<SAMPLES>(shape, kernel, step),
        measure_current_x8_block::<SAMPLES>(shape, step),
        measure_dpw_x8_block::<SAMPLES>(shape, kernel, step),
    );
}

fn assert_factored_dpw_equivalence() {
    let polynomial = |phase: f64| {
        let x = phase.mul_add(2.0, -1.0);
        x * x * x - x
    };
    let triangle_primitive = |phase: f64| {
        if phase < 0.5 {
            phase.mul_add(2.0 * phase, -phase)
        } else {
            (-2.0 * phase).mul_add(phase, 3.0 * phase - 1.0)
        }
    };
    for period in [1745, 109, 7] {
        let step = 1.0 / period as f64;
        for index in 0..period {
            let phase = index as f64 / period as f64;
            let previous = (phase - step).rem_euclid(1.0);
            let previous2 = (phase - 2.0 * step).rem_euclid(1.0);
            let x = phase.mul_add(2.0, -1.0);
            let previous_x = previous.mul_add(2.0, -1.0);
            let dpw2 = (x * x - previous_x * previous_x) / (4.0 * step);
            let dpw3 = (polynomial(phase) - 2.0 * polynomial(previous) + polynomial(previous2))
                / (24.0 * step * step);
            let triangle = (triangle_primitive(phase) - triangle_primitive(previous)) / step;
            assert!((f64::from(probe_dpw2_saw(phase as f32, step as f32)) - dpw2).abs() < 5.0e-4);
            assert!((f64::from(probe_dpw3_saw(phase as f32, step as f32)) - dpw3).abs() < 5.0e-4);
            assert!(
                (f64::from(probe_dpw2_triangle(phase as f32, step as f32)) - triangle).abs()
                    < 5.0e-4
            );
        }
    }
}

#[test]
#[ignore = "manual release-mode factored DPW2/DPW3 canonical second shot"]
fn factored_dpw_canonical_quality_transition_and_cpu_report() {
    crate::performance::select_detected_backend_for_probe();
    assert_factored_dpw_equivalence();
    let curve = drawn_curve();
    println!(
        "dpw_contract,baseline=SplineOptimized_1x,candidates=factored_DPW2|factored_DPW3|aligned_extrapolated_DPW23,state_bytes=0,phase=f32,quality=ideal_BL|wanted_transfer|folded_alias|off_grid|DC|gain,quality_path=oscillator+StereoOversampler_1x,transitions=rapid_24_32_frames|reset_replay,cpu=real_scalar_and_x8_stereo_accumulation_before_common_oversampler"
    );
    for shape in [Shape::Saw, Shape::Square, Shape::Pulse] {
        for kernel in DpwKernel::ALL {
            for period in [1745, 109, 7] {
                report_dpw_quality(shape, kernel, period, curve);
            }
            report_dpw_transitions(shape, kernel);
            for frequency in [440.0, 7_040.0] {
                report_dpw_cpu_for_block::<24>(shape, kernel, frequency);
                report_dpw_cpu_for_block::<32>(shape, kernel, frequency);
            }
        }
    }
    for period in [1745, 109, 7] {
        report_dpw_quality(Shape::Triangle, DpwKernel::Dpw2, period, curve);
    }
    report_dpw_transitions(Shape::Triangle, DpwKernel::Dpw2);
    for frequency in [440.0, 7_040.0] {
        report_dpw_cpu_for_block::<24>(Shape::Triangle, DpwKernel::Dpw2, frequency);
        report_dpw_cpu_for_block::<32>(Shape::Triangle, DpwKernel::Dpw2, frequency);
    }
}

const EQUIRIPPLE_BLEP: [f32; 33] = [
    -5.000_000_0e-1,
    -4.370_964_0e-1,
    -3.751_749_7e-1,
    -3.151_845_2e-1,
    -2.580_085_7e-1,
    -2.044_365_6e-1,
    -1.551_391_5e-1,
    -1.106_491_1e-1,
    -7.134_806_0e-2,
    -3.745_992_7e-2,
    -9.050_991e-3,
    1.396_365_7e-2,
    3.180_778_2e-2,
    4.482_667e-2,
    5.346_580_8e-2,
    5.824_683_2e-2,
    5.974_199_3e-2,
    5.854_847_5e-2,
    5.526_377_5e-2,
    5.046_326_3e-2,
    4.468_082e-2,
    3.839_327e-2,
    3.200_899e-2,
    2.586_093_9e-2,
    2.020_397_1e-2,
    1.521_618_1e-2,
    1.100_372_5e-2,
    7.608_528_7e-3,
    5.018_091e-3,
    3.176_623e-3,
    1.996_723_2e-3,
    7.798_161e-4,
    0.0,
];

const EQUIRIPPLE_BLAMP: [f32; 33] = [
    9.945_452e-2,
    7.017_026e-2,
    4.478_677_7e-2,
    2.321_304_4e-2,
    5.300_759_3e-3,
    -9.150_651e-3,
    -2.038_739_2e-2,
    -2.869_327_6e-2,
    -3.438_068_6e-2,
    -3.778_093_7e-2,
    -3.923_440_4e-2,
    -3.908_088e-2,
    -3.765_052_6e-2,
    -3.525_569_7e-2,
    -3.218_405_7e-2,
    -2.869_303_8e-2,
    -2.500_588_7e-2,
    -2.130_931e-2,
    -1.775_267_7e-2,
    -1.444_870_7e-2,
    -1.147_545_4e-2,
    -8.879_389e-3,
    -6.679_318_4e-3,
    -4.870_883e-3,
    -3.431_354_6e-3,
    -2.324_474_7e-3,
    -1.505_102_7e-3,
    -9.234_698_6e-4,
    -5.288_88e-4,
    -2.728_031_8e-4,
    -1.111_361_1e-4,
    -2.436_925_3e-5,
    0.0,
];

#[inline(always)]
fn probe_equiripple_table(position: f32, table: &[f32; 33], odd: bool) -> f32 {
    let distance = position.abs();
    if distance >= 2.0 {
        return 0.0;
    }
    let scaled = distance * 16.0;
    let index = scaled as usize;
    let value = (table[index + 1] - table[index]).mul_add(scaled - index as f32, table[index]);
    if odd && position < 0.0 { -value } else { value }
}

#[inline(always)]
fn probe_equiripple_edge(phase: f32, step: f32) -> f32 {
    if step <= f32::EPSILON {
        return 0.0;
    }
    let support = 2.0 * step;
    if support < 0.5 && phase >= support && phase <= 1.0 - support {
        return 0.0;
    }
    let inverse = step.recip();
    let residual = if support < 0.5 {
        probe_equiripple_table(
            if phase < 0.5 { phase } else { phase - 1.0 } * inverse,
            &EQUIRIPPLE_BLEP,
            true,
        )
    } else {
        probe_equiripple_table(phase * inverse, &EQUIRIPPLE_BLEP, true)
            + probe_equiripple_table((phase - 1.0) * inverse, &EQUIRIPPLE_BLEP, true)
    };
    2.0 * residual
}

#[inline(always)]
fn probe_equiripple_blamp(phase: f32, step: f32) -> f32 {
    if step <= f32::EPSILON {
        return 0.0;
    }
    let support = 2.0 * step;
    if support < 0.5 && phase >= support && phase <= 1.0 - support {
        return 0.0;
    }
    let inverse = step.recip();
    probe_equiripple_table(phase * inverse, &EQUIRIPPLE_BLAMP, false)
        + probe_equiripple_table((phase - 1.0) * inverse, &EQUIRIPPLE_BLAMP, false)
}

#[inline(always)]
fn probe_equiripple_shape(shape: Shape, phase: f32, step: f32) -> f32 {
    match shape {
        Shape::Saw => phase.mul_add(2.0, -1.0) - probe_equiripple_edge(phase, step),
        Shape::Square | Shape::Pulse => {
            let minimum = step.max(0.03);
            let width = shape.pulse_width().clamp(minimum, 1.0 - minimum);
            let shifted = probe_wrap_phase(phase + 1.0 - width);
            (if phase < width { 1.0 } else { -1.0 }) + probe_equiripple_edge(phase, step)
                - probe_equiripple_edge(shifted, step)
        }
        Shape::Triangle => {
            let peak = probe_wrap_phase(phase + 0.5);
            (8.0 * step).mul_add(
                probe_equiripple_blamp(phase, step) - probe_equiripple_blamp(peak, step),
                1.0 - 4.0 * (phase - 0.5).abs(),
            )
        }
        Shape::Custom => unreachable!("equiripple probe excludes custom curves"),
    }
}

#[inline(always)]
fn probe_equiripple_table8(position: f32x8, table: &[f32; 33], odd: bool) -> f32x8 {
    f32x8::from(
        position
            .to_array()
            .map(|value| probe_equiripple_table(value, table, odd)),
    )
}

#[inline(always)]
fn probe_equiripple_edge8(phase: f32x8, step: f32x8, inverse: f32x8) -> f32x8 {
    let support = step * f32x8::splat(2.0);
    let before = phase.cmp_lt(support);
    let event = before | phase.cmp_gt(f32x8::ONE - support);
    if !event.any() {
        return f32x8::ZERO;
    }
    let residual = if support.cmp_lt(f32x8::splat(0.5)).all() {
        probe_equiripple_table8(
            before.blend(phase, phase - f32x8::ONE) * inverse,
            &EQUIRIPPLE_BLEP,
            true,
        )
    } else {
        probe_equiripple_table8(phase * inverse, &EQUIRIPPLE_BLEP, true)
            + probe_equiripple_table8((phase - f32x8::ONE) * inverse, &EQUIRIPPLE_BLEP, true)
    };
    event.blend(residual * f32x8::splat(2.0), f32x8::ZERO)
}

#[inline(always)]
fn probe_equiripple_blamp8(phase: f32x8, step: f32x8, inverse: f32x8) -> f32x8 {
    let support = step * f32x8::splat(2.0);
    let event = phase.cmp_lt(support) | phase.cmp_gt(f32x8::ONE - support);
    if !event.any() {
        return f32x8::ZERO;
    }
    let residual = probe_equiripple_table8(phase * inverse, &EQUIRIPPLE_BLAMP, false)
        + probe_equiripple_table8((phase - f32x8::ONE) * inverse, &EQUIRIPPLE_BLAMP, false);
    event.blend(residual, f32x8::ZERO)
}

#[inline(always)]
fn probe_equiripple_shape8(
    shape: Shape,
    phase: f32x8,
    step: f32x8,
    inverse: f32x8,
    pulse_width: f32,
) -> f32x8 {
    match shape {
        Shape::Saw => {
            phase.mul_add(f32x8::splat(2.0), -f32x8::ONE)
                - probe_equiripple_edge8(phase, step, inverse)
        }
        Shape::Square | Shape::Pulse => {
            let width = f32x8::splat(pulse_width);
            let shifted = super::wrap_phase8(phase + f32x8::ONE - width);
            phase.cmp_lt(width).blend(f32x8::ONE, -f32x8::ONE)
                + probe_equiripple_edge8(phase, step, inverse)
                - probe_equiripple_edge8(shifted, step, inverse)
        }
        Shape::Triangle => {
            let half = f32x8::splat(0.5);
            let peak = super::wrap_phase8(phase + half);
            (step * f32x8::splat(8.0)).mul_add(
                probe_equiripple_blamp8(phase, step, inverse)
                    - probe_equiripple_blamp8(peak, step, inverse),
                f32x8::ONE - (phase - half).abs() * f32x8::splat(4.0),
            )
        }
        Shape::Custom => unreachable!("equiripple probe excludes custom curves"),
    }
}

fn render_equiripple(shape: Shape, period: usize, samples: usize) -> Vec<f64> {
    let step = 1.0 / period as f32;
    let mut phase = 0.0_f32;
    let mut oversampler = StereoOversampler::default();
    oversampler.reset(1);
    let mut output = Vec::with_capacity(samples);
    for index in 0..samples + period * 8 {
        let sample = probe_equiripple_shape(shape, phase, step);
        phase = probe_wrap_phase(phase + step);
        oversampler.push(sample, sample);
        if index >= period * 8 {
            output.push(f64::from(oversampler.output().0));
        }
    }
    output
}

fn spectral_error(output: &[f64], ideal_bins: &[Complex], period: usize) -> (f64, f64) {
    let mut bins = output
        .iter()
        .map(|sample| Complex::new(*sample, 0.0))
        .collect::<Vec<_>>();
    fft(&mut bins, false);
    for bin in &mut bins {
        *bin /= output.len() as f64;
    }
    let cycles = output.len() / period;
    let mut wanted_error = 0.0;
    let mut wanted_energy = 0.0;
    for harmonic in 1..=(period - 1) / 2 {
        let bin = harmonic * cycles;
        wanted_error += (bins[bin] - ideal_bins[bin]).norm_sqr();
        wanted_energy += ideal_bins[bin].norm_sqr();
    }
    let off_grid = bins[1..output.len() / 2]
        .iter()
        .enumerate()
        .filter(|(index, _)| (index + 1) % cycles != 0)
        .map(|(_, value)| value.norm_sqr())
        .sum::<f64>();
    (
        db_ratio(wanted_error, wanted_energy),
        db_ratio(off_grid, wanted_energy),
    )
}

fn report_equiripple_quality(shape: Shape, period: usize, curve: WaveCurveRt) {
    let samples = period * 32;
    let (ideal, ideal_bins) = reference(shape, period, samples, &[]);
    let current = render_shipping(shape, period, samples, 1, curve);
    let candidate = render_equiripple(shape, period, samples);
    let (_, current_rms, current_peak, current_dc, current_gain, _) =
        aligned_error_metrics(&current, &ideal, period);
    let (lag, rms, peak, dc, gain, candidate) = aligned_error_metrics(&candidate, &ideal, period);
    let ideal_energy = ideal.iter().map(|value| value * value).sum::<f64>();
    let (wanted_error, off_grid) = spectral_error(&candidate, &ideal_bins, period);
    let error = candidate
        .iter()
        .zip(&ideal)
        .map(|(actual, expected)| actual - expected)
        .collect::<Vec<_>>();
    let mut boundary = 0.0_f64;
    let mut global = 0.0_f64;
    for index in 1..error.len() {
        let delta = (error[index] - error[index - 1]).abs();
        global = global.max(delta);
        if index % period <= 2 || index % period >= period.saturating_sub(2) {
            boundary = boundary.max(delta);
        }
    }
    println!(
        "equiripple_quality,wave={},frequency_hz={:.6},period={period},candidate_lag={lag:.4},current_rms={current_rms:.9},candidate_rms={rms:.9},current_peak={current_peak:.9},candidate_peak={peak:.9},current_ideal_db={:.3},candidate_ideal_db={:.3},candidate_wanted_complex_error_db={wanted_error:.3},candidate_off_grid_db={off_grid:.3},current_dc={current_dc:.9},candidate_dc={dc:.9},current_gain={current_gain:.9},candidate_gain={gain:.9},candidate_boundary_residual={boundary:.9},candidate_global_residual={global:.9}",
        shape.name(),
        48_000.0 / period as f64,
        db_ratio(current_rms * current_rms * samples as f64, ideal_energy),
        db_ratio(rms * rms * samples as f64, ideal_energy),
    );
}

fn equiripple_transition_stats(shape: Shape) -> DpwTransitionStats {
    let mut phase = 0.137_f32;
    transition_stats(|step| {
        let sample = probe_equiripple_shape(shape, phase, step);
        phase = probe_wrap_phase(phase + step);
        sample
    })
}

fn report_equiripple_transitions(shape: Shape) {
    let current = current_transition_stats(shape);
    let candidate = equiripple_transition_stats(shape);
    println!(
        "equiripple_transition,wave={},schedule=440x24|7040x32|110x24|12000x32,current_peak={:.9},candidate_peak={:.9},current_rms={:.9},candidate_rms={:.9},current_dc={:.9},candidate_dc={:.9},current_global_step={:.9},candidate_global_step={:.9},current_pitch_event_step={:.9},candidate_pitch_event_step={:.9},candidate_state_bytes=0",
        shape.name(),
        current.peak,
        candidate.peak,
        current.rms,
        candidate.rms,
        current.dc,
        candidate.dc,
        current.global_step,
        candidate.global_step,
        current.pitch_event_step,
        candidate.pitch_event_step,
    );
}

fn measure_equiripple_scalar_block<const SAMPLES: usize>(shape: Shape, step: f32) -> [f64; 3] {
    const BLOCKS: usize = 20_000;
    let elapsed = measure_workload(|| {
        let mut phase = 0.137_f32;
        let mut left = [0.0_f32; SAMPLES];
        let mut right = [0.0_f32; SAMPLES];
        for _ in 0..BLOCKS {
            for frame in 0..SAMPLES {
                let sample = probe_equiripple_shape(shape, phase, step);
                phase = probe_wrap_phase(phase + step);
                left[frame] = sample.mul_add(0.371, left[frame]);
                right[frame] = sample.mul_add(-0.217, right[frame]);
            }
        }
        left[SAMPLES - 1] + right[0] + phase
    });
    elapsed.map(|value| value / (BLOCKS * SAMPLES) as f64)
}

#[inline(never)]
fn accumulate_equiripple8_block<const SAMPLES: usize>(
    oscillators: &mut [VaOscillator; 8],
    step: f32x8,
    left: &mut [f32x8; SAMPLES],
    right: &mut [f32x8; SAMPLES],
    shape: Shape,
    pulse_width: f32,
) {
    let mut phase = f32x8::from(std::array::from_fn(|index| oscillators[index].phase()));
    let inverse = f32x8::ONE / step;
    for frame in 0..SAMPLES {
        let sample = probe_equiripple_shape8(shape, phase, step, inverse, pulse_width);
        left[frame] = sample.mul_add(f32x8::splat(0.371), left[frame]);
        right[frame] = sample.mul_add(f32x8::splat(-0.217), right[frame]);
        let next = phase + step;
        phase = next.cmp_lt(f32x8::ONE).blend(next, next - f32x8::ONE);
    }
    for (oscillator, phase) in oscillators.iter_mut().zip(phase.to_array()) {
        oscillator.phase = phase;
    }
}

fn measure_equiripple_x8_block<const SAMPLES: usize>(shape: Shape, step: f32) -> [f64; 3] {
    const BLOCKS: usize = 20_000;
    let width = shape
        .pulse_width()
        .clamp(step.max(0.03), 1.0 - step.max(0.03));
    let elapsed = measure_workload(|| {
        let mut oscillators = seeded_oscillators();
        let mut left = [f32x8::ZERO; SAMPLES];
        let mut right = [f32x8::ZERO; SAMPLES];
        let step = f32x8::splat(step);
        for _ in 0..BLOCKS {
            accumulate_equiripple8_block(
                &mut oscillators,
                step,
                &mut left,
                &mut right,
                shape,
                width,
            );
        }
        left[SAMPLES - 1].to_array().into_iter().sum::<f32>()
            + right[0].to_array()[0]
            + oscillators[0].phase()
    });
    elapsed.map(|value| value / (BLOCKS * SAMPLES) as f64)
}

fn report_equiripple_cpu<const SAMPLES: usize>(shape: Shape, frequency: f32) {
    let step = frequency / 48_000.0;
    let current_scalar = measure_current_scalar_block::<SAMPLES>(shape, step);
    let candidate_scalar = measure_equiripple_scalar_block::<SAMPLES>(shape, step);
    let current_x8 = measure_current_x8_block::<SAMPLES>(shape, step);
    let candidate_x8 = measure_equiripple_x8_block::<SAMPLES>(shape, step);
    println!(
        "equiripple_cpu,wave={},frequency_hz={frequency:.0},frames={SAMPLES},scalar_current_ns={:.3},scalar_candidate_ns={:.3},scalar_delta_pct={:+.2},scalar_current_range={:.3}..{:.3},scalar_candidate_range={:.3}..{:.3},x8_current_ns={:.3},x8_candidate_ns={:.3},x8_delta_pct={:+.2},x8_current_range={:.3}..{:.3},x8_candidate_range={:.3}..{:.3}",
        shape.name(),
        current_scalar[0],
        candidate_scalar[0],
        (candidate_scalar[0] / current_scalar[0] - 1.0) * 100.0,
        current_scalar[1],
        current_scalar[2],
        candidate_scalar[1],
        candidate_scalar[2],
        current_x8[0],
        candidate_x8[0],
        (candidate_x8[0] / current_x8[0] - 1.0) * 100.0,
        current_x8[1],
        current_x8[2],
        candidate_x8[1],
        candidate_x8[2],
    );
}

fn assert_equiripple_contract() {
    assert_eq!(EQUIRIPPLE_BLEP[0], -0.5);
    assert_eq!(EQUIRIPPLE_BLEP[32], 0.0);
    assert_eq!(EQUIRIPPLE_BLAMP[32], 0.0);
    for shape in [Shape::Saw, Shape::Square, Shape::Pulse, Shape::Triangle] {
        for step in [110.0_f32 / 48_000.0, 880.0 / 48_000.0, 7_040.0 / 48_000.0] {
            let inverse = f32x8::splat(step.recip());
            for index in 0..4096 {
                let phases = std::array::from_fn(|lane| ((index * 8 + lane) as f32 / 4096.0) % 1.0);
                let scalar = phases.map(|phase| probe_equiripple_shape(shape, phase, step));
                let simd = probe_equiripple_shape8(
                    shape,
                    f32x8::from(phases),
                    f32x8::splat(step),
                    inverse,
                    shape
                        .pulse_width()
                        .clamp(step.max(0.03), 1.0 - step.max(0.03)),
                )
                .to_array();
                for lane in 0..8 {
                    assert!((scalar[lane] - simd[lane]).abs() <= 2.0e-6);
                    assert!(simd[lane].is_finite());
                }
            }
        }
    }
}

#[test]
#[ignore = "manual release-mode support-two equiripple BLEP/BLAMP experiment"]
fn support_two_equiripple_blep_quality_transition_and_cpu_report() {
    crate::performance::select_detected_backend_for_probe();
    assert_equiripple_contract();
    let curve = drawn_curve();
    println!(
        "equiripple_contract,baseline=SplineOptimized_1x,candidate=dolph_chebyshev_30db_sinc_63_at_16x_support2,blep_table_bytes=132,blamp_table_bytes=132,state_bytes=0,latency_samples=0,interpolation=linear,quality=ideal_BL|wanted_complex|off_grid|DC|gain,transitions=rapid_24_32_frames,cpu=real_scalar_and_x8_stereo_accumulation_before_common_oversampler"
    );
    for shape in [Shape::Saw, Shape::Square, Shape::Pulse, Shape::Triangle] {
        for period in [1745, 109, 7] {
            report_equiripple_quality(shape, period, curve);
        }
        report_equiripple_transitions(shape);
        for frequency in [440.0, 7_040.0] {
            report_equiripple_cpu::<24>(shape, frequency);
            report_equiripple_cpu::<32>(shape, frequency);
        }
    }
}

#[test]
#[ignore = "manual one-cycle ideal-projection metric validation"]
fn one_cycle_shape_alignment_validation_report() {
    const PERIOD: usize = 109;
    let (ideal, coefficients) = reference(Shape::Saw, PERIOD, PERIOD, &[]);
    let gain = ideal.iter().map(|sample| sample * 1.2).collect::<Vec<_>>();
    let dc = ideal.iter().map(|sample| sample + 0.1).collect::<Vec<_>>();
    let phase = circular_phase_shift(&ideal, 0.237);
    let shape = ideal
        .iter()
        .enumerate()
        .map(|(index, sample)| {
            sample + 0.04 * (std::f64::consts::TAU * 7.0 * index as f64 / PERIOD as f64).sin()
        })
        .collect::<Vec<_>>();
    let drift = (0..PERIOD)
        .map(|index| {
            let position = index as f64 + 0.5 * index as f64 / (PERIOD - 1) as f64;
            coefficients
                .iter()
                .enumerate()
                .map(|(bin, coefficient)| {
                    let signed_bin = if bin <= PERIOD / 2 {
                        bin as f64
                    } else {
                        bin as f64 - PERIOD as f64
                    };
                    let angle = std::f64::consts::TAU * signed_bin * position / PERIOD as f64;
                    (*coefficient * Complex::from_polar(1.0, angle)).re
                })
                .sum::<f64>()
        })
        .collect::<Vec<_>>();
    let cases = [
        ("clean", ideal.clone()),
        ("gain", gain),
        ("dc", dc),
        ("phase", phase),
        ("cycle_drift", drift),
        ("shape", shape),
    ];
    let metrics = cases
        .iter()
        .map(|(name, candidate)| {
            let metrics = cycle_shape_metrics(candidate, &ideal);
            println!(
                "cycle_shape_validation,defect={name},unaligned_rms={:.9},unaligned_peak={:.9},unaligned_phase_samples={:.6},unaligned_dc={:.9},unaligned_gain={:.9},residual_rms={:.9},residual_peak={:.9}",
                metrics.unaligned_rms,
                metrics.unaligned_peak,
                metrics.phase_offset_samples,
                metrics.dc_offset,
                metrics.gain,
                metrics.residual_rms,
                metrics.residual_peak,
            );
            metrics
        })
        .collect::<Vec<_>>();

    assert!(metrics[0].residual_rms < 1.0e-12);
    assert!((metrics[1].gain - 1.2).abs() < 1.0e-12);
    assert!(metrics[1].unaligned_rms > 0.05 && metrics[1].residual_rms < 1.0e-12);
    assert!((metrics[2].dc_offset - 0.1).abs() < 1.0e-12);
    assert!(metrics[2].unaligned_rms > 0.05 && metrics[2].residual_rms < 1.0e-12);
    assert!((metrics[3].phase_offset_samples - 0.237).abs() < 1.0e-6);
    assert!(metrics[3].unaligned_rms > 0.001 && metrics[3].residual_rms < 1.0e-7);
    assert!(metrics[4].residual_rms > 0.01);
    assert!(metrics[4].residual_peak > 0.2);
    assert!(metrics[5].phase_offset_samples.abs() < 0.01);
    assert!(metrics[5].residual_rms > 0.01);
}
