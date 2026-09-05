#!/usr/bin/env python3
"""Analytic continuous PM, ideal lowpass, fixed timestamp compensation.

No delay fitting, no gain fitting before total-error measurement. The support
split cannot distinguish aliases that overlap intended spectral components.
"""
import csv
import sys
import numpy as np

N = 4096
rows = np.genfromtxt(sys.argv[1], delimiter=",", skip_header=1)
if not np.isfinite(rows).all() or len(rows) != 96 * N:
    raise ValueError("Expected 96 finite cases, each containing 4096 samples")


def db(error, reference):
    return 10 * np.log10(max(np.mean(error * error), 1e-30) / np.mean(reference * reference))


def reference_spectrum(carrier, modulator, depth, nested, factor):
    t = np.arange(N * factor) / (N * factor)
    parent = np.sin(2 * np.pi * 367 * t)
    if nested < 0:
        modulation = depth * (1 - nested * parent) * np.sin(2 * np.pi * modulator * t)
    else:
        modulation = depth * np.sin(2 * np.pi * (modulator * t + nested * parent))
    phase = (carrier * t + 0.123 + modulation) % 1
    spectrum = np.fft.rfft(2 * phase - 1)[:N // 2 + 1] / factor
    spectrum[-1] = 0  # strictly below Nyquist
    return spectrum


writer = csv.writer(sys.stdout)
writer.writerow(["carrier", "modulator", "depth", "nested", "convergence",
                 "baseline", "local", "central", "adaa", "adaa_gain_db",
                 "adaa_wanted_error_db", "adaa_outside_reference_support_db"])
for samples in rows.reshape(-1, N, 9):
    carrier, modulator, depth, nested = samples[0, :4]
    low = reference_spectrum(carrier, modulator, depth, nested, 256)
    high = reference_spectrum(carrier, modulator, depth, nested, 1024)
    reference = np.fft.irfft(high, n=N)
    # ADAA averages [n-1,n], so its known timestamp is n-0.5. Shift the
    # continuous bandlimited oracle analytically, not the candidate output.
    delayed_spectrum = high * np.exp(-1j * np.pi * np.arange(len(high)) / N)
    delayed_reference = np.fft.irfft(delayed_spectrum, n=N)
    gain = np.dot(samples[:, 8], delayed_reference) / np.dot(delayed_reference, delayed_reference)
    actual = np.fft.rfft(samples[:, 8])
    support = np.abs(delayed_spectrum) >= np.max(np.abs(delayed_spectrum)) * 1e-4
    wanted = np.fft.irfft(np.where(support, actual - delayed_spectrum, 0), n=N)
    outside = np.fft.irfft(np.where(~support, actual - delayed_spectrum, 0), n=N)
    writer.writerow([carrier, modulator, depth, nested,
                     db(np.fft.irfft(low, n=N) - reference, reference),
                     *[db(samples[:, i] - reference, reference) for i in [5, 6, 7]],
                     db(samples[:, 8] - delayed_reference, delayed_reference),
                     20 * np.log10(abs(gain)), db(wanted, delayed_reference),
                     db(outside, delayed_reference)])
