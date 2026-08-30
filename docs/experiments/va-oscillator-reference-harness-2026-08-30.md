# VA oscillator reference harness baseline

Status: **successful infrastructure; baseline only, no oscillator candidate accepted**

## Contract

- Baseline commit: `427917dc30a176b70378b02053f10f6e905ca58f`
- Shipping path: `VaOscillator::generate_*_step` followed by the production `StereoOversampler`
- Compared factors: explicit 1x and the current default 2x
- Antialiasing: `SplineOptimized`, as selected by `Antialiasing::for_factor` for both factors
- Shapes: saw, square, 31% pulse, triangle, and one fixed six-knot drawn `WaveCurveRt`
- Rates: 44.1, 48, and 96 kHz; coherent periods nearest 110, 880, and 7,040 Hz
- Reference: exact f64 Fourier coefficients for canonical shapes; 65,536-point transform of the exact compiled custom-curve evaluator; harmonics strictly below Nyquist
- Alignment: integer search over half a cycle, 1/16-sample refinement, then exact periodic Fourier phase shift
- Offline metrics: aligned cycle RMS/max error, wanted-harmonic amplitude error, integrated alias/error energy, DC, RMS gain, and residual first-difference at cycle boundaries
- CPU: oscillator plus production delay/decimator, 2,000,000 host samples, nine release repetitions, warm-up excluded

All transforms, allocation, alignment, reporting, and timing storage are test-only and outside realtime code. The exercised shipping render and oversampler remain allocation-free and bounded.

## Reproduce

```bash
cargo test --release --no-default-features --lib \
  oscillators::va::experiment::shipping_1x_va_quality_and_cpu_report \
  --locked -- --ignored --nocapture
```

For comparable CPU numbers, pin the command to an otherwise idle core. This run used `taskset -c 4`.

## Machine and build

- Run: 2026-08-30 05:49 Asia/Jerusalem
- CPU: AMD Ryzen 7 7800X3D, 8 cores / 16 threads
- OS: Linux 7.2.0-rc7-1-cachyos-rc x86_64
- Rust: 1.98.0 (`88d9e12ae`, LLVM 22.1.8)
- Cargo: 1.98.0
- Profile: repository `release` (`thin` LTO, one codegen unit)
- Features: `--no-default-features`
- CPU 4 was pinned, but other parallel builds were active. Preserve the reported variance and rerun on an idle machine before using small CPU differences as gates.

## 48 kHz baseline results

Integrated alias/error energy is dB relative to ideal-reference energy; more negative is better. Frequencies are the coherent values 110.092, 872.727, and 6,857.143 Hz.

| Wave | 1x alias/error dB low / mid / high | 2x alias/error dB low / mid / high |
|---|---:|---:|
| Saw | -28.330 / -19.270 / -9.614 | -35.563 / -27.214 / -35.919 |
| Square | -30.076 / -21.224 / -11.104 | -36.992 / -28.205 / -35.148 |
| Pulse 31% | -30.165 / -21.050 / -12.202 | -37.136 / -29.177 / -41.064 |
| Triangle | -65.232 / -42.715 / -15.670 | -45.088 / -44.936 / -39.508 |
| Drawn | -60.600 / -37.189 / -8.749 | -45.587 / -39.039 / -14.218 |

| Wave | Factor | Median ns/host sample | Min | Max | Stddev |
|---|---:|---:|---:|---:|---:|
| Saw | 1x | 6.990 | 6.807 | 8.347 | 0.439 |
| Saw | 2x | 30.953 | 30.714 | 33.217 | 0.994 |
| Square | 1x | 6.597 | 6.556 | 6.728 | 0.059 |
| Square | 2x | 31.607 | 31.460 | 33.326 | 0.563 |
| Pulse 31% | 1x | 6.659 | 6.380 | 6.816 | 0.126 |
| Pulse 31% | 2x | 31.598 | 31.293 | 32.826 | 0.607 |
| Triangle | 1x | 11.062 | 10.894 | 11.316 | 0.110 |
| Triangle | 2x | 37.897 | 37.617 | 40.200 | 0.953 |
| Drawn | 1x | 10.222 | 10.040 | 11.739 | 0.493 |
| Drawn | 2x | 37.986 | 37.751 | 39.463 | 0.602 |

The complete command prints every metric for all 90 quality cases. At 48 kHz, representative aligned curve RMS errors were: saw 1x `0.022095842 / 0.062095641 / 0.173627877` versus 2x `0.009609603 / 0.024881702 / 0.008401733`; triangle 1x `0.000316110 / 0.004223616 / 0.094935532` versus 2x `0.003213992 / 0.003270551 / 0.006103152`; drawn 1x `0.000540284 / 0.008000760 / 0.206039997` versus 2x `0.003042918 / 0.006466300 / 0.109768630`.

## Findings and limitations

- The default 2x path costs roughly 3.4-4.8 times the measured 1x scalar path here, including its decimator.
- 2x substantially reduces high-frequency error for every tested shape and improves saw/square/pulse at every measured pitch.
- Current 1x is closer for low-frequency triangle and drawn curves. This is not a universal 1x win: 2x is better for those shapes at mid/high pitches, and its production equalizer changes low-frequency gain slightly.
- `alias_error_db` is the total residual from the ideal band-limited projection. It intentionally includes in-band magnitude/phase error and folded aliases, because a coherent periodic render cannot identify their provenance after folding.
- The custom oracle starts from the compiled 16-segment `WaveCurveRt`, not the editor's pre-compilation knot function. The harness therefore measures audible runtime-curve fidelity, not editor fitting loss.
- Constant pitch and static shape do not cover sync, PM/FM, audio-rate shape motion, or phase warp. Add those only as a candidate requires them; do not claim universal oscillator accuracy from this matrix.
- No thresholds were invented. A candidate succeeds only by reporting its deltas against both baselines, preserving realtime constraints, and stating any regression.
