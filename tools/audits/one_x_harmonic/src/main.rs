#![allow(dead_code)]
use std::{hint::black_box, time::Instant};
use wide::f32x8;
#[path = "../../../../src/oscillators/va/one_x_harmonic.rs"]
mod candidate;
mod antialias {
    include!(concat!(env!("OUT_DIR"), "/baseline.rs"));
}
use antialias as baseline;
use candidate::*;
fn old(shape: Shape, p: f64, d: f64) -> f64 {
    match shape {
        Shape::Saw => baseline::bandlimited_saw(p, d, baseline::Antialiasing::SplineOptimized),
        Shape::Triangle => {
            baseline::bandlimited_triangle(p, d, baseline::Antialiasing::SplineOptimized)
        }
        Shape::Pulse(w) => {
            let w = w.clamp(0.03, 0.97).max(d).min(1.0 - d);
            let raw = if p < w { 1.0 } else { -1.0 };
            raw + baseline::edge_blep(p, d, baseline::Antialiasing::SplineOptimized)
                - baseline::edge_blep(
                    (p + 1.0 - w).fract(),
                    d,
                    baseline::Antialiasing::SplineOptimized,
                )
        }
    }
}
fn old8(shape: Shape, p: f32x8, d: f32x8) -> f32x8 {
    match shape {
        Shape::Saw => baseline::bandlimited_saw8(p, d, baseline::Antialiasing::SplineOptimized),
        Shape::Triangle => {
            baseline::bandlimited_triangle8(p, d, baseline::Antialiasing::SplineOptimized)
        }
        Shape::Pulse(w) => baseline::bandlimited_saw_pulse_morph8(
            p,
            d,
            w as f32,
            1.0,
            baseline::Antialiasing::SplineOptimized,
        ),
    }
}
fn reference(h: &Harmonics, p: f64) -> f64 {
    (1..=h.count)
        .map(|k| {
            let (s, c) = (std::f64::consts::TAU * p * k as f64).sin_cos();
            h.sin[k - 1] * s + h.cos[k - 1] * c
        })
        .sum::<f64>()
        + h.dc
}
fn quality() {
    let libm = std::env::args().any(|s| s == "--libm");
    println!(
        "shape,hz,baseline_total_error_db,candidate_total_error_db,candidate_taper_error_db,simd_numerical_error_db"
    );
    for sh in [Shape::Saw, Shape::Triangle, Shape::Pulse(0.23)] {
        for hz in [
            1500.13, 3000.13, 6000.13, 8000.13, 11000.13, 16000.13, 21000.13,
        ] {
            let d = hz / 48000.0;
            let hard = Harmonics::new(d, sh, 1.0).unwrap();
            let h = Harmonics::new(d, sh, 0.9).unwrap();
            let x8 = Harmonics8::new(std::array::from_fn(|_| h.clone()));
            let (mut eo, mut ec, mut et, mut ev, mut er) = (0.0, 0.0, 0.0, 0.0, 0.0);
            for i in 0..16384 {
                let p = (i as f64 * d + 0.123).fract();
                let r = reference(&hard, p);
                let rt = reference(&h, p);
                let c = h.sample(p);
                let v: [f32; 8] = (if libm {
                    x8.sample(f32x8::splat(p as f32))
                } else {
                    x8.sample_fast(f32x8::splat(p as f32))
                })
                .into();
                eo += (old(sh, p, d) - r).powi(2);
                ec += (c - rt).powi(2);
                et += (c - r).powi(2);
                ev += (v[0] as f64 - reference(&h, p as f32 as f64)).powi(2);
                er += r * r;
                assert!((c - rt).abs() < 1e-11);
                assert!((h.sample_fast(p) - rt).abs() < 0.0001);
                assert!((v[0] as f64 - reference(&h, p as f32 as f64)).abs() < 0.0001);
            }
            println!(
                "{sh:?},{hz},{:.3},{:.3},{:.3},{:.3}",
                10.0 * (eo / er).log10(),
                10.0 * (ec / er).log10(),
                10.0 * (et / er).log10(),
                10.0 * (ev / er).log10()
            );
        }
    }
    assert!(Harmonics::new(440.0 / 48000.0, Shape::Saw, 0.9).is_none());
    assert!(Harmonics::new(f64::NAN, Shape::Saw, 0.9).is_none());
    // Coefficient continuity at all count changes and taper boundaries.
    for sh in [Shape::Saw, Shape::Triangle, Shape::Pulse(0.23)] {
        for k in 1..=16 {
            for scale in [0.9, 1.0] {
                let d = scale * 0.5 / k as f64;
                let Some(a) = Harmonics::new(d - 1e-10, sh, 0.9) else {
                    continue;
                };
                let b = Harmonics::new(d + 1e-10, sh, 0.9).unwrap();
                for i in 0..100 {
                    let p = i as f64 / 100.0;
                    assert!((a.sample(p) - b.sample(p)).abs() < 1e-6);
                }
            }
        }
    }
    let mixed: [Harmonics; 8] = std::array::from_fn(|l| {
        Harmonics::new(
            (1500.13 + l as f64 * 2700.0) / 48000.0,
            match l % 3 {
                0 => Shape::Saw,
                1 => Shape::Triangle,
                _ => Shape::Pulse(0.23),
            },
            0.9,
        )
        .unwrap()
    });
    let pack = Harmonics8::new(mixed.clone());
    for i in 0..4096 {
        let ps: [f32; 8] =
            std::array::from_fn(|l| (i as f32 * 0.0137 + l as f32 * 0.123).fract());
        let got: [f32; 8] = pack.sample_fast(ps.into()).into();
        for l in 0..8 {
            assert!((got[l] as f64 - reference(&mixed[l], ps[l] as f64)).abs() < 0.0001);
        }
    }
    eprintln!("PASS finite-series oracle, SIMD error, invalid input, harmonic/taper continuity");
}
fn timed(mut f: impl FnMut(usize) -> f64) -> f64 {
    let mut v = Vec::new();
    for _ in 0..7 {
        let t = Instant::now();
        let mut s = 0.0;
        for i in 0..262144 {
            s += black_box(f(black_box(i)));
        }
        black_box(s);
        v.push(t.elapsed().as_nanos() as f64 / 262144.0);
    }
    v.sort_by(f64::total_cmp);
    v[3]
}
fn bench() {
    let libm = std::env::args().any(|s| s == "--libm");
    println!("shape,hz,lanes,baseline_ns,candidate_ns,ratio,config_ns");
    let phases: Vec<f64> = (0..4096)
        .map(|i| (i as f64 * 0.61803398875).fract())
        .collect();
    for sh in [Shape::Saw, Shape::Triangle, Shape::Pulse(0.23)] {
        for hz in [1500.13, 3000.13, 6000.13, 11000.13, 16000.13, 21000.13] {
            let d = hz / 48000.0;
            let h = Harmonics::new(d, sh, 0.9).unwrap();
            let h8 = Harmonics8::new(std::array::from_fn(|_| h.clone()));
            let cfg = timed(|i| {
                let d = d + (i & 7) as f64 * 1e-8;
                black_box(Harmonics::new(d, sh, 0.9).unwrap()).sin[0]
            });
            for lanes in [1, 8] {
                let (a, b) = if lanes == 1 {
                    (
                        timed(|i| old(sh, phases[i & 4095], d)),
                        timed(|i| {
                            if libm {
                                h.sample(phases[i & 4095])
                            } else {
                                h.sample_fast(phases[i & 4095])
                            }
                        }),
                    )
                } else {
                    (
                        timed(|i| {
                            old8(
                                sh,
                                f32x8::splat(phases[i & 4095] as f32),
                                f32x8::splat(d as f32),
                            )
                            .reduce_add() as f64
                        }),
                        timed(|i| {
                            (if libm {
                                h8.sample(f32x8::splat(phases[i & 4095] as f32))
                            } else {
                                h8.sample_fast(f32x8::splat(phases[i & 4095] as f32))
                            })
                            .reduce_add() as f64
                        }),
                    )
                };
                println!("{sh:?},{hz},{lanes},{a:.3},{b:.3},{:.3},{cfg:.3}", b / a);
            }
        }
    }
}
fn spectrum() {
    let libm = std::env::args().any(|s| s == "--libm");
    println!("shape,hz,baseline_unwanted_db,candidate_unwanted_db");
    let n = 32768;
    for sh in [Shape::Saw, Shape::Triangle, Shape::Pulse(0.23)] {
        for bin in [1025, 2049, 4097, 5461, 7509, 10923, 14337] {
            let d = bin as f64 / n as f64;
            let h = Harmonics::new(d, sh, 0.9).unwrap();
            let h8 = Harmonics8::new(std::array::from_fn(|_| h.clone()));
            let mut residuals = [0.0; 2];
            for engine in 0..2 {
                let samples: Vec<f64> = (0..n)
                    .map(|i| {
                        let p = ((i * bin) % n) as f64 / n as f64;
                        if engine == 0 {
                            old(sh, p, d)
                        } else {
                            let a: [f32; 8] = (if libm {
                                h8.sample(f32x8::splat(p as f32))
                            } else {
                                h8.sample_fast(f32x8::splat(p as f32))
                            })
                            .into();
                            a[0] as f64
                        }
                    })
                    .collect();
                let mean = samples.iter().sum::<f64>() / n as f64;
                let mut projection = vec![mean; n];
                let mut wanted = 0.0;
                for k in 1..=h.count {
                    // Include mathematically wanted harmonics; don't mask even triangle bins.
                    if matches!(sh, Shape::Triangle) && k % 2 == 0 {
                        continue;
                    }
                    let (mut a, mut b) = (0.0, 0.0);
                    for i in 0..n {
                        let (sn, cs) = (std::f64::consts::TAU * ((i * bin) % n) as f64 / n as f64
                            * k as f64)
                            .sin_cos();
                        a += samples[i] * cs;
                        b += samples[i] * sn;
                    }
                    a *= 2.0 / n as f64;
                    b *= 2.0 / n as f64;
                    wanted += (a * a + b * b) * 0.5;
                    for i in 0..n {
                        let (sn, cs) = (std::f64::consts::TAU * ((i * bin) % n) as f64 / n as f64
                            * k as f64)
                            .sin_cos();
                        projection[i] += a * cs + b * sn;
                    }
                }
                residuals[engine] = 10.0
                    * ((samples
                        .iter()
                        .zip(projection)
                        .map(|(s, p)| (s - p).powi(2))
                        .sum::<f64>()
                        / n as f64)
                        / wanted)
                        .log10();
            }
            println!(
                "{sh:?},{},{:.3},{:.3}",
                d * 48000.0,
                residuals[0],
                residuals[1]
            );
        }
    }
}
fn main() {
    if std::env::args().any(|s| s == "--bench") {
        bench()
    } else if std::env::args().any(|s| s == "--spectrum") {
        spectrum()
    } else {
        quality()
    }
}
