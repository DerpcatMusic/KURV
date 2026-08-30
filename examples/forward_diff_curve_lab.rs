#![allow(dead_code)]

#[path = "../src/dsp.rs"]
mod dsp;
#[path = "../src/oversampling.rs"]
mod oversampling;
#[path = "../src/wave_curve.rs"]
mod wave_curve;

use std::fs::File;
use std::hint::black_box;
use std::io::{BufWriter, Write};
use std::mem::size_of;
use std::time::Instant;

use oversampling::StereoOversampler;
use truce_simd::simd::{f32x4, f32x8};
use wave_curve::WaveCurveRt;

const SEGMENTS: usize = 16;

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match args.first().map(String::as_str) {
        Some("check") => check(),
        Some("drift") if args.len() == 5 => drift(&args[1..]),
        Some("render") if args.len() == 5 => render(&args[1..]),
        Some("bench") if args.len() == 6 => bench(&args[1..]),
        _ => {
            eprintln!(
                "forward_diff_curve_lab check | drift <fd-s16|fd-time4|fd-time8|fd4-16|fd8-16|fd8-32> <fft-bin> <lanes> <frames> | render <mode> <fft-bin> <samples> <output.f32> | bench <raw1|raw2|fd-s16|fd-time4|fd-time8|fd4-16|fd8-16|fd8-32> <fft-bin> <lanes> <frames> <repeats>"
            );
            std::process::exit(2);
        }
    }
}

fn check() {
    for &bin in &[1, 89, 601, 4806, 7000] {
        for mode in [
            "fd-s16", "fd-time4", "fd-time8", "fd4-16", "fd8-16", "fd8-32",
        ] {
            let stats = measure_drift(mode, bin, 8, 262_144);
            assert!(stats.max_abs < 2.0e-5, "{mode}/{bin}: {}", stats.max_abs);
        }
    }
    println!(
        "forward-difference checks passed; scalar={}B,time4={}B,time8={}B,x4={}B,x8={}B",
        size_of::<ScalarFd>(),
        size_of::<TimeFd<4>>(),
        size_of::<TimeFd<8>>(),
        size_of::<Fd4>(),
        size_of::<Fd8>()
    );
}

#[derive(Default)]
struct DriftStats {
    max_abs: f32,
    rms: f64,
    boundary_max: f32,
    candidate_peak: f32,
    reference_peak: f32,
}

fn drift(args: &[String]) {
    let stats = measure_drift(
        &args[0],
        parse_usize(&args[1]),
        parse_usize(&args[2]),
        parse_usize(&args[3]),
    );
    println!(
        "mode={},bin={},lanes={},frames={},max_abs={:.9e},rms={:.9e},boundary_max={:.9e},candidate_peak={:.9},reference_peak={:.9}",
        args[0],
        args[1],
        args[2],
        args[3],
        stats.max_abs,
        stats.rms,
        stats.boundary_max,
        stats.candidate_peak,
        stats.reference_peak
    );
}

fn measure_drift(mode: &str, bin: usize, lanes: usize, frames: usize) -> DriftStats {
    assert!(matches!(lanes, 1 | 8 | 64));
    let mut raw = Engine::new("raw1", bin, lanes);
    let mut candidate = Engine::new(mode, bin, lanes);
    let mut stats = DriftStats::default();
    let mut square = 0.0;
    let mut old_segments = vec![0; lanes];
    for _ in 0..frames {
        for (old, phase) in old_segments.iter_mut().zip(&raw.phases) {
            *old = segment(*phase);
        }
        let reference = raw.frame();
        let actual = candidate.frame();
        let difference = (actual - reference).abs();
        stats.max_abs = stats.max_abs.max(difference);
        square += f64::from(difference) * f64::from(difference);
        stats.candidate_peak = stats.candidate_peak.max(actual.abs());
        stats.reference_peak = stats.reference_peak.max(reference.abs());
        if raw
            .phases
            .iter()
            .zip(&old_segments)
            .any(|(phase, old)| segment(*phase) != *old)
        {
            stats.boundary_max = stats.boundary_max.max(difference);
        }
    }
    stats.rms = (square / frames as f64).sqrt();
    stats
}

fn render(args: &[String]) {
    let mut engine = Engine::new(&args[0], parse_usize(&args[1]), 1);
    let samples = parse_usize(&args[2]);
    let mut output = BufWriter::new(File::create(&args[3]).expect("create output"));
    for frame in 0..samples * 2 {
        let sample = engine.frame();
        if frame >= samples {
            output
                .write_all(&sample.to_le_bytes())
                .expect("write sample");
        }
    }
}

