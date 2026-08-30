# Factored DPW canonical second shot (2026-08-30)

## Decision

Do **not** integrate this round into production. Stateless factored DPW2 is a
useful new frontier: it removes the low-frequency cancellation and retained
history of the earlier canonical DPW experiment, improves ideal-reference RMS
error strongly at normal and high notes, and is often cheaper than the current
1x `SplineOptimized` path. It is not universally Pareto-safe because its
low-note discontinuity peak is slightly worse, every rapid-pitch transition
probe regresses, and scalar square/pulse are slower at 440 Hz.

No production DSP, oscillator state/layout, preset contract, or version was
changed. The ignored test-only probe remains in `experiment.rs` so a future
crossover or hybrid can reuse the exact comparison.

## Question and prior gap

Baseline commit: `7850f6b` (`Add static phase-warp ideal reference`). Baseline
algorithm: current VA `SplineOptimized` at 1x oversampling.

Round 6 tested direct f32 polynomial differences for DPW2 and f64 DPW4. DPW2
lost low-note accuracy through cancellation and carried previous-polynomial
history; square, pulse, and triangle were then skipped. This second shot
algebraically factors the finite differences before evaluating them. That gives
bounded piecewise formulas with no persistent state beyond the oscillator's
existing phase.

Candidates:

- **DPW2 saw:** the first difference of the periodic quadratic. Away from the
  wrap it reduces to `2p - 1 - h`; across the wrap it uses
  `(1 - h) * (1 - 2p/h)`.
- **DPW3 saw:** the second difference of the periodic cubic `x^3 - x`, factored
  into two wrap intervals and the regular linear region.
- **DPW23:** DPW2 delayed by half a sample, extrapolated as
  `2 * DPW2 - DPW3`. Its theoretical response is
  `(2 sinc - sinc^2) exp(-jw)`.
- **Square and pulse:** differences of two phase-shifted DPW saws plus the exact
  DC term `2w - 1`.
- **Triangle:** DPW2 only, because the candidate is exactly the first finite
  difference of the triangle's periodic piecewise-quadratic primitive. Higher
  orders were not invented without the corresponding mathematical primitive.

The probe verifies the factored scalar formulas against the original periodic
polynomial differences at periods 1745, 109, and 7. Scalar and f32x8 paths use
the same formulas; the x8 reciprocal is hoisted once per block. Candidate RT
state cost is zero bytes and the measured loops allocate, lock, log, and perform
no I/O.

## Method

The quality path renders both baseline and candidate through the same
factor-1 `StereoOversampler`, then aligns them to an oversampled ideal
band-limited reference. Raw candidate samples are analyzed separately against
the known DPW transfer response to attribute wanted-transfer error, folded
alias/numeric error, and off-grid artifacts without confusing the common
oversampler delay with oscillator error. The report also records DC, gain,
boundary/global residual, reset replay, and a repeating rapid-pitch schedule:

`440 Hz x 24 | 7040 Hz x 32 | 110 Hz x 24 | 12000 Hz x 32`

CPU measurements run 20,000 real 24- or 32-frame blocks, repeat five times,
and report the median plus range. Scalar values are ns/frame for one oscillator;
x8 values are ns/frame for eight voices including stereo accumulation and phase
load/store, before the common oversampler. The reference uses the production
scalar generator and production x8 block accumulation seams, not an isolated
polynomial microbenchmark.

Machine: AMD Ryzen 7 7800X3D, release build, `-C target-cpu=native`, pinned to
logical CPU 8. Background VM load was present, so medians and ranges matter more
than single extrema.

```sh
CARGO_TARGET_DIR=/tmp/kurv-va-dpw-target \
  CARGO_BUILD_JOBS=1 \
  RUSTFLAGS='-C target-cpu=native' \
  cargo test --release --no-default-features --lib --locked \
    factored_dpw_canonical_quality_transition_and_cpu_report --no-run

taskset -c 8 \
  /tmp/kurv-va-dpw-target/release/deps/pure_va_dispersion_core-c78bc2803a9e2383 \
  oscillators::va::experiment::factored_dpw_canonical_quality_transition_and_cpu_report \
  --exact --ignored --nocapture --test-threads=1
```

Result: 1 passed, 0 failed, 390 filtered out, 5.42 s.

## Ideal-reference quality

Values below are aligned RMS error in dB; more negative is better. Delta is the
candidate improvement over the current path.

| Wave | Kernel | 27.5 Hz current -> candidate | 440.4 Hz | 6857.1 Hz |
|---|---|---:|---:|---:|
| Saw | DPW2 | -30.761 -> -31.888 (+1.127) | -22.289 -> -30.458 (+8.169) | -9.614 -> -17.761 (+8.147) |
| Saw | DPW3 | -30.761 -> -31.096 (+0.335) | -22.289 -> -24.954 (+2.665) | -9.614 -> -12.410 (+2.796) |
| Saw | DPW23 | -30.761 -> -29.540 (-1.221) | -22.289 -> -24.954 (+2.665) | -9.614 -> -12.410 (+2.796) |
| Square | DPW2 | -32.572 -> -32.921 (+0.349) | -24.405 -> -28.543 (+4.138) | -11.104 -> -18.796 (+7.692) |
| Square | DPW3 | -32.572 -> -33.183 (+0.611) | -24.405 -> -28.735 (+4.330) | -11.104 -> -15.581 (+4.477) |
| Pulse 31% | DPW2 | -32.526 -> -33.514 (+0.988) | -24.130 -> -32.013 (+7.883) | -12.202 -> -18.604 (+6.402) |
| Pulse 31% | DPW3 | -32.526 -> -32.914 (+0.388) | -24.130 -> -27.376 (+3.246) | -12.202 -> -15.925 (+3.723) |
| Triangle | DPW2 | -57.342 -> -57.343 (+0.001) | -51.640 -> -61.445 (+9.805) | -15.670 -> -26.490 (+10.820) |

