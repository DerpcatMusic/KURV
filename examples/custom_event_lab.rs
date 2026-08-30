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
use truce_simd::simd::f32x8;
use wave_curve::WaveCurveRt;
use wide::CmpLt;

const EVENTS: [(f32, f32); 2] = [(0.25, -8.0), (0.75, 8.0)];

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match args.first().map(String::as_str) {
        Some("check") => check(),
        Some("render") if args.len() == 5 => render(&args[1..]),
        Some("bench") if args.len() == 6 => bench(&args[1..]),
        _ => {
            eprintln!(
                "custom_event_lab check | render <raw1|events1|raw2> <fft-bin> <samples> <output.f32> | bench <raw1|events1|raw2> <fft-bin> <lanes> <frames> <repeats>"
            );
            std::process::exit(2);
        }
    }
}

fn check() {
    let curve = WaveCurveRt::default();
    let step = 601.0 / 65_536.0;
    let phases = f32x8::from([0.249, 0.251, 0.50, 0.749, 0.751, 0.1, 0.9, 0.0]);
    let vector: [f32; 8] = event_sample8(&curve, phases, f32x8::splat(step)).into();
    let scalar_phases: [f32; 8] = phases.into();
    for (phase, actual) in scalar_phases.into_iter().zip(vector) {
        let expected = event_sample(&curve, phase, step);
        assert!(
            (actual - expected).abs() < 2.0e-6,
            "{phase}: {actual} != {expected}"
        );
    }
    println!("custom-event scalar/SIMD check passed");
}

fn render(args: &[String]) {
    let (events, factor) = mode(&args[0]);
    let fft_bin = parse_usize(&args[1]);
    let samples = parse_usize(&args[2]);
    let internal_samples = samples * usize::from(factor);
    let step = fft_bin as f32 / internal_samples as f32;
    let curve = WaveCurveRt::default();
    let mut oversampler = StereoOversampler::default();
    oversampler.reset(factor);
    oversampler.set_spline_correction_immediate(factor == 2);
    let mut output = BufWriter::new(File::create(&args[3]).expect("create output"));
    for frame in 0..samples * 2 {
        for subframe in 0..usize::from(factor) {
            let index = frame * usize::from(factor) + subframe;
            let phase = ((index * fft_bin) % internal_samples) as f32 / internal_samples as f32;
            let sample = if events {
                event_sample(&curve, phase, step)
            } else {
                curve.eval(phase)
            };
            oversampler.push(sample, sample);
        }
        let sample = oversampler.output().0;
        if frame >= samples {
            output
                .write_all(&sample.to_le_bytes())
                .expect("write sample");
        }
    }
}

fn bench(args: &[String]) {
    let (events, factor) = mode(&args[0]);
    let fft_bin = parse_usize(&args[1]);
    let lanes = parse_usize(&args[2]);
    let frames = parse_usize(&args[3]);
    let repeats = parse_usize(&args[4]);
    assert!(lanes > 0 && lanes % 8 == 0);
    let step = f32x8::splat(fft_bin as f32 / 65_536.0 / f32::from(factor));
    let curve = WaveCurveRt::default();
    let groups = lanes / 8;
    let initial = (0..groups)
        .map(|group| {
            f32x8::from(std::array::from_fn(|lane| {
                ((group * 8 + lane) as f32 / lanes as f32).fract()
            }))
        })
        .collect::<Vec<_>>();
    let mut measurements = Vec::with_capacity(repeats);
    let mut checksum = 0.0_f32;
    for _ in 0..repeats {
        let mut phases = initial.clone();
        let mut oversampler = StereoOversampler::default();
        oversampler.reset(factor);
        oversampler.set_spline_correction_immediate(factor == 2);
        for _ in 0..4096 {
            run_frame(
                events,
                factor,
                &curve,
                step,
                &mut phases,
                &mut oversampler,
                &mut checksum,
            );
        }
        let start = Instant::now();
        for _ in 0..frames {
            run_frame(
                events,
                factor,
                &curve,
                step,
                &mut phases,
                &mut oversampler,
                &mut checksum,
            );
        }
        measurements.push(start.elapsed().as_nanos() as f64 / frames as f64);
    }
    measurements.sort_by(f64::total_cmp);
    println!(
        "mode={},fft_bin={fft_bin},lanes={lanes},frames={frames},repeats={repeats},median_ns_per_frame={:.3},min_ns_per_frame={:.3},max_ns_per_frame={:.3},checksum={checksum:.9}",
        args[0],
        measurements[repeats / 2],
        measurements[0],
        measurements[repeats - 1]
    );
}

