#![allow(dead_code, unused_imports)]
// truce-simd 6.3.0 reexports these exact wide 0.7 types.
extern crate self as truce_simd;
pub mod simd {
    pub use wide::{f32x4, f32x8};
}
include!(concat!(env!("OUT_DIR"), "/modules.rs"));
use oscillators::va::*;
use wide::{f32x4, f32x8};
mod pm_routing;
mod high_note_contract;

const N: usize = 64;
fn states() -> [VaOscillator; 8] {
    std::array::from_fn(|lane| {
        let mut state = VaOscillator::default();
        state.set_phase(0.03125 + lane as f64 * 0.109375);
        state
    })
}
fn assert_close(label: &str, actual: f32, reference: f32, maximum: &mut f32) {
    let error = (actual - reference).abs();
    *maximum = maximum.max(error);
    assert!(
        actual.is_finite() && error < 0.000_03,
        "{label}: actual={actual}, reference={reference}, abs_error={error}"
    );
}

fn check_canonical(mode: Antialiasing, shape: f32, step: f32, width: f32) -> f32 {
    let initial = states();
    let steps: [f32; 8] =
        std::array::from_fn(|lane| (step * (1.0 + (lane as f32 - 3.0) * 0.017)).min(0.449));
    let mut scalar = initial;
    let expected: [[f32; 8]; N] = std::array::from_fn(|_| {
        std::array::from_fn(|lane| {
            scalar[lane].generate_shape_step(shape, steps[lane], width, mode)
        })
    });
    let mut maximum = 0.0;
    for lanes in [4, 8] {
        let mut state = initial;
        let mut left = [f32x8::ZERO; N];
        let mut right = [f32x8::ZERO; N];
        if lanes == 8 {
            accumulate_shape8_block_constant(
                &mut state,
                f32x8::from(steps),
                f32x8::ONE,
                f32x8::ZERO,
                &mut left,
                &mut right,
                shape,
                width,
                mode,
            );
        } else {
            accumulate_shape4_block_constant(
                &mut state,
                f32x4::from([steps[0], steps[1], steps[2], steps[3]]),
                f32x4::ONE,
                f32x4::ZERO,
                &mut left,
                &mut right,
                shape,
                width,
                mode,
            );
        }
        for frame in 0..N {
            let actual: [f32; 8] = left[frame].into();
            for lane in 0..lanes {
                assert_close("block", actual[lane], expected[frame][lane], &mut maximum);
            }
        }
        for lane in 0..lanes {
            assert_eq!(state[lane].phase(), scalar[lane].phase());
        }
    }
    let mut time = initial[0];
    for frame in (0..N).step_by(8) {
        let actual = generate_shape_time8(&mut time, shape, steps[0], [0.0; 8], width, mode);
        for offset in 0..8 {
            assert_close(
                "time SIMD",
                actual[offset],
                expected[frame + offset][0],
                &mut maximum,
            );
        }
    }
    if shape == 2.0 {
        let mut state = initial;
        let mut left = [f32x8::ZERO; N];
        let mut right = [f32x8::ZERO; N];
        accumulate_saw8_block_constant(
            &mut state,
            f32x8::from(steps),
            f32x8::ONE,
            f32x8::ZERO,
            &mut left,
            &mut right,
            mode,
        );
        for frame in 0..N {
            let actual: [f32; 8] = left[frame].into();
            for lane in 0..8 {
                assert_close(
                    "selected backend",
                    actual[lane],
                    expected[frame][lane],
                    &mut maximum,
                );
            }
        }
    }
    maximum
}

fn main() {
    performance::initialize();
    if std::env::args().any(|arg| arg == "--bench") {
        benchmark();
        return;
    }
    pm_routing::check();
    high_note_contract::check();
    check_custom_and_warp();
    let mut maximum = 0.0_f32;
    let mut cases = 0;
    for factor in [1, 2, 3, 4] {
        let mode = Antialiasing::Spline.for_factor(factor);
        for shape in [0.0, 0.4, 1.0, 1.3, 2.0, 2.7, 3.0] {
            for step in [
                1.0 / 1024.0,
                0.01,
                0.125,
                0.199,
                0.201,
                0.225,
                0.249,
                0.251,
                0.375,
                0.449,
            ] {
                for width in [0.03, 0.37, 0.5, 0.97] {
                    maximum = maximum.max(check_canonical(mode, shape, step, width));
                    cases += 1;
                }
            }
        }
    }
    println!(
        "PASS: {cases} factor/shape/frequency/width cases; scalar, x4, x8, time SIMD and selected saw backend; maximum difference {maximum:.9}"
    );
    println!("Backend: {:?}", performance::spline_backend());
}

