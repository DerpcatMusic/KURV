//! Analytic VA oscillator with oscillator-domain spectral morphing.

use std::f32::consts::{PI, TAU};
use std::sync::Arc;

use realfft::num_complex::Complex32;
use realfft::{ComplexToReal, RealFftPlanner};

use crate::va::{SpectralEffect, Waveform};

const CYCLE_LEN: usize = 2048;
const SPECTRUM_LEN: usize = CYCLE_LEN / 2 + 1;
const UPDATE_INTERVAL: u16 = 128;
const CROSSFADE_SAMPLES: u16 = 128;

#[derive(Clone, Copy, Debug)]
pub struct ShapeSettings {
    pub waveform: Waveform,
    pub frequency_hz: f32,
    pub pulse_width: f32,
    pub center_hz: f32,
    pub spread_octaves: f32,
    pub mix: f32,
    pub sweep_phase: f32,
    pub keytrack: f32,
    pub stereo_offset: f32,
}

pub struct ShapeVaOscillator {
    sample_rate: f32,
    phase_l: f32,
    phase_r: f32,
    effect: SpectralEffect,
    inverse: Arc<dyn ComplexToReal<f32>>,
    source: Vec<Complex32>,
    spectrum: Vec<Complex32>,
    scratch: Vec<Complex32>,
    current: Vec<f32>,
    next: Vec<f32>,
    update_countdown: u16,
    crossfade: u16,
    rendered_key: u64,
    initialized: bool,
}

impl Default for ShapeVaOscillator {
    fn default() -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let inverse = planner.plan_fft_inverse(CYCLE_LEN);
        Self {
            sample_rate: 44_100.0,
            phase_l: 0.0,
            phase_r: 0.0,
            effect: SpectralEffect::PhaseDisperse,
            source: inverse.make_input_vec(),
            spectrum: inverse.make_input_vec(),
            scratch: inverse.make_scratch_vec(),
            current: inverse.make_output_vec(),
            next: inverse.make_output_vec(),
            inverse,
            update_countdown: 0,
            crossfade: 0,
            rendered_key: 0,
            initialized: false,
        }
    }
}

impl ShapeVaOscillator {
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate.max(1.0);
        self.update_countdown = 0;
    }

    pub fn set_effect(&mut self, effect: SpectralEffect) {
        if self.effect != effect {
            self.effect = effect;
            self.update_countdown = 0;
        }
    }

    pub fn reset(&mut self) {
        self.phase_l = 0.0;
        self.phase_r = 0.0;
        self.update_countdown = 0;
        self.crossfade = 0;
        self.initialized = false;
    }

    pub fn generate(&mut self, settings: ShapeSettings) -> (f32, f32) {
        let frequency_hz = settings.frequency_hz.clamp(1.0, self.sample_rate * 0.45);
        if self.update_countdown == 0 {
            let key = render_key(settings, self.effect, self.sample_rate);
            if !self.initialized || key != self.rendered_key {
                self.rebuild(settings);
                self.rendered_key = key;
            }
            self.update_countdown = UPDATE_INTERVAL;
        }
        self.update_countdown -= 1;

        let stereo_phase = settings.stereo_offset.clamp(0.0, 4.0) * 0.0025;
        let left = self.read_crossfaded(wrap01(self.phase_l - stereo_phase));
        let right = self.read_crossfaded(wrap01(self.phase_r + stereo_phase));
        let dt = frequency_hz / self.sample_rate;

        self.phase_l = wrap01(self.phase_l + dt);
        self.phase_r = wrap01(self.phase_r + dt);
        if self.crossfade > 0 {
            self.crossfade -= 1;
            if self.crossfade == 0 {
                std::mem::swap(&mut self.current, &mut self.next);
            }
        }

        (left, right)
    }

    pub fn write_preview(&mut self, settings: ShapeSettings, output: &mut [f32]) {
        self.rebuild_cycle(settings, false);
        resample_cycle(&self.next, output);
    }

    fn rebuild(&mut self, settings: ShapeSettings) {
        self.rebuild_cycle(settings, true);
        if self.initialized {
            self.crossfade = CROSSFADE_SAMPLES;
        } else {
            std::mem::swap(&mut self.current, &mut self.next);
            self.initialized = true;
            self.crossfade = 0;
        }
    }

    fn rebuild_cycle(&mut self, settings: ShapeSettings, retain_current: bool) {
        analytic_spectrum(&mut self.source, settings, self.sample_rate);
        self.spectrum.copy_from_slice(&self.source);
        apply_spectral_effect(&self.source, &mut self.spectrum, settings, self.effect);
        self.spectrum[0] = Complex32::new(0.0, 0.0);
        self.spectrum[SPECTRUM_LEN - 1] = Complex32::new(0.0, 0.0);

        if self
            .inverse
            .process_with_scratch(&mut self.spectrum, &mut self.next, &mut self.scratch)
            .is_err()
        {
            self.next.fill(0.0);
            return;
        }

        normalize_cycle(&mut self.next);
        if !retain_current {
            self.crossfade = 0;
        }
    }

    fn read_crossfaded(&self, phase: f32) -> f32 {
        let current = read_cycle(&self.current, phase);
        if self.crossfade == 0 {
            return current;
        }
        let mix = 1.0 - self.crossfade as f32 / CROSSFADE_SAMPLES as f32;
        current + (read_cycle(&self.next, phase) - current) * mix
    }
}

