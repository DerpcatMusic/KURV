# Derivative-lambda portfolio, round 9 (2026-08-30)

## Outcome

Rejected. Keep the shipped fixed `lambda = 1e-6` compiler and bounded selector in
0.8.7. A four-lambda portfolio found 34 incremental candidates, but one regressed
dense source error and one regressed strict ideal-bandlimited output despite each
passing the same bounded production proof. Four extra solves cost about 75.3 us on
the representative curve and would add a selection loop/tie-break contract for
only 6.6% more corpus coverage. No production code, version, RT layout, or
evaluator changed.

## Portfolio

The experiment compiled `[1e-8, 1e-7, 1e-5, 1e-4]` after the complete 0.8.7
compiler/selector. Every candidate was compared directly with the already shipped
curve using the exact same bounded proof: 1% source RMS improvement, peak and knot
guards, analytic range/overshoot, maximum smooth jump, derivative-event energy,
and hard/wrap preservation.

Among candidates that passed, the portfolio chose the lowest 256-probe squared
source error. Ties retain the first lambda in fixed ascending order, making the
decision deterministic.

## 512-curve result

- individually eligible by lambda: `[0, 0, 7, 33]`;
- unique incremental selections: 34/512;
- selected by lambda: `[0, 0, 7, 27]`;
- categories `[smooth, hard, clustered, near_duplicate, extrema, wrap,
  max_knots, random]`: `[1, 0, 0, 2, 0, 0, 31, 0]`;
- topology regressions under the full oracle: 0;
- dense source regressions: 1;
- strict worst-of-three ideal-BL regressions: 1;
- dense RMS reduction min/median/max:
  -0.300554% / 3.808001% / 88.014918%;
- worst BL delta min/median/max:
  -18.430797 / -0.249927 / +0.048271 dB;
- representative four-solve compile cost: 75.252 us.

The two smallest lambdas produced no eligible curves, so half the portfolio is
pure overhead. The remaining coverage is not zero-regression and does not justify
shipping FFT/dense gates or inventing another heuristic to identify the single
failure. The one-fixed-lambda contract is smaller, cheaper, and already proven.

## Commands and evidence

```text
taskset -c 4 cargo test --release --no-default-features --lib \
  wave_curve::compiler_experiment::derivative_lambda_portfolio_report \
  --locked -- --ignored --nocapture
```

Passed 1/1. It evaluated the complete seeded corpus, deterministic portfolio
selection, dense source, ideal-BL, analytic range, knots, derivative topology and
event energy, exact 256-byte layout, and compile cost.

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

Passed 1/1 in 4.42 s. Current 48 kHz/440 Hz 1x medians were 6.819 ns/sample saw,
6.916 square, 6.557 pulse31, 10.776 triangle, and 9.795 drawn. The portfolio never
entered production, so audio-path code and CPU are unchanged.
