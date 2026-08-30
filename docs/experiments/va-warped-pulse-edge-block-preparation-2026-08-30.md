# VA warped pulse-edge block preparation

Status: **ship in 0.8.9**

## Change

The scalar warped square/pulse renderer used to solve the inverse warped pulse
edge for every sample. That edge depends only on phase step, pulse width, warp
mode, and warp amount, all of which are constant in the fixed-shape block paths.

`VaOscillator::generate_shape_block_warped` now solves the edge once and reuses
the exact result across the block. The legacy single-lane renderer, the current
structural single-lane renderer, and constant-step scalar unison tails use it
only for fixed shapes above Saw. Dynamic shape and phase-step paths retain the
per-sample implementation.

The change adds no retained state, allocation, lock, I/O, approximation, table,
or latency. It removes redundant Newton/trigonometric work from the audio
callback.

## Exactness

The fixed-block check covers three Saw-to-Pulse/Pulse shapes, four phase steps
from `0.00001` to `0.44`, four pulse widths including both clamp limits, all
three warp modes, three warp depths, and both shipping spline evaluators. Every
32-sample output and final phase matched the scalar path bit for bit.

The release CPU report additionally compared 32,768 consecutive samples for
each measured width/mode pair before timing; all comparisons passed.

## Real 32-frame CPU

Ryzen 7 7800X3D, release mode, CPU 4, 100,000 blocks per repetition, median of
five. Ratios are prepared/current, so lower is better.

| Width | Warp | Current ns/block | Prepared ns/block | Ratio | Reduction |
|---:|---|---:|---:|---:|---:|
| 0.50 | PWM | 2914.022 | 677.370 | 0.232 | 76.8% |
| 0.50 | Phase Bend | 2494.359 | 620.130 | 0.249 | 75.1% |
| 0.50 | Harmonic | 2571.883 | 626.869 | 0.244 | 75.6% |
| 0.31 | PWM | 2936.152 | 657.884 | 0.224 | 77.6% |
| 0.31 | Phase Bend | 2485.397 | 627.907 | 0.253 | 74.7% |
| 0.31 | Harmonic | 2548.539 | 612.267 | 0.240 | 76.0% |

The current structural oscillator path includes its channel accumulation:

| Width | Warp | Current ns/block | Prepared ns/block | Ratio | Reduction |
|---:|---|---:|---:|---:|---:|
| 0.50 | PWM | 3130.958 | 751.147 | 0.240 | 76.0% |
| 0.50 | Phase Bend | 2560.002 | 695.165 | 0.272 | 72.8% |
| 0.50 | Harmonic | 2599.989 | 687.201 | 0.264 | 73.6% |
| 0.31 | PWM | 3044.831 | 737.406 | 0.242 | 75.8% |
| 0.31 | Phase Bend | 2564.576 | 689.927 | 0.269 | 73.1% |
| 0.31 | Harmonic | 2576.200 | 691.417 | 0.268 | 73.2% |

This is a strict CPU win for the measured fixed warped square/pulse paths. Sound,
aliasing, artifacts, reset behavior, and block-boundary transitions are unchanged
because the reused edge has the same bits as every former recomputation.

## Commands

```text
taskset -c 4 cargo test --release --no-default-features --lib warped_ --locked \
  -- --ignored --nocapture --test-threads=1
```

Passed 2/2. The direct block and structural block reports both used the real
32-frame methods.

```text
cargo test --release --no-default-features --lib \
  oscillators::va::phase_tests::fixed_warped_pulse_blocks_match_scalar_bits \
  --locked
```

Passed 1/1.

```text
taskset -c 4 cargo test --release --no-default-features --lib --locked
```

The full release library run completed with 350 passed, 9 failed, and 34
ignored. The fixed warped-block check and all VA tests passed. The nine failures
are outside this change: two RESYNTH artifact/vocoder checks, structural LFO
advancement, production-pool dispatch, and five internal-pool timing/eligibility
checks. This run records their current presence but does not prove whether they
predate 0.8.9.