#[cfg(test)]
pub fn preview_cycle(settings: ShapeSettings, output: &mut [f32]) {
    let mut oscillator = ShapeVaOscillator::default();
    oscillator.set_sample_rate(48_000.0);
    oscillator.write_preview(settings, output);
}

fn analytic_spectrum(output: &mut [Complex32], settings: ShapeSettings, sample_rate: f32) {
    output.fill(Complex32::new(0.0, 0.0));
    let frequency = settings.frequency_hz.clamp(1.0, sample_rate * 0.45);
    let harmonic_limit =
        ((sample_rate * 0.48 / frequency) as usize).min(output.len().saturating_sub(2));
    let scale = CYCLE_LEN as f32;

    for (harmonic, bin) in output
        .iter_mut()
        .enumerate()
        .take(harmonic_limit + 1)
        .skip(1)
    {
        let h = harmonic as f32;
        *bin = match settings.waveform {
            Waveform::Saw => Complex32::new(0.0, scale / (PI * h)),
            Waveform::Pulse => {
                let width = settings.pulse_width.clamp(0.03, 0.97);
                let magnitude = scale * 2.0 * (PI * h * width).sin() / (PI * h);
                Complex32::from_polar(magnitude, -PI * h * width)
            }
        };
    }
}

fn apply_spectral_effect(
    source: &[Complex32],
    output: &mut [Complex32],
    settings: ShapeSettings,
    effect: SpectralEffect,
) {
    let amount = settings.mix.clamp(0.0, 1.0);
    if amount <= f32::EPSILON {
        return;
    }
    let frequency = settings.frequency_hz.max(1.0);
    let tracked_focus = keytracked_center_hz(settings.center_hz, frequency, settings.keytrack);
    let focus = (tracked_focus / frequency).clamp(1.0, (SPECTRUM_LEN - 2) as f32);
    let shape = settings.spread_octaves.clamp(0.0, 6.0);

    match effect {
        SpectralEffect::PhaseDisperse => {
            phase_disperse(output, focus, shape, amount, settings.sweep_phase)
        }
        SpectralEffect::HarmonicStretch => harmonic_stretch(source, output, focus, shape, amount),
        SpectralEffect::Formant => formant(output, focus, shape, amount, settings.sweep_phase),
        SpectralEffect::SpectralFold => spectral_fold(source, output, focus, shape, amount),
    }
}

fn phase_disperse(spectrum: &mut [Complex32], focus: f32, shape: f32, amount: f32, motion: f32) {
    let curvature = 0.015 + shape * shape * 0.022;
    for (harmonic, bin) in spectrum.iter_mut().enumerate().skip(2) {
        let distance = harmonic as f32 - focus;
        let phase = amount * curvature * distance * distance * (1.0 + 0.18 * (motion * TAU).sin());
        *bin *= Complex32::from_polar(1.0, phase);
    }
}

fn harmonic_stretch(
    source: &[Complex32],
    output: &mut [Complex32],
    focus: f32,
    shape: f32,
    amount: f32,
) {
    output.fill(Complex32::new(0.0, 0.0));
    let power = 1.0 + shape * 0.12;
    for (harmonic, coefficient) in source.iter().copied().enumerate().skip(1) {
        let h = harmonic as f32;
        let stretched = if h <= 1.0 {
            1.0
        } else {
            1.0 + (h - 1.0) * (h / focus).max(1.0).powf(power - 1.0)
        };
        scatter(output, h + (stretched - h) * amount, coefficient);
    }
}

