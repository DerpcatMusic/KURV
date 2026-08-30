# Integrated corrected-square triangle, round 19 (rejected)

Date: 2026-08-30

Verdict: reject. Trapezoidal integration exposes a real 6-8 kHz x8 CPU/quality region, but it is not stable without correction, loses CPU at low notes and 12 kHz, and requires eight retained bytes per oscillator plus pitch-transition handling. Production remains unchanged.

## Candidate

This is distinct from the earlier BLIT integrator. Its input is KURV's shipping optimized spline-corrected 50% square. The minimum credible discrete integrator was:

```text
state += 2 * phase_step * (previous_square + square)
output = state * tan(pi * phase_step) / (pi * phase_step)
```

The tangent term exactly corrects the trapezoidal frequency response at the fundamental. It does not simultaneously correct every retained odd harmonic. The temporary x8 block renderer called the actual constant-step square path, included eight scalar tangent normalizations per block, integrated 64 frames, and accumulated stereo output. It retained one previous-square and one integrator float per lane. The prototype was reverted after the gates below failed.

Euler had an unavoidable sample of phase lag and RMS errors from 0.031 at 750 Hz to 0.405 at 12 kHz. A minimally leaky Euler form bounded DC but retained essentially the same error. Only trapezoidal integration earned the real renderer benchmark.

## Commands and release CPU

```text
CARGO_BUILD_JOBS=1 CARGO_PROFILE_RELEASE_STRIP=none \
RUSTFLAGS='-C target-cpu=native' \
  cargo test integrated_square_triangle_cpu_report --lib --release --locked --no-run

taskset -c 8 \
  target/release/deps/pure_va_dispersion_core-bcfc5e2fdb5df6ce \
  oscillators::va::experiment::integrated_square_triangle_cpu_report \
  --ignored --exact --nocapture --test-threads=1
```

Ryzen 7 7800X3D, detected AVX2/FMA current backend, 20,000 64-frame stereo x8 blocks per cell, medians of five runs.

| Hz | current triangle ns/block | integrated square ns/block | ratio |
|---:|---:|---:|---:|
| 750 | 289.496 | 290.195 | 1.002 |
| 3,000 | 310.589 | 319.360 | 1.028 |
| 6,000 | 456.210 | 364.877 | 0.800 |
| 7,040 | 665.311 | 447.761 | 0.673 |
| 8,000 | 518.015 | 421.122 | 0.813 |
| 9,600 | 514.647 | 499.020 | 0.970 |
| 12,000 | 514.539 | 520.146 | 1.011 |

These are favorable candidate numbers: they exclude the required DC/rephase correction and pitch-change crossfade. Even so, the candidate loses at 750, 3,000, and 12,000 Hz and has only 3% headroom at 9,600 Hz. Scalar and x4 promotion was stopped after the real x8 path failed the common gate; adding narrower lanes cannot repair state fidelity and would broaden fallback/ownership complexity.

## Ideal projection, phase, DC, and gain

Offline coherent cycles used the exact current optimized BLEP/BLAMP polynomials. Candidate RMS below is after the most favorable integer phase alignment, DC removal, and scalar gain fit; those corrections are not free runtime operations.

| Hz | current RMS | trapezoid RMS | fitted gain | fitted DC |
|---:|---:|---:|---:|---:|
| 750 | 0.003329 | 0.004255 | 1.003 | -0.0252 |
| 3,000 | 0.026680 | 0.026791 | 1.046 | -0.1009 |
| 6,000 | 0.075852 | 0.052145 | 1.188 | -0.2032 |
| 6,857 | 0.094936 | 0.058082 | 1.252 | -0.2464 |
| 8,000 | 0.108570 | 2.8e-9 | 1.359 | -0.2139 |
| 9,600 | 0.149392 | 0.002526 | 1.564 | -0.2946 |
| 12,000 | 0.216842 | 1.1e-16 | 2.052 | -0.4156 |

At cap 1 the fundamental normalization can be exact; at cap 3 the harmonic-dependent trapezoidal response prevents one gain from matching every wanted harmonic. At 750 Hz the candidate is already less accurate than current. The fitted DC is large and phase-dependent, so simply resetting state to zero is invalid.

## Long-run drift and pitch/reset behavior

A f32, non-coherent 300,000-sample run used the same corrected-square source and fundamental normalization. Mean output over successive 50,000-sample windows drifted:

| Hz | first mean | last mean | drift |
|---:|---:|---:|---:|
| 6,001.3 | -0.7362 | -0.7472 | -0.0110 |
| 7,040 | -0.7626 | -0.7702 | -0.00759 |
| 8,372.0 | -0.8125 | -0.8119 | +0.00054 |
| 10,003.7 | -0.8985 | -0.8988 | -0.00029 |
| 11,999.1 | -1.0389 | -1.0465 | -0.00756 |

No run became non-finite, but the offset is neither low nor stationary. Per-cycle or per-block authoritative rephase would bound it, but requires evaluating the triangle value that this backend is meant to replace and creates a correction step at every boundary. A leaky state removes long-term drift only by reintroducing the measured Euler-like phase/amplitude error.

Abrupt pitch changes also rescale the retained state. Fundamental normalization rises from 1.055 at 6 kHz to 1.103 at 8 kHz and 1.273 at 12 kHz, so retaining state produces an instantaneous gain jump; renormalizing state avoids that jump but still changes the harmonic-dependent fit. Note-on/reset needs phase-derived centered initialization, not zero. Pack movement therefore must preserve both floats per oscillator.

## State, memory, and RT safety

Correct ownership needs `integrator` and `previous_square` on every `VaOscillator`: eight logical bytes. The current struct is 40 bytes; unlike round 18's one-byte selector, two floats exceed its one padding byte and grow the mirrored layout to 48 bytes (20%). Structural repacking can copy the state correctly, but resets, phase changes, pitch changes, and shape/eligibility transitions all need explicit correction policy.

The temporary block code used only fixed stack arrays and bounded loops, with no allocation, locks, I/O, or logging. A production implementation could remain RT-safe, but not minimal or drift-free.

## Decision

Reject and revert. The mid/high x8 CPU result is interesting, and cap-1 ideal accuracy is excellent after favorable correction. The complete algorithm loses low/12 kHz CPU, loses low-note fidelity, drifts under non-coherent f32 phase, needs costly authoritative centering, and expands every oscillator by 20%. Correction and transition work can only reduce the already marginal 9.6 kHz/12 kHz CPU result. No production code, state, test helper, or version change is retained.

Limitations: wanted/unwanted separation is implicit in the exact coherent ideal comparison; the single normalization's cap-3 harmonic mismatch is reflected in RMS rather than a retained per-bin table. Scalar/x4 timings were deliberately stopped after the required real x8/state gate failed.
