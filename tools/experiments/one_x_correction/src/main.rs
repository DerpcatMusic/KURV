#![allow(dead_code)]
use std::{hint::black_box, io::Write, time::Instant};
mod antialias {
    include!(concat!(env!("OUT_DIR"), "/antialias.rs"));
}
#[path = "../../../../src/oscillators/va/one_x_correction.rs"]
mod candidate;
use antialias::Antialiasing::SplineOptimized;
use candidate::*;
const N: usize = 65536;
fn output<const R: usize>(
    shape: usize,
    p: f64,
    step: f64,
    c: &Correction<R>,
    table: &[[f64; 8]; R],
) -> f64 {
    if shape == 0 {
        c.saw(p, table)
    } else {
        c.pulse(p, if shape == 1 { 0.5 } else { 0.31_f64.max(step) }, table)
    }
}
fn render(method: usize, shape: usize, step: f64, n: usize) -> Vec<f64> {
    let c3 = Correction::<3>::new(step);
    let c4 = Correction::<4>::new(step);
    let c6 = Correction::<6>::new(step);
    let mut p = 0.173;
    (0..n)
        .map(|_| {
            let y = match method {
                0 => {
                    if shape == 0 {
                        antialias::bandlimited_saw(p, step, SplineOptimized)
                    } else {
                        antialias::bandlimited_pulse(
                            p,
                            step,
                            if shape == 1 { 0.5 } else { 0.31 },
                            SplineOptimized,
                        )
                    }
                }
                1 => output(shape, p, step, &c3, &KERNEL_3),
                2 => output(shape, p, step, &c4, &KERNEL_4),
                _ => output(shape, p, step, &c6, &KERNEL_6),
            };
            p += step;
            if p >= 1.0 {
                p -= 1.0;
            }
            y
        })
        .collect()
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("simd") {
        simd_benchmark();
        return;
    }
    if args.get(1).map(String::as_str) == Some("dump") {
        let mut file = std::fs::File::create(&args[2]).unwrap();
        for shape in 0..3 {
            for bin in [37, 601, 4001, 10001, 25001] {
                for method in 0..4 {
                    for y in render(method, shape, bin as f64 / N as f64, N) {
                        file.write_all(&y.to_le_bytes()).unwrap();
                    }
                }
            }
        }
        return;
    }
    println!("shape,bin,baseline_ns,k6_ratio,k8_ratio,k12_ratio");
    for shape in 0..3 {
        for bin in [37, 601, 4001, 10001, 25001] {
            let mut times = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
            for repeat in 0..9 {
                for offset in 0..4 {
                    let method = (repeat + offset) % 4;
                    let now = Instant::now();
                    black_box(render(
                        black_box(method),
                        black_box(shape),
                        black_box(bin as f64 / N as f64),
                        N,
                    ));
                    times[method].push(now.elapsed().as_nanos() as f64 / N as f64);
                }
            }
            let mut median = [0.0; 4];
            for m in 0..4 {
                times[m].sort_by(f64::total_cmp);
                median[m] = times[m][4];
            }
            println!(
                "{shape},{bin},{:.3},{:.3},{:.3},{:.3}",
                median[0],
                median[1] / median[0],
                median[2] / median[0],
                median[3] / median[0]
            );
        }
    }
}

fn simd_benchmark() {
    use wide::{CmpLt, f32x8};
    let n = 1 << 19;
    println!("hz,spread,current_ns,candidate_ns,ratio,max_scalar_simd_error");
    for hz in [27.1_f32, 440.2, 1000.0, 2930.4] {
        for spread in [false, true] {
            let step = hz / 48000.0;
            let inv = f32x8::splat(step.recip());
            let support = f32x8::splat(3.0 * step);
            let mut timings = [Vec::new(), Vec::new()];
            for repeat in 0..9 {
                for offset in 0..2 {
                    let method = (repeat + offset) % 2;
                    let mut p = f32x8::from(std::array::from_fn(|i| {
                        if spread { i as f32 / 8.0 } else { 0.173 }
                    }));
                    let dt = f32x8::splat(step);
                    let now = Instant::now();
                    let mut sum = f32x8::ZERO;
                    for _ in 0..n {
                        let y = if method == 0 {
                            p * f32x8::splat(2.0)
                                - f32x8::ONE
                                - antialias::spline_blep8_precomputed_static_with_bounds::<true>(
                                    p,
                                    f32x8::ONE.cmp_lt(f32x8::splat(2.0)),
                                    f32x8::splat(2.0 * step),
                                    f32x8::splat(1.0 - 2.0 * step),
                                    inv,
                                    true,
                                )
                        } else {
                            candidate::saw6_narrow8(p, inv, support)
                        };
                        sum += y;
                        p += dt;
                        p = p.cmp_lt(f32x8::ONE).blend(p, p - f32x8::ONE);
                    }
                    black_box(sum);
                    timings[method].push(now.elapsed().as_nanos() as f64 / n as f64);
                }
            }
            for t in &mut timings {
                t.sort_by(f64::total_cmp);
            }
            let c = candidate::Correction::<3>::new(step as f64);
            let mut err = 0.0_f64;
            for j in 0..10000 {
                let p = std::array::from_fn(|i| (j * 8 + i) as f32 / 80000.0);
                let y = candidate::saw6_narrow8(f32x8::from(p), inv, support).to_array();
                for i in 0..8 {
                    err = err.max((y[i] as f64 - c.saw(p[i] as f64, &candidate::KERNEL_3)).abs());
                }
            }
            println!(
                "{hz},{spread},{:.3},{:.3},{:.3},{err:.9}",
                timings[0][4],
                timings[1][4],
                timings[1][4] / timings[0][4]
            );
        }
    }
}
