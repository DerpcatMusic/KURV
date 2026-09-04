// Experimental, deliberately not connected to the production renderer.
// A finite Fourier series is bandlimited for constant pitch/width/coefficients.
// Audio-rate modulation (including PM) can create aliases; this is NOT a PM fix.
use std::f64::consts::{PI, TAU};
use wide::f32x8;
pub const MAX_HARMONIC: usize = 16;
#[derive(Clone, Copy, Debug)]
pub enum Shape {
    Saw,
    Triangle,
    Pulse(f64),
}
#[derive(Clone, Debug)]
pub struct Harmonics {
    pub sin: [f64; MAX_HARMONIC],
    pub cos: [f64; MAX_HARMONIC],
    pub dc: f64,
    pub count: usize,
    sine_only: bool,
    cosine_only: bool,
}
impl Harmonics {
    /// Configuration work: cache for static pitch. Rejects pitches requiring >16
    /// harmonics rather than silently throwing away wanted low-note harmonics.
    /// `taper_start` in (0,1] specifies the fraction of Nyquist where fading starts.
    /// 1 means brick-wall selection, for static reference/quality use only.
    pub fn new(step: f64, shape: Shape, taper_start: f64) -> Option<Self> {
        if !step.is_finite()
            || step <= 0.0
            || !taper_start.is_finite()
            || taper_start <= 0.0
            || taper_start > 1.0
        {
            return None;
        }
        let count = ((0.5 / step).ceil() as usize).saturating_sub(1);
        if count > MAX_HARMONIC {
            return None;
        }
        let width = match shape {
            Shape::Pulse(w) if w.is_finite() => w
                .clamp(0.03, 0.97)
                .max(step.min(0.5))
                .min(1.0 - step.min(0.5)),
            Shape::Pulse(_) => return None,
            _ => 0.5,
        };
        let mut out = Self {
            sin: [0.0; MAX_HARMONIC],
            cos: [0.0; MAX_HARMONIC],
            dc: match shape {
                Shape::Pulse(_) => 2.0 * width - 1.0,
                _ => 0.0,
            },
            count,
            sine_only: matches!(shape, Shape::Saw),
            cosine_only: matches!(shape, Shape::Triangle),
        };
        for k in 1..=count {
            let kf = k as f64;
            let x = if taper_start == 1.0 {
                1.0
            } else {
                ((1.0 - 2.0 * kf * step) / (1.0 - taper_start)).clamp(0.0, 1.0)
            };
            let gain = x * x * (3.0 - 2.0 * x);
            match shape {
                Shape::Saw => out.sin[k - 1] = -2.0 / (PI * kf) * gain,
                Shape::Triangle => {
                    if k % 2 == 1 {
                        out.cos[k - 1] = -8.0 / (PI * PI * kf * kf) * gain
                    }
                }
                Shape::Pulse(_) => {
                    let (s, c) = (TAU * kf * width).sin_cos();
                    out.cos[k - 1] = 2.0 * s / (PI * kf) * gain;
                    out.sin[k - 1] = 2.0 * (1.0 - c) / (PI * kf) * gain;
                }
            }
        }
        Some(out)
    }
    /// Stateless Clenshaw; no recurrence drift, ownership byte, or reset handling.
    #[inline]
    pub fn sample(&self, phase: f64) -> f64 {
        let (s, c) = (TAU * phase).sin_cos();
        self.sample_basis(s, c)
    }
    #[inline]
    pub fn sample_fast(&self, phase: f64) -> f64 {
        let p = phase as f32;
        let s = super::antialias::aligned_sine_phase(p + 0.25) as f64;
        let c = -super::antialias::aligned_sine_phase(p) as f64;
        self.sample_basis(s, c)
    }
    #[inline]
    fn sample_basis(&self, s: f64, c: f64) -> f64 {
        if self.sine_only || self.cosine_only {
            let coefficients = if self.sine_only { &self.sin } else { &self.cos };
            let (mut b1, mut b2) = (0.0, 0.0);
            for i in (0..self.count).rev() {
                let b = (2.0 * c).mul_add(b1, coefficients[i] - b2);
                b2 = b1;
                b1 = b;
            }
            return if self.sine_only {
                s * b1
            } else {
                c.mul_add(b1, -b2)
            };
        }
        let (mut s1, mut s2, mut c1, mut c2) = (0.0, 0.0, 0.0, 0.0);
        for i in (0..self.count).rev() {
            let sn = (2.0 * c).mul_add(s1, self.sin[i] - s2);
            let cn = (2.0 * c).mul_add(c1, self.cos[i] - c2);
            s2 = s1;
            s1 = sn;
            c2 = c1;
            c1 = cn;
        }
        s.mul_add(s1, c.mul_add(c1, self.dc - c2))
    }
}
#[derive(Clone)]
pub struct Harmonics8 {
    sin: [f32x8; MAX_HARMONIC],
    cos: [f32x8; MAX_HARMONIC],
    dc: f32x8,
    count: usize,
    sine_only: bool,
    cosine_only: bool,
}
impl Harmonics8 {
    pub fn new(lanes: [Harmonics; 8]) -> Self {
        Self {
            sin: std::array::from_fn(|k| {
                f32x8::from(std::array::from_fn(|l| lanes[l].sin[k] as f32))
            }),
            cos: std::array::from_fn(|k| {
                f32x8::from(std::array::from_fn(|l| lanes[l].cos[k] as f32))
            }),
            dc: f32x8::from(std::array::from_fn(|l| lanes[l].dc as f32)),
            count: lanes.iter().map(|l| l.count).max().unwrap(),
            sine_only: lanes.iter().all(|l| l.sine_only),
            cosine_only: lanes.iter().all(|l| l.cosine_only),
        }
    }
    #[inline]
    pub fn sample(&self, phase: f32x8) -> f32x8 {
        let (s, c) = (phase * f32x8::splat(std::f32::consts::TAU)).sin_cos();
        self.sample_basis(s, c)
    }
    #[inline]
    pub fn sample_fast(&self, phase: f32x8) -> f32x8 {
        let (s, c) = super::antialias::sine_cosine_phase8(phase);
        self.sample_basis(s, c)
    }
    #[inline]
    fn sample_basis(&self, s: f32x8, c: f32x8) -> f32x8 {
        let twice = c * f32x8::splat(2.0);
        if self.sine_only || self.cosine_only {
            let coefficients = if self.sine_only { &self.sin } else { &self.cos };
            let (mut b1, mut b2) = (f32x8::ZERO, f32x8::ZERO);
            for i in (0..self.count).rev() {
                let b = twice.mul_add(b1, coefficients[i] - b2);
                b2 = b1;
                b1 = b;
            }
            return if self.sine_only {
                s * b1
            } else {
                c.mul_add(b1, -b2)
            };
        }
        let (mut s1, mut s2, mut c1, mut c2) = (f32x8::ZERO, f32x8::ZERO, f32x8::ZERO, f32x8::ZERO);
        for i in (0..self.count).rev() {
            let sn = twice.mul_add(s1, self.sin[i] - s2);
            let cn = twice.mul_add(c1, self.cos[i] - c2);
            s2 = s1;
            s1 = sn;
            c2 = c1;
            c1 = cn;
        }
        s.mul_add(s1, c.mul_add(c1, self.dc - c2))
    }
}
