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
use std::time::Instant;

use oversampling::StereoOversampler;
use truce_simd::simd::{f32x4, f32x8};
use wave_curve::{WaveCurveData, WaveCurveRt, WaveKnot};

const SEGMENTS: usize = 16;

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match args.first().map(String::as_str) {
        Some("check") => check(),
        Some("render") if args.len() == 7 => render(&args[1..]),
        Some("bench") if args.len() == 8 => bench(&args[1..]),
        _ => {
            eprintln!(
                "adaa_curve_lab check | render <raw1|raw2|box-s|box4|box8|tri-s|tri4|tri8> <static|glide|jump|morph|morph-jump> <start-bin> <end-bin> <samples> <output.f32> | bench <mode> <motion> <start-bin> <end-bin> <lanes> <frames> <repeats>"
            );
            std::process::exit(2);
        }
    }
}

fn check() {
    let curve = WaveCurveRt::default();
    for &(phase, step) in &[
        (0.0, 0.0),
        (0.249, 1.0e-8),
        (0.251, 601.0 / 65_536.0),
        (0.99, 7000.0 / 65_536.0),
    ] {
        let box_s = box_average(&curve, phase, step);
        let tri_s = triangle_average(&curve, phase, step);
        let box4: [f32; 4] = box_average4(&curve, f32x4::splat(phase), f32x4::splat(step)).into();
        let box8: [f32; 8] = box_average8(&curve, f32x8::splat(phase), f32x8::splat(step)).into();
        let tri4: [f32; 4] =
            triangle_average4(&curve, f32x4::splat(phase), f32x4::splat(step)).into();
        let tri8: [f32; 8] =
            triangle_average8(&curve, f32x8::splat(phase), f32x8::splat(step)).into();
        assert!((box_s - box4[0]).abs() < 3.0e-6 && (box_s - box8[0]).abs() < 3.0e-6);
        assert!((tri_s - tri4[0]).abs() < 8.0e-6 && (tri_s - tri8[0]).abs() < 8.0e-6);
        assert!(box_s.is_finite() && tri_s.is_finite());
    }
    let complex = complex_curve();
    let phase = 0.99;
    let step = 0.11;
    let points = 262_144;
    let midpoint = (0..points)
        .map(|index| {
            let offset = (index as f32 + 0.5) / points as f32 - 0.5;
            complex.eval(wrap(phase + offset * step))
        })
        .sum::<f32>()
        / points as f32;
    let exact = box_average(&complex, phase, step);
    assert!(
        (exact - midpoint).abs() < 3.0e-6,
        "exact={exact}, midpoint={midpoint}"
    );
    println!(
        "exact box/triangle scalar-x4-x8 boundary, wrap, complex-curve, and near-equal checks passed; extra state=0B"
    );
}

fn render(args: &[String]) {
    let mode = args[0].as_str();
    let motion = args[1].as_str();
    let start = parse_f32(&args[2]);
    let end = parse_f32(&args[3]);
    let samples = parse_usize(&args[4]);
    let mut engine = Engine::new(mode, 1);
    let mut output = Vec::with_capacity(samples * 5);
    for frame in 0..samples * 5 {
        let (active_motion, motion_frame, motion_frames) = if frame < samples * 4 {
            ("static", frame, samples * 4)
        } else {
            (motion, frame - samples * 4, samples)
        };
        output.push(engine.frame(active_motion, start, end, motion_frame, motion_frames));
    }
    let mut file = BufWriter::new(File::create(&args[5]).expect("create output"));
    for sample in &output[samples * 4..] {
        file.write_all(&sample.to_le_bytes()).expect("write sample");
    }
}

