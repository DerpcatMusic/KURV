use std::hint::black_box;
use std::mem::size_of;
use std::time::Instant;

use truce_simd::simd::f32x8;
use wide::{CmpGe, CmpLt};

use crate::dsp::{Complex, fft};
use crate::wave_curve::WaveCurveRt;

use super::experiment::{Shape, aligned, db_ratio, reference, render_shipping};
use super::{
    Antialiasing, VaOscillator, accumulate_saw8_block_constant, accumulate_shape8_block_constant,
};

const SAMPLE_RATE: f64 = 48_000.0;
const TABLE_OVERSAMPLE: usize = 16;

const SHAPES: [Shape; 3] = [Shape::Saw, Shape::Square, Shape::Pulse];

struct MinBlepKernel {
    taps: usize,
    oversample: usize,
    step: Vec<f32>,
    mean_residual_area: f32,
    residual_area_span: f32,
}

impl MinBlepKernel {
    fn new(taps: usize, oversample: usize) -> Self {
        assert!(taps.is_power_of_two() && oversample.is_power_of_two());
        let length = taps * oversample;
        let zero_crossings = taps as f64 * 0.5;
        let mut impulse = (0..length)
            .map(|index| {
                let phase =
                    -zero_crossings + 2.0 * zero_crossings * index as f64 / (length - 1) as f64;
                let sinc = if phase.abs() <= f64::EPSILON {
                    1.0
                } else {
                    let angle = std::f64::consts::PI * phase;
                    angle.sin() / angle
                };
                let angle = std::f64::consts::TAU * index as f64 / (length - 1) as f64;
                let window = 0.358_75 - 0.488_29 * angle.cos() + 0.141_28 * (2.0 * angle).cos()
                    - 0.011_68 * (3.0 * angle).cos();
                Complex::new(sinc * window, 0.0)
            })
            .collect::<Vec<_>>();

        fft(&mut impulse, false);
        for value in &mut impulse {
            *value = Complex::new(value.norm().max((-30.0_f64).exp()).ln(), 0.0);
        }
        fft(&mut impulse, true);
        for value in &mut impulse[1..length / 2] {
            *value *= 2.0;
        }
        for value in &mut impulse[length / 2 + 1..] {
            *value = Complex::ZERO;
        }
        fft(&mut impulse, false);
        for value in &mut impulse {
            *value = Complex::from_polar(value.re.exp(), value.im);
        }
        fft(&mut impulse, true);

        let total = impulse.iter().map(|value| value.re).sum::<f64>();
        assert!(total.is_finite() && total.abs() > f64::EPSILON);
        let mut running = 0.0;
        let mut step = Vec::with_capacity(length + 1);
        for value in impulse {
            step.push((running / total) as f32);
            running += value.re;
        }
        step.push(1.0);
        assert!(step.iter().all(|value| value.is_finite()));
        let mut kernel = Self {
            taps,
            oversample,
            step,
            mean_residual_area: 0.0,
            residual_area_span: 0.0,
        };
        let mut area_min = f32::INFINITY;
        let mut area_max = f32::NEG_INFINITY;
        for fraction in 0..oversample {
            let elapsed = (fraction as f32 + 0.5) / oversample as f32;
            let area = (0..taps)
                .map(|tap| kernel.residual(tap, elapsed))
                .sum::<f32>();
            kernel.mean_residual_area += area / oversample as f32;
            area_min = area_min.min(area);
            area_max = area_max.max(area);
        }
        kernel.residual_area_span = area_max - area_min;
        kernel
    }

    #[inline(always)]
    fn residual(&self, tap: usize, elapsed: f32) -> f32 {
        let position = (tap as f32 + elapsed.clamp(0.0, 1.0)) * self.oversample as f32;
        let index = (position as usize).min(self.step.len() - 2);
        let blend = position - index as f32;
        (self.step[index + 1] - self.step[index]).mul_add(blend, self.step[index]) - 1.0
    }
}

#[derive(Clone)]
struct MinBlepScalar<const TAPS: usize> {
    phase: f32,
    ring: [f32; TAPS],
    cursor: usize,
    pulse_width: f32,
}

impl<const TAPS: usize> MinBlepScalar<TAPS> {
    fn new(phase: f32, pulse_width: f32) -> Self {
        Self {
            phase,
            ring: [0.0; TAPS],
            cursor: 0,
            pulse_width,
        }
    }