The important counterexample is peak error at 27.5 Hz. Saw rises from
`0.752724` to `0.814368` with DPW2 and `0.823815` with DPW3. Square rises from
`1.055902` to `1.067498`; pulse rises from `0.808497` to `0.814397`; triangle
rises from `0.001585` to `0.001706`. Better RMS alone therefore cannot justify
universal replacement.

DPW2 raw attribution behaves coherently: saw wanted/alias error is
`-32.601/-25.460 dB` at 440 Hz and `-19.965/-12.783 dB` at 6857 Hz; triangle is
`-63.264/-66.395 dB` and `-26.948/-38.653 dB`. Off-grid residuals at exact
integer periods fall below `-68 dB` at 440 Hz and below `-309 dB` at 6857 Hz.
Pulse DC remains the expected `-0.38` within float noise.

## Rapid transitions and reset

| Wave/kernel | Current pitch-event step | Candidate | Current global step | Candidate |
|---|---:|---:|---:|---:|
| Saw DPW2 | 1.587572 | 1.674217 | 1.587572 | 1.870696 |
| Saw DPW3 | 1.587572 | 1.706387 | 1.587572 | 1.706387 |
| Saw DPW23 | 1.587572 | 1.984317 | 1.587572 | 2.485223 |
| Square DPW2/3 | 1.880906 | 2.000000 | 1.880906 | 2.000000 |
| Pulse DPW2 | 1.880906 | 2.000000 | 1.880906 | 2.000000 |
| Triangle DPW2 | 0.991178 | 1.481522 | 0.991178 | 1.481522 |

All current and candidate reset replay errors are exactly zero. The transition
regression is inherent to immediately changing the step-dependent waveform,
not stale retained history. DPW23 is particularly unsafe: its peak reaches
`1.245` for saw and `1.315` for pulse.

## CPU

Median delta versus current; negative is faster.

| Wave/kernel | 440 scalar 24/32 | 440 x8 24/32 | 7040 scalar 24/32 | 7040 x8 24/32 |
|---|---:|---:|---:|---:|
| Saw DPW2 | -19.05% / -26.05% | -14.48% / -15.03% | -43.16% / -45.84% | -31.53% / -31.94% |
| Saw DPW3 | -17.37% / -13.27% | -5.54% / -6.64% | -29.83% / -39.33% | -19.60% / -32.84% |
| Saw DPW23 | +54.69% / +53.14% | +19.47% / +18.34% | +10.55% / +13.90% | -6.70% / -3.80% |
| Square DPW2 | +11.24% / +8.89% | -12.82% / -15.05% | -36.04% / -35.06% | -53.78% / -54.04% |
| Square DPW3 | +19.99% / +21.76% | +3.26% / +0.05% | -33.18% / -32.27% | -34.79% / -41.56% |
| Pulse DPW2 | +7.89% / +11.83% | -18.34% / -22.53% | -35.23% / -33.73% | -54.74% / -54.13% |
| Pulse DPW3 | +15.17% / +21.54% | -3.97% / -5.70% | -28.79% / -25.25% | -45.13% / -43.66% |
| Triangle DPW2 | -31.63% / -31.69% | -34.51% / -30.20% | -56.68% / -56.83% | -73.86% / -69.29% |

Representative ranges were tight for most rows: saw DPW2 scalar at 440 Hz/24
frames was `1.294..1.319 ns` candidate versus `1.587..1.614 ns` current;
triangle DPW2 x8 at 7040 Hz/24 frames was `3.625..3.997 ns` versus
`13.935..14.052 ns`. A few high-frequency x8 extrema show background jitter,
but the median direction repeated across earlier runs.

## Outcome

- **DPW2:** retain as a research frontier, not a production replacement.
- **DPW3:** reject for now; it gives less quality and smaller CPU gains than
  DPW2 while sharing the transition problem.
- **DPW23:** reject. Extrapolation does not survive actual peak, transition, and
  CPU checks despite attractive wanted-transfer cancellation in places.
- **Square/pulse by saw differences:** mathematically sound and strong at high
  notes/x8, but the scalar low-note cost and transition step prevent adoption.
- **Triangle DPW2:** the strongest isolated result, but still fails rapid pitch
  transitions and the strict low-note peak criterion.

If this family gets another round, the narrow next question is a smooth,
phase-aligned crossover between current and DPW2 based on phase step, with
explicit pitch-event smoothing. It must beat these exact transition and 24/32
block matrices before any production edit.
