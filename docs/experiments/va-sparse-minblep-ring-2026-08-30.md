# Sparse minimum-phase BLEP residual ring experiment (2026-08-30)

## Verdict

Reject for production and do not integrate.

The 8-sample sparse minimum-phase BLEP ring is a genuine steady-state quality improvement over KURV's current 1x optimized polynomial BLEP for saw, square, and pulse at all three measured pitches. It is not a universal Pareto improvement:

- scalar cost is `1.222x` to `1.966x` current;
- x8 cost is `1.646x` to `5.909x` current;
- overshoot and most pitch/reset/width transition steps are worse;
- current 2x remains substantially closer to the ideal band-limited curve;
- 16- and 32-sample minimum-phase kernels become slower and less curve-accurate despite improving wanted-harmonic magnitude.

This result is committed as a test-only negative experiment. Production DSP is unchanged.

## Question and distinction from earlier probes

Could a genuinely event-scheduled BLEP be sparse enough to beat the current canonical saw/square/pulse path at 1x while improving physical curve accuracy?

This probe is distinct from the rejected alternatives:

- The custom windowed-sinc experiment evaluated a fractional residual image at every sample. This probe deposits a finite residual only when an edge occurs and consumes one fixed ring slot per sample.
- The sparse canonical schedule duplicated/prewalked block phase state. This probe finds fractional crossings during the real phase walk.
- The analytic event iterator reorganized the whole block traversal. This probe retains the ordinary sample traversal and adds only an event insertion path.

The tested kernel is independently generated from a Blackman-Harris-windowed sinc through a real-cepstrum minimum-phase reconstruction and integration. It is not copied from VCV Rack/Fundamental.

## Primary sources

