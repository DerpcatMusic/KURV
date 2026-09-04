//! Objeckt's physical-object core adapted as a per-voice modal filter.
//!
//! Adapted from OBJEKT, Copyright (c) 2026 Derpcat, under the ISC license.
//! This keeps the ISC modal bank and its JOS/DF2T resonators. The standalone
//! processor's multi-second delay/feedback and soft clip stay out: a KURV
//! filter is per voice and remains a linear audio-domain filter.

use truce_simd::simd::f32x4;

pub(in crate::filters::engine) const MODE_COUNT: usize = 17;

const LN_001: f32 = -6.907_755;
const TWO_PI: f32 = std::f32::consts::TAU;
const NYQUIST_GUARD: f32 = 0.45;
const R_MAX: f32 = 0.999_7;
const BP_GAIN: f32 = 0.11;
const ENERGY_REF: f32 = 0.055;
const IDENTITY_COUNT: usize = 8;
const MORPH_MAX: f32 = 7.0;

#[derive(Clone, Copy)]
struct ModeSpec {
    ratio: f32,
    nx: u8,
    ny: u8,
}

#[derive(Clone, Copy)]
enum Shape {
    Line,
    Rect,
    Circle,
}

const fn spec(ratio: f32, nx: u8, ny: u8) -> ModeSpec {
    ModeSpec { ratio, nx, ny }
}

const SHAPE: [Shape; IDENTITY_COUNT] = [
    Shape::Rect,
    Shape::Circle,
    Shape::Rect,
    Shape::Line,
    Shape::Line,
    Shape::Line,
    Shape::Line,
    Shape::Line,
];

const MEMBRANE: [ModeSpec; MODE_COUNT] = [
    spec(1.000, 1, 1),
    spec(1.387, 2, 1),
    spec(1.754, 1, 2),
    spec(1.861, 3, 1),
    spec(2.000, 2, 2),
    spec(2.353, 3, 2),
    spec(2.370, 4, 1),
    spec(2.557, 1, 3),
    spec(2.732, 2, 3),
    spec(2.773, 4, 2),
    spec(2.896, 5, 1),
    spec(3.000, 3, 3),
    spec(3.235, 5, 2),
    spec(3.340, 4, 3),
    spec(3.374, 1, 4),
    spec(3.431, 6, 1),
    spec(3.508, 2, 4),
];

const DRUMHEAD: [ModeSpec; MODE_COUNT] = [
    spec(1.000, 0, 1),
    spec(1.593, 1, 1),
    spec(2.135, 2, 1),
    spec(2.295, 0, 2),
    spec(2.653, 3, 1),
    spec(2.917, 1, 2),
    spec(3.155, 4, 1),
    spec(3.500, 2, 2),
    spec(3.598, 0, 3),
    spec(3.647, 5, 1),
    spec(4.059, 3, 2),
    spec(4.132, 6, 1),
    spec(4.230, 1, 3),
    spec(4.601, 4, 2),
    spec(4.832, 2, 3),
    spec(4.903, 0, 4),
    spec(5.131, 5, 2),
];

const PLATE: [ModeSpec; MODE_COUNT] = [
    spec(1.000, 1, 1),
    spec(1.708, 2, 1),
    spec(2.887, 3, 1),
    spec(3.292, 1, 2),
    spec(4.000, 2, 2),
    spec(4.538, 4, 1),
    spec(5.179, 3, 2),
    spec(6.660, 5, 1),
    spec(6.830, 4, 2),
    spec(7.113, 1, 3),
    spec(7.821, 2, 3),
    spec(8.953, 5, 2),
    spec(8.999, 3, 3),
    spec(9.255, 6, 1),
    spec(10.651, 4, 3),
    spec(11.547, 6, 2),
    spec(12.321, 7, 1),
];

const STRING: [ModeSpec; MODE_COUNT] = [
    spec(1.0, 1, 0),
    spec(2.0, 2, 0),
    spec(3.0, 3, 0),
    spec(4.0, 4, 0),
    spec(5.0, 5, 0),
    spec(6.0, 6, 0),
    spec(7.0, 7, 0),
    spec(8.0, 8, 0),
    spec(9.0, 9, 0),
    spec(10.0, 10, 0),
    spec(11.0, 11, 0),
    spec(12.0, 12, 0),
    spec(13.0, 13, 0),
    spec(14.0, 14, 0),
    spec(15.0, 15, 0),
    spec(16.0, 16, 0),
    spec(17.0, 17, 0),
];

const BEAM: [ModeSpec; MODE_COUNT] = [
    spec(1.000, 1, 0),
    spec(2.757, 2, 0),
    spec(5.404, 3, 0),
    spec(8.933, 4, 0),
    spec(13.344, 5, 0),
    spec(18.638, 6, 0),
    spec(24.813, 7, 0),
    spec(31.871, 8, 0),
    spec(39.810, 9, 0),
    spec(48.632, 10, 0),
    spec(58.335, 11, 0),
    spec(68.920, 12, 0),
    spec(80.387, 13, 0),
    spec(92.736, 14, 0),
    spec(105.97, 15, 0),
    spec(120.08, 16, 0),
    spec(135.18, 17, 0),
];

