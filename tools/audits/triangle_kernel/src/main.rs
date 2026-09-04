#![allow(dead_code)]
use std::{hint::black_box, time::Instant};
use wide::{f32x4, f32x8};
mod before {
    include!(concat!(env!("OUT_DIR"), "/before.rs"));
}
mod after {
    include!(concat!(env!("OUT_DIR"), "/after.rs"));
}

fn verify() {
    let mut comparisons = 0;
    for optimized in [false, true] {
        let a = if optimized {
            before::Antialiasing::SplineOptimized
        } else {
            before::Antialiasing::Spline
        };
        let b = if optimized {
            after::Antialiasing::SplineOptimized
        } else {
            after::Antialiasing::Spline
        };
        for step in [
            0.0_f32,
            f32::EPSILON,
            0.0001,
            0.001,
            0.01,
            0.1,
            0.124999,
            0.125,
            0.249999,
            0.25,
            0.4,
            0.499,
        ] {
            for i in 0..32768 {
                let phases: [f32; 8] =
                    std::array::from_fn(|lane| ((i * 8 + lane) as f32 / 262144.0).fract());
                let steps =
                    std::array::from_fn(|lane| if lane % 3 == 0 { step * 0.51 } else { step });
                let old: [f32; 8] =
                    before::bandlimited_triangle8(phases.into(), steps.into(), a).into();
                let new: [f32; 8] =
                    after::bandlimited_triangle8(phases.into(), steps.into(), b).into();
                let old4: [f32; 4] = before::bandlimited_triangle4(
                    f32x4::from(&phases[..4]),
                    f32x4::from(&steps[..4]),
                    a,
                )
                .into();
                let new4: [f32; 4] = after::bandlimited_triangle4(
                    f32x4::from(&phases[..4]),
                    f32x4::from(&steps[..4]),
                    b,
                )
                .into();
                for lane in 0..8 {
                    assert_eq!(
                        old[lane].to_bits(),
                        new[lane].to_bits(),
                        "8: {step} {}",
                        phases[lane]
                    );
                    let x = before::bandlimited_triangle(
                        f64::from(phases[lane]),
                        f64::from(steps[lane]),
                        a,
                    );
                    let y = after::bandlimited_triangle(
                        f64::from(phases[lane]),
                        f64::from(steps[lane]),
                        b,
                    );
                    assert_eq!(x.to_bits(), y.to_bits(), "scalar: {step} {}", phases[lane]);
                    comparisons += 2;
                }
                for lane in 0..4 {
                    assert_eq!(old4[lane].to_bits(), new4[lane].to_bits());
                    comparisons += 1;
                }
            }
        }
    }
    eprintln!("PASS {comparisons} bit-identical output comparisons");
}

fn workload<F: FnMut(&[f32], &[f32]) -> f32>(lanes: usize, mode: usize, mut kernel: F) -> f32 {
    let mut phases = [0.0_f32; 64];
    for (i, p) in phases.iter_mut().enumerate() {
        *p = i as f32 * 0.6180339 % 1.0;
    }
    let mut sum = 0.0;
    for i in 0..262144 {
        // Precomputed in the full synth in many paths; this inexpensive deterministic
        // driver varies step per sample, including nested modulation of modulation.
        let t = (i & 1023) as f32 / 1024.0;
        let modulator = (t * 17.0 + (t * 7.0).fract() * 3.0).fract();
        let step = match mode {
            0 => 0.01,
            1 => 0.1,
            2 => 0.001 + modulator * 0.2,
            _ => 0.26 + modulator * 0.23,
        };
        let steps: [f32; 64] = std::array::from_fn(|lane| step * (1.0 - lane as f32 * 0.001));
        for lane in 0..lanes {
            phases[lane] += steps[lane];
            if phases[lane] >= 1.0 {
                phases[lane] -= 1.0;
            }
        }
        sum += kernel(black_box(&phases[..lanes]), black_box(&steps[..lanes]));
    }
    black_box(sum)
}

macro_rules! kernel {
    ($module:ident, $optimized:expr) => {
        |phases: &[f32], steps: &[f32]| -> f32 {
            let aa = if $optimized {
                $module::Antialiasing::SplineOptimized
            } else {
                $module::Antialiasing::Spline
            };
            if phases.len() == 1 {
                return $module::bandlimited_triangle(f64::from(phases[0]), f64::from(steps[0]), aa)
                    as f32;
            }
            if phases.len() == 4 {
                let out: [f32; 4] =
                    $module::bandlimited_triangle4(f32x4::from(phases), f32x4::from(steps), aa)
                        .into();
                return out.iter().sum();
            }
            let mut total = 0.0;
            for (p, s) in phases.chunks_exact(8).zip(steps.chunks_exact(8)) {
                let out: [f32; 8] =
                    $module::bandlimited_triangle8(f32x8::from(p), f32x8::from(s), aa).into();
                total += out.iter().sum::<f32>();
            }
            total
        }
    };
}
fn main() {
    verify();
    if std::env::args().any(|arg| arg == "--verify-only") {
        return;
    }
    println!("optimized,lanes,mode,before_ns,after_ns,change_pct");
    for optimized in [false, true] {
        for lanes in [1, 4, 8, 16, 64] {
            if lanes != 1 && std::env::args().any(|arg| arg == "--scalar-only") {
                continue;
            }
            for (mode, name) in ["low", "high", "nested_rate", "overlap"]
                .into_iter()
                .enumerate()
            {
                let mut old = Vec::new();
                let mut new = Vec::new();
                for repeat in 0..9 {
                    // Alternate order to reduce drift bias. First pair is warm-up.
                    let mut run = |after: bool| {
                        let start = Instant::now();
                        if after {
                            black_box(workload(lanes, mode, kernel!(after, optimized)));
                        } else {
                            black_box(workload(lanes, mode, kernel!(before, optimized)));
                        }
                        let elapsed = start.elapsed().as_nanos() as f64 / 262144.0;
                        if repeat > 0 {
                            if after {
                                new.push(elapsed);
                            } else {
                                old.push(elapsed);
                            }
                        }
                    };
                    run(repeat % 2 == 0);
                    run(repeat % 2 != 0);
                }
                old.sort_by(f64::total_cmp);
                new.sort_by(f64::total_cmp);
                let a = (old[3] + old[4]) / 2.0;
                let b = (new[3] + new[4]) / 2.0;
                println!(
                    "{optimized},{lanes},{name},{a:.3},{b:.3},{:.2}",
                    (b / a - 1.0) * 100.0
                );
            }
        }
    }
}
