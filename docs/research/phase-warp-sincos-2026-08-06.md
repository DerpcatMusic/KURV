# Phase-warp sine/cosine reuse

## Result

Keep the shared range-reduction path for PWM only. It removes duplicate SIMD
quadrant folding while preserving the existing polynomial, phase mapping, and
BLEP inputs bit-for-bit.

Bend and Harmonic used the same candidate during the audit, but their dense
v3 gains stayed below 5%. They therefore retain the simpler existing calls.
The scalar `sin_cos()` spelling compiled to the same instructions as the
existing paired `sin()`/`cos()` calls and was also rejected.

## CPU

The full-path workload is Spline Optimized 2x, saw, 24 notes, three
oscillators, no jitter, 65,536 host frames, and three `perf stat` repetitions.
Counts include the identical 16,384-frame warm-up. Lower is better.

| Target | PWM lanes | Baseline instructions | Candidate instructions | Change |
|---|---:|---:|---:|---:|
| x86-64-v3 | 1 | 5,764,794,279 | 5,764,793,655 | 0.00% |
| x86-64-v3 | 4 | 6,157,582,885 | 5,909,859,264 | -4.02% |
| x86-64-v3 | 8 | 6,506,328,959 | 6,223,216,009 | -4.35% |
| x86-64-v3 | 64 | 30,306,350,665 | 28,041,422,417 | **-7.47%** |
| x86-64 | 1 | 7,475,762,476 | 7,475,762,574 | 0.00% |
| x86-64 | 4 | 7,903,503,618 | 7,608,591,380 | -3.73% |
| x86-64 | 8 | 12,178,673,042 | 11,293,934,502 | **-7.27%** |
| x86-64 | 64 | 68,179,151,339 | 61,101,263,128 | **-10.38%** |

The dynamic 24-note, 64-lane, three-oscillator morph benchmark retired 4.99%
fewer v3 instructions and 7.56% fewer portable instructions. Unwarped,
Bend, and Harmonic rows stayed within 1.40% of the baseline across 1/4/8/64
lanes, with no regression.

## Quality

Portable and v3 release binaries rendered PWM, Bend, and Harmonic at 98%
amount for:

- saw and moving saw-pulse morph;
- low bin 997 and high bin 5003;
- 64 lanes at 2x quality.

All 24 candidate renders were bit-identical to the baseline: maximum sample
error `0`, RMS error `0`. The high-bin PWM saw alias residual was unchanged at
`-86.070 dBc`, so the alias delta is exactly 0 dB. This also preserves
monotonicity, endpoints, and raw pulse-edge correction because the generated
phase and step inputs are unchanged.

## Reproduction

```bash
CARGO_TARGET_DIR=/tmp/kurv-phase-v3 \
RUSTFLAGS='-C target-cpu=x86-64-v3' \
cargo build --release --no-default-features --example generator_lab

KURV_LAB_WARP=pwm taskset -c 8 perf stat -r 3 -e instructions:u -- \
  /tmp/kurv-phase-v3/release/examples/generator_lab \
  bench splineopt 2 saw 64 65536 1 69 0.5 0 0.7 24 noise 3
```