    #[inline(always)]
    fn insert(&mut self, kernel: &MinBlepKernel, start: usize, subsample: f32, jump: f32) {
        debug_assert_eq!(kernel.taps, TAPS);
        let elapsed = 1.0 - subsample.clamp(f32::EPSILON, 1.0);
        for tap in 0..TAPS {
            let index = (start + tap) & (TAPS - 1);
            self.ring[index] = jump.mul_add(kernel.residual(tap, elapsed), self.ring[index]);
        }
    }

    #[inline(always)]
    fn process(
        &mut self,
        shape: Shape,
        step: f32,
        requested_width: f32,
        kernel: &MinBlepKernel,
    ) -> f32 {
        let width = step.max(requested_width.clamp(0.03, 0.97)).min(1.0 - step);
        if !matches!(shape, Shape::Saw) && width.to_bits() != self.pulse_width.to_bits() {
            let old = raw_sample(shape, self.phase, self.pulse_width);
            let new = raw_sample(shape, self.phase, width);
            if new.to_bits() != old.to_bits() {
                self.insert(kernel, self.cursor, 1.0, new - old);
            }
            self.pulse_width = width;
        }

        let mut sample = raw_sample(shape, self.phase, width) + self.ring[self.cursor];
        if matches!(shape, Shape::Saw) {
            let correction_dc = (-2.0 * kernel.mean_residual_area) * step;
            sample -= correction_dc;
        }
        self.ring[self.cursor] = 0.0;
        let next_cursor = (self.cursor + 1) & (TAPS - 1);
        let next = self.phase + step;
        if !matches!(shape, Shape::Saw) && self.phase < width && next >= width {
            self.insert(kernel, next_cursor, (width - self.phase) / step, -2.0);
        }
        if next >= 1.0 {
            let jump = if matches!(shape, Shape::Saw) {
                -2.0
            } else {
                2.0
            };
            self.insert(kernel, next_cursor, (1.0 - self.phase) / step, jump);
            self.phase = next - 1.0;
        } else {
            self.phase = next;
        }
        self.cursor = next_cursor;
        sample
    }

    fn reset_phase(&mut self, phase: f32, shape: Shape, width: f32, kernel: &MinBlepKernel) {
        let jump = raw_sample(shape, phase, width) - raw_sample(shape, self.phase, width);
        self.phase = phase;
        if jump != 0.0 {
            self.insert(kernel, self.cursor, 1.0, jump);
        }
    }
}

#[derive(Clone)]
struct MinBlep8<const TAPS: usize> {
    phase: f32x8,
    ring: [[f32; 8]; TAPS],
    cursor: usize,
    pulse_width: f32x8,
}

impl<const TAPS: usize> MinBlep8<TAPS> {
    fn new(phase: f32x8, step: f32x8, pulse_width: f32) -> Self {
        let width = step
            .fast_max(f32x8::splat(pulse_width.clamp(0.03, 0.97)))
            .fast_min(f32x8::ONE - step);
        Self {
            phase,
            ring: [[0.0; 8]; TAPS],
            cursor: 0,
            pulse_width: width,
        }
    }

    #[inline(always)]
    fn insert_lane(
        &mut self,
        kernel: &MinBlepKernel,
        start: usize,
        lane: usize,
        subsample: f32,
        jump: f32,
    ) {
        debug_assert_eq!(kernel.taps, TAPS);
        let elapsed = 1.0 - subsample.clamp(f32::EPSILON, 1.0);
        for tap in 0..TAPS {
            let index = (start + tap) & (TAPS - 1);
            self.ring[index][lane] =
                jump.mul_add(kernel.residual(tap, elapsed), self.ring[index][lane]);
        }
    }

    #[inline(always)]
    fn insert_masked(
        &mut self,
        kernel: &MinBlepKernel,
        start: usize,
        mask: f32x8,
        subsample: f32x8,
        jump: f32x8,
    ) {
        if !mask.any() {
            return;
        }
        let masks: [f32; 8] = mask.into();
        let subsamples: [f32; 8] = subsample.into();
        let jumps: [f32; 8] = jump.into();
        for lane in 0..8 {
            if masks[lane].to_bits() != 0 {
                self.insert_lane(kernel, start, lane, subsamples[lane], jumps[lane]);
            }
        }
    }

