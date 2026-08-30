# Canonical Clenshaw round 7 (rejected)

Date: 2026-08-30

Baseline: `7aa501a` (production DSP unchanged)

Machine: AMD Ryzen 7 7800X3D, Linux x86-64 host-default release, CPU 8

Verdict: rejected; no runtime or benchmark code retained

## Candidate and gate

The saw-only gate evaluated the exact finite Fourier sum through the identity
`sin(k theta) = sin(theta) U_(k-1)(cos(theta))`. One `sin_cos` of authoritative
phase fed either:

- the requested backward Clenshaw recurrence over saw coefficients
  `-2 / (pi k)`, or
- the minimum comparison variant, a forward Chebyshev recurrence accumulating
  the same coefficients (the stateless analogue of prior additive evaluation).

Both variants were implemented for scalar, `f32x4`, and `f32x8`, with no
per-harmonic oscillator state. Phase is the only carried value, so reset and
arbitrary pitch changes cannot invalidate harmonic histories. The loops use
fixed locals and perform no allocation, lock, I/O, coefficient lookup, or
publication. Caps 2, 3, 6, and 13 were tested.

## Exact command and workload

```text
cargo fmt
taskset -c 8 cargo test clenshaw_saw_gate_report --lib --release --locked -- --ignored --nocapture --test-threads=1
```

CPU values are median nanoseconds from five repetitions of one million scalar
samples or 500,000 SIMD vectors. Current calls the real canonical
`generate_shape_step`, `generate_saw4`, or `generate_saw8` path. SIMD phases
used distinct lane offsets. Quality used 4096 coherent periods and the shared
ideal projection/alignment comparator.

## Quality and numerical agreement

| Cap | Ideal RMS | Ideal peak | Max Clenshaw vs forward delta |
|---:|---:|---:|---:|
| 2 | 0.000369468 | 0.002128210 | 1.19e-7 |
| 3 | 0.000445817 | 0.003192312 | 1.79e-7 |
| 6 | 0.000621178 | 0.006384594 | 3.58e-7 |
| 13 | 0.000906971 | 0.013832979 | 7.15e-7 |

The residual is `f32` trigonometric/phase/alignment error, not a missing
harmonic. Backward and forward recurrences agree closely enough that CPU, not
coefficient accuracy, decides the gate.

## CPU

Each cell is `current / forward / Clenshaw`.

| Cap | Hz | Scalar | x4 | x8 |
|---:|---:|---:|---:|---:|
| 2 | 440 | 5.049 / 31.381 / 22.740 | 2.823 / 18.470 / 17.613 | 6.208 / 34.261 / 23.127 |
| 2 | 7040 | 16.558 / 32.982 / 28.089 | 5.621 / 11.256 / 14.920 | 19.582 / 28.728 / 23.164 |
| 3 | 440 | 6.023 / 31.290 / 19.969 | 2.794 / 13.027 / 12.891 | 6.231 / 24.351 / 22.287 |
| 3 | 7040 | 8.870 / 26.142 / 21.344 | 5.654 / 12.878 / 12.780 | 12.374 / 23.052 / 20.734 |
| 6 | 440 | 4.618 / 40.411 / 30.045 | 2.777 / 18.650 / 17.901 | 6.198 / 30.592 / 26.017 |
| 6 | 7040 | 9.700 / 40.620 / 29.183 | 5.532 / 18.560 / 17.218 | 12.309 / 31.859 / 26.541 |
| 13 | 440 | 4.150 / 62.646 / 44.282 | 2.669 / 23.994 / 22.114 | 5.933 / 37.786 / 38.409 |
| 13 | 7040 | 9.945 / 75.698 / 45.321 | 5.591 / 30.774 / 27.296 | 10.970 / 47.904 / 33.949 |

Clenshaw often improves on the forward stateless recurrence, but it does not
beat current 1x in any measured scalar, x4, or x8 case. Even its closest result,
cap-2 x8 at 7040 Hz, is 18.3% slower than current before any block accumulation,
lane reduction, direct latency, or transition work.

## Pitch and cap transitions

Because evaluation starts from authoritative phase every sample, an abrupt
pitch change has no stale recurrence state. It cannot, however, remove the
Fourier-series change when the legal cap changes:

| Cap change | RMS omitted harmonic | Peak omitted harmonic |
|---:|---:|---:|
| 13 to 12 | 0.034627544 | 0.048970878 |
| 6 to 5 | 0.075026356 | 0.106103361 |
| 3 to 2 | 0.150052715 | 0.212206602 |
| 2 to 1 | 0.225079067 | 0.318309903 |

An abrupt 440-to-7040 Hz plus cap-6-to-cap-3 probe measured a consecutive
sample delta of 0.313551. A 110-to-7040 Hz exponential sweep with exact integer
cap changes reached 1.686999; that number includes the saw reset edge and is a
worst consecutive-sample bound, not an isolated click metric. The analytic cap
jump table is the transition-specific evidence and matches round 4.

## Verdict and limitations

Saw has no CPU winning region, so square, pulse, and triangle were not built.
The full block/unison/factor-2 structural workload was deliberately stopped at
the mandated earlier gate: adding accumulation and transition handling cannot
make a kernel that already loses every raw structural lane uniformly
Pareto-superior. No oscillator state, object size, publication cost, production
source, or RT behavior changed.
