#![allow(dead_code)]

#[path = "../src/dsp.rs"]
mod dsp;
#[path = "../src/wave_curve.rs"]
mod wave_curve;

use std::hint::black_box;
use std::time::Instant;

use dsp::{Complex, fft};
use truce_simd::simd::f32x8;
use wave_curve::{WaveCurveData, WaveCurveRt, WaveKnot};

const GRID: usize = 65_536;
const SEGMENTS: usize = 16;
const CAPS: [usize; 5] = [2, 3, 6, 13, 27];

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match args.first().map(String::as_str) {
        Some("report") => report(),
        Some("bench") => bench(),
        _ => {
            eprintln!("coefficient_mip_lab <report|bench>");
            std::process::exit(2);
        }
    }
}

#[derive(Clone, Copy)]
enum Shape {
    Saw,
    Square,
    Pulse,
    Triangle,
    Drawn,
}

impl Shape {
    const ALL: [Self; 5] = [
        Self::Saw,
        Self::Square,
        Self::Pulse,
        Self::Triangle,
        Self::Drawn,
    ];

    const fn name(self) -> &'static str {
        match self {
            Self::Saw => "saw",
            Self::Square => "square",
            Self::Pulse => "pulse37",
            Self::Triangle => "triangle",
            Self::Drawn => "drawn",
        }
    }

    fn source(self, phase: f32, drawn: WaveCurveRt) -> f32 {
        match self {
            Self::Saw => phase.mul_add(2.0, -1.0),
            Self::Square => {
                if phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            Self::Pulse => {
                if phase < 0.37 {
                    1.0
                } else {
                    -1.0
                }
            }
            Self::Triangle => 1.0 - 4.0 * (phase - 0.5).abs(),
            Self::Drawn => drawn.eval(phase),
        }
    }
}

fn report() {
    let drawn = drawn_curve();
    for shape in Shape::ALL {
        let spectrum = source_spectrum(shape, drawn);
        let frames = CAPS.map(|cap| compile_projection(&spectrum, cap));
        for (cap, frame) in CAPS.into_iter().zip(frames) {
            let (rms, peak) = projection_error(frame, &spectrum, cap);
            println!(
                "projection,shape={},cap={cap},rms={rms:.9},peak={peak:.9}",
                shape.name()
            );
        }
        for band in 0..CAPS.len() - 1 {
            let (sweep_peak, transition_peak, coefficient_peak) = sweep_error(
                frames[band],
                frames[band + 1],
                &spectrum,
                CAPS[band],
                CAPS[band + 1],
            );
            println!(
                "sweep,shape={},caps={}-{},sample_step_peak={sweep_peak:.9},same_phase_transition_peak={transition_peak:.9},coefficient_error_peak={coefficient_peak:.9}",
                shape.name(),
                CAPS[band],
                CAPS[band + 1]
            );
        }
    }
    println!(
        "storage,frames={},bytes_per_frame={},bytes_per_curve={}",
        CAPS.len(),
        std::mem::size_of::<WaveCurveRt>(),
        CAPS.len() * std::mem::size_of::<WaveCurveRt>()
    );
}

fn bench() {
    let spectrum = source_spectrum(Shape::Drawn, drawn_curve());
    let frames = CAPS.map(|cap| compile_projection(&spectrum, cap));
    let mut times = Vec::new();
    let mut checksum = 0.0;
    for _ in 0..7 {
        let started = Instant::now();
        for index in 0..5_000_000 {
            checksum += black_box(frames[2].eval(black_box((index & 65_535) as f32 / 65_536.0)));
        }
        times.push(started.elapsed().as_nanos() as f64 / 5_000_000.0);
    }
    times.sort_by(f64::total_cmp);
    let mut x8_times = Vec::new();
    for _ in 0..7 {
        let started = Instant::now();
        for index in 0..1_000_000 {
            let base = (index & 65_535) as f32 / 65_536.0;
            checksum += black_box(frames[2].eval8(f32x8::from(std::array::from_fn(|lane| {
                (base + lane as f32 * 0.071).fract()
            }))))
            .reduce_add();
        }
        x8_times.push(started.elapsed().as_nanos() as f64 / 8_000_000.0);
    }
    x8_times.sort_by(f64::total_cmp);
    let started = Instant::now();
    for index in 0..500_000 {
        checksum += black_box(interpolate(
            frames[1],
            frames[2],
            black_box((index & 255) as f32 / 255.0),
        ))
        .eval(black_box((index & 65_535) as f32 / 65_536.0));
    }
    let interpolation = started.elapsed().as_nanos() as f64 / 500_000.0;
    println!(
        "bench,eval_ns={:.3},eval8_ns_per_sample={:.3},coefficient_interpolate_plus_eval_ns={interpolation:.3},checksum={checksum:.9}",
        times[times.len() / 2],
        x8_times[x8_times.len() / 2]
    );
}