    #[inline(always)]
    fn process(
        &mut self,
        shape: Shape,
        step: f32x8,
        requested_width: f32,
        kernel: &MinBlepKernel,
    ) -> f32x8 {
        let one = f32x8::ONE;
        let width = step
            .fast_max(f32x8::splat(requested_width.clamp(0.03, 0.97)))
            .fast_min(one - step);
        if !matches!(shape, Shape::Saw) {
            let old = raw_sample8(shape, self.phase, self.pulse_width);
            let new = raw_sample8(shape, self.phase, width);
            self.insert_masked(
                kernel,
                self.cursor,
                old.cmp_lt(new) | new.cmp_lt(old),
                one,
                new - old,
            );
            self.pulse_width = width;
        }

        let raw = raw_sample8(shape, self.phase, width);
        let correction = f32x8::from(self.ring[self.cursor]);
        self.ring[self.cursor] = [0.0; 8];
        let next_cursor = (self.cursor + 1) & (TAPS - 1);
        let next = self.phase + step;
        if !matches!(shape, Shape::Saw) {
            let width_event = self.phase.cmp_lt(width) & next.cmp_ge(width);
            self.insert_masked(
                kernel,
                next_cursor,
                width_event,
                (width - self.phase) / step,
                f32x8::splat(-2.0),
            );
        }
        let wrap = next.cmp_ge(one);
        self.insert_masked(
            kernel,
            next_cursor,
            wrap,
            (one - self.phase) / step,
            f32x8::splat(if matches!(shape, Shape::Saw) {
                -2.0
            } else {
                2.0
            }),
        );
        self.phase = wrap.blend(next - one, next);
        self.cursor = next_cursor;
        if matches!(shape, Shape::Saw) {
            raw + correction - step * f32x8::splat(-2.0 * kernel.mean_residual_area)
        } else {
            raw + correction
        }
    }

    fn reset_phase(&mut self, phase: f32x8, shape: Shape, kernel: &MinBlepKernel) {
        let old = raw_sample8(shape, self.phase, self.pulse_width);
        let new = raw_sample8(shape, phase, self.pulse_width);
        self.phase = phase;
        self.insert_masked(
            kernel,
            self.cursor,
            old.cmp_lt(new) | new.cmp_lt(old),
            f32x8::ONE,
            new - old,
        );
    }
}

#[inline(always)]
fn raw_sample(shape: Shape, phase: f32, width: f32) -> f32 {
    match shape {
        Shape::Saw => phase.mul_add(2.0, -1.0),
        Shape::Square | Shape::Pulse => {
            if phase < width {
                1.0
            } else {
                -1.0
            }
        }
        Shape::Triangle | Shape::Custom => unreachable!("minimum-phase BLEP step probe"),
    }
}

#[inline(always)]
fn raw_sample8(shape: Shape, phase: f32x8, width: f32x8) -> f32x8 {
    match shape {
        Shape::Saw => phase.mul_add(f32x8::splat(2.0), -f32x8::ONE),
        Shape::Square | Shape::Pulse => phase.cmp_lt(width).blend(f32x8::ONE, -f32x8::ONE),
        Shape::Triangle | Shape::Custom => unreachable!("minimum-phase BLEP step probe"),
    }
}

#[derive(Clone, Copy)]
struct Quality {
    lag: f64,
    curve_rms: f64,
    curve_max: f64,
    wanted_magnitude_db: f64,
    wanted_complex_db: f64,
    alias_error_db: f64,
    dc: f64,
    gain: f64,
    peak: f64,
    max_step: f64,
}

fn quality(
    candidate: &[f64],
    reference: &[f64],
    reference_bins: &[Complex],
    period: usize,
) -> Quality {
    let samples = candidate.len();
    let (lag, candidate) = aligned(candidate, reference, period);
    let error = candidate
        .iter()
        .zip(reference)
        .map(|(actual, expected)| actual - expected)
        .collect::<Vec<_>>();
    let reference_energy = reference.iter().map(|value| value * value).sum::<f64>();
    let error_energy = error.iter().map(|value| value * value).sum::<f64>();
    let mut candidate_bins = candidate
        .iter()
        .map(|value| Complex::new(*value, 0.0))
        .collect::<Vec<_>>();
    fft(&mut candidate_bins, false);
    for value in &mut candidate_bins {
        *value /= samples as f64;
    }
    let cycles = samples / period;
    let mut magnitude_error = 0.0;
    let mut complex_error = 0.0;
    let mut wanted_energy = 0.0;
    for harmonic in 1..=(period - 1) / 2 {
        let bin = harmonic * cycles;
        magnitude_error += (candidate_bins[bin].norm() - reference_bins[bin].norm()).powi(2);
        complex_error += (candidate_bins[bin] - reference_bins[bin]).norm_sqr();
        wanted_energy += reference_bins[bin].norm_sqr();
    }
    Quality {
        lag,
        curve_rms: (error_energy / samples as f64).sqrt(),
        curve_max: error.iter().copied().map(f64::abs).fold(0.0, f64::max),
        wanted_magnitude_db: db_ratio(magnitude_error, wanted_energy),
        wanted_complex_db: db_ratio(complex_error, wanted_energy),
        alias_error_db: db_ratio(error_energy, reference_energy),
        dc: candidate.iter().sum::<f64>() / samples as f64,
        gain: (candidate.iter().map(|value| value * value).sum::<f64>() / reference_energy).sqrt(),
        peak: candidate.iter().copied().map(f64::abs).fold(0.0, f64::max),
        max_step: candidate
            .windows(2)
            .map(|window| (window[1] - window[0]).abs())
            .fold(0.0, f64::max),
    }
}