- Eli Brandt, [Hard Sync Without Aliasing](https://www.cs.cmu.edu/~eli/papers/icmc01-hardsync.pdf), ICMC 2001: the minimum-phase band-limited step construction and event-rate cost model.
- VCV Rack, [current MinBLEP implementation](https://github.com/VCVRack/Rack/blob/v2/include/dsp/minblep.hpp), and Fundamental, [current VCO use](https://github.com/VCVRack/Fundamental/blob/v2/src/VCO.cpp): contemporary evidence for an exact-subsample, serial-active-lane, fixed future-buffer architecture. Fundamental's current source uses `MinBlep<16, 16>`, corresponding to a 32-sample residual at 16x table resolution.

## Candidate

- 8, 16, and 32 output-sample residuals, each with 16 fractional table positions.
- Exact fractional saw wrap, pulse-width edge, and pulse wrap detection during the current phase walk.
- Fixed power-of-two scalar and x8 future rings; active SIMD lanes alone perform serial insertion.
- Audible phase reset and pulse-width changes deposit a value-step residual.
- Saw-only zero-state DC compensation subtracts `event rate * jump * mean finite-kernel area`. The mean is the exact integral of each linearly interpolated fractional table interval. This preserves pulse-width DC and avoids the state and per-sample history of a high-pass filter.
- No allocation, lock, I/O, logging, or resizing occurs in the candidate process/timed path. The experiment generates the table off-thread; a production version would require a static table.

The fractional residual-area span was about `0.9375` samples for every kernel, but the saw's raw sampled-curve bias changes oppositely with crossing fraction. The constant mean compensation held saw DC below `1.6e-6` in the coherent cases and below `3.92e-7` over the 16-position high-pitch fractional sweep.

| Residual | Table bytes | Scalar state | x8 state | Mean residual area |
|---:|---:|---:|---:|---:|
| 8 | 516 | 48 B | 352 B | -1.906430125 |
| 16 | 1,028 | 80 B | 608 B | -2.381638527 |
| 32 | 2,052 | 144 B | 1,120 B | -2.820178747 |

The x8 state is per oscillator pack. The 32-sample form's 1,120-byte pack state is already unattractive for KURV's many-lane architecture even before its CPU and quality losses.

## Measurement contract

- Checkout: `7850f6bfb4571cf7ba4a3d0102132d71ae05fb46` (`exp/va-minblep`).
- Host: AMD Ryzen 7 7800X3D, Linux x86-64, release build, `-C target-cpu=native`.
- Timing pinned to logical CPU 6 with a watchdog that aborted if another Cargo/rustc process appeared.
- Real KURV internal blocks: 24 (`FACTOR3_BLOCK_INTERNAL_SAMPLES`) and 32 (`BLOCK_INTERNAL_SAMPLES`).
- CPU reports are medians of seven paired repetitions with alternating current/candidate order, 40,000 blocks per repetition.
- Quality reference: exact ideal band-limited harmonic projection at coherent periods 436, 55, and 7 (`110.09`, `872.73`, and `6857.14 Hz` at 48 kHz).
- `wanted_magnitude_error_db` measures wanted-bin magnitude error; `wanted_complex_error_db` includes wanted-bin phase error; `alias_error_db` is total residual energy against the ideal projection and therefore includes folded alias, magnitude, and phase error that cannot be separated after coherent folding.
- Curves are aligned by integer then 1/16-sample fractional delay before curve/error metrics.
- A 16-position phase sweep checks fractional edge timing at period 7.
- Pitch, reset, and width transitions report peak, DC, global maximum step, and local maximum step around the transition.
- A 4,096-frame scalar/x8 comparison includes pitch, width, and reset transitions.

## Steady ideal-reference result

The best candidate was always 8 samples. The table shows total residual error in dB; more negative is better.

| Wave | Frequency | Current 1x | MinBLEP 8 | Current 2x |
|---|---:|---:|---:|---:|
| Saw | 110.09 Hz | -28.330 | -31.324 | -35.563 |
| Saw | 872.73 Hz | -19.270 | -21.978 | -27.214 |
| Saw | 6857.14 Hz | -9.614 | -14.010 | -35.919 |
| Square | 110.09 Hz | -30.076 | -33.047 | -36.992 |
| Square | 872.73 Hz | -21.224 | -25.599 | -28.205 |
| Square | 6857.14 Hz | -11.104 | -14.917 | -35.148 |
| Pulse 31% | 110.09 Hz | -30.165 | -33.874 | -37.136 |
| Pulse 31% | 872.73 Hz | -21.050 | -23.903 | -29.177 |
| Pulse 31% | 6857.14 Hz | -12.202 | -20.825 | -41.064 |

The 8-sample curve RMS also beats current 1x in every row, but current 2x remains better in every row. At 6857 Hz, for example:

| Wave | Current 1x RMS | MinBLEP 8 RMS | Current 2x RMS |
|---|---:|---:|---:|
| Saw | 0.173628 | 0.104666 | 0.008402 |
| Square | 0.264299 | 0.170391 | 0.016591 |
| Pulse 31% | 0.230006 | 0.085226 | 0.008292 |

The high-pitch 16-position sweep worsened the 8-sample worst case to `-12.461 dB` saw, `-14.917 dB` square, and `-19.432 dB` pulse. DC remained correct.

Longer kernels did not approach the physical curve more closely:

| Residual | Saw residual range | Square residual range | Pulse residual range |
|---:|---:|---:|---:|
| 8 | -31.324 to -14.010 dB | -33.047 to -14.917 dB | -33.874 to -20.825 dB |
| 16 | -28.863 to -10.607 dB | -30.582 to -10.617 dB | -30.401 to -17.977 dB |
| 32 | -26.144 to -6.465 dB | -27.901 to -5.553 dB | -27.731 to -16.296 dB |

Their wanted-bin magnitude error can be excellent (for example, 32-sample saw reaches `-45.832 dB` at 110 Hz and `-41.264 dB` at 6857 Hz), but minimum phase produces nonlinear harmonic phase. A single alignment delay cannot restore the ideal waveform, so more taps improve magnitude while making the time-domain curve less faithful.

## Transition and artifact result

The 8-sample candidate's scalar and x8 outputs were bit-exact in all nine parity cases (`max_abs_error=0`) through pitch, width, and reset transitions.

It nevertheless overshoots more than current 1x. Representative local maximum steps:

| Wave / transition | Current 1x | MinBLEP 8 |
|---|---:|---:|
| Saw pitch | 0.8414 | 1.3507 |
| Saw reset | 1.1195 | 1.5104 |
| Square pitch | 1.1394 | 1.6219 |
| Square reset | 1.1378 | 1.5288 |
| Square width | 1.1381 | 1.6041 |
| Pulse pitch | 1.1317 | 1.6975 |
| Pulse reset | 1.1378 | 1.6479 |
| Pulse width | 2.0000 | 1.6493 |

The pulse-width jump is the one clear transition improvement, because the candidate explicitly deposits a residual for that discontinuity. It does not offset the other transition regressions or the candidate's steady peaks around `1.27` to `1.29` versus at most `1.0` for current 1x.

## Real 24/32-block CPU result

Ratios are candidate/current; below 1 would win.

| Residual | Scalar ratio range | x8 ratio range |
|---:|---:|---:|
| 8 | 1.222-1.966x | 1.646-5.909x |
| 16 | 1.297-2.731x | 2.065-10.681x |
| 32 | 1.437-4.632x | 2.690-18.812x |

The 8-sample candidate's best case was square at 440 Hz and block 32 (`1.222x` scalar). Its worst x8 case was saw at 7040 Hz and block 32 (`5.909x`). The event-rate scaling predicted by Brandt is directly visible: sparse insertion is tolerable at low pitch but becomes dominant at high pitch, especially when eight staggered lanes generate frequent events.

Block 24 versus 32 did not change the decision. Adjacent ratios were close and neither block produced a candidate win.

## Why this stops here

This round validates the architecture rather than the product gate:

- sparse real-walk deposition is correct and avoids the earlier duplicated schedule;
- exact mean-area compensation avoids the causal MinBLEP DC failure without a high-pass state;
- fixed-ring scalar and x8 implementations agree through required transitions;
- 8 samples improve every measured current-1x steady curve.

But it is not Pareto-safe. The best version is slower in every measured workload, has worse transition/peak behavior, and remains far behind current 2x curve accuracy. Longer minimum-phase tails amplify both the CPU problem and nonlinear-phase curve error. No production integration or assembly round is warranted for this formulation.

## Commands

```bash
CARGO_TARGET_DIR=/tmp/kurv-va-minblep-target \
RUSTFLAGS='-C target-cpu=native' \
cargo test --release --no-default-features --lib \
  oscillators::va::minblep_experiment::sparse_minblep_kernel_and_state_are_bounded \
  --locked -- --nocapture --test-threads=1

taskset -c 6 env \
  CARGO_TARGET_DIR=/tmp/kurv-va-minblep-target \
  RUSTFLAGS='-C target-cpu=native' \
  cargo test --release --no-default-features --lib \
  oscillators::va::minblep_experiment::sparse_minblep_ring_report \
  --locked -- --ignored --exact --nocapture --test-threads=1
```

The full guarded report completed in `23.93 s`. The compile emitted only the checkout's pre-existing unused/dead-code warnings.
