# Uniform shared-slope wave-curve compiler experiment

Status: **promising infrastructure; rejected for production pending a seeded corpus**

## Contract

Both candidates compile into the existing 16 uniform cubic segments and 64 coefficients. The result remains a 256-byte `WaveCurveRt`; scalar, four-lane, eight-lane, atomic publication, interpolation, and audio-thread evaluation are byte-for-byte unchanged.

Two offline candidates were measured:

1. Cubic Hermite using the exact one-sided `SourceCurve` slopes, shared at non-hard uniform joins.
2. Cubic Hermite with slopes solved by deterministic constrained least squares over 31 interior samples per segment. Slopes are shared across ordinary joins but kept separate where an editor knot on that boundary has a real one-sided derivative jump.

The acceptance probe chooses a candidate only when 65,536-phase RMS and peak source error, explicit-knot peak error, analytic cubic clamp crossings, and artificial derivative jumps are all no worse than legacy. The selection policy is test-only in this commit.

## Reproduce

```bash
taskset -c 4 cargo test --release --no-default-features --lib \
  wave_curve::compiler_experiment::uniform_c1_compiler_report \
  --locked -- --ignored --nocapture

taskset -c 4 cargo test --release --no-default-features --lib \
  wave_curve::compiler_experiment::uniform_least_squares_c1_compiler_report \
  --locked -- --ignored --nocapture

cargo fmt --all -- --check
git diff --check
```

Run on 2026-08-30 Asia/Jerusalem with Rust 1.98.0, release thin LTO, one codegen unit, CPU 4 affinity, and baseline commit `5302141`. Each changed release test link took about 2m30s; reports took 0.12s and 0.19s.

## Exact-source-slope C1: rejected

| Curve | Shipping RMS / peak | C1 RMS / peak | Shipping BL dB low / mid / high | C1 BL dB low / mid / high |
|---|---:|---:|---:|---:|
| Default | 0.000000068 / 0.000000179 | 0.000000292 / 0.000000484 | -138.732 / -139.380 / -145.915 | -125.982 / -126.376 / -128.074 |
| Drawn | 0.007584623 / 0.048635125 | 0.013724320 / 0.091624469 | -37.647 / -39.503 / -47.603 | -32.494 / -32.802 / -38.440 |
| Tight | 0.237545145 / 1.683291793 | 0.252637759 / 1.778545737 | -10.730 / -11.056 / -15.551 | -10.194 / -10.507 / -15.504 |
| 16-knot | 0.001418472 / 0.003634572 | 0.007371573 / 0.018399715 | -52.547 / -53.675 / -52.608 | -38.231 / -38.219 / -32.692 |

It removes artificial derivative jumps but loses source, knot, and ideal-bandlimited accuracy on every curve. Compilation was 356-440 ns versus 221-1,057 ns for legacy; speed off the audio thread does not rescue the quality failure.

## Constrained least-squares C1: potential, not shipped

| Curve | Decision | Shipping RMS / peak | Candidate RMS / peak | Clamp crossings legacy / candidate | Compile ns legacy / candidate |
|---|---|---:|---:|---:|---:|
| Default | candidate | 0.000000068 / 0.000000179 | 0 / 0 | 2 roundoff / 0 | 930.8 / 10,028.1 |
| Drawn | legacy | 0.007584623 / 0.048635125 | 0.009288500 / 0.057725251 | 0 / 0 | 1,034.1 / 10,454.0 |
| Tight | legacy | 0.237545145 / 1.683291793 | 0.084131961 / 1.183224916 | 0 / 2 | 225.4 / 10,679.5 |
| 16-knot | candidate | 0.001418472 / 0.003634572 | 0.001392978 / 0.003379941 | 0 / 0 | 1,159.6 / 19,120.0 |

For the selected 16-knot case, ideal-bandlimited error improved from `-52.547/-53.675/-52.608` dB to `-52.705/-53.788/-56.889` dB. Its hard and wrap slope jumps remained intentional: `142.267/81.974` legacy versus `142.317/81.990` candidate. The drawn candidate reduced the artificial join jump from `10.294285` to `0.000006`, but its source and bandlimited errors regressed, so legacy correctly wins. The tight candidate strongly reduced source and bandlimited error, but analytic extrema found two hidden clamp crossings, so it is rejected.

The default candidate merely removes f32 interpolation noise and two `2.38e-7` range excursions. This is not a meaningful musical win by itself. Candidate compilation is roughly 10-19 microseconds and allocates only fixed stack matrices; it remains deterministic and off the audio thread.

## C2 boundary

A cubic can be globally C2 through a spline solve, but preserving every intentional editor slope corner requires splitting the solve at hard points. The C1 candidate already needs per-curve fallback and lacks corpus evidence, so a second solver was not added. Quintic Hermite would require six coefficients per segment and violate the fixed 256-byte runtime contract.

## Verdict

Do not change `WaveCurveData::compile_rt`, remove its clamp, or bump the crate version. Retain the test-only source-error, knot, join, extrema, Fourier, and compile-cost machinery for the next round. Production selection needs a large deterministic seeded editor-curve corpus proving that the legacy fallback catches every source, hard-point, range, and ideal-bandlimited regression before the small 16-knot gain is worth shipping.
