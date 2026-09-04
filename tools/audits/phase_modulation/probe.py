#!/usr/bin/env python3
"""Offline float64 model, NOT a KURV render or a CPU benchmark.

Requires numpy. Models the production SplineOptimized saw equation, drives
continuous periodic phase trajectories, then compares against independently
oversampled, ideal-FFT-lowpassed references. No gain/phase fitting or windowing.
"""
import json
import argparse
import numpy as np

N = 16384
FC = 83  # cycles per record; all signals are coherent with the FFT record
FM = 997
DEPTH = 0.22  # phase cycles, not radians


def residual(position):
    distance = np.abs(position)
    inner = (((0.09448366408759548 * distance - 0.2733964855652928)
              * distance - 0.0003685148413927544) * distance
             + 0.626745090389803) * distance - 0.5
    tail = 2 - distance
    outer = (((-0.0291066153422181 * tail - 0.026743309461271375)
              * tail + 0.005957221463576306) * tail
             - 0.002643542589373836) * tail
    result = np.where(distance < 1, inner, np.where(distance < 2, outer, 0))
    return np.where(position < 0, -result, result)


def saw(phase, step):
    phase = phase % 1
    assert np.all(np.asarray(step) > 0) and np.all(np.asarray(step) < 0.25)
    position = np.where(phase < 0.5, phase, phase - 1) / step
    return 2 * phase - 1 - 2 * residual(position)


def trajectory(t, case):
    modulation = DEPTH * np.sin(2 * np.pi * FM * t)
    if case == "nested_sine_pm":
        modulation = DEPTH * np.sin(2 * np.pi * FM * t
                                    + 1.4 * np.sin(2 * np.pi * 211 * t))
    if case == "four_stage_sine_pm":
        modulation = DEPTH * np.sin(2 * np.pi * FM * t + 1.4 * np.sin(
            2 * np.pi * 211 * t + 1.1 * np.sin(2 * np.pi * 53 * t)))
    if case == "static_saw":
        modulation = 0
    if case == "depth_clipping":
        # Models block_amount's saturation, not the entire route/tap system.
        amount = np.clip(0.8 + 0.6 * np.sin(2 * np.pi * 211 * t), -1, 1)
        modulation = amount * np.sin(2 * np.pi * FM * t)
    return 1 / 64 + FC * t + modulation


def reference(case, rate):
    t = np.arange(N * rate) / (N * rate)
    # Same continuous target saw; correction support shrinks with rate.
    high = saw(trajectory(t, case), FC / (N * rate))
    spectrum = np.fft.rfft(high) / rate
    # Keep strictly below output Nyquist to avoid endpoint conventions.
    low = np.zeros(N // 2 + 1, dtype=complex)
    low[:-1] = spectrum[:N // 2]
    return np.fft.irfft(low, n=N)


def db_ratio(error, signal):
    return float(10 * np.log10(max(np.sum(error * error), 1e-300)
                              / np.sum(signal * signal)))


def run():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--production-samples", help="Rust harness --dump-nested output")
    args = parser.parse_args()
    # Linear PM and direct tuning describe precisely the same phase trajectory.
    t = np.arange(256)
    phase = 1 / 64 + t / 16
    narrow = saw(phase, 1 / 128)
    equivalent = saw(phase, 1 / 16)
    max_error = float(np.max(np.abs(narrow - equivalent)))
    assert max_error > 0.1
    result = {"model": "float64 SplineOptimized; four_stage uses production dump when supplied",
              "linear_pm_equivalence_max_error": max_error, "cases": []}
    # An ideal sine carrier needs no BLEP, yet PM creates aliases. Its only
    # continuous-time components are +/- (1000 + k*9000) Hz. None is 20 kHz.
    # The +28 kHz sideband folds to -20 kHz at 48 kHz sample rate.
    time = np.arange(48000) / 48000
    sine_pm = np.cos(2 * np.pi * 1000 * time
                     + 2 * np.pi * 0.2 * np.sin(2 * np.pi * 9000 * time))
    folded_amplitude = abs(np.fft.rfft(sine_pm)[20000]) * 2 / len(time)
    assert folded_amplitude > 0.01
    result["ideal_sine_pm_20khz_alias_dbfs"] = float(20 * np.log10(folded_amplitude))
    for case in ["static_saw", "sine_pm", "nested_sine_pm", "four_stage_sine_pm", "depth_clipping"]:
        phase = trajectory(np.arange(N) / N, case)
        rendered = saw(phase, FC / N)
        production_difference = None
        if case == "four_stage_sine_pm" and args.production_samples:
            production = np.loadtxt(args.production_samples)
            assert production.shape == rendered.shape
            production_difference = float(np.max(np.abs(production - rendered)))
            assert production_difference < 2e-5, production_difference
            rendered = production
        ref64 = reference(case, 64)
        ref128 = reference(case, 128)
        # Remove Nyquist consistently from the candidate too.
        fft = np.fft.rfft(rendered)
        fft[-1] = 0
        rendered = np.fft.irfft(fft, n=N)
        error_db = db_ratio(rendered - ref128, ref128)
        convergence_db = db_ratio(ref64 - ref128, ref128)
        assert convergence_db < error_db - 20, (case, error_db, convergence_db)
        result["cases"].append({
            "case": case,
            "reconstruction_error_db": error_db,
            "reference_64x_vs_128x_db": convergence_db,
            "backward_phase_intervals": int(np.sum(np.diff(phase) < 0)),
            "production_vs_model_max_error": production_difference,
        })
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    run()
