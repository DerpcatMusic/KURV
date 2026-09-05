#![allow(dead_code, unused_imports)]
// truce-simd 6.3.0 reexports these exact wide 0.7 types.
extern crate self as truce_simd;
pub mod simd {
    pub use wide::{f32x4, f32x8};
}
include!(concat!(env!("OUT_DIR"), "/modules.rs"));
use oscillators::va::*;
use wide::{f32x4, f32x8};

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
    let mut scalar = initial;
    let expected: [[f32; 8]; N] = std::array::from_fn(|_| {
        std::array::from_fn(|lane| scalar[lane].generate_shape_step(shape, step, width, mode))
    });
    let mut maximum = 0.0;
    for lanes in [4, 8] {
        let mut state = initial;
        let mut left = [f32x8::ZERO; N];
        let mut right = [f32x8::ZERO; N];
        if lanes == 8 {
            accumulate_shape8_block_constant(
                &mut state,
                f32x8::splat(step),
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
                f32x4::splat(step),
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
        let actual = generate_shape_time8(&mut time, shape, step, [0.0; 8], width, mode);
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
            f32x8::splat(step),
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

mod partition_check;
mod pm_check;
fn main() {
    performance::initialize();
    pm_check::check();
    partition_check::check();
    let mut maximum = 0.0_f32;
    let mut cases = 0;
    for factor in [1, 2, 3, 4] {
        let mode = Antialiasing::Spline.for_factor(factor);
        for shape in [0.0, 1.0, 2.0, 3.0] {
            for step in [1.0 / 1024.0, 0.01, 0.125, 0.249, 0.251, 0.375, 0.449] {
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
