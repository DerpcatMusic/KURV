# Residual-weighted uniform C1 compiler, round 5 (2026-08-30)

## Outcome

Rejected for production. A two-pass residual-weighted least-squares fit kept the
existing 16 uniform cubic segments, 64 `f32` coefficients (256 bytes), endpoint
values, C1 unions, and explicit hard-point separation. It was not Pareto-safe
against the shipped shared-slope selector: useful source-error selections included
strict ideal-bandlimited regressions, while the established 25% RMS safety gate
selected no curves. No runtime/compiler code or version changed.

## Candidate

Starting from the shipped uniform shared-slope least-squares candidate, each of two
additional compile-only fits weighted its 31 interior samples per segment by
`1 + 16 * normalized_residual^2`. This is a small IRLS-style approximation to a
minimax fit: large residuals exert more influence without changing segment
boundaries, endpoint interpolation, hard/wrap discontinuity handling, storage, or
the evaluator.

## 512-curve corpus

The same deterministic corpus and gates covered smooth, hard, clustered,
near-duplicate, extrema, wrap, max-knot, and random curves. Candidates had to pass
bounded peak error, every-knot, analytic extrema/clamp/overshoot, artificial
derivative, hard-point, and wrap guards before the dense 8192-point source and
ideal-bandlimited oracle.

| Cheap RMS reduction | Selected | Dense source regressions | Strict BL regressions |
| ---: | ---: | ---: | ---: |
| 0.1% | 54 | 0 | 7 |
| 1% | 48 | 0 | 3 |
| 5% | 3 | 0 | 3 |
| 10% | 3 | 0 | 3 |
| 25% | 0 | 0 | 0 |

At 0.1%, selections were 21 hard, 3 near-duplicate, and 30 max-knot curves. All
three candidates surviving the 5% and 10% gates were near-duplicate curves, and
all three regressed ideal-bandlimited output. Representative compile cost was
73.414 us versus 10.974 us for the shipped shared-slope fit (6.69x slower).

## Commands and evidence

```text
taskset -c 4 cargo test --release --no-default-features --lib \
  wave_curve::compiler_experiment::residual_weighted_uniform_c1_selector_sweep \
  --locked -- --ignored --nocapture
```

Passed 1/1. Results are in the table above; `WaveCurveRt` remained 256 bytes.

```text
taskset -c 4 cargo test --release --no-default-features --lib \
  wave_curve::compiler_experiment::cheap_uniform_least_squares_c1_selector_sweep \
  --locked -- --ignored --nocapture
```

Passed 1/1. The shipped selector remained at 64/512 with zero dense-source and
strict ideal-bandlimited regressions.

```text
cargo test --release --no-default-features --lib \
  wave_curve::topology_tests --locked
```

Passed 3/3: neutral multi-knot, ideal saw, and ideal triangle topology contracts.

```text
taskset -c 5 cargo test --release --no-default-features --lib \
  oscillators::va::experiment::shipping_1x_va_quality_and_cpu_report \
  --locked -- --ignored --nocapture
```

Passed 1/1 in 4.41 s. Current 48 kHz/440 Hz median 1x CPU was 7.227 ns/sample saw,
6.882 square, 6.809 pulse31, 10.597 triangle, and 10.716 drawn. The candidate never
entered production, so layout, publication, evaluator instructions, audio quality,
and audio-path cost are unchanged by this experiment.
