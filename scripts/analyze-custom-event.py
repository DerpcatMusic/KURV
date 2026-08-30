#!/usr/bin/env python3

import argparse
import math

import numpy as np


parser = argparse.ArgumentParser()
parser.add_argument("raw")
parser.add_argument("fft_bin", type=int)
parser.add_argument("samples", type=int)
parser.add_argument("--delay", type=int, default=0)
args = parser.parse_args()

signal = np.fromfile(args.raw, dtype="<f4").astype(np.float64)
if signal.size != args.samples or not np.all(np.isfinite(signal)):
    raise SystemExit("invalid render")
spectrum = np.fft.rfft(signal)
power = np.abs(spectrum) ** 2
expected = np.zeros(power.size, dtype=bool)
expected[0] = True
bins = []
harmonic = 1
while harmonic * args.fft_bin < power.size:
    if harmonic % 2:
        expected[harmonic * args.fft_bin] = True
        bins.append(harmonic * args.fft_bin)
    harmonic += 1
expected_power = float(power[expected].sum() - power[0])
residual_power = float(power[~expected].sum())
measured = np.abs(spectrum[bins])
numbers = np.asarray(bins) / args.fft_bin
ideal = 1.0 / numbers**2
ratio = measured / max(float(measured[0]), np.finfo(float).tiny)
visible = ideal >= 1e-5
visible &= np.asarray(bins) <= args.samples * 20_000 / 48_000
error_db = 20.0 * np.log10(np.maximum(ratio[visible], np.finfo(float).tiny) / ideal[visible])
harmonic_numbers = np.asarray(bins, dtype=np.int64) // args.fft_bin
amplitudes = (
    8.0
    / math.pi**2
    * np.where(((harmonic_numbers - 1) // 2) % 2 == 0, 1.0, -1.0)
    / harmonic_numbers**2
)
expected_complex = -0.5j * args.samples * amplitudes
period_samples = args.samples / args.fft_bin
measured_delay = (
    -np.angle(spectrum[bins[0]] / expected_complex[0])
    * args.samples
    / (2.0 * math.pi * args.fft_bin)
)
fitted_delay = measured_delay + round((args.delay - measured_delay) / period_samples) * period_samples
delay_compensation = np.exp(
    2j * math.pi * np.arange(spectrum.size) * fitted_delay / args.samples
)
aligned_spectrum = spectrum * delay_compensation
complex_error_dbc = 10.0 * math.log10(
    float(np.sum(np.abs(aligned_spectrum[bins] - expected_complex) ** 2))
    / float(np.sum(np.abs(expected_complex) ** 2))
)
ideal_spectrum = np.zeros_like(spectrum)
ideal_spectrum[bins] = expected_complex
reference = np.fft.irfft(ideal_spectrum, args.samples)
aligned_signal = np.fft.irfft(aligned_spectrum, args.samples)
ideal_error_dbc = 20.0 * math.log10(
    math.sqrt(float(np.mean((aligned_signal - reference) ** 2)))
    / math.sqrt(float(np.mean(reference**2)))
)
steps = np.abs(np.diff(signal))
print(
    f"fft_bin={args.fft_bin},samples={args.samples},"
    f"alias_residual_dbc={10 * math.log10(residual_power / expected_power):.3f},"
    f"harmonic_error_rms_db={math.sqrt(float(np.mean(error_db**2))):.3f},"
    f"harmonic_error_peak_db={float(np.max(np.abs(error_db))):.3f},"
    f"complex_wanted_error_dbc={complex_error_dbc:.3f},"
    f"ideal_error_dbc={ideal_error_dbc:.3f},"
    f"fitted_delay={fitted_delay:.6f},"
    f"peak={float(np.max(np.abs(signal))):.9f},dc={float(np.mean(signal)):.9g},"
    f"max_sample_step={float(np.max(steps)):.9f}"
)