fn source_spectrum(shape: Shape, drawn: WaveCurveRt) -> Vec<Complex> {
    let mut samples = (0..GRID)
        .map(|index| {
            Complex::new(
                f64::from(shape.source(index as f32 / GRID as f32, drawn)),
                0.0,
            )
        })
        .collect::<Vec<_>>();
    fft(&mut samples, false);
    for value in &mut samples {
        *value /= GRID as f64;
    }
    samples
}

fn projected(spectrum: &[Complex], cap: usize, phase: f64) -> f32 {
    let mut value = spectrum[0].re;
    for harmonic in 1..=cap {
        value += 2.0
            * (spectrum[harmonic]
                * Complex::from_polar(1.0, std::f64::consts::TAU * harmonic as f64 * phase))
            .re;
    }
    value as f32
}

fn compile_projection(spectrum: &[Complex], cap: usize) -> WaveCurveRt {
    let mut coefficients = [0.0; 64];
    for segment in 0..SEGMENTS {
        let phase = segment as f64 / SEGMENTS as f64;
        let step = 1.0 / SEGMENTS as f64;
        let y0 = projected(spectrum, cap, phase);
        let y1 = projected(spectrum, cap, phase + step / 3.0);
        let y2 = projected(spectrum, cap, phase + step * 2.0 / 3.0);
        let y3 = projected(spectrum, cap, phase + step);
        let p = y1 - y0;
        let q = y2 - y0;
        let r = y3 - y0;
        let values = [
            4.5_f32.mul_add(r, 13.5 * (p - q)),
            (-4.5_f32).mul_add(r, (-22.5_f32).mul_add(p, 18.0 * q)),
            9.0_f32.mul_add(p, (-4.5_f32).mul_add(q, r)),
            y0,
        ];
        for (plane, value) in values.into_iter().enumerate() {
            coefficients[coefficient_index(segment, plane)] = value;
        }
    }
    WaveCurveRt::from_coefficients(coefficients)
}

fn projection_error(curve: WaveCurveRt, spectrum: &[Complex], cap: usize) -> (f64, f32) {
    let mut square = 0.0;
    let mut peak = 0.0_f32;
    for index in 0..GRID {
        let phase = index as f32 / GRID as f32;
        let error = curve.eval(phase) - projected(spectrum, cap, f64::from(phase));
        square += f64::from(error) * f64::from(error);
        peak = peak.max(error.abs());
    }
    ((square / GRID as f64).sqrt(), peak)
}

fn sweep_error(
    low: WaveCurveRt,
    high: WaveCurveRt,
    spectrum: &[Complex],
    low_cap: usize,
    high_cap: usize,
) -> (f32, f32, f32) {
    let mut previous = interpolate(low, high, 0.0).eval(0.173);
    let mut sample_step_peak = 0.0_f32;
    let mut coefficient_error_peak = 0.0_f32;
    for index in 1..=16_384 {
        let mix = index as f32 / 16_384.0;
        let phase = (0.173 + index as f32 * 0.007_31).fract();
        let curve = interpolate(low, high, mix);
        let sample = curve.eval(phase);
        sample_step_peak = sample_step_peak.max((sample - previous).abs());
        previous = sample;
        let ideal = projected(spectrum, low_cap, f64::from(phase)) * (1.0 - mix)
            + projected(spectrum, high_cap, f64::from(phase)) * mix;
        coefficient_error_peak = coefficient_error_peak.max((sample - ideal).abs());
    }
    let before = interpolate(low, high, 0.5 - 1.0 / 16_384.0);
    let after = interpolate(low, high, 0.5 + 1.0 / 16_384.0);
    let transition_peak = (0..GRID).fold(0.0_f32, |peak, index| {
        let phase = index as f32 / GRID as f32;
        peak.max((after.eval(phase) - before.eval(phase)).abs())
    });
    (sample_step_peak, transition_peak, coefficient_error_peak)
}

fn interpolate(a: WaveCurveRt, b: WaveCurveRt, mix: f32) -> WaveCurveRt {
    WaveCurveRt::interpolate(a, b, mix)
}

fn coefficient_index(segment: usize, plane: usize) -> usize {
    if cfg!(all(
        target_arch = "x86_64",
        target_feature = "avx2",
        target_feature = "fma"
    )) {
        plane * SEGMENTS + segment
    } else {
        segment * 4 + plane
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
