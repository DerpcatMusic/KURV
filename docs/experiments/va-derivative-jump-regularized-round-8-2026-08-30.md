# Derivative-jump-regularized shared-slope compiler, round 8 (2026-08-30)

## Outcome

Ship in 0.8.7. A second compile-only candidate uses independent uniform Hermite
endpoint slopes with a small `1e-6` Tikhonov penalty on slope differences at
smooth boundaries. Explicit hard knots and hard wrap events are never penalized.
A bounded selector accepts it only over the already selected legacy/shared-C1
curve and only when source, knot, range, maximum derivative jump, derivative-event
energy, and hard/wrap contracts all prove no regression.

The runtime remains the winning 16-cubic, 64-`f32`, 256-byte `WaveCurveRt` with
unchanged `eval`, `eval4`, and `eval8`. All extra fitting and proof work is outside
the audio callback.

## Why finite regularization

The existing shared-slope fit unions the two slopes at every smooth boundary, so
its derivative jump is exactly zero. Adding a penalty to those already shared
variables is mathematically redundant. This experiment instead keeps the two
endpoint slopes independent and adds `lambda * (left - right)^2` only across
smooth joins. It can trade tiny, bounded derivative events for source/BL accuracy,
but the selector forbids any increase in maximum smooth jump or total smooth-event
energy relative to the curve already selected for publication.

## Bounded production selector

The retained `lambda = 1e-6` candidate must satisfy:

- at least 1% RMS improvement on the existing bounded 256-phase source probe
  (`squared_error <= current * 0.9801`);
- peak error no worse plus `1e-7`;
- every explicit editor knot no worse plus `1e-6`;
- no extra analytic clamp crossing or range overshoot plus `1e-6`;
- maximum smooth derivative jump no worse plus `1e-6`;
- total smooth derivative-event energy no greater than current;
- intentional hard and wrap jumps retain at least 95% when present.

The old legacy-vs-shared-C1 25% selector remains the first stage. The regularized
candidate is compared only with that winner, preserving deterministic fallback.

## 512-curve oracle

The retained candidate selected 116/512 curves with:

- dense source regressions: 0;
- strict worst-of-three ideal-BL regressions: 0;
- ideal-BL wins: 116/116;
- category distribution `[smooth, hard, clustered, near_duplicate, extrema,
  wrap, max_knots, random]`: `[25, 64, 0, 5, 0, 0, 21, 1]`;
- dense RMS reduction min/median/max:
  0.940094% / 2.425071% / 80.998272%;
- worst-of-three BL delta min/median/max:
  -14.429213 / -0.169407 / -0.001654 dB (negative is better);
- derivative-event energy reduction min/median/max:
  0.000000 / 0.000122 / 218.232438;
- coefficient-interpolation identity failures: 0/2,044 for the retained lambda;
- representative candidate compile cost: 19.077 us.

The logarithmic sweep also showed why `1e-6` is retained. `1e-4` selected 80 with
zero BL regressions, `1e-2` selected 58 with zero, while `1` selected 57 but had
three strict BL regressions. The smallest lambda produced the broadest safe wins.

## Commands and evidence

```text
taskset -c 4 cargo test --release --no-default-features --lib \
  wave_curve::compiler_experiment::derivative_jump_regularized_selector_report \
  --locked -- --ignored --nocapture
```

Passed 1/1. The test asserts the production candidate is byte-identical to the
prototype and the production decision matches the bounded selector on all 512
curves, then validates selected curves with the dense source and FFT/BL oracle.

```text
taskset -c 4 cargo test --release --no-default-features --lib \
  wave_curve::compiler_experiment::cheap_uniform_least_squares_c1_selector_sweep \
  --locked -- --ignored --nocapture
```

Passed 1/1. The first-stage selector remains 64/512 with zero dense-source or BL
regressions.

```text
cargo test --release --no-default-features --lib \
  wave_curve::topology_tests --locked
```

Passed 3/3: neutral multi-knot, ideal saw, and ideal triangle.

```text
taskset -c 5 cargo test --release --no-default-features --lib \
  oscillators::va::experiment::shipping_1x_va_quality_and_cpu_report \
  --locked -- --ignored --nocapture
```

Passed 1/1 in 4.45 s. Current 48 kHz/440 Hz 1x medians were 6.767 ns/sample saw,
6.556 square, 6.675 pulse31, 11.824 triangle, and 9.851 drawn. Runtime evaluation
is unchanged; the extra selector work occurs only while compiling editor curves.

```text
cargo test --release --no-default-features --lib --locked
```

354 passed, 3 failed, 14 ignored in 2.16 s. All wave-curve tests passed. Current
checkout failures were outside this change: two resynth artifact/vocoder tests and
structural LFO advancement. This run does not establish whether they predate the
change.
