# KURV generator algorithm audit, round 2

Research commit: `839358a` (`experiment/fractional-combined`), 2026-08-06.

## Verdict

| Mode | Product decision | Reason |
|---|---|---|
| Spline 4PT, Normal 2x | **Default and keep** | Best portable balance: worst measured unwarped unwanted energy `-82.21 dBc`, median `-102.15 dBc`. |
| Spline 4PT, Eco 1x | Keep as an explicit economy mode | Cheap, but the `-34.50 dBc` worst case is not a pristine default. |
| Spline 4PT, Ultra 4x | Keep as optional offline/high-headroom quality | Worst case improves to `-100.03 dBc`, but costs about 80% more instructions than Normal 2x. |
| Legacy 2PT | Hide; retain only for compatibility/Eco comparison | Cheapest, but aliases at `-22.75 dBc` at 1x and `-42.95 dBc` at 2x. |
| Lagrange 4PT | Sunset from the UI; preserve parameter ID 38/value 2 | Strictly dominated by Spline in both quality and CPU at every useful factor. |
| Spectral 1x | Lab-only; preserve parameter ID 48 and old sessions | Excellent for static high notes, but architecture-sensitive, wasteful at low notes, and severely aliased under phase warp. |

Do not remove or renumber host parameters 33, 38, or 48. Old sessions must continue to resolve. The UI should expose Spline plus the existing factor selector; Legacy, Lagrange, and Spectral can remain hidden compatibility states.

## Method

The existing `generator_lab` was used without adding or changing tests. Two release binaries were built from the same source:

```text
baseline: RUSTFLAGS="-C target-cpu=x86-64"
modern:   RUSTFLAGS="-C target-cpu=x86-64-v3"
host:     Ryzen 7 7800X3D, Rust 1.97.1, Linux x86-64
```

The render matrix covered all five algorithms, factors 1x/2x/4x, sine/triangle/saw/pulse/static triangle-saw morph, and coherent FFT bins 89/601/4806/7000 at 48 kHz (`65.19`, `440.19`, `3520.02`, `5126.95 Hz`). Each render contained 65,536 samples after 16,384 warm-up frames. A second 360-render matrix covered PWM, Phase Bend, and Harmonic warp at 98% for triangle/saw/pulse/morph at bins 601 and 7000.

Unwanted energy is the FFT energy outside DC and exact integer harmonics, divided by wanted-harmonic energy. For sine, every bin except the fundamental is unwanted. Harmonic truth compares measured harmonic magnitudes through 20.5 kHz with the analytic Fourier coefficients after normalizing the fundamental.

CPU used the same dense 24-note, 64-unison, three-oscillator workload. Wall-clock medians were recorded, but CPU boost drift made cross-row timing misleading. The decision tables therefore use `perf` retired instructions over three repetitions, including the identical 16,384-frame warm-up. No pool was used in the per-algorithm matrix. Dynamic morph was measured separately in serial and pooled modes.

## Unwarped quality and CPU

The CPU columns are median retired instructions per host frame across the five shapes. They include identical setup/warm-up overhead and are intended for relative comparison.

| Algorithm | Factor | Worst unwanted dBc | Median unwanted dBc | Worst harmonic-shape error dB | Baseline instr/frame | v3 instr/frame |
|---|---:|---:|---:|---:|---:|---:|
| Legacy | 1x | -22.75 | -37.66 | -15.19 | 110,772 | 64,459 |
| Legacy | 2x | -42.95 | -61.76 | -27.27 | 221,844 | 129,086 |
| Legacy | 4x | -56.39 | -80.90 | -27.26 | 443,327 | 257,827 |
| Spline | 1x | -31.95 | -49.31 | -6.66 | 155,937 | 76,680 |
| Spline | 2x | -82.21 | -102.15 | -34.89 | 281,299 | 137,433 |
| Spline | 4x | -99.41 | -116.81 | -35.64 | 490,784 | 245,984 |
| Spline Optimized | 1x | -34.50 | -51.82 | -5.89 | 162,699 | 78,167 |
| Spline Optimized | 2x | -82.21 | -102.15 | -34.89 | 281,237 | 137,370 |
| Spline Optimized | 4x | -100.03 | -117.33 | -36.67 | 499,991 | 247,930 |
| Lagrange | 1x | -23.65 | -40.38 | -15.63 | 243,700 | 117,551 |
| Lagrange | 2x | -59.50 | -79.98 | -30.80 | 421,908 | 210,359 |
| Lagrange | 4x | -92.24 | -110.15 | -27.46 | 751,528 | 385,697 |
| Spectral | 1x | -56.63 | -124.82 | -27.82 | 413,301 | 106,207 |
| Spectral | 2x | -91.01 | -104.23 | -27.27 | 826,928 | 238,632 |
| Spectral | 4x | -96.88 | -115.83 | -27.26 | 1,333,570 | 414,224 |

The Spline label already selects `SplineOptimized` at 2x through `Antialiasing::for_factor()`. Their 2x samples and instruction counts are therefore identical. At 4x the optimized kernel buys only 0.62 dB in the worst alias case for 0.8% more v3 instructions and 1.9% more baseline instructions.

Lagrange has no remaining Pareto point. At 2x it is 22.71 dB worse than Spline while using 50.0% more baseline and 53.1% more v3 instructions. At 4x it remains 7.79 dB worse and over 50% more expensive.

### Is oversampling wasted?

Not at the current kernel quality:

