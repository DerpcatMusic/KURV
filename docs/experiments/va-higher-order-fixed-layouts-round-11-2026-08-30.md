# Higher-order fixed 256-byte layouts, round 11 (2026-08-30)

## Outcome

Rejected both layouts at the early 512-curve quality gate. No scalar/x4/x8 or
audio-path benchmark was run after the broad source, BL, knot, and topology
failures, as required by the experiment stop condition. Production remains the
0.8.7 16-cubic/256-byte compiler and evaluator; no version or runtime code changed.

## Layouts and constrained fit

- 12 uniform quartics: 12 x 5 = 60 active `f32` coefficients plus four padding
  floats, exactly 256 bytes.
- 10 uniform quintics: 10 x 6 = 60 active `f32` coefficients plus four padding
  floats, exactly 256 bytes.

Both use one fixed offline KKT system. The least-squares objective samples 15
interior phases per segment. Equality rows force exact SourceCurve values at both
segment endpoints and equal first and second derivatives at every smooth uniform
join and smooth wrap. An aligned intentional hard join, or hard wrap, omits its
derivative constraints. Thus C2 continuity is a solved constraint rather than a
penalty or dense approximation.

## 512-curve result

| Metric | 12 quartics | 10 quintics |
| --- | ---: | ---: |
| Dense source RMS/peak regressions | 306 | 285 |
| Explicit-knot regressions | 371 | 462 |
| Topology/hard/wrap regressions | 196 | 252 |
| Strict worst-of-three BL wins | 226 | 253 |
| Strict worst-of-three BL regressions | 286 | 259 |
| Cases with residual smooth derivative/curvature events | 0 | 19 |
| Lower slope-event energy | 479 | 460 |
| Lower curvature-event energy | 336 | 324 |
| Interpolation checks beyond `2e-6` tolerance | 0 | 14 |
| Representative compile cost | 350.159 us | 305.715 us |

The lower segment/event count often reduces derivative tails, but the price is
broad curve-shape and BL loss. Quintic coefficient interpolation remains linear
algebraically; its 14 numeric failures and 19 residual C2-event cases expose poor
conditioning/large-coefficient rounding in the constrained solve and Horner path,
which is another rejection reason.

Neither layout approaches a strict Pareto win. Their compile costs are also an
order of magnitude above the retained cubic candidates.

## Commands and evidence

```text
taskset -c 4 cargo test --release --no-default-features --lib \
  wave_curve::compiler_experiment::fixed_256_byte_quartic_and_quintic_report \
  --locked -- --ignored --nocapture
```

Passed 1/1. It ran all 512 source, knot, ideal-BL, C2 event/tail, hard/wrap, exact
layout, interpolation, and compile-cost checks. Candidate CPU benchmarks were
intentionally skipped after the early gate failed broadly.

```text
cargo test --release --no-default-features --lib \
  wave_curve::topology_tests --locked
```

Passed 3/3 for the unchanged shipping cubic compiler.
