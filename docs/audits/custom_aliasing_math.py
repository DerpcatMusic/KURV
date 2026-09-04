#!/usr/bin/env python3
"""Analytical witnesses, not an end-to-end plugin benchmark.

No external packages. Does not simulate FIR decimation or quantify aggregate
aliasing: these are narrow witnesses for already-in-band folding, new harmonics
from phase warp, hard clipping, and integer coefficient-mask discontinuities.
"""
import json
import math

N = 4096
TAU = math.tau
phase = [index / N for index in range(N)]


def amplitude(samples, harmonic):
    real = sum(value * math.cos(TAU * harmonic * p) for value, p in zip(samples, phase))
    imag = sum(value * math.sin(TAU * harmonic * p) for value, p in zip(samples, phase))
    return 2 * math.hypot(real, imag) / N


def db(value):
    return 20 * math.log10(value)


# 1500 Hz base at 96000 Hz internal rate. Harmonic 65 is 97500 Hz;
# direct evaluation folds it to 1500 Hz BEFORE the output decimator.
custom = [math.sin(TAU * 65 * ((index % 64) / 64)) for index in range(N)]
folded = [math.sin(TAU * ((index % 64) / 64)) for index in range(N)]
fold_error = max(abs(a - b) for a, b in zip(custom, folded))
assert fold_error < 1e-12

# ratio.rs keeps canonical fundamental (aligned negative cosine), warp.rs
# Harmonic mode at amount=1, sufficiently low phase step, has depth=0.95.
warped = [-math.cos(TAU * p - 0.95 * math.sin(TAU * p)) for p in phase]
second_db = db(amplitude(warped, 2) / amplitude(warped, 1))
assert second_db > -10

# Expression output is clamped. 2*sin(tau*x) before clamp contains only h1.
clipped = [max(-1, min(1, 2 * math.sin(TAU * p))) for p in phase]
third_db = db(amplitude(clipped, 3) / amplitude(clipped, 1))
assert third_db > -20

# At a saw cutoff just below/at h2, floor() includes a whole extra partial.
# The Fourier saw h2 amplitude is 2/(pi*2) = 1/pi.
cutoff_step = 1 / math.pi
assert cutoff_step > 0.3
print(json.dumps({
    'model': 'float64 mathematical witnesses; not captured KURV output',
    'custom_direct_fold': {'internal_rate_hz': 96000, 'base_hz': 1500,
        'source_harmonic': 65, 'source_hz': 97500, 'alias_hz': 1500,
        'max_sample_error_against_unit_amplitude_alias': fold_error},
    'fundamental_only_then_harmonic_warp': {'h2_relative_to_h1_db': second_db},
    'clipped_two_times_sine': {'h3_relative_to_h1_db': third_db},
    'saw_ratio_cutoff_crossing_2': {'added_partial_peak': cutoff_step,
        'added_partial_peak_dbfs': db(cutoff_step)},
}, indent=2))