fn render_minblep<const TAPS: usize>(
    shape: Shape,
    period: usize,
    samples: usize,
    phase: f32,
    kernel: &MinBlepKernel,
) -> Vec<f64> {
    let step = 1.0 / period as f32;
    let mut state = MinBlepScalar::<TAPS>::new(phase, shape.pulse_width());
    let mut output = Vec::with_capacity(samples);
    for frame in 0..samples + period * 8 {
        let sample = state.process(shape, step, shape.pulse_width(), kernel);
        if frame >= period * 8 {
            output.push(f64::from(sample));
        }
    }
    output
}

fn report_quality(mode: &str, shape: Shape, period: usize, candidate: &[f64]) -> Quality {
    let (reference, bins) = reference(shape, period, candidate.len(), &[]);
    let result = quality(candidate, &reference, &bins, period);
    println!(
        "minblep_quality,mode={mode},wave={},frequency_hz={:.6},period={period},lag={:.4},curve_rms={:.9},curve_max={:.9},wanted_magnitude_error_db={:.3},wanted_complex_error_db={:.3},alias_error_db={:.3},dc={:.9},gain={:.9},peak={:.9},max_step={:.9}",
        shape.name(),
        SAMPLE_RATE / period as f64,
        result.lag,
        result.curve_rms,
        result.curve_max,
        result.wanted_magnitude_db,
        result.wanted_complex_db,
        result.alias_error_db,
        result.dc,
        result.gain,
        result.peak,
        result.max_step,
    );
    result
}

fn shifted_reference(
    shape: Shape,
    period: usize,
    samples: usize,
    phase_offset: f64,
) -> (Vec<f64>, Vec<Complex>) {
    let (_, mut coefficients) = reference(shape, period, samples, &[]);
    let cycles = samples / period;
    for harmonic in 1..=(period - 1) / 2 {
        let bin = harmonic * cycles;
        coefficients[bin] *=
            Complex::from_polar(1.0, std::f64::consts::TAU * harmonic as f64 * phase_offset);
        coefficients[samples - bin] = coefficients[bin].conj();
    }
    let mut signal = coefficients
        .iter()
        .map(|value| *value * samples as f64)
        .collect::<Vec<_>>();
    fft(&mut signal, true);
    (
        signal.into_iter().map(|value| value.re).collect(),
        coefficients,
    )
}

fn report_fractional<const TAPS: usize>(shape: Shape, period: usize, kernel: &MinBlepKernel) {
    let samples = period * 32;
    let step = 1.0 / period as f32;
    let mut worst_rms = 0.0_f64;
    let mut worst_alias = f64::NEG_INFINITY;
    let mut worst_dc = 0.0_f64;
    let mut lag_min = f64::INFINITY;
    let mut lag_max = f64::NEG_INFINITY;
    for fraction in 0..16 {
        let phase = step * fraction as f32 / 16.0;
        let candidate = render_minblep::<TAPS>(shape, period, samples, phase, kernel);
        let (reference, bins) = shifted_reference(shape, period, samples, f64::from(phase));
        let result = quality(&candidate, &reference, &bins, period);
        worst_rms = worst_rms.max(result.curve_rms);
        worst_alias = worst_alias.max(result.alias_error_db);
        worst_dc = worst_dc.max(result.dc.abs());
        lag_min = lag_min.min(result.lag);
        lag_max = lag_max.max(result.lag);
    }
    println!(
        "minblep_fractional,taps={TAPS},wave={},frequency_hz={:.6},phases=16,worst_curve_rms={worst_rms:.9},worst_alias_error_db={worst_alias:.3},worst_abs_dc={worst_dc:.9},lag_min={lag_min:.4},lag_max={lag_max:.4}",
        shape.name(),
        SAMPLE_RATE / period as f64,
    );
}

