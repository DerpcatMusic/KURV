# Adaptive C1 wave-curve compiler experiment

Status: **rejected for production; test-only probe retained**

## Question and candidate

Could the 16 uniform independent cubics in `WaveCurveRt` be replaced by an error-driven shared partition without losing explicit editor points?

The probe starts with every sanitized editor knot as a hard segment boundary, bisects the source interval with the largest measured cubic error until the same 16-segment cap is full, and fits cubic Hermite pieces from the source's exact one-sided value and slope. Artificial subdivisions are C1; explicit knot and wrap slope jumps remain hard. Compilation, FFT analysis, extrema solving, allocation, and reporting are test-only and never run on the audio thread.

The runtime candidate needs 17 explicit boundaries in addition to 64 coefficients. Its selector uses `partition_point`; the production `WaveCurveRt` and all scalar/SIMD evaluators were left unchanged.

## Reproduce

```bash
taskset -c 4 cargo test --release --no-default-features --lib \
  wave_curve::compiler_experiment::adaptive_c1_compiler_report \
  --locked -- --ignored --nocapture

taskset -c 4 cargo test --release --no-default-features --lib \
  oscillators::va::experiment::shipping_1x_va_quality_and_cpu_report \
  --locked -- --ignored --nocapture

cargo fmt --all -- --check
git diff --check
```

Run on 2026-08-30 Asia/Jerusalem with Rust 1.98.0, release thin LTO, one codegen unit, and CPU 4 affinity. The first release link took 2m33s; the compiler report itself took 12.32s. Repository baseline was `92dfb168209888815c445760ae0e48ce2ea58e05`.

## Source fit and ideal-bandlimited error

RMS and peak compare the unclamped compiler output with `SourceCurve` over 65,536 phases. Bandlimited error compares complex Fourier coefficients retained by coherent periods 436, 55, and 7 (approximately 110 Hz, 873 Hz, and 6.86 kHz at 48 kHz). More-negative dB is better.

| Curve | Shipping RMS / peak | Adaptive RMS / peak | Shipping BL dB, low / mid / high | Adaptive BL dB, low / mid / high |
|---|---:|---:|---:|---:|
| Default four-knot | 0.000000068 / 0.000000179 | 0.000000292 / 0.000000490 | -138.732 / -139.380 / -145.915 | -125.944 / -126.132 / -127.093 |
| Drawn six-knot | 0.007584623 / 0.048635125 | 0.000033559 / 0.000104845 | -37.647 / -39.503 / -47.603 | -84.727 / -84.746 / -86.756 |
| Tight transition | 0.237545145 / 1.683291793 | 0.000012764 / 0.000211954 | -10.730 / -11.056 / -15.551 | -96.506 / -96.567 / -98.155 |
| Adversarial 16-knot | 0.001418472 / 0.003634572 | 0.007371573 / 0.018399715 | -52.547 / -53.675 / -52.608 | -38.231 / -38.219 / -32.692 |

The candidate is excellent when it has spare segments to allocate, but it regresses when all 16 slots are already occupied by hard editor points. Choosing it globally would violate the no-regression requirement at the same segment cap.

## Joins, range proof, cost, and layout

| Curve | Shipping artificial slope jump | Adaptive artificial slope jump | Preserved adaptive hard / wrap jump | Shipping / adaptive compile ns | Shipping / adaptive eval ns |
|---|---:|---:|---:|---:|---:|
| Default | 0.000008 | 0.000000 | 8.000000 / 0.000000 | 988.1 / 127599.1 | 10.796 / 9.312 |
| Drawn | 10.294285 | 0.000042 | 14.000305 / 30.764271 | 910.7 / 132708.1 | 6.311 / 7.908 |
| Tight | 28.780647 | 0.244507 | 2024.693359 / 6.413529 | 929.3 / 297923.7 | 18.823 / 24.508 |
| 16-knot | none | none | 143.667023 / 82.432190 | 2747.9 / 1670.5 | 16.357 / 25.513 |

The tight curve's `0.244507` residual is f32 coefficient cancellation against a hard slope near 2,025, about `1.2e-4` relative. The mathematical construction shares the same source slope across artificial joins.

Every cubic derivative was solved analytically. Candidate extrema remained inside `[-1, 1]` in all four cases, with zero clamp crossings. Shipping also had zero material crossings; the default curve reached `-1.000000238` and `1.000000238`, two f32-rounding excursions only. Therefore this matrix does not justify removing the production clamp.

Candidate state is 336 bytes versus the unchanged 256-byte `WaveCurveRt`. Lookup was slower in three representative/adversarial cases and cannot reuse the existing direct uniform index or coefficient-plane SIMD selection. CPU timing varies with curve/branch prediction, so these single-core figures establish the regression, not a universal throughput ratio.

The existing VA reference harness also passed unchanged. At 48 kHz its current shipping 1x/2x drawn-curve alias/error was `-60.600/-45.587`, `-37.189/-39.039`, and `-8.749/-14.218` dB at low/mid/high pitch; measured drawn CPU was 9.773 ns/sample at 1x and 37.913 ns/sample at 2x. This compiler probe measures editor-source loss before that oscillator/oversampler boundary and does not claim to repair custom-wave aliasing.

## Verdict

Reject the nonuniform adaptive C1 representation. Do not change production code and do not bump the crate version. A C2 variant was not built: after the C1 candidate failed the fixed-cap adversarial and realtime-layout gates, adding second-derivative constraints or two more polynomial coefficients could not repair those already-disqualifying costs without a different representation.

The useful finding is narrower: shared slopes remove artificial derivative events and error-driven allocation recovers narrow detail. A future layout-preserving experiment should keep the exact 16 uniform segments and 256-byte evaluator, then select a shared-slope fit only when an offline deterministic source-error and extrema comparison proves it no worse than the legacy coefficients.
