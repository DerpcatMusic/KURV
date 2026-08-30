# Range-safe Bézier/Hermite compiler, round 6 (2026-08-30)

## Outcome

Rejected for production. Constraining every uniform Hermite segment through its
equivalent Bézier control points proved all 512 candidates and all tested
interpolations pointwise safe in `[-1,1]`, but severely regressed source accuracy,
ideal-bandlimited output, and intended hard/wrap topology. Removing evaluator
clamps also produced only a small structural microbenchmark saving. No production
code, evaluator, layout, or version changed.

## Construction and proof

For Hermite endpoint values `y0`, `y1` and normalized endpoint slopes `m0`, `m1`,
the equivalent Bézier controls are:

```text
y0, y0 + m0/3, y1 - m1/3, y1
```

The experiment clamps slopes so all four controls lie in `[-1,1]`. At C1 joins,
the adjacent endpoint constraints are intersected and one shared slope is used;
explicit hard knots and a hard wrap keep independent slopes. A cubic Bézier lies
inside the convex hull of its controls, so this is an analytic range proof rather
than a dense-sample approximation.

Coefficient interpolation is also safe when both endpoints are safe: Hermite and
Bézier conversion is linear, so interpolating coefficients interpolates every
control point inside the convex set `[-1,1]`. The corpus confirmed 0 failures over
1,533 pair/mix checks (511 adjacent pairs at 0.25, 0.5, and 0.75).

## 512-curve result

- currently shipped curves already Bézier-safe: 353/512;
- limited candidates Bézier-safe: 512/512;
- candidates unchanged from the shared-slope fit: 0/512;
- source RMS/peak improvements: 137;
- source regressions: 370;
- worst-of-three ideal-BL improvements: 184;
- strict ideal-BL regressions: 328;
- derivative hard/wrap topology regressions: 161;
- worst BL delta min/median/max: -13.707067 / +3.710218 / +24.664489 dB;
- representative compile cost: 10.431 us;
- runtime object: unchanged 256 bytes.

This fails the Pareto gate by a wide margin. It also shows why clamps cannot be
removed from the current evaluator: 159/512 currently published corpus curves do
not satisfy the sufficient Bézier-control proof. `WaveCurveData::compile_rt`
feeds editor curves and VA table frames; transitions and table selection then
interpolate coefficients. Atomic table/state paths reconstruct those published
coefficients. Interpolation would preserve safety only after every originating
compiler/constructor had the safe invariant, which is not true today.

## Clamp microbenchmark

Matched scalar and portable structural four/eight-lane evaluators were measured on
one analytically safe representative curve. Values are nanoseconds per call (the
vector calls process four or eight samples):

| Path | Clamped | Raw | Saving |
| --- | ---: | ---: | ---: |
| scalar | 8.656 | 8.351 | 0.305 |
| eval4 | 31.690 | 30.124 | 1.566 per call, 0.392/lane |
| eval8 | 5.989 | 5.771 | 0.218 per call, 0.027/lane |

The saving is not material relative to the accuracy/topology loss and incomplete
publication invariant. The production clamps remain.

## Commands and evidence

```text
taskset -c 4 cargo test --release --no-default-features --lib \
  wave_curve::compiler_experiment::range_safe_bezier_compiler_report \
  --locked -- --ignored --nocapture
```

Passed 1/1 with the corpus, analytic range, interpolation, BL, topology, compile,
layout, and clamp-cost results above.

```text
cargo test --release --no-default-features --lib \
  wave_curve::topology_tests --locked
```

Passed 3/3: neutral multi-knot, ideal saw, and ideal triangle.

```text
taskset -c 4 cargo test --release --no-default-features --lib \
  oscillators::va::experiment::shipping_1x_va_quality_and_cpu_report \
  --locked -- --ignored --nocapture
```

Passed 1/1 in 4.41 s. A repeat captured current 48 kHz/440 Hz 1x medians of
7.268 ns/sample saw, 6.558 square, 6.557 pulse31, 11.648 triangle, and 9.886 drawn.
These are the unchanged production path with clamps present; the rejected limiter
never entered the audio path.
