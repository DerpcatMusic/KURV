#![allow(dead_code)]

#[path = "../src/dsp.rs"]
mod dsp;
#[path = "../src/wave_curve.rs"]
mod wave_curve;

use std::hint::black_box;
use std::time::Instant;

use truce_simd::simd::f32x8;
use wave_curve::WaveCurveRt;

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match args.first().map(String::as_str) {
        Some("check") => check(),
        Some("bench") if args.len() == 5 => bench(&args[1..]),
        _ => {
            eprintln!(
                "time_pack_curve_lab check | bench <scalar|pack8> <fft-bin> <blocks> <repeats>"
            );
            std::process::exit(2);
        }
    }
}

fn check() {
    for &bin in &[1, 89, 601, 7000] {
        check_size::<31>(bin);
        check_size::<32>(bin);
        check_size::<33>(bin);
    }
    println!("time-pack checks passed: outputs and continuation phases are bit-identical");
}

fn check_size<const SAMPLES: usize>(bin: usize) {
    let curve = WaveCurveRt::default();
    let step = bin as f32 / 65_536.0;
    let mut scalar_phase = 0.317_382_8;
    let mut packed_phase = scalar_phase;
    for _ in 0..32_768 {
        let scalar = scalar_block::<SAMPLES>(&curve, &mut scalar_phase, step);
        let packed = packed_block::<SAMPLES>(&curve, &mut packed_phase, step);
        assert!(
            scalar
                .iter()
                .zip(packed)
                .all(|(a, b)| a.to_bits() == b.to_bits()),
            "output mismatch at bin {bin}, size {SAMPLES}"
        );
        assert_eq!(
            scalar_phase.to_bits(),
            packed_phase.to_bits(),
            "phase mismatch at bin {bin}, size {SAMPLES}"
        );
    }
}

fn bench(args: &[String]) {
    let mode = args[0].as_str();
    let bin = parse(&args[1]);
    let blocks = parse(&args[2]);
    let repeats = parse(&args[3]);
    let curve = WaveCurveRt::default();
    let step = bin as f32 / 65_536.0;
    let mut measurements = Vec::with_capacity(repeats);
    let mut checksum = 0.0;
    for _ in 0..repeats {
        let mut phase = 0.317_382_8;
        for _ in 0..4096 {
            checksum += process(mode, &curve, &mut phase, step);
        }
        let start = Instant::now();
        for _ in 0..blocks {
            checksum += black_box(process(mode, &curve, &mut phase, step));
        }
        measurements.push(start.elapsed().as_nanos() as f64 / (blocks * 32) as f64);
    }
    measurements.sort_by(f64::total_cmp);
    println!(
        "mode={mode},bin={bin},blocks={blocks},repeats={repeats},median_ns_per_sample={:.3},min_ns_per_sample={:.3},max_ns_per_sample={:.3},checksum={checksum:.9}",
        measurements[repeats / 2],
        measurements[0],
        measurements[repeats - 1]
    );
}

fn process(mode: &str, curve: &WaveCurveRt, phase: &mut f32, step: f32) -> f32 {
    let samples = match mode {
        "scalar" => scalar_block::<32>(curve, phase, step),
        "pack8" => packed_block::<32>(curve, phase, step),
        _ => panic!("unknown mode"),
    };
    samples
        .into_iter()
        .enumerate()
        .fold(0.0, |sum, (index, sample)| {
            (sample * (index as f32 + 1.0)).mul_add(0.125, sum)
        })
}

fn scalar_block<const SAMPLES: usize>(
    curve: &WaveCurveRt,
    phase: &mut f32,
    step: f32,
) -> [f32; SAMPLES] {
    std::array::from_fn(|_| {
        let sample = curve.eval(*phase);
        *phase = wrap(*phase + step);
        sample
    })
}

fn packed_block<const SAMPLES: usize>(
    curve: &WaveCurveRt,
    phase: &mut f32,
    step: f32,
) -> [f32; SAMPLES] {
    let mut output = [0.0; SAMPLES];
    let mut offset = 0;
    while offset + 8 <= SAMPLES {
        let phases = std::array::from_fn(|_| {
            let current = *phase;
            *phase = wrap(*phase + step);
            current
        });
        let samples: [f32; 8] = curve.eval8(f32x8::from(phases)).into();
        output[offset..offset + 8].copy_from_slice(&samples);
        offset += 8;
    }
    while offset < SAMPLES {
        output[offset] = curve.eval(*phase);
        *phase = wrap(*phase + step);
        offset += 1;
    }
    output
}

fn wrap(phase: f32) -> f32 {
    if phase >= 1.0 { phase - 1.0 } else { phase }
}

fn parse(value: &str) -> usize {
    value.parse().expect("unsigned integer")
}
