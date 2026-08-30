# Constant custom scalar BLEP invariant hoist (2026-08-30)

## Verdict

Ship in 0.8.13. Exact custom saw and square blocks now prepare the scalar
support-two BLEP invariants once per fixed 24/32-frame block. All 32 fair CPU
cells improved, and the direct evaluator plus the production legacy and
structural routers remained bit-identical.

## Change

The old scalar path called `generate_custom_step` for every frame. Its exact
custom saw and square branches repeatedly rebuilt values that are constant for
the block: active state, support, reciprocal phase step, optimized residual
selection, square width, and the inverse warped square edge.

`VaOscillator::generate_custom_block` now prepares those values once. It is
selected only for fixed-step exact saw/square blocks with a partial custom mix.
Dynamic-step jitter/control paths, noncanonical morph positions, and a fully
custom mix stay on the old path. Legacy single-lane rendering, settled legacy
scalar tails, structural one-lane rendering, and settled structural scalar
tails use the candidate.

The audio-thread cost remains O(frames), stack-only, deterministic, and bounded.
There is no allocation, retained state, lock, I/O, dispatch table, latency, or
approximation.

## Bit identity

The direct 24/32-frame test covers exact saw and square, seven phase steps from
zero through the 0.25 support boundary to 0.45, four pulse widths, all five test
antialiasing variants, every warp mode at shallow/medium/full depth, three
partial custom mixes, and 64 consecutive blocks per cell. Every output and
final phase matched by `to_bits`.

The second check compares the production legacy and structural one-lane routers
with the old repeated-step router for 256 consecutive 24/32-frame blocks. Every
output matched by `to_bits`.

```text
taskset -c 4 cargo test custom_scalar_prepared_blep_bit_identity \
  --lib --release --no-default-features --locked -- \
  --ignored --nocapture --test-threads=1

taskset -c 4 cargo test \
  custom_scalar_prepared_blep_production_router_bit_identity \
  --lib --release --no-default-features --locked -- \
  --ignored --nocapture --test-threads=1
```

Both passed.

## Fair outer-router CPU

Ryzen 7 7800X3D, release thin LTO, CPU 4, 75,000 blocks per measurement, nine
alternating-order repetitions. Ratios are prepared/current paired medians; the
predeclared non-regression ceiling was 1.005.

| frames | shape | warp | legacy ns old -> new | ratio | structural ns old -> new | ratio |
|---:|---|---|---:|---:|---:|---:|
| 24 | saw | none | 339.715 -> 262.055 | 0.7768 | 346.045 -> 286.255 | 0.8257 |
| 24 | saw | PWM | 623.327 -> 365.850 | 0.5872 | 573.198 -> 436.004 | 0.7605 |
| 24 | saw | phase bend | 634.716 -> 307.355 | 0.4819 | 562.211 -> 380.785 | 0.6709 |
| 24 | saw | harmonic | 628.308 -> 312.854 | 0.4940 | 545.198 -> 378.339 | 0.6997 |
| 24 | square | none | 368.682 -> 269.901 | 0.7280 | 408.604 -> 302.832 | 0.7454 |
| 24 | square | PWM | 2084.074 -> 481.350 | 0.2310 | 2139.858 -> 535.750 | 0.2488 |
| 24 | square | phase bend | 1928.538 -> 406.205 | 0.2119 | 1965.040 -> 470.855 | 0.2394 |
| 24 | square | harmonic | 1936.354 -> 412.063 | 0.2107 | 1942.216 -> 465.580 | 0.2387 |
| 32 | saw | none | 411.013 -> 329.479 | 0.8016 | 452.028 -> 394.651 | 0.8627 |
| 32 | saw | PWM | 825.985 -> 486.242 | 0.5881 | 780.943 -> 581.952 | 0.7447 |
| 32 | saw | phase bend | 842.919 -> 419.618 | 0.4978 | 763.013 -> 505.604 | 0.6614 |
| 32 | saw | harmonic | 834.589 -> 414.026 | 0.4944 | 730.987 -> 501.243 | 0.6892 |
| 32 | square | none | 486.747 -> 348.077 | 0.7143 | 531.391 -> 408.185 | 0.7681 |
| 32 | square | PWM | 2793.546 -> 627.244 | 0.2237 | 2841.410 -> 696.554 | 0.2448 |
| 32 | square | phase bend | 2549.232 -> 517.775 | 0.2030 | 2618.533 -> 606.421 | 0.2310 |
| 32 | square | harmonic | 2581.561 -> 518.026 | 0.2011 | 2639.460 -> 600.620 | 0.2322 |

All paired medians and every individual repetition won. The worst individual
ratio was 0.8882. Unwarped saw isolates the BLEP preparation and still improves
13.7% to 22.3%; warped square also removes the previously repeated three-step
inverse-edge solve and improves 75.1% to 79.9%.

```text
taskset -c 4 cargo test custom_scalar_prepared_blep_outer_cpu_report \
  --lib --release --no-default-features --locked -- \
  --ignored --nocapture --test-threads=1
```

Passed 1/1 in 36.43 seconds.

## Regression validation

The focused release VA suite passed 23 tests, with 23 manual experiments
ignored. The full release library suite has unrelated baseline failures outside
this change's VA path:

- pinned to CPU 4: 351 passed, 9 failed, 51 ignored; the failures were two
  RESYNTH vocoder artifact checks, one structural LFO check, one production-pool
  check, and five internal RT-pool checks;
- unpinned: 357 passed, 3 failed, 51 ignored; the failures were the same two
  RESYNTH checks and structural LFO check, while all RT-pool checks passed.

```text
taskset -c 4 cargo test oscillators::va --lib --release \
  --no-default-features --locked -- --test-threads=1

taskset -c 4 cargo test --lib --release --no-default-features --locked -- \
  --test-threads=1

cargo test --lib --release --no-default-features --locked -- \
  --test-threads=1
```
