use std::f64::consts::TAU;

use super::shared::{
    ArtifactBuildError, remove_dc_and_peak_normalize, remove_dc_and_stereo_peak_normalize,
};
use crate::dsp::{Complex, fft, shortest_angle};

const FFT_SIZE: usize = 2_048;
const HOP_SIZE: usize = FFT_SIZE / 8;

pub(super) fn tune_stereo_with_cancel(
    mid: &[f32],
    side: &[f32],
    sample_rate: f32,
    root_hz: f32,
    pitch_at: impl Fn(f32) -> f32 + Copy,
    should_cancel: &dyn Fn() -> bool,
) -> Result<(Vec<f32>, Vec<f32>), ArtifactBuildError> {
    let mut tuned_mid =
        tune_channel_with_cancel(mid, sample_rate, root_hz, pitch_at, should_cancel)?;
    let mut tuned_side = if side.is_empty() {
        Vec::new()
    } else {
        tune_channel_with_cancel(side, sample_rate, root_hz, pitch_at, should_cancel)?
    };
    if tuned_side.is_empty() {
        remove_dc_and_peak_normalize(&mut tuned_mid);
    } else {
        remove_dc_and_stereo_peak_normalize(&mut tuned_mid, &mut tuned_side);
    }
    Ok((tuned_mid, tuned_side))
}

fn tune_channel_with_cancel(
    source: &[f32],
    sample_rate: f32,
    root_hz: f32,
    pitch_at: impl Fn(f32) -> f32,
    should_cancel: &dyn Fn() -> bool,
) -> Result<Vec<f32>, ArtifactBuildError> {
    if source.is_empty() {
        return Ok(Vec::new());
    }
    let mut output = vec![0.0_f64; source.len()];
    let mut weight = vec![0.0_f64; source.len()];
    let mut spectrum = vec![Complex::ZERO; FFT_SIZE];
    let mut shifted = vec![Complex::ZERO; FFT_SIZE];
    let mut previous_phase = vec![0.0_f64; FFT_SIZE / 2 + 1];
    let mut synthesis_phase = vec![0.0_f64; FFT_SIZE / 2 + 1];
    let mut initialized = false;
    let frames = source.len().div_ceil(HOP_SIZE) + 1;
    let bin_hz = f64::from(sample_rate) / FFT_SIZE as f64;
    for frame in 0..frames {
        if should_cancel() {
            return Err(ArtifactBuildError::Cancelled);
        }
        let center = frame * HOP_SIZE;
        let start = center as isize - FFT_SIZE as isize / 2;
        for (index, bin) in spectrum.iter_mut().enumerate() {
            let source_index = start + index as isize;
            let hann = 0.5 - 0.5 * (TAU * index as f64 / FFT_SIZE as f64).cos();
            bin.re = if (0..source.len() as isize).contains(&source_index) {
                f64::from(source[source_index as usize]) * hann.sqrt()
            } else {
                0.0
            };
            bin.im = 0.0;
        }
        fft(&mut spectrum, false);
        shifted.fill(Complex::ZERO);
        let position =
            center.min(source.len() - 1) as f32 / source.len().saturating_sub(1).max(1) as f32;
        let local_pitch = pitch_at(position).max(20.0);
        let ratio = (root_hz / local_pitch).clamp(0.01, 100.0) as f64;
        let harmonic_width = (bin_hz * 1.75 / f64::from(local_pitch)).clamp(0.06, 0.42);
        for bin in 1..FFT_SIZE / 2 {
            let value = spectrum[bin];
            let magnitude = value.norm();
            let phase = value.arg();
            let expected = TAU * bin as f64 * HOP_SIZE as f64 / FFT_SIZE as f64;
            let delta = shortest_angle(previous_phase[bin] + expected, phase);
            previous_phase[bin] = phase;
            let true_bin = bin as f64 + delta * FFT_SIZE as f64 / (TAU * HOP_SIZE as f64);
            let frequency = true_bin.max(0.0) * bin_hz;
            let harmonic = frequency / f64::from(local_pitch);
            let distance = (harmonic - harmonic.round()).abs();
            let mut tonal = if harmonic >= 0.75 {
                (1.0 - distance / harmonic_width).clamp(0.0, 1.0)
            } else {
                0.0
            };
            if frequency > 8_000.0 {
                let high_span = (f64::from(sample_rate) * 0.5 - 8_000.0).max(1.0);
                tonal *= 1.0 - 0.75 * ((frequency - 8_000.0) / high_span).clamp(0.0, 1.0);
            }
            shifted[bin] += value * (1.0 - tonal);
            if tonal <= f64::EPSILON {
                continue;
            }
            let target = true_bin * ratio;
            if !(1.0..FFT_SIZE as f64 / 2.0).contains(&target) {
                continue;
            }
            if initialized {
                synthesis_phase[bin] += TAU * target * HOP_SIZE as f64 / FFT_SIZE as f64;
            } else {
                synthesis_phase[bin] = phase;
            }
            let first = target.floor() as usize;
            let mix = target - first as f64;
            let partial = Complex::from_polar(magnitude * tonal, synthesis_phase[bin]);
            shifted[first] += partial * (1.0 - mix);
            shifted[first + 1] += partial * mix;
        }
        initialized = true;
        shifted[0] = spectrum[0];
        shifted[FFT_SIZE / 2] = spectrum[FFT_SIZE / 2];
        for bin in 1..FFT_SIZE / 2 {
            shifted[FFT_SIZE - bin] = shifted[bin].conj();
        }
        fft(&mut shifted, true);
        for (index, bin) in shifted.iter().enumerate() {
            let output_index = start + index as isize;
            if !(0..output.len() as isize).contains(&output_index) {
                continue;
            }
            let hann = 0.5 - 0.5 * (TAU * index as f64 / FFT_SIZE as f64).cos();
            output[output_index as usize] += bin.re * hann.sqrt();
            weight[output_index as usize] += hann;
        }
    }
    Ok(output
        .into_iter()
        .zip(weight)
        .map(|(sample, weight)| (sample / weight.max(1.0e-9)) as f32)
        .collect())
}