const MARIMBA: [ModeSpec; MODE_COUNT] = [
    spec(1.00, 1, 0),
    spec(4.00, 2, 0),
    spec(9.20, 3, 0),
    spec(16.0, 4, 0),
    spec(24.2, 5, 0),
    spec(33.8, 6, 0),
    spec(45.0, 7, 0),
    spec(57.6, 8, 0),
    spec(71.8, 9, 0),
    spec(87.4, 10, 0),
    spec(104.4, 11, 0),
    spec(122.8, 12, 0),
    spec(142.6, 13, 0),
    spec(163.8, 14, 0),
    spec(186.4, 15, 0),
    spec(210.4, 16, 0),
    spec(235.8, 17, 0),
];

const PIPE: [ModeSpec; MODE_COUNT] = [
    spec(1.0, 1, 0),
    spec(3.0, 3, 0),
    spec(5.0, 5, 0),
    spec(7.0, 7, 0),
    spec(9.0, 9, 0),
    spec(11.0, 11, 0),
    spec(13.0, 13, 0),
    spec(15.0, 15, 0),
    spec(17.0, 17, 0),
    spec(19.0, 19, 0),
    spec(21.0, 21, 0),
    spec(23.0, 23, 0),
    spec(25.0, 25, 0),
    spec(27.0, 27, 0),
    spec(29.0, 29, 0),
    spec(31.0, 31, 0),
    spec(33.0, 33, 0),
];

const IDENTITIES: [[ModeSpec; MODE_COUNT]; IDENTITY_COUNT] = [
    MEMBRANE, DRUMHEAD, PLATE, STRING, BEAM, MARIMBA, PIPE, STRING,
];
const STRIKE: [f32; IDENTITY_COUNT] = [0.42, 0.18, 0.31, 0.20, 0.22, 0.27, 0.55, 0.50];
const BETA: [f32; IDENTITY_COUNT] = [1.35, 1.50, 0.22, 0.55, 0.90, 1.05, 0.40, 0.48];

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(in crate::filters::engine) struct ObjectCoefficients {
    pub(in crate::filters::engine) gain: [f32; MODE_COUNT],
    pub(in crate::filters::engine) a1: [f32; MODE_COUNT],
    pub(in crate::filters::engine) a2: [f32; MODE_COUNT],
}

impl ObjectCoefficients {
    pub(in crate::filters::engine) fn new(
        morph: f32,
        freq_hz: f32,
        decay_s: f32,
        material: f32,
        formant: f32,
        sample_rate: f32,
    ) -> Self {
        let sr = sample_rate.max(8_000.0);
        let nyquist = sr * NYQUIST_GUARD;
        let f0 = freq_hz.clamp(20.0, nyquist);
        let decay = decay_s.clamp(0.02, 12.0);
        let material = material.clamp(0.0, 1.0);
        let morph = morph.clamp(0.0, 1.0) * MORPH_MAX;
        let (_, identity_beta) = shape_at(morph);
        let wood = 1.0 - material;
        let beta = (1.55 * wood).mul_add(wood, identity_beta.mul_add(0.45, 0.06));
        let beta = beta.clamp(0.05, 2.8);
        let ratios = ratios_at(morph);
        let weights = weights_at(morph, formant);
        let fade_lo = sr * 0.36;
        let mut frequencies = [0.0; MODE_COUNT];
        let mut radii = [0.0; MODE_COUNT];
        let mut gains = [0.0; MODE_COUNT];
        let mut weight_energy = 0.0;
        for i in 0..MODE_COUNT {
            frequencies[i] = ratios[i] * f0;
            let fade = nyquist_fade(frequencies[i], fade_lo, nyquist);
            let ratio = (frequencies[i] / f0).max(0.2);
            let t60 = (decay / (beta * ratio).mul_add(ratio, 1.0)).clamp(0.008, 20.0);
            gains[i] = weights[i] * fade * BP_GAIN;
            weight_energy += gains[i] * gains[i];
            radii[i] = (LN_001 / (t60 * sr)).exp().clamp(0.0, R_MAX);
        }
        let makeup = (ENERGY_REF / weight_energy.max(1.0e-10))
            .sqrt()
            .clamp(0.35, 2.8);
        let mut a1 = [0.0; MODE_COUNT];
        let mut a2 = [0.0; MODE_COUNT];
        for i in 0..MODE_COUNT {
            let theta = TWO_PI * frequencies[i].min(nyquist) / sr;
            let radius = radii[i];
            a1[i] = -2.0 * radius * theta.cos();
            a2[i] = radius * radius;
            gains[i] *= makeup;
        }
        Self {
            gain: gains,
            a1,
            a2,
        }
    }

