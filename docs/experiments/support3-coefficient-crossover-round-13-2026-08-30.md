# Support-three coefficient crossover round 13 (rejected)

Date: 2026-08-30

Baseline: `a43dc19` (production DSP unchanged)

Verdict: rejected at saw steady-high gate; no runtime code added

## Common coefficient representation

The bridge used three normalized-distance regions `[0,1)`, `[1,2)`, and
`[2,3)`, each represented by eight ascending coefficients for a degree-seven
Estrin evaluator. Shipping's optimized cubic/support-two residual embeds as:

- inner coefficients: `[-0.5, 0.6267450904, -0.0003685148,
  -0.2733964856, 0.0944836641, 0, 0, 0]`;
- outer tail polynomial expanded from `t = 2-d` into global `d`:
  `[-0.6611105205, 1.2311460612, -0.8530614035, 0.2595962322,
  -0.0291066153, 0, 0, 0]`;
- the `[2,3)` shipping region is all zero.

Round 11/12's quality coefficients are the same in every region:
`[-0.5, 0.8717257825, 0.7676317308, -2.1080998407, 1.4231892057,
-0.4203181843, 0.0540191377, -0.0021152837]`.

The transition linearly interpolates these coefficient vectors, then evaluates
one Estrin polynomial. A broad normalized-step band `0.075..0.140` was used;
below it the actual shipping path remains untouched and above it the quality
kernel is selected.

## Offline bridge checks

The exact Python/NumPy probe expanded the tail polynomial, evaluated coefficient
endpoints, checked every piece boundary at `epsilon=1e-7`, and rendered 110-to-
7040 Hz exponential sweeps:

```text
python3 - <<'PY'
# Define shipping inner/expanded outer/zero coefficient vectors and the
# support-three quality vector. Lerp coefficients, evaluate one Estrin form,
# and inspect d=1/2/3 plus 4,096- and 65,536-frame pitch sweeps.
PY
```

At blend zero, coefficient evaluation reproduces the embedded shipping
polynomial to floating-point evaluation precision; at blend one it reproduces
the quality polynomial. Worst left/right boundary differences were:

| Distance | Blend 0 | Blend 0.5 | Blend 1 |
|---:|---:|---:|---:|
| 1 | 3.71e-8 | 1.69e-8 | -3.37e-9 |
| 2 | 2.64e-10 | -1.30e-9 | -2.86e-9 |
| 3 | 0 | -5.78e-11 | -1.16e-10 |

Thus the common piecewise basis and coefficient interpolation do not create a
support-region discontinuity. Fast and slow up/down sweeps remained finite.
The maximum candidate-minus-shipping correction magnitude was 0.335-0.346 and
its largest adjacent change was 0.599-0.624; these include ordinary waveform
edge corrections and are not presented as isolated clicks. A production claim
would still require complete rendered transition comparison.

## Decisive steady and duty-cycle CPU gate

The above-band kernel is byte-for-byte the round-12 support-three Estrin
schedule, so round 12's two pinned real x8 block measurements are directly the
steady-state gate:

| Run | 7040 Hz current | Quality Estrin | Difference |
|---:|---:|---:|---:|
| 1 | 564.265 ns | 568.316 ns | +0.7% |
| 2 | 638.381 ns | 615.590 ns | -3.6% |

This is measurement parity, not a reproducible `<= current` win. The unchanged
low region was decisively faster than quality Estrin at 440 Hz: 299.257 versus
355.500 ns and 302.594 versus 371.106 ns. Coefficient interpolation during the
band adds seven vector lerps before the same evaluator, so any realistic duty
cycle is strictly more expensive than choosing unchanged current below plus the
measured quality kernel above. It cannot convert a noisy high-note tie into a
uniform win.

Scalar also remained about 2.2 times current at 7040 Hz and cannot be promoted;
x4 lost as well. A scalar fallback does not repair the required x8 structural
steady-high reproducibility.

## Decision and scope stop

Reject before runtime integration. The bridge is continuous, but the quality
kernel does not reproducibly beat current in its proposed steady high region,
and transition coefficient work only worsens CPU. Saw therefore fails the
explicit promotion gate. Square/pulse would apply the residual to additional
edges and cannot improve this CPU result; triangle also needs the corresponding
integrated residual, so none were expanded. No source, state, object size,
publication, version, or RT behavior changed.
