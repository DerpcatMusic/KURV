# Curvature-event regularization, round 10 (2026-08-30)

## Outcome

Rejected. Keep the shipped 0.8.7 slope-regularized compiler and selector. Adding a
second-derivative event penalty improved curvature-event energy on many corpus
curves, but zero candidates passed the complete bounded selector against the
already shipped curve. Four extra solves cost about 134.7 us and produced no
incremental coverage. Production compiler, version, 256-byte runtime layout, and
evaluator remain unchanged.

## Derivation

For a uniform Hermite cubic `p(t) = a*t^3 + b*t^2 + c*t + d` with segment width
`h`, the normalized second derivatives at the right and left endpoints are:

```text
p''(1) = 6a + 2b
p''(0) = 2b
```

At a boundary between left `L` and right `R`, the normalized curvature jump is:

```text
J = 2*bR - (6*aL + 2*bL)
```

Substituting Hermite endpoint values and physical slopes makes this affine in the
four neighboring endpoint slopes:

```text
J = (-6*yR0 + 6*yR1 - 6*yL0 + 6*yL1)
    - 2h*sL0 - 4h*sL1 - 4h*sR0 - 2h*sR1
```

Therefore `lambda * J^2` is a quadratic update to the existing fixed normal
system. The experiment added it alongside the shipped slope-jump penalty
`1e-6`. Intentional hard joins and a hard wrap were excluded; a smooth wrap was
penalized consistently with other smooth boundaries.

## 512-curve sweep

Curvature lambdas were `[1e-8, 1e-6, 1e-4, 1e-2]`. Every candidate still had to
pass the exact shipped bounded source/peak/knot/range/hard/wrap/slope-event proof,
plus curvature-event energy no greater than the current 0.8.7 curve.

- candidates passing the complete selector: `[0, 0, 0, 0]`;
- curves with lower slope-event energy: `[119, 112, 118, 141]`;
- curves with lower curvature-event energy: `[191, 192, 225, 332]`;
- deterministic portfolio selections: 0;
- dense-source, BL, and topology regressions among selections: 0, vacuously;
- four-solve representative compile cost: 134.692 us;
- runtime object: unchanged 256 bytes.

The penalty does what it is intended to do geometrically, but those energy gains
do not coexist with the existing source and topology proof often enough to select
even one curve. There is no reason to add production coefficients, a lambda loop,
or another selector branch.

## Commands and evidence

```text
taskset -c 4 cargo test --release --no-default-features --lib \
  wave_curve::compiler_experiment::curvature_event_regularized_report \
  --locked -- --ignored --nocapture
```

Passed 1/1. It swept the full seeded corpus and measured slope/curvature event
energies, bounded eligibility, dense/BL oracle results for eligible curves,
deterministic portfolio selection, compile cost, and exact layout.

```text
cargo test --release --no-default-features --lib \
  wave_curve::topology_tests --locked
```

Passed 3/3 for the unchanged shipping compiler.

```text
taskset -c 4 cargo test --release --no-default-features --lib \
  oscillators::va::experiment::shipping_1x_va_quality_and_cpu_report \
  --locked -- --ignored --nocapture
```

Passed 1/1 in 4.64 s. Current 48 kHz/440 Hz 1x medians were 7.178 ns/sample saw,
7.162 square, 6.714 pulse31, 10.831 triangle, and 9.934 drawn. Runtime code is
unchanged because the curvature candidate was rejected before integration.