fn formant(spectrum: &mut [Complex32], focus: f32, shape: f32, amount: f32, motion: f32) {
    let width = (0.22 + (6.0 - shape) * 0.10).max(0.12);
    let second = focus * (2.0 + 0.25 * (motion * TAU).sin());
    for (harmonic, bin) in spectrum.iter_mut().enumerate().skip(1) {
        let h = harmonic as f32;
        let first_distance = (h / focus).max(0.001).log2() / width;
        let second_distance = (h / second).max(0.001).log2() / (width * 1.35);
        let envelope = 0.18
            + 2.6 * (-0.5 * first_distance * first_distance).exp()
            + 1.25 * (-0.5 * second_distance * second_distance).exp();
        *bin *= 1.0 + (envelope - 1.0) * amount;
    }
}

fn spectral_fold(
    source: &[Complex32],
    output: &mut [Complex32],
    focus: f32,
    shape: f32,
    amount: f32,
) {
    output.fill(Complex32::new(0.0, 0.0));
    let pivot = (focus * (1.0 + shape * 0.35)).max(2.0);
    for (harmonic, coefficient) in source.iter().copied().enumerate().skip(1) {
        let h = harmonic as f32;
        let period = pivot * 2.0;
        let folded = {
            let position = h % period;
            if position > pivot {
                period - position
            } else {
                position
            }
        }
        .max(1.0);
        scatter(output, h + (folded - h) * amount, coefficient);
    }
}

fn scatter(output: &mut [Complex32], position: f32, coefficient: Complex32) {
    if !(1.0..(output.len() - 1) as f32).contains(&position) {
        return;
    }
    let lower = position.floor() as usize;
    let fraction = position - lower as f32;
    output[lower] += coefficient * (1.0 - fraction);
    if lower + 1 < output.len() - 1 {
        output[lower + 1] += coefficient * fraction;
    }
}

fn normalize_cycle(cycle: &mut [f32]) {
    let scale = CYCLE_LEN as f32;
    let peak = cycle
        .iter()
        .fold(0.0_f32, |peak, sample| peak.max(sample.abs() / scale));
    let gain = if peak > 1.0e-6 { 0.82 / peak } else { 0.0 } / scale;
    for sample in cycle {
        *sample *= gain;
    }
}

fn read_cycle(cycle: &[f32], phase: f32) -> f32 {
    let position = phase * CYCLE_LEN as f32;
    let index = position.floor() as usize;
    let fraction = position - index as f32;
    let y0 = cycle[(index + CYCLE_LEN - 1) % CYCLE_LEN];
    let y1 = cycle[index % CYCLE_LEN];
    let y2 = cycle[(index + 1) % CYCLE_LEN];
    let y3 = cycle[(index + 2) % CYCLE_LEN];
    let a = -0.5 * y0 + 1.5 * y1 - 1.5 * y2 + 0.5 * y3;
    let b = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
    let c = -0.5 * y0 + 0.5 * y2;
    ((a * fraction + b) * fraction + c) * fraction + y1
}

fn resample_cycle(cycle: &[f32], output: &mut [f32]) {
    let length = output.len().max(1) as f32;
    for (index, sample) in output.iter_mut().enumerate() {
        *sample = read_cycle(cycle, index as f32 / length);
    }
}

fn render_key(settings: ShapeSettings, effect: SpectralEffect, sample_rate: f32) -> u64 {
    let quantize = |value: f32, scale: f32| (value * scale).round() as u64;
    (settings.waveform as u64)
        | ((effect as u64) << 2)
        | (quantize(settings.frequency_hz, 4.0) << 4)
        | (quantize(settings.pulse_width, 256.0) << 20)
        | (quantize(settings.center_hz, 0.125) << 29)
        | (quantize(settings.spread_octaves, 64.0) << 42)
        | (quantize(settings.mix, 128.0) << 51)
            ^ quantize(settings.sweep_phase, 128.0).rotate_left(17)
            ^ quantize(settings.keytrack, 128.0).rotate_left(31)
            ^ quantize(sample_rate, 0.01).rotate_left(47)
}

fn keytracked_center_hz(center_hz: f32, note_hz: f32, keytrack: f32) -> f32 {
    let ratio = (note_hz / 110.0).clamp(0.1, 64.0);
    center_hz * ratio.powf(keytrack.clamp(0.0, 1.0))
}

fn wrap01(x: f32) -> f32 {
    x - x.floor()
}

#[cfg(test)]
mod legacy_phase_tests {
    use super::{ShapeSettings, TAU, wrap01};

    #[derive(Clone, Copy, Debug)]
    pub(super) struct ShapeField {
        pub(super) amount: f32,
        pub(super) focus: f32,
        width: f32,
        intensity: f32,
    }

    #[derive(Clone, Copy, Debug)]
    pub(super) struct ShapedPhase {
        pub(super) phase: f32,
        pub(super) slope: f32,
    }

