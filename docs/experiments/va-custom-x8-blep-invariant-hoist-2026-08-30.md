# Constant custom-x8 BLEP invariant hoist (2026-08-30)

## Verdict

Ship the block-invariant BLEP preparation in the exact custom saw and square
branches of `accumulate_custom8_block_constant`. The production change reuses
the existing precomputed x8 BLEP evaluator and changes no interface, state, or
sample arithmetic on active event lanes.

The exhaustive 24/32-frame probe remained bit-identical. In the fair structural
outer renderer, all 12 paired cells improved by 4.31% to 18.40%; every
individual repetition also beat the old path. The patch version is 0.8.11.

## Change

The old exact custom saw and square loops called `edge_blep8` for every frame.
That helper rebuilt four values which are constant for the entire block:

- the active-lane mask;
- two-sample BLEP support;
- reciprocal phase step;
- optimized-residual selection.

Each exact branch now prepares those values once before its frame loop and
calls `spline_blep8_precomputed`. The square path uses the same prepared state
for its wrap and pulse-edge corrections. This is valid because an active warp
uses the already inverted raw-domain pulse edge and therefore the raw constant
phase step; an inactive warp also returns that raw step.

There is no allocation, lock, I/O, new state, runtime dispatch, API change, or
additional abstraction.

## Bit identity

The ignored release probe compares the old unprepared implementation with the
production implementation at both 24 and 32 frames. It covers two phase-step
regimes (including zero, ordinary audio rates, the narrow-support boundary, and
0.45), all five test antialiasing variants, all four warp modes, exact saw, two
square widths, staggered phases, nonuniform stereo gains, and 64 consecutive
blocks per cell.

Every phase, left sample, and right sample matched by `to_bits`.

```text
cargo test custom_constant_precomputed_blep_bit_identity --lib --release \
  --no-default-features --locked -- --ignored --nocapture --test-threads=1
```

Passed 1/1.

## Fair outer-path CPU

The ignored benchmark keeps the structural engine, jitter, voice-count,
custom/warp/canonical routing, lane setup, and stereo accumulation identical.
A const test-only selector changes only the old unprepared custom-x8 helper
versus production. Each cell uses eight staggered unison lanes, 75,000 blocks,
nine alternating-order repetitions, and a predeclared 1.005 parity bound on
the paired median. The command was pinned to CPU 4.

| frames | shape | warp | old ns/block | production ns/block | paired ratio |
|---:|---|---|---:|---:|---:|
| 24 | saw | none | 181.310 | 149.070 | 0.8244 |
| 24 | saw | PWM | 321.962 | 305.760 | 0.9499 |
| 24 | saw | harmonic | 269.155 | 242.922 | 0.9037 |
| 24 | square | none | 253.967 | 208.667 | 0.8202 |
| 24 | square | PWM | 449.188 | 409.500 | 0.9146 |
| 24 | square | harmonic | 381.480 | 352.448 | 0.9240 |
| 32 | saw | none | 252.590 | 205.030 | 0.8160 |
| 32 | saw | PWM | 422.109 | 405.573 | 0.9569 |
| 32 | saw | harmonic | 356.838 | 323.121 | 0.9011 |
| 32 | square | none | 337.512 | 277.036 | 0.8232 |
| 32 | square | PWM | 555.154 | 508.031 | 0.9158 |
| 32 | square | harmonic | 470.927 | 428.646 | 0.9083 |

The largest individual old/production ratio was 0.9807, so the conclusion does
not depend on treating a small loss as noise.

```text
taskset -c 4 cargo test \
  custom_constant_precomputed_blep_outer_structural_cpu_report \
  --lib --release --no-default-features --locked -- \
  --ignored --nocapture --test-threads=1
```

Passed 1/1.

## Regression validation

The focused VA release suite passed 23 tests, failed none, and ignored 20.

The pinned full release library suite completed with 351 passed, 9 failed, and
45 ignored. The nine failures exactly match the pre-change baseline: two
RESYNTH artifact/vocoder checks, structural LFO advancement, production-pool
dispatch, and five internal-pool timing or eligibility checks. No VA or
custom-x8 test failed.
