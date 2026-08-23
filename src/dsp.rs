//! Shared numeric helpers used by editor and offline analysis.

use std::cell::RefCell;

use rustfft::FftPlanner;

pub(crate) type Complex = rustfft::num_complex::Complex<f64>;

thread_local! {
    static PLANNER: RefCell<FftPlanner<f64>> = RefCell::new(FftPlanner::new());
    static SCRATCH: RefCell<Vec<Complex>> = const { RefCell::new(Vec::new()) };
}

/// Worker/editor FFT. Inverse applies the `1/n` scale the compilers expect.
pub(crate) fn fft(values: &mut [Complex], inverse: bool) {
    let plan = PLANNER.with(|planner| {
        let mut planner = planner.borrow_mut();
        if inverse {
            planner.plan_fft_inverse(values.len())
        } else {
            planner.plan_fft_forward(values.len())
        }
    });
    SCRATCH.with(|scratch| {
        let mut scratch = scratch.borrow_mut();
        scratch.resize(plan.get_inplace_scratch_len(), Complex::ZERO);
        plan.process_with_scratch(values, &mut scratch);
    });
    if inverse {
        let scale = 1.0 / values.len() as f64;
        for value in values {
            *value *= scale;
        }
    }
}

pub(crate) fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

pub(crate) fn midi_note_hz(note: f32) -> f32 {
    440.0 * ((note - 69.0) / 12.0).exp2()
}

pub(crate) fn shortest_angle(from: f64, to: f64) -> f64 {
    (to - from + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
}

#[inline(always)]
pub(crate) fn curve_progress(progress: f32, curve: f32) -> f32 {
    let progress = progress.clamp(0.0, 1.0);
    let bias = curve.clamp(-1.0, 1.0).mul_add(0.5, 0.5).clamp(0.005, 0.995);
    progress / ((bias.recip() - 2.0).mul_add(1.0 - progress, 1.0))
}
