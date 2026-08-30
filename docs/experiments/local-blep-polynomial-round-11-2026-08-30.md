# Local BLEP polynomial round 11 (rejected)

Date: 2026-08-30

Baseline: `1960ee6` (production DSP unchanged)

Machine: AMD Ryzen 7 7800X3D, Linux x86-64 host-default release, CPU 8

Verdict: current optimized cubic is locally unbeaten; no runtime code retained

## Offline fits

The target was the ideal Nyquist step residual for positive normalized distance
`x`, `Si(pi x) / pi - 0.5`; odd application supplies the negative side. A dense
numeric integration of `sinc(x)` generated the fitting oracle. Constrained
least squares tested only the requested two upgrades:

- same two-sample support: piecewise degree-six polynomials, exact `-0.5` at
  the event, zero tail value/slope, and C2 continuity at the one-sample join;
- three-sample support: one degree-seven polynomial, exact `-0.5` at the event
  and exact zero value/first/second derivative at the support endpoint.

Offline target-fit RMS/max errors were 0.01034/0.04859 and 0.01136/0.03309,
respectively. Runtime prototypes retained current phase-step normalization,
odd sign, event masks, phase advance, and x8 precomputed-step block seam. They
added no state, publication, allocation, lock, I/O, or lookup.

## Exact commands

```text
python3 - <<'PY'
# Dense trapezoidal sinc oracle plus equality-constrained NumPy least squares.
# The fitted coefficients and constraints are recorded above and in the
# temporary test output; no fitting code entered production.
PY
cargo fmt
taskset -c 8 cargo test local_blep_upgrade_report --lib --release --locked -- --ignored --nocapture --test-threads=1
```

The first render exposed an offline-harness phase-wrap error. The candidate
phase was corrected to `(sample % period) / period`; all quality values below
come from the corrected rerun. CPU values are corrected final-run medians of
five repetitions: one million scalar samples, 500,000 x4/x8 vectors, and
12,000 64-frame x8 stereo accumulation blocks.

## Ideal saw accuracy

| Hz | Candidate | RMS / peak | Boundary residual | DC |
|---:|---|---:|---:|---:|
| 27.507 | degree6/support2 | 0.005399 / 0.097177 | 0.163364 | 3e-9 |
| 27.507 | degree7/support3 | 0.004291 / 0.066187 | 0.116248 | 1e-9 |
| 440.367 | degree6/support2 | 0.021592 / 0.097233 | 0.163503 | 1e-8 |
| 440.367 | degree7/support3 | 0.017156 / 0.066271 | 0.116444 | 1.7e-8 |
| 6857.143 | degree6/support2 | 0.077241 / 0.111932 | 0.203316 | 8.5e-8 |
| 6857.143 | degree7/support3 | 0.051166 / 0.091396 | 0.182779 | -1.69e-6 |

For context, earlier shared-reference current-cubic RMS at the same coherent
periods was 0.016720, 0.044112, and 0.173628. Both fits materially improve
whole-curve error, with support3 the quality leader. The ideal target fit and
coherent residual include wanted and unwanted components; this short gate did
not retain a separate FFT wanted/unwanted table after every candidate lost the
real CPU requirement. That is a limitation, not a claim of spectral dominance.

## CPU and structural gate

Nanoseconds per scalar sample or SIMD vector, `current / support2 / support3`:

| Hz | Scalar | x4 | x8 |
|---:|---:|---:|---:|
| 440 | 4.254 / 5.726 / 5.970 | 2.590 / 3.076 / 3.101 | 4.200 / 5.473 / 6.473 |
| 7040 | 6.401 / 8.651 / 14.824 | 4.607 / 6.786 / 5.849 | 10.037 / 12.767 / 10.285 |

Actual 64-frame x8 stereo blocks:

| Hz | Current | degree6/support2 | degree7/support3 |
|---:|---:|---:|---:|
| 440 | 326.137 ns | 392.466 ns | 402.528 ns |
| 7040 | 585.117 ns | 797.263 ns | 665.767 ns |

Support3's raw high-note x8 result comes within 2.5% of current, but the actual
block is 13.8% slower. At 440 Hz it is 23.4% slower. Support2 is 20.3-36.3%
slower structurally. Hardware instruction/performance-counter collection was
stopped after the pinned elapsed-cycle proxy lost every real block cell; no
instruction-count result is claimed. More FMAs and the wider event window make
the direction unsurprising, but the measured wall-time gate is the verdict.

## Decision

Neither allowed upgrade satisfies the promotion rule: quality improves, but
real CPU is never equal or lower. There is also no material CPU improvement at
non-worse quality. The existing optimized cubic therefore remains locally
Pareto-unbeaten among these exact-symmetry, two/three-sample polynomial event
residuals. No candidate source, state, version change, or benchmark helper was
retained.
