# Production VA integration executable

```
KURV_SIMD=baseline cargo run --release --manifest-path tools/audits/one_x_integration/Cargo.toml
KURV_SIMD=avx2 cargo run --release --manifest-path tools/audits/one_x_integration/Cargo.toml
```

Compiles the complete production VA, curve, warp, ratio, backend, performance,
oversampler and shared numerical modules. `build.rs` removes only framework
preset serialization derives/imports/adapters. It does not substitute oscillator
state, curve data, sample evaluators, kernels, or phase arithmetic. The SIMD
alias is the exact `wide` types reexported by `truce-simd` 6.3.0.

This is a standalone executable so production `#[cfg(test)]` experiments and
framework serialization tests do not enter the build. It checks 448 combinations
of factor, waveform, phase step and pulse width through the actual scalar,
x4/x8 constant blocks, time SIMD and runtime-selected saw backend, including
phase state advancement. The single executable is not a plugin-host or whole
voice test: private authorization and UI dependencies remain outside its scope.

Initial production-mode validation: all cases pass for baseline and AVX2/FMA;
maximum sample difference from scalar is 0.000007883 (f32 arithmetic).

## Opt-in production candidate

Build the plugin with `--features experimental-1x-dsp` to select `OneX` at
factor 1. Factors 2–4 keep their existing selection. The harness takes the same
feature flag:

```
KURV_SIMD=avx2 cargo run --release --manifest-path tools/audits/one_x_integration/Cargo.toml --features experimental-1x-dsp
KURV_SIMD=avx2 cargo run --release --manifest-path tools/audits/one_x_integration/Cargo.toml --features experimental-1x-dsp -- --bench
```

Final matrix: 1,120 canonical cases include mixed lane frequencies/phases,
shape morphing, and lanes on opposite sides of the crossover; 720 additional
custom/warp cases compare actual scalar and x8 block playback and assert active
warp output equals shipping mode. Both baseline and AVX2/FMA pass. Largest
scalar/SIMD absolute differences: 0.000029981 canonical, 0.000011206 custom.

The combined branch adds 144 explicit PM API cases with bit-identical baseline
output and phase state, including zero depth and nested offsets. It also checks
the actual scalar API against the one-harmonic Fourier oracle and tests neighboring
phase increments around .20, .225, .25 and .45. Generator-graph fallback in the
voice layer received source review, including worker/serial eligibility, but that
layer is outside this executable's compilation boundary.

The benchmark is paired, alternating before/after ordering, median of seven
runs, using real constant blocks, heterogeneous phases and detuned lanes. Solo
uses the real time-SIMD renderer. Eight or more saw lanes use the actual
runtime-selected saw backend. Outputs are accumulated and consumed. Numbers
are per lane sample, not complete voice or plugin CPU measurements. Results
are machine-dependent, not evidence of a universal ranking.

Committed CSV metadata: AMD EPYC 9V74 80-Core Processor, KVM guest exposing
9 CPUs; Rust 1.97.1 (`8bab26f4f`), LLVM 22.1.6, x86_64-unknown-linux-gnu;
release opt-level 3, thin LTO, no RUSTFLAGS or profile overrides. Runs are not
pinned and have no CPU isolation/frequency control. `KURV_SIMD` selects the
named runtime backend; it does not globally compile the crate for AVX2.

### Measured quality/CPU tradeoff (AVX2-selected backend)

| Shape / base phase step | 1 lane ratio | 8 lanes ratio | 64 lanes ratio |
| --- | ---: | ---: | ---: |
| Saw, 0.01 | 1.033 | 1.072 | 0.959 |
| Saw (crossover), 0.225 | 1.597 | 2.292 | 2.402 |
| Saw, 0.3 | 0.654 | 0.466 | 0.438 |
| Triangle (crossover), 0.225 | 1.137 | 1.928 | 1.829 |
| Triangle, 0.3 | 0.490 | 0.598 | 0.592 |
| Pulse (unchanged DSP), 0.3 | 0.930 | 1.000 | 1.002 |

Below 1 means cheaper than shipping. The crossover deliberately evaluates
both spline and harmonic renderers. The first integration lost the shipping
saw's AVX2 path and cost 8–9x at .225. The final AVX2 crossover kernel
precomputes the blend, harmonic weights and BLEP parameters once per block,
reducing the measured regression to **2.29x at eight lanes and 2.40x at 64**.
That remaining quality/CPU tradeoff is explicit: this is an opt-in experiment,
not an across-the-board optimization claim. Portable crossover paths still
cost more; see `results/baseline.csv`.

At .30 and .44 all detuned lanes are above the crossover. A compile-time
specialization eliminates BLEP, cosine and second-harmonic work, so the final
AVX2 eight-lane saw is approximately 53–57% cheaper in those measurements.
Generic constant-shape blocks also reach this backend for a pure saw.

The .20–.25 crossover corresponds to 9.6–12 kHz at 48 kHz host sample rate.
Saw harmonic two fades during .225–.25; the fundamental alone remains above
.25, with a separate .45–.50 Nyquist taper. Triangle uses its sole remaining
fundamental in this range. Pulse, active warp, ratio sources and arbitrary
custom curves receive no spectral improvement from this candidate. Applying
these pointwise formulas to deep nested PM can worsen error; production PM
routing must preserve the baseline until a separate method proves better.