#[inline(always)]
fn run_frame(
    events: bool,
    factor: u8,
    curve: &WaveCurveRt,
    step: f32x8,
    phases: &mut [f32x8],
    oversampler: &mut StereoOversampler,
    checksum: &mut f32,
) {
    for _ in 0..factor {
        let mut mixed = 0.0;
        for phase in &mut *phases {
            let sample = if events {
                event_sample8(curve, *phase, step)
            } else {
                curve.eval8(*phase)
            };
            *phase = wrap8(*phase + step);
            let values: [f32; 8] = black_box(sample).into();
            mixed += values.into_iter().sum::<f32>();
        }
        oversampler.push(mixed, mixed);
    }
    *checksum += black_box(oversampler.output().0);
}

#[inline(always)]
fn event_sample(curve: &WaveCurveRt, phase: f32, step: f32) -> f32 {
    EVENTS
        .iter()
        .fold(curve.eval(phase), |sample, &(event, slope_jump)| {
            slope_jump.mul_add(
                step * blamp_residual(signed_distance(phase, event) / step),
                sample,
            )
        })
}

#[inline(always)]
fn event_sample8(curve: &WaveCurveRt, phase: f32x8, step: f32x8) -> f32x8 {
    EVENTS
        .iter()
        .fold(curve.eval8(phase), |sample, &(event, slope_jump)| {
            let position = signed_distance8(phase, f32x8::splat(event)) / step;
            (step * f32x8::splat(slope_jump)).mul_add(blamp_residual8(position), sample)
        })
}

#[inline(always)]
fn blamp_residual(position: f32) -> f32 {
    let distance = position.abs();
    if distance >= 2.0 {
        return 0.0;
    }
    if distance < 1.0 {
        0.018_896_732_f32
            .mul_add(distance, -0.068_349_12)
            .mul_add(distance, -0.000_122_838_29)
            .mul_add(distance, 0.313_372_55)
            .mul_add(distance, -0.5)
            .mul_add(distance, 0.248_045_86)
    } else {
        let tail = 2.0 - distance;
        0.005_821_323_f32
            .mul_add(tail, 0.006_685_827_4)
            .mul_add(tail, -0.001_985_740_6)
            .mul_add(tail, 0.001_321_771_3)
            * tail
            * tail
    }
}

#[inline(always)]
fn blamp_residual8(position: f32x8) -> f32x8 {
    let distance = position.abs();
    let event = distance.cmp_lt(f32x8::splat(2.0));
    if !event.any() {
        return f32x8::ZERO;
    }
    let inner = f32x8::splat(0.018_896_732)
        .mul_add(distance, f32x8::splat(-0.068_349_12))
        .mul_add(distance, f32x8::splat(-0.000_122_838_29))
        .mul_add(distance, f32x8::splat(0.313_372_55))
        .mul_add(distance, f32x8::splat(-0.5))
        .mul_add(distance, f32x8::splat(0.248_045_86));
    let tail = f32x8::splat(2.0) - distance;
    let outer = f32x8::splat(0.005_821_323)
        .mul_add(tail, f32x8::splat(0.006_685_827_4))
        .mul_add(tail, f32x8::splat(-0.001_985_740_6))
        .mul_add(tail, f32x8::splat(0.001_321_771_3))
        * tail
        * tail;
    event.blend(distance.cmp_lt(f32x8::ONE).blend(inner, outer), f32x8::ZERO)
}

#[inline(always)]
fn signed_distance(phase: f32, event: f32) -> f32 {
    let distance = phase - event;
    if distance < -0.5 {
        distance + 1.0
    } else if distance > 0.5 {
        distance - 1.0
    } else {
        distance
    }
}

#[inline(always)]
fn signed_distance8(phase: f32x8, event: f32x8) -> f32x8 {
    let distance = phase - event;
    distance
        .cmp_lt(f32x8::splat(-0.5))
        .blend(distance + f32x8::ONE, distance)
        .cmp_lt(f32x8::splat(0.5))
        .blend(distance, distance - f32x8::ONE)
}

#[inline(always)]
fn wrap(phase: f32) -> f32 {
    if phase >= 1.0 { phase - 1.0 } else { phase }
}

#[inline(always)]
fn wrap8(phase: f32x8) -> f32x8 {
    phase.cmp_lt(f32x8::ONE).blend(phase, phase - f32x8::ONE)
}

fn mode(value: &str) -> (bool, u8) {
    match value {
        "raw1" => (false, 1),
        "events1" => (true, 1),
        "raw2" => (false, 2),
        _ => panic!("mode must be raw1, events1, or raw2"),
    }
}

fn parse_usize(value: &str) -> usize {
    value.parse().expect("unsigned integer")
}