    pub(in crate::filters::engine) fn interpolate(self, target: Self, amount: f32) -> Self {
        let amount = amount.clamp(0.0, 1.0);
        Self {
            gain: std::array::from_fn(|i| self.gain[i] + amount * (target.gain[i] - self.gain[i])),
            a1: std::array::from_fn(|i| self.a1[i] + amount * (target.a1[i] - self.a1[i])),
            a2: std::array::from_fn(|i| self.a2[i] + amount * (target.a2[i] - self.a2[i])),
        }
    }
}

#[inline]
pub(in crate::filters::engine) fn process_object(
    z1: &mut [f32x4; MODE_COUNT],
    z2: &mut [f32x4; MODE_COUNT],
    input: f32x4,
    coefficients: ObjectCoefficients,
) -> f32x4 {
    let mut output = f32x4::ZERO;
    for i in 0..MODE_COUNT {
        let gain = f32x4::splat(coefficients.gain[i]);
        let y = gain * input + z1[i];
        z1[i] = z2[i] - f32x4::splat(coefficients.a1[i]) * y;
        z2[i] = -gain * input - f32x4::splat(coefficients.a2[i]) * y;
        output += y;
    }
    output
}

pub(in crate::filters::engine) fn mode_frequency(
    morph: f32,
    freq_hz: f32,
    index: usize,
) -> Option<f32> {
    ratios_at(morph.clamp(0.0, 1.0) * MORPH_MAX)
        .get(index)
        .map(|ratio| freq_hz.clamp(20.0, f32::MAX) * ratio)
}

fn ratios_at(morph: f32) -> [f32; MODE_COUNT] {
    let (left, right, frac) = lerp_identities(morph);
    let a = IDENTITIES[left];
    let b = IDENTITIES[right];
    std::array::from_fn(|i| (b[i].ratio - a[i].ratio).mul_add(frac, a[i].ratio))
}

fn shape_at(morph: f32) -> (f32, f32) {
    let (left, right, frac) = lerp_identities(morph);
    (
        (STRIKE[right] - STRIKE[left]).mul_add(frac, STRIKE[left]),
        (BETA[right] - BETA[left]).mul_add(frac, BETA[left]),
    )
}

fn weights_at(morph: f32, formant: f32) -> [f32; MODE_COUNT] {
    let (left, right, frac) = lerp_identities(morph);
    let strike = formant.clamp(0.05, 0.95);
    let a = identity_weights(left, strike);
    if frac < 1.0e-4 {
        return a;
    }
    let b = identity_weights(right, strike);
    std::array::from_fn(|i| (b[i] - a[i]).mul_add(frac, a[i]))
}

fn identity_weights(identity: usize, strike: f32) -> [f32; MODE_COUNT] {
    let modes = IDENTITIES[identity];
    let mut out = [0.0; MODE_COUNT];
    match SHAPE[identity] {
        Shape::Line => {
            for (slot, mode) in out.iter_mut().zip(modes) {
                let n = f32::from(mode.nx.max(1));
                *slot = (n * std::f32::consts::PI * strike).sin().abs().max(0.04);
            }
        }
        Shape::Rect => {
            for (slot, mode) in out.iter_mut().zip(modes) {
                let wx = (f32::from(mode.nx) * std::f32::consts::PI * strike)
                    .sin()
                    .abs();
                let wy = (f32::from(mode.ny.max(1)) * std::f32::consts::PI * 0.37)
                    .sin()
                    .abs();
                *slot = (wx * wy).max(0.03);
            }
        }
        Shape::Circle => {
            let radius = (1.0 - strike).mul_add(0.70, 0.12).clamp(0.08, 0.86);
            for (slot, mode) in out.iter_mut().zip(modes) {
                let d = u32::from(mode.nx);
                let arg = mode.ratio * 2.404_826 * radius;
                *slot = bessel_j(d, arg).abs().max(0.03);
            }
        }
    }
    out
}

fn lerp_identities(morph: f32) -> (usize, usize, f32) {
    let morph = morph.clamp(0.0, MORPH_MAX);
    let left = (morph.floor() as usize).min(IDENTITY_COUNT - 1);
    let frac = morph - left as f32;
    if frac < 1.0e-4 || left + 1 >= IDENTITY_COUNT {
        (left, left, 0.0)
    } else {
        (left, left + 1, frac)
    }
}

fn bessel_j(n: u32, x: f32) -> f32 {
    if x.abs() < 1.0e-7 {
        return if n == 0 { 1.0 } else { 0.0 };
    }
    let xh = x * 0.5;
    let mut term = 1.0;
    for i in 1..=n {
        term *= xh / i as f32;
    }
    let mut sum = term;
    let xh2 = xh * xh;
    for k in 1..18 {
        term *= -xh2 / (k as f32 * (n + k) as f32);
        sum += term;
        if term.abs() < 1.0e-8 * sum.abs().max(1.0) {
            break;
        }
    }
    sum
}

fn nyquist_fade(freq: f32, lo: f32, hi: f32) -> f32 {
    if freq >= hi {
        0.0
    } else if freq <= lo {
        1.0
    } else {
        let t = (freq - lo) / (hi - lo).max(1.0);
        f32::midpoint(1.0, (t * std::f32::consts::PI).cos())
    }
}