fn bench(args: &[String]) {
    let mode = args[0].as_str();
    let bin = parse_usize(&args[1]);
    let lanes = parse_usize(&args[2]);
    let frames = parse_usize(&args[3]);
    let repeats = parse_usize(&args[4]);
    assert!(matches!(lanes, 1 | 8 | 64));
    let mut measurements = Vec::with_capacity(repeats);
    let mut checksum = 0.0;
    for _ in 0..repeats {
        let mut engine = Engine::new(mode, bin, lanes);
        for _ in 0..4096 {
            checksum += black_box(engine.frame());
        }
        let started = Instant::now();
        for _ in 0..frames {
            checksum += black_box(engine.frame());
        }
        measurements.push(started.elapsed().as_nanos() as f64 / frames as f64);
    }
    measurements.sort_by(f64::total_cmp);
    println!(
        "mode={mode},bin={bin},lanes={lanes},frames={frames},repeats={repeats},median_ns_per_frame={:.3},min_ns_per_frame={:.3},max_ns_per_frame={:.3},checksum={checksum:.9}",
        measurements[repeats / 2],
        measurements[0],
        measurements[repeats - 1]
    );
}

struct Engine {
    mode: u8,
    factor: u8,
    curve: WaveCurveRt,
    phases: Vec<f32>,
    step: f32,
    scalar: Vec<ScalarFd>,
    time4: Vec<TimeFd<4>>,
    time8: Vec<TimeFd<8>>,
    states4: Vec<Fd4>,
    states8: Vec<Fd8>,
    oversampler: StereoOversampler,
}

impl Engine {
    fn new(mode: &str, bin: usize, lanes: usize) -> Self {
        let (mode, factor) = parse_mode(mode);
        let mut oversampler = StereoOversampler::default();
        oversampler.reset(factor);
        oversampler.set_spline_correction_immediate(factor == 2);
        Self {
            mode,
            factor,
            curve: WaveCurveRt::default(),
            phases: (0..lanes).map(|lane| lane as f32 / lanes as f32).collect(),
            step: bin as f32 / 65_536.0 / f32::from(factor),
            scalar: (0..lanes).map(|_| ScalarFd::default()).collect(),
            time4: (0..lanes).map(|_| TimeFd::default()).collect(),
            time8: (0..lanes).map(|_| TimeFd::default()).collect(),
            states4: (0..lanes.div_ceil(4)).map(|_| Fd4::default()).collect(),
            states8: (0..lanes.div_ceil(8)).map(|_| Fd8::default()).collect(),
            oversampler,
        }
    }

    fn frame(&mut self) -> f32 {
        for _ in 0..self.factor {
            let mixed = match self.mode {
                0 => process_raw(&self.curve, &mut self.phases, self.step),
                1 => process_scalar(
                    &self.curve,
                    &mut self.scalar,
                    &mut self.phases,
                    self.step,
                    16,
                ),
                2 => process4(
                    &self.curve,
                    &mut self.states4,
                    &mut self.phases,
                    self.step,
                    16,
                ),
                3 => process8(
                    &self.curve,
                    &mut self.states8,
                    &mut self.phases,
                    self.step,
                    16,
                ),
                4 => process8(
                    &self.curve,
                    &mut self.states8,
                    &mut self.phases,
                    self.step,
                    32,
                ),
                5 => process_time(&self.curve, &mut self.time4, &mut self.phases, self.step),
                6 => process_time(&self.curve, &mut self.time8, &mut self.phases, self.step),
                _ => unreachable!(),
            };
            self.oversampler.push(black_box(mixed), black_box(mixed));
        }
        self.oversampler.output().0
    }
}

#[derive(Clone)]
struct ScalarFd {
    y: f32,
    d1: f32,
    d2: f32,
    d3: f32,
    segment: u8,
    countdown: u8,
}

impl Default for ScalarFd {
    fn default() -> Self {
        Self {
            y: 0.0,
            d1: 0.0,
            d2: 0.0,
            d3: 0.0,
            segment: u8::MAX,
            countdown: 0,
        }
    }
}