    pub(super) fn fields_for_settings(
        settings: ShapeSettings,
        frequency_hz: f32,
    ) -> (ShapeField, ShapeField) {
        let ratio = (settings.center_hz / frequency_hz).clamp(0.125, 24.0);
        let field = ShapeField {
            amount: settings.mix.clamp(0.0, 1.0),
            focus: wrap01(ratio.log2() * 0.137 + settings.sweep_phase * 0.08),
            width: 2.0_f32
                .powf(-settings.spread_octaves.clamp(0.0, 6.0))
                .clamp(0.010, 0.42),
            intensity: (ratio.log2().abs() * 0.18 + 0.55).clamp(0.35, 1.85),
        };
        (field, field)
    }

    pub(super) fn dispersion_phase(linear: f32, field: ShapeField) -> ShapedPhase {
        if field.amount <= 0.0 {
            return ShapedPhase {
                phase: linear,
                slope: 1.0,
            };
        }
        let mut distance = linear - field.focus;
        if distance > 0.5 {
            distance -= 1.0;
        } else if distance < -0.5 {
            distance += 1.0;
        }
        let normalized = distance / field.width;
        let envelope = (-normalized * normalized).exp();
        let displacement = (-distance * envelope
            + (TAU * normalized * 5.0).sin() * envelope * field.width * 0.2)
            * field.amount
            * field.intensity;
        let slope =
            1.0 + field.amount * field.intensity * (1.0 - 2.0 * normalized * normalized) * envelope;
        ShapedPhase {
            phase: wrap01(linear + displacement),
            slope: slope.clamp(0.08, 18.0),
        }
    }
}

#[cfg(test)]
use legacy_phase_tests::{dispersion_phase, fields_for_settings};

#[cfg(test)]
mod tests {
    use super::*;

    fn base_settings(mix: f32) -> ShapeSettings {
        ShapeSettings {
            waveform: Waveform::Saw,
            frequency_hz: 110.0,
            pulse_width: 0.5,
            center_hz: 700.0,
            spread_octaves: 2.2,
            mix,
            sweep_phase: 0.0,
            keytrack: 0.0,
            stereo_offset: 0.0,
        }
    }

    #[test]
    fn dry_shape_phase_is_identity() {
        let field = fields_for_settings(base_settings(0.0), 110.0).0;
        let shaped = dispersion_phase(0.37, field);

        assert!((shaped.phase - 0.37).abs() < 1e-6);
    }

    #[test]
    fn dispersion_shape_changes_phase_before_sample_generation() {
        let field = fields_for_settings(base_settings(1.0), 110.0).0;
        let shaped = dispersion_phase(0.41, field);

        assert!((shaped.phase - 0.41).abs() > 0.01);
    }

    #[test]
    fn dispersion_slope_creates_laser_like_group_delay_region() {
        let field = fields_for_settings(base_settings(1.0), 110.0).0;
        let focus = dispersion_phase(field.focus, field).slope;
        let away = dispersion_phase(wrap01(field.focus + 0.35), field).slope;

        assert!((focus - away).abs() > 0.25);
    }

    #[test]
    fn oscillator_output_changes_when_shape_mix_is_enabled() {
        let mut dry = ShapeVaOscillator::default();
        let mut wet = ShapeVaOscillator::default();
        dry.set_sample_rate(48_000.0);
        wet.set_sample_rate(48_000.0);

        let dry_settings = base_settings(0.0);
        let wet_settings = base_settings(1.0);
        let mut diff = 0.0_f32;

        for _ in 0..512 {
            let (dry_l, _) = dry.generate(dry_settings);
            let (wet_l, _) = wet.generate(wet_settings);
            diff += (dry_l - wet_l).abs();
        }

        assert!(diff > 1.0);
    }

    #[test]
    fn preview_cycle_writes_finite_single_cycle_values() {
        let mut cycle = [0.0; 128];
        preview_cycle(base_settings(1.0), &mut cycle);

        assert!(cycle.iter().all(|sample| sample.is_finite()));
    }

    #[test]
    fn oscillator_stays_finite_with_extreme_settings() {
        let mut osc = ShapeVaOscillator::default();
        osc.set_sample_rate(48_000.0);
        let mut settings = base_settings(1.0);
        settings.waveform = Waveform::Pulse;
        settings.pulse_width = 0.07;
        settings.spread_octaves = 6.0;

        let mut peak = 0.0_f32;
        for _ in 0..4096 {
            let (left, right) = osc.generate(settings);
            assert!(left.is_finite() && right.is_finite());
            peak = peak.max(left.abs()).max(right.abs());
        }

        assert!(peak <= 2.0);
    }
}