fn bench(args: &[String]) {
    let mode = args[0].as_str();
    let motion = args[1].as_str();
    let start = parse_f32(&args[2]);
    let end = parse_f32(&args[3]);
    let lanes = parse_usize(&args[4]);
    let frames = parse_usize(&args[5]);
    let repeats = parse_usize(&args[6]);
    assert!(matches!(lanes, 1 | 8 | 64));
    let mut measurements = Vec::with_capacity(repeats);
    let mut checksum = 0.0;
    for _ in 0..repeats {
        let mut engine = Engine::new(mode, lanes);
        for frame in 0..4096 {
            checksum += black_box(engine.frame(motion, start, end, frame, frames));
        }
        let started = Instant::now();
        for frame in 0..frames {
            checksum += black_box(engine.frame(motion, start, end, frame, frames));
        }
        measurements.push(started.elapsed().as_nanos() as f64 / frames as f64);
    }
    measurements.sort_by(f64::total_cmp);
    println!(
        "mode={mode},motion={motion},start_bin={start},end_bin={end},lanes={lanes},frames={frames},repeats={repeats},median_ns_per_frame={:.3},min_ns_per_frame={:.3},max_ns_per_frame={:.3},checksum={checksum:.9}",
        measurements[repeats / 2],
        measurements[0],
        measurements[repeats - 1]
    );
}

struct Engine {
    mode: u8,
    factor: u8,
    phases: Vec<f32>,
    steps: Vec<f32>,
    base: WaveCurveRt,
    target: WaveCurveRt,
    oversampler: StereoOversampler,
}

impl Engine {
    fn new(mode: &str, lanes: usize) -> Self {
        let (mode, factor) = parse_mode(mode);
        let mut oversampler = StereoOversampler::default();
        oversampler.reset(factor);
        Self {
            mode,
            factor,
            phases: (0..lanes).map(|lane| lane as f32 / lanes as f32).collect(),
            steps: vec![0.0; lanes],
            base: WaveCurveRt::default(),
            target: complex_curve(),
            oversampler,
        }
    }

    fn frame(&mut self, motion: &str, start: f32, end: f32, frame: usize, frames: usize) -> f32 {
        for subframe in 0..self.factor {
            let internal_frame = frame * usize::from(self.factor) + usize::from(subframe);
            let internal_frames = frames * usize::from(self.factor);
            fill_steps(
                &mut self.steps,
                motion,
                start / f32::from(self.factor),
                end / f32::from(self.factor),
                internal_frame,
                internal_frames,
            );
            let morph = morph_amount(motion, internal_frame, internal_frames);
            let curve = if morph == 0.0 {
                self.base
            } else {
                WaveCurveRt::interpolate(self.base, self.target, morph)
            };
            let mixed = self.process(&curve);
            self.oversampler.push(black_box(mixed), black_box(mixed));
        }
        self.oversampler.output().0
    }

    fn process(&mut self, curve: &WaveCurveRt) -> f32 {
        match self.mode {
            0 => process_raw(curve, &mut self.phases, &self.steps),
            1 | 4 => process_scalar(curve, &mut self.phases, &self.steps, self.mode == 4),
            2 | 5 => process4(curve, &mut self.phases, &self.steps, self.mode == 5),
            3 | 6 => process8(curve, &mut self.phases, &self.steps, self.mode == 6),
            _ => unreachable!(),
        }
    }
}

fn process_raw(curve: &WaveCurveRt, phases: &mut [f32], steps: &[f32]) -> f32 {
    if phases.len() >= 8 {
        return phases
            .chunks_exact_mut(8)
            .zip(steps.chunks_exact(8))
            .map(|(phase, step)| {
                let next = std::array::from_fn(|lane| wrap(phase[lane] + step[lane]));
                phase.copy_from_slice(&next);
                let values: [f32; 8] = curve.eval8(f32x8::from(next)).into();
                values.into_iter().sum::<f32>()
            })
            .sum();
    }
    phases
        .iter_mut()
        .zip(steps)
        .map(|(phase, &step)| {
            *phase = wrap(*phase + step);
            curve.eval(*phase)
        })
        .sum()
}

fn process_scalar(curve: &WaveCurveRt, phases: &mut [f32], steps: &[f32], triangle: bool) -> f32 {
    phases
        .iter_mut()
        .zip(steps)
        .map(|(phase, &step)| {
            *phase = wrap(*phase + step);
            if triangle {
                triangle_average(curve, *phase, step)
            } else {
                box_average(curve, *phase, step)
            }
        })
        .sum()
}