impl ScalarFd {
    fn next(&mut self, curve: &WaveCurveRt, phase: f32, step: f32, resync: u8) -> f32 {
        let current_segment = segment(phase) as u8;
        if self.segment != current_segment || self.countdown == 0 {
            let [a, b, c, d] = coefficients(curve, usize::from(current_segment));
            let t = phase.mul_add(SEGMENTS as f32, -f32::from(current_segment));
            let h = step * SEGMENTS as f32;
            self.y = a.mul_add(t, b).mul_add(t, c).mul_add(t, d);
            self.d1 = a * (3.0 * t * t * h + 3.0 * t * h * h + h * h * h)
                + b * (2.0 * t * h + h * h)
                + c * h;
            self.d2 = a * (6.0 * t * h * h + 6.0 * h * h * h) + 2.0 * b * h * h;
            self.d3 = 6.0 * a * h * h * h;
            self.segment = current_segment;
            self.countdown = resync;
        }
        let output = self.y.clamp(-1.0, 1.0);
        self.y += self.d1;
        self.d1 += self.d2;
        self.d2 += self.d3;
        self.countdown -= 1;
        output
    }
}

#[derive(Clone)]
struct TimeFd<const N: usize> {
    forward: ScalarFd,
    values: [f32; N],
    index: usize,
}

impl<const N: usize> Default for TimeFd<N> {
    fn default() -> Self {
        Self {
            forward: ScalarFd::default(),
            values: [0.0; N],
            index: N,
        }
    }
}

impl<const N: usize> TimeFd<N> {
    fn next(&mut self, curve: &WaveCurveRt, phase: f32, step: f32) -> f32 {
        if self.index == N {
            let mut future = phase;
            for value in &mut self.values {
                *value = self.forward.next(curve, future, step, 16);
                future = wrap(future + step);
            }
            self.index = 0;
        }
        let output = self.values[self.index];
        self.index += 1;
        output
    }
}

#[derive(Clone)]
struct Fd4 {
    y: f32x4,
    d1: f32x4,
    d2: f32x4,
    d3: f32x4,
    segment: [u8; 4],
    countdown: [u8; 4],
}

impl Default for Fd4 {
    fn default() -> Self {
        Self {
            y: f32x4::ZERO,
            d1: f32x4::ZERO,
            d2: f32x4::ZERO,
            d3: f32x4::ZERO,
            segment: [u8::MAX; 4],
            countdown: [0; 4],
        }
    }
}

#[derive(Clone)]
struct Fd8 {
    y: f32x8,
    d1: f32x8,
    d2: f32x8,
    d3: f32x8,
    segment: [u8; 8],
    countdown: [u8; 8],
}

impl Default for Fd8 {
    fn default() -> Self {
        Self {
            y: f32x8::ZERO,
            d1: f32x8::ZERO,
            d2: f32x8::ZERO,
            d3: f32x8::ZERO,
            segment: [u8::MAX; 8],
            countdown: [0; 8],
        }
    }
}

macro_rules! vector_fd {
    ($state:ty, $vector:ty, $width:expr) => {
        impl $state {
            fn next(
                &mut self,
                curve: &WaveCurveRt,
                phases: [f32; $width],
                step: f32,
                resync: u8,
                active: usize,
            ) -> $vector {
                let mut reload = false;
                for lane in 0..active {
                    let current = segment(phases[lane]) as u8;
                    if self.segment[lane] != current || self.countdown[lane] == 0 {
                        reload = true;
                    }
                }
                if reload {
                    let mut y: [f32; $width] = self.y.into();
                    let mut d1: [f32; $width] = self.d1.into();
                    let mut d2: [f32; $width] = self.d2.into();
                    let mut d3: [f32; $width] = self.d3.into();
                    let h = step * SEGMENTS as f32;
                    for lane in 0..active {
                        let current = segment(phases[lane]) as u8;
                        if self.segment[lane] != current || self.countdown[lane] == 0 {
                            let [a, b, c, d] = coefficients(curve, usize::from(current));
                            let t = phases[lane].mul_add(SEGMENTS as f32, -f32::from(current));
                            y[lane] = a.mul_add(t, b).mul_add(t, c).mul_add(t, d);
                            d1[lane] = a * (3.0 * t * t * h + 3.0 * t * h * h + h * h * h)
                                + b * (2.0 * t * h + h * h)
                                + c * h;
                            d2[lane] = a * (6.0 * t * h * h + 6.0 * h * h * h) + 2.0 * b * h * h;
                            d3[lane] = 6.0 * a * h * h * h;
                            self.segment[lane] = current;
                            self.countdown[lane] = resync;
                        }
                    }
                    self.y = <$vector>::from(y);
                    self.d1 = <$vector>::from(d1);
                    self.d2 = <$vector>::from(d2);
                    self.d3 = <$vector>::from(d3);
                }
                let output = self.y.fast_max(-<$vector>::ONE).fast_min(<$vector>::ONE);
                self.y += self.d1;
                self.d1 += self.d2;
                self.d2 += self.d3;
                for lane in 0..active {
                    self.countdown[lane] -= 1;
                }
                output
            }
        }
    };
}

