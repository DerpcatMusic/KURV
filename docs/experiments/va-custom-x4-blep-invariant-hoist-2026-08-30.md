# Constant custom-x4 BLEP invariant hoist (2026-08-30)

## Verdict

Ship the exact saw/square specialization in
`accumulate_custom4_block_constant`. Version 0.8.12 prepares the active mask,
two-sample support, reciprocal step, and optimized-mode flag once per block,
then reuses `spline_blep4_precomputed` for each wrap and pulse edge. Other
shapes retain the former evaluator.

This changes no interface, oscillator state, phase arithmetic, allocation,
lock, I/O, or runtime dispatch. The structural and legacy four-lane tails are
the existing consumers.

## Identity

The ignored 24/32-frame probe compares production with the preserved old
kernel across two step regimes (including zero, 0.249/0.251, and 0.45), all
five antialiasing enums, four warp modes, exact saw, two square widths,
staggered phases, nonuniform stereo gains, and 64 chained blocks per cell.
Every phase and output sample matched by `to_bits`.

## Fair structural-router CPU

Both sides traverse identical engine, jitter, voice-count, custom, warp,
canonical, lane-gain, and stereo-accumulation branches at four render voices;
a const test selector changes only the BLEP helper. Each cell used 75,000
blocks, nine alternating-order repetitions, CPU 4, and a predeclared 1.005
paired-median parity limit.

| frames | shape | warp | old ns/block | 0.8.12 ns/block | ratio |
|---:|---|---|---:|---:|---:|
| 24 | saw | none | 166.455 | 158.195 | 0.9470 |
| 24 | saw | PWM | 334.501 | 304.127 | 0.9099 |
| 24 | saw | harmonic | 285.388 | 256.976 | 0.9012 |
| 24 | square | none | 228.213 | 212.301 | 0.9306 |
| 24 | square | PWM | 441.305 | 421.471 | 0.9522 |
| 24 | square | harmonic | 388.167 | 350.676 | 0.9044 |
| 32 | saw | none | 224.380 | 213.525 | 0.9519 |
| 32 | saw | PWM | 453.314 | 413.138 | 0.9142 |
| 32 | saw | harmonic | 386.696 | 344.438 | 0.8921 |
| 32 | square | none | 310.604 | 286.340 | 0.9245 |
| 32 | square | PWM | 573.114 | 544.882 | 0.9495 |
| 32 | square | harmonic | 503.586 | 460.437 | 0.9145 |

All 12 paired medians improved by 4.78% to 10.79%, and the router's 256-block
per-cell identity check also passed.

## Validation

The focused VA release suite passed 23 tests with 21 ignored. The pinned full
release library suite completed with 351 passed, 9 known baseline failures,
and 47 ignored. The unpinned suite completed with 357 passed, the three known
RESYNTH-vocoder/structural-LFO failures, and 47 ignored. No VA or custom-x4
test failed.
