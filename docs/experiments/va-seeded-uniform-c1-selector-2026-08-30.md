# Seeded uniform C1 selector corpus

Status: **successful property infrastructure; rejected for production**

## Purpose and invariant

This round tests the constrained least-squares C1 candidate from `afe3a9f` over a deterministic broad editor-curve corpus. Both legacy and candidate compile to the exact existing 16 uniform cubics and 256-byte `WaveCurveRt`; the runtime evaluator, interpolation, publication, SIMD layout, and per-sample cost remain unchanged.

The seed is `0x4b555256c1012026`. The corpus contains 512 curves, 64 in each category:

- smooth musical curves;
- hard alternating points aligned to uniform boundaries;
- clustered/tight transitions;
- near-duplicate phases exercising sanitization;
- aggressive extrema/overshoot shapes;
- hard wrap transitions;
- maximum 16-knot curves;
- randomly jittered musical/adversarial curves.

## Oracle and selection gate

Every curve is sanitized through the production path. The test-only candidate is selected only when all of these hold:

- 8,192-phase source RMS improves by at least 0.1%;
- dense peak error is no worse within `1e-6`;
- every explicit knot is individually no worse within `1e-6`;
- analytic cubic extrema add no clamp crossing and no range overshoot beyond `1e-6`;
- artificial slope jumps are no worse;
- existing hard and wrap slope jumps retain at least 95% of legacy magnitude;
- ideal-bandlimited complex error at coherent periods 436, 55, and 7 regresses by no more than 0.1 dB.

The last tolerance was used only to determine whether the broad approach had useful coverage. It is not strict enough for production acceptance.

## Reproduce

```bash
taskset -c 4 cargo test --release --no-default-features --lib \
  wave_curve::compiler_experiment::seeded_uniform_least_squares_c1_property_report \
  --locked -- --ignored --nocapture

cargo fmt --all -- --check
git diff --check
```

Run on 2026-08-30 Asia/Jerusalem with Rust 1.98.0, release thin LTO, one codegen unit, CPU 4 affinity, and baseline commit `afe3a9f`. The changed release link took 2m36s and the corpus report took 19.45s.

## Results

| Category | Selected / 64 |
|---|---:|
| Smooth | 0 |
| Hard | 21 |
| Clustered | 0 |
| Near duplicate | 59 |
| Extrema | 0 |
| Wrap | 0 |
| Maximum 16 knots | 33 |
| Random | 7 |
| **Total** | **120 / 512 (23.438%)** |

Among selected curves, source RMS reduction was `0.287879% / 66.209128% / 96.588389%` minimum/median/maximum. Ideal-bandlimited error delta was `-38.105488 / -9.684470 / +0.076549` dB minimum/median/maximum; negative is better.

Median compiler-only time was 899.3 ns for legacy and 10,456.3 ns for the constrained candidate. This excludes the selection oracle. The full dense and FFT property evaluation averaged about 38 ms per curve in the 19.45-second corpus run, which is appropriate for an ignored experiment but not for routine editor publication.

The corpus confirms that constrained shared slopes can materially help hard, sanitized near-duplicate, maximum-knot, and some random curves. It also confirms that the full oracle is doing essential work: smooth, clustered, extrema, and wrap candidates were all rejected.

## Verdict

Do not integrate this selector, change `WaveCurveData::compile_rt`, remove runtime clamps, or bump the crate version. The gate is too expensive and its 0.1 dB tolerance admitted a measured `+0.076549` dB bandlimited regression. Retain the seeded corpus as the full offline oracle.

The next experiment should derive a small bounded production selector from source RMS/peak, per-knot and hard/wrap checks, analytic extrema, and derivative jumps only, then validate its choices against this corpus with a strict `0.0 dB` observed bandlimited-regression ceiling.
