#![allow(dead_code)]
use std::hint::black_box;
static MAX_ERROR: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
use wide::{CmpLt, f32x4, f32x8};
#[derive(Clone, Copy)]
pub struct VaOscillator {
    phase: f32,
}
mod performance {
    #[derive(PartialEq)]
    pub enum SplineBackend {
        Portable,
        Avx2Fma,
    }
    pub fn spline_backend() -> SplineBackend {
        if std::is_x86_feature_detected!("avx2") && std::is_x86_feature_detected!("fma") {
            SplineBackend::Avx2Fma
        } else {
            SplineBackend::Portable
        }
    }
}
mod backend {
    use super::*;
    include!(concat!(env!("OUT_DIR"), "/backend.rs"));
}
macro_rules! renderer {
    ($name:ident,$aa:literal,$render:literal) => {
        mod $name {
            use super::*;
            mod antialias {
                include!(concat!(env!("OUT_DIR"), $aa));
            }
            use antialias::*;
            include!(concat!(env!("OUT_DIR"), $render));
            pub fn block<const N: usize>(
                osc: &mut [VaOscillator; 8],
                steps: [f32; 8],
                pm: &[f32; N],
                optimized: bool,
                width: usize,
                modulated: bool,
                out: &mut [(f32, f32); N],
            ) {
                let l = [0.11, 0.22, 0.06, 0.05, 0.07, 0.01, 0.17, 0.04];
                let r = [0.04, 0.07, 0.12, 0.13, 0.15, 0.11, 0.02, 0.03];
                match width {
                    4 => accumulate_spline_saw4_phase_modulated_block(
                        osc,
                        steps[..4].try_into().unwrap(),
                        pm,
                        optimized,
                        l[..4].try_into().unwrap(),
                        r[..4].try_into().unwrap(),
                        out,
                    ),
                    8 => accumulate_spline_saw8_phase_modulated_block(
                        osc, steps, pm, optimized, l, r, out,
                    ),
                    16 => {
                        let mut left = [f32x8::ZERO; N];
                        let mut right = [f32x8::ZERO; N];
                        accumulate_spline_saw8_phase_modulated_lanes_block(
                            osc,
                            steps,
                            modulated.then_some(pm),
                            optimized,
                            f32x8::from(l),
                            f32x8::from(r),
                            &mut left,
                            &mut right,
                        );
                        for i in 0..N {
                            out[i] = (left[i].reduce_add(), right[i].reduce_add());
                        }
                    }
                    _ => unreachable!(),
                }
            }
        }
    };
}
renderer!(baseline, "/baseline_aa.rs", "/baseline_render.rs");
renderer!(candidate, "/candidate_aa.rs", "/candidate_render.rs");
fn case<const N: usize>(
    seed: usize,
    step: f32,
    depth: f32,
    width: usize,
    optimized: bool,
    modulated: bool,
) {
    let mut a = std::array::from_fn(|i| VaOscillator {
        phase: ((i * 7919 + seed * 313) % 65536) as f32 / 65536.0,
    });
    let mut b = a;
    let steps = std::array::from_fn(|i| step * (1.0 + i as f32 * 0.0003));
    let pm = std::array::from_fn(|i| {
        depth * ((i as f32 * 0.281 + seed as f32).sin() + 0.3 * (i as f32 * 2.1).cos())
    });
    let mut x = [(0.17, -0.23); N];
    let mut y = x;
    baseline::block(&mut a, steps, &pm, optimized, width, modulated, &mut x);
    candidate::block(&mut b, steps, &pm, optimized, width, modulated, &mut y);
    for i in 0..N {
        MAX_ERROR.fetch_max(
            (x[i].0 - y[i].0)
                .abs()
                .max((x[i].1 - y[i].1).abs())
                .to_bits(),
            std::sync::atomic::Ordering::Relaxed,
        );
        assert!(
            (x[i].0 - y[i].0).abs() < 2e-6,
            "L N={N} step={step} depth={depth} width={width} sample={i}: {} vs {}",
            x[i].0,
            y[i].0
        );
        assert!((x[i].1 - y[i].1).abs() < 2e-6);
    }
    for i in 0..8 {
        assert_eq!(a[i].phase.to_bits(), b[i].phase.to_bits());
    }
    if width == 16 {
        let mut left_a = [f32x8::from([0.11, -0.22, 0.33, -0.04, 0.25, -0.16, 0.07, -0.18]); N];
        let mut left_b = left_a;
        let mut right_a = [f32x8::splat(0.31); N];
        let mut right_b = right_a;
        baseline::accumulate_spline_saw8_phase_modulated_lanes_block(
            &mut a,
            steps,
            modulated.then_some(&pm),
            optimized,
            f32x8::splat(0.19),
            f32x8::splat(-0.23),
            &mut left_a,
            &mut right_a,
        );
        candidate::accumulate_spline_saw8_phase_modulated_lanes_block(
            &mut b,
            steps,
            modulated.then_some(&pm),
            optimized,
            f32x8::splat(0.19),
            f32x8::splat(-0.23),
            &mut left_b,
            &mut right_b,
        );
        for (a, b) in left_a.iter().zip(left_b).chain(right_a.iter().zip(right_b)) {
            let a: [f32; 8] = (*a).into();
            let b: [f32; 8] = b.into();
            for lane in 0..8 {
                let error = (a[lane] - b[lane]).abs();
                MAX_ERROR.fetch_max(error.to_bits(), std::sync::atomic::Ordering::Relaxed);
                assert!(error < 2e-6);
            }
        }
        for i in 0..8 {
            assert_eq!(a[i].phase.to_bits(), b[i].phase.to_bits());
        }
    }
}
fn verify() {
    let mut cases = 0;
    for seed in 0..32 {
        for step in [0.0, 1e-8, 0.0001, 0.001, 0.01, 0.04, 0.12, 0.249] {
            for depth in [0.0, 0.1, 0.49, 1.0, 4.0] {
                for width in [4, 8, 16] {
                    for optimized in [false, true] {
                        for modulated in [false, true] {
                            case::<1>(seed, step, depth, width, optimized, modulated);
                            case::<16>(seed, step, depth, width, optimized, modulated);
                            case::<64>(seed, step, depth, width, optimized, modulated);
                            cases += 3;
                        }
                    }
                }
            }
        }
    }
    println!(
        "{cases} block cases within 2e-6, final phase bit-identical; max error {}",
        f32::from_bits(MAX_ERROR.load(std::sync::atomic::Ordering::Relaxed))
    );
}
fn main() {
    if std::env::args().nth(1).as_deref() != Some("--bench") {
        verify();
        return;
    }
    let variant = std::env::args().nth(2).unwrap();
    let groups: usize = std::env::args()
        .nth(3)
        .map(|s| s.parse().unwrap())
        .unwrap_or(1);
    assert!((1..=8).contains(&groups));
    let mut osc = [[VaOscillator { phase: 0.125 }; 8]; 8];
    println!("variant,width,step,depth,optimized,ns_per_block");
    for width in [4, 8, 16] {
        for step in [0.0001, 0.01, 0.12] {
            for depth in [0.0, 0.49, 4.0] {
                for optimized in [false, true] {
                    let steps = std::array::from_fn(|i| step * (1.0 + i as f32 * 0.0003));
                    let pm: [f32; 64] = std::array::from_fn(|i| depth * (i as f32 * 0.281).sin());
                    let mut out = [(0.0, 0.0); 64];
                    let f = if variant == "baseline" {
                        baseline::block::<64>
                    } else {
                        candidate::block::<64>
                    };
                    let t = std::time::Instant::now();
                    let iterations = 20000 / groups;
                    for _ in 0..iterations {
                        for osc in &mut osc[..groups] {
                            f(
                                black_box(osc),
                                black_box(steps),
                                black_box(&pm),
                                black_box(optimized),
                                black_box(width),
                                true,
                                black_box(&mut out),
                            );
                            black_box(&out);
                        }
                    }
                    println!(
                        "{variant},{width},{step},{depth},{optimized},{}",
                        t.elapsed().as_nanos() as f64 / iterations as f64
                    );
                }
            }
        }
    }
}