vector_fd!(Fd4, f32x4, 4);
vector_fd!(Fd8, f32x8, 8);

fn process_raw(curve: &WaveCurveRt, phases: &mut [f32], step: f32) -> f32 {
    if phases.len() >= 8 {
        return phases
            .chunks_exact_mut(8)
            .map(|phase| {
                let next = std::array::from_fn(|lane| wrap(phase[lane] + step));
                phase.copy_from_slice(&next);
                let values: [f32; 8] = curve.eval8(f32x8::from(next)).into();
                values.into_iter().sum::<f32>()
            })
            .sum();
    }
    phases
        .iter_mut()
        .map(|phase| {
            *phase = wrap(*phase + step);
            curve.eval(*phase)
        })
        .sum()
}

fn process_scalar(
    curve: &WaveCurveRt,
    states: &mut [ScalarFd],
    phases: &mut [f32],
    step: f32,
    resync: u8,
) -> f32 {
    states
        .iter_mut()
        .zip(phases)
        .map(|(state, phase)| {
            *phase = wrap(*phase + step);
            state.next(curve, *phase, step, resync)
        })
        .sum()
}

fn process_time<const N: usize>(
    curve: &WaveCurveRt,
    states: &mut [TimeFd<N>],
    phases: &mut [f32],
    step: f32,
) -> f32 {
    states
        .iter_mut()
        .zip(phases)
        .map(|(state, phase)| {
            *phase = wrap(*phase + step);
            state.next(curve, *phase, step)
        })
        .sum()
}

fn process4(
    curve: &WaveCurveRt,
    states: &mut [Fd4],
    phases: &mut [f32],
    step: f32,
    resync: u8,
) -> f32 {
    states
        .iter_mut()
        .zip(phases.chunks_mut(4))
        .map(|(state, phase)| {
            let mut p = [0.0; 4];
            for lane in 0..phase.len() {
                phase[lane] = wrap(phase[lane] + step);
                p[lane] = phase[lane];
            }
            let values: [f32; 4] = state.next(curve, p, step, resync, phase.len()).into();
            values[..phase.len()].iter().sum::<f32>()
        })
        .sum()
}

fn process8(
    curve: &WaveCurveRt,
    states: &mut [Fd8],
    phases: &mut [f32],
    step: f32,
    resync: u8,
) -> f32 {
    states
        .iter_mut()
        .zip(phases.chunks_mut(8))
        .map(|(state, phase)| {
            let mut p = [0.0; 8];
            for lane in 0..phase.len() {
                phase[lane] = wrap(phase[lane] + step);
                p[lane] = phase[lane];
            }
            let values: [f32; 8] = state.next(curve, p, step, resync, phase.len()).into();
            values[..phase.len()].iter().sum::<f32>()
        })
        .sum()
}

fn coefficients(curve: &WaveCurveRt, segment: usize) -> [f32; 4] {
    let values = curve.coefficients();
    if cfg!(all(
        target_arch = "x86_64",
        target_feature = "avx2",
        target_feature = "fma"
    )) {
        std::array::from_fn(|coefficient| values[coefficient * SEGMENTS + segment])
    } else {
        std::array::from_fn(|coefficient| values[segment * 4 + coefficient])
    }
}

fn segment(phase: f32) -> usize {
    ((phase * SEGMENTS as f32) as usize).min(SEGMENTS - 1)
}
fn wrap(phase: f32) -> f32 {
    if phase >= 1.0 { phase - 1.0 } else { phase }
}
fn parse_mode(mode: &str) -> (u8, u8) {
    match mode {
        "raw1" => (0, 1),
        "raw2" => (0, 2),
        "fd-s16" => (1, 1),
        "fd4-16" => (2, 1),
        "fd8-16" => (3, 1),
        "fd8-32" => (4, 1),
        "fd-time4" => (5, 1),
        "fd-time8" => (6, 1),
        _ => panic!("unknown mode"),
    }
}
fn parse_usize(value: &str) -> usize {
    value.parse().expect("unsigned integer")
}