fn check_custom_and_warp() {
    let mode = Antialiasing::Spline.for_factor(1);
    let curve = wave_curve::WaveCurveData::default().compile_rt();
    let mut maximum = 0.0_f32;
    let mut cases = 0;
    for shape in [1.0, 1.3, 2.0, 2.7, 3.0] {
        for step in [0.01, 0.199, 0.201, 0.249, 0.251, 0.375] {
            for warp in [
                PhaseWarpMode::None,
                PhaseWarpMode::Pwm,
                PhaseWarpMode::PhaseBend,
                PhaseWarpMode::Harmonic,
            ] {
                for amount in [0.0, 0.6] {
                    for mix in [0.0, 0.37, 1.0] {
                        let mut scalar = states();
                        let mut packed = states();
                        let mut left = [f32x8::ZERO; N];
                        let mut right = [f32x8::ZERO; N];
                        accumulate_custom8_block_constant(
                            &mut packed,
                            f32x8::splat(step),
                            f32x8::ONE,
                            f32x8::ZERO,
                            &mut left,
                            &mut right,
                            curve,
                            mix,
                            shape,
                            0.37,
                            mode,
                            warp,
                            amount,
                        );
                        for frame in 0..N {
                            let actual: [f32; 8] = left[frame].into();
                            for lane in 0..8 {
                                let expected = scalar[lane].generate_custom_step(
                                    shape, step, 0.37, mode, warp, amount, curve, mix,
                                );
                                assert_close("custom/warp", actual[lane], expected, &mut maximum);
                            }
                        }
                        // Active warp must keep the complete shipping output,
                        // including fallback triangle correction coefficients.
                        if warp != PhaseWarpMode::None && amount > 0.0 {
                            let mut a = states();
                            let mut b = states();
                            for _ in 0..N {
                                let new: [f32; 8] = generate_shape8_warped(
                                    &mut a, shape, [step; 8], 0.37, mode, warp, amount,
                                )
                                .into();
                                let old: [f32; 8] = generate_shape8_warped(
                                    &mut b,
                                    shape,
                                    [step; 8],
                                    0.37,
                                    Antialiasing::SplineOptimized,
                                    warp,
                                    amount,
                                )
                                .into();
                                assert_eq!(new, old, "active warp must retain shipping mode");
                            }
                        }
                        cases += 1;
                    }
                }
            }
        }
    }
    println!("PASS: {cases} custom/warp cases, scalar vs x8, maximum difference {maximum:.9}");
}

fn bench(mode: Antialiasing, shape: f32, step: f32, lanes: usize) -> f64 {
    use std::{hint::black_box, time::Instant};
    let mode = black_box(mode);
    let mut oscillators = [VaOscillator::default(); 64];
    for (lane, state) in oscillators.iter_mut().enumerate() {
        state.set_phase((lane as f64 * 0.1193 + 0.039).fract());
    }
    let steps: [f32; 64] =
        std::array::from_fn(|lane| (step * (1.0 + (lane as f32 % 8.0 - 3.0) * 0.017)).min(0.449));
    let mut left = [f32x8::ZERO; N];
    let mut right = [f32x8::ZERO; N];
    let rounds = 4096 / lanes.max(1);
    let start = Instant::now();
    for _ in 0..rounds {
        if lanes == 1 {
            for _ in 0..N / 8 {
                black_box(generate_shape_time8(
                    &mut oscillators[0],
                    shape,
                    steps[0],
                    [0.0; 8],
                    0.37,
                    mode,
                ));
            }
        } else if lanes == 4 {
            accumulate_shape4_block_constant(
                &mut oscillators[..4],
                f32x4::from(<[f32; 4]>::try_from(&steps[..4]).unwrap()),
                f32x4::splat(0.7),
                f32x4::splat(0.3),
                &mut left,
                &mut right,
                shape,
                0.37,
                mode,
            );
        } else {
            for lane in (0..lanes).step_by(8) {
                let step = f32x8::from(<[f32; 8]>::try_from(&steps[lane..lane + 8]).unwrap());
                if shape == 2.0 {
                    accumulate_saw8_block_constant(
                        &mut oscillators[lane..lane + 8],
                        step,
                        f32x8::splat(0.7),
                        f32x8::splat(0.3),
                        &mut left,
                        &mut right,
                        mode,
                    );
                } else {
                    accumulate_shape8_block_constant(
                        &mut oscillators[lane..lane + 8],
                        step,
                        f32x8::splat(0.7),
                        f32x8::splat(0.3),
                        &mut left,
                        &mut right,
                        shape,
                        0.37,
                        mode,
                    );
                }
            }
        }
        black_box(&left);
        black_box(&right);
    }
    black_box(oscillators);
    start.elapsed().as_nanos() as f64 / (rounds * N * lanes) as f64
}

fn benchmark() {
    println!("shape,step,lanes,shipping_ns,one_x_ns,ratio");
    for shape in [1.0, 2.0, 3.0] {
        for step in [0.01, 0.19, 0.225, 0.3, 0.44] {
            for lanes in [1, 4, 8, 16, 64] {
                let mut old = [0.0_f64; 7];
                let mut new = [0.0_f64; 7];
                for n in 0..7 {
                    if n % 2 == 0 {
                        old[n] = bench(Antialiasing::SplineOptimized, shape, step, lanes);
                        new[n] = bench(Antialiasing::Spline.for_factor(1), shape, step, lanes);
                    } else {
                        new[n] = bench(Antialiasing::Spline.for_factor(1), shape, step, lanes);
                        old[n] = bench(Antialiasing::SplineOptimized, shape, step, lanes);
                    }
                }
                old.sort_by(f64::total_cmp);
                new.sort_by(f64::total_cmp);
                println!(
                    "{shape},{step},{lanes},{:.4},{:.4},{:.4}",
                    old[3],
                    new[3],
                    new[3] / old[3]
                );
            }
        }
    }
}