- Spline Optimized 1x to 2x improves the worst result from `-34.50` to `-82.21 dBc`, a 47.71 dB gain, for 72.9% more baseline and 75.7% more v3 instructions.
- Spline Optimized 2x to 4x improves the worst result from `-82.21` to `-100.03 dBc`, a further 17.82 dB, for 77.8% more baseline and 80.5% more v3 instructions.

Normal 2x is justified. Ultra 4x is a real quality improvement but not a sensible default. A future same-rate algorithm must beat `-82 dBc` on the same stress matrix before replacing 2x; current 1x BLEP/BLAMP does not.

## Spectral is not a shipping winner yet

Spectral 1x is exceptionally accurate when its table path is active. At 5126.95 Hz, static saw unwanted energy is `-123.40 dBc`, versus `-83.37 dBc` for Spline 2x, and its gain-normalized harmonic error is below `-118 dB`.

Below the 128-harmonic threshold it falls back to Spline Optimized. The sound then becomes the 1x spline result, but the engine retains Spectral's non-block execution path. Saw instructions per frame show the asymmetry:

| MIDI note | Spline Opt 2x baseline | Spectral 1x baseline | Spline Opt 2x v3 | Spectral 1x v3 |
|---:|---:|---:|---:|---:|
| 36 | 125,708 | 402,862 | 44,369 | 95,642 |
| 69 | 154,141 | 402,862 | 56,584 | 106,221 |
| 105 | 240,000 | 285,606 | 93,276 | 106,581 |

AVX2 gathers make Spectral much less costly in the v3 binary, but the baseline low-note penalty remains over 3x. This is not a portable default.

Phase warp is the decisive blocker. Worst unwanted energy across PWM/Bend/Harm at 98%, two notes, and four non-sine shapes was:

| Algorithm | 1x | 2x | 4x |
|---|---:|---:|---:|
| Legacy | -21.43 | -39.68 | -52.61 |
| Spline | -27.85 | -51.60 | -75.90 |
| Spline Optimized | -28.78 | -51.60 | -78.04 |
| Lagrange | -20.90 | -43.32 | -62.82 |
| Spectral | **-17.34** | **-29.12** | **-35.34** |

Spectral table truncation bandlimits the unwarped periodic waveform, not the spectrum created by nonlinear phase remapping. Spectral must remain hidden/lab-only until phase warp has a separately bandlimited construction. Existing Spectral sessions should fall back to procedural Spline when warp is active rather than taking the current table path.

## Signal truth

- Unwarped sine, triangle, saw, and morph DC stayed within `8.26e-10` of zero.
- A width-0.37 pulse has ideal DC `-0.26`; measured values ranged from `-0.259773` to `-0.261535`. Default Spline 2x produced `-0.261535`, consistent with its approximately +0.05 dB low-frequency gain.
- Default Spline 2x sine fundamental gain ranged from +0.012 to +0.086 dB over the four notes. Saw fundamental gain ranged from +0.050 to -0.099 dB. This is small but not mathematically unity.
- Constant-power static morphing can peak above unity. The default triangle-saw midpoint reached `1.5364`; Lagrange reached `1.5496`. Keep output headroom.
- Relative to Spline 2x, factor 1x is exactly about half a host sample early and factor 4x about a quarter sample late. This comes from taking the last internal sub-sample before decimator output. The synth has no dry parallel path, but quality modes are not phase-transparent despite sharing the same integer latency report.

## Architecture and dynamic morph

For Spline Optimized 2x saw, v3 retired about 2.7x fewer instructions than baseline. Five default 2x renders differed by only `-141.35` to `-143.18 dB` RMS, with maximum sample error `4.77e-7`. Separate v3 and conservative binaries therefore materially improve CPU without materially changing sound.

Dynamic 24x64x3 morph results at 32,768 host frames were:

| Build | Serial ns/frame | Pooled ns/frame | Pool fallbacks |
|---|---:|---:|---:|
| baseline | 23,237 | 6,413 | 0 |
| x86-64-v3 | 11,372 | 3,157 | 0 |

The modern build is about 51% faster in both modes. Pooled measurements were not CPU-pinned; pinning the process to one core also pins its helpers and invalidates the pool benchmark.

## Next experiments that still earn their complexity

1. **Fix the Spectral fallback seam before revisiting it.** Route low-note fallback through the normal block-major Spline path and bypass Spectral under any active phase warp. Retain it only if both baseline and v3 improve by at least 5% without a transition discontinuity.
2. **Attack 1x discontinuities, not the whole synth rate.** Test optimized longer-support spline/minBLEP, BLIT fractional-delay, DPW/PTR/EPTR, or derivative-jump BLEP/BLAMP against the exact `-82.21 dBc` Normal gate. The existing primary-source survey is in `perfect-oscillator-antialiasing.md` and `oversampling-alternatives.md`.
3. **Treat warped pulse Bend as its own event problem.** It is the current Spline 2x worst case at `-51.60 dBc`; 4x reaches `-78.04 dBc`. Correct every value/derivative discontinuity at its actual fractional raw-phase event before spending more CPU globally.
4. **Only then consider a high-note spectral hybrid.** Spectral 1x is pristine and near the 2x procedural cost at high pitch, especially on v3. A hybrid needs hysteresis/crossfade, mixed-note handling, baseline dispatch, and identical gain/phase at the handoff; it is not a one-line default change.
5. **Keep target-specific release binaries.** The v3 gain is too large to discard, while the baseline binary remains necessary for older CPUs. The two builds must continue to share presets and stay below the measured `-140 dB` output-difference floor.

No DSP or test files were changed during this audit.
