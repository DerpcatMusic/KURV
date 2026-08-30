# Canonical analytic ADAA round 10 (rejected)

Date: 2026-08-30

Baseline: `4d50d4a` (production DSP unchanged)

Machine: AMD Ryzen 7 7800X3D, Linux x86-64 host-default release, CPU 8

Verdict: rejected at saw; no runtime or benchmark code retained

## Candidates

This round removed the custom-curve segment solver entirely and used the
canonical ascending saw's analytic periodic primitives:

- centered box: exact interval average from the periodic quadratic first
  primitive `phase^2 - phase`;
- centered triangle: second difference of the periodic cubic second primitive,
  equivalent to a triangular/B-spline kernel over two sample intervals.

The box used f32 scalar/x4/x8 arithmetic. The triangle used f64 scalar
differences because its second finite difference cancels at low phase steps.
Steps below `1e-5` (box) or `1e-4` (triangle) take bounded direct/box limits.
Both are stateless beyond authoritative phase and step and perform no
allocation, locking, I/O, lookup, or retained publication.

## Exact command and workload

```text
cargo fmt
taskset -c 8 cargo test canonical_saw_adaa_report --lib --release --locked -- --ignored --nocapture --test-threads=1
```

Quality uses 4096 coherent periods and the shared exact ideal projection with
phase alignment. Wanted error is complex coefficient error; alias energy
excludes legal harmonic bins. Boundary residual is the largest adjacent error
change near the cycle wrap. CPU values are five-run medians over one million
scalar samples or 500,000 SIMD vectors. Structural timing covers 12,000 real
64-frame x8 blocks and includes phase advance and stereo accumulation.

## Saw quality

| Hz | Kernel | Curve RMS / peak | Wanted error | Alias | DC | Boundary residual |
|---:|---|---:|---:|---:|---:|---:|
| 27.507 | Box | 0.075970 / 0.326823 | -19.479 dB | -22.181 dB | 8e-9 | 0.326823 |
| 27.507 | Triangle | 0.008403 / 0.200699 | -36.746 dB | -63.871 dB | ~0 | 0.297727 |
| 440.367 | Box | 0.032403 / 0.181068 | -25.009 dB | -45.236 dB | -1.2e-8 | 0.276320 |
| 440.367 | Triangle | 0.032467 / 0.179214 | -24.951 dB | -76.863 dB | ~0 | 0.276523 |
| 6857.143 | Box | 0.125662 / 0.186073 | -12.422 dB | -61.869 dB | -2.2e-8 | 0.297918 |
| 6857.143 | Triangle | 0.125806 / 0.185898 | -12.412 dB | -75.706 dB | ~0 | 0.297848 |

The higher-order kernel strongly suppresses alias energy, but its `sinc^2`
wanted-band attenuation dominates high-note accuracy. The centered box has
less attenuation but leaves much more low-note alias/error. Neither reproduces
the exact hard ideal projection; both deliberately blur every harmonic rather
than only removing those above Nyquist.

## CPU

Nanoseconds per scalar sample or SIMD vector:

| Hz | Scalar current / box / triangle | x4 current / box | x8 current / box |
|---:|---:|---:|---:|
| 440 | 4.014 / 7.920 / 11.537 | 2.585 / 13.164 | 4.078 / 26.379 |
| 7040 | 6.290 / 7.863 / 11.500 | 4.484 / 14.208 | 8.805 / 24.573 |

Actual x8 64-frame stereo accumulation cost 316.287 ns current versus
1586.080 ns box at 440 Hz, and 608.294 versus 1564.920 ns at 7040 Hz. Analytic
primitives remove the custom solver but vector floor, finite differences, and
division still cost 2.6-6.5 times the current structural/SIMD paths.

## Pitch, reset, and boundary behavior

At phase 0.371, changing step abruptly from 440 to 7040 Hz changes box output
by only `9.24e-7`; no history can become stale. Resetting phase to zero changes
the output by 0.258, which is the waveform phase reset itself rather than an
ADAA state transient. The near-zero fallback returns `-0.258000016`, the raw
saw value at that phase. DC remains bounded, but 0.28-0.33 boundary residuals
show that finite centered smoothing is not an exact bandlimited cycle.

## Verdict and limitations

Reject. Saw has no CPU Pareto region in scalar, x4, x8, or the actual structural
path. The higher-order candidate buys alias rejection with severe wanted-band
attenuation and additional CPU; the cheaper box loses both low-note quality and
CPU. Square, pulse, and triangle waveform families were therefore not built.
No object/state bytes, publication cost, production source, or RT behavior
changed.
