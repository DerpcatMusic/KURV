# 1x finite harmonic candidate: stateless Clenshaw

Source under test: `src/oscillators/va/one_x_harmonic.rs`. Experimental, not wired to production. The baseline is the actual repository antialias.rs imported at build time with only the truce-simd import replaced by its underlying wide types. No private plugin dependency is needed.

Unlike rejected experiment18's time-recursive additive renderer, this evaluates a finite Fourier series at authoritative phase every sample. There is no retained recurrence state, drift, oscillator ownership byte, or reset/repacking policy. It provides scalar f64 and packed f32x8 evaluation, with libm/wide trig and shipping polynomial trig variants. Harmonics are bounded at16 and unsupported lower notes are rejected. Cached coefficients are appropriate for constant-pitch blocks. Generic coefficient rebuilding is unsuitable for cheap audio-rate modulation.

## Reproduce

From repository root, use Rust1.97.1 (the installed toolchain may need RUSTUP_TOOLCHAIN to prevent component downloads):

```sh
RUSTUP_TOOLCHAIN=1.97.1 cargo run --release --offline --manifest-path tools/audits/one_x_harmonic/Cargo.toml
RUSTUP_TOOLCHAIN=1.97.1 cargo run --release --offline --manifest-path tools/audits/one_x_harmonic/Cargo.toml -- --spectrum
RUSTUP_TOOLCHAIN=1.97.1 RUSTFLAGS='-C target-cpu=native' cargo run --release --offline --manifest-path tools/audits/one_x_harmonic/Cargo.toml -- --bench
```

Add `--libm` to any command to use libm/wide trig for candidate evaluation (the exact scalar oracle always uses f64 trig). `cpu-generic.csv` is generic target/libm; `cpu-native.csv` native/libm; `cpu-fast-native.csv` native/shipping polynomial. Likewise `quality.csv` and `spectrum.csv` are generic/libm, `*-fast.csv` native/polynomial. Outputs retained before formatting/refactoring; timing will vary. CPU AMD EPYC9V74, virtualized shared runner. Seven-run median, 262144 evaluations per run. Configuration timing separate. Not pinned; other cooperating CPU benchmarks paused. Not a complete oscillator/block/host benchmark; no stereo summing, note management, dispatch, cache packing, or transition cost. Thus this establishes candidate crossover regions, not final synth speedup.

## Findings

Native packed polynomial candidate/baseline cost ratios:

| Hz | Saw x8 | Triangle x8 |
| ---: | ---: | ---: |
| 3000 | 3.405 | 1.812 |
| 6000 | 1.979 | 0.984 |
| 11000 | 1.436 | 0.744 |
| 16000 | 0.949 | 0.564 |
| 21000 | 0.904 | 0.529 |

Scalar generic Clenshaw remains slower on the native target even at high notes; it should not replace the existing scalar kernel. Native packed high-note triangle is promising. A direct one/two-partial specialization can remove dynamic loop and coefficient-array overhead. For step>=0.2, triangle contains only its fundamental; saw has at most two harmonics. This is the recommended production follow-up, not broad generic Clenshaw deployment.

The finite Fourier sum is an exact analytic reference for these static waveforms; no oversampling convergence uncertainty is involved. Scalar Clenshaw agrees with the direct sum at ~-300dB relative numerical error. f32x8 error vs that sum is measured independently. Tests cover mixed shapes/counts in a SIMD pack, finite oracle error, scalar fast-basis error, invalid pitch, and continuity across count and taper boundaries. They do not establish quality under modulation.

The spectrum harness projects coherent 32768-sample runs onto wanted harmonic bins and reports residual power separately. Baseline saw unwanted power is -29 to-55dB relative wanted power over the high-note grid; the candidate approaches the f32 numerical floor. Residual outside wanted bins is not all possible aliasing: folded energy landing on wanted bins contributes wanted-amplitude error instead. `quality*.csv` separately reports total baseline error vs the ideal hard-cutoff waveform and the candidate's deliberate taper deviation from it. `candidate_total_error_db` specifically measures exact scalar Clenshaw vs the *tapered* analytic oracle, not an assertion of perfect full-synth output. `simd_numerical_error_db` measures packed implementation vs that oracle at the same f32 phase.

## Explicit quality and integration constraints

- A smoothstep fade covers the last10% of Nyquist. It removes discontinuous harmonic-count changes but attenuates wanted upper harmonics. At1500Hz, saw total taper deviation is about-35.5dB relative signal; at11kHz about-29.6dB. This is a musical/response tradeoff, not alias power.
- Stationary finite harmonics are bandlimited. Pitch changes, PWM, PM, AM, and nested modulation create sidebands; this candidate does not solve their aliases. Do not enable it based solely on instantaneous pitch for unrestricted PM.
- Switching from the shipping BLEP waveform restores upper-frequency gain and changes output. Smooth coefficient transitions inside this engine do not make a BLEP-to-harmonic switch continuous. Production integration needs a continuous crossover or a deliberately scoped quality mode, with its own transition and CPU tests.
- Coefficient setup costs tens to hundreds of ns per lane. High-note direct formulas avoid this; the generic cached engine does not.
- Only 1x is targeted; there is no evidence for replacing the current2x–4x renderer or the existing optimized low-note narrow SIMD paths.
- No plugin build or host test was possible because derpcat-access is private. No version bump: this commit adds an unconnected experiment and evidence only.
