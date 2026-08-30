# Tiny canonical projection tables, round 20 (rejected)

Date: 2026-08-30

Verdict: reject. Tiny immutable tables solve memory and interpolation accuracy, and adjacent-cap blending removes hard waveform jumps, but cubic lookup is far slower than both the actual current renderer and cap-limited additive recurrence. The softened cap target also introduces material wanted-harmonic error.

## Representation

Only saw and triangle need stored tables. Square and arbitrary-duty pulse can be derived from two shifted saw evaluations plus DC:

```text
pulse(phase, width) = saw(phase + 1 - width) - saw(phase) + 2 * width - 1
```

Caps 1-6 at 128, 256, or 512 samples therefore cost:

| size | saw + triangle, six caps |
|---:|---:|
| 128 | 6,144 bytes |
| 256 | 12,288 bytes |
| 512 | 24,576 bytes |

All are immutable and require no oscillator state or publication mechanism. The temporary test generated the exact Fourier cycles before timing; production would embed identical constants.

## Catmull-Rom interpolation accuracy

Dense 65,536-phase comparisons used KURV's existing periodic four-point Catmull-Rom formula.

| size | saw cap 1 RMS/max | saw cap 3 RMS/max | saw cap 6 RMS/max | triangle cap 6 RMS/max |
|---:|---:|---:|---:|---:|
| 128 | 6.14e-7 / 1.21e-6 | 6.17e-6 / 1.70e-5 | 3.10e-5 / 1.11e-4 | 4.81e-6 / 1.22e-5 |
| 256 | 7.66e-8 / 1.51e-7 | 7.61e-7 / 2.12e-6 | 3.71e-6 / 1.38e-5 | 5.83e-7 / 1.52e-6 |
| 512 | 9.57e-9 / 1.89e-8 | 9.48e-8 / 2.64e-7 | 4.58e-7 / 1.72e-6 | 7.23e-8 / 1.89e-7 |

The 128-point bank is already accurate enough that interpolation is not the rejection cause.

## Continuous cap blend and ideal error

For `cap_position = Nyquist / frequency`, the legal top harmonic was ramped from zero to one over the interval between adjacent integer caps. This is stateless and waveform-continuous at cap crossings, but deliberately attenuates a legal wanted harmonic through most of the band.

Representative error against the hard exact ideal projection:

| Hz | shape | softened RMS | peak | wanted error dB |
|---:|---|---:|---:|---:|
| 7,000 | saw | 0.085744 | 0.121261 | -15.74 |
| 7,000 | square | 0.171489 | 0.242522 | -14.86 |
| 7,000 | pulse31 | 0.037409 | 0.052904 | -27.98 |
| 7,000 | triangle | 0.036391 | 0.051465 | -24.00 |
| 9,000 | saw | 0.075026 | 0.106103 | -16.53 |
| 9,000 | pulse31 | 0.139515 | 0.197305 | -16.52 |
| 10,000 | saw | 0.135047 | 0.190986 | -11.43 |
| 10,000 | pulse31 | 0.251128 | 0.355148 | -11.42 |

Square/triangle have no even harmonic, so their cap-2 blend cells are exact. Exact integer boundaries at 6, 8, and 12 kHz also select the lower legal cap exactly. Between boundaries, the softened target is much less accurate than exact additive and can approach current-renderer curve error.

Slow/fast pitch sweeps have no hard cap jump because adjacent tables meet at blend endpoints. They retain a derivative kink at integer cap positions. Pulse width sweeps are phase-shift-continuous, but require twice the saw lookup cost. Adjacent-sample peaks contain no extra state/reset discontinuity; their remaining change is the continuous pitch/width modulation itself.

## Actual x8 block CPU

```text
CARGO_BUILD_JOBS=1 CARGO_PROFILE_RELEASE_STRIP=none \
RUSTFLAGS='-C target-cpu=native' \
  cargo test tiny_canonical_table_cpu_report --lib --release --locked --no-run

taskset -c 8 \
  target/release/deps/pure_va_dispersion_core-bcfc5e2fdb5df6ce \
  oscillators::va::experiment::tiny_canonical_table_cpu_report \
  --ignored --exact --nocapture --test-threads=1
```

The probe used 20,000 64-frame stereo x8 blocks and medians of five runs. Cap selection/blend was once per block; cubic lookup, phase advance, and accumulation were inside the sample loop.

| Hz | current saw ns/block | 128-table saw ns/block | ratio |
|---:|---:|---:|---:|
| 6,000 | 174.084 | 2,350.507 | 13.50 |
| 7,000 | 192.319 | 2,389.065 | 12.42 |
| 8,000 | 204.934 | 2,242.547 | 10.94 |
| 9,000 | 198.540 | 2,261.564 | 11.39 |
| 10,000 | 207.902 | 2,264.170 | 10.89 |
| 12,000 | 262.909 | 2,224.193 | 8.46 |

This scalar-lane cubic evaluator is the portable behavior and a best-cache coherent-lane case. Decorrelated lanes change indices and cannot improve it. The existing 2,048-table experiment showed native-style x8 lookup structure dominated by gather/interpolation; even an implausible 4x gain from a dedicated AVX2 gather leaves this candidate 2.1-3.4x slower than current. Pulse needs two table evaluations. Round 17's exact cap-limited additive saw was only about 146-179 ns/block, making the table roughly 13-16x slower than the stronger exact competitor.

## Decision

Reject and revert. The tiny bank is a successful memory/interpolation result, but lookup is the wrong execution architecture for KURV's x8 canonical hot loop. Cap blending trades clicks for substantial wanted attenuation, while exact additive is both more accurate and much faster in the same cap range. No table constants, runtime evaluator, state, version change, or production code is retained.

Limitations: the timed prototype used the portable scalar-lane cubic evaluator rather than a new handwritten AVX2 gather. The rejection remains robust to the measured order-of-magnitude gap, existing gather evidence, doubled pulse lookup, and faster additive baseline. Coherent lanes are the favorable cache case; decorrelated-lane CPU was not separately retained after this gate failed.
