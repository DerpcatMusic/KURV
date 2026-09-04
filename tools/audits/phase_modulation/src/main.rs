#![allow(dead_code)]
use wide::{CmpLt, f32x4, f32x8};
mod antialias {
    include!(concat!(env!("OUT_DIR"), "/antialias.rs"));
}
use antialias::{spline_saw4_narrow, spline_saw8_narrow, wrap_phase4, wrap_phase8};

// Only state used by the extracted functions. No render arithmetic is rewritten.
#[derive(Clone, Copy)]
pub struct VaOscillator {
    phase: f32,
}
include!(concat!(env!("OUT_DIR"), "/render.rs"));

fn render<const N: usize>(step: f32, pm: &[f32; N], width: usize) -> [f32; N] {
    let mut oscillators = [VaOscillator { phase: 1.0 / 64.0 }; 8];
    let mut out = [(0.0, 0.0); N];
    match width {
        4 => accumulate_spline_saw4_phase_modulated_block(
            &mut oscillators,
            [step; 4],
            pm,
            true,
            [1.0, 0.0, 0.0, 0.0],
            [0.0; 4],
            &mut out,
        ),
        8 => accumulate_spline_saw8_phase_modulated_block(
            &mut oscillators,
            [step; 8],
            pm,
            true,
            [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            [0.0; 8],
            &mut out,
        ),
        16 => {
            let mut left = [f32x8::ZERO; N];
            let mut right = [f32x8::ZERO; N];
            accumulate_spline_saw8_phase_modulated_lanes_block(
                &mut oscillators,
                [step; 8],
                Some(pm),
                true,
                f32x8::from([1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
                f32x8::ZERO,
                &mut left,
                &mut right,
            );
            for n in 0..N {
                out[n].0 = left[n].reduce_add();
            }
        }
        _ => unreachable!(),
    }
    out.map(|sample| sample.0)
}

fn main() {
    if std::env::args().any(|arg| arg == "--dump-nested") {
        const N: usize = 16384;
        let pm = std::array::from_fn(|n| {
            let t = n as f64 / N as f64;
            let tau = std::f64::consts::TAU;
            (0.22
                * (tau * 997.0 * t + 1.4 * (tau * 211.0 * t + 1.1 * (tau * 53.0 * t).sin()).sin())
                    .sin()) as f32
        });
        let samples: [f32; N] = render(83.0 / N as f32, &pm, 8);
        for sample in samples {
            println!("{sample:.9}");
        }
        return;
    }
    const N: usize = 256;
    let pm: [f32; N] = std::array::from_fn(|n| ((n as f32 * 7.0 / 128.0 + 0.5) % 1.0) - 0.5);
    for width in [4, 8, 16] {
        let modulated = render(1.0 / 128.0, &pm, width);
        let direct = render(1.0 / 16.0, &[0.0; N], width);
        let maximum = modulated
            .iter()
            .zip(direct)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0_f32, f32::max);
        let kernel = match width {
            4 => "x4",
            8 => "x8",
            _ => "x8-lane-accumulator",
        };
        println!("kernel={kernel} PM/direct-tuning max_abs_error={maximum:.9}");
        // Characterization, not an acceptance test locking in the limitation.
        // --require-equivalence opts into the proposed acceptance criterion.
        if std::env::args().any(|arg| arg == "--require-equivalence") {
            assert!(maximum < 1e-5, "equivalent trajectory acceptance criterion");
        }
        let zero = render(1.0 / 128.0, &[0.0; N], width);
        let wrapped = render(1.0 / 128.0, &[1.0; N], width);
        assert!(zero.iter().zip(wrapped).all(|(a, b)| (a - b).abs() < 1e-6));
    }
}
