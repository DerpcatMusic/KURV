#!/usr/bin/env python3

import argparse
import math

import numpy as np


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("raw", help="little-endian mono f32 samples")
    parser.add_argument("waveform", choices=("triangle", "saw", "pulse"))
    parser.add_argument("fft_bin", type=int)
    parser.add_argument("samples", type=int)
    args = parser.parse_args()

    signal = np.fromfile(args.raw, dtype="<f4")
    if signal.size != args.samples:
        raise SystemExit(f"expected {args.samples} samples, got {signal.size}")
    if not np.all(np.isfinite(signal)):
        raise SystemExit("render contains NaN or Inf")

    spectrum = np.fft.rfft(signal.astype(np.float64))
    power = np.abs(spectrum) ** 2
    expected = np.zeros(power.size, dtype=bool)
    expected[0] = True
    harmonic = 1
    harmonic_bins: list[int] = []
    while harmonic * args.fft_bin < power.size:
        if args.waveform == "saw" or harmonic % 2 == 1:
            expected[harmonic * args.fft_bin] = True
            harmonic_bins.append(harmonic * args.fft_bin)
        harmonic += 1

    expected_power = float(power[expected].sum() - power[0])
    residual_power = float(power[~expected].sum())
    alias_dbc = ratio_db(residual_power, expected_power)
    dc_dbfs = amplitude_db(abs(float(signal.mean())))
    residual_rms = math.sqrt(residual_power) * math.sqrt(2.0) / signal.size
    residual_dbfs = amplitude_db(residual_rms)

    magnitudes = np.abs(spectrum[harmonic_bins])
    measured_ratio = magnitudes / max(float(magnitudes[0]), np.finfo(float).tiny)
    harmonic_numbers = np.asarray(harmonic_bins, dtype=np.float64) / args.fft_bin
    if args.waveform == "triangle":
        ideal_ratio = 1.0 / harmonic_numbers**2
    else:
        ideal_ratio = 1.0 / harmonic_numbers
    visible = ideal_ratio >= 10.0 ** (-100.0 / 20.0)
    magnitude_error = 20.0 * np.log10(
        np.maximum(measured_ratio[visible], np.finfo(float).tiny) / ideal_ratio[visible]
    )
    magnitude_error_rms_db = math.sqrt(float(np.mean(magnitude_error**2)))
    magnitude_error_peak_db = float(np.max(np.abs(magnitude_error)))

    print(
        ",".join(
            (
                f"waveform={args.waveform}",
                f"fft_bin={args.fft_bin}",
                f"samples={args.samples}",
                f"peak={float(np.max(np.abs(signal))):.9g}",
                f"rms={math.sqrt(float(np.mean(signal.astype(np.float64) ** 2))):.9g}",
                f"dc_dbfs={dc_dbfs:.3f}",
                f"alias_residual_dbc={alias_dbc:.3f}",
                f"residual_dbfs={residual_dbfs:.3f}",
                f"harmonic_magnitude_error_rms_db={magnitude_error_rms_db:.3f}",
                f"harmonic_magnitude_error_peak_db={magnitude_error_peak_db:.3f}",
            )
        )
    )


def ratio_db(numerator: float, denominator: float) -> float:
    if numerator <= 0.0:
        return -math.inf
    if denominator <= 0.0:
        return math.inf
    return 10.0 * math.log10(numerator / denominator)


def amplitude_db(amplitude: float) -> float:
    if amplitude <= 0.0:
        return -math.inf
    return 20.0 * math.log10(amplitude)


if __name__ == "__main__":
    main()
