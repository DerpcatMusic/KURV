# Canonical DPW round 6 (rejected)

Date: 2026-08-30

Baseline: `5a967bc` (including current `main` through merge `17419d9`)

Decision: reject; no runtime code retained

## Question and implementation

This round revisited saw DPW with the current 1x VA kernels, deliberately using
the smallest useful order and only one higher order:

- DPW2: first difference of `x^2`, normalized by exactly `4 * phase_step`.
- DPW4: third difference of `x^4 - 2x^2`, evaluated in `f64` and normalized by
  exactly `192 * phase_step^3`.

The test-only prototypes primed every history from the analytical preceding
phases, so reset/startup did not contain an empty-history transient. Coherent
periods made the expected DC zero; the established ideal-projection comparator
also fitted DC, gain, phase, and fractional delay rather than crediting a
misaligned curve. DPW2 had scalar, `f32x4`, and `f32x8` forms. DPW4 had scalar
and two-bank `f64x4`/x8 forms. All state was fixed-size and the sample loops did
no allocation, locking, I/O, or lookup.

This is distinct from the earlier DPW5 experiment: it directly measures DPW2
and DPW4 against the current canonical saw evaluator and exact ideal projection.

## Exact command and workload

```text
cargo fmt
taskset -c 8 cargo test dpw_saw_quality_transition_and_cpu_report --lib --release --locked -- --ignored --nocapture --test-threads=1
```

Quality used 48 kHz coherent periods 1745, 109, and 7 (27.507, 440.367, and
6857.143 Hz). CPU medians used five runs, one million scalar samples or 500,000
SIMD vectors at 440 and 7040 Hz. Values below are ns per generated scalar sample
for scalar and ns per generated vector for x4/x8, matching the existing
experiment convention. The current columns call the real `generate_saw`,
`generate_saw4`, and `generate_saw8` kernels.

## Ideal-projection quality

| Hz | Current RMS / peak | DPW2 RMS / peak | DPW4 RMS / peak |
|---:|---:|---:|---:|
| 27.507 | 0.016720 / 0.752724 | 0.017697 / 1.146269 | 0.007032 / 0.189242 |
| 440.367 | 0.044112 / 0.285861 | 0.017358 / 0.094298 | 0.028135 / 0.188755 |
| 6857.143 | 0.173628 / 0.290918 | 0.067963 / 0.098142 | 0.111904 / 0.192232 |

DPW2 improves the mid/high-note projection but already loses both RMS and peak
at the low-note gate. Its 1.146 low-note peak is also the expected warning sign
for cancellation-sensitive normalization at small phase increments. DPW4 avoids
that quality loss by paying for `f64` histories and arithmetic.

## Pitch changes and reset

The exact-history reprime path was compared with carrying histories across
abrupt phase-step changes after a settled 440 Hz prefix:

| Relative step change | DPW2 carry / reprime peak | DPW4 carry / reprime peak |
|---:|---:|---:|
| +0.0833% | 0.988689 / 0.988689 | 0.974088 / 0.974088 |
| +0.7500% | 0.984838 / 0.984838 | 1.691710 / 0.967198 |

DPW2 is naturally bounded for these changes because it keeps one previous
polynomial value. DPW4 is not: carrying its three histories creates a 1.69 peak
for the larger change. Repriming removes the spike but changes state at the
pitch event and would require explicit transition policy in production.

## CPU gate

| Hz | scalar current / DPW2 / DPW4 | x4 current / DPW2 | x8 current / DPW2 / DPW4 |
|---:|---:|---:|---:|
| 440 | 3.827 / 9.255 / 7.443 | 2.542 / 2.109 | 7.172 / 2.990 / 16.383 |
| 7040 | 6.385 / 9.026 / 7.396 | 5.010 / 2.370 | 13.123 / 3.074 / 19.071 |

DPW2 vectorizes well in isolation, but the scalar form is 2.42x slower at
440 Hz and 1.41x slower at 7040 Hz. DPW4 loses scalar and x8 CPU and additionally
requires `f64` history. Therefore neither order clears the shared micro gate
uniformly across the scalar and existing x4/x8 structural lanes.

## Verdict and limitations

Saw failed before production integration: DPW2 loses low-note accuracy and
scalar cycles, while DPW4 loses real-path cycles and has an unsafe carried
pitch transition. Per the staged gate, square/pulse shifted differences and a
derived triangle were not implemented. The default 2x and full unison/block
workloads were also not run: a candidate that already loses the required 1x
scalar/quality gate cannot become uniformly Pareto-safe after adding routing,
state, transition, and crossover overhead. Consequently no production source,
state bytes, publication cost, or RT behavior changed.