fn process4(curve: &WaveCurveRt, phases: &mut [f32], steps: &[f32], triangle: bool) -> f32 {
    phases
        .chunks_mut(4)
        .zip(steps.chunks(4))
        .map(|(phase, step)| {
            let mut p = [0.0; 4];
            let mut s = [0.0; 4];
            p[..phase.len()].copy_from_slice(phase);
            s[..step.len()].copy_from_slice(step);
            for lane in 0..phase.len() {
                p[lane] = wrap(p[lane] + s[lane]);
            }
            phase.copy_from_slice(&p[..phase.len()]);
            let values: [f32; 4] = if triangle {
                triangle_average4(curve, f32x4::from(p), f32x4::from(s)).into()
            } else {
                box_average4(curve, f32x4::from(p), f32x4::from(s)).into()
            };
            values[..phase.len()].iter().sum::<f32>()
        })
        .sum()
}

fn process8(curve: &WaveCurveRt, phases: &mut [f32], steps: &[f32], triangle: bool) -> f32 {
    phases
        .chunks_mut(8)
        .zip(steps.chunks(8))
        .map(|(phase, step)| {
            let mut p = [0.0; 8];
            let mut s = [0.0; 8];
            p[..phase.len()].copy_from_slice(phase);
            s[..step.len()].copy_from_slice(step);
            for lane in 0..phase.len() {
                p[lane] = wrap(p[lane] + s[lane]);
            }
            phase.copy_from_slice(&p[..phase.len()]);
            let values: [f32; 8] = if triangle {
                triangle_average8(curve, f32x8::from(p), f32x8::from(s)).into()
            } else {
                box_average8(curve, f32x8::from(p), f32x8::from(s)).into()
            };
            values[..phase.len()].iter().sum::<f32>()
        })
        .sum()
}

fn box_average(curve: &WaveCurveRt, phase: f32, step: f32) -> f32 {
    if step.abs() < 1.0e-7 {
        return curve.eval(phase);
    }
    let phase = f64::from(phase);
    let step = f64::from(step);
    (integrate_moments(curve, phase - step * 0.5, phase + step * 0.5).0 / step) as f32
}

fn triangle_average(curve: &WaveCurveRt, phase: f32, step: f32) -> f32 {
    if step.abs() < 1.0e-5 {
        return curve.eval(phase);
    }
    let phase = f64::from(phase);
    let step = f64::from(step);
    let (left0, left1) = integrate_moments(curve, phase - step, phase);
    let (right0, right1) = integrate_moments(curve, phase, phase + step);
    ((left1 - (phase - step) * left0 + (phase + step) * right0 - right1) / (step * step)) as f32
}

fn integrate_moments(curve: &WaveCurveRt, mut start: f64, end: f64) -> (f64, f64) {
    let mut m0 = 0.0;
    let mut m1 = 0.0;
    for _ in 0..64 {
        if start >= end - 1.0e-8 {
            break;
        }
        let cycle = start.floor();
        let phase = start - cycle;
        let segment = ((phase * SEGMENTS as f64) as usize).min(SEGMENTS - 1);
        let boundary = (cycle + (segment + 1) as f64 / SEGMENTS as f64).min(end);
        let u0 = phase * SEGMENTS as f64 - segment as f64;
        let u1 = (boundary - cycle) * SEGMENTS as f64 - segment as f64;
        let [a, b, c, d] = coefficients(curve, segment);
        let [a, b, c, d] = [f64::from(a), f64::from(b), f64::from(c), f64::from(d)];
        let local0 = primitive0(a, b, c, d, u1) - primitive0(a, b, c, d, u0);
        let local1 =
            primitive1(a, b, c, d, segment as f64, u1) - primitive1(a, b, c, d, segment as f64, u0);
        m0 += local0 / SEGMENTS as f64;
        m1 += cycle * local0 / SEGMENTS as f64 + local1 / (SEGMENTS * SEGMENTS) as f64;
        start = boundary;
    }
    (m0, m1)
}

fn primitive0(a: f64, b: f64, c: f64, d: f64, u: f64) -> f64 {
    (((a * 0.25).mul_add(u, b / 3.0)).mul_add(u, c * 0.5)).mul_add(u, d) * u
}

