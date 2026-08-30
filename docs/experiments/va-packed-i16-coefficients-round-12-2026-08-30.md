# VA compiler round 12: signed-16 coefficient storage

## Verdict

Rejected. Per-plane signed-16 storage reduces a compiled curve from 256 to 144
bytes, but quantization fails the strict source, bandlimited, topology, range,
knot, and transition gates. The smaller table also did not improve the measured
64-curve cache traversal, and the portable eight-lane path was substantially
slower. Shipping `WaveCurveRt`, publication, transitions, and version 0.8.7 are
unchanged.

## Candidate

The probe preserves the shipped sixteen cubic segments and Horner polynomial.
Each of the four coefficient planes gets one `f32` symmetric scale; its sixteen
coefficients are rounded to `i16`. Evaluation loads and converts only the four
coefficients selected for the current segment. No dependency or native-half
path was added.

Storage is 128 bytes of coefficients plus 16 bytes of scales:

| structure | shipping | packed |
|---|---:|---:|
| curve | 256 B | 144 B |
| generation plus payload | 260 B | 148 B |
| two curves plus transition progress | 516 B | 292 B |

An actual atomic implementation would use atomic integer words and retain the
same generation protocol; the table reports payload accounting rather than
claiming a production layout.

## 512-curve oracle

The deterministic corpus and 65,536-point source/ideal-bandlimited oracle used
by the previous compiler rounds reported:

- source regressions: 332/512
- ideal-bandlimited regressions: 360/512
- topology regressions: 50/512
- analytic range regressions: 422/512
- knot regressions: 380/512
- transition sample errors above `1e-5`: 284
- mean curve-to-curve quantization RMS: 0.000023806
- peak curve-to-curve quantization error: 0.000562370
- worst ideal-bandlimited error delta: +46.546902070 dB
- encoding cost: 239.9 ns per curve

The large worst dB delta occurs where the floating baseline has an extremely
small residual; it still correctly demonstrates that coefficient quantization
adds a nonzero artifact floor. Hard/wrap events were not intentionally altered,
but coefficient rounding can move their measured magnitudes.

## Runtime measurements

Release-mode nanoseconds per output sample on this machine:

| path | shipping `f32` | packed `i16` |
|---|---:|---:|
| scalar, one resident curve | 9.734 | 8.820 |
| x4 | 7.141 | 5.919 |
| x8 | 2.472 | 5.817 |
| scalar, 64-curve strided table | 10.025 | 10.441 |

The scalar/x4 microbench gain does not survive the table/cache case, while x8
regresses 2.35x. These portable packed SIMD probes decode lanes independently;
a specialized native widening/gather implementation would add structural code
and cannot repair the already-failed quality gates.

## Commands

```text
cargo fmt --all
cargo test --release \
  wave_curve::compiler_experiment::packed_i16_coefficient_report \
  -- --ignored --nocapture
```

Passed 1/1. The result line is reproduced by the figures above.

```text
cargo test --release wave_curve::topology_tests --locked
```

Passed 3/3. Existing shipping topology remains intact.

