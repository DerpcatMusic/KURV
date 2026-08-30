# Fixed 256-byte quadratic/linear layouts, round 7 (2026-08-30)

## Outcome

Both requested alternatives were rejected before audio-path integration because
they failed the 512-curve quality and topology gates broadly. No production code,
runtime representation, evaluator, or version changed.

## Layouts

- 21 uniform quadratics: 63 active `f32` coefficients plus one padding float,
  exactly 256 bytes. Every segment fixes both SourceCurve endpoint values and
  chooses its remaining curvature coefficient by a 31-point least-squares fit.
  Evaluation is two-FMA Horner. Adjacent segments are C0.
- 32 uniform linears: slope/intercept pairs, 64 `f32` coefficients, exactly
  256 bytes. Every segment interpolates its two SourceCurve endpoints. Evaluation
  is one FMA. Adjacent segments are C0.

Both layouts interpolate coefficients pointwise correctly because evaluation is
linear in the coefficients. The corpus observed zero failures over 6,132 checks
per layout (511 adjacent curve pairs, three mixes, four phases).

## 512-curve result

| Metric | 21 quadratics | 32 linears |
| --- | ---: | ---: |
| Dense source RMS/peak regressions | 279 | 382 |
| Strict worst-of-three ideal-BL regressions | 254 | 348 |
| Derivative/hard/wrap topology regressions | 461 | 493 |
| Explicit-knot regressions | 410 | 267 |
| Analytic clamp-crossing cases | 172 | 0 (linear endpoints are bounded) |
| BL delta min | -33.093923 dB | -14.656634 dB |
| BL delta median | -0.317365 dB | +8.326797 dB |
| BL delta max | +93.844090 dB | +86.167106 dB |

Quadratics occasionally improve spectral error, but their worst failures and 90%
topology-regression rate are disqualifying. Linears are worse on the median BL
case and regress topology on 96% of the corpus. The experiment therefore stopped
before implementing either layout in the oscillator/audio publication path.

## Structural evaluator cost

Pinned release microbenchmarks exercised segment selection and Horner evaluation.
Results varied materially across two consecutive runs, so they are evidence of
direction rather than a release-grade speed claim:

- shipping scalar: 8.420-12.335 ns/call;
- quadratic scalar: 7.183-10.925 ns/call;
- linear scalar: 5.363-9.366 ns/call;
- quadratic four-lane scalar batch: 20.732-22.776 ns/call;
- linear four-lane scalar batch: 11.130-15.465 ns/call;
- shipping four-lane portable evaluator (second run): 49.448 ns/call;
- quadratic eight-lane scalar batch: 41.110-52.459 ns/call;
- linear eight-lane scalar batch: 23.189-32.134 ns/call;
- shipping eight-lane portable evaluator (second run): 43.945 ns/call.

The simpler layouts can reduce arithmetic, especially the linear candidate, but
the result is not Pareto-safe: large quality/topology failures dominate any
structural CPU opportunity. An actual oscillator-path prototype was intentionally
not added after the early quality stop.

## Commands and evidence

```text
taskset -c 4 cargo test --release --no-default-features --lib \
  wave_curve::compiler_experiment::fixed_256_byte_quadratic_and_linear_report \
  --locked -- --ignored --nocapture
```

Passed 1/1. It ran the complete 512 corpus, dense-source and ideal-BL oracles,
derivative/hard/wrap and knot checks, analytic quadratic extrema, interpolation
identity checks, exact layout assertions, and structural evaluator benchmarks.

```text
cargo test --release --no-default-features --lib \
  wave_curve::topology_tests --locked
```

Passed 3/3 for the unchanged shipping layout: neutral multi-knot, ideal saw, and
ideal triangle.