fn primitive1(a: f64, b: f64, c: f64, d: f64, segment: f64, u: f64) -> f64 {
    let p0 = primitive0(a, b, c, d, u) * segment;
    let pu = ((((a * 0.2).mul_add(u, b * 0.25)).mul_add(u, c / 3.0)).mul_add(u, d * 0.5)) * u * u;
    p0 + pu
}

macro_rules! vector_averages {
    ($box_fn:ident, $tri_fn:ident, $vector:ty, $width:expr) => {
        fn $box_fn(curve: &WaveCurveRt, phase: $vector, step: $vector) -> $vector {
            let p: [f32; $width] = phase.into();
            let s: [f32; $width] = step.into();
            <$vector>::from(std::array::from_fn(|lane| {
                box_average(curve, p[lane], s[lane])
            }))
        }
        fn $tri_fn(curve: &WaveCurveRt, phase: $vector, step: $vector) -> $vector {
            let p: [f32; $width] = phase.into();
            let s: [f32; $width] = step.into();
            <$vector>::from(std::array::from_fn(|lane| {
                triangle_average(curve, p[lane], s[lane])
            }))
        }
    };
}

vector_averages!(box_average4, triangle_average4, f32x4, 4);
vector_averages!(box_average8, triangle_average8, f32x8, 8);

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

fn complex_curve() -> WaveCurveRt {
    WaveCurveData {
        knots: vec![
            WaveKnot {
                phase: 0.0,
                value: -0.35,
                curve: 0.8,
                curve_x: -0.4,
            },
            WaveKnot {
                phase: 0.11,
                value: 0.95,
                curve: -0.7,
                curve_x: 0.35,
            },
            WaveKnot {
                phase: 0.37,
                value: 0.15,
                curve: 0.6,
                curve_x: -0.2,
            },
            WaveKnot {
                phase: 0.58,
                value: -0.9,
                curve: -0.5,
                curve_x: 0.25,
            },
            WaveKnot {
                phase: 0.83,
                value: 0.45,
                curve: 0.7,
                curve_x: 0.1,
            },
        ],
    }
    .compile_rt()
}

fn fill_steps(steps: &mut [f32], motion: &str, start: f32, end: f32, frame: usize, frames: usize) {
    if matches!(motion, "static" | "morph" | "morph-jump") && frame != 0 {
        return;
    }
    if motion == "jump" && frame != 0 && frame != frames / 2 {
        return;
    }
    let bin = match motion {
        "glide" => (end - start).mul_add(frame as f32 / frames.max(1) as f32, start),
        "jump" => {
            if frame < frames / 2 {
                start
            } else {
                end
            }
        }
        "static" | "morph" | "morph-jump" => start,
        _ => panic!("unknown motion"),
    };
    let lanes = steps.len();
    for (lane, step) in steps.iter_mut().enumerate() {
        let cents = if lanes == 1 {
            0.0
        } else {
            (lane as f32 / (lanes - 1) as f32 - 0.5) * 10.0
        };
        *step = bin / 65_536.0 * 2.0_f32.powf(cents / 1200.0);
    }
}

fn morph_amount(motion: &str, frame: usize, frames: usize) -> f32 {
    match motion {
        "morph" => frame as f32 / frames.max(1) as f32,
        "morph-jump" => f32::from(u8::from(frame >= frames / 2)),
        _ => 0.0,
    }
}

fn parse_mode(mode: &str) -> (u8, u8) {
    match mode {
        "raw1" => (0, 1),
        "raw2" => (0, 2),
        "box-s" => (1, 1),
        "box4" => (2, 1),
        "box8" => (3, 1),
        "tri-s" => (4, 1),
        "tri4" => (5, 1),
        "tri8" => (6, 1),
        _ => panic!("unknown mode"),
    }
}

fn wrap(phase: f32) -> f32 {
    if phase >= 1.0 { phase - 1.0 } else { phase }
}
fn parse_usize(value: &str) -> usize {
    value.parse().expect("unsigned integer")
}
fn parse_f32(value: &str) -> f32 {
    value.parse().expect("number")
}
