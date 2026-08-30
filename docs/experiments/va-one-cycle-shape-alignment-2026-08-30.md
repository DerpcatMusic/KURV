# One-cycle ideal-shape alignment harness

Date: 2026-08-30

Baseline: `e3f9964`, KURV `0.8.10`

Status: accepted as a test-harness metric. Production DSP, dependencies, and
the package version are unchanged.

## Gap and contract

The live VA reference harness already compared generated samples with an exact
band-limited Fourier projection, but its shared alignment searched as far as
half a cycle and left DC and gain inside the reported curve error. An older DPW
probe had fitted those nuisance terms, but that rejected probe was removed.

The retained test-only metric compares exactly one authoritative cycle and
reports both views:

- unaligned RMS and peak plus the fitted phase offset, DC offset, and gain;
- residual RMS and peak after removing only those three stationary terms.

Phase correction is an exact FFT circular shift limited to `+/-0.5` sample. A
`1/256`-sample grid finds the local cell, then a bounded refinement handles
arbitrary subsample offsets. DC and scalar gain are fitted independently for
each phase trial. The bounded phase allowance cannot erase a wrong cycle start,
accumulated phase drift, or a whole-sample timing error.

## Validation

```text
CARGO_BUILD_JOBS=1 cargo test one_cycle_shape_alignment_validation_report \
  --lib --release --locked -- --ignored --nocapture --test-threads=1
```

The synthetic source was one 109-sample exact band-limited saw cycle. Each row
changes only the named property.

| defect | raw RMS / peak | phase samples | DC | gain | residual RMS / peak |
| --- | ---: | ---: | ---: | ---: | ---: |
| clean | 0 / 0 | 0 | 0 | 1 | 0 / 0 |
| gain `1.2` | 0.114824 / 0.232132 | 0 | 0 | 1.2 | 0 / 0 |
| DC `+0.1` | 0.100000 / 0.100000 | 0 | 0.1 | 1 | 0 / 0 |
| phase `+0.237` | 0.044844 / 0.455295 | 0.237000 | 0 | 1 | 0 / 0.000000002 |
| within-cycle drift | 0.033441 / 0.297170 | 0.001600 | 0.002472 | 0.995978 | 0.033402 / 0.295834 |
| seventh-harmonic shape | 0.028284 / 0.039996 | 0 | 0 | 0.994482 | 0.028262 / 0.044116 |

The focused release run passed: stationary amplitude, DC, and off-grid phase
defects normalize to the numeric floor without disappearing from the raw/fitted
columns. Accumulated phase drift and a true harmonic-shape defect remain
material in both residual measures. The first run used an incorrect expectation
that drift must appear as one constant phase offset; the saw's authoritative
cycle boundary correctly anchored that fit near zero, while the large residual
already exposed the nonstationary drift. The assertion was narrowed to that
actual contract before the passing rerun.

## Scope

This metric is an offline diagnostic, not an audio-thread operation. It does not
replace the existing multi-cycle alias, folded/off-grid energy, transition,
scalar/SIMD parity, or CPU gates. It isolates cycle-shape fidelity so future VA
candidates cannot receive curve credit for a favorable constant DC, amplitude,
or tiny phase correction without reporting those corrections explicitly.