#[derive(Clone, Copy)]
struct TransitionMetrics {
    peak: f32,
    dc: f32,
    max_step: f32,
    local_step: f32,
}

fn transition_metrics(samples: &[f32], transition: usize) -> TransitionMetrics {
    let peak = samples.iter().copied().map(f32::abs).fold(0.0, f32::max);
    let dc = samples.iter().sum::<f32>() / samples.len() as f32;
    let max_step = samples
        .windows(2)
        .map(|window| (window[1] - window[0]).abs())
        .fold(0.0, f32::max);
    let start = transition.saturating_sub(64);
    let end = (transition + 64).min(samples.len());
    let local_step = samples[start..end]
        .windows(2)
        .map(|window| (window[1] - window[0]).abs())
        .fold(0.0, f32::max);
    TransitionMetrics {
        peak,
        dc,
        max_step,
        local_step,
    }
}

fn report_transition<const TAPS: usize>(shape: Shape, kind: &str, kernel: &MinBlepKernel) {
    const SAMPLES: usize = 8192;
    const CHANGE: usize = SAMPLES / 2;
    let low_step = 440.0 / SAMPLE_RATE as f32;
    let high_step = 7_040.0 / SAMPLE_RATE as f32;
    let initial = if kind == "width" {
        (0.5 - (CHANGE as f32 * low_step).fract()).rem_euclid(1.0)
    } else {
        0.137
    };
    let mut current = VaOscillator::default();
    current.set_phase(f64::from(initial));
    let mut candidate = MinBlepScalar::<TAPS>::new(initial, shape.pulse_width());
    let mut current_output = Vec::with_capacity(SAMPLES);
    let mut candidate_output = Vec::with_capacity(SAMPLES);
    for frame in 0..SAMPLES {
        let step = if kind == "pitch" && frame >= CHANGE {
            high_step
        } else {
            low_step
        };
        let width = if kind == "width" && frame >= CHANGE {
            0.69
        } else {
            shape.pulse_width()
        };
        if kind == "reset" && frame == CHANGE {
            current.set_phase(0.83);
            candidate.reset_phase(0.83, shape, width, kernel);
        }
        current_output.push(current.generate_shape_step(
            shape.shape(),
            step,
            width,
            Antialiasing::SplineOptimized,
        ));
        candidate_output.push(candidate.process(shape, step, width, kernel));
    }
    for (mode, output) in [
        ("shipping1", &current_output),
        ("minblep", &candidate_output),
    ] {
        let result = transition_metrics(output, CHANGE);
        println!(
            "minblep_transition,taps={TAPS},kind={kind},mode={mode},wave={},peak={:.9},dc={:.9},max_step={:.9},local_step={:.9}",
            shape.name(),
            result.peak,
            result.dc,
            result.max_step,
            result.local_step,
        );
    }
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn time_scalar_current<const BLOCK: usize>(shape: Shape, step: f32) -> (f64, f32) {
    const BLOCKS: usize = 40_000;
    let mut oscillator = VaOscillator::default();
    let mut left = [0.0_f32; BLOCK];
    let mut right = [0.0_f32; BLOCK];
    let mut checksum = 0.0;
    for _ in 0..128 {
        for frame in 0..BLOCK {
            let sample = oscillator.generate_shape_step(
                shape.shape(),
                step,
                shape.pulse_width(),
                Antialiasing::SplineOptimized,
            );
            left[frame] = sample.mul_add(0.37, left[frame]);
            right[frame] = sample.mul_add(-0.21, right[frame]);
        }
    }
    let started = Instant::now();
    for _ in 0..BLOCKS {
        for frame in 0..BLOCK {
            let sample = oscillator.generate_shape_step(
                shape.shape(),
                step,
                shape.pulse_width(),
                Antialiasing::SplineOptimized,
            );
            left[frame] = sample.mul_add(0.37, left[frame]);
            right[frame] = sample.mul_add(-0.21, right[frame]);
        }
        checksum += black_box(left[BLOCK - 1] + right[0]);
    }
    (
        started.elapsed().as_nanos() as f64 / (BLOCKS * BLOCK) as f64,
        checksum,
    )
}

fn time_scalar_minblep<const TAPS: usize, const BLOCK: usize>(
    shape: Shape,
    step: f32,
    kernel: &MinBlepKernel,
) -> (f64, f32) {
    const BLOCKS: usize = 40_000;
    let mut state = MinBlepScalar::<TAPS>::new(0.0, shape.pulse_width());
    let mut left = [0.0_f32; BLOCK];
    let mut right = [0.0_f32; BLOCK];
    let mut checksum = 0.0;
    for _ in 0..128 {
        for frame in 0..BLOCK {
            let sample = state.process(shape, step, shape.pulse_width(), kernel);
            left[frame] = sample.mul_add(0.37, left[frame]);
            right[frame] = sample.mul_add(-0.21, right[frame]);
        }
    }
    let started = Instant::now();
    for _ in 0..BLOCKS {
        for frame in 0..BLOCK {
            let sample = state.process(shape, step, shape.pulse_width(), kernel);
            left[frame] = sample.mul_add(0.37, left[frame]);
            right[frame] = sample.mul_add(-0.21, right[frame]);
        }
        checksum += black_box(left[BLOCK - 1] + right[0]);
    }
    (
        started.elapsed().as_nanos() as f64 / (BLOCKS * BLOCK) as f64,
        checksum,
    )
}

fn time_x8_current<const BLOCK: usize>(shape: Shape, base_step: f32) -> (f64, f32) {
    const BLOCKS: usize = 40_000;
    let phases = [0.013, 0.127, 0.239, 0.353, 0.467, 0.581, 0.697, 0.811];
    let steps = f32x8::from(std::array::from_fn(|lane| {
        base_step * (1.0 + (lane as f32 - 3.5) * 0.0007)
    }));
    let mut oscillators = [VaOscillator::default(); 8];
    for (oscillator, phase) in oscillators.iter_mut().zip(phases) {
        oscillator.set_phase(f64::from(phase));
    }
    let mut left = [f32x8::ZERO; BLOCK];
    let mut right = [f32x8::ZERO; BLOCK];
    let mut checksum = 0.0;
    for _ in 0..128 {
        if matches!(shape, Shape::Saw) {
            accumulate_saw8_block_constant(
                &mut oscillators,
                steps,
                f32x8::splat(0.37),
                f32x8::splat(-0.21),
                &mut left,
                &mut right,
                Antialiasing::SplineOptimized,
            );
        } else {
            accumulate_shape8_block_constant(
                &mut oscillators,
                steps,
                f32x8::splat(0.37),
                f32x8::splat(-0.21),
                &mut left,
                &mut right,
                shape.shape(),
                shape.pulse_width(),
                Antialiasing::SplineOptimized,
            );
        }
    }
    let started = Instant::now();
    for _ in 0..BLOCKS {
        if matches!(shape, Shape::Saw) {
            accumulate_saw8_block_constant(
                &mut oscillators,
                steps,
                f32x8::splat(0.37),
                f32x8::splat(-0.21),
                &mut left,
                &mut right,
                Antialiasing::SplineOptimized,
            );
        } else {
            accumulate_shape8_block_constant(
                &mut oscillators,
                steps,
                f32x8::splat(0.37),
                f32x8::splat(-0.21),
                &mut left,
                &mut right,
                shape.shape(),
                shape.pulse_width(),
                Antialiasing::SplineOptimized,
            );
        }
        checksum += black_box(left[BLOCK - 1].reduce_add() + right[0].reduce_add());
    }
    (
        started.elapsed().as_nanos() as f64 / (BLOCKS * BLOCK) as f64,
        checksum,
    )
}

fn time_x8_minblep<const TAPS: usize, const BLOCK: usize>(
    shape: Shape,
    base_step: f32,
    kernel: &MinBlepKernel,
) -> (f64, f32) {
    const BLOCKS: usize = 40_000;
    let phases = f32x8::from([0.013, 0.127, 0.239, 0.353, 0.467, 0.581, 0.697, 0.811]);
    let steps = f32x8::from(std::array::from_fn(|lane| {
        base_step * (1.0 + (lane as f32 - 3.5) * 0.0007)
    }));
    let mut state = MinBlep8::<TAPS>::new(phases, steps, shape.pulse_width());
    let mut left = [f32x8::ZERO; BLOCK];
    let mut right = [f32x8::ZERO; BLOCK];
    let mut checksum = 0.0;
    for _ in 0..128 {
        for frame in 0..BLOCK {
            let sample = state.process(shape, steps, shape.pulse_width(), kernel);
            left[frame] = sample.mul_add(f32x8::splat(0.37), left[frame]);
            right[frame] = sample.mul_add(f32x8::splat(-0.21), right[frame]);
        }
    }
    let started = Instant::now();
    for _ in 0..BLOCKS {
        for frame in 0..BLOCK {
            let sample = state.process(shape, steps, shape.pulse_width(), kernel);
            left[frame] = sample.mul_add(f32x8::splat(0.37), left[frame]);
            right[frame] = sample.mul_add(f32x8::splat(-0.21), right[frame]);
        }
        checksum += black_box(left[BLOCK - 1].reduce_add() + right[0].reduce_add());
    }
    (
        started.elapsed().as_nanos() as f64 / (BLOCKS * BLOCK) as f64,
        checksum,
    )
}

fn report_cpu<const TAPS: usize, const BLOCK: usize>(
    shape: Shape,
    frequency: f32,
    kernel: &MinBlepKernel,
) {
    const REPEATS: usize = 7;
    let step = frequency / SAMPLE_RATE as f32;
    let mut scalar_current = [0.0; REPEATS];
    let mut scalar_candidate = [0.0; REPEATS];
    let mut x8_current = [0.0; REPEATS];
    let mut x8_candidate = [0.0; REPEATS];
    let mut checksum = 0.0;
    for repeat in 0..REPEATS {
        if repeat % 2 == 0 {
            let (time, sum) = time_scalar_current::<BLOCK>(shape, step);
            scalar_current[repeat] = time;
            checksum += sum;
            let (time, sum) = time_scalar_minblep::<TAPS, BLOCK>(shape, step, kernel);
            scalar_candidate[repeat] = time;
            checksum += sum;
            let (time, sum) = time_x8_current::<BLOCK>(shape, step);
            x8_current[repeat] = time;
            checksum += sum;
            let (time, sum) = time_x8_minblep::<TAPS, BLOCK>(shape, step, kernel);
            x8_candidate[repeat] = time;
            checksum += sum;
        } else {
            let (time, sum) = time_x8_minblep::<TAPS, BLOCK>(shape, step, kernel);
            x8_candidate[repeat] = time;
            checksum += sum;
            let (time, sum) = time_x8_current::<BLOCK>(shape, step);
            x8_current[repeat] = time;
            checksum += sum;
            let (time, sum) = time_scalar_minblep::<TAPS, BLOCK>(shape, step, kernel);
            scalar_candidate[repeat] = time;
            checksum += sum;
            let (time, sum) = time_scalar_current::<BLOCK>(shape, step);
            scalar_current[repeat] = time;
            checksum += sum;
        }
    }
    let scalar_current = median(&mut scalar_current);
    let scalar_candidate = median(&mut scalar_candidate);
    let x8_current = median(&mut x8_current);
    let x8_candidate = median(&mut x8_candidate);
    println!(
        "minblep_cpu,taps={TAPS},block={BLOCK},wave={},frequency_hz={frequency:.3},scalar_current_ns={scalar_current:.3},scalar_minblep_ns={scalar_candidate:.3},scalar_ratio={:.3},x8_current_ns={x8_current:.3},x8_minblep_ns={x8_candidate:.3},x8_ratio={:.3},checksum={checksum:.9}",
        shape.name(),
        scalar_candidate / scalar_current,
        x8_candidate / x8_current,
    );
}

fn report_scalar_x8_parity<const TAPS: usize>(shape: Shape, kernel: &MinBlepKernel) {
    let phases = [0.013, 0.127, 0.239, 0.353, 0.467, 0.581, 0.697, 0.811];
    let mut scalar = phases.map(|phase| MinBlepScalar::<TAPS>::new(phase, shape.pulse_width()));
    let mut steps = f32x8::from(std::array::from_fn(|lane| {
        440.0 / SAMPLE_RATE as f32 * (1.0 + (lane as f32 - 3.5) * 0.0007)
    }));
    let mut vector = MinBlep8::<TAPS>::new(f32x8::from(phases), steps, shape.pulse_width());
    let mut max_error = 0.0_f32;
    for frame in 0..4096 {
        if frame == 1024 {
            steps *= f32x8::splat(16.0);
        }
        let width = if frame >= 2048 && !matches!(shape, Shape::Saw) {
            0.69
        } else {
            shape.pulse_width()
        };
        if frame == 3072 {
            let reset = f32x8::from([0.83, 0.71, 0.59, 0.47, 0.35, 0.23, 0.11, 0.97]);
            for (state, phase) in scalar.iter_mut().zip(reset.to_array()) {
                state.reset_phase(phase, shape, width, kernel);
            }
            vector.reset_phase(reset, shape, kernel);
        }
        let step_lanes = steps.to_array();
        let expected: [f32; 8] = std::array::from_fn(|lane| {
            scalar[lane].process(shape, step_lanes[lane], width, kernel)
        });
        let actual = vector.process(shape, steps, width, kernel).to_array();
        for lane in 0..8 {
            max_error = max_error.max((actual[lane] - expected[lane]).abs());
        }
    }
    println!(
        "minblep_parity,taps={TAPS},wave={},frames=4096,pitch_transition=1024,width_transition=2048,reset_transition=3072,max_abs_error={max_error:.9}",
        shape.name(),
    );
}

fn report_candidate<const TAPS: usize>(kernel: &MinBlepKernel) {
    for shape in SHAPES {
        for period in [436, 55, 7] {
            let samples = period * 32;
            let candidate = render_minblep::<TAPS>(shape, period, samples, 0.0, kernel);
            report_quality(&format!("minblep{TAPS}_mean"), shape, period, &candidate);
        }
        report_fractional::<TAPS>(shape, 7, kernel);
        report_transition::<TAPS>(shape, "pitch", kernel);
        report_transition::<TAPS>(shape, "reset", kernel);
        if !matches!(shape, Shape::Saw) {
            report_transition::<TAPS>(shape, "width", kernel);
        }
        report_scalar_x8_parity::<TAPS>(shape, kernel);
        for frequency in [440.0, 3_520.0, 7_040.0] {
            report_cpu::<TAPS, 24>(shape, frequency, kernel);
            report_cpu::<TAPS, 32>(shape, frequency, kernel);
        }
    }
}

#[test]
#[ignore = "manual sparse minimum-phase BLEP ring quality, transition, and CPU experiment"]
fn sparse_minblep_ring_report() {
    crate::performance::select_detected_backend_for_probe();
    let kernel8 = MinBlepKernel::new(8, TABLE_OVERSAMPLE);
    let kernel16 = MinBlepKernel::new(16, TABLE_OVERSAMPLE);
    let kernel32 = MinBlepKernel::new(32, TABLE_OVERSAMPLE);
    println!(
        "minblep_contract,reference=ideal_harmonic_projection,runtime=fractional_crossing_event_deposit+fixed_ring,table_oversample={TABLE_OVERSAMPLE},table_bytes=516|1028|2052,scalar_state_bytes={}|{}|{},x8_state_bytes={}|{}|{},blocks=24|32",
        size_of::<MinBlepScalar<8>>(),
        size_of::<MinBlepScalar<16>>(),
        size_of::<MinBlepScalar<32>>(),
        size_of::<MinBlep8<8>>(),
        size_of::<MinBlep8<16>>(),
        size_of::<MinBlep8<32>>(),
    );
    for (taps, kernel) in [(8, &kernel8), (16, &kernel16), (32, &kernel32)] {
        println!(
            "minblep_kernel,taps={taps},mean_residual_area={:.9},fractional_area_span={:.9},saw_dc_compensation=jump_x_event_rate_x_mean_area",
            kernel.mean_residual_area, kernel.residual_area_span,
        );
    }
    for shape in SHAPES {
        for period in [436, 55, 7] {
            let samples = period * 32;
            report_quality(
                "shipping1",
                shape,
                period,
                &render_shipping(shape, period, samples, 1, WaveCurveRt::default()),
            );
            report_quality(
                "shipping2",
                shape,
                period,
                &render_shipping(shape, period, samples, 2, WaveCurveRt::default()),
            );
        }
    }
    report_candidate::<8>(&kernel8);
    report_candidate::<16>(&kernel16);
    report_candidate::<32>(&kernel32);
}

#[test]
fn sparse_minblep_kernel_and_state_are_bounded() {
    let kernel = MinBlepKernel::new(16, TABLE_OVERSAMPLE);
    assert_eq!(kernel.step.len(), 16 * TABLE_OVERSAMPLE + 1);
    assert_eq!(kernel.step[0], 0.0);
    assert_eq!(*kernel.step.last().expect("kernel endpoint"), 1.0);
    let mut scalar = MinBlepScalar::<16>::new(0.997, 0.31);
    let mut vector = MinBlep8::<16>::new(
        f32x8::from([0.997, 0.31, 0.0, 0.8, 0.2, 0.5, 0.7, 0.1]),
        f32x8::splat(0.01),
        0.31,
    );
    for _ in 0..1024 {
        assert!(
            scalar
                .process(Shape::Pulse, 0.01, 0.31, &kernel)
                .is_finite()
        );
        assert!(
            vector
                .process(Shape::Pulse, f32x8::splat(0.01), 0.31, &kernel)
                .to_array()
                .into_iter()
                .all(f32::is_finite)
        );
    }
}
